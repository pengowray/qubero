//! JPEG XL: a codestream, on its own or inside the boxes of a container.
//!
//! The codestream is ISO/IEC 18181-1. It opens with `FF 0A` and then the
//! image header: SizeHeader, ImageMetadata (bit depth, extra channels such as
//! alpha, whether the samples are XYB, the colour encoding, orientation,
//! preview, animation, tone mapping) and the transform data, padded to a
//! byte. An ICC profile follows when the colour encoding asks for one, then
//! the preview frame if there is one, and then the frames. See [`headers`].
//!
//! Each frame is a FrameHeader, a table of contents (TOC) of section lengths,
//! and the sections it lists: LfGlobal, one LfGroup per 2048 by 2048 block,
//! HfGlobal, and a PassGroup for every pass of every 256 by 256 group. A frame
//! of one group and one pass has one TOC entry, covering all of them. Frames
//! follow one another until the one whose header says it is the last. See
//! [`frame`].
//!
//! Every header field is read low bit first, and many start in one byte and
//! end in the next: in the smallest file there is, SizeHeader's three-bit
//! `ratio` is the top two bits of the first header byte and the bottom bit of
//! the second. The IR reads such a field as one number; see
//! [`crate::decode::low_bit_first`].
//!
//! What stays bytes:
//!
//! * The sections. Each is placed and named, and what is inside is
//!   entropy-coded image data that only decoding reads.
//! * An ICC profile, and everything after it. The profile is entropy-coded,
//!   so where it ends is known only once it is decoded, and the preview and
//!   the frames after it cannot be found.
//! * A frame whose TOC is permuted, and everything after it. The order of the
//!   sections is then an entropy-coded Lehmer code, the same problem again.
//! * The names an extra channel or a frame may carry read as numbers, one for
//!   each byte, since each byte is eight bits read low bit first from wherever
//!   the field before it ended, and the IR reads text only from whole bytes.
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
mod frame;
mod headers;

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

/// A codestream: the signature, the image header, and the frames.
pub(crate) fn codestream() -> T {
    let icc = frame::Image::want_icc;
    let readable = || icc().negate();
    T::structure(
        "Codestream",
        vec![
            ("signature", T::magic(&SIGNATURE)),
            ("image_header", image_header()),
            ("icc", T::when(icc(), T::bytes(E::Remaining))),
            ("preview", T::when(readable().both(frame::Image::preview()), frame::frame(true))),
            ("frames", T::when(readable(), frame::frames())),
            ("trailer", T::when(readable(), T::bytes(E::Remaining))),
        ],
    )
    .field_doc(
        "icc",
        "The entropy-coded ICC profile and everything after the profile, shown as bytes. The end of the ICC profile can be found only by decoding the profile, which Qubero does not do.",
    )
    .field_doc("trailer", "Bytes after the last frame. The codestream ends with the last frame, so these bytes are not part of the codestream.")
}

/// The image header: SizeHeader, ImageMetadata and the transform data, and
/// the bits up to the next byte. Where an ICC profile follows, those bits are
/// its first ones rather than padding.
fn image_header() -> T {
    let metadata = |field: &str| E::within(&["metadata", field]);
    let xyb = E::cond(metadata("all_default").equal_to(E::lit(1)), E::lit(1), metadata("xyb_encoded"));
    let icc = || {
        metadata("all_default").equal_to(E::lit(0)).both(
            E::within(&["metadata", "colour_encoding", "all_default"])
                .equal_to(E::lit(0))
                .both(E::within(&["metadata", "colour_encoding", "want_icc"]).equal_to(E::lit(1))),
        )
    };
    let to_byte = || headers::pad_to_byte(E::bits_of("size").add(E::bits_of("metadata")).add(E::bits_of("transform")));
    T::structure(
        "ImageHeader",
        vec![
            ("size", headers::size_header()),
            ("metadata", headers::image_metadata()),
            ("transform", headers::transform_data(xyb)),
            ("pad", T::when(icc().negate(), to_byte())),
            ("icc_start", T::when(icc(), to_byte())),
        ],
    )
    .field_doc("pad", "ZeroPadToByte: bits set to 0 up to the next byte boundary, where the preview frame or the first frame starts.")
    .field_doc("icc_start", "The first bits of the ICC profile, up to the next byte boundary. The ICC profile starts straight after the transform data.")
}

#[cfg(test)]
mod tests;
