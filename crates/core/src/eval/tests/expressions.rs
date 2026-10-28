use super::*;

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

/// The three bitwise operators, over bytes a reader can check by hand. `Or`
/// beside them is still the value-or, which is the whole reason the new ones
/// have names of their own.
#[test]
fn the_bitwise_operators_work_on_the_bits() {
    let t = T::structure(
        "Root",
        vec![
            ("a", T::u8()),
            ("b", T::u8()),
            ("ored", T::computed(E::field("a").bit_or(E::field("b")))),
            ("xored", T::computed(E::field("a").bit_xor(E::field("b")))),
            ("flipped", T::computed(E::field("a").bit_not())),
            ("masked", T::computed(E::field("a").bit_not().and(E::lit(0xff)))),
            // The value-or, unchanged: the left side, because it is not zero.
            ("valued", T::computed(E::field("a").or(E::field("b")))),
        ],
    );
    let d = doc(&[0b1100, 0b1010]);
    let mut ev = Evaluator::new(Template::new("t", t));
    assert_eq!(ev.node(&d, &[2]).unwrap().value, Value::Int(0b1110));
    assert_eq!(ev.node(&d, &[3]).unwrap().value, Value::Int(0b0110));
    // Over the whole 128-bit number, so it is negative, and a mask brings it
    // back inside a byte. See `Expr::BitNot`.
    assert_eq!(ev.node(&d, &[4]).unwrap().value, Value::Int(-13));
    assert_eq!(ev.node(&d, &[5]).unwrap().value, Value::Int(0xf3));
    assert_eq!(ev.node(&d, &[6]).unwrap().value, Value::Int(0b1100));
}

/// The relations panel writes the new operators and the new measurements the
/// way it writes the rest: as the template writes it, then with each field's
/// value in its place.
#[test]
fn the_new_expressions_are_written_out_for_the_panel() {
    let t = T::structure(
        "Root",
        vec![
            ("flags", T::u8()),
            ("set", T::computed(E::field("flags").bit_or(E::lit(0x20)))),
            ("clear", T::computed(E::field("flags").bit_not().and(E::lit(0xff)))),
            ("run", T::bytes(E::SpaceSize.sub(E::field("flags")))),
        ],
    );
    // A file of four bytes, so the run is three: everything after the flags.
    let d = doc(&[1, 0, 0, 0]);
    let mut ev = Evaluator::new(Template::new("t", t));
    let one = |ev: &mut Evaluator, i: usize| {
        let r = ev.relations(&d, &[i]).unwrap();
        assert_eq!(r.len(), 1, "{r:?}");
        (r[0].written.clone(), r[0].substituted.clone(), r[0].result.clone())
    };
    assert_eq!(one(&mut ev, 1), ("flags | 0x20".into(), "1 | 0x20".into(), "33".into()));
    assert_eq!(one(&mut ev, 2), ("~flags & 0xff".into(), "~1 & 0xff".into(), "254".into()));
    // The size of the space is read in like any other leaf, so the row shows
    // the number it came to rather than the words for it.
    assert_eq!(one(&mut ev, 3), ("size of space - flags".into(), "4 - 1".into(), "3".into()));
}
