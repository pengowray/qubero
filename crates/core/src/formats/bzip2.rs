//! bzip2: four bytes of header, then blocks of up to nine hundred kilobytes
//! each, then a magic number and a checksum saying the stream is over.
//!
//! The header is bytes and everything after it is bits. A block starts with
//! the six bytes of pi that mark one, and the first block starts thirty-two
//! bits into the file, so it lands on a byte boundary; the second does not,
//! and neither does the end-of-stream marker after it. Nothing is padded
//! until the very end of the file, so where the second block begins depends
//! on how many bits the first one took, which is only known by decoding it.
//!
//! So this reads the stream header, the first block's marker and checksum,
//! and leaves the rest as the run of bits it is. Looking for the block magic
//! in the bytes would find it only in the files where it happens to be
//! aligned, and a split that is right one time in eight is worse than no
//! split at all.

use crate::template::{Endian::Big, Expr as E, StrLen, Template, Ty as T};

/// What one of these starts with. The letter after it is the block size.
pub const MAGIC: &[u8] = b"BZh";

/// The six bytes that open a block: the first digits of pi, as BCD.
const BLOCK_MAGIC: i128 = 0x3141_5926_5359;

/// The six that close the stream: the square root of pi, the same way.
const END_MAGIC: &[u8] = b"\x17\x72\x45\x38\x50\x90";

pub fn bzip2() -> Template {
    Template::new(
        "bzip2",
        T::structure(
            "Bzip2Stream",
            vec![
                ("magic", T::magic(MAGIC)),
                // A digit from 1 to 9: the block size in hundreds of
                // kilobytes.
                ("block_size_100k", T::decimal(StrLen::Fixed(E::lit(1)))),
                // A stream that compressed nothing has no blocks at all, and
                // then the end marker is what is here instead.
                (
                    "body",
                    T::switch(
                        E::peek(48, Big),
                        vec![(BLOCK_MAGIC, first_block())],
                        end_of_stream(),
                    ),
                ),
            ],
        ),
    )
}

/// The first block, which is the only one whose header is byte-aligned. Its
/// compressed bits run to the end of the file, taking any later block and the
/// end-of-stream marker with them.
fn first_block() -> T {
    T::structure(
        "Bzip2Block",
        vec![
            ("block_magic", T::UInt { bits: 48, endian: Big }),
            // CRC-32 of this block's uncompressed bytes, with the bits of
            // every byte reversed against the CRC gzip and PNG use.
            ("block_crc", T::u32(Big)),
            // Huffman-coded bits, which nothing here unpacks, and after them
            // whatever blocks follow and the end of the stream.
            ("compressed", T::bytes(E::Remaining)),
        ],
    )
}

/// The end of a stream that held no blocks: the marker, and a checksum of all
/// the block checksums, which for no blocks is zero.
fn end_of_stream() -> T {
    T::structure(
        "Bzip2StreamEnd",
        vec![("end_magic", T::magic(END_MAGIC)), ("combined_crc", T::u32(Big)), ("padding", T::bytes(E::Remaining))],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, eval::Evaluator, source::MemSource};

    #[test]
    fn a_stream_with_a_block_reads_its_marker_and_checksum() {
        let mut v = b"BZh9".to_vec();
        v.extend_from_slice(&[0x31, 0x41, 0x59, 0x26, 0x53, 0x59]);
        v.extend_from_slice(&0xdead_beefu32.to_be_bytes());
        v.extend_from_slice(&[0x80, 0x00, 0x00]);
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(bzip2());
        assert_eq!(e.node(&d, &[1]).unwrap().value.as_int(), Some(9));
        assert_eq!(e.node(&d, &[2, 1]).unwrap().value.as_int(), Some(0xdead_beef));
        assert_eq!(e.node(&d, &[2, 2]).unwrap().size_bits, 3 * 8);
    }

    /// `bzip2` given nothing to compress writes the header and the end marker
    /// and stops, and then the marker is where a block would have been.
    #[test]
    fn an_empty_stream_is_the_end_marker_and_nothing_else() {
        let mut v = b"BZh1".to_vec();
        v.extend_from_slice(END_MAGIC);
        v.extend_from_slice(&0u32.to_be_bytes());
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(bzip2());
        assert_eq!(e.node(&d, &[2, 1]).unwrap().value.as_int(), Some(0));
        assert_eq!(e.node(&d, &[2, 2]).unwrap().size_bits, 0);
    }
}
