//! The `._` files a zip made on a Mac leaves beside its copies, which are
//! too varied to keep in the repository: every one under the directories
//! `QUBERO_SAMPLES` names (several, separated by `;`). Skips when there is
//! none.
//!
//! What this checks is that the template reads what the archive tools
//! wrote, not only what the writer on macOS writes: every field of every
//! file must resolve, and the parts must sit where the table at the front
//! says they sit.

use std::path::{Path, PathBuf};

use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats::{appledouble, sniff};
use qubero_core::source::MemSource;

#[test]
fn reads_real_files_end_to_end() {
    let mut found = Vec::new();
    if let Ok(extra) = std::env::var("QUBERO_SAMPLES") {
        for dir in extra.split(';').filter(|s| !s.is_empty()) {
            collect(&PathBuf::from(dir), 6, &mut found);
        }
    }
    if found.is_empty() {
        eprintln!("skipped: no ._* file in hand. Set QUBERO_SAMPLES to a directory holding one.");
        return;
    }
    found.sort();
    let mut checked = 0;
    for path in found {
        check(&path, &mut checked);
    }
    assert!(checked > 0, "no AppleDouble file among the ._ files");
}

fn collect(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            if depth > 0 {
                collect(&path, depth - 1, out);
            }
        } else if path.file_name().is_some_and(|n| n.to_string_lossy().starts_with("._")) {
            out.push(path);
        }
    }
}

fn check(path: &Path, checked: &mut usize) {
    let bytes = std::fs::read(path).expect("reads");
    let len = bytes.len() as u64;
    // The name `._` is a habit, not a format: anything may unpack into one.
    // A file the sniffer does not claim is passed over rather than failed.
    if sniff(&bytes[..8192.min(bytes.len())], len) != Some("appledouble") {
        eprintln!("--- {}: not claimed by the sniffer, passed over", path.display());
        return;
    }
    *checked += 1;
    let d = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(appledouble());
    let mut seen = 0;
    walk(&mut ev, &d, &[], &mut seen, path);
    eprintln!("--- {}: {len} bytes, {seen} nodes read", path.display());
}

/// Every node under `path`, in order, failing on the first that cannot be
/// read. An attribute whose pointer misses its data is an error here
/// rather than a plausible-looking gap.
fn walk(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], seen: &mut usize, file: &Path) {
    if *seen >= 10_000 {
        return;
    }
    let n = ev
        .node(d, path)
        .unwrap_or_else(|e| panic!("{}: {path:?} does not read: {e:?}", file.display()))
        .child_count;
    *seen += 1;
    for i in 0..n as usize {
        let mut p = path.to_vec();
        p.push(i);
        walk(ev, d, &p, seen, file);
        if *seen >= 10_000 {
            return;
        }
    }
}
