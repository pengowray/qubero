//! PNG: signature plus a chunk stream that ends at IEND.

use crate::template::{
    Check, Checksum, Colour, Covers, Encoding, Endian::*, Expr as E, Packing, PngHeader, RasterOrder, Step, StrLen, Template, Ty as T, Until,
    Valid,
};

/// PNG colour types. 1, 5 and 7 are not defined by the spec, so a file holding
/// one shows the number with no name.
const COLOR_TYPE: &[(i128, &str)] = &[
    (0, "greyscale"),
    (2, "rgb"),
    (3, "indexed"),
    (4, "greyscale alpha"),
    (6, "rgba"),
];

/// What a chunk's CRC-32 covers: the type and the data, and neither the length
/// in front of them nor the sum itself. Written once and used by both chunk
/// definitions below, which differ only in what they read a chunk's payload as.
fn chunk_crc() -> Check {
    // From the end of the length to the start of the CRC. The declared length
    // is used rather than the measured size of `data`, because measuring `data`
    // in a cartridge would unpack the image to answer a question that is
    // written down four bytes earlier.
    Check::of(
        Checksum::Crc32,
        Covers::Run { at: E::size_of("length"), len: E::size_of("type").add(E::field("length")) },
    )
}

/// The header chunk, which every PNG opens with and which says what shape the
/// image is. Shared with the cartridge templates, PICO-8 and Picotron, which
/// read the same chunk and then go looking for what the picture is carrying.
pub(crate) fn ihdr() -> T {
    T::structure(
        "IHDR",
        vec![
            ("width", T::u32(Big)),
            ("height", T::u32(Big)),
            ("bit_depth", T::u8()),
            ("color_type", T::enumeration("ColorType", T::u8(), COLOR_TYPE)),
            ("compression", T::enumeration("Compression", T::u8(), &[(0, "deflate")])),
            ("filter", T::enumeration("FilterMethod", T::u8(), &[(0, "adaptive")])),
            ("interlace", T::enumeration("Interlace", T::u8(), &[(0, "none"), (1, "adam7")])),
        ],
    )
    // The two the specification writes out as lists rather than as ranges. A
    // PNG with a seventh colour type or a bit depth of three is not a PNG with
    // something Qubero has no name for; it is one no decoder will open, which
    // is what the reader came to find out.
    .field_valid("bit_depth", Valid::AnyOf(vec![E::lit(1), E::lit(2), E::lit(4), E::lit(8), E::lit(16)]))
    .field_valid("color_type", Valid::AnyOf(vec![E::lit(0), E::lit(2), E::lit(3), E::lit(4), E::lit(6)]))
}

/// tEXt: a NUL-terminated keyword, then the text filling the rest. Both are
/// Latin-1 by the spec, not UTF-8; iTXt is the chunk that carries UTF-8.
pub(crate) fn text() -> T {
    T::structure(
        "tEXt",
        vec![
            ("keyword", T::text(StrLen::Terminated { end: 0, or_end: false }, Encoding::Latin1)),
            (
                "text",
                T::text(StrLen::Fixed(E::field("length").sub(E::size_of("keyword"))), Encoding::Latin1),
            ),
        ],
    )
}

/// A PNG that carries a cartridge, as PICO-8 and Picotron both write one.
///
/// The same chunk stream an ordinary PNG is, with `idat` in place of the bytes
/// an IDAT chunk would otherwise be: whatever the cartridge template wants to
/// open over what the picture is carrying. `name` is what the file is called
/// in the listing.
pub(crate) fn cart_png(name: &'static str, idat: T) -> T {
    let chunk = T::structure_named(
        "Chunk",
        "type",
        "data",
        vec![
            ("length", T::u32(Big)),
            ("type", T::utf8(E::lit(4))),
            (
                "data",
                T::sized(
                    E::field("length"),
                    T::switch(
                        E::field("type"),
                        vec![(0x4948_4452, ihdr()), (0x7445_5874, text()), (0x4944_4154, idat)],
                        T::bytes(E::field("length")),
                    ),
                ),
            ),
            ("crc", T::u32(Big)),
        ],
    )
    .field_check("crc", chunk_crc());
    T::structure(
        name,
        vec![
            ("signature", T::magic(b"\x89PNG\r\n\x1a\n")),
            ("chunks", T::repeat(chunk, Until::FieldBytes { field: "type".into(), bytes: b"IEND".to_vec() })),
        ],
    )
}

/// Whether a PNG's header says the image is `width` by `height`, eight bits a
/// channel, colour type 6, which is RGBA, and not interlaced. The shape both
/// cartridge formats are written in, and the first of the two questions asked
/// of a picture that might be one.
///
/// Interlacing is in here because Adam7 lays an image out in seven passes, so a
/// row of the stream is not a row of the image and none of the arithmetic in
/// [`cart_pixels`] holds. No cartridge is interlaced.
pub(crate) fn is_size(head: &[u8], width: u32, height: u32) -> bool {
    if !head.starts_with(b"\x89PNG\r\n\x1a\n") || head.get(12..16) != Some(b"IHDR") {
        return false;
    }
    let (Some(w), Some(h)) = (dword(head, 16), dword(head, 20)) else { return false };
    w == width
        && h == height
        && head.get(24) == Some(&8)
        && head.get(25) == Some(&6)
        && head.get(28) == Some(&0)
}

/// The pixel bytes a cartridge image hides its payload in, as far as the bytes
/// of the file a sniff has been given reach.
///
/// A cartridge announces nothing about itself in the file: the only way to tell
/// one from a holiday snap of the same size is to go and read what is hidden in
/// the pixels, and that means running the front of the image through the same
/// three steps the template does. The chunks inside `head` are walked, the IDAT
/// data in them gathered, that stream partly inflated, and the whole rows which
/// came out unfiltered. A row cut off in the middle is dropped, since a filter
/// needs a whole row above it.
///
/// The result is the unfiltered pixels: `width * 4` bytes a row, and however
/// many rows the head reached. Empty if the header is not the right size and
/// shape, or if what came out is not scanlines. A caller reads its own payload
/// out of the rows it was given and decides on those; the rows past the end of
/// the head are the caller's problem, not this function's.
pub(crate) fn cart_pixels(head: &[u8], width: u32, height: u32) -> Vec<u8> {
    if !is_size(head, width, height) {
        return Vec::new();
    }
    let stride = width as usize * 4;
    // What a complete image comes to, filter bytes and all, which is the most
    // output worth keeping and far less than a hostile stream would produce.
    let cap = height as usize * (stride + 1);
    // The chunk stream starts after the eight-byte signature. IHDR is the first
    // chunk in it and is walked past like any other chunk which is not an IDAT.
    let mut at = 8;
    let mut idat: Vec<u8> = Vec::new();
    while at + 8 <= head.len() {
        let Some(len) = dword(head, at) else { break };
        let kind = &head[at + 4..at + 8];
        if kind == b"IEND" {
            break;
        }
        let start = at + 8;
        // A length is four bytes wide and says whatever it likes, so the end of
        // the chunk is worked out in a width that cannot wrap.
        let Some(end) = start.checked_add(len as usize) else { break };
        if kind == b"IDAT" {
            // The last IDAT in the head is the one running past its end, and
            // the bytes of it which are here are the point of the exercise.
            idat.extend_from_slice(&head[start..end.min(head.len())]);
        }
        let Some(next) = end.checked_add(4) else { break };
        if next > head.len() {
            break;
        }
        at = next;
    }
    let raw = crate::codec::inflate::inflate_prefix(&idat, cap);
    let rows = raw.len() / (stride + 1);
    if rows == 0 {
        return Vec::new();
    }
    match crate::codec::pixels::unfilter(&raw[..rows * (stride + 1)], width * 4, 4) {
        Ok((pixels, _)) => pixels,
        Err(_) => Vec::new(),
    }
}

fn dword(head: &[u8], at: usize) -> Option<u32> {
    let bytes: [u8; 4] = head.get(at..at + 4)?.try_into().ok()?;
    Some(u32::from_be_bytes(bytes))
}

pub fn png() -> Template {
    Template::new("png", image())
}

/// The image on its own, so a format that carries a whole PNG can place it. A
/// PNG refers to nothing by name, so the type is the whole of it and there is
/// no vocabulary to hand over with it.
///
/// Every step from the bytes to the pixels is a field. The chunks are read as
/// the file keeps them; then `image` joins every IDAT chunk's data, in order,
/// into the one zlib stream they were cut from, inflates it, unfilters the
/// scanlines that come out with the header's own width, depth, colour type
/// and interlacing, and places every pixel back where it goes in the picture.
/// So the pixel at column 13 of row 9 has a path, its samples are fields, and
/// each step down to it has a trace saying which bits of the step above made
/// it.
pub(crate) fn image() -> T {
    let chunk = T::structure_named(
        "Chunk",
        "type",
        "data",
        vec![
            ("length", T::u32(Big)),
            ("type", T::utf8(E::lit(4))),
            (
                "data",
                T::sized(
                    E::field("length"),
                    // A text field in an expression is its bytes as a big-endian number.
                    T::switch(E::field("type"), chunk_kinds(), T::bytes(E::field("length"))),
                ),
            ),
            ("crc", T::u32(Big)),
        ],
    )
    .field_check("crc", chunk_crc());
    T::structure(
        "PNG",
        vec![
            ("signature", T::magic(b"\x89PNG\r\n\x1a\n")),
            ("chunks", T::repeat(chunk, Until::FieldBytes { field: "type".into(), bytes: b"IEND".to_vec() })),
            // The IDAT chunks' data joined in the order the chunks are in,
            // which is one zlib stream however many chunks an encoder cut it
            // into. The walk lands on `image_data`, which only an IDAT chunk
            // has, so every other chunk is passed over.
            (
                "image",
                T::stitched(
                    vec![Step::field("chunks"), Step::each(), Step::field("data"), Step::field("image_data")],
                    None,
                    None,
                    super::zlib::part(scanlines()).root,
                ),
            ),
        ],
    )
}

/// A chunk type as the switch sees it: its four letters read as a big-endian
/// number.
const fn kind(name: &[u8; 4]) -> i128 {
    u32::from_be_bytes(*name) as i128
}

/// A field of the image header, from wherever in the file it is asked. IHDR is
/// the first chunk in every PNG, but a file that broke that rule is found by
/// its type rather than by its place, and one with no IHDR at all reads the
/// number as nought, which no image has.
fn header(field: &str) -> E {
    E::tagged_bytes("chunks", &["type"], b"IHDR", &["data", field])
}

/// The same field, asked from inside a chunk: the chunks before it are
/// searched for the header, which is the one list a chunk's own fields can
/// reach. A bKGD or a tRNS is laid out by the colour type.
fn header_before(field: &str) -> E {
    E::sibling_tagged(&["type"], E::lit(kind(b"IHDR")), &["data", field])
}

/// How many bits one pixel takes: the depth times the samples its colour type
/// has, three for RGB and one for a palette index.
fn bits_per_pixel() -> E {
    let ct = || header("color_type");
    let samples = E::cond(
        ct().equal_to(E::lit(2)),
        E::lit(3),
        E::cond(ct().equal_to(E::lit(4)), E::lit(2), E::cond(ct().equal_to(E::lit(6)), E::lit(4), E::lit(1))),
    );
    header("bit_depth").mul(samples)
}

/// What the image's zlib stream holds: scanlines, each a filter byte and a row
/// predicted from its neighbours, which unfilter into the rows of pixels, pass
/// by pass for an interlaced image.
fn scanlines() -> T {
    let packing = Packing::PngScanlines(std::sync::Arc::new(PngHeader {
        width: header("width"),
        height: header("height"),
        bit_depth: header("bit_depth"),
        color_type: header("color_type"),
        interlace: header("interlace"),
    }));
    let raster = |order| T::raster(header("width"), header("height"), bits_per_pixel(), order, pixel());
    let pixels = T::switch(header("interlace"), vec![(1, raster(RasterOrder::Adam7))], raster(RasterOrder::Rows));
    let unfiltered = T::structure_named("Unfiltered", "", "pixels", vec![("pixels", pixels)]);
    T::structure_named("Inflated", "", "scanlines", vec![("scanlines", T::decoded_as(E::Remaining, packing, unfiltered))])
}

/// One pixel: its samples, each as wide as the bit depth, named for what the
/// colour type says they are. A palette image's pixel is an index into PLTE.
fn pixel() -> T {
    let sample = || {
        T::switch(
            header("bit_depth"),
            vec![
                (1, T::UInt { bits: 1, endian: Big }),
                (2, T::UInt { bits: 2, endian: Big }),
                (4, T::UInt { bits: 4, endian: Big }),
                (8, T::u8()),
                (16, T::u16(Big)),
            ],
            T::bytes(E::lit(0)),
        )
    };
    let px = |name: &str, samples: &[&str]| {
        T::structure(name, samples.iter().map(|s| (*s, sample())).collect()).counted_as("pixel")
    };
    // Each sample is as wide as the depth, so its own width is its range. A
    // palette index is a colour only by way of PLTE, and is left a number.
    T::switch(
        header("color_type"),
        vec![
            (0, px("GreyPixel", &["grey"]).colour(Colour::grey())),
            (2, px("RgbPixel", &["red", "green", "blue"]).colour(Colour::rgb())),
            (3, px("IndexedPixel", &["index"])),
            (4, px("GreyAlphaPixel", &["grey", "alpha"]).colour(Colour::grey().alpha("alpha"))),
            (6, px("RgbaPixel", &["red", "green", "blue", "alpha"]).colour(Colour::rgb().alpha("alpha"))),
        ],
        // A colour type the specification does not define. The scanlines
        // refuse to unpack for it, so no pixel is ever asked to read as this.
        T::bytes(E::lit(0)),
    )
}

/// What each chunk's data reads as, by its type. A chunk not named here is its
/// bytes.
fn chunk_kinds() -> Vec<(i128, T)> {
    vec![
        (kind(b"IHDR"), ihdr()),
        (kind(b"tEXt"), text()),
        (kind(b"IDAT"), idat()),
        (kind(b"PLTE"), plte()),
        (kind(b"tRNS"), trns()),
        (kind(b"gAMA"), gama()),
        (kind(b"cHRM"), chrm()),
        (kind(b"sRGB"), srgb()),
        (kind(b"iCCP"), iccp()),
        (kind(b"sBIT"), sbit()),
        (kind(b"bKGD"), bkgd()),
        (kind(b"pHYs"), phys()),
        (kind(b"tIME"), time()),
        (kind(b"hIST"), hist()),
        (kind(b"sPLT"), splt()),
        (kind(b"zTXt"), ztxt()),
        (kind(b"iTXt"), itxt()),
        (kind(b"acTL"), actl()),
        (kind(b"fcTL"), fctl()),
        (kind(b"fdAT"), fdat()),
    ]
}

/// IDAT: a piece of the image's zlib stream. The pieces are read joined, as
/// the PNG's `image`, and a piece on its own is only bytes: an encoder may cut
/// the stream anywhere, even inside a deflate code.
fn idat() -> T {
    T::structure_named("IDAT", "", "image_data", vec![("image_data", T::bytes(E::Remaining))])
}

/// PLTE: the palette, three bytes an entry, as many entries as fit.
fn plte() -> T {
    let entry = T::inline_structure("PaletteEntry", vec![("red", T::u8()), ("green", T::u8()), ("blue", T::u8())])
        .counted_as("entry")
        .colour(Colour::rgb());
    T::structure_named("PLTE", "", "entries", vec![("entries", T::array(entry, E::Remaining.div(E::lit(3))))])
}

/// One colour written as the image's samples are, whatever the width of the
/// fields holding it: a grey level, or red, green and blue, each ranging over
/// the image's bit depth. bKGD and tRNS write sixteen bits a sample in an
/// image of any depth, so a field's own width is not its range.
fn sample_colour(name: &str, wide: T, rgb: bool) -> T {
    let depth = || header_before("bit_depth");
    if rgb {
        T::structure(name, vec![("red", wide.clone()), ("green", wide.clone()), ("blue", wide)])
            .colour(Colour::rgb().depth(depth()))
    } else {
        T::structure(name, vec![("grey", wide)]).colour(Colour::grey().depth(depth()))
    }
}

/// Three fields that are laid out by the colour type: one grey value, one of
/// each of red, green and blue, or, for a palette image, something of its own.
/// `wide` is the type of each value in the first two cases.
fn by_colour(name: &str, wide: T, palette: T) -> T {
    let grey = sample_colour(&format!("{name}Grey"), wide.clone(), false);
    let rgb = sample_colour(&format!("{name}Rgb"), wide, true);
    T::switch(header_before("color_type"), vec![(0, grey.clone()), (2, rgb.clone()), (3, palette), (4, grey), (6, rgb)], T::bytes(E::Remaining))
}

/// tRNS: which colour is transparent, or for a palette image the alpha of each
/// palette entry in turn, as many as the chunk holds. The colour types with an
/// alpha channel of their own have no tRNS.
fn trns() -> T {
    let alphas = T::structure_named("tRNSPalette", "", "alpha", vec![("alpha", T::array(T::u8(), E::Remaining))]);
    T::switch(
        header_before("color_type"),
        vec![
            (0, sample_colour("tRNSGrey", T::u16(Big), false)),
            (2, sample_colour("tRNSRgb", T::u16(Big), true)),
            (3, alphas),
        ],
        T::bytes(E::Remaining),
    )
}

/// A number the file stores as a whole number times 100,000, beside what it
/// is worth, the way GRIB's and NIfTI's scaled values read. gAMA and cHRM
/// write every value this way.
fn hundred_thousandths() -> T {
    T::inline_structure(
        "Scaled",
        vec![("stored", T::u32(Big)), ("worth", T::computed_real(E::field("stored").div(E::real(100_000.0))))],
    )
    .payload(&["stored"])
}

/// gAMA: the image's gamma, stored times 100,000. 45455 is 1/2.2, the gamma
/// most images are encoded with.
fn gama() -> T {
    T::structure("gAMA", vec![("gamma", hundred_thousandths())])
}

/// cHRM: the chromaticities of the white point and the three primaries, each
/// an x and a y stored times 100,000.
fn chrm() -> T {
    let names = ["white_x", "white_y", "red_x", "red_y", "green_x", "green_y", "blue_x", "blue_y"];
    T::structure("cHRM", names.iter().map(|n| (*n, hundred_thousandths())).collect())
}

/// sRGB: the image is in the sRGB colour space, and which rendering intent to
/// use when it has to be mapped into another.
fn srgb() -> T {
    let intent = T::enumeration(
        "RenderingIntent",
        T::u8(),
        &[(0, "perceptual"), (1, "relative colorimetric"), (2, "saturation"), (3, "absolute colorimetric")],
    );
    T::structure("sRGB", vec![("rendering_intent", intent)])
}

/// The one compression method PNG defines for its compressed text and
/// profiles.
fn compression_method() -> T {
    T::enumeration("CompressionMethod", T::u8(), &[(0, "deflate")])
}

/// A NUL-terminated Latin-1 keyword or name, 1 to 79 characters by the spec.
fn keyword() -> T {
    T::text(StrLen::Terminated { end: 0, or_end: false }, Encoding::Latin1)
}

/// iCCP: an embedded ICC profile, named, and compressed as a zlib stream. The
/// profile is read as its bytes; opened on its own it is recognised as the
/// ICC profile it is.
fn iccp() -> T {
    T::structure(
        "iCCP",
        vec![
            ("profile_name", keyword()),
            ("compression_method", compression_method()),
            ("profile", super::zlib::part(super::decoded_object()).root),
        ],
    )
}

/// sBIT: how many bits of each sample were significant in the original, before
/// the samples were scaled up to the image's bit depth.
fn sbit() -> T {
    let s = |name: &str, fields: &[&str]| T::structure(name, fields.iter().map(|f| (*f, T::u8())).collect());
    T::switch(
        header_before("color_type"),
        vec![
            (0, s("sBITGrey", &["grey"])),
            (2, s("sBITRgb", &["red", "green", "blue"])),
            (3, s("sBITPalette", &["red", "green", "blue"])),
            (4, s("sBITGreyAlpha", &["grey", "alpha"])),
            (6, s("sBITRgba", &["red", "green", "blue", "alpha"])),
        ],
        T::bytes(E::Remaining),
    )
}

/// bKGD: the colour to show the image against, as a grey level, an RGB
/// colour, or a palette index.
fn bkgd() -> T {
    by_colour("bKGD", T::u16(Big), T::structure("bKGDPalette", vec![("palette_index", T::u8())]))
}

/// pHYs: how many pixels fit in a unit each way, and whether the unit is the
/// metre or says only the aspect ratio. Pixels to the metre is what the file
/// says and dots to the inch is what everyone asks, so where the unit is the
/// metre the two are side by side: 11811 a metre is 300 dpi.
fn phys() -> T {
    let metre = || E::field("unit").equal_to(E::lit(1));
    let dpi = |field: &str| T::when(metre(), T::computed_real(E::field(field).mul(E::real(0.0254))));
    T::structure(
        "pHYs",
        vec![
            ("pixels_per_unit_x", T::u32(Big)),
            ("pixels_per_unit_y", T::u32(Big)),
            ("unit", T::enumeration("PhysUnit", T::u8(), &[(0, "unknown"), (1, "metre")])),
            ("dpi_x", dpi("pixels_per_unit_x")),
            ("dpi_y", dpi("pixels_per_unit_y")),
        ],
    )
    .field_doc("dpi_x", "Dots per inch across, worked out from pixels_per_unit_x. Only where the unit is the metre.")
    .field_doc("dpi_y", "Dots per inch down, worked out from pixels_per_unit_y. Only where the unit is the metre.")
}

/// tIME: when the image was last changed, in UTC, as the year and then five
/// single bytes.
fn time() -> T {
    T::structure(
        "tIME",
        vec![
            ("year", T::u16(Big)),
            ("month", T::u8()),
            ("day", T::u8()),
            ("hour", T::u8()),
            ("minute", T::u8()),
            ("second", T::u8()),
        ],
    )
}

/// hIST: how often each palette entry is used, one number an entry.
fn hist() -> T {
    T::structure_named("hIST", "", "frequencies", vec![("frequencies", T::array(T::u16(Big), E::Remaining.div(E::lit(2))))])
}

/// sPLT: a suggested palette, named, with its entries' samples one or two
/// bytes wide as `sample_depth` says, and a frequency for each.
fn splt() -> T {
    let sample = || T::switch(E::field("sample_depth"), vec![(16, T::u16(Big))], T::u8());
    let entry = T::inline_structure(
        "SuggestedEntry",
        vec![("red", sample()), ("green", sample()), ("blue", sample()), ("alpha", sample()), ("frequency", T::u16(Big))],
    )
    .counted_as("entry")
    .colour(Colour::rgb().alpha("alpha"));
    let size = E::cond(E::field("sample_depth").equal_to(E::lit(16)), E::lit(10), E::lit(6));
    T::structure(
        "sPLT",
        vec![("palette_name", keyword()), ("sample_depth", T::u8()), ("entries", T::array(entry, E::Remaining.div(size)))],
    )
}

/// zTXt: a keyword and its text, the text compressed as a zlib stream. Latin-1,
/// as tEXt is.
fn ztxt() -> T {
    let text = T::structure_named("Text", "", "text", vec![("text", T::text(StrLen::Fixed(E::Remaining), Encoding::Latin1))]);
    T::structure(
        "zTXt",
        vec![("keyword", keyword()), ("compression_method", compression_method()), ("text", super::zlib::part(text).root)],
    )
}

/// iTXt: a keyword, a language, the keyword translated, and the text, in UTF-8
/// and compressed as a zlib stream when the flag says so.
fn itxt() -> T {
    let utf8 = || T::text(StrLen::Fixed(E::Remaining), Encoding::Utf8);
    let packed = T::structure_named("Text", "", "text", vec![("text", utf8())]);
    T::structure(
        "iTXt",
        vec![
            ("keyword", keyword()),
            ("compression_flag", T::enumeration("CompressionFlag", T::u8(), &[(0, "uncompressed"), (1, "compressed")])),
            ("compression_method", compression_method()),
            ("language_tag", T::text(StrLen::Terminated { end: 0, or_end: false }, Encoding::Ascii)),
            ("translated_keyword", T::text(StrLen::Terminated { end: 0, or_end: false }, Encoding::Utf8)),
            ("text", T::switch(E::field("compression_flag"), vec![(1, super::zlib::part(packed).root)], utf8())),
        ],
    )
}

/// acTL: an animated PNG's frame count, and how many times to play it, where
/// nought means for ever.
fn actl() -> T {
    T::structure("acTL", vec![("num_frames", T::u32(Big)), ("num_plays", T::u32(Big))])
}

/// fcTL: one frame of an animation: where it sits on the canvas, how long it
/// shows for, and what happens to the canvas before the next.
fn fctl() -> T {
    T::structure(
        "fcTL",
        vec![
            ("sequence_number", T::u32(Big)),
            ("width", T::u32(Big)),
            ("height", T::u32(Big)),
            ("x_offset", T::u32(Big)),
            ("y_offset", T::u32(Big)),
            ("delay_num", T::u16(Big)),
            ("delay_den", T::u16(Big)),
            ("dispose_op", T::enumeration("DisposeOp", T::u8(), &[(0, "none"), (1, "background"), (2, "previous")])),
            ("blend_op", T::enumeration("BlendOp", T::u8(), &[(0, "source"), (1, "over")])),
            // A denominator of nought means a hundredth, so that a writer
            // that leaves it out still means something.
            (
                "delay_seconds",
                T::computed_real(E::field("delay_num").div(E::cond(
                    E::field("delay_den").equal_to(E::lit(0)),
                    E::real(100.0),
                    E::field("delay_den"),
                ))),
            ),
        ],
    )
    .field_doc("delay_num", "The frame's delay is delay_num / delay_den seconds, and a delay_den of 0 means 100.")
    .field_doc("delay_seconds", "How long the frame shows for: delay_num / delay_den, with a delay_den of 0 read as 100.")
}

/// fdAT: a piece of one animation frame's zlib stream, behind a sequence
/// number. Not joined into anything: which frame a piece belongs to is which
/// fcTL came before it, and a walk over the chunks cannot say that.
fn fdat() -> T {
    T::structure("fdAT", vec![("sequence_number", T::u32(Big)), ("frame_data", T::bytes(E::Remaining))])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = (data.len() as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(data);
        v.extend_from_slice(&[0; 4]); // CRC, not checked by the template
        v
    }

    #[test]
    fn text_chunk_splits_at_the_nul() {
        let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
        b.extend_from_slice(&chunk(b"tEXt", b"Author\0Ada Lovelace"));
        b.extend_from_slice(&chunk(b"IEND", b""));
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(png());
        let keyword = ev.node(&d, &[1, 0, 2, 0]).unwrap();
        assert_eq!(keyword.value, Value::Str("Author".into()));
        assert_eq!(keyword.type_name, "latin1 cstr");
        assert_eq!(keyword.size_bits, 7 * 8); // the NUL belongs to the keyword
        let text = ev.node(&d, &[1, 0, 2, 1]).unwrap();
        assert_eq!(text.value, Value::Str("Ada Lovelace".into()));
        assert_eq!(text.offset_bits, (8 + 8 + 7) * 8);
    }

    #[test]
    fn text_chunk_without_a_nul_is_an_error() {
        let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
        b.extend_from_slice(&chunk(b"tEXt", b"nokeyword"));
        b.extend_from_slice(&chunk(b"IEND", b""));
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(png());
        assert!(ev.node(&d, &[1, 0, 2, 0]).is_err());
    }

    #[test]
    fn png_parses_ihdr_and_stops_at_iend() {
        let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
        b.extend_from_slice(&13u32.to_be_bytes());
        b.extend_from_slice(b"IHDR");
        b.extend_from_slice(&640u32.to_be_bytes());
        b.extend_from_slice(&480u32.to_be_bytes());
        b.extend_from_slice(&[8, 6, 0, 0, 0]);
        b.extend_from_slice(&[0; 4]);
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(b"IEND");
        b.extend_from_slice(&[0; 4]);
        b.extend_from_slice(b"trailing junk");
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(png());
        let chunks = ev.node(&d, &[1]).unwrap();
        assert_eq!(chunks.child_count, 2);
        let ihdr = ev.node(&d, &[1, 0, 2]).unwrap();
        assert_eq!(ihdr.type_name, "IHDR");
        assert_eq!(ev.node(&d, &[1, 0, 2, 1]).unwrap().value, Value::UInt(480));
        assert_eq!(ev.node(&d, &[1, 1, 1]).unwrap().value, Value::Str("IEND".into()));
        let color = ev.node(&d, &[1, 0, 2, 3]).unwrap();
        assert_eq!(color.type_name, "ColorType");
        assert_eq!(color.value, Value::Enum { raw: 6, name: Some("rgba".into()), hex: false });
    }

    fn ihdr(width: u32, height: u32, depth: u8, colour_type: u8) -> Vec<u8> {
        let mut data = width.to_be_bytes().to_vec();
        data.extend_from_slice(&height.to_be_bytes());
        data.extend_from_slice(&[depth, colour_type, 0, 0, 0]);
        chunk(b"IHDR", &data)
    }

    /// `raw` as a zlib stream of one stored deflate block: no compression,
    /// so the test needs no encoder.
    fn stored(raw: &[u8]) -> Vec<u8> {
        let mut z = vec![0x78, 0x01, 0x01];
        let len = raw.len() as u16;
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(raw);
        z.extend_from_slice(&crate::checksum::adler32(raw).to_be_bytes());
        z
    }


    fn file(chunks: &[Vec<u8>]) -> Document<MemSource> {
        let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
        for c in chunks {
            b.extend_from_slice(c);
        }
        b.extend_from_slice(&chunk(b"IEND", b""));
        Document::new(MemSource(b))
    }

    /// The path of a field reached by name from `path`, a step at a time.
    /// An empty name is the first child, for the steps a stream takes.
    fn down(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], names: &[&str]) -> Vec<usize> {
        let mut at = path.to_vec();
        for name in names {
            if name.is_empty() {
                at.push(0);
            } else {
                at = ev.child_named(d, &at, name).unwrap().unwrap_or_else(|| panic!("no {name} under {at:?}"));
            }
        }
        at
    }

    #[test]
    fn palette_entries_and_pixels_are_colours() {
        let pixels = [0u8, 0xdc, 0xcd, 0x80, 0x00, 0x00, 0xff];
        let d = file(&[
            ihdr(2, 1, 8, 2),
            chunk(b"PLTE", &[0xdc, 0xcd, 0x80, 0x10, 0x20, 0x30]),
            chunk(b"IDAT", &stored(&pixels)),
        ]);
        let mut ev = Evaluator::new(png());
        // A palette entry is a colour,
        assert_eq!(ev.node(&d, &[1, 1, 2, 0, 0]).unwrap().colour.as_deref(), Some("#dccd80"));
        assert_eq!(ev.node(&d, &[1, 1, 2, 0, 1]).unwrap().colour.as_deref(), Some("#102030"));
        // and that is what it reads as on one line, in a chip or a cell.
        assert_eq!(ev.node(&d, &[1, 1, 2, 0, 1]).unwrap().line.as_deref(), Some("#102030"));
        // A list and a sample are not colours themselves.
        assert_eq!(ev.node(&d, &[1, 1, 2, 0]).unwrap().colour, None);
        assert_eq!(ev.node(&d, &[1, 1, 2, 0, 0, 0]).unwrap().colour, None);
        let px = down(&mut ev, &d, &[2], &["", "compressed", "", "scanlines", "", "pixels"]);
        // Every case of the pixel switch is a different structure and every
        // one is counted as a pixel.
        assert_eq!(ev.node(&d, &px).unwrap().unit.as_deref(), Some("pixel"));
        assert_eq!(ev.node(&d, &[px.as_slice(), &[0]].concat()).unwrap().colour.as_deref(), Some("#dccd80"));
        assert_eq!(ev.node(&d, &[px.as_slice(), &[1]].concat()).unwrap().colour.as_deref(), Some("#0000ff"));
    }

    #[test]
    fn a_narrow_grey_reaches_white_and_alpha_follows_it() {
        // Two-bit grey with no alpha: 0, 1, 2, 3 in one byte, filter none.
        let d = file(&[ihdr(4, 1, 2, 0), chunk(b"IDAT", &stored(&[0, 0b0001_1011]))]);
        let mut ev = Evaluator::new(png());
        let px = down(&mut ev, &d, &[2], &["", "compressed", "", "scanlines", "", "pixels"]);
        let colours: Vec<_> = (0..4).map(|i| ev.node(&d, &[px.as_slice(), &[i]].concat()).unwrap().colour.unwrap()).collect();
        assert_eq!(colours, ["#000000", "#555555", "#aaaaaa", "#ffffff"]);
        // Sixteen-bit grey with alpha.
        let d = file(&[ihdr(1, 1, 16, 4), chunk(b"IDAT", &stored(&[0, 0x80, 0x00, 0xff, 0xff]))]);
        let mut ev = Evaluator::new(png());
        let px = down(&mut ev, &d, &[2], &["", "compressed", "", "scanlines", "", "pixels"]);
        assert_eq!(ev.node(&d, &[px.as_slice(), &[0]].concat()).unwrap().colour.as_deref(), Some("#808080ff"));
    }

    #[test]
    fn background_and_transparent_colours_range_over_the_image_depth() {
        // A four-bit grey image: bKGD and tRNS write sixteen bits, and 15 is
        // white rather than nearly black.
        let d = file(&[ihdr(1, 1, 4, 0), chunk(b"bKGD", &[0, 15]), chunk(b"tRNS", &[0, 3])]);
        let mut ev = Evaluator::new(png());
        assert_eq!(ev.node(&d, &[1, 1, 2]).unwrap().colour.as_deref(), Some("#ffffff"));
        assert_eq!(ev.node(&d, &[1, 2, 2]).unwrap().colour.as_deref(), Some("#333333"));
        // Past the depth's range is a wrong file, and no colour is made up.
        let d = file(&[ihdr(1, 1, 4, 0), chunk(b"bKGD", &[0, 16])]);
        let mut ev = Evaluator::new(png());
        assert_eq!(ev.node(&d, &[1, 1, 2]).unwrap().colour, None);
        // An eight-bit RGB image.
        let d = file(&[ihdr(1, 1, 8, 2), chunk(b"bKGD", &[0, 255, 0, 128, 0, 0])]);
        let mut ev = Evaluator::new(png());
        assert_eq!(ev.node(&d, &[1, 1, 2]).unwrap().colour.as_deref(), Some("#ff8000"));
    }

    #[test]
    fn physical_size_reads_in_dots_per_inch_where_the_unit_is_the_metre() {
        let mut data = 11811u32.to_be_bytes().to_vec();
        data.extend_from_slice(&3937u32.to_be_bytes());
        data.push(1);
        let d = file(&[ihdr(1, 1, 8, 0), chunk(b"pHYs", &data)]);
        let mut ev = Evaluator::new(png());
        let Value::Float(x) = ev.node(&d, &[1, 1, 2, 3]).unwrap().value else { panic!("dpi_x is a real") };
        let Value::Float(y) = ev.node(&d, &[1, 1, 2, 4]).unwrap().value else { panic!("dpi_y is a real") };
        assert!((x - 300.0).abs() < 0.01 && (y - 100.0).abs() < 0.01, "{x} {y}");
        // An aspect ratio alone has no inches in it.
        *data.last_mut().unwrap() = 0;
        let d = file(&[ihdr(1, 1, 8, 0), chunk(b"pHYs", &data)]);
        let mut ev = Evaluator::new(png());
        assert!(ev.node(&d, &[1, 1, 2, 3]).unwrap().absent);
    }

    #[test]
    fn a_frame_delay_reads_in_seconds() {
        let fctl = |num: u16, den: u16| {
            let mut data = vec![0; 20];
            data.extend_from_slice(&num.to_be_bytes());
            data.extend_from_slice(&den.to_be_bytes());
            data.extend_from_slice(&[0, 0]);
            chunk(b"fcTL", &data)
        };
        let d = file(&[ihdr(1, 1, 8, 0), fctl(1, 10), fctl(7, 0)]);
        let mut ev = Evaluator::new(png());
        assert_eq!(ev.node(&d, &[1, 1, 2, 9]).unwrap().value, Value::Float(0.1));
        assert_eq!(ev.node(&d, &[1, 2, 2, 9]).unwrap().value, Value::Float(0.07));
    }
}