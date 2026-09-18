//! Real BUFR files, read as far as their sections and, through the side
//! reader, their values.
//!
//! The section checks are the part the file decides: how many messages there
//! are and where, which edition each is, how many subsets and descriptors
//! section 3 says, and that every section ends where the next begins. The
//! value checks pin numbers ecCodes 2.48.0 reads from the same files.
//!
//! The files live in the sample collection rather than here. Point
//! `QUBERO_SAMPLES` at it, or keep it beside the repository as
//! `qubero-samples`. With neither, the test says so and passes.

use std::path::PathBuf;

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Explain, Value};
use qubero_core::formats;
use qubero_core::formats::bufr_data::{self, Item, Reading, Role};
use qubero_core::source::MemSource;

fn sample(name: &str) -> Option<PathBuf> {
    qubero_samples::roots().into_iter().map(|r| r.join("bufr").join(name)).find(|p| p.exists())
}

fn read(name: &str) -> Option<(Document<MemSource>, Evaluator)> {
    let path = sample(name)?;
    let doc = Document::new(MemSource(std::fs::read(&path).unwrap()));
    Some((doc, Evaluator::new(formats::builtin("bufr").unwrap())))
}

fn int(d: &Document<MemSource>, ev: &mut Evaluator, path: &[usize]) -> i128 {
    let n = ev.node(d, path).unwrap_or_else(|e| panic!("{path:?}: {e:?}"));
    n.value.as_int().unwrap_or_else(|| panic!("{path:?} is {:?}", n.value))
}

/// What section 0 and section 3 of one message say, and where it is.
#[derive(Debug, PartialEq)]
struct Shape {
    offset: u64,
    length: i128,
    edition: i128,
    subsets: i128,
    compressed: bool,
    descriptors: u64,
}

/// Every message in a file, and a check that each one's sections tile it: the
/// end marker's last byte is the last byte the total length gives.
fn shapes(d: &Document<MemSource>, ev: &mut Evaluator) -> Vec<Shape> {
    let mut out = Vec::new();
    for m in 0..ev.node(d, &[]).unwrap().child_count as usize {
        let node = ev.node(d, &[m]).unwrap();
        if node.type_name != "Message" {
            continue;
        }
        let length = int(d, ev, &[m, 1, 0]);
        let end = ev.node(d, &[m, 1, 2, 4]).unwrap();
        assert_eq!(end.offset_bits + end.size_bits, node.offset_bits + length as u64 * 8, "message {m}'s sections");
        assert_eq!(node.size_bits, length as u64 * 8);
        let Value::Flags { raw, .. } = ev.node(d, &[m, 1, 2, 2, 3]).unwrap().value else { panic!("section 3's flags") };
        out.push(Shape {
            offset: node.offset_bits / 8,
            length,
            edition: int(d, ev, &[m, 1, 1]),
            subsets: int(d, ev, &[m, 1, 2, 2, 2]),
            compressed: raw & 0x40 != 0,
            descriptors: ev.node(d, &[m, 1, 2, 2, 4]).unwrap().child_count,
        });
    }
    out
}

fn shape(offset: u64, length: i128, edition: i128, subsets: i128, compressed: bool, descriptors: u64) -> Shape {
    Shape { offset, length, edition, subsets, compressed, descriptors }
}

#[test]
fn four_synop_bulletins_read_as_four_messages_in_their_envelopes() {
    let Some((d, mut ev)) = read("ISMD01_OKPR.bufr") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    assert_eq!(
        shapes(&d, &mut ev),
        vec![
            shape(31, 692, 4, 7, true, 1),
            shape(758, 714, 4, 7, true, 1),
            shape(1507, 700, 4, 7, true, 1),
            shape(2242, 710, 4, 7, true, 1),
        ]
    );
    // The heading in front of the first, and the end of each bulletin with
    // the heading of the next between them.
    let kinds: Vec<String> = (0..9).map(|i| ev.node(&d, &[i]).unwrap().type_name).collect();
    assert_eq!(kinds[0], "GtsEnvelope");
    assert_eq!(kinds[1], "Message");
    assert_eq!(kinds[2], "GtsEnvelope");
    let Value::Str(heading) = ev.node(&d, &[0, 0]).unwrap().value else { panic!("the heading") };
    assert!(heading.contains("ISMD01 OKPR 211200"), "{heading:?}");
    // Prague, and the one descriptor is the synoptic report sequence.
    let centre = ev.node(&d, &[1, 1, 2, 0, 2]).unwrap();
    assert_eq!(centre.value, Value::Enum { raw: 89, name: Some("Prague".into()), hex: false });
    // 21 November 2007, 12:00, as ecCodes reads it.
    assert_eq!(int(&d, &mut ev, &[1, 1, 2, 0, 11]), 2007);
    assert_eq!(int(&d, &mut ev, &[1, 1, 2, 0, 12]), 11);
    assert_eq!(int(&d, &mut ev, &[1, 1, 2, 0, 14]), 12);
    let name = ev.node(&d, &[1, 1, 2, 2, 4, 0]).unwrap().name;
    assert!(name.starts_with("[0] 307080 "), "{name}");
}

#[test]
fn every_sample_tiles_its_messages_with_its_sections() {
    let want: &[(&str, Vec<Shape>)] = &[
        ("temp_101.bufr", vec![shape(0, 1470, 3, 1, false, 16)]),
        ("iasi_241.bufr", vec![shape(0, 11050, 3, 15, true, 49)]),
        ("jaso_214.bufr", vec![shape(0, 5004, 3, 128, true, 73)]),
        ("metar_with_2_bias.bufr", vec![shape(0, 180, 3, 1, false, 28)]),
        ("b002_95.bufr", vec![shape(0, 760, 3, 1, false, 61)]),
        ("207003.bufr", vec![shape(0, 244, 3, 2, true, 1)]),
        ("uegabe.bufr", vec![shape(0, 494, 4, 1, false, 7)]),
    ];
    let mut read_any = false;
    for (name, shape) in want {
        let Some((d, mut ev)) = read(name) else { continue };
        read_any = true;
        assert_eq!(&shapes(&d, &mut ev), shape, "{name}");
    }
    if !read_any {
        eprintln!("{}", qubero_samples::missing());
    }
}

/// The bytes of message `m` of a file, where the template put it, read by the
/// side reader.
fn reading(name: &str, m: usize) -> Option<Reading> {
    let (d, mut ev) = read(name)?;
    let messages: Vec<usize> =
        (0..ev.node(&d, &[]).unwrap().child_count as usize).filter(|i| ev.node(&d, &[*i]).unwrap().type_name == "Message").collect();
    let node = ev.node(&d, &[messages[m]]).unwrap();
    let bytes = std::fs::read(sample(name)?).unwrap();
    let from = (node.offset_bits / 8) as usize;
    Some(bufr_data::read(&bytes[from..from + (node.size_bits / 8) as usize]))
}

/// The values of one subset that are not operators, as (descriptor, text).
fn values(r: &Reading, subset: usize) -> Vec<(u32, String)> {
    let (list, at) = if r.header.compressed { (0, subset) } else { (subset, 0) };
    r.subsets[list]
        .iter()
        .filter(|i| i.role != Role::Operator)
        .map(|i| (i.code, if i.missing(at) { "MISSING".to_string() } else { i.text(at) }))
        .collect()
}

/// The `nth` value of an element in a subset, as text. Associated fields and
/// markers carry the descriptor of the element they are about and are not it,
/// so they are passed over.
fn find(r: &Reading, subset: usize, code: u32, nth: usize) -> String {
    let (list, at) = if r.header.compressed { (0, subset) } else { (subset, 0) };
    r.subsets[list]
        .iter()
        .filter(|i| i.code == code && matches!(i.role, Role::Element | Role::Count | Role::Local))
        .nth(nth)
        .map_or("(none)".to_string(), |i| if i.missing(at) { "MISSING".to_string() } else { i.text(at) })
}

#[test]
fn a_radiosonde_ascent_reads_as_ecmwf_reads_it() {
    let Some(r) = reading("temp_101.bufr", 0) else { return };
    assert_eq!(r.problem, None);
    assert_eq!(r.tables_version, 13);
    let v = values(&r, 0);
    // Every value, the 427 per cent confidences the bitmap attaches included,
    // and every one of them matches ecCodes 2.48.0 and pybufrkit 0.2.25.
    assert_eq!(v.len(), 1531);
    assert_eq!(v.iter().filter(|x| x.1 == "MISSING").count(), 123);
    assert_eq!(find(&r, 0, 1001, 0), "70");
    assert_eq!(find(&r, 0, 1002, 0), "219");
    assert_eq!(find(&r, 0, 5001, 0), "60.77000");
    assert_eq!(find(&r, 0, 6001, 0), "-161.83000");
    // 75 levels, the first at 1020 hPa and 272.1 K.
    assert_eq!(find(&r, 0, 31001, 0), "75");
    assert_eq!(find(&r, 0, 7004, 0), "102000");
    assert_eq!(find(&r, 0, 7004, 3), "98500");
    assert_eq!(find(&r, 0, 12001, 0), "272.1");
    assert_eq!(find(&r, 0, 12003, 0), "270.4");
    let quality: Vec<&Item> = r.subsets[0].iter().filter(|i| i.code == 33007).collect();
    assert_eq!(quality.len(), 427);
    // The first per cent confidence is the block number's.
    assert_eq!(r.subsets[0][quality[0].refers_to.unwrap()].code, 1001);
    assert_eq!(r.bits_read, 10055);
}

#[test]
fn four_compressed_synops_read_subset_by_subset() {
    let Some(r) = reading("ISMD01_OKPR.bufr", 0) else { return };
    assert_eq!(r.problem, None);
    assert_eq!(values(&r, 2).len(), 116);
    assert_eq!(find(&r, 2, 1001, 0), "11");
    assert_eq!(find(&r, 2, 1002, 0), "518");
    assert_eq!(find(&r, 2, 1015, 0), "Praha-Ruzyne");
    assert_eq!(find(&r, 2, 5001, 0), "50.10083");
    assert_eq!(find(&r, 2, 6001, 0), "14.25778");
    assert_eq!(find(&r, 2, 7030, 0), "364.0");
    assert_eq!(find(&r, 2, 10004, 0), "97130");
    let name = r.subsets[0].iter().find(|i| i.code == 1015).unwrap();
    assert_eq!(name.increment_width, Some(20), "twenty characters a subset");
    let Some(last) = reading("ISMD01_OKPR.bufr", 3) else { return };
    assert_eq!(last.value_count(), 840);
}

#[test]
fn width_scale_and_associated_field_operators_on_a_compressed_altimeter() {
    let Some(r) = reading("jaso_214.bufr", 0) else { return };
    assert_eq!(r.problem, None);
    assert_eq!(r.value_count(), 9600);
    // 0-07-005 under 2-02-131: three decimal places rather than zero.
    assert_eq!(find(&r, 5, 7005, 0), "0.282");
    assert_eq!(find(&r, 5, 21062, 0), "11.59");
    assert_eq!(find(&r, 5, 21062, 1), "0.06");
    let associated = r.subsets[0].iter().filter(|i| i.role == Role::Associated).count();
    assert_eq!(associated * 128, 1152);
}

#[test]
fn a_compressed_satellite_sounding_reads_every_subset() {
    let Some(r) = reading("iasi_241.bufr", 0) else { return };
    assert_eq!(r.problem, None);
    assert_eq!((r.header.subsets, r.value_count()), (15, 15_345));
    assert_eq!(find(&r, 14, 5001, 0), "57.57794");
    assert_eq!(find(&r, 14, 6001, 0), "154.06107");
    assert_eq!(find(&r, 14, 4006, 0), "6.943");
    assert_eq!(find(&r, 14, 14046, 0), "5067");
    assert_eq!(find(&r, 14, 14046, 2), "4471");
}

#[test]
fn difference_statistics_are_read_against_the_element_the_bitmap_names() {
    let Some(r) = reading("metar_with_2_bias.bufr", 0) else { return };
    assert_eq!(r.problem, None);
    let markers: Vec<&Item> = r.subsets[0].iter().filter(|i| i.role == Role::Marker).collect();
    assert_eq!(markers.len(), 2);
    assert!(markers[0].missing(0));
    assert_eq!(markers[1].text(0), "-100");
    // 0-10-004, pressure, and one bit wider than it with a negative reference.
    let target = &r.subsets[0][markers[1].refers_to.unwrap()];
    assert_eq!(target.code, 10004);
    assert_eq!(markers[1].width, target.width + 1);
    assert_eq!(r.subsets[0].iter().filter(|i| i.code == 33007).count(), 23);
}

#[test]
fn local_descriptors_read_at_the_width_206_gives_them() {
    let Some(r) = reading("b002_95.bufr", 0) else { return };
    assert_eq!(r.problem, None);
    let local: Vec<&Item> = r.subsets[0].iter().filter(|i| i.role == Role::Local).collect();
    assert_eq!(local.len(), 43);
    assert_eq!((local[0].code, local[0].width, local[0].text(0)), (21192, 8, "59".to_string()));
    assert_eq!(values(&r, 0).len(), 492);
}

#[test]
fn increase_scale_reference_and_width_on_a_compressed_message() {
    let Some(r) = reading("207003.bufr", 0) else { return };
    assert_eq!(r.problem, None);
    assert_eq!(r.tables_version, 15);
    assert_eq!(find(&r, 0, 4006, 0), "27.584");
    assert_eq!(find(&r, 1, 27031, 0), "6675220.00");
    assert_eq!(find(&r, 1, 28031, 0), "2628450.50");
}

#[test]
fn associated_fields_in_front_of_a_radiosonde_in_edition_4() {
    let Some(r) = reading("uegabe.bufr", 0) else { return };
    assert_eq!(r.problem, None);
    let v = values(&r, 0);
    assert_eq!(v.len(), 334);
    // The four bits in front of the block number are all ones, which in an
    // associated field is 15 and not a missing value: ecCodes reads it so.
    assert_eq!(v[1], (1001, "15".to_string()));
    assert_eq!(r.subsets[0][2].role, Role::Associated);
    assert_eq!(find(&r, 0, 1001, 0), "10");
}

/// What the inspector would show with the cursor `bit` bits into section 4's
/// data of message `m`, whose chunk is at `chunk` in the file's run.
fn panel_at(name: &str, chunk: usize, data_offset: usize, bit: u64) -> Option<bufr_data::Panel> {
    let (d, mut ev) = read(name)?;
    let message = ev.node(&d, &[chunk]).unwrap();
    let at = message.offset_bits + data_offset as u64 * 8 + bit;
    match ev.explain(&d, &[chunk, 1, 2, 3, 2], Some(at)).unwrap() {
        Explain::BufrData(p) => Some(*p),
        other => panic!("not the BUFR panel: {other:?}"),
    }
}

#[test]
fn the_panel_takes_apart_the_value_under_the_cursor() {
    let Some(r) = reading("temp_101.bufr", 0) else { return };
    let t = r.subsets[0].iter().find(|i| i.code == 12001).unwrap();
    let Some(p) = panel_at("temp_101.bufr", 0, r.header.data_offset, t.bit + 3) else { return };
    assert_eq!(p.problem, None);
    assert_eq!((p.edition, p.tables_version, p.subsets, p.compressed), (3, 13, 1, false));
    let c = p.cursor.expect("a value under the cursor");
    assert_eq!((c.value.code, c.value.text.as_str(), c.value.unit.as_str()), (12001, "272.1", "K"));
    // Counted from section 4's first byte, header and all.
    assert_eq!(c.bit, t.bit + 32);
    assert_eq!((c.width, c.scale, c.packed), (12, 1, Some(2721)));
    // And the row in the list is marked.
    assert_eq!(p.values[c.index.unwrap()].code, 12001);
    assert_eq!(p.values_total, 1531);
    assert!(p.steps.iter().any(|s| s.contains("bitmap of 550 bits")), "{:?}", p.steps);
    assert_eq!(p.descriptors[0].code, 309007);
}

#[test]
fn on_a_compressed_difference_the_panel_shows_that_subset() {
    let Some(r) = reading("ISMD01_OKPR.bufr", 0) else { return };
    let name = r.subsets[0].iter().find(|i| i.code == 1015).unwrap();
    // The station name's run: the smallest, 160 bits; the 6-bit width; then
    // twenty characters a subset. Two subsets in is the third station.
    let third = name.bit + 160 + 6 + 2 * 20 * 8 + 5;
    let Some(p) = panel_at("ISMD01_OKPR.bufr", 1, r.header.data_offset, third) else { return };
    assert_eq!(p.subset, 2);
    let c = p.cursor.expect("the station name");
    assert_eq!(c.value.text, "Praha-Ruzyne");
    assert_eq!((c.increment_width, c.across.len()), (Some(20), 7));
    assert_eq!(p.values.iter().find(|v| v.code == 1002).map(|v| v.text.as_str()), Some("518"));
}

#[test]
fn a_radiosonde_names_its_descriptors_from_the_tables() {
    let Some((d, mut ev)) = read("temp_101.bufr") else { return };
    let names: Vec<String> = (0..4).map(|i| ev.node(&d, &[0, 1, 2, 2, 4, i]).unwrap().name).collect();
    assert!(names[0].starts_with("[0] 309007 "), "{names:?}");
    assert_eq!(names[1], "[1] Delayed replication of 4 descriptors");
    assert_eq!(names[2], "[2] 031001 Delayed descriptor replication factor");
    assert_eq!(names[3], "[3] 007004 Pressure");
    // ECMWF, version 13 of the master table, and section 2 is there.
    assert_eq!(int(&d, &mut ev, &[0, 1, 2, 0, 3]), 98);
    assert_eq!(int(&d, &mut ev, &[0, 1, 2, 0, 8]), 13);
    assert_eq!(ev.node(&d, &[0, 1, 2, 1]).unwrap().type_name, "LocalUse");
}
