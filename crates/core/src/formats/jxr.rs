//! JPEG XR: a TIFF on the outside and a bit stream on the inside.
//!
//! The file begins `II`, the same two letters a TIFF begins with, and then
//! `0xbc` where a TIFF writes 42. Everything after that is a TIFF directory,
//! entry for entry: a count, twelve bytes each, four bytes that are the value
//! when it fits and where to find it when it does not. So the directory is
//! read by the code that reads a TIFF's, against a different set of tag names.
//! Only the way round is settled in advance. A TIFF asks the file which way it
//! is; this format answers `II` and nothing else, so every number in it is
//! little-endian.
//!
//! What the entries hold is a picture the container itself never describes.
//! Two of them are an offset and a byte count, and what they point at is a
//! codestream: a signature, and then fields packed to the bit rather than to
//! the byte. `image offset` and `image byte count` are separate entries, and
//! either may come first, so the codestream is placed by searching the
//! directory for both rather than by reading them in order. An alpha channel
//! the file keeps as a picture of its own is two more entries pointing at a
//! second codestream of exactly the same shape.
//!
//! Inside, the four bytes after `WMPHOTO\0` are thirty-two bits of flags, and
//! several of them change what may be read next. `short header` says the width
//! and the height are sixteen bits rather than thirty-two; `tiling` says two
//! twelve-bit counts follow and then that many tile sizes; `windowing` says
//! four six-bit margins do. The plane header after them asks the same kind of
//! question again, of the colour format it has just read and of the bit depth
//! the image header read before it.
//!
//! A run of fields that thin does not end on a byte, and the format says so:
//! each plane header is followed by however many bits it takes to reach the
//! next one, because what comes after starts there. Nothing in the header says
//! how many that is; it is the length of the header itself, added up. That is
//! what `bits_of` is for.
//!
//! What is not read is the index table and the coded tiles. The table's length
//! follows from the bitstream order, the bands kept and the tile counts
//! together, and past it every byte is entropy-coded coefficients. The two
//! headers are where a file says what it is, and this reads both of them and
//! stops.

use crate::template::{Endian::*, Expr as E, Template, Ty as T};

use super::tiff;

/// What a JPEG XR directory's tags mean. The numbers below `0x8000` are TIFF's
/// and mean there what they mean here; the `0xbc` block is this format's own,
/// and is where everything that makes the file a picture rather than a
/// description of one is written.
pub const TAG: &[(i128, &str)] = &[
    (0x010d, "document name"),
    (0x010e, "image description"),
    (0x010f, "make"),
    (0x0110, "model"),
    (0x011d, "page name"),
    (0x0129, "page number"),
    (0x0131, "software"),
    (0x0132, "date time"),
    (0x013b, "artist"),
    (0x013c, "host computer"),
    (0x02bc, "xmp"),
    (0x4746, "rating stars"),
    (0x4749, "rating value"),
    (0x8298, "copyright"),
    (0x8649, "photoshop"),
    (0x8769, "exif ifd"),
    (0x8773, "icc profile"),
    (0x83bb, "iptc"),
    (0x8825, "gps ifd"),
    (0x9c9b, "caption"),
    (0xa001, "colour space"),
    (0xa005, "interoperability ifd"),
    // The format's own block. `pixel format` is a GUID rather than a number,
    // and is read as one below.
    (0xbc01, "pixel format"),
    (0xbc02, "spatial transform"),
    (0xbc03, "compression"),
    (0xbc04, "image type"),
    (0xbc05, "ptm colour info"),
    (0xbc06, "profile level container"),
    (0xbc80, "image width"),
    (0xbc81, "image height"),
    (0xbc82, "width resolution"),
    (0xbc83, "height resolution"),
    // The four that place the two codestreams, and the two that say how much
    // of each the writer kept.
    (0xbcc0, "image offset"),
    (0xbcc1, "image byte count"),
    (0xbcc2, "alpha offset"),
    (0xbcc3, "alpha byte count"),
    (0xbcc4, "image band presence"),
    (0xbcc5, "alpha band presence"),
    (0xea1c, "padding"),
];

/// The tags that point at a directory of their own, read against the camera's
/// names and the satellite's rather than against the ones above.
pub const SUB_IFD: &[(i128, tiff::Space)] = &[(0x8769, tiff::Space::Exif), (0x8825, tiff::Space::Gps)];

/// `II`, then the byte that says this is not a TIFF.
pub const MAGIC: &[u8] = b"II\xbc";

pub fn jxr() -> Template {
    let part = tiff::jxr_part();
    Template::new("jxr", part.root.clone()).with_part(&part)
}

/// The file: four bytes of header, and then a directory wherever it says.
///
/// A window of its own, the same as a TIFF's, because every offset inside
/// counts from the start of the format rather than from the start of the file.
/// Here those are the same place, so the window costs nothing and keeps the
/// one answer true.
pub fn jxr_file() -> T {
    T::sized(
        E::Remaining,
        T::structure(
            "JpegXr",
            vec![
                // `II` for the byte order, and `0xbc` where a TIFF writes 42.
                ("signature", T::magic(MAGIC)),
                ("version", T::enumeration("ContainerVersion", T::u8(), &[(0, "hd photo"), (1, "jpeg xr")])),
                ("ifd_offset", T::u32(Little)),
                ("ifd", T::at_in_window(E::field("ifd_offset"), T::Named(tiff::Space::Jxr.named(Little).into()))),
                // The codestreams, the values too big to sit inside an entry,
                // and whatever padding the writer left between them. Each of
                // them is pointed at from the directory above.
                ("body", T::bytes(E::Remaining)),
            ],
        ),
    )
}

/// The fields a JPEG XR directory carries that a TIFF's does not: the two
/// codestreams, placed by what the entries said rather than by where they sit.
///
/// This is the whole reason the directory is read before the picture. Neither
/// offset is at a fixed place in the file, and neither sits beside its own
/// byte count: `image offset` and `image byte count` are separate entries, in
/// whatever order the writer put them, so each of the four numbers is found by
/// searching the entries for the tag that holds it. Naming them here rather
/// than reaching into the search four more times lets a reader see what the
/// directory came to, and see the codestream sitting exactly there.
///
/// A file with no separate alpha picture writes neither of its two entries, so
/// both come back zero and nothing is read.
pub fn codestreams() -> Vec<(&'static str, T)> {
    let found = |tag: i128| E::tagged("entries", &["tag"], tag, &["value", "values"]);
    vec![
        ("image_offset", T::computed(found(0xbcc0))),
        ("image_byte_count", T::computed(found(0xbcc1))),
        ("alpha_offset", T::computed(found(0xbcc2))),
        ("alpha_byte_count", T::computed(found(0xbcc3))),
        ("image", placed("image_offset", "image_byte_count")),
        ("alpha", placed("alpha_offset", "alpha_byte_count")),
    ]
}

/// The entries whose value is a GUID rather than a number: `pixel format`,
/// and nothing else.
///
/// Sixteen bytes never fit in the four an entry has for a value, so what is in
/// the entry is an offset and the GUID is read where it points, the same as
/// any other value too big to sit in place.
pub fn guid_values(e: crate::template::Endian) -> Vec<(i128, T)> {
    vec![(
        0xbc01,
        T::inline_structure(
            "Elsewhere",
            vec![("offset", T::u32(e)), ("values", T::at_in_window(E::field("offset"), pixel_format()))],
        ),
    )]
}

/// What the picture's samples are, as Windows Imaging names them.
///
/// Every format the codec has is one GUID, and every one of those GUIDs is the
/// same fifteen bytes with a different last one. So the last byte is the whole
/// answer, and the fifteen before it are read as the constant they are. A GUID
/// that does not begin the way they all do is left as its sixteen bytes: the
/// last byte of something else means nothing, and naming it would be worse
/// than saying nothing.
fn pixel_format() -> T {
    T::switch(
        E::peek(32, Little),
        vec![(
            0x6fddc324,
            T::structure(
                "PixelFormat",
                vec![("codec", T::magic(GUID_PREFIX)), ("format", T::enumeration_hex("PixelFormatId", T::u8(), PIXEL_FORMAT))],
            ),
        )],
        T::bytes(E::lit(16)),
    )
}

/// The fifteen bytes every JPEG XR pixel format GUID begins with:
/// `6fddc324-4e03-4bfe-b185-3d7776 8dc9`, written the way a GUID is written to
/// a file, which is little-endian for its first three fields and in order for
/// the rest.
const GUID_PREFIX: &[u8] = &[0x24, 0xc3, 0xdd, 0x6f, 0x03, 0x4e, 0xfe, 0x4b, 0xb1, 0x85, 0x3d, 0x77, 0x76, 0x8d, 0xc9];

/// A codestream at the offset one field holds, as long as another says, and
/// nothing at all when the offset is zero.
fn placed(offset: &str, byte_count: &str) -> T {
    T::switch(
        E::field(offset),
        vec![(0, nothing())],
        T::at_in_window(E::field(offset), T::sized(E::field(byte_count), codestream())),
    )
}

/// The coded picture: a signature, the image header, a plane header for the
/// picture and another for an alpha channel woven into it, and then the tiles.
///
/// Everything from the signature to the end of the last plane header is
/// written bit by bit, most significant first, which is how these fields read.
fn codestream() -> T {
    T::structure(
        "Codestream",
        vec![
            ("signature", T::magic(b"WMPHOTO\0")),
            // Four bytes of flags. Which of the fields below exist, and how
            // wide they are, is decided here and nowhere else.
            ("version", bits(4)),
            // The specification reads this nibble as a hard-tiling flag and
            // three reserved bits, which is the same eight values.
            (
                "subversion",
                T::enumeration("CodecSubversion", bits(4), &[(0, "1.0"), (1, "1.1, soft tiles"), (9, "1.1, hard tiles")]),
            ),
            ("tiling", bits(1)),
            ("frequency_mode", T::enumeration("BitstreamOrder", bits(1), &[(0, "spatial"), (1, "frequency")])),
            ("orientation", T::enumeration("Orientation", bits(3), ORIENTATION)),
            ("index_table_present", bits(1)),
            ("overlap", T::enumeration("Overlap", bits(2), OVERLAP)),
            // The one that halves the width and height fields, and the tile
            // sizes with them.
            ("short_header", bits(1)),
            ("long_word", bits(1)),
            ("windowing", bits(1)),
            ("trim_flexbits", bits(1)),
            // Zero in a conformant file. Microsoft's decoder reads it as a
            // tile-stretching flag and takes a byte per tile when it is set,
            // which nothing writes and this does not follow.
            ("reserved_d", bits(1)),
            ("red_blue_not_swapped", bits(1)),
            ("premultiplied_alpha", bits(1)),
            // An alpha channel inside this codestream, which is a second plane
            // header below. Not the same thing as the separate alpha
            // codestream the container points at with two entries of its own.
            ("alpha_image_plane", bits(1)),
            // What the picture is meant to come out as, which is not
            // necessarily how it is coded: the plane header says that.
            ("output_colour_format", T::enumeration("OutputColourFormat", bits(4), OUTPUT_COLOUR_FORMAT)),
            ("output_bit_depth", T::enumeration("OutputBitDepth", bits(4), OUTPUT_BIT_DEPTH)),
            // One less than the real thing, so that a picture may be 2^32 wide
            // and none may be nothing wide.
            ("width_minus1", wide(16, 32)),
            ("height_minus1", wide(16, 32)),
            ("tiles", T::switch(E::field("tiling"), vec![(1, tiles())], nothing())),
            ("window", T::switch(E::field("windowing"), vec![(1, margins())], nothing())),
            ("plane", plane("PlaneHeader")),
            // The alpha plane, when the flag above said there is one. Written
            // exactly like the plane before it, and starting on a byte because
            // that one padded itself to one.
            ("alpha_plane", T::switch(E::field("alpha_image_plane"), vec![(1, plane("AlphaPlaneHeader"))], nothing())),
            // The index table, the tile headers and the coded coefficients.
            // Reading these is decoding the picture.
            ("coded_tiles", T::bytes(E::Remaining)),
        ],
    )
}

/// How the picture is cut up. Two counts of one less than the real number, and
/// then the size of every tile but the last, which is whatever is left over.
fn tiles() -> T {
    T::inline_structure(
        "Tiles",
        vec![
            ("vertical_minus1", bits(12)),
            ("horizontal_minus1", bits(12)),
            ("widths", T::array(wide(8, 16), E::field("vertical_minus1")).counted_as("tile")),
            ("heights", T::array(wide(8, 16), E::field("horizontal_minus1")).counted_as("tile")),
        ],
    )
}

/// How far in from each edge the picture a reader wants sits, for a file coded
/// larger than it is shown. The transform works in blocks of sixteen, so a
/// picture whose size is not a multiple of sixteen is coded to the next one up
/// and trimmed back here.
fn margins() -> T {
    T::inline_structure(
        "Window",
        vec![("top", bits(6)), ("left", bits(6)), ("bottom", bits(6)), ("right", bits(6))],
    )
}

/// One image plane header: how the picture is coded, and the quantization it
/// was coded with.
///
/// The colour format decides how many channels there are and whether four bits
/// of chroma centring follow it; the bit depth the image header read decides
/// whether a shift or a mantissa length comes after that. Then the
/// quantization, which is three questions asked in a chain: whether one DC
/// quantizer covers the whole plane, and if the picture has more than the DC
/// band, whether the low-pass band reuses that one, and if it has a high-pass
/// band, whether that reuses the low-pass. Every "no" costs a bit and may add
/// a quantizer of its own.
fn plane(name: &str) -> T {
    T::structure(
        name,
        vec![
            ("internal_colour_format", T::enumeration("InternalColourFormat", bits(3), INTERNAL_COLOUR_FORMAT)),
            // Lossless coding, which the format calls scaled arithmetic.
            ("scaled", bits(1)),
            ("bands_present", T::enumeration("BandsPresent", bits(4), BANDS_PRESENT)),
            // Four bits about where the chroma samples sit for the formats
            // that have chroma to place, four saying how many channels there
            // are for the format that does not say, and nothing for the two
            // that have already said everything.
            ("colour", colour()),
            // How a wide or floating-point sample was squeezed into the coded
            // one: a shift for the integer depths, a mantissa length and an
            // exponent bias for the float.
            ("depth", depth()),
            // Whether one DC quantizer covers the plane. Where it does not,
            // every tile carries its own and none of them is here.
            ("dc_uniform", bits(1)),
            ("dc_quantizer", T::switch(E::field("dc_uniform"), vec![(1, quantizer("DcQuantizer"))], nothing())),
            // The low-pass and high-pass bands, each only when the picture
            // kept them. `bands present` counts what was thrown away: 3 is the
            // DC band alone, 2 is that and the low-pass.
            ("lowpass", T::switch(unequal("bands_present", 3), vec![(1, band("Lowpass", "before the low-pass"))], nothing())),
            (
                "highpass",
                T::switch(
                    unequal("bands_present", 3).mul(unequal("bands_present", 2)),
                    vec![(1, band("Highpass", "before the high-pass"))],
                    nothing(),
                ),
            ),
            // Up to seven bits of nothing, so that the plane after this one,
            // or the coded picture, begins on a byte.
            ("padding", flush_to_byte()),
        ],
    )
}

/// The four bits after the bands, which say something different for each
/// colour format and are not written at all for two of them.
fn colour() -> T {
    let centring = |name: &str| T::inline_structure(name, vec![("reserved", bits(1)), ("centring", bits(3))]);
    T::switch(
        E::field("internal_colour_format"),
        vec![
            (1, T::inline_structure("Chroma", vec![("x", centring("ChromaX")), ("y", centring("ChromaY"))])),
            (2, T::inline_structure("Chroma", vec![("x", centring("ChromaX")), ("reserved", bits(4))])),
            (3, T::inline_structure("Chroma", vec![("reserved_f", bits(4)), ("reserved_h", bits(4))])),
            (6, T::inline_structure("Components", vec![("components_minus1", bits(4)), ("reserved", bits(4))])),
        ],
        nothing(),
    )
}

/// The extra byte, or two, that a wide or floating-point picture writes here.
/// `output bit depth` was read in the image header, which is the structure
/// this one sits in, so naming it reaches it.
fn depth() -> T {
    let shift = || T::inline_structure("Shift", vec![("shift_bits", T::u8())]);
    T::switch(
        E::field("output_bit_depth"),
        vec![
            (2, shift()),
            (3, shift()),
            (5, shift()),
            (6, shift()),
            (7, T::inline_structure("Float", vec![("mantissa_length", T::u8()), ("exponent_bias", T::Int { bits: 8, endian: Big })])),
        ],
        nothing(),
    )
}

/// A band that may quantize on its own or reuse the band before it. The first
/// bit says which; where it does not reuse, a second says whether one
/// quantizer covers the plane, and only then is one written.
fn band(name: &str, reused: &str) -> T {
    T::structure(
        name,
        vec![
            ("reuse", T::enumeration("Reuse", bits(1), &[(0, "its own quantizer"), (1, reused)])),
            (
                "own",
                T::switch(
                    E::field("reuse"),
                    vec![(
                        0,
                        T::inline_structure(
                            "Own",
                            vec![
                                ("uniform", bits(1)),
                                ("quantizer", T::switch(E::field("uniform"), vec![(1, quantizer("Quantizer"))], nothing())),
                            ],
                        ),
                    )],
                    nothing(),
                ),
            ),
        ],
    )
}

/// One quantizer: a mode, and then one index, two, or one per channel.
///
/// A picture of one channel has nothing to choose between, so it writes no
/// mode at all and its one index follows straight away. Which is why the count
/// has to be worked out before anything here can be read, and why an
/// `n-component` plane of a single component reads like a monochrome one
/// rather than like the six-channel plane beside it.
fn quantizer(name: &str) -> T {
    T::structure(
        name,
        vec![
            ("channels", channels()),
            ("mode", T::switch(alone(), vec![(1, nothing())], T::enumeration("ChannelMode", bits(2), CHANNEL_MODE))),
            ("luma", T::u8()),
            // One more index covering both chroma channels, or one for each of
            // the channels after the first, or none at all.
            ("chroma", T::switch(alone(), vec![(1, nothing())], by_mode())),
        ],
    )
}

/// One when this plane has a single channel, and the two bits of mode are not
/// there to read.
fn alone() -> E {
    E::field("channels").less_than(E::lit(2))
}

/// What follows the first index when there is more than one channel.
fn by_mode() -> T {
    T::switch(E::field("mode"), vec![(0, nothing()), (1, T::u8())], T::array(T::u8(), E::field("channels").sub(E::lit(1))))
}

/// How many channels the plane's colour format implies, in a field of no bits,
/// because the count is what the indices above are counted by and the file
/// never writes it.
///
/// One for luma alone, three for the three `YUV` formats, four for either
/// CMYK, and for `n-component` the number the plane header wrote plus one.
fn channels() -> T {
    T::switch(
        E::field("internal_colour_format"),
        vec![
            (1, T::computed(E::lit(3))),
            (2, T::computed(E::lit(3))),
            (3, T::computed(E::lit(3))),
            (4, T::computed(E::lit(4))),
            (5, T::computed(E::lit(4))),
            (6, T::computed(E::within(&["colour", "components_minus1"]).add(E::lit(1)))),
        ],
        T::computed(E::lit(1)),
    )
}

/// However many bits it takes to reach the next byte, and nothing where the
/// run already ended on one.
///
/// The distance is the length of everything above this in the plane header,
/// added up, which is the one thing a header of conditional bit fields cannot
/// say about itself. Eight cases rather than a width worked out as it is read,
/// because how wide a field is belongs to the template rather than to the run.
fn flush_to_byte() -> T {
    let so_far = ["internal_colour_format", "scaled", "bands_present", "colour", "depth", "dc_uniform", "dc_quantizer", "lowpass", "highpass"]
        .iter()
        .map(|f| E::bits_of(f))
        .reduce(|a, b| a.add(b))
        .expect("some fields");
    T::switch(so_far.pad_to(8), (1..8).map(|n| (n as i128, bits(n))).collect(), nothing())
}

/// A field that is the narrow width or the broad one depending on what
/// `short header` said, which is how this format keeps a small picture's
/// header small.
fn wide(narrow: u32, broad: u32) -> T {
    T::switch(E::field("short_header"), vec![(1, bits(narrow))], bits(broad))
}

/// One when the field does not hold that value. There is no test for "not
/// equal", and a band that is read unless one number was written needs one:
/// smaller than it, or else larger.
fn unequal(field: &str, v: i128) -> E {
    E::field(field).less_than(E::lit(v)).or(E::lit(v).less_than(E::field(field)))
}

/// A run of bits, packed most significant first, which is how everything in a
/// codestream is written.
fn bits(n: u32) -> T {
    T::UInt { bits: n, endian: Big }
}

/// No bits at all: what a field that is not written comes to.
fn nothing() -> T {
    T::bytes(E::lit(0))
}

/// How the picture is turned the right way up: the rotation first, then the
/// flips.
const ORIENTATION: &[(i128, &str)] = &[
    (0, "as coded"),
    (1, "flip vertical"),
    (2, "flip horizontal"),
    (3, "rotate 180"),
    (4, "rotate 90 clockwise"),
    (5, "rotate 90 clockwise, flip vertical"),
    (6, "rotate 90 clockwise, flip horizontal"),
    (7, "rotate 270 clockwise"),
];

/// How far the overlap filter reaches across a block edge, which is what keeps
/// a heavily compressed picture from showing its blocks.
const OVERLAP: &[(i128, &str)] = &[(0, "none"), (1, "one level"), (2, "two levels")];

/// What the picture is meant to be turned back into.
const OUTPUT_COLOUR_FORMAT: &[(i128, &str)] = &[
    (0, "luma only"),
    (1, "yuv 4:2:0"),
    (2, "yuv 4:2:2"),
    (3, "yuv 4:4:4"),
    (4, "cmyk"),
    (5, "cmyk direct"),
    (6, "n-component"),
    (7, "rgb"),
    (8, "rgbe"),
];

/// How it is coded, which may be fewer channels than it comes out as: an
/// encoder turns RGB into luma and chroma before it codes anything.
const INTERNAL_COLOUR_FORMAT: &[(i128, &str)] = &[
    (0, "luma only"),
    (1, "yuv 4:2:0"),
    (2, "yuv 4:2:2"),
    (3, "yuv 4:4:4"),
    (4, "cmyk"),
    (5, "cmyk direct"),
    (6, "n-component"),
];

/// What one sample is worth. The two ends of the table are the same one bit
/// read opposite ways round.
const OUTPUT_BIT_DEPTH: &[(i128, &str)] = &[
    (0, "1 bit, white first"),
    (1, "8 bit"),
    (2, "16 bit"),
    (3, "16 bit signed"),
    (4, "16 bit float"),
    (6, "32 bit signed"),
    (7, "32 bit float"),
    (8, "5 bit"),
    (9, "10 bit"),
    (10, "5:6:5"),
    (15, "1 bit, black first"),
];

/// Which bands of the transform survived. A file may be written with the
/// finest detail left out, and it is then smaller and still a picture.
const BANDS_PRESENT: &[(i128, &str)] = &[
    (0, "all"),
    (1, "no flexbits"),
    (2, "no highpass"),
    (3, "dc only"),
    (4, "isolated"),
];

/// Whether the channels share one quantizer, split luma from chroma, or each
/// have one of their own.
const CHANNEL_MODE: &[(i128, &str)] = &[(0, "uniform"), (1, "luma and chroma"), (2, "independent"), (3, "independent")];

/// The last byte of the GUID, which is the pixel format itself. `bpp` is bits
/// a pixel across every channel; `p` before a format with alpha means the
/// colour channels were already multiplied by it.
const PIXEL_FORMAT: &[(i128, &str)] = &[
    (0x00, "don't care"),
    (0x05, "black and white"),
    (0x08, "8bpp grey"),
    (0x09, "16bpp rgb555"),
    (0x0a, "16bpp rgb565"),
    (0x0b, "16bpp grey"),
    (0x0c, "24bpp bgr"),
    (0x0d, "24bpp rgb"),
    (0x0e, "32bpp bgr"),
    (0x0f, "32bpp bgra"),
    (0x10, "32bpp pbgra"),
    (0x11, "32bpp grey float"),
    (0x12, "48bpp rgb fixed point"),
    (0x13, "16bpp grey fixed point"),
    (0x14, "32bpp rgb101010"),
    (0x15, "48bpp rgb"),
    (0x16, "64bpp rgba"),
    (0x17, "64bpp prgba"),
    (0x18, "96bpp rgb fixed point"),
    (0x19, "128bpp rgba float"),
    (0x1a, "128bpp prgba float"),
    (0x1b, "128bpp rgb float"),
    (0x1c, "32bpp cmyk"),
    (0x1d, "64bpp rgba fixed point"),
    (0x1e, "128bpp rgba fixed point"),
    (0x1f, "64bpp cmyk"),
    (0x20, "24bpp 3 channels"),
    (0x21, "32bpp 4 channels"),
    (0x22, "40bpp 5 channels"),
    (0x23, "48bpp 6 channels"),
    (0x24, "56bpp 7 channels"),
    (0x25, "64bpp 8 channels"),
    (0x26, "48bpp 3 channels"),
    (0x27, "64bpp 4 channels"),
    (0x28, "80bpp 5 channels"),
    (0x29, "96bpp 6 channels"),
    (0x2a, "112bpp 7 channels"),
    (0x2b, "128bpp 8 channels"),
    (0x2c, "40bpp cmyk alpha"),
    (0x2d, "80bpp cmyk alpha"),
    (0x2e, "32bpp 3 channels alpha"),
    (0x2f, "40bpp 4 channels alpha"),
    (0x30, "48bpp 5 channels alpha"),
    (0x31, "56bpp 6 channels alpha"),
    (0x32, "64bpp 7 channels alpha"),
    (0x33, "72bpp 8 channels alpha"),
    (0x34, "64bpp 3 channels alpha"),
    (0x35, "80bpp 4 channels alpha"),
    (0x36, "96bpp 5 channels alpha"),
    (0x37, "112bpp 6 channels alpha"),
    (0x38, "128bpp 7 channels alpha"),
    (0x39, "144bpp 8 channels alpha"),
    (0x3a, "64bpp rgba half"),
    (0x3b, "48bpp rgb half"),
    (0x3d, "32bpp rgbe"),
    (0x3e, "16bpp grey half"),
    (0x3f, "32bpp grey fixed point"),
    (0x40, "64bpp rgb fixed point"),
    (0x41, "128bpp rgb fixed point"),
    (0x42, "64bpp rgb half"),
    (0x43, "80bpp cmyk direct alpha"),
    (0x44, "12bpp ycc420"),
    (0x45, "16bpp ycc422"),
    (0x46, "20bpp ycc422"),
    (0x47, "32bpp ycc422"),
    (0x48, "24bpp ycc444"),
    (0x49, "30bpp ycc444"),
    (0x4a, "48bpp ycc444"),
    (0x4b, "48bpp ycc444 fixed point"),
    (0x4c, "20bpp ycc420 alpha"),
    (0x4d, "24bpp ycc422 alpha"),
    (0x4e, "30bpp ycc422 alpha"),
    (0x4f, "48bpp ycc422 alpha"),
    (0x50, "32bpp ycc444 alpha"),
    (0x51, "40bpp ycc444 alpha"),
    (0x52, "64bpp ycc444 alpha"),
    (0x53, "64bpp ycc444 alpha fixed point"),
    (0x54, "32bpp cmyk direct"),
    (0x55, "64bpp cmyk direct"),
    (0x56, "40bpp cmyk direct alpha"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// A whole small file: five entries, a pixel format GUID after them, and a
    /// codestream at the end that the entries point at.
    ///
    /// Sixty-four by thirty-two, coded as 4:4:4 with every band kept and one
    /// quantizer over the whole plane. `alpha` is what makes the second pair
    /// of entries and a second codestream, which is what a file with a
    /// separate alpha picture writes.
    fn file(alpha: bool) -> Vec<u8> {
        let u16b = |v: u16| v.to_le_bytes().to_vec();
        let u32b = |v: u32| v.to_le_bytes().to_vec();
        let entry = |tag: u16, kind: u16, count: u32, value: u32| {
            let mut v = u16b(tag);
            v.extend_from_slice(&u16b(kind));
            v.extend_from_slice(&u32b(count));
            v.extend_from_slice(&u32b(value));
            v
        };

        let entries: u32 = if alpha { 7 } else { 5 };
        let ifd_at: u32 = 8;
        let guid_at = ifd_at + 2 + entries * 12 + 4;
        let image_at = guid_at + 16;
        let image_len = CODESTREAM.len() as u32;
        let alpha_at = image_at + image_len;

        let mut v = MAGIC.to_vec();
        v.push(1);
        v.extend_from_slice(&u32b(ifd_at));
        assert_eq!(v.len() as u32, ifd_at);

        v.extend_from_slice(&u16b(entries as u16));
        v.extend_from_slice(&entry(0xbc01, 1, 16, guid_at));
        v.extend_from_slice(&entry(0xbc80, 4, 1, 64));
        v.extend_from_slice(&entry(0xbc81, 4, 1, 32));
        // Deliberately after the byte count, since the two are separate
        // entries and nothing says which of them a writer puts first.
        v.extend_from_slice(&entry(0xbcc1, 4, 1, image_len));
        v.extend_from_slice(&entry(0xbcc0, 4, 1, image_at));
        if alpha {
            v.extend_from_slice(&entry(0xbcc2, 4, 1, alpha_at));
            v.extend_from_slice(&entry(0xbcc3, 4, 1, image_len));
        }
        v.extend_from_slice(&u32b(0));
        assert_eq!(v.len() as u32, guid_at);

        v.extend_from_slice(GUID_PREFIX);
        v.push(0x0d); // 24bpp rgb
        assert_eq!(v.len() as u32, image_at);

        v.extend_from_slice(CODESTREAM);
        if alpha {
            v.extend_from_slice(CODESTREAM);
        }
        v
    }

    /// The coded picture, written out bit by bit.
    ///
    /// The four bytes after the signature are the flags: version 1 and
    /// subversion 1; no tiling, spatial order, no rotation, an index table,
    /// one level of overlap; a short header, so the size is sixteen bits
    /// rather than thirty-two, red and blue not swapped, no alpha plane; and
    /// then RGB out at eight bits.
    ///
    /// The plane header after the size is 4:4:4 with every band kept, one DC
    /// quantizer of 5 shared by all three channels, and the low-pass and
    /// high-pass bands each reusing the band before. That comes to
    /// twenty-nine bits, so three bits of padding end it on a byte.
    const CODESTREAM: &[u8] = &[
        b'W', b'M', b'P', b'H', b'O', b'T', b'O', 0,
        0x11, 0x05, 0x84, 0x71,
        0x00, 0x3f, // width less one
        0x00, 0x1f, // height less one
        0x60, 0x00, 0x80, 0xb8, // the plane header
        0xde, 0xad, 0xbe, 0xef, // where the tiles would be
    ];

    /// Where the directory sits in the tree, and where the codestream does.
    const IFD: &[usize] = &[3, 0];

    fn at(tail: &[usize]) -> Vec<usize> {
        [IFD, tail].concat()
    }

    #[test]
    fn a_jpeg_xr_is_told_from_a_tiff_by_the_byte_where_a_tiff_writes_42() {
        assert_eq!(super::super::sniff(&file(false), 114), Some("jxr"));
        // And the two the format is otherwise identical to still read as what
        // they are.
        assert_eq!(super::super::sniff(b"II*\x00\x08\x00\x00\x00", 8), Some("tiff"));
        assert_eq!(super::super::sniff(b"MM\x00*\x00\x00\x00\x08", 8), Some("tiff"));
    }

    #[test]
    fn the_directory_reads_against_jpeg_xrs_names_rather_than_tiffs() {
        let d = Document::new(MemSource(file(false)));
        let mut ev = Evaluator::new(jxr());
        let ifd = ev.node(&d, IFD).unwrap();
        assert_eq!(ifd.type_name, "JxrIfd");
        assert_eq!(ev.node(&d, &at(&[0])).unwrap().value, Value::UInt(5));
        // 0xbc80 is a tag no TIFF has, and 0xbc01 holds a GUID.
        assert_eq!(
            ev.node(&d, &at(&[1, 1, 0])).unwrap().value,
            Value::Enum { raw: 0xbc80, name: Some("image width".into()), hex: false }
        );
        assert_eq!(ev.node(&d, &at(&[1, 1, 4, 0])).unwrap().value, Value::UInt(64));
        assert_eq!(ev.node(&d, &at(&[1, 2, 4, 0])).unwrap().value, Value::UInt(32));
    }

    #[test]
    fn the_pixel_format_is_the_last_byte_of_a_guid_the_rest_of_which_never_changes() {
        let d = Document::new(MemSource(file(false)));
        let mut ev = Evaluator::new(jxr());
        // The entry holds an offset, because sixteen bytes never fit in four.
        assert_eq!(ev.node(&d, &at(&[1, 0, 4, 0])).unwrap().value, Value::UInt(74));
        let guid = ev.node(&d, &at(&[1, 0, 4, 1, 0])).unwrap();
        assert_eq!(guid.type_name, "PixelFormat");
        assert_eq!(guid.offset_bits, 74 * 8);
        assert_eq!(
            ev.node(&d, &at(&[1, 0, 4, 1, 0, 1])).unwrap().value,
            Value::Enum { raw: 0x0d, name: Some("24bpp rgb".into()), hex: true }
        );
    }

    #[test]
    fn the_codestream_is_found_by_searching_the_entries_rather_than_by_reading_them_in_order() {
        let d = Document::new(MemSource(file(false)));
        let mut ev = Evaluator::new(jxr());
        // The byte count was written before the offset, and both are found.
        assert_eq!(ev.node(&d, &at(&[3])).unwrap().value, Value::Int(90), "image offset");
        assert_eq!(ev.node(&d, &at(&[4])).unwrap().value, Value::Int(24), "image byte count");
        assert_eq!(ev.node(&d, &at(&[5])).unwrap().value, Value::Int(0), "no separate alpha");
        let stream = ev.node(&d, &at(&[7, 0])).unwrap();
        assert_eq!(stream.type_name, "Codestream");
        assert_eq!(stream.offset_bits, 90 * 8);
        assert_eq!(stream.size_bits, 24 * 8);
        // With no alpha entries there is no second codestream, and the field
        // that would hold one covers nothing.
        assert_eq!(ev.node(&d, &at(&[8])).unwrap().size_bits, 0);
    }

    #[test]
    fn the_flags_after_the_signature_are_read_a_bit_at_a_time() {
        let d = Document::new(MemSource(file(false)));
        let mut ev = Evaluator::new(jxr());
        let f = |tail: &[usize]| at(&[&[7usize, 0][..], tail].concat());
        assert_eq!(ev.node(&d, &f(&[1])).unwrap().value, Value::UInt(1), "version");
        assert_eq!(
            ev.node(&d, &f(&[2])).unwrap().value,
            Value::Enum { raw: 1, name: Some("1.1, soft tiles".into()), hex: false }
        );
        assert_eq!(ev.node(&d, &f(&[3])).unwrap().value, Value::UInt(0), "tiling");
        assert_eq!(ev.node(&d, &f(&[6])).unwrap().value, Value::UInt(1), "index table present");
        assert_eq!(
            ev.node(&d, &f(&[7])).unwrap().value,
            Value::Enum { raw: 1, name: Some("one level".into()), hex: false }
        );
        assert_eq!(ev.node(&d, &f(&[8])).unwrap().value, Value::UInt(1), "short header");
        assert_eq!(
            ev.node(&d, &f(&[16])).unwrap().value,
            Value::Enum { raw: 7, name: Some("rgb".into()), hex: false }
        );
        assert_eq!(
            ev.node(&d, &f(&[17])).unwrap().value,
            Value::Enum { raw: 1, name: Some("8 bit".into()), hex: false }
        );
        // A short header, so the size is two bytes each rather than four, and
        // one less than it really is.
        let w = ev.node(&d, &f(&[18])).unwrap();
        assert_eq!((w.value, w.size_bits), (Value::UInt(63), 16));
        assert_eq!(ev.node(&d, &f(&[19])).unwrap().value, Value::UInt(31));
        // Neither tiling nor windowing, so neither costs anything.
        assert_eq!(ev.node(&d, &f(&[20])).unwrap().size_bits, 0);
        assert_eq!(ev.node(&d, &f(&[21])).unwrap().size_bits, 0);
    }

    #[test]
    fn the_plane_header_pads_itself_out_to_the_next_byte() {
        let d = Document::new(MemSource(file(false)));
        let mut ev = Evaluator::new(jxr());
        let p = |tail: &[usize]| at(&[&[7usize, 0, 22][..], tail].concat());
        let plane = ev.node(&d, &p(&[])).unwrap();
        assert_eq!(plane.type_name, "PlaneHeader");
        assert_eq!(plane.size_bits, 32, "four bytes, of which three bits are padding");
        assert_eq!(
            ev.node(&d, &p(&[0])).unwrap().value,
            Value::Enum { raw: 3, name: Some("yuv 4:4:4".into()), hex: false }
        );
        assert_eq!(
            ev.node(&d, &p(&[2])).unwrap().value,
            Value::Enum { raw: 0, name: Some("all".into()), hex: false }
        );
        // 4:4:4 writes four reserved bits and four more, and an eight-bit
        // picture writes nothing about its depth.
        assert_eq!(ev.node(&d, &p(&[3])).unwrap().size_bits, 8);
        assert_eq!(ev.node(&d, &p(&[4])).unwrap().size_bits, 0);
        // One quantizer over the plane: three channels, one mode, one index.
        assert_eq!(ev.node(&d, &p(&[5])).unwrap().value, Value::UInt(1), "dc uniform");
        assert_eq!(ev.node(&d, &p(&[6, 0])).unwrap().value, Value::Int(3), "channels");
        assert_eq!(
            ev.node(&d, &p(&[6, 1])).unwrap().value,
            Value::Enum { raw: 0, name: Some("uniform".into()), hex: false }
        );
        assert_eq!(ev.node(&d, &p(&[6, 2])).unwrap().value, Value::UInt(5), "the quantizer itself");
        assert_eq!(ev.node(&d, &p(&[6, 3])).unwrap().size_bits, 0, "uniform, so no second index");
        // Both other bands reuse the one before, which costs a bit each.
        assert_eq!(ev.node(&d, &p(&[7, 0])).unwrap().value, Value::Enum { raw: 1, name: Some("before the low-pass".into()), hex: false });
        assert_eq!(ev.node(&d, &p(&[7, 1])).unwrap().size_bits, 0);
        assert_eq!(ev.node(&d, &p(&[8, 0])).unwrap().size_bits, 1);
        // Twenty-nine bits of header, so three of padding.
        assert_eq!(ev.node(&d, &p(&[9])).unwrap().size_bits, 3);
        // And the coded picture starts on the byte after it.
        let tiles = ev.node(&d, &at(&[7, 0, 24])).unwrap();
        assert_eq!(tiles.offset_bits % 8, 0);
        assert_eq!(tiles.size_bits, 4 * 8);
    }

    #[test]
    fn a_file_that_keeps_its_alpha_as_a_picture_of_its_own_has_a_second_codestream() {
        let d = Document::new(MemSource(file(true)));
        let mut ev = Evaluator::new(jxr());
        // Two more entries, so the directory is longer and everything after it
        // has moved; the offsets are read rather than assumed.
        let image_at = 8 + 2 + 7 * 12 + 4 + 16;
        assert_eq!(ev.node(&d, &at(&[3])).unwrap().value, Value::Int(image_at));
        assert_eq!(ev.node(&d, &at(&[5])).unwrap().value, Value::Int(image_at + 24));
        let alpha = ev.node(&d, &at(&[8, 0])).unwrap();
        assert_eq!(alpha.type_name, "Codestream");
        assert_eq!(alpha.offset_bits, (image_at as u64 + 24) * 8);
        assert_eq!(alpha.size_bits, 24 * 8);
    }

    #[test]
    fn every_field_of_the_codestream_knows_where_its_bits_are() {
        let d = Document::new(MemSource(file(false)));
        let mut ev = Evaluator::new(jxr());
        let f = |tail: &[usize]| at(&[&[7usize, 0][..], tail].concat());
        // The signature is eight bytes from the start of the codestream, and
        // the flags are the four bits and one bit and three bits after it.
        assert_eq!(ev.node(&d, &f(&[0])).unwrap().offset_bits, 90 * 8);
        assert_eq!(ev.node(&d, &f(&[1])).unwrap().offset_bits, 98 * 8);
        let overlap = ev.node(&d, &f(&[7])).unwrap();
        assert_eq!((overlap.offset_bits, overlap.size_bits), (99 * 8 + 6, 2));
        // The cursor lands on `body` for a byte of the codestream, the same as
        // it does for a byte of a TIFF's image: what the directory placed sits
        // inside the run that covers everything after the header, and the run
        // is what the walk from the root reaches first.
        assert_eq!(ev.locate(&d, 99 * 8 + 6).unwrap(), vec![4]);
        // A byte of the directory is a byte of the directory, because that is
        // the one thing the run does not also cover.
        assert_eq!(ev.locate(&d, 8 * 8).unwrap(), at(&[0]));
    }
}
