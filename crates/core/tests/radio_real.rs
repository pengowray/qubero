//! Radio telescope files, read against what independent readers make of them.
//!
//! `tools/radio_reference.py` writes the reference: one fact a line, read
//! there with astropy, h5py, hdf5plugin and numpy, and read here through a
//! template. Its module doc lists the kinds of fact.
//!
//! Two sets of files are checked, each against a reference of its own:
//!
//! - The small ones in the sample collection (`psrfits/`, `sigproc/`,
//!   `guppi/` and `hdf5/filterbank-bitshuffle.h5`), against
//!   `tests/fixtures/radio/reference.tsv`. `tools/make_radio_samples.py` in
//!   the collection writes them; the PSRFITS ones are cut from the two
//!   observations below, and the rest are made from seeded random numbers.
//! - Whole observations, in `qubero2-extras/radio` beside the checkout or an
//!   ancestor of it, or wherever `QUBERO_RADIO` points, against the
//!   `radio_reference.tsv` in that folder: two PSRFITS files, two
//!   Breakthrough Listen filterbank files in HDF5, three Breakthrough Listen
//!   SIGPROC filterbank files and one GUPPI raw file, 395 bytes to 1.2 GB
//!   each. The Breakthrough Listen files state no licence, so none of them is
//!   cut into the collection.
//!
//! Without a folder the test that reads it says so and passes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

/// One line of a reference: what kind of fact, where in the file, the value.
struct Fact {
    kind: String,
    at: String,
    value: String,
}

/// A reference, by file, in the order the files were written.
fn reference(text: &str) -> BTreeMap<String, Vec<Fact>> {
    let mut out: BTreeMap<String, Vec<Fact>> = BTreeMap::new();
    for line in text.lines().filter(|l| !l.starts_with('#') && !l.is_empty()) {
        let [kind, file, at, value] = line.split('\t').collect::<Vec<_>>()[..] else { panic!("bad line {line}") };
        out.entry(file.into()).or_default().push(Fact { kind: kind.into(), at: at.into(), value: value.into() });
    }
    out
}

type Doc = Document<MemSource>;

/// The numbers a fact's place is written as, `3,0,15` and so on.
fn numbers(at: &str) -> Vec<usize> {
    at.split(',').map(|n| n.parse().unwrap_or_else(|_| panic!("not a number in {at}"))).collect()
}

/// Whether two values written as text are the same number, or the same text
/// where either is not a number. A float is compared to a part in 10^12,
/// since the two sides print one differently.
fn same(got: &str, want: &str) -> bool {
    match (got.trim().parse::<f64>(), want.trim().parse::<f64>()) {
        (Ok(a), Ok(b)) => a == b || (a - b).abs() <= 1e-12 * a.abs().max(b.abs()),
        _ => got.trim() == want.trim(),
    }
}

fn child(ev: &mut Evaluator, doc: &Doc, at: &[usize], name: &str) -> Option<Vec<usize>> {
    ev.child_named(doc, at, name).unwrap_or_else(|e| panic!("{name} under {at:?}: {e:?}"))
}

fn step(at: &[usize], i: usize) -> Vec<usize> {
    let mut p = at.to_vec();
    p.push(i);
    p
}

// ------------------------------------------------------------------ FITS

/// Where a unit's fields are: its cards, and its data.
const CARDS: usize = 0;
const DATA: usize = 3;
/// Where a table's rows are in its data.
const ROWS: usize = 1;

/// The value of the card with this keyword, as the text or the number it is.
fn card(ev: &mut Evaluator, doc: &Doc, hdu: usize, key: &str) -> Option<String> {
    let cards = [0, hdu, CARDS];
    let n = ev.node(doc, &cards).unwrap().child_count as usize;
    for i in 0..n {
        let at = step(&cards, i);
        let key_at = child(ev, doc, &at, "key")?;
        let Value::Str(k) = ev.node(doc, &key_at).unwrap().value else { continue };
        if k.trim() != key {
            continue;
        }
        let body = child(ev, doc, &at, "body")?;
        let value = child(ev, doc, &body, "value")?;
        return Some(match ev.node(doc, &value).unwrap().value {
            Value::Str(s) => s.trim().to_string(),
            // A quoted string: the text of its first piece.
            Value::Composite { .. } => {
                let parts = child(ev, doc, &value, "parts")?;
                let text = child(ev, doc, &step(&parts, 0), "text")?;
                match ev.node(doc, &text).unwrap().value {
                    Value::Str(s) => s.trim().to_string(),
                    other => panic!("{key}: {other:?}"),
                }
            }
            other => other.as_int().map(|n| n.to_string()).unwrap_or_else(|| panic!("{key}: {other:?}")),
        });
    }
    None
}

/// One cell of one row of a table: `rows[row].cells[column]`.
fn cell(hdu: usize, row: usize, column: usize) -> Vec<usize> {
    vec![0, hdu, DATA, ROWS, row, 0, column]
}

/// The run of samples in a row's `DATA` cell, which is the row's last cell.
fn search_samples(ev: &mut Evaluator, doc: &Doc, hdu: usize, row: usize) -> Vec<usize> {
    let cells = vec![0, hdu, DATA, ROWS, row, 0];
    let last = ev.node(doc, &cells).unwrap().child_count as usize - 1;
    let data = step(&cells, last);
    let node = ev.node(doc, &data).unwrap();
    assert!(node.name.ends_with("DATA"), "the last cell is {}", node.name);
    assert_eq!(node.type_name, "Search samples", "{}", node.name);
    child(ev, doc, &data, "samples").expect("a run of samples")
}

fn sample(ev: &mut Evaluator, doc: &Doc, run: &[usize], i: usize) -> i128 {
    let node = ev.node(doc, &step(run, i)).unwrap_or_else(|e| panic!("sample {i}: {e:?}"));
    node.value.as_int().unwrap_or_else(|| panic!("sample {i} is {:?}", node.value))
}

/// Every fact about one PSRFITS file. Returns how many were checked.
fn check_psrfits(doc: &Doc, facts: &[Fact], file: &str, wrong: &mut Vec<String>) -> usize {
    let mut ev = Evaluator::new(formats::builtin("fits").unwrap());
    let mut differ = |fact: &Fact, got: String| wrong.push(format!("{file} {} {}: {got}, reference {}", fact.kind, fact.at, fact.value));
    for fact in facts {
        match fact.kind.as_str() {
            "card" => {
                let (hdu, key) = fact.at.split_once(',').unwrap();
                let got = card(&mut ev, doc, hdu.parse().unwrap(), key).unwrap_or_else(|| "no such card".into());
                if !same(&got, &fact.value) {
                    differ(fact, got);
                }
            }
            "rows" => {
                let got = ev.node(doc, &[0, fact.at.parse().unwrap(), DATA, ROWS]).unwrap().child_count;
                if got.to_string() != fact.value {
                    differ(fact, got.to_string());
                }
            }
            "float" => {
                let [hdu, row, column, i] = numbers(&fact.at)[..] else { panic!("{}", fact.at) };
                let node = ev.node(doc, &step(&cell(hdu, row, column), i)).unwrap();
                let Value::Float(got) = node.value else { panic!("{}: {:?}", fact.at, node.value) };
                if !same(&got.to_string(), &fact.value) {
                    differ(fact, got.to_string());
                }
            }
            "sample" => {
                let [hdu, row, i] = numbers(&fact.at)[..] else { panic!("{}", fact.at) };
                let run = search_samples(&mut ev, doc, hdu, row);
                let got = sample(&mut ev, doc, &run, i);
                if got.to_string() != fact.value {
                    differ(fact, got.to_string());
                }
            }
            "sum" => {
                let [hdu, row, first, count] = numbers(&fact.at)[..] else { panic!("{}", fact.at) };
                let run = search_samples(&mut ev, doc, hdu, row);
                let got: i128 = (first..first + count).map(|i| sample(&mut ev, doc, &run, i)).sum();
                if got.to_string() != fact.value {
                    differ(fact, got.to_string());
                }
            }
            other => panic!("{file}: a {other} fact is not one a PSRFITS file has"),
        }
    }
    // The rows fill the table: nothing in a row is left unread, and the
    // table is the rows and no heap.
    let sub = (0..ev.node(doc, &[0]).unwrap().child_count as usize).last().unwrap();
    let rows = ev.node(doc, &[0, sub, DATA, ROWS]).unwrap();
    let width: u64 = card(&mut ev, doc, sub, "NAXIS1").unwrap().parse().unwrap();
    assert_eq!(rows.size_bits, rows.child_count * width * 8, "{file}: the rows are not NAXIS2 rows of NAXIS1 bytes");
    let cells = ev.node(doc, &[0, sub, DATA, ROWS, 0, 0]).unwrap();
    assert_eq!(cells.size_bits, width * 8, "{file}: the cells of a row do not fill it");
    assert_eq!(formats::fits_contents(&doc_head(doc)), Some("psrfits-search"), "{file}");
    facts.len()
}

/// The front of a file, as much as the sniff window takes.
fn doc_head(doc: &Doc) -> Vec<u8> {
    let n = (doc.len_bits() / 8).min(formats::SNIFF_WINDOW as u64);
    let mut head = vec![0u8; n as usize];
    assert!(doc.read_bytes(0, &mut head).is_empty(), "the head of the file");
    head
}

// ------------------------------------------------------------------ HDF5

/// Whether the panel's text for an element and the reference's are the same
/// number. The panel writes an f32 in the fewest digits that read back as it,
/// `34392010` for the f32 numpy prints as `34392008.0`, so an f32 is compared
/// as an f32.
fn same_element(got: &str, want: &str, element_type: &str) -> bool {
    match (element_type, got.parse::<f64>(), want.parse::<f64>()) {
        ("f32", Ok(a), Ok(b)) => a as f32 == b as f32,
        _ => same(got, want),
    }
}

/// The 64-bit FNV-1a hash, as `tools/radio_reference.py` works it out.
fn fnv1a(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

/// Every fact about one filterbank file in HDF5. Returns how many were
/// checked.
///
/// A chunk is found by where it is in the file, the way the hex view finds
/// it, and asked for the reading the chunk panel shows: its size unpacked and
/// its first elements. That is the template's own path to the chunk, its
/// filters and its datatype. The whole of the unpacked chunk is checked by
/// its hash, through the same decoder, given the filters the reference read.
fn check_h5(doc: &Doc, facts: &[Fact], file: &str, wrong: &mut Vec<String>) -> usize {
    use qubero_core::eval::Explain;
    use qubero_core::formats::hdf5_chunk::{decode, Filter};
    let mut ev = Evaluator::new(formats::builtin("hdf5").unwrap());
    let mut differ = |fact: &Fact, got: String| wrong.push(format!("{file} {} {}: {got}, reference {}", fact.kind, fact.at, fact.value));
    let mut filters: Vec<Filter> = Vec::new();
    for fact in facts {
        match fact.kind.as_str() {
            "filter" => {
                filters = fact
                    .value
                    .split(';')
                    .map(|f| {
                        let (id, cd) = f.split_once(':').unwrap();
                        Filter { id: id.parse().unwrap(), client_data: cd.split(',').filter(|c| !c.is_empty()).map(|c| c.parse().unwrap()).collect() }
                    })
                    .collect();
            }
            "chunk" => {
                let v: Vec<&str> = fact.value.split(',').collect();
                let (offset, packed, unpacked): (u64, u64, u64) = (v[0].parse().unwrap(), v[1].parse().unwrap(), v[2].parse().unwrap());
                let (hash, first) = (v[3], &v[4..]);
                // The chunk node, which is the field at that byte or one of
                // its ancestors.
                let at = ev.locate(doc, offset * 8).unwrap();
                let chunk = (0..=at.len()).rev().map(|n| at[..n].to_vec()).find(|p| ev.node(doc, p).unwrap().type_name == "FilteredChunk");
                let Some(chunk) = chunk else {
                    differ(fact, format!("no filtered chunk at byte {offset}, only {at:?}"));
                    continue;
                };
                let node = ev.node(doc, &chunk).unwrap();
                if (node.offset_bits, node.size_bits) != (offset * 8, packed * 8) {
                    differ(fact, format!("a chunk of {} bytes at byte {}", node.size_bits / 8, node.offset_bits / 8));
                    continue;
                }
                match ev.explain(doc, &chunk, None) {
                    Ok(Explain::Hdf5Chunk { decoded_bytes, values, problem, element_type, .. }) => {
                        let shown: Vec<&str> = values.iter().take(first.len()).map(String::as_str).collect();
                        let agree = problem.is_none()
                            && decoded_bytes == unpacked
                            && shown.len() == first.len()
                            && shown.iter().zip(first).all(|(a, b)| same_element(&a.replace(',', ""), b, &element_type));
                        if !agree {
                            differ(fact, format!("{decoded_bytes} bytes, {element_type} {shown:?}, {problem:?}"));
                        }
                    }
                    other => {
                        differ(fact, format!("{other:?}"));
                        continue;
                    }
                }
                let mut bytes = vec![0u8; packed as usize];
                assert!(doc.read_bytes(offset, &mut bytes).is_empty());
                // The element width is in bitshuffle's client data.
                let got = decode(&bytes, &filters, 0, 0);
                if format!("{:016x}", fnv1a(&got.bytes)) != hash {
                    differ(fact, format!("unpacked bytes hash {:016x}, {:?}", fnv1a(&got.bytes), got.problem));
                }
            }
            other => panic!("{file}: a {other} fact is not one an HDF5 file has"),
        }
    }
    facts.len()
}

// --------------------------------------------------------------- SIGPROC

/// A value as the reference writes it: a whole number, a float in the fewest
/// digits that read back as it, or text.
fn value_text(ev: &mut Evaluator, doc: &Doc, at: &[usize]) -> String {
    match ev.node(doc, at).unwrap().value {
        Value::Float(f) => f.to_string(),
        Value::Str(s) => s,
        Value::Composite { .. } => match child(ev, doc, at, "text") {
            Some(text) => value_text(ev, doc, &text),
            None => String::new(),
        },
        Value::Bytes { .. } => String::new(),
        other => other.as_int().map(|n| n.to_string()).unwrap_or_else(|| format!("{other:?}")),
    }
}

/// `hhmmss.s` as hours, or `ddmmss.s` as degrees.
fn sexagesimal(v: f64) -> f64 {
    let a = v.abs();
    let (whole, minutes, seconds) = ((a / 10_000.0).trunc(), ((a % 10_000.0) / 100.0).trunc(), a % 100.0);
    (whole + minutes / 60.0 + seconds / 3600.0).copysign(v)
}

/// What a run's elements are, in the words `same_element` takes: `f32` for
/// a run of floats, and anything else for the rest.
fn element_type(ev: &mut Evaluator, doc: &Doc, run: &[usize]) -> &'static str {
    if ev.node(doc, run).unwrap().type_name.starts_with("f32") { "f32" } else { "" }
}

/// The header record of a filterbank file whose keyword is `key`.
fn fil_record(ev: &mut Evaluator, doc: &Doc, key: &str) -> Option<Vec<usize>> {
    let n = ev.node(doc, &[0]).unwrap().child_count as usize;
    (0..n).map(|i| vec![0, i]).find(|at| {
        let k = child(ev, doc, at, "keyword").unwrap();
        ev.node(doc, &k).unwrap().value == Value::Str(key.into())
    })
}

/// Every fact about one SIGPROC filterbank file. Returns how many were
/// checked.
fn check_fil(doc: &Doc, facts: &[Fact], file: &str, wrong: &mut Vec<String>) -> usize {
    let mut ev = Evaluator::new(formats::builtin("sigproc").unwrap());
    let mut differ = |fact: &Fact, got: String| wrong.push(format!("{file} {} {}: {got}, reference {}", fact.kind, fact.at, fact.value));
    let samples = vec![1, 0];
    for fact in facts {
        match fact.kind.as_str() {
            "key" => {
                let (i, key) = fact.at.split_once(',').unwrap();
                let at = vec![0, i.parse().unwrap()];
                let k = child(&mut ev, doc, &at, "keyword").unwrap();
                let got_key = value_text(&mut ev, doc, &k);
                let v = child(&mut ev, doc, &at, "value").unwrap();
                let got = value_text(&mut ev, doc, &v);
                if got_key != key || !same(&got, &fact.value) {
                    differ(fact, format!("{got_key} {got}"));
                }
            }
            "h5key" => {
                let mut got = match fil_record(&mut ev, doc, &fact.at) {
                    Some(at) => {
                        let v = child(&mut ev, doc, &at, "value").unwrap();
                        value_text(&mut ev, doc, &v)
                    }
                    None => "no such record".into(),
                };
                // The filterbank writes a position as `hhmmss.s` and
                // `ddmmss.s`, and the HDF5 copy as hours and degrees.
                if matches!(fact.at.as_str(), "src_raj" | "src_dej") {
                    got = sexagesimal(got.parse().unwrap()).to_string();
                }
                if !same(&got, &fact.value) {
                    differ(fact, got);
                }
            }
            "data" => {
                let want: Vec<u64> = fact.value.split(',').map(|v| v.parse().unwrap()).collect();
                let run = ev.node(doc, &samples).unwrap();
                let shape = ev.table_shape(doc, &samples).unwrap().expect("a table");
                let got = [run.offset_bits / 8, shape.columns.unwrap_or(0), run.child_count / shape.columns.unwrap_or(1).max(1)];
                if got[..] != want[..] {
                    differ(fact, format!("{got:?}"));
                }
            }
            "value" => {
                let at = step(&samples, fact.at.parse().unwrap());
                let got = value_text(&mut ev, doc, &at);
                if !same_element(&got, &fact.value, element_type(&mut ev, doc, &samples)) {
                    differ(fact, got);
                }
            }
            "sum" => {
                let [first, count] = numbers(&fact.at)[..] else { panic!("{}", fact.at) };
                // Each value as the number it is on disk, an f32 widened,
                // which is what the reference sums.
                let f32s = element_type(&mut ev, doc, &samples) == "f32";
                let got: f64 = (first..first + count)
                    .map(|i| {
                        let text = value_text(&mut ev, doc, &step(&samples, i));
                        if f32s { f64::from(text.parse::<f32>().unwrap()) } else { text.parse::<f64>().unwrap() }
                    })
                    .sum();
                if !same(&got.to_string(), &fact.value) {
                    differ(fact, got.to_string());
                }
            }
            other => panic!("{file}: a {other} fact is not one a filterbank file has"),
        }
    }
    assert_eq!(formats::sniff(&doc_head(doc), doc.len_bits() / 8), Some("sigproc"), "{file}");
    facts.len()
}

// ----------------------------------------------------------------- GUPPI

/// The run of values in one channel of one block.
fn guppi_channel(ev: &mut Evaluator, doc: &Doc, block: usize, channel: usize) -> Vec<usize> {
    let channels = child(ev, doc, &[0, block, 2], "channels").expect("the channels");
    let at = step(&step(&channels, channel), 0);
    assert_eq!(ev.node(doc, &at).unwrap().name, "samples");
    at
}

/// Every fact about one GUPPI raw file. Returns how many were checked.
fn check_raw(doc: &Doc, facts: &[Fact], file: &str, wrong: &mut Vec<String>) -> usize {
    let mut ev = Evaluator::new(formats::builtin("guppi").unwrap());
    let mut differ = |fact: &Fact, got: String| wrong.push(format!("{file} {} {}: {got}, reference {}", fact.kind, fact.at, fact.value));
    for fact in facts {
        match fact.kind.as_str() {
            "block" => {
                let b: usize = fact.at.parse().unwrap();
                let block = ev.node(doc, &[0, b]).unwrap();
                let cards = ev.node(doc, &[0, b, 0]).unwrap();
                let data = ev.node(doc, &[0, b, 2]).unwrap();
                let got = format!("{},{},{},{}", block.offset_bits / 8, cards.child_count, data.offset_bits / 8, data.size_bits / 8);
                if got != fact.value {
                    differ(fact, got);
                }
            }
            "card" => {
                let (b, key) = fact.at.split_once(',').unwrap();
                let got = card(&mut ev, doc, b.parse().unwrap(), key).unwrap_or_else(|| "no such card".into());
                if !same(&got, &fact.value) {
                    differ(fact, got);
                }
            }
            "sample" => {
                let [b, c, i] = numbers(&fact.at)[..] else { panic!("{}", fact.at) };
                let run = guppi_channel(&mut ev, doc, b, c);
                let got = sample(&mut ev, doc, &run, i);
                if got.to_string() != fact.value {
                    differ(fact, got.to_string());
                }
            }
            "sum" => {
                let [b, c, first, count] = numbers(&fact.at)[..] else { panic!("{}", fact.at) };
                let run = guppi_channel(&mut ev, doc, b, c);
                let got: i128 = (first..first + count).map(|i| sample(&mut ev, doc, &run, i)).sum();
                if got.to_string() != fact.value {
                    differ(fact, got.to_string());
                }
            }
            other => panic!("{file}: a {other} fact is not one a GUPPI raw file has"),
        }
    }
    // The blocks fill the file.
    let blocks = ev.node(doc, &[0]).unwrap();
    assert_eq!(blocks.size_bits, doc.len_bits(), "{file}: the blocks do not fill the file");
    assert_eq!(formats::sniff(&doc_head(doc), doc.len_bits() / 8), Some("guppi"), "{file}");
    facts.len()
}

// ------------------------------------------------------------------ both

/// Read every file a reference names from `dir`, each with the reader its
/// extension asks for. Returns how many facts were checked.
fn check_all(dir: &Path, facts: &BTreeMap<String, Vec<Fact>>, wrong: &mut Vec<String>) -> usize {
    let mut checked = 0;
    for (file, facts) in facts {
        let path = dir.join(file);
        let Ok(bytes) = std::fs::read(&path) else {
            wrong.push(format!("{file}: in the reference and not in {}", dir.display()));
            continue;
        };
        let doc = Document::new(MemSource(bytes));
        let n = match path.extension().and_then(|e| e.to_str()) {
            Some("sf") => check_psrfits(&doc, facts, file, wrong),
            Some("h5") => check_h5(&doc, facts, file, wrong),
            Some("fil") => check_fil(&doc, facts, file, wrong),
            Some("raw") => check_raw(&doc, facts, file, wrong),
            other => panic!("{file}: no check for a {other:?} file"),
        };
        eprintln!("{file}: {n} facts");
        checked += n;
    }
    checked
}

/// The small files in the sample collection.
#[test]
fn the_collections_radio_files_read_as_independent_readers_read_them() {
    let Some(root) = qubero_samples::root() else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let facts = reference(include_str!("fixtures/radio/reference.tsv"));
    // A collection from before these files were added has none of them.
    let Some(root) = qubero_samples::roots().into_iter().find(|r| facts.keys().any(|f| r.join(f).exists())).or(Some(root)) else { return };
    if !facts.keys().any(|f| root.join(f).exists()) {
        eprintln!("skipped: {} has none of the radio samples", root.display());
        return;
    }
    let mut wrong = Vec::new();
    let checked = check_all(&root, &facts, &mut wrong);
    assert!(wrong.is_empty(), "{} of {checked} facts differ:\n{}", wrong.len(), wrong.join("\n"));
}

fn extras() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("QUBERO_RADIO") {
        return Some(PathBuf::from(p));
    }
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.ancestors().map(|a| a.join("qubero2-extras").join("radio")).find(|p| p.join("radio_reference.tsv").exists())
}

/// Whole observations, where the folder that holds them is on this machine.
#[test]
fn whole_observations_read_as_independent_readers_read_them() {
    let Some(dir) = extras() else {
        eprintln!("skipped: no qubero2-extras/radio beside the checkout, and QUBERO_RADIO is not set");
        return;
    };
    let text = std::fs::read_to_string(dir.join("radio_reference.tsv")).expect("radio_reference.tsv");
    let mut wrong = Vec::new();
    let checked = check_all(&dir, &reference(&text), &mut wrong);
    assert!(wrong.is_empty(), "{} of {checked} facts differ:\n{}", wrong.len(), wrong.join("\n"));
}
