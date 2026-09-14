//! How much of a file the annotation column names, against how much the tree
//! covers: `cargo run --release --example cover_probe -- <file-or-dir>...`
//!
//! The tree is walked from the root the way the field tree opens it, and every
//! leaf in the file's own address space adds its extent. A decoded stream adds
//! its run as well, and is walked into for anything inside it that points back
//! out at the file. Overlaps count once. The spans are asked twice: once for
//! the whole file, and once in windows the size of a screenful, which is what
//! the hex view does. Where the tree names bytes the spans do not, something
//! placed those bytes that the spans never found.

use std::fs;
use std::path::Path;

use qubero_core::document::Document;
use qubero_core::eval::{EvalError, Evaluator};
use qubero_core::source::MemSource;

/// A screenful of the hex view, roughly.
const WINDOW: u64 = 2048;

fn template_of(bytes: &[u8]) -> Option<&'static str> {
    let head = &bytes[..bytes.len().min(qubero_core::formats::SNIFF_WINDOW)];
    qubero_core::formats::sniff(head, bytes.len() as u64)
}

/// How many bits the union of these stretches covers.
fn union(mut v: Vec<(u64, u64)>) -> u64 {
    v.sort();
    let mut total = 0;
    let mut end = 0;
    for (a, b) in v {
        let a = a.max(end);
        if b > a {
            total += b - a;
            end = b;
        }
    }
    total
}

fn tree(ev: &mut Evaluator, doc: &Document<MemSource>, path: &mut Vec<usize>, out: &mut Vec<(u64, u64)>, opened: &mut usize) {
    *opened += 1;
    if *opened > 3_000_000 {
        return;
    }
    let Ok(n) = ev.node(doc, path) else { return };
    if n.space == 0 && n.size_bits > 0 && (n.decoded || !n.composite || n.child_count == 0) {
        out.push((n.offset_bits, n.offset_bits + n.size_bits));
    }
    if !n.composite {
        return;
    }
    for i in 0..n.child_count as usize {
        path.push(i);
        tree(ev, doc, path, out, opened);
        path.pop();
    }
}

fn spans(ev: &mut Evaluator, doc: &Document<MemSource>, from: u64, to: u64) -> Result<Vec<(u64, u64)>, String> {
    for _ in 0..200 {
        ev.begin_slice();
        match ev.spans(doc, from, to, 1_000_000) {
            Ok(v) => {
                return Ok(v
                    .iter()
                    .filter(|s| !s.gap)
                    .map(|s| (s.offset_bits.max(from), (s.offset_bits + s.size_bits).min(to)))
                    .filter(|(a, b)| b > a)
                    .collect());
            }
            Err(EvalError::Busy { .. }) => {}
            Err(e) => return Err(format!("{e:?}")),
        }
    }
    Err("never settled".into())
}

fn probe(path: &Path, rel: &str) {
    // A cap on what is read at all, for sweeping the whole collection: the
    // tree walk opens every leaf, and a forty-megabyte archive is minutes.
    let cap: u64 = std::env::var("COVER_MAX_BYTES").ok().and_then(|s| s.parse().ok()).unwrap_or(u64::MAX);
    if fs::metadata(path).map_or(true, |m| m.len() > cap) {
        return;
    }
    let Ok(bytes) = fs::read(path) else { return };
    let len = bytes.len() as u64;
    let Some(name) = template_of(&bytes) else {
        println!("{rel}\t-");
        return;
    };
    let t = qubero_core::formats::template(name).unwrap();
    let doc = Document::new(MemSource(bytes));

    let mut ev = Evaluator::new(t.clone());
    let mut leaves = Vec::new();
    let mut opened = 0;
    tree(&mut ev, &doc, &mut Vec::new(), &mut leaves, &mut opened);
    let tree_bytes = union(leaves) / 8;

    let mut ev = Evaluator::new(t.clone());
    ev.set_slice(Some(5_000));
    let whole = match spans(&mut ev, &doc, 0, len * 8) {
        Ok(v) => (union(v) / 8).to_string(),
        Err(e) => e,
    };

    let mut ev = Evaluator::new(t);
    ev.set_slice(Some(5_000));
    let mut windowed = Vec::new();
    let mut trouble = None;
    let mut at = 0;
    while at < len {
        match spans(&mut ev, &doc, at * 8, (at + WINDOW).min(len) * 8) {
            Ok(v) => windowed.extend(v),
            Err(e) => trouble = Some(e),
        }
        at += WINDOW;
    }
    let windowed = match trouble {
        Some(e) => e,
        None => (union(windowed) / 8).to_string(),
    };
    println!("{rel}\t{name}\t{len}\ttree {tree_bytes}\tspans {whole}\twindows {windowed}");
}

fn sweep(root: &Path, dir: &Path, out: &mut Vec<(String, std::path::PathBuf)>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            sweep(root, &path, out);
            continue;
        }
        let rel = path.strip_prefix(root.parent().unwrap_or(root)).unwrap_or(&path).display().to_string().replace('\\', "/");
        out.push((rel, path));
    }
}

fn main() {
    for arg in std::env::args().skip(1) {
        let path = Path::new(&arg);
        let mut files = Vec::new();
        if path.is_dir() {
            sweep(path, path, &mut files);
        } else {
            files.push((arg.clone(), path.to_path_buf()));
        }
        files.sort();
        for (rel, p) in files {
            probe(&p, &rel);
        }
    }
}
