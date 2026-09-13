//! What moment a field means.
//!
//! One question, [`Evaluator::time_of`], and the same rule the checksum file is
//! built around: a fact the core cannot establish is answered as nothing, never
//! guessed. A field the template has not declared as a time answers nothing; a
//! number whose instant falls outside the years this will name answers
//! [`Moment::Impossible`] rather than a wrapped-around date; a packed field
//! holding a thirteenth month answers the same. What must never come back is a
//! confident date that is wrong, because that is worse than the integer it
//! replaced: an integer is obviously something the reader has to decode, and a
//! wrong date is something they will believe.
//!
//! Cheap, but not free the way [`check_of`](super::Evaluator::check_of) is: the
//! field's own bytes are read, since the number in them is the answer. That is
//! the field the panel has already resolved to draw its value row, so asking
//! costs a memo hit, and no other byte of the file is touched.
//!
//! Nothing here formats anything. The answer is an instant and the
//! qualification that goes with it; the words a reader sees belong to the
//! interface, as [`Verdict`](super::Verdict)'s two strings do.

use super::*;
use crate::template::{Atomic, Counted, Epoch, Time, Unset, Zone};

pub mod leap_seconds;

/// The moment the field at a path means, and what is honest to say about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeInfo {
    pub moment: Moment,
    /// What the format says about the zone. For [`Zone::Local`] and
    /// [`Zone::Unknown`] the instant below is the reading laid on the UTC line,
    /// which is to say the digits as the file wrote them: an interface prints
    /// them and says which of the three this was, and must not shift them into
    /// a zone the file never named.
    pub zone: Zone,
    /// The smallest step the field can express, in nanoseconds: a thousand
    /// million for a field counting seconds, a hundred for a FILETIME, two
    /// thousand million for an MS-DOS time, which counts seconds in twos.
    ///
    /// It is here so that an interface prints the digits the file actually has.
    /// A journal's microseconds want six decimal places and a gzip `mtime`
    /// wants none, and showing `.000000` on the second is inventing precision
    /// the file never claimed.
    pub step_nanos: u64,
    /// What else has to be said for the moment to be read honestly, if
    /// anything. Only a count on a clock with leap seconds has anything to say
    /// yet.
    pub note: Option<TimeNote>,
}

/// A qualification on a moment that is right as far as it goes.
///
/// Not a fourth kind of [`Moment`], because the moment is still the best answer
/// there is and an interface should still show it. What it must not do is show
/// it as plainly as a count from 1970, and this is what it says instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeNote {
    /// The moment is after the last day the table of leap seconds vouches for.
    /// It is worked out as if none has been added since, and one announced
    /// after the table was copied would make it a second late. See
    /// [`leap_seconds::EXPIRES`].
    PastLeapSecondTable,
    /// The moment is before 1972, when UTC had rubber seconds and fractional
    /// steps rather than leap seconds, and before 1960 no UTC at all. It is
    /// worked out as the NASA CDF library works it out, which another library
    /// may not agree with to the millisecond, and before 1960 to the second.
    BeforeLeapSeconds,
}

/// What the number in the field came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Moment {
    /// The instant: seconds from 1970-01-01T00:00:00Z, negative before it, plus
    /// the sub-second part in nanoseconds.
    ///
    /// `nanos` is always in `0..1_000_000_000` and never negative, including
    /// for instants before 1970, so the pair reads as one number without the
    /// caller having to know which way the sign of the fraction goes: -1500
    /// milliseconds is (-2 seconds, 500,000,000 nanoseconds).
    At { unix_seconds: i64, nanos: u32 },
    /// An instant inside a leap second: `nanos` into second 60 of the minute
    /// whose second 59 is `unix_seconds`. The end of 2016 had one, and a CDF
    /// written at the time holds `2016-12-31T23:59:60.5` as a number like any
    /// other.
    ///
    /// Its own answer rather than [`Moment::At`] with a flag on it, because a
    /// Unix count has no number for this second, and an interface that printed
    /// the seconds it was given and ignored a flag would print `23:59:59` for
    /// a moment a second later. As a variant of its own, every interface has
    /// to decide what to print.
    LeapSecond { unix_seconds: i64, nanos: u32 },
    /// The stored value is the one this format writes when it has no time to
    /// record. See [`Time::unset`].
    Unset,
    /// The number does not name a moment. Either the instant falls outside
    /// [`FIRST_SECOND`]..=[`LAST_SECOND`], or a packed field holds something
    /// that is not a date.
    ///
    /// One answer for both, because an interface can do nothing different about
    /// them: either way the honest thing to show is that these bytes do not
    /// hold a date, and the bytes themselves are on the row above.
    Impossible,
}

/// The first instant this will name: 0001-01-01T00:00:00Z.
///
/// The band is the years a four-digit year covers, and it is deliberately
/// narrower than what a date library will take. A sixty-four bit count of
/// hundred-nanosecond ticks reaches the year sixty thousand, and a corrupt file
/// full of one repeated byte reaches it easily; every date library here would
/// render that, in an expanded notation, as a confident answer. It is not one.
/// Outside this band a number is telling the reader that the field was misread
/// or the file is damaged, not what year it is, and [`Moment::Impossible`] says
/// so. It also keeps every answer inside what a browser's `Date` holds with
/// room to spare, so an interface never has to render one it cannot.
pub const FIRST_SECOND: i64 = -62_135_596_800;
/// The last instant this will name: 9999-12-31T23:59:59Z. See [`FIRST_SECOND`].
pub const LAST_SECOND: i64 = 253_402_300_799;

const NANOS_PER_SECOND: i128 = 1_000_000_000;

/// The number in a field, as the kind of number it is.
///
/// Kept apart rather than widened to one type, because each kind has a way of
/// going wrong the other does not. An integer count multiplied out is exact and
/// only has to be checked for overflow. A float count is a double counting
/// milliseconds, which is a CDF_EPOCH, and the same multiplication done in
/// floating point moves a whole second by thousands of nanoseconds; it is also
/// the only kind that can be NaN. And a sentinel is compared against its own
/// kind only: 0.0 in a double is not the 0 a gzip writes.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Count {
    Int(i128),
    Float(f64),
}

impl Count {
    /// Whether this is one of the values the format writes when it has no time
    /// to record.
    fn is_unset(self, unset: &[Unset]) -> bool {
        unset.iter().any(|u| match (u, self) {
            (Unset::Int(want), Count::Int(v)) => *want == v,
            (Unset::Float(want), Count::Float(v)) => *want == v,
            _ => false,
        })
    }
}

impl Evaluator {
    /// The moment the field at `path` means, or nothing when it means none.
    ///
    /// Nothing, rather than an error, for every way this can fail to apply: the
    /// template has not said the field is a time, the value is not a number, or
    /// the other half of an MS-DOS pair is not there to be found. A number that
    /// *is* a time and does not come to a date is [`Moment::Impossible`], which
    /// is a different answer: the reader is looking at a field the format meant
    /// as a timestamp, and saying nothing there would leave them to work out
    /// why the row went away.
    ///
    /// Reads the field's own bytes, and for an MS-DOS pair its partner's, and
    /// nothing else. Bytes still on their way are `Pending` as everywhere else.
    pub fn time_of<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Option<TimeInfo>> {
        let Some(time) = self.time_at(doc, path)? else { return Ok(None) };
        let Some(value) = self.moment_number(doc, path)? else { return Ok(None) };
        let step_nanos = step_of(&time.epoch);
        if value.is_unset(&time.unset) {
            return Ok(Some(TimeInfo { moment: Moment::Unset, zone: time.zone, step_nanos, note: None }));
        }
        let moment = match &time.epoch {
            Epoch::Counted(c) => counted(value, *c),
            Epoch::Atomic(a) => {
                let (moment, note) = atomic(value, *a);
                return Ok(Some(TimeInfo { moment, zone: time.zone, step_nanos, note }));
            }
            Epoch::Dos => {
                // The date is the top half and the time the bottom, and a value
                // wider than the thirty-two bits this is packed into is not one
                // of these at all rather than one to be masked down to size.
                // Nor is a float, which no format packs a date into.
                match value {
                    Count::Int(v) => match u32::try_from(v) {
                        Ok(v) => dos(v >> 16, v & 0xffff),
                        Err(_) => Moment::Impossible,
                    },
                    Count::Float(_) => Moment::Impossible,
                }
            }
            Epoch::DosHalves { date, time } => {
                let (Some(d), Some(t)) = (self.half(doc, path, date)?, self.half(doc, path, time)?) else {
                    // The pair is what denotes the moment, and half of one
                    // denotes a day with no time in it. A template naming a
                    // field that is not there is caught by `times_declared`
                    // when the tests run; here it is simply not answerable.
                    return Ok(None);
                };
                dos(d, t)
            }
        };
        Ok(Some(TimeInfo { moment, zone: time.zone, step_nanos, note: None }))
    }

    /// What the template says about the field at `path`, if anything.
    ///
    /// Two places to look, and the second is what a list needs. A field of a
    /// structure carries its own declaration. An *element* of a list does not:
    /// an array has no `Field` between it and its elements, so the declaration
    /// sits on the field holding the list and applies to each element. A
    /// region file's `timestamps` is a thousand and twenty-four of them and is
    /// the reason this exists.
    ///
    /// The list itself is never a moment, which falls out of
    /// [`Evaluator::moment_number`]: a composite reads as its count, and a
    /// count is not a number this will accept.
    ///
    /// And a list of lists is the same thing again, so the walk climbs through
    /// as many lists as there are to the first structure. A CDF variable's
    /// values are rows of values, one row per record, and the declaration is
    /// on the field holding the rows: the second level of list is no more a
    /// place to hang one than the first was. The walk stops at a structure
    /// whether or not its field is declared, so a list of structures does not
    /// lend its declaration to the fields inside them.
    fn time_at<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Option<Time>> {
        self.resolve(doc, path)?;
        let Some((mut idx, mut parent)) = path.split_last().map(|(i, p)| (*i, p)) else { return Ok(None) };
        loop {
            match self.memo.get(parent).map(|r| &r.ty) {
                Some(Ty::Struct(s)) => return Ok(s.fields.get(idx).and_then(|f| f.time.clone())),
                Some(Ty::Array { .. } | Ty::Repeat { .. } | Ty::Chain { .. } | Ty::Gather { .. } | Ty::PointerList { .. }) => {
                    let Some((&up, rest)) = parent.split_last() else { return Ok(None) };
                    (idx, parent) = (up, rest);
                }
                _ => return Ok(None),
            }
        }
    }

    /// The number in the field, when it is one.
    ///
    /// Narrower than [`Value::as_int`] on purpose. That one answers for a short
    /// text or byte field as well, reading its bytes as a big-endian number so
    /// that a switch can key on `"IHDR"`, and it answers for a composite with
    /// how many children it has. Neither is a moment, and a timestamp
    /// mistakenly declared on either would show a date worked out from four
    /// letters. A number written as digits *is* accepted, since that is a
    /// number the format wrote in text: an `ar` member's `mtime` is twelve
    /// bytes of ASCII decimal and a cpio header's is eight of ASCII hex, and
    /// both read as [`Value::Int`] before they get here.
    ///
    /// A float is accepted as the count it is, fraction and all: a CDF_EPOCH
    /// is milliseconds in a double. See [`Counted`] for how it is multiplied
    /// out without losing the second it lands on.
    fn moment_number<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Option<Count>> {
        fn number(v: &Value) -> Option<Count> {
            match v {
                Value::UInt(v) => i128::try_from(*v).ok().map(Count::Int),
                Value::Int(v) => Some(Count::Int(*v)),
                Value::Float(v) => Some(Count::Float(*v)),
                _ => None,
            }
        }
        Ok(match &self.node(doc, path)?.value {
            Value::Enum { raw, .. } => Some(Count::Int(*raw)),
            // A slot the format leaves empty is still the number written there,
            // and a format that has both a sentinel of its own and a declared
            // unset value should have them agree.
            Value::Unset(inner) => number(inner),
            other => number(other),
        })
    }

    /// One half of an MS-DOS pair, read as the sixteen bits it is.
    ///
    /// Resolved outwards from the field asking, exactly as a check's
    /// [`Named::Here`](crate::template::Named::Here) is: the partner is a
    /// sibling in every format that writes one, and outwards is what already
    /// works and is already tested.
    fn half<S: Source>(&mut self, doc: &Document<S>, path: &[usize], name: &str) -> R<Option<u32>> {
        let Some(p) = self.field_out_from(doc, path, name)? else { return Ok(None) };
        let Some(Count::Int(v)) = self.moment_number(doc, &p)? else { return Ok(None) };
        Ok(u32::try_from(v).ok().filter(|v| *v <= 0xffff))
    }
}

/// How precise a field of this epoch is, in nanoseconds. An MS-DOS time counts
/// seconds in twos, and a reader who has been shown an odd second has been
/// shown a digit the file cannot hold.
fn step_of(epoch: &Epoch) -> u64 {
    match epoch {
        Epoch::Counted(c) => c.step_nanos,
        Epoch::Atomic(a) => a.step_nanos,
        Epoch::Dos | Epoch::DosHalves { .. } => 2_000_000_000,
    }
}

/// A count of steps from an epoch, resolved to an instant.
///
/// All of it in `i128`, and every multiplication checked. The widest thing that
/// arrives here is a sixty-four bit tick count, and a hundred-nanosecond step
/// takes it nowhere near the edge of an `i128`; a value that came from a text
/// field can be wider, so the multiplication is checked rather than assumed and
/// an overflow is the same answer an out-of-range instant gets. Never a panic,
/// and never a wrap.
fn counted(value: Count, c: Counted) -> Moment {
    let Some(steps) = nanos_of(value, c.step_nanos) else { return Moment::Impossible };
    let Some(nanos) = (c.zero as i128).checked_mul(NANOS_PER_SECOND).and_then(|z| steps.checked_add(z)) else {
        return Moment::Impossible;
    };
    instant(nanos)
}

/// How many nanoseconds a count of `step_nanos`-long steps comes to, or
/// nothing when that is not a number at all.
///
/// A float is split before it is multiplied. The whole part of a double is
/// exact, and so is that part times the step once it is an integer; the
/// fraction is less than one step and is the only thing rounded, to the
/// nearest nanosecond, since a double counting milliseconds is only ever an
/// approximation of the decimal fraction the writer meant. Multiplying the
/// whole double out first is what goes wrong: `62545910400000.0` milliseconds
/// is 6.25e19 nanoseconds, where adjacent doubles are eight thousand apart,
/// and a count on the second prints as one a few microseconds short of it.
///
/// NaN and the infinities are no count. So is a double too large to be an
/// integer an `i128` holds, which is also far outside the years this names;
/// CDF's fill value of -1.0E31 is one, and is caught as a sentinel before it
/// gets here when the template declares it.
fn nanos_of(value: Count, step_nanos: u64) -> Option<i128> {
    match value {
        Count::Int(v) => v.checked_mul(step_nanos as i128),
        Count::Float(v) => {
            if !v.is_finite() || v.abs() >= 1e30 {
                return None;
            }
            let whole = v.trunc();
            let part = ((v - whole) * step_nanos as f64).round() as i128;
            (whole as i128).checked_mul(step_nanos as i128)?.checked_add(part)
        }
    }
}

/// A count on a clock with leap seconds, resolved to the UTC moment it is and
/// whatever has to be said about it. See [`Atomic`] and [`leap_seconds`].
///
/// The count is laid on TAI first, which is plain arithmetic from its zero,
/// and only then taken to UTC, which is the table. A count so far out that the
/// table has no day for it is outside the years this names in any case, and
/// the TAI instant is held to within a day of those years before it is asked,
/// so the question is never about the year thirty thousand.
fn atomic(value: Count, a: Atomic) -> (Moment, Option<TimeNote>) {
    const SLACK: i128 = 86_400 * NANOS_PER_SECOND;
    let Some(tai) = nanos_of(value, a.step_nanos).and_then(|n| n.checked_add(a.zero_tai_nanos)) else {
        return (Moment::Impossible, None);
    };
    let band = FIRST_SECOND as i128 * NANOS_PER_SECOND - SLACK..=LAST_SECOND as i128 * NANOS_PER_SECOND + SLACK;
    if !band.contains(&tai) {
        return (Moment::Impossible, None);
    }
    let (moment, second) = match leap_seconds::utc_of(tai) {
        Some(leap_seconds::Utc::At(utc)) => (instant(utc), utc.div_euclid(NANOS_PER_SECOND)),
        Some(leap_seconds::Utc::Inserted { second_59, nanos }) => {
            let inside = (FIRST_SECOND..LAST_SECOND).contains(&second_59);
            let moment = if inside { Moment::LeapSecond { unix_seconds: second_59, nanos } } else { Moment::Impossible };
            (moment, second_59 as i128)
        }
        None => (Moment::Impossible, 0),
    };
    let note = match moment {
        Moment::At { .. } | Moment::LeapSecond { .. } if second >= leap_seconds::EXPIRES as i128 => {
            Some(TimeNote::PastLeapSecondTable)
        }
        Moment::At { .. } | Moment::LeapSecond { .. } if second < leap_seconds::FIRST_LEAP_SECOND_ERA as i128 => {
            Some(TimeNote::BeforeLeapSeconds)
        }
        _ => None,
    };
    (moment, note)
}

/// Nanoseconds from 1970-01-01T00:00:00Z as the instant they are, or
/// [`Moment::Impossible`] outside the years this names.
fn instant(nanos: i128) -> Moment {
    // Euclidean, so the fraction of an instant before 1970 is still a fraction
    // forwards: -1500 milliseconds is two seconds back and half of one on, not
    // one second back and half of one further back, which would print as a
    // negative fraction on a positive clock.
    let seconds = nanos.div_euclid(NANOS_PER_SECOND);
    let rest = nanos.rem_euclid(NANOS_PER_SECOND) as u32;
    match i64::try_from(seconds) {
        Ok(s) if (FIRST_SECOND..=LAST_SECOND).contains(&s) => Moment::At { unix_seconds: s, nanos: rest },
        _ => Moment::Impossible,
    }
}

/// MS-DOS's packed date and time, as two sixteen-bit halves.
///
/// Years from 1980 in seven bits, so the format runs out in 2107 and nothing
/// here can leave the band. What can fail is the calendar: a writer with no
/// date to record writes zeros, which is a month of nought, and a damaged file
/// can hold a thirteenth month or a thirtieth of February. Every one of those
/// is [`Moment::Impossible`]. Checking the day against the length of its own
/// month matters and is not fussiness: 30 February passes a `day <= 31` test
/// and then converts, quietly, to the second of March, which is the wrapped
/// date this whole file exists to refuse.
fn dos(date: u32, time: u32) -> Moment {
    let year = 1980 + ((date >> 9) & 0x7f) as i64;
    let month = (date >> 5) & 0x0f;
    let day = date & 0x1f;
    let hour = (time >> 11) & 0x1f;
    let minute = (time >> 5) & 0x3f;
    // Seconds are stored in twos, so five bits reach 62 and the top of the
    // range is not a second of any minute.
    let second = (time & 0x1f) * 2;
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) || hour > 23 || minute > 59 || second > 59
    {
        return Moment::Impossible;
    }
    let days = days_from_civil(year, month, day);
    let seconds = days * 86_400 + (hour as i64) * 3_600 + (minute as i64) * 60 + second as i64;
    Moment::At { unix_seconds: seconds, nanos: 0 }
}

/// Days from 1970-01-01 to a civil date, by Howard Hinnant's algorithm, which
/// is exact over every year this will name and has no table in it.
///
/// A date library would do this and nothing else here needs one: everything but
/// MS-DOS's packed pair arrives as a count and stays arithmetic. Pulling a
/// calendar crate, and the timezone database most of them carry, into a wasm
/// bundle for one format's sixteen bits is not the trade, and there is no zone
/// arithmetic here to want one for: an instant is a number of seconds, and the
/// only formatting anywhere is the interface's.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = month as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::{Moment, TimeInfo, TimeNote};
    use crate::document::Document;
    use crate::eval::Evaluator;
    use crate::source::MemSource;
    use crate::template::{Endian::Big, Endian::Little, Expr as E, StrLen, Template, Time, Ty as T, Zone};

    /// One field of a made-up format, declared as a time and read from the
    /// bytes given. Every test here is over a known instant worked out by hand,
    /// so what is being checked is the arithmetic rather than this file
    /// agreeing with itself.
    fn one(ty: T, time: Time, bytes: Vec<u8>) -> Option<TimeInfo> {
        let root = T::structure("Made", vec![("when", ty)]).field_time("when", time);
        let mut ev = Evaluator::new(Template::new("made-up", root));
        let doc = Document::new(MemSource(bytes));
        ev.time_of(&doc, &[0]).unwrap()
    }

    fn at(info: Option<TimeInfo>) -> (i64, u32) {
        match info.expect("this field is declared as a time").moment {
            Moment::At { unix_seconds, nanos } => (unix_seconds, nanos),
            other => panic!("expected an instant, got {other:?}"),
        }
    }

    fn moment(info: Option<TimeInfo>) -> Moment {
        info.expect("this field is declared as a time").moment
    }

    /// A signed sixty-four bit count. There is no `Ty::i64` builder, since no
    /// format here has wanted one; the variant takes any width.
    fn i64le() -> T {
        T::Int { bits: 64, endian: Little }
    }

    /// 2009-02-13T23:31:30Z. The instant every counted epoch below is checked
    /// against, so that four different arithmetics are shown landing on one
    /// second.
    const KNOWN: i64 = 1_234_567_890;

    #[test]
    fn unix_seconds() {
        let info = one(T::u32(Little), Time::unix(), (KNOWN as u32).to_le_bytes().to_vec());
        assert_eq!(at(info), (KNOWN, 0));
        assert_eq!(info.unwrap().zone, Zone::Utc);
        assert_eq!(info.unwrap().step_nanos, 1_000_000_000);
    }

    #[test]
    fn unix_microseconds_keep_their_fraction() {
        // The same second and a quarter of it, which is what a journal writes.
        let v = KNOWN as u64 * 1_000_000 + 250_000;
        let info = one(T::u64(Little), Time::unix_micros(), v.to_le_bytes().to_vec());
        assert_eq!(at(info), (KNOWN, 250_000_000));
        assert_eq!(info.unwrap().step_nanos, 1_000, "an interface prints six decimal places from this");
    }

    #[test]
    fn unix_milliseconds() {
        let v = KNOWN as u64 * 1_000 + 500;
        assert_eq!(at(one(T::u64(Little), Time::unix_millis(), v.to_le_bytes().to_vec())), (KNOWN, 500_000_000));
    }

    /// The Mac epoch is 1904, so the same instant is a larger number by the
    /// sixty-six years between them.
    #[test]
    fn mac_1904() {
        let v = (KNOWN + 2_082_844_800) as u32;
        assert_eq!(at(one(T::u32(Big), Time::mac(), v.to_be_bytes().to_vec())), (KNOWN, 0));
    }

    /// A FILETIME counts hundred-nanosecond ticks from 1601, so the same
    /// instant is a number ten million times larger again.
    #[test]
    fn windows_filetime() {
        let v = (KNOWN + 11_644_473_600) as u64 * 10_000_000 + 1_234_567;
        let info = one(T::u64(Little), Time::filetime(), v.to_le_bytes().to_vec());
        assert_eq!(at(info), (KNOWN, 123_456_700));
        assert_eq!(info.unwrap().step_nanos, 100);
    }

    /// The MS-DOS pair packed into one word, as a RAR 4 file block writes it:
    /// 1994-03-02 14:20:00, whose every field is something other than zero.
    #[test]
    fn dos_packed_into_one_field() {
        let date: u32 = ((1994 - 1980) << 9) | (3 << 5) | 2;
        let time: u32 = (14 << 11) | (20 << 5);
        let v: u32 = (date << 16) | time;
        let info = one(T::u32(Little), Time::dos(), v.to_le_bytes().to_vec());
        assert_eq!(at(info), (762_618_000, 0));
        assert_eq!(info.unwrap().zone, Zone::Local, "MS-DOS had no zone to record");
        assert_eq!(info.unwrap().step_nanos, 2_000_000_000, "seconds are stored in twos");
    }

    /// The same pair as two fields, which is what a ZIP and a cabinet write.
    /// Both halves answer the whole moment.
    #[test]
    fn dos_halves_answer_from_either_field() {
        let date: u16 = ((1994 - 1980) << 9) | (3 << 5) | 2;
        let time: u16 = (14 << 11) | (20 << 5);
        let root = T::structure("Made", vec![("time", T::u16(Little)), ("date", T::u16(Little))])
            .field_times(&["time", "date"], Time::dos_halves("date", "time"));
        let mut ev = Evaluator::new(Template::new("made-up", root));
        let mut bytes = time.to_le_bytes().to_vec();
        bytes.extend_from_slice(&date.to_le_bytes());
        let doc = Document::new(MemSource(bytes));
        for field in [0, 1] {
            assert_eq!(at(ev.time_of(&doc, &[field]).unwrap()), (762_618_000, 0), "field {field}");
        }
    }

    /// A packed date of zero is a month of nought, which is what a writer with
    /// no date to record puts there. It is not the first of January 1980.
    #[test]
    fn a_dos_date_of_zero_is_not_a_date() {
        assert_eq!(moment(one(T::u32(Little), Time::dos(), vec![0, 0, 0, 0])), Moment::Impossible);
    }

    /// The one a `day <= 31` test lets through. Converted, it would come out as
    /// the second of March, a date the file does not hold.
    #[test]
    fn the_thirtieth_of_february_is_refused() {
        let date: u32 = ((1994 - 1980) << 9) | (2 << 5) | 30;
        let v: u32 = (date << 16) | (12 << 11);
        assert_eq!(moment(one(T::u32(Little), Time::dos(), v.to_le_bytes().to_vec())), Moment::Impossible);
    }

    /// The 29th of February exists in 1996 and not in 1994, and the day is
    /// checked against the length of its own month rather than against 31.
    #[test]
    fn a_leap_day_is_kept_in_a_leap_year_and_refused_outside_one() {
        let packed = |year: u32| -> Vec<u8> {
            let date: u32 = ((year - 1980) << 9) | (2 << 5) | 29;
            ((date << 16) | (12u32 << 11)).to_le_bytes().to_vec()
        };
        assert!(matches!(moment(one(T::u32(Little), Time::dos(), packed(1996))), Moment::At { .. }));
        assert_eq!(moment(one(T::u32(Little), Time::dos(), packed(1994))), Moment::Impossible);
    }

    /// gzip writes zero when the compressor had no time to put there, and a
    /// reader shown 1970-01-01 has been told something the file does not say.
    #[test]
    fn a_declared_unset_value_is_not_the_epoch() {
        assert_eq!(moment(one(T::u32(Little), Time::unix().unset(0), vec![0, 0, 0, 0])), Moment::Unset);
        // The same zero without the declaration is the epoch, which is what a
        // format that means it should get.
        assert_eq!(
            moment(one(T::u32(Little), Time::unix(), vec![0, 0, 0, 0])),
            Moment::At { unix_seconds: 0, nanos: 0 }
        );
    }

    /// Some epochs allow times before them, and a signed field is how a format
    /// says so. utmp's `tv_sec` is an `i32`.
    #[test]
    fn a_negative_count_is_a_time_before_the_epoch() {
        let v: i32 = -1_000_000_000;
        assert_eq!(at(one(T::i32(Little), Time::unix(), v.to_le_bytes().to_vec())), (-1_000_000_000, 0));
    }

    /// And the fraction of one still runs forwards: -1500 milliseconds is two
    /// seconds back and half of one on, never a negative fraction on a clock.
    #[test]
    fn a_negative_count_with_a_fraction_keeps_the_fraction_positive() {
        let v: i64 = -1_500;
        assert_eq!(at(one(i64le(), Time::unix_millis(), v.to_le_bytes().to_vec())), (-2, 500_000_000));
    }

    /// A sixty-four bit tick count reaches the year sixty thousand, which is
    /// what a corrupt file full of one repeated byte looks like. It is not a
    /// date and must not be rendered as one.
    #[test]
    fn a_count_past_the_years_this_names_is_refused() {
        assert_eq!(moment(one(T::u64(Little), Time::filetime(), u64::MAX.to_le_bytes().to_vec())), Moment::Impossible);
        // And the far side: an instant before the year 1.
        let v: i64 = super::FIRST_SECOND - 1;
        assert_eq!(moment(one(i64le(), Time::unix(), v.to_le_bytes().to_vec())), Moment::Impossible);
        // While the two ends of the band themselves are answered.
        for edge in [super::FIRST_SECOND, super::LAST_SECOND] {
            assert_eq!(at(one(i64le(), Time::unix(), edge.to_le_bytes().to_vec())), (edge, 0));
        }
    }

    /// The multiplication that gets there is checked rather than assumed, so a
    /// number no fixed-width field could hold is an answer and not a panic. A
    /// number written as digits is the one thing here not bounded by a width.
    #[test]
    fn a_count_that_overflows_the_arithmetic_is_refused() {
        let digits = "9".repeat(38);
        let root = T::structure("Made", vec![("when", T::decimal(StrLen::Fixed(E::lit(38))))])
            .field_time("when", Time::unix_nanos());
        let mut ev = Evaluator::new(Template::new("made-up", root));
        let doc = Document::new(MemSource(digits.into_bytes()));
        assert_eq!(moment(ev.time_of(&doc, &[0]).unwrap()), Moment::Impossible);
    }

    /// A declaration on a list is about its elements. The list itself denotes
    /// nothing: a composite reads as how many children it has, and a count of
    /// a thousand and twenty-four is not a moment.
    #[test]
    fn a_declaration_on_a_list_reaches_its_elements_and_not_the_list() {
        let root =
            T::structure("Made", vec![("stamps", T::array(T::u32(Big), E::lit(2)))]).field_time("stamps", Time::unix());
        let mut ev = Evaluator::new(Template::new("made-up", root));
        let mut bytes = (KNOWN as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(&((KNOWN as u32) + 60).to_be_bytes());
        let doc = Document::new(MemSource(bytes));
        assert_eq!(at(ev.time_of(&doc, &[0, 0]).unwrap()), (KNOWN, 0));
        assert_eq!(at(ev.time_of(&doc, &[0, 1]).unwrap()), (KNOWN + 60, 0));
        assert!(ev.time_of(&doc, &[0]).unwrap().is_none(), "the array is not itself a moment");
    }

    /// A number a format wrote as text is still a number. An `ar` member's
    /// `mtime` is twelve bytes of ASCII decimal.
    #[test]
    fn a_time_written_as_digits_reads_as_one() {
        let root = T::structure("Made", vec![("when", T::decimal(StrLen::Padded { size: E::lit(12), pad: b' ' }))])
            .field_time("when", Time::unix());
        let mut ev = Evaluator::new(Template::new("made-up", root));
        let doc = Document::new(MemSource(b"1234567890  ".to_vec()));
        assert_eq!(at(ev.time_of(&doc, &[0]).unwrap()), (KNOWN, 0));
    }

    fn f64le(v: f64) -> Vec<u8> {
        v.to_le_bytes().to_vec()
    }

    /// A CDF_EPOCH, milliseconds from the year 0 in a double. The number is
    /// `cacsst2.cdf`'s one time, which `cdflib` encodes as
    /// `1982-01-01T00:00:00.000`. A whole second, and multiplied out to
    /// nanoseconds in floating point it would not be one.
    #[test]
    fn a_float_count_on_the_second_stays_on_it() {
        let info = one(T::F64(Little), Time::cdf_epoch(), f64le(62545910400000.0));
        assert_eq!(at(info), (378_691_200, 0));
        assert_eq!(info.unwrap().step_nanos, 1_000_000, "printed to the millisecond, as CDF's own tools do");
    }

    /// And a count with a fraction keeps it: half a millisecond past
    /// 2020-01-01T23:59:59.123, which `cdflib` encodes to the millisecond as
    /// `2020-01-01T23:59:59.123`.
    #[test]
    fn a_float_count_keeps_its_fraction() {
        let info = one(T::F64(Little), Time::cdf_epoch(), f64le(63745142399123.5));
        assert_eq!(at(info), (1_577_923_199, 123_500_000));
    }

    /// The fraction runs forwards before the epoch too, the same as an
    /// integer count's does.
    #[test]
    fn a_negative_float_count_keeps_its_fraction_positive() {
        let info = one(T::F64(Little), Time::unix_millis(), f64le(-1500.25));
        assert_eq!(at(info), (-2, 499_750_000));
    }

    /// NaN is what a float field holds when nobody measured anything, and it
    /// is not a count of anything.
    #[test]
    fn a_float_that_is_not_a_number_is_not_a_date() {
        for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(moment(one(T::F64(Little), Time::cdf_epoch(), f64le(v))), Moment::Impossible, "{v}");
        }
    }

    /// CDF's two values for no time. -1.0E31 is the fill value, and without the
    /// declaration it is still not a date, since it is far outside the years
    /// this names; 0.0 is the pad value, and is the first instant of the year
    /// 0, which is outside them too. Both are declared, so both say what they
    /// mean rather than only what they are not.
    #[test]
    fn a_cdf_epochs_fill_and_pad_values_are_no_time() {
        for v in [-1.0e31, 0.0] {
            assert_eq!(moment(one(T::F64(Little), Time::cdf_epoch(), f64le(v))), Moment::Unset, "{v}");
        }
        assert_eq!(moment(one(T::F64(Little), Time::unix_millis(), f64le(-1.0e31))), Moment::Impossible);
    }

    /// The ends of a CDF_EPOCH. A day past the pad value is 0000-01-02, which
    /// `cdflib` encodes and this does not name, since the year 0 is not one of
    /// the years it will; 9999-12-31T23:59:59.999 is the last millisecond it
    /// will, and one more is past it.
    #[test]
    fn a_cdf_epoch_at_the_year_0_and_the_year_9999() {
        assert_eq!(moment(one(T::F64(Little), Time::cdf_epoch(), f64le(86_400_000.0))), Moment::Impossible);
        let last = one(T::F64(Little), Time::cdf_epoch(), f64le(315_569_519_999_999.0));
        assert_eq!(at(last), (super::LAST_SECOND, 999_000_000));
        assert_eq!(moment(one(T::F64(Little), Time::cdf_epoch(), f64le(315_569_520_000_000.0))), Moment::Impossible);
        // And the first instant of the year 1, which is.
        let first = one(T::F64(Little), Time::cdf_epoch(), f64le(31_622_400_000.0));
        assert_eq!(at(first), (super::FIRST_SECOND, 0));
    }

    /// A sentinel is compared against a count of its own kind. A gzip's 0 is an
    /// integer, and a float field holding 0.0 is not it.
    #[test]
    fn an_integer_sentinel_does_not_match_a_float() {
        let info = one(T::F64(Little), Time::unix().unset(0), f64le(0.0));
        assert_eq!(moment(info), Moment::At { unix_seconds: 0, nanos: 0 });
    }

    /// A CDF variable's values are rows of values, one row per record, and the
    /// declaration on the rows reaches every value in them. The rows are not
    /// moments, any more than a list of them is.
    #[test]
    fn a_declaration_on_a_list_of_lists_reaches_the_numbers_at_the_bottom() {
        let rows = T::array(T::array(T::F64(Big), E::lit(2)), E::lit(2));
        let root = T::structure("Made", vec![("rows", rows)]).field_time("rows", Time::cdf_epoch());
        let mut ev = Evaluator::new(Template::new("made-up", root));
        let mut bytes = Vec::new();
        for ms in [62545910400000.0f64, 62545910401000.0, 62545910402000.0, -1.0e31] {
            bytes.extend(ms.to_be_bytes());
        }
        let doc = Document::new(MemSource(bytes));
        assert_eq!(at(ev.time_of(&doc, &[0, 0, 1]).unwrap()), (378_691_201, 0));
        assert_eq!(at(ev.time_of(&doc, &[0, 1, 0]).unwrap()), (378_691_202, 0));
        assert_eq!(moment(ev.time_of(&doc, &[0, 1, 1]).unwrap()), Moment::Unset);
        assert!(ev.time_of(&doc, &[0, 1]).unwrap().is_none(), "a row is not itself a moment");
        assert!(ev.time_of(&doc, &[0]).unwrap().is_none(), "nor are the rows");
    }

    /// A TT2000, an `int8` of nanoseconds, read through the table.
    fn tt2000(n: i64) -> TimeInfo {
        one(i64le(), Time::tt2000(), n.to_le_bytes().to_vec()).expect("declared as a time")
    }

    /// Seconds from 1970 for a UTC date and time of day, by the same
    /// arithmetic the core uses, so that a test reads as the date `cdflib`
    /// printed rather than as a number to check by hand.
    fn utc(year: i64, month: u32, day: u32, hour: i64, minute: i64, second: i64) -> i64 {
        super::days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second
    }

    /// J2000 itself. A TT2000 of nought is noon on 2000-01-01 in Terrestrial
    /// Time, which is 32.184 seconds ahead of TAI, which was 32 seconds ahead of
    /// UTC then: `cdflib` gives `2000-01-01T11:58:55.816000000`. And noon in
    /// UTC is 64.184 seconds further on.
    #[test]
    fn a_tt2000_of_nought_is_j2000() {
        let zero = tt2000(0);
        assert_eq!(zero.moment, Moment::At { unix_seconds: utc(2000, 1, 1, 11, 58, 55), nanos: 816_000_000 });
        assert_eq!(zero.step_nanos, 1, "printed to the nanosecond");
        assert_eq!(zero.note, None);
        assert_eq!(tt2000(64_184_000_000).moment, Moment::At { unix_seconds: utc(2000, 1, 1, 12, 0, 0), nanos: 0 });
    }

    /// The Parker Solar Probe file's first and last times, which `cdflib`
    /// encodes as `2020-01-04T02:33:30.000000000` and `...19:33:30...`. Five
    /// leap seconds after J2000, so a plain count from the right zero would be
    /// five seconds out.
    #[test]
    fn a_tt2000_from_the_parker_solar_probe() {
        assert_eq!(tt2000(631_377_279_184_000_000).moment, Moment::At { unix_seconds: utc(2020, 1, 4, 2, 33, 30), nanos: 0 });
        assert_eq!(tt2000(631_438_479_184_000_000).moment, Moment::At { unix_seconds: utc(2020, 1, 4, 19, 33, 30), nanos: 0 });
    }

    /// The leap second at the end of 2016, and the half seconds either side of
    /// it. The numbers are `cdflib`'s `compute_tt2000` for
    /// `2016-12-31 23:59:59.5`, `23:59:60.0`, `23:59:60.5` and
    /// `2017-01-01 00:00:00`. `cdflib`'s own `encode` prints the middle two as
    /// `23:60:00.000000000` and `23:60:00.500000000`, carrying the sixtieth
    /// second into the minute; the NASA library and ISO 8601 both write
    /// `23:59:60`, which is what this answers.
    #[test]
    fn a_tt2000_inside_the_leap_second_at_the_end_of_2016() {
        let last = utc(2016, 12, 31, 23, 59, 59);
        assert_eq!(tt2000(536_500_867_684_000_000).moment, Moment::At { unix_seconds: last, nanos: 500_000_000 });
        assert_eq!(tt2000(536_500_868_184_000_000).moment, Moment::LeapSecond { unix_seconds: last, nanos: 0 });
        assert_eq!(tt2000(536_500_868_684_000_000).moment, Moment::LeapSecond { unix_seconds: last, nanos: 500_000_000 });
        assert_eq!(tt2000(536_500_868_684_000_000 + 499_999_999).moment, Moment::LeapSecond { unix_seconds: last, nanos: 999_999_999 });
        assert_eq!(tt2000(536_500_869_184_000_000).moment, Moment::At { unix_seconds: last + 1, nanos: 0 });
    }

    /// 1972-01-01, the first instant of whole leap seconds, and the second
    /// before it, which is the rubber era: the CDF library's offset for the
    /// last day of 1971 is 9.890946 seconds, and 0.109054 of a second more was
    /// inserted at midnight to make it ten. `cdflib`'s numbers.
    #[test]
    fn a_tt2000_either_side_of_1972() {
        let midnight = utc(1972, 1, 1, 0, 0, 0);
        let first = tt2000(-883_655_957_816_000_000);
        assert_eq!(first.moment, Moment::At { unix_seconds: midnight, nanos: 0 });
        assert_eq!(first.note, None);
        assert_eq!(tt2000(-883_655_956_816_000_000).moment, Moment::At { unix_seconds: midnight + 1, nanos: 0 });
        let before = tt2000(-883_655_958_925_054_000);
        assert_eq!(before.moment, Moment::At { unix_seconds: midnight - 1, nanos: 0 });
        assert_eq!(before.note, Some(TimeNote::BeforeLeapSeconds));
        // The tenth of a second inserted at midnight is not a sixty-first
        // second, which UTC never wrote for it: the CDF library reads it with
        // 1972's offset, as the last tenth of 1971 over again, and `cdflib`
        // gives `1971-12-31T23:59:59.940946000` for this one.
        assert_eq!(
            tt2000(-883_655_958_925_054_000 + 1_050_000_000).moment,
            Moment::At { unix_seconds: midnight - 1, nanos: 940_946_000 }
        );
    }

    /// The first leap second of all, at the end of June 1972, is a sixty-first
    /// second like every one since. `cdflib` does not agree here and only
    /// here: it reads the first half of 1972 by its pre-1972 arithmetic, so
    /// its own `compute_tt2000` for `1972-06-30 23:59:60.5` encodes back as
    /// `23:59:59.500000000`, while the same for the end of 1972 comes back as
    /// the sixtieth second.
    #[test]
    fn the_first_leap_second_is_a_sixty_first_second() {
        let last = utc(1972, 6, 30, 23, 59, 59);
        assert_eq!(tt2000(-867_931_157_316_000_000).moment, Moment::LeapSecond { unix_seconds: last, nanos: 500_000_000 });
        assert_eq!(tt2000(-867_931_158_316_000_000).moment, Moment::At { unix_seconds: last, nanos: 500_000_000 });
        assert_eq!(tt2000(-867_931_156_816_000_000).moment, Moment::At { unix_seconds: last + 1, nanos: 0 });
    }

    /// Dates before 1972, where the offset drifts by the day. `cdflib` gives
    /// `1965-06-01T12:34:56.789000000` and `1962-03-04T05:06:07.000000000` for
    /// these, and before 1960 it takes the offset as nought, as the CDF library
    /// does: `1955-01-01T00:00:00.000000000`.
    #[test]
    fn a_tt2000_before_1972() {
        let mid_sixties = tt2000(-1_091_402_667_190_526_000);
        assert_eq!(mid_sixties.moment, Moment::At { unix_seconds: utc(1965, 6, 1, 12, 34, 56), nanos: 789_000_000 });
        assert_eq!(mid_sixties.note, Some(TimeNote::BeforeLeapSeconds));
        assert_eq!(tt2000(-1_193_813_598_899_942_000).moment, Moment::At { unix_seconds: utc(1962, 3, 4, 5, 6, 7), nanos: 0 });
        assert_eq!(tt2000(-1_420_113_567_816_000_000).moment, Moment::At { unix_seconds: utc(1955, 1, 1, 0, 0, 0), nanos: 0 });
    }

    /// A moment after the table's last day is still the best answer there is,
    /// and says so. `cdflib`, whose table stops at the same leap second, gives
    /// `2030-06-15T00:00:00.000000000`, and the largest TT2000 there is,
    /// `2292-04-11T11:46:07.670775807`.
    #[test]
    fn a_tt2000_after_the_table_of_leap_seconds_says_so() {
        let later = tt2000(960_984_069_184_000_000);
        assert_eq!(later.moment, Moment::At { unix_seconds: utc(2030, 6, 15, 0, 0, 0), nanos: 0 });
        assert_eq!(later.note, Some(TimeNote::PastLeapSecondTable));
        let last = tt2000(i64::MAX);
        assert_eq!(last.moment, Moment::At { unix_seconds: utc(2292, 4, 11, 11, 46, 7), nanos: 670_775_807 });
        assert_eq!(last.note, Some(TimeNote::PastLeapSecondTable));
        // Either side of the day the table expires. Since 2017 a TT2000 is
        // UTC plus 37 leap seconds plus TT's 32.184, counted from noon.
        let tt2000_of = |unix: i64| (unix - utc(2000, 1, 1, 12, 0, 0)) * 1_000_000_000 + 69_184_000_000;
        let expires = super::leap_seconds::EXPIRES;
        let before = tt2000(tt2000_of(expires - 1));
        assert_eq!(before.moment, Moment::At { unix_seconds: expires - 1, nanos: 0 });
        assert_eq!(before.note, None);
        assert_eq!(tt2000(tt2000_of(expires)).note, Some(TimeNote::PastLeapSecondTable));
    }

    /// CDF's fill value, the most negative `int8`, and its pad value, the one
    /// after it. Both are dates in 1707 read as counts, and both are no time;
    /// the one after those is a count like any other, the first nanosecond of
    /// `1707-09-22T12:12:10.961224194` by `cdflib`.
    #[test]
    fn a_tt2000s_fill_and_pad_values_are_no_time() {
        assert_eq!(tt2000(i64::MIN).moment, Moment::Unset);
        assert_eq!(tt2000(i64::MIN + 1).moment, Moment::Unset);
        let next = tt2000(i64::MIN + 2);
        assert_eq!(next.moment, Moment::At { unix_seconds: utc(1707, 9, 22, 12, 12, 10), nanos: 961_224_194 });
        assert_eq!(next.note, Some(TimeNote::BeforeLeapSeconds));
    }

    /// GPS seconds, as a GWF frame writes its start: 1167264018 is the first
    /// second of 2017 in UTC, eighteen leap seconds after the GPS epoch.
    #[test]
    fn gps_seconds_go_through_the_same_table() {
        let info = one(T::u32(Little), Time::gps_seconds(), 1_167_264_018u32.to_le_bytes().to_vec());
        assert_eq!(at(info), (utc(2017, 1, 1, 0, 0, 0), 0));
    }

    /// A field nothing declared is not a time, and the query says so rather
    /// than reading the number and guessing an epoch for it.
    #[test]
    fn an_undeclared_field_means_no_moment() {
        let root = T::structure("Made", vec![("when", T::u32(Little))]);
        let mut ev = Evaluator::new(Template::new("made-up", root));
        let doc = Document::new(MemSource(vec![1, 2, 3, 4]));
        assert!(ev.time_of(&doc, &[0]).unwrap().is_none());
    }
}
