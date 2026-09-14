//! Read one field of a file with nothing asked before it, on a thread with a
//! stack of a given size:
//! `cargo run --release --example cold_read -- <file> <KiB> <step>...`
//!
//! A step is a field name, an element index, or `last` for the last element.
//! A name that is not a child of the node reached goes through the one thing
//! that node holds first, which is how a pointer or a stream is stepped
//! through. Finding the way reads only the nodes on the way down; the field at
//! the end is what is read cold.
//!
//! Prints what came back, how many expressions were open inside one another
//! at most, and how long the read took. A stack that runs out takes the
//! process with it, which is the answer too.
use std::time::Instant;

use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats;
use qubero_core::source::MemSource;

fn main() {
    let mut args = std::env::args().skip(1);
    let file = args.next().expect("usage: cold_read <file> <KiB> <step>...");
    let kib: usize = args.next().and_then(|k| k.parse().ok()).expect("a stack size in KiB");
    let steps: Vec<String> = args.collect();
    let h = std::thread::Builder::new()
        .stack_size(kib << 10)
        .spawn(move || {
            let bytes = std::fs::read(&file).expect("read");
            let head = &bytes[..bytes.len().min(formats::SNIFF_WINDOW)];
            let name = formats::sniff(head, bytes.len() as u64).expect("no template matches");
            let doc = Document::new(MemSource(bytes));
            let mut ev = Evaluator::new(formats::template(name).expect("template"));
            let mut path: Vec<usize> = Vec::new();
            for step in &steps {
                if let Ok(i) = step.parse::<usize>() {
                    path.push(i);
                } else if step == "last" {
                    let n = ev.node(&doc, &path).expect("the list").child_count;
                    path.push(n as usize - 1);
                } else {
                    match ev.child_named(&doc, &path, step).expect("a way down") {
                        Some(p) => path = p,
                        None => {
                            path.push(0);
                            path = ev.child_named(&doc, &path, step).expect("a way down").unwrap_or_else(|| panic!("no {step}"));
                        }
                    }
                }
            }
            let started = Instant::now();
            let got = ev.node(&doc, &path);
            let took = started.elapsed();
            println!("template {name}, path {path:?}");
            println!("{:?}", got.map(|n| (n.name, n.value)));
            println!("deepest: {}, {:.3} ms", ev.deepest_question(), took.as_secs_f64() * 1000.0);
        })
        .unwrap();
    let _ = h.join();
}
