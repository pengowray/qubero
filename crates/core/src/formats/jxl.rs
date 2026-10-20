//! JPEG XL: a codestream, on its own or inside the boxes of a container.
//!
//! The codestream is ISO/IEC 18181-1. It opens with `FF 0A` and then the
//! image header: SizeHeader, ImageMetadata (bit depth, extra channels such as
//! alpha, whether the samples are XYB, the colour encoding, orientation,
//! preview, animation, tone mapping) and the transform data, padded to a
//! byte. An ICC profile follows when the colour encoding asks for one, then
//! the preview frame if there is one, and then the frames. Each frame is a
//! FrameHeader, a table of contents (TOC) of section lengths, and the
//! sections it lists: LfGlobal, one LfGroup per 2048 by 2048 block, HfGlobal,
//! and a PassGroup for every pass of every 256 by 256 group. A frame of one
//! group and one pass has one TOC entry, covering all of them.
//!
//! What stays bytes: everything after the signature. The headers are fields
//! packed from the bottom of each byte, LSB first, and a value regularly
//! starts in one byte and ends in the next: in the smallest file there is,
//! SizeHeader's three-bit `ratio` is the top two bits of the first header
//! byte and the bottom bit of the second. The IR places an LSB-first field
//! only when it fits in one byte (see [`crate::decode::lsb_offset`]), so the
//! headers cannot be described yet, and with them goes the TOC, which is the
//! only thing that says where the sections are.
//!
//! The container is ISO/IEC 18181-2: boxes, each a length, four letters and
//! contents, the box a JP2 file is made of (see [`crate::formats::isobox`]).
//! It opens with a signature box, `JXL `, whose length of twelve is the first
//! four bytes, and `ftyp`. The codestream is in one `jxlc` box, or cut into
//! numbered `jxlp` parts with metadata boxes between them; the parts are
//! joined here in the order they are in and read as one codestream, the way
//! a PNG's IDAT chunks are. See [`boxes`].
//!
//! One template reads both, since both are what a `.jxl` turns out to be and
//! the first two bytes say which.

use crate::template::{Endian::*, Expr as E, Template, Ty as T};

mod boxes;

/// The codestream's signature.
const SIGNATURE: [u8; 2] = [0xff, 0x0a];

pub fn jxl() -> Template {
    // Two bytes decide which of the two this is. A file too short to hold them
    // is neither, and reads as boxes, which will say so.
    let first = E::cond(E::Remaining.less_than(E::lit(2)), E::lit(0), E::peek(16, Big));
    let exif = super::tiff::tiff_part();
    Template::new("jxl", T::switch(first, vec![(0xff0a, codestream())], boxes::file()))
        .with_type("Box", boxes::jxl_box(&exif.root))
        .with_part(&exif)
}

/// A codestream: the signature, and then the headers and frames, which stay
/// bytes for the reason the module notes give.
pub(crate) fn codestream() -> T {
    T::structure("Codestream", vec![("signature", T::magic(&SIGNATURE)), ("data", T::bytes(E::Remaining))])
        .field_doc(
            "data",
            "The image header, then the frame header, TOC and sections of each frame. Qubero cannot yet show \
             LSB-first bit fields that cross a byte boundary, and the headers have many such bit fields, so \
             everything after the signature is shown as one run of bytes.",
        )
}

#[cfg(test)]
mod tests;
