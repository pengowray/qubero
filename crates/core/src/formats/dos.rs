//! MS-DOS executables: the `MZ` format on its own, with no Windows header
//! behind it.
//!
//! The header is fourteen words, a table of relocations somewhere after them,
//! and then the program. What makes it worth a template is that almost every
//! word is a length or an address: the program's size is a page count with the
//! odd bytes of the last page counted separately, the stack and the entry point
//! are segment and offset pairs the loader fixes up, and the relocation table
//! is where it says it is rather than where it happens to fall.
//!
//! The fields are flat in the root struct rather than gathered into a header
//! of their own, because `header_paragraphs` and `pages` have to be in scope
//! where the program after them is sized, and an expression can name a field
//! beside it but never one inside a sibling.
//!
//! Where the program ends is not where the file ends. Anything past the last
//! page is an overlay: a resource fork, an installer's payload, or a second
//! program appended to the first. DOS never loaded those bytes, so they are
//! their own field rather than part of the program.

use crate::code::Isa;
use crate::template::{Endian::*, Expr as E, Template, Ty as T, Until};

/// The fourteen words every `MZ` file starts with, Windows executables
/// included. `pe.rs` builds on these; a DOS program has nothing after them but
/// its relocations.
pub fn header_fields() -> Vec<(&'static str, T)> {
    vec![
        ("magic", T::magic(b"MZ")),
        // The program's length is a page count, with the used part of the last
        // page counted here. Zero means the last page is full, which is the
        // convention every DOS loader followed.
        ("bytes_on_last_page", T::u16(Little)),
        ("pages", T::u16(Little)),
        ("relocation_count", T::u16(Little)),
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
    ]
}

/// Bytes the fourteen words take, which is where the relocation table is
/// measured from.
const HEADER_WORDS: i128 = 28;

/// A paragraph, the unit the header measures itself in.
const PARAGRAPH: i128 = 16;

/// A page, the unit the program measures itself in.
const PAGE: i128 = 512;

pub fn dos() -> Template {
    // Each entry is an address in the program to add the load segment to, as
    // the offset and segment of that address rather than one number.
    let relocation = T::structure("Relocation", vec![("offset", T::u16(Little)), ("segment", T::u16(Little))]);

    let mut fields = header_fields();
    fields.extend(vec![
        // The table is where `relocation_table` says, which on a Microsoft
        // linker's output is two bytes after the words above and on someone
        // else's is not. A file with no relocations points nowhere, and is the
        // one case where that field says nothing at all.
        (
            "relocations",
            T::switch(
                E::field("relocation_count"),
                vec![(0, T::bytes(E::lit(0)))],
                T::structure(
                    "RelocationTable",
                    vec![
                        ("gap", T::bytes(E::field("relocation_table").sub(E::lit(HEADER_WORDS)))),
                        ("entries", T::array(relocation, E::field("relocation_count"))),
                    ],
                ),
            ),
        ),
        // The header is padded to a whole number of paragraphs, and a linker
        // may leave more room than that: what ends it is the count in the
        // header, not the sum of what has been read.
        (
            "header_padding",
            T::bytes(E::field("header_paragraphs").mul(E::lit(PARAGRAPH)).sub(E::lit(HEADER_WORDS)).sub(E::size_of("relocations"))),
        ),
        // The program itself: whole pages, less the header, less the unused
        // tail of the last page. There is no conditional expression, so the
        // full last page is its own case rather than a subtraction that would
        // have to know it is zero.
        (
            "load_module",
            T::switch(
                E::field("bytes_on_last_page"),
                vec![(0, load_module(module_len(E::lit(0))))],
                load_module(module_len(E::lit(PAGE).sub(E::field("bytes_on_last_page")))),
            ),
        ),
        // What DOS never loaded. Usually nothing, and worth a field of its own
        // when it is not: this is where a self-extracting archive keeps its
        // payload and where an overlay-linked program keeps its later parts.
        ("overlay", T::bytes(E::Remaining)),
    ]);

    Template::new("msdos", T::structure("DOSExecutable", fields))
}

/// The program, split where the loader jumps into it.
///
/// There is no table of sections in a DOS executable, so nothing marks which
/// of these bytes are code. What the header does say is where execution
/// starts, and from there to the end is read as instructions. A program that
/// keeps its data after its code has that data read as instructions too;
/// there is nothing in the file that would say otherwise.
///
/// An entry point outside the program is a broken header or a file that is not
/// what it says. Then the whole module is data, which is the reading that
/// claims least.
fn load_module(len: E) -> T {
    let entry = E::field("initial_cs").mul(E::lit(PARAGRAPH)).add(E::field("initial_ip"));
    let inside = entry.clone().less_than(len.clone());
    let start = entry.mul(inside.clone()).add(len.clone().mul(E::lit(1).sub(inside)));
    T::structure(
        "LoadModule",
        vec![
            ("data", T::bytes(start.clone())),
            ("code", T::sized(len.sub(start), T::repeat(T::insn(Isa::X86_16), Until::End))),
        ],
    )
}

/// Bytes of program, given how much of the last page goes unused.
fn module_len(unused: E) -> E {
    E::field("pages")
        .mul(E::lit(PAGE))
        .sub(unused)
        .sub(E::field("header_paragraphs").mul(E::lit(PARAGRAPH)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// An executable with two relocations, a header of two paragraphs' padding
    /// past its table, and `overlay` bytes appended past the last page.
    fn sample(overlay: usize) -> Vec<u8> {
        let header = 0x40usize; // bytes: four paragraphs
        let module = 600usize; // not a whole number of pages
        let mut v = vec![0u8; header + module];
        v[0..2].copy_from_slice(b"MZ");
        let total = header + module;
        let pages = total.div_ceil(512);
        v[0x02..0x04].copy_from_slice(&((total % 512) as u16).to_le_bytes());
        v[0x04..0x06].copy_from_slice(&(pages as u16).to_le_bytes());
        v[0x06..0x08].copy_from_slice(&2u16.to_le_bytes()); // relocation_count
        v[0x08..0x0a].copy_from_slice(&((header / 16) as u16).to_le_bytes());
        v[0x14..0x16].copy_from_slice(&0x30u16.to_le_bytes()); // initial_ip
        v[0x18..0x1a].copy_from_slice(&0x1eu16.to_le_bytes()); // relocation_table
        // Two entries at 0x1e: offset then segment.
        v[0x1e..0x22].copy_from_slice(&[0x11, 0x00, 0x22, 0x00]);
        v[0x22..0x26].copy_from_slice(&[0x33, 0x00, 0x44, 0x00]);
        v.extend(std::iter::repeat_n(0xccu8, overlay));
        v
    }

    fn read(bytes: Vec<u8>, path: &[usize]) -> Value {
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(dos());
        ev.node(&d, path).unwrap().value
    }

    /// How many bytes a field covers, for the ones that hold other fields.
    fn bytes_of(bytes: Vec<u8>, path: &[usize]) -> u64 {
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(dos());
        ev.node(&d, path).unwrap().size_bits / 8
    }

    /// Field positions in the root struct, past the fourteen words.
    const RELOCATIONS: usize = 14;
    const HEADER_PADDING: usize = 15;
    const LOAD_MODULE: usize = 16;
    const OVERLAY: usize = 17;

    fn len_of(v: Value) -> u64 {
        match v {
            Value::Bytes { len, .. } => len,
            other => panic!("not bytes: {other:?}"),
        }
    }

    #[test]
    fn the_relocation_table_is_where_the_header_says() {
        // Entry one, its segment: the second word of the second entry.
        assert_eq!(read(sample(0), &[RELOCATIONS, 1, 1, 1]), Value::UInt(0x44));
    }

    #[test]
    fn the_header_runs_to_the_paragraph_count_and_no_further() {
        // 0x40 bytes of header: 28 words, 2 bytes of gap, 8 of table, 26 left.
        assert_eq!(len_of(read(sample(0), &[HEADER_PADDING])), 0x40 - 28 - 2 - 8);
    }

    #[test]
    fn the_program_is_whole_pages_less_the_header_and_the_unused_tail() {
        assert_eq!(bytes_of(sample(0), &[LOAD_MODULE]), 600);
    }

    /// Nothing in the file marks where the code is; the entry point is what
    /// says where it starts, and everything before it is read as data.
    #[test]
    fn the_code_starts_where_the_loader_jumps() {
        assert_eq!(len_of(read(sample(0), &[LOAD_MODULE, 0])), 0x30);
        let d = Document::new(MemSource(sample(0)));
        let mut ev = Evaluator::new(dos());
        let code = ev.node(&d, &[LOAD_MODULE, 1]).unwrap();
        assert_eq!(code.offset_bits / 8, 0x40 + 0x30);
        assert_eq!(ev.node(&d, &[LOAD_MODULE, 1, 0]).unwrap().type_name, "x86-16");
    }

    /// An entry point past the end of the program is not one, and the module
    /// is then data from end to end.
    #[test]
    fn an_entry_point_outside_the_program_leaves_it_all_data() {
        let mut v = sample(0);
        v[0x16..0x18].copy_from_slice(&0xf000u16.to_le_bytes()); // initial_cs
        assert_eq!(len_of(read(v, &[LOAD_MODULE, 0])), 600);
    }

    #[test]
    fn a_full_last_page_is_counted_as_full_rather_than_as_none() {
        let mut v = sample(0);
        // Say the program fills its last page: two pages, no header padding
        // beyond the four paragraphs already there.
        let total = 2 * 512;
        v.resize(total, 0);
        v[0x02..0x04].copy_from_slice(&0u16.to_le_bytes());
        v[0x04..0x06].copy_from_slice(&2u16.to_le_bytes());
        assert_eq!(bytes_of(v, &[LOAD_MODULE]), 1024 - 0x40);
    }

    #[test]
    fn bytes_past_the_last_page_are_an_overlay_rather_than_program() {
        assert_eq!(len_of(read(sample(4096), &[OVERLAY])), 4096);
        assert_eq!(len_of(read(sample(0), &[OVERLAY])), 0);
    }

    #[test]
    fn a_file_with_no_relocations_reads_no_table() {
        let mut v = sample(0);
        v[0x06..0x08].copy_from_slice(&0u16.to_le_bytes());
        assert_eq!(len_of(read(v.clone(), &[RELOCATIONS])), 0);
        // Which the header padding then covers instead.
        assert_eq!(len_of(read(v, &[HEADER_PADDING])), 0x40 - 28);
    }
}
