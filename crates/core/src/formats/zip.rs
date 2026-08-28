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
/// The tag on the extra field that carries an entry's 64-bit sizes. A ZIP
/// says a size it cannot fit in 32 bits by writing 0xFFFFFFFF where the size
/// goes and the real one in an extra field tagged 1, and the advice a writer
/// is given is increasingly to do that for every entry whatever its size.
const ZIP64_EXTRA: i128 = 0x0001;
/// What a 32-bit field holds when the real value is in a ZIP64 extra field.
const MASKED32: i128 = 0xFFFF_FFFF;
/// What a 16-bit one holds for the same reason.
const MASKED16: i128 = 0xFFFF;
const EXTRA_IDS: &[(i128, &str)] = &[
    (0x0001, "ZIP64 extended info"),
    (0x0007, "AV info"),
    (0x0008, "language"),
    (0x0009, "OS/2"),
    (0x000a, "NTFS times"),
    (0x000c, "OpenVMS"),
    (0x000d, "UNIX"),
    (0x000e, "stream and fork descriptor"),
    (0x000f, "patch descriptor"),
    (0x0014, "PKCS#7 certificates"),
    (0x0015, "file signature"),
    (0x0016, "directory signature"),
    (0x0017, "strong encryption"),
    (0x0019, "certificate list"),
    (0x0065, "IBM attributes"),
    (0x0066, "IBM compressed attributes"),
    (0x4690, "POSZIP"),
    (0x5455, "extended timestamp"),
    (0x5855, "Info-ZIP UNIX"),
    (0x6375, "Unicode comment"),
    (0x7075, "Unicode name"),
    (0x7855, "Info-ZIP UNIX 2"),
    (0x7875, "Info-ZIP UNIX 3"),
    (0x9901, "AES encryption"),
    (0xa11e, "alignment padding"),
    (0xa220, "growth hint"),
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
    archive("zip")
}

/// A Zarr store written into a ZIP, which zarr-python calls a ZipStore. The
/// records are the records of any archive; the name is what says the entries
/// are a store's metadata and chunks rather than loose files.
pub fn zarrzip() -> Template {
    archive("zarrzip")
}

fn archive(name: &str) -> Template {
    Template::new(
        name,
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
                    // A signature nobody defined: bytes to the next `PK`, so a
                    // damaged stretch is one run rather than a 4-byte "record"
                    // out of every position of it.
                    T::bytes(E::to_bytes(b"PK")),
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

/// Bit `n` of `flags`, as a number that is one or zero.
fn flag_bit(n: u32) -> E {
    let f = E::field("flags");
    f.clone().div(E::lit(1i128 << n)).sub(f.div(E::lit(1i128 << (n + 1))).mul(E::lit(2)))
}

/// One when the 32-bit `field` holds 0xFFFFFFFF, which is a ZIP saying the
/// real number is in an extra field rather than here. Written as a comparison
/// because there is no test for equality: nothing else is above 0xFFFFFFFE.
fn masked(field: &str) -> E {
    E::lit(MASKED32 - 1).less_than(E::field(field))
}

/// The value `field` holds, or zero when it is the placeholder.
fn if_written(field: &str) -> E {
    E::field(field).mul(E::lit(1).sub(masked(field)))
}

/// How long a local entry's data is.
///
/// The header says, except in two cases, and an archive can be in both at
/// once. An entry too big for 32 bits, or one from a writer that no longer
/// bothers to ask, has 0xFFFFFFFF in `compressed_size` and its real size in
/// the extra field tagged 1. An entry written as a stream has zero (or the
/// placeholder) there, sets flag bit 3, and writes the real sizes in a data
/// descriptor after the data; the descriptor's own signature is the only mark
/// of where the data ends, so the length is measured by looking for it.
/// Walking the file record by record instead reads a 4-byte "record" out of
/// every position of the stream, which is what made a streamed archive take
/// minutes to open.
///
/// `Or` only asks its right side when the left is zero, so a sized entry
/// never looks at the extra fields and never scans. A streamed ZIP64 entry
/// asks both: its extra field is there but holds zeros, since the writer did
/// not know the sizes either, and the answer is the scan. The innermost `Or`
/// is the guard the template idiom uses for a question it cannot afford to
/// ask: when the descriptor flag is off, the left side is already 1 and the
/// scan is never run, and the 1 borrowed to say so is taken off again. A
/// zero-byte entry with no descriptor lands there and correctly measures zero.
///
/// A descriptor written without its (optional) signature is not found; the
/// scan then runs to the next entry that has one, or to the end of the file,
/// and the entries in between read as one run of data. Imperfect, but the
/// bytes are still there to look at.
fn data_len() -> E {
    let scan = E::to_bytes(b"PK\x07\x08").add(E::lit(1));
    let no_descriptor = E::lit(1).sub(flag_bit(3));
    let zip64 = E::tagged("extra", &["id"], ZIP64_EXTRA, &["data", "compressed_size"]);
    if_written("compressed_size").or(zip64.or(no_descriptor.or(scan).sub(E::lit(1))))
}

/// How long the entry is once unpacked, from wherever the writer put it.
/// Zero for a streamed entry, which is what its header says as well: the
/// number it did not know yet is in the descriptor after the data.
fn unpacked_len() -> E {
    let zip64 = E::tagged("extra", &["id"], ZIP64_EXTRA, &["data", "uncompressed_size"]);
    if_written("uncompressed_size").or(zip64)
}

/// The extra fields on a header: tagged records, in whatever order the writer
/// put them, holding everything the fixed header has no room for. `zip64` is
/// how the record tagged 1 reads, which is not the same in a local header as
/// in a central directory one.
fn extras(zip64: T) -> T {
    T::sized(
        E::field("extra_length"),
        T::repeat(
            T::structure_named(
                "ExtraField",
                "id",
                "data",
                vec![
                    ("id", T::enumeration_hex("ExtraFieldId", T::u16(Little), EXTRA_IDS)),
                    ("size", T::u16(Little)),
                    (
                        "data",
                        T::sized(
                            E::field("size"),
                            T::switch(E::field("id"), vec![(ZIP64_EXTRA, zip64)], T::bytes(E::Remaining)),
                        ),
                    ),
                ],
            ),
            Until::End,
        ),
    )
}

/// The ZIP64 record of a local header, which holds both sizes whatever the
/// header said (APPNOTE 4.5.3). A record too short for them is left as bytes
/// rather than read past its end: the placeholder is then never answered and
/// the entry measures itself the way a streamed one does.
fn zip64_local() -> T {
    T::switch(
        E::lit(15).less_than(E::Remaining),
        vec![(
            1,
            T::structure(
                "Zip64Sizes",
                vec![
                    ("uncompressed_size", T::u64(Little)),
                    ("compressed_size", T::u64(Little)),
                    ("rest", T::bytes(E::Remaining)),
                ],
            ),
        )],
        T::bytes(E::Remaining),
    )
}

/// The ZIP64 record of a central directory header, where the same tag means
/// something else: only the fields whose 32-bit counterparts hold the
/// placeholder are written, in this order, so an entry past 4 GB from the
/// front of a small archive carries an offset and no sizes at all.
fn zip64_central() -> T {
    T::structure(
        "Zip64Fields",
        vec![
            ("uncompressed_size", present_if(masked("uncompressed_size"), 8, T::u64(Little))),
            ("compressed_size", present_if(masked("compressed_size"), 8, T::u64(Little))),
            ("local_header_offset", present_if(masked("local_header_offset"), 8, T::u64(Little))),
            ("disk_number", present_if(E::lit(MASKED16 - 1).less_than(E::field("disk_number")), 4, T::u32(Little))),
            ("rest", T::bytes(E::Remaining)),
        ],
    )
}

/// A field that is there only when `when` says so, and only while the record
/// still has room for it. A writer that leaves one out has to be read as
/// having left it out, or every field after it is read from the wrong bytes.
fn present_if(when: E, bytes: i128, ty: T) -> T {
    let room = E::lit(bytes - 1).less_than(E::Remaining);
    T::switch(when.mul(room), vec![(1, ty)], T::bytes(E::lit(0)))
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
            ("extra", extras(zip64_local())),
            // The two sizes again, now that the extra fields have been read
            // and the placeholders above can be answered. A plain archive
            // repeats itself here; a ZIP64 one says what it meant.
            ("data_size", T::computed(data_len())),
            ("unpacked_size", T::computed(unpacked_len())),
            ("data", T::bytes(E::field("data_size"))),
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
            ("extra", extras(zip64_central())),
            ("comment", text("comment_length")),
        ],
    )
}

/// The record a streamed entry writes after its data, holding the numbers its
/// header could not.
///
/// Nothing in the descriptor says how wide its two sizes are: they are eight
/// bytes each when the entry is a ZIP64 one and four otherwise, and the only
/// way to know which is to ask the entry. `Sibling` searches back through the
/// records for the nearest one with a local header's `compressed_size`, which
/// is the entry this descriptor belongs to, and a placeholder there is what
/// made it ZIP64.
///
/// A writer that streams a ZIP64 entry while leaving the header sizes at zero
/// is read as the narrower one; the eight bytes left over then read as a
/// record nobody defined, which the walk passes over as bytes to the next
/// `PK`. Wrong shape, right place: the entries after it still line up.
fn descriptor() -> T {
    let wide = E::lit(MASKED32 - 1).less_than(E::sibling(&["body", "compressed_size"]));
    T::switch(
        wide,
        vec![(
            1,
            T::structure(
                "Zip64DataDescriptor",
                vec![
                    ("crc32", T::u32(Little)),
                    ("compressed_size", T::u64(Little)),
                    ("uncompressed_size", T::u64(Little)),
                ],
            ),
        )],
        T::structure(
            "DataDescriptor",
            vec![
                ("crc32", T::u32(Little)),
                ("compressed_size", T::u32(Little)),
                ("uncompressed_size", T::u32(Little)),
            ],
        ),
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
/// The end record a ZIP64 archive writes before the ordinary one, holding the
/// counts and offsets that do not fit in it. Its size counts the bytes after
/// itself, which is what leaves room for the fields a later version added:
/// version 2 of this record describes an encrypted central directory, and
/// whatever it holds sits in the room this record's own size left for it.
fn zip64_end() -> T {
    T::structure(
        "Zip64End",
        vec![
            ("record_size", T::u64(Little)),
            (
                "record",
                T::sized(
                    E::field("record_size"),
                    // A record too short for the fields it should hold is left
                    // as bytes, the way a truncated header is.
                    T::switch(E::lit(43).less_than(E::field("record_size")), vec![(1, T::structure(
                        "Zip64EndRecord",
                        vec![
                            ("version_made_by", T::u16(Little)),
                            ("version_needed", T::u16(Little)),
                            ("disk_number", T::u32(Little)),
                            ("directory_disk", T::u32(Little)),
                            ("entries_on_disk", T::u64(Little)),
                            ("entries_total", T::u64(Little)),
                            ("directory_size", T::u64(Little)),
                            ("directory_offset", T::u64(Little)),
                            ("extensible_data", T::bytes(E::Remaining)),
                        ],
                    ))], T::bytes(E::Remaining)),
                ),
            ),
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

    /// A local file header and its data, written the way a writer would.
    fn entry(name: &[u8], flags: u16, compressed: u32, uncompressed: u32, extra: &[u8], data: &[u8]) -> Vec<u8> {
        let mut v = b"PK\x03\x04".to_vec();
        v.extend_from_slice(&20u16.to_le_bytes());
        v.extend_from_slice(&flags.to_le_bytes());
        v.extend_from_slice(&[0; 10]); // method, times, crc
        v.extend_from_slice(&compressed.to_le_bytes());
        v.extend_from_slice(&uncompressed.to_le_bytes());
        v.extend_from_slice(&(name.len() as u16).to_le_bytes());
        v.extend_from_slice(&(extra.len() as u16).to_le_bytes());
        v.extend_from_slice(name);
        v.extend_from_slice(extra);
        v.extend_from_slice(data);
        v
    }

    /// The extra field that carries a local header's real sizes.
    fn zip64_extra(uncompressed: u64, compressed: u64) -> Vec<u8> {
        let mut v = 1u16.to_le_bytes().to_vec();
        v.extend_from_slice(&16u16.to_le_bytes());
        v.extend_from_slice(&uncompressed.to_le_bytes());
        v.extend_from_slice(&compressed.to_le_bytes());
        v
    }

    fn end_record() -> Vec<u8> {
        let mut v = b"PK\x05\x06".to_vec();
        v.extend_from_slice(&[0; 18]);
        v
    }

    #[test]
    fn local_entry_and_end_record() {
        let mut v = entry(b"a.txt", 0, 3, 3, &[], b"abc");
        v.extend_from_slice(&end_record());
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(zip());
        assert_eq!(e.node(&d, &[0]).unwrap().child_count, 2);
        assert_eq!(
            e.node(&d, &[0, 0, 1, 10]).unwrap().value,
            Value::Str("a.txt".into())
        );
        assert_eq!(e.node(&d, &[0, 0, 1, 14]).unwrap().size_bits, 24);
    }

    /// An entry written as a stream: `compressed_size` is zero, flag bit 3 is
    /// set, and the real sizes are in a data descriptor after the data. The
    /// data has to measure to the descriptor's signature, not to zero, or the
    /// whole rest of the archive reads as a 4-byte record per position.
    #[test]
    fn streamed_entry_measures_to_its_descriptor() {
        let mut v = entry(b"a.txt", 8, 0, 0, &[], b"seven by");
        v.extend_from_slice(b"PK\x07\x08"); // data descriptor
        v.extend_from_slice(&[0; 12]);
        v.extend_from_slice(&end_record());
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(zip());
        // Three records: the local file, its descriptor, and the end record.
        assert_eq!(e.node(&d, &[0]).unwrap().child_count, 3);
        // The data runs from the end of the name to the descriptor.
        assert_eq!(e.node(&d, &[0, 0, 1, 14]).unwrap().size_bits, 8 * 8);
        let sig = e.node(&d, &[0, 1, 0]).unwrap().value;
        assert!(matches!(sig, Value::Enum { raw: 0x0807_4b50, .. }), "descriptor not recognised: {sig:?}");
    }

    /// A writer told to use ZIP64 whatever the sizes writes 0xFFFFFFFF in the
    /// header and the real sizes in an extra field. Measuring the entry by the
    /// header would run four gigabytes past the end of the archive and take
    /// every record after it with it.
    #[test]
    fn a_zip64_entry_measures_by_its_extra_field() {
        let extra = zip64_extra(9, 4);
        let mut v = entry(b"a.txt", 0, 0xFFFF_FFFF, 0xFFFF_FFFF, &extra, b"abcd");
        v.extend_from_slice(&end_record());
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(zip());
        assert_eq!(e.node(&d, &[0]).unwrap().child_count, 2);
        // The sizes the entry really has, and the data measured by them.
        assert_eq!(e.node(&d, &[0, 0, 1, 12]).unwrap().value.as_int(), Some(4));
        assert_eq!(e.node(&d, &[0, 0, 1, 13]).unwrap().value.as_int(), Some(9));
        assert_eq!(e.node(&d, &[0, 0, 1, 14]).unwrap().size_bits, 32);
        // The extra field is read as a record, not as a run of bytes.
        assert_eq!(
            e.node(&d, &[0, 0, 1, 11, 0, 2, 1]).unwrap().value.as_int(),
            Some(4)
        );
        // And the size says where it came from, so a reader can go and look.
        let origins = e.origins(&d, &[0, 0, 1, 12]).unwrap();
        let from = origins.iter().find(|o| o.label.starts_with("extra[")).expect("no extra field named");
        assert_eq!(from.label, "extra[0].data.compressed_size");
        assert_eq!(from.path, vec![0, 0, 1, 11, 0, 2, 1]);
    }

    /// Both at once: a ZIP64 entry written as a stream. Its extra field is
    /// there, and holds zeros, because the writer did not know the sizes when
    /// it wrote the header either. The data measures to the descriptor, and
    /// the descriptor's sizes are eight bytes each.
    #[test]
    fn a_streamed_zip64_entry_falls_through_to_its_descriptor() {
        let extra = zip64_extra(0, 0);
        let mut v = entry(b"a.txt", 8, 0xFFFF_FFFF, 0xFFFF_FFFF, &extra, b"seven by");
        v.extend_from_slice(b"PK\x07\x08");
        v.extend_from_slice(&0u32.to_le_bytes()); // crc
        v.extend_from_slice(&8u64.to_le_bytes());
        v.extend_from_slice(&8u64.to_le_bytes());
        v.extend_from_slice(&end_record());
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(zip());
        assert_eq!(e.node(&d, &[0]).unwrap().child_count, 3);
        assert_eq!(e.node(&d, &[0, 0, 1, 14]).unwrap().size_bits, 8 * 8);
        // Twenty bytes of descriptor body: a crc and two eight-byte sizes.
        assert_eq!(e.node(&d, &[0, 1, 1]).unwrap().size_bits, 20 * 8);
        assert_eq!(e.node(&d, &[0, 1, 1, 1]).unwrap().value.as_int(), Some(8));
    }

    /// The same tag means something else in the central directory: only the
    /// fields whose 32-bit counterparts hold the placeholder are written. An
    /// entry that sits past four gigabytes but is small carries an offset and
    /// no sizes at all.
    #[test]
    fn a_central_zip64_field_is_only_the_part_that_did_not_fit() {
        let mut v = b"PK\x01\x02".to_vec();
        v.extend_from_slice(&20u16.to_le_bytes()); // version made by
        v.extend_from_slice(&20u16.to_le_bytes()); // version needed
        v.extend_from_slice(&[0; 12]); // flags, method, times, crc
        v.extend_from_slice(&7u32.to_le_bytes()); // compressed_size
        v.extend_from_slice(&7u32.to_le_bytes()); // uncompressed_size
        v.extend_from_slice(&5u16.to_le_bytes()); // name_length
        v.extend_from_slice(&12u16.to_le_bytes()); // extra_length
        v.extend_from_slice(&[0; 6]); // comment length, disk, internal attributes
        v.extend_from_slice(&0u32.to_le_bytes()); // external attributes
        v.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // offset: elsewhere
        v.extend_from_slice(b"a.txt");
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&8u16.to_le_bytes());
        v.extend_from_slice(&5_000_000_000u64.to_le_bytes());
        v.extend_from_slice(&end_record());
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(zip());
        assert_eq!(e.node(&d, &[0]).unwrap().child_count, 2);
        // The record's one written field is the offset, third in the order.
        let offset = e.node(&d, &[0, 0, 1, 17, 0, 2, 2]).unwrap();
        assert_eq!(offset.value.as_int(), Some(5_000_000_000));
    }

    /// The end record a ZIP64 archive writes before the ordinary one.
    #[test]
    fn the_zip64_end_record_reads_as_fields() {
        let mut v = b"PK\x06\x06".to_vec();
        v.extend_from_slice(&44u64.to_le_bytes()); // record size
        v.extend_from_slice(&45u16.to_le_bytes()); // version made by
        v.extend_from_slice(&45u16.to_le_bytes()); // version needed
        v.extend_from_slice(&[0; 8]); // this disk, the directory's disk
        v.extend_from_slice(&2u64.to_le_bytes()); // entries on this disk
        v.extend_from_slice(&2u64.to_le_bytes()); // entries in total
        v.extend_from_slice(&100u64.to_le_bytes()); // directory size
        v.extend_from_slice(&5_000_000_000u64.to_le_bytes()); // directory offset
        v.extend_from_slice(&end_record());
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(zip());
        assert_eq!(e.node(&d, &[0, 0, 1, 1, 5]).unwrap().value.as_int(), Some(2));
        assert_eq!(
            e.node(&d, &[0, 0, 1, 1, 7]).unwrap().value.as_int(),
            Some(5_000_000_000)
        );
    }
}
