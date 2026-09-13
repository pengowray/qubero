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
