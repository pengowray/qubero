//! What a character is, and whether a stretch of them is text.
//!
//! One question a byte or a code unit at a time, then the battery a whole
//! stretch has to answer: that it stays on one page of Unicode, that it holds
//! letters, that it is not the same character over and over. A wide run is
//! asked all of them, because two bytes that read as a character read as one
//! whether or not anybody wrote it. See [`super::runs`] for what does the
//! asking.

use super::*;

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
