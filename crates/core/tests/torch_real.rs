//! The `torch.save` files in the sample collection, read as the tensors they
//! hold.
//!
//! What it checks is the thing the pickle alone cannot say: a tensor's numbers
//! are in another entry of the archive, and the table over the tensor is those
//! numbers, cell for cell, at the strides the pickle declared. The values are
//! known because the generator wrote them (`tools/make_torch_joblib_samples.py`
//! in the collection), so the assertions are against `arange` and not against
//! whatever came out.
//!
//! Like the rest of the collection this skips when the folder is not beside
//! the repository. Point `QUBERO_SAMPLES` at it to run it elsewhere.

use std::path::PathBuf;

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::template::Cells;
use qubero_core::formats;
use qubero_core::source::MemSource;

/// Every archive in `torch/` is recognised as one, by its front and by its
/// central directory alike.
#[test]
fn a_torch_archive_is_told_from_an_ordinary_zip() {
    let Some(dir) = folder() else { return };
    let mut seen = 0;
    for path in files(&dir) {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let bytes = std::fs::read(&path).unwrap();
        let head = &bytes[..bytes.len().min(0x9000)];
        // The legacy file is not an archive at all; `torch_legacy.rs` is
        // where that one is read.
        if name.ends_with("legacy.pt") {
            continue;
        }
        assert_eq!(formats::sniff(head, bytes.len() as u64), Some("torchzip"), "{name}: the front of it");
        // And again from the end, which is the path a checkpoint too large to
        // sniff whole takes: the central directory names every entry.
        let tail = &bytes[bytes.len().saturating_sub(formats::SNIFF_TAIL_WINDOW)..];
        assert_eq!(formats::sniff_ends(head, tail, bytes.len() as u64), Some("torchzip"), "{name}: the end of it");
        seen += 1;
    }
    assert!(seen >= 9, "only {seen} archives");
}

/// Each archive's pickle is placed where the archive keeps it, and reads as
/// the form written for what torch writes.
#[test]
fn every_archive_reads_as_the_tensors_it_holds() {
    let Some(dir) = folder() else { return };
    let want: &[(&str, &str)] = &[
        ("checkpoint-zip.pt", "torch-tensors-p2-p3-v1"),
        ("edge-shapes-zip.pt", "torch-tensors-p2-p3-v1"),
        ("every-dtype-zip.pt", "torch-tensors-p2-p3-v1"),
        ("long-storage-zip.pt", "torch-tensors-p2-p3-v1"),
        ("optimizer-state-zip.pt", "torch-tensors-p2-p3-v1"),
        ("parameters-zip.pt", "torch-tensors-p2-p3-v1"),
        ("shared-storage-views-zip.pt", "torch-tensors-p2-p3-v1"),
        ("state-dict-zip.pt", "torch-tensors-p2-p3-v1"),
        ("state-dict-zip-protocol4.pt", "torch-tensors-p4-p5-v1"),
        ("tensor-float64-zip.pt", "torch-tensors-p2-p3-v1"),
    ];
    for (name, form) in want {
        let (doc, mut ev) = open(&dir, name);
        let held = pickle_at(&doc, &mut ev);
        assert_eq!(row(&doc, &mut ev, &held, "header/form"), Value::Str((*form).to_string()), "{name}");
    }
}

/// `layer.weight` is `arange(12).reshape(3, 4)`, and the table over it is
/// those twelve numbers in three rows of four.
#[test]
fn a_tensors_table_is_the_numbers_in_the_entry_it_names() {
    let Some(dir) = folder() else { return };
    let (doc, mut ev) = open(&dir, "state-dict-zip.pt");
    let at = tensor_at(&doc, &mut ev, "layer.weight");
    assert_eq!(row(&doc, &mut ev, &at, "dtype"), Value::Str("float32".into()));
    assert_eq!(row(&doc, &mut ev, &at, "shape"), Value::Str("3 x 4".into()));
    assert_eq!(row(&doc, &mut ev, &at, "numbers"), Value::Str("12 values in data/0".into()));
    // The entry the key names, found in the archive's own records, so the
    // reader can go to those bytes.
    assert_eq!(row(&doc, &mut ev, &at, "stored at"), Value::Str("0x380, 48 bytes".into()));
    let shape = ev.table_shape(&doc, &at).unwrap().unwrap();
    assert!(matches!(shape.cells, Some(Cells::Computed { rows: 3 })), "{:?}", shape.cells);
    let cells = ev.pickle_cells(&doc, &at, 0, 3).unwrap();
    assert_eq!(numbers(&cells), vec![vec![0.0, 1.0, 2.0, 3.0], vec![4.0, 5.0, 6.0, 7.0], vec![8.0, 9.0, 10.0, 11.0]]);
    // The bias is three zeroes in one column, and the step count is one value
    // and so no table at all.
    let bias = tensor_at(&doc, &mut ev, "layer.bias");
    assert_eq!(numbers(&ev.pickle_cells(&doc, &bias, 0, 3).unwrap()), vec![vec![0.0], vec![0.0], vec![0.0]]);
    let steps = tensor_at(&doc, &mut ev, "steps");
    assert_eq!(row(&doc, &mut ev, &steps, "shape"), Value::Str("()".into()));
    assert!(ev.table_shape(&doc, &steps).unwrap().is_none());
}

/// Two windows onto one storage, one of them transposed, read as the two
/// tensors they are rather than as the bytes they share.
#[test]
fn a_view_is_read_at_the_stride_it_declares() {
    let Some(dir) = folder() else { return };
    let (doc, mut ev) = open(&dir, "shared-storage-views-zip.pt");
    // `base = arange(24, float32)`, saved whole, as its tail from twelve, and
    // as `base.reshape(4, 6).t()`, which is the same storage with its strides
    // the other way round.
    let whole = tensor_at(&doc, &mut ev, "whole");
    let cells = ev.pickle_cells(&doc, &whole, 0, 24).unwrap();
    assert_eq!(numbers(&cells).concat(), (0..24).map(|n| n as f64).collect::<Vec<_>>());
    let tail = tensor_at(&doc, &mut ev, "tail");
    assert_eq!(row(&doc, &mut ev, &tail, "storage offset"), Value::Str("12".into()));
    // The same entry, twelve elements in: one storage, two tensors.
    assert_eq!(row(&doc, &mut ev, &tail, "storage"), Value::Str("0".into()));
    assert_eq!(row(&doc, &mut ev, &tail, "stored at"), Value::Str("0x3b0, 48 bytes".into()));
    assert_eq!(numbers(&ev.pickle_cells(&doc, &tail, 0, 12).unwrap()).concat(), (12..24).map(|n| n as f64).collect::<Vec<_>>());
    // The transpose: shape 6 by 4 over a storage laid out 4 by 6, so going
    // along a row of the table steps six values through the file.
    let grid = tensor_at(&doc, &mut ev, "grid");
    assert_eq!(row(&doc, &mut ev, &grid, "shape"), Value::Str("6 x 4".into()));
    assert_eq!(row(&doc, &mut ev, &grid, "stride"), Value::Str("1, 6".into()));
    let cells = numbers(&ev.pickle_cells(&doc, &grid, 0, 6).unwrap());
    assert_eq!(cells[0], vec![0.0, 6.0, 12.0, 18.0]);
    assert_eq!(cells[5], vec![5.0, 11.0, 17.0, 23.0]);
}

/// One tensor of each dtype torch has a storage class for, each holding ones.
#[test]
fn every_dtype_reads_as_the_numbers_it_is() {
    let Some(dir) = folder() else { return };
    let (doc, mut ev) = open(&dir, "every-dtype-zip.pt");
    for word in ["float16", "bfloat16", "float32", "float64", "uint8", "int8", "int16", "int32", "int64", "bool"] {
        let at = tensor_at(&doc, &mut ev, word);
        assert_eq!(row(&doc, &mut ev, &at, "dtype"), Value::Str(word.to_string()), "{word}");
        let cells = ev.pickle_cells(&doc, &at, 0, 4).unwrap();
        let held: Vec<Value> = cells.into_iter().flatten().flatten().collect();
        assert_eq!(held.len(), 4, "{word}");
        for value in held {
            let one = match value {
                Value::Float(f) => f,
                Value::Int(n) => n as f64,
                Value::UInt(n) => n as f64,
                other => panic!("{word}: {other:?}"),
            };
            assert_eq!(one, 1.0, "{word}");
        }
    }
}

/// A module's own weights, which torch wraps in `_rebuild_parameter`.
#[test]
fn a_parameter_reads_as_the_tensor_it_wraps() {
    let Some(dir) = folder() else { return };
    let (doc, mut ev) = open(&dir, "parameters-zip.pt");
    let at = tensor_at(&doc, &mut ev, "layer");
    assert_eq!(row(&doc, &mut ev, &at, "is"), Value::Str("parameter".into()));
    assert_eq!(row(&doc, &mut ev, &at, "requires grad"), Value::Str("True".into()));
    assert_eq!(row(&doc, &mut ev, &at, "shape"), Value::Str("2 x 4".into()));
    let cells = numbers(&ev.pickle_cells(&doc, &at, 0, 2).unwrap());
    assert_eq!(cells, vec![vec![0.0, 1.0, 2.0, 3.0], vec![4.0, 5.0, 6.0, 7.0]]);
}

/// A storage longer than any sniff window, which is the file that proves
/// recognition does not depend on seeing the whole archive.
#[test]
fn a_long_storage_reads_at_both_ends() {
    let Some(dir) = folder() else { return };
    let (doc, mut ev) = open(&dir, "long-storage-zip.pt");
    let at = tensor_at(&doc, &mut ev, "long");
    assert_eq!(row(&doc, &mut ev, &at, "numbers"), Value::Str("100000 values in data/0".into()));
    let cells = numbers(&ev.pickle_cells(&doc, &at, 99_998, 100_000).unwrap());
    assert_eq!(cells, vec![vec![99_998.0], vec![99_999.0]]);
}

/// A shape with no dimensions and a shape with a dimension of nought, which
/// are the two tensors that are not a rectangle.
#[test]
fn the_shapes_that_are_not_a_rectangle_still_read() {
    let Some(dir) = folder() else { return };
    let (doc, mut ev) = open(&dir, "edge-shapes-zip.pt");
    let scalar = tensor_at(&doc, &mut ev, "scalar");
    assert_eq!(row(&doc, &mut ev, &scalar, "numbers"), Value::Str("1 value in data/0".into()));
    assert!(ev.table_shape(&doc, &scalar).unwrap().is_none());
    let empty = tensor_at(&doc, &mut ev, "empty");
    assert_eq!(row(&doc, &mut ev, &empty, "shape"), Value::Str("0 x 3".into()));
    assert_eq!(row(&doc, &mut ev, &empty, "numbers"), Value::Str("0 values in data/1".into()));
    assert!(ev.pickle_cells(&doc, &empty, 0, 4).unwrap().is_empty());
}

/// The summary a reader opens a checkpoint for: one row per tensor, before
/// any of the structure that holds them.
#[test]
fn a_state_dict_opens_as_a_list_of_what_is_in_it() {
    let Some(dir) = folder() else { return };
    let (doc, mut ev) = open(&dir, "state-dict-zip.pt");
    let held = pickle_at(&doc, &mut ev);
    let at = under(&doc, &mut ev, &held, "data");
    let shape = ev.table_shape(&doc, &at).unwrap().unwrap();
    assert_eq!(shape.names, vec!["name", "dtype", "shape", "values"]);
    assert_eq!(shape.row_word.as_deref(), Some("tensor"));
    assert!(matches!(shape.cells, Some(Cells::Computed { rows: 3 })), "{:?}", shape.cells);
    let cells = ev.pickle_cells(&doc, &at, 0, 3).unwrap();
    let said: Vec<Vec<String>> = cells
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| match cell {
                    Some(Value::Str(s)) => s.clone(),
                    Some(Value::UInt(n)) => n.to_string(),
                    other => panic!("{other:?}"),
                })
                .collect()
        })
        .collect();
    assert_eq!(said[0], vec!["layer.weight", "float32", "3 x 4", "12"]);
    assert_eq!(said[1], vec!["layer.bias", "float32", "3", "3"]);
    assert_eq!(said[2], vec!["steps", "int64", "()", "1"]);
    // A checkpoint's top level is not one: it holds an epoch and a note
    // beside the weights. The state dict inside it is.
    let (doc, mut ev) = open(&dir, "checkpoint-zip.pt");
    let held = pickle_at(&doc, &mut ev);
    let top = under(&doc, &mut ev, &held, "data");
    assert!(ev.table_shape(&doc, &top).unwrap().is_none());
    let model = under(&doc, &mut ev, &held, "data/model/value");
    let shape = ev.table_shape(&doc, &model).unwrap().unwrap();
    assert!(matches!(shape.cells, Some(Cells::Computed { rows: 3 })), "{:?}", shape.cells);
}

/// What `optimizer.state_dict()` really holds, which is a state dict with a
/// `_metadata` attribute on it and the optimizer's own bookkeeping beside it.
#[test]
fn a_real_state_dict_reads_with_its_metadata_beside_it() {
    let Some(dir) = folder() else { return };
    let (doc, mut ev) = open(&dir, "optimizer-state-zip.pt");
    let at = { let held = pickle_at(&doc, &mut ev); under(&doc, &mut ev, &held, "data/model/value/attributes") };
    assert_eq!(row(&doc, &mut ev, &at, "_metadata"), Value::Str("OrderedDict of 1".into()));
    let weight = { let held = pickle_at(&doc, &mut ev); under(&doc, &mut ev, &held, "data/model/value/weight/value") };
    assert_eq!(row(&doc, &mut ev, &weight, "shape"), Value::Str("2 x 3".into()));
}

fn open(dir: &PathBuf, name: &str) -> (Document<MemSource>, Evaluator) {
    let bytes = std::fs::read(dir.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    let template = formats::builtin("torchzip").unwrap();
    (Document::new(MemSource(bytes)), Evaluator::new(template))
}

/// One row's value, by its name under `at`.
fn row(doc: &Document<MemSource>, ev: &mut Evaluator, at: &[usize], name: &str) -> Value {
    let here = under(doc, ev, at, name);
    ev.node(doc, &here).unwrap_or_else(|e| panic!("{name}: {e:?}")).value.clone()
}

/// The pickle the checkpoint placed: the one entry it reads, and the pickle
/// inside the node that puts it at its offset in the archive.
fn pickle_at(doc: &Document<MemSource>, ev: &mut Evaluator) -> Vec<usize> {
    let mut here = under(doc, ev, &[], "checkpoint");
    here.push(0);
    under(doc, ev, &here, "data")
}

/// The tensor one entry of the top-level dictionary holds.
fn tensor_at(doc: &Document<MemSource>, ev: &mut Evaluator, name: &str) -> Vec<usize> {
    let held = pickle_at(doc, ev);
    under(doc, ev, &held, &format!("data/{name}/value"))
}

/// The same, from `at`. Walked by asking each node what its children are
/// called, which is what a reader following the tree does. The parts are
/// separated by a slash, because a state dict's own keys hold dots.
fn under(doc: &Document<MemSource>, ev: &mut Evaluator, at: &[usize], name: &str) -> Vec<usize> {
    let mut here = at.to_vec();
    for part in name.split('/') {
        let node = ev.node(doc, &here).unwrap_or_else(|e| panic!("{name} at {here:?}: {e:?}"));
        let mut found = None;
        for i in 0..node.child_count as usize {
            let mut next = here.clone();
            next.push(i);
            if ev.node(doc, &next).map(|n| n.name == part).unwrap_or(false) {
                found = Some(i);
                break;
            }
        }
        here.push(found.unwrap_or_else(|| panic!("no {part} under {here:?} for {name}")));
    }
    here
}

/// A table's cells as plain numbers, with nothing where the table has none.
fn numbers(cells: &[Vec<Option<Value>>]) -> Vec<Vec<f64>> {
    cells
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| match cell {
                    Some(Value::Float(f)) => *f,
                    Some(Value::Int(n)) => *n as f64,
                    Some(Value::UInt(n)) => *n as f64,
                    other => panic!("{other:?}"),
                })
                .collect()
        })
        .collect()
}

fn folder() -> Option<PathBuf> {
    let dir = qubero_samples::root()?.join("torch");
    dir.is_dir().then_some(dir)
}

fn files(dir: &PathBuf) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "pt"))
        .collect();
    out.sort();
    out
}

/// The legacy file, which is not an archive: five pickles in a row and then
/// the storages. It is recognised by its first pickle, which is the same
/// fifteen bytes in every file torch has written this way, and read as fields
/// at the offsets a walk of those pickles works out.
#[test]
fn a_legacy_checkpoint_reads_as_five_pickles_and_its_storages() {
    let Some(dir) = folder() else { return };
    let bytes = std::fs::read(dir.join("state-dict-legacy.pt")).unwrap();
    let head = &bytes[..bytes.len().min(0x9000)];
    assert_eq!(formats::sniff(head, bytes.len() as u64), Some("torchlegacy"));
    let (doc, mut ev) = legacy(&dir, "state-dict-legacy.pt");
    let placed = placements(&doc, &mut ev);
    let names: Vec<&str> = placed.iter().map(|(n, ..)| n.as_str()).collect();
    assert_eq!(&names[..5], ["magic number", "protocol version", "system info", "data", "storage keys"]);
    // Three storages, one per tensor, named by the key the data pickle gave
    // each of them.
    assert_eq!(names.len(), 5 + 3 * 2);
    // Every byte of the file is one of those fields: the five pickles, and
    // then a count and a run of numbers per storage.
    let mut want = 0;
    for (name, at, end) in &placed {
        assert_eq!(*at, want, "{name} leaves bytes over at {want:#x}");
        want = *end;
    }
    assert_eq!(want, bytes.len() as u64, "bytes left over at {want:#x}");
    // A storage says how many elements it holds and never how wide one is.
    // What says that is the storage class in the persistent id of whichever
    // tensor names the key, so the run is typed by reading the data pickle.
    // The generator saved three float32 zeroes, one int64 and twelve
    // float32 counts, in the order sorted keys put them.
    let root = ev.node(&doc, &[]).unwrap();
    let mut held = Vec::new();
    for i in 5..root.child_count as usize {
        let count = ev.node(&doc, &[i, 0, 0]).unwrap();
        let numbers = ev.node(&doc, &[i, 1, 0]).unwrap();
        held.push((count.value.clone(), numbers.type_name.clone(), numbers.child_count));
    }
    assert_eq!(held[0], (Value::Int(3), "f32 le[]".to_string(), 3));
    assert_eq!(held[1], (Value::Int(1), "i64 le[]".to_string(), 1));
    assert_eq!(held[2], (Value::Int(12), "f32 le[]".to_string(), 12));
}

/// A legacy checkpoint opened under its own template.
fn legacy(dir: &PathBuf, name: &str) -> (Document<MemSource>, Evaluator) {
    let bytes = std::fs::read(dir.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    (Document::new(MemSource(bytes)), Evaluator::new(formats::builtin("torchlegacy").unwrap()))
}

/// Every run of the file the template placed, in file order: the name it goes
/// by and the bytes it covers.
///
/// A field placed at an offset covers no bytes itself and holds the thing it
/// placed, so the run is the child's.
fn placements(doc: &Document<MemSource>, ev: &mut Evaluator) -> Vec<(String, u64, u64)> {
    let mut out = Vec::new();
    let root = ev.node(doc, &[]).unwrap();
    for i in 0..root.child_count as usize {
        let node = ev.node(doc, &[i]).unwrap();
        // A storage is a structure of two placed fields; a pickle is one.
        let paths: Vec<Vec<usize>> = match node.type_name.starts_with("at ") {
            true => vec![vec![i, 0]],
            false => (0..node.child_count as usize).map(|j| vec![i, j, 0]).collect(),
        };
        for path in paths {
            let held = ev.node(doc, &path).unwrap();
            out.push((held.name.clone(), held.offset_bits / 8, (held.offset_bits + held.size_bits) / 8));
        }
    }
    out
}

/// A legacy checkpoint's tensors open as the same tables the archive's do.
///
/// The same state dict saved both ways: one as a ZIP with a storage per entry,
/// one as five pickles and the numbers after them. Nothing of that reaches the
/// reader, so the two files open as the same tensors, cell for cell.
#[test]
fn a_legacy_tensor_opens_as_the_table_the_archive_holds() {
    let Some(dir) = folder() else { return };
    let (zip_doc, mut zip_ev) = open(&dir, "state-dict-zip.pt");
    let (old_doc, mut old_ev) = legacy(&dir, "state-dict-legacy.pt");
    for name in ["layer.weight", "layer.bias", "steps"] {
        let new_at = tensor_at(&zip_doc, &mut zip_ev, name);
        let old_at = legacy_tensor_at(&old_doc, &mut old_ev, name);
        // Everything but the storage's name, which is an entry number in the
        // archive and the address the storage happened to be at in the
        // legacy file. Neither says anything about the tensor.
        for said in ["dtype", "shape", "stride", "storage offset", "requires grad"] {
            assert_eq!(row(&old_doc, &mut old_ev, &old_at, said), row(&zip_doc, &mut zip_ev, &new_at, said), "{name}: {said}");
        }
        let want = zip_ev.pickle_cells(&zip_doc, &new_at, 0, 64).unwrap();
        let said = old_ev.pickle_cells(&old_doc, &old_at, 0, 64).unwrap();
        assert_eq!(numbers(&said), numbers(&want), "{name}");
        // And the row saying where the numbers are points into this file,
        // which for a legacy checkpoint is the run after the fifth pickle.
        let stored = row(&old_doc, &mut old_ev, &old_at, "stored at");
        assert!(matches!(&stored, Value::Str(s) if s.contains("0x")), "{name}: {stored:?}");
    }
}

/// The tensor one entry of a legacy checkpoint's data pickle holds.
fn legacy_tensor_at(doc: &Document<MemSource>, ev: &mut Evaluator, name: &str) -> Vec<usize> {
    let held = under(doc, ev, &[], "data/data");
    under(doc, ev, &held, &format!("data/{name}/value"))
}

/// Every dtype and every view read the legacy way too.
///
/// A dtype is a storage class in a persistent id, and it is what says how wide
/// one element of the run after the fifth pickle is, so a file holding ten of
/// them is where a width read from the wrong place shows. Three tensors over
/// one storage is the other: the legacy format writes that storage once, and
/// the three windows onto it are told apart only by their offsets and strides.
#[test]
fn every_legacy_dtype_and_every_legacy_view_reads() {
    let Some(dir) = folder() else { return };
    let (doc, mut ev) = legacy(&dir, "every-dtype-legacy.pt");
    for word in ["float16", "bfloat16", "float32", "float64", "uint8", "int8", "int16", "int32", "int64", "bool"] {
        let at = legacy_tensor_at(&doc, &mut ev, word);
        assert_eq!(row(&doc, &mut ev, &at, "dtype"), Value::Str(word.to_string()), "{word}");
        // Four ones. A tensor of one dimension is one value a row.
        let cells = ev.pickle_cells(&doc, &at, 0, 8).unwrap();
        assert_eq!(numbers(&cells), vec![vec![1.0]; 4], "{word}");
    }
    // One storage of 24 floats under three tensors: the whole of it, its tail
    // from element twelve, and a transpose whose strides are the other way
    // round. The same three tables the archive holds.
    let (zip_doc, mut zip_ev) = open(&dir, "shared-storage-views-zip.pt");
    let (old_doc, mut old_ev) = legacy(&dir, "shared-storage-views-legacy.pt");
    for name in ["whole", "tail", "grid"] {
        let new_at = tensor_at(&zip_doc, &mut zip_ev, name);
        let old_at = legacy_tensor_at(&old_doc, &mut old_ev, name);
        let want = zip_ev.pickle_cells(&zip_doc, &new_at, 0, 64).unwrap();
        let said = old_ev.pickle_cells(&old_doc, &old_at, 0, 64).unwrap();
        assert_eq!(numbers(&said), numbers(&want), "{name}");
    }
}
