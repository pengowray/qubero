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

/// How many expressions one read may have open inside one another.
///
/// The other recursion a read has, beside the nesting of the file that
/// `DEEPEST_PATH` counts. A computed field naming the field before it, a switch
/// keyed on the element before, a search back through a list for a label: each
/// of those reads a field whose own answer may ask another, and nothing on the
/// way down is finished, so nothing is remembered to stop it. Read in order
/// the chain is one link long, since what came before is known; read from a
/// cold start at the far end it is as long as the file is. That costs the
/// stack whether or not the path gets any longer, so `DEEPEST_PATH` never sees
/// it, and neither does `STACK_BUDGET`, which is only measured while a size is
/// being worked out. In wasm a stack that runs out takes the whole module
/// with it.
///
/// Counted per expression rather than per field reached, so the arithmetic
/// between two fields counts too: a lookup written as twenty-six conditions is
/// twenty-six deep, and spends the stack that way.
///
/// Measured on 2026-09-14, in a release build like the one wasm ships, on a
/// thread of one megabyte. The dearest shape per expression is an element
/// whose type is a switch on the element before it, which ran out at 142
/// expressions; a switch on a path into the element before, found by a search
/// back, at 191; a computed field naming the one before at 202; a `prev` chain
/// at about 300 and a search by label at about 350, which spend two
/// expressions a link. A length taken from the element before is stopped by
/// `STACK_BUDGET` first, at 104. So the limit is what the
/// dearest of them fits in `STACK_BUDGET`, the room a read is given with some
/// left for whoever called in: 640 KiB at 7.3 KiB each is 88. The deepest real
/// reading is 52, an Arrow file with many columns read from its last buffer
/// first, the same in a sweep of every sample in `check_tree` and
/// `spans_probe` and in the sample tests.
///
/// A debug build's frames are about seven times as large, and a megabyte
/// carries 28 of the dearest shape there. Tests run on the stack
/// `.cargo/config.toml` gives them, and nothing ships in debug.
///
/// `cargo run --release --example stack_probe -- <levels> <shape> <KiB>` is
/// where these numbers come from.
pub(super) const DEEPEST_QUESTION: usize = 88;

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
    /// How many expressions are being worked out inside one another, and the
    /// most there have been since the evaluator was made. See
    /// `DEEPEST_QUESTION`.
    asking: usize,
    deepest_asked: usize,
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
        self.asking = 0;
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

    /// Note that one more expression is being worked out inside the ones
    /// already open, or say no when that would be more than a read may have.
    ///
    /// Paired with `answered`, and only ever by `Evaluator::eval_expr_at` and
    /// its two siblings, which keep the pair around the one call between them.
    pub(super) fn ask(&mut self) -> bool {
        if self.asking >= DEEPEST_QUESTION {
            return false;
        }
        self.asking += 1;
        self.deepest_asked = self.deepest_asked.max(self.asking);
        true
    }

    /// That expression has its answer, or its error. Only ever called where
    /// `ask` said yes.
    pub(super) fn answered(&mut self) {
        self.asking -= 1;
    }

    /// The most expressions that have been open inside one another at once.
    pub(super) fn deepest_asked(&self) -> usize {
        self.deepest_asked
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
