//! FLAC: the four bytes `fLaC`, the metadata blocks, and the audio frames.
//!
//! Each metadata block starts with one byte holding a last-block flag and a
//! seven-bit block type, then a 24-bit big-endian length. The blocks go on
//! until one sets the flag. STREAMINFO comes first and says how the audio is
//! encoded: block sizes, sample rate, channels, bits per sample and the total
//! number of samples, packed into bit fields. The layouts here are the ones in
//! RFC 9639, sections 8 and 9.
//!
//! The Vorbis comment block is the one place FLAC writes little-endian
//! numbers, because it is the same structure Ogg Vorbis and Opus use for their
//! tags. [`comment_fields`] is that structure, and `ogg.rs` uses it too.
//!
//! **The audio frames are one run of bytes.** A frame has no length field: it
//! ends where the next frame's sync code starts. The two sync bytes, `FF F8`
//! or `FF F9`, also occur inside frames, so the next occurrence is often not a
//! frame boundary. In a 14 MB bat recording there are about 3,800 such pairs
//! and 2,344 frames. A decoder tells the real ones apart by checking the
//! header fields and the header's CRC-8, and [`Expr::ToMarker`] cannot do
//! that, so the frames are not divided. Inside an Ogg file each packet is one
//! frame, and there [`frame`] reads them one by one.
//!
//! [`Expr::ToMarker`]: crate::template::Expr::ToMarker

use crate::template::{Check, Checksum, Covers, Encoding, Endian::*, Expr as E, Named, Part, StrLen, Template, Ty as T, Until, Valid};

/// What a FLAC file starts with.
pub const MAGIC: &[u8] = b"fLaC";

/// The metadata block types. 127 is forbidden, so that a block header can
/// never look like the start of a frame; 7 to 126 are reserved.
const BLOCK_TYPE: &[(i128, &str)] = &[
    (0, "STREAMINFO"),
    (1, "PADDING"),
    (2, "APPLICATION"),
    (3, "SEEKTABLE"),
    (4, "VORBIS_COMMENT"),
    (5, "CUESHEET"),
    (6, "PICTURE"),
];

/// What a PICTURE block says the picture is. The same list as ID3v2's APIC
/// frame.
const PICTURE_TYPE: &[(i128, &str)] = &[
    (0, "other"),
    (1, "32x32 PNG file icon"),
    (2, "other file icon"),
    (3, "front cover"),
    (4, "back cover"),
    (5, "leaflet page"),
    (6, "media (label side of a CD)"),
    (7, "lead artist"),
    (8, "artist"),
    (9, "conductor"),
    (10, "band or orchestra"),
    (11, "composer"),
    (12, "lyricist"),
    (13, "recording location"),
    (14, "during recording"),
    (15, "during performance"),
    (16, "screen capture"),
    (17, "bright coloured fish"),
    (18, "illustration"),
    (19, "band or artist logo"),
    (20, "publisher or studio logo"),
];

/// A frame header's four block size bits. 6 and 7 say the size is written
/// out later in the header, as the size minus 1.
const BLOCK_SIZE: &[(i128, &str)] = &[
    (0, "reserved"),
    (1, "192 samples"),
    (2, "576 samples"),
    (3, "1152 samples"),
    (4, "2304 samples"),
    (5, "4608 samples"),
    (6, "in block_size_minus1 (8 bits)"),
    (7, "in block_size_minus1 (16 bits)"),
    (8, "256 samples"),
    (9, "512 samples"),
    (10, "1024 samples"),
    (11, "2048 samples"),
    (12, "4096 samples"),
    (13, "8192 samples"),
    (14, "16384 samples"),
    (15, "32768 samples"),
];

/// A frame header's four sample rate bits. 12 to 14 say the rate is written
/// out later in the header.
const SAMPLE_RATE: &[(i128, &str)] = &[
    (0, "from STREAMINFO"),
    (1, "88.2 kHz"),
    (2, "176.4 kHz"),
    (3, "192 kHz"),
    (4, "8 kHz"),
    (5, "16 kHz"),
    (6, "22.05 kHz"),
    (7, "24 kHz"),
    (8, "32 kHz"),
    (9, "44.1 kHz"),
    (10, "48 kHz"),
    (11, "96 kHz"),
    (12, "in sample_rate_khz"),
    (13, "in sample_rate_hz"),
    (14, "in sample_rate_tens_of_hz"),
    (15, "forbidden"),
];

/// A frame header's four channel bits: 0 to 7 are 1 to 8 independent
/// channels, and 8 to 10 are stereo with one channel coded as the difference
/// of the two.
const CHANNELS: &[(i128, &str)] = &[
    (0, "mono"),
    (1, "left, right"),
    (2, "left, right, centre"),
    (3, "4 channels"),
    (4, "5 channels"),
    (5, "5.1 channels"),
    (6, "6.1 channels"),
    (7, "7.1 channels"),
    (8, "left, side"),
    (9, "side, right"),
    (10, "mid, side"),
];

/// A frame header's three bits per sample bits.
const SAMPLE_SIZE: &[(i128, &str)] = &[
    (0, "from STREAMINFO"),
    (1, "8 bits"),
    (2, "12 bits"),
    (3, "reserved"),
    (4, "16 bits"),
    (5, "20 bits"),
    (6, "24 bits"),
    (7, "32 bits"),
];

/// The first eight bytes of every PNG, read as one big-endian number.
const PNG_SIGNATURE: i128 = 0x8950_4e47_0d0a_1a0a;

/// An unsigned field `n` bits wide, read from the top bit down, which is how
/// FLAC packs everything but the Vorbis comment block.
fn bits(n: u32) -> T {
    T::UInt { bits: n, endian: Big }
}

pub fn flac() -> Template {
    let part = part();
    Template::new("flac", part.root.clone()).with_part(&part)
}

/// The whole file, and the named types its pictures refer to: an embedded
/// PNG or JPEG brings its EXIF and ICC readers with it.
pub(crate) fn part() -> Part {
    let root = T::structure(
        "FLAC",
        vec![("magic", T::magic(MAGIC)), ("metadata_blocks", metadata_blocks()), ("frames", T::bytes(E::Remaining))],
    )
    .field_doc(
        "frames",
        "All the audio frames, as one run of bytes. Each frame is a header, one subframe per channel, and a CRC-16. Frames have no length field, and the sync code that starts a frame can also occur inside a frame, so the frames are not split here.",
    );
    let mut part = Part::new(root);
    for borrowed in picture_parts() {
        part.types.extend(borrowed.types);
    }
    part
}

/// The image formats a PICTURE block's data is read as, with the names each
/// one refers to.
fn picture_parts() -> Vec<Part> {
    let jpeg = super::jpeg::jpeg();
    vec![super::png::part(), Part { root: jpeg.root, types: jpeg.types.into_iter().collect() }]
}

/// Metadata blocks until the one that says it is the last.
pub(crate) fn metadata_blocks() -> T {
    T::repeat(metadata_block(), Until::FieldValue { field: "last_block".into(), value: 1 })
}

/// One metadata block: the header byte, the length, and the block's data read
/// by its type. An Ogg FLAC stream carries one of these in each header packet.
pub(crate) fn metadata_block() -> T {
    let png = super::png::image();
    let jpeg = super::jpeg::jpeg().root;
    T::structure_named(
        "MetadataBlock",
        "block_type",
        "data",
        vec![
            ("last_block", bits(1)),
            ("block_type", T::enumeration("FlacBlockType", bits(7), BLOCK_TYPE)),
            ("length", bits(24)),
            (
                "data",
                T::sized(
                    E::field("length"),
                    T::switch(
                        E::field("block_type"),
                        vec![
                            (0, stream_info()),
                            (1, T::bytes(E::Remaining)),
                            (2, application()),
                            (3, seek_table()),
                            (4, vorbis_comment()),
                            (5, cue_sheet()),
                            (6, picture(png, jpeg)),
                        ],
                        T::bytes(E::Remaining),
                    ),
                ),
            ),
        ],
    )
    .counted_as("block")
    .field_doc("last_block", "1 if this is the last metadata block, and the audio frames come next.")
    .field_doc("length", "Length of the block's data in bytes, not counting the 4-byte block header.")
    .field_valid("block_type", Valid::Expr { expr: E::This.not_equal(E::lit(127)), msg: Some("Block type 127 is forbidden.".into()) })
}

/// STREAMINFO: how the audio is encoded, in bit fields. Where the file stores
/// a number minus 1, the number itself follows as a computed field.
fn stream_info() -> T {
    let known = |f: &str| E::field(f).greater_than(E::lit(0));
    T::structure(
        "StreamInfo",
        vec![
            ("min_block_size", T::u16(Big)),
            ("max_block_size", T::u16(Big)),
            ("min_frame_size", T::unset_int(bits(24), 0)),
            ("max_frame_size", T::unset_int(bits(24), 0)),
            ("sample_rate", bits(20)),
            ("channels_minus1", bits(3)),
            ("channels", T::computed(E::field("channels_minus1").add(E::lit(1)))),
            ("bits_per_sample_minus1", bits(5)),
            ("bits_per_sample", T::computed(E::field("bits_per_sample_minus1").add(E::lit(1)))),
            ("total_samples", T::unset_int(bits(36), 0)),
            (
                "duration",
                T::when(
                    known("total_samples").both(known("sample_rate")),
                    T::computed_real(E::field("total_samples").div(E::field("sample_rate"))),
                ),
            ),
            ("md5", T::bytes(E::lit(16))),
        ],
    )
    .reads_as(&[
        ("sample_rate", "{} Hz", ""),
        ("channels", "{} channels", ""),
        ("bits_per_sample", "{}-bit", ""),
        ("duration", "{} s", ""),
    ])
    .field_doc("min_block_size", "Fewest samples per channel in any frame except the last.")
    .field_doc("max_block_size", "Most samples per channel in any frame.")
    .field_doc("min_frame_size", "Size of the smallest frame in bytes. Unset (0) if unknown.")
    .field_doc("max_frame_size", "Size of the largest frame in bytes. Unset (0) if unknown.")
    .field_doc("sample_rate", "Samples per second, per channel.")
    .field_doc("channels_minus1", "Number of channels minus 1.")
    .field_doc("bits_per_sample_minus1", "Bits per sample minus 1.")
    .field_doc("total_samples", "Samples per channel in the whole stream. Unset (0) if unknown.")
    .field_doc("duration", "Length of the audio in seconds: total_samples divided by sample_rate.")
    .field_doc("md5", "MD5 of the decoded audio: the samples interleaved, signed and little-endian. All zeros if the encoder did not compute it.")
}

/// APPLICATION: a registered four-byte ID, then whatever that application
/// stores.
fn application() -> T {
    T::structure(
        "Application",
        vec![("application_id", T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii)), ("data", T::bytes(E::Remaining))],
    )
    .field_doc("application_id", "ID of the application that defines this block's data. Usually four ASCII characters, registered with IANA.")
}

/// SEEKTABLE: points that say where frames start, so a player can seek
/// without scanning the frames.
fn seek_table() -> T {
    let point = T::structure(
        "SeekPoint",
        vec![
            ("sample_number", T::unset_int(T::u64(Big), u64::MAX as i128)),
            ("stream_offset", T::u64(Big)),
            ("frame_samples", T::u16(Big)),
        ],
    )
    .counted_as("seek point")
    .positions(&["stream_offset"])
    .reads_as(&[("sample_number", "sample {}", ""), ("stream_offset", "offset {}", ""), ("frame_samples", "{} samples", "")])
    .field_doc("sample_number", "Number of the first sample in the target frame. Unset (all ones) for a placeholder point, which reserves space for a seek point added later.")
    .field_doc("stream_offset", "Where the target frame starts, in bytes from the start of the first frame.")
    .field_doc("frame_samples", "Samples per channel in the target frame.");
    T::structure_named("SeekTable", "", "points", vec![("points", T::array(point, E::Remaining.div(E::lit(18))))])
}

/// VORBIS_COMMENT: the tags, in the layout Ogg Vorbis and Opus use too.
pub(crate) fn vorbis_comment() -> T {
    T::structure("VorbisComment", comment_fields())
}

/// The fields of a Vorbis comment list: a vendor string and `NAME=value`
/// comments, each behind a 32-bit little-endian length. FLAC's
/// VORBIS_COMMENT block is exactly these. The Vorbis comment header and
/// OpusTags put a signature in front and may add bytes after, so each of them
/// builds its own structure around these fields.
pub(crate) fn comment_fields() -> Vec<(&'static str, T)> {
    vec![
        ("vendor_length", T::u32(Little)),
        ("vendor", T::utf8(E::field("vendor_length"))),
        ("comment_count", T::u32(Little)),
        // At most one comment for every four bytes left, since each starts
        // with a four-byte length. A count past that is a damaged block, and
        // reading that many would only repeat the same error.
        ("comments", T::array(user_comment(), E::field("comment_count").at_most(E::Remaining.div(E::lit(4))))),
    ]
}

/// One comment: its length, then `NAME=value` in UTF-8. The name is ASCII and
/// case-insensitive.
fn user_comment() -> T {
    let text = T::structure_named(
        "CommentText",
        "name",
        "",
        vec![
            ("name", T::text(StrLen::Terminated { end: b'=', or_end: true }, Encoding::Ascii)),
            ("value", T::utf8(E::Remaining)),
        ],
    );
    T::structure_named("UserComment", "text.name", "text", vec![("length", T::u32(Little)), ("text", T::sized(E::field("length"), text))])
        .counted_as("comment")
}

/// CUESHEET: the track layout of a CD, or of any other medium with tracks.
fn cue_sheet() -> T {
    let index = T::structure(
        "CueSheetIndex",
        vec![("offset", T::u64(Big)), ("index_number", T::u8()), ("reserved", T::bytes(E::lit(3)))],
    )
    .counted_as("index point")
    .reads_as(&[("index_number", "index {}", ""), ("offset", "at sample {}", "")])
    .field_doc("offset", "Where the index point is, in samples from the start of the track.");
    let track = T::structure_named(
        "CueSheetTrack",
        "track_number",
        "",
        vec![
            ("offset", T::u64(Big)),
            ("track_number", T::u8()),
            ("isrc", T::text(StrLen::Padded { size: E::lit(12), pad: 0 }, Encoding::Ascii)),
            ("track_type", T::enumeration("TrackType", bits(1), &[(0, "audio"), (1, "non-audio")])),
            ("pre_emphasis", bits(1)),
            ("reserved_bits", bits(6)),
            ("reserved", T::bytes(E::lit(13))),
            ("index_count", T::u8()),
            ("indexes", T::array(index, E::field("index_count"))),
        ],
    )
    .counted_as("track")
    .field_doc("offset", "Where the track starts, in samples from the start of the audio.")
    .field_doc("track_number", "The track's number. The lead-out track is number 170 on a CD and 255 otherwise.")
    .field_doc("isrc", "The track's International Standard Recording Code (ISRC). Empty if the track has none.")
    .field_doc("pre_emphasis", "1 if the audio was recorded with pre-emphasis.");
    T::structure(
        "CueSheet",
        vec![
            ("media_catalog_number", T::text(StrLen::Padded { size: E::lit(128), pad: 0 }, Encoding::Ascii)),
            ("lead_in_samples", T::u64(Big)),
            ("is_cd", bits(1)),
            ("reserved_bits", bits(7)),
            ("reserved", T::bytes(E::lit(258))),
            ("track_count", T::u8()),
            ("tracks", T::array(track, E::field("track_count"))),
        ],
    )
    .field_doc("media_catalog_number", "The disc's catalogue number in ASCII, padded with zero bytes.")
    .field_doc("lead_in_samples", "Samples in the lead-in before the first track. Only meaningful for a CD.")
    .field_doc("is_cd", "1 if the cue sheet describes a CD.")
    .field_doc("track_count", "Number of tracks, including the lead-out track.")
}

/// PICTURE: an image, usually cover art, with its MIME type and description.
/// The data is read as a PNG or JPEG when it starts like one.
fn picture(png: T, jpeg: T) -> T {
    let starts_with = |bytes: u32, value: i128| E::Remaining.greater_or_equal(E::lit(i128::from(bytes))).both(E::peek(bytes * 8, Big).equal_to(E::lit(value)));
    // "-->" as a MIME type says the data is the address of the picture
    // rather than the picture.
    let is_link = E::field("mime_type_length").equal_to(E::lit(3)).both(E::field("mime_type").equal_to(E::lit(0x2d2d3e)));
    let kind = E::cond(is_link, E::lit(3), E::cond(starts_with(8, PNG_SIGNATURE), E::lit(1), E::cond(starts_with(3, 0xff_d8ff), E::lit(2), E::lit(0))));
    T::structure(
        "Picture",
        vec![
            ("picture_type", T::enumeration("PictureType", T::u32(Big), PICTURE_TYPE)),
            ("mime_type_length", T::u32(Big)),
            ("mime_type", T::text(StrLen::Fixed(E::field("mime_type_length")), Encoding::Ascii)),
            ("description_length", T::u32(Big)),
            ("description", T::utf8(E::field("description_length"))),
            ("width", T::u32(Big)),
            ("height", T::u32(Big)),
            ("color_depth", T::u32(Big)),
            ("color_count", T::u32(Big)),
            ("data_length", T::u32(Big)),
            (
                "data",
                T::sized(
                    E::field("data_length"),
                    T::switch(kind, vec![(1, png), (2, jpeg), (3, T::utf8(E::Remaining))], T::bytes(E::Remaining)),
                ),
            ),
        ],
    )
    .reads_as(&[("picture_type", "", ""), ("mime_type", "", "")])
    .field_doc("mime_type", "The image's MIME type, such as image/png. \"-->\" means the picture data is a URL.")
    .field_doc("width", "Image width in pixels. 0 if unknown.")
    .field_doc("height", "Image height in pixels. 0 if unknown.")
    .field_doc("color_depth", "Bits per pixel. 0 if unknown.")
    .field_doc("color_count", "Number of colours in the palette, for an indexed image such as a GIF. 0 otherwise.")
}

/// One audio frame, read inside a window that holds exactly that frame: the
/// header, its CRC-8, the subframes, and the CRC-16 of the whole frame. Ogg
/// FLAC gives each frame a packet of its own, which is where that window
/// comes from.
pub(crate) fn frame() -> T {
    let crc8 = Check::of(Checksum::Crc8Flac, Covers::Field { name: Named::here("header") });
    let crc16 = Check::of(
        Checksum::Crc16Flac,
        Covers::Run { at: E::lit(0), len: E::size_of("header").add(E::size_of("crc8")).add(E::size_of("subframes")) },
    );
    T::structure_named(
        "Frame",
        "",
        "",
        vec![
            ("header", frame_header()),
            ("crc8", T::u8()),
            ("subframes", T::bytes(E::Remaining.sub(E::lit(2)))),
            ("crc16", T::u16(Big)),
        ],
    )
    .counted_as("frame")
    .field_check("crc8", crc8)
    .field_check("crc16", crc16)
    .field_doc("crc8", "CRC-8 of the frame header, polynomial 0x07.")
    .field_doc("subframes", "One subframe per channel, bit-packed, then padded with 0 bits to a byte boundary.")
    .field_doc("crc16", "CRC-16 of every byte of the frame before this CRC, polynomial 0x8005.")
}

/// The frame header up to its CRC-8: the sync code and the four codes, the
/// frame or sample number, and the block size and sample rate where the codes
/// say they are written out.
fn frame_header() -> T {
    let code = |f: &str, v: i128| E::field(f).equal_to(E::lit(v));
    T::structure(
        "FrameHeader",
        vec![
            ("sync", bits(15)),
            ("blocking_strategy", T::enumeration("BlockingStrategy", bits(1), &[(0, "fixed"), (1, "variable")])),
            ("block_size_code", T::enumeration("BlockSizeCode", bits(4), BLOCK_SIZE)),
            ("sample_rate_code", T::enumeration("SampleRateCode", bits(4), SAMPLE_RATE)),
            ("channel_assignment", T::enumeration("ChannelAssignment", bits(4), CHANNELS)),
            ("bits_per_sample_code", T::enumeration("BitsPerSampleCode", bits(3), SAMPLE_SIZE)),
            ("reserved", bits(1)),
            ("frame_number", T::when(code("blocking_strategy", 0), coded_number())),
            ("sample_number", T::when(code("blocking_strategy", 1), coded_number())),
            (
                "block_size_minus1",
                T::when(
                    code("block_size_code", 6).either(code("block_size_code", 7)),
                    T::switch(E::field("block_size_code"), vec![(6, T::u8()), (7, T::u16(Big))], T::bytes(E::lit(0))),
                ),
            ),
            ("sample_rate_khz", T::when(code("sample_rate_code", 12), T::u8())),
            ("sample_rate_hz", T::when(code("sample_rate_code", 13), T::u16(Big))),
            ("sample_rate_tens_of_hz", T::when(code("sample_rate_code", 14), T::u16(Big))),
            ("block_size", T::computed(block_size())),
        ],
    )
    .reads_as(&[("block_size", "{} samples", ""), ("channel_assignment", "", "")])
    .field_valid("sync", Valid::Eq(E::lit(0x7ffc)))
    .field_valid("reserved", Valid::Eq(E::lit(0)))
    .field_doc("sync", "Sync code, 15 bits: 0b111111111111100. Every frame starts with it, on a byte boundary.")
    .field_doc("blocking_strategy", "Fixed: every frame except the last has the same block size, and each frame header has a frame number. Variable: block sizes can differ, and each frame header has the number of its first sample.")
    .field_doc("frame_number", "This frame's number, counting from 0, in FLAC's UTF-8-like coding.")
    .field_doc("sample_number", "This frame's first sample, counting from 0, in FLAC's UTF-8-like coding.")
    .field_doc("block_size_minus1", "Samples per channel in this frame, minus 1. 8 bits if the block size code is 6, 16 bits if it is 7.")
    .field_doc("block_size", "Samples per channel in this frame.")
}

/// How many samples a frame holds, from its block size code or the number
/// written after the frame number.
fn block_size() -> E {
    let code = || E::field("block_size_code");
    let table = |c: i128| match c {
        1 => 192,
        2..=5 => 576 << (c - 2),
        8..=15 => 256 << (c - 8),
        _ => 0,
    };
    let mut e = E::lit(0);
    for c in (1..=15).rev() {
        let size = match c {
            6 | 7 => E::field("block_size_minus1").add(E::lit(1)),
            _ => E::lit(table(c)),
        };
        e = E::cond(code().equal_to(E::lit(c)), size, e);
    }
    e
}

/// A frame or sample number in FLAC's extension of UTF-8: one byte for a
/// number under 128, and otherwise a first byte whose leading 1 bits count
/// the bytes, followed by bytes of the form 10xxxxxx. Up to seven bytes, for a
/// number of up to 36 bits. `encoded` is the bytes as written and `value` is
/// the number.
fn coded_number() -> T {
    let lead = || E::peek(8, Big);
    // The number of leading 1 bits in the first byte, which is the number of
    // bytes, except that a byte under 0x80 is a number on its own.
    let mut count = E::lit(7);
    for (n, below) in [(6, 0xfe), (5, 0xfc), (4, 0xf8), (3, 0xf0), (2, 0xe0), (1, 0x80)] {
        count = E::cond(lead().less_than(E::lit(below)), E::lit(n), count);
    }
    let encoded = T::switch(count, (1..=7).map(|n| (n as i128, bits(8 * n))).collect(), T::bytes(E::lit(0)));
    let mut value = E::lit(0);
    for n in (1..=7u32).rev() {
        value = E::cond(E::size_of("encoded").equal_to(E::lit(i128::from(n))), decoded(n), value);
    }
    T::inline_structure("CodedNumber", vec![("encoded", encoded), ("value", T::computed(value))]).reads_as(&[("value", "", "")])
}

/// The number an `n`-byte coded number holds: the bits of the first byte after
/// its length marker, then six bits from each byte after it.
fn decoded(n: u32) -> E {
    let raw = || E::field("encoded");
    if n == 1 {
        return raw();
    }
    let byte = |k: u32| raw().shr(E::lit(i128::from(8 * (n - 1 - k)))).and(E::lit(0xff));
    let lead_bits = (1i128 << (7 - n)) - 1;
    let mut value = byte(0).and(E::lit(lead_bits)).shl(E::lit(i128::from(6 * (n - 1))));
    for k in 1..n {
        value = value.add(byte(k).and(E::lit(0x3f)).shl(E::lit(i128::from(6 * (n - 1 - k)))));
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checksum::{crc16_flac, crc8_flac};
    use crate::{document::Document, eval::Evaluator, source::MemSource};

    /// A metadata block: the header byte, the 24-bit length, and the data.
    pub(crate) fn block(kind: u8, last: bool, data: &[u8]) -> Vec<u8> {
        let mut v = vec![kind | if last { 0x80 } else { 0 }];
        v.extend_from_slice(&(data.len() as u32).to_be_bytes()[1..]);
        v.extend_from_slice(data);
        v
    }

    /// STREAMINFO for 44.1 kHz stereo 16-bit audio of `total` samples.
    pub(crate) fn stream_info_bytes(total: u64) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&4096u16.to_be_bytes());
        v.extend_from_slice(&4096u16.to_be_bytes());
        v.extend_from_slice(&[0x00, 0x00, 0x0e]); // min frame size 14
        v.extend_from_slice(&[0x00, 0x10, 0x00]); // max frame size 4096
        // 20 bits of rate, 3 of channels - 1, 5 of bits - 1, 36 of samples.
        let packed: u64 = (44_100u64 << 44) | (1 << 41) | (15 << 36) | total;
        v.extend_from_slice(&packed.to_be_bytes());
        v.extend_from_slice(&[0xab; 16]);
        v
    }

    fn file(blocks: &[Vec<u8>], frames: &[u8]) -> Vec<u8> {
        let mut v = MAGIC.to_vec();
        for b in blocks {
            v.extend_from_slice(b);
        }
        v.extend_from_slice(frames);
        v
    }

    fn open(bytes: Vec<u8>) -> (Document<MemSource>, Evaluator) {
        (Document::new(MemSource(bytes)), Evaluator::new(flac()))
    }

    fn path(e: &mut Evaluator, d: &Document<MemSource>, names: &[&str]) -> Vec<usize> {
        let mut p = Vec::new();
        for name in names {
            let next = match name.parse::<usize>() {
                Ok(i) => i,
                Err(_) => e.child_named(d, &p, name).unwrap().unwrap_or_else(|| panic!("no field called {name} under {p:?}"))[p.len()],
            };
            p.push(next);
        }
        p
    }

    fn int(e: &mut Evaluator, d: &Document<MemSource>, names: &[&str]) -> i128 {
        let p = path(e, d, names);
        e.node(d, &p).unwrap().value.as_int().unwrap_or_else(|| panic!("{names:?} is not a number"))
    }

    fn text(e: &mut Evaluator, d: &Document<MemSource>, names: &[&str]) -> String {
        let p = path(e, d, names);
        match e.node(d, &p).unwrap().value {
            crate::eval::Value::Str(s) => s,
            other => panic!("{names:?} is not text: {other:?}"),
        }
    }

    #[test]
    fn stream_info_unpacks_its_bit_fields_and_adds_one_where_the_file_subtracts_it() {
        let bytes = file(&[block(0, true, &stream_info_bytes(441_000))], &[0xff, 0xf8, 0, 0]);
        let (d, mut e) = open(bytes);
        let si = ["metadata_blocks", "0", "data"];
        let f = |name: &'static str| [si[0], si[1], si[2], name];
        assert_eq!(int(&mut e, &d, &f("min_block_size")), 4096);
        assert_eq!(int(&mut e, &d, &f("min_frame_size")), 14);
        assert_eq!(int(&mut e, &d, &f("max_frame_size")), 4096);
        assert_eq!(int(&mut e, &d, &f("sample_rate")), 44_100);
        assert_eq!(int(&mut e, &d, &f("channels_minus1")), 1);
        assert_eq!(int(&mut e, &d, &f("channels")), 2);
        assert_eq!(int(&mut e, &d, &f("bits_per_sample")), 16);
        assert_eq!(int(&mut e, &d, &f("total_samples")), 441_000);
        let p = path(&mut e, &d, &f("duration"));
        assert_eq!(e.node(&d, &p).unwrap().value, crate::eval::Value::Float(10.0));
        // The block is exactly its length, and the frames follow it.
        let block = path(&mut e, &d, &["metadata_blocks", "0"]);
        assert_eq!(e.node(&d, &block).unwrap().size_bits, (4 + 34) * 8);
        let frames = path(&mut e, &d, &["frames"]);
        assert_eq!(e.node(&d, &frames).unwrap().size_bits, 4 * 8);
    }

    #[test]
    fn blocks_go_on_until_the_one_with_the_last_block_flag() {
        let bytes = file(
            &[block(0, false, &stream_info_bytes(0)), block(1, false, &[0; 10]), block(2, true, b"ATCHsome data")],
            &[0xff, 0xf8],
        );
        let (d, mut e) = open(bytes);
        let blocks = path(&mut e, &d, &["metadata_blocks"]);
        assert_eq!(e.node(&d, &blocks).unwrap().child_count, 3);
        assert_eq!(int(&mut e, &d, &["metadata_blocks", "1", "block_type"]), 1);
        assert_eq!(text(&mut e, &d, &["metadata_blocks", "2", "data", "application_id"]), "ATCH");
        let data = path(&mut e, &d, &["metadata_blocks", "2", "data", "data"]);
        assert_eq!(e.node(&d, &data).unwrap().size_bits, 9 * 8);
        // Zero total samples is unknown, and there is no duration to give.
        let duration = path(&mut e, &d, &["metadata_blocks", "0", "data", "duration"]);
        assert!(!matches!(e.node(&d, &duration).unwrap().value, crate::eval::Value::Float(_)));
    }

    #[test]
    fn a_seek_table_is_eighteen_byte_points_and_a_placeholder_is_unset() {
        let mut data = Vec::new();
        for (sample, offset, n) in [(0u64, 0u64, 4096u16), (40_960, 12_345, 4096), (u64::MAX, 0, 0)] {
            data.extend_from_slice(&sample.to_be_bytes());
            data.extend_from_slice(&offset.to_be_bytes());
            data.extend_from_slice(&n.to_be_bytes());
        }
        let bytes = file(&[block(0, false, &stream_info_bytes(0)), block(3, true, &data)], &[]);
        let (d, mut e) = open(bytes);
        let points = path(&mut e, &d, &["metadata_blocks", "1", "data", "points"]);
        assert_eq!(e.node(&d, &points).unwrap().child_count, 3);
        assert_eq!(int(&mut e, &d, &["metadata_blocks", "1", "data", "points", "1", "stream_offset"]), 12_345);
        assert_eq!(int(&mut e, &d, &["metadata_blocks", "1", "data", "points", "1", "sample_number"]), 40_960);
        let placeholder = path(&mut e, &d, &["metadata_blocks", "1", "data", "points", "2", "sample_number"]);
        assert!(matches!(e.node(&d, &placeholder).unwrap().value, crate::eval::Value::Unset(_)));
    }

    /// The comment block's lengths are little-endian, unlike everything else
    /// in FLAC, and each comment is split at its first `=`.
    pub(crate) fn comment_bytes(vendor: &str, comments: &[&str]) -> Vec<u8> {
        let mut v = (vendor.len() as u32).to_le_bytes().to_vec();
        v.extend_from_slice(vendor.as_bytes());
        v.extend_from_slice(&(comments.len() as u32).to_le_bytes());
        for c in comments {
            v.extend_from_slice(&(c.len() as u32).to_le_bytes());
            v.extend_from_slice(c.as_bytes());
        }
        v
    }

    #[test]
    fn vorbis_comments_are_a_vendor_and_name_value_pairs_with_little_endian_lengths() {
        let data = comment_bytes("reference libFLAC 1.4.3", &["TITLE=Sine wave", "ARTIST=Nobody", "NOEQUALS"]);
        let bytes = file(&[block(0, false, &stream_info_bytes(0)), block(4, true, &data)], &[]);
        let (d, mut e) = open(bytes);
        let vc = |rest: &[&'static str]| -> Vec<&'static str> { [&["metadata_blocks", "1", "data"][..], rest].concat() };
        assert_eq!(text(&mut e, &d, &vc(&["vendor"])), "reference libFLAC 1.4.3");
        assert_eq!(int(&mut e, &d, &vc(&["comment_count"])), 3);
        assert_eq!(text(&mut e, &d, &vc(&["comments", "0", "text", "name"])), "TITLE");
        assert_eq!(text(&mut e, &d, &vc(&["comments", "0", "text", "value"])), "Sine wave");
        assert_eq!(text(&mut e, &d, &vc(&["comments", "1", "text", "value"])), "Nobody");
        // A comment with no `=` is all name, and stays inside its length.
        assert_eq!(text(&mut e, &d, &vc(&["comments", "2", "text", "name"])), "NOEQUALS");
        let last = path(&mut e, &d, &vc(&["comments", "2"]));
        assert_eq!(e.node(&d, &last).unwrap().size_bits, (4 + 8) * 8);
    }

    #[test]
    fn a_cue_sheet_reads_its_tracks_and_their_index_points() {
        let mut data = vec![0u8; 128];
        data[..13].copy_from_slice(b"1234567890123");
        data.extend_from_slice(&88_200u64.to_be_bytes());
        data.push(0x80); // a CD, then seven reserved bits
        data.extend_from_slice(&[0; 258]);
        data.push(2);
        // Track 1, audio, with one index point at 0.
        data.extend_from_slice(&0u64.to_be_bytes());
        data.push(1);
        data.extend_from_slice(b"USRC17607839");
        data.push(0x00);
        data.extend_from_slice(&[0; 13]);
        data.push(1);
        data.extend_from_slice(&0u64.to_be_bytes());
        data.push(1);
        data.extend_from_slice(&[0; 3]);
        // The lead-out, 170 on a CD, with no index points.
        data.extend_from_slice(&441_000u64.to_be_bytes());
        data.push(170);
        data.extend_from_slice(&[0; 12]);
        data.push(0x00);
        data.extend_from_slice(&[0; 13]);
        data.push(0);
        assert_eq!(data.len(), 396 + 36 + 12 + 36);
        let bytes = file(&[block(0, false, &stream_info_bytes(0)), block(5, true, &data)], &[]);
        let (d, mut e) = open(bytes);
        let cs = |rest: &[&'static str]| -> Vec<&'static str> { [&["metadata_blocks", "1", "data"][..], rest].concat() };
        assert_eq!(text(&mut e, &d, &cs(&["media_catalog_number"])), "1234567890123");
        assert_eq!(int(&mut e, &d, &cs(&["is_cd"])), 1);
        assert_eq!(int(&mut e, &d, &cs(&["lead_in_samples"])), 88_200);
        assert_eq!(text(&mut e, &d, &cs(&["tracks", "0", "isrc"])), "USRC17607839");
        assert_eq!(int(&mut e, &d, &cs(&["tracks", "0", "indexes", "0", "index_number"])), 1);
        assert_eq!(int(&mut e, &d, &cs(&["tracks", "1", "track_number"])), 170);
        assert_eq!(int(&mut e, &d, &cs(&["tracks", "1", "offset"])), 441_000);
        let block = path(&mut e, &d, &["metadata_blocks", "1"]);
        assert_eq!(e.node(&d, &block).unwrap().size_bits, (4 + data.len() as u64) * 8);
    }

    fn picture_bytes(mime: &str, image: &[u8]) -> Vec<u8> {
        let mut v = 3u32.to_be_bytes().to_vec();
        v.extend_from_slice(&(mime.len() as u32).to_be_bytes());
        v.extend_from_slice(mime.as_bytes());
        v.extend_from_slice(&5u32.to_be_bytes());
        v.extend_from_slice(b"cover");
        for n in [1u32, 1, 24, 0] {
            v.extend_from_slice(&n.to_be_bytes());
        }
        v.extend_from_slice(&(image.len() as u32).to_be_bytes());
        v.extend_from_slice(image);
        v
    }

    /// A one-pixel PNG, signature to IEND.
    fn tiny_png() -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut chunk = |kind: &[u8], data: &[u8]| {
            v.extend_from_slice(&(data.len() as u32).to_be_bytes());
            let mut body = kind.to_vec();
            body.extend_from_slice(data);
            v.extend_from_slice(&body);
            v.extend_from_slice(&crate::checksum::crc32(&body).to_be_bytes());
        };
        chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0]);
        chunk(b"IEND", &[]);
        v
    }

    #[test]
    fn a_picture_reads_its_header_and_its_data_as_the_png_it_is() {
        let png = tiny_png();
        let bytes = file(&[block(0, false, &stream_info_bytes(0)), block(6, true, &picture_bytes("image/png", &png))], &[]);
        let (d, mut e) = open(bytes);
        let pic = |rest: &[&'static str]| -> Vec<&'static str> { [&["metadata_blocks", "1", "data"][..], rest].concat() };
        assert_eq!(int(&mut e, &d, &pic(&["picture_type"])), 3);
        assert_eq!(text(&mut e, &d, &pic(&["mime_type"])), "image/png");
        assert_eq!(text(&mut e, &d, &pic(&["description"])), "cover");
        assert_eq!(int(&mut e, &d, &pic(&["color_depth"])), 24);
        assert_eq!(int(&mut e, &d, &pic(&["data", "chunks", "0", "data", "width"])), 1);
        let data = path(&mut e, &d, &pic(&["data"]));
        assert_eq!(e.node(&d, &data).unwrap().size_bits, png.len() as u64 * 8);
    }

    /// The smallest JPEG the template reads: start of image, one comment
    /// segment, end of image.
    #[test]
    fn a_picture_reads_its_data_as_the_jpeg_it_is() {
        let jpeg = [0xff, 0xd8, 0xff, 0xfe, 0x00, 0x04, b'h', b'i', 0xff, 0xd9];
        let bytes = file(&[block(0, false, &stream_info_bytes(0)), block(6, true, &picture_bytes("image/jpeg", &jpeg))], &[]);
        let (d, mut e) = open(bytes);
        let pic = |rest: &[&'static str]| -> Vec<&'static str> { [&["metadata_blocks", "1", "data", "data"][..], rest].concat() };
        let segments = path(&mut e, &d, &pic(&["segments"]));
        assert_eq!(e.node(&d, &segments).unwrap().child_count, 2);
        assert_eq!(int(&mut e, &d, &pic(&["segments", "0", "marker"])), 0xfffe);
        assert_eq!(int(&mut e, &d, &pic(&["segments", "1", "marker"])), 0xffd9);
        let data = path(&mut e, &d, &pic(&[]));
        assert_eq!(e.node(&d, &data).unwrap().size_bits, jpeg.len() as u64 * 8);
    }

    #[test]
    fn a_picture_that_is_not_an_image_stays_bytes_and_a_link_reads_as_text() {
        let bytes = file(
            &[
                block(0, false, &stream_info_bytes(0)),
                block(6, false, &picture_bytes("image/x-unknown", b"not an image")),
                block(6, true, &picture_bytes("-->", b"https://example.com/cover.png")),
            ],
            &[],
        );
        let (d, mut e) = open(bytes);
        let p = path(&mut e, &d, &["metadata_blocks", "1", "data", "data"]);
        assert!(e.node(&d, &p).unwrap().type_name.starts_with("bytes"));
        assert_eq!(e.node(&d, &p).unwrap().size_bits, 12 * 8);
        assert_eq!(text(&mut e, &d, &["metadata_blocks", "2", "data", "data"]), "https://example.com/cover.png");
    }

    /// A frame header with its CRC-8, the subframe bytes, and the CRC-16, for
    /// block size code `bs` and frame number `number` coded as FLAC codes it.
    pub(crate) fn frame_bytes(bs: u8, extra: &[u8], number: &[u8], subframes: &[u8]) -> Vec<u8> {
        // Fixed blocking, 44.1 kHz, stereo, 16 bits.
        let mut v = vec![0xff, 0xf8, (bs << 4) | 9, (1 << 4) | (4 << 1)];
        v.extend_from_slice(number);
        v.extend_from_slice(extra);
        v.push(crc8_flac(&v));
        v.extend_from_slice(subframes);
        let sum = crc16_flac(&v);
        v.extend_from_slice(&sum.to_be_bytes());
        v
    }

    fn frame_doc(bytes: Vec<u8>) -> (Document<MemSource>, Evaluator) {
        let t = Template::new("frame", T::structure("One", vec![("frame", T::sized(E::Remaining, frame()))]));
        (Document::new(MemSource(bytes)), Evaluator::new(t))
    }

    #[test]
    fn a_frame_header_reads_its_codes_and_checks_both_crcs() {
        let bytes = frame_bytes(12, &[], &[0x05], &[0x11; 20]);
        let len = bytes.len() as u64;
        let (d, mut e) = frame_doc(bytes);
        let h = |name: &str| ["frame", "header", name].map(String::from);
        let get = |e: &mut Evaluator, d: &Document<MemSource>, names: &[String]| {
            let names: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
            int(e, d, &names)
        };
        assert_eq!(get(&mut e, &d, &h("sync")), 0x7ffc);
        assert_eq!(get(&mut e, &d, &h("block_size_code")), 12);
        assert_eq!(get(&mut e, &d, &h("block_size")), 4096);
        assert_eq!(get(&mut e, &d, &h("sample_rate_code")), 9);
        assert_eq!(get(&mut e, &d, &h("channel_assignment")), 1);
        assert_eq!(get(&mut e, &d, &h("bits_per_sample_code")), 4);
        assert_eq!(int(&mut e, &d, &["frame", "header", "frame_number", "value"]), 5);
        for check in ["crc8", "crc16"] {
            let p = path(&mut e, &d, &["frame", check]);
            let v = e.run_check(&d, &p).unwrap().expect("a verdict");
            assert!(v.ok, "{check}: computed {}, stored {}", v.computed, v.stored);
        }
        let subframes = path(&mut e, &d, &["frame", "subframes"]);
        assert_eq!(e.node(&d, &subframes).unwrap().size_bits, 20 * 8);
        let frame = path(&mut e, &d, &["frame"]);
        assert_eq!(e.node(&d, &frame).unwrap().size_bits, len * 8);
    }

    #[test]
    fn an_uncommon_block_size_is_written_after_the_number_as_the_size_minus_one() {
        // Code 6: an 8-bit size minus 1 after the number. 0xc2 0xa0 is 160 in
        // two bytes of UTF-8.
        let (d, mut e) = frame_doc(frame_bytes(6, &[99], &[0xc2, 0xa0], &[0; 4]));
        assert_eq!(int(&mut e, &d, &["frame", "header", "frame_number", "value"]), 160);
        assert_eq!(int(&mut e, &d, &["frame", "header", "block_size_minus1"]), 99);
        assert_eq!(int(&mut e, &d, &["frame", "header", "block_size"]), 100);
        let p = path(&mut e, &d, &["frame", "crc8"]);
        assert!(e.run_check(&d, &p).unwrap().expect("a verdict").ok);
        // Code 7: sixteen bits.
        let (d, mut e) = frame_doc(frame_bytes(7, &[0x12, 0x33], &[0], &[0; 4]));
        assert_eq!(int(&mut e, &d, &["frame", "header", "block_size"]), 0x1234);
    }

    #[test]
    fn a_coded_number_of_seven_bytes_holds_thirty_six_bits() {
        // 0xfe then six continuation bytes: 36 bits, all ones here.
        let (d, mut e) = frame_doc(frame_bytes(12, &[], &[0xfe, 0xbf, 0xbf, 0xbf, 0xbf, 0xbf, 0xbf], &[0; 2]));
        assert_eq!(int(&mut e, &d, &["frame", "header", "frame_number", "value"]), (1 << 36) - 1);
        // Three bytes: 0xe2 0x82 0xac is U+20AC.
        let (d, mut e) = frame_doc(frame_bytes(12, &[], &[0xe2, 0x82, 0xac], &[0; 2]));
        assert_eq!(int(&mut e, &d, &["frame", "header", "frame_number", "value"]), 0x20ac);
    }

    #[test]
    fn a_frame_someone_edited_fails_its_crc_16_and_keeps_its_crc_8() {
        let mut bytes = frame_bytes(12, &[], &[0], &[0x42; 10]);
        bytes[8] ^= 1;
        let (d, mut e) = frame_doc(bytes);
        let p = path(&mut e, &d, &["frame", "crc8"]);
        assert!(e.run_check(&d, &p).unwrap().expect("a verdict").ok);
        let p = path(&mut e, &d, &["frame", "crc16"]);
        assert!(!e.run_check(&d, &p).unwrap().expect("a verdict").ok);
    }

    /// A seek point's offset counts bytes from the first frame. A cue sheet's
    /// offsets count samples, and are not positions.
    #[test]
    fn a_seek_point_offset_is_a_position() {
        let t = flac();
        crate::machinery::assert_marks(&t, "SeekPoint", &["stream_offset"], &["sample_number", "frame_samples"]);
        crate::machinery::assert_marks(&t, "CueSheetTrack", &[], &["offset"]);
    }
}
