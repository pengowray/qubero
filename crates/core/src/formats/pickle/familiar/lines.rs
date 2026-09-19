//! Protocol 0, where every value is an opcode and a line of text.
//!
//! The lines are the whole of what protocol 0 adds. A number is written the
//! way Python spells one, a text is written escaped, and a byte string is a
//! Python 2 `repr`. So a line is not always the value it stands for: a text
//! holding a character above 0x7f, or a backslash, or a newline, reaches the
//! file as a spelling of itself, and reading it means undoing that spelling.
//!
//! What a real pickler writes is the whole of what is read here. An escape
//! outside the lists below, a float spelled a way `repr` never spells one, and
//! a number with a leading zero or a plus in front of it are each a non-match,
//! because each is a file no pickler wrote.
//!
//! The escaping a text line goes out under is undone by
//! [`crate::codec::pytext`], which is also the decoder a node opens a protocol
//! 0 array's numbers with. One reading of a line, wherever it is asked for.

use super::cursor::Cursor;
use super::memo::Bound;
use super::{Kind, Value};

/// The escapes Python 2's `repr` of a `str` writes, which is what a `STRING`
/// line holds: the backslash, the quote the line is written in, the three
/// whitespace characters `repr` names, and `\xNN` for every other byte outside
/// the printable range. Nothing else, so a `\d` or a `\0` is a non-match.
fn unescape_bytes(line: &[u8], quote: u8) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(line.len());
    let mut rest = line;
    while let Some((first, tail)) = rest.split_first() {
        if *first != b'\\' {
            // The quote the line is written in is escaped wherever it appears,
            // so a bare one inside the run is a line that ended early.
            if *first == quote {
                return None;
            }
            out.push(*first);
            rest = tail;
            continue;
        }
        let (code, tail) = tail.split_first()?;
        rest = tail;
        out.push(match code {
            b'\\' => b'\\',
            b'\'' => b'\'',
            b'"' => b'"',
            b'n' => b'\n',
            b'r' => b'\r',
            b't' => b'\t',
            b'x' => {
                let digits = std::str::from_utf8(tail.get(..2)?).ok()?;
                if !digits.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
                    return None;
                }
                rest = tail.get(2..)?;
                u8::from_str_radix(digits, 16).ok()?
            }
            _ => return None,
        });
    }
    Some(out)
}

/// What a protocol 0 line spells, read back out of the run the file holds.
///
/// `quote` is the quote a Python 2 `repr` of a `str` was written in, and
/// nothing for a text, which goes out `raw-unicode-escape`. Nothing at all for
/// a line no pickler wrote, which is what the recogniser holds a line to.
///
/// Read on demand rather than kept: every caller of this has the run in front
/// of it, and a file of escaped strings would otherwise be held twice over.
pub fn spelled(line: &[u8], quote: Option<u8>) -> Option<Vec<u8>> {
    match quote {
        Some(quote) => unescape_bytes(line, quote),
        None => crate::codec::pytext::unescaped(line).map(String::into_bytes),
    }
}

/// What a run a reference names is worth as text, given the protocol the file
/// was written at.
///
/// At protocol 0 a text is a line, and a line is not always the text it stands
/// for; from protocol 1 up the run is the text. One place decides which, so
/// the recogniser and the reading never disagree about what a named run holds.
pub fn named(run: &[u8], proto: u8) -> Option<Vec<u8>> {
    match is_named_text(run, proto) {
        true => Some(run.to_vec()),
        false => spelled(run, None),
    }
}

/// Whether the run a reference names is the text it stands for rather than a
/// line spelling one. Protocol 0 is the only protocol that writes a text as a
/// line, and most of its lines are still what they stand for.
pub(super) fn is_named_text(run: &[u8], proto: u8) -> bool {
    proto > 0 || is_itself(run)
}

/// Whether a run of bytes is what it stands for, which is what says a text may
/// be read where it sits rather than through a node of its own. A byte over
/// 0x7f stands for the character of that number, which is two bytes of UTF-8,
/// so only an unescaped ASCII run is itself.
fn is_itself(line: &[u8]) -> bool {
    line.iter().all(|b| b.is_ascii() && *b != b'\\')
}

/// Whether this is a number spelled the way Python spells one.
///
/// Digits, with no leading zero unless the number is nought, no plus in front
/// of it and nothing else. `repr` of an integer writes exactly this, and so
/// does the `%ld` Python 2's `cPickle` writes.
pub(super) fn is_whole(digits: &str) -> bool {
    let body = digits.strip_prefix('-').unwrap_or(digits);
    !body.is_empty() && body.bytes().all(|b| b.is_ascii_digit()) && (body.len() == 1 || !body.starts_with('0'))
}

/// Whether this is a float spelled the way a pickler writes one.
///
/// Two spellings are written and both are read. `pickle.py` writes `repr`,
/// which is the shortest run of digits that reads back as the same number and
/// always carries a decimal point or an exponent. Python 2's `cPickle` and
/// Python 3.4's C pickler write `%.17g` instead, which drops the point from a
/// whole number, so `3.0` goes out as `3`. Python 3.6 and later write `repr`
/// from both picklers, so the spelling says which release rather than which
/// pickler and the form reads it and says nothing.
fn is_real(said: &str) -> bool {
    if matches!(said, "inf" | "-inf" | "nan") {
        return true;
    }
    if java_real(said) {
        return true;
    }
    let body = said.strip_prefix('-').unwrap_or(said);
    let (mantissa, exponent) = match body.split_once('e') {
        Some((m, e)) => (m, Some(e)),
        None => (body, None),
    };
    // An exponent is written with its sign and at least two digits, which is
    // what both spellings do: `1e+308`, `1e-06`.
    if let Some(exponent) = exponent {
        let digits = exponent.strip_prefix('+').or_else(|| exponent.strip_prefix('-'));
        match digits {
            Some(digits) if digits.len() >= 2 && digits.bytes().all(|b| b.is_ascii_digit()) => {}
            _ => return false,
        }
    }
    let (whole, fraction) = match mantissa.split_once('.') {
        Some((w, f)) => (w, Some(f)),
        None => (mantissa, None),
    };
    if !is_whole(whole) {
        return false;
    }
    match fraction {
        None => true,
        Some(fraction) => !fraction.is_empty() && fraction.bytes().all(|b| b.is_ascii_digit()),
    }
}

/// Whether this is a float spelled the way Java's `Double.toString` spells
/// one, which is what GraalPy's `_pickle` writes at protocol 0.
///
/// Java writes a mantissa that always carries a point with a digit each side,
/// an upper-case `E` in front of an exponent written with no plus and no
/// padding, and the three names `Infinity`, `-Infinity` and `NaN`. Python's
/// `repr` writes none of that, so the two spellings never overlap on a number
/// with an exponent or a special value, and a small whole number such as
/// `1.0` is the same run either way.
///
/// This one is the pickler and not the runtime: GraalPy's `pickle.py` writes
/// `repr` like every other copy of it, so a file with a line like this came
/// from the `_pickle` beside it.
fn java_real(said: &str) -> bool {
    if matches!(said, "Infinity" | "-Infinity" | "NaN") {
        return true;
    }
    let body = said.strip_prefix('-').unwrap_or(said);
    let (mantissa, exponent) = match body.split_once('E') {
        Some((m, e)) => (m, e),
        None => return false,
    };
    let digits = exponent.strip_prefix('-').unwrap_or(exponent);
    if !is_whole(digits) || digits.starts_with('-') {
        return false;
    }
    let Some((whole, fraction)) = mantissa.split_once('.') else { return false };
    is_whole(whole) && !fraction.is_empty() && fraction.bytes().all(|b| b.is_ascii_digit())
}

impl Cursor<'_> {
    /// A `UNICODE` line, which is how protocol 0 writes text.
    pub(super) fn text_line(&mut self) -> Option<Value> {
        let start = self.at;
        self.exact(b"V")?;
        let (at, len) = self.line()?;
        let kind = self.spelling(at, len, false)?;
        self.memoize(Bound::Text { at, len })?;
        Some(self.span(start, kind))
    }

    /// A `STRING` line, which is how protocol 0 writes a Python 2 `str`: the
    /// `repr` of it, quotes and all. The value is the run between the quotes.
    pub(super) fn string_line(&mut self) -> Option<Value> {
        let start = self.at;
        self.exact(b"S")?;
        let (at, len) = self.line()?;
        // `repr` writes single quotes unless the string holds one and no
        // double quote, so both are read and the pair has to match.
        let quote = *self.bytes.get(at)?;
        if !matches!(quote, b'\'' | b'"') || len < 2 || self.bytes.get(at + len - 1) != Some(&quote) {
            return None;
        }
        let (at, len) = (at + 1, len - 2);
        let kind = self.spelling(at, len, true)?;
        self.memoize(Bound::Text { at, len })?;
        Some(self.span(start, kind))
    }

    /// What a line's run is worth: the run itself when it is what it stands
    /// for, and the thing it spells otherwise, decoded once here and kept
    /// beside the match.
    ///
    /// `quoted` says this is a Python 2 `str` rather than text, which decides
    /// both the escapes and what the run comes to.
    fn spelling(&mut self, at: usize, len: usize, quoted: bool) -> Option<Kind> {
        let line = self.bytes.get(at..at + len)?;
        if is_itself(line) {
            return Some(match quoted {
                false => Kind::Text { at, len },
                // A Python 2 `str` is bytes that were usually text, and reads
                // the way the opcode listing reads one.
                true => match std::str::from_utf8(line).is_ok() {
                    true => Kind::Text { at, len },
                    false => Kind::Bytes { at, len },
                },
            });
        }
        let quote = match quoted {
            true => Some(*self.bytes.get(at.checked_sub(1)?)?),
            false => None,
        };
        // Read once here to say the line is one a pickler wrote, and read
        // again wherever what it spells is wanted. Nothing is kept: the run is
        // in the file and this is a few bytes of arithmetic over it.
        let held = spelled(line, quote)?;
        // A `str` whose bytes are not UTF-8 is a byte string, as it is
        // everywhere else; text is text, since it was decoded from characters.
        let bytes = quoted && std::str::from_utf8(&held).is_err();
        Some(Kind::Spelled { at, len, quote, bytes })
    }

    /// An `INT` or a `LONG` line, which is how protocol 0 and protocol 1 write
    /// a number no binary opcode held.
    ///
    /// Protocol 0 writes every number this way: Python 2 an `int` as `INT` and
    /// a `long` as `LONG`, Python 3 every integer as `LONG`. So the range
    /// rules the binary opcodes are held to do not apply, and what is checked
    /// is the spelling.
    pub(super) fn number_line(&mut self) -> Option<Value> {
        let start = self.at;
        let long = self.byte()? == b'L';
        let (at, len) = self.line()?;
        let said = std::str::from_utf8(self.bytes.get(at..at + len)?).ok()?;
        // `True` and `False`, which had no opcode of their own below
        // protocol 2 and are the one place a leading zero is written.
        if !long && matches!(said, "01" | "00") {
            return Some(self.span(start, Kind::Bool(said == "01")));
        }
        // A `long` carries the letter Python 2 spelled one with.
        let digits = match long {
            true => said.strip_suffix('L')?,
            false => said,
        };
        if !is_whole(digits) {
            return None;
        }
        let len = len - usize::from(long);
        // More digits than the reader's integer type holds, which is what a
        // 128-bit `uuid.UUID` and `2 ** 200` both are. The number is its digits
        // and the line is a row beneath them.
        let kind = match digits.parse::<i128>() {
            Ok(value) => Kind::Int { value, at, len, spelled: true },
            Err(_) => Kind::Wide { at, len, digits: digits.to_string(), spelled: true },
        };
        Some(self.span(start, kind))
    }

    /// A `FLOAT` line, which is the number as Python spells it.
    pub(super) fn float_line(&mut self) -> Option<Value> {
        let start = self.at;
        self.exact(b"F")?;
        let (at, len) = self.line()?;
        let said = std::str::from_utf8(self.bytes.get(at..at + len)?).ok()?;
        if !is_real(said) {
            return None;
        }
        if java_real(said) {
            self.wrote(super::Pickler::Graal)?;
        }
        let value = said.parse::<f64>().ok()?;
        Some(self.span(start, Kind::Float { value, at, len, spelled: true }))
    }
}
