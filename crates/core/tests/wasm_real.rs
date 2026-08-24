//! A smoke test over a real module, which is the editor's own wasm build.
//! It is not in the repository, so this skips when it has not been built.

use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats::{wasm, WasmModule};
use qubero_core::source::MemSource;

#[test]
fn disassembles_the_editors_own_binary() {
    check(concat!(env!("CARGO_MANIFEST_DIR"), "/../../web/src/pkg/qubero_wasm_bg.wasm"));
}

/// The same crate built with `-C target-feature=+simd128`, which the default
/// wasm32 target does not emit. Nothing in the repository produces it, so this
/// skips unless someone has built one there.
#[test]
fn disassembles_a_build_that_uses_simd() {
    check(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/simdtest/wasm32-unknown-unknown/release/qubero_wasm.wasm"));
}

fn check(path: &str) {
    let Ok(bytes) = std::fs::read(path) else {
        eprintln!("skipped: no wasm build at {path}");
        return;
    };
    eprintln!("--- {path}");
    let d = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(wasm());
    let m = WasmModule::read(&mut ev, &d).unwrap();
    eprintln!("{} imports, {} defined functions", m.first_defined(), m.func_count());
    let named: Vec<String> = (0..(m.first_defined() + m.func_count()) as u32)
        .map(|i| m.func_name(i))
        .filter(|n| !n.starts_with("func"))
        .collect();
    eprintln!("{} names that are not the index: {:?}", named.len(), &named[..named.len().min(8)]);
    assert!(m.func_count() > 0);

    let mut stopped = 0usize;
    let mut calls_named = 0usize;
    let mut simd = 0usize;
    let mut sample: Option<String> = None;
    let take = m.func_count().min(200);
    for n in 0..take {
        let text = m.disassemble(&mut ev, &d, n).unwrap();
        if text.contains(";; disassembly stopped") {
            stopped += 1;
        }
        if text.contains("call $") {
            calls_named += 1;
        }
        for line in text.lines().map(str::trim) {
            if line.starts_with("v128.") || line.starts_with("i8x16.") || line.starts_with("i32x4.") {
                simd += 1;
                sample.get_or_insert_with(|| line.to_string());
            }
        }
    }
    eprintln!("of {take} bodies: {calls_named} name a call, {stopped} stop early, {simd} vector instructions");
    if let Some(s) = sample {
        eprintln!("first vector instruction: {s}");
    }
    assert!(calls_named > 0, "no call resolved to a name");
}
