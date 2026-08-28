//! cpio archives in the `newc` format, which is what an initramfs is.
//!
//! Every number in the header is written as eight hexadecimal digits, which
//! is what makes the format portable and what makes it awkward: the length of
//! a file is text, and the padding after it has to be worked out from that
//! text read as a number.
//!
//! An initramfs is one of these, usually compressed, and often two of them
//! concatenated: the first holds the CPU microcode and is never compressed,
//! because the kernel reads it before it has a decompressor, and the second
//! is everything else. So the archive here ends at its trailer and whatever
//! was appended after it is named by its own first bytes and left as a run.
//! Reading inside a compressed one means decompressing it, which is a thing
//! this editor does not yet do anywhere.

use crate::template::{Encoding, Expr as E, StrLen, Template, Ty as T, Until};

/// The name of the entry that ends an archive. It is a file like any other,
/// with no data and a name nothing would use.
const TRAILER: &[u8] = b"TRAILER!!!\0";

pub fn cpio() -> Template {
    Template::new(
        "cpio",
        T::structure(
            "CpioArchive",
            vec![
                (
                    "entries",
                    T::repeat(entry(), Until::FieldBytes { field: "name".into(), bytes: TRAILER.to_vec() }),
                ),
                ("appended", appended()),
            ],
        ),
    )
}

fn entry() -> T {
    T::structure_named(
        "CpioEntry",
        "name",
        "data",
        vec![
            // `070701`, or `070702` for the same thing with a checksum of the
            // file's bytes in `c_check`, which is otherwise zero.
            ("c_magic", T::text(StrLen::Fixed(E::lit(6)), Encoding::Ascii)),
            ("c_ino", number()),
            ("c_mode", number()),
            ("c_uid", number()),
            ("c_gid", number()),
            ("c_nlink", number()),
            ("c_mtime", number()),
            ("c_filesize", number()),
            ("c_devmajor", number()),
            ("c_devminor", number()),
            ("c_rdevmajor", number()),
            ("c_rdevminor", number()),
            // The name's length, counting the NUL that ends it.
            ("c_namesize", number()),
            ("c_check", number()),
            // As long as `c_namesize` says, of which the value is everything
            // before the NUL: the terminator is counted in the length and is
            // not part of the name.
            ("name", T::text(StrLen::Padded { size: E::field("c_namesize"), pad: 0 }, Encoding::Utf8)),
            // The name is padded so that the file's bytes start on a
            // four-byte boundary, counting from the front of the header.
            ("name_padding", T::bytes(E::lit(HEADER).add(E::field("c_namesize")).pad_to(4))),
            ("data", T::bytes(E::field("c_filesize"))),
            ("data_padding", T::bytes(E::field("c_filesize").pad_to(4))),
        ],
    )
    .counted_as("entry")
}

/// How long the fixed part of a header is: six characters of magic and
/// thirteen numbers of eight digits each.
const HEADER: i128 = 6 + 13 * 8;

fn number() -> T {
    T::hex_digits(StrLen::Fixed(E::lit(8)))
}

/// What was concatenated after the trailer, named by what it starts with.
///
/// A kernel is handed one file and finds several archives in it, so the room
/// after the trailer is part of the format rather than junk at the end. What
/// is in it cannot be read here: it is a compressed stream, and naming which
/// compressor wrote it is as far as this goes.
fn appended() -> T {
    let stream = |name: &str| T::structure(name, vec![("data", T::bytes(E::Remaining))]);
    let named = T::switch(
        E::peek(16, crate::template::Endian::Big),
        vec![
            (0x1f8b, stream("GzipStream")),
            (0xfd37, stream("XzStream")),
            (0x28b5, stream("ZstdStream")),
            (0x425a, stream("Bzip2Stream")),
            (0x0422, stream("Lz4Stream")),
            (0x894c, stream("LzoStream")),
            (0x5d00, stream("LzmaStream")),
            (0x3037, stream("CpioArchive")),
        ],
        // A run of padding a builder left between two archives is nothing to
        // name, and neither is anything else nobody recognises.
        T::bytes(E::Remaining),
    );
    // An archive that ends where the file ends has nothing to look at, and
    // looking anyway is an error rather than an answer.
    T::switch(E::lit(1).less_than(E::Remaining), vec![(1, named)], T::bytes(E::Remaining))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::Document,
        eval::{Evaluator, Value},
        source::MemSource,
    };

    fn entry_bytes(name: &[u8], data: &[u8]) -> Vec<u8> {
        let mut v = b"070701".to_vec();
        let mut number = |n: usize| v.extend_from_slice(format!("{n:08X}").as_bytes());
        for _ in 0..6 {
            number(0);
        }
        number(data.len()); // c_filesize
        for _ in 0..4 {
            number(0);
        }
        number(name.len() + 1); // c_namesize, counting the NUL
        number(0); // c_check
        v.extend_from_slice(name);
        v.push(0);
        while v.len() % 4 != 0 {
            v.push(0);
        }
        v.extend_from_slice(data);
        while v.len() % 4 != 0 {
            v.push(0);
        }
        v
    }

    fn trailer() -> Vec<u8> {
        entry_bytes(b"TRAILER!!!", b"")
    }

    #[test]
    fn an_entry_measures_by_numbers_written_as_digits() {
        let mut v = entry_bytes(b"init", b"#!/bin/sh\n");
        v.extend_from_slice(&trailer());
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(cpio());
        assert_eq!(e.node(&d, &[0]).unwrap().child_count, 2);
        assert_eq!(e.node(&d, &[0, 0, 14]).unwrap().value, Value::Str("init".into()));
        assert_eq!(e.node(&d, &[0, 0, 7]).unwrap().value.as_int(), Some(10));
        assert_eq!(e.node(&d, &[0, 0, 16]).unwrap().size_bits, 10 * 8);
        // 110 header bytes and five of name comes to 115, so one of padding.
        assert_eq!(e.node(&d, &[0, 0, 15]).unwrap().size_bits, 8);
        // Ten bytes of file, so two more to the next boundary.
        assert_eq!(e.node(&d, &[0, 0, 17]).unwrap().size_bits, 2 * 8);
    }

    /// The microcode archive an initramfs starts with, and the compressed one
    /// the kernel unpacks after it.
    #[test]
    fn what_follows_the_trailer_is_named_by_its_magic() {
        let mut v = entry_bytes(b"kernel/x86/microcode/AuthenticAMD.bin", b"\x01\x02\x03\x04");
        v.extend_from_slice(&trailer());
        v.extend_from_slice(b"\x1f\x8b\x08\x00rest of the image");
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(cpio());
        let after = e.node(&d, &[1]).unwrap();
        assert_eq!(after.size_bits, 21 * 8);
        assert!(after.type_name.contains("Gzip"), "not named as a gzip stream: {}", after.type_name);
    }

    /// An archive with nothing after it has an empty run at the end rather
    /// than a missing field.
    #[test]
    fn an_archive_that_ends_at_its_trailer_appends_nothing() {
        let d = Document::new(MemSource(trailer()));
        let mut e = Evaluator::new(cpio());
        assert_eq!(e.node(&d, &[0]).unwrap().child_count, 1);
        assert_eq!(e.node(&d, &[1]).unwrap().size_bits, 0);
    }
}
