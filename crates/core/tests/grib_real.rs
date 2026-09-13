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
        // Section 7 says which codestream it holds and stops there: there is
        // no JPEG 2000 template here to open it with.
        assert_eq!(ev.node(&d, &[0, 1, 4, 5, 2]).unwrap().type_name, "Jpeg2000PackedData");
        let data = ev.node(&d, &[0, 1, 4, 5, 2, 0]).unwrap();
        assert_eq!((data.name.as_str(), data.type_name.as_str()), ("codestream", "bytes[]"));
        // And it covers the whole of the section after the header.
        let section = ev.node(&d, &[0, 1, 4, 5]).unwrap();
        assert_eq!(data.offset_bits + data.size_bits, section.offset_bits + section.size_bits);
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
        // Complex packing with spatial differencing: the header reads, and so
        // does section 7, group by group.
        let packing = ev.node(&d, &[m, 1, 4, 3, 2, 2]).unwrap();
        assert_eq!(packing.type_name, "ComplexPackingSpatial", "message {m}");
        let groups = ev.node(&d, &[m, 1, 4, 3, 2, 2, 9]).unwrap().value.as_int().unwrap();
        assert!(groups > 0, "message {m}: {groups} groups");
        let order = ev.node(&d, &[m, 1, 4, 3, 2, 2, 16]).unwrap().value.as_int().unwrap();
        assert!((1..=2).contains(&order), "message {m}: differencing order {order}");
        let body = ev.node(&d, &[m, 1, 4, 5, 2]).unwrap();
        assert_eq!(body.type_name, "ComplexPackedData", "message {m}");
        // Every group the header counted is there, and the lengths in the
        // table add up to the number of points on the grid.
        let list = ev.node(&d, &[m, 1, 4, 5, 2, 18]).unwrap();
        assert_eq!(i128::from(list.child_count), groups, "message {m}");
        let mut total = 0i128;
        for g in 0..groups as usize {
            let width = ev.node(&d, &[m, 1, 4, 5, 2, 18, g, 0]).unwrap().value.as_int().unwrap();
            let count = ev.node(&d, &[m, 1, 4, 5, 2, 18, g, 1]).unwrap().value.as_int().unwrap();
            let values = ev.node(&d, &[m, 1, 4, 5, 2, 18, g, 2]).unwrap();
            assert_eq!(i128::from(values.child_count), count, "message {m} group {g}");
            assert_eq!(u64::from(values.size_bits), (count * width) as u64, "message {m} group {g}");
            total += count;
        }
        assert_eq!(total, ni * nj, "message {m}");
        // And the groups end inside the section rather than past it.
        let section = ev.node(&d, &[m, 1, 4, 5]).unwrap();
        assert!(list.offset_bits + list.size_bits <= section.offset_bits + section.size_bits, "message {m}");
    }
    // And the three of them cover the file end to end.
    let last = ev.node(&d, &[2]).unwrap();
    assert_eq!(last.offset_bits + last.size_bits, d.len_bits());
}

/// Complex packing without spatial differencing, which is the plainer half of
/// what section 7 now reads. The numbers checked here came out of ecCodes
/// 2.48, which wrote the file: eleven groups, the widths and lengths it chose,
/// and the packed number the first point turns out to be.
#[test]
fn complex_packing_reads_as_groups_of_different_widths() {
    let Some((d, mut ev)) = read("regular_ll_complex.grib2") else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    assert_eq!(ev.node(&d, &[0, 1, 4, 3, 2, 2]).unwrap().type_name, "ComplexPacking");
    let body = ev.node(&d, &[0, 1, 4, 5, 2]).unwrap();
    assert_eq!(body.type_name, "ComplexPackedData");
    // The three tables, each with one entry per group.
    for table in [8usize, 10, 12] {
        assert_eq!(ev.node(&d, &[0, 1, 4, 5, 2, table]).unwrap().child_count, 11, "table {table}");
    }
    // The widths ecCodes chose, straight out of the table and with the
    // reference added. One group is zero bits wide: every value in it is the
    // group's reference and nothing is written for them at all.
    let widths: Vec<i128> =
        (0..11).map(|g| ev.node(&d, &[0, 1, 4, 5, 2, 14, g, 0]).unwrap().value.as_int().unwrap()).collect();
    assert_eq!(widths, vec![11, 0, 9, 9, 7, 10, 11, 10, 8, 10, 11]);
    let counts: Vec<i128> =
        (0..11).map(|g| ev.node(&d, &[0, 1, 4, 5, 2, 14, g, 1]).unwrap().value.as_int().unwrap()).collect();
    assert_eq!(counts, vec![40, 50, 6, 25, 26, 71, 86, 64, 43, 54, 31]);
    // The last of those is not in the table: it is `last_group_length` from
    // section 5, which is the one group the scaled form could not hold.
    let last = ev.node(&d, &[0, 1, 4, 3, 2, 2, 14]).unwrap().value.as_int().unwrap();
    assert_eq!(last, 31);
    assert_eq!(counts.iter().sum::<i128>(), 16 * 31);
    // The zero-width group takes no room and still has all its values.
    let flat = ev.node(&d, &[0, 1, 4, 5, 2, 14, 1, 2]).unwrap();
    assert_eq!((flat.child_count, flat.size_bits), (50, 0));
    // The first point, as the file holds it: group 0's reference plus the
    // first eleven bits under it. (253.02 + 639 * 2^-5) is 272.9888, which is
    // what ecCodes reads for it.
    let reference = ev.node(&d, &[0, 1, 4, 5, 2, 8, 0]).unwrap().value.as_int().unwrap();
    let first = ev.node(&d, &[0, 1, 4, 5, 2, 14, 0, 2, 0]).unwrap().value.as_int().unwrap();
    assert_eq!((reference, first), (0, 639));
}

/// Section 5's numbers and section 7's bytes, gathered from the tree the way a
/// panel would, so the side reader can be asked what the message holds.
fn packing_and_data(name: &str, message: usize) -> Option<(qubero_core::formats::grib_values::Packing, Vec<u8>)> {
    let path = sample(name)?;
    let raw = std::fs::read(&path).unwrap();
    let d = Document::new(MemSource(raw.clone()));
    let mut ev = Evaluator::new(formats::builtin("grib").unwrap());
    let head = [message, 1, 4, 5, 2];
    let body = ev.node(&d, &head).ok()?;
    let spatial = body.child_count == 19;
    let at = |ev: &mut Evaluator, i: usize| -> i64 {
        let mut p = head.to_vec();
        p.push(i);
        ev.node(&d, &p).unwrap().value.as_int().unwrap() as i64
    };
    // Section 5's reference value and its two scale factors, which section 7
    // does not copy in: nothing it places depends on them.
    let five = [message, 1, 4, 3, 2, 2];
    let float = |ev: &mut Evaluator, i: usize| -> f32 {
        let mut p = five.to_vec();
        p.push(i);
        match ev.node(&d, &p).unwrap().value {
            qubero_core::eval::Value::Float(f) => f as f32,
            other => panic!("reference value is {other:?}"),
        }
    };
    let signed = |ev: &mut Evaluator, i: usize| -> i32 {
        let mut p = five.to_vec();
        p.push(i);
        ev.node(&d, &p).unwrap().value.as_int().unwrap() as i32
    };
    let p = qubero_core::formats::grib_values::Packing {
        reference: float(&mut ev, 0),
        binary_scale: signed(&mut ev, 1),
        decimal_scale: signed(&mut ev, 2),
        bits_per_value: at(&mut ev, 0) as u32,
        // Section 7 has no use for this one, so it comes from section 5 with
        // the reference value and the scale factors.
        missing_value_management: signed(&mut ev, 6) as u32,
        n_groups: at(&mut ev, 1) as u32,
        group_widths_reference: at(&mut ev, 2) as u32,
        group_widths_bits: at(&mut ev, 3) as u32,
        group_lengths_reference: at(&mut ev, 4) as u32,
        group_length_increment: at(&mut ev, 5) as u32,
        last_group_length: at(&mut ev, 6) as u32,
        group_lengths_bits: at(&mut ev, 7) as u32,
        spatial_order: if spatial { at(&mut ev, 8) as u32 } else { 0 },
        extra_bytes: if spatial { at(&mut ev, 9) as u32 } else { 0 },
    };
    let from = (body.offset_bits / 8) as usize;
    let to = from + (body.size_bits / 8) as usize;
    Some((p, raw[from..to].to_vec()))
}

/// What the numbers are worth, against ecCodes 2.48 reading the same files.
/// The template says where every packed number is; this is the other half,
/// and the numbers on the right of these assertions came out of ecCodes.
#[test]
fn the_values_a_complex_packed_message_stands_for_match_another_reader() {
    use qubero_core::formats::grib_values::complex;
    let Some((p, bytes)) = packing_and_data("regular_ll_complex.grib2", 0) else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    // Complex packing without differencing: a group reference and a scaling,
    // and nothing else.
    let r = complex(&p, &bytes);
    assert_eq!(r.problem, None);
    assert_eq!(r.values.len(), 16 * 31);
    let near = |a: f64, b: f64| assert!((a - b).abs() < 5e-4, "{a} is not {b}");
    near(r.values[0], 272.9888);
    near(r.values[1], 279.5513);
    near(r.values[2], 285.3638);
    near(*r.values.last().unwrap(), 254.5825);

    // And the operational one, which is second-order spatial differencing on
    // top of that: the running sum over 65,160 points has to land on the same
    // number ecCodes lands on, at both ends.
    let Some((p, bytes)) = packing_and_data("gfs-1p00-3messages.grib2", 0) else { return };
    assert_eq!((p.spatial_order, p.extra_bytes), (2, 2));
    let r = complex(&p, &bytes);
    assert_eq!(r.problem, None);
    assert_eq!(r.values.len(), 360 * 181);
    near(r.values[0], 101124.03125);
    near(*r.values.last().unwrap(), 104141.83125);
    let lo = r.values.iter().cloned().fold(f64::INFINITY, f64::min);
    let hi = r.values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    near(lo, 94732.43125);
    near(hi, 107593.43125);

    // The third message is the other end of the range: a decimal scale of 9,
    // so every value is a millionth of what the sum came to.
    let Some((p, bytes)) = packing_and_data("gfs-1p00-3messages.grib2", 2) else { return };
    let r = complex(&p, &bytes);
    assert_eq!(r.problem, None);
    let tiny = |a: f64, b: f64| assert!((a - b).abs() < 1e-13, "{a} is not {b}");
    tiny(r.values[0], 8.52e-07);
    tiny(*r.values.last().unwrap(), 4.0e-09);

    // Every step is named, in the order it was done, and the last is the
    // scaling that turns a packed number into a measurement.
    let steps: Vec<&str> = r.steps.iter().map(|s| s.what.as_str()).collect();
    assert_eq!(steps.len(), 8, "{steps:?}");
    assert!(steps.last().unwrap().contains("10^9"), "{:?}", steps.last());
}

/// A section 7 that holds a PNG opens as one, the way a stored ZIP member
/// opens as whatever it holds.
#[test]
fn a_png_packed_section_opens_as_a_png() {
    let Some((d, mut ev)) = read("regular_ll_png.grib2") else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    assert_eq!(ev.node(&d, &[0, 1, 4, 3, 2, 2]).unwrap().type_name, "PngPacking");
    let body = ev.node(&d, &[0, 1, 4, 5, 2]).unwrap();
    assert_eq!(body.type_name, "PngPackedData");
    // The run itself, and then the space a reading opens over it.
    let at = [0, 1, 4, 5, 2, 0];
    let run = ev.node(&d, &at).unwrap();
    assert!(run.size_bits > 0, "an empty PNG section");
    let id = ev.open_space(&d, 0, &at).expect("resolves").expect("the codestream opens");
    let space = ev.space(id).expect("just opened");
    assert_eq!(space.template, "png", "section 7 opened as {:?}", space.template);
    assert_eq!(space.len_bytes() as u64, u64::from(run.size_bits) / 8);
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
