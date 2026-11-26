//! Sample descriptions, `stsd`: what format a track's samples are in, and the
//! settings a decoder needs before the first of them.
//!
//! Each entry starts the same way, size, format, six reserved bytes and a data
//! reference index, and then has a layout chosen by the kind of media: a video
//! entry is 78 bytes of picture settings, an audio entry 20 bytes of sound
//! settings (QuickTime adds 16 or 36 more in versions 1 and 2), and after
//! either come boxes of the codec's own, such as `avcC` for H.264 or `esds`
//! for AAC.

use super::{golomb, media};
use super::{bits, cc, child_boxes, fourcc, full_box, full_box_flags, i16be, signed, u16be, u32be};
use crate::template::{Endian::*, Expr as E, Ty as T, Until};

/// The codec configuration boxes found inside sample entries, by type.
pub(super) fn boxes() -> Vec<(&'static str, T)> {
    vec![
        ("avcC", avcc()),
        ("hvcC", hvcc()),
        ("av1C", av1c()),
        ("vpcC", vpcc()),
        ("pasp", pasp()),
        ("btrt", btrt()),
        ("colr", colr()),
        ("clap", clap()),
        ("fiel", T::structure("FieldHandling", vec![("field_count", T::u8()), ("field_ordering", T::u8())])),
        ("gama", T::structure("Gamma", vec![("gamma", T::fixed(32, 16, Big))])),
        ("chan", chan()),
        ("esds", esds()),
        ("dOps", dops()),
        ("dfLa", dfla()),
        ("alac", alac()),
        ("dac3", dac3()),
        ("pcmC", pcmc()),
        ("srat", T::structure("SamplingRate", { let mut f = full_box(); f.push(("sampling_rate", u32be())); f })),
        ("schm", schm()),
        ("tenc", tenc()),
        ("pssh", pssh()),
    ]
}

pub(super) fn stsd() -> T {
    let mut fields = full_box();
    fields.extend(vec![("entry_count", u32be()), ("entries", T::array(sample_entry(), E::field("entry_count")))]);
    T::structure("SampleDescription", fields)
}

const VISUAL: &[&str] = &[
    "avc1", "avc2", "avc3", "avc4", "hvc1", "hev1", "dvh1", "dvhe", "dvav", "dva1", "av01", "vp08", "vp09", "mp4v", "s263", "jpeg",
    "mjpa", "mjpb", "mjp2", "png ", "apcn", "apch", "apcs", "apco", "ap4h", "ap4x", "encv", "rle ", "dvc ", "dvcp", "dv5n",
    "dv5p", "yuv2", "2vuy", "v210", "brlt", "brxq", "brst", "brvm", "brhq", "brvl", "brvn", "brvo",
];

const AUDIO: &[&str] = &[
    "mp4a", "alac", "Opus", "fLaC", "ac-3", "ec-3", "ac-4", "twos", "sowt", "lpcm", "ipcm", "fpcm", "in24", "in32", "fl32", "fl64",
    "ulaw", "alaw", "ima4", "samr", "sawb", "enca", "mp3 ", ".mp3", "dtsc", "dtsh", "dtsl", "dtse", "mlpa",
];

fn sample_entry() -> T {
    let rest = E::field("size").sub(E::lit(16)).at_least(E::lit(0)).at_most(E::Remaining);
    let mut cases: Vec<(i128, T)> = Vec::new();
    for f in VISUAL {
        cases.push((cc(f), T::Named("VisualSampleEntry".into())));
    }
    for f in AUDIO {
        cases.push((cc(f), T::Named("AudioSampleEntry".into())));
    }
    cases.extend(vec![
        (cc("tx3g"), timed_text_entry()),
        (cc("tmcd"), timecode_entry()),
        (cc("mebx"), T::structure("TimedMetadataEntry", vec![("boxes", child_boxes())])),
        (cc("wvtt"), T::structure("WebVttEntry", vec![("boxes", child_boxes())])),
    ]);
    // A format not listed is laid out by the kind of track it is in: the
    // handler in this track's mdia says vide or soun.
    let handler = E::sibling_tagged(&["type"], E::lit(cc("hdlr")), &["body", "handler_type"]);
    let by_handler = T::switch(
        handler,
        vec![(cc("vide"), T::Named("VisualSampleEntry".into())), (cc("soun"), T::Named("AudioSampleEntry".into()))],
        T::bytes(E::Remaining),
    );
    T::structure_named(
        "SampleEntry",
        "format",
        "body",
        vec![
            ("size", u32be()),
            ("format", fourcc()),
            ("reserved", T::array(T::u8(), E::lit(6))),
            ("data_reference_index", u16be()),
            ("body", T::sized(rest, T::switch(E::field("format"), cases, by_handler))),
        ],
    )
    .field_doc("format", "The codec: avc1 for H.264, hvc1 or hev1 for HEVC, av01 for AV1, mp4a for AAC and other MPEG-4 audio, twos or sowt for uncompressed audio.")
    .field_doc("data_reference_index", "Which entry of dref says where the samples are, counting from 1.")
}

/// The 78 bytes every video sample entry has before its own boxes.
///
/// ISO 14496-12 calls the first 16 of them pre_defined and reserved, and
/// requires zero. QuickTime keeps a version, a vendor and two quality
/// settings there, and those are the names used.
pub(super) fn visual_sample_entry() -> T {
    T::structure(
        "VisualSampleEntry",
        vec![
            ("version", u16be()),
            ("revision_level", u16be()),
            ("vendor", fourcc()),
            ("temporal_quality", u32be()),
            ("spatial_quality", u32be()),
            ("width", u16be()),
            ("height", u16be()),
            ("horiz_resolution", T::fixed(32, 16, Big)),
            ("vert_resolution", T::fixed(32, 16, Big)),
            ("data_size", u32be()),
            ("frame_count", u16be()),
            // 32 bytes: a length byte, then that many characters, then padding.
            (
                "compressor_name",
                T::structure("CompressorName", vec![("length", T::u8()), ("text", T::utf8_padded(E::lit(31), 0))]).named_by("text"),
            ),
            ("depth", u16be()),
            ("color_table_id", i16be()),
            ("boxes", child_boxes()),
        ],
    )
    .field_doc("vendor", "QuickTime: who made the codec, such as appl or FFMP. Zero in an MP4.")
    .field_doc("horiz_resolution", "Pixels per inch, 72 by convention.")
    .field_doc("data_size", "Reserved, zero.")
    .field_doc("frame_count", "How many frames each sample holds, 1 for every common codec.")
    .field_doc("depth", "Bits per pixel: 24 for colour with no alpha.")
    .field_doc("color_table_id", "-1: no colour table.")
}

/// The sound settings every audio sample entry has before its own boxes:
/// QuickTime's sound description, of which an MP4 uses version 0.
pub(super) fn audio_sample_entry() -> T {
    let version = E::field("version");
    T::structure(
        "AudioSampleEntry",
        vec![
            ("version", u16be()),
            ("revision_level", u16be()),
            ("vendor", fourcc()),
            ("channel_count", u16be()),
            ("sample_size", u16be()),
            ("compression_id", i16be()),
            ("packet_size", u16be()),
            ("sample_rate", T::fixed(32, 16, Big)),
            ("v1", T::when(version.clone().equal_to(E::lit(1)), sound_v1())),
            ("v2", T::when(version.equal_to(E::lit(2)), sound_v2())),
            ("boxes", child_boxes()),
        ],
    )
    .field_doc("version", "QuickTime sound description version: 0, 1 or 2. Versions 1 and 2 add fields after sample_rate. ISO 14496-12 calls this entry_version.")
    .field_doc("vendor", "QuickTime: who made the codec. ISO 14496-12 reserves these four bytes and requires zero.")
    .field_doc("sample_size", "Bits per sample per channel, for uncompressed audio. 16 for anything compressed.")
    .field_doc("compression_id", "QuickTime: -2 in a version 2 entry, 0 for uncompressed, -1 for a compressed version 1 entry.")
    .field_doc("sample_rate", "Samples per second. In a version 2 entry this is 1.0, and the rate is in audio_sample_rate.")
}

fn sound_v1() -> T {
    T::structure(
        "SoundDescriptionV1",
        vec![
            ("samples_per_packet", u32be()),
            ("bytes_per_packet", u32be()),
            ("bytes_per_frame", u32be()),
            ("bytes_per_sample", u32be()),
        ],
    )
}

fn sound_v2() -> T {
    T::structure(
        "SoundDescriptionV2",
        vec![
            ("size_of_struct_only", u32be()),
            ("audio_sample_rate", T::F64(Big)),
            ("audio_channels", u32be()),
            ("always_7f000000", u32be()),
            ("const_bits_per_channel", u32be()),
            (
                "format_specific_flags",
                T::flags(
                    "LpcmFlags",
                    u32be(),
                    &[(0, "float"), (1, "big-endian"), (2, "signed integer"), (3, "packed"), (4, "aligned high"), (5, "non-interleaved")],
                ),
            ),
            ("const_bytes_per_audio_packet", u32be()),
            ("const_lpcm_frames_per_audio_packet", u32be()),
        ],
    )
}

fn timed_text_entry() -> T {
    let rgba = || T::array(T::u8(), E::lit(4));
    T::structure(
        "TimedTextEntry",
        vec![
            ("display_flags", u32be()),
            ("horizontal_justification", T::Int { bits: 8, endian: Big }),
            ("vertical_justification", T::Int { bits: 8, endian: Big }),
            ("background_color", rgba()),
            ("text_box", T::structure("TextBox", vec![("top", i16be()), ("left", i16be()), ("bottom", i16be()), ("right", i16be())])),
            (
                "default_style",
                T::structure(
                    "StyleRecord",
                    vec![
                        ("start_char", u16be()),
                        ("end_char", u16be()),
                        ("font_id", u16be()),
                        ("face_style_flags", T::flags("FaceStyle", T::u8(), &[(0, "bold"), (1, "italic"), (2, "underline")])),
                        ("font_size", T::u8()),
                        ("text_color", rgba()),
                    ],
                ),
            ),
            ("boxes", child_boxes()),
        ],
    )
}

fn timecode_entry() -> T {
    T::structure(
        "TimecodeEntry",
        vec![
            ("reserved", u32be()),
            ("flags", T::flags("TimecodeFlags", u32be(), &[(0, "drop frame"), (1, "24 hour max"), (2, "negative times OK"), (3, "counter")])),
            ("timescale", u32be()),
            ("frame_duration", u32be()),
            ("number_of_frames", T::u8()),
            ("reserved2", T::u8()),
            ("boxes", child_boxes()),
        ],
    )
    .field_doc("number_of_frames", "Frames per second, rounded: 30 for 29.97.")
}

/// AVCDecoderConfigurationRecord: what a decoder needs before the first frame.
fn avcc() -> T {
    let param_set = T::structure(
        "ParameterSet",
        vec![("length", u16be()), ("nal", T::sized(E::field("length").at_most(E::Remaining), T::Named("H264NalUnit".into())))],
    )
    .named_by("nal");
    let profile = E::field("profile");
    let high = [100, 110, 122, 144].iter().map(|p| profile.clone().equal_to(E::lit(*p))).reduce(|a, b| a.either(b)).unwrap();
    let extension = T::structure(
        "AvcHighProfileExtension",
        vec![
            ("reserved", bits(6)),
            ("chroma_format", T::enumeration("ChromaFormat", bits(2), CHROMA_FORMAT)),
            ("reserved2", bits(5)),
            ("bit_depth_luma_minus8", bits(3)),
            ("reserved3", bits(5)),
            ("bit_depth_chroma_minus8", bits(3)),
            ("sps_ext_count", T::u8()),
            ("sps_ext", T::array(param_set.clone(), E::field("sps_ext_count"))),
        ],
    );
    T::structure(
        "AVCConfig",
        vec![
            ("configuration_version", T::u8()),
            ("profile", T::enumeration("H264Profile", T::u8(), golomb::PROFILE)),
            ("profile_compatibility", T::u8()),
            ("level", T::u8()),
            ("reserved", bits(6)),
            ("length_size_minus_one", bits(2)),
            ("reserved2", bits(3)),
            ("sps_count", bits(5)),
            ("sps", T::array(param_set.clone(), E::field("sps_count"))),
            ("pps_count", T::u8()),
            ("pps", T::array(param_set, E::field("pps_count"))),
            ("high_profile", T::when(high.both(E::Remaining.greater_or_equal(E::lit(4))), extension)),
        ],
    )
    .field_doc("level", "The level times ten: 30 is level 3.0, 41 is level 4.1.")
    .field_doc("length_size_minus_one", "The size in bytes of the length field before each NAL unit in a sample, less one: 3 means four-byte lengths.")
    .field_doc("high_profile", "Chroma format, bit depths and SPS extensions. Only for High profiles, and optional even then.")
}

const CHROMA_FORMAT: &[(i128, &str)] = &[(0, "monochrome"), (1, "4:2:0"), (2, "4:2:2"), (3, "4:4:4")];

/// HEVCDecoderConfigurationRecord, ISO/IEC 14496-15 section 8.3.3.
fn hvcc() -> T {
    let nal = T::structure(
        "HevcParameterSet",
        vec![("length", u16be()), ("nal", T::sized(E::field("length").at_most(E::Remaining), T::Named("HevcNalUnit".into())))],
    )
    .named_by("nal");
    let array = T::structure(
        "HevcNalArray",
        vec![
            ("array_completeness", bits(1)),
            ("reserved", bits(1)),
            ("nal_unit_type", bits(6)),
            ("nalu_count", u16be()),
            ("nalus", T::array(nal, E::field("nalu_count"))),
        ],
    );
    T::structure(
        "HEVCConfig",
        vec![
            ("configuration_version", T::u8()),
            ("general_profile_space", bits(2)),
            ("general_tier_flag", bits(1)),
            ("general_profile_idc", T::enumeration("HevcProfile", bits(5), &[(1, "Main"), (2, "Main 10"), (3, "Main Still Picture"), (4, "range extensions")])),
            ("general_profile_compatibility_flags", u32be()),
            ("general_constraint_indicator_flags", T::UInt { bits: 48, endian: Big }),
            ("general_level_idc", T::u8()),
            ("reserved", bits(4)),
            ("min_spatial_segmentation_idc", bits(12)),
            ("reserved2", bits(6)),
            ("parallelism_type", bits(2)),
            ("reserved3", bits(6)),
            ("chroma_format", T::enumeration("ChromaFormat", bits(2), CHROMA_FORMAT)),
            ("reserved4", bits(5)),
            ("bit_depth_luma_minus8", bits(3)),
            ("reserved5", bits(5)),
            ("bit_depth_chroma_minus8", bits(3)),
            ("avg_frame_rate", u16be()),
            ("constant_frame_rate", bits(2)),
            ("num_temporal_layers", bits(3)),
            ("temporal_id_nested", bits(1)),
            ("length_size_minus_one", bits(2)),
            ("num_of_arrays", T::u8()),
            ("arrays", T::array(array, E::field("num_of_arrays"))),
        ],
    )
    .field_doc("general_level_idc", "The level times 30: 93 is level 3.1, 120 is level 4.")
    .field_doc("avg_frame_rate", "Frames per 256 seconds, or 0 when not given.")
    .field_doc("length_size_minus_one", "The size in bytes of the length field before each NAL unit in a sample, less one: 3 means four-byte lengths.")
}

/// AV1CodecConfigurationRecord, from the AV1 in ISO BMFF binding.
fn av1c() -> T {
    T::structure(
        "AV1Config",
        vec![
            ("marker", bits(1)),
            ("version", bits(7)),
            ("seq_profile", T::enumeration("Av1Profile", bits(3), &[(0, "Main"), (1, "High"), (2, "Professional")])),
            ("seq_level_idx_0", bits(5)),
            ("seq_tier_0", bits(1)),
            ("high_bitdepth", bits(1)),
            ("twelve_bit", bits(1)),
            ("monochrome", bits(1)),
            ("chroma_subsampling_x", bits(1)),
            ("chroma_subsampling_y", bits(1)),
            ("chroma_sample_position", bits(2)),
            ("reserved", bits(3)),
            ("initial_presentation_delay_present", bits(1)),
            ("initial_presentation_delay_minus_one", bits(4)),
            ("config_obus", media::obus()),
        ],
    )
}

fn vpcc() -> T {
    let mut fields = full_box();
    fields.extend(vec![
        ("profile", T::u8()),
        ("level", T::u8()),
        ("bit_depth", bits(4)),
        ("chroma_subsampling", bits(3)),
        ("video_full_range_flag", bits(1)),
        ("colour_primaries", T::enumeration("ColourPrimaries", T::u8(), PRIMARIES)),
        ("transfer_characteristics", T::enumeration("TransferCharacteristics", T::u8(), TRANSFER)),
        ("matrix_coefficients", T::enumeration("MatrixCoefficients", T::u8(), MATRIX)),
        ("codec_initialization_data_size", u16be()),
        ("codec_initialization_data", T::bytes(E::field("codec_initialization_data_size").at_most(E::Remaining))),
    ]);
    T::structure("VPCodecConfig", fields)
}

fn pasp() -> T {
    T::structure("PixelAspectRatio", vec![("h_spacing", u32be()), ("v_spacing", u32be())])
        .doc("Pixel aspect ratio, h_spacing to v_spacing. 1 to 1 is square.")
}

fn btrt() -> T {
    T::structure("BitRate", vec![("buffer_size_db", u32be()), ("max_bitrate", u32be()), ("avg_bitrate", u32be())])
        .field_doc("buffer_size_db", "Decoding buffer size in bytes.")
        .field_doc("max_bitrate", "The highest bit rate in any one-second window, in bits per second.")
}

/// Colour primaries, from ITU-T H.273 table 2.
const PRIMARIES: &[(i128, &str)] = &[
    (1, "BT.709"),
    (2, "unspecified"),
    (4, "BT.470 M"),
    (5, "BT.470 BG (PAL)"),
    (6, "SMPTE 170M (NTSC)"),
    (7, "SMPTE 240M"),
    (8, "generic film"),
    (9, "BT.2020"),
    (10, "SMPTE ST 428-1 (XYZ)"),
    (11, "SMPTE RP 431-2 (DCI-P3)"),
    (12, "SMPTE EG 432-1 (Display P3)"),
    (22, "EBU Tech 3213-E"),
];

/// Transfer characteristics, from ITU-T H.273 table 3.
const TRANSFER: &[(i128, &str)] = &[
    (1, "BT.709"),
    (2, "unspecified"),
    (4, "gamma 2.2"),
    (5, "gamma 2.8"),
    (6, "SMPTE 170M"),
    (7, "SMPTE 240M"),
    (8, "linear"),
    (9, "log 100:1"),
    (10, "log 316:1"),
    (11, "IEC 61966-2-4"),
    (12, "BT.1361"),
    (13, "sRGB"),
    (14, "BT.2020 10-bit"),
    (15, "BT.2020 12-bit"),
    (16, "PQ (SMPTE ST 2084)"),
    (17, "SMPTE ST 428-1"),
    (18, "HLG (ARIB STD-B67)"),
];

/// Matrix coefficients, from ITU-T H.273 table 4.
const MATRIX: &[(i128, &str)] = &[
    (0, "identity (RGB)"),
    (1, "BT.709"),
    (2, "unspecified"),
    (4, "FCC"),
    (5, "BT.470 BG"),
    (6, "SMPTE 170M"),
    (7, "SMPTE 240M"),
    (8, "YCgCo"),
    (9, "BT.2020 non-constant luminance"),
    (10, "BT.2020 constant luminance"),
    (11, "SMPTE ST 2085"),
    (12, "chromaticity-derived non-constant"),
    (13, "chromaticity-derived constant"),
    (14, "ICtCp"),
];

fn colr() -> T {
    let codes = |full_range: bool| {
        let mut f = vec![
            ("colour_primaries", T::enumeration("ColourPrimaries", u16be(), PRIMARIES)),
            ("transfer_characteristics", T::enumeration("TransferCharacteristics", u16be(), TRANSFER)),
            ("matrix_coefficients", T::enumeration("MatrixCoefficients", u16be(), MATRIX)),
        ];
        if full_range {
            f.push(("full_range_flag", bits(1)));
            f.push(("reserved", bits(7)));
        }
        f
    };
    T::structure(
        "ColourInformation",
        vec![
            ("colour_type", fourcc()),
            (
                "colour",
                T::switch(
                    E::field("colour_type"),
                    vec![
                        (cc("nclx"), T::structure("ColourCodes", codes(true))),
                        (cc("nclc"), T::structure("QuickTimeColourCodes", codes(false))),
                    ],
                    T::bytes(E::Remaining),
                ),
            ),
        ],
    )
    .field_doc("colour_type", "nclx: three code points from ITU-T H.273. nclc: the same in QuickTime, without the range flag. prof or rICC: an ICC profile.")
}

fn clap() -> T {
    T::structure(
        "CleanAperture",
        vec![
            ("width_n", u32be()),
            ("width_d", u32be()),
            ("height_n", u32be()),
            ("height_d", u32be()),
            ("horiz_off_n", i32_be()),
            ("horiz_off_d", u32be()),
            ("vert_off_n", i32_be()),
            ("vert_off_d", u32be()),
        ],
    )
    .doc("The part of the picture to show, as fractions: width is width_n / width_d pixels. Height, and the offset of the part's centre from the picture's centre, are written the same way.")
}

fn i32_be() -> T {
    T::Int { bits: 32, endian: Big }
}

/// QuickTime's channel layout.
fn chan() -> T {
    let description = T::structure(
        "ChannelDescription",
        vec![("channel_label", u32be()), ("channel_flags", u32be()), ("coordinates", T::array(T::F32(Big), E::lit(3)))],
    );
    let mut fields = full_box();
    fields.extend(vec![
        ("channel_layout_tag", T::enumeration_hex("ChannelLayout", u32be(), CHANNEL_LAYOUTS)),
        ("channel_bitmap", u32be()),
        ("description_count", u32be()),
        ("descriptions", T::array(description, E::field("description_count"))),
    ]);
    T::structure("ChannelLayout", fields)
        .field_doc("channel_layout_tag", "A Core Audio layout tag, with the number of channels in the low 16 bits. 0: the channel descriptions list the channels. 0x10000: channel_bitmap lists them.")
}

const CHANNEL_LAYOUTS: &[(i128, &str)] = &[
    (0, "use channel descriptions"),
    (0x10000, "use channel bitmap"),
    ((100 << 16) | 1, "mono"),
    ((101 << 16) | 2, "stereo"),
    ((102 << 16) | 2, "stereo headphones"),
    ((103 << 16) | 2, "matrix stereo"),
    ((108 << 16) | 4, "quadraphonic"),
    ((113 << 16) | 3, "MPEG 3.0 A (L R C)"),
    ((117 << 16) | 5, "MPEG 5.0 A"),
    ((121 << 16) | 6, "MPEG 5.1 A (L R C LFE Ls Rs)"),
    ((126 << 16) | 8, "MPEG 7.1 C"),
];

/// The ES descriptor box: MPEG-4 systems descriptors, which is how an MP4
/// says what kind of MPEG-4 stream a track is and, for AAC, the
/// AudioSpecificConfig.
fn esds() -> T {
    let mut fields = full_box();
    fields.push(("descriptors", T::repeat(descriptor(Level::Top), Until::End)));
    T::structure("ElementaryStreamDescriptor", fields)
}

/// The initial object descriptor box: the profiles and levels the movie needs.
pub(super) fn iods() -> T {
    let mut fields = full_box();
    fields.push(("descriptors", T::repeat(descriptor(Level::Top), Until::End)));
    T::structure("InitialObjectDescriptor", fields)
}

/// How deep in the descriptor tree a descriptor is, which is all the decoder
/// specific info needs to know: inside a decoder config it is the codec's own
/// configuration, and the decoder config says which codec.
#[derive(Clone, Copy, PartialEq)]
enum Level {
    Top,
    InEs,
    InDecoderConfig,
}

const DESCRIPTOR_TAG: &[(i128, &str)] = &[
    (0x01, "object descriptor"),
    (0x02, "initial object descriptor"),
    (0x03, "ES descriptor"),
    (0x04, "decoder config descriptor"),
    (0x05, "decoder specific info"),
    (0x06, "SL config descriptor"),
    (0x0e, "ES ID inc"),
    (0x0f, "ES ID ref"),
    (0x10, "MP4 initial object descriptor"),
    (0x11, "MP4 object descriptor"),
];

/// One MPEG-4 descriptor: a tag, a size written seven bits a byte with the
/// top bit saying another byte follows, and the body.
fn descriptor(level: Level) -> T {
    let mut cases: Vec<(i128, T)> = vec![(0x06, sl_config()), (0x0e, T::structure("EsIdInc", vec![("track_id", u32be())]))];
    if level == Level::Top {
        cases.push((0x03, es_descriptor()));
        cases.push((0x10, initial_object_descriptor()));
        cases.push((0x02, initial_object_descriptor()));
    }
    if level == Level::InEs {
        cases.push((0x04, decoder_config()));
    }
    if level == Level::InDecoderConfig {
        cases.push((0x05, decoder_specific_info()));
    }
    T::structure(
        "Descriptor",
        vec![
            ("tag", T::enumeration_hex("DescriptorTag", T::u8(), DESCRIPTOR_TAG)),
            ("size", T::vlq()),
            ("body", T::sized(E::field("size").at_most(E::Remaining), T::switch(E::field("tag"), cases, T::bytes(E::Remaining)))),
        ],
    )
    .named_by("tag")
    .field_doc("size", "How many bytes the body has. The size is coded seven bits per byte, with the top bit set on every byte but the last.")
}

fn es_descriptor() -> T {
    T::structure(
        "EsDescriptor",
        vec![
            ("es_id", u16be()),
            ("stream_dependence_flag", bits(1)),
            ("url_flag", bits(1)),
            ("ocr_stream_flag", bits(1)),
            ("stream_priority", bits(5)),
            ("depends_on_es_id", T::when(E::field("stream_dependence_flag"), u16be())),
            ("url_length", T::when(E::field("url_flag"), T::u8())),
            ("url", T::when(E::field("url_flag"), T::utf8(E::field("url_length")))),
            ("ocr_es_id", T::when(E::field("ocr_stream_flag"), u16be())),
            ("descriptors", T::repeat(descriptor(Level::InEs), Until::End)),
        ],
    )
}

/// MPEG-4 object type indications, from the MP4 registration authority.
const OBJECT_TYPE: &[(i128, &str)] = &[
    (0x20, "MPEG-4 Visual"),
    (0x21, "H.264"),
    (0x23, "HEVC"),
    (0x40, "MPEG-4 Audio (AAC)"),
    (0x60, "MPEG-2 Visual Simple"),
    (0x61, "MPEG-2 Visual Main"),
    (0x66, "MPEG-2 AAC Main"),
    (0x67, "MPEG-2 AAC LC"),
    (0x68, "MPEG-2 AAC SSR"),
    (0x69, "MPEG-2 Audio (MP3)"),
    (0x6a, "MPEG-1 Visual"),
    (0x6b, "MPEG-1 Audio (MP3)"),
    (0x6c, "JPEG"),
    (0x6d, "PNG"),
    (0xa5, "AC-3"),
    (0xa6, "E-AC-3"),
    (0xa9, "DTS"),
    (0xad, "Opus"),
    (0xdd, "Vorbis"),
];

const STREAM_TYPE: &[(i128, &str)] = &[
    (1, "object descriptor"),
    (2, "clock reference"),
    (3, "scene description"),
    (4, "visual"),
    (5, "audio"),
    (6, "MPEG-7"),
    (7, "IPMP"),
    (8, "object content info"),
    (9, "MPEG-J"),
    (10, "interaction"),
    (11, "IPMP tool"),
];

fn decoder_config() -> T {
    T::structure(
        "DecoderConfigDescriptor",
        vec![
            ("object_type_indication", T::enumeration_hex("ObjectType", T::u8(), OBJECT_TYPE)),
            ("stream_type", T::enumeration("StreamType", bits(6), STREAM_TYPE)),
            ("up_stream", bits(1)),
            ("reserved", bits(1)),
            ("buffer_size_db", bits(24)),
            ("max_bitrate", u32be()),
            ("avg_bitrate", u32be()),
            ("descriptors", T::repeat(descriptor(Level::InDecoderConfig), Until::End)),
        ],
    )
    .field_doc("avg_bitrate", "Bits per second, or 0 when the bit rate varies.")
}

fn decoder_specific_info() -> T {
    let object = E::field("object_type_indication");
    let aac = [0x40, 0x66, 0x67, 0x68].iter().map(|o| object.clone().equal_to(E::lit(*o))).reduce(|a, b| a.either(b)).unwrap();
    let config = golomb::then_the_rest("AudioSpecificConfigBits", audio_specific_config(), "What follows the fields read here: settings for other object types, or padding.");
    T::switch(aac, vec![(1, config)], T::bytes(E::Remaining))
}

/// MPEG-4 audio object types, from ISO/IEC 14496-3 table 1.17.
const AUDIO_OBJECT_TYPE: &[(i128, &str)] = &[
    (1, "AAC Main"),
    (2, "AAC LC"),
    (3, "AAC SSR"),
    (4, "AAC LTP"),
    (5, "SBR (HE-AAC)"),
    (6, "AAC Scalable"),
    (7, "TwinVQ"),
    (8, "CELP"),
    (9, "HVXC"),
    (17, "ER AAC LC"),
    (19, "ER AAC LTP"),
    (20, "ER AAC Scalable"),
    (22, "ER BSAC"),
    (23, "ER AAC LD"),
    (29, "PS (HE-AAC v2)"),
    (31, "escape"),
    (32, "Layer-1"),
    (33, "Layer-2"),
    (34, "Layer-3 (MP3)"),
    (36, "ALS"),
    (39, "ER AAC ELD"),
    (42, "USAC (xHE-AAC)"),
];

const FREQUENCY_INDEX: &[(i128, &str)] = &[
    (0, "96000 Hz"),
    (1, "88200 Hz"),
    (2, "64000 Hz"),
    (3, "48000 Hz"),
    (4, "44100 Hz"),
    (5, "32000 Hz"),
    (6, "24000 Hz"),
    (7, "22050 Hz"),
    (8, "16000 Hz"),
    (9, "12000 Hz"),
    (10, "11025 Hz"),
    (11, "8000 Hz"),
    (12, "7350 Hz"),
    (15, "given in sampling_frequency"),
];

const CHANNEL_CONFIGURATION: &[(i128, &str)] = &[
    (0, "in the program config element"),
    (1, "mono"),
    (2, "stereo"),
    (3, "3.0 (C, L, R)"),
    (4, "4.0 (C, L, R, rear C)"),
    (5, "5.0"),
    (6, "5.1"),
    (7, "7.1"),
];

/// AudioSpecificConfig, ISO/IEC 14496-3 section 1.6.2.1: the bits an AAC
/// decoder starts from. Read as far as the general audio settings and the
/// signal for an SBR extension, which is what an HE-AAC file adds.
fn audio_specific_config() -> T {
    let aot = E::field("audio_object_type");
    let general = [1, 2, 3, 4, 6, 7, 17, 19, 20, 21, 22, 23].iter().map(|t| aot.clone().equal_to(E::lit(*t))).reduce(|a, b| a.either(b)).unwrap();
    let sync = E::Remaining.greater_or_equal(E::lit(2)).both(E::peek(11, Big).equal_to(E::lit(0x2b7)));
    // The SBR signal is its own structure, there or not as a whole, so that
    // nothing after it asks about a field the file did not write: a phone's
    // AAC config with no signal failed with `sync_extension_type is not in
    // this file`.
    let extension = T::structure(
        "SyncExtension",
        vec![
            ("sync_extension_type", bits(11)),
            ("extension_audio_object_type", T::enumeration("AudioObjectType", bits(5), AUDIO_OBJECT_TYPE)),
            ("sbr_present_flag", T::when(E::field("extension_audio_object_type").equal_to(E::lit(5)), bits(1))),
        ],
    )
    .field_doc("sync_extension_type", "0x2b7: an SBR extension follows. A decoder that only supports AAC LC can ignore it.")
    .field_doc("sbr_present_flag", "1: HE-AAC, with spectral band replication doubling the output sample rate.");
    T::structure(
        "AudioSpecificConfig",
        vec![
            ("audio_object_type", T::enumeration("AudioObjectType", bits(5), AUDIO_OBJECT_TYPE)),
            ("audio_object_type_ext", T::when(aot.clone().equal_to(E::lit(31)), bits(6))),
            ("sampling_frequency_index", T::enumeration("SamplingFrequency", bits(4), FREQUENCY_INDEX)),
            ("sampling_frequency", T::when(E::field("sampling_frequency_index").equal_to(E::lit(15)), bits(24))),
            ("channel_configuration", T::enumeration("ChannelConfiguration", bits(4), CHANNEL_CONFIGURATION)),
            ("frame_length_flag", T::when(general.clone(), bits(1))),
            ("depends_on_core_coder", T::when(general.clone(), bits(1))),
            ("core_coder_delay", T::when(general.clone().both(E::field("depends_on_core_coder")), bits(14))),
            ("extension_flag", T::when(general, bits(1))),
            ("sync_extension", T::when(sync, extension)),
        ],
    )
    .field_doc("audio_object_type_ext", "When audio_object_type is 31, the type is 32 plus this value.")
    .field_doc("frame_length_flag", "0: 1024 samples per frame. 1: 960 samples per frame.")
}

fn sl_config() -> T {
    T::structure("SlConfigDescriptor", vec![("predefined", T::u8()), ("custom", T::bytes(E::Remaining))])
        .field_doc("predefined", "2: the fixed settings ISO 14496-14 requires inside an MP4 file.")
}

fn initial_object_descriptor() -> T {
    T::structure(
        "InitialObjectDescriptorBody",
        vec![
            ("object_descriptor_id", bits(10)),
            ("url_flag", bits(1)),
            ("include_inline_profile_level_flag", bits(1)),
            ("reserved", bits(4)),
            ("url_length", T::when(E::field("url_flag"), T::u8())),
            ("url", T::when(E::field("url_flag"), T::utf8(E::field("url_length")))),
            ("od_profile_level", T::when(E::field("url_flag").negate(), T::u8())),
            ("scene_profile_level", T::when(E::field("url_flag").negate(), T::u8())),
            ("audio_profile_level", T::when(E::field("url_flag").negate(), T::u8())),
            ("visual_profile_level", T::when(E::field("url_flag").negate(), T::u8())),
            ("graphics_profile_level", T::when(E::field("url_flag").negate(), T::u8())),
            ("descriptors", T::repeat(descriptor(Level::InEs), Until::End)),
        ],
    )
    .field_doc("audio_profile_level", "0xff: no audio capability required.")
}

/// Opus in MP4: the Opus identification header, big-endian.
fn dops() -> T {
    T::structure(
        "OpusSpecific",
        vec![
            ("version", T::u8()),
            ("output_channel_count", T::u8()),
            ("pre_skip", u16be()),
            ("input_sample_rate", u32be()),
            ("output_gain", signed(16, 8)),
            ("channel_mapping_family", T::u8()),
            ("stream_count", T::when(E::field("channel_mapping_family").not_equal(E::lit(0)), T::u8())),
            ("coupled_count", T::when(E::field("channel_mapping_family").not_equal(E::lit(0)), T::u8())),
            (
                "channel_mapping",
                T::when(E::field("channel_mapping_family").not_equal(E::lit(0)), T::array(T::u8(), E::field("output_channel_count"))),
            ),
        ],
    )
    .field_doc("pre_skip", "Samples at 48 kHz to throw away from the start of the decoded audio.")
    .field_doc("input_sample_rate", "The sample rate the audio had before it was encoded, for information only; playback normally uses 48 kHz.")
    .field_doc("output_gain", "Gain in dB to apply to the decoded audio.")
}

/// FLAC in MP4: the FLAC metadata blocks.
fn dfla() -> T {
    let streaminfo = T::structure(
        "FlacStreamInfo",
        vec![
            ("min_block_size", u16be()),
            ("max_block_size", u16be()),
            ("min_frame_size", bits(24)),
            ("max_frame_size", bits(24)),
            ("sample_rate", bits(20)),
            ("channels_minus1", bits(3)),
            ("bits_per_sample_minus1", bits(5)),
            ("total_samples", T::UInt { bits: 36, endian: Big }),
            ("md5", T::bytes(E::lit(16))),
        ],
    );
    let block = T::structure(
        "FlacMetadataBlock",
        vec![
            ("last", bits(1)),
            (
                "block_type",
                T::enumeration(
                    "FlacBlockType",
                    bits(7),
                    &[(0, "STREAMINFO"), (1, "PADDING"), (2, "APPLICATION"), (3, "SEEKTABLE"), (4, "VORBIS_COMMENT"), (5, "CUESHEET"), (6, "PICTURE")],
                ),
            ),
            ("length", bits(24)),
            ("data", T::sized(E::field("length").at_most(E::Remaining), T::switch(E::field("block_type"), vec![(0, streaminfo)], T::bytes(E::Remaining)))),
        ],
    )
    .named_by("block_type");
    let mut fields = full_box();
    fields.push(("blocks", T::repeat(block, Until::End)));
    T::structure("FlacSpecific", fields)
}

/// Apple Lossless: the ALAC magic cookie.
fn alac() -> T {
    let mut fields = full_box();
    fields.extend(vec![
        ("frame_length", u32be()),
        ("compatible_version", T::u8()),
        ("bit_depth", T::u8()),
        ("pb", T::u8()),
        ("mb", T::u8()),
        ("kb", T::u8()),
        ("num_channels", T::u8()),
        ("max_run", u16be()),
        ("max_frame_bytes", u32be()),
        ("avg_bit_rate", u32be()),
        ("sample_rate", u32be()),
    ]);
    T::structure("AlacSpecific", fields)
        .field_doc("frame_length", "Samples per frame, normally 4096.")
        .field_doc("pb", "Rice coding settings: pb, mb and kb are normally 40, 10 and 14.")
}

/// Dolby Digital: the AC-3 specific box.
fn dac3() -> T {
    T::structure(
        "Ac3Specific",
        vec![
            ("fscod", T::enumeration("Ac3SampleRate", bits(2), &[(0, "48000 Hz"), (1, "44100 Hz"), (2, "32000 Hz")])),
            ("bsid", bits(5)),
            ("bsmod", bits(3)),
            ("acmod", bits(3)),
            ("lfeon", bits(1)),
            ("bit_rate_code", bits(5)),
            ("reserved", bits(5)),
        ],
    )
    .field_doc("acmod", "Audio coding mode: which full-range channels there are. 2 is stereo, 7 is L C R Ls Rs.")
    .field_doc("lfeon", "1: there is an LFE (subwoofer) channel.")
}

/// Uncompressed audio in ISO 23003-5: how each sample is written.
fn pcmc() -> T {
    let mut fields = full_box();
    fields.extend(vec![("format_flags", T::flags("PcmFormat", T::u8(), &[(0, "little-endian")])), ("pcm_sample_size", T::u8())]);
    T::structure("PcmConfig", fields)
}

/// Which protection scheme an encrypted track uses.
fn schm() -> T {
    let mut fields = full_box_flags("SchemeFlags", &[(0, "URI present")]);
    fields.extend(vec![
        ("scheme_type", fourcc()),
        ("scheme_version", u32be()),
        ("scheme_uri", T::when(E::field("flags").bit(0), T::utf8_padded(E::Remaining, 0))),
    ]);
    T::structure("SchemeType", fields).field_doc("scheme_type", "cenc, cbc1, cens or cbcs for common encryption.")
}

/// Common encryption's default settings for a track.
fn tenc() -> T {
    let mut fields = full_box();
    fields.extend(vec![
        ("reserved", T::u8()),
        ("crypt_skip", T::u8()),
        ("default_is_protected", T::u8()),
        ("default_per_sample_iv_size", T::u8()),
        ("default_kid", T::bytes(E::lit(16))),
        (
            "default_constant_iv_size",
            T::when(E::field("default_is_protected").equal_to(E::lit(1)).both(E::field("default_per_sample_iv_size").equal_to(E::lit(0))), T::u8()),
        ),
        ("default_constant_iv", T::when(E::Remaining.greater_than(E::lit(0)), T::bytes(E::Remaining))),
    ]);
    T::structure("TrackEncryption", fields)
        .field_doc("crypt_skip", "In version 1, the pattern: encrypted blocks in the high four bits, clear blocks in the low four bits. Zero in version 0.")
        .field_doc("default_kid", "The ID of the key the samples are encrypted with.")
}

/// A DRM system's data for the key it protects the movie with.
fn pssh() -> T {
    let mut fields = full_box();
    fields.extend(vec![
        ("system_id", T::bytes(E::lit(16))),
        ("kid_count", T::when(E::field("version").greater_than(E::lit(0)), u32be())),
        ("kids", T::when(E::field("version").greater_than(E::lit(0)), T::array(T::bytes(E::lit(16)), E::field("kid_count")))),
        ("data_size", u32be()),
        ("data", T::bytes(E::field("data_size").at_most(E::Remaining))),
    ]);
    T::structure("ProtectionSystemHeader", fields)
        .field_doc("system_id", "Which DRM system: edef8ba9... is Widevine, 9a04f079... PlayReady, 94ce86fb... FairPlay.")
}

#[cfg(test)]
mod tests {
    use super::super::mp4;
    use super::super::tests::{boxed, cat, full, value};
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    fn named(raw: i128, name: &str) -> Value {
        Value::Enum { raw, name: Some(name.into()), hex: false }
    }

    /// The sound settings of a version 0 entry, with `boxes` after them.
    fn audio_entry(format: &[u8; 4], channels: u16, rate: u32, boxes: &[u8]) -> Vec<u8> {
        let mut body = vec![0, 0, 0, 0, 0, 0, 0, 1]; // reserved, data reference index
        body.extend_from_slice(&[0; 8]); // version, revision, vendor
        body.extend_from_slice(&channels.to_be_bytes());
        body.extend_from_slice(&16u16.to_be_bytes());
        body.extend_from_slice(&[0; 4]);
        body.extend_from_slice(&(rate << 16).to_be_bytes());
        body.extend_from_slice(boxes);
        boxed(format, &body)
    }

    #[test]
    fn an_aac_entry_reads_down_to_its_audio_specific_config() {
        // The esds ffmpeg wrote for aac-audio-only.m4a: AAC LC, 22050 Hz, mono,
        // with the HE-AAC sync extension saying there is no SBR.
        let esds = boxed(b"esds", &hex("0000000003808080250001000480808017401500000000008069000080690580808005138856e500068080800102"));
        let stsd = full(b"stsd", 0, 0, &cat(&[&1u32.to_be_bytes(), &audio_entry(b"mp4a", 1, 22050, &esds)]));
        let d = Document::new(MemSource(stsd));
        let mut ev = Evaluator::new(mp4());
        let entry = ["0", "body", "entries", "0", "body"];
        let at = |names: &[&'static str]| -> Vec<&'static str> { entry.iter().chain(names).copied().collect() };
        assert_eq!(value(&mut ev, &d, &at(&["channel_count"])), Value::UInt(1));
        assert_eq!(value(&mut ev, &d, &at(&["sample_rate"])), Value::Float(22050.0));
        let config = at(&["boxes", "0", "body", "descriptors", "0", "body", "descriptors", "0", "body"]);
        let mut under = |names: &[&'static str]| value(&mut ev, &d, &config.iter().chain(names).copied().collect::<Vec<_>>());
        assert_eq!(under(&["object_type_indication"]), Value::Enum { raw: 0x40, name: Some("MPEG-4 Audio (AAC)".into()), hex: true });
        assert_eq!(under(&["stream_type"]), named(5, "audio"));
        let asc = ["descriptors", "0", "body", "header"];
        let mut asc_field = |name: &'static str| under(&asc.iter().copied().chain([name]).collect::<Vec<_>>());
        assert_eq!(asc_field("audio_object_type"), named(2, "AAC LC"));
        assert_eq!(asc_field("sampling_frequency_index"), named(7, "22050 Hz"));
        assert_eq!(asc_field("channel_configuration"), named(1, "mono"));
        let sync = ["descriptors", "0", "body", "header", "sync_extension"];
        let mut sync_field = |name: &'static str| under(&sync.iter().copied().chain([name]).collect::<Vec<_>>());
        assert_eq!(sync_field("sync_extension_type"), Value::UInt(0x2b7));
        assert_eq!(sync_field("sbr_present_flag"), Value::UInt(0));
    }

    #[test]
    fn an_hevc_configuration_reads_its_parameter_set_arrays() {
        // Main profile, level 93, four-byte NAL lengths, and one array holding
        // one VPS of two bytes: the NAL unit header 40 01.
        let mut hvcc = vec![1, 0x01];
        hvcc.extend_from_slice(&[0x60, 0, 0, 0]);
        hvcc.extend_from_slice(&[0x90, 0, 0, 0, 0, 0]);
        hvcc.extend_from_slice(&[93, 0xf0, 0x00, 0xfc, 0xfd, 0xf8, 0xf8, 0, 0, 0x0f, 1]);
        hvcc.extend_from_slice(&[0xa0, 0, 1, 0, 2, 0x40, 0x01]);
        let d = Document::new(MemSource(boxed(b"hvcC", &hvcc)));
        let mut ev = Evaluator::new(mp4());
        assert_eq!(value(&mut ev, &d, &["0", "body", "general_profile_idc"]), named(1, "Main"));
        assert_eq!(value(&mut ev, &d, &["0", "body", "general_level_idc"]), Value::UInt(93));
        assert_eq!(value(&mut ev, &d, &["0", "body", "chroma_format"]), named(1, "4:2:0"));
        assert_eq!(value(&mut ev, &d, &["0", "body", "length_size_minus_one"]), Value::UInt(3));
        let vps = value(&mut ev, &d, &["0", "body", "arrays", "0", "nalus", "0", "nal", "unit_type"]);
        assert_eq!(vps, named(32, "VPS"));
    }

    #[test]
    fn a_high_profile_avc_configuration_reads_its_extra_fields() {
        // High profile, no parameter sets, then 4:2:0, 8-bit luma and chroma,
        // and no SPS extensions.
        let avcc = [1u8, 100, 0, 30, 0xff, 0xe0, 0, 0xfd, 0xf8, 0xf8, 0];
        let d = Document::new(MemSource(boxed(b"avcC", &avcc)));
        let mut ev = Evaluator::new(mp4());
        assert_eq!(value(&mut ev, &d, &["0", "body", "high_profile", "chroma_format"]), named(1, "4:2:0"));
        assert_eq!(value(&mut ev, &d, &["0", "body", "high_profile", "bit_depth_luma_minus8"]), Value::UInt(0));
        assert_eq!(value(&mut ev, &d, &["0", "body", "high_profile", "sps_ext_count"]), Value::UInt(0));
    }

    #[test]
    fn a_quicktime_pcm_entry_reads_its_channel_layout() {
        let mut chan = vec![0u8; 4];
        chan.extend_from_slice(&0x0064_0001u32.to_be_bytes());
        chan.extend_from_slice(&[0; 8]);
        let entry = audio_entry(b"twos", 1, 22050, &boxed(b"chan", &chan));
        let stsd = full(b"stsd", 0, 0, &cat(&[&1u32.to_be_bytes(), &entry]));
        let d = Document::new(MemSource(stsd));
        let mut ev = Evaluator::new(mp4());
        let tag = value(&mut ev, &d, &["0", "body", "entries", "0", "body", "boxes", "0", "body", "channel_layout_tag"]);
        assert_eq!(tag, Value::Enum { raw: 0x0064_0001, name: Some("mono".into()), hex: true });
    }
}
