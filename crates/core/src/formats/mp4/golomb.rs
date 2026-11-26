//! The exp-Golomb coded headers of H.264: the sequence parameter set, the
//! picture parameter set, and the first fields of every slice.
//!
//! An exp-Golomb number, ue(v), is n zero bits, a one bit, then n more bits,
//! and its value is 2^n - 1 plus those n bits: `1` is 0, `010` is 1, `011` is
//! 2, `00100` is 3. The zeros are counted by looking at the next 16 bits
//! before reading them, so each number is three fields a reader can check
//! against the bits, and a fourth with the value.
//!
//! A NAL unit's payload is not quite its bits. Wherever the encoder would have
//! written `00 00 00`, `00 00 01`, `00 00 02` or `00 00 03`, it writes an extra
//! `03` after the two zeros, so that no start code appears by accident, and a
//! decoder takes the `03` out again. The IR has no way to read through bytes
//! with those taken out, so a header is read only where no `03` comes in its
//! first 10 bytes for an SPS, 8 for a PPS and 6 for a slice, which is further
//! than the fields read here reach in every file of the sample collection. A
//! header with an earlier `03` stays bytes, and so does everything after the
//! fields read.
//!
//! The check is a guess at how far the fields reach, not a measurement: an
//! SPS with large cropping offsets could run past byte 10, and if a `03` sat
//! just past it the fields after it would read wrong.

use super::bits;
use crate::template::{Endian::*, Expr as E, Ty as T};

/// One ue(v) number. `value` is what it says.
fn ue() -> T {
    ue_with(T::computed(value()))
}

/// The same, with the value read as one of `cases`.
fn ue_named(name: &str, cases: &[(i128, &str)]) -> T {
    ue_with(T::enumeration(name, T::computed(value()), cases))
}

fn value() -> E {
    E::lit(1).shl(E::bits_of("leading_zeros")).sub(E::lit(1)).add(E::field("info"))
}

fn ue_with(value: T) -> T {
    // The next 16 bits, or the next 8 with zeros after them where fewer than
    // two bytes are left.
    let ahead = E::cond(E::Remaining.greater_or_equal(E::lit(2)), E::peek(16, Big), E::peek(8, Big).shl(E::lit(8)));
    T::structure(
        "ExpGolomb",
        vec![
            ("leading_zeros", T::uint_expr(E::lit(15).sub(ahead.log2()), Big)),
            ("marker", bits(1)),
            ("info", T::uint_expr(E::bits_of("leading_zeros"), Big)),
            ("value", value),
        ],
    )
    .named_by("value")
    .machinery(&["leading_zeros", "marker", "info"])
}

/// One se(v) number: a ue(v) code k read as 0, 1, -1, 2, -2, ... in turn.
fn se() -> T {
    let code = E::field("code");
    let signed = E::cond(code.clone().and(E::lit(1)), code.clone().add(E::lit(1)).div(E::lit(2)), E::lit(0).sub(code.div(E::lit(2))));
    let ahead = E::cond(E::Remaining.greater_or_equal(E::lit(2)), E::peek(16, Big), E::peek(8, Big).shl(E::lit(8)));
    T::structure(
        "SignedExpGolomb",
        vec![
            ("leading_zeros", T::uint_expr(E::lit(15).sub(ahead.log2()), Big)),
            ("marker", bits(1)),
            ("info", T::uint_expr(E::bits_of("leading_zeros"), Big)),
            ("code", T::computed(value())),
            ("value", T::computed(signed)),
        ],
    )
    .named_by("value")
    .machinery(&["leading_zeros", "marker", "info", "code"])
}

fn v(field: &str) -> E {
    E::within(&[field, "value"])
}

/// Whether the first `n` bytes from here hold no emulation prevention `03`.
fn clean_for(n: i128) -> E {
    E::to_bytes(&[0, 0, 3]).add(E::lit(2)).greater_or_equal(E::lit(n).at_most(E::Remaining))
}

/// H.264 profiles, as profile_idc writes them.
pub(super) const PROFILE: &[(i128, &str)] = &[
    (44, "CAVLC 4:4:4 intra"),
    (66, "baseline"),
    (77, "main"),
    (83, "scalable baseline"),
    (86, "scalable high"),
    (88, "extended"),
    (100, "high"),
    (110, "high 10"),
    (118, "multiview high"),
    (122, "high 4:2:2"),
    (128, "stereo high"),
    (244, "high 4:4:4"),
];

/// An SPS payload, read up to the VUI, or bytes where an emulation
/// prevention byte comes too early.
pub(super) fn sps_rbsp() -> T {
    let rest = "The rest of the SPS, unparsed: the VUI parameters (when vui_parameters_present_flag is 1) and the stop bit. Also every field after seq_scaling_matrix_present_flag when that is 1, or after pic_order_cnt_type when that is 1.";
    T::switch(clean_for(10), vec![(1, then_the_rest("SequenceParameterSetRbsp", sps(), rest))], T::bytes(E::Remaining))
}

/// `header`, then what is left of the payload: first the bits up to the next
/// byte boundary, so that the bytes after them are whole bytes and every bit
/// of the payload belongs to a field.
pub(super) fn then_the_rest(name: &str, header: T, rest: &str) -> T {
    let to_byte = E::lit(8).sub(E::bits_of("header").modulo(E::lit(8))).modulo(E::lit(8));
    T::structure(name, vec![("header", header), ("rest_bits", T::uint_expr(to_byte, Big)), ("rest", T::bytes(E::Remaining))])
        .contents("header")
        .named_by("header")
        .field_doc("rest_bits", "The bits of the rest that come before the next byte boundary.")
        .field_doc("rest", rest)
}

fn sps() -> T {
    let profile = E::field("profile_idc");
    let high = [100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135]
        .iter()
        .map(|p| profile.clone().equal_to(E::lit(*p)))
        .reduce(|a, b| a.either(b))
        .unwrap();
    // Scaling matrices and the second kind of picture order count are lists
    // of their own, which are not read; everything after either stays bytes.
    let plain = E::cond(high.clone(), E::field("seq_scaling_matrix_present_flag").equal_to(E::lit(0)), E::lit(1));
    let simple = plain.clone().both(v("pic_order_cnt_type").not_equal(E::lit(1)));
    let cropped = simple.clone().both(E::field("frame_cropping_flag"));
    let chroma = E::cond(high.clone(), v("chroma_format_idc"), E::lit(1));
    let frame_mbs_only = E::field("frame_mbs_only_flag");
    // How many luma samples one unit of cropping is, across and down.
    let crop_x = E::cond(chroma.clone().equal_to(E::lit(1)).either(chroma.clone().equal_to(E::lit(2))), E::lit(2), E::lit(1));
    let crop_y = E::cond(chroma.equal_to(E::lit(1)), E::lit(2), E::lit(1)).mul(E::lit(2).sub(frame_mbs_only.clone()));
    let crop = |a: &str, b: &str| E::cond(E::field("frame_cropping_flag"), v(a).add(v(b)), E::lit(0));
    let width = v("pic_width_in_mbs_minus1").add(E::lit(1)).mul(E::lit(16)).sub(crop_x.mul(crop("frame_crop_left_offset", "frame_crop_right_offset")));
    let height = E::lit(2)
        .sub(frame_mbs_only.clone())
        .mul(v("pic_height_in_map_units_minus1").add(E::lit(1)))
        .mul(E::lit(16))
        .sub(crop_y.mul(crop("frame_crop_top_offset", "frame_crop_bottom_offset")));
    T::structure(
        "SequenceParameterSet",
        vec![
            ("profile_idc", T::enumeration("H264Profile", T::u8(), PROFILE)),
            (
                "constraint_flags",
                T::flags(
                    "ConstraintFlags",
                    T::u8(),
                    &[(7, "constraint_set0"), (6, "constraint_set1"), (5, "constraint_set2"), (4, "constraint_set3"), (3, "constraint_set4"), (2, "constraint_set5")],
                ),
            ),
            ("level_idc", T::u8()),
            ("seq_parameter_set_id", ue()),
            ("chroma_format_idc", T::when(high.clone(), ue_named("ChromaFormat", &[(0, "monochrome"), (1, "4:2:0"), (2, "4:2:2"), (3, "4:4:4")]))),
            ("separate_colour_plane_flag", T::when(high.clone().both(v("chroma_format_idc").equal_to(E::lit(3))), bits(1))),
            ("bit_depth_luma_minus8", T::when(high.clone(), ue())),
            ("bit_depth_chroma_minus8", T::when(high.clone(), ue())),
            ("qpprime_y_zero_transform_bypass_flag", T::when(high.clone(), bits(1))),
            ("seq_scaling_matrix_present_flag", T::when(high, bits(1))),
            ("log2_max_frame_num_minus4", T::when(plain.clone(), ue())),
            ("pic_order_cnt_type", T::when(plain.clone(), ue())),
            ("log2_max_pic_order_cnt_lsb_minus4", T::when(plain.both(v("pic_order_cnt_type").equal_to(E::lit(0))), ue())),
            ("max_num_ref_frames", T::when(simple.clone(), ue())),
            ("gaps_in_frame_num_value_allowed_flag", T::when(simple.clone(), bits(1))),
            ("pic_width_in_mbs_minus1", T::when(simple.clone(), ue())),
            ("pic_height_in_map_units_minus1", T::when(simple.clone(), ue())),
            ("frame_mbs_only_flag", T::when(simple.clone(), bits(1))),
            ("mb_adaptive_frame_field_flag", T::when(simple.clone().both(frame_mbs_only.equal_to(E::lit(0))), bits(1))),
            ("direct_8x8_inference_flag", T::when(simple.clone(), bits(1))),
            ("frame_cropping_flag", T::when(simple.clone(), bits(1))),
            ("frame_crop_left_offset", T::when(cropped.clone(), ue())),
            ("frame_crop_right_offset", T::when(cropped.clone(), ue())),
            ("frame_crop_top_offset", T::when(cropped.clone(), ue())),
            ("frame_crop_bottom_offset", T::when(cropped, ue())),
            ("vui_parameters_present_flag", T::when(simple.clone(), bits(1))),
            ("width", T::when(simple.clone(), T::computed(width))),
            ("height", T::when(simple, T::computed(height))),
        ],
    )
    .field_doc("level_idc", "The level times ten: 30 is level 3.0.")
    .field_doc("pic_width_in_mbs_minus1", "The picture's width in 16-pixel macroblocks, less one.")
    .field_doc("width", "The picture's width in pixels after cropping, calculated from pic_width_in_mbs_minus1 and the frame_crop offsets.")
    .field_doc("height", "The picture's height in pixels after cropping, calculated from pic_height_in_map_units_minus1, frame_mbs_only_flag and the frame_crop offsets.")
}

/// A PPS payload, read as far as its fixed fields go.
pub(super) fn pps_rbsp() -> T {
    let rest = "The rest of the PPS, unparsed: any High profile fields, then the stop bit.";
    T::switch(clean_for(8), vec![(1, then_the_rest("PictureParameterSetRbsp", pps(), rest))], T::bytes(E::Remaining))
}

fn pps() -> T {
    let plain = v("num_slice_groups_minus1").equal_to(E::lit(0));
    T::structure(
        "PictureParameterSet",
        vec![
            ("pic_parameter_set_id", ue()),
            ("seq_parameter_set_id", ue()),
            ("entropy_coding_mode_flag", T::enumeration("EntropyCoding", bits(1), &[(0, "CAVLC"), (1, "CABAC")])),
            ("bottom_field_pic_order_in_frame_present_flag", bits(1)),
            ("num_slice_groups_minus1", ue()),
            ("num_ref_idx_l0_default_active_minus1", T::when(plain.clone(), ue())),
            ("num_ref_idx_l1_default_active_minus1", T::when(plain.clone(), ue())),
            ("weighted_pred_flag", T::when(plain.clone(), bits(1))),
            ("weighted_bipred_idc", T::when(plain.clone(), bits(2))),
            ("pic_init_qp_minus26", T::when(plain.clone(), se())),
            ("pic_init_qs_minus26", T::when(plain.clone(), se())),
            ("chroma_qp_index_offset", T::when(plain.clone(), se())),
            ("deblocking_filter_control_present_flag", T::when(plain.clone(), bits(1))),
            ("constrained_intra_pred_flag", T::when(plain.clone(), bits(1))),
            ("redundant_pic_cnt_present_flag", T::when(plain, bits(1))),
        ],
    )
    .field_doc("num_slice_groups_minus1", "Number of slice groups, less one. Above 0 only with flexible macroblock ordering (baseline and extended profiles); the rest of the PPS is then left unparsed.")
}

const SLICE_TYPE: &[(i128, &str)] = &[
    (0, "P"),
    (1, "B"),
    (2, "I"),
    (3, "SP"),
    (4, "SI"),
    (5, "P (all)"),
    (6, "B (all)"),
    (7, "I (all)"),
    (8, "SP (all)"),
    (9, "SI (all)"),
];

/// A slice: the three fields every slice header starts with, then the rest
/// of the header and the slice data, which depend on the SPS and PPS.
pub(super) fn slice_rbsp() -> T {
    T::switch(
        clean_for(6),
        vec![(
            1,
            then_the_rest(
                "SliceRbsp",
                T::structure(
                    "SliceHeader",
                    vec![
                        ("first_mb_in_slice", ue()),
                        ("slice_type", ue_named("SliceType", SLICE_TYPE)),
                        ("pic_parameter_set_id", ue()),
                    ],
                )
                .named_by("slice_type")
                .field_doc("slice_type", "I: decoded on its own. P: predicted from earlier pictures. B: predicted from up to two pictures per block. 5 to 9: as 0 to 4, and every slice in the picture has the same type."),
                "The rest of the slice header and the slice data, CAVLC or CABAC coded.",
            ),
        )],
        T::bytes(E::Remaining),
    )
}
