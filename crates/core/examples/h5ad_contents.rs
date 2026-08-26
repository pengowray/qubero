//! What an HDF5 file holds, in its own terms:
//! `cargo run --release --example h5ad_contents -- path/to/file.h5ad`
//!
//! The contents list the app shows beside the byte map, printed on its own so
//! that what it says about a real file can be checked against what the file
//! says about itself.

use std::cell::RefCell;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::time::Instant;

use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats::h5ad::{self, Storage};
use qubero_core::formats::hdf5;
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
        if f.seek(SeekFrom::Start(offset)).is_err() || f.read_exact(out).is_err() {
            out.fill(0);
        }
        Vec::new()
    }
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: h5ad_contents <file.h5ad>");
    let file = File::open(&path).expect("open");
    let len = file.metadata().expect("metadata").len();
    let doc = Document::new(FileSource { file: RefCell::new(file), len });
    let mut ev = Evaluator::new(hdf5());

    let start = Instant::now();
    let contents = h5ad::contents(&mut ev, &doc).expect("contents");
    println!("{path}: {len} bytes");
    if contents.anndata {
        println!(
            "AnnData ({}) · {} rows × {} columns",
            if contents.encoding.is_empty() { "by what it holds" } else { "by what it says" },
            contents.rows,
            contents.columns
        );
    }
    for object in &contents.objects {
        let shape = if object.shape.is_empty() {
            String::new()
        } else {
            object.shape.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(" × ")
        };
        let storage = match &object.storage {
            Storage::None => String::new(),
            Storage::Contiguous(n) => format!("{n} bytes in one run"),
            Storage::Compact(n) => format!("{n} bytes in the header"),
            Storage::Chunked { dims, filters } => {
                let dims = dims.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(" × ");
                let filters = if filters.is_empty() { String::new() } else { format!(", {}", filters.join(" then ")) };
                format!("chunks of {dims}{filters}")
            }
        };
        println!(
            "{:<44} {:<12} {:<16} {:<10} {storage}",
            object.name,
            object.encoding,
            shape,
            object.element
        );
    }
    println!(
        "\n{} objects ({} shown) in {:?}",
        contents.total,
        contents.objects.len(),
        start.elapsed()
    );
}
