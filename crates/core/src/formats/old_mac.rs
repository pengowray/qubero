//! Containers used to move classic Macintosh files off an HFS disk.
//!
//! These layouts follow the corresponding readers in pappadf/peeler. They
//! expose both forks and the Finder metadata rather than treating an archive
//! as an undifferentiated compressed run. Compression streams remain bytes:
//! like gzip elsewhere in the template set, their on-disk structure is the
//! format shown by the editor.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};

const FINDER_FLAGS: &[(u32, &str)] = &[
    (0, "is on desk"),
    (6, "shared"),
    (7, "has no INITs"),
    (8, "has been inited"),
    (10, "has custom icon"),
    (11, "stationery"),
    (12, "name locked"),
    (13, "has bundle"),
    (14, "invisible"),
    (15, "alias"),
];

fn text(n: E) -> T {
    // Classic names are MacRoman. Unknown is preferable to falsely claiming
    // ASCII and preserves every byte while the core has no MacRoman codec.
    T::text(StrLen::Fixed(n), Encoding::Unknown)
}

fn code() -> T {
    T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii)
}

fn padding(n: E, block: i128) -> T {
    let rounded = n.clone().add(E::lit(block - 1)).div(E::lit(block)).mul(E::lit(block));
    T::bytes(rounded.sub(n))
}

/// MacBinary I, II and III. The two Finder-flag bytes are non-contiguous, so
/// each is labelled in its own byte with the global bit's meaning.
pub fn macbinary() -> Template {
    let high = &[(0, "has been inited"), (2, "has custom icon"), (3, "stationery"), (4, "name locked"), (5, "has bundle"), (6, "invisible"), (7, "alias")];
    let low = &[(0, "is on desk"), (6, "shared"), (7, "has no INITs")];
    Template::new(
        "macbinary",
        T::structure(
            "MacBinary",
            vec![
                ("old_version", T::u8()),
                ("name_length", T::u8()),
                ("name", text(E::field("name_length"))),
                ("name_padding", T::bytes(E::lit(63).sub(E::field("name_length")))),
                ("file_type", code()),
                ("creator", code()),
                ("finder_flags_high", T::flags("FinderFlagsHigh", T::u8(), high)),
                ("zero_74", T::u8()),
                ("icon_v", T::u16(Big)),
                ("icon_h", T::u16(Big)),
                ("folder", T::u16(Big)),
                ("protected", T::flags("Protected", T::u8(), &[(0, "protected")])),
                ("zero_82", T::u8()),
                ("data_length", T::u32(Big)),
                ("resource_length", T::u32(Big)),
                ("created", T::u32(Big)),
                ("modified", T::u32(Big)),
                ("comment_length", T::u16(Big)),
                ("finder_flags_low", T::flags("FinderFlagsLow", T::u8(), low)),
                ("reserved", T::bytes(E::lit(14))),
                ("unpacked_length", T::u32(Big)),
                ("secondary_length", T::u16(Big)),
                ("writer_version", T::enumeration("WriterVersion", T::u8(), &[(0, "MacBinary I"), (129, "MacBinary II"), (130, "MacBinary III")])),
                ("minimum_version", T::u8()),
                ("header_crc", T::u16(Big)),
                ("header_tail", T::bytes(E::lit(2))),
                ("secondary", T::bytes(E::field("secondary_length"))),
                ("secondary_padding", padding(E::field("secondary_length"), 128)),
                ("data_fork", T::bytes(E::field("data_length"))),
                ("data_padding", padding(E::field("data_length"), 128)),
                ("resource_fork", T::bytes(E::field("resource_length"))),
                ("resource_padding", padding(E::field("resource_length"), 128)),
                ("finder_comment", text(E::field("comment_length"))),
                ("comment_padding", padding(E::field("comment_length"), 128)),
            ],
        ),
    )
}

const SIT_METHOD: &[(i128, &str)] = &[
    (0, "stored"),
    (1, "RLE"),
    (2, "LZW"),
    (3, "Huffman"),
    (5, "LZAH"),
    (6, "fixed Huffman"),
    (8, "MW"),
    (13, "LZSS/Huffman"),
    (14, "LZSS arithmetic"),
    (15, "Arsenic"),
    (0x20, "folder start"),
    (0x21, "folder end"),
];

/// Classic StuffIt 1.x-4.x archives. Entries run to the physical end because
/// the header count counts top-level folders, not serialized marker records.
pub fn stuffit() -> Template {
    Template::new("stuffit", T::switch(E::peek(32, Big), vec![(0x5374_7566, stuffit5_body())], classic_stuffit_body()))
}

fn classic_stuffit_body() -> T {
    T::structure(
        "StuffIt",
        vec![
            ("signature", code()),
            ("top_level_count", T::u16(Big)),
            ("archive_size", T::u32(Big)),
            ("magic", T::magic(b"rLau")),
            ("version", T::u8()),
            ("reserved", T::bytes(E::lit(7))),
            ("entries", T::repeat(sit_entry(), Until::End)),
        ],
    )
}

fn sit_entry() -> T {
    T::structure_named(
        "Entry",
        "name",
        "",
        vec![
            ("resource_method", T::enumeration_hex("Method", T::u8(), SIT_METHOD)),
            ("data_method", T::enumeration_hex("Method", T::u8(), SIT_METHOD)),
            ("name_length", T::u8()),
            ("name", text(E::field("name_length"))),
            ("name_padding", T::bytes(E::lit(63).sub(E::field("name_length")))),
            ("file_type", code()),
            ("creator", code()),
            ("finder_flags", T::flags("FinderFlags", T::u16(Big), FINDER_FLAGS)),
            ("created", T::u32(Big)),
            ("modified", T::u32(Big)),
            ("resource_original_length", T::u32(Big)),
            ("data_original_length", T::u32(Big)),
            ("resource_compressed_length", T::u32(Big)),
            ("data_compressed_length", T::u32(Big)),
            ("resource_crc", T::u16(Big)),
            ("data_crc", T::u16(Big)),
            ("reserved", T::bytes(E::lit(6))),
            ("header_crc", T::u16(Big)),
            ("resource_fork", T::bytes(E::field("resource_compressed_length"))),
            ("data_fork", T::bytes(E::field("data_compressed_length"))),
        ],
    )
    .counted_as("entry")
}

/// StuffIt 5's linked entry layout. Archives normally serialize the linked
/// entries in next-offset order; exposing the pointers as fields also makes a
/// non-contiguous or damaged chain apparent to the reader.
fn stuffit5_body() -> T {
    T::structure(
        "StuffIt5",
        vec![
            ("banner", T::text(StrLen::Fixed(E::lit(80)), Encoding::Ascii)),
            ("unknown", T::u32(Big)),
            ("archive_size", T::u32(Big)),
            ("first_entry_offset", T::u32(Big)),
            ("declared_entry_count", T::u16(Big)),
            ("initial_cursor", T::u32(Big)),
            ("reserved", T::bytes(E::field("initial_cursor").sub(E::lit(98)))),
            ("entries", T::repeat(sit5_entry(), Until::End)),
        ],
    )
}

fn sit5_entry() -> T {
    T::structure_named(
        "Entry5",
        "name",
        "",
        vec![
            ("magic", T::magic(b"\xa5\xa5\xa5\xa5")),
            ("version", T::u8()),
            ("unknown_1", T::u8()),
            ("header_size", T::u16(Big)),
            ("unknown_2", T::u8()),
            ("flags", T::flags("EntryFlags", T::u8(), &[(5, "encrypted"), (6, "folder")])),
            ("created", T::u32(Big)),
            ("modified", T::u32(Big)),
            ("previous_offset", T::u32(Big)),
            ("next_offset", T::u32(Big)),
            ("parent_offset", T::u32(Big)),
            ("name_length", T::u16(Big)),
            ("header_crc", T::u16(Big)),
            ("data_original_length", T::u32(Big)),
            ("data_compressed_length", T::u32(Big)),
            ("data_crc", T::u16(Big)),
            ("unknown_3", T::u16(Big)),
            ("data_method_or_child_count_high", T::enumeration_hex("Method", T::u8(), SIT_METHOD)),
            ("password_length_or_child_count_low", T::u8()),
            ("name", text(E::field("name_length"))),
            ("header_tail", T::bytes(E::field("header_size").sub(E::lit(48)).sub(E::field("name_length")))),
            ("body", T::switch(E::field("data_original_length"), vec![(0xffff_ffff, T::bytes(E::lit(0)))], sit5_second_header())),
        ],
    )
    .counted_as("entry")
}

fn sit5_second_header() -> T {
    T::structure(
        "Header2",
        vec![
            ("flags", T::flags("ForkFlags", T::u16(Big), &[(0, "resource fork present")])),
            ("has_resource", T::computed(E::field("flags").sub(E::field("flags").div(E::lit(2)).mul(E::lit(2))))),
            ("unknown_a", T::u16(Big)),
            ("file_type", code()),
            ("creator", code()),
            ("finder_flags", T::flags("FinderFlags", T::u16(Big), FINDER_FLAGS)),
            ("unknown_b", T::u16(Big)),
            ("maybe_date", T::u32(Big)),
            ("unknown_c", T::bytes(E::lit(12))),
            ("unknown_d", T::bytes(E::lit(4))),
            ("resource_part", T::switch(E::field("has_resource"), vec![(1, sit5_resource_part())], T::bytes(E::lit(0)))),
            ("data_fork", T::bytes(E::field("data_compressed_length"))),
        ],
    )
}

fn sit5_resource_part() -> T {
    T::structure(
        "ResourcePart",
        vec![
            ("resource_original_length", T::u32(Big)),
            ("resource_compressed_length", T::u32(Big)),
            ("resource_crc", T::u16(Big)),
            ("unknown", T::u16(Big)),
            ("resource_method", T::enumeration_hex("Method", T::u8(), SIT_METHOD)),
            ("password_length", T::u8()),
            ("resource_fork", T::bytes(E::field("resource_compressed_length"))),
        ],
    )
}

/// Compact Pro archives. Directory entries form a recursive depth-first tree;
/// each file also places its two compressed forks at its absolute data offset.
pub fn compactpro() -> Template {
    let entry = compact_entry();
    Template::new(
        "compactpro",
        T::sized(
            E::Remaining,
            T::structure(
                "CompactPro",
                vec![
                ("magic", T::magic(b"\x01")),
                ("volume", T::u8()),
                ("cross_volume_id", T::u16(Big)),
                ("directory_offset", T::u32(Big)),
                ("compressed_data", T::at(E::lit(8), T::bytes(E::field("directory_offset").sub(E::lit(8))))),
                ("directory", T::at(E::field("directory_offset"), compact_directory())),
                ],
            ),
        ),
    )
    .with_type("CompactEntry", entry)
}

fn compact_directory() -> T {
    T::structure(
        "Directory",
        vec![
            ("crc", T::u32(Big)),
            ("entry_count", T::u16(Big)),
            ("comment_length", T::u8()),
            ("comment", text(E::field("comment_length"))),
            ("entries", T::array(T::Named("CompactEntry".into()), E::field("entry_count"))),
        ],
    )
}

fn compact_entry() -> T {
    T::structure_named(
        "CompactEntry",
        "name",
        "body",
        vec![
            ("name_and_kind", T::flags("EntryKind", T::u8(), &[(7, "directory")])),
            ("is_directory", T::computed(E::field("name_and_kind").div(E::lit(128)))),
            ("name_length", T::computed(E::field("name_and_kind").sub(E::field("is_directory").mul(E::lit(128))))),
            ("name", text(E::field("name_length"))),
            ("body", T::switch(E::field("is_directory"), vec![(1, compact_folder())], compact_file())),
        ],
    )
    .counted_as("entry")
}

fn compact_folder() -> T {
    // This counts all following records in the subtree, not only immediate
    // children. The directory array is already depth-first, so nesting it
    // here would consume descendants of nested folders twice.
    T::structure("Folder", vec![("subtree_count", T::u16(Big))])
}

fn compact_file() -> T {
    T::structure(
        "File",
        vec![
            ("volume", T::u8()),
            ("data_offset", T::u32(Big)),
            ("file_type", code()),
            ("creator", code()),
            ("created", T::u32(Big)),
            ("modified", T::u32(Big)),
            ("finder_flags", T::flags("FinderFlags", T::u16(Big), FINDER_FLAGS)),
            ("data_crc", T::u32(Big)),
            ("flags", T::flags("FileFlags", T::u16(Big), &[(0, "encrypted"), (1, "resource fork LZH"), (2, "data fork LZH")])),
            ("resource_original_length", T::u32(Big)),
            ("data_original_length", T::u32(Big)),
            ("resource_compressed_length", T::u32(Big)),
            ("data_compressed_length", T::u32(Big)),
            (
                "forks",
                T::at(
                    E::field("data_offset"),
                    T::structure(
                        "Forks",
                        vec![("resource", T::bytes(E::field("resource_compressed_length"))), ("data", T::bytes(E::field("data_compressed_length")))],
                    ),
                ),
            ),
        ],
    )
}

/// BinHex 4.0's text envelope. The payload is six-bit encoded and RLE90
/// compressed, so its decoded fork structure is not at physical file offsets.
pub fn binhex() -> Template {
    Template::new(
        "binhex",
        T::structure(
            "BinHex",
            vec![
                ("preamble", text(E::to_bytes(b":"))),
                ("open", T::magic(b":")),
                ("encoded_payload", T::text(StrLen::Fixed(E::to_last_bytes(b":")), Encoding::Ascii)),
                ("close", T::magic(b":")),
                ("trailing", T::bytes(E::Remaining)),
            ],
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
    fn macbinary_places_aligned_forks() {
        let mut b = vec![0u8; 128];
        b[1] = 1;
        b[2] = b'X';
        b[83..87].copy_from_slice(&3u32.to_be_bytes());
        b[87..91].copy_from_slice(&2u32.to_be_bytes());
        b.extend_from_slice(b"abc");
        b.resize(256, 0);
        b.extend_from_slice(b"RS");
        b.resize(384, 0);
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(macbinary());
        assert_eq!(ev.node(&d, &[28]).unwrap().offset_bits, 128 * 8);
        assert_eq!(ev.node(&d, &[30]).unwrap().offset_bits, 256 * 8);
    }

    #[test]
    fn classic_finder_bits_have_their_real_numbers() {
        let mut b = vec![0u8; 128];
        b[1] = 1;
        b[2] = b'X';
        b[73] = 0x40;
        b[101] = 0x40;
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(macbinary());
        assert!(matches!(ev.node(&d, &[6]).unwrap().value, Value::Flags { ref set, .. } if set == &["invisible"]));
        assert!(matches!(ev.node(&d, &[18]).unwrap().value, Value::Flags { ref set, .. } if set == &["shared"]));
    }

    #[test]
    fn old_mac_containers_are_sniffed_without_extensions() {
        let mut mb = vec![0u8; 128];
        mb[1] = 1;
        mb[2] = b'X';
        mb[65..73].copy_from_slice(b"TEXTttxt");
        assert_eq!(crate::formats::sniff(&mb, mb.len() as u64), Some("macbinary"));

        let hqx = b"(This file must be converted with BinHex 4.0):!!!!:";
        assert_eq!(crate::formats::sniff(hqx, hqx.len() as u64), Some("binhex"));

        let mut classic = vec![0u8; 22];
        classic[..4].copy_from_slice(b"SIT!");
        classic[10..14].copy_from_slice(b"rLau");
        assert_eq!(crate::formats::sniff(&classic, classic.len() as u64), Some("stuffit"));

        let mut sit5 = vec![0u8; 100];
        sit5[..80].copy_from_slice(b"StuffIt (c)1997-1997 Aladdin Systems, Inc., http://www.aladdinsys.com/StuffIt/\r\n");
        assert_eq!(crate::formats::sniff(&sit5, sit5.len() as u64), Some("stuffit"));

        let cpt = [1, 1, 0, 0, 0, 0, 0, 8, 0];
        assert_eq!(crate::formats::sniff(&cpt, cpt.len() as u64), Some("compactpro"));
    }
}
