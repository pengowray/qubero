//! Linear Executable: the 32-bit format between the NE and the PE.
//!
//! Two signatures, one layout. `LE` is what a Windows VxD driver and a DOS
//! extender's client are, and `LX` is what a 32-bit OS/2 program is. The
//! header is the same 196 bytes either way; what differs is one field in it
//! and the entries of the object page map, and both are switched on here.
//!
//! What makes it worth reading is the same thing that makes the NE worth
//! reading: the program is a list of objects, each saying in its flags whether
//! it holds code, and each owning a run of pages the map says where to find.
//! An object's code reads as the 32-bit x86 the decoder already knows.
//!
//! The two indirections are the point. An object names a run in the page map
//! rather than a place in the file, and a map entry names a page rather than
//! an offset: in an `LE` by its number, counted from one, and in an `LX` by an
//! offset to shift. Both are followed here, so an object's pages read as the
//! bytes they are rather than as a table of numbers to work out by hand.
//!
//! Where this stops: an iterated page holds its bytes compressed elsewhere,
//! and a page the map calls invalid or zero filled has none in the file at
//! all. Those read as nothing rather than as whatever lies at the offset the
//! arithmetic would have given.

use crate::code::Isa;
use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};

/// Which of the two the file is. They share the header and part company at
/// the field the header keeps at 0x2c and at the page map's entries.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Le,
    Lx,
}

/// The processor the module needs. Anything past a 486 was written long after
/// the format stopped being used, so the list is short by the format's doing.
const CPU_TYPE: &[(i128, &str)] = &[(1, "286"), (2, "386"), (3, "486"), (4, "586"), (0x20, "i860 N10"), (0x21, "i860 N11")];

/// Which system loads it. A `4` is a Windows VxD, which is why a driver in a
/// Windows directory is a linear executable at all.
const OS_TYPE: &[(i128, &str)] = &[
    (0, "unknown"),
    (1, "OS/2"),
    (2, "Windows"),
    (3, "European MS-DOS 4.x"),
    (4, "Windows 386"),
    (5, "IBM Microkernel"),
];

/// What the module is. A VxD writes several of these at once: the bit that
/// says library and the bit that says device driver together are what makes
/// one, which is why nothing here is named "VxD".
const MODULE_FLAGS: &[(u32, &str)] = &[
    (0, "single data"),
    (2, "per-process initialization"),
    (4, "internal fixups done"),
    (5, "external fixups done"),
    (8, "not PM compatible"),
    (9, "PM compatible"),
    (13, "link error"),
    (15, "library"),
    (16, "protected memory library"),
    (17, "device driver"),
    (30, "per-process termination"),
];

/// An object's flags. Bit 2 is the one that decides how its pages read here.
/// Bit 8 is the one bit the two formats spend differently, so it is added by
/// the builder rather than written twice.
const OBJECT_FLAGS: &[(u32, &str)] = &[
    (0, "readable"),
    (1, "writable"),
    (2, "executable"),
    (3, "resource"),
    (4, "discardable"),
    (5, "shareable"),
    (6, "has preload pages"),
    (7, "has invalid pages"),
    (9, "permanent and resident"),
    (10, "permanent and long lockable"),
    (12, "16:16 alias required"),
    (13, "big"),
    (14, "conforming"),
    (15, "IO privilege level"),
];

/// What the map says about one page. Only a valid one has its bytes where the
/// arithmetic says; the rest are made at load time or unpacked from elsewhere.
const PAGE_FLAGS: &[(i128, &str)] = &[
    (0, "valid"),
    (1, "iterated"),
    (2, "invalid"),
    (3, "zero filled"),
    (4, "range"),
];

pub fn le() -> Template {
    // The DOS program in front, and the pointer at 0x3c that says where the
    // real header is: the same arrangement an NE and a PE use.
    let mut dos_fields = super::dos::header_fields();
    dos_fields.extend(vec![
        ("reserved", T::bytes(E::lit(8))),
        ("oem_id", T::u16(Little)),
        ("oem_info", T::u16(Little)),
        ("reserved2", T::bytes(E::lit(20))),
        ("header_offset", T::u32(Little)),
        ("dos_stub", T::bytes(E::field("header_offset").sub(E::lit(64)))),
    ]);
    let dos = T::structure("DOSHeader", dos_fields);

    Template::new(
        "le",
        T::structure(
            "LinearExecutable",
            vec![
                ("dos", dos),
                (
                    "exe",
                    T::switch(
                        E::peek(16, Little),
                        vec![(0x454c, header(Kind::Le)), (0x584c, header(Kind::Lx))],
                        T::bytes(E::Remaining),
                    ),
                ),
            ],
        ),
    )
}

fn header(kind: Kind) -> T {
    let name = match kind {
        Kind::Le => "LEHeader",
        Kind::Lx => "LXHeader",
    };
    // The field the two formats spend differently. An LE writes how many
    // bytes of the last page in the file are real; an LX writes how many bits
    // to shift a page's offset by.
    let union = match kind {
        Kind::Le => ("last_page_size", T::u32(Little)),
        Kind::Lx => ("page_shift", T::u32(Little)),
    };
    let signature = match kind {
        Kind::Le => T::magic(b"LE"),
        Kind::Lx => T::magic(b"LX"),
    };
    T::structure(
        name,
        vec![
            ("signature", signature),
            // Zero for both means the little-endian, low-word-first ordering
            // every file that exists uses. Nothing was ever built the other
            // way, so the rest of this template reads little-endian outright.
            ("byte_order", T::u8()),
            ("word_order", T::u8()),
            ("format_level", T::u32(Little)),
            ("cpu_type", T::enumeration("CPUType", T::u16(Little), CPU_TYPE)),
            ("os_type", T::enumeration("OSType", T::u16(Little), OS_TYPE)),
            ("module_version", T::u32(Little)),
            ("flags", T::flags("ModuleFlags", T::u32(Little), MODULE_FLAGS)),
            ("page_count", T::u32(Little)),
            ("entry_object", T::u32(Little)),
            ("eip", T::u32(Little)),
            ("stack_object", T::u32(Little)),
            ("esp", T::u32(Little)),
            ("page_size", T::u32(Little)),
            union,
            ("fixup_size", T::u32(Little)),
            ("fixup_checksum", T::u32(Little)),
            ("loader_size", T::u32(Little)),
            ("loader_checksum", T::u32(Little)),
            ("object_table_offset", T::u32(Little)),
            ("object_count", T::u32(Little)),
            ("page_map_offset", T::u32(Little)),
            ("iterated_data_offset", T::u32(Little)),
            ("resource_table_offset", T::u32(Little)),
            ("resource_count", T::u32(Little)),
            ("resident_names_offset", T::u32(Little)),
            ("entry_table_offset", T::u32(Little)),
            ("directives_offset", T::u32(Little)),
            ("directive_count", T::u32(Little)),
            ("fixup_page_table_offset", T::u32(Little)),
            ("fixup_record_table_offset", T::u32(Little)),
            ("import_module_table_offset", T::u32(Little)),
            ("import_module_count", T::u32(Little)),
            ("import_procedure_table_offset", T::u32(Little)),
            ("page_checksum_offset", T::u32(Little)),
            // Counted from the start of the file, as are the two below it.
            // Every other offset here counts from the start of this header.
            ("data_pages_offset", T::u32(Little)),
            ("preload_page_count", T::u32(Little)),
            ("nonresident_names_offset", T::u32(Little)),
            ("nonresident_names_size", T::u32(Little)),
            ("nonresident_names_checksum", T::u32(Little)),
            ("auto_data_object", T::u32(Little)),
            ("debug_offset", T::u32(Little)),
            ("debug_size", T::u32(Little)),
            ("instance_preload_count", T::u32(Little)),
            ("instance_demand_count", T::u32(Little)),
            ("heap_size", T::u32(Little)),
            ("stack_size", T::u32(Little)),
            ("tail", tail()),
            // The map comes before the objects because an object reaches into
            // it: its pages are a run of entries, and an entry is what says
            // where a page is. Both are read where the header says they are,
            // so the order they are written in here is not the file's.
            ("page_map", at_header(E::field("page_map_offset"), T::array(map_entry(kind), E::field("page_count")))),
            ("objects", at_header(E::field("object_table_offset"), T::array(object(kind), E::field("object_count")))),
            ("resident_names", at_header(E::field("resident_names_offset"), names("ResidentName"))),
            (
                "import_modules",
                at_header(
                    E::field("import_module_table_offset"),
                    T::array(import_name(), E::field("import_module_count")),
                ),
            ),
            (
                "nonresident_names",
                T::at(
                    E::field("nonresident_names_offset"),
                    T::sized(E::field("nonresident_names_size"), names("NonResidentName")),
                ),
            ),
        ],
    )
}

/// The twenty bytes that pad the header to 196. A Windows VxD spends four of
/// its fields on where its version resource is and which device it is; an
/// OS/2 module leaves all twenty alone.
fn tail() -> T {
    T::switch(
        E::field("os_type"),
        vec![(
            4,
            T::structure(
                "VxDInfo",
                vec![
                    ("reserved", T::bytes(E::lit(8))),
                    ("version_resource_offset", T::u32(Little)),
                    ("version_resource_size", T::u32(Little)),
                    ("device_id", T::u16(Little)),
                    ("ddk_version", T::u16(Little)),
                ],
            ),
        )],
        T::bytes(E::lit(20)),
    )
}

/// A table at an offset counted from the start of the LE header, which is
/// where the DOS header at the front of the file points.
fn at_header(at: E, inner: T) -> T {
    T::at(E::within(&["dos", "header_offset"]).add(at), inner)
}

/// One entry of the object page map: which page, and whether it is a page
/// with bytes in the file at all.
///
/// An LE names the page by its number, counted from one, and writes that
/// number the other way round from every other number in the file. An LX
/// names it by an offset to shift and says how many of its bytes are real.
fn map_entry(kind: Kind) -> T {
    match kind {
        Kind::Le => T::structure(
            "PageEntry",
            vec![
                ("page_number", T::Int { bits: 24, endian: Big }),
                ("flags", T::enumeration("PageFlags", T::u8(), PAGE_FLAGS)),
            ],
        ),
        Kind::Lx => T::structure(
            "PageEntry",
            vec![
                ("page_offset", T::u32(Little)),
                ("data_size", T::u16(Little)),
                ("flags", T::enumeration("PageFlags", T::u16(Little), PAGE_FLAGS)),
            ],
        ),
    }
}

/// One object: how much room it wants, where it wants it, what it holds, and
/// which run of the page map its pages are.
fn object(kind: Kind) -> T {
    // The one bit the two formats spend differently.
    let mut flags = OBJECT_FLAGS.to_vec();
    flags.push(match kind {
        Kind::Le => (8, "permanent and swappable"),
        Kind::Lx => (8, "zero filled"),
    });
    T::structure(
        "Object",
        vec![
            ("size", T::u32(Little)),
            ("address", T::u32(Little)),
            ("flags", T::flags("ObjectFlags", T::u32(Little), &flags)),
            // Where its pages start in the map, counted from one.
            ("map_index", T::u32(Little)),
            ("map_size", T::u32(Little)),
            ("reserved", T::u32(Little)),
            ("pages", T::array(page(kind), E::field("map_size"))),
        ],
    )
    .counted_as("object")
}

/// One of an object's pages, read where the map entry for it says. Only a
/// valid page has bytes in the file; the rest read as nothing.
fn page(kind: Kind) -> T {
    // Which entry of the map this page is: the object's first, plus how far
    // along the object this page is.
    let entry = || E::field("map_index").sub(E::lit(1)).add(E::idx());
    let at = |field: &'static str| E::elem_field("page_map", entry(), &[field]);
    let (offset, size) = match kind {
        Kind::Le => (
            E::field("data_pages_offset").add(at("page_number").sub(E::lit(1)).mul(E::field("page_size"))),
            // Every page is a full one but the last in the file, and the
            // header says how much of that one is real.
            T::switch(
                at("page_number").sub(E::field("page_count")),
                vec![(0, body(E::field("last_page_size")))],
                body(E::field("page_size")),
            ),
        ),
        Kind::Lx => (
            E::field("data_pages_offset").add(at("page_offset").shl(E::field("page_shift"))),
            body(at("data_size")),
        ),
    };
    T::switch(at("flags"), vec![(0, T::at(offset, size))], T::bytes(E::lit(0)))
}

/// What is in a page: instructions where the object that owns it says it holds
/// code, and bytes where it does not.
fn body(size: E) -> T {
    T::switch(
        E::field("flags").bit(2),
        vec![(1, T::sized(size.clone(), T::repeat(T::insn(Isa::X86_32), Until::End)))],
        T::bytes(size),
    )
    .counted_as("page")
}

/// A name table: a length byte, that many characters, and the ordinal the name
/// stands for. A length of zero ends it.
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

/// The name of a module this one imports from: a length byte and that many
/// characters, with no ordinal after it.
fn import_name() -> T {
    T::structure_named(
        "ImportModule",
        "text",
        "",
        vec![("length", T::u8()), ("text", T::text(StrLen::Fixed(E::field("length")), Encoding::Ascii))],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    // Where things are in the tree: the header after the DOS one, and the two
    // tables that point at each other.
    const EXE: usize = 1;
    const OBJECTS: usize = 49;

    /// A whole linear executable, small enough to write out here: a DOS header
    /// pointing at 0x80, the 196-byte header there, one object of two pages,
    /// and the pages themselves at 0x200. Pages of 16 bytes rather than 4096,
    /// so that the file is a page of its own.
    fn file(signature: &[u8; 2], union: u32, map: &[u8]) -> Vec<u8> {
        let names = 0xdc + map.len() as u32;
        let mut h = signature.to_vec();
        h.extend_from_slice(&[0, 0]); // byte order, word order
        h.extend_from_slice(&0u32.to_le_bytes()); // format level
        h.extend_from_slice(&2u16.to_le_bytes()); // a 386
        h.extend_from_slice(&1u16.to_le_bytes()); // OS/2, so the tail is reserved
        for n in [
            0, 0, 2, 1, 0, 0, 0, 16, union, 0, 0, 0, 0, 0xc4, 1, 0xdc, 0, 0, 0, names, 0, 0, 0, 0, 0, names + 6, 0,
            0, 0, 0x200, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ] {
            h.extend_from_slice(&(n as u32).to_le_bytes());
        }
        h.extend_from_slice(&[0; 20]);
        assert_eq!(h.len(), 196);

        // One object: readable and executable, owning both pages.
        for n in [20u32, 0x1_0000, 0x5, 1, 2, 0] {
            h.extend_from_slice(&n.to_le_bytes());
        }
        h.extend_from_slice(map);
        h.extend_from_slice(&[2, b'H', b'I', 1, 0, 0]); // one resident name, then the end

        let mut v = vec![0u8; 0x80];
        v[0..2].copy_from_slice(b"MZ");
        v[0x18..0x1a].copy_from_slice(&0x40u16.to_le_bytes());
        v[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        v.extend_from_slice(&h);
        v.resize(0x200, 0);
        v.extend_from_slice(&[0x90; 20]); // two pages of nothing but nops
        v
    }

    /// The page map an LE writes: a page number, counted from one and written
    /// the other way round from every other number in the file.
    fn le_file() -> Vec<u8> {
        file(b"LE", 4, &[0, 0, 1, 0, 0, 0, 2, 0])
    }

    /// The one an LX writes: an offset to shift, and how many bytes are real.
    fn lx_file() -> Vec<u8> {
        let mut map = Vec::new();
        for (offset, size) in [(0u32, 16u16), (16, 4)] {
            map.extend_from_slice(&offset.to_le_bytes());
            map.extend_from_slice(&size.to_le_bytes());
            map.extend_from_slice(&0u16.to_le_bytes()); // valid
        }
        file(b"LX", 0, &map)
    }

    /// Both files hold the same program, so both read the same way: the
    /// object's two pages, at the two places the map sends them.
    fn pages_read(bytes: Vec<u8>, header: &str) {
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(le());
        assert_eq!(ev.node(&d, &[EXE]).unwrap().type_name, header);
        let object = [EXE, OBJECTS, 0, 0];
        let flags = ev.node(&d, &[object.as_slice(), &[2]].concat()).unwrap();
        assert_eq!(
            flags.value,
            Value::Flags { raw: 5, set: vec!["readable".into(), "executable".into()], unnamed: 0 }
        );
        // The first page is a full one, and the second is what is left: four
        // bytes, which an LE says in the header and an LX in the map entry.
        let page = |k: usize| [object.as_slice(), &[6, k, 0]].concat();
        let first = ev.node(&d, &page(0)).unwrap();
        assert_eq!(first.offset_bits, 0x200 * 8);
        assert_eq!(first.child_count, 16);
        let second = ev.node(&d, &page(1)).unwrap();
        assert_eq!(second.offset_bits, 0x210 * 8);
        assert_eq!(second.child_count, 4);
        // An executable object's pages read as instructions.
        assert_eq!(ev.node(&d, &[page(0).as_slice(), &[0]].concat()).unwrap().value, Value::Str("nop".into()));
    }

    #[test]
    fn an_le_finds_its_pages_by_number() {
        pages_read(le_file(), "LEHeader");
    }

    #[test]
    fn an_lx_finds_its_pages_by_offset() {
        pages_read(lx_file(), "LXHeader");
    }

    #[test]
    fn both_signatures_reach_the_same_template() {
        for bytes in [le_file(), lx_file()] {
            assert_eq!(crate::formats::sniff(&bytes, bytes.len() as u64), Some("le"));
        }
    }

    /// A page the map calls invalid has no bytes in the file, so nothing is
    /// read at the offset the arithmetic would have given.
    #[test]
    fn a_page_that_is_not_in_the_file_reads_as_nothing() {
        let bytes = file(b"LE", 4, &[0, 0, 1, 2, 0, 0, 2, 0]);
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(le());
        let first = ev.node(&d, &[EXE, OBJECTS, 0, 0, 6, 0]).unwrap();
        assert_eq!(first.size_bits, 0);
        assert_eq!(ev.node(&d, &[EXE, OBJECTS, 0, 0, 6, 1, 0]).unwrap().child_count, 4);
    }
}
