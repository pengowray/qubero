//! A 7z archive: thirty-two bytes at the front that say where everything else
//! is, the packed streams, and a header at the end describing them.
//!
//! Putting the header last is what lets the archiver write an entry before it
//! knows how large the entry turned out to be, and the front of the file is
//! then a pointer to it: an offset, a length, and a checksum, with a second
//! checksum over those three so that a truncated file is told from a corrupt
//! one.
//!
//! The header itself is where the names, the sizes, the times and the folder
//! structure live, and it is normally compressed: what the offset points at is
//! then not the header but an encoded stream that unpacks into one. So the
//! interesting half of this format is behind an LZMA decoder, and what is left
//! is the front, which is the part that says whether the file is whole.

use crate::template::{Endian::Little, Expr as E, Template, Ty as T};

/// What one of these starts with.
pub const MAGIC: &[u8] = b"7z\xbc\xaf\x27\x1c";

pub fn sevenzip() -> Template {
    Template::new(
        "7z",
        T::structure(
            "SevenZipArchive",
            vec![
                ("magic", T::magic(MAGIC)),
                ("version_major", T::u8()),
                ("version_minor", T::u8()),
                // A CRC-32 of the twenty bytes after it, which is what says
                // the three numbers below can be trusted.
                ("start_header_crc", T::u32(Little)),
                // Where the header is, counted from the end of these
                // thirty-two bytes rather than from the start of the file.
                ("next_header_offset", T::u64(Little)),
                ("next_header_size", T::u64(Little)),
                ("next_header_crc", T::u32(Little)),
                // Everything the archive holds, as the archiver packed it:
                // runs of compressed bytes whose boundaries are described in
                // the header and nowhere else.
                ("packed_streams", T::bytes(E::field("next_header_offset"))),
                // The header, which is usually itself a compressed stream and
                // is left as bytes either way.
                ("next_header", T::bytes(E::field("next_header_size"))),
            ],
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, eval::Evaluator, source::MemSource};

    fn archive(packed: &[u8], header: &[u8]) -> Vec<u8> {
        let mut v = MAGIC.to_vec();
        v.extend_from_slice(&[0, 4]);
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&(packed.len() as u64).to_le_bytes());
        v.extend_from_slice(&(header.len() as u64).to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(packed);
        v.extend_from_slice(header);
        v
    }

    #[test]
    fn the_front_places_the_packed_streams_and_the_header_after_them() {
        let d = Document::new(MemSource(archive(b"packed bytes", b"\x01\x00")));
        let mut e = Evaluator::new(sevenzip());
        assert_eq!(e.node(&d, &[7]).unwrap().offset_bits, 32 * 8);
        assert_eq!(e.node(&d, &[7]).unwrap().size_bits, 12 * 8);
        assert_eq!(e.node(&d, &[8]).unwrap().offset_bits, 44 * 8);
        assert_eq!(e.node(&d, &[8]).unwrap().size_bits, 2 * 8);
    }

    /// An archive with nothing in it: the header sits straight after the
    /// front, because there are no packed streams to skip.
    #[test]
    fn an_empty_archive_has_no_packed_streams_at_all() {
        let d = Document::new(MemSource(archive(b"", b"\x01\x00")));
        let mut e = Evaluator::new(sevenzip());
        assert_eq!(e.node(&d, &[7]).unwrap().size_bits, 0);
        assert_eq!(e.node(&d, &[8]).unwrap().offset_bits, 32 * 8);
    }
}
