//! The grammar itself: what a form takes and what it turns away, the bounds
//! a file made to cost something cannot talk its way past, the calls the
//! builtins form allows, and which form a file is read under.

use super::*;

#[test]
fn captures_basic_values_without_a_machine() {
    let bytes = framed(b"}\x94\x8c\x01a\x94]\x94(K\x01K\x02es.");
    let found = recognise(&bytes).unwrap();
    assert_eq!(found.form, "basic-p4-p5-v5");
    // Every node spans the bytes its production consumed, and a leaf says
    // where inside that its value proper sits.
    assert_eq!((found.value.at, found.value.len), (11, 15));
    let Kind::Dict(entries) = &found.value.kind else {
        panic!("dict expected")
    };
    let (key, value) = &entries[0];
    assert_eq!(entries.len(), 1);
    assert_eq!((key.at, key.len, &key.kind), (13, 4, &Kind::Text { at: 15, len: 1 }));
    assert_eq!((value.at, value.len), (17, 8));
    let Kind::List(items) = &value.kind else {
        panic!("list expected")
    };
    let seen: Vec<_> = items.iter().map(|v| (v.at, v.len, &v.kind)).collect();
    assert_eq!(
        seen,
        vec![
            (20, 2, &Kind::Int { value: 1, at: 21, len: 1, spelled: false }),
            (22, 2, &Kind::Int { value: 2, at: 23, len: 1, spelled: false }),
        ]
    );
    assert_eq!(
        found.text(Deduce::Builds, bytes.len() as u64).as_deref(),
        Some(stop_message("basic-p4-p5-v5").as_str())
    );
    assert!(recognise(b"\x80\x05N.").is_some());
}

#[test]
fn rejects_incomplete_or_unfamiliar_programs() {
    let bytes = framed(b"}\x94\x8c\x01a\x94K\x01s.");
    for end in 0..bytes.len() {
        assert!(recognise(&bytes[..end]).is_none());
    }
    let mut trailing = bytes.clone();
    trailing.push(b'N');
    assert!(recognise(&trailing).is_none());
    for body in [
        &b"N0N."[..],
        b"N",
        b"N..",
        // A name for a slot the file never wrote. The slot it would have
        // is the list itself, which a form does read.
        b"]\x94h\x01a.",
        b"\x8c\x01\xff\x94.",
        b"\x95\0\0\0\0\0\0\0\0N.",
    ] {
        assert!(recognise(&framed(body)).is_none(), "{body:?}");
    }
    let mut wrong_frame = bytes;
    wrong_frame[3] -= 1;
    assert!(recognise(&wrong_frame).is_none());
}

#[test]
fn recursion_is_bounded() {
    let mut bytes = vec![0x80, 4];
    for _ in 0..MAX_DEPTH + 1 {
        bytes.extend_from_slice(b"]\x94");
    }
    bytes.push(b'N');
    bytes.extend(std::iter::repeat_n(b'a', MAX_DEPTH + 1));
    bytes.push(b'.');
    assert!(recognise(&bytes).is_none());
}

/// The bounds a file cannot talk its way past. None of these is a shape
/// CPython writes; they are what a file made to cost something looks
/// like, and each is turned away by its own limit rather than by running
/// out of bytes.
#[test]
fn a_hostile_file_costs_what_the_bounds_allow() {
    // Marks opened and never closed, which is what a nesting bomb is.
    let mut marks = vec![0x80, 4];
    marks.extend(std::iter::repeat_n(b'(', MAX_DEPTH + 1));
    marks.push(b'.');
    assert!(recognise(&marks).is_none());
    // Values pushed and never folded, past both the budget and the stack.
    let mut wide = vec![0x80, 4];
    wide.extend(std::iter::repeat_n(b'N', MAX_VALUES + 1));
    wide.push(b'.');
    assert!(recognise(&wide).is_none());
    // Memo marks with nothing in front of them to file.
    let mut memo = vec![0x80, 4];
    memo.extend(std::iter::repeat_n(0x94, 1000));
    memo.push(b'.');
    assert!(recognise(&memo).is_none());
    // A list whose one batch is longer than CPython ever writes.
    let mut batch = vec![0x80, 4, b']', 0x94, b'('];
    batch.extend(std::iter::repeat_n(b'N', MAX_BATCH + 1));
    batch.extend_from_slice(b"e.");
    assert!(recognise(&batch).is_none());
    // And the same length written the way CPython writes it, which is two
    // batches and a match.
    let mut batches = vec![0x80, 4, b']', 0x94, b'('];
    batches.extend(std::iter::repeat_n(b'N', MAX_BATCH));
    batches.push(b'e');
    batches.extend_from_slice(b"(Ne.");
    let found = recognise(&batches).unwrap();
    let Kind::List(items) = &found.value.kind else { panic!("list") };
    assert_eq!(items.len(), MAX_BATCH + 1);
}

#[test]
fn unmatched_input_keeps_symbolic_inspection() {
    let bytes = b"\x80\x04\x8c\x02os\x94\x8c\x06system\x94\x93.";
    assert!(recognise(bytes).is_none());
    let fallback = Program.run(bytes);
    let old = machine::Program.run(bytes);
    for at in 0..=bytes.len() as u64 {
        assert_eq!(
            fallback.text(Deduce::Builds, at),
            old.text(Deduce::Builds, at)
        );
    }
}

#[test]
fn nested_empty_containers_and_wide_text_are_captured() {
    let found = recognise(&framed(b"]\x94(]\x94K\x01}\x94\x8c\x01x\x94e.")).unwrap();
    let Kind::List(values) = found.value.kind else {
        panic!("list")
    };
    let kinds: Vec<_> = values[..3].iter().map(|v| &v.kind).collect();
    assert_eq!(
        kinds,
        vec![
            &Kind::List(vec![]),
            &Kind::Int { value: 1, at: 17, len: 1, spelled: false },
            &Kind::Dict(vec![])
        ]
    );
    assert!(matches!(values[3].kind, Kind::Text { len: 1, .. }));
    for (code, width) in [(0x58, 4), (0x8d, 8)] {
        let mut body = vec![code];
        body.extend_from_slice(&300u64.to_le_bytes()[..width]);
        body.extend(std::iter::repeat_n(b'x', 300));
        body.extend_from_slice(b"\x94.");
        assert!(matches!(
            recognise(&framed(&body)).unwrap().value.kind,
            Kind::Text { len: 300, .. }
        ));
        body[1..1 + width].fill(255);
        assert!(recognise(&framed(&body)).is_none());
    }
}

/// The builtins a pickle writes as a call: what each one accepts, and what
/// none of them do.
#[test]
fn the_builtin_calls_take_what_python_writes_and_nothing_else() {
    let call = |name: &str, args: &[u8], arity: u8| {
        // The empty tuple that says a call took no arguments is the one
        // CPython does not memoise.
        let close: Vec<u8> = match arity {
            0 => vec![b')'],
            n => vec![0x84 + n, 0x94],
        };
        cat(&[&word("builtins"), &word(name), b"\x93\x94", args, &close, b"R\x94"])
    };
    let one = |body: Vec<u8>| framed(&cat(&[&body, b"."]));
    let cases: Vec<(Vec<u8>, Shape)> = vec![
        (call("slice", b"K\x01K\x0aK\x02", 3), Shape::Slice),
        (call("slice", b"NNN", 3), Shape::Slice),
        (call("slice", b"M\x39\x30J\xff\xff\xff\xffK\x02", 3), Shape::Slice),
        (call("range", b"K\0K\x0aK\x02", 3), Shape::Range),
        (call("complex", b"G\x3f\xf8\0\0\0\0\0\0G\xc0\x04\0\0\0\0\0\0", 2), Shape::Complex),
        (call("bytearray", &blob(b"ab"), 1), Shape::ByteArray),
        // An empty bytearray is the class called with no arguments at all.
        (call("bytearray", b"", 0), Shape::ByteArray),
    ];
    for (body, want) in &cases {
        let found = recognise(&one(body.clone())).unwrap_or_else(|| panic!("{want:?} was not matched"));
        assert_eq!(found.form, "builtins-values-p4-p5-v3", "{want:?}");
        let Kind::Made { what, .. } = &found.value.kind else { panic!("{want:?} is not an object") };
        assert_eq!(what, want);
    }

    // The values themselves, read from the bytes they were written in.
    let found = recognise(&one(call("complex", b"G\x3f\xf8\0\0\0\0\0\0G\xc0\x04\0\0\0\0\0\0", 2))).unwrap();
    let Kind::Made { items, names, .. } = &found.value.kind else { panic!("object") };
    assert_eq!(*names, HALVES);
    let halves: Vec<f64> = items
        .iter()
        .map(|v| match v.kind {
            Kind::Float { value, .. } => value,
            _ => panic!("not a float"),
        })
        .collect();
    assert_eq!(halves, vec![1.5, -2.5]);

    // And what none of them take: another callable of the same module, the
    // wrong arity, a value of the wrong kind, and a module that is not
    // builtins.
    for body in [
        call("eval", b"K\x01K\x0aK\x02", 3),
        call("slice", b"K\x01K\x0a", 2),
        call("slice", b"K\x01K\x0aK\x02K\x03", 3),
        call("range", b"NK\x0aK\x02", 3),
        call("complex", b"K\x01K\x02", 2),
        call("bytearray", &word("ab"), 1),
        cat(&[&word("os"), &word("system"), b"\x93\x94)\x94R\x94"]),
    ] {
        assert!(recognise(&one(body.clone())).is_none(), "accepted {body:?}");
    }
}

/// What a builtins call made may be named again, and its arguments may not.
///
/// Two pandas frames whose blocks sit in the same places share the slice that
/// says so, and the second frame writes a `BINGET` where the first wrote the
/// call. The slice is something the form built; the tuple of arguments the
/// call was handed is only something it matched its way past.
#[test]
fn a_builtin_call_may_be_named_again_and_its_arguments_may_not() {
    // `[slice(1, 10, 2), <slot>]`. Slot 0 is the list, 1 and 2 the two words,
    // 3 the class, 4 the tuple of arguments and 5 the slice.
    let listed = |slot: u8| {
        let slice = cat(&[&word("builtins"), &word("slice"), b"\x93\x94", b"K\x01K\x0aK\x02", b"\x87\x94", b"R\x94"]);
        framed(&cat(&[b"]\x94(", &slice, &get(slot), b"e."]))
    };
    let found = recognise(&listed(5)).expect("a name for the slice");
    assert_eq!(found.form, "builtins-values-p4-p5-v3");
    let Kind::List(items) = &found.value.kind else { panic!("not a list") };
    let [spelled, named] = items.as_slice() else { panic!("not two items") };
    assert!(matches!(spelled.kind, Kind::Made { what: Shape::Slice, .. }));
    // The name says where the slice was written, which is where the call
    // that made it starts.
    let Kind::Ref(Names::Made { what: Shape::Slice, at, hashable: true }) = named.kind else { panic!("{:?}", named.kind) };
    assert_eq!(at, spelled.at);
    assert!(recognise(&listed(4)).is_none(), "named the arguments of a call");
}

/// The two picklers CPython ships, and the tails that tell them apart.
///
/// `_pickle` writes a batch for whatever a container has left over after a
/// full one, even when that is nothing; `pickle.py` writes APPEND or SETITEM
/// for a single item and nothing at all for none. Both are what a real
/// pickler writes, so both are read, and the file says which it was. A file
/// showing one of them at one batch edge and the other at another was written
/// by neither.
#[test]
fn a_file_shows_one_pickler_or_neither_and_never_both() {
    // A thousand items, which is the batch CPython fills before it opens
    // another, and then whatever the writer had left.
    let full = |open: &[u8], item: &[u8], close: u8| {
        let mut out = open.to_vec();
        out.push(b'(');
        out.extend((0..MAX_BATCH).flat_map(|_| item.iter().copied()));
        out.push(close);
        out
    };
    let list = |tail: &[u8]| framed(&cat(&[&full(b"]\x94", b"K\x01", b'e'), tail, b"."]));
    let dict = |tail: &[u8]| framed(&cat(&[&full(b"}\x94", b"K\x01K\x02", b'u'), tail, b"."]));
    let said = |bytes: &[u8]| recognise(bytes).map(|found| found.pickler);

    // One item over a full batch, and a container whose length is exactly a
    // multiple of a thousand.
    assert_eq!(said(&list(b"(K\x01e")), Some(Pickler::C));
    assert_eq!(said(&list(b"K\x01a")), Some(Pickler::Python));
    assert_eq!(said(&dict(b"(K\x01K\x02u")), Some(Pickler::C));
    assert_eq!(said(&dict(b"(u")), Some(Pickler::C));
    assert_eq!(said(&dict(b"K\x01K\x02s")), Some(Pickler::Python));
    assert_eq!(said(&dict(b"")), Some(Pickler::Python));
    // A set has no shorthand in either pickler, so a batch of one after a
    // full one says nothing about who wrote it.
    assert_eq!(said(&framed(&cat(&[&full(b"\x8f\x94", b"K\x01", 0x90), b"(K\x01\x90."]))), Some(Pickler::Undetermined));
    // And neither does a file with no batch edge in it at all.
    assert_eq!(said(&framed(b"}\x94\x8c\x01a\x94K\x01s.")), Some(Pickler::Undetermined));

    // Two containers in one file, spelled by two different picklers.
    let mixed = framed(&cat(&[
        b"]\x94(",
        &full(b"]\x94", b"K\x01", b'e'),
        b"(K\x01e",
        &full(b"}\x94", b"K\x01K\x02", b'u'),
        b"K\x01K\x02s",
        b"e.",
    ]));
    assert!(recognise(&mixed).is_none(), "one file, two picklers");

    // The memo mark after a BYTEARRAY8, which pickle.py did not write until
    // Python 3.10. Every slot after it is numbered one lower.
    let marked = proto5(b"]\x94(\x96\x02\0\0\0\0\0\0\0ab\x94\x8c\x01x\x94h\x02e.");
    assert_eq!(said(&marked), Some(Pickler::Undetermined));
    let unmarked = proto5(b"]\x94(\x96\x02\0\0\0\0\0\0\0ab\x8c\x01x\x94h\x01e.");
    assert_eq!(said(&unmarked), Some(Pickler::Python));
    // The same bytes with the slot numbering of the other spelling name a
    // slot the file never wrote.
    assert!(recognise(&proto5(b"]\x94(\x96\x02\0\0\0\0\0\0\0ab\x8c\x01x\x94h\x02e.")).is_none());
}

/// A form is the productions it allows, and a file is read under exactly
/// one of them.
#[test]
fn a_form_is_the_productions_it_allows() {
    assert_eq!(recognise(&framed(b"}\x94.")).unwrap().form, "basic-p4-p5-v5");
    let slice = cat(&[
        b"}\x94",
        &word("s"),
        &word("builtins"),
        &word("slice"),
        b"\x93\x94K\x01K\x02K\x03\x87\x94R\x94s.",
    ]);
    assert_eq!(recognise(&framed(&slice)).unwrap().form, "builtins-values-p4-p5-v3");
    let array = cat(&[b"}\x94", &word("a"), &one_array(2, "i1", b'|', b"K\x02\x85\x94", &[1, 2]), b"s."]);
    assert_eq!(recognise(&framed(&array)).unwrap().form, "numpy-array-p4-p5-v6");
    // A file holding both is read under the mixed form, which is every
    // family's productions at once and says which of them the file used.
    // Neither single-family form takes it: each requires the file to have
    // used its own production and nothing else's.
    let both = cat(&[
        b"}\x94(",
        &word("a"),
        &one_array(2, "i1", b'|', b"K\x02\x85\x94", &[1, 2]),
        &word("s"),
        &word("builtins"),
        &word("slice"),
        b"\x93\x94K\x01K\x02K\x03\x87\x94R\x94u.",
    ]);
    let found = recognise(&framed(&both)).unwrap();
    assert_eq!(found.form, "mixed-values-p4-p5-v1");
    assert_eq!(found.families(), "basic, builtins, numpy");
}

/// A LONG1 declares up to 255 bytes and sixteen is as far as the reader's
/// integer type reaches, so past that the number is the digits it comes to,
/// worked out once as the form reads the run.
#[test]
fn a_number_past_the_integer_type_is_its_digits() {
    let long1 = |run: &[u8]| {
        let mut body = vec![0x8a, run.len() as u8];
        body.extend_from_slice(run);
        body.push(b'.');
        framed(&body)
    };
    let kind = |run: &[u8]| recognise(&long1(run)).unwrap_or_else(|| panic!("{run:02x?} was not read")).value.kind;
    // The two ends of the reader's own integer type, each in the sixteen
    // bytes CPython writes it in.
    let mut most = vec![0xff; 15];
    most.push(0x7f);
    let mut least = vec![0x00; 15];
    least.push(0x80);
    assert_eq!(kind(&most), Kind::Int { value: i128::MAX, at: 13, len: 16, spelled: false });
    assert_eq!(kind(&least), Kind::Int { value: i128::MIN, at: 13, len: 16, spelled: false });
    // One past each of those takes a seventeenth byte, which carries the
    // sign, and is read as its digits instead.
    let mut above = least.clone();
    above.push(0x00);
    let mut below = most.clone();
    below.push(0xff);
    let digits = |run: &[u8]| match kind(run) {
        Kind::Wide { digits, at, len, spelled } => {
            assert_eq!((at, len, spelled), (13, run.len(), false));
            digits
        }
        other => panic!("{run:02x?} was read as {other:?}"),
    };
    assert_eq!(digits(&above), "170141183460469231731687303715884105728");
    assert_eq!(digits(&below), "-170141183460469231731687303715884105729");
    // A `uuid.UUID` is 128 bits, so whenever its top bit is set it goes out
    // as sixteen bytes and the nought that says it is not negative.
    let mut uuid = vec![0xff; 16];
    uuid.push(0x00);
    assert_eq!(digits(&uuid), "340282366920938463463374607431768211455");
    // Two to the two hundredth, both signs, in the twenty-six bytes CPython
    // writes it in.
    let mut huge = vec![0x00; 25];
    huge.push(0x01);
    let mut down = vec![0x00; 25];
    down.push(0xff);
    assert_eq!(digits(&huge), "1606938044258990275541962092341162602522202993782792835301376");
    assert_eq!(digits(&down), "-1606938044258990275541962092341162602522202993782792835301376");
    // The widest run the opcode can declare.
    let mut widest = vec![0x00; 254];
    widest.push(0x01);
    assert!(matches!(kind(&widest), Kind::Wide { len: 255, .. }));
    // CPython writes no byte a number does not need, at any width, and a
    // LONG1 of nothing is the integer nought, which is a BININT1 here.
    for wrong in [&[][..], &[0xff, 0xff][..], &[0x01, 0x00][..]] {
        assert!(recognise(&long1(wrong)).is_none(), "{wrong:02x?} was read");
    }
    let mut padded = most.clone();
    padded.push(0x00);
    padded.push(0x00);
    assert!(recognise(&long1(&padded)).is_none());
    let mut sign_padded = huge.clone();
    sign_padded.push(0x00);
    assert!(recognise(&long1(&sign_padded)).is_none());
}
