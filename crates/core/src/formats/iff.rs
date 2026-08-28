//! IFF: the Amiga's container, and the one RIFF was copied from.
//!
//! A file is `FORM`, a size, a four-character form type, and then chunks. Each
//! chunk is an id, a size, and a body padded to an even length. The one thing
//! that separates it from RIFF is the direction of the numbers: the Amiga was
//! a 68000, so every size is big-endian.
//!
//! The frame lives here and the two formats built on it, AIFF and ILBM, say
//! only what their own chunks hold.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};

/// A four-character chunk id as the big-endian number a switch compares.
pub(super) fn cc(s: &str) -> i128 {
    s.bytes().fold(0i128, |acc, b| (acc << 8) | b as i128)
}

/// The IFF frame, with `body` deciding what each chunk's contents are.
pub(super) fn iff(name: &str, body: T) -> Template {
    Template::new(
        name,
        T::structure(
            "FORM",
            vec![
                ("magic", T::magic(b"FORM")),
                ("size", T::u32(Big)),
                ("form", T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii)),
                ("chunks", T::repeat(chunk(body), Until::End)),
            ],
        ),
    )
}

fn chunk(body: T) -> T {
    // Odd-sized bodies are followed by a pad byte that the size does not
    // count.
    let pad = E::field("size").pad_to(2);
    T::structure_named(
        "Chunk",
        "id",
        "body",
        vec![
            ("id", T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii)),
            ("size", T::u32(Big)),
            ("body", T::sized(E::field("size"), body)),
            ("pad", T::bytes(pad)),
        ],
    )
}

/// A run of text filling its chunk. IFF puts titles, authors and copyright
/// lines in chunks that hold nothing else.
pub(super) fn chunk_text() -> T {
    T::text(StrLen::Fixed(E::Remaining), Encoding::Latin1)
}
