use super::*;

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
