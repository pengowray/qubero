//! Reading a file as the text it is, a screenful at a time.
//!
//! The hex grid answers "what is at this address" and the listing answers "how
//! is this file put together". Neither reads a file the way it was written to
//! be read, and plenty of files were written to be read: a log, a source file,
//! a manifest, a terminal captured to disk, a hex dump somebody pasted. This
//! is the view for those, and it is the same discipline as every other view
//! here. The file is not loaded. A window of it is read, turned into lines,
//! and handed over; the next window is asked for when it is wanted.
//!
//! Four things about a text file are not written down in it, and all four have
//! come up already in [`hexdump`](crate::hexdump):
//!
//! * **Which encoding.** A byte-order mark settles it where there is one; where
//!   there is not, the bytes decide and the answer says it was a guess. The
//!   same [`text::settle`](crate::text::settle) the rest of the crate uses.
//! * **Which line ending.** A file may use one, the other, both, or a lone
//!   carriage return, which is what a terminal writes when it draws over the
//!   line it is on. Each line says which it ended with rather than the file
//!   claiming one.
//! * **Where a line stops when it does not.** A minified file is one line of
//!   two gigabytes and a view that waits for the end of it never draws. So a
//!   line is cut at [`MAX_LINE`] and says it was cut, and what follows carries
//!   on from there.
//! * **What an escape sequence is.** A capture of a coloured terminal is full
//!   of them. They are neither dropped nor shown as gibberish: each line says
//!   which stretches of it are escapes, so a view can paint them, dim them or
//!   spell them out, and none of that is decided here.
//!
//! Scrolling is by byte offset rather than by line number, for the reason the
//! listing already has: nothing can say how many lines a file has without
//! reading all of it. [`back`] walks the other way by looking for endings in a
//! window before the position, which is the only way back through a file whose
//! lines are not a fixed length.

use crate::source::{Missing, Source};
use crate::template::Encoding;
use crate::text::{decode_settled, settle, Settled};

/// Bytes read in one go, going forwards or back. A screenful of text is far
/// less than this; the size is what makes a step over a long line finish.
pub const WINDOW: u64 = 64 * 1024;

/// The longest a line is allowed to be before it is cut and said to be cut.
/// A minified file, a base64 blob on one line, or a file with no endings at
/// all is one line as long as the file, and a view that waits for the end of
/// it never draws.
pub const MAX_LINE: u64 = 4096;

/// How a file's text was settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reading {
    pub settled: Settled,
    /// Bytes of a byte-order mark at the front. They belong to the file and
    /// not to the text, so the first line starts after them.
    pub mark: usize,
    /// True when nothing in the file said which encoding it was and the bytes
    /// were asked instead.
    pub guessed: bool,
}

impl Reading {
    /// Bytes one character takes at least, which is what a line start has to
    /// be a multiple of: half a UTF-16 code unit is not a place in the text.
    pub fn unit(self) -> u64 {
        self.settled.unit() as u64
    }

    /// Round a byte offset down to somewhere a character could start.
    pub fn align(self, at: u64) -> u64 {
        let mark = self.mark as u64;
        if at <= mark {
            return mark;
        }
        mark + (at - mark) / self.unit() * self.unit()
    }
}

/// Settle how a file reads from its first bytes.
pub fn reading(head: &[u8]) -> Reading {
    let enc = Encoding::Bom { fallback: Box::new(Encoding::Unknown) };
    let (settled, mark, _) = settle(&enc, head);
    Reading { settled, mark, guessed: mark == 0 }
}

/// Settle how a file reads, having been told.
pub fn reading_as(settled: Settled, head: &[u8]) -> Reading {
    let mark = match (settled, head) {
        (Settled::Utf8, [0xef, 0xbb, 0xbf, ..]) => 3,
        (Settled::Utf16(_), [0xff, 0xfe, ..] | [0xfe, 0xff, ..]) => 2,
        _ => 0,
    };
    Reading { settled, mark, guessed: false }
}

/// How a line ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    /// The end of the file, with nothing after the last character.
    None,
    Lf,
    Cr,
    CrLf,
    /// The line was cut at [`MAX_LINE`] rather than ended. What follows is the
    /// rest of the same line.
    Cut,
}

impl Ending {
    /// Bytes the ending takes.
    pub fn bytes(self, unit: u64) -> u64 {
        match self {
            Ending::None | Ending::Cut => 0,
            Ending::Lf | Ending::Cr => unit,
            Ending::CrLf => unit * 2,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Ending::None => "no ending",
            Ending::Lf => "LF",
            Ending::Cr => "CR",
            Ending::CrLf => "CRLF",
            Ending::Cut => "cut",
        }
    }
}

/// One line of the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextLine {
    /// Where the line's first character is, in bytes.
    pub at: u64,
    /// Bytes the line takes, its ending included.
    pub len: u64,
    pub ending: Ending,
    /// The line's characters, escape sequences left in.
    pub text: String,
    /// Stretches of `text` that are escape sequences, as a character index and
    /// a length. Left in rather than removed, because a capture of a coloured
    /// terminal is a file whose escapes are part of what is in it.
    pub escapes: Vec<(u32, u32)>,
    /// True when the bytes did not fit the encoding and the text is a repair.
    pub lossy: bool,
}

impl TextLine {
    /// Where the line's ending starts, which is where its text stops.
    pub fn text_end(&self, unit: u64) -> u64 {
        self.at + self.len - self.ending.bytes(unit)
    }
}

/// A window of lines, and what it needed to answer.
#[derive(Debug, Clone, Default)]
pub struct Window {
    pub lines: Vec<TextLine>,
    /// Chunks this step needs before it can answer. When this is not empty the
    /// lines are whatever was readable and the caller should fetch and re-ask.
    pub missing: Vec<Missing>,
    /// Where the line after the last one starts.
    pub next: u64,
}

/// Read up to `want` lines starting at `from`, which must be where a line
/// starts. [`line_start`] finds that from any offset.
pub fn window<S: Source>(src: &S, reading: Reading, from: u64, want: usize) -> Window {
    let len = src.len_bytes();
    let mut out = Window { next: from.min(len), ..Default::default() };
    let unit = reading.unit();
    let mut at = reading.align(from).min(len);
    while out.lines.len() < want && at < len {
        let take = WINDOW.min(len - at);
        // One unit of lookahead past the window, so a CRLF straddling its edge
        // is read as the one ending it is and not as a CR then an LF.
        let read = (take + unit).min(len - at);
        let mut buf = vec![0u8; read as usize];
        let missing = src.read_bytes(at, &mut buf);
        if !missing.is_empty() {
            out.missing = missing;
            return out;
        }
        let mut used = 0u64;
        while out.lines.len() < want && used < take {
            let line = one_line(reading, &buf[used as usize..], at + used, at + read >= len);
            let step = line.len;
            used += step;
            out.next = line.at + line.len;
            out.lines.push(line);
            if step == 0 {
                return out;
            }
        }
        // A line that ran to the end of the window may go on past it, so the
        // next window starts where the last whole line stopped.
        at = out.next;
        if used == 0 {
            break;
        }
        let _ = unit;
    }
    out
}

/// Where the line holding `at` starts.
pub fn line_start<S: Source>(src: &S, reading: Reading, at: u64) -> (u64, Vec<Missing>) {
    let unit = reading.unit();
    let base = reading.mark as u64;
    let at = reading.align(at.min(src.len_bytes()));
    if at <= base {
        return (base, Vec::new());
    }
    let from = at.saturating_sub(MAX_LINE).max(base);
    let n = at - from;
    // One unit of lookahead past `at`, so a CR right before it is told apart
    // from the front half of a CRLF. A CRLF is one ending, and the place
    // between its bytes is not a line start.
    let read = (n + unit).min(src.len_bytes() - from);
    let mut buf = vec![0u8; read as usize];
    let missing = src.read_bytes(from, &mut buf);
    if !missing.is_empty() {
        return (at, missing);
    }
    // The last ending before `at` is where this line began. Nothing found
    // inside a window this long means the line is longer than one is allowed
    // to be, so it began where the cut before it left off.
    let mut best = from;
    let mut i = 0u64;
    while i + unit <= n {
        if let Some(end) = ending_at(reading, &buf[i as usize..]) {
            let after = i + end.bytes(unit);
            if from + after <= at {
                best = from + after;
            }
            i = after.max(i + unit);
        } else {
            i += unit;
        }
    }
    (best, Vec::new())
}

/// Walk back `lines` line starts from `at`.
pub fn back<S: Source>(src: &S, reading: Reading, at: u64, lines: usize) -> (u64, Vec<Missing>) {
    let mut here = at;
    for _ in 0..lines {
        let base = reading.mark as u64;
        if here <= base {
            return (base, Vec::new());
        }
        let (start, missing) = line_start(src, reading, here);
        if !missing.is_empty() {
            return (here, missing);
        }
        if start == here {
            // Already at a line start, so step back over the ending before it.
            let (prev, missing) = line_start(src, reading, here - reading.unit());
            if !missing.is_empty() {
                return (here, missing);
            }
            here = prev;
        } else {
            here = start;
        }
    }
    (here, Vec::new())
}

/// The character at the front of `buf`, where there is a whole one.
fn char_at(reading: Reading, buf: &[u8]) -> Option<char> {
    let unit = reading.settled.unit();
    if buf.len() < unit {
        return None;
    }
    let (s, _) = decode_settled(reading.settled, &buf[..unit]);
    s.chars().next()
}

/// The line ending starting at the front of `buf`, if one does.
fn ending_at(reading: Reading, buf: &[u8]) -> Option<Ending> {
    let unit = reading.settled.unit();
    match char_at(reading, buf) {
        Some('\n') => Some(Ending::Lf),
        Some('\r') => match char_at(reading, buf.get(unit..).unwrap_or(&[])) {
            Some('\n') => Some(Ending::CrLf),
            _ => Some(Ending::Cr),
        },
        _ => None,
    }
}

/// Read one line from the front of `buf`, which starts at `at`.
///
/// `last` says whether `buf` runs to the end of the file, which is what tells
/// a line with no ending apart from one whose ending is past the window.
fn one_line(reading: Reading, buf: &[u8], at: u64, last: bool) -> TextLine {
    let unit = reading.settled.unit();
    let mut i = 0usize;
    let mut ending = Ending::None;
    while i + unit <= buf.len() {
        if let Some(e) = ending_at(reading, &buf[i..]) {
            ending = e;
            break;
        }
        if i as u64 >= MAX_LINE {
            ending = Ending::Cut;
            break;
        }
        i += unit;
    }
    if i + unit > buf.len() && !last && ending == Ending::None {
        // The line runs past what was read. Cut it here rather than claim an
        // end the file has not reached.
        ending = Ending::Cut;
    }
    let (raw, lossy) = decode_settled(reading.settled, &buf[..i]);
    let (text, escapes) = escapes_in(&raw);
    let len = i as u64 + ending.bytes(unit as u64);
    TextLine { at, len, ending, text, escapes, lossy }
}

/// Where the escape sequences in a line are, as character indices.
///
/// Three shapes, which is all a captured terminal carries: a control sequence
/// (`ESC [` up to a byte from `@` to `~`), an operating system command (`ESC ]`
/// up to a bell or a string terminator), and the two-character escapes that
/// are neither. An `ESC` followed by nothing recognised is one character on
/// its own rather than a guess about what came after it.
fn escapes_in(raw: &str) -> (String, Vec<(u32, u32)>) {
    if !raw.contains('\u{1b}') {
        return (raw.to_string(), Vec::new());
    }
    let chars: Vec<char> = raw.chars().collect();
    let mut spans = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] != '\u{1b}' {
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        match chars.get(i) {
            Some('[') => {
                i += 1;
                while i < chars.len() {
                    let c = chars[i];
                    i += 1;
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                i += 1;
                while i < chars.len() {
                    let c = chars[i];
                    i += 1;
                    if c == '\u{7}' {
                        break;
                    }
                    if c == '\u{1b}' && chars.get(i) == Some(&'\\') {
                        i += 1;
                        break;
                    }
                }
            }
            Some(_) => i += 1,
            None => {}
        }
        spans.push((start as u32, (i - start) as u32));
    }
    (raw.to_string(), spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::MemSource;
    use crate::template::Endian;

    fn src(s: &str) -> MemSource {
        MemSource(s.as_bytes().to_vec())
    }

    #[test]
    fn every_ending_is_read_as_the_one_it_is() {
        let s = src("unix\nwindows\r\nold mac\rlast");
        let r = reading(&s.0);
        let w = window(&s, r, 0, 10);
        let got: Vec<(&str, Ending)> = w.lines.iter().map(|l| (l.text.as_str(), l.ending)).collect();
        assert_eq!(
            got,
            vec![("unix", Ending::Lf), ("windows", Ending::CrLf), ("old mac", Ending::Cr), ("last", Ending::None)]
        );
        assert_eq!(w.lines[1].at, 5);
        assert_eq!(w.lines[1].len, 9);
    }

    #[test]
    fn a_byte_order_mark_belongs_to_the_file_and_not_to_the_line() {
        let mut bytes = vec![0xef, 0xbb, 0xbf];
        bytes.extend_from_slice(b"hello\nthere\n");
        let s = MemSource(bytes);
        let r = reading(&s.0);
        assert_eq!(r.mark, 3);
        assert!(!r.guessed);
        let w = window(&s, r, 0, 4);
        assert_eq!(w.lines[0].text, "hello");
        assert_eq!(w.lines[0].at, 3);
    }

    #[test]
    fn a_wider_encoding_steps_in_its_own_units() {
        let mut bytes = vec![0xff, 0xfe];
        for c in "ab\ncd\n".chars() {
            bytes.push(c as u8);
            bytes.push(0);
        }
        let s = MemSource(bytes);
        let r = reading(&s.0);
        assert_eq!(r.settled, Settled::Utf16(Endian::Little));
        let w = window(&s, r, 0, 4);
        assert_eq!(w.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), vec!["ab", "cd"]);
        assert_eq!(w.lines[1].at, 2 + 6);
        // A line start is never half a character.
        assert_eq!(r.align(9), 8);
    }

    #[test]
    fn a_line_with_no_end_in_sight_is_cut_and_says_so() {
        let long = "x".repeat((MAX_LINE * 2) as usize);
        let s = src(&long);
        let r = reading(&s.0);
        let w = window(&s, r, 0, 4);
        assert_eq!(w.lines[0].ending, Ending::Cut);
        assert_eq!(w.lines[0].len, MAX_LINE);
        assert_eq!(w.lines[1].at, MAX_LINE);
        assert!(w.lines.len() >= 2, "the rest of the line carries on");
    }

    #[test]
    fn an_escape_sequence_is_kept_and_pointed_at() {
        let s = src("\u{1b}[1;31mred\u{1b}[0m plain\n");
        let r = reading(&s.0);
        let w = window(&s, r, 0, 2);
        let l = &w.lines[0];
        assert_eq!(l.escapes, vec![(0, 7), (10, 4)]);
        assert_eq!(l.text.chars().count(), 20);
        // What is left once a view drops them is what a terminal would show.
        let shown: String = l
            .text
            .chars()
            .enumerate()
            .filter(|(i, _)| !l.escapes.iter().any(|(s, n)| *i >= *s as usize && *i < (*s + *n) as usize))
            .map(|(_, c)| c)
            .collect();
        assert_eq!(shown, "red plain");
    }

    #[test]
    fn the_way_back_lands_on_line_starts() {
        let s = src("one\ntwo\nthree\nfour\n");
        let r = reading(&s.0);
        assert_eq!(line_start(&s, r, 9).0, 8, "inside 'two' is the start of 'two'");
        assert_eq!(line_start(&s, r, 8).0, 8, "the start of a line is its own start");
        assert_eq!(back(&s, r, 14, 1).0, 8);
        assert_eq!(back(&s, r, 14, 2).0, 4);
        assert_eq!(back(&s, r, 14, 99).0, 0, "the front of the file and no further");
    }

    #[test]
    fn the_way_back_steps_a_crlf_in_one() {
        let s = src("one\r\ntwo\r\nthree\r\nfour");
        let r = reading(&s.0);
        assert_eq!(line_start(&s, r, 16).0, 10, "between the bytes of a CRLF is not a line start");
        assert_eq!(back(&s, r, 17, 1).0, 10);
        assert_eq!(back(&s, r, 17, 2).0, 5);
        assert_eq!(back(&s, r, 17, 3).0, 0);
        assert_eq!(back(&s, r, 21, 4).0, 0, "from the end of the file, four steps is the front");
    }

    #[test]
    fn a_crlf_straddling_the_read_window_is_one_ending() {
        // Lines of "a\n" up to two bytes short of the window, then "z\r\n" so
        // the CR is the window's last byte and the LF the first byte past it.
        let mut bytes = b"a\n".repeat((WINDOW as usize - 2) / 2);
        bytes.extend_from_slice(b"z\r\nend");
        let s = MemSource(bytes);
        let r = reading(&s.0);
        let w = window(&s, r, 0, usize::MAX);
        let last = &w.lines[w.lines.len() - 2..];
        let got: Vec<(&str, Ending)> = last.iter().map(|l| (l.text.as_str(), l.ending)).collect();
        assert_eq!(got, vec![("z", Ending::CrLf), ("end", Ending::None)]);
    }

    #[test]
    fn a_window_says_where_the_next_one_starts() {
        let s = src("one\ntwo\nthree\nfour\n");
        let r = reading(&s.0);
        let w = window(&s, r, 0, 2);
        assert_eq!(w.lines.len(), 2);
        assert_eq!(w.next, 8);
        let w2 = window(&s, r, w.next, 2);
        assert_eq!(w2.lines[0].text, "three");
    }

    #[test]
    fn bytes_that_are_not_text_are_read_as_latin_one_and_said_to_be_a_guess() {
        let s = MemSource(vec![0x80, 0x81, b'\n', 0x82]);
        let r = reading(&s.0);
        assert_eq!(r.settled, Settled::Latin1);
        assert!(r.guessed);
        let w = window(&s, r, 0, 4);
        assert_eq!(w.lines.len(), 2);
        assert_eq!(w.lines[0].text.chars().count(), 2);
    }
}
