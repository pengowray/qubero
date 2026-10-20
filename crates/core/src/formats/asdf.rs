//! ASDF, the Advanced Scientific Data Format: a YAML tree saying what the data
//! is, then the data in binary blocks.
//!
//! The file opens with `#ASDF 1.0.0` and more comment lines, then the tree as
//! one YAML document ending in `...`. After it come the blocks, each a magic
//! number, a header of sizes and a checksum, and the data, which may be
//! compressed. Last, optionally, a second YAML document listing where each
//! block starts.
//!
//! A block does not say what it holds. The tree does: an `ndarray` node names a
//! block by number and gives its element type, byte order and shape, so each
//! block's contents are a [`Ty::Schema`](crate::template::Ty::Schema) node built
//! from the tree (see `asdf_tree.rs`) and read as the arrays the tree says.
//!
//! The Roman Space Telescope writes its images this way, every block lz4
//! compressed in asdf's own framing: the block's data is chunks, each a
//! big-endian length and then an lz4 block of at most 4 MiB unpacked. The chunks
//! are joined into one space and the array read across them.

use std::sync::Arc;

use crate::codec::Codec;
use crate::template::{Encoding, Endian::*, Expr as E, KeyPart, Step, StrLen, Template, Ty as T, Until};

use super::asdf_tree::{Blocks, KIND};

/// What every ASDF file opens with.
pub const MAGIC: &[u8] = b"#ASDF ";

/// What every block opens with, and the same four bytes as a big-endian number.
const BLOCK_MAGIC: &[u8] = b"\xd3BLK";
const BLOCK_WORD: i128 = 0xd342_4c4b;

/// The four bytes naming a block's compression, read as a big-endian number.
const NONE: i128 = 0;
const ZLIB: i128 = 0x7a6c_6962;
const BZP2: i128 = 0x627a_7032;
const LZ4: i128 = 0x6c7a_3400;

/// The block header's fields after the size of it, which is how long a header
/// of this version of the standard is.
const HEADER_SIZE: i128 = 48;

/// A line of text, its newline included.
fn line() -> T {
    T::text(StrLen::Terminated { end: b'\n', or_end: true }, Encoding::Ascii)
}

/// The arrays the tree reads from this block, built from the tree.
fn arrays() -> T {
    let streamed = E::field("flags").and(E::lit(1));
    T::schema(KIND, vec![Step::field("tree")], vec![KeyPart::Int(E::idx()), KeyPart::Int(streamed)])
}

/// One chunk of asdf's lz4: how long the rest of it is, how long it comes to
/// unpacked, and an lz4 block. The unpacked length is part of what python-lz4
/// writes in front of a block, and the lz4 itself has no header.
fn lz4_chunk() -> T {
    T::structure(
        "Lz4Chunk",
        vec![
            ("length", T::u32(Big)),
            ("unpacked_size", T::u32(Little)),
            ("block", T::decoded(E::field("length").sub(E::lit(4)), Codec::Lz4Block, T::bytes(E::Remaining))),
        ],
    )
    .counted_as("chunk")
}

/// A block's data as it is stored, and what it is once unpacked.
fn data() -> T {
    let lz4 = T::structure(
        "Lz4Chunks",
        vec![
            ("chunks", T::repeat(lz4_chunk(), Until::End)),
            (
                "contents",
                T::stitched(
                    vec![Step::field("chunks"), Step::each(), Step::field("block")],
                    Some(E::field("unpacked_size")),
                    Some(E::field("data_size")),
                    arrays(),
                ),
            ),
        ],
    )
    .contents("contents");
    T::switch(
        E::field("compression"),
        vec![
            (NONE, arrays()),
            (ZLIB, T::decoded(E::Remaining, Codec::Zlib, arrays())),
            (BZP2, T::decoded(E::Remaining, Codec::Bzip2, arrays())),
            (LZ4, lz4),
        ],
        T::bytes(E::Remaining),
    )
}

fn block() -> T {
    let streamed = E::field("flags").and(E::lit(1));
    T::structure(
        "Block",
        vec![
            ("magic", T::magic(BLOCK_MAGIC)),
            ("header_size", T::u16(Big)),
            ("flags", T::flags("BlockFlags", T::u32(Big), &[(0, "STREAMED")])),
            (
                "compression",
                T::enumeration_hex("Compression", T::u32(Big), &[(NONE, "none"), (ZLIB, "zlib"), (BZP2, "bzp2"), (LZ4, "lz4")]),
            ),
            ("allocated_size", T::u64(Big)),
            ("used_size", T::u64(Big)),
            ("data_size", T::u64(Big)),
            ("checksum", T::bytes(E::lit(16))),
            // A later version of the standard may write a longer header, and
            // what it added is skipped.
            (
                "header_rest",
                T::when(E::lit(HEADER_SIZE).less_than(E::field("header_size")), T::bytes(E::field("header_size").sub(E::lit(HEADER_SIZE)))),
            ),
            // A streamed block's sizes are all zero and its data runs to the
            // end of the file.
            ("data", T::switch(streamed, vec![(1, T::sized(E::Remaining, data()))], T::sized(E::field("used_size"), data()))),
            ("padding", T::bytes(E::field("allocated_size").sub(E::field("used_size")))),
        ],
    )
    .contents("data")
    .counted_as("block")
}

pub fn asdf() -> Template {
    let peek = || E::peek(8, Big);
    let tree_starts = peek().equal_to(E::lit(b'%' as i128)).either(peek().equal_to(E::lit(b'-' as i128)));
    let root = T::structure(
        "Asdf",
        vec![
            ("header", line()),
            ("comments", T::repeat(line(), Until::While(peek().equal_to(E::lit(b'#' as i128))))),
            // The tree is one YAML document and ends at the line `...`.
            ("tree", T::when(tree_starts, T::text(StrLen::Fixed(E::to_bytes(b"\n...\n").add(E::lit(5))), Encoding::Utf8))),
            // Room a writer left to rewrite the tree in place.
            ("padding", T::bytes(E::to_bytes(BLOCK_MAGIC))),
            ("blocks", T::repeat(T::Named("Block".into()), Until::While(E::peek(32, Big).equal_to(E::lit(BLOCK_WORD))))),
            // Where each block starts, as a YAML list. Optional, and a reader
            // that finds it wrong is to ignore it.
            ("block_index", T::when(E::lit(0).less_than(E::Remaining), T::text(StrLen::Fixed(E::Remaining), Encoding::Utf8))),
        ],
    );
    Template::new("asdf", root).with_type("Block", block()).with_schema(KIND, Arc::new(Blocks::default()))
}

#[cfg(test)]
mod tests;
