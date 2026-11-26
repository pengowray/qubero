//! MP4 (ISO base media file format, ISO/IEC 14496-12), and the QuickTime
//! movie it grew out of.
//!
//! Every box is size, four-character type, payload. Container boxes hold more
//! boxes, which is why the IR grew `Ty::Named`: `Box` refers to itself through
//! the template's type table.
//!
//! What is here: the box tree, the movie, track and media headers in both
//! their 32-bit and 64-bit versions, edit lists, data references, QuickTime and
//! iTunes metadata, Nero chapters, and Blackmagic RAW's per-frame metadata. The sample
//! descriptions and the decoder configurations inside them are in
//! [`entries`], the sample tables in [`tables`], movie fragments in
//! [`fragments`], HEIF and AVIF items in [`items`], what a sample holds in
//! [`media`], and the exp-Golomb coded H.264 headers in [`golomb`]. The
//! sample tables, the fragments and the item locations place every sample
//! and item over the `mdat` it sits in, so a byte of media data reads as the
//! sample it belongs to.
//!
//! What is not: coded audio and video past their headers, the parts of an
//! H.264 header that come after an emulation prevention byte, and proprietary
//! compressed BRAW essence.

mod entries;
mod fragments;
mod golomb;
mod items;
mod media;
mod tables;

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Time, Ty as T, Until};

/// A four-character box type as the big-endian number the IR compares against.
pub(super) fn cc(s: &str) -> i128 {
    s.bytes().fold(0i128, |acc, b| (acc << 8) | b as i128)
}

pub(super) fn u32be() -> T {
    T::u32(Big)
}

pub(super) fn u16be() -> T {
    T::u16(Big)
}

pub(super) fn i16be() -> T {
    T::Int { bits: 16, endian: Big }
}

pub(super) fn i32be() -> T {
    T::Int { bits: 32, endian: Big }
}

pub(super) fn bits(n: u32) -> T {
    T::UInt { bits: n, endian: Big }
}

/// A four-character code read as the letters it is. Latin-1, because Apple
/// writes the copyright sign that starts `©nam` and `©xyz` as the one byte
/// 0xa9.
pub(super) fn fourcc() -> T {
    T::text(StrLen::Fixed(E::lit(4)), Encoding::Latin1)
}

/// Signed fixed point: `signed(32, 16)` is MP4's 16.16 and `signed(32, 30)`
/// the 2.30 of a matrix's last column.
pub(super) fn signed(bits: u32, frac: u32) -> T {
    T::Fixed { bits, frac, endian: Big, signed: true }
}

/// `v0` in a version 0 box and `v1` in a version 1 box: what every header
/// with a 64-bit variant switches on.
pub(super) fn by_version(v0: T, v1: T) -> T {
    T::switch(E::field("version"), vec![(1, v1)], v0)
}

/// Boxes until the end of the box they are in.
pub(super) fn child_boxes() -> T {
    T::repeat(T::Named("Box".into()), Until::End)
}

/// Boxes whose payload is nothing but more boxes. `moof` is not here: its
/// boxes are read inside an origin, so that a track run can count its data
/// offset from the start of the fragment.
const CONTAINERS: &[&str] = &[
    "moov", "trak", "mdia", "minf", "edts", "dinf", "udta", "mvex", "traf", "mfra", "tref", "gmhd", "wave", "sinf", "schi",
    "iprp", "ipco", "meco", "clip", "matt", "imap", "tapt", "rinf",
];

pub fn mp4() -> Template {
    template_named("mp4")
}

pub(crate) fn template_named(name: &str) -> Template {
    Template::new(name, T::repeat(T::Named("Box".into()), Until::End))
        .with_type("Box", boxes())
        .with_type("BoxBody", payload())
        // Declared once each, rather than once for every format and box
        // that holds one.
        .with_type("VisualSampleEntry", entries::visual_sample_entry())
        .with_type("AudioSampleEntry", entries::audio_sample_entry())
        .with_type("SampleContent", media::sample_content())
        .with_type("H264Sample", media::h264_sample())
        .with_type("HevcSample", media::hevc_sample())
        .with_type("H264NalUnit", media::h264_nal_unit())
        .with_type("HevcNalUnit", media::hevc_nal_unit())
        .with_type("BrawFrame", braw_frame())
        .with_type("TrackReference", track_reference())
}

/// A box, and an `mdat` as a box of its own kind.
///
/// An `mdat`'s body is a second reading of its bytes: the samples placed over
/// it from the sample tables, the track runs and the item locations are what
/// those bytes are, and the body is what is left where none is. That is said
/// on the box rather than inside its body, so that a run of samples filling
/// the whole body is still narrower than what holds the second reading, and
/// a byte of it still reads as the sample.
fn boxes() -> T {
    let is_mdat = E::Remaining.greater_or_equal(E::lit(8)).both(E::peek_at(E::lit(32), 32, Big).equal_to(E::lit(cc("mdat"))));
    T::switch(
        is_mdat,
        vec![(
            1,
            box_of("MediaDataBox", media_data())
                .field_aside("body")
                .field_doc("body", "Sample data. The sample tables in moov, the track runs in each moof, or the item locations in meta say which sample or item each byte belongs to."),
        )],
        box_of("Box", T::Named("BoxBody".into())),
    )
}

/// A box whose payload is read as `payload`, in a window as long as the box
/// says. The payload is declared once and named, rather than written out in
/// each of the three ways a box can give its size.
fn box_of(name: &str, payload: T) -> T {
    // A box is 8 bytes of header, or 16 when size is 1 and the real size is a
    // 64-bit field after the type.
    let short = E::field("size").sub(E::lit(8));
    let long = E::field("largesize").sub(E::lit(16));
    T::structure_named(
        name,
        "type",
        "body",
        vec![
            ("size", u32be()),
            ("type", fourcc()),
            (
                "body",
                T::switch(
                    E::field("size"),
                    vec![
                        // Size 0 means "to the end of the file". The spec allows
                        // it only for the last box; a file that uses it anywhere
                        // else really does swallow what follows.
                        (0, T::sized(E::Remaining, payload.clone())),
                        (
                            1,
                            T::structure(
                                "LargeBox",
                                vec![("largesize", T::u64(Big)), ("payload", T::sized(long, payload.clone()))],
                            ),
                        ),
                    ],
                    T::sized(short, payload),
                ),
            ),
        ],
    )
    .counted_as("box")
}

/// What is inside a box, chosen by its type. Read in a window as long as the
/// box's payload, so `Remaining` at its start is the payload's length.
fn payload() -> T {
    let mut cases: Vec<(i128, T)> = CONTAINERS.iter().map(|c| (cc(c), child_boxes())).collect();
    cases.extend(vec![
        (cc("ftyp"), ftyp()),
        (cc("styp"), ftyp()),
        (cc("mvhd"), mvhd()),
        (cc("tkhd"), tkhd()),
        (cc("mdhd"), mdhd()),
        (cc("hdlr"), hdlr()),
        (cc("elst"), elst()),
        (cc("dref"), dref()),
        (cc("url "), data_entry_url()),
        (cc("urn "), data_entry_urn()),
        (cc("vmhd"), vmhd()),
        (cc("smhd"), smhd()),
        (cc("nmhd"), T::structure("NullMediaHeader", full_box())),
        (cc("sthd"), T::structure("SubtitleMediaHeader", full_box())),
        (cc("gmin"), gmin()),
        // QuickTime's text media information, inside gmhd: only a matrix.
        (cc("text"), T::structure("TextMediaInfo", vec![("matrix", matrix())])),
        (cc("tmcd"), timecode_media_info()),
        (cc("tcmi"), timecode_media_info_body()),
        (cc("stbl"), tables::sample_table()),
        (cc("stsd"), entries::stsd()),
        (cc("meta"), metadata()),
        (cc("keys"), metadata_keys()),
        (cc("ilst"), metadata_items()),
        (cc("cprt"), copyright()),
        (cc("chpl"), chapter_list()),
        (
            cc("encd"),
            T::structure("TextEncoding", vec![("encoding", u32be())])
                .field_doc("encoding", "Which text encoding the sample's text is in. ffmpeg writes 0x00000100 here to mark its chapter titles as UTF-8."),
        ),
        (cc("iods"), entries::iods()),
        (cc("uuid"), uuid()),
        (cc("mogy"), motion_vector("Gyroscope")),
        (cc("moac"), motion_vector("Accelerometer")),
        (cc("free"), unused()),
        (cc("skip"), unused()),
        (cc("wide"), unused()),
        (cc("frma"), T::structure("OriginalFormat", vec![("data_format", fourcc())])),
    ]);
    for (kind, ty) in tables::boxes() {
        cases.push((cc(kind), ty));
    }
    for (kind, ty) in fragments::boxes() {
        cases.push((cc(kind), ty));
    }
    for (kind, ty) in entries::boxes() {
        cases.push((cc(kind), ty));
    }
    for (kind, ty) in items::boxes() {
        cases.push((cc(kind), ty));
    }
    for kind in TRACK_REFERENCES {
        cases.push((cc(kind), T::Named("TrackReference".into())));
    }
    // A QuickTime user data item whose type starts with the copyright sign is
    // text: `©nam` the title, `©day` the date, `©xyz` where it was filmed.
    // The type reads as Latin-1, and as a number it is the UTF-8 of that
    // text: the sign is the two bytes c2 a9, and three letters follow it.
    let quicktime_text = T::switch(E::field("type").shr(E::lit(24)), vec![(0xc2a9, user_data_text())], T::bytes(E::Remaining));
    T::switch(E::field("type"), cases, quicktime_text)
}

fn ftyp() -> T {
    T::structure(
        "FileType",
        vec![
            ("major_brand", fourcc()),
            ("minor_version", u32be()),
            ("compatible_brands", T::array(fourcc(), E::Remaining.div(E::lit(4)))),
        ],
    )
}

/// version and flags, the first four bytes of every "full box".
pub(super) fn full_box() -> Vec<(&'static str, T)> {
    vec![("version", T::u8()), ("flags", bits(24))]
}

/// The same, for a box whose flags have names: bit 0 is the least significant
/// bit of the 24.
pub(super) fn full_box_flags(name: &str, named: &[(u32, &str)]) -> Vec<(&'static str, T)> {
    vec![("version", T::u8()), ("flags", T::flags(name, bits(24), named))]
}

/// The two dates a movie, track or media header begins with, said once for all
/// six of the structures that carry them.
///
/// Seconds from 1904-01-01, which is what ISO 14496-12 says and what QuickTime
/// said before it, and in UTC by the same word. Zero is what a muxer writes
/// when it will not say when the file was made, which is ffmpeg's default and
/// therefore most of the MP4s anyone has; it is not the first instant of 1904.
fn mac_times(ty: T) -> T {
    ty.field_times(&["creation_time", "modification_time"], Time::mac().unset(0))
}

/// The 3 by 3 matrix a movie or track header ends with, which maps the video
/// onto the screen. `a b u / c d v / x y w`, applied to the row vector
/// `(x, y, 1)`: `a` to `d` scale and rotate, `x` and `y` translate, and `u`,
/// `v`, `w` are 2.30 fixed point rather than 16.16.
fn matrix() -> T {
    let f = || signed(32, 16);
    let g = || signed(32, 30);
    T::structure(
        "Matrix",
        vec![("a", f()), ("b", f()), ("u", g()), ("c", f()), ("d", f()), ("v", g()), ("x", f()), ("y", f()), ("w", g())],
    )
    .doc("How the video is placed on the screen, as the matrix a b u / c d v / x y w. 1 0 0 / 0 1 0 / 0 0 1 is no change. A video turned a quarter turn has a and d at 0, and b and c at 1 and -1.")
}

fn mvhd() -> T {
    let v0 = mac_times(T::structure(
        "MovieTimes32",
        vec![
            ("creation_time", u32be()),
            ("modification_time", u32be()),
            ("timescale", u32be()),
            ("duration", u32be()),
        ],
    ));
    let v1 = mac_times(T::structure(
        "MovieTimes64",
        vec![
            ("creation_time", T::u64(Big)),
            ("modification_time", T::u64(Big)),
            ("timescale", u32be()),
            ("duration", T::u64(Big)),
        ],
    ));
    let mut fields = full_box();
    fields.extend(vec![
        ("times", by_version(v0, v1)),
        ("rate", T::fixed(32, 16, Big)),
        ("volume", T::fixed(16, 8, Big)),
        ("reserved", u16be()),
        ("reserved2", T::array(u32be(), E::lit(2))),
        ("matrix", matrix()),
        // Six words that ISO 14496-12 calls pre_defined and fills with zeros.
        // QuickTime gives them these names.
        ("preview_time", u32be()),
        ("preview_duration", u32be()),
        ("poster_time", u32be()),
        ("selection_time", u32be()),
        ("selection_duration", u32be()),
        ("current_time", u32be()),
        ("next_track_id", u32be()),
    ]);
    T::structure("MovieHeader", fields)
        .field_doc("rate", "Preferred playback rate. 1.0 is normal speed.")
        .field_doc("volume", "Preferred volume. 1.0 is full volume.")
        .field_doc("preview_time", "QuickTime only: when the preview starts, in the movie's timescale. ISO 14496-12 calls the six 32-bit fields from here to current_time pre_defined and requires zero.")
        .field_doc("next_track_id", "One more than the largest track ID in use, for a program that adds a track.")
}

fn tkhd() -> T {
    let v0 = mac_times(T::structure(
        "TrackTimes32",
        vec![
            ("creation_time", u32be()),
            ("modification_time", u32be()),
            ("track_id", u32be()),
            ("reserved", u32be()),
            ("duration", u32be()),
        ],
    ));
    let v1 = mac_times(T::structure(
        "TrackTimes64",
        vec![
            ("creation_time", T::u64(Big)),
            ("modification_time", T::u64(Big)),
            ("track_id", u32be()),
            ("reserved", u32be()),
            ("duration", T::u64(Big)),
        ],
    ));
    let mut fields = full_box_flags("TrackFlags", &[(0, "enabled"), (1, "in movie"), (2, "in preview"), (3, "size is aspect ratio")]);
    fields.extend(vec![
        ("times", by_version(v0, v1)),
        ("reserved2", T::array(u32be(), E::lit(2))),
        ("layer", i16be()),
        ("alternate_group", i16be()),
        ("volume", T::fixed(16, 8, Big)),
        ("reserved3", u16be()),
        ("matrix", matrix()),
        ("width", T::fixed(32, 16, Big)),
        ("height", T::fixed(32, 16, Big)),
    ]);
    T::structure("TrackHeader", fields)
        .field_doc("layer", "Front-to-back order of video tracks. Lower numbers are closer to the viewer.")
        .field_doc("alternate_group", "Tracks with the same nonzero number are alternatives to each other, such as audio in different languages.")
        .field_doc("volume", "1.0 for an audio track, 0 for any other.")
        .field_doc("width", "Display width in pixels: the size each picture is scaled to before the matrix is applied.")
}

fn mdhd() -> T {
    let v0 = mac_times(T::structure(
        "MediaTimes32",
        vec![
            ("creation_time", u32be()),
            ("modification_time", u32be()),
            ("timescale", u32be()),
            ("duration", u32be()),
        ],
    ));
    let v1 = mac_times(T::structure(
        "MediaTimes64",
        vec![
            ("creation_time", T::u64(Big)),
            ("modification_time", T::u64(Big)),
            ("timescale", u32be()),
            ("duration", T::u64(Big)),
        ],
    ));
    let mut fields = full_box();
    // The language is three five-bit letters, each offset from 0x60.
    fields.extend(vec![
        ("times", by_version(v0, v1)),
        ("pad", bits(1)),
        ("language", T::array(bits(5), E::lit(3))),
        ("pre_defined", u16be()),
    ]);
    T::structure("MediaHeader", fields)
        .field_doc("language", "ISO 639-2/T code, each letter less 0x60: 5, 14, 7 is \"eng\". QuickTime may write a Macintosh language code here instead, which is below 0x400, or 0x7fff for none.")
        .field_doc("pre_defined", "QuickTime calls this quality.")
}

fn hdlr() -> T {
    let mut fields = full_box();
    fields.extend(vec![
        ("component_type", fourcc()),
        // The handler is four characters, and they read as themselves.
        ("handler_type", fourcc()),
        ("component_manufacturer", fourcc()),
        ("component_flags", u32be()),
        ("component_flags_mask", u32be()),
        // QuickTime writes the name with a length byte in front, ISO with a NUL
        // after. A first byte that is exactly the length of the rest is the
        // QuickTime form; a C string's first byte is its first letter.
        (
            "name",
            T::switch(
                E::Remaining.greater_than(E::lit(0)).both(E::peek(8, Big).add(E::lit(1)).equal_to(E::Remaining)),
                vec![(1, pascal_string())],
                // Padded rather than terminated, so a box whose writer left the
                // NUL out still reads.
                T::utf8_padded(E::Remaining, 0),
            ),
        ),
    ]);
    T::structure("Handler", fields)
        .field_doc("component_type", "QuickTime: mhlr for a media handler, dhlr for a data handler. ISO 14496-12 calls this pre_defined and requires zero.")
        .field_doc("handler_type", "What the track or meta box holds. vide: video. soun: sound. subt or sbtl: subtitles. tmcd: timecode. pict: a HEIF or AVIF image. mdir: iTunes metadata. mdta: QuickTime metadata.")
        .field_doc("component_manufacturer", "QuickTime only; ISO 14496-12 reserves this field and the two 32-bit fields after it, and requires zero.")
}

/// A length byte and that many characters: how QuickTime writes a name.
fn pascal_string() -> T {
    T::structure("PascalString", vec![("length", T::u8()), ("text", T::utf8(E::field("length")))]).named_by("text")
}

fn elst() -> T {
    let entry = T::structure(
        "EditListEntry",
        vec![
            ("segment_duration", by_version(u32be(), T::u64(Big))),
            (
                "media_time",
                by_version(
                    T::enumeration("MediaTime", i32be(), &[(-1, "empty edit")]),
                    T::enumeration("MediaTime", T::Int { bits: 64, endian: Big }, &[(-1, "empty edit")]),
                ),
            ),
            ("media_rate", signed(32, 16)),
        ],
    )
    .field_doc("segment_duration", "How long this edit plays, in the movie's timescale (mvhd).")
    .field_doc("media_time", "Where in the media this edit starts, in the media's timescale (mdhd). -1 is an empty edit: nothing plays for the duration.")
    .field_doc("media_rate", "Playback rate for the edit. 1.0: normal speed. 0: one frame is shown for the whole segment_duration.");
    tables::counted_full_box("EditList", entry)
}

fn dref() -> T {
    let mut fields = full_box();
    fields.extend(vec![("entry_count", u32be()), ("entries", T::array(T::Named("Box".into()), E::field("entry_count")))]);
    T::structure("DataReference", fields)
}

fn data_entry_url() -> T {
    let mut fields = full_box_flags("DataEntryFlags", &[(0, "in this file")]);
    fields.push(("location", T::utf8_padded(E::Remaining, 0)));
    T::structure("DataEntryUrl", fields)
        .field_doc("flags", "Bit 0 set: the media data is in this file, and no location string follows.")
}

fn data_entry_urn() -> T {
    let mut fields = full_box_flags("DataEntryFlags", &[(0, "in this file")]);
    fields.extend(vec![("name", T::cstr()), ("location", T::utf8_padded(E::Remaining, 0))]);
    T::structure("DataEntryUrn", fields)
}

fn vmhd() -> T {
    let mut fields = full_box();
    fields.extend(vec![("graphics_mode", u16be()), ("opcolor", T::array(u16be(), E::lit(3)))]);
    T::structure("VideoMediaHeader", fields)
        .field_doc("graphics_mode", "How the video is composited: 0 is copy, which draws the video over anything behind it.")
        .field_doc("opcolor", "Red, green and blue for the graphics modes that use a colour.")
}

fn smhd() -> T {
    let mut fields = full_box();
    fields.extend(vec![("balance", signed(16, 8)), ("reserved", u16be())]);
    T::structure("SoundMediaHeader", fields).field_doc("balance", "Stereo balance: 0 is centre, -1.0 full left, 1.0 full right.")
}

/// QuickTime's base media information, inside `gmhd`.
fn gmin() -> T {
    let mut fields = full_box();
    fields.extend(vec![
        ("graphics_mode", u16be()),
        ("opcolor", T::array(u16be(), E::lit(3))),
        ("balance", signed(16, 8)),
        ("reserved", u16be()),
    ]);
    T::structure("BaseMediaInfo", fields)
}

/// `tmcd` is a QuickTime timecode media information box inside `gmhd`, and a
/// track reference everywhere else. The one inside `gmhd` holds a `tcmi`.
fn timecode_media_info() -> T {
    let holds_tcmi = E::Remaining.greater_or_equal(E::lit(8)).both(E::peek_at(E::lit(32), 32, Big).equal_to(E::lit(cc("tcmi"))));
    T::switch(holds_tcmi, vec![(1, child_boxes())], T::Named("TrackReference".into()))
}

fn timecode_media_info_body() -> T {
    let mut fields = full_box();
    fields.extend(vec![
        ("text_font", u16be()),
        ("text_face", u16be()),
        ("text_size", u16be()),
        ("reserved", u16be()),
        ("text_color", T::array(u16be(), E::lit(3))),
        ("background_color", T::array(u16be(), E::lit(3))),
        ("font_name", pascal_string()),
    ]);
    T::structure("TimecodeMediaInfo", fields)
}

/// The kinds of reference a track can make to other tracks, inside `tref`.
/// Each is a list of track IDs.
const TRACK_REFERENCES: &[&str] = &[
    "chap", "hint", "cdsc", "font", "hind", "vdep", "vplx", "subt", "forc", "mpod", "dpnd", "ipir", "sync", "scpt", "ssrc",
    "thmb", "auxl", "fall", "folw",
];

fn track_reference() -> T {
    T::structure("TrackReference", vec![("track_ids", T::repeat(u32be(), Until::End))])
}

/// One item of QuickTime user data text: a length, a language, the text. An
/// item may hold the same text in several languages, one after another.
fn user_data_text() -> T {
    T::repeat(
        T::structure(
            "UserDataText",
            vec![("length", u16be()), ("language", u16be()), ("text", T::utf8(E::field("length").at_most(E::Remaining)))],
        )
        .named_by("text")
        .field_doc("language", "Below 0x400: a Macintosh language code (0 is English). From 0x400: an ISO 639-2/T code packed as three five-bit letters."),
        Until::End,
    )
}

/// Nero's chapter list, `chpl` in `moov/udta`: each chapter's start time and
/// title. ffmpeg writes it, with version 1, whenever a file has chapters, and
/// a QuickTime chapter track beside it for players that read only that.
fn chapter_list() -> T {
    let chapter = T::structure(
        "Chapter",
        vec![
            ("start", T::u64(Big)),
            ("start_seconds", T::computed_real(E::field("start").div(E::real(10_000_000.0)))),
            ("title_length", T::u8()),
            ("title", T::utf8(E::field("title_length").at_most(E::Remaining))),
        ],
    )
    .named_by("title")
    .field_doc("start", "When the chapter starts, in units of 100 nanoseconds from the start of the file.");
    let mut fields = full_box();
    fields.extend(vec![
        ("reserved", by_version(T::bytes(E::lit(0)), u32be())),
        ("chapter_count", T::u8()),
        ("chapters", T::array(chapter, E::field("chapter_count"))),
    ]);
    T::structure("ChapterList", fields).field_doc("reserved", "Four bytes in version 1, written as 0. Version 0 has none.")
}

fn copyright() -> T {
    let mut fields = full_box();
    fields.extend(vec![("pad", bits(1)), ("language", T::array(bits(5), E::lit(3))), ("notice", T::utf8_padded(E::Remaining, 0))]);
    T::structure("Copyright", fields)
}

fn uuid() -> T {
    T::structure("UserExtension", vec![("usertype", T::bytes(E::lit(16))), ("data", T::bytes(E::Remaining))])
        .field_doc("usertype", "A 16-byte UUID saying whose box this is.")
}

/// Free space: `free`, `skip`, and QuickTime's `wide`, which is eight bytes
/// kept so that an `mdat` after it can grow a 64-bit size without moving.
fn unused() -> T {
    T::bytes(E::Remaining)
}

/// Metadata: a handler saying which kind, then boxes. ISO 14496-12 makes `meta`
/// a full box; QuickTime's own `meta` has no version and flags, and its first
/// box starts straight away. The four bytes after a QuickTime `meta` header
/// are the type of that first box, `hdlr`.
fn metadata() -> T {
    let quicktime = E::Remaining.greater_or_equal(E::lit(8)).both(E::peek_at(E::lit(32), 32, Big).equal_to(E::lit(cc("hdlr"))));
    let mut fields = full_box();
    fields.push(("boxes", child_boxes()));
    T::switch(quicktime, vec![(1, child_boxes())], T::structure("Metadata", fields))
}

fn metadata_keys() -> T {
    let entry = T::structure(
        "MetadataKey",
        vec![
            ("size", u32be()),
            ("namespace", fourcc()),
            ("name", T::utf8(E::field("size").sub(E::lit(8)))),
        ],
    )
    .named_by("name")
    .field_doc("namespace", "mdta for a reverse-DNS key such as com.apple.quicktime.title, udta for a four-character one.");
    let mut fields = full_box();
    fields.extend(vec![("entry_count", u32be()), ("entries", T::array(entry, E::field("entry_count")))]);
    T::structure("MetadataKeys", fields)
}

/// The metadata items. Each is a box whose type is its key: four characters
/// in iTunes metadata (`©nam`, `trkn`, `covr`), or a 1-based index into
/// `keys` in QuickTime metadata. An index has a zero first byte and four
/// characters never do, which is how the two are told apart.
fn metadata_items() -> T {
    T::repeat(
        T::structure(
            "MetadataItem",
            vec![
                ("size", u32be()),
                ("key", T::switch(E::peek(8, Big).equal_to(E::lit(0)), vec![(1, u32be())], fourcc())),
                ("values", T::sized(E::field("size").sub(E::lit(8)).at_least(E::lit(0)).at_most(E::Remaining), T::repeat(item_box(), Until::End))),
            ],
        )
        .named_by("key")
        .field_doc("key", "Four characters for an iTunes item, or, for a QuickTime item, an index into the keys box, counting from 1."),
        Until::End,
    )
}

/// One box inside a metadata item: its value in `data`, and for a freeform
/// `----` item the `mean` and `name` that make up its key.
fn item_box() -> T {
    let body = E::field("size").sub(E::lit(8)).at_least(E::lit(0)).at_most(E::Remaining);
    let mut text = full_box();
    text.push(("text", T::utf8(E::Remaining)));
    T::structure_named(
        "ItemBox",
        "type",
        "body",
        vec![
            ("size", u32be()),
            ("type", fourcc()),
            (
                "body",
                T::sized(
                    body.clone(),
                    T::switch(
                        E::field("type"),
                        vec![(cc("data"), metadata_value()), (cc("mean"), T::structure("ItemMean", text.clone())), (cc("name"), T::structure("ItemName", text))],
                        T::bytes(body),
                    ),
                ),
            ),
        ],
    )
}

/// A `data` box: what type the value is, the locale, the value.
fn metadata_value() -> T {
    let int_by_size = |signed: bool| {
        let int = |bits: u32| if signed { T::Int { bits, endian: Big } } else { T::UInt { bits, endian: Big } };
        T::switch(E::Remaining, vec![(1, int(8)), (2, int(16)), (3, int(24)), (4, int(32)), (8, int(64))], T::bytes(E::Remaining))
    };
    // What an implicit value is, the iTunes way: by the key of the item.
    let key = E::within(&["key"]);
    let implicit = T::switch(
        key,
        vec![
            (cc("trkn"), numbered("TrackNumber", "track")),
            (cc("disk"), numbered("DiscNumber", "disc")),
            (cc("gnre"), T::structure("Genre", vec![("genre", u16be())]).field_doc("genre", "An ID3v1 genre number plus one.")),
        ],
        T::bytes(E::Remaining),
    );
    T::structure(
        "MetadataData",
        vec![
            ("type", T::enumeration("MetadataType", u32be(), METADATA_TYPES)),
            ("locale", u32be()),
            (
                "value",
                T::switch(
                    E::field("type"),
                    vec![
                        (0, implicit),
                        (1, T::utf8(E::Remaining)),
                        (2, T::text(StrLen::Fixed(E::Remaining), Encoding::Utf16(Big))),
                        (4, T::utf8(E::Remaining)),
                        (5, T::text(StrLen::Fixed(E::Remaining), Encoding::Utf16(Big))),
                        (21, int_by_size(true)),
                        (22, int_by_size(false)),
                        (23, T::F32(Big)),
                        (24, T::F64(Big)),
                        (65, T::Int { bits: 8, endian: Big }),
                        (66, i16be()),
                        (67, i32be()),
                        (70, T::array(T::F32(Big), E::lit(2))),
                        (71, T::array(T::F32(Big), E::lit(2))),
                        (72, T::array(T::F32(Big), E::lit(4))),
                        (74, T::Int { bits: 64, endian: Big }),
                        (75, T::u8()),
                        (76, u16be()),
                        (77, u32be()),
                        (78, T::u64(Big)),
                        (79, T::array(T::F64(Big), E::lit(9))),
                    ],
                    T::bytes(E::Remaining),
                ),
            ),
        ],
    )
    .field_doc("locale", "0 for every country and language; otherwise a country code and a language code, 16 bits each.")
}

/// iTunes' track and disc numbers: this one, and how many there are.
fn numbered(name: &str, what: &str) -> T {
    let count = format!("{what}_count");
    let fields: Vec<(&str, T)> = vec![("reserved", u16be()), (what, u16be()), (&count, u16be()), ("reserved2", T::bytes(E::Remaining))];
    T::structure(name, fields)
}

const METADATA_TYPES: &[(i128, &str)] = &[
    (0, "implicit"),
    (1, "UTF-8"),
    (2, "UTF-16"),
    (3, "Shift JIS"),
    (4, "UTF-8 sort"),
    (5, "UTF-16 sort"),
    (13, "JPEG"),
    (14, "PNG"),
    (21, "signed int"),
    (22, "unsigned int"),
    (23, "float32"),
    (24, "float64"),
    (27, "BMP"),
    (28, "QuickTime metadata box"),
    (65, "signed int8"),
    (66, "signed int16"),
    (67, "signed int32"),
    (70, "point/size (2 × float32)"),
    (71, "dimensions (2 × float32)"),
    (72, "rectangle (4 × float32)"),
    (74, "signed int64"),
    (75, "unsigned int8"),
    (76, "unsigned int16"),
    (77, "unsigned int32"),
    (78, "unsigned int64"),
    (79, "affine transform (9 × float64)"),
];

fn motion_vector(name: &'static str) -> T {
    T::structure(name, vec![("x", T::F32(Little)), ("y", T::F32(Little)), ("z", T::F32(Little))])
}

/// The media data, as bytes. The samples placed over it are what its bytes
/// are; see [`boxes`].
///
/// Blackmagic RAW's first sample is decoded here as well, when an `mdat`
/// starts with its `bmdf` marker.
fn media_data() -> T {
    let too_short = E::Remaining.less_than(E::lit(8));
    let marker = E::peek_at(E::lit(4 * 8), 32, Big);
    T::switch(
        too_short.or(marker),
        vec![
            (cc("bmdf"), first_braw_sample()),
            (cc("mogy"), motion_samples()),
            (cc("moac"), motion_samples()),
        ],
        T::bytes(E::Remaining),
    )
}

fn motion_samples() -> T {
    let body_len = E::field("size").sub(E::lit(8));
    T::repeat(
        T::structure(
            "BlackmagicMotionSample",
            vec![
                ("size", u32be()),
                ("type", fourcc()),
                (
                    "value",
                    T::sized(
                        body_len.clone(),
                        T::switch(
                            E::field("type"),
                            vec![(cc("mogy"), motion_vector("Gyroscope")), (cc("moac"), motion_vector("Accelerometer"))],
                            T::bytes(body_len),
                        ),
                    ),
                ),
            ],
        ),
        Until::End,
    )
}

fn first_braw_sample() -> T {
    let mut fields = braw_frame_fields();
    fields.push(("remaining_samples", T::bytes(E::Remaining)));
    T::structure("BrawMediaData", fields)
}

/// One Blackmagic RAW frame, as a sample the sample tables place: its
/// metadata, then the picture header, then the compressed picture.
pub(super) fn braw_frame() -> T {
    T::structure("BrawFrame", braw_frame_fields())
}

fn braw_frame_fields() -> Vec<(&'static str, T)> {
    vec![
        ("metadata_size", u32be()),
        ("metadata_magic", T::magic(b"bmdf")),
        (
            "frame_metadata",
            T::sized(E::field("metadata_size").sub(E::lit(8)), T::repeat(frame_metadata_atom(), Until::End)),
        ),
        ("picture_magic", T::magic(b"braw")),
        ("picture_size", u32be()),
        (
            "picture",
            T::sized(
                E::field("picture_size").sub(E::lit(8)),
                T::structure(
                    "BrawPicture",
                    vec![
                        ("sample_header", T::bytes(E::lit(12))),
                        ("width", u16be()),
                        ("height", u16be()),
                        ("slice_height", u16be()),
                        ("compressed_essence", T::bytes(E::Remaining)),
                    ],
                ),
            ),
        ),
    ]
}

fn frame_metadata_atom() -> T {
    let body_len = E::field("size").sub(E::lit(8));
    T::structure(
        "BrawFrameMetadata",
        vec![
            ("size", u32be()),
            ("type", fourcc()),
            (
                "value",
                T::sized(
                    body_len.clone(),
                    T::switch(
                        E::field("type"),
                        vec![
                            (cc("srte"), T::structure("SensorRate", vec![("numerator", u32be()), ("denominator", u32be())])),
                            (cc("innd"), T::F32(Big)),
                            (cc("agpf"), T::F32(Big)),
                            (cc("expo"), T::F32(Big)),
                            (cc("isoe"), u32be()),
                            (cc("wkel"), u32be()),
                            (cc("wtin"), u16be()),
                            (cc("asct"), u32be()),
                            (cc("asti"), u16be()),
                            (cc("shtv"), T::utf8_padded(body_len.clone(), 0)),
                            (cc("aptr"), T::utf8_padded(body_len.clone(), 0)),
                            (cc("dsnc"), T::utf8_padded(body_len.clone(), 0)),
                            (cc("fcln"), T::utf8_padded(body_len.clone(), 0)),
                        ],
                        T::bytes(body_len),
                    ),
                ),
            ),
        ],
    )
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    pub fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(body);
        v
    }

    /// A full box: version, flags, then the body.
    pub fn full(kind: &[u8; 4], version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
        let mut v = vec![version];
        v.extend_from_slice(&flags.to_be_bytes()[1..]);
        v.extend_from_slice(body);
        boxed(kind, &v)
    }

    pub fn cat(parts: &[&[u8]]) -> Vec<u8> {
        parts.concat()
    }

    /// The path of the field called `name` under `path`, by asking each child
    /// its name, so a test does not break when a field is added before it.
    pub fn child(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], name: &str) -> Vec<usize> {
        let n = ev.node(d, path).unwrap().child_count;
        for i in 0..n as usize {
            let mut p = path.to_vec();
            p.push(i);
            // A label is the field's name and then whatever names it.
            let label = ev.node(d, &p).unwrap().name;
            if label == name || label.starts_with(&format!("{name} ")) {
                return p;
            }
        }
        panic!("no child {name} under {path:?}");
    }

    /// Follow names down from the root: `walk(ev, d, &["1", "body", "0"])`,
    /// where a number is an index.
    pub fn walk(ev: &mut Evaluator, d: &Document<MemSource>, names: &[&str]) -> Vec<usize> {
        let mut p = Vec::new();
        for n in names {
            match n.parse::<usize>() {
                Ok(i) => p.push(i),
                Err(_) => p = child(ev, d, &p, n),
            }
        }
        p
    }

    pub fn value(ev: &mut Evaluator, d: &Document<MemSource>, names: &[&str]) -> Value {
        let p = walk(ev, d, names);
        ev.node(d, &p).unwrap().value
    }

    fn sample() -> Vec<u8> {
        let mut ftyp = b"isom".to_vec();
        ftyp.extend_from_slice(&512u32.to_be_bytes());
        ftyp.extend_from_slice(b"isomavc1");
        let mut mvhd = vec![0u8, 0, 0, 0]; // version 0, no flags
        mvhd.extend_from_slice(&0u32.to_be_bytes()); // creation
        mvhd.extend_from_slice(&0u32.to_be_bytes()); // modification
        mvhd.extend_from_slice(&600u32.to_be_bytes()); // timescale
        mvhd.extend_from_slice(&1200u32.to_be_bytes()); // duration
        mvhd.extend_from_slice(&0x0001_0000u32.to_be_bytes()); // rate
        mvhd.extend_from_slice(&0x0100u16.to_be_bytes()); // volume
        mvhd.extend_from_slice(&[0; 10]);
        // The identity matrix.
        for w in [0x10000u32, 0, 0, 0, 0x10000, 0, 0, 0, 0x4000_0000] {
            mvhd.extend_from_slice(&w.to_be_bytes());
        }
        mvhd.extend_from_slice(&[0; 24]);
        mvhd.extend_from_slice(&2u32.to_be_bytes()); // next track id

        let sps = [0x67u8, 0x42, 0xc0, 0x1e]; // NAL header 0x67 = SPS, ref_idc 3
        let mut avcc = vec![1u8, 0x42, 0xc0, 0x1e, 0xff, 0xe1];
        avcc.extend_from_slice(&(sps.len() as u16).to_be_bytes());
        avcc.extend_from_slice(&sps);
        avcc.push(0); // no PPS

        let mut hdlr = vec![0u8, 0, 0, 0]; // version, flags
        hdlr.extend_from_slice(&0u32.to_be_bytes()); // pre_defined
        hdlr.extend_from_slice(b"vide");
        hdlr.extend_from_slice(&[0; 12]); // reserved
        hdlr.extend_from_slice(b"VideoHandler\0");

        let mut moov = boxed(b"mvhd", &mvhd);
        moov.extend_from_slice(&boxed(b"hdlr", &hdlr));
        moov.extend_from_slice(&boxed(b"avcC", &avcc));

        let mut out = boxed(b"ftyp", &ftyp);
        out.extend_from_slice(&boxed(b"moov", &moov));
        out.extend_from_slice(&boxed(b"mdat", &[0xaa; 16]));
        out
    }

    #[test]
    fn box_tree_recurses_and_reads_headers() {
        let d = Document::new(MemSource(sample()));
        let mut ev = Evaluator::new(mp4());
        assert_eq!(ev.node(&d, &[]).unwrap().child_count, 3);
        assert_eq!(ev.node(&d, &[0, 1]).unwrap().value, Value::Str("ftyp".into()));
        assert_eq!(ev.node(&d, &[0, 2, 0]).unwrap().value, Value::Str("isom".into()));
        assert_eq!(ev.node(&d, &[0, 2, 2]).unwrap().child_count, 2);

        // moov holds boxes, reached through Ty::Named.
        assert_eq!(ev.node(&d, &[1, 2]).unwrap().child_count, 3);
        assert_eq!(ev.node(&d, &[1, 2, 0, 1]).unwrap().value, Value::Str("mvhd".into()));
        assert_eq!(ev.node(&d, &[1, 2, 0, 2, 2, 2]).unwrap().value, Value::UInt(600));
        assert_eq!(ev.node(&d, &[1, 2, 0, 2, 2, 3]).unwrap().value, Value::UInt(1200));

        // The matrix is nine numbers: a and d are 1.0, w is 1.0 in 2.30.
        let matrix = walk(&mut ev, &d, &["1", "body", "0", "body", "matrix"]);
        assert_eq!(ev.node(&d, &matrix).unwrap().child_count, 9);
        for (i, want) in [(0usize, 1.0f64), (1, 0.0), (4, 1.0), (8, 1.0)] {
            let mut p = matrix.clone();
            p.push(i);
            match ev.node(&d, &p).unwrap().value {
                Value::Float(f) => assert_eq!(f, want, "matrix[{i}]"),
                other => panic!("matrix[{i}] read as {other:?}"),
            }
        }
        assert_eq!(value(&mut ev, &d, &["1", "body", "0", "body", "next_track_id"]), Value::UInt(2));

        // hdlr's name is padded text: the trailing NUL is not part of the value.
        let name = walk(&mut ev, &d, &["1", "body", "1", "body", "name"]);
        let node = ev.node(&d, &name).unwrap();
        assert_eq!(node.value, Value::Str("VideoHandler".into()));
        assert_eq!(node.size_bits, 13 * 8);
        assert!(node.editable);
        let w = ev.prepare_write(&d, &name, "Cam").unwrap();
        assert_eq!(w.data, b"Cam\0\0\0\0\0\0\0\0\0\0".to_vec());

        // avcC, down to the bits of the SPS NAL header.
        assert_eq!(ev.node(&d, &[1, 2, 2, 1]).unwrap().value, Value::Str("avcC".into()));
        let profile = value(&mut ev, &d, &["1", "body", "2", "body", "profile"]);
        assert_eq!(profile, Value::Enum { raw: 66, name: Some("baseline".into()), hex: false });
        assert_eq!(value(&mut ev, &d, &["1", "body", "2", "body", "length_size_minus_one"]), Value::UInt(3));
        assert_eq!(value(&mut ev, &d, &["1", "body", "2", "body", "sps_count"]), Value::UInt(1));
        let nal_type = value(&mut ev, &d, &["1", "body", "2", "body", "sps", "0", "nal", "unit_type"]);
        assert_eq!(nal_type, Value::Enum { raw: 7, name: Some("SPS".into()), hex: false });

        // mdat has nothing placing samples over it, so it reads as its bytes.
        assert_eq!(ev.node(&d, &[2, 2]).unwrap().size_bits, 16 * 8);
    }

    #[test]
    fn a_box_of_size_zero_runs_to_the_end_of_the_file() {
        let mut b = boxed(b"ftyp", b"isom\0\0\0\0");
        // A size of 0 means the box takes everything that is left.
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(b"mdat");
        b.extend_from_slice(&[0x11; 20]);
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(mp4());
        assert_eq!(ev.node(&d, &[]).unwrap().child_count, 2);
        let mdat = ev.node(&d, &[1]).unwrap();
        assert_eq!(mdat.size_bits, (8 + 20) * 8);
        assert_eq!(ev.node(&d, &[1, 2]).unwrap().size_bits, 20 * 8);
    }

    #[test]
    fn quicktime_handler_names_are_pascal_strings() {
        let mut hdlr = b"mhlr".to_vec();
        hdlr.extend_from_slice(b"vide");
        hdlr.extend_from_slice(b"appl");
        hdlr.extend_from_slice(&[0; 8]);
        hdlr.push(12);
        hdlr.extend_from_slice(b"VideoHandler");
        let d = Document::new(MemSource(full(b"hdlr", 0, 0, &hdlr)));
        let mut ev = Evaluator::new(mp4());
        assert_eq!(value(&mut ev, &d, &["0", "body", "component_type"]), Value::Str("mhlr".into()));
        assert_eq!(value(&mut ev, &d, &["0", "body", "component_manufacturer"]), Value::Str("appl".into()));
        assert_eq!(value(&mut ev, &d, &["0", "body", "name", "text"]), Value::Str("VideoHandler".into()));
    }

    #[test]
    fn edit_lists_read_both_versions_and_the_empty_edit() {
        let mut v0 = 1u32.to_be_bytes().to_vec();
        v0.extend_from_slice(&1000u32.to_be_bytes());
        v0.extend_from_slice(&(-1i32).to_be_bytes());
        v0.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        let mut v1 = 1u32.to_be_bytes().to_vec();
        v1.extend_from_slice(&5_000_000_000u64.to_be_bytes());
        v1.extend_from_slice(&2048i64.to_be_bytes());
        v1.extend_from_slice(&0x0000_8000u32.to_be_bytes());
        let file = cat(&[&full(b"elst", 0, 0, &v0), &full(b"elst", 1, 0, &v1)]);
        let d = Document::new(MemSource(file));
        let mut ev = Evaluator::new(mp4());
        assert_eq!(value(&mut ev, &d, &["0", "body", "entries", "0", "segment_duration"]), Value::UInt(1000));
        let empty = value(&mut ev, &d, &["0", "body", "entries", "0", "media_time"]);
        assert_eq!(empty, Value::Enum { raw: -1, name: Some("empty edit".into()), hex: false });
        assert_eq!(value(&mut ev, &d, &["1", "body", "entries", "0", "segment_duration"]), Value::UInt(5_000_000_000));
        assert_eq!(value(&mut ev, &d, &["1", "body", "entries", "0", "media_rate"]), Value::Float(0.5));
    }

    #[test]
    fn data_references_hold_url_boxes() {
        let url = full(b"url ", 0, 1, b"");
        let mut body = 1u32.to_be_bytes().to_vec();
        body.extend_from_slice(&url);
        let d = Document::new(MemSource(full(b"dref", 0, 0, &body)));
        let mut ev = Evaluator::new(mp4());
        assert_eq!(value(&mut ev, &d, &["0", "body", "entries", "0", "type"]), Value::Str("url ".into()));
        let flags = walk(&mut ev, &d, &["0", "body", "entries", "0", "body", "flags"]);
        assert_eq!(ev.node(&d, &flags).unwrap().value.as_int(), Some(1));
    }

    #[test]
    fn itunes_items_and_quicktime_text_read_as_text() {
        let mut data = 1u32.to_be_bytes().to_vec(); // UTF-8
        data.extend_from_slice(&0u32.to_be_bytes());
        data.extend_from_slice(b"Sine 440");
        let item = boxed(b"\xa9nam", &boxed(b"data", &data));
        let mut trkn = 0u32.to_be_bytes().to_vec(); // implicit
        trkn.extend_from_slice(&0u32.to_be_bytes());
        trkn.extend_from_slice(&[0, 0, 0, 3, 0, 12, 0, 0]);
        let track = boxed(b"trkn", &boxed(b"data", &trkn));
        let ilst = boxed(b"ilst", &cat(&[&item, &track]));
        let mut xyz = 18u16.to_be_bytes().to_vec();
        xyz.extend_from_slice(&0x15c7u16.to_be_bytes());
        xyz.extend_from_slice(b"+37.7749-122.4194/");
        let udta = boxed(b"udta", &boxed(b"\xa9xyz", &xyz));
        let d = Document::new(MemSource(cat(&[&ilst, &udta])));
        let mut ev = Evaluator::new(mp4());
        assert_eq!(value(&mut ev, &d, &["0", "body", "0", "key"]), Value::Str("\u{a9}nam".into()));
        assert_eq!(value(&mut ev, &d, &["0", "body", "0", "values", "0", "body", "value"]), Value::Str("Sine 440".into()));
        assert_eq!(value(&mut ev, &d, &["0", "body", "1", "values", "0", "body", "value", "track"]), Value::UInt(3));
        assert_eq!(value(&mut ev, &d, &["0", "body", "1", "values", "0", "body", "value", "track_count"]), Value::UInt(12));
        assert_eq!(value(&mut ev, &d, &["1", "body", "0", "body", "0", "text"]), Value::Str("+37.7749-122.4194/".into()));
    }

    /// A Nero chapter list as ffmpeg writes it: version 1, four reserved
    /// bytes, a one-byte count, then each chapter's start and title.
    #[test]
    fn nero_chapters_read_start_and_title() {
        let mut body = 0u32.to_be_bytes().to_vec(); // reserved
        body.push(2);
        body.extend_from_slice(&0u64.to_be_bytes());
        body.push(9);
        body.extend_from_slice(b"Emergence");
        body.extend_from_slice(&15_000_000u64.to_be_bytes());
        body.push(12);
        body.extend_from_slice(b"Feeding buzz");
        let udta = boxed(b"udta", &full(b"chpl", 1, 0, &body));
        let d = Document::new(MemSource(udta));
        let mut ev = Evaluator::new(mp4());
        let chpl = ["0", "body", "0", "body"];
        assert_eq!(value(&mut ev, &d, &[&chpl[..], &["chapter_count"]].concat()), Value::UInt(2));
        let second = [&chpl[..], &["chapters", "1"]].concat();
        assert_eq!(value(&mut ev, &d, &[&second[..], &["title"]].concat()), Value::Str("Feeding buzz".into()));
        assert_eq!(value(&mut ev, &d, &[&second[..], &["start"]].concat()), Value::UInt(15_000_000));
        assert_eq!(value(&mut ev, &d, &[&second[..], &["start_seconds"]].concat()), Value::Float(1.5));

        // Version 0 has no reserved bytes.
        let d = Document::new(MemSource(boxed(b"udta", &full(b"chpl", 0, 0, &body[4..]))));
        let mut ev = Evaluator::new(mp4());
        assert_eq!(value(&mut ev, &d, &[&chpl[..], &["chapter_count"]].concat()), Value::UInt(2));
        assert_eq!(value(&mut ev, &d, &[&chpl[..], &["chapters", "0", "title"]].concat()), Value::Str("Emergence".into()));
    }

    #[test]
    fn braw_media_exposes_frame_metadata_and_picture_header() {
        let mut metadata_atom = 12u32.to_be_bytes().to_vec();
        metadata_atom.extend_from_slice(b"expo");
        metadata_atom.extend_from_slice(&1.5f32.to_be_bytes());

        let mut media = 20u32.to_be_bytes().to_vec();
        media.extend_from_slice(b"bmdf");
        media.extend_from_slice(&metadata_atom);
        media.extend_from_slice(b"braw");
        media.extend_from_slice(&29u32.to_be_bytes());
        media.extend_from_slice(&[0x11; 12]);
        media.extend_from_slice(&4096u16.to_be_bytes());
        media.extend_from_slice(&2160u16.to_be_bytes());
        media.extend_from_slice(&270u16.to_be_bytes());
        media.extend_from_slice(&[0xaa, 0xbb, 0xcc]);

        let mut file = boxed(b"wide", b"");
        file.extend_from_slice(&boxed(b"mdat", &media));
        let d = Document::new(MemSource(file));
        let mut ev = Evaluator::new(template_named("braw"));

        assert_eq!(ev.node(&d, &[1, 2, 0]).unwrap().value, Value::UInt(20));
        assert_eq!(ev.node(&d, &[1, 2, 2, 0, 1]).unwrap().value, Value::Str("expo".into()));
        assert_eq!(ev.node(&d, &[1, 2, 5, 1]).unwrap().value, Value::UInt(4096));
        assert_eq!(ev.node(&d, &[1, 2, 5, 2]).unwrap().value, Value::UInt(2160));
        assert_eq!(ev.node(&d, &[1, 2, 5, 4]).unwrap().size_bits, 24);
    }

    #[test]
    fn quicktime_metadata_keys_and_typed_values_are_visible() {
        let mut keys = vec![0; 4];
        keys.extend_from_slice(&1u32.to_be_bytes());
        keys.extend_from_slice(&26u32.to_be_bytes());
        keys.extend_from_slice(b"mdta");
        keys.extend_from_slice(b"com.blackmagic.iso");

        let mut data = 20u32.to_be_bytes().to_vec();
        data.extend_from_slice(b"data");
        data.extend_from_slice(&77u32.to_be_bytes());
        data.extend_from_slice(&0u32.to_be_bytes());
        data.extend_from_slice(&800u32.to_be_bytes());
        let mut item = 28u32.to_be_bytes().to_vec();
        item.extend_from_slice(&1u32.to_be_bytes());
        item.extend_from_slice(&data);

        let mut file = boxed(b"keys", &keys);
        file.extend_from_slice(&boxed(b"ilst", &item));
        let d = Document::new(MemSource(file));
        let mut ev = Evaluator::new(template_named("braw"));

        assert_eq!(ev.node(&d, &[0, 2, 3, 0, 2]).unwrap().value, Value::Str("com.blackmagic.iso".into()));
        assert_eq!(value(&mut ev, &d, &["1", "body", "0", "key"]), Value::UInt(1));
        assert_eq!(value(&mut ev, &d, &["1", "body", "0", "values", "0", "body", "value"]), Value::UInt(800));
    }

    #[test]
    fn blackmagic_motion_samples_decode_three_axes() {
        let mut sample = 20u32.to_be_bytes().to_vec();
        sample.extend_from_slice(b"mogy");
        sample.extend_from_slice(&1.0f32.to_le_bytes());
        sample.extend_from_slice(&2.0f32.to_le_bytes());
        sample.extend_from_slice(&3.0f32.to_le_bytes());
        let d = Document::new(MemSource(boxed(b"mdat", &sample)));
        let mut ev = Evaluator::new(template_named("braw"));

        assert_eq!(ev.node(&d, &[0, 2, 0, 1]).unwrap().value, Value::Str("mogy".into()));
        assert_eq!(ev.node(&d, &[0, 2, 0, 2]).unwrap().child_count, 3);
    }
}
