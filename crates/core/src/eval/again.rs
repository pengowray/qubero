//! Asking a read that was refused for depth again, from the bottom up.
//!
//! A read is refused when its expressions nest past `go::DEEPEST_QUESTION`:
//! a computed field naming the one before it, ten thousand times over, read
//! from the far end with nothing known. Nothing on the way down was finished,
//! so nothing was remembered, and asking again from the top would be refused
//! at the same place. Read in order, the same chain is one link deep, because
//! every field it asks about already has its answer.
//!
//! So the outermost expression of a refused read asks it once more, this time
//! keeping one expression in every `go::KEPT_EVERY` on the way down, and then
//! asks those again itself, deepest first. Each is asked with the whole depth
//! to itself, and what it reads is remembered the way any read's is, so the
//! one above it finds its answers waiting and goes no deeper than the gap
//! between them. One that is refused again on its own leaves a trail of its
//! own, which is asked first. When every one has its answer, the outermost is
//! asked a last time.
//!
//! This follows the chain wherever it goes rather than guessing at it: from a
//! buffer to the node it belongs to, from that node back through every node
//! before it, into a list that is no ancestor of the field asked for.
//!
//! Nothing is kept while a read goes as it should, so the reads that are never
//! refused, which is nearly all of them, pay for none of it but the test of a
//! flag in each expression.
//!
//! What cannot be helped is refused as before. An expression nested past the
//! limit inside itself has nothing under it to remember, and a path longer
//! than `DEEPEST_PATH` is as long the second time; a question refused a
//! second time after everything under it was answered is where it stops.

use super::go::{Asked, Question, Refused};
use super::*;

/// The most questions kept waiting to be asked again at once. A chain long
/// enough to need more is refused, as every chain past the limit was before:
/// each kept question holds a copy of its expression, and a file can make the
/// chain as long as it likes.
const MOST_WAITING: usize = 1 << 14;

/// Whether an answer is a refusal for depth.
fn refused<T>(out: &R<T>) -> bool {
    matches!(out, Err(EvalError::Failed(why)) if Evaluator::is_refusal(why))
}

impl Evaluator {
    /// Work out the outermost expression of a read with `ask`, and when that
    /// is refused for depth, answer the questions it was asking from the
    /// deepest up and ask once more.
    ///
    /// Out of line and asked only at the top of a read, so none of it sits in
    /// the frame of an expression inside another.
    ///
    /// A refusal that comes back from here says what expression it stopped
    /// at, written here rather than where it stopped: see `refused_here`.
    #[inline(never)]
    pub(super) fn outermost<S: Source, T>(&mut self, doc: &Document<S>, ask: impl Fn(&mut Self) -> R<T>) -> R<T> {
        let first = ask(self);
        // Where this one stopped, or one passed over on the way: either way
        // nobody reads what it said.
        self.go.take_refused();
        if !refused(&first) {
            return first;
        }
        self.go.take_trail();
        self.go.set_asking_again(true);
        let second = ask(self);
        let second_stopped = self.go.take_refused();
        let worked = match refused(&second) {
            true => {
                let trail = self.go.take_trail();
                self.work_through(doc, trail)
            }
            // Answered this time: the first asking can leave enough
            // remembered for the second to get through.
            false => Ok(false),
        };
        self.go.take_trail();
        self.go.set_asking_again(false);
        self.go.take_refused();
        match worked {
            Ok(true) => {
                let last = ask(self);
                let stopped = self.go.take_refused();
                self.say_where(last, stopped)
            }
            Ok(false) => self.say_where(second, second_stopped),
            Err(e) => Err(e),
        }
    }

    /// A refusal for depth, with the expression it stopped at written into
    /// what it says. Anything else as it is.
    fn say_where<T>(&self, out: R<T>, stopped: Option<Refused>) -> R<T> {
        match (out, stopped) {
            (Err(EvalError::Failed(why)), Some(stopped)) if Self::is_refusal(&why) => {
                let full = self.asked_too_deep(&stopped.at, &stopped.expr);
                Err(EvalError::Failed(why.replacen(&stopped.said, &full, 1)))
            }
            (out, _) => out,
        }
    }

    /// Answer the questions on `trail` and on the trails any of them leave,
    /// deepest first. True when every one was answered, or failed for some
    /// reason of its own, which its asker will meet again and say. False when
    /// one was refused for depth a second time, or left no question to start
    /// from: asking the read again would only be refused again.
    ///
    /// An interruption is handed back as it is. What was answered before it
    /// is remembered, and the next go starts from further down.
    fn work_through<S: Source>(&mut self, doc: &Document<S>, trail: Vec<Question>) -> R<bool> {
        // Kept shallowest first, so the next one taken is the deepest.
        let mut waiting: Vec<(Question, bool)> = trail.into_iter().map(|q| (q, false)).collect();
        if waiting.is_empty() {
            return Ok(false);
        }
        while let Some((question, _)) = waiting.last() {
            let got = self.ask_again(doc, question);
            let deeper = self.go.take_trail();
            match got {
                Err(e) if e.interrupted() => return Err(e),
                Err(EvalError::Failed(why)) if Self::is_refusal(&why) => {
                    let tried = waiting.last_mut().is_none_or(|(_, tried)| std::mem::replace(tried, true));
                    if tried || deeper.is_empty() || waiting.len() + deeper.len() > MOST_WAITING {
                        return Ok(false);
                    }
                    waiting.extend(deeper.into_iter().map(|q| (q, false)));
                }
                _ => {
                    waiting.pop();
                }
            }
        }
        Ok(true)
    }

    /// Ask one kept question again, for what it leaves remembered.
    fn ask_again<S: Source>(&mut self, doc: &Document<S>, q: &Question) -> R<()> {
        match q.asked {
            Asked::Whole => self.eval_expr_at(doc, &q.at, &q.expr, q.here).map(drop),
            Asked::Real => self.eval_real_at(doc, &q.at, &q.expr, q.here).map(drop),
            Asked::Text => self.text_at(doc, &q.at, &q.expr, q.here).map(drop),
        }
    }
}
