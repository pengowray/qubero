//! Which template each file in a directory tree is, counted up. For seeing
//! what a real collection of files holds and what nothing here reads.

use std::collections::BTreeMap;

use qubero_core::formats;

fn main() {
    let root = std::env::args().nth(1).expect("usage: sniff_tree <dir>");
    let mut tally: BTreeMap<String, usize> = BTreeMap::new();
    let mut unknown: Vec<String> = Vec::new();
    walk(std::path::Path::new(&root), &mut tally, &mut unknown);
    for (name, count) in &tally {
        println!("{count:6}  {name}");
    }
    println!("--- first unknown files:");
    for f in unknown.iter().take(25) {
        println!("  {f}");
    }
}

fn walk(dir: &std::path::Path, tally: &mut BTreeMap<String, usize>, unknown: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, tally, unknown);
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let head = &bytes[..bytes.len().min(4096)];
        match formats::sniff(head, bytes.len() as u64) {
            Some(name) => *tally.entry(name.to_string()).or_default() += 1,
            None => {
                *tally.entry("(none)".into()).or_default() += 1;
                let head: String = head.iter().take(4).map(|b| format!("{b:02x} ")).collect();
                unknown.push(format!("{head} {}", path.display()));
            }
        }
    }
}
