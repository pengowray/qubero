//! Read the executable sections of a program and print the first and last of
//! their instructions, with how long the whole sweep took.

use std::time::Instant;

use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats;
use qubero_core::source::MemSource;

fn main() {
    let path = std::env::args().nth(1).expect("usage: scan_code <file>");
    let bytes = std::fs::read(&path).unwrap();
    let name = formats::sniff(&bytes[..bytes.len().min(64)], bytes.len() as u64).unwrap_or("elf");
    println!("template: {name}");
    let doc = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(formats::builtin(name).unwrap());
    let sections = ev.node(&doc, &[7, 15]).unwrap();
    for i in 0..sections.child_count as usize {
        let n = ev.node(&doc, &[7, 15, i]).unwrap();
        if !n.type_name.ends_with("[]") || n.type_name == "bytes[]" || n.type_name == "cstr[]" {
            continue;
        }
        let start = Instant::now();
        let count = n.child_count as usize;
        println!("section {i}: {} {count} instructions", n.type_name);
        for k in (0..count).step_by(if count > 20 { count / 10 } else { 1 }).take(12) {
            let insn = ev.node(&doc, &[7, 15, i, k]).unwrap();
            println!("  {k:7} {:8x}  {:?}", insn.offset_bits / 8, insn.value);
        }
        println!("  walked in {:?}", start.elapsed());
    }
}
