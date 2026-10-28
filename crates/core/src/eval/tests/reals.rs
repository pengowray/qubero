use super::*;

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
    assert_eq!(within(E::pow2(E::lit(4))), "field extends beyond its parent");
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
