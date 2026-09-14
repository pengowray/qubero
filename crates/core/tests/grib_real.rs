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
    let values = ev.node(&d, &[0, 1, 4, 5, 2, 5]).unwrap();
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
    // which reads, and the packing is 5.40. The field is a constant, zero
    // bits a value, so ECMWF's template writes no codestream at all and
    // section 7 is its five-byte header and nothing else.
    if let Some((d, mut ev)) = read("reduced_gg_sfc_jpeg.grib2") {
        assert_eq!(ev.node(&d, &[0, 1, 4, 1, 2, 5]).unwrap().type_name, "GaussianGrid");
        assert_eq!(ev.node(&d, &[0, 1, 4, 3, 2, 2]).unwrap().type_name, "Jpeg2000Packing");
        assert_eq!(ev.node(&d, &[0, 1, 4, 3, 2, 2, 3]).unwrap().value.as_int(), Some(0));
        assert_eq!(ev.node(&d, &[0, 1, 4, 5, 2]).unwrap().type_name, "Jpeg2000PackedData");
        let data = ev.node(&d, &[0, 1, 4, 5, 2, 0]).unwrap();
        assert_eq!((data.name.as_str(), data.size_bits), ("codestream", 0));
        // And it covers the whole of the section after the header, which is
        // none of it.
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
    assert_eq!(steps.len(), 9, "{steps:?}");
    assert!(steps.last().unwrap().contains("10^9"), "{:?}", steps.last());
}

/// The panel's answer for a cursor on a packed value, however far under
/// section 7's data it is: which value of the message it is, what it is worth,
/// and the whole packed integer, against the same ecCodes numbers.
#[test]
fn a_cursor_on_a_packed_value_is_told_what_that_value_is_worth() {
    use qubero_core::eval::{Explain, GribPlace};
    let Some((d, mut ev)) = read("regular_ll_complex.grib2") else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let near = |text: &str, b: f64| {
        let a: f64 = text.parse().unwrap();
        assert!((a - b).abs() < 5e-4, "{a} is not {b}");
    };
    // Four levels down: the groups, group 2, its values, its first. Groups 0
    // and 1 hold 40 and 50, so this is value 90 of the message.
    match ev.explain(&d, &[0, 1, 4, 5, 2, 14, 2, 2, 0], None).unwrap() {
        Explain::GribValues { template, declared, total, values, at, problem, steps, .. } => {
            assert_eq!(problem, None);
            assert_eq!((template, declared, total), (2, 16 * 31, 16 * 31));
            near(&values[0], 272.9888);
            near(&values[2], 285.3638);
            let at = at.expect("the cursor is on a value");
            assert_eq!(at.index, 90);
            assert!(matches!(at.place, GribPlace::Group { group: 2, position: 0, .. }), "{:?}", at.place);
            assert_eq!(steps.last().unwrap().label, "scaling");
        }
        other => panic!("{other:?}"),
    }
    // The first value, whose field holds 639 above a group reference of 0.
    match ev.explain(&d, &[0, 1, 4, 5, 2, 14, 0, 2, 0], None).unwrap() {
        Explain::GribValues { at: Some(at), .. } => {
            assert_eq!((at.index, at.packed), (0, 639));
            assert_eq!(at.place, GribPlace::Group { group: 0, position: 0, written: 639 });
            near(&at.value, 272.9888);
        }
        other => panic!("{other:?}"),
    }
    // On a table rather than a value: the section's answer, and no value.
    match ev.explain(&d, &[0, 1, 4, 5, 2, 10, 3], None).unwrap() {
        Explain::GribValues { at, .. } => assert_eq!(at, None),
        other => panic!("{other:?}"),
    }

    // Simple packing, two levels down: the value is where it is in the run.
    let Some((d, mut ev)) = read("regular_ll_sfc.grib2") else { return };
    match ev.explain(&d, &[0, 1, 4, 5, 2, 5, 5, 0], None).unwrap() {
        Explain::GribValues { template, at: Some(at), total, problem, .. } => {
            assert_eq!(problem, None);
            assert_eq!((template, total), (0, 16 * 31));
            assert_eq!((at.index, at.place), (5, GribPlace::Values));
        }
        other => panic!("{other:?}"),
    }

    // Second-order spatial differencing: the first values are the message's
    // first, and the last value lands where ecCodes lands.
    let Some((d, mut ev)) = read("gfs-1p00-3messages.grib2") else { return };
    let body = [0, 1, 4, 5, 2];
    assert_eq!(ev.node(&d, &body).unwrap().type_name, "ComplexPackedData");
    let first = ev.child_named(&d, &body, "first_values").unwrap().expect("5.3 has first values");
    let mut second = first.clone();
    second.push(1);
    match ev.explain(&d, &second, None).unwrap() {
        Explain::GribValues { template, spatial_order, minimum, at: Some(at), total, problem, values, .. } => {
            assert_eq!(problem, None);
            assert_eq!((template, spatial_order, total), (3, 2, 360 * 181));
            assert!(minimum.is_some());
            assert_eq!((at.index, at.place), (1, GribPlace::First));
            // Written to hundredths: the reference is 947324.3 over a decimal
            // scale of 1, and ecCodes' 101124.03125 is those four bytes read
            // exactly.
            assert_eq!(values[0], "101124.03");
        }
        other => panic!("{other:?}"),
    }
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

/// A section 7 that holds a JPEG 2000 codestream opens as one, and the image
/// that codestream describes is the grid: as many samples as section 5 packed,
/// each as deep as section 5 said.
///
/// The file is ECMWF's edition 2 sample repacked by ecCodes as `grid_jpeg`,
/// which hands the grid to OpenJPEG: a 16 by 31 latitude/longitude grid of
/// 496 points, every one with a value, at 12 bits a value.
#[test]
fn a_jpeg2000_packed_section_opens_as_a_codestream_the_size_of_the_grid() {
    let Some((d, mut ev)) = read("regular_ll_jpeg.grib2") else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let sections = sections(&d, &mut ev);
    let path = |n: i128| sections.iter().find(|(m, _)| *m == n).map(|(_, p)| p.clone()).unwrap();
    let int = |ev: &mut Evaluator, p: &[usize]| ev.node(&d, p).unwrap().value.as_int().unwrap();
    // Section 3's grid, and section 5's count and width.
    let grid = [&path(3)[..], &[2, 5]].concat();
    let (ni, nj) = (int(&mut ev, &[&grid[..], &[7]].concat()), int(&mut ev, &[&grid[..], &[8]].concat()));
    let packing = [&path(5)[..], &[2]].concat();
    assert_eq!(ev.node(&d, &[&packing[..], &[2]].concat()).unwrap().type_name, "Jpeg2000Packing");
    let count = int(&mut ev, &[&packing[..], &[0]].concat());
    let bits = int(&mut ev, &[&packing[..], &[2, 3]].concat());
    assert_eq!((ni, nj, count, bits), (16, 31, 496, 12));

    // Section 7 opens as a JPEG 2000 codestream.
    let at = [&path(7)[..], &[2, 0]].concat();
    let run = ev.node(&d, &at).unwrap();
    let id = ev.open_space(&d, 0, &at).expect("resolves").expect("the codestream opens");
    let space = ev.space(id).expect("just opened");
    assert_eq!(space.template, "jpeg2000", "section 7 opened as {:?}", space.template);
    assert_eq!(space.len_bytes(), u64::from(run.size_bits) / 8);

    // And read with that template, its SIZ is the grid.
    let codestream = Document::new(MemSource(space.bytes().to_vec()));
    let mut j2k = Evaluator::new(formats::builtin("jpeg2000").unwrap());
    let siz = [1, 0, 1];
    assert_eq!(j2k.node(&codestream, &[1, 0, 0]).unwrap().value.as_int(), Some(0xff51));
    let field = |j2k: &mut Evaluator, name: &str| {
        let n = j2k.node(&codestream, &siz).unwrap().child_count as usize;
        let i = (0..n).find(|i| j2k.node(&codestream, &[&siz[..], &[*i]].concat()).unwrap().name == name).unwrap();
        j2k.node(&codestream, &[&siz[..], &[i]].concat()).unwrap().value.as_int().unwrap()
    };
    let (width, height) = (field(&mut j2k, "width"), field(&mut j2k, "height"));
    assert_eq!((width, height), (ni, nj), "the image is the grid's shape");
    assert_eq!(width * height, count);
    assert_eq!(field(&mut j2k, "Csiz"), 1);
    // One component, whose depth is section 5's bits a value.
    let component = [&siz[..], &[11, 0]].concat();
    let depth = j2k.node(&codestream, &[&component[..], &[4]].concat()).unwrap();
    assert_eq!(depth.name, "depth");
    assert_eq!(depth.value.as_int(), Some(bits));
    assert_eq!(j2k.node(&codestream, &[&component[..], &[3]].concat()).unwrap().value.as_int(), Some(0), "unsigned");
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

/// Every simply packed value in the collection, each one's worth as the
/// template works it out against the same value from `grib_values`, the side
/// reader the panel shows. The two are separate arithmetic over the same
/// numbers: the template's `(reference_value + stored * pow2(E)) / pow10(D)`
/// over its own fields, and the reader's over the bytes of section 7 and a
/// reference value read here straight from section 5's four bytes.
///
/// The template reads the reference value as the float its row shows, the
/// shortest decimal that reads back as the same bits, and the side reader
/// reads the bits exactly, so the two meet within a ten-millionth of the value
/// rather than at the bit.
#[test]
fn a_simply_packed_value_agrees_with_the_panel() {
    use qubero_core::eval::Value;
    use qubero_core::formats::grib_values::{simple, Packing};
    let mut checked = 0usize;
    for name in ["regular_ll_sfc.grib2", "two-messages.grib2", "lambert_bf.grib2", "gfs-1p00-3messages.grib2"] {
        let Some((d, mut ev)) = read(name) else {
            eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
            return;
        };
        let raw = std::fs::read(sample(name).unwrap()).unwrap();
        let found = sections(&d, &mut ev);
        let mut five: Option<Vec<usize>> = None;
        for (number, path) in found {
            if number == 5 {
                five = Some(path);
                continue;
            }
            if number != 7 {
                continue;
            }
            // Simple packing's section 7, and not the one of bytes that a
            // packing this does not read leaves behind under the same name.
            let body = [path.clone(), vec![2]].concat();
            if ev.node(&d, &body).unwrap().type_name != "PackedData" || ev.child_named(&d, &body, "reference_value").unwrap().is_none() {
                continue;
            }
            let template = [five.clone().expect("a section 5 before section 7"), vec![2, 2]].concat();
            let field = |ev: &mut Evaluator, of: &[usize], name: &str| {
                let p = ev.child_named(&d, of, name).unwrap().unwrap_or_else(|| panic!("no {name}"));
                ev.node(&d, &p).unwrap()
            };
            let reference_at = (field(&mut ev, &template, "reference_value").offset_bits / 8) as usize;
            let reference = f32::from_be_bytes(raw[reference_at..reference_at + 4].try_into().unwrap());
            let p = Packing {
                reference,
                binary_scale: field(&mut ev, &template, "binary_scale_factor").value.as_int().unwrap() as i32,
                decimal_scale: field(&mut ev, &template, "decimal_scale_factor").value.as_int().unwrap() as i32,
                bits_per_value: field(&mut ev, &template, "bits_per_value").value.as_int().unwrap() as u32,
                missing_value_management: 0,
                n_groups: 0,
                group_widths_reference: 0,
                group_widths_bits: 0,
                group_lengths_reference: 0,
                group_length_increment: 0,
                last_group_length: 0,
                group_lengths_bits: 0,
                spatial_order: 0,
                extra_bytes: 0,
            };
            let node = ev.node(&d, &body).unwrap();
            let count = field(&mut ev, &body, "count").value.as_int().unwrap() as usize;
            let from = (node.offset_bits / 8) as usize;
            let reading = simple(&p, &raw[from..from + (node.size_bits / 8) as usize], count);
            assert_eq!(reading.problem, None, "{name}");
            assert_eq!(reading.values.len(), count, "{name}");
            let values = ev.child_named(&d, &body, "values").unwrap().unwrap();
            for (i, want) in reading.values.iter().enumerate() {
                let worth = ev.node(&d, &[values.clone(), vec![i, 1]].concat()).unwrap();
                let Value::Float(got) = worth.value else { panic!("{name} value {i} is worth {:?}", worth.value) };
                assert!((got - want).abs() <= 1e-7 * want.abs().max(1.0), "{name} value {i}: {got} against the panel's {want}");
                checked += 1;
            }
        }
    }
    eprintln!("{checked} simply packed values checked");
    assert!(checked > 0, "no simply packed value in the collection");
}

/// Every simply packed message in the collection is a field of one value,
/// written in no bits, so the test above never sees a packed number that is
/// not nought or a binary scale that does anything. This one makes a message
/// that does, out of a real one: `regular_ll_complex.grib2` unpacked by the
/// side reader, whose values ecCodes agrees with, and packed again as simple
/// packing with the same reference value and scale factors, every packed
/// number whole and as many bits wide as the widest of them needs. The
/// template then has to land on the values ecCodes read from the original.
#[test]
fn a_real_field_packed_again_simply_is_worth_what_eccodes_read() {
    use qubero_core::eval::Value;
    use qubero_core::formats::grib_values::{complex, simple};
    let Some((p, bytes)) = packing_and_data("regular_ll_complex.grib2", 0) else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let original = complex(&p, &bytes);
    assert_eq!(original.problem, None);
    let raw = std::fs::read(sample("regular_ll_complex.grib2").unwrap()).unwrap();
    let (d, mut ev) = read("regular_ll_complex.grib2").unwrap();
    let widest = original.packed.iter().copied().max().unwrap() as u64;
    let bits = (64 - widest.leading_zeros()).max(1);
    let count = original.packed.len();

    // Section 7: the packed numbers, most significant bit first.
    let mut packed = vec![0u8; (count * bits as usize).div_ceil(8)];
    for (i, x) in original.packed.iter().enumerate() {
        for b in 0..bits as usize {
            if (*x as u64) >> (bits as usize - 1 - b) & 1 == 1 {
                let at = i * bits as usize + b;
                packed[at / 8] |= 0x80 >> (at % 8);
            }
        }
    }
    let section = |number: u8, body: &[u8]| {
        let mut s = ((body.len() + 5) as u32).to_be_bytes().to_vec();
        s.push(number);
        s.extend_from_slice(body);
        s
    };
    let mut message = Vec::new();
    for (number, path) in sections(&d, &mut ev) {
        let node = ev.node(&d, &path).unwrap();
        let (from, len) = ((node.offset_bits / 8) as usize, (node.size_bits / 8) as usize);
        assert_eq!(u32::from_be_bytes(raw[from..from + 4].try_into().unwrap()) as usize, len, "section {number} is its whole length");
        match number {
            // Template 5.0 with the original's reference value and scale
            // factors, byte for byte, and the new width.
            5 => {
                let template = [path.clone(), vec![2, 2]].concat();
                let at = |ev: &mut Evaluator, name: &str| {
                    let p = ev.child_named(&d, &template, name).unwrap().unwrap();
                    (ev.node(&d, &p).unwrap().offset_bits / 8) as usize
                };
                let reference = at(&mut ev, "reference_value");
                let mut body = (count as u32).to_be_bytes().to_vec();
                body.extend_from_slice(&0u16.to_be_bytes());
                body.extend_from_slice(&raw[reference..reference + 8]);
                body.push(bits as u8);
                body.push(0);
                message.extend(section(5, &body));
            }
            7 => message.extend(section(7, &packed)),
            _ => message.extend_from_slice(&raw[from..from + len]),
        }
    }
    message.extend_from_slice(b"7777");
    let mut file = raw[..8].to_vec();
    file.extend_from_slice(&((message.len() + 16) as u64).to_be_bytes());
    file.extend(message);

    let d = Document::new(MemSource(file));
    let mut ev = Evaluator::new(formats::builtin("grib").unwrap());
    let found = sections(&d, &mut ev);
    let seven = found.iter().find(|(n, _)| *n == 7).unwrap().1.clone();
    let body = [seven, vec![2]].concat();
    assert_eq!(ev.node(&d, &body).unwrap().type_name, "PackedData");
    let values = ev.child_named(&d, &body, "values").unwrap().unwrap();
    let list = ev.node(&d, &values).unwrap();
    assert_eq!((list.type_name.as_str(), list.child_count), ("Packed[]", count as u64));
    // The side reader over the new bytes says what the original said, which
    // is what makes this a message about the same field.
    let again = simple(&qubero_core::formats::grib_values::Packing { bits_per_value: bits, n_groups: 0, spatial_order: 0, ..p.clone() }, &packed, count);
    assert_eq!(again.values, original.values);
    for (i, want) in original.values.iter().enumerate() {
        let stored = ev.node(&d, &[values.clone(), vec![i, 0]].concat()).unwrap().value;
        assert_eq!(stored.as_int(), Some(i128::from(original.packed[i])), "value {i}");
        let Value::Float(got) = ev.node(&d, &[values.clone(), vec![i, 1]].concat()).unwrap().value else { panic!("value {i}") };
        assert!((got - want).abs() <= 1e-7 * want.abs().max(1.0), "value {i}: {got} against the panel's {want}");
    }
    // And the numbers ecCodes 2.48 gave for the original file.
    let worth = |ev: &mut Evaluator, i: usize| match ev.node(&d, &[values.clone(), vec![i, 1]].concat()).unwrap().value {
        Value::Float(f) => f,
        other => panic!("{other:?}"),
    };
    for (i, eccodes) in [(0, 272.9888), (1, 279.5513), (2, 285.3638), (count - 1, 254.5825)] {
        let got = worth(&mut ev, i);
        assert!((got - eccodes).abs() < 5e-4, "value {i}: {got} against ecCodes' {eccodes}");
    }
    // What a reader is shown for the first: a binary scale that does something.
    let rel = ev.relations(&d, &[values.clone(), vec![0, 1]].concat()).unwrap();
    assert_eq!(rel[0].substituted, "(253.02 + 639 * pow2(-5)) / pow10(0)");
}
