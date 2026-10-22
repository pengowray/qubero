//! Real Avro files, read against what fastavro reads from them.
//!
//! `tools/avro_reference.py` wrote `reference.tsv` beside the files in the
//! sample collection's `avro` folder: every value of every record, at its path
//! in the record. Each is read here through the template, down that path, and
//! has to be the same value. The folder holds a real Rubin alert in the
//! Confluent wire format (schema 9.0, three cutouts), alert_packet's container
//! file of schema 11.1, and container files written by the collection's
//! `tools/make_avro_samples.py` with every Avro type in them and one codec
//! each. Without the collection the tests say so and pass.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use qubero_core::checksum::crc32;
use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, NodeInfo, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

fn samples() -> Option<PathBuf> {
    qubero_samples::roots().into_iter().map(|r| r.join("avro")).find(|p| p.join("reference.tsv").exists())
}

/// What fastavro read: per file, the record count and each record's values by
/// path.
#[derive(Default)]
struct Expected {
    records: usize,
    values: BTreeMap<usize, Vec<(String, String)>>,
}

fn reference(dir: &Path) -> BTreeMap<String, Expected> {
    let text = std::fs::read_to_string(dir.join("reference.tsv")).unwrap();
    let mut out: BTreeMap<String, Expected> = BTreeMap::new();
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let cols: Vec<&str> = line.splitn(4, '\t').collect();
        let [file, record, path, value] = cols[..] else { panic!("bad line {line}") };
        let e = out.entry(file.into()).or_default();
        match record {
            "-" => e.records = value.parse().unwrap(),
            i => e.values.entry(i.parse().unwrap()).or_default().push((path.into(), value.into())),
        }
    }
    out
}

/// One step of a reference path.
#[derive(Debug)]
enum Step {
    Field(String),
    Index(usize),
    Key(String),
    Len,
}

fn steps(path: &str) -> Vec<Step> {
    let mut out = Vec::new();
    let mut name = String::new();
    let mut chars = path.chars().peekable();
    let flush = |name: &mut String, out: &mut Vec<Step>| {
        if !name.is_empty() {
            out.push(Step::Field(std::mem::take(name)));
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '.' => flush(&mut name, &mut out),
            '#' => {
                flush(&mut name, &mut out);
                out.push(Step::Len);
            }
            '[' => {
                flush(&mut name, &mut out);
                let mut inside = String::new();
                if chars.peek() == Some(&'"') {
                    // A JSON string, which may hold `]` itself.
                    let mut escaped = false;
                    inside.push(chars.next().unwrap());
                    for c in chars.by_ref() {
                        inside.push(c);
                        if c == '"' && !escaped {
                            break;
                        }
                        escaped = c == '\\' && !escaped;
                    }
                    assert_eq!(chars.next(), Some(']'));
                    out.push(Step::Key(unquote(&inside)));
                } else {
                    for c in chars.by_ref() {
                        if c == ']' {
                            break;
                        }
                        inside.push(c);
                    }
                    out.push(Step::Index(inside.parse().unwrap()));
                }
            }
            c => name.push(c),
        }
    }
    flush(&mut name, &mut out);
    out
}

/// Text as Python's `json.dumps(s, ensure_ascii=False)` writes it.
fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn unquote(s: &str) -> String {
    match qubero_core::json::parse(s.as_bytes()).unwrap().kind {
        qubero_core::json::Kind::Text(t) => t,
        other => panic!("{s} is {other:?}"),
    }
}

struct Reader {
    ev: Evaluator,
    doc: Document<MemSource>,
}

impl Reader {
    fn new(bytes: Vec<u8>) -> Reader {
        Reader { ev: Evaluator::new(formats::builtin("avro").unwrap()), doc: Document::new(MemSource(bytes)) }
    }

    fn node(&mut self, at: &[usize]) -> NodeInfo {
        self.ev.node(&self.doc, at).unwrap_or_else(|e| panic!("{at:?}: {e:?}"))
    }

    fn child(&mut self, at: &[usize], name: &str) -> Vec<usize> {
        self.ev.child_named(&self.doc, at, name).unwrap().unwrap_or_else(|| panic!("no {name} under {at:?}"))
    }

    fn kids(&mut self, at: &[usize]) -> Vec<Vec<usize>> {
        (0..self.node(at).child_count as usize).map(|i| [at, &[i]].concat()).collect()
    }

    /// A union is a branch and a value, and a path goes through to the value.
    fn unwrap(&mut self, mut at: Vec<usize>) -> Vec<usize> {
        loop {
            let n = self.node(&at);
            if n.child_count != 2 || n.absent {
                return at;
            }
            let first = self.node(&[at.as_slice(), &[0]].concat());
            if first.name != "branch" || first.type_name != "branch" {
                return at;
            }
            at = self.child(&at, "value");
        }
    }

    /// Every item of an array or a map, across its blocks.
    fn items(&mut self, at: &[usize]) -> Vec<Vec<usize>> {
        let mut out = Vec::new();
        for block in self.kids(at) {
            let items = self.child(&block, "items");
            out.extend(self.kids(&items));
        }
        out
    }

    /// Every record in the file.
    fn records(&mut self) -> Vec<Vec<usize>> {
        let root = self.node(&[]);
        if root.type_name == "FramedRecord" {
            return vec![self.child(&[], "record")];
        }
        let blocks = self.child(&[], "blocks");
        let mut out = Vec::new();
        for block in self.kids(&blocks) {
            let mut at = self.child(&block, "data");
            // Through a codec's run, and the CRC-32 snappy puts after it, to
            // the list of records.
            loop {
                let n = self.node(&at);
                if n.type_name == "SnappyBlock" {
                    at = self.child(&at, "data");
                } else if n.decoded && !n.list {
                    at.push(0);
                } else {
                    break;
                }
            }
            out.extend(self.kids(&at));
        }
        out
    }

    fn walk(&mut self, record: &[usize], path: &str) -> Result<Vec<usize>, String> {
        let mut at = record.to_vec();
        for step in steps(path) {
            at = self.unwrap(at);
            at = match step {
                Step::Field(name) => self.ev.child_named(&self.doc, &at, &name).unwrap().ok_or(format!("no field {name}"))?,
                Step::Index(i) => self.items(&at).get(i).cloned().ok_or(format!("no item {i}"))?,
                Step::Key(k) => {
                    let mut found = None;
                    for entry in self.items(&at) {
                        let key = self.child(&entry, "key");
                        let text = self.child(&key, "text");
                        if self.node(&text).value == Value::Str(k.clone()) {
                            found = Some(self.child(&entry, "value"));
                            break;
                        }
                    }
                    found.ok_or(format!("no key {k}"))?
                }
                // The count stands in for the list, and is read below.
                Step::Len => return Ok(at),
            };
        }
        Ok(self.unwrap(at))
    }

    /// The bytes under a node, from the space it is in.
    fn bytes_of(&mut self, n: &NodeInfo) -> Vec<u8> {
        let (from, len) = ((n.offset_bits / 8) as usize, (n.size_bits / 8) as usize);
        let space = if n.space == 0 { self.doc.source().0.as_slice() } else { self.ev.space_bytes(n.space).expect("an opened space") };
        space[from..from + len].to_vec()
    }

    /// What the template reads at `path`, written the way the reference
    /// writes values.
    fn value(&mut self, record: &[usize], path: &str) -> Result<(String, String), String> {
        let at = self.walk(record, path)?;
        let n = self.node(&at);
        if path.ends_with('#') {
            let count = self.items(&at).len();
            return Ok((count.to_string(), n.type_name));
        }
        if n.absent {
            return Ok(("null".into(), n.type_name));
        }
        let text = match (n.type_name.as_str(), &n.value) {
            ("string", _) => {
                let t = self.child(&at, "text");
                match self.node(&t).value {
                    Value::Str(s) => quote(&s),
                    other => return Err(format!("text is {other:?}")),
                }
            }
            ("bytes", _) => {
                let len = self.child(&at, "length");
                let len = self.node(&len).value.as_int().unwrap();
                let data = self.child(&at, "data");
                let data = self.node(&data);
                let bytes = self.bytes_of(&data);
                assert_eq!(bytes.len() as i128, len);
                format!("bytes:{}:{:08x}", bytes.len(), crc32(&bytes))
            }
            (_, Value::Bytes { .. }) => {
                let bytes = self.bytes_of(&n);
                format!("bytes:{}:{:08x}", bytes.len(), crc32(&bytes))
            }
            ("boolean", Value::Enum { name: Some(b), .. }) => b.clone(),
            (_, Value::Enum { name: Some(s), .. }) => quote(s),
            (_, Value::Int(v)) => v.to_string(),
            (_, Value::UInt(v)) => v.to_string(),
            (_, Value::Float(f)) => format!("{f:?}"),
            (_, other) => return Err(format!("{other:?}")),
        };
        Ok((text, n.type_name))
    }
}

/// Whether the template's value is the one fastavro read. A float is compared
/// at the width it is stored in, and bit for bit, so -0.0 is not 0.0.
fn same(found: &str, ty: &str, expected: &str) -> bool {
    let (Ok(f), Ok(e)) = (found.parse::<f64>(), expected.parse::<f64>()) else { return found == expected };
    if !ty.starts_with('f') {
        return found == expected;
    }
    match ty {
        _ if e.is_nan() => f.is_nan(),
        "f32 le" => (f as f32).to_bits() == (e as f32).to_bits(),
        _ => f.to_bits() == e.to_bits(),
    }
}

#[test]
fn every_value_reads_as_fastavro_reads_it() {
    let Some(dir) = samples() else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let mut wrong = Vec::new();
    let mut checked = 0;
    let reference = reference(&dir);
    assert!(reference.len() >= 8, "{} files in the reference", reference.len());
    for (file, expected) in reference {
        let mut r = Reader::new(std::fs::read(dir.join(&file)).unwrap());
        let records = r.records();
        if records.len() != expected.records {
            wrong.push(format!("{file}: {} records, fastavro says {}", records.len(), expected.records));
            continue;
        }
        for (i, values) in expected.values {
            for (path, value) in values {
                checked += 1;
                match r.value(&records[i], &path) {
                    Ok((found, ty)) if same(&found, &ty, &value) => {}
                    Ok((found, ty)) => wrong.push(format!("{file} [{i}] {path}: {found} ({ty}), fastavro says {value}")),
                    Err(why) => wrong.push(format!("{file} [{i}] {path}: {why}, fastavro says {value}")),
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{} of {checked} values differ:\n{}", wrong.len(), wrong.join("\n"));
    assert!(checked > 1500, "only {checked} values");
}

/// Each cutout of the real alert is a FITS file in a bytes field, and opens as
/// one: a primary image and the extensions astropy's CCDData writes.
#[test]
fn an_alerts_cutouts_open_as_fits() {
    let Some(dir) = samples() else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let mut r = Reader::new(std::fs::read(dir.join("7912941781254298.avro")).unwrap());
    let record = r.child(&[], "record");
    for cutout in ["cutoutDifference", "cutoutScience", "cutoutTemplate"] {
        let at = r.walk(&record, cutout).unwrap();
        let data = r.child(&at, "data");
        let id = r.ev.open_space(&r.doc, 0, &data).expect("resolves").expect("the cutout opens");
        let space = r.ev.space(id).expect("just opened");
        assert_eq!(space.template, "fits", "{cutout} opened as {:?}", space.template);
        let space = r.ev.space_mut(id).unwrap();
        let (ev, doc) = space.reading().expect("a reading of its own");
        let hdus = ev.node(doc, &[0]).unwrap();
        assert_eq!(hdus.name, "hdus");
        assert!(hdus.child_count >= 2, "{cutout} has {} HDUs", hdus.child_count);
        for h in 0..hdus.child_count as usize {
            let hdu = ev.node(doc, &[0, h]).unwrap();
            assert!(hdu.problems_within == (0, 0), "{cutout} HDU {h}: {:?}", hdu.problems_within);
        }
    }
}

#[test]
fn a_framed_alert_is_recognised_by_its_name_and_its_schema_id() {
    let Some(dir) = samples() else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let bytes = std::fs::read(dir.join("7912941781254298.avro")).unwrap();
    let head = &bytes[..bytes.len().min(formats::SNIFF_WINDOW)];
    let len = bytes.len() as u64;
    assert_eq!(formats::sniff_named(head, len, "7912941781254298.avro"), Some("avro"));
    // One zero byte is no signature: under another name it is not an alert.
    assert_ne!(formats::sniff_named(head, len, "7912941781254298.bin"), Some("avro"));
    // Nor is a schema ID Qubero has no schema for.
    let mut other = head.to_vec();
    other[1..5].copy_from_slice(&1205u32.to_be_bytes());
    assert_ne!(formats::sniff_named(&other, len, "x.avro"), Some("avro"));
    let container = std::fs::read(dir.join("fakeAlert.avro")).unwrap();
    assert_eq!(formats::sniff(&container[..64], container.len() as u64), Some("avro"));
}

/// Every snappy block is followed by the CRC-32 of what it unpacks to, and the
/// sum is taken and passes; every block's sync marker is the header's.
#[test]
fn snappy_sums_and_sync_markers_check_out() {
    let Some(dir) = samples() else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let mut r = Reader::new(std::fs::read(dir.join("types-snappy.avro")).unwrap());
    let header_sync = r.child(&[], "header");
    let header_sync = r.child(&header_sync, "sync");
    let blocks = r.child(&[], "blocks");
    let blocks = r.kids(&blocks);
    assert_eq!(blocks.len(), 4);
    for block in blocks {
        let data = r.child(&block, "data");
        let crc = r.child(&data, "crc32");
        let v = r.ev.run_check(&r.doc, &crc).unwrap().expect("the check is taken");
        assert!(v.ok, "{block:?}: computed {}, stored {}", v.computed, v.stored);
        let sync = r.child(&block, "sync");
        assert!(r.ev.valid_of(&r.doc, &sync).unwrap().is_some_and(|v| v.ok), "{block:?}: {:?}", r.node(&sync).problem);
        assert_eq!(r.node(&sync).value, r.node(&header_sync).value);
    }
    // A sync marker that is not the header's is marked.
    let mut bytes = std::fs::read(dir.join("types.avro")).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    let mut r = Reader::new(bytes);
    let blocks = r.child(&[], "blocks");
    let last = r.kids(&blocks).pop().unwrap();
    let sync = r.child(&last, "sync");
    assert!(r.node(&sync).problem.is_some());
}
