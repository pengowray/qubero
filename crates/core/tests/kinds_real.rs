//! What the whole-file kind totals come to over the sample collection.
//!
//! The collection lives outside this repository; point `QUBERO_SAMPLES` at it,
//! or keep it beside the repository as `qubero-samples`. Without it the test
//! says so and passes, the way the other sample tests do.
//!
//! Two things are checked of every file. The walk finishes, in goes of a
//! bounded allowance, rather than running for ever on a run whose length its
//! own bytes give. And the numbers it answers with hold together: nothing
//! reaches past the end of the file, and what is covered and what is a gap
//! never come to more than what has been reached. A walk that counted a
//! structure's bits and its children's would break the second within a few
//! files.
//!
//! It also prints what each file cost, which is the number this exists to keep
//! honest: a run of same-sized records is meant to cost one element's walk,
//! and a run of variable-length ones is meant to cost one go per few hundred.

use std::path::{Path, PathBuf};

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, KindWalk};
use qubero_core::formats;
use qubero_core::source::MemSource;

/// What one go of the walk may spend, matching what the editor gives it.
const SLICE: u64 = 5_000;

/// Past this many goes a file is taken to be one the walk cannot finish, and
/// the test says so rather than running until someone stops it.
const GOES: usize = 20_000;

#[test]
fn every_sample_adds_up() {
    let Some(root) = samples() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let mut files = Vec::new();
    collect(&root, &mut files);
    assert!(!files.is_empty(), "no files under {}", root.display());
    for path in files {
        // Files kept because Qubero refuses them have nothing to total.
        if path.components().any(|c| c.as_os_str() == "does-not-read") {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        let head = &bytes[..bytes.len().min(0x9000)];
        let name = match path.extension().is_some_and(|e| e.eq_ignore_ascii_case("com")) {
            true => "com",
            false => match formats::sniff(head, bytes.len() as u64) {
                Some(name) => name,
                None => continue,
            },
        };
        let Some(template) = formats::builtin(name) else { continue };
        let len = bytes.len() as u64 * 8;
        let doc = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(template);
        ev.set_slice(Some(SLICE));
        let mut walk = KindWalk::new(len);
        let mut goes = 0;
        let mut peak = 0;
        let out = loop {
            goes += 1;
            assert!(goes <= GOES, "{} as {name} is still walking after {GOES} goes", path.display());
            ev.begin_slice();
            let out = match ev.kind_totals_step(&doc, &mut walk) {
                Ok(out) => out,
                // Every byte is here already, so nothing can be pending; a
                // file the template cannot read at all is the format's
                // business and `every_sample_still_reads` catches it.
                Err(e) => panic!("{} as {name}: {e:?}", path.display()),
            };
            peak = peak.max(ev.memo_len());
            if out.done {
                break out;
            }
        };
        assert!(out.reached_bits <= len, "{} as {name} reached past the end of the file", path.display());
        if out.covered_bits + out.unmapped_bits > out.reached_bits {
            for t in &out.totals {
                eprintln!("   {} {} {} bits x{}", t.kind, t.type_name, t.bits, t.count);
            }
        }
        assert!(
            out.covered_bits + out.unmapped_bits <= out.reached_bits,
            "{} as {name}: {} covered and {} unmapped of {} reached",
            path.display(),
            out.covered_bits,
            out.unmapped_bits,
            out.reached_bits
        );
        let sum: u64 = out.totals.iter().map(|t| t.bits).sum();
        assert_eq!(sum, out.covered_bits, "{} as {name}: the entries do not add up to what is covered", path.display());
        eprintln!(
            "{} as {name}: {goes} goes, {peak} nodes at most, {} kinds, {}% of {} bytes covered",
            path.display(),
            out.totals.len(),
            out.covered_bits * 100 / len.max(1),
            len / 8
        );
    }
}

fn samples() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("QUBERO_SAMPLES") {
        let p = PathBuf::from(dir);
        if p.is_dir() {
            return Some(p);
        }
    }
    let beside = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../qubero-samples"));
    beside.is_dir().then_some(beside)
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "tools" || n == ".git") {
                continue;
            }
            collect(&path, out);
        } else if path.extension().is_none_or(|x| x != "md" && x != "tsv") {
            out.push(path);
        }
    }
}
