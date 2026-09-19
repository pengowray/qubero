//! The `torch.save` files older releases wrote, read as the same tensors the
//! newest ones give.
//!
//! `torch_real.rs` is the same claims against what torch 2.14 writes. This one
//! is the files in `torch/` named for the release that wrote them, from the
//! container matrix (`tools/make_torch_joblib_matrix.py` in the collection),
//! and what it checks is that the differences between the eras are differences
//! in the instructions and not in the data. The same state dict saved by torch
//! 0.4 and by torch 2.1 opens as the same three tables, cell for cell.
//!
//! What varies, by release: the backward hooks of a tensor were `None` until
//! 1.0 and an empty `OrderedDict` after; `torch.Size` was closed by NEWOBJ
//! until torch gave the class a `__reduce__`; a parameter was the class called
//! with its tensor until `_rebuild_parameter`; a module saved whole carried
//! the source text of its class until 1.6; the archive's folder, the order of
//! its entries and the storage keys all moved.
//!
//! Like the rest of the collection this skips when the folder is not beside
//! the repository. Point `QUBERO_SAMPLES` at it to run it elsewhere.

use std::path::PathBuf;

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

/// The state dicts of the matrix run, one per release whose file is laid out
/// differently from the one before it, and which container each is.
///
/// `weights` is `arange(12).reshape(3, 4)` as float32, the bias three zeroes
/// and `steps` the 0-d int64 seven, in every one of them.
const STATE_DICTS: &[(&str, &str)] = &[
    ("v0.4-state-dict-legacy.pt", "torchlegacy"),
    ("v1.0-state-dict-legacy.pt", "torchlegacy"),
    ("v1.5-state-dict-legacy.pt", "torchlegacy"),
    ("v1.5-protocol4-legacy.pt", "torchlegacy"),
    ("v1.5-state-dict-zip.pt", "torchzip"),
    ("v1.8-state-dict-zip.pt", "torchzip"),
    ("v1.13-state-dict-zip.pt", "torchzip"),
    ("v2.1-state-dict-zip.pt", "torchzip"),
    ("state-dict-legacy.pt", "torchlegacy"),
    ("state-dict-zip.pt", "torchzip"),
];

/// Every release's file is recognised as the container it is.
///
/// The archive is asked twice, as the editor asks it: from the front, which is
/// the first local record, and from the central directory at the end. torch
/// 1.5 wrote `version` before `data.pkl` and put the ZIP64 records between the
/// directory and the end record, so it is the file that only the second
/// question answers.
#[test]
fn every_release_is_recognised_as_the_container_it_wrote() {
    let Some(dir) = folder() else { return };
    for (name, want) in STATE_DICTS {
        let bytes = std::fs::read(dir.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let head = &bytes[..bytes.len().min(formats::SNIFF_WINDOW)];
        let tail = &bytes[bytes.len().saturating_sub(formats::SNIFF_TAIL_WINDOW)..];
        assert_eq!(formats::sniff_ends(head, tail, bytes.len() as u64), Some(*want), "{name}");
    }
}

/// Every release's state dict opens as the same three tensors, cell for cell.
#[test]
fn every_release_state_dict_holds_the_same_three_tensors() {
    let Some(dir) = folder() else { return };
    for (name, template) in STATE_DICTS {
        let (doc, mut ev) = open(&dir, name, template);
        let held = data_of(&doc, &mut ev, template);
        // A file saved with `pickle_protocol=4` is the same instructions at
        // the other protocol, which is a form of its own.
        let form = match name.contains("protocol4") {
            true => "torch-tensors-p4-p5-v1",
            false => "torch-tensors-p2-p3-v1",
        };
        assert_eq!(row(&doc, &mut ev, &held, "header/form"), Value::Str(form.into()), "{name}");
        let data = under(&doc, &mut ev, &held, "data");

        let weight = under(&doc, &mut ev, &data, "layer.weight/value");
        assert_eq!(row(&doc, &mut ev, &weight, "dtype"), Value::Str("float32".into()), "{name}");
        assert_eq!(row(&doc, &mut ev, &weight, "shape"), Value::Str("3 x 4".into()), "{name}");
        assert_eq!(row(&doc, &mut ev, &weight, "stride"), Value::Str("4, 1".into()), "{name}");
        let cells = numbers(&ev.pickle_cells(&doc, &weight, 0, 3).unwrap());
        assert_eq!(cells, vec![vec![0.0, 1.0, 2.0, 3.0], vec![4.0, 5.0, 6.0, 7.0], vec![8.0, 9.0, 10.0, 11.0]], "{name}");

        let bias = under(&doc, &mut ev, &data, "layer.bias/value");
        assert_eq!(numbers(&ev.pickle_cells(&doc, &bias, 0, 3).unwrap()), vec![vec![0.0], vec![0.0], vec![0.0]], "{name}");

        // A tensor with no dimensions: one value, and no table at all.
        let steps = under(&doc, &mut ev, &data, "steps/value");
        assert_eq!(row(&doc, &mut ev, &steps, "dtype"), Value::Str("int64".into()), "{name}");
        assert_eq!(row(&doc, &mut ev, &steps, "shape"), Value::Str("()".into()), "{name}");
        assert!(ev.table_shape(&doc, &steps).unwrap().is_none(), "{name}");
    }
}

/// The three windows onto one storage, saved the legacy way by torch 1.5,
/// which named each storage by the address its buffer happened to be at.
#[test]
fn an_older_view_is_read_at_the_stride_it_declares() {
    let Some(dir) = folder() else { return };
    let (doc, mut ev) = open(&dir, "v1.5-shared-storage-views-legacy.pt", "torchlegacy");
    let data = under(&doc, &mut ev, &[], "data/data/data");
    // `base = arange(24, float32)`, saved whole, as its tail from twelve, and
    // as `base.reshape(4, 6).t()`.
    let whole = under(&doc, &mut ev, &data, "whole/value");
    assert_eq!(numbers(&ev.pickle_cells(&doc, &whole, 0, 24).unwrap()).concat(), (0..24).map(|n| n as f64).collect::<Vec<_>>());
    let tail = under(&doc, &mut ev, &data, "tail/value");
    assert_eq!(row(&doc, &mut ev, &tail, "storage offset"), Value::Str("12".into()));
    assert_eq!(numbers(&ev.pickle_cells(&doc, &tail, 0, 12).unwrap()).concat(), (12..24).map(|n| n as f64).collect::<Vec<_>>());
    // The transpose: shape 6 by 4 over a storage laid out 4 by 6, so going
    // along a row of the table steps six values through the file.
    let grid = under(&doc, &mut ev, &data, "grid/value");
    assert_eq!(row(&doc, &mut ev, &grid, "shape"), Value::Str("6 x 4".into()));
    assert_eq!(row(&doc, &mut ev, &grid, "stride"), Value::Str("1, 6".into()));
    let cells = numbers(&ev.pickle_cells(&doc, &grid, 0, 6).unwrap());
    assert_eq!(cells[0], vec![0.0, 6.0, 12.0, 18.0]);
    assert_eq!(cells[5], vec![5.0, 11.0, 17.0, 23.0]);
    // One storage behind all three, so the same key and the same run.
    for name in ["tail", "grid"] {
        let held = under(&doc, &mut ev, &data, &format!("{name}/value"));
        assert_eq!(row(&doc, &mut ev, &held, "storage"), row(&doc, &mut ev, &whole, "storage"), "{name}");
    }
}

/// torch 0.4 wrote a parameter as `Parameter(tensor, requires_grad)` rather
/// than through `_rebuild_parameter`, which arrived in 1.0. Both read as the
/// tensor with a note that it is a parameter.
#[test]
fn a_parameter_the_oldest_release_wrote_is_the_tensor_it_wraps() {
    let Some(dir) = folder() else { return };
    let (doc, mut ev) = open(&dir, "v0.4-parameter-legacy.pt", "torchlegacy");
    let at = under(&doc, &mut ev, &[], "data/data/data/p/value");
    assert_eq!(row(&doc, &mut ev, &at, "is"), Value::Str("parameter".into()));
    assert_eq!(row(&doc, &mut ev, &at, "requires grad"), Value::Str("True".into()));
    assert_eq!(row(&doc, &mut ev, &at, "shape"), Value::Str("2 x 3".into()));
    assert_eq!(row(&doc, &mut ev, &at, "parameter call/parameter class"), Value::Str("Parameter".into()));
    // `torch.nn.Parameter(torch.ones(2, 3))`.
    assert_eq!(numbers(&ev.pickle_cells(&doc, &at, 0, 2).unwrap()), vec![vec![1.0; 3]; 2]);
}

/// A shape, a device and a dtype saved as values by torch 0.4, where
/// `torch.Size` had no `__reduce__` of its own and pickle wrote NEWOBJ.
#[test]
fn an_older_size_a_device_and_a_dtype_read_as_the_values_they_are() {
    let Some(dir) = folder() else { return };
    let (doc, mut ev) = open(&dir, "v0.4-size-device-dtype-legacy.pt", "torchlegacy");
    let at = under(&doc, &mut ev, &[], "data/data/data");
    assert_eq!(row(&doc, &mut ev, &at, "size"), Value::Str("torch.Size".into()));
    assert_eq!(row(&doc, &mut ev, &at, "device"), Value::Str("torch.device".into()));
    assert_eq!(row(&doc, &mut ev, &at, "dtype"), Value::Str("torch.float32".into()));
}

/// A module saved whole before torch 1.6, whose class arrives through the
/// persistent id `('module', cls, source_file, source)`.
///
/// The source text is in the file, so it is a row: a reader looking at a
/// checkpoint of a class they do not have wants to see what it was. Nothing is
/// compiled and nothing is run.
#[test]
fn a_module_saved_whole_the_old_way_carries_its_class_source() {
    let Some(dir) = folder() else { return };
    let (doc, mut ev) = open(&dir, "v1.0-whole-module-legacy.pt", "torchlegacy");
    let at = under(&doc, &mut ev, &[], "data/data/data");
    assert_eq!(row(&doc, &mut ev, &at, "class"), Value::Str("torch.nn.modules.linear.Linear".into()));
    let named = under(&doc, &mut ev, &at, "class/module persistent id");
    assert_eq!(row(&doc, &mut ev, &named, "persistent id kind"), Value::Str("module".into()));
    let Value::Str(source) = row(&doc, &mut ev, &named, "class source") else { panic!("the source") };
    assert!(source.starts_with("class Linear(Module):"), "{source:.40}");
    let Value::Str(file) = row(&doc, &mut ev, &named, "class source file") else { panic!("the path") };
    assert!(file.ends_with("torch/nn/modules/linear.py"), "{file}");
    // And the weights are read where they are, which is the run after the
    // fifth pickle.
    let weight = under(&doc, &mut ev, &at, "_parameters/value/weight/value");
    assert_eq!(row(&doc, &mut ev, &weight, "is"), Value::Str("parameter".into()));
    assert_eq!(row(&doc, &mut ev, &weight, "shape"), Value::Str("3 x 4".into()));
    assert_eq!(ev.pickle_cells(&doc, &weight, 0, 3).unwrap().len(), 3);
}

/// A real checkpoint from torch 0.4: the epoch and the note beside the model's
/// weights and the optimizer's own state, which is what an old file on disk
/// is.
#[test]
fn an_older_checkpoint_opens_as_its_weights_and_its_bookkeeping() {
    let Some(dir) = folder() else { return };
    let (doc, mut ev) = open(&dir, "v0.4-checkpoint-legacy.pt", "torchlegacy");
    let at = under(&doc, &mut ev, &[], "data/data/data");
    assert_eq!(row(&doc, &mut ev, &at, "epoch"), Value::Str("3".into()));
    assert_eq!(row(&doc, &mut ev, &at, "note"), Value::Str("after epoch three".into()));
    let model = under(&doc, &mut ev, &at, "model/value");
    // The weights inside it are a state dict, which opens as one row a tensor.
    let shape = ev.table_shape(&doc, &model).unwrap().unwrap();
    assert_eq!(shape.names, vec!["name", "dtype", "shape", "values"]);
    let weight = under(&doc, &mut ev, &model, "weight/value");
    assert_eq!(row(&doc, &mut ev, &weight, "shape"), Value::Str("3 x 4".into()));
    assert_eq!(ev.pickle_cells(&doc, &weight, 0, 3).unwrap().len(), 3);
}

fn open(dir: &PathBuf, name: &str, template: &str) -> (Document<MemSource>, Evaluator) {
    let bytes = std::fs::read(dir.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    (Document::new(MemSource(bytes)), Evaluator::new(formats::builtin(template).unwrap()))
}

/// The data pickle, wherever this container keeps it: the entry the archive
/// places, or the fourth of the legacy file's five pickles.
fn data_of(doc: &Document<MemSource>, ev: &mut Evaluator, template: &str) -> Vec<usize> {
    match template {
        "torchzip" => {
            let mut here = under(doc, ev, &[], "checkpoint");
            here.push(0);
            under(doc, ev, &here, "data")
        }
        _ => under(doc, ev, &[], "data/data"),
    }
}

/// One row's value, by its name under `at`.
fn row(doc: &Document<MemSource>, ev: &mut Evaluator, at: &[usize], name: &str) -> Value {
    let here = under(doc, ev, at, name);
    ev.node(doc, &here).unwrap_or_else(|e| panic!("{name}: {e:?}")).value.clone()
}

/// A node under `at`, walked by asking each node what its children are called.
/// The parts are separated by a slash, because a state dict's own keys hold
/// dots.
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

/// A table's cells as plain numbers.
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
