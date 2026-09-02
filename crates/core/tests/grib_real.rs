//! Real GRIB files, read as far as their numbers.
//!
//! What it checks is the part the file decides rather than the bytes: how many
//! messages there are, which template each section was written to, and that a
//! simply packed section 7 holds as many values as section 5 counted, each as
//! wide as section 5 said.
//!
//! The files live in the sample collection rather than here. Point
//! `QUBERO_SAMPLES` at it, or keep it beside the repository as
//! `qubero-samples`. With neither, the test says so and passes.

use std::path::PathBuf;

use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats;
use qubero_core::source::MemSource;

fn sample(name: &str) -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(set) = std::env::var("QUBERO_SAMPLES") {
        roots.extend(set.split(';').filter(|s| !s.is_empty()).map(PathBuf::from));
    }
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../qubero-samples"));
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../qubero-samples"));
    roots.into_iter().map(|r| r.join("grib").join(name)).find(|p| p.exists())
}

fn read(name: &str) -> Option<(Document<MemSource>, Evaluator)> {
    let path = sample(name)?;
    let doc = Document::new(MemSource(std::fs::read(&path).unwrap()));
    Some((doc, Evaluator::new(formats::builtin("grib").unwrap())))
}

/// Every section of every message in a file, as (number, path).
fn sections(d: &Document<MemSource>, ev: &mut Evaluator) -> Vec<(i128, Vec<usize>)> {
    let mut out = Vec::new();
    for m in 0..ev.node(d, &[]).unwrap().child_count as usize {
        if ev.node(d, &[m]).unwrap().type_name != "Message" {
            continue;
        }
        let list = [m, 1, 4];
        for s in 0..ev.node(d, &list).unwrap().child_count as usize {
            let path = vec![m, 1, 4, s];
            let mut number = path.clone();
            number.push(1);
            if let Ok(n) = ev.node(d, &number) {
                if let Some(n) = n.value.as_int() {
                    out.push((n, path));
                }
            }
        }
    }
    out
}

#[test]
fn a_real_message_reads_as_its_grid_and_its_values() {
    let Some((d, mut ev)) = read("regular_ll_sfc.grib2") else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    assert_eq!(ev.node(&d, &[]).unwrap().child_count, 1);
    let found = sections(&d, &mut ev);
    let numbers: Vec<_> = found.iter().map(|(n, _)| *n).collect();
    assert_eq!(numbers, vec![1, 3, 4, 5, 6, 7]);
    // Section 3 is a plain latitude/longitude grid, 16 by 31.
    let grid = [0, 1, 4, 1, 2, 5];
    assert_eq!(ev.node(&d, &grid).unwrap().type_name, "LatLonGrid");
    let ni = ev.node(&d, &[0, 1, 4, 1, 2, 5, 7]).unwrap().value.as_int().unwrap();
    let nj = ev.node(&d, &[0, 1, 4, 1, 2, 5, 8]).unwrap().value.as_int().unwrap();
    assert_eq!((ni, nj), (16, 31));
    // Its corners, which are the fields written as sign and magnitude.
    let lat = ev.node(&d, &[0, 1, 4, 1, 2, 5, 11]).unwrap().value.as_int().unwrap();
    assert!((-90_000_000..=90_000_000).contains(&lat), "first latitude {lat}");
    // Section 5 is simple packing, and section 7 holds what it counted.
    assert_eq!(ev.node(&d, &[0, 1, 4, 3, 2, 2]).unwrap().type_name, "SimplePacking");
    let count = ev.node(&d, &[0, 1, 4, 3, 2, 0]).unwrap().value.as_int().unwrap();
    let bpv = ev.node(&d, &[0, 1, 4, 3, 2, 2, 3]).unwrap().value.as_int().unwrap();
    assert_eq!(count, ni * nj);
    let values = ev.node(&d, &[0, 1, 4, 5, 2, 2]).unwrap();
    assert_eq!(i128::from(values.child_count), count);
    assert_eq!(u64::from(values.size_bits), (count * bpv) as u64);
}

#[test]
fn a_grid_this_reads_and_one_it_does_not_both_keep_their_extent() {
    // A spectral field: grid template 63 and packing template 53, neither of
    // which reads as fields. What must hold is that the section keeps its
    // length and the bytes stay where they are, so that the message after it
    // is still found.
    if let Some((d, mut ev)) = read("lambert_bf.grib2") {
        let section = ev.node(&d, &[0, 1, 4, 1]).unwrap();
        let length = ev.node(&d, &[0, 1, 4, 1, 0]).unwrap().value.as_int().unwrap();
        assert_eq!(u64::from(section.size_bits), length as u64 * 8);
        assert_eq!(ev.node(&d, &[0, 1, 4, 1, 2, 5]).unwrap().type_name, "bytes[]");
        assert_eq!(ev.node(&d, &[0, 1, 4, 3, 2, 2]).unwrap().type_name, "bytes[]");
        // And the message is as long as its indicator said.
        let message = ev.node(&d, &[0]).unwrap();
        let total = ev.node(&d, &[0, 1, 3]).unwrap().value.as_int().unwrap();
        assert_eq!(u64::from(message.size_bits), total as u64 * 8);
    }
    // A reduced Gaussian grid packed as JPEG 2000: the grid is template 40,
    // which reads, and the packing is 5.40, whose header reads and whose data
    // stays bytes.
    if let Some((d, mut ev)) = read("reduced_gg_sfc_jpeg.grib2") {
        assert_eq!(ev.node(&d, &[0, 1, 4, 1, 2, 5]).unwrap().type_name, "GaussianGrid");
        assert_eq!(ev.node(&d, &[0, 1, 4, 3, 2, 2]).unwrap().type_name, "Jpeg2000Packing");
        let data = ev.node(&d, &[0, 1, 4, 5, 2, 0]).unwrap();
        assert_eq!(data.type_name, "bytes[]");
    }
}

#[test]
fn a_real_edition_1_message_reads_as_its_five_sections() {
    let Some((d, mut ev)) = read("regular_ll_sfc.grib1") else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let message = ev.node(&d, &[0, 1]).unwrap();
    assert_eq!(message.type_name, "Grib1");
    let sections = ev.node(&d, &[0, 1, 2]).unwrap();
    assert_eq!(sections.child_count, 5);
    // The product definition says which parameter and at what level.
    let pds = ev.node(&d, &[0, 1, 2, 0]).unwrap();
    assert_eq!(pds.type_name, "ProductDefinition1");
    let flags = ev.node(&d, &[0, 1, 2, 0, 5]).unwrap().value.as_int().unwrap();
    // A grid definition is there, which is what its own flag says.
    assert_eq!(flags & 0x80, 0x80);
    let grid = ev.node(&d, &[0, 1, 2, 1, 4]).unwrap();
    assert_eq!(grid.type_name, "LatLonGrid1");
    // And the data section holds as many values as the grid has points.
    let ni = ev.node(&d, &[0, 1, 2, 1, 4, 0]).unwrap().value.as_int().unwrap();
    let nj = ev.node(&d, &[0, 1, 2, 1, 4, 1]).unwrap().value.as_int().unwrap();
    let bpv = ev.node(&d, &[0, 1, 2, 3, 6]).unwrap().value.as_int().unwrap();
    let values = ev.node(&d, &[0, 1, 2, 3, 7, 1]).unwrap();
    if bpv > 0 {
        assert!(i128::from(values.child_count) >= ni * nj, "{} values for {ni} by {nj}", values.child_count);
    }
    // The end marker is the last thing in the message.
    let end = ev.node(&d, &[0, 1, 2, 4]).unwrap();
    assert_eq!(end.size_bits, 4 * 8);
    assert_eq!(end.offset_bits + end.size_bits, sections.offset_bits + sections.size_bits);
}

#[test]
fn an_operational_forecast_reads_as_three_messages_on_one_grid() {
    let Some((d, mut ev)) = read("gfs-1p00-3messages.grib2") else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    assert_eq!(ev.node(&d, &[]).unwrap().child_count, 3);
    for m in 0..3usize {
        assert_eq!(ev.node(&d, &[m]).unwrap().type_name, "Message");
        // The same one-degree grid in all three: 360 by 181.
        let grid = ev.node(&d, &[m, 1, 4, 1, 2, 5]).unwrap();
        assert_eq!(grid.type_name, "LatLonGrid", "message {m}");
        let ni = ev.node(&d, &[m, 1, 4, 1, 2, 5, 7]).unwrap().value.as_int().unwrap();
        let nj = ev.node(&d, &[m, 1, 4, 1, 2, 5, 8]).unwrap().value.as_int().unwrap();
        assert_eq!((ni, nj), (360, 181));
        // The grid runs from the north pole down, so the first latitude is
        // the larger and the last one is negative: sign and magnitude.
        let first = ev.node(&d, &[m, 1, 4, 1, 2, 5, 11]).unwrap().value.as_int().unwrap();
        let last = ev.node(&d, &[m, 1, 4, 1, 2, 5, 14]).unwrap().value.as_int().unwrap();
        assert_eq!((first, last), (90_000_000, -90_000_000));
        // Complex packing with spatial differencing: the header reads, and
        // the values stay bytes because their widths are inside them.
        let packing = ev.node(&d, &[m, 1, 4, 3, 2, 2]).unwrap();
        assert_eq!(packing.type_name, "ComplexPackingSpatial", "message {m}");
        let groups = ev.node(&d, &[m, 1, 4, 3, 2, 2, 9]).unwrap().value.as_int().unwrap();
        assert!(groups > 0, "message {m}: {groups} groups");
        let order = ev.node(&d, &[m, 1, 4, 3, 2, 2, 16]).unwrap().value.as_int().unwrap();
        assert!((1..=2).contains(&order), "message {m}: differencing order {order}");
        assert_eq!(ev.node(&d, &[m, 1, 4, 5, 2, 0]).unwrap().type_name, "bytes[]");
    }
    // And the three of them cover the file end to end.
    let last = ev.node(&d, &[2]).unwrap();
    assert_eq!(last.offset_bits + last.size_bits, d.len_bits());
}

#[test]
fn a_file_of_several_messages_reads_as_all_of_them() {
    let Some((d, mut ev)) = read("two-messages.grib2") else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let root = ev.node(&d, &[]).unwrap();
    assert_eq!(root.child_count, 2);
    for m in 0..2usize {
        assert_eq!(ev.node(&d, &[m]).unwrap().type_name, "Message");
        // Each ends where the next begins, with nothing between them.
        let message = ev.node(&d, &[m]).unwrap();
        let length = ev.node(&d, &[m, 1, 3]).unwrap().value.as_int().unwrap();
        assert_eq!(u64::from(message.size_bits), length as u64 * 8);
    }
}
