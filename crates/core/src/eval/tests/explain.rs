use super::*;

/// The prose a format carries about a field reaches the node, and where a
/// field has none, the structure it is speaks for it.
#[test]
fn a_node_carries_what_the_format_says_about_it() {
    let chunk = || {
        T::structure("Chunk", vec![("len", T::u8()), ("body", T::bytes(E::field("len")))])
            .doc("A length and the bytes it counts.")
            .field_doc("len", "How many bytes of body follow.")
    };
    let t = Template::new("t", T::structure("Root", vec![("first", chunk()), ("second", chunk())]));
    let d = doc(&[2, 7, 8, 1, 9]);
    let mut ev = Evaluator::new(t);

    // The declaration's own prose, on the field that has it.
    assert_eq!(ev.node(&d, &[0, 0]).unwrap().doc.as_deref(), Some("How many bytes of body follow."));
    // A field with none of its own: nothing, since `body` is a run of bytes
    // and no structure speaks for it.
    assert_eq!(ev.node(&d, &[0, 1]).unwrap().doc, None);
    // The structure's own prose, on a field declared as that structure.
    assert_eq!(ev.node(&d, &[1]).unwrap().doc.as_deref(), Some("A length and the bytes it counts."));
    // A structure nobody wrote prose for says nothing.
    assert_eq!(ev.node(&d, &[]).unwrap().doc, None);
}

/// A padding field sized to a boundary says so without anyone writing it
/// down, and one of no bytes says why there are none.
#[test]
fn alignment_padding_explains_itself() {
    let rec = T::structure(
        "Rec",
        vec![("len", T::u8()), ("text", T::bytes(E::field("len"))), ("padding", T::bytes(E::size_of("text").pad_to(4)))],
    );
    let t = Template::new("t", T::array(rec, E::lit(2)));
    let mut ev = Evaluator::new(t);
    // Three bytes of text: one byte of padding. Four bytes: none.
    let d = doc(&[3, b'a', b'b', b'c', 0, 4, b'a', b'b', b'c', b'd']);
    assert_eq!(ev.node(&d, &[0, 2]).unwrap().doc.as_deref(), Some("Alignment padding: pads text to a 4-byte boundary."));
    assert_eq!(
        ev.node(&d, &[1, 2]).unwrap().doc.as_deref(),
        Some("Alignment padding: pads text to a 4-byte boundary. 0 bytes here because text already ends on one.")
    );
    // A field with prose of its own keeps it.
    let said = T::structure("Rec", vec![("len", T::u8()), ("padding", T::bytes(E::field("len").pad_to(2)))]).field_doc("padding", "Reserved.");
    let mut ev = Evaluator::new(Template::new("t", said));
    assert_eq!(ev.node(&doc(&[1, 0]), &[1]).unwrap().doc.as_deref(), Some("Reserved."));
}

/// What an enum value means is a fact about the value, kept beside it in the
/// definition rather than folded into the field's own prose.
#[test]
fn an_enum_value_keeps_its_own_prose() {
    let ty = T::enumeration("Method", T::u8(), &[(0, "stored"), (8, "deflate")])
        .enum_doc(0, "The bytes as they are, with no compression at all.");
    let Ty::Enum { def, .. } = &ty else { panic!("not an enum") };
    assert_eq!(def.doc_of(0), Some("The bytes as they are, with no compression at all."));
    assert_eq!(def.doc_of(8), None);
    // It is the value's, not the field's: the node goes on saying nothing.
    let t = Template::new("t", T::structure("Root", vec![("method", ty.clone())]));
    let d = doc(&[0]);
    assert_eq!(Evaluator::new(t).node(&d, &[0]).unwrap().doc, None);
}
