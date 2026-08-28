//! RPM packages: a lead nobody reads any more, two headers of tagged values,
//! and a compressed archive of the files.
//!
//! The two headers are the same structure twice. The first holds the
//! signatures and digests of everything after it; the second holds what the
//! package is, who built it, what it needs, and every path it installs. Each
//! is a run of sixteen-byte index entries and a store of bytes those entries
//! point into, so the values are in no particular order and the room between
//! them is padding: a value of four-byte numbers starts on a four-byte
//! boundary, and what that skips belongs to nothing.
//!
//! Which is why the store is a pointer list here rather than a run of values.
//! Reading it in order would mean guessing at padding; reading it by the
//! offsets the entries hold puts every value where its own entry says, and
//! what no entry claims stays a gap.
//!
//! One asymmetry is worth knowing about, because a file read a byte late is
//! unreadable from there on: the signature header is padded to a multiple of
//! eight bytes and the header after it is not.
//!
//! The payload is a cpio archive, compressed with whatever the build chose,
//! and naming which compressor that was is as far as this goes.

use crate::template::{Anchor, Endian::Big, Expr as E, Template, Ty as T};

/// What the lead starts with, and the only part of the lead anything trusts.
pub const MAGIC: &[u8] = b"\xed\xab\xee\xdb";

/// What both headers start with: three bytes and a version of one.
const HEADER_MAGIC: &[u8] = b"\x8e\xad\xe8";

pub fn rpm() -> Template {
    Template::new(
        "rpm",
        T::structure(
            "RpmPackage",
            vec![
                ("lead", lead()),
                ("signature", header("RpmSignature", "RpmSigTag", SIGNATURE_TAGS)),
                // The signature section is padded so that the header after it
                // starts on an eight-byte boundary. Nothing pads the header
                // itself, and a reader that pads both is one byte into the
                // payload before it notices.
                ("signature_padding", T::bytes(E::size_of("signature").pad_to(8))),
                ("header", header("RpmHeader", "RpmTag", HEADER_TAGS)),
                ("payload", payload()),
            ],
        ),
    )
}

/// The ninety-six bytes at the front, which say the package's name, its
/// architecture and its operating system as numbers from a list that stopped
/// growing in the nineties. `rpm` itself reads nothing here but the magic:
/// everything in it is written again, properly, in the header, and a name too
/// long for sixty-six bytes is simply cut off. It is kept because every tool
/// that identifies a file reads it.
fn lead() -> T {
    T::structure(
        "RpmLead",
        vec![
            ("magic", T::magic(MAGIC)),
            ("major", T::u8()),
            ("minor", T::u8()),
            ("type", T::enumeration("RpmPackageType", T::u16(Big), &[(0, "binary"), (1, "source")])),
            ("archnum", T::u16(Big)),
            ("name", T::utf8_padded(E::lit(66), 0)),
            ("osnum", T::u16(Big)),
            ("signature_type", T::enumeration("RpmSignatureType", T::u16(Big), &[(5, "header")])),
            ("reserved", T::bytes(E::lit(16))),
        ],
    )
}

/// A header: how many entries and how big the store is, then the entries, then
/// the store with a value at every offset an entry named.
fn header(name: &str, tag_enum: &str, tags: &'static [(i128, &'static str)]) -> T {
    T::structure(
        name,
        vec![
            ("magic", T::magic(HEADER_MAGIC)),
            ("version", T::u8()),
            ("reserved", T::bytes(E::lit(4))),
            ("index_count", T::u32(Big)),
            ("store_size", T::u32(Big)),
            ("entries", T::array(entry(tag_enum, tags), E::field("index_count"))),
            // Placed by the offsets above, counted from the start of the
            // store, which is what the window makes the anchor.
            (
                "store",
                T::sized(
                    E::field("store_size"),
                    // The list sits inside the window rather than being it, so
                    // that the offsets count from the store's own start: a
                    // window does not anchor the list that is itself.
                    T::structure(
                        "RpmStore",
                        vec![(
                            "values",
                            T::pointer_list_sized("entries", &["offset"], Anchor::Window, E::lit(0), value()),
                        )],
                    ),
                ),
            ),
        ],
    )
}

/// One row of the index: what the value is, what shape it has, where in the
/// store it starts and how many of it there are. A tag with no name here is
/// still a tag, and shows as its number.
fn entry(tag_enum: &str, tags: &'static [(i128, &'static str)]) -> T {
    T::inline_structure(
        "RpmIndexEntry",
        vec![
            ("tag", T::enumeration(tag_enum, T::u32(Big), tags)),
            ("type", T::enumeration("RpmType", T::u32(Big), TYPES)),
            ("offset", T::u32(Big)),
            ("count", T::u32(Big)),
        ],
    )
    .counted_as("entry")
}

/// What one entry points at. Every value is a run of `count` of something,
/// including the ones a reader thinks of as single: a package's size is an
/// array of one number, because that is how the format writes it.
///
/// A type nobody defined covers no bytes rather than a guessed number of
/// them: a wrong length here would place every value after it.
fn value() -> T {
    let count = || E::elem_field("entries", E::idx(), &["count"]);
    T::switch(
        E::elem_field("entries", E::idx(), &["type"]),
        vec![
            (0, T::bytes(E::lit(0))),
            (1, T::array(T::u8(), count())),
            (2, T::array(T::u8(), count())),
            (3, T::array(T::u16(Big), count())),
            (4, T::array(T::u32(Big), count())),
            (5, T::array(T::u64(Big), count())),
            // A string is one string whatever the count says.
            (6, T::cstr()),
            (7, T::bytes(count())),
            (8, T::array(T::cstr(), count())),
            (9, T::array(T::cstr(), count())),
        ],
        T::bytes(E::lit(0)),
    )
}

/// The files, as a cpio archive that has been compressed. Which compressor is
/// written in the header as text, in `payload_compressor`, and is also in the
/// first bytes here, which is what this reads: a package built with a
/// compressor nobody has heard of still says so in its own first bytes.
fn payload() -> T {
    let stream = |name: &str| T::structure(name, vec![("data", T::bytes(E::Remaining))]);
    let named = T::switch(
        E::peek(16, Big),
        vec![
            (0x1f8b, stream("GzipStream")),
            (0xfd37, stream("XzStream")),
            (0x28b5, stream("ZstdStream")),
            (0x425a, stream("Bzip2Stream")),
            (0x5d00, stream("LzmaStream")),
            (0x0422, stream("Lz4Stream")),
            // A package built with no compression at all, which is the
            // archive itself.
            (0x3037, stream("CpioArchive")),
        ],
        T::bytes(E::Remaining),
    );
    T::switch(E::lit(1).less_than(E::Remaining), vec![(1, named)], T::bytes(E::Remaining))
}

/// What an index entry's bytes are.
const TYPES: &[(i128, &str)] = &[
    (0, "null"),
    (1, "char"),
    (2, "int8"),
    (3, "int16"),
    (4, "int32"),
    (5, "int64"),
    (6, "string"),
    (7, "bin"),
    (8, "string_array"),
    (9, "i18n_string"),
];

/// The tags of the signature header: digests and signatures over what follows
/// it, and the sizes those cover. The numbers overlap with the header's own
/// tags and mean something else entirely, which is why the two have separate
/// lists: 1000 here is how big the rest of the file is, and 1000 in the header
/// is the package's name.
const SIGNATURE_TAGS: &[(i128, &str)] = &[
    (62, "signatures"),
    (267, "dsa"),
    (268, "rsa"),
    (269, "sha1"),
    (270, "long_size"),
    (271, "long_archive_size"),
    (273, "sha256"),
    (1000, "size"),
    (1002, "pgp"),
    (1004, "md5"),
    (1005, "gpg"),
    (1007, "payload_size"),
    (1008, "reserved_space"),
];

/// The tags of the header proper. There are hundreds; these are the ones a
/// package written today fills in, and the rest show as their numbers.
const HEADER_TAGS: &[(i128, &str)] = &[
    (62, "immutable"),
    (63, "regions"),
    (100, "i18n_table"),
    (1000, "name"),
    (1001, "version"),
    (1002, "release"),
    (1003, "epoch"),
    (1004, "summary"),
    (1005, "description"),
    (1006, "build_time"),
    (1007, "build_host"),
    (1009, "size"),
    (1010, "distribution"),
    (1011, "vendor"),
    (1014, "license"),
    (1015, "packager"),
    (1016, "group"),
    (1020, "url"),
    (1021, "os"),
    (1022, "arch"),
    (1028, "file_sizes"),
    (1030, "file_modes"),
    (1033, "file_devices"),
    (1034, "file_mtimes"),
    (1035, "file_digests"),
    (1036, "file_link_targets"),
    (1037, "file_flags"),
    (1039, "file_user_name"),
    (1040, "file_group_name"),
    (1044, "source_rpm"),
    (1047, "provide_name"),
    (1048, "require_flags"),
    (1049, "require_name"),
    (1050, "require_version"),
    (1053, "conflict_flags"),
    (1054, "conflict_name"),
    (1055, "conflict_version"),
    (1064, "rpm_version"),
    (1090, "obsolete_name"),
    (1112, "provide_flags"),
    (1113, "provide_version"),
    (1116, "dir_indexes"),
    (1117, "base_names"),
    (1118, "dir_names"),
    (1124, "payload_format"),
    (1125, "payload_compressor"),
    (1126, "payload_flags"),
    (5092, "payload_digest"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::Document,
        eval::{Evaluator, Value},
        source::MemSource,
    };

    /// One index entry: tag, type, where in the store, how many.
    fn index(tag: u32, ty: u32, offset: u32, count: u32) -> Vec<u8> {
        [tag, ty, offset, count].iter().flat_map(|n| n.to_be_bytes()).collect()
    }

    fn section(entries: &[Vec<u8>], store: &[u8]) -> Vec<u8> {
        let mut v = HEADER_MAGIC.to_vec();
        v.push(1);
        v.extend_from_slice(&[0; 4]);
        v.extend_from_slice(&(entries.len() as u32).to_be_bytes());
        v.extend_from_slice(&(store.len() as u32).to_be_bytes());
        for e in entries {
            v.extend_from_slice(e);
        }
        v.extend_from_slice(store);
        v
    }

    fn lead_bytes() -> Vec<u8> {
        let mut v = MAGIC.to_vec();
        v.extend_from_slice(&[3, 0]);
        v.extend_from_slice(&0u16.to_be_bytes()); // binary
        v.extend_from_slice(&1u16.to_be_bytes()); // i386
        let mut name = b"qubero-1.0-1".to_vec();
        name.resize(66, 0);
        v.extend_from_slice(&name);
        v.extend_from_slice(&1u16.to_be_bytes());
        v.extend_from_slice(&5u16.to_be_bytes());
        v.extend_from_slice(&[0; 16]);
        assert_eq!(v.len(), 96);
        v
    }

    /// A package whose signature section is 36 bytes long, so four of padding
    /// follow it, and whose header holds a string, a number placed after the
    /// byte that aligns it, and an array of two strings.
    fn package() -> Vec<u8> {
        let mut v = lead_bytes();
        v.extend_from_slice(&section(&[index(1000, 4, 0, 1)], &1234u32.to_be_bytes()));
        v.extend_from_slice(&[0; 4]);
        let mut store = b"qubero\0".to_vec();
        store.push(0); // the byte that puts the number on a four-byte boundary
        store.extend_from_slice(&42u32.to_be_bytes());
        store.extend_from_slice(b"usr\0bin\0");
        v.extend_from_slice(&section(
            &[index(1000, 6, 0, 1), index(1009, 4, 8, 1), index(1117, 8, 12, 2)],
            &store,
        ));
        v.extend_from_slice(b"\x1f\x8b\x08\x00the files");
        v
    }

    #[test]
    fn the_lead_says_what_the_package_is_called() {
        let d = Document::new(MemSource(package()));
        let mut e = Evaluator::new(rpm());
        assert_eq!(e.node(&d, &[0, 5]).unwrap().value, Value::Str("qubero-1.0-1".into()));
    }

    /// Four bytes of padding after the signature section and none after the
    /// header: get this wrong and everything below is read at the wrong offset.
    #[test]
    fn only_the_signature_section_is_padded() {
        let d = Document::new(MemSource(package()));
        let mut e = Evaluator::new(rpm());
        assert_eq!(e.node(&d, &[1]).unwrap().size_bits, 36 * 8);
        assert_eq!(e.node(&d, &[2]).unwrap().size_bits, 4 * 8);
        assert_eq!(e.node(&d, &[3]).unwrap().offset_bits, (96 + 40) * 8);
    }

    /// Every value is where its own entry says, not where the one before it
    /// ended: the byte between the name and the number belongs to nobody.
    #[test]
    fn a_value_is_read_at_the_offset_its_entry_holds() {
        let d = Document::new(MemSource(package()));
        let mut e = Evaluator::new(rpm());
        assert_eq!(e.node(&d, &[3, 6, 0, 0]).unwrap().value, Value::Str("qubero".into()));
        assert_eq!(e.node(&d, &[3, 6, 0, 1, 0]).unwrap().value.as_int(), Some(42));
        assert_eq!(e.node(&d, &[3, 6, 0, 2]).unwrap().child_count, 2);
        assert_eq!(e.node(&d, &[3, 6, 0, 2, 1]).unwrap().value, Value::Str("bin".into()));
    }

    #[test]
    fn the_payload_is_named_by_the_compressor_that_wrote_it() {
        let d = Document::new(MemSource(package()));
        let mut e = Evaluator::new(rpm());
        let payload = e.node(&d, &[4]).unwrap();
        assert_eq!(payload.size_bits, 13 * 8);
        assert!(payload.type_name.contains("Gzip"), "not named as a gzip stream: {}", payload.type_name);
    }
}
