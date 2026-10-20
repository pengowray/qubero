//! Real ASDF files, read against what the asdf library reads from them.
//!
//! `tools/asdf_reference.py` wrote `reference.tsv` beside the files: for every
//! numeric array, values at the first and last elements, the middle, a seeded
//! random few, and either side of every 4 MiB, where asdf's lz4 starts a new
//! chunk. Each is read here through the template, down the array the tree path
//! names, and has to be the same number.
//!
//! The sample collection's `asdf` folder has the small files, written by
//! `tools/make_asdf_samples.py` there. One simulated Roman WFI exposure is
//! checked from `qubero2-extras/roman` beside the checkout or an ancestor of
//! it, or wherever `QUBERO_ROMAN` points: its Level 1 file (1.07 GB, stored
//! blocks), its calibrated Level 2 image (205 MB, lz4) and its WCS file. They
//! are too large for the collection, and no licence for them is stated.
//! Without either folder the tests say so and pass.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

fn samples() -> Option<PathBuf> {
    qubero_samples::roots().into_iter().map(|r| r.join("asdf")).find(|p| p.join("reference.tsv").exists())
}

fn roman() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("QUBERO_ROMAN") {
        return Some(PathBuf::from(p));
    }
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.ancestors().map(|a| a.join("qubero2-extras").join("roman")).find(|p| p.join("reference.tsv").exists())
}

/// One expected value: where it is in the array, and what asdf read there.
struct Expected {
    index: Vec<usize>,
    dtype: String,
    value: String,
}

/// The reference, by file and then by array path.
fn reference(dir: &Path) -> BTreeMap<String, BTreeMap<String, Vec<Expected>>> {
    let text = std::fs::read_to_string(dir.join("reference.tsv")).unwrap();
    let mut out: BTreeMap<String, BTreeMap<String, Vec<Expected>>> = BTreeMap::new();
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let cols: Vec<&str> = line.split('\t').collect();
        let [file, array, _shape, dtype, index, value] = cols[..] else { panic!("bad line {line}") };
        let index = index.split(',').map(|i| i.parse().unwrap()).collect();
        let e = Expected { index, dtype: dtype.into(), value: value.into() };
        out.entry(file.into()).or_default().entry(array.into()).or_default().push(e);
    }
    out
}

/// Every array the template placed, by its path in the tree: the children of
/// what each block unpacks to, a few levels under the block's data.
fn arrays(ev: &mut Evaluator, d: &Document<MemSource>) -> BTreeMap<String, Vec<usize>> {
    let mut out = BTreeMap::new();
    let blocks = ev.child_named(d, &[], "blocks").unwrap().unwrap();
    let count = ev.node(d, &blocks).unwrap().child_count;
    for b in 0..count as usize {
        let block = [blocks.as_slice(), &[b]].concat();
        let data = ev.child_named(d, &block, "data").unwrap().unwrap();
        let mut todo = vec![(data, 0)];
        while let Some((at, depth)) = todo.pop() {
            let n = ev.node(d, &at).unwrap();
            if n.type_name == "Arrays" {
                for i in 0..n.child_count as usize {
                    let child = [at.as_slice(), &[i]].concat();
                    out.insert(ev.node(d, &child).unwrap().name, child);
                }
                continue;
            }
            if depth < 3 && n.name != "chunks" {
                for i in 0..n.child_count as usize {
                    todo.push(([at.as_slice(), &[i]].concat(), depth + 1));
                }
            }
        }
    }
    out
}

/// The element at `index` of the array at `at`, through the structures a view
/// and a spaced element wrap it in.
fn element(ev: &mut Evaluator, d: &Document<MemSource>, at: &[usize], index: &[usize]) -> Value {
    let mut path = at.to_vec();
    for &i in index {
        if ev.node(d, &path).unwrap().type_name == "View" {
            path = ev.child_named(d, &path, "view").unwrap().unwrap();
        }
        path.push(i);
        if ev.node(d, &path).unwrap().type_name == "Strided" {
            path = ev.child_named(d, &path, "value").unwrap().unwrap();
        }
    }
    ev.node(d, &path).unwrap().value
}

/// Whether the template's value is the one asdf read. A float is compared at
/// the width it is stored in: the template keeps the shortest decimal that is
/// the same 16 or 32-bit float, and asdf writes out that float exactly.
fn same(value: &Value, e: &Expected) -> bool {
    let expected = e.value.as_str();
    match value {
        Value::Float(f) => match expected.parse::<f64>() {
            _ if expected == "nan" => f.is_nan(),
            Err(_) => false,
            Ok(x) if e.dtype.ends_with("f4") => x as f32 == *f as f32,
            // Within half a step of a 16-bit float at that size: eleven bits
            // of precision, and a fixed step below the smallest normal.
            Ok(x) if e.dtype.ends_with("f2") => (x - f).abs() <= (x.abs() * 2f64.powi(-11)).max(2f64.powi(-25)),
            Ok(x) => x == *f,
        },
        Value::Int(n) => expected.parse::<i128>() == Ok(*n),
        Value::UInt(n) => expected.parse::<u128>() == Ok(*n as u128),
        _ => false,
    }
}

/// Read every file the reference names, and say what differs.
fn check(dir: &Path) {
    let mut wrong = Vec::new();
    let mut checked = 0;
    for (file, expected) in reference(dir) {
        let bytes = std::fs::read(dir.join(&file)).unwrap();
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(formats::builtin("asdf").unwrap());
        let found = arrays(&mut ev, &d);
        for (array, values) in expected {
            let Some(at) = found.get(&array) else {
                wrong.push(format!("{file}: no array {array} in any block"));
                continue;
            };
            for e in values {
                let v = element(&mut ev, &d, at, &e.index);
                checked += 1;
                if !same(&v, &e) {
                    wrong.push(format!("{file}: {array}{:?} is {v:?}, asdf says {}", e.index, e.value));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{} of {checked} values differ:\n{}", wrong.len(), wrong.join("\n"));
    assert!(checked > 0);
}

#[test]
fn the_samples_read_as_the_asdf_library_reads_them() {
    let Some(dir) = samples() else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    check(&dir);
}

#[test]
fn a_roman_exposure_reads_as_the_asdf_library_reads_it() {
    let Some(dir) = roman() else {
        eprintln!("no qubero2-extras/roman beside the checkout, and QUBERO_ROMAN is not set");
        return;
    };
    check(&dir);
}
