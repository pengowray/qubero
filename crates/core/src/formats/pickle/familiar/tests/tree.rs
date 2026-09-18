//! The familiar-form template over a match: the rows a reader sees, the type
//! and the value each one reads as, where it sits, and that the fields of a
//! node tile it with no bytes left over.

use super::*;

#[test]
fn template_publishes_the_match_message_at_stop() {
    use crate::{document::Document, eval::Evaluator, source::MemSource};
    let doc = Document::new(MemSource(vec![0x80, 4, b'N', b'.']));
    let mut ev = Evaluator::new(crate::formats::pickle::pickle());
    let node = ev.node(&doc, &[2, 1]).unwrap();
    // The contract's sentence, the form that matched, and where the data
    // is. The first sentence is the one the design document fixes.
    let said = stop_message("basic-p4-p5-v5");
    assert_eq!(node.value, crate::eval::Value::Str(said.clone()));
    assert!(said.starts_with("Matched a Familiar Pickle Form ("));
    assert!(said.contains(": bypassed Pickle stack machine decoding."));
    assert!(said.ends_with(&format!("Switch to the \"{FAMILIAR_LABEL}\" template to see the data.")));
}

/// Every row the familiar-form template shows for a small dictionary, in
/// file order: the header, the instructions the form fixed, the names and
/// the values.
#[test]
fn the_familiar_template_places_the_decoded_values() {
    let bytes = framed(b"}\x94\x8c\x01a\x94]\x94(K\x01K\x02es.");
    let seen = dump(&bytes);
    let said: Vec<(usize, &str, &str, u64, u64)> =
        seen.iter().map(|r| (r.depth, r.name.as_str(), r.ty.as_str(), r.at, r.len)).collect();
    assert_eq!(
        said,
        vec![
            (0, "file", "pickle", 0, 27),
            (1, "header", "header", 0, 11),
            (2, "message", "computed text", 0, 0),
            (2, "form", "computed text", 0, 0),
            (2, "pickler", "computed text", 0, 0),
            (2, "proto", "bytes[]", 0, 1),
            (2, "protocol", "u8", 1, 1),
            (2, "frame", "bytes[]", 2, 9),
            // The dictionary, from its EMPTY_DICT to the SETITEM that
            // filled it, with both of those inside it.
            (1, "data", "dict", 11, 15),
            (2, "empty_dict", "bytes[]", 11, 1),
            (2, "memoize", "bytes[]", 12, 1),
            (2, "a", "entry", 13, 12),
            (3, "short_binunicode", "bytes[]", 13, 2),
            (3, "key", "utf8[]", 15, 1),
            (3, "memoize", "bytes[]", 16, 1),
            (3, "value", "list", 17, 8),
            (4, "empty_list", "bytes[]", 17, 1),
            (4, "memoize", "bytes[]", 18, 1),
            (4, "mark", "bytes[]", 19, 1),
            (4, "binint1", "bytes[]", 20, 1),
            (4, "[0]", "u8", 21, 1),
            (4, "binint1", "bytes[]", 22, 1),
            (4, "[1]", "u8", 23, 1),
            (4, "appends", "bytes[]", 24, 1),
            (2, "setitem", "bytes[]", 25, 1),
            (1, "stop", "bytes[]", 26, 1),
        ]
    );
    assert_eq!(named_row(&seen, "message").value, V::Str(MESSAGE.into()));
    assert_eq!(named_row(&seen, "form").value, V::Str("basic-p4-p5-v5".into()));
    // Nothing in this file tells the two picklers apart, and the row says so
    // rather than being left out.
    assert_eq!(named_row(&seen, "pickler").value, V::Str(Pickler::Undetermined.name().into()));
    assert_eq!(named_row(&seen, "protocol").value, V::UInt(4));
    assert_eq!(named_row(&seen, "key").value, V::Str("a".into()));
    assert_eq!(named_row(&seen, "[0]").value, V::UInt(1));
    assert_eq!(named_row(&seen, "[1]").value, V::UInt(2));
    // The instructions fold away; the values do not.
    assert!(named_row(&seen, "setitem").machinery, "an instruction is the value's machinery");
    assert!(!named_row(&seen, "key").machinery);
    assert!(!named_row(&seen, "data").machinery);
}

/// Nothing a matched form consumed is left over. A form fixes its
/// instructions, so a byte no field covers would be a byte the form
/// matched and the template could not name.

#[test]
fn a_matched_file_has_no_unmapped_bytes() {
    for bytes in [
        framed(b"}\x94\x8c\x01a\x94]\x94(K\x01K\x02es."),
        framed(b"]\x94(N\x88\x89M\x39\x30J\xff\xff\xff\xffG\x3f\xf0\x00\x00\x00\x00\x00\x00C\x02\xde\xad\x94e."),
        framed(b"}\x94."),
        // The productions added since: tuples of each arity, a set, a
        // frozenset, a long integer, and a key the file named rather
        // than spelled a second time.
        framed(b"]\x94(K\x01\x85\x94K\x01K\x02\x86\x94K\x01K\x02K\x03\x87\x94(K\x01K\x02K\x03K\x04t\x94)e."),
        framed(b"]\x94(\x8f\x94(K\x01K\x02\x90(K\x03\x91\x94\x8a\x05\0\0\0\0\x01e."),
        framed(b"}\x94(\x8c\x03key\x94K\x01h\x01K\x02u."),
        proto5(b"}\x94(\x8c\x01a\x94\x96\x03\0\0\0\0\0\0\0abc\x94\x8c\x01b\x94h\x01u."),
        MATRIX.to_vec(),
        // An array of shape 0: its numbers are a field the file wrote and
        // a run of no bytes at once.
        framed(&cat(&[b"}\x94", &word("a"), &one_array(2, "f8", b'<', b"K\0\x85\x94", &[]), b"s."])),
    ] {
        let seen = dump(&bytes);
        assert_eq!(seen[0].len, bytes.len() as u64, "the root is the file");
        tiles(&seen);
    }
}

/// An array whose shape is 0 holds no numbers, and the row that says so is
/// still a field of the file: it sits at the byte the payload would have
/// started at, right after the SHORT_BINBYTES that wrote a length of zero,
/// rather than being pushed back to the start of the array.
#[test]
fn an_empty_array_reads_its_numbers_where_they_would_have_been() {
    let bytes = framed(&cat(&[b"}\x94", &word("a"), &one_array(2, "f8", b'<', b"K\0\x85\x94", &[]), b"s."]));
    let seen = dump(&bytes);
    assert_eq!(named_row(&seen, "form").value, V::Str("numpy-numeric-array-p4-p5-v5".into()));
    assert_eq!(named_row(&seen, "shape").value, V::Str("0".into()));
    let numbers = named_row(&seen, "numbers");
    assert_eq!((numbers.ty.as_str(), numbers.len, &numbers.value), ("f64 le[]", 0, &V::Composite { count: 0 }));
    // The SHORT_BINBYTES before it is the opcode and the length byte, so
    // the numbers begin two bytes later and end where they begin.
    let header = seen.iter().rev().find(|r| r.name == "short_binbytes" && r.at < numbers.at).unwrap();
    assert_eq!((header.len, numbers.at), (2, header.at + 2));
    let array = named_row(&seen, "value");
    assert!(numbers.at > array.at, "the numbers are placed, not pushed back to {:#x}", array.at);
    tiles(&seen);
}

/// The other leaf types, each at the bytes it was written in.
#[test]
fn every_leaf_reads_as_the_type_its_bytes_are() {
    let bytes = framed(b"]\x94(N\x88\x89M\x39\x30J\xff\xff\xff\xffG\x3f\xf0\x00\x00\x00\x00\x00\x00C\x02\xde\xad\x94e.");
    // The values of the list, which is what the file holds: the header
    // says what matched and is not part of it.
    let seen: Vec<(String, u64, V)> = dump(&bytes)
        .into_iter()
        .skip_while(|r| r.name != "data")
        .filter(|r| !r.machinery && r.depth == 2)
        .map(|r| (r.ty, r.len, r.value))
        .collect();
    assert_eq!(
        seen,
        vec![
            ("null".into(), 1, named(0x4e, "None")),
            ("bool".into(), 1, named(0x88, "True")),
            ("bool".into(), 1, named(0x89, "False")),
            ("u16 le".into(), 2, V::UInt(12345)),
            ("i32 le".into(), 4, V::Int(-1)),
            ("f64 be".into(), 8, V::Float(1.0)),
            ("bytes[]".into(), 2, V::Bytes { len: 2, preview: vec![0xde, 0xad] }),
        ]
    );
}

/// A value the file named rather than wrote again: the BINGET is the
/// whole of it, so the row that says what it names is worked out from the
/// match and sits where the reference does. An entry a named string is
/// the key of is still called by that string.
#[test]
fn a_named_value_says_what_it_names() {
    let seen = dump(&framed(b"}\x94(\x8c\x03key\x94K\x01h\x01K\x02u."));
    let said: Vec<(usize, &str, &str, u64, u64, &V)> = seen
        .iter()
        .skip_while(|r| r.name != "data")
        .map(|r| (r.depth, r.name.as_str(), r.ty.as_str(), r.at, r.len, &r.value))
        .collect();
    // The first entry spelled its key out; the second named it instead.
    assert_eq!(
        said[10..17],
        [
            (2, "key", "entry", 22, 4, &V::Composite { count: 3 }),
            (3, "key", "reference", 22, 2, &V::Composite { count: 2 }),
            (4, "refers to", "computed text", 22, 0, &V::Str("key".into())),
            (4, "binget", "bytes[]", 22, 2, &V::Bytes { len: 2, preview: vec![0x68, 1] }),
            (3, "binint1", "bytes[]", 24, 1, &V::Bytes { len: 1, preview: vec![b'K'] }),
            (3, "value", "u8", 25, 1, &V::UInt(2)),
            (2, "setitems", "bytes[]", 26, 1, &V::Bytes { len: 1, preview: vec![b'u'] }),
        ]
    );
    tiles(&seen);
}

/// The values the widened grammar added, each read as what it is: a set
/// and a frozenset hold their members, a long integer is a signed run of
/// bytes as wide as it needs, and a protocol 5 bytearray holds the bytes
/// it was made from the way the protocol 4 call to the class does.
#[test]
fn the_widened_values_read_as_the_types_their_bytes_are() {
    let seen = dump(&framed(b"]\x94(\x8f\x94(K\x01K\x02\x90(K\x03\x91\x94\x8a\x05\0\0\0\0\x01e."));
    let kinds: Vec<(&str, &str)> = seen.iter().map(|r| (r.name.as_str(), r.ty.as_str())).collect();
    assert!(kinds.contains(&("[0]", "set")), "{kinds:?}");
    assert!(kinds.contains(&("[1]", "frozenset")), "{kinds:?}");
    let long = seen.iter().rev().find(|r| r.name == "[2]").unwrap();
    assert_eq!((long.ty.as_str(), long.len, &long.value), ("i40 le", 5, &V::Int(4_294_967_296)));

    let seen = dump(&proto5(b"}\x94(\x8c\x01a\x94\x96\x03\0\0\0\0\0\0\0abc\x94\x8c\x01b\x94h\x01u."));
    let held = named_row(&seen, "bytes");
    assert_eq!((held.ty.as_str(), held.at, held.len), ("bytes[]", 27, 3));
    assert_eq!(named_row(&seen, "value").ty, "bytearray");
}

/// A matched array says what it is before it says what it holds, and the
/// call that rebuilt it holds the names and letters the form matched.
#[test]
fn a_matched_array_carries_its_dtype_shape_and_order() {
    let seen = dump(MATRIX);
    assert_eq!(named_row(&seen, "form").value, V::Str("numpy-numeric-array-p4-p5-v5".into()));
    assert_eq!(named_row(&seen, "value").ty, "array");
    assert_eq!(named_row(&seen, "dtype").value, V::Str("<f4".into()));
    assert_eq!(named_row(&seen, "shape").value, V::Str("4 x 6".into()));
    assert_eq!(named_row(&seen, "order").value, V::Str("C".into()));
    // The call, and the names it was made with.
    let call = named_row(&seen, "ndarray reconstruct call");
    assert_eq!(call.ty, "call");
    assert_eq!(named_row(&seen, "module").value, V::Str("numpy._core.multiarray".into()));
    assert_eq!(named_row(&seen, "callable").value, V::Str("_reconstruct".into()));
    assert_eq!(named_row(&seen, "class module").value, V::Str("numpy".into()));
    assert_eq!(named_row(&seen, "class").value, V::Str("ndarray".into()));
    assert_eq!(named_row(&seen, "dtype class").value, V::Str("dtype".into()));
    assert_eq!(named_row(&seen, "byte order").value, V::Str("<".into()));
    let numbers = named_row(&seen, "numbers");
    assert_eq!((numbers.ty.as_str(), numbers.len, &numbers.value), ("f32 le[]", 96, &V::Composite { count: 24 }));
}

/// The numbers of a matched array are a table of its rows, six to a row
/// for a 4 x 6 array, and nothing else in the file is.
#[test]
fn a_matched_array_is_a_table_of_its_rows() {
    // The protocol 5 array's numbers sit one level deeper, inside the
    // call that was handed them, so the table has to be found there too.
    let numbers: Vec<u8> = (0u8..12).flat_map(|n| [n, 0]).collect();
    let buffered = proto5(&cat(&[&frombuffer(&mutable(&numbers), "i2", b'<', b"K\x03K\x04\x86\x94", "C"), b"."]));
    for (bytes, columns) in [(MATRIX.to_vec(), 6), (buffered, 4)] {
        let (doc, mut ev) = read(&bytes);
        let mut tables = Vec::new();
        let mut stack = vec![Vec::new()];
        while let Some(path) = stack.pop() {
            let info = ev.node(&doc, &path).unwrap();
            if info.table {
                tables.push((info.name.clone(), ev.table_shape(&doc, &path).unwrap().expect("a shape").columns));
            }
            if path.len() < 6 {
                stack.extend((0..info.child_count.min(80) as usize).map(|i| [path.as_slice(), &[i]].concat()));
            }
        }
        assert_eq!(tables, vec![("numbers".to_string(), Some(columns))]);
    }
}

/// A pickle no form matches has nothing for this template to show, and
/// says so rather than showing part of a reading.
#[test]
fn an_unfamiliar_program_has_no_tree() {
    let (doc, mut ev) = read(b"\x80\x04\x8c\x02os\x94\x8c\x06system\x94\x93.");
    let root = ev.node(&doc, &[]);
    assert!(root.is_err(), "an unmatched file resolved to {root:?}");
    assert!(ev.node(&doc, &[0]).is_err());
    assert!(ev.node(&doc, &[1]).is_err());
}
