//! Microsoft Cabinet archives: the container Windows installers, driver
//! packages and `.msu` updates are made of.
//!
//! A cabinet is three tables and a heap. Folders say where a run of compressed
//! blocks starts and how the blocks were compressed; files say how big they
//! are and where in a folder's decompressed stream they begin; blocks are the
//! stream itself, cut into pieces of at most 32 KiB so that a reader can start
//! part way through one. So a file's bytes are not in one place and are not
//! anywhere until the folder holding it has been decompressed: the offset in a
//! `CabFile` counts in a stream this template does not produce.
//!
//! The header is the awkward part, and it is awkward in a way the IR already
//! had an answer for: three of its fields are there only when a flag says so,
//! and two more are strings that are there only when a different flag says so.
//! A field that is not there is not the same as a field holding zero, but the
//! sizes it would have carried are what everything below measures its reserved
//! space by, so those three read as a computed zero rather than as nothing:
//! the format says the reserve is empty when the flag is clear, and that is a
//! number the folders and the blocks can be sized against.
//!
//! What is not read here: the compressed bytes. MSZIP is deflate with two
//! bytes in front, LZX and Quantum are neither, and all three would have to be
//! decompressed before a file in them is anything but an offset.

use crate::template::{Endian::Little, Expr as E, Template, Ty as T};

/// What a cabinet starts with.
pub const MAGIC: &[u8] = b"MSCF";

pub fn cab() -> Template {
    Template::new(
        "cab",
        T::structure(
            "CabFile",
            vec![
                ("magic", T::magic(MAGIC)),
                ("reserved1", T::u32(Little)),
                // How long the cabinet says it is, which is what a reader
                // checks a file against before it trusts anything else in it.
                ("cabinet_size", T::u32(Little)),
                ("reserved2", T::u32(Little)),
                ("files_offset", T::u32(Little)),
                ("reserved3", T::u32(Little)),
                ("version_minor", T::u8()),
                ("version_major", T::u8()),
                ("folder_count", T::u16(Little)),
                ("file_count", T::u16(Little)),
                ("flags", T::flags("CabFlags", T::u16(Little), FLAGS)),
                // Which multi-cabinet set this is part of, and which member of
                // it: a file split across three disks writes the same set id
                // in all three.
                ("set_id", T::u16(Little)),
                ("cabinet_index", T::u16(Little)),
                // Room a program using the cabinet library asked for, in the
                // header, in every folder and in every block. Absent unless
                // the flag says otherwise, and zero when absent, which is what
                // the fields below have to measure against.
                ("header_reserve_size", reserved_if(2, T::u16(Little))),
                ("folder_reserve_size", reserved_if(2, T::u8())),
                ("block_reserve_size", reserved_if(2, T::u8())),
                ("header_reserve", T::bytes(E::field("header_reserve_size"))),
                // The cabinet before this one and the disk it is on, then the
                // same for the one after.
                ("previous_cabinet", string_if(0)),
                ("previous_disk", string_if(0)),
                ("next_cabinet", string_if(1)),
                ("next_disk", string_if(1)),
                ("folders", T::array(folder(), E::field("folder_count"))),
                ("files", T::at(E::field("files_offset"), T::array(file(), E::field("file_count")))),
                // Each folder's blocks, at the offset the folder holds. They
                // are declared last because that is what a list placed by
                // offsets reads to: the stretch the offsets point into.
                (
                    "blocks",
                    T::pointer_list_sized(
                        "folders",
                        &["first_block"],
                        crate::template::Anchor::File,
                        E::lit(0),
                        T::array(block(), E::elem_field("folders", E::idx(), &["block_count"])),
                    ),
                ),
            ],
        ),
    )
}

/// A field that is only written when bit `bit` of the flags is set, and whose
/// value is zero when it is not. A reserve size is a number either way: the
/// format says an absent one is empty, and the fields measured against it need
/// that answer rather than an error.
fn reserved_if(bit: u32, ty: T) -> T {
    T::switch(E::field("flags").bit(bit), vec![(1, ty)], T::computed(E::lit(0)))
}

/// A string that is only written when bit `bit` of the flags is set. Nothing
/// measures itself against these, so an absent one is no bytes rather than an
/// empty string nobody wrote.
fn string_if(bit: u32) -> T {
    T::switch(E::field("flags").bit(bit), vec![(1, T::cstr())], T::bytes(E::lit(0)))
}

/// A folder: where its blocks start, how many there are, and what was done to
/// them. The compression is one field in the file and two here, because the
/// low byte says which compressor and the high byte is that compressor's own
/// business: LZX writes the window it used, and Quantum its level.
fn folder() -> T {
    T::inline_structure(
        "CabFolder",
        vec![
            ("first_block", T::u32(Little)),
            ("block_count", T::u16(Little)),
            ("compression", T::enumeration("CabCompression", T::u8(), COMPRESSION)),
            ("compression_parameter", T::u8()),
            ("reserve", T::bytes(E::field("folder_reserve_size"))),
        ],
    )
    .counted_as("folder")
}

/// A file: how big it is once unpacked, where it starts in its folder's
/// decompressed stream, and which folder that is. The three folder numbers at
/// the top of the range are not folders: they say the file carries on from the
/// cabinet before this one, into the one after, or both.
fn file() -> T {
    T::structure_named(
        "CabEntry",
        "name",
        "",
        vec![
            ("size", T::u32(Little)),
            ("folder_offset", T::u32(Little)),
            (
                "folder_index",
                T::enumeration(
                    "CabFolderIndex",
                    T::u16(Little),
                    &[
                        (0xfffd, "continued from previous"),
                        (0xfffe, "continued to next"),
                        (0xffff, "continued from previous and to next"),
                    ],
                ),
            ),
            // MS-DOS date and time, which is what a format from 1994 keeps.
            ("date", T::u16(Little)),
            ("time", T::u16(Little)),
            ("attributes", T::flags("CabAttributes", T::u16(Little), ATTRIBUTES)),
            // The last attribute bit says which of two readings the name has:
            // UTF-8, or the code page of whichever machine wrote it.
            ("name", T::cstr()),
        ],
    )
    .counted_as("file")
}

/// One block of a folder's stream: at most 32 KiB once unpacked, so that a
/// reader after one file does not have to decompress the whole folder.
fn block() -> T {
    T::structure(
        "CabBlock",
        vec![
            ("checksum", T::u32(Little)),
            ("compressed_size", T::u16(Little)),
            ("uncompressed_size", T::u16(Little)),
            ("reserve", T::bytes(E::field("block_reserve_size"))),
            ("data", T::bytes(E::field("compressed_size"))),
        ],
    )
}

const FLAGS: &[(u32, &str)] = &[(0, "previous cabinet"), (1, "next cabinet"), (2, "reserve present")];

const COMPRESSION: &[(i128, &str)] = &[(0, "none"), (1, "mszip"), (2, "quantum"), (3, "lzx")];

const ATTRIBUTES: &[(u32, &str)] =
    &[(0, "read-only"), (1, "hidden"), (2, "system"), (5, "archive"), (6, "run after extraction"), (7, "name is utf-8")];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::Document,
        eval::{Evaluator, Value},
        source::MemSource,
    };

    /// A cabinet of one folder and two files, with as much reserved space as
    /// the caller asks for: the header's, the folders' and the blocks'.
    fn cabinet(reserve: Option<(&[u8], usize, usize)>) -> Vec<u8> {
        let mut files = Vec::new();
        for (name, size) in [("readme.txt", 11u32), ("setup.exe", 4096)] {
            files.extend_from_slice(&size.to_le_bytes());
            files.extend_from_slice(&0u32.to_le_bytes());
            files.extend_from_slice(&0u16.to_le_bytes());
            files.extend_from_slice(&0x5a21u16.to_le_bytes());
            files.extend_from_slice(&0x4800u16.to_le_bytes());
            files.extend_from_slice(&0x20u16.to_le_bytes());
            files.extend_from_slice(name.as_bytes());
            files.push(0);
        }
        let (header_reserve, folder_reserve, block_reserve) = reserve.unwrap_or((&[], 0, 0));
        let header_len = 36 + if reserve.is_some() { 4 + header_reserve.len() } else { 0 };
        let files_at = header_len + 8 + folder_reserve;
        let blocks_at = files_at + files.len();

        let mut v = MAGIC.to_vec();
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes()); // filled in below
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&(files_at as u32).to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&[3, 1]);
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        v.extend_from_slice(&(if reserve.is_some() { 4u16 } else { 0 }).to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        if reserve.is_some() {
            v.extend_from_slice(&(header_reserve.len() as u16).to_le_bytes());
            v.push(folder_reserve as u8);
            v.push(block_reserve as u8);
            v.extend_from_slice(header_reserve);
        }
        assert_eq!(v.len(), header_len);
        v.extend_from_slice(&(blocks_at as u32).to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes()); // mszip, no parameter
        v.extend_from_slice(&vec![0u8; folder_reserve]);
        v.extend_from_slice(&files);
        v.extend_from_slice(&0x1234_5678u32.to_le_bytes());
        v.extend_from_slice(&6u16.to_le_bytes());
        v.extend_from_slice(&4107u16.to_le_bytes());
        v.extend_from_slice(&vec![0u8; block_reserve]);
        v.extend_from_slice(b"CK\x01\x02\x03\x04");
        let size = v.len() as u32;
        v[8..12].copy_from_slice(&size.to_le_bytes());
        v
    }

    #[test]
    fn a_cabinet_places_its_files_and_its_blocks_from_the_header() {
        let d = Document::new(MemSource(cabinet(None)));
        let mut e = Evaluator::new(cab());
        // One folder, compressed with MSZIP.
        assert_eq!(e.node(&d, &[21]).unwrap().child_count, 1);
        assert_eq!(e.node(&d, &[21, 0, 2]).unwrap().value.as_int(), Some(1));
        // Two files, named by the string at the end of each entry.
        assert_eq!(e.node(&d, &[22, 0]).unwrap().child_count, 2);
        assert_eq!(e.node(&d, &[22, 0, 1, 6]).unwrap().value, Value::Str("setup.exe".into()));
        assert_eq!(e.node(&d, &[22, 0, 1, 0]).unwrap().value.as_int(), Some(4096));
        // One folder's worth of blocks, and the block is as long as it says.
        assert_eq!(e.node(&d, &[23]).unwrap().child_count, 1);
        assert_eq!(e.node(&d, &[23, 0]).unwrap().child_count, 1);
        assert_eq!(e.node(&d, &[23, 0, 0, 4]).unwrap().size_bits, 6 * 8);
    }

    /// With the flag clear the three reserve sizes are not written, and every
    /// field measured against them still measures: an absent size is the zero
    /// the format says it is, not an error.
    #[test]
    fn reserved_space_nobody_asked_for_takes_up_nothing() {
        let d = Document::new(MemSource(cabinet(None)));
        let mut e = Evaluator::new(cab());
        assert_eq!(e.node(&d, &[13]).unwrap().size_bits, 0);
        assert_eq!(e.node(&d, &[13]).unwrap().value.as_int(), Some(0));
        assert_eq!(e.node(&d, &[21, 0]).unwrap().size_bits, 8 * 8);
    }

    /// With it set, everything after the header moves by as much room as the
    /// header, the folders and the blocks each reserved.
    #[test]
    fn reserved_space_moves_everything_after_it() {
        let d = Document::new(MemSource(cabinet(Some((b"cabinet-signature", 2, 3)))));
        let mut e = Evaluator::new(cab());
        assert_eq!(e.node(&d, &[13]).unwrap().value.as_int(), Some(17));
        assert_eq!(e.node(&d, &[21]).unwrap().size_bits, 10 * 8);
        assert_eq!(e.node(&d, &[22, 0, 0, 6]).unwrap().value, Value::Str("readme.txt".into()));
        assert_eq!(e.node(&d, &[23, 0, 0, 3]).unwrap().size_bits, 3 * 8);
        assert_eq!(e.node(&d, &[23, 0, 0, 4]).unwrap().size_bits, 6 * 8);
    }
}
