//! xz: a stream header, some blocks, an index of the blocks, and a footer.
//!
//! The footer is what makes the rest findable. Its backward size says how
//! long the index is, in units of four bytes, so the index can be placed from
//! the end of the file without reading anything in between, and the blocks
//! are then everything between the header and the index. That is the one
//! measurement this format needs which no field in front of it gives.
//!
//! The index is the part worth having. It holds, for every block, how many
//! bytes it took and how many it produced, which is the whole shape of the
//! file without decompressing any of it.
//!
//! A block's own header says how long the compressed data is only when the
//! encoder chose to write it, which the `xz` tool does not. Without it a block
//! runs to the end of the block region, which is right for the one-block files
//! almost every `.xz` is and stops short of splitting a multi-block file the
//! index has already described.
//!
//! Not read: the compressed data, and the filter chain in a block header,
//! which is a list of identifiers and each filter's own properties.

use crate::codec::Codec;
use crate::template::{Endian::{Big, Little}, Expr as E, Part, Template, Ty as T};

/// What one of these starts with.
pub const MAGIC: &[u8] = b"\xfd7zXZ\x00";

/// The two bytes at the very end.
const FOOTER_MAGIC: &[u8] = b"YZ";

/// What the integrity check at the end of every block is.
const CHECKS: &[(i128, &str)] = &[(0, "none"), (1, "crc32"), (4, "crc64"), (10, "sha256")];

/// How long the index is: the footer's backward size, eight bytes from the end
/// of the file, counted in units of four bytes and written one short. The
/// distance is in bits, and negative, which is what reads from the end.
fn index_size() -> E {
    E::peek_at(E::lit(-8 * 8), 32, Little).add(E::lit(1)).mul(E::lit(4))
}

pub fn xz() -> Template {
    Template::new("xz", part(super::decoded_text()).root)
}

/// The same stream, for a format that carries one inside itself. A ROOT record
/// compressed with `XZ` is a nine-byte block header and then a whole xz stream,
/// footer and all, which is what makes the index at the end of it findable.
///
/// `inner` is what the blocks turn out to hold, which is the caller's business:
/// a file of its own holds text, and a ROOT record holds an object.
pub fn part(inner: T) -> Part {
    Part::new(
        T::structure(
            "XzStream",
            vec![
                ("magic", T::magic(MAGIC)),
                ("stream_flags_reserved", T::u8()),
                ("stream_flags_check_reserved", T::UInt { bits: 4, endian: Big }),
                // Which check every block ends with, which is also what says
                // how long that check is.
                ("check_type", T::enumeration("XzCheck", T::UInt { bits: 4, endian: Big }, CHECKS)),
                ("stream_flags_crc32", T::u32(Little)),
                // The length of that check, as a number the blocks can be
                // measured against rather than as bytes anybody wrote.
                ("check_size", check_size()),
                // Everything between the header and the index.
                (
                    "blocks",
                    T::sized(
                        E::Remaining.sub(index_size()).sub(E::lit(12)),
                        T::repeat(block(), crate::template::Until::End),
                    ),
                ),
                ("index", T::sized(index_size(), index())),
                ("footer", footer()),
            // What the whole stream comes to. Nothing in it can be opened on
            // its own: a block is a step of a decoder's state and not a run
            // that stands by itself, so the field that holds the answer is the
            // stream. It costs no bytes where it stands and covers the stream
            // from its first byte, which is the file when the stream is the
            // file and the block payload when a ROOT record carries one.
                ("decoded", T::at_in_window(E::lit(0), T::decoded(E::Remaining, Codec::Xz, inner))),

            ],
        ),
    )
}

/// How many bytes the stream's check takes after every block. A field of no
/// bits: the number is in the header's check type, and the blocks need it as
/// a length.
fn check_size() -> T {
    T::switch(
        E::field("check_type"),
        vec![
            (0, T::computed(E::lit(0))),
            (1, T::computed(E::lit(4))),
            (4, T::computed(E::lit(8))),
            (10, T::computed(E::lit(32))),
        ],
        T::computed(E::lit(0)),
    )
}

/// One block: a header whose first byte says how long it is, the compressed
/// data, padding to a multiple of four, and the check.
fn block() -> T {
    T::structure(
        "XzBlock",
        vec![
            // The header's length in units of four bytes, written one short.
            ("header_size", T::u8()),
            ("uncompressed_size_present", T::UInt { bits: 1, endian: Big }),
            ("compressed_size_present", T::UInt { bits: 1, endian: Big }),
            ("flags_reserved", T::UInt { bits: 4, endian: Big }),
            // How many filters the data went through, written one short.
            ("filter_count", T::UInt { bits: 2, endian: Big }),
            ("compressed_size", T::present_if(E::field("compressed_size_present"), T::leb_u())),
            ("uncompressed_size", T::present_if(E::field("uncompressed_size_present"), T::leb_u())),
            // The filter chain and the padding after it, which fill the
            // header out to the length its first byte gave.
            (
                "filter_flags",
                T::bytes(
                    E::field("header_size")
                        .add(E::lit(1))
                        .mul(E::lit(4))
                        .sub(E::lit(6))
                        .sub(E::size_of("compressed_size"))
                        .sub(E::size_of("uncompressed_size"))
                        .at_least(E::lit(0)),
                ),
            ),
            ("header_crc32", T::u32(Little)),
            // The compressed data. Its length is in the header only when the
            // encoder wrote it there; without it, everything left in the block
            // region but the check, which takes the block's own padding in
            // with the data since nothing left says where one ends.
            (
                "compressed",
                T::bytes(E::field("compressed_size").or(E::Remaining.sub(E::field("check_size")).at_least(E::lit(0)))),
            ),
            ("block_padding", T::bytes(E::size_of("compressed").pad_to(4))),
            (
                "check",
                T::switch(
                    E::field("check_type"),
                    vec![(0, T::bytes(E::lit(0))), (1, T::u32(Little)), (4, T::u64(Little)), (10, T::bytes(E::lit(32)))],
                    T::bytes(E::lit(0)),
                ),
            ),
        ],
    )
    .counted_as("block")
}

/// The index: one record per block, saying what the block cost and what it
/// held. This is the file's shape, and it is here rather than in the blocks
/// so that a decoder can seek without reading them.
fn index() -> T {
    T::structure(
        "XzIndex",
        vec![
            ("indicator", T::magic(&[0])),
            ("record_count", T::leb_u()),
            (
                "records",
                T::array(
                    T::inline_structure(
                        "XzIndexRecord",
                        vec![
                            // The block without its padding: header and data
                            // and check.
                            ("unpadded_size", T::leb_u()),
                            ("uncompressed_size", T::leb_u()),
                        ],
                    )
                    .counted_as("record"),
                    E::field("record_count"),
                ),
            ),
            ("index_padding", T::bytes(E::Remaining.sub(E::lit(4)).at_least(E::lit(0)))),
            ("index_crc32", T::u32(Little)),
        ],
    )
}

/// The last twelve bytes, which say how long the index is and repeat the
/// stream flags so that a reader working backwards knows the check as well.
fn footer() -> T {
    T::structure(
        "XzFooter",
        vec![
            ("footer_crc32", T::u32(Little)),
            ("backward_size", T::u32(Little)),
            ("stream_flags", T::u16(Big)),
            ("magic", T::magic(FOOTER_MAGIC)),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, eval::Evaluator, source::MemSource};

    /// A stream of one block whose header does not say how long its data is,
    /// which is what `xz` writes.
    fn stream(data: &[u8]) -> Vec<u8> {
        let mut v = MAGIC.to_vec();
        v.extend_from_slice(&[0x00, 0x01]);
        v.extend_from_slice(&0u32.to_le_bytes());
        // A block header of eight bytes: one filter, no sizes.
        let block_start = v.len();
        v.extend_from_slice(&[0x01, 0x00, 0x21, 0x01, 0x00, 0x00, 0x00, 0x00]);
        v.extend_from_slice(data);
        while (v.len() - block_start) % 4 != 0 {
            v.push(0);
        }
        v.extend_from_slice(&0u32.to_le_bytes()); // the crc32 check
        // The size of the block without its padding: header, data and check.
        let unpadded = (8 + data.len() + 4) as u8;
        let _ = block_start;

        let index_start = v.len();
        v.extend_from_slice(&[0x00, 0x01, unpadded, data.len() as u8]);
        while (v.len() - index_start) % 4 != 0 {
            v.push(0);
        }
        v.extend_from_slice(&0u32.to_le_bytes());
        let index_size = v.len() - index_start;

        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&((index_size / 4 - 1) as u32).to_le_bytes());
        v.extend_from_slice(&[0x00, 0x01]);
        v.extend_from_slice(FOOTER_MAGIC);
        v
    }

    #[test]
    fn the_footer_places_the_index_and_the_index_places_the_blocks() {
        let d = Document::new(MemSource(stream(b"compressed bytes here")));
        let mut e = Evaluator::new(xz());
        assert_eq!(e.node(&d, &[3]).unwrap().value.as_int(), Some(1));
        assert_eq!(e.node(&d, &[5]).unwrap().value.as_int(), Some(4));
        // One block, whose data runs to the padding and the check.
        assert_eq!(e.node(&d, &[6]).unwrap().child_count, 1);
        assert_eq!(e.node(&d, &[6, 0, 9]).unwrap().size_bits, 24 * 8);
        // One index record, saying the same block's two sizes.
        assert_eq!(e.node(&d, &[7, 2]).unwrap().child_count, 1);
        assert_eq!(e.node(&d, &[7, 2, 0, 0]).unwrap().value.as_int(), Some(33));
        assert_eq!(e.node(&d, &[8, 3]).unwrap().size_bits, 2 * 8);
    }
}
