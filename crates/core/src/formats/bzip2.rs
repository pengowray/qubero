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
//! So the fields here stop after the first block's marker and checksum. What
//! follows them is declared as one run of bits called `compressed`, and that
//! run is the rest of the file: any second block, any block after that, and
//! the end-of-stream marker are all inside it and none of them is named.
//! That is where this template stops being a description of the format and
//! starts being a description of what can be addressed. Looking for the block
//! magic in the bytes would find it only in the files where it happens to be
//! aligned, and a split that is right one time in eight is worse than no
//! split at all.
//!
//! What is not addressable is still readable. The last field opens the whole
//! stream, from its `BZh` to its last bit, and the bytes that come out of it
//! are where the file's contents actually are. The fields above lie over the
//! same bytes, which is what [`super::xz`] does for the same reason: a
//! compressed stream is one run to a decoder and a header with fields in it to
//! a reader, and both are true at once.

use crate::codec::Codec;
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
                // What the whole stream comes to. Not what `compressed` comes
                // to: a block is a step of a decoder's state, its bits begin
                // and end wherever the block before it left off, and only the
                // stream stands on its own. So this costs no bytes where it
                // stands and covers the stream from its first byte, the same
                // bytes the header fields above were read from.
                ("decoded", T::at_in_window(E::lit(0), T::decoded(E::Remaining, Codec::Bzip2, super::decoded_text()))),
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
            // CRC-32 of this block's uncompressed bytes, most significant bit
            // first: polynomial 0x04c11db7, in and out inverted, and the bits
            // of every byte the other way round from the CRC-32 gzip and PNG
            // write. It is not `Checksum::Crc32` and would need a variant of
            // its own.
            //
            // Declared as a number and not as a check, on purpose. What it
            // covers is one block's share of the unpacked bytes, and nothing
            // here knows where that share ends: the trace over a bzip2 stream
            // is one step across all of it, because the crate that reads one
            // gives bytes and says nothing about where a block stopped. A
            // check written here would sum the *whole* output against the
            // *first* block's number and report a mismatch on every file with
            // more than one block. That is a false mismatch on a file that is
            // fine, which is the one thing `eval::check` exists to prevent.
            // Block boundaries in the trace come first; then the sum, and only
            // then this.
            ("block_crc", T::u32(Big)),
            // Huffman-coded bits, which nothing addresses: after them come
            // whatever blocks follow and the end of the stream, and where any
            // of that begins is not a number of bytes. Read as unpacked bytes
            // by the stream's `decoded` field, not by anything in here.
            ("compressed", T::bytes(E::Remaining)),
        ],
    )
}

/// The end of a stream that held no blocks: the marker, and a checksum of all
/// the block checksums, which for no blocks is zero.
///
/// Not a check either, and for a reason of its own: it is not a sum over any
/// bytes. Each block's number is rotated into the one before it, so what it
/// covers is the other checksums rather than the file or the output.
fn end_of_stream() -> T {
    T::structure(
        "Bzip2StreamEnd",
        vec![("end_magic", T::magic(END_MAGIC)), ("combined_crc", T::u32(Big)), ("padding", T::bytes(E::Remaining))],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, eval::Evaluator, source::MemSource, Value};

    /// A real stream over `data`, written by the encoder the tests keep for
    /// the purpose. `level` is the block size in hundreds of kilobytes.
    fn pack(data: &[u8], level: u32) -> Vec<u8> {
        use std::io::Write;
        let mut e = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::new(level));
        e.write_all(data).expect("writes");
        e.finish().expect("finishes")
    }

    /// Where every block marker in a stream is, counted in bits from the front
    /// of it. A rolling window rather than a search through the bytes: the
    /// markers after the first are not on byte boundaries, which is the whole
    /// reason the fields above stop where they do.
    fn markers(data: &[u8]) -> Vec<u64> {
        let mut window = 0u64;
        let mut out = Vec::new();
        for i in 0..data.len() as u64 * 8 {
            let bit = (data[(i / 8) as usize] >> (7 - i % 8)) & 1;
            window = (window << 1 | bit as u64) & 0xffff_ffff_ffff;
            if i >= 47 && window == BLOCK_MAGIC as u64 {
                out.push(i - 47);
            }
        }
        out
    }

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

    /// The header fields and the opened stream over the same bytes: the block
    /// size is read where it is written, and the text comes out of the run
    /// that starts four bytes in front of it.
    #[test]
    fn a_stream_opens_into_what_was_compressed() {
        let text = "Qubero reads the shape of a compressed file without decompressing it.";
        let packed = pack(text.as_bytes(), 9);
        let d = Document::new(MemSource(packed));
        let mut e = Evaluator::new(bzip2());
        assert_eq!(e.node(&d, &[1]).unwrap().value.as_int(), Some(9));
        let node = e.node(&d, &[3, 0, 0, 0]).unwrap();
        assert_eq!(node.value, Value::Str(text.into()));
        // A space of its own, starting at its own front, and nowhere in the
        // file to write it back to.
        assert_ne!(node.space, 0);
        assert_eq!(node.offset_bits, 0);
        assert!(!node.editable);
    }

    /// A stream of several blocks, which is what a low block size over enough
    /// data gives. The fields describe the first block only; the field that
    /// opens the stream gives back every byte that went in.
    #[test]
    fn a_stream_of_several_blocks_still_opens_whole() {
        // No run of four repeated bytes anywhere, so bzip2's run-length pass
        // in front of the blocking does not shrink this before it is split.
        let text = "the quick brown fox jumps over the lazy dog, and again: ".repeat(4500);
        assert!(text.len() > 250_000);
        // A block size of one hundred kilobytes, so this cannot be one block.
        let packed = pack(text.as_bytes(), 1);

        let at = markers(&packed);
        assert_eq!(at.len(), 3, "three blocks of a hundred kilobytes each");
        assert_eq!(at[0], 32, "the first block starts where the four-byte header ends");
        assert!(at[1] % 8 != 0, "the second block does not start on a byte, which is why it is not a field");

        let d = Document::new(MemSource(packed));
        let mut e = Evaluator::new(bzip2());
        // The first block's marker is a field; the rest of the stream is the
        // one run the template calls `compressed`.
        assert_eq!(e.node(&d, &[2, 0]).unwrap().value.as_int(), Some(BLOCK_MAGIC));
        // The whole of it came out: the field is as long as what went in.
        // What the value holds is the first 256 bytes and an ellipsis, which
        // is all any text field shows; the bytes themselves are checked
        // against the input in `codec::bzip2`.
        let node = e.node(&d, &[3, 0, 0, 0]).unwrap();
        assert_eq!(node.size_bits, text.len() as u64 * 8);
        let Value::Str(shown) = node.value else { panic!("the stream did not open") };
        assert!(text.starts_with(shown.trim_end_matches('\u{2026}')), "the stream opened into something else");
    }
}
