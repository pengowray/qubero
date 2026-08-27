//! Microsoft COFF relocatable objects: headers, sections, relocations, and
//! symbols before a linker turns them into a PE image.

use crate::template::{Anchor, Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T};

const MACHINES: &[(i128, &str)] = &[
    (0x014c, "i386"),
    (0x0166, "MIPS little-endian"),
    (0x01c0, "ARM"),
    (0x01c4, "ARM Thumb-2"),
    (0x01f0, "PowerPC"),
    (0x0200, "Itanium"),
    (0x5032, "RISC-V 32"),
    (0x5064, "RISC-V 64"),
    (0x5128, "RISC-V 128"),
    (0x8664, "x86-64"),
    (0xaa64, "ARM64"),
];

const SECTION_FLAGS: &[(u32, &str)] = &[
    (5, "code"),
    (6, "initialized data"),
    (7, "uninitialized data"),
    (9, "link info"),
    (11, "remove"),
    (12, "COMDAT"),
    (20, "execute"),
    (21, "read"),
    (22, "write"),
];

const STORAGE: &[(i128, &str)] = &[
    (2, "external"),
    (3, "static"),
    (6, "label"),
    (101, "function"),
    (103, "file"),
    (104, "section"),
    (105, "weak external"),
];

pub fn coff() -> Template {
    Template::new(
        "coff",
        T::structure(
            "COFFObject",
            vec![
                (
                    "machine",
                    T::enumeration_hex("Machine", T::u16(Little), MACHINES),
                ),
                ("section_count", T::u16(Little)),
                ("timestamp", T::u32(Little)),
                ("symbol_table_offset", T::u32(Little)),
                ("symbol_count", T::u32(Little)),
                ("optional_header_size", T::u16(Little)),
                ("characteristics", T::u16(Little)),
                (
                    "optional_header",
                    T::bytes(E::field("optional_header_size")),
                ),
                ("sections", T::array(section(), E::field("section_count"))),
                ("symbols", symbols()),
                ("string_table", strings()),
                (
                    "section_data",
                    T::pointer_list_sized(
                        "sections",
                        &["raw_data_offset"],
                        Anchor::File,
                        E::lit(0),
                        section_data(),
                    )
                    .skipping_zero(),
                ),
            ],
        ),
    )
}

fn section() -> T {
    T::structure_named(
        "Section",
        "name",
        "",
        vec![
            (
                "name",
                T::text(
                    StrLen::Padded {
                        size: E::lit(8),
                        pad: 0,
                    },
                    Encoding::Ascii,
                ),
            ),
            ("virtual_size", T::u32(Little)),
            ("virtual_address", T::u32(Little)),
            ("raw_data_size", T::u32(Little)),
            ("raw_data_offset", T::u32(Little)),
            ("relocation_offset", T::u32(Little)),
            ("line_number_offset", T::u32(Little)),
            ("relocation_count", T::u16(Little)),
            ("line_number_count", T::u16(Little)),
            (
                "characteristics",
                T::flags("SectionCharacteristics", T::u32(Little), SECTION_FLAGS),
            ),
        ],
    )
    .counted_as("section")
}

fn section_data() -> T {
    T::structure(
        "SectionData",
        vec![
            (
                "bytes",
                T::bytes(E::elem_field("sections", E::idx(), &["raw_data_size"])),
            ),
            (
                "relocations",
                T::at(
                    E::elem_field("sections", E::idx(), &["relocation_offset"]),
                    T::array(
                        relocation(),
                        E::elem_field("sections", E::idx(), &["relocation_count"]),
                    ),
                ),
            ),
        ],
    )
}

fn relocation() -> T {
    T::structure(
        "Relocation",
        vec![
            ("virtual_address", T::u32(Little)),
            ("symbol_index", T::u32(Little)),
            ("type", T::u16(Little)),
        ],
    )
    .counted_as("relocation")
}

fn symbols() -> T {
    T::switch(
        E::field("symbol_table_offset"),
        vec![(0, T::bytes(E::lit(0)))],
        T::at(
            E::field("symbol_table_offset"),
            T::array(symbol(), E::field("symbol_count")),
        ),
    )
}

fn symbol() -> T {
    T::structure(
        "Symbol",
        vec![
            ("name", T::bytes(E::lit(8))),
            ("value", T::u32(Little)),
            (
                "section_number",
                T::Int {
                    bits: 16,
                    endian: Little,
                },
            ),
            ("type", T::u16(Little)),
            (
                "storage_class",
                T::enumeration("StorageClass", T::u8(), STORAGE),
            ),
            ("auxiliary_count", T::u8()),
        ],
    )
    .counted_as("symbol")
}

fn strings() -> T {
    let at = E::field("symbol_table_offset").add(E::field("symbol_count").mul(E::lit(18)));
    T::switch(
        E::field("symbol_table_offset"),
        vec![(0, T::bytes(E::lit(0)))],
        T::at(
            at,
            T::structure(
                "StringTable",
                vec![
                    ("size", T::u32(Little)),
                    ("strings", T::bytes(E::field("size").sub(E::lit(4)))),
                ],
            ),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    #[test]
    fn a_section_places_its_bytes_and_relocations() {
        let mut v = Vec::new();
        v.extend_from_slice(&0x014cu16.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&74u32.to_le_bytes());
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(b".text\0\0\0");
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&4u32.to_le_bytes());
        v.extend_from_slice(&60u32.to_le_bytes());
        v.extend_from_slice(&64u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&0x6000_0020u32.to_le_bytes());
        v.extend_from_slice(&[0x90; 4]);
        v.extend_from_slice(&[0; 10]);
        v.extend_from_slice(&[0; 18]);
        v.extend_from_slice(&4u32.to_le_bytes());
        let doc = Document::new(MemSource(v));
        let mut ev = Evaluator::new(coff());
        assert_eq!(
            ev.node(&doc, &[8, 0, 0]).unwrap().value,
            Value::Str(".text".into())
        );
        assert_eq!(ev.node(&doc, &[11, 0, 0]).unwrap().offset_bits, 60 * 8);
        assert_eq!(ev.node(&doc, &[11, 0, 1, 0]).unwrap().offset_bits, 64 * 8);
    }
}
