//! Read every file in a directory tree with the template it sniffs as, and
//! report the ones that do not read. A sweep over a real collection of files,
//! for finding what a template gets wrong before a person does.

use std::path::{Path, PathBuf};

use qubero_core::document::Document;
use qubero_core::eval::{EvalError, Evaluator};
use qubero_core::formats;
use qubero_core::source::MemSource;

fn main() {
    let root = std::env::args().nth(1).expect("usage: check_tree <dir> [template]");
    let only = std::env::args().nth(2).filter(|a| a != "-");
    let depth: usize = std::env::args().nth(3).and_then(|d| d.parse().ok()).unwrap_or(5);
    let mut files = Vec::new();
    collect(Path::new(&root), &mut files);
    let (mut read, mut failed) = (0, 0);
    for path in files {
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let Some(name) = formats::sniff(&bytes[..bytes.len().min(formats::SNIFF_WINDOW)], bytes.len() as u64) else { continue };
        if only.as_deref().is_some_and(|want| want != name) {
            continue;
        }
        let Some(template) = formats::builtin(name) else { continue };
        let doc = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(template);
        let mut errors = Vec::new();
        walk(&mut ev, &doc, &[], 0, depth, &mut errors);
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
