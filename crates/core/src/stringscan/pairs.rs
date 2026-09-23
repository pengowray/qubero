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
/// Big-endian text that starts on a zero something else may own is settled
/// by whether that something is text on its own account, and that is asked
/// before the lengths, which a stray character at either end can tip:
///
/// * The second zero of a little-endian string's terminator, where that string
///   was kept. The later reading starts after the terminator, as the next
///   string in the list. A minidump's `…Default 00 00 C 00 : 00` came out as
///   big-endian `C:\src\crashpad\0`, one character longer for the stray
///   after it.
/// * The zero after eight-bit text. `…lar 00 51 00 75 00` is eight-bit text
///   then big-endian "Qu", or a C string, its terminator, then little-endian
///   "Qu", and the bytes are the same either way. What decides is whether the
///   eight-bit text is a C string: one that starts after a zero is, like the
///   one before it, and the zero after it is its terminator. One that starts
///   after anything else is text packed end to end, and the zero is the first
///   byte of the next string. The first is a list of names in a PE file; the
///   second is a TrueType `name` table, Mac Roman names followed by the same
///   names in UTF-16 BE.
///
/// The pairs are settled in the order they sit in the file, so a string in a
/// list is judged by the strings before it as they were settled, and not as
/// they were found. A list of big-endian strings has a shifted little-endian
/// reading of each, terminators and all, and those must not count.
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
    // Where each eight-bit run ends, and whether a zero is in front of it.
    let narrow: HashMap<usize, bool> = runs
        .iter()
        .filter(|r| !r.enc.wide())
        .map(|r| (r.end, r.start.checked_sub(1).is_some_and(|k| buf[k] == 0)))
        .collect();
    // The second byte of each little-endian terminator, and whose it is.
    let terminated: HashMap<usize, usize> = runs
        .iter()
        .enumerate()
        .filter(|(_, r)| r.enc == Enc::Utf16Le && terminator(buf, r.end, r.enc).is_some())
        .map(|(k, r)| (r.end + 1, k))
        .collect();
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for i in 0..runs.len() {
        if !runs[i].enc.wide() {
            continue;
        }
        let Some(js) = at.get(&(runs[i].start + 1)) else { continue };
        pairs.extend(js.iter().filter(|&&j| runs[j].enc != runs[i].enc).map(|&j| (i, j)));
    }
    pairs.sort_by_key(|&(i, _)| runs[i].start);
    for (i, j) in pairs {
        let s = runs[i].start;
        // Whether the zero a big-endian reading starts on belongs to text in
        // front of it, and so whether the little-endian one a byte later is
        // the string. None where nothing claims it.
        let owned = match runs[i].enc {
            Enc::Utf16Be if buf[s] == 0 => match terminated.get(&s) {
                Some(&k) if ok[k] => Some(true),
                _ => narrow.get(&s).copied(),
            },
            _ => None,
        };
        let keep = if ascii_text(buf[s]) {
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
        } else if let Some(owned) = owned {
            if owned { j } else { i }
        } else if runs[i].chars != runs[j].chars {
            if runs[i].chars > runs[j].chars { i } else { j }
        } else if (base + s as u64) % 2 == 0 {
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
/// Takes the first character off a little-endian run whose first byte is the
/// last letter of an eight-bit run, where that run is long enough without it.
///
/// Little-endian text straight after eight-bit text reads as starting one
/// character early: the last letter of the eight-bit text and the zero after
/// it are a perfectly good wide character. So is the same stretch read
/// big-endian from the zero, and the two readings are what
/// [`shifted_readings`] chooses between. Left with the letter, the
/// little-endian one is always the longer, and it wins on that: a TrueType
/// `name` table, which is Mac Roman names followed by the same names in UTF-16
/// BE, came out as one little-endian run starting with the last letter of the
/// Mac Roman block, and the Mac Roman block, which then overlapped it, was
/// dropped whole. A PE file's `BCryptGetProperty` followed by its wide
/// `HashDigestLength` came out as `yHashDigestLength`.
///
/// The letter is the eight-bit run's because that run is text on its own
/// account. A few printable bytes that only make a run with the letter are
/// not, and the letter stays with the wide text.
pub(super) fn leave_narrow_runs_whole(buf: &[u8], min: usize, runs: &mut Vec<Run>) {
    // The eight-bit runs come out of their walk in order and never overlap, so
    // the one holding a byte is found by bisection.
    let narrow: Vec<(usize, usize)> =
        runs.iter().filter(|r| !r.enc.wide() && r.chars as usize > min).map(|r| (r.start, r.end)).collect();
    let inside = |k: usize| {
        let n = narrow.partition_point(|&(_, end)| end <= k);
        narrow.get(n).is_some_and(|&(start, _)| start < k)
    };
    runs.retain_mut(|r| {
        if r.enc != Enc::Utf16Le || !ascii_text(buf[r.start]) || !inside(r.start) {
            return true;
        }
        r.start += 2;
        r.chars -= 1;
        r.units -= 1;
        r.chars as usize >= min
            && unit_at(buf, r.start, false).and_then(|u| char::from_u32(u as u32)).is_some_and(starter)
    });
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
