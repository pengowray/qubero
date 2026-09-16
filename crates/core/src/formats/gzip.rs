//! GZIP: a ten-byte header, four optional pieces the flag byte gates, the
//! deflate stream, and a checksum and length at the very end.
//!
//! The optional pieces are what makes this worth writing down. Each one exists
//! only when its bit in `flg` is set, and the template has no `if`: a field of
//! `bit * n` bytes is the field when the bit is one and nothing at all when it
//! is zero. The two string fields need a real choice, since a C string has no
//! length to multiply, so those are a switch on the bit with an empty case.
//!
//! The compressed data cannot be measured without decompressing it, so it is
//! everything between the header and the last eight bytes.

use crate::codec::Codec;
use crate::template::{Check, Checksum, Covers, Named, Encoding, Endian::*, Expr as E, Step, StrLen, Template, Time, Ty as T, Until};

/// The bits of `flg`, and what each one puts after the header.
const FLAGS: &[(u32, &str)] = &[
    (0, "text"),
    (1, "header crc"),
    (2, "extra field"),
    (3, "name"),
    (4, "comment"),
];

/// The system the file was made on, as the numbers FAT and Amiga and Unix were
/// given in 1996.
const OS: &[(i128, &str)] = &[
    (0, "fat"),
    (1, "amiga"),
    (2, "vms"),
    (3, "unix"),
    (4, "vm/cms"),
    (5, "atari tos"),
    (6, "hpfs"),
    (7, "macintosh"),
    (8, "z-system"),
    (9, "cp/m"),
    (10, "tops-20"),
    (11, "ntfs"),
    (12, "qdos"),
    (13, "acorn riscos"),
    (255, "unknown"),
];

/// Bit `n` of `flg`, as a number that is one or zero.
fn bit(n: u32) -> E {
    let f = E::field("flg");
    f.clone().div(E::lit(1i128 << n)).sub(f.div(E::lit(1i128 << (n + 1))).mul(E::lit(2)))
}

/// The extra field: its own length, and then that many bytes of subfields,
/// each of which is a two-byte name, a length, and a payload. What is in there
/// is up to whoever wrote it, so the bytes are left whole.
fn extra_field() -> T {
    T::structure(
        "Extra",
        vec![("length", T::u16(Little)), ("data", T::bytes(E::field("length")))],
    )
}

/// A string that is there only when its flag bit is set.
fn optional_string() -> T {
    T::text(StrLen::Terminated { end: 0, or_end: false }, Encoding::Latin1)
}

/// A gzip file: one member after another, and what they unpack to, joined.
///
/// Nearly every gzip is one member, and `gzip` itself only ever writes one.
/// But a member is what the format defines, and the format says a file is any
/// number of them one after the other, unpacked to the concatenation of what
/// each holds: `cat a.gz b.gz > ab.gz` is a gzip, and a tar split into eight
/// pieces and packed piece by piece is the one in the samples. Each member
/// seals its own output with its own trailer, so a check made over the wrong
/// member's output is a good file reported broken, which is what a template
/// of one member did to the seven trailers it never read.
///
/// The members are where the bytes are; `decoded` is the stream they come to.
/// Each member's deflate run is sized by where its stream stops, since gzip
/// writes no length for it (see [`E::StreamLen`]); a run too large to
/// measure that way is read as everything before the trailer, the way the
/// one-member template read it, so a file of one member past the cap still
/// places its trailer.
pub fn gzip() -> Template {
    let stream = E::stream_len(Codec::Deflate).or(E::Remaining.sub(E::lit(8)));
    Template::new(
        "gzip",
        T::structure(
            "Gzip",
            vec![
                ("members", T::repeat(member("GzipMember", extra_field(), stream, cut()), Until::End)),
                // Every member's output, in order, as one stream. What the
                // file holds is here rather than in any one member: one
                // member of a split tar is a piece of a tar, and only the
                // pieces joined are the tar. Each part is as long as its
                // member's trailer says, so no member is inflated to place
                // the stream.
                (
                    "decoded",
                    T::stitched(
                        vec![Step::field("members"), Step::each(), Step::field("compressed")],
                        Some(E::field("original_size")),
                        None,
                        super::decoded_text(),
                    ),
                ),
            ],
        ),
    )
}

/// What one member unpacks to on its own: a piece of the stream `decoded`
/// joins, which reads as bytes and is not opened as what its front looks
/// like. See [`crate::template::StructDef::cut`].
fn cut() -> T {
    T::structure_named("Cut", "", "bytes", vec![("bytes", T::bytes(E::Remaining))]).as_cut()
}

/// One gzip member, which is the whole of a gzip file and one block of a BGZF
/// file. `extra` is what the extra field reads as when its flag is set,
/// `stream` how long the deflate run is, and `contents` what it opens into.
///
/// Shared because a BGZF block is a gzip member to the byte, and a second copy
/// of the header, the flags and the two checks would drift from this one. What
/// BGZF adds is the names inside the extra field and a size for the whole
/// member, which is why the run's length is the caller's to say: a BGZF block
/// is sized by its header and its run is everything before the trailer,
/// measured without inflating anything, and sixteen thousand blocks are
/// placed that way. A plain member has no such number and is sized by where
/// its stream stops.
pub(crate) fn member(name: &str, extra: T, stream: E, contents: T) -> T {
    T::structure(
        name,
        vec![
            ("magic", T::magic(b"\x1f\x8b")),
            ("method", T::enumeration("Method", T::u8(), &[(8, "deflate")])),
            ("flg", T::flags("Flags", T::u8(), FLAGS)),
            // Seconds since 1970, or zero when the compressor had no time
            // to give: gzip writes zero for input it read from a pipe.
            ("mtime", T::u32(Little)),
            ("extra_flags", T::enumeration("ExtraFlags", T::u8(), &[(2, "best compression"), (4, "fastest")])),
            ("os", T::enumeration("Os", T::u8(), OS)),
            ("extra", T::switch(bit(2), vec![(1, extra)], T::bytes(E::lit(0)))),
            ("name", T::switch(bit(3), vec![(1, optional_string())], T::bytes(E::lit(0)))),
            ("comment", T::switch(bit(4), vec![(1, optional_string())], T::bytes(E::lit(0)))),
            // Two bytes when the bit says so and nothing at all when it
            // does not, like the three fields above. A number rather than
            // the run of bytes it used to be: it is a sixteen-bit check
            // written little-endian, and reading it as bytes left the one
            // thing about it that matters, which way round it goes, written
            // down nowhere.
            ("header_crc", T::switch(bit(1), vec![(1, T::u16(Little))], T::bytes(E::lit(0)))),
            // Deflate, as long as the caller said. The run keeps its own
            // length and stays where it is; what comes out of it is read as
            // fields of its own, and what the decoder read on the way is read
            // as the blocks it read them from.
            ("compressed", T::decoded(stream, Codec::Deflate, contents)),
            ("crc32", T::u32(Little)),
            // The size of what was compressed, modulo four gigabytes,
            // which is why a large file's number looks wrong.
            ("original_size", T::u32(Little)),
        ],
    )
    // Everything written before it, which for a header is everything there
    // is: the check is the last thing in it. Only when the flag put it
    // there at all, since a field of no bytes reads as zero and zero is a
    // number a sixteen-bit sum can honestly come to.
    .field_check("header_crc", Check::of(Checksum::Crc32Low16, Covers::UpToHere).only_when(bit(1)))
    // Zero is not the first instant of 1970 here: it is what the compressor
    // wrote because it had none, which the comment on the field says and
    // the reader is owed as well.
    .field_time("mtime", Time::unix().unset(0))
    // The file that went in, not the deflate stream it came out as. What
    // the trailer says that comes to is only for deciding whether to unpack
    // it unasked; it is written modulo four gigabytes and a large file's
    // number is smaller than the file.
    .field_check(
        "crc32",
        Check::of(
            Checksum::Crc32,
            Covers::Unpacked { name: Named::here("compressed"), len: Some(E::field("original_size")) },
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn member(flg: u8, after_header: &[u8]) -> Vec<u8> {
        let mut v = vec![0x1f, 0x8b, 8, flg];
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&[0, 3]);
        v.extend_from_slice(after_header);
        v.extend_from_slice(&[0x03, 0x00]); // an empty deflate stream
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v
    }

    /// A member with no name and no flags, holding `content` in one stored
    /// deflate block, with the trailer gzip would write for it.
    fn packed(content: &[u8]) -> Vec<u8> {
        let mut v = vec![0x1f, 0x8b, 8, 0];
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&[0, 3]);
        // One stored block, final: type zero, then the length and its
        // complement, then the bytes as they are.
        v.push(0x01);
        v.extend_from_slice(&(content.len() as u16).to_le_bytes());
        v.extend_from_slice(&(!(content.len() as u16)).to_le_bytes());
        v.extend_from_slice(content);
        v.extend_from_slice(&crate::checksum::crc32(content).to_le_bytes());
        v.extend_from_slice(&(content.len() as u32).to_le_bytes());
        v
    }

    /// The first member of the file: a gzip is a list of them, and the one
    /// nearly every file has is the first.
    const FIRST: &[usize] = &[0, 0];

    fn under(base: &[usize], i: usize) -> Vec<usize> {
        [base, &[i]].concat()
    }

    #[test]
    fn a_name_is_read_only_when_its_bit_is_set() {
        let d = Document::new(MemSource(member(0x08, b"hello.txt\0")));
        let mut ev = Evaluator::new(gzip());
        assert_eq!(ev.node(&d, &under(FIRST, 7)).unwrap().value, Value::Str("hello.txt".into()));
        assert_eq!(ev.node(&d, &under(FIRST, 8)).unwrap().size_bits, 0);
        assert_eq!(ev.node(&d, &under(FIRST, 10)).unwrap().size_bits, 2 * 8);

        // The same file without the flag: the name is not there, and the
        // deflate stream starts where the header ends.
        let d = Document::new(MemSource(member(0, b"")));
        let mut ev = Evaluator::new(gzip());
        assert_eq!(ev.node(&d, &under(FIRST, 7)).unwrap().size_bits, 0);
        assert_eq!(ev.node(&d, &under(FIRST, 10)).unwrap().offset_bits, 10 * 8);
    }

    #[test]
    fn the_trailer_is_the_last_eight_bytes_whatever_came_before() {
        let d = Document::new(MemSource(member(0x08, b"a\0")));
        let mut ev = Evaluator::new(gzip());
        let size = ev.node(&d, &under(FIRST, 12)).unwrap();
        assert_eq!(size.value, Value::UInt(0));
        assert_eq!(size.offset_bits, (10 + 2 + 2 + 4) * 8);
    }

    /// Two members one after the other, which is what `cat a.gz b.gz` makes
    /// and what a tar packed in pieces is. Each member's run ends where its
    /// stream does, so the second member starts on its own magic, each
    /// trailer checks the output of its own member, and the file's `decoded`
    /// is both outputs joined.
    #[test]
    fn a_second_member_starts_where_the_first_stream_stops_and_checks_its_own_output() {
        let (a, b) = (b"the first piece of the file, ".as_slice(), b"and the second.".as_slice());
        let mut bytes = packed(a);
        let first_len = bytes.len();
        bytes.extend_from_slice(&packed(b));
        let total = bytes.len();
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(gzip());

        assert_eq!(ev.node(&d, &[0]).unwrap().child_count, 2, "two members");
        let second: &[usize] = &[0, 1];
        assert_eq!(ev.node(&d, second).unwrap().offset_bits, first_len as u64 * 8);
        assert_eq!(ev.node(&d, &under(second, 12)).unwrap().offset_bits, (total - 4) as u64 * 8);
        // The first member's run is its stream and no more: the second
        // member's header is not part of it.
        let run = ev.node(&d, &under(FIRST, 10)).unwrap();
        assert_eq!(run.size_bits, (1 + 4 + a.len()) as u64 * 8);

        // Each trailer against its own member's output. Before the members
        // were told apart, the one trailer in reach was the last member's
        // and the one stream read was the first member's.
        for base in [FIRST, second] {
            let v = ev.run_check(&d, &under(base, 11)).unwrap().expect("a trailer checks the member");
            assert!(v.ok, "member at {base:?}: computed {} stored {}", v.computed, v.stored);
        }

        // One member's output is a piece and opens as bytes; the joined
        // stream is the file.
        let piece = ev.open_space(&d, 0, &under(FIRST, 10)).unwrap().expect("the first run opens");
        assert_eq!(ev.space(piece).unwrap().bytes(), a);
        assert!(!ev.space(piece).unwrap().recognised, "a piece is not opened as what its front looks like");
        let whole = ev.open_space(&d, 0, &[1]).unwrap().expect("the joined stream opens");
        assert_eq!(ev.space(whole).unwrap().bytes(), [a, b].concat());
    }

    /// A run that will not decode has no length of its own to be measured
    /// by, and is read as it always was: everything before the trailer. The
    /// trailer is still placed, and still the last eight bytes.
    #[test]
    fn a_stream_that_will_not_decode_runs_to_the_trailer() {
        let mut bytes = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3];
        // A block of the reserved type, which no decoder reads.
        bytes.extend_from_slice(&[0x07, 0x00, 0x00, 0x00]);
        bytes.extend_from_slice(&[0; 8]);
        let total = bytes.len();
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(gzip());
        assert_eq!(ev.node(&d, &[0]).unwrap().child_count, 1);
        assert_eq!(ev.node(&d, &under(FIRST, 10)).unwrap().size_bits, 4 * 8);
        assert_eq!(ev.node(&d, &under(FIRST, 12)).unwrap().offset_bits, (total - 4) as u64 * 8);
    }
}
