//! What a Picotron cartridge holds: the entry list inside the LZ4 block, and
//! what one entry's bytes turn out to be.
//!
//! The container the entries arrive in is [`super::picotron`]; this is
//! everything from the first path onwards. Split out because the two are
//! different subjects. The container is a header, an LZ4 block and, in the
//! image form, a picture; an entry is a path, a length and a file, and the
//! file is a format of its own.
//!
//! ## The entry list
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
//! ## What an entry holds
//!
//! Picotron does not agree with itself about what a file is. A cartridge holds
//! plain files of any format, a QOI image for the label the browser shows, a
//! PNG, JavaScript in an exporter. It also holds PODs, "Picotron Object Data",
//! which the manual describes as a string that encodes a Lua value the way
//! JSON does. A text POD opens with `--[[pod` and closes the bracket after its
//! metadata, which is a Lua comment and so doubles as both a header and
//! something Lua will ignore. None of that is length prefixed or flagged in
//! the entry, so this stops at the bytes and lets a reader see the prelude in
//! them.

use crate::template::{Encoding, Endian::{Big, Little}, Expr as E, StrLen, Until, Ty as T};

/// A one-byte length of 255 is not a length. It says a four-byte one follows.
pub(super) const LONG_SIZE: i128 = 255;

/// The cartridge as it comes out of the block: entries to the end, with no
/// count anywhere and nothing between them.
pub(super) fn cart() -> T {
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

/// The bytes of a cartridge to read the entry list against, shared with the
/// container's own tests.
#[cfg(test)]
pub(super) mod sample {
    use super::LONG_SIZE;

    /// One raw LZ4 block holding `data` and nothing but literals, which is
    /// what an encoder writes when it finds no match and is all a decoder
    /// needs to be given here.
    pub(crate) fn lz4_literals(data: &[u8]) -> Vec<u8> {
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
    pub(crate) fn entry_bytes(name: &str, body: Option<&[u8]>) -> Vec<u8> {
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
    pub(crate) fn rom() -> Vec<u8> {
        let long = vec![b'x'; 300];
        let mut cart = entry_bytes("gfx/", None);
        cart.extend_from_slice(&entry_bytes(".info.pod", Some(b"--[[pod,revision=3]]")));
        cart.extend_from_slice(&entry_bytes("gfx/0.gfx", Some(&long)));
        let block = lz4_literals(&cart);

        let mut v = super::super::picotron::MAGIC.to_vec();
        v.push(2);
        v.extend_from_slice(&(block.len() as u32).to_le_bytes());
        v.extend_from_slice(&block);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::super::picotron::p64rom;
    use super::sample::rom;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    #[test]
    fn a_folder_is_a_path_ending_in_a_slash_and_nothing_else() {
        let d = Document::new(MemSource(rom()));
        let mut ev = Evaluator::new(p64rom());
        // cart -> PicotronCart -> entries -> entry 0.
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
        assert_eq!(ev.node(&d, &[3, 0, 0, 2, 2]).unwrap().value, Value::UInt(super::LONG_SIZE as u128));
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
