//! The builtins productions: the types a pickle writes as a call of the class
//! rather than as a literal, which are a slice, a range, a complex number and,
//! below protocol 5, a bytearray. Only the builtins form allows them.

use super::cursor::Cursor;
use super::memo::Bound;
use super::{Kind, Shape, Value, BOUNDS, CONTENT, HALVES, SPELLED};

/// The encoding Python 2 hands `bytearray` beside the text, which is the one
/// that maps every byte to the character of the same number. Spelled with the
/// hyphen, where `_codecs.encode` is handed the name without one: the two are
/// the same encoding and each call is written the way its own caller wrote it.
const BYTEARRAY_ENCODING: &str = "latin-1";

impl Cursor<'_> {
    /// The builtin types a pickle writes as a call rather than as a literal.
    ///
    /// A bytearray is one of these only below protocol 5, which gave it an
    /// opcode of its own. An empty one is called with no arguments at all,
    /// and CPython does not memoise the empty tuple that says so.
    pub(super) fn builtin(&mut self) -> Option<Value> {
        let start = self.at;
        let here = self.save();
        let calls: &[(&str, Shape, &'static [&'static str], usize)] = &[
            ("slice", Shape::Slice, BOUNDS, 3),
            ("range", Shape::Range, BOUNDS, 3),
            ("complex", Shape::Complex, HALVES, 2),
            // Python 2 had no type for a run of bytes to hand the class, so it
            // hands over the latin-1 text those bytes spell and the encoding
            // that turns the one back into the other, which is the same detour
            // a byte string itself takes at protocol 2.
            ("bytearray", Shape::ByteArray, SPELLED, 2),
            ("bytearray", Shape::ByteArray, CONTENT, 1),
            ("bytearray", Shape::ByteArray, CONTENT, 0),
        ];
        // Python 2 knew the builtins under another module name, and `range`
        // under another name as well: `fix_imports` writes those below
        // protocol 3 and the Python 3 names from there up.
        let old = self.proto < 3;
        let module: &[&str] = match old {
            true => &["__builtin__"],
            false => &["builtins"],
        };
        for (name, what, names, arity) in calls.iter().copied() {
            if what == Shape::ByteArray && self.proto > 4 {
                continue;
            }
            // The two-argument spelling is Python 2's, and Python 2 stopped at
            // protocol 2.
            if names == SPELLED && self.proto > 2 {
                continue;
            }
            let name = match (old, what) {
                (true, Shape::Range) => "xrange",
                _ => name,
            };
            self.restore(here);
            let made = (|| {
                self.global(module, name, "module", "class")?;
                if arity > 0 {
                    self.open_tuple()?;
                }
                let mut items = Vec::new();
                for i in 0..arity {
                    items.push(match what {
                        // A slice's bounds are integers or None; a range's are
                        // always integers; and a complex is two floats.
                        Shape::Slice => self.int_or_none()?,
                        Shape::Range => self.integer()?,
                        Shape::Complex => self.binfloat()?,
                        // Python 2's bytearray: the text and the encoding it
                        // was spelled in. Everywhere else a bytearray is made
                        // from one byte string.
                        _ if names == SPELLED && i == 0 => self.latin1_text()?,
                        _ if names == SPELLED => self.exact_word(BYTEARRAY_ENCODING, true)?,
                        _ => self.byte_string()?,
                    });
                }
                if arity == 0 {
                    self.empty_tuple()?;
                } else {
                    self.close_tuple(arity)?;
                    self.memoize(Bound::Opaque)?;
                }
                self.exact(b"R")?;
                // What the call made, which a later part of the file may name:
                // two pandas frames whose blocks sit in the same places share
                // the slice that says so, and the second names it. The tuple
                // of arguments above is only matched past and stays opaque. A
                // name hashes as the thing spelled out does, which is every
                // one of these but a bytearray.
                self.memoize(Bound::Made { what, at: start, hashable: what != Shape::ByteArray })?;
                Some(self.span(start, Kind::Made { what: what, names: names, callable: None, items: items, state: None }))
            })();
            if let Some(value) = made {
                // The names the call was made with stay as the instructions
                // they are; the value already says what it is.
                self.says.truncate(here.says);
                self.objects += 1;
                return Some(value);
            }
        }
        self.restore(here);
        None
    }

    fn int_or_none(&mut self) -> Option<Value> {
        self.gate()?;
        if self.peek()? == b'N' {
            let start = self.at;
            self.byte()?;
            return Some(self.span(start, Kind::None));
        }
        self.integer()
    }
}
