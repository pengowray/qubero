//! Walk a real GGUF's data section from the command line:
//! `cargo run --example scan_gguf -- path/to/model.gguf`
//!
//! Prints the tensor count, the first and last few data children with their
//! names, offsets and sizes, and how long the whole walk took.

use std::cell::RefCell;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::time::Instant;

use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats::gguf;
use qubero_core::source::{Missing, Source};

struct FileSource {
    file: RefCell<File>,
    len: u64,
}

impl Source for FileSource {
    fn len_bytes(&self) -> u64 {
        self.len
    }
    fn read_bytes(&self, offset: u64, out: &mut [u8]) -> Vec<Missing> {
        let mut f = self.file.borrow_mut();
        f.seek(SeekFrom::Start(offset)).expect("seek");
        f.read_exact(out).expect("read");
        Vec::new()
    }
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: scan_gguf <file.gguf>");
    let file = File::open(&path).expect("open");
    let len = file.metadata().expect("metadata").len();
    let doc = Document::new(FileSource { file: RefCell::new(file), len });
    let mut ev = Evaluator::new(gguf());

    let t0 = Instant::now();
    let md = ev.node(&doc, &[4]).expect("metadata");
    println!("metadata: {} entries in {:?}, {} memo entries", md.child_count, t0.elapsed(), ev.memo_len());
    let mut total = 0u64;
    for i in 0..md.child_count as usize {
        // metadata[i].value.items, when the value is an array.
        if let Ok(items) = ev.node(&doc, &[4, i, 2, 2]) {
            if items.child_count > 1000 {
                let k = ev.node(&doc, &[4, i]).expect("entry");
                println!("  {} holds {} items", k.name, items.child_count);
            }
            total += items.child_count;
        }
    }
    println!("  {total} array elements in the metadata");
    let t1 = Instant::now();
    let ts = ev.node(&doc, &[5]).expect("tensors");
    println!("tensor table: {} records in {:?}", ts.child_count, t1.elapsed());
    let start = Instant::now();
    let data = ev.node(&doc, &[6]).expect("data node");
    println!("{path}: {} bytes, data section {} children, sized in {:?}", len, data.child_count, start.elapsed());

    let n = data.child_count;
    let show: Vec<u64> = (0..n.min(4)).chain(n.saturating_sub(2)..n).collect();
    for i in show {
        let t = Instant::now();
        let c = ev.node(&doc, &[6, i as usize]).expect("child");
        println!(
            "  {:<40} offset 0x{:>10x}  {:>14} bytes  ({:?})",
            c.name,
            c.offset_bits / 8,
            c.size_bits / 8,
            t.elapsed()
        );
    }
    let t = Instant::now();
    let spans = ev.spans(&doc, data.offset_bits, data.offset_bits + 4096 * 8, 20).expect("spans");
    println!("  {} spans at the data start in {:?}:", spans.len(), t.elapsed());
    for s in spans.iter().take(6) {
        println!("    {:<40} 0x{:>10x} {:>14} bytes{}", s.name, s.offset_bits / 8, s.size_bits / 8, if s.gap { "  (gap)" } else { "" });
    }
}
