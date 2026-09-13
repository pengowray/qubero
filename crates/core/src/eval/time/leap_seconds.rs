//! From a clock that counts every second to the one people write dates on.
//!
//! TAI counts SI seconds and never stops for anything. UTC is TAI held back by
//! a whole number of seconds, and every so often, when the Earth has fallen far
//! enough behind, the IERS announces one more: the last minute of a June or a
//! December gets a sixty-first second, `23:59:60`, and TAI - UTC goes up by one.
//! Twenty-seven of those since 1972 make it 37 seconds now. A count on TAI, or
//! on a clock a fixed distance from it, turns into a UTC date only through the
//! table of when they happened, and a date worked out as if there were none is
//! up to 37 seconds out: the confident wrong date `time.rs` exists to refuse.
//!
//! Before 1972 there were no leap seconds, and UTC was steered instead by
//! seconds very slightly longer than TAI's and by steps of a fraction of one.
//! The table carries that era too, as the NASA CDF library defines it: an
//! offset and a drift per day, taken at noon of the UTC day and held for the
//! whole of it. That is not the only way to read those rows, and the rows
//! themselves start in 1960 where the USNO's start in 1961, so a moment before
//! 1972 is worked out as the CDF library works it out rather than as history
//! has it; [`super::TimeNote::BeforeLeapSeconds`] says so. Before 1960 the
//! library takes TAI - UTC as nought, and so does this.
//!
//! The rows from 1972 on are the IERS list, and every copy of it agrees:
//! `cdflib`'s `CDFLeapSeconds.txt` (which says it was last updated on 25
//! October 2016, for the leap second at the end of that year), the IERS's
//! `Leap_Second.dat` as `astropy` ships it, and the tz database's
//! `leapseconds`. What differs between copies is how long each vouches for:
//! the IERS list read here was "updated through IERS Bulletin C 72 issued in
//! July 2026" and "expires on 28 June 2027", which is the last day on which a
//! leap second nobody has announced yet cannot have happened. See
//! [`EXPIRES`] for what is done about a moment after it.
//!
//! Checked against `cdflib`'s `breakdown_tt2000` over 34,640 counts from 1958
//! to 2035, most of them within three seconds of a row and within twenty
//! milliseconds of a rubber-era midnight. They agree to the nanosecond but for
//! two things, both `cdflib`'s. Before July 1972 it works the offset out in
//! floating point and truncates, so some counts there come out a nanosecond
//! away from the exact arithmetic here; the NASA library's C does the same.
//! And at the first leap second, the end of June 1972, it reads with its
//! pre-1972 arithmetic and has no second 60, so its own count for `23:59:60.5`
//! comes back as `23:59:59.5`. Every later leap second it finds, though it
//! prints one as `23:60:00`, carrying the sixtieth second into the minute.

use super::NANOS_PER_SECOND;

/// Which IERS announcement the rows below are up to date with.
pub const UPDATED: &str = "IERS Bulletin C 72, July 2026";

/// The first instant the rows below do not vouch for, as seconds from
/// 1970-01-01T00:00:00Z: 2027-06-28T00:00:00Z, the expiry date of the IERS
/// list they were copied from.
///
/// A moment after this is still worked out from the table, with no leap second
/// after 2016, because that is the best answer there is and most likely the
/// right one. But a leap second announced since would put it a second late,
/// and a count from 2040 might be several out, so the answer comes with
/// [`super::TimeNote::PastLeapSecondTable`] rather than as a plain date. When
/// the IERS publishes another bulletin, the rows and this date move together.
pub const EXPIRES: i64 = 1_814_140_800;

/// 1972-01-01T00:00:00Z, the first instant of whole leap seconds. Before it
/// the rows are the CDF library's reading of UTC's rubber seconds.
pub const FIRST_LEAP_SECOND_ERA: i64 = 63_072_000;

/// One row of the table: from this UTC date on, TAI - UTC is `offset` seconds,
/// plus `drift` seconds for every day since the modified Julian day `base`.
///
/// Every number is a whole count of nanoseconds, which the rows allow exactly:
/// the offsets are given to the seventh decimal place of a second and the
/// drifts to the seventh of a second a day, and the CDF library's reading of
/// the drift takes the day at its noon, which halves an even number.
#[derive(Debug, Clone, Copy)]
struct Row {
    year: i64,
    month: u32,
    offset_nanos: i64,
    base_mjd: i64,
    drift_nanos: i64,
}

/// A row of the era before leap seconds, as the CDF library's table writes it:
/// the offset and the drift in seconds, here in nanoseconds.
const fn rubber(year: i64, month: u32, offset_nanos: i64, base_mjd: i64, drift_nanos: i64) -> Row {
    Row { year, month, offset_nanos, base_mjd, drift_nanos }
}

/// A row of whole leap seconds, from the first of the month.
const fn leap(year: i64, month: u32, seconds: i64) -> Row {
    Row { year, month, offset_nanos: seconds * 1_000_000_000, base_mjd: 0, drift_nanos: 0 }
}

/// TAI - UTC from each first of the month on. Every row starts on the first,
/// which is what lets a row be named by its year and month alone.
const ROWS: &[Row] = &[
    rubber(1960, 1, 1_417_818_000, 37_300, 1_296_000),
    rubber(1961, 1, 1_422_818_000, 37_300, 1_296_000),
    rubber(1961, 8, 1_372_818_000, 37_300, 1_296_000),
    rubber(1962, 1, 1_845_858_000, 37_665, 1_123_200),
    rubber(1963, 11, 1_945_858_000, 37_665, 1_123_200),
    rubber(1964, 1, 3_240_130_000, 38_761, 1_296_000),
    rubber(1964, 4, 3_340_130_000, 38_761, 1_296_000),
    rubber(1964, 9, 3_440_130_000, 38_761, 1_296_000),
    rubber(1965, 1, 3_540_130_000, 38_761, 1_296_000),
    rubber(1965, 3, 3_640_130_000, 38_761, 1_296_000),
    rubber(1965, 7, 3_740_130_000, 38_761, 1_296_000),
    rubber(1965, 9, 3_840_130_000, 38_761, 1_296_000),
    rubber(1966, 1, 4_313_170_000, 39_126, 2_592_000),
    rubber(1968, 2, 4_213_170_000, 39_126, 2_592_000),
    leap(1972, 1, 10),
    leap(1972, 7, 11),
    leap(1973, 1, 12),
    leap(1974, 1, 13),
    leap(1975, 1, 14),
    leap(1976, 1, 15),
    leap(1977, 1, 16),
    leap(1978, 1, 17),
    leap(1979, 1, 18),
    leap(1980, 1, 19),
    leap(1981, 7, 20),
    leap(1982, 7, 21),
    leap(1983, 7, 22),
    leap(1985, 7, 23),
    leap(1988, 1, 24),
    leap(1990, 1, 25),
    leap(1991, 1, 26),
    leap(1992, 7, 27),
    leap(1993, 7, 28),
    leap(1994, 7, 29),
    leap(1996, 1, 30),
    leap(1997, 7, 31),
    leap(1999, 1, 32),
    leap(2006, 1, 33),
    leap(2009, 1, 34),
    leap(2012, 7, 35),
    leap(2015, 7, 36),
    leap(2017, 1, 37),
];

const DAY_NANOS: i128 = 86_400 * NANOS_PER_SECOND;

/// The modified Julian day of 1970-01-01.
const UNIX_MJD: i64 = 40_587;

/// Where a TAI instant lands on UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Utc {
    /// Nanoseconds from 1970-01-01T00:00:00Z, on UTC's own count, in which
    /// every day is 86,400 seconds long and a leap second is not a number.
    At(i128),
    /// Inside a second UTC inserted: second 60 of the minute whose second 59
    /// is `second_59` seconds from 1970, and `nanos` into it.
    Inserted { second_59: i64, nanos: u32 },
}

/// TAI - UTC, in nanoseconds, all through the UTC day `day` days from
/// 1970-01-01.
fn offset_on(day: i64) -> i128 {
    let Some(row) = ROWS.iter().rev().find(|r| super::days_from_civil(r.year, r.month, 1) <= day) else {
        // Before 1960, which is where the CDF library's table starts and
        // where it takes the offset as nought.
        return 0;
    };
    let drift = match row.drift_nanos {
        0 => 0,
        // Taken at the day's noon, which is half a day past its modified
        // Julian day number.
        d => (day + UNIX_MJD - row.base_mjd) as i128 * d as i128 + (d / 2) as i128,
    };
    row.offset_nanos as i128 + drift
}

/// The UTC moment a TAI instant is, given as nanoseconds from
/// 1970-01-01T00:00:00 read on TAI, or nothing if it is too far from now for a
/// day number to be worked out at all.
///
/// Worked out a day at a time, because a day is what an offset belongs to: the
/// UTC day `d` runs over the TAI instants from its midnight plus `offset_on(d)`
/// for 86,400 seconds, and the instant is on day `d` when taking that offset
/// off lands it inside `d`. The offset now in force puts the day within one of
/// the right one, so three days are tried.
///
/// Two things can happen at a midnight where the offset changes. When it goes
/// up, there are TAI instants no day claims: the old offset puts them past
/// midnight and the new one before it. Where it went up by a whole second,
/// which is every leap second, those are the inserted second, `23:59:60`, and
/// are answered as [`Utc::Inserted`]. Where it went up by less, which is every
/// midnight of the rubber era and the tenth of a second at the start of 1972,
/// they are read with the new day's offset, as the CDF library reads them.
/// When it goes down, as it did twice in the 1960s, two days claim the same
/// instants, and the later day is taken, which is again what the library does:
/// the earlier one's last fraction of a second is the part UTC skipped.
pub fn utc_of(tai: i128) -> Option<Utc> {
    let day = |n: i128| i64::try_from(n.div_euclid(DAY_NANOS)).ok();
    let guess = day(tai - offset_on(day(tai)?))?;
    let candidates = [guess - 1, guess, guess + 1];
    if let Some(&d) = candidates.iter().rev().find(|&&d| day(tai - offset_on(d)) == Some(d)) {
        return Some(Utc::At(tai - offset_on(d)));
    }
    candidates.iter().find_map(|&d| {
        let midnight = (d as i128 + 1) * DAY_NANOS;
        let past = tai - offset_on(d) - midnight;
        let next = tai - offset_on(d + 1);
        if !(0..NANOS_PER_SECOND).contains(&past) || next >= midnight {
            return None;
        }
        match offset_on(d + 1) - offset_on(d) {
            NANOS_PER_SECOND => Some(Utc::Inserted { second_59: (midnight / NANOS_PER_SECOND) as i64 - 1, nanos: past as u32 }),
            // A sliver less than a second, which only the rubber era has: the
            // drift of a millisecond or two at every midnight, and the tenth
            // of a second at the start of 1972. UTC never wrote a sixty-first
            // second for those, and the CDF library reads the sliver with the
            // new day's offset, as the last moments of the old day over again.
            // So does this, since the library defines what these counts mean.
            _ => Some(Utc::At(next)),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rows are in order, each month named once, and every leap second
    /// from 1972 on is one second more than the last.
    #[test]
    fn the_rows_are_in_order_and_step_by_one_second_after_1972() {
        for pair in ROWS.windows(2) {
            assert!((pair[0].year, pair[0].month) < (pair[1].year, pair[1].month), "{pair:?}");
        }
        let leaps: Vec<&Row> = ROWS.iter().filter(|r| r.year >= 1972).collect();
        assert_eq!(leaps.len(), 28);
        for pair in leaps.windows(2) {
            assert_eq!(pair[1].offset_nanos - pair[0].offset_nanos, 1_000_000_000, "{pair:?}");
        }
        assert_eq!(leaps.last().unwrap().offset_nanos, 37_000_000_000);
    }

    /// The two dates named in the doc are the instants they say.
    #[test]
    fn the_named_instants_are_the_days_they_say() {
        assert_eq!(super::super::days_from_civil(2027, 6, 28) * 86_400, EXPIRES);
        assert_eq!(super::super::days_from_civil(1972, 1, 1) * 86_400, FIRST_LEAP_SECOND_ERA);
    }

    fn seconds(s: i64) -> i128 {
        s as i128 * NANOS_PER_SECOND
    }

    /// Around the leap second at the end of 2016: TAI - UTC was 36 before it
    /// and 37 after, and the second between belongs to neither day.
    #[test]
    fn the_second_inserted_at_the_end_of_2016() {
        let midnight = 1_483_228_800; // 2017-01-01T00:00:00Z
        assert_eq!(utc_of(seconds(midnight - 1 + 36)), Some(Utc::At(seconds(midnight - 1))));
        assert_eq!(utc_of(seconds(midnight + 36)), Some(Utc::Inserted { second_59: midnight - 1, nanos: 0 }));
        assert_eq!(
            utc_of(seconds(midnight + 36) + 999_999_999),
            Some(Utc::Inserted { second_59: midnight - 1, nanos: 999_999_999 })
        );
        assert_eq!(utc_of(seconds(midnight + 37)), Some(Utc::At(seconds(midnight))));
    }
}
