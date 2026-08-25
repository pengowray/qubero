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
fn a_scanned_field_steps_over_its_separators_and_stops_at_the_next() {
    use crate::template::{Encoding, StrLen};
    let token = || {
        T::text(StrLen::Scan { skip: b" \t\r\n".to_vec(), ends: b" \t\r\n".to_vec() }, Encoding::Ascii)
    };
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
fn a_record_can_be_switched_on_a_byte_further_along_than_any_field() {
    // Two layouts of four bytes, told apart by the last of them, which comes
    // after the fields whose meaning it settles.
    let wide = T::structure("Wide", vec![("n", T::u16(Big)), ("pad", T::u8()), ("kind", T::u8())]);
    let narrow = T::structure(
        "Narrow",
        vec![("a", T::u8()), ("b", T::u8()), ("pad", T::u8()), ("kind", T::u8())],
    );
    let rec = T::switch(E::peek_at(3 * 8, 8), vec![(1, wide)], narrow);
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
    assert!(ev.locate(&d, (2 + 16 * 3 + 1) as u64 * 8).is_err());
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

    // The bytes that are there, and that they are not the bytes asked for.
    let wrong = doc(b"\x89PNh\r\n\x1a\n");
    let mut ev = Evaluator::new(Template::new("t", T::structure("Root", vec![("magic", T::magic(b"\x89PNG\r\n\x1a\n"))])));
    assert_eq!(
        listing::brief(&ev.node(&wrong, &[0]).unwrap().value),
        r#""\x89PNh\r\n\x1a\n" does not match"#
    );
}
