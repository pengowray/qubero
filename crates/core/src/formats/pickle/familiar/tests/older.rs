//! Protocols 2 and 3: the spellings the older grammar has and the newer one
//! does not, and the ones it must turn away.
//!
//! Every body here is written out rather than edited from a fixture, so that a
//! test says which alternative it is exercising. A memo slot below protocol 4
//! carries its number in the file, so the comments count the slots out.

use super::*;

/// The opener of a file at a protocol below 4, which has no frame. Protocol 1
/// has no opener at all: the file starts at its first value.
fn older(proto: u8, body: &[u8]) -> Vec<u8> {
    let mut bytes = match proto >= 2 {
        true => vec![0x80, proto],
        false => Vec::new(),
    };
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
    assert_eq!(found.pickler, Pickler::CPickle);
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
    assert_eq!(found.pickler, Pickler::CPickle);
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
    let Kind::Made { what, items, .. } = &found.value.kind else { panic!("a call expected") };
    assert_eq!(*what, Shape::Bytes);
    assert_eq!(&items[0].kind, &Kind::Text { at: 25, len: 2 });
    // A character latin-1 never spelled is a text no pickler wrote there: the
    // original bytes each became the character of the same number, so every
    // one of them is under 0x100.
    let wide_char = older(
        2,
        &cat(&[b"c_codecs\nencode\n", &at_slot(0), &wide("\u{4e2d}"), &at_slot(1), &wide("latin1"), &at_slot(2), b"\x86", &at_slot(3), b"R", &at_slot(4), b"."]),
    );
    assert!(recognise(&wide_char).is_none());
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
        let Kind::Made { what: made, .. } = &found.value.kind else { panic!("a call expected") };
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

/// An array Python 2 wrote, whose numbers are a `str` and so are the bytes
/// they are rather than the latin-1 text protocol 2 makes Python 3 write.
///
/// No file in the corpus is one: none of the Python 2 environments has NumPy.
/// So this is a branch of the grammar with a body written to it and no sample
/// behind it, the way the protocol 5 `_frombuffer` branch was before a file
/// turned up: it says what the reader does with those bytes, not that a
/// particular NumPy release writes them.
#[test]
fn an_array_written_by_python_2_holds_its_numbers_as_they_are() {
    let numbers: &[u8] = b"\x01\0\0\0\0\0\0\0\x02\0\0\0\0\0\0\0";
    let body = cat(&[
        b"cnumpy.core.multiarray\n_reconstruct\n",
        &at_slot(0),
        b"cnumpy\nndarray\n",
        &at_slot(1),
        b"K\0\x85",
        &at_slot(2),
        b"U\x01b",
        &at_slot(3),
        b"\x87",
        &at_slot(4),
        b"R",
        &at_slot(5),
        b"(K\x01K\x02\x85",
        &at_slot(6),
        b"cnumpy\ndtype\n",
        &at_slot(7),
        b"U\x02i8",
        &at_slot(8),
        b"\x89\x88\x87",
        &at_slot(9),
        b"R",
        &at_slot(10),
        b"(K\x03U\x01<",
        &at_slot(11),
        b"NNNJ\xff\xff\xff\xffJ\xff\xff\xff\xffK\0t",
        &at_slot(12),
        b"b\x89U\x10",
        numbers,
        &at_slot(13),
        b"t",
        &at_slot(14),
        b"b.",
    ]);
    let bytes = older(2, &body);
    let found = recognise(&bytes).unwrap_or_else(|| panic!("read as far as {:#x}", furthest(&bytes)));
    assert_eq!(found.form, "numpy-array-p2-p3-v1");
    let Kind::Array { at, len, storage, dimensions, .. } = &found.value.kind else { panic!("array expected") };
    // The run in the file is the numbers, so nothing was decoded beside the
    // match and the opcode listing may read the run as the values it holds.
    assert_eq!(*storage, Storage::Raw);
    assert_eq!(*len, numbers.len());
    assert_eq!(dimensions.as_slice(), &[2]);
    assert!(found.decoded(*at).is_none());
    assert_eq!(&bytes[*at..*at + *len], numbers);
}

/// Protocol 1 is protocol 2 with four things missing: the opener, the two
/// singleton opcodes, the counted tuples and the binary `long`.
#[test]
fn a_file_at_protocol_1_reads_without_an_opener() {
    // `{"name": [1, True]}`, which at protocol 2 would open with PROTO, write
    // NEWTRUE and close the list with APPENDS the same way.
    let bytes = older(1, &cat(&[b"}", &at_slot(0), &wide("name"), &at_slot(1), b"]", &at_slot(2), b"(K\x01I01\nes."]));
    let found = recognise(&bytes).unwrap_or_else(|| panic!("read as far as {:#x}", furthest(&bytes)));
    assert_eq!(found.form, "basic-p1-v1");
    assert_eq!(found.proto, 1);
    let Kind::Dict(entries) = &found.value.kind else { panic!("dict expected") };
    let Kind::List(items) = &entries[0].1.kind else { panic!("list expected") };
    assert_eq!(items[1].kind, Kind::Bool(true));
    // `I1` and `I0` are the integers, not the singletons, and neither is a
    // number BININT1 could have held.
    assert!(recognise(&older(1, &cat(&[b"]", &at_slot(0), b"I1\na."]))).is_none());
    // The opcodes protocol 2 brought are not protocol 1's.
    assert!(recognise(&older(1, &cat(&[b"]", &at_slot(0), b"\x88a."]))).is_none());
    assert!(recognise(&older(1, &cat(&[b"K\x01K\x02\x86", &at_slot(0), b"."]))).is_none());
    assert!(recognise(&older(1, &cat(&[b"\x8a\x05\0\0\0\0\x01."]))).is_none());
    // And a PROTO opener is not: protocol 1 has no such opcode.
    assert!(recognise(&cat(&[b"\x80\x01}", &at_slot(0), b"."])).is_none());
}

/// A `long` at protocol 1 is a line of digits with the letter Python 2 spelled
/// one with, and anything a four-byte BININT holds was written as one.
#[test]
fn a_long_at_protocol_1_is_a_line_of_digits() {
    let line = |digits: &str| older(1, &cat(&[b"L", digits.as_bytes(), b"L\n."]));
    let bytes = line("1208925819614629174706176");
    let found = recognise(&bytes).unwrap();
    assert_eq!(found.value.kind, Kind::Int { value: 1_208_925_819_614_629_174_706_176, at: 1, len: 25 });
    // A number a four-byte BININT holds was written as one at protocol 1. It
    // is still a protocol 0 file, where every integer is a line, so what is
    // claimed is that none of these is read as protocol 1.
    for wrong in ["5", "-1", "0000000000005", "+2147483648", "1e30"] {
        assert_ne!(recognise(&line(wrong)).map(|m| m.proto), Some(1), "{wrong} was read as a protocol 1 long");
    }
    // The trailing `L` is part of the spelling.
    assert!(recognise(&older(1, b"L2147483648\n.")).is_none());
}

/// Ordinary text is not a familiar form, whatever its bytes walk as.
///
/// Protocol 0 and 1 files have no opener, so a form reading them cannot lean
/// on one. What it leans on instead is that a match covers the whole file and
/// ends at a STOP with one value on the stack. `Nadal.` walks as four opcodes
/// and a full stop and is a word; it is a non-match because `N` `a` `d` `a` is
/// not a value a form built.
#[test]
fn ordinary_text_is_not_a_familiar_form() {
    for text in [
        &b"hello, world\n"[..],
        b"# a magic file\n0\tstring\tGIF\tGIF image\n",
        b"{\n  \"name\": \"qubero\"\n}\n",
        b"data.",
        b"steal.",
        b"Nadal.",
        b"",
        // A line of digits that is not a number Python wrote, and a full stop
        // after nothing at all.
        b"I0001\n.",
        b".",
    ] {
        assert!(recognise(text).is_none(), "{:?} read as a familiar form", &text[..text.len().min(20)]);
    }
}

/// Protocol 0, where every value is an opcode and a line.
#[test]
fn a_file_at_protocol_0_reads_its_values_out_of_lines() {
    // `{"name": [1, True]}`, written the way protocol 0 writes one: a MARK
    // and a DICT for the dictionary, a SETITEM an entry, and no batching.
    let bytes = b"(dp0\nVname\np1\n(lp2\nL1L\naI01\nas.";
    let found = recognise(bytes).unwrap_or_else(|| panic!("read as far as {:#x}", furthest(bytes)));
    assert_eq!(found.form, "basic-p0-v1");
    assert_eq!(found.proto, 0);
    let Kind::Dict(entries) = &found.value.kind else { panic!("dict expected") };
    assert_eq!(entries[0].0.kind, Kind::Text { at: 6, len: 4 });
    let Kind::List(items) = &entries[0].1.kind else { panic!("list expected") };
    assert_eq!(items[0].kind, Kind::Int { value: 1, at: 20, len: 1 });
    assert_eq!(items[1].kind, Kind::Bool(true));
    // Protocol 0 fills a container one entry at a time and has no batch
    // opcode at all.
    assert!(recognise(b"(lp0\n(L1L\nL2L\nes.").is_none());
}

/// A line that spells its value rather than being it is a value of its own,
/// with the run the file holds under it.
#[test]
fn a_line_with_an_escape_in_it_is_the_thing_it_spells() {
    // `{"caf\u00e9": "a\\b"}`: the first text holds a character above 0x7f,
    // which `raw-unicode-escape` writes as the byte 0xe9, and the second
    // holds a backslash, which `save_str` writes as `\u005c`.
    let bytes = b"(dp0\nVcaf\xe9\np1\nVa\\u005cb\np2\ns.";
    let found = recognise(bytes).unwrap_or_else(|| panic!("read as far as {:#x}", furthest(bytes)));
    let Kind::Dict(entries) = &found.value.kind else { panic!("dict expected") };
    let (key, value) = &entries[0];
    assert!(matches!(key.kind, Kind::Spelled { bytes: false, .. }));
    let Kind::Spelled { at: key_at, .. } = key.kind else { panic!() };
    assert_eq!(found.decoded(key_at).map(|h| h.to_vec()), Some("caf\u{e9}".as_bytes().to_vec()));
    let Kind::Spelled { at: value_at, .. } = value.kind else { panic!("a spelled value") };
    assert_eq!(found.decoded(value_at).map(|h| h.to_vec()), Some(b"a\\b".to_vec()));
}

/// The lines a pickler never wrote are not read.
#[test]
fn a_line_no_pickler_wrote_is_a_non_match() {
    for body in [
        // An escape `raw-unicode-escape` does not write, and a bare
        // backslash, which `save_str` replaces before it writes the line.
        &b"Va\\nb\np0\n."[..],
        &b"Va\\b\np0\n."[..],
        &b"Va\\u00E9b\np0\n."[..],
        // A `STRING` line with no closing quote, with the wrong one, and with
        // an escape Python 2's `repr` does not write.
        &b"S'abc\np0\n."[..],
        &b"S'abc\"\np0\n."[..],
        &b"S'a\\qb'\np0\n."[..],
        // Numbers spelled a way `repr` never spells one.
        &b"F0005.0\n."[..],
        &b"F+5.0\n."[..],
        &b"F1e5\n."[..],
        &b"L007L\n."[..],
        &b"L5\n."[..],
        // A memo mark to a slot that is not the next one.
        &b"(lp3\n."[..],
        &b"Vone\np0\nVtwo\np2\n."[..],
        // The opcodes CPython reads and never writes.
        &b"(i__builtin__\nset\np0\n."[..],
        &b"(c__builtin__\nset\no."[..],
        &b"Pabc\n."[..],
    ] {
        assert!(recognise(body).is_none(), "{body:?} was read");
    }
    // A float spelled either way a pickler spells one, which is the same
    // number twice: `repr` from the pure picklers, and `%.17g` from Python 2's
    // cPickle and Python 3.4's C one, which drops the point from a whole
    // number.
    for right in [&b"F3.0\n."[..], b"F3\n.", b"F-0.0\n.", b"Finf\n.", b"F-inf\n.", b"Fnan\n.", b"F1e+308\n.", b"F1e-06\n."] {
        assert!(recognise(right).is_some(), "{right:?} was not read");
    }
}
