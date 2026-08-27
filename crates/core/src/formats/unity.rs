//! Unity serialized `.assets` files and UnityFS AssetBundle archives.
//!
//! Serialized files put metadata before an aligned object-data area. Version
//! 22 added the extended 64-bit header; both forms are represented here. The
//! metadata preamble is decoded in the byte order declared by the header, and
//! the version-specific type trees/object table remain bounded as metadata.
//! Those tables require Unity class/type-tree knowledge to interpret safely.

use crate::template::{Endian, Endian::*, Expr as E, Template, Ty as T};

pub fn unity_assets() -> Template {
    let mut cases = Vec::new();
    for version in 9..=21 {
        cases.push((version, legacy_body()));
    }
    cases.push((22, modern_body()));
    cases.push((23, modern_body()));
    Template::new(
        "unityassets",
        T::structure(
            "UnitySerializedFile",
            vec![
                ("legacy_metadata_size", T::u32(Big)),
                ("legacy_file_size", T::u32(Big)),
                ("format_version", T::u32(Big)),
                ("legacy_data_offset", T::u32(Big)),
                (
                    "body",
                    T::switch(E::field("format_version"), cases, T::bytes(E::Remaining)),
                ),
            ],
        ),
    )
}

fn endian_field() -> T {
    T::enumeration(
        "UnityEndianness",
        T::u8(),
        &[(0, "little endian"), (1, "big endian")],
    )
}

fn legacy_body() -> T {
    T::structure(
        "LegacySerializedFileBody",
        vec![
            ("endian", endian_field()),
            ("reserved", T::bytes(E::lit(3))),
            (
                "metadata",
                T::sized(
                    E::field("legacy_metadata_size"),
                    T::switch(
                        E::field("endian"),
                        vec![(1, metadata(Big))],
                        metadata(Little),
                    ),
                ),
            ),
            (
                "object_data",
                T::at(
                    E::field("legacy_data_offset"),
                    T::bytes(E::field("legacy_file_size").sub(E::field("legacy_data_offset"))),
                ),
            ),
        ],
    )
}

fn modern_body() -> T {
    T::structure(
        "ModernSerializedFileBody",
        vec![
            ("endian", endian_field()),
            ("reserved", T::bytes(E::lit(3))),
            ("metadata_size", T::u32(Big)),
            ("file_size", T::u64(Big)),
            ("data_offset", T::u64(Big)),
            ("unknown", T::u64(Big)),
            (
                "metadata",
                T::sized(
                    E::field("metadata_size"),
                    T::switch(
                        E::field("endian"),
                        vec![(1, metadata(Big))],
                        metadata(Little),
                    ),
                ),
            ),
            (
                "object_data",
                T::at(
                    E::field("data_offset"),
                    T::bytes(E::field("file_size").sub(E::field("data_offset"))),
                ),
            ),
        ],
    )
}

fn metadata(endian: Endian) -> T {
    T::structure(
        "SerializedFileMetadata",
        vec![
            ("unity_version", T::cstr()),
            ("target_platform", T::i32(endian)),
            ("type_tree_enabled", T::u8()),
            ("type_count", T::i32(endian)),
            ("type_trees_objects_and_externals", T::bytes(E::Remaining)),
        ],
    )
}

pub fn unity_bundle() -> Template {
    Template::new(
        "unitybundle",
        T::structure(
            "UnityAssetBundle",
            vec![
                ("signature", T::cstr()),
                ("format_version", T::u32(Big)),
                ("unity_version", T::cstr()),
                ("unity_revision", T::cstr()),
                (
                    "unityfs",
                    T::matches(
                        E::field("signature"),
                        vec![("UnityFS", unityfs_body())],
                        T::bytes(E::Remaining),
                    ),
                ),
            ],
        ),
    )
}

fn unityfs_body() -> T {
    T::structure(
        "UnityFsArchive",
        vec![
            ("file_size", T::u64(Big)),
            ("compressed_blocks_info_size", T::u32(Big)),
            ("uncompressed_blocks_info_size", T::u32(Big)),
            (
                "flags",
                T::flags(
                    "UnityFsFlags",
                    T::u32(Big),
                    &[
                        (6, "blocks info at end"),
                        (7, "old web plugin compatibility"),
                        (9, "block info needs padding"),
                    ],
                ),
            ),
            // Compression is encoded in the low six flag bits. Keeping the
            // block-info blob whole preserves LZMA/LZ4 bytes exactly.
            (
                "compressed_blocks_info",
                T::bytes(E::field("compressed_blocks_info_size")),
            ),
            ("data_blocks", T::bytes(E::Remaining)),
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
    fn modern_header_uses_its_64_bit_offsets() {
        let metadata = b"2022.3.1f1\0\x13\0\0\0\0\x02\0\0\0tail";
        let data_at = 80u64;
        let mut v = Vec::new();
        v.extend_from_slice(&0u32.to_be_bytes());
        v.extend_from_slice(&0u32.to_be_bytes());
        v.extend_from_slice(&22u32.to_be_bytes());
        v.extend_from_slice(&0u32.to_be_bytes());
        v.extend_from_slice(&[0, 0, 0, 0]);
        v.extend_from_slice(&(metadata.len() as u32).to_be_bytes());
        v.extend_from_slice(&84u64.to_be_bytes());
        v.extend_from_slice(&data_at.to_be_bytes());
        v.extend_from_slice(&0u64.to_be_bytes());
        v.extend_from_slice(metadata);
        v.resize(data_at as usize, 0);
        v.extend_from_slice(&[1, 2, 3, 4]);
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(unity_assets());
        assert_eq!(ev.node(&d, &[2]).unwrap().value, Value::UInt(22));
        assert_eq!(ev.node(&d, &[4, 7, 0]).unwrap().offset_bits, data_at * 8);
    }

    #[test]
    fn unityfs_header_exposes_archive_sizes() {
        let mut v = b"UnityFS\0".to_vec();
        v.extend_from_slice(&8u32.to_be_bytes());
        v.extend_from_slice(b"2022.3.1f1\0");
        v.extend_from_slice(b"2022.3.1f1\0");
        v.extend_from_slice(&64u64.to_be_bytes());
        v.extend_from_slice(&4u32.to_be_bytes());
        v.extend_from_slice(&8u32.to_be_bytes());
        v.extend_from_slice(&0u32.to_be_bytes());
        v.extend_from_slice(&[1, 2, 3, 4]);
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(unity_bundle());
        assert_eq!(ev.node(&d, &[4, 1]).unwrap().value, Value::UInt(4));
    }
}
