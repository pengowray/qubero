//! What an HDF5 file holds, read the way the app reads it:
//! `cargo run --release --example scan_hdf5 -- path/to/file.h5 [depth]`
//!
//! The point is the walk. Every object in the file is reached by address, so
//! the tree this prints is the template following one pointer after another,
//! and how much of the file it had to read to do it is the number at the end.

use std::cell::RefCell;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::time::Instant;

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats::hdf5;
use qubero_core::source::{Missing, Source};

struct FileSource {
    file: RefCell<File>,
    len: u64,
    reads: RefCell<u64>,
    bytes: RefCell<u64>,
}

impl Source for FileSource {
    fn len_bytes(&self) -> u64 {
        self.len
    }
    fn read_bytes(&self, offset: u64, out: &mut [u8]) -> Vec<Missing> {
        *self.reads.borrow_mut() += 1;
        *self.bytes.borrow_mut() += out.len() as u64;
        let mut f = self.file.borrow_mut();
        f.seek(SeekFrom::Start(offset)).expect("seek");
        let end = (offset + out.len() as u64).min(self.len);
        let n = (end.saturating_sub(offset)) as usize;
        f.read_exact(&mut out[..n]).expect("read");
        Vec::new()
    }
}

fn show(v: &Value) -> String {
    match v {
        Value::UInt(n) => n.to_string(),
        Value::Int(n) => n.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Str(s) => format!("{s:?}"),
        Value::Bytes { len, .. } => format!("{len} bytes"),
        Value::Unread { len } => format!("{len} bytes, unread"),
        Value::Magic { ok, bytes } => format!("{}{}", String::from_utf8_lossy(bytes), if *ok { "" } else { " (wrong)" }),
        Value::Enum { raw, name, .. } => match name {
            Some(n) => format!("{n} ({raw})"),
            None => raw.to_string(),
        },
        Value::Flags { set, .. } => set.join(", "),
        Value::Composite { .. } => String::new(),
    }
}

fn dump(ev: &mut Evaluator, doc: &Document<FileSource>, path: &[usize], depth: usize, limit: usize) {
    let node = match ev.node(doc, path) {
        Ok(n) => n,
        Err(e) => {
            println!("{:indent$}<{e:?}>", "", indent = depth * 2);
            // A list whose total size failed still has elements, and which of
            // them went wrong is the thing worth seeing.
            for i in 0..64 {
                let mut p = path.to_vec();
                p.push(i);
                match ev.node(doc, &p) {
                    Ok(n) => println!("{:indent$}[{i}] {} @{}", "", n.type_name, n.offset_bits / 8, indent = depth * 2 + 2),
                    Err(e) => {
                        println!("{:indent$}[{i}] <{e:?}>", "", indent = depth * 2 + 2);
                        break;
                    }
                }
            }
            return;
        }
    };
    let value = show(&node.value);
    let gap = if value.is_empty() { "" } else { " = " };
    println!(
        "{:indent$}{} : {}{gap}{value}   @{}",
        "",
        node.name,
        node.type_name,
        node.offset_bits / 8,
        indent = depth * 2
    );
    if depth >= limit || node.child_count == 0 {
        return;
    }
    let count = node.child_count.min(64);
    for i in 0..count {
        let mut p = path.to_vec();
        p.push(i as usize);
        dump(ev, doc, &p, depth + 1, limit);
    }
    if node.child_count > count {
        println!("{:indent$}... {} more", "", node.child_count - count, indent = (depth + 1) * 2);
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: scan_hdf5 <file.h5> [depth]");
    let limit: usize = args.next().map(|d| d.parse().expect("depth")).unwrap_or(8);
    let file = File::open(&path).expect("open");
    let len = file.metadata().expect("metadata").len();
    let doc = Document::new(FileSource {
        file: RefCell::new(file),
        len,
        reads: RefCell::new(0),
        bytes: RefCell::new(0),
    });
    let mut ev = Evaluator::new(hdf5());
    println!("{path}: {len} bytes");
    // With a third argument, what the views would say about the byte at that
    // offset: which field covers it, and what a screenful of spans from there
    // reads as. That is the question the hex grid asks, and the answer for a
    // format whose bytes are all reached through pointers is not obvious.
    if let Some(at) = std::env::args().nth(3).map(|s| s.parse::<u64>().expect("offset")) {
        // The index of placed stretches is built a go at a time, so a large
        // file answers "nothing covers this yet" until the walk reaches it.
        // Asking again is what the app does when the cursor moves.
        let probe = Instant::now();
        for go in 1..=40 {
            match ev.locate(&doc, at * 8) {
                Ok(p) if !p.is_empty() => {
                    println!("  found after {go} goes, {:?}", probe.elapsed());
                    break;
                }
                Ok(_) => println!("  go {go}: nothing covers it yet ({:?})", probe.elapsed()),
                Err(e) => {
                    println!("  go {go}: {e:?}");
                    break;
                }
            }
        }
        match ev.locate(&doc, at * 8) {
            Ok(p) => {
                let n = ev.node(&doc, &p).expect("node");
                println!("locate({at}) -> {p:?} {} : {} @{}", n.name, n.type_name, n.offset_bits / 8);
            }
            Err(e) => println!("locate({at}) -> {e:?}"),
        }
        match ev.spans(&doc, at * 8, at * 8 + 16 * 8, 8) {
            Ok(spans) => {
                for s in spans {
                    println!("  span {:?}", s);
                }
            }
            Err(e) => println!("spans -> {e:?}"),
        }
        println!("  locate took {:?}", probe.elapsed());
        println!(
            "  {} reads, {} bytes read, {} memo entries",
            doc.source().reads.borrow(),
            doc.source().bytes.borrow(),
            ev.memo_len()
        );
        return;
    }
    let start = Instant::now();
    dump(&mut ev, &doc, &[], 0, limit);
    println!(
        "\nwalked in {:?}, {} reads, {} bytes read, {} memo entries",
        start.elapsed(),
        doc.source().reads.borrow(),
        doc.source().bytes.borrow(),
        ev.memo_len()
    );
}
