//! The memo table: the slots a pickle files values in, and what a form is
//! prepared to say each one holds. It is a type of its own so that a
//! production asks it for a slot by name instead of indexing a vector, and so
//! that the methods which read or write a slot are in one place.

use super::cursor::Cursor;
use super::{Pickler, MAX_MEMO};

/// What a memo slot holds, as far as a form is prepared to say.
///
/// A slot a form cannot name is [`Bound::Opaque`], and a reference to one is
/// a non-match. Only the things a form spelled out itself can be referred to
/// again, which is what keeps `BINGET` from being an arbitrary stack effect.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Bound {
    Opaque,
    Text { at: usize, len: usize },
    Bytes { at: usize, len: usize },
    /// A module and a callable the form named exactly, spelled `module.name`.
    Global(String),
    /// A completed NumPy dtype, as the `<f4` spelling it ends up with. The
    /// slot is written at the REDUCE that makes the dtype and the byte order
    /// arrives at the BUILD just after, so the binding is filled in there.
    Dtype(String),
}

/// The table itself, which a form appends to and reads back and never edits,
/// but for the one slot a dtype's byte order arrives too late to fill in.
pub(super) struct Memo {
    slots: Vec<Bound>,
}

impl Memo {
    pub(super) fn new() -> Self {
        Memo { slots: Vec::new() }
    }

    /// How many slots the file has written, which is the number the next one
    /// gets.
    pub(super) fn len(&self) -> usize {
        self.slots.len()
    }

    /// Back to the length it was, for an alternative that failed.
    pub(super) fn truncate(&mut self, len: usize) {
        self.slots.truncate(len);
    }

    /// File a value in the next slot, or refuse when the file has written more
    /// slots than a form will follow.
    fn bind(&mut self, bound: Bound) -> Option<usize> {
        if self.slots.len() >= MAX_MEMO {
            return None;
        }
        self.slots.push(bound);
        Some(self.slots.len() - 1)
    }

    /// What a slot holds, or nothing when the file has not written it.
    fn get(&self, slot: usize) -> Option<&Bound> {
        self.slots.get(slot)
    }

    /// Say what a slot holds after the fact, which is what the BUILD that
    /// gives a dtype its byte order does to the slot the REDUCE filed.
    pub(super) fn fill(&mut self, slot: usize, bound: Bound) {
        self.slots[slot] = bound;
    }
}

impl Cursor<'_> {
    /// MEMOIZE, which files the value just built in the next slot. Protocol 4
    /// writes no index: the slot is the count of marks before it.
    pub(super) fn memoize(&mut self, bound: Bound) -> Option<usize> {
        self.exact(&[0x94])?;
        self.memo.bind(bound)
    }

    /// The memo mark after a BYTEARRAY8, which only one of the two picklers
    /// always wrote.
    ///
    /// `_pickle` files a bytearray in the memo; `pickle.py` did not until
    /// Python 3.10, so a BYTEARRAY8 with nothing behind it is that pickler's
    /// spelling. Every slot after it is numbered one lower, which is why the
    /// mark cannot simply be skipped.
    pub(super) fn bytearray_memoize(&mut self, bound: Bound) -> Option<()> {
        if self.peek() == Some(0x94) {
            self.memoize(bound)?;
            return Some(());
        }
        self.wrote(Pickler::Python)
    }

    /// BINGET or LONG_BINGET, and what the slot it names holds. A slot past
    /// the end, or one the file has not written, is a non-match.
    pub(super) fn reference(&mut self) -> Option<&Bound> {
        let slot = match self.byte()? {
            b'h' => self.byte()? as usize,
            b'j' => u32::from_le_bytes(self.take(4)?.try_into().ok()?) as usize,
            _ => return None,
        };
        self.memo.get(slot)
    }

    pub(super) fn at_reference(&self) -> bool {
        matches!(self.peek(), Some(b'h') | Some(b'j'))
    }

    /// SHORT_BINUNICODE with exactly this spelling, and the memo mark that
    /// files it. Comes back as the bytes the word sits in.
    pub(super) fn word(&mut self, text: &str) -> Option<(usize, usize)> {
        self.gate()?;
        self.exact(&[0x8c, u8::try_from(text.len()).ok()?])?;
        let at = self.at;
        self.exact(text.as_bytes())?;
        self.memoize(Bound::Text { at, len: text.len() })?;
        Some((at, text.len()))
    }

    /// The same word, spelled out here or referred to where it was spelled.
    /// A reference has no bytes of its own to read as text, so it comes back
    /// as nothing to name and the BINGET stays an instruction.
    pub(super) fn word_or_reference(&mut self, text: &str) -> Option<Option<(usize, usize)>> {
        self.gate()?;
        if self.at_reference() {
            let here = self.save();
            let held = self.reference().cloned();
            if let Some(Bound::Text { at, len }) = held {
                if self.bytes.get(at..at + len) == Some(text.as_bytes()) {
                    return Some(None);
                }
            }
            self.restore(here);
            return None;
        }
        Some(Some(self.word(text)?))
    }

    /// A module and a callable, joined by STACK_GLOBAL, or a reference to the
    /// slot the same pair was filed in earlier. `modules` is every spelling
    /// the form accepts, each written out.
    pub(super) fn global(
        &mut self,
        modules: &[&str],
        name: &str,
        module_says: &'static str,
        name_says: &'static str,
    ) -> Option<String> {
        // A slot holding this exact module and callable stands for the pair.
        // Any other reference here is the module name on its own, which the
        // spelled-out production below reads.
        self.gate()?;
        if self.at_reference() {
            let here = self.save();
            if let Some(Bound::Global(full)) = self.reference().cloned() {
                if modules.iter().any(|m| full == format!("{m}.{name}")) {
                    return Some(full);
                }
            }
            self.restore(here);
        }
        let here = self.save();
        let module = modules.iter().find_map(|m| {
            self.restore(here);
            self.word_or_reference(m).map(|said| {
                if let Some((at, len)) = said {
                    self.says(module_says, at, len);
                }
                (*m).to_string()
            })
        })?;
        let (at, len) = self.word(name)?;
        self.says(name_says, at, len);
        self.exact(&[0x93])?;
        let full = format!("{module}.{name}");
        self.memoize(Bound::Global(full.clone()))?;
        Some(full)
    }
}
