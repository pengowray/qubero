//! A zlib stream, RFC 1950: two bytes of header, deflate, and a checksum of
//! what the deflate stream would produce.
//!
//! Nothing here announces itself. The two header bytes are a compression
//! method and a window size in one nibble each, a preset-dictionary bit, a
//! compression level, and five bits chosen so that the pair read as a
//! big-endian number is a multiple of 31. That last rule is the whole of the
//! evidence a file is one of these, which is why the sniffer asks about it
//! after every format with a signature has spoken.
//!
//! The deflate stream in the middle has no length on it, so it is everything
//! between the header and the four-byte checksum at the end, the same way a
//! gzip member's body is.

use crate::template::{Endian::Big, Expr as E, Part, Template, Ty as T};

/// The compression level the header names, which says how hard the encoder
/// tried rather than anything a decoder needs.
const LEVELS: &[(i128, &str)] = &[(0, "fastest"), (1, "fast"), (2, "default"), (3, "best compression")];

pub fn zlib() -> Template {
    Template::new("zlib", part().root)
}

/// The same stream, for a format that carries one inside itself. A ROOT record
/// compressed with `ZL` is a nine-byte block header and then exactly this.
pub fn part() -> Part {
    Part::new(
        T::structure(
            "ZlibStream",
            vec![
                // The window the encoder used, as a power of two from 256
                // bytes up, and the method, which has only ever been deflate.
                ("cinfo", T::UInt { bits: 4, endian: Big }),
                ("cm", T::enumeration("ZlibMethod", T::UInt { bits: 4, endian: Big }, &[(8, "deflate")])),
                ("flevel", T::enumeration("ZlibLevel", T::UInt { bits: 2, endian: Big }, LEVELS)),
                ("fdict", T::UInt { bits: 1, endian: Big }),
                // The five bits that make the header a multiple of 31.
                ("fcheck", T::UInt { bits: 5, endian: Big }),
                // The Adler-32 of a dictionary the two ends agreed on
                // beforehand, written only when the bit above says there is
                // one. The dictionary itself is not in the file.
                ("dictid", T::present_if(E::field("fdict"), T::u32(Big))),
                // Deflate, which nothing here unpacks.
                ("compressed", T::bytes(E::Remaining.sub(E::lit(4)))),
                // Adler-32 of the uncompressed data, big-endian, which is the
                // one thing in this format that is not little-endian.
                ("adler32", T::u32(Big)),
            ],
        ),
    )
}

/// Whether these bytes open a zlib stream. Method 8, a window no larger than
/// the format allows, and the header's own check: the two bytes as a
/// big-endian number are a multiple of 31.
pub fn is_zlib(head: &[u8]) -> bool {
    let (Some(&cmf), Some(&flg)) = (head.first(), head.get(1)) else { return false };
    // A stream with a preset dictionary needs the dictionary to be read at
    // all, and a file holding one on its own is not a thing anybody writes.
    cmf & 0x0f == 8 && cmf >> 4 <= 7 && flg & 0x20 == 0 && (cmf as u16 * 256 + flg as u16) % 31 == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::Document,
        eval::{Evaluator, Value},
        source::MemSource,
    };

    fn stream(body: &[u8]) -> Vec<u8> {
        let mut v = vec![0x78, 0x9c];
        v.extend_from_slice(body);
        v.extend_from_slice(&1u32.to_be_bytes());
        v
    }

    #[test]
    fn the_header_is_two_bytes_and_the_body_runs_to_the_checksum() {
        let d = Document::new(MemSource(stream(&[0x03, 0x00])));
        let mut e = Evaluator::new(zlib());
        assert_eq!(e.node(&d, &[0]).unwrap().value.as_int(), Some(7));
        assert_eq!(e.node(&d, &[1]).unwrap().value, Value::Enum { raw: 8, name: Some("deflate".into()), hex: false });
        assert_eq!(e.node(&d, &[5]).unwrap().size_bits, 0);
        assert_eq!(e.node(&d, &[6]).unwrap().offset_bits, 2 * 8);
        assert_eq!(e.node(&d, &[6]).unwrap().size_bits, 2 * 8);
        assert_eq!(e.node(&d, &[7]).unwrap().value.as_int(), Some(1));
    }

    #[test]
    fn the_check_bits_are_what_recognises_one() {
        assert!(is_zlib(&[0x78, 0x9c]));
        assert!(is_zlib(&[0x78, 0x01]));
        assert!(is_zlib(&[0x78, 0xda]));
        // The right method, the wrong five bits.
        assert!(!is_zlib(&[0x78, 0x9d]));
        // Not deflate.
        assert!(!is_zlib(&[0x79, 0x9c]));
    }
}
