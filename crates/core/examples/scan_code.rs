//! Find the instructions in a program, whatever the format, and print a few of
//! them with how long the sweep took.

use std::time::Instant;

use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats;
use qubero_core::source::MemSource;

const ISAS: &[&str] = &["x86-64", "x86", "x86-16", "arm64", "arm", "thumb", "riscv32", "riscv64", "BpfInsn"];

fn main() {
    let path = std::env::args().nth(1).expect("usage: scan_code <file>");
    let bytes = std::fs::read(&path).unwrap();
    // A second argument names the template, for a file that has no header to
    // say what it is.
    let name = match std::env::args().nth(2) {
        Some(name) => Box::leak(name.into_boxed_str()) as &str,
        None => match formats::sniff(&bytes[..bytes.len().min(4096)], bytes.len() as u64) {
            Some(name) => name,
            None => {
                println!("no template");
                return;
            }
        },
    };
    println!("template: {name}");
    let doc = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(formats::builtin(name).unwrap());
    // The symbol tables, when the file is one that has them, so a branch can
    // say where it goes by name.
    let program = match name {
        "elf" | "bpf" => formats::ElfProgram::read(&mut ev, &doc).ok(),
        _ => None,
    };
    walk(&mut ev, &doc, &program, &[], 0);
}

fn walk(
    ev: &mut Evaluator,
    doc: &Document<MemSource>,
    program: &Option<formats::ElfProgram>,
    path: &[usize],
    depth: usize,
) {
    if depth > 8 {
        return;
    }
    let Ok(n) = ev.node(doc, path) else { return };
    let isa = n.type_name.strip_suffix("[]").unwrap_or("");
    if ISAS.contains(&isa) {
        let start = Instant::now();
        let count = n.child_count as usize;
        let last = ev.node(doc, &child(path, count.saturating_sub(1))).map(|l| l.offset_bits / 8).unwrap_or(0);
        println!("{path:?} {} instructions, {} to {last:x}", count, n.offset_bits / 8);
        // The first few branches, which are the rows a naming pass changes.
        let mut shown = 0;
        for k in 0..count.min(4000) {
            if shown >= 8 {
                break;
            }
            let at = child(path, k);
            let Ok(insn) = ev.node(doc, &at) else { continue };
            if !format!("{:?}", insn.value).contains('$') {
                continue;
            }
            let named = program.as_ref().and_then(|p| p.machine_line(ev, doc, &at).ok().flatten());
            match named {
                Some(line) => println!("  {k:7} {:8x}  {line}", insn.offset_bits / 8),
                None => println!("  {k:7} {:8x}  {:?} (no name)", insn.offset_bits / 8, insn.value),
            }
            shown += 1;
        }
        for k in (0..count).step_by(if count > 8 { count / 6 } else { 1 }).take(0) {
            let at = child(path, k);
            if let Ok(insn) = ev.node(doc, &at) {
                let named = program.as_ref().and_then(|p| p.machine_line(ev, doc, &at).ok().flatten());
                match named {
                    Some(line) => println!("  {k:7} {:8x}  {line}", insn.offset_bits / 8),
                    None => println!("  {k:7} {:8x}  {:?}", insn.offset_bits / 8, insn.value),
                }
            }
        }
        println!("  walked in {:?}", start.elapsed());
        return;
    }
    for i in 0..n.child_count.min(64) as usize {
        walk(ev, doc, program, &child(path, i), depth + 1);
    }
}

fn child(path: &[usize], i: usize) -> Vec<usize> {
    let mut p = path.to_vec();
    p.push(i);
    p
}
