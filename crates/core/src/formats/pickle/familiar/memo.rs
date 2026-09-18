//! The memo table: the slots a pickle files values in, and what a form is
//! prepared to say each one holds. It is a type of its own so that a
//! production asks it for a slot by name instead of indexing a vector, and so
//! that the methods which read or write a slot are in one place.

use super::cursor::Cursor;
use super::{Dtype, Pickler, Shape, MAX_MEMO};

/// What a memo slot holds, as far as a form is prepared to say.
///
/// A slot a form cannot name is [`Bound::Opaque`], and a reference to one is
/// a non-match. Only the things a form built itself can be referred to
/// again, which is what keeps `BINGET` from being an arbitrary stack effect.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Bound {
    Opaque,
    Text { at: usize, len: usize },
    Bytes { at: usize, len: usize },
    /// A container the basic productions built: where the file wrote it, and
    /// whether Python could hash it. A list, a dictionary and a set are filed
    /// when they are created and before anything is put in them, so this is
    /// also what a container still being filled is named by.
    Made { what: Shape, at: usize, hashable: bool },
    /// A module and a callable the form named exactly, spelled `module.name`.
    Global(String),
    /// A completed NumPy dtype. The slot is written at the REDUCE that makes
    /// the dtype and the rest of it arrives at the BUILD just after, so the
    /// binding is filled in there.
    Dtype(Dtype),
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
    /// The memo mark that files the value just built, in whichever spelling
    /// the protocol has.
    ///
    /// Protocol 4 writes MEMOIZE and no index: the slot is the count of marks
    /// before it. Protocols 2 and 3 write BINPUT or LONG_BINPUT with the index
    /// in the file, and [`Cursor::put`] holds them to the number the slot
    /// actually has.
    pub(super) fn memoize(&mut self, bound: Bound) -> Option<()> {
        self.memoize_at(bound).map(|_| ())
    }

    /// The same, and which slot the value went in, for the one production that
    /// has to fill a slot in later. Nothing at all when the file left the mark
    /// out, which only `cPickle` does.
    pub(super) fn memoize_at(&mut self, bound: Bound) -> Option<Option<usize>> {
        if self.proto >= 4 {
            self.exact(&[0x94])?;
            return self.memo.bind(bound).map(Some);
        }
        self.put(bound)
    }

    /// BINPUT or LONG_BINPUT, and the slot number in it.
    ///
    /// The number is not read from the file and believed: a slot is the count
    /// of marks written before it, so the index has to be exactly that, offset
    /// by the number the file started counting at. Every pickler starts at
    /// nought but Python 2's `cPickle`, which starts at one; the first mark
    /// fixes which, and anything else is a non-match.
    ///
    /// A value may have no mark at all. `cPickle` leaves it out for a value
    /// nothing else in the program holds a reference to, which is why a fresh
    /// dictionary key of its is spelled again rather than named. No other
    /// pickler leaves one out, so this is read only while the numbering has
    /// not yet shown itself to start at nought. A missing mark files no slot,
    /// and the numbering carries on where it was.
    fn put(&mut self, bound: Bound) -> Option<Option<usize>> {
        let wide = match self.peek() {
            Some(0x71) => false,
            Some(0x72) => true,
            _ => {
                if self.memo_base == Some(0) {
                    return None;
                }
                self.skipped += 1;
                return Some(None);
            }
        };
        self.byte()?;
        let index = match wide {
            false => usize::from(self.byte()?),
            true => u32::from_le_bytes(self.take(4)?.try_into().ok()?) as usize,
        };

        let base = match self.memo_base {
            Some(base) => base,
            None => {
                let base = index.checked_sub(self.memo.len())?;
                if base > 1 || (base == 0 && self.skipped > 0) {
                    return None;
                }
                self.memo_base = Some(base);
                if base == 1 {
                    // Only `cPickle` numbers from one.
                    self.wrote(Pickler::C)?;
                }
                base
            }
        };
        if index != base + self.memo.len() {
            return None;
        }
        self.memo.bind(bound).map(Some)
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
    ///
    /// The number in the file counts from wherever the file's own marks count
    /// from, which is one for Python 2's `cPickle` and nought for every other
    /// pickler.
    pub(super) fn reference(&mut self) -> Option<&Bound> {
        let slot = match self.byte()? {
            b'h' => self.byte()? as usize,
            b'j' => u32::from_le_bytes(self.take(4)?.try_into().ok()?) as usize,
            _ => return None,
        };
        let slot = slot.checked_sub(self.memo_base.unwrap_or(0))?;
        self.memo.get(slot)
    }

    pub(super) fn at_reference(&self) -> bool {
        matches!(self.peek(), Some(b'h') | Some(b'j'))
    }

    /// A short text with exactly this spelling, and the memo mark that files
    /// it. Comes back as the bytes the word sits in.
    ///
    /// Protocol 4 counts a text's length in one byte where protocols 2 and 3
    /// write four, and each protocol has only its own spelling.
    pub(super) fn word(&mut self, text: &str) -> Option<(usize, usize)> {
        self.gate()?;
        match self.proto >= 4 {
            true => self.exact(&[0x8c, u8::try_from(text.len()).ok()?])?,
            false => {
                self.exact(&[0x58])?;
                self.exact(&u32::try_from(text.len()).ok()?.to_le_bytes())?;
            }
        }
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

    /// A module and a callable, joined by STACK_GLOBAL at protocol 4 or
    /// written as GLOBAL's two lines below it, or a reference to the slot the
    /// same pair was filed in earlier. `modules` is every spelling the form
    /// accepts, each written out.
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
        if self.proto < 4 {
            return self.global_lines(modules, name, module_says, name_says);
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

    /// GLOBAL, which is how protocols 2 and 3 name a callable: one opcode and
    /// two newline-terminated lines, filed in one slot rather than three.
    fn global_lines(
        &mut self,
        modules: &[&str],
        name: &str,
        module_says: &'static str,
        name_says: &'static str,
    ) -> Option<String> {
        let (module, name_at, name_len) = self.global_words(modules, name)?;
        self.says(module_says, module.1, module.2);
        self.says(name_says, name_at, name_len);
        let full = format!("{}.{name}", module.0);
        self.memoize(Bound::Global(full.clone()))?;
        Some(full)
    }

    /// The two lines of a GLOBAL, matched against the spellings a form
    /// accepts. Comes back as the module it matched and where each line sits.
    pub(super) fn global_words(&mut self, modules: &[&str], name: &str) -> Option<((String, usize, usize), usize, usize)> {
        self.exact(b"c")?;
        let (module_at, module_len) = self.line()?;
        let held = self.bytes.get(module_at..module_at + module_len)?;
        let module = modules.iter().find(|m| m.as_bytes() == held)?.to_string();
        let (name_at, name_len) = self.line()?;
        if self.bytes.get(name_at..name_at + name_len)? != name.as_bytes() {
            return None;
        }
        Some(((module, module_at, module_len), name_at, name_len))
    }
}
