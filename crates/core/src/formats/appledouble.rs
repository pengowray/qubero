//! AppleDouble, the file macOS leaves beside a copy when the destination
//! cannot hold the original's attributes: the `._` files inside a zip made
//! on a Mac.
//!
//! The front is a table of entries, each an id, an offset from the start of
//! the file and a length, and the rest is whatever the table points at. The
//! two an archive of a real file carries are the Finder info, which the
//! newer writer extends with the file's extended attributes in a block of
//! their own, and the resource fork, which is nearly always an empty one.
//! Neither id says where it points, so the table places the parts the way a
//! region's does, and each part is read in a window the entry's length cut,
//! which is what makes the offsets inside a resource fork mean what they
//! say. An entry of no length points at the end of the file and covers
//! nothing.

use crate::template::{Anchor, Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T};

/// The kinds of entry an AppleDouble file can carry, from the same list the
/// writer on macOS keeps.
const ENTRY: &[(i128, &str)] = &[
    (1, "data"),
    (2, "resource fork"),
    (3, "real name"),
    (4, "comment"),
    (5, "icon b/w"),
    (6, "icon color"),
    (7, "unused"),
    (8, "file dates"),
    (9, "finder info"),
    (10, "mac info"),
    (11, "prodos info"),
    (12, "msdos info"),
    (13, "afp name"),
    (14, "afp info"),
    (15, "afp dir id"),
];

pub fn appledouble() -> Template {
    Template::new("appledouble", body("AppleDouble", b"\x00\x05\x16\x07"))
}

/// AppleSingle: the same table, in a file that carries the data fork as one
/// more entry instead of leaving it beside. Rare next to AppleDouble, but
/// the only thing that differs is the magic.
pub fn applesingle() -> Template {
    Template::new("applesingle", body("AppleSingle", b"\x00\x05\x16\x00"))
}

/// The header both formats share: the magic that says which, the version,
/// the sixteen bytes macOS signs its name in, and the table of entries.
fn body(name: &'static str, magic: &'static [u8]) -> T {
    T::structure(
        name,
        vec![
            ("magic", T::magic(magic)),
            ("version", T::enumeration_hex("Version", T::u32(Big), &[(0x00010000, "1"), (0x00020000, "2")])),
            // All sixteen bytes, spaces and all: a padded field stops at the
            // first pad byte, which would cut "Mac OS X        " to "Mac".
            ("filler", T::text(StrLen::Fixed(E::lit(16)), Encoding::Ascii)),
            ("count", T::u16(Big)),
            ("entries", T::array(entry(), E::field("count"))),
            ("parts", T::pointer_list_sized("entries", &["offset"], Anchor::File, E::lit(0), part()).skipping_zero()),
        ],
    )
}

/// One row of the table: what the part is and where in the file it sits.
fn entry() -> T {
    T::inline_structure(
        "Entry",
        vec![
            ("id", T::enumeration("EntryID", T::u32(Big), ENTRY)),
            ("offset", T::u32(Big)),
            ("length", T::u32(Big)),
        ],
    )
    .counted_as("entry")
}

/// A part of the file, to the length its entry keeps, shaped by the id the
/// entry names rather than by any bytes of its own. An entry of no length
/// is left as nothing: the resource fork a zip made on a Mac carries is
/// usually this, an id and an offset at the end of the file with nothing
/// under it, and reading a header out of it would run off the end. A
/// Finder info entry of exactly thirty-two bytes is the older shape, with
/// no pad and no attributes after it, so it stops there for the same
/// reason.
fn part() -> T {
    T::switch(
        E::elem_field("entries", E::idx(), &["length"]),
        vec![
            (0, T::bytes(E::lit(0))),
            (
                32,
                T::sized(
                    E::lit(32),
                    T::switch(E::elem_field("entries", E::idx(), &["id"]), vec![(9, info())], T::bytes(E::Remaining)),
                ),
            ),
        ],
        T::sized(
            E::elem_field("entries", E::idx(), &["length"]),
            T::switch(
                E::elem_field("entries", E::idx(), &["id"]),
                vec![(9, finder_info()), (2, resource_fork())],
                T::bytes(E::Remaining),
            ),
        ),
    )
}

/// The Finder info entry: thirty-two bytes of Finder info, and after the
/// two pad bytes the writer keeps for alignment, the extended attributes
/// when the file carries any. The length the entry keeps counts the whole
/// of it, so the block sits inside the part rather than beside it. Older
/// files stop at the Finder info, so anything that is not the attribute
/// block's magic is left as the bytes it is.
fn finder_info() -> T {
    T::structure(
        "FinderInfo",
        vec![
            ("info", info()),
            ("pad", T::bytes(E::lit(2))),
            (
                "more",
                T::switch(
                    E::peek(32, Big),
                    vec![(0x41545452, attrs())],
                    T::bytes(E::Remaining),
                ),
            ),
        ],
    )
}

/// The thirty-two bytes the Finder keeps about a file: the four-character
/// codes the file's kind and the application that made it were known by,
/// the flags the Finder drew it from, and where its icon sat in the window
/// that held it. A file that came out of an archive usually has all of it
/// zero, which is worth seeing rather than worth hiding.
fn info() -> T {
    T::structure(
        "FinderFileInfo",
        vec![
            ("file_type", T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii)),
            ("creator", T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii)),
            ("finder_flags", T::flags("FinderFlags", T::u16(Big), FINDER_FLAGS)),
            ("icon_v", T::Int { bits: 16, endian: Big }),
            ("icon_h", T::Int { bits: 16, endian: Big }),
            ("folder", T::Int { bits: 16, endian: Big }),
            ("reserved", T::bytes(E::lit(8))),
            ("ext_finder_flags", T::flags("ExtFinderFlags", T::u16(Big), EXT_FINDER_FLAGS)),
            ("ext_reserved", T::bytes(E::lit(2))),
            ("put_away_folder", T::i32(Big)),
        ],
    )
}

/// The Finder's flags, by the names the Carbon headers give them.
const FINDER_FLAGS: &[(u32, &str)] = &[
    (0, "is on desk"),
    (7, "shared"),
    (8, "has no INITs"),
    (9, "has been inited"),
    (11, "has custom icon"),
    (12, "stationery"),
    (13, "name locked"),
    (14, "has bundle"),
    (15, "invisible"),
];

/// The flags the Finder keeps in the second half of the block.
const EXT_FINDER_FLAGS: &[(u32, &str)] = &[
    (6, "extended flags are invalid"),
    (7, "has custom badge"),
    (15, "object is busy"),
];

/// The block of extended attributes: its own head over the AppleDouble
/// one, then one entry per attribute, each naming the offset its data sits
/// at, counted from the start of the file.
fn attrs() -> T {
    T::structure(
        "ExtendedAttributes",
        vec![
            ("magic", T::magic(b"ATTR")),
            ("debug_tag", T::u32(Big)),
            ("total_size", T::u32(Big)),
            ("data_start", T::u32(Big)),
            ("data_length", T::u32(Big)),
            ("reserved", T::bytes(E::lit(12))),
            ("flags", T::u16(Big)),
            ("count", T::u16(Big)),
            ("attrs", T::array(attr(), E::field("count"))),
        ],
    )
}

/// One attribute: the run of its header, then the data it points at. The
/// header ends on a four-byte boundary, so the pad after the name is what
/// the name's length leaves of the round-up.
fn attr() -> T {
    let header = E::lit(11).add(E::field("namelen"));
    T::structure_named(
        "Attribute",
        "name",
        "",
        vec![
            ("at", T::u32(Big)),
            ("length", T::u32(Big)),
            ("flags", T::u16(Big)),
            ("namelen", T::u8()),
            ("name", T::text(StrLen::Terminated { end: 0, or_end: true }, Encoding::Utf8)),
            ("pad", T::bytes(header.clone().add(E::lit(3)).div(E::lit(4)).mul(E::lit(4)).sub(header))),
            ("data", T::at(E::field("at"), T::bytes(E::field("length")))),
        ],
    )
}

/// A resource fork by its header, which says where the resource data and
/// the map that indexes it sit. Both are counted from the start of the
/// fork, not of the file, so they are read in the window the entry cut.
/// The fork an archive usually carries is the empty one macOS writes: 256
/// bytes of header whose system area says so in words, no data, and a map
/// holding nothing.
fn resource_fork() -> T {
    T::structure(
        "ResourceFork",
        vec![
            ("data_offset", T::u32(Big)),
            ("map_offset", T::u32(Big)),
            ("data_length", T::u32(Big)),
            ("map_length", T::u32(Big)),
            ("system", T::text(StrLen::Padded { size: E::lit(112), pad: 0 }, Encoding::Ascii)),
            ("app", T::bytes(E::lit(128))),
            ("data", T::at_in_window(E::field("data_offset"), T::bytes(E::field("data_length")))),
            ("map", T::at_in_window(E::field("map_offset"), T::sized(E::field("map_length"), resource_map()))),
        ],
    )
}

/// The map at the back of a resource fork: a copy of the header the fork
/// began with, then where the list of types and the list of names sit,
/// both counted from the start of the map. An empty fork has a type count
/// of -1, which is how a list of none is written here.
fn resource_map() -> T {
    T::structure(
        "ResourceMap",
        vec![
            ("header_copy", T::bytes(E::lit(16))),
            ("next_map", T::u32(Big)),
            ("file_ref", T::u16(Big)),
            ("attributes", T::u16(Big)),
            ("types_at", T::u16(Big)),
            ("names_at", T::u16(Big)),
            ("type_count_minus_one", T::Int { bits: 16, endian: Big }),
            ("rest", T::bytes(E::Remaining)),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn be(v: &mut Vec<u8>, x: u32) {
        v.extend_from_slice(&x.to_be_bytes());
    }

    /// A region-shaped AppleDouble file: a Finder info entry carrying one
    /// attribute, and an empty resource fork.
    fn appledouble_file() -> Vec<u8> {
        let name = b"com.apple.lastuseddate#PS\0";
        let entry_len = (11 + name.len() + 3) & !3;
        let data_start = 120 + entry_len;
        let data = b"0910065c5e70\0";
        let fork_at = data_start + data.len();

        let mut v = Vec::new();
        be(&mut v, 0x00051607);
        be(&mut v, 0x00020000);
        v.extend_from_slice(b"Mac OS X        ");
        v.extend_from_slice(&2u16.to_be_bytes());
        be(&mut v, 9);
        be(&mut v, 50);
        be(&mut v, (fork_at - 50) as u32);
        be(&mut v, 2);
        be(&mut v, fork_at as u32);
        be(&mut v, 286);
        v.resize(50 + 32 + 2, 0);
        v.extend_from_slice(b"ATTR");
        be(&mut v, 0);
        be(&mut v, (data_start + data.len() - 50) as u32);
        be(&mut v, data_start as u32);
        be(&mut v, data.len() as u32);
        v.resize(v.len() + 12, 0);
        v.extend_from_slice(&0u16.to_be_bytes());
        v.extend_from_slice(&1u16.to_be_bytes());
        be(&mut v, data_start as u32);
        be(&mut v, data.len() as u32);
        v.extend_from_slice(&0u16.to_be_bytes());
        v.push(name.len() as u8);
        v.extend_from_slice(name);
        v.resize(data_start, 0);
        v.extend_from_slice(data);
        // The empty resource fork macOS writes: a 256-byte header, no data,
        // and a 30-byte map holding nothing.
        be(&mut v, 256);
        be(&mut v, 256);
        be(&mut v, 0);
        be(&mut v, 30);
        v.extend_from_slice(b"This resource fork intentionally left blank   ");
        v.resize(fork_at + 256, 0);
        be(&mut v, 256);
        be(&mut v, 256);
        be(&mut v, 0);
        be(&mut v, 30);
        v.resize(fork_at + 256 + 24, 0);
        v.extend_from_slice(&28u16.to_be_bytes());
        v.extend_from_slice(&30u16.to_be_bytes());
        v.extend_from_slice(&(-1i16).to_be_bytes());
        v
    }

    /// AppleSingle is the same header under a different magic, and the
    /// sniffer must not take one for the other.
    #[test]
    fn applesingle_is_told_apart_and_read() {
        let mut v = Vec::new();
        be(&mut v, 0x00051600);
        be(&mut v, 0x00020000);
        v.extend_from_slice(b"Mac OS X        ");
        v.extend_from_slice(&1u16.to_be_bytes());
        be(&mut v, 1);
        be(&mut v, 38);
        be(&mut v, 5);
        v.extend_from_slice(b"hello");

        assert_eq!(crate::formats::sniff(&v, v.len() as u64), Some("applesingle"));
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(applesingle());
        assert_eq!(
            ev.node(&d, &[4, 0, 0]).unwrap().value,
            Value::Enum { raw: 1, name: Some("data".into()), hex: false }
        );
        let data = ev.node(&d, &[5, 0]).unwrap();
        assert_eq!(data.offset_bits, 38 * 8);
        assert_eq!(data.size_bits, 5 * 8);
    }

    /// A Finder info entry that stops at thirty-two bytes, which is what a
    /// writer that is not macOS leaves: no pad, no attributes after it.
    #[test]
    fn a_bare_finder_info_entry_stops_where_it_ends() {
        let mut v = Vec::new();
        be(&mut v, 0x00051607);
        be(&mut v, 0x00020000);
        v.extend_from_slice(b"                ");
        v.extend_from_slice(&1u16.to_be_bytes());
        be(&mut v, 9);
        be(&mut v, 38);
        be(&mut v, 32);
        v.extend_from_slice(b"TEXTttxt");
        v.resize(38 + 32, 0);

        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(appledouble());
        let part = ev.node(&d, &[5, 0]).unwrap();
        assert_eq!(part.size_bits, 32 * 8);
        let Value::Str(kind) = &ev.node(&d, &[5, 0, 0]).unwrap().value else { panic!("not text") };
        assert_eq!(kind, "TEXT");
    }

    #[test]
    fn the_table_places_every_part_it_names() {
        let v = appledouble_file();
        assert_eq!(crate::formats::sniff(&v, v.len() as u64), Some("appledouble"));
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(appledouble());
        assert_eq!(ev.node(&d, &[2]).unwrap().value, Value::Str("Mac OS X        ".into()));
        assert_eq!(ev.node(&d, &[3]).unwrap().value, Value::UInt(2));
        assert_eq!(
            ev.node(&d, &[4, 0, 0]).unwrap().value,
            Value::Enum { raw: 9, name: Some("finder info".into()), hex: false }
        );

        // The Finder info part holds the attribute block, and the attribute
        // holds its data where its entry says.
        let finder = ev.node(&d, &[5, 0]).unwrap();
        assert_eq!(finder.offset_bits, 50 * 8);
        let attr = &[5, 0, 2, 8, 0];
        let a = ev.node(&d, attr).unwrap();
        assert_eq!(a.name, "[0] com.apple.lastuseddate#PS");
        // The pointer field itself takes no room where it stands; the
        // bytes it reaches sit under it.
        let data = ev.node(&d, &[5, 0, 2, 8, 0, 6]).unwrap();
        assert_eq!(data.offset_bits, 160 * 8);
        let bytes = ev.node(&d, &[5, 0, 2, 8, 0, 6, 0]).unwrap();
        assert_eq!(bytes.offset_bits, 160 * 8);
        assert_eq!(bytes.size_bits, 13 * 8);

        // The resource fork lands where its entry points, on the blank tag.
        let system = ev.node(&d, &[5, 1, 4]).unwrap();
        let Value::Str(s) = &system.value else { panic!("not text") };
        assert!(s.starts_with("This resource fork"));

        // Its map is where the header says, and holds nothing.
        let count = ev.node(&d, &[5, 1, 7, 0, 6]).unwrap();
        assert_eq!(count.value, Value::Int(-1));
        // The fork begins at 173, its map 256 into that, the count 28 into the map.
        assert_eq!(count.offset_bits, (173 + 256 + 28) * 8);
    }
}
