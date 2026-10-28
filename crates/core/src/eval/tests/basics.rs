use super::*;

#[test]
fn spans_cover_a_stretch_without_a_call_per_field() {
    // A header, a run of numbers too long to list, and a window with room
    // left over at the end of it.
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("tag", T::u16(Big)),
                ("codes", T::array(T::u8(), E::lit(12))),
                ("window", T::sized(E::lit(4), T::structure("Inner", vec![("a", T::u16(Big))]))),
            ],
        ),
    );
    let d = doc(&[0xab, 0xcd, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 0, 7, 0, 0]);
    let mut ev = Evaluator::new(t);

    let all = ev.spans(&d, 0, 18 * 8, 100).unwrap();
    assert_eq!(all.len(), 4);
    assert_eq!((all[0].name.as_str(), all[0].size_bits), ("tag", 16));
    assert!(all[0].trail.is_empty());

    // Twelve numbers as one entry, saying how many it stands for.
    assert_eq!(all[1].name, "codes");
    assert_eq!(all[1].count, 12);
    assert_eq!(all[1].size_bits, 12 * 8);

    assert_eq!(all[2].name, "a");
    assert_eq!(all[2].trail, vec!["window"]);
    assert_eq!(all[2].value, Value::UInt(7));

    // The two bytes the window leaves over are a gap, not a field.
    assert!(all[3].gap);
    assert_eq!(all[3].offset_bits, 16 * 8);
    assert_eq!(all[3].size_bits, 2 * 8);

    // Asking for part of the file starts at the field covering that bit,
    // whether or not the field starts there.
    let part = ev.spans(&d, 5 * 8, 8 * 8, 100).unwrap();
    assert_eq!(part.len(), 1);
    assert_eq!(part[0].name, "codes");
    assert_eq!(part[0].offset_bits, 2 * 8);

    // The same for a gap: asked from its second byte, it is still the whole
    // gap, not the part of it below the view.
    let mid = ev.spans(&d, 17 * 8, 18 * 8, 100).unwrap();
    assert_eq!(mid.len(), 1);
    assert!(mid[0].gap);
    assert_eq!((mid[0].offset_bits, mid[0].size_bits), (16 * 8, 2 * 8));

    // A shorter run stays one entry per field.
    let t2 = Template::new("t", T::array(T::u8(), E::lit(4)));
    let mut ev2 = Evaluator::new(t2);
    let each = ev2.spans(&d, 0, 4 * 8, 100).unwrap();
    assert_eq!(each.len(), 4);
    assert_eq!(each[3].name, "[3]");

    // The count is a limit, not a target.
    assert_eq!(ev2.spans(&d, 0, 4 * 8, 2).unwrap().len(), 2);
}

#[test]
fn listing_summarises_only_large_runs_of_records() {
    let record = || T::structure("Record", vec![("value", T::u8())]);
    let bytes: Vec<u8> = (0..40).collect();
    let d = doc(&bytes);

    // A large record array is a logical section first. Its exact total and a
    // bounded sample of element extents are enough to decide whether to open
    // it; resolving all forty internal fields into rows is not.
    let mut large = Evaluator::new(Template::new("t", T::array(record(), E::lit(40))));
    let spans = large.spans(&d, 0, 40 * 8, 100).unwrap();
    assert_eq!(spans.len(), 1);
    assert_eq!((spans[0].count, spans[0].size_bits), (40, 40 * 8));
    assert_eq!(spans[0].parts.len(), 5);
    assert_eq!(spans[0].parts[0].size_bits, 8);
    assert!(spans[0].parts[4].rest);
    assert_eq!(spans[0].parts[4].size_bits, 36 * 8);

    // Short record arrays stay expanded; WAV chunks and other small repeated
    // structures should not turn into summaries merely because they repeat.
    let mut small = Evaluator::new(Template::new("t", T::array(record(), E::lit(12))));
    let spans = small.spans(&d, 0, 12 * 8, 100).unwrap();
    assert_eq!(spans.len(), 12);
    assert!(spans.iter().all(|span| span.count == 0));
}

#[test]
fn struct_with_count_driven_array() {
    let t = Template::new(
        "t",
        T::structure("Root", vec![("n", T::u8()), ("items", T::array(T::u16(Little), E::field("n")))]),
    );
    let d = doc(&[3, 1, 0, 2, 0, 3, 0, 99]);
    let mut ev = Evaluator::new(t);
    let root = ev.node(&d, &[]).unwrap();
    assert_eq!(root.size_bits, 7 * 8);
    assert_eq!(root.child_count, 2);
    let items = ev.node(&d, &[1]).unwrap();
    assert_eq!(items.child_count, 3);
    assert_eq!(items.offset_bits, 8);
    let third = ev.node(&d, &[1, 2]).unwrap();
    assert_eq!(third.value, Value::UInt(3));
    assert_eq!(third.offset_bits, 5 * 8);
}

#[test]
fn repeat_until_end_and_leb128() {
    // Records: leb128 length, then bytes. Three records.
    let t = Template::new(
        "t",
        T::repeat(
            T::structure("Rec", vec![("len", T::leb_u()), ("data", T::bytes(E::field("len")))]),
            Until::End,
        ),
    );
    let mut bytes = vec![2, 0xAA, 0xBB, 0, 0x80, 0x01];
    bytes.extend(std::iter::repeat_n(7u8, 128));
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);
    let root = ev.node(&d, &[]).unwrap();
    assert_eq!(root.child_count, 3);
    assert_eq!(root.size_bits, bytes.len() as u64 * 8);
    let third_len = ev.node(&d, &[2, 0]).unwrap();
    assert_eq!(third_len.value, Value::UInt(128));
    assert_eq!(third_len.size_bits, 16);
}

#[test]
fn sized_switch_and_pending() {
    use crate::source::ChunkStore;
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("kind", T::u8()),
                ("size", T::u8()),
                (
                    "body",
                    T::sized(
                        E::field("size"),
                        T::switch(E::field("kind"), vec![(1, T::u32(Big))], T::bytes(E::field("size"))),
                    ),
                ),
            ],
        ),
    );
    let mut d = Document::new(ChunkStore::new(6, 4, 8));
    let mut ev = Evaluator::new(t.clone());
    assert!(matches!(ev.node(&d, &[]), Err(EvalError::Pending(_))));
    d.source_mut().insert(0, vec![1, 4, 0, 0].into_boxed_slice());
    assert!(matches!(ev.node(&d, &[2]), Err(EvalError::Pending(_))));
    d.source_mut().insert(1, vec![1, 2].into_boxed_slice());
    let body = ev.node(&d, &[2]).unwrap();
    assert_eq!(body.type_name, "u32 be");
    assert_eq!(body.value, Value::UInt(0x0102));
    assert_eq!(body.size_bits, 32);
    // A size that overruns the file is an error, not a zero.
    let d2 = doc(&[9, 40, 0]);
    let mut ev2 = Evaluator::new(t);
    assert!(matches!(ev2.node(&d2, &[2]), Err(EvalError::Failed(_))));
}

#[test]
fn a_length_too_large_to_count_in_bits_fails_rather_than_wrapping() {
    // A u64 read from a corrupt file, then a byte. Each length is past what
    // bits can count: the first two once multiplied by eight, the third only
    // once added to where the field starts.
    let failed = |ty: T, path: &[usize], n: u64| {
        let t = Template::new("t", T::structure("Root", vec![("n", T::u64(Little)), ("body", ty)]));
        let mut bytes = n.to_le_bytes().to_vec();
        bytes.push(0);
        failure(Evaluator::new(t).node(&doc(&bytes), path).unwrap_err())
    };
    for n in [0x7fff_ffff_ffff_ffff, 0xffff_ffff_ffff_ffff, 0x1fff_ffff_ffff_ffff] {
        let sized = T::sized(E::field("n"), T::u8());
        assert_eq!(failed(sized, &[1], n), format!("field of {n} bytes extends beyond its parent"));
        let sized_bits = T::SizedBits { bits: E::field("n").mul(E::lit(8)), inner: Box::new(T::u8()) };
        assert_eq!(failed(sized_bits, &[1], n), format!("{} bits run past the end of the container", n as i128 * 8));
        assert_eq!(failed(T::bytes(E::field("n")), &[1], n), "field extends beyond its parent");
        assert_eq!(failed(T::at(E::field("n"), T::u8()), &[1, 0], n), "field extends beyond its parent");
        // A list of them, placed by stride when eight times the size fits and
        // by walking the first element when it does not.
        let each = T::sized(E::field("n"), T::u8());
        assert!(failed(T::array(each, E::lit(2)), &[1, 1], n).ends_with("extends beyond its parent"));
        // And a count that many elements long.
        assert_eq!(failed(T::array(T::u16(Little), E::field("n")), &[1], n), "field extends beyond its parent");
    }
}

#[test]
fn a_count_too_large_for_a_u64_fails_rather_than_wrapping() {
    // The largest u64, and one more: as a u64 that count wraps round to an
    // empty list, which is a corrupt file read as a well-formed one.
    let t = Template::new("t", T::structure("Root", vec![("n", T::u64(Little)), ("body", T::array(T::u8(), E::field("n").add(E::lit(1))))]));
    let got = Evaluator::new(t).node(&doc(&u64::MAX.to_le_bytes()), &[1]).map(|n| n.child_count).map_err(failure);
    assert_eq!(got, Err("count 18446744073709551616 does not fit in a u64".into()));
}

#[test]
fn huge_variable_size_array_does_not_recurse() {
    // 50k LEB128 elements; the count itself is a 3-byte LEB128.
    let n = 50_000u32;
    let mut bytes = vec![(n & 0x7f) as u8 | 0x80, ((n >> 7) & 0x7f) as u8 | 0x80, (n >> 14) as u8];
    for i in 0..n {
        let v = i % 300;
        if v < 128 {
            bytes.push(v as u8);
        } else {
            bytes.push((v & 0x7f) as u8 | 0x80);
            bytes.push((v >> 7) as u8);
        }
    }
    let t = Template::new(
        "t",
        T::structure("Root", vec![("n", T::leb_u()), ("xs", T::array(T::leb_u(), E::field("n")))]),
    );
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);
    // Size of the array first, before any element is resolved.
    let xs = ev.node(&d, &[1]).unwrap();
    assert_eq!(xs.child_count, 50_000);
    assert_eq!(xs.size_bits, (bytes.len() as u64 - 3) * 8);
    assert_eq!(ev.node(&d, &[1, 49_999]).unwrap().value, Value::UInt(49_999 % 300));
    // Fresh evaluator, jump straight to the last element.
    let mut ev2 = Evaluator::new(ev.template().clone());
    assert_eq!(ev2.node(&d, &[1, 49_999]).unwrap().value, Value::UInt(49_999 % 300));
}

#[test]
fn bitfields_read_msb_first() {
    let t = Template::new(
        "t",
        T::structure("B", vec![("a", T::UInt { bits: 3, endian: Big }), ("b", T::UInt { bits: 5, endian: Big })]),
    );
    let d = doc(&[0b101_01100]);
    let mut ev = Evaluator::new(t);
    assert_eq!(ev.node(&d, &[0]).unwrap().value, Value::UInt(0b101));
    assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::UInt(0b01100));
    assert_eq!(ev.node(&d, &[1]).unwrap().offset_bits, 3);
}

#[test]
fn writing_a_field_hits_only_its_own_bits() {
    let t = Template::new(
        "t",
        T::structure(
            "B",
            vec![
                ("a", T::UInt { bits: 3, endian: Big }),
                ("b", T::UInt { bits: 5, endian: Big }),
                ("n", T::u16(Little)),
                ("tag", T::utf8(E::lit(4))),
            ],
        ),
    );
    let mut d = doc(&[0b101_01100, 0x34, 0x12, b'I', b'H', b'D', b'R']);
    let mut ev = Evaluator::new(t);
    assert!(ev.node(&d, &[0]).unwrap().editable);
    assert!(!ev.node(&d, &[]).unwrap().editable);

    for (path, text) in [(vec![1], "31"), (vec![2], "0xbeef"), (vec![3], "iend")] {
        let w = ev.prepare_write(&d, &path, text).unwrap();
        d.overwrite_bits(w.offset_bits, &w.data, w.n_bits);
        ev.invalidate();
    }
    assert_eq!(ev.node(&d, &[0]).unwrap().value, Value::UInt(0b101));
    assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::UInt(31));
    assert_eq!(ev.node(&d, &[2]).unwrap().value, Value::UInt(0xbeef));
    assert_eq!(ev.node(&d, &[3]).unwrap().value, Value::Str("iend".into()));

    let mut out = [0u8; 7];
    d.read_bytes(0, &mut out);
    assert_eq!(out, [0b101_11111, 0xef, 0xbe, b'i', b'e', b'n', b'd']);

    // Rejections carry a reason and leave the document alone.
    assert!(matches!(ev.prepare_write(&d, &[1], "32"), Err(EvalError::Failed(_))));
    assert!(matches!(ev.prepare_write(&d, &[3], "toolong"), Err(EvalError::Failed(_))));
    assert!(matches!(ev.prepare_write(&d, &[], "1"), Err(EvalError::Failed(_))));
}

#[test]
fn locate_finds_the_field_under_a_bit() {
    let t = Template::new(
        "t",
        T::structure(
            "B",
            vec![
                ("a", T::UInt { bits: 3, endian: Big }),
                ("b", T::UInt { bits: 5, endian: Big }),
                ("items", T::array(T::u16(Big), E::lit(3))),
            ],
        ),
    );
    let d = doc(&[0b101_01100, 0, 1, 0, 2, 0, 3]);
    let mut ev = Evaluator::new(t);
    assert_eq!(ev.locate(&d, 0).unwrap(), vec![0]);
    assert_eq!(ev.locate(&d, 2).unwrap(), vec![0]);
    assert_eq!(ev.locate(&d, 3).unwrap(), vec![1]);
    assert_eq!(ev.locate(&d, 7).unwrap(), vec![1]);
    // Into the array: element 1 starts at byte 3.
    assert_eq!(ev.locate(&d, 8).unwrap(), vec![2, 0]);
    assert_eq!(ev.locate(&d, 3 * 8 + 4).unwrap(), vec![2, 1]);
    assert_eq!(ev.locate(&d, 6 * 8).unwrap(), vec![2, 2]);
    assert!(ev.locate(&d, 7 * 8).is_err());
}

#[test]
fn text_is_read_and_written_in_its_own_encoding() {
    use crate::template::{Encoding, StrLen};
    let t = Template::new(
        "t",
        T::structure(
            "R",
            vec![
                ("dos", T::text(StrLen::Padded { size: E::lit(8), pad: 0 }, Encoding::Cp437)),
                ("wide", T::text(StrLen::Padded { size: E::lit(10), pad: 0 }, Encoding::Bom { fallback: Box::new(Encoding::Latin1) })),
            ],
        ),
    );
    // CP437 0xE1 is the sharp s; the rest of the field is padding.
    let mut bytes = vec![b'D', b'O', b'S', 0xe1, 0, 0, 0, 0];
    // UTF-16 LE with a byte-order mark: "Hi", then NUL units.
    bytes.extend_from_slice(&[0xff, 0xfe, b'H', 0, b'i', 0, 0, 0, 0, 0]);
    let mut d = doc(&bytes);
    let mut ev = Evaluator::new(t);

    let dos = ev.node(&d, &[0]).unwrap();
    assert_eq!(dos.value, Value::Str("DOS\u{00df}".into()));
    assert_eq!(dos.value_bytes, 4);
    assert_eq!(dos.type_name, "cp437 nul-pad");

    let wide = ev.node(&d, &[1]).unwrap();
    assert_eq!(wide.value, Value::Str("Hi".into()));
    // The mark is part of the field, not of the value.
    assert_eq!(wide.value_offset_bits, wide.offset_bits + 16);
    assert_eq!(wide.value_bytes, 4);
    assert_eq!(wide.read_as.as_deref(), Some("Read as UTF-16 LE, from a byte-order mark"));

    // Writing keeps the encoding and the mark, and pads in whole units.
    let w = ev.prepare_write(&d, &[1], "Sun").unwrap();
    assert_eq!(w.data, vec![0xff, 0xfe, b'S', 0, b'u', 0, b'n', 0, 0, 0]);
    d.overwrite_bits(w.offset_bits, &w.data, w.n_bits);
    ev.invalidate();
    assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::Str("Sun".into()));

    // A character CP437 does not have is refused, not mangled.
    assert!(matches!(ev.prepare_write(&d, &[0], "\u{20ac}"), Err(EvalError::Failed(_))));
    let w = ev.prepare_write(&d, &[0], "\u{00df}\u{00df}").unwrap();
    assert_eq!(w.data, vec![0xe1, 0xe1, 0, 0, 0, 0, 0, 0]);
}

#[test]
fn an_enum_is_written_by_name() {
    let t = Template::new(
        "t",
        T::structure("R", vec![("kind", T::enumeration("Kind", T::u8(), &[(1, "one"), (2, "two")]))]),
    );
    let d = doc(&[1]);
    let mut ev = Evaluator::new(t);
    assert_eq!(ev.prepare_write(&d, &[0], "two").unwrap().data, vec![2]);
    assert_eq!(ev.prepare_write(&d, &[0], "9").unwrap().data, vec![9]);
    assert!(matches!(ev.prepare_write(&d, &[0], "three"), Err(EvalError::Failed(_))));
}

#[test]
fn a_value_past_the_named_ones_is_named_by_the_run_it_falls_in() {
    let t = Template::new(
        "t",
        T::structure(
            "R",
            vec![(
                "kind",
                T::enum_ranged("Kind", T::u8(), &[(0, "nothing")], &[(12, 2, "blob, {n} bytes"), (13, 2, "text, {n} bytes")]),
            )],
        ),
    );
    let named = |b: u8| {
        let mut ev = Evaluator::new(t.clone());
        match ev.node(&doc(&[b]), &[0]).unwrap().value {
            Value::Enum { name, .. } => name,
            other => panic!("not an enum: {other:?}"),
        }
    };
    assert_eq!(named(0), Some("nothing".into()));
    assert_eq!(named(12), Some("blob, 0 bytes".into()));
    assert_eq!(named(13), Some("text, 0 bytes".into()));
    assert_eq!(named(17), Some("text, 2 bytes".into()));
    assert_eq!(named(30), Some("blob, 9 bytes".into()));
    // Below where the runs start and above where the names stop: still a
    // value, still shown, and nobody pretends to know what it is called.
    assert_eq!(named(7), None);
}

#[test]
fn remaining_measures_to_the_end_of_the_container() {
    use crate::template::{Encoding, StrLen};
    let t = Template::new(
        "t",
        T::structure(
            "R",
            vec![
                ("n", T::u8()),
                ("head", T::bytes(E::field("n"))),
                ("rest", T::bytes(E::Remaining)),
            ],
        ),
    );
    let d = doc(&[2, 0xaa, 0xbb, 1, 2, 3]);
    let mut ev = Evaluator::new(t);
    assert_eq!(ev.node(&d, &[2]).unwrap().size_bits, 3 * 8);

    // Inside a Sized window it stops at the window, not at the file.
    let t2 = Template::new(
        "t",
        T::structure(
            "R",
            vec![
                ("win", T::sized(E::lit(3), T::structure("W", vec![("a", T::u8()), ("b", T::bytes(E::Remaining))]))),
                ("after", T::u8()),
            ],
        ),
    );
    let mut ev2 = Evaluator::new(t2);
    assert_eq!(ev2.node(&d, &[0, 1]).unwrap().size_bits, 2 * 8);
    assert_eq!(ev2.node(&d, &[1]).unwrap().offset_bits, 3 * 8);

    // A repeat whose element takes the rest has exactly one element.
    let t3 = Template::new("t", T::repeat(T::sized(E::Remaining, T::bytes(E::Remaining)), Until::End));
    let mut ev3 = Evaluator::new(t3);
    assert_eq!(ev3.node(&d, &[]).unwrap().child_count, 1);
    let _ = StrLen::Fixed(E::lit(0));
    let _ = Encoding::Utf8;
}

#[test]
fn a_last_line_without_a_terminator_still_reads() {
    use crate::template::{Encoding, StrLen};
    let line = T::text(StrLen::Terminated { end: b'\n', or_end: true }, Encoding::Utf8);
    let t = Template::new("t", T::repeat(line, Until::End));
    let d = doc(b"one\ntwo");
    let mut ev = Evaluator::new(t);
    assert_eq!(ev.node(&d, &[]).unwrap().child_count, 2);
    assert_eq!(ev.node(&d, &[0]).unwrap().value, Value::Str("one".into()));
    assert_eq!(ev.node(&d, &[0]).unwrap().size_bits, 4 * 8);
    let last = ev.node(&d, &[1]).unwrap();
    assert_eq!(last.value, Value::Str("two".into()));
    assert_eq!(last.size_bits, 3 * 8);
    // Nothing to write the terminator back into, so the tail is read-only.
    assert!(!last.editable);
    assert!(ev.node(&d, &[0]).unwrap().editable);

    // Without `or_end` the same bytes are an error, not a guess.
    let strict = T::text(StrLen::Terminated { end: b'\n', or_end: false }, Encoding::Utf8);
    let mut ev2 = Evaluator::new(Template::new("t", T::repeat(strict, Until::End)));
    assert!(ev2.node(&d, &[1]).is_err());
}

/// A run of fields that are each there or not is a run of the type they are
/// instances of, and the type column and the listing both have to say so. The
/// switch a `present_if` is written as never gets resolved here: one is picked
/// per element, and the list itself has to be named before any of them is
/// read. See `Ty::agreed_case`.
#[test]
fn a_list_of_optional_fields_keeps_its_element_type_and_unit() {
    let filter = T::structure("Filter", vec![("id", T::u8()), ("size", T::u8())]).counted_as("filter");
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![("count", T::u8()), ("filters", T::array(T::present_if(E::lit(1), filter), E::field("count")))],
        ),
    );
    let d = doc(&[2, 0x21, 1, 0x03, 1]);
    let mut ev = Evaluator::new(t);
    let list = ev.node(&d, &[1]).unwrap();
    assert_eq!(list.type_name, "Filter[]");
    assert_eq!(list.unit.as_deref(), Some("filter"));
}

/// And the other way: a switch standing for shapes that have nothing to do
/// with one another has no name of its own, and inventing one out of whichever
/// case is written first would name the list after a shape most of its
/// elements are not.
#[test]
fn a_list_of_unlike_shapes_is_still_a_switch() {
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("n", T::u8()),
                ("types", T::array(T::u8(), E::field("n"))),
                (
                    "vals",
                    T::array(
                        T::switch(E::elem("types", E::idx()), vec![(1, T::u8()), (2, T::u16(Big))], T::bytes(E::lit(0))),
                        E::field("n"),
                    ),
                ),
            ],
        ),
    );
    let d = doc(&[2, 2, 1, 0, 5, 7]);
    let mut ev = Evaluator::new(t);
    assert_eq!(ev.node(&d, &[2]).unwrap().type_name, "switch[]");
}

#[test]
fn sqlite_varints_read_and_write_at_their_own_size() {
    let t = Template::new(
        "t",
        T::structure("Root", vec![("a", T::sqlite_varint()), ("b", T::sqlite_varint())]),
    );
    // 128 in two bytes, then -1 in the nine-byte form.
    let d = doc(&[0x81, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
    let mut ev = Evaluator::new(t);
    let a = ev.node(&d, &[0]).unwrap();
    assert_eq!((a.value.clone(), a.size_bits), (Value::Int(128), 16));
    let b = ev.node(&d, &[1]).unwrap();
    assert_eq!((b.value.clone(), b.size_bits), (Value::Int(-1), 72));
    // Writing keeps the size: 3 pads out to two bytes, -2 to nine.
    let w = ev.prepare_write(&d, &[0], "3").unwrap();
    assert_eq!((w.data, w.n_bits), (vec![0x80, 0x03], 16));
    let w = ev.prepare_write(&d, &[1], "-2").unwrap();
    assert_eq!(w.n_bits, 72);
    let d2 = doc(&w.data);
    let mut ev2 = Evaluator::new(Template::new("t", T::structure("R", vec![("v", T::sqlite_varint())])));
    assert_eq!(ev2.node(&d2, &[0]).unwrap().value, Value::Int(-2));
}

#[test]
fn a_fixed_stride_array_is_sized_without_touching_its_elements() {
    // A count and then that many u16s. The count says two hundred thousand;
    // the file holds them all, and sizing the array must not resolve them.
    let t = Template::new(
        "t",
        T::structure("Root", vec![("n", T::u32(Little)), ("samples", T::array(T::u16(Little), E::field("n")))]),
    );
    let n: u32 = 200_000;
    let mut bytes = n.to_le_bytes().to_vec();
    bytes.resize(4 + n as usize * 2, 0);
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);
    let arr = ev.node(&d, &[1]).unwrap();
    assert_eq!(arr.size_bits, n as u64 * 16);
    assert_eq!(arr.child_count, n as u64);
    // Sizing memoised the array and its parent, not two hundred thousand rows.
    assert!(ev.memo.len() < 10, "sized by arithmetic, not by a walk: {} entries", ev.memo.len());
    // An element in the middle is still one lookup.
    assert_eq!(ev.node(&d, &[1, 150_000]).unwrap().offset_bits, (4 + 300_000) * 8);
}

#[test]
fn an_array_of_a_named_record_is_sized_without_touching_its_elements() {
    // The same run, each element a record of two u16s written as the name of
    // its type, which is how an Arrow batch lists its nodes and how a Kaitai
    // format lists anything it declared under `types`. Resolving a name is
    // resolving the type it stands for, so the stride is the same as the
    // record written out in place.
    let t = Template::new(
        "t",
        T::structure("Root", vec![("n", T::u32(Little)), ("pairs", T::array(T::Named("Pair".into()), E::field("n")))]),
    )
    .with_type("Pair", T::structure("Pair", vec![("a", T::u16(Little)), ("b", T::u16(Little))]));
    let n: u32 = 100_000;
    let mut bytes = n.to_le_bytes().to_vec();
    bytes.resize(4 + n as usize * 4, 0);
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);
    // A go of a thousand elements. A long walk keeps few nodes, so what shows
    // the walk is the go running out before the list is sized.
    ev.set_slice(Some(1_000));
    ev.begin_slice();
    let arr = ev.node(&d, &[1]).unwrap();
    assert_eq!((arr.size_bits, arr.child_count), (n as u64 * 32, n as u64));
    assert_eq!(ev.node(&d, &[1, 75_000]).unwrap().offset_bits, (4 + 300_000) * 8);
    // A name with no type behind it is no stride, and the walk says what is wrong.
    let t = Template::new("t", T::structure("Root", vec![("pairs", T::array(T::Named("Missing".into()), E::lit(2)))]));
    let mut ev = Evaluator::new(t);
    assert!(ev.node(&d, &[0]).is_err());
}

#[test]
fn a_narrow_float_reads_at_the_width_it_was_stored_in() {
    // The same number in sixteen bits and in thirty-two. Widening either to an
    // f64 and printing that gives a dozen digits the file never held.
    let t = Template::new(
        "t",
        T::structure("Root", vec![("half", T::F16(Little)), ("single", T::F32(Little))]),
    );
    let mut bytes = 0x1bedu16.to_le_bytes().to_vec(); // f16 0.00387
    bytes.extend_from_slice(&0.3f32.to_le_bytes());
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);
    assert_eq!(ev.node(&d, &[0]).unwrap().value, Value::Float(0.00387));
    assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::Float(0.3));
    // Shorter, and still the same bits: writing it back changes nothing.
    let w = ev.prepare_write(&d, &[0], "0.00387").unwrap();
    assert_eq!(w.data, 0x1bedu16.to_le_bytes().to_vec());
}

#[test]
fn a_brain_float_reads_as_the_float_it_is_the_top_half_of() {
    // The same sixteen bits read as both sixteen-bit floats. 0x3f80 is 1.0 as
    // a brain float and 1.875 as a half float: same bits, different meaning,
    // which is why the two are separate types rather than one width.
    let t = Template::new(
        "t",
        T::structure("Root", vec![("brain", T::BF16(Little)), ("half", T::F16(Little))]),
    );
    let mut bytes = 0x3f80u16.to_le_bytes().to_vec();
    bytes.extend_from_slice(&0x3f80u16.to_le_bytes());
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);
    let brain = ev.node(&d, &[0]).unwrap();
    assert_eq!((brain.value.clone(), brain.type_name.as_str()), (Value::Float(1.0), "bf16 le"));
    assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::Float(1.875));

    // Three digits is what eight bits of significand hold, and writing those
    // three digits back gives the same sixteen bits.
    let mut bytes = 0x3e59u16.to_le_bytes().to_vec(); // 0.212
    bytes.extend_from_slice(&[0, 0]);
    let d = doc(&bytes);
    let mut ev = Evaluator::new(Template::new("t", T::structure("Root", vec![("v", T::BF16(Little))])));
    assert_eq!(ev.node(&d, &[0]).unwrap().value, Value::Float(0.212));
    let w = ev.prepare_write(&d, &[0], "0.212").unwrap();
    assert_eq!(w.data, 0x3e59u16.to_le_bytes().to_vec());
    // A number too fine for the type lands on the nearest one it has.
    let w = ev.prepare_write(&d, &[0], "0.2121").unwrap();
    assert_eq!(w.data, 0x3e59u16.to_le_bytes().to_vec());
}
