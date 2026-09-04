//! Print the calls of a 16-bit Windows program with the names the relocations
//! give them.

use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats;
use qubero_core::source::MemSource;

fn main() {
    let path = std::env::args().nth(1).expect("usage: scan_ne <file>");
    let bytes = std::fs::read(&path).unwrap();
    let doc = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(formats::ne());
    let p = formats::NeProgram::read(&mut ev, &doc).unwrap();
    println!("modules: {:?}", p.modules);
    let segments = vec![1usize, 33, 0];
    let count = ev.node(&doc, &segments).unwrap().child_count as usize;
    for i in 0..count {
        let body = vec![1, 33, 0, i, 4, 0, 0];
        let Ok(node) = ev.node(&doc, &body) else { continue };
        if !node.type_name.starts_with("x86") {
            continue;
        }
        println!("segment {i}: {} instructions", node.child_count);
        let mut shown = 0;
        for k in 0..node.child_count as usize {
            if shown >= 10 {
                break;
            }
            let mut at = body.clone();
            at.push(k);
            if let Ok(Some(line)) = p.instruction_line(&mut ev, &doc, &at, i) {
                let raw = ev.node(&doc, &at).unwrap().value;
                println!("  {k:6} {raw:?} -> {line}");
                shown += 1;
            }
        }
    }
}
