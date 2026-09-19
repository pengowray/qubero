//! An integer, in whichever of the widths CPython writes one in.
//!
//! Every protocol has more than one spelling for a whole number and each
//! range belongs to exactly one of them, so the width is part of the grammar
//! rather than a detail of it: a small number written in a wide field is a
//! program no pickler wrote. The two text lines Python 2 wrote inside a
//! binary protocol are in [`python2`](super::python2) and protocol 0's own
//! line is in [`lines`](super::lines); this is where the three of them meet.

use super::cursor::Cursor;
use super::values::{decimal, minimal, two_complement};
use super::{Kind, Value};

impl Cursor<'_> {
    /// An integer, in the width CPython writes it in.
    ///
    /// BININT1 takes nought to 255, BININT2 the next two bytes' worth, and
    /// BININT anything else a signed four-byte integer holds. Past that comes
    /// LONG1, a run of two's-complement bytes as short as the number allows.
    /// Each range belongs to exactly one of them, so a small number written
    /// in a wide field is a non-match.
    pub(super) fn integer(&mut self) -> Option<Value> {
        self.gate()?;
        if self.proto == 0 {
            return self.number_line();
        }
        let start = self.at;
        let kind = match self.byte()? {
            b'K' => Kind::Int { value: i128::from(self.byte()?), at: start + 1, len: 1, spelled: false },
            b'M' => {
                let value = i128::from(u16::from_le_bytes(self.take(2)?.try_into().ok()?));
                if value < 256 {
                    return None;
                }
                Kind::Int { value, at: start + 1, len: 2, spelled: false }
            }
            b'J' => {
                let value = i128::from(i32::from_le_bytes(self.take(4)?.try_into().ok()?));
                if (0..=0xffff).contains(&value) {
                    return None;
                }
                Kind::Int { value, at: start + 1, len: 4, spelled: false }
            }
            0x8a if self.proto >= 2 => {
                let len = usize::from(self.byte()?);
                let at = self.at;
                let run = self.take(len)?;
                // CPython writes no byte a number does not need.
                if !minimal(run) {
                    return None;
                }
                match two_complement(run) {
                    // Anything a four-byte integer holds is written as one,
                    // from protocol 3 up. At protocol 2 a `long` is a type
                    // rather than a width: Python 2 wrote every one of them
                    // here whatever its size, and which numbers were `long`
                    // depended on how wide the interpreter's `int` was. So a
                    // small number here is ordinary and says nothing.
                    Some(value) if self.proto > 2 && i32::try_from(value).is_ok() => return None,
                    Some(value) => Kind::Int { value, at, len, spelled: false },
                    // Past sixteen bytes there is no integer type to read the
                    // run as, so the number is its digits and the run is a row
                    // beneath them.
                    None => Kind::Wide { at, len, digits: decimal(run)?, spelled: false },
                }
            }
            // The two lines Python 2 wrote inside a binary protocol, which is
            // the one place a protocol's own spellings are not the whole of
            // it: see [`python2`](super::python2).
            b'I' if self.proto <= 2 => self.int_line()?,
            b'L' if self.proto <= 1 => self.long_line()?,
            _ => return None,
        };
        Some(self.span(start, kind))
    }
}
