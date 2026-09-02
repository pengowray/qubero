//! The LZ4 frame format: a magic number, two bytes of options, whatever those
//! two bytes said to expect, and then blocks until one of size zero.
//!
//! Every block is a four-byte size and that many bytes. The top bit of the
//! size says the block was stored rather than compressed, which is what an
//! encoder writes when compressing made the block larger, so the size is the
//! other thirty-one bits. A size of zero is not a block: it is the end mark,
//! and the frame is over.
//!
//! The byte after the options is a checksum of the options, which is why a
//! file with a plausible header is almost certainly one of these.
//!
//! Not read: the compressed data, which is a sequence of literals and matches
//! with no structure a template can measure, and the frames an application
//! writes into an LZ4 stream for its own purposes, which share their magic
//! range with zstd's skippable frames.

use crate::codec::Codec;
use crate::template::{Endian::{Big, Little}, Expr as E, Template, Until, Ty as T};

/// What one of these starts with.
pub const MAGIC: &[u8] = b"\x04\x22\x4d\x18";

/// The largest block the encoder said it would write, which is what a decoder
/// sizes its buffer by.
const BLOCK_MAX: &[(i128, &str)] = &[(4, "64 KiB"), (5, "256 KiB"), (6, "1 MiB"), (7, "4 MiB")];

/// The top bit of a block's size word, which says the block was stored as it
/// came.
const UNCOMPRESSED_BIT: i128 = 1 << 31;

pub fn lz4() -> Template {
    Template::new(
        "lz4",
        T::structure(
            "Lz4Frame",
            vec![
                ("magic", T::magic(MAGIC)),
                // FLG, most significant bit first.
                ("version", T::UInt { bits: 2, endian: Big }),
                // Set when each block can be decompressed on its own, clear
                // when a block may refer back into the one before it.
                ("block_independence", T::UInt { bits: 1, endian: Big }),
                ("block_checksum_flag", T::UInt { bits: 1, endian: Big }),
                ("content_size_flag", T::UInt { bits: 1, endian: Big }),
                ("content_checksum_flag", T::UInt { bits: 1, endian: Big }),
                ("flg_reserved", T::UInt { bits: 1, endian: Big }),
                ("dictionary_id_flag", T::UInt { bits: 1, endian: Big }),
                // BD, which is one number and six reserved bits.
                ("bd_reserved", T::UInt { bits: 1, endian: Big }),
                ("block_max_size", T::enumeration("Lz4BlockMax", T::UInt { bits: 3, endian: Big }, BLOCK_MAX)),
                ("bd_reserved_low", T::UInt { bits: 4, endian: Big }),
                ("content_size", T::present_if(E::field("content_size_flag"), T::u64(Little))),
                ("dictionary_id", T::present_if(E::field("dictionary_id_flag"), T::u32(Little))),
                // A byte of the xxhash-32 of everything between the magic and
                // here, which is what makes a header this small checkable.
                ("header_checksum", T::u8()),
                // Blocks, up to and including the end mark: a size of zero
                // with nothing after it.
                ("blocks", T::repeat(block(), Until::FieldValue { field: "block_header".into(), value: 0 })),
                ("content_checksum", T::present_if(E::field("content_checksum_flag"), T::u32(Little))),
            ],
        ),
    )
}

/// One block, or the end mark, which is a block whose size word is zero and
/// which has nothing else in it at all.
fn block() -> T {
    T::structure(
        "Lz4Block",
        vec![
            ("block_header", T::u32(Little)),
            ("uncompressed", T::computed(E::field("block_header").bit(31))),
            ("block_size", T::computed(E::field("block_header").sub(E::field("block_header").bit(31).mul(E::lit(UNCOMPRESSED_BIT))))),
            // A compressed block is one LZ4 block and opens on its own; a
            // stored block is the bytes as they came and has nothing to open.
            (
                "data",
                T::switch(
                    E::field("uncompressed"),
                    vec![(1, T::bytes(E::field("block_size")))],
                    T::decoded(E::field("block_size"), Codec::Lz4Block, super::decoded_text()),
                ),
            ),
            // Only a real block has one: the end mark is the size word and
            // nothing more.
            (
                "block_checksum",
                T::present_if(
                    E::field("block_checksum_flag").mul(E::lit(0).less_than(E::field("block_size"))),
                    T::u32(Little),
                ),
            ),
        ],
    )
    .counted_as("block")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, eval::Evaluator, source::MemSource};

    /// A frame of one stored block, with a content size and a content
    /// checksum but no per-block checksum.
    pub fn frame(content: &[u8]) -> Vec<u8> {
        let mut v = MAGIC.to_vec();
        v.push(0b0110_1100); // version 1, blocks independent, both sizes checked
        v.push(0b0100_0000); // 64 KiB blocks
        v.extend_from_slice(&(content.len() as u64).to_le_bytes());
        v.push(0x00); // the header checksum, which nothing here computes
        v.extend_from_slice(&((content.len() as u32) | 0x8000_0000).to_le_bytes());
        v.extend_from_slice(content);
        v.extend_from_slice(&0u32.to_le_bytes()); // the end mark
        v.extend_from_slice(&0u32.to_le_bytes()); // the content checksum
        v
    }

    #[test]
    fn blocks_run_to_the_end_mark_and_a_stored_block_says_so() {
        let d = Document::new(MemSource(frame(b"hello lz4")));
        let mut e = Evaluator::new(lz4());
        assert_eq!(e.node(&d, &[9]).unwrap().value.as_int(), Some(4));
        assert_eq!(e.node(&d, &[11]).unwrap().value.as_int(), Some(9));
        assert_eq!(e.node(&d, &[12]).unwrap().size_bits, 0);
        // Two elements: the block, and the end mark that stopped the run.
        assert_eq!(e.node(&d, &[14]).unwrap().child_count, 2);
        assert_eq!(e.node(&d, &[14, 0, 1]).unwrap().value.as_int(), Some(1));
        assert_eq!(e.node(&d, &[14, 0, 2]).unwrap().value.as_int(), Some(9));
        assert_eq!(e.node(&d, &[14, 1]).unwrap().size_bits, 4 * 8);
        assert_eq!(e.node(&d, &[15]).unwrap().size_bits, 4 * 8);
    }
}
