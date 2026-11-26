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
    let f = E::field;
    let present = |name: &str| f(name).greater_than(E::lit(0));
    T::structure(
        "MP3",
        vec![
            ("id3v2", T::when(id3v2_ahead(E::SpaceSize.sub(E::SpacePos)), id3v2(E::SpaceSize.sub(E::SpacePos)))),
            // How long each tag at the end is, found from the end of the file.
            // Worked out once here, so that the expressions below name them
            // rather than repeat the search, and a walk over the frames does
            // not look at the end of the file again for each one.
            ("id3v1_size", T::computed(id3v1_size())),
            ("lyrics3_size", T::computed(lyrics3_size(f("id3v1_size")))),
            ("apev2_size", T::computed(apev2_size(f("id3v1_size").add(f("lyrics3_size"))))),
            // Everything between the front tag and the back ones.
            (
                "audio",
                T::sized(
                    E::SpaceSize.sub(E::SpacePos).sub(f("id3v1_size")).sub(f("lyrics3_size")).sub(f("apev2_size")).at_least(E::lit(0)),
                    audio(),
                ),
            ),
            ("apev2", T::when(present("apev2_size"), T::sized(f("apev2_size"), apev2()))),
            ("lyrics3", T::when(present("lyrics3_size"), T::sized(f("lyrics3_size"), lyrics3()))),
            ("id3v1", T::when(present("id3v1_size"), T::sized(E::lit(128), id3v1()))),
        ],
    )
    .machinery(&["id3v1_size", "lyrics3_size", "apev2_size"])
    .field_doc(
        "id3v1_size",
        "Size in bytes of the ID3v1 tag at the end of the file: 128, or 0 when there is none. Qubero computes it; it is \
         not stored in the file.",
    )
    .field_doc(
        "lyrics3_size",
        "Size in bytes of the Lyrics3v2 tag before the ID3v1 tag, or 0 when there is none. Qubero computes it from the \
         size the tag ends with.",
    )
    .field_doc(
        "apev2_size",
        "Size in bytes of the APEv2 tag before the Lyrics3v2 and ID3v1 tags, or 0 when there is none. Qubero computes \
         it from the tag's footer.",
    )
    .field_doc("id3v2", "ID3v2 tag at the start of the file, with information such as the title, artist and cover art.")
    .field_doc("audio", "The MPEG audio frames: everything between the ID3v2 tag at the start of the file and the tags at the end.")
    .field_doc("apev2", "APEv2 tag near the end of the file: a list of key and value pairs. It comes before any Lyrics3v2 or ID3v1 tag.")
    .field_doc(
        "lyrics3",
        "Lyrics3v2 tag, just before the ID3v1 tag. It contains lyrics and other text, such as the author and extended \
         versions of the title, artist and album.",
    )
    .field_doc(
        "id3v1",
        "ID3v1 tag: the last 128 bytes of the file, starting with \"TAG\". It has fixed-size fields for the title, artist, \
         album, year, comment and genre.",
    )
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
        "frames",
        "The MPEG audio frames, one after another. The list can also contain runs of zero bytes between frames, and \
         another ID3v2 tag where several files were joined into one.",
    )
    .field_doc(
        "not_frames",
        "Bytes after the last frame that Qubero cannot read as a frame or as one of the tags at the end. For example, \
         a frame cut off by the end of the file, free-format frames, whose length is not in the header, or a tag in a \
         format Qubero does not read.",
    )
}

// What comes next in the run of frames.
const NOTHING: i128 = 0;
const FRAME: i128 = 1;
const TAG: i128 = 2;
const ZEROS: i128 = 3;

/// Which kind of element starts here: a frame, an ID3v2 tag (which a stream
/// joined from several files has between them), a run of zero bytes (which
/// some taggers leave after the tag in front), or nothing this reads. A frame
/// counts only when all of it is there, so one cut off by the end of the file
/// is left to `not_frames`.
fn ahead() -> E {
    let has = |n: i128| E::Remaining.greater_or_equal(E::lit(n));
    E::cond(
        has(4).both(header_ok(&Peeked)).both(frame_length(&Peeked).less_or_equal(E::Remaining)),
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

/// An ID3v2 tag in a window as long as its header says, so that the audio
/// starts where the header says the tag ends, whatever its frames read as.
fn id3v2(room: E) -> T {
    let byte = |n: i128| E::peek_at(E::lit(8 * n), 8, Big);
    let size = (6..10).fold(E::lit(0), |acc, n| acc.mul(E::lit(128)).add(byte(n).and(E::lit(0x7f))));
    let footer = byte(3).equal_to(E::lit(4)).both(byte(5).bit(4)).mul(E::lit(10));
    T::sized(E::lit(10).add(size).add(footer).at_most(room), super::id3::file_tag())
}

fn element() -> T {
    T::switch(
        ahead(),
        vec![
            (FRAME, T::sized(frame_length(&Peeked), frame())),
            (TAG, id3v2(E::Remaining)),
            (
                ZEROS,
                T::structure("Padding", vec![("zeros", T::bytes(E::run(&[0])))])
                    .field_doc("zeros", "Zero bytes between frames. Some taggers leave a run of zero bytes after the ID3v2 tag."),
            ),
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
const MODE_EXTENSION_LAYER_1_2: &[(i128, &str)] = &[
    (0, "intensity stereo in subbands 4 to 31"),
    (1, "intensity stereo in subbands 8 to 31"),
    (2, "intensity stereo in subbands 12 to 31"),
    (3, "intensity stereo in subbands 16 to 31"),
];
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
    .field_doc("sync", "11 bits, all set to 1, at the start of every frame header.")
    .field_doc(
        "version",
        "MPEG version: MPEG-1, MPEG-2 or MPEG-2.5; value 1 is reserved. MPEG-2 uses half the sample rates of MPEG-1, \
         and MPEG-2.5 (an unofficial extension) uses a quarter of them.",
    )
    .field_doc("layer", "MPEG audio layer: Layer I, II or III. Value 0 is reserved.")
    .field_doc("protection_bit", "0 means a 16-bit CRC follows the frame header, and 1 means there is no CRC.")
    .field_doc(
        "bitrate_index",
        "Index into the bitrate table for this frame's MPEG version and layer, shown as the bitrate in kbit/s. \
         0 means free format (the bitrate is not in the header), and 15 is not allowed.",
    )
    .field_doc("sample_rate_index", "Index into the sample rate table for this frame's MPEG version, shown as the sample rate in Hz.")
    .field_doc(
        "padding_bit",
        "1 means the frame has one extra slot: 4 bytes in Layer I, 1 byte in Layers II and III. Encoders set it on \
         some frames so that the stream matches the bitrate exactly.",
    )
    .field_doc("private_bit", "A bit for applications to use; decoders ignore it.")
    .field_doc("channel_mode", "Stereo, joint stereo, dual channel or single channel. Dual channel is two independent mono channels.")
    .field_doc(
        "mode_extension",
        "Joint stereo settings, used only when channel_mode is joint stereo. In Layer III they say whether intensity \
         stereo and M/S stereo are on; in Layers I and II they say which subbands use intensity stereo.",
    )
    .field_doc("copyright", "1 means the audio is copyrighted.")
    .field_doc("original", "1 means original media, 0 means a copy.")
    .field_doc("emphasis", "Type of de-emphasis the decoder must apply to the audio. Most files have none.")
    .field_doc(
        "frame_length",
        "Length of the frame in bytes, including the header. Qubero computes it from the bitrate, sample rate and \
         padding bit; it is not stored in the file.",
    )
    .field_doc("crc", "16-bit CRC that a decoder can use to check the frame. Present only when protection_bit is 0.")
    .field_doc(
        "side_info",
        "Layer III side information: where this frame's main data starts, and how the main data for each granule \
         and channel is coded.",
    )
    .field_doc(
        "xing",
        "Xing header in a frame that contains no audio, written by LAME and FFmpeg so that players know the playing \
         time and can seek. In constant bitrate files its ID is \"Info\" instead of \"Xing\".",
    )
    .field_doc(
        "vbri",
        "VBRI header, written by Fraunhofer encoders in a frame that contains no audio. It always starts 36 bytes \
         after the start of the frame.",
    )
    .field_doc(
        "main_data",
        "The part of this frame's main data (scale factors and Huffman-coded samples) that is stored in this frame. \
         The first main_data_begin bytes of the main data are at the end of earlier frames, in the bit reservoir.",
    )
    .field_doc(
        "ancillary_data",
        "Bytes after this frame's main data that are not main data for any frame: ancillary data or encoder padding. \
         LAME writes its version string here, for example \"LAME3.100\".",
    )
    .field_doc(
        "later_main_data",
        "Main data for the next frame or frames, which the encoder stored in spare bytes of this frame (the bit reservoir).",
    )
    .field_doc("audio_data", "Layers I and II: bit allocation, scale factors and samples for this frame.")
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
    let docs = |t: T| {
        t.field_doc(
            "main_data_begin",
            "Number of bytes of this frame's main data that are in earlier frames, not counting the headers and side \
             information of those frames. 0 means the main data starts in this frame.",
        )
        .field_doc("private_bits", "Bits for applications to use.")
        .field_doc(
            "scfsi",
            "MPEG-1 only: 4 bits for each channel, one for each of 4 groups of scale factor bands. A bit set to 1 means \
             granule 1 reuses the scale factors of granule 0 for that group.",
        )
        .field_doc("granules", "Coding settings for each granule and channel: 2 granules in MPEG-1, 1 granule in MPEG-2 and 2.5.")
    };
    T::switch(
        E::field("version"),
        vec![(
            3,
            docs(T::structure(
                "SideInfo",
                vec![
                    ("main_data_begin", bits(9)),
                    ("private_bits", T::switch(mono(), vec![(1, bits(5))], bits(3))),
                    ("scfsi", T::array(bits(4), channels())),
                    ("granules", T::array(T::array(granule(true), channels()), E::lit(2))),
                ],
            )),
        )],
        docs(T::structure(
            "SideInfo",
            vec![
                ("main_data_begin", bits(8)),
                ("private_bits", T::switch(mono(), vec![(1, bits(1))], bits(2))),
                ("granules", T::array(T::array(granule(false), channels()), E::lit(1))),
            ],
        )),
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
        .field_doc("part2_3_length", "Number of bits of scale factors and Huffman-coded data for this granule and channel.")
        .field_doc("big_values", "Number of pairs of Huffman-coded values in the big-values region of this granule and channel.")
        .field_doc("global_gain", "Quantizer step size for this granule and channel.")
        .field_doc("scalefac_compress", "Code for the number of bits the scale factors use.")
        .field_doc(
            "window_switching_flag",
            "1 means this granule and channel use a block type other than normal, and block_type and mixed_block_flag follow.",
        )
        .field_doc("block_type", "Block type for this granule and channel: start, short (3 short windows) or end. Value 0 is reserved.")
        .field_doc("mixed_block_flag", "1 means the lowest bands use long blocks and the rest use short blocks.")
        .field_doc(
            "table_select",
            "Which Huffman table each region of the big-values region uses: 2 regions when window_switching_flag is 1, \
             otherwise 3.",
        )
        .field_doc("subblock_gain", "Gain offset for each of the 3 short windows.")
        .field_doc("region0_count", "Boundary between region 0 and region 1 of the big-values region, in scale factor bands.")
        .field_doc("region1_count", "Boundary between region 1 and region 2 of the big-values region, in scale factor bands.")
        .field_doc("preflag", "MPEG-1 only: 1 adds a preset amount to the scale factors of the high-frequency bands.")
        .field_doc("scalefac_scale", "Step size of the scale factors: 0 means √2 and 1 means 2.")
        .field_doc("count1table_select", "Which of two Huffman tables codes the count1 region, where each value is -1, 0 or 1.")
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
    .field_doc("id", "\"Xing\" in variable bitrate files, \"Info\" in constant bitrate files.")
    .field_doc("flags", "Which of frame_count, byte_count, toc and quality are present.")
    .field_doc("frame_count", "Number of audio frames.")
    .field_doc("byte_count", "Number of bytes of audio frames, including this frame.")
    .field_doc(
        "toc",
        "Seek table of 100 entries. Entry i is the offset into the audio at which i% of the playing time has passed, \
         in units of byte_count / 256.",
    )
    .field_doc(
        "quality",
        "Quality number: Xing defined 0 as best and 100 as worst. LAME writes 100 − 10 × V − q, where V and q are its \
         -V and -q settings.",
    )
    .field_doc("lame", "LAME tag: 36 bytes of encoder settings, ReplayGain values, and the encoder delay and end padding.")
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
const SOURCE_RATE: &[(i128, &str)] = &[(0, "32000 Hz or lower"), (1, "44100 Hz"), (2, "48000 Hz"), (3, "higher than 48000 Hz")];
const SURROUND: &[(i128, &str)] = &[(0, "none"), (1, "Dolby Pro Logic"), (2, "Dolby Pro Logic II"), (3, "Ambisonic")];
const GAIN_NAME: &[(i128, &str)] = &[(0, "not set"), (1, "radio (track gain)"), (2, "audiophile (album gain)")];
const GAIN_ORIGINATOR: &[(i128, &str)] =
    &[(0, "not set"), (1, "set by artist"), (2, "set by user"), (3, "set automatically"), (4, "set from an RMS average")];

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
    .field_doc("name", "Which gain this is: radio (track gain), audiophile (album gain) or not set.")
    .field_doc("originator", "Who or what set this gain.")
    .field_doc("negative", "Sign bit of the gain: 1 means the gain is negative.")
    .field_doc("gain_tenths_db", "Size of the gain adjustment in tenths of a dB. The negative bit gives the sign.")
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
            (
                "encoding_flags",
                T::flags(
                    "LameFlags",
                    bits(4),
                    &[(0, "--nspsytune"), (1, "--nssafejoint"), (2, "continues into the next track"), (3, "continues from the previous track")],
                ),
            ),
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
    .field_doc(
        "encoder",
        "Encoder name and version in 9 characters, such as \"LAME3.100\". FFmpeg writes \"Lavc\" or \"Lavf\" followed by its version.",
    )
    .field_doc("revision", "Revision number of the LAME tag.")
    .field_doc("vbr_method", "Bitrate mode the encoder used, such as constant bitrate, average bitrate (ABR) or one of the VBR methods.")
    .field_doc("lowpass", "Frequency of the encoder's lowpass filter, in units of 100 Hz. 0 means unknown.")
    .field_doc("peak", "Peak signal amplitude, where 1.0 is the largest amplitude the decoded format can store. 0 means unknown.")
    .field_doc("radio_gain", "ReplayGain track gain (radio gain).")
    .field_doc("audiophile_gain", "ReplayGain album gain (audiophile gain).")
    .field_doc(
        "encoding_flags",
        "Flags for the LAME options --nspsytune and --nssafejoint, and for gapless encoding (--nogap) that continues \
         into the next track or from the previous track.",
    )
    .field_doc("ath_type", "Type of LAME's absolute threshold of hearing model.")
    .field_doc(
        "bitrate",
        "Bitrate in kbit/s: the target bitrate for ABR, the bitrate for CBR, and the minimum bitrate for VBR. 255 means \
         255 kbit/s or more.",
    )
    .field_doc(
        "encoder_delay",
        "Number of samples the encoder added at the start of the audio. A player drops these samples to get back the original audio.",
    )
    .field_doc(
        "end_padding",
        "Number of zero samples the encoder added at the end to complete the last frame. A player drops these samples \
         to get back the original audio.",
    )
    .field_doc("source_sample_rate", "Sample rate of the audio before it was encoded.")
    .field_doc("unwise_settings", "1 means the encoder was run with settings that are likely to lower the audio quality.")
    .field_doc("stereo_mode", "Stereo mode the encoder used.")
    .field_doc("noise_shaping", "LAME noise shaping type, 0 to 3.")
    .field_doc(
        "mp3_gain",
        "Gain applied losslessly by a tool such as mp3gain, in steps of 1.5 dB. The value is a sign bit and a magnitude.",
    )
    .field_doc("surround", "Surround sound format of the audio.")
    .field_doc("preset", "LAME preset number used for encoding. 0 means no preset.")
    .field_doc(
        "music_length",
        "Number of bytes from the start of this frame to the end of the last audio frame, when LAME wrote the file. \
         Tags at the end of the file are not counted.",
    )
    .field_doc("music_crc", "CRC-16 of the audio frames after this frame, computed when LAME wrote them.")
    .field_doc("tag_crc", "CRC-16 that protects the LAME tag. The LAME tag specification says it covers the first 190 bytes of this frame.")
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
    .field_doc("before", "Bytes between the side information and the VBRI header.")
    .field_doc("version", "Version of the VBRI header.")
    .field_doc("toc_entries", "Number of entries in toc, the seek table.")
    .field_doc("toc_scale", "Scale factor for the entries in toc.")
    .field_doc("toc_entry_size", "Size of each entry in toc, in bytes.")
    .field_doc("toc", "Seek table: toc_entries entries, each toc_entry_size bytes long.")
}

// The tags at the back, found from the end of the file the way players find
// them: ID3v1 is the last 128 bytes, Lyrics3v2 ends just before it, and APEv2
// ends just before that.

/// A peek at `back` bytes before the end of the file.
fn from_end(back: E, bits: u32, endian: crate::template::Endian) -> E {
    E::PeekIn { at: Box::new(E::SpaceSize.sub(back).mul(E::lit(8))), bits, endian }
}

/// How many bytes the ID3v1 tag takes: 128 when the file ends with one.
fn id3v1_size() -> E {
    let room = E::SpaceSize.greater_or_equal(E::lit(128));
    E::cond(room.both(from_end(E::lit(128), 24, Big).equal_to(E::lit(0x54_41_47))), E::lit(128), E::lit(0))
}

/// How many bytes the Lyrics3v2 tag takes when one ends `after` bytes before
/// the end of the file. Its size is six ASCII digits before the closing
/// `LYRICS200`, and counts from `LYRICSBEGIN` up to those digits.
fn lyrics3_size(after: E) -> E {
    let end = |n: i128| from_end(after.clone().add(E::lit(n)), 8, Big);
    // Each digit times its place, added in pairs rather than one after
    // another: the computed field this is part of is worked out inside other
    // expressions, and a deep tree of them runs out of stack.
    let digit = |i: u32| end(15 - i as i128).sub(E::lit(0x30)).mul(E::lit(10i128.pow(5 - i)));
    let digits = digit(0).add(digit(1)).add(digit(2).add(digit(3))).add(digit(4).add(digit(5)));
    let total = digits.add(E::lit(15));
    let room = || E::SpaceSize.sub(after.clone());
    // `LYRICS20` and then `0`: nine bytes, more than one peek holds.
    let signed = room()
        .greater_or_equal(E::lit(15 + 11))
        .both(from_end(after.clone().add(E::lit(9)), 64, Big).equal_to(E::lit(0x4c_59_52_49_43_53_32_30_i128)).both(end(1).equal_to(E::lit(0x30))));
    let fits = total.clone().less_or_equal(room()).both(total.clone().greater_or_equal(E::lit(15 + 11)));
    let begins = from_end(after.add(total.clone()), 64, Big).equal_to(E::lit(0x4c_59_52_49_43_53_42_45_i128));
    E::cond(signed.both(fits).both(begins), total, E::lit(0))
}

/// How many bytes the APEv2 tag takes when its footer ends `after` bytes
/// before the end of the file. The footer's size counts the items and the
/// footer, and the header in front of them when its flags say there is one.
fn apev2_size(after: E) -> E {
    let room = || E::SpaceSize.sub(after.clone());
    let size = from_end(after.clone().add(E::lit(20)), 32, Little);
    let header = from_end(after.clone().add(E::lit(12)), 32, Little).bit(31).mul(E::lit(32));
    let total = size.clone().add(header);
    let found = room()
        .greater_or_equal(E::lit(32))
        .both(from_end(after.clone().add(E::lit(32)), 64, Big).equal_to(E::lit(APETAGEX)))
        .both(size.greater_or_equal(E::lit(32)))
        .both(total.clone().less_or_equal(room()));
    E::cond(found, total, E::lit(0))
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
    .field_doc("header", "32-byte tag header with the same layout as the footer. Present when the footer's flags say the tag has a header.")
    .field_doc("items", "The tag's items: key and value pairs.")
    .field_doc(
        "footer",
        "32 bytes at the end of the tag, with the same layout as the header. Programs reading from the end of the file \
         find the tag by the \"APETAGEX\" in this footer.",
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
            // Bit 30 set means there is no footer. The Hydrogenaudio wiki's
            // APEv2 page says the opposite, but TagLib reads it this way and
            // writes it clear, and so do the files.
            ("flags", T::flags("ApeFlags", T::u32(Little), &[(0, "read-only"), (29, "this is the header"), (30, "has no footer"), (31, "has a header")])),
            ("reserved", T::bytes(E::lit(8))),
        ],
    )
    .field_doc("version", "APE tag version: 1000 means 1.0 and 2000 means 2.0.")
    .field_doc("tag_size", "Size in bytes of the items and the footer, not counting the header.")
    .field_doc("flags", "Flags for whether the tag has a header, whether it has a footer, whether this is the header, and whether the tag is read-only.")
    .field_doc("reserved", "8 reserved bytes, all zero.")
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
    .field_doc("value_size", "Size of the value in bytes.")
    .field_doc("flags", "Flags for this item: whether it is read-only.")
    .field_doc("content_type", "Type of the item's value: UTF-8 text, binary, or an external locator (a URL or file path).")
    .field_doc("key", "Name of the item: ASCII text ending with a zero byte.")
    .field_doc("value", "The item's value: value_size bytes, of the type given by content_type.")
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
    .field_doc("begin", "The text \"LYRICSBEGIN\", which starts the Lyrics3v2 tag.")
    .field_doc(
        "fields",
        "Each field is a 3-letter ID, a 5-digit size and the text. The IDs are IND (indications), LYR (lyrics), INF \
         (information), AUT (author), EAL, EAR and ETT (extended album, artist and title), and IMG (image links).",
    )
    .field_doc("size", "Size of the tag as 6 digits: the number of bytes from \"LYRICSBEGIN\" to the end of the last field.")
    .field_doc("end", "The text \"LYRICS200\", which ends the Lyrics3v2 tag.")
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
    .field_doc("title", "Title: 30 bytes of ISO-8859-1 text, padded with zero bytes.")
    .field_doc("artist", "Artist: 30 bytes of ISO-8859-1 text, padded with zero bytes.")
    .field_doc("album", "Album: 30 bytes of ISO-8859-1 text, padded with zero bytes.")
    .field_doc("year", "Year as 4 characters of text.")
    .field_doc("comment", "Comment: 30 bytes of text, or 28 bytes in ID3v1.1, where zero_byte and track take the last 2 bytes.")
    .field_doc("zero_byte", "ID3v1.1 only: a zero byte that ends the comment and means the next byte is a track number.")
    .field_doc("track", "ID3v1.1 only: the track number.")
    .field_doc("genre", "Genre, as a number in the ID3v1 genre list. 255 means none.")
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
/// sample rate. Fewer count when the file runs out first: one frame that ends
/// where the file does, or two and then a frame cut off by the end.
fn frames_at(h: &[u8], at: usize, len: u64) -> bool {
    let mut pos = at;
    let mut first = None;
    let mut found = 0;
    while found < 3 {
        if pos as u64 == len {
            return found >= 1;
        }
        if pos as u64 > len {
            return found >= 2;
        }
        if pos + 4 > h.len() {
            // The file goes on past the bytes read, and so may the frames.
            return found >= 1;
        }
        let Some((n, same)) = header_length(&h[pos..]) else { return false };
        if *first.get_or_insert(same) != same {
            return false;
        }
        pos += n;
        found += 1;
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

    /// Where the audio is among the root's fields: after the ID3v2 tag and
    /// the three sizes of the tags at the end.
    const AUDIO: usize = 4;

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
            let frames = ev.node(&d, &[AUDIO, 0]).unwrap();
            assert_eq!(frames.child_count, 2, "{header:08x}");
            let first = ev.node(&d, &[AUDIO, 0, 0]).unwrap();
            assert_eq!(first.size_bits / 8, header_length(&header.to_be_bytes()).unwrap().0 as u64, "{header:08x}");
        }
    }

    /// The path of the child of `path` called `name`.
    fn child(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], name: &str) -> Vec<usize> {
        let n = ev.node(d, path).unwrap().child_count as usize;
        for i in 0..n {
            let mut p = path.to_vec();
            p.push(i);
            if ev.node(d, &p).unwrap().name == name {
                return p;
            }
        }
        panic!("{path:?} has no child {name}");
    }

    /// The value at a path of names from the root.
    fn value(ev: &mut Evaluator, d: &Document<MemSource>, names: &[&str]) -> Value {
        let mut p = Vec::new();
        for name in names {
            p = match name.parse::<usize>() {
                Ok(i) => [p, vec![i]].concat(),
                Err(_) => child(ev, d, &p, name),
            };
        }
        ev.node(d, &p).unwrap().value
    }

    fn size(ev: &mut Evaluator, d: &Document<MemSource>, names: &[&str]) -> u64 {
        let mut p = Vec::new();
        for name in names {
            p = match name.parse::<usize>() {
                Ok(i) => [p, vec![i]].concat(),
                Err(_) => child(ev, d, &p, name),
            };
        }
        ev.node(d, &p).unwrap().size_bits / 8
    }

    fn uint(v: Value) -> u128 {
        match v {
            Value::UInt(n) => n as u128,
            Value::Enum { raw, .. } => raw as u128,
            Value::Int(n) => n as u128,
            other => panic!("not a number: {other:?}"),
        }
    }

    fn name(v: Value) -> String {
        match v {
            Value::Enum { name: Some(n), .. } => n,
            Value::Str(s) => s,
            other => panic!("no name: {other:?}"),
        }
    }

    fn id3v1(comment: &[u8], track: Option<u8>, genre: u8) -> Vec<u8> {
        let field = |s: &[u8], n: usize| {
            let mut v = s.to_vec();
            v.resize(n, 0);
            v
        };
        let mut v = b"TAG".to_vec();
        v.extend(field(b"Title", 30));
        v.extend(field(b"Artist", 30));
        v.extend(field(b"Album", 30));
        v.extend(b"2026");
        match track {
            Some(t) => {
                v.extend(field(comment, 28));
                v.extend([0, t]);
            }
            None => v.extend(field(comment, 30)),
        }
        v.push(genre);
        v
    }

    fn ape_item(key: &str, value: &[u8], flags: u32) -> Vec<u8> {
        let mut v = (value.len() as u32).to_le_bytes().to_vec();
        v.extend(flags.to_le_bytes());
        v.extend(key.as_bytes());
        v.push(0);
        v.extend(value);
        v
    }

    fn apev2(items: &[Vec<u8>], header: bool) -> Vec<u8> {
        let body: Vec<u8> = items.concat();
        let size = body.len() as u32 + 32;
        let frame = |flags: u32| {
            let mut v = b"APETAGEX".to_vec();
            for n in [2000, size, items.len() as u32, flags] {
                v.extend(n.to_le_bytes());
            }
            v.extend([0; 8]);
            v
        };
        let mut v = if header { frame(0xa000_0000) } else { Vec::new() };
        v.extend(body);
        v.extend(frame(if header { 0x8000_0000 } else { 0 }));
        v
    }

    fn lyrics3v2(fields: &[(&str, &[u8])]) -> Vec<u8> {
        let mut v = b"LYRICSBEGIN".to_vec();
        for (id, text) in fields {
            v.extend(id.as_bytes());
            v.extend(format!("{:05}", text.len()).as_bytes());
            v.extend(*text);
        }
        let n = v.len();
        v.extend(format!("{n:06}").as_bytes());
        v.extend(b"LYRICS200");
        v
    }

    /// MPEG-1 Layer III, 128 kbit/s, 44100 Hz, mono: 417 bytes, with 17 bytes
    /// of side information.
    const MONO: u32 = 0xfffb_90c0;

    #[test]
    fn a_header_reads_as_named_fields() {
        let d = Document::new(MemSource(frame_bytes(0xfffb_9264)));
        let mut ev = Evaluator::new(mp3());
        let f = |n: &'static str| vec!["audio", "frames", "0", n];
        assert_eq!(name(value(&mut ev, &d, &f("version"))), "MPEG-1");
        assert_eq!(name(value(&mut ev, &d, &f("layer"))), "Layer III");
        assert_eq!(name(value(&mut ev, &d, &f("bitrate_index"))), "128 kbit/s");
        assert_eq!(name(value(&mut ev, &d, &f("sample_rate_index"))), "44100 Hz");
        assert_eq!(uint(value(&mut ev, &d, &f("padding_bit"))), 1);
        assert_eq!(name(value(&mut ev, &d, &f("channel_mode"))), "joint stereo");
        assert_eq!(name(value(&mut ev, &d, &f("mode_extension"))), "intensity stereo off, M/S stereo on");
        assert_eq!(uint(value(&mut ev, &d, &f("frame_length"))), 418);
        // Joint stereo is two channels: 32 bytes of side information.
        assert_eq!(size(&mut ev, &d, &f("side_info")), 32);
        // The same index means another rate in another version and layer.
        let d = Document::new(MemSource(frame_bytes(0xfff3_9000)));
        let mut ev = Evaluator::new(mp3());
        assert_eq!(name(value(&mut ev, &d, &["audio", "frames", "0", "bitrate_index"])), "80 kbit/s");
        assert_eq!(name(value(&mut ev, &d, &["audio", "frames", "0", "sample_rate_index"])), "22050 Hz");
        let d = Document::new(MemSource(frame_bytes(0xffe3_1800)));
        let mut ev = Evaluator::new(mp3());
        assert_eq!(name(value(&mut ev, &d, &["audio", "frames", "0", "version"])), "MPEG-2.5");
        assert_eq!(name(value(&mut ev, &d, &["audio", "frames", "0", "sample_rate_index"])), "8000 Hz");
    }

    #[test]
    fn a_protected_frame_has_a_crc_after_its_header() {
        // MPEG-1 Layer II, 192 kbit/s, 48000 Hz, protection bit 0.
        let d = Document::new(MemSource([frame_bytes(0xfffc_a400), frame_bytes(0xfffc_a400)].concat()));
        let mut ev = Evaluator::new(mp3());
        assert_eq!(size(&mut ev, &d, &["audio", "frames", "0", "crc"]), 2);
        assert_eq!(name(value(&mut ev, &d, &["audio", "frames", "0", "protection_bit"])), "CRC follows");
        // Layer II has no side information, so the rest is one run.
        assert_eq!(size(&mut ev, &d, &["audio", "frames", "0", "audio_data"]), 576 - 6);
        assert_eq!(size(&mut ev, &d, &["audio", "frames", "0", "side_info"]), 0);
    }

    #[test]
    fn layer_3_main_data_is_split_where_the_side_information_says() {
        // Two mono frames. The first has 100 bytes of main data in two
        // granules; the second's main data begins 30 bytes back, in the first.
        let side = |begin: u64, parts: [u64; 2]| {
            // main_data_begin (9), private_bits (5), scfsi (4), then per
            // granule part2_3_length (12) and 47 bits of zeros.
            let mut bits: Vec<bool> = Vec::new();
            let mut put = |v: u64, n: u32| (0..n).rev().for_each(|i| bits.push(v >> i & 1 == 1));
            put(begin, 9);
            put(0, 9);
            for p in parts {
                put(p, 12);
                put(0, 47);
            }
            bits.chunks(8).map(|c| c.iter().fold(0u8, |a, b| a << 1 | *b as u8)).collect::<Vec<u8>>()
        };
        let mut first = frame_bytes(MONO);
        first[4..21].copy_from_slice(&side(0, [400, 400]));
        let mut second = frame_bytes(MONO);
        second[4..21].copy_from_slice(&side(30, [200, 0]));
        let d = Document::new(MemSource([first, second].concat()));
        let mut ev = Evaluator::new(mp3());
        let f = |i: &'static str, n: &'static str| vec!["audio", "frames", i, n];
        assert_eq!(uint(value(&mut ev, &d, &["audio", "frames", "1", "side_info", "main_data_begin"])), 30);
        assert_eq!(size(&mut ev, &d, &f("0", "main_data")), 100);
        // 417 bytes, less the header, the side information, the main data and
        // the 30 bytes the next frame starts its main data in.
        assert_eq!(size(&mut ev, &d, &f("0", "ancillary_data")), 417 - 4 - 17 - 100 - 30);
        assert_eq!(size(&mut ev, &d, &f("0", "later_main_data")), 30);
        // 200 bits is 25 bytes, and 30 of them were in the frame before.
        assert_eq!(size(&mut ev, &d, &f("1", "main_data")), 0);
        assert_eq!(size(&mut ev, &d, &f("1", "ancillary_data")), 417 - 4 - 17);
    }

    #[test]
    fn a_xing_header_reads_only_the_fields_its_flags_name() {
        let mut frame = frame_bytes(MONO);
        let mut xing = b"Xing".to_vec();
        xing.extend(3u32.to_be_bytes()); // frame count and byte count
        xing.extend(100u32.to_be_bytes());
        xing.extend(41_700u32.to_be_bytes());
        let mut lame = b"LAME3.100".to_vec();
        lame.push(0x13); // revision 1, VBR method 3
        lame.push(195); // lowpass 19500 Hz
        lame.extend(0.5f32.to_be_bytes());
        lame.extend([0x2c, 0x1e]); // radio: name 1, originator 3, +3.0 dB
        lame.extend([0, 0]);
        lame.push(0x03); // no flags, ATH type 3
        lame.push(128);
        lame.extend([0x24, 0x04, 0xb0]); // delay 576, padding 1200
        lame.push(0b01_0_011_00); // 44.1 kHz source, joint stereo
        lame.push(0x83); // MP3 gain -3: sign and magnitude
        lame.extend([0, 0]);
        lame.extend(41_700u32.to_be_bytes());
        lame.extend([0, 0, 0, 0]);
        xing.extend(lame);
        frame[21..21 + xing.len()].copy_from_slice(&xing);
        let d = Document::new(MemSource([frame, frame_bytes(MONO)].concat()));
        let mut ev = Evaluator::new(mp3());
        let x = |n: &'static str| vec!["audio", "frames", "0", "xing", n];
        assert_eq!(uint(value(&mut ev, &d, &x("frame_count"))), 100);
        assert_eq!(uint(value(&mut ev, &d, &x("byte_count"))), 41_700);
        assert_eq!(size(&mut ev, &d, &x("toc")), 0);
        assert_eq!(size(&mut ev, &d, &x("quality")), 0);
        let l = |n: &'static str| vec!["audio", "frames", "0", "xing", "lame", n];
        assert_eq!(name(value(&mut ev, &d, &l("encoder"))), "LAME3.100");
        assert_eq!(name(value(&mut ev, &d, &l("vbr_method"))), "VBR method 1");
        assert_eq!(value(&mut ev, &d, &l("peak")), Value::Float(0.5));
        assert_eq!(name(value(&mut ev, &d, &["audio", "frames", "0", "xing", "lame", "radio_gain", "name"])), "radio (track gain)");
        assert_eq!(uint(value(&mut ev, &d, &["audio", "frames", "0", "xing", "lame", "radio_gain", "gain_tenths_db"])), 30);
        assert_eq!(uint(value(&mut ev, &d, &l("encoder_delay"))), 576);
        assert_eq!(uint(value(&mut ev, &d, &l("end_padding"))), 1200);
        assert_eq!(name(value(&mut ev, &d, &l("stereo_mode"))), "joint stereo");
        assert_eq!(name(value(&mut ev, &d, &l("source_sample_rate"))), "44100 Hz");
        assert_eq!(value(&mut ev, &d, &l("mp3_gain")), Value::Int(-3));
        assert_eq!(uint(value(&mut ev, &d, &l("music_length"))), 41_700);
        assert_eq!(size(&mut ev, &d, &["audio", "frames", "1", "xing"]), 0);
    }

    #[test]
    fn a_vbri_header_is_32_bytes_after_the_frame_header() {
        let mut frame = frame_bytes(MONO);
        let mut vbri = b"VBRI".to_vec();
        for n in [1u16, 0x0f00, 75] {
            vbri.extend(n.to_be_bytes());
        }
        vbri.extend(41_700u32.to_be_bytes());
        vbri.extend(100u32.to_be_bytes());
        for n in [2u16, 1, 2, 50, 0x0100, 0x0200] {
            vbri.extend(n.to_be_bytes());
        }
        frame[36..36 + vbri.len()].copy_from_slice(&vbri);
        let d = Document::new(MemSource([frame, frame_bytes(MONO)].concat()));
        let mut ev = Evaluator::new(mp3());
        let v = |n: &'static str| vec!["audio", "frames", "0", "vbri", n];
        // Mono side information ends 21 bytes in, 15 short of the header.
        assert_eq!(size(&mut ev, &d, &v("before")), 15);
        assert_eq!(uint(value(&mut ev, &d, &v("frame_count"))), 100);
        assert_eq!(uint(value(&mut ev, &d, &["audio", "frames", "0", "vbri", "toc", "1"])), 0x0200);
        assert_eq!(size(&mut ev, &d, &["audio", "frames", "0", "xing"]), 0);
    }

    #[test]
    fn tags_at_the_end_are_found_from_the_end() {
        let frames = [frame_bytes(MONO), frame_bytes(MONO)].concat();
        let ape = apev2(&[ape_item("Title", "Caf\u{e9}".as_bytes(), 0), ape_item("Cover Art (Front)", b"a.png\0\x89PNG", 0b011)], true);
        let lyrics = lyrics3v2(&[("IND", b"100"), ("LYR", b"[00:01]la la")]);
        let file = [frames.clone(), ape, lyrics, id3v1(b"A comment", Some(9), 17)].concat();
        let d = Document::new(MemSource(file));
        let mut ev = Evaluator::new(mp3());
        assert_eq!(ev.node(&d, &[AUDIO, 0]).unwrap().child_count, 2);
        assert_eq!(size(&mut ev, &d, &["audio", "not_frames"]), 0);
        assert_eq!(name(value(&mut ev, &d, &["apev2", "items", "0", "value"])), "Caf\u{e9}");
        assert_eq!(name(value(&mut ev, &d, &["apev2", "items", "1", "content_type"])), "binary");
        assert_eq!(name(value(&mut ev, &d, &["apev2", "items", "1", "key"])), "Cover Art (Front)");
        assert_eq!(uint(value(&mut ev, &d, &["apev2", "footer", "item_count"])), 2);
        assert_eq!(name(value(&mut ev, &d, &["lyrics3", "fields", "1", "id"])), "LYR");
        assert_eq!(name(value(&mut ev, &d, &["lyrics3", "fields", "1", "value"])), "[00:01]la la");
        assert_eq!(name(value(&mut ev, &d, &["id3v1", "comment"])), "A comment");
        assert_eq!(uint(value(&mut ev, &d, &["id3v1", "track"])), 9);
        assert_eq!(name(value(&mut ev, &d, &["id3v1", "genre"])), "Rock");

        // Each field of the root read first, by an evaluator that has read
        // nothing else: the tag sizes are worked out inside one another then,
        // and that has to stay within the stack a read may use.
        for first in 0..8 {
            let mut ev = Evaluator::new(mp3());
            ev.node(&d, &[first]).unwrap_or_else(|e| panic!("root field {first} read first: {e:?}"));
            read_all(&mut ev, &d, &[first], 3);
        }

        // An APEv2 tag with no header, and an ID3v1.0 tag with no track.
        let ape = apev2(&[ape_item("Artist", b"Someone", 0)], false);
        let file = [frames.clone(), ape, id3v1(b"A comment thirty bytes long...", None, 255)].concat();
        let d = Document::new(MemSource(file));
        let mut ev = Evaluator::new(mp3());
        assert_eq!(size(&mut ev, &d, &["apev2", "header"]), 0);
        assert_eq!(name(value(&mut ev, &d, &["apev2", "items", "0", "key"])), "Artist");
        assert_eq!(size(&mut ev, &d, &["lyrics3"]), 0);
        assert_eq!(name(value(&mut ev, &d, &["id3v1", "comment"])), "A comment thirty bytes long...");
        assert_eq!(size(&mut ev, &d, &["id3v1", "track"]), 0);
        assert_eq!(name(value(&mut ev, &d, &["id3v1", "genre"])), "none");
    }

    #[test]
    fn bytes_that_are_not_a_frame_are_named_and_the_tags_still_found() {
        let file = [frame_bytes(MONO), b"junk!".to_vec(), id3v1(b"", None, 0)].concat();
        let d = Document::new(MemSource(file));
        let mut ev = Evaluator::new(mp3());
        assert_eq!(ev.node(&d, &[AUDIO, 0]).unwrap().child_count, 1);
        assert_eq!(size(&mut ev, &d, &["audio", "not_frames"]), 5);
        assert_eq!(size(&mut ev, &d, &["id3v1"]), 128);
    }

    #[test]
    fn zero_bytes_and_a_second_id3_tag_between_frames_are_read() {
        let tag = |n: usize| {
            let mut v = b"ID3\x03\x00\x00\x00\x00\x00".to_vec();
            v.push(n as u8);
            v.extend(vec![0; n]);
            v
        };
        let file = [tag(20), vec![0; 7], frame_bytes(MONO), tag(0), frame_bytes(MONO)].concat();
        let d = Document::new(MemSource(file));
        let mut ev = Evaluator::new(mp3());
        assert_eq!(size(&mut ev, &d, &["id3v2", "padding"]), 20);
        let frames = ev.node(&d, &[AUDIO, 0]).unwrap().child_count;
        assert_eq!(frames, 4);
        let kinds: Vec<String> = (0..4).map(|i| ev.node(&d, &[AUDIO, 0, i]).unwrap().type_name).collect();
        assert_eq!(kinds, ["Padding", "Frame", "ID3", "Frame"]);
    }

    /// Every node under `path`, read, to a depth that reaches the granules.
    fn read_all(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], depth: usize) {
        let n = ev.node(d, path).unwrap_or_else(|e| panic!("{path:?}: {e:?}"));
        if depth == 0 {
            return;
        }
        for i in 0..n.child_count as usize {
            read_all(ev, d, &[path, &[i]].concat(), depth - 1);
        }
    }

    #[test]
    fn a_frame_cut_off_by_the_end_of_the_file_is_left_out_of_the_frames() {
        let mut xing = frame_bytes(MONO);
        xing[21..29].copy_from_slice(b"Info\0\0\0\x0f");
        for first in [frame_bytes(MONO), xing] {
            for cut in [1, 3, 5, 10, 30, 100, 400] {
                let file = [first.clone(), first[..cut].to_vec()].concat();
                let d = Document::new(MemSource(file));
                let mut ev = Evaluator::new(mp3());
                read_all(&mut ev, &d, &[], 8);
                assert_eq!(ev.node(&d, &[AUDIO, 0]).unwrap().child_count, 1, "cut at {cut}");
                assert_eq!(size(&mut ev, &d, &["audio", "not_frames"]), cut as u64, "cut at {cut}");
            }
        }
        // A file that is only the start of a frame.
        let d = Document::new(MemSource(frame_bytes(MONO)[..40].to_vec()));
        let mut ev = Evaluator::new(mp3());
        read_all(&mut ev, &d, &[], 8);
        assert_eq!(size(&mut ev, &d, &["audio", "not_frames"]), 40);
    }

    /// An ID3v2 tag header of the given version and flags around `body`.
    fn id3v2(version: u8, flags: u8, body: &[u8]) -> Vec<u8> {
        let n = body.len();
        let mut v = vec![b'I', b'D', b'3', version, 0, flags];
        v.extend([(n >> 21) as u8 & 0x7f, (n >> 14) as u8 & 0x7f, (n >> 7) as u8 & 0x7f, n as u8 & 0x7f]);
        v.extend(body);
        v
    }

    #[test]
    fn the_audio_starts_where_the_id3v2_header_says_whatever_its_frames_read_as() {
        let frames = [frame_bytes(MONO), frame_bytes(MONO)].concat();

        // Version 2.2: three-letter IDs and 24-bit sizes.
        let mut body = b"TT2\0\0\x03\0Hi".to_vec();
        body.extend([0; 20]);
        let d = Document::new(MemSource([id3v2(2, 0, &body), frames.clone()].concat()));
        let mut ev = Evaluator::new(mp3());
        assert_eq!(ev.node(&d, &[AUDIO]).unwrap().offset_bits / 8, 10 + body.len() as u64);
        assert_eq!(ev.node(&d, &[AUDIO, 0]).unwrap().child_count, 2);
        assert_eq!(name(value(&mut ev, &d, &["id3v2", "frames", "0", "id"])), "TT2");
        assert_eq!(name(value(&mut ev, &d, &["id3v2", "frames", "0", "body", "text"])), "Hi");
        assert_eq!(size(&mut ev, &d, &["id3v2", "padding"]), 20);

        // Version 2.3 with an extended header: 6 bytes of flags and padding size.
        let mut body = vec![0, 0, 0, 6, 0, 0, 0, 0, 0, 0];
        body.extend(b"TIT2\0\0\0\x03\0\0\0Hi");
        let d = Document::new(MemSource([id3v2(3, 0x40, &body), frames.clone()].concat()));
        let mut ev = Evaluator::new(mp3());
        assert_eq!(size(&mut ev, &d, &["id3v2", "extended_header"]), 10);
        assert_eq!(name(value(&mut ev, &d, &["id3v2", "frames", "0", "body", "text"])), "Hi");
        assert_eq!(ev.node(&d, &[AUDIO, 0]).unwrap().child_count, 2);

        // A frame whose size runs past the end of the tag stops at the end of
        // the tag, and the audio is still where the header says.
        let body = b"TIT2\0\0\x10\0\0\0\x03Hi".to_vec();
        let d = Document::new(MemSource([id3v2(3, 0, &body), frames].concat()));
        let mut ev = Evaluator::new(mp3());
        read_all(&mut ev, &d, &[0], 6);
        assert_eq!(ev.node(&d, &[AUDIO]).unwrap().offset_bits / 8, 10 + body.len() as u64);
        assert_eq!(ev.node(&d, &[AUDIO, 0]).unwrap().child_count, 2);
    }

    #[test]
    fn the_recogniser_wants_frames_that_follow_one_another() {
        let frames = [frame_bytes(MONO), frame_bytes(MONO), frame_bytes(MONO)].concat();
        assert!(is_mpeg_audio(&frames, frames.len() as u64));
        // One header and then text is not a stream of frames.
        let mut one = frame_bytes(MONO);
        one.extend(b"not another frame header");
        assert!(!is_mpeg_audio(&one, one.len() as u64));
        // UTF-16 text opens with FF FE, which is a sync and an MPEG-1 Layer I
        // header, and text after it is not a second header.
        let text: Vec<u8> = [0xff, 0xfe].into_iter().chain("Hello, world".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        assert!(!is_mpeg_audio(&text, text.len() as u64));

        let mut tagged = b"ID3\x04\x00\x00\x00\x00\x00\x0a".to_vec();
        tagged.extend([0; 10]);
        let mp3 = [tagged.clone(), frames.clone()].concat();
        assert!(is_tagged_mp3(&mp3, mp3.len() as u64));
        // A tag in front of FLAC is a tag in front of something else.
        let flac = [tagged.clone(), b"fLaC\0\0\0\x22".to_vec()].concat();
        assert!(!is_tagged_mp3(&flac, flac.len() as u64));
        // A tag longer than the bytes read, as cover art makes one, is taken
        // for an MP3 on the tag alone.
        let mut big = b"ID3\x03\x00\x00\x00\x01\x00\x00".to_vec();
        big.extend([0; 100]);
        assert!(is_tagged_mp3(&big, 20_000));
    }
}
