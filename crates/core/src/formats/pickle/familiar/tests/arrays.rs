//! The NumPy productions, against the fixture and against pickles written
//! here: dimensions, dtypes and byte order, the protocol 5 buffer call, the
//! scalar, what a second array may name out of the memo, and what a changed
//! instruction does to a match.

use super::*;

#[test]
fn captures_numpy_payload_and_rejects_changed_structure() {
    let found = recognise(MATRIX).unwrap();
    assert_eq!(found.form, "numpy-array-p4-p5-v6");
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
    assert_eq!(found.form, "numpy-array-p4-p5-v6");
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
    assert_eq!(found.form, "numpy-array-p4-p5-v6");
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
