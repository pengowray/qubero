//! Placing element `n` of a list whose elements are not all the same size.
//!
//! Such a list can only be measured by walking it: element `n` starts where
//! element `n - 1` ends. Remembering every element as it goes is what makes a
//! short list cheap to browse, and what makes a long one impossible: a GGUF
//! whose phonemizer holds a million rules leaves six million nodes behind,
//! which is more memory than the file itself.
//!
//! So a long list is walked differently. Every thousandth element's offset is
//! kept, the last few elements are kept because the element being placed may
//! ask the one before it for a value, and everything else is dropped as the
//! walk passes it. Reaching an element later starts from the nearest kept
//! offset rather than from the beginning: bounded memory, and a bounded walk.

use std::collections::VecDeque;

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

/// What one walk has added to the memo, oldest first, and how much of it each
/// element accounted for. Dropping the element that has fallen out of the
/// window behind the walk is then a matter of taking that many off the front.
#[derive(Default)]
pub(super) struct WalkJournal {
    added: VecDeque<Vec<usize>>,
    per_element: VecDeque<usize>,
}

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
        // Start from the nearest known offset at or before the target and walk
        // forward. `j` is the element that `at` is the start of.
        let (mut j, mut at) = self.nearest_start(parent, idx);
        let mut p = parent.to_vec();
        self.journals.push(WalkJournal::default());
        let walked = self.walk_from(doc, &pr, &mut p, &mut j, &mut at, idx);
        // The walk is over, so the window it kept behind it is over too. The
        // element before the target stays, since the one about to be placed may
        // ask it for a value. Without this, reading a long list a screen at a
        // time would leave a window behind at every screen.
        let mut keep = parent.to_vec();
        keep.push(idx.saturating_sub(1));
        let journal = self.journals.pop().expect("pushed above");
        self.drop_nodes(journal.added, &keep);
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
            let before = self.journals.last().map_or(0, |w| w.added.len());
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
            self.close_step(before);
        }
        Ok(*at)
    }

    /// Note what this step added, and drop the step that has fallen out of the
    /// window behind the walk.
    fn close_step(&mut self, before: usize) {
        let dropped: Vec<Vec<usize>> = {
            let Some(w) = self.journals.last_mut() else { return };
            w.per_element.push_back(w.added.len().saturating_sub(before));
            if w.per_element.len() <= KEEP {
                return;
            }
            let oldest = w.per_element.pop_front().unwrap_or(0);
            w.added.drain(..oldest).collect()
        };
        for path in dropped {
            self.memo.remove(&path);
            self.lists.remove(&path);
        }
    }

    /// Drop these nodes, except `keep` and what is inside it.
    fn drop_nodes(&mut self, added: VecDeque<Vec<usize>>, keep: &[usize]) {
        for path in added {
            if path.starts_with(keep) {
                continue;
            }
            self.memo.remove(&path);
            self.lists.remove(&path);
        }
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

    /// Which element of a long list covers `bit`, found by walking from the
    /// nearest kept offset. Looking at every element from the start instead
    /// would put the whole list back in memory, which is what the walk was for:
    /// putting the cursor in the middle of a million rules must not undo it.
    pub(super) fn child_covering<S: Source>(
        &mut self,
        doc: &Document<S>,
        path: &[usize],
        n: u64,
        bit: u64,
    ) -> R<Option<usize>> {
        let pr = self.memo[path].clone();
        if bit < pr.offset {
            return Ok(None);
        }
        let (mut j, mut at) = self.nearest_start_before(path, bit);
        if j as u64 >= n {
            return Ok(None);
        }
        let mut p = path.to_vec();
        self.journals.push(WalkJournal::default());
        let found = self.scan_from(doc, &pr, &mut p, &mut j, &mut at, n, bit);
        let journal = self.journals.pop().expect("pushed above");
        // The element the bit is in stays: it is the one about to be read.
        let mut keep = path.to_vec();
        if let Ok(Some(i)) = found {
            keep.push(i);
        }
        self.drop_nodes(journal.added, &keep);
        found
    }

    /// Walk forward from element `j` until one covers `bit`.
    fn scan_from<S: Source>(
        &mut self,
        doc: &Document<S>,
        pr: &Resolved,
        p: &mut Vec<usize>,
        j: &mut usize,
        at: &mut u64,
        n: u64,
        bit: u64,
    ) -> R<Option<usize>> {
        let parent = p.clone();
        while (*j as u64) < n {
            let before = self.journals.last().map_or(0, |w| w.added.len());
            p.push(*j);
            let known = self.memo.get(p.as_slice()).and_then(|r| r.size.map(|s| (r.offset, s)));
            let (start, size) = match known {
                Some(v) => v,
                None => {
                    self.place(doc, p, pr, *j, *at)?;
                    let size = self.size_of(doc, p)?;
                    (self.memo[p.as_slice()].offset, size)
                }
            };
            p.pop();
            // Before the first element, or in the slack between two of them.
            if bit < start {
                self.close_step(before);
                return Ok(None);
            }
            if bit < start + size {
                return Ok(Some(*j));
            }
            *j += 1;
            *at = start + size;
            self.list_mut(&parent).walk_at = Some((*j, *at));
            if *j % CHECKPOINT == 0 {
                self.checkpoint(&parent, *j, *at);
            }
            self.close_step(before);
        }
        Ok(None)
    }

    /// The nearest known element start at or before the bit `bit`, as an
    /// element index and its offset. Checkpoints rise in both index and offset,
    /// so the one to start from can be found by halving rather than scanning.
    fn nearest_start_before(&self, parent: &[usize], bit: u64) -> (usize, u64) {
        let Some(state) = self.lists.get(parent) else { return (0, self.memo[parent].offset) };
        let mut best = (0, self.memo[parent].offset);
        let k = state.checkpoints.partition_point(|(_, at)| *at <= bit);
        if k > 0 {
            best = state.checkpoints[k - 1];
        }
        match state.walk_at {
            Some((j, at)) if at <= bit && j >= best.0 => (j, at),
            _ => best,
        }
    }

    /// Whether this list is long enough to be walked with its middle dropped.
    /// A `Repeat` does not know how many elements it has without walking it,
    /// and so is guarded from the start rather than found to be long too late.
    pub(super) fn guarded<S: Source>(&mut self, doc: &Document<S>, path: &[usize], r: &Resolved) -> R<bool> {
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
    /// `idx`, else the nearest kept offset, else the list's own start. Reading
    /// a list in order then starts each step where the last one ended.
    fn nearest_start(&self, parent: &[usize], idx: usize) -> (usize, u64) {
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
        if let Some(w) = self.journals.last_mut() {
            w.added.push_back(path.to_vec());
        }
        self.memo.insert(path.to_vec(), r);
    }
}
