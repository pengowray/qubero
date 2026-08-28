//! Mach-O: the programs, libraries and objects macOS and iOS run.
//!
//! A file is a header, a run of load commands, and whatever those commands
//! point at. Everything in it is a command: which segments to map, where the
//! symbols are, which libraries to load, what version of what system it was
//! built for. A command whose number this does not know is still read as far
//! as its own length, so an unknown command costs a row rather than the file.
//!
//! One file can hold several programs. A universal binary opens with a table
//! of architectures and a whole Mach-O for each, which is why the template
//! refers to itself: the slices are read with the same type as the file would
//! be if it held only one.
//!
//! The magic says both how wide the addresses are and which way round the
//! numbers go, and there are four of them for the four combinations. Only the
//! little-endian pair turn up now; the others are how a PowerPC program was
//! written, and reading them costs two more cases.

use crate::code::Isa;
use crate::template::{Anchor, Encoding, Endian, Endian::*, Expr as E, Part, StrLen, Template, Ty as T, Until};

/// What a slice of a universal binary, and a whole file, is for. The top byte
/// says whether the addresses are 64-bit, so the same machine appears twice.
const CPU_TYPE: &[(i128, &str)] = &[
    (1, "VAX"),
    (6, "68000"),
    (7, "x86"),
    (0x0100_0007, "x86-64"),
    (10, "88000"),
    (11, "SPARC"),
    (12, "ARM"),
    (0x0100_000c, "ARM64"),
    (0x0200_000c, "ARM64 32-bit"),
    (14, "SPARC"),
    (18, "PowerPC"),
    (0x0100_0012, "PowerPC 64"),
];

const FILE_TYPE: &[(i128, &str)] = &[
    (1, "object"),
    (2, "executable"),
    (3, "fixed VM library"),
    (4, "core dump"),
    (5, "preloaded executable"),
    (6, "dynamic library"),
    (7, "dynamic linker"),
    (8, "bundle"),
    (9, "dynamic library stub"),
    (10, "debug symbols"),
    (11, "kernel extension"),
    (12, "file set"),
    (13, "GPU program"),
    (14, "GPU dynamic library"),
];

/// The load commands worth naming. The high bit says the loader must
/// understand the command or refuse the file, and it is part of the number.
const COMMAND: &[(i128, &str)] = &[
    (0x1, "segment"),
    (0x2, "symtab"),
    (0x3, "symseg"),
    (0x4, "thread"),
    (0x5, "unix thread"),
    (0x6, "load fixed VM library"),
    (0x7, "fixed VM library id"),
    (0x8, "ident"),
    (0x9, "fixed VM file"),
    (0xa, "prepage"),
    (0xb, "dynamic symtab"),
    (0xc, "load dylib"),
    (0xd, "dylib id"),
    (0xe, "load dynamic linker"),
    (0xf, "dynamic linker id"),
    (0x10, "prebound dylib"),
    (0x11, "routines"),
    (0x12, "sub framework"),
    (0x13, "sub umbrella"),
    (0x14, "sub client"),
    (0x15, "sub library"),
    (0x16, "two-level hints"),
    (0x17, "prebind checksum"),
    (0x8000_0018, "load weak dylib"),
    (0x19, "segment 64"),
    (0x1a, "routines 64"),
    (0x1b, "uuid"),
    (0x8000_001c, "run path"),
    (0x1d, "code signature"),
    (0x1e, "segment split info"),
    (0x8000_001f, "reexport dylib"),
    (0x20, "lazy load dylib"),
    (0x21, "encryption info"),
    (0x22, "dyld info"),
    (0x8000_0022, "dyld info only"),
    (0x8000_0023, "load upward dylib"),
    (0x24, "minimum macOS version"),
    (0x25, "minimum iOS version"),
    (0x26, "function starts"),
    (0x27, "dyld environment"),
    (0x8000_0028, "main"),
    (0x29, "data in code"),
    (0x2a, "source version"),
    (0x2b, "dylib code signing DRs"),
    (0x2c, "encryption info 64"),
    (0x2d, "linker option"),
    (0x2e, "linker optimisation hint"),
    (0x2f, "minimum tvOS version"),
    (0x30, "minimum watchOS version"),
    (0x31, "note"),
    (0x32, "build version"),
    (0x8000_0033, "dyld exports trie"),
    (0x8000_0034, "dyld chained fixups"),
    (0x8000_0035, "file set entry"),
];

const PROTECTION: &[(u32, &str)] = &[(0, "read"), (1, "write"), (2, "execute")];

/// A section's flags: the low byte is what kind of section it is, and the top
/// bits are what is true of it. The one that matters here is the bit that says
/// the section is instructions and nothing else.
const SECTION_FLAGS: &[(u32, &str)] = &[
    (8, "has local relocations"),
    (9, "has external relocations"),
    (10, "some instructions"),
    (25, "debug"),
    (26, "self modifying code"),
    (27, "live support"),
    (28, "no dead strip"),
    (29, "strip static symbols"),
    (30, "no toc"),
    (31, "pure instructions"),
];

const PLATFORM: &[(i128, &str)] = &[
    (1, "macOS"),
    (2, "iOS"),
    (3, "tvOS"),
    (4, "watchOS"),
    (5, "bridgeOS"),
    (6, "Mac Catalyst"),
    (7, "iOS simulator"),
    (8, "tvOS simulator"),
    (9, "watchOS simulator"),
    (10, "driver kit"),
    (11, "visionOS"),
    (12, "visionOS simulator"),
];

/// The machines whose instructions this can read. A slice for anything else
/// keeps its code as bytes.
fn decoded() -> Vec<(i128, Isa)> {
    vec![
        (7, Isa::X86_32),
        (0x0100_0007, Isa::X86_64),
        (12, Isa::Arm),
        (0x0100_000c, Isa::Aarch64),
        // The 32-bit ARM of an old iPhone, whose four-byte encoding this
        // reads; the two-byte one it mixes with is work for later.
        (0x0200_000c, Isa::Aarch64),
    ]
}

pub fn macho() -> Template {
    Template::new("macho", T::Named("MachO".into())).with_part(&part())
}

/// The type that reads one Mach-O, and everything it refers to by name. A
/// universal binary holds several of these, so the type has to be reachable by
/// name rather than written out where it is used.
fn part() -> Part {
    Part::new(T::Named("MachO".into())).with_type(
        "MachO",
        T::switch(
            E::peek(32, Big),
            vec![
                // The magic read as four bytes in order, so the number here
                // says both which way round the file writes numbers and how
                // wide its addresses are: a little-endian file writes the
                // magic backwards, which is what makes it readable at all.
                (0xfeed_face, file(32, Big)),
                (0xfeed_facf, file(64, Big)),
                (0xcefa_edfe, file(32, Little)),
                (0xcffa_edfe, file(64, Little)),
                (0xcafe_babe, fat(32)),
                (0xcafe_babf, fat(64)),
            ],
            T::bytes(E::Remaining),
        ),
    )
}

/// A universal binary: a table of what is inside, and the files themselves at
/// the offsets it gives. Its own numbers are big-endian whatever the slices
/// are, being older than the machines that made that a question.
fn fat(bits: u32) -> T {
    let word = |b: u32| if b == 64 { T::u64(Big) } else { T::u32(Big) };
    let arch = T::structure(
        "Architecture",
        vec![
            ("cpu_type", T::enumeration_hex("CpuType", T::u32(Big), CPU_TYPE)),
            ("cpu_subtype", T::u32(Big)),
            ("offset", word(bits)),
            ("size", word(bits)),
            ("align", T::u32(Big)),
        ],
    )
    .counted_as("architecture");
    T::structure(
        "Universal",
        vec![
            ("magic", T::u32(Big)),
            ("count", T::u32(Big)),
            ("architectures", T::array(arch, E::field("count"))),
            (
                "slices",
                // Each file inside is read in a window of its own, because a
                // section's offset is counted from the start of its own file
                // rather than from the start of the one holding it.
                T::pointer_list_sized(
                    "architectures",
                    &["offset"],
                    Anchor::File,
                    E::lit(0),
                    T::sized(E::elem_field("architectures", E::idx(), &["size"]), T::Named("MachO".into())),
                )
                .skipping_zero(),
            ),
        ],
    )
}

/// One Mach-O: the header, the commands, and what the commands describe.
fn file(bits: u32, e: Endian) -> T {
    let mut fields: Vec<(&str, T)> = vec![
        ("magic", T::u32(e)),
        ("cpu_type", T::enumeration_hex("CpuType", T::u32(e), CPU_TYPE)),
        ("cpu_subtype", T::u32(e)),
        ("file_type", T::enumeration("FileType", T::u32(e), FILE_TYPE)),
        ("command_count", T::u32(e)),
        ("command_bytes", T::u32(e)),
        ("flags", T::u32(e)),
    ];
    if bits == 64 {
        fields.push(("reserved", T::u32(e)));
    }
    fields.push(("commands", T::array(command(e), E::field("command_count"))));
    T::structure("MachOFile", fields)
}

/// One load command, read as far as its own length says whatever it turns out
/// to be. The length is what makes an unknown command harmless: it says where
/// the next one starts without anything having to understand this one.
fn command(e: Endian) -> T {
    let linkedit = T::structure("LinkEditData", vec![("offset", T::u32(e)), ("size", T::u32(e))]);
    let version = T::structure(
        "VersionMinimum",
        vec![("version", T::u32(e)), ("sdk_version", T::u32(e))],
    );
    let dylib = T::structure(
        "Dylib",
        vec![
            // Where the name is, counted from the start of the command.
            ("name_offset", T::u32(e)),
            ("timestamp", T::u32(e)),
            ("current_version", T::u32(e)),
            ("compatibility_version", T::u32(e)),
            // The name fills the rest of the command, padded with zeros to
            // whatever the linker rounded the command's length to.
            ("name", T::text(StrLen::Padded { size: E::field("size").sub(E::lit(24)), pad: 0 }, Encoding::Utf8)),
        ],
    );
    let path = T::structure(
        "Path",
        vec![
            ("name_offset", T::u32(e)),
            ("name", T::text(StrLen::Padded { size: E::field("size").sub(E::lit(12)), pad: 0 }, Encoding::Utf8)),
        ],
    );
    let cases: Vec<(i128, T)> = vec![
        (0x1, segment(32, e)),
        (0x19, segment(64, e)),
        (
            0x2,
            T::structure(
                "SymbolTable",
                vec![
                    ("symbol_offset", T::u32(e)),
                    ("symbol_count", T::u32(e)),
                    ("string_offset", T::u32(e)),
                    ("string_size", T::u32(e)),
                ],
            ),
        ),
        (0xc, dylib.clone()),
        (0xd, dylib.clone()),
        (0x8000_0018, dylib.clone()),
        (0x8000_001f, dylib),
        (0xe, path.clone()),
        (0xf, path.clone()),
        (0x8000_001c, path),
        (0x1b, T::structure("Uuid", vec![("uuid", T::bytes(E::lit(16)))])),
        (
            0x8000_0028,
            T::structure("Main", vec![("entry_offset", T::u64(e)), ("stack_size", T::u64(e))]),
        ),
        (0x2a, T::structure("SourceVersion", vec![("version", T::u64(e))])),
        (0x24, version.clone()),
        (0x25, version.clone()),
        (0x2f, version.clone()),
        (0x30, version),
        (
            0x32,
            T::structure(
                "BuildVersion",
                vec![
                    ("platform", T::enumeration("Platform", T::u32(e), PLATFORM)),
                    ("minimum_os", T::u32(e)),
                    ("sdk", T::u32(e)),
                    ("tool_count", T::u32(e)),
                    (
                        "tools",
                        T::array(
                            T::structure("Tool", vec![("tool", T::u32(e)), ("version", T::u32(e))]),
                            E::field("tool_count"),
                        ),
                    ),
                ],
            ),
        ),
        (0x1d, linkedit.clone()),
        (0x1e, linkedit.clone()),
        (0x26, linkedit.clone()),
        (0x29, linkedit.clone()),
        (0x2b, linkedit.clone()),
        (0x2e, linkedit.clone()),
        (0x8000_0033, linkedit.clone()),
        (0x8000_0034, linkedit),
    ];
    T::structure(
        "LoadCommand",
        vec![
            ("command", T::enumeration_hex("Command", T::u32(e), COMMAND)),
            ("size", T::u32(e)),
            // Not a window: what a section says about where its bytes are is
            // counted from the start of the file it is in, and a window here
            // would put every section inside the command that named it.
            ("body", T::switch(E::field("command"), cases, T::bytes(E::field("size").sub(E::lit(8))))),
            // Whatever the command has room for and did not use. A linker
            // rounds a command up, and the bytes it rounded with belong to it.
            ("padding", T::bytes(E::field("size").sub(E::lit(8)).sub(E::size_of("body")))),
        ],
    )
    .counted_as("command")
}

/// A segment: what to map, where, and the sections inside it. A file's code
/// and data are all sections of some segment, and where a section is in the
/// file is its own business rather than the segment's.
fn segment(bits: u32, e: Endian) -> T {
    let word = || if bits == 64 { T::u64(e) } else { T::u32(e) };
    T::structure_named(
        "Segment",
        "name",
        "",
        vec![
            ("name", T::text(StrLen::Padded { size: E::lit(16), pad: 0 }, Encoding::Ascii)),
            ("address", word()),
            ("memory_size", word()),
            ("file_offset", word()),
            ("file_size", word()),
            ("maximum_protection", T::flags("Protection", T::u32(e), PROTECTION)),
            ("initial_protection", T::flags("Protection", T::u32(e), PROTECTION)),
            ("section_count", T::u32(e)),
            ("flags", T::u32(e)),
            ("sections", T::array(section(bits, e), E::field("section_count"))),
        ],
    )
}

/// One section, and its bytes where it says they are. A section marked as
/// nothing but instructions is read as instructions, for the machines whose
/// instructions this knows; a section with an offset of zero has no bytes in
/// the file at all, which is what `__bss` is.
fn section(bits: u32, e: Endian) -> T {
    let word = || if bits == 64 { T::u64(e) } else { T::u32(e) };
    let mut machines: Vec<(i128, T)> = Vec::new();
    for (machine, isa) in decoded() {
        machines.push((machine, T::sized(E::field("size"), T::repeat(T::insn(isa), Until::End))));
    }
    let contents = T::switch(
        E::field("flags").bit(31),
        vec![(1, T::switch(E::field("cpu_type"), machines, T::bytes(E::field("size"))))],
        T::bytes(E::field("size")),
    );
    let mut fields: Vec<(&str, T)> = vec![
        ("name", T::text(StrLen::Padded { size: E::lit(16), pad: 0 }, Encoding::Ascii)),
        ("segment_name", T::text(StrLen::Padded { size: E::lit(16), pad: 0 }, Encoding::Ascii)),
        ("address", word()),
        ("size", word()),
        ("offset", T::u32(e)),
        ("align", T::u32(e)),
        ("relocation_offset", T::u32(e)),
        ("relocation_count", T::u32(e)),
        ("flags", T::flags("SectionFlags", T::u32(e), SECTION_FLAGS)),
        ("reserved1", T::u32(e)),
        ("reserved2", T::u32(e)),
    ];
    if bits == 64 {
        fields.push(("reserved3", T::u32(e)));
    }
    // Counted from the start of this file, which in a universal binary is not
    // the start of the one on disk.
    fields.push((
        "contents",
        T::switch(E::field("offset"), vec![(0, T::bytes(E::lit(0)))], T::at_in_window(E::field("offset"), contents)),
    ));
    T::structure_named("Section", "name", "", fields).counted_as("section")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// A 64-bit file for x86-64 with one segment, one section of code, and a
    /// command this template does not know, to show that one costs a row and
    /// not the file.
    fn sample() -> Vec<u8> {
        let text: [u8; 6] = [0xb8, 0x01, 0x00, 0x00, 0x00, 0xc3]; // mov eax, 1; ret
        let header = 32u32;
        let segment_size = 72 + 80; // the command, and one section
        let unknown_size = 16u32;
        let text_at = header + segment_size as u32 + unknown_size;

        let mut v = Vec::new();
        v.extend_from_slice(&0xfeed_facfu32.to_le_bytes());
        v.extend_from_slice(&0x0100_0007u32.to_le_bytes()); // x86-64
        v.extend_from_slice(&3u32.to_le_bytes());
        v.extend_from_slice(&2u32.to_le_bytes()); // an executable
        v.extend_from_slice(&2u32.to_le_bytes()); // two commands
        v.extend_from_slice(&(segment_size as u32 + unknown_size).to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());

        v.extend_from_slice(&0x19u32.to_le_bytes()); // segment 64
        v.extend_from_slice(&(segment_size as u32).to_le_bytes());
        v.extend_from_slice(b"__TEXT\0\0\0\0\0\0\0\0\0\0");
        v.extend_from_slice(&0u64.to_le_bytes()); // address
        v.extend_from_slice(&0x1000u64.to_le_bytes()); // memory size
        v.extend_from_slice(&0u64.to_le_bytes()); // file offset
        v.extend_from_slice(&0x1000u64.to_le_bytes()); // file size
        v.extend_from_slice(&5u32.to_le_bytes()); // read and execute
        v.extend_from_slice(&5u32.to_le_bytes());
        v.extend_from_slice(&1u32.to_le_bytes()); // one section
        v.extend_from_slice(&0u32.to_le_bytes());

        v.extend_from_slice(b"__text\0\0\0\0\0\0\0\0\0\0");
        v.extend_from_slice(b"__TEXT\0\0\0\0\0\0\0\0\0\0");
        v.extend_from_slice(&(text_at as u64).to_le_bytes()); // address
        v.extend_from_slice(&(text.len() as u64).to_le_bytes());
        v.extend_from_slice(&text_at.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&0x8000_0400u32.to_le_bytes()); // pure instructions
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());

        // A command nothing here knows, which is still as long as it says.
        v.extend_from_slice(&0x4242u32.to_le_bytes());
        v.extend_from_slice(&unknown_size.to_le_bytes());
        v.extend_from_slice(&[0; 8]);

        assert_eq!(v.len(), text_at as usize);
        v.extend_from_slice(&text);
        v
    }

    #[test]
    fn a_section_of_code_reads_as_instructions() {
        let d = Document::new(MemSource(sample()));
        let mut ev = Evaluator::new(macho());
        // The header's commands, the segment, its sections, the first one.
        let contents = ev.node(&d, &[8, 0, 2, 9, 0, 12, 0]).unwrap();
        assert_eq!(contents.type_name, "x86-64[]");
        assert_eq!(ev.node(&d, &[8, 0, 2, 9, 0, 12, 0, 0]).unwrap().value, Value::Str("mov eax, 0x1".into()));
        assert_eq!(ev.node(&d, &[8, 0, 2, 9, 0, 12, 0, 1]).unwrap().value, Value::Str("ret".into()));
    }

    /// Two files in one, which is how a program ships for both machines a Mac
    /// has had. The table is big-endian whatever the files inside it are.
    #[test]
    fn a_universal_binary_reads_the_files_inside_it() {
        let inner = sample();
        let at = 0x1000usize;
        let mut v = Vec::new();
        v.extend_from_slice(&0xcafe_babeu32.to_be_bytes());
        v.extend_from_slice(&1u32.to_be_bytes());
        v.extend_from_slice(&0x0100_0007u32.to_be_bytes()); // x86-64
        v.extend_from_slice(&3u32.to_be_bytes());
        v.extend_from_slice(&(at as u32).to_be_bytes());
        v.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        v.extend_from_slice(&12u32.to_be_bytes());
        v.resize(at, 0);
        v.extend_from_slice(&inner);

        assert_eq!(crate::formats::sniff(&v, v.len() as u64), Some("macho"));
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(macho());
        // The slices, the one file, its commands, the segment, its sections.
        let slice = ev.node(&d, &[3, 0]).unwrap();
        assert_eq!(slice.type_name, "MachOFile");
        assert_eq!(slice.offset_bits, at as u64 * 8);
        let first = ev.node(&d, &[3, 0, 8, 0, 2, 9, 0, 12, 0, 0]).unwrap();
        assert_eq!(first.value, Value::Str("mov eax, 0x1".into()));
    }

    #[test]
    fn a_command_nothing_knows_is_as_long_as_it_says() {
        let d = Document::new(MemSource(sample()));
        let mut ev = Evaluator::new(macho());
        let unknown = ev.node(&d, &[8, 1]).unwrap();
        assert_eq!(unknown.size_bits, 16 * 8);
    }
}
