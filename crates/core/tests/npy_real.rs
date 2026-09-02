//! Real NPY files, read in the shape their headers give them.
//!
//! Three of them: a C-ordered grid of doubles, the same idea written the
//! Fortran way round, and a structured dtype of several hundred named fields.
//! What it checks is the part the header decides rather than the bytes: how
//! many rows there are, which way a row runs, and what a record dtype is read
//! as.
//!
//! The files live in the sample collection rather than here. Point
//! `QUBERO_SAMPLES` at it, or keep it beside the repository as
//! `qubero-samples`. With neither, the test says so and passes.

use std::path::PathBuf;

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

fn sample(name: &str) -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(set) = std::env::var("QUBERO_SAMPLES") {
        roots.extend(set.split(';').filter(|s| !s.is_empty()).map(PathBuf::from));
    }
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../qubero-samples"));
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../qubero-samples"));
    roots.into_iter().map(|r| r.join("npy").join(name)).find(|p| p.exists())
}

fn read(name: &str) -> Option<(Document<MemSource>, Evaluator)> {
    let path = sample(name)?;
    let doc = Document::new(MemSource(std::fs::read(&path).unwrap()));
    Some((doc, Evaluator::new(formats::builtin("npy").unwrap())))
}

#[test]
fn a_real_grid_is_rows_of_its_last_dimension() {
    let Some((d, mut ev)) = read("grid-f64-v1.npy") else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    // `'shape': (8, 12)` in C order: eight rows of twelve doubles.
    let data = ev.node(&d, &[5]).unwrap();
    assert_eq!(data.child_count, 8);
    let row = ev.node(&d, &[5, 0]).unwrap();
    assert_eq!((row.type_name.as_str(), row.child_count), ("f64 le[]", 12));
    assert_eq!(row.size_bits, 12 * 64);
    assert_eq!(data.offset_bits + data.size_bits, d.len_bits());
}

#[test]
fn a_real_fortran_ordered_array_runs_the_other_way() {
    let Some((d, mut ev)) = read("columns-i2be-fortran.npy") else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    // `'shape': (6, 10)` written column by column: ten runs of six.
    let data = ev.node(&d, &[5]).unwrap();
    assert_eq!(data.child_count, 10);
    let column = ev.node(&d, &[5, 0]).unwrap();
    assert_eq!((column.type_name.as_str(), column.child_count), ("i16 be[]", 6));
    assert_eq!(data.offset_bits + data.size_bits, d.len_bits());
}

#[test]
fn a_real_structured_dtype_lists_the_fields_it_names() {
    let Some((d, mut ev)) = read("channels-structured-v2.npy") else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    // The record view covers the dtype's own bytes and takes none of its own.
    let record = ev.node(&d, &[4, 2]).unwrap();
    assert_eq!(record.size_bits, 0);
    let fields = ev.node(&d, &[4, 2, 0]).unwrap();
    assert!(fields.child_count > 100, "{} fields", fields.child_count);
    assert_eq!(ev.node(&d, &[4, 2, 0, 0, 1]).unwrap().value, Value::Str("channel_0000".into()));
    assert_eq!(ev.node(&d, &[4, 2, 0, 0, 4]).unwrap().value, Value::Str("<f4".into()));
    assert_eq!(ev.node(&d, &[4, 2, 0, 1, 1]).unwrap().value, Value::Str("channel_0001".into()));
    // And the numbers are records, each value typed by the entry of that list
    // that names it: over a hundred f32 columns a record.
    let data = ev.node(&d, &[5]).unwrap();
    assert_eq!(data.type_name, "Record[]");
    let n = fields.child_count;
    assert_eq!(ev.node(&d, &[5, 0, 0]).unwrap().child_count, n);
    assert_eq!(ev.node(&d, &[5, 0, 0, 0]).unwrap().type_name, "f32 le");
    assert_eq!(ev.node(&d, &[5, 0]).unwrap().size_bits, n * 4 * 8);
}
