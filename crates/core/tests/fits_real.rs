//! A real FITS binary table, read as its rows and columns.
//!
//! `tb.fits` is one of the files the FITS tools have been tested against since
//! 2001: a header-only primary unit, and then a table of four columns, one of
//! each of the kinds a table is mostly written in. What it checks is that the
//! `TFORMn` cards typed the columns, since that is the part no fixed layout
//! could do: the width of a row is in the header, and what is in it is in the
//! header too, spelled out a keyword at a time.
//!
//! The file lives in the sample collection rather than here. Point
//! `QUBERO_SAMPLES` at it, or keep it beside the repository as
//! `qubero-samples`. With neither, the test says so and passes.

use std::path::PathBuf;

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

fn sample() -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(set) = std::env::var("QUBERO_SAMPLES") {
        roots.extend(set.split(';').filter(|s| !s.is_empty()).map(PathBuf::from));
    }
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../qubero-samples"));
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../qubero-samples"));
    roots.into_iter().map(|r| r.join("fits/tb.fits")).find(|p| p.exists())
}

#[test]
fn a_real_binary_tables_columns_are_typed_by_its_header() {
    let Some(path) = sample() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let doc = Document::new(MemSource(std::fs::read(&path).unwrap()));
    let mut ev = Evaluator::new(formats::builtin("fits").unwrap());
    // The primary unit holds nothing, so the table is the second one.
    let hdu = [0usize, 1];
    let at = |tail: &[usize]| -> Vec<usize> { hdu.iter().chain(tail).copied().collect() };

    // Two rows of twelve bytes, and no heap after them.
    let rows = ev.node(&doc, &at(&[2, 0])).unwrap();
    assert_eq!(rows.child_count, 2);
    assert_eq!(ev.node(&doc, &at(&[2, 0, 0])).unwrap().size_bits, 12 * 8);
    assert_eq!(ev.node(&doc, &at(&[2, 1])).unwrap().size_bits, 0);

    // `TFORM1 = '1J'`: one 32-bit integer, big-endian as everything here is.
    let c1 = ev.node(&doc, &at(&[2, 0, 0, 0])).unwrap();
    assert_eq!((c1.type_name.as_str(), c1.child_count), ("i32 be[]", 1));
    assert_eq!(ev.node(&doc, &at(&[2, 0, 0, 0, 0])).unwrap().value, Value::Int(1));
    assert_eq!(ev.node(&doc, &at(&[2, 0, 1, 0, 0])).unwrap().value, Value::Int(2));

    // `TFORM2 = '3A'`: three characters, read as one run of text.
    let c2 = ev.node(&doc, &at(&[2, 0, 0, 1])).unwrap();
    assert_eq!(c2.size_bits, 3 * 8);
    assert!(matches!(&c2.value, Value::Str(s) if s.trim() == "abc"), "{:?}", c2.value);

    // `TFORM3 = '1E'`: one float.
    assert_eq!(ev.node(&doc, &at(&[2, 0, 0, 2, 0])).unwrap().value, Value::Float(1.1f32 as f64));

    // `TFORM4 = '1L'`: a logical, written as the letter T or F; the first row says F.
    let c4 = ev.node(&doc, &at(&[2, 0, 0, 3])).unwrap();
    assert_eq!(c4.size_bits, 8);
    assert!(matches!(&c4.value, Value::Str(s) if s == "F"), "{:?}", c4.value);

    // The columns add up to the width the header gave the row.
    let widths: u64 = (0..4).map(|i| ev.node(&doc, &at(&[2, 0, 0, i])).unwrap().size_bits).sum();
    assert_eq!(widths, 12 * 8);
}
