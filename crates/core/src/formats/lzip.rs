//! lzip: an LZMA stream with a header that says how large a dictionary it
//! needs and a trailer that says what came out.
//!
//! The dictionary size is one byte holding two numbers. The low five bits are
//! a power of two, and the top three subtract that many thirty-seconds of it,
//! so a size between two powers can be named without a second field. The
//! template reads the two halves; multiplying them out is arithmetic a reader
//! can do with both numbers in front of them.
//!
//! The stream between header and trailer has no length on it, so it is
//! measured from the end: the last twenty bytes are the trailer, and
//! everything before them is LZMA.
//!
//! A file may hold several members one after another, each with its own
//! header and trailer. This reads the first, whose trailer says how long it
//! is, and a second member would be past where this template stops.

use crate::template::{Endian::{Big, Little}, Expr as E, Template, Ty as T};

/// What one of these starts with.
pub const MAGIC: &[u8] = b"LZIP";

/// The trailer: a checksum, what came out, and how long the member is.
const TRAILER: i128 = 20;

pub fn lzip() -> Template {
    Template::new(
        "lzip",
        T::structure(
            "LzipMember",
            vec![
                ("magic", T::magic(MAGIC)),
                ("version", T::u8()),
                // Two numbers in one byte: subtract this many thirty-seconds
                // of the base below from it.
                ("dict_size_fraction", T::UInt { bits: 3, endian: Big }),
                // The base, as a power of two: 12 is 4 KiB, 29 is 512 MiB.
                ("dict_size_base", T::UInt { bits: 5, endian: Big }),
                // The LZMA stream, which nothing here unpacks. It ends with a
                // marker rather than a length, so it is measured backwards
                // from the trailer.
                ("lzma_stream", T::bytes(E::Remaining.sub(E::lit(TRAILER)))),
                ("crc32", T::u32(Little)),
                ("data_size", T::u64(Little)),
                // The whole member including this field, which is what makes
                // the next member in a multi-member file findable.
                ("member_size", T::u64(Little)),
            ],
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, eval::Evaluator, source::MemSource};

    fn member(lzma: &[u8]) -> Vec<u8> {
        let mut v = MAGIC.to_vec();
        v.push(1);
        v.push(0x0c);
        v.extend_from_slice(lzma);
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&0u64.to_le_bytes());
        let size = (v.len() + 8) as u64;
        v.extend_from_slice(&size.to_le_bytes());
        v
    }

    #[test]
    fn the_stream_is_measured_back_from_the_trailer() {
        let d = Document::new(MemSource(member(b"\x00\x01\x02\x03\x04")));
        let mut e = Evaluator::new(lzip());
        assert_eq!(e.node(&d, &[2]).unwrap().value.as_int(), Some(0));
        assert_eq!(e.node(&d, &[3]).unwrap().value.as_int(), Some(12));
        assert_eq!(e.node(&d, &[4]).unwrap().size_bits, 5 * 8);
        assert_eq!(e.node(&d, &[7]).unwrap().value.as_int(), Some(31));
    }
}
