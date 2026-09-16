//! An offset beside the thing it places, which is the commonest pointer any
//! format writes: a TIFF entry's four bytes and the text they lead to, a
//! header's address and the table at it.
//!
//! Both halves are read here from a structure of two fields, because the fix
//! belongs to every format that writes one and not to the template that
//! showed it up. What the reader is owed is the same either way: the line over
//! the bytes says the value came from somewhere else and where, and the panel
//! about the offset says what is at the other end.

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Role};
use qubero_core::source::MemSource;
use qubero_core::template::{Encoding, Endian::Big, Expr as E, StrLen, Template, Ty as T};

/// Four bytes holding the address of a string, and the string, at `0x20`.
fn file() -> (Document<MemSource>, Evaluator) {
    let mut bytes = vec![0u8; 0x2c];
    bytes[..4].copy_from_slice(&0x20u32.to_be_bytes());
    bytes[0x20..0x2a].copy_from_slice(b"2003:07:19");
    let template = Template::new(
        "pointed",
        T::structure(
            "Root",
            vec![(
                "value",
                T::inline_structure(
                    "Elsewhere",
                    vec![
                        ("offset", T::u32(Big)),
                        ("values", T::at(E::field("offset"), T::text(StrLen::Fixed(E::lit(10)), Encoding::Ascii))),
                    ],
                ),
            )],
        ),
    );
    (Document::new(MemSource(bytes)), Evaluator::new(template))
}

/// The line the annotation column puts over the four bytes. Without the
/// address it reads as the text being those four bytes.
#[test]
fn a_reading_of_a_pointer_says_where_the_value_was_read() {
    let (doc, mut ev) = file();
    let node = ev.node(&doc, &[0]).unwrap();
    assert_eq!(node.line.as_deref(), Some("@0x20 · 2003:07:19"));
    // The same reading reaches the hex view as the span's own line, so the
    // chip beside the bytes and the panel about them cannot disagree.
    let spans = ev.spans(&doc, 0, 4 * 8, 16).unwrap();
    let span = spans.iter().find(|s| s.name == "value").expect("a span for the pointer");
    assert_eq!(span.line.as_deref(), Some("@0x20 · 2003:07:19"));
}

/// The pointer field covers four bytes and the value it holds is eleven, at
/// the other end. A panel that showed the field's own extent beside that
/// value would say the eleven bytes are not there.
#[test]
fn a_pointer_holds_the_bytes_it_points_at() {
    let (doc, mut ev) = file();
    let at = ev.node(&doc, &[0, 1]).unwrap();
    assert_eq!(at.size_bits, 0);
    assert_eq!(at.value_offset_bits, 0x20 * 8);
    assert_eq!(at.value_bytes, 10);
}

/// The panel about the offset itself: which field reads it, where that put
/// what it read, and what is there.
#[test]
fn an_offset_points_at_what_its_sibling_read() {
    let (doc, mut ev) = file();
    let origins = ev.origins(&doc, &[0, 0]).unwrap();
    let points = origins.iter().find(|o| o.role == Role::Points).expect("the offset points somewhere");
    assert_eq!(points.label, "values");
    assert_eq!(points.target_bits, Some(0x20 * 8));
    assert_eq!(points.value, "2003:07:19");
}

/// A pointer the file chose between several shapes still points where it
/// landed: an ELF section's name is a switch over whether the file has a name
/// table at all, and the case it took is read at an address.
#[test]
fn a_pointer_a_switch_picked_points_where_it_landed() {
    let mut bytes = vec![0u8; 0x2c];
    bytes[..4].copy_from_slice(&0x20u32.to_be_bytes());
    bytes[0x20..0x2a].copy_from_slice(b"2003:07:19");
    let template = Template::new(
        "picked",
        T::structure(
            "Root",
            vec![(
                "value",
                T::inline_structure(
                    "Elsewhere",
                    vec![
                        ("offset", T::u32(Big)),
                        (
                            "values",
                            T::switch(
                                E::field("offset"),
                                vec![(0, T::bytes(E::lit(0)))],
                                T::at(E::field("offset"), T::text(StrLen::Fixed(E::lit(10)), Encoding::Ascii)),
                            ),
                        ),
                    ],
                ),
            )],
        ),
    );
    let doc = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(template);
    let origins = ev.origins(&doc, &[0, 0]).unwrap();
    let points = origins.iter().find(|o| o.role == Role::Points).expect("the offset points somewhere");
    assert_eq!((points.label.as_str(), points.target_bits, points.value.as_str()), ("values", Some(0x20 * 8), "2003:07:19"));
}

/// A field no pointer reads points nowhere, which is nearly every field.
#[test]
fn a_plain_field_points_nowhere() {
    let doc = Document::new(MemSource(vec![0u8; 8]));
    let template = Template::new("plain", T::structure("Root", vec![("a", T::u32(Big)), ("b", T::u32(Big))]));
    let mut ev = Evaluator::new(template);
    let origins = ev.origins(&doc, &[0]).unwrap();
    assert!(origins.iter().all(|o| o.role != Role::Points), "{origins:?}");
}
