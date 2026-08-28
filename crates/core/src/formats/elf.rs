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
//! table as relocations. A section of type `nobits` is the one that has to be
//! handled apart. `.bss` has a size and an offset like any other section and
//! occupies none of the file, so reading its bytes where it points would claim
//! bytes belonging to whatever comes next.
//!
//! [`bpf`] is the same template with one addition: a section that is program
//! bits and marked executable is read as eBPF instructions rather than as
//! bytes. Which is a guess in general and not a guess here, since the file
//! said its machine was BPF before this template was chosen at all.
//!
//! What the template cannot do is name anything. A section's name is an offset
//! into another section, a symbol's name an offset into a third, and the IR's
//! expressions reach siblings and ancestors, not across the file by index.
//! Names are a pass over the parsed tree: see [`super::bpf_disasm`].

use super::bpf_opcodes::{OPCODES, REGS};
use crate::template::{Anchor, Endian, Endian::*, Expr as E, Template, Ty as T, Until};

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

/// Any ELF file. Executable sections stay bytes: what a machine's instructions
/// mean is that machine's business, and this template is not a disassembler
/// for all of them.
pub fn elf() -> Template {
    Template::new("elf", ident(false))
}

/// An ELF object for the BPF machine, whose executable sections are read as
/// instructions.
pub fn bpf() -> Template {
    Template::new("bpf", ident(true))
}

/// The sixteen bytes every ELF opens with, and then the header the class and
/// the data encoding between them select.
fn ident(decode_code: bool) -> T {
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
                        (1, by_endian(32, decode_code)),
                        (2, by_endian(64, decode_code)),
                    ],
                    T::bytes(E::lit(0)),
                ),
            ),
        ],
    )
}

fn by_endian(bits: u32, decode_code: bool) -> T {
    T::switch(
        E::field("data"),
        vec![(1, body(bits, Little, decode_code)), (2, body(bits, Big, decode_code))],
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
fn body(bits: u32, e: Endian, decode_code: bool) -> T {
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
            // Which section holds the section names. A pass over the tree
            // needs this to name anything; the template only records it.
            ("section_name_table", T::u16(e)),
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
                    section_body(bits, e, decode_code),
                )
                .skipping_zero(),
            ),
        ],
    )
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
    T::structure("ProgramHeader", fields).counted_as("segment")
}

fn section_header(bits: u32, e: Endian) -> T {
    T::structure(
        "SectionHeader",
        vec![
            // An offset into the section name table, not a name.
            ("name_offset", T::u32(e)),
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
    .counted_as("section")
}

/// What is in a section, read as its type says. Anything unrecognised is the
/// bytes, which is what a section of program data is.
fn section_body(bits: u32, e: Endian, decode_code: bool) -> T {
    let size = || E::elem_field("section_headers", E::idx(), &["size"]);
    let raw = T::bytes(size());
    // Program bits marked executable are code. Only the BPF template reads
    // them as instructions, and only for exactly `alloc | execute`, which is
    // what a compiler writes for a program section.
    let progbits = if decode_code {
        T::switch(
            E::elem_field("section_headers", E::idx(), &["flags"]),
            vec![(6, T::sized(size(), T::repeat(instruction(e), Until::End)))],
            T::bytes(size()),
        )
    } else {
        raw.clone()
    };
    T::switch(
        E::elem_field("section_headers", E::idx(), &["type"]),
        vec![
            (1, progbits),
            (2, T::array(symbol(bits, e), size().div(E::lit(sym_size(bits))))),
            (3, T::sized(size(), T::repeat(T::cstr(), Until::End))),
            (4, T::array(relocation(bits, e, true), size().div(E::lit(rela_size(bits))))),
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
        let code = ev.node(&d, &[7, 15, 1]).unwrap();
        assert_eq!(code.type_name, "BpfInsn[]");
        assert_eq!(code.offset_bits, 64 * 8);
        assert_eq!(code.child_count, 2);
        // `r1 = 2`: the destination is in the low nibble of the second byte,
        // which is what a little-endian object writes.
        assert_eq!(ev.node(&d, &[7, 15, 1, 0, 2]).unwrap().value, Value::Enum { raw: 1, name: Some("r1".into()), hex: false });
        assert_eq!(ev.node(&d, &[7, 15, 1, 0, 4]).unwrap().value, Value::Int(2));
    }

    #[test]
    fn the_plain_template_leaves_code_as_bytes() {
        let d = Document::new(MemSource(object()));
        let mut ev = Evaluator::new(elf());
        let code = ev.node(&d, &[7, 15, 1]).unwrap();
        assert_eq!(code.type_name, "bytes[]");
        assert_eq!(code.size_bits, 16 * 8);
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
