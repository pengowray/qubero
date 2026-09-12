//! Microsoft program databases: the debug symbols a Visual C++ or a Rust build
//! writes beside its executable, and what a debugger reads to turn an address
//! back into a function, a file and a line.
//!
//! A PDB is not laid out, it is paged. The file is a run of blocks, all of the
//! one size the header states, and everything in it lives in a *stream*: a
//! list of block numbers, in the order the stream's bytes go, pointing at
//! whichever blocks were free when the compiler wrote them. Two blocks of one
//! stream need not be next to each other and need not be in order. So the
//! container is a small file system, and the symbols, the types, the modules
//! and the source file names are its files.
//!
//! Finding them is three hops. The header says where the *block map* is; the
//! block map lists the blocks the *stream directory* is written across; the
//! directory holds every stream's length and every stream's block numbers.
//! All three read here, and the stream table at the end of them is what a
//! reader opening a PDB wants first: how many streams, how long each one is,
//! and which blocks of the file it is written in.
//!
//! **Where this stops.** A stream of more than one block is nowhere in the
//! file as a run of bytes, so no field inside it can be placed: a template
//! describes bytes where they sit. It is the wall a SQLite record that
//! overflowed and a CAB folder's files stop at, and the only thing between a
//! PDB and its contents, since the blocks are stored rather than compressed:
//! nothing here needs decoding, only joining. So a stream reads as its block
//! list, and the header of the info stream, the type records and the module
//! list stay inside those blocks unread.
//!
//! The stream directory is the one stream this does read through, and only
//! when it fits in a single block. That is where a compiler puts it for
//! anything short of a large program: 16 streams of a C# stub and 87 of a Rust
//! build script both fit in one. Spread over several blocks it stays the
//! blocks it is, with the reason on it.
//!
//! The older container is here too. `2.00` is the same idea with pages instead
//! of blocks, 16-bit page numbers and its geometry at different offsets, which
//! is why it is a template of its own rather than a switch: nothing but the
//! first sixteen bytes is shared. Its header and the pages of its directory
//! read; the directory's own contents do not, since no `2.00` file was to hand
//! to check a reading against.
//!
//! Two other things are called `.pdb` and are not this. A Portable PDB is
//! ECMA-335 metadata and opens with `BSJB`; see [`super::ppdb`]. A Protein
//! Data Bank entry is a text file of `ATOM` records with no signature at all,
//! so nothing identifies one and no template guesses at it.

use crate::template::{Anchor, Endian::Little, Expr as E, Template, Ty as T};

/// What the current container opens with: 24 characters, the end-of-file byte
/// a `type` of the file stops at, `DS`, and three zeroes.
pub const MAGIC: &[u8] = b"Microsoft C/C++ MSF 7.00\r\n\x1aDS\0\0\0";

/// The 1990s container, whose two letters are the initials of the programmer
/// who wrote it rather than anything about the format.
pub const MAGIC_2: &[u8] = b"Microsoft C/C++ program database 2.00\r\n\x1aJG\0\0";

/// The streams a PDB always has at a fixed number. Everything above these is
/// named in the info stream's own table or in the module list, both of which
/// are inside blocks this cannot join yet.
const KINDS: &[(i128, &str)] = &[
    (0, "old directory"),
    (1, "PDB info"),
    (2, "type info (TPI)"),
    (3, "debug info (DBI)"),
    (4, "id info (IPI)"),
];

/// How many blocks a run of `bytes` bytes takes, at `block_size` to a block.
/// There is no ceiling operator, so the division rounds up the usual way.
fn blocks_for(bytes: E, block_size: E) -> E {
    bytes.add(block_size.clone()).sub(E::lit(1)).div(block_size)
}

pub fn pdb() -> Template {
    let size = || E::field("block_size");
    Template::new(
        "pdb",
        T::structure(
            "PdbFile",
            vec![
                ("magic", T::magic(MAGIC)),
                // 4096 in everything a modern toolchain writes, 512 in the
                // stubs a C# build leaves behind.
                ("block_size", T::u32(Little)),
                // Blocks 1 and 2 are both free-block maps and the writer uses
                // one of them at a time; this says which. The other is where
                // the next write puts its map, which is how a PDB is updated
                // without being rewritten.
                ("free_block_map", T::enumeration("PdbFreeBlockMap", T::u32(Little), &[(1, "first"), (2, "second")])),
                ("block_count", T::u32(Little)),
                ("directory_bytes", T::u32(Little)),
                ("unused", T::u32(Little)),
                ("block_map_block", T::u32(Little)),
                // What the geometry says the file is: a PDB is a whole number
                // of blocks, so this is the length, and a file that is shorter
                // has lost its tail.
                ("claimed_size", T::computed(size().mul(E::field("block_count")))),
                // The map in use, one bit per block of the first interval:
                // set means free. A map covers as many blocks as it has bits,
                // so a file longer than that has another pair further in, at
                // the same place in each interval.
                ("free_blocks", T::at(size().mul(E::field("free_block_map")), T::bytes(size()))),
                // The one indirection the header points at: the blocks the
                // stream directory is written across.
                (
                    "block_map",
                    T::at(
                        size().mul(E::field("block_map_block")),
                        T::array(block_ref(), blocks_for(E::field("directory_bytes"), size())),
                    ),
                ),
                // The directory itself, a block at a time where the map said.
                // Declared last because a list placed by offsets reads to the
                // end of its container, and these blocks are anywhere.
                (
                    "directory",
                    T::pointer_list_sized("block_map", &["at"], Anchor::File, E::lit(0), directory_block()),
                ),
            ],
        ),
    )
}

pub fn pdb2() -> Template {
    let size = || E::field("page_size");
    Template::new(
        "pdb2",
        T::structure(
            "Pdb2File",
            vec![
                ("magic", T::magic(MAGIC_2)),
                ("page_size", T::u32(Little)),
                // Where the free page map is, and how many pages the file has.
                // Both are 16-bit here, which is what caps this container at
                // a few hundred megabytes and why the format was replaced.
                ("free_page_map", T::u16(Little)),
                ("page_count", T::u16(Little)),
                ("directory_bytes", T::u32(Little)),
                ("directory_unused", T::u32(Little)),
                ("claimed_size", T::computed(size().mul(E::field("page_count")))),
                // The directory's pages are listed here in the header itself
                // rather than behind a map of their own.
                (
                    "directory_pages",
                    T::array(page_ref(), blocks_for(E::field("directory_bytes"), size())),
                ),
                // What those pages hold is a stream table like the one above,
                // with 16-bit page numbers. It is left as its pages until a
                // file to check the reading against turns up.
                (
                    "directory",
                    T::pointer_list_sized("directory_pages", &["at"], Anchor::File, E::lit(0), T::bytes(size())),
                ),
            ],
        ),
    )
}

/// One block of the stream directory. The whole directory reads as the table
/// it is when it fits in this one block; otherwise the table's own fields
/// would be cut in half by the block boundary, and it stays bytes.
fn directory_block() -> T {
    T::switch(
        blocks_for(E::field("directory_bytes"), E::field("block_size")).equals(E::lit(1)),
        vec![(1, T::sized(E::field("block_size"), directory()))],
        T::bytes(E::field("block_size")),
    )
}

/// The stream table: how many streams, how long each one is, and then each
/// one's blocks in the order its bytes go.
///
/// The two lists are read separately because that is how they are written: all
/// the lengths, and then all the block numbers, whose count each length
/// decides. A stream's length is repeated beside its blocks so that a row
/// holding a block list says what it is a list for.
fn directory() -> T {
    T::structure(
        "PdbDirectory",
        vec![
            ("stream_count", T::u32(Little)),
            ("stream_sizes", T::array(T::u32(Little), E::field("stream_count"))),
            ("streams", T::array(stream(), E::field("stream_count"))),
        ],
    )
}

/// One stream, as the blocks it is written in.
///
/// A length of all ones is not a length: it is how the format says a stream
/// number is not in use, and the writer leaves out its block list entirely. A
/// count worked out from it would ask for eight million blocks.
fn stream() -> T {
    let size = || E::elem("stream_sizes", E::idx());
    T::structure_named(
        "PdbStream",
        "kind",
        "blocks",
        vec![
            ("kind", T::enumeration("PdbStreamKind", T::computed(E::idx()), KINDS)),
            ("size", T::computed(size())),
            (
                "blocks",
                T::switch(
                    size().equals(E::lit(0xffff_ffffi64)),
                    vec![(1, T::array(block_ref(), E::lit(0)))],
                    T::array(block_ref(), blocks_for(size(), E::field("block_size"))),
                ),
            ),
        ],
    )
    .counted_as("stream")
}

/// A block number and the offset it works out to, which is what a list placed
/// by these needs and what saves a reader the multiplication.
fn block_ref() -> T {
    T::inline_structure(
        "PdbBlockRef",
        vec![("block", T::u32(Little)), ("at", T::computed(E::field("block").mul(E::field("block_size"))))],
    )
    .counted_as("block")
}

/// The same for the older container, whose page numbers are half as wide.
fn page_ref() -> T {
    T::inline_structure(
        "Pdb2PageRef",
        vec![("page", T::u16(Little)), ("at", T::computed(E::field("page").mul(E::field("page_size"))))],
    )
    .counted_as("page")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::Document,
        eval::{Evaluator, Value},
        source::MemSource,
    };

    const BLOCK: usize = 512;

    /// Field indices into the root, which is what a test asks a node for.
    const BLOCK_COUNT: usize = 3;
    const CLAIMED: usize = 7;
    const BLOCK_MAP: usize = 9;
    const DIRECTORY: usize = 10;

    /// A PDB of four streams, laid out the way a compiler lays one out: the
    /// superblock, the two free-block maps, the directory, and then the blocks
    /// the streams are written in.
    ///
    /// The lengths are the interesting part. One stream is empty, one is the
    /// all-ones that means the number is not in use, and one is longer than a
    /// block, so the table has to say two blocks for it.
    fn pdb_file(directory_bytes: Option<u32>, map_blocks: &[u32]) -> Vec<u8> {
        let sizes: [u32; 4] = [0x20, 0, 0xffff_ffff, 700];
        let blocks: [&[u32]; 4] = [&[4], &[], &[], &[5, 6]];

        let mut directory = 4u32.to_le_bytes().to_vec();
        for size in sizes {
            directory.extend_from_slice(&size.to_le_bytes());
        }
        for list in blocks {
            for block in list {
                directory.extend_from_slice(&block.to_le_bytes());
            }
        }

        let mut v = MAGIC.to_vec();
        v.extend_from_slice(&(BLOCK as u32).to_le_bytes());
        v.extend_from_slice(&2u32.to_le_bytes());
        v.extend_from_slice(&8u32.to_le_bytes());
        v.extend_from_slice(&directory_bytes.unwrap_or(directory.len() as u32).to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&7u32.to_le_bytes());
        v.resize(BLOCK * 8, 0);
        // The directory in block 3, and the map of where it is in block 7.
        v[BLOCK * 3..BLOCK * 3 + directory.len()].copy_from_slice(&directory);
        for (i, block) in map_blocks.iter().enumerate() {
            v[BLOCK * 7 + i * 4..BLOCK * 7 + i * 4 + 4].copy_from_slice(&block.to_le_bytes());
        }
        v
    }

    /// The header places the map, the map places the directory, and the
    /// directory reads as the streams it holds.
    #[test]
    fn three_hops_from_the_header_to_the_stream_table() {
        let d = Document::new(MemSource(pdb_file(None, &[3])));
        let mut e = Evaluator::new(pdb());
        // A whole number of blocks, which is what the file length has to be.
        assert_eq!(e.node(&d, &[BLOCK_COUNT]).unwrap().value.as_int(), Some(8));
        assert_eq!(e.node(&d, &[CLAIMED]).unwrap().value.as_int(), Some(BLOCK as i128 * 8));
        // One block holds the directory, and the map says which.
        assert_eq!(e.node(&d, &[BLOCK_MAP, 0]).unwrap().child_count, 1);
        assert_eq!(e.node(&d, &[BLOCK_MAP, 0, 0, 0]).unwrap().value.as_int(), Some(3));
        assert_eq!(e.node(&d, &[BLOCK_MAP, 0, 0, 1]).unwrap().value.as_int(), Some(BLOCK as i128 * 3));
        // Four streams, the first of them the old directory.
        assert_eq!(e.node(&d, &[DIRECTORY, 0, 0]).unwrap().value.as_int(), Some(4));
        assert_eq!(e.node(&d, &[DIRECTORY, 0, 2]).unwrap().child_count, 4);
        assert_eq!(e.node(&d, &[DIRECTORY, 0, 2, 0, 0]).unwrap().value, Value::Enum {
            raw: 0,
            name: Some("old directory".into()),
            hex: false
        });
    }

    /// A stream's blocks are as many as its length needs, and the two lengths
    /// that mean no blocks at all take none: an empty stream, and the all-ones
    /// that says the number is not in use.
    #[test]
    fn a_streams_blocks_are_counted_from_its_length() {
        let d = Document::new(MemSource(pdb_file(None, &[3])));
        let mut e = Evaluator::new(pdb());
        let blocks = |e: &mut Evaluator, i: usize| e.node(&d, &[DIRECTORY, 0, 2, i, 2]).unwrap().child_count;
        assert_eq!(blocks(&mut e, 0), 1);
        assert_eq!(blocks(&mut e, 1), 0);
        assert_eq!(blocks(&mut e, 2), 0);
        // 700 bytes is two blocks of 512, and the second of them is block 6.
        assert_eq!(blocks(&mut e, 3), 2);
        assert_eq!(e.node(&d, &[DIRECTORY, 0, 2, 3, 1]).unwrap().value.as_int(), Some(700));
        assert_eq!(e.node(&d, &[DIRECTORY, 0, 2, 3, 2, 1, 0]).unwrap().value.as_int(), Some(6));
    }

    /// A directory spread over more than one block stays the blocks it is:
    /// the table's own fields would be cut in half by the boundary, and a
    /// count read across it would be a number nobody wrote.
    #[test]
    fn a_directory_of_several_blocks_reads_as_its_blocks() {
        let d = Document::new(MemSource(pdb_file(Some(600), &[3, 4])));
        let mut e = Evaluator::new(pdb());
        assert_eq!(e.node(&d, &[BLOCK_MAP, 0]).unwrap().child_count, 2);
        assert_eq!(e.node(&d, &[DIRECTORY]).unwrap().child_count, 2);
        let first = e.node(&d, &[DIRECTORY, 0]).unwrap();
        assert_eq!(first.size_bits, BLOCK as u64 * 8);
        assert!(matches!(first.value, Value::Bytes { .. }));
    }

    /// The container before this one: pages rather than blocks, the geometry
    /// at its own offsets, and the directory's pages in the header itself.
    #[test]
    fn the_older_container_reads_its_geometry_and_its_directory_pages() {
        let mut v = MAGIC_2.to_vec();
        v.extend_from_slice(&1024u32.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        v.extend_from_slice(&6u16.to_le_bytes());
        v.extend_from_slice(&1200u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&3u16.to_le_bytes());
        v.extend_from_slice(&5u16.to_le_bytes());
        v.resize(1024 * 6, 0);
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(pdb2());
        assert_eq!(e.node(&d, &[1]).unwrap().value.as_int(), Some(1024));
        assert_eq!(e.node(&d, &[6]).unwrap().value.as_int(), Some(1024 * 6));
        // 1200 bytes of directory is two pages of 1024, and it says which.
        assert_eq!(e.node(&d, &[7]).unwrap().child_count, 2);
        assert_eq!(e.node(&d, &[7, 1, 1]).unwrap().value.as_int(), Some(1024 * 5));
        assert_eq!(e.node(&d, &[8]).unwrap().child_count, 2);
    }
}
