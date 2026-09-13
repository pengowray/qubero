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

/// The chunk indexes that only show themselves in a dataset of more than a
/// thousand chunks, each walked to every chunk it places and the places
/// checked against what the HDF5 library itself reports.
///
/// `chunk-indexes-large.h5` holds an implicit index over a dataset that can
/// grow in its second dimension, fixed arrays past one page (one of them with
/// a page that was never written, one filtered), and extensible arrays past
/// their index blocks into data blocks and secondary blocks (one filtered).
/// None of the indexes writes down how its entries are laid out: that comes
/// from logarithms and rounded-up divisions of numbers in a header, so a
/// layout worked out wrongly reads entries from the wrong bytes and the
/// chunks they name are not where h5py says.
///
/// The expected numbers were read out of the file with h5py 3.16, from
/// `get_chunk_info`: how many chunks, and the byte offsets of the first and
/// the last. Every chunk in between was compared once, the same way, when this
/// was written. The implicit index places three more chunks than h5py lists,
/// since its run is laid out for the largest the dataset can grow to and h5py
/// counts the chunks of the extent it has now; the nine h5py lists are among
/// them, at the places it gives.
#[test]
fn large_chunk_indexes_reach_every_chunk_where_the_library_puts_it() {
    let Some(dir) = sample_dir() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let path = dir.join("hdf5").join("chunk-indexes-large.h5");
    if !path.exists() {
        eprintln!("skipped: no {}", path.display());
        return;
    }
    let file = File::open(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let len = file.metadata().unwrap().len();
    let doc = Document::new(FileSource { file: RefCell::new(file), len });
    let mut ev = Evaluator::new(hdf5());

    let mut found: Vec<Placed> = Vec::new();
    chunks_by_dataset(&mut ev, &doc, &[], &mut String::new(), false, false, &mut found, &path);
    for d in &found {
        eprintln!("--- {}: {} chunks from {:?} to {:?}, {} pages never written", d.name, d.chunks.len(), d.chunks.first(), d.chunks.last(), d.unwritten_pages);
    }

    // Name, chunks, where the first and last are, and how many pages were set
    // aside and never written.
    let want: &[(&str, usize, u64, u64, usize)] = &[
        ("implicit", 12, 2048, 2312, 0),
        ("fixed_paged", 2100, 2336, 23365, 0),
        ("fixed_paged_sparse", 2, 63378, 63380, 1),
        ("fixed_paged_shuffled", 1100, 23367, 49790, 0),
        ("extensible", 600, 49794, 53040, 0),
        ("extensible_shuffled", 300, 53042, 63370, 0),
        ("extensible_paged_sparse", 3, 53890, 63376, 2),
    ];
    for &(name, count, first, last, unwritten) in want {
        let Some(d) = found.iter().find(|d| d.name == name) else {
            panic!("{}: no chunks reached for {name}", path.display());
        };
        assert_eq!(d.chunks.len(), count, "{}: {name}", path.display());
        assert_eq!(d.chunks.first(), Some(&first), "{}: {name}", path.display());
        assert_eq!(d.chunks.last(), Some(&last), "{}: {name}", path.display());
        assert_eq!(d.unwritten_pages, unwritten, "{}: {name}", path.display());
    }
    // The first chunk of the implicit index's second row, which is where the
    // largest extent and not the current one decides it is: h5py puts the
    // chunk starting at row 4, column 0 at byte 2144.
    let implicit = found.iter().find(|d| d.name == "implicit").unwrap();
    assert_eq!(implicit.chunks[4], 2144, "{}: the implicit index counted rows across the wrong extent", path.display());
}

/// The chunks one dataset's index led to.
struct Placed {
    name: String,
    /// Byte offsets, in the order the index lists them.
    chunks: Vec<u64>,
    /// Pages of index entries whose bit says they were never written.
    unwritten_pages: usize,
}

/// Every chunk under `path`, as the byte offset it was placed at, under the
/// name of the dataset it belongs to. A chunk is the run of elements an
/// unfiltered index entry points at, or the packed bytes a filtered one does,
/// and only under a chunked layout: the same run of elements is what a
/// contiguous dataset or an attribute reads as.
#[allow(clippy::too_many_arguments)]
fn chunks_by_dataset(
    ev: &mut Evaluator,
    doc: &Document<FileSource>,
    path: &[usize],
    name: &mut String,
    chunked: bool,
    in_pages: bool,
    out: &mut Vec<Placed>,
    file: &Path,
) {
    let node = ev
        .node(doc, path)
        .unwrap_or_else(|e| panic!("{}: {path:?} does not read: {e:?}", file.display()));
    if let (Value::Str(s), true) = (&node.value, node.name == "name") {
        if !s.is_empty() {
            *name = s.clone();
        }
    }
    let chunked = chunked || node.type_name == "Chunked";
    let is_chunk = chunked && (node.type_name == "Data" || node.type_name == "FilteredChunk");
    let unwritten = in_pages && matches!(node.value, Value::Bytes { .. });
    if is_chunk || unwritten {
        let at = match out.iter().position(|d| d.name == *name) {
            Some(i) => i,
            None => {
                out.push(Placed { name: name.clone(), chunks: Vec::new(), unwritten_pages: 0 });
                out.len() - 1
            }
        };
        match is_chunk {
            true => out[at].chunks.push(node.offset_bits / 8),
            false => out[at].unwritten_pages += 1,
        }
        return;
    }
    for i in 0..node.child_count as usize {
        let mut p = path.to_vec();
        p.push(i);
        chunks_by_dataset(ev, doc, &p, name, chunked, node.name == "pages", out, file);
    }
}

/// Every link in a group of two thousand is named under the template, the
/// ones in a table under the root table among them, and every node of the
/// version 2 tree indexing them is where the walk found it.
///
/// `fractal-heap-deep.h5` is the one file in the collection whose heap has
/// grown past its direct rows, so it is the one that says a table's row count
/// and which of its rows are tables were worked out right: get either wrong
/// and the 160 links under the second table are not reached, or are read out
/// of the wrong bytes. The names are known without h5py, because the
/// generator writes them as a number and a fixed tail, and h5py lists exactly
/// those.
#[test]
fn every_link_in_a_heap_grown_past_its_direct_rows_is_named() {
    use qubero_core::formats::hdf5_tree::Kind;

    let Some(dir) = sample_dir() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let path = dir.join("hdf5").join("fractal-heap-deep.h5");
    let Ok(file) = File::open(&path) else {
        eprintln!("skipped: no {}", path.display());
        return;
    };
    let len = file.metadata().unwrap().len();
    let doc = Document::new(FileSource { file: RefCell::new(file), len });
    let mut ev = Evaluator::new(hdf5());

    let mut found = Found::default();
    links(&mut ev, &doc, &[], 0, &mut found, &path);

    let tail = "abcdefghijklmnopqrstuvwxyz".repeat(10);
    let want: Vec<String> = (0..2000).map(|i| format!("link_{i:04}_{tail}")).collect();
    let mut got = found.names.clone();
    got.sort();
    assert_eq!(got.len(), want.len(), "{}: {} links named in the heap", path.display(), got.len());
    let wrong: Vec<&String> = got.iter().zip(&want).filter(|(g, w)| g != w).map(|(g, _)| g).take(3).collect();
    assert!(wrong.is_empty(), "{}: names read that were not written: {wrong:?}", path.display());
    assert_eq!(found.nested, 160, "{}: links under a table under the root table", path.display());

    assert_eq!(found.trees.len(), 1, "{}: {:?}", path.display(), found.trees);
    let tree = qubero_core::formats::hdf5_tree::tree(&mut ev, &doc, &found.trees[0], 4096)
        .expect("walks")
        .expect("a tree");
    assert_eq!(tree.omitted, 0);
    assert_eq!(tree.records_total, 2000);
    assert_eq!(tree.nodes.iter().map(|n| n.entries).sum::<u64>(), 2000);
    assert_eq!(tree.nodes.iter().map(|n| n.depth).max(), Some(2), "{}: not a tree of depth 2", path.display());
    assert!(tree.nodes.iter().filter(|n| n.kind == Kind::Leaf).count() > 1);
    for node in &tree.nodes {
        assert!(!node.path.is_empty(), "{}: the node at {:#x} has no path", path.display(), node.address);
        let at = ev.node(&doc, &node.path).expect("the node reads").offset_bits / 8;
        assert_eq!(at, node.address, "{}: the node walked at {:#x} is placed at {at:#x}", path.display(), node.address);
    }
}

#[derive(Default)]
struct Found {
    /// The name of every link read out of a heap block.
    names: Vec<String>,
    /// How many of those are in a block under more than one table.
    nested: usize,
    /// Where each version 2 tree header is.
    trees: Vec<Vec<usize>>,
}

/// Every link under `path`, following only the one link that leads to the big
/// group, so the two thousand hard links to one dataset are not each walked
/// into it. `tables` is how many heap tables the walk is inside.
fn links(
    ev: &mut Evaluator,
    doc: &Document<FileSource>,
    path: &[usize],
    tables: usize,
    found: &mut Found,
    file: &Path,
) {
    let node = ev
        .node(doc, path)
        .unwrap_or_else(|e| panic!("{}: {path:?} does not read: {e:?}", file.display()));
    let tables = tables + usize::from(node.type_name == "HeapIndirectBlock");
    if node.type_name == "BTree2" {
        found.trees.push(path.to_vec());
        return;
    }
    if node.type_name == "Link" {
        let name = ev.child_named(doc, path, "name").expect("reads").expect("a link has a name");
        // The bytes the field covers rather than its value, which is cut short
        // for display, and these names are 270 characters long.
        let info = ev.node(doc, &name).expect("reads");
        assert!(matches!(info.value, Value::Str(_)), "{}: a link name that is not text", file.display());
        let mut bytes = vec![0u8; (info.size_bits / 8) as usize];
        assert!(doc.read_bits(info.offset_bits, info.size_bits, &mut bytes).is_empty());
        let name = String::from_utf8(bytes).expect("a name in UTF-8");
        if tables > 0 {
            found.names.push(name.clone());
            found.nested += usize::from(tables > 1);
        }
        if name != "wide" {
            return;
        }
    }
    for i in 0..node.child_count as usize {
        let mut p = path.to_vec();
        p.push(i);
        links(ev, doc, &p, tables, found, file);
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
    let (mut walked, mut behind_a_block) = (0usize, 0usize);
    for path in &found {
        let Ok(file) = File::open(path) else { continue };
        let Ok(len) = file.metadata().map(|m| m.len()) else { continue };
        let doc = Document::new(FileSource { file: RefCell::new(file), len });
        let mut ev = Evaluator::new(hdf5());
        // A file behind a user block as well as one that opens with its
        // signature. The walk reads the nodes as bytes and every pointer it
        // follows counts from the base address, so the files where that is
        // not the front of the file are the ones that say it was counted.
        let signed = |ev: &mut Evaluator, at: &[usize]| {
            matches!(ev.node(&doc, at).map(|n| n.value), Ok(Value::Magic { ok: true, .. }))
        };
        let blocked = !signed(&mut ev, &[0]);
        if blocked && !signed(&mut ev, &[1, 0]) {
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
            behind_a_block += usize::from(blocked);
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
            // The template works the pointer widths out for itself, from the
            // same three numbers, and places every node. Where it put each one
            // is where the walk read it, or one of the two has a width wrong.
            for node in &tree.nodes {
                assert!(!node.path.is_empty(), "{}: the node at {:#x} has no template path", path.display(), node.address);
                let placed = ev.node(&doc, &node.path).map(|n| n.offset_bits / 8);
                assert_eq!(
                    placed.as_ref().ok(),
                    Some(&node.address),
                    "{}: the template places the node walked at {:#x} at {placed:?}",
                    path.display(),
                    node.address
                );
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
                // Where the records are. Both kinds write them straight after
                // the six bytes of signature, version and type, so a leaf's
                // last record ends exactly at the four-byte checksum the node
                // finishes with; an internal node has its child pointers in
                // between, and ends further along than its records do.
                assert_eq!(node.first_entry_bits, 6 * 8, "{}: records not at the head of a node", path.display());
                let end = node.first_entry_bits + node.entries * node.entry_bits;
                let checked = node.size_bits.saturating_sub(32);
                match node.kind {
                    Kind::Leaf => assert_eq!(
                        end,
                        checked,
                        "{}: the records of the leaf at {:#x} do not fill it up to its checksum",
                        path.display(),
                        node.address
                    ),
                    _ => assert!(
                        end < checked,
                        "{}: no room for the child pointers of the node at {:#x}",
                        path.display(),
                        node.address
                    ),
                }
                // The last record, read back at the place the stride puts it.
                // An unfiltered chunk record is an address and then one offset
                // per dimension, which is what the walk wrote out as the
                // node's last key: a stride out by a field reads those offsets
                // from the wrong end of the record and they do not match.
                if node.kind == Kind::Leaf && node.entries > 0 && tree.coords > 0 && tree.coords <= 16 {
                    let at = node.address * 8 + node.first_entry_bits + (node.entries - 1) * node.entry_bits + 64;
                    let mut buf = vec![0u8; tree.coords as usize * 8];
                    assert!(doc.read_bits(at, tree.coords * 64, &mut buf).is_empty());
                    let read: Vec<String> = buf
                        .chunks(8)
                        .map(|c| u64::from_le_bytes(c.try_into().unwrap()).to_string())
                        .collect();
                    assert_eq!(
                        read.join(", "),
                        node.last_key,
                        "{}: the last record of the leaf at {:#x} is not where its stride puts it",
                        path.display(),
                        node.address
                    );
                }
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
    eprintln!("--- version 2 trees walked: {walked}, {behind_a_block} of them behind a user block");
    if sample_dir().is_some_and(|d| d.join("hdf5").join("btree-v2-userblock.h5").exists()) {
        assert!(behind_a_block > 0, "the collection has a version 2 tree behind a user block and the sweep did not reach it");
    }
}

/// A tree behind a user block is the same tree as in the file without one,
/// 512 bytes further on, and is found from a cursor that has not moved.
///
/// Three pairs, one per way of reading a tree: the version 1 trees of
/// `groups-and-datasets.h5` and `vlen-strings.h5`, which the walk reads as
/// fields, and the version 2 trees of `btree-v2-internal.h5`, which it reads
/// as bytes. Every node of the twin behind the block is at the address the
/// plain file's node is at, plus the block, and has a template path that lands
/// there. A MATLAB 7.3 file is then the same question asked through the MAT
/// template, whose HDF5 file is two fields down rather than one.
///
/// A cursor at rest is asked as well as a cursor on the tree. The root group is
/// found by looking for the superblock, and looking only at the root of the
/// field tree found none behind a block, so the answer was no tree at all.
#[test]
fn a_tree_behind_a_user_block_is_the_same_tree_further_on() {
    use qubero_core::formats::hdf5_tree::Tree;

    let Some(dir) = sample_dir() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let open = |path: &Path| {
        let file = File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let len = file.metadata().unwrap().len();
        Document::new(FileSource { file: RefCell::new(file), len })
    };
    // Every tree in a file, found the way the template places them, and each
    // walked from its own header or node.
    let trees = |path: &Path, template: qubero_core::template::Template| -> Vec<Tree> {
        let doc = open(path);
        let mut ev = Evaluator::new(template);
        let (mut at, mut seen) = (Vec::new(), 0usize);
        trees1(&mut ev, &doc, &[], &mut seen, &mut at);
        let mut seen = 0usize;
        headers2(&mut ev, &doc, &[], &mut seen, &mut at);
        let resting = qubero_core::formats::hdf5_tree::tree(&mut ev, &doc, &[], 4096);
        assert!(matches!(resting, Ok(Some(_))), "{}: a cursor at rest answered {resting:?}", path.display());
        let mut out: Vec<Tree> = Vec::new();
        for p in &at {
            let tree = qubero_core::formats::hdf5_tree::tree(&mut ev, &doc, p, 4096)
                .unwrap_or_else(|e| panic!("{}: the tree at {p:?} does not walk: {e:?}", path.display()))
                .unwrap_or_else(|| panic!("{}: no tree at {p:?}", path.display()));
            for node in &tree.nodes {
                assert!(!node.path.is_empty(), "{}: the node at {:#x} has no template path", path.display(), node.address);
                let placed = ev.node(&doc, &node.path).map(|n| n.offset_bits / 8).ok();
                assert_eq!(placed, Some(node.address), "{}: a node walked at {:#x}", path.display(), node.address);
            }
            // The nodes under a version 1 root answer with the same tree, so
            // one copy of each is kept.
            if !out.iter().any(|t| t.nodes.first().map(|n| n.address) == tree.nodes.first().map(|n| n.address)) {
                out.push(tree);
            }
        }
        out
    };
    let mut pairs = 0usize;
    for (plain, blocked) in [
        ("groups-and-datasets.h5", "userblock-512.h5"),
        ("vlen-strings.h5", "vlen-strings-userblock.h5"),
        ("btree-v2-internal.h5", "btree-v2-userblock.h5"),
    ] {
        let (plain, blocked) = (dir.join("hdf5").join(plain), dir.join("hdf5").join(blocked));
        if !blocked.exists() {
            continue;
        }
        let a = trees(&plain, hdf5());
        let b = trees(&blocked, hdf5());
        assert!(!a.is_empty(), "{}: no tree", plain.display());
        assert_eq!(a.len(), b.len(), "{} and {}", plain.display(), blocked.display());
        for (a, b) in a.iter().zip(&b) {
            assert_eq!((a.version, a.nodes.len(), a.omitted), (b.version, b.nodes.len(), b.omitted), "{}", blocked.display());
            for (x, y) in a.nodes.iter().zip(&b.nodes) {
                assert_eq!(x.address + 512, y.address, "{}", blocked.display());
                assert_eq!((x.entries, &x.first_key, &x.last_key), (y.entries, &y.first_key, &y.last_key));
                assert_eq!((x.first_entry_bits, x.entry_bits, x.size_bits), (y.first_entry_bits, y.entry_bits, y.size_bits));
            }
        }
        eprintln!("--- {}: {} trees, the same as its twin 512 bytes on", blocked.display(), b.len());
        pairs += 1;
    }
    let mat = dir.join("mat").join("testhdf5_7.4_GLNX86.mat");
    if mat.exists() {
        let through_hdf5 = trees(&mat, hdf5());
        let through_mat = trees(&mat, qubero_core::formats::mat());
        assert!(!through_mat.is_empty(), "{}: no tree", mat.display());
        let addresses = |t: &[Tree]| t.iter().map(|t| t.nodes.iter().map(|n| n.address).collect::<Vec<_>>()).collect::<Vec<_>>();
        assert_eq!(addresses(&through_hdf5), addresses(&through_mat), "{}", mat.display());
        assert!(through_mat[0].nodes[0].address >= 512);
    }
    assert!(pairs > 0, "no user-block pair in the collection");
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

/// A version 1 node's stride lands on the entries the template placed, and on
/// nothing else.
///
/// The two numbers a node carries are what lets a view divide the box it draws
/// into the entries the node holds, so the thing to check is that dividing on
/// them arrives where the file's own entries are. Four ways of asking, of every
/// tree in every file written by a library old enough to have these nodes at
/// all:
///
/// - Every element of the node's array sits exactly where the stride puts it,
///   not merely the first, which is the one the stride was measured from.
/// - Every entry of an index node ends at the address of the child underneath
///   it, since a child address is the last eight bytes of an entry whichever
///   of the two kinds of key is in front of it. Read out of the file rather
///   than out of the template: a stride that is short by a field reads this
///   from the middle of the entry before it, and no number comes back.
/// - The name at the last place the stride puts is the name the walk says the
///   node's range ends at, for a link table.
/// - The chunk offsets at the last place the stride puts are the offsets the
///   walk says a level-zero chunk node's range ends at.
#[test]
fn a_version_1_node_s_stride_lands_on_its_own_entries() {
    use qubero_core::formats::hdf5_tree::Job;

    let Some(dir) = sample_dir() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let path = dir.join("hdf5").join("groups-and-datasets.h5");
    let file = File::open(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let len = file.metadata().unwrap().len();
    let doc = Document::new(FileSource { file: RefCell::new(file), len });
    let mut ev = Evaluator::new(hdf5());

    // A cursor that has not moved: the root group's tree, which every file
    // with a version 0 superblock has.
    let tree = qubero_core::formats::hdf5_tree::tree(&mut ev, &doc, &[], 4096)
        .expect("walks")
        .expect("a tree");
    assert_eq!(tree.version, 1, "{}: not a version 1 tree", path.display());
    let tables = stride_lands(&mut ev, &doc, &tree, &path);
    assert!(tables > 0, "{}: a version 1 group tree with no link table under it", path.display());

    // And every other version 1 tree in every file to hand, for the sake of
    // the other kind of entry. A chunk tree's entries are the wider of the
    // two: a size, a filter mask and one offset per dimension of the dataset
    // in front of the child address, with the dimension count in a message of
    // an object header above the tree rather than in the node, so a stride
    // taken from the widths a group entry has would be short by all of it.
    //
    // Insisted on now that `chunks-btree-v1.h5` is in the collection: a file
    // written to the default library bound indexes its chunks this way and no
    // other. What was checked is printed as well, because which files are to
    // hand depends on where this is run.
    let mut dirs = vec![PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../web/public"))];
    dirs.push(dir.join("hdf5"));
    if let Ok(extra) = std::env::var("QUBERO_SAMPLES") {
        dirs.extend(extra.split(';').filter(|s| !s.is_empty()).map(PathBuf::from));
    }
    let mut found = Vec::new();
    for dir in &dirs {
        collect(dir, 3, &mut found);
    }
    found.sort();
    found.dedup();
    let (mut groups, mut chunks) = (0usize, 0usize);
    for path in &found {
        let Ok(file) = File::open(path) else { continue };
        let Ok(len) = file.metadata().map(|m| m.len()) else { continue };
        let doc = Document::new(FileSource { file: RefCell::new(file), len });
        let mut ev = Evaluator::new(hdf5());
        // The signature is the first field of a file that opens with one and
        // the second of a file that keeps a user block in front of it.
        let signed = |ev: &mut Evaluator, at: &[usize]| {
            matches!(ev.node(&doc, at).map(|n| n.value), Ok(Value::Magic { ok: true, .. }))
        };
        if !signed(&mut ev, &[0]) && !signed(&mut ev, &[1, 0]) {
            continue;
        }
        let (mut trees, mut seen) = (Vec::new(), 0usize);
        trees1(&mut ev, &doc, &[], &mut seen, &mut trees);
        for at in &trees {
            let Ok(Some(tree)) = qubero_core::formats::hdf5_tree::tree(&mut ev, &doc, at, 4096) else { continue };
            if tree.version != 1 {
                continue;
            }
            match tree.job {
                Job::Chunk => chunks += 1,
                _ => groups += 1,
            }
            stride_lands(&mut ev, &doc, &tree, path);
        }
    }
    eprintln!("--- version 1 trees whose entries were placed by their stride: {groups} group, {chunks} chunk");
    assert!(groups > 0);
    // Only where the collection is to hand: the run above this one has just
    // the two files in `web/public`, and neither has a chunk tree.
    if dir.join("hdf5").join("chunks-btree-v1.h5").exists() {
        assert!(chunks > 0, "the collection has a version 1 chunk tree and the sweep did not reach it");
    }
}

/// The checks above, over one walked tree, answering how many link tables it
/// held. Every version 1 tree to hand is asked the same questions, because
/// what one of these files is made of depends on which library wrote it and
/// what was put in it.
fn stride_lands(
    ev: &mut Evaluator,
    doc: &Document<FileSource>,
    tree: &qubero_core::formats::hdf5_tree::Tree,
    path: &Path,
) -> usize {
    use qubero_core::formats::hdf5_tree::{Job, Kind, NO_PARENT};

    // Every address written inside an HDF5 file counts from the base address
    // its superblock names, and a walked node says where it is in the file, so
    // a child address read out of an entry is short by exactly that. It is 512
    // in the one file here that keeps a user block in front of its superblock
    // and nought in the rest, and leaving it out reads the right bytes and
    // calls them wrong.
    let base = base_of(ev, doc);
    let mut tables = 0usize;
    for (i, node) in tree.nodes.iter().enumerate() {
        // A node with nothing in it says no stride rather than a stride there
        // is nothing at, which is the one answer a view must not divide on.
        if node.entries == 0 {
            assert_eq!((node.first_entry_bits, node.entry_bits), (0, 0), "{}: {:#x}", path.display(), node.address);
            continue;
        }
        assert!(node.entry_bits > 0, "{}: no stride at {:#x}", path.display(), node.address);
        let end = node.first_entry_bits + node.entries * node.entry_bits;
        assert!(
            end <= node.size_bits,
            "{}: the {} entries at {:#x} run {end} bits past a node of {}",
            path.display(),
            node.entries,
            node.address,
            node.size_bits
        );
        // Every one of them, against where the template put it.
        let field = if node.kind == Kind::LinkTable { "symbols" } else { "entries" };
        let array = ev.child_named(doc, &node.path, field).expect("reads").expect("an array");
        for k in 0..node.entries {
            let mut one = array.clone();
            one.push(k as usize);
            let placed = ev.node(doc, &one).expect("reads").offset_bits;
            assert_eq!(
                placed,
                node.address * 8 + node.first_entry_bits + k * node.entry_bits,
                "{}: entry {k} of the node at {:#x} is not where its stride says",
                path.display(),
                node.address
            );
        }
        if node.kind == Kind::LinkTable {
            tables += 1;
            // The name at the last place the stride puts, which is the name
            // the walk says the range ends at.
            let mut last = array.clone();
            last.push(node.entries as usize - 1);
            assert_eq!(name_of(ev, doc, &last), node.last_key, "{}: {:#x}", path.display(), node.address);
            continue;
        }
        // The bottom row of a chunk tree points at the chunks, which are the
        // dataset's payload rather than nodes, so there is nothing under it to
        // check the addresses against. What it has instead is its own key
        // range: the offsets of the last chunk it indexes, eight bytes into
        // the entry the stride puts last, behind the size and the filter mask.
        if tree.job == Job::Chunk && node.level == 0 {
            let at = node.address * 8 + node.first_entry_bits + (node.entries - 1) * node.entry_bits + 64;
            let mut buf = vec![0u8; tree.coords as usize * 8];
            assert!(doc.read_bits(at, tree.coords * 64, &mut buf).is_empty());
            let read: Vec<String> =
                buf.chunks(8).map(|c| u64::from_le_bytes(c.try_into().unwrap()).to_string()).collect();
            assert_eq!(
                read.join(", "),
                node.last_key,
                "{}: the last chunk of the node at {:#x} is not where its stride puts it",
                path.display(),
                node.address
            );
            continue;
        }
        // The child addresses, read out of the file at the end of each entry.
        // Only where every child was reached: a node the cap stopped at still
        // places its entries, and what it cannot do is name all of them.
        if node.truncated {
            continue;
        }
        let children: Vec<u64> =
            tree.nodes.iter().filter(|n| n.parent != NO_PARENT && n.parent == i).map(|n| n.address).collect();
        assert_eq!(children.len(), node.entries as usize, "{}: {:#x}", path.display(), node.address);
        for (j, child) in children.iter().enumerate() {
            let at = node.address * 8 + node.first_entry_bits + (j as u64 + 1) * node.entry_bits - 64;
            let mut buf = [0u8; 8];
            assert!(doc.read_bits(at, 64, &mut buf).is_empty());
            assert_eq!(
                u64::from_le_bytes(buf) + base,
                *child,
                "{}: entry {j} of the node at {:#x} does not end at its child's address",
                path.display(),
                node.address
            );
        }
    }
    tables
}

/// Every version 1 B-tree node the template places under `path`. The tree a
/// walk starts from is the root above whichever of these it is handed, so the
/// nodes below one are looked at again and answer with the same tree; what
/// this is for is finding the trees at all, in a file whose datasets keep
/// theirs a long way down.
fn trees1(
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
    if node.type_name == "BTree" {
        out.push(path.to_vec());
        // And on down through it. A group's datasets hang under the link
        // tables this tree points at, so a walk that stopped at the first
        // `TREE` it met found every group tree in a file and no chunk tree at
        // all: the sweep printed "0 chunk" for a file written with four
        // hundred of them.
    }
    for i in 0..node.child_count as usize {
        let mut p = path.to_vec();
        p.push(i);
        trees1(ev, doc, &p, seen, out);
        if *seen >= BUDGET {
            return;
        }
    }
}

/// The text a variable-length element reads as, where in the file those bytes
/// are, and what the note in the column says about them.
struct VlenString {
    text: String,
    /// Where the string's bytes are, in bytes from the front of the file.
    at: u64,
    /// Where the note that points at them is, which is in the column.
    note_at: u64,
    length: i128,
}

/// A column of variable-length strings reads as the strings, at the places the
/// file put them.
///
/// A variable-length element is sixteen bytes that say how long the string is,
/// which global heap collection holds it and which object of that collection
/// it is. The objects vary in size, so nothing but a walk of the collection
/// finds the one with a given index, and until now the template stopped at the
/// note. Two thousand strings over two collections, with the indices in the
/// order the writing happened rather than in any order arithmetic could guess,
/// is what says the walk finds the right one.
///
/// Both files, because the placement is an address like every other address in
/// this format and counts from where this copy of the file begins. Behind a
/// 512-byte user block that is not the front of the file, and a base counted
/// wrong reads the same on the file without one.
///
/// The count is also what says the reading closes: every note found is one
/// written in a column or an attribute, and every object one of them points at
/// reads as text and holds nothing further. A note reached through another
/// note's object would show up here as more notes than were written.
#[test]
fn a_column_of_variable_length_strings_reads_the_strings_themselves() {
    let Some(dir) = sample_dir() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    for name in ["vlen-strings.h5", "vlen-strings-userblock.h5"] {
        let path = dir.join("hdf5").join(name);
        let Ok(file) = File::open(&path) else {
            eprintln!("skipped: no {}", path.display());
            continue;
        };
        let raw = std::fs::read(&path).expect("reads");
        let len = file.metadata().unwrap().len();
        let doc = Document::new(FileSource { file: RefCell::new(file), len });
        let mut ev = Evaluator::new(hdf5());

        let mut found = Vec::new();
        vlen_strings(&mut ev, &doc, &[], &mut String::new(), &mut found, &path);
        let column: Vec<&VlenString> = found.iter().filter(|s| s.text.starts_with("vl-")).collect();
        eprintln!("--- {}: {} variable-length strings, {} in the column", path.display(), found.len(), column.len());

        assert_eq!(found.len(), 2009, "{}: notes written, not notes reached through notes", path.display());
        assert_eq!(column.len(), 2000, "{}: the column is 2000 strings", path.display());
        assert_eq!(column[0].text, "vl-00000-aaa", "{}", path.display());
        assert_eq!(column[1999].text, "vl-01999-jjj", "{}", path.display());

        // The bytes at the place the template put the string are the string,
        // which is what says the address was worked out and not guessed.
        for s in &column {
            let at = s.at as usize;
            let want = s.text.as_bytes();
            assert_eq!(&raw[at..at + want.len()], want, "{}: {} is not at {at:#x}", path.display(), s.text);
            assert_eq!(s.length, want.len() as i128, "{}: {} says the wrong length", path.display(), s.text);
        }

        // The notes keep their stride. A column of sixteen-byte notes is
        // walked by arithmetic, and a note that has to be read to find out how
        // long it is would mean reading two thousand of them to reach the last.
        let first = column[0].note_at;
        for (i, s) in column.iter().enumerate() {
            assert_eq!(s.note_at, first + 16 * i as u64, "{}: the notes lost their stride at {i}", path.display());
        }

        // Strings the attributes hold, read the same way. One of them is a
        // note in a different object header pointing into the same collection.
        let mut labels: Vec<String> = found.iter().map(|s| s.text.clone()).filter(|t| !t.starts_with("vl-")).collect();
        labels.sort();
        labels.dedup();
        for want in ["first label", "metres per second", "variable-length strings in a global heap"] {
            assert!(labels.contains(&want.to_string()), "{}: no attribute read as {want:?}: {labels:?}", path.display());
        }

        // The cursor on one of those strings lands on it as the note reads it,
        // exactly on its bytes and no wider. The bytes are still counted in
        // the collection and not here, which is what `Field::aside` says; what
        // a reader standing on them is looking at is the string.
        let bit = column[1999].at * 8;
        let landed = ev.locate(&doc, bit).expect("the cursor lands somewhere");
        let node = ev.node(&doc, &landed).expect("and on something that reads");
        assert_eq!(node.offset_bits, bit, "{}: the cursor landed at {:#x}", path.display(), node.offset_bits / 8);
        assert_eq!(node.size_bits, 12 * 8, "{}: the cursor covered {} bits", path.display(), node.size_bits);
        assert_eq!(node.value, Value::Str("vl-01999-jjj".into()), "{}: the cursor landed on {:?}", path.display(), node.value);
    }
}

/// Every variable-length string under `path`, with the name of the dataset or
/// attribute it belongs to.
fn vlen_strings(
    ev: &mut Evaluator,
    doc: &Document<FileSource>,
    path: &[usize],
    name: &mut String,
    out: &mut Vec<VlenString>,
    file: &Path,
) {
    let node = ev
        .node(doc, path)
        .unwrap_or_else(|e| panic!("{}: {path:?} does not read: {e:?}", file.display()));
    if let (Value::Str(s), true) = (&node.value, node.name == "name") {
        if !s.is_empty() {
            *name = s.clone();
        }
    }
    if node.type_name == "GlobalHeapId" {
        let number = |ev: &mut Evaluator, field: &str| -> i128 {
            match ev.child_named(doc, path, field) {
                Ok(Some(p)) => ev.node(doc, &p).map(|n| n.value.as_int().unwrap_or(-1)).unwrap_or(-1),
                _ => -1,
            }
        };
        let length = number(ev, "length");
        let object = ev.child_named(doc, path, "object").ok().flatten().expect("a note carries its object");
        let mut at = object;
        at.push(0);
        if let Ok(inner) = ev.node(doc, &at) {
            if let Value::Str(text) = inner.value {
                out.push(VlenString { text, at: inner.offset_bits / 8, note_at: node.offset_bits / 8, length });
            }
        }
        return;
    }
    for i in 0..node.child_count as usize {
        let mut p = path.to_vec();
        p.push(i);
        vlen_strings(ev, doc, &p, name, out, file);
    }
}

/// What the file's superblock says its addresses count from. Nought for a file
/// that begins with its signature, and whatever is in front of the superblock
/// for a file that keeps a user block there.
fn base_of(ev: &mut Evaluator, doc: &Document<FileSource>) -> u64 {
    // Two or three levels down, depending on the file: a file that opens with
    // its signature has the superblock among the root's fields, and one with a
    // user block in front has the whole of that file inside a field of its
    // own.
    let mut at: Vec<Vec<usize>> = vec![Vec::new()];
    for _ in 0..3 {
        let mut below = Vec::new();
        for path in at {
            if let Ok(Some(field)) = ev.child_named(doc, &path, "base_address") {
                if let Ok(node) = ev.node(doc, &field) {
                    if let Some(base) = node.value.as_int() {
                        return u64::try_from(base).unwrap_or(0);
                    }
                }
            }
            let Ok(node) = ev.node(doc, &path) else { continue };
            for i in 0..node.child_count as usize {
                let mut one = path.clone();
                one.push(i);
                below.push(one);
            }
        }
        at = below;
    }
    0
}

/// The link name of a symbol table entry, which the template reads out of the
/// group's heap and places under a field of no bytes of its own.
fn name_of(ev: &mut Evaluator, doc: &Document<FileSource>, entry: &[usize]) -> String {
    let Ok(Some(field)) = ev.child_named(doc, entry, "name") else { return String::new() };
    let mut at = field;
    // Down through whatever stands between the field and the text: the field
    // is placed at an address, and what is at that address is the string.
    for _ in 0..4 {
        match ev.node(doc, &at) {
            Ok(node) => {
                if let Value::Str(text) = node.value {
                    return text;
                }
                if node.child_count == 0 {
                    return String::new();
                }
                at.push(0);
            }
            Err(_) => return String::new(),
        }
    }
    String::new()
}

/// A row of `compound-and-vlen-seq.h5`'s `table` as `make_hdf5_samples.py`
/// writes it and h5py reads it back, written out the way [`shown`] writes a
/// record: `(100, 0.0, b'row-0', (-33.5, 151.25), [0, 0, 0])` is
/// `{id: 100, x: 0, label: row-0, pos: {lat: -33.5, lon: 151.25}, samples: [0, 0, 0]}`.
fn table_row(i: i64) -> String {
    let f = |v: f64| format!("{v}");
    format!(
        "{{id: {}, x: {}, label: row-{i}, pos: {{lat: {}, lon: {}}}, samples: [{i}, {}, {}]}}",
        100 + i,
        f(i as f64 * 1.25),
        f(-33.5 + i as f64),
        f(151.25 - i as f64),
        -i,
        i * i
    )
}

/// A value the template read, written out: a number or a string as itself, a
/// record of a compound as its members by name, a list as its elements.
///
/// A member's name is read off its label, `[2] label`, which is what says the
/// label is the member's own name and not an index alone.
fn shown(ev: &mut Evaluator, doc: &Document<FileSource>, path: &[usize]) -> String {
    let node = ev.node(doc, path).unwrap_or_else(|e| panic!("{path:?} does not read: {e:?}"));
    match &node.value {
        Value::Int(v) => return v.to_string(),
        Value::UInt(v) => return v.to_string(),
        Value::Float(v) => return format!("{v}"),
        Value::Str(s) => return s.clone(),
        Value::Bytes { len: 0, .. } => return "[]".into(),
        _ => {}
    }
    if node.type_name == "Record" {
        let values = ev.child_named(doc, path, "values").expect("reads").expect("a record has values");
        let n = ev.node(doc, &values).expect("reads").child_count;
        let mut parts = Vec::new();
        for j in 0..n as usize {
            let mut one = values.clone();
            one.push(j);
            let label = ev.node(doc, &one).expect("reads").name;
            let prefix = format!("[{j}] ");
            let name = label.strip_prefix(&prefix).unwrap_or_else(|| panic!("{one:?} is labelled {label:?}"));
            parts.push(format!("{name}: {}", shown(ev, doc, &one)));
        }
        return format!("{{{}}}", parts.join(", "));
    }
    let mut parts = Vec::new();
    for j in 0..node.child_count as usize {
        let mut one = path.to_vec();
        one.push(j);
        parts.push(shown(ev, doc, &one));
    }
    format!("[{}]", parts.join(", "))
}

/// Every run of elements under `path`, and whether it is an attribute's. The
/// copies of datatype messages and the heap collections are passed over: they
/// hold no runs, and the second is most of the file.
fn runs(ev: &mut Evaluator, doc: &Document<FileSource>, path: &[usize], attribute: bool, out: &mut Vec<(Vec<usize>, bool)>) {
    let Ok(node) = ev.node(doc, path) else { return };
    if node.name == "elements" && node.composite {
        out.push((path.to_vec(), attribute));
        return;
    }
    if node.name == "datatype" || node.name == "collection" {
        return;
    }
    let attribute = attribute || node.type_name == "Attribute";
    for i in 0..node.child_count as usize {
        let mut p = path.to_vec();
        p.push(i);
        runs(ev, doc, &p, attribute, out);
    }
}

/// The version nibble of the datatype message a run of elements was read by:
/// the copy its run keeps, or an attribute's own.
fn datatype_version(ev: &mut Evaluator, doc: &Document<FileSource>, run: &[usize], attribute: bool) -> i128 {
    let up = if attribute { &run[..run.len() - 2] } else { &run[..run.len() - 1] };
    let datatype = ev.child_named(doc, up, "datatype").expect("reads").expect("a datatype in reach");
    let mut at = datatype;
    if ev.child_named(doc, &at, "version").expect("reads").is_none() {
        at.push(0);
    }
    let version = ev.child_named(doc, &at, "version").expect("reads").expect("a version");
    ev.node(doc, &version).expect("reads").value.as_int().expect("a number")
}

/// The object header of every object `h5ad::contents` finds, by the path the
/// file names it by.
fn objects(ev: &mut Evaluator, doc: &Document<FileSource>) -> Vec<(String, Vec<usize>)> {
    let contents = qubero_core::formats::h5ad::contents(ev, doc).expect("contents");
    contents.objects.into_iter().map(|o| (o.name, o.path)).collect()
}

/// A compound element reads as its members, each at its own offset and read by
/// its own datatype, labelled with its name, and the numbers are the ones h5py
/// reads.
///
/// `compound-and-vlen-seq.h5` holds every shape a compound's datatype message
/// takes. The default file writes `table` as version 2, because it holds an
/// array member, and `pos` inside it as version 2 too, since the library raises
/// the compounds inside a type along with it; the `calibration` attribute holds
/// no array and is version 1. The latest-bound file writes all three as version
/// 5, and HDF5 2.0 writes a compound of that version the way version 3 does,
/// names unpadded and offsets one byte wide.
/// `table` is aligned, so its members have room between them, and
/// `calibration` lists `gain` first though its bytes come second. The same rows
/// are read again through chunks: a version 1 b-tree in the default file and an
/// extensible array in the other.
#[test]
fn a_compound_element_reads_as_its_members_by_name() {
    let Some(dir) = sample_dir() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let mut checked = 0usize;
    for (name, table_version, pos_version, calibration_version) in
        [("compound-and-vlen-seq.h5", 2, 2, 1), ("compound-and-vlen-seq-latest.h5", 5, 5, 5)]
    {
        let path = dir.join("hdf5").join(name);
        let Ok(file) = File::open(&path) else {
            eprintln!("skipped: no {}", path.display());
            continue;
        };
        let len = file.metadata().unwrap().len();
        let doc = Document::new(FileSource { file: RefCell::new(file), len });
        let mut ev = Evaluator::new(hdf5());
        let objects = objects(&mut ev, &doc);
        let header = |want: &str| objects.iter().find(|(n, _)| n == want).map(|(_, p)| p.clone()).expect(want);

        // The contiguous table and its attribute.
        let (table, chunked) = (header("/table"), header("/table_chunked"));
        let mut found = Vec::new();
        runs(&mut ev, &doc, &table, false, &mut found);
        let data: Vec<_> = found.iter().filter(|(_, a)| !a).cloned().collect();
        let attrs: Vec<_> = found.iter().filter(|(_, a)| *a).cloned().collect();
        assert_eq!((data.len(), attrs.len()), (1, 1), "{}: {found:?}", path.display());
        let (run, _) = &data[0];
        assert_eq!(datatype_version(&mut ev, &doc, run, false), table_version, "{}", path.display());
        let run_at = ev.node(&doc, run).expect("reads").offset_bits;
        for i in 0..5 {
            let mut row = run.clone();
            row.push(i);
            assert_eq!(shown(&mut ev, &doc, &row), table_row(i as i64), "{}: row {i}", path.display());
            // Where each member is: its offset in the element, which the
            // alignment leaves room between.
            let values = ev.child_named(&doc, &row, "values").unwrap().unwrap();
            for (j, offset) in [0u64, 8, 16, 24, 32].into_iter().enumerate() {
                let mut one = values.clone();
                one.push(j);
                let at = ev.node(&doc, &one).expect("reads").offset_bits;
                assert_eq!(at, run_at + (i as u64 * 40 + offset) * 8, "{}: row {i} member {j}", path.display());
            }
            // The nested record keeps its own datatype, and says which
            // version it is.
            let mut pos = values.clone();
            pos.push(3);
            let copy = ev.child_named(&doc, &pos, "datatype").unwrap().expect("a nested record places its datatype");
            let mut at = copy;
            at.push(0);
            let version = ev.child_named(&doc, &at, "version").unwrap().unwrap();
            assert_eq!(ev.node(&doc, &version).unwrap().value.as_int(), Some(pos_version), "{}", path.display());
        }
        let label = {
            let mut one = run.clone();
            one.extend_from_slice(&[2, 0, 2]);
            ev.node(&doc, &one).expect("reads").name
        };
        assert_eq!(label, "[2] label", "{}", path.display());

        let (run, _) = &attrs[0];
        assert_eq!(datatype_version(&mut ev, &doc, run, true), calibration_version, "{}", path.display());
        assert_eq!(shown(&mut ev, &doc, run), "[{gain: 1.5, offset: -2}, {gain: 0.25, offset: 7}]", "{}", path.display());
        // Listed first, placed second.
        let at = ev.node(&doc, run).expect("reads").offset_bits;
        let mut gain = run.clone();
        gain.extend_from_slice(&[1, 0, 0]);
        let mut offset = run.clone();
        offset.extend_from_slice(&[1, 0, 1]);
        assert_eq!(ev.node(&doc, &gain).unwrap().offset_bits, at + (8 + 4) * 8, "{}", path.display());
        assert_eq!(ev.node(&doc, &offset).unwrap().offset_bits, at + 8 * 8, "{}", path.display());

        // The chunked copy, two rows a chunk.
        let mut found = Vec::new();
        runs(&mut ev, &doc, &chunked, false, &mut found);
        let mut rows = Vec::new();
        for (run, _) in found.iter().filter(|(_, a)| !a) {
            let n = ev.node(&doc, run).expect("reads").child_count as usize;
            for i in 0..n {
                let mut row = run.clone();
                row.push(i);
                rows.push(shown(&mut ev, &doc, &row));
            }
        }
        // A chunk is whole: the last one holds row 4 and a row of fill.
        assert_eq!(rows.len(), 6, "{}: {rows:?}", path.display());
        assert_eq!(&rows[..5], &(0..5).map(table_row).collect::<Vec<_>>()[..], "{}", path.display());
        checked += 1;
    }
    if checked == 0 {
        eprintln!("skipped: no compound-and-vlen-seq.h5 in the collection");
    }
}

/// A variable-length sequence reads as its elements, typed by the datatype
/// inside the variable-length one, over the heap object's own bytes.
///
/// A sequence element is the same sixteen-byte note a string is, and its
/// `length` counts elements rather than bytes. `sequences` in both
/// `compound-and-vlen-seq` files holds four runs of 32-bit integers, which
/// `make_hdf5_samples.py` writes as `arange(n) * 7 - 3` for n of 0, 1, 4 and 9
/// and h5py reads back the same. The empty one is written with address
/// nought, which is not a collection, and points at nothing.
#[test]
fn a_variable_length_sequence_reads_as_its_elements() {
    let Some(dir) = sample_dir() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let want: Vec<Vec<i64>> = [0i64, 1, 4, 9].iter().map(|&n| (0..n).map(|k| k * 7 - 3).collect()).collect();
    let mut checked = 0usize;
    for name in ["compound-and-vlen-seq.h5", "compound-and-vlen-seq-latest.h5"] {
        let path = dir.join("hdf5").join(name);
        let Ok(file) = File::open(&path) else {
            eprintln!("skipped: no {}", path.display());
            continue;
        };
        let raw = std::fs::read(&path).expect("reads");
        let len = file.metadata().unwrap().len();
        let doc = Document::new(FileSource { file: RefCell::new(file), len });
        let mut ev = Evaluator::new(hdf5());
        let objects = objects(&mut ev, &doc);
        let header = objects.iter().find(|(n, _)| n == "/sequences").map(|(_, p)| p.clone()).expect("/sequences");
        let mut found = Vec::new();
        runs(&mut ev, &doc, &header, false, &mut found);
        assert_eq!(found.len(), 1, "{}: {found:?}", path.display());
        let (run, _) = &found[0];
        assert_eq!(ev.node(&doc, run).expect("reads").child_count, 4, "{}", path.display());
        for (i, want) in want.iter().enumerate() {
            let mut note = run.clone();
            note.push(i);
            let length = ev.child_named(&doc, &note, "length").unwrap().unwrap();
            let length = ev.node(&doc, &length).unwrap();
            assert_eq!(length.value.as_int(), Some(want.len() as i128), "{}: sequence {i}", path.display());
            let mut object = ev.child_named(&doc, &note, "object").unwrap().expect("a note carries its object");
            if want.is_empty() {
                assert_eq!(shown(&mut ev, &doc, &object), "[]", "{}: sequence {i}", path.display());
                continue;
            }
            object.push(0);
            let elements = ev.node(&doc, &object).expect("reads");
            assert_eq!(elements.type_name, "i32 le[]", "{}: sequence {i}", path.display());
            let text: Vec<String> = want.iter().map(|v| v.to_string()).collect();
            assert_eq!(shown(&mut ev, &doc, &object), format!("[{}]", text.join(", ")), "{}: sequence {i}", path.display());
            // Over the heap object's own bytes.
            let at = (elements.offset_bits / 8) as usize;
            let bytes: Vec<u8> = want.iter().flat_map(|v| (*v as i32).to_le_bytes()).collect();
            assert_eq!(&raw[at..at + bytes.len()], &bytes[..], "{}: sequence {i} is not at {at:#x}", path.display());
            assert_eq!(elements.size_bits, bytes.len() as u64 * 8, "{}: sequence {i}", path.display());
        }
        checked += 1;
    }
    if checked == 0 {
        eprintln!("skipped: no compound-and-vlen-seq.h5 in the collection");
    }
}
