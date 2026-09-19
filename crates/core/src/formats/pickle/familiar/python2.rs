//! What Python 2 wrote and Python 3 does not.
//!
//! Three spellings, each of them a type Python 2 had and Python 3 dropped, and
//! each of them still arriving today: PyPy 2.7 writes them, and so do Jython
//! and IronPython 2.7, which are Python 2 whatever they are written in.
//!
//! - A `str`, which is a run of bytes that was usually text.
//! - An `int` too wide for BININT, written as a text line inside a binary
//!   protocol, and a `long`, written as a line with the letter Python 2
//!   spelled one with.
//! - A `str` handed to `bytes` with the encoding that turns it back, which is
//!   what IronPython 2.7 writes where CPython 2 writes the bytes themselves.

use super::cursor::Cursor;
use super::memo::Bound;
use super::{Kind, Shape, Value, NO_OPCODE, SPELLED};

/// The same encoding under the name its other callers write it by. A codec is
/// handed `latin1` and a class is handed `latin-1`; each call is written the
/// way its own caller wrote it.
const HYPHENATED: &str = "latin-1";

impl Cursor<'_> {
    /// A Python 2 `str`, which is a run of bytes that was usually text.
    ///
    /// The opcode listing reads one as text of an encoding nobody declared, so
    /// this does the same: text when the bytes are UTF-8, which covers every
    /// ASCII string, and a byte string when they are not. Python 2 wrote these
    /// for every ordinary string it had, and Python 3 writes none of them.
    pub(super) fn py2_string(&mut self) -> Option<Value> {
        if self.proto > 2 {
            return None;
        }
        let start = self.at;
        let code = self.byte()?;
        let (at, len) = self.counted(code, b'U', b'T', NO_OPCODE)?;
        let held = self.bytes.get(at..at + len)?;
        let kind = match std::str::from_utf8(held).is_ok() {
            true => Kind::Text { at, len },
            false => Kind::Bytes { at, len },
        };
        self.memoize(match kind {
            Kind::Text { .. } => Bound::Text { at, len },
            _ => Bound::Bytes { at, len },
        })?;
        Some(self.span(start, kind))
    }

    /// `bytes(text, 'latin-1')`, which is what IronPython 2.7 writes for a
    /// `str` where CPython 2 writes the bytes themselves.
    ///
    /// The same detour as `_codecs.encode`, through the class rather than
    /// through the codec, and with the encoding spelled the way the class's
    /// own caller spells it. IronPython's `str` is a .NET string, so its
    /// `__reduce__` hands over the characters and the encoding that turns
    /// them back into bytes; both of its picklers write it.
    pub(super) fn spelled_call(&mut self, start: usize) -> Option<Value> {
        if self.proto > 2 {
            return None;
        }
        self.global(&["__builtin__"], "bytes", "module", "class")?;
        self.open_tuple()?;
        let text = self.latin1_text()?;
        let encoding = self.exact_word(HYPHENATED, true)?;
        self.close_tuple(2)?;
        self.memoize(Bound::Opaque)?;
        self.exact(b"R")?;
        self.memoize(Bound::Made { what: Shape::Bytes, at: start, hashable: true })?;
        Some(self.span(start, Kind::Made { what: Shape::Bytes, names: SPELLED, callable: None, items: vec![text, encoding], state: None, attrs: None }))
    }

    /// A text line inside a binary protocol, which is what Python 2 wrote for
    /// an `int` too wide for BININT. Its `int` was a machine word, so this
    /// covers the range between a four-byte and an eight-byte one and nothing
    /// else: anything wider was a `long` and went out as LONG1. Python 3
    /// writes none of these.
    ///
    /// Comes back as the kind, with the span left to the caller, since `I01`
    /// and `I00` are a boolean rather than a number and end the reading here.
    pub(super) fn int_line(&mut self) -> Option<Kind> {
            let (at, len) = self.line()?;
            let digits = std::str::from_utf8(self.bytes.get(at..at + len)?).ok()?;
            // `I01` and `I00` are `True` and `False`, which had no opcode
            // of their own below protocol 2. The integers one and nought
            // are `I1` and `I0` where they are written as lines at all,
            // so the leading zero is the whole of the difference.
            if self.proto <= 1 && matches!(digits, "01" | "00") {
                return Some(Kind::Bool(digits == "01"));
            }
            let value = i128::from(digits.parse::<i64>().ok()?);
            // CPython writes no leading zero, no plus and no space, so the
            // digits have to be what the number is spelled as.
            if digits != value.to_string() || i32::try_from(value).is_ok() {
                return None;
            }
            Some(Kind::Int { value, at, len, spelled: true })
    }

    /// A `long`, which protocol 2 writes as LONG1 and protocol 1 as a line of
    /// digits with the `L` Python 2 spelled one with. Python 3 writes the same
    /// line, `L` and all.
    pub(super) fn long_line(&mut self) -> Option<Kind> {
            let (at, len) = self.line()?;
            let digits = std::str::from_utf8(self.bytes.get(at..at + len)?).ok()?.strip_suffix('L')?;
            // CPython writes no leading zero and no plus in front of one.
            if !super::lines::is_whole(digits) {
                return None;
            }
            Some(match digits.parse::<i128>() {
                // Python 3 writes this line only for a number no BININT
                // holds; Python 2 wrote it for every `long`, and on an
                // interpreter whose `int` was four bytes wide that
                // included small ones. Both are read.
                Ok(value) => Kind::Int { value, at, len: len - 1, spelled: true },
                // More digits than the reader's integer type holds, which
                // is the same number `LONG1` writes in seventeen bytes or
                // more at the protocols above this one.
                Err(_) => Kind::Wide { at, len: len - 1, digits: digits.to_string(), spelled: true },
            })
    }
}
