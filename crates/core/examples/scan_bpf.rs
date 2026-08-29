//! Walk an eBPF object and print its tree, for checking the template against a
//! real file.

use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats;
use qubero_core::source::MemSource;

fn main() {
    let path = std::env::args().nth(1).expect("usage: scan_bpf <file>");
    let bytes = std::fs::read(&path).unwrap();
    let name = formats::sniff(&bytes[..bytes.len().min(formats::SNIFF_WINDOW)], bytes.len() as u64).unwrap_or("elf");
    println!("template: {name}");
    let doc = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(formats::builtin(name).unwrap());
    if std::env::args().any(|a| a == "--tree") {
        walk(&mut ev, &doc, &[], 0);
    }
    let p = formats::ElfProgram::read(&mut ev, &doc).unwrap();
    for s in &p.sections {
        println!("section {:16} type {:3} {:6} bytes at {}", s.name, s.kind, s.size, s.offset);
    }
    for s in &p.symbols {
        println!("symbol  {:16} type {} in section {} at {}", s.name, s.kind, s.section, s.value);
    }
    println!("{}", p.listing(&mut ev, &doc).unwrap());
}

fn walk(ev: &mut Evaluator, doc: &Document<MemSource>, path: &[usize], depth: usize) {
    if depth > 7 {
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
        "{:indent$}{path:?} {} : {} = {:?} ({} bits at {})",
        "",
        n.name,
        n.type_name,
        n.value,
        n.size_bits,
        n.offset_bits / 8,
        indent = depth * 2
    );
    for i in 0..n.child_count.min(40) as usize {
        let mut p = path.to_vec();
        p.push(i);
        walk(ev, doc, &p, depth + 1);
    }
}
