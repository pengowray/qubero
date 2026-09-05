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
//! JSON does. Nothing in the entry says which of those it is, so an entry's
//! bytes are read as whatever their own first bytes say.
//!
//! A file's bytes are metadata and then a payload, and the metadata is
//! optional. When they open with `--[[pod` there is a header: `--[[`, a POD
//! written as `pod` on its own or as `pod,key=value,...`, and `]]`. Writing it
//! as a Lua block comment is the whole trick. A `.info.pod` carries the
//! cartridge's `title` and `bbs_id` and when it was last written; a `.lua`
//! carries `pod_format="raw"`, saying its payload is bytes and not a POD; and
//! Lua skips the comment and runs the file either way.
//!
//! The payload is then read from its own first four bytes:
//!
//! - `lz4\0` is a POD that was compressed: the magic, a four-byte packed
//!   length, a four-byte unpacked length, and one raw LZ4 block holding the
//!   POD as text.
//! - `qoif` is a QOI image, which is what a cartridge label is.
//! - The PNG signature is a PNG.
//! - Anything else is the file's own bytes, which after a metadata header is
//!   the source of a `.lua` and is read as text.
//!
//! `b64:` is left out. `pod()` will encode a POD base64 so it survives being
//! pasted into a forum, and Picotron writes such PODs inside Lua source as
//! `unpod("b64:...")` rather than as a cartridge entry. Neither sample holds
//! one, and there is no `Codec` for base64 yet to open it with.
//!
//! ## pxu
//!
//! `pod()` takes flags saying how to encode a value: 0x1 pxu, which the manual
//! calls encoding userdata "in a compressed (RLE-style) form", 0x2 the LZ4 pass
//! above, and 0x4 base64 on top. pxu is not a container round a POD. It sits
//! *inside* the POD's text, where a `userdata()` value would otherwise be
//! written: a reader scans the text for `pxu\0` and swaps each run it finds for
//! the userdata that run decodes to. So a run is reached from a POD's text and
//! not from an entry. [`crate::codec::pxu`] decodes one, and no field here
//! lays one out yet; that module says why.
//!
//! Read out of `picotron_fs.py` of thisismypassport/shrinko8, which reads and
//! writes all of this, and checked against the two cartridges in the sample
//! collection, whose entries account for every byte. The `lz4\0` header is
//! confirmed by the manual as well: the embedded image it prints in section
//! 5.1 begins `unpod("b64:bHo0AC4AAABGAAAA`, which is `lz4\0`, 46 and 70.

use crate::codec::Codec;
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
            // The file as it was written, read as whatever its own first
            // bytes say it is.
            ("data", T::present_if(not(E::field("is_folder")), T::sized(E::field("size"), file()))),
        ],
    )
    .counted_as("entry")
}

/// One file: its metadata, when it has any, and then its payload.
///
/// `--[[pod` is the one thing that says a file has metadata at all, so the
/// switch is on those seven bytes read as a number without consuming them. A
/// file of six bytes or fewer cannot carry them, and is read as a payload
/// without asking.
fn file() -> T {
    let with_meta = T::inline_structure("PicotronFile", vec![("meta", meta()), ("payload", payload(text()))]);
    let tagged = T::switch(E::peek(META_BITS, Big), vec![(META_TAG, with_meta)], payload(T::bytes(E::Remaining)));
    T::switch(E::lit(6).less_than(E::Remaining), vec![(1, tagged)], payload(T::bytes(E::Remaining)))
}

/// `--[[pod`, and how wide a read of it is.
const META_TAG: i128 = 0x2d_2d_5b_5b_70_6f_64;
const META_BITS: u32 = 56;

/// The metadata: a Lua block comment holding a POD, which is the cartridge's
/// own record of the file. `pod` on its own says the payload is a POD and
/// nothing more is known about it, `pod,pod_format="raw"` says the payload is
/// bytes, and a `.info.pod` carries the cartridge's title, author and the rest.
///
/// The fields run to the first `]]` because an encoder makes sure there is no
/// other: shrinko8's `escape_meta` rewrites a `]]` inside the POD as `\93]`,
/// which is the same string to Lua and not a close bracket. They are left as
/// the text they are rather than taken apart, since the POD grammar is Lua's
/// table constructor and a reader can see `title="..."` as written.
fn meta() -> T {
    T::inline_structure(
        "PicotronMeta",
        vec![
            ("open", T::magic(b"--[[")),
            ("fields", T::text(StrLen::Fixed(E::to_bytes(META_CLOSE)), Encoding::Utf8)),
            // A file cut off before its close bracket measures to its own end,
            // which leaves nothing here for the bracket to be.
            ("close", T::present_if(E::lit(0).less_than(E::Remaining), T::magic(META_CLOSE))),
        ],
    )
}

const META_CLOSE: &[u8] = b"]]";

/// The bytes after the metadata, read as whatever their first four say.
///
/// `rest` is what to do with bytes that say nothing: the source of a `.lua`,
/// which follows a metadata header, is text, and a file with no header at all
/// is whatever format it is and stays bytes. A payload of nothing is a real
/// file, so the width is checked before the tag is read.
fn payload(rest: T) -> T {
    let tagged = T::switch(
        E::peek(32, Big),
        vec![(LZ4_TAG, lz4_pod()), (QOIF_TAG, super::qoi::image()), (PNG_TAG, super::png::image())],
        rest.clone(),
    );
    T::switch(E::lit(3).less_than(E::Remaining), vec![(1, tagged)], rest)
}

/// `lz4\0`, `qoif`, and the first four bytes of the PNG signature, as the
/// numbers a 32-bit big-endian read of them gives.
const LZ4_TAG: i128 = 0x6c_7a_34_00;
const QOIF_TAG: i128 = 0x71_6f_69_66;
const PNG_TAG: i128 = 0x89_50_4e_47;

/// A POD compressed with LZ4: the magic, how long the block is, how long what
/// comes out of it is, and the block.
///
/// Both lengths are written because an LZ4 block says neither: a decoder has
/// to be told how much to read and how much to expect. What comes out is the
/// POD as text, which is what `pod()` produced before the compression pass ran
/// over it.
fn lz4_pod() -> T {
    T::structure(
        "PicotronLz4Pod",
        vec![
            ("magic", T::magic(b"lz4\0")),
            ("packed_size", T::u32(Little)),
            ("unpacked_size", T::u32(Little)),
            ("pod", T::decoded(E::field("packed_size"), Codec::Lz4Block, text())),
        ],
    )
}

/// Everything left, as text.
fn text() -> T {
    T::text(StrLen::Fixed(E::Remaining), Encoding::Utf8)
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

    /// A cartridge of one file per kind an entry can hold: a POD written as
    /// text, a Lua source file with a metadata header in front of it, a POD
    /// compressed with LZ4, a QOI image, a file too short to carry a header
    /// and a file of nothing at all.
    pub(crate) fn pods() -> Vec<u8> {
        // A QOI of no pixels: the header, no chunks, and the end marker.
        let mut qoi = b"qoif".to_vec();
        qoi.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 1, 4, 0]);
        qoi.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1]);

        let text = b"{ 1, 2, 3 }";
        let block = lz4_literals(text);
        let mut lz4 = b"lz4\0".to_vec();
        lz4.extend_from_slice(&(block.len() as u32).to_le_bytes());
        lz4.extend_from_slice(&(text.len() as u32).to_le_bytes());
        lz4.extend_from_slice(&block);

        let mut cart = entry_bytes(".info.pod", Some(b"--[[pod,title=\"a\"]]"));
        cart.extend_from_slice(&entry_bytes("main.lua", Some(b"--[[pod_format=\"raw\"]]print(1)")));
        cart.extend_from_slice(&entry_bytes("gfx/0.gfx", Some(&lz4)));
        cart.extend_from_slice(&entry_bytes("label.qoi", Some(&qoi)));
        cart.extend_from_slice(&entry_bytes("short", Some(b"ab")));
        cart.extend_from_slice(&entry_bytes("empty", Some(b"")));
        wrap(&lz4_literals(&cart))
    }

    /// A cartridge of a folder, a short file and a file long enough to need
    /// the four-byte length.
    pub(crate) fn rom() -> Vec<u8> {
        let long = vec![b'x'; 300];
        let mut cart = entry_bytes("gfx/", None);
        cart.extend_from_slice(&entry_bytes(".info.pod", Some(b"--[[pod,revision=3]]")));
        cart.extend_from_slice(&entry_bytes("gfx/0.gfx", Some(&long)));
        wrap(&lz4_literals(&cart))
    }

    /// The eight bytes of ROM header, and the block they measure.
    fn wrap(block: &[u8]) -> Vec<u8> {
        let mut v = super::super::picotron::MAGIC.to_vec();
        v.push(2);
        v.extend_from_slice(&(block.len() as u32).to_le_bytes());
        v.extend_from_slice(block);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::super::picotron::p64rom;
    use super::sample::{self, rom};
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

    /// The metadata is a header and the payload is what follows it, and both
    /// of them read as what they are: the POD's own text, and the Lua source
    /// the header said was raw.
    #[test]
    fn a_file_opening_with_the_pod_comment_shows_its_fields_and_then_its_body() {
        let d = Document::new(MemSource(sample::pods()));
        let mut ev = Evaluator::new(p64rom());
        // entry 0, data, meta, fields.
        let fields = ev.node(&d, &[3, 0, 0, 0, 6, 0, 1]).unwrap();
        assert_eq!(fields.value, Value::Str("pod,title=\"a\"".into()));
        // Nothing after the bracket: a `.info.pod` is its metadata.
        assert_eq!(ev.node(&d, &[3, 0, 0, 0, 6, 1]).unwrap().value, Value::Str("".into()));

        let fields = ev.node(&d, &[3, 0, 0, 1, 6, 0, 1]).unwrap();
        assert_eq!(fields.value, Value::Str("pod_format=\"raw\"".into()));
        assert_eq!(ev.node(&d, &[3, 0, 0, 1, 6, 1]).unwrap().value, Value::Str("print(1)".into()));
    }

    /// A payload of `lz4\0` is a POD that was compressed, and what comes out
    /// of the block is the POD as it was written.
    #[test]
    fn a_compressed_pod_opens_to_its_text() {
        let d = Document::new(MemSource(sample::pods()));
        let mut ev = Evaluator::new(p64rom());
        let node = ev.node(&d, &[3, 0, 0, 2, 6]).unwrap();
        assert_eq!(node.type_name, "PicotronLz4Pod");
        assert_eq!(ev.node(&d, &[3, 0, 0, 2, 6, 2]).unwrap().value, Value::UInt(11));
        // The block, and the text inside it.
        assert_eq!(ev.node(&d, &[3, 0, 0, 2, 6, 3]).unwrap().type_name, "lz4");
        assert_eq!(ev.node(&d, &[3, 0, 0, 2, 6, 3, 0]).unwrap().value, Value::Str("{ 1, 2, 3 }".into()));
    }

    /// A cartridge label is a QOI, and it is read as one rather than left as
    /// the bytes of an image.
    #[test]
    fn a_label_is_read_as_the_image_it_is() {
        let d = Document::new(MemSource(sample::pods()));
        let mut ev = Evaluator::new(p64rom());
        assert_eq!(ev.node(&d, &[3, 0, 0, 3, 6]).unwrap().type_name, "QOI");
        assert_eq!(ev.node(&d, &[3, 0, 0, 3, 6, 1]).unwrap().value, Value::UInt(1));
    }

    /// A file too short to hold `--[[pod` or a four-byte tag is still a file,
    /// and reading its first bytes must not run past its end.
    #[test]
    fn a_file_shorter_than_any_tag_is_left_as_the_bytes_it_is() {
        let d = Document::new(MemSource(sample::pods()));
        let mut ev = Evaluator::new(p64rom());
        assert_eq!(ev.node(&d, &[3, 0, 0, 4, 6]).unwrap().size_bits, 2 * 8);
        assert_eq!(ev.node(&d, &[3, 0, 0, 5, 6]).unwrap().size_bits, 0);
    }

    #[test]
    fn a_name_with_a_slash_in_the_middle_is_still_a_file() {
        let d = Document::new(MemSource(rom()));
        let mut ev = Evaluator::new(p64rom());
        assert_eq!(ev.node(&d, &[3, 0, 0, 2, 0]).unwrap().value, Value::Int(0));
    }
}
