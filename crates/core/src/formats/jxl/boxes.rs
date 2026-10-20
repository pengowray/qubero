//! The JPEG XL container, ISO/IEC 18181-2: the boxes a codestream is kept in,
//! beside the metadata that goes with it.
//!
//! A container is the signature box, `ftyp`, and then any of these in any
//! order: `jxll`, which says what level of the codestream to expect; `jxlc`,
//! the whole codestream; `jxlp`, one part of it, numbered; `jbrd`, what it
//! takes to rebuild the JPEG file the codestream was losslessly recompressed
//! from; `Exif`, `xml ` and `jumb` metadata, and `brob`, any of those
//! compressed with Brotli; and `jxli`, an index of the frames an animation
//! can be started from.
//!
//! A codestream cut into `jxlp` boxes may be cut anywhere, inside a header or
//! a section, so no one part reads as anything. The parts are joined in the
//! order the boxes are in, which the container requires to be the order of
//! their numbers, and read as one codestream in `codestream` after the boxes.
//!
//! What stays bytes: `jbrd`, which is LSB-first bit fields like the
//! codestream's headers, followed by Brotli; a JUMBF content box this does
//! not name; and any box this does not know.

use super::codestream;
use crate::codec::Codec;
use crate::formats::isobox::{boxed, boxes, cc};
use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Step, Ty as T};

/// A whole container: boxes until the end, and the codestream its `jxlp`
/// boxes hold, joined. A file whose codestream is in `jxlc` has no parts to
/// join, and its codestream reads inside that box.
pub(super) fn file() -> T {
    let parts = E::tagged_bytes("boxes", &["TBox"], b"jxlp", &["TBox"]).not_equal(E::lit(0));
    let joined = T::stitched(vec![Step::field("boxes"), Step::each(), Step::field("DBox"), Step::field("part")], None, None, codestream());
    T::structure("Container", vec![("boxes", boxes()), ("codestream", T::when(parts, joined))])
}

/// One box, named `Box` in the template so that a superbox can hold more of
/// them. `exif` is the TIFF an `Exif` box holds.
pub(super) fn jxl_box(exif: &T) -> T {
    boxed(contents(exif))
}

/// What is inside a box, by its type.
fn contents(exif: &T) -> T {
    let mut cases = vec![
        (cc(b"JXL "), T::magic(&[0x0d, 0x0a, 0x87, 0x0a])),
        (cc(b"ftyp"), ftyp()),
        (cc(b"jxll"), T::enumeration("Level", T::u8(), &[(5, "level 5"), (10, "level 10")])),
        (cc(b"jxlc"), codestream()),
        (cc(b"jxlp"), part()),
        (cc(b"jbrd"), T::bytes(E::Remaining)),
        (cc(b"brob"), brob(exif)),
        (cc(b"jxli"), frame_index()),
    ];
    cases.extend(metadata(exif));
    T::switch(E::field("TBox"), cases, T::bytes(E::Remaining))
}

/// The boxes that hold metadata, which are the ones a `brob` box may hold
/// compressed: every box but the signature, `ftyp`, `jxll`, `jxli`, the
/// codestream's boxes, `jbrd` and `brob` itself. The JUMBF boxes are here
/// because a `jumb` superbox holds them.
fn metadata(exif: &T) -> Vec<(i128, T)> {
    vec![
        (cc(b"Exif"), exif_box(exif)),
        (cc(b"xml "), T::text(StrLen::Fixed(E::Remaining), Encoding::Utf8)),
        (cc(b"jumb"), boxes()),
        (cc(b"jumd"), jumd()),
        (cc(b"json"), T::text(StrLen::Fixed(E::Remaining), Encoding::Utf8)),
        (cc(b"uuid"), T::structure("Uuid", vec![("ID", T::bytes(E::lit(16))), ("DATA", T::bytes(E::Remaining))])),
    ]
}

/// `ftyp`: the brand the file is written to and the brands a reader of which
/// could read it, which for JPEG XL are both `jxl `.
fn ftyp() -> T {
    T::structure(
        "FileType",
        vec![
            ("major_brand", T::utf8(E::lit(4))),
            ("minor_version", T::u32(Big)),
            ("compatible_brands", T::array(T::utf8(E::lit(4)), E::Remaining.div(E::lit(4)))),
        ],
    )
}

/// `jxlp`: one part of the codestream. The number counts from zero, and its
/// top bit is set on the last part only.
fn part() -> T {
    T::structure_named(
        "CodestreamPart",
        "",
        "part",
        vec![
            ("index", T::u32(Big)),
            ("number", T::computed(E::field("index").and(E::lit(0x7fff_ffff)))),
            ("last", T::computed(E::field("index").bit(31))),
            ("part", T::bytes(E::Remaining)),
        ],
    )
}

/// `brob`: another box's contents compressed with Brotli, and the type of the
/// box it was. What it unpacks to reads as that box would.
fn brob(exif: &T) -> T {
    T::structure(
        "BrotliBox",
        vec![
            ("original_type", T::utf8(E::lit(4))),
            ("contents", T::decoded(E::Remaining, Codec::Brotli, T::switch(E::field("original_type"), metadata(exif), T::bytes(E::Remaining)))),
        ],
    )
}

/// `Exif`: a TIFF file, as a JPEG's APP1 holds one, after four bytes saying
/// how far into the rest it starts. A writer that copied an `Exif\0\0`
/// prefix along with the TIFF says six; libjxl writes zero.
fn exif_box(exif: &T) -> T {
    T::structure(
        "Exif",
        vec![
            ("tiff_header_offset", T::u32(Big)),
            ("prefix", T::bytes(E::field("tiff_header_offset").at_most(E::Remaining))),
            ("tiff", exif.clone()),
        ],
    )
}

/// `jumd`: the description box a JUMBF superbox opens with. `TYPE` says what
/// the content boxes after it are, as a UUID whose first four bytes are a box
/// type when the rest is ISO's suffix: `json`, `cbor`, `c2pa` and so on.
/// `TOGGLES` says which of the other fields are written.
fn jumd() -> T {
    let has = |bit: u32| E::field("TOGGLES").bit(bit);
    T::structure(
        "JumbfDescription",
        vec![
            ("TYPE", T::inline_structure("ContentType", vec![("code", T::utf8(E::lit(4))), ("suffix", T::bytes(E::lit(12)))])),
            (
                "TOGGLES",
                T::flags(
                    "Toggles",
                    T::u8(),
                    &[(0, "requestable"), (1, "label present"), (2, "ID present"), (3, "signature present"), (4, "private box present")],
                ),
            ),
            ("LABEL", T::when(has(1), T::text(StrLen::Terminated { end: 0, or_end: true }, Encoding::Utf8))),
            ("ID", T::when(has(2), T::u32(Big))),
            ("SIGNATURE", T::when(has(3), T::bytes(E::lit(32)))),
            ("PRIVATE", T::when(has(4), T::Named("Box".into()))),
        ],
    )
}

/// `jxli`: the frames an animation can be started from. Each entry says how
/// far on in the codestream its frame starts from where the one before it
/// did, the first counting from the start of the codestream, and how many
/// ticks and how many frames lie between the two. A tick is `TNUM / TDEN`
/// seconds. The counts are LEB128, seven bits a byte, low bits first.
fn frame_index() -> T {
    let entry = T::inline_structure("IndexedFrame", vec![("OFFi", T::leb_u()), ("Ti", T::leb_u()), ("Fi", T::leb_u())]);
    T::structure(
        "FrameIndex",
        vec![
            ("NF", T::leb_u()),
            ("TNUM", T::u32(Big)),
            ("TDEN", T::u32(Big)),
            ("frames", T::array(entry.counted_as("frame"), E::field("NF"))),
        ],
    )
}
