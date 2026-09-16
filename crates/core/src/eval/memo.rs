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
//! everything that ended before it standing, with the nodes it sits in, and
//! drops the rest, and what counts as "the rest" is the same question for all
//! three: see `forget_after`, which is the reason this is one type and not
//! three fields.

use std::sync::Arc;

use rustc_hash::{FxHashMap, FxHashSet};

use super::{ListState, Resolved};
use crate::formats::pickle::familiar::Match;
use crate::json;
use crate::template::{Deduced, Ty, Until};

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
    /// What the Familiar Pickle Form recogniser made of the file, for the
    /// pickle field it was run over. Kept beside the nodes for the reason the
    /// parsed JSON is: the values are placed from it rather than read.
    pickle: FxHashMap<Vec<usize>, Arc<Match>>,
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
            self.pickle.remove(&p);
        }
        // A stitched stream's node is a field of the file and stays, but the
        // parts its walk found were read from bytes that may be what changed,
        // and the space they made is going. So the walk starts again.
        for l in self.lists.values_mut() {
            l.stitch = None;
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

    /// Forget one node and everything inside it.
    ///
    /// For an element that was placed and then could not be read, which ends
    /// the run it is in: the fields read before it failed would otherwise
    /// stay with nothing above them. Every node held is looked at, so this is
    /// not for the walks, which forget an element at a time; an element that
    /// ends a run happens a handful of times across the whole sample
    /// collection, with fewer than a hundred nodes held each time.
    pub(super) fn forget_under(&mut self, path: &[usize]) {
        self.forget_node(path);
        let inside = |p: &Vec<usize>| p.len() > path.len() && p.starts_with(path);
        self.nodes.retain(|p, _| !inside(p));
        self.lists.retain(|p, _| !inside(p));
        self.json.retain(|p, _| !inside(p));
        self.pickle.retain(|p, _| !inside(p));
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
            chain_reach: Vec::new(),
            chain_end_reach: 0,
            gather: None,
            stitch: None,
            stretched: Vec::new(),
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

    /// What a Familiar Pickle Form made of the field at `path`, if it has been
    /// run over it.
    pub(super) fn pickle(&self, path: &[usize]) -> Option<&Arc<Match>> {
        self.pickle.get(path)
    }

    pub(super) fn remember_pickle(&mut self, path: Vec<usize>, found: Arc<Match>) {
        self.pickle.insert(path, found);
    }

    /// What running the file said about it, if it has been run.
    pub(super) fn deduced(&self) -> Option<&Arc<dyn Deduced>> {
        self.deduced.as_ref()
    }

    pub(super) fn remember_deduced(&mut self, r: Arc<dyn Deduced>) {
        self.deduced = Some(r);
    }

    /// How far into the file what placed the node at `path` was read from.
    ///
    /// The judgement `forget_after` makes of the nodes above one it keeps,
    /// made when a walk reads a field somewhere else and places an element
    /// with it, and written down as a number: an overwrite at or past it
    /// leaves the element where it was. Each node on the line was placed from
    /// what came before it, so where it starts is as far as that reaches;
    /// JSON is the exception, whose values are placed by a parse that runs to
    /// its end. An element of a chain or a gather reaches as far as the walk
    /// that found it read. A node that is not held, or one in a space of its
    /// own, whose offsets are not bits of the file, says nothing, and that is
    /// `u64::MAX`.
    pub(super) fn placed_from(&self, path: &[usize]) -> u64 {
        let mut reach = 0;
        for k in 0..=path.len() {
            let Some(r) = self.nodes.get(&path[..k]) else { return u64::MAX };
            if r.space != 0 {
                return u64::MAX;
            }
            let here = match r.ty {
                Ty::Json(..) | Ty::Pickle(..) => r.size.map_or(u64::MAX, |size| r.offset.saturating_add(size)),
                _ => r.offset,
            };
            reach = reach.max(here);
            if let Some((&idx, list)) = path[..k].split_last() {
                reach = reach.max(self.element_reach(list, idx));
            }
        }
        reach
    }

    /// How far what placed element `idx` of the list at `list` was read from,
    /// beyond what placed the list. Nothing for a list whose elements follow
    /// one another or sit where a table before the list says. For a chain or
    /// a gather, what its walk wrote down when it found the element, and
    /// `u64::MAX` when it has nothing written down, since then nothing says.
    fn element_reach(&self, list: &[usize], idx: usize) -> u64 {
        let noted = match self.nodes.get(list).map(|r| &r.ty) {
            Some(Ty::Chain { .. }) => self.lists.get(list).and_then(|l| l.chain_reach.get(idx)),
            Some(Ty::Gather { .. }) => self.lists.get(list).and_then(|l| l.gather.as_deref()).and_then(|g| g.reaches.get(idx)),
            _ => return 0,
        };
        noted.copied().unwrap_or(u64::MAX)
    }

    /// How far what says how many elements the list at `path` has was read
    /// from, beyond what placed the list.
    ///
    /// An array's count and a pointer list's table are read before the list,
    /// and a run divided by its stride is as long as its room, which was
    /// settled before it too. A run walked to its end reaches that end, and a
    /// bit further when the element after it would not read, since an edit
    /// there may make it read. A chain or a gather has as many elements as
    /// its walk found, and reaches as far as that walk read.
    pub(super) fn count_reach(&self, path: &[usize]) -> u64 {
        let Some(r) = self.nodes.get(path) else { return u64::MAX };
        let l = self.lists.get(path);
        match &r.ty {
            Ty::Array { .. } | Ty::PointerList { .. } => 0,
            // A `While` run asks its question at the start of the element it
            // decides not to read, and the question may look at bytes there:
            // an ImHex list carries on while the four bytes ahead of it are
            // not zero. Those bytes are past where the run ends, so an edit to
            // them changes how many elements there are, and saying the run
            // reaches only its own end would leave the old count on screen.
            // Every edit re-counts such a run until something narrower than
            // "somewhere ahead" tracks how far its question looked.
            Ty::Repeat { until: Until::While(_), .. } => u64::MAX,
            Ty::Repeat { until, .. } => match l {
                Some(l) if l.repeat_done => l.repeat_end.map_or(r.offset, |end| end + u64::from(l.repeat_trouble.is_some())),
                _ if matches!(until, Until::End) => 0,
                _ => u64::MAX,
            },
            Ty::Chain { .. } => l.filter(|l| l.chain_done).map_or(u64::MAX, |l| l.chain_end_reach),
            Ty::Gather { .. } => l.and_then(|l| l.gather.as_deref()).filter(|g| g.done).map_or(u64::MAX, |g| g.walk.reach),
            _ => r.size.map_or(u64::MAX, |size| r.offset.saturating_add(size)),
        }
    }

    /// Forget everything. For a change to the document that moves bytes about,
    /// or a change of template, after which none of this stands.
    pub(super) fn forget(&mut self) {
        self.nodes.clear();
        self.lists.clear();
        self.json.clear();
        self.pickle.clear();
        self.tags.clear();
        self.tag_entries = 0;
        self.deduced = None;
    }

    /// Forget what an overwrite at `bit` could have changed, and keep the
    /// rest. What makes that safe is in `Evaluator::invalidate_from`, which is
    /// the only caller; what it comes to for each of the three is here.
    pub(super) fn forget_after(&mut self, bit: u64) {
        // A node with no size worked out yet has not ended before the edit:
        // nothing says where it ends, so nothing says it ended before it.
        let ended = |r: &Resolved| r.size.is_some_and(|size| r.offset + size <= bit);
        // A node that ended before the edit is kept with every node above it,
        // since a name is looked up through those and a node that is held is
        // not placed again. One kept for that alone is as good as one that
        // ended, bar its size: where it starts, the room it has and what type
        // it is were worked out from what came before it. Two kinds of node
        // are not, and what is under them goes instead. One that starts after
        // the edit: only a pointer puts a node before the one it sits in, and
        // a pointer read after the edit may say somewhere else now. And JSON:
        // where a value in it ends is where the parse found the next one, and
        // the parse covers the edit.
        let holds = |r: &Resolved| ended(r) || (r.offset <= bit && !matches!(r.ty, Ty::Json(..) | Ty::Pickle(..)));
        // An element of a chain or a gather is placed from a field that need
        // not be on its line at all: the link in the element before, or the
        // record the walk reached. Where that was read is written down by
        // the walk, and an element placed from after the edit goes whatever
        // its line says, with what is under it.
        //
        // Most files hold no such list, and the ones that do hold a few, so a
        // node is only looked up as one when a list of that kind sits at the
        // depth its parent does. Asked of every node on every line otherwise,
        // this is a lookup per node more than the rest of the pass makes.
        let (mut depths, mut deep) = (0u128, false);
        for (path, r) in &self.nodes {
            if matches!(r.ty, Ty::Chain { .. } | Ty::Gather { .. }) {
                match 1u128.checked_shl(path.len() as u32) {
                    Some(at) => depths |= at,
                    None => deep = true,
                }
            }
        }
        let placed = |p: &[usize]| {
            p.split_last().is_none_or(|(&idx, list)| {
                let maybe = deep || 1u128.checked_shl(list.len() as u32).is_some_and(|at| depths & at != 0);
                !maybe || self.element_reach(list, idx) <= bit
            })
        };
        let (nodes, lists) = (&self.nodes, &self.lists);
        let mut judged = FxHashMap::default();
        let mut above = FxHashSet::default();
        let mut cut_off = FxHashSet::default();
        for (path, r) in nodes {
            if !ended(r) {
                continue;
            }
            if !line_holds(path, &mut judged, |p| nodes.get(p).is_some_and(holds) && placed(p)) {
                cut_off.insert(path.clone());
                continue;
            }
            // Up to the first one already counted, whose line was counted
            // with it.
            for k in (0..path.len()).rev() {
                if !above.insert(&path[..k]) {
                    break;
                }
            }
        }
        let structure: FxHashSet<Vec<usize>> = above.into_iter().filter(|p| !ended(&nodes[*p])).map(<[usize]>::to_vec).collect();
        // What a list learned goes with anything on its line that was placed
        // from after the edit, since the list may be somewhere else now. A
        // node that is not held is passed over rather than counted against
        // it: what a list learned outlives the elements a walk let go of.
        let mut judged = FxHashMap::default();
        let misplaced: FxHashSet<Vec<usize>> = lists
            .keys()
            .filter(|path| !line_holds(path, &mut judged, |p| nodes.get(p).is_none_or(holds) && placed(p)))
            .cloned()
            .collect();
        self.nodes.retain(|path, r| if ended(r) { !cut_off.contains(path) } else { structure.contains(path) });
        for path in &structure {
            let Some(r) = self.nodes.get_mut(path) else { continue };
            r.size = None;
            // A run stretched to take in an element that overran its room
            // has the room back from before any element that reaches past the
            // edit, which may not overrun it now.
            if let Some(l) = self.lists.get_mut(path) {
                while r.limit > bit {
                    let Some((limit, declared)) = l.stretched.pop() else { break };
                    (r.limit, r.declared_size) = (limit, declared);
                }
            }
        }
        // The parsed text of a JSON field stands only if the field ended
        // before the edit.
        self.json.retain(|path, _| self.nodes.get(path).is_some_and(ended));
        // A recognition covers the whole file, so nothing that was edited
        // ended before the edit and the file is recognised again.
        self.pickle.retain(|path, _| self.nodes.get(path).is_some_and(ended));
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
            if misplaced.contains(path) {
                return false;
            }
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
            // A chain keeps the elements it found from links that ended
            // before the edit, which are the ones before the first that did
            // not: each is found from the one before, so what was read to
            // find it only grows along the chain. The walk carries on from
            // there, and is over only if what ended it ended before the edit.
            let kept = l.chain_reach.partition_point(|&reach| reach <= bit);
            if kept < l.chain_starts.len() || l.chain_end_reach > bit {
                l.chain_done = false;
                l.chain_end_reach = 0;
            }
            l.chain_starts.truncate(kept);
            l.chain_reach.truncate(kept);
            // A gather's walk stands, with everything it found, when it is
            // over and nothing it read reaches the edit: every record, and
            // every step that said there was nothing more, is where it was
            // and says what it said. Otherwise it starts again. The walk is a
            // stack of steps, which cannot be cut back to a child, and the
            // children before the first placed from after the edit are still
            // held and are found again where they are. A walk with a record
            // it wrote no reach for starts again too: a schema's build keeps
            // the records it reached here while it is under way, and says how
            // far they reach only once it is built.
            if l.gather.as_deref().is_some_and(|g| !g.done || g.walk.reach > bit || g.reaches.len() != g.records.len()) {
                l.gather = None;
            }
            // The walk to a stitched stream's parts starts again whatever it
            // read: the space it opened is gone already.
            l.stitch = None;
            // A run that is gone is placed again in the room the template
            // gives it, and stretched again if it needs to be.
            let node = nodes.get(path);
            if node.is_none() {
                l.stretched.clear();
            }
            let empty = l.checkpoints.is_empty()
                && l.walk_at.is_none()
                && l.repeat_len == 0
                && !l.repeat_done
                && l.stretched.is_empty();
            // What a chain or a gather found is kept like the checkpoints,
            // whether or not the list is held: the rules above already cut it
            // back to what was read before the edit, and what was read to
            // place the list itself is part of that.
            let walked = l.chain_done || !l.chain_starts.is_empty() || l.gather.is_some();
            // A list kept only for what is under it has not ended, so it
            // keeps what the rules above leave and no more.
            !empty || walked || node.is_some_and(ended)
        });
    }
}

/// Whether `holds` is true of every path from the root down to `path`. What
/// was found for each is kept in `judged`, since paths share their lines: the
/// elements of a list would otherwise each ask again about everything above.
fn line_holds<'a>(path: &'a [usize], judged: &mut FxHashMap<&'a [usize], bool>, holds: impl Fn(&[usize]) -> bool) -> bool {
    let (mut from, mut ok) = (0, true);
    for k in (0..=path.len()).rev() {
        if let Some(&known) = judged.get(&path[..k]) {
            (from, ok) = (k + 1, known);
            break;
        }
    }
    for k in from..=path.len() {
        ok = ok && holds(&path[..k]);
        judged.insert(&path[..k], ok);
    }
    ok
}

#[cfg(test)]
mod tests;

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
