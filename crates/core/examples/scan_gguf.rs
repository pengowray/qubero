//! What it costs to open a real GGUF, and what its data section reads as:
//! `cargo run --release --example scan_gguf -- path/to/model.gguf`
//!
//! Prints how long each part of the file takes to place, how many nodes are
//! left in memory afterwards, and the first and last few tensors with the
//! offsets and sizes the template gives them. A file of a few gigabytes is the
//! point: the numbers here are what the browser has to do on open.

use std::cell::RefCell;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::time::Instant;

use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats::gguf;
use qubero_core::source::{Missing, Source};

/// Reads straight from the file. The browser reads in chunks and waits for the
/// ones it has not got yet; this one always has them, so the timings here are
/// the work itself without the fetching.
struct FileSource {
    file: RefCell<BufReader<File>>,
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
    println!("{path}: {len} bytes");
    let doc = Document::new(FileSource { file: RefCell::new(BufReader::new(file)), len });
    let mut ev = Evaluator::new(gguf());

    // The metadata is the expensive half: entries of uneven size, some holding
    // arrays of a million strings, and the tensor table cannot be placed until
    // the end of it is known.
    let t = Instant::now();
    let md = ev.node(&doc, &[4]).expect("metadata");
    println!("  metadata: {} entries in {:?}", md.child_count, t.elapsed());
    let mut elements = 0u64;
    for i in 0..md.child_count as usize {
        // metadata[i].value.items, when the value is an array.
        if let Ok(items) = ev.node(&doc, &[4, i, 2, 2]) {
            if items.child_count > 1000 {
                let entry = ev.node(&doc, &[4, i]).expect("entry");
                println!("    {} holds {} items", entry.name, items.child_count);
            }
            elements += items.child_count;
        }
    }
    println!("    {elements} array elements in all, {} nodes kept", ev.memo_len());

    let t = Instant::now();
    let tensors = ev.node(&doc, &[5]).expect("tensors");
    println!("  tensor table: {} records in {:?}", tensors.child_count, t.elapsed());

    let t = Instant::now();
    let data = ev.node(&doc, &[6]).expect("data");
    println!("  data: {} children in {:?}", data.child_count, t.elapsed());

    let n = data.child_count;
    for i in (0..n.min(4)).chain(n.saturating_sub(2)..n) {
        let t = Instant::now();
        let c = ev.node(&doc, &[6, i as usize]).expect("child");
        println!("    {:<44} 0x{:>10x} {:>14} bytes ({:?})", c.name, c.offset_bits / 8, c.size_bits / 8, t.elapsed());
    }
    // What the hex view asks when the cursor lands in the middle of a long
    // list: the field under a bit, found without the list being all in memory.
    let mid = md.offset_bits + md.size_bits / 2;
    let t = Instant::now();
    match ev.locate(&doc, mid) {
        Ok(path) => {
            let elapsed = t.elapsed();
            let node = ev.node(&doc, &path).expect("located");
            println!("  locate at 0x{:x}: {} ({:?})", mid / 8, node.name, elapsed);
        }
        Err(e) => println!("  locate at 0x{:x} failed: {e:?}", mid / 8),
    }
    println!("  {} nodes kept in all", ev.memo_len());
}
