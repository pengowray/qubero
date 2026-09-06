//! Answers a field cannot reach by pointing at another field.
//!
//! Nearly every expression in the IR names something: a sibling, an element of
//! a list, a path down into a header. That works because nearly every format
//! writes what a field is beside the field. A pickle does not. It writes a
//! program, and what a run of bytes turns out to be is decided by opcodes that
//! never touch it, over a stack that exists only while the program runs.
//!
//! So [`Deduce`] is the small set of questions a template may ask about a
//! field that only running the container answers, and this is where they are
//! answered. The run happens once per document and is kept beside the memo,
//! the way a parsed JSON header is: a file of a hundred thousand opcodes is
//! walked once however many rows a reader opens.
//!
//! Every answer has a nothing case, and the nothing case is always the honest
//! one. A file that is not a pickle, a pickle whose stack does not balance,
//! bytes no recogniser knows: all of them read as bytes, which is what they
//! were before any of this and what they are.

use std::sync::Arc;

use super::*;
use crate::formats::pickle::{self, machine};
use crate::template::Deduce;

impl Evaluator {
    /// What running the file said about it, running it if nothing has yet.
    ///
    /// Only for a document being read as a pickle: the run is over opcodes,
    /// and asking it of anything else would be reading a JPEG as a program.
    fn ran<S: Source>(&mut self, doc: &Document<S>) -> R<Arc<machine::Reading>> {
        if let Some(r) = self.memo.deduced() {
            return Ok(r.clone());
        }
        if self.template.name != "pickle" {
            return fail("this file is not being read as a pickle");
        }
        // The whole file, because a pickle is the whole file and the answer
        // for a byte near the end depends on every opcode before it. Too large
        // to hold is a file that reads as opcodes and nothing more, which is
        // an answer rather than a failure and is worth remembering.
        let len = doc.len_bits() / 8;
        if len > most_bytes() {
            let empty = Arc::new(machine::Reading::default());
            self.memo.remember_deduced(empty.clone());
            return Ok(empty);
        }
        // Through the evaluator's own read, which is what says a chunk has
        // not arrived yet. Reading the document directly would hand the
        // machine a run of zeros where the file has not been fetched, and the
        // answer worked out from those would then be remembered as if it were
        // the file's. `Pending` goes back up and the caller asks again.
        let bytes = self.read_in(doc, 0, 0, len * 8)?;
        let run = Arc::new(machine::run(&pickle::opcodes(&bytes)));
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
        let payload = run.arrays.get(&offset);
        Ok(match (what, payload) {
            (Deduce::PayloadShape, Some(p)) => p.shape as i128,
            (Deduce::PayloadCount, Some(p)) => p.count as i128,
            // A field asking for a number and getting a word is a template
            // fault rather than a file's, and reads as nothing rather than as
            // a guess.
            (Deduce::Builds, _) | (_, None) => NOTHING,
        })
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
        // The word is filed over the bytes the opcode covers, and the field
        // asking is the last of that opcode's fields and no bytes wide, so it
        // sits at the byte after the opcode rather than inside it. The byte
        // before that is the opcode's own last, whether the opcode was one
        // byte or five.
        Ok(run.builds_at(offset.saturating_sub(1)).unwrap_or_default().to_string())
    }
}

/// What a deduced number reads as when nothing was deduced. Negative, so a
/// `Switch` over a table of cases falls to its default rather than landing on
/// case zero.
const NOTHING: i128 = -1;

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
