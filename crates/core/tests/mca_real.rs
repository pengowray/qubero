//! The regions a world actually saves, which are too big to keep in the
//! repository: every `*.mca` under the directories `QUBERO_SAMPLES` names
//! (several, separated by `;`). Skips when there is none.
//!
//! What this checks is the thing a made-up region cannot: that the sniffer
//! claims a file nobody built for it, out of bytes that carry no magic
//! number, and that every pointer the region's two tables hold lands on a
//! chunk that reads.

use std::path::{Path, PathBuf};

use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats::{mca, sniff};
use qubero_core::source::MemSource;

#[test]
fn reads_real_regions_end_to_end() {
    let mut found = Vec::new();
    if let Ok(extra) = std::env::var("QUBERO_SAMPLES") {
        for dir in extra.split(';').filter(|s| !s.is_empty()) {
            collect(&PathBuf::from(dir), 3, &mut found);
        }
    }
    if found.is_empty() {
        eprintln!("skipped: no region file in hand. Set QUBERO_SAMPLES to a directory holding one.");
        return;
    }
    found.sort();
    let mut checked = 0;
    for path in found {
        check(&path, &mut checked);
    }
    assert!(checked > 0, "no region among the .mca files");
}

fn collect(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            if depth > 0 {
                collect(&path, depth - 1, out);
            }
        } else if path.extension().is_some_and(|x| x == "mca") {
            out.push(path);
        }
    }
}

fn check(path: &Path, checked: &mut usize) {
    let bytes = std::fs::read(path).expect("reads");
    let len = bytes.len() as u64;
    // The extension is not the format: the name `.mca` is taken by more than
    // one format, and only the shape of the tables says which file is theirs.
    // A file they do not claim is passed over rather than failed.
    let head = &bytes[..8192.min(bytes.len())];
    if sniff(head, len) != Some("mca") {
        eprintln!("--- {}: not claimed by the sniffer, passed over", path.display());
        return;
    }
    *checked += 1;

    let d = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(mca());
    assert_eq!(ev.node(&d, &[0]).unwrap().child_count, 1024);
    assert_eq!(ev.node(&d, &[1]).unwrap().child_count, 1024);

    // Every entry the tables generated lands on bytes that read as a chunk,
    // inside the file; every one they never generated covers nothing and
    // keeps its place all the same.
    assert_eq!(ev.node(&d, &[2]).unwrap().child_count, 1024);
    let mut generated = 0;
    for i in 0..1024usize {
        let at = ev.node(&d, &[0, i, 2]).unwrap().value.as_int().unwrap();
        let chunk = ev.node(&d, &[2, i]).unwrap();
        if at == 0 {
            assert_eq!(chunk.size_bits, 0, "{}: entry {i} is zero yet its chunk covers bytes", path.display());
            continue;
        }
        generated += 1;
        assert_eq!(chunk.offset_bits / 8, at as u64, "{}: chunk {i} not where its entry points", path.display());
        for f in 0..3 {
            ev.node(&d, &[2, i, f]).unwrap();
        }
        let end = chunk.offset_bits / 8 + chunk.size_bits / 8;
        assert!(end <= len, "{}: chunk {i} runs past the end", path.display());
    }
    assert!(generated > 0, "{}: a region with no chunk in it", path.display());
    eprintln!("--- {}: {len} bytes, {generated} chunks read", path.display());
}
