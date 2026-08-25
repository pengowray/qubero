//! Sun/NeXT audio: six big-endian numbers, an optional line of text, and the
//! samples. The oldest audio format still in use, and the one that puts its
//! numbers the way the machine that invented it did.
//!
//! A data size of 0xffffffff means "to the end of the file", which is what a
//! program piping audio out writes when it does not know how much there will
//! be. The samples run to the end here either way, so nothing has to special
//! case it.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T};

/// The encodings, which is really a list of every way a telephone company has
/// written a sample. 1 is mu-law, the one an `.au` almost always is.
const ENCODING: &[(i128, &str)] = &[
    (1, "8-bit mu-law"),
    (2, "8-bit pcm"),
    (3, "16-bit pcm"),
    (4, "24-bit pcm"),
    (5, "32-bit pcm"),
    (6, "32-bit float"),
    (7, "64-bit float"),
    (23, "4-bit adpcm g721"),
    (24, "8-bit adpcm g722"),
    (25, "3-bit adpcm g723"),
    (26, "5-bit adpcm g723"),
    (27, "8-bit a-law"),
];

pub fn au() -> Template {
    Template::new(
        "au",
        T::structure(
            "AU",
            vec![
                ("magic", T::magic(b".snd")),
                ("data_offset", T::u32(Big)),
                ("data_size", T::u32(Big)),
                ("encoding", T::enumeration("Encoding", T::u32(Big), ENCODING)),
                ("sample_rate", T::u32(Big)),
                ("channels", T::u32(Big)),
                // The header is at least 24 bytes and the rest of it up to
                // data_offset is a comment, NUL padded.
                ("annotation", T::text(StrLen::Padded { size: E::field("data_offset").sub(E::lit(24)), pad: 0 }, Encoding::Ascii)),
                ("samples", T::bytes(E::Remaining)),
            ],
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    #[test]
    fn the_annotation_fills_the_room_the_offset_leaves() {
        let mut v = b".snd".to_vec();
        v.extend_from_slice(&32u32.to_be_bytes()); // data starts at 32
        v.extend_from_slice(&u32::MAX.to_be_bytes()); // size unknown
        v.extend_from_slice(&1u32.to_be_bytes()); // mu-law
        v.extend_from_slice(&8000u32.to_be_bytes());
        v.extend_from_slice(&1u32.to_be_bytes());
        v.extend_from_slice(b"hi\0\0\0\0\0\0");
        v.extend_from_slice(&[0xff; 16]);

        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(au());
        assert_eq!(ev.node(&d, &[4]).unwrap().value, Value::UInt(8000));
        assert_eq!(ev.node(&d, &[6]).unwrap().value, Value::Str("hi".into()));
        assert_eq!(ev.node(&d, &[6]).unwrap().size_bits, 8 * 8);
        assert_eq!(ev.node(&d, &[7]).unwrap().size_bits, 16 * 8);
    }
}
