//! The leaf values every form reads: text, byte strings, numbers, a name for
//! something the file wrote earlier, and what Python can hash. The stack these
//! are pushed on is in [`basic`](super::basic), and the productions a protocol
//! writes as a line rather than as an opcode are in [`lines`](super::lines).

use super::cursor::Cursor;
use super::memo::Bound;
use super::{Kind, Names, Shape, Value, CONTENT, NO_OPCODE};

/// A run of little-endian two's-complement bytes, as the number it spells.
///
/// LONG1 declares a length of up to 255, which reaches numbers no integer
/// type here holds. Sixteen bytes is where that stops, so a longer one is a
/// non-match rather than a number read wrong.
pub(super) fn two_complement(bytes: &[u8]) -> Option<i128> {
    if bytes.is_empty() || bytes.len() > 16 {
        return None;
    }
    let mut value: i128 = if bytes[bytes.len() - 1] & 0x80 == 0 { 0 } else { -1 };
    for byte in bytes.iter().rev() {
        value = (value << 8) | i128::from(*byte);
    }
    Some(value)
}

/// How many two's-complement bytes a number needs, which is how many CPython
/// writes: it takes one more byte than the magnitude's bits fill and drops it
/// again when the sign bits in it are redundant.
pub(super) fn shortest(value: i128) -> usize {
    (1..16)
        .find(|len| {
            let bits = len * 8 - 1;
            value >= -(1i128 << bits) && value < (1i128 << bits)
        })
        .unwrap_or(16)
}

/// Whether a value may be a dictionary key or a set member.
///
/// Python hashes numbers, strings, byte strings, tuples of hashable things,
/// frozensets and the three singletons, and nothing else a form builds. A key
/// of any other kind is not something CPython could have been asked to write.
pub(super) fn hashable(value: &Value) -> bool {
    match &value.kind {
        Kind::None | Kind::Bool(_) | Kind::Int { .. } | Kind::Float { .. } => true,
        Kind::Text { .. } | Kind::Bytes { .. } | Kind::Spelled { .. } => true,
        // A name stands for whatever the slot holds, which hashes or does
        // not for the same reasons the thing itself does.
        Kind::Ref(names) => match names {
            Names::Text { .. } | Names::Bytes { .. } => true,
            Names::Made { hashable, .. } => *hashable,
        },
        Kind::Tuple(items) | Kind::FrozenSet(items) => items.iter().all(hashable),
        // A NumPy scalar hashes; an array does not.
        Kind::Array { dimensions, .. } => dimensions.is_empty(),
        // A byte string below protocol 3 is a call rather than a literal, and
        // Python hashes it the same as any other byte string.
        Kind::Made { what: Shape::Bytes, .. } => true,
        Kind::Made { what, items, .. } => {
            matches!(what, Shape::Slice | Shape::Range | Shape::Complex) && items.iter().all(hashable)
        }
        // A class hashes in Python and an object of one usually does, but no
        // file in the corpus writes either as a key, so neither is read as
        // one until something does.
        Kind::List(_) | Kind::Set(_) | Kind::Dict(_) => false,
        Kind::Class { .. } | Kind::Instance { .. } | Kind::Objects { .. } | Kind::DType(_) => false,
    }
}

impl Cursor<'_> {
    /// Text, in the widths the protocol in hand has.
    ///
    /// Protocol 4 added the one-byte and eight-byte lengths and CPython uses
    /// all three from there. Protocols 2 and 3 have only BINUNICODE, whose
    /// length is four bytes however short the text is.
    pub(super) fn text(&mut self) -> Option<Value> {
        self.gate()?;
        if self.proto == 0 {
            return self.text_line();
        }
        let start = self.at;
        let code = self.byte()?;
        let (at, len) = match self.proto >= 4 {
            true => self.counted(code, 0x8c, 0x58, 0x8d)?,
            false => self.counted(code, NO_OPCODE, 0x58, NO_OPCODE)?,
        };
        std::str::from_utf8(self.bytes.get(at..at + len)?).ok()?;
        self.memoize(Bound::Text { at, len })?;
        Some(self.span(start, Kind::Text { at, len }))
    }

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

    /// BINGET or LONG_BINGET where a value belongs: the file naming
    /// something it wrote earlier rather than writing it again.
    ///
    /// Any value the basic productions built may be named, which covers one
    /// list under several keys and a list holding itself: a container is
    /// filed when it is created, so a name for it exists while it is still
    /// being filled. What a slot a form could not name holds is opaque, and a
    /// reference to one is a non-match as before.
    pub(super) fn named(&mut self) -> Option<Value> {
        self.gate()?;
        let start = self.at;
        let kind = match self.reference()?.clone() {
            Bound::Text { at, len } => Kind::Ref(Names::Text { at, len }),
            Bound::Bytes { at, len } => Kind::Ref(Names::Bytes { at, len }),
            Bound::Made { what, at, hashable } => Kind::Ref(Names::Made { what, at, hashable }),
            // A class the file named earlier, which is how the second block of
            // a frame names the callable the first one spelled out. Only the
            // modules this form may name a class from: a slot holding one of
            // the globals a NumPy call names inside its own fixed run is not a
            // class this form has anything to say about.
            Bound::Global(path) if self.may_name(&path) => Kind::Class { path, parts: Vec::new() },
            _ => return None,
        };
        Some(self.span(start, kind))
    }

    /// BYTEARRAY8, which protocol 5 writes for a bytearray where protocol 4
    /// calls the class. The bytes it was made from are a field of their own,
    /// so both spellings read the same way.
    pub(super) fn bytearray(&mut self) -> Option<Value> {
        if self.proto < 5 {
            return None;
        }
        self.gate()?;
        let start = self.at;
        self.exact(&[0x96])?;
        let (at, len) = self.counted(0x96, NO_OPCODE, NO_OPCODE, 0x96)?;
        self.bytearray_memoize(Bound::Made { what: Shape::ByteArray, at: start, hashable: false })?;
        let held = Value { at, len, kind: Kind::Bytes { at, len } };
        Some(self.span(start, Kind::Made { what: Shape::ByteArray, names: CONTENT, callable: None, items: vec![held], state: None }))
    }

    pub(super) fn binfloat(&mut self) -> Option<Value> {
        self.gate()?;
        if self.proto == 0 {
            return self.float_line();
        }
        let start = self.at;
        self.exact(b"G")?;
        let value = f64::from_be_bytes(self.take(8)?.try_into().ok()?);
        Some(self.span(start, Kind::Float { value, at: start + 1, len: 8 }))
    }

    /// A byte string written as one.
    ///
    /// Protocol 3 is where a pickle got a type for bytes at all, and the
    /// eight-byte length arrived with protocol 4. Protocol 2 writes a byte
    /// string as a call instead, which is [`Cursor::spelled_bytes`].
    pub(super) fn byte_string(&mut self) -> Option<Value> {
        if self.proto < 3 {
            return self.spelled_bytes();
        }
        self.gate()?;
        let start = self.at;
        let code = self.byte()?;
        let widest = if self.proto >= 4 { 0x8e } else { NO_OPCODE };
        let (at, len) = self.counted(code, b'C', b'B', widest)?;
        self.memoize(Bound::Bytes { at, len })?;
        Some(self.span(start, Kind::Bytes { at, len }))
    }
}
