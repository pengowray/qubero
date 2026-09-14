//! The JP2 file format, ISO/IEC 15444-1 Annex I: the boxes a codestream is
//! kept in.
//!
//! A box is `LBox`, the length of the whole box, `TBox`, four letters saying
//! what it is, and its contents, `DBox`. A length of 1 means the real length
//! is the eight bytes after the letters, `XLBox`, for a box past four
//! gigabytes; a length of 0 means the box runs to the end of the file, which
//! only the last one may say. That is the ISO base media file format's box,
//! and an MP4 is read the same way.
//!
//! A JP2 file is the signature box, `ftyp`, the `jp2h` header, and the
//! codestream in `jp2c`, with anything else a writer wants to add in between
//! or after: XML, UUID-tagged data, and a list of where to find out what that
//! data is. `jp2h` and `res ` are superboxes, whose contents are more boxes,
//! and so is `uinf`. The same boxes are what a JPX or a JPM file opens with,
//! and a box this does not know keeps its length and its bytes.

use super::{codestream, depth};
use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Ty as T, Until};

/// A four-character box type as the big-endian number a switch compares it
/// against.
fn cc(s: &[u8; 4]) -> i128 {
    s.iter().fold(0i128, |acc, b| (acc << 8) | *b as i128)
}

/// The colour spaces a JP2 `colr` box may name, from I.5.3.3. JPX names a
/// score more, which show as their numbers.
const ENUM_CS: &[(i128, &str)] = &[(16, "sRGB"), (17, "greyscale"), (18, "sYCC")];

/// A whole JP2 file: boxes until the end.
pub(super) fn file() -> T {
    T::repeat(T::Named("Box".into()), Until::End)
}

/// One box, named `Box` in the template so that a superbox can hold more of
/// them.
///
/// How long the contents are is a choice of three, and it is written as the
/// choice rather than folded into arithmetic: `LBox` less its eight bytes,
/// `XLBox` less its sixteen, or everything left. A length too short to hold
/// the box's own header is a file gone wrong, and the contents are then
/// empty, so a run of boxes still moves on.
pub(super) fn jp2_box() -> T {
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
            ("DBox", T::sized(len.at_least(E::lit(0)).at_most(E::Remaining), contents())),
        ],
    )
    .counted_as("box")
}

/// What is inside a box, by its type.
fn contents() -> T {
    let superbox = || T::repeat(T::Named("Box".into()), Until::End);
    T::switch(
        E::field("TBox"),
        vec![
            (cc(b"jP  "), T::magic(&[0x0d, 0x0a, 0x87, 0x0a])),
            (cc(b"ftyp"), ftyp()),
            (cc(b"jp2h"), superbox()),
            (cc(b"ihdr"), ihdr()),
            (cc(b"bpcc"), T::array(T::u8(), E::Remaining)),
            (cc(b"colr"), colr()),
            (cc(b"pclr"), pclr()),
            (cc(b"cmap"), cmap()),
            (cc(b"cdef"), cdef()),
            (cc(b"res "), superbox()),
            (cc(b"resc"), resolution(["VRcN", "VRcD", "HRcN", "HRcD", "VRcE", "HRcE", "VRc", "HRc"])),
            (cc(b"resd"), resolution(["VRdN", "VRdD", "HRdN", "HRdD", "VRdE", "HRdE", "VRd", "HRd"])),
            (cc(b"jp2c"), codestream()),
            (cc(b"jp2i"), T::text(StrLen::Fixed(E::Remaining), Encoding::Utf8)),
            (cc(b"xml "), T::text(StrLen::Fixed(E::Remaining), Encoding::Utf8)),
            (cc(b"uuid"), T::structure("Uuid", vec![("ID", T::bytes(E::lit(16))), ("DATA", T::bytes(E::Remaining))])),
            (cc(b"uinf"), superbox()),
            (cc(b"ulst"), ulst()),
            (cc(b"url "), url()),
        ],
        T::bytes(E::Remaining),
    )
}

/// `ftyp`: the brand the file is written to, and every brand a reader of
/// which could read it. A JP2 file says `jp2 `; a JPX file says `jpx ` and
/// lists `jp2 ` as well when a JP2 reader would get a picture out of it.
fn ftyp() -> T {
    T::structure(
        "FileType",
        vec![("BR", T::utf8(E::lit(4))), ("MinV", T::u32(Big)), ("CL", T::array(T::utf8(E::lit(4)), E::Remaining.div(E::lit(4))))],
    )
}

/// `ihdr`: the image, as the codestream's SIZ says it. `BPC` is written the
/// way `Ssiz` is, or is 255 when the components differ and a `bpcc` box says
/// each one's depth. `C` is always 7, which says the codestream is JPEG 2000
/// Part 1; JPX gives other numbers to other codings.
fn ihdr() -> T {
    let varies = E::field("BPC").equal_to(E::lit(255));
    T::structure(
        "ImageHeader",
        vec![
            ("HEIGHT", T::u32(Big)),
            ("WIDTH", T::u32(Big)),
            ("NC", T::u16(Big)),
            ("BPC", T::u8()),
            ("C", T::enumeration("CompressionType", T::u8(), &[(7, "JPEG 2000")])),
            ("UnkC", T::enumeration("ColourspaceKnown", T::u8(), &[(0, "colourspace known"), (1, "colourspace unknown")])),
            ("IPR", T::enumeration("IntellectualProperty", T::u8(), &[(0, "no jp2i box"), (1, "jp2i box present")])),
            ("signed", T::when(varies.clone().equal_to(E::lit(0)), T::computed(E::field("BPC").bit(7)))),
            ("depth", T::when(varies.equal_to(E::lit(0)), T::computed(depth(E::field("BPC"))))),
        ],
    )
}

/// `colr`: what colour the components are. Either a colour space by number,
/// or an ICC profile written into the box, which stays bytes.
fn colr() -> T {
    let enumerated = E::field("METH").equal_to(E::lit(1));
    T::structure(
        "ColourSpecification",
        vec![
            ("METH", T::enumeration("ColourMethod", T::u8(), &[(1, "enumerated colourspace"), (2, "restricted ICC profile")])),
            ("PREC", T::Int { bits: 8, endian: Big }),
            ("APPROX", T::u8()),
            ("EnumCS", T::when(enumerated.clone(), T::enumeration("EnumCS", T::u32(Big), ENUM_CS))),
            ("PROFILE", T::when(enumerated.equal_to(E::lit(0)), T::bytes(E::Remaining))),
        ],
    )
}

/// `pclr`: a palette, `NE` entries of `NPC` columns. Each column is as deep as
/// its `B` says, written the way `Ssiz` is, and each entry of that column is
/// that many bits rounded up to whole bytes. So an entry is a row of numbers
/// of different widths, and which width is the column's: `Idx` inside an
/// entry is the column.
fn pclr() -> T {
    let width = depth(E::elem("B", E::Idx)).div_ceil(E::lit(8)).mul(E::lit(8));
    let entry = T::array(T::uint_expr(width, Big), E::field("NPC"));
    T::structure(
        "Palette",
        vec![
            ("NE", T::u16(Big)),
            ("NPC", T::u8()),
            ("B", T::array(T::u8(), E::field("NPC"))),
            ("C", T::array(entry, E::field("NE"))),
        ],
    )
}

/// `cmap`: which codestream component makes each channel, and whether it is
/// used as it is or looked up in the palette's column `PCOL`.
fn cmap() -> T {
    let channel = T::inline_structure(
        "ComponentMapping",
        vec![
            ("CMP", T::u16(Big)),
            ("MTYP", T::enumeration("MappingType", T::u8(), &[(0, "direct use"), (1, "palette mapping")])),
            ("PCOL", T::u8()),
        ],
    );
    T::array(channel, E::Remaining.div(E::lit(4)))
}

/// `cdef`: what each channel is, colour or opacity, and which colour it goes
/// with. An `Asoc` of 0 is the whole image and 65535 is none; anything else is
/// the number of a colour, counted from 1.
fn cdef() -> T {
    let channel = T::inline_structure(
        "ChannelDefinition",
        vec![
            ("Cn", T::u16(Big)),
            (
                "Typ",
                T::enumeration(
                    "ChannelType",
                    T::u16(Big),
                    &[(0, "colour"), (1, "opacity"), (2, "premultiplied opacity"), (65535, "unspecified")],
                ),
            ),
            ("Asoc", T::enumeration("ChannelAssociation", T::u16(Big), &[(0, "whole image"), (65535, "none")])),
        ],
    );
    T::structure("ChannelDefinitions", vec![("N", T::u16(Big)), ("channels", T::array(channel, E::field("N")))])
}

/// `resc` or `resd`: the resolution the image was captured at, or should be
/// shown at, in grid points a metre. Each direction is a fraction and a power
/// of ten, `VRcN / VRcD * 10^VRcE`, worked out beside the numbers it is made
/// of. `names` are the box's own names for the six numbers and the two
/// answers, which differ between the two boxes only in the `c` or the `d`.
fn resolution(names: [&'static str; 8]) -> T {
    let [vn, vd, hn, hd, ve, he, v, h] = names;
    let worth = |n: &str, d: &str, e: &str| T::computed_real(E::field(n).div(E::field(d)).mul(E::pow10(E::field(e))));
    T::structure(
        "Resolution",
        vec![
            (vn, T::u16(Big)),
            (vd, T::u16(Big)),
            (hn, T::u16(Big)),
            (hd, T::u16(Big)),
            (ve, T::Int { bits: 8, endian: Big }),
            (he, T::Int { bits: 8, endian: Big }),
            (v, worth(vn, vd, ve)),
            (h, worth(hn, hd, he)),
        ],
    )
}

/// `ulst`: the UUIDs a `url ` box says where to find out about.
fn ulst() -> T {
    T::structure("UuidList", vec![("NU", T::u16(Big)), ("ID", T::array(T::bytes(E::lit(16)), E::field("NU")))])
}

/// `url `: a version, three bytes of flags, and a URL ending in a NUL.
fn url() -> T {
    T::structure(
        "DataEntryUrl",
        vec![
            ("VERS", T::u8()),
            ("FLAG", T::bytes(E::lit(3))),
            ("LOC", T::text(StrLen::Terminated { end: 0, or_end: true }, Encoding::Utf8)),
        ],
    )
}
