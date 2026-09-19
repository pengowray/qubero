//! The `joblib.dump` files older releases wrote, read as the same arrays the
//! newest ones give.
//!
//! The wrapper joblib puts in front of an array gained a
//! `numpy_array_alignment_bytes` attribute in 1.2, and with it a byte saying
//! how much padding follows and the padding itself. joblib 1.1 and older wrote
//! neither, so the numbers begin at the BUILD that finishes the wrapper. The
//! form has read that variant since it was written and no file tested it until
//! the container matrix; these are those files
//! (`tools/make_torch_joblib_matrix.py` in the collection).
//!
//! Like the rest of the collection this skips when the folder is not beside
//! the repository. Point `QUBERO_SAMPLES` at it to run it elsewhere.

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

/// Every file joblib 1.1 and older wrote is recognised and matches the form,
/// at the protocol Python 3.6 and 3.7 wrote by default.
#[test]
fn an_unaligned_wrapper_is_the_form_that_anticipated_it() {
    let Some(dir) = qubero_samples::dir("joblib") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    for name in ["v0.11-array-float64.joblib", "v0.11-array-fortran-order.joblib", "v0.11-array-big-endian.joblib", "v0.11-dict-of-arrays.joblib", "v1.1-dict-of-arrays.joblib"] {
        let bytes = std::fs::read(dir.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let found = formats::pickle::familiar::recognise(&bytes).unwrap_or_else(|| panic!("{name}: no form"));
        assert_eq!(found.form, "joblib-arrays-p2-p3-v1", "{name}");
        assert_eq!(found.proto, 3, "{name}");
        let window = &bytes[..bytes.len().min(0x9000)];
        assert_eq!(formats::sniff(window, bytes.len() as u64), Some("joblib"), "{name}");
    }
}

/// The numbers of an un-aligned array are the numbers that were dumped into
/// it, cell for cell, the same as the aligned file of the same array.
///
/// `arange(24, float64).reshape(4, 6)` in C order and again in Fortran order,
/// whose run goes down its first axis, and `arange(6, '>i4')`, whose numbers
/// are the other way round.
#[test]
fn an_unaligned_arrays_numbers_are_the_ones_it_was_dumped_with() {
    let Some(dir) = qubero_samples::dir("joblib") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let rows: Vec<String> = (0..24u64).map(|n| n.to_string()).collect();
    let down: Vec<String> = (0..6u64).flat_map(|c| (0..4u64).map(move |r| (r * 6 + c).to_string())).collect();
    let cases: &[(&str, &str, &[String])] = &[
        ("v0.11-array-float64.joblib", "array-float64.joblib", &rows),
        ("v0.11-array-fortran-order.joblib", "array-fortran-order.joblib", &down),
        ("v0.11-array-big-endian.joblib", "array-big-endian.joblib", &rows[..6]),
    ];
    for (old, new, want) in cases {
        let held = numbers(&std::fs::read(dir.join(old)).unwrap(), old);
        assert_eq!(&held, want, "{old}");
        // And the same array written by joblib 1.6, which pads its run: two
        // layouts, one reading.
        assert_eq!(numbers(&std::fs::read(dir.join(new)).unwrap(), new), held, "{old} against {new}");
    }
}

/// A bundle of two arrays: the second wrapper names the words the first
/// spelled, and neither has a padding byte in front of its numbers.
#[test]
fn an_unaligned_bundle_holds_both_its_arrays() {
    let Some(dir) = qubero_samples::dir("joblib") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    for name in ["v0.11-dict-of-arrays.joblib", "v1.1-dict-of-arrays.joblib"] {
        let bytes = std::fs::read(dir.join(name)).unwrap();
        let (doc, mut ev) = open(&bytes, "joblib");
        let mut rows = Vec::new();
        walk(&doc, &mut ev, &[], &mut rows);
        let runs: Vec<Vec<usize>> = rows.iter().filter(|(n, t, _)| n == "numbers" && t.ends_with("[]")).map(|(_, _, p)| p.clone()).collect();
        assert_eq!(runs.len(), 2, "{name}");
        // `weights` is `arange(24, float64).reshape(4, 6)` and `bias` four
        // float32 zeroes.
        assert_eq!(cells(&doc, &mut ev, &runs[0]), (0..24u64).map(|n| n.to_string()).collect::<Vec<_>>(), "{name}");
        assert_eq!(cells(&doc, &mut ev, &runs[1]), vec!["0"; 4], "{name}");
    }
}

/// A compressed file from the same release holds the un-aligned stream, byte
/// for byte like the plain one.
#[test]
fn an_unaligned_file_inside_zlib_is_the_same_file() {
    let Some(dir) = qubero_samples::dir("joblib") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let plain = std::fs::read(dir.join("v1.1-dict-of-arrays.joblib")).unwrap();
    let bytes = std::fs::read(dir.join("v1.1-dict-of-arrays-zlib.joblib")).unwrap();
    let window = &bytes[..bytes.len().min(0x9000)];
    assert_eq!(formats::sniff(window, bytes.len() as u64), Some("zlib"));
    let (doc, mut ev) = open(&bytes, "zlib");
    let run = decoded(&doc, &mut ev).expect("a compressed run");
    let id = ev.open_space(&doc, 0, &run).unwrap().expect("the stream opens");
    let space = ev.space(id).unwrap();
    assert_eq!(space.template, "joblib");
    assert_eq!(space.bytes(), plain.as_slice());
}

/// A compressed file joblib 0.9 wrote is joblib's own container and not the
/// compressor's: `ZF`, the unpacked length as text, and a zlib stream. What
/// comes out of the stream is the whole pickle, arrays and all, because a
/// compressed file has nowhere to keep a `.npy` beside it.
#[test]
fn a_joblib_0_9_compressed_file_is_its_own_container() {
    let Some(dir) = qubero_samples::dir("joblib") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let bytes = std::fs::read(dir.join("v0.9-dict-of-arrays-zfile.joblib")).unwrap();
    let window = &bytes[..bytes.len().min(0x9000)];
    assert_eq!(formats::sniff(window, bytes.len() as u64), Some("joblibzfile"));
    let (doc, mut ev) = open(&bytes, "joblibzfile");
    // The header says how long the pickle is, as `hex()` spells it.
    let length = ev.node(&doc, &[1]).unwrap();
    assert_eq!(length.name, "unpacked size");
    assert_eq!(length.value, Value::Str("0x226              ".into()));
    let run = decoded(&doc, &mut ev).expect("a compressed run");
    let id = ev.open_space(&doc, 0, &run).unwrap().expect("the stream opens");
    let space = ev.space(id).unwrap();
    assert_eq!(space.template, "picklefpf");
    assert!(space.recognised, "the template came from the stream and not from the bytes");
    let held = space.bytes().to_vec();
    assert_eq!(held.len(), 0x226);
    let found = formats::pickle::familiar::recognise(&held).expect("a form reads what came out");
    assert_eq!(found.form, "numpy-array-p2-p3-v1");
}

/// An array of objects has no numbers to write beside the pickle, so joblib
/// 0.9 wrote no wrapper for one at all and the file is an ordinary NumPy
/// pickle.
#[test]
fn a_joblib_0_9_object_array_has_no_wrapper() {
    let Some(dir) = qubero_samples::dir("joblib") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let bytes = std::fs::read(dir.join("v0.9-array-of-objects.joblib")).unwrap();
    let found = formats::pickle::familiar::recognise(&bytes).expect("no form");
    assert_eq!(found.form, "numpy-array-p2-p3-v1");
    let window = &bytes[..bytes.len().min(0x9000)];
    assert_eq!(formats::sniff(window, bytes.len() as u64), Some("picklefpf"));
}

/// What joblib wrote before 0.10: the pickle names a `.npy` file per array,
/// and the numbers are in those files and nowhere in this one.
///
/// The main file is an ordinary pickle by every test there is -- the opcodes
/// run to the end of it -- so it is not recognised as a joblib file and does
/// not need to be. What says it is one is the form it matches.
#[test]
fn a_joblib_0_9_pickle_names_the_npy_file_each_array_is_in() {
    let Some(dir) = qubero_samples::dir("joblib") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    // The folder carries the version, because each pickle names its own
    // `.npy` by name and renaming either would leave it naming a file that is
    // not there.
    let dir = dir.join("v0.9-npy-files");
    let cases: &[(&str, &[&str])] = &[
        ("joblib-array-float64.joblib", &["joblib-array-float64.joblib_01.npy"]),
        ("joblib-dict-of-arrays.joblib", &["joblib-dict-of-arrays.joblib_01.npy", "joblib-dict-of-arrays.joblib_02.npy"]),
        // A `numpy.matrix`, which the wrapper names the same way it names an
        // ndarray: the class is in the `.npy` beside it and not in the
        // pickle, so this file says nothing about which of NumPy's array
        // classes it is.
        ("joblib-matrix.joblib", &["joblib-matrix.joblib_01.npy"]),
    ];
    for (name, files) in cases {
        let bytes = std::fs::read(dir.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let found = formats::pickle::familiar::recognise(&bytes).unwrap_or_else(|| panic!("{name}: no form"));
        assert_eq!(found.form, "joblib-npy-files-p2-p3-v1", "{name}");
        // The form is what says a pickle was written by joblib: the opcodes
        // run to the end of the file, so no probe can tell it from any other
        // pickle and the familiar reading is what names it.
        let window = &bytes[..bytes.len().min(0x9000)];
        assert_eq!(formats::sniff(window, bytes.len() as u64), Some("picklefpf"), "{name}");

        // Each wrapper says which file, and the file it says is beside it.
        let (doc, mut ev) = open(&bytes, "picklefpf");
        let mut rows = Vec::new();
        walk(&doc, &mut ev, &[], &mut rows);
        let named: Vec<String> = rows
            .iter()
            // The root node is called `file` as well, so the row wanted is
            // the one inside a wrapper rather than the whole document.
            .filter(|(n, _, at)| n == "file" && !at.is_empty())
            .map(|(_, _, at)| match ev.node(&doc, at).unwrap().value {
                Value::Str(s) => s,
                other => panic!("{name}: {other:?}"),
            })
            .collect();
        assert_eq!(named, *files, "{name}");
        // And each of those is an ordinary `.npy` the existing template reads.
        for file in *files {
            let held = std::fs::read(dir.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
            assert_eq!(formats::sniff(&held, held.len() as u64), Some("npy"), "{file}");
        }
    }
}

/// The numbers of an array joblib 0.9 wrote are in the `.npy` beside the
/// pickle, and they are the same numbers a later release wrote inline.
#[test]
fn the_npy_beside_a_joblib_0_9_pickle_holds_the_numbers() {
    let Some(dir) = qubero_samples::dir("joblib") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let bytes = std::fs::read(dir.join("v0.9-npy-files/joblib-array-float64.joblib_01.npy")).unwrap();
    let (doc, mut ev) = open(&bytes, "npy");
    let mut rows = Vec::new();
    walk(&doc, &mut ev, &[], &mut rows);
    // Four rows of six, which the template lays out as rows of values.
    let at = rows.iter().find(|(n, t, _)| n == "data" && t.ends_with("[][]")).expect("the numbers");
    let held: Vec<String> = (0..4)
        .flat_map(|r| {
            let mut row = at.2.clone();
            row.push(r);
            cells(&doc, &mut ev, &row)
        })
        .collect();
    // `arange(24, float64).reshape(4, 6)`, which is what every other
    // `array-float64` in this folder holds.
    assert_eq!(held, (0..24u64).map(|n| n.to_string()).collect::<Vec<_>>());
}

fn open(bytes: &[u8], template: &str) -> (Document<MemSource>, Evaluator) {
    (Document::new(MemSource(bytes.to_vec())), Evaluator::new(formats::builtin(template).unwrap()))
}

/// Every node of the tree: its name, its type and where it is.
fn walk(doc: &Document<MemSource>, ev: &mut Evaluator, at: &[usize], out: &mut Vec<(String, String, Vec<usize>)>) {
    let node = ev.node(doc, at).unwrap_or_else(|e| panic!("{at:?}: {e:?}"));
    let (name, ty, count) = (node.name.clone(), node.type_name.clone(), node.child_count as usize);
    out.push((name, ty.clone(), at.to_vec()));
    // A run of numbers is counted rather than walked: it is a value, not a
    // part of the file's shape.
    if ty.ends_with("[]") {
        return;
    }
    for i in 0..count {
        let mut next = at.to_vec();
        next.push(i);
        walk(doc, ev, &next, out);
    }
}

/// The one array's numbers, as the text a reader sees in each cell.
fn numbers(bytes: &[u8], where_: &str) -> Vec<String> {
    let (doc, mut ev) = open(bytes, "joblib");
    let mut rows = Vec::new();
    walk(&doc, &mut ev, &[], &mut rows);
    let at = rows
        .iter()
        .find(|(name, ty, _)| name == "numbers" && ty.ends_with("[]"))
        .unwrap_or_else(|| panic!("{where_}: no numbers row"));
    cells(&doc, &mut ev, &at.2)
}

fn cells(doc: &Document<MemSource>, ev: &mut Evaluator, at: &[usize]) -> Vec<String> {
    let node = ev.node(doc, at).unwrap();
    (0..node.child_count as usize)
        .map(|i| {
            let mut here = at.to_vec();
            here.push(i);
            match ev.node(doc, &here).unwrap().value {
                Value::Float(f) => format!("{f}"),
                Value::Int(n) => n.to_string(),
                Value::UInt(n) => n.to_string(),
                other => panic!("{other:?}"),
            }
        })
        .collect()
}

/// The first field of a reading whose bytes are a stream of their own, which
/// is what a compressed file unpacks to.
fn decoded(doc: &Document<MemSource>, ev: &mut Evaluator) -> Option<Vec<usize>> {
    fn find(doc: &Document<MemSource>, ev: &mut Evaluator, at: &[usize], depth: usize) -> Option<Vec<usize>> {
        if depth > 6 {
            return None;
        }
        let node = ev.node(doc, at).ok()?;
        if node.decoded {
            return Some(at.to_vec());
        }
        let count = node.child_count as usize;
        for i in 0..count {
            let mut next = at.to_vec();
            next.push(i);
            if let Some(found) = find(doc, ev, &next, depth + 1) {
                return Some(found);
            }
        }
        None
    }
    find(doc, ev, &[], 0)
}
