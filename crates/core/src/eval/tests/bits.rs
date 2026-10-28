use super::*;

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

    // The offsets count from the bottom of the byte, in the order the fields
    // were declared, and say so. The cursor counts from the top: bit 7 is the
    // bottom bit, where `final` is, and bit 1 is the second from the top,
    // inside `window`.
    assert_eq!(ev.node(&d, &[0]).unwrap().offset_bits, 0);
    assert_eq!(ev.node(&d, &[3]).unwrap().offset_bits, 4);
    assert!(ev.node(&d, &[3]).unwrap().lsb_first);
    assert!(!ev.node(&d, &[5]).unwrap().lsb_first);
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

/// Three fields of 3, 10 and 3 bits packed from the bottom of two bytes: 5,
/// 677 and 6, which is `0xd52d` read as a little-endian number.
fn three_ten_three() -> Template {
    Template::new(
        "t",
        T::structure(
            "R",
            vec![
                ("a", T::UInt { bits: 3, endian: Little }),
                ("peeked", T::computed(E::peek(10, Little))),
                ("b", T::UInt { bits: 10, endian: Little }),
                ("c", T::UInt { bits: 3, endian: Little }),
            ],
        ),
    )
}

#[test]
fn a_low_bit_first_field_across_a_byte_boundary_reads_as_one_number() {
    let d = doc(&[0x2d, 0xd5]);
    let mut ev = Evaluator::new(three_ten_three());
    let values: Vec<Value> = [0, 2, 3].iter().map(|&i| ev.node(&d, &[i]).unwrap().value).collect();
    assert_eq!(values, [Value::UInt(5), Value::UInt(677), Value::UInt(6)]);
    // A peek of the same ten bits from where `b` starts says the same.
    assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::Int(677));

    // Each field is one run counted from the bottom of each byte: `b` is the
    // top five bits of the first byte and the bottom five of the second.
    let at = |ev: &mut Evaluator, i: usize| {
        let n = ev.node(&d, &[i]).unwrap();
        (n.offset_bits, n.size_bits, n.lsb_first)
    };
    assert_eq!(at(&mut ev, 0), (0, 3, true));
    assert_eq!(at(&mut ev, 2), (3, 10, true));
    assert_eq!(at(&mut ev, 3), (13, 3, true));

    // The cursor counts each byte from the top. In the first byte the bottom
    // bit is `a`'s and the top bit is `b`'s; in the second the bottom five are
    // `b`'s and the top three `c`'s.
    for (bit, field) in [(7, 0), (5, 0), (4, 2), (0, 2), (15, 2), (11, 2), (10, 3), (8, 3)] {
        assert_eq!(ev.locate(&d, bit).unwrap(), vec![field], "bit {bit}");
    }
}

#[test]
fn a_low_bit_first_field_across_a_byte_boundary_is_read_but_not_written() {
    let mut d = doc(&[0x2d, 0xd5]);
    let mut ev = Evaluator::new(three_ten_three());
    assert!(!ev.node(&d, &[2]).unwrap().editable);
    let Err(EvalError::Failed(msg)) = ev.prepare_write(&d, &[2], "1") else {
        panic!("a write across a byte boundary should be refused");
    };
    assert_eq!(msg, crate::encode::NOT_EDITABLE_MSG);
    // The ones inside a byte still write where they are read.
    assert!(ev.node(&d, &[0]).unwrap().editable);
    let w = ev.prepare_write(&d, &[3], "1").unwrap();
    d.overwrite_bits(w.offset_bits, &w.data, w.n_bits);
    ev.invalidate();
    assert_eq!(ev.node(&d, &[3]).unwrap().value, Value::UInt(1));
    assert_eq!(ev.node(&d, &[2]).unwrap().value, Value::UInt(677));
    let mut out = [0u8; 2];
    d.read_bytes(0, &mut out);
    assert_eq!(out, [0x2d, 0x35]);
}

#[test]
fn floats_and_widths_from_a_field_are_read_low_bit_first_off_a_byte_boundary() {
    // A one-bit flag and then an f16 of 1.0, which is 0x3c00, so the float
    // starts a bit into the first byte and ends a bit into the third.
    let t = Template::new("t", T::structure("R", vec![("flag", T::UInt { bits: 1, endian: Little }), ("x", T::F16(Little))]));
    let d = doc(&[0x01, 0x78, 0x00]);
    let mut ev = Evaluator::new(t);
    assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::Float(1.0));
    assert_eq!((ev.node(&d, &[1]).unwrap().offset_bits, ev.node(&d, &[1]).unwrap().size_bits), (1, 16));

    // A width an earlier field gives: four bits saying nine, then nine bits.
    let t = Template::new(
        "t",
        T::structure("R", vec![("n", T::UInt { bits: 4, endian: Little }), ("v", T::uint_expr(E::field("n"), Little))]),
    );
    let d = doc(&[0x39, 0x1a]);
    let mut ev = Evaluator::new(t);
    assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::UInt(0x1a3));
    assert!(ev.node(&d, &[1]).unwrap().lsb_first);
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
fn a_width_read_from_the_file_is_placed_before_it_is_known() {
    // Packed from the bottom of the byte, at a width nothing knows until the
    // field before it has been read. It is placed at the count all the same,
    // and the five bits it leaves in the second byte are the top five.
    let t = Template::new(
        "t",
        T::structure("R", vec![("w", T::u8()), ("v", T::uint_expr(E::field("w"), Little))]),
    );
    let d = doc(&[11, 0xff, 0xff]);
    let mut ev = Evaluator::new(t);
    let v = ev.node(&d, &[1]).unwrap();
    assert_eq!((v.value, v.offset_bits, v.size_bits, v.lsb_first), (Value::UInt(0x7ff), 8, 11, true));
}
