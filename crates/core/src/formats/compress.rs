//! A `.Z` file, which is what `compress` wrote before gzip existed: three
//! bytes of header and then LZW codes.
//!
//! The header is as small as a header gets. Two bytes say what the file is,
//! and one byte says the largest code width the encoder allowed and whether
//! it was willing to start the table over when it filled up. Everything after
//! that is codes, packed from nine bits up to that width, least significant
//! bit first, which is the one order the field types here cannot read: a code
//! is not a field, so the codes are one run of bytes.

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
                // LZW codes, packed least significant bit first, which nothing
                // here unpacks.
                ("compressed", T::bytes(E::Remaining)),
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
}
