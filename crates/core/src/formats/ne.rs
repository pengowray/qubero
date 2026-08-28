//! New Executable: 16-bit Windows and OS/2 programs and libraries.
//!
//! The format between MS-DOS and the PE. A file opens as a DOS program whose
//! whole job is to say the program needs Windows, and the header at the offset
//! that stub's own header holds says `NE` rather than `PE`.
//!
//! What makes it worth reading now is that everything in it is a segment: the
//! program is a list of them, each saying in its flags whether it holds code
//! or data, and the code is the 16-bit x86 the decoder here already reads. A
//! segment's place in the file is written in units the header names, as a
//! count of sectors and the number of bits to shift it by, which is why the
//! IR grew a shift.
//!
//! The tables are all offsets from the start of the NE header rather than from
//! the start of the file, with one exception: the non-resident name table,
//! which counts from the front of the file. That inconsistency is in the
//! format, not here.

use crate::code::Isa;
use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};

/// What the module is, in the header's flags word.
const FORMAT_FLAGS: &[(u32, &str)] = &[
    (0, "single data"),
    (1, "multiple data"),
    (4, "win32s"),
    (8, "framebuffer"),
    (9, "console"),
    (11, "self loading"),
    (13, "link error"),
    (14, "calls WEP"),
    (15, "library"),
];

/// Which system the module is built for. A file for none of them is usually a
/// driver, which is loaded by something that already knows what it is.
const TARGET_OS: &[(i128, &str)] = &[
    (0, "unknown"),
    (1, "OS/2"),
    (2, "Windows"),
    (3, "European MS-DOS 4.x"),
    (4, "Windows 386"),
    (5, "Borland Operating System Services"),
];

/// A segment's flags. The first bit is the one that decides everything else
/// here: whether the bytes are instructions or data.
const SEGMENT_FLAGS: &[(u32, &str)] = &[
    (0, "data"),
    (1, "allocated"),
    (2, "loaded"),
    (3, "iterated"),
    (4, "moveable"),
    (5, "shareable"),
    (6, "preload"),
    (7, "read only or execute only"),
    (8, "has relocations"),
    (11, "self loading"),
    (12, "discardable"),
    (13, "32-bit"),
];

const RESOURCE_FLAGS: &[(u32, &str)] = &[(4, "moveable"), (5, "shareable"), (6, "preload")];

/// The resource types Windows numbers itself. A type whose top bit is clear is
/// one of these; with it set, the number is an offset to a name instead.
const RESOURCE_TYPE: &[(i128, &str)] = &[
    (0x8001, "cursor"),
    (0x8002, "bitmap"),
    (0x8003, "icon"),
    (0x8004, "menu"),
    (0x8005, "dialog"),
    (0x8006, "string table"),
    (0x8007, "font directory"),
    (0x8008, "font"),
    (0x8009, "accelerator table"),
    (0x800a, "resource data"),
    (0x800b, "message table"),
    (0x800c, "group cursor"),
    (0x800e, "group icon"),
    (0x8010, "version"),
];

pub fn ne() -> Template {
    // The DOS program in front, which exists to print that this one needs
    // Windows. The same fields the `msdos` template reads, and the same
    // pointer a PE keeps at 0x3c.
    let mut dos_fields = super::dos::header_fields();
    dos_fields.extend(vec![
        ("reserved", T::bytes(E::lit(8))),
        ("oem_id", T::u16(Little)),
        ("oem_info", T::u16(Little)),
        ("reserved2", T::bytes(E::lit(20))),
        ("ne_header_offset", T::u32(Little)),
        ("dos_stub", T::bytes(E::field("ne_header_offset").sub(E::lit(64)))),
    ]);
    let dos = T::structure("DOSHeader", dos_fields);

    Template::new("ne", T::structure("NE", vec![("dos", dos), ("ne", header())]))
}

fn header() -> T {
    T::structure(
        "NEHeader",
        vec![
            ("signature", T::magic(b"NE")),
            ("linker_version", T::u8()),
            ("linker_revision", T::u8()),
            ("entry_table_offset", T::u16(Little)),
            ("entry_table_size", T::u16(Little)),
            ("crc", T::u32(Little)),
            ("flags", T::flags("ModuleFlags", T::u16(Little), FORMAT_FLAGS)),
            // Which segment holds the module's own data, counted from one.
            ("auto_data_segment", T::u16(Little)),
            ("heap_size", T::u16(Little)),
            ("stack_size", T::u16(Little)),
            ("entry_ip", T::u16(Little)),
            ("entry_segment", T::u16(Little)),
            ("stack_pointer", T::u16(Little)),
            ("stack_segment", T::u16(Little)),
            ("segment_count", T::u16(Little)),
            ("module_reference_count", T::u16(Little)),
            ("nonresident_names_size", T::u16(Little)),
            ("segment_table_offset", T::u16(Little)),
            ("resource_table_offset", T::u16(Little)),
            ("resident_names_offset", T::u16(Little)),
            ("module_reference_offset", T::u16(Little)),
            ("imported_names_offset", T::u16(Little)),
            // The one table counted from the start of the file rather than
            // from the start of this header.
            ("nonresident_names_offset", T::u32(Little)),
            ("moveable_entry_count", T::u16(Little)),
            // How many bits to shift a segment's position by. Two of these
            // files in three write 4, so a segment starts on a paragraph.
            ("sector_shift", T::u16(Little)),
            ("resource_segment_count", T::u16(Little)),
            ("target_os", T::enumeration("TargetOS", T::u8(), TARGET_OS)),
            ("other_flags", T::u8()),
            ("return_thunks_offset", T::u16(Little)),
            ("segment_reference_thunks_offset", T::u16(Little)),
            ("code_swap_area_size", T::u16(Little)),
            ("expected_windows_minor", T::u8()),
            ("expected_windows_major", T::u8()),
            // Everything below is read where the header says it is, without
            // moving the cursor, so the order the tables are written in is
            // the file's business rather than this template's.
            ("segments", at_header(E::field("segment_table_offset"), T::array(segment(), E::field("segment_count")))),
            ("resources", resources()),
            ("resident_names", at_header(E::field("resident_names_offset"), names("ResidentName"))),
            (
                "module_references",
                at_header(
                    E::field("module_reference_offset"),
                    T::array(T::u16(Little), E::field("module_reference_count")),
                ),
            ),
            // Nothing says how long this table is. What ends it is the table
            // after it, which the header does say where to find.
            (
                "imported_names",
                at_header(
                    E::field("imported_names_offset"),
                    T::sized(E::field("entry_table_offset").sub(E::field("imported_names_offset")), imported_names()),
                ),
            ),
            (
                "entry_table",
                at_header(E::field("entry_table_offset"), T::sized(E::field("entry_table_size"), entry_bundles())),
            ),
            (
                "nonresident_names",
                T::at(E::field("nonresident_names_offset"), T::sized(E::field("nonresident_names_size"), names("NonResidentName"))),
            ),
        ],
    )
}

/// A table at an offset counted from the start of the NE header, which is
/// itself at an offset the DOS header at the front of the file holds. Every
/// table in the format is written this way but one.
fn at_header(at: E, inner: T) -> T {
    T::at(E::within(&["dos", "ne_header_offset"]).add(at), inner)
}

/// One segment: where it is, how long it is, and what it holds. The position
/// is in sectors of whatever size the header's shift count says, and a zero
/// there means a segment with nothing in the file at all.
fn segment() -> T {
    T::structure(
        "Segment",
        vec![
            ("sector", T::u16(Little)),
            // Zero means 64K, which is as much as a 16-bit segment holds.
            ("length", T::u16(Little)),
            ("flags", T::flags("SegmentFlags", T::u16(Little), SEGMENT_FLAGS)),
            ("minimum_size", T::u16(Little)),
            (
                "contents",
                T::switch(
                    E::field("sector"),
                    vec![(0, T::bytes(E::lit(0)))],
                    T::at(E::field("sector").shl(E::field("sector_shift")), body()),
                ),
            ),
        ],
    )
    .counted_as("segment")
}

/// What is in a segment: its bytes, read as instructions when its flags say it
/// holds code, and the relocations after them when its flags say there are
/// any.
fn body() -> T {
    let size = || E::field("length").or(E::lit(0x1_0000));
    let code = T::switch(
        // The bit that says the segment is 32-bit code, which an OS/2 or a
        // Windows 386 module has and a plain Windows one does not.
        E::field("flags").bit(13),
        vec![(1, T::sized(size(), T::repeat(T::insn(Isa::X86_32), Until::End)))],
        T::sized(size(), T::repeat(T::insn(Isa::X86_16), Until::End)),
    );
    T::structure(
        "SegmentBody",
        vec![
            ("bytes", T::switch(E::field("flags").bit(0), vec![(1, T::bytes(size()))], code)),
            (
                "relocations",
                T::switch(
                    E::field("flags").bit(8),
                    vec![(
                        1,
                        T::structure(
                            "Relocations",
                            vec![
                                ("count", T::u16(Little)),
                                ("entries", T::array(relocation(), E::field("count"))),
                            ],
                        ),
                    )],
                    T::bytes(E::lit(0)),
                ),
            ),
        ],
    )
}

/// One relocation: what kind of address to patch, where in the segment it is,
/// and what to put there. The last four bytes mean different things for each
/// kind of target, which is why they stay two numbers here.
fn relocation() -> T {
    T::structure(
        "Relocation",
        vec![
            (
                "address_type",
                T::enumeration(
                    "AddressType",
                    T::u8(),
                    &[(0, "low byte"), (2, "segment"), (3, "far pointer"), (5, "offset"), (11, "48-bit pointer"), (13, "32-bit offset")],
                ),
            ),
            (
                "type",
                T::enumeration(
                    "RelocationType",
                    T::u8(),
                    &[(0, "internal reference"), (1, "imported ordinal"), (2, "imported name"), (3, "OS fixup")],
                ),
            ),
            ("offset", T::u16(Little)),
            ("target1", T::u16(Little)),
            ("target2", T::u16(Little)),
        ],
    )
    .counted_as("relocation")
}

/// The resource table: an alignment shift of its own, then a run of type
/// blocks that ends with a type of zero.
fn resources() -> T {
    T::switch(
        E::field("resource_table_offset"),
        vec![(0, T::bytes(E::lit(0)))],
        at_header(
            E::field("resource_table_offset"),
            T::structure(
                "ResourceTable",
                vec![
                    ("shift", T::u16(Little)),
                    ("types", T::repeat(resource_type(), Until::FieldBytes { field: "type".into(), bytes: vec![0, 0] })),
                ],
            ),
        ),
    )
}

fn resource_type() -> T {
    T::structure(
        "ResourceType",
        vec![
            // With the top bit set the number names one of the types Windows
            // knows; without it, it is an offset to a name in this table.
            ("type", T::enumeration_hex("ResourceKind", T::u16(Little), RESOURCE_TYPE)),
            // What ends the table is a type of zero and nothing after it: two
            // bytes where every other entry is eight and then a run of
            // resources. Reading the rest of an entry that is not there is
            // what would run off the end of the table.
            (
                "entries",
                T::switch(
                    E::field("type"),
                    vec![(0, T::bytes(E::lit(0)))],
                    T::structure(
                        "ResourcesOfType",
                        vec![
                            ("count", T::u16(Little)),
                            ("reserved", T::u32(Little)),
                            ("resources", T::array(resource(), E::field("count"))),
                        ],
                    ),
                ),
            ),
        ],
    )
    .counted_as("resource type")
}

fn resource() -> T {
    T::structure(
        "Resource",
        vec![
            // In units of the table's own shift, which is not the header's.
            ("sector", T::u16(Little)),
            ("length", T::u16(Little)),
            ("flags", T::flags("ResourceFlags", T::u16(Little), RESOURCE_FLAGS)),
            ("id", T::u16(Little)),
            ("handle", T::u16(Little)),
            ("usage", T::u16(Little)),
            (
                "bytes",
                T::at(
                    E::field("sector").shl(E::sibling(&["shift"])),
                    T::bytes(E::field("length").shl(E::sibling(&["shift"]))),
                ),
            ),
        ],
    )
    .counted_as("resource")
}

/// A table of names, each a length byte, that many characters, and the ordinal
/// the name stands for. A length of zero ends the table; the first name is the
/// module's own and has no ordinal worth reading.
fn names(name: &str) -> T {
    T::repeat(
        T::structure_named(
            name,
            "text",
            "",
            vec![
                ("length", T::u8()),
                ("text", T::text(StrLen::Fixed(E::field("length")), Encoding::Ascii)),
                ("ordinal", T::switch(E::field("length"), vec![(0, T::bytes(E::lit(0)))], T::u16(Little))),
            ],
        )
        .counted_as("name"),
        Until::FieldBytes { field: "length".into(), bytes: vec![0] },
    )
}

/// The names of the modules this one imports from, and of the functions it
/// imports by name. Every entry is a length byte and that many characters, and
/// what refers to one is its offset into the table rather than its number, so
/// the table is read as the run of strings it is.
fn imported_names() -> T {
    T::repeat(
        T::structure_named(
            "ImportedName",
            "text",
            "",
            vec![("length", T::u8()), ("text", T::text(StrLen::Fixed(E::field("length")), Encoding::Ascii))],
        ),
        Until::End,
    )
}

/// The entry table: bundles of entry points, each bundle saying how many it
/// holds and which segment they are in. A bundle of none ends the table.
fn entry_bundles() -> T {
    T::repeat(
        T::structure(
            "EntryBundle",
            vec![
                ("count", T::u8()),
                // A bundle of none is the end of the table, and is that one
                // byte: what would be the segment is the next table's.
                (
                    "bundle",
                    T::switch(
                        E::field("count"),
                        vec![(0, T::bytes(E::lit(0)))],
                        T::structure(
                            "Bundle",
                            vec![
                                // Zero means the numbers this bundle stands
                                // for are unused, 0xff means the entries can
                                // move and carry a segment each, and anything
                                // else is the segment they are all in.
                                ("segment", T::u8()),
                                (
                                    "entries",
                                    T::switch(
                                        E::field("segment"),
                                        vec![
                                            (0, T::bytes(E::lit(0))),
                                            (0xff, T::array(moveable_entry(), E::field("count"))),
                                        ],
                                        T::array(fixed_entry(), E::field("count")),
                                    ),
                                ),
                            ],
                        ),
                    ),
                ),
            ],
        ),
        Until::FieldBytes { field: "count".into(), bytes: vec![0] },
    )
}

fn fixed_entry() -> T {
    T::structure("Entry", vec![("flags", T::u8()), ("offset", T::u16(Little))]).counted_as("entry")
}

fn moveable_entry() -> T {
    T::structure(
        "MoveableEntry",
        vec![
            ("flags", T::u8()),
            // The instruction the loader replaces, which is an int 3f.
            ("thunk", T::magic(&[0xcd, 0x3f])),
            ("segment", T::u8()),
            ("offset", T::u16(Little)),
        ],
    )
    .counted_as("entry")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// A module with one code segment of two instructions, one name, and
    /// nothing else: enough for every table the template places to have
    /// somewhere to be.
    fn sample() -> Vec<u8> {
        let ne_at = 0x40usize;
        let mut v = vec![0u8; ne_at];
        v[0..2].copy_from_slice(b"MZ");
        v[0x18..0x1a].copy_from_slice(&0x40u16.to_le_bytes());
        v[0x3c..0x40].copy_from_slice(&(ne_at as u32).to_le_bytes());

        // The NE header is 0x40 bytes, so its own tables start at 0x40 from
        // it, which is 0x80 in the file.
        let mut h = vec![0u8; 0x40];
        h[0..2].copy_from_slice(b"NE");
        h[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes()); // one segment
        h[0x22..0x24].copy_from_slice(&0x40u16.to_le_bytes()); // segment table
        h[0x26..0x28].copy_from_slice(&0x48u16.to_le_bytes()); // resident names
        h[0x28..0x2a].copy_from_slice(&0x54u16.to_le_bytes()); // module references
        h[0x2a..0x2c].copy_from_slice(&0x54u16.to_le_bytes()); // imported names
        h[0x04..0x06].copy_from_slice(&0x54u16.to_le_bytes()); // entry table
        h[0x32..0x34].copy_from_slice(&4u16.to_le_bytes()); // sector shift
        h[0x36] = 2; // Windows
        v.extend_from_slice(&h);

        // One segment: code, four bytes of it, at sector 0x10.
        v.extend_from_slice(&0x10u16.to_le_bytes());
        v.extend_from_slice(&4u16.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&0x100u16.to_le_bytes());

        // The module's own name, then the byte that ends the table.
        v.push(3);
        v.extend_from_slice(b"WIN");
        v.extend_from_slice(&0u16.to_le_bytes());
        v.push(0);
        // The entry table, which holds nothing at all.
        v.push(0);

        v.resize(0x10 << 4, 0);
        // mov ax, 0x1234 and retf.
        v.extend_from_slice(&[0xb8, 0x34, 0x12, 0xcb]);
        v
    }

    #[test]
    fn a_segment_is_read_where_the_sector_count_says() {
        let d = Document::new(MemSource(sample()));
        let mut ev = Evaluator::new(ne());
        // Field 15 of the header is the segment table, placed elsewhere, so
        // its one child is the array.
        let segments = ev.node(&d, &[1, 33, 0]).unwrap();
        assert_eq!(segments.child_count, 1);
        let body = ev.node(&d, &[1, 33, 0, 0, 4, 0]).unwrap();
        assert_eq!(body.offset_bits, (0x10 << 4) * 8);
    }

    #[test]
    fn a_code_segment_reads_as_instructions() {
        let d = Document::new(MemSource(sample()));
        let mut ev = Evaluator::new(ne());
        let first = ev.node(&d, &[1, 33, 0, 0, 4, 0, 0, 0]).unwrap();
        assert_eq!(first.value, Value::Str("mov ax, 0x1234".into()));
        assert_eq!(ev.node(&d, &[1, 33, 0, 0, 4, 0, 0, 1]).unwrap().value, Value::Str("retf".into()));
    }

    #[test]
    fn the_resident_names_read_as_names() {
        let d = Document::new(MemSource(sample()));
        let mut ev = Evaluator::new(ne());
        assert_eq!(ev.node(&d, &[1, 35, 0, 0, 1]).unwrap().value, Value::Str("WIN".into()));
    }
}
