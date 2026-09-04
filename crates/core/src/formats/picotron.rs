//! Picotron cartridge ROM: eight bytes of header and then the whole cartridge
//! filesystem as one LZ4 block.
//!
//! A Picotron cartridge is a folder rather than a file. The manual says so
//! plainly: "Cartridge files (.p64, .p64.png) in Picotron are logically
//! folders", and the shell copies things out of one with `cp`, so what a
//! cartridge holds is a tree of paths, each with bytes under it. The `.p64`
//! form writes that tree as a text file and the `.p64.png` form carries it in
//! an image. The `.rom` form is the tree on its own, with nothing wrapped
//! around it, which is what the exporters carry and what this reads.
//!
//! The header is the three letters `p64`, a version byte, and a four-byte
//! little-endian count of the bytes after it. Then that many bytes of one raw
//! LZ4 block: not the LZ4 frame format, which has its own magic and its own
//! block headers, but the bare sequence of literals and matches. The count is
//! the compressed length, and it came to the file length less eight in every
//! sample read, so a file that disagrees has been cut short.
//!
//! Inside the block the entries follow one another with nothing between them.
//! An entry is a path, NUL terminated, and then how long its bytes are and the
//! bytes. The length is written small when it can be: one byte holds a length
//! up to 254, and 255 is not a length but an escape saying a four-byte
//! little-endian one follows. A path ending in `/` is a folder, and a folder
//! has no length and no bytes at all, so the next path starts straight after
//! the NUL. Paths are written whole, `system/apps/procman.lua` rather than a
//! name under a folder that came before, and the folders are listed anyway.
//!
//! What is in the bytes is the file, and Picotron does not agree with itself
//! about what a file is. A cartridge holds plain files of any format, a QOI
//! image for the label the browser shows, a PNG, JavaScript in an exporter.
//! It also holds PODs, "Picotron Object Data", which the manual describes as
//! a string that encodes a Lua value the way JSON does. A text POD opens with
//! `--[[pod` and closes the bracket after its metadata, which is a Lua comment
//! and so doubles as both a header and something Lua will ignore: the cartridge
//! metadata in `.info.pod` is written that way, and so is the `pod_format="raw"`
//! that marks a Lua source file. None of that is length prefixed or flagged in
//! the entry, so this template stops at the bytes and lets a reader see the
//! prelude in them.
//!
//! Nothing here is pxu. `pod()` takes flags saying how to encode a value:
//! 0x1 pxu, which the manual calls encoding userdata "in a compressed
//! (RLE-style) form", 0x2 an LZ4 pass, and 0x4 base64 on top for a POD that has
//! to survive being pasted into a forum. A binary POD compressed that way opens
//! with `lz4\0`. That is a property of one file's bytes, not of the cartridge,
//! and no sample read had one, so there is no compression field in an entry to
//! name. When pxu is written as a `Codec` it belongs on a file payload that
//! opens with those bytes, in the space this template already decodes into.
//!
//! Read against the Picotron manual and its POD and filesystem pages at
//! lexaloffle.com/dl/docs/, and worked out byte for byte from the 28 `.p64.rom`
//! files committed to github.com/akd-io/picotron, which parse to their last
//! byte under the rules above. Certain, because every one of those files is
//! accounted for: the header, the LZ4 block, the entry encoding, the escape at
//! 255 and the trailing slash. Inferred: that the byte after `p64` is a version
//! and not a flag, since every sample holds 2 and none holds anything else;
//! that 255 is the only escape, since no sample needed another; that a path is
//! UTF-8, since none of them left ASCII; and that a `.p64.png` carries this
//! same ROM, which follows from the manual putting a limit on one in "ROM
//! data" but was not read out of an image here.

use crate::codec::Codec;
use crate::template::{Encoding, Endian::{Big, Little}, Expr as E, StrLen, Template, Until, Ty as T};

/// What one of these starts with. Three lower-case letters, which is far too
/// little on its own, so `recognise` weighs the version and the length as well.
pub const MAGIC: &[u8] = b"p64";

/// The bytes of header before the compressed block.
pub const HEADER_LEN: u64 = 8;

/// A one-byte length of 255 is not a length. It says a four-byte one follows.
const LONG_SIZE: i128 = 255;

pub fn p64rom() -> Template {
    Template::new(
        "p64rom",
        T::structure(
            "P64Rom",
            vec![
                ("magic", T::magic(MAGIC)),
                // 2 in everything read so far.
                ("version", T::u8()),
                // Of the block below, so a short file shows as one.
                ("payload_size", T::u32(Little)),
                ("cart", T::decoded(E::field("payload_size"), Codec::Lz4Block, cart())),
            ],
        ),
    )
}

/// The cartridge as it comes out of the block: entries to the end, with no
/// count anywhere and nothing between them.
fn cart() -> T {
    T::structure("PicotronCart", vec![("entries", T::repeat(entry(), Until::End))])
}

/// One path and, unless it is a folder, its bytes.
fn entry() -> T {
    T::structure_named(
        "PicotronEntry",
        "name",
        "",
        vec![
            // A folder is a path ending in `/` and nothing else, so what the
            // rest of the entry is rests on the last character of a name
            // whose length is not written down. Reach it by measuring to the
            // NUL that ends the name and stepping back one byte, then read
            // that byte without consuming it: 1 when it is `/`, which is 47,
            // and 0 for every other character.
            ("is_folder", T::computed(is_folder())),
            ("name", T::text(StrLen::Terminated { end: 0, or_end: true }, Encoding::Utf8)),
            ("size_tag", T::present_if(not(E::field("is_folder")), T::u8())),
            ("size_is_long", T::computed(E::lit(LONG_SIZE - 1).less_than(E::field("size_tag")))),
            ("size_long", T::present_if(E::field("size_is_long"), T::u32(Little))),
            // One or the other: the field that is not there reads as nothing.
            (
                "size",
                T::computed(E::field("size_long").add(E::field("size_tag").mul(not(E::field("size_is_long"))))),
            ),
            // The file as it was written. A Lua source file or a `.info.pod`
            // opens with a `--[[pod` comment holding its metadata; anything
            // else is whatever format it is.
            ("data", T::present_if(not(E::field("is_folder")), T::bytes(E::field("size")))),
        ],
    )
    .counted_as("entry")
}

/// Whether the name starting here ends in a `/`.
fn is_folder() -> E {
    let last = E::peek_at(E::Find { needle: vec![0], last: false }.sub(E::lit(1)).mul(E::lit(8)), 8, Big);
    // Equality, out of the one comparison there is: below 48 and not below 47.
    last.clone().less_than(E::lit(48)).sub(last.less_than(E::lit(47)))
}

/// The other way round, for a number that is one or zero.
fn not(e: E) -> E {
    E::lit(1).sub(e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// One raw LZ4 block holding `data` and nothing but literals, which is
    /// what an encoder writes when it finds no match and is all a decoder
    /// needs to be given here.
    fn lz4_literals(data: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        if data.len() < 15 {
            v.push((data.len() as u8) << 4);
        } else {
            v.push(0xf0);
            let mut left = data.len() - 15;
            while left >= 255 {
                v.push(255);
                left -= 255;
            }
            v.push(left as u8);
        }
        v.extend_from_slice(data);
        v
    }

    /// An entry: the path, then its length written the short way or the long
    /// way, then the bytes. A path ending in `/` is written on its own.
    fn entry_bytes(name: &str, body: Option<&[u8]>) -> Vec<u8> {
        let mut v = name.as_bytes().to_vec();
        v.push(0);
        if let Some(body) = body {
            if body.len() < LONG_SIZE as usize {
                v.push(body.len() as u8);
            } else {
                v.push(LONG_SIZE as u8);
                v.extend_from_slice(&(body.len() as u32).to_le_bytes());
            }
            v.extend_from_slice(body);
        }
        v
    }

    /// A cartridge of a folder, a short file and a file long enough to need
    /// the four-byte length.
    fn rom() -> Vec<u8> {
        let long = vec![b'x'; 300];
        let mut cart = entry_bytes("gfx/", None);
        cart.extend_from_slice(&entry_bytes(".info.pod", Some(b"--[[pod,revision=3]]")));
        cart.extend_from_slice(&entry_bytes("gfx/0.gfx", Some(&long)));
        let block = lz4_literals(&cart);

        let mut v = MAGIC.to_vec();
        v.push(2);
        v.extend_from_slice(&(block.len() as u32).to_le_bytes());
        v.extend_from_slice(&block);
        v
    }

    #[test]
    fn the_header_counts_the_compressed_bytes_after_it() {
        let d = Document::new(MemSource(rom()));
        let mut ev = Evaluator::new(p64rom());
        assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::UInt(2));
        let size = match ev.node(&d, &[2]).unwrap().value {
            Value::UInt(n) => n as u64,
            other => panic!("payload_size read as {other:?}"),
        };
        assert_eq!(size, rom().len() as u64 - HEADER_LEN);
    }

    #[test]
    fn a_folder_is_a_path_ending_in_a_slash_and_nothing_else() {
        let d = Document::new(MemSource(rom()));
        let mut ev = Evaluator::new(p64rom());
        // cart -> PicotronCart -> files -> entry 0.
        assert_eq!(ev.node(&d, &[3, 0, 0, 0, 0]).unwrap().value, Value::Int(1));
        assert_eq!(ev.node(&d, &[3, 0, 0, 0, 1]).unwrap().value, Value::Str("gfx/".into()));
        // No length and no bytes, so the whole entry is the path and its NUL.
        assert_eq!(ev.node(&d, &[3, 0, 0, 0]).unwrap().size_bits, 5 * 8);
    }

    #[test]
    fn a_short_length_is_one_byte_and_a_long_one_escapes_to_four() {
        let d = Document::new(MemSource(rom()));
        let mut ev = Evaluator::new(p64rom());

        // The short file: the tag is the length.
        assert_eq!(ev.node(&d, &[3, 0, 0, 1, 1]).unwrap().value, Value::Str(".info.pod".into()));
        assert_eq!(ev.node(&d, &[3, 0, 0, 1, 2]).unwrap().value, Value::UInt(20));
        assert_eq!(ev.node(&d, &[3, 0, 0, 1, 3]).unwrap().value, Value::Int(0));
        assert_eq!(ev.node(&d, &[3, 0, 0, 1, 5]).unwrap().value, Value::Int(20));
        assert_eq!(ev.node(&d, &[3, 0, 0, 1, 6]).unwrap().size_bits, 20 * 8);

        // The long one: a tag of 255 and four bytes after it.
        assert_eq!(ev.node(&d, &[3, 0, 0, 2, 1]).unwrap().value, Value::Str("gfx/0.gfx".into()));
        assert_eq!(ev.node(&d, &[3, 0, 0, 2, 2]).unwrap().value, Value::UInt(LONG_SIZE as u128));
        assert_eq!(ev.node(&d, &[3, 0, 0, 2, 3]).unwrap().value, Value::Int(1));
        assert_eq!(ev.node(&d, &[3, 0, 0, 2, 4]).unwrap().value, Value::UInt(300));
        assert_eq!(ev.node(&d, &[3, 0, 0, 2, 5]).unwrap().value, Value::Int(300));
        assert_eq!(ev.node(&d, &[3, 0, 0, 2, 6]).unwrap().size_bits, 300 * 8);
    }

    #[test]
    fn a_name_with_a_slash_in_the_middle_is_still_a_file() {
        let d = Document::new(MemSource(rom()));
        let mut ev = Evaluator::new(p64rom());
        assert_eq!(ev.node(&d, &[3, 0, 0, 2, 0]).unwrap().value, Value::Int(0));
    }
}
