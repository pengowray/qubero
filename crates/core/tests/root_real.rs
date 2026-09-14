//! The ROOT files in the sample collection, which is not in this repository:
//! every `*.root` under a directory `QUBERO_SAMPLES` names (several, separated
//! by `;`), or under `qubero-samples` beside the checkout. Skips when there is
//! none.
//!
//! What a made-up file cannot show is that the compressed stream inside a
//! record is the stream the compressor really wrote. These five were written
//! by five releases of ROOT with four different compressors, so a block header
//! that named the algorithm and then handed on the wrong bytes shows up here:
//! an xz stream is placed by reading its footer backwards from the end of the
//! window the block gave it, and a window one byte out reads as nothing.
//!
//! The second half of this file is the reader beside the template: the class
//! descriptions out of `StreamerInfo`, the trees read with them, and the
//! baskets the directory never lists. Every number checked there was read out
//! of the file twice, once by this crate and once by uproot 5.7.6 (BSD-3), and
//! the two agreed. That is what makes a basket offset worth writing down: it
//! is not what this reader happened to produce, it is where the basket is.

use std::path::{Path, PathBuf};

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats::{root, root_tree};
use qubero_core::source::MemSource;

/// The header's fields, and the record each of the last three places. The
/// class descriptions are declared before the directory, so that every object
/// under the directory can walk back to them.
const DIRECTORY: [usize; 2] = [17, 0];
/// A record's own fields: twelve of key, and then whatever it holds.
const K_FIELDS: usize = 12;

#[test]
fn reads_the_real_files_and_what_is_inside_their_blocks() {
    let mut found = Vec::new();
    for dir in dirs() {
        collect(&dir, 3, &mut found);
    }
    if found.is_empty() {
        eprintln!("skipped: no ROOT file in hand. Set QUBERO_SAMPLES to a directory holding one.");
        return;
    }
    found.sort();
    let mut algorithms = Vec::new();
    let mut nested = 0;
    for path in &found {
        let (algorithm, dirs) = check(path);
        algorithms.push(algorithm);
        nested += dirs;
    }
    // Between them the samples cover every algorithm ROOT writes today, and a
    // file with no compression at all.
    eprintln!("{} files read, {nested} subdirectories walked: {algorithms:?}", found.len());
    assert!(nested > 0, "no sample here has a subdirectory, so nothing tested the walk");
}

/// Reads one file, and answers with the algorithm of the first compressed
/// block found in it and how many subdirectories the walk went into.
fn check(path: &Path) -> (Option<String>, usize) {
    let bytes = std::fs::read(path).expect("reads");
    let d = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(root());
    let say = |what: &str| format!("{}: {what}", path.display());

    // The header, and the top directory's record at the offset it gives.
    assert!(
        matches!(ev.node(&d, &[0]).unwrap().value, Value::Magic { ok: true, .. }),
        "{}",
        say("not a ROOT file")
    );
    let dir = ev.node(&d, &DIRECTORY).unwrap();
    assert_eq!(dir.offset_bits / 8, 100, "{}", say("the top record is not at fBEGIN"));

    // The directory's key list, and every key in it.
    let keys = [DIRECTORY.as_slice(), &[K_FIELDS + 2, 11, 0, K_FIELDS + 1]].concat();
    let n = ev.node(&d, &keys).unwrap().child_count;
    assert!(n > 0, "{}", say("a file with no keys in its top directory"));

    let mut dirs = 0;
    let algorithm = walk(&d, &mut ev, &keys, path, &mut dirs);
    eprintln!("--- {}: {n} keys, first block {algorithm:?}", path.display());
    (algorithm, dirs)
}

/// Every key of one key list, and every key of every directory under it. The
/// depth is the template's own: past [`MAX_DEPTH`] a directory key is a plain
/// record and has no key list to find, which is what stops this.
fn walk(
    d: &Document<MemSource>,
    ev: &mut Evaluator,
    keys: &[usize],
    path: &Path,
    dirs: &mut usize,
) -> Option<String> {
    let say = |what: &str| format!("{}: {what}", path.display());
    let n = ev.node(d, keys).unwrap().child_count;
    let mut algorithm = None;
    for i in 0..n as usize {
        let entry = [keys, &[i]].concat();
        // Every key names a class and an object, which is what makes the
        // listing readable.
        let class = ev.node(d, &[entry.as_slice(), &[9, 1]].concat()).unwrap().value;
        let Value::Str(class) = class else { panic!("{}", say("a key with no class name")) };
        assert!(!class.is_empty(), "{}", say("a key whose class name is empty"));
        let name = ev.node(d, &[entry.as_slice(), &[10, 1]].concat()).unwrap().value;
        assert!(matches!(name, Value::Str(_)), "{}", say("a key with no object name"));

        // What the key points at. A directory is walked into; anything else
        // is a record whose body is blocks or bytes.
        let body = [entry.as_slice(), &[K_FIELDS, 0, K_FIELDS]].concat();
        let body_node = ev.node(d, &body).unwrap();
        if class.starts_with("TDirectory") {
            // A directory record holds the sixty bytes of a directory, and
            // its key list is read the same way the top one was.
            assert_eq!(body_node.size_bits, 60 * 8, "{}", say("a directory record that is not sixty bytes"));
            *dirs += 1;
            let inner = [body.as_slice(), &[11, 0, K_FIELDS + 1]].concat();
            let m = ev.node(d, &inner).unwrap().child_count;
            eprintln!("--- {}: {class} {name:?} holds {m} keys", path.display());
            algorithm = algorithm.or_else(|| walk(d, ev, &inner, path, dirs));
            continue;
        }
        if class.contains("RNTuple") {
            algorithm = algorithm.or_else(|| anchor(d, ev, &body, path));
            continue;
        }
        for b in 0..body_node.child_count as usize {
            algorithm = algorithm.or_else(|| block(d, ev, &[body.as_slice(), &[b]].concat(), path));
        }
    }
    algorithm
}

/// One compressed block: the algorithm it names, and the stream inside it read
/// by that format's own template.
fn block(d: &Document<MemSource>, ev: &mut Evaluator, at: &[usize], path: &Path) -> Option<String> {
    let value = ev.node(d, &[at, &[0]].concat()).ok()?.value;
    let Value::Enum { name: Some(name), .. } = value else { return None };
    let stream = [at, &[4]].concat();
    let node = ev.node(d, &stream).unwrap();
    let compressed = ev.node(d, &[at, &[2]].concat()).unwrap().value.as_int().unwrap();
    assert_eq!(node.size_bits / 8, compressed as u64, "{}: the stream is not as long as the block says", path.display());
    // The first field of each of these is what says the template found the
    // stream it was handed: a magic number, or zlib's own two bytes.
    match name.as_str() {
        // An xz stream begins with its own magic; a zstd stream is a list of
        // frames, and the magic is the first field of the first of them.
        "xz" | "zstd" => {
            let to_magic: &[usize] = match name.as_str() {
                "zstd" => &[0, 0, 0],
                _ => &[0],
            };
            let magic = ev.node(d, &[stream.as_slice(), to_magic].concat()).unwrap().value;
            assert!(
                matches!(magic, Value::Magic { ok: true, .. }),
                "{}: an {name} block whose stream does not start with {name}",
                path.display()
            );
            // An xz stream is placed by reading its footer backwards from the
            // end of the window the block gave it, so a window a byte out
            // reads as nothing. The footer is the ninth field and is twelve
            // bytes wherever it lands.
            if name == "xz" {
                let footer = ev.node(d, &[stream.as_slice(), &[8]].concat()).unwrap();
                assert_eq!(footer.size_bits, 12 * 8, "{}: an xz footer that is not twelve bytes", path.display());
                let end = (footer.offset_bits + footer.size_bits) / 8;
                assert_eq!(
                    end,
                    (node.offset_bits + node.size_bits) / 8,
                    "{}: the xz stream does not end where the block does",
                    path.display()
                );
            }
        }
        "zlib" => {
            let method = ev.node(d, &[stream.as_slice(), &[1]].concat()).unwrap().value;
            assert_eq!(
                method,
                Value::Enum { raw: 8, name: Some("deflate".into()), hex: false },
                "{}: a ZL block that is not a zlib stream",
                path.display()
            );
        }
        "lz4" => {
            // ROOT's lz4 is a checksum and a bare block, so what proves it was
            // read right is the arithmetic: eight bytes of hash, and the rest
            // of the block after it.
            ev.node(d, &[stream.as_slice(), &[0]].concat()).unwrap();
            let raw = ev.node(d, &[stream.as_slice(), &[1]].concat()).unwrap();
            assert_eq!(
                raw.size_bits / 8,
                compressed as u64 - 8,
                "{}: an L4 block whose raw block is not what is left after the hash",
                path.display()
            );
        }
        _ => {}
    }
    Some(name)
}

/// The anchor of an RNTuple, and the two envelopes it points at.
///
/// The anchor is often compressed, and then the numbers saying where the
/// envelopes are do not exist in the file at all: they are bytes of the
/// stream. Reading them means opening the stream, and the offsets they hold
/// are still offsets of the file, so the envelopes land where the anchor says
/// whether or not the anchor was written in the clear. That is the whole of
/// what a decoded stream is for, on a file nobody here wrote.
fn anchor(d: &Document<MemSource>, ev: &mut Evaluator, at: &[usize], path: &Path) -> Option<String> {
    let node = ev.node(d, at).unwrap();
    let mut at = at.to_vec();
    let mut algorithm = None;
    if node.size_bits != 78 * 8 || node.child_count != 17 {
        algorithm = (0..node.child_count as usize).find_map(|b| block(d, ev, &[at.as_slice(), &[b]].concat(), path));
        let found = anchor_inside(d, ev, &at, 8);
        let Some(found) = found else {
            panic!("{}: a compressed anchor whose seventeen numbers never turned up", path.display())
        };
        eprintln!("--- {}: the anchor is compressed, and opens into its seventeen numbers", path.display());
        // The numbers are read out of the stream, so they are not bytes of the
        // file and say so.
        assert_ne!(ev.node(d, &found).unwrap().space, 0, "{}: an opened anchor still in the file's space", path.display());
        at = found;
    }
    let field = |ev: &mut Evaluator, i: usize| ev.node(d, &[at.as_slice(), &[i]].concat()).unwrap().value.as_int().unwrap();
    assert_eq!(field(ev, 3), 1, "{}: an anchor whose epoch is not 1", path.display());
    for (i, envelope) in [(7usize, 15usize), (10, 16)] {
        let seek = field(ev, i);
        let nbytes = field(ev, i + 1);
        let placed = ev.node(d, &[at.as_slice(), &[envelope, 0]].concat()).unwrap();
        assert_eq!(placed.offset_bits / 8, seek as u64, "{}: an envelope is not where the anchor says", path.display());
        assert_eq!(placed.size_bits / 8, nbytes as u64, "{}: an envelope is not as long as the anchor says", path.display());
        // An envelope is at an offset of the file, whatever space the number
        // naming it was read in. This is the one thing a compressed anchor
        // could get wrong and the only way to tell: an offset read inside the
        // stream and then followed inside the stream would land nowhere.
        assert_eq!(placed.space, 0, "{}: an envelope placed outside the file", path.display());
    }
    eprintln!("--- {}: an RNTuple anchor, both envelopes placed", path.display());
    algorithm
}

/// The anchor structure inside whatever the block turned out to hold. Which
/// path reaches it depends on the codec, so it is looked for rather than
/// spelled: the anchor is the one node with seventeen children called by its
/// own name.
fn anchor_inside(d: &Document<MemSource>, ev: &mut Evaluator, at: &[usize], depth: u32) -> Option<Vec<usize>> {
    if depth == 0 {
        return None;
    }
    let node = ev.node(d, at).ok()?;
    if node.type_name == "RNTupleAnchor" && node.child_count == 17 {
        return Some(at.to_vec());
    }
    // Wide as well as deep would be a walk over the whole record. Every step
    // from a block to the anchor is a wrapper holding a handful of fields.
    for i in 0..node.child_count.min(16) as usize {
        if let Some(found) = anchor_inside(d, ev, &[at, &[i]].concat(), depth - 1) {
            return Some(found);
        }
    }
    None
}

fn dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(named) = std::env::var("QUBERO_SAMPLES") {
        out.extend(named.split(';').filter(|s| !s.is_empty()).map(PathBuf::from));
    }
    out.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../qubero-samples"));
    out.retain(|p| p.is_dir());
    out
}

fn collect(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            if depth > 0 {
                collect(&path, depth - 1, out);
            }
        } else if path.extension().is_some_and(|x| x == "root") {
            out.push(path);
        }
    }
}

fn root_samples() -> Option<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(paths) = std::env::var("QUBERO_SAMPLES") {
        roots.extend(paths.split(';').filter(|s| !s.is_empty()).map(PathBuf::from));
    }
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../qubero-samples"));
    roots.into_iter().map(|p| p.join("root")).find(|p| p.is_dir())
}

fn contents_of(folder: &PathBuf, name: &str) -> (Document<MemSource>, root_tree::Contents) {
    let path = folder.join(name);
    let doc = Document::new(MemSource(std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))));
    let mut ev = Evaluator::new(root());
    let contents = root_tree::contents(&mut ev, &doc).unwrap_or_else(|e| panic!("{}: {e:?}", path.display()));
    (doc, contents)
}

/// The schema is the first thing read and everything else waits on it, so it
/// is checked on its own: the classes a TTree file must describe, and the
/// members of the one class the basket offsets come out of.
#[test]
fn the_streamer_info_describes_the_classes_a_tree_is_made_of() {
    let Some(root) = root_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    let (_, contents) = contents_of(&root, "uproot-Zmumu-lz4.root");
    assert_eq!(contents.trouble, None);
    assert_eq!(contents.classes.len(), 18);
    for want in ["TTree", "TBranch", "TLeaf", "TLeafD", "TLeafI", "TLeafC", "TObjArray", "TNamed", "TObject"] {
        assert!(contents.classes.iter().any(|c| c.name == want), "no description of {want}");
    }
    let tree = contents.classes.iter().find(|c| c.name == "TTree").expect("TTree");
    assert_eq!(tree.version, 19);
    assert_eq!(tree.checksum, 0x58a3_96eb);

    let branch = contents.classes.iter().find(|c| c.name == "TBranch").expect("TBranch");
    assert_eq!(branch.version, 12);
    // The three arrays a basket is found through, as the file describes them:
    // an int* and two long long*, each counted by fMaxBaskets.
    let seek = branch.members.iter().find(|m| m.name == "fBasketSeek").expect("fBasketSeek");
    assert_eq!(seek.type_name, "Long64_t*");
    assert_eq!(seek.code, 56);
    let bytes = branch.members.iter().find(|m| m.name == "fBasketBytes").expect("fBasketBytes");
    assert_eq!(bytes.type_name, "int*");
    assert_eq!(bytes.code, 43);
    // The first two members of a TBranch are base classes, not members.
    assert!(branch.members[0].base && branch.members[0].name == "TNamed");
    assert!(branch.members[1].base && branch.members[1].name == "TAttFill");

    // A fixed array is the one shape whose dimensions have to survive the
    // read: TStreamerElement's own fMaxIndex is five ints.
    let element = contents.classes.iter().find(|c| c.name == "TLeaf").expect("TLeaf");
    assert!(element.members.iter().any(|m| m.name == "fLeafCount" && m.type_name == "TLeaf*"));
}

/// A tree of twenty simple branches, one basket each, read start to finish.
/// The offsets and lengths are uproot's `branch.member('fBasketSeek')` and
/// `fBasketBytes`; the values are `branch.array()`.
#[test]
fn the_zmumu_tree_reads_its_branches_baskets_and_values() {
    let Some(root) = root_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    let (doc, contents) = contents_of(&root, "uproot-Zmumu-lz4.root");
    assert_eq!(contents.trees.len(), 1);
    let tree = &contents.trees[0];
    assert_eq!(tree.name, "events");
    assert_eq!(tree.title, "Z -> mumu events");
    assert_eq!(tree.entries, 2304);
    assert_eq!(tree.trouble, None);
    assert_eq!(tree.branch_total, 20);
    let names: Vec<&str> = tree.branches.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Type", "Run", "Event", "E1", "px1", "py1", "pz1", "pt1", "eta1", "phi1", "Q1", "E2", "px2", "py2",
            "pz2", "pt2", "eta2", "phi2", "Q2", "M"
        ]
    );
    // Every branch has exactly one basket, and the file is 213 KB of them.
    assert!(tree.branches.iter().all(|b| b.basket_total == 1));
    let baskets: Vec<(u64, i64)> = tree.branches.iter().map(|b| (b.baskets[0].at, b.baskets[0].bytes)).collect();
    assert_eq!(baskets[0], (224, 9741), "Type");
    assert_eq!(baskets[1], (9965, 143), "Run");
    assert_eq!(baskets[2], (10108, 3263), "Event");
    assert_eq!(baskets[3], (13371, 13948), "E1");
    assert_eq!(baskets[19], (188177, 18502), "M");
    let named: i64 = baskets.iter().map(|(_, n)| n).sum();
    assert_eq!(named, 206_455, "the baskets are 202 KB of a 208 KB file");

    // The values, for the branches whose baskets hold numbers.
    let run = &tree.branches[1];
    let data = root_tree::branch_basket(&doc, run, 0).expect("Run basket");
    assert_eq!(data.entries, 2304);
    assert!(data.offsets.is_empty(), "a fixed-width branch writes no entry offsets");
    match data.values {
        root_tree::Values::Ints(v) => {
            assert_eq!(v.len(), 2304);
            assert_eq!(&v[..3], &[148031, 148031, 148031]);
        }
        other => panic!("Run came back as {other:?}"),
    }
    let e1 = &tree.branches[3];
    match root_tree::branch_basket(&doc, e1, 0).expect("E1 basket").values {
        root_tree::Values::Floats(v) => {
            assert_eq!(v.len(), 2304);
            assert!((v[0] - 82.201_866_387_5).abs() < 1e-9, "E1[0] was {}", v[0]);
            assert!((v[1] - 62.344_928_948_1).abs() < 1e-9, "E1[1] was {}", v[1]);
        }
        other => panic!("E1 came back as {other:?}"),
    }
    let m = &tree.branches[19];
    match root_tree::branch_basket(&doc, m, 0).expect("M basket").values {
        root_tree::Values::Floats(v) => assert!((v[0] - 82.462_691_56).abs() < 1e-6, "M[0] was {}", v[0]),
        other => panic!("M came back as {other:?}"),
    }

    // The one branch of this tree whose entries vary in length says so rather
    // than handing back numbers, and its entry offsets are still read.
    let text = &tree.branches[0];
    assert!(matches!(&text.reading, root_tree::Reading::Not(why) if why.contains("variable-length")));
    let data = root_tree::branch_basket(&doc, text, 0).expect("Type basket");
    assert_eq!(data.offsets.len(), 2305, "one per entry and one for the end");
    assert!(matches!(data.values, root_tree::Values::None(_)));
}

/// The same tree written with each of the three compressors ROOT uses. What is
/// being checked is that unpacking a record does not change what is in it: the
/// branch names, the entry count and the values are the same three ways, and
/// only the offsets differ.
#[test]
fn the_same_tree_reads_the_same_through_lz4_lzma_and_zstd() {
    let Some(root) = root_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    for (name, first_basket) in
        [("uproot-Zmumu-lz4.root", 224u64), ("uproot-Zmumu-lzma.root", 226), ("uproot-Zmumu-zstd.root", 254)]
    {
        let (doc, contents) = contents_of(&root, name);
        assert_eq!(contents.trouble, None, "{name}");
        let tree = &contents.trees[0];
        assert_eq!(tree.name, "events", "{name}");
        assert_eq!(tree.entries, 2304, "{name}");
        assert_eq!(tree.branches.len(), 20, "{name}");
        assert_eq!(tree.branches[0].baskets[0].at, first_basket, "{name}");
        match root_tree::branch_basket(&doc, &tree.branches[1], 0).expect(name).values {
            root_tree::Values::Ints(v) => assert_eq!(&v[..3], &[148031, 148031, 148031], "{name}"),
            other => panic!("{name}: Run came back as {other:?}"),
        }
        match root_tree::branch_basket(&doc, &tree.branches[3], 0).expect(name).values {
            root_tree::Values::Floats(v) => {
                assert!((v[0] - 82.201_866_387_5).abs() < 1e-9, "{name}: E1[0] was {}", v[0])
            }
            other => panic!("{name}: E1 came back as {other:?}"),
        }
    }
}

/// A tree with one branch of every simple shape: single values, fixed arrays,
/// strings and variable-length arrays side by side. Each of the four shapes
/// reads or says why it does not.
#[test]
fn a_flat_tree_reads_single_values_and_fixed_arrays_and_refuses_the_rest() {
    let Some(root) = root_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    let (doc, contents) = contents_of(&root, "uproot-small-flat-tree.root");
    let tree = &contents.trees[0];
    assert_eq!(tree.name, "tree");
    assert_eq!(tree.entries, 100);
    assert_eq!(tree.branches.len(), 20);

    let by_name = |n: &str| tree.branches.iter().find(|b| b.name == n).unwrap_or_else(|| panic!("no branch {n}"));

    // Single values, one per entry, of six widths.
    for (name, at) in [
        ("Int32", 258u64),
        ("Int64", 502),
        ("UInt32", 761),
        ("UInt64", 1006),
        ("Float32", 1266),
        ("Float64", 1551),
    ] {
        let branch = by_name(name);
        assert_eq!(branch.baskets[0].at, at, "{name}");
        assert_eq!(branch.baskets[0].entries, 100, "{name}");
        let data = root_tree::branch_basket(&doc, branch, 0).expect(name);
        match data.values {
            root_tree::Values::Ints(v) => assert_eq!(&v[..3], &[0, 1, 2], "{name}"),
            root_tree::Values::Floats(v) => assert_eq!(&v[..3], &[0.0, 1.0, 2.0], "{name}"),
            other => panic!("{name}: {other:?}"),
        }
    }

    // A fixed array: ten copies of the entry number.
    let array = by_name("ArrayInt32");
    assert_eq!(array.baskets[0].at, 2312);
    assert!(matches!(array.reading, root_tree::Reading::Fixed { per_entry: 10, width: 4, .. }));
    match root_tree::branch_basket(&doc, array, 0).expect("ArrayInt32").values {
        root_tree::Values::Ints(v) => {
            assert_eq!(v.len(), 1000);
            assert_eq!(&v[..11], &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
        }
        other => panic!("{other:?}"),
    }

    // Text, and an array counted by another branch. Both are listed and
    // neither is read.
    let text = by_name("Str");
    assert_eq!(text.baskets[0].at, 1848);
    assert!(matches!(&text.reading, root_tree::Reading::Not(why) if why.contains("variable-length")));
    let slice = by_name("SliceInt32");
    assert_eq!(slice.baskets[0].at, 5023);
    assert_eq!(slice.leaves[0].counted_by, "N");
    assert!(matches!(&slice.reading, root_tree::Reading::Not(why) if why.starts_with("N values per entry")));
}

/// A branch with more than one basket, which is the shape everything about
/// `fBasketEntry` exists for: five baskets of seven entries each, and the
/// entry each one starts at.
#[test]
fn a_branch_of_several_baskets_says_where_each_one_starts() {
    let Some(root) = root_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    let (doc, contents) = contents_of(&root, "uproot-sample-6.20.04-uncompressed.root");
    let tree = &contents.trees[0];
    assert_eq!(tree.name, "sample");
    assert_eq!(tree.entries, 30);
    assert_eq!(tree.branch_total, 35);

    let n = tree.branches.iter().find(|b| b.name == "n").expect("branch n");
    assert_eq!(n.basket_total, 5);
    let at: Vec<u64> = n.baskets.iter().map(|b| b.at).collect();
    assert_eq!(at, [6894, 15789, 25545, 34160, 39541]);
    let starts: Vec<i64> = n.baskets.iter().map(|b| b.first_entry).collect();
    assert_eq!(starts, [0, 7, 14, 21, 28]);
    assert_eq!(n.baskets[0].entries, 7);
    match root_tree::branch_basket(&doc, n, 0).expect("n basket 0").values {
        root_tree::Values::Ints(v) => assert_eq!(v, vec![0, 1, 2, 3, 4, 0, 1]),
        other => panic!("{other:?}"),
    }
    match root_tree::branch_basket(&doc, n, 1).expect("n basket 1").values {
        root_tree::Values::Ints(v) => assert_eq!(v, vec![2, 3, 4, 0, 1, 2, 3]),
        other => panic!("{other:?}"),
    }

    // Signedness comes from the leaf, not from the width: these two branches
    // are the same byte for byte and read differently.
    let i1 = tree.branches.iter().find(|b| b.name == "i1").expect("i1");
    match root_tree::branch_basket(&doc, i1, 0).expect("i1").values {
        root_tree::Values::Ints(v) => assert_eq!(&v[..3], &[-15, -14, -13]),
        other => panic!("{other:?}"),
    }
    let u1 = tree.branches.iter().find(|b| b.name == "u1").expect("u1");
    match root_tree::branch_basket(&doc, u1, 0).expect("u1").values {
        root_tree::Values::Ints(v) => assert_eq!(&v[..3], &[0, 1, 2]),
        other => panic!("{other:?}"),
    }
}

/// Trees in directories, at three different depths, each named by the path it
/// goes by in the file.
#[test]
fn trees_in_directories_are_found_and_named_by_their_path() {
    let Some(root) = root_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    let (_, contents) = contents_of(&root, "uproot-nesteddirs.root");
    let names: Vec<&str> = contents.trees.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["one/two/tree", "one/tree", "three/tree"]);
    assert_eq!(contents.trees[0].at, 9903);
    assert_eq!(contents.trees[0].entries, 100);
    assert_eq!(contents.trees[1].entries, 4);
    assert_eq!(contents.trees[2].entries, 100);
    assert_eq!(contents.trees[0].branches[0].baskets[0].at, 1359);
}

/// Every sample in the folder reads without an error, and a file with no tree
/// in it says so by having none rather than by failing.
#[test]
fn every_sample_reads() {
    let Some(root) = root_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    let mut checked = 0;
    for entry in std::fs::read_dir(&root).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "root") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let (doc, contents) = contents_of(&root, &name);
        assert_eq!(contents.trouble, None, "{name}");
        assert!(!contents.classes.is_empty(), "{name}: no class descriptions");
        for tree in &contents.trees {
            assert_eq!(tree.trouble, None, "{name} {}", tree.name);
            assert!(!tree.branches.is_empty(), "{name} {}: no branches", tree.name);
            for branch in &tree.branches {
                // Every basket offset points at a key inside the file, and
                // every basket reads or says why it does not.
                for basket in &branch.baskets {
                    assert!(basket.at > 0 && basket.at < doc.len_bytes(), "{name} {}: basket at {}", branch.name, basket.at);
                }
                if let Some(basket) = branch.baskets.first() {
                    let data = root_tree::branch_basket(&doc, branch, 0)
                        .unwrap_or_else(|e| panic!("{name} {} at {}: {e:?}", branch.name, basket.at));
                    assert_eq!(data.entries, basket.entries, "{name} {}", branch.name);
                }
            }
        }
        eprintln!("{name}: {} classes, {} trees", contents.classes.len(), contents.trees.len());
        checked += 1;
    }
    assert!(checked >= 8, "only {checked} samples read");
}

// ---------------------------------------------------------------------------
// RNTuple, read by the template. Every number and name below was read out of
// the same two files by uproot 5.7.6 (`f['Staff'].field_records`,
// `.column_records`, `.page_link_list`, `.arrays()`), and the template has to
// come to the same.

/// The first node under `at`, breadth first, whose type is called `name`. The
/// path from an anchor to its envelope depends on whether it was compressed and
/// with what, so it is looked for rather than spelled.
fn find_type(d: &Document<MemSource>, ev: &mut Evaluator, at: &[usize], name: &str, depth: u32) -> Option<Vec<usize>> {
    let mut level = vec![at.to_vec()];
    for _ in 0..depth {
        let mut next = Vec::new();
        for p in level {
            let Ok(node) = ev.node(d, &p) else { continue };
            if node.type_name == name {
                return Some(p);
            }
            for i in 0..node.child_count.min(16) as usize {
                next.push([p.as_slice(), &[i]].concat());
            }
        }
        level = next;
    }
    None
}

/// The one RNTuple anchor in a sample, which in both samples is the first key
/// of the top directory, opened out of its block if it was compressed.
fn rntuple_anchor(d: &Document<MemSource>, ev: &mut Evaluator) -> Vec<usize> {
    let record = [DIRECTORY.as_slice(), &[K_FIELDS + 2, 11, 0, K_FIELDS + 1, 0, K_FIELDS, 0, K_FIELDS]].concat();
    find_type(d, ev, &record, "RNTupleAnchor", 10).expect("an anchor")
}

fn rntuple_sample(folder: &Path, name: &str) -> (Document<MemSource>, Evaluator) {
    let path = folder.join(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    (Document::new(MemSource(bytes)), Evaluator::new(root()))
}

/// What a column's type is called, as the template reads it.
fn enum_name(value: Value) -> String {
    match value {
        Value::Enum { name: Some(name), .. } => name,
        other => panic!("not a named value: {other:?}"),
    }
}

#[test]
fn rntuple_headers_list_the_fields_and_columns_uproot_reads() {
    let Some(folder) = root_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    let staff_fields = [
        ("Category", "std::int32_t"),
        ("Flag", "std::uint32_t"),
        ("Age", "std::int32_t"),
        ("Service", "std::int32_t"),
        ("Children", "std::int32_t"),
        ("Grade", "std::int32_t"),
        ("Step", "std::int32_t"),
        ("Hrweek", "std::int32_t"),
        ("Cost", "std::int32_t"),
        ("Division", "std::string"),
        ("Nation", "std::string"),
    ];
    // Column type and the field it belongs to. A compressed ntuple stores its
    // integers split and zigzagged and its string offsets split and as
    // differences; an uncompressed one stores both as they are.
    let mut staff_columns: Vec<(&str, usize)> = (0..9).map(|i| (if i == 1 { "SplitUInt32" } else { "SplitInt32" }, i)).collect();
    staff_columns.extend([("SplitIndex64", 9), ("Char", 9), ("SplitIndex64", 10), ("Char", 10)]);
    let viewer_fields = [("firstName", "std::string"), ("lastName", "std::string")];
    let viewer_columns = [("Index64", 0), ("Char", 0), ("Index64", 1), ("Char", 1)];
    let cases: [(&str, &str, &[(&str, &str)], &[(&str, usize)], bool); 2] = [
        ("ntpl001_staff_rntuple_v1-0-1-0.root", "Staff", &staff_fields, &staff_columns, true),
        ("rntviewer-testfile-uncomp-single-rntuple-v1-0-0-0.root", "Contributors", &viewer_fields, &viewer_columns, false),
    ];
    for (file, ntuple, fields, columns, packed) in cases {
        let (d, mut ev) = rntuple_sample(&folder, file);
        let anchor = rntuple_anchor(&d, &mut ev);
        let header = find_type(&d, &mut ev, &[anchor.as_slice(), &[15]].concat(), "RNTupleEnvelope", 10).expect("a header");
        // Whether the header came out of a block is what the anchor's two
        // lengths say, and the envelope says which space it is in.
        assert_eq!(ev.node(&d, &header).unwrap().space != 0, packed, "{file}");
        let payload = [header.as_slice(), &[2]].concat();
        let at = |more: &[usize]| [payload.as_slice(), more].concat();
        assert_eq!(ev.node(&d, &at(&[2, 1])).unwrap().value, Value::Str(ntuple.into()), "{file}");
        let writer = ev.node(&d, &at(&[4, 1])).unwrap().value;
        assert!(matches!(&writer, Value::Str(s) if s.starts_with("ROOT v6.")), "{file}: {writer:?}");

        assert_eq!(ev.node(&d, &at(&[5, 2])).unwrap().child_count as usize, fields.len(), "{file}");
        for (i, (name, type_name)) in fields.iter().enumerate() {
            let field = at(&[5, 2, i]);
            assert_eq!(ev.node(&d, &field).unwrap().name, format!("[{i}] {name}"), "{file}");
            assert_eq!(ev.node(&d, &[field.as_slice(), &[7, 1]].concat()).unwrap().value, Value::Str((*type_name).into()));
            // Every field of both files is top-level, so its own parent.
            assert_eq!(ev.node(&d, &[field.as_slice(), &[3]].concat()).unwrap().value.as_int(), Some(i as i128));
        }
        assert_eq!(ev.node(&d, &at(&[6, 2])).unwrap().child_count as usize, columns.len(), "{file}");
        for (i, (type_name, field)) in columns.iter().enumerate() {
            let column = at(&[6, 2, i]);
            assert_eq!(enum_name(ev.node(&d, &[column.as_slice(), &[1]].concat()).unwrap().value), *type_name, "{file} column {i}");
            assert_eq!(ev.node(&d, &[column.as_slice(), &[3]].concat()).unwrap().value.as_int(), Some(*field as i128));
            assert_eq!(ev.node(&d, &column).unwrap().name, format!("[{i}] {}", fields[*field].0), "{file}");
        }

        // The footer names the header it goes with by the same checksum the
        // header ends with.
        let footer = find_type(&d, &mut ev, &[anchor.as_slice(), &[16]].concat(), "RNTupleEnvelope", 10).expect("a footer");
        let header_sum = ev.node(&d, &[header.as_slice(), &[3]].concat()).unwrap().value;
        assert_eq!(ev.node(&d, &[footer.as_slice(), &[2, 2]].concat()).unwrap().value, header_sum, "{file}");
        eprintln!("--- {file}: {} fields and {} columns, as uproot reads them", fields.len(), columns.len());
    }
}

/// One cluster's pages, one per column in both samples: element count, then
/// the locator's size and offset. Every page in both files has a checksum after
/// it. uproot's `page_link_list`.
const STAFF_PAGES: [(i128, u64, u64); 13] = [
    (3354, 3643, 642),
    (3354, 1196, 4293),
    (3354, 2226, 5497),
    (3354, 1392, 7731),
    (3354, 1051, 9131),
    (3354, 1504, 10190),
    (3354, 1655, 11702),
    (3354, 273, 13365),
    (3354, 6147, 13646),
    (3354, 591, 19801),
    (7811, 2062, 20400),
    (3354, 32, 22470),
    (6708, 1747, 22510),
];
const VIEWER_PAGES: [(i128, u64, u64); 4] = [(22, 176, 620), (178, 178, 804), (22, 176, 990), (193, 193, 1174)];

/// The page list of the first cluster group, found through the footer the
/// anchor points at, and the payload of the envelope it turned out to be.
fn page_list_payload(d: &Document<MemSource>, ev: &mut Evaluator, anchor: &[usize]) -> (Vec<usize>, Vec<usize>) {
    let footer = find_type(d, ev, &[anchor, &[16]].concat(), "RNTupleEnvelope", 10).expect("a footer");
    let group = [footer.as_slice(), &[2, 4, 2, 0]].concat();
    let list = find_type(d, ev, &[group.as_slice(), &[6]].concat(), "RNTupleEnvelope", 10).expect("a page list");
    (group, [list.as_slice(), &[2]].concat())
}

#[test]
fn rntuple_pages_are_placed_where_uproot_finds_them() {
    let Some(folder) = root_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    // The file, its entries, the page list's length unpacked and its
    // locator, the pages, and the compression settings every column has.
    let cases: [(&str, u64, (u64, u64, u64), &[(i128, u64, u64)], u64); 2] = [
        ("ntpl001_staff_rntuple_v1-0-1-0.root", 3354, (604, 194, 24307), &STAFF_PAGES, 505),
        ("rntviewer-testfile-uncomp-single-rntuple-v1-0-0-0.root", 22, (244, 244, 1409), &VIEWER_PAGES, 0),
    ];
    for (file, entries, (length, size, offset), pages, settings) in cases {
        let (d, mut ev) = rntuple_sample(&folder, file);
        let anchor = rntuple_anchor(&d, &mut ev);
        let (group, payload) = page_list_payload(&d, &mut ev, &anchor);
        let at = |base: &[usize], more: &[usize]| [base, more].concat();
        assert_eq!(ev.node(&d, &at(&group, &[2])).unwrap().value, Value::UInt(entries as u128), "{file}");
        assert_eq!(ev.node(&d, &at(&group, &[4])).unwrap().value, Value::UInt(length as u128), "{file}");
        assert_eq!(ev.node(&d, &at(&group, &[5, 0])).unwrap().value.as_int(), Some(size as i128), "{file}");
        let placed = ev.node(&d, &at(&group, &[6, 0])).unwrap();
        assert_eq!((placed.offset_bits / 8, placed.size_bits / 8, placed.space), (offset, size, 0), "{file}");

        assert_eq!(ev.node(&d, &at(&payload, &[1, 2, 0, 2])).unwrap().value, Value::UInt(entries as u128), "{file}");
        let columns = at(&payload, &[2, 2, 0, 2]);
        assert_eq!(ev.node(&d, &columns).unwrap().child_count as usize, pages.len(), "{file}");
        let mut covered = 0;
        for (j, (elements, size, offset)) in pages.iter().enumerate() {
            let column = at(&columns, &[j]);
            assert_eq!(ev.node(&d, &at(&column, &[5])).unwrap().child_count, 1, "{file} column {j}");
            assert_eq!(ev.node(&d, &at(&column, &[7])).unwrap().value, Value::UInt(settings as u128), "{file} column {j}");
            let entry = at(&column, &[5, 0]);
            assert_eq!(ev.node(&d, &at(&entry, &[1])).unwrap().value.as_int(), Some(*elements), "{file} column {j}");
            // Negative, because a checksum follows.
            assert_eq!(ev.node(&d, &at(&entry, &[0])).unwrap().value.as_int(), Some(-*elements), "{file} column {j}");
            let data = ev.node(&d, &at(&entry, &[3, 0, 0])).unwrap();
            assert_eq!((data.offset_bits / 8, data.size_bits / 8, data.space), (*offset, *size, 0), "{file} column {j}");
            let sum = ev.node(&d, &at(&entry, &[3, 0, 1])).unwrap();
            assert_eq!((sum.offset_bits / 8, sum.size_bits / 8), (offset + size, 8), "{file} column {j}");
            covered += size + 8;
        }
        eprintln!("--- {file}: {} pages placed, {covered} of {} bytes", pages.len(), d.len_bytes());
    }
}

/// How many bytes of the file some field of the tree names: the union of every
/// field with no children in the file's own space. `skip` names fields whose
/// contents are not walked, which is how the same file is measured as the
/// template read it before page lists were followed.
///
/// A compressed run is one of those. What it holds is in a space of its own,
/// and a run whose codec leaves no trace of blocks to lay over it, as ROOT's
/// lz4 does not, has no child in the file at all: the run is the field that
/// names those bytes.
fn named_bytes(d: &Document<MemSource>, ev: &mut Evaluator, skip: &dyn Fn(&str, &[usize]) -> bool) -> u64 {
    let mut spans = Vec::new();
    let mut stack = vec![Vec::new()];
    while let Some(p) = stack.pop() {
        let Ok(node) = ev.node(d, &p) else { continue };
        if skip(&node.name, &p) {
            continue;
        }
        if node.decoded && node.space == 0 {
            spans.push((node.offset_bits, node.offset_bits + node.size_bits));
        }
        // Walked into whatever space it is in, since a page list read out of
        // a compressed footer places its pages back in the file; counted only
        // where it is the file's own bytes.
        if node.child_count == 0 {
            if node.size_bits > 0 && node.space == 0 {
                spans.push((node.offset_bits, node.offset_bits + node.size_bits));
            }
            continue;
        }
        for i in 0..node.child_count as usize {
            stack.push([p.as_slice(), &[i]].concat());
        }
    }
    spans.sort();
    let (mut total, mut reach) = (0, 0);
    for (start, end) in spans {
        let start = start.max(reach);
        if end > start {
            total += end - start;
            reach = end;
        }
    }
    total / 8
}

#[test]
fn rntuple_pages_read_as_the_values_uproot_reads() {
    let Some(folder) = root_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    // Stored as they stand, header and all, so every column's type can be read
    // from its page list and every page reads as its values: a string's offsets
    // are where each one ends, and its characters are all of them one after
    // another.
    let first = [
        "Jakob", "Philippe", "Axel", "Danilo", "Simon", "Bertrand", "Max", "Javier", "Enrico", "Sergey", "Giovanna",
        "Jerry", "Florine Willemijn", "Bernhard Manfred", "Vincenzo Eduardo", "Jolly", "Alaettin Serhan", "Jonas",
        "Maciej", "Giacomo", "Grigori", "Special thanks",
    ];
    let last = [
        "Blomer", "Canal", "Naumann", "Piparo", "Leisibach", "Bellenot", "Orok", "Lopez-Gomez", "Guiraud", "Linev",
        "Lazzari Miotto", "Ling", "de Geus", "Gruber", "Padulano", "Chen", "Mete", "Hahnfeld", "Szymanski", "Parolini",
        "Rybkine", "to all framework developers in the experiments",
    ];
    let file = "rntviewer-testfile-uncomp-single-rntuple-v1-0-0-0.root";
    let (d, mut ev) = rntuple_sample(&folder, file);
    let anchor = rntuple_anchor(&d, &mut ev);
    let (_, payload) = page_list_payload(&d, &mut ev, &anchor);
    let columns = [payload.as_slice(), &[2, 2, 0, 2]].concat();
    let at = |base: &[usize], more: &[usize]| [base, more].concat();
    for (k, (field, names)) in [("firstName", first), ("lastName", last)].into_iter().enumerate() {
        let (index, chars) = (at(&columns, &[2 * k]), at(&columns, &[2 * k + 1]));
        assert_eq!(ev.node(&d, &index).unwrap().name, format!("[{}] {field}", 2 * k));
        assert_eq!(enum_name(ev.node(&d, &at(&index, &[3])).unwrap().value), "Index64");
        assert_eq!(enum_name(ev.node(&d, &at(&chars, &[3])).unwrap().value), "Char");
        let offsets = at(&index, &[5, 0, 3, 0, 0]);
        let mut end = 0u128;
        for (i, name) in names.iter().enumerate() {
            end += name.len() as u128;
            assert_eq!(ev.node(&d, &at(&offsets, &[i])).unwrap().value, Value::UInt(end), "{field} [{i}]");
        }
        assert_eq!(ev.node(&d, &offsets).unwrap().child_count, 22);
        let text = ev.node(&d, &at(&chars, &[5, 0, 3, 0, 0])).unwrap().value;
        assert_eq!(text, Value::Str(names.concat()), "{field}");
    }

    // Compressed, header and all, so no page can learn its column's type (see
    // `typed` in the template). Each page still opens, because it opens as a
    // zstd block, and what comes out is as long as its elements at the width
    // the header gives them: 7,811 characters of `Division` is what uproot
    // reads as 3,354 strings.
    let file = "ntpl001_staff_rntuple_v1-0-1-0.root";
    let (d, mut ev) = rntuple_sample(&folder, file);
    let anchor = rntuple_anchor(&d, &mut ev);
    let (_, payload) = page_list_payload(&d, &mut ev, &anchor);
    let columns = [payload.as_slice(), &[2, 2, 0, 2]].concat();
    let widths = [4, 4, 4, 4, 4, 4, 4, 4, 4, 8, 1, 8, 1];
    for (j, ((elements, size, _), width)) in STAFF_PAGES.iter().zip(widths).enumerate() {
        let column = at(&columns, &[j]);
        assert!(ev.node(&d, &at(&column, &[3])).unwrap().absent, "column {j} has a type it cannot have read");
        let data = at(&column, &[5, 0, 3, 0, 0]);
        assert_eq!(ev.node(&d, &data).unwrap().size_bits / 8, *size);
        let block = at(&data, &[0]);
        assert_eq!(enum_name(ev.node(&d, &at(&block, &[0])).unwrap().value), "zstd", "column {j}");
        let opened = ev.node(&d, &at(&block, &[4, 1, 0, 0])).unwrap();
        assert_ne!(opened.space, 0);
        assert_eq!(opened.size_bits / 8, (*elements as u64) * width, "column {j}");
    }

    // What the tree names of each file, with the page lists followed and as it
    // was before they were: the anchor and its envelopes and nothing they point
    // at. `spans` cannot say this for a ROOT file, because it indexes a placed
    // field only once its walk has passed the field that places it, and every
    // key list is written after the records it lists.
    for file in ["ntpl001_staff_rntuple_v1-0-1-0.root", "rntviewer-testfile-uncomp-single-rntuple-v1-0-0-0.root"] {
        let (d, mut ev) = rntuple_sample(&folder, file);
        let before = named_bytes(&d, &mut ev, &|name, _| name == "page_list");
        let (d, mut ev) = rntuple_sample(&folder, file);
        let after = named_bytes(&d, &mut ev, &|_, _| false);
        eprintln!("--- {file}: {before} bytes named without the page lists, {after} with, of {}", d.len_bytes());
        assert!(after > before, "{file}");
    }
}

// ---------------------------------------------------------------------------
// Streamed objects, read by the template with the class descriptions the file
// carries. The side reader above is the oracle: it reads the same bytes with
// hand-written code, and the two have to agree wherever both answer.

/// Where the header places the `StreamerInfo` record.
const STREAMER: [usize; 2] = [16, 0];

/// Down from `from` a name at a time, through a field that points elsewhere
/// and into what a stream holds, the way a path in the template goes. A list's
/// element is named by its index.
fn go(d: &Document<MemSource>, ev: &mut Evaluator, from: &[usize], names: &[&str]) -> Option<Vec<usize>> {
    let mut p = from.to_vec();
    for name in names {
        inside(d, ev, &mut p);
        match name.parse::<usize>() {
            Ok(i) => p.push(i),
            Err(_) => p = ev.child_named(d, &p, name).ok()??,
        }
    }
    inside(d, ev, &mut p);
    Some(p)
}

/// Step from a field of no bits to the one thing it holds, where it is one.
fn inside(d: &Document<MemSource>, ev: &mut Evaluator, p: &mut Vec<usize>) {
    while let Ok(node) = ev.node(d, p) {
        let pointing = node.type_name.starts_with("at \u{2192}") || node.type_name.starts_with("joined");
        if !pointing || node.child_count != 1 {
            break;
        }
        p.push(0);
    }
}

fn text(d: &Document<MemSource>, ev: &mut Evaluator, from: &[usize], names: &[&str]) -> String {
    let Some(p) = go(d, ev, from, names) else { return String::new() };
    match ev.node(d, &p).map(|n| n.value) {
        Ok(Value::Str(s)) => s,
        _ => String::new(),
    }
}

fn int(d: &Document<MemSource>, ev: &mut Evaluator, from: &[usize], names: &[&str]) -> Option<i128> {
    let p = go(d, ev, from, names)?;
    ev.node(d, &p).ok()?.value.as_int()
}

/// The class descriptions as the template reads them, in the shape the side
/// reader lists them in.
fn template_classes(d: &Document<MemSource>, ev: &mut Evaluator) -> Vec<root_tree::Class> {
    let list = go(d, ev, &STREAMER, &["object", "members", "elements"]).expect("the StreamerInfo list");
    let n = ev.node(d, &list).unwrap().child_count as usize;
    let mut out = Vec::new();
    for i in 0..n {
        let entry = [list.as_slice(), &[i]].concat();
        if text(d, ev, &entry, &["class_name"]) != "TStreamerInfo" {
            continue;
        }
        let info = go(d, ev, &entry, &["object", "members"]).expect("a description");
        let name = text(d, ev, &info, &["TNamed", "members", "fName", "text"]);
        let version = int(d, ev, &info, &["fClassVersion"]).unwrap_or(0) as i32;
        let checksum = int(d, ev, &info, &["fCheckSum"]).unwrap_or(0) as u32;
        let mut members = Vec::new();
        if let Some(elements) = go(d, ev, &info, &["fElements", "object", "members", "elements"]) {
            let m = ev.node(d, &elements).unwrap().child_count as usize;
            for j in 0..m {
                let item = [elements.as_slice(), &[j]].concat();
                let kind = text(d, ev, &item, &["class_name"]);
                let own = go(d, ev, &item, &["object", "members"]).expect("an element");
                // What every element class has, which is in its base.
                let base: &[&str] = match kind.as_str() {
                    "TStreamerSTLstring" => &["TStreamerSTL", "members", "TStreamerElement", "members"],
                    _ => &["TStreamerElement", "members"],
                };
                fn joined<'a>(base: &[&'a str], more: &[&'a str]) -> Vec<&'a str> {
                    base.iter().chain(more).copied().collect()
                }
                let at = |more: &[&'static str]| joined(base, more);
                let type_name = text(d, ev, &own, &at(&["fTypeName", "text"]));
                let is_base = kind == "TStreamerBase" || type_name == "BASE";
                let dims = match is_base {
                    true => Vec::new(),
                    false => {
                        let n = int(d, ev, &own, &at(&["fArrayDim"])).unwrap_or(0).clamp(0, 5) as usize;
                        (0..n)
                            .map(|k| {
                                let k = k.to_string();
                                int(d, ev, &own, &joined(base, &["fMaxIndex", &k])).unwrap_or(0) as i32
                            })
                            .collect()
                    }
                };
                members.push(root_tree::Member {
                    name: text(d, ev, &own, &at(&["TNamed", "members", "fName", "text"])),
                    type_name,
                    code: int(d, ev, &own, &at(&["fType"])).unwrap_or(-1) as i32,
                    size: int(d, ev, &own, &at(&["fSize"])).unwrap_or(0) as i32,
                    dims,
                    base: is_base,
                    comment: text(d, ev, &own, &at(&["TNamed", "members", "fTitle", "text"])),
                });
            }
        }
        out.push(root_tree::Class { name, version, checksum, members });
    }
    out
}

/// Every sample's class descriptions, read by the template as the streamed
/// objects they are, come to what the side reader lists. That is the bootstrap
/// classes (`TList`, `TStreamerInfo` and the element classes, which no file
/// describes) checked against `root_streamer`'s hand-written reading of the
/// same bytes, and it is what every object the file describes is built from.
#[test]
fn the_streamer_info_record_reads_as_the_classes_the_side_reader_lists() {
    let Some(folder) = root_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    let mut checked = 0;
    for entry in std::fs::read_dir(&folder).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "root") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let (d, contents) = contents_of(&folder, &name);
        let mut ev = Evaluator::new(root());
        let classes = template_classes(&d, &mut ev);
        assert!(!classes.is_empty(), "{name}: no classes");
        assert_eq!(classes.len(), contents.classes.len(), "{name}");
        for (got, want) in classes.iter().zip(&contents.classes) {
            assert_eq!(got, want, "{name}: {}", want.name);
        }
        eprintln!("--- {name}: {} classes read the same both ways", classes.len());
        checked += 1;
    }
    assert!(checked >= 8, "only {checked} samples read");
}

/// The first `TStreamerInfo` in the list spells its class out; the second
/// writes where that spelling is, counted from the start of the key, and the
/// template reads that as a field placed back at the first one's name.
#[test]
fn a_second_object_of_a_class_reads_its_name_from_where_the_first_spelled_it() {
    let Some(folder) = root_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    let (d, mut ev) = rntuple_sample(&folder, "uproot-Zmumu-lz4.root");
    let list = go(&d, &mut ev, &STREAMER, &["object", "members", "elements"]).expect("the list");
    let first = ev.child_named(&d, &[list.as_slice(), &[0]].concat(), "class_name").unwrap().expect("a name");
    let spelled = ev.node(&d, &first).unwrap();
    assert_eq!(spelled.value, Value::Str("TStreamerInfo".into()));
    assert_eq!(spelled.type_name, "cstr");

    let second = ev.child_named(&d, &[list.as_slice(), &[1]].concat(), "class_name").unwrap().expect("a name");
    let pointer = ev.node(&d, &second).unwrap();
    assert!(pointer.type_name.starts_with("at \u{2192}"), "{}", pointer.type_name);
    assert_eq!(pointer.size_bits, 0);
    let named = ev.node(&d, &[second.as_slice(), &[0]].concat()).unwrap();
    assert_eq!(named.value, Value::Str("TStreamerInfo".into()));
    // The same bytes the first entry spelled, in the same space.
    assert_eq!((named.offset_bits, named.space), (spelled.offset_bits, spelled.space));
    // And the object is read as that class for it.
    let object = go(&d, &mut ev, &list, &["1", "object"]).expect("an object");
    assert_eq!(ev.node(&d, &object).unwrap().type_name, "TStreamerInfo");
}

/// Every basket the template placed for the tree whose key is at `key`: where
/// it is and how long, in bytes, and its path.
fn template_baskets(d: &Document<MemSource>, ev: &mut Evaluator, key: &[usize]) -> Vec<(u64, u64, Vec<usize>)> {
    let record = [key, &[K_FIELDS, 0]].concat();
    let list = ev.child_named(d, &record, "baskets").unwrap().expect("a tree record has baskets");
    let n = ev.node(d, &list).unwrap().child_count as usize;
    (0..n)
        .map(|i| {
            let p = [list.as_slice(), &[i]].concat();
            let node = ev.node(d, &p).unwrap();
            (node.offset_bits / 8, node.size_bits / 8, p)
        })
        .collect()
}

/// Every basket the side reader finds through a tree's branches is a basket
/// the template placed, at the same offset with the same length, in the same
/// order, and each is a `TBasket` key holding as many entries as its branch
/// says it does.
#[test]
fn every_basket_the_side_reader_lists_is_placed_by_the_template() {
    let Some(folder) = root_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    let mut placed = 0;
    for entry in std::fs::read_dir(&folder).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "root") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let (d, contents) = contents_of(&folder, &name);
        let mut ev = Evaluator::new(root());
        for tree in &contents.trees {
            let want: Vec<(u64, u64, i64)> = tree
                .branches
                .iter()
                .flat_map(|b| b.baskets.iter().map(|k| (k.at, k.bytes as u64, k.entries)))
                .collect();
            let got = template_baskets(&d, &mut ev, &tree.path);
            let at: Vec<(u64, u64)> = got.iter().map(|(o, s, _)| (*o, *s)).collect();
            assert_eq!(at, want.iter().map(|(o, s, _)| (*o, *s)).collect::<Vec<_>>(), "{name} {}", tree.name);
            for ((offset, _, p), (_, _, entries)) in got.iter().zip(&want) {
                assert_eq!(text(&d, &mut ev, p, &["fClassName", "text"]), "TBasket", "{name} @{offset}");
                assert_eq!(int(&d, &mut ev, p, &["fNevBuf"]), Some(*entries as i128), "{name} @{offset}");
                assert_eq!(int(&d, &mut ev, p, &["fSeekKey"]), Some(*offset as i128), "{name} @{offset}");
            }
            placed += got.len();
            eprintln!("--- {name} {}: {} baskets placed where the side reader lists them", tree.name, got.len());
        }
    }
    assert!(placed > 0, "no sample had a basket to place");
}

/// The same tree written with lz4, lzma and zstd places twenty baskets each,
/// at each file's own offsets, holding the same entries and unpacking to the
/// same length, with the block inside each named by the codec that wrote it.
#[test]
fn the_same_tree_places_the_same_baskets_through_lz4_lzma_and_zstd() {
    let Some(folder) = root_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    let mut shapes = Vec::new();
    for (name, first, codec) in [
        ("uproot-Zmumu-lz4.root", 224u64, "lz4"),
        ("uproot-Zmumu-lzma.root", 226, "xz"),
        ("uproot-Zmumu-zstd.root", 254, "zstd"),
    ] {
        let (d, contents) = contents_of(&folder, name);
        let mut ev = Evaluator::new(root());
        let baskets = template_baskets(&d, &mut ev, &contents.trees[0].path);
        assert_eq!(baskets.len(), 20, "{name}");
        assert_eq!(baskets[0].0, first, "{name}");
        let mut shape = Vec::new();
        let mut packed = 0;
        for (_, _, p) in &baskets {
            // A basket that would not have got smaller is written as it
            // stands, and has no block to name a codec.
            let stored = int(&d, &mut ev, p, &["fNbytes"]).unwrap() - int(&d, &mut ev, p, &["fKeylen"]).unwrap();
            if stored < int(&d, &mut ev, p, &["fObjlen"]).unwrap() {
                let block = go(&d, &mut ev, p, &["body", "0", "algorithm"]).expect("a block");
                let algorithm = ev.node(&d, &block).unwrap().value;
                assert!(matches!(&algorithm, Value::Enum { name: Some(n), .. } if n == codec), "{name}: {algorithm:?}");
                packed += 1;
            }
            shape.push((
                text(&d, &mut ev, p, &["fName", "text"]),
                int(&d, &mut ev, p, &["fNevBuf"]),
                int(&d, &mut ev, p, &["fObjlen"]),
            ));
        }
        assert!(packed > 0, "{name}: no basket was compressed");
        eprintln!("--- {name}: {packed} of {} baskets compressed with {codec}", baskets.len());
        shapes.push(shape);
    }
    assert_eq!(shapes[0], shapes[1]);
    assert_eq!(shapes[0], shapes[2]);
}

/// A basket says where it came from: the branch's `BasketRef` that placed it,
/// named by the walk through the tree's object, and that record's `seek` is an
/// element of `fBasketSeek`, which is a counted array because the element of
/// `TBranch`'s description in `StreamerInfo` says `Long64_t*`.
#[test]
fn a_baskets_type_names_the_streamer_element_it_came_from() {
    let Some(folder) = root_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    let (d, contents) = contents_of(&folder, "uproot-Zmumu-lz4.root");
    let mut ev = Evaluator::new(root());
    let baskets = template_baskets(&d, &mut ev, &contents.trees[0].path);
    let (_, _, e1) = &baskets[3];
    let origins = ev.origins(&d, e1).unwrap();
    let placed = origins.iter().find(|o| o.role == qubero_core::eval::Role::Position).expect("placed by a record");
    assert_eq!(placed.label, "object.members.fBranches.members.elements[3].object.members.baskets[0]");
    let record = placed.path.clone();
    assert_eq!(int(&d, &mut ev, &record, &["seek"]), Some(13371));

    // The seek is read from the branch's own array.
    let seek = ev.child_named(&d, &record, "seek").unwrap().expect("seek");
    let from = ev.origins(&d, &seek).unwrap();
    assert!(from.iter().any(|o| o.label == "fBasketSeek.values[0]"), "{from:?}");

    // And that array is what it is because of one element of one description.
    let members = record[..record.len() - 2].to_vec();
    let array = ev.child_named(&d, &members, "fBasketSeek").unwrap().expect("fBasketSeek");
    assert_eq!(ev.node(&d, &array).unwrap().type_name, "Long64_t*");
    let typed = ev.origins(&d, &array).unwrap();
    let element = typed
        .iter()
        .find(|o| o.role == qubero_core::eval::Role::Type && o.path.starts_with(&STREAMER))
        .unwrap_or_else(|| panic!("no description named: {typed:?}"));
    assert!(element.label.starts_with("streamer_info.object.members.elements["), "{}", element.label);
    let described = go(&d, &mut ev, &element.path, &["object", "members"]).expect("an element");
    assert_eq!(text(&d, &mut ev, &described, &["TStreamerElement", "members", "TNamed", "members", "fName", "text"]), "fBasketSeek");
    assert_eq!(text(&d, &mut ev, &described, &["TStreamerElement", "members", "fTypeName", "text"]), "Long64_t*");
    // The branch as a whole was built from `TBranch`'s description.
    let branch = ev.relations(&d, &members).unwrap();
    assert!(branch.iter().any(|r| r.result == "TBranch v12"), "{branch:?}");
}

/// A tree walk of the file names most of its bytes once the baskets are
/// placed, where before them it named the header, the directory and the
/// records the directory lists: a few per cent. Every sample says both, and
/// the deepest chain of expressions the walk asked is well inside the guard.
#[test]
fn zmumu_names_nine_tenths_of_its_bytes() {
    let Some(folder) = root_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    let mut names: Vec<String> = std::fs::read_dir(&folder)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "root"))
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let mut zmumu = None;
    for name in names {
        let (d, mut ev) = rntuple_sample(&folder, &name);
        let before = named_bytes(&d, &mut ev, &|n, _| n == "baskets");
        let (d, mut ev) = rntuple_sample(&folder, &name);
        let after = named_bytes(&d, &mut ev, &|_, _| false);
        let deepest = ev.deepest_question();
        let len = d.len_bytes();
        eprintln!(
            "--- {name}: {before} bytes named without the baskets ({:.1}%), {after} with ({:.1}%), of {len}; deepest chain of expressions {deepest}",
            100.0 * before as f64 / len as f64,
            100.0 * after as f64 / len as f64
        );
        assert!(deepest < 88, "{name}: {deepest}");
        if name == "uproot-Zmumu-lz4.root" {
            zmumu = Some((after, len));
        }
    }
    let (after, len) = zmumu.expect("uproot-Zmumu-lz4.root is in the collection");
    assert!(after * 10 >= len * 9, "uproot-Zmumu-lz4.root: {after} of {len}");
}

/// How many values of each basket are compared one by one. The rest are
/// counted: a basket of the uncompressed sample holds a handful and one of
/// Zmumu two thousand, and every one of those as a node is a slow test that
/// says nothing the first few hundred did not.
const VALUES_COMPARED: usize = 300;

/// Every basket of every sample reads as the side reader reads it: the values
/// of a branch it reads, number for number, by the leaf's type; bytes for a
/// branch it does not; and the table of entry offsets, where there is one, as
/// the same offsets less the key's length.
#[test]
fn a_baskets_values_match_the_side_reader() {
    let Some(folder) = root_samples() else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    let (mut typed, mut left, mut tables) = (0, 0, 0);
    for entry in std::fs::read_dir(&folder).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "root") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let (d, contents) = contents_of(&folder, &name);
        let mut ev = Evaluator::new(root());
        for tree in &contents.trees {
            let placed = template_baskets(&d, &mut ev, &tree.path);
            let listed = tree.branches.iter().flat_map(|b| b.baskets.iter().map(move |k| (b, k)));
            for ((branch, basket), (_, _, p)) in listed.zip(&placed) {
                let say = format!("{name} {} @{}", branch.name, basket.at);
                let side = root_tree::read_basket(&d, basket.at, &branch.reading).unwrap_or_else(|e| panic!("{say}: {e:?}"));
                let values = go(&d, &mut ev, p, &["entries", "values"]).unwrap_or_else(|| panic!("{say}: no values"));
                let node = ev.node(&d, &values).unwrap_or_else(|e| panic!("{say}: {e:?}"));
                let at = |ev: &mut Evaluator, i: usize| ev.node(&d, &[values.as_slice(), &[i]].concat()).unwrap().value;
                match &side.values {
                    root_tree::Values::Ints(want) => {
                        assert_eq!(node.child_count as usize, want.len(), "{say}: {}", node.type_name);
                        for (i, w) in want.iter().enumerate().take(VALUES_COMPARED) {
                            let got = match at(&mut ev, i) {
                                Value::Int(v) => v as i64,
                                Value::UInt(v) => v as u64 as i64,
                                other => panic!("{say} [{i}]: {other:?}"),
                            };
                            assert_eq!(got, *w, "{say} [{i}]");
                        }
                        typed += 1;
                    }
                    root_tree::Values::Floats(want) => {
                        assert_eq!(node.child_count as usize, want.len(), "{say}: {}", node.type_name);
                        // A four-byte float is shown as the shortest decimal
                        // that reads back as those four bytes, and the side
                        // reader widens the same bits to a double, so the two
                        // are compared as the float the file holds.
                        let narrow = node.type_name.starts_with("f32");
                        for (i, w) in want.iter().enumerate().take(VALUES_COMPARED) {
                            let Value::Float(got) = at(&mut ev, i) else { panic!("{say} [{i}]: not a float") };
                            match narrow {
                                true => assert_eq!((got as f32).to_bits(), (*w as f32).to_bits(), "{say} [{i}]"),
                                false => assert_eq!(got.to_bits(), w.to_bits(), "{say} [{i}]"),
                            }
                        }
                        typed += 1;
                    }
                    root_tree::Values::None(why) => {
                        assert_eq!(node.type_name, "bytes[]", "{say}: the side reader reads nothing ({why})");
                        assert_eq!(node.size_bits / 8, side.border as u64, "{say}");
                        left += 1;
                    }
                }
                // The table after the values: its length, one offset per
                // entry counted from the key, and one the side reader puts the
                // border in place of.
                if !side.offsets.is_empty() {
                    let table = go(&d, &mut ev, p, &["entries", "offsets"]).unwrap_or_else(|| panic!("{say}: no offsets"));
                    let n = ev.node(&d, &table).unwrap().child_count as usize;
                    assert_eq!(n, side.offsets.len() + 1, "{say}");
                    let key_len = int(&d, &mut ev, p, &["fKeylen"]).unwrap();
                    for (i, w) in side.offsets.iter().enumerate().take(side.offsets.len() - 1).take(VALUES_COMPARED) {
                        let raw = ev.node(&d, &[table.as_slice(), &[i + 1]].concat()).unwrap().value.as_int().unwrap();
                        assert_eq!(raw - key_len, *w as i128, "{say} offset [{i}]");
                    }
                    tables += 1;
                }
            }
        }
    }
    eprintln!("--- {typed} baskets read as their leaf's values, {left} left as bytes, {tables} with entry offsets");
    assert!(typed > 0 && left > 0 && tables > 0);
}
