//! Windows `Thumbs.db`, an OLE Compound File Binary container.
//!
//! The header and its inline DIFAT are fixed. The first directory sector is
//! also opened, giving the Catalog and numbered thumbnail streams their real
//! names and stream sizes. Following arbitrary FAT chains is intentionally
//! left to a future container walker: a template must not pretend sectors are
//! contiguous when the FAT says otherwise.

use crate::template::{Encoding, Endian::Little, Expr as E, StrLen, Template, Ty as T};

const OBJECT_TYPE: &[(i128, &str)] = &[
    (0, "unknown"),
    (1, "storage"),
    (2, "stream"),
    (5, "root storage"),
];
const COLOUR: &[(i128, &str)] = &[(0, "red"), (1, "black")];

pub fn thumbsdb() -> Template {
    Template::new(
        "thumbsdb",
        T::structure(
            "ThumbsDatabase",
            vec![
                ("signature", T::magic(b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1")),
                ("header_clsid", T::bytes(E::lit(16))),
                ("minor_version", T::u16(Little)),
                ("major_version", T::u16(Little)),
                ("byte_order", T::u16(Little)),
                ("sector_shift", T::u16(Little)),
                ("mini_sector_shift", T::u16(Little)),
                ("reserved", T::bytes(E::lit(6))),
                ("directory_sector_count", T::u32(Little)),
                ("fat_sector_count", T::u32(Little)),
                ("first_directory_sector", T::u32(Little)),
                ("transaction_signature", T::u32(Little)),
                ("mini_stream_cutoff", T::u32(Little)),
                ("first_mini_fat_sector", T::u32(Little)),
                ("mini_fat_sector_count", T::u32(Little)),
                ("first_difat_sector", T::u32(Little)),
                ("difat_sector_count", T::u32(Little)),
                ("header_difat", T::array(T::u32(Little), E::lit(109))),
                (
                    "first_directory",
                    T::switch(
                        E::field("sector_shift"),
                        vec![(9, directory_at(512, 4)), (12, directory_at(4096, 32))],
                        T::bytes(E::lit(0)),
                    ),
                ),
                ("sectors", T::bytes(E::Remaining)),
            ],
        ),
    )
}

fn directory_at(sector_size: i128, entries: i128) -> T {
    T::at(
        E::field("first_directory_sector")
            .add(E::lit(1))
            .mul(E::lit(sector_size)),
        T::sized(
            E::lit(sector_size),
            T::array(directory_entry(), E::lit(entries)),
        ),
    )
}

fn directory_entry() -> T {
    // The length at byte 64 of the entry includes the two-byte terminator.
    let name_bytes = E::peek_at(E::lit(64 * 8), 16, Little);
    T::structure_named(
        "CompoundDirectoryEntry",
        "name",
        "",
        vec![
            ("name_size_ahead", T::computed(name_bytes)),
            (
                "name",
                T::text(
                    StrLen::Fixed(E::field("name_size_ahead")),
                    Encoding::Utf16(Little),
                ),
            ),
            (
                "name_padding",
                T::bytes(E::lit(64).sub(E::field("name_size_ahead"))),
            ),
            ("name_size", T::u16(Little)),
            (
                "object_type",
                T::enumeration("DirectoryObjectType", T::u8(), OBJECT_TYPE),
            ),
            ("colour", T::enumeration("RedBlackColour", T::u8(), COLOUR)),
            ("left_sibling", T::u32(Little)),
            ("right_sibling", T::u32(Little)),
            ("child", T::u32(Little)),
            ("clsid", T::bytes(E::lit(16))),
            ("state_bits", T::u32(Little)),
            ("creation_time", T::u64(Little)),
            ("modified_time", T::u64(Little)),
            ("first_sector", T::u32(Little)),
            ("stream_size", T::u64(Little)),
        ],
    )
    .counted_as("directory entry")
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
    fn first_directory_sector_is_placed_and_named() {
        let mut v = vec![0; 1024];
        v[..8].copy_from_slice(b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1");
        v[24..26].copy_from_slice(&0x3eu16.to_le_bytes());
        v[26..28].copy_from_slice(&3u16.to_le_bytes());
        v[28..30].copy_from_slice(&0xfffeu16.to_le_bytes());
        v[30..32].copy_from_slice(&9u16.to_le_bytes());
        v[32..34].copy_from_slice(&6u16.to_le_bytes());
        v[48..52].copy_from_slice(&0u32.to_le_bytes());
        let name: Vec<u8> = "Catalog\0"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        v[512..512 + name.len()].copy_from_slice(&name);
        v[576..578].copy_from_slice(&(name.len() as u16).to_le_bytes());
        v[578] = 2;
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(thumbsdb());
        assert_eq!(ev.node(&d, &[18]).unwrap().offset_bits, 512 * 8);
        assert_eq!(
            ev.node(&d, &[18, 0, 0, 3]).unwrap().value,
            Value::UInt(name.len() as u128)
        );
    }
}
