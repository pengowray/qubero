//! Quake PAK: the same idea as a WAD, with paths instead of eight-letter
//! names.
//!
//! Header, then the files, then a directory at the end. As with a WAD, the
//! directory is read after the bytes it points at, so the entries say where
//! each file is rather than the template placing it there. Each entry is a
//! full path, which is what let the game load `sound/weapons/rocket.wav` out
//! of an archive as if it were on disk.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T};

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
                ("files", T::bytes(E::field("directory_offset").sub(E::lit(12)))),
                ("directory", T::array(entry(), E::field("directory_size").div(E::lit(64)))),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    #[test]
    fn the_directory_holds_a_path_for_every_file() {
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
        assert_eq!(ev.node(&d, &[4]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[4, 0, 0]).unwrap().value, Value::Str("sound/items/health.wav".into()));
        assert_eq!(ev.node(&d, &[4, 1, 1]).unwrap().value, Value::Int(20));
        assert_eq!(ev.node(&d, &[4, 1, 2]).unwrap().value, Value::Int(4));
    }
}
