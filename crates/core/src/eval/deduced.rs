//! Answers a field cannot reach by pointing at another field.
//!
//! Nearly every expression in the IR names something: a sibling, an element of
//! a list, a path down into a header. That works because nearly every format
//! writes what a field is beside the field. A pickle does not. It writes a
//! program, and what a run of bytes turns out to be is decided by opcodes that
//! never touch it, over a stack that exists only while the program runs.
//!
//! So [`Deduce`] is the small set of questions a template may ask about a
//! field that only running the container answers, and this is where the
//! asking is arranged. Not the answering: a template that has such fields
//! says what runs it, as a [`Deducer`](crate::template::Deducer), and nothing
//! here knows which format that is. What this owes the format is that the run
//! happens once per document and is kept beside the memo, the way a parsed
//! JSON header is: a file of a hundred thousand opcodes is walked once
//! however many rows a reader opens.
//!
//! Every answer has a nothing case, and the nothing case is always the honest
//! one. A format with nothing to run it, a run that stopped where a stack
//! stopped balancing, bytes no recogniser knows: all of them read as bytes,
//! which is what they were before any of this and what they are.

use std::sync::Arc;

use super::*;
use crate::template::{Deduce, Deduced};

impl Evaluator {
    /// What running the file said about it, running it if nothing has yet.
    ///
    /// Only for a template that said what runs it. A run is over a format
    /// written as a program, and asking it of anything else would be reading
    /// a JPEG as one.
    fn ran<S: Source>(&mut self, doc: &Document<S>) -> R<Arc<dyn Deduced>> {
        if let Some(r) = self.memo.deduced() {
            return Ok(r.clone());
        }
        let Some(deducer) = self.template.deducer.clone() else {
            return fail("this file is not being read as a format that runs");
        };
        // The whole file, because a format that has to be run is the whole
        // file and the answer for a byte near the end depends on everything
        // before it. Too large to hold is a file that reads as its own fields
        // and nothing more, which is an answer rather than a failure and is
        // worth remembering.
        let len = doc.len_bits() / 8;
        if len > most_bytes() {
            let empty: Arc<dyn Deduced> = Arc::new(Nothing);
            self.memo.remember_deduced(empty.clone());
            return Ok(empty);
        }
        // Through the evaluator's own read, which is what says a chunk has
        // not arrived yet. Reading the document directly would hand the
        // deducer a run of zeros where the file has not been fetched, and the
        // answer worked out from those would then be remembered as if it were
        // the file's. `Pending` goes back up and the caller asks again.
        let bytes = self.read_in(doc, 0, 0, len * 8)?;
        let run = deducer.run(&bytes);
        self.memo.remember_deduced(run.clone());
        Ok(run)
    }

    /// Where the bytes this expression is asked about start, as a byte offset
    /// in the file.
    ///
    /// The answers are keyed by offset rather than by path, because that is
    /// what the machine and the listing agree on: the machine knows a payload
    /// by where it is, and a node knows where it is. A path would have to be
    /// counted through frames and switches, and would break the first time the
    /// template's shape changed.
    fn deduced_at(&self, at: &[usize], here: Option<(u64, u64)>) -> Option<u64> {
        let offset = here.map(|(o, _)| o).or_else(|| self.memo.get(at).map(|r| r.offset))?;
        Some(offset / 8)
    }

    /// A number the run of the file answers.
    pub(super) fn deduced_int<S: Source>(
        &mut self,
        doc: &Document<S>,
        at: &[usize],
        what: Deduce,
        here: Option<(u64, u64)>,
    ) -> R<i128> {
        let Some(offset) = self.deduced_at(at, here) else { return Ok(NOTHING) };
        let run = self.ran(doc)?;
        Ok(run.int(what, offset).unwrap_or(NOTHING))
    }

    /// A word the run of the file answers, for the rows that would otherwise
    /// say only that an opcode happened.
    pub(super) fn deduced_text<S: Source>(
        &mut self,
        doc: &Document<S>,
        at: &[usize],
        what: Deduce,
        here: Option<(u64, u64)>,
    ) -> R<String> {
        if what != Deduce::Builds {
            return Ok(String::new());
        }
        let Some(offset) = self.deduced_at(at, here) else { return Ok(String::new()) };
        let run = self.ran(doc)?;
        Ok(run.text(what, offset).unwrap_or_default())
    }
}

/// A run that said nothing, for a file too large to be run at all.
///
/// Every question has a nothing case and this answers all of them with it, so
/// a file past [`most_bytes`] reads exactly as one whose run recognised none
/// of its bytes: as the bytes it holds.
struct Nothing;

impl Deduced for Nothing {
    fn int(&self, _what: Deduce, _at: u64) -> Option<i128> {
        None
    }
    fn text(&self, _what: Deduce, _at: u64) -> Option<String> {
        None
    }
}

/// What a deduced number reads as when nothing was deduced. Zero rather than
/// -1, so that a format can put its own nothing case first in the table it
/// switches on: a `Switch` then reaches the honest answer at a known position
/// rather than scanning a thousand cases to miss all of them. `pickle::shapes`
/// is written that way.
const NOTHING: i128 = 0;

/// The largest file run as a program. Past this the opcodes are still listed
/// and placed; only the annotation stops, which is the part that costs memory
/// proportional to what is in the file rather than to what is on screen.
///
/// Lower in the browser, where the whole address space is four gigabytes and
/// a copy of the file is a copy the tab can fail to make.
fn most_bytes() -> u64 {
    match cfg!(target_pointer_width = "32") {
        true => 64 << 20,
        false => 256 << 20,
    }
}
