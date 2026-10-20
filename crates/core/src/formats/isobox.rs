//! The box JPEG 2000 and JPEG XL keep their files in.
//!
//! A box is `LBox`, the length of the whole box, `TBox`, four letters saying
//! what it is, and its contents, `DBox`. A length of 1 means the real length
//! is the eight bytes after the letters, `XLBox`, for a box past four
//! gigabytes; a length of 0 means the box runs to the end of the file, which
//! only the last one may say. That is the ISO base media file format's box,
//! and an MP4 is read the same way. The names are the ones ISO/IEC 15444-1
//! gives them in Annex I.
//!
//! What a box holds is the format's business, and so is what a superbox is:
//! each format registers this under the type name `Box`, with a switch of its
//! own for the contents, and a superbox is a list of `Box` again.

use crate::template::{Endian::*, Expr as E, Ty as T, Until};

/// A four-character box type as the big-endian number a switch compares it
/// against.
pub(crate) fn cc(s: &[u8; 4]) -> i128 {
    s.iter().fold(0i128, |acc, b| (acc << 8) | *b as i128)
}

/// Boxes until the end: a whole file, or the contents of a superbox.
pub(crate) fn boxes() -> T {
    T::repeat(T::Named("Box".into()), Until::End)
}

/// One box, with `contents` read in a window as long as the box says.
/// `contents` is a switch on `TBox`, which it can name as a field.
///
/// How long the contents are is a choice of three, and it is written as the
/// choice rather than folded into arithmetic: `LBox` less its eight bytes,
/// `XLBox` less its sixteen, or everything left. A length too short to hold
/// the box's own header is a file gone wrong, and the contents are then
/// empty, so a run of boxes still moves on.
pub(crate) fn boxed(contents: T) -> T {
    let lbox = E::field("LBox");
    let len = E::cond(
        lbox.clone().equal_to(E::lit(1)),
        E::field("XLBox").sub(E::lit(16)),
        E::cond(lbox.clone().equal_to(E::lit(0)), E::Remaining, lbox.sub(E::lit(8))),
    );
    T::structure_named(
        "Box",
        "TBox",
        "DBox",
        vec![
            ("LBox", T::u32(Big)),
            ("TBox", T::utf8(E::lit(4))),
            ("XLBox", T::when(E::field("LBox").equal_to(E::lit(1)), T::u64(Big))),
            ("DBox", T::sized(len.at_least(E::lit(0)).at_most(E::Remaining), contents)),
        ],
    )
    .counted_as("box")
}
