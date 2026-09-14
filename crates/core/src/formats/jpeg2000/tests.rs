use super::*;
use crate::document::Document;
use crate::eval::{Evaluator, NodeInfo, Value};
use crate::source::MemSource;

/// A marker segment: the marker, a length that counts itself, the parameters.
fn seg(marker: u16, params: &[u8]) -> Vec<u8> {
    let mut v = marker.to_be_bytes().to_vec();
    v.extend_from_slice(&((params.len() + 2) as u16).to_be_bytes());
    v.extend_from_slice(params);
    v
}

fn be16(n: u16) -> [u8; 2] {
    n.to_be_bytes()
}

fn be32(n: u32) -> [u8; 4] {
    n.to_be_bytes()
}

/// SIZ for an image `width` by `height` in tiles `tile_w` by `tile_h`, with a
/// component for each `(Ssiz, XRsiz, YRsiz)` given.
fn siz(width: u32, height: u32, tile_w: u32, tile_h: u32, components: &[(u8, u8, u8)]) -> Vec<u8> {
    let mut p = be16(0).to_vec();
    for n in [width, height, 0, 0, tile_w, tile_h, 0, 0] {
        p.extend_from_slice(&be32(n));
    }
    p.extend_from_slice(&be16(components.len() as u16));
    for (s, x, y) in components {
        p.extend_from_slice(&[*s, *x, *y]);
    }
    seg(0xff51, &p)
}

/// A tile-part: SOT, the header segments given, SOD, and the data, with a
/// `Psot` that covers exactly that or the one given.
fn tile_part(isot: u16, psot: Option<u32>, tpsot: u8, tnsot: u8, header: &[u8], data: &[u8]) -> Vec<u8> {
    let whole = (2 + 10 + header.len() + 2 + data.len()) as u32;
    let mut v = vec![0xff, 0x90];
    v.extend_from_slice(&be16(10));
    v.extend_from_slice(&be16(isot));
    v.extend_from_slice(&be32(psot.unwrap_or(whole)));
    v.extend_from_slice(&[tpsot, tnsot]);
    v.extend_from_slice(header);
    v.extend_from_slice(&[0xff, 0x93]);
    v.extend_from_slice(data);
    v
}

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

fn ints(ev: &mut Evaluator, d: &Document<MemSource>, steps: &[&str]) -> Vec<i128> {
    let p = at(ev, d, steps);
    let n = ev.node(d, &p).unwrap().child_count as usize;
    (0..n).map(|i| ev.node(d, &[&p[..], &[i]].concat()).unwrap().value.as_int().unwrap()).collect()
}

/// A codestream with every main header segment Part 1 has, two components
/// that differ, and two tile-parts, the second of which says its length is
/// zero and so runs to EOC.
fn everything() -> (Vec<u8>, u32) {
    let mut v = vec![0xff, 0x4f];
    // Eight by six in tiles of four by six: two tiles. An eight-bit component,
    // and a signed twelve-bit one sampled every other point.
    v.extend(siz(8, 6, 4, 6, &[(0x07, 1, 1), (0x8b, 2, 2)]));
    // Precincts, SOP and EPH; RPCL, three layers, no transform; two levels,
    // 64 by 32 code-blocks, bypass, 5-3; three precinct sizes.
    v.extend(seg(0xff52, &[0x07, 2, 0, 3, 0, 2, 4, 3, 0x01, 1, 0x77, 0x88, 0x99]));
    // Component 1: one level, 16 by 16, 9-7, no precincts of its own.
    v.extend(seg(0xff53, &[1, 0, 1, 2, 2, 0, 0]));
    // Scalar expounded with two guard bits: seven step sizes, for two levels.
    let mut qcd = vec![(2 << 5) | 2];
    for i in 0..7u16 {
        qcd.extend_from_slice(&be16(((8 + i) << 11) | (0x100 + i)));
    }
    v.extend(seg(0xff5c, &qcd));
    // Component 1 unquantized with one guard bit: four exponents, one level.
    v.extend(seg(0xff5d, &[1, 1 << 5, 9 << 3, 10 << 3, 10 << 3, 11 << 3]));
    // Component 0 scalar derived: one step size.
    let mut derived = vec![0, (2 << 5) | 1];
    derived.extend_from_slice(&be16((13 << 11) | 0x7ff));
    v.extend(seg(0xff5d, &derived));
    v.extend(seg(0xff5e, &[1, 0, 5]));
    v.extend(seg(0xff5f, &[0, 0, 0, 3, 3, 2, 4, 1, 1, 0, 2, 2, 0, 0]));
    v.extend(seg(0xff57, &[0, 3, 0x81, 0x00, 0x05]));
    v.extend(seg(0xff60, &[0, 0, 0, 0, 2, 0xaa, 0xbb]));
    v.extend(seg(0xff63, &[0, 0, 0, 0, 0x80, 0, 0x80, 0]));
    v.extend(seg(0xff64, &[0, 1, b'h', b'i', 0xe9]));
    v.extend(seg(0xff64, &[0, 0, 1, 2, 3]));
    // A reserved marker with no parameters, which must not be given a length.
    v.extend([0xff, 0x30]);
    // TLM, filled in once the first tile-part's length is known: one-byte
    // tile numbers and four-byte lengths.
    let first_header = [seg(0xff58, &[0, 0x03, 0x82, 0x01]), seg(0xff61, &[0, 0xde, 0xad])].concat();
    let first = tile_part(0, None, 0, 1, &first_header, &[0x12, 0xff, 0x91, 0x00, 0x04, 0x00, 0x00, 0x34]);
    let psot = first.len() as u32;
    let mut tlm = vec![0, 0x50, 0];
    tlm.extend_from_slice(&be32(psot));
    tlm.push(1);
    tlm.extend_from_slice(&be32(0));
    v.extend(seg(0xff55, &tlm));
    v.extend(first);
    v.extend(tile_part(1, Some(0), 0, 0, &[], &[0x55, 0x66, 0xff, 0x80]));
    v.extend([0xff, 0xd9]);
    (v, psot)
}

#[test]
fn every_main_header_segment_reads_as_its_fields() {
    let (bytes, psot) = everything();
    let d = Document::new(MemSource(bytes.clone()));
    let mut ev = Evaluator::new(jpeg2000());
    assert_eq!(node(&mut ev, &d, &[]).type_name, "Codestream");
    let segments = node(&mut ev, &d, &["segments"]);
    assert_eq!(segments.child_count, 18);
    let marker = |ev: &mut Evaluator, i: usize| int(ev, &d, &["segments", &i.to_string(), "marker"]);
    let markers: Vec<i128> = (0..18).map(|i| marker(&mut ev, i)).collect();
    assert_eq!(
        markers,
        vec![
            0xff51, 0xff52, 0xff53, 0xff5c, 0xff5d, 0xff5d, 0xff5e, 0xff5f, 0xff57, 0xff60, 0xff63, 0xff64, 0xff64, 0xff30,
            0xff55, 0xff90, 0xff90, 0xffd9
        ]
    );

    // SIZ, and what it works out.
    let s = |name: &'static str| ["segments", "0", "body", name];
    assert_eq!(node(&mut ev, &d, &s("")[..3]).type_name, "SIZ");
    assert_eq!(int(&mut ev, &d, &s("Lsiz")), 38 + 3 * 2);
    assert_eq!((int(&mut ev, &d, &s("width")), int(&mut ev, &d, &s("height"))), (8, 6));
    assert_eq!((int(&mut ev, &d, &s("tiles_across")), int(&mut ev, &d, &s("tiles_down"))), (2, 1));
    let comp = |ev: &mut Evaluator, i: &str, f: &str| int(ev, &d, &["segments", "0", "body", "components", i, f]);
    assert_eq!((comp(&mut ev, "1", "Ssiz"), comp(&mut ev, "1", "signed"), comp(&mut ev, "1", "depth")), (0x8b, 1, 12));
    assert_eq!((comp(&mut ev, "1", "XRsiz"), comp(&mut ev, "1", "YRsiz")), (2, 2));
    assert_eq!((comp(&mut ev, "0", "signed"), comp(&mut ev, "0", "depth")), (0, 8));

    // COD.
    let c = |name: &'static str| ["segments", "1", "body", name];
    assert_eq!(
        node(&mut ev, &d, &c("progression_order")).value,
        Value::Enum { raw: 2, name: Some("RPCL".into()), hex: false }
    );
    assert_eq!(int(&mut ev, &d, &c("layers")), 3);
    assert_eq!((int(&mut ev, &d, &c("code_block_width")), int(&mut ev, &d, &c("code_block_height"))), (64, 32));
    let precincts = at(&mut ev, &d, &c("precinct_sizes"));
    assert_eq!(ev.node(&d, &precincts).unwrap().child_count, 3);
    assert_eq!(int(&mut ev, &d, &["segments", "1", "body", "precinct_sizes", "2", "PPy"]), 9);
    assert_eq!(int(&mut ev, &d, &["segments", "1", "body", "precinct_sizes", "2", "PPx"]), 9);

    // COC for component 1, with no precinct sizes since Scoc says none.
    assert_eq!(int(&mut ev, &d, &["segments", "2", "body", "Ccoc"]), 1);
    assert_eq!(node(&mut ev, &d, &["segments", "2", "body", "Ccoc"]).size_bits, 8);
    assert!(node(&mut ev, &d, &["segments", "2", "body", "precinct_sizes"]).absent);
    assert_eq!(int(&mut ev, &d, &["segments", "2", "body", "code_block_width"]), 16);

    // QCD: seven step sizes, which is two levels.
    let q = |name: &'static str| ["segments", "3", "body", name];
    assert_eq!((int(&mut ev, &d, &q("guard_bits")), int(&mut ev, &d, &q("quantization_style"))), (2, 2));
    assert_eq!(int(&mut ev, &d, &q("decomposition_levels")), 2);
    let steps = at(&mut ev, &d, &q("SPqcd"));
    assert_eq!(ev.node(&d, &steps).unwrap().child_count, 7);
    assert_eq!(int(&mut ev, &d, &["segments", "3", "body", "SPqcd", "6", "exponent"]), 14);
    assert_eq!(int(&mut ev, &d, &["segments", "3", "body", "SPqcd", "6", "mantissa"]), 0x106);

    // QCC for component 1: four exponents, which is one level.
    assert_eq!(int(&mut ev, &d, &["segments", "4", "body", "decomposition_levels"]), 1);
    let exps: Vec<i128> =
        (0..4).map(|i| int(&mut ev, &d, &["segments", "4", "body", "SPqcc", &i.to_string(), "exponent"])).collect();
    assert_eq!(exps, vec![9, 10, 10, 11]);
    // And scalar derived for component 0, which writes one and no levels.
    assert!(node(&mut ev, &d, &["segments", "5", "body", "decomposition_levels"]).absent);
    assert_eq!(int(&mut ev, &d, &["segments", "5", "body", "SPqcc", "exponent"]), 13);
    assert_eq!(int(&mut ev, &d, &["segments", "5", "body", "SPqcc", "mantissa"]), 0x7ff);

    // RGN and POC.
    assert_eq!((int(&mut ev, &d, &["segments", "6", "body", "Crgn"]), int(&mut ev, &d, &["segments", "6", "body", "SPrgn"])), (1, 5));
    assert_eq!(node(&mut ev, &d, &["segments", "7", "body", "changes"]).child_count, 2);
    assert_eq!(int(&mut ev, &d, &["segments", "7", "body", "changes", "0", "LYEpoc"]), 3);
    assert_eq!(int(&mut ev, &d, &["segments", "7", "body", "changes", "0", "Ppoc"]), 4);
    assert_eq!(int(&mut ev, &d, &["segments", "7", "body", "changes", "1", "CSpoc"]), 1);

    // PLM: one tile-part's lengths, 128 and 5.
    assert_eq!(ints(&mut ev, &d, &["segments", "8", "body", "tile_parts", "0", "Iplm"]), vec![128, 5]);
    // PPM: one tile-part's two bytes of headers.
    assert_eq!(int(&mut ev, &d, &["segments", "9", "body", "tile_parts", "0", "Nppm"]), 2);
    // CRG: the second component half a sample over and down.
    assert_eq!(int(&mut ev, &d, &["segments", "10", "body", "offsets", "1", "Xcrg"]), 0x8000);

    // COM as text when Rcom says so, and as bytes when it does not.
    assert_eq!(node(&mut ev, &d, &["segments", "11", "body", "Ccom"]).value, Value::Str("hi\u{e9}".into()));
    assert_eq!(node(&mut ev, &d, &["segments", "12", "body", "Ccom"]).type_name, "bytes[]");

    // The reserved marker took two bytes and nothing more.
    assert_eq!(node(&mut ev, &d, &["segments", "13"]).size_bits, 16);

    // TLM: one-byte tile numbers, four-byte lengths, and the first length is
    // the first tile-part's Psot.
    let t = |name: &'static str| ["segments", "14", "body", name];
    assert_eq!((int(&mut ev, &d, &t("ST")), int(&mut ev, &d, &t("SP"))), (1, 1));
    assert_eq!(node(&mut ev, &d, &t("tile_parts")).child_count, 2);
    assert_eq!(int(&mut ev, &d, &["segments", "14", "body", "tile_parts", "1", "Ttlm"]), 1);
    assert_eq!(int(&mut ev, &d, &["segments", "14", "body", "tile_parts", "0", "Ptlm"]), i128::from(psot));
    assert_eq!(int(&mut ev, &d, &["segments", "15", "body", "Psot"]), i128::from(psot));

    // Nothing after EOC.
    assert_eq!(node(&mut ev, &d, &["trailer"]).size_bits, 0);
}

#[test]
fn a_tile_part_is_as_long_as_psot_says() {
    let (bytes, psot) = everything();
    let d = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(jpeg2000());
    let sot = node(&mut ev, &d, &["segments", "15"]);
    assert_eq!(sot.size_bits, u64::from(psot) * 8);
    assert_eq!(node(&mut ev, &d, &["segments", "15", "body", "header"]).child_count, 3);
    // PLT: packet lengths 3 and 257.
    assert_eq!(ints(&mut ev, &d, &["segments", "15", "body", "header", "0", "body", "Iplt"]), vec![3, 257]);
    assert_eq!(int(&mut ev, &d, &["segments", "15", "body", "header", "1", "body", "Lppt"]), 5);
    assert_eq!(int(&mut ev, &d, &["segments", "15", "body", "header", "2", "marker"]), 0xff93);
    // The data holds bytes that look like a SOP marker and is not cut short by
    // them.
    assert_eq!(node(&mut ev, &d, &["segments", "15", "body", "data"]).size_bits, 8 * 8);
}

#[test]
fn a_tile_part_with_a_psot_of_zero_runs_to_eoc() {
    let (bytes, _) = everything();
    let d = Document::new(MemSource(bytes.clone()));
    let mut ev = Evaluator::new(jpeg2000());
    assert_eq!(int(&mut ev, &d, &["segments", "16", "body", "Psot"]), 0);
    let data = node(&mut ev, &d, &["segments", "16", "body", "data"]);
    assert_eq!(data.size_bits, 4 * 8);
    let eoc = node(&mut ev, &d, &["segments", "17"]);
    assert_eq!(data.offset_bits + data.size_bits, eoc.offset_bits);
    assert_eq!(eoc.offset_bits, (bytes.len() as u64 - 2) * 8);
}

#[test]
fn a_tile_part_with_a_psot_of_zero_and_no_eoc_runs_to_the_end() {
    let mut v = vec![0xff, 0x4f];
    v.extend(siz(4, 4, 4, 4, &[(0x07, 1, 1)]));
    v.extend(tile_part(0, Some(0), 0, 1, &[], &[1, 2, 3, 0xff, 0x7f, 5]));
    let d = Document::new(MemSource(v.clone()));
    let mut ev = Evaluator::new(jpeg2000());
    assert_eq!(node(&mut ev, &d, &["segments"]).child_count, 2);
    let data = node(&mut ev, &d, &["segments", "1", "body", "data"]);
    assert_eq!(data.size_bits, 6 * 8);
    assert_eq!(data.offset_bits + data.size_bits, v.len() as u64 * 8);
}

#[test]
fn a_codestream_of_more_than_256_components_numbers_them_in_two_bytes() {
    let mut v = vec![0xff, 0x4f];
    v.extend(siz(1, 1, 1, 1, &vec![(0x07, 1, 1); 300]));
    v.extend(seg(0xff53, &[0x01, 0x2b, 0, 0, 4, 4, 0, 1]));
    v.extend(seg(0xff5d, &[0x01, 0x2b, 0x40, 0x48, 0x50, 0x50, 0x58]));
    v.extend(seg(0xff5f, &[0, 0, 0, 0, 1, 1, 0x01, 0x2c, 0]));
    v.extend(tile_part(0, None, 0, 1, &seg(0xff5e, &[0x01, 0x2b, 0, 3]), &[]));
    v.extend([0xff, 0xd9]);
    let d = Document::new(MemSource(v));
    let mut ev = Evaluator::new(jpeg2000());
    assert_eq!(int(&mut ev, &d, &["segments", "1", "body", "Ccoc"]), 299);
    assert_eq!(node(&mut ev, &d, &["segments", "1", "body", "Ccoc"]).size_bits, 16);
    assert_eq!(int(&mut ev, &d, &["segments", "1", "body", "code_block_width"]), 64);
    // QCC's step sizes are what is left once the two-byte number is taken off.
    assert_eq!(node(&mut ev, &d, &["segments", "2", "body", "SPqcc"]).child_count, 4);
    assert_eq!(int(&mut ev, &d, &["segments", "3", "body", "changes", "0", "CEpoc"]), 300);
    // And an RGN in a tile-part header finds SIZ in the list outside its own.
    assert_eq!(int(&mut ev, &d, &["segments", "4", "body", "header", "0", "body", "Crgn"]), 299);
    assert_eq!(int(&mut ev, &d, &["segments", "4", "body", "header", "0", "body", "SPrgn"]), 3);
}

/// A box: its length, its letters and its contents.
fn jp2_box(kind: &[u8; 4], contents: &[u8]) -> Vec<u8> {
    let mut v = be32((contents.len() + 8) as u32).to_vec();
    v.extend_from_slice(kind);
    v.extend_from_slice(contents);
    v
}

/// A small codestream to put in a `jp2c` box.
fn small_codestream() -> Vec<u8> {
    let mut v = vec![0xff, 0x4f];
    v.extend(siz(8, 6, 8, 6, &[(0x07, 1, 1)]));
    v.extend(seg(0xff52, &[0, 0, 0, 1, 0, 0, 4, 4, 0, 1]));
    v.extend(seg(0xff5c, &[0x40, 0x48]));
    v.extend(tile_part(0, None, 0, 1, &[], &[0xaa, 0xbb]));
    v.extend([0xff, 0xd9]);
    v
}

/// A JP2 file with a palette, an XML box whose length is written in the
/// eight bytes after its letters, and a codestream box that runs to the end
/// of the file.
fn jp2() -> Vec<u8> {
    let mut v = jp2_box(b"jP  ", &[0x0d, 0x0a, 0x87, 0x0a]);
    v.extend(jp2_box(b"ftyp", &[b"jp2 ".as_slice(), &be32(0), b"jp2 "].concat()));
    let mut ihdr = [be32(6), be32(8)].concat();
    ihdr.extend_from_slice(&be16(1));
    ihdr.extend_from_slice(&[7, 7, 0, 0]);
    let mut colr = vec![1, 0, 0];
    colr.extend_from_slice(&be32(16));
    // Two entries of two columns: an eight-bit one and a twelve-bit one, which
    // takes two bytes.
    let mut pclr = be16(2).to_vec();
    pclr.extend_from_slice(&[2, 0x07, 0x0b, 1]);
    pclr.extend_from_slice(&be16(0x0203));
    pclr.push(4);
    pclr.extend_from_slice(&be16(0x0506));
    let cmap = [0, 0, 1, 0, 0, 0, 1, 1];
    let mut cdef = be16(1).to_vec();
    // Channel 0 is colour, and the first colour.
    cdef.extend_from_slice(&[0, 0, 0, 0, 0, 1]);
    // Capture resolution: 3 / 2 * 10^2 down and 5 / 4 * 10^-1 across.
    let resc = [be16(3).as_slice(), &be16(2), &be16(5), &be16(4), &[2, 0xff]].concat();
    let header = [
        jp2_box(b"ihdr", &ihdr),
        jp2_box(b"colr", &colr),
        jp2_box(b"pclr", &pclr),
        jp2_box(b"cmap", &cmap),
        jp2_box(b"cdef", &cdef),
        jp2_box(b"res ", &jp2_box(b"resc", &resc)),
    ]
    .concat();
    v.extend(jp2_box(b"jp2h", &header));
    let xml = b"<a>long</a>";
    v.extend_from_slice(&be32(1));
    v.extend_from_slice(b"xml ");
    v.extend_from_slice(&((xml.len() + 16) as u64).to_be_bytes());
    v.extend_from_slice(xml);
    v.extend_from_slice(&be32(0));
    v.extend_from_slice(b"jp2c");
    v.extend(small_codestream());
    v
}

#[test]
fn a_jp2_file_reads_as_its_boxes() {
    let bytes = jp2();
    let d = Document::new(MemSource(bytes.clone()));
    let mut ev = Evaluator::new(jpeg2000());
    let boxes = node(&mut ev, &d, &[]);
    assert_eq!(boxes.child_count, 5);
    let kinds: Vec<Value> = (0..5).map(|i| node(&mut ev, &d, &[&i.to_string(), "TBox"]).value).collect();
    assert_eq!(kinds, ["jP  ", "ftyp", "jp2h", "xml ", "jp2c"].map(|k| Value::Str(k.into())));
    assert!(matches!(node(&mut ev, &d, &["0", "DBox"]).value, Value::Magic { ok: true, .. }));
    assert_eq!(node(&mut ev, &d, &["1", "DBox", "CL", "0"]).value, Value::Str("jp2 ".into()));
    assert!(node(&mut ev, &d, &["1", "XLBox"]).absent);

    // The header: six boxes, the last a superbox of one.
    let h = |steps: &[&'static str]| [&["2", "DBox"], steps].concat();
    assert_eq!(node(&mut ev, &d, &h(&[])).child_count, 6);
    assert_eq!((int(&mut ev, &d, &h(&["0", "DBox", "WIDTH"])), int(&mut ev, &d, &h(&["0", "DBox", "depth"]))), (8, 8));
    assert_eq!(node(&mut ev, &d, &h(&["1", "DBox", "EnumCS"])).value, Value::Enum { raw: 16, name: Some("sRGB".into()), hex: false });
    assert!(node(&mut ev, &d, &h(&["1", "DBox", "PROFILE"])).absent);
    // The palette's second column is two bytes an entry because its depth is
    // twelve bits.
    assert_eq!(ints(&mut ev, &d, &h(&["2", "DBox", "C", "0"])), vec![1, 0x0203]);
    assert_eq!(ints(&mut ev, &d, &h(&["2", "DBox", "C", "1"])), vec![4, 0x0506]);
    assert_eq!(node(&mut ev, &d, &h(&["2", "DBox", "C", "1", "1"])).size_bits, 16);
    assert_eq!(node(&mut ev, &d, &h(&["3", "DBox"])).child_count, 2);
    assert_eq!(int(&mut ev, &d, &h(&["3", "DBox", "1", "MTYP"])), 1);
    assert_eq!(int(&mut ev, &d, &h(&["4", "DBox", "channels", "0", "Asoc"])), 1);
    let vrc = node(&mut ev, &d, &h(&["5", "DBox", "0", "DBox", "VRc"])).value;
    let hrc = node(&mut ev, &d, &h(&["5", "DBox", "0", "DBox", "HRc"])).value;
    assert_eq!((vrc, hrc), (Value::Float(150.0), Value::Float(0.125)));

    // XLBox: the length is the eight bytes after the letters.
    assert_eq!(int(&mut ev, &d, &["3", "LBox"]), 1);
    assert_eq!(int(&mut ev, &d, &["3", "XLBox"]), 11 + 16);
    assert_eq!(node(&mut ev, &d, &["3", "DBox"]).value, Value::Str("<a>long</a>".into()));
    assert_eq!(node(&mut ev, &d, &["3"]).size_bits, 27 * 8);

    // A length of zero runs to the end, and the codestream reads inside it.
    let codestream = node(&mut ev, &d, &["4", "DBox"]);
    assert_eq!(codestream.type_name, "Codestream");
    assert_eq!(codestream.offset_bits + codestream.size_bits, bytes.len() as u64 * 8);
    assert_eq!(int(&mut ev, &d, &["4", "DBox", "segments", "0", "body", "Xsiz"]), 8);
    assert_eq!(node(&mut ev, &d, &["4", "DBox", "segments", "3", "body", "data"]).size_bits, 16);
}

#[test]
fn a_colour_box_with_a_profile_reads_the_profile_and_no_colour_space() {
    let mut colr = vec![2, 0, 1];
    colr.extend_from_slice(b"a profile");
    let mut v = jp2_box(b"jP  ", &[0x0d, 0x0a, 0x87, 0x0a]);
    v.extend(jp2_box(b"jp2h", &jp2_box(b"colr", &colr)));
    let d = Document::new(MemSource(v));
    let mut ev = Evaluator::new(jpeg2000());
    assert!(node(&mut ev, &d, &["1", "DBox", "0", "DBox", "EnumCS"]).absent);
    assert_eq!(node(&mut ev, &d, &["1", "DBox", "0", "DBox", "PROFILE"]).size_bits, 9 * 8);
}

#[test]
fn a_box_too_short_for_its_own_header_still_lets_the_next_one_read() {
    let mut v = jp2_box(b"jP  ", &[0x0d, 0x0a, 0x87, 0x0a]);
    v.extend_from_slice(&be32(3));
    v.extend_from_slice(b"junk");
    v.extend(jp2_box(b"xml ", b"<b/>"));
    let d = Document::new(MemSource(v));
    let mut ev = Evaluator::new(jpeg2000());
    assert_eq!(node(&mut ev, &d, &[]).child_count, 3);
    assert_eq!(node(&mut ev, &d, &["2", "DBox"]).value, Value::Str("<b/>".into()));
}
