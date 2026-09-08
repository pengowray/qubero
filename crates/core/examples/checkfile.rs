//! Every check in one file, taken and printed. A scratch tool for verifying a
//! declaration against a real file.

use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats;
use qubero_core::source::MemSource;

fn main() {
    let path = std::env::args().nth(1).expect("a file");
    let forced = std::env::args().nth(2);
    let bytes = std::fs::read(&path).unwrap();
    let head = &bytes[..bytes.len().min(0x9000)];
    let name = forced.unwrap_or_else(|| formats::sniff(head, bytes.len() as u64).expect("sniffed").to_string());
    println!("{path} as {name}");
    let doc = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(formats::builtin(&name).expect("template"));
    walk(&mut ev, &doc, &mut Vec::new(), 0);
}

fn walk(ev: &mut Evaluator, doc: &Document<MemSource>, path: &mut Vec<usize>, depth: usize) {
    if depth > 10 {
        return;
    }
    if let Ok(Some(info)) = ev.check_of(doc, path) {
        let v = ev.run_check(doc, path);
        let name = ev.node(doc, path).map(|n| n.name).unwrap_or_default();
        println!(
            "  {path:?} {name}: {} over {:?} unpacked {:?} {} bytes -> {}",
            info.algorithm,
            info.over,
            info.unpacked_from,
            info.covered_bytes,
            match v {
                Ok(Some(v)) if v.ok => format!("OK {}", v.computed),
                Ok(Some(v)) => format!("MISMATCH computed {} stored {}", v.computed, v.stored),
                Ok(None) => "nothing to compare".to_string(),
                Err(e) => format!("{e:?}"),
            }
        );
    }
    let Ok(info) = ev.node(doc, path) else { return };
    if !info.composite {
        return;
    }
    for i in 0..info.child_count.min(64) {
        path.push(i as usize);
        walk(ev, doc, path, depth + 1);
        path.pop();
    }
}
