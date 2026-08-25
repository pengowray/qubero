//! Doom WAD: a header of three numbers, the lumps, and a directory at the end
//! saying where each of them is.
//!
//! The directory being last is the whole difficulty. A `PointerList` places
//! children at offsets read from an earlier field, and here the offsets are
//! read after everything they point at, so the lumps cannot be placed by the
//! table that describes them. What the template can say is where the table is
//! and what is in it: every lump has its offset, its size and its name, and
//! the space they sit in is one region.
//!
//! Reading it the way the format means it needs a list whose offsets are read
//! after the region they point into, and that is a gap in the IR.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T};

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
                // Everything between the header and the directory: the lumps
                // themselves, in whatever order the builder wrote them.
                ("lumps", T::bytes(E::field("directory_offset").sub(E::lit(12)))),
                ("directory", T::array(entry(), E::field("lump_count"))),
            ],
        ),
    )
}

/// One directory entry. A lump of size zero is a marker: `F_START` and the
/// names like it exist only to bracket the lumps between them.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn wad_bytes() -> Vec<u8> {
        let lumps: [(&[u8], &[u8; 8]); 2] = [(b"pixels", b"TITLEPIC"), (b"mus", b"D_E1M1\0\0")];
        let mut data = Vec::new();
        let mut dir = Vec::new();
        for (body, name) in lumps {
            dir.extend_from_slice(&((12 + data.len()) as i32).to_le_bytes());
            dir.extend_from_slice(&(body.len() as i32).to_le_bytes());
            dir.extend_from_slice(name);
            data.extend_from_slice(body);
        }
        let mut v = b"IWAD".to_vec();
        v.extend_from_slice(&2i32.to_le_bytes());
        v.extend_from_slice(&((12 + data.len()) as i32).to_le_bytes());
        v.extend_from_slice(&data);
        v.extend_from_slice(&dir);
        v
    }

    #[test]
    fn the_directory_at_the_end_says_where_every_lump_is() {
        let d = Document::new(MemSource(wad_bytes()));
        let mut ev = Evaluator::new(wad());
        assert_eq!(ev.node(&d, &[0]).unwrap().value, Value::Str("IWAD".into()));
        assert_eq!(ev.node(&d, &[3]).unwrap().size_bits, 9 * 8);
        let dir = ev.node(&d, &[4]).unwrap();
        assert_eq!(dir.child_count, 2);
        assert_eq!(ev.node(&d, &[4, 0, 2]).unwrap().value, Value::Str("TITLEPIC".into()));
        assert_eq!(ev.node(&d, &[4, 0, 0]).unwrap().value, Value::Int(12));
        // A short name keeps its padding out of the value.
        assert_eq!(ev.node(&d, &[4, 1, 2]).unwrap().value, Value::Str("D_E1M1".into()));
        assert_eq!(ev.node(&d, &[4, 1, 1]).unwrap().value, Value::Int(3));
    }
}
