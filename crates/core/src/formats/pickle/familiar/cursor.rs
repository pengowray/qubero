//! The cursor a form is matched with: the bytes, how far it has read, what is
//! left of the work budget, and the frames CPython wrote the file in. Every
//! production reads bytes through this one, so it sits apart from all of them.

use super::memo::Memo;
use super::forms::Allow;
use super::{BIG_PAYLOAD, Call, Kind, MAX_LINE, NO_OPCODE, Pickler, Said, Value};
use crate::formats::pickle::known::Payload;

pub(super) struct Cursor<'a> {
    pub(super) bytes: &'a [u8],
    pub(super) at: usize,
    pub(super) left: usize,
    /// The protocol the file declared, which decides between spellings that
    /// exist at one protocol and not the other.
    pub(super) proto: u8,
    /// The named operands the form has matched so far inside its fixed runs.
    pub(super) says: Vec<Said>,
    /// What the file has filed under each memo slot, in the order it wrote
    /// them. A slot number is the count of memo marks before it, so every
    /// mark a production consumes has to land here or the numbering drifts.
    pub(super) memo: Memo,
    /// The number the file gave its first memo slot, which is nought for
    /// every pickler but Python 2's `cPickle` and one for that. Nothing at
    /// protocol 4 writes a slot number at all, so this stays `None` there.
    pub(super) memo_base: Option<usize>,
    /// How many values the file filed no slot for. `cPickle` leaves the mark
    /// out for a value nothing else holds a reference to, and it is the only
    /// pickler that leaves one out at all.
    pub(super) skipped: usize,
    /// Which way the dictionaries of a file `cPickle` wrote end, which its two
    /// implementations do not agree on. Only read when [`Cursor::memo_base`]
    /// is one; every other pickler's dictionaries and lists agree and are held
    /// to [`Cursor::pickler`].
    pub(super) dicts: Pickler,
    pub(super) framing: Framing,
    /// Which pickler the spellings seen so far belong to. The two are told
    /// apart only at a batch edge and at the memo mark after a bytearray, so
    /// this stays [`Pickler::Undetermined`] for most files.
    pub(super) pickler: Pickler,
    pub(super) allow: Allow,
    pub(super) calls: Vec<Call>,
    pub(super) payloads: Vec<(usize, Payload)>,
    /// The bytes of every run the file wrote as something other than bytes,
    /// decoded once as the run is read. See [`Match::decoded`].
    pub(super) runs: Vec<(usize, std::sync::Arc<Vec<u8>>)>,
    /// How many of each specific production fired, which is what says the
    /// file belongs to the form that allows it.
    pub(super) arrays: usize,
    pub(super) objects: usize,
    /// How many objects of a named class the file built, which is what says a
    /// library form read what it is for.
    pub(super) instances: usize,
    pub(super) furthest: usize,
}

/// Where a frame boundary stands. CPython writes a pickle as a run of frames:
/// a frame is committed when it fills, and a payload of [`BIG_PAYLOAD`] bytes
/// or more is written between frames instead of inside one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Framing {
    /// The file has no frames at all, which protocol 4 permits.
    Unframed,
    /// Inside a frame ending at this offset.
    Inside(usize),
    /// Inside a frame that holds a payload too large to frame, ending at this
    /// offset. Python 3.4 to 3.6 wrote such a payload into the frame it was
    /// filling and committed that frame at the next object, so the frame ends
    /// at the next object boundary and nowhere else.
    Full(usize),
    /// A frame has ended here and the next has not begun. The next thing is a
    /// FRAME header, or the payload too large to frame that ended it.
    Between(usize),
    /// After a payload written between frames. What follows is a new frame, or
    /// the end of the file within [`MIN_FRAME`] bytes, which is the one run of
    /// unframed bytes CPython writes without a header in front of it.
    Tail(usize),
}

/// Everything a rewind has to put back. The work budget is deliberately not
/// in it: an alternative that failed still cost what it cost.
#[derive(Debug, Clone, Copy)]
pub(super) struct Save {
    pub(super) at: usize,
    pub(super) memo: usize,
    pub(super) memo_base: Option<usize>,
    pub(super) skipped: usize,
    pub(super) dicts: Pickler,
    pub(super) says: usize,
    pub(super) calls: usize,
    pub(super) payloads: usize,
    pub(super) runs: usize,
    pub(super) framing: Framing,
    pub(super) pickler: Pickler,
    pub(super) arrays: usize,
    pub(super) objects: usize,
    pub(super) instances: usize,
}

impl<'a> Cursor<'a> {
    pub(super) fn take(&mut self, len: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(len)?;
        let out = self.bytes.get(self.at..end)?;
        self.at = end;
        if end > self.furthest {
            self.furthest = end;
        }
        Some(out)
    }
    pub(super) fn byte(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    pub(super) fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }
    pub(super) fn exact(&mut self, expected: &[u8]) -> Option<()> {
        (self.take(expected.len())? == expected).then_some(())
    }

    /// A newline-terminated run, which is how protocols 2 and 3 write a module
    /// name, a callable's name and an integer too wide for `BININT`. Comes
    /// back as the bytes before the newline.
    ///
    /// A line is bounded: nothing a form reads as one is longer than a module
    /// path or a number's digits, and an unterminated run would otherwise walk
    /// the whole file.
    pub(super) fn line(&mut self) -> Option<(usize, usize)> {
        let at = self.at;
        let end = self.bytes.get(at..at + MAX_LINE.min(self.bytes.len() - at.min(self.bytes.len())))?.iter().position(|b| *b == b'\n')?;
        self.take(end + 1)?;
        Some((at, end))
    }

    /// A run of fixed bytes in which each piece is an object of its own.
    ///
    /// CPython's framer ends a frame at the start of every `save`, and a
    /// value inside a fixed run, such as the `(0,)` a NumPy array is
    /// reconstructed with or the eight values a dtype's state holds, is a
    /// `save` like any other. So a frame may end in front of any of these
    /// pieces, and the boundary is taken before each one rather than only
    /// before the run.
    pub(super) fn atoms(&mut self, pieces: &[&[u8]]) -> Option<()> {
        for piece in pieces {
            self.gate()?;
            self.exact(piece)?;
        }
        Some(())
    }

    pub(super) fn save(&self) -> Save {
        Save {
            at: self.at,
            memo: self.memo.len(),
            memo_base: self.memo_base,
            skipped: self.skipped,
            dicts: self.dicts,
            says: self.says.len(),
            calls: self.calls.len(),
            payloads: self.payloads.len(),
            runs: self.runs.len(),
            framing: self.framing,
            pickler: self.pickler,
            arrays: self.arrays,
            objects: self.objects,
            instances: self.instances,
        }
    }

    /// What the spelling just read says about which pickler wrote the file.
    ///
    /// Both spellings are real, and a file is written by one pickler, so the
    /// first one seen fixes the reading: a file that shows the C pickler at
    /// one batch edge and `pickle.py` at another was written by neither.
    pub(super) fn wrote(&mut self, which: Pickler) -> Option<()> {
        match self.pickler {
            Pickler::Undetermined => {
                self.pickler = which;
                Some(())
            }
            seen => (seen == which).then_some(()),
        }
    }

    /// Put everything back except the work already spent, which an
    /// alternative that failed does not get to spend again.
    pub(super) fn restore(&mut self, s: Save) {
        self.at = s.at;
        self.memo.truncate(s.memo);
        self.memo_base = s.memo_base;
        self.skipped = s.skipped;
        self.dicts = s.dicts;
        self.says.truncate(s.says);
        self.calls.truncate(s.calls);
        self.payloads.truncate(s.payloads);
        self.runs.truncate(s.runs);
        self.framing = s.framing;
        self.pickler = s.pickler;
        self.arrays = s.arrays;
        self.objects = s.objects;
        self.instances = s.instances;
    }

    /// FRAME and its eight-byte length, which must land inside the file.
    pub(super) fn frame_header(&mut self) -> Option<()> {
        self.exact(&[0x95])?;
        let size = u64::from_le_bytes(self.take(8)?.try_into().ok()?);
        let end = self.at.checked_add(usize::try_from(size).ok()?)?;
        if end > self.bytes.len() {
            return None;
        }
        self.framing = Framing::Inside(end);
        Some(())
    }

    /// The boundary between two objects, where a frame may end and the next
    /// one begin. Every object production starts here.
    pub(super) fn gate(&mut self) -> Option<()> {
        match self.framing {
            Framing::Inside(end) => {
                if self.at > end {
                    return None;
                }
                if self.at == end {
                    self.framing = Framing::Between(end);
                }
            }
            // A frame holding an oversized payload is committed at the next
            // object, which is here, so the frame ends exactly here.
            Framing::Full(end) => {
                if self.at != end {
                    return None;
                }
                self.framing = Framing::Between(end);
            }
            _ => {}
        }
        if matches!(self.framing, Framing::Between(_) | Framing::Tail(_)) && self.peek() == Some(0x95) {
            self.frame_header()?;
        }
        Some(())
    }

    /// A counted run of bytes, and the framing a large one demands.
    ///
    /// The opcode byte has already been read and sits at `self.at - 1`.
    ///
    /// From Python 3.7 a payload of [`BIG_PAYLOAD`] bytes or more begins
    /// exactly where a frame ended and a new frame begins after it. Python
    /// 3.4 to 3.6 had no such path and wrote the payload into the frame it
    /// was filling, which is then committed at the next object; both are
    /// read, and the second is what [`Framing::Full`] holds the file to. A
    /// payload smaller than that is never written between frames.
    pub(super) fn counted(&mut self, code: u8, short: u8, wide: u8, widest: u8) -> Option<(usize, usize)> {
        let opcode_at = self.at - 1;
        let between = matches!(self.framing, Framing::Between(from) if from == opcode_at);
        let len = self.length(code, short, wide, widest)?;
        if between && len < BIG_PAYLOAD {
            return None;
        }
        let at = self.at;
        self.take(len)?;
        if between {
            // The writer starts a new frame right after the bytes it wrote
            // between frames, unless what is left is too short to frame.
            self.framing = Framing::Tail(self.at);
            self.gate()?;
        } else if len >= BIG_PAYLOAD {
            if let Framing::Inside(end) = self.framing {
                self.framing = Framing::Full(end);
            }
        }
        Some((at, len))
    }

    fn length(&mut self, code: u8, short: u8, wide: u8, widest: u8) -> Option<usize> {
        let n = if code == short {
            u64::from(self.byte()?)
        } else if code == wide {
            u32::from_le_bytes(self.take(4)?.try_into().ok()?) as u64
        } else if code == widest {
            u64::from_le_bytes(self.take(8)?.try_into().ok()?)
        } else {
            return None;
        };
        usize::try_from(n).ok()
    }

    /// A counted run of text in the spelling the protocol has, with no memo
    /// mark after it: what a fixed run reads where it knows what the text has
    /// to say. Protocol 4 counts the length in one byte, protocols 2 and 3 in
    /// four.
    pub(super) fn text_run(&mut self) -> Option<(usize, usize)> {
        self.gate()?;
        // A word Python 2 wrote is a `str` rather than text, so at protocol 2
        // the same word has two spellings and the writer decides which.
        if self.proto <= 2 && matches!(self.peek(), Some(b'U') | Some(b'T')) {
            let code = self.byte()?;
            return self.counted(code, b'U', b'T', NO_OPCODE);
        }
        let len = match self.proto >= 4 {
            true => {
                self.exact(&[0x8c])?;
                usize::from(self.byte()?)
            }
            false => {
                self.exact(&[0x58])?;
                u32::from_le_bytes(self.take(4)?.try_into().ok()?) as usize
            }
        };
        let at = self.at;
        self.take(len)?;
        Some((at, len))
    }

    /// A value and the bytes it was written in, which is everything the
    /// production consumed.
    pub(super) fn span(&self, start: usize, kind: Kind) -> Value {
        Value {
            at: start,
            len: self.at - start,
            kind,
        }
    }

    /// Remember a named operand inside the run of instructions being matched.
    pub(super) fn says(&mut self, name: &'static str, at: usize, len: usize) {
        self.says.push(Said { name, at, len });
    }

    /// The run of instructions just matched, with the names spelled inside it.
    pub(super) fn finish_call(&mut self, name: &'static str, at: usize, end: usize) {
        let says = std::mem::take(&mut self.says);
        self.calls.push(Call { name, at, len: end - at, says });
    }
}
