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
//!
//! **A stream is read where it lies, when it lies in one piece.** A template
//! describes bytes where they sit, and a stream scattered over the file does
//! not sit anywhere: joining its blocks is the one thing the IR cannot say,
//! and it is all that would be needed, since the blocks are stored rather than
//! compressed. But a compiler allocating a stream usually gets a run of blocks
//! in one go, and a stream whose blocks are one ascending run *is* a run of
//! bytes in the file, at the first block and as long as the stream says. So
//! every stream is checked for that and read where it lies when it holds:
//! [`runs_together`] is the check, and it is a proof rather than a guess.
//!
//! Of the 39 program databases on the machine this was written on, all 39 kept
//! their stream directory in one run, 38 kept the info stream in one, 34 the
//! type stream, and 13 the debug information stream. A stream that fails the
//! check reads as its block list and nothing else, which is the honest answer
//! until the IR can stitch the blocks.
//!
//! What reads inside a stream: the info stream's version, signature, age and
//! GUID, which is what a debugger matches a PDB to an executable by, and the
//! table of named streams after it, so `/names` and `/LinkInfo` can be found
//! by the numbers they were given. The type and id streams' headers, and the
//! type records themselves as the lengths and kinds they are. The debug
//! information stream's header and the seven substreams its sizes place.
//!
//! What does not: what a CodeView record *says*. A type record's kind is a
//! number here rather than an `LF_STRUCTURE`, and the fields inside one are
//! its bytes; the symbol records in the module streams are the same. That is
//! the next thing to write, and it is a table of a hundred record shapes
//! rather than anything the IR is missing.
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

use crate::formats::pe_tables::MACHINE;
use crate::template::{Anchor, Endian::Little, Expr as E, Template, Until, Ty as T};

/// What the current container opens with: 24 characters, the end-of-file byte
/// a `type` of the file stops at, `DS`, and three zeroes.
pub const MAGIC: &[u8] = b"Microsoft C/C++ MSF 7.00\r\n\x1aDS\0\0\0";

/// The 1990s container, whose two letters are the initials of the programmer
/// who wrote it rather than anything about the format.
pub const MAGIC_2: &[u8] = b"Microsoft C/C++ program database 2.00\r\n\x1aJG\0\0";

/// The streams a PDB always has at a fixed number. Everything above these is
/// named in the info stream's table of named streams or in the module list.
const KINDS: &[(i128, &str)] = &[
    (0, "old directory"),
    (1, "PDB info"),
    (2, "type info (TPI)"),
    (3, "debug info (DBI)"),
    (4, "id info (IPI)"),
];

/// Which toolchain wrote the info stream, as a date rather than a version: the
/// format's own way of saying which fields are there.
const INFO_VERSIONS: &[(i128, &str)] = &[
    (19941610, "VC2"),
    (19950623, "VC4"),
    (19950814, "VC41"),
    (19960307, "VC50"),
    (19970604, "VC98"),
    (19990604, "VC70 (deprecated)"),
    (19990903, "VC70"),
    (20000404, "VC80"),
    (20030901, "VC110"),
    (20091201, "VC140"),
];

/// The same for the type stream, which counts its versions its own way.
const TPI_VERSIONS: &[(i128, &str)] =
    &[(19950410, "V40"), (19951122, "V41"), (19961031, "V50"), (19990903, "V70"), (20040203, "V80")];

/// And for the debug information stream.
const DBI_VERSIONS: &[(i128, &str)] =
    &[(930803, "VC41"), (19960307, "VC50"), (19970606, "VC60"), (19990903, "VC70"), (20091201, "VC110")];

/// What the debug information stream says about how it was linked.
const DBI_FLAGS: &[(u32, &str)] =
    &[(0, "incrementally linked"), (1, "private symbols stripped"), (2, "conflicting types")];

/// How many blocks a run of `bytes` bytes takes, at `block_size` to a block.
/// There is no ceiling operator, so the division rounds up the usual way.
fn blocks_for(bytes: E, block_size: E) -> E {
    bytes.add(block_size.clone()).sub(E::lit(1)).div(block_size)
}

/// A list of `count` numbers, each the block a run would have started at for
/// the entry that holds it to be where it is: block number less position.
///
/// Every entry of a list of consecutive ascending blocks answers the same
/// number here, and no other list does. It is a field of its own because an
/// aggregate reads an array's own values and these are not written anywhere:
/// what the file holds is the blocks, and this is the question asked of them.
fn run_starts(blocks: &str, count: E) -> T {
    T::array(T::computed(E::elem(blocks, E::idx()).sub(E::idx())), count)
}

/// Whether the blocks of `starts` are one ascending run, which is what lets
/// the stream they hold be read where it lies.
///
/// A sum is at most the count times the largest, and is that only when every
/// one of them is the largest. So this says every entry agrees on where the
/// run began, which is exactly what "consecutive and in order" means. A
/// repeated block number disagrees, a swapped pair disagrees, and a list that
/// holds the right blocks in the wrong order disagrees, so nothing is read as
/// a run that is not one.
fn runs_together(starts: &str, count: E) -> E {
    E::sum_of(starts).equals(E::max_of(starts).mul(count))
}

pub fn pdb() -> Template {
    let size = || E::field("block_size");
    let blocks = || blocks_for(E::field("directory_bytes"), size());
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
                ("block_map", T::at(size().mul(E::field("block_map_block")), T::array(T::u32(Little), blocks()))),
                ("directory_run", run_starts("block_map", blocks())),
                // The directory itself, read where it lies when its blocks are
                // one run, and as those blocks when they are not.
                (
                    "directory",
                    T::switch(
                        runs_together("directory_run", blocks()),
                        vec![(
                            1,
                            T::at(
                                E::elem("block_map", E::lit(0)).mul(size()),
                                T::sized(E::field("directory_bytes"), directory()),
                            ),
                        )],
                        scattered_directory(blocks()),
                    ),
                ),
            ],
        )
        .machinery(&["directory_run"]),
    )
}

/// A stream directory whose blocks are scattered: the blocks themselves, read
/// again as the offsets they work out to so that the bytes of each one can be
/// looked at, and no table over them.
fn scattered_directory(count: E) -> T {
    T::structure(
        "PdbScatteredDirectory",
        vec![
            (
                "blocks",
                T::at(E::field("block_size").mul(E::field("block_map_block")), T::array(block_ref(), count)),
            ),
            ("pages", T::pointer_list_sized("blocks", &["at"], Anchor::File, E::lit(0), T::bytes(E::field("block_size")))),
        ],
    )
}

/// The stream table: how many streams, how long each one is, and then each
/// one's blocks in the order its bytes go.
///
/// The two lists are read separately because that is how they are written: all
/// the lengths, and then all the block numbers, whose count each length
/// decides.
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

/// One stream: its blocks, and what is in them when they are one run.
///
/// A length of all ones is not a length: it is how the format says a stream
/// number is not in use, and the writer leaves out its block list entirely. A
/// count worked out from it would ask for eight million blocks, so that length
/// and a length of zero both come to no blocks and nothing read.
fn stream() -> T {
    let size = || E::elem("stream_sizes", E::idx());
    let count = || blocks_for(size(), E::field("block_size"));
    let empty = || size().equals(E::lit(0)).or(size().equals(E::lit(0xffff_ffffi64)));
    let nothing = || T::bytes(E::lit(0));
    T::structure_named(
        "PdbStream",
        "kind",
        "",
        vec![
            ("kind", T::enumeration("PdbStreamKind", T::computed(E::idx()), KINDS)),
            ("size", T::computed(size())),
            ("blocks", T::switch(empty(), vec![(1, T::array(T::u32(Little), E::lit(0)))], T::array(T::u32(Little), count()))),
            ("run", T::switch(empty(), vec![(1, run_starts("blocks", E::lit(0)))], run_starts("blocks", count()))),
            (
                "contents",
                T::switch(
                    empty(),
                    vec![(1, nothing())],
                    T::switch(
                        runs_together("run", count()),
                        vec![(
                            1,
                            T::at(E::elem("blocks", E::lit(0)).mul(E::field("block_size")), T::sized(size(), body())),
                        )],
                        nothing(),
                    ),
                ),
            ),
        ],
    )
    .counted_as("stream")
    .machinery(&["run"])
}

/// What a stream holds, by which stream it is. Everything above the fixed five
/// is a module's symbols, a hash table or a string table, and which of those
/// is written in the streams that name them rather than in the stream.
fn body() -> T {
    T::switch(
        E::idx(),
        vec![(1, info_stream()), (2, type_stream("TpiStream")), (4, type_stream("IpiStream")), (3, dbi_stream())],
        T::bytes(E::Remaining),
    )
}

/// The info stream: which build this PDB is for, and the table of named
/// streams that says where the rest of it is.
///
/// The GUID and the age are the point of it. A debugger opening an executable
/// reads the same two out of its debug directory and will not use a PDB whose
/// pair does not match, which is what stops yesterday's symbols being read
/// against today's code.
fn info_stream() -> T {
    T::structure(
        "PdbInfoStream",
        vec![
            ("version", T::enumeration("PdbInfoVersion", T::u32(Little), INFO_VERSIONS)),
            // Seconds since 1970, from when the PDB was written.
            ("signature", T::u32(Little)),
            // How many times it has been written since: an incremental link
            // updates a PDB in place and counts here.
            ("age", T::u32(Little)),
            ("guid", T::bytes(E::lit(16))),
            // The names of the named streams, one after another, and then the
            // hash table that maps each name to the stream it was given.
            ("names_size", T::u32(Little)),
            ("names", T::sized(E::field("names_size"), T::structure("PdbNames", vec![("names", T::repeat(T::cstr(), Until::End))]))),
            ("named_count", T::u32(Little)),
            ("named_capacity", T::u32(Little)),
            // Which buckets hold something and which held something that was
            // taken out, as bit vectors of as many words as they say.
            ("present_words", T::u32(Little)),
            ("present", T::array(T::u32(Little), E::field("present_words"))),
            ("deleted_words", T::u32(Little)),
            ("deleted", T::array(T::u32(Little), E::field("deleted_words"))),
            ("named_streams", T::array(named_stream(), E::field("named_count"))),
            // What the writer could do, which is how a reader knows whether to
            // trust the id stream and the hash tables.
            ("features", T::array(T::u32(Little), E::Remaining.div(E::lit(4)))),
        ],
    )
    .field_time("signature", crate::template::Time::unix())
}

/// One named stream: where its name is in the buffer above, and the stream
/// number that name stands for. The name is not repeated here, and nothing in
/// the IR reaches a string by the offset another field holds, so the two are
/// read side by side rather than joined.
fn named_stream() -> T {
    T::inline_structure("PdbNamedStream", vec![("name_offset", T::u32(Little)), ("stream", T::u32(Little))])
        .counted_as("name")
}

/// The type stream, and the id stream, which is the same shape: a header of
/// where everything in it is, and then the records themselves.
fn type_stream(name: &str) -> T {
    T::structure(
        name,
        vec![
            ("version", T::enumeration("TpiVersion", T::u32(Little), TPI_VERSIONS)),
            // How long this header is, which is what the records start after.
            ("header_size", T::u32(Little)),
            // Types are numbered from 0x1000; below that the numbers are the
            // built-in types, which no record describes.
            ("type_index_begin", T::u32(Little)),
            ("type_index_end", T::u32(Little)),
            ("type_record_bytes", T::u32(Little)),
            // The hash of the records lives in a stream of its own, and these
            // say which stream and how it is laid out inside it.
            ("hash_stream", T::u16(Little)),
            ("hash_aux_stream", T::u16(Little)),
            ("hash_key_size", T::u32(Little)),
            ("hash_bucket_count", T::u32(Little)),
            ("hash_values_at", T::i32(Little)),
            ("hash_values_bytes", T::u32(Little)),
            ("index_offsets_at", T::i32(Little)),
            ("index_offsets_bytes", T::u32(Little)),
            ("hash_adjustments_at", T::i32(Little)),
            ("hash_adjustments_bytes", T::u32(Little)),
            // The records, which tile the space exactly: one after another,
            // each as long as it says, and as many of them as the two type
            // numbers above are apart.
            ("records", T::at_in_window(E::field("header_size"), T::sized(E::field("type_record_bytes"), T::repeat(type_record(), Until::End)))),
        ],
    )
}

/// One CodeView type record: how long it is, what kind it is, and its bytes.
///
/// The kind is a number and not a name. Naming them is a table of a hundred
/// record shapes, each with its own fields, and it is the next thing to write
/// here rather than something the IR cannot hold.
fn type_record() -> T {
    T::structure(
        "TypeRecord",
        vec![
            // The length does not count itself, which is what makes a record
            // two bytes longer than it says.
            ("length", T::u16(Little)),
            ("kind", T::u16(Little)),
            ("data", T::bytes(E::field("length").sub(E::lit(2)))),
        ],
    )
    .counted_as("record")
}

/// The debug information stream: which streams hold the symbols, what the
/// linker was, and the seven substreams that follow its header.
///
/// The substreams are placed and not read. Each is a table of its own with its
/// own record shapes, and the module list in the first of them is the way in
/// to a module's symbols and its line numbers, which is the largest thing left
/// undone here.
fn dbi_stream() -> T {
    T::structure(
        "DbiStream",
        vec![
            // Always -1, which is how a reader tells this header from the one
            // the format had before it had a version number.
            ("version_signature", T::i32(Little)),
            ("version", T::enumeration("DbiVersion", T::u32(Little), DBI_VERSIONS)),
            ("age", T::u32(Little)),
            ("global_symbols_stream", T::u16(Little)),
            // The linker's version, packed: the major and minor number, and a
            // bit that says the fields after it are the new arrangement.
            ("build_number", T::u16(Little)),
            ("public_symbols_stream", T::u16(Little)),
            ("pdb_dll_version", T::u16(Little)),
            ("symbol_records_stream", T::u16(Little)),
            ("pdb_dll_rebuild", T::u16(Little)),
            ("module_info_size", T::i32(Little)),
            ("section_contribution_size", T::i32(Little)),
            ("section_map_size", T::i32(Little)),
            ("source_info_size", T::i32(Little)),
            ("type_server_map_size", T::i32(Little)),
            ("mfc_type_server_index", T::u32(Little)),
            ("optional_debug_header_size", T::i32(Little)),
            ("edit_and_continue_size", T::i32(Little)),
            ("flags", T::flags("DbiFlags", T::u16(Little), DBI_FLAGS)),
            ("machine", T::enumeration("Machine", T::u16(Little), MACHINE)),
            ("padding", T::u32(Little)),
            ("module_info", T::bytes(E::field("module_info_size"))),
            ("section_contributions", T::bytes(E::field("section_contribution_size"))),
            ("section_map", T::bytes(E::field("section_map_size"))),
            ("source_info", T::bytes(E::field("source_info_size"))),
            ("type_server_map", T::bytes(E::field("type_server_map_size"))),
            ("edit_and_continue", T::bytes(E::field("edit_and_continue_size"))),
            ("optional_debug_header", T::bytes(E::field("optional_debug_header_size"))),
        ],
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
                // Both are 16-bit here, which is what caps this container at a
                // few hundred megabytes and why it was replaced.
                ("free_page_map", T::u16(Little)),
                ("page_count", T::u16(Little)),
                ("directory_bytes", T::u32(Little)),
                ("directory_unused", T::u32(Little)),
                ("claimed_size", T::computed(size().mul(E::field("page_count")))),
                // The directory's pages are listed here in the header itself
                // rather than behind a map of their own.
                ("directory_pages", T::array(page_ref(), blocks_for(E::field("directory_bytes"), size()))),
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
    const DIRECTORY: usize = 11;

    /// The streams the fixture holds: the old directory, the info stream, an
    /// empty one, a number that is not in use, and one whose blocks are in the
    /// wrong order to be read where they lie.
    const SIZES: [u32; 5] = [0x20, 84, 0, 0xffff_ffff, 700];
    const BLOCKS: [&[u32]; 5] = [&[4], &[5], &[], &[], &[7, 6]];

    /// An info stream: the four numbers a debugger matches a PDB by, two named
    /// streams, and one feature code.
    fn info() -> Vec<u8> {
        let mut v = 20000404u32.to_le_bytes().to_vec();
        v.extend_from_slice(&0x5a2bu32.to_le_bytes());
        v.extend_from_slice(&3u32.to_le_bytes());
        v.extend_from_slice(&[0x11; 16]);
        v.extend_from_slice(&12u32.to_le_bytes());
        v.extend_from_slice(b"\0/names\0abc\0");
        v.extend_from_slice(&2u32.to_le_bytes());
        v.extend_from_slice(&4u32.to_le_bytes());
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend_from_slice(&3u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        for (name_offset, stream) in [(1u32, 5u32), (8, 9)] {
            v.extend_from_slice(&name_offset.to_le_bytes());
            v.extend_from_slice(&stream.to_le_bytes());
        }
        v.extend_from_slice(&20091201u32.to_le_bytes());
        assert_eq!(v.len() as u32, SIZES[1]);
        v
    }

    /// A PDB laid out the way a compiler lays one out: the superblock, the two
    /// free-block maps, the directory, the streams' blocks, and the block map.
    ///
    /// `directory_bytes` and `map` are the two things a test varies, since
    /// what changes between a directory that reads and one that does not is
    /// how many blocks it claims and where they are.
    fn pdb_file(directory_bytes: Option<u32>, map: &[u32]) -> Vec<u8> {
        let mut directory = (SIZES.len() as u32).to_le_bytes().to_vec();
        for size in SIZES {
            directory.extend_from_slice(&size.to_le_bytes());
        }
        for list in BLOCKS {
            for block in list {
                directory.extend_from_slice(&block.to_le_bytes());
            }
        }

        let mut v = MAGIC.to_vec();
        v.extend_from_slice(&(BLOCK as u32).to_le_bytes());
        v.extend_from_slice(&2u32.to_le_bytes());
        v.extend_from_slice(&9u32.to_le_bytes());
        v.extend_from_slice(&directory_bytes.unwrap_or(directory.len() as u32).to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&8u32.to_le_bytes());
        v.resize(BLOCK * 9, 0);
        let write = |v: &mut Vec<u8>, block: usize, bytes: &[u8]| {
            v[BLOCK * block..BLOCK * block + bytes.len()].copy_from_slice(bytes);
        };
        write(&mut v, 3, &directory);
        write(&mut v, 5, &info());
        for (i, block) in map.iter().enumerate() {
            write(&mut v, 8, &[]);
            v[BLOCK * 8 + i * 4..BLOCK * 8 + i * 4 + 4].copy_from_slice(&block.to_le_bytes());
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
        assert_eq!(e.node(&d, &[BLOCK_COUNT]).unwrap().value.as_int(), Some(9));
        assert_eq!(e.node(&d, &[CLAIMED]).unwrap().value.as_int(), Some(BLOCK as i128 * 9));
        // One block holds the directory, and the map says which.
        assert_eq!(e.node(&d, &[BLOCK_MAP, 0]).unwrap().child_count, 1);
        assert_eq!(e.node(&d, &[BLOCK_MAP, 0, 0]).unwrap().value.as_int(), Some(3));
        // Five streams, the first of them the old directory.
        assert_eq!(e.node(&d, &[DIRECTORY, 0, 0]).unwrap().value.as_int(), Some(5));
        assert_eq!(e.node(&d, &[DIRECTORY, 0, 2]).unwrap().child_count, 5);
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
        assert_eq!(blocks(&mut e, 2), 0);
        assert_eq!(blocks(&mut e, 3), 0);
        // 700 bytes is two blocks of 512, and the second of them is block 6.
        assert_eq!(blocks(&mut e, 4), 2);
        assert_eq!(e.node(&d, &[DIRECTORY, 0, 2, 4, 1]).unwrap().value.as_int(), Some(700));
        assert_eq!(e.node(&d, &[DIRECTORY, 0, 2, 4, 2, 1]).unwrap().value.as_int(), Some(6));
    }

    /// A stream whose blocks are one run is read where it lies, and the info
    /// stream reads as the four numbers and the names it holds.
    #[test]
    fn a_stream_in_one_run_is_read_where_it_lies() {
        let d = Document::new(MemSource(pdb_file(None, &[3])));
        let mut e = Evaluator::new(pdb());
        let info = [DIRECTORY, 0, 2, 1, 4, 0];
        let at = |field: usize| [info.as_slice(), &[field]].concat();
        assert_eq!(e.node(&d, &at(0)).unwrap().value, Value::Enum {
            raw: 20000404,
            name: Some("VC80".into()),
            hex: false
        });
        assert_eq!(e.node(&d, &at(2)).unwrap().value.as_int(), Some(3));
        assert_eq!(e.node(&d, &at(3)).unwrap().size_bits, 16 * 8);
        // The buffer holds the empty string an index of zero means, and the
        // two names after it.
        assert_eq!(e.node(&d, &[at(5).as_slice(), &[0]].concat()).unwrap().child_count, 3);
        assert_eq!(
            e.node(&d, &[at(5).as_slice(), &[0, 1]].concat()).unwrap().value,
            Value::Str("/names".into())
        );
        // Two named streams, the second of them stream 9.
        assert_eq!(e.node(&d, &at(12)).unwrap().child_count, 2);
        assert_eq!(e.node(&d, &[at(12).as_slice(), &[1, 1]].concat()).unwrap().value.as_int(), Some(9));
        assert_eq!(e.node(&d, &at(13)).unwrap().child_count, 1);
        // The whole stream is read: it ends where its length said.
        assert_eq!(e.node(&d, &info).unwrap().size_bits, u64::from(SIZES[1]) * 8);
    }

    /// A stream whose blocks hold the right blocks in the wrong order is not
    /// read where it lies, because it does not lie anywhere: the check is on
    /// the order as well as the numbers.
    #[test]
    fn a_stream_out_of_order_is_its_block_list_and_nothing_else() {
        let d = Document::new(MemSource(pdb_file(None, &[3])));
        let mut e = Evaluator::new(pdb());
        let contents = e.node(&d, &[DIRECTORY, 0, 2, 4, 4]).unwrap();
        assert_eq!(contents.size_bits, 0);
        assert_eq!(contents.child_count, 0);
    }

    /// A directory spread over blocks that are not one run stays those blocks:
    /// the table's own fields would be cut in half by a boundary, and a count
    /// read across one would be a number nobody wrote.
    #[test]
    fn a_scattered_directory_reads_as_its_blocks() {
        let d = Document::new(MemSource(pdb_file(Some(600), &[3, 6])));
        let mut e = Evaluator::new(pdb());
        assert_eq!(e.node(&d, &[BLOCK_MAP, 0]).unwrap().child_count, 2);
        // The blocks, read again as the offsets they work out to, and the
        // bytes at each one.
        assert_eq!(e.node(&d, &[DIRECTORY, 0, 0]).unwrap().child_count, 2);
        assert_eq!(e.node(&d, &[DIRECTORY, 0, 0, 1, 1]).unwrap().value.as_int(), Some(BLOCK as i128 * 6));
        assert_eq!(e.node(&d, &[DIRECTORY, 1]).unwrap().child_count, 2);
        assert_eq!(e.node(&d, &[DIRECTORY, 1, 0]).unwrap().size_bits, BLOCK as u64 * 8);
    }

    /// A directory over blocks that *are* one run reads, however many they
    /// are: two blocks side by side are a run of bytes like any other.
    #[test]
    fn a_directory_of_two_blocks_side_by_side_still_reads() {
        let d = Document::new(MemSource(pdb_file(Some(600), &[3, 4])));
        let mut e = Evaluator::new(pdb());
        assert_eq!(e.node(&d, &[DIRECTORY, 0, 0]).unwrap().value.as_int(), Some(5));
        assert_eq!(e.node(&d, &[DIRECTORY, 0]).unwrap().size_bits, 600 * 8);
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
