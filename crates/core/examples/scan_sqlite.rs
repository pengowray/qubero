//! What it costs to open a real database, and what a page in the middle of one
//! reads as: `cargo run --release --example scan_sqlite -- path/to/file.db`
//!
//! A database is a run of same-sized pages, so the interesting numbers are how
//! long the run takes to count, what reaching a page near the end costs, and
//! what the cursor landing in the middle of one costs. Those are what the
//! browser does when someone drags the scrollbar.

use std::cell::RefCell;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::time::Instant;

use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats::sqlite;
use qubero_core::source::{Missing, Source};

struct FileSource {
    file: RefCell<File>,
    len: u64,
    reads: RefCell<u64>,
}

impl Source for FileSource {
    fn len_bytes(&self) -> u64 {
        self.len
    }
    fn read_bytes(&self, offset: u64, out: &mut [u8]) -> Vec<Missing> {
        *self.reads.borrow_mut() += 1;
        let mut f = self.file.borrow_mut();
        f.seek(SeekFrom::Start(offset)).expect("seek");
        f.read_exact(out).expect("read");
        Vec::new()
    }
}

/// Field indices into the root struct, and into a b-tree page.
const PAGE_SIZE: usize = 1;
const PAGES: usize = 24;
const BODY: usize = 1;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: scan_sqlite <file.db>");
    let file = File::open(&path).expect("open");
    let len = file.metadata().expect("metadata").len();
    let doc = Document::new(FileSource {
        file: RefCell::new(file),
        len,
        reads: RefCell::new(0),
    });
    let mut ev = Evaluator::new(sqlite());

    let page_size = ev
        .node(&doc, &[PAGE_SIZE])
        .expect("page size")
        .value
        .as_int()
        .unwrap_or(0);
    println!(
        "{path}: {len} bytes, {page_size}-byte pages, {} pages",
        len as i128 / page_size.max(1)
    );

    let t = Instant::now();
    let pages = ev.node(&doc, &[PAGES]).expect("pages");
    println!(
        "  counting the pages: {} in {:?} ({} nodes)",
        pages.child_count,
        t.elapsed(),
        ev.memo_len()
    );

    let last = pages.child_count as usize - 1;
    for i in [1usize, last / 2, last] {
        let t = Instant::now();
        let kind = ev.node(&doc, &[PAGES, i, 0]).expect("page type");
        println!(
            "  page {i}: {} at {:#x} in {:?}",
            kind.value.as_int().unwrap_or(0),
            kind.offset_bits / 8,
            t.elapsed()
        );
    }

    // The cursor dropped into the middle of the file: which field is it in?
    let bit = (len / 2) * 8;
    let t = Instant::now();
    match ev.locate(&doc, bit) {
        Ok(p) => println!("  locate at {:#x}: {p:?} in {:?}", bit / 8, t.elapsed()),
        Err(e) => println!("  locate at {:#x}: {e:?} in {:?}", bit / 8, t.elapsed()),
    }

    // What the annotation column would draw for a screenful there.
    let t = Instant::now();
    match ev.spans(&doc, bit, bit + 4096 * 8, 64) {
        Ok(s) => println!("  {} spans across a page in {:?}", s.len(), t.elapsed()),
        Err(e) => println!("  spans: {e:?} in {:?}", t.elapsed()),
    }

    let t = Instant::now();
    let cells = ev.node(&doc, &[PAGES, last / 2, BODY, 5]);
    match cells {
        Ok(c) => println!(
            "  cells on the middle page: {} in {:?}",
            c.child_count,
            t.elapsed()
        ),
        Err(e) => println!("  cells on the middle page: {e:?} in {:?}", t.elapsed()),
    }
    println!("  {} nodes in memory at the end", ev.memo_len());
}
