//! What one of the standard library's own values comes to, in the words
//! Python writes it in: a date, a time, an exact number, an id or a path.
//!
//! Apart from [`picklesaid`](super::picklesaid) because a packed `datetime` is
//! not a value the tree holds anywhere. It is ten bytes handed to a class, and
//! what they say is a year, a month, a day and a clock; the row a reader sees
//! is worked out here from those bytes and from the zone beside them. See
//! [`stdlib`](crate::formats::pickle::familiar) for how they are read.

use super::pickleparts::made_at;
use super::*;
use crate::formats::pickle::familiar::{Kind, Match, Names, Shape, Value};

/// The classes whose value is worked out here rather than read where it sits.
pub(super) fn is_stdlib(what: Shape) -> bool {
    matches!(
        what,
        Shape::DateTime | Shape::Date | Shape::Time | Shape::TimeDelta | Shape::TimeZone | Shape::Decimal | Shape::Fraction | Shape::Path
    ) || is_container(what)
}

/// The standard library's own containers, which read the way a dictionary and
/// a list read: what kind of thing, and how much of it.
fn is_container(what: Shape) -> bool {
    matches!(what, Shape::Counter | Shape::OrderedDict | Shape::DefaultDict | Shape::Deque)
}

/// The dotted path of the class an object was made from.
fn class_of(value: &Value) -> Option<&str> {
    let Kind::Instance { class, .. } = &value.kind else { return None };
    match &class.kind {
        Kind::Class { path, .. } => Some(path),
        _ => None,
    }
}

/// Whether this object is a `uuid.UUID`, which is an ordinary object holding
/// one 128-bit number and reads as the hyphenated hexadecimal everything else
/// writes an id in.
pub(super) fn is_uuid(value: &Value) -> bool {
    class_of(value).is_some_and(|path| path == "uuid.UUID")
}

/// The name of the one attribute a `uuid.UUID` is rebuilt from.
const UUID_INT: &str = "int";

/// The whole number a value holds, for the two widths a 128-bit number takes.
fn whole_number(value: &Value) -> Option<u128> {
    match &value.kind {
        Kind::Int { value, .. } => u128::try_from(*value).ok(),
        Kind::Wide { digits, .. } => digits.parse().ok(),
        _ => None,
    }
}

/// An id as everything but a pickle writes one: thirty-two hexadecimal digits
/// in five runs.
fn hyphenated(number: u128) -> String {
    let digits = format!("{number:032x}");
    let mut out = String::with_capacity(36);
    for (i, cut) in [8usize, 12, 16, 20].iter().enumerate() {
        let from = [0usize, 8, 12, 16][i];
        out.push_str(&digits[from..*cut]);
        out.push('-');
    }
    out.push_str(&digits[20..]);
    out
}

/// The year a packed date carries, which is written most significant byte
/// first where everything else in a pickle is the other way round.
fn year(packed: &[u8]) -> u16 {
    u16::from_be_bytes([packed[0], packed[1]])
}

/// A packed date as ISO 8601 writes one. The top bit of the month is the fold
/// flag rather than part of the number.
fn iso_date(packed: &[u8]) -> String {
    format!("{:04}-{:02}-{:02}", year(packed), packed[2] & 0x7f, packed[3])
}

/// A packed clock as ISO 8601 writes one, with the microseconds only where
/// there are any, which is what `time.isoformat` does. The top bit of the hour
/// is the fold flag.
fn iso_time(packed: &[u8]) -> String {
    let micro = u32::from_be_bytes([0, packed[3], packed[4], packed[5]]);
    let clock = format!("{:02}:{:02}:{:02}", packed[0] & 0x7f, packed[1], packed[2]);
    match micro {
        0 => clock,
        _ => format!("{clock}.{micro:06}"),
    }
}

/// A `timedelta` as Python's own `str` writes one: the days where there are
/// any, the clock, and the microseconds where there are any.
fn said_span(days: i128, seconds: i128, micro: i128) -> String {
    let (minutes, second) = (seconds / 60, seconds % 60);
    let (hour, minute) = (minutes / 60, minutes % 60);
    let mut out = String::new();
    if days != 0 {
        let plural = if days.abs() == 1 { "" } else { "s" };
        out.push_str(&format!("{days} day{plural}, "));
    }
    out.push_str(&format!("{hour}:{minute:02}:{second:02}"));
    if micro != 0 {
        out.push_str(&format!(".{micro:06}"));
    }
    out
}

/// The three numbers a `timedelta` was made from, which Python keeps
/// normalised: the seconds and the microseconds are never negative and a
/// negative span is a negative count of days.
fn span_of(value: &Value) -> Option<(i128, i128, i128)> {
    let Kind::Made { what: Shape::TimeDelta, items, .. } = &value.kind else { return None };
    let mut held = items.iter().map(|item| match item.kind {
        Kind::Int { value, .. } => Some(value),
        _ => None,
    });
    Some((held.next()??, held.next()??, held.next()??))
}

/// What a zone adds to an ISO 8601 time: the offset from UTC, signed, in hours
/// and minutes, and seconds where the offset has any.
fn said_offset(value: &Value) -> Option<String> {
    let Kind::Made { what: Shape::TimeZone, items, .. } = &value.kind else { return None };
    let (days, seconds, micro) = span_of(items.first()?)?;
    if micro != 0 {
        return None;
    }
    let total = days.checked_mul(86_400)?.checked_add(seconds)?;
    let sign = if total < 0 { '-' } else { '+' };
    let away = total.abs();
    let (hour, rest) = (away / 3600, away % 3600);
    let (minute, second) = (rest / 60, rest % 60);
    let mut out = format!("{sign}{hour:02}:{minute:02}");
    if second != 0 {
        out.push_str(&format!(":{second:02}"));
    }
    Some(out)
}

/// The zone a date or a time was written with, which is spelled out the first
/// time the file uses it and named out of the memo after that.
///
/// A name says where the file wrote the thing, so the zone is looked for at
/// that offset: see [`made_at`].
fn zone_of<'a>(found: &'a Match, value: &'a Value) -> Option<&'a Value> {
    match &value.kind {
        Kind::Made { what: Shape::TimeZone, .. } => Some(value),
        Kind::Ref(Names::Made { what: Shape::TimeZone, at, .. }) => made_at(found, Shape::TimeZone, *at),
        _ => None,
    }
}

/// A path as Python's `str` writes one: the parts joined by the separator the
/// class uses, with the root run into the first part after it.
fn said_path(class: Option<&str>, parts: &[String]) -> String {
    let windows = class.is_some_and(|path| path.ends_with("PureWindowsPath"));
    let separator = if windows { "\\" } else { "/" };
    let mut out = String::new();
    for (i, part) in parts.iter().enumerate() {
        // The root is a part of its own, `/` or `C:\`, and the part after it
        // follows with no separator between them.
        if i > 0 && !out.ends_with('/') && !out.ends_with('\\') {
            out.push_str(separator);
        }
        out.push_str(part);
    }
    match out.is_empty() {
        true => ".".to_string(),
        false => out,
    }
}

impl Evaluator {
    /// What one of the standard library's values comes to, or nothing for any
    /// other value, which leaves the row to [`Evaluator::pickle_said`].
    pub(super) fn pickle_stdlib<S: Source>(
        &self,
        doc: &Document<S>,
        found: &Match,
        whole: &Resolved,
        base: u64,
        v: &Value,
    ) -> R<Option<String>> {
        // An id is the one 128-bit number its state holds under `int`. The
        // attribute is named rather than taken as the only one:
        // `UUID.__getstate__` writes a second when it knows whether the id is
        // safe to use across a fork, and what that one says is not the number.
        if is_uuid(v) {
            let Kind::Instance { state: Some(state), .. } = &v.kind else { return Ok(None) };
            let Kind::Dict(entries) = &state.kind else { return Ok(None) };
            for (key, held) in entries {
                if self.pickle_text(doc, whole, base, key)?.as_deref() == Some(UUID_INT) {
                    return Ok(whole_number(held).map(hyphenated));
                }
            }
            return Ok(None);
        }
        let Kind::Made { what, items, state, .. } = &v.kind else { return Ok(None) };
        if is_container(*what) {
            // What it holds and how much of it, which is what a dictionary and
            // a list say. The class is the row inside it, as an object's is.
            let held = match state.as_deref().map(|s| &s.kind) {
                Some(Kind::Dict(entries)) => entries.len(),
                Some(Kind::List(items)) => items.len(),
                _ => return Ok(None),
            };
            let word = what.name();
            return Ok(Some(match held {
                0 => format!("empty {word}"),
                n => format!("{word} of {n}"),
            }));
        }
        Ok(match what {
            Shape::Date => self.packed(doc, found, whole, base, items.first(), 4)?.map(|p| iso_date(&p)),
            Shape::Time => {
                let zone = items.get(1).and_then(|z| zone_of(found, z)).and_then(said_offset);
                match (self.packed(doc, found, whole, base, items.first(), 6)?, items.len() > 1) {
                    (Some(packed), false) => Some(iso_time(&packed)),
                    (Some(packed), true) => zone.map(|zone| format!("{}{zone}", iso_time(&packed))),
                    (None, _) => None,
                }
            }
            Shape::DateTime => {
                let zone = items.get(1).and_then(|z| zone_of(found, z)).and_then(said_offset);
                let said = self.packed(doc, found, whole, base, items.first(), 10)?.map(|p| format!("{}T{}", iso_date(&p), iso_time(&p[4..])));
                match (said, items.len() > 1) {
                    (Some(said), false) => Some(said),
                    // A value whose zone the file named rather than spelled,
                    // and that is nowhere the walk above reached, reads as
                    // nothing at all: the clock without its offset is a time
                    // this reader cannot vouch for.
                    (Some(said), true) => zone.map(|zone| format!("{said}{zone}")),
                    (None, _) => None,
                }
            }
            Shape::TimeDelta => span_of(v).map(|(days, seconds, micro)| said_span(days, seconds, micro)),
            Shape::TimeZone => said_offset(v),
            // The text is the value: a `Decimal` and a one-argument
            // `Fraction` are built from exactly what their own `str` writes.
            Shape::Decimal => self.pickle_text(doc, whole, base, items.first().unwrap_or(v))?,
            Shape::Fraction => match items.len() {
                2 => {
                    let said: Vec<String> =
                        items.iter().filter_map(|x| self.pickle_text(doc, whole, base, x).ok().flatten()).collect();
                    (said.len() == 2).then(|| said.join("/"))
                }
                _ => self.pickle_text(doc, whole, base, items.first().unwrap_or(v))?,
            },
            Shape::Path => {
                let Some(Kind::Tuple(parts)) = items.first().map(|x| &x.kind) else { return Ok(None) };
                let mut said = Vec::with_capacity(parts.len());
                for part in parts {
                    match self.pickle_text(doc, whole, base, part)? {
                        Some(text) => said.push(text),
                        None => return Ok(None),
                    }
                }
                let class = match &v.kind {
                    Kind::Made { callable: Some(callable), .. } => match &callable.kind {
                        Kind::Class { path, .. } => Some(path.as_str()),
                        _ => None,
                    },
                    _ => None,
                };
                Some(said_path(class, &said))
            }
            _ => None,
        })
    }

    /// The bytes a packed date, time or datetime is, however the protocol had
    /// to write them, and nothing where the run is not the length it must be.
    ///
    /// The same four spellings the recogniser reads: the bytes themselves, the
    /// latin-1 text Python 3 spells them as below protocol 3, that text again
    /// as a protocol 0 line, and the Python 2 `str` that is the bytes.
    fn packed<S: Source>(
        &self,
        doc: &Document<S>,
        found: &Match,
        whole: &Resolved,
        base: u64,
        value: Option<&Value>,
        wide: usize,
    ) -> R<Option<Vec<u8>>> {
        let Some(value) = value else { return Ok(None) };
        let held = match &value.kind {
            Kind::Bytes { at, len } | Kind::Text { at, len } => Some(self.read(doc, whole, base + *at as u64 * 8, *len as u64 * 8)?),
            Kind::Ref(Names::Bytes { at, len }) | Kind::Ref(Names::Text { at, len }) => {
                Some(self.read(doc, whole, base + *at as u64 * 8, *len as u64 * 8)?)
            }
            Kind::Spelled { at, .. } => found.decoded(*at).map(|held| held.to_vec()),
            Kind::Made { what: Shape::Bytes, items, .. } => match items.first().map(|x| &x.kind) {
                Some(Kind::Text { at, len }) => {
                    let run = self.read(doc, whole, base + *at as u64 * 8, *len as u64 * 8)?;
                    crate::formats::pickle::familiar::Storage::Latin1.read(&run)
                }
                Some(Kind::Spelled { at, .. }) => {
                    found.decoded(*at).and_then(|held| crate::formats::pickle::familiar::Storage::Latin1.read(&held))
                }
                _ => None,
            },
            _ => None,
        };
        Ok(held.filter(|held| held.len() == wide))
    }
}
