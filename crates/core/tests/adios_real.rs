//! Real ADIOS2 files, read against what ADIOS2 itself reads out of them.
//!
//! One small dataset written three times by ADIOS2 2.12.1 through its Python
//! API: as BP3 (a file of indices and a subfile of data), as a BP4 directory
//! and as a BP5 directory. Two steps; a 4 by 3 grid of doubles written in two
//! blocks a step, six floats, a single int32, a local array of five uint64s,
//! three int8s, a 2 by 2 grid of uint16s, two complex doubles, a string, and
//! three int16s written in the second step only; five attributes, one of them
//! a variable's. The values below are what `adios2.FileReader` gave for each.
//!
//! Every file of a directory is its own template, so what a file says about
//! another, a set's payload offset into `data.0` or an index record's offset
//! into `md.0`, is checked here by reading both. The BP5 directory's `data.0`
//! is not in the collection by itself: nothing in it says what it is, so
//! nothing reads it, and every file there has to be read by something. It is
//! in `steps.bp5.zip`, the directory stored in a ZIP, which reads as one
//! dataset: the records in `md.0` typed by the formats in `mmd.0`, and each
//! block's values placed in `data.0`.
//!
//! The files live in the sample collection rather than here. Point
//! `QUBERO_SAMPLES` at it, or keep it beside the repository as
//! `qubero-samples`. With neither, the test says so and passes.

use std::path::PathBuf;

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, NodeInfo, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

fn sample(name: &str) -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(set) = std::env::var("QUBERO_SAMPLES") {
        roots.extend(set.split(';').filter(|s| !s.is_empty()).map(PathBuf::from));
    }
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../qubero-samples"));
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../qubero-samples"));
    roots.into_iter().map(|r| r.join("adios").join(name)).find(|p| p.exists())
}

/// A sample opened with the template its bytes are recognised as.
struct Open {
    doc: Document<MemSource>,
    ev: Evaluator,
    bytes: Vec<u8>,
}

fn open(name: &str) -> Option<Open> {
    Some(open_bytes(std::fs::read(sample(name)?).unwrap(), name))
}

/// Bytes opened with the template they are recognised as, from their front and
/// their end.
fn open_bytes(bytes: Vec<u8>, name: &str) -> Open {
    let template = sniffed(&bytes).unwrap_or_else(|| panic!("{name} is not recognised"));
    let ev = Evaluator::new(formats::builtin(template).unwrap());
    Open { doc: Document::new(MemSource(bytes.clone())), ev, bytes }
}

/// What a file is recognised as, from the window at its front and the one at
/// its end, as the web app asks.
fn sniffed(bytes: &[u8]) -> Option<&'static str> {
    let head = &bytes[..bytes.len().min(formats::SNIFF_WINDOW)];
    let tail = &bytes[bytes.len().saturating_sub(formats::SNIFF_TAIL_WINDOW)..];
    formats::sniff_ends(head, tail, bytes.len() as u64)
}

macro_rules! open_or_skip {
    ($name:expr) => {
        match open($name) {
            Some(o) => o,
            None => {
                eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
                return;
            }
        }
    };
}

impl Open {
    fn node(&mut self, path: &[usize]) -> NodeInfo {
        self.ev.node(&self.doc, path).unwrap_or_else(|e| panic!("{path:?}: {e:?}"))
    }

    /// A path by field names and list indices, a name matching either the
    /// field's own name or the name a record is labelled with.
    fn at(&mut self, names: &[&str]) -> Vec<usize> {
        let mut path = vec![];
        for name in names {
            if let Ok(i) = name.parse::<usize>() {
                path.push(i);
                continue;
            }
            let n = self.node(&path).child_count as usize;
            let found = (0..n).find(|&i| {
                let p = [&path[..], &[i]].concat();
                let info = self.node(&p);
                // A field whose structure names itself reads `body MetaData`.
                info.name == *name || info.name == format!("[{i}] {name}") || info.name.starts_with(&format!("{name} "))
            });
            path.push(found.unwrap_or_else(|| panic!("no {name} under {path:?}")));
        }
        path
    }

    fn get(&mut self, names: &[&str]) -> NodeInfo {
        let p = self.at(names);
        self.node(&p)
    }

    fn int(&mut self, names: &[&str]) -> i128 {
        let v = self.get(names).value;
        match v {
            Value::Enum { raw, .. } => raw,
            other => other.as_int().unwrap_or_else(|| panic!("{names:?} is {other:?}")),
        }
    }

    fn text(&mut self, names: &[&str]) -> String {
        match self.get(names).value {
            Value::Str(s) => s,
            other => panic!("{names:?} is {other:?}"),
        }
    }

    /// Every number under a node, rows flattened, as reals.
    fn reals(&mut self, names: &[&str]) -> Vec<f64> {
        let p = self.at(names);
        let mut out = vec![];
        self.collect(&p, &mut out);
        out
    }

    fn collect(&mut self, path: &[usize], out: &mut Vec<f64>) {
        let info = self.node(path);
        match info.value {
            Value::Float(f) => out.push(f),
            Value::Int(i) => out.push(i as f64),
            Value::UInt(u) => out.push(u as f64),
            Value::Composite { count } => {
                for i in 0..count as usize {
                    self.collect(&[path, &[i]].concat(), out);
                }
            }
            other => panic!("{path:?} is {other:?}"),
        }
    }

    fn byte_offset(&mut self, names: &[&str]) -> u64 {
        self.get(names).offset_bits / 8
    }
}

/// The grid for step `s`, counted from nought: `(i + 0.25) * (s + 1)`.
fn temperature(s: usize) -> Vec<f64> {
    (0..12).map(|i| (i as f64 + 0.25) * (s as f64 + 1.0)).collect()
}

fn pressure(s: usize) -> Vec<f64> {
    (0..6).map(|i| i as f64 * 0.5 - s as f64).collect()
}

fn ids(s: usize) -> Vec<f64> {
    (0..5u64).map(|i| (i * (1 << 40) + s as u64) as f64).collect()
}

#[test]
fn every_file_of_the_dataset_is_told_apart() {
    let Some(_) = sample("steps.bp4/md.idx") else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let expect = [
        ("steps_bp3.bp", Some("adiosbp3")),
        ("steps_bp3.bp.dir/steps_bp3.bp.0", Some("adiosbp3")),
        ("steps.bp4/md.idx", Some("adiosbp4idx")),
        ("steps.bp4/md.0", Some("adiosbp4md")),
        ("steps.bp4/data.0", Some("adiosbp4data")),
        ("steps.bp5/md.idx", Some("adiosbp5idx")),
        ("steps.bp5/md.0", Some("adiosbp5md")),
        ("steps.bp5/mmd.0", Some("adiosbp5mmd")),
        ("steps.bp5.zip", Some("adioszip")),
    ];
    for (name, template) in expect {
        let bytes = std::fs::read(sample(name).unwrap()).unwrap();
        let head = &bytes[..bytes.len().min(formats::SNIFF_WINDOW)];
        assert_eq!(formats::sniff(head, bytes.len() as u64), template, "{name}");
        // A BP3 file too long for its footer to be seen is known by its front:
        // a process group, or for the file of indices alone a group index.
        if name.starts_with("steps_bp3") {
            assert_eq!(formats::sniff(&bytes[..128], 1 << 32), template, "{name} by its front");
        }
    }
}

/// The values of every block in a BP4 data file, read from the block headers
/// with no index, are the values ADIOS2 reads.
#[test]
fn a_bp4_data_file_holds_what_adios2_reads() {
    let mut f = open_or_skip!("steps.bp4/data.0");
    let groups = f.get(&["process_groups"]);
    assert_eq!(groups.child_count, 2);
    assert_eq!(groups.offset_bits + groups.size_bits, f.bytes.len() as u64 * 8);
    for s in 0..2 {
        let g = s.to_string();
        let g = g.as_str();
        assert_eq!(f.int(&["process_groups", g, "step"]), s as i128 + 1);
        // The grid is written in two blocks of two rows each.
        let mut grid = f.reals(&["process_groups", g, "variables", "0", "values"]);
        grid.extend(f.reals(&["process_groups", g, "variables", "1", "values"]));
        assert_eq!(grid, temperature(s));
        assert_eq!(f.get(&["process_groups", g, "variables", "0", "values"]).child_count, 2);
        assert_eq!(f.reals(&["process_groups", g, "variables", "pressure", "values"]), pressure(s));
        assert_eq!(f.reals(&["process_groups", g, "variables", "step_count", "values"]), [10.0 + s as f64]);
        assert_eq!(f.reals(&["process_groups", g, "variables", "ids", "values"]), ids(s));
        assert_eq!(f.reals(&["process_groups", g, "variables", "small", "values"]), [-1.0, 0.0, 1.0 + s as f64]);
        assert_eq!(f.reals(&["process_groups", g, "variables", "u16", "values"]), [1.0, 2.0, 3.0, 65535.0 - s as f64]);
        assert_eq!(f.reals(&["process_groups", g, "variables", "wave", "values"]), [1.0, 2.0, -3.5, s as f64]);
        assert_eq!(f.text(&["process_groups", g, "variables", "label", "values", "text"]), format!("step {s}"));
    }
    assert_eq!(f.get(&["process_groups", "0", "variables"]).child_count, 9);
    assert_eq!(f.reals(&["process_groups", "1", "variables", "late", "values"]), [-7.0, 8.0, 9.0]);
    // Attributes are written once, in the first group.
    assert_eq!(f.get(&["process_groups", "0", "attributes"]).child_count, 5);
    assert_eq!(f.get(&["process_groups", "1", "attributes"]).child_count, 0);
    assert_eq!(f.reals(&["process_groups", "0", "attributes", "grid", "value", "values"]), [4.0, 3.0]);
    assert_eq!(f.reals(&["process_groups", "0", "attributes", "scale", "value", "values"]), [1.5]);
    assert_eq!(f.text(&["process_groups", "0", "attributes", "units", "value", "text"]), "K");
    assert_eq!(f.text(&["process_groups", "0", "attributes", "temperature/units", "value", "text"]), "kelvin");
    assert_eq!(f.text(&["process_groups", "0", "attributes", "names", "value", "strings", "1", "text"]), "beta");
}

/// A BP4 `md.0` indexes each step's blocks, and each set's payload offset is
/// where `data.0` has that block's values.
#[test]
fn a_bp4_metadata_file_indexes_the_data_file() {
    let mut md = open_or_skip!("steps.bp4/md.0");
    let mut data = open("steps.bp4/data.0").unwrap();
    assert_eq!(md.get(&["steps"]).child_count, 2);
    assert_eq!(md.get(&["steps", "0", "variables_index", "entries"]).child_count, 8);
    assert_eq!(md.get(&["steps", "1", "variables_index", "entries"]).child_count, 9);
    assert_eq!(md.get(&["steps", "0", "attributes_index", "entries"]).child_count, 5);
    assert_eq!(md.get(&["steps", "1", "attributes_index", "entries"]).child_count, 0);
    for s in 0..2 {
        let step = s.to_string();
        let step = step.as_str();
        let sets = ["steps", step, "variables_index", "entries", "temperature", "sets"];
        assert_eq!(md.get(&sets).child_count, 2);
        for b in 0..2 {
            let set = [&sets[..], &[if b == 0 { "0" } else { "1" }]].concat();
            assert_eq!(md.int(&[&set[..], &["step"]].concat()), s as i128 + 1);
            let rows = &temperature(s)[b * 6..b * 6 + 6];
            let minmax = [&set[..], &["characteristics", "minmax", "body"]].concat();
            assert_eq!(md.reals(&[&minmax[..], &["min"]].concat()), [rows[0]]);
            assert_eq!(md.reals(&[&minmax[..], &["max"]].concat()), [rows[5]]);
            let offset = md.int(&[&set[..], &["characteristics", "payload offset", "body"]].concat()) as u64;
            let group = if s == 0 { "0" } else { "1" };
            let block = if b == 0 { "0" } else { "1" };
            assert_eq!(data.byte_offset(&["process_groups", group, "variables", block, "values"]), offset);
        }
        let value = ["steps", step, "variables_index", "entries", "step_count", "sets", "0", "characteristics", "value", "body"];
        assert_eq!(md.int(&value), 10 + s as i128);
        let label = ["steps", step, "variables_index", "entries", "label", "sets", "0", "characteristics", "value", "body", "text"];
        assert_eq!(md.text(&label), format!("step {s}"));
    }
    let attrs = ["steps", "0", "attributes_index", "entries"];
    let value = |name: &'static str| [&attrs[..], &[name, "sets", "0", "characteristics", "value", "body"]].concat();
    assert_eq!(md.reals(&value("grid")), [4.0, 3.0]);
    assert_eq!(md.reals(&value("scale")), [1.5]);
    assert_eq!(md.text(&[&value("temperature/units")[..], &["text"]].concat()), "kelvin");
    assert_eq!(md.text(&[&value("names")[..], &["0", "text"]].concat()), "alpha");
}

/// Each record of `md.idx` says where its step's indices start in `md.0`.
#[test]
fn a_bp4_index_file_places_each_step_in_the_metadata_file() {
    let mut idx = open_or_skip!("steps.bp4/md.idx");
    let mut md = open("steps.bp4/md.0").unwrap();
    assert_eq!(idx.get(&["steps"]).child_count, 2);
    for s in 0..2 {
        let step = s.to_string();
        let step = step.as_str();
        assert_eq!(idx.int(&["steps", step, "step"]), s as i128 + 1);
        for (field, index) in [("pg_index_offset", "pg_index"), ("variables_index_offset", "variables_index"), ("attributes_index_offset", "attributes_index")] {
            assert_eq!(idx.int(&["steps", step, field]) as u64, md.byte_offset(&["steps", step, index]), "{field}");
        }
        let end = md.get(&["steps", step]);
        assert_eq!(idx.int(&["steps", step, "step_end_offset"]) as u64, (end.offset_bits + end.size_bits) / 8);
    }
}

/// A BP3 file holding its data reads it twice over: in the process groups in
/// the order it was written, and from each set of the index by its payload
/// offset. Both are the same bytes, and the values ADIOS2 reads.
#[test]
fn a_bp3_file_places_each_block_from_its_index() {
    let mut f = open_or_skip!("steps_bp3.bp.dir/steps_bp3.bp.0");
    assert_eq!(f.int(&["footer", "footer", "flags"]), 0);
    assert_eq!(f.int(&["footer", "footer", "bp_version"]), 3);
    assert_eq!(f.get(&["process_groups"]).child_count, 2);
    assert_eq!(f.get(&["pg_index", "pg_index", "entries"]).child_count, 2);
    let entries = ["variables_index", "variables_index", "entries"];
    assert_eq!(f.get(&entries).child_count, 9);
    // BP3 keeps every step's blocks in one entry: four sets of the grid.
    let sets = [&entries[..], &["temperature", "sets"]].concat();
    assert_eq!(f.get(&sets).child_count, 4);
    let mut grid = vec![];
    for b in 0..4 {
        let values = [&sets[..], &[["0", "1", "2", "3"][b], "values", "values"]].concat();
        grid.extend(f.reals(&values));
        let group = if b < 2 { "0" } else { "1" };
        let block = if b % 2 == 0 { "0" } else { "1" };
        assert_eq!(f.byte_offset(&values), f.byte_offset(&["process_groups", group, "variables", block, "values"]));
    }
    assert_eq!(grid, [temperature(0), temperature(1)].concat());
    for (name, expect) in [("pressure", [pressure(0), pressure(1)].concat()), ("ids", [ids(0), ids(1)].concat())] {
        let mut got = vec![];
        for set in ["0", "1"] {
            got.extend(f.reals(&[&entries[..], &[name, "sets", set, "values", "values"]].concat()));
        }
        assert_eq!(got, expect, "{name}");
    }
    assert_eq!(f.reals(&[&entries[..], &["late", "sets", "0", "values", "values"]].concat()), [-7.0, 8.0, 9.0]);
    assert_eq!(f.text(&[&entries[..], &["label", "sets", "1", "values", "values", "text"]].concat()), "step 1");
    assert_eq!(f.reals(&["process_groups", "1", "variables", "wave", "values"]), [1.0, 2.0, -3.5, 1.0]);
    let attrs = ["attributes_index", "attributes_index", "entries"];
    assert_eq!(f.get(&attrs).child_count, 5);
    assert_eq!(f.reals(&[&attrs[..], &["grid", "sets", "0", "characteristics", "value", "body"]].concat()), [4.0, 3.0]);
    assert_eq!(f.reals(&["process_groups", "0", "attributes", "scale", "value", "values"]), [1.5]);
}

/// A BP3 file of indices alone says its data is in subfiles, and places
/// nothing from its offsets.
#[test]
fn a_bp3_file_of_indices_alone_places_nothing() {
    let mut f = open_or_skip!("steps_bp3.bp");
    assert_eq!(f.int(&["footer", "footer", "flags"]), 3);
    assert_eq!(f.get(&["process_groups"]).child_count, 0);
    let entries = ["variables_index", "variables_index", "entries"];
    assert_eq!(f.get(&entries).child_count, 9);
    assert!(f.get(&[&entries[..], &["temperature", "sets", "0", "values"]].concat()).absent);
    assert_eq!(f.int(&[&entries[..], &["step_count", "sets", "1", "characteristics", "value", "body"]].concat()), 11);
    assert_eq!(f.get(&["attributes_index", "attributes_index", "entries"]).child_count, 5);
}

/// A BP5 index, its metadata and its formats, each checked against the next.
#[test]
fn a_bp5_directory_reads_to_its_ffs_records() {
    let mut idx = open_or_skip!("steps.bp5/md.idx");
    let mut md = open("steps.bp5/md.0").unwrap();
    let mut mmd = open("steps.bp5/mmd.0").unwrap();
    assert_eq!(idx.get(&["records"]).child_count, 3);
    assert_eq!(idx.int(&["records", "writer map", "body", "writer_count"]), 1);
    assert_eq!(md.get(&[]).child_count, 2);
    let mut data_offsets = vec![];
    for (record, step) in [("1", "0"), ("2", "1")] {
        let offset = idx.int(&["records", record, "body", "metadata_offset"]) as u64;
        let size = idx.int(&["records", record, "body", "metadata_size"]) as u64;
        let placed = md.get(&[step]);
        assert_eq!((placed.offset_bits / 8, placed.size_bits / 8), (offset, size));
        data_offsets.push(idx.int(&["records", record, "body", "writers", "0", "data_offset"]));
    }
    assert_eq!(data_offsets, [0, 4096]);
    // Only the first step carries attributes.
    assert!(!md.get(&["0", "blocks", "attributes"]).absent);
    assert!(md.get(&["1", "blocks", "attributes"]).absent);
    // The metadata of the first step is written in the format `mmd.0` holds
    // first, by ID.
    let id_at = md.byte_offset(&["0", "blocks", "metadata", "format_id"]) as usize;
    let format_at = mmd.byte_offset(&["0", "id"]) as usize;
    assert_eq!(md.bytes[id_at..id_at + 12], mmd.bytes[format_at..format_at + 12]);
    assert_eq!(mmd.get(&[]).child_count, 3);
    let fields = ["0", "description", "subformats", "0", "body", "fields"];
    let names: Vec<String> = (0..mmd.get(&fields).child_count as usize)
        .map(|i| mmd.text(&[&fields[..], &[&i.to_string(), "name", "name"]].concat()))
        .collect();
    for want in ["BitFieldCount", "BitField", "DataBlockSize", "BPG_8_10_temperature", "BPg_step_count", "BPL_8_8_ids", "BPg_label"] {
        assert!(names.iter().any(|n| n == want), "{want} not in {names:?}");
    }
    assert_eq!(mmd.text(&["0", "description", "subformats", "1", "body", "name", "name"]), "MetaArrayMM8");
    // The third format adds `late`, which the second step wrote.
    let late = ["2", "description", "subformats", "0", "body", "fields"];
    let n = mmd.get(&late).child_count as usize;
    assert!((0..n).any(|i| mmd.text(&[&late[..], &[&i.to_string(), "name", "name"]].concat()).ends_with("_late")));
}

/// A ZIP of stored files, written the way the web app writes a dropped folder:
/// local records in the order given, then the central directory.
fn stored_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let at = out.len() as u32;
        let crc = qubero_core::checksum::crc32(data);
        let fixed = |sig: u32, v: &mut Vec<u8>| {
            v.extend(sig.to_le_bytes());
            if sig == 0x0201_4b50 {
                v.extend(20u16.to_le_bytes());
            }
            v.extend(20u16.to_le_bytes());
            v.extend([0u8; 4]);
            v.extend(0u16.to_le_bytes());
            v.extend(0x21u16.to_le_bytes());
            v.extend(crc.to_le_bytes());
            v.extend((data.len() as u32).to_le_bytes());
            v.extend((data.len() as u32).to_le_bytes());
            v.extend((name.len() as u16).to_le_bytes());
            v.extend(0u16.to_le_bytes());
        };
        fixed(0x0403_4b50, &mut out);
        out.extend(name.as_bytes());
        out.extend(*data);
        fixed(0x0201_4b50, &mut central);
        // The comment's length, the disk, and the two attribute fields.
        central.extend([0u8; 10]);
        central.extend(at.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let (cd_at, cd_len) = (out.len() as u32, central.len() as u32);
    out.extend(central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0u8; 4]);
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((files.len() as u16).to_le_bytes());
    out.extend(cd_len.to_le_bytes());
    out.extend(cd_at.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out
}

/// The field of a BP5 record's data that holds the variable `name`, which BP5
/// names with its shape, element size and type in front: `BPG_8_10_temperature`.
fn variable_field(f: &mut Open, data: &[&str], name: &str) -> Option<String> {
    let p = f.at(data);
    let n = f.node(&p).child_count as usize;
    (0..n).map(|i| f.node(&[&p[..], &[i]].concat()).name).find(|field| field.starts_with("BP") && field.ends_with(&format!("_{name}")))
}

/// What ADIOS2 reads of one block: counts, starts, bounds and values.
struct Block {
    count: Vec<f64>,
    start: Option<Vec<f64>>,
    bounds: Option<(f64, f64)>,
    values: Vec<f64>,
}

fn block(count: &[u64], start: Option<&[u64]>, bounds: Option<(f64, f64)>, values: Vec<f64>) -> Block {
    let reals = |v: &[u64]| v.iter().map(|&x| x as f64).collect();
    Block { count: reals(count), start: start.map(reals), bounds, values }
}

fn bounds(v: &[f64]) -> Option<(f64, f64)> {
    Some((v.iter().cloned().fold(f64::MAX, f64::min), v.iter().cloned().fold(f64::MIN, f64::max)))
}

/// Where the dataset's files are: `dataset` itself when the archive stores
/// every file, and the stream they are joined in, which keeps the node's name,
/// when it packed any.
fn dataset_at(f: &mut Open) -> Vec<&'static str> {
    match joined(f) {
        true => vec!["dataset", "dataset"],
        false => vec!["dataset"],
    }
}

/// Whether the dataset's files are joined into one stream, whose one child
/// keeps the stream's name, rather than placed in the archive.
fn joined(f: &mut Open) -> bool {
    let at = f.at(&["dataset"]);
    f.node(&at).child_count == 1 && f.node(&[&at[..], &[0]].concat()).name == "dataset"
}

/// Checks every block of the variable `name` in step `s`'s metadata against
/// what ADIOS2 reads.
fn check_variable(f: &mut Open, s: usize, name: &str, shape: &str, data_type: &str, blocks: &[Block], placed: bool) {
    let step = s.to_string();
    let dataset = dataset_at(f);
    let data = [&dataset[..], &["md_0", "0", &step, "blocks", "metadata", "data"]].concat();
    let field = variable_field(f, &data, name).unwrap_or_else(|| panic!("no {name} in step {s}"));
    let var = [&data[..], &[field.as_str()]].concat();
    assert_eq!(f.text(&[&var[..], &["variable", "variable"]].concat()), name);
    let shape_value = f.get(&[&var[..], &["shape", "shape"]].concat()).value;
    assert!(matches!(&shape_value, Value::Enum { name: Some(n), .. } if n == shape), "{name} shape {shape_value:?}");
    let type_value = f.get(&[&var[..], &["data_type"]].concat()).value;
    assert!(matches!(&type_value, Value::Enum { name: Some(n), .. } if n == data_type), "{name} type {type_value:?}");
    assert_eq!(f.get(&[&var[..], &["blocks"]].concat()).child_count as usize, blocks.len(), "{name} blocks in step {s}");
    for (b, want) in blocks.iter().enumerate() {
        let b = b.to_string();
        let at = [&var[..], &["blocks", &b]].concat();
        assert_eq!(f.reals(&[&at[..], &["count"]].concat()), want.count, "{name} count");
        let p = f.at(&at);
        let children: Vec<NodeInfo> = (0..f.node(&p).child_count as usize).map(|i| f.node(&[&p[..], &[i]].concat())).collect();
        match &want.start {
            Some(start) => assert_eq!(&f.reals(&[&at[..], &["start"]].concat()), start, "{name} start"),
            None => assert!(children.iter().all(|c| c.name != "start" || c.absent), "{name} is a local array and has no start"),
        }
        match want.bounds {
            Some((min, max)) => {
                assert_eq!(f.reals(&[&at[..], &["min"]].concat()), [min], "{name} min");
                assert_eq!(f.reals(&[&at[..], &["max"]].concat()), [max], "{name} max");
            }
            None => assert!(f.get(&[&at[..], &["min"]].concat()).absent, "{name} has no bounds"),
        }
        match placed {
            true => assert_eq!(f.reals(&[&at[..], &["values"]].concat()), want.values, "{name} values in step {s}"),
            false => assert!(children.iter().all(|c| c.name != "values"), "{name} places no values"),
        }
    }
}

/// A BP5 directory stored in a ZIP reads as one dataset: every variable of
/// every step as ADIOS2 reads it, with its shape and type from its name, its
/// blocks' counts, starts and bounds from `md.0` in the formats `mmd.0` holds,
/// and each block's values placed in `data.0` from where `md.idx` says the
/// step's data starts.
#[test]
fn a_bp5_directory_in_a_zip_reads_each_variable_as_adios2_does() {
    let mut f = open_or_skip!("steps.bp5.zip");
    check_dataset(&mut f);
}

/// Every variable and attribute of `steps.bp5` read out of an archive of it,
/// as ADIOS2 reads them.
fn check_dataset(f: &mut Open) {
    let ds = dataset_at(f);
    assert_eq!(f.get(&[&ds[..], &["md_0", "0"]].concat()).child_count, 2);
    for s in 0..2 {
        let step = s.to_string();
        assert_eq!(f.int(&[&ds[..], &["md_0", "0", &step, "data_offset"]].concat()), [0, 4096][s]);
        let t = temperature(s);
        let halves = [block(&[2, 3], Some(&[0, 0]), bounds(&t[..6]), t[..6].to_vec()), block(&[2, 3], Some(&[2, 0]), bounds(&t[6..]), t[6..].to_vec())];
        check_variable(f, s, "temperature", "global array", "double", &halves, true);
        let p = pressure(s);
        check_variable(f, s, "pressure", "global array", "float", &[block(&[6], Some(&[0]), bounds(&p), p.clone())], true);
        let i = ids(s);
        check_variable(f, s, "ids", "local array", "uint64", &[block(&[5], None, bounds(&i), i.clone())], true);
        let small = vec![-1.0, 0.0, 1.0 + s as f64];
        check_variable(f, s, "small", "global array", "char", &[block(&[3], Some(&[0]), None, small)], true);
        let u16s = vec![1.0, 2.0, 3.0, 65535.0 - s as f64];
        check_variable(f, s, "u16", "global array", "uint16", &[block(&[2, 2], Some(&[0, 0]), bounds(&u16s), u16s.clone())], true);
        let wave = vec![1.0, 2.0, -3.5, s as f64];
        check_variable(f, s, "wave", "global array", "complex double", &[block(&[2], Some(&[0]), None, wave)], true);
        let data = [&ds[..], &["md_0", "0", &step, "blocks", "metadata", "data"]].concat();
        assert_eq!(f.int(&[&data[..], &["BPg_step_count"]].concat()), 10 + s as i128);
        assert_eq!(f.text(&[&data[..], &["BPg_label", "text", "text"]].concat()), format!("step {s}"));
    }
    // `late` is written in the second step only, in the third format.
    assert!(variable_field(f, &[&ds[..], &["md_0", "0", "0", "blocks", "metadata", "data"]].concat(), "late").is_none());
    check_variable(f, 1, "late", "global array", "int16", &[block(&[3], Some(&[0]), Some((-7.0, 9.0)), vec![-7.0, 8.0, 9.0])], true);

    // The attributes, in the first step's attribute block, each named with its
    // type in front.
    let attrs = [&ds[..], &["md_0", "0", "0", "blocks", "attributes", "data"]].concat();
    let prim = [&attrs[..], &["PrimAttrs", "values", "values"]].concat();
    assert_eq!(f.get(&prim).child_count, 2);
    let mut numbers = std::collections::BTreeMap::new();
    for i in ["0", "1"] {
        let a = [&prim[..], &[i]].concat();
        let name = f.text(&[&a[..], &["attribute", "attribute"]].concat());
        numbers.insert(name, f.reals(&[&a[..], &["values"]].concat()));
    }
    assert_eq!(numbers["grid"], [4.0, 3.0]);
    assert_eq!(numbers["scale"], [1.5]);
    let strs = [&attrs[..], &["StrAttrs", "values", "values"]].concat();
    assert_eq!(f.get(&strs).child_count, 3);
    let mut texts = std::collections::BTreeMap::new();
    for i in ["0", "1", "2"] {
        let a = [&strs[..], &[i]].concat();
        let name = f.text(&[&a[..], &["attribute", "attribute"]].concat());
        let values = [&a[..], &["Values", "values", "values"]].concat();
        let n = f.get(&values).child_count as usize;
        let got: Vec<String> = (0..n).map(|k| f.text(&[&values[..], &[&k.to_string(), "text", "text"]].concat())).collect();
        texts.insert(name, got);
    }
    assert_eq!(texts["names"], ["alpha", "beta"]);
    assert_eq!(texts["temperature/units"], ["kelvin"]);
    assert_eq!(texts["units"], ["K"]);
}

/// A byte of the dataset is found as what the dataset reads it as, not as the
/// archive's bytes: a byte of `md.0` is a field of its record, and a byte of
/// `data.0` is a value of the block the metadata placed there.
#[test]
fn a_byte_of_a_bp5_zip_is_found_as_the_dataset_reads_it() {
    let mut f = open_or_skip!("steps.bp5.zip");
    let data = ["dataset", "md_0", "0", "0", "blocks", "metadata", "data"];
    let count = f.get(&[&data[..], &["BitFieldCount"]].concat());
    let found = f.ev.locate(&f.doc, count.offset_bits + 8).unwrap();
    assert_eq!(found, f.at(&[&data[..], &["BitFieldCount"]].concat()));
    let value = ["dataset", "md_0", "0", "1", "blocks", "metadata", "data", "BPG_8_10_temperature", "blocks", "1", "values", "values", "1", "2"];
    let placed = f.get(&value);
    assert_eq!(placed.value, Value::Float(22.5));
    let found = f.ev.locate(&f.doc, placed.offset_bits + 3).unwrap();
    assert_eq!(f.node(&found).value, Value::Float(22.5));
    assert_eq!(f.node(&found).offset_bits, placed.offset_bits);
}

/// `md.0` opened alone reads to its FFS records, and each record's data says
/// its formats are in `mmd.0` rather than failing the file.
#[test]
fn a_bp5_metadata_file_alone_says_its_formats_are_in_mmd0() {
    let mut md = open_or_skip!("steps.bp5/md.0");
    for s in ["0", "1"] {
        let data = md.get(&[s, "blocks", "metadata", "data"]);
        let length = md.int(&[s, "blocks", "metadata", "data_length"]) as u64;
        assert_eq!(data.size_bits, length * 8);
        let doc = data.doc.unwrap_or_default();
        assert!(doc.contains("mmd.0"), "step {s}: {doc:?}");
        assert!(data.type_name.starts_with("FFS format 0200"), "{}", data.type_name);
    }
}

/// Without `data.0` the dataset still reads each block's counts and bounds,
/// and places no values; with the files under a folder, the names match by
/// their last part. Without `mmd.0` the metadata says where its formats are.
#[test]
fn a_bp5_zip_without_its_data_file_reads_the_blocks_without_values() {
    let Some(dir) = sample("steps.bp5") else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let read = |n: &str| std::fs::read(dir.join(n)).unwrap();
    let (idx, mmd, md) = (read("md.idx"), read("mmd.0"), read("md.0"));
    let zip = stored_zip(&[("steps.bp5/md.idx", &idx), ("steps.bp5/mmd.0", &mmd), ("steps.bp5/md.0", &md)]);
    let mut f = open_bytes(zip, "a zip without data.0");
    assert_eq!(f.ev.template().name, "adioszip");
    let t = temperature(0);
    let halves = [block(&[2, 3], Some(&[0, 0]), bounds(&t[..6]), vec![]), block(&[2, 3], Some(&[2, 0]), bounds(&t[6..]), vec![])];
    check_variable(&mut f, 0, "temperature", "global array", "double", &halves, false);
    let zip = stored_zip(&[("md.idx", &idx), ("md.0", &md)]);
    let mut f = open_bytes(zip, "a zip without mmd.0");
    let data = f.get(&["dataset", "md_0", "0", "0", "blocks", "metadata", "data"]);
    assert!(data.doc.unwrap_or_default().contains("mmd.0"));
}

/// The files of `steps.bp5` as the stored sample archive holds them, in its
/// order: `md.idx`, `mmd.0`, `md.0` and `data.0`, each under the folder's name.
fn bp5_files() -> Option<Vec<(String, Vec<u8>)>> {
    let bytes = std::fs::read(sample("steps.bp5.zip")?).unwrap();
    let (mut out, mut at) = (Vec::new(), 0usize);
    while bytes[at..at + 4] == *b"PK\x03\x04" {
        let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]) as usize;
        let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
        let (size, name_length, extra_length) = (u32_at(at + 18), u16_at(at + 26), u16_at(at + 28));
        let name = String::from_utf8(bytes[at + 30..at + 30 + name_length].to_vec()).unwrap();
        let data_at = at + 30 + name_length + extra_length;
        out.push((name, bytes[data_at..data_at + size].to_vec()));
        at = data_at + size;
    }
    Some(out)
}

macro_rules! files_or_skip {
    () => {
        match bp5_files() {
            Some(files) => files,
            None => {
                eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
                return;
            }
        }
    };
}

/// How a test archive writes an entry.
#[derive(Clone, Copy, PartialEq)]
enum Method {
    Stored,
    /// Deflated, with its sizes and sum in the header.
    Deflated,
    /// Deflated as a stream: nought in the header, and the sizes and sum in a
    /// data descriptor after the data, as a writer piping its output does.
    Streamed,
}

/// A ZIP of `files`, each written as `Method` says, in the order given, then
/// the central directory and the end record.
fn zip_of(files: &[(&str, &[u8], Method)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data, how) in files {
        let at = out.len() as u32;
        let crc = qubero_core::checksum::crc32(data);
        let packed = match how {
            Method::Stored => data.to_vec(),
            _ => miniz_oxide::deflate::compress_to_vec(data, 6),
        };
        let (flags, method): (u16, u16) = match how {
            Method::Stored => (0, 0),
            Method::Deflated => (0, 8),
            Method::Streamed => (8, 8),
        };
        let streamed = *how == Method::Streamed;
        let header = |v: &mut Vec<u8>, sizes: bool| {
            v.extend(flags.to_le_bytes());
            v.extend(method.to_le_bytes());
            v.extend(0u16.to_le_bytes());
            v.extend(0x21u16.to_le_bytes());
            let (c, p, u) = if sizes { (crc, packed.len() as u32, data.len() as u32) } else { (0, 0, 0) };
            v.extend(c.to_le_bytes());
            v.extend(p.to_le_bytes());
            v.extend(u.to_le_bytes());
            v.extend((name.len() as u16).to_le_bytes());
            v.extend(0u16.to_le_bytes());
        };
        out.extend(0x0403_4b50u32.to_le_bytes());
        out.extend(20u16.to_le_bytes());
        header(&mut out, !streamed);
        out.extend(name.as_bytes());
        out.extend(&packed);
        if streamed {
            out.extend(0x0807_4b50u32.to_le_bytes());
            out.extend(crc.to_le_bytes());
            out.extend((packed.len() as u32).to_le_bytes());
            out.extend((data.len() as u32).to_le_bytes());
        }
        central.extend(0x0201_4b50u32.to_le_bytes());
        central.extend(20u16.to_le_bytes());
        central.extend(20u16.to_le_bytes());
        header(&mut central, true);
        // The comment's length, the disk, and the two attribute fields.
        central.extend([0u8; 10]);
        central.extend(at.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let (cd_at, cd_len) = (out.len() as u32, central.len() as u32);
    out.extend(central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0u8; 4]);
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((files.len() as u16).to_le_bytes());
    out.extend(cd_len.to_le_bytes());
    out.extend(cd_at.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out
}

/// `bytes` with `n` bytes that do not compress added at the end. A data file
/// longer than it needs to be reads the same: every block is at the offset the
/// metadata gives, and nothing reads past the last.
fn padded(bytes: &[u8], n: usize) -> Vec<u8> {
    let mut out = bytes.to_vec();
    let mut x = 0x9e37_79b9_7f4a_7c15u64;
    out.extend((0..n).map(|_| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x as u8
    }));
    out
}

/// A BP5 directory whose files a ZIP deflated reads as the stored one does:
/// every variable, block, bound and value, from the files unpacked and joined.
#[test]
fn a_bp5_directory_in_a_deflated_zip_reads_each_variable_as_adios2_does() {
    let files = files_or_skip!();
    let entries: Vec<_> = files.iter().map(|(n, d)| (n.as_str(), d.as_slice(), Method::Deflated)).collect();
    let zip = zip_of(&entries);
    let head = &zip[..zip.len().min(formats::SNIFF_WINDOW)];
    assert_eq!(formats::sniff(head, zip.len() as u64), Some("adioszip"), "a deflated md.idx is told from the front");
    let mut f = open_bytes(zip, "a deflated zip");
    assert!(joined(&mut f), "the files are joined into one stream");
    check_dataset(&mut f);
}

/// Entries written as streams, whose headers say nothing of their sizes, read
/// the same: each file is as long as the descriptor after it says.
#[test]
fn a_bp5_directory_in_a_streamed_zip_reads_each_variable_as_adios2_does() {
    let files = files_or_skip!();
    let entries: Vec<_> = files.iter().map(|(n, d)| (n.as_str(), d.as_slice(), Method::Streamed)).collect();
    let mut f = open_bytes(zip_of(&entries), "a streamed zip");
    assert_eq!(f.ev.template().name, "adioszip");
    check_dataset(&mut f);
}

/// A data file larger than the window recognition reads, written first as a
/// writer that lists the folder in name order does: the front shows nothing of
/// the dataset, and the central directory names it. Stored or deflated, with
/// the other files deflated after it.
#[test]
fn a_bp5_zip_with_a_large_data_file_first_is_told_from_its_central_directory() {
    let files = files_or_skip!();
    let data = padded(&files[3].1, 3 * formats::SNIFF_WINDOW);
    for data_method in [Method::Stored, Method::Deflated] {
        let mut entries = vec![(files[3].0.as_str(), data.as_slice(), data_method)];
        entries.extend(files[..3].iter().map(|(n, d)| (n.as_str(), d.as_slice(), Method::Deflated)));
        let zip = zip_of(&entries);
        let head = &zip[..formats::SNIFF_WINDOW];
        assert_eq!(formats::sniff(head, zip.len() as u64), Some("zip"), "the front is all data.0");
        assert_eq!(sniffed(&zip), Some("adioszip"));
        let mut f = open_bytes(zip, "a zip with data.0 first");
        check_dataset(&mut f);
    }
    // And with every file stored, the dataset is read where the archive keeps
    // it, so a byte of the archive is still a field of the dataset.
    let mut entries = vec![(files[3].0.as_str(), data.as_slice(), Method::Stored)];
    entries.extend(files[..3].iter().map(|(n, d)| (n.as_str(), d.as_slice(), Method::Stored)));
    let mut f = open_bytes(zip_of(&entries), "a stored zip with data.0 first");
    assert!(!joined(&mut f), "every file is stored, so none is joined");
    check_dataset(&mut f);
}

/// Archives of `steps.bp5` made by other writers read as the stored one does:
/// Python's `zipfile` deflating every file in sorted order, `data.0` first,
/// and Info-ZIP's `zip -r`, which adds an entry for the folder itself and, on
/// Windows, a security descriptor to every file.
#[test]
fn a_bp5_directory_zipped_by_other_writers_reads_each_variable_as_adios2_does() {
    for name in ["steps.bp5.deflated.zip", "steps.bp5.info-zip.zip"] {
        let mut f = open_or_skip!(name);
        assert_eq!(f.ev.template().name, "adioszip", "{name}");
        assert!(joined(&mut f), "{name} deflates its files");
        check_dataset(&mut f);
    }
}

/// `zip -r` of a dataset whose data file is a megabyte, written first and
/// deflated to more than the front recognition reads: the central directory
/// names the dataset, and every value of both steps reads as ADIOS2 wrote it.
#[test]
fn a_bp5_zip_whose_data_file_hides_the_rest_from_the_front_reads_every_value() {
    let Some(path) = sample("grid.bp5.info-zip.zip") else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(formats::sniff(&bytes[..formats::SNIFF_WINDOW], bytes.len() as u64), Some("zip"));
    let mut f = open_bytes(bytes, "grid.bp5.info-zip.zip");
    assert_eq!(f.ev.template().name, "adioszip");
    let ds = dataset_at(&mut f);
    for s in 0..2 {
        let step = s.to_string();
        let data = [&ds[..], &["md_0", "0", &step, "blocks", "metadata", "data"]].concat();
        // One number written as an array of one, which is a global array.
        let n = 100.0 + s as f64;
        check_variable(&mut f, s, "count", "global array", "int32", &[block(&[1], Some(&[0]), Some((n, n)), vec![n])], true);
        let field = variable_field(&mut f, &data, "field").expect("a field variable");
        let block = [&data[..], &[field.as_str(), "blocks", "0"]].concat();
        assert_eq!(f.reals(&[&block[..], &["count"]].concat()), [256.0, 256.0]);
        let (min, max) = (s as f64, 65535.0 * 0.5 + s as f64);
        assert_eq!((f.reals(&[&block[..], &["min"]].concat()), f.reals(&[&block[..], &["max"]].concat())), (vec![min], vec![max]));
        let values = [&block[..], &["values", "values"]].concat();
        assert_eq!(f.get(&values).child_count, 256, "rows");
        for row in [0usize, 97, 255] {
            let want: Vec<f64> = (0..256).map(|c| (row * 256 + c) as f64 * 0.5 + s as f64).collect();
            assert_eq!(f.reals(&[&values[..], &[&row.to_string()]].concat()), want, "step {s} row {row}");
        }
    }
}

/// An archive of files that only share names with a dataset's is not taken
/// for one by its end, and one folder's `md.idx` does not make a dataset of
/// another folder's `mmd.0` and `md.0`.
#[test]
fn names_in_the_central_directory_make_a_dataset_only_together() {
    let big = padded(&[], 2 * formats::SNIFF_WINDOW);
    let zip = zip_of(&[("big.bin", &big, Method::Stored), ("a/md.idx", b"x", Method::Stored), ("b/mmd.0", b"x", Method::Stored), ("a/md.0", b"x", Method::Stored)]);
    assert_eq!(sniffed(&zip), Some("zip"));
    let zip = zip_of(&[("big.bin", &big, Method::Stored), ("a/md.idx", b"x", Method::Stored), ("a/mmd.0", b"x", Method::Stored), ("a/md.0", b"x", Method::Stored)]);
    assert_eq!(sniffed(&zip), Some("adioszip"));
    let zip = zip_of(&[("big.bin", &big, Method::Stored), ("store.zarr/.zgroup", b"{}", Method::Deflated)]);
    assert_eq!(sniffed(&zip), Some("zarrzip"));
}

/// A file of the dataset packed with a method nothing here unpacks stays bytes
/// at its entry in the archive, saying so, and the files that do read still
/// read: here `md.0` claims deflate64, so its records are not read, while the
/// index and the formats are.
#[test]
fn a_bp5_file_packed_with_a_method_not_unpacked_stays_bytes_saying_why() {
    let files = files_or_skip!();
    let entries: Vec<_> = files.iter().map(|(n, d)| (n.as_str(), d.as_slice(), Method::Deflated)).collect();
    let mut zip = zip_of(&entries);
    // The third local header is md.0's, and its method is at byte 8. The
    // central directory's copy is left: the dataset reads the local records.
    let mut at = 0usize;
    for _ in 0..2 {
        let size = u32::from_le_bytes(zip[at + 18..at + 22].try_into().unwrap()) as usize;
        let name = u16::from_le_bytes([zip[at + 26], zip[at + 27]]) as usize;
        at += 30 + name + size;
    }
    zip[at + 8..at + 10].copy_from_slice(&9u16.to_le_bytes());
    let mut f = open_bytes(zip, "a zip with a deflate64 md.0");
    let ds = dataset_at(&mut f);
    let unread = f.get(&[&ds[..], &["md_0", "md_0"]].concat());
    let doc = unread.doc.unwrap_or_default();
    assert!(doc.contains("deflate64"), "{doc:?} on {}", unread.type_name);
    assert_eq!(f.get(&[&ds[..], &["md_idx", "md_idx", "records"]].concat()).child_count, 3);
}
