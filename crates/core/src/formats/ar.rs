//! `ar` archives: a magic line, then members one after another, each with a
//! sixty-byte header of text.
//!
//! It is the oldest archive format Unix still uses, and two things keep it in
//! use. A static library is one of these with object files in it and a table
//! of the symbols they define. A Debian package is one of these with exactly
//! three members: a version stamp, the package's control files, and the files
//! it installs, the last two being tar archives that are almost always
//! compressed. So the same template reads both, and `deb` is the name it goes
//! by when the first member says which it is.
//!
//! Every number in the header is written as digits, left aligned in a field
//! padded with spaces, and the mode is written in octal because that is how a
//! Unix mode is read. A field nobody filled in is spaces all the way, which
//! GNU `ar` leaves in the header of its long-name table, so a field starting
//! with a space is read as the spaces it holds rather than as a number that
//! was never written.
//!
//! What is not read here: a BSD archive writes a name too long for the header
//! into the front of the member's own data and puts `#1/` and its length in
//! the name field. The name is then in the data, and this template leaves it
//! there, because splitting it would mean reading a number out of the middle
//! of another field. A GNU archive has no such problem: its long names are all
//! in one member and the header holds an offset into it.

use crate::template::{Encoding, Endian, Expr as E, StrLen, Template, Ty as T, Until};

/// The line every one of these starts with.
pub const MAGIC: &[u8] = b"!<arch>\n";

/// The first member of a Debian package, whose contents are the format
/// version: `2.0` and a newline, for every package anyone has seen.
pub const DEBIAN_BINARY: &str = "debian-binary";

pub fn ar() -> Template {
    Template::new("ar", archive("ArArchive"))
}

/// The same archive, under the name the package it is goes by. Nothing in the
/// bytes differs: what makes a `.deb` a `.deb` is which members are in it.
pub fn deb() -> Template {
    Template::new("deb", archive("DebPackage"))
}

fn archive(name: &str) -> T {
    T::structure(name, vec![("magic", T::magic(MAGIC)), ("members", T::repeat(member(), Until::End))])
}

fn member() -> T {
    T::structure_named(
        "ArMember",
        "name",
        "data",
        vec![
            // Padded with spaces, and in a System V archive ending with a
            // slash, which is there to keep a trailing space in a name from
            // being lost. A name of `/` and a number is an offset into the
            // long-name table, and a name of `//` or `/` is that table or the
            // symbol table itself.
            ("name", T::utf8_padded(E::lit(16), b' ')),
            ("mtime", number(12)),
            ("uid", number(6)),
            ("gid", number(6)),
            ("mode", mode()),
            ("size", T::decimal(StrLen::Padded { size: E::lit(10), pad: b' ' })),
            ("end", T::magic(b"`\n")),
            ("data", data()),
            // A member of an odd length is followed by one byte, so the next
            // header starts on an even one.
            ("padding", T::bytes(E::field("size").pad_to(2))),
        ],
    )
    .counted_as("member")
}

/// A number written as decimal digits, or the spaces of a field nobody wrote.
fn number(width: i128) -> T {
    blank_or(width, T::decimal(StrLen::Padded { size: E::lit(width), pad: b' ' }))
}

/// The same, in octal, which is the only base a Unix mode is written in.
fn mode() -> T {
    blank_or(8, T::octal(StrLen::Padded { size: E::lit(8), pad: b' ' }))
}

/// `digits`, unless the field starts with a space. Every one of these fields
/// is left aligned, so a space at the front means there is nothing in it, and
/// an empty field is not a number: reading it as one would report an error
/// where the archive did nothing wrong.
fn blank_or(width: i128, digits: T) -> T {
    T::switch(
        E::peek(8, Endian::Big),
        vec![(b' ' as i128, T::text(StrLen::Fixed(E::lit(width)), Encoding::Ascii))],
        digits,
    )
}

/// What a member holds, in the window its own header measures out. Three
/// names mean something to every archive that has them; the rest is the file
/// that was put in, named by what it starts with.
fn data() -> T {
    T::sized(
        E::field("size"),
        T::matches(
            E::field("name"),
            vec![
                ("//", long_names()),
                ("/", symbol_table(T::u32(Endian::Big))),
                ("/SYM64/", symbol_table(T::u64(Endian::Big))),
                (
                    DEBIAN_BINARY,
                    T::structure(
                        "DebianBinary",
                        vec![("version", T::text(StrLen::Fixed(E::Remaining), Encoding::Ascii))],
                    ),
                ),
            ],
            contents(),
        ),
    )
}

/// The long-name table: every name too long for a header, one per line, each
/// ending with the slash that ends a name in this format.
fn long_names() -> T {
    T::structure(
        "ArLongNames",
        vec![(
            "names",
            T::repeat(T::text(StrLen::Terminated { end: b'\n', or_end: true }, Encoding::Utf8), Until::End)
                .counted_as("name"),
        )],
    )
}

/// The symbol table a linker reads first: how many symbols there are, where
/// the member defining each one starts, and then the names in the same order.
/// `/SYM64/` is the same table with room for an archive past four gigabytes,
/// so the only difference is how wide a number is.
fn symbol_table(offset: T) -> T {
    T::structure(
        "ArSymbolTable",
        vec![
            ("count", offset.clone()),
            ("offsets", T::array(offset, E::field("count"))),
            ("names", T::repeat(T::cstr(), Until::End).counted_as("name")),
        ],
    )
}

/// A member's file, named by its first bytes where those say something. A
/// Debian package's two tar archives are compressed, and naming which
/// compressor wrote one is as far as this goes: reading inside it would mean
/// decompressing it.
fn contents() -> T {
    let stream = |name: &str| T::structure(name, vec![("data", T::bytes(E::Remaining))]);
    let named = T::switch(
        E::peek(16, Endian::Big),
        vec![
            (0x1f8b, stream("GzipStream")),
            (0xfd37, stream("XzStream")),
            (0x28b5, stream("ZstdStream")),
            (0x425a, stream("Bzip2Stream")),
            (0x5d00, stream("LzmaStream")),
            // An object file, which is what a static library is full of.
            (0x7f45, stream("ElfObject")),
        ],
        T::bytes(E::Remaining),
    );
    // A member with nothing in it has no first bytes to look at, and looking
    // anyway is an error rather than an answer.
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

    /// A header as `ar` writes one: every field left aligned in its width.
    fn header(name: &str, size: usize) -> Vec<u8> {
        let mut v = format!("{name:<16}{:<12}{:<6}{:<6}{:<8}{size:<10}", 1_700_000_000u64, 0, 0, "100644").into_bytes();
        v.extend_from_slice(b"`\n");
        v
    }

    fn member(name: &str, data: &[u8]) -> Vec<u8> {
        let mut v = header(name, data.len());
        v.extend_from_slice(data);
        if data.len() % 2 == 1 {
            v.push(b'\n');
        }
        v
    }

    fn archive(members: &[Vec<u8>]) -> Vec<u8> {
        let mut v = MAGIC.to_vec();
        for m in members {
            v.extend_from_slice(m);
        }
        v
    }

    #[test]
    fn a_member_is_its_header_and_as_many_bytes_as_the_header_says() {
        let v = archive(&[member("hello.o/", b"odd"), member("world.o/", b"even")]);
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(ar());
        assert_eq!(e.node(&d, &[1]).unwrap().child_count, 2);
        assert_eq!(e.node(&d, &[1, 0, 0]).unwrap().value, Value::Str("hello.o/".into()));
        assert_eq!(e.node(&d, &[1, 0, 1]).unwrap().value.as_int(), Some(1_700_000_000));
        // Written in octal, so the mode a Unix reader knows as 644.
        assert_eq!(e.node(&d, &[1, 0, 4]).unwrap().value.as_int(), Some(0o100644));
        assert_eq!(e.node(&d, &[1, 0, 5]).unwrap().value.as_int(), Some(3));
        // Three bytes of file, so one of padding, and none after four.
        assert_eq!(e.node(&d, &[1, 0, 8]).unwrap().size_bits, 8);
        assert_eq!(e.node(&d, &[1, 1, 8]).unwrap().size_bits, 0);
    }

    /// GNU `ar` writes the header of its long-name table with everything but
    /// the size left blank.
    #[test]
    fn a_field_of_spaces_is_the_spaces_and_not_a_broken_number() {
        let names = b"a-name-too-long-for-a-header.o/\nanother-one.o/\n";
        let mut m = format!("{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}", "//", "", "", "", "", names.len()).into_bytes();
        m.extend_from_slice(b"`\n");
        m.extend_from_slice(names);
        let d = Document::new(MemSource(archive(&[m])));
        let mut e = Evaluator::new(ar());
        assert_eq!(e.node(&d, &[1, 0, 1]).unwrap().value, Value::Str(" ".repeat(12)));
        let table = e.node(&d, &[1, 0, 7, 0]).unwrap();
        assert_eq!(table.child_count, 2);
        assert_eq!(e.node(&d, &[1, 0, 7, 0, 0]).unwrap().value, Value::Str("a-name-too-long-for-a-header.o/".into()));
    }

    #[test]
    fn the_symbol_table_names_as_many_symbols_as_it_places() {
        let mut data = 2u32.to_be_bytes().to_vec();
        data.extend_from_slice(&8u32.to_be_bytes());
        data.extend_from_slice(&76u32.to_be_bytes());
        data.extend_from_slice(b"main\0puts\0");
        let d = Document::new(MemSource(archive(&[member("/", &data)])));
        let mut e = Evaluator::new(ar());
        assert_eq!(e.node(&d, &[1, 0, 7, 0]).unwrap().value.as_int(), Some(2));
        assert_eq!(e.node(&d, &[1, 0, 7, 1]).unwrap().child_count, 2);
        assert_eq!(e.node(&d, &[1, 0, 7, 2, 1]).unwrap().value, Value::Str("puts".into()));
    }

    /// A Debian package: the version stamp, then two tar archives that say
    /// nothing about themselves except which compressor wrote them.
    #[test]
    fn a_package_reads_its_stamp_and_names_the_streams_after_it() {
        let v = archive(&[
            member(DEBIAN_BINARY, b"2.0\n"),
            member("control.tar.gz", b"\x1f\x8b\x08\x00control"),
            member("data.tar.xz", b"\xfd7zXZ\x00data"),
        ]);
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(deb());
        assert_eq!(e.node(&d, &[1, 0, 7, 0]).unwrap().value, Value::Str("2.0\n".into()));
        assert!(e.node(&d, &[1, 1, 7]).unwrap().type_name.contains("Gzip"));
        assert!(e.node(&d, &[1, 2, 7]).unwrap().type_name.contains("Xz"));
    }

    /// An empty member has no first bytes to name it by, and reading one is
    /// not an error.
    #[test]
    fn an_empty_member_keeps_its_place() {
        let d = Document::new(MemSource(archive(&[member("empty/", b"")])));
        let mut e = Evaluator::new(ar());
        assert_eq!(e.node(&d, &[1, 0, 7]).unwrap().size_bits, 0);
    }
}
