//! Rubin images in the LSST Science Pipelines' layout (afw), read against what
//! astropy reads from them.
//!
//! `tools/afw_reference.py` wrote `afw_reference.tsv` beside the files: the
//! mask planes each mask header names, mask pixels with each plane set, the
//! primary header's archive ID cards, every row of the archive index, and
//! every cell of bytes in the archive's tables with what it starts with. Each
//! is read here through the template and has to say the same.
//!
//! The files are in `qubero2-extras/rubin` beside the checkout or an ancestor
//! of it, or wherever `QUBERO_RUBIN` points: two DP2 images, a visit image and
//! a difference image, whose masks are tile-compressed, and two of afw's own
//! test files with plain ones. The DP2 images have no licence to be passed on,
//! so they are not in the sample collection. Without the folder the tests say
//! so and pass.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Explain, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

fn rubin() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("QUBERO_RUBIN") {
        return Some(PathBuf::from(p));
    }
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.ancestors().map(|a| a.join("qubero2-extras").join("rubin")).find(|p| p.join("afw_reference.tsv").exists())
}

/// One line of the reference: what kind of fact, which HDU, where in it, and
/// the value.
struct Fact {
    kind: String,
    hdu: usize,
    at: String,
    value: String,
}

/// The reference, by file.
fn reference(dir: &Path) -> BTreeMap<String, Vec<Fact>> {
    let text = std::fs::read_to_string(dir.join("afw_reference.tsv")).unwrap();
    let mut out: BTreeMap<String, Vec<Fact>> = BTreeMap::new();
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let [kind, file, hdu, at, value] = line.split('\t').collect::<Vec<_>>()[..] else { panic!("bad line {line}") };
        let fact = Fact { kind: kind.into(), hdu: hdu.parse().unwrap(), at: at.into(), value: value.into() };
        out.entry(file.into()).or_default().push(fact);
    }
    out
}

/// A file and the evaluator reading it.
fn open(dir: &Path, file: &str) -> (Document<MemSource>, Evaluator) {
    let bytes = std::fs::read(dir.join(file)).unwrap();
    (Document::new(MemSource(bytes)), Evaluator::new(formats::builtin("fits").unwrap()))
}

/// Where a unit's fields are: its cards, and its data.
const CARDS: usize = 0;
const DATA: usize = 3;

/// The names of the bits set in `value`, from the planes the reference gives,
/// and how many set bits no plane names.
fn set_in(value: u64, planes: &BTreeMap<u32, String>) -> (Vec<String>, u32) {
    let mut set = Vec::new();
    let mut unnamed = 0;
    for bit in (0..64).filter(|b| value >> b & 1 == 1) {
        match planes.get(&bit) {
            Some(n) => set.push(n.clone()),
            None => unnamed += 1,
        }
    }
    (set, unnamed)
}

fn check_masks(dir: &Path, wrong: &mut Vec<String>) -> usize {
    let mut checked = 0;
    for (file, facts) in reference(dir) {
        let (d, mut ev) = open(dir, &file);
        let mut planes: BTreeMap<usize, BTreeMap<u32, String>> = BTreeMap::new();
        for f in facts.iter().filter(|f| f.kind == "plane") {
            planes.entry(f.hdu).or_default().insert(f.value.parse().unwrap(), f.at.clone());
        }
        for (&hdu, named) in &planes {
            let data = vec![0, hdu, DATA];
            let node = ev.node(&d, &data).unwrap();
            let tile = ev.fits_tile(&d, &data).unwrap_or_else(|e| panic!("{file} {e:?}"));
            let width = match &tile {
                Some(t) => t.shape[0] as usize,
                None => {
                    let axes = ev.node(&d, &[0, hdu, 2, 0]).unwrap().value;
                    axes.as_int().unwrap() as usize
                }
            };
            for f in facts.iter().filter(|f| f.kind == "pixel" && f.hdu == hdu) {
                let (y, x) = f.at.split_once(',').unwrap();
                let k = y.parse::<usize>().unwrap() * width + x.parse::<usize>().unwrap();
                let want: i64 = f.value.parse().unwrap();
                let (names, unnamed) = set_in(want as u64, named);
                checked += 1;
                match &tile {
                    // A compressed mask's pixels are in the tile, and the
                    // panel writes the names of a pixel's bits after it.
                    Some(t) => {
                        let text = t.text(k);
                        let mut expect = want.to_string();
                        if want != 0 {
                            let mut all = names.clone();
                            if unnamed > 0 {
                                all.push(format!("{unnamed} more"));
                            }
                            expect = format!("{want}\u{a0}({})", all.join("|"));
                        }
                        if text != expect {
                            wrong.push(format!("{file}: tile pixel ({}) is {text:?}, astropy says {expect:?}", f.at));
                        }
                    }
                    None => {
                        let mut at = data.clone();
                        at.push(k);
                        let pixel = ev.node(&d, &at).unwrap();
                        let ok = match &pixel.value {
                            Value::Flags { raw, set, unnamed: u } => *raw as i64 == want && *set == names && *u == unnamed,
                            // A mask the header scales keeps the reading every
                            // scaled image has, and is checked by its worth.
                            _ => pixel_worth(&mut ev, &d, &at) == Some(want),
                        };
                        if !ok {
                            wrong.push(format!("{file}: pixel ({}) is {:?}, astropy says {want} {names:?}", f.at, pixel.value));
                        }
                    }
                }
            }
            // Every plane the header names is a bit with that name, and no
            // other bit has one.
            let first = match &tile {
                Some(t) => {
                    let got: BTreeMap<u32, String> = t.planes.iter().cloned().collect();
                    if &got != named {
                        wrong.push(format!("{file}: the tile's planes are {got:?}, astropy says {named:?}"));
                    }
                    continue;
                }
                None => [data.as_slice(), &[0]].concat(),
            };
            checked += 1;
            match ev.explain(&d, &first, None).unwrap() {
                Explain::Flags { bits, .. } => {
                    if node.type_name != "Mask planes[]" {
                        wrong.push(format!("{file}: the mask reads as {}", node.type_name));
                    }
                    let got: BTreeMap<u32, String> = bits.iter().filter_map(|b| Some((b.bit, b.name.clone()?))).collect();
                    if &got != named {
                        wrong.push(format!("{file}: the mask's planes are {got:?}, astropy says {named:?}"));
                    }
                }
                // Scaled, and so not named; see `fits_afw`.
                _ => assert_eq!(file, "archiveImportTest.fits", "{file}: the mask is not read as named bits"),
            }
        }
    }
    checked
}

/// What a scaled pixel is worth.
fn pixel_worth(ev: &mut Evaluator, d: &Document<MemSource>, at: &[usize]) -> Option<i64> {
    let worth = ev.child_named(d, at, "worth").ok()??;
    ev.node(d, &worth).ok()?.value.as_int().map(|v| v as i64)
}

#[test]
fn mask_pixels_read_as_the_planes_the_header_names() {
    let Some(dir) = rubin() else {
        eprintln!("no qubero2-extras/rubin beside the checkout, and QUBERO_RUBIN is not set");
        return;
    };
    let mut wrong = Vec::new();
    let checked = check_masks(&dir, &mut wrong);
    assert!(wrong.is_empty(), "{} of {checked} differ:\n{}", wrong.len(), wrong.join("\n"));
    assert!(checked > 40, "only {checked} checked");
}

/// The text of a node, trimmed.
fn text(ev: &mut Evaluator, d: &Document<MemSource>, at: &[usize]) -> String {
    match ev.node(d, at).unwrap().value {
        Value::Str(s) => s.trim().to_string(),
        Value::Int(v) => v.to_string(),
        other => format!("{other:?}"),
    }
}

#[test]
fn the_archive_index_reads_as_astropy_reads_it() {
    let Some(dir) = rubin() else {
        eprintln!("no qubero2-extras/rubin beside the checkout, and QUBERO_RUBIN is not set");
        return;
    };
    let mut wrong = Vec::new();
    let mut checked = 0;
    for (file, facts) in reference(&dir) {
        let (d, mut ev) = open(&dir, &file);
        // Which part of the image each object is, by its ID, from the cards
        // in the primary header: `ARCHIVE_ID_PSF` and `PSF_ID` both say PSF.
        let mut parts: BTreeMap<i128, String> = BTreeMap::new();
        let cards = ev.node(&d, &[0, 0, CARDS]).unwrap().child_count as usize;
        for f in facts.iter().filter(|f| f.kind == "card") {
            let part = f.at.strip_prefix("ARCHIVE_ID_").or_else(|| f.at.strip_suffix("_ID")).unwrap();
            parts.insert(f.value.parse().unwrap(), part.to_string());
            // The card is labelled with its keyword, long or short, and its
            // value is the number astropy reads.
            let found = (0..cards).find(|&i| ev.node(&d, &[0, 0, CARDS, i]).unwrap().name.split_whitespace().nth(1) == Some(&f.at));
            checked += 1;
            let Some(i) = found else {
                wrong.push(format!("{file}: no card labelled {}", f.at));
                continue;
            };
            let card = [0, 0, CARDS, i];
            let value = match ev.child_named(&d, &[&card[..], &[2]].concat(), "name").unwrap() {
                // space, name, mark, value; in the value its parts, and in
                // those the whole number.
                Some(_) => text(&mut ev, &d, &[&card[..], &[2, 3, 1, 0, 1]].concat()),
                None => text(&mut ev, &d, &[&card[..], &[2, 1]].concat()),
            };
            if value != f.value {
                wrong.push(format!("{file}: {} reads as {value}, astropy says {}", f.at, f.value));
            }
        }
        for f in facts.iter().filter(|f| f.kind == "index") {
            let row = [0, f.hdu, DATA, 1, f.at.parse().unwrap()];
            let cells: Vec<&str> = f.value.split(';').collect();
            checked += 1;
            let label = ev.node(&d, &row).unwrap_or_else(|e| panic!("{file} {row:?} {e:?}")).name;
            if label.trim_end() != format!("[{}] {}", f.at, cells[5]) {
                wrong.push(format!("{file}: index row {} is labelled {label:?}, its class is {}", f.at, cells[5]));
            }
            for (c, want) in cells.iter().enumerate() {
                // A number is a list of one; text is the cell.
                let at = [&row[..], &[0, c]].concat();
                let got = match c < 5 {
                    true => text(&mut ev, &d, &[&at[..], &[0]].concat()),
                    false => text(&mut ev, &d, &at),
                };
                if got != *want {
                    wrong.push(format!("{file}: index row {} cell {c} is {got:?}, astropy says {want:?}", f.at));
                }
            }
            let id: i128 = cells[0].parse().unwrap();
            let component = ev.node(&d, &[&row[..], &[1]].concat()).unwrap().value;
            let got = match component {
                Value::Enum { name, .. } => name,
                _ => None,
            };
            if got.as_deref() != parts.get(&id).map(String::as_str) {
                wrong.push(format!("{file}: index row {} is component {got:?}, the cards say {:?}", f.at, parts.get(&id)));
            }
        }
    }
    assert!(wrong.is_empty(), "{} of {checked} differ:\n{}", wrong.len(), wrong.join("\n"));
    assert!(checked > 900, "only {checked} checked");
}

#[test]
fn bytes_in_the_archive_open_as_what_they_hold() {
    let Some(dir) = rubin() else {
        eprintln!("no qubero2-extras/rubin beside the checkout, and QUBERO_RUBIN is not set");
        return;
    };
    let mut wrong = Vec::new();
    let mut checked = 0;
    for (file, facts) in reference(&dir) {
        let (d, mut ev) = open(&dir, &file);
        let bytes = std::fs::read(dir.join(&file)).unwrap();
        let mut by_hdu: BTreeMap<usize, BTreeMap<String, &str>> = BTreeMap::new();
        for f in facts.iter().filter(|f| f.kind == "blob") {
            by_hdu.entry(f.hdu).or_default().insert(f.at.clone(), &f.value);
        }
        for (hdu, blobs) in by_hdu {
            let heap = [0, hdu, DATA, 2];
            let n = ev.node(&d, &heap).unwrap().child_count as usize;
            let mut seen = 0;
            for k in 0..n {
                let at = [&heap[..], &[k]].concat();
                // Which row and column put it there, from the descriptor that
                // placed it, and the column's name from the cell's label.
                let placed = ev.origins(&d, &at).unwrap();
                let from = &placed[0].path;
                let cell = ev.node(&d, &from[..from.len() - 2]).unwrap().name;
                let row = from[from.len() - 5];
                let column = cell.split_whitespace().nth(1).unwrap_or("").to_string();
                let Some(want) = blobs.get(&format!("{column},{row}")) else { continue };
                seen += 1;
                checked += 1;
                let node = ev.node(&d, &at).unwrap();
                let start = (node.offset_bits / 8) as usize;
                let len = (node.size_bits / 8) as usize;
                let kind = match node.type_name.as_str() {
                    // A run that is already what it is, which opens with the
                    // template its bytes sniff as, the way a PNG in a GRIB
                    // message does.
                    "stored" => match ev.open_space(&d, 0, &at) {
                        Ok(Some(id)) => match ev.space(id).map(|s| s.template.clone()) {
                            Some(t) if t == "pickle" || t == "picklefpf" => "pickle".to_string(),
                            other => format!("stored, opened as {other:?}"),
                        },
                        other => format!("stored, not opened: {other:?}"),
                    },
                    "utf8[]" => "text".to_string(),
                    "u8[]" => "bytes".to_string(),
                    other => other.to_string(),
                };
                let head: String = bytes[start..start + len.min(8)].iter().map(|b| format!("{b:02x}")).collect();
                let got = format!("{kind} {len} {head}");
                if got != **want {
                    wrong.push(format!("{file}: HDU {hdu} {column} row {row} reads as {got}, astropy says {want}"));
                }
            }
            if seen != blobs.len() {
                wrong.push(format!("{file}: HDU {hdu} has {seen} of the {} cells of bytes astropy reads", blobs.len()));
            }
        }
    }
    assert!(wrong.is_empty(), "{} of {checked} differ:\n{}", wrong.len(), wrong.join("\n"));
    assert!(checked > 800, "only {checked} checked");
}
