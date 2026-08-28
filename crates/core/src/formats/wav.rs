//! RIFF/WAVE, and the metadata chunks that bat recordings carry.
//!
//! A RIFF chunk is an id, a little-endian size, and a body padded to an even
//! number of bytes. The pad byte is outside the size but inside the file, so
//! it is a field of its own: `size` modulo two, written with a divide and a
//! multiply because the expression language has no modulo.
//!
//! GUANO (`guan`) is the metadata format bat detectors write: UTF-8 lines of
//! `Key:Value`, one per line, sometimes with a trailing NUL inside the chunk
//! size. Lines are read with a terminator that tolerates its own absence, so
//! the last line reads whether or not it ends in a newline.
//!
//! `wamd` is Wildlife Acoustics' own metadata: a flat stream of 16-bit tag,
//! 32-bit length, payload. Its tag numbers here were read out of files, not
//! from a specification.
//!
//! The rest is what studio files carry: the extensible format header, the
//! broadcast extension `bext` (EBU Tech 3285), `smpl` and `inst` for samplers,
//! `plst`, the labels in an `adtl` list, XML chunks, and an ID3 tag, which is
//! read by the same template that reads one at the front of an MP3.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};

/// A four-character chunk id as the big-endian number a switch compares.
fn cc(s: &str) -> i128 {
    s.bytes().fold(0i128, |acc, b| (acc << 8) | b as i128)
}

/// The format tags a WAVE file's `fmt ` chunk can carry.
const FORMAT_TAG: &[(i128, &str)] = &[
    (0x0001, "pcm"),
    (0x0003, "ieee float"),
    (0x0006, "a-law"),
    (0x0007, "mu-law"),
    (0x5741, "w4v"),
    (0xfffe, "extensible"),
];

/// Speaker layouts the extensible format's channel mask spells out. The mask is
/// one bit per speaker; these are the combinations that have a name.
const CHANNEL_MASK: &[(i128, &str)] = &[
    (0x003, "stereo"),
    (0x004, "mono"),
    (0x033, "quad"),
    (0x03f, "5.1"),
    (0x60f, "5.1 side"),
    (0x0ff, "7.1"),
    (0x63f, "7.1 side"),
];

/// How a sampler plays a loop.
const LOOP_TYPE: &[(i128, &str)] = &[(0, "forward"), (1, "alternating"), (2, "backward")];

/// The frame rates `smpl` can give its offset in.
const SMPTE_FORMAT: &[(i128, &str)] =
    &[(0, "none"), (24, "24 fps"), (25, "25 fps"), (29, "30 fps drop"), (30, "30 fps")];

/// `wamd` tag numbers, as seen in Song Meter files.
const WAMD_TAG: &[(i128, &str)] = &[
    (0, "version"),
    (1, "model"),
    (2, "serial"),
    (3, "firmware"),
    (4, "prefix"),
    (5, "timestamp"),
    (6, "position"),
    (7, "temperature"),
];

pub fn wav() -> Template {
    riff("wav", chunk_body(Some(samples())))
}

/// The RIFF frame shared by WAVE and its relatives.
pub(super) fn riff(name: &str, body: T) -> Template {
    Template::new(
        name,
        T::structure(
            "RIFF",
            vec![
                ("magic", T::magic(b"RIFF")),
                ("size", T::u32(Little)),
                ("form", T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii)),
                ("chunks", T::repeat(T::Named("Chunk".into()), Until::End)),
            ],
        ),
    )
    .with_type("Chunk", chunk(body))
    .with_type("ListItem", list_item())
}

fn chunk(body: T) -> T {
    // A chunk body is padded to an even length. The pad byte is not counted in
    // the size, so it is a field of its own: size - (size / 2) * 2.
    // A chunk of an odd length is padded to an even one, and the last chunk in
    // a file often is not: the writer had nothing to follow it with. So the
    // pad byte is there when the length is odd and there is a byte left to be
    // it, which is what the guard multiplies by.
    let pad = || {
        E::field("size")
            .sub(E::field("size").div(E::lit(2)).mul(E::lit(2)))
            .mul(E::lit(0).less_than(E::Remaining))
    };
    T::structure_named(
        "Chunk",
        "id",
        "body",
        vec![
            ("id", T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii)),
            ("size", T::u32(Little)),
            ("body", T::sized(E::field("size"), body)),
            ("pad", T::bytes(pad())),
        ],
    )
}

/// The samples in a `data` chunk, read as what the `fmt ` chunk earlier in the
/// file said they are. `fmt ` is a sibling chunk rather than a field of this
/// one, and a `fact` or a `LIST` can sit between the two, so the width is
/// asked of the nearest earlier chunk that declares one.
///
/// The samples in a `data` chunk, read as what the `fmt ` chunk earlier in the
/// file said they are. `fmt ` is a sibling chunk rather than a field of this
/// one, and a `fact` or a `LIST` can sit between the two, so the width is
/// asked of the nearest earlier chunk that declares one.
///
/// Samples are interleaved: with two channels the values alternate left,
/// right, in the order they sit in. Which width to read is settled once, above
/// the list, so every element of it is the same size and the millionth sample
/// can be reached without reading the ones before it.
///
/// A file whose format nobody here knows keeps its data as bytes, rather than
/// being read as something it was never said to be.
fn samples() -> T {
    let bits = || E::sibling(&["body", "bits_per_sample"]);
    let raw = || T::bytes(E::Remaining);
    // A run of samples `width` bytes apart, which is what the rest of the
    // chunk holds.
    let run = |width: i128, elem: T| T::array(elem, E::Remaining.div(E::lit(width)));
    // Integer samples are signed, except 8-bit, which the format defines as
    // unsigned with 128 for silence.
    let pcm = |bits: u32| T::Int { bits, endian: Little };
    let by_width = || {
        T::switch(
            bits(),
            vec![
                (8, run(1, T::u8())),
                (16, run(2, pcm(16))),
                (24, run(3, pcm(24))),
                (32, run(4, pcm(32))),
            ],
            raw(),
        )
    };
    let floats = || T::switch(bits(), vec![(32, run(4, T::F32(Little))), (64, run(8, T::F64(Little)))], raw());
    let by_format = |format: E| {
        T::switch(format, vec![(0x0001, by_width()), (0x0003, floats())], raw())
    };
    // An extensible file gives its real format at the front of the sub-format
    // GUID, and the tag in front only says to look there.
    T::switch(
        E::sibling(&["body", "format"]),
        vec![(0xfffe, by_format(E::sibling(&["body", "extra", "sub_format", "format"])))],
        by_format(E::sibling(&["body", "format"])),
    )
}

/// What is inside a chunk, by its id. `data` is left as bytes unless a format
/// on top of WAVE knows how to read it.
pub(super) fn chunk_body(data: Option<T>) -> T {
    let mut cases = vec![
            (cc("fmt "), fmt()),
            (cc("fact"), T::structure("Fact", vec![("sample_count", T::u32(Little))])),
            (cc("guan"), guano()),
            (cc("wamd"), T::repeat(wamd_item(), Until::End)),
            (cc("cue "), cue()),
            (cc("LIST"), list()),
            (cc("ds64"), ds64()),
            (cc("bext"), bext()),
            (cc("smpl"), smpl()),
            (cc("inst"), inst()),
            (cc("plst"), plst()),
            // Whole documents kept inside a chunk: XML from recorders and
            // editing suites, and an ID3 tag as it appears in an MP3.
            (cc("iXML"), xml()),
            (cc("_PMX"), xml()),
            (cc("axml"), xml()),
            (cc("id3 "), super::id3::tag()),
            (cc("ID3 "), super::id3::tag()),
    ];
    if let Some(d) = data {
        cases.push((cc("data"), d));
    }
    T::switch(E::field("id"), cases, T::bytes(E::Remaining))
}

fn fmt() -> T {
    T::structure(
        "Format",
        vec![
            ("format", T::enumeration_hex("FormatTag", T::u16(Little), FORMAT_TAG)),
            ("channels", T::u16(Little)),
            ("sample_rate", T::u32(Little)),
            ("byte_rate", T::u32(Little)),
            ("block_align", T::u16(Little)),
            ("bits_per_sample", T::u16(Little)),
            // 16-byte fmt chunks stop here. Longer ones carry an extension,
            // which the extensible format fills in and others leave opaque.
            ("extra", T::switch(E::field("format"), vec![(0xfffe, extensible())], T::bytes(E::Remaining))),
        ],
    )
}

/// What `WAVE_FORMAT_EXTENSIBLE` adds: how many of the bits per sample are
/// real, which speaker each channel drives, and the format tag again, this
/// time as the first two bytes of a GUID.
fn extensible() -> T {
    let sub_format = T::structure(
        "SubFormat",
        vec![
            ("format", T::enumeration_hex("FormatTag", T::u16(Little), FORMAT_TAG)),
            ("guid", T::bytes(E::lit(14))),
        ],
    );
    T::structure(
        "Extensible",
        vec![
            ("extension_size", T::u16(Little)),
            ("valid_bits_per_sample", T::u16(Little)),
            ("channel_mask", T::enumeration_hex("ChannelMask", T::u32(Little), CHANNEL_MASK)),
            ("sub_format", sub_format),
        ],
    )
}

/// GUANO: one `Key:Value` per line, in UTF-8.
fn guano() -> T {
    T::repeat(
        T::text(StrLen::Terminated { end: b'\n', or_end: true }, Encoding::Utf8),
        Until::End,
    )
}

fn wamd_item() -> T {
    T::structure(
        "Item",
        vec![
            ("tag", T::enumeration("WamdTag", T::u16(Little), WAMD_TAG)),
            ("length", T::u32(Little)),
            // Ids 1 to 6 hold text; the rest are bytes, so read them as text
            // only when they say they are.
            (
                "value",
                T::sized(
                    E::field("length"),
                    T::switch(
                        E::field("tag"),
                        (1..=7).map(|t| (t as i128, T::text(StrLen::Fixed(E::Remaining), Encoding::Latin1))).collect(),
                        T::bytes(E::Remaining),
                    ),
                ),
            ),
        ],
    )
}

fn cue() -> T {
    let point = T::structure(
        "CuePoint",
        vec![
            ("id", T::u32(Little)),
            ("position", T::u32(Little)),
            ("chunk", T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii)),
            ("chunk_start", T::u32(Little)),
            ("block_start", T::u32(Little)),
            ("sample_offset", T::u32(Little)),
        ],
    );
    T::structure("Cue", vec![("count", T::u32(Little)), ("points", T::array(point, E::field("count")))])
}

/// A LIST holds a type and then more chunks, of whichever flavour that type says.
fn list() -> T {
    T::structure(
        "List",
        vec![
            ("type", T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii)),
            ("items", T::repeat(T::Named("ListItem".into()), Until::End)),
        ],
    )
}

fn list_item() -> T {
    // A chunk of an odd length is padded to an even one, and the last chunk in
    // a file often is not: the writer had nothing to follow it with. So the
    // pad byte is there when the length is odd and there is a byte left to be
    // it, which is what the guard multiplies by.
    let pad = || {
        E::field("size")
            .sub(E::field("size").div(E::lit(2)).mul(E::lit(2)))
            .mul(E::lit(0).less_than(E::Remaining))
    };
    // A LIST member is a chunk like any other, so it is called one.
    T::structure(
        "Chunk",
        vec![
            ("id", T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii)),
            ("size", T::u32(Little)),
            (
                "body",
                T::sized(
                    E::field("size"),
                    T::switch(
                        E::field("id"),
                        vec![
                            (cc("labl"), labelled()),
                            (cc("note"), labelled()),
                            (cc("ltxt"), labelled_text()),
                        ],
                        // INFO items are NUL-terminated Latin-1 text.
                        T::text(StrLen::Terminated { end: 0, or_end: true }, Encoding::Latin1),
                    ),
                ),
            ),
            ("pad", T::bytes(pad())),
        ],
    )
}

fn labelled() -> T {
    T::structure(
        "Label",
        vec![
            ("cue_id", T::u32(Little)),
            ("text", T::text(StrLen::Terminated { end: 0, or_end: true }, Encoding::Latin1)),
        ],
    )
}

/// `ltxt`: a label that covers a stretch of samples rather than a point.
fn labelled_text() -> T {
    T::structure(
        "LabelledText",
        vec![
            ("cue_id", T::u32(Little)),
            ("sample_length", T::u32(Little)),
            ("purpose", T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii)),
            ("country", T::u16(Little)),
            ("language", T::u16(Little)),
            ("dialect", T::u16(Little)),
            ("code_page", T::u16(Little)),
            ("text", T::text(StrLen::Padded { size: E::Remaining, pad: 0 }, Encoding::Latin1)),
        ],
    )
}

/// The Broadcast Wave extension (EBU Tech 3285). All three versions are 602
/// bytes before the coding history: version 1 took the UMID out of the reserved
/// area, version 2 took the loudness values out of what was left. So the version
/// says which of those two exist, and what is reserved is what they did not use.
fn bext() -> T {
    let ascii = |n: i128| T::text(StrLen::Padded { size: E::lit(n), pad: 0 }, Encoding::Ascii);
    let i16le = || T::Int { bits: 16, endian: Little };
    // Each of these is hundredths of a unit, so -2300 is -23.00 LUFS.
    let loudness = T::structure(
        "Loudness",
        vec![
            ("integrated", i16le()),
            ("range", i16le()),
            ("max_true_peak", i16le()),
            ("max_momentary", i16le()),
            ("max_short_term", i16le()),
        ],
    );
    T::structure(
        "Broadcast",
        vec![
            ("description", ascii(256)),
            ("originator", ascii(32)),
            ("originator_reference", ascii(32)),
            ("origination_date", ascii(10)),
            ("origination_time", ascii(8)),
            // Samples since midnight, as the two halves of a 64-bit count.
            ("time_reference_low", T::u32(Little)),
            ("time_reference_high", T::u32(Little)),
            ("version", T::u16(Little)),
            ("umid", T::switch(E::field("version"), vec![(0, T::bytes(E::lit(0)))], T::bytes(E::lit(64)))),
            ("loudness", T::switch(E::field("version"), vec![(2, loudness)], T::bytes(E::lit(0)))),
            // 254 bytes, minus whatever those two took out of them.
            ("reserved", T::bytes(E::lit(254).sub(E::size_of("umid")).sub(E::size_of("loudness")))),
            ("coding_history", T::text(StrLen::Padded { size: E::Remaining, pad: 0 }, Encoding::Ascii)),
        ],
    )
}

/// `smpl`: what a sampler needs to play the file as an instrument.
fn smpl() -> T {
    let sample_loop = T::structure(
        "Loop",
        vec![
            ("id", T::u32(Little)),
            ("type", T::enumeration("LoopType", T::u32(Little), LOOP_TYPE)),
            ("start", T::u32(Little)),
            ("end", T::u32(Little)),
            ("fraction", T::u32(Little)),
            // Zero means keep looping.
            ("play_count", T::u32(Little)),
        ],
    );
    T::structure(
        "Sampler",
        vec![
            ("manufacturer", T::u32(Little)),
            ("product", T::u32(Little)),
            // Nanoseconds per sample: 22675 at 44.1 kHz.
            ("sample_period", T::u32(Little)),
            ("midi_unity_note", T::u32(Little)),
            ("midi_pitch_fraction", T::u32(Little)),
            ("smpte_format", T::enumeration("SmpteFormat", T::u32(Little), SMPTE_FORMAT)),
            ("smpte_offset", T::u32(Little)),
            ("loop_count", T::u32(Little)),
            ("sampler_data", T::u32(Little)),
            ("loops", T::array(sample_loop, E::field("loop_count"))),
            ("extra", T::bytes(E::Remaining)),
        ],
    )
}

/// `inst`: the same idea in seven bytes, for samplers that want no more.
fn inst() -> T {
    let i8 = || T::Int { bits: 8, endian: Little };
    T::structure(
        "Instrument",
        vec![
            ("unshifted_note", T::u8()),
            // Cents, and decibels, either side of zero.
            ("fine_tune", i8()),
            ("gain", i8()),
            ("low_note", T::u8()),
            ("high_note", T::u8()),
            ("low_velocity", T::u8()),
            ("high_velocity", T::u8()),
        ],
    )
}

/// `plst`: play these stretches, in this order.
fn plst() -> T {
    let segment = T::structure(
        "Segment",
        vec![("cue_id", T::u32(Little)), ("length", T::u32(Little)), ("repeats", T::u32(Little))],
    );
    T::structure("Playlist", vec![("count", T::u32(Little)), ("segments", T::array(segment, E::field("count")))])
}

/// A chunk holding a whole XML document, padded out with NULs.
fn xml() -> T {
    T::text(StrLen::Padded { size: E::Remaining, pad: 0 }, Encoding::Utf8)
}

fn ds64() -> T {
    T::structure(
        "DataSize64",
        vec![
            ("riff_size", T::u64(Little)),
            ("data_size", T::u64(Little)),
            ("sample_count", T::u64(Little)),
            ("table_length", T::u32(Little)),
            ("table", T::bytes(E::Remaining)),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn chunk_bytes(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = id.to_vec();
        v.extend_from_slice(&(body.len() as u32).to_le_bytes());
        v.extend_from_slice(body);
        if body.len() % 2 == 1 {
            v.push(0);
        }
        v
    }

    /// A file whose last chunk is an odd number of bytes long and has no pad
    /// byte after it, which is how a good many recorders wrote them. The pad
    /// belongs to the chunk, and a chunk with nothing after it never had one.
    #[test]
    fn a_last_chunk_of_odd_length_need_not_be_padded() {
        let mut fmt = Vec::new();
        fmt.extend_from_slice(&1u16.to_le_bytes());
        fmt.extend_from_slice(&1u16.to_le_bytes());
        fmt.extend_from_slice(&8000u32.to_le_bytes());
        fmt.extend_from_slice(&8000u32.to_le_bytes());
        fmt.extend_from_slice(&1u16.to_le_bytes());
        fmt.extend_from_slice(&8u16.to_le_bytes());
        let mut body = b"WAVE".to_vec();
        body.extend(chunk_bytes(b"fmt ", &fmt));
        body.extend_from_slice(b"data");
        body.extend_from_slice(&3u32.to_le_bytes());
        body.extend_from_slice(&[0x80, 0x81, 0x82]);
        let mut v = b"RIFF".to_vec();
        v.extend_from_slice(&(body.len() as u32).to_le_bytes());
        v.extend_from_slice(&body);

        let d = Document::new(MemSource(v.clone()));
        let mut ev = Evaluator::new(wav());
        // The whole file is covered, to its last byte.
        let root = ev.node(&d, &[]).unwrap();
        assert_eq!(root.size_bits, v.len() as u64 * 8);
        let data = ev.node(&d, &[3, 1]).unwrap();
        assert_eq!(data.type_name, "Chunk");
        assert_eq!(data.size_bits, (8 + 3) * 8);
    }

    pub(super) fn sample() -> Vec<u8> {
        let mut fmt = Vec::new();
        fmt.extend_from_slice(&1u16.to_le_bytes()); // pcm
        fmt.extend_from_slice(&1u16.to_le_bytes()); // mono
        fmt.extend_from_slice(&500_000u32.to_le_bytes());
        fmt.extend_from_slice(&1_000_000u32.to_le_bytes());
        fmt.extend_from_slice(&2u16.to_le_bytes());
        fmt.extend_from_slice(&16u16.to_le_bytes());

        // A GUANO block, ending with a NUL inside the chunk size, as recorders write.
        let mut guano = b"GUANO|Version:1.0\nMake:Wildlife Acoustics, Inc.\nLoc Position:-26.46550 31.94508\n".to_vec();
        guano.push(0);

        let mut wamd = Vec::new();
        wamd.extend_from_slice(&1u16.to_le_bytes());
        wamd.extend_from_slice(&9u32.to_le_bytes());
        wamd.extend_from_slice(b"SM4BAT-FS");

        // One cue point, and a LIST of INFO text.
        let mut cue = 1u32.to_le_bytes().to_vec();
        cue.extend_from_slice(&7u32.to_le_bytes()); // id
        cue.extend_from_slice(&1000u32.to_le_bytes()); // position
        cue.extend_from_slice(b"data");
        cue.extend_from_slice(&0u32.to_le_bytes());
        cue.extend_from_slice(&0u32.to_le_bytes());
        cue.extend_from_slice(&1000u32.to_le_bytes()); // sample offset

        let mut list = b"INFO".to_vec();
        list.extend_from_slice(&chunk_bytes(b"IART", b"Wildlife Acoustics\0"));

        let mut body = b"WAVE".to_vec();
        body.extend_from_slice(&chunk_bytes(b"fmt ", &fmt));
        body.extend_from_slice(&chunk_bytes(b"data", &[0x11; 6]));
        body.extend_from_slice(&chunk_bytes(b"cue ", &cue));
        body.extend_from_slice(&chunk_bytes(b"LIST", &list));
        body.extend_from_slice(&chunk_bytes(b"guan", &guano));
        body.extend_from_slice(&chunk_bytes(b"wamd", &wamd));

        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    /// Text in a fixed field, padded out with NULs.
    fn padded(v: &mut Vec<u8>, s: &[u8], n: usize) {
        let mut f = s.to_vec();
        f.resize(n, 0);
        v.extend_from_slice(&f);
    }

    /// A version 2 broadcast extension: 602 fixed bytes, then coding history.
    fn bext_bytes() -> Vec<u8> {
        let mut v = Vec::new();
        padded(&mut v, b"Nightjar, 2 km east of the dam", 256);
        padded(&mut v, b"Song Meter SM4BAT", 32);
        padded(&mut v, b"ZA-WLA-20260823-01", 32);
        padded(&mut v, b"2026-08-23", 10);
        padded(&mut v, b"21:14:07", 8);
        v.extend_from_slice(&76_412_928u32.to_le_bytes()); // samples since midnight
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        v.extend_from_slice(&[0x06; 64]); // umid
        for x in [-2300i16, 700, -150, -1800, -2000] {
            v.extend_from_slice(&x.to_le_bytes());
        }
        v.extend_from_slice(&[0u8; 180]);
        v.extend_from_slice(b"A=PCM,F=256000,W=16,M=mono");
        v
    }

    /// A tag as an MP3 carries it, here inside a chunk instead.
    fn id3_bytes() -> Vec<u8> {
        let mut frame = b"TIT2".to_vec();
        let text = b"\x03Dawn chorus";
        frame.extend_from_slice(&(text.len() as u32).to_be_bytes());
        frame.extend_from_slice(&[0, 0]);
        frame.extend_from_slice(text);

        let mut v = b"ID3".to_vec();
        v.extend_from_slice(&[3, 0, 0]);
        let n = frame.len();
        v.extend_from_slice(&[(n >> 21) as u8 & 0x7f, (n >> 14) as u8 & 0x7f, (n >> 7) as u8 & 0x7f, n as u8 & 0x7f]);
        v.extend_from_slice(&frame);
        v
    }

    /// The other half of what WAVE files hold: a studio file rather than a
    /// recorder's, with the extensible header and the chunks an editor writes.
    fn studio() -> Vec<u8> {
        let mut fmt = Vec::new();
        fmt.extend_from_slice(&0xfffeu16.to_le_bytes());
        fmt.extend_from_slice(&2u16.to_le_bytes());
        fmt.extend_from_slice(&48_000u32.to_le_bytes());
        fmt.extend_from_slice(&288_000u32.to_le_bytes());
        fmt.extend_from_slice(&6u16.to_le_bytes());
        fmt.extend_from_slice(&24u16.to_le_bytes());
        fmt.extend_from_slice(&22u16.to_le_bytes()); // extension size
        fmt.extend_from_slice(&24u16.to_le_bytes()); // valid bits
        fmt.extend_from_slice(&3u32.to_le_bytes()); // stereo
        // The PCM sub-format GUID: a format tag, then the fixed rest of it.
        fmt.extend_from_slice(&[1, 0, 0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71]);

        let mut smpl = Vec::new();
        for x in [0u32, 0, 20_833, 60, 0, 0, 0, 1, 0] {
            smpl.extend_from_slice(&x.to_le_bytes());
        }
        for x in [0u32, 1, 4_800, 96_000, 0, 0] {
            smpl.extend_from_slice(&x.to_le_bytes()); // one alternating loop
        }

        let inst = [60u8, 250, 251, 48, 72, 1, 127]; // fine tune -6, gain -5

        let mut adtl = b"adtl".to_vec();
        let mut labl = 7u32.to_le_bytes().to_vec();
        labl.extend_from_slice(b"Take 3\0");
        adtl.extend_from_slice(&chunk_bytes(b"labl", &labl));
        let mut ltxt = 7u32.to_le_bytes().to_vec();
        ltxt.extend_from_slice(&4_800u32.to_le_bytes());
        ltxt.extend_from_slice(b"rgn ");
        ltxt.extend_from_slice(&[0; 8]);
        ltxt.extend_from_slice(b"Chorus");
        adtl.extend_from_slice(&chunk_bytes(b"ltxt", &ltxt));

        let mut body = b"WAVE".to_vec();
        body.extend_from_slice(&chunk_bytes(b"fmt ", &fmt));
        body.extend_from_slice(&chunk_bytes(b"bext", &bext_bytes()));
        body.extend_from_slice(&chunk_bytes(b"iXML", b"<BWFXML><PROJECT>Dam</PROJECT></BWFXML>"));
        body.extend_from_slice(&chunk_bytes(b"smpl", &smpl));
        body.extend_from_slice(&chunk_bytes(b"inst", &inst));
        body.extend_from_slice(&chunk_bytes(b"LIST", &adtl));
        body.extend_from_slice(&chunk_bytes(b"id3 ", &id3_bytes()));
        body.extend_from_slice(&chunk_bytes(b"data", &[0x22; 12]));

        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    #[test]
    fn a_studio_file_reads_down_to_its_metadata() {
        let d = Document::new(MemSource(studio()));
        let mut ev = Evaluator::new(wav());
        assert_eq!(ev.node(&d, &[3]).unwrap().child_count, 8);

        // The extensible header names the same format twice: once as a tag,
        // once at the front of the sub-format GUID.
        assert_eq!(ev.node(&d, &[3, 0, 2, 6]).unwrap().type_name, "Extensible");
        assert_eq!(
            ev.node(&d, &[3, 0, 2, 6, 2]).unwrap().value,
            Value::Enum { raw: 3, name: Some("stereo".into()), hex: true }
        );
        assert_eq!(
            ev.node(&d, &[3, 0, 2, 6, 3, 0]).unwrap().value,
            Value::Enum { raw: 1, name: Some("pcm".into()), hex: true }
        );

        // Broadcast extension, version 2: the UMID and the loudness values are
        // both there, and the reserved area is what they left.
        assert_eq!(ev.node(&d, &[3, 1, 2]).unwrap().type_name, "Broadcast");
        assert_eq!(
            ev.node(&d, &[3, 1, 2, 0]).unwrap().value,
            Value::Str("Nightjar, 2 km east of the dam".into())
        );
        assert_eq!(ev.node(&d, &[3, 1, 2, 7]).unwrap().value, Value::UInt(2));
        assert_eq!(ev.node(&d, &[3, 1, 2, 8]).unwrap().size_bits, 64 * 8);
        assert_eq!(ev.node(&d, &[3, 1, 2, 9, 0]).unwrap().value, Value::Int(-2300));
        assert_eq!(ev.node(&d, &[3, 1, 2, 10]).unwrap().size_bits, 180 * 8);
        assert_eq!(
            ev.node(&d, &[3, 1, 2, 11]).unwrap().value,
            Value::Str("A=PCM,F=256000,W=16,M=mono".into())
        );

        // A version 0 extension has no UMID, and 254 bytes reserved instead.
        // The coding history lands in the same place either way.
        let mut v0 = studio();
        let version_at = v0.windows(4).position(|w| w == b"bext").unwrap() + 8 + 346;
        v0[version_at] = 0;
        let d0 = Document::new(MemSource(v0));
        let mut ev0 = Evaluator::new(wav());
        assert_eq!(ev0.node(&d0, &[3, 1, 2, 8]).unwrap().size_bits, 0);
        assert_eq!(ev0.node(&d0, &[3, 1, 2, 10]).unwrap().size_bits, 254 * 8);
        assert_eq!(
            ev0.node(&d0, &[3, 1, 2, 11]).unwrap().value,
            Value::Str("A=PCM,F=256000,W=16,M=mono".into())
        );

        assert_eq!(
            ev.node(&d, &[3, 2, 2]).unwrap().value,
            Value::Str("<BWFXML><PROJECT>Dam</PROJECT></BWFXML>".into())
        );

        // The sampler chunk and its one loop.
        assert_eq!(ev.node(&d, &[3, 3, 2, 7]).unwrap().value, Value::UInt(1));
        assert_eq!(ev.node(&d, &[3, 3, 2, 9]).unwrap().child_count, 1);
        assert_eq!(
            ev.node(&d, &[3, 3, 2, 9, 0, 1]).unwrap().value,
            Value::Enum { raw: 1, name: Some("alternating".into()), hex: false }
        );
        assert_eq!(ev.node(&d, &[3, 3, 2, 9, 0, 3]).unwrap().value, Value::UInt(96_000));

        // Instrument settings run either side of zero, so they are signed.
        assert_eq!(ev.node(&d, &[3, 4, 2, 1]).unwrap().value, Value::Int(-6));
        assert_eq!(ev.node(&d, &[3, 4, 2, 2]).unwrap().value, Value::Int(-5));

        // A labelled point and a labelled stretch, inside the adtl list.
        assert_eq!(ev.node(&d, &[3, 5, 2, 0]).unwrap().value, Value::Str("adtl".into()));
        assert_eq!(ev.node(&d, &[3, 5, 2, 1, 0, 2, 1]).unwrap().value, Value::Str("Take 3".into()));
        assert_eq!(ev.node(&d, &[3, 5, 2, 1, 1, 2]).unwrap().type_name, "LabelledText");
        assert_eq!(ev.node(&d, &[3, 5, 2, 1, 1, 2, 2]).unwrap().value, Value::Str("rgn ".into()));
        assert_eq!(ev.node(&d, &[3, 5, 2, 1, 1, 2, 7]).unwrap().value, Value::Str("Chorus".into()));

        // An ID3 tag in a chunk reads as the tag it is.
        assert_eq!(ev.node(&d, &[3, 6, 2]).unwrap().type_name, "ID3");
        assert_eq!(ev.node(&d, &[3, 6, 2, 8, 0, 0]).unwrap().value, Value::Str("TIT2".into()));
        assert_eq!(ev.node(&d, &[3, 6, 2, 8, 0, 3, 1]).unwrap().value, Value::Str("Dawn chorus".into()));
    }

    #[test]
    fn chunks_pad_to_even_and_guano_reads_as_lines() {
        let d = Document::new(MemSource(sample()));
        let mut ev = Evaluator::new(wav());
        assert_eq!(ev.node(&d, &[3]).unwrap().child_count, 6);

        let fmt = ev.node(&d, &[3, 0, 2]).unwrap();
        assert_eq!(fmt.type_name, "Format");
        assert_eq!(ev.node(&d, &[3, 0, 2, 0]).unwrap().value, Value::Enum { raw: 1, name: Some("pcm".into()), hex: true });
        assert_eq!(ev.node(&d, &[3, 0, 2, 2]).unwrap().value, Value::UInt(500_000));

        // The data chunk is six bytes, so no pad; guano is odd, so one.
        assert_eq!(ev.node(&d, &[3, 1, 3]).unwrap().size_bits, 0);
        assert_eq!(ev.node(&d, &[3, 4, 3]).unwrap().size_bits, 8);

        // A cue point, then a LIST whose members are chunks like any other.
        assert_eq!(ev.node(&d, &[3, 2, 2, 0]).unwrap().value, Value::UInt(1));
        assert_eq!(ev.node(&d, &[3, 2, 2, 1, 0, 1]).unwrap().value, Value::UInt(1000));
        assert_eq!(ev.node(&d, &[3, 3, 2, 0]).unwrap().value, Value::Str("INFO".into()));
        let item = ev.node(&d, &[3, 3, 2, 1, 0]).unwrap();
        assert_eq!(item.type_name, "Chunk");
        assert_eq!(ev.node(&d, &[3, 3, 2, 1, 0, 0]).unwrap().value, Value::Str("IART".into()));
        assert_eq!(ev.node(&d, &[3, 3, 2, 1, 0, 2]).unwrap().value, Value::Str("Wildlife Acoustics".into()));

        // GUANO lines.
        let lines = ev.node(&d, &[3, 4, 2]).unwrap();
        assert_eq!(lines.child_count, 4); // three fields and the trailing NUL
        assert_eq!(ev.node(&d, &[3, 4, 2, 0]).unwrap().value, Value::Str("GUANO|Version:1.0".into()));
        assert_eq!(
            ev.node(&d, &[3, 4, 2, 2]).unwrap().value,
            Value::Str("Loc Position:-26.46550 31.94508".into())
        );

        // wamd is a stream of tagged items.
        let wamd = ev.node(&d, &[3, 5, 2, 0]).unwrap();
        assert_eq!(wamd.child_count, 3);
        assert_eq!(
            ev.node(&d, &[3, 5, 2, 0, 0]).unwrap().value,
            Value::Enum { raw: 1, name: Some("model".into()), hex: false }
        );
        assert_eq!(ev.node(&d, &[3, 5, 2, 0, 2]).unwrap().value, Value::Str("SM4BAT-FS".into()));
    }
    /// A `fmt ` chunk, something in between, and then samples: the width comes
    /// from the chunk that declared it however far back it sits.
    fn with_samples(bits: u16, format: u16, channels: u16, body: &[u8]) -> Vec<u8> {
        let mut fmt = Vec::new();
        fmt.extend_from_slice(&format.to_le_bytes());
        fmt.extend_from_slice(&channels.to_le_bytes());
        fmt.extend_from_slice(&44_100u32.to_le_bytes());
        fmt.extend_from_slice(&0u32.to_le_bytes()); // byte rate, not read here
        let align = channels * bits / 8;
        fmt.extend_from_slice(&align.to_le_bytes());
        fmt.extend_from_slice(&bits.to_le_bytes());
        let mut inner = chunk_bytes(b"fmt ", &fmt);
        // A chunk between the two, which is what `Prev` could not see past.
        inner.extend_from_slice(&chunk_bytes(b"fact", &1234u32.to_le_bytes()));
        inner.extend_from_slice(&chunk_bytes(b"data", body));
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&((inner.len() + 4) as u32).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(&inner);
        out
    }

    #[test]
    fn samples_are_read_as_the_width_an_earlier_chunk_declared() {
        // 16-bit stereo: four frames of two samples each.
        let body: Vec<u8> = [0i16, -1, 32767, -32768, 100, -100, 7, 8]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let d = Document::new(MemSource(with_samples(16, 1, 2, &body)));
        let mut ev = Evaluator::new(wav());
        // chunks[2] is data; its body is the frames.
        // Two channels interleaved: eight samples, left and right in turn.
        let samples = ev.node(&d, &[3, 2, 2]).unwrap();
        assert_eq!(samples.child_count, 8);
        assert_eq!(ev.node(&d, &[3, 2, 2, 1]).unwrap().value, Value::Int(-1));
        assert_eq!(ev.node(&d, &[3, 2, 2, 2]).unwrap().value, Value::Int(32767));
        assert_eq!(ev.node(&d, &[3, 2, 2, 7]).unwrap().value, Value::Int(8));
    }

    #[test]
    fn float_samples_read_as_floats_and_8_bit_ones_as_unsigned() {
        let body: Vec<u8> = [1.0f32, -0.5, 0.25].iter().flat_map(|v| v.to_le_bytes()).collect();
        let d = Document::new(MemSource(with_samples(32, 3, 1, &body)));
        let mut ev = Evaluator::new(wav());
        assert_eq!(ev.node(&d, &[3, 2, 2]).unwrap().child_count, 3);
        assert_eq!(ev.node(&d, &[3, 2, 2, 1]).unwrap().value, Value::Float(-0.5));

        // 8-bit PCM is unsigned, with 128 for silence.
        let d = Document::new(MemSource(with_samples(8, 1, 1, &[0, 128, 255])));
        let mut ev = Evaluator::new(wav());
        assert_eq!(ev.node(&d, &[3, 2, 2]).unwrap().child_count, 3);
        assert_eq!(ev.node(&d, &[3, 2, 2, 1]).unwrap().value, Value::UInt(128));
    }

    #[test]
    fn data_with_no_format_to_read_it_by_stays_bytes() {
        // No `fmt ` at all: nothing says what the bytes are, so they stay bytes
        // rather than being read as a width nobody declared.
        let mut inner = chunk_bytes(b"data", &[1, 2, 3, 4, 5, 6, 7, 8]);
        inner.extend_from_slice(&chunk_bytes(b"fact", &9u32.to_le_bytes()));
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&((inner.len() + 4) as u32).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(&inner);
        let d = Document::new(MemSource(out));
        let mut ev = Evaluator::new(wav());
        let body = ev.node(&d, &[3, 0, 2]).unwrap();
        assert!(!body.composite);
        assert_eq!(body.size_bits, 8 * 8);
    }

}
