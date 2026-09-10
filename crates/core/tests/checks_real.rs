//! Every check every sample declares, taken, over the whole collection.
//!
//! The collection lives outside this repository; point `QUBERO_SAMPLES` at it,
//! or keep it beside the repository as `qubero-samples`. Without it the test
//! says so and passes, the way the other sample tests do.
//!
//! One thing is asserted, and it is the thing a checksum in an interface lives
//! or dies by: **no file in the collection reports a mismatch**. These are
//! files that are what they say they are, so a check that fails one of them is
//! a check declared wrongly, and a reader shown a red mismatch on a good file
//! learns to ignore every other one. A refusal is fine, and so is a check that
//! does not apply; a wrong answer is not.
//!
//! The other half of the same rule is that the checks have to actually happen.
//! A declaration that quietly resolves to nothing everywhere would pass an
//! assertion about mismatches trivially, so the run prints what it took and
//! fails if a format the collection has samples of never manages one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use qubero_core::document::Document;
use qubero_core::eval::{EvalError, Evaluator};
use qubero_core::formats;
use qubero_core::source::MemSource;

/// How many nodes of one file are looked at. A sample is here to exercise a
/// format rather than to be read to the end, and the checks a format has are
/// in the first few thousand nodes of it or in none of them.
const NODES: usize = 20_000;

/// How deep the walk goes. Deeper than any format nests its checksums, and
/// shallow enough that a format which nests without bound stops.
const DEEP: usize = 12;

#[test]
fn no_valid_sample_reports_a_mismatch() {
    let Some(root) = samples() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let mut files = Vec::new();
    collect(&root, &mut files);
    assert!(!files.is_empty(), "no files under {}", root.display());

    // What each format's checks came to, so the run says which declarations
    // are doing anything at all rather than only that nothing broke.
    let mut tally: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
    let mut bad: Vec<String> = Vec::new();
    for path in files {
        // Files kept because Qubero refuses them are not files that are what
        // they say they are, and a broken file failing a check is the check
        // working.
        if path.components().any(|c| c.as_os_str() == "does-not-read" || c.as_os_str() == "corrupt") {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        let head = &bytes[..bytes.len().min(0x9000)];
        let Some(name) = formats::sniff(head, bytes.len() as u64) else { continue };
        let Some(template) = formats::builtin(name) else { continue };
        let doc = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(template);
        let mut w = Walk { visited: 0, seen: 0, passed: 0, refused: 0, bad: Vec::new() };
        w.node(&mut ev, &doc, &mut Vec::new(), &path);
        let entry = tally.entry(name.to_string()).or_default();
        entry.0 += w.passed;
        entry.1 += w.refused;
        entry.2 += w.seen;
        bad.extend(w.bad);
    }
    for (name, (passed, refused, seen)) in &tally {
        if *seen > 0 {
            eprintln!("{name}: {seen} checks, {passed} passed, {refused} refused");
        }
    }
    assert!(bad.is_empty(), "checks failed on files that are not broken:\n{}", bad.join("\n"));
    // The collection has a PNG, a ZIP and a gzip in it; if none of the three
    // managed a single check, something between the template and the query has
    // come apart and the assertion above proved nothing.
    let took: usize = tally.values().map(|(passed, _, _)| passed).sum();
    assert!(took > 0, "not one check was taken over the whole collection");
    eprintln!("{took} checks passed across {} formats", tally.len());
}

/// One file, named, checked all the way through the panel's own two calls.
///
/// The sweep above says no sample reports a mismatch, which a format whose
/// checks all quietly resolve to nothing passes as easily as a format whose
/// checks all work. This says one particular check on one particular file was
/// taken and passed, and it is here rather than beside the template because
/// only a file somebody else's encoder wrote can say that.
///
/// `hello.txt.xz` is the default case and nothing else: `xz` with no arguments
/// writes one block, one LZMA2 filter, and a CRC-64 over what the block
/// unpacks to. That check had no arithmetic behind it at all until recently,
/// so the file opened with nothing on screen able to say whether its data was
/// intact.
#[test]
fn the_block_check_of_a_default_xz_is_taken_and_passes() {
    let Some(root) = samples() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let path = root.join("compressed").join("hello.txt.xz");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("skipped: no {}", path.display());
        return;
    };
    let doc = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(formats::builtin("xz").expect("the xz template"));
    let at = |ev: &mut Evaluator, path: &[usize], name: &str| {
        ev.child_named(&doc, path, name).unwrap().unwrap_or_else(|| panic!("no {name} under {path:?}"))
    };
    let blocks = at(&mut ev, &[], "blocks");
    let block = [&blocks[..], &[0]].concat();
    let check = at(&mut ev, &block, "check");

    let info = ev.check_of(&doc, &check).unwrap().expect("a block's check checks something");
    assert_eq!(info.algorithm, "crc64", "xz writes a CRC-64 unless it is told otherwise");
    // Over bytes that are nowhere in the file, so what a reader is sent to is
    // the packed run those bytes came out of.
    assert_eq!(info.over, None);
    assert!(info.unpacked_from.is_some());
    let v = ev.run_check(&doc, &check).unwrap().expect("the check is taken");
    assert!(v.ok, "computed {}, stored {}", v.computed, v.stored);
    eprintln!("{}: block check {} passed", path.display(), v.stored);
}

struct Walk {
    /// Nodes looked at, which is what the budget is spent on: a file with no
    /// checks in it must not walk to the end of a million-element table to
    /// find that out.
    visited: usize,
    seen: usize,
    passed: usize,
    refused: usize,
    bad: Vec<String>,
}

impl Walk {
    /// Every node under `path`, asking each one what it checks.
    fn node<S: qubero_core::source::Source>(
        &mut self,
        ev: &mut Evaluator,
        doc: &Document<S>,
        path: &mut Vec<usize>,
        file: &Path,
    ) {
        if self.visited >= NODES || path.len() >= DEEP {
            return;
        }
        self.visited += 1;
        match ev.check_of(doc, path) {
            Ok(Some(_)) => {
                self.seen += 1;
                match ev.run_check(doc, path) {
                    Ok(Some(v)) if v.ok => self.passed += 1,
                    Ok(Some(v)) => self.bad.push(format!(
                        "  {} at {path:?}: computed {}, stored {}",
                        file.display(),
                        v.computed,
                        v.stored
                    )),
                    // A check that resolved and then had nothing to compare
                    // against, which is a template pointing an algorithm at a
                    // field of the wrong shape.
                    Ok(None) => self.bad.push(format!("  {} at {path:?}: nothing to compare", file.display())),
                    Err(EvalError::Failed(_)) => self.refused += 1,
                    // Every byte is here already, so nothing can be pending.
                    Err(e) => panic!("{} at {path:?}: {e:?}", file.display()),
                }
            }
            Ok(None) => {}
            // A node the template cannot read is the format's business and
            // `samples_real` catches it.
            Err(_) => return,
        }
        let Ok(info) = ev.node(doc, path) else { return };
        if !info.composite {
            return;
        }
        for i in 0..info.child_count.min(256) {
            path.push(i as usize);
            self.node(ev, doc, path, file);
            path.pop();
            if self.visited >= NODES {
                return;
            }
        }
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
