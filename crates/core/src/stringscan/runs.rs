//! The two walks: where a run of text starts and stops.
//!
//! One walks a byte at a time for ASCII and UTF-8, the other a code unit at a
//! time for UTF-16, at both byte parities because nothing aligns a string in a
//! binary. Neither decides what text is; they ask [`super::text`], offer a run
//! whole and then in pieces, and keep what answers. See [`super::pairs`] for
//! which of a wide run's two readings survives.

use super::*;
use super::text::*;

/// The part of a wide run that is the string, without what it ran into at
/// either end.
///
/// A run is maximal, and what lies either side of a string in a binary is
/// whatever the compiler put there. Two bytes of that are usually some
/// character, so a run reaches back over the pointer in front of the string
/// and forward over the one behind it, and those characters are why an
/// otherwise coherent run stops looking coherent. Both ends are cut back:
/// first the units that are eight-bit text read wide, then the units that are
/// not from the part of Unicode the rest of the run is from. What is left is
/// the string, and the tests that follow are asked about that rather than
/// about it plus its neighbours.
///
/// A surrogate is never cut, since half of a pair at the end of a run is the
/// character the reader came for.
pub(super) fn trim(buf: &[u8], start: usize, end: usize, big: bool) -> (usize, usize) {
    let high = usize::from(!big);
    let mut a = start;
    let mut b = end;
    while a + 2 <= b && ascii_text(buf[a + high]) {
        a += 2;
    }
    while b >= a + 2 && ascii_text(buf[b - 2 + high]) {
        b -= 2;
    }
    let Some(page) = dominant_page(buf, a, b, big) else { return (a, b) };
    let odd = |i: usize| {
        let u = unit_at(buf, i, big).unwrap_or(0);
        (u >> 8) as u8 != page && !(0xd800..0xe000).contains(&u)
    };
    while a + 2 <= b && odd(a) {
        a += 2;
    }
    while b >= a + 2 && odd(b - 2) {
        b -= 2;
    }
    (a, b)
}
/// Runs of printable bytes, taking a valid UTF-8 sequence as the one character
/// it is. A run holding one is UTF-8; a run of nothing but ASCII is ASCII,
/// which is true of it in every single-byte code page as well.
pub(super) fn narrow_runs(buf: &[u8], from: usize, min: usize, out: &mut Vec<Run>) {
    let mut i = from;
    while i < buf.len() {
        let start = i;
        let mut chars = 0u32;
        let mut multi = 0u32;
        // The longest stretch of plain ASCII in the run, which is what the run
        // would be worth without any wide character in it.
        let mut plain = 0u32;
        let mut ascii = 0u32;
        let mut j = i;
        while let Some((c, n)) = utf8_char(buf, j) {
            if if chars == 0 { !starter(c) } else { !printable(c) } {
                break;
            }
            if n > 1 {
                multi += 1;
                ascii = 0;
            } else {
                ascii += 1;
                plain = plain.max(ascii);
            }
            chars += 1;
            j += n;
        }
        // One wide character does not make a run of compiled code into text,
        // and must not be what carries a run over the minimum. Two bytes of
        // x86 are a valid UTF-8 character often enough that taking one at its
        // word turns half a code section into strings: c6 8b is a perfectly
        // good character, and it welds ")" and "D$P" into a five-character
        // string that neither half was. So a run earns its place on a stretch
        // of ASCII long enough on its own, or on holding a run of wide
        // characters long enough on its own.
        if j > start {
            if chars as usize >= min && (plain as usize >= min || multi as usize >= min) {
                let enc = if multi > 0 { Enc::Utf8 } else { Enc::Ascii };
                out.push(Run { start, end: j, enc, chars, units: (j - start) as u32, lone: false, quality: 3 });
            }
            i = j;
        } else {
            i += 1;
        }
    }
}
/// Runs of UTF-16, at both byte parities: nothing in a binary aligns a string,
/// and a UTF-16 run starting at an odd address is as common as one starting at
/// an even one.
///
/// A run whose bytes are all printable ASCII is not reported, because the
/// eight-bit reading already explains them and is the simpler account: "Hello
/// world" read two bytes at a time is a row of CJK characters, and a scanner
/// that believed that would report every English sentence twice.
pub(super) fn wide_runs(buf: &[u8], from: usize, min: usize, enc: Enc, out: &mut Vec<Run>, tables: &mut [bool]) {
    let big = enc.big();
    for parity in 0..2usize {
        let mut i = from + parity;
        while i + 1 < buf.len() {
            let start = i;
            let mut chars = 0u32;
            let mut j = i;
            while let Some(u) = unit_at(buf, j, big) {
                if is_high_surrogate(u) {
                    match unit_at(buf, j + 2, big) {
                        Some(v) if is_low_surrogate(v) => {
                            chars += 1;
                            j += 4;
                            continue;
                        }
                        // A surrogate with no partner is WTF-16, which is real
                        // and worth keeping. It cannot start a run, so a
                        // stretch of arbitrary bytes that happens to land in
                        // the surrogate block is not a string.
                        _ if chars > 0 => {
                            chars += 1;
                            j += 2;
                            continue;
                        }
                        _ => break,
                    }
                }
                if is_low_surrogate(u) {
                    if chars == 0 {
                        break;
                    }
                    chars += 1;
                    j += 2;
                    continue;
                }
                match char::from_u32(u as u32) {
                    Some(c) if if chars == 0 { starter(c) } else { printable(c) } => {
                        chars += 1;
                        j += 2;
                    }
                    _ => break,
                }
            }
            if j > start {
                let (a, b) = trim(buf, start, j, big);
                // The run, and then the run in pieces. Both, because a run is
                // allowed a stray character and a string that ends in a
                // separator with the next string behind it is a run with a
                // stray in it: three hundred lines of a .NET user string heap
                // read as one run of ten thousand characters. The whole
                // outranks its pieces and takes the bytes wherever it stands;
                // where it cannot say it is a string, the pieces are there to
                // be taken instead. See `page_pieces`.
                //
                // If the run held together, its ends have been trimmed and the
                // pieces come from what is left. If it did not, they come from
                // the raw run: trimming assumes a run is a string with rubbish
                // at its ends, and a run that failed may be rubbish with
                // strings in it. One run of thirteen thousand bytes in
                // `System.dll` was cut back to a point before the first of the
                // strings inside it.
                let (lo, hi) = match consider(buf, a, b, enc, min, out, tables) {
                    true => (a, b),
                    false => (start, j),
                };
                let mut at = lo;
                while let Some((p, q)) = page_pieces(buf, at, hi, big) {
                    if (p, q) != (lo, hi) {
                        consider(buf, p, q, enc, min, out, tables);
                    }
                    at = q;
                }
                i = j;
            } else {
                i += 2;
            }
        }
    }
}
/// Reports `[start, end)` as a wide run if everything about it says string,
/// and says whether it did.
pub(super) fn consider(buf: &[u8], start: usize, end: usize, enc: Enc, min: usize, out: &mut Vec<Run>, tables: &mut [bool]) -> bool {
    let big = enc.big();
    if start + 2 > end {
        return false;
    }
    // A piece can begin anywhere its page does, which is not always somewhere
    // a string can begin.
    if !unit_at(buf, start, big)
        .and_then(|u| char::from_u32(u as u32))
        .is_some_and(starter)
        && !unit_at(buf, start, big).is_some_and(is_high_surrogate)
    {
        return false;
    }
    let (chars, units, lone) = measure(buf, start, end, enc);
    // A surrogate with no partner is WTF-16 and is worth keeping, but a run
    // that is half of them is not a name with one bad character in it. It is
    // compressed bytes: a Godot pack is full of runs that read as two unpaired
    // halves and two Hangul syllables, and every other test here passes them.
    if chars as usize >= min
        && lone * 3 < chars
        && wide_enough(buf, start, end, big)
        && one_page(buf, start, end, big)
        && varied(buf, start, end, big)
        && letters(buf, start, end, big)
    {
        // Asked last, and of runs that would otherwise have been reported,
        // because a run judged a table takes its bytes out of the reckoning
        // for every other reading of them. A four-unit scrap from the middle
        // of a string is not allowed to do that.
        if counting(buf, start, end, big) {
            tables[start..end].fill(true);
            return false;
        }
        let latin = (start..end)
            .step_by(2)
            .all(|k| unit_at(buf, k, big).is_some_and(|u| u < 0x100 || u >= 0xd800));
        let quality = if latin { 3 } else { 2 };
        out.push(Run { start, end, enc, chars, units, lone: lone > 0, quality });
        return true;
    }
    false
}
/// The next stretch of `[at, end)` whose characters all come from one page.
///
/// A maximal run reaches over whatever the compiler left either side of the
/// string, and one character of the string's own page in among that rubbish
/// pins the trim in place: in a .NET `#US` heap the byte pairs before
/// "providerOptions" read as three Han characters with a 'z' between each of
/// them, so the cut stopped at the first 'z' and a run of four pages went to
/// the page test and lost. Cutting instead at every change of page offers the
/// string on its own, and offers the rubbish separately, where it fails on its
/// own account.
///
/// This is only reached when the run as a whole has already failed, so the
/// allowance for a stray character is untouched: a run that keeps its page is
/// never cut up.
pub(super) fn page_pieces(buf: &[u8], at: usize, end: usize, big: bool) -> Option<(usize, usize)> {
    if at + 2 > end {
        return None;
    }
    // A surrogate has no page of its own worth the name, so it stays with
    // whatever it was found next to.
    let page_of = |i: usize| {
        let u = unit_at(buf, i, big).unwrap_or(0);
        (!(0xd800..0xe000).contains(&u)).then_some((u >> 8) as u8)
    };
    let page = (at..end).step_by(2).find_map(page_of);
    let mut b = at + 2;
    while b + 2 <= end && page_of(b).is_none_or(|p| Some(p) == page) {
        b += 2;
    }
    Some((at, b))
}
