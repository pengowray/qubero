use super::*;

/// A float that is not a number is classified from the bits the file holds,
/// not from the double they were read into: an f32 signalling NaN widened to
/// a double comes back quiet, and a field read big-endian read the other way
/// round is a different NaN again.
#[test]
fn a_float_that_is_not_a_number_is_read_off_the_bytes() {
    let t = Template::new("t", T::structure("Root", vec![("sample", T::F32(Big))]));
    let mut ev = Evaluator::new(t);
    let d = doc(&[0x7f, 0x80, 0x00, 0x01]);
    let node = ev.node(&d, &[0]).unwrap();
    let problem = node.problem.expect("a NaN is worth saying");
    assert_eq!(problem.tier, crate::eval::Tier::Undefined);
    assert_eq!(problem.text, "Not a number (signalling NaN, payload 0x1)");

    // The same four bytes the other way round are an ordinary large number.
    let mut ev = Evaluator::new(Template::new("t", T::structure("Root", vec![("sample", T::F32(Little))])));
    assert_eq!(ev.node(&d, &[0]).unwrap().problem, None);

    // An infinity says which one, and a number says nothing.
    let mut ev = Evaluator::new(Template::new("t", T::structure("Root", vec![("sample", T::F64(Big))])));
    let inf = doc(&f64::NEG_INFINITY.to_bits().to_be_bytes());
    assert_eq!(ev.node(&inf, &[0]).unwrap().problem.expect("an infinity").text, "Negative infinity");
    let one = doc(&1.5f64.to_bits().to_be_bytes());
    assert_eq!(ev.node(&one, &[0]).unwrap().problem, None);
}

#[test]
fn a_signature_reads_as_the_string_it_is() {
    let t = Template::new("t", T::structure("Root", vec![("magic", T::magic(b"\x89PNG\r\n\x1a\n"))]));
    let mut ev = Evaluator::new(t);
    let d = doc(b"\x89PNG\r\n\x1a\n");
    assert_eq!(listing::brief(&ev.node(&d, &[0]).unwrap().value), r#""\x89PNG\r\n\x1a\n""#);

    // The bytes that are there. What was wanted instead is the field's
    // `problem`, not part of the value: a reader sees the bytes in the value
    // column and the reason beside them, in the same words in every view.
    let wrong = doc(b"\x89PNh\r\n\x1a\n");
    let mut ev = Evaluator::new(Template::new("t", T::structure("Root", vec![("magic", T::magic(b"\x89PNG\r\n\x1a\n"))])));
    let node = ev.node(&wrong, &[0]).unwrap();
    assert_eq!(listing::brief(&node.value), r#""\x89PNh\r\n\x1a\n""#);
    let problem = node.problem.clone().expect("a wrong signature is a problem");
    assert_eq!(problem.tier, crate::eval::Tier::Invalid);
    assert_eq!(problem.text, r#"Does not match: expected "\x89PNG\r\n\x1a\n""#);
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

/// A run that stops once it has read as many elements as a field ahead of it
/// said. The question is asked before each element, so the count is the index
/// of the element that is never read.
#[test]
fn a_while_run_stops_at_the_count_beside_it() {
    let counted = || {
        T::structure(
            "Root",
            vec![
                ("count", T::u8()),
                ("items", T::repeat(T::u8(), Until::While(E::Idx.less_than(E::field("count"))))),
                ("tail", T::u8()),
            ],
        )
    };
    let d = doc(&[3, 10, 20, 30, 99]);
    let mut ev = Evaluator::new(Template::new("t", counted()));
    assert_eq!(ev.node(&d, &[1]).unwrap().child_count, 3);
    assert_eq!(ev.node(&d, &[1]).unwrap().size_bits, 3 * 8);
    // The byte after the run belongs to the field after it, which is the whole
    // difference from `Until::Cond`: nothing read the element that ended it.
    assert_eq!(ev.node(&d, &[2]).unwrap().value, Value::UInt(99));

    // A count of nought is a run of nothing, which a question asked after the
    // element could not say.
    let d0 = doc(&[0, 77]);
    let mut ev0 = Evaluator::new(Template::new("t", counted()));
    assert_eq!(ev0.node(&d0, &[1]).unwrap().child_count, 0);
    assert_eq!(ev0.node(&d0, &[2]).unwrap().value, Value::UInt(77));
}

/// A run that stops when the next element would start past a limit the file
/// wrote down. The position in the question is where that element would
/// begin, not where the last one did.
#[test]
fn a_while_run_stops_before_the_element_that_would_pass_a_limit() {
    let t = T::structure(
        "Root",
        vec![
            ("limit", T::u8()),
            ("items", T::repeat(T::u16(Big), Until::While(E::SpacePos.less_than(E::field("limit"))))),
            ("rest", T::bytes(E::Remaining)),
        ],
    );
    // The run starts at 1 and the limit is 6, so elements start at 1, 3 and 5;
    // the one that would start at 7 is never read.
    let d = doc(&[6, 0, 1, 0, 2, 0, 3, 0, 4]);
    let mut ev = Evaluator::new(Template::new("t", t));
    assert_eq!(ev.node(&d, &[1]).unwrap().child_count, 3);
    assert_eq!(ev.node(&d, &[1]).unwrap().size_bits, 6 * 8);
    assert_eq!(ev.node(&d, &[2]).unwrap().size_bits, 2 * 8);

    // The question reads nothing of the element it is asked about, which is
    // what lets a run stop where reading one more would run off the end.
    let room =
        T::structure("Root", vec![("items", T::repeat(T::u16(Big), Until::While(E::Remaining.at_least(E::lit(2)))))]);
    let mut ev2 = Evaluator::new(Template::new("t", room));
    assert_eq!(ev2.node(&doc(&[0, 1, 0, 2, 9]), &[0]).unwrap().child_count, 2);
}

/// Inside a window, where a field is in the file and where it is in the window
/// are two different numbers, and so are the two sizes.
#[test]
fn space_position_and_size_ignore_the_window() {
    let inner = T::structure(
        "Inner",
        vec![
            ("a", T::u16(Big)),
            ("pos", T::computed(E::Pos)),
            ("space_pos", T::computed(E::SpacePos)),
            ("window", T::computed(E::WindowSize)),
            ("space", T::computed(E::SpaceSize)),
        ],
    );
    let t = T::structure("Root", vec![("head", T::bytes(E::lit(3))), ("body", T::sized(E::lit(4), inner))]);
    let d = doc(&[9, 9, 9, 0, 7, 0, 0, 1, 1]);
    let mut ev = Evaluator::new(Template::new("t", t));
    // Two bytes into the window, five bytes into the file.
    assert_eq!(ev.node(&d, &[1, 1]).unwrap().value, Value::Int(2));
    assert_eq!(ev.node(&d, &[1, 2]).unwrap().value, Value::Int(5));
    // The window is four bytes; the file is nine.
    assert_eq!(ev.node(&d, &[1, 3]).unwrap().value, Value::Int(4));
    assert_eq!(ev.node(&d, &[1, 4]).unwrap().value, Value::Int(9));
}

/// A peek at an address rather than at a distance, asked from inside a window
/// that holds neither address, one of them behind the field asking. Neither is
/// something a relative skip can say.
#[test]
fn a_peek_in_the_space_reads_at_an_address() {
    let peek = |at: i64, bits: u32| T::computed(E::PeekIn { at: Box::new(E::lit(at)), bits, endian: Big });
    let inner = T::structure("Inner", vec![("a", T::u8()), ("front", peek(0, 16)), ("back", peek(8 * 8, 8))]);
    let t = T::structure("Root", vec![("head", T::bytes(E::lit(4))), ("body", T::sized(E::lit(2), inner))]);
    let d = doc(&[0xab, 0xcd, 0, 0, 5, 0, 0, 0, 0x42]);
    let mut ev = Evaluator::new(Template::new("t", t));
    assert_eq!(ev.node(&d, &[1, 1]).unwrap().value, Value::Int(0xabcd));
    assert_eq!(ev.node(&d, &[1, 2]).unwrap().value, Value::Int(0x42));

    // Past the end of the file is a failure, not a number nobody wrote.
    let over = T::structure("Root", vec![("a", peek(80 * 8, 8))]);
    assert!(Evaluator::new(Template::new("t", over)).node(&d, &[0]).is_err());
}

/// A union: every field starts where the structure does, and the structure is
/// as long as the longest of them.
#[test]
fn a_union_lays_its_fields_over_one_another() {
    let u = T::union_structure(
        "Colour",
        vec![("packed", T::u32(Big)), ("bytes", T::array(T::u8(), E::lit(4))), ("high", T::u16(Big))],
    );
    let t = T::structure("Root", vec![("head", T::u8()), ("colour", u), ("tail", T::u8())]);
    let d = doc(&[9, 0x11, 0x22, 0x33, 0x44, 0x55]);
    let mut ev = Evaluator::new(Template::new("t", t));

    // Four bytes of the file, whichever of the three readings is asked.
    let whole = ev.node(&d, &[1]).unwrap();
    assert_eq!((whole.offset_bits, whole.size_bits), (8, 4 * 8));
    for (i, size) in [(0usize, 4u64), (1, 4), (2, 2)] {
        let f = ev.node(&d, &[1, i]).unwrap();
        assert_eq!((f.offset_bits, f.size_bits), (8, size * 8), "field {i}");
    }
    assert_eq!(ev.node(&d, &[1, 0]).unwrap().value, Value::UInt(0x11223344));
    assert_eq!(ev.node(&d, &[1, 2]).unwrap().value, Value::UInt(0x1122));
    // The field after the union starts past the longest of its readings.
    assert_eq!(ev.node(&d, &[2]).unwrap().offset_bits, 5 * 8);
    assert_eq!(ev.node(&d, &[2]).unwrap().value, Value::UInt(0x55));

    // Only the first reading is counted; the others are second readings of the
    // same bytes, the way `Field::aside` marks one by hand.
    assert!(!ev.aside(&[1, 0]));
    assert!(ev.aside(&[1, 1]) && ev.aside(&[1, 2]));

    // And the panel says so rather than calling the first one "first field".
    for i in 0..3 {
        assert_eq!(ev.shape(&d, &[1, i]).unwrap().placed, shape::Placed::Overlap, "field {i}");
    }

    // The listing draws one row per field, all at the same place: overlapping
    // spans, the way a second reading of bytes is already drawn.
    let rows = ev.spans(&d, 8, 5 * 8, 100).unwrap();
    assert_eq!(rows.iter().map(|s| (s.name.clone(), s.offset_bits)).collect::<Vec<_>>(), vec![("packed".into(), 8)]);
}

/// A union whose longest field is only as long as the file says, which is the
/// case no arithmetic over the declaration alone can settle.
#[test]
fn a_union_is_as_long_as_the_file_makes_its_longest_field() {
    let u = T::union_structure("Body", vec![("head", T::u8()), ("run", T::bytes(E::field("len")))]);
    let t = T::structure("Root", vec![("len", T::u8()), ("body", u), ("tail", T::u8())]);
    let d = doc(&[3, 1, 2, 3, 7]);
    let mut ev = Evaluator::new(Template::new("t", t));
    assert_eq!(ev.node(&d, &[1]).unwrap().size_bits, 3 * 8);
    assert_eq!(ev.node(&d, &[2]).unwrap().value, Value::UInt(7));
}
#[test]
fn text_expected_where_the_field_is_not_text_is_an_error_not_a_panic() {
    use crate::template::{Encoding, StrLen};
    // An enum whose underlying type is text: the value arm for text is reached
    // with a field whose type is the enum, so measuring the text finds none.
    let t = Template::new(
        "t",
        T::structure(
            "R",
            vec![("kind", T::enumeration("Kind", T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii), &[(0, "none")]))],
        ),
    );
    let d = doc(b"ABCD");
    let mut ev = Evaluator::new(t);
    let msg = failure(ev.node(&d, &[0]).unwrap_err());
    assert!(msg.contains("kind"), "{msg}");
}

#[test]
fn a_run_of_zero_size_elements_is_an_error_not_a_panic() {
    // A run to the end of elements that take no room at all would be counted
    // by dividing the room by nought.
    let t = Template::new("t", T::structure("R", vec![("items", T::repeat(T::bytes(E::lit(0)), Until::End))]));
    let d = doc(&[1, 2, 3, 4]);
    let mut ev = Evaluator::new(t);
    let msg = failure(ev.node(&d, &[0]).unwrap_err());
    assert!(msg.contains("items"), "{msg}");
}

#[test]
fn an_element_naming_its_own_list_is_refused_the_same_way_as_one_reading_itself() {
    // An element that asks for the list it sits in has asked for itself, and
    // both ways of asking take the same road out: the depth limit, not a walk
    // that never comes back. `Expr::Elem` has always found the list this way;
    // `Expr::Ref` used to climb past it and find nothing.
    let by_name = T::structure("Rec", vec![("data", T::bytes(E::field("items")))]);
    let by_index = T::structure("Rec", vec![("data", T::bytes(E::elem("items", E::lit(0))))]);
    for each in [by_name, by_index] {
        let t = Template::new("t", T::structure("R", vec![("items", T::array(each, E::lit(2)))]));
        let d = doc(&[1, 2, 3, 4]);
        let mut ev = Evaluator::new(t);
        let msg = failure(ev.node(&d, &[0, 0, 0]).unwrap_err());
        assert!(msg.contains("nested too deep"), "{msg}");
    }
}
