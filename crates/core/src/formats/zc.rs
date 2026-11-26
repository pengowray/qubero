//! Anabat zero-crossing files (`.zc`), the bat call recordings Titley
//! Scientific's Anabat detectors and their software write.
//!
//! A zero-crossing file holds no waveform. The detector divides the frequency
//! of the signal by `divratio`, and the file records how long it was between
//! one zero crossing of the divided signal and the next. Bat software draws
//! each crossing as a dot, at its time and at the frequency two intervals in a
//! row give: `divratio * 1,000,000 / (interval + next interval)` in Hz, with
//! the intervals in microseconds.
//!
//! The layout is Chris Corben's "Anabat File Formats" page, for the four
//! sequence file types 129 to 132. Every number is little-endian.
//!
//! - 0x000: the pointer to the data information table, always 0x011a; the
//!   file type at 0x003; and a 275-byte text header from 0x006 of fixed-size
//!   ASCII fields.
//! - 0x11a: the data information table: where the intervals start, the timing
//!   resolution, the division ratio and the display scale.
//! - 0x120, type 132 only: the date and time of the recording, an ID code, and
//!   the GPS position as text.
//! - 0x150, type 132 only: GUANO metadata, when the intervals start later than
//!   0x150 to make room for it. ZCANT writes it there.
//! - Then the intervals, to the end of the file.
//!
//! An interval is one to four bytes, and the top bits of its first byte say
//! which kind it is. Most are one byte: a 7-bit signed change from the
//! interval before. The rest give the interval outright, or are status codes
//! for the dots after them. The codes differ between file types: type 129
//! has its own coding of a whole interval and a one-byte code that turns dots
//! off, type 130 has the coding 131 and 132 use but the same one-byte code,
//! and types 131 and 132 have a two-byte status code. Each interval's value
//! and the time since the start of the file are worked out for every code, as
//! fields of their own, by adding up the codes before it.
//!
//! Oversample decodes every type with the type 131 and 132 codes. That reads
//! type 129 and 130 files wrongly wherever they turn dots off, and type 129
//! files wrongly wherever an interval is given outright.

use super::wav;
use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T, Until, Valid};

/// Where the data information table is, in every Anabat file.
const DATA_INFO_AT: i128 = 0x11a;

/// Where the type 132 block of date, time and position starts, and where it
/// ends.
const RECORDING_AT: i128 = 0x120;
const RECORDING_END: i128 = 0x150;

pub fn zc() -> Template {
    let file_type = || E::field("file_type");
    let data_pointer = || E::within(&["data_info", "data_pointer"]);
    // Where the header ends: after the data information table, and after the
    // date, time and position too in a type 132 file.
    let header_end = || {
        E::field("data_info_pointer")
            .add(E::lit(6))
            .add(file_type().equal_to(E::lit(132)).mul(E::lit(RECORDING_END - RECORDING_AT)))
    };
    let before_intervals = || data_pointer().sub(header_end()).at_least(E::lit(0)).at_most(E::Remaining);
    let starts_with_guano = E::lit(5).less_or_equal(E::Remaining).both(E::peek(40, Big).equal_to(E::lit(cc("GUANO"))));
    Template::new(
        "zc",
        T::structure(
            "Anabat",
            vec![
                ("data_info_pointer", T::u16(Little)),
                ("padding", T::u8()),
                ("file_type", T::u8()),
                ("padding_2", T::u16(Little)),
                ("text", text_header()),
                (
                    "unused",
                    T::switch(
                        E::field("data_info_pointer").equal_to(E::lit(DATA_INFO_AT)),
                        vec![(1, T::u8())],
                        T::bytes(E::field("data_info_pointer").sub(E::lit(0x119)).at_least(E::lit(0))),
                    ),
                ),
                ("data_info", data_info()),
                ("recording_info", T::when(file_type().equal_to(E::lit(132)), recording_info())),
                (
                    "before_intervals",
                    T::when(
                        before_intervals().greater_than(E::lit(0)),
                        T::sized(before_intervals(), T::switch(starts_with_guano, vec![(1, wav::guano())], T::bytes(E::Remaining))),
                    ),
                ),
                (
                    "intervals",
                    T::switch(
                        file_type(),
                        vec![(129, T::repeat(code_129(), Until::End)), (130, T::repeat(code_130(), Until::End))],
                        T::repeat(code_131(), Until::End),
                    ),
                ),
            ],
        )
        .field_valid("data_info_pointer", Valid::Eq(E::lit(DATA_INFO_AT)))
        .field_valid("file_type", Valid::Range { min: E::lit(129), max: E::lit(132) })
        .field_valid("padding", Valid::Eq(E::lit(0)))
        .field_valid("padding_2", Valid::Eq(E::lit(0)))
        .field_doc("data_info_pointer", "Offset of the data information table. Always 0x011a.")
        .field_doc(
            "file_type",
            "Which of the four layouts the file uses, types 129 to 132. Type 132 adds the recording's start time and GPS position \
             at 0x0120. The interval codes differ between types.",
        )
        .field_doc("padding", "Always 0.")
        .field_doc("padding_2", "Always 0.")
        .field_doc("unused", "Not defined by the format.")
        .field_doc(
            "before_intervals",
            "Bytes between the last header field and the first interval code. When they start with GUANO, they are GUANO \
             metadata, which ZCANT writes here.",
        )
        .field_doc(
            "intervals",
            "Interval codes of 1 to 4 bytes, in recording order. Each code gives the time from one zero crossing to the next, \
             or sets the status of the dots after it. Anabat's software draws each zero crossing as a dot.",
        ),
    )
}

/// A four-character code, or any short run of ASCII, as the big-endian number
/// a peek compares it with.
fn cc(s: &str) -> i128 {
    s.bytes().fold(0i128, |acc, b| (acc << 8) | b as i128)
}

/// Text in a fixed-size field. Anabat's own software pads it with spaces and
/// other programs with zero bytes; either way the field keeps its size.
fn text(n: i128) -> T {
    T::text(StrLen::Padded { size: E::lit(n), pad: 0 }, Encoding::Ascii)
}

/// The 275 bytes of text at 0x006, in the fields Anabat's software shows.
fn text_header() -> T {
    T::structure(
        "TextHeader",
        vec![
            ("tape", text(8)),
            ("date", text(8)),
            ("location", text(40)),
            ("species", text(50)),
            ("spec", text(16)),
            ("note", text(73)),
            ("note1", text(80)),
        ],
    )
    .doc("Anabat's text fields TAPE, DATE, LOC, SPECIES, SPEC, NOTE and NOTE1, each of a fixed size.")
    .field_doc("date", "DATE, as text. A type 132 file also has the start time as numbers at 0x0120.")
    .field_doc("location", "LOC: where the recording was made.")
    .field_doc("species", "SPECIES: the species in the recording.")
    .field_doc("note1", "NOTE1: Anabat's software shows it as the second line of NOTE.")
}

/// The data information table at 0x11a.
fn data_info() -> T {
    let scale = T::enumeration(
        "Scale",
        T::UInt { bits: 3, endian: Big },
        &[(0, "10"), (1, "25"), (2, "50"), (3, "100"), (4, "250"), (5, "500"), (6, "1000"), (7, "2500")],
    );
    T::structure(
        "DataInfo",
        vec![
            ("data_pointer", T::u16(Little)),
            ("res1", T::u16(Little)),
            ("divratio", T::u8()),
            (
                "vres",
                T::structure(
                    "Vres",
                    vec![("unused", T::UInt { bits: 1, endian: Big }), ("scale", scale), ("axis", T::UInt { bits: 4, endian: Big })],
                )
                .field_doc("unused", "Bit 7. Not defined by the format.")
                .field_doc(
                    "scale",
                    "Bits 4 to 6: which of Anabat's eight SCALE values its software draws the recording with. The format note \
                     gives the values and not their unit.",
                )
                .field_doc("axis", "Bits 0 to 3: which of Anabat's three vertical axes its software draws the recording with."),
            ),
        ],
    )
    .field_valid("divratio", Valid::Min(E::lit(1)))
    .field_doc("data_pointer", "Offset of the first interval code: 0x0120 in types 129 to 131, 0x0150 or more in type 132.")
    .field_doc(
        "res1",
        "RES1: the number of counts in 25 ms. It is 25000, one count per microsecond, unless the detector was recalibrated.",
    )
    .field_doc(
        "divratio",
        "DIVRATIO: the division ratio. The detector divided the signal's frequency by this before timing its zero crossings.",
    )
    .field_doc("vres", "VRES: which vertical axis and SCALE Anabat's software uses when it draws the recording.")
}

/// The block at 0x120 in a type 132 file: when the recording was made, an ID,
/// and where.
fn recording_info() -> T {
    T::structure(
        "RecordingInfo",
        vec![
            ("year", T::u16(Little)),
            ("month", T::u8()),
            ("day", T::u8()),
            ("hour", T::u8()),
            ("minute", T::u8()),
            ("second", T::u8()),
            ("hundredths", T::u8()),
            ("microseconds", T::u16(Little)),
            ("id_code", text(6)),
            ("gps", gps()),
        ],
    )
    .field_valid("month", Valid::Max(E::lit(12)))
    .field_valid("day", Valid::Max(E::lit(31)))
    .field_valid("hour", Valid::Max(E::lit(23)))
    .field_valid("minute", Valid::Max(E::lit(59)))
    .field_valid("second", Valid::Max(E::lit(59)))
    .field_valid("hundredths", Valid::Max(E::lit(99)))
    .field_valid("microseconds", Valid::Max(E::lit(9999)))
    .doc(
        "Type 132 only: when the recording started, then an ID code and the GPS position. The time fields are all 0 when the \
         detector did not record the time.",
    )
    .field_doc("hundredths", "Hundredths of a second.")
    .field_doc("microseconds", "Microseconds after the hundredths, 0 to 9999.")
    .field_doc("id_code", "ID code. Its meaning is not defined by the format.")
}

/// The 32 bytes of GPS text at 0x130: a datum, a position in degrees or in UTM,
/// and an altitude. The first character of the position says which.
fn gps() -> T {
    let separator = || text(1);
    let degrees = T::structure(
        "Degrees",
        vec![
            ("latitude_hemisphere", text(1)),
            ("latitude", text(7)),
            ("separator", separator()),
            ("longitude_hemisphere", text(1)),
            ("longitude", text(8)),
        ],
    )
    .field_doc("latitude_hemisphere", "N or S.")
    .field_doc("latitude", "Degrees of latitude, with a decimal point and four decimal places.")
    .field_doc("longitude_hemisphere", "E or W.")
    .field_doc("longitude", "Degrees of longitude, with a decimal point and four decimal places.");
    let utm = T::structure(
        "Utm",
        vec![
            ("zone", text(3)),
            ("separator", separator()),
            ("easting", text(6)),
            ("separator_2", separator()),
            ("northing", text(7)),
        ],
    )
    .field_doc("zone", "UTM zone and latitude band, such as 10S.")
    .field_doc("easting", "Easting in metres.")
    .field_doc("northing", "Northing in metres.");
    let mut by_first: Vec<(i128, T)> = vec![(b'N' as i128, degrees.clone()), (b'S' as i128, degrees)];
    by_first.extend((b'0'..=b'9').map(|d| (d as i128, utm.clone())));
    T::structure(
        "Gps",
        vec![
            ("datum", text(10)),
            ("position", T::switch(E::peek(8, Big), by_first, text(18))),
            ("altitude", text(4)),
        ],
    )
    .field_doc("datum", "The map datum the position is in, such as WGS84.")
    .field_doc(
        "position",
        "Latitude and longitude in degrees when it starts with N or S, a UTM grid position when it starts with a digit, \
         and blank when the recorder had no position.",
    )
    .field_doc("altitude", "Altitude in metres, -999 to 9999.")
}

/// The two fields every interval code ends with, worked out from the codes
/// before it: the interval itself, and the time since the start of the file
/// once it has passed. `interval` is the expression for this code's interval;
/// a code that only turns dots off or sets their status passes on the
/// interval before it, so that a change after it is a change from that.
fn running(interval: E) -> Vec<(&'static str, T)> {
    vec![("interval", T::computed(interval)), ("time", T::computed(E::prev("time").add(E::field("interval"))))]
}

/// The same for a code that is not an interval: both values carry on from the
/// code before.
fn carried() -> Vec<(&'static str, T)> {
    vec![("interval", T::computed(E::prev("interval"))), ("time", T::computed(E::prev("time")))]
}

/// Doc text for the two worked-out fields of a code that gives an interval.
fn with_running_docs(t: T) -> T {
    t.field_doc(
        "interval",
        "The interval, worked out from this code and the codes before it. In microseconds when res1 is 25000; otherwise one \
         unit is 25 ms / res1.",
    )
    .field_doc(
        "time",
        "Time from the start of the recording to the end of this interval: the sum of every interval so far, in the same units \
         as interval.",
    )
}

/// The same for a code that gives no interval.
fn with_carried_docs(t: T) -> T {
    t.field_doc(
        "interval",
        "This code gives no interval, so this is the interval before it, which the next change is added to.",
    )
    .field_doc("time", "This code gives no interval, so this is the time at the end of the interval before it.")
}

/// A one-byte change: bit 7 is 0, and the other seven bits are a signed number
/// added to the interval before.
fn change() -> T {
    let mut fields: Vec<(&str, T)> = vec![
        ("kind", T::enumeration("CodeKind", T::UInt { bits: 1, endian: Big }, &[(0, "change")])),
        ("change", T::Int { bits: 7, endian: Big }),
    ];
    fields.extend(running(E::prev("interval").add(E::field("change"))));
    with_running_docs(T::structure("Change", fields))
        .field_doc("kind", "Bit 7 is 0: a one-byte change from the interval before.")
        .field_doc("change", "-64 to 63, added to the interval before.")
}

/// An interval given outright, in the low five bits of the first byte and the
/// whole of the bytes after it.
fn whole(bits: u32) -> T {
    let mut fields: Vec<(&str, T)> = vec![
        ("kind", T::enumeration("CodeKind", T::UInt { bits: 3, endian: Big }, &[(4, "13-bit interval"), (5, "21-bit interval"), (6, "29-bit interval")])),
        ("value", T::UInt { bits, endian: Big }),
    ];
    fields.extend(running(E::field("value")));
    with_running_docs(T::structure("Interval", fields))
        .field_doc("kind", "Bits 7 to 5: 4 (binary 100), 5 (101) or 6 (110) when one, two or three bytes follow this byte.")
        .field_doc("value", "The interval, high bits first: the low five bits of the first byte, then the bytes after it.")
}

/// Type 129's interval given outright: bit 7 is 1, bits 3 to 6 are a shift,
/// bits 0 to 2 and the next byte an 11-bit number, and the interval is the
/// number shifted left that many times.
fn shifted() -> T {
    let mut fields: Vec<(&str, T)> = vec![
        ("kind", T::enumeration("CodeKind", T::UInt { bits: 1, endian: Big }, &[(1, "shifted interval")])),
        ("shift", T::UInt { bits: 4, endian: Big }),
        ("value", T::UInt { bits: 11, endian: Big }),
    ];
    fields.extend(running(E::field("value").shl(E::field("shift"))));
    with_running_docs(T::structure("ShiftedInterval", fields))
        .field_doc("kind", "Bit 7 is 1: an interval given outright, in two bytes.")
        .field_doc("shift", "How many places the value is shifted left to give the interval.")
        .field_doc("value", "Bits 0 to 2 of the first byte, then the second byte, high bits first.")
}

/// A one-byte code that turns the next few dots off: the bits above `count`
/// are all 1. Type 129 has 3 bits of count, type 130 has 5.
fn turn_off(count_bits: u32) -> T {
    let mut fields: Vec<(&str, T)> = vec![
        ("kind", T::enumeration("CodeKind", T::UInt { bits: 8 - count_bits, endian: Big }, &[((1 << (8 - count_bits)) - 1, "turn-off code")])),
        ("count", T::UInt { bits: count_bits, endian: Big }),
    ];
    fields.extend(carried());
    with_carried_docs(T::structure("TurnOff", fields))
        .field_doc("kind", "The top bits are all 1, five bits in type 129 and three in type 130: a turn-off code.")
        .field_doc(
            "count",
            "How many dots after this code are turned off. Their intervals are still in the file; the dots were turned off by \
             editing before the file was saved.",
        )
}

/// The two-byte status code of types 131 and 132: bits 7 to 5 are all 1, the
/// low five bits are a status, and the next byte says how many of the dots
/// after it have that status.
fn status() -> T {
    let mut fields: Vec<(&str, T)> = vec![
        ("kind", T::enumeration("CodeKind", T::UInt { bits: 3, endian: Big }, &[(7, "status code")])),
        (
            "status",
            T::enumeration(
                "DotStatus",
                T::UInt { bits: 5, endian: Big },
                &[(0, "out of range"), (1, "turned off"), (2, "normal"), (3, "main dot")],
            ),
        ),
        ("count", T::u8()),
    ];
    fields.extend(carried());
    with_carried_docs(T::structure("Status", fields))
        .field_valid("status", Valid::InEnum)
        .field_doc("kind", "Bits 7 to 5 are all 1: a status code for the dots after it.")
        .field_doc("status", "The status of the dots after this code. In practice only 1 and 3 are used.")
        .field_doc("count", "How many of the dots after this code have the status.")
}

/// The top three bits of the next byte, which say what kind of code it is in
/// types 130 to 132.
fn top_three() -> E {
    E::peek(3, Big)
}

/// A code in a type 131 or 132 file.
fn code_131() -> T {
    T::switch(top_three(), vec![(4, whole(13)), (5, whole(21)), (6, whole(29)), (7, status())], change())
}

/// A code in a type 130 file: the same, except that a code starting with
/// three 1 bits is one byte, which turns dots off.
fn code_130() -> T {
    T::switch(top_three(), vec![(4, whole(13)), (5, whole(21)), (6, whole(29)), (7, turn_off(5))], change())
}

/// A code in a type 129 file.
fn code_129() -> T {
    T::switch(
        E::peek(1, Big),
        vec![(1, T::switch(E::peek(5, Big).equal_to(E::lit(0x1f)), vec![(1, turn_off(3))], shifted()))],
        change(),
    )
}

/// Whether these bytes are an Anabat file, by the bytes alone: the pointer
/// every one of them has, the zero bytes around the file type, text in the
/// text header, and a data information table whose values fit the file.
pub fn is_zc(head: &[u8], len: u64) -> bool {
    let Some(h) = head.get(..RECORDING_AT as usize) else { return false };
    let u16_at = |at: usize| u16::from_le_bytes([h[at], h[at + 1]]);
    let file_type = h[3];
    if u16_at(0) != DATA_INFO_AT as u16 || h[2] != 0 || h[4] != 0 || h[5] != 0 || !(129..=132).contains(&file_type) {
        return false;
    }
    let is_text = |b: &u8| *b == 0 || *b == b'\t' || (*b >= 0x20 && *b != 0x7f);
    if !h[0x006..0x119].iter().all(is_text) {
        return false;
    }
    let first_interval = if file_type == 132 { RECORDING_END } else { RECORDING_AT } as u64;
    let data_pointer = u16_at(0x11a) as u64;
    let res1 = u16_at(0x11c);
    let divratio = h[0x11e];
    (first_interval..=len).contains(&data_pointer) && (10_000..=60_000).contains(&res1) && divratio > 0
}

/// Whether a file named `.zc` is an Anabat file: a file type of 129 to 132
/// and a pointer to a data information table inside the file. The test
/// Oversample makes, which takes a file whose header [`is_zc`] would not.
pub fn is_named_zc(head: &[u8], len: u64) -> bool {
    let Some(h) = head.get(..0x119) else { return false };
    let pointer = u16::from_le_bytes([h[0], h[1]]) as u64;
    (129..=132).contains(&h[3]) && pointer >= 0x119 && pointer + 6 <= len
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// The timing resolution of a detector that has not been recalibrated:
    /// 25,000 counts in 25 ms, so one count is one microsecond.
    const DEFAULT_RES1: u16 = 25_000;

    /// A header of the given type, with text in a few fields, and the data
    /// information table saying the intervals start at `data_pointer`.
    fn header(file_type: u8, data_pointer: u16) -> Vec<u8> {
        let mut v = vec![0u8; 0x120];
        v[0..2].copy_from_slice(&0x011au16.to_le_bytes());
        v[3] = file_type;
        v[0x16..0x16 + 13].copy_from_slice(b"Skeleton Cave");
        v[0x3e..0x3e + 6].copy_from_slice(b"Myotis");
        v[0x11a..0x11c].copy_from_slice(&data_pointer.to_le_bytes());
        v[0x11c..0x11e].copy_from_slice(&DEFAULT_RES1.to_le_bytes());
        v[0x11e] = 16;
        v[0x11f] = 0x52;
        v
    }

    /// A type 132 file: the date, the time, a position in degrees, GUANO
    /// when `guano` is not empty, and then `codes`.
    fn type_132(guano: &[u8], codes: &[u8]) -> Vec<u8> {
        let mut v = header(132, (RECORDING_END as usize + guano.len()) as u16);
        v.extend_from_slice(&2024u16.to_le_bytes());
        v.extend_from_slice(&[3, 15, 21, 4, 59, 42]);
        v.extend_from_slice(&1234u16.to_le_bytes());
        v.extend_from_slice(b"ABC123");
        let mut gps = b"WGS84     S33.7394 E150.2539".to_vec();
        gps.resize(28, b' ');
        gps.extend_from_slice(b" 120");
        v.extend_from_slice(&gps);
        assert_eq!(v.len(), RECORDING_END as usize);
        v.extend_from_slice(guano);
        v.extend_from_slice(codes);
        v
    }

    fn eval(bytes: Vec<u8>) -> (Document<MemSource>, Evaluator) {
        (Document::new(MemSource(bytes)), Evaluator::new(zc()))
    }

    /// Child `name` of the node at `path`.
    fn child(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], name: &str) -> Vec<usize> {
        let n = ev.node(d, path).unwrap().child_count as usize;
        for i in 0..n {
            let mut p = path.to_vec();
            p.push(i);
            if ev.node(d, &p).unwrap().name == name {
                return p;
            }
        }
        panic!("no {name} under {path:?}");
    }

    /// The value of child `name` of the node at `path`.
    fn under(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], name: &str) -> Value {
        let p = child(ev, d, path, name);
        ev.node(d, &p).unwrap().value
    }

    fn at(ev: &mut Evaluator, d: &Document<MemSource>, names: &[&str]) -> Value {
        let mut p = Vec::new();
        for n in names {
            p = child(ev, d, &p, n);
        }
        ev.node(d, &p).unwrap().value
    }

    fn int(v: Value) -> i128 {
        match v {
            Value::Int(v) => v,
            Value::UInt(v) => v as i128,
            other => panic!("{other:?}"),
        }
    }

    /// The interval and time of every code in the file, read in order.
    fn intervals_and_times(ev: &mut Evaluator, d: &Document<MemSource>) -> Vec<(i128, i128)> {
        let list = child(ev, d, &[], "intervals");
        let n = ev.node(d, &list).unwrap().child_count as usize;
        (0..n)
            .map(|i| {
                let p = [list.clone(), vec![i]].concat();
                (int(under(ev, d, &p, "interval")), int(under(ev, d, &p, "time")))
            })
            .collect()
    }

    #[test]
    fn a_type_132_header_reads_its_text_table_time_and_position() {
        let (d, mut ev) = eval(type_132(&[], &[0x80, 0x32]));
        assert_eq!(at(&mut ev, &d, &["file_type"]), Value::UInt(132));
        assert_eq!(at(&mut ev, &d, &["text", "location"]), Value::Str("Skeleton Cave".into()));
        assert_eq!(at(&mut ev, &d, &["text", "species"]), Value::Str("Myotis".into()));
        assert_eq!(at(&mut ev, &d, &["data_info", "res1"]), Value::UInt(25_000));
        assert_eq!(at(&mut ev, &d, &["data_info", "divratio"]), Value::UInt(16));
        // 0x52: scale 5, axis 2.
        assert_eq!(
            at(&mut ev, &d, &["data_info", "vres", "scale"]),
            Value::Enum { raw: 5, name: Some("500".into()), hex: false }
        );
        assert_eq!(at(&mut ev, &d, &["data_info", "vres", "axis"]), Value::UInt(2));
        assert_eq!(at(&mut ev, &d, &["recording_info", "year"]), Value::UInt(2024));
        assert_eq!(at(&mut ev, &d, &["recording_info", "second"]), Value::UInt(59));
        assert_eq!(at(&mut ev, &d, &["recording_info", "hundredths"]), Value::UInt(42));
        assert_eq!(at(&mut ev, &d, &["recording_info", "microseconds"]), Value::UInt(1234));
        assert_eq!(at(&mut ev, &d, &["recording_info", "id_code"]), Value::Str("ABC123".into()));
        assert_eq!(at(&mut ev, &d, &["recording_info", "gps", "datum"]), Value::Str("WGS84     ".into()));
        assert_eq!(at(&mut ev, &d, &["recording_info", "gps", "position", "latitude_hemisphere"]), Value::Str("S".into()));
        assert_eq!(at(&mut ev, &d, &["recording_info", "gps", "position", "latitude"]), Value::Str("33.7394".into()));
        assert_eq!(at(&mut ev, &d, &["recording_info", "gps", "position", "longitude"]), Value::Str("150.2539".into()));
        assert_eq!(at(&mut ev, &d, &["recording_info", "gps", "altitude"]), Value::Str(" 120".into()));
        // Every byte of the file is covered.
        let len = d.len_bits();
        assert_eq!(ev.node(&d, &[]).unwrap().size_bits, len);
    }

    #[test]
    fn a_utm_position_reads_as_zone_easting_and_northing() {
        let mut bytes = type_132(&[], &[0x80, 0x32]);
        let mut gps = b"WGS84     10S 551234 4181234".to_vec();
        gps.resize(32, b' ');
        bytes[0x130..0x150].copy_from_slice(&gps);
        let (d, mut ev) = eval(bytes);
        assert_eq!(at(&mut ev, &d, &["recording_info", "gps", "position", "zone"]), Value::Str("10S".into()));
        assert_eq!(at(&mut ev, &d, &["recording_info", "gps", "position", "easting"]), Value::Str("551234".into()));
        assert_eq!(at(&mut ev, &d, &["recording_info", "gps", "position", "northing"]), Value::Str("4181234".into()));
    }

    #[test]
    fn guano_between_the_header_and_the_intervals_reads_as_fields() {
        let guano = b"GUANO|Version: 1.0\nMake: Titley\nZCANT|Amplitudes: AAA=\n";
        let (d, mut ev) = eval(type_132(guano, &[0x80, 0x32, 0x10]));
        let guano = child(&mut ev, &d, &[], "before_intervals");
        assert_eq!(ev.node(&d, &guano).unwrap().type_name, "Guano");
        let lines = child(&mut ev, &d, &guano, "fields");
        assert_eq!(ev.node(&d, &lines).unwrap().child_count, 3);
        let first = [lines, vec![0]].concat();
        assert_eq!(under(&mut ev, &d, &first, "namespace"), Value::Str("GUANO".into()));
        assert_eq!(under(&mut ev, &d, &first, "key"), Value::Str("Version".into()));
        assert_eq!(under(&mut ev, &d, &first, "value"), Value::Str("1.0".into()));
        // The intervals still start where the table says.
        assert_eq!(intervals_and_times(&mut ev, &d), vec![(50, 50), (66, 116)]);
    }

    /// Changes, whole intervals and a status code between them: the status
    /// passes the interval on, so the change after it is from the interval
    /// before the status.
    #[test]
    fn intervals_add_up_across_changes_whole_values_and_status_codes() {
        let codes = [
            0x80, 0x32, // 13-bit: 50
            0x10, // +16: 66
            0x7f, // -1: 65
            0xe1, 0x02, // turn the next two dots off
            0x03, // +3: 68
            0xa0, 0x01, 0x00, // 21-bit: 256
            0xc0, 0x01, 0x00, 0x00, // 29-bit: 65536
        ];
        let (d, mut ev) = eval(type_132(&[], &codes));
        assert_eq!(
            intervals_and_times(&mut ev, &d),
            vec![(50, 50), (66, 116), (65, 181), (65, 181), (68, 249), (256, 505), (65_536, 66_041)]
        );
        let status = [child(&mut ev, &d, &[], "intervals"), vec![3]].concat();
        assert_eq!(ev.node(&d, &status).unwrap().type_name, "Status");
        assert_eq!(
            under(&mut ev, &d, &status, "status"),
            Value::Enum { raw: 1, name: Some("turned off".into()), hex: false }
        );
        assert_eq!(under(&mut ev, &d, &status, "count"), Value::UInt(2));
    }

    /// Type 130's code for turning dots off is one byte, where 131's is two.
    #[test]
    fn type_130_turns_dots_off_with_one_byte() {
        let mut v = header(130, 0x120);
        v.extend_from_slice(&[0x80, 0x32, 0xe2, 0x10]);
        let (d, mut ev) = eval(v);
        assert_eq!(intervals_and_times(&mut ev, &d), vec![(50, 50), (50, 50), (66, 116)]);
        let off = [child(&mut ev, &d, &[], "intervals"), vec![1]].concat();
        assert_eq!(ev.node(&d, &off).unwrap().type_name, "TurnOff");
        assert_eq!(under(&mut ev, &d, &off, "count"), Value::UInt(2));
    }

    /// Type 129 gives an interval outright as a number and a shift, and turns
    /// dots off with three bits of count. The bytes are the worked example on
    /// Corben's page: 128,100 is 100; 251 turns three dots off; 149,213 is
    /// (5 * 256 + 213) shifted left twice, 5972.
    #[test]
    fn type_129_reads_shifted_intervals_and_its_own_turn_off_code() {
        let mut v = header(129, 0x120);
        v.extend_from_slice(&[128, 100, 50, 10, 251, 40, 149, 213]);
        let (d, mut ev) = eval(v);
        assert_eq!(
            intervals_and_times(&mut ev, &d),
            vec![(100, 100), (150, 250), (160, 410), (160, 410), (200, 610), (5972, 6582)]
        );
        let off = [child(&mut ev, &d, &[], "intervals"), vec![3]].concat();
        assert_eq!(under(&mut ev, &d, &off, "count"), Value::UInt(3));
    }

    /// A long run of changes read from its far end first, with nothing read
    /// before it: each code's interval is the one before it plus its change,
    /// so the last asks about every code in the file, far more than one read
    /// may ask about at once. A debug build takes about 1.5 ms a code for
    /// this; read in order, each code takes a few microseconds.
    #[test]
    fn the_last_of_five_thousand_codes_adds_up_read_first() {
        const N: i128 = 5_000;
        let mut codes = vec![0x80, 0x00];
        codes.extend(std::iter::repeat(0x01).take(N as usize));
        let (d, mut ev) = eval(type_132(&[], &codes));
        let list = child(&mut ev, &d, &[], "intervals");
        let last = [list, vec![N as usize]].concat();
        assert_eq!(int(under(&mut ev, &d, &last, "interval")), N);
        assert_eq!(int(under(&mut ev, &d, &last, "time")), N * (N + 1) / 2);
    }

    #[test]
    fn recognised_by_its_header_and_by_its_name() {
        let bytes = type_132(&[], &[0x80, 0x32]);
        assert!(is_zc(&bytes, bytes.len() as u64));
        assert_eq!(super::super::sniff(&bytes, bytes.len() as u64), Some("zc"));
        // Type 129 to 131 intervals start at 0x120.
        let old = header(131, 0x120);
        assert!(is_zc(&old, old.len() as u64));
        // A table pointing past the end, a type out of range, and control
        // bytes in the text are each enough to refuse it.
        let mut past = bytes.clone();
        past[0x11a..0x11c].copy_from_slice(&0x2000u16.to_le_bytes());
        assert!(!is_zc(&past, past.len() as u64));
        let mut wrong = bytes.clone();
        wrong[3] = 133;
        assert!(!is_zc(&wrong, wrong.len() as u64));
        let mut binary = bytes.clone();
        binary[0x20] = 0x01;
        assert!(!is_zc(&binary, binary.len() as u64));
        // The name takes a file the header test refuses, as Oversample does.
        assert!(is_named_zc(&binary, binary.len() as u64));
        assert_eq!(super::super::sniff_named(&binary, binary.len() as u64, "A1011811.zc"), Some("zc"));
        assert!(!is_named_zc(&wrong, wrong.len() as u64));
    }
}
