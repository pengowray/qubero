//! Netpbm: PBM, PGM and PPM, the formats that exist so a picture can be typed
//! out by hand.
//!
//! A magic of P and a digit says which of six the file is: 1, 2 and 3 write
//! the pixels as decimal numbers with spaces between them, and 4, 5 and 6
//! write the same pixels as raw bytes. Between the magic and the pixels are a
//! width, a height, and for the grey and colour ones a maximum value, each a
//! run of digits, separated by any whitespace, with comment lines allowed
//! among them.
//!
//! Any whitespace is what this IR cannot say. A field can run to one named
//! byte, not to whichever of space, tab and newline comes first, and it cannot
//! be told to skip a comment line and keep looking. So the header is read as
//! the lines the writers of these files actually produce: one number, or one
//! pair of numbers, per line. A file that puts its whole header on one line is
//! still laid out honestly, since the first line then holds all of it, but the
//! header lines after it will land on the pixels. Reading it properly needs a
//! length a scan decides, and that is a gap in the IR rather than in the
//! format.

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

pub fn pnm() -> Template {
    Template::new(
        "pnm",
        T::structure(
            "Netpbm",
            vec![
                ("magic", T::enumeration("Kind", T::u16(Big), KIND)),
                // A bitmap has no maximum value, so its header is one line
                // shorter than the other four.
                (
                    "header",
                    T::switch(
                        E::field("magic"),
                        vec![(0x5031, lines(E::lit(2))), (0x5034, lines(E::lit(2)))],
                        lines(E::lit(3)),
                    ),
                ),
                ("pixels", T::bytes(E::Remaining)),
            ],
        ),
    )
}

/// The header lines: the newline right after the magic counts as the end of
/// the first of them, which is why a canonical file has one more line here
/// than it has numbers.
fn lines(count: E) -> T {
    T::array(T::text(StrLen::Terminated { end: b'\n', or_end: true }, Encoding::Ascii), count).counted_as("line")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    #[test]
    fn a_binary_ppm_reads_its_kind_and_leaves_the_pixels_whole() {
        let mut v = b"P6\n2 2\n255\n".to_vec();
        v.extend_from_slice(&[0xff; 12]);
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(pnm());
        assert_eq!(
            ev.node(&d, &[0]).unwrap().value,
            Value::Enum { raw: 0x5036, name: Some("ppm, colour, binary".into()), hex: false }
        );
        assert_eq!(ev.node(&d, &[1, 1]).unwrap().value, Value::Str("2 2".into()));
        assert_eq!(ev.node(&d, &[1, 2]).unwrap().value, Value::Str("255".into()));
        assert_eq!(ev.node(&d, &[2]).unwrap().offset_bits, 11 * 8);
        assert_eq!(ev.node(&d, &[2]).unwrap().size_bits, 12 * 8);
    }

    #[test]
    fn a_bitmap_has_one_header_line_fewer() {
        let d = Document::new(MemSource(b"P4\n8 1\n\xff".to_vec()));
        let mut ev = Evaluator::new(pnm());
        assert_eq!(ev.node(&d, &[1]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[1, 1]).unwrap().value, Value::Str("8 1".into()));
        assert_eq!(ev.node(&d, &[2]).unwrap().size_bits, 8);
    }
}
