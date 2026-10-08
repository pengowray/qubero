//! ELF objects, executables and shared libraries, and the eBPF programs that
//! come as ELF relocatable objects.
//!
//! One layout serves four files. The first two bytes past the magic say how
//! wide an address is and which way round a number is written, and everything
//! after them is read accordingly: the header switches on the class and then
//! on the data encoding, and the four bodies below it are the same fields
//! built four times over. That is what the IR can say. A template fixes a
//! field's width and endianness where the field is declared, so a file that
//! announces both in its first bytes has to declare each combination it
//! allows.
//!
//! Section contents are read for what the section header says they are: a
//! symbol table as symbols, a string table as the strings in it, a relocation
//! table as relocations, the dynamic section as its tagged entries with each
//! library named, and a note section as its notes. A section of type `nobits`
//! is the one that has to be
//! handled apart. `.bss` has a size and an offset like any other section and
//! occupies none of the file, so reading its bytes where it points would claim
//! bytes belonging to whatever comes next.
//!
//! [`bpf`] is the same template with one addition: a section that is program
//! bits and marked executable is read as eBPF instructions rather than as
//! bytes. Which is a guess in general and not a guess here, since the file
//! said its machine was BPF before this template was chosen at all.
//!
//! Sections name themselves. A section's name is an offset into another
//! section, which for a long time was taken to be past what a template could
//! say, so a section read as `[2]` everywhere and only the disassembler's own
//! pass knew it was `.text`. It is not past what a template can say: the name
//! table's offset is read once into `section_name_base`, straight out of the
//! header table rather than through it, and each header then reads its own name
//! from there with `at`. `SectionHeader` is `named_by` that field, and the
//! bytes a header places borrow their header's name, so the megabyte of code in
//! the middle of a program is `.text` in the listing, the treemap and the
//! inspector alike.
//!
//! Symbol names are still a pass over the parsed tree, along with everything
//! that needs two tables at once: see [`super::elf_disasm`].

use super::bpf_opcodes::{OPCODES, REGS};
use crate::code::Isa;
use crate::template::{Anchor, Encoding, Endian, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};

const CLASS: &[(i128, &str)] = &[(1, "32-bit"), (2, "64-bit")];

const DATA: &[(i128, &str)] = &[(1, "little-endian"), (2, "big-endian")];

const OSABI: &[(i128, &str)] = &[
    (0, "System V"),
    (1, "HP-UX"),
    (2, "NetBSD"),
    (3, "Linux"),
    (6, "Solaris"),
    (7, "AIX"),
    (8, "IRIX"),
    (9, "FreeBSD"),
    (12, "OpenBSD"),
    (13, "OpenVMS"),
    (64, "ARM EABI"),
    (97, "ARM"),
    (255, "standalone"),
];

const OBJECT_TYPE: &[(i128, &str)] =
    &[(0, "none"), (1, "relocatable"), (2, "executable"), (3, "shared object"), (4, "core dump")];

/// The machines worth naming. The list the standard keeps runs to hundreds;
/// these are the ones a file on a desk today is likely to be for, plus 247,
/// which is the one this template was written for.
const MACHINES: &[(i128, &str)] = &[
    (0, "none"),
    (2, "SPARC"),
    (3, "i386"),
    (4, "68000"),
    (8, "MIPS"),
    (18, "SPARC32PLUS"),
    (20, "PowerPC"),
    (21, "PowerPC 64"),
    (22, "S/390"),
    (40, "ARM"),
    (42, "SuperH"),
    (43, "SPARC v9"),
    (50, "Itanium"),
    (62, "x86-64"),
    (83, "AVR"),
    (94, "Xtensa"),
    (183, "AArch64"),
    (186, "STM8"),
    (220, "Z80"),
    (243, "RISC-V"),
    (247, "BPF"),
    (252, "CSKY"),
    (258, "LoongArch"),
];

const SECTION_TYPE: &[(i128, &str)] = &[
    (0, "null"),
    (1, "progbits"),
    (2, "symtab"),
    (3, "strtab"),
    (4, "rela"),
    (5, "hash"),
    (6, "dynamic"),
    (7, "note"),
    (8, "nobits"),
    (9, "rel"),
    (10, "shlib"),
    (11, "dynsym"),
    (14, "init_array"),
    (15, "fini_array"),
    (16, "preinit_array"),
    (17, "group"),
    (18, "symtab_shndx"),
    (0x6fff_fff5, "gnu_attributes"),
    (0x6fff_fff6, "gnu_hash"),
    (0x6fff_fffd, "gnu_verdef"),
    (0x6fff_fffe, "gnu_verneed"),
    (0x6fff_ffff, "gnu_versym"),
];

const SECTION_FLAGS: &[(u32, &str)] = &[
    (0, "write"),
    (1, "alloc"),
    (2, "execute"),
    (4, "merge"),
    (5, "strings"),
    (6, "info link"),
    (7, "link order"),
    (8, "OS nonconforming"),
    (9, "group"),
    (10, "TLS"),
    (11, "compressed"),
];

const SEGMENT_TYPE: &[(i128, &str)] = &[
    (0, "null"),
    (1, "load"),
    (2, "dynamic"),
    (3, "interp"),
    (4, "note"),
    (5, "shlib"),
    (6, "phdr"),
    (7, "TLS"),
    (0x6474_e550, "gnu_eh_frame"),
    (0x6474_e551, "gnu_stack"),
    (0x6474_e552, "gnu_relro"),
    (0x6474_e553, "gnu_property"),
];

const SEGMENT_FLAGS: &[(u32, &str)] = &[(0, "execute"), (1, "write"), (2, "read")];

/// What an entry of the dynamic section is. The tag is the whole of what says
/// how to read the word after it: an address, a size, a count, a set of
/// flags, or an offset into the string table the section links to.
const DYNAMIC_TAG: &[(i128, &str)] = &[
    (0, "null"),
    (1, "needed"),
    (2, "pltrelsz"),
    (3, "pltgot"),
    (4, "hash"),
    (5, "strtab"),
    (6, "symtab"),
    (7, "rela"),
    (8, "relasz"),
    (9, "relaent"),
    (10, "strsz"),
    (11, "syment"),
    (12, "init"),
    (13, "fini"),
    (14, "soname"),
    (15, "rpath"),
    (16, "symbolic"),
    (17, "rel"),
    (18, "relsz"),
    (19, "relent"),
    (20, "pltrel"),
    (21, "debug"),
    (22, "textrel"),
    (23, "jmprel"),
    (24, "bind_now"),
    (25, "init_array"),
    (26, "fini_array"),
    (27, "init_arraysz"),
    (28, "fini_arraysz"),
    (29, "runpath"),
    (30, "flags"),
    (32, "preinit_array"),
    (33, "preinit_arraysz"),
    (34, "symtab_shndx"),
    (35, "relrsz"),
    (36, "relr"),
    (37, "relrent"),
    (0x6fff_fef5, "gnu_hash"),
    (0x6fff_fff0, "versym"),
    (0x6fff_fff9, "relacount"),
    (0x6fff_fffa, "relcount"),
    (0x6fff_fffb, "flags_1"),
    (0x6fff_fffc, "verdef"),
    (0x6fff_fffd, "verdefnum"),
    (0x6fff_fffe, "verneed"),
    (0x6fff_ffff, "verneednum"),
];

/// The tags whose word is an offset into the dynamic string table: a library
/// to load, this library's own name, and the two lists of directories to look
/// for libraries in.
const DYNAMIC_NAMED: &[i128] = &[1, 14, 15, 29];

/// `DT_FLAGS`, the bits the standard defines.
const DYNAMIC_FLAGS: &[(u32, &str)] = &[(0, "origin"), (1, "symbolic"), (2, "textrel"), (3, "bind now"), (4, "static TLS")];

/// `DT_FLAGS_1`, the bits the GNU and Solaris linkers added after it. `now`
/// says the same thing as `DT_FLAGS`' `bind now`, and `pie` is the one way an
/// executable built to load anywhere says so in a word of its own.
const DYNAMIC_FLAGS_1: &[(u32, &str)] = &[
    (0, "now"),
    (1, "global"),
    (2, "group"),
    (3, "nodelete"),
    (4, "loadfltr"),
    (5, "initfirst"),
    (6, "noopen"),
    (7, "origin"),
    (8, "direct"),
    (9, "trans"),
    (10, "interpose"),
    (11, "nodeflib"),
    (12, "nodump"),
    (13, "confalt"),
    (14, "endfiltee"),
    (15, "dispreldne"),
    (16, "disprelpnd"),
    (17, "nodirect"),
    (18, "ignmuldef"),
    (19, "noksyms"),
    (20, "nohdr"),
    (21, "edited"),
    (22, "noreloc"),
    (23, "symintpose"),
    (24, "globaudit"),
    (25, "singleton"),
    (26, "stub"),
    (27, "pie"),
];

/// What a note owned by `GNU` is. A note's type means something only to the
/// owner whose name it carries, so these names are given only to a note whose
/// name is `GNU`.
const GNU_NOTE: &[(i128, &str)] = &[(1, "ABI tag"), (3, "build ID"), (4, "gold version"), (5, "property")];

/// `GNU\0` read as one big-endian word, which is how a note's type field asks
/// whether the name after it is GNU's.
const GNU_NAME: i128 = 0x474e_5500;

const SYMBOL_BIND: &[(i128, &str)] = &[(0, "local"), (1, "global"), (2, "weak"), (10, "GNU unique")];

const SYMBOL_TYPE: &[(i128, &str)] = &[
    (0, "notype"),
    (1, "object"),
    (2, "function"),
    (3, "section"),
    (4, "file"),
    (5, "common"),
    (6, "TLS"),
    (10, "GNU ifunc"),
];

const SYMBOL_VISIBILITY: &[(i128, &str)] = &[(0, "default"), (1, "internal"), (2, "hidden"), (3, "protected")];

/// The relocations an eBPF object uses. There are few of them because there
/// is little to relocate: a map is a 64-bit load whose immediate the loader
/// fills in, and a call to another program in the same object is a jump.
const BPF_RELOCATIONS: &[(i128, &str)] =
    &[(0, "R_BPF_NONE"), (1, "R_BPF_64_64"), (2, "R_BPF_64_ABS64"), (3, "R_BPF_64_ABS32"), (4, "R_BPF_64_NODYLD32"), (10, "R_BPF_64_32")];

/// The machines whose instructions this can read, by the number the header
/// writes. A file for anything else keeps its code as bytes, which is honest:
/// a decoder that does not know the machine would make words up.
///
/// 32-bit ARM is not here, because one number covers two encodings and the
/// header says which; see [`arm_code`].
fn decoded(bits: u32) -> Vec<(i128, Isa)> {
    vec![
        (3, Isa::X86_32),
        (62, Isa::X86_64),
        (183, Isa::Aarch64),
        // One number for the machine either way: how wide its registers are
        // is what the class already said.
        (243, if bits == 64 { Isa::Riscv64 } else { Isa::Riscv32 }),
    ]
}

/// 32-bit ARM code, in whichever of the machine's two encodings this file
/// uses.
///
/// One machine number covers both, and the entry point says which: the address
/// a program starts at has its lowest bit set when it starts in the two-byte
/// encoding, because that bit is not part of the address but the machine's
/// note to itself about which mode to enter. A microcontroller has only the
/// two-byte encoding and so always sets it.
///
/// A program can hold both and mark the boundaries with `$a` and `$t` symbols.
/// Following those is work for later; taking the entry point's word for it is
/// right for every file that is all one or all the other, which a
/// microcontroller's is.
fn arm_code(size: &impl Fn() -> E) -> T {
    let whole = |isa| T::sized(size(), T::repeat(T::insn(isa), Until::End));
    T::switch(E::field("entry").bit(0), vec![(1, whole(Isa::Thumb))], whole(Isa::Arm))
}

/// Any ELF file, with the code in it read as instructions when the machine is
/// one this knows.
pub fn elf() -> Template {
    Template::new("elf", ident())
}

/// An ELF object for the BPF machine. The same template under another name, so
/// that what the file is called and what the naming pass runs on agree.
pub fn bpf() -> Template {
    Template::new("bpf", ident())
}

/// The sixteen bytes every ELF opens with, and then the header the class and
/// the data encoding between them select.
fn ident() -> T {
    T::structure(
        "ELF",
        vec![
            ("magic", T::magic(b"\x7fELF")),
            ("class", T::enumeration("Class", T::u8(), CLASS)),
            ("data", T::enumeration("Data", T::u8(), DATA)),
            // The version of the format, which has been 1 since 1995.
            ("ident_version", T::u8()),
            ("osabi", T::enumeration("OSABI", T::u8(), OSABI)),
            ("abi_version", T::u8()),
            ("padding", T::bytes(E::lit(7))),
            (
                "header",
                T::switch(
                    E::field("class"),
                    vec![
                        (1, by_endian(32)),
                        (2, by_endian(64)),
                    ],
                    T::bytes(E::lit(0)),
                ),
            ),
        ],
    )
    .field_doc("padding", "Reserved: the rest of the 16-byte identification, unused since the first ELF and written as zeros.")
}

fn by_endian(bits: u32) -> T {
    T::switch(
        E::field("data"),
        vec![(1, body(bits, Little)), (2, body(bits, Big))],
        T::bytes(E::lit(0)),
    )
}

/// A word as wide as an address on this machine: four bytes in a 32-bit file,
/// eight in a 64-bit one. The header, the section headers and the program
/// headers are all mostly made of these.
fn addr(bits: u32, e: Endian) -> T {
    if bits == 64 { T::u64(e) } else { T::u32(e) }
}

/// How many bytes one entry of a table takes, which is what turns a section's
/// size into a count of what is in it. The file writes this in the section
/// header as well; using the known value keeps the count readable when a
/// stripped or damaged file writes zero there and the division would fail.
fn sym_size(bits: u32) -> i128 {
    if bits == 64 { 24 } else { 16 }
}

fn rel_size(bits: u32) -> i128 {
    if bits == 64 { 16 } else { 8 }
}

fn rela_size(bits: u32) -> i128 {
    if bits == 64 { 24 } else { 12 }
}

/// The rest of the header, the two tables it points at, and the sections
/// themselves.
fn body(bits: u32, e: Endian) -> T {
    T::structure(
        "ELFHeader",
        vec![
            ("type", T::enumeration("ObjectType", T::u16(e), OBJECT_TYPE)),
            ("machine", T::enumeration("Machine", T::u16(e), MACHINES)),
            ("version", T::u32(e)),
            ("entry", addr(bits, e)),
            ("program_header_offset", addr(bits, e)),
            ("section_header_offset", addr(bits, e)),
            ("flags", T::u32(e)),
            ("header_size", T::u16(e)),
            ("program_header_entry_size", T::u16(e)),
            ("program_header_count", T::u16(e)),
            ("section_header_entry_size", T::u16(e)),
            ("section_header_count", T::u16(e)),
            // Which section holds the section names.
            ("section_name_table", T::u16(e)),
            // Where that section's bytes start.
            //
            // Every section in the file is named by a string in one section,
            // and the header of that section is the only place its offset is
            // written. Reading it through `section_headers` would mean an
            // element of that array reaching back into the array it is in, so
            // it is read straight out of the file instead: the table starts at
            // `section_header_offset`, its entries are `section_header_entry_
            // size` apart, and the `offset` field sits a fixed distance into
            // one, because the ELF spec fixes that layout and `section_header`
            // below is only writing it down again.
            //
            // It occupies no bytes: `at` places a field somewhere else and
            // leaves the cursor where it was.
            (
                "section_name_base",
                T::at(
                    E::field("section_header_offset")
                        .add(E::field("section_name_table").mul(E::field("section_header_entry_size")))
                        .add(E::lit(if bits == 64 { 24 } else { 16 })),
                    addr(bits, e),
                ),
            ),
            // Both tables are read where they sit without moving the cursor,
            // so what they say is in hand before the sections are placed.
            (
                "program_headers",
                T::at(
                    E::field("program_header_offset"),
                    T::array(program_header(bits, e), E::field("program_header_count")),
                ),
            ),
            (
                "section_headers",
                T::at(
                    E::field("section_header_offset"),
                    T::array(section_header(bits, e), E::field("section_header_count")),
                ),
            ),
            // Every section at the offset its own header gives. Section 0 is
            // the null section and points nowhere, which is what
            // `skipping_zero` is for.
            (
                "sections",
                T::pointer_list_sized(
                    "section_headers",
                    &["offset"],
                    Anchor::File,
                    E::lit(0),
                    section_body(bits, e),
                )
                .skipping_zero(),
            ),
        ],
    )
    // Those eight bytes are one field of one section header, read a second
    // time from a fixed place in the table. The header owns them.
    .field_aside("section_name_base")
}

fn program_header(bits: u32, e: Endian) -> T {
    // The one place where 64-bit is not 32-bit with wider words: the flags
    // move up next to the type, to sit in what would otherwise be padding.
    let mut fields: Vec<(&str, T)> = vec![("type", T::enumeration("SegmentType", T::u32(e), SEGMENT_TYPE))];
    if bits == 64 {
        fields.push(("flags", T::flags("SegmentFlags", T::u32(e), SEGMENT_FLAGS)));
    }
    fields.extend([
        ("offset", addr(bits, e)),
        ("virtual_address", addr(bits, e)),
        ("physical_address", addr(bits, e)),
        ("file_size", addr(bits, e)),
        ("memory_size", addr(bits, e)),
    ]);
    if bits == 32 {
        fields.push(("flags", T::flags("SegmentFlags", T::u32(e), SEGMENT_FLAGS)));
    }
    fields.push(("align", addr(bits, e)));
    T::structure("ProgramHeader", fields).named_by("type").counted_as("segment").reads_as(&[
        ("type", "", ""),
        ("flags", "", ""),
        ("file_size", "{} bytes in the file", ""),
        ("memory_size", "{} bytes in memory", ""),
    ])
}

fn section_header(bits: u32, e: Endian) -> T {
    // Named by the string it points at, which is the whole reason for the
    // `name` field below: a header called `[2]` says nothing, and the section
    // its offset places borrows its label, so naming the record here is what
    // puts `.text` on the megabyte of code as well as on the header.
    T::structure_named(
        "SectionHeader",
        "name",
        "",
        vec![
            // An offset into the section name table, not a name.
            ("name_offset", T::u32(e)),
            // The name itself, read where the section name table keeps it, the
            // way a device tree reads every property name out of its strings
            // block. `section_name_base` is a field of the header above, which
            // an expression reaches by looking out through the structures this
            // one is nested in.
            //
            // A base of nought means there is no name table. The spec spells
            // that `SHN_UNDEF` and writes it as section 0, whose header is all
            // zeros, so the base comes out nought either way. Reading a name
            // from offset nought would hand back the file's own magic bytes
            // and call the section `\x7fELF`, which is worse than `[1]`.
            (
                "name",
                T::switch(
                    E::field("section_name_base"),
                    vec![(0, T::bytes(E::lit(0)))],
                    T::at(E::field("section_name_base").add(E::field("name_offset")), T::cstr()),
                ),
            ),
            ("type", T::enumeration("SectionType", T::u32(e), SECTION_TYPE)),
            ("flags", T::flags("SectionFlags", addr(bits, e), SECTION_FLAGS)),
            ("address", addr(bits, e)),
            ("offset", addr(bits, e)),
            ("size", addr(bits, e)),
            ("link", T::u32(e)),
            ("info", T::u32(e)),
            ("align", addr(bits, e)),
            ("entry_size", addr(bits, e)),
        ],
    )
    // The name is read out of the section name table, so those bytes belong to
    // that section and are counted there. See `Field::aside`.
    .field_aside("name")
    .counted_as("section")
    // The name is the record's own name, so the line is what kind of section
    // it is and how big.
    .reads_as(&[("type", "", ""), ("flags", "", ""), ("size", "{} bytes", "")])
}

/// What is in a section, read as its type says. Anything unrecognised is the
/// bytes, which is what a section of program data is.
fn section_body(bits: u32, e: Endian) -> T {
    let size = || E::elem_field("section_headers", E::idx(), &["size"]);
    let raw = T::bytes(size());
    // Program bits marked executable are code. Only the BPF template reads
    // them as instructions, and only for exactly `alloc | execute`, which is
    // what a compiler writes for a program section.
    // A section of program bits marked executable is code. Which machine's
    // code it is, the header said, and that is a field this switch can reach:
    // an expression looks through the structures it is nested in, and the
    // machine is one of the header's own fields.
    let mut machines: Vec<(i128, T)> = vec![(247, T::sized(size(), T::repeat(instruction(e), Until::End)))];
    for (machine, isa) in decoded(bits) {
        machines.push((machine, T::sized(size(), T::repeat(T::insn(isa), Until::End))));
    }
    machines.push((40, arm_code(&size)));
    let progbits = T::switch(
        // The bit that says the section holds instructions. The rest of the
        // word says whether it is written to, aligned or grouped, and a
        // linker writes those as it likes.
        E::elem_field("section_headers", E::idx(), &["flags"]).bit(2),
        vec![(1, T::switch(E::field("machine"), machines, T::bytes(size())))],
        T::bytes(size()),
    );
    T::switch(
        E::elem_field("section_headers", E::idx(), &["type"]),
        vec![
            (1, progbits),
            (2, T::array(symbol(bits, e), size().div(E::lit(sym_size(bits))))),
            (3, T::sized(size(), T::repeat(T::cstr(), Until::End))),
            (4, T::array(relocation(bits, e, true), size().div(E::lit(rela_size(bits))))),
            (6, dynamic(bits, e)),
            (7, T::sized(size(), T::repeat(note(e), Until::End))),
            // A section with no bits in the file. Its size is what it will
            // take in memory, and reading that many bytes here would read
            // whatever follows it in the file instead.
            (8, T::bytes(E::lit(0))),
            (9, T::array(relocation(bits, e, false), size().div(E::lit(rel_size(bits))))),
            (11, T::array(symbol(bits, e), size().div(E::lit(sym_size(bits))))),
        ],
        raw,
    )
}

fn symbol(bits: u32, e: Endian) -> T {
    // `info` is two fields in one byte: what kind of symbol it is, and how
    // far it binds. Read as the two it is.
    let info: Vec<(&str, T)> = vec![
        ("binding", T::enumeration("SymbolBinding", T::UInt { bits: 4, endian: Big }, SYMBOL_BIND)),
        ("type", T::enumeration("SymbolType", T::UInt { bits: 4, endian: Big }, SYMBOL_TYPE)),
    ];
    let visibility = T::enumeration("SymbolVisibility", T::u8(), SYMBOL_VISIBILITY);
    let mut fields: Vec<(&str, T)> = vec![("name_offset", T::u32(e))];
    if bits == 64 {
        fields.push(("info", T::inline_structure("SymbolInfo", info)));
        fields.push(("visibility", visibility));
        fields.push(("section_index", T::u16(e)));
        fields.push(("value", T::u64(e)));
        fields.push(("size", T::u64(e)));
    } else {
        fields.push(("value", T::u32(e)));
        fields.push(("size", T::u32(e)));
        fields.push(("info", T::inline_structure("SymbolInfo", info)));
        fields.push(("visibility", visibility));
        fields.push(("section_index", T::u16(e)));
    }
    T::structure("Symbol", fields).counted_as("symbol")
}

/// The dynamic section: what the loader reads to put a program together, as a
/// list of tagged words ending in a `null` one.
///
/// Four of the tags name something, a library or a directory, and the word
/// after them is an offset into the string table this section's header links
/// to. That table's offset is written in its own section header and nowhere
/// else, so it is read the way `section_name_base` is: straight out of the
/// section header table, at `link` entries in, rather than through
/// `section_headers`. `strings_at` is then a field of this structure, and each
/// entry reads its name from there by looking outward through the list it is
/// in.
fn dynamic(bits: u32, e: Endian) -> T {
    let link = E::elem_field("section_headers", E::idx(), &["link"]);
    let size = E::elem_field("section_headers", E::idx(), &["size"]);
    let word = if bits == 64 { 8 } else { 4 };
    T::structure(
        "Dynamic",
        vec![
            (
                "strings_at",
                T::at(
                    E::field("section_header_offset")
                        .add(link.mul(E::field("section_header_entry_size")))
                        .add(E::lit(if bits == 64 { 24 } else { 16 })),
                    addr(bits, e),
                ),
            ),
            ("entries", T::array(dynamic_entry(bits, e), size.div(E::lit(2 * word)))),
        ],
    )
    // One field of the string table's own header, read a second time. The
    // header owns those bytes.
    .field_aside("strings_at")
}

/// One entry of the dynamic section: a tag, and a word the tag says how to
/// read.
fn dynamic_entry(bits: u32, e: Endian) -> T {
    let named = DYNAMIC_NAMED
        .iter()
        .map(|&tag| E::field("tag").equal_to(E::lit(tag)))
        .reduce(|a, b| a.either(b))
        .expect("at least one tag names something");
    T::structure(
        "DynamicEntry",
        vec![
            ("tag", T::enumeration_hex("DynamicTag", addr(bits, e), DYNAMIC_TAG)),
            (
                "value",
                T::switch(
                    E::field("tag"),
                    vec![
                        (30, T::flags("DynamicFlags", addr(bits, e), DYNAMIC_FLAGS)),
                        (0x6fff_fffb, T::flags("DynamicFlags1", addr(bits, e), DYNAMIC_FLAGS_1)),
                    ],
                    addr(bits, e),
                ),
            ),
            // The library or directory the word points at, for the four tags
            // whose word is an offset into the string table. Not there for the
            // rest, and not there either when the section links to no string
            // table: section 0's offset is nought, and a name read from there
            // would be the file's own magic.
            (
                "name",
                T::when(
                    E::field("strings_at").not_equal(E::lit(0)).both(named),
                    T::at(E::field("strings_at").add(E::field("value")), T::cstr()),
                ),
            ),
        ],
    )
    .named_by("tag")
    // The name's bytes belong to the string table and are counted there.
    .field_aside("name")
    .counted_as("entry")
    // Named by its tag, so the line is what the tag says: the library for a
    // `needed`, the bits for `flags`, and the number for the rest. A `needed`
    // shows its offset after the name, which is the number the file holds.
    .reads_as(&[("name", "", ""), ("value", "", "")])
}

/// One note: who wrote it, what kind it is, and what it says. A note's type is
/// a number whose meaning belongs to the owner named after it, so the type is
/// read as one of GNU's only where the four bytes after it are `GNU\0`, which
/// is what the name is when it is GNU's. Asking that before the name has been
/// read is what `peek_at` is for: the name starts four bytes past where the
/// type does.
///
/// The name and the description are each padded to a multiple of four bytes.
/// A note segment aligned to eight may pad to eight instead; the GNU notes this
/// is read for have sizes that come out the same either way.
fn note(e: Endian) -> T {
    let gnu = E::field("name_size").equal_to(E::lit(4)).both(E::peek_at(E::lit(32), 32, Big).equal_to(E::lit(GNU_NAME)));
    T::structure(
        "Note",
        vec![
            ("name_size", T::u32(e)),
            ("desc_size", T::u32(e)),
            ("type", T::switch(gnu, vec![(1, T::enumeration("GnuNoteType", T::u32(e), GNU_NOTE))], T::u32(e))),
            (
                "name",
                T::text(
                    StrLen::Padded { size: E::field("name_size").add(E::field("name_size").pad_to(4)), pad: 0 },
                    Encoding::Utf8,
                ),
            ),
            ("desc", T::bytes(E::field("desc_size"))),
            ("padding", T::bytes(E::field("desc_size").pad_to(4))),
        ],
    )
    .named_by("name")
    .counted_as("note")
    .reads_as(&[("type", "", ""), ("desc_size", "{} bytes", "")])
}

/// A relocation: where to patch, which symbol to patch it with, and how. The
/// two halves of `info` are a symbol index and a type, and how they are packed
/// depends on the class rather than on the machine.
fn relocation(bits: u32, e: Endian, addend: bool) -> T {
    let mut fields: Vec<(&str, T)> = vec![("offset", addr(bits, e))];
    if bits == 64 {
        // A 64-bit `info` reading the symbol and the type as two words is the
        // same bytes either way round, as long as each is read the way the
        // file writes its numbers.
        if e == Little {
            fields.push(("type", T::enumeration("RelocationType", T::u32(e), BPF_RELOCATIONS)));
            fields.push(("symbol", T::u32(e)));
        } else {
            fields.push(("symbol", T::u32(e)));
            fields.push(("type", T::enumeration("RelocationType", T::u32(e), BPF_RELOCATIONS)));
        }
    } else {
        fields.push(("info", T::u32(e)));
    }
    if addend {
        fields.push(("addend", addr(bits, e)));
    }
    T::structure(if addend { "RelocationAddend" } else { "Relocation" }, fields).counted_as("relocation")
}

/// One eBPF instruction: an opcode, two registers packed into a byte, a signed
/// offset and a signed immediate. Eight bytes, except for the load of a 64-bit
/// immediate, which is followed by a second word carrying the top half.
///
/// Which nibble of the register byte is which follows the file's byte order,
/// because the kernel declares them as bitfields and a compiler lays those out
/// low bits first on a little-endian target.
fn instruction(e: Endian) -> T {
    let reg = |name: &'static str| (name, T::enumeration("Register", T::UInt { bits: 4, endian: Big }, REGS));
    let (first, second) = if e == Little { (reg("src"), reg("dst")) } else { (reg("dst"), reg("src")) };
    // One row per instruction in the linear views rather than one per field:
    // an opcode, its registers and its immediate are one instruction.
    T::inline_structure(
        "BpfInsn",
        vec![
            ("opcode", T::enumeration_hex("BpfOpcode", T::u8(), OPCODES)),
            first,
            second,
            ("offset", T::Int { bits: 16, endian: e }),
            ("imm", T::Int { bits: 32, endian: e }),
            (
                "wide",
                T::switch(
                    E::field("opcode"),
                    vec![(
                        0x18,
                        T::structure(
                            "ImmediateHigh",
                            vec![("reserved", T::bytes(E::lit(4))), ("imm_high", T::Int { bits: 32, endian: e })],
                        ),
                    )],
                    T::bytes(E::lit(0)),
                ),
            ),
        ],
    )
    .counted_as("instruction")
}

/// Small ELF files assembled from parts, for the tests here and for the ones
/// that read what a program was built with.
#[cfg(test)]
pub(crate) mod fixture {
    /// A 64-bit little-endian ELF: the header, the program headers straight
    /// after it, each section's bytes in order eight-aligned, the section name
    /// table, and the section header table last. A section is loaded at the
    /// same address as its offset in the file, which keeps a segment that
    /// covers it simple to write down.
    #[derive(Debug, Clone, Default)]
    pub struct Elf {
        pub kind: u16,
        pub machine: u16,
        pub sections: Vec<Section>,
        pub segments: Vec<Segment>,
        /// Write no section header table, the way `sstrip` leaves a program.
        pub no_section_headers: bool,
    }

    #[derive(Debug, Clone, Default)]
    pub struct Section {
        pub name: String,
        pub kind: u32,
        pub flags: u64,
        /// The section linked to, by name.
        pub link: Option<String>,
        pub entry_size: u64,
        pub data: Vec<u8>,
    }

    /// What a segment covers.
    #[derive(Debug, Clone)]
    pub enum Covers {
        Nothing,
        Section(String),
        WholeFile,
    }

    #[derive(Debug, Clone)]
    pub struct Segment {
        pub kind: u32,
        pub flags: u32,
        pub covers: Covers,
    }

    pub const HEADER: u64 = 64;
    pub const PHDR: u64 = 56;
    pub const SHDR: u64 = 64;

    impl Elf {
        pub fn new(kind: u16, machine: u16) -> Elf {
            Elf { kind, machine, ..Elf::default() }
        }

        pub fn section(mut self, name: &str, kind: u32, data: Vec<u8>) -> Elf {
            self.sections.push(Section { name: name.into(), kind, data, ..Section::default() });
            self
        }

        /// A section that links to another by name, with the size of one of
        /// its entries.
        pub fn linked(mut self, name: &str, kind: u32, link: &str, entry_size: u64, data: Vec<u8>) -> Elf {
            self.sections.push(Section { name: name.into(), kind, link: Some(link.into()), entry_size, data, ..Section::default() });
            self
        }

        pub fn segment(mut self, kind: u32, flags: u32, covers: Covers) -> Elf {
            self.segments.push(Segment { kind, flags, covers });
            self
        }

        /// The section name table's bytes, and each section's name offset in
        /// it. The table is the last section.
        fn names(&self) -> (Vec<u8>, Vec<u32>) {
            let mut table = vec![0u8];
            let mut at = Vec::new();
            for s in &self.sections {
                at.push(table.len() as u32);
                table.extend_from_slice(s.name.as_bytes());
                table.push(0);
            }
            at.push(table.len() as u32);
            table.extend_from_slice(b".shstrtab\0");
            (table, at)
        }

        /// Where each section's bytes start, the name table's last, and where
        /// the section header table starts.
        fn layout(&self) -> (Vec<u64>, u64) {
            let (names, _) = self.names();
            let mut at = HEADER + PHDR * self.segments.len() as u64;
            let mut out = Vec::new();
            for len in self.sections.iter().map(|s| s.data.len()).chain([names.len()]) {
                at = at.next_multiple_of(8);
                out.push(at);
                at += len as u64;
            }
            (out, at.next_multiple_of(8))
        }

        /// Where the named section's bytes start in the built file.
        pub fn offset_of(&self, name: &str) -> u64 {
            let i = self.sections.iter().position(|s| s.name == name).expect("a section of that name");
            self.layout().0[i]
        }

        /// The index the named section has in the section header table.
        pub fn index_of(&self, name: &str) -> usize {
            1 + self.sections.iter().position(|s| s.name == name).expect("a section of that name")
        }

        pub fn build(&self) -> Vec<u8> {
            let (names, name_at) = self.names();
            let (offsets, headers_at) = self.layout();
            let count = self.sections.len() as u16 + 2;
            let mut v = b"\x7fELF\x02\x01\x01\x00".to_vec();
            v.extend_from_slice(&[0; 8]);
            v.extend_from_slice(&self.kind.to_le_bytes());
            v.extend_from_slice(&self.machine.to_le_bytes());
            v.extend_from_slice(&1u32.to_le_bytes());
            v.extend_from_slice(&0u64.to_le_bytes()); // entry
            v.extend_from_slice(&(if self.segments.is_empty() { 0 } else { HEADER }).to_le_bytes());
            v.extend_from_slice(&(if self.no_section_headers { 0 } else { headers_at }).to_le_bytes());
            v.extend_from_slice(&0u32.to_le_bytes()); // flags
            v.extend_from_slice(&64u16.to_le_bytes());
            v.extend_from_slice(&(PHDR as u16).to_le_bytes());
            v.extend_from_slice(&(self.segments.len() as u16).to_le_bytes());
            v.extend_from_slice(&(SHDR as u16).to_le_bytes());
            v.extend_from_slice(&(if self.no_section_headers { 0 } else { count }).to_le_bytes());
            v.extend_from_slice(&(if self.no_section_headers { 0 } else { count - 1 }).to_le_bytes());
            let total = headers_at + if self.no_section_headers { 0 } else { SHDR * count as u64 };
            for seg in &self.segments {
                let (at, len) = match &seg.covers {
                    Covers::Nothing => (0, 0),
                    Covers::Section(name) => (self.offset_of(name), self.sections[self.index_of(name) - 1].data.len() as u64),
                    Covers::WholeFile => (0, total),
                };
                v.extend_from_slice(&seg.kind.to_le_bytes());
                v.extend_from_slice(&seg.flags.to_le_bytes());
                for word in [at, at, at, len, len, 8] {
                    v.extend_from_slice(&word.to_le_bytes());
                }
            }
            for (data, at) in self.sections.iter().map(|s| s.data.as_slice()).chain([names.as_slice()]).zip(&offsets) {
                v.resize(*at as usize, 0);
                v.extend_from_slice(data);
            }
            v.resize(headers_at as usize, 0);
            if self.no_section_headers {
                return v;
            }
            v.extend_from_slice(&[0; SHDR as usize]);
            let strtab = Section { kind: 3, ..Section::default() };
            for (i, s) in self.sections.iter().chain([&strtab]).enumerate() {
                let len = if i == self.sections.len() { names.len() } else { s.data.len() } as u64;
                let link = s.link.as_deref().map_or(0, |l| self.index_of(l)) as u32;
                v.extend_from_slice(&name_at[i].to_le_bytes());
                v.extend_from_slice(&s.kind.to_le_bytes());
                v.extend_from_slice(&s.flags.to_le_bytes());
                v.extend_from_slice(&offsets[i].to_le_bytes()); // address
                v.extend_from_slice(&offsets[i].to_le_bytes());
                v.extend_from_slice(&len.to_le_bytes());
                v.extend_from_slice(&link.to_le_bytes());
                v.extend_from_slice(&0u32.to_le_bytes());
                v.extend_from_slice(&8u64.to_le_bytes());
                v.extend_from_slice(&s.entry_size.to_le_bytes());
            }
            v
        }
    }

    /// A dynamic section's bytes from its tags and words.
    pub fn dynamic(entries: &[(u64, u64)]) -> Vec<u8> {
        entries.iter().flat_map(|(tag, value)| [tag.to_le_bytes(), value.to_le_bytes()]).flatten().collect()
    }

    /// A string table holding `names`, and the offset of each in it.
    pub fn strings(names: &[&str]) -> (Vec<u8>, Vec<u64>) {
        let mut table = vec![0u8];
        let mut at = Vec::new();
        for n in names {
            at.push(table.len() as u64);
            table.extend_from_slice(n.as_bytes());
            table.push(0);
        }
        (table, at)
    }

    /// A symbol table whose symbols have these name offsets, after the null
    /// symbol every table starts with. Global functions, undefined.
    pub fn symbols(names: &[u64]) -> Vec<u8> {
        let mut v = vec![0u8; 24];
        for &n in names {
            v.extend_from_slice(&(n as u32).to_le_bytes());
            v.push(0x12);
            v.push(0);
            v.extend_from_slice(&0u16.to_le_bytes());
            v.extend_from_slice(&0u64.to_le_bytes());
            v.extend_from_slice(&0u64.to_le_bytes());
        }
        v
    }

    /// One note, its name and description each padded to four bytes.
    pub fn note(name: &str, kind: u32, desc: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&(name.len() as u32 + 1).to_le_bytes());
        v.extend_from_slice(&(desc.len() as u32).to_le_bytes());
        v.extend_from_slice(&kind.to_le_bytes());
        v.extend_from_slice(name.as_bytes());
        v.push(0);
        v.resize(v.len().next_multiple_of(4), 0);
        v.extend_from_slice(desc);
        v.resize(v.len().next_multiple_of(4), 0);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// A little-endian 64-bit object for the BPF machine, with one executable
    /// section holding two instructions and one section with no bits in the
    /// file at all.
    pub(super) fn object() -> Vec<u8> {
        let text: Vec<u8> = vec![
            // r1 = 2
            0xb7, 0x01, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, //
            // exit
            0x95, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let mut v = b"\x7fELF".to_vec();
        v.extend_from_slice(&[2, 1, 1, 0, 0]);
        v.extend_from_slice(&[0; 7]);
        v.extend_from_slice(&1u16.to_le_bytes()); // relocatable
        v.extend_from_slice(&247u16.to_le_bytes()); // BPF
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend_from_slice(&0u64.to_le_bytes()); // entry
        v.extend_from_slice(&0u64.to_le_bytes()); // program header offset
        v.extend_from_slice(&80u64.to_le_bytes()); // section header offset
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&64u16.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&64u16.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes()); // two section headers
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&text);
        // Section 0 is the null section, which points nowhere.
        v.extend_from_slice(&[0; 64]);
        // Section 1: program bits, allocated and executable, the code above.
        let mut shdr = Vec::new();
        shdr.extend_from_slice(&0u32.to_le_bytes()); // name offset
        shdr.extend_from_slice(&1u32.to_le_bytes()); // progbits
        shdr.extend_from_slice(&6u64.to_le_bytes()); // alloc | execute
        shdr.extend_from_slice(&0u64.to_le_bytes()); // address
        shdr.extend_from_slice(&64u64.to_le_bytes()); // offset
        shdr.extend_from_slice(&(text.len() as u64).to_le_bytes());
        shdr.extend_from_slice(&0u32.to_le_bytes());
        shdr.extend_from_slice(&0u32.to_le_bytes());
        shdr.extend_from_slice(&8u64.to_le_bytes());
        shdr.extend_from_slice(&0u64.to_le_bytes());
        v.extend_from_slice(&shdr);
        v
    }

    #[test]
    fn an_executable_section_reads_as_instructions() {
        let d = Document::new(MemSource(object()));
        let mut ev = Evaluator::new(bpf());
        assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::Enum { raw: 2, name: Some("64-bit".into()), hex: false });
        // The code is at the offset its own section header gives.
        let code = ev.node(&d, &[7, 16, 1]).unwrap();
        assert_eq!(code.type_name, "BpfInsn[]");
        assert_eq!(code.offset_bits, 64 * 8);
        assert_eq!(code.child_count, 2);
        // `r1 = 2`: the destination is in the low nibble of the second byte,
        // which is what a little-endian object writes.
        assert_eq!(ev.node(&d, &[7, 16, 1, 0, 2]).unwrap().value, Value::Enum { raw: 1, name: Some("r1".into()), hex: false });
        assert_eq!(ev.node(&d, &[7, 16, 1, 0, 4]).unwrap().value, Value::Int(2));
    }

    /// A table a header points at is somewhere for a reason, and the reason is
    /// a field of the header. Asked of the table itself, which is the node the
    /// cursor lands on: the field that declares it covers no bytes.
    #[test]
    fn a_table_the_header_points_at_names_the_offset_that_placed_it() {
        use crate::eval::{Placed, Role, Sizing};
        let d = Document::new(MemSource(object()));
        let mut ev = Evaluator::new(elf());
        let table = ev.origins(&d, &[7, 15, 0]).unwrap();
        let roles: Vec<_> = table.iter().map(|x| (x.role, x.label.as_str(), x.value.as_str())).collect();
        assert!(roles.contains(&(Role::Position, "section_header_offset", "80")), "{roles:?}");
        assert!(roles.contains(&(Role::Count, "section_header_count", "2")), "{roles:?}");
        // Not from the field as the header declares it: that one covers no
        // bytes where it sits, and the offset it names placed its contents
        // rather than it. Saying so there would put `section_header_offset`
        // under a position of 0x40, which is where the declaration is.
        let declared: Vec<_> = ev.origins(&d, &[7, 15]).unwrap().into_iter().map(|x| x.role).collect();
        assert!(!declared.contains(&Role::Position), "{declared:?}");
        // Placed at an address the file gave, and as long as the count says.
        let shape = ev.shape(&d, &[7, 15, 0]).unwrap();
        assert_eq!((shape.placed, shape.sized), (Placed::Address, Sizing::Count));
        // A field of the header, for contrast: after the one before it, and as
        // wide as its own type. `Type` rather than `Fixed`: a `u16` is two
        // bytes because it is a `u16`, and the panel says nothing about that
        // because the type is written beside the length already.
        let plain = ev.shape(&d, &[7, 5]).unwrap();
        assert_eq!((plain.placed, plain.sized), (Placed::Follows, Sizing::Type));
        // A section's bytes are wherever its own header said.
        let code = ev.shape(&d, &[7, 16, 1]).unwrap();
        assert_eq!(code.placed, Placed::Pointer);
    }

    /// The same object with the machine changed to x86-64, whose code the
    /// header's own field is what picks a decoder for.
    #[test]
    fn the_machine_in_the_header_picks_the_decoder() {
        let mut bytes = object();
        bytes[18] = 62; // x86-64
        // mov eax, 1 (five bytes) and ret (one).
        bytes[64..80].copy_from_slice(&[
            0xb8, 0x01, 0x00, 0x00, 0x00, 0xc3, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90,
        ]);
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(elf());
        let code = ev.node(&d, &[7, 16, 1]).unwrap();
        assert_eq!(code.type_name, "x86-64[]");
        assert_eq!(ev.node(&d, &[7, 16, 1, 0]).unwrap().value, Value::Str("mov eax, 0x1".into()));
        assert_eq!(ev.node(&d, &[7, 16, 1, 0]).unwrap().size_bits, 5 * 8);
        assert_eq!(ev.node(&d, &[7, 16, 1, 1]).unwrap().value, Value::Str("ret".into()));
    }

    /// A long run of anything else is one row saying how many there are. A
    /// program is the exception: the rows are what there is to read.
    #[test]
    fn every_instruction_gets_its_own_row() {
        let mut bytes = object();
        bytes[18] = 62; // x86-64
        bytes[64..80].copy_from_slice(&[0x90; 16]); // sixteen nops
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(elf());
        let rows = ev.spans(&d, 64 * 8, 80 * 8, 32).unwrap();
        assert_eq!(rows.len(), 16, "{rows:?}");
        assert!(rows.iter().all(|r| r.type_name == "x86-64"), "{rows:?}");
    }

    /// A machine with no decoder here keeps its code as bytes rather than
    /// having words invented for it.
    #[test]
    fn a_machine_with_no_decoder_keeps_its_bytes() {
        let mut bytes = object();
        bytes[18] = 22; // S/390
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(elf());
        assert_eq!(ev.node(&d, &[7, 16, 1]).unwrap().type_name, "bytes[]");
    }

    /// A file with no section name table keeps its indexes. Section 0 is all
    /// zeros, so the base reads nought, and a name read from offset nought
    /// would be the file's own magic: `[1] \x7fELF` is worse than `[1]`.
    #[test]
    fn a_section_with_no_name_table_keeps_its_index() {
        let d = Document::new(MemSource(object()));
        let mut ev = Evaluator::new(elf());
        assert_eq!(ev.node(&d, &[7, 16, 1]).unwrap().name, "[1]");
    }

    /// A section is called what the section name table calls it, and so are the
    /// bytes its header places. `[1]` said nothing; `.text` says what the
    /// biggest box on the treemap is.
    #[test]
    fn a_section_is_named_by_the_section_name_table() {
        let d = Document::new(MemSource(named_sections()));
        let mut ev = Evaluator::new(elf());
        // The header carries the name, read from the far end of the file.
        assert_eq!(ev.node(&d, &[7, 15, 0, 1]).unwrap().name, "[1] .text");
        // And the bytes that header places borrow it, which is the whole point:
        // a megabyte of code called `[2]` is a box nobody can place.
        assert_eq!(ev.node(&d, &[7, 16, 1]).unwrap().name, "[1] .text");
        assert_eq!(ev.node(&d, &[7, 16, 2]).unwrap().name, "[2] .shstrtab");
    }

    /// A 64-bit little-endian object with three section headers: the null one,
    /// a `.text` of four bytes, and the name table that names them.
    fn named_sections() -> Vec<u8> {
        let names = b"\0.text\0.shstrtab\0";
        let text = [0x90u8, 0x90, 0x90, 0xc3];
        let headers_at = 64 + text.len() + names.len();
        let mut v = b"\x7fELF\x02\x01\x01\x00".to_vec();
        v.extend_from_slice(&[0; 8]);
        v.extend_from_slice(&1u16.to_le_bytes()); // relocatable
        v.extend_from_slice(&62u16.to_le_bytes()); // x86-64
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend_from_slice(&0u64.to_le_bytes()); // entry
        v.extend_from_slice(&0u64.to_le_bytes()); // program header offset
        v.extend_from_slice(&(headers_at as u64).to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes()); // flags
        v.extend_from_slice(&64u16.to_le_bytes()); // header size
        v.extend_from_slice(&0u16.to_le_bytes()); // program header entry size
        v.extend_from_slice(&0u16.to_le_bytes()); // program header count
        v.extend_from_slice(&64u16.to_le_bytes()); // section header entry size
        v.extend_from_slice(&3u16.to_le_bytes()); // section header count
        v.extend_from_slice(&2u16.to_le_bytes()); // the name table is section 2
        v.extend_from_slice(&text);
        v.extend_from_slice(names);
        // name offset, type, flags, address, offset, size, link, info, align,
        // entry size: the ten words of a 64-bit section header.
        let shdr = |name: u32, kind: u32, flags: u64, at: u64, size: u64| {
            let mut h = Vec::new();
            h.extend_from_slice(&name.to_le_bytes());
            h.extend_from_slice(&kind.to_le_bytes());
            h.extend_from_slice(&flags.to_le_bytes());
            h.extend_from_slice(&0u64.to_le_bytes());
            h.extend_from_slice(&at.to_le_bytes());
            h.extend_from_slice(&size.to_le_bytes());
            h.extend_from_slice(&0u32.to_le_bytes());
            h.extend_from_slice(&0u32.to_le_bytes());
            h.extend_from_slice(&1u64.to_le_bytes());
            h.extend_from_slice(&0u64.to_le_bytes());
            h
        };
        v.extend(shdr(0, 0, 0, 0, 0));
        v.extend(shdr(1, 1, 6, 64, text.len() as u64));
        v.extend(shdr(7, 3, 0, 64 + text.len() as u64, names.len() as u64));
        v
    }

    /// A shared library's dynamic section and the string table it links to:
    /// a library it needs, a directory to look in, and the two words of flags.
    fn dynamic_library() -> fixture::Elf {
        let (strings, at) = fixture::strings(&["libc.so.6", "$ORIGIN/lib"]);
        let entries = fixture::dynamic(&[
            (1, at[0]),         // needed
            (29, at[1]),        // runpath
            (30, 0x8),          // flags: bind now
            (0x6fff_fffb, 0x0800_0001), // flags_1: now, pie
            (0, 0),
        ]);
        fixture::Elf::new(3, 62).section(".dynstr", 3, strings).linked(".dynamic", 6, ".dynstr", 16, entries)
    }

    /// Every entry of the dynamic section named by its tag, and a library
    /// named by the string the entry's word points at. The string table's
    /// offset is a field of the section, and an entry two lists further in
    /// reaches it by looking outward.
    #[test]
    fn a_dynamic_section_names_the_libraries_it_needs() {
        let elf = dynamic_library();
        let d = Document::new(MemSource(elf.build()));
        let mut ev = Evaluator::new(super::elf());
        let at = elf.index_of(".dynamic");
        let body = vec![7, 16, at];
        assert_eq!(ev.node(&d, &body).unwrap().type_name, "Dynamic");
        // `at` is a node with the field it placed as its one child.
        let strings_at = ev.child_named(&d, &body, "strings_at").unwrap().unwrap();
        assert_eq!(ev.node(&d, &[strings_at.as_slice(), &[0]].concat()).unwrap().value, Value::UInt(elf.offset_of(".dynstr") as u128));
        let entries = ev.child_named(&d, &body, "entries").unwrap().unwrap();
        assert_eq!(ev.node(&d, &entries).unwrap().child_count, 5);
        let entry = |i: usize| [entries.as_slice(), &[i]].concat();
        assert_eq!(ev.node(&d, &entry(0)).unwrap().name, "[0] needed");
        let name = |ev: &mut Evaluator, i: usize| [ev.child_named(&d, &entry(i), "name").unwrap().unwrap(), vec![0]].concat();
        let first = name(&mut ev, 0);
        assert_eq!(ev.node(&d, &first).unwrap().value, Value::Str("libc.so.6".into()));
        // Its bytes are the string table's, read from there.
        assert_eq!(ev.node(&d, &first).unwrap().offset_bits, (elf.offset_of(".dynstr") + 1) * 8);
        let second = name(&mut ev, 1);
        assert_eq!(ev.node(&d, &second).unwrap().value, Value::Str("$ORIGIN/lib".into()));
        let flags = ev.child_named(&d, &entry(2), "value").unwrap().unwrap();
        assert!(matches!(ev.node(&d, &flags).unwrap().value, Value::Flags { set, .. } if set == ["bind now"]));
        let flags = ev.child_named(&d, &entry(3), "value").unwrap().unwrap();
        assert!(matches!(ev.node(&d, &flags).unwrap().value, Value::Flags { set, .. } if set == ["now", "pie"]));
        // A tag whose word is not an offset into the strings has no name.
        let name = ev.child_named(&d, &entry(2), "name").unwrap().unwrap();
        assert!(ev.node(&d, &name).unwrap().absent);
    }

    /// A note is called what its owner calls itself, and its type is read as
    /// one of GNU's only when the owner is GNU: type 3 of a Go note is not a
    /// build ID.
    #[test]
    fn a_note_is_named_by_its_owner_and_typed_by_it() {
        let mut notes = fixture::note("GNU", 3, &[0xab; 20]);
        notes.extend(fixture::note("Go", 3, b"go-build-id"));
        let elf = fixture::Elf::new(2, 62).section(".note", 7, notes);
        let d = Document::new(MemSource(elf.build()));
        let mut ev = Evaluator::new(super::elf());
        let body = vec![7, 16, elf.index_of(".note")];
        assert_eq!(ev.node(&d, &body).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[body.as_slice(), &[0]].concat()).unwrap().name, "[0] GNU");
        let kind = ev.child_named(&d, &[body.as_slice(), &[0]].concat(), "type").unwrap().unwrap();
        assert_eq!(ev.node(&d, &kind).unwrap().value, Value::Enum { raw: 3, name: Some("build ID".into()), hex: false });
        let desc = ev.child_named(&d, &[body.as_slice(), &[0]].concat(), "desc").unwrap().unwrap();
        assert_eq!(ev.node(&d, &desc).unwrap().size_bits, 20 * 8);
        // The Go note's description is eleven bytes, padded to twelve.
        let go = [body.as_slice(), &[1]].concat();
        assert_eq!(ev.node(&d, &go).unwrap().name, "[1] Go");
        let kind = ev.child_named(&d, &go, "type").unwrap().unwrap();
        assert_eq!(ev.node(&d, &kind).unwrap().value, Value::UInt(3));
        let padding = ev.child_named(&d, &go, "padding").unwrap().unwrap();
        assert_eq!(ev.node(&d, &padding).unwrap().size_bits, 8);
    }

    /// A big-endian 32-bit header, which is the other end of what the switches
    /// cover: the words are half as wide and the numbers read the other way.
    #[test]
    fn a_big_endian_32_bit_header_reads_its_own_way() {
        let mut v = b"\x7fELF".to_vec();
        v.extend_from_slice(&[1, 2, 1, 0, 0]);
        v.extend_from_slice(&[0; 7]);
        v.extend_from_slice(&2u16.to_be_bytes()); // executable
        v.extend_from_slice(&40u16.to_be_bytes()); // ARM
        v.extend_from_slice(&1u32.to_be_bytes());
        v.extend_from_slice(&0x8000u32.to_be_bytes()); // entry
        v.extend_from_slice(&0u32.to_be_bytes());
        v.extend_from_slice(&0u32.to_be_bytes());
        v.extend_from_slice(&0u32.to_be_bytes());
        v.extend_from_slice(&52u16.to_be_bytes());
        v.extend_from_slice(&[0; 10]);
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(elf());
        assert_eq!(ev.node(&d, &[7, 1]).unwrap().value, Value::Enum { raw: 40, name: Some("ARM".into()), hex: false });
        assert_eq!(ev.node(&d, &[7, 3]).unwrap().value, Value::UInt(0x8000));
    }
}
