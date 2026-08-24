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
