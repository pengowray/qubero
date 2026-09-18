//! What `joblib.dump` writes: the wrapper in place of an array, the padding
//! in front of the numbers, and the numbers themselves.
//!
//! The fixture is one real file, `numpy.arange(6, dtype="float64")
//! .reshape(2, 3)` dumped by joblib 1.6, and every refusal here is that file
//! with one run replaced by another of the same length, so that nothing but
//! the thing under test changes.

use super::*;

/// One array, dumped on its own: `joblib.dump(numpy.arange(6,
/// dtype="float64").reshape(2, 3), path)`.
const SMALL: &[u8] = include_bytes!("../../../../../tests/fixtures/pickle/joblib-array-small.joblib");

/// Where the padding count byte sits, where the numbers start, and where they
/// end. Read off the fixture once so the tests below can say what they mean.
const PAD_AT: usize = 222;
const DATA_AT: usize = 224;
const DATA_LEN: usize = 48;

fn edited(from: &[u8], to: &[u8]) -> Vec<u8> {
    let mut bytes = SMALL.to_vec();
    assert_eq!(from.len(), to.len(), "an edit has to keep every other offset where it was");
    replace(&mut bytes, from, to);
    bytes
}

#[test]
fn an_array_joblib_wrote_is_the_array_a_plain_pickle_would_hold() {
    let found = recognise(SMALL).unwrap();
    assert_eq!(found.form, "joblib-arrays-p4-p5-v1");
    assert_eq!(found.proto, 4);
    let Kind::Array { at, len, dtype, dimensions, fortran_order, storage } = &found.value.kind else { panic!("array") };
    assert_eq!((spelling(dtype), dimensions.as_slice(), *fortran_order), ("<f8", &[2, 3][..], false));
    // The numbers are where joblib put them, which is past the padding and
    // nowhere the file states.
    assert_eq!((*at, *len, *storage), (DATA_AT, DATA_LEN, Storage::Raw));
    assert_eq!(SMALL[DATA_AT..DATA_AT + 8], 0f64.to_le_bytes());
    // The wrapper is folded away: what is left is the array, covering the
    // whole run from the class the file named to the last of the numbers.
    assert_eq!((found.value.at, found.value.at + found.value.len), (11, DATA_AT + DATA_LEN));
}

/// The rows a reader sees: the array says what it is, the run of instructions
/// that describes it is one field, the padding is named, and nothing is left
/// over anywhere.
#[test]
fn the_wrapper_the_padding_and_the_numbers_are_each_a_row() {
    let seen = dump(SMALL);
    assert_eq!(named_row(&seen, "dtype").value, V::Str("<f8".into()));
    assert_eq!(named_row(&seen, "shape").value, V::Str("2 x 3".into()));
    assert_eq!(named_row(&seen, "order").value, V::Str("C".into()));
    // The class is on the run of instructions the form matched, so a reader
    // can see which object the file wrote in place of the array.
    let wrapper = named_row(&seen, "wrapper");
    assert_eq!((wrapper.at, wrapper.at + wrapper.len), (11, PAD_AT as u64));
    assert_eq!(named_row(&seen, "wrapper module").value, V::Str("joblib.numpy_pickle".into()));
    assert_eq!(named_row(&seen, "wrapper class").value, V::Str("NumpyArrayWrapper".into()));
    assert_eq!(named_row(&seen, "array class").value, V::Str("ndarray".into()));
    // The padding is bytes nothing reads, and says so.
    let padding = named_row(&seen, "padding");
    assert_eq!((padding.at, padding.len, padding.machinery), (PAD_AT as u64, (DATA_AT - PAD_AT) as u64, true));
    let numbers = named_row(&seen, "numbers");
    assert_eq!((numbers.at, numbers.len, numbers.ty.as_str()), (DATA_AT as u64, DATA_LEN as u64, "f64 le[]"));
    assert_eq!(numbers.value, V::Composite { count: 6 });
    tiles(&seen);
}

/// A list of three arrays, dumped in one go: `joblib.dump([arange(4, "int8"),
/// arange(4, "int8"), arange(3, "float32")], path)`.
const THREE: &[u8] = include_bytes!("../../../../../tests/fixtures/pickle/joblib-three-arrays.joblib");

/// A file holding several arrays: each one's bytes sit between the opcodes,
/// the file carries on after them, and the second and third wrapper name the
/// class and the attribute words out of the memo instead of spelling them.
#[test]
fn every_array_of_a_file_has_its_own_run_and_the_pickle_goes_on() {
    let found = recognise(THREE).unwrap();
    assert_eq!(found.form, "joblib-arrays-p4-p5-v1");
    let Kind::List(items) = &found.value.kind else { panic!("list") };
    assert_eq!(items.len(), 3);
    let mut reached = 0;
    for item in items {
        let Kind::Array { at, len, .. } = item.kind else { panic!("{item:?}") };
        assert!(at > reached, "each array's numbers come after the last one's");
        reached = at + len;
    }
    let seen = dump(THREE);
    assert_eq!(seen.iter().filter(|r| r.name == "padding").count(), 3);
    assert_eq!(seen.iter().filter(|r| r.name == "numbers").count(), 3);
    tiles(&seen);
}

/// The padding count is not read and believed: joblib works it out from where
/// the byte sits, so a file saying anything else was not written by it.
#[test]
fn the_padding_has_to_be_what_the_alignment_asks_for() {
    assert_eq!(SMALL[PAD_AT], 1, "the fixture pads by one byte");
    // The count byte, one either way.
    for wrong in [0u8, 2, 16] {
        let mut bytes = SMALL.to_vec();
        bytes[PAD_AT] = wrong;
        assert!(recognise(&bytes).is_none(), "padding of {wrong}");
    }
    // The padding itself, which is 0xff and never anything else.
    let mut zeroed = SMALL.to_vec();
    zeroed[PAD_AT + 1] = 0;
    assert!(recognise(&zeroed).is_none());
    // An alignment the one byte of padding no longer reaches. Every power of
    // two up to 32 asks for one byte where this byte sits, so 64 is the
    // nearest alignment that would have been written differently.
    assert!(recognise(&edited(b"K\x10ub", b"K\x40ub")).is_none());
    // And a number that is no alignment at all.
    assert!(recognise(&edited(b"K\x10ub", b"K\x0bub")).is_none());
}

/// The run is as long as the shape and the dtype say, and a file claiming
/// more than it holds is a non-match rather than a read past the end.
#[test]
fn the_numbers_are_as_many_as_the_wrapper_measures() {
    // Four rows of three where the file holds two: 96 bytes wanted, 48 there.
    assert!(recognise(&edited(b"K\x02K\x03", b"K\x04K\x03")).is_none());
    // And fewer, which leaves bytes over before the STOP.
    assert!(recognise(&edited(b"K\x02K\x03", b"K\x01K\x03")).is_none());
}

/// The state is these six attributes and no others. A key this has not been
/// measured against is a file whose wrapper means something else.
#[test]
fn a_state_key_the_form_has_not_seen_is_a_non_match() {
    assert!(recognise(&edited(b"allow_mmap", b"allow_mmaq")).is_none());
    assert!(recognise(&edited(b"subclass", b"subclasz")).is_none());
    assert!(recognise(&edited(b"numpy_array_alignment_bytes", b"numpy_array_alignment_byteZ")).is_none());
}

/// `allow_mmap` is a flag, written with the opcode for one. Anything else in
/// its place is not what joblib writes.
#[test]
fn allow_mmap_is_written_as_a_flag() {
    // The file says True; False is the other thing joblib writes there, and
    // it is read.
    assert!(recognise(&edited(b"allow_mmap\x94\x88", b"allow_mmap\x94\x89")).is_some());
    // Nothing else there is one.
    assert!(recognise(&edited(b"allow_mmap\x94\x88", b"allow_mmap\x94N")).is_none());
    assert!(recognise(&edited(b"allow_mmap\x94\x88", b"allow_mmap\x94]")).is_none());
}

/// Only this one class, in this one module. A module under `joblib` is not a
/// licence to name whatever is in it.
#[test]
fn only_the_wrapper_joblib_writes_is_named() {
    assert!(recognise(&edited(b"joblib.numpy_pickle", b"joblib.numpy_picklX")).is_none());
    assert!(recognise(&edited(b"NumpyArrayWrapper", b"NumpyArrayWrappeR")).is_none());
    assert!(recognise(&edited(b"NumpyArrayWrapper", b"SomethingElseHere")).is_none());
    // The array's own class is `numpy.ndarray` and nothing else yet.
    assert!(recognise(&edited(b"\x8c\x07ndarray", b"\x8c\x07ndarraz")).is_none());
}

/// A file with no wrapper in it is not read under a joblib form, whatever
/// else it holds. The plain NumPy form reads that one.
#[test]
fn a_pickle_with_no_wrapper_in_it_is_not_a_joblib_file() {
    let found = recognise(MATRIX).unwrap();
    assert_eq!(found.form, "numpy-array-p4-p5-v6");
}
