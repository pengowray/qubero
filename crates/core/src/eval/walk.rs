//! Placing element `n` of a list whose elements are not all the same size.
//!
//! Such a list can only be measured by walking it: element `n` starts where
//! element `n - 1` ends. Remembering every element as it goes is what makes a
//! short list cheap to browse, and what makes a long one impossible: a GGUF
//! whose tokenizer holds two million strings leaves six million nodes behind,
//! which is more memory than the file itself.
//!
//! So a long list is walked differently. Every thousandth element's offset is
//! kept, the last few elements are kept because the one being placed may ask
//! the one before it for a value, and everything else is dropped as the walk
//! passes it. Reaching an element later starts from the nearest kept offset
//! rather than from the beginning: bounded memory, and a bounded walk.

use super::*;

/// A list longer than this is walked with its middle dropped. Below it, a list
/// is small enough that remembering all of it is the cheaper answer.
pub(super) const GUARD_ABOVE: usize = 4096;

/// Keep an offset every this many elements. The re-walk to reach an element is
/// at most this long.
pub(super) const CHECKPOINT: usize = 1024;

/// How many elements behind the walk are kept whole. `Prev` asks the element
/// before this one for a field, so dropping it would turn a walk into a walk
/// per element.
pub(super) const KEEP: usize = 64;

impl Evaluator {
    /// Where child `idx` of a list of variable-size elements starts.
    pub(super) fn walk_to<S: Source>(&mut self, doc: &Document<S>, parent: &[usize], idx: usize) -> R<u64> {
        let pr = self.memo[parent].clone();
        // A short list is remembered whole: every element stays in the memo,
        // and placing the next one is a lookup.
        if !self.guarded(doc, parent, &pr)? {
            self.resolve_upto(doc, parent, idx)?;
            let mut prev = parent.to_vec();
            prev.push(idx - 1);
            return Ok(self.memo[&prev].offset + self.size_of(doc, &prev)?);
        }
        // Start from the nearest kept offset at or before the target and walk
        // forward, keeping only the last few elements and every thousandth
        // offset. `j` is the element that `at` is the start of.
        let (mut j, mut at) = self.nearest_checkpoint(parent, idx);
        let mut p = parent.to_vec();
        self.guard_depth += 1;
        let walked = self.walk_from(doc, &pr, &mut p, &mut j, &mut at, idx);
        self.guard_depth -= 1;
        if self.guard_depth == 0 {
            self.journal.clear();
        }
        walked
    }

    /// The walk itself, from element `j` at offset `at` up to element `idx`.
    fn walk_from<S: Source>(
        &mut self,
        doc: &Document<S>,
        pr: &Resolved,
        p: &mut Vec<usize>,
        j: &mut usize,
        at: &mut u64,
        idx: usize,
    ) -> R<u64> {
        let parent = p.clone();
        while *j < idx {
            let mark = self.journal.len();
            p.push(*j);
            // The element may already be here, from the walk that sized the
            // list or from the caller looking at it.
            let known = self.memo.get(p.as_slice()).and_then(|r| r.size.map(|s| r.offset + s));
            let end = match known {
                Some(end) => end,
                None => {
                    self.place(doc, p, pr, *j, *at)?;
                    let size = self.size_of(doc, p)?;
                    self.memo[p.as_slice()].offset + size
                }
            };
            p.pop();
            *j += 1;
            *at = end;
            self.list_mut(&parent).walk_at = Some((*j, *at));
            if *j % CHECKPOINT == 0 {
                self.checkpoint(&parent, *j, *at);
            }
            // Drop what this step left behind, once it is far enough back that
            // nothing will ask for it again.
            if idx - *j > KEEP {
                self.forget(mark);
            } else {
                self.journal.truncate(mark);
            }
        }
        Ok(*at)
    }

    /// Resolve child `idx` of a list, knowing where it starts. The ordinary
    /// path asks the element before it, which is the walk this is inside of.
    fn place<S: Source>(&mut self, doc: &Document<S>, path: &[usize], pr: &Resolved, idx: usize, offset: u64) -> R<()> {
        let ty = match &pr.ty {
            Ty::Array { elem, .. } | Ty::Repeat { elem, .. } => (**elem).clone(),
            _ => return fail("not a list"),
        };
        if offset > pr.limit {
            return fail("runs past the end of its container");
        }
        let r = self.effective(doc, path, format!("[{idx}]"), ty, offset, pr.limit)?;
        self.remember(path, r);
        Ok(())
    }

    /// Whether this list is long enough to be walked with its middle dropped.
    /// A `Repeat` does not know how many elements it has without walking it,
    /// and so is guarded from the start rather than found to be long too late.
    fn guarded<S: Source>(&mut self, doc: &Document<S>, path: &[usize], r: &Resolved) -> R<bool> {
        Ok(match &r.ty {
            Ty::Repeat { .. } => true,
            Ty::Array { count, .. } => {
                let count = count.clone();
                self.eval_expr(doc, path, &count)? > GUARD_ABOVE as i128
            }
            _ => false,
        })
    }

    /// The nearest known offset at or before element `idx`, and which element
    /// it is the start of: where the last walk stopped, if that was before
    /// `idx`, else the nearest kept offset, else the list's own start.
    /// Reading a list in order starts each step where the last one ended.
    fn nearest_checkpoint(&self, parent: &[usize], idx: usize) -> (usize, u64) {
        let state = self.lists.get(parent);
        let mut best = (0, self.memo[parent].offset);
        if let Some(&(j, at)) = state.and_then(|l| l.checkpoints.iter().rev().find(|(j, _)| *j <= idx)) {
            best = (j, at);
        }
        match state.and_then(|l| l.walk_at) {
            Some((j, at)) if j <= idx && j >= best.0 => (j, at),
            _ => best,
        }
    }

    fn checkpoint(&mut self, parent: &[usize], j: usize, at: u64) {
        let l = self.list_mut(parent);
        if l.checkpoints.last().is_none_or(|(last, _)| *last < j) {
            l.checkpoints.push((j, at));
        }
    }

    /// Record a node, and note it as droppable while a guarded walk is running.
    pub(super) fn remember(&mut self, path: &[usize], r: Resolved) {
        if self.guard_depth > 0 {
            self.journal.push(path.to_vec());
        }
        self.memo.insert(path.to_vec(), r);
    }

    /// Drop everything recorded since `mark`.
    fn forget(&mut self, mark: usize) {
        let dropped: Vec<Vec<usize>> = self.journal.drain(mark..).collect();
        for path in dropped {
            self.memo.remove(&path);
            self.lists.remove(&path);
        }
    }
}
