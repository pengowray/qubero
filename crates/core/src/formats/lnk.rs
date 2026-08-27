//! Windows Shell Link (`.lnk`): the fixed header followed by the pieces its
//! link flags select.  The target's PIDL and extra-data blocks are shell-owned
//! binary records, so this template keeps their contents whole while exposing
//! the lengths that delimit them.

use crate::template::{Encoding, Endian::Little, Expr as E, StrLen, Template, Ty as T};

const LINK_FLAGS: &[(u32, &str)] = &[
    (0, "has link target id list"),
    (1, "has link info"),
    (2, "has name"),
    (3, "has relative path"),
    (4, "has working directory"),
    (5, "has arguments"),
    (6, "has icon location"),
    (7, "is unicode"),
    (8, "force no link info"),
    (9, "has exp string"),
    (10, "run in separate process"),
    (13, "run as user"),
    (19, "enable target metadata"),
];

const FILE_ATTRIBUTES: &[(u32, &str)] = &[
    (0, "read only"), (1, "hidden"), (2, "system"), (4, "directory"),
    (5, "archive"), (7, "normal"), (8, "temporary"), (9, "sparse file"),
    (10, "reparse point"), (11, "compressed"), (12, "offline"),
    (13, "not content indexed"), (14, "encrypted"),
];

const SHOW_COMMAND: &[(i128, &str)] = &[(1, "normal"), (3, "maximized"), (7, "minimized")];

/// Bit `n` of `link_flags`, as one or zero.  Expressions deliberately have no
/// bitwise operators; this arithmetic form is also used by the gzip template.
fn bit(n: u32) -> E {
    let flags = E::field("link_flags");
    flags.clone().div(E::lit(1i128 << n)).sub(flags.div(E::lit(1i128 << (n + 1))).mul(E::lit(2)))
}

fn absent_or(flag: u32, present: T) -> T {
    T::switch(bit(flag), vec![(1, present)], T::bytes(E::lit(0)))
}

pub fn lnk() -> Template {
    Template::new(
        "lnk",
        T::structure(
            "ShellLink",
            vec![
                ("header_size", T::magic(b"\x4c\0\0\0")),
                // LinkCLSID {00021401-0000-0000-C000-000000000046}, stored
                // in the mixed-endian byte order Windows uses for GUIDs.
                ("link_clsid", T::magic(b"\x01\x14\x02\0\0\0\0\0\xc0\0\0\0\0\0\0\x46")),
                ("link_flags", T::flags("LinkFlags", T::u32(Little), LINK_FLAGS)),
                ("file_attributes", T::flags("FileAttributes", T::u32(Little), FILE_ATTRIBUTES)),
                ("creation_time", T::u64(Little)),
                ("access_time", T::u64(Little)),
                ("write_time", T::u64(Little)),
                ("file_size", T::u32(Little)),
                ("icon_index", T::i32(Little)),
                ("show_command", T::enumeration("ShowCommand", T::u32(Little), SHOW_COMMAND)),
                ("hot_key", T::u16(Little)),
                ("reserved1", T::u16(Little)),
                ("reserved2", T::u32(Little)),
                ("reserved3", T::u32(Little)),
                ("target_id_list", absent_or(0, id_list())),
                ("link_info", absent_or(1, link_info())),
                ("name", absent_or(2, string_data())),
                ("relative_path", absent_or(3, string_data())),
                ("working_directory", absent_or(4, string_data())),
                ("arguments", absent_or(5, string_data())),
                ("icon_location", absent_or(6, string_data())),
                // ExtraData is a sequence of signature-specific blocks ending
                // in a zero size.  Keeping it raw avoids pretending that a
                // shell extension's private block has a universal layout.
                ("extra_data", T::bytes(E::Remaining)),
            ],
        ),
    )
}

fn id_list() -> T {
    T::structure("LinkTargetIDList", vec![
        ("size", T::u16(Little)),
        ("items", T::bytes(E::field("size"))),
    ])
}

fn link_info() -> T {
    T::structure("LinkInfo", vec![
        ("size", T::u32(Little)),
        ("body", T::sized(E::field("size").sub(E::lit(4)), link_info_body())),
    ])
}

fn link_info_body() -> T {
    T::structure("LinkInfoBody", vec![
        ("header_size", T::u32(Little)),
        ("flags", T::u32(Little)),
        ("volume_id_offset", T::u32(Little)),
        ("local_base_path_offset", T::u32(Little)),
        ("common_network_relative_link_offset", T::u32(Little)),
        ("common_path_suffix_offset", T::u32(Little)),
        // Present only in the 0x24-byte LinkInfo header introduced for
        // Unicode paths.  The remaining bytes include the offset-addressed
        // strings and shell-defined VolumeID/network records.
        ("unicode_offsets", T::switch(E::field("header_size"), vec![(0x24, T::structure("UnicodeOffsets", vec![
            ("local_base_path_offset_unicode", T::u32(Little)),
            ("common_path_suffix_offset_unicode", T::u32(Little)),
        ]))], T::bytes(E::lit(0)))),
        ("contents", T::bytes(E::Remaining)),
    ])
}

fn string_data() -> T {
    T::structure("StringData", vec![
        ("characters", T::u16(Little)),
        ("value", T::switch(
            bit(7),
            vec![(1, T::text(StrLen::Fixed(E::field("characters").mul(E::lit(2))), Encoding::Utf16(Little)))],
            T::text(StrLen::Fixed(E::field("characters")), Encoding::Latin1),
        )),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn shortcut(flags: u32, tail: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"\x4c\0\0\0\x01\x14\x02\0\0\0\0\0\xc0\0\0\0\0\0\0\x46");
        v.extend_from_slice(&flags.to_le_bytes());
        v.resize(76, 0);
        v.extend_from_slice(tail);
        v
    }

    #[test]
    fn a_unicode_name_and_relative_path_follow_their_flag_bits() {
        let mut tail = Vec::new();
        tail.extend_from_slice(&4u16.to_le_bytes());
        tail.extend("Menu".encode_utf16().flat_map(u16::to_le_bytes));
        tail.extend_from_slice(&9u16.to_le_bytes());
        tail.extend(".\\app.exe".encode_utf16().flat_map(u16::to_le_bytes));
        let v = shortcut((1 << 2) | (1 << 3) | (1 << 7), &tail);
        assert_eq!(crate::formats::sniff(&v, v.len() as u64), Some("lnk"));
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(lnk());
        assert_eq!(ev.node(&d, &[16, 1]).unwrap().value, Value::Str("Menu".into()));
        assert_eq!(ev.node(&d, &[17, 1]).unwrap().value, Value::Str(".\\app.exe".into()));
        assert_eq!(ev.node(&d, &[18]).unwrap().size_bits, 0);
    }

    #[test]
    fn an_ansi_name_uses_one_byte_characters() {
        let v = shortcut(1 << 2, &[3, 0, b'f', b'o', b'o']);
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(lnk());
        assert_eq!(ev.node(&d, &[16, 1]).unwrap().value, Value::Str("foo".into()));
        assert_eq!(ev.node(&d, &[16]).unwrap().size_bits, 5 * 8);
    }
}
