//! What one go of reading is allowed to spend, and what it has spent.
//!
//! A caller with a screen to draw asks for the answer a few thousand elements
//! at a time: the reading hands back `Busy` when the allowance runs out, says
//! how far it got, and carries on where it stopped when asked again. That
//! allowance, how far the reading has reached, the bytes an answer was given
//! without, and how much stack the reading is holding are all the same
//! question asked of one go, so they are kept together and away from what the
//! reading has worked out, which outlives any number of goes.

use super::{EvalError, Missing, R, fail};

/// How much of the stack one read may spend, in bytes.
///
/// `DEEPEST_PATH` is a promise made from two measured shapes. This keeps the
/// promise when a third shape costs more per component than either of them:
/// the read stops and says so, rather than the process going down under it.
/// For everything measured the count is reached first and this never fires,
/// which is what makes it a backstop rather than the limit.
///
/// 640 KiB fits in the megabyte wasm is given with room left for whoever
/// called in, and carries about 175 components of the dearest measured shape
/// in a debug build, which is comfortably past the 128 the count allows.
const STACK_BUDGET: usize = 640 << 10;

#[derive(Default)]
pub(super) struct Go {
    /// Elements left before this go has to hand back, and how many each go is
    /// allowed. None works to the end, which is what a caller with nothing to
    /// draw meanwhile wants.
    left: Option<u64>,
    slice: Option<u64>,
    /// How far into the file the reading has got, at its furthest.
    reached_bits: u64,
    /// Bytes an answer was given without: previews that have not arrived. The
    /// caller fetches them and asks again, and meanwhile has its rows.
    wanted: Vec<Missing>,
    /// How many reads are open, and where the stack was when the outermost of
    /// them started. See `STACK_BUDGET`.
    nest: usize,
    stack_base: usize,
}

impl Go {
    /// Work in goes of `elements` at a time. None works until the answer is
    /// ready however long that takes.
    pub(super) fn set_slice(&mut self, elements: Option<u64>) {
        self.slice = elements;
        self.left = elements;
    }

    /// Start another go: the allowance is refilled and the list of bytes to
    /// fetch starts again. What has been worked out already is not touched.
    pub(super) fn begin(&mut self) {
        self.left = self.slice;
        self.wanted.clear();
        // A go that is starting has no read open, whatever a panic part way
        // through the last one may have left behind.
        self.nest = 0;
    }

    /// The same, and back to the start of the file, for when what was worked
    /// out has been thrown away as well.
    pub(super) fn restart(&mut self) {
        self.begin();
        self.reached_bits = 0;
    }

    /// Bytes wanted for previews that were answered without them, since the
    /// last `begin`. Fetching these and asking again fills them in.
    pub(super) fn wanted(&self) -> Vec<Missing> {
        let mut out = self.wanted.clone();
        out.sort_by_key(|m| m.chunk);
        out.dedup();
        out
    }

    pub(super) fn want(&mut self, missing: Vec<Missing>) {
        self.wanted.extend(missing);
    }

    /// How far into the file the reading has got, at its furthest.
    pub(super) fn reached_bits(&self) -> u64 {
        self.reached_bits
    }

    /// Charge one element against this go's allowance, and note how far the
    /// reading has reached.
    pub(super) fn spend(&mut self, at_bits: u64) -> R<()> {
        self.reached_bits = self.reached_bits.max(at_bits);
        let Some(left) = self.left.as_mut() else { return Ok(()) };
        if *left == 0 {
            return Err(EvalError::Busy { reached_bits: self.reached_bits });
        }
        *left -= 1;
        Ok(())
    }

    /// Note that one more read is open, and refuse it if the stack this go has
    /// spent is past what a read may. `depth` is only for what it says.
    ///
    /// The first read of a go is where the stack was when the reading started,
    /// and every read under it is that far down from there. Which call this
    /// belongs to is the point: placing a node returns before what it placed
    /// is measured, so `size_of` is the one that stays open the whole way
    /// down, and `resolve` on its own goes no deeper than the path is long.
    ///
    /// Paired with `leave`, and only ever by `Evaluator::reading`, so that
    /// no reading can forget the other half.
    pub(super) fn enter(&mut self, depth: usize) -> R<()> {
        let probe = 0u8;
        let here = &probe as *const u8 as usize;
        if self.nest == 0 {
            self.stack_base = here;
        } else if self.stack_base.saturating_sub(here) > STACK_BUDGET {
            return fail(format!("nested too deep to read: ran out of stack {depth} fields down"));
        }
        self.nest += 1;
        Ok(())
    }

    /// That read is over. Only ever called where `enter` said yes.
    pub(super) fn leave(&mut self) {
        self.nest -= 1;
    }

    /// Say that the reading started at the very top of memory, so that the
    /// next read is past any budget however shallow it really is. For the test
    /// of the backstop, which no file shape measured can reach.
    #[cfg(test)]
    pub(super) fn pretend_out_of_room(&mut self) {
        self.nest = 1;
        self.stack_base = usize::MAX;
    }
}
