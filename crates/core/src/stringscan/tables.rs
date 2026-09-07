//! Telling a table of numbers from a run of text.
//!
//! Offsets, lengths and indices laid end to end are printable often enough for
//! any scanner to find them, and are not strings. Five shapes a table has that
//! text does not.

use super::*;
use super::text::*;

/// How many strings a kind of number has to count, and what share of the runs
/// in the window, before it is taken to be how this file counts its strings.
///
/// A one-byte number matches a run's length by accident about one time in a
/// hundred and sixty, so a window of a thousand runs throws up half a dozen
/// for nothing. A file that really counts its strings that way counts nearly
/// all of them. An eighth of the runs is twenty times what chance produces and
/// well under what a format produces, and the floor stops a window holding
/// three runs from proving anything.
pub(super) const TABLE_LEAST: usize = 3;
pub(super) const TABLE_SHARE: usize = 8;
/// How many different numbers a kind has to take before it is counting
/// anything rather than delimiting it.
pub(super) const TABLE_VALUES: usize = 3;
/// Which runs sit in a table of counted strings.
///
/// This is the only thing that makes a number no wider than one character
/// worth anything. A run is maximal, so whatever sits in front of it is a
/// value that could not be part of it, and the values that could not be are
/// the small ones, which is also what a short string's length looks like: a
/// match there is the run's own boundary read a second time, and on the sample
/// collection it happened eleven thousand times.
///
/// What says otherwise is the same kind of number counting a good share of
/// every string in the window, and taking a different value as it goes. A
/// pickle, a Thrift structure and a MATLAB file all look like that, their
/// strings scattered through metadata rather than packed together.
///
/// Both halves of that are needed. Without the share, six coincidences in a
/// thousand runs would speak for the file. Without the variation, a delimiter
/// would: a run of format strings separated by newlines has 0x0a in front of
/// every one of them, and `Access: %x`, `Modify: %y` and `Change: %z` are all
/// ten bytes long, so all three "match" and none of them is counted.
///
/// What this cannot do is speak for a table of five counted strings sitting in
/// a megabyte of code, since five matches in a window of five hundred runs is
/// what chance looks like too. Those readings are shown as the coincidences
/// they may be, with the bytes beside them.
pub(super) fn in_a_table(runs: &[Run], counted: &[Vec<(PrefixKind, usize, u64)>]) -> Vec<bool> {
    use std::collections::HashMap;
    let mut uses: HashMap<PrefixKind, Vec<u64>> = HashMap::new();
    for row in counted {
        for &(kind, _, value) in row {
            uses.entry(kind).or_default().push(value);
        }
    }
    let n = runs.len();
    let counts = |kind: PrefixKind| {
        uses.get(&kind).is_some_and(|values| {
            if values.len() < TABLE_LEAST || values.len() * TABLE_SHARE < n {
                return false;
            }
            let mut seen: Vec<u64> = values.clone();
            seen.sort_unstable();
            seen.dedup();
            seen.len() >= TABLE_VALUES
        })
    };
    // One answer per kind rather than one per run: the question is about the
    // file, and a window holds thousands of runs.
    let told: HashMap<PrefixKind, bool> =
        uses.keys().map(|&kind| (kind, counts(kind))).collect();
    counted
        .iter()
        .map(|row| row.iter().any(|&(kind, _, _)| told.get(&kind).copied().unwrap_or(false)))
        .collect()
}
/// Whether a wide run is a column of numbers rather than characters.
///
/// A table of offsets into something is a run of sixteen-bit numbers, and read
/// two bytes at a time that is a run of characters passing every other test
/// here: `00 a0 08 a0 10 a0 18 a0` is four Yi syllables and is a jump table in
/// a Godot executable. Two things give one away.
///
/// Every entry is aligned, so every character is a multiple of eight. A letter
/// is a multiple of eight about one time in eight, so five in a row is a table
/// and not a word, and there are few enough wide runs in a file for one in
/// thirty thousand to be no risk at all.
///
/// Or every entry is the same distance above the last. That is the same table
/// without the alignment. A step of one is left alone while a run is short,
/// since "abcdef" is a word a file might hold and "0123456789" certainly is,
/// but twelve characters each one above the last is a sorted list: a C library
/// carries its collation tables that way, sixty Han characters in a row in
/// code point order.
///
/// Or every character is written twice. Stereo audio at sixteen bits is a run
/// of doubled samples, and eighty-five of them in one recording read as
/// Odia with the same character twice over and over. A word has a double
/// letter in it here and there; it does not have one in every place.
///
/// Both are needed. A table with a gap in it is no longer a progression, and
/// refusing only the exact ones hands the bytes to a reading of the same table
/// with a step missing: on a Godot executable the strict test alone removed
/// forty-two rows and put back seventy-four.
pub(super) fn counting(buf: &[u8], start: usize, end: usize, big: bool) -> bool {
    let units = || (start..end).step_by(2).filter_map(|i| unit_at(buf, i, big));
    let n = units().count();
    if n >= 5 && units().fold(0u16, |a, u| a | u) & 7 == 0 {
        return true;
    }
    let all: Vec<u16> = units().collect();
    if n >= 8 && n % 2 == 0 && all.chunks(2).all(|c| c[0] == c[1]) {
        return true;
    }
    // Or every character is above the last, or every one below it. That is a
    // sorted list of code points, which is what a font's coverage table and a
    // library's collation table are; a word is not in alphabetical order.
    // Strictly, so that a field padded with spaces before its letters is not
    // caught by it.
    if n >= 8 && (all.windows(2).all(|w| w[0] < w[1]) || all.windows(2).all(|w| w[0] > w[1])) {
        return true;
    }
    // Or it says the same short thing over and over. A font's metrics repeat,
    // and `$H$H$H$H` for sixty characters is a column of one number written
    // twice; the leading character that is not part of the cycle is what
    // carries it past the test for a run that says more than one thing.
    if n >= 8 {
        for period in 1..=4 {
            let same = all.iter().skip(period).zip(&all).filter(|(a, b)| a == b).count();
            if same * 5 >= (n - period) * 4 {
                return true;
            }
        }
    }
    let mut it = units();
    let (Some(a), Some(b)) = (it.next(), it.next()) else { return false };
    let step = b as i32 - a as i32;
    let least = if step.unsigned_abs() < 2 { 12 } else { 4 };
    if step == 0 || n < least {
        return false;
    }
    let mut last = b;
    it.all(|u| {
        let ok = u as i32 - last as i32 == step;
        last = u;
        ok
    })
}
