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

/// How many of a file's references were followed, of how many it writes.
#[derive(Debug, Default, PartialEq)]
struct Reached {
    /// Scientific data groups whose dataset opened, and all of them.
    datasets: (usize, usize),
    /// Members of groups and vgroups that were found, and all of them.
    members: (usize, usize),
    /// Attributes of vdatas and vgroups whose value was found, and all of
    /// them. Counted once each: a vdata header is read again under its rows.
    attributes: (usize, usize),
    /// Descriptors read through another with the same reference number (a
    /// table's rows, an image, scales, a maximum and minimum) that were read
    /// that way, and all of them.
    by_ref: (usize, usize),
}

fn reached(ev: &mut Evaluator, doc: &Document<MemSource>, found: &BTreeMap<String, Vec<Vec<usize>>>) -> Reached {
    let mut out = Reached::default();
    for at in found.get("Hdf4ScientificDataGroup").cloned().unwrap_or_default() {
        out.datasets.1 += 1;
        if ev.node(doc, &[at, vec![1]].concat()).unwrap().type_name == "Hdf4ScientificDataset" {
            out.datasets.0 += 1;
        }
    }
    for kind in ["Hdf4GroupMember", "Hdf4VgroupMember"] {
        for at in found.get(kind).cloned().unwrap_or_default() {
            out.members.1 += 1;
            if value(ev, doc, &[at, vec![2]].concat()) != Value::Int(0) {
                out.members.0 += 1;
            }
        }
    }
    let mut attributes = std::collections::BTreeMap::new();
    // Where the offset is in each: a vgroup's attribute has no field index.
    for (kind, offset) in [("Hdf4VdataAttribute", 3), ("Hdf4VgroupAttribute", 2)] {
        for at in found.get(kind).cloned().unwrap_or_default() {
            let start = ev.node(doc, &at).unwrap().offset_bits;
            let placed = value(ev, doc, &[at, vec![offset]].concat()) != Value::Int(0);
            attributes.insert(start, placed);
        }
    }
    out.attributes = (attributes.values().filter(|p| **p).count(), attributes.len());
    let reads_as = [(1963, "Hdf4VdataRecords"), (302, "Hdf4RasterImage"), (703, "Hdf4SdScales"), (707, "Hdf4MaxAndMin")];
    for at in found.get("Hdf4Descriptor").cloned().unwrap_or_default() {
        let tag = match value(ev, doc, &[at.clone(), vec![0]].concat()) {
            Value::Enum { raw, .. } => raw,
            _ => continue,
        };
        let Some((_, name)) = reads_as.iter().find(|(t, _)| *t == tag) else { continue };
        if ev.node(doc, &[at.clone(), vec![4]].concat()).unwrap().child_count == 0 {
            continue;
        }
        out.by_ref.1 += 1;
        if ev.node(doc, &[at, vec![4, 0]].concat()).unwrap().type_name == *name {
            out.by_ref.0 += 1;
        }
    }
    out
}

/// Every reference a sample writes is followed, whichever block holds the
/// thing it names. Before the index covered every block, `ntcheck.hdf` opened
/// 7 of its 8 datasets, `litend.hdf` 6 of 8, and `tvattr.hdf` the rows of 15
/// of its 17 tables; the counts here are what a walk of the descriptor tables
/// says each file holds.
#[test]
fn every_reference_is_followed_whichever_block_holds_what_it_names() {
    let files = samples();
    if files.is_empty() {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    }
    for (name, bytes) in &files {
        let doc = Document::new(MemSource(bytes.clone()));
        let mut ev = Evaluator::new(formats::builtin("hdf4").unwrap());
        let found = walk(&mut ev, &doc, name);
        let got = reached(&mut ev, &doc, &found);
        eprintln!("{name}: {got:?}");
        let want = match name.as_str() {
            "Image_with_Palette.hdf" => Reached { datasets: (0, 0), members: (5, 5), attributes: (0, 0), by_ref: (1, 1) },
            // The last vgroup names a vgroup with reference number 0, which
            // no descriptor can have, and that member points nowhere.
            "grtdfui83.hdf" => Reached { datasets: (0, 0), members: (11, 12), attributes: (0, 0), by_ref: (4, 4) },
            "litend.hdf" => Reached { datasets: (8, 8), members: (16, 16), attributes: (0, 0), by_ref: (0, 0) },
            "swf32.hdf" => Reached { datasets: (2, 2), members: (18, 18), attributes: (0, 0), by_ref: (2, 2) },
            "ntcheck.hdf" => Reached { datasets: (8, 8), members: (36, 36), attributes: (0, 0), by_ref: (14, 14) },
            // Every dataset's values are in linked blocks. The six members
            // naming those values name the plain tag while the file holds only
            // the special element's, and are found under that, the way pyhdf's
            // library finds them; so is each dataset's data.
            "tdata.hdf" => Reached { datasets: (3, 3), members: (36, 36), attributes: (0, 0), by_ref: (3, 3) },
            // Fifteen attributes, as pyhdf counts them: eleven on two tables
            // and their columns, and two on each of two vgroups.
            "tvattr.hdf" => Reached { datasets: (0, 0), members: (2, 2), attributes: (15, 15), by_ref: (17, 17) },
            other => panic!("{other} is in the collection with nothing counted of it"),
        };
        assert_eq!(got, want, "{name}");
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
            // Seven datasets under eight groups, since the first is named by
            // both the old scientific data group and the numeric data group,
            // and the group of the sixth is a block on from its members. Each
            // is ten by ten counting up along the second dimension, which is
            // what pyhdf reads out of them.
            "ntcheck.hdf" => {
                assert_eq!(of("Hdf4ScientificDataset"), 8);
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
                assert_eq!(ten_by_ten, 8);
                // The class byte is read for what the type makes it: 1 is
                // IEEE where the type is a float and Motorola byte order
                // where it is a whole number, and naming it one way for both
                // would call half of them something the file did not say.
                assert!(of("Hdf4FloatClass") > 0, "a float's class byte");
                assert!(of("Hdf4IntClass") > 0, "a whole number's class byte");
                // Every dataset has a scale along its first dimension and
                // none along its second, and the flag bytes are the only
                // thing that says so.
                assert_eq!(of("Hdf4SdScales"), 7, "one scales record per dataset");
                for at in found["Hdf4SdScales"].clone() {
                    assert_eq!(value(&mut ev, &doc, &[at.clone(), vec![1, 0]].concat()), Value::UInt(1));
                    assert_eq!(value(&mut ev, &doc, &[at.clone(), vec![1, 1]].concat()), Value::UInt(0));
                    assert_eq!(ev.node(&doc, &[at.clone(), vec![2, 0, 2]].concat()).unwrap().child_count, 10);
                    assert_eq!(ev.node(&doc, &[at, vec![2, 1, 2]].concat()).unwrap().size_bits, 0);
                }
                // A maximum and a minimum in the dataset's own number type,
                // for every dataset, including the one whose number type
                // record is in the block before.
                assert_eq!(of("Hdf4MaxAndMin"), 7);
            }
            // The same datasets the other way round. A value of 1 is written
            // `01 00` and reads as 1, not as 256.
            "litend.hdf" => {
                // Eight groups, and two of them name members in the block
                // before their own.
                assert_eq!(of("Hdf4ScientificDataset"), 8);
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
            // blocks. What the values read as is checked against pyhdf in
            // `tdata_values_kept_in_linked_blocks_match_pyhdf`.
            "tdata.hdf" => {
                assert_eq!(of("Hdf4ScientificDataset"), 3, "each dataset opens through its linked blocks");
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
                // values are really in, and each read twice: under its own
                // descriptor, and under the dataset that reads the values.
                assert_eq!(of("Hdf4LinkedBlocks"), 6);
                for at in found["Hdf4LinkedBlocks"].clone() {
                    let blocks = number(value(&mut ev, &doc, &[at, vec![2]].concat()));
                    assert!(blocks > 0.0, "a chain of no blocks holds nothing");
                }
            }
            // Seventeen tables, and the rows of each take their columns from
            // the header with the same reference number. That is usually the
            // descriptor after them, twice it is the first slot of the next
            // block, and once it is seven slots on.
            "tvattr.hdf" => {
                assert_eq!(of("Hdf4VdataRecords"), 17);
                let fields = found["Hdf4VdataField"].clone();
                // What pyhdf counts too: records times fields, summed over
                // every vdata `VS.inquire` reports.
                assert_eq!(fields.len(), 27, "one per column per record, over every table with rows");
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
            // One dataset of two by three by four, named by both kinds of
            // group, with a label, a unit and a format for each dimension and
            // none of its own. The strings are the ones pyhdf gives each
            // dimension as `long_name`, `units` and `format`.
            "swf32.hdf" => {
                assert_eq!(of("Hdf4ScientificDataset"), 2);
                let values = [found["Hdf4ScientificDataset"][0].clone(), vec![2, 0]].concat();
                assert_eq!(ev.node(&doc, &values).unwrap().child_count, 2);
                assert_eq!(number(value(&mut ev, &doc, &[values.clone(), vec![0, 1, 2]].concat())), 12.0);
                assert_eq!(number(value(&mut ev, &doc, &[values, vec![1, 2, 3]].concat())), 123.0);
                let strings = found["Hdf4SdStrings"].clone();
                let expected = [["Time", "Line", "Column"], ["Second", "Inch", "Cm"], ["Int32", "Int16", "Int32"]];
                assert_eq!(strings.len(), 3, "labels, units and formats");
                for (at, want) in strings.into_iter().zip(expected) {
                    assert_eq!(value(&mut ev, &doc, &[at.clone(), vec![1]].concat()), Value::Str("".into()));
                    for (i, s) in want.iter().enumerate() {
                        assert_eq!(value(&mut ev, &doc, &[at.clone(), vec![2, i]].concat()), Value::Str((*s).into()));
                    }
                }
            }
            other => panic!("{other} is in the collection with nothing checked of it"),
        }
        checked += 1;
    }
    assert_eq!(checked, 7);
}

/// Every value of `tdata.hdf`'s three datasets, each kept in linked blocks,
/// against what pyhdf reads out of the same file.
///
/// Each dataset has an unlimited first dimension, so the library moved its
/// values into linked blocks when it grew: a first block of what was written
/// before, and a second as long as a block, listed in a link table of 128
/// slots. The values are the two blocks joined and cut at the length the
/// header gives, read in the shape the dimension record gives them. The
/// numbers below are `SD('tdata.hdf').select(i).get().ravel()` for each
/// dataset, from pyhdf on 2026-09-14.
#[test]
fn tdata_values_kept_in_linked_blocks_match_pyhdf() {
    let Some(bytes) = samples().into_iter().find(|(name, _)| name == "tdata.hdf").map(|(_, b)| b) else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    let a: Vec<f64> = (0..5).flat_map(|r| (1..=6).map(move |i| (10 * r + i) as f64)).collect();
    let b = [1, 2, 3, 2, 3, 4, 3, 4, 5, 4, 5, 6, 7, 8, 9].map(f64::from).to_vec();
    let c = [1, 2, 3, 4, 5].map(f64::from).to_vec();
    let want: [(&[i128], Vec<f64>); 3] = [(&[5, 2, 3], a), (&[5, 3], b), (&[5], c)];

    let doc = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(formats::builtin("hdf4").unwrap());
    let found = walk(&mut ev, &doc, "tdata.hdf");
    let under = |kind: &str, at: &[usize]| -> Vec<Vec<usize>> {
        found.get(kind).into_iter().flatten().filter(|p| p.starts_with(at)).cloned().collect()
    };
    let datasets = found["Hdf4ScientificDataset"].clone();
    assert_eq!(datasets.len(), 3);
    let mut seen = Vec::new();
    for dataset in datasets {
        let [dims_at] = &under("Hdf4SdDimensions", &dataset)[..] else { panic!("{dataset:?} has one dimension record") };
        let rank = number(value(&mut ev, &doc, &[dims_at.clone(), vec![0]].concat())) as usize;
        let recorded: Vec<i128> =
            (0..rank).map(|i| number(value(&mut ev, &doc, &[dims_at.clone(), vec![1, i]].concat())) as i128).collect();

        let [linked] = &under("Hdf4LinkedBlocks", &dataset)[..] else {
            panic!("{recorded:?}: the values are in one run of linked blocks")
        };
        let tables = ev.child_named(&doc, linked, "tables").unwrap().unwrap();
        let values = [ev.child_named(&doc, linked, "values").unwrap().unwrap(), vec![0]].concat();
        // The shape the values read in: how many at each level, down the first
        // of each.
        let dims: Vec<i128> = (0..rank)
            .map(|k| ev.node(&doc, &[values.clone(), vec![0; k]].concat()).unwrap().child_count as i128)
            .collect();
        let (_, expected) =
            want.iter().find(|(d, _)| *d == dims.as_slice()).unwrap_or_else(|| panic!("no dataset of {dims:?} in pyhdf"));
        // The dimension record was written before the dataset grew, and says
        // so: every dimension but the unlimited first one is what it says.
        assert_eq!(recorded[1..], dims[1..], "{dims:?}");
        assert!(recorded[0] <= dims[0], "{dims:?}: the record says {recorded:?}");

        let length = number(value(&mut ev, &doc, &[linked.clone(), vec![0]].concat())) as u64;
        assert_eq!(length, expected.len() as u64 * 4, "{dims:?}: int32, four bytes a value");
        assert_eq!(ev.node(&doc, &tables).unwrap().child_count, 1, "{dims:?}: one link table, whose next is nothing");
        let root = ev.node(&doc, &values).unwrap();
        assert!(root.joined && root.space != 0, "{dims:?}: read in the space the blocks make");
        assert_eq!(root.size_bits, length * 8, "{dims:?}: cut at the length, not at the end of the second block");

        // Every value, in the order the leaves come, which is the order the
        // file writes them: the last index is the one that changes fastest.
        let mut got = Vec::new();
        let mut stack = vec![values.clone()];
        while let Some(at) = stack.pop() {
            let node = ev.node(&doc, &at).unwrap();
            if node.child_count == 0 {
                got.push(number(node.value));
                continue;
            }
            for i in (0..node.child_count as usize).rev() {
                stack.push([at.clone(), vec![i]].concat());
            }
        }
        assert_eq!(&got, expected, "{dims:?}");

        // The first block is the dataset as it was before it grew, as long as
        // its own descriptor says, and the byte after it is the first of the
        // second block.
        let first = ev.node(&doc, &[tables.clone(), vec![0, 3, 0, 0]].concat()).unwrap().size_bits / 8;
        assert!(first > 0 && first < length, "{dims:?}: a first block of {first} bytes");
        let hit = ev.part_of(&doc, root.space, first).unwrap().expect("a byte of the values");
        assert_eq!((hit.index, hit.label.as_str(), hit.in_part), (1, "tables[0].blocks[1]", 0), "{dims:?}");
        assert!(!hit.packed);
        seen.push(dims);
    }
    seen.sort();
    assert_eq!(seen, [vec![5], vec![5, 2, 3], vec![5, 3]]);

    // The special element's own descriptor reads the same run as bytes, with
    // nothing to say what they hold.
    let datasets = &found["Hdf4ScientificDataset"];
    let descriptors: Vec<Vec<usize>> =
        found["Hdf4LinkedBlocks"].iter().filter(|p| !datasets.iter().any(|d| p.starts_with(d))).cloned().collect();
    assert_eq!(descriptors.len(), 3);
    for linked in descriptors {
        let length = number(value(&mut ev, &doc, &[linked.clone(), vec![0]].concat())) as u64;
        let values = [ev.child_named(&doc, &linked, "values").unwrap().unwrap(), vec![0]].concat();
        let bytes = ev.field_bytes(&doc, &values, 1 << 16).unwrap().0;
        assert_eq!(bytes.len() as u64, length);
        assert_eq!(bytes[..4], [0, 0, 0, 1], "every dataset starts at 1");
    }
}

/// The same values with each dataset's run of linked blocks opened as a tab of
/// its own, which is what the listing offers on the row.
///
/// What the values are laid out by is not in the run: the dimension record and
/// the number type are fields of the group beside it, so a reading of the
/// run's bytes on their own had nothing to lay them out by and failed at its
/// root. The tab reads them where they were declared, and every value comes
/// out in the same shape, counted from the front of the tab.
#[test]
fn tdata_values_opened_as_tabs_read_as_pyhdf_reads_them() {
    let Some(bytes) = samples().into_iter().find(|(name, _)| name == "tdata.hdf").map(|(_, b)| b) else {
        eprintln!("skipped: set QUBERO_SAMPLES to the sample collection");
        return;
    };
    let doc = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(formats::builtin("hdf4").unwrap());
    let found = walk(&mut ev, &doc, "tdata.hdf");
    let mut read = Vec::new();
    for dataset in found["Hdf4ScientificDataset"].clone() {
        let Some(linked) = found["Hdf4LinkedBlocks"].iter().find(|p| p.starts_with(&dataset)).cloned() else {
            panic!("{dataset:?}: the values are in linked blocks")
        };
        let stream = ev.child_named(&doc, &linked, "values").unwrap().unwrap();
        let id = ev.open_space(&doc, 0, &stream).unwrap().expect("the values open as a tab");
        let root = ev.tab_node(&doc, id, &[]).unwrap();
        let inside = ev.node(&doc, &[stream.clone(), vec![0]].concat()).unwrap();
        assert_eq!((root.offset_bits, root.space, root.joined), (0, 0, false), "{stream:?}: the tab's own bytes");
        assert_eq!((root.size_bits, root.child_count), (inside.size_bits, inside.child_count), "{stream:?}");
        assert_eq!(ev.space(id).unwrap().len_bytes() * 8, root.size_bits, "{stream:?}: the tab is as long as the values");

        // The shape, down the first of each level, and every value in order.
        let mut dims = Vec::new();
        let mut at = Vec::new();
        loop {
            let node = ev.tab_node(&doc, id, &at).unwrap();
            if node.child_count == 0 {
                break;
            }
            dims.push(node.child_count as usize);
            at.push(0);
        }
        let mut values = Vec::new();
        let mut stack = vec![Vec::new()];
        while let Some(at) = stack.pop() {
            let node = ev.tab_node(&doc, id, &at).unwrap();
            assert_eq!(node.path, at);
            if node.child_count == 0 {
                values.push(number(node.value));
                continue;
            }
            for i in (0..node.child_count as usize).rev() {
                stack.push([at.clone(), vec![i]].concat());
            }
        }
        read.push((dims, values));
    }
    read.sort_by(|a, b| a.0.cmp(&b.0));
    let a: Vec<f64> = (0..5).flat_map(|r| (1..=6).map(move |i| (10 * r + i) as f64)).collect();
    let b = [1, 2, 3, 2, 3, 4, 3, 4, 5, 4, 5, 6, 7, 8, 9].map(f64::from).to_vec();
    let c = [1, 2, 3, 4, 5].map(f64::from).to_vec();
    assert_eq!(read, [(vec![5], c), (vec![5, 2, 3], a), (vec![5, 3], b)]);
}
