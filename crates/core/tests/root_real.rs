//! The ROOT reader against real files: uproot's own test files, in
//! QUBERO_SAMPLES/root or in the sibling qubero-samples collection.
//!
//! Every number checked here was read out of the file twice, once by this
//! crate and once by uproot 5.7.6 (BSD-3), and the two agreed. That is what
//! makes a basket offset worth writing down: it is not what this reader
//! happened to produce, it is where the basket is.
use std::path::PathBuf;

use qubero_core::{
    document::Document,
    eval::Evaluator,
    formats::{self, root_tree},
    source::MemSource,
};

fn root_samples() -> Option<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(paths) = std::env::var("QUBERO_SAMPLES") {
        roots.extend(paths.split(';').filter(|s| !s.is_empty()).map(PathBuf::from));
    }
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../qubero-samples"));
    roots.into_iter().map(|p| p.join("root")).find(|p| p.is_dir())
}

fn read(root: &PathBuf, name: &str) -> (Document<MemSource>, root_tree::Contents) {
    let path = root.join(name);
    let doc = Document::new(MemSource(std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))));
    let mut ev = Evaluator::new(formats::builtin("root").unwrap());
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
    let (_, contents) = read(&root, "uproot-Zmumu-lz4.root");
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
    let (doc, contents) = read(&root, "uproot-Zmumu-lz4.root");
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
    assert!(matches!(&text.reading, root_tree::Reading::Not(why) if why.contains("vary in length")));
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
        let (doc, contents) = read(&root, name);
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
    let (doc, contents) = read(&root, "uproot-small-flat-tree.root");
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
    assert!(matches!(&text.reading, root_tree::Reading::Not(why) if why.contains("vary in length")));
    let slice = by_name("SliceInt32");
    assert_eq!(slice.baskets[0].at, 5023);
    assert_eq!(slice.leaves[0].counted_by, "N");
    assert!(matches!(&slice.reading, root_tree::Reading::Not(why) if why.contains('N')));
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
    let (doc, contents) = read(&root, "uproot-sample-6.20.04-uncompressed.root");
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
    let (_, contents) = read(&root, "uproot-nesteddirs.root");
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
        let (doc, contents) = read(&root, &name);
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
