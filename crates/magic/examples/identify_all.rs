//! Run the file(1) rules over every file under a folder and print one line
//! each: the path, then what the rules said. For diffing one build of the
//! rules against another, or against `file -b` itself.
//!
//!   cargo run --release -p qubero-magic --example identify_all -- <folder>

use std::path::Path;

fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
            continue;
        }
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn main() {
    let root = std::env::args().nth(1).expect("folder to read");
    let mut files = Vec::new();
    walk(Path::new(&root), &mut files);
    for p in files {
        let Ok(bytes) = std::fs::read(&p) else { continue };
        let head = &bytes[..bytes.len().min(64 * 1024)];
        let json = qubero_magic::identify(head);
        let message = json
            .split_once("\"message\":\"")
            .map(|(_, rest)| rest.split("\",\"mime\"").next().unwrap_or(""))
            .unwrap_or("");
        let rel = p.strip_prefix(&root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
        println!("{rel}\t{message}");
    }
}
