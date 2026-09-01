use super::*;
use crate::source::MemSource;
use crate::template::{Anchor, Endian::*, Expr as E, Ty as T};

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
