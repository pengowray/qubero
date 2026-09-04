//! What the annotation column would show, for one window or over a whole
//! collection: `cargo run --example spans_probe -- <file-or-dir> [start] [bytes] [max]`
//!
//! A directory sweeps every file in it and prints one line each: the template,
//! how many spans came back for the whole file, and how many bytes of it they
//! name. Two runs of that can be diffed to see what a change to `spans` did.

use std::fs;
use std::path::Path;

use qubero_core::document::Document;
use qubero_core::eval::{EvalError, Evaluator};
use qubero_core::source::MemSource;

fn template_of(bytes: &[u8]) -> Option<&'static str> {
    let head = &bytes[..bytes.len().min(qubero_core::formats::SNIFF_WINDOW)];
    qubero_core::formats::sniff(head, bytes.len() as u64)
}

fn spans_of(bytes: Vec<u8>, name: &str, start: u64, count: u64, max: usize) -> Result<Vec<qubero_core::eval::Span>, String> {
    let d = Document::new(MemSource(bytes));
    let Some(t) = qubero_core::formats::builtin(name) else { return Err("no builtin".into()) };
    let mut ev = Evaluator::new(t);
    ev.set_slice(Some(5_000));
    for _ in 0..200 {
        ev.begin_slice();
        match ev.spans(&d, start * 8, (start + count) * 8, max) {
            Ok(v) => return Ok(v),
            Err(EvalError::Busy { .. }) => {}
            Err(e) => return Err(format!("{e:?}")),
        }
    }
    Err("never settled".into())
}

fn sweep(root: &Path, dir: &Path, max: usize, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            sweep(root, &path, max, out);
            continue;
        }
        let Ok(bytes) = fs::read(&path) else { continue };
        let rel = path.strip_prefix(root).unwrap_or(&path).display().to_string().replace('\\', "/");
        let len = bytes.len() as u64;
        let Some(name) = template_of(&bytes) else {
            out.push(format!("{rel}\t-\t-\t-"));
            continue;
        };
        match spans_of(bytes, name, 0, len, max) {
            Ok(v) => {
                let named: u64 = v.iter().filter(|s| !s.gap).map(|s| s.size_bits / 8).sum();
                out.push(format!("{rel}\t{name}\t{}\t{named}", v.len()));
            }
            Err(e) => out.push(format!("{rel}\t{name}\terr\t{e}")),
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: spans_probe <file-or-dir> [start] [bytes] [max]");
    let start: u64 = args.next().map(|s| s.parse().unwrap()).unwrap_or(0);
    let count: Option<u64> = args.next().map(|s| s.parse().unwrap());
    let max: usize = args.next().map(|s| s.parse().unwrap()).unwrap_or(600);
    let path = Path::new(&path);
    if path.is_dir() {
        let mut out = Vec::new();
        sweep(path, path, max.max(4000), &mut out);
        out.sort();
        for line in out {
            println!("{line}");
        }
        return;
    }
    let bytes = fs::read(path).expect("read");
    let len = bytes.len() as u64;
    let name = template_of(&bytes).expect("no template");
    println!("template {name}");
    match spans_of(bytes, name, start, count.unwrap_or(len), max) {
        Err(e) => println!("{e}"),
        Ok(v) => {
            println!("{} spans", v.len());
            for s in &v {
                println!(
                    "  {:#08x} +{:<8} {}{}",
                    s.offset_bits / 8,
                    s.size_bits / 8,
                    if s.gap { "(gap) " } else { "" },
                    s.name
                );
            }
        }
    }
}
