//! What template each file under a directory is, one line per file, as
//! tab-separated text. Written for keeping an index of a collection of sample
//! files: the collection stays outside this repository, and this says what
//! Qubero makes of each thing in it.

use std::path::Path;

fn main() {
    let root = std::env::args().nth(1).expect("usage: sniff_list <dir>");
    let root = Path::new(&root);
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    for line in out {
        println!("{line}");
    }
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, out);
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else { continue };
        // Enough of the file for the tests that read a header the front of the
        // file only points at: an ISO keeps its first descriptor at 0x8000.
        let head = &bytes[..bytes.len().min(qubero_core::formats::SNIFF_WINDOW)];
        let name = qubero_core::formats::sniff(head, bytes.len() as u64).unwrap_or("-");
        let rel = path.strip_prefix(root).unwrap_or(&path).display().to_string().replace('\\', "/");
        out.push(format!("{rel}\t{}\t{name}", bytes.len()));
    }
}
