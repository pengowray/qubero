//! Print a file's tree as the template reads it: path, name, type, value.
//! A second argument names the template for a file nothing sniffs.

use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats;
use qubero_core::source::MemSource;

fn main() {
    // A thread of its own, because the main one on Windows starts with a
    // megabyte and a debug build's frames are fat. A file nested as deep as
    // the evaluator allows should come back with an error, not take the tool
    // down on the way to one.
    std::thread::Builder::new().stack_size(8 << 20).spawn(run).unwrap().join().unwrap();
}

fn run() {
    let path = std::env::args().nth(1).expect("usage: dump_tree <file> [template] [depth]");
    let bytes = std::fs::read(&path).unwrap();
    let name = match std::env::args().nth(2) {
        Some(name) => Box::leak(name.into_boxed_str()) as &str,
        None => formats::sniff(&bytes[..bytes.len().min(formats::SNIFF_WINDOW)], bytes.len() as u64).expect("nothing sniffs it"),
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
    // What this field is machinery for, and what the template says about it,
    // so the listing's folding can be checked without a browser.
    let owner = match (n.consumed_by, n.machinery) {
        (None, None) => String::new(),
        (c, m) => {
            let mut parts = Vec::new();
            if let Some(i) = c {
                parts.push(format!("places #{i}"));
            }
            match m {
                Some(true) => parts.push("hint:machinery".to_string()),
                Some(false) => parts.push("hint:payload".to_string()),
                None => {}
            }
            format!(" [{}]", parts.join(" "))
        }
    };
    println!(
        "{:indent$}{path:?} {} : {} = {:?} ({} bits at {:x}){owner}",
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
