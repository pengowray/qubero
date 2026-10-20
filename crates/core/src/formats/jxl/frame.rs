//! One frame of a JPEG XL codestream, ISO/IEC 18181-1 Annex C: its header,
//! its table of contents, and the sections the table lists.
//!
//! The header says how the frame is coded and how big it is, and from that
//! comes how many groups it is cut into: 256 by 256 pixels by default, and an
//! LF group for every 8 by 8 of those. The TOC is a length for each section in
//! order: LfGlobal, one LfGroup each, HfGlobal, and a PassGroup for every
//! group in every pass. A frame of one group and one pass keeps all four in
//! one section with one length. The sections themselves are entropy-coded and
//! stay bytes, each named for what it is.
//!
//! The TOC may say its sections are stored in another order, and give the
//! order as an entropy-coded permutation, whose length is known only once it
//! is decoded. A frame like that is read as far as the flag, and the rest of
//! the codestream stays bytes.

use super::headers::{bits, extensions, f16, flag, named, pad_to_byte, u32, u64, D::*};
use crate::template::{Expr as E, Ty as T, Until};

/// What the image header says, from inside a frame. Each is the default the
/// specification gives when the bundle holding it was left at its defaults.
pub(super) struct Image;

impl Image {
    fn metadata(path: &[&str]) -> E {
        E::within(&[&["image_header", "metadata"], path].concat())
    }

    fn metadata_default() -> E {
        Image::metadata(&["all_default"]).equal_to(E::lit(1))
    }

    /// Whether the samples are XYB.
    pub(super) fn xyb() -> E {
        E::cond(Image::metadata_default(), E::lit(1), Image::metadata(&["xyb_encoded"]))
    }

    /// How many extra channels there are.
    pub(super) fn extra_channels() -> E {
        E::cond(Image::metadata_default(), E::lit(0), Image::metadata(&["num_extra_channels", "value"]))
    }

    /// Whether the image is an animation.
    fn animated() -> E {
        E::cond(
            Image::metadata_default(),
            E::lit(0),
            E::cond(Image::metadata(&["extra_fields"]).equal_to(E::lit(1)), Image::metadata(&["have_animation"]), E::lit(0)),
        )
    }

    /// Whether each frame of an animation carries a timecode.
    fn timecodes() -> E {
        Image::animated().both(Image::metadata(&["animation", "have_timecodes"]).equal_to(E::lit(1)))
    }

    /// Whether the colour encoding is an ICC profile written after the image
    /// header.
    pub(super) fn want_icc() -> E {
        Image::metadata_default().negate().both(
            Image::metadata(&["colour_encoding", "all_default"])
                .equal_to(E::lit(0))
                .both(Image::metadata(&["colour_encoding", "want_icc"]).equal_to(E::lit(1))),
        )
    }

    /// Whether a preview frame comes before the first frame.
    pub(super) fn preview() -> E {
        Image::metadata_default()
            .negate()
            .both(Image::metadata(&["extra_fields"]).equal_to(E::lit(1)))
            .both(Image::metadata(&["have_preview"]).equal_to(E::lit(1)))
    }

    /// The picture's size, or the preview's for the preview frame.
    fn size(preview: bool, side: &str) -> E {
        match preview {
            true => Image::metadata(&["preview", side]),
            false => E::within(&["image_header", "size", side]),
        }
    }
}

const FRAME_TYPE: &[(i128, &str)] = &[(0, "regular"), (1, "LF frame"), (2, "reference only"), (3, "skip progressive")];
const ENCODING: &[(i128, &str)] = &[(0, "VarDCT"), (1, "modular")];
const FLAGS: &[(u32, &str)] = &[(0, "noise"), (1, "patches"), (4, "splines"), (5, "use LF frame"), (7, "skip adaptive LF smoothing")];
const BLEND_MODE: &[(i128, &str)] = &[(0, "replace"), (1, "add"), (2, "blend"), (3, "alpha-weighted add"), (4, "multiply")];

/// Passes: how many there are, and for a progressive frame, which pass each
/// downsampled version is complete after.
fn passes() -> T {
    let many = || E::within(&["num_passes", "value"]).not_equal(E::lit(1));
    let ds = || E::within(&["num_ds", "value"]);
    T::structure(
        "Passes",
        vec![
            ("num_passes", u32([Val(1), Val(2), Val(3), Bits(3, 4)])),
            ("num_ds", T::when(many(), u32([Val(0), Val(1), Val(2), Bits(1, 3)]))),
            ("shift", T::when(many(), T::array(bits(2), E::within(&["num_passes", "value"]).sub(E::lit(1))))),
            ("downsample", T::when(many().both(ds().not_equal(E::lit(0))), T::array(u32([Val(1), Val(2), Val(4), Val(8)]), ds()))),
            ("last_pass", T::when(many().both(ds().not_equal(E::lit(0))), T::array(u32([Val(0), Val(1), Val(2), Bits(3, 0)]), ds()))),
        ],
    )
}

/// A crop's origin and size. The origin is a signed number packed into a U32
/// with its sign in the bottom bit, and a reference-only frame has none.
fn crop() -> T {
    let d = [Bits(8, 0), Bits(11, 256), Bits(14, 2304), Bits(30, 18688)];
    let origin = || T::when(E::field("frame_type").equal_to(E::lit(2)).negate(), u32(d));
    T::structure("Crop", vec![("ux0", origin()), ("uy0", origin()), ("xsize", u32(d)), ("ysize", u32(d))])
}

/// BlendingInfo: how the frame is laid over the canvas. `partial` is whether
/// the frame covers less than the whole canvas.
fn blending_info(partial: E) -> T {
    let mode = || E::within(&["mode", "value"]);
    let alpha = || Image::extra_channels().not_equal(E::lit(0)).both(mode().equal_to(E::lit(2)).either(mode().equal_to(E::lit(3))));
    T::structure(
        "BlendingInfo",
        vec![
            ("mode", named("BlendMode", [Val(0), Val(1), Val(2), Bits(2, 3)], BLEND_MODE)),
            ("alpha_channel", T::when(alpha(), u32([Val(0), Val(1), Val(2), Bits(3, 3)]))),
            ("clamp", T::when(alpha().either(mode().equal_to(E::lit(4))), flag())),
            ("source", T::when(mode().not_equal(E::lit(0)).either(partial), bits(2))),
        ],
    )
}

/// RestorationFilter: the Gabor-like smoothing and the edge-preserving filter
/// run over the decoded frame, with custom weights where the codestream gives
/// them. `modular` is whether the frame is modular-coded.
fn restoration_filter(modular: E) -> T {
    let set = || E::field("all_default").equal_to(E::lit(0));
    let on = |name: &str| E::field(name).equal_to(E::lit(1));
    let epf = || set().both(E::field("epf_iters").not_equal(E::lit(0)));
    let vardct = || modular.clone().negate();
    T::structure(
        "RestorationFilter",
        vec![
            ("all_default", flag()),
            ("gab", T::when(set(), flag())),
            ("gab_custom", T::when(set().both(on("gab")), flag())),
            ("gab_weights", T::when(set().both(on("gab")).both(on("gab_custom")), T::array(f16(), E::lit(6)))),
            ("epf_iters", T::when(set(), bits(2))),
            ("epf_sharp_custom", T::when(epf().both(vardct()), flag())),
            ("epf_sharp_lut", T::when(epf().both(vardct()).both(on("epf_sharp_custom")), T::array(f16(), E::lit(8)))),
            ("epf_weight_custom", T::when(epf(), flag())),
            ("epf_channel_scale", T::when(epf().both(on("epf_weight_custom")), T::array(f16(), E::lit(3)))),
            ("epf_pass1_zeroflush", T::when(epf().both(on("epf_weight_custom")), f16())),
            ("epf_pass2_zeroflush", T::when(epf().both(on("epf_weight_custom")), f16())),
            ("epf_sigma_custom", T::when(epf(), flag())),
            ("epf_quant_mul", T::when(epf().both(on("epf_sigma_custom")).both(vardct()), f16())),
            ("epf_pass0_sigma_scale", T::when(epf().both(on("epf_sigma_custom")), f16())),
            ("epf_pass2_sigma_scale", T::when(epf().both(on("epf_sigma_custom")), f16())),
            ("epf_border_sad_mul", T::when(epf().both(on("epf_sigma_custom")), f16())),
            ("epf_sigma_for_modular", T::when(epf().both(modular), f16())),
            ("extensions", T::when(set(), extensions())),
        ],
    )
}

/// FrameHeader: how the frame is coded, where it goes and how big it is, and
/// what is done with it once it is decoded.
///
/// The last few fields are worked out rather than read: the frame's size as
/// it is coded, after any crop, upsampling and LF level, and from that how
/// many groups and LF groups the TOC lists sections for. `preview` is whether
/// this is the preview frame, whose size the preview header gives.
fn frame_header(preview: bool) -> T {
    let set = || E::field("all_default").equal_to(E::lit(0));
    let kind = || E::field("frame_type");
    let is = |n: i128| set().both(kind().equal_to(E::lit(n)));
    let isnt = |n: i128| set().both(kind().not_equal(E::lit(n)));
    let normal = || set().both(kind().equal_to(E::lit(0)).either(kind().equal_to(E::lit(3))));
    let modular = || set().both(E::field("encoding").equal_to(E::lit(1)));
    let vardct = || set().both(E::field("encoding").equal_to(E::lit(0)));
    let feature = |bit: i128| E::within(&["flags", "value"]).shr(E::lit(bit)).and(E::lit(1));
    let no_lf_frame = || set().both(feature(5).equal_to(E::lit(0)));
    let ycbcr = || set().both(Image::xyb().negate()).both(E::field("do_ycbcr").equal_to(E::lit(1)));
    let channels = Image::extra_channels;
    let cropped = || isnt(1).both(E::field("have_crop").equal_to(E::lit(1)));
    let animated = || normal().both(Image::animated());
    // `is_last` is read only for a normal frame; any other kind is never last,
    // and a frame left at its defaults always is.
    let last = || E::cond(set(), E::cond(normal(), E::field("is_last"), E::lit(0)), E::lit(1));
    let duration = || E::cond(animated(), E::within(&["duration", "value"]), E::lit(0));
    let save_as_reference = || E::cond(set().both(kind().not_equal(E::lit(1))).both(last().negate()), E::field("save_as_reference"), E::lit(0));
    let replaces = || E::within(&["blending_info", "mode", "value"]).equal_to(E::lit(0));
    let save_before_ct = || {
        let referenced = last().negate().both(isnt(1));
        let normal_reset = normal()
            .both(replaces())
            .both(E::field("have_crop").equal_to(E::lit(0)))
            .both(duration().equal_to(E::lit(0)).either(save_as_reference().not_equal(E::lit(0))));
        referenced.both(is(2).either(normal_reset))
    };
    let name_len = || E::within(&["name_len", "value"]);

    // What the frame is as coded: its own size where it is cropped, the
    // picture's otherwise, an eighth per LF level for an LF frame, and then
    // divided by the upsampling it is drawn back up with.
    let upsampling = E::cond(no_lf_frame(), E::within(&["upsampling", "value"]), E::lit(1));
    let lf_level = E::cond(is(1), E::within(&["lf_level", "value"]), E::lit(0));
    let side = |name: &str, crop: &str| {
        let whole = Image::size(preview, name);
        let whole = E::cond(lf_level.clone().not_equal(E::lit(0)), whole.clone().div_ceil(E::lit(1).shl(lf_level.clone().mul(E::lit(3)))), whole);
        E::cond(cropped(), E::within(&["crop", crop, "value"]), whole).div_ceil(upsampling.clone())
    };
    let group_dim = E::lit(128).shl(E::cond(modular(), E::field("group_size_shift"), E::lit(1)));
    let count = |dim: E| E::field("width").div_ceil(dim.clone()).mul(E::field("height").div_ceil(dim));

    T::structure(
        "FrameHeader",
        vec![
            ("all_default", flag()),
            ("frame_type", T::when(set(), T::enumeration("FrameType", bits(2), FRAME_TYPE))),
            ("encoding", T::when(set(), T::enumeration("Encoding", bits(1), ENCODING))),
            ("flags", T::when(set(), u64())),
            ("features", T::when(set(), T::flags("FrameFlags", T::computed(E::within(&["flags", "value"])), FLAGS))),
            ("do_ycbcr", T::when(set().both(Image::xyb().negate()), flag())),
            ("jpeg_upsampling", T::when(ycbcr().both(feature(5).equal_to(E::lit(0))), T::array(bits(2), E::lit(3)))),
            ("upsampling", T::when(no_lf_frame(), u32([Val(1), Val(2), Val(4), Val(8)]))),
            ("ec_upsampling", T::when(no_lf_frame().both(channels().not_equal(E::lit(0))), T::array(u32([Val(1), Val(2), Val(4), Val(8)]), channels()))),
            ("group_size_shift", T::when(modular(), bits(2))),
            ("x_qm_scale", T::when(vardct().both(Image::xyb()), bits(3))),
            ("b_qm_scale", T::when(vardct().both(Image::xyb()), bits(3))),
            ("passes", T::when(isnt(2), passes())),
            ("lf_level", T::when(is(1), u32([Val(1), Val(2), Val(3), Val(4)]))),
            ("have_crop", T::when(isnt(1), flag())),
            ("crop", T::when(cropped(), crop())),
            ("blending_info", T::when(normal(), blending_info(cropped()))),
            ("ec_blending_info", T::when(normal().both(channels().not_equal(E::lit(0))), T::array(blending_info(cropped()), channels()))),
            ("duration", T::when(animated(), u32([Val(0), Val(1), Bits(8, 0), Bits(32, 0)]))),
            ("timecode", T::when(animated().both(Image::timecodes()), bits(32))),
            ("is_last", T::when(normal(), flag())),
            ("save_as_reference", T::when(set().both(kind().not_equal(E::lit(1))).both(last().negate()), bits(2))),
            ("save_before_ct", T::when(set().both(save_before_ct()), flag())),
            ("name_len", T::when(set(), u32([Val(0), Bits(4, 0), Bits(5, 16), Bits(10, 48)]))),
            ("name", T::when(set().both(name_len().not_equal(E::lit(0))), T::array(bits(8), name_len()))),
            ("restoration_filter", T::when(set(), restoration_filter(E::field("encoding").equal_to(E::lit(1))))),
            ("extensions", T::when(set(), extensions())),
            ("last", T::computed(last())),
            ("width", T::computed(side("width", "xsize"))),
            ("height", T::computed(side("height", "ysize"))),
            ("group_dim", T::computed(group_dim)),
            ("groups", T::computed(count(E::field("group_dim")))),
            ("lf_groups", T::computed(count(E::field("group_dim").mul(E::lit(8))))),
            ("num_passes", T::computed(E::cond(isnt(2), E::within(&["passes", "num_passes", "value"]), E::lit(1)))),
        ],
    )
}

/// The TOC: whether the sections are stored out of order, and a length in
/// bytes for each of them, padded to a byte before and after.
///
/// `before` is how many bits of the frame come before the TOC, which is what
/// the first padding makes up to a byte. How many lengths there are is the
/// frame header's to say.
fn toc() -> T {
    let entries = E::cond(
        E::within(&["frame_header", "groups"]).equal_to(E::lit(1)).both(E::within(&["frame_header", "num_passes"]).equal_to(E::lit(1))),
        E::lit(1),
        E::lit(2)
            .add(E::within(&["frame_header", "lf_groups"]))
            .add(E::within(&["frame_header", "groups"]).mul(E::within(&["frame_header", "num_passes"]))),
    );
    let in_order = || E::field("permuted").equal_to(E::lit(0));
    let entry = u32([Bits(10, 0), Bits(14, 1024), Bits(22, 17408), Bits(30, 4_211_712)]).counted_as("section");
    T::structure(
        "Toc",
        vec![
            ("permuted", flag()),
            ("permutation_start", T::when(in_order().negate(), pad_to_byte(E::bits_of("frame_header").add(E::lit(1))))),
            ("pad", T::when(in_order(), pad_to_byte(E::bits_of("frame_header").add(E::lit(1))))),
            ("entries", T::when(in_order(), T::array(entry, entries))),
            ("end_pad", T::when(in_order(), pad_to_byte(E::bits_of("entries")))),
        ],
    )
    .field_doc("permutation_start", "The first bits of the TOC permutation, up to the next byte boundary. The permutation starts straight after the permuted flag.")
    .field_doc("pad", "ZeroPadToByte: bits set to 0 up to the next byte boundary, where the section lengths start.")
    .field_doc("end_pad", "ZeroPadToByte: bits set to 0 up to the next byte boundary, where the first section starts.")
}

/// The sections the TOC lists, each as the bytes its length says.
fn sections() -> T {
    let length = |i: E| E::elem_within(&["toc", "entries"], i, &["value"]);
    let header = |name: &str| E::within(&["frame_header", name]);
    let lf_groups = || header("lf_groups");
    let groups = || header("groups");
    let pass_group = |pass: E| T::bytes(length(E::lit(2).add(lf_groups()).add(pass.mul(groups())).add(E::Idx))).counted_as("group");
    let each_pass = T::structure("Pass", vec![("pass", T::computed(E::Idx)), ("groups", T::array(pass_group(E::field("pass")), groups()))])
        .counted_as("pass");
    T::structure(
        "Sections",
        vec![
            ("LfGlobal", T::bytes(length(E::lit(0)))),
            ("LfGroup", T::array(T::bytes(length(E::lit(1).add(E::Idx))).counted_as("LF group"), lf_groups())),
            ("HfGlobal", T::bytes(length(E::lit(1).add(lf_groups())))),
            (
                "PassGroup",
                T::switch(header("num_passes"), vec![(1, T::array(pass_group(E::lit(0)), groups()))], T::array(each_pass, header("num_passes"))),
            ),
        ],
    )
}

/// One frame: its header, its TOC, and its sections. A frame whose TOC is
/// permuted has its sections and everything after them left as bytes.
pub(super) fn frame(preview: bool) -> T {
    let single = E::within(&["frame_header", "groups"]).equal_to(E::lit(1)).both(E::within(&["frame_header", "num_passes"]).equal_to(E::lit(1)));
    let permuted = || E::within(&["toc", "permuted"]).equal_to(E::lit(1));
    let one = T::bytes(E::elem_within(&["toc", "entries"], E::lit(0), &["value"]));
    T::structure(
        "Frame",
        vec![
            ("frame_header", frame_header(preview)),
            ("toc", toc()),
            ("sections", T::when(permuted().negate(), T::switch(single, vec![(1, one)], sections()))),
            ("unread", T::when(permuted(), T::bytes(E::Remaining))),
        ],
    )
    .field_doc(
        "sections",
        "The sections listed in the TOC. A frame with one group and one pass has a single section, which contains LfGlobal, LfGroup, HfGlobal and PassGroup.",
    )
    .field_doc(
        "unread",
        "The rest of the codestream, shown as bytes. The TOC of this frame gives the order of the sections as an entropy-coded permutation, and the end of the permutation can be found only by decoding the permutation, which Qubero does not do.",
    )
    .counted_as("frame")
}

/// The frames: one after another until the one that says it is the last, or
/// one whose sections cannot be found.
pub(super) fn frames() -> T {
    let last = E::within(&["frame_header", "last"]).equal_to(E::lit(1));
    T::repeat(frame(false), Until::Cond(last.either(E::within(&["toc", "permuted"]).equal_to(E::lit(1)))))
}
