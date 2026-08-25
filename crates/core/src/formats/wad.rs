//! Doom WAD: a header of three numbers, the lumps, and a directory at the end
//! saying where each of them is.
//!
//! The directory being last is what makes this worth writing down. A pointer
//! list places children at offsets read from an earlier field, so the offsets
//! have to be in hand before the bytes they point at are reached, and here
//! they are written after all of them. What closes the distance is a field
//! that costs no bytes and reads its contents somewhere else: the directory is
//! declared straight after the header, at the offset the header gives, and the
//! cursor does not move. The lumps are then a list over everything after the
//! header, one child per entry, each placed where its entry says.
//!
//! The directory sits inside that stretch and belongs to no lump. Being
//! declared first is what settles it: the cursor lands in the directory rather
//! than in the space between two lumps, and every byte of the file is named
//! once.
//!
//! A lump of size zero is a marker rather than a resource: `F_START`,
//! `S_END` and the names like them exist only to bracket the lumps between
//! them, which is how a level knows where its own graphics are.

use crate::template::{Anchor, Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T};

pub fn wad() -> Template {
    Template::new(
        "wad",
        T::structure(
            "WAD",
            vec![
                // IWAD is a game; PWAD is a patch that replaces lumps in one.
                ("magic", T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii)),
                ("lump_count", T::i32(Little)),
                ("directory_offset", T::i32(Little)),
                // Read at the end of the file without going there, so the
                // lumps below can be placed by what it holds.
                ("directory", T::at(E::field("directory_offset"), T::array(entry(), E::field("lump_count")))),
                // Everything after the header, with each lump at the offset
                // its own entry names. The directory sits in that stretch too
                // and belongs to no lump; it is read by the field above, which
                // is declared first and so is the one the cursor lands in.
                ("lumps", T::pointer_list_sized("directory", &["offset"], Anchor::File, E::lit(0), lump())),
            ],
        ),
    )
}

/// One directory entry: where a lump is, how long it is, and what it is called.
fn entry() -> T {
    T::structure_named(
        "Lump",
        "name",
        "",
        vec![
            ("offset", T::i32(Little)),
            ("size", T::i32(Little)),
            // Eight bytes, NUL padded, and not NUL terminated when the name
            // fills them.
            ("name", T::text(StrLen::Padded { size: E::lit(8), pad: 0 }, Encoding::Ascii)),
        ],
    )
    .counted_as("lump")
}

/// The bytes of one lump, as long as its own directory entry says.
fn lump() -> T {
    T::bytes(E::elem_field("directory", E::idx(), &["size"]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn wad_bytes() -> Vec<u8> {
        let lumps: [(&[u8], &[u8; 8]); 3] =
            [(b"pixels", b"TITLEPIC"), (b"", b"F_START\0"), (b"mus", b"D_E1M1\0\0")];
        let mut data = Vec::new();
        let mut dir = Vec::new();
        for (body, name) in lumps {
            dir.extend_from_slice(&((12 + data.len()) as i32).to_le_bytes());
            dir.extend_from_slice(&(body.len() as i32).to_le_bytes());
            dir.extend_from_slice(name);
            data.extend_from_slice(body);
        }
        let mut v = b"IWAD".to_vec();
        v.extend_from_slice(&3i32.to_le_bytes());
        v.extend_from_slice(&((12 + data.len()) as i32).to_le_bytes());
        v.extend_from_slice(&data);
        v.extend_from_slice(&dir);
        v
    }

    #[test]
    fn the_directory_reads_where_the_header_says_it_is() {
        let d = Document::new(MemSource(wad_bytes()));
        let mut ev = Evaluator::new(wad());
        assert_eq!(ev.node(&d, &[0]).unwrap().value, Value::Str("IWAD".into()));
        // The field costs nothing where it is declared.
        let directory = ev.node(&d, &[3]).unwrap();
        assert_eq!(directory.size_bits, 0);
        assert_eq!(directory.offset_bits, 12 * 8);
        // And its contents are at the end of the file.
        let entries = ev.node(&d, &[3, 0]).unwrap();
        assert_eq!(entries.offset_bits, 21 * 8);
        assert_eq!(entries.child_count, 3);
        assert_eq!(ev.node(&d, &[3, 0, 0, 2]).unwrap().value, Value::Str("TITLEPIC".into()));
        assert_eq!(ev.node(&d, &[3, 0, 2, 2]).unwrap().value, Value::Str("D_E1M1".into()));
    }

    #[test]
    fn every_lump_is_placed_by_its_own_entry() {
        let d = Document::new(MemSource(wad_bytes()));
        let mut ev = Evaluator::new(wad());
        let lumps = ev.node(&d, &[4]).unwrap();
        assert_eq!(lumps.child_count, 3);
        // The list covers everything after the header.
        assert_eq!(lumps.offset_bits, 12 * 8);

        let first = ev.node(&d, &[4, 0]).unwrap();
        assert_eq!(first.offset_bits, 12 * 8);
        assert_eq!(first.size_bits, 6 * 8);
        // A marker covers no bytes, and the lump after it is not moved by it.
        assert_eq!(ev.node(&d, &[4, 1]).unwrap().size_bits, 0);
        assert_eq!(ev.node(&d, &[4, 2]).unwrap().offset_bits, 18 * 8);
        assert_eq!(ev.node(&d, &[4, 2]).unwrap().size_bits, 3 * 8);
    }

    #[test]
    fn the_cursor_finds_a_lump_and_a_directory_entry_alike() {
        let d = Document::new(MemSource(wad_bytes()));
        let mut ev = Evaluator::new(wad());
        // A byte of the third lump.
        assert_eq!(ev.locate(&d, 19 * 8).unwrap(), vec![4, 2]);
        // A byte of the directory, which sits past everything declared above
        // it and is still found.
        assert_eq!(ev.locate(&d, (21 + 8) * 8).unwrap(), vec![3, 0, 0, 2]);
    }

    #[test]
    fn the_linear_view_reads_the_file_in_the_order_it_is_written() {
        let d = Document::new(MemSource(wad_bytes()));
        let mut ev = Evaluator::new(wad());
        let spans = ev.spans(&d, 0, 21 * 8 + 48 * 8, 100).unwrap();
        let seen: Vec<_> = spans.iter().map(|s| (s.offset_bits / 8, s.size_bits / 8, s.name.clone(), s.gap)).collect();
        // The header, the lumps that hold bytes, and then the directory,
        // every byte named once and nothing named twice. The marker between
        // the two lumps covers nothing, so there is no row for it here; it is
        // still an entry in the directory below and a child of the list.
        let want: Vec<(u64, u64, &str, bool)> = vec![
            (0, 4, "magic", false),
            (4, 4, "lump_count", false),
            (8, 4, "directory_offset", false),
            (12, 6, "[0] TITLEPIC", false),
            (18, 3, "[2] D_E1M1", false),
            (21, 4, "offset", false),
            (25, 4, "size", false),
            (29, 8, "name", false),
            (37, 4, "offset", false),
            (41, 4, "size", false),
            (45, 8, "name", false),
            (53, 4, "offset", false),
            (57, 4, "size", false),
            (61, 8, "name", false),
        ];
        assert_eq!(seen, want.into_iter().map(|(a, b, c, d)| (a, b, c.to_string(), d)).collect::<Vec<_>>());
    }
}
