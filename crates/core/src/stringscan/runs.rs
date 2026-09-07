//! Where a run of text starts and stops, and whether it is text at all.
//!
//! Two walks, one a byte at a time and one a code unit at a time, and the
//! battery of questions a wide run has to answer before it is reported. What
//! the answers are for is in [`super`].

use super::*;
use super::tables::*;

// --- what counts as text ---------------------------------------------------

/// Whether a character is one a string would hold. Tab is text; the other
/// controls are not, and a newline ends a run rather than joining two lines
/// into one string, which is what `strings(1)` does and what makes a list of
/// found strings readable.
pub(super) fn printable(c: char) -> bool {
    if c == '\t' {
        return true;
    }
    if c.is_control() || c == '\u{fffd}' {
        return false;
    }
    // Noncharacters are guaranteed never to be a character, so a run of them
    // is never text. It is worth naming them: a firmware image padded with
    // 0xff reads two bytes at a time as U+FFFF over and over, which is
    // otherwise a perfectly coherent run of one Unicode page.
    let u = c as u32;
    !(0xfdd0..=0xfdef).contains(&u) && (u & 0xfffe) != 0xfffe
}
/// Whether a character can be the first one of a string.
///
/// A combining mark attaches to the character before it, and a format
/// character is invisible, so neither begins a string. Refusing them is worth
/// more than it looks: arbitrary bytes land on the combining block often
/// enough that letting one start a run makes the run one character longer
/// than it is, which is enough to lose a reading to a wrong one that covers
/// more of the file. What is refused is a start, not a character: a mark
/// inside a string is the string's, and stays.
pub(super) fn starter(c: char) -> bool {
    if !printable(c) {
        return false;
    }
    let u = c as u32;
    !matches!(u,
        0x0300..=0x036f      // combining diacritics
        | 0x0483..=0x0489    // Cyrillic marks
        | 0x0590..=0x05bf    // Hebrew points
        | 0x0610..=0x061a    // Arabic marks
        | 0x064b..=0x065f
        | 0x0e31 | 0x0e34..=0x0e3a | 0x0e47..=0x0e4e   // Thai marks
        | 0x1ab0..=0x1aff    // combining extensions
        | 0x1dc0..=0x1dff
        | 0x00ad             // soft hyphen
        | 0x200b..=0x200f    // zero width and direction marks
        | 0x202a..=0x202e
        | 0x2060..=0x206f
        | 0x20d0..=0x20f0    // combining marks for symbols
        | 0xfe00..=0xfe0f    // variation selectors
        | 0xfe20..=0xfe2f
        | 0xfeff)
}
/// Whether a byte on its own would be printable ASCII, which is what tells a
/// genuine UTF-16 run from an ASCII one misread as wide characters.
pub(super) fn ascii_text(b: u8) -> bool {
    b == b'\t' || (0x20..0x7f).contains(&b)
}
/// Whether a wide run's characters all come from one part of Unicode.
///
/// This is what tells wide text from arbitrary bytes read two at a time.
/// Almost every sixteen-bit number is some printable character, so a stretch
/// of compiled code read as UTF-16 is a run of them, and a scanner with
/// nothing to say about that reports a page of gibberish beside the strings
/// worth reading. Real text does not wander: a run of it is Latin, or Greek,
/// or Cyrillic, and the high byte of its characters says which.
///
/// How much wandering is allowed depends on how long the run is, because one
/// stray character out of four is a different claim from one out of forty. A
/// run is allowed one character from somewhere else for every eight it has,
/// which is none at all at the shortest length reported. That is what
/// separates a MIPS instruction word, which is three characters of one page
/// and one of another, from a sentence.
///
/// Half of a surrogate pair is never counted against a run: an emoji in a line
/// of Latin text is two units from a page of its own and is still the line the
/// reader wrote.
pub(super) fn one_page(buf: &[u8], start: usize, end: usize, big: bool) -> bool {
    let Some(page) = dominant_page(buf, start, end, big) else { return false };
    if !text_page(page) {
        return false;
    }
    let mut units = 0u32;
    let mut stray = 0u32;
    let mut i = start;
    while i + 2 <= end {
        let u = unit_at(buf, i, big).unwrap_or(0);
        if (u >> 8) as u8 != page && !(0xd800..0xe000).contains(&u) {
            // A character from somewhere else, with a control byte for its
            // low half, is not a character at all: it is the end of one
            // string and the start of the number counting the next, read as
            // one unit. A .NET user string heap joins its lines that way,
            // three hundred of them, and the allowance below would let the
            // whole heap pass as one string of ten thousand characters.
            //
            // Nor is one from a part of Unicode nobody writes in. That is
            // asked of the page the run is mostly from, and a stray has to
            // answer it too: a dialog template begins `ff ff` and a class
            // number, which reads as two fullwidth characters in front of the
            // string, and that reading is two characters longer than the one
            // that is right.
            if u & 0xff < 0x20 || !text_page((u >> 8) as u8) {
                return false;
            }
            stray += 1;
        }
        units += 1;
        i += 2;
    }
    units > 0 && stray <= units / 8
}
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
/// Whether a part of Unicode is one a run of text is written in.
///
/// Everything above the surrogates is private use, compatibility forms,
/// halfwidth and fullwidth forms, and specials. Nobody writes a string in
/// those, and every one of them is where sixteen-bit numbers land: eight-bit
/// audio at low amplitude has a high byte of 0xff, and read two bytes at a
/// time that is a run of fullwidth punctuation, coherent and printable and not
/// a string. Measured on a recording of a bat, this is what the last three
/// hundred false readings had in common, and every one of them had a zero
/// after it as well, since a silent sample is two zero bytes.
///
/// The cost is a run written entirely in fullwidth forms or halfwidth katakana
/// and in nothing else, which goes the way CJK-only text goes and for the same
/// reason. A run with any ordinary text in it keeps its page and is found.
pub(super) fn text_page(page: u8) -> bool {
    page < 0xd8
}
/// The Unicode page most of a run's characters are from.
pub(super) fn dominant_page(buf: &[u8], start: usize, end: usize, big: bool) -> Option<u8> {
    let mut pages = [0u32; 256];
    let mut i = start;
    while i + 2 <= end {
        pages[(unit_at(buf, i, big).unwrap_or(0) >> 8) as usize] += 1;
        i += 2;
    }
    let (page, n) = pages.iter().enumerate().max_by_key(|(_, n)| **n)?;
    (*n > 0).then_some(page as u8)
}
/// Whether a wide run's characters are characters, or a column of small
/// numbers under a constant high byte.
///
/// A Thrift field header, a table of flags and a run of enum values all read
/// as wide text with the same page over and over and a low byte that never
/// leaves the control range: `15 10 15 04 15 06 15 08` is four perfectly good
/// Canadian Syllabics characters and is a Parquet record. In a script, the low
/// byte moves through the block: the letters are spread across it, and a word
/// of four cannot have all four sitting in the first thirty-two places.
pub(super) fn letters(buf: &[u8], start: usize, end: usize, big: bool) -> bool {
    let mut i = start;
    while i + 2 <= end {
        if unit_at(buf, i, big).unwrap_or(0) & 0xff >= 0x20 {
            return true;
        }
        i += 2;
    }
    false
}
/// Whether a wide run says more than one thing.
///
/// A stretch of 90 90 90 90 is x86 padding and reads as a row of the same
/// character; so does a run of zeroes in a table, or a fill byte in a disk
/// image. None of them is a string, and every one of them passes every other
/// test here, because one character repeated is perfectly coherent. Real text
/// spreads itself: no character in a line of it takes two thirds of the line,
/// and even the shortest line has three different characters in it.
pub(super) fn varied(buf: &[u8], start: usize, end: usize, big: bool) -> bool {
    let mut seen: std::collections::HashMap<u16, u32> = std::collections::HashMap::new();
    let mut units = 0u32;
    let mut most = 0u32;
    let mut i = start;
    while i + 2 <= end {
        let n = seen.entry(unit_at(buf, i, big).unwrap_or(0)).or_insert(0);
        *n += 1;
        most = most.max(*n);
        units += 1;
        i += 2;
    }
    // Three different characters, which every string of four has and an
    // alternating pair of numbers does not: `01 00 00 01 01 00 00 01` reads as
    // two characters taking turns, and that is a table, not a word.
    units > 0 && most * 3 <= units * 2 && (seen.len() >= 3 || units < 3)
}
/// Whether a run of wide characters is one, or eight-bit text read two bytes
/// at a time.
///
/// Every English sentence in a file is also a run of perfectly good CJK when
/// the bytes are taken in pairs, so something has to tell the two apart, and
/// it is the high byte of each unit. A UTF-16 character of a Latin, Greek,
/// Cyrillic, Hebrew, Arabic or Indic script has a high byte below 0x20, and
/// half of a surrogate pair has one above 0x7f. Eight-bit text read as wide
/// characters has a printable byte in both halves of every unit. Half the
/// units decide it, so one odd pair in a run does not lose the run.
///
/// The cost is that a file whose wide text is nothing but CJK or kana is not
/// found this way: those live where a pair of printable bytes lives, and
/// nothing in the bytes says which reading was meant. Such a file reads in the
/// text view with UTF-16 chosen by hand.
pub(super) fn wide_enough(buf: &[u8], start: usize, end: usize, big: bool) -> bool {
    let high = usize::from(!big);
    let mut wide = 0usize;
    let mut units = 0usize;
    let mut i = start;
    while i + 2 <= end {
        units += 1;
        if !ascii_text(buf[i + high]) {
            wide += 1;
        }
        i += 2;
    }
    units > 0 && wide * 2 >= units
}
/// The character at `i`, read as UTF-8, and how many bytes it took.
pub(super) fn utf8_char(buf: &[u8], i: usize) -> Option<(char, usize)> {
    let b = *buf.get(i)?;
    if b < 0x80 {
        return Some((b as char, 1));
    }
    let n = match b {
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return None,
    };
    let s = buf.get(i..i + n)?;
    let text = std::str::from_utf8(s).ok()?;
    let c = text.chars().next()?;
    Some((c, n))
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
pub(super) fn unit_at(buf: &[u8], i: usize, big: bool) -> Option<u16> {
    let pair = buf.get(i..i + 2)?;
    Some(if big { u16::from_be_bytes([pair[0], pair[1]]) } else { u16::from_le_bytes([pair[0], pair[1]]) })
}
pub(super) fn is_high_surrogate(u: u16) -> bool {
    (0xd800..0xdc00).contains(&u)
}
pub(super) fn is_low_surrogate(u: u16) -> bool {
    (0xdc00..0xe000).contains(&u)
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
/// Characters, code units and how many surrogates went unpaired, over a range
/// already known to be text.
pub(super) fn measure(buf: &[u8], start: usize, end: usize, enc: Enc) -> (u32, u32, u32) {
    if !enc.wide() {
        let mut chars = 0u32;
        let mut i = start;
        while i < end {
            match utf8_char(buf, i) {
                Some((_, n)) if i + n <= end => {
                    chars += 1;
                    i += n;
                }
                _ => {
                    chars += 1;
                    i += 1;
                }
            }
        }
        return (chars, (end - start) as u32, 0);
    }
    let big = enc.big();
    let mut chars = 0u32;
    let mut lone = 0u32;
    let mut i = start;
    while i + 2 <= end {
        let u = unit_at(buf, i, big).unwrap_or(0);
        if is_high_surrogate(u) && i + 4 <= end && unit_at(buf, i + 2, big).is_some_and(is_low_surrogate) {
            chars += 1;
            i += 4;
            continue;
        }
        if is_high_surrogate(u) || is_low_surrogate(u) {
            lone += 1;
        }
        chars += 1;
        i += 2;
    }
    (chars, ((end - start) / 2) as u32, lone)
}
/// The text of a range, with an unpaired surrogate standing as U+FFFD. It is
/// the one character that cannot come out of a valid string, so it says
/// "something was here that this is not" without pretending to be it.
pub(super) fn decode(bytes: &[u8], enc: Enc) -> String {
    if !enc.wide() {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let big = enc.big();
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|p| if big { u16::from_be_bytes([p[0], p[1]]) } else { u16::from_le_bytes([p[0], p[1]]) })
        .collect();
    char::decode_utf16(units).map(|r| r.unwrap_or('\u{fffd}')).collect()
}
