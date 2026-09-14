//! Read one field of a file with nothing asked before it, on a thread with a
//! stack of a given size:
//! `cargo run --release --example cold_read -- <file> <KiB> [paint] <step>...`
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
//!
//! With `paint`, the stack below the read is filled with a known byte first,
//! and how much of it the read wrote over is printed as `stack: <bytes>`, the
//! way `stack_probe` does it. The size has to be larger than `PAINT`.
use std::time::Instant;

use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats;
use qubero_core::source::MemSource;

/// How much of the stack `paint` fills, in bytes.
const PAINT: usize = 6 << 20;

/// The byte the stack is filled with.
const MARK: u8 = 0xA5;

/// Fill `PAINT` bytes of the stack below the caller with `MARK`, and say
/// where they start and end.
#[inline(never)]
fn paint() -> (usize, usize) {
    let mut block = [0u8; PAINT];
    block.fill(MARK);
    let low = std::hint::black_box(&mut block).as_ptr() as usize;
    (low, low + PAINT)
}

/// How many bytes at the top of what `paint` filled no longer hold `MARK`.
#[inline(never)]
fn written_over(low: usize, high: usize) -> usize {
    let mut at = low;
    // SAFETY: the bytes are the stack this thread committed for `paint`, and
    // nothing has been freed; they are only read.
    while at < high && unsafe { std::ptr::read_volatile(at as *const u8) } == MARK {
        at += 1;
    }
    high - at
}

fn main() {
    let mut args = std::env::args().skip(1).peekable();
    let file = args.next().expect("usage: cold_read <file> <KiB> [paint] <step>...");
    let kib: usize = args.next().and_then(|k| k.parse().ok()).expect("a stack size in KiB");
    let painting = args.next_if(|a| a == "paint").is_some();
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
            let painted = painting.then(paint);
            let started = Instant::now();
            let got = ev.node(&doc, &path);
            let took = started.elapsed();
            if let Some((low, high)) = painted {
                println!("stack: {}", written_over(low, high));
            }
            println!("template {name}, path {path:?}");
            println!("{:?}", got.map(|n| (n.name, n.value)));
            println!("deepest: {}, {:.3} ms", ev.deepest_question(), took.as_secs_f64() * 1000.0);
        })
        .unwrap();
    let _ = h.join();
}
