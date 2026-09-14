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
//! into `md.0`, is checked here by reading both.
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
    let bytes = std::fs::read(sample(name)?).unwrap();
    let head = &bytes[..bytes.len().min(formats::SNIFF_WINDOW)];
    let template = formats::sniff(head, bytes.len() as u64).unwrap_or_else(|| panic!("{name} is not recognised"));
    let ev = Evaluator::new(formats::builtin(template).unwrap());
    Some(Open { doc: Document::new(MemSource(bytes.clone())), ev, bytes })
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
        ("steps.bp5/data.0", None),
    ];
    for (name, template) in expect {
        let bytes = std::fs::read(sample(name).unwrap()).unwrap();
        let head = &bytes[..bytes.len().min(formats::SNIFF_WINDOW)];
        assert_eq!(formats::sniff(head, bytes.len() as u64), template, "{name}");
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
    assert_eq!(f.int(&["footer", "footer", "subfiles"]), 0);
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
    assert_eq!(f.int(&["footer", "footer", "subfiles"]), 3);
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
