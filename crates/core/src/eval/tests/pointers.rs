use super::*;

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
