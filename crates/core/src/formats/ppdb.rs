//! Portable PDB: the debug symbols a .NET build writes, which are ECMA-335
//! metadata in a file of their own rather than a paged container.
//!
//! Nothing about this is the PDB next door to it. A native program database
//! is a small file system of blocks ([`super::pdb`]); this is the same
//! metadata format an assembly carries inside its PE image, written out on its
//! own and holding the tables that say which source file and which line a
//! method's instructions came from. A reader meeting a `.pdb` has to look at
//! the first four bytes to know which of the two it has, and `BSJB` is this
//! one. Roslyn writes it, Mono reads it, and it is the same on every platform
//! the runtime runs on, which is the whole reason it exists.
//!
//! The shape is a root, a table of streams and the streams themselves. Every
//! stream's offset counts from the front of the metadata root, which for a
//! standalone file is the front of the file, so all of it is where it says it
//! is and the whole thing reads without a hop the IR cannot make.
//!
//! Six streams, and this reads what each one is rather than leaving them as
//! bytes: `#Pdb` says which build these symbols belong to and how many rows
//! the assembly's own tables had, `#~` is the table stream and its header
//! names every table present and how many rows each one has, `#Strings` is a
//! run of C strings and reads as them, `#GUID` is a list of 16-byte ids.
//!
//! **Where this stops.** The rows themselves. A metadata table's columns are
//! as wide as the file needs them to be: an index into a heap is two bytes or
//! four depending on how large that heap is, and an index into another table
//! is two or four depending on how many rows *it* has, with a coded index
//! spending its low bits on which of several tables it means. So a row's
//! layout is a function of the header above it, which the IR can express, and
//! of a set of rules about which tables a coded index may name, which it
//! cannot. The header is read and the rows are the bytes after it.

use crate::template::{Anchor, Endian::Little, Expr as E, StrLen, Template, Ty as T};

/// The four letters at the front of a metadata root, and the version string
/// that says these are debug symbols rather than an assembly's own metadata
/// written out. Both are needed: the root itself is what a `.dll` keeps
/// inside it, and only the string tells the two apart at offset zero.
pub const MAGIC: &[u8] = b"BSJB\x01\x00\x01\x00\x00\x00\x00\x00\x0c\x00\x00\x00PDB v1.0";

/// Every metadata table, by the bit that says it is present. The type system's
/// own tables are here because a Portable PDB may carry rows of them, and the
/// last eight are the ones that only debug symbols have.
const TABLES: &[(u32, &str)] = &[
    (0x00, "Module"),
    (0x01, "TypeRef"),
    (0x02, "TypeDef"),
    (0x03, "FieldPtr"),
    (0x04, "Field"),
    (0x05, "MethodPtr"),
    (0x06, "MethodDef"),
    (0x07, "ParamPtr"),
    (0x08, "Param"),
    (0x09, "InterfaceImpl"),
    (0x0a, "MemberRef"),
    (0x0b, "Constant"),
    (0x0c, "CustomAttribute"),
    (0x0d, "FieldMarshal"),
    (0x0e, "DeclSecurity"),
    (0x0f, "ClassLayout"),
    (0x10, "FieldLayout"),
    (0x11, "StandAloneSig"),
    (0x12, "EventMap"),
    (0x13, "EventPtr"),
    (0x14, "Event"),
    (0x15, "PropertyMap"),
    (0x16, "PropertyPtr"),
    (0x17, "Property"),
    (0x18, "MethodSemantics"),
    (0x19, "MethodImpl"),
    (0x1a, "ModuleRef"),
    (0x1b, "TypeSpec"),
    (0x1c, "ImplMap"),
    (0x1d, "FieldRVA"),
    (0x1e, "EncLog"),
    (0x1f, "EncMap"),
    (0x20, "Assembly"),
    (0x21, "AssemblyProcessor"),
    (0x22, "AssemblyOS"),
    (0x23, "AssemblyRef"),
    (0x24, "AssemblyRefProcessor"),
    (0x25, "AssemblyRefOS"),
    (0x26, "File"),
    (0x27, "ExportedType"),
    (0x28, "ManifestResource"),
    (0x29, "NestedClass"),
    (0x2a, "GenericParam"),
    (0x2b, "MethodSpec"),
    (0x2c, "GenericParamConstraint"),
    (0x30, "Document"),
    (0x31, "MethodDebugInformation"),
    (0x32, "LocalScope"),
    (0x33, "LocalVariable"),
    (0x34, "LocalConstant"),
    (0x35, "ImportScope"),
    (0x36, "StateMachineMethod"),
    (0x37, "CustomDebugInformation"),
];

/// Which heaps are indexed with four bytes rather than two, which is what
/// makes a row's width a property of the file rather than of the format.
const HEAP_SIZES: &[(u32, &str)] =
    &[(0, "wide string indexes"), (1, "wide guid indexes"), (2, "wide blob indexes")];

pub fn ppdb() -> Template {
    Template::new(
        "portablepdb",
        T::origin(T::structure(
            "MetadataRoot",
            vec![
                ("magic", T::magic(b"BSJB")),
                ("major_version", T::u16(Little)),
                ("minor_version", T::u16(Little)),
                ("reserved", T::u32(Little)),
                // The length counts the padding that keeps what follows on a
                // four-byte boundary, so the string is shorter than the field.
                ("version_length", T::u32(Little)),
                ("version", T::text(StrLen::Padded { size: E::field("version_length"), pad: 0 }, crate::template::Encoding::Utf8)),
                ("flags", T::u16(Little)),
                ("stream_count", T::u16(Little)),
                ("streams", T::array(stream_header(), E::field("stream_count"))),
                // Each stream where its header said, read as whatever that
                // header called it. Declared last, as a list placed by offsets
                // has to be.
                (
                    "contents",
                    T::pointer_list_sized("streams", &["offset"], Anchor::Origin, E::lit(0), stream()),
                ),
            ],
        )),
    )
}

/// One entry of the table of streams: where the stream is, how long it is, and
/// its name, which is what says how to read it.
fn stream_header() -> T {
    T::structure_named(
        "MetadataStream",
        "name",
        "",
        vec![
            ("offset", T::u32(Little)),
            ("size", T::u32(Little)),
            ("name", T::cstr()),
            // To the next four-byte boundary, so that the next entry starts on
            // one. Nothing when the name already ended on one.
            ("padding", T::bytes(E::size_of("name").pad_to(4))),
        ],
    )
    .counted_as("stream")
}

/// A stream, read as what its name says it holds.
///
/// The name is the type, which is why this is a match on text rather than a
/// switch on a number: the format has no number for what a stream is, and a
/// writer may leave any of them out.
fn stream() -> T {
    let size = || E::elem_field("streams", E::idx(), &["size"]);
    T::sized(
        size(),
        T::matches(
            E::elem_field("streams", E::idx(), &["name"]),
            vec![
                ("#Pdb", pdb_stream()),
                ("#~", tables()),
                // The string heap: C strings one after another, the first of
                // them empty so that an index of zero means no name.
                ("#Strings", T::structure("StringHeap", vec![("strings", T::repeat(T::cstr(), crate::template::Until::End))])),
                (
                    "#GUID",
                    T::structure("GuidHeap", vec![("guids", T::array(T::bytes(E::lit(16)), E::Remaining.div(E::lit(16))))]),
                ),
                // The user string heap and the blob heap are both runs of
                // length-prefixed bytes, and what a blob holds is decided by
                // whichever row points at it rather than by the heap.
                ("#US", T::structure("UserStringHeap", vec![("blobs", T::bytes(E::Remaining))])),
                ("#Blob", T::structure("BlobHeap", vec![("blobs", T::bytes(E::Remaining))])),
            ],
            T::bytes(E::Remaining),
        ),
    )
}

/// The `#Pdb` stream: which build these symbols are for, where the program
/// starts, and how large the assembly's own tables were when they were
/// written, which is what lets a row here index a row there.
fn pdb_stream() -> T {
    T::structure(
        "PdbStream",
        vec![
            // The same twenty bytes the assembly's debug directory holds, and
            // what a debugger matches the two by.
            ("id", T::bytes(E::lit(20))),
            // A row of MethodDef, or zero for an assembly that is a library.
            ("entry_point", T::u32(Little)),
            ("referenced_tables", T::flags("MetadataTables", T::u64(Little), TABLES)),
            ("referenced_rows", T::array(T::u32(Little), E::pop_count("referenced_tables"))),
        ],
    )
}

/// The `#~` stream's header: which tables are here and how many rows each one
/// has. The rows are the bytes after it; see the module docs for why.
fn tables() -> T {
    T::structure(
        "TableStream",
        vec![
            ("reserved", T::u32(Little)),
            ("major_version", T::u8()),
            ("minor_version", T::u8()),
            ("heap_sizes", T::flags("HeapSizes", T::u8(), HEAP_SIZES)),
            ("reserved2", T::u8()),
            ("valid", T::flags("MetadataTables", T::u64(Little), TABLES)),
            ("sorted", T::flags("MetadataTables", T::u64(Little), TABLES)),
            ("rows", T::array(T::u32(Little), E::pop_count("valid"))),
            ("tables", T::bytes(E::Remaining)),
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

    /// Field indices into the root: the table of streams, and the streams.
    const STREAMS: usize = 8;
    const CONTENTS: usize = 9;

    /// A metadata root holding the streams it is given, each at the offset it
    /// works out to. The names decide their own padding, which is what the
    /// template has to get right to read the second entry at all.
    fn metadata(streams: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let padded = |name: &str| (name.len() + 1).next_multiple_of(4);
        let mut at = 0x20 + streams.iter().map(|(n, _)| 8 + padded(n)).sum::<usize>();

        let mut v = b"BSJB".to_vec();
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&12u32.to_le_bytes());
        v.extend_from_slice(b"PDB v1.0\0\0\0\0");
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&(streams.len() as u16).to_le_bytes());
        for (name, body) in streams {
            v.extend_from_slice(&(at as u32).to_le_bytes());
            v.extend_from_slice(&(body.len() as u32).to_le_bytes());
            v.extend_from_slice(name.as_bytes());
            v.resize(v.len() + padded(name) - name.len(), 0);
            at += body.len();
        }
        for (_, body) in streams {
            v.extend_from_slice(body);
        }
        v
    }

    /// The `#Pdb` stream of a program whose assembly had two tables.
    fn pdb_body() -> Vec<u8> {
        let mut v = vec![0xab; 20];
        v.extend_from_slice(&6u32.to_le_bytes());
        v.extend_from_slice(&((1u64 << 0) | (1 << 6)).to_le_bytes());
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend_from_slice(&40u32.to_le_bytes());
        v
    }

    /// The `#~` stream of a Portable PDB: two of the debug tables, the rows
    /// they have, and the rows themselves.
    fn tables_body() -> Vec<u8> {
        let mut v = vec![0, 0, 0, 0, 2, 0, 4, 1];
        v.extend_from_slice(&(((1u64 << 0x30) | (1 << 0x31)) as u64).to_le_bytes());
        v.extend_from_slice(&0u64.to_le_bytes());
        v.extend_from_slice(&2u32.to_le_bytes());
        v.extend_from_slice(&5u32.to_le_bytes());
        v.extend_from_slice(&[0x11; 12]);
        v
    }

    fn sample() -> Vec<u8> {
        metadata(&[
            ("#Pdb", pdb_body()),
            ("#~", tables_body()),
            ("#Strings", b"\0alpha\0beta\0".to_vec()),
            ("#GUID", vec![0x22; 32]),
            ("#Junk", vec![0x33; 8]),
        ])
    }

    /// The table of streams reads, padding and all, and every stream is where
    /// its entry said it was.
    #[test]
    fn the_streams_are_found_by_the_table_at_the_front() {
        let d = Document::new(MemSource(sample()));
        let mut e = Evaluator::new(ppdb());
        assert_eq!(e.node(&d, &[5]).unwrap().value, Value::Str("PDB v1.0".into()));
        assert_eq!(e.node(&d, &[STREAMS]).unwrap().child_count, 5);
        assert_eq!(e.node(&d, &[STREAMS, 1, 2]).unwrap().value, Value::Str("#~".into()));
        // The second entry's name is two characters and a nul, so one byte of
        // padding follows it; getting that wrong misreads every entry after.
        assert_eq!(e.node(&d, &[STREAMS, 1, 3]).unwrap().size_bits, 8);
        assert_eq!(e.node(&d, &[STREAMS, 2, 2]).unwrap().value, Value::Str("#Strings".into()));
        assert_eq!(e.node(&d, &[CONTENTS]).unwrap().child_count, 5);
    }

    /// A stream is read as its name says: the id and the referenced tables of
    /// `#Pdb`, the header of `#~`, and the strings of `#Strings`.
    #[test]
    fn a_stream_is_read_by_the_name_its_entry_gave_it() {
        let d = Document::new(MemSource(sample()));
        let mut e = Evaluator::new(ppdb());
        // One row count per table the assembly had, counted from the bits.
        assert_eq!(e.node(&d, &[CONTENTS, 0, 1]).unwrap().value.as_int(), Some(6));
        assert_eq!(e.node(&d, &[CONTENTS, 0, 3]).unwrap().child_count, 2);
        assert_eq!(e.node(&d, &[CONTENTS, 0, 3, 1]).unwrap().value.as_int(), Some(40));
        // The table stream: two tables, two row counts, and the rows after.
        assert_eq!(e.node(&d, &[CONTENTS, 1, 7]).unwrap().child_count, 2);
        assert_eq!(e.node(&d, &[CONTENTS, 1, 7, 1]).unwrap().value.as_int(), Some(5));
        assert_eq!(e.node(&d, &[CONTENTS, 1, 8]).unwrap().size_bits, 12 * 8);
        // The string heap opens with the empty string, so that an index of
        // zero means no name.
        assert_eq!(e.node(&d, &[CONTENTS, 2, 0]).unwrap().child_count, 3);
        assert_eq!(e.node(&d, &[CONTENTS, 2, 0, 1]).unwrap().value, Value::Str("alpha".into()));
        assert_eq!(e.node(&d, &[CONTENTS, 3, 0]).unwrap().child_count, 2);
    }

    /// A stream nothing here knows the name of stays the bytes it is, rather
    /// than being read as whichever shape came last.
    #[test]
    fn an_unknown_stream_stays_bytes() {
        let d = Document::new(MemSource(sample()));
        let mut e = Evaluator::new(ppdb());
        let junk = e.node(&d, &[CONTENTS, 4]).unwrap();
        assert_eq!(junk.size_bits, 8 * 8);
        assert!(matches!(junk.value, Value::Bytes { .. }));
    }
}
