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
    roots.into_iter().map(|r| r.join("bufr").join(name)).find(|p| p.exists())
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
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
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
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
    }
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
