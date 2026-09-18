//! Melco expanded embroidery (.exp): the needle's moves, two bytes each, and
//! nothing else.
//!
//! There is no header, no footer, no stitch count and no colour table. The
//! file is the stream a Melco machine reads: a signed byte for how far the
//! hoop moves across and a signed byte for how far it moves up, in tenths of a
//! millimetre, and the needle comes down after each. So a move is at most
//! 12.7 mm, and a longer one is written as several.
//!
//! The one byte a move across cannot be is `0x80`, which is -128, and that is
//! the escape: `80`, a control byte, and then the same two bytes of movement.
//! `80 01` stops the machine for a thread change, `80 04` moves without
//! stitching, `80 80` trims the thread. The colours themselves are not in the
//! file; whoever runs the machine has them on paper, or in the `.inf` some
//! programs write beside it.
//!
//! Because nothing announces the format, nothing recognises it but its name:
//! a design that opens with a control is `80 01`, `80 02` or `80 04`, which is
//! also how a Python pickle of protocol 1, 2 or 4 opens, and `80 05` is what
//! file(1) calls a XENIX object. See `is_melco_exp` in `recognise.rs` for what
//! the bytes have to look like before the extension is believed.

use crate::template::{Endian::*, Expr as E, Template, Ty as T, Until};

/// The byte that cannot be a move across, and so says a control follows.
pub const ESCAPE: u8 = 0x80;

/// The controls a reader of these files knows. `02` is a stitch written the
/// long way, which no writer has a reason to produce and some files hold.
pub const CONTROL: &[(i128, &str)] = &[(0x01, "colour change"), (0x02, "stitch"), (0x04, "jump"), (0x80, "trim")];

/// Whether a byte after the escape is a control this template names.
pub fn is_control(b: u8) -> bool {
    CONTROL.iter().any(|(code, _)| *code == i128::from(b))
}

pub fn exp() -> Template {
    Template::new("exp", T::structure("EXP", vec![("steps", T::repeat(step(), Until::End))]))
}

/// One step of the design, which the first byte decides the length of.
fn step() -> T {
    T::switch(E::peek(8, Little), vec![(i128::from(ESCAPE), command())], stitch())
}

fn i8() -> T {
    T::Int { bits: 8, endian: Little }
}

/// A move and a stitch at the end of it.
fn stitch() -> T {
    T::inline_structure("Stitch", vec![("dx", i8()), ("dy", i8())])
        .field_doc("dx", "How far the hoop moves across before the needle comes down, in units of 0.1 mm. Positive is right.")
        .field_doc("dy", "How far the hoop moves up before the needle comes down, in units of 0.1 mm. Positive is up.")
        .counted_as("step")
}

/// The escape, what to do, and the move that goes with it.
fn command() -> T {
    T::structure_named(
        "Command",
        "control",
        "",
        vec![("escape", T::magic(&[ESCAPE])), ("control", T::enumeration_hex("Control", T::u8(), CONTROL)), ("dx", i8()), ("dy", i8())],
    )
    .field_doc("escape", "0x80, which is -128 and so not a distance a stitch can move: it says the next byte is a control.")
    .field_doc("dx", "How far the hoop moves across, in units of 0.1 mm. Positive is right.")
    .field_doc("dy", "How far the hoop moves up, in units of 0.1 mm. Positive is up.")
    .reads_as(&[("control", "", ""), ("dx", "", ""), ("dy", "", "")])
    .counted_as("step")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    #[test]
    fn a_stitch_is_two_bytes_and_a_control_is_four() {
        // A jump to the start, two stitches, a thread change, a stitch, a trim.
        let bytes = vec![0x80, 0x04, 0x10, 0xf0, 0x05, 0xfb, 0x05, 0x05, 0x80, 0x01, 0x00, 0x00, 0x7f, 0x81, 0x80, 0x80, 0x07, 0x00];
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(exp());
        assert_eq!(ev.node(&d, &[0]).unwrap().child_count, 6);
        let sizes: Vec<u64> = (0..6).map(|i| ev.node(&d, &[0, i]).unwrap().size_bits / 8).collect();
        assert_eq!(sizes, [4, 2, 2, 4, 2, 4]);
        // The jump is named for what it does, and moves 16 across and 16 down.
        assert!(ev.node(&d, &[0, 0]).unwrap().name.ends_with("jump"));
        assert_eq!(ev.node(&d, &[0, 0, 2]).unwrap().value, Value::Int(16));
        assert_eq!(ev.node(&d, &[0, 0, 3]).unwrap().value, Value::Int(-16));
        // The longest moves a stitch can make, either way.
        assert_eq!(ev.node(&d, &[0, 4, 0]).unwrap().value, Value::Int(127));
        assert_eq!(ev.node(&d, &[0, 4, 1]).unwrap().value, Value::Int(-127));
    }
}
