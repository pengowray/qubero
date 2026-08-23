//! A smoke test over a real module, which is the editor's own wasm build.
//! It is not in the repository, so this skips when it has not been built.

use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats::{wasm, WasmModule};
use qubero_core::source::MemSource;

#[test]
fn disassembles_the_editors_own_binary() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../web/src/pkg/qubero_wasm_bg.wasm");
    let Ok(bytes) = std::fs::read(path) else {
        eprintln!("skipped: no wasm build at {path}");
        return;
    };
    let d = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(wasm());
    let m = WasmModule::read(&mut ev, &d).unwrap();
    eprintln!("{} imports, {} defined functions", m.first_defined(), m.func_count());
    assert!(m.func_count() > 0);

    let mut stopped = 0usize;
    let mut calls_named = 0usize;
    let take = m.func_count().min(200);
    for n in 0..take {
        let text = m.disassemble(&mut ev, &d, n).unwrap();
        if text.contains(";; stops here") {
            stopped += 1;
        }
        if text.contains("call $") {
            calls_named += 1;
        }
    }
    eprintln!("of {take} bodies: {calls_named} name a call, {stopped} stop early");
    if std::env::var("DUMP").is_ok() {
        for n in 0..take {
            let t = m.disassemble(&mut ev, &d, n).unwrap();
            if t.lines().count() > 12 && t.contains("call $") {
                eprintln!("{t}");
                break;
            }
        }
    }
    assert!(calls_named > 0, "no call resolved to a name");
}
