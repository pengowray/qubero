//! The NumPy productions, against the fixture and against pickles written
//! here: dimensions, dtypes and byte order, the protocol 5 buffer call, the
//! scalar, what a second array may name out of the memo, and what a changed
//! instruction does to a match.

use super::*;

#[test]
fn captures_numpy_payload_and_rejects_changed_structure() {
    let found = recognise(MATRIX).unwrap();
    assert_eq!(found.form, "numpy-array-p4-p5-v7");
    let Kind::Dict(entries) = &found.value.kind else {
        panic!("dict expected")
    };
    let Kind::Array {
        at,
        dimensions,
        dtype,
        fortran_order,
        ..
    } = &entries[0].1.kind
    else {
        panic!("array expected")
    };
    let at = *at;
    assert_eq!(dimensions, &[4, 6]);
    assert_eq!(spelling(dtype), "<f4");
    assert!(!fortran_order);
    assert_eq!(found.int(Deduce::PayloadCount, at as u64), Some(24));
    let floats: Vec<_> = MATRIX[at..at + 96]
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    assert_eq!(floats, (0..24).map(|n| n as f32).collect::<Vec<_>>());
    for end in 0..MATRIX.len() {
        assert!(recognise(&MATRIX[..end]).is_none());
    }
    // Mutate fixed control bytes, shape, callable, memo reference and dtype.
    for offset in [0x30, 0x40, 0x65, 0x6b, 0x79, 0x9b, 0x100] {
        let mut changed = MATRIX.to_vec();
        changed[offset] ^= 1;
        assert!(
            recognise(&changed).is_none(),
            "accepted mutation at {offset:x}"
        );
    }
    let mut payload = MATRIX.to_vec();
    payload[at] = b'R'; // An opcode byte inside captured data is just data.
    assert!(recognise(&payload).is_some());
    let mut appended = MATRIX.to_vec();
    appended.push(b'N');
    assert!(recognise(&appended).is_none());
}

#[test]
fn standalone_arrays_preserve_dimensions_dtype_and_storage_order() {
    let mut body = standalone();
    replace(&mut body, b"K\x04K\x06\x86\x94", b"K\x02K\x03K\x04\x87\x94");
    replace(
        &mut body,
        b"\x8c\x16numpy._core.multiarray",
        b"\x8c\x15numpy.core.multiarray",
    );
    replace(&mut body, b"\x8c\x02f4\x94", b"\x8c\x02i4\x94");
    replace(&mut body, b"\x8c\x01<\x94NNN", b"\x8c\x01>\x94NNN");
    replace(&mut body, b"t\x94b\x89C", b"t\x94b\x88C");
    let bytes = framed(&body);
    let found = recognise(&bytes).unwrap();
    let Kind::Array {
        at,
        len,
        dtype,
        dimensions,
        fortran_order,
        ..
    } = &found.value.kind
    else {
        panic!("array")
    };
    assert_eq!(
        (*len, spelling(dtype), dimensions.as_slice(), *fortran_order),
        (96, ">i4", &[2, 3, 4][..], true)
    );
    assert_eq!(found.int(Deduce::PayloadCount, *at as u64), Some(24));
    assert_eq!(
        found.int(Deduce::PayloadShape, *at as u64),
        Some(shapes::dtype(">i4").unwrap().0 as i128)
    );
    // The standalone memo reference must match the slot for numpy, not
    // the dictionary variant's slot or any other existing slot.
    replace(&mut body, b"h\x03\x8c\x05dtype", b"h\x05\x8c\x05dtype");
    assert!(recognise(&framed(&body)).is_none());
}

#[test]
fn numeric_dtype_branches_check_byte_order_and_payload_width() {
    for (kind, width, order) in [
        ("b1", 1, b'|'),
        ("i1", 1, b'|'),
        ("u1", 1, b'|'),
        ("i2", 2, b'<'),
        ("u2", 2, b'>'),
        ("i4", 4, b'<'),
        ("u4", 4, b'>'),
        ("i8", 8, b'<'),
        ("u8", 8, b'>'),
        ("f2", 2, b'<'),
        ("f4", 4, b'>'),
        ("f8", 8, b'<'),
        ("c8", 8, b'>'),
        ("c16", 16, b'<'),
    ] {
        let mut body = standalone();
        let mut dtype = vec![0x8c, kind.len() as u8];
        dtype.extend_from_slice(kind.as_bytes());
        dtype.push(0x94);
        replace(&mut body, b"\x8c\x02f4\x94", &dtype);
        replace(
            &mut body,
            b"\x8c\x01<\x94NNN",
            &[0x8c, 1, order, 0x94, b'N', b'N', b'N'],
        );
        let mut dims = vec![b'K', (96 / width) as u8, 0x85, 0x94];
        replace(&mut body, b"K\x04K\x06\x86\x94", &dims);
        assert!(recognise(&framed(&body)).is_some(), "{kind}");
        dims[1] += 1;
        let before = [b'K', (96 / width) as u8, 0x85, 0x94];
        replace(&mut body, &before, &dims);
        assert!(
            recognise(&framed(&body)).is_none(),
            "wrong length for {kind}"
        );
    }
}

/// Fixed-width text is the one plain dtype whose letters do not say how many
/// bytes a value is: `U3` is three characters and twelve bytes. So the width
/// is read out of the state, where every other plain dtype writes minus one,
/// and an array of labels is measured against that. A classifier fitted on
/// labels that are strings holds one.
#[test]
fn a_text_dtype_takes_its_width_from_the_state_rather_than_its_letters() {
    for (kind, order, elsize, alignment, flags) in [("U2", b'<', 8u8, 4u8, 8u8), ("U3", b'>', 12, 4, 8), ("S4", b'|', 4, 1, 0), ("S1", b'|', 1, 1, 0)] {
        let mut body = standalone();
        replace(&mut body, b"\x8c\x02f4\x94", &word(kind));
        let bounds = [b'K', elsize, b'K', alignment, b'K', flags];
        let state = cat(&[&[0x8c, 1, order, 0x94][..], b"NNN", &bounds]);
        replace(&mut body, b"\x8c\x01<\x94NNNJ\xff\xff\xff\xffJ\xff\xff\xff\xffK\x00", &state);
        replace(&mut body, b"K\x04K\x06\x86\x94", &[b'K', 96 / elsize, 0x85, 0x94]);
        let found = recognise(&framed(&body)).unwrap_or_else(|| panic!("{kind} was refused"));
        let Kind::Array { dtype, dimensions, len, .. } = &found.value.kind else { panic!("array expected for {kind}") };
        assert_eq!((spelling(dtype), dimensions.as_slice(), *len), (format!("{}{kind}", char::from(order)).as_str(), &[96 / u64::from(elsize)][..], 96));
        // The width in the state and the width in the letters have to agree,
        // or the array would be measured in values of the wrong size.
        let mut wrong = body.clone();
        replace(&mut wrong, &bounds, &[b'K', elsize + 1, b'K', alignment, b'K', flags]);
        assert!(recognise(&framed(&wrong)).is_none(), "accepted a width the letters of {kind} disagree with");
    }
}

#[test]
fn scalar_empty_and_wide_arrays_obey_length_and_shape_constraints() {
    let mut base = standalone();
    // Replace the complete payload with one float (the scalar form).
    let payload = &MATRIX[0x9b..0xfd];
    assert_eq!(&payload[..2], b"C\x60");
    replace(&mut base, payload, b"C\x04\0\0\x80\x3f");
    replace(&mut base, b"K\x04K\x06\x86\x94", b")");
    let scalar = recognise(&framed(&base)).unwrap();
    let Kind::Array { dimensions, .. } = scalar.value.kind else {
        panic!("array")
    };
    assert!(dimensions.is_empty());

    let mut empty = standalone();
    replace(&mut empty, payload, b"C\0");
    replace(&mut empty, b"K\x04K\x06\x86\x94", b"K\0\x85\x94");
    assert!(recognise(&framed(&empty)).is_some());

    for code in [b'B', 0x8e] {
        let mut wide = standalone();
        replace(&mut wide, b"K\x04K\x06\x86\x94", b"M\0\x01\x85\x94");
        let mut data = vec![code];
        if code == b'B' {
            data.extend_from_slice(&1024u32.to_le_bytes());
        } else {
            data.extend_from_slice(&1024u64.to_le_bytes());
        }
        data.resize(data.len() + 1024, 0);
        replace(&mut wide, payload, &data);
        assert!(recognise(&framed(&wide)).is_some());
    }
    let mut overflow = standalone();
    replace(
        &mut overflow,
        b"K\x04K\x06\x86\x94",
        b"J\xff\xff\xff\x7fJ\xff\xff\xff\x7fJ\xff\xff\xff\x7f\x87\x94",
    );
    assert!(recognise(&framed(&overflow)).is_none());
    let mut negative = standalone();
    replace(
        &mut negative,
        b"K\x04K\x06\x86\x94",
        b"J\xff\xff\xff\xff\x85\x94",
    );
    assert!(recognise(&framed(&negative)).is_none());
}

/// NumPy's protocol 5 call, which hands the array's numbers over as a
/// buffer rather than writing them after the call that rebuilds it.
#[test]
fn an_array_at_protocol_5_is_rebuilt_around_its_buffer() {
    let numbers: Vec<u8> = (0u8..12).flat_map(|n| [n, 0]).collect();
    let whole = proto5(&cat(&[&frombuffer(&mutable(&numbers), "i2", b'<', b"K\x03K\x04\x86\x94", "C"), b"."]));
    let found = recognise(&whole).unwrap();
    assert_eq!(found.form, "numpy-array-p4-p5-v7");
    let Kind::Array { dtype, dimensions, len, fortran_order, .. } = &found.value.kind else { panic!("array") };
    assert_eq!((spelling(dtype), dimensions.as_slice(), *len, *fortran_order), ("<i2", &[3, 4][..], 24, false));

    // A read-only array hands over a byte string instead, and Fortran
    // order is the other letter.
    let readonly = proto5(&cat(&[&frombuffer(&blob(&numbers), "i2", b'<', b"K\x03K\x04\x86\x94", "F"), b"."]));
    let found = recognise(&readonly).unwrap();
    let Kind::Array { fortran_order, .. } = &found.value.kind else { panic!("array") };
    assert!(fortran_order);

    // The numbers are one of the call's arguments, so the call holds them
    // and the array has nothing beside it.
    let seen = dump(&whole);
    assert_eq!(named_row(&seen, "shape").value, V::Str("3 x 4".into()));
    let call = named_row(&seen, "ndarray frombuffer call");
    let held = named_row(&seen, "numbers");
    assert_eq!((call.at, call.len), (11, whole.len() as u64 - 12));
    assert!(held.at > call.at && held.at + held.len < call.at + call.len);
    assert_eq!((held.ty.as_str(), held.len, &held.value), ("i16 le[]", 24, &V::Composite { count: 12 }));
    assert_eq!(named_row(&seen, "order letter").value, V::Str("C".into()));
    tiles(&seen);

    // Protocol 4 never writes this call, whatever else is right about it.
    assert!(recognise(&framed(&cat(&[&frombuffer(&blob(&numbers), "i2", b'<', b"K\x03K\x04\x86\x94", "C"), b"."]))).is_none());
    // Nor does NumPy write a letter that is not C or F, or a buffer that
    // does not come to the shape it declared.
    for (buffer, shape, letter) in [
        (&numbers[..], &b"K\x03K\x04\x86\x94"[..], "A"),
        (&numbers[..2], b"K\x03K\x04\x86\x94", "C"),
    ] {
        let wrong = proto5(&cat(&[&frombuffer(&blob(buffer), "i2", b'<', shape, letter), b"."]));
        assert!(recognise(&wrong).is_none(), "accepted {letter} and {} bytes", buffer.len());
    }
}

/// Every way the second array of a file may name what the first one wrote,
/// and the ones that are not a way.
#[test]
fn a_later_array_may_name_what_an_earlier_one_wrote() {
    // The whole finished dtype, out of the slot its REDUCE filed it in.
    let shared = two_arrays(&get(17));
    let found = recognise(&framed(&shared)).unwrap();
    assert_eq!(found.form, "numpy-array-p4-p5-v7");
    let Kind::Dict(entries) = &found.value.kind else { panic!("dict") };
    let dtypes: Vec<&str> = entries
        .iter()
        .map(|(_, v)| match &v.kind {
            Kind::Array { dtype, .. } => spelling(dtype),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(dtypes, vec!["|i1", "|i1"]);
    // The class and the letter separately: the dtype class out of its
    // slot and the byte order out of the slot the first array wrote it in.
    let apart = two_arrays(&cat(&[
        &get(14),
        &word("i1"),
        b"\x89\x88\x87\x94R\x94(K\x03",
        &get(18),
        b"NNNJ\xff\xff\xff\xffJ\xff\xff\xff\xffK\0t\x94b",
    ]));
    assert!(recognise(&framed(&apart)).is_some());
    // And the whole dtype written out again, which is what a file with two
    // unlike arrays does. Its module name may be a reference or a word.
    let again = two_arrays(&dtype_state(&get(5), "i1", b'|'));
    assert!(recognise(&framed(&again)).is_some());
    let again = two_arrays(&dtype_state(&get(9), "i1", b'|'));
    assert!(recognise(&framed(&again)).is_none(), "slot 9 holds a byte string, not a module name");
    // And the numbers themselves. Two arrays holding the same bytes are one
    // byte string to Python, so the second names the run the first wrote and
    // has no mark of its own after it. A fitted `SVC` writes its two empty
    // probability arrays that way. The first array's run is slot 20, which is
    // the byte order at 18 and the dtype's own last mark at 19.
    let mut shared_run = two_arrays(&get(17));
    replace(&mut shared_run, b"(K\x01K\x03\x85\x94", b"(K\x01K\x02\x85\x94");
    replace(&mut shared_run, &blob(&[1, 2, 3]), &get(20));
    let found = recognise(&framed(&shared_run)).unwrap();
    let Kind::Dict(entries) = &found.value.kind else { panic!("dict") };
    let runs: Vec<(usize, usize)> = entries
        .iter()
        .map(|(_, v)| match &v.kind {
            Kind::Array { at, len, .. } => (*at, *len),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(runs[0], runs[1], "both arrays read the one run the file wrote");
    // Only a slot holding a run of bytes. Slot 17 is the finished dtype and
    // slot 1 a key, and an array's numbers are neither.
    for slot in [17u8, 1] {
        let mut wrong = shared_run.clone();
        replace(&mut wrong, &get(20), &get(slot));
        assert!(recognise(&framed(&wrong)).is_none(), "accepted an array's numbers at slot {slot}");
    }
}

/// A reference is only ever to a slot the form itself filled with the
/// thing the grammar expects to find there.
#[test]
fn a_reference_to_the_wrong_slot_is_a_non_match() {
    assert!(recognise(&framed(&two_arrays(&get(17)))).is_some());
    // Past the end of the memo, at a slot the file has not written yet,
    // and at slots holding the dictionary, a key, a global, a byte string
    // and the byte order: none of those is a dtype.
    for slot in [255u8, 30, 0, 1, 4, 9, 18] {
        assert!(recognise(&framed(&two_arrays(&get(slot)))).is_none(), "accepted a dtype at slot {slot}");
    }
    // The ndarray class where the reconstructor belongs, and the other way
    // round: both slots hold a global, and neither holds the right one.
    let swapped = cat(&[
        b"}\x94(",
        &word("a"),
        &reconstruct(),
        b"(K\x01K\x02\x85\x94",
        &dtype_state(&get(5), "i1", b'|'),
        b"\x89",
        &blob(&[1, 2]),
        b"t\x94b",
        &word("b"),
        &get(7),
        &get(4),
        b"K\0\x85\x94",
        &get(9),
        b"\x87\x94R\x94(K\x01K\x03\x85\x94",
        &get(17),
        b"\x89",
        &blob(&[1, 2, 3]),
        b"t\x94bu.",
    ]);
    assert!(recognise(&framed(&swapped)).is_none());
    // LONG_BINGET reaches the same slots the long way round, which is what
    // a file with more than 256 of them has to do.
    let long = cat(&[
        b"}\x94(",
        &word("a"),
        &reconstruct(),
        b"(K\x01K\x02\x85\x94",
        &dtype_state(&get(5), "i1", b'|'),
        b"\x89",
        &blob(&[1, 2]),
        b"t\x94b",
        &word("b"),
        b"j\x04\0\0\0j\x07\0\0\0K\0\x85\x94j\x09\0\0\0\x87\x94R\x94(K\x01K\x03\x85\x94j\x11\0\0\0\x89",
        &blob(&[1, 2, 3]),
        b"t\x94b",
        b"u.",
    ]);
    assert!(recognise(&framed(&long)).is_some());
}

/// A NumPy scalar is one number written as a call of its own.
#[test]
fn a_numpy_scalar_is_one_number_of_its_dtype() {
    let scalar = |data: &[u8]| {
        cat(&[
            b"}\x94(",
            &word("a"),
            &reconstruct(),
            b"(K\x01K\x02\x85\x94",
            &dtype_state(&get(5), "i2", b'<'),
            b"\x89",
            &blob(&[1, 0, 2, 0]),
            b"t\x94b",
            &word("b"),
            &get(2),
            &word("scalar"),
            b"\x93\x94",
            &get(17),
            &blob(data),
            b"\x86\x94R\x94",
            b"u.",
        ])
    };
    let found = recognise(&framed(&scalar(&[7, 0]))).unwrap();
    let Kind::Dict(entries) = &found.value.kind else { panic!("dict") };
    let Kind::Array { dtype, dimensions, len, .. } = &entries[1].1.kind else { panic!("array") };
    assert_eq!((spelling(dtype), dimensions.as_slice(), *len), ("<i2", &[][..], 2));
    assert_eq!(found.calls.len(), 2);
    assert_eq!(found.calls[1].name, "numpy scalar call");
    // One value of the dtype and no more: a scalar is not an array of one.
    assert!(recognise(&framed(&scalar(&[7, 0, 8, 0]))).is_none(), "two values in a scalar");
    assert!(recognise(&framed(&scalar(&[7]))).is_none(), "half a value in a scalar");
}

/// An instruction moved, dropped or added, a length written in another
/// width, bytes after the STOP, and every truncation of the file.
#[test]
fn a_changed_instruction_leaves_no_match() {
    let body = two_arrays(&get(17));
    let whole = framed(&body);
    assert!(recognise(&whole).is_some());

    for end in 0..whole.len() {
        assert!(recognise(&whole[..end]).is_none(), "accepted {end} bytes of the file");
    }
    let mut after = whole.clone();
    after.push(b'N');
    assert!(recognise(&after).is_none(), "accepted a value after the STOP");

    // One instruction dropped: the memo mark that files the dictionary,
    // the TUPLE1 that closes the placeholder shape, and a BUILD.
    for cut in [0x94u8, 0x85, b'b'] {
        let at = body.iter().position(|b| *b == cut).unwrap();
        let mut short = body.clone();
        short.remove(at);
        assert!(recognise(&framed(&short)).is_none(), "accepted the file without {cut:#x}");
    }
    // One instruction inserted, beside every memo mark in the file.
    for at in 0..body.len() {
        if body[at] != 0x94 {
            continue;
        }
        let mut longer = body.clone();
        longer.insert(at, 0x94);
        assert!(recognise(&framed(&longer)).is_none(), "accepted an extra memo mark at {at}");
    }
    // Two instructions swapped: the flags a dtype is built with.
    let mut reordered = body.clone();
    let at = reordered.windows(2).position(|w| w == b"\x89\x88").unwrap();
    reordered.swap(at, at + 1);
    assert!(recognise(&framed(&reordered)).is_none(), "accepted NEWTRUE before NEWFALSE");

    // A length in another width. BINBYTES is an alternative of its own, so
    // it is matched; a length that no longer comes to the declared shape
    // is not.
    let at = body.windows(4).position(|w| w == b"C\x03\x01\x02").unwrap();
    let mut wide = body.clone();
    wide.splice(at..at + 2, [b'B', 3, 0, 0, 0]);
    assert!(recognise(&framed(&wide)).is_some());
    let mut mismatched = wide.clone();
    mismatched[at + 1] = 4;
    assert!(recognise(&framed(&mismatched)).is_none(), "a length past the shape it declared");
}

/// The reconstructor is handed `self.__class__`, so the class named in it is
/// the array's own. NumPy's are enumerated and read, each with its own name on
/// the node; anyone else's is a non-match, because what such a class does to an
/// array when it is rebuilt is that class's business and no enumeration covers
/// it.
#[test]
fn numpy_s_own_array_classes_are_read_and_anyone_else_s_is_not() {
    let body = one_array(0, "f8", b'<', b"K\x02\x85\x94", &[0; 16]);
    let at = body.windows(10).position(|w| w == b"\x8c\x07ndarray\x94").unwrap();
    let named = |name: &str| {
        let mut changed = body.clone();
        changed.splice(at..at + 10, word(name));
        changed.push(b'.');
        framed(&changed)
    };
    for (name, shape) in [("ndarray", Shape::Array), ("matrix", Shape::Matrix), ("memmap", Shape::MemMap)] {
        let whole = named(name);
        let found = recognise(&whole).unwrap_or_else(|| panic!("{name}: read as far as {:#x}", furthest(&whole)));
        let Kind::Array { class, dimensions, .. } = &found.value.kind else { panic!("an array expected") };
        assert_eq!((*class, dimensions.as_slice()), (shape, &[2][..]), "{name}");
        // The node says which class it is where every other array says
        // `array`, and everything else about it is an array's.
        assert_eq!(named_row(&dump(&whole), "data").ty, shape.name(), "{name}");
    }
    // A record array, which holds a record and not the plain numbers this
    // body writes; one of NumPy's classes that is not an array at all; and a
    // name close enough to be a typo.
    for name in ["recarray", "dtype", "ndarray_"] {
        assert!(recognise(&named(name)).is_none(), "{name} was read as an array");
    }
}

/// One masked array written out in full: the reconstructor, the class and the
/// class its data is an array of, the placeholder shape and dtype letter, and
/// the BUILD whose state is an ordinary array's five things, the mask and the
/// fill value.
///
/// Counting the memo marks out: 6 is the word `numpy`, which the dtype names
/// again, and 12 is what the call made.
fn masked(data: &[u8], mask: &[u8], fill: &[u8]) -> Vec<u8> {
    framed(&cat(&[
        &word("numpy.ma.core"),
        &word("_mareconstruct"),
        b"\x93\x94(",
        &word("numpy.ma"),
        &word("MaskedArray"),
        b"\x93\x94",
        &word("numpy"),
        &word("ndarray"),
        b"\x93\x94",
        b"K\x00\x85\x94",
        &word("b"),
        b"t\x94R\x94",
        b"(K\x01K\x04\x85\x94",
        &dtype_state(&word("numpy"), "f8", b'<'),
        b"\x89",
        &blob(data),
        &blob(mask),
        fill,
        b"t\x94b.",
    ]))
}

/// A `numpy.ma.MaskedArray`, which is `_mareconstruct` and a BUILD whose
/// state holds two runs: the numbers, and one byte an entry saying which of
/// them count. Both are read as the arrays they are.
#[test]
fn a_masked_array_is_the_numbers_the_mask_and_the_fill_value() {
    let numbers = &[0u8; 32];
    let whole = masked(numbers, b"\x00\x01\x00\x01", b"N");
    let found = recognise(&whole).unwrap_or_else(|| panic!("read as far as {:#x}", furthest(&whole)));
    assert_eq!(found.form, "numpy-array-p4-p5-v7");
    let Kind::Masked { data, mask, fill } = &found.value.kind else { panic!("a masked array expected") };
    let run = |v: &Value| match &v.kind {
        Kind::Array { at, len, dtype, dimensions, .. } => (*at, *len, spelling(dtype).to_string(), dimensions.clone()),
        other => panic!("an array expected, not {other:?}"),
    };
    assert_eq!(run(data), (170, 32, "<f8".to_string(), vec![4]));
    assert_eq!(run(mask), (205, 4, "|b1".to_string(), vec![4]));
    // Nothing where the array kept NumPy's default for its dtype.
    assert_eq!(fill.kind, Kind::None);
    let seen = dump(&whole);
    tiles(&seen);
    assert_eq!(named_row(&seen, "data").ty, "masked array");
    assert_eq!(named_row(&seen, "mask").ty, "array");
    assert_eq!(named_row(&seen, "fill value").ty, "null");

    // A fill value the array was given, which NumPy writes as an array of no
    // dimensions.
    let given = cat(&[&word("numpy._core.multiarray"), &word("_reconstruct"), b"\x93\x94", &word("numpy"), &word("ndarray"), b"\x93\x94K\x00\x85\x94", &blob(b"b"), b"\x87\x94R\x94(K\x01)", &dtype_state(&word("numpy"), "f8", b'<'), b"\x89", &blob(&[0; 8]), b"t\x94b"]);
    let whole = masked(numbers, b"\x00\x01\x00\x01", &given);
    let found = recognise(&whole).unwrap_or_else(|| panic!("a given fill value: read as far as {:#x}", furthest(&whole)));
    let Kind::Masked { fill, .. } = &found.value.kind else { panic!("a masked array expected") };
    assert!(matches!(fill.kind, Kind::Array { dimensions: ref d, .. } if d.is_empty()));

    // A mask that is not one byte an entry, and a run of numbers that is not
    // the shape the state declared.
    assert!(recognise(&masked(numbers, b"\x00\x01\x00", b"N")).is_none(), "a mask short of one byte an entry");
    assert!(recognise(&masked(&[0; 24], b"\x00\x01\x00\x01", b"N")).is_none(), "numbers short of the shape");
}

/// A `numpy.recarray`, whose dtype is the one dtype NumPy writes as a class:
/// `numpy.dtype(numpy.record, ...)` where every other dtype writes the letters
/// it is spelled by. The width the letters would have said is in the state.
#[test]
fn a_record_array_names_its_dtype_by_the_class_rather_than_by_letters() {
    const COLUMNS: &[(&str, &str, &str, u64)] = &[("a", "i4", "<", 0), ("b", "f8", "<", 8)];
    // The class moved between NumPy 1 and 2 and the class it holds did not.
    for (module, name) in [("numpy", "recarray"), ("numpy.rec", "recarray")] {
        let mut w = Writing::default();
        w.record_array_of(module, name, 2, 16, COLUMNS, &[0; 32], true);
        let bytes = framed(&cat(&[&w.out, b"."]));
        let found = recognise(&bytes).unwrap_or_else(|| panic!("{module}.{name}: read as far as {:#x}", furthest(&bytes)));
        assert_eq!(found.form, "numpy-array-p4-p5-v7");
        let Kind::Array { dtype, dimensions, class, .. } = &found.value.kind else { panic!("array expected") };
        assert_eq!((*class, dimensions.as_slice()), (Shape::RecArray, &[2][..]));
        assert_eq!(
            dtype,
            &Dtype::Record {
                width: 16,
                columns: vec![
                    Column { name: "a".into(), dtype: "<i4".into(), at: 0 },
                    Column { name: "b".into(), dtype: "<f8".into(), at: 8 },
                ],
            }
        );
        let seen = dump(&bytes);
        tiles(&seen);
        assert_eq!(named_row(&seen, "data").ty, "recarray");
    }
    // The class is named where the letters would be and nowhere else: an
    // ordinary array's dtype is letters, and a class no form wrote down is a
    // non-match however respectable its module looks.
    let mut w = Writing::default();
    w.record_array_of("numpy", "ndarray", 2, 16, COLUMNS, &[0; 32], false);
    assert!(recognise(&framed(&cat(&[&w.out, b"."]))).is_some());
    let mut w = Writing::default();
    w.record_array_of("numpy", "matrix", 2, 16, COLUMNS, &[0; 32], true);
    assert!(recognise(&framed(&cat(&[&w.out, b"."]))).is_some(), "a matrix may write the class too");
    let mut w = Writing::default();
    w.record_array_of("numpy", "bogusarray", 2, 16, COLUMNS, &[0; 32], true);
    assert!(recognise(&framed(&cat(&[&w.out, b"."]))).is_none(), "a class no form named was read");
}

/// A masked array of a structured dtype, whose mask is one boolean per column
/// per row rather than one per entry. That is what `make_mask_descr` builds,
/// and it makes the mask a narrower run than the numbers it covers.
#[test]
fn a_masked_record_s_mask_is_one_boolean_a_column() {
    // Two rows of an `i4` and an `f8`, which is sixteen bytes a row, with a
    // mask of two bytes a row.
    let whole = |mask: &[u8]| {
        framed(&cat(&[
            &word("numpy.ma.core"),
            &word("_mareconstruct"),
            b"\x93\x94(",
            &word("numpy.ma"),
            &word("MaskedArray"),
            b"\x93\x94",
            &word("numpy"),
            &word("ndarray"),
            b"\x93\x94",
            b"K\x00\x85\x94",
            &word("b"),
            b"t\x94R\x94",
            b"(K\x01K\x02\x85\x94",
            &{
                let mut w = Writing::default();
                // The slots the dtype files run on from the ones the call
                // above took, which is what the words before it counted.
                w.slots = 14;
                w.record_dtype(16, &[("a", "i4", "<", 0), ("b", "f8", "<", 8)]);
                w.out
            },
            b"\x89",
            &blob(&[0; 32]),
            &blob(mask),
            b"Nt\x94b.",
        ]))
    };
    let bytes = whole(b"\x00\x01\x00\x00");
    let found = recognise(&bytes).unwrap_or_else(|| panic!("read as far as {:#x}", furthest(&bytes)));
    let Kind::Masked { data, mask, .. } = &found.value.kind else { panic!("a masked array expected") };
    assert!(matches!(&data.kind, Kind::Array { dtype: Dtype::Record { width: 16, .. }, .. }));
    let Kind::Array { dtype: Dtype::Record { columns, width }, .. } = &mask.kind else { panic!("a record mask expected") };
    assert_eq!(*width, 2);
    assert_eq!(
        columns,
        &vec![
            Column { name: "a".into(), dtype: "|b1".into(), at: 0 },
            Column { name: "b".into(), dtype: "|b1".into(), at: 1 },
        ]
    );
    // One byte an entry is what a mask over plain numbers is, and it is the
    // wrong length here.
    assert!(recognise(&whole(b"\x00\x01")).is_none(), "a mask of one byte a row");
    assert!(recognise(&whole(b"\x00\x01\x00\x00\x00")).is_none(), "a mask longer than its columns");
}
