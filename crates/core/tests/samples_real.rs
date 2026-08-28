//! A smoke test over the sample collection, which lives outside this
//! repository because the files are large and none of them are ours.
//!
//! Point `QUBERO_SAMPLES` at it, or keep it beside the repository as
//! `qubero-samples`, which is where `tools/build_index.py` in that folder
//! expects to find this one. With neither, the test says so and passes: a
//! checkout without the collection is not a broken checkout.
//!
//! What it checks is that every file still reads: the same template is picked,
//! the root's fields all resolve, and the instructions at both ends of every
//! run of code decode. A decoder crate that changes its mind about a byte
//! shows up here rather than in the editor.

use std::path::{Path, PathBuf};

use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats;
use qubero_core::source::MemSource;

#[test]
fn every_sample_still_reads() {
    let Some(root) = samples() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let mut files = Vec::new();
    collect(&root, &mut files);
    assert!(!files.is_empty(), "no files under {}", root.display());
    let mut read_count = 0;
    for path in files {
        let bytes = std::fs::read(&path).unwrap();
        let head = &bytes[..bytes.len().min(0x9000)];
        // A `.COM` file has no header to say what it is, so the extension is
        // what says it. Everything else the file itself announces.
        let name = match path.extension().is_some_and(|e| e.eq_ignore_ascii_case("com")) {
            true => "com",
            false => match formats::sniff(head, bytes.len() as u64) {
                Some(name) => name,
                None => panic!("nothing reads {}", path.display()),
            },
        };
        let template = formats::builtin(name).unwrap_or_else(|| panic!("no template {name}"));
        let doc = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(template);
        eprintln!("--- {} as {name}", path.display());
        read(&mut ev, &doc, &[], 0);
        read_count += 1;
    }
    eprintln!("{read_count} samples read");
}

/// Resolve a node and enough of what is under it to prove the file reads. A
/// long list is read at both ends rather than throughout: what breaks is the
/// first element or the last, and reading a million rows would make this test
/// something nobody runs.
fn read(ev: &mut Evaluator, doc: &Document<MemSource>, path: &[usize], depth: usize) {
    if depth > 4 {
        return;
    }
    let node = match ev.node(doc, path) {
        Ok(n) => n,
        Err(e) => panic!("{path:?} does not read: {e:?}"),
    };
    let count = node.child_count as usize;
    let ends: Vec<usize> = if count > 8 {
        (0..4).chain(count - 4..count).collect()
    } else {
        (0..count).collect()
    };
    for i in ends {
        let mut child = path.to_vec();
        child.push(i);
        read(ev, doc, &child, depth + 1);
    }
}

fn samples() -> Option<PathBuf> {
    let named = std::env::var_os("QUBERO_SAMPLES").map(PathBuf::from);
    let beside = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../qubero-samples");
    named.into_iter().chain(std::iter::once(beside)).find(|p| p.is_dir())
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "tools" || n == ".git") {
                continue;
            }
            collect(&path, out);
        } else if path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
            // The repository's own files: the collection is versioned by its
            // lists, and those are not samples.
            continue;
        } else if path.extension().is_none_or(|e| e != "md" && e != "tsv" && e != "py") {
            out.push(path);
        }
    }
}
