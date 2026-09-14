//! The templates against files built here a byte at a time.

use super::indices::*;
use super::recognise::sniff;
use super::*;
use crate::document::Document;
use crate::eval::{Evaluator, NodeInfo, Value};
use crate::source::MemSource;

/// Writes the records BP3 and BP4 share, little-endian.
#[derive(Default)]
struct W(Vec<u8>);

impl W {
    fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend(v.to_le_bytes());
        self
    }
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend(v.to_le_bytes());
        self
    }
    fn u64(&mut self, v: u64) -> &mut Self {
        self.0.extend(v.to_le_bytes());
        self
    }
    fn bytes(&mut self, v: &[u8]) -> &mut Self {
        self.0.extend_from_slice(v);
        self
    }
    fn name(&mut self, s: &str) -> &mut Self {
        self.u16(s.len() as u16).bytes(s.as_bytes())
    }
    fn len(&self) -> usize {
        self.0.len()
    }
}

fn header(kind: &str, version: u8) -> Vec<u8> {
    let mut h = format!("ADIOS-BP v2.<.1 {kind}").into_bytes();
    h.resize(32, 0);
    h.extend(b"2<1\0");
    h.extend([0, version, 0, 0]);
    h.resize(64, 0);
    h
}

/// A characteristic set, from records already packed.
fn set(records: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = records.concat();
    let mut w = W::default();
    w.u8(records.len() as u8).u32(body.len() as u32).bytes(&body);
    w.0
}

fn dims(d: &[(u64, u64, u64)]) -> Vec<u8> {
    let mut w = W::default();
    w.u8(DIMENSIONS as u8).u8(d.len() as u8).u16(24 * d.len() as u16);
    for (c, s, o) in d {
        w.u64(*c).u64(*s).u64(*o);
    }
    w.0
}

fn rec(id: i128, body: &[u8]) -> Vec<u8> {
    [vec![id as u8], body.to_vec()].concat()
}

/// An index entry: the header BP4 writes and the sets.
fn entry(name: &str, data_type: u8, sets: &[Vec<u8>]) -> Vec<u8> {
    let mut body = W::default();
    body.u32(0).u16(0).name(name).u8(b'K').u8(0).u8(data_type).u64(sets.len() as u64);
    for s in sets {
        body.bytes(s);
    }
    let mut w = W::default();
    w.u32(body.len() as u32).bytes(&body.0);
    w.0
}

fn node(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize]) -> NodeInfo {
    ev.node(d, path).unwrap()
}

/// Finds a child by name, so a test says which field it means.
fn child(ev: &mut Evaluator, d: &Document<MemSource>, parent: &[usize], name: &str) -> Vec<usize> {
    let n = node(ev, d, parent).child_count as usize;
    for i in 0..n {
        let p = [parent, &[i]].concat();
        let info = node(ev, d, &p);
        if info.name == name || info.name.ends_with(&format!(" {name}")) {
            return p;
        }
    }
    panic!("no {name} under {parent:?}");
}

fn at(ev: &mut Evaluator, d: &Document<MemSource>, names: &[&str]) -> Vec<usize> {
    names.iter().fold(vec![], |p, n| match n.parse::<usize>() {
        Ok(i) => [&p[..], &[i]].concat(),
        Err(_) => child(ev, d, &p, n),
    })
}

#[test]
fn a_bp4_index_is_a_header_and_a_record_a_step() {
    let mut f = header("Index Table", 4);
    for step in 1..=2u64 {
        let mut w = W::default();
        w.u64(step).u64(0).u64(64).u64(111).u64(200).u64(300).u64(1_780_000_000).u64(0);
        f.extend(w.0);
    }
    assert_eq!(sniff(&f, f.len() as u64), Some("adiosbp4idx"));
    let d = Document::new(MemSource(f));
    let mut ev = Evaluator::new(adios_bp4_index());
    let steps = at(&mut ev, &d, &["steps"]);
    assert_eq!(node(&mut ev, &d, &steps).child_count, 2);
    let step = at(&mut ev, &d, &["steps", "1", "step"]);
    assert_eq!(node(&mut ev, &d, &step).value.as_int(), Some(2));
    let bp = at(&mut ev, &d, &["header", "bp_version"]);
    assert_eq!(node(&mut ev, &d, &bp).value.as_int(), Some(4));
    let minor = at(&mut ev, &d, &["header", "adios_minor"]);
    assert_eq!(node(&mut ev, &d, &minor).value.as_int(), Some(12));
}

/// An attribute's value is as many numbers as the dimensions record before
/// it says, and a variable's sets read their records by tag whatever order
/// they are in.
#[test]
fn characteristics_are_read_by_their_tags() {
    let var = entry(
        "temperature",
        6,
        &[set(&[
            rec(TIME_INDEX, &2u32.to_le_bytes()),
            rec(FILE_INDEX, &0u32.to_le_bytes()),
            dims(&[(2, 4, 0), (3, 3, 0)]),
            rec(MINMAX, &[1u16.to_le_bytes().to_vec(), 0.5f64.to_le_bytes().to_vec(), 9.5f64.to_le_bytes().to_vec()].concat()),
            rec(OFFSET, &77u64.to_le_bytes()),
            rec(PAYLOAD_OFFSET, &99u64.to_le_bytes()),
        ])],
    );
    let attr = entry(
        "grid",
        2,
        &[set(&[
            rec(TIME_INDEX, &1u32.to_le_bytes()),
            dims(&[(2, 0, 0)]),
            rec(VALUE, &[4i32.to_le_bytes(), 3i32.to_le_bytes()].concat()),
        ])],
    );
    let mut w = W::default();
    // A process group index of one entry.
    let mut pg = W::default();
    pg.name("sim").u8(b'n').u32(0).name("2").u32(2).u64(64);
    w.u64(1).u64(pg.len() as u64 + 2).u16(pg.len() as u16).bytes(&pg.0);
    w.u32(1).u64(var.len() as u64).bytes(&var);
    w.u32(1).u64(attr.len() as u64).bytes(&attr);
    let f = [header("Metadata", 4), w.0].concat();
    assert_eq!(sniff(&f, f.len() as u64), Some("adiosbp4md"));
    let d = Document::new(MemSource(f));
    let mut ev = Evaluator::new(adios_bp4_metadata());
    let steps = at(&mut ev, &d, &["steps"]);
    assert_eq!(node(&mut ev, &d, &steps).child_count, 1);
    let sets = at(&mut ev, &d, &["steps", "0", "variables_index", "entries", "0", "sets"]);
    let s = [&sets[..], &[0]].concat();
    let step = child(&mut ev, &d, &s, "step");
    assert_eq!(node(&mut ev, &d, &step).value.as_int(), Some(2));
    let records = child(&mut ev, &d, &s, "characteristics");
    assert_eq!(node(&mut ev, &d, &records).child_count, 6);
    let max = at(&mut ev, &d, &["steps", "0", "variables_index", "entries", "0", "sets", "0", "characteristics", "3", "body", "max"]);
    assert_eq!(node(&mut ev, &d, &max).value, Value::Float(9.5));
    let value = at(&mut ev, &d, &["steps", "0", "attributes_index", "entries", "0", "sets", "0", "characteristics", "2", "body"]);
    let v = node(&mut ev, &d, &value);
    assert_eq!(v.child_count, 2);
    assert_eq!(node(&mut ev, &d, &[&value[..], &[1]].concat()).value, Value::Int(3));
}

/// A process group in a data file reads each block's values as its type
/// and dimensions say, and a block whose size does not match them as bytes.
#[test]
fn a_data_file_reads_blocks_by_their_headers() {
    let block = |name: &str, data_type: u8, counts: &[u64], values: &[u8]| {
        let mut body = W::default();
        body.u32(0).name(name).u8(b'K').u8(0).u8(data_type).u8(b'n').u8(counts.len() as u8).u16(27 * counts.len() as u16);
        for c in counts {
            body.u8(b'n').u64(*c).u8(b'n').u64(*c).u8(b'n').u64(0);
        }
        body.bytes(&set(&[])).u8(4).bytes(b"VMD]").bytes(values);
        let mut w = W::default();
        w.bytes(b"[VMD").u64(body.len() as u64 + 8).bytes(&body.0);
        w.0
    };
    let doubles: Vec<u8> = (0..6).flat_map(|i| (i as f64).to_le_bytes()).collect();
    let blocks = [block("grid", 6, &[2, 3], &doubles), block("packed", 6, &[4], &[1, 2, 3])].concat();
    let mut pg = W::default();
    pg.u8(b'n').name("sim").u32(0).name("1").u32(1).u8(1).u16(3).u8(2).u16(0);
    pg.u32(2).u64(blocks.len() as u64).bytes(&blocks).u32(0).u64(0).bytes(b"PGI]");
    let mut w = W::default();
    w.bytes(b"[PGI").u64(pg.len() as u64 + 8).bytes(&pg.0);
    let f = [header("Data", 4), w.0].concat();
    assert_eq!(sniff(&f, f.len() as u64), Some("adiosbp4data"));
    let n = f.len() as u64;
    let d = Document::new(MemSource(f));
    let mut ev = Evaluator::new(adios_bp4_data());
    let groups = at(&mut ev, &d, &["process_groups"]);
    let g = node(&mut ev, &d, &groups);
    assert_eq!((g.child_count, g.offset_bits + g.size_bits), (1, n * 8));
    let grid = at(&mut ev, &d, &["process_groups", "0", "variables", "0", "values"]);
    assert_eq!(node(&mut ev, &d, &grid).child_count, 2);
    assert_eq!(node(&mut ev, &d, &[&grid[..], &[1, 2]].concat()).value, Value::Float(5.0));
    let packed = at(&mut ev, &d, &["process_groups", "0", "variables", "1", "values"]);
    assert_eq!(node(&mut ev, &d, &packed).type_name, "bytes[]");
}

#[test]
fn a_bp5_index_reads_records_by_kind_and_length() {
    let mut w = W::default();
    w.u8(b'w').u64(32).u64(1).u64(1).u64(1).u64(0);
    w.u8(b's').u64(32).u64(0).u64(1216).u64(0).u64(0);
    w.u8(b'x').u64(3).bytes(&[1, 2, 3]);
    let f = [header("Index Table", 5), w.0].concat();
    assert_eq!(sniff(&f, f.len() as u64), Some("adiosbp5idx"));
    let d = Document::new(MemSource(f));
    let mut ev = Evaluator::new(adios_bp5_index());
    let records = at(&mut ev, &d, &["records"]);
    assert_eq!(node(&mut ev, &d, &records).child_count, 3);
    let writers = at(&mut ev, &d, &["records", "1", "body", "writer_count"]);
    assert_eq!(node(&mut ev, &d, &writers).value.as_int(), Some(1));
    let size = at(&mut ev, &d, &["records", "1", "body", "metadata_size"]);
    assert_eq!(node(&mut ev, &d, &size).value.as_int(), Some(1216));
}

/// One writer's sizes adding up to the total is what reads a step into
/// its blocks; two writers' cannot, and stay bytes.
#[test]
fn a_bp5_step_of_one_writer_reads_its_ffs_blocks() {
    let record = |len: usize| {
        let mut w = W::default();
        w.bytes(&[2, 0, 0, 5, 1, 2, 3, 4, 5, 6, 7, 8]).u64(len as u64 - 24).u32(0).bytes(&vec![0xAB; len - 24]);
        w.0
    };
    let (meta, attrs) = (record(64), record(40));
    let mut w = W::default();
    w.u64(16 + 64 + 40).u64(64).u64(40).bytes(&meta).bytes(&attrs);
    assert_eq!(sniff(&w.0, w.0.len() as u64), Some("adiosbp5md"));
    // Two writers: metadata of 64 and 9 bytes, attributes of 40 and none.
    let two = [32u64 + 64 + 9 + 40, 64, 9, 40, 0].iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>();
    w.bytes(&two).bytes(&meta).bytes(&[0; 9]).bytes(&attrs);
    let d = Document::new(MemSource(w.0));
    let mut ev = Evaluator::new(adios_bp5_metadata());
    assert_eq!(node(&mut ev, &d, &[]).child_count, 2);
    // With no formats read beside it, a record's data is its bytes, saying
    // where the formats are.
    let data = at(&mut ev, &d, &["0", "blocks", "metadata", "data"]);
    let data = node(&mut ev, &d, &data);
    assert_eq!(data.size_bits, 40 * 8);
    assert!(data.doc.unwrap_or_default().starts_with("mmd.0 not opened"));
    let second = at(&mut ev, &d, &["1", "blocks"]);
    let second = node(&mut ev, &d, &second);
    assert_eq!((second.type_name.as_str(), second.child_count), ("Bp5UnknownBlocks", 1));
    assert!(second.doc.unwrap_or_default().contains("md.idx"));
}

/// Writes an FFS format's representation the way `mmd.0` keeps it, and records
/// written in it.
mod ffs_bytes {
    /// One subformat: its name, its fields as name, type, size and offset,
    /// its record length, and whether it is big-endian.
    pub fn subformat(name: &str, fields: &[(&str, &str, i32, i32)], length: i32, big: bool) -> Vec<u8> {
        let n16 = |v: u16| if big { v.to_be_bytes() } else { v.to_le_bytes() };
        let n32 = |v: u32| if big { v.to_be_bytes() } else { v.to_le_bytes() };
        let mut strings = Vec::new();
        let base = 28 + 16 * fields.len();
        let offset_of = |s: &str, strings: &mut Vec<u8>| {
            let at = base + strings.len();
            strings.extend(s.as_bytes());
            strings.push(0);
            at as u32
        };
        let name_at = offset_of(name, &mut strings);
        let mut table = Vec::new();
        for (field, ty, size, offset) in fields {
            let (f, t) = (offset_of(field, &mut strings), offset_of(ty, &mut strings));
            table.extend(n32(f));
            table.extend(n32(t));
            table.extend(n32(*size as u32));
            table.extend(n32(*offset as u32));
        }
        let total = base + strings.len();
        let mut out = Vec::new();
        out.extend((total as u16).to_be_bytes());
        out.extend([2, big as u8]);
        out.extend(n32(name_at));
        out.extend(n32(fields.len() as u32));
        out.extend(n32(length as u32));
        out.extend([8, 28]);
        out.extend(n16(if big { 1 } else { 2 }));
        out.extend(n16(0));
        out.extend([0, 8]);
        out.extend(0u16.to_be_bytes());
        out.extend(n16(0));
        out.extend(table);
        out.extend(strings);
        out
    }

    /// A block of `mmd.0`: the ID, and the format, its subformats after it.
    pub fn format(id: [u8; 12], subformats: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = subformats.concat();
        let mut rep = Vec::new();
        rep.extend(((8 + body.len()) as u16).to_be_bytes());
        rep.extend([0, 2, (subformats.len() - 1) as u8, 0]);
        rep.extend(0u16.to_be_bytes());
        rep.extend(body);
        while rep.len() % 8 != 0 {
            rep.push(0);
        }
        let mut out = Vec::new();
        out.extend(12u64.to_le_bytes());
        out.extend((rep.len() as u64).to_le_bytes());
        out.extend(id);
        out.extend(rep);
        out
    }

    /// An encoded record: the ID, the length of the data, padding, the data.
    pub fn record(id: [u8; 12], data: &[u8]) -> Vec<u8> {
        let mut out = id.to_vec();
        out.extend((data.len() as u64).to_le_bytes());
        out.extend([0; 4]);
        out.extend(data);
        out
    }
}

/// A made-up template reading formats and then records, the way the dataset
/// reads `mmd.0` before `md.0`.
fn formats_then_records(formats: &[u8]) -> crate::template::Template {
    let root = T::structure(
        "Joined",
        vec![
            ("mmd_0", T::sized(E::lit(formats.len() as i128), super::ffs::metametadata_root())),
            ("records", T::repeat(super::ffs::ffs_record(crate::template::Endian::Little, super::ffs_schema::FORMATS), Until::End)),
        ],
    );
    super::ffs_schema::register(crate::template::Template::new("joined", root))
}

use crate::template::{Expr as E, Ty as T, Until};

const RECORD_ID: [u8; 12] = [2, 0, 0, 9, 1, 2, 3, 4, 5, 6, 7, 8];

/// A format of every kind of field FFS writes: a count, a pointer to that
/// many integers, a string, a subformat written in place, a fixed run of
/// characters, a pointer whose count comes after it, and a list of strings.
fn record_format(big: bool) -> Vec<u8> {
    let rec = ffs_bytes::subformat(
        "Rec",
        &[
            ("count", "integer", 4, 0),
            ("values", "integer[count]", 8, 8),
            ("label", "string", 8, 16),
            ("inner", "Inner", 16, 24),
            ("tag", "char[4]", 1, 40),
            ("early", "float[later]", 8, 48),
            ("later", "unsigned integer", 8, 56),
            ("names", "string[count]", 8, 64),
        ],
        72,
        big,
    );
    let inner = ffs_bytes::subformat("Inner", &[("a", "unsigned integer", 2, 0), ("b", "float", 8, 8)], 16, big);
    ffs_bytes::format(RECORD_ID, &[rec, inner])
}

fn record_data(big: bool) -> Vec<u8> {
    let (u16b, u32b, u64b) = (
        |v: u16, big: bool| if big { v.to_be_bytes() } else { v.to_le_bytes() },
        |v: u32, big: bool| if big { v.to_be_bytes() } else { v.to_le_bytes() },
        |v: u64, big: bool| if big { v.to_be_bytes() } else { v.to_le_bytes() },
    );
    let mut d = Vec::new();
    d.extend(u32b(2, big));
    d.extend([0; 4]);
    d.extend(u64b(72, big));
    d.extend(u64b(88, big));
    d.extend(u16b(0x1234, big));
    d.extend([0; 6]);
    d.extend(if big { 2.5f64.to_be_bytes() } else { 2.5f64.to_le_bytes() });
    d.extend(b"abcd");
    d.extend([0; 4]);
    d.extend(u64b(96, big));
    d.extend(u64b(3, big));
    d.extend(u64b(104, big));
    // The variable part: the integers, the string, and the strings.
    d.extend(u64b(7, big));
    d.extend(u64b((-8i64) as u64, big));
    d.extend(b"hi\0\0\0\0\0\0");
    d.extend([0; 8]);
    d.extend(u64b(120, big));
    d.extend(u64b(0, big));
    d.extend(b"x\0\0\0\0\0\0\0");
    d
}

/// A record reads as the format its ID names: each field at its offset, a
/// pointer as its offset and what it points at, counted by the field its type
/// names, a subformat in place, and the byte order the format says.
#[test]
fn an_ffs_record_reads_by_the_format_its_id_names() {
    for big in [false, true] {
        let formats = record_format(big);
        let mut file = formats.clone();
        file.extend(ffs_bytes::record(RECORD_ID, &record_data(big)));
        let d = Document::new(MemSource(file));
        let mut ev = Evaluator::new(formats_then_records(&formats));
        let data = at(&mut ev, &d, &["records", "0", "data"]);
        let info = node(&mut ev, &d, &data);
        assert_eq!(info.type_name, "Rec", "big-endian {big}: {:?}", info.doc);
        let get = |ev: &mut Evaluator, names: &[&str]| {
            let p = at(ev, &d, &[&["records", "0", "data"][..], names].concat());
            node(ev, &d, &p)
        };
        assert_eq!(get(&mut ev, &["count"]).value, Value::Int(2));
        assert_eq!(get(&mut ev, &["values", "offset"]).value, Value::UInt(72));
        assert_eq!(get(&mut ev, &["values", "values", "values", "1"]).value, Value::Int(-8));
        assert_eq!(get(&mut ev, &["label", "text", "text"]).value, Value::Str("hi".into()));
        assert_eq!(get(&mut ev, &["inner", "a"]).value, Value::UInt(0x1234));
        assert_eq!(get(&mut ev, &["inner", "b"]).value, Value::Float(2.5));
        assert_eq!(get(&mut ev, &["tag"]).size_bits, 32);
        // A pointer whose count is read after it stays the offset it is.
        assert_eq!(get(&mut ev, &["early"]).value, Value::UInt(96));
        assert_eq!(get(&mut ev, &["later"]).value, Value::UInt(3));
        assert_eq!(get(&mut ev, &["names", "values", "values", "0", "text", "text"]).value, Value::Str("x".into()));
        assert!(get(&mut ev, &["names", "values", "values", "1", "text"]).absent, "a pointer of nought points at nothing");
        // Padding where the format leaves a gap, to the record length.
        assert_eq!(get(&mut ev, &["padding"]).size_bits, 32);
        assert_eq!(info.size_bits, 128 * 8);
    }
}

/// A record whose ID is not among the formats, and a record read with no
/// formats at all, stay bytes and say which.
#[test]
fn a_record_whose_format_is_not_there_says_so() {
    let formats = record_format(false);
    let mut file = formats.clone();
    let other = [2, 0, 0, 9, 9, 9, 9, 9, 9, 9, 9, 9];
    file.extend(ffs_bytes::record(other, &record_data(false)));
    let d = Document::new(MemSource(file));
    let mut ev = Evaluator::new(formats_then_records(&formats));
    let data = at(&mut ev, &d, &["records", "0", "data"]);
    let info = node(&mut ev, &d, &data);
    assert_eq!(info.size_bits, 128 * 8);
    assert_eq!(info.doc.as_deref(), Some("no format 020000090909090909090909 in mmd.0"));
}

/// A ZIP is a BP5 dataset when it stores a BP5 index or format list, and a ZIP
/// like any other when those are compressed.
#[test]
fn a_zip_storing_a_bp5_directory_is_told_from_one_that_compresses_it() {
    let index = [header("Index Table", 5), vec![0; 16]].concat();
    let entry = |name: &str, method: u16, data: &[u8]| {
        let mut w = W::default();
        w.u32(0x0403_4b50).u16(20).u16(0).u16(method).u16(0).u16(0x21).u32(0).u32(data.len() as u32).u32(data.len() as u32);
        w.u16(name.len() as u16).u16(0).bytes(name.as_bytes()).bytes(data);
        w.0
    };
    let stored = [entry("steps.bp5/", 0, &[]), entry("steps.bp5/md.idx", 0, &index)].concat();
    assert_eq!(crate::formats::sniff(&stored, stored.len() as u64 + 100), Some("adioszip"));
    let packed = entry("steps.bp5/md.idx", 8, &index);
    assert_ne!(crate::formats::sniff(&packed, packed.len() as u64 + 100), Some("adioszip"));
}

/// A BP3 file of one process group, one variable of three int32s and one
/// double attribute, with its indices and footer.
fn bp3_file() -> Vec<u8> {
    let characteristics = |records: &[Vec<u8>]| set(records);
    let values: Vec<u8> = [7i32, 8, 9].iter().flat_map(|v| v.to_le_bytes()).collect();
    let mut var = W::default();
    var.u32(0).name("v").name("").u8(2).u8(b'n').u8(1).u16(27);
    var.u8(b'n').u64(3).u8(b'n').u64(3).u8(b'n').u64(0);
    var.bytes(&characteristics(&[dims(&[(3, 3, 0)]), rec(MIN, &7i32.to_le_bytes()), rec(MAX, &9i32.to_le_bytes())]));
    let header_len = 8 + var.len();
    let mut attr = W::default();
    attr.u32(0).name("a").name("").u8(b'n').u8(6).u32(8).bytes(&2.5f64.to_le_bytes());
    let mut group = W::default();
    group.u8(b'n').name("io").u32(0).name("1").u32(1).u8(1).u16(3).u8(2).u16(0);
    let variables_at = 8 + group.len() + 12;
    group.u32(1).u64(0).u64((header_len + values.len()) as u64).bytes(&var.0).bytes(&values);
    group.u32(1).u64(8 + 4 + attr.len() as u64).u32(4 + attr.len() as u32).bytes(&attr.0);
    let attribute_at = 8 + group.len() - attr.len() - 4;
    let mut f = W::default();
    f.u64(group.len() as u64).bytes(&group.0);
    let pg_start = f.len() as u64;
    let mut pg = W::default();
    pg.name("io").u8(b'n').u32(0).name("1").u32(1).u64(0);
    f.u64(1).u64(pg.len() as u64 + 2).u16(pg.len() as u16).bytes(&pg.0);
    let vars_start = f.len() as u64;
    let placed = |at: usize, payload: usize| [rec(OFFSET, &(at as u64).to_le_bytes()), rec(PAYLOAD_OFFSET, &(payload as u64).to_le_bytes())];
    let [off, pay] = placed(variables_at, variables_at + header_len);
    let var_set = characteristics(&[
        rec(TIME_INDEX, &1u32.to_le_bytes()),
        rec(FILE_INDEX, &0u32.to_le_bytes()),
        rec(MIN, &7i32.to_le_bytes()),
        rec(MAX, &9i32.to_le_bytes()),
        dims(&[(3, 3, 0)]),
        off,
        pay,
    ]);
    let index_entry = |name: &str, data_type: u8, s: &[u8]| {
        let mut body = W::default();
        body.u32(0).u16(0).name(name).u16(0).u8(data_type).u64(1).bytes(s);
        let mut w = W::default();
        w.u32(body.len() as u32).bytes(&body.0);
        w.0
    };
    let entry = index_entry("v", 2, &var_set);
    f.u32(1).u64(entry.len() as u64).bytes(&entry);
    let attrs_start = f.len() as u64;
    let [off, pay] = placed(attribute_at, attribute_at + 4 + 4 + 3 + 2 + 1 + 1);
    let attr_set = characteristics(&[
        rec(TIME_INDEX, &1u32.to_le_bytes()),
        rec(FILE_INDEX, &0u32.to_le_bytes()),
        dims(&[(1, 0, 0)]),
        rec(VALUE, &2.5f64.to_le_bytes()),
        off,
        pay,
    ]);
    let entry = index_entry("a", 6, &attr_set);
    f.u32(1).u64(entry.len() as u64).bytes(&entry);
    let mut tag = b"ADIOS-BP v2.<.1".to_vec();
    tag.resize(24, 0);
    f.bytes(&tag).bytes(b"2<1\0").u64(pg_start).u64(vars_start).u64(attrs_start).bytes(&[0, 0, 0, 3]);
    f.0
}

/// The values a BP3 index places by payload offset are the ones the
/// process group before it reads in order, and the file is recognised
/// by its footer when the whole of it is seen and by its front when not.
#[test]
fn a_bp3_index_places_the_values_its_process_group_holds() {
    let f = bp3_file();
    assert_eq!(sniff(&f, f.len() as u64), Some("adiosbp3"));
    assert_eq!(sniff(&f[..64], f.len() as u64 + 4096), Some("adiosbp3"));
    let n = f.len() as u64;
    let d = Document::new(MemSource(f));
    let mut ev = Evaluator::new(adios_bp3());
    let walked = at(&mut ev, &d, &["process_groups", "0", "variables", "0", "values"]);
    let walked = node(&mut ev, &d, &walked);
    assert_eq!(walked.child_count, 3);
    let placed = at(&mut ev, &d, &["variables_index", "variables_index", "entries", "0", "sets", "0", "values", "values"]);
    let placed_info = node(&mut ev, &d, &placed);
    assert_eq!((placed_info.offset_bits, placed_info.child_count), (walked.offset_bits, 3));
    assert_eq!(node(&mut ev, &d, &[&placed[..], &[2]].concat()).value, Value::Int(9));
    let attribute = at(&mut ev, &d, &["attributes_index", "attributes_index", "entries", "0", "sets", "0", "characteristics", "3", "body"]);
    assert_eq!(node(&mut ev, &d, &attribute).value, Value::Float(2.5));
    let in_group = at(&mut ev, &d, &["process_groups", "0", "attributes", "0", "value", "values", "0"]);
    assert_eq!(node(&mut ev, &d, &in_group).value, Value::Float(2.5));
    let footer = at(&mut ev, &d, &["footer", "footer"]);
    let footer = node(&mut ev, &d, &footer);
    assert_eq!(footer.offset_bits + footer.size_bits, n * 8);
}

#[test]
fn nothing_else_is_claimed() {
    assert_eq!(sniff(b"ADIOS-BP v2.9.0 Something\0\0\0\0\0\0\0", 32), None);
    let mut zeros = vec![0u8; 256];
    assert_eq!(sniff(&zeros, 256), None);
    zeros[0] = 12;
    assert_eq!(sniff(&zeros, 256), None);
}
