//! Getting lines of text out of a file that is not necessarily bytes of text.
//!
//! A dump saved from a terminal has been through whatever the terminal and the
//! shell do to text on the way out. PowerShell writes UTF-16 with a byte-order
//! mark when it is told to. A capture that kept the colours has an escape
//! sequence around most of the bytes. A file written on Windows ends its lines
//! with two characters and one written anywhere else with one.
//!
//! None of that is what the dump says, so it comes off here, before anything
//! tries to read a column. What is left is a line of text and where in the file
//! it came from, because every digit that ends up standing for a byte has to be
//! able to say which bytes of the file it was written as.

use crate::template::Encoding;
use crate::text::{decode_settled, settle, Settled};

/// One line of the dump, with the escape sequences and the line ending gone.
pub struct Line {
    /// Where the line starts in the file, in bytes.
    pub at: u64,
    /// How many bytes of the file the line takes, its ending included.
    pub len: u64,
    /// The line itself.
    pub text: String,
    /// Where each character of `text` came from, when that is not simply `at`
    /// plus the character's index: a wider encoding, or an escape sequence
    /// that was stepped over. One entry per character, plus a last entry for
    /// the end, so a span of characters gives a span of bytes.
    pub origin: Option<Vec<u32>>,
}

impl Line {
    /// Where character `i` of the line came from. `i` may be the length of the
    /// line, which gives where the line's text ended.
    pub fn origin_of(&self, i: usize) -> u64 {
        match &self.origin {
            Some(map) => self.at + map[i.min(map.len() - 1)] as u64,
            None => self.at + i as u64,
        }
    }
}

/// How a run of bytes reads as text, settled the way a text field settles it.
/// A dump saved by a shell says nothing about its own encoding, so a mark is
/// taken when there is one and the bytes decide when there is not. Text that
/// is UTF-16 and carries no mark is not caught, because nothing distinguishes
/// it from bytes with a zero in every other place.
pub fn reading(head: &[u8]) -> (Settled, usize) {
    let enc = Encoding::Bom { fallback: Box::new(Encoding::Unknown) };
    let (s, mark, _) = settle(&enc, head);
    (s, mark)
}

/// The line endings, in the order they are looked for. A lone carriage return
/// is a line ending too: it is what a terminal writes when it draws over the
/// line it is on, and a capture of one keeps it.
fn ends_line(c: char) -> bool {
    c == '\n' || c == '\r'
}

/// Split `bytes` into lines, reporting them at `base` plus their offset.
///
/// `bytes` is the whole of what is being read, from `base`, so a line that runs
/// past the end of it is still returned: the caller decides whether to trust a
/// last line that may have been cut in half.
pub fn split(settled: Settled, mark: usize, bytes: &[u8], base: u64) -> Vec<Line> {
    let mut out = Vec::new();
    let mut at = mark;
    while at < bytes.len() {
        let end = line_end(settled, bytes, at);
        let after = skip_ending(settled, bytes, end);
        let (raw, _) = decode_settled(settled, &bytes[at..end]);
        let (text, origin) = strip_escapes(&raw, settled);
        out.push(Line { at: base + at as u64, len: (after - at) as u64, text, origin });
        at = after;
    }
    out
}

/// Where the line's text stops, which is the first line ending or the end of
/// what was read.
fn line_end(settled: Settled, bytes: &[u8], from: usize) -> usize {
    let unit = settled.unit();
    let mut at = from;
    while at + unit <= bytes.len() {
        let (s, _) = decode_settled(settled, &bytes[at..at + unit]);
        if s.chars().next().is_some_and(ends_line) {
            return at;
        }
        at += unit;
    }
    bytes.len()
}

/// Step over the line ending, which is one character or the two a carriage
/// return and a line feed make together.
fn skip_ending(settled: Settled, bytes: &[u8], end: usize) -> usize {
    let unit = settled.unit();
    if end + unit > bytes.len() {
        return bytes.len();
    }
    let first = char_at(settled, bytes, end);
    let mut at = end + unit;
    if first == Some('\r') && at + unit <= bytes.len() && char_at(settled, bytes, at) == Some('\n') {
        at += unit;
    }
    at
}

fn char_at(settled: Settled, bytes: &[u8], at: usize) -> Option<char> {
    let unit = settled.unit();
    if at + unit > bytes.len() {
        return None;
    }
    let (s, _) = decode_settled(settled, &bytes[at..at + unit]);
    s.chars().next()
}

/// Remove ANSI escape sequences, keeping a note of where every character that
/// survived came from.
///
/// Three shapes are recognised, which is all a dump ever carries: a control
/// sequence (`ESC [` up to a byte from `@` to `~`), an operating system
/// command (`ESC ]` up to a bell or a string terminator), and the two-character
/// escapes that are neither. Anything else after an `ESC` is left alone rather
/// than guessed at, because a dump whose text column happens to hold an escape
/// byte has written it as a dot.
///
/// `unit` is how many bytes one character of the source took, which is what
/// turns a character index back into a file offset.
fn strip_escapes(raw: &str, settled: Settled) -> (String, Option<Vec<u32>>) {
    let simple = settled.unit() == 1 && raw.is_ascii() && !raw.contains('\u{1b}');
    if simple {
        return (raw.to_string(), None);
    }
    let mut text = String::with_capacity(raw.len());
    let mut origin = Vec::new();
    let mut chars = raw.chars().peekable();
    // Bytes of the file consumed so far. A decoded string's own offsets are not
    // the file's: one character of UTF-16 is two bytes and one of CP437 is one.
    let mut consumed = 0usize;
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            origin.push(consumed as u32);
            consumed += source_width(settled, c);
            text.push(c);
            continue;
        }
        consumed += source_width(settled, c);
        match chars.peek().copied() {
            Some('[') => {
                consumed += source_width(settled, chars.next().unwrap());
                while let Some(c) = chars.next() {
                    consumed += source_width(settled, c);
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                consumed += source_width(settled, chars.next().unwrap());
                while let Some(c) = chars.next() {
                    consumed += source_width(settled, c);
                    if c == '\u{7}' {
                        break;
                    }
                    if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                        consumed += source_width(settled, chars.next().unwrap());
                        break;
                    }
                }
            }
            Some(_) => consumed += source_width(settled, chars.next().unwrap()),
            None => {}
        }
    }
    origin.push(consumed as u32);
    (text, Some(origin))
}

/// How many bytes of the file one character took, in the encoding it was read
/// in. This is what a character index costs when it is turned back into a place
/// in the file.
fn source_width(settled: Settled, c: char) -> usize {
    match settled {
        Settled::Utf8 => c.len_utf8(),
        Settled::Utf16(_) => c.len_utf16() * 2,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_lines_need_no_map() {
        let bytes = b"one\ntwo\r\nthree";
        let lines = split(Settled::Utf8, 0, bytes, 0);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].text, "one");
        assert_eq!(lines[1].text, "two");
        assert_eq!(lines[1].at, 4);
        assert_eq!(lines[1].len, 5);
        assert_eq!(lines[2].text, "three");
        assert!(lines[0].origin.is_none());
    }

    #[test]
    fn escapes_come_off_and_say_where_the_rest_was() {
        let bytes = b"\x1b[1;31m00\x1b[0m ab";
        let lines = split(Settled::Utf8, 0, bytes, 0);
        assert_eq!(lines[0].text, "00 ab");
        // The two zeros were written seven bytes in, after the colour.
        assert_eq!(lines[0].origin_of(0), 7);
        assert_eq!(lines[0].origin_of(2), 13);
    }

    #[test]
    fn a_wider_encoding_counts_in_its_own_units() {
        let mut bytes = vec![0xff, 0xfe];
        for c in "ab\ncd".chars() {
            bytes.push(c as u8);
            bytes.push(0);
        }
        let (settled, mark) = reading(&bytes);
        assert_eq!(settled, Settled::Utf16(crate::template::Endian::Little));
        let lines = split(settled, mark, &bytes, 0);
        assert_eq!(lines[0].text, "ab");
        assert_eq!(lines[1].text, "cd");
        assert_eq!(lines[1].at, 2 + 6);
    }
}
