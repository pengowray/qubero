//! ZIP archives as local entries, directory entries, descriptors, and the end record.

use crate::template::{Encoding, Endian::Little, Expr as E, StrLen, Template, Ty as T, Until};

const SIGS: &[(i128, &str)] = &[
    (0x0403_4b50, "local file"),
    (0x0807_4b50, "data descriptor"),
    (0x0201_4b50, "central directory file"),
    (0x0605_4b50, "end of central directory"),
    (0x0606_4b50, "ZIP64 end"),
    (0x0706_4b50, "ZIP64 locator"),
];
const METHODS: &[(i128, &str)] = &[
    (0, "stored"),
    (1, "shrunk"),
    (6, "imploded"),
    (8, "deflate"),
    (9, "deflate64"),
    (12, "bzip2"),
    (14, "lzma"),
    (93, "zstandard"),
    (95, "xz"),
    (98, "ppmd"),
    (99, "aes"),
];

pub fn zip() -> Template {
    Template::new(
        "zip",
        T::structure(
            "ZIP",
            vec![(
                "records",
                T::repeat(
                    record(),
                    Until::FieldBytes {
                        field: "signature".into(),
                        bytes: b"PK\x05\x06".to_vec(),
                    },
                ),
            )],
        ),
    )
}

fn record() -> T {
    T::structure_named(
        "ZipRecord",
        "signature",
        "body",
        vec![
            (
                "signature",
                T::enumeration_hex("Signature", T::u32(Little), SIGS),
            ),
            (
                "body",
                T::switch(
                    E::field("signature"),
                    vec![
                        (0x0403_4b50, local()),
                        (0x0807_4b50, descriptor()),
                        (0x0201_4b50, central()),
                        (0x0605_4b50, end()),
                        (0x0606_4b50, zip64_end()),
                        (0x0706_4b50, zip64_locator()),
                    ],
                    T::bytes(E::lit(0)),
                ),
            ),
        ],
    )
    .counted_as("record")
}

fn flags() -> T {
    T::flags(
        "GeneralPurposeFlags",
        T::u16(Little),
        &[
            (0, "encrypted"),
            (3, "data descriptor follows"),
            (6, "strong encryption"),
            (11, "UTF-8 names"),
            (13, "directory encryption"),
        ],
    )
}
fn text(len: &str) -> T {
    T::text(StrLen::Fixed(E::field(len)), Encoding::Unknown)
}

fn local() -> T {
    T::structure(
        "LocalFile",
        vec![
            ("version_needed", T::u16(Little)),
            ("flags", flags()),
            (
                "compression",
                T::enumeration("CompressionMethod", T::u16(Little), METHODS),
            ),
            ("modified_time", T::u16(Little)),
            ("modified_date", T::u16(Little)),
            ("crc32", T::u32(Little)),
            ("compressed_size", T::u32(Little)),
            ("uncompressed_size", T::u32(Little)),
            ("name_length", T::u16(Little)),
            ("extra_length", T::u16(Little)),
            ("name", text("name_length")),
            ("extra", T::bytes(E::field("extra_length"))),
            ("data", T::bytes(E::field("compressed_size"))),
        ],
    )
}

fn central() -> T {
    T::structure(
        "CentralDirectoryFile",
        vec![
            ("version_made_by", T::u16(Little)),
            ("version_needed", T::u16(Little)),
            ("flags", flags()),
            (
                "compression",
                T::enumeration("CompressionMethod", T::u16(Little), METHODS),
            ),
            ("modified_time", T::u16(Little)),
            ("modified_date", T::u16(Little)),
            ("crc32", T::u32(Little)),
            ("compressed_size", T::u32(Little)),
            ("uncompressed_size", T::u32(Little)),
            ("name_length", T::u16(Little)),
            ("extra_length", T::u16(Little)),
            ("comment_length", T::u16(Little)),
            ("disk_number", T::u16(Little)),
            ("internal_attributes", T::u16(Little)),
            ("external_attributes", T::u32(Little)),
            ("local_header_offset", T::u32(Little)),
            ("name", text("name_length")),
            ("extra", T::bytes(E::field("extra_length"))),
            ("comment", text("comment_length")),
        ],
    )
}

fn descriptor() -> T {
    T::structure(
        "DataDescriptor",
        vec![
            ("crc32", T::u32(Little)),
            ("compressed_size", T::u32(Little)),
            ("uncompressed_size", T::u32(Little)),
        ],
    )
}
fn end() -> T {
    T::structure(
        "EndOfCentralDirectory",
        vec![
            ("disk_number", T::u16(Little)),
            ("directory_disk", T::u16(Little)),
            ("entries_on_disk", T::u16(Little)),
            ("entries_total", T::u16(Little)),
            ("directory_size", T::u32(Little)),
            ("directory_offset", T::u32(Little)),
            ("comment_length", T::u16(Little)),
            ("comment", text("comment_length")),
        ],
    )
}
fn zip64_end() -> T {
    T::structure(
        "Zip64End",
        vec![
            ("record_size", T::u64(Little)),
            ("record", T::bytes(E::field("record_size"))),
        ],
    )
}
fn zip64_locator() -> T {
    T::structure(
        "Zip64Locator",
        vec![
            ("directory_disk", T::u32(Little)),
            ("directory_offset", T::u64(Little)),
            ("disk_count", T::u32(Little)),
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
    #[test]
    fn local_entry_and_end_record() {
        let mut v = b"PK\x03\x04".to_vec();
        v.extend_from_slice(&20u16.to_le_bytes());
        v.extend_from_slice(&[0; 12]);
        v.extend_from_slice(&3u32.to_le_bytes());
        v.extend_from_slice(&3u32.to_le_bytes());
        v.extend_from_slice(&5u16.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(b"a.txtabc");
        v.extend_from_slice(b"PK\x05\x06");
        v.extend_from_slice(&[0; 18]);
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(zip());
        assert_eq!(e.node(&d, &[0]).unwrap().child_count, 2);
        assert_eq!(
            e.node(&d, &[0, 0, 1, 10]).unwrap().value,
            Value::Str("a.txt".into())
        );
        assert_eq!(e.node(&d, &[0, 0, 1, 12]).unwrap().size_bits, 24);
    }
}
