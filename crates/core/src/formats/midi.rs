//! Standard MIDI Files: a header chunk, then one track chunk per track.
//!
//! Chunks look like RIFF's but simpler: four ASCII characters, a big-endian
//! length, and a body with no padding. A track body is a run of events, each
//! one a delta time and then a message.
//!
//! Delta times are variable-length quantities, seven bits per byte with the
//! high bit meaning "another byte follows". That is `Ty::Vlq`, added for this
//! format: LEB128 packs the same seven bits in the opposite order.
//!
//! Running status is why an event has both a `status` field and an
//! `effective_status` one. A message may leave its status byte out and mean
//! "the same as last time", which most files written by a sequencer do, so it
//! is the common case rather than a corner one. A real status byte is 0x80 or
//! above, so the field exists only when the byte at its own start says so, and
//! is no bits wide when it does not. `effective_status` is then this event's
//! status or, when there is none, the one the event before it settled on.
//!
//! Per the spec a system message cancels running status, and this carries it
//! through one instead. That matches what lenient sequencers accept and only
//! misreads a file that is already invalid, where the alternative would be to
//! stop reading a valid one.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};

/// A four-character chunk id as the big-endian number a switch compares.
fn cc(s: &str) -> i128 {
    s.bytes().fold(0i128, |acc, b| (acc << 8) | b as i128)
}

/// What the tracks in the file are to each other.
const FILE_FORMAT: &[(i128, &str)] =
    &[(0, "one track"), (1, "tracks play together"), (2, "tracks play separately")];

/// The frame rate a SMPTE division names, as the low seven bits of the
/// negative frame count the file stores: 104 is -24, and so 24 frames a second.
const FRAME_RATE: &[(i128, &str)] =
    &[(98, "30 fps"), (99, "29.97 fps drop"), (103, "25 fps"), (104, "24 fps")];

/// Meta events: everything that is about the music rather than played.
const META_TYPE: &[(i128, &str)] = &[
    (0x00, "sequence number"),
    (0x01, "text"),
    (0x02, "copyright"),
    (0x03, "track name"),
    (0x04, "instrument name"),
    (0x05, "lyric"),
    (0x06, "marker"),
    (0x07, "cue point"),
    (0x08, "program name"),
    (0x09, "device name"),
    (0x20, "channel prefix"),
    (0x21, "port"),
    (0x2f, "end of track"),
    (0x51, "tempo"),
    (0x54, "smpte offset"),
    (0x58, "time signature"),
    (0x59, "key signature"),
    (0x7f, "sequencer specific"),
];

/// The seven kinds of channel message, by the high nibble of the status byte.
/// What follows each one is in `channel_message`.
const CHANNEL_MESSAGE: &[(u8, &str)] = &[
    (0x8, "note off"),
    (0x9, "note on"),
    (0xa, "aftertouch"),
    (0xb, "control change"),
    (0xc, "program change"),
    (0xd, "channel pressure"),
    (0xe, "pitch bend"),
];

pub fn midi() -> Template {
    Template::new("midi", T::repeat(T::Named("Chunk".into()), Until::End)).with_type("Chunk", chunk())
}

fn chunk() -> T {
    T::structure_named(
        "Chunk",
        "id",
        "body",
        vec![
            ("id", T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii)),
            ("size", T::u32(Big)),
            (
                "body",
                T::sized(
                    E::field("size"),
                    T::switch(
                        E::field("id"),
                        vec![(cc("MThd"), header()), (cc("MTrk"), T::repeat(event(), Until::End))],
                        T::bytes(E::Remaining),
                    ),
                ),
            ),
        ],
    )
}

fn header() -> T {
    T::structure(
        "Header",
        vec![
            ("format", T::enumeration("FileFormat", T::u16(Big), FILE_FORMAT)),
            ("track_count", T::u16(Big)),
            ("division", division()),
            // Room for fields no version has defined yet.
            ("extra", T::bytes(E::Remaining)),
        ],
    )
}

/// How long a tick is. The top bit of the two bytes picks between counting
/// ticks in a quarter note and counting them in a film or video frame, so it
/// is a field of its own and the other fifteen bits mean whatever it says.
fn division() -> T {
    let smpte = T::structure(
        "Smpte",
        vec![
            ("frame_rate", T::enumeration("FrameRate", T::UInt { bits: 7, endian: Big }, FRAME_RATE)),
            ("ticks_per_frame", T::u8()),
        ],
    );
    T::structure(
        "Division",
        vec![
            ("in_frames", T::UInt { bits: 1, endian: Big }),
            (
                "rate",
                T::switch(
                    E::field("in_frames"),
                    vec![(1, smpte)],
                    T::UInt { bits: 15, endian: Big },
                ),
            ),
        ],
    )
}

/// One event: how long to wait, what the message is, and the message itself.
fn event() -> T {
    // Every status byte gets a name, and a case saying what follows it.
    let mut names: Vec<(i128, String)> = vec![
        (0xf0, "sysex".to_string()),
        (0xf7, "sysex escape".to_string()),
        (0xff, "meta".to_string()),
    ];
    let mut cases: Vec<(i128, T)> = vec![
        (0xf0, sysex()),
        (0xf7, sysex()),
        (0xff, meta()),
    ];
    for (high, name) in CHANNEL_MESSAGE {
        for channel in 0..16u8 {
            let status = ((high << 4) | channel) as i128;
            names.push((status, format!("{name} ch{}", channel + 1)));
            cases.push((status, channel_message(*high)));
        }
    }
    let names: Vec<(i128, &str)> = names.iter().map(|(v, s)| (*v, s.as_str())).collect();

    // The status an event runs on is a field of no bits, so no linear view
    // would show it. Naming the event by it puts `note on ch1` on the row
    // whether or not this event is the one that spelled it out.
    T::structure_named(
        "Event",
        "effective_status",
        "message",
        vec![
            ("delta", T::vlq()),
            // A status byte has its top bit set. Where it does not, this event
            // left the byte out and the field is no bits wide.
            (
                "status",
                T::switch(
                    E::peek(8).div(E::lit(128)),
                    vec![(1, T::enumeration_hex("Status", T::u8(), &names))],
                    T::computed(E::lit(0)),
                ),
            ),
            // This event's status, or the one the event before it settled on.
            (
                "effective_status",
                T::enumeration_hex("Status", T::computed(E::field("status").or(E::prev("effective_status"))), &names),
            ),
            ("message", T::switch(E::field("effective_status"), cases, running_status())),
        ],
    )
}

fn channel_message(high: u8) -> T {
    let two = |a: &str, b: &str| T::structure("Message", vec![(a, T::u8()), (b, T::u8())]);
    match high {
        0x8 | 0x9 => two("note", "velocity"),
        0xa => two("note", "pressure"),
        0xb => two("controller", "value"),
        0xc => T::structure("Message", vec![("program", T::u8())]),
        0xd => T::structure("Message", vec![("pressure", T::u8())]),
        // Fourteen bits of bend, low seven first, centred on 8192.
        _ => two("bend_low", "bend_high"),
    }
}

fn sysex() -> T {
    T::structure(
        "SysEx",
        vec![("length", T::vlq()), ("data", T::sized(E::field("length"), T::bytes(E::Remaining)))],
    )
}

fn meta() -> T {
    // Text meta events differ only in what the text is for. MIDI never said
    // which encoding, and files hold all of them, so the bytes decide.
    let text = T::text(StrLen::Fixed(E::Remaining), Encoding::Unknown);
    let mut cases: Vec<(i128, T)> = (0x01..=0x09).map(|t| (t as i128, text.clone())).collect();
    cases.push((0x00, T::structure("SequenceNumber", vec![("number", T::u16(Big))])));
    cases.push((0x20, T::structure("ChannelPrefix", vec![("channel", T::u8())])));
    cases.push((0x21, T::structure("Port", vec![("port", T::u8())])));
    // Microseconds in a quarter note: 500000 is 120 beats a minute.
    cases.push((0x51, T::structure("Tempo", vec![("microseconds_per_quarter", T::UInt { bits: 24, endian: Big })])));
    cases.push((
        0x54,
        T::structure(
            "SmpteOffset",
            vec![
                ("hour", T::u8()),
                ("minute", T::u8()),
                ("second", T::u8()),
                ("frame", T::u8()),
                ("subframe", T::u8()),
            ],
        ),
    ));
    cases.push((
        0x58,
        T::structure(
            "TimeSignature",
            vec![
                ("numerator", T::u8()),
                // A power of two: 3 means an eighth note.
                ("denominator", T::u8()),
                ("clocks_per_click", T::u8()),
                ("32nds_per_quarter", T::u8()),
            ],
        ),
    ));
    cases.push((
        0x59,
        T::structure(
            "KeySignature",
            vec![
                // Sharps if positive, flats if negative.
                ("sharps", T::Int { bits: 8, endian: Big }),
                ("scale", T::enumeration("Scale", T::u8(), &[(0, "major"), (1, "minor")])),
            ],
        ),
    ));

    T::structure(
        "Meta",
        vec![
            ("type", T::enumeration_hex("MetaType", T::u8(), META_TYPE)),
            ("length", T::vlq()),
            ("value", T::sized(E::field("length"), T::switch(E::field("type"), cases, T::bytes(E::Remaining)))),
        ],
    )
}

/// A track that opens with a data byte has no status to run from, which no
/// valid file does. The rest of it is bytes, because guessing a status would
/// be inventing one.
fn running_status() -> T {
    T::structure("NoStatus", vec![("rest_of_track", T::bytes(E::Remaining))])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn chunk_bytes(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = id.to_vec();
        v.extend_from_slice(&(body.len() as u32).to_be_bytes());
        v.extend_from_slice(body);
        v
    }

    /// A delta time, as the file writes it.
    fn vlq_bytes(mut v: u32) -> Vec<u8> {
        let mut groups = vec![(v & 0x7f) as u8];
        v >>= 7;
        while v != 0 {
            groups.push((v & 0x7f) as u8);
            v >>= 7;
        }
        let mut out = Vec::new();
        while let Some(g) = groups.pop() {
            out.push(if groups.is_empty() { g } else { g | 0x80 });
        }
        out
    }

    fn file() -> Vec<u8> {
        let mut head = 1u16.to_be_bytes().to_vec(); // tracks play together
        head.extend_from_slice(&1u16.to_be_bytes());
        head.extend_from_slice(&480u16.to_be_bytes()); // ticks per quarter

        let mut track = Vec::new();
        // Track name.
        track.extend_from_slice(&vlq_bytes(0));
        track.extend_from_slice(&[0xff, 0x03, 5]);
        track.extend_from_slice(b"Piano");
        // Tempo: 120 beats a minute.
        track.extend_from_slice(&vlq_bytes(0));
        track.extend_from_slice(&[0xff, 0x51, 3, 0x07, 0xa1, 0x20]);
        // Note on, then off a quarter note later. Both carry their status.
        track.extend_from_slice(&vlq_bytes(0));
        track.extend_from_slice(&[0x90, 60, 100]);
        track.extend_from_slice(&vlq_bytes(480));
        track.extend_from_slice(&[0x80, 60, 64]);
        track.extend_from_slice(&vlq_bytes(0));
        track.extend_from_slice(&[0xff, 0x2f, 0]);

        let mut out = chunk_bytes(b"MThd", &head);
        out.extend_from_slice(&chunk_bytes(b"MTrk", &track));
        out
    }

    #[test]
    fn a_track_reads_as_events() {
        let d = Document::new(MemSource(file()));
        let mut ev = Evaluator::new(midi());
        assert_eq!(ev.node(&d, &[]).unwrap().child_count, 2);

        // The header, and a division counted in ticks per quarter note.
        assert_eq!(
            ev.node(&d, &[0, 2, 0]).unwrap().value,
            Value::Enum { raw: 1, name: Some("tracks play together".into()), hex: false }
        );
        assert_eq!(ev.node(&d, &[0, 2, 2, 0]).unwrap().value, Value::UInt(0));
        assert_eq!(ev.node(&d, &[0, 2, 2, 1]).unwrap().value, Value::UInt(480));

        let track = ev.node(&d, &[1, 2]).unwrap();
        assert_eq!(track.child_count, 5);

        // A meta event: the name of the track.
        assert_eq!(
            ev.node(&d, &[1, 2, 0, 3, 0]).unwrap().value,
            Value::Enum { raw: 3, name: Some("track name".into()), hex: true }
        );
        assert_eq!(ev.node(&d, &[1, 2, 0, 3, 2]).unwrap().value, Value::Str("Piano".into()));
        assert_eq!(ev.node(&d, &[1, 2, 1, 3, 2, 0]).unwrap().value, Value::UInt(500_000));

        // A note, and the wait before the one that ends it.
        assert_eq!(
            ev.node(&d, &[1, 2, 2, 1]).unwrap().value,
            Value::Enum { raw: 0x90, name: Some("note on ch1".into()), hex: true }
        );
        assert_eq!(ev.node(&d, &[1, 2, 2, 3, 0]).unwrap().value, Value::UInt(60));
        assert_eq!(ev.node(&d, &[1, 2, 2, 3, 1]).unwrap().value, Value::UInt(100));
        let off = ev.node(&d, &[1, 2, 3, 0]).unwrap();
        assert_eq!(off.value, Value::UInt(480));
        assert_eq!(off.size_bits, 16); // two bytes of seven bits each

        // A delta time can be edited, and keeps the bytes it already had.
        let w = ev.prepare_write(&d, &[1, 2, 3, 0], "3").unwrap();
        assert_eq!(w.data, vec![0x80, 0x03]);
        assert_eq!(w.n_bits, 16);
    }

    #[test]
    fn a_smpte_division_reads_its_frame_rate() {
        let mut head = 0u16.to_be_bytes().to_vec();
        head.extend_from_slice(&1u16.to_be_bytes());
        head.extend_from_slice(&[0xe8, 80]); // -24 frames a second, 80 ticks each
        let d = Document::new(MemSource(chunk_bytes(b"MThd", &head)));
        let mut ev = Evaluator::new(midi());
        assert_eq!(ev.node(&d, &[0, 2, 2, 0]).unwrap().value, Value::UInt(1));
        assert_eq!(
            ev.node(&d, &[0, 2, 2, 1, 0]).unwrap().value,
            Value::Enum { raw: 104, name: Some("24 fps".into()), hex: false }
        );
        assert_eq!(ev.node(&d, &[0, 2, 2, 1, 1]).unwrap().value, Value::UInt(80));
    }

    #[test]
    fn running_status_carries_the_status_forward() {
        let mut track = vlq_bytes(0);
        track.extend_from_slice(&[0x90, 60, 100]);
        // The same note, off, with the status byte left out.
        track.extend_from_slice(&vlq_bytes(480));
        track.extend_from_slice(&[60, 0]);
        // And once more, so the status has been carried through an event that
        // did not carry it either.
        track.extend_from_slice(&vlq_bytes(10));
        track.extend_from_slice(&[62, 0]);
        let mut out = Vec::new();
        let mut head = 0u16.to_be_bytes().to_vec();
        head.extend_from_slice(&1u16.to_be_bytes());
        head.extend_from_slice(&480u16.to_be_bytes());
        out.extend_from_slice(&chunk_bytes(b"MThd", &head));
        out.extend_from_slice(&chunk_bytes(b"MTrk", &track));

        let d = Document::new(MemSource(out));
        let mut ev = Evaluator::new(midi());
        // Three events, not one event and a stretch of bytes.
        assert_eq!(ev.node(&d, &[1, 2]).unwrap().child_count, 3);
        let on = Value::Enum { raw: 0x90, name: Some("note on ch1".into()), hex: true };
        // The first event carries its status byte and the field has bits.
        assert_eq!(ev.node(&d, &[1, 2, 0, 1]).unwrap().value, on);
        assert_eq!(ev.node(&d, &[1, 2, 0, 1]).unwrap().size_bits, 8);
        // The second leaves it out, so the field is no bits wide and the
        // effective status is the one before it.
        assert_eq!(ev.node(&d, &[1, 2, 1, 1]).unwrap().size_bits, 0);
        assert_eq!(ev.node(&d, &[1, 2, 1, 2]).unwrap().value, on);
        assert_eq!(ev.node(&d, &[1, 2, 1, 3, 0]).unwrap().value, Value::UInt(60));
        assert_eq!(ev.node(&d, &[1, 2, 1, 3, 1]).unwrap().value, Value::UInt(0));
        // The third is carried through an event that had no status of its own.
        assert_eq!(ev.node(&d, &[1, 2, 2, 2]).unwrap().value, on);
        assert_eq!(ev.node(&d, &[1, 2, 2, 3, 0]).unwrap().value, Value::UInt(62));
    }

    #[test]
    fn a_track_that_opens_with_a_data_byte_has_no_status_to_run_from() {
        // No valid file does this: there is nothing to repeat.
        let mut track = vlq_bytes(0);
        track.extend_from_slice(&[60, 100]);
        let d = Document::new(MemSource(chunk_bytes(b"MTrk", &track)));
        let mut ev = Evaluator::new(midi());
        let stopped = ev.node(&d, &[0, 2, 0, 3]).unwrap();
        assert_eq!(stopped.type_name, "NoStatus");
    }
}
