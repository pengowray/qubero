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
use qubero_core::eval::{Evaluator, FrameCell, Value};
use qubero_core::template::Cells;
use qubero_core::formats;
use qubero_core::source::MemSource;

/// Every archive in `torch/` is recognised as one, by its front and by its
/// central directory alike.
///
/// The one exception is what torch 1.5 wrote. Every entry's local header sets
/// the streaming flag and says nought for both sizes, so a walk of the front
/// gets no further than the first record, and 1.5 put `version` there rather
/// than `data.pkl`. That file is recognised from its central directory alone,
/// which is the question the editor asks of every archive.
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
        if !name.starts_with("v1.5-") {
            assert_eq!(formats::sniff(head, bytes.len() as u64), Some("torchzip"), "{name}: the front of it");
        }
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
        // What torch 2.14 writes that the older storage classes have no name
        // for, and the kinds the form refused until 2026-09-19.
        ("dtype-complex64-zip.pt", "torch-tensors-p2-p3-v1"),
        ("dtype-complex128-zip.pt", "torch-tensors-p2-p3-v1"),
        ("dtype-float8-e4m3fn-zip.pt", "torch-tensors-p2-p3-v1"),
        ("dtype-float8-e5m2-zip.pt", "torch-tensors-p2-p3-v1"),
        ("dtype-uint16-zip.pt", "torch-tensors-p2-p3-v1"),
        ("dtype-uint32-zip.pt", "torch-tensors-p2-p3-v1"),
        ("dtype-uint64-zip.pt", "torch-tensors-p2-p3-v1"),
        ("quantized-int8-zip.pt", "torch-tensors-p2-p3-v1"),
        ("sparse-coo-zip.pt", "torch-tensors-p2-p3-v1"),
        ("size-device-dtype-zip.pt", "torch-tensors-p2-p3-v1"),
        ("whole-module-zip.pt", "torch-tensors-p2-p3-v1"),
        ("module-state-dict-zip.pt", "torch-tensors-p2-p3-v1"),
        ("checkpoint-sgd-momentum-zip.pt", "torch-tensors-p2-p3-v1"),
    ];
    for (name, form) in want {
        let (doc, mut ev) = open(&dir, name);
        let held = pickle_at(&doc, &mut ev);
        assert_eq!(row(&doc, &mut ev, &held, "header/form"), Value::Str((*form).to_string()), "{name}");
    }
}

/// `layer.weight` is `arange(12).reshape(3, 4)`, and its numbers are a field
/// of their own at the entry the key names: twelve `f32` at 0x380, with a
/// table over them of three rows of four.
#[test]
fn a_tensors_numbers_are_a_field_in_the_entry_it_names() {
    let Some(dir) = folder() else { return };
    let (doc, mut ev) = open(&dir, "state-dict-zip.pt");
    let at = tensor_at(&doc, &mut ev, "layer.weight");
    assert_eq!(row(&doc, &mut ev, &at, "dtype"), Value::Str("float32".into()));
    assert_eq!(row(&doc, &mut ev, &at, "shape"), Value::Str("3 x 4".into()));
    // The numbers themselves, placed in the entry's data rather than
    // described by a row: where they are, how long they are, and what they
    // read as.
    let held = under(&doc, &mut ev, &at, "numbers");
    let node = ev.node(&doc, &held).unwrap();
    assert_eq!((node.offset_bits / 8, node.size_bits / 8), (0x380, 48));
    assert_eq!(node.child_count, 12);
    assert_eq!(values(&doc, &mut ev, &held), (0..12).map(|n| n as f64).collect::<Vec<_>>());
    // And they are the entry's own bytes, read from the file itself rather
    // than through the tree.
    let bytes = std::fs::read(dir.join("state-dict-zip.pt")).unwrap();
    let said: Vec<f32> = bytes[0x380..0x380 + 48].chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect();
    assert_eq!(said, (0..12).map(|n| n as f32).collect::<Vec<_>>());
    // The table is the ordinary one a run of numbers gets: the last axis is
    // the columns, and nothing works the cells out.
    let shape = ev.table_shape(&doc, &held).unwrap().unwrap();
    assert_eq!(shape.columns, Some(4));
    assert!(shape.cells.is_none(), "{:?}", shape.cells);
    // The tensor itself is no longer a table: the numbers under it are.
    assert!(ev.table_shape(&doc, &at).unwrap().is_none());
    // The bias is three zeroes, and the step count is one value, which is a
    // field of one number and no table.
    let held = tensor_at(&doc, &mut ev, "layer.bias");
    let bias = under(&doc, &mut ev, &held, "numbers");
    assert_eq!(values(&doc, &mut ev, &bias), vec![0.0, 0.0, 0.0]);
    let steps = tensor_at(&doc, &mut ev, "steps");
    assert_eq!(row(&doc, &mut ev, &steps, "shape"), Value::Str("()".into()));
    assert!(ev.table_shape(&doc, &steps).unwrap().is_none());
    let one = under(&doc, &mut ev, &steps, "numbers");
    assert!(ev.table_shape(&doc, &one).unwrap().is_none());
    assert_eq!(values(&doc, &mut ev, &one).len(), 1);
}

/// The numbers are outside the tensor that named them, and what is left under
/// it still tiles its own bytes.
///
/// The one rule a field placed elsewhere has to keep, which is the rule an
/// `At` keeps in every other template: the node it hangs off is neither
/// longer nor shorter for it, and nothing in between reads as a gap.
#[test]
fn the_numbers_are_placed_outside_the_tensor_and_leave_it_whole() {
    let Some(dir) = folder() else { return };
    let (doc, mut ev) = open(&dir, "state-dict-zip.pt");
    let at = tensor_at(&doc, &mut ev, "layer.weight");
    let node = ev.node(&doc, &at).unwrap();
    let (from, to) = (node.offset_bits, node.offset_bits + node.size_bits);
    let mut placed = Vec::new();
    let mut want = from;
    for i in 0..node.child_count as usize {
        let mut here = at.to_vec();
        here.push(i);
        let kid = ev.node(&doc, &here).unwrap();
        if kid.offset_bits < from || kid.offset_bits >= to {
            placed.push(kid.name.clone());
            continue;
        }
        // A row worked out from the match covers no bytes and belongs where
        // the tensor starts.
        if kid.size_bits == 0 {
            assert_eq!(kid.offset_bits, from, "{}", kid.name);
            continue;
        }
        assert_eq!(kid.offset_bits, want, "{} leaves bytes over", kid.name);
        want = kid.offset_bits + kid.size_bits;
    }
    assert_eq!(want, to, "bytes left over under the tensor");
    assert_eq!(placed, vec!["numbers".to_string()]);
}

/// Two windows onto one storage, one of them transposed, read as the two
/// tensors they are rather than as the bytes they share.
///
/// The two that read their storage in order are runs of their own, over
/// overlapping bytes: the entry's own reading of those bytes is the archive's
/// and is the one put aside, so nothing is counted twice. The transpose is
/// not a run at all and keeps the table whose cells are worked out.
#[test]
fn a_view_is_read_at_the_stride_it_declares() {
    let Some(dir) = folder() else { return };
    let (doc, mut ev) = open(&dir, "shared-storage-views-zip.pt");
    // `base = arange(24, float32)`, saved whole, as its tail from twelve, and
    // as `base.reshape(4, 6).t()`, which is the same storage with its strides
    // the other way round.
    let held = tensor_at(&doc, &mut ev, "whole");
    let whole = under(&doc, &mut ev, &held, "numbers");
    assert_eq!(values(&doc, &mut ev, &whole), (0..24).map(|n| n as f64).collect::<Vec<_>>());
    let tail = tensor_at(&doc, &mut ev, "tail");
    assert_eq!(row(&doc, &mut ev, &tail, "storage offset"), Value::Str("12".into()));
    // The same entry, twelve elements in: one storage, two tensors.
    assert_eq!(row(&doc, &mut ev, &tail, "storage"), Value::Str("0".into()));
    let tail = under(&doc, &mut ev, &tail, "numbers");
    let node = ev.node(&doc, &tail).unwrap();
    assert_eq!((node.offset_bits / 8, node.size_bits / 8), (0x3b0, 48));
    assert_eq!(values(&doc, &mut ev, &tail), (12..24).map(|n| n as f64).collect::<Vec<_>>());
    // The two runs start twelve elements apart in the same entry, which is
    // one storage read twice over and is what the file says.
    assert_eq!(ev.node(&doc, &whole).unwrap().offset_bits / 8 + 48, node.offset_bits / 8);
    // The transpose: shape 6 by 4 over a storage laid out 4 by 6. Its strides
    // run down the columns rather than along the rows, which is one run of
    // the file all the same, so it is a field and its `order` row says which
    // way the run goes. The table over it is the run as the file holds it:
    // four rows of six, each of them one column of the tensor.
    let grid = tensor_at(&doc, &mut ev, "grid");
    assert_eq!(row(&doc, &mut ev, &grid, "shape"), Value::Str("6 x 4".into()));
    assert_eq!(row(&doc, &mut ev, &grid, "stride"), Value::Str("1, 6".into()));
    assert_eq!(row(&doc, &mut ev, &grid, "order"), Value::Str("Fortran".into()));
    let grid = under(&doc, &mut ev, &grid, "numbers");
    let node = ev.node(&doc, &grid).unwrap();
    assert_eq!((node.offset_bits / 8, node.size_bits / 8), (0x380, 96));
    assert_eq!(ev.table_shape(&doc, &grid).unwrap().unwrap().columns, Some(6));
    assert_eq!(values(&doc, &mut ev, &grid), (0..24).map(|n| n as f64).collect::<Vec<_>>());
}

/// One tensor of each dtype torch has a storage class for, each holding ones.
#[test]
fn every_dtype_reads_as_the_numbers_it_is() {
    let Some(dir) = folder() else { return };
    let (doc, mut ev) = open(&dir, "every-dtype-zip.pt");
    for word in ["float16", "bfloat16", "float32", "float64", "uint8", "int8", "int16", "int32", "int64", "bool"] {
        let at = tensor_at(&doc, &mut ev, word);
        assert_eq!(row(&doc, &mut ev, &at, "dtype"), Value::Str(word.to_string()), "{word}");
        let held = under(&doc, &mut ev, &at, "numbers");
        assert_eq!(values(&doc, &mut ev, &held), vec![1.0; 4], "{word}");
    }
}

/// The dtypes torch 2.14 writes that the legacy storage classes have no name
/// for, each a tensor of `ones(4)`.
///
/// Two ways of writing a tensor meet here. A complex tensor keeps a storage
/// class of its own, so it arrives through `_rebuild_tensor_v2` like every
/// older type; the eight-bit floats and the wide unsigned integers arrive
/// through `_rebuild_tensor_v3`, which names an untyped storage and hands the
/// dtype over as an argument. The claim is the same for both: the dtype in
/// plain words, and four ones read out of the entry.
#[test]
fn the_dtypes_without_a_storage_class_read_as_the_numbers_they_are() {
    let Some(dir) = folder() else { return };
    let want: &[(&str, &str)] = &[
        ("dtype-complex64-zip.pt", "complex64"),
        ("dtype-complex128-zip.pt", "complex128"),
        ("dtype-float8-e4m3fn-zip.pt", "float8_e4m3fn"),
        ("dtype-float8-e5m2-zip.pt", "float8_e5m2"),
        ("dtype-uint16-zip.pt", "uint16"),
        ("dtype-uint32-zip.pt", "uint32"),
        ("dtype-uint64-zip.pt", "uint64"),
    ];
    for (name, word) in want {
        let (doc, mut ev) = open(&dir, name);
        let held = pickle_at(&doc, &mut ev);
        assert_eq!(row(&doc, &mut ev, &held, "header/form"), Value::Str("torch-tensors-p2-p3-v1".into()), "{name}");
        let at = under(&doc, &mut ev, &held, "data");
        assert_eq!(row(&doc, &mut ev, &at, "dtype"), Value::Str((*word).to_string()), "{name}");
        let held = under(&doc, &mut ev, &at, "numbers");
        assert_eq!(ev.node(&doc, &held).unwrap().child_count, 4, "{name}");
        // A complex element is a pair of fields, the real part and then the
        // imaginary one, which is how Python writes a complex number.
        match word.starts_with("complex") {
            true => {
                for i in 0..4 {
                    let mut one = held.clone();
                    one.push(i);
                    assert_eq!(row(&doc, &mut ev, &one, "real"), Value::Float(1.0), "{name}");
                    assert_eq!(row(&doc, &mut ev, &one, "imaginary"), Value::Float(0.0), "{name}");
                }
            }
            false => assert_eq!(values(&doc, &mut ev, &held), vec![1.0; 4], "{name}"),
        }
    }
}

/// A quantised tensor reads as the whole numbers it stores, with what they
/// stand for beside them.
///
/// `quantize_per_tensor(arange(4).float(), 0.5, 0, qint8)` is the integers 0,
/// 2, 4 and 6: the real value divided by the scale. Nothing is dequantised on
/// the way out, so the table is those integers and the scale is a row.
#[test]
fn a_quantised_tensor_reads_as_its_stored_integers() {
    let Some(dir) = folder() else { return };
    let (doc, mut ev) = open(&dir, "quantized-int8-zip.pt");
    let held = pickle_at(&doc, &mut ev);
    let at = under(&doc, &mut ev, &held, "data");
    assert_eq!(row(&doc, &mut ev, &at, "dtype"), Value::Str("qint8".into()));
    assert_eq!(row(&doc, &mut ev, &at, "scale"), Value::Str("0.5".into()));
    assert_eq!(row(&doc, &mut ev, &at, "zero point"), Value::Str("0".into()));
    let held = under(&doc, &mut ev, &at, "numbers");
    assert_eq!(values(&doc, &mut ev, &held), vec![0.0, 2.0, 4.0, 6.0]);
}

/// A sparse tensor reads as the two tensors it is made of.
///
/// `eye(3).to_sparse()` keeps the three ones and the row and column each sits
/// at. Nothing is densified: the indices are one tensor and the values
/// another, and both open as their own table.
#[test]
fn a_sparse_tensor_reads_as_its_indices_and_its_values() {
    let Some(dir) = folder() else { return };
    let (doc, mut ev) = open(&dir, "sparse-coo-zip.pt");
    let held = pickle_at(&doc, &mut ev);
    let data = under(&doc, &mut ev, &held, "data/data");
    let indices = under(&doc, &mut ev, &data, "[0]");
    assert_eq!(row(&doc, &mut ev, &indices, "dtype"), Value::Str("int64".into()));
    assert_eq!(row(&doc, &mut ev, &indices, "shape"), Value::Str("2 x 3".into()));
    let cells = numbers(&ev.pickle_cells(&doc, &indices, 0, 2).unwrap());
    assert_eq!(cells, vec![vec![0.0, 1.0, 2.0], vec![0.0, 1.0, 2.0]]);
    let values = under(&doc, &mut ev, &data, "[1]");
    assert_eq!(row(&doc, &mut ev, &values, "dtype"), Value::Str("float32".into()));
    assert_eq!(numbers(&ev.pickle_cells(&doc, &values, 0, 3).unwrap()).concat(), vec![1.0, 1.0, 1.0]);
}

/// A shape, a device and a dtype saved as values, which is a torch file with
/// no tensor in it at all.
#[test]
fn a_size_a_device_and_a_dtype_read_as_the_values_they_are() {
    let Some(dir) = folder() else { return };
    let (doc, mut ev) = open(&dir, "size-device-dtype-zip.pt");
    let held = pickle_at(&doc, &mut ev);
    assert_eq!(row(&doc, &mut ev, &held, "header/form"), Value::Str("torch-tensors-p2-p3-v1".into()));
    let at = under(&doc, &mut ev, &held, "data");
    assert_eq!(row(&doc, &mut ev, &at, "size"), Value::Str("torch.Size".into()));
    assert_eq!(row(&doc, &mut ev, &at, "device"), Value::Str("torch.device".into()));
    assert_eq!(row(&doc, &mut ev, &at, "dtype"), Value::Str("torch.float32".into()));
}

/// A whole `nn.Module` pickled as an object, which is what the documentation
/// advises against and what people save anyway.
///
/// It is read as a plain object of a class under `torch.nn`, because its state
/// is only what the forms already read: flags, dictionaries, empty
/// `OrderedDict`s of hooks, and the parameters themselves. Nothing about the
/// class is run, and no wider prefix is named.
#[test]
fn a_whole_module_reads_as_an_object_of_a_class_under_torch_nn() {
    let Some(dir) = folder() else { return };
    let (doc, mut ev) = open(&dir, "whole-module-zip.pt");
    let held = pickle_at(&doc, &mut ev);
    assert_eq!(row(&doc, &mut ev, &held, "header/form"), Value::Str("torch-tensors-p2-p3-v1".into()));
    let at = under(&doc, &mut ev, &held, "data");
    assert_eq!(row(&doc, &mut ev, &at, "class"), Value::Str("torch.nn.modules.linear.Linear".into()));
    let weight = under(&doc, &mut ev, &held, "data/_parameters/value/weight/value");
    assert_eq!(row(&doc, &mut ev, &weight, "is"), Value::Str("parameter".into()));
    assert_eq!(row(&doc, &mut ev, &weight, "shape"), Value::Str("3 x 4".into()));
}

/// A real module's state dict and a real checkpoint, which is what torch
/// files in the wild are.
#[test]
fn a_real_state_dict_and_checkpoint_open_as_their_tensors() {
    let Some(dir) = folder() else { return };
    // `nn.Sequential(Linear(4, 3), BatchNorm1d(3))`, whose state dict holds
    // the weights, the batch norm's running statistics and its batch count.
    let (doc, mut ev) = open(&dir, "module-state-dict-zip.pt");
    let held = pickle_at(&doc, &mut ev);
    let at = under(&doc, &mut ev, &held, "data");
    let shape = ev.table_shape(&doc, &at).unwrap().unwrap();
    assert_eq!(shape.names, vec!["name", "dtype", "shape", "values"]);
    let said: Vec<String> = ev.pickle_cells(&doc, &at, 0, 1).unwrap()[0]
        .iter()
        .map(|cell| match &cell.value {
            Some(Value::Str(s)) => s.clone(),
            Some(Value::UInt(n)) => n.to_string(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(said, vec!["0.weight", "float32", "3 x 4", "12"]);
    // An SGD-with-momentum checkpoint: the epoch and the note beside the
    // model's weights and the optimizer's own state.
    let (doc, mut ev) = open(&dir, "checkpoint-sgd-momentum-zip.pt");
    let held = pickle_at(&doc, &mut ev);
    assert_eq!(row(&doc, &mut ev, &held, "data/epoch"), Value::Str("3".into()));
    let model = under(&doc, &mut ev, &held, "data/model/value");
    assert!(ev.table_shape(&doc, &model).unwrap().is_some());
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
    let held = under(&doc, &mut ev, &at, "numbers");
    let node = ev.node(&doc, &held).unwrap();
    assert_eq!(node.child_count, 100_000);
    // The far end of it, read as the field it is rather than the whole run.
    for (i, want) in [(0usize, 0.0), (99_998, 99_998.0), (99_999, 99_999.0)] {
        let mut one = held.clone();
        one.push(i);
        assert_eq!(ev.node(&doc, &one).unwrap().value, Value::Float(want));
    }
}

/// A shape with no dimensions and a shape with a dimension of nought, which
/// are the two tensors that are not a rectangle.
#[test]
fn the_shapes_that_are_not_a_rectangle_still_read() {
    let Some(dir) = folder() else { return };
    let (doc, mut ev) = open(&dir, "edge-shapes-zip.pt");
    let scalar = tensor_at(&doc, &mut ev, "scalar");
    assert!(ev.table_shape(&doc, &scalar).unwrap().is_none());
    let one = under(&doc, &mut ev, &scalar, "numbers");
    assert_eq!(ev.node(&doc, &one).unwrap().child_count, 1);
    let empty = tensor_at(&doc, &mut ev, "empty");
    assert_eq!(row(&doc, &mut ev, &empty, "shape"), Value::Str("0 x 3".into()));
    // No values at all, so the run covers no bytes and holds nothing.
    let none = under(&doc, &mut ev, &empty, "numbers");
    let node = ev.node(&doc, &none).unwrap();
    assert_eq!((node.child_count, node.size_bits), (0, 0));
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
                .map(|cell| match &cell.value {
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

/// The values of a run of numbers, read as the fields they are.
fn values(doc: &Document<MemSource>, ev: &mut Evaluator, at: &[usize]) -> Vec<f64> {
    let count = ev.node(doc, at).unwrap().child_count;
    (0..count as usize)
        .map(|i| {
            let mut here = at.to_vec();
            here.push(i);
            match ev.node(doc, &here).unwrap().value {
                Value::Float(f) => f,
                Value::Int(n) => n as f64,
                Value::UInt(n) => n as f64,
                other => panic!("{other:?}"),
            }
        })
        .collect()
}

/// A table's cells as plain numbers, with nothing where the table has none.
fn numbers(cells: &[Vec<FrameCell>]) -> Vec<Vec<f64>> {
    cells
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| match &cell.value {
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
        let want = under(&zip_doc, &mut zip_ev, &new_at, "numbers");
        let said = under(&old_doc, &mut old_ev, &old_at, "numbers");
        assert_eq!(values(&old_doc, &mut old_ev, &said), values(&zip_doc, &mut zip_ev, &want), "{name}");
        // And the numbers are placed in this file, which for a legacy
        // checkpoint is the run after the fifth pickle.
        let node = old_ev.node(&old_doc, &said).unwrap();
        assert!(node.offset_bits > 0, "{name}");
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
