//! The data behind the descriptors, in files written by the HDF4 library
//! itself. The fixtures live in QUBERO_SAMPLES/hdf4, or in the sibling
//! qubero-samples collection.
//!
//! Nothing in an HDF4 file says what a run of bytes is on its own: an image
//! asks its dimension record, a table's rows ask their header, a dataset's
//! values ask the group that names both. So what these check is not that the
//! bytes were placed but that the questions were asked and answered, against
//! files whose answers `pyhdf` agrees with.
use std::collections::BTreeMap;
use std::path::PathBuf;
use qubero_core::{document::Document, eval::{Evaluator, Value}, formats, source::MemSource};

fn hdf4_samples() -> Option<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(paths) = std::env::var("QUBERO_SAMPLES") {
        roots.extend(paths.split(';').filter(|s| !s.is_empty()).map(PathBuf::from));
    }
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../qubero-samples"));
    roots.into_iter().map(|p| p.join("hdf4")).find(|p| p.is_dir())
}

fn samples() -> Vec<(String, Vec<u8>)> {
    let Some(root) = hdf4_samples() else { return Vec::new() };
    let mut out = Vec::new();
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "hdf") { continue; }
        out.push((path.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read(&path).unwrap()));
    }
    out.sort();
    out
}

/// Every node of a file, by the type it read as. What the tests ask of a file
/// is "is there one of these in it", not "is it at this path": a path through
/// a descriptor block says nothing a reader would recognise.
fn walk(ev: &mut Evaluator, doc: &Document<MemSource>, name: &str) -> BTreeMap<String, Vec<Vec<usize>>> {
    let mut found: BTreeMap<String, Vec<Vec<usize>>> = BTreeMap::new();
    let mut stack = vec![Vec::new()];
    let mut seen = 0;
    while let Some(at) = stack.pop() {
        let node = ev.node(doc, &at).unwrap_or_else(|e| panic!("{name} {at:?}: {e:?}"));
        seen += 1;
        assert!(seen < 200_000, "{name}: more nodes than a file this size can hold");
        assert!(node.child_count < 100_000, "{name} {at:?}: unbounded children");
        found.entry(node.type_name.clone()).or_default().push(at.clone());
        for i in (0..node.child_count as usize).rev() {
            let mut next = at.clone();
            next.push(i);
            stack.push(next);
        }
    }
    found
}

fn value(ev: &mut Evaluator, doc: &Document<MemSource>, at: &[usize]) -> Value {
    ev.node(doc, at).unwrap().value
}

/// What a value counts as, whichever of the twelve number types it was read
/// with. A dataset of int8 and a dataset of float64 hold the same run of
/// numbers in this file and are checked the same way.
fn number(v: Value) -> f64 {
    match v {
        Value::Int(n) => n as f64,
        Value::UInt(n) => n as f64,
        Value::Float(f) => f,
        other => panic!("not a number: {other:?}"),
    }
}

/// Every node of every sample reads, and every one of them sits inside the
/// file. A lookup that finds nothing has to fall back to bytes rather than
/// placing a run at offset zero or four gigabytes past the end.
#[test]
fn every_node_of_every_sample_reads_and_stays_inside_the_file() {
    let files = samples();
    if files.is_empty() {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    }
    for (name, bytes) in &files {
        let len = bytes.len() as u64 * 8;
        let doc = Document::new(MemSource(bytes.clone()));
        let mut ev = Evaluator::new(formats::builtin("hdf4").unwrap());
        let found = walk(&mut ev, &doc, name);
        for paths in found.values() {
            for at in paths {
                let node = ev.node(&doc, at).unwrap();
                assert!(node.offset_bits + node.size_bits <= len, "{name} {at:?} runs past the end");
            }
        }
        eprintln!("{name}: {} kinds of node", found.len());
    }
}

/// Each sample was chosen for one thing the template has to get right, and
/// this is that thing, file by file.
#[test]
fn the_data_behind_the_descriptors_is_opened() {
    let files = samples();
    if files.is_empty() {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    }
    let mut checked = 0;
    for (name, bytes) in &files {
        let doc = Document::new(MemSource(bytes.clone()));
        let mut ev = Evaluator::new(formats::builtin("hdf4").unwrap());
        let found = walk(&mut ev, &doc, name);
        let of = |what: &str| found.get(what).map_or(0, |v| v.len());
        match name.as_str() {
            // Twelve datasets, one per number type, under thirteen groups,
            // and the seven whose members are in the same block open. Each is
            // ten by ten counting up along the second dimension, which is
            // what pyhdf reads out of them.
            "ntcheck.hdf" => {
                assert_eq!(of("Hdf4ScientificDataset"), 7);
                let mut ten_by_ten = 0;
                for at in found["Hdf4ScientificDataset"].clone() {
                    let rows = [at, vec![2, 0]].concat();
                    if ev.node(&doc, &rows).unwrap().child_count != 10 {
                        continue;
                    }
                    assert_eq!(ev.node(&doc, &[rows.clone(), vec![0]].concat()).unwrap().child_count, 10);
                    let across = |ev: &mut Evaluator, i: usize, j: usize| {
                        number(value(ev, &doc, &[rows.clone(), vec![i, j]].concat()))
                    };
                    let (first, ninth) = (across(&mut ev, 0, 0), across(&mut ev, 0, 9));
                    assert_eq!(ninth, first + 9.0, "the last index is the one that counts up");
                    let (next_row, next_column) = (across(&mut ev, 1, 0), across(&mut ev, 0, 1));
                    assert_ne!(next_row, next_column, "the first dimension is the slowest to change");
                    ten_by_ten += 1;
                }
                assert_eq!(ten_by_ten, 7);
                // The class byte is read for what the type makes it: 1 is
                // IEEE where the type is a float and Motorola byte order
                // where it is a whole number, and naming it one way for both
                // would call half of them something the file did not say.
                assert!(of("Hdf4FloatClass") > 0, "a float's class byte");
                assert!(of("Hdf4IntClass") > 0, "a whole number's class byte");
                // Every dataset has a scale along its first dimension and
                // none along its second, and the flag bytes are the only
                // thing that says so.
                assert_eq!(of("Hdf4SdScales"), 7, "the rest name a dimension record a block away");
                for at in found["Hdf4SdScales"].clone() {
                    assert_eq!(value(&mut ev, &doc, &[at.clone(), vec![1, 0]].concat()), Value::UInt(1));
                    assert_eq!(value(&mut ev, &doc, &[at.clone(), vec![1, 1]].concat()), Value::UInt(0));
                    assert_eq!(ev.node(&doc, &[at.clone(), vec![2, 0, 2]].concat()).unwrap().child_count, 10);
                    assert_eq!(ev.node(&doc, &[at, vec![2, 1, 2]].concat()).unwrap().size_bits, 0);
                }
                // A maximum and a minimum in the dataset's own number type.
                assert_eq!(of("Hdf4MaxAndMin"), 6);
            }
            // The same datasets the other way round. A value of 1 is written
            // `01 00` and reads as 1, not as 256.
            "litend.hdf" => {
                // Eight groups, and six of them name a descriptor this
                // block's index reaches. The other two are the gap a
                // per-block index leaves.
                assert_eq!(of("Hdf4ScientificDataset"), 6);
                let mut little = 0;
                for at in found["Hdf4ScientificDataset"].clone() {
                    let first = [at, vec![2, 0, 0, 1]].concat();
                    if let Ok(node) = ev.node(&doc, &first) {
                        if node.value == Value::Int(1) && node.size_bits > 8 {
                            little += 1;
                        }
                    }
                }
                assert!(little >= 2, "a wide value read the little end first");
            }
            // A raster and its palette, both named by a vgroup whose members
            // are all written after it.
            "Image_with_Palette.hdf" => {
                assert_eq!(of("Hdf4RasterImage"), 1);
                assert_eq!(of("Hdf4Palette"), 1);
                let colours = [found["Hdf4Palette"][0].clone(), vec![0]].concat();
                assert_eq!(ev.node(&doc, &colours).unwrap().child_count, 256);
                let rows = [found["Hdf4RasterImage"][0].clone(), vec![2, 0]].concat();
                assert_eq!(ev.node(&doc, &rows).unwrap().child_count, 5);
                // Five pixels across, two samples each, which is what the
                // image dimension record beside it says.
                assert_eq!(ev.node(&doc, &[rows.clone(), vec![0]].concat()).unwrap().child_count, 5);
                assert_eq!(ev.node(&doc, &[rows, vec![0, 0]].concat()).unwrap().child_count, 2);
                // Every member of the vgroup was found, forward, in its block.
                let vgroup = found["Hdf4Vgroup"][0].clone();
                for i in 0..4 {
                    let offset = [vgroup.clone(), vec![3, i, 2]].concat();
                    assert!(value(&mut ev, &doc, &offset) != Value::Int(0), "member {i} went unfound");
                }
            }
            // Rank one, two and three, and every run of values kept in linked
            // blocks, so no dataset opens and none of them breaks either.
            "tdata.hdf" => {
                assert_eq!(of("Hdf4ScientificDataset"), 0, "the values are in linked blocks");
                assert!(of("Hdf4VdataRecords") >= 3, "the dimensions are vdatas");
                let ranks: Vec<i128> = found["Hdf4SdDimensions"]
                    .iter()
                    .filter_map(|at| match value(&mut ev, &doc, &[at.clone(), vec![0]].concat()) {
                        Value::Int(v) => Some(v),
                        _ => None,
                    })
                    .collect();
                assert!(ranks.contains(&3) && ranks.contains(&2) && ranks.contains(&1));
                // Three special elements, each naming the chain of blocks its
                // values are really in.
                assert_eq!(of("Hdf4LinkedBlocks"), 3);
                for at in found["Hdf4LinkedBlocks"].clone() {
                    let blocks = number(value(&mut ev, &doc, &[at, vec![2]].concat()));
                    assert!(blocks > 0.0, "a chain of no blocks holds nothing");
                }
            }
            // Twenty tables, and the rows of each take their columns from the
            // header with the same reference number, which in this file is
            // always the descriptor after them.
            "tvattr.hdf" => {
                assert!(of("Hdf4VdataRecords") >= 15);
                let fields = found["Hdf4VdataField"].clone();
                assert_eq!(fields.len(), 25, "one per column per record, over every table with rows");
                for at in fields {
                    let name = ev.node(&doc, &at).unwrap().name;
                    let (index, column) = name.split_once(' ').unwrap_or((&name, ""));
                    assert!(index.starts_with('['), "{name} is not an index");
                    assert!(!column.is_empty(), "{name} has no column name behind its index");
                }
            }
            // Three rasters of the same image, one per interlacing, and a
            // fourth of its own size. The interlacing decides the nesting, so
            // it decides which shape each one reads as.
            "grtdfui83.hdf" => {
                assert_eq!(of("Hdf4RasterImage"), 4);
                assert_eq!(of("Hdf4RasterPixels"), 2, "the samples of a pixel together");
                assert_eq!(of("Hdf4RasterComponentLines"), 1, "one run per component in a row");
                assert_eq!(of("Hdf4RasterComponentPlanes"), 1, "one of them is written a component at a time");
                let planes = [found["Hdf4RasterComponentPlanes"][0].clone(), vec![0]].concat();
                assert_eq!(ev.node(&doc, &planes).unwrap().child_count, 3);
                assert_eq!(ev.node(&doc, &[planes, vec![0]].concat()).unwrap().child_count, 15);
            }
            other => panic!("{other} is in the collection with nothing checked of it"),
        }
        checked += 1;
    }
    assert_eq!(checked, 6);
}
