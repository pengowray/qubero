//! What the walk in `kinds.rs` tells anyone else who wants to follow it.
//!
//! That walk is the one place that accounts for every byte of a file exactly
//! once: it defers what an offset placed until the fields laid out in order are
//! done, passes over second readings, counts a run of same-shaped elements once
//! and multiplies, and forgets what it has gone past. The report's byte ledger,
//! its format profile and its extent audit all need that same accounting, so
//! they follow the walk rather than taking one of their own. A second walk
//! would be a second place for a byte to be counted twice.
//!
//! The walk's one rule is that everything that can want bytes or run out of the
//! allowance is asked before anything is written down, so that a go which
//! stops part way redoes one child and counts nothing twice. The hooks keep to
//! it: the ones that return `R` are asked before the walk writes anything
//! about the node they name and may be asked again for the same node on the
//! next go, so they must only work things out, never add them up. The rest are
//! told after the fact and cannot fail.

use super::*;

/// A frame the walk is about to close, and what its children left over.
pub(super) struct Closing<'a> {
    pub path: &'a [usize],
    /// Where the node starts and ends. The root's end is the file's end.
    pub offset: u64,
    pub end: u64,
    /// Where the children laid out in order reached.
    pub cursor: u64,
    /// Bits the children left over, which are a gap unless `framed`.
    pub leftover: u64,
    /// Whether the leftover is one stretch, from `cursor` to `end`. A region
    /// whose children an offset scattered leaves over bits in no one place.
    pub stretch: bool,
    /// Whether the leftover is the node's own punctuation: a JSON object's
    /// braces.
    pub framed: bool,
    /// How many times over the file holds this node. See `KindWalk`.
    pub scale: u64,
}

pub(super) trait Watch<S: Source> {
    /// The node at `path` is about to be counted, as a leaf or as a frame to
    /// walk into. May read and may be interrupted; asked again next go if so.
    fn ready(&mut self, _ev: &mut Evaluator, _doc: &Document<S>, _path: &[usize], _r: &Resolved) -> R<()> {
        Ok(())
    }

    /// Child `idx` of the frame at `parent` would not read, so the walk is
    /// about to end that frame's run there. `at` is where the child would have
    /// started. May be interrupted, before the frame gives up on the child.
    fn failed(&mut self, _ev: &mut Evaluator, _doc: &Document<S>, _parent: &[usize], _idx: u64, _at: u64, _why: &str) -> R<()> {
        Ok(())
    }

    /// The node at `path` is a second reading of bytes counted somewhere
    /// else, and the walk is about to pass it over. May be interrupted.
    fn aside(&mut self, _ev: &mut Evaluator, _doc: &Document<S>, _path: &[usize], _r: &Resolved) -> R<()> {
        Ok(())
    }

    /// The frame is about to close. May be interrupted.
    fn closing(&mut self, _ev: &mut Evaluator, _doc: &Document<S>, _f: &Closing) -> R<()> {
        Ok(())
    }

    /// A node with nothing under it covers `bits` bits, counted `scale` times.
    /// `bits` is already multiplied.
    fn leaf(&mut self, _ev: &Evaluator, _path: &[usize], _r: &Resolved, _bits: u64, _scale: u64) {}

    /// The walk has gone into the node at `path`, whose children stand for
    /// `scale` of theirs each.
    fn open(&mut self, _ev: &Evaluator, _path: &[usize], _r: &Resolved, _scale: u64) {}

    /// Bits from `from` to `to` in the frame at `parent` that no child covers,
    /// between two children laid out in order, counted `scale` times.
    fn gap(&mut self, _parent: &[usize], _from: u64, _to: u64, _scale: u64) {}

    /// The frame closed.
    fn close(&mut self, _ev: &Evaluator, _f: &Closing) {}
}

/// Nobody following: the kind totals on their own.
impl<S: Source> Watch<S> for () {}
