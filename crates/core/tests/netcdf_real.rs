//! The NetCDF samples, read as far as their numbers.
//!
//! The smoke test over the whole collection checks that a file still reads.
//! This one checks that it reads as what it is: the dimensions shape the data,
//! and a record variable's records are all placed rather than only its first.
//!
//! And that the records are placed where the file put them rather than where
//! the header's arithmetic suggests. A `vsize` is rounded up to four, but a
//! file with exactly one record variable writes no padding between records,
//! so `unpadded-records-cdf*.nc` steps by six bytes while saying eight. Both
//! `scipy.io.netcdf_file` and `netCDF4` read every record of those files as
//! the ramp they hold, which is what the values below are checked against.

use std::path::PathBuf;

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

fn sample(name: &str) -> Option<PathBuf> {
    qubero_samples::roots().into_iter().map(|r| r.join("netcdf").join(name)).find(|p| p.exists())
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
        let data = ev.node(&d, &[7, 8]).unwrap();
        assert!(data.child_count > 0, "{} placed no data", path.display());
        for i in 0..data.child_count as usize {
            let var = ev.node(&d, &[7, 8, i]).unwrap();
            assert_eq!(var.type_name, "VarData", "{} variable {i}", path.display());
            // The shape fields, which are what the dimensions came to.
            let rows = ev.node(&d, &[7, 8, i, 7]).unwrap().value.as_int().unwrap();
            let row = ev.node(&d, &[7, 8, i, 3]).unwrap().value.as_int().unwrap();
            assert!(rows >= 0 && row >= 1, "{} variable {i}: {rows} by {row}", path.display());
            // And the values themselves, which must not be a run of bytes.
            let values = ev.node(&d, &[7, 8, i, 9]).unwrap();
            assert_ne!(values.type_name, "bytes[]", "{} variable {i}", path.display());
        }
        // How far apart two records are, which nothing in the file writes.
        let recsize = ev.node(&d, &[7, 7]).unwrap().value.as_int().unwrap();
        assert!(recsize > 0, "{} has no record variables", path.display());
        // The record count, and a record variable that has that many.
        let numrecs = ev.node(&d, &[2]).unwrap().value.as_int().unwrap();
        let mut found = false;
        for i in 0..ev.node(&d, &[7, 8]).unwrap().child_count as usize {
            if ev.node(&d, &[7, 8, i, 4]).unwrap().value != Value::Int(1) {
                continue;
            }
            found = true;
            let later = ev.node(&d, &[7, 8, i, 11]).unwrap();
            assert_eq!(i128::from(later.child_count), numrecs - 1, "{} variable {i}", path.display());
            // The last record is a whole file's worth of records on from the
            // first, and still inside the file.
            let first = ev.node(&d, &[7, 8, i, 9]).unwrap();
            let last = ev.node(&d, &[7, 8, i, 11, later.child_count as usize - 1, 0]).unwrap();
            let expected = first.offset_bits + ((numrecs - 1) * recsize) as u64 * 8;
            assert_eq!(last.offset_bits, expected, "{} variable {i}", path.display());
        }
        assert!(found, "{} has no record variable", path.display());
    }
    if seen == 0 {
        eprintln!("{}", qubero_samples::missing());
    }
}

/// `pulse` is the only record variable in its file, so its six-byte records
/// follow each other with nothing between them and the header's `vsize` of
/// eight is not where the next one starts. Record k holds 10k+1, 10k+2 and
/// 10k+3, so reading the last record's numbers is a test of where it was
/// placed: two bytes further on each record and the ramp comes apart.
#[test]
fn one_record_variable_of_an_odd_width_runs_on_without_padding() {
    let mut seen = 0;
    for version in [1, 2, 5] {
        let Some(path) = sample(&format!("unpadded-records-cdf{version}.nc")) else { continue };
        seen += 1;
        let d = Document::new(MemSource(std::fs::read(&path).unwrap()));
        let mut ev = Evaluator::new(formats::builtin("netcdf").unwrap());
        let file = path.display();
        // What the header says a record costs, and what it actually does.
        assert_eq!(ev.node(&d, &[7, 2, 0, 5]).unwrap().value.as_int(), Some(8), "{file} vsize");
        assert_eq!(ev.node(&d, &[7, 7]).unwrap().value.as_int(), Some(6), "{file} recsize");
        let numrecs = ev.node(&d, &[2]).unwrap().value.as_int().unwrap();
        assert_eq!(numrecs, 6, "{file}");
        // Three shorts a record and no bytes left over to call padding.
        let first = ev.node(&d, &[7, 8, 0, 9]).unwrap();
        assert_eq!((first.child_count, first.size_bits), (3, 48), "{file}");
        assert_eq!(ev.node(&d, &[7, 8, 0, 10]).unwrap().size_bits, 0, "{file} padding");
        let later = ev.node(&d, &[7, 8, 0, 11]).unwrap();
        assert_eq!(i128::from(later.child_count), numrecs - 1, "{file}");
        // Every record after the first, by where it is and by what it holds.
        for k in 1..numrecs as usize {
            let at = [7, 8, 0, 11, k - 1, 0];
            let slab = ev.node(&d, &at).unwrap();
            assert_eq!(slab.offset_bits, first.offset_bits + k as u64 * 6 * 8, "{file} record {k}");
            for i in 0..3usize {
                let mut value = at.to_vec();
                value.push(i);
                let want = (k * 10 + i + 1) as i128;
                assert_eq!(ev.node(&d, &value).unwrap().value.as_int(), Some(want), "{file} record {k}");
            }
        }
    }
    if seen == 0 {
        eprintln!("{}", qubero_samples::missing());
    }
}

/// The same widths with three record variables instead of one, which is where
/// the padding is real: every slab is rounded up to four, a record is sixteen
/// bytes, and the last of each variable's records is four records on.
#[test]
fn several_record_variables_of_odd_widths_keep_their_padding() {
    let Some(path) = sample("padded-records-cdf1.nc") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let d = Document::new(MemSource(std::fs::read(&path).unwrap()));
    let mut ev = Evaluator::new(formats::builtin("netcdf").unwrap());
    assert_eq!(ev.node(&d, &[7, 7]).unwrap().value.as_int(), Some(16));
    let numrecs = ev.node(&d, &[2]).unwrap().value.as_int().unwrap();
    assert_eq!(numrecs, 5);
    let last = numrecs as usize - 2;
    // pulse: six bytes used and two padded, and the last record still a ramp.
    assert_eq!(ev.node(&d, &[7, 8, 0, 9]).unwrap().size_bits, 48);
    assert_eq!(ev.node(&d, &[7, 8, 0, 10]).unwrap().size_bits, 16);
    assert_eq!(ev.node(&d, &[7, 8, 0, 11, last, 0, 0]).unwrap().value.as_int(), Some(41));
    assert_eq!(ev.node(&d, &[7, 8, 0, 11, last, 0, 2]).unwrap().value.as_int(), Some(43));
    // mark: one byte a record, not the four its vsize reserves.
    let mark = ev.node(&d, &[7, 8, 1, 9]).unwrap();
    assert_eq!((mark.child_count, mark.size_bits), (1, 8));
    assert_eq!(ev.node(&d, &[7, 8, 1, 10]).unwrap().size_bits, 24);
    assert_eq!(ev.node(&d, &[7, 8, 1, 11, last, 0, 0]).unwrap().value.as_int(), Some(104));
    // label: a character variable, whose innermost dimension is the string.
    assert_eq!(ev.node(&d, &[7, 8, 2, 9]).unwrap().value, Value::Str("r0!".into()));
    assert_eq!(ev.node(&d, &[7, 8, 2, 11, last, 0]).unwrap().value, Value::Str("r4!".into()));
    // And each of them a whole record on from the one before.
    let slab = ev.node(&d, &[7, 8, 1, 11, last, 0]).unwrap();
    assert_eq!(slab.offset_bits, mark.offset_bits + (last as u64 + 1) * 16 * 8);
}
