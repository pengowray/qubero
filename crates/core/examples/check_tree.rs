//! Read every file in a directory tree with the template it sniffs as, and
//! report the ones that do not read. A sweep over a real collection of files,
//! for finding what a template gets wrong before a person does.

use std::path::{Path, PathBuf};

use qubero_core::document::Document;
use qubero_core::eval::{EvalError, Evaluator};
use qubero_core::formats;
use qubero_core::source::MemSource;

/// A file in a folder of this name is here to be refused: a file the reader
/// is meant to say no to rather than read. It does not count against the
/// sweep, and one of them that reads is what the sweep should say.
const REFUSED: &str = "does-not-read";

fn main() {
    let root = std::env::args().nth(1).expect("usage: check_tree <dir> [template]");
    let only = std::env::args().nth(2).filter(|a| a != "-");
    let depth: usize = std::env::args().nth(3).and_then(|d| d.parse().ok()).unwrap_or(5);
    let mut files = Vec::new();
    collect(Path::new(&root), &mut files);
    let (mut read, mut failed) = (0, 0);
    // Files under the folder, and the two ways one of them can leave the
    // sweep without having been turned away: it read, or nothing read it.
    let (mut kept, mut read_anyway, mut not_checked) = (0, 0, 0);
    for path in files {
        // The folder's own notes are notes, not a sample kept to be refused.
        let notes = path.extension().is_some_and(|e| e == "md");
        let meant_to_fail = !notes && path.components().any(|c| c.as_os_str() == REFUSED);
        let Ok(bytes) = std::fs::read(&path) else { continue };
        if meant_to_fail {
            kept += 1;
        }
        let sniffed = formats::sniff(&bytes[..bytes.len().min(formats::SNIFF_WINDOW)], bytes.len() as u64);
        let Some(name) = sniffed else {
            // Silence is right for the collection at large, where a file no
            // template matches is one waiting for a template. A file kept to
            // be refused says so, since a file nothing read is a file the
            // sweep has not tested.
            if meant_to_fail {
                not_checked += 1;
                println!("{}: not checked, no template matches it", path.display());
            }
            continue;
        };
        if only.as_deref().is_some_and(|want| want != name) {
            if meant_to_fail {
                not_checked += 1;
            }
            continue;
        }
        let Some(template) = formats::builtin(name) else {
            if meant_to_fail {
                not_checked += 1;
            }
            continue;
        };
        let doc = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(template);
        let mut errors = Vec::new();
        walk(&mut ev, &doc, &[], 0, depth, &mut errors);
        if meant_to_fail {
            if errors.is_empty() {
                read_anyway += 1;
                println!("{} as {name} reads, but files in {REFUSED} should not read", path.display());
            }
            continue;
        }
        read += 1;
        if !errors.is_empty() {
            failed += 1;
            println!("{} as {name}", path.display());
            for (path, err) in errors.iter().take(3) {
                println!("    {path:?} {err:?}");
            }
        }
    }
    println!("{read} files read, {failed} with something that does not read");
    if kept > 0 {
        let s = if kept == 1 { "" } else { "s" };
        println!("{kept} file{s} in {REFUSED}, {read_anyway} read anyway, {not_checked} not checked");
    }
}

fn walk(
    ev: &mut Evaluator,
    doc: &Document<MemSource>,
    path: &[usize],
    depth: usize,
    max: usize,
    out: &mut Vec<(Vec<usize>, EvalError)>,
) {
    if depth > max || out.len() > 8 {
        return;
    }
    let node = match ev.node(doc, path) {
        Ok(n) => n,
        Err(e) => {
            out.push((path.to_vec(), e));
            return;
        }
    };
    let count = node.child_count as usize;
    let ends: Vec<usize> = if count > 8 { (0..4).chain(count - 4..count).collect() } else { (0..count).collect() };
    for i in ends {
        let mut child = path.to_vec();
        child.push(i);
        walk(ev, doc, &child, depth + 1, max, out);
    }
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else {
            out.push(path);
        }
    }
}
