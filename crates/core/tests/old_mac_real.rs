//! Optional end-to-end checks over old-Mac files in `QUBERO_SAMPLES`.

use std::path::{Path, PathBuf};

use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats::{builtin, sniff};
use qubero_core::source::MemSource;

#[test]
fn reads_real_old_mac_containers_end_to_end() {
    let Ok(extra) = std::env::var("QUBERO_SAMPLES") else {
        eprintln!("skipped: set QUBERO_SAMPLES to old-Mac sample directories");
        return;
    };
    let mut found = Vec::new();
    for dir in extra.split(';').filter(|s| !s.is_empty()) {
        collect(&PathBuf::from(dir), 6, &mut found);
    }
    let mut checked = 0;
    for path in found {
        let bytes = std::fs::read(&path).expect("reads");
        let Some(name @ ("macbinary" | "binhex" | "stuffit" | "compactpro")) =
            sniff(&bytes[..8192.min(bytes.len())], bytes.len() as u64)
        else {
            continue;
        };
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(builtin(name).expect("built in"));
        let mut seen = 0;
        walk(&mut ev, &d, &[], &mut seen, &path);
        eprintln!("--- {}: {name}, {seen} nodes read", path.display());
        checked += 1;
    }
    assert!(checked > 0, "no recognized old-Mac container in QUBERO_SAMPLES");
}

fn collect(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && depth > 0 {
            collect(&path, depth - 1, out);
        } else if path.is_file() {
            out.push(path);
        }
    }
}

fn walk(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], seen: &mut usize, file: &Path) {
    if *seen >= 100_000 { return; }
    let n = ev.node(d, path).unwrap_or_else(|e| panic!("{}: {path:?}: {e:?}", file.display())).child_count;
    *seen += 1;
    for i in 0..n as usize {
        let mut child = path.to_vec();
        child.push(i);
        walk(ev, d, &child, seen, file);
    }
}
