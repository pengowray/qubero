//! LHA/LZH: one header per file, each followed by the compressed bytes it
//! describes, and a zero byte where the next header would be to end the
//! archive.
//!
//! The header comes in three levels and they disagree about their own start.
//! Levels 0 and 1 open with a single byte of header size and a checksum;
//! level 2 uses those same two bytes as a sixteen-bit size and has no checksum
//! at all. What settles it is the level byte at offset 20, which sits after
//! the fields whose meaning it decides, so no field read so far can be
//! switched on. Looking ahead at it without reading it is what the expression
//! language calls a peek, and one that starts further on than here is what
//! this needs: the switch keys on the byte twenty along, and each level then
//! reads its own layout from the beginning.
//!
//! One thing is still guessed at. The end of the archive is a header size of
//! zero, and for a level 2 header the byte holding it is only the low half of
//! a sixteen-bit size, so an archive whose last member has a header of exactly
//! 256 or 512 bytes ends early. That is what the format leaves ambiguous, and
//! every tool that reads these files has the same problem.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};

/// The method five characters name, which is also the window size: `-lh5-`
/// compresses against the last 8K, `-lh7-` against the last 64K.
const METHOD: &[(i128, &str)] = &[
    (0x2d6c_6830_2di128, "stored"),
    (0x2d6c_6831_2di128, "lzhuff 4k"),
    (0x2d6c_6834_2di128, "lzhuff 4k, static"),
    (0x2d6c_6835_2di128, "lzhuff 8k"),
    (0x2d6c_6836_2di128, "lzhuff 32k"),
    (0x2d6c_6837_2di128, "lzhuff 64k"),
    (0x2d6c_6864_2di128, "directory"),
];

/// The system that wrote the archive, as a single character.
const OS: &[(i128, &str)] = &[
    (b'M' as i128, "ms-dos"),
    (b'2' as i128, "os/2"),
    (b'9' as i128, "os9"),
    (b'K' as i128, "os/68k"),
    (b'3' as i128, "os/386"),
    (b'H' as i128, "human68k"),
    (b'U' as i128, "unix"),
    (b'C' as i128, "cp/m"),
    (b'F' as i128, "flex"),
    (b'm' as i128, "macintosh"),
    (b'J' as i128, "java"),
    (b'A' as i128, "amiga"),
    (b'w' as i128, "windows"),
];

pub fn lha() -> Template {
    Template::new(
        "lha",
        T::structure(
            "LHA",
            // A header size of zero is where the archive ends, and it is the
            // only thing marking the end.
            vec![("entries", T::repeat(entry(), Until::FieldBytes { field: "header_size".into(), bytes: vec![0] }))],
        ),
    )
}

/// One archive member. The zero byte that ends the archive is an element of
/// this list too, and there is nothing after it, so a header size of zero has
/// no body: reading the rest of a header that is not there would run off the
/// end of the file.
fn entry() -> T {
    T::structure_named(
        "Entry",
        "",
        "header",
        vec![
            // In levels 0 and 1, counted from the byte after the checksum, so
            // the header is this plus two. In level 2 it is the low half of a
            // sixteen-bit size, which the level 2 layout puts back together.
            ("header_size", T::u8()),
            (
                "header",
                T::switch(
                    E::field("header_size"),
                    vec![(0, T::bytes(E::lit(0)))],
                    // Nineteen bytes on from here is offset 20 of the entry,
                    // which is the level byte in all three layouts.
                    T::switch(E::peek_at(19 * 8, 8), vec![(2, level2())], header()),
                ),
            ),
        ],
    )
    .counted_as("entry")
}

/// Levels 0 and 1.
fn header() -> T {
    T::structure_named(
        "Header",
        "name",
        "",
        vec![
            ("header_checksum", T::u8()),
            ("method", T::enumeration("Method", T::UInt { bits: 40, endian: Big }, METHOD)),
            ("compressed_size", T::u32(Little)),
            ("original_size", T::u32(Little)),
            // Packed the way MS-DOS packed a directory entry: seconds in
            // twos, and years counted from 1980.
            ("timestamp", T::u32(Little)),
            ("attribute", T::u8()),
            ("level", T::u8()),
            ("name_length", T::u8()),
            ("name", T::text(StrLen::Fixed(E::field("name_length")), Encoding::Cp437)),
            ("crc", T::u16(Little)),
            // Level 0 stops after the CRC; level 1 adds the operating system
            // and a chain of extended headers, which the rest of the header
            // covers here without being taken apart.
            ("rest", T::switch(E::field("level"), vec![(1, os_and_extensions())], T::bytes(E::lit(0)))),
            ("data", T::bytes(E::field("compressed_size"))),
        ],
    )
}

/// Level 2, which threw out the checksum, gave the header a sixteen-bit size,
/// and moved the name out of the base header into an extended one. The name is
/// the reason: an extended header can be as long as it likes, and a level 0
/// header could hold 255 bytes of path and no more.
///
/// The size byte the entry already read is the low half of that sixteen-bit
/// number, so the high half is read here and the two are put together. The
/// timestamp changed meaning too: level 2 writes seconds since 1970 rather
/// than the packed MS-DOS date the two levels before it used.
fn level2() -> T {
    T::structure(
        "Level2Header",
        vec![
            ("size_high", T::u8()),
            ("header_bytes", T::computed(E::field("size_high").mul(E::lit(256)).add(E::field("header_size")))),
            ("method", T::enumeration("Method", T::UInt { bits: 40, endian: Big }, METHOD)),
            ("compressed_size", T::u32(Little)),
            ("original_size", T::u32(Little)),
            ("timestamp", T::u32(Little)),
            ("reserved", T::u8()),
            ("level", T::u8()),
            ("crc", T::u16(Little)),
            ("os", T::enumeration("Os", T::u8(), OS)),
            ("next_header_size", T::u16(Little)),
            // The chain of extended headers, one of which holds the name.
            // They fill the rest of the header, whose size is now known.
            ("extensions", T::bytes(E::field("header_bytes").sub(E::lit(26)))),
            ("data", T::bytes(E::field("compressed_size"))),
        ],
    )
}

/// What level 1 puts after the CRC. The extended headers that follow have
/// their own sizes and are not split apart here; what is certain is that the
/// header ends where `header_size` said it would.
fn os_and_extensions() -> T {
    T::structure(
        "Level1Tail",
        vec![
            ("os", T::enumeration("Os", T::u8(), OS)),
            // The header runs to header_size + 1 bytes past the size byte, and
            // everything up to and including the operating system byte has
            // taken 24 of them plus the name.
            ("extensions", T::bytes(E::field("header_size").sub(E::size_of("name")).sub(E::lit(23)))),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn level0(name: &str, data: &[u8]) -> Vec<u8> {
        let mut h = Vec::new();
        h.extend_from_slice(b"-lh5-");
        h.extend_from_slice(&(data.len() as u32).to_le_bytes());
        h.extend_from_slice(&(data.len() as u32 * 3).to_le_bytes());
        h.extend_from_slice(&0u32.to_le_bytes());
        h.push(0x20); // an ordinary file
        h.push(0); // level 0
        h.push(name.len() as u8);
        h.extend_from_slice(name.as_bytes());
        h.extend_from_slice(&0u16.to_le_bytes()); // crc

        let mut v = vec![h.len() as u8, 0];
        v.extend_from_slice(&h);
        v.extend_from_slice(data);
        v
    }

    #[test]
    fn each_entry_names_its_method_and_the_bytes_after_it() {
        let mut v = level0("README.TXT", &[0xaa; 12]);
        v.extend_from_slice(&level0("DISK.ID", &[0xbb; 4]));
        v.push(0); // the header size that ends the archive

        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(lha());
        let entries = ev.node(&d, &[0]).unwrap();
        // Two entries, and the zero byte that ends the list.
        assert_eq!(entries.child_count, 3);
        assert_eq!(
            ev.node(&d, &[0, 0, 1, 1]).unwrap().value,
            Value::Enum { raw: 0x2d6c_6835_2d, name: Some("lzhuff 8k".into()), hex: false }
        );
        assert_eq!(ev.node(&d, &[0, 0, 1, 8]).unwrap().value, Value::Str("README.TXT".into()));
        assert_eq!(ev.node(&d, &[0, 0, 1, 2]).unwrap().value, Value::UInt(12));
        assert_eq!(ev.node(&d, &[0, 0, 1, 11]).unwrap().size_bits, 12 * 8);
        assert_eq!(ev.node(&d, &[0, 1, 1, 8]).unwrap().value, Value::Str("DISK.ID".into()));
        assert_eq!(ev.node(&d, &[0, 1, 1, 11]).unwrap().size_bits, 4 * 8);
        // The byte that ends the archive is an entry with nothing after it.
        assert_eq!(ev.node(&d, &[0, 2]).unwrap().size_bits, 8);
    }

    #[test]
    fn a_level_2_header_is_told_by_the_byte_twenty_along() {
        // A name held in an extended header, which is where level 2 puts it.
        // Its own size word, a type byte, the name, and the size of the
        // header after it, which is zero because there is none.
        let mut extension = 17u16.to_le_bytes().to_vec();
        extension.push(0x01); // a filename header
        extension.extend_from_slice(b"LONGNAME.TXT");
        extension.extend_from_slice(&0u16.to_le_bytes());
        assert_eq!(extension.len(), 17);
        // Twenty-four bytes of base header, then the chain.
        let total = 24 + extension.len();

        let mut v = (total as u16).to_le_bytes().to_vec();
        v.extend_from_slice(b"-lh7-");
        v.extend_from_slice(&40u32.to_le_bytes()); // compressed
        v.extend_from_slice(&100u32.to_le_bytes()); // original
        v.extend_from_slice(&1_700_000_000u32.to_le_bytes());
        v.push(0x20); // reserved
        v.push(2); // level
        v.extend_from_slice(&0u16.to_le_bytes()); // crc
        v.push(b'U'); // written on unix
        v.extend_from_slice(&extension);
        v.extend_from_slice(&[0xcc; 40]);
        v.push(0); // the end of the archive

        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(lha());
        assert_eq!(ev.node(&d, &[0]).unwrap().child_count, 2);
        let header = ev.node(&d, &[0, 0, 1]).unwrap();
        assert_eq!(header.type_name, "Level2Header");
        // The two halves of the size, put back together.
        assert_eq!(ev.node(&d, &[0, 0, 1, 1]).unwrap().value, Value::Int(total as i128));
        assert_eq!(
            ev.node(&d, &[0, 0, 1, 2]).unwrap().value,
            Value::Enum { raw: 0x2d6c_6837_2d, name: Some("lzhuff 64k".into()), hex: false }
        );
        assert_eq!(ev.node(&d, &[0, 0, 1, 9]).unwrap().value, Value::Enum { raw: b'U' as i128, name: Some("unix".into()), hex: false });
        assert_eq!(ev.node(&d, &[0, 0, 1, 11]).unwrap().size_bits, (extension.len() as u64 - 2) * 8);
        assert_eq!(ev.node(&d, &[0, 0, 1, 12]).unwrap().size_bits, 40 * 8);
        // And the level 0 archive above still reads as level 0.
        let d0 = Document::new(MemSource({
            let mut v = level0("A.TXT", &[0; 2]);
            v.push(0);
            v
        }));
        let mut ev0 = Evaluator::new(lha());
        assert_eq!(ev0.node(&d0, &[0, 0, 1]).unwrap().type_name, "Header");
    }
}
