//! What `torch.save` writes: the call that rebuilds a tensor, the persistent
//! id inside it, and the rows a reader sees over one.
//!
//! The fixture is the `data.pkl` of a real `torch.save` archive, and every
//! refusal here is that file with one run replaced by another of the same
//! length, so that nothing but the thing under test changes.

use super::*;
use crate::formats::pickle::familiar::TensorType;

/// The pickle out of `torch/state-dict-zip.pt`, which is an `OrderedDict` of
/// `layer.weight` (`arange(12).reshape(3, 4)`, float32), `layer.bias` (three
/// zeroes) and `steps` (one int64).
const STATE: &[u8] = include_bytes!("../../../../../tests/fixtures/pickle/torch-state-dict-data.pkl");

fn edited(from: &[u8], to: &[u8]) -> Vec<u8> {
    let mut bytes = STATE.to_vec();
    assert_eq!(from.len(), to.len(), "an edit has to keep every other offset where it was");
    replace(&mut bytes, from, to);
    bytes
}

/// The tensors of a match, in the order the file wrote them.
fn tensors(found: &Match) -> Vec<&crate::formats::pickle::familiar::Tensor> {
    let Kind::Made { state: Some(state), .. } = &found.value.kind else { panic!("an OrderedDict") };
    let Kind::Dict(entries) = &state.kind else { panic!("entries") };
    entries
        .iter()
        .map(|(_, v)| match &v.kind {
            Kind::Tensor(t) => t,
            other => panic!("a tensor, not {other:?}"),
        })
        .collect()
}

#[test]
fn a_state_dict_is_the_tensors_it_holds() {
    let found = recognise(STATE).unwrap();
    assert_eq!(found.form, "torch-tensors-p2-p3-v1");
    assert_eq!(found.proto, 2);
    // A state dict is an `OrderedDict`, and the form reads it without the
    // file counting as the standard library's: `torch` is the one family here.
    assert_eq!(found.extensions(), "torch");
    let held = tensors(&found);
    assert_eq!(held.len(), 3);
    let weight = held[0];
    assert_eq!(weight.dtype, TensorType::Float32);
    assert_eq!(weight.storage_class, "FloatStorage");
    assert_eq!((weight.size.as_slice(), weight.stride.as_slice()), (&[3, 4][..], &[4, 1][..]));
    assert_eq!((weight.offset, weight.count), (0, 12));
    assert!(!weight.requires_grad && !weight.parameter);
    // The storage key and the device are runs of the file, read where they
    // sit, so the rows over them are the file's own bytes.
    assert_eq!(&STATE[weight.key.0..weight.key.0 + weight.key.1], b"0");
    assert_eq!(&STATE[weight.location.0..weight.location.0 + weight.location.1], b"cpu");
    // A tensor with no dimensions at all: one value, and both tuples empty.
    let steps = held[2];
    assert_eq!(steps.dtype, TensorType::Int64);
    assert!(steps.size.is_empty() && steps.stride.is_empty());
    assert_eq!(steps.values(), 1);
}

/// The second and third tensors name the class, the word `storage` and the
/// device out of the memo instead of spelling them again, and the reading is
/// the same either way.
#[test]
fn a_tensor_that_names_its_class_reads_as_one_that_spells_it() {
    let found = recognise(STATE).unwrap();
    let held = tensors(&found);
    let (weight, bias) = (held[0], held[1]);
    assert_eq!(weight.storage_class, bias.storage_class);
    // The device run is the one the first tensor wrote: a BINGET has no bytes
    // of its own to read as text.
    assert_eq!(weight.location, bias.location);
    assert_eq!(&STATE[bias.location.0..bias.location.0 + bias.location.1], b"cpu");
    assert_eq!(bias.key, (209, 1));
    assert_eq!(&STATE[209..210], b"1");
}

/// What a reader sees over a tensor: what one value is, how many there are,
/// and where the numbers are, which is not here.
#[test]
fn a_tensor_says_what_it_is_and_where_its_numbers_are() {
    let seen = dump(STATE);
    assert_eq!(named_row(&seen, "dtype").value, V::Str("float32".into()));
    assert_eq!(named_row(&seen, "shape").value, V::Str("3 x 4".into()));
    assert_eq!(named_row(&seen, "stride").value, V::Str("4, 1".into()));
    assert_eq!(named_row(&seen, "order").value, V::Str("C".into()));
    assert_eq!(named_row(&seen, "storage offset").value, V::Str("0".into()));
    assert_eq!(named_row(&seen, "storage").value, V::Str("0".into()));
    assert_eq!(named_row(&seen, "location").value, V::Str("cpu".into()));
    assert_eq!(named_row(&seen, "requires grad").value, V::Str("False".into()));
    // Nothing in this file holds them: a `data.pkl` on its own is the pickle
    // and none of the storages, so the row says where they are in the format
    // and that they are not here. In an archive the same row is the numbers
    // themselves, placed in the entry it names.
    assert_eq!(named_row(&seen, "numbers").value, V::Str("12 values in data/0, not in this file".into()));
    // The entry row of the dictionary entry above it says the whole of what a
    // reader skimming a state dict wants.
    assert_eq!(named_row(&seen, "layer.weight").value, V::Str("float32 tensor 3 x 4".into()));
    // The names the persistent id was written with are inside the one run of
    // instructions that rebuilt the tensor.
    assert_eq!(named_row(&seen, "rebuild module").value, V::Str("torch._utils".into()));
    assert_eq!(named_row(&seen, "rebuild call").value, V::Str("_rebuild_tensor_v2".into()));
    assert_eq!(named_row(&seen, "storage module").value, V::Str("torch".into()));
    assert_eq!(named_row(&seen, "storage class").value, V::Str("FloatStorage".into()));
    assert_eq!(named_row(&seen, "persistent id kind").value, V::Str("storage".into()));
    assert_eq!(named_row(&seen, "storage key").value, V::Str("0".into()));
    tiles(&seen);
}

/// A storage class nothing has measured is a non-match, not a guess at what
/// the letters mean.
#[test]
fn a_storage_class_no_sample_names_is_refused() {
    // A class of exactly the same width as the one the file names, so that
    // every other offset stays where it was and nothing but the name changes.
    assert!(recognise(&edited(b"ctorch\nFloatStorage\n", b"ctorch\nFloatStorafe\n")).is_none());
}

/// The dtypes are written down twice and the two lists say the same thing.
///
/// [`DTYPES`](crate::formats::pickle::familiar::torch::DTYPES) maps the name
/// torch writes to what one element is, and `DTYPE_NAMES` is the same names as
/// the whole dotted paths a form's `names` column holds. The second cannot be
/// made from the first, because a form's tables are const, so this holds them
/// to each other.
#[test]
fn a_dtype_is_named_in_both_tables() {
    use crate::formats::pickle::familiar::torch::{DTYPES, DTYPE_NAMES};
    let made: Vec<String> = DTYPES.iter().map(|d| format!("torch.{}", d.class)).collect();
    assert_eq!(made, DTYPE_NAMES);
}

/// A persistent id of any other shape is a non-match, whatever it holds.
#[test]
fn a_persistent_id_of_the_wrong_shape_is_refused() {
    // The word the tuple opens with says which of the two persistent ids it
    // is. A legacy file writes `('module', cls, file, source)` for a class
    // whose source it saved, and that one is read somewhere else entirely:
    // neither production takes the other's word.
    assert!(recognise(&edited(b"X\x07\x00\x00\x00storage", b"X\x07\x00\x00\x00storagf")).is_none());
    // The device has to be a text. Five whole numbers in its place make a
    // tuple of nine, the same bytes long and nothing torch wrote.
    assert!(recognise(&edited(b"X\x03\x00\x00\x00cpuq\x07", b"K\x00K\x00K\x00K\x00K\x00")).is_none());
    // And the element count has to be one: two flags in its place make a
    // tuple of six whose last two parts are not a number.
    assert!(recognise(&edited(b"K\x0ctq\x08Q", b"\x88\x89tq\x08Q")).is_none());
}

/// A view has to fit in the storage it is a window onto.
#[test]
fn a_tensor_reaching_past_its_own_storage_is_refused() {
    // The storage holds twelve elements and the shape asks for sixteen.
    assert!(recognise(&edited(b"K\x0ctq\x08QK\x00K\x03K\x04", b"K\x0ctq\x08QK\x00K\x04K\x04")).is_none());
    // The same, reached by starting further in rather than by asking for more.
    assert!(recognise(&edited(b"K\x0ctq\x08QK\x00K\x03K\x04", b"K\x0ctq\x08QK\x01K\x03K\x04")).is_none());
}

/// The backward hooks are empty in every file anything ever saved, and a
/// dictionary with something in it is hooks the tensor would be given.
#[test]
fn hooks_that_are_not_an_empty_ordered_dict_are_refused() {
    // The empty tuple the class is called with becomes an empty dictionary,
    // which is not what `OrderedDict()` is written as.
    assert!(recognise(&edited(b"\x89h\x00)Rq\x0b", b"\x89h\x00}Rq\x0b")).is_none());
}

/// The data pickle of `torch.save({'p': Parameter(...)}, path)` under torch
/// 0.4.1, which is the release before the two spellings below changed. The
/// parameter holds `arange(6).reshape(2, 3)` as float32.
const V04_PARAMETER: &[u8] = include_bytes!("../../../../../tests/fixtures/pickle/torch-v0.4-parameter-data.pkl");
/// The data pickle of `torch.save({'size': .., 'device': .., 'dtype': ..})`
/// under the same release, for the `torch.Size` that NEWOBJ closes.
const V04_SIZE: &[u8] = include_bytes!("../../../../../tests/fixtures/pickle/torch-v0.4-size-data.pkl");

/// torch 0.4 handed `_rebuild_tensor_v2` the tensor's own `_backward_hooks`,
/// which is `None` until a hook is registered. Every tensor that release
/// wrote has a `None` where later files have an empty `OrderedDict()`.
#[test]
fn a_tensor_whose_hooks_are_none_is_the_tensor_torch_0_4_wrote() {
    let found = recognise(V04_PARAMETER).unwrap();
    assert_eq!(found.form, "torch-tensors-p2-p3-v1");
    let Kind::Dict(entries) = &found.value.kind else { panic!("a dictionary") };
    let Kind::Tensor(tensor) = &entries[0].1.kind else { panic!("a tensor") };
    assert_eq!(tensor.dtype, TensorType::Float32);
    assert_eq!((tensor.size.as_slice(), tensor.stride.as_slice()), (&[2, 3][..], &[3, 1][..]));
    assert_eq!(tensor.count, 6);
    // `Parameter.__reduce_ex__` returned the class itself then, called with
    // the tensor and the flag, so the parameter is read from that call rather
    // than from `_rebuild_parameter`.
    assert!(tensor.parameter && tensor.requires_grad);
    let seen = dump(V04_PARAMETER);
    assert_eq!(named_row(&seen, "parameter module").value, V::Str("torch.nn.parameter".into()));
    assert_eq!(named_row(&seen, "parameter class").value, V::Str("Parameter".into()));
    tiles(&seen);
}

/// A tensor whose hooks are neither of the two spellings torch has written is
/// a non-match: a dictionary with something in it is hooks the tensor would
/// be given.
#[test]
fn hooks_that_are_neither_spelling_are_refused() {
    let mut other = V04_PARAMETER.to_vec();
    // The `None` in front of the TUPLE that closes the rebuild call, which is
    // the last of the six arguments.
    let at = other.windows(3).position(|w| w == b"\x88Nt").unwrap() + 1;
    other[at] = 0x88;
    assert!(recognise(&other).is_none());
}

/// `torch.Size` had no `__reduce__` of its own in torch 1.0 and older, so
/// pickle wrote `cls.__new__(cls, (2, 3))` for the tuple subclass. The row for
/// that spelling is the same call closed by NEWOBJ.
#[test]
fn a_size_that_newobj_closes_reads_as_the_one_reduce_closes() {
    let found = recognise(V04_SIZE).unwrap();
    assert_eq!(found.form, "torch-tensors-p2-p3-v1");
    let Kind::Dict(entries) = &found.value.kind else { panic!("a dictionary") };
    let Kind::Made { what: Shape::Size, state: Some(extents), .. } = &entries[0].1.kind else { panic!("a Size") };
    let Kind::Tuple(held) = &extents.kind else { panic!("the extents") };
    assert_eq!(held.len(), 2);
    // The rows are the ones the REDUCE spelling gives: the class the file
    // named and the extents under it, whichever opcode closed the call.
    let seen = dump(V04_SIZE);
    assert_eq!(named_row(&seen, "size").value, V::Str("torch.Size".into()));
    assert_eq!(named_row(&seen, "class").value, V::Str("torch.Size".into()));
    tiles(&seen);
}

/// A NEWOBJ with arguments is only ever one of the enumerated calls. A class
/// the form may name, handed values it is not written with, is a non-match.
#[test]
fn a_newobj_with_arguments_that_no_row_names_is_refused() {
    let mut other = V04_SIZE.to_vec();
    // `torch Size` becomes `torch dtype`, which is the same width and is no
    // row of the calls table.
    replace(&mut other, b"ctorch\nSize\n", b"ctorch\ndtype\n");
    assert!(recognise(&other).is_none());
}

/// The data pickle of `torch.save(torch.nn.Linear(4, 3), path)` under torch
/// 1.0.1, which is the legacy way of saving a module whole: the class arrives
/// through a persistent id carrying its own source text, and the module's
/// state holds the backend that release gave every module.
const V10_MODULE: &[u8] = include_bytes!("../../../../../tests/fixtures/pickle/torch-v1.0-whole-module-data.pkl");

/// A module saved whole before torch 1.6 names its class through
/// `('module', cls, source_file, source)`, which is what `persistent_id`
/// returns for a subclass of `nn.Module`. Nothing is compiled: the source is
/// a run of text with a row of its own.
#[test]
fn a_module_saved_whole_carries_the_source_of_its_class() {
    let found = recognise(V10_MODULE).unwrap();
    assert_eq!(found.form, "torch-tensors-p2-p3-v1");
    let Kind::Instance { class, state: Some(state) } = &found.value.kind else { panic!("an object") };
    let Kind::Class { path, .. } = &class.kind else { panic!("a class") };
    assert_eq!(path, "torch.nn.modules.linear.Linear");
    assert!(matches!(state.kind, Kind::Dict(_)));
    let seen = dump(V10_MODULE);
    assert_eq!(named_row(&seen, "persistent id kind").value, V::Str("module".into()));
    let V::Str(source) = &named_row(&seen, "class source").value else { panic!("the source") };
    assert!(source.starts_with("class Linear(Module):"));
    assert_eq!(named_row(&seen, "weight").value, V::Str("float32 parameter 3 x 4".into()));
    tiles(&seen);
}

/// The class has to be one a saved module's class may be, which is `torch.nn`
/// and nothing wider. A persistent id naming anything else is a non-match.
#[test]
fn a_module_persistent_id_over_another_class_is_refused() {
    let mut other = V10_MODULE.to_vec();
    replace(&mut other, b"ctorch.nn.modules.linear\nLinear\n", b"ctorch.jit.modules.linear\nLinear\n");
    assert!(recognise(&other).is_none());
}

/// `BINPERSID` is read inside the tensor's own run and nowhere else.
#[test]
fn a_persistent_id_outside_a_tensor_is_not_a_familiar_form() {
    // `proto4-persistent-id.pickle` in the collection is two objects named
    // this way and nothing else. Built here so the test says what it means:
    // a dictionary whose value is a persistent id over a tuple of two.
    let body = cat(&[
        b"}\x94\x8c\x07weights\x94(\x8c\x07storage\x94K\x00\x86\x94Qs",
    ]);
    assert!(recognise(&framed(&body)).is_none());
}
