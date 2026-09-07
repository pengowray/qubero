//! Which way round a stretch of UTF-16 was written.
//!
//! ASCII in UTF-16 LE reads as UTF-16 BE shifted a byte along, minus its first
//! character, so the same bytes give two runs and only one of them is what the
//! file meant. Settled here, before the two readings compete for the bytes.

use super::*;
use super::text::*;

/// Settles which way round a stretch of wide text was written.
///
/// UTF-16 whose characters sit in one page of 256 reads the same at the other
/// endianness one byte over, less its first character: `70 00 72 00` is "pr"
/// little-endian and "r" big-endian starting a byte later. Every ASCII string
/// in a UTF-16 file is like that, so both readings are always found and one of
/// them has to go. The shifted one is always the later of the two, since what
/// it loses is the first character.
///
/// What tells them apart is the byte the later reading starts on, which is the
/// second byte of the earlier reading's first character. Little-endian puts
/// the character first and the zero second, so that byte is a zero and the
/// byte in front of it is a letter: the later reading has started halfway
/// through a character and is the shifted one. Big-endian puts the zero first,
/// so the byte in front is a zero and nothing is settled, which is right,
/// because for big-endian text the later reading is the shifted one for the
/// same reason and there is nothing to choose it by. Then the reading the file
/// vouched for wins, then the longer one, then the one on an even address.
///
/// Whichever survives is vouched for by anything either reading found, since a
/// terminator or a length belongs to the text rather than to one way of
/// reading it. A .NET `#US` string is a length, its characters, then a flag
/// byte, and that flag byte with the next string's length reads as `00 00` to
/// the shifted run and as nothing at all to the real one, so `System.dll`
/// reported three and a half thousand strings missing their first letter.
pub(super) fn shifted_readings(buf: &[u8], base: u64, runs: &[Run], vouch: &[bool], table: &[bool], ok: &mut [bool]) {
    use std::collections::HashMap;
    let mut at: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, r) in runs.iter().enumerate() {
        if r.enc.wide() {
            at.entry(r.start).or_default().push(i);
        }
    }
    for i in 0..runs.len() {
        if !runs[i].enc.wide() {
            continue;
        }
        let Some(js) = at.get(&(runs[i].start + 1)) else { continue };
        for &j in js {
            if runs[j].enc == runs[i].enc {
                continue;
            }
            let keep = if ascii_text(buf[runs[i].start]) {
                i
            } else if table[i] != table[j] {
                // Counted the way the rest of the file counts its strings
                // beats counted once by a number that happens to fit. A
                // Windows string table packs its strings behind a `u16` and
                // leaves its empty slots zero, and those zeroes with the next
                // string's length read as a `u32` or a `u64` in front of the
                // shifted run, which is wide enough to speak for it.
                if table[i] { i } else { j }
            } else if vouch[i] != vouch[j] {
                if vouch[i] { i } else { j }
            } else if runs[i].chars != runs[j].chars {
                if runs[i].chars > runs[j].chars { i } else { j }
            } else if (base + runs[i].start as u64) % 2 == 0 {
                // UTF-16 in a file is nearly always laid on two-byte
                // boundaries. The two starts are a byte apart, so exactly one
                // of them is even.
                i
            } else {
                j
            };
            let (keep, drop) = if keep == i { (i, j) } else { (j, i) };
            ok[keep] = vouch[i] || vouch[j];
            ok[drop] = false;
        }
    }
}
/// Whether anything around a wide run says it was written as a string.
///
/// A wide run gets its evidence too cheaply. Four printable ASCII bytes in a
/// row are four bytes of a file agreeing; four printable UTF-16 LE characters
/// of Latin text are four bytes agreeing and four bytes that only have to be
/// zero, and quiet audio, a depth buffer and a table of small integers all
/// supply those zeroes for nothing. Measured on a recording of a bat: three
/// hundred kilobytes of samples produced thirty eight-bit strings and four
/// thousand nine hundred wide ones, none of them a string.
///
/// So a wide run is reported only when something around it says it was written
/// as one. Three things can: a zero code unit after it, which is two bytes that
/// had to be anything and are zero; a number in front of it wider than one code
/// unit, which has to carry bytes the run's own boundary did not need; or a
/// place in a table, which is what [`in_a_table`] answers.
///
/// The eight-bit pass is deliberately not held to any of this. Four printable
/// bytes in a row is what `strings(1)` reports and what a reader of this view
/// expects, noise and all.
pub(super) fn vouched(buf: &[u8], run: Run, counted: &[(PrefixKind, usize, u64)], table: bool) -> bool {
    let unit = run.enc.unit() as usize;
    run.chars as usize >= LONG_ENOUGH
        || table
        || terminator(buf, run.end, run.enc).is_some()
        || counted.iter().any(|&(_, w, _)| w > unit)
}
/// How long a wide run has to be to speak for itself.
///
/// The rule above exists because four wide characters are cheap: four bytes of
/// a file agreeing and four that only have to be zero. They stop being cheap
/// quickly. Every character after the first has to keep to the same page and
/// be printable, which is about one arrangement of two bytes in seven, so
/// sixteen of them in a row is one stretch in `7^15`, and a file would have to
/// be millions of times larger than any file to throw one up by chance.
///
/// The Windows shortcut that made this rule holds
/// `.shell:::{3080F90D-D7AD-11D9-BD98-0000947B0257}`, forty-six characters of
/// UTF-16 with a `93` after it rather than a zero and nothing but padding in
/// front, and it was refused for the same reason a four-character run with
/// nothing to say for itself is refused.
pub(super) const LONG_ENOUGH: usize = 16;
