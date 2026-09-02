//! The NetCDF samples, read as far as their numbers.
//!
//! The smoke test over the whole collection checks that a file still reads.
//! This one checks that it reads as what it is: the dimensions shape the data,
//! and a record variable's records are all placed rather than only its first.

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
    roots.into_iter().map(|r| r.join("netcdf").join(name)).find(|p| p.exists())
}

#[test]
fn a_real_file_reads_as_rows_and_as_records() {
    let mut seen = 0;
    for version in [1, 2, 5] {
        let Some(path) = sample(&format!("sst-cdf{version}.nc")) else { continue };
        seen += 1;
        let d = Document::new(MemSource(std::fs::read(&path).unwrap()));
        let mut ev = Evaluator::new(formats::builtin("netcdf").unwrap());
        // Every variable's data is placed, and none of it fails to resolve.
        let data = ev.node(&d, &[7, 5]).unwrap();
        assert!(data.child_count > 0, "{} placed no data", path.display());
        for i in 0..data.child_count as usize {
            let var = ev.node(&d, &[7, 5, i]).unwrap();
            assert_eq!(var.type_name, "VarData", "{} variable {i}", path.display());
            // The shape fields, which are what the dimensions came to.
            let rows = ev.node(&d, &[7, 5, i, 6]).unwrap().value.as_int().unwrap();
            let row = ev.node(&d, &[7, 5, i, 3]).unwrap().value.as_int().unwrap();
            assert!(rows >= 0 && row >= 1, "{} variable {i}: {rows} by {row}", path.display());
            // And the values themselves, which must not be a run of bytes.
            let values = ev.node(&d, &[7, 5, i, 8]).unwrap();
            assert_ne!(values.type_name, "bytes[]", "{} variable {i}", path.display());
        }
        // How far apart two records are, which nothing in the file writes.
        let recsize = ev.node(&d, &[7, 4]).unwrap().value.as_int().unwrap();
        assert!(recsize > 0, "{} has no record variables", path.display());
        // The record count, and a record variable that has that many.
        let numrecs = ev.node(&d, &[2]).unwrap().value.as_int().unwrap();
        let mut found = false;
        for i in 0..ev.node(&d, &[7, 5]).unwrap().child_count as usize {
            if ev.node(&d, &[7, 5, i, 4]).unwrap().value != Value::Int(1) {
                continue;
            }
            found = true;
            let later = ev.node(&d, &[7, 5, i, 10]).unwrap();
            assert_eq!(i128::from(later.child_count), numrecs - 1, "{} variable {i}", path.display());
            // The last record is a whole file's worth of records on from the
            // first, and still inside the file.
            let first = ev.node(&d, &[7, 5, i, 8]).unwrap();
            let last = ev.node(&d, &[7, 5, i, 10, later.child_count as usize - 1, 0]).unwrap();
            let expected = first.offset_bits + ((numrecs - 1) * recsize) as u64 * 8;
            assert_eq!(last.offset_bits, expected, "{} variable {i}", path.display());
        }
        assert!(found, "{} has no record variable", path.display());
    }
    if seen == 0 {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
    }
}
