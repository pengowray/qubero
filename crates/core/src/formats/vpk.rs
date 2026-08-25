//! Valve VPK: a directory of three nested lists, each ended by an empty
//! string.
//!
//! The tree groups files by extension, then by folder, then by name, and each
//! of the three levels runs until it hits a NUL where a string should be. That
//! is a list whose end is a field value, and the IR says it already: a repeat
//! ending at an element whose name field holds a single zero byte.
//!
//! The bytes of the files themselves are usually not here. `archive_index`
//! names a numbered `_000.vpk` sitting beside this one, and only an index of
//! 0x7fff means the bytes are in this file, after the tree. What is here for
//! every entry either way is a few preload bytes, which is what let the engine
//! start reading a file before opening the archive holding the rest of it.

use crate::template::{Endian::*, Expr as E, Template, Ty as T, Until};

pub fn vpk() -> Template {
    Template::new(
        "vpk",
        T::structure(
            "VPK",
            vec![
                ("magic", T::magic(b"\x34\x12\xaa\x55")),
                ("version", T::u32(Little)),
                ("tree_size", T::u32(Little)),
                // Version 2 adds four more sizes and a signature section.
                ("v2", T::switch(E::field("version"), vec![(2, v2_header())], T::bytes(E::lit(0)))),
                ("tree", T::sized(E::field("tree_size"), tree())),
                ("data", T::bytes(E::Remaining)),
            ],
        ),
    )
}

/// What version 2 puts between the header and the tree.
fn v2_header() -> T {
    T::structure(
        "V2Header",
        vec![
            ("file_data_size", T::u32(Little)),
            ("archive_md5_size", T::u32(Little)),
            ("other_md5_size", T::u32(Little)),
            ("signature_size", T::u32(Little)),
        ],
    )
}

/// Extensions, then folders inside each, then files inside each of those.
/// Every level ends at an empty string, which is one NUL byte.
///
/// The element that ends a list is still an element, and it holds nothing but
/// that NUL: reading the rest of the record after it would run off the end of
/// the tree. So each level switches on the text of its own name, and an empty
/// one has no body at all. It has to be the switch on text rather than the one
/// on numbers: a path here is longer than any number a field can be read as.
fn tree() -> T {
    let ended = |field: &str| Until::FieldBytes { field: field.into(), bytes: vec![0] };
    let empty = || T::bytes(E::lit(0));

    let entry = T::structure(
        "Entry",
        vec![
            ("crc", T::u32(Little)),
            ("preload_bytes", T::u16(Little)),
            // 0x7fff means the bytes are in this file rather than a numbered
            // archive beside it.
            ("archive_index", T::u16(Little)),
            ("entry_offset", T::u32(Little)),
            ("entry_length", T::u32(Little)),
            ("terminator", T::u16(Little)),
            ("preload", T::bytes(E::field("preload_bytes"))),
        ],
    );
    let file = T::structure_named(
        "File",
        "name",
        "entry",
        vec![("name", T::cstr()), ("entry", T::matches(E::field("name"), vec![("", empty())], entry))],
    )
    .counted_as("file");

    let folder = T::structure_named(
        "Folder",
        "path",
        "files",
        vec![
            ("path", T::cstr()),
            ("files", T::matches(E::field("path"), vec![("", empty())], T::repeat(file, ended("name")))),
        ],
    )
    .counted_as("folder");

    let extension = T::structure_named(
        "Extension",
        "extension",
        "folders",
        vec![
            ("extension", T::cstr()),
            ("folders", T::matches(E::field("extension"), vec![("", empty())], T::repeat(folder, ended("path")))),
        ],
    )
    .counted_as("extension");

    T::repeat(extension, ended("extension"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn entry(name: &str, archive: u16, offset: u32, length: u32) -> Vec<u8> {
        let mut v = name.as_bytes().to_vec();
        v.push(0);
        v.extend_from_slice(&0u32.to_le_bytes()); // crc
        v.extend_from_slice(&0u16.to_le_bytes()); // no preload
        v.extend_from_slice(&archive.to_le_bytes());
        v.extend_from_slice(&offset.to_le_bytes());
        v.extend_from_slice(&length.to_le_bytes());
        v.extend_from_slice(&0xffffu16.to_le_bytes());
        v
    }

    fn vpk_bytes() -> Vec<u8> {
        let mut tree = b"vmt\0".to_vec();
        tree.extend_from_slice(b"materials/models\0");
        tree.extend_from_slice(&entry("gun", 1, 0, 512));
        tree.extend_from_slice(&entry("crate", 1, 512, 256));
        tree.push(0); // no more files in this folder
        tree.push(0); // no more folders with this extension
        tree.extend_from_slice(b"txt\0");
        tree.extend_from_slice(b" \0"); // the root, written as a single space
        tree.extend_from_slice(&entry("readme", 0x7fff, 0, 16));
        tree.push(0);
        tree.push(0);
        tree.push(0); // no more extensions

        let mut v = vec![0x34, 0x12, 0xaa, 0x55];
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend_from_slice(&(tree.len() as u32).to_le_bytes());
        v.extend_from_slice(&tree);
        v.extend_from_slice(&[0xcd; 16]);
        v
    }

    #[test]
    fn the_three_levels_each_end_at_an_empty_string() {
        let d = Document::new(MemSource(vpk_bytes()));
        let mut ev = Evaluator::new(vpk());
        let tree = ev.node(&d, &[4]).unwrap();
        // Two extensions, and the empty string that ends the list.
        assert_eq!(tree.child_count, 3);
        assert_eq!(ev.node(&d, &[4, 0, 0]).unwrap().value, Value::Str("vmt".into()));
        assert_eq!(ev.node(&d, &[4, 0, 1, 0, 0]).unwrap().value, Value::Str("materials/models".into()));
        // Two files, and the empty name that ends them.
        assert_eq!(ev.node(&d, &[4, 0, 1, 0, 1]).unwrap().child_count, 3);
        assert_eq!(ev.node(&d, &[4, 0, 1, 0, 1, 1, 0]).unwrap().value, Value::Str("crate".into()));
        assert_eq!(ev.node(&d, &[4, 0, 1, 0, 1, 1, 1, 4]).unwrap().value, Value::UInt(256));
        // The empty name that ends the list carries no record after it.
        assert_eq!(ev.node(&d, &[4, 0, 1, 0, 1, 2]).unwrap().size_bits, 8);
        assert_eq!(ev.node(&d, &[4, 1, 0]).unwrap().value, Value::Str("txt".into()));
        // The bytes after the tree, which only entries in archive 0x7fff use.
        assert_eq!(ev.node(&d, &[5]).unwrap().size_bits, 16 * 8);
    }
}
