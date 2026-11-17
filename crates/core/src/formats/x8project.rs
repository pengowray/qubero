//! The Tascam Portacapture X8's project file: the 268-byte `.bin` file the
//! recorder saves beside a take's WAV files, such as `260430_0070.bin` beside
//! `260430_0070_3-4.wav` and `260430_0070_MIX.wav`.
//!
//! The file holds the take's length and the mixer settings the take was
//! recorded with: MS decode, and a level and a pan for each of the six
//! tracks, and the master level. It does not hold an input's source, gain or
//! phantom power. Every field is little-endian.
//!
//! The layout was read from the recorder's V2.03 firmware, which writes the
//! file at the end of a take and reads it when the take is opened, and it was
//! compared with 117 files an X8 recorded. Tascam does not document the
//! file. Where the firmware's use of a field was not traced, the field is
//! named `unknown` with its offset, and its note gives the range the
//! firmware's mixer range table allows and the values in the 117 files. Two
//! parts are named by inference only, and their notes say so: the six
//! 32-byte blocks at 0x40 have the shape of a 4-band EQ, and the 12 bytes at
//! 0x100 may be reverb settings.
//!
//! Nothing in the file is a signature. [`is_project`] recognises a file by
//! its length of exactly 268 bytes and by every field with a known range
//! being inside that range.

use crate::template::{Endian::Little, Expr as E, Template, Ty as T, Valid};

/// The length of every project file, in bytes.
pub const FILE_BYTES: usize = 268;

/// Where the six mixer channels start, and how many bytes each one is.
const CHANNELS_AT: usize = 0x10;
const CHANNEL_BYTES: usize = 4;

/// The recorder has six tracks.
const TRACKS: usize = 6;

/// The highest level, pan, percentage and MS DECODE values the firmware's
/// mixer range table allows.
const MAX_LEVEL: u8 = 127;
const MAX_PAN: u8 = 40;
const MAX_PERCENT: u8 = 100;
const MAX_MS_DECODE: u8 = 3;

/// The recording apps whose number is confirmed. The X8 has other apps, and
/// one of the 117 files has 5 here, but only MANUAL's number was matched to
/// its name.
const APPS: &[(i128, &str)] = &[(2, "MANUAL")];

/// The MS DECODE setting, by the names the recorder's screen uses.
const MS_DECODE: &[(i128, &str)] = &[(0, "OFF"), (1, "INPUT 1-2"), (2, "INPUT 3-4"), (3, "INPUT 5-6")];

/// Whether a file is an X8 project file. The file has no signature, so every
/// one of these has to hold:
///
/// - the file is exactly 268 bytes long;
/// - the version is 0 or 1;
/// - MS DECODE is at most 3, and the MS decode width and the byte after it
///   are at most 100;
/// - each of the six mixer channels has a level of at most 127, a pan of at
///   most 40, a third byte of at most 100 and a flag of 0 or 1;
/// - the master level is at most 127 and the byte at 0x32 is 0 or 1;
/// - the three bytes at 0x34, which the firmware does not write, are zero;
/// - at least one of the six levels is above 0.
///
/// The last rule refuses a 268-byte file of zeros, which passes all the
/// others. A take saved with all six levels at 0 is refused with it, and can
/// still be read by choosing the template.
pub fn is_project(head: &[u8], len: u64) -> bool {
    if len != FILE_BYTES as u64 {
        return false;
    }
    let Some(h) = head.get(..FILE_BYTES) else { return false };
    let version = u32::from_le_bytes([h[0], h[1], h[2], h[3]]);
    let channels = h[CHANNELS_AT..CHANNELS_AT + TRACKS * CHANNEL_BYTES].chunks_exact(CHANNEL_BYTES);
    version <= 1
        && h[0x0c] <= MAX_MS_DECODE
        && h[0x0d] <= MAX_PERCENT
        && h[0x0e] <= MAX_PERCENT
        && channels.clone().all(|c| c[0] <= MAX_LEVEL && c[1] <= MAX_PAN && c[2] <= MAX_PERCENT && c[3] <= 1)
        && channels.clone().any(|c| c[0] > 0)
        && h[0x30] <= MAX_LEVEL
        && h[0x32] <= 1
        && h[0x34..0x37].iter().all(|&b| b == 0)
}

pub fn x8project() -> Template {
    Template::new(
        "x8project",
        T::structure(
            "X8Project",
            vec![
                ("version", T::u32(Little)),
                ("length_seconds", T::u32(Little)),
                ("recording_app", T::enumeration("X8RecordingApp", T::u8(), APPS)),
                ("file_mode", T::u8()),
                ("unknown_0a", T::u8()),
                ("unknown_0b", T::u8()),
                ("ms_decode", T::enumeration("X8MsDecode", T::u8(), MS_DECODE)),
                ("ms_decode_width", T::u8()),
                ("unknown_0e", T::u8()),
                ("unknown_0f", T::u8()),
                ("channels", T::array(channel(), E::lit(TRACKS as i128))),
                ("unknown_entries", T::array(unknown_entry(), E::lit(2))),
                ("master_level", T::u8()),
                ("unknown_31", T::u8()),
                ("unknown_32", T::u8()),
                ("unknown_33", T::u8()),
                ("unused", T::bytes(E::lit(3))),
                ("unknown_37", T::u8()),
                ("unknown_38", T::u32(Little)),
                ("unknown_3c", T::u32(Little)),
                ("track_blocks", T::array(track_block(), E::lit(TRACKS as i128))),
                ("unknown_100", block_at_100()),
            ],
        )
        .field_valid("version", Valid::AnyOf(vec![E::lit(0), E::lit(1)]))
        .field_valid("ms_decode", Valid::InEnum)
        .field_valid("ms_decode_width", Valid::Max(E::lit(MAX_PERCENT)))
        .field_valid("unknown_0e", Valid::Max(E::lit(MAX_PERCENT)))
        .field_valid("master_level", Valid::Max(E::lit(MAX_LEVEL)))
        .field_valid("unknown_32", Valid::Max(E::lit(1)))
        .field_doc(
            "version",
            "The version of the file's layout. The X8's V2.03 firmware writes 1. A file with 0 here is in an older layout. When the firmware reads a file with 0 here that is at least 0x34 bytes long, it fills everything from 0x40 on with default values and sets the version to 1.",
        )
        .field_doc("length_seconds", "The length of the take in whole seconds: the number of audio frames divided by the sample rate, with the fraction dropped.")
        .field_doc(
            "recording_app",
            "The number of the recording app the take was made with. 2 is MANUAL. The firmware writes 9 for apps 9, 10 and 11. The names of the apps numbered 1, 3, 4, 5 and 6 are not confirmed. Of the 117 files compared, 116 have 2 and 1 has 5.",
        )
        .field_doc("file_mode", "A file mode setting. Its values were not traced. All 117 files compared have 3.")
        .field_doc("unknown_0a", "Not traced. All 117 files compared have 0.")
        .field_doc("unknown_0b", "Not traced. All 117 files compared have 0.")
        .field_doc("ms_decode", "The MS DECODE setting: OFF, or the pair of inputs whose mid-side signal the mixer decodes to left and right.")
        .field_doc(
            "ms_decode_width",
            "The MS decode width, from 0 to 100. The default is 50. The firmware uses this value as an index into its table of mid gains, and 100 minus this value as an index into its table of side gains.",
        )
        .field_doc("unknown_0e", "Not traced. The firmware allows 0 to 100, and the default is 100. All 117 files compared have 100.")
        .field_doc("unknown_0f", "Not traced. All 117 files compared have 0.")
        .field_doc("channels", "The mixer channels of tracks 1 to 6, in track order, 4 bytes each.")
        .field_doc(
            "unknown_entries",
            "Two 4-byte entries after the six mixer channels. Their use was not traced. In all 117 files compared, the first entry is 97, 0, 0, 0 and the second is 97, 40, 0, 0. If the first two bytes are a level and a pan, as they are in a mixer channel, those values are the default level with pan fully left, and the default level with pan fully right.",
        )
        .field_doc(
            "master_level",
            "The mixer's master level, from 0 to 127. The 117 files compared have 97, 100 or 113. The value's unit was not traced; a channel's level, which has the same range, is an index into a gain table.",
        )
        .field_doc("unknown_31", "Not traced. All 117 files compared have 70.")
        .field_doc("unknown_32", "A setting that is 0 or 1. Its meaning was not traced. Of the 117 files compared, 78 have 0 and 39 have 1.")
        .field_doc("unknown_33", "Not traced. All 117 files compared have 0.")
        .field_doc("unused", "Three bytes the firmware does not write. All 117 files compared have zeros.")
        .field_doc(
            "unknown_37",
            "Not traced. The firmware copies this byte from the recorder's state, or keeps the value from the project that was open. All 117 files compared have 0.",
        )
        .field_doc(
            "unknown_38",
            "Not traced. The firmware copies this 32-bit value from the recorder's state, or keeps the value from the project that was open. All 117 files compared have 0.",
        )
        .field_doc(
            "unknown_3c",
            "Not traced. The firmware copies this 32-bit value from the recorder's state, or keeps the value from the project that was open. All 117 files compared have 0.",
        )
        .field_doc(
            "track_blocks",
            "One 32-byte block for each of tracks 1 to 6, in track order. A block's layout, four 4-byte entries and then 16 zero bytes, suggests the settings of a 4-band EQ, but the meaning of its bytes was not traced. The firmware saves these blocks from the recorder's settings only in apps 9, 10 and 11. All 117 files compared have the default values.",
        )
        .field_doc(
            "unknown_100",
            "Twelve bytes whose use was not traced. They may be reverb settings. The firmware saves them from the recorder's settings only in apps 9, 10 and 11. All 117 files compared have the default values.",
        ),
    )
}

/// One track's mixer channel.
fn channel() -> T {
    T::structure("X8MixerChannel", vec![("level", T::u8()), ("pan", T::u8()), ("unknown_2", T::u8()), ("out_of_mix", T::u8())])
        .field_valid("level", Valid::Max(E::lit(MAX_LEVEL)))
        .field_valid("pan", Valid::Max(E::lit(MAX_PAN)))
        .field_valid("unknown_2", Valid::Max(E::lit(MAX_PERCENT)))
        .field_valid("out_of_mix", Valid::Max(E::lit(1)))
        .field_doc("level", "The track's level in the mix, from 0 to 127. The default is 97. The value is an index into the firmware's gain table, not a number of decibels.")
        .field_doc("pan", "The track's pan in the mix, from 0 to 40: 0 is fully left, 20 is centre and 40 is fully right.")
        .field_doc(
            "unknown_2",
            "Not traced. The firmware allows 0 to 100. The byte's position in the mixer channel suggests a send level. All 117 files compared have 0.",
        )
        .field_doc("out_of_mix", "1 if the track is left out of the mix, and 0 if the track is in the mix. The firmware uses this flag only in app 6.")
}

/// One of the two entries after the six channels, which are the same size as
/// a channel and whose use was not traced.
fn unknown_entry() -> T {
    T::structure("X8UnknownEntry", vec![("unknown_0", T::u8()), ("unknown_1", T::u8()), ("unknown_2", T::u8()), ("unknown_3", T::u8())])
}

/// One track's 32-byte block at 0x40 and after.
fn track_block() -> T {
    T::structure("X8TrackBlock", vec![("bands", T::array(band(), E::lit(4))), ("unused", T::bytes(E::lit(16)))])
        .field_doc(
            "bands",
            "Four 4-byte entries. The default values of their first three bytes are 16, 12, 3 in the first entry, 32, 12, 3 in the second and third entries, and 16, 12, 3 in the fourth entry. The name bands is an inference from the block's layout, which suggests a 4-band EQ.",
        )
        .field_doc("unused", "Sixteen bytes that are zero in the default values and in all 117 files compared.")
}

/// One of a track block's four entries.
fn band() -> T {
    T::structure("X8TrackBand", vec![("unknown_0", T::u8()), ("unknown_1", T::u8()), ("unknown_2", T::u8()), ("padding", T::u8())])
        .field_doc("unknown_0", "Not traced. The default is 16 in a block's first and fourth entries and 32 in its second and third entries.")
        .field_doc("unknown_1", "Not traced. The default is 12.")
        .field_doc("unknown_2", "Not traced. The default is 3.")
        .field_doc("padding", "A byte that is 0 in the default values and in all 117 files compared.")
}

/// The 12 bytes at 0x100.
fn block_at_100() -> T {
    T::structure(
        "X8UnknownBlock",
        vec![
            ("unknown_0", T::u32(Little)),
            ("unknown_4", T::u16(Little)),
            ("unknown_6", T::u16(Little)),
            ("unknown_8", T::u8()),
            ("unknown_flags", T::u8()),
            ("unknown_a", T::u8()),
            ("unknown_b", T::u8()),
        ],
    )
    .field_doc("unknown_0", "Not traced. The default is 120.")
    .field_doc("unknown_4", "Not traced. The default is 4.")
    .field_doc("unknown_6", "Not traced. The default is 50.")
    .field_doc("unknown_8", "Not traced. The default is 0.")
    .field_doc("unknown_flags", "A flag byte. The meaning of its bits was not traced. The default is 2.")
    .field_doc("unknown_a", "Not traced. The default is 27.")
    .field_doc("unknown_b", "Not traced. The default is 0.")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// A project file as the firmware's defaults make it: a take of 157
    /// seconds recorded in MANUAL, every level at 97 and every pan at centre.
    pub(crate) fn default_project() -> Vec<u8> {
        let mut v = vec![0u8; FILE_BYTES];
        v[0..4].copy_from_slice(&1u32.to_le_bytes());
        v[4..8].copy_from_slice(&157u32.to_le_bytes());
        v[0x08] = 2;
        v[0x09] = 3;
        v[0x0d] = 50;
        v[0x0e] = 100;
        for entry in 0..8 {
            v[0x10 + 4 * entry] = 97;
            v[0x11 + 4 * entry] = 20;
        }
        v[0x30] = 97;
        v[0x31] = 70;
        for track in 0..TRACKS {
            let at = 0x40 + 0x20 * track;
            v[at..at + 16].copy_from_slice(&[16, 12, 3, 0, 32, 12, 3, 0, 32, 12, 3, 0, 16, 12, 3, 0]);
        }
        v[0x100..0x10c].copy_from_slice(&[120, 0, 0, 0, 4, 0, 50, 0, 0, 2, 27, 0]);
        v
    }

    fn int(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], names: &[&str]) -> i128 {
        let mut p = path.to_vec();
        for name in names {
            p = ev.child_named(d, &p, name).unwrap().unwrap_or_else(|| panic!("no {name} under {p:?}"));
        }
        ev.node(d, &p).unwrap().value.as_int().unwrap_or_else(|| panic!("{names:?} is not a number"))
    }

    #[test]
    fn the_default_project_reads_field_by_field() {
        let mut bytes = default_project();
        bytes[0x0c] = 2;
        bytes[0x10 + 4 * 2] = 64;
        bytes[0x11 + 4 * 2] = 40;
        assert!(is_project(&bytes, FILE_BYTES as u64));
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(x8project());
        let (ev, d) = (&mut ev, &d);
        // Every byte of the file is in a field.
        assert_eq!(ev.node(d, &[]).unwrap().size_bits, FILE_BYTES as u64 * 8);
        assert_eq!(int(ev, d, &[], &["version"]), 1);
        assert_eq!(int(ev, d, &[], &["length_seconds"]), 157);
        assert_eq!(int(ev, d, &[], &["recording_app"]), 2);
        assert_eq!(int(ev, d, &[], &["ms_decode"]), 2);
        assert_eq!(int(ev, d, &[], &["ms_decode_width"]), 50);
        assert_eq!(int(ev, d, &[], &["master_level"]), 97);
        let channels = ev.child_named(d, &[], "channels").unwrap().unwrap();
        assert_eq!(ev.node(d, &channels).unwrap().child_count, 6);
        let third = [channels.clone(), vec![2]].concat();
        assert_eq!(ev.node(d, &third).unwrap().offset_bits, 0x18 * 8);
        assert_eq!((int(ev, d, &third, &["level"]), int(ev, d, &third, &["pan"])), (64, 40));
        let blocks = ev.child_named(d, &[], "track_blocks").unwrap().unwrap();
        let last = [blocks.clone(), vec![5]].concat();
        assert_eq!(ev.node(d, &last).unwrap().offset_bits, 0xe0 * 8);
        assert_eq!(ev.node(d, &last).unwrap().size_bits, 0x20 * 8);
        let second_band = [last.clone(), vec![0, 1]].concat();
        assert_eq!(int(ev, d, &second_band, &["unknown_0"]), 32);
        let tail = ev.child_named(d, &[], "unknown_100").unwrap().unwrap();
        assert_eq!(ev.node(d, &tail).unwrap().offset_bits, 0x100 * 8);
        assert_eq!(int(ev, d, &tail, &["unknown_0"]), 120);
        assert_eq!(int(ev, d, &tail, &["unknown_a"]), 27);
    }

    #[test]
    fn the_settings_with_names_show_them() {
        let mut bytes = default_project();
        bytes[0x0c] = 3;
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(x8project());
        let app = ev.child_named(&d, &[], "recording_app").unwrap().unwrap();
        assert_eq!(ev.node(&d, &app).unwrap().value, Value::Enum { raw: 2, name: Some("MANUAL".into()), hex: false });
        let ms = ev.child_named(&d, &[], "ms_decode").unwrap().unwrap();
        assert_eq!(ev.node(&d, &ms).unwrap().value, Value::Enum { raw: 3, name: Some("INPUT 5-6".into()), hex: false });
    }

    /// App 5 is in one real file. Its name is not confirmed, so it shows as a
    /// number, and it is not marked as a wrong value.
    #[test]
    fn an_app_with_no_confirmed_name_shows_its_number() {
        let mut bytes = default_project();
        bytes[0x08] = 5;
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(x8project());
        let app = ev.child_named(&d, &[], "recording_app").unwrap().unwrap();
        assert_eq!(ev.node(&d, &app).unwrap().value, Value::Enum { raw: 5, name: None, hex: false });
        assert!(ev.valid_of(&d, &app).unwrap().is_none());
    }

    #[test]
    fn a_value_outside_its_range_fails_its_check() {
        let mut bytes = default_project();
        bytes[0x0c] = 4;
        bytes[0x11] = 41;
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(x8project());
        let ms = ev.child_named(&d, &[], "ms_decode").unwrap().unwrap();
        assert!(!ev.valid_of(&d, &ms).unwrap().expect("ms_decode has a constraint").ok);
        let channels = ev.child_named(&d, &[], "channels").unwrap().unwrap();
        let first = [channels, vec![0]].concat();
        let pan = ev.child_named(&d, &first, "pan").unwrap().unwrap();
        assert!(!ev.valid_of(&d, &pan).unwrap().expect("pan has a constraint").ok);
        let level = ev.child_named(&d, &first, "level").unwrap().unwrap();
        assert!(ev.valid_of(&d, &level).unwrap().expect("level has a constraint").ok);
    }

    #[test]
    fn a_file_that_breaks_one_rule_is_refused() {
        let good = default_project();
        let len = FILE_BYTES as u64;
        assert!(is_project(&good, len));
        // The older layout's version number is accepted too.
        let mut old = good.clone();
        old[0] = 0;
        assert!(is_project(&old, len));

        // One byte longer or shorter.
        let mut longer = good.clone();
        longer.push(0);
        assert!(!is_project(&longer, len + 1));
        assert!(!is_project(&good[..FILE_BYTES - 1], len - 1));
        // A head that stops before the end of the file.
        assert!(!is_project(&good[..0x40], len));
        // A 268-byte file of zeros.
        assert!(!is_project(&[0; FILE_BYTES], len));

        // One byte out of range at a time: the offset, and the lowest value
        // that is refused there.
        let refused: &[(usize, u8)] = &[
            (0x00, 2),
            (0x01, 1),
            (0x0c, 4),
            (0x0d, 101),
            (0x0e, 101),
            (0x10, 128),
            (0x11, 41),
            (0x12, 101),
            (0x13, 2),
            (0x24, 128),
            (0x25, 41),
            (0x26, 101),
            (0x27, 2),
            (0x30, 128),
            (0x32, 2),
            (0x34, 1),
            (0x35, 1),
            (0x36, 1),
        ];
        for &(at, value) in refused {
            let mut bad = good.clone();
            bad[at] = value;
            assert!(!is_project(&bad, len), "{value} at {at:#x} was accepted");
            bad[at] = value - 1;
            assert!(is_project(&bad, len), "{} at {at:#x} was refused", value - 1);
        }

        // The fields with no known range do not count: the app number, and
        // the two entries after the six channels.
        let mut other = good.clone();
        other[0x08] = 5;
        other[0x29] = 255;
        assert!(is_project(&other, len));
    }
}
