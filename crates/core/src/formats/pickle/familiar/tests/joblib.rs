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

/// An array of objects, `joblib.dump(numpy.array(["a", None, 3],
/// dtype=object), path)`. joblib has no way to write these values as bytes,
/// so `write_array` hands the array to `pickle.dump` and lets it write a
/// whole pickle into the stream where the numbers would have gone.
const OBJECTS: &[u8] = include_bytes!("../../../../../tests/fixtures/pickle/joblib-array-of-objects.joblib");

/// Where that pickle starts, and where its own STOP leaves off. The outer
/// pickle's STOP is the byte after, which is the last byte of the file.
const NESTED_AT: usize = 220;
const NESTED_END: usize = 376;

fn edited_objects(from: &[u8], to: &[u8]) -> Vec<u8> {
    let mut bytes = OBJECTS.to_vec();
    assert_eq!(from.len(), to.len(), "an edit has to keep every other offset where it was");
    replace(&mut bytes, from, to);
    bytes
}

/// What the wrapper stood for is the array the nested pickle holds, with the
/// values in it rather than a run of bytes nothing can read.
#[test]
fn an_array_of_objects_is_the_array_the_pickle_after_the_wrapper_holds() {
    let found = recognise(OBJECTS).unwrap();
    assert_eq!(found.form, "joblib-arrays-p4-p5-v1");
    // The stream is protocol 4 and the pickle inside it is protocol 5: two
    // writers, each naming its own.
    assert_eq!(found.proto, 4);
    assert_eq!(OBJECTS[NESTED_AT..NESTED_AT + 2], [0x80, 5]);
    let Kind::Objects { dimensions, fortran_order, items, nested } = &found.value.kind else { panic!("object array") };
    assert_eq!((dimensions.as_slice(), *fortran_order, *nested), (&[3][..], false, Some(NESTED_AT)));
    // The value covers the wrapper and the pickle after it, and the outer
    // STOP is the one byte left.
    assert_eq!((found.value.at, found.value.at + found.value.len), (11, NESTED_END));
    assert_eq!(found.ops.last().map(|op| (op.at, op.end)), Some((NESTED_END, OBJECTS.len())));
    // A string, a None and a number, which is more than text and `None`.
    assert!(matches!(items[0].kind, Kind::Text { .. }));
    assert!(matches!(items[1].kind, Kind::None));
    assert!(matches!(items[2].kind, Kind::Int { value: 3, .. }));
}

/// The nested pickle is a row of its own, with the protocol it declared under
/// it, so every byte of the file is accounted for.
#[test]
fn the_nested_pickle_is_a_row_with_a_protocol_of_its_own() {
    let seen = dump(OBJECTS);
    assert_eq!(named_row(&seen, "dtype").value, V::Str("|O8".into()));
    assert_eq!(named_row(&seen, "shape").value, V::Str("3".into()));
    let nested = named_row(&seen, "nested pickle");
    assert_eq!(nested.ty, "nested pickle");
    assert_eq!((nested.at, nested.at + nested.len), (NESTED_AT as u64, NESTED_END as u64));
    // Two protocol rows, the file's and this one's, and they say different
    // numbers.
    let protocols: Vec<&Row> = seen.iter().filter(|r| r.name == "protocol").collect();
    assert_eq!(protocols.len(), 2);
    assert_eq!((protocols[0].at, protocols[0].value.clone()), (1, V::UInt(4)));
    assert_eq!((protocols[1].at, protocols[1].value.clone()), (NESTED_AT as u64 + 1, V::UInt(5)));
    tiles(&seen);
}

/// The memo of the nested pickle is its own: it numbers from nought while the
/// outer stream has filled twenty-odd slots, and the outer stream's slots are
/// still what its own names point at.
#[test]
fn the_nested_pickle_numbers_its_memo_from_nought() {
    // Slot 3 of the inner pickle is `numpy`, which is the word at 273. Under
    // the outer stream's numbering the same slot holds the wrapper object.
    assert_eq!(OBJECTS[312..314], [b'h', 3]);
    assert!(recognise(OBJECTS).is_some());
    // A slot the inner pickle never wrote is a non-match, however many the
    // outer one has.
    assert!(recognise(&edited_objects(b"h\x03\x8c\x05dtype", b"h\x7f\x8c\x05dtype")).is_none());
}

/// One nested pickle after the wrapper and nothing else. A second one, or
/// anything at all between the nested STOP and the opcode the outer stream
/// carries on with, is a file joblib did not write.
#[test]
fn only_one_nested_pickle_follows_the_wrapper() {
    // A second pickle in front of the outer STOP.
    let mut twice = OBJECTS.to_vec();
    twice.splice(NESTED_END..NESTED_END, OBJECTS[NESTED_AT..NESTED_END].iter().copied());
    assert!(recognise(&twice).is_none());
    // One byte between the nested STOP and the outer one.
    let mut spaced = OBJECTS.to_vec();
    spaced.insert(NESTED_END, b'N');
    assert!(recognise(&spaced).is_none());
}

/// The pickle after the wrapper holds the array and nothing else. One holding
/// a value of any other kind is a file nothing wrote.
#[test]
fn the_nested_pickle_holds_an_array_and_not_some_other_value() {
    // A whole protocol 5 pickle of the string "a" in place of the one the
    // file holds, and then the outer STOP. The outer frame ends at the
    // nested pickle, so nothing in front of this moves.
    let mut text = OBJECTS[..NESTED_AT].to_vec();
    text.extend_from_slice(&[0x80, 5]);
    text.extend_from_slice(&word("a"));
    text.extend_from_slice(b"..");
    assert!(recognise(&text).is_none());
}

/// The wrapper describes the array and the pickle after it holds one, so the
/// two have to agree. A shape or an order that does not is a non-match.
#[test]
fn the_nested_array_is_the_one_the_wrapper_described() {
    // The wrapper says three values and the array says three; saying four in
    // either place is two readings of one array.
    assert!(recognise(&edited_objects(b"\x8c\x05shape\x94K\x03", b"\x8c\x05shape\x94K\x04")).is_none());
    // The wrapper's own order letter, which the array inside states again.
    assert!(recognise(&edited_objects(b"\x8c\x05order\x94\x8c\x01C", b"\x8c\x05order\x94\x8c\x01F")).is_none());
}

/// A nested pickle only ever follows a wrapper whose dtype is `O8`. After any
/// other the run is the numbers, and a pickle where the numbers belong is
/// read as the numbers it is not.
#[test]
fn a_nested_pickle_after_a_plain_wrapper_is_a_non_match() {
    // The wrapper's own dtype, which the array inside the nested pickle
    // spells again further down. Only the first of the two is edited: an
    // `i8` wrapper measures three bytes of numbers where the pickle begins.
    const WRAPPER_DTYPE: usize = 138;
    assert_eq!(&OBJECTS[WRAPPER_DTYPE..WRAPPER_DTYPE + 2], b"O8");
    let mut plain = OBJECTS.to_vec();
    plain[WRAPPER_DTYPE..WRAPPER_DTYPE + 2].copy_from_slice(b"i8");
    assert!(recognise(&plain).is_none());
}

/// Nothing new may be named inside the nested pickle: it is read by the same
/// grammar and the same enumerated globals as the stream around it.
#[test]
fn the_nested_pickle_names_only_what_the_form_names() {
    assert!(recognise(&edited_objects(b"\x8c\x16numpy._core.multiarray\x94\x8c\x0c_reconstruct", b"\x8c\x16numpy._core.multiarrax\x94\x8c\x0c_reconstruct")).is_none());
    // And no wrapper of joblib's own: `pickle.dump` writes none.
    assert!(recognise(&edited_objects(b"\x8c\x0c_reconstruct", b"\x8c\x0c_reconstrucT")).is_none());
}

/// A frame with named columns and a text column, dumped by joblib: two arrays
/// of numbers written as runs of bytes and two object arrays written as
/// pickles of their own, in the one stream.
const FRAME: &[u8] = include_bytes!("../../../../../tests/fixtures/pickle/joblib-frame-named-columns.joblib");

/// A file holding both kinds of run, one after the other. The opcode walk
/// stops and starts again at each of them, and the two kinds interleave, so
/// this is where the segments could be got wrong and nowhere else.
#[test]
fn a_file_holding_both_kinds_of_run_still_names_every_byte() {
    let found = recognise(FRAME).unwrap();
    assert_eq!(found.form, "mixed-values-p4-p5-v1");
    let seen = dump(FRAME);
    // pandas gathers the columns of one dtype into one block, so the two
    // number columns are two raw runs, and the column names and the text
    // column are a nested pickle each.
    assert_eq!(seen.iter().filter(|r| r.name == "padding").count(), 2);
    assert_eq!(seen.iter().filter(|r| r.name == "nested pickle").count(), 2);
    // The file's own protocol row and one for each nested pickle.
    assert_eq!(seen.iter().filter(|r| r.name == "protocol").count(), 3);
    tiles(&seen);
}
