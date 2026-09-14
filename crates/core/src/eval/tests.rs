use super::*;
use crate::source::MemSource;
use crate::template::{Anchor, Endian::*, Expr as E, Step, Ty as T};

fn doc(bytes: &[u8]) -> Document<MemSource> {
    Document::new(MemSource(bytes.to_vec()))
}

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
        assert_eq!(failed(sized, &[1], n), format!("size {n} runs past the end of its container"));
        let sized_bits = T::SizedBits { bits: E::field("n").mul(E::lit(8)), inner: Box::new(T::u8()) };
        assert_eq!(failed(sized_bits, &[1], n), format!("{} bits run past the end of the container", n as i128 * 8));
        assert_eq!(failed(T::bytes(E::field("n")), &[1], n), "runs past the end of its container");
        assert_eq!(failed(T::at(E::field("n"), T::u8()), &[1, 0], n), "runs past the end of the file");
        // A list of them, placed by stride when eight times the size fits and
        // by walking the first element when it does not.
        let each = T::sized(E::field("n"), T::u8());
        assert!(failed(T::array(each, E::lit(2)), &[1, 1], n).ends_with("runs past the end of its container"));
        // And a count that many elements long.
        assert_eq!(failed(T::array(T::u16(Little), E::field("n")), &[1], n), "runs past the end of its container");
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

/// A template whose items sit at offsets held in an earlier array, in the
/// order the offsets are in rather than the order they sit in.
fn pointer_template() -> Template {
    let item = T::structure("Item", vec![("len", T::u8()), ("text", T::utf8(E::field("len")))]);
    Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("count", T::u8()),
                ("ptrs", T::array(T::u16(Big), E::field("count"))),
                ("items", T::pointer_list("ptrs", Anchor::Window, E::lit(0), item)),
            ],
        ),
    )
}

// count, two offsets, a byte belonging to nothing, then the two items with
// the later one pointed at first.
const POINTED: &[u8] = &[2, 0, 10, 0, 6, 0xff, 3, b'b', b'e', b'e', 2, b'o', b'k'];

#[test]
fn pointed_at_items_read_in_offset_order() {
    let d = doc(POINTED);
    let mut ev = Evaluator::new(pointer_template());
    assert_eq!(ev.node(&d, &[2]).unwrap().child_count, 2);
    assert_eq!(ev.node(&d, &[2, 0]).unwrap().offset_bits, 10 * 8);
    assert_eq!(ev.node(&d, &[2, 0, 1]).unwrap().value, Value::Str("ok".into()));
    assert_eq!(ev.node(&d, &[2, 1, 1]).unwrap().value, Value::Str("bee".into()));
    // The cursor finds the item that covers a byte, wherever it is in the list.
    assert_eq!(ev.locate(&d, 7 * 8).unwrap(), vec![2, 1, 1]);
    assert_eq!(ev.locate(&d, 11 * 8).unwrap(), vec![2, 0, 1]);
}

#[test]
fn a_field_can_read_its_contents_somewhere_else_and_still_cost_nothing() {
    // A header naming a table at the end, and a run of bytes in between that
    // is placed as if the table field were not there.
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("table_at", T::u8()),
                ("table", T::at(E::field("table_at"), T::array(T::u16(Big), E::lit(2)))),
                ("body", T::bytes(E::lit(3))),
            ],
        ),
    );
    let d = doc(&[4, 0xaa, 0xbb, 0xcc, 0, 1, 0, 2]);
    let mut ev = Evaluator::new(t);

    // The field is at the cursor and covers nothing.
    let table = ev.node(&d, &[1]).unwrap();
    assert_eq!(table.offset_bits, 8);
    assert_eq!(table.size_bits, 0);
    assert_eq!(table.child_count, 1);
    // What it points at is at the far offset.
    assert_eq!(ev.node(&d, &[1, 0]).unwrap().offset_bits, 4 * 8);
    assert_eq!(ev.node(&d, &[1, 0, 1]).unwrap().value, Value::UInt(2));
    // And the field after it is placed as if it were not there.
    let body = ev.node(&d, &[2]).unwrap();
    assert_eq!(body.offset_bits, 8);
    assert_eq!(body.size_bits, 3 * 8);

    // Naming it in an expression means the table, not the nothing standing in
    // its place: two elements, and the second of them.
    let t2 = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("table_at", T::u8()),
                ("table", T::at(E::field("table_at"), T::array(T::u16(Big), E::lit(2)))),
                ("n", T::computed(E::field("table"))),
                ("second", T::computed(E::elem("table", E::lit(1)))),
            ],
        ),
    );
    let mut ev2 = Evaluator::new(t2);
    assert_eq!(ev2.node(&d, &[2]).unwrap().value, Value::Int(2));
    assert_eq!(ev2.node(&d, &[3]).unwrap().value, Value::Int(2));

    // The cursor reaches it. A structure is still as long as its last field
    // ends, and the table is past that, but where a placed field put its
    // contents is indexed, so a byte inside the table is the table: see
    // `placed.rs`, and the HDF5 template, which is nothing but this.
    assert_eq!(ev2.locate(&d, 6 * 8).unwrap(), vec![1, 0, 1]);
}

/// A list whose elements point somewhere is walked to the end of it, however
/// long it is.
///
/// The index of placed stretches gives up on a list once its elements stop
/// adding anything, which is what saves it from walking a column of a hundred
/// thousand strings that all name the same heap. Whether an element added
/// anything is a question about the whole of it, though, and hardly ever about
/// the element itself: a Parquet column chunk points at its pages from four
/// levels inside it, and a file of sixty-six of them had the last two left out
/// of the index and reading as a gap.
#[test]
fn a_long_list_of_pointers_is_walked_to_the_end() {
    const N: usize = 70;
    const DATA: usize = 200;
    let t = Template::new(
        "t",
        T::array(
            T::structure("Record", vec![
                ("at", T::u16(Big)),
                ("data", T::at(E::field("at"), T::bytes(E::lit(4)))),
            ]),
            E::lit(N as i128),
        ),
    );
    let mut bytes = vec![0u8; DATA + N * 4];
    for i in 0..N {
        let at = (DATA + i * 4) as u16;
        bytes[i * 2..i * 2 + 2].copy_from_slice(&at.to_be_bytes());
    }
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);

    // The last element's stretch is past everything the root covers, so only
    // the index can place it.
    let last = ((DATA + (N - 1) * 4) * 8) as u64;
    assert_eq!(ev.locate(&d, last).unwrap(), vec![N - 1, 1, 0], "the last element's bytes read as a gap");
    let first = (DATA * 8) as u64;
    assert_eq!(ev.locate(&d, first).unwrap(), vec![0, 1, 0]);
}

#[test]
fn a_scanned_field_steps_over_its_separators_and_stops_at_the_next() {
    use crate::template::{Encoding, StrLen};
    let token = |comment| {
        T::text(StrLen::Scan { skip: b" \t\r\n".to_vec(), ends: b" \t\r\n".to_vec(), comment }, Encoding::Ascii)
    };
    let token = || token(None);
    let t = Template::new(
        "t",
        T::structure("Root", vec![("a", token()), ("b", token()), ("rest", T::bytes(E::Remaining))]),
    );
    let d = doc(b"  12\t\n 345\nxyz");
    let mut ev = Evaluator::new(t.clone());

    let a = ev.node(&d, &[0]).unwrap();
    assert_eq!(a.value, Value::Str("12".into()));
    // Two spaces, the digits, and the tab that ends them.
    assert_eq!(a.offset_bits, 0);
    assert_eq!(a.size_bits, 5 * 8);
    // The value starts past the separators, not at the field.
    assert_eq!(a.value_offset_bits, 2 * 8);
    assert_eq!(a.value_bytes, 2);
    // Whitespace before it is stepped over however much of it there is.
    let b = ev.node(&d, &[1]).unwrap();
    assert_eq!(b.value, Value::Str("345".into()));
    assert_eq!(b.offset_bits, 5 * 8);
    assert_eq!(b.size_bits, 6 * 8);
    assert_eq!(ev.node(&d, &[2]).unwrap().size_bits, 3 * 8);
    // Nothing to write back: how much whitespace to put where is the format's
    // business, and the field would change size.
    assert!(!a.editable);

    // A field with no separator after it is not a value, the same answer a
    // terminated field gives.
    let mut ev = Evaluator::new(t);
    assert!(ev.node(&doc(b"  12"), &[0]).is_err());
}

#[test]
fn a_scanned_field_steps_over_comments_among_its_separators() {
    use crate::template::{Encoding, StrLen};
    let token = || {
        T::text(
            StrLen::Scan { skip: b" \t\r\n".to_vec(), ends: b" \t\r\n".to_vec(), comment: Some((b'#', b'\n')) },
            Encoding::Ascii,
        )
    };
    let t = Template::new(
        "t",
        T::structure("Root", vec![("a", token()), ("b", token()), ("rest", T::bytes(E::Remaining))]),
    );

    let d = doc(b" # a note\n 12 #another\n34 x");
    let mut ev = Evaluator::new(t.clone());
    let a = ev.node(&d, &[0]).unwrap();
    assert_eq!(a.value, Value::Str("12".into()));
    // The space, the comment and the space after it all belong to the field,
    // and none of them to the value.
    assert_eq!(a.offset_bits, 0);
    assert_eq!(a.value_offset_bits, 11 * 8);
    assert_eq!(a.size_bits, 14 * 8);
    assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::Str("34".into()));
    assert_eq!(ev.node(&d, &[2]).unwrap().size_bits, 8);

    // A comment longer than the 256 bytes a scan reads at a time: the state
    // has to survive the join, or the field ends inside the comment.
    let mut bytes = b"#".to_vec();
    bytes.extend(std::iter::repeat_n(b'.', 600));
    bytes.extend_from_slice(b"\n7 ok");
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);
    let a = ev.node(&d, &[0]).unwrap();
    assert_eq!(a.value, Value::Str("7".into()));
    assert_eq!(a.value_offset_bits, 602 * 8);
}

#[test]
fn a_record_can_be_switched_on_a_byte_further_along_than_any_field() {
    // Two layouts of four bytes, told apart by the last of them, which comes
    // after the fields whose meaning it settles.
    let wide = T::structure("Wide", vec![("n", T::u16(Big)), ("pad", T::u8()), ("kind", T::u8())]);
    let narrow = T::structure(
        "Narrow",
        vec![("a", T::u8()), ("b", T::u8()), ("pad", T::u8()), ("kind", T::u8())],
    );
    let rec = T::switch(E::peek_at(E::lit(3 * 8), 8, Big), vec![(1, wide)], narrow);
    let t = Template::new("t", T::repeat(rec, Until::End));
    let d = doc(&[0x12, 0x34, 0, 1, 0x56, 0x78, 0, 2]);
    let mut ev = Evaluator::new(t);
    assert_eq!(ev.node(&d, &[0]).unwrap().type_name, "Wide");
    assert_eq!(ev.node(&d, &[0, 0]).unwrap().value, Value::UInt(0x1234));
    assert_eq!(ev.node(&d, &[1]).unwrap().type_name, "Narrow");
    assert_eq!(ev.node(&d, &[1, 0]).unwrap().value, Value::UInt(0x56));

    // Looking past the end of the container is an error, not a guess: the
    // same answer a peek at the field's own start gives.
    let short = doc(&[0x12, 0x34]);
    let mut ev = Evaluator::new(ev.template().clone());
    assert!(ev.node(&short, &[0]).is_err());
}

#[test]
fn a_peek_reads_the_way_round_it_is_told_to() {
    // The same two bytes, looked at both ways: a peek says which way round it
    // reads, the same as a field does, so a format that writes its numbers
    // little-endian can be switched on one.
    let layouts = |e| {
        T::structure(
            "Root",
            vec![
                ("kind", T::switch(E::peek(16, e), vec![(0x0102, T::u8())], T::u16(Big))),
                ("rest", T::bytes(E::Remaining)),
            ],
        )
    };
    let d = doc(&[0x02, 0x01, 0xff, 0xff]);
    // Little-endian, so those bytes read as 0x0102 and the case is taken.
    let mut ev = Evaluator::new(Template::new("t", layouts(Little)));
    assert_eq!(ev.node(&d, &[0]).unwrap().size_bits, 8);
    // Big-endian, so they read as 0x0201 and it is not.
    let mut ev = Evaluator::new(Template::new("t", layouts(Big)));
    assert_eq!(ev.node(&d, &[0]).unwrap().size_bits, 16);
    assert_eq!(ev.node(&d, &[0]).unwrap().value, Value::UInt(0x0201));

    // Bits narrower than a byte have no bytes to order, so what a peek says
    // there is which end of the byte to take them from, the same as a field
    // of them does. Big is the top of the byte, Little the bottom.
    let three = |e| Template::new("t", T::structure("Root", vec![("n", T::computed(E::peek(3, e)))]));
    let d = doc(&[0b101_00_110]);
    assert_eq!(Evaluator::new(three(Big)).node(&d, &[0]).unwrap().value, Value::Int(0b101));
    assert_eq!(Evaluator::new(three(Little)).node(&d, &[0]).unwrap().value, Value::Int(0b110));
}

#[test]
fn a_pointer_back_at_something_already_open_is_refused_rather_than_followed() {
    // A directory that says where the next one is, and a file where the next
    // one is itself. Without a guard this is not slow, it is endless: asking
    // what covers a byte would go round the ring forever.
    let dir = T::structure(
        "Dir",
        vec![("next", T::u8()), ("chain", T::at(E::field("next"), T::Named("Dir".into())))],
    );
    let t = || Template::new("t", T::Named("Dir".into())).with_type("Dir", dir.clone());

    // Byte 0 says the next directory is at 0, which is this one.
    let d = doc(&[0, 0, 0, 0]);
    let mut ev = Evaluator::new(t());
    assert!(ev.node(&d, &[1, 0]).is_err());
    // What it says about itself still reads; only the step back is refused.
    assert_eq!(ev.node(&d, &[0]).unwrap().value, Value::UInt(0));

    // And the cursor, which is what the ring would have trapped: asking what
    // covers a byte answers instead of going round.
    assert_eq!(ev.locate(&d, 0).unwrap(), vec![0]);

    // A ring of two is caught the same way, on the step that closes it.
    let d = doc(&[2, 0, 0, 0]);
    let mut ev = Evaluator::new(t());
    assert_eq!(ev.node(&d, &[1, 0, 0]).unwrap().value, Value::UInt(0));
    assert!(ev.node(&d, &[1, 0, 1, 0]).is_err());

    // A chain that goes somewhere new each time is not a ring and is read to
    // the end of it.
    let d = doc(&[1, 2, 3, 0]);
    let mut ev = Evaluator::new(t());
    assert_eq!(ev.node(&d, &[1, 0, 1, 0, 1, 0]).unwrap().offset_bits, 3 * 8);
    // Two pointers landing on the same place from different lines are two
    // pointers, not a ring: neither is above the other.
    let together = Template::new(
        "t",
        T::structure(
            "Two",
            vec![
                ("a", T::at(E::lit(3), T::u8())),
                ("b", T::at(E::lit(3), T::u8())),
                ("rest", T::bytes(E::Remaining)),
            ],
        ),
    );
    let mut ev = Evaluator::new(together);
    assert_eq!(ev.node(&d, &[0, 0]).unwrap().offset_bits, 3 * 8);
    assert_eq!(ev.node(&d, &[1, 0]).unwrap().offset_bits, 3 * 8);
}

#[test]
fn an_offset_can_count_from_the_copy_it_is_written_inside() {
    // A layout that names a table by where it is, written once and read both
    // as a file of its own and as a copy of that file inside something else.
    let format = || {
        T::structure(
            "Format",
            vec![
                ("where", T::u8()),
                ("table", T::at_in_window(E::field("where"), T::u16(Big))),
                ("body", T::bytes(E::Remaining)),
            ],
        )
    };

    // On its own, with no window anywhere, the offset counts from the start
    // of the file, which is where the format begins.
    let d = doc(&[3, 0, 0, 0xaa, 0xbb]);
    let mut ev = Evaluator::new(Template::new("t", format()));
    assert_eq!(ev.node(&d, &[1]).unwrap().size_bits, 0);
    let table = ev.node(&d, &[1, 0]).unwrap();
    assert_eq!(table.offset_bits, 3 * 8);
    assert_eq!(table.value, Value::UInt(0xaabb));

    // The same bytes with two in front of them, inside a window of their own.
    // The offset still means three from where the format starts, which is now
    // three from the window rather than three from the file.
    let embedded = Template::new(
        "t",
        T::structure(
            "Outer",
            vec![("skip", T::bytes(E::lit(2))), ("inner", T::sized(E::Remaining, format()))],
        ),
    );
    let d = doc(&[9, 9, 3, 0, 0, 0xaa, 0xbb]);
    let mut ev = Evaluator::new(embedded);
    let table = ev.node(&d, &[1, 1, 0]).unwrap();
    assert_eq!(table.offset_bits, 5 * 8);
    assert_eq!(table.value, Value::UInt(0xaabb));

    // An offset counted from the file rather than the window ignores the
    // window, which is what a format that means the file wants.
    let from_file = Template::new(
        "t",
        T::structure(
            "Outer",
            vec![
                ("skip", T::bytes(E::lit(2))),
                (
                    "inner",
                    T::sized(E::Remaining, T::structure("Format", vec![("table", T::at(E::lit(3), T::u16(Big)))])),
                ),
            ],
        ),
    );
    let mut ev = Evaluator::new(from_file);
    assert_eq!(ev.node(&d, &[1, 0, 0]).unwrap().offset_bits, 3 * 8);
}

#[test]
fn a_stream_with_no_length_runs_to_the_next_marker() {
    // What a JPEG scan needs: bits with no count anywhere, ending at the next
    // 0xff that is not followed by one of the bytes that make it data.
    let t = || {
        Template::new(
            "t",
            T::structure(
                "Root",
                vec![
                    ("stream", T::bytes(E::to_marker(0xff, &[0x00, 0xd0, 0xd1]))),
                    ("rest", T::bytes(E::Remaining)),
                ],
            ),
        )
    };
    let len = |bytes: &[u8]| Evaluator::new(t()).node(&doc(bytes), &[0]).unwrap().size_bits / 8;

    // Stops before the marker, so the marker belongs to what comes next.
    assert_eq!(len(&[1, 2, 3, 0xff, 0xda, 9]), 3);
    // An 0xff written as data is escaped by the byte after it, and so are the
    // restart markers, so neither ends the stream.
    assert_eq!(len(&[1, 0xff, 0x00, 2, 0xff, 0xd0, 3, 0xff, 0xd9]), 7);
    // A marker at the very front measures nothing at all.
    assert_eq!(len(&[0xff, 0xd9, 1, 2]), 0);
    // No marker anywhere: a file cut off mid-stream still places its bytes.
    assert_eq!(len(&[1, 2, 3, 4]), 4);
    // A lead byte with nothing after it is not a marker: nothing has said so.
    assert_eq!(len(&[1, 2, 0xff]), 3);

    // The blocks the search reads in are 4096 bytes, and a marker split
    // across that boundary is still one marker.
    let mut v = vec![7u8; 4095];
    v.extend_from_slice(&[0xff, 0xd9, 0, 0]);
    assert_eq!(len(&v), 4095);
    // The same split, but escaped, so the search carries on past the join.
    let mut v = vec![7u8; 4095];
    v.extend_from_slice(&[0xff, 0x00, 7, 0xff, 0xd9]);
    assert_eq!(len(&v), 4098);
}

#[test]
fn a_marker_can_be_more_than_one_byte() {
    // What an H.264 Annex B stream needs: a NAL unit runs to the next start
    // code, which is three bytes and not one, and the byte after it is the NAL
    // header rather than an escape, so there is nothing to tell apart.
    let t = || {
        Template::new(
            "t",
            T::structure(
                "Root",
                vec![
                    ("nal", T::bytes(E::to_marker_seq(&[0, 0, 1], &[]))),
                    ("rest", T::bytes(E::Remaining)),
                ],
            ),
        )
    };
    let len = |bytes: &[u8]| Evaluator::new(t()).node(&doc(bytes), &[0]).unwrap().size_bits / 8;

    // Stops before the start code, so it belongs to the unit after it.
    assert_eq!(len(&[9, 8, 7, 0, 0, 1, 0x65]), 3);
    // Two of the three bytes are not the marker, and neither is a run of
    // zeros with nothing after it: emulation prevention writes `00 00 03`.
    assert_eq!(len(&[0, 0, 3, 1, 0, 0, 1, 0x41]), 4);
    // A start code at the very end is still a start code, since with nothing
    // to tell it apart from there is no successor to wait for.
    assert_eq!(len(&[9, 0, 0, 1]), 1);
    // None anywhere: a stream cut off still places its bytes.
    assert_eq!(len(&[9, 8, 7, 0, 0]), 5);
    // A four-byte start code is the three-byte one with a zero in front, and
    // the measure stops at the three: the leading zero stays with the unit
    // before it, which is where the standard puts it too.
    assert_eq!(len(&[9, 8, 0, 0, 0, 1, 0x67]), 3);

    // Split across the seam between two of the 4096-byte blocks the search
    // reads in, which is what the overlap is for.
    let mut v = vec![7u8; 4095];
    v.extend_from_slice(&[0, 0, 1, 0x65]);
    assert_eq!(len(&v), 4095);
    let mut v = vec![7u8; 4094];
    v.extend_from_slice(&[0, 0, 1, 0x65]);
    assert_eq!(len(&v), 4094);

    // A lead of no bytes measures to nothing and says so.
    let empty = Template::new("t", T::structure("Root", vec![("nal", T::bytes(E::to_marker_seq(&[], &[])))]));
    let d = doc(&[1, 2, 3]);
    assert!(matches!(Evaluator::new(empty).node(&d, &[0]), Err(EvalError::Failed(_))));
}

#[test]
fn a_backwards_peek_reads_the_end_of_what_holds_it() {
    // A file signed at the far end, and a body that stops before the
    // signature or runs to the end depending on whether one is there.
    let signed = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("signature", T::computed(E::peek_at(E::lit(-32), 32, Big))),
                (
                    "body",
                    T::switch(
                        E::field("signature"),
                        vec![(0x454e4421, T::bytes(E::Remaining.sub(E::lit(4))))],
                        T::bytes(E::Remaining),
                    ),
                ),
                ("end", T::bytes(E::Remaining)),
            ],
        ),
    );
    let d = doc(b"payloadEND!");
    let mut ev = Evaluator::new(signed.clone());
    assert_eq!(ev.node(&d, &[0]).unwrap().value, Value::Int(0x454e4421));
    assert_eq!(ev.node(&d, &[1]).unwrap().size_bits, 7 * 8);
    assert_eq!(ev.node(&d, &[2]).unwrap().size_bits, 4 * 8);

    // The same template on a file with no signature: the body takes it all.
    let d = doc(b"payload");
    let mut ev = Evaluator::new(signed);
    assert_eq!(ev.node(&d, &[1]).unwrap().size_bits, 7 * 8);
    assert_eq!(ev.node(&d, &[2]).unwrap().size_bits, 0);

    // Counting back further than the container reaches is an error, the same
    // as looking past the end of it.
    let too_far = Template::new("t", T::structure("Root", vec![("s", T::computed(E::peek_at(E::lit(-64), 64, Big)))]));
    let mut ev = Evaluator::new(too_far);
    assert!(ev.node(&doc(b"tiny"), &[0]).is_err());
}

#[test]
fn an_offset_of_zero_points_at_nothing_when_the_list_says_so() {
    let item = T::structure("Item", vec![("len", T::u8()), ("text", T::utf8(E::field("len")))]);
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("count", T::u8()),
                ("ptrs", T::array(T::u16(Big), E::field("count"))),
                ("items", T::pointer_list("ptrs", Anchor::Window, E::lit(0), item).skipping_zero()),
            ],
        ),
    );
    // Three entries, of which the middle one is zero: a table with room for
    // more than the file holds.
    let d = doc(&[3, 0, 8, 0, 0, 0, 11, 0xff, 2, b'o', b'k', 3, b'b', b'e', b'e']);
    let mut ev = Evaluator::new(t);
    assert_eq!(ev.node(&d, &[2]).unwrap().child_count, 3);
    assert_eq!(ev.node(&d, &[2, 0, 1]).unwrap().value, Value::Str("ok".into()));
    // The zero keeps its place and covers nothing, rather than reading the
    // header the offsets are counted from.
    let none = ev.node(&d, &[2, 1]).unwrap();
    assert_eq!(none.size_bits, 0);
    assert_eq!(ev.node(&d, &[2, 2, 1]).unwrap().value, Value::Str("bee".into()));

    // Without it, that same zero is an offset outside the list, which is what
    // it would be if the format did not mean anything by it.
    let item = T::structure("Item", vec![("len", T::u8()), ("text", T::utf8(E::field("len")))]);
    let plain = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("count", T::u8()),
                ("ptrs", T::array(T::u16(Big), E::field("count"))),
                ("items", T::pointer_list("ptrs", Anchor::Window, E::lit(0), item)),
            ],
        ),
    );
    let mut ev = Evaluator::new(plain);
    assert!(ev.node(&d, &[2, 1]).is_err());
}

#[test]
fn space_between_pointed_at_items_is_a_gap_of_its_own() {
    let d = doc(POINTED);
    let mut ev = Evaluator::new(pointer_template());
    let spans = ev.spans(&d, 5 * 8, 13 * 8, 20).unwrap();
    // The byte no offset points at, then the earlier item, then the later.
    assert!(spans[0].gap);
    assert_eq!((spans[0].offset_bits, spans[0].size_bits), (5 * 8, 8));
    assert_eq!(spans[1].name, "len");
    assert_eq!(spans[2].value, Value::Str("bee".into()));
    assert_eq!(spans[4].value, Value::Str("ok".into()));
}

#[test]
fn an_offset_outside_the_list_fails_only_that_item() {
    let mut b = POINTED.to_vec();
    b[2] = 200; // the first offset now points past the end
    let d = doc(&b);
    let mut ev = Evaluator::new(pointer_template());
    assert!(ev.node(&d, &[2, 0]).is_err());
    assert_eq!(ev.node(&d, &[2, 1, 1]).unwrap().value, Value::Str("bee".into()));
    assert_eq!(ev.locate(&d, 7 * 8).unwrap(), vec![2, 1, 1]);
}

#[test]
fn a_field_takes_its_type_from_a_list_read_earlier() {
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
    assert_eq!(ev.node(&d, &[2, 0]).unwrap().value, Value::UInt(5));
    assert_eq!(ev.node(&d, &[2, 1]).unwrap().value, Value::UInt(7));
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

/// A stored integer, a slope and an intercept as floats, and what the integer
/// is worth: the shape of a NIfTI voxel and a FITS cell.
fn scaled_voxel(stored: i16, slope: f32, inter: f32, worth: Expr) -> (Document<MemSource>, Evaluator) {
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("slope", T::F32(Little)),
                ("inter", T::F32(Little)),
                ("stored", T::Int { bits: 16, endian: Little }),
                ("worth", T::computed_real(worth)),
            ],
        ),
    );
    let mut bytes = slope.to_le_bytes().to_vec();
    bytes.extend_from_slice(&inter.to_le_bytes());
    bytes.extend_from_slice(&stored.to_le_bytes());
    (doc(&bytes), Evaluator::new(t))
}

fn failure(e: EvalError) -> String {
    match e {
        EvalError::Failed(s) => s,
        other => panic!("not a failure: {other:?}"),
    }
}

#[test]
fn a_real_computed_field_reads_as_the_number_it_works_out() {
    let worth = E::field("stored").mul(E::field("slope")).add(E::field("inter"));
    let (d, mut ev) = scaled_voxel(11980, 0.0754, 3100.76, worth);
    let node = ev.node(&d, &[3]).unwrap();
    assert_eq!((node.type_name.as_str(), node.size_bits), ("computed real", 0));
    // The floats as they read, which is the value the row beside them shows,
    // and the arithmetic done in doubles.
    assert_eq!(node.value, Value::Float(11980.0 * 0.0754 + 3100.76));
    // Kept on the node, and the same the second time.
    assert_eq!(ev.node(&d, &[3]).unwrap().value, node.value);

    // A literal with a fraction, a condition that stays a whole-number
    // question, and a negative power of two, which is a fraction too.
    let halves = E::cond(E::field("stored").less_than(E::lit(0)), E::real(0.5), E::field("stored").mul(E::pow2(E::lit(-3))));
    let (d, mut ev) = scaled_voxel(12, 1.0, 0.0, halves);
    assert_eq!(ev.node(&d, &[3]).unwrap().value, Value::Float(1.5));
    let (d, mut ev) = scaled_voxel(-12, 1.0, 0.0, E::cond(E::field("stored").less_than(E::lit(0)), E::real(0.5), E::lit(7)));
    assert_eq!(ev.node(&d, &[3]).unwrap().value, Value::Float(0.5));

    // A slope of nought is an answer rather than a reason to take the other
    // side, and a division by nought fails as it does in a whole number.
    let (d, mut ev) = scaled_voxel(3, 0.5, 0.0, E::field("slope").or(E::real(1.0)).mul(E::field("stored")));
    assert_eq!(ev.node(&d, &[3]).unwrap().value, Value::Float(1.5));
    let (d, mut ev) = scaled_voxel(3, 0.0, 0.0, E::field("slope").or(E::real(1.0)).mul(E::field("stored")));
    assert_eq!(ev.node(&d, &[3]).unwrap().value, Value::Float(3.0));
    let (d, mut ev) = scaled_voxel(3, 0.0, 0.0, E::field("stored").div(E::field("inter")));
    assert_eq!(failure(ev.node(&d, &[3]).unwrap_err()), "division by zero");
}

#[test]
fn a_real_has_no_place_in_a_size_or_a_count() {
    let within = |len: Expr| {
        let t = Template::new(
            "t",
            T::structure("Root", vec![("n", T::F32(Little)), ("text", T::bytes(E::lit(4))), ("data", T::bytes(len))]),
        );
        let mut bytes = 2.0f32.to_le_bytes().to_vec();
        bytes.extend_from_slice(b"2.0 ");
        bytes.extend_from_slice(&[0; 8]);
        let mut ev = Evaluator::new(t);
        failure(ev.node(&doc(&bytes), &[2]).unwrap_err())
    };
    let hint = "trunc(...) drops the fraction";
    // A float named where a length is wanted says it was read as a whole
    // number, rather than calling it not a number at all.
    assert_eq!(within(E::field("n")), format!("n is a real number, but a whole number is needed here; {hint}"));
    assert_eq!(within(E::within(&["n"])), format!("n is a real number, but a whole number is needed here; {hint}"));
    // A real literal, and the real text spells.
    for len in [E::real(2.0), E::real_text(E::field("text"))] {
        assert_eq!(within(len.clone()), format!("a real number where a whole number is needed; {hint}"), "{len:?}");
    }
    // A negative power is a fraction, and says so rather than sending the
    // reader to `trunc`, which would make it nought.
    assert_eq!(within(E::pow2(E::lit(-1))), "two to a negative power is not a whole number");
    assert_eq!(within(E::pow10(E::lit(-1))), "ten to a negative power is not a whole number");
    // A count the same way, and a computed whole number too.
    let t = Template::new(
        "t",
        T::structure("Root", vec![("n", T::F32(Little)), ("k", T::computed(E::field("n"))), ("items", T::array(T::u8(), E::real(1.0)))]),
    );
    let mut ev = Evaluator::new(t);
    let d = doc(&[0, 0, 0x80, 0x3f, 9]);
    assert!(failure(ev.node(&d, &[2]).unwrap_err()).ends_with(hint));
    assert!(failure(ev.node(&d, &[1]).unwrap_err()).starts_with("n is a real number"));
    // And a power that is whole is the whole number it comes to.
    assert_eq!(within(E::pow2(E::lit(4))), "runs past the end of its container");
}

/// A length or an offset read from the file can be any 64-bit number, and
/// the largest of them are more bytes than a `u64` counts in bits. Each is
/// refused as running past what holds it, the same as a merely large one,
/// instead of overflowing the multiplication by eight.
#[test]
fn a_length_too_large_to_count_in_bits_runs_past_its_container() {
    let huge = [0xFF; 8];
    let reading = |field: T, at: &[usize]| {
        let t = Template::new("t", T::structure("Root", vec![("n", T::u64(Little)), ("x", field)]));
        let mut ev = Evaluator::new(t);
        let d = doc(&[&huge[..], &[1, 2, 3, 4]].concat());
        ev.node(&d, at).map(|n| n.size_bits).map_err(failure)
    };
    assert_eq!(reading(T::sized(E::field("n"), T::bytes(E::Remaining)), &[1]), Err("size 18446744073709551615 runs past the end of its container".into()));
    assert_eq!(reading(T::bytes(E::field("n")), &[1]), Err("runs past the end of its container".into()));
    // The pointer itself covers nothing; what it points at is refused.
    assert_eq!(reading(T::at(E::field("n"), T::u8()), &[1, 0]), Err("runs past the end of the file".into()));
}

#[test]
fn the_whole_part_of_a_float_places_bytes() {
    let placed = |offset: f32| {
        let t = Template::new(
            "t",
            T::structure("Root", vec![("vox_offset", T::F32(Little)), ("voxel", T::at(E::trunc(E::field("vox_offset")), T::u8()))]),
        );
        let mut bytes = offset.to_le_bytes().to_vec();
        bytes.extend_from_slice(&[10, 11, 12, 13, 14]);
        let mut ev = Evaluator::new(t);
        ev.node(&doc(&bytes), &[1, 0]).map(|n| (n.offset_bits / 8, n.value)).map_err(failure)
    };
    assert_eq!(placed(6.0), Ok((6, Value::UInt(12))));
    // Towards nought, as nibabel takes a fraction off.
    assert_eq!(placed(6.9), Ok((6, Value::UInt(12))));
    assert_eq!(placed(-1.5), Err("negative offset".to_string()));
    assert_eq!(placed(f32::NAN), Err("trunc(...) of NaN".to_string()));
    assert_eq!(placed(f32::INFINITY), Err("trunc(...) of infinity".to_string()));
    assert_eq!(placed(3e38), Err("trunc(...) of a number too large to hold".to_string()));
}

#[test]
fn digits_read_as_the_real_they_spell() {
    let spelled = |text: &str| {
        let t = Template::new(
            "t",
            T::structure(
                "Root",
                vec![
                    ("text", T::text(StrLen::Fixed(E::Remaining), Encoding::Ascii)),
                    ("value", T::computed_real(E::real_text(E::field("text")))),
                ],
            ),
        );
        let mut ev = Evaluator::new(t);
        match ev.node(&doc(text.as_bytes()), &[1]) {
            Ok(n) => Ok(n.value),
            Err(e) => Err(failure(e)),
        }
    };
    for (text, want) in [
        ("2.0E+01", 20.0),
        ("1.0D-3", 0.001),
        ("   .5  ", 0.5),
        ("1.", 1.0),
        ("32768", 32768.0),
        ("-2.5e2", -250.0),
        ("", 0.0),
        ("        ", 0.0),
    ] {
        assert_eq!(spelled(text), Ok(Value::Float(want)), "{text:?}");
    }
    assert_eq!(spelled("abc"), Err("\"abc\" is not a number".to_string()));
    assert_eq!(spelled("nan"), Err("\"nan\" is not a number".to_string()));
    // A search that finds no card is nothing, which reads as nought, and so
    // `Or` can give it a default.
    let card = T::structure("Card", vec![("key", T::bytes(E::lit(2))), ("text", T::text(StrLen::Fixed(E::lit(6)), Encoding::Ascii))]);
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("cards", T::array(card, E::lit(1))),
                ("scale", T::computed_real(E::real_text(E::tagged_bytes("cards", &["key"], b"SC", &["text"])).or(E::real(1.0)))),
                ("zero", T::computed_real(E::real_text(E::tagged_bytes("cards", &["key"], b"ZE", &["text"])).or(E::real(1.0)))),
            ],
        ),
    );
    let mut ev = Evaluator::new(t);
    let d = doc(b"ZE 0.25 ");
    assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::Float(1.0));
    assert_eq!(ev.node(&d, &[2]).unwrap().value, Value::Float(0.25));
}

#[test]
fn two_to_a_negative_power_is_a_real_and_not_a_shift() {
    let (d, mut ev) = scaled_voxel(639, 1.0, 253.02, E::field("inter").add(E::field("stored").mul(E::pow2(E::lit(-5)))));
    assert_eq!(ev.node(&d, &[3]).unwrap().value, Value::Float(253.02 + 639.0 / 32.0));
    // Over a power of ten, the way a GRIB value is.
    let (d, mut ev) = scaled_voxel(5, 1.0, 0.0, E::field("stored").mul(E::pow2(E::lit(3))).div(E::pow10(E::lit(2))));
    assert_eq!(ev.node(&d, &[3]).unwrap().value, Value::Float(0.4));
    // In a whole number the same two are the shift and the power they would
    // be, and nothing past what 128 bits hold.
    let whole = |e: Expr| {
        let t = Template::new("t", T::structure("Root", vec![("k", T::computed(e))]));
        Evaluator::new(t).node(&doc(&[]), &[0]).map(|n| n.value).map_err(failure)
    };
    assert_eq!(whole(E::pow2(E::lit(10))), Ok(Value::Int(1024)));
    assert_eq!(whole(E::pow10(E::lit(38))), Ok(Value::Int(10i128.pow(38))));
    assert_eq!(whole(E::pow2(E::lit(127))), Err("two to the power 127 is too large to hold".to_string()));
    assert_eq!(whole(E::pow10(E::lit(39))), Err("ten to the power 39 is too large to hold".to_string()));
    // And ten to a power is exact as far as a double holds one.
    let real = |e: Expr| {
        let t = Template::new("t", T::structure("Root", vec![("k", T::computed_real(e))]));
        Evaluator::new(t).node(&doc(&[]), &[0]).unwrap().value
    };
    assert_eq!(real(E::pow10(E::lit(22))), Value::Float(1e22));
    assert_eq!(real(E::pow10(E::lit(-9))), Value::Float(1e-9));
    assert_eq!(real(E::pow10(E::lit(-30))), Value::Float(1e-30));
    assert_eq!(real(E::pow10(E::lit(300))), Value::Float(1e300));
}

/// The relations panel writes a real formula the way it writes a whole one:
/// as the template writes it, then with each field's value in its place, a
/// float as its row shows it.
#[test]
fn a_real_relation_is_written_with_its_values_in_place() {
    let worth = E::field("stored").mul(E::field("slope")).add(E::field("inter"));
    let (d, mut ev) = scaled_voxel(11980, 0.0754, 3100.76, worth);
    let rel = ev.relations(&d, &[3]).unwrap();
    assert_eq!(rel.len(), 1, "{rel:?}");
    assert_eq!(rel[0].role, Role::Value);
    assert_eq!(rel[0].written, "stored * slope + inter");
    assert_eq!(rel[0].substituted, "11980 * 0.0754 + 3100.76");
    assert_eq!(rel[0].result, (11980.0 * 0.0754 + 3100.76f64).to_string());

    // A scale read from text, with its default: the card's number in place of
    // the card, and the literal keeping its point.
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("text", T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii)),
                ("stored", T::u8()),
                ("worth", T::computed_real(E::real_text(E::field("text")).or(E::real(1.0)).mul(E::field("stored")))),
            ],
        ),
    );
    let d = doc(b" 2.5\x04");
    let mut ev = Evaluator::new(t);
    let rel = ev.relations(&d, &[2]).unwrap();
    assert_eq!(rel[0].written, "(real(text) or else 1.0) * stored");
    assert_eq!(rel[0].substituted, "(2.5 or else 1.0) * 4");
    assert_eq!(rel[0].result, "10");

    // And a float's whole part placing bytes, in a relation that is a whole
    // number: the float is still written in as the float.
    let t = Template::new(
        "t",
        T::structure("Root", vec![("off", T::F32(Little)), ("body", T::bytes(E::trunc(E::field("off")).sub(E::lit(4))))]),
    );
    let mut bytes = 6.9f32.to_le_bytes().to_vec();
    bytes.extend_from_slice(&[0, 0]);
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);
    let rel = ev.relations(&d, &[1]).unwrap();
    assert_eq!((rel[0].written.as_str(), rel[0].substituted.as_str(), rel[0].result.as_str()), ("trunc(off) - 4", "trunc(6.9) - 4", "2"));

    // A float found by walking back through a list, which as a whole number
    // passes over the float and answers nought: written in as the float, the
    // way a GRIB section 7 reads section 5's reference value.
    let section = T::switch(
        E::peek(8, Big),
        vec![(1, T::structure("Five", vec![("kind", T::u8()), ("reference", T::F32(Big))]))],
        T::structure("Seven", vec![("kind", T::u8()), ("worth", T::computed_real(E::sibling(&["reference"]).add(E::lit(1))))]),
    );
    let t = Template::new("t", T::structure("Root", vec![("sections", T::array(section, E::lit(2)))]));
    let mut bytes = vec![1];
    bytes.extend_from_slice(&2.5f32.to_be_bytes());
    bytes.push(7);
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);
    assert_eq!(ev.node(&d, &[0, 1, 1]).unwrap().value, Value::Float(3.5));
    let rel = ev.relations(&d, &[0, 1, 1]).unwrap();
    assert_eq!((rel[0].written.as_str(), rel[0].substituted.as_str(), rel[0].result.as_str()), ("earlier(reference) + 1", "2.5 + 1", "3.5"));
}

/// What a scaled number is worth is a reading of the stored integer, and the
/// stored integer is what an edit writes.
#[test]
fn a_real_computed_field_is_not_editable() {
    let (d, mut ev) = scaled_voxel(7, 2.0, 0.5, E::field("stored").mul(E::field("slope")).add(E::field("inter")));
    assert!(!ev.node(&d, &[3]).unwrap().editable);
    assert!(ev.node(&d, &[2]).unwrap().editable);
    assert!(ev.prepare_write(&d, &[3], "15").is_err());
}

/// The origins of a worth are the fields its formula reads, with what each
/// holds, and a float's whole part points at the float.
#[test]
fn a_real_field_names_the_fields_it_reads() {
    let worth = E::field("stored").mul(E::pow2(E::field("stored").sub(E::lit(5)))).add(E::field("inter"));
    let (d, mut ev) = scaled_voxel(3, 1.0, 0.25, worth);
    assert_eq!(ev.node(&d, &[3]).unwrap().value, Value::Float(1.0));
    let seen: Vec<_> = ev.origins(&d, &[3]).unwrap().into_iter().map(|o| (o.role, o.label, o.value)).collect();
    assert_eq!(
        seen,
        vec![
            (Role::Value, "stored".to_string(), "3".to_string()),
            (Role::Value, "stored".to_string(), "3".to_string()),
            (Role::Value, "inter".to_string(), "0.25".to_string()),
        ]
    );
    let t = Template::new(
        "t",
        T::structure("Root", vec![("vox_offset", T::F32(Little)), ("voxel", T::at(E::trunc(E::field("vox_offset")), T::u8()))]),
    );
    let mut bytes = 4.0f32.to_le_bytes().to_vec();
    bytes.push(9);
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);
    let labels: Vec<_> = ev.origins(&d, &[1, 0]).unwrap().into_iter().map(|o| (o.role, o.label)).collect();
    assert_eq!(labels, vec![(Role::Position, "vox_offset".to_string())]);
}

/// What a `computed` field was is what it is: a whole number, cached on the
/// node, that reads a float field as a refusal and not as a nought.
#[test]
fn an_integer_computed_field_reads_as_it_did() {
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("a", T::u16(Big)),
                ("b", T::u16(Big)),
                ("total", T::computed(E::field("a").add(E::field("b")).div(E::lit(3)))),
                ("items", T::array(T::structure("Item", vec![("v", T::u8()), ("running", T::computed(E::prev("running").add(E::field("v"))))]), E::lit(3))),
            ],
        ),
    );
    let d = doc(&[0, 10, 0, 1, 1, 2, 3]);
    let mut ev = Evaluator::new(t);
    let total = ev.node(&d, &[2]).unwrap();
    assert_eq!((total.type_name.as_str(), total.value.clone(), total.size_bits), ("computed", Value::Int(3), 0));
    assert_eq!(ev.node(&d, &[3, 2, 1]).unwrap().value, Value::Int(6));
}

#[test]
fn a_run_of_same_sized_blocks_is_counted_by_division() {
    // What a paged file is: a header saying how big a block is, then blocks of
    // that size until the file runs out. Nothing here is fixed at template
    // time, and the count is still arithmetic.
    let block = T::sized(E::field("block_size"), T::structure("Block", vec![("kind", T::u8())]));
    let t = Template::new(
        "t",
        T::structure("Root", vec![("block_size", T::u16(Little)), ("blocks", T::repeat(block, Until::End))]),
    );
    let (size, count) = (4096usize, 50_000usize);
    let mut bytes = (size as u16).to_le_bytes().to_vec();
    bytes.resize(2 + size * count, 0);
    bytes[2] = 9;
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);
    let all = ev.node(&d, &[1]).unwrap();
    assert_eq!((all.child_count, all.size_bits), (count as u64, (size * count) as u64 * 8));
    assert!(ev.memo.len() < 10, "counted by division, not by a walk: {} entries", ev.memo.len());
    // A block near the end is one division away, and so is the cursor
    // standing in it.
    let at = (2 + size * 49_999) as u64 * 8;
    assert_eq!(ev.node(&d, &[1, 49_999, 0]).unwrap().offset_bits, at);
    assert_eq!(ev.locate(&d, at).unwrap(), vec![1, 49_999, 0]);
    assert!(ev.memo.len() < 20, "{} entries", ev.memo.len());
}

#[test]
fn a_file_cut_off_mid_block_keeps_the_blocks_it_has() {
    // Half a block at the end is not a block. It belongs to nothing, rather
    // than taking the whole run down with it.
    let block = T::sized(E::field("block_size"), T::bytes(E::Remaining));
    let t = Template::new(
        "t",
        T::structure("Root", vec![("block_size", T::u16(Little)), ("blocks", T::repeat(block, Until::End))]),
    );
    let mut bytes = 16u16.to_le_bytes().to_vec();
    bytes.resize(2 + 16 * 3 + 5, 0);
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);
    let all = ev.node(&d, &[1]).unwrap();
    assert_eq!((all.child_count, all.size_bits), (3, 3 * 16 * 8));
    // The half block at the end belongs to no field, which is a gap: the root
    // and nothing under it, rather than an error.
    assert!(ev.locate(&d, (2 + 16 * 3 + 1) as u64 * 8).unwrap().is_empty());
}

#[test]
fn a_run_that_stops_on_what_it_reads_is_still_walked() {
    // The elements are all the same size, but the run ends at the one holding
    // a chosen value, and where that is cannot be divided out.
    let block = T::sized(E::field("block_size"), T::structure("Block", vec![("kind", T::u8())]));
    let until = Until::FieldBytes { field: "kind".into(), bytes: vec![0xff] };
    let t = Template::new(
        "t",
        T::structure("Root", vec![("block_size", T::u16(Little)), ("blocks", T::repeat(block, until))]),
    );
    let mut bytes = 4u16.to_le_bytes().to_vec();
    bytes.resize(2 + 4 * 10, 0);
    bytes[2 + 4 * 2] = 0xff;
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);
    assert_eq!(ev.node(&d, &[1]).unwrap().child_count, 3);
}

#[test]
fn counting_a_long_run_costs_the_same_however_long_it_is() {
    // A run that ends where the file does, of elements no two of which are the
    // same length. How many there are can only be found by walking, and the
    // walk must not leave one node per element behind: this is the shape a
    // file whose contents are a list of a billion things has.
    let count = |n: usize| {
        let mut bytes = Vec::new();
        for i in 0..n {
            let s = "x".repeat(i % 17 + 1);
            bytes.extend_from_slice(&(s.len() as u64).to_le_bytes());
            bytes.extend_from_slice(s.as_bytes());
        }
        let string = T::structure("String", vec![("len", T::u64(Little)), ("text", T::utf8(E::field("len")))]);
        let d = doc(&bytes);
        let mut ev = Evaluator::new(Template::new("t", T::repeat(string, Until::End)));
        assert_eq!(ev.node(&d, &[]).unwrap().child_count, n as u64);
        ev.memo_len()
    };
    // Three times the elements, and what is left in memory is the same: the
    // few thousand a short run is remembered by, and the window after them.
    let (small, large) = (count(20_000), count(60_000));
    assert!(large < 3 * small / 2, "counting kept {small} nodes for 20,000 and {large} for 60,000");
    assert!(large < 20_000, "counting kept {large} nodes");
}

#[test]
fn counting_a_long_run_a_bit_at_a_time_costs_no_more_than_counting_it_at_once() {
    // The browser counts in goes of a few thousand elements, so that the page
    // can draw between them. Every go but the first carries on a count that
    // already knows its run is long, and must keep dropping what it walks
    // past: this is the shape the count actually has when a file is open.
    let n = 40_000usize;
    let mut bytes = Vec::new();
    for i in 0..n {
        let s = "x".repeat(i % 17 + 1);
        bytes.extend_from_slice(&(s.len() as u64).to_le_bytes());
        bytes.extend_from_slice(s.as_bytes());
    }
    let string = T::structure("String", vec![("len", T::u64(Little)), ("text", T::utf8(E::field("len")))]);
    let d = doc(&bytes);
    let mut ev = Evaluator::new(Template::new("t", T::repeat(string, Until::End)));
    ev.set_slice(Some(500));
    let mut goes = 0;
    let counted = loop {
        goes += 1;
        assert!(goes < 1000, "the count is not getting anywhere");
        ev.begin_slice();
        match ev.node(&d, &[]) {
            Ok(info) => break info.child_count,
            Err(e) if e.interrupted() => continue,
            Err(e) => panic!("{e:?}"),
        }
    };
    assert_eq!(counted, n as u64);
    assert!(goes > 10, "the count was not interrupted, so this proves nothing");
    assert!(ev.memo_len() < 20_000, "counting in goes kept {} nodes", ev.memo_len());
}

#[test]
fn a_listing_asked_again_in_goes_gets_further_every_go() {
    // The shape of a Thrift footer: runs that end on a stop record, inside
    // runs, so every row of the listing sits two walks deep. `spans` starts
    // again from the top of its window each time it is asked, and going back
    // over the rows the last go reached used to be charged as if they were
    // being read for the first time. Once that cost more than a go allows,
    // every go ran out in the same place: a Parquet file of 73 KB never
    // finished at 5,000 a go and finished in one at 12,000.
    let item = T::structure("Item", vec![("kind", T::u8()), ("v", T::u8())]);
    let group = T::structure("Group", vec![("items", T::repeat(item, Until::FieldValue { field: "kind".into(), value: 0 }))]);
    let t = Template::new("t", T::repeat(group, Until::End));
    let mut bytes = Vec::new();
    for _ in 0..30 {
        for f in 1..=20u8 {
            bytes.extend_from_slice(&[1, f]);
        }
        bytes.extend_from_slice(&[0, 0]);
    }
    let d = doc(&bytes);
    let len = d.len_bits();
    let want = Evaluator::new(t.clone()).spans(&d, 0, len, 4000).unwrap();
    assert!(want.len() > 1000, "one row per field, not one per run: {}", want.len());

    let mut ev = Evaluator::new(t);
    ev.set_slice(Some(200));
    let mut goes = 0;
    let got = loop {
        goes += 1;
        assert!(goes <= 50, "asking again is not getting anywhere");
        ev.begin_slice();
        match ev.spans(&d, 0, len, 4000) {
            Ok(v) => break v,
            Err(e) if e.interrupted() => continue,
            Err(e) => panic!("{e:?}"),
        }
    };
    assert!(goes > 1, "the listing was not interrupted, so this proves nothing");
    assert_eq!(got.len(), want.len());
    for (g, w) in got.iter().zip(&want) {
        assert_eq!((g.offset_bits, g.size_bits, &g.name), (w.offset_bits, w.size_bits, &w.name));
    }
}

#[test]
fn a_long_list_of_uneven_elements_is_walked_without_being_remembered() {
    // Strings of growing length, so every element sits at an offset only the
    // walk can find, and one the test can work out for itself.
    let n = 20_000u64;
    let mut bytes = n.to_le_bytes().to_vec();
    let mut starts = vec![8u64];
    for i in 0..n {
        let s = "x".repeat((i % 17) as usize + 1);
        bytes.extend_from_slice(&(s.len() as u64).to_le_bytes());
        bytes.extend_from_slice(s.as_bytes());
        starts.push(bytes.len() as u64);
    }
    let string = T::structure("String", vec![("len", T::u64(Little)), ("text", T::utf8(E::field("len")))]);
    let t = Template::new(
        "t",
        T::structure("Root", vec![("n", T::u64(Little)), ("items", T::array(string, E::field("n")))]),
    );
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);

    let items = ev.node(&d, &[1]).unwrap();
    assert_eq!(items.child_count, n);
    assert_eq!(items.offset_bits + items.size_bits, bytes.len() as u64 * 8);
    // Twenty thousand elements, and what is left behind is the window the walk
    // keeps rather than one node per element.
    assert!(ev.memo_len() < 100, "the walk kept {} nodes", ev.memo_len());

    // An element in the middle, reached long after the walk passed it, is
    // where the file says it is and reads as what it holds.
    for i in [15_000usize, 3, 19_999, 7_777] {
        let e = ev.node(&d, &[1, i]).unwrap();
        assert_eq!(e.offset_bits, starts[i] * 8, "element {i}");
        let text = ev.node(&d, &[1, i, 1]).unwrap();
        assert_eq!(text.value, Value::Str("x".repeat(i % 17 + 1)));
    }
    assert!(ev.memo_len() < 100, "reaching into it kept {} nodes", ev.memo_len());
}

#[test]
fn a_walk_past_a_node_read_into_keeps_the_node() {
    // Four records, each a length and then a structure whose second field is
    // that long. The run is guarded from its first element, so every walk
    // along it keeps a journal of what it placed and drops that at the end.
    let mut bytes = Vec::new();
    for i in 0..4u8 {
        bytes.extend([i + 1, 0xa0 + i]);
        bytes.extend(std::iter::repeat_n(0xb0 + i, i as usize + 1));
    }
    let inner = T::structure("Inner", vec![("a", T::u8()), ("b", T::bytes(E::field("len")))]);
    let item = T::structure("Item", vec![("len", T::u8()), ("inner", inner)]);
    let t = Template::new("t", T::structure("Root", vec![("items", T::repeat(item, Until::End))]));
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);

    // Reading into record 1 places it, with nothing asking how long it is.
    assert_eq!(ev.node(&d, &[0, 1, 1, 0]).unwrap().value, Value::UInt(0xa1));
    // Reaching record 3 walks over record 1 and places it again. It was here
    // before the walk, so the walk must not take it away when it ends while
    // what was read inside it stays.
    ev.node(&d, &[0, 3]).unwrap();
    assert_eq!(ev.memo.without_parent(), Vec::<Vec<usize>>::new());
    // A field of record 1 read now looks up past its structure for `len`,
    // which is only there if record 1 is.
    let b = ev.node(&d, &[0, 1, 1, 1]).unwrap();
    assert_eq!(b.size_bits, 2 * 8);
}

#[test]
fn a_walk_inside_a_walk_keeps_nothing_its_parent_let_go_of() {
    // Records of a width, seven bytes of entries, and a tail as long as the
    // third entry says. Each entry is a count and that many values of the
    // record's width. Sizing a record finds its third entry, which walks the
    // entries inside the walk along the records.
    let entry = T::structure(
        "Entry",
        vec![("len", T::u8()), ("body", T::array(T::sized(E::field("w"), T::bytes(E::Remaining)), E::field("len")))],
    );
    let item = T::structure(
        "Item",
        vec![
            ("w", T::u8()),
            ("entries", T::sized(E::lit(7), T::repeat(entry, Until::End))),
            ("tail", T::bytes(E::elem_field("entries", E::lit(2), &["len"]))),
        ],
    );
    let t = Template::new("t", T::structure("Root", vec![("items", T::repeat(item, Until::End))]));
    let mut bytes = Vec::new();
    for i in 0..4u8 {
        bytes.extend([1, /* entries */ 1, 0xa0 + i, 2, 0xb0 + i, 0xb8 + i, 1, 0xc0 + i, /* tail */ 0xd0 + i]);
    }
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);

    // Reaching record 3 walks over records 0 to 2, and sizing each walks its
    // entries to the third. That inner walk keeps the second entry for the
    // third to ask, and the outer walk then lets record 0 go. The entry must
    // go with it: left behind, it has no record above it to look up to.
    ev.node(&d, &[0, 3]).unwrap();
    assert_eq!(ev.memo.without_parent(), Vec::<Vec<usize>>::new());
    // A value of record 0's second entry, asked for by its path. Where it
    // starts is worked out from `w`, which is in the record.
    let v = ev.node(&d, &[0, 0, 1, 1, 1, 1]).unwrap();
    assert_eq!((v.offset_bits, v.size_bits), (5 * 8, 8));
}

#[test]
fn an_element_that_ends_a_run_leaves_none_of_its_fields_behind() {
    // Records of a length and that many bytes. The third says nine, and nine
    // bytes are not there: its length is read before its bytes fail, and the
    // run ends before it.
    let rec = || T::structure("Rec", vec![("len", T::u8()), ("body", T::bytes(E::field("len")))]);
    let bytes = [1, 0xa0, 2, 0xb0, 0xb1, 9, 0xc0];

    // A run with no room outside it to try the element again in.
    let d = doc(&bytes);
    let mut ev = Evaluator::new(Template::new("t", T::repeat(rec(), Until::End)));
    assert_eq!(ev.node(&d, &[]).unwrap().child_count, 2);
    assert_eq!(ev.memo.without_parent(), Vec::<Vec<usize>>::new());

    // A run in a window one byte short of the file, which is tried again with
    // that byte and fails again.
    let t = T::structure("Root", vec![("items", T::sized(E::lit(6), T::repeat(rec(), Until::End))), ("tail", T::bytes(E::Remaining))]);
    let mut ev = Evaluator::new(Template::new("t", t));
    assert_eq!(ev.node(&d, &[0]).unwrap().child_count, 2);
    assert_eq!(ev.memo.without_parent(), Vec::<Vec<usize>>::new());
}

#[test]
fn an_overwrite_keeps_the_nodes_above_what_it_keeps() {
    // A length, three records of a byte and that many bytes in a room of
    // three each, and a tail for the edit to land in.
    let rec = T::sized(E::lit(3), T::structure("Rec", vec![("a", T::u8()), ("b", T::bytes(E::field("n")))]));
    let t = T::structure("Root", vec![("n", T::u8()), ("items", T::array(rec, E::lit(3))), ("tail", T::bytes(E::Remaining))]);
    let mut d = doc(&[2, 0xa0, 0xa1, 0xa2, 0xb0, 0xb1, 0xb2, 0xc0, 0xc1, 0xc2, 0xee, 0xee]);
    let mut ev = Evaluator::new(Template::new("t", t));
    // Record 1 is placed and sized by its room, with nothing inside it read
    // and nothing above it sized.
    assert_eq!(ev.node(&d, &[1, 1]).unwrap().offset_bits, 4 * 8);

    d.overwrite_bytes(11, &[0x55]);
    ev.invalidate_from(11 * 8);
    assert_eq!(ev.memo.without_parent(), Vec::<Vec<usize>>::new());
    // How long its second field is comes from `n`, found through the root.
    let b = ev.node(&d, &[1, 1, 1]).unwrap();
    assert_eq!((b.offset_bits, b.size_bits), (5 * 8, 2 * 8));
}

#[test]
fn a_node_kept_above_an_overwrite_is_measured_again() {
    // Records of a length and that many bytes, then a tail.
    let rec = T::structure("Rec", vec![("a", T::u8()), ("b", T::bytes(E::field("a")))]);
    let t = || Template::new("t", T::structure("Root", vec![("n", T::u8()), ("items", T::array(rec.clone(), E::lit(3))), ("tail", T::bytes(E::Remaining))]));
    let mut d = doc(&[3, 1, 0xa0, 2, 0xb0, 0xb1, 1, 0xc0, 0xee, 0xee, 0xee, 0xee]);
    let mut ev = Evaluator::new(t());
    assert_eq!(ev.node(&d, &[2]).unwrap().offset_bits, 8 * 8);

    // Records 1 and 2 shorter. Record 0 ended before them and stays, and so
    // do the list and the root, but the list is not the length it was.
    d.overwrite_bytes(3, &[1, 0xb0, 0]);
    ev.invalidate_from(3 * 8);
    assert_eq!(ev.memo.without_parent(), Vec::<Vec<usize>>::new());
    assert!(ev.memo.contains_key(&[1, 0]));
    let mut fresh = Evaluator::new(t());
    for path in [&[1][..], &[2], &[1, 2, 1], &[]] {
        assert_eq!(ev.node(&d, path).unwrap(), fresh.node(&d, path).unwrap(), "{path:?}");
    }
    assert_eq!(ev.node(&d, &[1]).unwrap().size_bits, 5 * 8);
}

#[test]
fn an_overwrite_of_a_pointer_drops_what_it_pointed_at() {
    // A byte, a pointer to a byte, and the pointed-at byte, declared after the
    // pointer but lying before it.
    let t = T::structure("Root", vec![("hdr", T::u8()), ("p", T::u8()), ("pad", T::u8()), ("t", T::at(E::field("p"), T::u8()))]);
    let mut d = doc(&[0x11, 0, 0x33]);
    let mut ev = Evaluator::new(Template::new("t", t));
    assert_eq!(ev.node(&d, &[3, 0]).unwrap().value, Value::UInt(0x11));

    // The byte it points at ended before the edit, but where it is was read
    // after it.
    d.overwrite_bytes(1, &[2]);
    ev.invalidate_from(8);
    assert_eq!(ev.memo.without_parent(), Vec::<Vec<usize>>::new());
    let pointed = ev.node(&d, &[3, 0]).unwrap();
    assert_eq!((pointed.offset_bits, pointed.value), (2 * 8, Value::UInt(0x33)));
}

#[test]
fn an_overwrite_of_an_element_that_overran_its_room_gives_the_room_back() {
    // Records of a length and that many bytes, in a room one byte short of the
    // second one, which is read past the room since there is file outside it.
    let rec = T::structure("Rec", vec![("len", T::u8()), ("body", T::bytes(E::field("len")))]);
    let t = || Template::new("t", T::structure("Root", vec![("items", T::sized(E::lit(4), T::repeat(rec.clone(), Until::End))), ("tail", T::bytes(E::Remaining))]));
    let mut d = doc(&[1, 0xa0, 2, 0xb0, 0xb1, 0xee, 0xee, 0xee]);
    let mut ev = Evaluator::new(t());
    assert_eq!(ev.node(&d, &[0]).unwrap().child_count, 2);
    assert_eq!(ev.node(&d, &[1]).unwrap().offset_bits, 5 * 8);

    // The second record now fits the room, so the room is what the template
    // said it was.
    d.overwrite_bytes(2, &[1]);
    ev.invalidate_from(2 * 8);
    assert_eq!(ev.memo.without_parent(), Vec::<Vec<usize>>::new());
    let mut fresh = Evaluator::new(t());
    for path in [&[0][..], &[1], &[0, 1]] {
        assert_eq!(ev.node(&d, path).unwrap(), fresh.node(&d, path).unwrap(), "{path:?}");
    }
    assert_eq!(ev.node(&d, &[1]).unwrap().offset_bits, 4 * 8);
}

#[test]
fn an_overwrite_inside_json_drops_every_value_the_parse_placed() {
    // The first element of `a` ends before the edit, but where `a` ends is
    // where the member after it starts, and the edit moves that.
    let text = br#"{"a":[1,2,3],"c":4}"#;
    let mut d = doc(text);
    let mut ev = Evaluator::new(Template::new("json", T::json()));
    ev.node(&d, &[0]).unwrap();
    ev.node(&d, &[0, 0]).unwrap();

    d.overwrite_bytes(9, br#"],"d":5}  "#);
    ev.invalidate_from(9 * 8);
    assert_eq!(ev.memo.without_parent(), Vec::<Vec<usize>>::new());
    let mut fresh = Evaluator::new(Template::new("json", T::json()));
    for path in [&[0][..], &[0, 0], &[1]] {
        assert_eq!(ev.node(&d, path).unwrap(), fresh.node(&d, path).unwrap(), "{path:?}");
    }
}

#[test]
fn the_field_under_a_bit_is_found_without_the_list_coming_back() {
    // The same long list of uneven strings, asked the question the hex cursor
    // asks: what is under this bit, in the middle of ten thousand elements.
    let n = 20_000u64;
    let mut bytes = n.to_le_bytes().to_vec();
    let mut starts = vec![8u64];
    for i in 0..n {
        let s = "y".repeat((i % 13) as usize + 1);
        bytes.extend_from_slice(&(s.len() as u64).to_le_bytes());
        bytes.extend_from_slice(s.as_bytes());
        starts.push(bytes.len() as u64);
    }
    let string = T::structure("String", vec![("len", T::u64(Little)), ("text", T::utf8(E::field("len")))]);
    let t = Template::new(
        "t",
        T::structure("Root", vec![("n", T::u64(Little)), ("items", T::array(string, E::field("n")))]),
    );
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);
    ev.node(&d, &[1]).unwrap();

    // A bit inside element 12,345: the length, and then a byte of its text.
    let elem = 12_345usize;
    assert_eq!(ev.locate(&d, starts[elem] * 8).unwrap(), vec![1, elem, 0]);
    assert_eq!(ev.locate(&d, (starts[elem] + 9) * 8).unwrap(), vec![1, elem, 1]);
    // The answer came from a walk between checkpoints, not from putting twelve
    // thousand elements back in the memo.
    assert!(ev.memo_len() < 5_000, "locating kept {} nodes", ev.memo_len());
}

#[test]
fn an_overwrite_keeps_what_it_could_not_have_changed() {
    // A count, then that many strings of uneven length, then a run of bytes
    // standing in for the part of a file an edit usually lands in.
    let n = 20_000u64;
    let mut bytes = n.to_le_bytes().to_vec();
    for i in 0..n {
        let s = "z".repeat((i % 11) as usize + 1);
        bytes.extend_from_slice(&(s.len() as u64).to_le_bytes());
        bytes.extend_from_slice(s.as_bytes());
    }
    let list_end = bytes.len() as u64;
    bytes.extend_from_slice(&[0x11; 64]);
    let string = T::structure("String", vec![("len", T::u64(Little)), ("text", T::utf8(E::field("len")))]);
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("n", T::u64(Little)),
                ("items", T::array(string, E::field("n"))),
                ("tail", T::bytes(E::Remaining)),
            ],
        ),
    );
    let mut d = doc(&bytes);
    let mut ev = Evaluator::new(t);
    assert_eq!(ev.node(&d, &[2]).unwrap().offset_bits, list_end * 8);
    let after_sizing = ev.memo_len();

    // An overwrite in the tail: everything the walk learned about the list
    // still holds, so asking where the tail starts does not walk it again.
    d.overwrite_bytes(list_end + 8, &[0x22]);
    ev.invalidate_from((list_end + 8) * 8);
    assert_eq!(ev.memo.without_parent(), Vec::<Vec<usize>>::new());
    assert!(ev.memo_len() >= after_sizing / 2, "the edit threw away work it did not have to");
    assert_eq!(ev.node(&d, &[2]).unwrap().offset_bits, list_end * 8);
    assert!(ev.memo_len() < 200, "and it is still not the whole list: {}", ev.memo_len());

    // The edited bytes read as what was written, and a field before the edit
    // still reads as what the file says.
    let tail = ev.node(&d, &[2]).unwrap();
    let Value::Bytes { preview, .. } = tail.value else { panic!("not bytes") };
    assert_eq!(preview[8], 0x22);
    assert_eq!(ev.node(&d, &[1, 5, 1]).unwrap().value, Value::Str("zzzzzz".into()));

    // An overwrite inside the list drops what came after it and keeps the rest.
    let kept = ev.memo_len();
    ev.invalidate_from(8 * 8);
    assert_eq!(ev.memo.without_parent(), Vec::<Vec<usize>>::new());
    assert!(ev.memo_len() < kept);
    assert_eq!(ev.node(&d, &[2]).unwrap().offset_bits, list_end * 8);
}

#[test]
fn an_edit_that_moves_bytes_throws_the_whole_memo_away() {
    let bytes = [4u8, 0, 0, 0, 1, 2, 3, 4];
    let t = Template::new(
        "t",
        T::structure("Root", vec![("n", T::u32(Little)), ("data", T::bytes(E::field("n")))]),
    );
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);
    ev.node(&d, &[1]).unwrap();
    assert!(ev.memo_len() > 0);
    ev.invalidate();
    assert_eq!(ev.memo_len(), 0);
}

#[test]
fn work_done_in_goes_reaches_the_same_answer() {
    // A list long enough that a small allowance cannot finish it in one go.
    let n = 20_000u64;
    let mut bytes = n.to_le_bytes().to_vec();
    for i in 0..n {
        let s = "w".repeat((i % 7) as usize + 1);
        bytes.extend_from_slice(&(s.len() as u64).to_le_bytes());
        bytes.extend_from_slice(s.as_bytes());
    }
    let end = bytes.len() as u64;
    let string = T::structure("String", vec![("len", T::u64(Little)), ("text", T::utf8(E::field("len")))]);
    let t = Template::new(
        "t",
        T::structure("Root", vec![("n", T::u64(Little)), ("items", T::array(string, E::field("n")))]),
    );
    let d = doc(&bytes);

    let mut whole = Evaluator::new(t.clone());
    let want = whole.node(&d, &[1]).unwrap();
    assert_eq!(want.offset_bits + want.size_bits, end * 8);

    // The same question, answered five hundred elements at a time.
    let mut sliced = Evaluator::new(t);
    sliced.set_slice(Some(500));
    let mut goes = 0;
    let got = loop {
        goes += 1;
        assert!(goes < 500, "asking again is not getting anywhere");
        sliced.begin_slice();
        match sliced.node(&d, &[1]) {
            Ok(node) => break node,
            Err(EvalError::Busy { reached_bits }) => {
                // Each go says how far it has got, and it only ever goes forward.
                assert!(reached_bits <= end * 8);
                assert_eq!(reached_bits, sliced.reached_bits());
                let estimate = sliced.extent_estimate().expect("the array walk has a projection");
                assert_eq!(estimate.path, vec![1]);
                assert_eq!(estimate.total_items, n);
                assert!(estimate.measured_items > 0 && estimate.measured_items < n);
                // The elements vary from nine to fifteen bytes. An average of
                // the prefix should stay comfortably around the actual total.
                assert!(estimate.estimated_bits > want.size_bits / 2);
                assert!(estimate.estimated_bits < want.size_bits * 2);
            }
            Err(e) => panic!("{e:?}"),
        }
    };
    assert!(goes > 5, "a small allowance should have taken several goes, took {goes}");
    assert_eq!(got.offset_bits, want.offset_bits);
    assert_eq!(got.size_bits, want.size_bits);
    assert_eq!(got.child_count, want.child_count);
    assert!(sliced.memo_len() < 500, "and it is still bounded: {}", sliced.memo_len());

    // Reading one element after the goes is the same either way.
    sliced.begin_slice();
    let mut mid = loop {
        sliced.begin_slice();
        match sliced.node(&d, &[1, 9_999, 1]) {
            Ok(node) => break node,
            Err(EvalError::Busy { .. }) => continue,
            Err(e) => panic!("{e:?}"),
        }
    };
    let want_mid = whole.node(&d, &[1, 9_999, 1]).unwrap();
    mid.path.clone_from(&want_mid.path);
    assert_eq!(mid.value, want_mid.value);
    assert_eq!(mid.offset_bits, want_mid.offset_bits);
}

#[test]
fn the_chunk_read_longest_ago_is_the_one_that_goes() {
    use crate::source::ChunkStore;
    // Room for two chunks. The first is read again and again; the second is
    // loaded and left alone. Loading a third must take the idle one.
    let mut store = ChunkStore::new(3 * 8, 8, 2);
    store.insert(0, vec![1u8; 8].into_boxed_slice());
    store.insert(1, vec![2u8; 8].into_boxed_slice());
    let mut buf = [0u8; 8];
    for _ in 0..3 {
        assert!(store.read_bytes(0, &mut buf).is_empty());
    }
    store.insert(2, vec![3u8; 8].into_boxed_slice());
    assert!(store.has(0), "the chunk being read is the one to keep");
    assert!(!store.has(1), "the one nothing has looked at since it arrived goes");
    assert!(store.has(2));
}

#[test]
fn a_signature_reads_as_the_string_it_is() {
    let t = Template::new("t", T::structure("Root", vec![("magic", T::magic(b"\x89PNG\r\n\x1a\n"))]));
    let mut ev = Evaluator::new(t);
    let d = doc(b"\x89PNG\r\n\x1a\n");
    assert_eq!(listing::brief(&ev.node(&d, &[0]).unwrap().value), r#""\x89PNG\r\n\x1a\n""#);

    // The bytes that are there, and the bytes that were wanted. A signature
    // that is wrong is only worth reading beside the one it should have been.
    let wrong = doc(b"\x89PNh\r\n\x1a\n");
    let mut ev = Evaluator::new(Template::new("t", T::structure("Root", vec![("magic", T::magic(b"\x89PNG\r\n\x1a\n"))])));
    let node = ev.node(&wrong, &[0]).unwrap();
    assert_eq!(
        listing::brief(&node.value),
        r#""\x89PNh\r\n\x1a\n" does not match "\x89PNG\r\n\x1a\n""#
    );
    // The expected bytes are on the value, not only in the template, which is
    // what lets anything holding one say what was wanted.
    assert_eq!(
        node.value,
        Value::Magic {
            ok: false,
            bytes: b"\x89PNh\r\n\x1a\n".to_vec(),
            expected: b"\x89PNG\r\n\x1a\n".to_vec()
        }
    );
}

#[test]
fn digits_read_as_the_number_they_spell() {
    use crate::template::StrLen;
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("width", T::decimal(StrLen::Fixed(E::lit(3)))),
                ("count", T::decimal(StrLen::Scan { skip: b" ".to_vec(), ends: b" ".to_vec(), comment: None })),
                // The proof that it is a number and not its bytes: as bytes
                // the same field is 0x3132, which is 12,594 and past the end.
                ("body", T::bytes(E::field("count"))),
            ],
        ),
    );
    let d = doc(b"007  12 abcdefghijkl");
    let mut ev = Evaluator::new(t);
    let width = ev.node(&d, &[0]).unwrap();
    assert_eq!(width.value, Value::Int(7));
    assert_eq!(width.size_bits, 3 * 8);
    let count = ev.node(&d, &[1]).unwrap();
    assert_eq!(count.value, Value::Int(12));
    // Two spaces, the digits, and the space that ends them; the value is only
    // the digits, as it is for text.
    assert_eq!(count.size_bits, 5 * 8);
    assert_eq!(count.value_offset_bits, 5 * 8);
    assert_eq!(ev.node(&d, &[2]).unwrap().size_bits, 12 * 8);
    // Read only: writing one would have to decide how wide to pad it.
    assert!(!count.editable);
}

#[test]
fn digits_that_are_not_digits_are_an_error_rather_than_a_number() {
    use crate::template::StrLen;
    let t = Template::new("t", T::structure("Root", vec![("n", T::decimal(StrLen::Fixed(E::lit(3))))]));
    let mut ev = Evaluator::new(t);
    assert!(ev.node(&doc(b"1x2"), &[0]).is_err());
}

#[test]
fn a_run_can_be_measured_to_a_word_rather_than_to_a_byte() {
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("body", T::bytes(E::to_bytes(b"end"))),
                ("end", T::magic(b"end")),
                ("rest", T::bytes(E::Remaining)),
            ],
        ),
    );
    let mut ev = Evaluator::new(t.clone());
    let d = doc(b"ee en enend!");
    // Not the `e` of `ee`, nor the `en` of `enen`: the whole word or nothing.
    assert_eq!(ev.node(&d, &[0]).unwrap().size_bits, 8 * 8);
    assert_eq!(ev.node(&d, &[2]).unwrap().size_bits, 8);

    // A word that never comes measures to the end of the container, so a file
    // cut off in the middle still shows what it has.
    let mut ev = Evaluator::new(t);
    assert_eq!(ev.node(&doc(b"eeeee"), &[0]).unwrap().size_bits, 5 * 8);
}

#[test]
fn the_last_of_a_word_is_found_wherever_it_is() {
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![("head", T::computed(E::to_last_bytes(b"go"))), ("all", T::bytes(E::Remaining))],
        ),
    );
    let mut ev = Evaluator::new(t.clone());
    assert_eq!(ev.node(&doc(b"go..go..go.."), &[0]).unwrap().value, Value::Int(8));

    // Across the seam between two blocks, where half the word is read in one
    // and half in the next.
    let mut bytes = vec![b'.'; 4095];
    bytes.extend_from_slice(b"go");
    bytes.extend_from_slice(&[b'.'; 10]);
    let mut ev2 = Evaluator::new(t);
    assert_eq!(ev2.node(&doc(&bytes), &[0]).unwrap().value, Value::Int(4095));

    // Backward too, where the seam falls a block up from the end of the file
    // rather than a block down from the front of it.
    let mut bytes = vec![b'.'; 4196];
    bytes[0..2].copy_from_slice(b"go");
    bytes[99..101].copy_from_slice(b"go");
    let mut ev3 = Evaluator::new(Template::new(
        "t",
        T::structure("Root", vec![("head", T::computed(E::to_last_bytes(b"go")))]),
    ));
    assert_eq!(ev3.node(&doc(&bytes), &[0]).unwrap().value, Value::Int(99));
}

/// The last of a word is read from the end of the file, not found by reading
/// the whole of it.
///
/// A reader that holds a window rather than the file cannot do the second.
/// Every block it has not got stops the walk and is fetched, the walk starts
/// again from where it started, and the blocks it read first have been dropped
/// to make room for the ones it read last: the front of the file is fetched
/// again, evicts the back, and the walk gets no further than it did before. It
/// is not slow, it does not finish. Which is what a PDF asks for, since the
/// pointer to its table is written at the end and looked for by this.
///
/// Room here is four blocks of a forty-block file, so a walk from the front
/// would run out of it after a tenth of the way.
#[test]
fn the_last_of_a_word_is_read_from_the_end_of_a_file_that_arrives_in_pieces() {
    use crate::source::ChunkStore;
    const CHUNK: u64 = 4096;
    const CHUNKS: u64 = 40;
    let mut bytes = vec![b'.'; (CHUNK * CHUNKS) as usize];
    let at = bytes.len() - 20;
    bytes[at..at + 9].copy_from_slice(b"startxref");

    let mut d = Document::new(ChunkStore::new(bytes.len() as u64, CHUNK, 4));
    let mut ev = Evaluator::new(Template::new(
        "t",
        T::structure("Root", vec![("head", T::computed(E::to_last_bytes(b"startxref")))]),
    ));

    // The host's loop: what it was asked for is fetched, and the question is
    // asked again.
    let mut fetched = 0;
    let value = loop {
        match ev.node(&d, &[0]) {
            Ok(n) => break n.value,
            Err(EvalError::Pending(missing)) => {
                for m in missing {
                    let from = (m.chunk * CHUNK) as usize;
                    let to = (from + CHUNK as usize).min(bytes.len());
                    d.source_mut().insert(m.chunk, bytes[from..to].to_vec().into_boxed_slice());
                    fetched += 1;
                    assert!(fetched < 200, "asking for blocks that keep being dropped again");
                }
            }
            Err(e) => panic!("{e:?}"),
        }
    };
    assert_eq!(value, Value::Int(at as i128));
    assert!(fetched <= 4, "the end of the file is all that is read: {fetched} blocks fetched");
}
/// `Less` is one or zero, and `Or` after it is what makes that a choice: the
/// look-ahead on its right is never read while the left says there is no room
/// for one.
#[test]
fn less_than_answers_one_or_zero_and_or_stops_at_the_one() {
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("head", T::u8()),
                ("short", T::computed(E::Remaining.less_than(E::lit(4)))),
                ("long", T::computed(E::lit(4).less_than(E::Remaining))),
                // The right side reads the byte four along, which a two-byte
                // file does not have. It is never asked for there, because
                // the left side has already answered one.
                ("guarded", T::computed(E::Remaining.less_than(E::lit(4)).or(E::peek_at(E::lit(4 * 8), 8, Big)))),
            ],
        ),
    );
    // One byte left after the head: too short, and the peek is not made.
    let mut ev = Evaluator::new(t.clone());
    let d = doc(b"ab");
    assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::Int(1));
    assert_eq!(ev.node(&d, &[2]).unwrap().value, Value::Int(0));
    assert_eq!(ev.node(&d, &[3]).unwrap().value, Value::Int(1));

    // Five bytes left: room for the peek, which reads the byte it found.
    let mut ev = Evaluator::new(t);
    let d = doc(b"abcdef");
    assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::Int(0));
    assert_eq!(ev.node(&d, &[2]).unwrap().value, Value::Int(1));
    assert_eq!(ev.node(&d, &[3]).unwrap().value, Value::Int(i128::from(b'f')));

    // Equal is not less, either way round.
    let both = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("head", T::u8()),
                ("short", T::computed(E::Remaining.less_than(E::lit(4)))),
                ("long", T::computed(E::lit(4).less_than(E::Remaining))),
            ],
        ),
    );
    let mut ev = Evaluator::new(both);
    let d = doc(b"abcde");
    assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::Int(0));
    assert_eq!(ev.node(&d, &[2]).unwrap().value, Value::Int(0));

    // Negative numbers compare as numbers, not as the bytes they came from.
    let mut ev = Evaluator::new(Template::new(
        "t",
        T::structure("Root", vec![("n", T::computed(E::lit(-3).less_than(E::lit(2))))]),
    ));
    assert_eq!(ev.node(&doc(b"x"), &[0]).unwrap().value, Value::Int(1));
}

/// A field beside this one, and a path down into it. `Ref` stops at the field:
/// an HDF5 attribute writes the datatype of its own value inside itself, and
/// how wide one element is, is a field of that datatype rather than a field of
/// the attribute.
#[test]
fn an_expression_can_read_a_field_inside_the_field_beside_it() {
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("kind", T::structure("Kind", vec![("tag", T::u8()), ("width", T::u8())])),
                ("count", T::u8()),
                ("values", T::array(T::bytes(E::within(&["kind", "width"])), E::field("count"))),
            ],
        ),
    );
    let mut ev = Evaluator::new(t);
    let d = doc(&[7, 2, 3, 0xa0, 0xa1, 0xb0, 0xb1, 0xc0, 0xc1]);
    let values = ev.node(&d, &[2]).unwrap();
    assert_eq!(values.child_count, 3);
    assert_eq!(ev.node(&d, &[2, 2]).unwrap().offset_bits, 7 * 8);
    assert_eq!(ev.node(&d, &[2, 2]).unwrap().size_bits, 16);

    // A path that names nothing is an error on that field and not a wrong
    // number that reads as if it were right.
    let bad = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("kind", T::structure("Kind", vec![("tag", T::u8())])),
                ("value", T::bytes(E::within(&["kind", "width"]))),
            ],
        ),
    );
    let mut ev = Evaluator::new(bad);
    assert!(ev.node(&doc(&[1, 2, 3]), &[1]).is_err());
}

/// Padding to a boundary, including the case every hand-written version of
/// this arithmetic gets wrong: a run that already ends on one is followed by
/// no padding rather than by a whole unit of it.
#[test]
fn padding_measures_to_the_next_boundary_and_no_further() {
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("len", T::u8()),
                ("value", T::bytes(E::field("len"))),
                ("padding", T::bytes(E::field("len").pad_to(4))),
                ("after", T::u8()),
            ],
        ),
    );
    let mut ev = Evaluator::new(t.clone());
    // Five bytes of value, so three of padding, and the byte after them.
    let d = doc(&[5, 1, 2, 3, 4, 5, 0, 0, 0, 0xaa]);
    assert_eq!(ev.node(&d, &[2]).unwrap().size_bits, 3 * 8);
    assert_eq!(ev.node(&d, &[3]).unwrap().value, Value::UInt(0xaa));
    // Four bytes of value ends on the boundary, so nothing follows it.
    let mut ev = Evaluator::new(t);
    let d = doc(&[4, 1, 2, 3, 4, 0xbb]);
    assert_eq!(ev.node(&d, &[2]).unwrap().size_bits, 0);
    assert_eq!(ev.node(&d, &[3]).unwrap().value, Value::UInt(0xbb));
}

/// A field read only while there is room for it, which is how a header that
/// grew a field at a time is read by whoever wrote it.
#[test]
fn a_field_with_no_room_left_is_not_read_at_all() {
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("size", T::u8()),
                (
                    "header",
                    T::sized(
                        E::field("size"),
                        T::structure(
                            "Header",
                            vec![("a", T::u16(Big)), ("b", T::if_room(T::u16(Big))), ("c", T::if_room(T::u32(Big)))],
                        ),
                    ),
                ),
            ],
        ),
    );
    // Four bytes of header: `b` is there and `c` is not.
    let mut ev = Evaluator::new(t.clone());
    let d = doc(&[4, 0, 1, 0, 2, 9, 9, 9, 9]);
    assert_eq!(ev.node(&d, &[1, 1]).unwrap().value, Value::UInt(2));
    assert_eq!(ev.node(&d, &[1, 2]).unwrap().size_bits, 0);
    // Two bytes, and neither of them is read from the bytes after the window.
    let mut ev = Evaluator::new(t);
    let d = doc(&[2, 0, 1, 9, 9, 9, 9]);
    assert_eq!(ev.node(&d, &[1, 0]).unwrap().value, Value::UInt(1));
    assert_eq!(ev.node(&d, &[1, 1]).unwrap().size_bits, 0);
    assert_eq!(ev.node(&d, &[1, 2]).unwrap().size_bits, 0);
}

#[test]
fn nesting_past_the_limit_is_an_error_rather_than_a_crash() {
    // A CBOR array holding an array holding an array, three hundred times over.
    // Every one of them is well formed, and the file is 301 bytes: nothing
    // about it is large except how far down its last value is.
    let t = crate::formats::builtin("cbor").expect("cbor is built in");
    let mut bytes = vec![0x81; 300];
    bytes.push(0x01);
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t.clone());
    let Err(EvalError::Failed(msg)) = ev.node(&d, &[]) else {
        panic!("a file nested past the limit should say so");
    };
    assert!(msg.contains("nested more than"), "{msg}");

    // Nesting a file does reach is read to the bottom: twenty arrays, and the
    // number they hold is the number that was put there.
    let mut bytes = vec![0x81; 20];
    bytes.push(0x07);
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);
    let mut path = Vec::new();
    for _ in 0..20 {
        path.extend_from_slice(&[3, 0]);
    }
    path.push(0);
    assert_eq!(ev.node(&d, &path).unwrap().value, Value::Enum { raw: 7, name: None, hex: true });
}


#[test]
fn a_run_that_holds_a_run_is_refused_at_the_same_depth() {
    // The other shape a file nests in: not a list of lists, whose length is
    // arithmetic, but a run that stops on what it reads and so is walked. This
    // is how bencode nests, and it costs the stack three times as much per
    // level, so it is the shape the limit is set by.
    let item = T::structure(
        "Item",
        vec![
            ("tag", T::u8()),
            ("kids", T::repeat(T::Named("Item".into()), Until::FieldBytes { field: "tag".into(), bytes: vec![b'e'] })),
        ],
    );
    let t = Template::new("nest", T::Named("Item".into())).with_type("Item", item);
    let mut bytes = vec![b'd'; 300];
    bytes.extend(std::iter::repeat_n(b'e', 301));
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);
    let Err(EvalError::Failed(msg)) = ev.node(&d, &[]) else {
        panic!("a run nested past the limit should say so");
    };
    assert!(msg.contains("nested more than"), "{msg}");
}

#[test]
fn a_read_with_no_stack_left_stops_rather_than_the_process() {
    // The backstop behind the depth count, which nothing measured reaches:
    // both known shapes are stopped by the count first. It is reached here by
    // telling the evaluator the stack started further up than it did, which is
    // what a shape costing more per field than any measured one would do.
    let t = crate::formats::builtin("cbor").expect("cbor is built in");
    let d = doc(&[0x81, 0x81, 0x01]);
    let mut ev = Evaluator::new(t);
    ev.go.pretend_out_of_room();
    let Err(EvalError::Failed(msg)) = ev.node(&d, &[]) else {
        panic!("a read with no room left should say so");
    };
    assert!(msg.contains("too deep to read"), "{msg}");
}

#[test]
fn low_bit_first_fields_sit_at_the_bottom_of_the_byte() {
    // A Zig packed struct, and the same shape a DEFLATE block header has:
    // declared front to back, written bottom to top. The byte is
    // 0b1_101_10_1_1, so `final` is the low bit and `window` is the three
    // below the spare one at the top.
    let t = Template::new(
        "t",
        T::structure(
            "Header",
            vec![
                ("final", T::UInt { bits: 1, endian: Little }),
                ("kind", T::UInt { bits: 1, endian: Little }),
                ("level", T::UInt { bits: 2, endian: Little }),
                ("window", T::UInt { bits: 3, endian: Little }),
                ("spare", T::UInt { bits: 1, endian: Little }),
                ("len", T::u16(Little)),
            ],
        ),
    );
    let d = doc(&[0b1_101_10_1_1, 0x34, 0x12]);
    let mut ev = Evaluator::new(t);
    assert_eq!(ev.node(&d, &[0]).unwrap().value, Value::UInt(1));
    assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::UInt(1));
    assert_eq!(ev.node(&d, &[2]).unwrap().value, Value::UInt(0b10));
    assert_eq!(ev.node(&d, &[3]).unwrap().value, Value::UInt(0b101));
    assert_eq!(ev.node(&d, &[4]).unwrap().value, Value::UInt(1));
    // Whole bytes on a byte boundary are byte order and nothing new.
    assert_eq!(ev.node(&d, &[5]).unwrap().value, Value::UInt(0x1234));

    // The bits are where the value says they are: `final` is the last bit of
    // the byte, not the first, and the cursor lands on it there.
    assert_eq!(ev.node(&d, &[0]).unwrap().offset_bits, 7);
    assert_eq!(ev.node(&d, &[3]).unwrap().offset_bits, 1);
    assert_eq!(ev.locate(&d, 7).unwrap(), vec![0]);
    assert_eq!(ev.locate(&d, 1).unwrap(), vec![3]);

    // The same eight bits packed the way this IR always packed them, to make
    // the difference the endian makes visible.
    let msb = Template::new(
        "t",
        T::structure("Header", vec![("first", T::UInt { bits: 3, endian: Big }), ("rest", T::UInt { bits: 5, endian: Big })]),
    );
    let mut ev = Evaluator::new(msb);
    assert_eq!(ev.node(&d, &[0]).unwrap().value, Value::UInt(0b110));
    assert_eq!(ev.node(&d, &[0]).unwrap().offset_bits, 0);
}

#[test]
fn a_low_bit_first_field_is_written_back_where_it_was_read() {
    let t = Template::new(
        "t",
        T::structure(
            "Header",
            vec![("low", T::UInt { bits: 3, endian: Little }), ("high", T::UInt { bits: 5, endian: Little })],
        ),
    );
    let mut d = doc(&[0b00000_001]);
    let mut ev = Evaluator::new(t);
    assert_eq!(ev.node(&d, &[0]).unwrap().value, Value::UInt(1));
    assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::UInt(0));

    for (path, text) in [(vec![0], "5"), (vec![1], "31")] {
        let w = ev.prepare_write(&d, &path, text).unwrap();
        d.overwrite_bits(w.offset_bits, &w.data, w.n_bits);
        ev.invalidate();
    }
    let mut out = [0u8; 1];
    d.read_bytes(0, &mut out);
    assert_eq!(out, [0b11111_101]);
    assert_eq!(ev.node(&d, &[0]).unwrap().value, Value::UInt(5));
    assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::UInt(31));
}

#[test]
fn a_low_bit_first_field_across_a_byte_boundary_is_refused() {
    // Twelve bits from the bottom of one byte are all of that byte and half of
    // the next, which no single range of a bit address numbered from the top
    // of each byte can name. Better said than placed somewhere it is not.
    let t = Template::new("t", T::structure("R", vec![("wide", T::UInt { bits: 12, endian: Little })]));
    let d = doc(&[0xff, 0xff]);
    let mut ev = Evaluator::new(t);
    let Err(EvalError::Failed(msg)) = ev.node(&d, &[0]) else {
        panic!("a field that cannot be placed should say so");
    };
    assert!(msg.contains("cross a byte boundary"), "{msg}");
}

// ----- sign and magnitude -----

#[test]
fn a_sign_magnitude_number_is_not_twos_complement() {
    // The same two bytes read three ways. 0x8005 is -5 with the sign bit and
    // a magnitude of five; as two's complement the same bytes are -32763,
    // which is a plausible-looking number and wrong by the width of the field.
    let t = Template::new(
        "t",
        T::structure(
            "R",
            vec![
                ("south", T::sign_magnitude(16, Big)),
                ("north", T::sign_magnitude(16, Big)),
                ("as_int", T::at(E::lit(0), T::Int { bits: 16, endian: Big })),
            ],
        ),
    );
    let d = doc(&[0x80, 0x05, 0x00, 0x05]);
    let mut ev = Evaluator::new(t);
    assert_eq!(ev.node(&d, &[0]).unwrap().value, Value::Int(-5));
    assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::Int(5));
    assert_eq!(ev.node(&d, &[2, 0]).unwrap().value, Value::Int(-32763));
    // The type column says which of the two readings this is.
    assert_eq!(ev.node(&d, &[0]).unwrap().type_name, "sm16 be");
}

#[test]
fn negative_zero_is_zero_and_the_range_is_symmetrical() {
    let t = Template::new("t", T::structure("R", vec![("v", T::sign_magnitude(8, Big))]));
    let mut d = doc(&[0x80]);
    let mut ev = Evaluator::new(t);
    assert_eq!(ev.node(&d, &[0]).unwrap().value, Value::Int(0), "negative zero is zero");
    // Written back, an eight-bit field reaches -127 and no further: the sign
    // costs a bit that two's complement spends on one more negative number.
    let w = ev.prepare_write(&d, &[0], "-127").unwrap();
    d.overwrite_bits(w.offset_bits, &w.data, w.n_bits);
    ev.invalidate();
    assert_eq!(ev.node(&d, &[0]).unwrap().value, Value::Int(-127));
    assert!(ev.prepare_write(&d, &[0], "-128").is_err());
}

// ----- a width read from the file -----

#[test]
fn a_number_is_as_wide_as_an_earlier_field_says() {
    // Three values of eleven bits each, packed one after another, after the
    // byte that says eleven: 0b00000000001_00000000010_00000000011.
    let t = Template::new(
        "t",
        T::structure(
            "R",
            vec![
                ("bits_per_value", T::u8()),
                ("values", T::array(T::uint_expr(E::field("bits_per_value"), Big), E::lit(3))),
            ],
        ),
    );
    let d = doc(&[11, 0b0000_0000, 0b0010_0000, 0b0000_1000, 0b0000_0001, 0b1000_0000]);
    let mut ev = Evaluator::new(t);
    assert_eq!(ev.node(&d, &[1, 0]).unwrap().value, Value::UInt(1));
    assert_eq!(ev.node(&d, &[1, 1]).unwrap().value, Value::UInt(2));
    assert_eq!(ev.node(&d, &[1, 2]).unwrap().value, Value::UInt(3));
    // Each value is as wide as the header said, and the run is that times the
    // count: the list is measured by arithmetic, not by walking it.
    assert_eq!(ev.node(&d, &[1, 0]).unwrap().size_bits, 11);
    assert_eq!(ev.node(&d, &[1]).unwrap().size_bits, 33);
    // The type column names the field that decided the width.
    assert_eq!(ev.node(&d, &[1, 0]).unwrap().type_name, "u bits_per_value be");
    // And the connection is exposed: the reader can go to the field that
    // settled it.
    let origins = ev.origins(&d, &[1, 0]).unwrap();
    let width = origins.iter().find(|o| o.role == Role::Width).expect("a width has an origin");
    assert_eq!((width.label.as_str(), width.value.as_str(), width.path.as_slice()), ("bits_per_value", "11", &[0][..]));
}

#[test]
fn a_width_of_no_bits_is_a_value_of_no_bits() {
    // A GRIB whose values are all the same writes a width of zero and no data
    // at all. The values are still there to be counted; they are all zero.
    let t = Template::new(
        "t",
        T::structure(
            "R",
            vec![("w", T::u8()), ("values", T::array(T::uint_expr(E::field("w"), Big), E::lit(4)))],
        ),
    );
    let d = doc(&[0]);
    let mut ev = Evaluator::new(t);
    assert_eq!(ev.node(&d, &[1]).unwrap().size_bits, 0);
    assert_eq!(ev.node(&d, &[1, 3]).unwrap().value, Value::UInt(0));
}

#[test]
fn a_width_the_template_cannot_place_is_refused() {
    // Packed from the bottom of the byte, at a width nothing knows until the
    // field has been read: where it goes cannot be settled, so it is refused
    // rather than placed somewhere it is not.
    let t = Template::new(
        "t",
        T::structure("R", vec![("w", T::u8()), ("v", T::uint_expr(E::field("w"), Little))]),
    );
    let d = doc(&[11, 0xff, 0xff]);
    let mut ev = Evaluator::new(t);
    let Err(EvalError::Failed(msg)) = ev.node(&d, &[1]) else { panic!("should refuse") };
    assert!(msg.contains("low-bit-first"), "{msg}");
}

// ----- finding a sibling by a key this record works out -----

/// A stream of records where each one carries a class number, and what that
/// class is called is written in an earlier record of the same stream. This is
/// the shape GWF has, and the one no name can reach: the records are elements
/// of a list, not fields declared beside the field asking.
fn classed() -> Template {
    let record = T::structure(
        "Rec",
        vec![
            ("class", T::u8()),
            ("class_num", T::u8()),
            ("name", T::utf8(E::lit(4))),
            // The name of whichever earlier record numbered itself with this
            // record's class byte.
            ("of_class", T::Computed(E::sibling_tagged(&["class_num"], E::field("class"), &["class_num"]))),
        ],
    );
    Template::new("t", T::repeat(record, Until::End))
}

#[test]
fn a_record_finds_the_earlier_one_that_defines_its_class() {
    // Two definitions and then a record of class 9.
    let d = doc(b"\x00\x08dict\x00\x09trce\x09\x00data");
    let mut ev = Evaluator::new(classed());
    // The third record's class byte is 9, and the record that numbered itself
    // 9 is the second one.
    assert_eq!(ev.node(&d, &[2, 3]).unwrap().value.as_int(), Some(9));
    // Which is a connection, not a coincidence: the reader is pointed at the
    // element the answer came from, and at the byte that sent it there.
    let origins = ev.origins(&d, &[2, 3]).unwrap();
    let labels: Vec<&str> = origins.iter().map(|o| o.label.as_str()).collect();
    // Named for the list it was found in, which here is the whole file.
    assert_eq!(labels, vec!["class", "file[1].class_num"]);
    assert_eq!(origins[1].path, vec![1, 1]);
    // And the relationship is written out both ways.
    let rel = ev.relations(&d, &[2, 3]).unwrap();
    assert_eq!(rel[0].written, "earlier[class_num = class].class_num");
    assert_eq!(rel[0].substituted, "earlier[class_num = 9].class_num");
}

#[test]
fn a_lookup_that_finds_nothing_is_zero_rather_than_an_error() {
    // A record whose class nothing earlier defined. Zero, so `Or` can name
    // what to do without one, the same answer every other search here gives.
    let d = doc(b"\x07\x00data");
    let mut ev = Evaluator::new(classed());
    assert_eq!(ev.node(&d, &[0, 3]).unwrap().value.as_int(), Some(0));
}

#[test]
fn a_lookup_by_computed_key_reads_text_as_well_as_numbers() {
    // The same search, used to pick a type by the word an earlier record
    // holds rather than by a number. What a format that names its own record
    // types needs.
    let record = T::structure(
        "Rec",
        vec![
            ("class", T::u8()),
            ("class_num", T::u8()),
            ("name", T::utf8(E::lit(4))),
            (
                "body",
                T::matches(
                    E::sibling_tagged(&["class_num"], E::field("class"), &["name"]),
                    vec![("trce", T::u16(Big))],
                    T::bytes(E::lit(0)),
                ),
            ),
        ],
    );
    let d = doc(b"\x00\x08dict\x00\x09trce\x09\x00data\xbe\xef");
    let mut ev = Evaluator::new(Template::new("t", T::repeat(record, Until::End)));
    // The third record's class is 9, the record numbered 9 is called `trce`,
    // and a `trce` body is a sixteen-bit number.
    assert_eq!(ev.node(&d, &[2, 3]).unwrap().value, Value::UInt(0xbeef));
    // The first two are not, so their bodies are nothing at all.
    assert_eq!(ev.node(&d, &[0, 3]).unwrap().size_bits, 0);
}

/// A record: where the next one is, and a byte of its own.
fn linked() -> T {
    T::structure("Rec", vec![("next", T::u16(Big)), ("value", T::u8())])
}

#[test]
fn a_chain_of_pointers_is_a_flat_list() {
    // Three records, written in an order that has nothing to do with the order
    // they are read in: the header points at the last of them, which points
    // back at the first, which points at the middle one, which ends the chain.
    let t = T::structure(
        "Root",
        vec![("head", T::u16(Big)), ("recs", T::chain(E::field("head"), &["next"], Anchor::File, linked()))],
    );
    //          0..2 head=8      2..5 rec at 2      5..8 rec at 5     8..11 rec at 8
    let d = doc(&[0, 8, /* @2 */ 0, 5, 0xaa, /* @5 */ 0, 0, 0xbb, /* @8 */ 0, 2, 0xcc]);
    let mut ev = Evaluator::new(Template::new("t", t));
    let list = ev.node(&d, &[1]).unwrap();
    // Flat: three rows, not three levels.
    assert_eq!(list.child_count, 3);
    // The list itself covers no bytes; its elements are what cover them.
    assert_eq!(list.size_bits, 0);
    assert_eq!(list.type_name, "chain \u{2192} Rec");
    let value = |ev: &mut Evaluator, i: usize| ev.node(&d, &[1, i, 1]).unwrap().value.as_int();
    assert_eq!((value(&mut ev, 0), value(&mut ev, 1), value(&mut ev, 2)), (Some(0xcc), Some(0xaa), Some(0xbb)));
    // Read in the order the pointers give, not the order they sit in.
    assert_eq!(ev.node(&d, &[1, 0]).unwrap().offset_bits, 8 * 8);
    assert_eq!(ev.node(&d, &[1, 2]).unwrap().offset_bits, 5 * 8);
    // And the cursor finds them where they are, out of order and all. The
    // chain covers no bytes itself, so the search through the structure's
    // fields has to look inside it rather than at how long it is.
    assert_eq!(ev.locate(&d, 10 * 8).unwrap(), vec![1, 0, 1]);
    assert_eq!(ev.locate(&d, 3 * 8).unwrap(), vec![1, 1, 0]);
    assert_eq!(ev.locate(&d, 0).unwrap(), vec![0]);
}

#[test]
fn a_chain_stops_rather_than_going_round_for_ever() {
    let chain = |head: E| {
        T::structure("Root", vec![("head", T::u16(Big)), ("recs", T::chain(head, &["next"], Anchor::File, linked()))])
    };
    let count = |bytes: &[u8], head: E| {
        let d = doc(bytes);
        let mut ev = Evaluator::new(Template::new("t", chain(head)));
        ev.node(&d, &[1]).unwrap().child_count
    };
    // A record pointing at itself: one element, not for ever.
    assert_eq!(count(&[0, 2, 0, 2, 0xaa], E::field("head")), 1);
    // Two pointing at each other.
    assert_eq!(count(&[0, 2, 0, 5, 0xaa, 0, 2, 0xbb], E::field("head")), 2);
    // All ones for the width of the `next` field, which here is sixteen bits.
    assert_eq!(count(&[0, 2, 0xff, 0xff, 0xaa], E::field("head")), 1);
    // Past the end of the file.
    assert_eq!(count(&[0, 2, 0, 99, 0xaa], E::field("head")), 1);
    // A chain that starts nowhere is a list of nothing rather than an error.
    assert_eq!(count(&[0, 0, 0, 0, 0], E::field("head")), 0);
}

/// A chain whose offsets count from somewhere the bytes do not begin.
///
/// This is what a run of records unpacked out of the middle of a file needs:
/// every offset in a compressed CDF counts from the front of the file it was
/// before it was squeezed, and what comes out of the stream starts eight bytes
/// into that. The adjustment moves where each element is read and leaves the
/// tests that end the walk alone, so a nought still ends it.
#[test]
fn a_chain_reads_its_offsets_with_an_adjustment() {
    let t = T::structure(
        "Root",
        vec![
            ("head", T::u16(Big)),
            ("recs", T::chain_adjusted(E::field("head"), &["next"], Anchor::File, E::lit(-4), linked())),
        ],
    );
    // The offsets are written four too large: the head says 12 for the record
    // at 8, and that record says 6 for the one at 2.
    let d = doc(&[0, 12, /* @2 */ 0, 0, 0xaa, /* @5 */ 0, 0, 0, /* @8 */ 0, 6, 0xcc]);
    let mut ev = Evaluator::new(Template::new("t", t));
    assert_eq!(ev.node(&d, &[1]).unwrap().child_count, 2);
    assert_eq!(ev.node(&d, &[1, 0]).unwrap().offset_bits, 8 * 8);
    assert_eq!(ev.node(&d, &[1, 1]).unwrap().offset_bits, 2 * 8);
    assert_eq!(ev.node(&d, &[1, 1, 1]).unwrap().value, Value::UInt(0xaa));
    // A nought is still the end of the walk, and not the record four bytes
    // before the file: what the tests are made on is the offset as written.
    assert_eq!(ev.node(&d, &[1]).unwrap().child_count, 2);
}

/// A record that places one gathered element: where it is, and how long.
fn placing() -> T {
    T::structure("Rec", vec![("off", T::u8()), ("len", T::u8())])
}

/// Two groups of records, then a region the records place their elements in.
/// Each element is as long as the record that placed it says.
fn gathered(elem: T) -> T {
    let group = T::structure("Group", vec![("n", T::u8()), ("recs", T::array(placing(), E::field("n")))]);
    let from = vec![Step::field("groups"), Step::each(), Step::field("recs"), Step::each()];
    T::structure(
        "Root",
        vec![
            ("groups", T::array(group, E::lit(2))),
            ("region", T::sized(E::Remaining, T::gather(from, E::field("off"), Anchor::File, E::lit(0), elem))),
        ],
    )
}

/// Group 0 holds two records and group 1 one; the region runs from byte 8 to
/// byte 20, and the three elements sit in it out of order, with gaps between.
fn gathered_bytes() -> Vec<u8> {
    let mut b = vec![2, 14, 3, 10, 2, /* group 1 */ 1, 18, 1];
    //        8  9     10    11    12 13    14    15    16    17    18    19
    b.extend([0, 0, 0xa0, 0xa1, 0, 0, 0xb0, 0xb1, 0xb2, 0, 0xc0, 0]);
    b
}

#[test]
fn a_gather_places_elements_from_records_two_lists_deep() {
    let t = gathered(T::bytes(E::placer(E::field("len"))));
    let d = doc(&gathered_bytes());
    let mut ev = Evaluator::new(Template::new("t", t));
    let list = ev.node(&d, &[1]).unwrap();
    // Flat: one element per record, however deep the records were.
    assert_eq!(list.child_count, 3);
    assert_eq!(list.type_name, "descriptors \u{2192} bytes[]");
    // The region is what the window gave it, not what its elements come to.
    assert_eq!(list.size_bits, 12 * 8);
    // Numbered in the order the walk found the records, and placed where each
    // record said, which is not that order.
    let at = |ev: &mut Evaluator, i: usize| {
        let n = ev.node(&d, &[1, i]).unwrap();
        (n.offset_bits / 8, n.size_bits / 8)
    };
    assert_eq!((at(&mut ev, 0), at(&mut ev, 1), at(&mut ev, 2)), ((14, 3), (10, 2), (18, 1)));
    // The cursor finds each element where it is, and a byte of the region no
    // element claims is the region itself.
    assert_eq!(ev.locate(&d, 15 * 8).unwrap(), vec![1, 0]);
    assert_eq!(ev.locate(&d, 10 * 8).unwrap(), vec![1, 1]);
    assert_eq!(ev.locate(&d, 18 * 8).unwrap(), vec![1, 2]);
    assert_eq!(ev.locate(&d, 12 * 8).unwrap(), vec![1]);
    // A record is still where it was, and still a record.
    assert_eq!(ev.locate(&d, 3 * 8).unwrap(), vec![0, 0, 1, 1, 0]);
    assert_eq!(ev.shape(&d, &[1, 0]).unwrap().placed, Placed::Gathered);
    // The listing reads the region as the elements and the gaps between them.
    let spans = ev.spans(&d, 8 * 8, 20 * 8, 100).unwrap();
    let seen: Vec<(u64, u64, bool)> = spans.iter().map(|s| (s.offset_bits / 8, s.size_bits / 8, s.gap)).collect();
    assert_eq!(
        seen,
        vec![(8, 2, true), (10, 2, false), (12, 2, true), (14, 3, false), (17, 1, true), (18, 1, false), (19, 1, true)]
    );
}

#[test]
fn a_gathered_element_says_which_record_placed_it() {
    let t = gathered(T::bytes(E::placer(E::field("len"))));
    let d = doc(&gathered_bytes());
    let mut ev = Evaluator::new(Template::new("t", t));
    // The record first, named the way the walk reached it, and then the field
    // of it the offset was read from.
    let origins = ev.origins(&d, &[1, 1]).unwrap();
    let seen: Vec<(Role, &str, &[usize])> = origins.iter().map(|o| (o.role, o.label.as_str(), o.path.as_slice())).collect();
    assert_eq!(seen[0], (Role::Position, "groups[0].recs[1]", &[0, 0, 1, 1][..]));
    assert_eq!(seen[1], (Role::Position, "off", &[0, 0, 1, 1, 0][..]));
    assert_eq!(origins[1].value, "10");
    // And the length names the record's field too, not a field beside the
    // element, since the element has none.
    assert!(origins.iter().any(|o| o.role == Role::Length && o.path == vec![0, 0, 1, 1, 1]), "{origins:?}");
}

#[test]
fn a_placer_reads_the_record_that_placed_its_element() {
    // Each element is a structure asking its record two things: how long its
    // bytes are, and where the record said it was.
    let elem = T::structure(
        "Piece",
        vec![("bytes", T::bytes(E::placer(E::field("len")))), ("was_at", T::computed(E::placer(E::field("off"))))],
    );
    let d = doc(&gathered_bytes());
    let mut ev = Evaluator::new(Template::new("t", gathered(elem)));
    assert_eq!(ev.node(&d, &[1, 2, 1]).unwrap().value.as_int(), Some(18));
    assert_eq!(ev.node(&d, &[1, 0, 0]).unwrap().size_bits, 3 * 8);
    // Asked anywhere that no gather placed, there is no record to ask.
    let t = T::structure("Root", vec![("n", T::u8()), ("v", T::computed(E::placer(E::field("n"))))]);
    let mut ev = Evaluator::new(Template::new("t", t));
    assert!(ev.node(&d, &[1]).is_err());
}

#[test]
fn a_gather_finds_a_tagged_list_wherever_it_is_written() {
    // Two tables, each a number saying what it is and records. Only the
    // records of table 7 place anything, whichever of the two comes first.
    let table = T::structure(
        "Table",
        vec![("id", T::u8()), ("n", T::u8()), ("recs", T::array(placing(), E::field("n")))],
    );
    let from = vec![Step::field("tables"), Step::tagged(&["id"], 7, "sevens"), Step::field("recs"), Step::each()];
    let t = T::structure(
        "Root",
        vec![
            ("tables", T::array(table, E::lit(2))),
            ("region", T::sized(E::Remaining, T::gather(from, E::field("off"), Anchor::File, E::lit(0), T::bytes(E::placer(E::field("len")))))),
        ],
    );
    //                  table 3 at 0: one record    table 7 at 4: two records
    let first_three = [3, 1, 12, 1, 7, 2, 13, 2, 15, 1, 0, 0, 0xee, 0xaa, 0xaa, 0xbb];
    let first_seven = [7, 2, 13, 2, 15, 1, 3, 1, 12, 1, 0, 0, 0xee, 0xaa, 0xaa, 0xbb];
    for bytes in [first_three, first_seven] {
        let d = doc(&bytes);
        let mut ev = Evaluator::new(Template::new("t", t.clone()));
        assert_eq!(ev.node(&d, &[1]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[1, 0]).unwrap().offset_bits, 13 * 8);
        assert_eq!(ev.node(&d, &[1, 1]).unwrap().offset_bits, 15 * 8);
        // The byte table 3 pointed at belongs to nothing.
        assert_eq!(ev.locate(&d, 12 * 8).unwrap(), vec![1]);
        // Named by the step, not by the question it asked.
        let label = ev.origins(&d, &[1, 1]).unwrap()[0].label.clone();
        assert_eq!(label, "tables.sevens.recs[1]");
    }
}

#[test]
fn a_record_that_places_nothing_is_passed_over() {
    // A record holds an offset only when its kind says so. One of kind 1
    // points outside the region, which is passed over too, and so is an
    // offset of zero once the gather is told zero means nothing.
    let rec = T::structure(
        "Rec",
        vec![
            ("kind", T::u8()),
            ("body", T::switch(E::field("kind"), vec![(1, T::structure("Placed", vec![("off", T::u8())]))], T::bytes(E::lit(1)))),
        ],
    );
    let from = vec![Step::field("recs"), Step::each()];
    let region = |skip_zero: bool| {
        let g = T::gather(from.clone(), E::within(&["body", "off"]), Anchor::File, E::lit(0), T::u8());
        let g = if skip_zero { g.skipping_zero() } else { g };
        T::structure("Root", vec![("recs", T::array(rec.clone(), E::lit(5))), ("region", T::sized(E::Remaining, g))])
    };
    //          no offset   at 11     no offset  outside   at 0
    let bytes = [0, 9, 1, 11, 2, 9, 1, 99, 1, 0, 0xaa, 0xbb];
    let d = doc(&bytes);
    let mut ev = Evaluator::new(Template::new("t", region(false)));
    // Zero is a place in the file, though not one inside this region.
    assert_eq!(ev.node(&d, &[1]).unwrap().child_count, 1);
    assert_eq!(ev.node(&d, &[1, 0]).unwrap().value.as_int(), Some(0xbb));
    let mut ev = Evaluator::new(Template::new("t", region(true)));
    assert_eq!(ev.node(&d, &[1]).unwrap().child_count, 1);
}

/// Forty records, each placing a byte of the region after them, the last
/// record's first.
fn many_gathered() -> (T, Vec<u8>) {
    let from = vec![Step::field("recs"), Step::each()];
    let t = T::structure(
        "Root",
        vec![
            ("recs", T::array(placing(), E::lit(40))),
            ("region", T::sized(E::Remaining, T::gather(from, E::field("off"), Anchor::File, E::lit(0), T::bytes(E::placer(E::field("len")))))),
        ],
    );
    let mut b = Vec::new();
    for i in 0..40u8 {
        b.extend([80 + 2 * (39 - i), 1]);
    }
    b.extend((0..80u8).map(|i| if i % 2 == 0 { i } else { 0 }));
    (t, b)
}

#[test]
fn a_gather_carries_on_across_goes() {
    let (t, bytes) = many_gathered();
    let d = doc(&bytes);
    let mut whole = Evaluator::new(Template::new("t", t.clone()));
    let want: Vec<u64> = (0..40).map(|i| whole.node(&d, &[1, i]).unwrap().offset_bits).collect();
    let mut ev = Evaluator::new(Template::new("t", t));
    ev.set_slice(Some(8));
    let mut goes = 0;
    let n = loop {
        goes += 1;
        assert!(goes < 50, "the walk is not getting any further");
        ev.begin_slice();
        match ev.node(&d, &[1]) {
            Ok(info) => break info.child_count,
            Err(EvalError::Busy { .. }) => {}
            Err(e) => panic!("{e:?}"),
        }
    };
    // It did stop part way, and what it found in goes is what it finds in one.
    assert!(goes > 2, "{goes}");
    assert_eq!(n, 40);
    let got: Vec<u64> = (0..40).map(|i| ev.node(&d, &[1, i]).unwrap().offset_bits).collect();
    assert_eq!(got, want);
}

#[test]
fn a_whole_window_of_spans_over_a_gather_finishes_in_goes() {
    // `spans` starts again from the top of its window every go. A walk that
    // charged for going back over the records the last go reached would spend
    // each new go doing that, and never reach the end: bug B1, for a list walk.
    let (t, bytes) = many_gathered();
    let d = doc(&bytes);
    let mut ev = Evaluator::new(Template::new("t", t));
    ev.set_slice(Some(8));
    let mut goes = 0;
    let spans = loop {
        goes += 1;
        assert!(goes < 200, "spans over the gather never settled");
        ev.begin_slice();
        match ev.spans(&d, 0, d.len_bits(), 1000) {
            Ok(spans) => break spans,
            Err(EvalError::Busy { .. }) => {}
            Err(e) => panic!("{e:?}"),
        }
    };
    let region: Vec<&Span> = spans.iter().filter(|s| s.offset_bits >= 80 * 8).collect();
    // Forty bytes placed and forty between them, each its own entry.
    assert_eq!(region.iter().filter(|s| !s.gap).count(), 40);
    assert_eq!(region.iter().filter(|s| s.gap).count(), 40);
    assert_eq!(region.first().map(|s| s.path.clone()), Some(vec![1, 39]));
}

#[test]
fn a_lookup_can_be_keyed_by_text_found_somewhere_else() {
    // A table of definitions labelled in words, and records that say which
    // definition they follow by writing the word rather than a number. The key
    // field is padded to a fixed width and the pointing field is not, and the
    // two still match: what is compared is what the fields read as.
    let defs = T::array(
        T::structure("Def", vec![("name", T::utf8_padded(E::lit(6), b' ')), ("width", T::u8())]),
        E::lit(2),
    );
    let rec = T::structure(
        "Rec",
        vec![
            ("kind", T::utf8(E::lit(4))),
            // What that definition says one of these is worth.
            ("width", T::computed(E::tagged_by_text("defs", &["name"], E::field("kind"), &["width"]))),
            // And what it is called, read back as text.
            ("called", T::computed_text(E::tagged_by_text("defs", &["name"], E::field("kind"), &["name"]))),
        ],
    );
    let t = T::structure("Root", vec![("defs", defs), ("recs", T::repeat(rec, Until::End))]);
    let d = doc(b"flux  \x04time  \x08timefluxnope");
    let mut ev = Evaluator::new(Template::new("t", t));
    assert_eq!(ev.node(&d, &[1, 0, 1]).unwrap().value.as_int(), Some(8));
    assert_eq!(ev.node(&d, &[1, 1, 1]).unwrap().value.as_int(), Some(4));
    assert_eq!(ev.node(&d, &[1, 0, 2]).unwrap().value, Value::Str("time".into()));
    // A word nothing is labelled with finds nothing, which is an answer rather
    // than an error: zero for the number, and no text at all.
    assert_eq!(ev.node(&d, &[1, 2, 1]).unwrap().value.as_int(), Some(0));
    assert_eq!(ev.node(&d, &[1, 2, 2]).unwrap().value, Value::Str(String::new()));
    // Both ends of the search are connections: the field the word was read
    // from, and the definition it landed on.
    let seen: Vec<_> = ev.origins(&d, &[1, 0, 1]).unwrap().into_iter().map(|o| (o.role, o.label)).collect();
    assert!(seen.contains(&(Role::Value, "kind".to_string())), "{seen:?}");
    assert!(seen.iter().any(|(_, l)| l == "defs[1].width"), "{seen:?}");
    // And the question is written out with the word in its place, rather than
    // with the expression that found the word. `text` says the key is compared
    // as text, which is what tells this search from one keyed on a number.
    let rel = ev.relations(&d, &[1, 0, 1]).unwrap();
    assert_eq!(rel[0].written, "defs[name = text kind].width");
    assert_eq!(rel[0].substituted, "defs[name = \"time\"].width");
    assert_eq!(rel[0].result, "8");
}

#[test]
fn a_list_inside_a_sibling_can_be_indexed() {
    // The widths are in a table inside the header, and the values that are
    // those widths are the header's sibling. A name reaches only a field
    // beside the one asking, so a path is what gets there.
    let header = T::structure("Header", vec![("n", T::u8()), ("widths", T::array(T::u8(), E::field("n")))]);
    let value = T::structure("Value", vec![("v", T::uint_expr(E::elem_within(&["header", "widths"], E::idx(), &[]).mul(E::lit(8)), Big))]);
    let t = T::structure(
        "Root",
        vec![("header", header), ("values", T::array(value, E::within(&["header", "n"])))],
    );
    let d = doc(&[2, 1, 2, 0xaa, 0xbb, 0xcc]);
    let mut ev = Evaluator::new(Template::new("t", t));
    // One byte wide, then two.
    assert_eq!(ev.node(&d, &[1, 0, 0]).unwrap().value, Value::UInt(0xaa));
    assert_eq!(ev.node(&d, &[1, 1, 0]).unwrap().value, Value::UInt(0xbbcc));
    // The row says which entry of the table decided it, path and all.
    let seen: Vec<_> = ev.origins(&d, &[1, 1, 0]).unwrap().into_iter().map(|o| (o.role, o.label)).collect();
    assert!(seen.contains(&(Role::Width, "header.widths[1]".to_string())), "{seen:?}");
    let rel = ev.relations(&d, &[1, 1, 0]).unwrap();
    assert_eq!(rel[0].written, "header.widths[index] * 8");
}

/// A bit vector says which of a list of things has something, and the table
/// after it has one row per bit that was set. That count is written nowhere,
/// which is what `PopCount` is for.
#[test]
fn a_table_can_be_counted_by_the_bits_set_in_the_vector_before_it() {
    let t = T::structure(
        "Root",
        vec![
            ("present", T::bytes(E::lit(2))),
            ("values", T::array(T::u8(), E::pop_count("present"))),
        ],
    );
    // 0b1010_0001 0b0000_0110: five bits set, so five values follow.
    let d = doc(b"\xa1\x06\x0a\x0b\x0c\x0d\x0e");
    let mut ev = Evaluator::new(Template::new("t", t));
    assert_eq!(ev.node(&d, &[1]).unwrap().child_count, 5);
    assert_eq!(ev.node(&d, &[1, 4]).unwrap().value.as_int(), Some(0x0e));
}

/// Counted over the field's own bits, not its bytes. A vector of five items
/// lives in one byte, and whatever a format left in the three bits past the
/// end of it must not decide how long the table is.
#[test]
fn the_padding_at_the_end_of_a_bit_vector_is_not_counted() {
    let t = T::structure(
        "Root",
        vec![
            ("present", T::UInt { bits: 5, endian: crate::template::Endian::Big }),
            ("padding", T::UInt { bits: 3, endian: crate::template::Endian::Big }),
            ("values", T::array(T::u8(), E::pop_count("present"))),
        ],
    );
    // 0b1010_0111: two of the top five bits set, and the three padding bits
    // all set. Counting the byte would ask for five values.
    let d = doc(b"\xa7\x0a\x0b\x0c");
    let mut ev = Evaluator::new(Template::new("t", t));
    assert_eq!(ev.node(&d, &[2]).unwrap().child_count, 2);
}

#[test]
fn a_field_can_take_its_displayed_name_from_the_file() {
    // The name is written in a table earlier in the file, and the field it
    // names is a plain number the template calls `col1`.
    let t = T::structure(
        "Root",
        vec![
            ("labels", T::array(T::utf8(E::lit(4)), E::lit(2))),
            ("col1", T::u8()),
            ("col2", T::u8()),
        ],
    )
    .field_named_from("col1", E::elem_field("labels", E::lit(0), &[]))
    .field_named_from("col2", E::elem_field("labels", E::lit(1), &[]));
    let d = doc(b"fluxtime\x07\x09");
    let mut ev = Evaluator::new(Template::new("t", t));
    assert_eq!(ev.node(&d, &[1]).unwrap().name, "col1 flux");
    assert_eq!(ev.node(&d, &[2]).unwrap().name, "col2 time");
    // The declared name is still the name: a path and an expression are
    // written with it, and it does not move when the labels are edited.
    assert_eq!(ev.child_named(&d, &[], "col1").unwrap(), Some(vec![1]));
    // And the connection is exposed, as a name rather than as a value.
    let seen: Vec<_> = ev.origins(&d, &[1]).unwrap().into_iter().map(|o| (o.role, o.label)).collect();
    assert_eq!(seen, vec![(Role::Name, "labels[0]".to_string())]);
}

#[test]
fn the_elements_of_a_list_can_take_their_displayed_names_from_the_file() {
    // Two names in a table, and a list of three numbers named by position in
    // it: the third has no name written for it.
    let t = T::structure(
        "Root",
        vec![
            ("labels", T::array(T::utf8(E::lit(4)), E::lit(2))),
            ("vals", T::array(T::u8(), E::lit(3))),
            ("after", T::computed(E::elem_field("vals", E::lit(1), &[]))),
        ],
    )
    .field_elem_named_from("vals", E::elem_field("labels", E::idx(), &[]));
    let d = doc(b"fluxtime\x07\x09\x0b");
    let mut ev = Evaluator::new(Template::new("t", t));
    assert_eq!(ev.node(&d, &[1, 0]).unwrap().name, "[0] flux");
    assert_eq!(ev.node(&d, &[1, 1]).unwrap().name, "[1] time");
    // Nothing to read for the third, so it keeps its index and nothing fails.
    assert_eq!(ev.node(&d, &[1, 2]).unwrap().name, "[2]");
    // The list itself is not renamed: the declaration is about its elements.
    assert_eq!(ev.node(&d, &[1]).unwrap().name, "vals");
    // The index is still the name an expression reaches an element by.
    assert_eq!(ev.node(&d, &[2]).unwrap().value.as_int(), Some(9));
    // And the connection is exposed, as a name rather than as a value.
    let seen: Vec<_> = ev.origins(&d, &[1, 1]).unwrap().into_iter().map(|o| (o.role, o.label)).collect();
    assert_eq!(seen, vec![(Role::Name, "labels[1]".to_string())]);
}

#[test]
fn a_bit_field_of_a_number_is_a_shift_and_a_mask() {
    // A word packing six-bit differences, the way a Steim2 word does, read as
    // fields of the number rather than as bits of the bytes.
    let t = T::structure(
        "Root",
        vec![
            ("word", T::u32(Big)),
            ("d0", T::computed(E::bit_field(E::field("word"), 29, 6))),
            ("d1", T::computed(E::bit_field(E::field("word"), 23, 6))),
            ("s0", T::computed(E::signed_bit_field(E::field("word"), 29, 6))),
            ("s1", T::computed(E::signed_bit_field(E::field("word"), 23, 6))),
            ("none", T::computed(E::bit_field(E::field("word"), 29, 0))),
        ],
    );
    // 0b10_100001_111111_00...: bits 29..24 are 0b100001, which is 33 read
    // plain and -31 read as two's complement; bits 23..18 are all ones.
    let d = doc(&[0b1010_0001, 0b1111_1100, 0, 0]);
    let mut ev = Evaluator::new(Template::new("t", t));
    assert_eq!(ev.node(&d, &[1]).unwrap().value.as_int(), Some(33));
    assert_eq!(ev.node(&d, &[2]).unwrap().value.as_int(), Some(63));
    assert_eq!(ev.node(&d, &[3]).unwrap().value.as_int(), Some(-31));
    assert_eq!(ev.node(&d, &[4]).unwrap().value.as_int(), Some(-1));
    // A field of no bits is no bits, and asks the file nothing.
    assert_eq!(ev.node(&d, &[5]).unwrap().value.as_int(), Some(0));
    // The reader is shown the shift and the mask, not thirty added-up bits,
    // and the mask in hex, where six set bits look like six set bits.
    assert_eq!(
        write_expr(&E::bit_field(E::field("word"), 29, 6)).as_deref(),
        Some("word >> 24 & 0x3f")
    );
}

#[test]
fn a_division_rounds_up_and_a_logarithm_rounds_down() {
    // Ten rows in chunks of four is three chunks; a run of 4096 doubled from
    // 512 is three doublings, and 4095 is still two.
    let t = T::structure(
        "Root",
        vec![
            ("rows", T::u16(Big)),
            ("chunk", T::u16(Big)),
            ("chunks", T::computed(E::field("rows").div_ceil(E::field("chunk")))),
            ("exact", T::computed(E::lit(12).div_ceil(E::field("chunk")))),
            ("big", T::computed(E::lit(4096).log2().sub(E::lit(512).log2()))),
            ("under", T::computed(E::lit(4095).log2().sub(E::lit(512).log2()))),
            ("one", T::computed(E::lit(1).log2())),
        ],
    );
    let d = doc(&[0, 10, 0, 4]);
    let mut ev = Evaluator::new(Template::new("t", t));
    assert_eq!(ev.node(&d, &[2]).unwrap().value.as_int(), Some(3));
    assert_eq!(ev.node(&d, &[3]).unwrap().value.as_int(), Some(3));
    assert_eq!(ev.node(&d, &[4]).unwrap().value.as_int(), Some(3));
    assert_eq!(ev.node(&d, &[5]).unwrap().value.as_int(), Some(2));
    assert_eq!(ev.node(&d, &[6]).unwrap().value.as_int(), Some(0));
    assert_eq!(write_expr(&E::field("rows").div_ceil(E::field("chunk"))).as_deref(), Some("ceil(rows / chunk)"));
    assert_eq!(write_expr(&E::field("rows").add(E::lit(1)).log2()).as_deref(), Some("log2(rows + 1)"));
}

#[test]
fn a_logarithm_of_nothing_and_a_division_by_nothing_are_refused() {
    for e in [E::field("n").log2(), E::lit(5).div_ceil(E::field("n"))] {
        let t = T::structure("Root", vec![("n", T::u8()), ("x", T::computed(e))]);
        let mut ev = Evaluator::new(Template::new("t", t));
        assert!(ev.node(&doc(&[0]), &[1]).is_err());
    }
}

/// A scratch template with one byte in it, so an expression made of literals
/// has somewhere to be asked from.
fn nowhere() -> (Document<MemSource>, Evaluator) {
    let t = T::structure("Root", vec![("n", T::u8()), ("after", T::u8())]);
    let d = doc(&[7, 0]);
    let mut ev = Evaluator::new(Template::new("t", t));
    ev.resolve(&d, &[]).unwrap();
    (d, ev)
}

/// Kaitai's `%` and Python's: the answer takes the sign of the divisor, so
/// `-5 % 3` is 1. The machine's own remainder would say -2, and a template
/// written against a specification that says the first would read the file
/// wrongly.
#[test]
fn a_modulo_takes_the_sign_of_the_divisor() {
    let (d, mut ev) = nowhere();
    let mut m = |a: i128, b: i128| ev.eval_expr(&d, &[1], &E::lit(a).modulo(E::lit(b))).unwrap();
    assert_eq!(m(7, 3), 1);
    assert_eq!(m(6, 3), 0);
    assert_eq!(m(-5, 3), 1);
    assert_eq!(m(5, -3), -1);
    assert_eq!(m(-5, -3), -2);
    // The padding idiom it replaces, as a check that the two agree.
    assert_eq!(m(13, 4), 13 - (13 / 4) * 4);
    // And nothing to divide by is refused, as it is for a division.
    assert!(ev.eval_expr(&d, &[1], &E::field("n").modulo(E::lit(0))).is_err());
}

/// Each comparison answers one or nothing, so it is a number like any other.
#[test]
fn the_comparisons_answer_one_or_nothing() {
    let (d, mut ev) = nowhere();
    let l = E::lit;
    let cases = [
        (l(3).equal_to(l(3)), 1),
        (l(3).equal_to(l(4)), 0),
        (l(3).not_equal(l(4)), 1),
        (l(3).not_equal(l(3)), 0),
        (l(3).less_than(l(4)), 1),
        (l(3).less_or_equal(l(3)), 1),
        (l(4).less_or_equal(l(3)), 0),
        (l(4).greater_than(l(3)), 1),
        (l(3).greater_than(l(3)), 0),
        (l(3).greater_or_equal(l(3)), 1),
        (l(2).greater_or_equal(l(3)), 0),
        // Signs are the arithmetic's, not the bytes'.
        (l(-1).less_than(l(0)), 1),
    ];
    for (e, want) in cases {
        assert_eq!(ev.eval_expr(&d, &[1], &e).unwrap(), want, "{e:?}");
    }
    // The field a comparison names is read, and the answer is about its value.
    assert_eq!(ev.eval_expr(&d, &[1], &E::field("n").equal_to(l(7))).unwrap(), 1);
}

/// `and` and `or` answer a truth, and leave the far side alone once the near
/// side has settled it: a guard is only a guard while what it guards stays
/// unread.
#[test]
fn the_boolean_operators_stop_once_the_answer_is_settled() {
    let (d, mut ev) = nowhere();
    let l = E::lit;
    // Something no reading can answer, to stand where the far side is.
    let bad = || l(1).div(l(0));
    assert_eq!(ev.eval_expr(&d, &[1], &l(1).both(l(2))).unwrap(), 1);
    assert_eq!(ev.eval_expr(&d, &[1], &l(1).both(l(0))).unwrap(), 0);
    assert_eq!(ev.eval_expr(&d, &[1], &l(0).both(bad())).unwrap(), 0);
    assert!(ev.eval_expr(&d, &[1], &l(1).both(bad())).is_err());
    assert_eq!(ev.eval_expr(&d, &[1], &l(0).either(l(9))).unwrap(), 1);
    assert_eq!(ev.eval_expr(&d, &[1], &l(0).either(l(0))).unwrap(), 0);
    assert_eq!(ev.eval_expr(&d, &[1], &l(5).either(bad())).unwrap(), 1);
    assert_eq!(ev.eval_expr(&d, &[1], &l(0).negate()).unwrap(), 1);
    assert_eq!(ev.eval_expr(&d, &[1], &l(5).negate()).unwrap(), 0);
    // A truth, not a value: the value-or answers 12 where this answers 1.
    assert_eq!(ev.eval_expr(&d, &[1], &l(12).either(l(4))).unwrap(), 1);
    assert_eq!(ev.eval_expr(&d, &[1], &l(12).or(l(4))).unwrap(), 12);
}

/// Only the branch taken is worked out, and a `then` of nothing is still the
/// answer, which is where the older `Less` and `Or` pairing went wrong.
#[test]
fn only_the_branch_a_condition_takes_is_worked_out() {
    let (d, mut ev) = nowhere();
    let l = E::lit;
    let bad = || l(1).div(l(0));
    assert_eq!(ev.eval_expr(&d, &[1], &E::cond(l(1), l(5), bad())).unwrap(), 5);
    assert_eq!(ev.eval_expr(&d, &[1], &E::cond(l(0), bad(), l(9))).unwrap(), 9);
    // Zero is an answer, not a fall-through.
    assert_eq!(ev.eval_expr(&d, &[1], &E::cond(l(1), l(0), l(9))).unwrap(), 0);
    // Which is what the pair it replaces gets wrong: `or` takes its right
    // side whenever the left comes to nothing.
    assert_eq!(ev.eval_expr(&d, &[1], &l(0).or(l(9))).unwrap(), 9);
    // The condition itself is read from the file like anything else.
    assert_eq!(ev.eval_expr(&d, &[1], &E::cond(E::field("n").equal_to(l(7)), l(1), l(2))).unwrap(), 1);
    // A condition that cannot be worked out is a failure, not a branch.
    assert!(ev.eval_expr(&d, &[1], &E::cond(bad(), l(1), l(2))).is_err());
}

/// The condition decides which branch a field's shape came from, so the panel
/// names the field the condition read and the fields of the branch that was
/// taken, and nothing from the branch that was not.
#[test]
fn a_condition_names_the_branch_it_took_and_not_the_other() {
    let t = T::structure(
        "Root",
        vec![
            ("wide", T::u8()),
            ("short", T::u8()),
            ("long", T::u8()),
            ("body", T::bytes(E::cond(E::field("wide"), E::field("long"), E::field("short")))),
        ],
    );
    let d = doc(&[1, 2, 4, 0, 0, 0, 0]);
    let mut ev = Evaluator::new(Template::new("t", t));
    assert_eq!(ev.node(&d, &[3]).unwrap().size_bits, 4 * 8);
    let origins = ev.origins(&d, &[3]).unwrap();
    let labels: Vec<&str> = origins.iter().map(|o| o.label.as_str()).collect();
    assert_eq!(labels, vec!["wide", "long"]);
    // Written out as the template writes it, with both branches shown: a
    // reader checking the answer needs the case that did not come up.
    let rel = ev.relations(&d, &[3]).unwrap();
    assert_eq!(rel[0].written, "wide ? long : short");
    assert_eq!(rel[0].substituted, "1 ? 4 : 2");
    assert_eq!(rel[0].result, "4");
}

/// Where a field starts is counted from the window around it rather than from
/// the file, in bytes, as everything else here that measures a distance is.
#[test]
fn a_position_is_counted_in_bytes_from_the_window_it_sits_in() {
    let inner = T::structure(
        "Inner",
        vec![("a", T::u16(Big)), ("here", T::computed(E::Pos)), ("room", T::computed(E::WindowSize))],
    );
    let t = T::structure(
        "Root",
        vec![
            ("tag", T::u8()),
            ("body", T::sized(E::lit(6), inner)),
            ("top", T::computed(E::Pos)),
            ("whole", T::computed(E::WindowSize)),
        ],
    );
    let d = doc(&[9, 0, 1, 0, 0, 0, 0, 0]);
    let mut ev = Evaluator::new(Template::new("t", t));
    // Two bytes into a window that starts one byte into the file, and the
    // window is six bytes: neither number counts the byte in front of it.
    assert_eq!(ev.node(&d, &[1, 1]).unwrap().value.as_int(), Some(2));
    assert_eq!(ev.node(&d, &[1, 2]).unwrap().value.as_int(), Some(6));
    // Outside any window the file is the window, so the position is the
    // field's own offset and the size is the file's.
    assert_eq!(ev.node(&d, &[2]).unwrap().value.as_int(), Some(7));
    assert_eq!(ev.node(&d, &[3]).unwrap().value.as_int(), Some(8));
    assert_eq!(write_expr(&E::Pos).as_deref(), Some("pos"));
    assert_eq!(write_expr(&E::WindowSize).as_deref(), Some("size of window"));

    // A field partway through a byte is in that byte, so the answer rounds
    // down: four bits in is still byte nought. `BitsOf` is what counts bits.
    let packed = T::structure(
        "Root",
        vec![
            ("nibble", T::UInt { bits: 4, endian: Big }),
            ("here", T::computed(E::Pos)),
            ("rest", T::UInt { bits: 12, endian: Big }),
            ("later", T::computed(E::Pos)),
        ],
    );
    let mut ev = Evaluator::new(Template::new("t", packed));
    assert_eq!(ev.node(&doc(&[0xab, 0xcd]), &[1]).unwrap().value.as_int(), Some(0));
    assert_eq!(ev.node(&doc(&[0xab, 0xcd]), &[3]).unwrap().value.as_int(), Some(2));
}

/// How many elements a list holds is a different number from how many bytes
/// it took, and only a list has one at all.
#[test]
fn a_count_of_elements_is_not_a_count_of_bytes() {
    let t = T::structure(
        "Root",
        vec![
            ("n", T::u8()),
            ("items", T::array(T::u16(Big), E::field("n"))),
            ("count", T::computed(E::len_of("items"))),
            ("bytes", T::computed(E::SizeOf("items".into()))),
        ],
    );
    let d = doc(&[3, 0, 1, 0, 2, 0, 3]);
    let mut ev = Evaluator::new(Template::new("t", t));
    assert_eq!(ev.node(&d, &[2]).unwrap().value.as_int(), Some(3));
    assert_eq!(ev.node(&d, &[3]).unwrap().value.as_int(), Some(6));
    assert_eq!(write_expr(&E::len_of("items")).as_deref(), Some("count of items"));

    // A run that stops at the end of its window is counted the same way.
    let t2 = T::structure(
        "Root",
        vec![
            ("body", T::sized(E::lit(4), T::repeat(T::u16(Big), Until::End))),
            ("count", T::computed(E::len_of("body"))),
        ],
    );
    assert_eq!(Evaluator::new(Template::new("t", t2)).node(&doc(&[0; 4]), &[1]).unwrap().value.as_int(), Some(2));

    // Something that is not a list has no element count, and is told so
    // rather than answered with its length.
    let t3 = T::structure("Root", vec![("n", T::u8()), ("count", T::computed(E::len_of("n")))]);
    let mut ev3 = Evaluator::new(Template::new("t", t3));
    let e = ev3.node(&doc(&[3]), &[1]).unwrap_err();
    assert!(format!("{e:?}").contains("not a list"), "{e:?}");
    // And a name nothing declared is an error too.
    let t4 = T::structure("Root", vec![("n", T::u8()), ("count", T::computed(E::len_of("nowhere")))]);
    assert!(Evaluator::new(Template::new("t", t4)).node(&doc(&[3]), &[1]).is_err());
}

/// A run that ends on a question rather than on one field holding one fixed
/// thing. The element that answers is part of the run.
#[test]
fn a_run_can_stop_on_a_question_about_its_element() {
    let rec = || T::structure("Rec", vec![("tag", T::u8()), ("len", T::u8())]);
    let ends_empty = || Until::Cond(E::field("len").equal_to(E::lit(0)));

    // Three records, the last of them empty, and two bytes after the run that
    // belong to nothing.
    let t = T::structure("Root", vec![("items", T::repeat(rec(), ends_empty()))]);
    let d = doc(&[1, 3, 2, 4, 3, 0, 9, 9]);
    let mut ev = Evaluator::new(Template::new("t", t));
    assert_eq!(ev.node(&d, &[0]).unwrap().child_count, 3);
    assert_eq!(ev.node(&d, &[0]).unwrap().size_bits, 6 * 8);

    // A file that stops before the element that would have ended the run is
    // read as far as it goes, the same as a run told to read to the end.
    let t2 = T::structure("Root", vec![("items", T::repeat(rec(), ends_empty()))]);
    let mut ev2 = Evaluator::new(Template::new("t", t2));
    assert_eq!(ev2.node(&doc(&[1, 3, 2, 4]), &[0]).unwrap().child_count, 2);
}

/// The index in the question is the element's place in the run, and what is
/// left over is measured in the list's own container once the element has
/// been read.
#[test]
fn a_stopping_question_knows_the_index_and_the_room_left() {
    // Stop after the element at index 1, which is two of them.
    let t = T::structure("Root", vec![("items", T::repeat(T::u8(), Until::Cond(E::Idx.equal_to(E::lit(1)))))]);
    let d = doc(&[7, 7, 7, 7]);
    assert_eq!(Evaluator::new(Template::new("t", t)).node(&d, &[0]).unwrap().child_count, 2);

    // Stop once there is no room for another two-byte element: a window of
    // five bytes holds two of them and a byte nobody reads.
    let run = T::repeat(T::u16(Big), Until::Cond(E::Remaining.less_than(E::lit(2))));
    let t2 = T::structure("Root", vec![("body", T::sized(E::lit(5), run))]);
    let mut ev = Evaluator::new(Template::new("t", t2));
    assert_eq!(ev.node(&d5(), &[0]).unwrap().child_count, 2);
    assert_eq!(ev.node(&d5(), &[0, 1]).unwrap().offset_bits, 2 * 8);
}

fn d5() -> Document<MemSource> {
    doc(&[0, 1, 0, 2, 0])
}

fn optional_record() -> Ty {
    T::structure(
        "Root",
        vec![
            ("flags", T::u8()),
            ("extra", T::when(E::field("flags").bit(0), T::u16(Big))),
            ("tail", T::u8()),
        ],
    )
}

/// A field the file wrote is read as itself, with nothing in the type column
/// about the question that let it in.
#[test]
fn an_optional_field_the_file_wrote_reads_as_itself() {
    let d = doc(&[1, 0, 9, 7]);
    let mut ev = Evaluator::new(Template::new("t", optional_record()));
    let n = ev.node(&d, &[1]).unwrap();
    assert_eq!(n.size_bits, 16);
    assert_eq!(n.type_name, "u16 be");
    assert_eq!(n.value.as_int(), Some(9));
    assert!(!n.absent);
    assert_eq!(ev.node(&d, &[2]).unwrap().offset_bits, 3 * 8);
}

/// One the file left out is absent: no bytes, nothing inside, and a type
/// column that says the field may not be here rather than one that shows an
/// empty structure. Nothing after it moves.
#[test]
fn an_optional_field_the_file_left_out_is_absent_and_not_empty() {
    let d = doc(&[0, 9, 7]);
    let mut ev = Evaluator::new(Template::new("t", optional_record()));
    let n = ev.node(&d, &[1]).unwrap();
    assert_eq!(n.size_bits, 0);
    assert!(n.absent);
    assert_eq!(n.type_name, "optional u16 be");
    assert_eq!(n.child_count, 0);
    assert!(!n.composite);
    // The field after it starts where it would have, and reads the bytes the
    // absent one did not take.
    let after = ev.node(&d, &[2]).unwrap();
    assert_eq!((after.offset_bits, after.value.as_int()), (8, Some(9)));

    // The question is a connection like any other: the reader is pointed at
    // the field that decided it, and shown the working.
    let origins = ev.origins(&d, &[1]).unwrap();
    let labels: Vec<&str> = origins.iter().map(|o| o.label.as_str()).collect();
    assert_eq!(labels, vec!["flags"]);
    let rel = ev.relations(&d, &[1]).unwrap();
    assert_eq!(rel[0].written, "bit(flags, 0)");
    assert_eq!(rel[0].substituted, "bit(0, 0)");
}

/// A field that is not in the file has no number, and is not nought either.
/// Reading one as nought would let a switch quietly take case 0 and a length
/// quietly be none, with nothing said about either.
#[test]
fn an_absent_field_has_no_number_rather_than_nought() {
    let t = |cond: Expr| {
        T::structure(
            "Root",
            vec![
                ("flags", T::u8()),
                ("extra", T::when(E::field("flags"), T::u8())),
                ("body", T::bytes(cond)),
            ],
        )
    };
    // Naming it outright is refused, and so is asking how long it is.
    let d = doc(&[0, 1, 2, 3]);
    for e in [E::field("extra"), E::SizeOf("extra".into())] {
        let mut ev = Evaluator::new(Template::new("t", t(e)));
        let err = ev.node(&d, &[2]).unwrap_err();
        assert!(format!("{err:?}").contains("not in this file"), "{err:?}");
    }
    // Asking whether it is there first is how a template reads one: the
    // branch that names it is never taken when it is not.
    let guarded = E::cond(E::field("flags"), E::field("extra"), E::lit(2));
    let mut ev = Evaluator::new(Template::new("t", t(guarded.clone())));
    assert_eq!(ev.node(&d, &[2]).unwrap().size_bits, 2 * 8);
    // And with the flag set it reads the field, which is there.
    let d2 = doc(&[1, 3, 0, 0, 0]);
    assert_eq!(Evaluator::new(Template::new("t", t(guarded))).node(&d2, &[2]).unwrap().size_bits, 3 * 8);
}

/// A whole structure can be the optional thing, and an absent one has no rows
/// at all rather than a heading with nothing under it.
#[test]
fn an_absent_structure_has_no_rows_under_it() {
    let inner = || T::structure("Inner", vec![("a", T::u8()), ("b", T::u8())]);
    let t = |bytes: &[u8]| {
        let ty = T::structure(
            "Root",
            vec![("n", T::u8()), ("body", T::when(E::field("n"), inner())), ("tail", T::u8())],
        );
        (doc(bytes), Evaluator::new(Template::new("t", ty)))
    };
    let (d, mut ev) = t(&[0, 5]);
    let n = ev.node(&d, &[1]).unwrap();
    assert!(n.absent);
    assert_eq!((n.child_count, n.size_bits), (0, 0));
    assert_eq!(n.type_name, "optional Inner");
    assert_eq!(ev.node(&d, &[2]).unwrap().value.as_int(), Some(5));
    // And present, it is an ordinary structure again.
    let (d2, mut ev2) = t(&[1, 2, 3, 5]);
    let n2 = ev2.node(&d2, &[1]).unwrap();
    assert!(!n2.absent);
    assert_eq!((n2.child_count, n2.size_bits), (2, 16));
    assert_eq!(n2.type_name, "Inner");
    assert_eq!(ev2.node(&d2, &[2]).unwrap().value.as_int(), Some(5));
}

#[test]
fn a_shift_of_more_than_a_word_is_refused_either_way() {
    let t = T::structure("Root", vec![("n", T::u32(Big)), ("after", T::u8())]);
    let d = doc(&[0, 0, 0, 4, 0]);
    let mut ev = Evaluator::new(Template::new("t", t));
    ev.resolve(&d, &[]).unwrap();
    assert!(ev.eval_expr(&d, &[1], &E::field("n").shr(E::lit(64))).is_err());
    assert!(ev.eval_expr(&d, &[1], &E::field("n").shr(E::lit(-1))).is_err());
    // Anding is the arithmetic, sign and all: a mask of -1 is every bit.
    assert_eq!(ev.eval_expr(&d, &[1], &E::field("n").and(E::lit(-1))).unwrap(), 4);
}

// ----- decoded streams -----

use crate::codec::Codec;

/// A file that is a two-byte header and then a zlib stream holding a
/// structure: the shape every format that compresses part of itself has.
fn packed_doc(inner: &[u8]) -> (Document<MemSource>, usize) {
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(inner, 6);
    let mut bytes = vec![0xaa, 0xbb];
    bytes.extend_from_slice(&packed);
    (doc(&bytes), packed.len())
}

fn packed_template(len: usize) -> Template {
    Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("tag", T::u16(Big)),
                (
                    "stream",
                    T::decoded(
                        E::lit(len as i128),
                        Codec::Zlib,
                        T::structure("Object", vec![("a", T::u16(Big)), ("b", T::u32(Big))]),
                    ),
                ),
            ],
        ),
    )
}

#[test]
fn a_stream_keeps_its_own_bytes_and_its_children_count_from_the_decoded_ones() {
    let (d, len) = packed_doc(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66]);
    let mut ev = Evaluator::new(packed_template(len));

    // The field is the compressed run: where it is in the file, and as long as
    // the file makes it. Not as long as what came out of it.
    let stream = ev.node(&d, &[1]).unwrap();
    assert_eq!(stream.offset_bits, 2 * 8);
    assert_eq!(stream.size_bits, len as u64 * 8);
    assert_eq!(stream.space, 0);
    assert_eq!(stream.refused, None);
    // What came out of it, and what the decoder read to get there.
    assert_eq!(stream.child_count, 2);
    assert!(stream.composite);
    assert_eq!(stream.type_name, "zlib");

    // Its contents count from the front of the decoded bytes, in a space of
    // their own.
    let object = ev.node(&d, &[1, 0]).unwrap();
    assert_eq!(object.offset_bits, 0);
    assert_eq!(object.space, 1);
    let a = ev.node(&d, &[1, 0, 0]).unwrap();
    assert_eq!((a.offset_bits, a.size_bits, a.space), (0, 16, 1));
    assert_eq!(a.value.as_int(), Some(0x1122));
    let b = ev.node(&d, &[1, 0, 1]).unwrap();
    assert_eq!((b.offset_bits, b.size_bits, b.space), (16, 32, 1));
    assert_eq!(b.value.as_int(), Some(0x33445566));
}

/// Nothing inside a stream is written back: a decoded byte is a function of
/// every compressed byte before it, and there is nowhere to put the change.
#[test]
fn nothing_inside_a_stream_is_editable() {
    let (d, len) = packed_doc(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66]);
    let mut ev = Evaluator::new(packed_template(len));
    assert!(!ev.node(&d, &[1, 0, 0]).unwrap().editable);
    assert!(!ev.node(&d, &[1, 0, 1]).unwrap().editable);
    // The same type outside a stream is.
    assert!(ev.node(&d, &[0]).unwrap().editable);
}

/// The cursor never lands on a field of what came *out* of a stream: those are
/// at offsets of the decoded bytes and no bit of the file is any one of them.
/// It does land on what the decoder read, which is bits of the file: the run
/// is a header, some tables and a run of symbols, and every one of those is
/// somewhere.
#[test]
fn locate_lands_on_what_the_decoder_read_and_never_on_what_it_produced() {
    let (d, len) = packed_doc(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66]);
    let mut ev = Evaluator::new(packed_template(len));
    assert_eq!(ev.locate(&d, 0).unwrap(), vec![0]);
    for byte in 2..2 + len as u64 {
        let at = ev.locate(&d, byte * 8).unwrap();
        // Never inside the decoded space, which is child 0.
        assert_ne!(at.get(1), Some(&0), "at byte {byte}, the cursor is inside the decoded bytes");
        assert_eq!(&at[..1], &[1], "at byte {byte}");
        // The two zlib header bytes and the four of the checksum belong to no
        // block, so the run itself is the answer for those.
        let deep = at.len() > 1;
        assert_eq!(deep, (4..2 + len as u64 - 4).contains(&byte), "at byte {byte}, landed on {at:?}");
    }
    // And the hex view draws the wrapper's bytes as the run they are, with
    // the blocks between them as entries of their own rather than swallowed.
    let spans = ev.spans(&d, 2 * 8, (2 + len as u64) * 8, 100).unwrap();
    assert!(spans.len() > 2, "the run drew as {} entries", spans.len());
    assert_eq!(spans[0].name, "stream");
    assert_eq!(spans[0].size_bits, 2 * 8, "the zlib header is the wrapper's two bytes");
    // The tail is whatever the last block did not use of its last byte, and
    // then the Adler-32.
    assert_eq!(spans.last().unwrap().name, "stream");
    assert!(spans.last().unwrap().size_bits >= 4 * 8, "the tail is shorter than the Adler-32");
    // A block is one entry, not its header fields and then its symbols: the
    // column beside the bytes says which block they are in, and the reader who
    // opens it gets `bfinal`, the tables and every symbol.
    assert!(spans.iter().any(|s| s.name.contains("block")), "no block in {:?}", names(&spans));
    assert!(!spans.iter().any(|s| s.name == "bfinal"), "block insides in the column: {:?}", names(&spans));
    // Nothing overlaps and nothing is skipped.
    let mut at = 2 * 8;
    for s in &spans {
        assert_eq!(s.offset_bits, at, "{:?} starts in the wrong place", s.name);
        at += s.size_bits;
    }
    assert_eq!(at, (2 + len as u64) * 8);
}

fn names(spans: &[crate::eval::Span]) -> Vec<String> {
    spans.iter().map(|s| s.name.clone()).collect()
}

/// An edit anywhere drops the trace, and so has to drop the fields laid out
/// from it. They are bits of the file like any other, so nothing else would.
#[test]
fn an_edit_drops_the_fields_the_trace_laid_down() {
    let (d, len) = packed_doc(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66]);
    let mut ev = Evaluator::new(packed_template(len));
    let field = ev.node(&d, &[1, 1, 0, 0]).unwrap();
    let before = ev.memo_len();
    // An edit past everything: `forget_after` would keep every node here,
    // since they all end before it. They still have to go, because the trace
    // they were laid out from does.
    ev.invalidate_from(u64::MAX);
    assert_eq!(ev.memo.without_parent(), Vec::<Vec<usize>>::new());
    assert!(ev.memo_len() < before, "{before} nodes kept, trace or no trace");
    // And asking again works: the stream is opened again and the fields come
    // back the same.
    assert_eq!(ev.node(&d, &[1, 1, 0, 0]).unwrap(), field);
}

/// A run that will not open is the bytes it is, with the node saying which way
/// it would not open. Not an error: a broken block should not take the listing
/// down with it.
#[test]
fn a_stream_that_will_not_open_says_so_and_holds_nothing() {
    let d = doc(&[0xaa, 0xbb, 1, 2, 3, 4, 5, 6]);
    let mut ev = Evaluator::new(packed_template(6));
    let stream = ev.node(&d, &[1]).unwrap();
    assert_eq!(stream.child_count, 0);
    assert_eq!(stream.refused.as_deref(), Some("failed"));
    assert_eq!(stream.size_bits, 6 * 8);
    assert_eq!(ev.locate(&d, 4 * 8).unwrap(), vec![1]);
}

/// The field a decoded one came out of, so the reader can go and look at it.
#[test]
fn a_field_inside_a_stream_names_the_stream_it_came_from() {
    let (d, len) = packed_doc(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66]);
    let mut ev = Evaluator::new(packed_template(len));
    ev.node(&d, &[1, 0, 0]).unwrap();
    let origins = ev.origins(&d, &[1, 0, 0]).unwrap();
    let from = origins.iter().find(|o| o.role == Role::Value).expect("came from the stream");
    assert_eq!(from.label, "stream");
    assert_eq!(from.path, vec![1]);
    assert_eq!(from.value, "zlib");
}

/// An edit drops what was read inside a stream and the stream is opened
/// again from the bytes as they now stand. Forgetting by offset alone would keep
/// them: everything in a stream is at offset 0 of its own space and so looks
/// like it ended before any edit anywhere.
#[test]
fn editing_the_file_reopens_the_stream_rather_than_keeping_what_it_said() {
    let (mut d, len) = packed_doc(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66]);
    let mut ev = Evaluator::new(packed_template(len));
    assert_eq!(ev.node(&d, &[1, 0, 0]).unwrap().value.as_int(), Some(0x1122));

    // A byte of the header, well before the stream. The stream still opens,
    // and every field inside it has been read afresh from a space opened
    // afresh: a stale buffer would have been freed and the read would fail.
    d.overwrite_bits(0, &[0xcc], 8);
    ev.invalidate_from(0);
    assert_eq!(ev.memo.without_parent(), Vec::<Vec<usize>>::new());
    assert_eq!(ev.node(&d, &[0]).unwrap().value.as_int(), Some(0xccbb));
    let a = ev.node(&d, &[1, 0, 0]).unwrap();
    assert_eq!(a.value.as_int(), Some(0x1122));
    assert_eq!(a.space, 1);

    // A byte of the compressed run itself: it is not the stream it was, so
    // what it holds is worked out again rather than remembered.
    d.overwrite_bits(5 * 8, &[0x5a], 8);
    ev.invalidate_from(5 * 8);
    assert_eq!(ev.memo.without_parent(), Vec::<Vec<usize>>::new());
    let stream = ev.node(&d, &[1]).unwrap();
    assert!(stream.child_count == 0 || ev.node(&d, &[1, 0, 0]).is_ok());
}

/// Nothing decoded is written back, and asking is refused rather than writing
/// a bit of a stream to the byte of the file with the same number.
#[test]
fn a_write_into_a_stream_is_refused_rather_than_landing_in_the_file() {
    let (d, len) = packed_doc(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66]);
    let mut ev = Evaluator::new(packed_template(len));
    let err = ev.prepare_write(&d, &[1, 0, 0], "9999").unwrap_err();
    assert!(matches!(err, EvalError::Failed(_)), "a write into a stream produced {err:?}");
    // The same field outside a stream still writes.
    assert!(ev.prepare_write(&d, &[0], "1").is_ok());
}

/// A stream's contents have a declared type like any other child, so asking
/// what shaped them is an answer rather than an error.
#[test]
fn the_contents_of_a_stream_can_be_asked_what_shaped_them() {
    let (d, len) = packed_doc(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66]);
    let mut ev = Evaluator::new(packed_template(len));
    ev.node(&d, &[1, 0]).unwrap();
    let origins = ev.origins(&d, &[1, 0]).unwrap();
    let from = origins.iter().find(|o| o.role == Role::Value).expect("came from the stream");
    assert_eq!((from.label.as_str(), from.value.as_str()), ("stream", "zlib"));
    assert_eq!(from.path, vec![1]);
    // And the relations do not fall over on it either.
    ev.relations(&d, &[1, 0]).unwrap();
}

/// A switch that peeks at the byte it is about to read has to peek at the
/// stream's byte, not at the file's byte with the same number. The two differ
/// here on purpose: the file's byte 0 is 0xaa and the stream's is 0x02.
#[test]
fn a_peek_inside_a_stream_looks_at_the_stream() {
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&[0x02, 0x77], 6);
    let mut bytes = vec![0xaa, 0xbb];
    bytes.extend_from_slice(&packed);
    let d = doc(&bytes);
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("tag", T::u16(Big)),
                (
                    "stream",
                    T::decoded(
                        E::lit(packed.len() as i128),
                        Codec::Zlib,
                        // The first byte says which shape follows. Read from
                        // the file instead, that byte is 0xaa and neither case
                        // is taken.
                        T::switch(
                            E::peek(8, Big),
                            vec![(2, T::structure("Two", vec![("kind", T::u8()), ("value", T::u8())]))],
                            T::structure("Other", vec![("wrong", T::u8())]),
                        ),
                    ),
                ),
            ],
        ),
    );
    let mut ev = Evaluator::new(t);
    let picked = ev.node(&d, &[1, 0]).unwrap();
    assert_eq!(picked.type_name, "Two", "the peek read the file rather than the stream");
    assert_eq!(ev.node(&d, &[1, 0, 1]).unwrap().value.as_int(), Some(0x77));
}

/// A field's bytes come from the space the field is in. Read from the file at
/// the same offset instead, these would be the file's first bytes, which are
/// some other field entirely.
#[test]
fn a_fields_bytes_come_from_the_space_it_is_in() {
    let inner = [0x11u8, 0x22, 0x33, 0x44, 0x55, 0x66];
    let (d, len) = packed_doc(&inner);
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("tag", T::u16(Big)),
                ("stream", T::decoded(E::lit(len as i128), Codec::Zlib, T::bytes(E::Remaining))),
            ],
        ),
    );
    let mut ev = Evaluator::new(t);
    let (bytes, cut) = ev.field_bytes(&d, &[1, 0], 64).unwrap();
    assert_eq!(bytes, inner);
    assert!(!cut);
    // The file at offset 0 is the header, and nothing here read it by mistake.
    let (head, _) = ev.field_bytes(&d, &[0], 64).unwrap();
    assert_eq!(head, vec![0xaa, 0xbb]);
    // And a field longer than the limit says it was cut.
    let (some, cut) = ev.field_bytes(&d, &[1, 0], 3).unwrap();
    assert_eq!((some.as_slice(), cut), (&inner[..3], true));
}

/// Writing a scalar inside a JSON field: the numbers, the words, and a header
/// whose length another field already recorded.
mod json_edits {
    use super::*;

    const TEXT: &str = r#"{"n": 12, "f": 1.5, "on": true, "gone": null, "xs": [1, 2, [3, "four"]]}"#;

    /// Write into the JSON at `path` and hand back the file, or the refusal.
    fn write(text: &[u8], t: Template, path: &[usize], typed: &str) -> Result<Vec<u8>, String> {
        let mut d = doc(text);
        let mut ev = Evaluator::new(t);
        let w = ev.prepare_write(&d, path, typed).map_err(|e| match e {
            EvalError::Failed(why) => why,
            other => panic!("{other:?}"),
        })?;
        d.replace_bits(w.offset_bits, &w.data, w.n_bits, w.old_bits);
        let mut out = vec![0u8; (d.len_bits() / 8) as usize];
        d.read_bytes(0, &mut out);
        Ok(out)
    }

    fn plain(path: &[usize], typed: &str) -> Result<String, String> {
        let out = write(TEXT.as_bytes(), Template::new("json", T::json()), path, typed)?;
        Ok(String::from_utf8(out).unwrap())
    }

    #[test]
    fn a_number_is_written_as_it_was_typed() {
        assert!(plain(&[0], "4096").unwrap().starts_with(r#"{"n": 4096,"#));
        // An integer stays an integer, and a float keeps the form it was given.
        assert!(plain(&[1], "1.5e10").unwrap().contains(r#""f": 1.5e10,"#));
        assert!(plain(&[0], "-7").unwrap().starts_with(r#"{"n": -7,"#));
    }

    #[test]
    fn what_json_would_not_call_a_number_is_refused() {
        for bad in ["05", "+1", ".5", "NaN", "Infinity", "0x10", "twelve", ""] {
            let err = plain(&[0], bad).unwrap_err();
            assert!(err.contains("Not a JSON number"), "{bad}: {err}");
        }
    }

    #[test]
    fn a_boolean_is_one_of_two_words() {
        assert!(plain(&[2], "false").unwrap().contains(r#""on": false,"#));
        assert!(plain(&[2], "0").unwrap_err().contains("Expected true or false."));
    }

    #[test]
    fn null_stays_null() {
        assert_eq!(plain(&[3], "null").unwrap(), TEXT);
        assert!(plain(&[3], "0").unwrap_err().contains("Expected null."));
    }

    #[test]
    fn an_element_deep_in_an_array_moves_the_rest_along() {
        let after = plain(&[4, 2, 1], "a much longer string").unwrap();
        assert!(after.contains(r#"[3, "a much longer string"]"#), "{after}");
        // And the file still parses as the shape it was.
        let d = doc(after.as_bytes());
        let mut ev = Evaluator::new(Template::new("json", T::json()));
        assert_eq!(ev.node(&d, &[4]).unwrap().child_count, 3);
        assert_eq!(ev.node(&d, &[4, 2, 0]).unwrap().value, Value::Int(3));
        assert_eq!(ev.node(&d, &[3]).unwrap().name, "gone");
    }

    #[test]
    fn a_value_is_offered_for_editing_as_the_value_rather_than_the_literal() {
        let text = r#"{"s": "a \"quoted\" é", "n": 1.5e10, "on": true, "gone": null, "o": {}}"#;
        let d = doc(text.as_bytes());
        let mut ev = Evaluator::new(Template::new("json", T::json()));
        assert_eq!(ev.text_value(&d, &[0]).unwrap().0, "a \"quoted\" \u{e9}");
        // A number keeps the form the file gave it rather than being rounded
        // through a reading of it.
        assert_eq!(ev.text_value(&d, &[1]).unwrap().0, "1.5e10");
        assert_eq!(ev.text_value(&d, &[2]).unwrap().0, "true");
        assert_eq!(ev.text_value(&d, &[3]).unwrap().0, "null");
        assert!(ev.text_value(&d, &[4]).is_err(), "an object is its members");
    }

    #[test]
    fn a_number_is_edited_as_the_digits_the_file_wrote() {
        let text = r#"{"a": 1.0, "b": 1.5e10, "c": 3}"#;
        let d = doc(text.as_bytes());
        let mut ev = Evaluator::new(Template::new("json", T::json()));
        let starts: Vec<_> = (0..3).map(|i| ev.node(&d, &[i]).unwrap().edit_text.unwrap()).collect();
        assert_eq!(starts, ["1.0", "1.5e10", "3"]);
        // Opening a field and applying it unchanged leaves the file alone,
        // which reading the number and writing it back would not.
        for (i, start) in starts.iter().enumerate() {
            let w = ev.prepare_write(&d, &[i], start).unwrap();
            assert_eq!(w.n_bits, w.old_bits, "{start}");
            assert_eq!(w.data, start.as_bytes());
        }
    }

    #[test]
    fn json_inside_a_sized_record_is_as_fixed_as_json_sized_outright() {
        let body = br#"{"a": "xy"}"#;
        let mut bytes = vec![body.len() as u8];
        bytes.extend_from_slice(body);
        let t = Template::new(
            "wrapped",
            T::structure(
                "Root",
                vec![
                    ("len", T::u8()),
                    ("rec", T::sized(E::field("len"), T::structure("Rec", vec![("hdr", T::json())]))),
                ],
            ),
        );
        let err = write(&bytes, t, &[1, 0, 0], "longer").unwrap_err();
        assert!(err.contains("Length is fixed here"), "{err}");
    }

    #[test]
    fn an_array_and_an_object_are_not_scalars() {
        // Compared against the constant rather than against words in it, so
        // that rewording the refusal stays a question for the refusal.
        for path in [vec![4], vec![4, 2], vec![]] {
            let err = plain(&path, "[]").unwrap_err();
            assert_eq!(err, crate::encode::NOT_EDITABLE_MSG, "{path:?} was refused with someone else's message");
        }
    }

    /// A header whose length sits in front of it, the way safetensors writes
    /// one. Changing the length would leave that number wrong.
    fn sized() -> (Vec<u8>, Template) {
        let body = br#"{"a": 1, "b": "xy"}"#;
        let mut bytes = (body.len() as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(body);
        let t = Template::new(
            "sized",
            T::structure("Root", vec![("len", T::u32(Big)), ("header", T::sized(E::field("len"), T::json()))]),
        );
        (bytes, t)
    }

    #[test]
    fn a_json_field_with_a_recorded_length_keeps_it() {
        let (bytes, t) = sized();
        let err = write(&bytes, t, &[1, 1], "longer").unwrap_err();
        assert!(err.contains("Length is fixed here"), "{err}");
    }

    #[test]
    fn the_same_length_is_still_written_there() {
        let (bytes, t) = sized();
        let after = write(&bytes, t, &[1, 1], "ab").unwrap();
        assert!(after.ends_with(br#"{"a": 1, "b": "ab"}"#), "{after:?}");
        assert_eq!(after.len(), bytes.len());
    }
}

/// An address counted from where this copy of a format begins, with a window
/// between it and that origin. `Anchor::Window` would answer with the window,
/// which is the whole reason the origin marker exists: an HDF5 message body is
/// sized, and an address written inside one counts from the front of the HDF5
/// file rather than from the front of the message.
#[test]
fn an_origin_is_counted_from_past_the_windows_between() {
    // Inside the copy: a sized box holding a pointer, and the thing it points
    // at eight bytes into the copy.
    let target = T::structure("Target", vec![("value", T::u16(Big))]);
    let boxed = T::sized(E::lit(4), T::structure("Box", vec![("at", T::u16(Big)), ("target", T::at_origin(E::field("at"), target))]));
    let copy = T::origin(T::structure("Copy", vec![("head", T::u16(Big)), ("boxed", boxed)]));
    let t = Template::new("t", T::structure("File", vec![("lead", T::array(T::u8(), E::lit(3))), ("copy", copy)]));

    // Three bytes of lead, then the copy: a header, a box saying 8, padding,
    // and at offset 8 of the copy the two bytes pointed at.
    let d = doc(&[0xee, 0xee, 0xee, 0, 1, 0, 8, 0, 0, 0, 0, 0xbe, 0xef]);
    let mut ev = Evaluator::new(t);

    // The target sits at 3 + 8 = 11, not at the start of the sized box.
    let node = ev.node(&d, &[1, 1, 1, 0, 0]).expect("target");
    assert_eq!(node.offset_bits / 8, 11);
    assert_eq!(node.value, Value::UInt(0xbeef));
}

/// With no origin around it, the same address counts from the front of the
/// file. That is what makes one layout serve a format read on its own and the
/// same format written into the middle of another file.
#[test]
fn an_origin_anchor_with_no_origin_counts_from_the_file() {
    let target = T::structure("Target", vec![("value", T::u16(Big))]);
    let t = Template::new(
        "t",
        T::structure("File", vec![("at", T::u16(Big)), ("target", T::at_origin(E::field("at"), target))]),
    );
    let d = doc(&[0, 4, 0, 0, 0xbe, 0xef]);
    let mut ev = Evaluator::new(t);
    let node = ev.node(&d, &[1, 0, 0]).expect("target");
    assert_eq!(node.offset_bits / 8, 4);
    assert_eq!(node.value, Value::UInt(0xbeef));
}

// ----- the whole web of connections under one node -----

/// A count in front of a run and a length in front of some bytes: the two
/// shapes nearly every format is made of, and the two arrows a graph has to
/// get the right way round.
fn connected() -> Template {
    Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("n", T::u8()),
                ("len", T::u8()),
                ("items", T::array(T::u16(Big), E::field("n"))),
                ("data", T::bytes(E::field("len"))),
            ],
        ),
    )
}

/// Where the node with this path ended up in the list, so that a test can name
/// a field the way the template does rather than by counting the walk.
fn node_at(g: &Graph, path: &[usize]) -> usize {
    g.nodes.iter().position(|n| n.path == path).unwrap_or_else(|| panic!("no node at {path:?}"))
}

/// Every edge as the two fields it joins, which is what the assertion is
/// about: the indices are an artefact of the order the walk happened to run in.
fn joined(g: &Graph) -> Vec<(&[usize], &[usize], &'static str)> {
    g.edges.iter().map(|e| (g.nodes[e.from].path.as_slice(), g.nodes[e.to].path.as_slice(), e.role)).collect()
}

#[test]
fn a_graph_draws_an_arrow_from_the_field_that_decided() {
    // Two items and three bytes of data.
    let d = doc(&[2, 3, 0, 1, 0, 2, 7, 7, 7]);
    let mut ev = Evaluator::new(connected());
    let g = ev.graph(&d, &[], 100).unwrap();

    // The root, its four fields, and then the two items: breadth-first, so
    // every field arrives before anything inside one of them.
    let paths: Vec<&[usize]> = g.nodes.iter().map(|n| n.path.as_slice()).collect();
    assert_eq!(paths, vec![&[][..], &[0], &[1], &[2], &[3], &[2, 0], &[2, 1]]);
    assert_eq!(g.omitted, 0);
    assert!(g.nodes.iter().all(|n| !n.truncated));

    // What each node is called and where it sits comes from the memo, without
    // reading a single field's value.
    assert_eq!(g.nodes[node_at(&g, &[2])].name, "items");
    assert_eq!(g.nodes[node_at(&g, &[2, 1])].name, "[1]");
    assert_eq!(g.nodes[node_at(&g, &[3])].offset_bits / 8, 6);
    assert_eq!(g.nodes[node_at(&g, &[3])].size_bits / 8, 3);
    assert_eq!(g.nodes[node_at(&g, &[2])].child_count, 2);

    // The root has nothing above it; everything else names the node it hangs
    // under.
    assert_eq!(g.nodes[0].parent, NO_PARENT);
    assert_eq!(g.nodes[node_at(&g, &[2, 0])].parent, node_at(&g, &[2]));

    // The kinds a view groups by: the resolved type, said coarsely.
    let kinds: Vec<&str> = g.nodes.iter().map(|n| n.kind.as_str()).collect();
    assert_eq!(kinds, vec!["struct", "u8", "u8", "array", "bytes", "u16", "u16"]);

    // And the arrows, each running from the field that decided to the field it
    // decided about.
    assert_eq!(joined(&g), vec![(&[0][..], &[2][..], "count"), (&[1][..], &[3][..], "length")]);
}

#[test]
fn an_arrow_from_outside_the_subtree_is_dropped_rather_than_left_hanging() {
    // The same file, asked only about the run. What said how many items there
    // are is a field of the root, which is not in this graph, so the arrow has
    // nowhere to come from and is left out rather than pointed at nothing.
    let d = doc(&[2, 3, 0, 1, 0, 2, 7, 7, 7]);
    let mut ev = Evaluator::new(connected());
    let g = ev.graph(&d, &[2], 100).unwrap();
    let paths: Vec<&[usize]> = g.nodes.iter().map(|n| n.path.as_slice()).collect();
    assert_eq!(paths, vec![&[2][..], &[2, 0], &[2, 1]]);
    assert!(g.edges.is_empty(), "{:?}", joined(&g));
    // The subtree's own node is its root here, whatever it is a child of in
    // the file.
    assert_eq!(g.nodes[0].parent, NO_PARENT);
}

#[test]
fn a_capped_graph_keeps_the_shape_and_says_what_it_left_out() {
    let d = doc(&[2, 3, 0, 1, 0, 2, 7, 7, 7]);
    let mut ev = Evaluator::new(connected());
    let g = ev.graph(&d, &[], 2).unwrap();
    // Two nodes, and they are the top two. A cap on a depth-first walk would
    // have kept the root and then dived into the first field, and said nothing
    // about the shape of the file.
    let paths: Vec<&[usize]> = g.nodes.iter().map(|n| n.path.as_slice()).collect();
    assert_eq!(paths, vec![&[][..], &[0]]);
    // The root still says how many fields it has, and says that most of them
    // were not walked.
    assert_eq!(g.nodes[0].child_count, 4);
    assert!(g.nodes[0].truncated);
    assert!(!g.nodes[1].truncated);
    assert_eq!(g.omitted, 3);
    // With the deciding fields' targets outside the cap there is nothing for
    // an arrow to join.
    assert!(g.edges.is_empty(), "{:?}", joined(&g));
}

/// The same connections `origins` gives, minus what the fields say. This is
/// what makes a graph of a large file affordable: reading the values is the
/// expensive half, and no arrow shows them.
#[test]
fn a_graph_asks_for_the_connections_without_reading_the_values() {
    let d = doc(&[2, 3, 0, 1, 0, 2, 7, 7, 7]);
    let mut ev = Evaluator::new(connected());
    let told = ev.origins(&d, &[3]).unwrap();
    assert_eq!(told.len(), 1);
    assert_eq!((told[0].role, told[0].label.as_str(), told[0].value.as_str()), (Role::Length, "len", "3"));

    let mut bare = Evaluator::new(connected());
    let quiet = bare.origins_no_values(&d, &[3]).unwrap();
    assert_eq!(quiet.len(), 1);
    assert_eq!((quiet[0].role, quiet[0].label.as_str(), quiet[0].value.as_str()), (Role::Length, "len", ""));
    assert_eq!(quiet[0].path, told[0].path);
}

/// A pointer list's offset and the child it placed are two ends of one
/// connection, and the graph draws both: the offset placed the record, and the
/// offset points at it. The second is the only arrow that runs outward from
/// the field holding the number rather than inward to it.
#[test]
fn a_pointer_and_what_it_points_at_are_joined_both_ways() {
    let d = doc(POINTED);
    let mut ev = Evaluator::new(pointer_template());
    let g = ev.graph(&d, &[], 100).unwrap();
    let seen = joined(&g);
    assert!(seen.contains(&(&[1, 0][..], &[2, 0][..], "position")), "{seen:?}");
    assert!(seen.contains(&(&[1, 0][..], &[2, 0][..], "points")), "{seen:?}");
    assert!(seen.contains(&(&[1, 1][..], &[2, 1][..], "position")), "{seen:?}");
    // And a length inside one of the records reads the same as any other.
    assert!(seen.contains(&(&[2, 0, 0][..], &[2, 0, 1][..], "length")), "{seen:?}");
}

// ----- what the file's bytes went on -----

/// Run the whole-file walk to the end, and say how many goes it took. Each go
/// gets a fresh allowance, which is what the host does between frames.
fn kinds_to_the_end(ev: &mut Evaluator, d: &Document<MemSource>) -> (KindTotals, usize) {
    let mut walk = KindWalk::new(d.len_bits());
    for goes in 1..100_000 {
        ev.begin_slice();
        let out = ev.kind_totals_step(d, &mut walk).expect("nothing to fetch, nothing to fail");
        if out.done {
            return (out, goes);
        }
    }
    panic!("the walk never finished");
}

/// What one entry of the answer says, for an assertion that does not care what
/// order the entries came in.
fn spent(totals: &KindTotals, kind: &str, type_name: &str) -> (u64, u64) {
    totals
        .totals
        .iter()
        .find(|t| t.kind == kind && t.type_name == type_name)
        .map_or((0, 0), |t| (t.bits, t.count))
}

/// Every bit of a file is either on some field or on nothing, and the two add
/// up to the file. A structure that does not fill the file leaves the rest
/// unmapped, which is what a reader asking where a format spends its bytes
/// most wants told.
#[test]
fn kind_totals_account_for_every_bit_of_a_small_file() {
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![("magic", T::magic(b"QB")), ("count", T::u16(Big)), ("codes", T::array(T::u8(), E::lit(4)))],
        ),
    );
    let d = doc(&[b'Q', b'B', 0, 4, 1, 2, 3, 4, 0xff, 0xff]);
    let mut ev = Evaluator::new(t);
    let (out, _) = kinds_to_the_end(&mut ev, &d);

    assert_eq!(spent(&out, "magic", "magic[2]"), (16, 1));
    assert_eq!(spent(&out, "uint", "u16 be"), (16, 1));
    // Four bytes as four fields, from one element's walk: see the run below.
    assert_eq!(spent(&out, "uint", "u8"), (32, 4));
    assert_eq!(out.covered_bits, 8 * 8);
    // The two bytes past the structure are described by nothing.
    assert_eq!(out.unmapped_bits, 2 * 8);
    assert_eq!(out.reached_bits, d.len_bits());
    assert_eq!(out.covered_bits + out.unmapped_bits, d.len_bits());
    // The structure's own bits are its children's, and counting them again
    // would say the file is twice the size it is.
    assert_eq!(spent(&out, "composite", "Root"), (0, 0));
}

/// A run of same-sized elements is counted by arithmetic, not by walking it.
/// Two hundred thousand records walked one at a time is two hundred thousand
/// goes of a bounded allowance and as many nodes; walked once and multiplied
/// it is one go, which is what makes the question answerable for a file whose
/// weights are the whole of it.
#[test]
fn a_fixed_size_run_is_counted_without_walking_it() {
    const N: usize = 200_000;
    let t = Template::new(
        "t",
        T::array(T::structure("Pair", vec![("lo", T::u16(Big)), ("hi", T::u16(Big))]), E::lit(N as i128)),
    );
    let d = doc(&vec![7u8; N * 4]);
    let mut ev = Evaluator::new(t);
    // Small enough that walking the run would run out hundreds of times over.
    ev.set_slice(Some(1_000));
    let (out, goes) = kinds_to_the_end(&mut ev, &d);

    assert_eq!(goes, 1, "a run counted by arithmetic takes one go");
    assert_eq!(spent(&out, "uint", "u16 be"), (N as u64 * 2 * 16, N as u64 * 2));
    assert_eq!(out.covered_bits, d.len_bits());
    assert_eq!(out.unmapped_bits, 0);
    // One element's worth of nodes, not two hundred thousand.
    assert!(ev.memo_len() < 32, "{} nodes left behind", ev.memo_len());
}

/// A run whose elements are each as long as their own bytes say has to be
/// walked, and a walk that long does not fit in one go. It has to come back
/// where it left off, count every element once, and not remember them.
#[test]
fn a_variable_length_run_resumes_and_is_not_remembered() {
    const N: usize = 5_000;
    let t = Template::new("t", T::array(T::cstr(), E::lit(N as i128)));
    let mut bytes = Vec::new();
    for _ in 0..N {
        bytes.extend_from_slice(b"ab\0");
    }
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);
    ev.set_slice(Some(200));
    let (out, goes) = kinds_to_the_end(&mut ev, &d);

    assert!(goes > 1, "five thousand strings do not fit in one go of two hundred");
    assert_eq!(spent(&out, "str", "cstr"), (N as u64 * 3 * 8, N as u64));
    assert_eq!(out.covered_bits, d.len_bits());
    assert_eq!(out.unmapped_bits, 0);
    // The walk drops what it has gone past, so what is left is the window it
    // keeps and not five thousand strings.
    assert!(ev.memo_len() < 64, "{} nodes left behind", ev.memo_len());
}

/// A run that fills its container with elements whose length their own bytes
/// give cannot be counted without decoding all of it, so the walk counts them
/// as it goes and stops when the room runs out.
#[test]
fn a_run_with_no_count_is_walked_to_the_end_of_its_room() {
    let t = Template::new("t", T::repeat(T::cstr(), Until::End));
    let d = doc(b"one\0two\0three\0");
    let mut ev = Evaluator::new(t);
    let (out, _) = kinds_to_the_end(&mut ev, &d);
    assert_eq!(spent(&out, "str", "cstr"), (14 * 8, 3));
    assert_eq!(out.covered_bits, d.len_bits());
    assert_eq!(out.unmapped_bits, 0);
}

/// A field that points back at bytes its siblings have already described is a
/// second reading of them, not more of the file. npy writes its dtype as text
/// and then a record view over the same text; xz lists its blocks and then
/// points at the whole stream to say what it unpacks to. Counting both would
/// say the file is twice the size it is.
#[test]
fn a_view_over_bytes_the_fields_describe_is_not_counted_twice() {
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![("a", T::u32(Big)), ("view", T::at_in_window(E::lit(0), T::bytes(E::lit(4))))],
        ),
    );
    let d = doc(&[1, 2, 3, 4]);
    let mut ev = Evaluator::new(t);
    let (out, _) = kinds_to_the_end(&mut ev, &d);
    assert_eq!(spent(&out, "uint", "u32 be"), (32, 1));
    assert_eq!(spent(&out, "bytes", "bytes[]"), (0, 0));
    assert_eq!(out.covered_bits, 32);
    assert_eq!(out.unmapped_bits, 0);

    // The same field pointing somewhere the run has not covered is a field
    // like any other: this is how an AppleDouble reaches its data fork.
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![("a", T::u32(Big)), ("fork", T::at_in_window(E::lit(4), T::bytes(E::lit(4))))],
        ),
    );
    let d = doc(&[1, 2, 3, 4, 5, 6, 7, 8]);
    let mut ev = Evaluator::new(t);
    let (out, _) = kinds_to_the_end(&mut ev, &d);
    assert_eq!(spent(&out, "bytes", "bytes[]"), (32, 1));
    assert_eq!(out.covered_bits, 64);
    assert_eq!(out.unmapped_bits, 0);
}

/// Two addresses naming the same thing reach it twice and count it once. An
/// HDF5 group can hold any number of links to one object, and the template
/// follows every one, since each is a way a reader gets there; counted every
/// time, a group of two thousand links to one dataset covers more bits than
/// its file has.
///
/// Something else at the same place is not the same thing: an address to the
/// same start with a different length still counts.
#[test]
fn a_thing_two_addresses_name_is_counted_once() {
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("first", T::u8()),
                ("second", T::u8()),
                ("one", T::at(E::field("first"), T::structure("Thing", vec![("a", T::u16(Big))]))),
                ("two", T::at(E::field("second"), T::structure("Thing", vec![("a", T::u16(Big))]))),
                ("wider", T::at(E::field("first"), T::bytes(E::lit(4)))),
            ],
        ),
    );
    let d = doc(&[4, 4, 0, 0, 1, 2, 3, 4]);
    let mut ev = Evaluator::new(t);
    let (out, _) = kinds_to_the_end(&mut ev, &d);
    assert_eq!(spent(&out, "uint", "u16 be"), (16, 1));
    assert_eq!(spent(&out, "bytes", "bytes[]"), (32, 1));
    assert!(out.covered_bits <= d.len_bits(), "{out:?}");
}

/// Bytes a structure does not cover are a gap. Nothing but the gap is
/// recorded: a composite's own bits are its children's, and the only bits it
/// contributes are the ones it is left holding.
#[test]
fn a_composite_contributes_only_what_its_children_leave_over() {
    // A window of eight bytes holding a structure that fills four.
    let t = Template::new("t", T::sized(E::lit(8), T::structure("Head", vec![("a", T::u32(Big))])));
    let d = doc(&[0; 8]);
    let mut ev = Evaluator::new(t);
    let (out, _) = kinds_to_the_end(&mut ev, &d);
    assert_eq!(spent(&out, "uint", "u32 be"), (32, 1));
    assert_eq!(out.unmapped_bits, 32);
    assert_eq!(out.covered_bits + out.unmapped_bits, d.len_bits());
}

/// The prose a format carries about a field reaches the node, and where a
/// field has none, the structure it is speaks for it.
#[test]
fn a_node_carries_what_the_format_says_about_it() {
    let chunk = || {
        T::structure("Chunk", vec![("len", T::u8()), ("body", T::bytes(E::field("len")))])
            .doc("A length and the bytes it counts.")
            .field_doc("len", "How many bytes of body follow.")
    };
    let t = Template::new("t", T::structure("Root", vec![("first", chunk()), ("second", chunk())]));
    let d = doc(&[2, 7, 8, 1, 9]);
    let mut ev = Evaluator::new(t);

    // The declaration's own prose, on the field that has it.
    assert_eq!(ev.node(&d, &[0, 0]).unwrap().doc.as_deref(), Some("How many bytes of body follow."));
    // A field with none of its own: nothing, since `body` is a run of bytes
    // and no structure speaks for it.
    assert_eq!(ev.node(&d, &[0, 1]).unwrap().doc, None);
    // The structure's own prose, on a field declared as that structure.
    assert_eq!(ev.node(&d, &[1]).unwrap().doc.as_deref(), Some("A length and the bytes it counts."));
    // A structure nobody wrote prose for says nothing.
    assert_eq!(ev.node(&d, &[]).unwrap().doc, None);
}

/// What an enum value means is a fact about the value, kept beside it in the
/// definition rather than folded into the field's own prose.
#[test]
fn an_enum_value_keeps_its_own_prose() {
    let ty = T::enumeration("Method", T::u8(), &[(0, "stored"), (8, "deflate")])
        .enum_doc(0, "The bytes as they are, with no compression at all.");
    let Ty::Enum { def, .. } = &ty else { panic!("not an enum") };
    assert_eq!(def.doc_of(0), Some("The bytes as they are, with no compression at all."));
    assert_eq!(def.doc_of(8), None);
    // It is the value's, not the field's: the node goes on saying nothing.
    let t = Template::new("t", T::structure("Root", vec![("method", ty.clone())]));
    let d = doc(&[0]);
    assert_eq!(Evaluator::new(t).node(&d, &[0]).unwrap().doc, None);
}

/// A tagged search reaches a list that lives behind an address, and finds the
/// element whose label the record asking works out for itself.
///
/// Both halves matter and neither was proven before. The list is named by a
/// path down into a field whose contents are somewhere else, so the path has
/// to step through the `At` the way every other path does. The label is not a
/// number the template fixed but one written in the record asking, which is
/// how a variable-length element says which object of a heap collection holds
/// its bytes.
#[test]
fn a_tagged_search_reaches_a_list_through_an_at() {
    let element = || {
        T::structure(
            "Element",
            vec![("index", T::u8()), ("size", T::u8()), ("payload", T::bytes(E::field("size")))],
        )
    };
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("collection_address", T::u8()),
                ("want", T::u8()),
                (
                    "collection",
                    T::at(
                        E::field("collection_address"),
                        T::structure("Collection", vec![("elements", T::repeat(element(), Until::End))]),
                    ),
                ),
                (
                    "found",
                    T::computed(E::tagged_in_by(
                        E::within(&["collection", "elements"]),
                        &["index"],
                        E::field("want"),
                        &["size"],
                    )),
                ),
            ],
        ),
    );
    // At byte 4: element 7 of one byte, element 3 of two, element 5 of three.
    let d = doc(&[4, 3, 0, 0, 7, 1, b'a', 3, 2, b'b', b'c', 5, 3, b'd', b'e', b'f']);
    let mut ev = Evaluator::new(t);
    let found = ev.node(&d, &[3]).unwrap();
    assert_eq!(found.value.as_int(), Some(2), "the search should land on the element labelled 3");

    // And the path it lands on is the one inside the collection, so a reader
    // asking where the answer came from is sent to those bytes.
    let here = ev.memo.get(&vec![3usize]).map(|r| (r.offset, r.limit));
    let search = match &ev.template.root {
        Ty::Struct(s) => match &s.fields[3].ty {
            Ty::Computed(Expr::Tagged(t)) => t.clone(),
            other => panic!("not a tagged search: {other:?}"),
        },
        other => panic!("not a structure: {other:?}"),
    };
    let (path, label) =
        ev.tagged_path(&d, &[3], &search, here).unwrap().expect("the search should find an element");
    // Root, then the collection field, then the one child of its `At`, then
    // the list, then the second element, then the field read from it.
    assert_eq!(path, vec![2, 0, 0, 1, 1]);
    assert_eq!(label, "collection.elements[1].size");
}

/// A collection several records reach by address is read once, however many of
/// them reach it.
///
/// This is the difference between a column of variable-length strings that can
/// be read and one that cannot. Each of them carries the address of the heap
/// collection its bytes are in, so each is a path of its own to the same
/// stretch of the file, and walking that stretch once per string is quadratic
/// in the length of the column.
#[test]
fn a_tagged_search_over_a_list_reached_by_address_is_walked_once() {
    let searching = |field: &[&str]| {
        T::computed(E::tagged_in_by(
            E::within(&["collection", "elements"]),
            &["index"],
            E::field("want"),
            field,
        ))
    };
    let reference = T::structure(
        "Reference",
        vec![
            ("address", T::u8()),
            ("want", T::u8()),
            (
                "collection",
                T::at(
                    E::field("address"),
                    T::structure(
                        "Collection",
                        vec![(
                            "elements",
                            T::repeat(
                                T::structure("Element", vec![("index", T::u8()), ("size", T::u8())]),
                                Until::End,
                            ),
                        )],
                    ),
                ),
            ),
            ("found", searching(&["size"])),
        ],
    );
    let t = Template::new("t", T::structure("Root", vec![("refs", T::array(reference, E::lit(2)))]));

    // Two references to one collection of two hundred elements, both after
    // the same one, which is a long way in.
    let mut bytes = vec![4u8, 150, 4, 150];
    for i in 0..200u8 {
        bytes.push(i);
        bytes.push(i / 4);
    }
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);

    assert_eq!(ev.node(&d, &[0, 0, 3]).unwrap().value.as_int(), Some(150 / 4));
    let after_first = ev.memo.len();
    assert_eq!(ev.node(&d, &[0, 1, 3]).unwrap().value.as_int(), Some(150 / 4));
    let grew = ev.memo.len() - after_first;

    // The second reference places its own note and its own view of the
    // collection, and nothing else: the elements it would have walked are
    // already read, under the first reference's path.
    assert!(grew < 12, "the second reference read {grew} more nodes, so it walked the collection again");
    assert!(!ev.memo.contains_key(&vec![0usize, 1, 2, 0, 0, 0]), "the second reference placed an element of its own");
}

/// A label is found wherever in the list it was written, and a search that
/// picks up where an earlier one stopped still answers with the first element
/// carrying the label.
///
/// A global heap does not renumber itself: rewriting a string leaves a new
/// object at the end of the collection with the index the old one had, so the
/// indices are in whatever order the writing happened in.
#[test]
fn a_tagged_search_finds_an_element_written_out_of_order() {
    let reference = T::structure(
        "Reference",
        vec![
            ("address", T::u8()),
            ("want", T::u8()),
            (
                "collection",
                T::at(
                    E::field("address"),
                    T::structure(
                        "Collection",
                        vec![(
                            "elements",
                            T::repeat(
                                T::structure(
                                    "Element",
                                    vec![("index", T::u8()), ("size", T::u8()), ("payload", T::bytes(E::field("size")))],
                                ),
                                Until::End,
                            ),
                        )],
                    ),
                ),
            ),
            (
                "found",
                T::computed(E::tagged_in_by(
                    E::within(&["collection", "elements"]),
                    &["index"],
                    E::field("want"),
                    &["size"],
                )),
            ),
        ],
    );
    let t = Template::new("t", T::structure("Root", vec![("refs", T::array(reference, E::lit(3)))]));

    // Three references, and a list with the label 7 in it twice. The first
    // search stops in the middle of the list, the second carries on from there
    // and passes the second 7 on its way, and the third asks for 7: the answer
    // is the first element carrying it, which is the one the search would have
    // reached and is behind where the walk had got to.
    let d = doc(&[
        6, 5, 6, 9, 6, 7, //
        7, 1, b'a', //
        3, 2, b'b', b'c', //
        5, 3, b'd', b'e', b'f', //
        7, 4, b'g', b'h', b'i', b'j', //
        9, 1, b'k',
    ]);
    let mut ev = Evaluator::new(t);
    assert_eq!(ev.node(&d, &[0, 0, 3]).unwrap().value.as_int(), Some(3), "label 5 is the third element");
    assert_eq!(ev.node(&d, &[0, 1, 3]).unwrap().value.as_int(), Some(1), "label 9 is the last");
    assert_eq!(ev.node(&d, &[0, 2, 3]).unwrap().value.as_int(), Some(1), "label 7 is the first, not the fourth");
}

/// Where a field is, rather than what it says.
#[test]
fn start_of_names_where_an_earlier_field_begins() {
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("head", T::u16(Big)),
                ("body", T::bytes(E::lit(4))),
                ("body_at", T::computed(E::start_of(E::field("body")))),
                // Through a path into an earlier field, and through a list,
                // which is what a search hands it.
                ("inner", T::structure("Inner", vec![("a", T::u8()), ("b", T::u8())])),
                ("b_at", T::computed(E::start_of(E::within(&["inner", "b"])))),
            ],
        ),
    );
    let d = doc(&[0, 1, 2, 3, 4, 5, 6, 7]);
    let mut ev = Evaluator::new(t);
    assert_eq!(ev.node(&d, &[2]).unwrap().value.as_int(), Some(2));
    assert_eq!(ev.node(&d, &[4]).unwrap().value.as_int(), Some(7));
}

/// And counted from where this copy of the format begins, so that it reads as
/// an address of the format rather than as a place in whatever holds it.
///
/// An HDF5 file behind a 512-byte user block writes every address as if the
/// block were not there, and a MATLAB 7.3 file is exactly that. An answer
/// counted from the front of the file would be 512 too large and would be
/// silently right on every file without one.
#[test]
fn start_of_counts_from_the_nearest_origin() {
    let inner = || {
        T::structure(
            "Inner",
            vec![("head", T::u16(Big)), ("body", T::bytes(E::lit(4))), ("body_at", T::computed(E::start_of(E::field("body"))))],
        )
    };
    let d = doc(&[0; 16]);

    let plain = Template::new("t", T::structure("Root", vec![("preamble", T::bytes(E::lit(3))), ("inner", inner())]));
    let mut ev = Evaluator::new(plain);
    assert_eq!(ev.node(&d, &[1, 2]).unwrap().value.as_int(), Some(5), "from the front of the file");

    let framed =
        Template::new("t", T::structure("Root", vec![("preamble", T::bytes(E::lit(3))), ("inner", T::origin(inner()))]));
    let mut ev = Evaluator::new(framed);
    // An origin is a marker and takes no level of its own, so the field is in
    // the same place it was.
    assert_eq!(ev.node(&d, &[1, 2]).unwrap().value.as_int(), Some(2), "from the front of this copy of the format");
}

/// A field placed at where a search landed covers that element's bytes: the
/// two halves together are how the Nth element of a list whose elements vary
/// in size is read.
#[test]
fn an_at_placed_at_a_found_element_covers_its_bytes() {
    let found = |field: &[&str]| {
        E::tagged_in_by(E::within(&["collection", "elements"]), &["index"], E::field("want"), field)
    };
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("address", T::u8()),
                ("want", T::u8()),
                (
                    "collection",
                    T::at(
                        E::field("address"),
                        T::structure(
                            "Collection",
                            vec![(
                                "elements",
                                T::repeat(
                                    T::structure(
                                        "Element",
                                        vec![
                                            ("index", T::u8()),
                                            ("size", T::u8()),
                                            ("payload", T::bytes(E::field("size"))),
                                        ],
                                    ),
                                    Until::End,
                                ),
                            )],
                        ),
                    ),
                ),
                (
                    "object",
                    T::at(
                        E::start_of(found(&["payload"])),
                        T::sized(found(&["size"]), T::text(StrLen::Fixed(E::Remaining), Encoding::Utf8)),
                    ),
                ),
            ],
        ),
    );
    // At byte 2: element 7 of one byte, element 3 of two, element 5 of three.
    let d = doc(&[2, 5, 7, 1, b'a', 3, 2, b'b', b'c', 5, 3, b'd', b'e', b'f']);
    let mut ev = Evaluator::new(t);
    let object = ev.node(&d, &[3, 0]).unwrap();
    assert_eq!(object.value, Value::Str("def".into()));
    assert_eq!(object.offset_bits / 8, 11, "the third element's payload starts at byte 11");
    assert_eq!(object.size_bits, 3 * 8);
}

/// A source that says how many times it was asked for bytes, so that a saving
/// in reading rather than in memory can be seen.
struct Counting {
    bytes: Vec<u8>,
    reads: std::cell::Cell<usize>,
}

impl crate::source::Source for Counting {
    fn len_bytes(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn read_bytes(&self, offset: u64, out: &mut [u8]) -> Vec<crate::source::Missing> {
        self.reads.set(self.reads.get() + 1);
        let o = offset as usize;
        out.copy_from_slice(&self.bytes[o..o + out.len()]);
        Vec::new()
    }
}

/// The other shape that asks, and the other kind of saving. Every cell of a
/// FITS table reads its column's `TFORM` card out of the header, and the
/// header is one list at one path, so nothing was placed twice: what a second
/// cell used to cost was reading every card's keyword again and comparing it.
///
/// Measured in reads rather than in nodes for that reason, and against two
/// headers of different lengths: what the second cell costs is now the same in
/// both, which is the whole claim.
#[test]
fn a_cell_finds_its_card_without_rewalking_the_header() {
    let cost_of_the_second_cell = |cards: usize| -> usize {
        let card = T::structure("Card", vec![("key", T::bytes(E::lit(8))), ("body", T::u8())]);
        let t = Template::new(
            "t",
            T::structure(
                "Root",
                vec![
                    ("cards", T::array(card, E::lit(cards as i128))),
                    (
                        "cells",
                        T::array(
                            T::structure(
                                "Cell",
                                vec![("form", T::computed(E::tagged_bytes("cards", &["key"], b"TFORM1  ", &["body"])))],
                            ),
                            E::lit(2),
                        ),
                    ),
                ],
            ),
        );
        // Filler cards, and then the one every cell is after, written last so
        // that a search from the front reads all of them.
        let mut bytes = Vec::new();
        for i in 0..cards - 1 {
            bytes.extend_from_slice(format!("FILLER{i:02}").as_bytes());
            bytes.push(i as u8);
        }
        bytes.extend_from_slice(b"TFORM1  ");
        bytes.push(9);
        let d = Document::new(Counting { bytes, reads: std::cell::Cell::new(0) });
        let mut ev = Evaluator::new(t);
        assert_eq!(ev.node(&d, &[1, 0, 0]).unwrap().value.as_int(), Some(9));
        let before = d.source().reads.get();
        assert_eq!(ev.node(&d, &[1, 1, 0]).unwrap().value.as_int(), Some(9));
        d.source().reads.get() - before
    };
    let short = cost_of_the_second_cell(4);
    let long = cost_of_the_second_cell(40);
    assert_eq!(short, long, "the second cell still pays for the length of the header");
}

/// A field placed past the end of the root, by a choice made on text, is found
/// by the placement index. ROOT picks what a key's offset leads to by the class
/// name written in the key, and the index pruned every choice made that way as
/// placing nothing, so an RNTuple's envelopes and pages read as one gap.
#[test]
fn a_placement_behind_a_choice_by_text_is_indexed() {
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("class", T::utf8(E::lit(1))),
                ("offset", T::u8()),
                ("pick", T::matches(E::field("class"), vec![("k", T::at(E::field("offset"), T::u32(Big)))], T::bytes(E::lit(0)))),
            ],
        ),
    );
    let d = doc(&[b'k', 8, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4]);
    let mut ev = Evaluator::new(t);
    let spans = ev.spans(&d, 8 * 8, 12 * 8, 100).unwrap();
    assert_eq!(spans.len(), 1, "{spans:?}");
    assert!(!spans[0].gap, "the placed number reads as a gap: {spans:?}");
    assert_eq!((spans[0].offset_bits, spans[0].value.clone()), (8 * 8, Value::UInt(0x0102_0304)));
}

/// A field placed inside what the root covers, by a field nested below the
/// structure the walk down stops in, is found by the placement index as well.
/// An AppleDouble attribute's value is placed by the attribute, three levels
/// inside the entry before it, and the entry does not cover it.
#[test]
fn a_placement_inside_the_root_by_a_nested_field_is_found() {
    let entry = T::structure(
        "Entry",
        vec![("offset", T::u8()), ("len", T::u8()), ("value", T::at(E::field("offset"), T::bytes(E::field("len"))))],
    );
    let t = Template::new("t", T::sized(E::lit(8), T::structure("Root", vec![("entry", entry)])));
    let d = doc(&[4, 3, 0, 0, 7, 8, 9, 0]);
    let mut ev = Evaluator::new(t);
    let path = ev.locate(&d, 5 * 8).unwrap();
    let found = ev.node(&d, &path).unwrap();
    assert_eq!((found.offset_bits, found.size_bits), (4 * 8, 3 * 8), "{path:?}");
    let spans = ev.spans(&d, 0, 8 * 8, 100).unwrap();
    let named: Vec<_> = spans.iter().map(|s| (s.offset_bits / 8, s.size_bits / 8, s.gap)).collect();
    assert_eq!(named, vec![(0, 1, false), (1, 1, false), (2, 2, true), (4, 3, false), (7, 1, true)]);
}

/// Two lists placed over the same stretch are both walked, and a bit only the
/// second has an element at is that element. An Impulse Tracker module puts
/// its instruments, samples and patterns each in a list over the whole file,
/// and with no instruments the first is empty: the index kept one of the
/// three, and every sample header read as a gap.
#[test]
fn lists_placed_over_the_same_stretch_are_each_asked() {
    let item = |name| T::structure(name, vec![("x", T::u16(Big))]);
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("na", T::u8()),
                ("a_offsets", T::array(T::u8(), E::field("na"))),
                ("nb", T::u8()),
                ("b_offsets", T::array(T::u8(), E::field("nb"))),
                ("a", T::at(E::lit(0), T::pointer_list("a_offsets", Anchor::File, E::lit(0), item("A")))),
                ("b", T::at(E::lit(0), T::pointer_list("b_offsets", Anchor::File, E::lit(0), item("B")))),
            ],
        ),
    );
    let d = doc(&[0, 1, 6, 0, 0, 0, 0xbe, 0xef]);
    let mut ev = Evaluator::new(t);
    let path = ev.locate(&d, 6 * 8).unwrap();
    assert_eq!(ev.node(&d, &path).unwrap().value, Value::UInt(0xbeef), "{path:?}");
    // The stretch between the root and the element is a gap, from where the
    // root ends to where the element begins.
    let spans = ev.spans(&d, 0, 8 * 8, 100).unwrap();
    let gaps: Vec<_> = spans.iter().filter(|s| s.gap).map(|s| (s.offset_bits / 8, s.size_bits / 8)).collect();
    assert_eq!(gaps, vec![(3, 3)], "{spans:?}");
}