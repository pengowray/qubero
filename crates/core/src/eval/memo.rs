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
use crate::template::Deduced;

/// A label a tagged search looks for, in the form a map can be keyed by.
///
/// The three ways a format labels a record, as [`crate::template::Tag`] has
/// them once a computed label has been worked out. Text is held trimmed at the
/// end, because that is how `tag_matches` compares it: a key stored with the
/// padding of a fixed-width field on it would never be found again.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum TagKey {
    Int(i128),
    Text(String),
    Bytes(Vec<u8>),
}

/// Which list a tagged search walked, and what it found there.
///
/// One of these per stretch of bytes read as a list, rather than one per path,
/// which is the whole point: an HDF5 global heap collection is reached through
/// the address every variable-length element carries, so a column of two
/// thousand strings is two thousand paths to one collection. Walking it once
/// per path is quadratic; walking it once and answering the rest from here is
/// not.
pub(super) struct TagIndex {
    /// The list the labels were read from, which is whichever path asked
    /// first. Everything else that reaches these bytes is answered from this
    /// walk rather than from its own.
    pub(super) list: Vec<usize>,
    /// How many of its elements have been read. Everything below this is in
    /// `found` unless its label could not be read at all, so a label missing
    /// from `found` is a label the walk has not reached yet.
    pub(super) scanned: usize,
    /// Which element carried each label, the first one where a label is
    /// written twice: a search answers with the first match, and so does this.
    pub(super) found: FxHashMap<TagKey, usize>,
    /// True once this stopped growing because the index was full, after which
    /// a search that misses walks its own list the old way.
    pub(super) full: bool,
}

/// How many labels one list may have indexed. Far past any header: the largest
/// FITS header anyone writes is a few hundred cards, and a global heap
/// collection holds a few thousand objects. A list longer than this is one
/// where the walk is the cost whatever is remembered about it, and remembering
/// a million labels to save a comparison each is the memory this was meant to
/// save.
const TAG_INDEX_CAP: usize = 100_000;

#[derive(Default)]
pub(super) struct Memo {
    /// Where each node starts, how long it is, and what type it turned out to
    /// be. The answer every other one is keyed against: a list that is not
    /// here has not been reached, whatever a stale note beside it says.
    nodes: FxHashMap<Vec<usize>, Resolved>,
    lists: FxHashMap<Vec<usize>, ListState>,
    json: FxHashMap<Vec<usize>, Arc<json::Val>>,
    /// What a tagged search over a named list has learned, by the stretch of
    /// bytes the list covers and the field of an element the label is read
    /// from: `(space, offset, limit, key)`. Not by path, so that every
    /// referrer to one collection shares one walk. See [`TagIndex`].
    tags: FxHashMap<(u32, u64, u64, Arc<[String]>), TagIndex>,
    /// How many labels are held across every list, so that a file full of
    /// them cannot grow this without bound.
    tag_entries: usize,
    /// What running the whole file said about it. One per document rather
    /// than one per path: a format that has to be run is run whole, and the
    /// run is over the file rather than over any node. What the reading is
    /// belongs to the format that produced it; here it is a thing kept and
    /// handed back.
    deduced: Option<Arc<dyn Deduced>>,
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

    /// Every node held without the node it sits in. A name is looked up
    /// through the nodes above the one asking, and a node that is held is not
    /// placed again, so one of these reads as a field with nothing around it.
    #[cfg(test)]
    pub(super) fn without_parent(&self) -> Vec<Vec<usize>> {
        self.nodes.keys().filter(|p| p.split_last().is_some_and(|(_, parent)| !self.nodes.contains_key(parent))).cloned().collect()
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
        // What a search learned about a list inside a stream goes with the
        // stream: the offsets it is keyed by count in that space, and the
        // space is about to be opened again.
        self.tags.retain(|(space, ..), _| *space == 0);
        self.tag_entries = self.tags.values().map(|t| t.found.len()).sum();
    }

    /// How many nodes are held. What a walk over a long list costs in memory
    /// is measured here rather than guessed at.
    pub(super) fn len(&self) -> usize {
        self.nodes.len()
    }

    /// What has been learned about the labels of the list covering these
    /// bytes, read from this field of each element. Nothing until a search has
    /// walked one.
    pub(super) fn tag_index(&self, slot: &(u32, u64, u64, Arc<[String]>)) -> Option<&TagIndex> {
        self.tags.get(slot)
    }

    /// The same, made if it is not there yet, with `list` as the one path every
    /// referrer will be answered from.
    pub(super) fn tag_index_mut(
        &mut self,
        slot: (u32, u64, u64, Arc<[String]>),
        list: &[usize],
    ) -> &mut TagIndex {
        self.tags.entry(slot).or_insert_with(|| TagIndex {
            list: list.to_vec(),
            scanned: 0,
            found: FxHashMap::default(),
            full: false,
        })
    }

    /// Write down which element of a list carried a label, unless the index is
    /// full or that label is already written down. The first element wins,
    /// because that is the one a search answers with.
    pub(super) fn remember_tag(&mut self, slot: &(u32, u64, u64, Arc<[String]>), key: TagKey, idx: usize) {
        let full = self.tag_entries >= TAG_INDEX_CAP;
        let Some(entry) = self.tags.get_mut(slot) else { return };
        if full {
            entry.full = true;
            return;
        }
        // Written once and never changed. A label can appear twice in a list,
        // and a search answers with the first element carrying it; a walk that
        // resumes and meets the second would otherwise replace the answer with
        // it. FITS headers do this: `COMMENT` and `HISTORY` are written as
        // often as anyone likes.
        if let std::collections::hash_map::Entry::Vacant(slot) = entry.found.entry(key) {
            slot.insert(idx);
            self.tag_entries += 1;
        }
    }

    /// How far the walk of this list has got, once it has gone further.
    pub(super) fn tag_scanned(&mut self, slot: &(u32, u64, u64, Arc<[String]>), upto: usize) {
        if let Some(entry) = self.tags.get_mut(slot) {
            entry.scanned = entry.scanned.max(upto);
        }
    }

    /// Forget what was learned about one list, for a hit that turned out to
    /// name an element that is no longer labelled that way.
    pub(super) fn forget_tags(&mut self, slot: &(u32, u64, u64, Arc<[String]>)) {
        if let Some(entry) = self.tags.remove(slot) {
            self.tag_entries = self.tag_entries.saturating_sub(entry.found.len());
        }
    }

    /// Forget one node, and what any list at that path had learned. The two go
    /// together: this is a node the walk has moved past, and a note about a
    /// list nothing has reached is a note about nothing.
    pub(super) fn forget_node(&mut self, path: &[usize]) {
        self.nodes.remove(path);
        self.lists.remove(path);
        // How far a sequential walk of the parent had got is a note about
        // nodes that are in here, and this one has just left. `resolve_upto`
        // starts from that mark and would step straight over the hole,
        // leaving the element after it to be placed from a sibling nothing
        // has read. So the mark comes back to where the hole begins.
        if let Some((&last, parent)) = path.split_last() {
            if let Some(l) = self.lists.get_mut(parent) {
                l.seq_end = l.seq_end.min(last);
            }
        }
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
            repeat_trouble: None,
            walk_at: None,
            expected_count: None,
            checkpoints: Vec::new(),
            pointer_starts: None,
            chain_starts: Vec::new(),
            chain_done: false,
            gather: None,
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

    /// What running the file said about it, if it has been run.
    pub(super) fn deduced(&self) -> Option<&Arc<dyn Deduced>> {
        self.deduced.as_ref()
    }

    pub(super) fn remember_deduced(&mut self, r: Arc<dyn Deduced>) {
        self.deduced = Some(r);
    }

    /// Forget everything. For a change to the document that moves bytes about,
    /// or a change of template, after which none of this stands.
    pub(super) fn forget(&mut self) {
        self.nodes.clear();
        self.lists.clear();
        self.json.clear();
        self.tags.clear();
        self.tag_entries = 0;
        self.deduced = None;
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
        // A run over the whole file says nothing about which half of it an
        // edit touched, so an edit anywhere means running it again.
        self.deduced = None;
        // What a search learned about a list is a claim about the bytes the
        // list covers, so it stands exactly when those bytes ended before the
        // edit. The index is keyed by the stretch, so the range is right
        // there and nothing has to be looked up to decide.
        self.tags.retain(|(_, _, limit, _), _| *limit <= bit);
        self.tag_entries = self.tags.values().map(|t| t.found.len()).sum();
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
                l.repeat_trouble = None;
            }
            // Where a pointer list's children start was read from a field that
            // may be anywhere, and where a sequential walk had got to counts
            // children some of which have just gone. Both are cheap to redo.
            l.pointer_starts = None;
            l.seq_end = 0;
            // What a gather found was read from records that may sit after
            // the edit even when the children do not, and a Parquet footer
            // sits after every page it places. So the walk starts again.
            l.gather = None;
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
