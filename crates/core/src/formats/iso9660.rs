//! ISO 9660 / ECMA-119 optical-disc images.
//!
//! Sixteen 2048-byte sectors form the system area. Volume descriptors follow
//! and repeat through the type-255 terminator. The primary descriptor exposes
//! the paired little/big-endian numbers, path-table locations, root directory
//! record, identifiers, and timestamps that define the filesystem.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};

const DESCRIPTOR_TYPES: &[(i128, &str)] = &[
    (0, "boot"),
    (1, "primary"),
    (2, "supplementary"),
    (3, "partition"),
    (255, "terminator"),
];

pub fn iso9660() -> Template {
    Template::new(
        "iso9660",
        T::structure(
            "ISO9660Image",
            vec![
                ("system_area", T::bytes(E::lit(16 * 2048))),
                (
                    "volume_descriptors",
                    T::repeat(
                        descriptor(),
                        Until::FieldBytes {
                            field: "type".into(),
                            bytes: vec![0xff],
                        },
                    ),
                ),
                ("volume_data", T::bytes(E::Remaining)),
            ],
        ),
    )
}

fn descriptor() -> T {
    T::structure_named(
        "VolumeDescriptor",
        "type",
        "body",
        vec![
            (
                "type",
                T::enumeration("DescriptorType", T::u8(), DESCRIPTOR_TYPES),
            ),
            ("identifier", T::magic(b"CD001")),
            ("version", T::u8()),
            (
                "body",
                T::sized(
                    E::lit(2041),
                    T::switch(
                        E::field("type"),
                        vec![(1, primary())],
                        T::bytes(E::Remaining),
                    ),
                ),
            ),
        ],
    )
    .counted_as("volume descriptor")
}

fn primary() -> T {
    T::structure(
        "PrimaryVolumeDescriptor",
        vec![
            ("unused", T::u8()),
            ("system_identifier", ascii(32)),
            ("volume_identifier", ascii(32)),
            ("unused2", T::bytes(E::lit(8))),
            ("volume_space_size", both32()),
            ("unused3", T::bytes(E::lit(32))),
            ("volume_set_size", both16()),
            ("volume_sequence_number", both16()),
            ("logical_block_size", both16()),
            ("path_table_size", both32()),
            ("type_l_path_table", T::u32(Little)),
            ("optional_type_l_path_table", T::u32(Little)),
            ("type_m_path_table", T::u32(Big)),
            ("optional_type_m_path_table", T::u32(Big)),
            ("root_directory", root_directory()),
            ("volume_set_identifier", ascii(128)),
            ("publisher_identifier", ascii(128)),
            ("data_preparer_identifier", ascii(128)),
            ("application_identifier", ascii(128)),
            ("copyright_file_identifier", ascii(37)),
            ("abstract_file_identifier", ascii(37)),
            ("bibliographic_file_identifier", ascii(37)),
            ("creation_time", ascii(17)),
            ("modification_time", ascii(17)),
            ("expiration_time", ascii(17)),
            ("effective_time", ascii(17)),
            ("file_structure_version", T::u8()),
            ("reserved", T::u8()),
            ("application_use", T::bytes(E::lit(512))),
            ("reserved2", T::bytes(E::lit(653))),
        ],
    )
}

fn both16() -> T {
    T::structure(
        "BothEndian16",
        vec![("little", T::u16(Little)), ("big", T::u16(Big))],
    )
}

fn both32() -> T {
    T::structure(
        "BothEndian32",
        vec![("little", T::u32(Little)), ("big", T::u32(Big))],
    )
}

fn root_directory() -> T {
    T::structure(
        "RootDirectoryRecord",
        vec![
            ("record_length", T::u8()),
            ("extended_attribute_length", T::u8()),
            ("extent_location", both32()),
            ("data_length", both32()),
            ("recording_time", T::bytes(E::lit(7))),
            (
                "flags",
                T::flags(
                    "DirectoryFlags",
                    T::u8(),
                    &[
                        (0, "hidden"),
                        (1, "directory"),
                        (2, "associated"),
                        (7, "multi-extent"),
                    ],
                ),
            ),
            ("file_unit_size", T::u8()),
            ("interleave_gap_size", T::u8()),
            ("volume_sequence_number", both16()),
            ("identifier_length", T::u8()),
            ("identifier", T::bytes(E::field("identifier_length"))),
        ],
    )
}

fn ascii(size: i128) -> T {
    T::text(
        StrLen::Padded {
            size: E::lit(size),
            pad: b' ',
        },
        Encoding::Ascii,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    #[test]
    fn primary_descriptor_starts_at_sector_sixteen() {
        let mut v = vec![0; 16 * 2048];
        let mut pvd = vec![0; 2048];
        pvd[0] = 1;
        pvd[1..6].copy_from_slice(b"CD001");
        pvd[6] = 1;
        pvd[8..40].fill(b' ');
        pvd[40..72].fill(b' ');
        pvd[40..46].copy_from_slice(b"RETRO ");
        pvd[128..130].copy_from_slice(&2048u16.to_le_bytes());
        pvd[130..132].copy_from_slice(&2048u16.to_be_bytes());
        pvd[881] = 1;
        v.extend_from_slice(&pvd);
        let mut end = vec![0; 2048];
        end[0] = 255;
        end[1..6].copy_from_slice(b"CD001");
        end[6] = 1;
        v.extend_from_slice(&end);
        let doc = Document::new(MemSource(v));
        let mut ev = Evaluator::new(iso9660());
        assert_eq!(ev.node(&doc, &[1, 0]).unwrap().offset_bits, 32768 * 8);
        assert_eq!(
            ev.node(&doc, &[1, 0, 3, 2]).unwrap().value,
            Value::Str("RETRO".into())
        );
        assert_eq!(ev.node(&doc, &[1]).unwrap().child_count, 2);
    }
}
