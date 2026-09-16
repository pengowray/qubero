//! tar: a header of five hundred and twelve bytes, the file it describes
//! rounded up to five hundred and twelve, and the next header after that.
//!
//! Every number in the header is written as octal digits with a terminator,
//! which is the format's age showing: it was made to be read by a program
//! that had no byte order to agree on. The size is the one that matters, since
//! it is what places the next header.
//!
//! An archive ends with two headers of nothing but zeros, and a writer may put
//! more zeros after those to fill out the block it was writing. A header of
//! zeros has no octal digits in it, so the choice between a header and an end
//! block is made on the first byte before anything tries to read a number.
//!
//! What this does not do is give an entry the meaning its type letter carries.
//! A long name and a pax attribute are each an entry whose data is a
//! description of the entry after it, and reading one of those would mean the
//! template rewriting what it had already said. They are measured and named
//! like any other entry.
//!
//! The one type letter that changes the header's own shape is GNU's `S`, a
//! sparse file. Its header keeps the sparse map where ustar keeps the prefix,
//! and a map too long for the four slots there spills into extension blocks
//! that follow the header and precede the data. An extension block is a
//! record of its own: twenty-one more map entries and a flag, with no name,
//! no size and no checksum. Read as a header it would be checked as one, and
//! a block of offsets never sums to what its "checksum" field holds.

use crate::template::{Check, Checksum, Covers, Encoding, Endian::Big, Expr as E, StrLen, Template, Time, Until, Ty as T};

/// Where a ustar archive writes its signature, which is the only thing in one
/// that marks the format.
pub const MAGIC_AT: usize = 257;

/// The type letter, which says what an entry is rather than what it holds.
const TYPES: &[(i128, &str)] = &[
    (0, "file"),
    (b'0' as i128, "file"),
    (b'1' as i128, "hard link"),
    (b'2' as i128, "symbolic link"),
    (b'3' as i128, "character device"),
    (b'4' as i128, "block device"),
    (b'5' as i128, "directory"),
    (b'6' as i128, "fifo"),
    (b'7' as i128, "contiguous file"),
    (b'g' as i128, "pax global header"),
    (b'x' as i128, "pax header"),
    (b'L' as i128, "gnu long name"),
    (b'K' as i128, "gnu long link name"),
    (b'S' as i128, "gnu sparse file"),
    (b'D' as i128, "gnu dump directory"),
    (b'M' as i128, "gnu multi-volume continuation"),
    (b'V' as i128, "gnu volume header"),
];

/// Where the type letter sits in a header, which is what tells a GNU sparse
/// header apart from a ustar one before either is read.
const TYPEFLAG_AT: i128 = 156;

pub fn tar() -> Template {
    Template::new(
        "tar",
        T::structure("TarArchive", vec![("entries", T::repeat(entry(), Until::End))]),
    )
}

/// One entry, or one of the blocks of zeros that end the archive. A header
/// begins with a name and an end block begins with a zero, which is the whole
/// of the difference at the point it has to be told. Among headers, the type
/// letter picks the GNU sparse shape from the ustar one.
fn entry() -> T {
    T::switch(
        E::peek(8, Big),
        vec![(0, end_block())],
        T::switch(E::peek_at(E::lit(TYPEFLAG_AT * 8), 8, Big), vec![(b'S' as i128, sparse_header())], header()),
    )
}

/// The first 345 bytes of a header, which ustar and GNU agree on. What comes
/// after them is where the two part.
fn header_front() -> Vec<(&'static str, T)> {
    vec![
        ("name", text(100)),
        ("mode", octal(8)),
        ("uid", octal(8)),
        ("gid", octal(8)),
        // How many bytes the file holds, which is what places the header
        // after this one.
        ("size", octal(12)),
        ("mtime", octal(12)),
        // The sum of every byte of the header with this field read as
        // spaces, which is the one check a tar has.
        ("checksum", octal(8)),
        ("typeflag", T::enumeration("TarType", T::u8(), TYPES)),
        ("linkname", text(100)),
        ("magic", text(6)),
        ("version", text(2)),
        // The names, rather than the numbers, of who owned the file: a
        // number means nothing on the machine the archive is unpacked on.
        ("uname", text(32)),
        ("gname", text(32)),
        ("devmajor", octal(8)),
        ("devminor", octal(8)),
    ]
}

/// The data an entry carries and the zeros that round it out to a block.
fn entry_data() -> Vec<(&'static str, T)> {
    vec![
        ("data", T::bytes(E::field("size"))),
        // Every entry starts on a block boundary, so a file that does not
        // fill its last block is followed by the zeros that do.
        ("padding", T::bytes(E::field("size").pad_to(512))),
    ]
}

/// The one check and the one time every header has, whichever shape it is.
fn checked_header(name: &str, fields: Vec<(&'static str, T)>) -> T {
    T::structure_named(name, "name", "data", fields)
        .counted_as("entry")
        .field_check("checksum", header_sum())
        // Octal digits, and still a count of seconds from 1970 once they are
        // read as the number they are.
        .field_time("mtime", Time::unix())
}

fn header() -> T {
    let mut fields = header_front();
    fields.extend([
        // A name too long for the field at the front is split, and this is
        // everything before the last slash that fits.
        ("prefix", text(155)),
        ("header_padding", T::bytes(E::lit(12))),
    ]);
    fields.extend(entry_data());
    checked_header("TarEntry", fields)
}

/// A GNU sparse file's header: the ustar front, and in place of the prefix
/// the first four entries of the map that says where the data goes when it
/// is unpacked. A map longer than four entries carries on in extension
/// blocks after the header, each saying whether another follows it.
fn sparse_header() -> T {
    let mut fields = header_front();
    fields.extend([
        ("atime", octal(12)),
        ("ctime", octal(12)),
        // Where this volume's piece of a multi-volume file starts.
        ("offset", octal(12)),
        ("longnames", T::bytes(E::lit(4))),
        ("unused", T::bytes(E::lit(1))),
        ("sparse", T::array(sparse_chunk(), E::lit(4))),
        // Whether extension blocks follow the header.
        ("isextended", T::u8()),
        // How big the file is once its holes are put back.
        ("realsize", octal(12)),
        ("header_padding", T::bytes(E::lit(17))),
        // Each extension block says whether another follows, and the header
        // says whether the first does. A header with no extensions has an
        // empty run here rather than a block read from the data.
        (
            "extensions",
            T::switch(
                E::field("isextended"),
                vec![(0, T::bytes(E::lit(0)))],
                T::repeat(sparse_extension(), Until::Cond(E::field("isextended").equal_to(E::lit(0)))),
            ),
        ),
    ]);
    fields.extend(entry_data());
    checked_header("TarSparseEntry", fields)
}

/// One block of a sparse map that outgrew its header: twenty-one more entries
/// and the flag that says whether another block follows. It has no checksum,
/// and the header's check does not reach it: that check covers the first 512
/// bytes of the entry, which is the header alone.
fn sparse_extension() -> T {
    T::structure(
        "TarSparseExtension",
        vec![
            ("sparse", T::array(sparse_chunk(), E::lit(21))),
            ("isextended", T::u8()),
            ("padding", T::bytes(E::lit(7))),
        ],
    )
}

/// One run of real data in a sparse file: where it goes in the unpacked
/// file and how much of it there is. The runs are stored back to back in
/// `data`, and the holes between them are not stored at all.
fn sparse_chunk() -> T {
    T::structure("TarSparseChunk", vec![("offset", octal(12)), ("numbytes", octal(12))])
}

/// The one check a tar has: every byte of the five-hundred-and-twelve-byte
/// header added up, with the eight bytes of `checksum` itself read as spaces.
///
/// The spaces are not a convention someone chose to be tidy. A writer filling
/// in the header cannot know the number until it has added up the field that
/// is going to hold it, so the format says what to put there while counting,
/// and every reader has to do the same or agree with no archive ever written.
///
/// The run starts at the front of the entry, which is the front of the header:
/// offsets in a check are counted from the structure it sits in, and `data`
/// and its padding are declared after the header inside that same structure.
fn header_sum() -> Check {
    Check::of(Checksum::ByteSum, Covers::Run { at: E::lit(0), len: E::lit(512) }).blanking(b' ')
}

/// One of the blocks of zeros at the end. Two of them are the end of the
/// archive, and a writer filling out its buffer may add more.
fn end_block() -> T {
    T::structure("TarEndBlock", vec![("zeros", T::bytes(E::lit(512)))])
}

/// A fixed run of text, of which the value is everything before the first
/// zero byte.
fn text(width: i128) -> T {
    T::text(StrLen::Padded { size: E::lit(width), pad: 0 }, Encoding::Utf8)
}

/// A number written as octal digits, or the zeros of a field nobody filled
/// in. A device number on an entry that is not a device is written as nothing
/// at all, and nothing is not a number: reading it as one would report an
/// error where the archive did nothing wrong.
fn octal(width: i128) -> T {
    T::switch(
        E::peek(8, Big),
        vec![(0, T::bytes(E::lit(width)))],
        // The digits end at a zero byte or at a space, and writers disagree
        // about which: the format allows either, and some write one of each in
        // the same header. So the digits are a token read inside a window of
        // the field's own width, which keeps every field where the format put
        // it whichever the writer chose.
        T::sized(E::lit(width), T::octal(StrLen::token(&[], &[0, b' ']))),
    )
}

/// Whether these bytes open a tar archive. Nothing marks the front of one:
/// the signature is 257 bytes in, which is where the header stopped being
/// what it was in 1979.
pub fn is_tar(head: &[u8]) -> bool {
    head.get(MAGIC_AT..MAGIC_AT + 5) == Some(b"ustar")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        checksum::byte_sum,
        document::Document,
        eval::{Evaluator, Value},
        source::MemSource,
    };

    fn entry_bytes_with(name: &str, data: &[u8], terminator: u8) -> Vec<u8> {
        let mut v = vec![0u8; 512];
        v[..name.len()].copy_from_slice(name.as_bytes());
        v[100..108].copy_from_slice(b"0000644\0");
        v[124..136].copy_from_slice(format!("{:011o}{}", data.len(), terminator as char).as_bytes());
        v[136..148].copy_from_slice(b"15245725034\0");
        v[156] = b'0';
        v[257..265].copy_from_slice(b"ustar\x0000");
        // The header's own sum, taken the way the format says: the eight bytes
        // that will hold it read as spaces. Written in GNU tar's shape, six
        // octal digits then a zero and a space, so the fixture is an archive a
        // real reader would accept rather than one with a made-up number in it.
        v[148..156].copy_from_slice(b"        ");
        let sum = byte_sum(&v[..512]);
        v[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        v.extend_from_slice(data);
        v.resize(512 + data.len().div_ceil(512) * 512, 0);
        v
    }

    fn entry_bytes(name: &str, data: &[u8]) -> Vec<u8> {
        entry_bytes_with(name, data, 0)
    }

    fn archive() -> Vec<u8> {
        let mut v = entry_bytes("hello.txt", b"hello, tar\n");
        v.extend_from_slice(&entry_bytes("notes.txt", b"and again\n"));
        v.extend_from_slice(&[0u8; 1024]);
        v
    }

    #[test]
    fn each_entry_places_the_next_one_by_its_size_rounded_up() {
        let d = Document::new(MemSource(archive()));
        let mut e = Evaluator::new(tar());
        // Two entries and the two end blocks.
        assert_eq!(e.node(&d, &[0]).unwrap().child_count, 4);
        assert_eq!(e.node(&d, &[0, 0, 0]).unwrap().value, Value::Str("hello.txt".into()));
        assert_eq!(e.node(&d, &[0, 0, 4]).unwrap().value.as_int(), Some(11));
        assert_eq!(e.node(&d, &[0, 0, 17]).unwrap().size_bits, 11 * 8);
        assert_eq!(e.node(&d, &[0, 0, 18]).unwrap().size_bits, 501 * 8);
        assert_eq!(e.node(&d, &[0, 1, 0]).unwrap().offset_bits, 1024 * 8);
        assert_eq!(e.node(&d, &[0, 2]).unwrap().size_bits, 512 * 8);
        assert!(is_tar(&archive()));
    }

    /// A device number is written as nothing on an entry that is not a
    /// device, and a field of zeros is not an octal number.
    #[test]
    fn a_field_nobody_filled_in_is_not_read_as_a_number() {
        let d = Document::new(MemSource(archive()));
        let mut e = Evaluator::new(tar());
        assert_eq!(e.node(&d, &[0, 0, 13]).unwrap().size_bits, 8 * 8);
        assert_eq!(e.node(&d, &[0, 0, 1]).unwrap().value.as_int(), Some(0o644));
    }

    /// Some writers end a numeric field with a space rather than a zero byte,
    /// and such an archive is not wrong: the field is still twelve bytes, and
    /// GNU tar writes the checksum as six digits, a zero and a space, so the
    /// digits end before the window does and the zero is not part of them.
    #[test]
    fn a_checksum_ending_in_a_zero_then_a_space_is_a_number() {
        let bytes = archive();
        let d = Document::new(MemSource(bytes.clone()));
        let mut e = Evaluator::new(tar());
        let n = e.node(&d, &[0, 0, 6]).unwrap();
        assert_eq!(n.name, "checksum");
        let mut header = bytes[..512].to_vec();
        header[148..156].copy_from_slice(b"        ");
        assert_eq!(n.value.as_int(), Some(byte_sum(&header) as i128));
    }

    /// The header sums to what it says it does, with its own eight bytes read
    /// as spaces. Every tar ever written depends on that substitution, and a
    /// check that took the bytes as they sit there would call all of them
    /// broken.
    #[test]
    fn a_header_sums_to_what_it_wrote_down_with_its_checksum_read_as_spaces() {
        let d = Document::new(MemSource(archive()));
        let mut e = Evaluator::new(tar());
        for entry in 0..2 {
            let path = [0, entry, 6];
            let info = e.check_of(&d, &path).unwrap().expect("the checksum field checks the header");
            assert_eq!(info.algorithm, "sum");
            assert_eq!(info.over, Some((entry as u64 * 1024, 512)));
            let blanked = info.blanked.expect("its own bytes are read as something else");
            assert_eq!((blanked.at, blanked.len, blanked.byte), (entry as u64 * 1024 + 148, 8, b' '));
            let v = e.run_check(&d, &path).unwrap().expect("and the sum can be taken");
            assert!(v.ok, "entry {entry}: computed {}, stored {}", v.computed, v.stored);
        }
    }

    /// One byte of the name changed, and the header no longer sums to what it
    /// says. The other half of the test above: a check that passed whatever
    /// the bytes were would pass that one too.
    #[test]
    fn a_header_somebody_edited_no_longer_sums() {
        let mut bytes = archive();
        bytes[0] = b'H';
        let d = Document::new(MemSource(bytes));
        let mut e = Evaluator::new(tar());
        let v = e.run_check(&d, &[0, 0, 6]).unwrap().expect("the check still applies");
        assert!(!v.ok, "computed {}, stored {}", v.computed, v.stored);
    }

    /// the size in it still places the header after this one.
    #[test]
    fn a_size_ending_in_a_space_is_read_the_same_way() {
        let mut v = entry_bytes_with("hello.txt", b"hello, tar\n", b' ');
        v.extend_from_slice(&[0u8; 1024]);
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(tar());
        assert_eq!(e.node(&d, &[0, 0, 4]).unwrap().value.as_int(), Some(11));
        assert_eq!(e.node(&d, &[0, 0, 4]).unwrap().size_bits, 12 * 8);
        assert_eq!(e.node(&d, &[0, 1, 0]).unwrap().offset_bits, 1024 * 8);
    }
}
