//! MPEG audio: an MP3 file, and any other stream of MPEG-1, MPEG-2 or
//! MPEG-2.5 Layer I, II or III frames.
//!
//! The stream has no header of its own. It is frames, one after another, and
//! each frame opens with a 32-bit header that says how long the frame is: the
//! bitrate, the sample rate and the padding bit are enough to work it out. So
//! the length of every frame is computed from its own first four bytes, and a
//! file of thousands of frames is walked without reading any of them further.
//!
//! What sits around the frames is tags. An ID3v2 tag at the front, borrowed
//! from [`super::id3`]. At the back, in this order when they are all there,
//! an APEv2 tag, a Lyrics3v2 tag and the 128-byte ID3v1 tag. Each is found
//! from the end of the file, the way players find them, so the frames stop
//! where the first of them begins.
//!
//! The first frame of a file from LAME, FFmpeg or a Fraunhofer encoder often
//! carries no sound. It holds a Xing or Info header (from LAME and FFmpeg) or
//! a VBRI header (from Fraunhofer) with the number of frames and bytes, a
//! table for seeking, and in a LAME tag the encoder delay and padding.
//!
//! Sources: the frame header as ISO/IEC 11172-3 and 13818-3 define it, in
//! Gabriel Bouvigne's summary at mp3-tech.org; the frame length and Layer III
//! side information as FFmpeg's `mpegaudiodecheader.c` and
//! `mpegaudiodec_template.c` read them; the Xing, LAME and VBRI headers from
//! the Mp3 Info Tag rev 1 specification and FFmpeg's `mp3dec.c`; ID3v1 and
//! Lyrics3v2 from id3.org; APEv2 from the Hydrogenaudio wiki.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T, Until, Valid};

pub fn mp3() -> Template {
    Template::new("mp3", file())
}

/// The whole file: a tag in front, the frames, and the tags at the back.
fn file() -> T {
    let lyrics3_size = || trailer_lyrics3(trailer_id3v1()).sub(trailer_id3v1());
    let apev2_size = || trailers().sub(trailer_lyrics3(trailer_id3v1()));
    T::structure(
        "MP3",
        vec![
            ("id3v2", T::when(id3v2_ahead(E::SpaceSize.sub(E::SpacePos)), super::id3::file_tag())),
            // Everything between the front tag and the back ones. Worked out
            // once here, so that a walk over the frames does not look at the
            // end of the file again for each one.
            ("audio", T::sized(E::SpaceSize.sub(E::SpacePos).sub(trailers()).at_least(E::lit(0)), audio())),
            ("apev2", T::when(apev2_size().greater_than(E::lit(0)), T::sized(apev2_size(), apev2()))),
            ("lyrics3", T::when(lyrics3_size().greater_than(E::lit(0)), T::sized(lyrics3_size(), lyrics3()))),
            ("id3v1", T::when(trailer_id3v1().greater_than(E::lit(0)), T::sized(E::lit(128), id3v1()))),
        ],
    )
    .field_doc("id3v2", "ID3v2 tag: title, artist, cover art and other information about the recording.")
}

/// MPEG audio frames to the end of the window they are in: the part of an MP3
/// file between its tags, or the `data` chunk of a WAVE file whose format tag
/// is 0x0055. Only looks at the bytes in its own window, so a format that
/// carries MPEG audio inside it can borrow this as its contents.
pub(super) fn audio() -> T {
    T::structure(
        "MpegAudio",
        vec![
            ("frames", T::repeat(element(), Until::While(ahead().greater_than(E::lit(0))))),
            ("not_frames", T::when(E::Remaining.greater_than(E::lit(0)), T::bytes(E::Remaining))),
        ],
    )
    .field_doc(
        "not_frames",
        "Bytes after the last frame that do not start with a valid frame header. Often the end of a frame \
         that was cut off, or a tag this template does not read.",
    )
}

// What comes next in the run of frames.
const NOTHING: i128 = 0;
const FRAME: i128 = 1;
const TAG: i128 = 2;
const ZEROS: i128 = 3;

/// Which kind of element starts here: a frame, an ID3v2 tag (which a stream
/// joined from several files has between them), a run of zero bytes (which
/// some taggers leave after the tag in front), or nothing this reads.
fn ahead() -> E {
    let has = |n: i128| E::Remaining.greater_or_equal(E::lit(n));
    E::cond(
        has(4).both(header_ok(&Peeked)),
        E::lit(FRAME),
        E::cond(
            id3v2_ahead(E::Remaining),
            E::lit(TAG),
            E::cond(has(1).both(E::peek(8, Big).equal_to(E::lit(0))), E::lit(ZEROS), E::lit(NOTHING)),
        ),
    )
}

fn id3v2_ahead(room: E) -> E {
    room.greater_or_equal(E::lit(10)).both(E::peek(24, Big).equal_to(E::lit(0x49_44_33)))
}

fn element() -> T {
    T::switch(
        ahead(),
        vec![
            (FRAME, T::sized(frame_length(&Peeked).at_most(E::Remaining), frame())),
            (TAG, super::id3::file_tag()),
            (ZEROS, T::structure("Padding", vec![("zeros", T::bytes(E::run(&[0])))])),
        ],
        T::bytes(E::Remaining),
    )
}

/// Where the header's fields come from. The run of frames has to size a frame
/// before reading it, so it reads them straight from the next four bytes; the
/// frame itself has them as fields.
trait HeaderBits {
    fn get(&self, name: &str) -> E;
}

/// The next 32 bits, cut into the header's fields.
struct Peeked;

impl HeaderBits for Peeked {
    fn get(&self, name: &str) -> E {
        let (top, width) = place(name);
        E::bit_field(E::peek(32, Big), top, width)
    }
}

/// The 32 bits at a byte address, for a header that is not the next thing to
/// be read: the frame after this one.
struct Absolute(E);

impl HeaderBits for Absolute {
    fn get(&self, name: &str) -> E {
        let (top, width) = place(name);
        E::PeekIn { at: Box::new(self.0.clone().mul(E::lit(8)).add(E::lit(31 - i128::from(top)))), bits: width, endian: Big }
    }
}

/// Where a header field is in the 32 bits: its top bit, counting from 0 at
/// the low end, and its width.
fn place(name: &str) -> (u32, u32) {
    match name {
        "sync" => (31, 11),
        "version" => (20, 2),
        "layer" => (18, 2),
        "protection_bit" => (16, 1),
        "bitrate_index" => (15, 4),
        "sample_rate_index" => (11, 2),
        "padding_bit" => (9, 1),
        "channel_mode" => (7, 2),
        _ => unreachable!("no header field {name}"),
    }
}

/// The frame's own fields.
struct Fields;

impl HeaderBits for Fields {
    fn get(&self, name: &str) -> E {
        E::field(name)
    }
}

/// A header every decoder accepts: the sync bits set, and no field holding the
/// value the standard reserves. A bitrate index of 0 is free format, whose
/// frames have no length in the header, and is not read.
fn header_ok(h: &dyn HeaderBits) -> E {
    h.get("sync")
        .equal_to(E::lit(0x7ff))
        .both(h.get("version").not_equal(E::lit(1)))
        .both(h.get("layer").not_equal(E::lit(0)))
        .both(h.get("bitrate_index").not_equal(E::lit(0)))
        .both(h.get("bitrate_index").not_equal(E::lit(15)))
        .both(h.get("sample_rate_index").not_equal(E::lit(3)))
}

/// The bitrate tables in kbit/s, one per column of the standard's table, by
/// bitrate index. Index 0 is free format and 15 is not allowed; 15 is left out
/// and 0 is 0 here.
const KBPS: [[i128; 15]; 5] = [
    // MPEG-1 Layer I
    [0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448],
    // MPEG-1 Layer II
    [0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384],
    // MPEG-1 Layer III
    [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320],
    // MPEG-2 and 2.5 Layer I
    [0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256],
    // MPEG-2 and 2.5 Layers II and III
    [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
];

/// Rates `from` to `to` of one table, as 6-bit numbers of eighths with the
/// first in the lowest bits. Ten of them fit under the 64-bit limit on a
/// shift, so each table is two of these.
const fn packed(rates: &[i128; 15], from: usize, to: usize) -> i128 {
    let mut out = 0;
    let mut i = from;
    while i < to {
        out |= (rates[i] / 8) << (6 * (i - from));
        i += 1;
    }
    out
}

/// Which of the five tables a header's version and layer pick, 0 to 4.
fn table(h: &dyn HeaderBits) -> E {
    // Layer is 3 for Layer I and 1 for Layer III, so 3 - layer counts the
    // MPEG-1 tables in order.
    E::cond(
        h.get("version").equal_to(E::lit(3)),
        E::lit(3).sub(h.get("layer")),
        E::cond(h.get("layer").equal_to(E::lit(3)), E::lit(3), E::lit(4)),
    )
}

/// The bitrate in kbit/s.
fn kbps(h: &dyn HeaderBits) -> E {
    let t = table(h);
    let i = h.get("bitrate_index");
    let low = i.clone().less_than(E::lit(10));
    let pick = |n: usize| E::cond(low.clone(), E::lit(packed(&KBPS[n], 0, 10)), E::lit(packed(&KBPS[n], 10, 15)));
    let tables = E::cond(
        t.clone().equal_to(E::lit(0)),
        pick(0),
        E::cond(
            t.clone().equal_to(E::lit(1)),
            pick(1),
            E::cond(t.clone().equal_to(E::lit(2)), pick(2), E::cond(t.equal_to(E::lit(3)), pick(3), pick(4))),
        ),
    );
    let place = E::cond(low.clone(), i.clone(), i.sub(E::lit(10)));
    tables.shr(place.mul(E::lit(6))).and(E::lit(63)).mul(E::lit(8))
}

/// The sample rate in Hz: MPEG-1's rates, halved for MPEG-2 and quartered for
/// MPEG-2.5.
fn sample_rate(h: &dyn HeaderBits) -> E {
    let i = h.get("sample_rate_index");
    let base = E::cond(
        i.clone().equal_to(E::lit(0)),
        E::lit(44100),
        E::cond(i.equal_to(E::lit(1)), E::lit(48000), E::lit(32000)),
    );
    let v = h.get("version");
    let shift = E::cond(v.clone().equal_to(E::lit(3)), E::lit(0), E::cond(v.equal_to(E::lit(2)), E::lit(1), E::lit(2)));
    base.shr(shift)
}

/// The frame's length in bytes, header included. Layer I counts in 4-byte
/// slots; Layers II and III in bytes, and Layer III in MPEG-2 and 2.5 has half
/// as many samples per frame and so half the bytes.
fn frame_length(h: &dyn HeaderBits) -> E {
    let rate = sample_rate(h);
    let pad = h.get("padding_bit");
    let layer_1 = E::lit(12000).mul(kbps(h)).div(rate.clone()).add(pad.clone()).mul(E::lit(4));
    let half = h.get("layer").equal_to(E::lit(1)).both(h.get("version").not_equal(E::lit(3)));
    let others = E::lit(144000).mul(kbps(h)).div(rate.shl(half)).add(pad);
    E::cond(h.get("layer").equal_to(E::lit(3)), layer_1, others)
}

fn bits(n: u32) -> T {
    T::UInt { bits: n, endian: Big }
}

const VERSION: &[(i128, &str)] = &[(0, "MPEG-2.5"), (1, "reserved"), (2, "MPEG-2"), (3, "MPEG-1")];
const LAYER: &[(i128, &str)] = &[(0, "reserved"), (1, "Layer III"), (2, "Layer II"), (3, "Layer I")];
const PROTECTION: &[(i128, &str)] = &[(0, "CRC follows"), (1, "no CRC")];
const CHANNEL_MODE: &[(i128, &str)] = &[(0, "stereo"), (1, "joint stereo"), (2, "dual channel"), (3, "single channel")];
const EMPHASIS: &[(i128, &str)] = &[(0, "none"), (1, "50/15 µs"), (2, "reserved"), (3, "CCITT J.17")];
const MODE_EXTENSION_LAYER_1_2: &[(i128, &str)] =
    &[(0, "bands 4 to 31"), (1, "bands 8 to 31"), (2, "bands 12 to 31"), (3, "bands 16 to 31")];
const MODE_EXTENSION_LAYER_3: &[(i128, &str)] = &[
    (0, "intensity stereo off, M/S stereo off"),
    (1, "intensity stereo on, M/S stereo off"),
    (2, "intensity stereo off, M/S stereo on"),
    (3, "intensity stereo on, M/S stereo on"),
];

/// The bitrate index, named with the rate it stands for in the table this
/// header's version and layer pick.
fn bitrate_index() -> T {
    let cases = (0..5)
        .map(|t| {
            let labels: Vec<(i128, String)> = (0..16)
                .map(|i| {
                    let label = match i {
                        0 => "free format".to_string(),
                        15 => "not allowed".to_string(),
                        _ => format!("{} kbit/s", KBPS[t][i as usize]),
                    };
                    (i, label)
                })
                .collect();
            let refs: Vec<(i128, &str)> = labels.iter().map(|(i, s)| (*i, s.as_str())).collect();
            (t as i128, T::enumeration("Bitrate", bits(4), &refs))
        })
        .collect();
    T::switch(table(&Fields), cases, bits(4))
}

fn sample_rate_index() -> T {
    let rates = |r: [&'static str; 3]| T::enumeration("SampleRate", bits(2), &[(0, r[0]), (1, r[1]), (2, r[2]), (3, "reserved")]);
    T::switch(
        E::field("version"),
        vec![
            (3, rates(["44100 Hz", "48000 Hz", "32000 Hz"])),
            (2, rates(["22050 Hz", "24000 Hz", "16000 Hz"])),
            (0, rates(["11025 Hz", "12000 Hz", "8000 Hz"])),
        ],
        bits(2),
    )
}

/// One frame: the header, the CRC when there is one, and what the layer puts
/// after them.
fn frame() -> T {
    let f = E::field;
    let layer_3 = || f("layer").equal_to(E::lit(1));
    T::structure(
        "Frame",
        vec![
            ("sync", bits(11)),
            ("version", T::enumeration("MpegVersion", bits(2), VERSION)),
            ("layer", T::enumeration("Layer", bits(2), LAYER)),
            ("protection_bit", T::enumeration("Protection", bits(1), PROTECTION)),
            ("bitrate_index", bitrate_index()),
            ("sample_rate_index", sample_rate_index()),
            ("padding_bit", bits(1)),
            ("private_bit", bits(1)),
            ("channel_mode", T::enumeration("ChannelMode", bits(2), CHANNEL_MODE)),
            (
                "mode_extension",
                T::switch(
                    layer_3(),
                    vec![(1, T::enumeration("ModeExtension", bits(2), MODE_EXTENSION_LAYER_3))],
                    T::enumeration("ModeExtension", bits(2), MODE_EXTENSION_LAYER_1_2),
                ),
            ),
            ("copyright", bits(1)),
            ("original", bits(1)),
            ("emphasis", T::enumeration("Emphasis", bits(2), EMPHASIS)),
            ("frame_length", T::computed(frame_length(&Fields))),
            ("crc", T::when(f("protection_bit").equal_to(E::lit(0)), T::u16(Big))),
            ("side_info", T::when(layer_3(), side_info())),
            ("xing", T::when(xing_ahead(), xing())),
            ("vbri", T::when(vbri_ahead(), vbri())),
            ("main_data", T::when(layer_3().both(own_main_data().greater_than(E::lit(0))), T::bytes(own_main_data()))),
            (
                "ancillary_data",
                T::when(
                    layer_3().both(E::Remaining.greater_than(later_main_data())),
                    T::bytes(E::Remaining.sub(later_main_data())),
                ),
            ),
            ("later_main_data", T::when(layer_3().both(E::Remaining.greater_than(E::lit(0))), T::bytes(E::Remaining))),
            ("audio_data", T::when(layer_3().equal_to(E::lit(0)), T::bytes(E::Remaining))),
        ],
    )
    .field_valid("sync", Valid::Eq(E::lit(0x7ff)))
}

/// How many bytes of this frame's main data are in this frame. The main data
/// is every granule's `part2_3_length` bits, and its first `main_data_begin`
/// bytes are at the end of earlier frames: that is the bit reservoir.
fn own_main_data() -> E {
    let part = |g: &str, c: &str| E::within(&["side_info", "granules", g, c, "part2_3_length"]);
    let stereo = |e: E| E::cond(mono(), E::lit(0), e);
    let mpeg_1 = part("0", "0").add(part("1", "0")).add(stereo(part("0", "1").add(part("1", "1"))));
    let others = part("0", "0").add(stereo(part("0", "1")));
    let bits = E::cond(E::field("version").equal_to(E::lit(3)), mpeg_1, others);
    let begin = E::within(&["side_info", "main_data_begin"]);
    bits.div_ceil(E::lit(8)).sub(begin).at_least(E::lit(0)).at_most(E::Remaining)
}

/// How many bytes at the end of this frame are main data of later frames: the
/// next frame's `main_data_begin`, when the next frame is a Layer III frame
/// of the same version. Read straight from the file, past the end of this
/// frame.
fn later_main_data() -> E {
    let next = E::start_of(E::field("sync")).add(E::field("frame_length"));
    let at = Absolute(next.clone());
    let crc = at.get("protection_bit").equal_to(E::lit(0)).mul(E::lit(16));
    let begin_at = next.clone().mul(E::lit(8)).add(E::lit(32)).add(crc);
    let peek = |bits: u32| E::PeekIn { at: Box::new(begin_at.clone()), bits, endian: Big };
    let mpeg_1 = E::field("version").equal_to(E::lit(3));
    let found = E::SpaceSize
        .greater_or_equal(next.add(E::lit(8)))
        .both(header_ok(&at))
        .both(at.get("layer").equal_to(E::lit(1)))
        .both(at.get("version").equal_to(E::field("version")));
    E::cond(found, E::cond(mpeg_1, peek(9), peek(8)), E::lit(0)).at_most(E::Remaining)
}

/// Mono, which halves the side information: one channel instead of two.
fn mono() -> E {
    E::field("channel_mode").equal_to(E::lit(3))
}

fn channels() -> E {
    E::lit(2).sub(mono())
}

/// Layer III side information: where the frame's main data starts, and how
/// each granule of each channel is coded. MPEG-1 has two granules per frame;
/// MPEG-2 and 2.5 have one, and a wider `scalefac_compress`.
fn side_info() -> T {
    T::switch(
        E::field("version"),
        vec![(
            3,
            T::structure(
                "SideInfo",
                vec![
                    ("main_data_begin", bits(9)),
                    ("private_bits", T::UIntExpr { bits: Box::new(E::cond(mono(), E::lit(5), E::lit(3))), endian: Big }),
                    ("scfsi", T::array(bits(4), channels())),
                    ("granules", T::array(T::array(granule(true), channels()), E::lit(2))),
                ],
            ),
        )],
        T::structure(
            "SideInfo",
            vec![
                ("main_data_begin", bits(8)),
                ("private_bits", T::UIntExpr { bits: Box::new(channels()), endian: Big }),
                ("granules", T::array(T::array(granule(false), channels()), E::lit(1))),
            ],
        ),
    )
}

const BLOCK_TYPE: &[(i128, &str)] = &[(0, "reserved"), (1, "start"), (2, "short"), (3, "end")];

/// How one channel of one granule is coded.
fn granule(mpeg_1: bool) -> T {
    let switched = || E::field("window_switching_flag");
    let mut fields = vec![
        ("part2_3_length", bits(12)),
        ("big_values", bits(9)),
        ("global_gain", bits(8)),
        ("scalefac_compress", bits(if mpeg_1 { 4 } else { 9 })),
        ("window_switching_flag", bits(1)),
        ("block_type", T::when(switched(), T::enumeration("BlockType", bits(2), BLOCK_TYPE))),
        ("mixed_block_flag", T::when(switched(), bits(1))),
        ("table_select", T::array(bits(5), E::cond(switched(), E::lit(2), E::lit(3)))),
        ("subblock_gain", T::when(switched(), T::array(bits(3), E::lit(3)))),
        ("region0_count", T::when(switched().equal_to(E::lit(0)), bits(4))),
        ("region1_count", T::when(switched().equal_to(E::lit(0)), bits(3))),
    ];
    if mpeg_1 {
        fields.push(("preflag", bits(1)));
    }
    fields.push(("scalefac_scale", bits(1)));
    fields.push(("count1table_select", bits(1)));
    T::structure("Granule", fields)
}

const XING: i128 = 0x58_69_6e_67;
const INFO: i128 = 0x49_6e_66_6f;

fn xing_ahead() -> E {
    let id = || E::peek(32, Big);
    E::Remaining.greater_or_equal(E::lit(8)).both(id().equal_to(E::lit(XING)).either(id().equal_to(E::lit(INFO))))
}

/// The Xing header that LAME writes in the first frame of a VBR file, or the
/// same header named Info in a CBR file, and the LAME tag that follows it.
fn xing() -> T {
    let flag = |n: u32| E::field("flags").bit(n);
    T::structure(
        "XingHeader",
        vec![
            ("id", T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii)),
            ("flags", T::flags("XingFlags", T::u32(Big), &[(0, "frame count"), (1, "byte count"), (2, "TOC"), (3, "quality")])),
            ("frame_count", T::when(flag(0), T::u32(Big))),
            ("byte_count", T::when(flag(1), T::u32(Big))),
            ("toc", T::when(flag(2), T::array(T::u8(), E::lit(100)))),
            ("quality", T::when(flag(3), T::u32(Big))),
            // The LAME tag starts with the encoder's name: LAME, or Lavc or
            // Lavf when FFmpeg wrote it.
            (
                "lame",
                T::when(
                    E::Remaining.greater_or_equal(E::lit(36)).both(E::peek(8, Big).greater_or_equal(E::lit(0x41))).both(E::peek(8, Big).less_or_equal(E::lit(0x7a))),
                    lame(),
                ),
            ),
        ],
    )
}

const VBR_METHOD: &[(i128, &str)] = &[
    (0, "unknown"),
    (1, "constant bitrate"),
    (2, "average bitrate"),
    (3, "VBR method 1"),
    (4, "VBR method 2"),
    (5, "VBR method 3"),
    (6, "VBR method 4"),
    (8, "constant bitrate, 2 passes"),
    (9, "average bitrate, 2 passes"),
    (15, "reserved"),
];
const STEREO_MODE: &[(i128, &str)] = &[
    (0, "mono"),
    (1, "stereo"),
    (2, "dual channel"),
    (3, "joint stereo"),
    (4, "forced joint stereo"),
    (5, "auto"),
    (6, "intensity stereo"),
    (7, "undefined"),
];
const SOURCE_RATE: &[(i128, &str)] = &[(0, "32 kHz or lower"), (1, "44.1 kHz"), (2, "48 kHz"), (3, "higher than 48 kHz")];
const SURROUND: &[(i128, &str)] = &[(0, "none"), (1, "Dolby Pro Logic"), (2, "Dolby Pro Logic II"), (3, "Ambisonic")];
const GAIN_NAME: &[(i128, &str)] = &[(0, "not set"), (1, "radio"), (2, "audiophile")];
const GAIN_ORIGINATOR: &[(i128, &str)] =
    &[(0, "not set"), (1, "set by artist"), (2, "set by user"), (3, "set by model"), (4, "set by RMS average")];

fn replay_gain() -> T {
    T::structure(
        "ReplayGain",
        vec![
            ("name", T::enumeration("GainName", bits(3), GAIN_NAME)),
            ("originator", T::enumeration("GainOriginator", bits(3), GAIN_ORIGINATOR)),
            ("negative", bits(1)),
            ("gain_tenths_db", bits(9)),
        ],
    )
}

/// The 36 bytes LAME adds after the Xing header.
fn lame() -> T {
    T::structure(
        "LameTag",
        vec![
            ("encoder", T::text(StrLen::Padded { size: E::lit(9), pad: 0 }, Encoding::Ascii)),
            ("revision", bits(4)),
            ("vbr_method", T::enumeration("VbrMethod", bits(4), VBR_METHOD)),
            ("lowpass", T::u8()),
            ("peak", T::F32(Big)),
            ("radio_gain", replay_gain()),
            ("audiophile_gain", replay_gain()),
            ("encoding_flags", T::flags("LameFlags", bits(4), &[(0, "nspsytune"), (1, "nssafejoint"), (2, "continues in the next track"), (3, "continues the previous track")])),
            ("ath_type", bits(4)),
            ("bitrate", T::u8()),
            ("encoder_delay", bits(12)),
            ("end_padding", bits(12)),
            ("source_sample_rate", T::enumeration("SourceRate", bits(2), SOURCE_RATE)),
            ("unwise_settings", bits(1)),
            ("stereo_mode", T::enumeration("StereoMode", bits(3), STEREO_MODE)),
            ("noise_shaping", bits(2)),
            ("mp3_gain", T::sign_magnitude(8, Big)),
            ("unused", bits(2)),
            ("surround", T::enumeration("Surround", bits(3), SURROUND)),
            ("preset", bits(11)),
            ("music_length", T::u32(Big)),
            ("music_crc", T::u16(Big)),
            ("tag_crc", T::u16(Big)),
        ],
    )
}

const VBRI: i128 = 0x56_42_52_49;

/// Fraunhofer's encoder puts its VBRI header 32 bytes after the frame header,
/// whatever the frame's version and channel mode.
fn vbri_ahead() -> E {
    let skip = E::lit(36).sub(E::Pos);
    E::Pos
        .less_or_equal(E::lit(36))
        .both(E::Remaining.greater_or_equal(skip.clone().add(E::lit(26))))
        .both(E::peek_at(skip.mul(E::lit(8)), 32, Big).equal_to(E::lit(VBRI)))
}

fn vbri() -> T {
    T::structure(
        "VbriHeader",
        vec![
            ("before", T::bytes(E::lit(36).sub(E::Pos))),
            ("id", T::magic(b"VBRI")),
            ("version", T::u16(Big)),
            ("delay", T::u16(Big)),
            ("quality", T::u16(Big)),
            ("byte_count", T::u32(Big)),
            ("frame_count", T::u32(Big)),
            ("toc_entries", T::u16(Big)),
            ("toc_scale", T::u16(Big)),
            ("toc_entry_size", T::u16(Big)),
            ("frames_per_toc_entry", T::u16(Big)),
            (
                "toc",
                T::array(T::UIntExpr { bits: Box::new(E::field("toc_entry_size").mul(E::lit(8))), endian: Big }, E::field("toc_entries")),
            ),
        ],
    )
}

// The tags at the back, found from the end of the file the way players find
// them: ID3v1 is the last 128 bytes, Lyrics3v2 ends just before it, and APEv2
// ends just before that.

/// A peek at `back` bytes before the end of the file.
fn from_end(back: E, bits: u32, endian: crate::template::Endian) -> E {
    E::PeekIn { at: Box::new(E::SpaceSize.sub(back).mul(E::lit(8))), bits, endian }
}

/// How many bytes the ID3v1 tag takes: 128 when the file ends with one.
fn trailer_id3v1() -> E {
    let room = E::SpaceSize.greater_or_equal(E::lit(128));
    E::cond(room.both(from_end(E::lit(128), 24, Big).equal_to(E::lit(0x54_41_47))), E::lit(128), E::lit(0))
}

/// How many bytes the tags after `after` take, with the Lyrics3v2 tag added
/// when one ends where they begin. Its size is six ASCII digits before the
/// closing `LYRICS200`, and counts from `LYRICSBEGIN` up to those digits.
fn trailer_lyrics3(after: E) -> E {
    let end = |n: i128| from_end(after.clone().add(E::lit(n)), 8, Big);
    let digits = (0..6).fold(E::lit(0), |acc, i| acc.mul(E::lit(10)).add(end(15 - i).sub(E::lit(0x30))));
    let total = digits.add(E::lit(15));
    let room = || E::SpaceSize.sub(after.clone());
    let found = room()
        .greater_or_equal(E::lit(15 + 11))
        // `LYRICS20` and then `0`: nine bytes, more than one peek holds.
        .both(from_end(after.clone().add(E::lit(9)), 64, Big).equal_to(E::lit(0x4c_59_52_49_43_53_32_30_i128)))
        .both(end(1).equal_to(E::lit(0x30)))
        .both(total.clone().less_or_equal(room()))
        .both(total.clone().greater_or_equal(E::lit(15 + 11)))
        .both(from_end(after.clone().add(total.clone()), 64, Big).equal_to(E::lit(0x4c_59_52_49_43_53_42_45_i128)));
    E::cond(found, after.clone().add(total), after)
}

/// The same, with an APEv2 tag added when its footer ends where the tags
/// after it begin. The footer's size counts the items and the footer, and the
/// header in front of them when its flags say there is one.
fn trailer_apev2(after: E) -> E {
    let room = || E::SpaceSize.sub(after.clone());
    let size = from_end(after.clone().add(E::lit(20)), 32, Little);
    let header = from_end(after.clone().add(E::lit(12)), 32, Little).bit(31).mul(E::lit(32));
    let total = size.clone().add(header);
    let found = room()
        .greater_or_equal(E::lit(32))
        .both(from_end(after.clone().add(E::lit(32)), 64, Big).equal_to(E::lit(APETAGEX)))
        .both(size.greater_or_equal(E::lit(32)))
        .both(total.clone().less_or_equal(room()));
    E::cond(found, after.clone().add(total), after)
}

/// Every tag at the back, in bytes.
fn trailers() -> E {
    trailer_apev2(trailer_lyrics3(trailer_id3v1()))
}

fn apev2() -> T {
    T::structure(
        "APEv2",
        vec![
            ("header", T::when(E::peek(64, Big).equal_to(E::lit(APETAGEX)), ape_header())),
            ("items", T::sized(E::Remaining.sub(E::lit(32)), T::repeat(ape_item(), Until::End))),
            ("footer", ape_header()),
        ],
    )
}

/// `APETAGEX`, as the big-endian number a 64-bit peek reads it as.
const APETAGEX: i128 = 0x41_50_45_54_41_47_45_58;

const APE_VERSION: &[(i128, &str)] = &[(1000, "1.0"), (2000, "2.0")];

fn ape_header() -> T {
    T::structure(
        "ApeHeader",
        vec![
            ("preamble", T::magic(b"APETAGEX")),
            ("version", T::enumeration("ApeVersion", T::u32(Little), APE_VERSION)),
            ("tag_size", T::u32(Little)),
            ("item_count", T::u32(Little)),
            ("flags", T::flags("ApeFlags", T::u32(Little), &[(0, "read-only"), (29, "this is the header"), (30, "has a footer"), (31, "has a header")])),
            ("reserved", T::bytes(E::lit(8))),
        ],
    )
}

const APE_CONTENT: &[(i128, &str)] = &[(0, "UTF-8 text"), (1, "binary"), (2, "external locator"), (3, "reserved")];

fn ape_item() -> T {
    let size = || E::field("value_size");
    let utf8 = || T::text(StrLen::Fixed(size()), Encoding::Utf8);
    T::structure_named(
        "ApeItem",
        "key",
        "value",
        vec![
            ("value_size", T::u32(Little)),
            ("flags", T::flags("ApeItemFlags", T::u32(Little), &[(0, "read-only")])),
            ("content_type", T::enumeration("ApeContent", T::computed(E::bit_field(E::field("flags"), 2, 2)), APE_CONTENT)),
            ("key", T::text(StrLen::Terminated { end: 0, or_end: false }, Encoding::Ascii)),
            ("value", T::switch(E::field("content_type"), vec![(0, utf8()), (2, utf8())], T::bytes(size()))),
        ],
    )
}

fn lyrics3() -> T {
    T::structure(
        "Lyrics3v2",
        vec![
            ("begin", T::magic(b"LYRICSBEGIN")),
            ("fields", T::sized(E::Remaining.sub(E::lit(15)), T::repeat(lyrics3_field(), Until::End))),
            ("size", T::decimal(StrLen::Fixed(E::lit(6)))),
            ("end", T::magic(b"LYRICS200")),
        ],
    )
}

fn lyrics3_field() -> T {
    T::structure_named(
        "Lyrics3Field",
        "id",
        "value",
        vec![
            ("id", T::text(StrLen::Fixed(E::lit(3)), Encoding::Ascii)),
            ("size", T::decimal(StrLen::Fixed(E::lit(5)))),
            ("value", T::text(StrLen::Fixed(E::field("size")), Encoding::Latin1)),
        ],
    )
}

fn id3v1() -> T {
    let text = |n: i128| T::text(StrLen::Padded { size: E::lit(n), pad: 0 }, Encoding::Latin1);
    // ID3v1.1 takes the last two bytes of the comment for a zero and a track
    // number. A zero followed by anything else is how a reader tells.
    let v1_1 = E::peek_at(E::lit(28 * 8), 8, Big).equal_to(E::lit(0)).both(E::peek_at(E::lit(29 * 8), 8, Big).not_equal(E::lit(0)));
    let short = || E::size_of("comment").equal_to(E::lit(28));
    let genres: Vec<(i128, &str)> = GENRES.iter().enumerate().filter(|(_, g)| !g.is_empty()).map(|(i, g)| (i as i128, *g)).chain([(255, "none")]).collect();
    T::structure(
        "ID3v1",
        vec![
            ("magic", T::magic(b"TAG")),
            ("title", text(30)),
            ("artist", text(30)),
            ("album", text(30)),
            ("year", text(4)),
            ("comment", T::switch(v1_1, vec![(1, text(28))], text(30))),
            ("zero_byte", T::when(short(), T::u8())),
            ("track", T::when(short(), T::u8())),
            ("genre", T::enumeration("Genre", T::u8(), &genres)),
        ],
    )
}

/// ID3v1 genres: 0 to 79 from the original list, 80 to 125 added by Winamp,
/// as the ID3v2.3 specification's appendix lists them, and 126 to 191 added by
/// later Winamp versions, as FFmpeg's table has them. 133 is left unnamed.
const GENRES: &[&str] = &[
    "Blues", "Classic Rock", "Country", "Dance", "Disco", "Funk", "Grunge", "Hip-Hop", "Jazz", "Metal",
    "New Age", "Oldies", "Other", "Pop", "R&B", "Rap", "Reggae", "Rock", "Techno", "Industrial",
    "Alternative", "Ska", "Death Metal", "Pranks", "Soundtrack", "Euro-Techno", "Ambient", "Trip-Hop", "Vocal", "Jazz+Funk",
    "Fusion", "Trance", "Classical", "Instrumental", "Acid", "House", "Game", "Sound Clip", "Gospel", "Noise",
    "AlternRock", "Bass", "Soul", "Punk", "Space", "Meditative", "Instrumental Pop", "Instrumental Rock", "Ethnic", "Gothic",
    "Darkwave", "Techno-Industrial", "Electronic", "Pop-Folk", "Eurodance", "Dream", "Southern Rock", "Comedy", "Cult", "Gangsta",
    "Top 40", "Christian Rap", "Pop/Funk", "Jungle", "Native American", "Cabaret", "New Wave", "Psychadelic", "Rave", "Showtunes",
    "Trailer", "Lo-Fi", "Tribal", "Acid Punk", "Acid Jazz", "Polka", "Retro", "Musical", "Rock & Roll", "Hard Rock",
    "Folk", "Folk-Rock", "National Folk", "Swing", "Fast Fusion", "Bebob", "Latin", "Revival", "Celtic", "Bluegrass",
    "Avantgarde", "Gothic Rock", "Progressive Rock", "Psychedelic Rock", "Symphonic Rock", "Slow Rock", "Big Band", "Chorus", "Easy Listening", "Acoustic",
    "Humour", "Speech", "Chanson", "Opera", "Chamber Music", "Sonata", "Symphony", "Booty Bass", "Primus", "Porn Groove",
    "Satire", "Slow Jam", "Club", "Tango", "Samba", "Folklore", "Ballad", "Power Ballad", "Rhythmic Soul", "Freestyle",
    "Duet", "Punk Rock", "Drum Solo", "A cappella", "Euro-House", "Dance Hall", "Goa", "Drum & Bass", "Club-House", "Hardcore Techno",
    "Terror", "Indie", "BritPop", "", "Polsk Punk", "Beat", "Christian Gangsta Rap", "Heavy Metal", "Black Metal", "Crossover",
    "Contemporary Christian", "Christian Rock", "Merengue", "Salsa", "Thrash Metal", "Anime", "Jpop", "Synthpop", "Abstract", "Art Rock",
    "Baroque", "Bhangra", "Big Beat", "Breakbeat", "Chillout", "Downtempo", "Dub", "EBM", "Eclectic", "Electro",
    "Electroclash", "Emo", "Experimental", "Garage", "Global", "IDM", "Illbient", "Industro-Goth", "Jam Band", "Krautrock",
    "Leftfield", "Lounge", "Math Rock", "New Romantic", "Nu-Breakz", "Post-Punk", "Post-Rock", "Psytrance", "Shoegaze", "Space Rock",
    "Trop Rock", "World Music", "Neoclassical", "Audiobook", "Audio Theatre", "Neue Deutsche Welle", "Podcast", "Indie Rock", "G-Funk", "Dubstep",
    "Garage Rock", "Psybient",
];

/// A frame header's length in bytes, for the recogniser, or `None` when the
/// four bytes are not a header this template reads.
fn header_length(h: &[u8]) -> Option<(usize, u32)> {
    let w = u32::from_be_bytes(h.get(..4)?.try_into().ok()?);
    let field = |top: u32, width: u32| (w >> (top + 1 - width)) & ((1 << width) - 1);
    let (version, layer, index, rate, pad) = (field(20, 2), field(18, 2), field(15, 4), field(11, 2), field(9, 1));
    if field(31, 11) != 0x7ff || version == 1 || layer == 0 || index == 0 || index == 15 || rate == 3 {
        return None;
    }
    let table = if version == 3 { 3 - layer } else if layer == 3 { 3 } else { 4 } as usize;
    let kbps = KBPS[table][index as usize] as u32;
    let hz = [44100u32, 48000, 32000][rate as usize] >> [2, 0, 1, 0][version as usize];
    let len = match layer {
        3 => (12000 * kbps / hz + pad) * 4,
        1 if version != 3 => 72000 * kbps / hz + pad,
        _ => 144000 * kbps / hz + pad,
    };
    // What has to agree from one frame to the next: the version, the layer
    // and the sample rate. Not the channel mode, because FFmpeg writes the
    // Info frame in front of LAME's joint stereo frames as plain stereo.
    Some((len as usize, w & 0xfffe_0c00))
}

/// Whether a run of frames starts at `at`: three headers in a row, each where
/// the one before says the next begins, that agree on version, layer and
/// sample rate. A run that reaches the end of the file sooner counts too.
fn frames_at(h: &[u8], at: usize, len: u64) -> bool {
    let mut pos = at;
    let mut first = None;
    for _ in 0..3 {
        if pos as u64 == len {
            return first.is_some();
        }
        if pos + 4 > h.len() {
            // Past what was read: as good as the frames before it.
            return first.is_some();
        }
        let Some((n, same)) = header_length(&h[pos..]) else { return false };
        if *first.get_or_insert(same) != same {
            return false;
        }
        pos += n;
    }
    true
}

/// An MP3 file with an ID3v2 tag in front: frames where the tag ends, after
/// any zero bytes, or a tag too long to see past in the bytes read (cover art
/// makes most of them so). Asked before the signature table, which would
/// otherwise take it as a tag and nothing else. A tag in front of something
/// that is not MPEG audio, such as AAC or FLAC, stays an `id3`.
pub(super) fn is_tagged_mp3(h: &[u8], len: u64) -> bool {
    if h.len() < 10 || !h.starts_with(b"ID3") {
        return false;
    }
    let size = h[6..10].iter().fold(0usize, |acc, b| acc << 7 | (*b & 0x7f) as usize);
    let footer = if h[3] == 4 && h[5] & 0x10 != 0 { 10 } else { 0 };
    let mut end = 10 + size + footer;
    if end >= h.len() {
        return end as u64 <= len;
    }
    while end < h.len() && h[end] == 0 {
        end += 1;
    }
    end == h.len() || frames_at(h, end, len)
}

/// A stream of MPEG audio frames with no tag in front.
pub(super) fn is_mpeg_audio(h: &[u8], len: u64) -> bool {
    frames_at(h, 0, len) && header_length(h).is_some()
}

/// A file whose name says it is MPEG audio: believed when it opens with an
/// ID3v2 tag, or with one frame header after any zero bytes.
pub(super) fn is_named_mpeg_audio(h: &[u8], _len: u64) -> bool {
    let start = h.iter().position(|b| *b != 0).unwrap_or(h.len());
    h.starts_with(b"ID3") || header_length(&h[start..]).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// A frame header and a body of zeros that make up the length it says.
    fn frame_bytes(header: u32) -> Vec<u8> {
        let (len, _) = header_length(&header.to_be_bytes()).unwrap();
        let mut v = header.to_be_bytes().to_vec();
        v.resize(len, 0);
        v
    }

    #[test]
    fn frame_lengths_follow_the_standard() {
        // MPEG-1 Layer III, 128 kbit/s, 44100 Hz: the 417 everyone quotes.
        assert_eq!(header_length(&0xfffb_9000u32.to_be_bytes()).unwrap().0, 417);
        // The same, padded.
        assert_eq!(header_length(&0xfffb_9200u32.to_be_bytes()).unwrap().0, 418);
        // MPEG-2 Layer III, 48 kbit/s, 16000 Hz: 72 * 48000 / 16000.
        assert_eq!(header_length(&0xfff3_68c0u32.to_be_bytes()).unwrap().0, 216);
        // MPEG-1 Layer I, 384 kbit/s, 48000 Hz: (12 * 384000 / 48000) * 4.
        assert_eq!(header_length(&0xffff_c400u32.to_be_bytes()).unwrap().0, 384);
        // MPEG-1 Layer II, 192 kbit/s, 48000 Hz: 144 * 192000 / 48000.
        assert_eq!(header_length(&0xfffd_a400u32.to_be_bytes()).unwrap().0, 576);
        // MPEG-2.5 Layer III, 8 kbit/s, 8000 Hz: 72 * 8000 / 8000.
        assert_eq!(header_length(&0xffe3_1800u32.to_be_bytes()).unwrap().0, 72);
    }

    #[test]
    fn the_template_sizes_frames_the_way_the_recogniser_does() {
        for header in [0xfffb_9000u32, 0xfffb_9200, 0xfff3_68c0, 0xffff_c400, 0xfffd_a400, 0xffe3_1800, 0xfff5_8800] {
            let mut file = frame_bytes(header);
            file.extend(frame_bytes(header));
            let d = Document::new(MemSource(file));
            let mut ev = Evaluator::new(mp3());
            let frames = ev.node(&d, &[1, 0]).unwrap();
            assert_eq!(frames.child_count, 2, "{header:08x}");
            let first = ev.node(&d, &[1, 0, 0]).unwrap();
            assert_eq!(first.size_bits / 8, header_length(&header.to_be_bytes()).unwrap().0 as u64, "{header:08x}");
            let _ = Value::Int(0);
        }
    }
}
