//! Print a file's tree as the template reads it: path, name, type, value.
//! A second argument names the template for a file nothing sniffs.

use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats;
use qubero_core::source::MemSource;

fn main() {
    let path = std::env::args().nth(1).expect("usage: dump_tree <file> [template] [depth]");
    let bytes = std::fs::read(&path).unwrap();
    let name = match std::env::args().nth(2) {
        Some(name) => Box::leak(name.into_boxed_str()) as &str,
        None => formats::sniff(&bytes[..bytes.len().min(0x9000)], bytes.len() as u64).expect("nothing sniffs it"),
    };
    let depth: usize = std::env::args().nth(3).and_then(|d| d.parse().ok()).unwrap_or(4);
    println!("template: {name}");
    let doc = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(formats::builtin(name).unwrap());
    walk(&mut ev, &doc, &[], 0, depth);
}

fn walk(ev: &mut Evaluator, doc: &Document<MemSource>, path: &[usize], depth: usize, max: usize) {
    if depth > max {
        return;
    }
    let n = match ev.node(doc, path) {
        Ok(n) => n,
        Err(e) => {
            println!("{:indent$}{path:?} ERROR {e:?}", "", indent = depth * 2);
            return;
        }
    };
    println!(
        "{:indent$}{path:?} {} : {} = {:?} ({} bits at {:x})",
        "",
        n.name,
        n.type_name,
        n.value,
        n.size_bits,
        n.offset_bits / 8,
        indent = depth * 2
    );
    for i in 0..n.child_count.min(80) as usize {
        let mut p = path.to_vec();
        p.push(i);
        walk(ev, doc, &p, depth + 1, max);
    }
}
