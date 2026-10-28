use super::*;

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
