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
use crate::template::{Counted, Epoch, Time, Zone};

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
        if time.unset == Some(value) {
            return Ok(Some(TimeInfo { moment: Moment::Unset, zone: time.zone, step_nanos: step_of(&time.epoch) }));
        }
        let moment = match &time.epoch {
            Epoch::Counted(c) => counted(value, *c),
            Epoch::Dos => {
                // The date is the top half and the time the bottom, and a value
                // wider than the thirty-two bits this is packed into is not one
                // of these at all rather than one to be masked down to size.
                match u32::try_from(value) {
                    Ok(v) => dos(v >> 16, v & 0xffff),
                    Err(_) => Moment::Impossible,
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
        Ok(Some(TimeInfo { moment, zone: time.zone, step_nanos: step_of(&time.epoch) }))
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
    fn time_at<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Option<Time>> {
        self.resolve(doc, path)?;
        let Some((&idx, parent)) = path.split_last() else { return Ok(None) };
        if let Some(t) = self.declared(parent, idx) {
            return Ok(Some(t));
        }
        let Some((&pidx, grand)) = parent.split_last() else { return Ok(None) };
        if !matches!(
            self.memo.get(parent).map(|r| &r.ty),
            Some(Ty::Array { .. } | Ty::Repeat { .. } | Ty::Chain { .. } | Ty::PointerList { .. })
        ) {
            return Ok(None);
        }
        Ok(self.declared(grand, pidx))
    }

    /// The declaration on field `idx` of the structure at `parent`, if that is
    /// a structure and the field has one.
    fn declared(&self, parent: &[usize], idx: usize) -> Option<Time> {
        match self.memo.get(parent).map(|r| &r.ty) {
            Some(Ty::Struct(s)) => s.fields.get(idx).and_then(|f| f.time.clone()),
            _ => None,
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
    fn moment_number<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Option<i128>> {
        Ok(match &self.node(doc, path)?.value {
            Value::UInt(v) => i128::try_from(*v).ok(),
            Value::Int(v) => Some(*v),
            Value::Enum { raw, .. } => Some(*raw),
            // A slot the format leaves empty is still the number written there,
            // and a format that has both a sentinel of its own and a declared
            // unset value should have them agree.
            Value::Unset(inner) => match &**inner {
                Value::UInt(v) => i128::try_from(*v).ok(),
                Value::Int(v) => Some(*v),
                _ => None,
            },
            _ => None,
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
        let Some(v) = self.moment_number(doc, &p)? else { return Ok(None) };
        Ok(u32::try_from(v).ok().filter(|v| *v <= 0xffff))
    }
}

/// How precise a field of this epoch is, in nanoseconds. An MS-DOS time counts
/// seconds in twos, and a reader who has been shown an odd second has been
/// shown a digit the file cannot hold.
fn step_of(epoch: &Epoch) -> u64 {
    match epoch {
        Epoch::Counted(c) => c.step_nanos,
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
fn counted(value: i128, c: Counted) -> Moment {
    let Some(steps) = value.checked_mul(c.step_nanos as i128) else { return Moment::Impossible };
    let Some(nanos) = (c.zero as i128).checked_mul(NANOS_PER_SECOND).and_then(|z| steps.checked_add(z)) else {
        return Moment::Impossible;
    };
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
    use super::{Moment, TimeInfo};
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
