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

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T};

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

pub fn gzip() -> Template {
    Template::new(
        "gzip",
        T::structure(
            "GZIP",
            vec![
                ("magic", T::magic(b"\x1f\x8b")),
                ("method", T::enumeration("Method", T::u8(), &[(8, "deflate")])),
                ("flg", T::flags("Flags", T::u8(), FLAGS)),
                // Seconds since 1970, or zero when the compressor had no time
                // to give: gzip writes zero for input it read from a pipe.
                ("mtime", T::u32(Little)),
                ("extra_flags", T::enumeration("ExtraFlags", T::u8(), &[(2, "best compression"), (4, "fastest")])),
                ("os", T::enumeration("Os", T::u8(), OS)),
                ("extra", T::switch(bit(2), vec![(1, extra_field())], T::bytes(E::lit(0)))),
                ("name", T::switch(bit(3), vec![(1, optional_string())], T::bytes(E::lit(0)))),
                ("comment", T::switch(bit(4), vec![(1, optional_string())], T::bytes(E::lit(0)))),
                ("header_crc", T::bytes(bit(1).mul(E::lit(2)))),
                // Deflate, which nothing here unpacks. The last eight bytes
                // are the trailer, so the stream is everything before them.
                ("compressed", T::bytes(E::Remaining.sub(E::lit(8)))),
                ("crc32", T::u32(Little)),
                // The size of what was compressed, modulo four gigabytes,
                // which is why a large file's number looks wrong.
                ("original_size", T::u32(Little)),
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

    #[test]
    fn a_name_is_read_only_when_its_bit_is_set() {
        let d = Document::new(MemSource(member(0x08, b"hello.txt\0")));
        let mut ev = Evaluator::new(gzip());
        assert_eq!(ev.node(&d, &[7]).unwrap().value, Value::Str("hello.txt".into()));
        assert_eq!(ev.node(&d, &[8]).unwrap().size_bits, 0);
        assert_eq!(ev.node(&d, &[10]).unwrap().size_bits, 2 * 8);

        // The same file without the flag: the name is not there, and the
        // deflate stream starts where the header ends.
        let d = Document::new(MemSource(member(0, b"")));
        let mut ev = Evaluator::new(gzip());
        assert_eq!(ev.node(&d, &[7]).unwrap().size_bits, 0);
        assert_eq!(ev.node(&d, &[10]).unwrap().offset_bits, 10 * 8);
    }

    #[test]
    fn the_trailer_is_the_last_eight_bytes_whatever_came_before() {
        let d = Document::new(MemSource(member(0x08, b"a\0")));
        let mut ev = Evaluator::new(gzip());
        let size = ev.node(&d, &[12]).unwrap();
        assert_eq!(size.value, Value::UInt(0));
        assert_eq!(size.offset_bits, (10 + 2 + 2 + 4) * 8);
    }
}
