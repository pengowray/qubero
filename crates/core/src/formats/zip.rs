//! ZIP archives as local entries, directory entries, descriptors, and the end record.

use crate::codec::Codec;
use crate::template::{Check, Checksum, Covers, Named, Encoding, Endian::Little, Expr as E, StrLen, Template, Time, Ty as T, Until};

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
            ("uncompressed_size", T::present_if(masked("uncompressed_size"), T::u64(Little))),
            ("compressed_size", T::present_if(masked("compressed_size"), T::u64(Little))),
            ("local_header_offset", T::present_if(masked("local_header_offset"), T::u64(Little))),
            (
                "disk_number",
                T::present_if(E::lit(MASKED16 - 1).less_than(E::field("disk_number")), T::u32(Little)),
            ),
            ("rest", T::bytes(E::Remaining)),
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
            ("extra", extras(zip64_local())),
            // The two sizes again, now that the extra fields have been read
            // and the placeholders above can be answered. A plain archive
            // repeats itself here; a ZIP64 one says what it meant.
            ("data_size", T::computed(data_len())),
            ("unpacked_size", T::computed(unpacked_len())),
            // Method 8 is deflate, and a deflate run opens: what came out of
            // it, and the blocks the decoder read to get there. Methods 12, 93
            // and 95 are a whole bzip2, zstd or xz stream written where the
            // deflate would be, and open the same way.
            //
            // Method 0 is the file written into the archive verbatim, so those
            // bytes are already a document and open too, through the codec
            // that copies. They used to be plain bytes, which meant an archive
            // of stored files offered nothing to open anywhere: not in the
            // listing, not on a chip, not as a tab.
            //
            // Two are named in the table above and not here. Method 9 is
            // deflate64, which nothing here reads. Method 14 is LZMA behind
            // nine bytes of ZIP's own preamble: the properties are inside the
            // run rather than in a field, so no expression can name them, and
            // a decoder handed the run whole would read the preamble as the
            // stream. It wants a codec that knows that preamble, the way
            // `Codec::Lzip` knows lzip's header. A method nothing here decodes
            // stays bytes, which is the honest answer for it.
            (
                "data",
                T::switch(
                    E::field("compression"),
                    vec![
                        (0, T::decoded(E::field("data_size"), Codec::Stored, super::decoded_text())),
                        (8, T::decoded(E::field("data_size"), Codec::Deflate, super::decoded_text())),
                        (12, T::decoded(E::field("data_size"), Codec::Bzip2, super::decoded_text())),
                        (93, T::decoded(E::field("data_size"), Codec::Zstd, super::decoded_text())),
                        (95, T::decoded(E::field("data_size"), Codec::Xz, super::decoded_text())),
                    ],
                    T::bytes(E::field("data_size")),
                ),
            ),
        ],
    )
    // The sum is of the file, so it is of what the run unpacks to, which for a
    // stored entry is the run itself. A method nothing here unpacks leaves
    // `data` as plain bytes, and that is the template saying the check cannot
    // be made rather than an omission.
    //
    // Two entries are skipped. An encrypted one has twelve bytes of header in
    // front of the data and the sum is of the plaintext, which is not in the
    // file. One whose sizes were written after the fact carries zeroes here and
    // in both size fields, and a sum of no bytes against a stored zero would
    // read as a pass: the honest answer for those is nothing at all.
    .field_check(
        "crc32",
        Check::of(Checksum::Crc32, Covers::Unpacked { name: Named::here("data"), len: Some(E::field("unpacked_size")) })
            .only_when(E::lit(1).sub(flag_bit(0)).mul(E::lit(1).sub(flag_bit(3)))),
    )
    // The MS-DOS pair, and both fields carry both names so that either one
    // answers the whole moment: a packed date is a day with no time of day in
    // it. Local, since MS-DOS had no zone to record and a ZIP does not add one.
    // An archiver may also write a real timestamp in an extra field, tag 0x5455
    // or 0x000a, which nothing here reads yet.
    .field_times(&["modified_time", "modified_date"], Time::dos_halves("modified_date", "modified_time"))
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
    // The central directory keeps its own copy of the entry's stamp, in the
    // same packed pair. See the local header above.
    .field_times(&["modified_time", "modified_date"], Time::dos_halves("modified_date", "modified_time"))
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
    // Both sides are the same record at two widths, and the check is of the
    // entry rather than of the width, so both carry it.
    let sizes = |name: &str, w: T| {
        T::structure(
            name,
            vec![("crc32", T::u32(Little)), ("compressed_size", w.clone()), ("uncompressed_size", w)],
        )
        .field_check("crc32", streamed_crc())
    };
    T::switch(
        wide,
        vec![(1, sizes("Zip64DataDescriptor", T::u64(Little)))],
        sizes("DataDescriptor", T::u32(Little)),
    )
}

/// The sum a streamed entry could not write in its header, taken over the file
/// the entry before this record holds.
///
/// The one check here that reaches backwards. Everything else a template can
/// say looks at what it sits in or at what that sits in, and a descriptor sits
/// in neither: it is a record of its own, written after the data, because the
/// writer was piping bytes and did not know the number until they had gone
/// past. `Named::earlier` searches back through the records the way
/// [`Expr::Sibling`] does, and the local entry is the nearest one with a
/// `body.data` in it.
///
/// This is what the local header's own `crc32` gives up when bit 3 is set: it
/// holds a zero there, and a sum of an entry against a stored zero would have
/// shown a reader a green tick over nothing.
fn streamed_crc() -> Check {
    Check::of(
        Checksum::Crc32,
        Covers::Unpacked {
            name: Named::earlier(&["body", "data"]),
            len: Some(E::field("uncompressed_size")),
        },
    )
    // An encrypted entry's plaintext is not in the archive, so the sum is over
    // bytes nothing here can produce. The flag is the entry's, not this
    // record's, which is why it is read the same way the data is.
    .only_when(E::lit(1).sub(E::sibling(&["body", "flags"]).bit(0)))
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

    /// The check that reaches backwards: a streamed entry's sum is in the
    /// record *after* the data, and it is over the entry before it.
    ///
    /// The local header of such an entry writes a zero where its sum would go,
    /// and the panel used to show a green "Valid" over nothing for it, because
    /// a sum of no bytes against a stored zero agrees. The descriptor is where
    /// the number actually is.
    #[test]
    fn a_streamed_entry_is_checked_by_the_descriptor_after_it() {
        let data = b"seven by";
        let mut v = entry(b"a.txt", 8, 0, 0, &[], data);
        v.extend_from_slice(b"PK\x07\x08");
        v.extend_from_slice(&crate::checksum::crc32(data).to_le_bytes());
        v.extend_from_slice(&(data.len() as u32).to_le_bytes());
        v.extend_from_slice(&(data.len() as u32).to_le_bytes());
        v.extend_from_slice(&end_record());
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(zip());

        // The local header's own crc32 still checks nothing: the file did not
        // know the number when it wrote that field.
        assert_eq!(e.check_of(&d, &[0, 0, 1, 6]).unwrap(), None);

        // The descriptor's does, and it covers the entry's data, which is a
        // record back and two levels in.
        let at = [0, 1, 1, 0];
        let info = e.check_of(&d, &at).unwrap().expect("the descriptor checks the entry before it");
        assert_eq!(info.algorithm, "crc32");
        assert_eq!(info.covered_bytes, data.len() as u64);
        assert!(e.run_check(&d, &at).unwrap().expect("and the sum can be taken").ok);
    }

    /// The same archive with one byte of the data changed. Without this the
    /// test above would pass just as well over a check that resolved to
    /// nothing and answered about no bytes at all.
    #[test]
    fn a_streamed_entry_whose_data_changed_fails_its_descriptor() {
        let mut v = entry(b"a.txt", 8, 0, 0, &[], b"seven by");
        let at = v.len() - 1;
        v[at] = b'e';
        v.extend_from_slice(b"PK\x07\x08");
        v.extend_from_slice(&crate::checksum::crc32(b"seven by").to_le_bytes());
        v.extend_from_slice(&8u32.to_le_bytes());
        v.extend_from_slice(&8u32.to_le_bytes());
        v.extend_from_slice(&end_record());
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(zip());
        let v = e.run_check(&d, &[0, 1, 1, 0]).unwrap().expect("the check applies");
        assert!(!v.ok, "computed {}, stored {}", v.computed, v.stored);
    }

    /// Method 12 is a whole bzip2 stream where the deflate would be. The
    /// entry's CRC-32 is of the file, so declaring the run decoded is what
    /// makes the check of a bzip2 entry possible at all: over the packed
    /// bytes it would match nothing.
    #[test]
    fn a_bzip2_entry_opens_and_its_crc_is_of_what_came_out() {
        let text = b"a bzip2 entry in a zip, which is a method few writers pick\n";
        let packed = {
            use std::io::Write;
            let mut e = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::new(9));
            e.write_all(text).expect("writes");
            e.finish().expect("finishes")
        };
        let mut v = entry(b"a.txt", 0, packed.len() as u32, text.len() as u32, &[], &packed);
        // The method, which `entry` leaves at zero.
        v[8..10].copy_from_slice(&12u16.to_le_bytes());
        v[14..18].copy_from_slice(&crate::checksum::crc32(text).to_le_bytes());
        v.extend_from_slice(&end_record());
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(zip());
        let data = e.node(&d, &[0, 0, 1, 14]).unwrap();
        assert_eq!(data.type_name, "bzip2");
        assert!(data.decoded && data.refused.is_none(), "the stream opened: {data:?}");
        let crc = e.child_named(&d, &[0, 0, 1], "crc32").unwrap().expect("a crc32 field");
        let v = e.run_check(&d, &crc).unwrap().expect("the sum is of the file, so of what unpacks");
        assert!(v.ok, "computed {}, stored {}", v.computed, v.stored);
    }

    /// The three methods that are a whole stream of another format written
    /// where the deflate would be. Each opens, and each entry's CRC-32 is then
    /// a check of the file rather than of nothing: over the packed bytes it
    /// would match nothing at all.
    #[test]
    fn an_entry_packed_with_another_format_opens_and_its_crc_is_of_the_file() {
        let text = b"a zip entry packed with something other than deflate\n".repeat(4);
        for (method, packed) in [
            (12u16, {
                use std::io::Write;
                let mut e = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::new(9));
                e.write_all(&text).expect("writes");
                e.finish().expect("finishes")
            }),
            (95, {
                let mut out = Vec::new();
                lzma_rs::xz_compress(&mut &text[..], &mut out).expect("packs");
                out
            }),
        ] {
            let mut v = entry(b"a.txt", 0, packed.len() as u32, text.len() as u32, &[], &packed);
            v[8..10].copy_from_slice(&method.to_le_bytes());
            v[14..18].copy_from_slice(&crate::checksum::crc32(&text).to_le_bytes());
            v.extend_from_slice(&end_record());
            let d = Document::new(MemSource(v));
            let mut e = Evaluator::new(zip());
            let data = e.node(&d, &[0, 0, 1, 14]).unwrap();
            assert!(data.decoded && data.refused.is_none(), "method {method}: {data:?}");
            let crc = e.child_named(&d, &[0, 0, 1], "crc32").unwrap().expect("a crc32 field");
            let v = e.run_check(&d, &crc).unwrap().unwrap_or_else(|| panic!("method {method} checks nothing"));
            assert!(v.ok, "method {method}: computed {}, stored {}", v.computed, v.stored);
        }
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
