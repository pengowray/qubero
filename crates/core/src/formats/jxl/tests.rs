use super::*;
use crate::document::Document;
use crate::eval::{Evaluator, NodeInfo, Value};
use crate::source::MemSource;

fn be32(n: u32) -> [u8; 4] {
    n.to_be_bytes()
}

/// A box: its length, its letters and its contents.
fn jxl_box(kind: &[u8; 4], contents: &[u8]) -> Vec<u8> {
    let mut v = be32((contents.len() + 8) as u32).to_vec();
    v.extend_from_slice(kind);
    v.extend_from_slice(contents);
    v
}

/// The signature box and `ftyp`, which every container opens with.
fn opening() -> Vec<u8> {
    let mut v = jxl_box(b"JXL ", &[0x0d, 0x0a, 0x87, 0x0a]);
    v.extend(jxl_box(b"ftyp", &[b"jxl ".as_slice(), &be32(0), b"jxl "].concat()));
    v
}

/// The smallest codestream there is: ImageMagick's 8 by 8 picture with
/// alpha, 115 bytes, its one TOC entry covering everything after the eighth.
const SMALL: [u8; 115] = [
    0xff, 0x0a, 0x41, 0xc0, 0x4e, 0x01, 0xac, 0x01, 0x6b, 0x0b, 0x00, 0x00, 0x54, 0xa8, 0x8c, 0x32, 0x6e, 0xf0, 0x72, 0xae, 0xe7, 0xcb, 0x0f,
    0x1d, 0x16, 0xd3, 0x36, 0xae, 0x33, 0xb4, 0xd5, 0xb6, 0x85, 0x25, 0x89, 0x22, 0xd3, 0xcd, 0x8f, 0x28, 0x1d, 0xca, 0xe0, 0x21, 0x49, 0x2c,
    0x40, 0xe1, 0x6d, 0xa0, 0x26, 0xf0, 0x70, 0x00, 0x74, 0x12, 0x11, 0x41, 0x44, 0x14, 0x11, 0x44, 0x44, 0x14, 0x41, 0x44, 0x44, 0x14, 0x44,
    0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0xc4, 0x7c, 0x60, 0xef, 0x1c,
    0x9f, 0xb7, 0x95, 0x24, 0x12, 0x81, 0xed, 0xda, 0x4e, 0x49, 0x40, 0xde, 0xef, 0xff, 0x5d, 0x5b, 0x8a, 0x00, 0xe9, 0xc7, 0x00, 0x89, 0x32,
];

/// A little-endian TIFF of one directory holding one entry: orientation 1.
fn tiff() -> Vec<u8> {
    let mut v = b"II*\0".to_vec();
    v.extend_from_slice(&8u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&0x0112u16.to_le_bytes());
    v.extend_from_slice(&3u16.to_le_bytes());
    v.extend_from_slice(&1u32.to_le_bytes());
    v.extend_from_slice(&[1, 0, 0, 0]);
    v.extend_from_slice(&0u32.to_le_bytes());
    v
}

/// `<rdf:li>one</rdf:li><rdf:li>two</rdf:li>` twice, compressed with Brotli
/// by Node's `zlib.brotliCompressSync`.
const BROTLI_XMP: [u8; 38] = [
    0x1b, 0x4f, 0x00, 0xf8, 0x45, 0x77, 0x78, 0x9e, 0xfd, 0x99, 0xbe, 0xab, 0x22, 0xd2, 0x48, 0xd4, 0x8d, 0x14, 0xa3, 0x4d, 0x41, 0x7a, 0x81,
    0x0b, 0x46, 0xb8, 0xc1, 0x63, 0x3d, 0xb5, 0xcb, 0x38, 0x01, 0x01, 0x58, 0xf7, 0x4c, 0x0e,
];

/// The path to a node by the names of the fields on the way, where a name
/// that is a number is an element of a list.
fn at(ev: &mut Evaluator, d: &Document<MemSource>, steps: &[&str]) -> Vec<usize> {
    let mut path = Vec::new();
    for step in steps {
        if let Ok(i) = step.parse::<usize>() {
            path.push(i);
            continue;
        }
        let n = ev.node(d, &path).unwrap().child_count as usize;
        let i = (0..n)
            .find(|i| ev.node(d, &[&path[..], &[*i]].concat()).unwrap().name == *step)
            .unwrap_or_else(|| panic!("no {step} under {path:?}"));
        path.push(i);
    }
    path
}

fn node(ev: &mut Evaluator, d: &Document<MemSource>, steps: &[&str]) -> NodeInfo {
    let p = at(ev, d, steps);
    ev.node(d, &p).unwrap()
}

fn int(ev: &mut Evaluator, d: &Document<MemSource>, steps: &[&str]) -> i128 {
    let n = node(ev, d, steps);
    n.value.as_int().unwrap_or_else(|| panic!("{steps:?} holds {:?}", n.value))
}

fn text(ev: &mut Evaluator, d: &Document<MemSource>, steps: &[&str]) -> Value {
    node(ev, d, steps).value
}

#[test]
fn a_bare_codestream_is_its_signature_and_its_bytes() {
    let d = Document::new(MemSource(SMALL.to_vec()));
    let mut ev = Evaluator::new(jxl());
    let root = node(&mut ev, &d, &[]);
    assert_eq!(root.type_name, "Codestream");
    assert!(matches!(node(&mut ev, &d, &["signature"]).value, Value::Magic { ok: true, .. }));
    let data = node(&mut ev, &d, &["data"]);
    assert_eq!((data.offset_bits, data.size_bits), (16, 113 * 8));
}

/// A container with the codestream whole in `jxlc`, and one of every
/// metadata box: `jxll`, `Exif`, `xml `, a JUMBF superbox, a frame index,
/// and a `uuid` box whose length is in the eight bytes after its letters.
fn container() -> Vec<u8> {
    let mut v = opening();
    v.extend(jxl_box(b"jxll", &[10]));
    // Six bytes of `Exif\0\0` before the TIFF, which the offset skips.
    let mut exif = be32(6).to_vec();
    exif.extend_from_slice(b"Exif\0\0");
    exif.extend(tiff());
    v.extend(jxl_box(b"Exif", &exif));
    v.extend(jxl_box(b"xml ", b"<x/>"));
    let mut jumd = b"json".to_vec();
    jumd.extend_from_slice(&[0x00, 0x11, 0x00, 0x10, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71]);
    // Requestable, with a label and an ID.
    jumd.push(0b0111);
    jumd.extend_from_slice(b"meta\0");
    jumd.extend_from_slice(&be32(7));
    let jumbf = [jxl_box(b"jumd", &jumd), jxl_box(b"json", b"{}")].concat();
    v.extend(jxl_box(b"jumb", &jumbf));
    // Two frames a tick of 1/30 s apart, the second 300 bytes on, which
    // LEB128 writes in two bytes.
    let mut jxli = vec![2];
    jxli.extend_from_slice(&be32(1));
    jxli.extend_from_slice(&be32(30));
    jxli.extend_from_slice(&[0, 0, 1, 0xac, 0x02, 1, 1]);
    v.extend(jxl_box(b"jxli", &jxli));
    v.extend_from_slice(&be32(1));
    v.extend_from_slice(b"uuid");
    v.extend_from_slice(&(16u64 + 16 + 3).to_be_bytes());
    v.extend_from_slice(&[0x11; 16]);
    v.extend_from_slice(b"abc");
    v.extend(jxl_box(b"jxlc", &SMALL));
    v
}

#[test]
fn a_container_reads_as_its_boxes() {
    let bytes = container();
    let d = Document::new(MemSource(bytes.clone()));
    let mut ev = Evaluator::new(jxl());
    assert_eq!(node(&mut ev, &d, &[]).type_name, "Container");
    let boxes = node(&mut ev, &d, &["boxes"]);
    assert_eq!(boxes.child_count, 9);
    let kinds: Vec<Value> = (0..9).map(|i| text(&mut ev, &d, &["boxes", &i.to_string(), "TBox"])).collect();
    let want = ["JXL ", "ftyp", "jxll", "Exif", "xml ", "jumb", "jxli", "uuid", "jxlc"];
    assert_eq!(kinds, want.map(|k| Value::Str(k.into())));

    let b = |i: &'static str, steps: &[&'static str]| [&["boxes", i, "DBox"], steps].concat();
    assert!(matches!(text(&mut ev, &d, &b("0", &[])), Value::Magic { ok: true, .. }));
    assert_eq!(text(&mut ev, &d, &b("1", &["compatible_brands", "0"])), Value::Str("jxl ".into()));
    assert_eq!(text(&mut ev, &d, &b("2", &[])), Value::Enum { raw: 10, name: Some("level 10".into()), hex: false });

    // The Exif box's TIFF starts past the six bytes its offset names, and
    // reads as a TIFF does.
    assert_eq!(node(&mut ev, &d, &b("3", &["prefix"])).size_bits, 6 * 8);
    let tiff = node(&mut ev, &d, &b("3", &["tiff"]));
    assert_eq!(tiff.type_name, "TIFF");
    assert_eq!(int(&mut ev, &d, &b("3", &["tiff", "byte_order"])), 0x4949);

    assert_eq!(text(&mut ev, &d, &b("4", &[])), Value::Str("<x/>".into()));

    // The JUMBF superbox: its description, then the JSON it describes.
    assert_eq!(text(&mut ev, &d, &b("5", &["0", "DBox", "TYPE", "code"])), Value::Str("json".into()));
    assert_eq!(text(&mut ev, &d, &b("5", &["0", "DBox", "LABEL"])), Value::Str("meta".into()));
    assert_eq!(int(&mut ev, &d, &b("5", &["0", "DBox", "ID"])), 7);
    assert!(node(&mut ev, &d, &b("5", &["0", "DBox", "SIGNATURE"])).absent);
    assert_eq!(text(&mut ev, &d, &b("5", &["1", "DBox"])), Value::Str("{}".into()));

    // The frame index's offsets are LEB128.
    assert_eq!(int(&mut ev, &d, &b("6", &["NF"])), 2);
    assert_eq!(int(&mut ev, &d, &b("6", &["TDEN"])), 30);
    assert_eq!(int(&mut ev, &d, &b("6", &["frames", "1", "OFFi"])), 300);
    assert_eq!(int(&mut ev, &d, &b("6", &["frames", "1", "Fi"])), 1);

    // XLBox: the length is the eight bytes after the letters.
    assert_eq!(int(&mut ev, &d, &["boxes", "7", "LBox"]), 1);
    assert_eq!(node(&mut ev, &d, &b("7", &["DATA"])).size_bits, 3 * 8);

    // The codestream reads inside its box, and there are no parts to join.
    let codestream = node(&mut ev, &d, &b("8", &[]));
    assert_eq!(codestream.type_name, "Codestream");
    assert_eq!(codestream.offset_bits + codestream.size_bits, bytes.len() as u64 * 8);
    assert!(node(&mut ev, &d, &["codestream"]).absent);
}

#[test]
fn a_codestream_in_parts_is_joined_and_read_whole() {
    // Cut where no header ends: after three bytes, inside SizeHeader, and
    // again after the TOC.
    let mut v = opening();
    v.extend(jxl_box(b"jxlp", &[&be32(0)[..], &SMALL[..3]].concat()));
    v.extend(jxl_box(b"xml ", b"<between/>"));
    v.extend(jxl_box(b"jxlp", &[&be32(1)[..], &SMALL[3..8]].concat()));
    v.extend(jxl_box(b"jxlp", &[&be32(0x8000_0002)[..], &SMALL[8..]].concat()));
    let d = Document::new(MemSource(v));
    let mut ev = Evaluator::new(jxl());

    assert_eq!(int(&mut ev, &d, &["boxes", "2", "DBox", "number"]), 0);
    assert_eq!(int(&mut ev, &d, &["boxes", "2", "DBox", "last"]), 0);
    assert_eq!(node(&mut ev, &d, &["boxes", "2", "DBox", "part"]).size_bits, 3 * 8);
    assert_eq!(int(&mut ev, &d, &["boxes", "5", "DBox", "number"]), 2);
    assert_eq!(int(&mut ev, &d, &["boxes", "5", "DBox", "last"]), 1);

    let joined = node(&mut ev, &d, &["codestream", "codestream"]);
    assert_eq!((joined.type_name.as_str(), joined.size_bits), ("Codestream", 115 * 8));
    assert!(matches!(node(&mut ev, &d, &["codestream", "codestream", "signature"]).value, Value::Magic { ok: true, .. }));
    let data = node(&mut ev, &d, &["codestream", "codestream", "data"]);
    assert_eq!(data.size_bits, 113 * 8);
}

#[test]
fn a_brotli_box_reads_as_the_box_it_was() {
    let mut v = opening();
    v.extend(jxl_box(b"brob", &[b"xml ".as_slice(), &BROTLI_XMP].concat()));
    let mut exif = be32(0).to_vec();
    exif.extend(tiff());
    // Not compressed at all, but a box type `brob` may not hold reads as
    // bytes rather than as that box.
    v.extend(jxl_box(b"brob", &[b"jxlc".as_slice(), &[1, 2, 3]].concat()));
    let d = Document::new(MemSource(v));
    let mut ev = Evaluator::new(jxl());
    assert_eq!(text(&mut ev, &d, &["boxes", "2", "DBox", "original_type"]), Value::Str("xml ".into()));
    let xmp = "<rdf:li>one</rdf:li><rdf:li>two</rdf:li>".repeat(2);
    assert_eq!(text(&mut ev, &d, &["boxes", "2", "DBox", "contents", "contents"]), Value::Str(xmp));
    assert_eq!(node(&mut ev, &d, &["boxes", "2", "DBox", "contents"]).size_bits, BROTLI_XMP.len() as u64 * 8);
}

#[test]
fn a_box_running_to_the_end_of_the_file_takes_the_rest() {
    let mut v = opening();
    v.extend_from_slice(&be32(0));
    v.extend_from_slice(b"jxlc");
    v.extend_from_slice(&SMALL);
    let d = Document::new(MemSource(v.clone()));
    let mut ev = Evaluator::new(jxl());
    let codestream = node(&mut ev, &d, &["boxes", "2", "DBox"]);
    assert_eq!(codestream.offset_bits + codestream.size_bits, v.len() as u64 * 8);
    assert_eq!(node(&mut ev, &d, &["boxes", "2", "DBox", "data"]).size_bits, 113 * 8);
}
