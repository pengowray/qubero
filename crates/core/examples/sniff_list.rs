//! What template each file under a directory is, one line per file, as
//! tab-separated text. Written for keeping an index of a collection of sample
//! files: the collection stays outside this repository, and this says what
//! Qubero makes of each thing in it.
//!
//! Named files work too, which is how a single answer is asked for.

use std::io::Read;
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    assert!(!args.is_empty(), "usage: sniff_list <dir|file>...");
    let mut out = Vec::new();
    for arg in &args {
        let path = Path::new(arg);
        if path.is_dir() {
            walk(path, path, &mut out);
        } else {
            let parent = path.parent().unwrap_or(Path::new(""));
            one(parent, path, &mut out);
        }
    }
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
        one(root, &path, out);
    }
}

/// One file's answer. Only the head is read: a disc image is seven hundred
/// megabytes and the question is about its first few sectors.
fn one(root: &Path, path: &Path, out: &mut Vec<String>) {
    let Ok(len) = std::fs::metadata(path).map(|m| m.len()) else { return };
    let Ok(mut file) = std::fs::File::open(path) else { return };
    let mut head = vec![0u8; (len as usize).min(qubero_core::formats::SNIFF_WINDOW)];
    // Enough of the file for the tests that read a header the front of the
    // file only points at: an ISO keeps its first descriptor at 0x8000.
    let Ok(()) = file.read_exact(&mut head) else { return };
    let name = qubero_core::formats::sniff(&head, len).unwrap_or("-");
    let rel = path.strip_prefix(root).unwrap_or(path).display().to_string().replace('\\', "/");
    out.push(format!("{rel}\t{len}\t{name}"));
}
