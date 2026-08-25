//! Netpbm: PBM, PGM and PPM, the formats that exist so a picture can be typed
//! out by hand.
//!
//! A magic of P and a digit says which of six the file is: 1, 2 and 3 write
//! the pixels as decimal numbers with spaces between them, and 4, 5 and 6
//! write the same pixels as raw bytes. Between the magic and the pixels are a
//! width, a height, and for the grey and colour ones a maximum value, each a
//! run of digits, separated by any whitespace in any amount.
//!
//! That is what a scanned field is for: step over the separators, read to the
//! next one, and the field lands on the number wherever the writer put it.
//! One line, three lines, tabs, two spaces between them: all read the same,
//! and the pixels start exactly where the last separator ends.
//!
//! What is still not read is a comment. A `#` and everything to the end of
//! that line may appear anywhere among the numbers, and stepping over a run
//! that ends at a byte is not the same as stepping over one that starts at
//! one. A file with a comment in its header reads its numbers wrong from that
//! point on. GIMP and other editors write one, so this is worth saying: it is
//! the last gap this format leaves open.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T};

/// The six variants, told apart by the digit after the P.
const KIND: &[(i128, &str)] = &[
    (0x5031, "pbm, bitmap, text"),
    (0x5032, "pgm, greyscale, text"),
    (0x5033, "ppm, colour, text"),
    (0x5034, "pbm, bitmap, binary"),
    (0x5035, "pgm, greyscale, binary"),
    (0x5036, "ppm, colour, binary"),
];

/// Space, tab, carriage return, newline: what the format calls whitespace.
const SPACE: &[u8] = b" \t\r\n";

pub fn pnm() -> Template {
    Template::new(
        "pnm",
        T::structure(
            "Netpbm",
            vec![
                ("magic", T::enumeration("Kind", T::u16(Big), KIND)),
                ("width", number()),
                ("height", number()),
                // A bitmap is one bit a pixel, so there is nothing to be the
                // maximum of and the field is not written.
                (
                    "max_value",
                    T::switch(E::field("magic"), vec![(0x5031, nothing()), (0x5034, nothing())], number()),
                ),
                ("pixels", T::bytes(E::Remaining)),
            ],
        ),
    )
}

/// One number of the header: the whitespace before it, the digits, and the one
/// whitespace byte that ends it.
fn number() -> T {
    T::text(StrLen::Scan { skip: SPACE.to_vec(), ends: SPACE.to_vec() }, Encoding::Ascii)
}

fn nothing() -> T {
    T::bytes(E::lit(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    #[test]
    fn a_binary_ppm_reads_its_numbers_and_leaves_the_pixels_whole() {
        let mut v = b"P6\n2 2\n255\n".to_vec();
        v.extend_from_slice(&[0xff; 12]);
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(pnm());
        assert_eq!(
            ev.node(&d, &[0]).unwrap().value,
            Value::Enum { raw: 0x5036, name: Some("ppm, colour, binary".into()), hex: false }
        );
        assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::Str("2".into()));
        assert_eq!(ev.node(&d, &[2]).unwrap().value, Value::Str("2".into()));
        assert_eq!(ev.node(&d, &[3]).unwrap().value, Value::Str("255".into()));
        assert_eq!(ev.node(&d, &[4]).unwrap().offset_bits, 11 * 8);
        assert_eq!(ev.node(&d, &[4]).unwrap().size_bits, 12 * 8);
        // The width field covers the newline before it as well as the space
        // after it; only the digits are the value.
        let width = ev.node(&d, &[1]).unwrap();
        assert_eq!(width.offset_bits, 2 * 8);
        assert_eq!(width.size_bits, 3 * 8);
        assert_eq!(width.value_offset_bits, 3 * 8);
    }

    #[test]
    fn the_same_header_on_one_line_reads_the_same() {
        let mut v = b"P6 2\t2  255 ".to_vec();
        v.extend_from_slice(&[0xff; 12]);
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(pnm());
        assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::Str("2".into()));
        assert_eq!(ev.node(&d, &[2]).unwrap().value, Value::Str("2".into()));
        assert_eq!(ev.node(&d, &[3]).unwrap().value, Value::Str("255".into()));
        assert_eq!(ev.node(&d, &[4]).unwrap().size_bits, 12 * 8);
    }

    #[test]
    fn a_bitmap_has_no_maximum_value() {
        let d = Document::new(MemSource(b"P4\n8 1\n\xff".to_vec()));
        let mut ev = Evaluator::new(pnm());
        assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::Str("8".into()));
        assert_eq!(ev.node(&d, &[2]).unwrap().value, Value::Str("1".into()));
        assert_eq!(ev.node(&d, &[3]).unwrap().size_bits, 0);
        assert_eq!(ev.node(&d, &[4]).unwrap().size_bits, 8);
    }
}
