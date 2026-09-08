//! RAR 4: seven bytes of signature and then a chain of blocks, each of which
//! is a checksum, a kind, two bytes of flags, and the size of its own header.
//!
//! That size is what makes the chain walkable, the same as in RAR 5: a reader
//! that does not know a block steps over it by the number and finds the next
//! one. What is different is everything else. RAR 5 writes every header number
//! as a variable-length integer and so has no fixed offsets at all; RAR 4
//! writes fixed-width little-endian fields, and the same four of them open
//! every block. The kind decides what follows.
//!
//! The top flag bit says a data area follows the header, and how long it is,
//! is a four-byte number written straight after the size. A file block always
//! has one, and there the number is the packed size: the header describes the
//! file and the bytes after it are the file.
//!
//! The same two bytes of flags mean three different things, which is why each
//! kind is a structure of its own here rather than one header and a switch
//! inside it. It buys something else as well: a file block reads its own name
//! as a field of itself, and so can be named by it, which is what lets a
//! listing say `hello.txt` where it would otherwise say `[1]`.
//!
//! Three bits in the middle of a file block's flags are not flags. They hold
//! the dictionary the file was compressed against, 64 KB doubling to 4096 KB,
//! and the eighth value is not a dictionary: all three bits set is how RAR 4
//! marks a directory. So `dictionary` is read out of the flags as a number.
//!
//! The signature is itself a well-formed block of the chain, and read as one
//! it is a checksum of 0x6152, a marker kind of 0x72, flags of 0x1a21 and a
//! size of seven. It is read as a signature here all the same, because there
//! is nothing in it that varies.
//!
//! What is not read here: the packed bytes, which are RAR's own compressors
//! and nothing a template describes. Nor the extended timestamps at the end of
//! a file header, the encoding a unicode name is written in after the NUL that
//! ends its ASCII form, the comment a 2.x archive keeps compressed inside its
//! main header, or the blocks a modern RAR no longer writes: the old comment,
//! authenticity, signature and recovery-record blocks. Each of those is
//! reached, sized and placed; what it holds is left as the bytes it is.

use crate::codec::Codec;
use crate::template::{Check, Checksum, Covers, Named, Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};

/// What one of these starts with. RAR 5 has the same first six bytes and one
/// more at the end, so the seventh byte is what tells the two apart: a zero
/// here, and a 1 and a 0 there.
pub const MAGIC: &[u8] = b"Rar!\x1a\x07\x00";

/// The block kinds. Everything from the old comment block down is what a RAR
/// before 3.0 wrote, and a modern one writes none of them.
const TYPES: &[(i128, &str)] = &[
    (0x72, "marker"),
    (0x73, "main archive"),
    (0x74, "file"),
    (0x75, "old comment"),
    (0x76, "authenticity"),
    (0x77, "old service"),
    (0x78, "recovery record"),
    (0x79, "signature"),
    (0x7a, "service"),
    (0x7b, "end of archive"),
];

const MAIN: i128 = 0x73;
const FILE: i128 = 0x74;
/// A service block: the same layout as a file block, and the name says which
/// service it is rather than which file. This is where a RAR 3 or later keeps
/// the archive comment, the recovery record and the NTFS streams.
const SERVICE: i128 = 0x7a;
/// The kind the chain stops at.
const END_OF_ARCHIVE: i128 = 0x7b;

/// The two bits every kind of block has, whatever else its flags mean.
const HEAD_FLAGS: &[(u32, &str)] = &[(14, "skip if unknown"), (15, "data area")];

const MAIN_FLAGS: &[(u32, &str)] = &[
    (0, "volume"),
    (1, "comment in header"),
    (2, "locked"),
    (3, "solid"),
    (4, "new volume naming"),
    (5, "authenticity"),
    (6, "recovery record"),
    (7, "headers encrypted"),
    (8, "first volume"),
    (14, "skip if unknown"),
    (15, "data area"),
];

/// A file block's flags. Bits 5 to 7 are missing on purpose: they are the
/// dictionary, which is a number rather than three independent bits, and is
/// read as one below.
const FILE_FLAGS: &[(u32, &str)] = &[
    (0, "data continues from previous volume"),
    (1, "data continues in next volume"),
    (2, "encrypted"),
    (3, "comment in header"),
    (4, "solid"),
    (8, "high size fields"),
    (9, "unicode name"),
    (10, "salt"),
    (11, "old file version"),
    (12, "extended time"),
    (14, "skip if unknown"),
    (15, "data area"),
];

const END_FLAGS: &[(u32, &str)] = &[
    (0, "more volumes follow"),
    (1, "archive checksum"),
    (2, "recovery space reserved"),
    (3, "volume number"),
    (14, "skip if unknown"),
    (15, "data area"),
];

/// The dictionary the file was compressed against, which is bits 5 to 7 of a
/// file block's flags. Seven is not a dictionary: it is how the format marks a
/// directory, and a directory has no packed bytes to hold a window over.
const DICTIONARY: &[(i128, &str)] =
    &[(0, "64k"), (1, "128k"), (2, "256k"), (3, "512k"), (4, "1024k"), (5, "2048k"), (6, "4096k"), (7, "directory")];

/// How hard the compressor tried, which is also which of RAR's compressors ran.
/// The method that writes the file in verbatim, so its bytes are the file.
const STORE: i128 = 0x30;

const METHOD: &[(i128, &str)] =
    &[(0x30, "store"), (0x31, "fastest"), (0x32, "fast"), (0x33, "normal"), (0x34, "good"), (0x35, "best")];

/// The system that wrote the entry, which is what says how to read `attr`.
const HOST_OS: &[(i128, &str)] =
    &[(0, "ms-dos"), (1, "os/2"), (2, "windows"), (3, "unix"), (4, "mac os"), (5, "beos")];

/// How many bytes a file header is before its name and anything optional. The
/// format's own number: `SIZEOF_FILEHEAD3` in every reader there is.
const FILE_HEAD_SIZE: i128 = 32;

pub fn rar4() -> Template {
    Template::new(
        "rar4",
        T::structure(
            "Rar4Archive",
            vec![
                ("magic", T::magic(MAGIC)),
                (
                    "blocks",
                    T::repeat(block(), Until::FieldValue { field: "head_type".into(), value: END_OF_ARCHIVE }),
                ),
            ],
        ),
    )
}

/// Which layout this block is written in, decided before a byte of it is read.
///
/// The kind is the third byte, and the two bytes of flags after it mean
/// different things depending on it, so nothing can be read as the right thing
/// until the kind is known. Looking at a byte without consuming it is what a
/// peek is for, and the shape it settles here is the whole block: the four
/// fields every kind shares are declared in each of them rather than in a
/// wrapper, because a wrapper is exactly what would put a file's name one level
/// down and out of reach of what names the block.
fn block() -> T {
    T::switch(
        E::peek_at(E::lit(2 * 8), 8, Big),
        vec![(MAIN, main_block()), (FILE, file_block()), (SERVICE, file_block()), (END_OF_ARCHIVE, end_block())],
        other_block(),
    )
}

/// What a block's `head_crc` covers: the header from the kind byte to the end
/// of what the size measures, summed as a CRC-32 and kept to sixteen bits.
///
/// The two bytes it skips are the sum itself, which is why the run starts at
/// two rather than at nothing. `head_size` counts from the front of the block,
/// so what is left once the sum is off the front is the size less two.
fn head_crc(when: Option<E>) -> Check {
    let c = Check::of(
        Checksum::Crc32Low16,
        Covers::Run { at: E::lit(2), len: E::field("head_size").sub(E::lit(2)) },
    );
    match when {
        Some(w) => c.only_when(w),
        None => c,
    }
}

/// One when the named bit of the block's flags is off. Every guard here is of
/// this shape: a flag says the header is not laid out the way the sum above
/// assumes, so the check disappears rather than failing.
fn without(bit: u32) -> E {
    E::lit(1).sub(E::field("head_flags").bit(bit))
}

/// The four fields every block opens with. The checksum covers the header from
/// the kind byte to the end of what the size measures, so it is the one field
/// of a block not under its own checksum.
///
/// Two kinds of block are not summed that way, and both are from before RAR 3.
/// A file block that keeps its comment in its own header is summed only as far
/// as the comment starts, and the authenticity and signature blocks were
/// written with checksums so unreliable that readers skip the check on them
/// rather than call every one of them broken.
fn base(flags: T) -> Vec<(&'static str, T)> {
    vec![
        ("head_crc", T::u16(Little)),
        ("head_type", T::enumeration_hex("Rar4BlockType", T::u8(), TYPES)),
        ("head_flags", flags),
        // Counted from the front of the block, so it takes in these seven
        // bytes as well as everything after them. Where the next block starts,
        // once the data area has been stepped over too.
        ("head_size", T::u16(Little)),
    ]
}

/// A file, and a service block, which is the same layout with a service name
/// where the file name goes.
///
/// The four-byte number a long block writes after its size is the packed size
/// here, and a file block is always a long block: the bytes after the header
/// are the file. The size is written twice over when the file is larger than
/// 4 GiB, as a low half here and a high half further down, because the format
/// is older than files that size and grew the fields rather than moving them.
fn file_block() -> T {
    let mut fields = base(T::flags("Rar4FileFlags", T::u16(Little), FILE_FLAGS));
    fields.extend(vec![
        ("pack_size", T::u32(Little)),
        // What it unpacks to. All ones with no high half beside it is how RAR
        // says it does not know, which is what packing from a pipe leaves: the
        // end of the file is then a mark in the compressed bits and nowhere
        // else.
        ("unp_size", T::u32(Little)),
        ("host_os", T::enumeration("Rar4HostOs", T::u8(), HOST_OS)),
        ("file_crc", T::u32(Little)),
        // MS-DOS packed date and time: seconds in twos, years from 1980.
        ("ftime", T::u32(Little)),
        // Ten times the version of RAR that packed it, so 20 is RAR 2.0 and
        // 29 is 2.9, which is what every 3.x and 4.x archive writes.
        ("unp_ver", T::u8()),
        ("method", T::enumeration_hex("Rar4Method", T::u8(), METHOD)),
        ("name_size", T::u16(Little)),
        // What the host OS makes of it: MS-DOS attribute bits from a DOS or
        // Windows writer, and a Unix mode from a Unix one, in which a symbolic
        // link is what the top four bits say.
        ("attr", T::u32(Little)),
        // Three bits of the flags read as a number rather than as flags, which
        // is what tells a directory from a file. No bytes of its own.
        ("dictionary", T::enumeration("Rar4Dictionary", T::computed(E::bit_field(E::field("head_flags"), 7, 3)), DICTIONARY)),
        // The high halves of both sizes, written only for a file too big for
        // the low halves alone.
        ("high_pack_size", T::present_if(E::field("head_flags").bit(8), T::u32(Little))),
        ("high_unp_size", T::present_if(E::field("head_flags").bit(8), T::u32(Little))),
        // The name takes every byte the size counts, and reads to the first
        // NUL in them. With the unicode flag set that NUL is not the end of
        // the field: RAR's own encoding of the wide name follows it, and the
        // ASCII form before it is what a reader without that encoding gets.
        (
            "name",
            T::sized(bounded(E::field("name_size")), T::text(StrLen::Terminated { end: 0, or_end: true }, Encoding::Cp437)),
        ),
        ("salt", T::present_if(E::field("head_flags").bit(10), T::bytes(E::lit(8)))),
        // What the header still has room for once everything above is off it:
        // the extended timestamps, and in a service block the fields the
        // service carries. A salted service block would put those two the
        // other way round, which is a combination nothing writes.
        (
            "extra",
            T::bytes(bounded(
                E::field("head_size")
                    .sub(E::lit(FILE_HEAD_SIZE))
                    .sub(E::size_of("high_pack_size"))
                    .sub(E::size_of("high_unp_size"))
                    .sub(E::size_of("name"))
                    .sub(E::size_of("salt")),
            )),
        ),
        // The file itself, which is why the archive is the size it is. An
        // absent high half reads as zero, so this is the whole number whether
        // the format wrote it in one field or two.
        //
        // Stored means the file was written in verbatim, so those bytes are
        // already a document and can be opened as one: that is what the codec
        // that copies is for. Anything packed stays bytes, since nothing here
        // unpacks RAR.
        (
            "data",
            {
                let size = bounded(E::field("pack_size").add(E::field("high_pack_size").shl(E::lit(32))));
                T::switch(
                    E::field("method"),
                    vec![(STORE, T::decoded(size.clone(), Codec::Stored, super::decoded_text()))],
                    T::bytes(size),
                )
            },
        ),
    ]);
    T::structure_named("Rar4File", "name", "data", fields)
        .counted_as("block")
        // A 2.x file block with its comment inside the header is summed only
        // as far as the comment starts, which is a layout nothing here reads:
        // the check has to disappear for it rather than call the archive
        // broken.
        .field_check("head_crc", head_crc(Some(without(3))))
        // The file, which the data area is only when nothing packed it. The
        // switch above leaves the data as plain bytes for every other method,
        // and that is what says the check cannot be made.
        //
        // Encrypted, and split across volumes, are the two other ways the data
        // area stops being the file: one has the plaintext nowhere in the
        // archive, and the other has only part of the file here while the sum
        // is over the whole of it.
        .field_check(
            "file_crc",
            Check::of(
                Checksum::Crc32,
                Covers::Unpacked {
                    name: Named::here("data"),
                    len: Some(E::field("unp_size").add(E::field("high_unp_size").shl(E::lit(32)))),
                },
            )
            // A directory is a file block with no file: RAR writes a zero sum
            // and no bytes, and a check that passed over nothing would be a
            // green tick meaning nothing. Three bits of the flags say so, and
            // seven of them is the mark.
            .only_when(
                without(0)
                    .mul(without(1))
                    .mul(without(2))
                    .mul(E::lit(1).sub(E::field("dictionary").equals(E::lit(7)))),
            ),
        )
}

/// The archive header, which is the first block of every RAR 4 and says what
/// the archive as a whole is: a volume, a solid archive, one with its headers
/// encrypted. The two fields after the flags are where an old authenticity
/// signature was; a RAR that never had one writes zeroes.
fn main_block() -> T {
    let mut fields = base(T::flags("Rar4MainFlags", T::u16(Little), MAIN_FLAGS));
    fields.extend(vec![
        ("add_size", T::present_if(E::field("head_flags").bit(15), T::u32(Little))),
        ("high_pos_av", T::u16(Little)),
        ("pos_av", T::u32(Little)),
        // A 2.x archive keeps its comment in here, compressed, which is what
        // the comment flag means and what this is the room for.
        (
            "extra",
            T::bytes(bounded(E::field("head_size").sub(E::lit(13)).sub(E::size_of("add_size")))),
        ),
        ("data", T::bytes(bounded(E::field("add_size")))),
    ]);
    T::structure("Rar4Main", fields).counted_as("block").field_check("head_crc", head_crc(Some(without(1))))
}

/// The block that ends the archive, and the only thing that says where the
/// chain stops. Both of its fields are for a multi-volume set: which volume
/// this is, and a checksum over the whole of it.
fn end_block() -> T {
    let mut fields = base(T::flags("Rar4EndFlags", T::u16(Little), END_FLAGS));
    fields.extend(vec![
        ("archive_crc", T::present_if(E::field("head_flags").bit(1), T::u32(Little))),
        ("volume_number", T::present_if(E::field("head_flags").bit(3), T::u16(Little))),
        // Seven zero bytes when the reserved-space flag is set, which is room
        // a recovery volume writes its own numbers into later.
        (
            "extra",
            T::bytes(bounded(
                E::field("head_size").sub(E::lit(7)).sub(E::size_of("archive_crc")).sub(E::size_of("volume_number")),
            )),
        ),
    ]);
    T::structure("Rar4End", fields).counted_as("block").field_check("head_crc", head_crc(None))
}

/// Every other kind. There is nothing to read in them that the four fields
/// above have not already said, and there does not need to be: the size steps
/// over the header and the long-block number steps over whatever followed it,
/// which is the whole reason a format writes both.
fn other_block() -> T {
    let mut fields = base(T::flags("Rar4HeadFlags", T::u16(Little), HEAD_FLAGS));
    fields.extend(vec![
        ("add_size", T::present_if(E::field("head_flags").bit(15), T::u32(Little))),
        ("extra", T::bytes(bounded(E::field("head_size").sub(E::lit(7)).sub(E::size_of("add_size"))))),
        ("data", T::bytes(bounded(E::field("add_size")))),
    ]);
    T::structure("Rar4Block", fields).counted_as("block")
}

/// A length the file gave, kept between nothing and what is left of the file.
///
/// Both ends happen. An archive that stopped mid-transfer says its last file
/// is longer than what arrived, and a header whose size is smaller than the
/// fields it is supposed to hold would leave a negative amount over. Placing
/// what is there says more than refusing the block that ran off the end, and
/// the run that is left short is itself the thing a reader wants to see.
fn bounded(n: E) -> E {
    n.at_least(E::lit(0)).at_most(E::Remaining)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::Document,
        eval::{Evaluator, Value},
        source::MemSource,
    };

    /// A block, with its checksum worked out the way the format works it out:
    /// over the header from the kind byte to the end of what the size counts.
    fn block_bytes(head_type: u8, flags: u16, body: &[u8]) -> Vec<u8> {
        let head_size = 7 + body.len() as u16;
        let mut header = vec![head_type];
        header.extend_from_slice(&flags.to_le_bytes());
        header.extend_from_slice(&head_size.to_le_bytes());
        header.extend_from_slice(body);
        let crc = crc32(&header) as u16;
        let mut v = crc.to_le_bytes().to_vec();
        v.extend_from_slice(&header);
        v
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for b in bytes {
            crc ^= u32::from(*b);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
            }
        }
        !crc
    }

    /// A stored file block: the header, the name, and the file straight after
    /// it. `flags` carries the long-block bit and whatever else is being tested.
    fn file(name: &[u8], data: &[u8], attr: u32, flags: u16) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&(data.len() as u32).to_le_bytes());
        body.extend_from_slice(&(data.len() as u32).to_le_bytes());
        body.push(2); // written on Windows
        body.extend_from_slice(&crc32(data).to_le_bytes());
        body.extend_from_slice(&0x5D28_6400u32.to_le_bytes()); // an MS-DOS date
        body.push(20); // packed by RAR 2.0, which is what a stored file says
        body.push(0x30); // stored
        body.extend_from_slice(&(name.len() as u16).to_le_bytes());
        body.extend_from_slice(&attr.to_le_bytes());
        body.extend_from_slice(name);
        let mut v = block_bytes(0x74, flags, &body);
        v.extend_from_slice(data);
        v
    }

    /// An archive of two files and a directory between them, which is what a
    /// RAR of a folder looks like: an archive header, a block for every entry,
    /// and the block that ends the chain.
    fn archive() -> Vec<u8> {
        let mut v = MAGIC.to_vec();
        v.extend_from_slice(&block_bytes(0x73, 0x0000, &[0; 6]));
        v.extend_from_slice(&file(b"hello.txt", b"hello, world\n", 0x20, 0x8000));
        // A directory: every dictionary bit set, and no bytes after it.
        v.extend_from_slice(&file(b"notes", b"", 0x10, 0x8000 | 0x00e0));
        v.extend_from_slice(&file(b"notes\\second.txt", b"the second one\n", 0x20, 0x8000));
        v.extend_from_slice(&block_bytes(0x7b, 0x4000, &[]));
        v
    }

    #[test]
    fn the_chain_stops_at_the_block_that_ends_the_archive() {
        let d = Document::new(MemSource(archive()));
        let mut e = Evaluator::new(rar4());
        // Five blocks: the archive header, three entries and the end.
        assert_eq!(e.node(&d, &[1]).unwrap().child_count, 5);
        assert_eq!(e.node(&d, &[1, 0]).unwrap().type_name, "Rar4Main");
        assert_eq!(e.node(&d, &[1, 1]).unwrap().type_name, "Rar4File");
        assert_eq!(e.node(&d, &[1, 4]).unwrap().type_name, "Rar4End");
        assert_eq!(
            e.node(&d, &[1, 4, 1]).unwrap().value,
            Value::Enum { raw: 0x7b, name: Some("end of archive".into()), hex: true }
        );
        // The end block is seven bytes and nothing else, and the archive ends
        // where it ends.
        let end = e.node(&d, &[1, 4]).unwrap();
        assert_eq!(end.size_bits, 7 * 8);
    }

    #[test]
    fn a_file_block_is_named_by_the_name_in_it_and_the_file_follows_the_header() {
        let d = Document::new(MemSource(archive()));
        let mut e = Evaluator::new(rar4());
        assert_eq!(e.node(&d, &[1, 1, 16]).unwrap().value, Value::Str("hello.txt".into()));
        assert_eq!(e.node(&d, &[1, 3, 16]).unwrap().value, Value::Str("notes\\second.txt".into()));
        // And the block wears it, which is the whole point: a megabyte of
        // packed bytes called `[3]` is a box nobody can place.
        assert_eq!(e.node(&d, &[1, 1]).unwrap().name, "[1] hello.txt");
        assert_eq!(e.node(&d, &[1, 3]).unwrap().name, "[3] notes\\second.txt");
        // The packed size is the length of the data area, which is the file.
        assert_eq!(e.node(&d, &[1, 1, 4]).unwrap().value.as_int(), Some(13));
        assert_eq!(e.node(&d, &[1, 1, 19]).unwrap().size_bits, 13 * 8);
        assert_eq!(e.node(&d, &[1, 3, 19]).unwrap().size_bits, 15 * 8);
        // And the block is as long as its header and its data together, so the
        // one after it starts where it ends.
        assert_eq!(e.node(&d, &[1, 1]).unwrap().size_bits, (32 + 9 + 13) * 8);
        assert_eq!(e.node(&d, &[1, 1, 10]).unwrap().value, Value::Enum { raw: 0x30, name: Some("store".into()), hex: true });
        assert_eq!(
            e.node(&d, &[1, 1, 6]).unwrap().value,
            Value::Enum { raw: 2, name: Some("windows".into()), hex: false }
        );
    }

    /// The three bits in the middle of the flags are a number, and the value
    /// they cannot hold as a dictionary is what marks a directory.
    #[test]
    fn a_directory_is_told_from_a_file_by_the_dictionary_bits() {
        let d = Document::new(MemSource(archive()));
        let mut e = Evaluator::new(rar4());
        assert_eq!(
            e.node(&d, &[1, 1, 13]).unwrap().value,
            Value::Enum { raw: 0, name: Some("64k".into()), hex: false }
        );
        assert_eq!(
            e.node(&d, &[1, 2, 13]).unwrap().value,
            Value::Enum { raw: 7, name: Some("directory".into()), hex: false }
        );
        // The directory takes no bytes after its header, and reads as its name.
        assert_eq!(e.node(&d, &[1, 2, 19]).unwrap().size_bits, 0);
        assert_eq!(e.node(&d, &[1, 2, 16]).unwrap().value, Value::Str("notes".into()));
    }

    /// With the high halves absent the header is thirty-two bytes and a name,
    /// and there is nothing left over. With them there it is eight bytes more,
    /// the size is the two halves put together, and the extended timestamps
    /// are what the header still has room for.
    #[test]
    fn the_optional_fields_move_everything_after_them() {
        let d = Document::new(MemSource(archive()));
        let mut e = Evaluator::new(rar4());
        assert_eq!(e.node(&d, &[1, 1, 14]).unwrap().size_bits, 0, "no high halves");
        assert_eq!(e.node(&d, &[1, 1, 15]).unwrap().size_bits, 0, "no high halves");
        assert_eq!(e.node(&d, &[1, 1, 17]).unwrap().size_bits, 0, "no salt");
        assert_eq!(e.node(&d, &[1, 1, 18]).unwrap().size_bits, 0, "and nothing left over");

        // The same file with the high halves, a salt, and four bytes of
        // extended time on the end.
        let mut body = Vec::new();
        body.extend_from_slice(&8u32.to_le_bytes()); // pack_size, low half
        body.extend_from_slice(&8u32.to_le_bytes()); // unp_size, low half
        body.push(3); // written on Unix
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        body.push(29);
        body.push(0x33); // normal
        body.extend_from_slice(&5u16.to_le_bytes());
        body.extend_from_slice(&0x81a4u32.to_le_bytes()); // a Unix mode
        body.extend_from_slice(&1u32.to_le_bytes()); // high_pack_size: 4 GiB and eight bytes
        body.extend_from_slice(&0u32.to_le_bytes()); // high_unp_size
        body.extend_from_slice(b"large");
        body.extend_from_slice(&[0xaa; 8]); // salt
        body.extend_from_slice(&[0x00; 4]); // extended time

        let mut v = MAGIC.to_vec();
        v.extend_from_slice(&block_bytes(0x73, 0x0000, &[0; 6]));
        v.extend_from_slice(&block_bytes(0x74, 0x8000 | 0x0100 | 0x0400 | 0x1000, &body));
        // Nowhere near four gigabytes of file, so the data area is cut to what
        // is left rather than the block failing.
        v.extend_from_slice(&[0xee; 8]);

        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(rar4());
        assert_eq!(e.node(&d, &[1, 1, 14]).unwrap().size_bits, 4 * 8);
        assert_eq!(e.node(&d, &[1, 1, 16]).unwrap().value, Value::Str("large".into()));
        assert_eq!(e.node(&d, &[1, 1, 17]).unwrap().size_bits, 8 * 8, "the salt");
        assert_eq!(e.node(&d, &[1, 1, 18]).unwrap().size_bits, 4 * 8, "the extended time");
        // The two halves make a number no single field could hold.
        assert_eq!(e.node(&d, &[1, 1, 19]).unwrap().size_bits, 8 * 8);
        assert_eq!(
            e.node(&d, &[1, 1, 6]).unwrap().value,
            Value::Enum { raw: 3, name: Some("unix".into()), hex: false }
        );
    }

    /// A kind nothing here reads is still stepped over by the two numbers the
    /// format writes for exactly that: the header size, and the data size a
    /// long block puts after it.
    #[test]
    fn a_block_of_an_unread_kind_is_still_sized_and_stepped_over() {
        let mut v = MAGIC.to_vec();
        v.extend_from_slice(&block_bytes(0x73, 0x0000, &[0; 6]));
        // An old recovery record: a long block, four bytes of data size, and
        // fifteen bytes of header nothing here takes apart.
        let mut body = 4096u32.to_le_bytes().to_vec();
        body.extend_from_slice(&[0x11; 15]);
        v.extend_from_slice(&block_bytes(0x78, 0x8000, &body));
        v.extend_from_slice(&[0x22; 4096]);
        v.extend_from_slice(&block_bytes(0x7b, 0x4000, &[]));

        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(rar4());
        assert_eq!(e.node(&d, &[1]).unwrap().child_count, 3);
        assert_eq!(e.node(&d, &[1, 1]).unwrap().type_name, "Rar4Block");
        assert_eq!(
            e.node(&d, &[1, 1, 1]).unwrap().value,
            Value::Enum { raw: 0x78, name: Some("recovery record".into()), hex: true }
        );
        assert_eq!(e.node(&d, &[1, 1, 5]).unwrap().size_bits, 15 * 8, "the header nothing reads");
        assert_eq!(e.node(&d, &[1, 1, 6]).unwrap().size_bits, 4096 * 8, "the data area it stepped over");
        assert_eq!(e.node(&d, &[1, 2, 1]).unwrap().value.as_int(), Some(END_OF_ARCHIVE));
    }

    /// The end block carries two fields for a volume set, and the reserved
    /// space a recovery volume writes into is what the size still counts.
    #[test]
    fn the_end_block_of_a_volume_says_which_volume_it_was() {
        let mut body = 0x1234_5678u32.to_le_bytes().to_vec();
        body.extend_from_slice(&3u16.to_le_bytes());
        body.extend_from_slice(&[0; 7]);
        let mut v = MAGIC.to_vec();
        v.extend_from_slice(&block_bytes(0x73, 0x0001, &[0; 6]));
        v.extend_from_slice(&block_bytes(0x7b, 0x4000 | 0x0002 | 0x0004 | 0x0008, &body));

        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(rar4());
        assert_eq!(e.node(&d, &[1, 1, 4]).unwrap().value.as_int(), Some(0x1234_5678));
        assert_eq!(e.node(&d, &[1, 1, 5]).unwrap().value.as_int(), Some(3));
        assert_eq!(e.node(&d, &[1, 1, 6]).unwrap().size_bits, 7 * 8);
    }
}
