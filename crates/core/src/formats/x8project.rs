//! The Tascam Portacapture X8's project file: the 268-byte `.bin` file the
//! recorder saves beside a take's WAV files, such as `260430_0070.bin` beside
//! `260430_0070_3-4.wav` and `260430_0070_MIX.wav`.
//!
//! The file holds the take's length, the recording app, the DUAL REC setting
//! and the mixer settings the take was recorded with: MS decode, a level and
//! a pan for each of the six tracks, the master level and the SOLO switch.
//! In POCKET STUDIO it also holds each track's 4-band EQ and the metronome's
//! settings. It does not hold an input's source, gain or phantom power. Every
//! field is little-endian.
//!
//! With DUAL REC on, the recorder makes a second project for the MP3 or
//! 32-bit float copy and writes the same 268 bytes for it, named with an `M`
//! or an `F` after the take's name: `TASCAM_0001M.bin` or `TASCAM_0001F.bin`
//! beside `TASCAM_0001.bin`.
//!
//! The layout was read from the recorder's V2.03 firmware, which writes the
//! file at the end of a take and reads it when the take is opened, and it was
//! compared with 127 files an X8 recorded. Tascam does not document the
//! file. A field the firmware stores but never uses for anything that was
//! found is named `unknown` with its offset, and its note says what the
//! firmware does with it and what values the 127 files have. The six 32-byte
//! blocks at 0x40 (the EQ) and the 12 bytes at 0x100 (the metronome) were
//! read from the firmware's code and tables. Every one of the 127 files has
//! the default values there, so no file has confirmed a value other than a
//! default.
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

/// The recording apps, by the number the firmware's C code gives each one.
/// The firmware writes 9 for POCKET STUDIO and also for the tuner and the
/// metronome opened inside it (10 and 11). TUNER (7) and METRONOME (8) do not
/// record.
const APPS: &[(i128, &str)] =
    &[(1, "ASMR"), (2, "MANUAL"), (3, "MUSIC"), (4, "VOICE"), (5, "FIELD"), (6, "PODCAST"), (9, "POCKET STUDIO")];

/// The DUAL REC setting's values. The firmware's code also tests for 0 and
/// 2, but no setting in V2.03 writes either.
const DUAL_REC: &[(i128, &str)] = &[(1, "ON"), (3, "OFF")];

/// The MS DECODE setting's values, named as in the reference manual.
const MS_DECODE: &[(i128, &str)] = &[(0, "OFF"), (1, "TRACK 1-2"), (2, "TRACK 3-4"), (3, "TRACK 5-6")];

/// Off and on, for the switches.
const OFF_ON: &[(i128, &str)] = &[(0, "OFF"), (1, "ON")];

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
                ("dual_rec", T::enumeration("X8DualRec", T::u8(), DUAL_REC)),
                ("unknown_0a", T::u8()),
                ("unknown_0b", T::u8()),
                ("ms_decode", T::enumeration("X8MsDecode", T::u8(), MS_DECODE)),
                ("ms_decode_width", T::u8()),
                ("unknown_0e", T::u8()),
                ("unknown_0f", T::u8()),
                ("channels", T::array(channel(), E::lit(TRACKS as i128))),
                ("unused_channels", T::array(channel(), E::lit(2))),
                ("master_level", T::u8()),
                ("unknown_31", T::u8()),
                ("solo", T::enumeration("X8Solo", T::u8(), OFF_ON)),
                ("unknown_33", T::u8()),
                ("unused", T::bytes(E::lit(3))),
                ("unknown_37", T::u8()),
                ("unknown_38", T::u32(Little)),
                ("unknown_3c", T::u32(Little)),
                ("eq", T::array(track_eq(), E::lit(TRACKS as i128))),
                ("metronome", metronome()),
            ],
        )
        .field_valid("version", Valid::AnyOf(vec![E::lit(0), E::lit(1)]))
        .field_valid("ms_decode", Valid::InEnum)
        .field_valid("ms_decode_width", Valid::Max(E::lit(MAX_PERCENT)))
        .field_valid("unknown_0e", Valid::Max(E::lit(MAX_PERCENT)))
        .field_valid("master_level", Valid::Max(E::lit(MAX_LEVEL)))
        .field_valid("solo", Valid::InEnum)
        .field_doc(
            "version",
            "The version of the file's layout. The X8's V2.03 firmware writes 1. A file with 0 here is in an older layout. When the firmware reads a file with 0 here that is at least 0x34 bytes long, it fills everything from 0x40 on with default values and sets the version to 1.",
        )
        .field_doc("length_seconds", "The length of the take in whole seconds: the number of audio frames divided by the sample rate, with the fraction dropped.")
        .field_doc(
            "recording_app",
            "The recording app the take was made with. The values for MANUAL, MUSIC, FIELD and POCKET STUDIO were read from the firmware's code. The values for ASMR, VOICE and PODCAST were matched by elimination and from how the firmware treats each app. The firmware also writes 9 (POCKET STUDIO) when the tuner or the metronome inside POCKET STUDIO is open. Of the 127 files compared, 126 have 2 (MANUAL). One has 5 (FIELD), and that take's only file is a stereo master.",
        )
        .field_doc(
            "dual_rec",
            "The DUAL REC setting (GENERAL SETTINGS > REC SETTINGS): 3 is OFF and 1 is ON. With DUAL REC on, the recorder also writes the second copy's project file, whose file name has M (MP3) or F (32-bit float) after the take's name. All 127 files compared have 3.",
        )
        .field_doc(
            "unknown_0a",
            "A setting from 0 to 8, set by the firmware's internal settings message 0xFFD. The firmware's audio code reads it as bits: bit 0 when the app is VOICE, bit 1 when it is MUSIC and bit 2 when it is PODCAST. Each of those bits changes how channels 3 and 4 are routed in the mixer. The screen setting it belongs to was not found. All 127 files compared have 0.",
        )
        .field_doc("unknown_0b", "No code was found in V2.03 that reads or writes this byte on its own. The firmware copies it together with the neighbouring bytes. All 127 files compared have 0.")
        .field_doc("ms_decode", "The MS DECODE setting: OFF, or the pair of tracks whose mid-side signal the mixer decodes to left and right.")
        .field_doc(
            "ms_decode_width",
            "The MS decode width, from 0 to 100. The default is 50. The firmware uses this value as an index into its table of mid gains, and 100 minus this value as an index into its table of side gains.",
        )
        .field_doc(
            "unknown_0e",
            "A mixer setting from 0 to 100, set by the firmware's internal settings message 0xC06, which follows 0xC05 (the MS decode width). The default is 100. The firmware stores the value and can report it, but no audio code reads it, so in V2.03 the value has no effect on the sound. All 127 files compared have 100.",
        )
        .field_doc("unknown_0f", "No code was found in V2.03 that reads or writes this byte on its own. The firmware copies it together with the neighbouring bytes. All 127 files compared have 0.")
        .field_doc("channels", "The mixer channels of tracks 1 to 6, in track order, 4 bytes each.")
        .field_doc(
            "unused_channels",
            "Two more entries laid out like a mixer channel. The firmware's defaults set both to level 97, the first with pan 0 and the second with pan 40, and no code was found that changes them afterwards. The audio code's mixer reads only the six track channels. All 127 files compared have the defaults.",
        )
        .field_doc(
            "master_level",
            "The mixer's master level, from 0 to 127. The default is 97. Like a channel's level, the value is an index into the firmware's gain table. The 127 files compared have 97, 100 or 113.",
        )
        .field_doc("unknown_31", "The firmware's defaults set this byte to 70. No other code was found that reads or writes it on its own. All 127 files compared have 70.")
        .field_doc(
            "solo",
            "The SOLO switch, which solos one track for monitoring (AFL); added in firmware V2.00 for MANUAL and PODCAST. The firmware passes this switch and the soloed track to the audio code. On two screens, probably INPUT SETTINGS and MIXER SETTINGS, the audio code solos the chosen track only while this switch is 1; on other screens it solos a chosen track whatever the switch is. The file does not record which track was soloed. Of the 127 files compared, 86 have 0 and 41 have 1. The 41 are consecutive takes from 4 to 18 February 2026, so the switch stays as it was left from one take to the next.",
        )
        .field_doc("unknown_33", "No code was found in V2.03 that reads or writes this byte on its own. The firmware copies it together with the neighbouring bytes. All 127 files compared have 0.")
        .field_doc("unused", "Three bytes the firmware does not write. All 127 files compared have zeros.")
        .field_doc(
            "unknown_37",
            "In POCKET STUDIO the firmware copies this byte from a value its file writer keeps during the take. In other apps it keeps the value from the project that was open, or writes 0. All 127 files compared have 0.",
        )
        .field_doc(
            "unknown_38",
            "In POCKET STUDIO the firmware copies this 32-bit value from the first 32-bit word of the firmware's table entry for the take's first open file. In other apps it keeps the value from the project that was open, or writes 0. All 127 files compared have 0.",
        )
        .field_doc(
            "unknown_3c",
            "In POCKET STUDIO the firmware copies this 32-bit value from the second 32-bit word of the firmware's table entry for the take's first open file. In other apps it keeps the value from the project that was open, or writes 0. All 127 files compared have 0.",
        )
        .field_doc(
            "eq",
            "The 4-band EQ of each of tracks 1 to 6, in track order, 32 bytes each. The firmware saves these from the recorder's settings only in POCKET STUDIO; a take from another app has the default values, or the values of the project that was open. All 127 files compared have the default values.",
        )
        .field_doc(
            "metronome",
            "The metronome's settings, 12 bytes. They are the settings on POCKET STUDIO's METRONOME settings screen. The firmware saves them from the recorder's settings only in POCKET STUDIO; a take from another app has the default values. All 127 files compared have the default values.",
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
            "A setting whose range in the firmware's mixer range table is 0 to 100. When two tracks are stereo-linked, the firmware copies this byte and the level from the second track of the pair to the first. The audio code's mixer does not read it. All 127 files compared have 0.",
        )
        .field_doc("out_of_mix", "1 if the track is left out of the mix, and 0 if the track is in the mix. The firmware uses this flag only in PODCAST.")
}

/// The EQ's gain steps: 0 is -12 dB, 12 is 0 dB and 24 is 12 dB.
const MAX_EQ_GAIN: u8 = 24;
/// The EQ's Q steps.
const EQ_Q: &[(i128, &str)] = &[(0, "0.25"), (1, "0.5"), (2, "1"), (3, "2"), (4, "4"), (5, "8"), (6, "16")];

/// One track's EQ: four bands, then 16 bytes the firmware leaves at 0.
fn track_eq() -> T {
    T::structure("X8TrackEq", vec![("bands", T::array(eq_band(), E::lit(4))), ("unused", T::bytes(E::lit(16)))])
        .field_doc(
            "bands",
            "Bands 1 to 4 (LOW, L-MID, H-MID and HIGH), 4 bytes each. The defaults are 400 Hz, 1,700 Hz, 1,700 Hz and 5,500 Hz, each with 0 dB of gain and a Q of 2.",
        )
        .field_doc("unused", "Sixteen bytes that are zero in the default values and in all 127 files compared.")
}

/// One EQ band.
fn eq_band() -> T {
    T::structure(
        "X8EqBand",
        vec![("frequency", T::u8()), ("gain", T::u8()), ("q", T::enumeration("X8EqQ", T::u8(), EQ_Q)), ("padding", T::u8())],
    )
    .field_valid("gain", Valid::Max(E::lit(MAX_EQ_GAIN)))
    .field_doc(
        "frequency",
        "The band's frequency, as an index into the firmware's table of 64 frequencies from 32 Hz to 18,000 Hz. Bands 1 to 3 count from the table's start, so 16 is 400 Hz and 32 is 1,700 Hz. Band 4 counts from the table's 33rd entry, so 16 is 5,500 Hz. The default is 16 in bands 1 and 4, and 32 in bands 2 and 3.",
    )
    .field_doc("gain", "The band's gain in steps of 1 dB: 0 is -12 dB, 12 is 0 dB and 24 is 12 dB. The default is 12.")
    .field_doc("q", "The band's Q: 0.25, 0.5, 1, 2, 4, 8 or 16. The default is 2.")
    .field_doc("padding", "A byte that is 0 in the default values and in all 127 files compared.")
}

/// The metronome's click sounds.
const METRONOME_SOUND: &[(i128, &str)] = &[(0, "CLICK"), (1, "STICK"), (2, "BELL")];
/// When the metronome's click is heard.
const METRONOME_CLICK: &[(i128, &str)] = &[(0, "OFF"), (1, "REC"), (2, "REC&PLAY")];

/// The time signatures, 1/1 to 12/8: twelve beat counts for each of the note
/// values 1, 2, 4 and 8, so 4/4 is 27.
fn signatures() -> Vec<(i128, String)> {
    [1, 2, 4, 8]
        .iter()
        .enumerate()
        .flat_map(|(at, note)| (1..=12).map(move |beats| ((at * 12 + beats - 1) as i128, format!("{beats}/{note}"))))
        .collect()
}

/// The metronome's settings, at 0x100.
fn metronome() -> T {
    let signatures = signatures();
    let signatures: Vec<(i128, &str)> = signatures.iter().map(|(v, n)| (*v, n.as_str())).collect();
    T::structure(
        "X8Metronome",
        vec![
            ("tempo", T::u32(Little)),
            ("beat", T::u16(Little)),
            ("level", T::u16(Little)),
            ("sound", T::enumeration("X8MetronomeSound", T::u8(), METRONOME_SOUND)),
            ("bar_display", T::enumeration("X8BarDisplay", T::UInt { bits: 1, endian: Little }, OFF_ON)),
            ("accent", T::enumeration("X8Accent", T::UInt { bits: 1, endian: Little }, OFF_ON)),
            ("click", T::enumeration("X8MetronomeClick", T::UInt { bits: 2, endian: Little }, METRONOME_CLICK)),
            ("unused_bits", T::UInt { bits: 4, endian: Little }),
            ("signature", T::enumeration("X8Signature", T::u8(), &signatures)),
            ("unused", T::u8()),
        ],
    )
    .field_valid("sound", Valid::InEnum)
    .field_valid("click", Valid::InEnum)
    .field_valid("signature", Valid::InEnum)
    .field_doc("tempo", "The tempo in beats per minute, from 20 to 250. The default is 120. The firmware uses only the low 16 bits of this 32-bit value.")
    .field_doc("beat", "The number of beats in a bar, from 1 to 9. The default is 4.")
    .field_doc("level", "The metronome's level, from 0 to 100. The default is 50.")
    .field_doc("sound", "The click sound: CLICK, STICK or BELL. The default is CLICK.")
    .field_doc("bar_display", "1 if the project status bar counts in bars and beats instead of hours, minutes and seconds. The default is 0. This is the one switch in the manual's metronome settings left after ACCENT and CLICK were matched; the firmware only stores it and reports it to the screens.")
    .field_doc("accent", "1 if the metronome plays a different sound on the first beat of each bar. The default is 1.")
    .field_doc("click", "When the click is heard: OFF, REC (while recording) or REC&PLAY (while recording and playing). The default is OFF.")
    .field_doc("unused_bits", "The top 4 bits of the byte that holds bar_display, accent and click. The firmware leaves them at 0.")
    .field_doc(
        "signature",
        "The time signature, as an index into the firmware's list of 48 signatures: 1/1 to 12/1, then 1/2 to 12/2, 1/4 to 12/4 and 1/8 to 12/8. The default is 27, which is 4/4. When the number of beats is set to n, the firmware sets the signature to n/4. The order of the list was worked out from the default, that n/4 rule and the count of 48. The list's text was not found in the firmware.",
    )
    .field_doc("unused", "A byte that is 0 in the default values.")
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
        let blocks = ev.child_named(d, &[], "eq").unwrap().unwrap();
        let last = [blocks.clone(), vec![5]].concat();
        assert_eq!(ev.node(d, &last).unwrap().offset_bits, 0xe0 * 8);
        assert_eq!(ev.node(d, &last).unwrap().size_bits, 0x20 * 8);
        let second_entry = [last.clone(), vec![0, 1]].concat();
        assert_eq!(int(ev, d, &second_entry, &["frequency"]), 32);
        let tail = ev.child_named(d, &[], "metronome").unwrap().unwrap();
        assert_eq!(ev.node(d, &tail).unwrap().offset_bits, 0x100 * 8);
        assert_eq!(int(ev, d, &tail, &["tempo"]), 120);
        assert_eq!(int(ev, d, &tail, &["signature"]), 27);
    }

    fn named_value(bytes: Vec<u8>, path: &[&str]) -> Value {
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(x8project());
        let mut p = vec![];
        for name in path {
            p = ev.child_named(&d, &p, name).unwrap().unwrap_or_else(|| panic!("no {name}"));
        }
        ev.node(&d, &p).unwrap().value
    }

    fn enum_value(raw: i128, name: &str) -> Value {
        Value::Enum { raw, name: Some(name.into()), hex: false }
    }

    #[test]
    fn the_settings_with_names_show_them() {
        let mut bytes = default_project();
        bytes[0x0c] = 3;
        bytes[0x32] = 1;
        assert_eq!(named_value(bytes.clone(), &["recording_app"]), enum_value(2, "MANUAL"));
        assert_eq!(named_value(bytes.clone(), &["dual_rec"]), enum_value(3, "OFF"));
        assert_eq!(named_value(bytes.clone(), &["ms_decode"]), enum_value(3, "TRACK 5-6"));
        assert_eq!(named_value(bytes, &["solo"]), enum_value(1, "ON"));
    }

    /// The default metronome flags byte, 2, is ACCENT on with the bar display
    /// and the click off. Bit 0 is the first field.
    #[test]
    fn the_metronome_reads_by_name_and_bit() {
        let tail = |field: &str| named_value(default_project(), &["metronome", field]);
        assert_eq!(tail("sound"), enum_value(0, "CLICK"));
        assert_eq!(tail("bar_display"), enum_value(0, "OFF"));
        assert_eq!(tail("accent"), enum_value(1, "ON"));
        assert_eq!(tail("click"), enum_value(0, "OFF"));
        assert_eq!(tail("signature"), enum_value(27, "4/4"));

        let mut bytes = default_project();
        bytes[0x108] = 2;
        bytes[0x109] = 0b1001;
        bytes[0x10a] = 47;
        let tail = |field: &str| named_value(bytes.clone(), &["metronome", field]);
        assert_eq!(tail("sound"), enum_value(2, "BELL"));
        assert_eq!(tail("bar_display"), enum_value(1, "ON"));
        assert_eq!(tail("accent"), enum_value(0, "OFF"));
        assert_eq!(tail("click"), enum_value(2, "REC&PLAY"));
        assert_eq!(tail("signature"), enum_value(47, "12/8"));
    }

    #[test]
    fn each_time_signature_has_its_place_in_the_list() {
        let list = signatures();
        assert_eq!(list.len(), 48);
        assert_eq!(list[0], (0, "1/1".to_string()));
        // Setting the number of beats to n writes n + 23.
        for beats in 1..=9 {
            assert_eq!(list[beats + 23].1, format!("{beats}/4"));
        }
    }

    /// TUNER and METRONOME do not record, so a file never has 7 or 8. The
    /// number shows as it is, and it is not marked as a wrong value.
    #[test]
    fn an_app_with_no_name_shows_its_number() {
        let mut bytes = default_project();
        bytes[0x08] = 7;
        assert_eq!(named_value(bytes.clone(), &["recording_app"]), Value::Enum { raw: 7, name: None, hex: false });
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(x8project());
        let app = ev.child_named(&d, &[], "recording_app").unwrap().unwrap();
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
        // the two unused channels after the six track channels.
        let mut other = good.clone();
        other[0x08] = 5;
        other[0x29] = 255;
        assert!(is_project(&other, len));
    }
}
