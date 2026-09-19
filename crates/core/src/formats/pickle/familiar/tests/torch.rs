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
    assert_eq!(found.families(), "basic, torch");
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
    assert_eq!(named_row(&seen, "storage offset").value, V::Str("0".into()));
    assert_eq!(named_row(&seen, "storage").value, V::Str("0".into()));
    assert_eq!(named_row(&seen, "location").value, V::Str("cpu".into()));
    assert_eq!(named_row(&seen, "requires grad").value, V::Str("False".into()));
    assert_eq!(named_row(&seen, "numbers").value, V::Str("12 values in data/0".into()));
    // Nothing in this file holds them: a `data.pkl` on its own is the pickle
    // and none of the storages.
    assert_eq!(named_row(&seen, "stored at").value, V::Str("not in this file".into()));
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
    // `ComplexFloatStorage` is one torch really writes and this reader has no
    // sample of. The edit takes a class of exactly the same width instead, so
    // that every other offset in the file stays where it was.
    assert!(recognise(&edited(b"ctorch\nFloatStorage\n", b"ctorch\nFloatStorafe\n")).is_none());
}

/// A persistent id of any other shape is a non-match, whatever it holds.
#[test]
fn a_persistent_id_of_the_wrong_shape_is_refused() {
    // The word the tuple opens with says it is a storage. A legacy file
    // writes `('module', cls, file, source)` for a class whose source it
    // saved, and nothing here reads one.
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
