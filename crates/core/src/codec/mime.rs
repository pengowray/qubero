//! The ways MIME writes bytes into a message that may only carry short lines
//! of ASCII: base64 and quoted-printable, RFC 2045 sections 6.8 and 6.7, and
//! the Q encoding RFC 2047 writes non-ASCII header text in, which is
//! quoted-printable with `_` for a space.
//!
//! Neither is compression. Base64 spells three bytes in four characters out of
//! sixty-four, so an attachment comes out three quarters the size it went in.
//! Quoted-printable leaves text that is already ASCII as it is and writes any
//! other byte as `=` and two hexadecimal digits, so a Latin-1 `é` is `=E9`,
//! and it breaks a long line with `=` at the end of it, which the decoder
//! takes out again.
//!
//! What they map is exact, and the trace says it: each base64 group of four
//! characters made the three bytes after the ones before it, each `=E9` made
//! one byte, and a run of text copied through is one step. What the decoder
//! reads past and writes nothing for, which is the line breaks between base64
//! lines, a soft line break, and the blanks a quoted-printable line may carry
//! at its end, is a step of its own that writes nothing.

use crate::codec::{frames, Refusal, StepKind, Trace, TraceBuilder};

/// Base64 as MIME writes it: four characters of `A-Z a-z 0-9 + /` to every
/// three bytes, `=` making up the last group when the bytes do not divide by
/// three, and a line break every 76 characters.
///
/// Blanks and line breaks anywhere between characters are read past, as RFC
/// 2045 says they are. Any other byte outside the alphabet is refused rather
/// than read past, though the RFC allows a decoder to drop those too: a
/// character that is not base64 in a base64 part is damage, and a run that
/// will not decode is shown as the bytes it is with a note saying so, where
/// one decoded with the damage dropped would look whole. So are characters
/// after the `=` that ends the data, and a last group of a single character,
/// which spells less than a byte. A last group of two or three characters with
/// its `=` left off is read as the one or two bytes it spells.
pub fn base64(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    let mut out = Vec::with_capacity(data.len() / 4 * 3);
    let mut steps = Steps::default();
    let mut i = 0;
    let mut ended = false;
    while i < data.len() {
        if is_blank(data[i]) {
            let start = i;
            while i < data.len() && is_blank(data[i]) {
                i += 1;
            }
            steps.saw(start, out.len(), StepKind::Ignored);
            continue;
        }
        if ended {
            return Err(Refusal::Failed);
        }
        // One group: four characters, or what is left of them, with any
        // blanks between them read as part of it. Blanks after its last
        // character are not, so they are a step of their own.
        let start = i;
        let (mut bits, mut n, mut pad, mut end) = (0u32, 0, 0, i);
        while n + pad < 4 && i < data.len() {
            let c = data[i];
            i += 1;
            if is_blank(c) {
                continue;
            }
            end = i;
            if c == b'=' {
                // Padding stands for the third and fourth characters only.
                if n < 2 {
                    return Err(Refusal::Failed);
                }
                pad += 1;
                continue;
            }
            if pad > 0 {
                return Err(Refusal::Failed);
            }
            bits = bits << 6 | u32::from(sextet(c).ok_or(Refusal::Failed)?);
            n += 1;
        }
        i = end;
        if n < 2 || (pad > 0 && n + pad < 4) {
            return Err(Refusal::Failed);
        }
        // Two characters are twelve bits and one byte, three are eighteen and
        // two; the bits past the last whole byte are the encoder's padding.
        let len = n - 1;
        let value = bits >> (6 * n - 8 * len);
        steps.saw(start, out.len(), StepKind::Base64 { value, len: len as u8 });
        for k in (0..len).rev() {
            out.push((value >> (8 * k)) as u8);
        }
        ended = pad > 0 || n < 4;
    }
    let trace = steps.done(data.len(), out.len());
    Ok((out, trace))
}

/// Quoted-printable, as RFC 2045 writes it: every byte as itself except `=`,
/// which starts `=XX` for the byte numbered XX in hexadecimal, or a soft line
/// break when it is the last thing on its line.
///
/// Read the way the RFC asks a robust decoder to read it. Blanks at the end of
/// a line are padding a mail system added and are dropped, and so are the
/// blanks after a soft break's `=`. Lower-case hexadecimal is read as upper
/// case is. A `=` that starts neither an escape nor a soft break is kept as
/// the `=` it is, since a message whose sender wrote one unencoded is far more
/// common than one that means something else by it. Line breaks are copied
/// through as they are written, so a file with bare `\n` comes out with bare
/// `\n`. Nothing is refused.
pub fn quoted_printable(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    let mut out = Vec::with_capacity(data.len());
    let mut steps = Steps::default();
    let mut i = 0;
    while i < data.len() {
        let c = data[i];
        if c == b'=' {
            let after = blanks_from(data, i + 1);
            if let Some(end) = line_break_at(data, after) {
                steps.saw(i, out.len(), StepKind::Ignored);
                i = end;
                continue;
            }
            if let (Some(hi), Some(lo)) = (data.get(i + 1).and_then(|h| hex(*h)), data.get(i + 2).and_then(|l| hex(*l))) {
                let byte = hi << 4 | lo;
                steps.saw(i, out.len(), StepKind::Literal(byte));
                out.push(byte);
                i += 3;
                continue;
            }
        }
        if is_space(c) {
            let end = blanks_from(data, i);
            if line_break_at(data, end).is_some() {
                steps.saw(i, out.len(), StepKind::Ignored);
                i = end;
                continue;
            }
            steps.copy(i, out.len());
            out.extend_from_slice(&data[i..end]);
            i = end;
            continue;
        }
        steps.copy(i, out.len());
        out.push(c);
        i += 1;
    }
    let trace = steps.done(data.len(), out.len());
    Ok((out, trace))
}

/// The Q encoding of an RFC 2047 encoded word, the `Q` in
/// `=?iso-8859-1?Q?caf=E9_night?=`: quoted-printable without line breaks of
/// any kind, and with `_` standing for a space, since a space would end the
/// word.
///
/// Read as leniently as [`quoted_printable`] is: a `=` that starts no escape
/// is kept as the `=` it is, and nothing is refused.
pub fn q_encoding(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    let mut out = Vec::with_capacity(data.len());
    let mut steps = Steps::default();
    let mut i = 0;
    while i < data.len() {
        let c = data[i];
        if c == b'=' {
            if let (Some(hi), Some(lo)) = (data.get(i + 1).and_then(|h| hex(*h)), data.get(i + 2).and_then(|l| hex(*l))) {
                let byte = hi << 4 | lo;
                steps.saw(i, out.len(), StepKind::Literal(byte));
                out.push(byte);
                i += 3;
                continue;
            }
        }
        if c == b'_' {
            steps.saw(i, out.len(), StepKind::Literal(b' '));
            out.push(b' ');
        } else {
            steps.copy(i, out.len());
            out.push(c);
        }
        i += 1;
    }
    let trace = steps.done(data.len(), out.len());
    Ok((out, trace))
}

/// The six bits a base64 character stands for.
fn sextet(c: u8) -> Option<u8> {
    match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

fn hex(c: u8) -> Option<u8> {
    (c as char).to_digit(16).map(|d| d as u8)
}

/// What base64 reads past: blanks and line breaks.
fn is_blank(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n')
}

/// What a quoted-printable line may carry at its end: blanks, and not line
/// breaks.
fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t')
}

/// Where the run of blanks starting at `i` ends.
fn blanks_from(data: &[u8], mut i: usize) -> usize {
    while i < data.len() && is_space(data[i]) {
        i += 1;
    }
    i
}

/// Where a line break at `i` ends, with the end of the run counting as one:
/// the last line of a part has no break after it, the one before the boundary
/// being the boundary's.
fn line_break_at(data: &[u8], i: usize) -> Option<usize> {
    match (data.get(i), data.get(i + 1)) {
        (None, _) => Some(i),
        (Some(b'\n'), _) => Some(i + 1),
        (Some(b'\r'), Some(b'\n')) => Some(i + 2),
        _ => None,
    }
}

/// The trace of a decoding that reads text a few characters at a time.
///
/// Characters copied through as themselves are run together into one step,
/// since a step apiece would be a map the size of the message. Past the step
/// budget the map is given up and the run gets the one-step trace, the way
/// `pytext` gives it up.
#[derive(Default)]
struct Steps {
    builder: TraceBuilder,
    /// Whether the step being built is a run of characters copied through,
    /// which the next one joins rather than starting one of its own.
    copying: bool,
    given_up: bool,
}

impl Steps {
    /// A step of its own, starting at byte `read` of the run and byte `wrote`
    /// of what comes out.
    fn saw(&mut self, read: usize, wrote: usize, kind: StepKind) {
        self.copying = false;
        if self.given_up {
            return;
        }
        if self.builder.over_budget() {
            self.given_up = true;
            return;
        }
        self.builder.push(read as u64 * 8, wrote as u64, kind);
    }

    /// A character copied through, which joins the run before it if there is
    /// one.
    fn copy(&mut self, read: usize, wrote: usize) {
        if !self.copying {
            self.saw(read, wrote, StepKind::Stored);
            self.copying = true;
        }
    }

    fn done(mut self, in_bytes: usize, out_bytes: usize) -> Trace {
        if self.given_up {
            return frames::whole(in_bytes, out_bytes);
        }
        self.builder.finish_at(in_bytes as u64 * 8, out_bytes as u64);
        self.builder.done()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_is_three_bytes_to_four_characters() {
        let (out, trace) = base64(b"aWQsY291bnQNCjEsMw0KMiw1DQo=").unwrap();
        assert_eq!(out, b"id,count\r\n1,3\r\n2,5\r\n");
        trace.check_tiles().unwrap();
        assert_eq!(trace.out_bytes(), out.len() as u64);
        // The first group is `aWQs` and spells `id,`.
        let first = trace.step(0).unwrap();
        assert_eq!(first.in_bits, 0..32);
        assert_eq!(first.out_bytes, 0..3);
        assert_eq!(first.kind, StepKind::Base64 { value: 0x69642c, len: 3 });
        // The last is `DQo=`, which is two bytes and a pad.
        let last = trace.map_out(out.len() as u64 - 1).unwrap();
        assert_eq!(last.in_bits, 24 * 8..28 * 8);
        assert_eq!(last.kind, StepKind::Base64 { value: 0x0d0a, len: 2 });
    }

    #[test]
    fn line_breaks_between_base64_lines_are_read_past() {
        let (out, trace) = base64(b"aGVs\r\nbG8=\r\n").unwrap();
        assert_eq!(out, b"hello");
        trace.check_tiles().unwrap();
        assert_eq!(trace.map_in(4 * 8).unwrap().kind, StepKind::Ignored);
        assert_eq!(trace.map_in(4 * 8).unwrap().out_bytes, 3..3);
        // The break after the padding is read past too, and is its own step.
        assert_eq!(trace.map_in(10 * 8).unwrap().kind, StepKind::Ignored);
        // A group split by a break holds the break.
        let (out, trace) = base64(b"aG\r\nVs").unwrap();
        assert_eq!(out, b"hel");
        assert_eq!(trace.len(), 1);
    }

    #[test]
    fn a_short_last_group_is_read_with_or_without_its_padding() {
        assert_eq!(base64(b"YQ==").unwrap().0, b"a");
        assert_eq!(base64(b"YWI=").unwrap().0, b"ab");
        assert_eq!(base64(b"YQ").unwrap().0, b"a");
        assert_eq!(base64(b"YWI").unwrap().0, b"ab");
        assert_eq!(base64(b"").unwrap().0, b"");
    }

    #[test]
    fn base64_that_is_damaged_is_refused() {
        // A character outside the alphabet, one character too few to spell a
        // byte, padding in the wrong place, and more after the end.
        assert_eq!(base64(b"aGVs*G8=").err(), Some(Refusal::Failed));
        assert_eq!(base64(b"aGVsb").err(), Some(Refusal::Failed));
        assert_eq!(base64(b"a===").err(), Some(Refusal::Failed));
        assert_eq!(base64(b"YQ=a").err(), Some(Refusal::Failed));
        assert_eq!(base64(b"YQ==YQ==").err(), Some(Refusal::Failed));
        assert_eq!(base64(b"YQ=").err(), Some(Refusal::Failed));
    }

    #[test]
    fn quoted_printable_writes_a_byte_as_its_hex_and_breaks_lines_softly() {
        let text = b"This post has a =\r\nbold word and a caf=E9.\r\n";
        let (out, trace) = quoted_printable(text).unwrap();
        assert_eq!(out, b"This post has a bold word and a caf\xe9.\r\n");
        trace.check_tiles().unwrap();
        // The soft break writes nothing and is its own step.
        let soft = trace.map_in(16 * 8).unwrap();
        assert_eq!(soft.kind, StepKind::Ignored);
        assert_eq!(soft.in_bits, 16 * 8..19 * 8);
        assert_eq!(soft.out_bytes, 16..16);
        // `=E9` is one byte, read from three.
        let e9 = trace.map_out(35).unwrap();
        assert_eq!(e9.kind, StepKind::Literal(0xe9));
        assert_eq!(e9.in_bits, 38 * 8..41 * 8);
        // The text around it is copied through in runs.
        assert_eq!(trace.map_out(0).unwrap().kind, StepKind::Stored);
        assert_eq!(trace.map_out(0).unwrap().out_bytes, 0..16);
    }

    #[test]
    fn the_q_encoding_writes_a_space_as_an_underscore() {
        let (out, trace) = q_encoding(b"The_numbers_for_caf=C3=A9_night").unwrap();
        assert_eq!(out, "The numbers for caf\u{e9} night".as_bytes());
        trace.check_tiles().unwrap();
        assert_eq!(trace.map_out(3).unwrap().kind, StepKind::Literal(b' '));
        assert_eq!(trace.map_out(3).unwrap().in_bits, 24..32);
        // A line break is text here, and a stray `=` is kept.
        assert_eq!(q_encoding(b"a=\r\nb=z").unwrap().0, b"a=\r\nb=z");
    }

    #[test]
    fn quoted_printable_drops_what_a_mail_system_added_and_keeps_a_stray_equals() {
        // Blanks at the end of a line, and after a soft break's `=`.
        assert_eq!(quoted_printable(b"a  \r\nb\t\nc =  \r\nd").unwrap().0, b"a\r\nb\nc d");
        // Blanks inside a line are text.
        assert_eq!(quoted_printable(b"a  b").unwrap().0, b"a  b");
        // Lower-case hex, a `=` that is neither, and one at the very end.
        assert_eq!(quoted_printable(b"caf=e9").unwrap().0, b"caf\xe9");
        assert_eq!(quoted_printable(b"x=y 1=2").unwrap().0, b"x=y 1=2");
        assert_eq!(quoted_printable(b"end=").unwrap().0, b"end");
        let (out, trace) = quoted_printable(b"").unwrap();
        assert!(out.is_empty());
        trace.check_tiles().unwrap();
    }
}
