//! A byte string at protocol 2, which is the one plain value that protocol has
//! no opcode for.
//!
//! Protocol 3 is where a pickle got a type for bytes. Below it CPython hands
//! the bytes to `_codecs.encode` as the text they spell in latin-1, so the run
//! in the file is that text written UTF-8: a byte under 0x80 is itself and
//! every byte above it is two. The bytes are therefore not in the file as
//! bytes, and the value says so rather than pretending the run is the value.
//!
//! The whole thing is a fixed run of instructions with no stack in it, so it
//! is read here rather than through the calls table, and the encoding word is
//! checked to be `latin1` and nothing else.

use super::cursor::Cursor;
use super::memo::Bound;
use super::{Kind, Names, Shape, Storage, Value, LATIN1, NO_OPCODE, SPELLED};

impl Cursor<'_> {
    /// `_codecs.encode(text, 'latin1')`, or `bytes()` for an empty one.
    pub(super) fn spelled_bytes(&mut self) -> Option<Value> {
        if self.proto >= 3 {
            return None;
        }
        let start = self.at;
        let said = self.says.len();
        let here = self.save();
        let made = match self.encoded(start) {
            Some(value) => Some(value),
            None => {
                self.restore(here);
                self.empty_bytes(start)
            }
        };
        if made.is_some() {
            // The names the call was made with stay as the instructions they
            // are; the two values under the byte string are what varies.
            self.says.truncate(said);
        }
        made
    }

    /// The call alone, and where the text it was handed sits, for a run of
    /// bytes that belongs to something else: a NumPy array's numbers reach
    /// protocol 2 this way, and the array files the slot the REDUCE writes.
    /// An array with nothing in it takes the empty spelling, so both are here.
    pub(super) fn bytes_run(&mut self) -> Option<(usize, usize, Storage)> {
        // Python 2 had a type for a run of bytes, its `str`, so a pickler
        // there writes one out and the bytes are the bytes. The detour through
        // `_codecs` is Python 3 writing at a protocol with no type for them.
        if self.proto == 0 {
            let here = self.save();
            return match self.encode_call() {
                Some(held) => Some(held),
                None => {
                    self.restore(here);
                    Some((self.empty_call()?, 0, Storage::Raw))
                }
            };
        }
        if self.proto <= 2 && matches!(self.peek(), Some(b'U') | Some(b'T')) {
            let code = self.byte()?;
            let (at, len) = self.counted(code, b'U', b'T', NO_OPCODE)?;
            return Some((at, len, Storage::Raw));
        }
        let said = self.says.len();
        let here = self.save();
        let held = match self.encode_call() {
            Some(held) => held,
            None => {
                self.restore(here);
                (self.empty_call()?, 0, Storage::Raw)
            }
        };
        self.says.truncate(said);
        Some(held)
    }

    /// `_codecs.encode` over a text and the word `latin1`, up to and including
    /// the REDUCE. The memo mark the REDUCE's result goes in belongs to
    /// whatever the bytes are part of, so it is left to the caller.
    fn encode_call(&mut self) -> Option<(usize, usize, Storage)> {
        self.global(&["_codecs"], "encode", "module", "callable")?;
        self.open_tuple()?;
        let (at, len, storage) = match self.text()?.kind {
            // The run in the file is the latin-1 text, which is what the
            // caller reads the bytes out of.
            Kind::Text { at, len } => {
                self.latin1(at, len)?;
                (at, len, Storage::Latin1)
            }
            // Protocol 0 wrote the text as a line with an escape in it, which
            // the form read as it went. What the call makes of it is those
            // characters one byte each, so the run is read again that way and
            // kept in its place: nothing else names the text of a call whose
            // result is a buffer.
            Kind::Spelled { at, len, bytes: false } => {
                let held = Storage::Latin1.read(&self.decoded_at(at)?)?;
                self.replace_run(at, held);
                (at, len, Storage::Escaped)
            }
            _ => return None,
        };
        self.encoding_word()?;
        self.close_tuple(2)?;
        self.memoize(Bound::Opaque)?;
        self.exact(b"R")?;
        Some((at, len, storage))
    }

    fn encoded(&mut self, start: usize) -> Option<Value> {
        self.global(&["_codecs"], "encode", "module", "callable")?;
        self.open_tuple()?;
        let text = self.text()?;
        if let Kind::Text { at, len } = text.kind {
            self.latin1(at, len)?;
        }
        let encoding = self.encoding_word()?;
        self.close_tuple(2)?;
        self.memoize(Bound::Opaque)?;
        self.exact(b"R")?;
        // Python hashes a byte string, so a name for one may stand where a
        // dictionary key belongs.
        self.memoize(Bound::Made { what: Shape::Bytes, at: start, hashable: true })?;
        Some(self.span(start, Kind::Made { what: Shape::Bytes, names: SPELLED, callable: None, items: vec![text, encoding], state: None }))
    }

    /// `bytes()` with no arguments, which is what an empty byte string is
    /// below protocol 3. The value is the empty run it stands for.
    fn empty_bytes(&mut self, start: usize) -> Option<Value> {
        let at = self.empty_call()?;
        self.memoize(Bound::Bytes { at, len: 0 })?;
        Some(self.span(start, Kind::Bytes { at, len: 0 }))
    }

    /// `bytes()` up to and including the REDUCE, leaving the memo mark after
    /// it to whoever the byte string belongs to. Comes back as the empty run
    /// it stands for, which is where the file has got to.
    fn empty_call(&mut self) -> Option<usize> {
        self.global(&["__builtin__"], "bytes", "module", "class")?;
        self.empty_tuple()?;
        self.exact(b"R")?;
        Some(self.at)
    }

    /// Whether this text is one latin-1 could have spelled, which is a text of
    /// characters under 0x100. A pickler wrote every byte of the original as
    /// the character of the same number, so anything above that is a text no
    /// pickler put there and a run this has no bytes to read back.
    fn latin1(&self, at: usize, len: usize) -> Option<()> {
        let held = std::str::from_utf8(self.bytes.get(at..at + len)?).ok()?;
        held.chars().all(|c| u32::from(c) < 0x100).then_some(())
    }

    /// The encoding the bytes were handed to `_codecs` under, spelled here or
    /// named where an earlier byte string spelled it. Only `latin1`: any other
    /// encoding is a byte string this does not know how to read back.
    fn encoding_word(&mut self) -> Option<Value> {
        self.gate()?;
        let start = self.at;
        if self.at_reference() {
            let here = self.save();
            if let Some(Bound::Text { at, len }) = self.reference().cloned() {
                if self.bytes.get(at..at + len) == Some(LATIN1.as_bytes()) {
                    return Some(self.span(start, Kind::Ref(Names::Text { at, len })));
                }
            }
            self.restore(here);
            return None;
        }
        let value = self.text()?;
        match value.kind {
            Kind::Text { at, len } if self.bytes.get(at..at + len) == Some(LATIN1.as_bytes()) => Some(value),
            _ => None,
        }
    }

}
