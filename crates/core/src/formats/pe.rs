//! PE: the Windows executable format, and the DOS header still in front of it.
//!
//! A PE file opens with a DOS executable that does nothing but print a refusal,
//! and the real header sits wherever `pe_header_offset` at 0x3c points. That
//! makes the stub a field of computed length rather than a fixed one, which is
//! why the stub belongs to the DOS header here: a length expression can only
//! name a field beside it or above it, never one inside a sibling.
//!
//! The optional header is neither optional nor one layout. Its `magic` says
//! whether addresses in it are 32 or 64 bits wide, and the two layouts differ
//! in four places, so `Switch` picks between them. Its declared size is what
//! bounds it, not the sum of its fields, because the data directory count at
//! the end of it is a field of the file rather than a constant.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T};

/// The machines a PE says it is for. The list is longer than anything still
/// running: the point of a name here is that a file claiming `0x14c` is read
/// as 32-bit x86 rather than left as a number to look up.
const MACHINE: &[(i128, &str)] = &[
    (0x0000, "unknown"),
    (0x014c, "i386"),
    (0x0162, "mips r3000"),
    (0x0166, "mips r4000"),
    (0x0168, "mips r10000"),
    (0x0169, "mips wce v2"),
    (0x0184, "alpha"),
    (0x01a2, "sh3"),
    (0x01a3, "sh3 dsp"),
    (0x01a6, "sh4"),
    (0x01a8, "sh5"),
    (0x01c0, "arm"),
    (0x01c2, "thumb"),
    (0x01c4, "arm thumb-2"),
    (0x01d3, "am33"),
    (0x01f0, "powerpc"),
    (0x01f1, "powerpc fp"),
    (0x0200, "ia64"),
    (0x0266, "mips16"),
    (0x0284, "alpha64"),
    (0x0366, "mips fpu"),
    (0x0466, "mips16 fpu"),
    (0x0ebc, "efi bytecode"),
    (0x5032, "riscv32"),
    (0x5064, "riscv64"),
    (0x5128, "riscv128"),
    (0x8664, "amd64"),
    (0xaa64, "arm64"),
];

/// What the loader hands the image to.
const SUBSYSTEM: &[(i128, &str)] = &[
    (0, "unknown"),
    (1, "native"),
    (2, "windows gui"),
    (3, "windows console"),
    (5, "os/2 console"),
    (7, "posix console"),
    (8, "native windows"),
    (9, "windows ce gui"),
    (10, "efi application"),
    (11, "efi boot service driver"),
    (12, "efi runtime driver"),
    (13, "efi rom"),
    (14, "xbox"),
    (16, "windows boot application"),
];

/// The file header's characteristics. Half of these describe a file nobody has
/// produced since the 1990s, which is the point of naming them: a bit set in a
/// modern binary that means "16-bit machine" is worth seeing.
const CHARACTERISTICS: &[(u32, &str)] = &[
    (0, "relocations stripped"),
    (1, "executable"),
    (2, "line numbers stripped"),
    (3, "local symbols stripped"),
    (4, "aggressive working set trim"),
    (5, "large address aware"),
    (7, "bytes reversed low"),
    (8, "32-bit machine"),
    (9, "debug stripped"),
    (10, "run from swap if removable"),
    (11, "run from swap if network"),
    (12, "system file"),
    (13, "dll"),
    (14, "uniprocessor only"),
    (15, "bytes reversed high"),
];

/// What the loader may do with the image. The mitigation bits (ASLR, DEP,
/// control flow guard) are the ones people look for.
const DLL_CHARACTERISTICS: &[(u32, &str)] = &[
    (5, "high entropy va"),
    (6, "dynamic base"),
    (7, "force integrity"),
    (8, "nx compatible"),
    (9, "no isolation"),
    (10, "no seh"),
    (11, "no bind"),
    (12, "appcontainer"),
    (13, "wdm driver"),
    (14, "control flow guard"),
    (15, "terminal server aware"),
];

/// A section's characteristics. Bits 20 to 23 hold an alignment as a number
/// rather than as flags, so they are left unnamed here: a name on each would
/// say four things where the format says one.
const SECTION_FLAGS: &[(u32, &str)] = &[
    (3, "no pad"),
    (5, "code"),
    (6, "initialized data"),
    (7, "uninitialized data"),
    (9, "info"),
    (11, "remove"),
    (12, "comdat"),
    (15, "global pointer relative"),
    (24, "extended relocations"),
    (25, "discardable"),
    (26, "not cached"),
    (27, "not paged"),
    (28, "shared"),
    (29, "execute"),
    (30, "read"),
    (31, "write"),
];

/// The two shapes of the optional header, told apart by its first two bytes.
const OPTIONAL_MAGIC: &[(i128, &str)] = &[(0x107, "rom"), (0x10b, "pe32"), (0x20b, "pe32+")];

/// Bytes of the optional header before `Switch` reaches the part that differs:
/// the magic, the two linker version bytes, and five longs.
const OPTIONAL_FIXED: i128 = 2 + 2 + 5 * 4;

/// The directories, in the order the header lists them. Nothing in the file
/// names them: entry three is the exception table because it is third.
const DIRECTORY: &[(i128, &str)] = &[
    (0, "export"),
    (1, "import"),
    (2, "resource"),
    (3, "exception"),
    (4, "certificate"),
    (5, "base relocation"),
    (6, "debug"),
    (7, "architecture"),
    (8, "global pointer"),
    (9, "thread local storage"),
    (10, "load config"),
    (11, "bound import"),
    (12, "import address table"),
    (13, "delay import"),
    (14, "clr runtime"),
    (15, "reserved"),
];

/// One data directory entry: an address and a length, and nothing saying which
/// directory it is. Its position in the array is what decides that, so the
/// position picks the name. `Switch` on the index is how the IR says it, and
/// the type each entry reads as is the directory's own name.
fn directory_entry() -> T {
    let fields = || vec![("rva", T::u32(Little)), ("size", T::u32(Little))];
    let cases: Vec<(i128, T)> = DIRECTORY.iter().map(|(i, name)| (*i, T::structure(name, fields()))).collect();
    // Past the sixteen the format defines, an entry is still an entry.
    T::switch(E::idx(), cases, T::structure("DataDirectory", fields()))
}

/// The part of the optional header after the addresses, which is the same for
/// both layouts except that the four stack and heap sizes follow the pointer.
fn optional_tail(pointer: T) -> Vec<(&'static str, T)> {
    vec![
        ("section_alignment", T::u32(Little)),
        ("file_alignment", T::u32(Little)),
        ("os_version_major", T::u16(Little)),
        ("os_version_minor", T::u16(Little)),
        ("image_version_major", T::u16(Little)),
        ("image_version_minor", T::u16(Little)),
        ("subsystem_version_major", T::u16(Little)),
        ("subsystem_version_minor", T::u16(Little)),
        ("win32_version", T::u32(Little)),
        ("image_size", T::u32(Little)),
        ("headers_size", T::u32(Little)),
        ("checksum", T::u32(Little)),
        ("subsystem", T::enumeration("Subsystem", T::u16(Little), SUBSYSTEM)),
        ("dll_characteristics", T::flags("DllCharacteristics", T::u16(Little), DLL_CHARACTERISTICS)),
        ("stack_reserve", pointer.clone()),
        ("stack_commit", pointer.clone()),
        ("heap_reserve", pointer.clone()),
        ("heap_commit", pointer),
        ("loader_flags", T::u32(Little)),
        ("data_directory_count", T::u32(Little)),
        ("data_directory", T::array(directory_entry(), E::field("data_directory_count"))),
    ]
}

pub fn pe() -> Template {
    // The 64-byte DOS header, then the stub filling the gap to the real one.
    // MS-DOS itself read only as far as `relocation_table`; everything from
    // `reserved` on exists to hold this one pointer.
    let dos = T::structure(
        "DOSHeader",
        vec![
            ("magic", T::magic(b"MZ")),
            ("bytes_on_last_page", T::u16(Little)),
            ("pages", T::u16(Little)),
            ("relocations", T::u16(Little)),
            ("header_paragraphs", T::u16(Little)),
            ("min_extra_paragraphs", T::u16(Little)),
            ("max_extra_paragraphs", T::u16(Little)),
            ("initial_ss", T::u16(Little)),
            ("initial_sp", T::u16(Little)),
            ("checksum", T::u16(Little)),
            ("initial_ip", T::u16(Little)),
            ("initial_cs", T::u16(Little)),
            ("relocation_table", T::u16(Little)),
            ("overlay_number", T::u16(Little)),
            ("reserved", T::bytes(E::lit(8))),
            ("oem_id", T::u16(Little)),
            ("oem_info", T::u16(Little)),
            ("reserved2", T::bytes(E::lit(20))),
            ("pe_header_offset", T::u32(Little)),
            // The refusal message and the code that prints it. A file whose
            // real header starts at 64 has none at all.
            ("dos_stub", T::bytes(E::field("pe_header_offset").sub(E::lit(64)))),
        ],
    );

    let section = T::structure(
        "Section",
        vec![
            // Eight bytes, NUL-padded rather than NUL-terminated: a name of
            // exactly eight characters has no terminator at all.
            ("name", T::text(StrLen::Padded { size: E::lit(8), pad: 0 }, Encoding::Ascii)),
            ("virtual_size", T::u32(Little)),
            ("virtual_address", T::u32(Little)),
            ("raw_size", T::u32(Little)),
            ("raw_offset", T::u32(Little)),
            ("relocations_offset", T::u32(Little)),
            ("line_numbers_offset", T::u32(Little)),
            ("relocation_count", T::u16(Little)),
            ("line_number_count", T::u16(Little)),
            ("characteristics", T::flags("SectionFlags", T::u32(Little), SECTION_FLAGS)),
        ],
    );

    let mut pe32: Vec<(&str, T)> = vec![
        // The only field PE32+ drops: with a 64-bit image base there is no
        // room for it and no need.
        ("data_base", T::u32(Little)),
        ("image_base", T::u32(Little)),
    ];
    pe32.extend(optional_tail(T::u32(Little)));
    let mut pe32plus: Vec<(&str, T)> = vec![("image_base", T::u64(Little))];
    pe32plus.extend(optional_tail(T::u64(Little)));

    let optional = T::structure(
        "OptionalHeader",
        vec![
            ("magic", T::enumeration_hex("OptionalMagic", T::u16(Little), OPTIONAL_MAGIC)),
            ("linker_version_major", T::u8()),
            ("linker_version_minor", T::u8()),
            ("code_size", T::u32(Little)),
            ("initialized_data_size", T::u32(Little)),
            ("uninitialized_data_size", T::u32(Little)),
            ("entry_point", T::u32(Little)),
            ("code_base", T::u32(Little)),
            (
                "addresses",
                T::switch(
                    E::field("magic"),
                    vec![(0x10b, T::structure("PE32", pe32)), (0x20b, T::structure("PE32Plus", pe32plus))],
                    // A magic neither layout claims leaves the rest as bytes,
                    // measured from the size the file declared for the header.
                    T::bytes(E::field("optional_header_size").sub(E::lit(OPTIONAL_FIXED))),
                ),
            ),
        ],
    );

    let header = T::structure(
        "PEHeader",
        vec![
            ("signature", T::magic(b"PE\0\0")),
            ("machine", T::enumeration_hex("Machine", T::u16(Little), MACHINE)),
            ("section_count", T::u16(Little)),
            ("timestamp", T::u32(Little)),
            ("symbol_table", T::u32(Little)),
            ("symbol_count", T::u32(Little)),
            ("optional_header_size", T::u16(Little)),
            ("characteristics", T::flags("Characteristics", T::u16(Little), CHARACTERISTICS)),
            // Bounded by what the file says, not by what the fields add up to:
            // a linker may leave room after the data directory.
            ("optional", T::sized(E::field("optional_header_size"), optional)),
            ("sections", T::array(section, E::field("section_count"))),
        ],
    );

    Template::new("pe", T::structure("PE", vec![("dos", dos), ("pe", header)]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// A PE32+ with one section, built by hand: enough header for every field
    /// the template places to have somewhere to be.
    fn sample() -> Vec<u8> {
        let mut v = vec![0u8; 64];
        v[0..2].copy_from_slice(b"MZ");
        let pe_at: u32 = 0x80;
        v[0x3c..0x40].copy_from_slice(&pe_at.to_le_bytes());
        v.resize(pe_at as usize, 0); // the stub

        v.extend_from_slice(b"PE\0\0");
        v.extend_from_slice(&0x8664u16.to_le_bytes()); // machine: amd64
        v.extend_from_slice(&1u16.to_le_bytes()); // one section
        v.extend_from_slice(&0x6600_0000u32.to_le_bytes()); // timestamp
        v.extend_from_slice(&0u32.to_le_bytes()); // symbol table
        v.extend_from_slice(&0u32.to_le_bytes()); // symbol count
        v.extend_from_slice(&0xf0u16.to_le_bytes()); // optional header size
        v.extend_from_slice(&0x0022u16.to_le_bytes()); // characteristics

        let start = v.len();
        v.extend_from_slice(&0x20bu16.to_le_bytes()); // PE32+
        v.push(14); // linker major
        v.push(0); // linker minor
        v.extend_from_slice(&0x200u32.to_le_bytes()); // code size
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&0x1000u32.to_le_bytes()); // entry point
        v.extend_from_slice(&0x1000u32.to_le_bytes()); // code base
        v.extend_from_slice(&0x1_4000_0000u64.to_le_bytes()); // image base
        v.extend_from_slice(&0x1000u32.to_le_bytes()); // section alignment
        v.extend_from_slice(&0x200u32.to_le_bytes()); // file alignment
        for _ in 0..6 {
            v.extend_from_slice(&0u16.to_le_bytes()); // the six version numbers
        }
        v.extend_from_slice(&0u32.to_le_bytes()); // win32 version
        v.extend_from_slice(&0x2000u32.to_le_bytes()); // image size
        v.extend_from_slice(&0x200u32.to_le_bytes()); // headers size
        v.extend_from_slice(&0u32.to_le_bytes()); // checksum
        v.extend_from_slice(&3u16.to_le_bytes()); // subsystem: console
        v.extend_from_slice(&0u16.to_le_bytes()); // dll characteristics
        for _ in 0..4 {
            v.extend_from_slice(&0x10_0000u64.to_le_bytes()); // stack and heap
        }
        v.extend_from_slice(&0u32.to_le_bytes()); // loader flags
        v.extend_from_slice(&2u32.to_le_bytes()); // two data directory entries
        for i in 0..2u32 {
            v.extend_from_slice(&(0x1000 + i).to_le_bytes()); // rva
            v.extend_from_slice(&0x40u32.to_le_bytes()); // size
        }
        // Pad out to the size the header declared.
        v.resize(start + 0xf0, 0);

        v.extend_from_slice(b".text\0\0\0");
        v.extend_from_slice(&0x100u32.to_le_bytes()); // virtual size
        v.extend_from_slice(&0x1000u32.to_le_bytes()); // virtual address
        v.extend_from_slice(&0x200u32.to_le_bytes()); // raw size
        v.extend_from_slice(&0x200u32.to_le_bytes()); // raw offset
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&0x6000_0020u32.to_le_bytes()); // characteristics
        v.resize(0x400, 0);
        v
    }

    fn value(e: &mut Evaluator, doc: &Document<MemSource>, path: &[usize]) -> Value {
        e.node(doc, path).expect("field").value
    }

    #[test]
    fn reads_the_dos_header_and_finds_the_real_one() {
        let doc = Document::new(MemSource(sample()));
        let mut e = Evaluator::new(pe());
        // dos.magic, then the pointer that says where the PE header starts.
        assert!(matches!(value(&mut e, &doc, &[0, 0]), Value::Magic { ok: true }));
        assert_eq!(value(&mut e, &doc, &[0, 18]).as_int(), Some(0x80));
        // pe.signature sits exactly there, which is the whole point of it.
        let sig = e.node(&doc, &[1, 0]).expect("signature");
        assert_eq!(sig.offset_bits, 0x80 * 8);
        assert!(matches!(sig.value, Value::Magic { ok: true }));
    }

    #[test]
    fn the_stub_stretches_to_meet_it() {
        let doc = Document::new(MemSource(sample()));
        let mut e = Evaluator::new(pe());
        let stub = e.node(&doc, &[0, 19]).expect("stub");
        assert_eq!(stub.offset_bits, 64 * 8);
        assert_eq!(stub.size_bits, (0x80 - 64) * 8);
    }

    #[test]
    fn machine_and_subsystem_read_as_names() {
        let doc = Document::new(MemSource(sample()));
        let mut e = Evaluator::new(pe());
        let machine = value(&mut e, &doc, &[1, 1]);
        assert!(matches!(&machine, Value::Enum { name: Some(n), .. } if n == "amd64"), "{machine:?}");
    }

    #[test]
    fn the_magic_picks_the_64_bit_layout() {
        let doc = Document::new(MemSource(sample()));
        let mut e = Evaluator::new(pe());
        // optional.addresses is the switch; under PE32+ its first field is the
        // 64-bit image base, with no separate data base in front of it.
        let addresses = e.node(&doc, &[1, 8, 8]).expect("addresses");
        assert_eq!(addresses.type_name, "PE32Plus");
        let image_base = e.node(&doc, &[1, 8, 8, 0]).expect("image base");
        assert_eq!(image_base.name, "image_base");
        assert_eq!(image_base.size_bits, 64);
        assert_eq!(image_base.value.as_int(), Some(0x1_4000_0000));
    }

    #[test]
    fn characteristics_read_as_named_bits() {
        let doc = Document::new(MemSource(sample()));
        let mut e = Evaluator::new(pe());
        // 0x0022: bit 1 executable, bit 5 large address aware.
        let v = value(&mut e, &doc, &[1, 7]);
        let Value::Flags { raw, set, unnamed } = v else { panic!("expected flags, got {v:?}") };
        assert_eq!(raw, 0x22);
        assert_eq!(set, ["executable", "large address aware"]);
        assert_eq!(unnamed, 0);
    }

    #[test]
    fn every_bit_is_explained_named_or_not() {
        use crate::eval::Explain;
        let doc = Document::new(MemSource(sample()));
        let mut e = Evaluator::new(pe());
        let Explain::Flags { name, raw, bits } = e.explain(&doc, &[1, 7]).expect("explain") else {
            panic!("expected flags")
        };
        assert_eq!(name, "Characteristics");
        assert_eq!(raw, 0x22);
        assert_eq!(bits.len(), 16, "one entry per bit of the field, set or not");
        assert!(bits[1].set && bits[1].name.as_deref() == Some("executable"));
        assert!(!bits[0].set && bits[0].name.as_deref() == Some("relocations stripped"));
        // Bit 6 has no meaning in the format, and is listed anyway.
        assert!(bits[6].name.is_none());
    }

    #[test]
    fn a_magic_field_says_what_it_wanted() {
        use crate::eval::Explain;
        let doc = Document::new(MemSource(sample()));
        let mut e = Evaluator::new(pe());
        let Explain::Magic { expected, actual } = e.explain(&doc, &[1, 0]).expect("explain") else {
            panic!("expected magic")
        };
        assert_eq!(expected, b"PE\0\0");
        assert_eq!(actual, b"PE\0\0");
    }

    #[test]
    fn an_enum_lists_what_else_it_would_accept() {
        use crate::eval::Explain;
        let doc = Document::new(MemSource(sample()));
        let mut e = Evaluator::new(pe());
        let Explain::Enum { name, cases, current, .. } = e.explain(&doc, &[1, 1]).expect("explain") else {
            panic!("expected enum")
        };
        assert_eq!(name, "Machine");
        assert_eq!(current, 0x8664);
        assert_eq!(cases.len(), MACHINE.len());
        assert!(cases.iter().any(|(v, n)| *v == 0x014c && n == "i386"));
    }

    #[test]
    fn the_section_table_is_as_long_as_the_header_says() {
        let doc = Document::new(MemSource(sample()));
        let mut e = Evaluator::new(pe());
        let sections = e.node(&doc, &[1, 9]).expect("sections");
        assert!(matches!(sections.value, Value::Composite { count: 1 }));
        let name = value(&mut e, &doc, &[1, 9, 0, 0]);
        assert!(matches!(&name, Value::Str(s) if s == ".text"), "{name:?}");
    }

    #[test]
    fn a_section_starts_where_the_optional_header_ends() {
        let doc = Document::new(MemSource(sample()));
        let mut e = Evaluator::new(pe());
        let first = e.node(&doc, &[1, 9, 0]).expect("first section");
        // 0x80 + 4 signature + 20 file header + 0xf0 declared optional header.
        assert_eq!(first.offset_bits, (0x80 + 24 + 0xf0) * 8);
        assert_eq!(first.size_bits, 40 * 8);
    }
}
