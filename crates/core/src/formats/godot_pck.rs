//! `.pck`: everything a Godot game ships, in one file.
//!
//! A header, a directory of paths, and the files themselves. What makes it
//! worth reading rather than skipping is that every entry names a real file
//! with a real length, so the pack divides cleanly: a treemap of one says
//! which scene, which texture and which sound is spending the download, and
//! each entry's bytes are a file of its own.
//!
//! The same pack can be appended to the game's executable rather than shipped
//! beside it, which is what a single-file export is. That is why the offsets
//! are read from an origin: they count from where the pack begins, not from
//! where the file does.
//!
//! **Four versions.** Godot 3 wrote version 1: a fixed header, then the
//! directory, then the files, with absolute offsets. Godot 4 added a flags
//! word and a base offset (version 2), then moved the directory to the end and
//! wrote where it is (version 3), then added per-file deltas for patch packs
//! (version 4). A version none of them wrote stops the read.
//!
//! **What this does not read.**
//!
//! - An encrypted directory. The flag says when there is one, and the read
//!   stops there: the paths, the offsets and the sizes are all inside the
//!   encryption, so there is nothing left to place the files by.
//! - What is in each file. A pack holds scenes, textures, audio and shaders,
//!   and nothing in the directory says which is which beyond the extension on
//!   the path.
//! - The alignment padding between the header and the first file, and between
//!   the last file and the directory. Both read as gaps, which is what they
//!   are.

use crate::template::{Anchor, Encoding, Endian::Little, Expr as E, StrLen, Template, Ty as T};

pub const MAGIC: &[u8] = b"GDPC";

/// Flags on the pack as a whole.
const PACK_FLAGS: &[(u32, &str)] =
    &[(0, "encrypted directory"), (1, "offsets relative to pack"), (2, "sparse bundle")];

/// Flags on one entry. A removal is how a patch pack says a file the base pack
/// had is gone.
const FILE_FLAGS: &[(u32, &str)] = &[(0, "encrypted"), (1, "removal"), (2, "delta")];

/// One row of the directory.
///
/// The path length counts the NUL padding that rounds it up to four bytes, so
/// the field is exactly as long as the number says and the padding is inside
/// it rather than beside it.
fn entry(version: u32) -> T {
    let mut fields = vec![
        ("path_length", T::u32(Little)),
        ("path", T::text(StrLen::Padded { size: E::field("path_length"), pad: 0 }, Encoding::Utf8)),
        ("offset", T::u64(Little)),
        ("size", T::u64(Little)),
        // Of the file's contents, for the engine to check on load.
        ("md5", T::bytes(E::lit(16))),
    ];
    if version >= 2 {
        fields.push(("flags", T::flags("FileFlags", T::u32(Little), FILE_FLAGS)));
    }
    T::structure_named("PackEntry", "path", "", fields).counted_as("file")
}

/// The bytes of one packed file, named by the directory row that placed it.
///
/// The name is the whole point. A pack with four thousand entries laid out as
/// `[0]` to `[3999]` says nothing about a game; laid out as
/// `res://levels/forest.scn` it is the project tree.
fn packed_file() -> T {
    T::structure_named(
        "PackedFile",
        "path",
        "",
        vec![
            ("path", T::computed_text(E::elem_field("entries", E::idx(), &["path"]))),
            ("data", T::bytes(E::elem_field("entries", E::idx(), &["size"]))),
        ],
    )
    .counted_as("file")
}

/// The count, the rows, and then the files those rows point at.
///
/// `base` is what an entry's offset is counted from: nothing in version 1,
/// where the offsets are already absolute, and the pack's own file base after
/// that.
fn directory(version: u32, base: E) -> Vec<(&'static str, T)> {
    vec![
        ("file_count", T::u32(Little)),
        ("entries", T::array(entry(version), E::field("file_count"))),
        (
            "files",
            T::pointer_list_sized("entries", &["offset"], Anchor::Origin, base, packed_file()),
        ),
    ]
}

/// The same, for a version that keeps its directory at the end of the pack and
/// says in the header where.
///
/// Two fields that take no bytes where they are declared rather than one: a
/// pointer list needs an array it can name, and naming a structure holding the
/// count and the array would reach the structure. So the count is read at the
/// directory offset and the rows four bytes after it.
fn remote_directory(version: u32) -> Vec<(&'static str, T)> {
    let at = || E::field("directory_offset");
    vec![
        ("file_count", T::at_origin(at(), T::u32(Little))),
        ("entries", T::at_origin(at().add(E::lit(4)), T::array(entry(version), E::field("file_count")))),
        (
            "files",
            T::pointer_list_sized(
                "entries",
                &["offset"],
                Anchor::Origin,
                E::field("file_base"),
                packed_file(),
            ),
        ),
    ]
}

/// A directory nothing here can read, and the files it would have placed.
fn encrypted() -> T {
    T::structure("EncryptedDirectory", vec![("contents", T::bytes(E::lit(0)))])
}

/// Version 1, which is every pack Godot 3 wrote: no flags, no base, and the
/// directory straight after the reserved words.
fn version1() -> T {
    let mut fields = vec![("reserved", T::array(T::u32(Little), E::lit(16)))];
    fields.extend(directory(1, E::lit(0)));
    T::structure("GodotPack1", fields)
}

/// Version 2: a flags word and a base offset, and the directory still in the
/// same place.
fn version2() -> T {
    let mut fields = vec![
        ("pack_flags", T::flags("PackFlags", T::u32(Little), PACK_FLAGS)),
        // Where the first file's bytes begin. An entry's offset counts from
        // here, which is what lets a pack be appended to an executable.
        ("file_base", T::u64(Little)),
        ("reserved", T::array(T::u32(Little), E::lit(16))),
    ];
    fields.push((
        "directory",
        T::switch(
            E::field("pack_flags").bit(0),
            vec![(1, encrypted())],
            T::structure("PackDirectory", directory(2, E::field("file_base"))),
        ),
    ));
    T::structure("GodotPack2", fields)
}

/// Versions 3 and 4, which put the directory at the end so that a pack can be
/// written without knowing in advance how long it will be.
fn version34(version: u32) -> T {
    let name = if version == 3 { "GodotPack3" } else { "GodotPack4" };
    T::structure(
        name,
        vec![
            ("pack_flags", T::flags("PackFlags", T::u32(Little), PACK_FLAGS)),
            ("file_base", T::u64(Little)),
            ("directory_offset", T::u64(Little)),
            ("reserved", T::array(T::u32(Little), E::lit(16))),
            (
                "directory",
                T::switch(
                    E::field("pack_flags").bit(0),
                    vec![(1, encrypted())],
                    T::structure("PackDirectory", remote_directory(version)),
                ),
            ),
        ],
    )
}

pub fn godot_pck() -> Template {
    let root = T::origin(T::structure(
        "GodotPack",
        vec![
            ("magic", T::magic(MAGIC)),
            ("pack_version", T::u32(Little)),
            ("engine_version_major", T::u32(Little)),
            ("engine_version_minor", T::u32(Little)),
            ("engine_version_patch", T::u32(Little)),
            (
                "contents",
                T::switch(
                    E::field("pack_version"),
                    vec![(1, version1()), (2, version2()), (3, version34(3)), (4, version34(4))],
                    // Nothing below here is where this template thinks it is.
                    T::bytes(E::Remaining),
                ),
            ),
        ],
    ));
    Template::new("godotpck", root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn u32le(n: u32) -> Vec<u8> {
        n.to_le_bytes().to_vec()
    }

    /// A version 4 pack holding two files, with the directory at the end.
    fn pack() -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        b.extend(u32le(4)); // pack version
        b.extend(u32le(4)); // engine major
        b.extend(u32le(3)); // engine minor
        b.extend(u32le(1)); // engine patch
        b.extend(u32le(2)); // offsets relative to the pack
        let base_at = b.len();
        b.extend(0u64.to_le_bytes());
        let dir_at = b.len();
        b.extend(0u64.to_le_bytes());
        b.extend([0; 64]); // sixteen reserved words

        let file_base = b.len() as u64;
        b[base_at..base_at + 8].copy_from_slice(&file_base.to_le_bytes());
        b.extend_from_slice(b"the first file's bytes");
        let second = b.len() as u64 - file_base;
        b.extend_from_slice(b"and the second's");
        let dir = b.len() as u64;
        b[dir_at..dir_at + 8].copy_from_slice(&dir.to_le_bytes());

        b.extend(u32le(2)); // two entries
        for (path, offset, size) in
            [("res://one.txt", 0u64, 22u64), ("res://two.txt", second, 16)]
        {
            // The length counts the NULs that round it up to four bytes.
            let padded = path.len().next_multiple_of(4);
            b.extend(u32le(padded as u32));
            b.extend_from_slice(path.as_bytes());
            b.extend(std::iter::repeat_n(0u8, padded - path.len()));
            b.extend(offset.to_le_bytes());
            b.extend(size.to_le_bytes());
            b.extend([0; 16]); // md5
            b.extend(u32le(0)); // flags
        }
        b
    }

    fn ev(bytes: Vec<u8>) -> (Document<MemSource>, Evaluator) {
        (Document::new(MemSource(bytes)), Evaluator::new(godot_pck()))
    }

    #[test]
    fn the_header_says_which_pack_version_and_which_engine() {
        let (d, mut e) = ev(pack());
        assert_eq!(e.node(&d, &[1]).unwrap().value, Value::UInt(4));
        assert_eq!(e.node(&d, &[2]).unwrap().value, Value::UInt(4));
        assert_eq!(e.node(&d, &[4]).unwrap().value, Value::UInt(1));
    }

    /// The directory is not where it is declared; it is where the header says.
    #[test]
    fn the_directory_is_read_at_the_offset_the_header_gives() {
        let (d, mut e) = ev(pack());
        let offset = e.node(&d, &[5, 2]).unwrap().value;
        let count = e.node(&d, &[5, 4, 0, 0]).unwrap();
        assert_eq!(offset, Value::UInt((count.offset_bits / 8).into()));
        assert_eq!(count.value, Value::UInt(2));
        assert_eq!(e.node(&d, &[5, 4, 1, 0]).unwrap().child_count, 2);
    }

    #[test]
    fn an_entry_reads_as_its_path_offset_and_size() {
        let (d, mut e) = ev(pack());
        assert_eq!(e.node(&d, &[5, 4, 1, 0, 0, 1]).unwrap().value, Value::Str("res://one.txt".into()));
        assert_eq!(e.node(&d, &[5, 4, 1, 0, 1, 1]).unwrap().value, Value::Str("res://two.txt".into()));
        assert_eq!(e.node(&d, &[5, 4, 1, 0, 1, 3]).unwrap().value, Value::UInt(16));
    }

    /// Each packed file sits at its own entry's offset, counted from the base
    /// the header gave, and carries that entry's path as its name.
    #[test]
    fn every_file_is_placed_by_its_own_entry() {
        let (d, mut e) = ev(pack());
        let files = e.node(&d, &[5, 4, 2]).unwrap();
        assert_eq!(files.child_count, 2);
        let base = e.node(&d, &[5, 1]).unwrap().value.as_int().unwrap() as u64;
        let first = e.node(&d, &[5, 4, 2, 0]).unwrap();
        assert_eq!(first.offset_bits / 8, base);
        assert_eq!(first.size_bits / 8, 22);
        assert_eq!(e.node(&d, &[5, 4, 2, 1, 0]).unwrap().value, Value::Str("res://two.txt".into()));
        assert_eq!(e.node(&d, &[5, 4, 2, 1]).unwrap().size_bits / 8, 16);
    }

    /// Version 1 keeps its directory right after the header and counts its
    /// offsets from the start of the pack.
    #[test]
    fn a_godot_3_pack_reads_its_directory_in_place() {
        let mut b = MAGIC.to_vec();
        b.extend(u32le(1)); // pack version
        b.extend(u32le(3));
        b.extend(u32le(6));
        b.extend(u32le(3));
        b.extend([0; 64]); // reserved
        b.extend(u32le(1)); // one entry
        b.extend(u32le(16));
        b.extend_from_slice(b"res://only.txt\0\0");
        let offset_at = b.len();
        b.extend(0u64.to_le_bytes());
        b.extend(5u64.to_le_bytes());
        b.extend([0; 16]); // md5, and no flags word in version 1
        let start = b.len() as u64;
        b[offset_at..offset_at + 8].copy_from_slice(&start.to_le_bytes());
        b.extend_from_slice(b"bytes");

        let (d, mut e) = ev(b);
        assert_eq!(e.node(&d, &[5, 1]).unwrap().value, Value::UInt(1));
        assert_eq!(e.node(&d, &[5, 2, 0, 1]).unwrap().value, Value::Str("res://only.txt".into()));
        let placed = e.node(&d, &[5, 3, 0]).unwrap();
        assert_eq!(placed.offset_bits / 8, start);
        assert_eq!(placed.size_bits / 8, 5);
    }

    /// Version 2, which Godot 4.0 to 4.2 wrote: a flags word and a file base
    /// like the later ones, and the directory in front of the files like the
    /// earlier one.
    #[test]
    fn a_version_2_pack_keeps_its_directory_before_the_files() {
        let mut b = MAGIC.to_vec();
        b.extend(u32le(2)); // pack version
        b.extend(u32le(4));
        b.extend(u32le(1));
        b.extend(u32le(0));
        b.extend(u32le(2)); // offsets relative to the pack
        let base_at = b.len();
        b.extend(0u64.to_le_bytes());
        b.extend([0; 64]); // reserved
        b.extend(u32le(1)); // one entry
        b.extend(u32le(16));
        b.extend_from_slice(b"res://only.txt\0\0");
        b.extend(0u64.to_le_bytes()); // at the file base itself
        b.extend(5u64.to_le_bytes());
        b.extend([0; 16]); // md5
        b.extend(u32le(0)); // flags, which version 1 does not have
        let file_base = b.len() as u64;
        b[base_at..base_at + 8].copy_from_slice(&file_base.to_le_bytes());
        b.extend_from_slice(b"bytes");

        let (d, mut e) = ev(b);
        assert_eq!(e.node(&d, &[1]).unwrap().value, Value::UInt(2));
        assert_eq!(e.node(&d, &[5, 3, 1, 0, 1]).unwrap().value, Value::Str("res://only.txt".into()));
        // The offset counts from the base rather than from the start of the
        // pack, which is the whole of what version 2 added.
        let placed = e.node(&d, &[5, 3, 2, 0]).unwrap();
        assert_eq!(placed.offset_bits / 8, file_base);
        assert_eq!(placed.size_bits / 8, 5);
    }

    /// An encrypted directory is not guessed at: the rows and everything they
    /// would have placed stay unread.
    #[test]
    fn an_encrypted_directory_stops_rather_than_reading_noise() {
        let mut b = pack();
        b[20] = 0x03; // pack_flags: encrypted directory, relative base
        let (d, mut e) = ev(b);
        assert_eq!(e.node(&d, &[5, 4]).unwrap().child_count, 1);
        assert_eq!(e.node(&d, &[5, 4, 0]).unwrap().size_bits, 0);
    }
}
