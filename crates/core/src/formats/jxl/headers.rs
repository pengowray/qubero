//! The image header of a JPEG XL codestream, ISO/IEC 18181-1 Annex A and the
//! bundles of Annex B it is made of: SizeHeader, ImageMetadata and the
//! transform data, and the small bundles inside them.
//!
//! Every field is read low bit first, and one may start anywhere in a byte and
//! run on into the next. Most of them are written in one of three ways the
//! specification names, which are the helpers here: a Bool of one bit, a fixed
//! number of bits, and a U32, which spends two bits choosing one of four ways
//! the number is written. A bundle that opens with `all_default` holds nothing
//! else when that bit is set, and every field in it takes its default.

use crate::template::{Endian::*, Expr as E, Ty as T};

/// `n` bits read low bit first, which is how every header field is written.
pub(super) fn bits(n: u32) -> T {
    T::UInt { bits: n, endian: Little }
}

/// A Bool: one bit.
pub(super) fn flag() -> T {
    T::enumeration("Bool", bits(1), &[(0, "false"), (1, "true")])
}

/// An F16: a half-precision float of sixteen bits, read low bit first like
/// everything else.
pub(super) fn f16() -> T {
    T::F16(Little)
}

/// What one of the four choices of a [`u32`] reads.
#[derive(Clone, Copy)]
pub(super) enum D {
    /// A number the choice stands for, with no bits.
    Val(i128),
    /// That many bits, and a number to add to them.
    Bits(u32, i128),
}
use D::*;

/// One of the four, by the two-bit selector.
fn by_selector(d: [D; 4], f: impl Fn(D) -> E) -> E {
    let is = |n: i128| E::field("selector").equal_to(E::lit(n));
    E::cond(is(0), f(d[0]), E::cond(is(1), f(d[1]), E::cond(is(2), f(d[2]), f(d[3]))))
}

/// The bits a U32's chosen way reads, as a field of that width, or nothing
/// where the way it chose is a number with no bits.
fn chosen_bits(d: [D; 4]) -> Option<T> {
    let cases: Vec<(i128, T)> = d
        .iter()
        .enumerate()
        .filter_map(|(i, x)| match x {
            Bits(n, _) => Some((i as i128, bits(*n))),
            Val(_) => None,
        })
        .collect();
    let reads = cases.iter().map(|(i, _)| E::field("selector").equal_to(E::lit(*i))).reduce(|a, b| a.either(b))?;
    Some(T::when(reads, T::switch(E::field("selector"), cases, T::bytes(E::lit(0)))))
}

/// A U32: two bits choosing one of four ways the number is written, then the
/// bits that way reads, if it reads any. `value` is the number, and is what
/// the structure reads as on one line.
pub(super) fn u32(d: [D; 4]) -> T {
    let value = by_selector(d, |x| match x {
        Val(v) => E::lit(v),
        Bits(_, offset) => E::field("bits").add(E::lit(offset)),
    });
    let mut fields = vec![("selector", bits(2))];
    fields.extend(chosen_bits(d).map(|t| ("bits", t)));
    fields.push(("value", T::computed(value)));
    T::inline_structure("U32", fields).reads_as(&[("value", "", "")])
}

/// A U32 whose number names one of `cases`.
pub(super) fn named(name: &str, d: [D; 4], cases: &[(i128, &str)]) -> T {
    let value = by_selector(d, |x| match x {
        Val(v) => E::lit(v),
        Bits(_, offset) => E::field("bits").add(E::lit(offset)),
    });
    let mut fields = vec![("selector", bits(2))];
    fields.extend(chosen_bits(d).map(|t| ("bits", t)));
    fields.push(("value", T::enumeration(name, T::computed(value), cases)));
    T::inline_structure(name, fields).reads_as(&[("value", "", "")])
}

/// What the specification calls an Enum: a U32 naming one of `cases`, which
/// is always written the same way.
pub(super) fn enumerated(name: &str, cases: &[(i128, &str)]) -> T {
    named(name, [Val(0), Val(1), Bits(4, 2), Bits(6, 18)], cases)
}

/// A U64: two bits choosing zero, one of two short ranges, or twelve bits
/// followed by as many groups of eight as a continuation bit before each one
/// asks for, the seventh of them only four bits wide.
pub(super) fn u64() -> T {
    let sel = |n: i128| E::field("selector").equal_to(E::lit(n));
    let long = || sel(3);
    let mut fields = vec![
        ("selector", bits(2)),
        ("short", T::when(sel(1), bits(4))),
        ("byte", T::when(sel(2), bits(8))),
        ("low", T::when(long(), bits(12))),
    ];
    // Each continuation bit is there only when every one before it said to go
    // on, and the group after it only when it says so itself. `chain` is that
    // condition so far, asked left to right so that no bit is read that the
    // file did not write.
    const MORE: [&str; 7] = ["more_1", "more_2", "more_3", "more_4", "more_5", "more_6", "more_7"];
    const GROUP: [&str; 7] = ["group_1", "group_2", "group_3", "group_4", "group_5", "group_6", "group_7"];
    let mut chain = long();
    let mut long_value = E::field("low");
    for i in 0..7 {
        fields.push((MORE[i], T::when(chain.clone(), bits(1))));
        chain = chain.both(E::field(MORE[i]).equal_to(E::lit(1)));
        fields.push((GROUP[i], T::when(chain.clone(), bits(if i == 6 { 4 } else { 8 }))));
        // Each group sits above the last; one the file did not write adds nothing.
        long_value = long_value.add(E::cond(chain.clone(), E::field(GROUP[i]).shl(E::lit(12 + 8 * i as i128)), E::lit(0)));
    }
    let value = E::cond(
        sel(0),
        E::lit(0),
        E::cond(sel(1), E::field("short").add(E::lit(1)), E::cond(sel(2), E::field("byte").add(E::lit(17)), long_value)),
    );
    fields.push(("value", T::computed(value)));
    T::inline_structure("U64", fields).reads_as(&[("value", "", "")])
}

/// Zero bits up to the next byte, after `before` bits from the last one:
/// what the specification calls ZeroPadToByte. A field as wide as it needs to
/// be, from one bit to seven, or nothing.
pub(super) fn pad_to_byte(before: E) -> T {
    let width = before.pad_to(8);
    T::when(width.clone().not_equal(E::lit(0)), T::switch(width, (1..8).map(|n| (n as i128, bits(n))).collect(), T::bytes(E::lit(0))))
}

/// The extensions a bundle ends with: a U64 with a bit set for each one, a
/// U64 for each of those saying how many bits it holds, and the bits, which
/// nothing defines yet. Writers today set no bits.
pub(super) fn extensions() -> T {
    let mask = || E::within(&["mask", "value"]);
    let count = (0..64).map(|i| mask().shr(E::lit(i)).and(E::lit(1))).reduce(|a, b| a.add(b)).expect("64 bits");
    let total = (0..64)
        .map(|i| E::cond(E::lit(i).less_than(E::field("count")), E::elem_field("sizes", E::lit(i), &["value"]), E::lit(0)))
        .reduce(|a, b| a.add(b))
        .expect("64 sizes");
    let any = || mask().not_equal(E::lit(0));
    T::inline_structure(
        "Extensions",
        vec![
            ("mask", u64()),
            ("count", T::when(any(), T::computed(count))),
            ("sizes", T::when(any(), T::array(u64(), E::field("count")))),
            ("payload", T::when(any(), T::uint_expr(total, Little))),
        ],
    )
    .reads_as(&[("mask", "", "")])
}

/// How a SizeHeader or a PreviewHeader gives the width from the height:
/// `ratio` picks a fixed aspect ratio, or 0 says the width is written too.
const RATIO: &[(i128, &str)] = &[
    (0, "width given"),
    (1, "1:1"),
    (2, "12:10"),
    (3, "4:3"),
    (4, "3:2"),
    (5, "16:9"),
    (6, "5:4"),
    (7, "2:1"),
];

/// The width a ratio gives a picture `height` high, rounded down.
fn width_by_ratio(ratio: E, height: E) -> E {
    let r = |n: i128| ratio.clone().equal_to(E::lit(n));
    let by = |num: i128, den: i128| height.clone().mul(E::lit(num)).div(E::lit(den));
    E::cond(
        r(1),
        height.clone(),
        E::cond(
            r(2),
            by(12, 10),
            E::cond(r(3), by(4, 3), E::cond(r(4), by(3, 2), E::cond(r(5), by(16, 9), E::cond(r(6), by(5, 4), by(2, 1))))),
        ),
    )
}

/// SizeHeader: the picture's size. `small` says both sides are multiples of
/// eight up to 256, written as five bits each; otherwise each side is a U32.
/// The width is left out when `ratio` gives it from the height.
pub(super) fn size_header() -> T {
    let small = || E::field("small").equal_to(E::lit(1));
    let given = || E::field("ratio").equal_to(E::lit(0));
    let side = || u32([Bits(9, 1), Bits(13, 1), Bits(18, 1), Bits(30, 1)]);
    let height = E::cond(small(), E::field("ysize_div8_minus_1").add(E::lit(1)).mul(E::lit(8)), E::within(&["ysize", "value"]));
    let width = E::cond(
        given(),
        E::cond(small(), E::field("xsize_div8_minus_1").add(E::lit(1)).mul(E::lit(8)), E::within(&["xsize", "value"])),
        width_by_ratio(E::field("ratio"), E::field("height")),
    );
    T::structure(
        "SizeHeader",
        vec![
            ("small", flag()),
            ("ysize_div8_minus_1", T::when(small(), bits(5))),
            ("ysize", T::when(small().negate(), side())),
            ("ratio", T::enumeration("Ratio", bits(3), RATIO)),
            ("xsize_div8_minus_1", T::when(small().both(given()), bits(5))),
            ("xsize", T::when(small().negate().both(given()), side())),
            ("height", T::computed(height)),
            ("width", T::computed(width)),
        ],
    )
}

/// PreviewHeader: the size of the preview frame, written the way SizeHeader
/// writes the picture's but with other ranges, and in eights when `div8` is
/// set.
fn preview_header() -> T {
    let div8 = || E::field("div8").equal_to(E::lit(1));
    let given = || E::field("ratio").equal_to(E::lit(0));
    let eights = || u32([Val(16), Val(32), Bits(5, 1), Bits(9, 33)]);
    let ones = || u32([Bits(6, 1), Bits(8, 65), Bits(10, 321), Bits(12, 1345)]);
    let height = E::cond(div8(), E::within(&["ysize_div8", "value"]).mul(E::lit(8)), E::within(&["ysize", "value"]));
    let width = E::cond(
        given(),
        E::cond(div8(), E::within(&["xsize_div8", "value"]).mul(E::lit(8)), E::within(&["xsize", "value"])),
        width_by_ratio(E::field("ratio"), E::field("height")),
    );
    T::structure(
        "PreviewHeader",
        vec![
            ("div8", flag()),
            ("ysize_div8", T::when(div8(), eights())),
            ("ysize", T::when(div8().negate(), ones())),
            ("ratio", T::enumeration("Ratio", bits(3), RATIO)),
            ("xsize_div8", T::when(div8().both(given()), eights())),
            ("xsize", T::when(div8().negate().both(given()), ones())),
            ("height", T::computed(height)),
            ("width", T::computed(width)),
        ],
    )
}

/// AnimationHeader: ticks per second as a fraction, how many times to play,
/// zero meaning for ever, and whether each frame carries a timecode.
fn animation_header() -> T {
    T::structure(
        "AnimationHeader",
        vec![
            ("tps_numerator", u32([Val(100), Val(1000), Bits(10, 1), Bits(30, 1)])),
            ("tps_denominator", u32([Val(1), Val(1001), Bits(8, 1), Bits(10, 1)])),
            ("num_loops", u32([Val(0), Bits(3, 0), Bits(16, 0), Bits(32, 0)])),
            ("have_timecodes", flag()),
        ],
    )
}

/// BitDepth: integer samples of some number of bits, or floats with an
/// exponent of some number of bits.
fn bit_depth() -> T {
    let float = || E::field("float_sample").equal_to(E::lit(1));
    T::structure(
        "BitDepth",
        vec![
            ("float_sample", flag()),
            (
                "bits_per_sample",
                T::switch(
                    E::field("float_sample"),
                    vec![(1, u32([Val(32), Val(16), Val(24), Bits(6, 1)]))],
                    u32([Val(8), Val(10), Val(12), Bits(6, 1)]),
                ),
            ),
            ("exp_bits_minus_1", T::when(float(), bits(4))),
        ],
    )
}

/// What an extra channel holds, from Table A.25 and the ones reserved after.
const EXTRA_CHANNEL: &[(i128, &str)] = &[
    (0, "alpha"),
    (1, "depth"),
    (2, "spot colour"),
    (3, "selection mask"),
    (4, "black"),
    (5, "colour filter array"),
    (6, "thermal"),
    (15, "non-optional"),
    (16, "optional"),
];

/// ExtraChannelInfo: one channel beyond the colour ones. All defaults is an
/// alpha channel of the image's own depth. The name is its bytes, each eight
/// bits read low bit first.
fn extra_channel() -> T {
    let set = || E::field("all_default").equal_to(E::lit(0));
    let kind = |n: i128| set().both(E::within(&["type", "value"]).equal_to(E::lit(n)));
    let name_len = || E::within(&["name_len", "value"]);
    T::structure(
        "ExtraChannelInfo",
        vec![
            ("all_default", flag()),
            ("type", T::when(set(), enumerated("ExtraChannelType", EXTRA_CHANNEL))),
            ("bit_depth", T::when(set(), bit_depth())),
            ("dim_shift", T::when(set(), u32([Val(0), Val(3), Val(4), Bits(3, 1)]))),
            ("name_len", T::when(set(), u32([Val(0), Bits(4, 0), Bits(5, 16), Bits(10, 48)]))),
            ("name", T::when(set().both(name_len().not_equal(E::lit(0))), T::array(bits(8), name_len()))),
            ("alpha_associated", T::when(kind(0), flag())),
            ("spot_colour", T::when(kind(2), T::array(f16(), E::lit(4)))),
            ("cfa_channel", T::when(kind(5), u32([Val(1), Bits(2, 0), Bits(4, 3), Bits(8, 19)]))),
        ],
    )
    .counted_as("channel")
}

const COLOUR_SPACE: &[(i128, &str)] = &[(0, "RGB"), (1, "greyscale"), (2, "XYB"), (3, "unknown")];
const WHITE_POINT: &[(i128, &str)] = &[(1, "D65"), (2, "custom"), (10, "E"), (11, "DCI")];
const PRIMARIES: &[(i128, &str)] = &[(1, "sRGB"), (2, "custom"), (9, "BT.2100"), (11, "P3")];
const TRANSFER_FUNCTION: &[(i128, &str)] =
    &[(1, "BT.709"), (2, "unknown"), (8, "linear"), (13, "sRGB"), (16, "PQ"), (17, "DCI"), (18, "HLG")];
const RENDERING_INTENT: &[(i128, &str)] = &[(0, "perceptual"), (1, "relative"), (2, "saturation"), (3, "absolute")];

/// Customxy: a chromaticity, each coordinate a U32 holding a signed number in
/// millionths, the sign in the bottom bit.
fn customxy() -> T {
    let d = [Bits(19, 0), Bits(19, 524_288), Bits(20, 1_048_576), Bits(21, 2_097_152)];
    let signed = |name: &str| {
        let v = E::within(&[name, "value"]);
        E::cond(v.clone().and(E::lit(1)), v.clone().add(E::lit(1)).div(E::lit(2)).mul(E::lit(-1)), v.div(E::lit(2)))
    };
    T::structure(
        "Customxy",
        vec![
            ("x_packed", u32(d)),
            ("y_packed", u32(d)),
            ("x", T::computed_real(signed("x_packed").div(E::real(1_000_000.0)))),
            ("y", T::computed_real(signed("y_packed").div(E::real(1_000_000.0)))),
        ],
    )
}

/// ColourEncoding: the colour space, white point, primaries, transfer function
/// and rendering intent, or that an ICC profile follows the image header and
/// says instead. All defaults is sRGB.
///
/// The profile is entropy-coded, so how long it is is known only once it is
/// decoded, and the codestream after it stays bytes; see the module notes.
fn colour_encoding() -> T {
    let set = || E::field("all_default").equal_to(E::lit(0));
    let space = || E::within(&["colour_space", "value"]);
    let enums = || set().both(E::field("want_icc").equal_to(E::lit(0)));
    let not_xyb = || enums().both(space().not_equal(E::lit(2)));
    let custom_white = || not_xyb().both(E::within(&["white_point", "value"]).equal_to(E::lit(2)));
    let primaries = || not_xyb().both(space().not_equal(E::lit(1)));
    let custom_primaries = || primaries().both(E::within(&["primaries", "value"]).equal_to(E::lit(2)));
    let gamma = || not_xyb().both(E::field("have_gamma").equal_to(E::lit(1)));
    T::structure(
        "ColourEncoding",
        vec![
            ("all_default", flag()),
            ("want_icc", T::when(set(), flag())),
            ("colour_space", T::when(set(), enumerated("ColourSpace", COLOUR_SPACE))),
            ("white_point", T::when(not_xyb(), enumerated("WhitePoint", WHITE_POINT))),
            ("white", T::when(custom_white(), customxy())),
            ("primaries", T::when(primaries(), enumerated("Primaries", PRIMARIES))),
            ("red", T::when(custom_primaries(), customxy())),
            ("green", T::when(custom_primaries(), customxy())),
            ("blue", T::when(custom_primaries(), customxy())),
            ("have_gamma", T::when(not_xyb(), flag())),
            ("gamma", T::when(gamma(), bits(24))),
            ("transfer_function", T::when(not_xyb().both(E::field("have_gamma").equal_to(E::lit(0))), enumerated("TransferFunction", TRANSFER_FUNCTION))),
            ("rendering_intent", T::when(enums(), enumerated("RenderingIntent", RENDERING_INTENT))),
        ],
    )
}

/// ToneMapping: the brightest the picture is meant to be shown, in nits, and
/// how to map it onto a display that is not that bright.
fn tone_mapping() -> T {
    let set = || E::field("all_default").equal_to(E::lit(0));
    T::structure(
        "ToneMapping",
        vec![
            ("all_default", flag()),
            ("intensity_target", T::when(set(), f16())),
            ("min_nits", T::when(set(), f16())),
            ("relative_to_max_display", T::when(set(), flag())),
            ("linear_below", T::when(set(), f16())),
        ],
    )
}

/// ImageMetadata: everything about the picture beyond its size. All defaults
/// is eight-bit sRGB, stored as XYB, with no extra channels.
pub(super) fn image_metadata() -> T {
    let set = || E::field("all_default").equal_to(E::lit(0));
    let extra = || set().both(E::field("extra_fields").equal_to(E::lit(1)));
    let has = |name: &str| extra().both(E::field(name).equal_to(E::lit(1)));
    let channels = || E::within(&["num_extra_channels", "value"]);
    T::structure(
        "ImageMetadata",
        vec![
            ("all_default", flag()),
            ("extra_fields", T::when(set(), flag())),
            ("orientation_minus_1", T::when(extra(), bits(3))),
            ("have_intrinsic_size", T::when(extra(), flag())),
            ("intrinsic_size", T::when(has("have_intrinsic_size"), size_header())),
            ("have_preview", T::when(extra(), flag())),
            ("preview", T::when(has("have_preview"), preview_header())),
            ("have_animation", T::when(extra(), flag())),
            ("animation", T::when(has("have_animation"), animation_header())),
            ("bit_depth", T::when(set(), bit_depth())),
            ("modular_16_bit_buffer_sufficient", T::when(set(), flag())),
            ("num_extra_channels", T::when(set(), u32([Val(0), Val(1), Bits(4, 2), Bits(12, 1)]))),
            ("extra_channels", T::when(set().both(channels().not_equal(E::lit(0))), T::array(extra_channel(), channels()))),
            ("xyb_encoded", T::when(set(), flag())),
            ("colour_encoding", T::when(set(), colour_encoding())),
            ("tone_mapping", T::when(extra(), tone_mapping())),
            ("extensions", T::when(set(), extensions())),
        ],
    )
}

/// OpsinInverseMatrix: the matrix and biases that turn XYB back into linear
/// RGB, when they are not the standard ones.
fn opsin_inverse_matrix() -> T {
    let set = || E::field("all_default").equal_to(E::lit(0));
    T::structure(
        "OpsinInverseMatrix",
        vec![
            ("all_default", flag()),
            ("inverse_matrix", T::when(set(), T::array(f16(), E::lit(9)))),
            ("opsin_biases", T::when(set(), T::array(f16(), E::lit(3)))),
            ("quant_biases", T::when(set(), T::array(f16(), E::lit(4)))),
        ],
    )
}

/// The transform data: the XYB matrix, and the weights for upsampling by two,
/// four and eight where the codestream gives its own. `xyb` is whether the
/// image metadata says the samples are XYB.
pub(super) fn transform_data(xyb: E) -> T {
    let set = || E::field("all_default").equal_to(E::lit(0));
    let mask = |bit: u32| set().both(E::field("custom_weights_mask").bit(bit));
    T::structure(
        "CustomTransformData",
        vec![
            ("all_default", flag()),
            ("opsin_inverse_matrix", T::when(set().both(xyb), opsin_inverse_matrix())),
            ("custom_weights_mask", T::when(set(), bits(3))),
            ("upsampling2_weights", T::when(mask(0), T::array(f16(), E::lit(15)))),
            ("upsampling4_weights", T::when(mask(1), T::array(f16(), E::lit(55)))),
            ("upsampling8_weights", T::when(mask(2), T::array(f16(), E::lit(210)))),
        ],
    )
}
