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
//! Comments are stepped over the same way. A `#` and everything to the end of
//! that line counts as separator, so the `# CREATOR:` line GIMP writes costs
//! the reader nothing. What is not read is a comment glued to a number with no
//! whitespace in front of it, which the format allows and nothing writes: that
//! `#` and the rest of the line read as part of the number before it.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};

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
                ("pixels", pixels()),
            ],
        ),
    )
}

/// One number of the header: the whitespace and comments before it, the
/// digits, and the one whitespace byte that ends it.
fn number() -> T {
    T::text(StrLen::token_past_comments(SPACE, SPACE, (b'#', b'\n')), Encoding::Ascii)
}

/// The picture. In the three binary variants it is packed bytes and there is
/// nothing to read in them without knowing the width; in the three text ones
/// it is the same numbers the header is written in, separated the same way, so
/// each one is a field.
///
/// A colour file writes three numbers per pixel, red then green then blue, so
/// they are read as a pixel rather than as a run of numbers three times too
/// long. A greyscale or bitmap file writes one.
fn pixels() -> T {
    let value = || T::decimal(StrLen::token_past_comments(SPACE, SPACE, (b'#', b'\n')));
    let rgb = T::inline_structure("Pixel", vec![("red", value()), ("green", value()), ("blue", value())]);
    T::switch(
        E::field("magic"),
        vec![
            (0x5031, T::repeat(value().counted_as("pixel"), Until::End)),
            (0x5032, T::repeat(value().counted_as("pixel"), Until::End)),
            (0x5033, T::repeat(rgb.counted_as("pixel"), Until::End)),
        ],
        T::bytes(E::Remaining),
    )
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
    fn a_comment_among_the_numbers_is_stepped_over_with_the_whitespace() {
        // The header GIMP writes, comment line and all. The comment runs from
        // byte 3 to byte 41, so the width starts at 42.
        let mut v = b"P6\n# CREATOR: GIMP PNM Filter Version 1.1\n2 2\n255\n".to_vec();
        v.extend_from_slice(&[0xff; 12]);
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(pnm());
        assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::Str("2".into()));
        assert_eq!(ev.node(&d, &[2]).unwrap().value, Value::Str("2".into()));
        assert_eq!(ev.node(&d, &[3]).unwrap().value, Value::Str("255".into()));
        assert_eq!(ev.node(&d, &[4]).unwrap().size_bits, 12 * 8);
        // The comment belongs to the field that stepped over it, and the value
        // starts after it rather than at the field.
        let width = ev.node(&d, &[1]).unwrap();
        assert_eq!(width.offset_bits, 2 * 8);
        assert_eq!(width.value_offset_bits, 42 * 8);
    }

    #[test]
    fn a_comment_between_two_numbers_is_stepped_over_too() {
        let mut v = b"P5\n8 # width above, height below\n1\n255\n".to_vec();
        v.push(0xff);
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(pnm());
        assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::Str("8".into()));
        assert_eq!(ev.node(&d, &[2]).unwrap().value, Value::Str("1".into()));
        assert_eq!(ev.node(&d, &[3]).unwrap().value, Value::Str("255".into()));
        assert_eq!(ev.node(&d, &[4]).unwrap().size_bits, 8);
    }

    /// The colour text variant, which is what a `.ppma` holds: three numbers
    /// a pixel, laid out however the writer felt like laying them out.
    #[test]
    fn a_text_ppm_reads_its_pixels_as_numbers() {
        let d = Document::new(MemSource(b"P3\n2 1\n255\n255 0 0\n  0 128\n64\n".to_vec()));
        let mut ev = Evaluator::new(pnm());
        let pixels = ev.node(&d, &[4]).unwrap();
        assert_eq!(pixels.child_count, 2);
        assert_eq!(pixels.unit.as_deref(), Some("pixel"));
        assert_eq!(ev.node(&d, &[4, 0, 0]).unwrap().value, Value::Int(255));
        assert_eq!(ev.node(&d, &[4, 0, 1]).unwrap().value, Value::Int(0));
        assert_eq!(ev.node(&d, &[4, 1, 1]).unwrap().value, Value::Int(128));
        assert_eq!(ev.node(&d, &[4, 1, 2]).unwrap().value, Value::Int(64));
    }

    /// The greyscale text variant writes one number a pixel, so a pixel is a
    /// number rather than a group of three.
    #[test]
    fn a_text_pgm_reads_one_number_per_pixel() {
        let d = Document::new(MemSource(b"P2\n3 1\n255\n0 128 255\n".to_vec()));
        let mut ev = Evaluator::new(pnm());
        assert_eq!(ev.node(&d, &[4]).unwrap().child_count, 3);
        assert_eq!(ev.node(&d, &[4, 1]).unwrap().value, Value::Int(128));
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
