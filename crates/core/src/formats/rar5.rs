//! RAR 5: eight bytes of signature and then a chain of blocks, each of which
//! is a checksum, a size, and a header whose fields depend on what kind of
//! block it is.
//!
//! Every number in a header is a variable-length integer: seven bits to a
//! byte, least significant group first, with the top bit set on every byte but
//! the last. So no field in a header is at a fixed offset, and the size that
//! comes before them is what lets a reader step over a block it does not
//! understand.
//!
//! Two flag bits are what the block layout turns on: one says an extra area
//! follows the fields, and the other says a data area follows the header
//! altogether. A file block has both, and the data area is the file.
//!
//! What is not read here: the fields particular to each kind of block, which
//! are the file names, sizes, times and compression parameters, and the
//! encryption that may cover the headers themselves. The chain, the kinds and
//! the sizes are what this shows.

use crate::template::{Endian::Little, Expr as E, Template, Until, Ty as T};

/// What one of these starts with. RAR 4 has the same first six bytes and one
/// less at the end, so the eighth byte is what tells the two apart.
pub const MAGIC: &[u8] = b"Rar!\x1a\x07\x01\x00";

/// The block kinds. The last one ends the archive, which is what stops the
/// chain.
const TYPES: &[(i128, &str)] =
    &[(1, "main archive"), (2, "file"), (3, "service"), (4, "archive encryption"), (5, "end of archive")];

/// The kind of block the chain stops at.
const END_OF_ARCHIVE: i128 = 5;

const FLAGS: &[(u32, &str)] = &[
    (0, "extra area"),
    (1, "data area"),
    (2, "skip if unknown"),
    (3, "data continues from previous volume"),
    (4, "data continues in next volume"),
    (5, "depends on previous block"),
    (6, "child block"),
    (7, "inherited"),
];

pub fn rar5() -> Template {
    Template::new(
        "rar5",
        T::structure(
            "Rar5Archive",
            vec![
                ("magic", T::magic(MAGIC)),
                (
                    "blocks",
                    T::repeat(block(), Until::FieldValue { field: "block_type".into(), value: END_OF_ARCHIVE }),
                ),
            ],
        ),
    )
}

/// One block: a checksum over everything from the size field to the end of
/// the header, the header itself in the window that size measures out, and
/// then the data area when the flags said there is one.
fn block() -> T {
    T::structure(
        "Rar5Block",
        vec![
            ("header_crc32", T::u32(Little)),
            // How long the header is, counted from the byte after this field.
            ("header_size", T::leb_u()),
            ("header", T::sized(E::field("header_size"), header())),
            // Which kind this was, taken back out of the header so that the
            // chain can stop at the block that ends the archive.
            ("block_type", T::computed(E::within(&["header", "header_type"]))),
            // The file, or whatever else the block put outside its header.
            ("data", T::bytes(E::within(&["header", "data_size"]))),
        ],
    )
    .counted_as("block")
}

fn header() -> T {
    T::structure(
        "Rar5Header",
        vec![
            ("header_type", T::enumeration("Rar5BlockType", T::leb_u(), TYPES)),
            ("header_flags", T::flags("Rar5HeaderFlags", T::leb_u(), FLAGS)),
            ("extra_area_size", T::present_if(E::field("header_flags").bit(0), T::leb_u())),
            ("data_size", T::present_if(E::field("header_flags").bit(1), T::leb_u())),
            // The fields this kind of block carries, and the extra area after
            // them: names, times, and the records a newer version of the
            // format added without moving anything.
            ("header_fields", T::bytes(E::Remaining)),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::Document,
        eval::{Evaluator, Value},
        source::MemSource,
    };

    fn block_bytes(block_type: u8, flags: u8, rest: &[u8]) -> Vec<u8> {
        let mut header = vec![block_type, flags];
        header.extend_from_slice(rest);
        let mut v = 0u32.to_le_bytes().to_vec();
        v.push(header.len() as u8);
        v.extend_from_slice(&header);
        v
    }

    /// An archive of one file block between the main header and the end: the
    /// chain stops at the end block, and the file's bytes are outside the
    /// header the flags placed them after.
    fn archive() -> Vec<u8> {
        let mut v = MAGIC.to_vec();
        v.extend_from_slice(&block_bytes(1, 0, b"\x00"));
        v.extend_from_slice(&block_bytes(2, 0x02, b"\x09rest of it"));
        v.extend_from_slice(b"file data");
        v.extend_from_slice(&block_bytes(5, 0, b"\x00"));
        v
    }

    #[test]
    fn the_chain_stops_at_the_block_that_ends_the_archive() {
        let d = Document::new(MemSource(archive()));
        let mut e = Evaluator::new(rar5());
        assert_eq!(e.node(&d, &[1]).unwrap().child_count, 3);
        assert_eq!(
            e.node(&d, &[1, 0, 2, 0]).unwrap().value,
            Value::Enum { raw: 1, name: Some("main archive".into()), hex: false }
        );
        // The file block: a data area of nine bytes, after its header.
        assert_eq!(e.node(&d, &[1, 1, 2, 3]).unwrap().value.as_int(), Some(9));
        assert_eq!(e.node(&d, &[1, 1, 4]).unwrap().size_bits, 9 * 8);
        assert_eq!(e.node(&d, &[1, 2, 3]).unwrap().value.as_int(), Some(5));
        // A block with no data area has none, rather than reading to the end.
        assert_eq!(e.node(&d, &[1, 0, 4]).unwrap().size_bits, 0);
    }
}
