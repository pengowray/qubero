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

/// Every version 2 B-tree in every file to hand, walked, and the walk checked
/// against itself.
///
/// The widths of the two counts in a child pointer are not written anywhere in
/// an HDF5 file. They come out of the node size, the record size and the depth
/// by an arithmetic the library does and the format does not record, so the
/// only proof that arithmetic is right is a real file: get a width wrong by one
/// byte and every child address after the first is read from the middle of the
/// pointer before it.
///
/// What is asserted is what a wrong width breaks. An internal node points at
/// one more child than it holds records, always, because a version 2 tree is a
/// B-tree and a record sits between every two children; a node's level is one
/// below its parent's; and a node whose bytes are not the `BTIN` or `BTLF` the
/// level called for is refused, which shows up as a parent that says it did not
/// read all of its children. So a tree that comes back with the right number of
/// children on every node, nothing refused and nothing left out, was walked by
/// pointers that landed where they were meant to.
///
/// Skips where there is no file with one in it. A version 2 tree is written by
/// a recent library and only for a group with more links than fit in its
/// header, or a dataset with more than one unlimited dimension, so plenty of
/// real HDF5 files have none.
#[test]
fn every_version_2_btree_is_walked_by_pointers_that_land() {
    use qubero_core::formats::hdf5_tree::{Job, Kind, Records, Tree, NO_PARENT};

    let mut dirs = vec![PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../web/public"))];
    if let Ok(extra) = std::env::var("QUBERO_SAMPLES") {
        dirs.extend(extra.split(';').filter(|s| !s.is_empty()).map(PathBuf::from));
    }
    let mut found = Vec::new();
    for dir in &dirs {
        collect(dir, 3, &mut found);
    }
    found.sort();
    let mut walked = 0usize;
    for path in &found {
        let Ok(file) = File::open(path) else { continue };
        let Ok(len) = file.metadata().map(|m| m.len()) else { continue };
        let doc = Document::new(FileSource { file: RefCell::new(file), len });
        let mut ev = Evaluator::new(hdf5());
        if !matches!(ev.node(&doc, &[0]).map(|n| n.value), Ok(Value::Magic { ok: true, .. })) {
            continue;
        }
        let mut headers = Vec::new();
        let mut seen = 0usize;
        headers2(&mut ev, &doc, &[], &mut seen, &mut headers);
        // A cursor that has not moved yet, in a file that has a tree
        // somewhere. The root group of a file a recent library wrote usually
        // keeps its handful of links in its own header and has no tree of its
        // own, so answering with the root group's and stopping would say "no
        // B-tree" about a file with one; the walk goes on to the first tree
        // under it.
        if !headers.is_empty() {
            let from_nowhere = qubero_core::formats::hdf5_tree::tree(&mut ev, &doc, &[], 4096);
            assert!(
                matches!(from_nowhere, Ok(Some(_))),
                "{}: a file with {} version 2 trees answered a resting cursor with {from_nowhere:?}",
                path.display(),
                headers.len()
            );
        }
        for header in &headers {
            let tree: Tree = match qubero_core::formats::hdf5_tree::tree(&mut ev, &doc, header, 4096) {
                Ok(Some(t)) => t,
                Ok(None) => panic!("{}: a BTree2 at {header:?} answered with no tree", path.display()),
                Err(e) => panic!("{}: a BTree2 at {header:?} does not walk: {e:?}", path.display()),
            };
            assert_eq!(tree.version, 2, "{}: {header:?} walked as a version 1 tree", path.display());
            walked += 1;
            let depth = tree.nodes.iter().map(|n| n.depth).max().unwrap_or(0);
            let leaves = tree.nodes.iter().filter(|n| n.kind == Kind::Leaf).count();
            let records: u64 = tree.nodes.iter().map(|n| n.entries).sum();
            eprintln!(
                "--- {}: {} tree at {:#x}, {} deep, {} nodes, {leaves} leaves, {records} records, {} omitted",
                path.display(),
                tree.record_type_name,
                tree.nodes.first().map(|n| n.address).unwrap_or(0),
                depth + 1,
                tree.nodes.len(),
                tree.omitted
            );
            assert_eq!(tree.nodes[0].parent, NO_PARENT);
            // A version 2 tree never has the row a version 1 group tree has.
            assert!(tree.nodes.iter().all(|n| n.kind != Kind::LinkTable), "{}: a link table in a version 2 tree", path.display());
            // A group tree's records name links in a heap, so there is nothing
            // to show as a range and nothing is shown.
            if tree.job == Job::Group {
                assert!(tree.nodes.iter().all(|n| n.first_key.is_empty()), "{}: a hash shown as a name", path.display());
            }
            if tree.omitted > 0 {
                continue;
            }
            let mut children = vec![0u64; tree.nodes.len()];
            for node in tree.nodes.iter().skip(1) {
                children[node.parent] += 1;
                let up = &tree.nodes[node.parent];
                assert_eq!(node.level + 1, up.level, "{}: a node not one level below its parent", path.display());
                assert_eq!(node.depth, up.depth + 1, "{}: a node not one row below its parent", path.display());
            }
            for (i, node) in tree.nodes.iter().enumerate() {
                assert!(!node.truncated, "{}: node {i} at {:#x} refused a child", path.display(), node.address);
                match node.kind {
                    Kind::Leaf => {
                        assert_eq!(node.level, 0);
                        assert_eq!(node.sign, "BTLF");
                        assert_eq!(children[i], 0);
                    }
                    _ => {
                        assert!(node.level > 0);
                        assert_eq!(node.sign, "BTIN");
                        // The one number a wrong pointer width would not give.
                        assert_eq!(
                            children[i],
                            node.entries + 1,
                            "{}: node {i} at {:#x} holds {} records and has {} children",
                            path.display(),
                            node.address,
                            node.entries,
                            children[i]
                        );
                    }
                }
            }
            // An unfiltered chunk tree's leaves read their own ranges, so a
            // tree that was walked whole has one on every leaf that holds
            // anything.
            if tree.records == Records::Read && tree.job == Job::Chunk {
                assert!(tree.coords > 0 && !tree.coords_pad);
                assert!(
                    tree.nodes.iter().all(|n| n.kind != Kind::Leaf || n.entries == 0 || !n.first_key.is_empty()),
                    "{}: a leaf with records and no range",
                    path.display()
                );
            }
        }
    }
    if walked == 0 {
        eprintln!("skipped: no version 2 B-tree in {dirs:?}. Put an HDF5 file written with libver=latest there.");
    }
}

/// Every version 2 B-tree header under `path`. The template names one `BTree2`
/// wherever it places one, which is what makes the search a walk rather than a
/// scan for the signature: a `BTHD` in the middle of a dataset's bytes is not
/// a tree the file reached.
fn headers2(
    ev: &mut Evaluator,
    doc: &Document<FileSource>,
    path: &[usize],
    seen: &mut usize,
    out: &mut Vec<Vec<usize>>,
) {
    if *seen >= BUDGET {
        return;
    }
    let Ok(node) = ev.node(doc, path) else { return };
    *seen += 1;
    if node.type_name == "BTree2" {
        out.push(path.to_vec());
        // The nodes under it are the tree, and walking into them here would be
        // a second walk of the same bytes for nothing.
        return;
    }
    for i in 0..node.child_count as usize {
        let mut p = path.to_vec();
        p.push(i);
        headers2(ev, doc, &p, seen, out);
        if *seen >= BUDGET {
            return;
        }
    }
}
