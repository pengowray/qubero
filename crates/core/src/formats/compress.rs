//! A `.Z` file, which is what `compress` wrote before gzip existed: three
//! bytes of header and then LZW codes.
//!
//! The header is as small as a header gets. Two bytes say what the file is,
//! and one byte says the largest code width the encoder allowed and whether
//! it was willing to start the table over when it filled up. Everything after
//! that is codes, packed from nine bits up to that width, least significant
//! bit first, which is the one order the field types here cannot read: a code
//! is not a field, so as fields the codes are one run of bytes.
//!
//! What they say is another matter, and [`crate::codec::compress`] reads it: a
//! second address space over the whole file, and a step per code, so the run
//! that is grey in the hex view is a list of literals and repeats in the
//! listing.

use crate::codec::Codec;
use crate::template::{Endian::Big, Expr as E, Template, Ty as T};

/// What one of these starts with.
pub const MAGIC: &[u8] = b"\x1f\x9d";

pub fn compress() -> Template {
    Template::new(
        "compress",
        T::structure(
            "CompressStream",
            vec![
                ("magic", T::magic(MAGIC)),
                // Set when the encoder starts the code table over once it is
                // full, which every version since 1985 does.
                ("block_mode", T::UInt { bits: 1, endian: Big }),
                ("reserved", T::UInt { bits: 2, endian: Big }),
                // The widest code in the file, nine to sixteen bits.
                ("max_bits", T::UInt { bits: 5, endian: Big }),
                // LZW codes, packed least significant bit first, which is the
                // one order the field types here cannot read: a code is not a
                // field, so as fields these are one run of bytes. What they
                // hold is opened by the field below.
                ("compressed", T::bytes(E::Remaining)),
                // What the file comes to. The run is the whole file and not
                // the codes alone, because the codes cannot be read without
                // the byte in front of them: the width they start at and
                // whether the table is ever cleared are things the header
                // says. So the field costs no bytes where it stands, covers
                // the file from its first byte, and the fields above lie over
                // those same bytes. `xz` is laid out this way for the same
                // reason.
                (
                    "decoded",
                    T::at_in_window(E::lit(0), T::decoded(E::Remaining, Codec::Compress, super::decoded_text())),
                ),
            ],
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, eval::Evaluator, source::MemSource};

    #[test]
    fn the_third_byte_is_a_flag_and_a_width() {
        let d = Document::new(MemSource(vec![0x1f, 0x9d, 0x90, 0x61, 0xc4, 0x00]));
        let mut e = Evaluator::new(compress());
        assert_eq!(e.node(&d, &[1]).unwrap().value.as_int(), Some(1));
        assert_eq!(e.node(&d, &[3]).unwrap().value.as_int(), Some(16));
        assert_eq!(e.node(&d, &[4]).unwrap().size_bits, 3 * 8);
    }

    /// The codes open as what they say, in a space of their own, while the
    /// header goes on being three fields over the same three bytes.
    #[test]
    fn the_codes_open_into_the_text_they_hold() {
        // `aaaaa` at nine bits in block mode, checked against `gzip -dc`.
        let d = Document::new(MemSource(b"\x1f\x9d\x90\x61\x02\x06\x04".to_vec()));
        let mut e = Evaluator::new(compress());
        // The run itself is every byte of the file after the header.
        assert_eq!(e.node(&d, &[4]).unwrap().size_bits, 4 * 8);
        // And the stream it opens is the whole file, so the text under it is
        // in a space of its own and starts at the front of that space.
        let text = e.node(&d, &[5, 0, 0, 0]).unwrap();
        assert_eq!(text.value, crate::eval::Value::Str("aaaaa".into()));
        assert_ne!(text.space, 0, "text out of a stream, still in the file's space");
        assert_eq!(text.offset_bits, 0);
        assert!(!text.editable, "a decoded field offered for editing");
    }
}
