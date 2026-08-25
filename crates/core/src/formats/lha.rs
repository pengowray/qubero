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
//! Levels 1 and 2 keep what will not fit in a chain of extended headers, each
//! holding the size of the one after it and the last saying zero. Level 1 then
//! counts those headers in the same number that gives the size of the
//! compressed data, so where the data starts is not known until the chain has
//! been walked. Walking it is what a list ending at a field value is for, and
//! the size of an element coming from the element before it is what `Prev` is
//! for; the data is then what is left of the number once the chain is off it.
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

/// What an extended header carries. The name being one of these is the whole
/// reason the later levels have them: a level 0 header could hold 255 bytes of
/// path and no more.
const EXTENDED: &[(i128, &str)] = &[
    (0x00, "header crc"),
    (0x01, "filename"),
    (0x02, "directory"),
    (0x3f, "comment"),
    (0x40, "ms-dos attributes"),
    (0x41, "windows timestamps"),
    (0x42, "file size"),
    (0x50, "unix permission"),
    (0x51, "unix owner"),
    (0x52, "unix group name"),
    (0x53, "unix user name"),
    (0x54, "unix modified time"),
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
                    T::switch(E::peek_at(E::lit(19 * 8), 8), vec![(2, level2())], header()),
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
            // In level 0 this is the compressed data and nothing else. In
            // level 1 it is what a reader has to step over to reach the next
            // entry, which is the data plus every extended header after this
            // one. The two are told apart below.
            ("packed_size", T::u32(Little)),
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
            // and a chain of extended headers.
            ("rest", T::switch(E::field("level"), vec![(1, level1_tail())], T::bytes(E::lit(0)))),
            // What the number above meant, now that the headers it counted
            // have been read: the tail is the operating system byte, the size
            // of the first extended header, and then the chain itself, so
            // taking those three bytes back off leaves what the chain came to.
            (
                "compressed_size",
                T::switch(
                    E::field("level"),
                    vec![(1, T::computed(E::field("packed_size").sub(E::size_of("rest")).add(E::lit(3))))],
                    T::computed(E::field("packed_size")),
                ),
            ),
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
            // The same chain level 1 has, in the room the header size leaves
            // for it. A level 2 header may be padded out to an even length,
            // and what the chain does not fill reads as the gap it is.
            ("extensions", T::sized(E::field("header_bytes").sub(E::lit(26)), extended_headers())),
            ("data", T::bytes(E::field("compressed_size"))),
        ],
    )
}

/// What level 1 puts after the CRC: the system it was written on, and then a
/// chain of extended headers that runs until one says nothing follows it.
///
/// This is where the base header ends. `header_size` counts up to and
/// including the size of the first extended header, and the chain itself sits
/// past that, which is why the data cannot be placed until the chain has been
/// walked.
fn level1_tail() -> T {
    T::structure(
        "Level1Tail",
        vec![
            ("os", T::enumeration("Os", T::u8(), OS)),
            ("next_header_size", T::u16(Little)),
            ("extensions", extended_headers()),
        ],
    )
}

/// The chain of extended headers. Each one holds its own contents and then the
/// size of the one after it, so a header is as long as the number before it
/// said, and a size of zero is where the chain stops.
///
/// The size of the first is the `next_header_size` in the header above; every
/// other takes it from the element before. That is what `Prev` is for, and it
/// answers zero outside a list and for the first element, which is exactly
/// when the field above is the right answer instead.
fn extended_headers() -> T {
    let header = T::structure_named(
        "Extended",
        "kind",
        "data",
        vec![
            ("size", T::computed(E::prev("next_size").or(E::field("next_header_size")))),
            ("kind", T::enumeration("ExtendedKind", T::u8(), EXTENDED)),
            // The size counts the kind byte and the two bytes of the next
            // size, neither of which is contents.
            ("data", T::bytes(E::field("size").sub(E::lit(3)))),
            ("next_size", T::u16(Little)),
        ],
    )
    .counted_as("header");
    // A first size of zero means there is no chain at all.
    T::switch(
        E::field("next_header_size"),
        vec![(0, T::bytes(E::lit(0)))],
        T::repeat(header, Until::FieldBytes { field: "next_size".into(), bytes: vec![0, 0] }),
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

    /// A level 1 entry whose name sits in the base header and whose extended
    /// headers carry a directory and a CRC. `skip_size` is what the format
    /// stores: the data plus every extended header after the first size.
    fn level1(name: &str, extended: &[(u8, &[u8])], data: &[u8]) -> Vec<u8> {
        // The chain, back to front, so each header can hold the size of the
        // one after it. The last says zero.
        let mut chain = Vec::new();
        let mut next = 0u16;
        for (kind, body) in extended.iter().rev() {
            let mut one = vec![*kind];
            one.extend_from_slice(body);
            one.extend_from_slice(&next.to_le_bytes());
            next = one.len() as u16;
            let mut joined = one;
            joined.append(&mut chain);
            chain = joined;
        }
        // `next` is now the size of the first header, which the base one holds.
        let first = next;

        let mut h = Vec::new();
        h.extend_from_slice(b"-lh5-");
        h.extend_from_slice(&((data.len() + chain.len()) as u32).to_le_bytes());
        h.extend_from_slice(&(data.len() as u32 * 3).to_le_bytes());
        h.extend_from_slice(&0u32.to_le_bytes());
        h.push(0x20);
        h.push(1); // level 1
        h.push(name.len() as u8);
        h.extend_from_slice(name.as_bytes());
        h.extend_from_slice(&0u16.to_le_bytes()); // crc
        h.push(b'M'); // written on ms-dos
        h.extend_from_slice(&first.to_le_bytes());

        let mut v = vec![h.len() as u8, 0];
        v.extend_from_slice(&h);
        v.extend_from_slice(&chain);
        v.extend_from_slice(data);
        v
    }

    #[test]
    fn a_level_1_chain_is_walked_and_taken_off_the_size_that_counted_it() {
        let extended: &[(u8, &[u8])] = &[(0x02, b"GAMES\\"), (0x00, &[0x12, 0x34])];
        let mut v = level1("DATA.BIN", extended, &[0xee; 20]);
        v.push(0);

        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(lha());
        let header = ev.node(&d, &[0, 0, 1]).unwrap();
        assert_eq!(header.type_name, "Header");
        assert_eq!(ev.node(&d, &[0, 0, 1, 8]).unwrap().value, Value::Str("DATA.BIN".into()));

        // Two extended headers, each as long as the size before it said.
        let chain = ev.node(&d, &[0, 0, 1, 10, 2]).unwrap();
        assert_eq!(chain.child_count, 2);
        assert_eq!(
            ev.node(&d, &[0, 0, 1, 10, 2, 0, 1]).unwrap().value,
            Value::Enum { raw: 2, name: Some("directory".into()), hex: false }
        );
        assert_eq!(ev.node(&d, &[0, 0, 1, 10, 2, 0, 2]).unwrap().size_bits, 6 * 8);
        assert_eq!(
            ev.node(&d, &[0, 0, 1, 10, 2, 1, 1]).unwrap().value,
            Value::Enum { raw: 0, name: Some("header crc".into()), hex: false }
        );
        // The last one says nothing follows it, which is what ends the chain.
        assert_eq!(ev.node(&d, &[0, 0, 1, 10, 2, 1, 3]).unwrap().value, Value::UInt(0));

        // The stored number counted the chain; the data is what is left.
        // Nine bytes for the first header, five for the second.
        let chain_bytes = 9 + 5;
        assert_eq!(ev.node(&d, &[0, 0, 1, 2]).unwrap().value, Value::UInt(20 + chain_bytes as u128));
        assert_eq!(ev.node(&d, &[0, 0, 1, 11]).unwrap().value, Value::Int(20));
        let data = ev.node(&d, &[0, 0, 1, 12]).unwrap();
        assert_eq!(data.size_bits, 20 * 8);
        // And the entry after it starts where the data ends, which is the
        // whole point of the number counting what it counts.
        assert_eq!(ev.node(&d, &[0]).unwrap().child_count, 2);
    }

    #[test]
    fn a_level_1_entry_with_no_extended_headers_still_places_its_data() {
        let mut v = level1("PLAIN.TXT", &[], &[0x55; 8]);
        v.push(0);
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(lha());
        assert_eq!(ev.node(&d, &[0, 0, 1, 10, 1]).unwrap().value, Value::UInt(0));
        assert_eq!(ev.node(&d, &[0, 0, 1, 10, 2]).unwrap().size_bits, 0);
        assert_eq!(ev.node(&d, &[0, 0, 1, 11]).unwrap().value, Value::Int(8));
        assert_eq!(ev.node(&d, &[0, 0, 1, 12]).unwrap().size_bits, 8 * 8);
        assert_eq!(ev.node(&d, &[0]).unwrap().child_count, 2);
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
        assert_eq!(ev.node(&d, &[0, 0, 1, 12]).unwrap().size_bits, 12 * 8);
        assert_eq!(ev.node(&d, &[0, 1, 1, 8]).unwrap().value, Value::Str("DISK.ID".into()));
        assert_eq!(ev.node(&d, &[0, 1, 1, 12]).unwrap().size_bits, 4 * 8);
        // The byte that ends the archive is an entry with nothing after it.
        assert_eq!(ev.node(&d, &[0, 2]).unwrap().size_bits, 8);
    }

    #[test]
    fn a_level_2_header_is_told_by_the_byte_twenty_along() {
        // A name held in an extended header, which is where level 2 puts it.
        // One extended header: a type byte, the name, and the size of the
        // header after it, which is zero because there is none.
        let mut extension = vec![0x01u8];
        extension.extend_from_slice(b"LONGNAME.TXT");
        extension.extend_from_slice(&0u16.to_le_bytes());
        assert_eq!(extension.len(), 15);
        // Twenty-four bytes of base header, the size of the first extended
        // header, and then the chain.
        let total = 26 + extension.len();

        let mut v = (total as u16).to_le_bytes().to_vec();
        v.extend_from_slice(b"-lh7-");
        v.extend_from_slice(&40u32.to_le_bytes()); // compressed
        v.extend_from_slice(&100u32.to_le_bytes()); // original
        v.extend_from_slice(&1_700_000_000u32.to_le_bytes());
        v.push(0x20); // reserved
        v.push(2); // level
        v.extend_from_slice(&0u16.to_le_bytes()); // crc
        v.push(b'U'); // written on unix
        v.extend_from_slice(&(extension.len() as u16).to_le_bytes());
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
        assert_eq!(ev.node(&d, &[0, 0, 1, 11]).unwrap().size_bits, extension.len() as u64 * 8);
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
