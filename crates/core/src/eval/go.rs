//! What one go of reading is allowed to spend, and what it has spent.
//!
//! A caller with a screen to draw asks for the answer a few thousand elements
//! at a time: the reading hands back `Busy` when the allowance runs out, says
//! how far it got, and carries on where it stopped when asked again. That
//! allowance, how far the reading has reached, the bytes an answer was given
//! without, and how much stack the reading is holding are all the same
//! question asked of one go, so they are kept together and away from what the
//! reading has worked out, which outlives any number of goes.

use crate::template::Expr;

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
/// A read refused here is not the end of it. Its outermost expression asks it
/// again a stretch of the chain at a time, from the bottom up, so that each
/// stretch finds the one below it answered; see `eval::again`. What stays
/// refused is what no order of asking helps: an expression nested this deep
/// in itself.
///
/// Counted per expression rather than per field reached, so the arithmetic
/// between two fields counts too: a lookup written as twenty-six conditions is
/// twenty-six deep, and spends the stack that way.
///
/// The limit is what the dearest shape fits in `STACK_BUDGET`, the room a read
/// is given with some left for whoever called in. It was set when the dearest
/// cost 7.3 KiB an expression, found by shrinking a thread of one megabyte
/// until a chain ran out: 640 KiB at 7.3 KiB each is 88.
///
/// Measured again on 2026-09-14, after an expression came to read a field's
/// value without the rest of its `NodeInfo`, in a release build like the one
/// wasm ships, by filling the stack with a known byte and counting how much of
/// it a read wrote over. The dearest shapes are an element whose type is a
/// switch on the element before it, and a switch on a path into the element
/// before found by a search back, at 5.1 KiB an expression, and read 88 deep
/// they wrote over 453 KiB. A length taken from the element before costs 3.5
/// KiB, a computed field naming the one before 3.3, a `prev` chain and a
/// search by label about 2.2, spending two expressions a link. By that
/// arithmetic the limit could be 124, and it stays at 88: a read refused here
/// is asked again, so a higher limit would save only some of that asking,
/// and the dearest shapes have not been measured in a wasm build. The deepest
/// real reading is 52, an Arrow file with many columns read from its last
/// buffer first, the same in a sweep of every sample in `check_tree` and
/// `spans_probe` and in the sample tests. One of that file's field nodes read
/// with nothing asked before it goes past the limit and is asked again.
///
/// That one was measured in wasm, as the web app builds it, run in Node 26:
/// the column of `more-types.arrow`'s last node read first wrote over 94 KiB
/// of the megabyte of stack rust-lld gives the module, against 178 KiB of a
/// native release build: the stack is the first megabyte of wasm memory, so
/// it was filled with a known byte before the read and counted after it.
///
/// A debug build's frames are six to ten times as large: read to the limit, a
/// chain of computed fields wrote over 2.8 MiB there. Tests run on the stack
/// `.cargo/config.toml` gives them, which is far more, except the ones that
/// read to the limit on purpose: `deep_questions` and the Arrow nodes read
/// first in `arrow_real` read on threads of 640 KiB in a release build and 4
/// MiB in a debug one. A read whose stack per expression grows by half fails
/// `deep_questions` in either build; the Arrow read takes less, 178 KiB and
/// 2.0 MiB, and fails a debug build when its stack about doubles.
///
/// `cargo run --release --example stack_probe -- <levels> <shape> <KiB> [paint]`
/// is where these numbers come from.
pub(super) const DEEPEST_QUESTION: usize = 88;

/// How far apart the questions kept while a refused read is asked again are,
/// counted in expressions. Asking one of them again reads down to the one kept
/// below it, which by then has its answer, so this is how deep that asking
/// goes: far enough under `DEEPEST_QUESTION` to leave room for the arithmetic
/// around a field, and far enough apart that a chain ten thousand long keeps a
/// few hundred copies of an expression rather than ten thousand.
pub(super) const KEPT_EVERY: usize = 16;

/// Which of the three readings of an expression a question asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Asked {
    Whole,
    Real,
    Text,
}

/// One expression a read was working out when it was refused for depth,
/// kept so that it can be asked again on its own: where it was asked, what,
/// the frame it was asked in, and how many expressions were open around it.
#[derive(Clone, Debug)]
pub(super) struct Question {
    pub(super) at: Vec<usize>,
    pub(super) expr: Expr,
    pub(super) here: Option<(u64, u64)>,
    pub(super) asked: Asked,
    pub(super) depth: usize,
}

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
    /// True while a refused read is being asked again. Then every expression
    /// open at a depth that is a multiple of `KEPT_EVERY` is kept on
    /// `trail`, shallowest first, and taken off again when it is answered;
    /// and no expression starts a second round of asking again inside it.
    /// See `Evaluator::outermost`. Nothing is kept otherwise, so a read that
    /// is never refused pays for none of this but the test of the flag.
    asking_again: bool,
    trail: Vec<Question>,
    /// A refusal for depth has been met, so the trail is left as it stood
    /// when it was, rather than emptied as the refusal passes out. An
    /// expression opened after it means the refusal was passed over, and
    /// lets the trail go on.
    held: bool,
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
        self.asking_again = false;
        self.trail.clear();
        self.held = false;
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
        if self.asking_again {
            self.opened();
        }
        true
    }

    /// That expression has its answer, or its error. Only ever called where
    /// `ask` said yes.
    #[inline]
    pub(super) fn answered<T>(&mut self, out: &R<T>) {
        if self.asking_again {
            let refused = matches!(out, Err(EvalError::Failed(why)) if super::Evaluator::is_refusal(why));
            self.closed(refused);
        }
        self.asking -= 1;
    }

    /// The most expressions that have been open inside one another at once.
    pub(super) fn deepest_asked(&self) -> usize {
        self.deepest_asked
    }

    /// Whether an expression about to be worked out is the outermost one of
    /// its read, and not one being asked again for a refused read.
    pub(super) fn outermost(&self) -> bool {
        self.asking == 0 && !self.asking_again
    }

    /// Whether the expression just opened is one the trail keeps.
    pub(super) fn keeps_this_one(&self) -> bool {
        self.asking_again && self.asking % KEPT_EVERY == 0
    }

    /// Keep the expression just opened on the trail.
    pub(super) fn keep(&mut self, at: &[usize], expr: &Expr, here: Option<(u64, u64)>, asked: Asked) {
        let depth = self.asking;
        self.trail.push(Question { at: at.to_vec(), expr: expr.clone(), here, asked, depth });
    }

    /// An expression has been opened while asking again. One opened after a
    /// refusal means the refusal was passed over, so what the trail held for
    /// it goes.
    #[cold]
    fn opened(&mut self) {
        if self.held {
            self.held = false;
            let depth = self.asking;
            self.trail.retain(|q| q.depth < depth);
        }
    }

    /// An expression has been answered while asking again: taken off the
    /// trail if it was kept, or the trail held as it stands if the answer is a
    /// refusal for depth, whether that started here or further in.
    #[cold]
    fn closed(&mut self, refused: bool) {
        if refused {
            self.held = true;
        } else if !self.held && self.trail.last().is_some_and(|q| q.depth == self.asking) {
            self.trail.pop();
        }
    }

    /// The expression that would have been one too many, which is where a
    /// refusal for depth starts: kept on the trail as its deepest question.
    pub(super) fn refused_here(&mut self, at: &[usize], expr: &Expr, here: Option<(u64, u64)>, asked: Asked) {
        if !self.asking_again {
            return;
        }
        let depth = self.asking + 1;
        self.trail.retain(|q| q.depth < depth);
        self.trail.push(Question { at: at.to_vec(), expr: expr.clone(), here, asked, depth });
        self.held = true;
    }

    /// The trail as the last refusal left it, shallowest first, and none left
    /// behind.
    pub(super) fn take_trail(&mut self) -> Vec<Question> {
        self.held = false;
        std::mem::take(&mut self.trail)
    }

    /// Say whether the questions of a refused read are being asked again.
    pub(super) fn set_asking_again(&mut self, again: bool) {
        self.asking_again = again;
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
