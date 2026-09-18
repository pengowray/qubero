//! Protocols 2 and 3: the spellings the older grammar has and the newer one
//! does not, and the ones it must turn away.
//!
//! Every body here is written out rather than edited from a fixture, so that a
//! test says which alternative it is exercising. A memo slot below protocol 4
//! carries its number in the file, so the comments count the slots out.

use super::*;

/// The opener of a file at a protocol below 4, which has no frame.
fn older(proto: u8, body: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0x80, proto];
    bytes.extend_from_slice(body);
    bytes
}

/// BINUNICODE, which is the only spelling of text protocols 2 and 3 have: a
/// four-byte length however short the text is.
fn wide(text: &str) -> Vec<u8> {
    let mut bytes = vec![0x58];
    bytes.extend_from_slice(&(text.len() as u32).to_le_bytes());
    bytes.extend_from_slice(text.as_bytes());
    bytes
}

/// BINPUT of a slot, which is the memo mark below protocol 4.
fn at_slot(slot: u8) -> Vec<u8> {
    vec![0x71, slot]
}

fn cat(pieces: &[&[u8]]) -> Vec<u8> {
    pieces.concat()
}

/// `{"name": [1, 2]}` at protocol 3, which is protocol 4 with the memo marks
/// numbered and the text counted four bytes wide.
fn small(proto: u8) -> Vec<u8> {
    older(
        proto,
        &cat(&[
            b"}",
            &at_slot(0),
            &wide("name"),
            &at_slot(1),
            b"]",
            &at_slot(2),
            b"(K\x01K\x02es.",
        ]),
    )
}

#[test]
fn a_file_below_protocol_4_reads_as_the_object_it_holds() {
    let bytes = small(3);
    let found = recognise(&bytes).unwrap();
    assert_eq!(found.form, "basic-p2-p3-v1");
    let Kind::Dict(entries) = &found.value.kind else { panic!("dict expected") };
    let (key, value) = &entries[0];
    assert_eq!(&key.kind, &Kind::Text { at: 10, len: 4 });
    let Kind::List(items) = &value.kind else { panic!("list expected") };
    assert_eq!(items.len(), 2);
    // Nothing in it says which pickler wrote it, which is most files.
    assert_eq!(found.pickler, Pickler::Undetermined);
    // And the same object at protocol 2, where text is spelled the same way.
    assert_eq!(recognise(&small(2)).unwrap().form, "basic-p2-p3-v1");
}

/// A slot number is the count of marks before it, so the file cannot say
/// otherwise.
#[test]
fn a_memo_mark_to_the_wrong_slot_is_a_non_match() {
    let mut bytes = small(3);
    assert_eq!(&bytes[2..5], b"}\x71\x00");
    for wrong in [1u8, 2, 255] {
        bytes[4] = wrong;
        assert!(recognise(&bytes).is_none(), "a first mark to slot {wrong} was read");
    }
    // And a slot numbered from one throughout, which is what Python 2's
    // cPickle writes.
    let shifted = older(
        2,
        &cat(&[b"}", &at_slot(1), &wide("name"), &at_slot(2), b"]", &at_slot(3), b"(K\x01K\x02es."]),
    );
    let found = recognise(&shifted).unwrap();
    assert_eq!(found.pickler, Pickler::C);
}

/// Only `cPickle` leaves a memo mark out, and it numbers from one, so a file
/// numbering from nought has to have written every mark.
#[test]
fn a_memo_mark_left_out_is_only_cpickle_s() {
    let dropped = older(2, &cat(&[b"}", &at_slot(0), &wide("name"), b"]", &at_slot(2), b"(K\x01K\x02es."]));
    assert!(recognise(&dropped).is_none());
    // The same file numbered from one: the text has no mark, so the list's is
    // slot two rather than three, and that is `cPickle` to the letter.
    let cpickle = older(2, &cat(&[b"}", &at_slot(1), &wide("name"), b"]", &at_slot(2), b"(K\x01K\x02es."]));
    let found = recognise(&cpickle).unwrap();
    assert_eq!(found.pickler, Pickler::C);
}

/// A byte string at protocol 2 is a call to `_codecs.encode` over the text the
/// bytes spell in latin-1, and the encoding has to be that one.
#[test]
fn a_byte_string_below_protocol_3_is_a_call_to_codecs() {
    let made = |encoding: &str| {
        older(
            2,
            &cat(&[b"c_codecs\nencode\n", &at_slot(0), &wide("hi"), &at_slot(1), &wide(encoding), &at_slot(2), b"\x86", &at_slot(3), b"R", &at_slot(4), b"."]),
        )
    };
    let bytes = made("latin1");
    let found = recognise(&bytes).unwrap();
    assert_eq!(found.form, "basic-p2-p3-v1");
    let Kind::Object { what, items, .. } = &found.value.kind else { panic!("a call expected") };
    assert_eq!(*what, Shape::Bytes);
    assert_eq!(&items[0].kind, &Kind::Text { at: 25, len: 2 });
    // Any other encoding is a byte string this cannot read back.
    for other in ["latin2", "LATIN1", "utf-8", "ascii-"] {
        assert!(recognise(&made(other)).is_none(), "{other} was read as latin-1");
    }
    // An empty byte string is `bytes()` instead, which is the one spelling
    // protocol 2 has for it.
    let empty = older(2, &cat(&[b"c__builtin__\nbytes\n", &at_slot(0), b")R", &at_slot(1), b"."]));
    assert_eq!(recognise(&empty).unwrap().value.kind, Kind::Bytes { at: 25, len: 0 });
}

/// A set below protocol 4 is the class called with the members, and the class
/// is the one thing the file may name.
#[test]
fn a_set_below_protocol_4_is_a_call_over_its_members() {
    let set = older(2, b"c__builtin__\nset\nq\x00]q\x01(K\x01K\x02e\x85q\x02Rq\x03.");
    let found = recognise(&set).unwrap();
    assert_eq!(found.form, "basic-p2-p3-v1");
    let Kind::Set(members) = &found.value.kind else { panic!("set expected, not {:?}", found.value.kind) };
    assert_eq!(members.len(), 2);
    let frozen = older(2, b"c__builtin__\nfrozenset\nq\x00]q\x01(K\x01K\x02K\x03e\x85q\x02Rq\x03.");
    assert!(matches!(recognise(&frozen).unwrap().value.kind, Kind::FrozenSet(_)));
    // PyPy hands the class a tuple of the members rather than a list.
    let pypy = older(2, b"c__builtin__\nset\nq\x00(K\x01K\x02K\x03K\x04tq\x01\x85q\x02Rq\x03.");
    assert!(matches!(recognise(&pypy).unwrap().value.kind, Kind::Set(_)));
    // At protocol 3 the module is the name Python 3 knows it by, and each
    // spelling belongs to its own protocol.
    let three = older(3, b"cbuiltins\nset\nq\x00]q\x01(K\x01K\x02e\x85q\x02Rq\x03.");
    assert!(recognise(&three).is_some());
    assert!(recognise(&older(3, b"c__builtin__\nset\nq\x00]q\x01(K\x01K\x02e\x85q\x02Rq\x03.")).is_none());
    assert!(recognise(&older(2, b"cbuiltins\nset\nq\x00]q\x01(K\x01K\x02e\x85q\x02Rq\x03.")).is_none());
}

/// A class is named so that it can be called. One the file hands over as a
/// value is a class the reader would be shown as data, which no form below a
/// library one allows.
#[test]
fn a_class_that_is_not_called_is_a_non_match() {
    assert!(recognise(&older(2, b"c__builtin__\nset\nq\x00.")).is_none());
    assert!(recognise(&older(2, b"]q\x00c__builtin__\nset\nq\x01a.")).is_none());
    // And a global no form listed, called or not.
    assert!(recognise(&older(2, b"cos\nsystem\nq\x00.")).is_none());
    assert!(recognise(&older(2, b"cos\nsystem\nq\x00]q\x01\x85q\x02Rq\x03.")).is_none());
}

/// Python 2 wrote an `int` too wide for BININT as a line of digits, which is a
/// text opcode inside a binary protocol.
#[test]
fn an_int_too_wide_for_binint_is_a_line_of_digits() {
    let line = |digits: &str| older(2, &cat(&[b"I", digits.as_bytes(), b"\n."]));
    let bytes = line("-1099511627776");
    let found = recognise(&bytes).unwrap();
    assert_eq!(found.value.kind, Kind::Int { value: -1_099_511_627_776, at: 3, len: 14 });
    // Anything a four-byte integer holds was written as BININT, and CPython
    // writes no leading zero, no sign it does not need and no spaces.
    for wrong in ["5", "-1", "0000000000000005", "+1099511627776", " 1099511627776", "1e30", "99999999999999999999"] {
        assert!(recognise(&line(wrong)).is_none(), "{wrong} was read as an integer");
    }
    // Protocol 3 is Python 3's, whose integers are all one type.
    assert!(recognise(&older(3, b"I-1099511627776\n.")).is_none());
}

/// Python 2's `str` is a run of bytes that was usually text, and reads as the
/// listing reads it: text when the bytes are UTF-8 and bytes when they are not.
#[test]
fn a_python_2_string_reads_as_text_when_it_is_text() {
    let short = older(2, b"U\x04nameq\x00.");
    assert_eq!(recognise(&short).unwrap().value.kind, Kind::Text { at: 4, len: 4 });
    let raw = older(2, b"U\x02\xff\xfeq\x00.");
    assert_eq!(recognise(&raw).unwrap().value.kind, Kind::Bytes { at: 4, len: 2 });
    // BINSTRING, the four-byte-length spelling of the same thing.
    let long = older(2, b"T\x04\x00\x00\x00nameq\x00.");
    assert_eq!(recognise(&long).unwrap().value.kind, Kind::Text { at: 7, len: 4 });
    // Python 3 never wrote one, and protocol 3 is Python 3's.
    assert!(recognise(&older(3, b"U\x04nameq\x00.")).is_none());
}

/// The builtins a pickle writes as a call, under the names Python 2 knew them
/// by. Each of these was written by CPython and copied here byte for byte.
#[test]
fn the_builtins_form_reads_the_older_names() {
    let cases: &[(&[u8], Shape)] = &[
        (b"\x80\x02c__builtin__\ncomplex\nq\x00G?\xf8\x00\x00\x00\x00\x00\x00G@\x04\x00\x00\x00\x00\x00\x00\x86q\x01Rq\x02.", Shape::Complex),
        (b"\x80\x02c__builtin__\nslice\nq\x00K\x01NK\x02\x87q\x01Rq\x02.", Shape::Slice),
        (b"\x80\x02c__builtin__\nxrange\nq\x00K\x00K\x05K\x02\x87q\x01Rq\x02.", Shape::Range),
        (b"\x80\x02c__builtin__\nbytearray\nq\x00c_codecs\nencode\nq\x01X\x02\x00\x00\x00hiq\x02X\x06\x00\x00\x00latin1q\x03\x86q\x04Rq\x05\x85q\x06Rq\x07.", Shape::ByteArray),
        (b"\x80\x03cbuiltins\nbytearray\nq\x00C\x02hiq\x01\x85q\x02Rq\x03.", Shape::ByteArray),
        (b"\x80\x03cbuiltins\ncomplex\nq\x00G?\xf8\x00\x00\x00\x00\x00\x00G@\x04\x00\x00\x00\x00\x00\x00\x86q\x01Rq\x02.", Shape::Complex),
    ];
    for (bytes, what) in cases {
        let found = recognise(bytes).unwrap_or_else(|| panic!("{bytes:?} matched no form"));
        assert_eq!(found.form, "builtins-values-p2-p3-v1");
        let Kind::Object { what: made, .. } = &found.value.kind else { panic!("a call expected") };
        assert_eq!(made, what);
    }
    // `range` kept the name Python 2 knew it by below protocol 3, so the two
    // names do not cross.
    assert!(recognise(b"\x80\x02c__builtin__\nrange\nq\x00K\x00K\x05K\x02\x87q\x01Rq\x02.").is_none());
}

/// Neither protocol borrows the other's spellings.
#[test]
fn the_two_grammars_do_not_share_opcodes() {
    // MEMOIZE, SHORT_BINUNICODE, EMPTY_SET, FROZENSET and STACK_GLOBAL all
    // arrived with protocol 4.
    assert!(recognise(&older(3, b"}\x94\x8c\x01a\x94K\x01s.")).is_none());
    assert!(recognise(&older(3, b"\x8c\x04nameq\x00.")).is_none());
    assert!(recognise(&older(3, b"\x8f\x71\x00(K\x01\x90.")).is_none());
    assert!(recognise(&older(3, b"(K\x01\x91q\x00.")).is_none());
    // And BINPUT, GLOBAL, SHORT_BINSTRING and the INT line are not written at
    // protocol 4.
    assert!(recognise(&framed(b"}q\x00\x8c\x01a\x94K\x01s.")).is_none());
    assert!(recognise(&framed(b"U\x04name\x94.")).is_none());
    assert!(recognise(&framed(b"I-1099511627776\n.")).is_none());
    assert!(recognise(&framed(b"c__builtin__\nset\n\x94]\x94(K\x01e\x85\x94R\x94.")).is_none());
    // A frame header is protocol 4's too, and below it the byte is not one.
    assert!(recognise(&{
        let mut bytes = small(3);
        bytes.splice(2..2, [0x95, 0, 0, 0, 0, 0, 0, 0, 0]);
        bytes
    })
    .is_none());
}

/// The opcodes CPython 3 reads and never writes stay unread, whichever
/// protocol they turn up at.
#[test]
fn the_opcodes_no_pickler_writes_are_still_refused() {
    // INST, OBJ, EXT1, EXT2, EXT4, PERSID, BINPERSID and the text protocols'
    // PUT and GET.
    for body in [
        &b"(i__builtin__\nset\np0\n."[..],
        &b"(c__builtin__\nset\no."[..],
        &b"\x82\x01."[..],
        &b"\x83\x01\x00."[..],
        &b"\x84\x01\x00\x00\x00."[..],
        &b"Pabc\n."[..],
        &b"]q\x00Q."[..],
        &b"]p0\n."[..],
        &b"]q\x00g0\n."[..],
    ] {
        assert!(recognise(&older(2, body)).is_none(), "{body:?} was read");
    }
}
