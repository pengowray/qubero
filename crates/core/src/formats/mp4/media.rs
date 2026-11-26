//! What one sample holds, by the format of the track it is in.
//!
//! The sample tables and the track runs place each sample; this reads it.
//! An H.264 or HEVC sample is NAL units, each with a length in front of it
//! that is as many bytes as the decoder configuration says. An AV1 sample is
//! OBUs. Uncompressed audio is the numbers themselves. Everything else, AAC
//! frames included, is the bytes of one coded frame, which only a decoder
//! takes further.
//!
//! The structure placing a sample has a `format` field, and a
//! `nal_length_size` where the format is NAL units, which is what this reads.

use super::{bits, golomb, u16be, u32be};
use crate::template::{Endian, Endian::*, Expr as E, Ty as T, Until};

const fn code(s: &[u8; 4]) -> i128 {
    ((s[0] as i128) << 24) | ((s[1] as i128) << 16) | ((s[2] as i128) << 8) | s[3] as i128
}

/// The sample entry formats that are named when a track's format is shown.
pub(super) const FORMATS: &[(i128, &str)] = &[
    (code(b"avc1"), "avc1 (H.264)"),
    (code(b"avc3"), "avc3 (H.264)"),
    (code(b"hvc1"), "hvc1 (HEVC)"),
    (code(b"hev1"), "hev1 (HEVC)"),
    (code(b"dvh1"), "dvh1 (Dolby Vision HEVC)"),
    (code(b"dvhe"), "dvhe (Dolby Vision HEVC)"),
    (code(b"av01"), "av01 (AV1)"),
    (code(b"vp08"), "vp08 (VP8)"),
    (code(b"vp09"), "vp09 (VP9)"),
    (code(b"mp4v"), "mp4v (MPEG-4 Visual)"),
    (code(b"jpeg"), "jpeg (Motion JPEG)"),
    (code(b"mjpa"), "mjpa (Motion JPEG)"),
    (code(b"apcn"), "apcn (ProRes 422)"),
    (code(b"apch"), "apch (ProRes 422 HQ)"),
    (code(b"apcs"), "apcs (ProRes 422 LT)"),
    (code(b"apco"), "apco (ProRes 422 Proxy)"),
    (code(b"ap4h"), "ap4h (ProRes 4444)"),
    (code(b"mp4a"), "mp4a (MPEG-4 audio)"),
    (code(b"alac"), "alac (Apple Lossless)"),
    (code(b"Opus"), "Opus"),
    (code(b"fLaC"), "fLaC (FLAC)"),
    (code(b"ac-3"), "ac-3 (Dolby Digital)"),
    (code(b"ec-3"), "ec-3 (Dolby Digital Plus)"),
    (code(b"twos"), "twos (big-endian PCM)"),
    (code(b"sowt"), "sowt (little-endian PCM)"),
    (code(b"raw "), "raw (unsigned 8-bit PCM)"),
    (code(b"in24"), "in24 (24-bit PCM)"),
    (code(b"in32"), "in32 (32-bit PCM)"),
    (code(b"fl32"), "fl32 (32-bit float PCM)"),
    (code(b"fl64"), "fl64 (64-bit float PCM)"),
    (code(b"lpcm"), "lpcm (QuickTime PCM)"),
    (code(b"ipcm"), "ipcm (ISO PCM)"),
    (code(b"ulaw"), "ulaw (μ-law)"),
    (code(b"alaw"), "alaw (A-law)"),
    (code(b"tx3g"), "tx3g (timed text)"),
    (code(b"text"), "text (QuickTime text)"),
    (code(b"wvtt"), "wvtt (WebVTT)"),
    (code(b"stpp"), "stpp (TTML)"),
    (code(b"tmcd"), "tmcd (timecode)"),
    (code(b"mebx"), "mebx (timed metadata)"),
    (code(b"encv"), "encv (encrypted video)"),
    (code(b"enca"), "enca (encrypted audio)"),
];

fn any_of(e: E, codes: &[&[u8; 4]]) -> E {
    codes.iter().map(|c| e.clone().equal_to(E::lit(code(c)))).reduce(|a, b| a.either(b)).unwrap_or(E::lit(0))
}

const H264: &[&[u8; 4]] = &[b"avc1", b"avc2", b"avc3", b"avc4"];
const HEVC: &[&[u8; 4]] = &[b"hvc1", b"hev1", b"dvh1", b"dvhe"];
const BRAW: &[&[u8; 4]] = &[b"brlt", b"brxq", b"brst", b"brvm", b"brhq", b"brvl", b"brvn", b"brvo"];
const PCM: &[&[u8; 4]] = &[b"twos", b"sowt", b"raw ", b"in24", b"in32", b"fl32", b"fl64"];

/// Whether a track of this format writes its samples as length-prefixed NAL
/// units.
pub(super) fn is_nal_format(format: E) -> E {
    any_of(format.clone(), H264).either(any_of(format, HEVC))
}

/// Whether a track of this format is uncompressed audio read here.
pub(super) fn is_pcm_format(format: E) -> E {
    any_of(format, PCM)
}

/// What a sample of the track holds.
pub(super) fn sample_content() -> T {
    let format = E::field("format");
    let mut cases: Vec<(i128, T)> = Vec::new();
    for c in H264 {
        cases.push((code(c), T::Named("H264Sample".into())));
    }
    for c in HEVC {
        cases.push((code(c), T::Named("HevcSample".into())));
    }
    cases.extend(vec![
        (code(b"av01"), obus()),
        (code(b"twos"), pcm(Big)),
        (code(b"sowt"), pcm(Little)),
        (code(b"raw "), T::array(T::u8(), E::Remaining)),
        (code(b"in24"), T::array(T::Int { bits: 24, endian: Big }, E::Remaining.div(E::lit(3)))),
        (code(b"in32"), T::array(T::Int { bits: 32, endian: Big }, E::Remaining.div(E::lit(4)))),
        (code(b"fl32"), T::array(T::F32(Big), E::Remaining.div(E::lit(4)))),
        (code(b"fl64"), T::array(T::F64(Big), E::Remaining.div(E::lit(8)))),
        (code(b"tx3g"), text_sample()),
        (code(b"text"), text_sample()),
        (code(b"tmcd"), T::structure("TimecodeSample", vec![("frame_number", u32be())])),
    ]);
    for c in BRAW {
        cases.push((code(c), T::Named("BrawFrame".into())));
    }
    T::switch(format, cases, coded_frame())
}

/// A sample read no further: one coded frame. A structure of its own, so
/// that each sample is an entry of its own in the hex view's column rather
/// than part of one run of bytes covering a whole chunk.
fn coded_frame() -> T {
    T::structure("CodedFrame", vec![("frame", T::bytes(E::Remaining))])
        .field_doc("frame", "One coded frame, as the encoder wrote it. Only a decoder for the track's format reads it further.")
}

/// QuickTime's `twos` and `sowt`: signed integers, 16 bits unless the
/// sample is a byte per channel. One sample is one frame, a number for each
/// channel.
fn pcm(endian: Endian) -> T {
    let per_channel = E::Remaining.div(E::field("channel_count").at_least(E::lit(1)));
    T::switch(
        per_channel,
        vec![(1, T::array(T::Int { bits: 8, endian }, E::Remaining))],
        T::array(T::Int { bits: 16, endian }, E::Remaining.div(E::lit(2))),
    )
}

/// A timed text sample: the text, then any style boxes.
fn text_sample() -> T {
    T::structure(
        "TextSample",
        vec![("length", u16be()), ("text", T::utf8(E::field("length").at_most(E::Remaining))), ("modifiers", T::bytes(E::Remaining))],
    )
    .named_by("text")
}

/// An H.264 sample: NAL units, each with its length in front.
pub(super) fn h264_sample() -> T {
    length_prefixed(T::Named("H264NalUnit".into()))
}

/// An HEVC sample: the same, with HEVC's two-byte NAL unit header.
pub(super) fn hevc_sample() -> T {
    length_prefixed(T::Named("HevcNalUnit".into()))
}

/// NAL units one after another, each with its length in front, or bytes
/// where the decoder configuration gives no length size that can be.
fn length_prefixed(unit: T) -> T {
    let length = E::field("nal_length_size");
    let usable = length.clone().equal_to(E::lit(1)).either(length.clone().equal_to(E::lit(2))).either(length.equal_to(E::lit(4)));
    T::switch(usable, vec![(1, nal_units(unit))], T::bytes(E::Remaining))
}

/// NAL units one after another, each with its length in front.
fn nal_units(unit: T) -> T {
    T::repeat(
        T::structure(
            "LengthPrefixedNalUnit",
            vec![
                ("length", T::uint_expr(E::field("nal_length_size").mul(E::lit(8)), Big)),
                ("nal", T::sized(E::field("length").at_most(E::Remaining), unit)),
            ],
        )
        .named_by("nal")
        .field_doc("length", "The size in bytes of the NAL unit that follows."),
        Until::End,
    )
}

/// NAL unit types from the H.264 spec, table 7-1.
pub(super) const NAL_TYPE: &[(i128, &str)] = &[
    (1, "non-IDR slice"),
    (2, "partition A"),
    (3, "partition B"),
    (4, "partition C"),
    (5, "IDR slice"),
    (6, "SEI"),
    (7, "SPS"),
    (8, "PPS"),
    (9, "access unit delimiter"),
    (10, "end of sequence"),
    (11, "end of stream"),
    (12, "filler"),
    (13, "SPS extension"),
    (14, "prefix NAL unit"),
    (15, "subset SPS"),
    (19, "auxiliary slice"),
    (20, "slice extension"),
];

/// One H.264 NAL unit: a byte of header bits, then the payload. SEI messages
/// are read, and so is the exp-Golomb coded start of an SPS, a PPS and a
/// slice (see [`golomb`]); the rest stays bytes.
pub(super) fn h264_nal_unit() -> T {
    T::structure(
        "NalUnit",
        vec![
            ("forbidden_zero", bits(1)),
            ("ref_idc", bits(2)),
            ("unit_type", T::enumeration("NalType", bits(5), NAL_TYPE)),
            (
                "rbsp",
                T::switch(
                    E::field("unit_type"),
                    vec![
                        (1, golomb::slice_rbsp()),
                        (5, golomb::slice_rbsp()),
                        (6, sei_messages()),
                        (7, golomb::sps_rbsp()),
                        (8, golomb::pps_rbsp()),
                    ],
                    T::bytes(E::Remaining),
                ),
            ),
        ],
    )
    .named_by("unit_type")
    .field_doc("ref_idc", "0 when no other picture is predicted from this one, so a decoder may drop it.")
}

/// HEVC NAL unit types, from table 7-1 of H.265.
const HEVC_NAL_TYPE: &[(i128, &str)] = &[
    (0, "TRAIL_N slice"),
    (1, "TRAIL_R slice"),
    (2, "TSA_N slice"),
    (3, "TSA_R slice"),
    (4, "STSA_N slice"),
    (5, "STSA_R slice"),
    (6, "RADL_N slice"),
    (7, "RADL_R slice"),
    (8, "RASL_N slice"),
    (9, "RASL_R slice"),
    (16, "BLA_W_LP slice"),
    (17, "BLA_W_RADL slice"),
    (18, "BLA_N_LP slice"),
    (19, "IDR_W_RADL slice"),
    (20, "IDR_N_LP slice"),
    (21, "CRA slice"),
    (32, "VPS"),
    (33, "SPS"),
    (34, "PPS"),
    (35, "access unit delimiter"),
    (36, "end of sequence"),
    (37, "end of bitstream"),
    (38, "filler"),
    (39, "prefix SEI"),
    (40, "suffix SEI"),
];

/// One HEVC NAL unit: two bytes of header bits, then the payload.
pub(super) fn hevc_nal_unit() -> T {
    T::structure(
        "HevcNalUnit",
        vec![
            ("forbidden_zero", bits(1)),
            ("unit_type", T::enumeration("HevcNalType", bits(6), HEVC_NAL_TYPE)),
            ("layer_id", bits(6)),
            ("temporal_id_plus1", bits(3)),
            ("rbsp", T::switch(E::field("unit_type").equal_to(E::lit(39)).either(E::field("unit_type").equal_to(E::lit(40))), vec![(1, sei_messages())], T::bytes(E::Remaining))),
        ],
    )
    .named_by("unit_type")
}

/// SEI payload types that are named, from Annex D of H.264 and H.265.
const SEI_TYPE: &[(i128, &str)] = &[
    (0, "buffering period"),
    (1, "picture timing"),
    (2, "pan-scan rectangle"),
    (3, "filler payload"),
    (4, "user data registered (ITU-T T.35)"),
    (5, "user data unregistered"),
    (6, "recovery point"),
    (19, "film grain characteristics"),
    (45, "frame packing arrangement"),
    (47, "display orientation"),
    (128, "structure of pictures"),
    (129, "active parameter sets"),
    (132, "decoded picture hash"),
    (137, "mastering display colour volume"),
    (144, "content light level"),
    (147, "alternative transfer characteristics"),
];

/// A number written as 0xff bytes, each worth 255, and a last byte that is
/// added to them: how an SEI message writes its type and its size.
fn ff_coded(name: &str) -> T {
    T::structure(
        name,
        vec![
            ("ff_bytes", T::repeat(T::u8(), Until::While(E::peek(8, Big).equal_to(E::lit(0xff))))),
            ("last_byte", T::u8()),
            ("value", T::computed(E::len_of("ff_bytes").mul(E::lit(255)).add(E::field("last_byte")))),
        ],
    )
    .named_by("value")
}

/// The SEI messages of an SEI NAL unit, then the stop bit.
fn sei_messages() -> T {
    let message = T::structure(
        "SeiMessage",
        vec![
            ("type_code", ff_coded("SeiPayloadType")),
            ("payload_type", T::enumeration("SeiType", T::computed(E::within(&["type_code", "value"])), SEI_TYPE)),
            ("size_code", ff_coded("SeiPayloadSize")),
            (
                "payload",
                T::sized(
                    E::within(&["size_code", "value"]).at_most(E::Remaining),
                    T::switch(
                        E::within(&["type_code", "value"]),
                        vec![(
                            5,
                            T::structure(
                                "UnregisteredUserData",
                                vec![("uuid", T::bytes(E::lit(16))), ("text", T::utf8_padded(E::Remaining, 0))],
                            )
                            .field_doc("uuid", "A UUID for whoever wrote this SEI message. x264 writes its version and settings as text after the UUID."),
                        )],
                        T::bytes(E::Remaining),
                    ),
                ),
            ),
        ],
    )
    .named_by("payload_type")
    .machinery(&["type_code", "size_code"]);
    T::structure(
        "SeiRbsp",
        vec![
            ("messages", T::repeat(message, Until::While(E::Remaining.greater_than(E::lit(1))))),
            ("trailing_bits", T::u8()),
        ],
    )
    .field_doc("trailing_bits", "The stop bit and the zero bits after it, 0x80.")
}

/// AV1 OBU types, from section 6.2.2 of the AV1 specification.
const OBU_TYPE: &[(i128, &str)] = &[
    (1, "sequence header"),
    (2, "temporal delimiter"),
    (3, "frame header"),
    (4, "tile group"),
    (5, "metadata"),
    (6, "frame"),
    (7, "redundant frame header"),
    (8, "tile list"),
    (15, "padding"),
];

/// AV1 OBUs one after another. In MP4 each has its size written.
pub(super) fn obus() -> T {
    T::repeat(
        T::structure(
            "Obu",
            vec![
                ("forbidden_bit", bits(1)),
                ("obu_type", T::enumeration("ObuType", bits(4), OBU_TYPE)),
                ("extension_flag", bits(1)),
                ("has_size_field", bits(1)),
                ("reserved_1bit", bits(1)),
                ("temporal_id", T::when(E::field("extension_flag"), bits(3))),
                ("spatial_id", T::when(E::field("extension_flag"), bits(2))),
                ("extension_reserved", T::when(E::field("extension_flag"), bits(3))),
                ("obu_size", T::when(E::field("has_size_field"), T::leb_u())),
                ("payload", T::bytes(E::cond(E::field("has_size_field"), E::field("obu_size").at_most(E::Remaining), E::Remaining))),
            ],
        )
        .named_by("obu_type"),
        Until::End,
    )
}
