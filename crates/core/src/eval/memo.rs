//! What the reading has worked out about a file, kept by path.
//!
//! Three answers per node, kept in three maps rather than one record, because
//! they are asked for at different rates. Where a node starts and how long it
//! is, is asked of every node there is. What a list has learned about itself
//! is asked only of lists, and holds a checkpoint per thousand elements, which
//! is why it is not part of the first: resolving a child copies its parent's
//! record, and a million-element list would copy a thousand checkpoints a
//! child. The text of a JSON field, parsed, is asked only of the few fields
//! that hold JSON.
//!
//! They go together because they go stale together. An edit at a byte leaves
//! everything that ended before it standing and drops the rest, and what
//! counts as "the rest" is the same question for all three: see `forget_after`,
//! which is the reason this is one type and not three fields.

use std::sync::Arc;

use rustc_hash::FxHashMap;

use super::{ListState, Resolved};
use crate::json;

#[derive(Default)]
pub(super) struct Memo {
    /// Where each node starts, how long it is, and what type it turned out to
    /// be. The answer every other one is keyed against: a list that is not
    /// here has not been reached, whatever a stale note beside it says.
    nodes: FxHashMap<Vec<usize>, Resolved>,
    lists: FxHashMap<Vec<usize>, ListState>,
    json: FxHashMap<Vec<usize>, Arc<json::Val>>,
}

impl Memo {
    pub(super) fn contains_key(&self, path: &[usize]) -> bool {
        self.nodes.contains_key(path)
    }

    pub(super) fn get(&self, path: &[usize]) -> Option<&Resolved> {
        self.nodes.get(path)
    }

    pub(super) fn get_mut(&mut self, path: &[usize]) -> Option<&mut Resolved> {
        self.nodes.get_mut(path)
    }

    pub(super) fn insert(&mut self, path: Vec<usize>, r: Resolved) {
        self.nodes.insert(path, r);
    }

    /// Drop every node read inside a decoded stream.
    ///
    /// `forget_after` keeps what ended before the edit, and a decoded field is
    /// at offset 0 of its own space, so every one of them would look like it
    /// ended before any edit anywhere. They are worked out from the stream's
    /// bytes and the stream may be what was edited, so they all go. Cheap: the
    /// nodes are few, and opening the stream again is one inflate.
    pub(super) fn forget_decoded(&mut self) {
        // Everything a stream produced, which is in a space of its own, and
        // everything the decoder read to produce it, which is not: a node laid
        // out from a trace is bits of the file and looks like any other node,
        // and the trace it was laid out from is about to go.
        let streams: Vec<Vec<usize>> = self
            .nodes
            .iter()
            .filter(|(_, r)| matches!(r.ty, crate::template::Ty::Decoded { .. }))
            .map(|(p, _)| p.clone())
            .collect();
        let under = |p: &[usize]| streams.iter().any(|s| p.len() > s.len() && p.starts_with(s));
        let gone: Vec<Vec<usize>> = self
            .nodes
            .iter()
            .filter(|(p, r)| r.space != 0 || under(p))
            .map(|(p, _)| p.clone())
            .collect();
        for p in gone {
            self.nodes.remove(&p);
            self.lists.remove(&p);
            self.json.remove(&p);
        }
    }

    /// How many nodes are held. What a walk over a long list costs in memory
    /// is measured here rather than guessed at.
    pub(super) fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Forget one node, and what any list at that path had learned. The two go
    /// together: this is a node the walk has moved past, and a note about a
    /// list nothing has reached is a note about nothing.
    pub(super) fn forget_node(&mut self, path: &[usize]) {
        self.nodes.remove(path);
        self.lists.remove(path);
    }

    /// What the list at `path` has learned about itself. A node that is not a
    /// list, or one nothing has been learned about yet, has learned nothing,
    /// which is what the default says.
    ///
    /// Lent rather than handed over. The walk asks this once an element, and a
    /// list it has walked a million elements into holds a thousand
    /// checkpoints: a copy an element is the crawl these were split apart to
    /// avoid.
    pub(super) fn list(&self, path: &[usize]) -> &ListState {
        static NOTHING: ListState = ListState {
            repeat_len: 0,
            repeat_end: None,
            repeat_done: false,
            walk_at: None,
            expected_count: None,
            checkpoints: Vec::new(),
            pointer_starts: None,
            chain_starts: Vec::new(),
            chain_done: false,
            seq_end: 0,
        };
        self.lists.get(path).unwrap_or(&NOTHING)
    }

    pub(super) fn list_mut(&mut self, path: &[usize]) -> &mut ListState {
        self.lists.entry(path.to_vec()).or_default()
    }

    /// Every list anything has been learned about, with its path.
    pub(super) fn lists(&self) -> impl Iterator<Item = (&Vec<usize>, &ListState)> {
        self.lists.iter()
    }

    /// The parsed text of the JSON field at `path`, if it has been read.
    pub(super) fn json(&self, path: &[usize]) -> Option<&Arc<json::Val>> {
        self.json.get(path)
    }

    pub(super) fn remember_json(&mut self, path: Vec<usize>, val: Arc<json::Val>) {
        self.json.insert(path, val);
    }

    /// Forget everything. For a change to the document that moves bytes about,
    /// or a change of template, after which none of this stands.
    pub(super) fn forget(&mut self) {
        self.nodes.clear();
        self.lists.clear();
        self.json.clear();
    }

    /// Forget what an overwrite at `bit` could have changed, and keep the
    /// rest. What makes that safe is in `Evaluator::invalidate_from`, which is
    /// the only caller; what it comes to for each of the three is here.
    pub(super) fn forget_after(&mut self, bit: u64) {
        // A node with no size worked out yet goes: nothing says where it ends,
        // so nothing says it ended before the edit.
        self.nodes.retain(|_, r| r.size.is_some_and(|size| r.offset + size <= bit));
        // The parsed text of a JSON field goes when the field itself does.
        self.json.retain(|path, _| self.nodes.contains_key(path));
        let nodes = &self.nodes;
        self.lists.retain(|path, l| {
            l.checkpoints.retain(|(_, at)| *at <= bit);
            if l.walk_at.is_some_and(|(_, at)| at > bit) {
                l.walk_at = None;
            }
            // A repeat's count is only as good as the walk that reached it.
            if l.repeat_end.is_none_or(|end| end > bit) {
                l.repeat_len = 0;
                l.repeat_end = None;
                l.repeat_done = false;
            }
            // Where a pointer list's children start was read from a field that
            // may be anywhere, and where a sequential walk had got to counts
            // children some of which have just gone. Both are cheap to redo.
            l.pointer_starts = None;
            l.seq_end = 0;
            let empty = l.checkpoints.is_empty()
                && l.walk_at.is_none()
                && l.repeat_len == 0
                && !l.repeat_done;
            !empty || nodes.contains_key(path)
        });
    }
}

/// Reading a node that is not there is a bug rather than a case: every caller
/// that indexes has resolved the node first, and says so by indexing.
impl<Q> std::ops::Index<&Q> for Memo
where
    Q: ?Sized + std::hash::Hash + Eq,
    Vec<usize>: std::borrow::Borrow<Q>,
{
    type Output = Resolved;

    fn index(&self, path: &Q) -> &Resolved {
        &self.nodes[path]
    }
}
