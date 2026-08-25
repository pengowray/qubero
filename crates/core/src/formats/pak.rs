//! Quake PAK: the same idea as a WAD, with paths instead of eight-letter
//! names.
//!
//! Header, then the files, then a directory at the end. As with a WAD, the
//! directory is read where the header says it is by a field that costs no
//! bytes, so its entries are in hand before the files they place. Each entry
//! is a full path, which is what let the game load `sound/weapons/rocket.wav`
//! out of an archive as if it were on disk.

use crate::template::{Anchor, Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T};

pub fn pak() -> Template {
    Template::new(
        "pak",
        T::structure(
            "PAK",
            vec![
                ("magic", T::magic(b"PACK")),
                ("directory_offset", T::i32(Little)),
                // In bytes, not in entries: one entry is 64 of them.
                ("directory_size", T::i32(Little)),
                // Read at the end of the file without going there.
                (
                    "directory",
                    T::at(E::field("directory_offset"), T::array(entry(), E::field("directory_size").div(E::lit(64)))),
                ),
                // Everything after the header, with each file where its own
                // entry says. The directory is in that stretch and is read by
                // the field above, which is declared first.
                ("files", T::pointer_list_sized("directory", &["offset"], Anchor::File, E::lit(0), file())),
            ],
        ),
    )
}

fn entry() -> T {
    T::structure_named(
        "File",
        "name",
        "",
        vec![
            ("name", T::text(StrLen::Padded { size: E::lit(56), pad: 0 }, Encoding::Ascii)),
            ("offset", T::i32(Little)),
            ("size", T::i32(Little)),
        ],
    )
    .counted_as("file")
}

/// The bytes of one file, as long as its own directory entry says.
fn file() -> T {
    T::bytes(E::elem_field("directory", E::idx(), &["size"]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    #[test]
    fn every_file_is_placed_by_the_entry_that_names_it() {
        let files: [(&str, &[u8]); 2] = [("sound/items/health.wav", b"RIFF...."), ("progs/player.mdl", b"IDPO")];
        let mut data = Vec::new();
        let mut dir = Vec::new();
        for (name, body) in files {
            let mut padded = name.as_bytes().to_vec();
            padded.resize(56, 0);
            dir.extend_from_slice(&padded);
            dir.extend_from_slice(&((12 + data.len()) as i32).to_le_bytes());
            dir.extend_from_slice(&(body.len() as i32).to_le_bytes());
            data.extend_from_slice(body);
        }
        let mut v = b"PACK".to_vec();
        v.extend_from_slice(&((12 + data.len()) as i32).to_le_bytes());
        v.extend_from_slice(&(dir.len() as i32).to_le_bytes());
        v.extend_from_slice(&data);
        v.extend_from_slice(&dir);

        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(pak());
        let dir = ev.node(&d, &[3, 0]).unwrap();
        assert_eq!(dir.child_count, 2);
        assert_eq!(ev.node(&d, &[3, 0, 0, 0]).unwrap().value, Value::Str("sound/items/health.wav".into()));
        assert_eq!(ev.node(&d, &[3, 0, 1, 1]).unwrap().value, Value::Int(20));
        assert_eq!(ev.node(&d, &[3, 0, 1, 2]).unwrap().value, Value::Int(4));
        // Each file is placed by its entry, and its bytes are its own.
        assert_eq!(ev.node(&d, &[4]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[4, 0]).unwrap().offset_bits, 12 * 8);
        assert_eq!(ev.node(&d, &[4, 0]).unwrap().size_bits, 8 * 8);
        assert_eq!(ev.node(&d, &[4, 1]).unwrap().offset_bits, 20 * 8);
        assert_eq!(ev.node(&d, &[4, 1]).unwrap().size_bits, 4 * 8);
        // A byte of a file, and a byte of the directory at the end.
        assert_eq!(ev.locate(&d, 13 * 8).unwrap(), vec![4, 0]);
        assert_eq!(ev.locate(&d, 24 * 8).unwrap(), vec![3, 0, 0, 0]);
    }
}
