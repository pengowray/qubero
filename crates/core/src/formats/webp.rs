//! WebP: a RIFF file whose form is `WEBP`, holding one image as lossy VP8
//! data, lossless VP8L data, or, in the extended format, a `VP8X` header and
//! the chunks it lists: an ICC profile, alpha, animation frames, EXIF and XMP.
//! RFC 9649 describes the container and the lossless format. A lossy image is
//! one VP8 key frame, which RFC 6386 describes.
//!
//! The chunks are RIFF chunks, read by `wav::riff`, the same RIFF reader WAVE
//! uses, so the size rules and the pad byte after an odd-sized chunk are
//! written once, in `wav.rs`. An `ANMF` frame holds chunks of its own, and
//! they are read as the same `Chunk` type as the file's top-level chunks.
//!
//! Every byte that holds bit fields is a structure drawn on one row, so the
//! annotation column has an entry for the byte that names what is in it. The
//! fields of VP8 and VP8L are packed least-significant bit first and cross
//! byte boundaries, and without that row the column, which steps through the
//! file from the top bit of each byte, lost its place at the first of them and
//! showed the rest of the file as one gap.
//!
//! Both image formats are entropy-coded below their headers. VP8 is a boolean
//! (arithmetic) code and VP8L a set of Huffman codes read least-significant
//! bit first, and neither can be cut into fields without decoding it. So the
//! headers are fields, bit by bit, and each coded stream is one named run of
//! bytes with a doc saying what it holds. The same goes for the alpha plane in
//! `ALPH`, and, until a shared ICC reader lands, for the ICC profile.
//!
//! The `EXIF` chunk is a TIFF file, read by the TIFF template the way a JPEG's
//! `APP1` segment is, with offsets counted from the TIFF's own start. Some
//! writers put JPEG's `Exif\0\0` in front of it, and that is read when it is
//! there.

use super::wav;
use crate::template::{Colour, Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T, Until, Valid};

/// A chunk id as the big-endian number a switch on a text field compares.
const fn cc(id: &[u8; 4]) -> i128 {
    u32::from_be_bytes(*id) as i128
}

/// A field of `n` bits taken from the top of the byte down, which is how the
/// spec draws the flag bytes of `VP8X`, `ALPH` and `ANMF`.
fn high_bits(n: u32) -> T {
    T::UInt { bits: n, endian: Big }
}

/// A field of `n` bits taken from the bottom of the byte up, which is how VP8
/// packs its frame tag and sizes and how VP8L reads its whole bitstream.
fn low_bits(n: u32) -> T {
    T::UInt { bits: n, endian: Little }
}

/// The container's 24-bit little-endian numbers.
fn u24() -> T {
    T::UInt { bits: 24, endian: Little }
}

/// A stored number plus one, which is how WebP writes every width and height.
fn plus_one(field: &str) -> T {
    T::computed(E::field(field).add(E::lit(1)))
}

const RESERVED_DOC: &str = "Reserved bits. Writers must set them to 0, and readers ignore them.";

pub fn webp() -> Template {
    let exif = super::tiff::tiff_part();
    wav::riff("webp", chunk_body(&exif.root)).with_part(&exif)
}

/// What a chunk's payload reads as, by its id. An id not named here is its
/// bytes: the spec tells readers to skip chunks they do not know.
fn chunk_body(tiff: &T) -> T {
    T::switch(
        E::field("id"),
        vec![
            (cc(b"VP8X"), vp8x()),
            (cc(b"VP8 "), vp8()),
            (cc(b"VP8L"), vp8l()),
            (cc(b"ALPH"), alph()),
            (cc(b"ANIM"), anim()),
            (cc(b"ANMF"), anmf()),
            (cc(b"ICCP"), icc_profile()),
            (cc(b"EXIF"), exif(tiff)),
            (cc(b"XMP "), T::text(StrLen::Fixed(E::Remaining), Encoding::Utf8)),
        ],
        T::bytes(E::Remaining),
    )
}

/// `VP8X`: which optional features the file uses, and the canvas size.
fn vp8x() -> T {
    let flags = T::inline_structure(
        "VP8XFlags",
        vec![
            ("reserved_high", high_bits(2)),
            ("icc_profile", high_bits(1)),
            ("alpha", high_bits(1)),
            ("exif", high_bits(1)),
            ("xmp", high_bits(1)),
            ("animation", high_bits(1)),
            ("reserved_low", high_bits(1)),
        ],
    )
    .reads_as(&[
        ("icc_profile", "icc_profile", ""),
        ("alpha", "alpha", ""),
        ("exif", "exif", ""),
        ("xmp", "xmp", ""),
        ("animation", "animation", ""),
    ])
    .field_doc("reserved_high", RESERVED_DOC)
    .field_doc("icc_profile", "1 if the file has an ICCP chunk with an ICC colour profile. Without an ICCP chunk, viewers assume sRGB.")
    .field_doc("alpha", "1 if any frame of the image contains transparency (alpha) information.")
    .field_doc("exif", "1 if the file has EXIF metadata in an EXIF chunk.")
    .field_doc("xmp", "1 if the file has XMP metadata in an XMP chunk.")
    .field_doc("animation", "1 if the image is animated. The animation's settings are then in the ANIM chunk, and its frames are in ANMF chunks.")
    .field_doc("reserved_low", "Reserved bit. Writers must set it to 0, and readers ignore it.");
    T::structure(
        "VP8X",
        vec![
            ("flags", flags),
            ("reserved", u24()),
            ("canvas_width_minus_one", u24()),
            ("canvas_height_minus_one", u24()),
            ("canvas_width", plus_one("canvas_width_minus_one")),
            ("canvas_height", plus_one("canvas_height_minus_one")),
        ],
    )
    .doc("The extended format header: flags for the features the file uses (ICC profile, alpha, EXIF, XMP and animation) and the canvas size in pixels.")
    .field_doc("flags", "The feature flags, from the top bit down: 2 reserved bits, icc_profile, alpha, exif, xmp, animation and 1 reserved bit.")
    .field_doc("reserved", RESERVED_DOC)
    .field_doc("canvas_width_minus_one", "The canvas width in pixels, minus 1.")
    .field_doc("canvas_height_minus_one", "The canvas height in pixels, minus 1.")
    .field_doc("canvas_width", "The canvas width in pixels: canvas_width_minus_one + 1. canvas_width × canvas_height must be at most 2^32 - 1 pixels.")
    .field_doc("canvas_height", "The canvas height in pixels: canvas_height_minus_one + 1. canvas_width × canvas_height must be at most 2^32 - 1 pixels.")
}

/// How much a VP8 frame is meant to be scaled up after it is decoded.
const VP8_SCALE: &[(i128, &str)] = &[(0, "none"), (1, "×5/4"), (2, "×5/3"), (3, "×2")];

/// `VP8 `: one VP8 key frame (RFC 6386, sections 9.1 and 19.1).
///
/// The frame tag is three bytes read as one little-endian number and taken
/// apart from its lowest bit: `key_frame = tmp & 1`, `version = (tmp >> 1) &
/// 7`, and so on, which is a run of fields read low bit first. The width and
/// height are each a little-endian 16-bit number with the size in its low 14
/// bits and the scale in its top 2, so those are fields read low bit first
/// too. A key frame follows the tag with a start code and the two sizes, ten
/// bytes in all, and an interframe goes straight on to the first partition.
/// WebP allows only key frames, but the tag says which this is, so the
/// fields after it are read only for a key frame.
fn vp8() -> T {
    let tag = |name: &str| E::within(&["frame_tag", name]);
    let key = || tag("frame_type").equal_to(E::lit(0));
    let frame_tag = T::inline_structure(
        "FrameTag",
        vec![
            ("frame_type", T::enumeration("VP8FrameType", low_bits(1), &[(0, "key frame"), (1, "interframe")])),
            ("version", low_bits(3)),
            ("show_frame", low_bits(1)),
            ("first_partition_size", low_bits(19)),
        ],
    )
    .field_valid("frame_type", Valid::Eq(E::lit(0)))
    .reads_as(&[
        ("frame_type", "", ""),
        ("version", "version", ""),
        ("show_frame", "show_frame", ""),
        ("first_partition_size", "first partition {} bytes", ""),
    ])
    .field_doc("frame_type", "0 for a key frame, 1 for an interframe. A VP8 chunk in a WebP file must contain a key frame.")
    .field_doc(
        "version",
        "The decoding profile, from 0 to 3; 4 to 7 are reserved. 0: bicubic reconstruction filter, normal loop filter. 1: bilinear reconstruction filter, simple loop filter. 2: bilinear reconstruction filter, no loop filter. 3: no reconstruction filter (full-pixel motion vectors), no loop filter. Decoders take the loop filter type from the compressed frame header in first_partition.",
    )
    .field_doc("show_frame", "1 if the frame is meant to be displayed.")
    .field_doc("first_partition_size", "The size of first_partition in bytes. first_partition starts right after the 10-byte uncompressed header (frame tag, start code, width and height).");
    let size_code = |name: &str, size: &str, scale: &str, word: &str, size_doc: &str, scale_doc: &str| {
        T::inline_structure(name, vec![(size, low_bits(14)), (scale, T::enumeration("VP8Scale", low_bits(2), VP8_SCALE))])
            .reads_as(&[(size, word, ""), (scale, "scale", "none")])
            .field_doc(size, size_doc)
            .field_doc(scale, scale_doc)
    };
    let horizontal = size_code(
        "HorizontalSizeCode",
        "width",
        "horizontal_scale",
        "{} pixels wide",
        "The frame width in pixels.",
        "How much the image is meant to be scaled up horizontally after decoding. The value does not change how the frame decodes.",
    );
    let vertical = size_code(
        "VerticalSizeCode",
        "height",
        "vertical_scale",
        "{} pixels high",
        "The frame height in pixels.",
        "How much the image is meant to be scaled up vertically after decoding. The value does not change how the frame decodes.",
    );
    T::structure(
        "VP8",
        vec![
            ("frame_tag", frame_tag),
            ("start_code", T::when(key(), T::magic(&[0x9d, 0x01, 0x2a]))),
            ("horizontal_size_code", T::when(key(), horizontal)),
            ("vertical_size_code", T::when(key(), vertical)),
            ("first_partition", T::bytes(tag("first_partition_size").at_most(E::Remaining))),
            ("token_partitions", T::bytes(E::Remaining)),
        ],
    )
    .doc("Lossy image data: one VP8 key frame.")
    .field_doc("frame_tag", "The frame tag: 3 bytes read as one 24-bit little-endian number, with frame_type in its lowest bit.")
    .field_doc("start_code", "The key frame start code, always 9d 01 2a.")
    .field_doc("horizontal_size_code", "A 16-bit little-endian number: width in its low 14 bits, horizontal_scale in its top 2 bits.")
    .field_doc("vertical_size_code", "A 16-bit little-endian number: height in its low 14 bits, vertical_scale in its top 2 bits.")
    .field_doc(
        "first_partition",
        "The compressed frame header (colour space, segmentation, loop filter settings, quantizer indices, the number of DCT token partitions and probability updates), then each macroblock's prediction modes. These bytes are boolean-entropy-coded, and the editor does not decode them.",
    )
    .field_doc(
        "token_partitions",
        "The quantized DCT and WHT coefficients, boolean-entropy-coded in 1, 2, 4 or 8 partitions. These bytes start with a 3-byte little-endian size for each partition except the last. The editor cannot show these sizes as separate fields, because the partition count is in first_partition.",
    )
}

/// `VP8L`: a lossless image (RFC 9649, section 3). After the signature byte
/// the whole chunk is one bitstream read low bit first, and its first 32 bits
/// are the size, the alpha hint and the version. The signature and those 32
/// bits are the `header`, drawn on one row.
fn vp8l() -> T {
    let header = T::inline_structure(
        "VP8LHeader",
        vec![
            ("signature", T::magic(&[0x2f])),
            ("width_minus_one", low_bits(14)),
            ("height_minus_one", low_bits(14)),
            ("alpha_is_used", low_bits(1)),
            ("version", low_bits(3)),
            ("width", plus_one("width_minus_one")),
            ("height", plus_one("height_minus_one")),
        ],
    )
    .field_valid("version", Valid::Eq(E::lit(0)))
    .reads_as(&[
        ("width", "{} pixels wide", ""),
        ("height", "{} pixels high", ""),
        ("alpha_is_used", "alpha_is_used", ""),
        ("version", "version", ""),
    ])
    .field_doc("signature", "The VP8L signature byte, always 0x2f.")
    .field_doc("width_minus_one", "The image width in pixels, minus 1.")
    .field_doc("height_minus_one", "The image height in pixels, minus 1.")
    .field_doc("alpha_is_used", "A hint that the image uses alpha. This field should be 0 when every alpha value in the image is 255.")
    .field_doc("version", "The format version, which must be 0. Decoders treat any other value as an error.")
    .field_doc("width", "The image width in pixels: width_minus_one + 1.")
    .field_doc("height", "The image height in pixels: height_minus_one + 1.");
    T::structure("VP8L", vec![("header", header), ("image_data", T::bytes(E::Remaining))])
        .doc("Lossless image data: a signature byte, 4 bytes with the image size, an alpha hint and a version number, then the compressed image as an LSB-first bitstream.")
        .field_doc("header", "The signature byte, then 32 bits read LSB-first: width_minus_one, height_minus_one, alpha_is_used and version.")
        .field_doc(
            "image_data",
            "The compressed image, as an LSB-first bitstream: the optional transforms (predictor, colour, subtract green, colour indexing), then the colour cache size, the prefix (Huffman) codes and the entropy-coded ARGB pixels. The editor does not decode this bitstream.",
        )
}

/// `ALPH`: the alpha plane of a lossy image, one header byte and the data.
fn alph() -> T {
    let header = T::inline_structure(
        "AlphaHeader",
        vec![
            ("reserved", high_bits(2)),
            ("preprocessing", T::enumeration("AlphaPreprocessing", high_bits(2), &[(0, "none"), (1, "level reduction")])),
            (
                "filtering_method",
                T::enumeration("AlphaFiltering", high_bits(2), &[(0, "none"), (1, "horizontal"), (2, "vertical"), (3, "gradient")]),
            ),
            ("compression_method", T::enumeration("AlphaCompression", high_bits(2), &[(0, "none"), (1, "lossless")])),
        ],
    )
    .field_valid("compression_method", Valid::InEnum)
    .reads_as(&[
        ("compression_method", "compression_method", ""),
        ("filtering_method", "filtering_method", ""),
        ("preprocessing", "preprocessing", ""),
    ])
    .field_doc("reserved", RESERVED_DOC)
    .field_doc(
        "preprocessing",
        "1 if the encoder reduced the number of distinct alpha values (level reduction). Decoding does not depend on this field, but decoders may dither the alpha values when it is 1.",
    )
    .field_doc(
        "filtering_method",
        "How each alpha value was predicted before storing: horizontal from the pixel to the left, vertical from the pixel above, gradient as left + above - above-left. Each stored value is the alpha value minus its prediction.",
    )
    .field_doc(
        "compression_method",
        "0 means alpha_bitstream is uncompressed: one byte per pixel of the image, each byte a filtered alpha value. 1 means alpha_bitstream is a WebP lossless (VP8L) image stream with no signature or size header, and the same width and height as the image. The alpha values are in the stream's green channel.",
    );
    T::structure("ALPH", vec![("header", header), ("alpha_bitstream", T::bytes(E::Remaining))])
        .doc("Alpha (transparency) values for the lossy image in the VP8 chunk, since the VP8 format has no alpha channel.")
        .field_doc("header", "The header byte, from the top bit down: 2 reserved bits, then preprocessing, filtering_method and compression_method, 2 bits each.")
        .field_doc("alpha_bitstream", "The image's alpha values, in the format set by compression_method.")
}

/// `ANIM`: the background colour and loop count of an animation. The colour
/// is stored blue first. `Colour` finds its channels by field name, so the
/// fields are laid out in the stored order and the swatch is still right.
fn anim() -> T {
    let colour = T::inline_structure(
        "BackgroundColor",
        vec![("blue", T::u8()), ("green", T::u8()), ("red", T::u8()), ("alpha", T::u8())],
    )
    .colour(Colour::rgb().alpha("alpha"));
    T::structure("ANIM", vec![("background_color", colour), ("loop_count", T::u16(Little))])
        .doc("Settings for the whole animation: the background colour and how many times the animation loops.")
        .field_doc(
            "background_color",
            "The colour a viewer may fill the canvas with, in blue, green, red, alpha byte order. When a frame's disposal_method is 1, the frame's rectangle is filled with this colour.",
        )
        .field_doc("loop_count", "How many times to loop the animation. 0 means it loops forever.")
}

/// `ANMF`: one frame of an animation, and then the frame's own chunks.
fn anmf() -> T {
    let flags = T::inline_structure(
        "AnimationFrameFlags",
        vec![
            ("reserved", high_bits(6)),
            ("blending_method", T::enumeration("BlendingMethod", high_bits(1), &[(0, "alpha-blend"), (1, "no blend")])),
            ("disposal_method", T::enumeration("DisposalMethod", high_bits(1), &[(0, "none"), (1, "background")])),
        ],
    )
    .reads_as(&[("blending_method", "blending", ""), ("disposal_method", "disposal", "")])
    .field_doc("reserved", RESERVED_DOC)
    .field_doc(
        "blending_method",
        "How the frame is drawn onto the canvas: 0 means the frame is alpha-blended onto the canvas, and 1 means the frame's pixels replace the canvas pixels in the frame's rectangle.",
    )
    .field_doc(
        "disposal_method",
        "What happens to the frame's rectangle before the next frame is drawn: 0 means the rectangle is left as it is, and 1 means the rectangle is filled with background_color from the ANIM chunk.",
    );
    T::structure(
        "ANMF",
        vec![
            ("frame_x_div_2", u24()),
            ("frame_y_div_2", u24()),
            ("frame_width_minus_one", u24()),
            ("frame_height_minus_one", u24()),
            ("duration_ms", u24()),
            ("flags", flags),
            ("frame_x", T::computed(E::field("frame_x_div_2").mul(E::lit(2)))),
            ("frame_y", T::computed(E::field("frame_y_div_2").mul(E::lit(2)))),
            ("frame_width", plus_one("frame_width_minus_one")),
            ("frame_height", plus_one("frame_height_minus_one")),
            ("frame_data", T::repeat(T::Named("Chunk".into()), Until::End)),
        ],
    )
    .doc("One frame of an animation: its position and size on the canvas, its duration, its blending and disposal methods, and the chunks that contain its image.")
    .field_doc("frame_x_div_2", "The distance from the left edge of the canvas to the left edge of the frame, in pixels, divided by 2. frame_x is the same distance in pixels.")
    .field_doc("frame_y_div_2", "The distance from the top edge of the canvas to the top edge of the frame, in pixels, divided by 2. frame_y is the same distance in pixels.")
    .field_doc("frame_width_minus_one", "The frame width in pixels, minus 1.")
    .field_doc("frame_height_minus_one", "The frame height in pixels, minus 1.")
    .field_doc("duration_ms", "How long to wait before showing the next frame, in milliseconds. What a viewer does with 0 and other very small durations is up to the viewer.")
    .field_doc("flags", "From the top bit down: 6 reserved bits, blending_method and disposal_method.")
    .field_doc("frame_x", "The distance from the left edge of the canvas to the left edge of the frame, in pixels: frame_x_div_2 × 2.")
    .field_doc("frame_y", "The distance from the top edge of the canvas to the top edge of the frame, in pixels: frame_y_div_2 × 2.")
    .field_doc("frame_width", "The frame width in pixels: frame_width_minus_one + 1.")
    .field_doc("frame_height", "The frame height in pixels: frame_height_minus_one + 1.")
    .field_doc("frame_data", "The frame's own chunks: an optional ALPH chunk, then one VP8 or VP8L chunk, then any unknown chunks.")
}

/// `ICCP`: the image's ICC profile, as its bytes. A shared ICC reader is to
/// take this one's place, which is why it has a function of its own.
fn icc_profile() -> T {
    T::structure_named("ICCP", "", "profile", vec![("profile", T::bytes(E::Remaining))])
        .doc("The ICC colour profile for the image.")
        .field_doc("profile", "The whole ICC colour profile. The ICCP chunk must come before the image data.")
}

/// `EXIF`: a TIFF file, with `Exif\0\0` in front of it when the writer copied
/// a JPEG's `APP1` segment whole. The TIFF part is sized to what is left of
/// the chunk, so its offsets count from its own first byte either way.
fn exif(tiff: &T) -> T {
    let prefixed = T::structure_named("Exif", "", "tiff", vec![("identifier", T::magic(b"Exif\0\0")), ("tiff", tiff.clone())])
        .doc("EXIF metadata as a TIFF file after a 6-byte Exif\\0\\0 identifier, with offsets counted from the start of the TIFF header.")
        .field_doc(
            "identifier",
            "The 6 bytes Exif\\0\\0, which some programs write before the TIFF header, as in a JPEG's APP1 segment. Offsets in the EXIF data count from the start of the TIFF header, after these 6 bytes.",
        );
    T::switch(E::peek(32, Big), vec![(cc(b"Exif"), prefixed)], tiff.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, NodeInfo, Value};
    use crate::source::MemSource;

    /// A RIFF chunk: id, little-endian size, payload, and a zero pad byte
    /// when the size is odd.
    fn chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = id.to_vec();
        v.extend_from_slice(&(data.len() as u32).to_le_bytes());
        v.extend_from_slice(data);
        if data.len() % 2 == 1 {
            v.push(0);
        }
        v
    }

    fn riff(chunks: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = chunks.concat();
        let mut b = b"RIFF".to_vec();
        b.extend_from_slice(&(body.len() as u32 + 4).to_le_bytes());
        b.extend_from_slice(b"WEBP");
        b.extend_from_slice(&body);
        b
    }

    fn file(chunks: &[Vec<u8>]) -> Document<MemSource> {
        Document::new(MemSource(riff(chunks)))
    }

    fn le24(n: u32) -> [u8; 3] {
        let b = n.to_le_bytes();
        [b[0], b[1], b[2]]
    }

    /// The node reached from the root by field names, with a number for an
    /// element of a list.
    fn node(ev: &mut Evaluator, d: &Document<MemSource>, steps: &[&str]) -> NodeInfo {
        let mut at = Vec::new();
        for step in steps {
            match step.parse::<usize>() {
                Ok(i) => at.push(i),
                Err(_) => at = ev.child_named(d, &at, step).unwrap().unwrap_or_else(|| panic!("no {step} under {at:?}")),
            }
        }
        ev.node(d, &at).unwrap()
    }

    fn int(ev: &mut Evaluator, d: &Document<MemSource>, steps: &[&str]) -> i128 {
        match node(ev, d, steps).value {
            Value::UInt(n) => n as i128,
            Value::Int(n) => n,
            Value::Enum { raw, .. } => raw,
            other => panic!("{steps:?} is {other:?}"),
        }
    }

    #[test]
    fn vp8x_flags_are_read_from_the_top_bit_down() {
        // ICC, EXIF and XMP set: 0b0010_1100. A 640 by 480 canvas.
        let mut data = vec![0x2c, 0, 0, 0];
        data.extend_from_slice(&le24(639));
        data.extend_from_slice(&le24(479));
        let d = file(&[chunk(b"VP8X", &data)]);
        let mut ev = Evaluator::new(webp());
        let flag = |ev: &mut Evaluator, name: &str| int(ev, &d, &["chunks", "0", "body", "flags", name]);
        let size = |ev: &mut Evaluator, name: &str| int(ev, &d, &["chunks", "0", "body", name]);
        assert_eq!(flag(&mut ev, "reserved_high"), 0);
        assert_eq!(flag(&mut ev, "icc_profile"), 1);
        assert_eq!(flag(&mut ev, "alpha"), 0);
        assert_eq!(flag(&mut ev, "exif"), 1);
        assert_eq!(flag(&mut ev, "xmp"), 1);
        assert_eq!(flag(&mut ev, "animation"), 0);
        assert_eq!(flag(&mut ev, "reserved_low"), 0);
        assert_eq!(size(&mut ev, "canvas_width_minus_one"), 639);
        assert_eq!(size(&mut ev, "canvas_width"), 640);
        assert_eq!(size(&mut ev, "canvas_height"), 480);
        // The chunk is named by its id in the listing.
        assert_eq!(node(&mut ev, &d, &["chunks", "0", "body"]).type_name, "VP8X");
        // Alpha alone is 0x10, and animation alone is 0x02.
        let d = file(&[chunk(b"VP8X", &[0x12, 0, 0, 0, 0, 0, 0, 0, 0, 0])]);
        let mut ev = Evaluator::new(webp());
        assert_eq!(int(&mut ev, &d, &["chunks", "0", "body", "flags", "alpha"]), 1);
        assert_eq!(int(&mut ev, &d, &["chunks", "0", "body", "flags", "animation"]), 1);
        assert_eq!(int(&mut ev, &d, &["chunks", "0", "body", "flags", "exif"]), 0);
    }

    #[test]
    fn vp8l_header_bits_are_read_from_the_bottom_bit_up() {
        // The header Pillow writes for a 4 by 4 image with alpha: width - 1
        // is 3, height - 1 is 3, alpha_is_used 1, version 0.
        let data = [0x2f, 0x03, 0xc0, 0x00, 0x10, 0xaa, 0xbb, 0xcc];
        let d = file(&[chunk(b"VP8L", &data)]);
        let mut ev = Evaluator::new(webp());
        let at = |name: &'static str| ["chunks", "0", "body", "header", name];
        assert_eq!(int(&mut ev, &d, &at("width_minus_one")), 3);
        assert_eq!(int(&mut ev, &d, &at("height_minus_one")), 3);
        assert_eq!(int(&mut ev, &d, &at("alpha_is_used")), 1);
        assert_eq!(int(&mut ev, &d, &at("version")), 0);
        assert_eq!(int(&mut ev, &d, &at("width")), 4);
        assert_eq!(int(&mut ev, &d, &at("height")), 4);
        let image = node(&mut ev, &d, &["chunks", "0", "body", "image_data"]);
        assert_eq!((image.offset_bits / 8, image.size_bits / 8), (20 + 5, 3));
        // Wide sizes reach across the byte boundaries: 16383 by 1, version 5.
        let bits: u32 = 0x3fff | (0 << 14) | (0 << 28) | (5 << 29);
        let mut data = vec![0x2f];
        data.extend_from_slice(&bits.to_le_bytes());
        let d = file(&[chunk(b"VP8L", &data)]);
        let mut ev = Evaluator::new(webp());
        assert_eq!(int(&mut ev, &d, &at("width")), 16384);
        assert_eq!(int(&mut ev, &d, &at("height")), 1);
        assert_eq!(int(&mut ev, &d, &at("version")), 5);
    }

    #[test]
    fn a_vp8_frame_tag_gives_the_first_partition_its_size() {
        // The frame tag from RFC 6386: key frame 0, version 2, show_frame 1,
        // first partition 5 bytes, as tmp = c0 | c1 << 8 | c2 << 16.
        let tmp: u32 = 0 | (2 << 1) | (1 << 4) | (5 << 5);
        let mut data = tmp.to_le_bytes()[..3].to_vec();
        data.extend_from_slice(&[0x9d, 0x01, 0x2a]);
        // 400 wide, scaled by 5/4; 300 high, scaled by 2.
        data.extend_from_slice(&(400u16 | 1 << 14).to_le_bytes());
        data.extend_from_slice(&(300u16 | 3 << 14).to_le_bytes());
        data.extend_from_slice(&[1, 2, 3, 4, 5]);
        data.extend_from_slice(&[6, 7, 8, 9]);
        let d = file(&[chunk(b"VP8 ", &data)]);
        let mut ev = Evaluator::new(webp());
        let tag = |name: &'static str| ["chunks", "0", "body", "frame_tag", name];
        let frame_type = node(&mut ev, &d, &tag("frame_type"));
        assert_eq!(frame_type.value, Value::Enum { raw: 0, name: Some("key frame".into()), hex: false });
        assert_eq!(int(&mut ev, &d, &tag("version")), 2);
        assert_eq!(int(&mut ev, &d, &tag("show_frame")), 1);
        assert_eq!(int(&mut ev, &d, &tag("first_partition_size")), 5);
        assert_eq!(int(&mut ev, &d, &["chunks", "0", "body", "horizontal_size_code", "width"]), 400);
        assert_eq!(int(&mut ev, &d, &["chunks", "0", "body", "horizontal_size_code", "horizontal_scale"]), 1);
        assert_eq!(int(&mut ev, &d, &["chunks", "0", "body", "vertical_size_code", "height"]), 300);
        assert_eq!(int(&mut ev, &d, &["chunks", "0", "body", "vertical_size_code", "vertical_scale"]), 3);
        let at = |name: &'static str| ["chunks", "0", "body", name];
        let first = node(&mut ev, &d, &at("first_partition"));
        assert_eq!((first.offset_bits / 8, first.size_bits / 8), (20 + 10, 5));
        let tokens = node(&mut ev, &d, &at("token_partitions"));
        assert_eq!((tokens.offset_bits / 8, tokens.size_bits / 8), (20 + 15, 4));
    }

    #[test]
    fn an_animation_frame_holds_chunks_of_its_own() {
        // A background of rgba(40, 80, 160, 255), written blue first, played
        // three times.
        let anim = chunk(b"ANIM", &[160, 80, 40, 255, 3, 0]);
        // A frame 10 by 6 at x 12, y 8, for 250 ms, not blended and
        // disposed to the background, holding ALPH and VP8L.
        let mut frame = Vec::new();
        for n in [6, 4, 9, 5, 250] {
            frame.extend_from_slice(&le24(n));
        }
        frame.push(0b0000_0011);
        frame.extend_from_slice(&chunk(b"ALPH", &[0b0000_1001, 0xee, 0xef]));
        frame.extend_from_slice(&chunk(b"VP8L", &[0x2f, 0x09, 0x40, 0x01, 0x00]));
        let d = file(&[chunk(b"VP8X", &[0x02, 0, 0, 0, 23, 0, 0, 15, 0, 0]), anim, chunk(b"ANMF", &frame)]);
        let mut ev = Evaluator::new(webp());
        let bg = node(&mut ev, &d, &["chunks", "1", "body", "background_color"]);
        assert_eq!(bg.colour.as_deref(), Some("#2850a0ff"));
        assert_eq!(int(&mut ev, &d, &["chunks", "1", "body", "loop_count"]), 3);
        let at = |name: &'static str| ["chunks", "2", "body", name];
        assert_eq!(int(&mut ev, &d, &at("frame_x")), 12);
        assert_eq!(int(&mut ev, &d, &at("frame_y")), 8);
        assert_eq!(int(&mut ev, &d, &at("frame_width")), 10);
        assert_eq!(int(&mut ev, &d, &at("frame_height")), 6);
        assert_eq!(int(&mut ev, &d, &at("duration_ms")), 250);
        assert_eq!(int(&mut ev, &d, &["chunks", "2", "body", "flags", "blending_method"]), 1);
        assert_eq!(int(&mut ev, &d, &["chunks", "2", "body", "flags", "disposal_method"]), 1);
        assert_eq!(node(&mut ev, &d, &at("frame_data")).child_count, 2);
        let alph = |name: &'static str| ["chunks", "2", "body", "frame_data", "0", "body", "header", name];
        let filtering = node(&mut ev, &d, &alph("filtering_method"));
        assert_eq!(filtering.value, Value::Enum { raw: 2, name: Some("vertical".into()), hex: false });
        assert_eq!(int(&mut ev, &d, &alph("compression_method")), 1);
        // The ALPH chunk is 3 bytes, so a pad byte comes before the VP8L.
        let pad = node(&mut ev, &d, &["chunks", "2", "body", "frame_data", "0", "pad"]);
        assert_eq!(pad.size_bits, 8);
        assert_eq!(node(&mut ev, &d, &["chunks", "2", "body", "frame_data", "1", "id"]).value, Value::Str("VP8L".into()));
        assert_eq!(int(&mut ev, &d, &["chunks", "2", "body", "frame_data", "1", "body", "header", "height"]), 6);
    }

    #[test]
    fn an_odd_sized_chunk_is_followed_by_its_pad_byte() {
        let d = file(&[chunk(b"VP8L", &[0x2f, 0, 0, 0, 0, 0xab, 0xcd]), chunk(b"XMP ", b"<x/>")]);
        let mut ev = Evaluator::new(webp());
        let pad = node(&mut ev, &d, &["chunks", "0", "pad"]);
        assert_eq!((pad.offset_bits / 8, pad.size_bits / 8), (12 + 8 + 7, 1));
        assert_eq!(node(&mut ev, &d, &["chunks", "1", "id"]).value, Value::Str("XMP ".into()));
        assert_eq!(node(&mut ev, &d, &["chunks", "1", "body"]).value, Value::Str("<x/>".into()));
        // An even chunk has no pad byte.
        let pad = node(&mut ev, &d, &["chunks", "1", "pad"]);
        assert_eq!(pad.size_bits, 0);
    }

    #[test]
    fn exif_reads_as_a_tiff_with_or_without_the_jpeg_prefix() {
        // A big-endian TIFF with one directory of one ASCII tag, Make, whose
        // eight bytes are behind an offset counted from the TIFF's start.
        let mut tiff = b"MM\0\x2a\0\0\0\x08".to_vec();
        tiff.extend_from_slice(&1u16.to_be_bytes());
        tiff.extend_from_slice(&0x010fu16.to_be_bytes());
        tiff.extend_from_slice(&2u16.to_be_bytes());
        tiff.extend_from_slice(&8u32.to_be_bytes());
        tiff.extend_from_slice(&26u32.to_be_bytes());
        tiff.extend_from_slice(&0u32.to_be_bytes());
        tiff.extend_from_slice(b"Qubero\0\0");
        let mut prefixed = b"Exif\0\0".to_vec();
        prefixed.extend_from_slice(&tiff);
        for (data, tiff_at) in [(tiff.clone(), vec!["chunks", "0", "body"]), (prefixed, vec!["chunks", "0", "body", "tiff"])] {
            let d = file(&[chunk(b"EXIF", &data)]);
            let mut ev = Evaluator::new(webp());
            assert_eq!(node(&mut ev, &d, &tiff_at).type_name, "TIFF");
            let order = [tiff_at.as_slice(), &["byte_order"]].concat();
            assert_eq!(int(&mut ev, &d, &order), 0x4d4d);
        }
    }

    /// The fields read low bit first start partway through a byte, and the
    /// annotation column steps through the file from the top bit of each
    /// byte. Where it lost its place it showed the whole file as one gap.
    #[test]
    fn the_annotation_column_has_a_field_for_every_byte() {
        let mut vp8 = (0u32 | (1 << 4) | (2 << 5)).to_le_bytes()[..3].to_vec();
        vp8.extend_from_slice(&[0x9d, 0x01, 0x2a, 4, 0, 4, 0, 0xaa, 0xbb, 0xcc]);
        let mut frame = Vec::new();
        for n in [0, 0, 3, 3, 100] {
            frame.extend_from_slice(&le24(n));
        }
        frame.push(0b0000_0010);
        frame.extend_from_slice(&chunk(b"VP8L", &[0x2f, 0x03, 0xc0, 0x00, 0x00, 0x11]));
        let b = riff(&[
            chunk(b"VP8X", &[0x12, 0, 0, 0, 3, 0, 0, 3, 0, 0]),
            chunk(b"ANIM", &[0, 0, 0, 0, 0, 0]),
            chunk(b"ALPH", &[0b0000_0101, 1, 2]),
            chunk(b"VP8 ", &vp8),
            chunk(b"ANMF", &frame),
        ]);
        let len = b.len() as u64;
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(webp());
        let spans = ev.spans(&d, 0, len * 8, 10_000).unwrap();
        let gaps: Vec<_> = spans.iter().filter(|s| s.gap).map(|s| (s.offset_bits / 8, s.size_bits / 8, s.name.clone())).collect();
        assert!(gaps.is_empty(), "gaps: {gaps:?}");
        assert_eq!(spans.iter().map(|s| s.size_bits).sum::<u64>(), len * 8);
        // Each byte of bit fields is one row, which says what is in it.
        let line = |name: &str| spans.iter().find(|s| s.name == name).and_then(|s| s.line.clone()).unwrap_or_else(|| panic!("no row for {name}"));
        assert_eq!(line("frame_tag"), "key frame \u{b7} version 0 \u{b7} show_frame 1 \u{b7} first partition 2 bytes");
        assert_eq!(line("horizontal_size_code"), "4 pixels wide");
        assert_eq!(line("header"), "compression_method lossless \u{b7} filtering_method horizontal \u{b7} preprocessing none");
    }

    #[test]
    fn an_unknown_chunk_is_its_bytes() {
        let d = file(&[chunk(b"ABCD", &[1, 2, 3])]);
        let mut ev = Evaluator::new(webp());
        let body = node(&mut ev, &d, &["chunks", "0", "body"]);
        assert_eq!(body.size_bits, 24);
        assert!(matches!(body.value, Value::Bytes { len: 3, .. }));
    }
}
