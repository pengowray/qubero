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
//! What is not read here: the extra area's records, which are a chain of their
//! own carrying encryption parameters, hard links, high-precision times and
//! the rest; the compression itself; and the encryption that may cover the
//! headers, which turns everything after it into bytes nothing can read
//! without a password.

use crate::codec::Codec;
use crate::template::{Check, Checksum, Covers, Named, Encoding, Endian::Little, Expr as E, StrLen, Template, Ty as T, Until};

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
const END_OF_ARCHIVE: i128 = 5;

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
                    T::repeat(block(), Until::FieldValue { field: "block_type".into(), value: END_OF_ARCHIVE }),
                ),
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
                    vec![(MAIN_ARCHIVE, main_fields()), (FILE, file_fields()), (SERVICE, file_fields()), (END_OF_ARCHIVE, end_fields())],
                    T::bytes(E::Remaining),
                ),
            ),
            // The records a newer version of the format added without moving
            // anything. A chain of its own, left unread: see the module doc.
            ("extra_area", T::bytes(E::field("extra_area_size").or(E::lit(0)))),
        ],
    )
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
    // The unpacked file, which the data area is only when the method is store.
    // Nothing here unpacks RAR, so every other method leaves a sum with
    // nothing to compare against; and a file split across volumes has only
    // part of itself in this one.
    //
    // The flag is asked as well as the method: without it the field is not
    // there at all, and a sum of no bytes against a zero read out of nothing
    // would come back as a file that checks out.
    .field_check(
        "data_crc32",
        Check::of(Checksum::Crc32, Covers::Field { name: Named::here("data") }).only_when(
            E::field("file_flags")
                .bit(2)
                .mul(E::field("method").equals(E::lit(0)))
                .mul(E::lit(1).sub(E::field("header_flags").bit(3)))
                .mul(E::lit(1).sub(E::field("header_flags").bit(4))),
        ),
    )
}

/// A file block's data area: the file itself when nothing packed it, and the
/// packed bytes when something did.
fn file_data() -> T {
    let size = || E::within(&["header", "data_size"]);
    T::switch(
        E::within(&["header", "fields", "method"]),
        vec![(0, T::decoded(size(), Codec::Stored, super::decoded_text()))],
        T::bytes(size()),
    )
}

/// The block that ends the archive, and says whether another volume follows.
fn end_fields() -> T {
    T::structure("Rar5End", vec![("end_flags", T::flags("Rar5EndFlags", T::leb_u(), END_FLAGS))])
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
        let mut v = MAGIC.to_vec();
        v.extend_from_slice(&block_bytes(1, 0, b"\x00"));
        v.extend_from_slice(&block_bytes(2, 0x02, b"\x09rest of it"));
        v.extend_from_slice(b"file data");
        v.extend_from_slice(&block_bytes(5, 0, b"\x00"));
        v
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

    /// A file block for `name` holding `data`, written in verbatim, with both
    /// sums correct: the header's, and the file's.
    fn stored_file(name: &str, data: &[u8], method: u64) -> Vec<u8> {
        let mut h = vec![2u8];
        h.extend(vint(0x02)); // header flags: data area follows
        h.extend(vint(data.len() as u64));
        h.extend(vint(0x04)); // file flags: checksum present
        h.extend(vint(data.len() as u64));
        h.extend(vint(0x20)); // attributes
        h.extend_from_slice(&crate::checksum::crc32(data).to_le_bytes());
        h.extend(vint(method << 7));
        h.extend(vint(0)); // host os
        h.extend(vint(name.len() as u64));
        h.extend_from_slice(name.as_bytes());
        sealed(&h)
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
        let data = e.node(&d, &[1, 1, 4]).unwrap();
        assert_eq!(data.name, "data");
        assert!(data.decoded && data.refused.is_none(), "a stored file opens: {data:?}");
    }

    #[test]
    fn a_packed_file_offers_no_check_of_its_contents() {
        // The same archive with the method set to normal. The data area is no
        // longer the file, and nothing here unpacks it, so the sum has nothing
        // to be taken over: it has to disappear rather than run over the
        // packed bytes and call the archive broken.
        let d = Document::new(MemSource(stored_archive(3)));
        let mut e = Evaluator::new(rar5());
        let crc = e.child_named(&d, &[1, 1, 2, 4], "data_crc32").unwrap().expect("data_crc32");
        assert_eq!(e.check_of(&d, &crc).unwrap(), None);
        // The header sum is not conditional and is still made.
        assert!(e.run_check(&d, &[1, 1, 0]).unwrap().unwrap().ok);
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
        assert_eq!(e.node(&d, &[1, 1, 4]).unwrap().size_bits, 9 * 8);
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
        assert_eq!(e.node(&d, &[1, 1, 4]).unwrap().size_bits, 9 * 8);
        assert_eq!(e.node(&d, &[1, 2, 3]).unwrap().value.as_int(), Some(5));
        // A block with no data area has none, rather than reading to the end.
        assert_eq!(e.node(&d, &[1, 0, 4]).unwrap().size_bits, 0);
    }
}
