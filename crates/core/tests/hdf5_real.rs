//! A smoke test over real HDF5 files, which are too big to keep in the
//! repository: any `*.h5` or `*.h5ad` in `web/public`, or under a directory
//! `QUBERO_SAMPLES` names (several, separated by `;`). Skips when there is
//! none.
//!
//! The test is the walk. Every object in one of these files is reached by
//! address, so opening every group and every message from the root is what
//! shows that the addresses were read as the file meant them: a header at a
//! wrong address is not a wrong number, it is a signature that is not there.

use std::cell::RefCell;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Explain, Value};
use qubero_core::formats::hdf5;
use qubero_core::source::{Missing, Source};

struct FileSource {
    file: RefCell<File>,
    len: u64,
}

impl Source for FileSource {
    fn len_bytes(&self) -> u64 {
        self.len
    }
    fn read_bytes(&self, offset: u64, out: &mut [u8]) -> Vec<Missing> {
        let mut f = self.file.borrow_mut();
        if f.seek(SeekFrom::Start(offset)).is_err() || f.read_exact(out).is_err() {
            out.fill(0);
        }
        Vec::new()
    }
}

#[test]
fn reads_real_files_end_to_end() {
    let mut dirs = vec![PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../web/public"))];
    if let Ok(extra) = std::env::var("QUBERO_SAMPLES") {
        dirs.extend(extra.split(';').filter(|s| !s.is_empty()).map(PathBuf::from));
    }
    let mut found = Vec::new();
    for dir in &dirs {
        collect(dir, 3, &mut found);
    }
    if found.is_empty() {
        eprintln!("skipped: no HDF5 file in {dirs:?}. Put one there, or set QUBERO_SAMPLES.");
        return;
    }
    found.sort();
    for path in found {
        check(&path);
    }
}

fn collect(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            if depth > 0 {
                collect(&path, depth - 1, out);
            }
        } else if path.extension().is_some_and(|x| x == "h5" || x == "h5ad") {
            out.push(path);
        }
    }
}

/// How many nodes one file is allowed to cost, so a five-gigabyte atlas with a
/// hundred thousand chunks does not turn a smoke test into an afternoon.
const BUDGET: usize = 200_000;

fn check(path: &Path) {
    let file = File::open(path).expect("opens");
    let len = file.metadata().expect("has a size").len();
    let doc = Document::new(FileSource { file: RefCell::new(file), len });
    let mut ev = Evaluator::new(hdf5());

    // The extension is not the format: an archive unpacked on a Mac leaves a
    // resource fork beside every file, under the same name and with none of
    // the bytes. The signature is the first field of a file that opens with
    // one and the second of a file that keeps a user block in front of it, so
    // both are asked. A file with neither is passed over rather than failed,
    // since nothing here claimed it was one.
    let signed = |ev: &mut Evaluator, at: &[usize]| {
        matches!(ev.node(&doc, at).map(|n| n.value), Ok(Value::Magic { ok: true, .. }))
    };
    if !signed(&mut ev, &[0]) && !signed(&mut ev, &[1, 0]) {
        eprintln!("--- {}: no HDF5 signature, passed over", path.display());
        return;
    }

    let mut seen = 0usize;
    let mut names = Vec::new();
    walk(&mut ev, &doc, &[], &mut seen, &mut names, path);
    eprintln!("--- {}: {len} bytes, {seen} nodes read", path.display());
    eprintln!("  names: {}", names.join(" "));
    // A file with nothing in it would pass everything above, so the root group
    // has to have led somewhere.
    assert!(!names.is_empty(), "{}: no named object was reached", path.display());
}

/// The same file with and without a user block reads the same objects.
///
/// Every address in an HDF5 file counts from the base address its superblock
/// names, and the two files in the collection differ only in what that address
/// is: nought in one and 512 in the other. Reading them as different files
/// would mean the base was not being counted from, which is the one thing this
/// pair is there to say.
#[test]
fn a_user_block_changes_where_the_file_begins_and_nothing_else() {
    let Some(dir) = sample_dir() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let named = |name: &str| -> Vec<String> {
        let path = dir.join("hdf5").join(name);
        let file = File::open(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let len = file.metadata().unwrap().len();
        let doc = Document::new(FileSource { file: RefCell::new(file), len });
        let mut ev = Evaluator::new(hdf5());
        let (mut seen, mut names) = (0usize, Vec::new());
        walk(&mut ev, &doc, &[], &mut seen, &mut names, &path);
        names
    };
    let plain = named("groups-and-datasets.h5");
    assert!(plain.contains(&"measurements".to_string()), "{plain:?}");
    assert_eq!(plain, named("userblock-512.h5"));
}

/// A version 4 layout message names one of five ways of indexing a dataset's
/// chunks, and the file in the collection has a dataset for each. All five are
/// named, and four of them lead to the chunks themselves: an implicit index
/// writes no entries anywhere, so it is the one that says where its run starts
/// and stops there.
///
/// The numbers are what says the indexes were walked rather than merely read.
/// Every unfiltered dataset in that file counts up from nought, and a chunk
/// reached through a wrong address, or read as a run of the wrong length, does
/// not produce them.
#[test]
fn every_version_4_chunk_index_reaches_its_chunks() {
    let Some(dir) = sample_dir() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let path = dir.join("hdf5").join("chunk-indexes-v4.h5");
    let file = File::open(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let len = file.metadata().unwrap().len();
    let doc = Document::new(FileSource { file: RefCell::new(file), len });
    let mut ev = Evaluator::new(hdf5());

    let (mut indexes, mut numbers, mut filtered) = (Vec::new(), Vec::new(), 0usize);
    gather(&mut ev, &doc, &[], &mut 0usize, &mut indexes, &mut numbers, &mut filtered, &path);

    indexes.sort();
    indexes.dedup();
    assert_eq!(
        indexes,
        vec!["extensible array", "fixed array", "implicit", "single chunk", "version 2 b-tree"],
        "{}: not every chunk index was named",
        path.display()
    );
    for want in 0..16 {
        assert!(numbers.contains(&want), "{}: the chunks never gave up {want}", path.display());
    }
    assert!(filtered >= 2, "{}: only {filtered} filtered chunks", path.display());
}

/// Every node under `path`, collecting what the chunk indexes are called, the
/// numbers the chunks hold, and how many chunks a filter wrote.
#[allow(clippy::too_many_arguments)]
fn gather(
    ev: &mut Evaluator,
    doc: &Document<FileSource>,
    path: &[usize],
    seen: &mut usize,
    indexes: &mut Vec<String>,
    numbers: &mut Vec<i128>,
    filtered: &mut usize,
    file: &Path,
) {
    if *seen >= BUDGET {
        return;
    }
    let node = ev
        .node(doc, path)
        .unwrap_or_else(|e| panic!("{}: {path:?} does not read: {e:?}", file.display()));
    *seen += 1;
    if node.name == "index_type" {
        if let Value::Enum { name: Some(n), .. } = &node.value {
            indexes.push(n.clone());
        }
    }
    if node.type_name == "FilteredChunk" {
        *filtered += 1;
    }
    if let Value::Int(v) = node.value {
        numbers.push(v);
    }
    for i in 0..node.child_count as usize {
        let mut p = path.to_vec();
        p.push(i);
        gather(ev, doc, &p, seen, indexes, numbers, filtered, file);
        if *seen >= BUDGET {
            return;
        }
    }
}

/// The sample collection, wherever it is. None when there is none.
fn sample_dir() -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(set) = std::env::var("QUBERO_SAMPLES") {
        roots.extend(set.split(';').filter(|s| !s.is_empty()).map(PathBuf::from));
    }
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../qubero-samples"));
    roots.into_iter().find(|r| r.join("hdf5").is_dir())
}

/// Every node under `path`, in order, failing on the first that cannot be
/// read. A pointer read wrongly lands on bytes that are not what they claim,
/// which is an error here rather than a plausible-looking wrong answer.
fn walk(
    ev: &mut Evaluator,
    doc: &Document<FileSource>,
    path: &[usize],
    seen: &mut usize,
    names: &mut Vec<String>,
    file: &Path,
) {
    if *seen >= BUDGET {
        return;
    }
    let node = ev
        .node(doc, path)
        .unwrap_or_else(|e| panic!("{}: {path:?} does not read: {e:?}", file.display()));
    *seen += 1;
    // A chunk that went through a filter is the one thing in the file whose
    // contents are not in the file. Undoing it is the reader's job rather than
    // a field's, so this asks for that reading wherever it meets one.
    if node.type_name == "FilteredChunk" {
        match ev.explain(doc, path, None) {
            Ok(Explain::Hdf5Chunk { packed_bytes, decoded_bytes, steps, values, element_type, problem, .. }) => {
                assert!(problem.is_none(), "{}: {path:?}: {problem:?}", file.display());
                assert!(decoded_bytes >= packed_bytes, "{}: a filtered chunk that got smaller", file.display());
                assert!(!steps.is_empty(), "{}: a filtered chunk with no filters undone", file.display());
                if names.len() < 24 {
                    names.push(format!(
                        "[chunk {packed_bytes}->{decoded_bytes} {element_type} {}]",
                        values.first().cloned().unwrap_or_default()
                    ));
                }
            }
            Ok(other) => panic!("{}: a filtered chunk explained as {other:?}", file.display()),
            Err(e) => panic!("{}: a filtered chunk does not read: {e:?}", file.display()),
        }
    }
    if let (Value::Str(s), true) = (&node.value, node.name == "name") {
        if !s.is_empty() && names.len() < 24 {
            names.push(s.clone());
        }
    }
    // A dataset of ten million numbers would fill the budget on its own and
    // leave the rest of the file unread, so a long list is sampled at both
    // ends and in the middle. The last element is the one worth having: a
    // stride read wrongly ends somewhere other than where the run does.
    let n = node.child_count as usize;
    let sample: Vec<usize> = if n <= 64 {
        (0..n).collect()
    } else {
        (0..8).chain([n / 2]).chain(n - 8..n).collect()
    };
    for i in sample {
        let mut p = path.to_vec();
        p.push(i);
        walk(ev, doc, &p, seen, names, file);
        if *seen >= BUDGET {
            return;
        }
    }
}
