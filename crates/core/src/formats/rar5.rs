//! RAR 5: eight bytes of signature and then a chain of blocks, each of which
//! is a checksum, a size, and a header whose fields depend on what kind of
//! block it is.
//!
//! Every number in a header is a variable-length integer: seven bits to a
//! byte, least significant group first, with the top bit set on every byte but
//! the last. So no field in a header is at a fixed offset, and the size that
//! comes before them is what lets a reader step over a block it does not
//! understand.
//!
//! Two flag bits are what the block layout turns on: one says an extra area
//! follows the fields, and the other says a data area follows the header
//! altogether. A file block has both, and the data area is the file.
//!
//! The fields particular to each kind of block are read too: a file block's
//! name, its unpacked size, when it was written, how it was packed and what
//! its bytes checksum to. They sit behind a switch on the block type, because
//! the same run of bytes after the common header means something different in
//! each kind and there is no length to step over them by.
//!
//! The extra area is read as the chain of records it is, far enough to say
//! what each one is and how long: one of them says the entry is encrypted, and
//! an entry nothing can read without a password must not be handed to a
//! decompressor that would make noise of it and then blame the file.
//!
//! What is not read here: the insides of those records, which carry encryption
//! parameters, hard links, high-precision times and the rest; and the
//! encryption that may cover the headers themselves, which turns everything
//! after it into bytes nothing can read without a password.
//!
//! ## What opens, and what does not
//!
//! A stored entry is the file written in verbatim and has always opened. A
//! packed one now opens too, by [`Codec::Rar5`], but only when the entry stands
//! on its own. Four things mean it does not, and each of them leaves the run as
//! bytes rather than decoding it wrongly:
//!
//! - **Solid.** The entry's first match may reach back into the entry before
//!   it. Nothing in `Ty::Decoded` can say "this run's decode continues the
//!   previous run's window", so a solid entry has nowhere to get its history
//!   from. Note this is the *entry's* solid bit and not the archive's: the
//!   first entry of a solid archive starts from an empty window and opens.
//! - **Encrypted.** The data area is ciphertext, and a decoder handed it would
//!   either refuse or, with luck against it, produce the declared number of
//!   bytes and fail a checksum on a file that is perfectly sound.
//! - **Split across volumes.** Half the entry is in another file.
//! - **An unpacked size the archiver did not know**, which is written as a
//!   number that means nothing. The decoder is told where the file stops by
//!   that number and by nothing else, RAR having no end-of-stream marker.
//!
//! A compression version other than 0 is turned away for the same kind of
//! reason: RAR 7 packs its dictionary differently, and a reader of version 0
//! would be reading a different format under the same name.

use crate::codec::Codec;
use crate::template::{Check, Checksum, Covers, Named, Encoding, Endian::Little, Expr as E, Packing, StrLen, Template, Time, Ty as T, Until};

/// What one of these starts with. RAR 4 has the same first six bytes and one
/// less at the end, so the eighth byte is what tells the two apart.
pub const MAGIC: &[u8] = b"Rar!\x1a\x07\x01\x00";

/// The block kinds. The last one ends the archive, which is what stops the
/// chain.
const TYPES: &[(i128, &str)] =
    &[(1, "main archive"), (2, "file"), (3, "service"), (4, "archive encryption"), (5, "end of archive")];

/// The kinds this reads the fields of, and the one the chain stops at.
const MAIN_ARCHIVE: i128 = 1;
const FILE: i128 = 2;
const SERVICE: i128 = 3;
const ARCHIVE_ENCRYPTION: i128 = 4;
const END_OF_ARCHIVE: i128 = 5;

/// What an archive encryption block's flags say. The one bit says whether the
/// twelve bytes that let a reader tell a wrong password from a damaged file
/// are there.
const CRYPT_FLAGS: &[(u32, &str)] = &[(0, "password check present")];

/// What a main archive header's flags say about the archive.
const ARCHIVE_FLAGS: &[(u32, &str)] =
    &[(0, "volume"), (1, "volume number present"), (2, "solid"), (3, "recovery record"), (4, "locked")];

/// A file block's own flags. "unpacked size unknown" is the one that changes
/// how another field reads: the size is written anyway and means nothing.
const FILE_FLAGS: &[(u32, &str)] =
    &[(0, "directory"), (1, "time present"), (2, "checksum present"), (3, "unpacked size unknown")];

const END_FLAGS: &[(u32, &str)] = &[(0, "more volumes follow")];

/// How hard the packer worked. The same six RAR 4 has, under the same names,
/// so the two templates read alike.
const METHOD: &[(i128, &str)] =
    &[(0, "store"), (1, "fastest"), (2, "fast"), (3, "normal"), (4, "good"), (5, "best")];

/// The window the unpacker needs, as a power of two from 128k. A file packed
/// with a bigger dictionary cannot be read by an unpacker that will not give
/// it the memory, which is what makes this worth showing.
const DICTIONARY: &[(i128, &str)] = &[
    (0, "128k"),
    (1, "256k"),
    (2, "512k"),
    (3, "1M"),
    (4, "2M"),
    (5, "4M"),
    (6, "8M"),
    (7, "16M"),
    (8, "32M"),
    (9, "64M"),
    (10, "128M"),
    (11, "256M"),
    (12, "512M"),
    (13, "1G"),
    (14, "2G"),
    (15, "4G"),
];

/// Which system wrote the file, which is what its attribute word means.
const HOST_OS: &[(i128, &str)] = &[(0, "windows"), (1, "unix")];

/// The extra area record that says an entry is encrypted, and the only one of
/// them this asks about.
///
/// The numbers are not one namespace: in a file or service header 1 is the
/// encryption parameters, and in the main archive header the same 1 is the
/// quick-open locator. So the question below is asked only of the kinds of
/// block where it means encryption.
const EX_CRYPT: i128 = 1;

const FLAGS: &[(u32, &str)] = &[
    (0, "extra area"),
    (1, "data area"),
    (2, "skip if unknown"),
    (3, "data continues from previous volume"),
    (4, "data continues in next volume"),
    (5, "depends on previous block"),
    (6, "child block"),
    (7, "inherited"),
];

pub fn rar5() -> Template {
    Template::new(
        "rar5",
        T::structure(
            "Rar5Archive",
            vec![
                ("magic", T::magic(MAGIC)),
                (
                    "blocks",
                    T::repeat(block(), Until::FieldValue { field: "chain_ends".into(), value: 1 }),
                ),
                // Everything after a block that says the rest is encrypted.
                //
                // Without this the reading simply stopped: the ciphertext does
                // not read as a block, the chain ran out, and six hundred of a
                // seven hundred byte file were in no row at all. A run that
                // says what it is beats a file that quietly ends early.
                ("encrypted_headers", T::bytes(E::Remaining)),
            ],
        ),
    )
}

/// One block: a checksum over everything from the size field to the end of
/// the header, the header itself in the window that size measures out, and
/// then the data area when the flags said there is one.
fn block() -> T {
    T::structure(
        "Rar5Block",
        vec![
            ("header_crc32", T::u32(Little)),
            // How long the header is, counted from the byte after this field.
            ("header_size", T::leb_u()),
            ("header", T::sized(E::field("header_size"), header())),
            // Which kind this was, taken back out of the header so that the
            // chain can stop at the block that ends the archive.
            ("block_type", T::computed(E::within(&["header", "header_type"]))),
            // And whether it is the last one anything can read. Two blocks
            // end the chain and they end it for different reasons: after the
            // end-of-archive block there is nothing, and after the archive
            // encryption block there is ciphertext. A repeat can watch one
            // field, so the two are added up into one.
            (
                "chain_ends",
                T::computed(
                    E::field("block_type")
                        .equals(E::lit(END_OF_ARCHIVE))
                        .or(E::field("block_type").equals(E::lit(ARCHIVE_ENCRYPTION))),
                ),
            ),
            // The file, or whatever else the block put outside its header.
            //
            // Stored is the file written in verbatim, so those bytes are
            // already a document and open as one: a RAR of stored files used
            // to offer nothing to open anywhere, not in the listing and not as
            // a tab, the way an archive of stored ZIP entries once did. Every
            // other method needs a RAR decompressor, which there is not one of
            // here, so the run stays bytes and says so.
            (
                "data",
                T::switch(
                    E::field("block_type"),
                    vec![(FILE, file_data()), (SERVICE, file_data())],
                    // Every other kind's data area is not a file and has no
                    // method to ask about: an end-of-archive block has no
                    // `method` field at all, and asking for one would fail the
                    // block rather than answer about it.
                    T::bytes(E::within(&["header", "data_size"])),
                ),
            ),
        ],
    )
    .counted_as("block")
    // Everything from the size field to the end of the header, which is what
    // the module comment above says and what every RAR 5 reader does. The run
    // starts where the sum ends and takes in the size field itself, so a
    // header whose length was tampered with fails here rather than reading on
    // into the next block.
    .field_check(
        "header_crc32",
        Check::of(
            Checksum::Crc32,
            Covers::Run {
                at: E::size_of("header_crc32"),
                len: E::size_of("header_size").add(E::field("header_size")),
            },
        ),
    )
}

fn header() -> T {
    T::structure(
        "Rar5Header",
        vec![
            ("header_type", T::enumeration("Rar5BlockType", T::leb_u(), TYPES)),
            ("header_flags", T::flags("Rar5HeaderFlags", T::leb_u(), FLAGS)),
            ("extra_area_size", T::present_if(E::field("header_flags").bit(0), T::leb_u())),
            ("data_size", T::present_if(E::field("header_flags").bit(1), T::leb_u())),
            // What this kind of block carries. A file and a service block are
            // laid out the same way and differ only in what the name means, so
            // they share a case; anything else keeps its bytes, which is the
            // honest answer for a kind nothing here reads.
            (
                "fields",
                T::switch(
                    E::field("header_type"),
                    vec![
                        (MAIN_ARCHIVE, main_fields()),
                        (FILE, file_fields()),
                        (SERVICE, file_fields()),
                        (ARCHIVE_ENCRYPTION, crypt_fields()),
                        (END_OF_ARCHIVE, end_fields()),
                    ],
                    T::bytes(E::Remaining),
                ),
            ),
            // The records a newer version of the format added without moving
            // anything: a chain of size, kind, and a body this does not go
            // into. See the module doc for why the kinds are not named here.
            (
                "extra_area",
                T::sized(E::field("extra_area_size").or(E::lit(0)), T::repeat(extra_record(), Until::End)),
            ),
            // Whether a password stands between this block's data area and
            // anybody reading it. Asked of the extra area, which is the only
            // place RAR 5 says so, and asked only where record 1 means
            // encryption rather than the quick-open locator.
            (
                "encrypted",
                T::computed(
                    E::field("header_type")
                        .equals(E::lit(FILE))
                        .or(E::field("header_type").equals(E::lit(SERVICE)))
                        .mul(E::tagged("extra_area", &["record_type"], EX_CRYPT, &["record_type"])),
                ),
            ),
        ],
    )
}

/// One record of an extra area: how long it is, what kind it is, and a body
/// left as bytes.
///
/// The size counts from after itself, so what is left for the body is that
/// less the width of the kind. A record claiming less than its own kind field
/// leaves nothing, rather than a negative length that would read backwards.
fn extra_record() -> T {
    T::structure(
        "Rar5Extra",
        vec![
            ("record_size", T::leb_u()),
            ("record_type", T::leb_u()),
            (
                "record_data",
                T::bytes(E::field("record_size").sub(E::size_of("record_type")).at_least(E::lit(0))),
            ),
        ],
    )
    .counted_as("record")
}

/// What a main archive header says about the archive as a whole.
fn main_fields() -> T {
    T::structure(
        "Rar5Main",
        vec![
            ("archive_flags", T::flags("Rar5ArchiveFlags", T::leb_u(), ARCHIVE_FLAGS)),
            // Which volume of a set this is, written only when the flags say
            // there is a set to be part of.
            ("volume_number", T::present_if(E::field("archive_flags").bit(1), T::leb_u())),
        ],
    )
}

/// A file, or a service block, which has the same shape and whose name says
/// what service it is rather than what file: `CMT` for the archive comment,
/// `QO` for the quick open index.
///
/// Three of these are written only when a flag says so, and each one that is
/// missing moves everything after it, which is why they are `present_if` and
/// not a fixed run.
fn file_fields() -> T {
    T::structure(
        "Rar5File",
        vec![
            ("file_flags", T::flags("Rar5FileFlags", T::leb_u(), FILE_FLAGS)),
            // What the file comes to once unpacked. A stream whose length the
            // archiver did not know when it wrote the header says so in the
            // flags and writes nothing useful here.
            ("unpacked_size", T::leb_u()),
            ("attributes", T::leb_u()),
            ("mtime", T::present_if(E::field("file_flags").bit(1), T::u32(Little))),
            ("data_crc32", T::present_if(E::field("file_flags").bit(2), T::u32(Little))),
            // One number holding five: the format version, whether the file
            // continues a solid block, the method, and how big a window the
            // unpacker needs. Split out below rather than left as a number
            // nobody can read.
            ("compression_info", T::leb_u()),
            ("compression_version", T::computed(E::bit_field(E::field("compression_info"), 5, 6))),
            ("solid", T::computed(E::field("compression_info").bit(6))),
            ("method", T::enumeration("Rar5Method", T::computed(E::bit_field(E::field("compression_info"), 9, 3)), METHOD)),
            ("dictionary", T::enumeration("Rar5Dictionary", T::computed(E::bit_field(E::field("compression_info"), 13, 4)), DICTIONARY)),
            ("host_os", T::enumeration("Rar5HostOs", T::leb_u(), HOST_OS)),
            ("name_length", T::leb_u()),
            // UTF-8, and the one field of the header a reader is looking for.
            ("name", T::text(StrLen::Fixed(E::field("name_length")), Encoding::Utf8)),
        ],
    )
    .counted_as("file")
    // Seconds from 1970, and only there at all when the flag says so. RAR 5
    // can also write this as a FILETIME in an extra-area record, which nothing
    // here reads: see the note at the top of the file.
    .field_time("mtime", Time::unix())
    // The file, which is what the run unpacks to rather than the packed bytes
    // themselves. For a stored entry the two are the same run and a reader can
    // be sent to it; for a packed one the summed bytes are nowhere in the file,
    // which is what [`Covers::Unpacked`] is for.
    //
    // What keeps this honest is not the guard but `Unpacked` itself: it answers
    // nothing at all when the run it names is not a stream, and `file_data`
    // above leaves the run as plain bytes for every entry this cannot open. So
    // the list of reasons an entry does not open is written once, where the
    // decision is made, rather than twice and drifting apart. An entry that
    // opens but will not decode is a refusal with a reason, never a mismatch.
    //
    // The flag is still asked: without it the field is not there at all, and a
    // sum of no bytes against a zero read out of nothing would come back as a
    // file that checks out.
    //
    // `unpacked_size` is only so an interface can decide whether the work is
    // worth doing; the sum is over whatever the decoder actually produced.
    .field_check(
        "data_crc32",
        Check::of(
            Checksum::Crc32,
            Covers::Unpacked { name: Named::here("data"), len: Some(E::field("unpacked_size")) },
        )
        .only_when(E::field("file_flags").bit(2)),
    )
}

/// A field of the file header, reached from the data area outside it.
fn f(name: &str) -> E {
    E::within(&["header", "fields", name])
}

/// Whether the bytes in the data area are this entry's bytes at all.
///
/// Encryption and a volume split are the two things that make them something
/// else, and neither has anything to do with how the entry was packed: a
/// stored entry that is encrypted is ciphertext sitting where a reader would
/// otherwise be told the file is, and summing it would call a sound archive
/// broken.
fn readable() -> E {
    let flag = |bit: u32| E::lit(1).sub(E::within(&["header", "header_flags"]).bit(bit));
    E::lit(1)
        .sub(E::within(&["header", "encrypted"]))
        .mul(flag(3))
        .mul(flag(4))
}

/// Whether a packed entry is one this can unpack on its own: a method there is
/// a decoder for, the compression format that decoder reads, a window that
/// starts empty, and a length to stop at.
///
/// See the module doc for what each of these turns away and why. The window is
/// the sharpest of them: a solid entry's history is the entry before it, and
/// there is nowhere here to have kept it.
fn packable() -> E {
    E::lit(0)
        .less_than(f("method"))
        .mul(f("method").less_than(E::lit(6)))
        .mul(f("compression_version").equals(E::lit(0)))
        .mul(E::lit(1).sub(f("solid")))
        .mul(E::lit(1).sub(f("file_flags").bit(3)))
}

/// How the data area opens: not at all, as the file written in verbatim, or by
/// the RAR 5 decompressor. `Or` answers the first of its sides that is not
/// zero, so the two cases cannot both be taken.
fn how_it_opens() -> E {
    let stored = f("method").equals(E::lit(0));
    readable().mul(stored.or(packable().mul(E::lit(2))))
}

/// The two numbers RAR 5's stream does not carry and its header does.
fn packing() -> Packing {
    Packing::Rar5 { dictionary: f("dictionary"), unpacked: f("unpacked_size") }
}

/// A file block's data area: the file itself when nothing packed it, the file
/// unpacked when something did, and the bytes as they are when this cannot say
/// they are the file.
fn file_data() -> T {
    let size = || E::within(&["header", "data_size"]);
    T::switch(
        how_it_opens(),
        vec![
            (1, T::decoded(size(), Codec::Stored, super::decoded_text())),
            (2, T::decoded_as(size(), packing(), super::decoded_text())),
        ],
        T::bytes(size()),
    )
}

/// The block that ends the archive, and says whether another volume follows.
fn end_fields() -> T {
    T::structure("Rar5End", vec![("end_flags", T::flags("Rar5EndFlags", T::leb_u(), END_FLAGS))])
}

/// The block that says every header after it is encrypted.
///
/// A RAR made with `-hp` puts one of these first, and from the end of it the
/// file is ciphertext: not the file data alone, which `-p` encrypts, but the
/// names, the sizes and the block structure itself. So this is the last block
/// anything can read, and the chain stops on it.
///
/// Nothing here decrypts. What it does is say so, which is the difference
/// between a reader seeing an archive whose contents are locked and a reader
/// seeing a file that stopped making sense. The salt and the password check
/// are shown because they are the only part of the scheme that is in the
/// clear, and because a reader comparing two archives can see from them
/// whether the same password was used.
fn crypt_fields() -> T {
    T::structure(
        "Rar5ArchiveEncryption",
        vec![
            ("encryption_version", T::leb_u()),
            ("crypt_flags", T::flags("Rar5CryptFlags", T::leb_u(), CRYPT_FLAGS)),
            // How many rounds the key derivation ran, as a power of two: 15
            // here means 32,768. Written as the exponent because the count is
            // always a power of two and one byte then says it. Not an
            // algorithm number, which is what it looks like sitting next to
            // `encryption_version` and is not.
            ("kdf_count", T::u8()),
            ("kdf_rounds", T::computed(E::lit(1).shl(E::field("kdf_count")))),
            ("salt", T::bytes(E::lit(16))),
            // Twelve bytes that tell a wrong password from a damaged archive.
            // Without them a reader has no way to know which it is looking at.
            ("password_check", T::present_if(E::field("crypt_flags").bit(0), T::bytes(E::lit(12)))),
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

    fn block_bytes(block_type: u8, flags: u8, rest: &[u8]) -> Vec<u8> {
        let mut header = vec![block_type, flags];
        header.extend_from_slice(rest);
        let mut v = 0u32.to_le_bytes().to_vec();
        v.push(header.len() as u8);
        v.extend_from_slice(&header);
        v
    }

    /// An archive of one file block between the main header and the end: the
    /// chain stops at the end block, and the file's bytes are outside the
    /// header the flags placed them after.
    fn archive() -> Vec<u8> {
        stored_archive(0)
    }

    /// A variable-length integer, seven bits to a byte, least significant
    /// group first, with the top bit set on every byte but the last.
    fn vint(mut n: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let byte = (n & 0x7f) as u8;
            n >>= 7;
            if n == 0 {
                out.push(byte);
                return out;
            }
            out.push(byte | 0x80);
        }
    }

    /// A file block for `name`, with the flags saying there is a checksum and
    /// a data area, packed by the normal method into a 128k window.
    fn file_block(name: &str, packed: u64, unpacked: u64) -> Vec<u8> {
        let mut h = vec![2u8]; // header type: file
        h.extend(vint(0x02)); // header flags: data area follows
        h.extend(vint(packed)); // data size
        h.extend(vint(0x04)); // file flags: checksum present
        h.extend(vint(unpacked));
        h.extend(vint(0x20)); // attributes
        h.extend_from_slice(&0xdeadbeefu32.to_le_bytes()); // data crc32
        // Version 0, not solid, method 3, dictionary 0.
        h.extend(vint(3 << 7));
        h.extend(vint(0)); // host os: windows
        h.extend(vint(name.len() as u64));
        h.extend_from_slice(name.as_bytes());
        let mut v = 0u32.to_le_bytes().to_vec();
        v.extend(vint(h.len() as u64));
        v.extend_from_slice(&h);
        v
    }

    /// A block with the CRC-32 the format asks for in front of it: the sum is
    /// over the size and the header together, which is what `header_crc32`
    /// covers and what every RAR 5 reader checks.
    fn sealed(header: &[u8]) -> Vec<u8> {
        let mut sized = vint(header.len() as u64);
        sized.extend_from_slice(header);
        let mut v = crate::checksum::crc32(&sized).to_le_bytes().to_vec();
        v.extend_from_slice(&sized);
        v
    }

    /// A file block for `name` holding `data`, with both sums correct: the
    /// header's, and the file's. `info` is the packed word saying how it was
    /// compressed, so a test can set the solid bit as easily as the method.
    fn file_block_with(name: &str, data: &[u8], info: u64, header_flags: u64) -> Vec<u8> {
        file_block_extra(name, data, info, header_flags, &[])
    }

    /// The same, with `extra` written as the block's extra area. A record is
    /// its size, its kind, and a body; `\x01\x01` is the shortest encryption
    /// record there is, being a size of one and a kind of one with nothing
    /// after it.
    fn file_block_extra(name: &str, data: &[u8], info: u64, header_flags: u64, extra: &[u8]) -> Vec<u8> {
        let mut h = vec![2u8];
        let has_extra = if extra.is_empty() { 0 } else { 0x01 };
        h.extend(vint(0x02 | has_extra | header_flags)); // header flags
        if has_extra != 0 {
            h.extend(vint(extra.len() as u64));
        }
        h.extend(vint(data.len() as u64));
        h.extend(vint(0x04)); // file flags: checksum present
        h.extend(vint(data.len() as u64));
        h.extend(vint(0x20)); // attributes
        h.extend_from_slice(&crate::checksum::crc32(data).to_le_bytes());
        h.extend(vint(info));
        h.extend(vint(0)); // host os
        h.extend(vint(name.len() as u64));
        h.extend_from_slice(name.as_bytes());
        h.extend_from_slice(extra);
        sealed(&h)
    }

    /// The same, written in verbatim by `method`.
    fn stored_file(name: &str, data: &[u8], method: u64) -> Vec<u8> {
        file_block_with(name, data, method << 7, 0)
    }

    /// An archive of one stored file, with every sum in it correct.
    fn stored_archive(method: u64) -> Vec<u8> {
        let mut v = MAGIC.to_vec();
        v.extend_from_slice(&sealed(b"\x01\x00"));
        v.extend_from_slice(&stored_file("notes.txt", b"file data", method));
        v.extend_from_slice(b"file data");
        v.extend_from_slice(&sealed(b"\x05\x00"));
        v
    }

    /// Both sums a RAR 5 writes, over an archive built to have them right.
    ///
    /// The header sum is the format's own arithmetic and holds for every block
    /// of every archive. The file sum only means anything when the method is
    /// store, since nothing here unpacks RAR, and the test below is the other
    /// half of that.
    #[test]
    fn a_stored_file_checks_out_header_and_all() {
        let d = Document::new(MemSource(stored_archive(0)));
        let mut e = Evaluator::new(rar5());
        for block in 0..3 {
            let v = e.run_check(&d, &[1, block, 0]).unwrap().expect("every block seals its header");
            assert!(v.ok, "block {block}: computed {} stored {}", v.computed, v.stored);
        }
        let crc = e.child_named(&d, &[1, 1, 2, 4], "data_crc32").unwrap().expect("data_crc32");
        let info = e.check_of(&d, &crc).unwrap().expect("a stored file is checkable");
        assert_eq!(info.algorithm, "crc32");
        assert_eq!(info.over, Some((42, 9)), "the data area, which is the file");
        assert!(e.run_check(&d, &crc).unwrap().unwrap().ok);

        // And those bytes are a document of their own, so a stored file in a
        // RAR has somewhere to be opened: in the listing, on a chip, as a tab.
        // A packed one has nothing to open, since there is no RAR
        // decompressor here, and the test below says so.
        let at = e.child_named(&d, &[1, 1], "data").unwrap().expect("data");
        let data = e.node(&d, &at).unwrap();
        assert_eq!(data.name, "data");
        assert!(data.decoded && data.refused.is_none(), "a stored file opens: {data:?}");
    }

    /// An archive of one file block with `info` as its compression word and
    /// `header_flags` set on top of the data-area bit.
    fn archive_of(info: u64, header_flags: u64) -> Vec<u8> {
        let mut v = MAGIC.to_vec();
        v.extend_from_slice(&sealed(b"\x01\x00"));
        v.extend_from_slice(&file_block_with("notes.txt", b"file data", info, header_flags));
        v.extend_from_slice(b"file data");
        v.extend_from_slice(&sealed(b"\x05\x00"));
        v
    }

    /// The data area of that archive, and whether its checksum is offered.
    fn crc_of(d: &Document<MemSource>, e: &mut Evaluator) -> Vec<usize> {
        e.child_named(d, &[1, 1, 2, 4], "data_crc32").unwrap().expect("data_crc32")
    }

    #[test]
    fn a_packed_entry_that_will_not_decode_refuses_rather_than_reporting_a_mismatch() {
        // Nine bytes of "file data" declared as packed by method 3. There is a
        // decoder for that method now, so the check is offered; the bytes are
        // not a RAR stream and it will not open them. What must not come back
        // is a sum that disagrees, which would put a damaged-file verdict in
        // front of a reader whose archive is only unreadable by this.
        let d = Document::new(MemSource(stored_archive(3)));
        let mut e = Evaluator::new(rar5());
        let crc = crc_of(&d, &mut e);
        assert!(e.check_of(&d, &crc).unwrap().is_some(), "a packed method has a decoder");
        assert!(e.run_check(&d, &crc).is_err(), "a stream that will not open is a refusal, not a mismatch");
        // The header sum is not conditional and is still made.
        assert!(e.run_check(&d, &[1, 1, 0]).unwrap().unwrap().ok);
    }

    /// The four things that leave a packed entry as bytes, each on its own.
    ///
    /// Every one of them has a data area a decompressor would take, and every
    /// one of them would then be a checksum failing over an archive that is
    /// perfectly sound. See the module doc.
    #[test]
    fn an_entry_this_cannot_read_on_its_own_offers_no_check_at_all() {
        // Method 3, and then one thing wrong with it each time: the solid bit,
        // a compression version this does not read, an unpacked size the
        // archiver never knew, and a data area continued in the next volume.
        let packed = 3u64 << 7;
        for (what, info, flags) in [
            ("solid", packed | 1 << 6, 0),
            ("version 1", packed | 1, 0),
            ("continued in the next volume", packed, 1 << 4),
            ("continued from the previous volume", packed, 1 << 3),
        ] {
            let d = Document::new(MemSource(archive_of(info, flags)));
            let mut e = Evaluator::new(rar5());
            let crc = crc_of(&d, &mut e);
            assert_eq!(e.check_of(&d, &crc).unwrap(), None, "{what}: this must offer no check");
            let at = e.child_named(&d, &[1, 1], "data").unwrap().expect("data");
        let data = e.node(&d, &at).unwrap();
            assert!(!data.decoded, "{what}: this must not open");
        }
    }

    /// An entry behind a password, whichever way it was packed.
    ///
    /// The stored half is the one worth having: those bytes used to be handed
    /// to a reader as the file and summed as the file, and since they are
    /// ciphertext the sum disagreed and the archive was reported damaged. A
    /// An archive made with `-hp` encrypts the block structure itself, so the
    /// archive encryption block is the last thing anything can read.
    ///
    /// The chain stops there and the rest is one run that says what it is.
    /// Before this the reading just ran out: the ciphertext does not parse as
    /// a block, the repeat ended, and most of the file was in no row at all,
    /// which looks exactly like a file that stopped making sense.
    #[test]
    fn an_archive_whose_headers_are_encrypted_stops_and_says_so() {
        // The block: a crc, a size, type 4, no flags, then the encryption
        // fields. The ciphertext after it is whatever bytes; nothing reads it.
        let mut header = vec![4u8, 0];
        header.extend_from_slice(&[0, 1, 15]);
        header.extend_from_slice(&[0xab; 16]);
        header.extend_from_slice(&[0xcd; 12]);
        let mut v = MAGIC.to_vec();
        v.extend_from_slice(&crate::checksum::crc32(&[&[header.len() as u8][..], &header].concat()).to_le_bytes());
        v.push(header.len() as u8);
        v.extend_from_slice(&header);
        v.extend_from_slice(&[0x99; 40]);
        let d = Document::new(MemSource(v.clone()));
        let mut e = Evaluator::new(rar5());

        // One block, and it is the crypt block.
        assert_eq!(e.node(&d, &[1]).unwrap().child_count, 1);
        let kind = e.child_named(&d, &[1, 0], "block_type").unwrap().expect("block_type");
        assert_eq!(e.node(&d, &kind).unwrap().value.as_int(), Some(ARCHIVE_ENCRYPTION));
        // Its own header still checks out: the block that says the rest is
        // locked is itself in the clear.
        let crc = e.child_named(&d, &[1, 0], "header_crc32").unwrap().expect("header_crc32");
        assert!(e.run_check(&d, &crc).unwrap().expect("a verdict").ok);
        // The rounds are read from the exponent, not taken for an algorithm.
        let fields = e.child_named(&d, &[1, 0, 2], "fields").unwrap().expect("fields");
        let rounds = e.child_named(&d, &fields, "kdf_rounds").unwrap().expect("kdf_rounds");
        assert_eq!(e.node(&d, &rounds).unwrap().value.as_int(), Some(1 << 15));
        // And every byte after the block is in a row of its own.
        let rest = e.node(&d, &[2]).unwrap();
        assert_eq!(rest.name, "encrypted_headers");
        assert_eq!(rest.size_bits, 40 * 8);
        assert_eq!(rest.offset_bits + rest.size_bits, v.len() as u64 * 8, "the file is accounted for");
    }

    /// real archive of exactly this shape is in the collection, with two
    /// plaintext stored entries beside two encrypted packed ones; this is the
    /// same thing small enough to read.
    #[test]
    fn an_encrypted_entry_offers_no_check_however_it_was_packed() {
        for (what, info) in [("stored", 0u64), ("packed", 3 << 7)] {
            let mut v = MAGIC.to_vec();
            v.extend_from_slice(&sealed(b"\x01\x00"));
            v.extend_from_slice(&file_block_extra("secret.txt", b"file data", info, 0, b"\x01\x01"));
            v.extend_from_slice(b"file data");
            v.extend_from_slice(&sealed(b"\x05\x00"));
            let d = Document::new(MemSource(v));
            let mut e = Evaluator::new(rar5());
            // The record really was read as an encryption record.
            let enc = e.child_named(&d, &[1, 1, 2], "encrypted").unwrap().expect("encrypted");
            assert_eq!(e.node(&d, &enc).unwrap().value.as_int(), Some(1), "{what}: the crypt record was not seen");
            let crc = crc_of(&d, &mut e);
            assert_eq!(e.check_of(&d, &crc).unwrap(), None, "{what}: ciphertext must not be summed");
            let at = e.child_named(&d, &[1, 1], "data").unwrap().expect("data");
            assert!(!e.node(&d, &at).unwrap().decoded, "{what}: this must not open");
        }
    }

    /// A method with no decoder behind it, which is every one this does not
    /// list. The format has six and there is a decoder for all six, so the
    /// case is reached only by a number no archiver writes.
    #[test]
    fn a_method_nothing_reads_stays_bytes() {
        let d = Document::new(MemSource(archive_of(6 << 7, 0)));
        let mut e = Evaluator::new(rar5());
        let crc = crc_of(&d, &mut e);
        assert_eq!(e.check_of(&d, &crc).unwrap(), None);
        let at = e.child_named(&d, &[1, 1], "data").unwrap().expect("data");
        assert!(!e.node(&d, &at).unwrap().decoded);
    }

    #[test]
    fn a_changed_header_byte_fails_the_block_sum() {
        let mut bytes = stored_archive(0);
        // The declared unpacked size, inside the header the sum covers.
        let at = bytes.iter().position(|&b| b == b'n').expect("the name is in there") - 12;
        bytes[at] ^= 0x40;
        let d = Document::new(MemSource(bytes));
        let mut e = Evaluator::new(rar5());
        let v = e.run_check(&d, &[1, 1, 0]).unwrap().expect("the block still seals its header");
        assert!(!v.ok, "a changed header byte is a changed header");
    }

    /// The name is what a reader opens an archive to find, and it was inside a
    /// run of bytes called `header_fields` that nothing read. So are the sizes,
    /// the checksum and how the file was packed.
    #[test]
    fn a_file_block_says_what_the_file_is_called_and_how_it_was_packed() {
        let mut v = MAGIC.to_vec();
        v.extend_from_slice(&block_bytes(1, 0, b"\x00"));
        v.extend_from_slice(&file_block("notes.txt", 9, 40));
        v.extend_from_slice(b"file data");
        v.extend_from_slice(&block_bytes(5, 0, b"\x00"));
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(rar5());
        // The file block's fields, under the switch that picked them.
        let name = e.child_named(&d, &[1, 1, 2, 4], "name").unwrap().expect("name");
        assert_eq!(e.node(&d, &name).unwrap().value, Value::Str("notes.txt".into()));
        let unpacked = e.child_named(&d, &[1, 1, 2, 4], "unpacked_size").unwrap().expect("unpacked_size");
        assert_eq!(e.node(&d, &unpacked).unwrap().value.as_int(), Some(40));
        let method = e.child_named(&d, &[1, 1, 2, 4], "method").unwrap().expect("method");
        assert_eq!(
            e.node(&d, &method).unwrap().value,
            Value::Enum { raw: 3, name: Some("normal".into()), hex: false }
        );
        let dict = e.child_named(&d, &[1, 1, 2, 4], "dictionary").unwrap().expect("dictionary");
        assert_eq!(e.node(&d, &dict).unwrap().value, Value::Enum { raw: 0, name: Some("128k".into()), hex: false });
        // And the file itself is still outside the header, where its size said.
        let data = e.child_named(&d, &[1, 1], "data").unwrap().expect("data");
        assert_eq!(e.node(&d, &data).unwrap().size_bits, 9 * 8);
    }

    #[test]
    fn the_chain_stops_at_the_block_that_ends_the_archive() {
        let d = Document::new(MemSource(archive()));
        let mut e = Evaluator::new(rar5());
        assert_eq!(e.node(&d, &[1]).unwrap().child_count, 3);
        assert_eq!(
            e.node(&d, &[1, 0, 2, 0]).unwrap().value,
            Value::Enum { raw: 1, name: Some("main archive".into()), hex: false }
        );
        // The file block: a data area of nine bytes, after its header.
        assert_eq!(e.node(&d, &[1, 1, 2, 3]).unwrap().value.as_int(), Some(9));
        let data = e.child_named(&d, &[1, 1], "data").unwrap().expect("data");
        assert_eq!(e.node(&d, &data).unwrap().size_bits, 9 * 8);
        let kind = e.child_named(&d, &[1, 2], "block_type").unwrap().expect("block_type");
        assert_eq!(e.node(&d, &kind).unwrap().value.as_int(), Some(5));
        // A block with no data area has none, rather than reading to the end.
        let none = e.child_named(&d, &[1, 0], "data").unwrap().expect("data");
        assert_eq!(e.node(&d, &none).unwrap().size_bits, 0);
        // Nothing is left over: an archive that ends properly has no run of
        // encrypted headers after it.
        assert_eq!(e.node(&d, &[2]).unwrap().size_bits, 0);
    }
}
