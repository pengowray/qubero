//! Zstandard: frames, each of which is a descriptor byte, a few numbers that
//! byte says whether to expect, and then blocks.
//!
//! The descriptor is the format's whole argument in one byte. Two bits say how
//! many bytes the frame content size takes, and zero there means no size at
//! all unless the single-segment bit is set, in which case it means one; the
//! single-segment bit also removes the window descriptor, because a frame that
//! fits in one segment is as large as its content and needs no window; two
//! more bits say how many bytes of dictionary identifier follow.
//!
//! A block is a three-byte header packed the other way round from everything
//! else: the low bit says whether it is the last block, the next two say what
//! kind it is, and the top twenty-one are the size. Reading it as one
//! little-endian number and taking the pieces out with arithmetic is the way
//! to say that here, since a field narrower than a byte is packed
//! most-significant first and these are not.
//!
//! A raw block's size is its bytes, a compressed block's size is its
//! compressed bytes, and an RLE block is three bytes of header and one byte
//! that repeats that many times, so the size is not the size.
//!
//! A skippable frame is anything an application wants to put in the middle of
//! a stream: a magic in a range of sixteen, a length, and that many bytes
//! nobody but the application reads.

use crate::template::{Endian::{Big, Little}, Expr as E, Template, Until, Ty as T};

/// What a zstd frame starts with.
pub const MAGIC: &[u8] = b"\x28\xb5\x2f\xfd";

/// The first of the sixteen skippable frame magics. Little-endian, these are
/// 0x184d2a50 through 0x184d2a5f, and lz4 uses the same range.
const SKIPPABLE_FIRST: i128 = 0x184d_2a50;

const BLOCK_TYPES: &[(i128, &str)] = &[(0, "raw"), (1, "rle"), (2, "compressed"), (3, "reserved")];

pub fn zstd() -> Template {
    Template::new(
        "zstd",
        T::structure("ZstdStream", vec![("frames", T::repeat(frame(), Until::End))]),
    )
}

/// One frame, which is either zstd's own or a skippable one an application
/// wrote. Both begin with four bytes, so the choice is made on those.
fn frame() -> T {
    T::switch(
        E::peek(32, Little),
        vec![(0xfd2f_b528, zstd_frame())],
        skippable_frame(),
    )
}

fn zstd_frame() -> T {
    T::structure(
        "ZstdFrame",
        vec![
            ("magic", T::magic(MAGIC)),
            // The descriptor, most significant bit first.
            ("frame_content_size_flag", T::UInt { bits: 2, endian: Big }),
            ("single_segment_flag", T::UInt { bits: 1, endian: Big }),
            ("unused_bit", T::UInt { bits: 1, endian: Big }),
            ("reserved_bit", T::UInt { bits: 1, endian: Big }),
            ("content_checksum_flag", T::UInt { bits: 1, endian: Big }),
            ("dictionary_id_flag", T::UInt { bits: 2, endian: Big }),
            // How much of the decompressed data a decoder must keep to hand.
            // Absent when the frame is one segment, since then that is the
            // content size.
            (
                "window_descriptor",
                T::switch(
                    E::field("single_segment_flag"),
                    vec![(0, T::structure("ZstdWindow", vec![("exponent", T::UInt { bits: 5, endian: Big }), ("mantissa", T::UInt { bits: 3, endian: Big })]))],
                    T::bytes(E::lit(0)),
                ),
            ),
            // Which dictionary the frame was compressed against, in zero, one,
            // two or four bytes as the flag says.
            (
                "dictionary_id",
                T::switch(
                    E::field("dictionary_id_flag"),
                    vec![(1, T::u8()), (2, T::u16(Little)), (3, T::u32(Little))],
                    T::bytes(E::lit(0)),
                ),
            ),
            // How large the frame's content is once decompressed. The two
            // flag bits mean zero, two, four or eight bytes, except that zero
            // means one byte in a single-segment frame and nothing otherwise.
            // The two-byte form is the size less 256, which is the one reading
            // here that is not the number the field holds.
            (
                "frame_content_size",
                T::switch(
                    E::field("frame_content_size_flag").mul(E::lit(2)).add(E::field("single_segment_flag")),
                    vec![
                        (1, T::u8()),
                        (2, T::u16(Little)),
                        (3, T::u16(Little)),
                        (4, T::u32(Little)),
                        (5, T::u32(Little)),
                        (6, T::u64(Little)),
                        (7, T::u64(Little)),
                    ],
                    T::bytes(E::lit(0)),
                ),
            ),
            ("blocks", T::repeat(block(), Until::FieldValue { field: "last_block".into(), value: 1 })),
            // An xxhash-64 of the decompressed data, cut to its low 32 bits.
            ("content_checksum", T::present_if(E::field("content_checksum_flag"), T::u32(Little))),
        ],
    )
}

/// One block. The header is three bytes read as one number, because its
/// pieces are packed from the low bit up.
fn block() -> T {
    T::structure(
        "ZstdBlock",
        vec![
            ("block_header", T::UInt { bits: 24, endian: Little }),
            ("last_block", T::computed(E::field("block_header").bit(0))),
            (
                "block_type",
                T::enumeration(
                    "ZstdBlockType",
                    T::computed(E::field("block_header").div(E::lit(2)).sub(E::field("block_header").div(E::lit(8)).mul(E::lit(4)))),
                    BLOCK_TYPES,
                ),
            ),
            ("block_size", T::computed(E::field("block_header").div(E::lit(8)))),
            // A block of the third kind holds one byte that stands for
            // `block_size` of them; every other kind holds that many bytes.
            (
                "data",
                T::switch(
                    E::field("block_type"),
                    vec![(1, T::bytes(E::lit(1)))],
                    T::bytes(E::field("block_size")),
                ),
            ),
        ],
    )
    .counted_as("block")
}

/// A frame something other than zstd wrote, which a decoder steps over. The
/// low nibble of the magic is the application's to choose.
fn skippable_frame() -> T {
    T::structure(
        "ZstdSkippableFrame",
        vec![
            ("magic", T::enumeration_hex("ZstdSkippable", T::u32(Little), &[(SKIPPABLE_FIRST, "skippable frame")])),
            ("frame_size", T::u32(Little)),
            ("user_data", T::bytes(E::field("frame_size"))),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, eval::Evaluator, source::MemSource};

    /// A frame of one raw block, with a content size and a checksum.
    fn frame_bytes(content: &[u8]) -> Vec<u8> {
        let mut v = MAGIC.to_vec();
        // Single segment, one byte of content size, a checksum, no dictionary.
        v.push(0b0010_0100);
        v.push(content.len() as u8);
        let header = (content.len() as u32) << 3 | 1;
        v.extend_from_slice(&header.to_le_bytes()[..3]);
        v.extend_from_slice(content);
        v.extend_from_slice(&0u32.to_le_bytes());
        v
    }

    #[test]
    fn a_single_segment_frame_has_no_window_and_a_one_byte_size() {
        let d = Document::new(MemSource(frame_bytes(b"hello")));
        let mut e = Evaluator::new(zstd());
        assert_eq!(e.node(&d, &[0]).unwrap().child_count, 1);
        assert_eq!(e.node(&d, &[0, 0, 2]).unwrap().value.as_int(), Some(1));
        assert_eq!(e.node(&d, &[0, 0, 7]).unwrap().size_bits, 0);
        assert_eq!(e.node(&d, &[0, 0, 9]).unwrap().value.as_int(), Some(5));
        // One block, the last one, raw, five bytes long.
        assert_eq!(e.node(&d, &[0, 0, 10]).unwrap().child_count, 1);
        assert_eq!(e.node(&d, &[0, 0, 10, 0, 1]).unwrap().value.as_int(), Some(1));
        assert_eq!(e.node(&d, &[0, 0, 10, 0, 3]).unwrap().value.as_int(), Some(5));
        assert_eq!(e.node(&d, &[0, 0, 10, 0, 4]).unwrap().size_bits, 5 * 8);
        assert_eq!(e.node(&d, &[0, 0, 11]).unwrap().size_bits, 4 * 8);
    }

    /// A frame nothing but the application understands: four bytes of magic,
    /// a length, and that many bytes stepped over.
    #[test]
    fn a_skippable_frame_is_measured_and_left_alone() {
        let mut v = 0x184d_2a50u32.to_le_bytes().to_vec();
        v.extend_from_slice(&3u32.to_le_bytes());
        v.extend_from_slice(b"abc");
        v.extend_from_slice(&frame_bytes(b"x"));
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(zstd());
        assert_eq!(e.node(&d, &[0]).unwrap().child_count, 2);
        assert_eq!(e.node(&d, &[0, 0, 2]).unwrap().size_bits, 3 * 8);
        assert_eq!(e.node(&d, &[0, 1, 0]).unwrap().offset_bits, 11 * 8);
    }
}
