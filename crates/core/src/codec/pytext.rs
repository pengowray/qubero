//! The two ways a pickle wrote a run of bytes before the format had an opcode
//! for one.
//!
//! Protocol 3 is where a pickle got a type for bytes. Below it CPython hands
//! the bytes to `_codecs.encode` as the text they spell in latin-1, and that
//! text is written UTF-8: a byte under 0x80 is itself and every byte above it
//! is two. So a protocol 2 NumPy array's numbers are in the file as text and
//! nowhere in it as numbers, and [`latin1_text`] is what turns the one back
//! into the other.
//!
//! Protocol 0 writes the same text as a line, and a line cannot hold a newline
//! or a backslash, so `raw-unicode-escape` escapes those before it goes out.
//! [`escaped_latin1_text`] undoes both layers at once, and [`unescape`] is the
//! one that undoes the outer one. The recogniser reads lines with that same
//! function, so what a line spells is worked out in one place rather than two.
//!
//! Neither is compression. Both are a spelling, and what they map is exact: a
//! byte of the result came from the one, two, six or ten bytes of the run that
//! spelled it, and the trace says which.

use crate::codec::{frames, Refusal, StepKind, Trace, TraceBuilder};

/// Undo the escaping `raw-unicode-escape` writes, a character at a time.
///
/// `each` is handed every character and how many bytes of the line spelled it,
/// which is one for a character written as itself, six for `\uXXXX` and ten
/// for `\UXXXXXXXX`. A byte under 0x100 is written as itself, so the line is
/// not UTF-8 and a byte above 0x7f stands for the character of that number.
///
/// Nothing for a line no pickler wrote. `pickle.py` escapes the five
/// characters that would break the line or the escaping itself and writes
/// nothing else as an escape, so a backslash that does not start `\u` or `\U`
/// is a file this has no reading of. CPython's `raw-unicode-escape` writes its
/// hexadecimal in lower case and Jython's in upper, so the line is held to one
/// case throughout rather than to either one.
pub fn unescape(line: &[u8], mut each: impl FnMut(char, usize)) -> Option<()> {
    let mut rest = line;
    let mut upper: Option<bool> = None;
    while let Some((first, tail)) = rest.split_first() {
        if *first != b'\\' {
            each(char::from(*first), 1);
            rest = tail;
            continue;
        }
        let wide = match tail.first()? {
            b'u' => 4,
            b'U' => 8,
            _ => return None,
        };
        let digits = std::str::from_utf8(tail.get(1..1 + wide)?).ok()?;
        if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let here = digits.bytes().any(|b| b.is_ascii_uppercase());
        if digits.bytes().any(|b| b.is_ascii_lowercase()) && here {
            return None;
        }
        match upper {
            Some(before) if before != here && digits.bytes().any(|b| b.is_ascii_alphabetic()) => return None,
            _ if digits.bytes().any(|b| b.is_ascii_alphabetic()) => upper = Some(here),
            _ => {}
        }
        each(char::from_u32(u32::from_str_radix(digits, 16).ok()?)?, 2 + wide);
        rest = tail.get(1 + wide..)?;
    }
    Some(())
}

/// The text a line spells, for a caller that wants it whole.
pub fn unescaped(line: &[u8]) -> Option<String> {
    let mut out = String::with_capacity(line.len());
    unescape(line, |c, _| out.push(c))?;
    Some(out)
}

/// How many characters a line spells, for a caller that wants the length and
/// not the text. What the recogniser checks an array's shape against.
pub fn unescaped_len(line: &[u8]) -> Option<usize> {
    let mut n = 0usize;
    unescape(line, |_, _| n += 1)?;
    Some(n)
}

/// Text whose characters are each one byte of what it stands for.
///
/// The run is UTF-8 and every character in it is under 0x100, which is what
/// latin-1 could have spelled. A character above that is a run this has no
/// bytes to read back, so it is refused rather than truncated.
pub fn latin1_text(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    let text = std::str::from_utf8(data).map_err(|_| Refusal::Failed)?;
    let mut out = Vec::with_capacity(text.len());
    let mut steps = Steps::default();
    for c in text.chars() {
        let byte = u8::try_from(u32::from(c)).map_err(|_| Refusal::Failed)?;
        steps.saw(c.len_utf8(), byte);
        out.push(byte);
    }
    let trace = steps.done(data.len(), &out);
    Ok((out, trace))
}

/// The same text written as a protocol 0 line: escaped, and then one byte a
/// character. Both layers come off here, so the bytes out are the bytes the
/// pickle stood for and the steps between them are the line's own.
pub fn escaped_latin1_text(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    let mut out = Vec::with_capacity(data.len());
    let mut steps = Steps::default();
    let mut wide = false;
    let read = unescape(data, |c, width| match u8::try_from(u32::from(c)) {
        Ok(byte) => {
            steps.saw(width, byte);
            out.push(byte);
        }
        // A character latin-1 never spelled. Noted rather than returned: the
        // walk hands characters over and has nowhere to put an answer.
        Err(_) => wide = true,
    });
    if read.is_none() || wide {
        return Err(Refusal::Failed);
    }
    let trace = steps.done(data.len(), &out);
    Ok((out, trace))
}

/// The trace of a decoding that writes one byte per character it reads.
///
/// A step a character, with the characters the run wrote as themselves run
/// together: a stretch of those is a stretch of bytes copied through, and a
/// step apiece would be a map the size of the file. Past the step budget the
/// map is given up and the run gets the one-step trace a codec that cannot say
/// more gets, which is still the truth about where the bytes came from.
#[derive(Default)]
struct Steps {
    builder: TraceBuilder,
    /// Bytes of the run read so far, and bytes written so far.
    read: usize,
    wrote: usize,
    /// Whether the step being built is a run of characters written as
    /// themselves, which the next such character joins rather than starting
    /// one of its own.
    copying: bool,
    given_up: bool,
}

impl Steps {
    fn saw(&mut self, width: usize, byte: u8) {
        if !self.given_up {
            if self.builder.over_budget() {
                self.given_up = true;
            } else if width == 1 {
                if !self.copying {
                    self.builder.push(self.read as u64 * 8, self.wrote as u64, StepKind::Stored);
                    self.copying = true;
                }
            } else {
                self.builder.push(self.read as u64 * 8, self.wrote as u64, StepKind::Literal(byte));
                self.copying = false;
            }
        }
        self.read += width;
        self.wrote += 1;
    }

    fn done(mut self, in_bytes: usize, out: &[u8]) -> Trace {
        if self.given_up {
            return frames::whole(in_bytes, out.len());
        }
        self.builder.finish_at(in_bytes as u64 * 8, out.len() as u64);
        self.builder.done()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin_1_text_is_one_byte_a_character() {
        // `caf\u{e9}` is five bytes of UTF-8 and four bytes of latin-1.
        let (out, trace) = latin1_text("caf\u{e9}".as_bytes()).unwrap();
        assert_eq!(out, b"caf\xe9");
        trace.check_tiles().unwrap();
        assert_eq!(trace.in_bits(), 5 * 8);
        assert_eq!(trace.out_bytes(), 4);
        // The three ASCII characters are one step and the two-byte one is its
        // own, since only that one is not a byte copied through.
        assert_eq!(trace.len(), 2);
        assert_eq!(trace.step(0).unwrap().in_bits, 0..24);
        assert_eq!(trace.step(0).unwrap().kind, StepKind::Stored);
        assert_eq!(trace.step(1).unwrap().in_bits, 24..40);
        assert_eq!(trace.step(1).unwrap().kind, StepKind::Literal(0xe9));
    }

    /// The map runs both ways: the byte a reader is standing on in the
    /// decoded space says which bytes of the run spelled it.
    #[test]
    fn a_decoded_byte_says_which_bytes_of_the_run_spelled_it() {
        let (_, trace) = latin1_text("\u{ff}a\u{80}".as_bytes()).unwrap();
        assert_eq!(trace.map_out(0).unwrap().in_bits, 0..16);
        assert_eq!(trace.map_out(1).unwrap().in_bits, 16..24);
        assert_eq!(trace.map_out(2).unwrap().in_bits, 24..40);
        assert_eq!(trace.map_in(20).unwrap().out_bytes, 1..2);
    }

    #[test]
    fn a_character_above_latin_1_has_no_byte_to_read_back() {
        assert_eq!(latin1_text("\u{100}".as_bytes()).err(), Some(Refusal::Failed));
        assert_eq!(latin1_text(b"\xff\xfe").err(), Some(Refusal::Failed));
    }

    #[test]
    fn an_empty_run_decodes_to_nothing() {
        let (out, trace) = latin1_text(b"").unwrap();
        assert!(out.is_empty());
        trace.check_tiles().unwrap();
        assert_eq!(trace.out_bytes(), 0);
    }

    /// A protocol 0 line: the bytes under 0x100 are themselves and the five
    /// characters that would break the line are escapes.
    #[test]
    fn an_escaped_line_comes_off_both_layers_at_once() {
        let (out, trace) = escaped_latin1_text(b"\x07\xe4\\u000a[\xf5").unwrap();
        assert_eq!(out, b"\x07\xe4\n[\xf5");
        trace.check_tiles().unwrap();
        assert_eq!(trace.in_bits(), 10 * 8);
        assert_eq!(trace.out_bytes(), 5);
        // The escape is the one step that is not a byte copied through, and it
        // reads the six bytes that spelled it. The two bytes after it are one
        // step between them, being bytes copied through.
        assert_eq!(trace.map_out(2).unwrap().in_bits, 16..64);
        assert_eq!(trace.map_out(2).unwrap().kind, StepKind::Literal(b'\n'));
        assert_eq!(trace.map_out(3).unwrap().in_bits, 64..80);
    }

    #[test]
    fn a_line_no_pickler_wrote_is_not_decoded() {
        // An escape outside `\u` and `\U`, a short one, and hexadecimal in
        // both cases at once.
        assert_eq!(escaped_latin1_text(b"a\\nb").err(), Some(Refusal::Failed));
        assert_eq!(escaped_latin1_text(b"a\\u00").err(), Some(Refusal::Failed));
        assert_eq!(escaped_latin1_text(b"\\u00aB").err(), Some(Refusal::Failed));
        // And a line spelling a character latin-1 never held.
        assert_eq!(escaped_latin1_text(b"\\u0100").err(), Some(Refusal::Failed));
    }

    #[test]
    fn a_line_holding_no_escape_is_the_bytes_it_spells() {
        let (out, _) = escaped_latin1_text(b"\x07\xe4\x01\x02").unwrap();
        assert_eq!(out, b"\x07\xe4\x01\x02");
        assert_eq!(unescaped_len(b"\x07\xe4\x01\x02"), Some(4));
        assert_eq!(unescaped(b"a\\u005cb").as_deref(), Some("a\\b"));
    }
}
