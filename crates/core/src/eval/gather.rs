//! The walk behind a [`Ty::Gather`]: finding the records that place its
//! children, one at a time, and asking them again later.
//!
//! A pointer list knows how many children it has from the length of its table,
//! and a chain finds each child from the one before. A gather knows neither.
//! Its records are wherever the template put them, as many lists deep as the
//! format nests them, and the only way to learn how many there are is to walk
//! to every one. So the walk is kept, the way a chain's is: each record reached
//! adds a child or is passed over, and the walk stops where it is when a go of
//! reading runs out, to carry on from the same record when it is asked again.
//!
//! What it keeps for each child is where the child starts and the path of the
//! record that placed it. The first is what the cursor and the gaps need, and
//! the second is what [`Expr::Placer`] asks: a heap array's length is in the
//! descriptor that placed it, and the heap has no way back to that descriptor
//! but this.
//!
//! Only reaching a record is charged against a go, and a record is reached
//! once. That is the lesson of the list walks in `walk.rs`: `spans` asks again
//! from the top of its window every go, and a walk that charged for what the
//! last go already found would spend each new go going back over it.

use std::sync::Arc;

use super::*;
use crate::template::{Step, CHAIN_CAP};

/// What a walk down a run of steps may stop on, which is the one thing the two
/// walks that share this code disagree about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Landing {
    /// A record with fields to read an offset from, for a gather. A list of
    /// plain numbers holds none, and fanning out over one is passed over in
    /// one step rather than element by element.
    Records,
    /// Any node at all, for a stitched stream: the parts it walks to are runs
    /// of bytes, and a list of those is exactly what it wants. Through an `At`,
    /// the way a path always goes, since the field of no bits is not the run.
    Runs,
}

impl Evaluator {
    /// What the gather at `list` has found so far, to change. Made the first
    /// time anything asks.
    fn gather_mut(&mut self, list: &[usize]) -> &mut GatherState {
        self.list_mut(list).gather.get_or_insert_with(Default::default)
    }

    /// Walk the gather at `list` until child `want` has been found, or the
    /// walk has reached its end. See [`Ty::Gather`] for what a walk passes
    /// over.
    pub(super) fn extend_gather_to<S: Source>(&mut self, doc: &Document<S>, list: &[usize], want: usize) -> R<()> {
        let lr = self.memo[list].clone();
        let Ty::Gather { from, offset, anchor, adjust, skip_zero, .. } = &lr.ty else {
            return fail("not a gathered list");
        };
        let (from, offset, anchor, adjust, skip_zero) = (from.clone(), offset.clone(), *anchor, adjust.clone(), *skip_zero);
        if self.list(list).gather.as_deref().is_some_and(|g| g.done || g.starts.len() > want) {
            return Ok(());
        }
        // Worked out once per go rather than once per record: neither asks
        // anything of a record.
        let base = self.anchor_base(list, lr.offset, anchor);
        let adjust = self.eval_expr(doc, list, &adjust)?;
        let room = self.gather_room(doc, list, &lr, anchor);
        // A gather that is itself a region holds its children inside it. One
        // that covers nothing where it is declared holds them from its anchor
        // on, as far as the room it was given.
        let lowest = if lr.declared_size.is_some() { lr.offset } else { base };
        loop {
            let found = self.list(list).gather.as_deref().map_or(0, |g| g.starts.len());
            if found > want {
                return Ok(());
            }
            if found >= CHAIN_CAP {
                self.gather_mut(list).done = true;
                return Ok(());
            }
            let Some(record) = self.walk_next(doc, list, &from, Landing::Records)? else {
                self.gather_mut(list).done = true;
                return Ok(());
            };
            // Charged for reaching a record nothing has reached before, and
            // before anything is read from it, so a go that runs out here
            // leaves the walk standing on this record rather than past it.
            let reached = self.memo.get(&record).map_or(lr.offset, |r| r.offset);
            self.spend(reached)?;
            let (end, here) = self.record_frame(doc, &record)?;
            let at = match self.eval_expr_at(doc, &end, &offset, here) {
                Ok(at) => Some(at),
                Err(e) if e.interrupted() => return Err(e),
                // A record with no offset in it places nothing, and the
                // records after it are still worth reading.
                Err(_) => None,
            };
            let start = at
                .filter(|at| !(skip_zero && *at == 0))
                .map(|at| base as i128 + (at + adjust) * 8)
                .filter(|bits| *bits >= lowest as i128 && *bits <= room as i128);
            let g = self.gather_mut(list);
            if let Some(bits) = start {
                g.starts.push(bits as u64);
                g.records.push(record);
            }
            self.walk_past(list, Landing::Records);
        }
    }

    /// How far a gathered child may run: the region, when a `Sized` round the
    /// list gave it one, and otherwise whatever its anchor reaches, the way an
    /// `At` anchored the same way is bounded.
    pub(super) fn gather_room<S: Source>(&self, doc: &Document<S>, list: &[usize], lr: &Resolved, anchor: Anchor) -> u64 {
        if lr.declared_size.is_some() {
            return lr.limit;
        }
        match anchor {
            Anchor::File => doc.len_bits(),
            Anchor::Origin => self.origin_of(list).map_or(doc.len_bits(), |(_, limit)| limit),
            Anchor::Window | Anchor::SelfAligned(_) => lr.limit,
        }
    }

    /// The walk the node at `at` is taking, kept in whichever slot of its list
    /// state belongs to the kind of walk it is. Made the first time, standing
    /// where the node is declared: the first step names a field before it, the
    /// way the first name of a path does.
    fn walk_mut(&mut self, at: &[usize], landing: Landing) -> &mut Walk {
        let state = self.list_mut(at);
        let walk = match landing {
            Landing::Records => &mut state.gather.get_or_insert_with(Default::default).walk,
            Landing::Runs => &mut state.stitch.get_or_insert_with(Default::default).walk,
        };
        if !walk.started {
            walk.frames = vec![GatherFrame { node: at.to_vec(), next: 0, last: None }];
            walk.started = true;
        }
        walk
    }

    /// Step the walk off the landing it stands on, once the caller has read
    /// what it wanted from it.
    pub(super) fn walk_past(&mut self, at: &[usize], landing: Landing) {
        if let Some(top) = self.walk_mut(at, landing).frames.last_mut() {
            top.next += 1;
        }
    }

    /// The landing the walk from `at` stands on, or the next one after the
    /// frames have moved past everything that holds none. Nothing when the
    /// walk is over.
    ///
    /// Moving the frames on is not moving past a landing: the walk stands on
    /// one until the caller has read it and said so with [`Self::walk_past`],
    /// so a read that has to wait for bytes asks this again and is handed the
    /// same one.
    pub(super) fn walk_next<S: Source>(&mut self, doc: &Document<S>, at: &[usize], from: &[Step], landing: Landing) -> R<Option<Vec<usize>>> {
        loop {
            let (k, node, next, last) = {
                let w = self.walk_mut(at, landing);
                let Some(top) = w.frames.last() else { return Ok(None) };
                (w.frames.len() - 1, top.node.clone(), top.next, top.last.clone())
            };
            let Some(step) = from.get(k) else { return Ok(None) };
            let got = match step {
                Step::Deep(name) if k > 0 => self.deep_step(doc, name, &node, next, last),
                _ => self.walk_step(doc, at, step, k, &node, next, landing),
            };
            let got = match got {
                Err(e) if !e.interrupted() => None,
                other => other?,
            };
            let w = self.walk_mut(at, landing);
            // Reading a step can put nodes back and take nodes away, and a walk
            // whose own frames went with them has nothing left to stand on.
            if w.frames.len() != k + 1 {
                return fail("the gathered walk lost its place");
            }
            match got {
                Some((j, child)) => {
                    w.frames[k].next = j;
                    if matches!(step, Step::Deep(_)) {
                        w.frames[k].last = Some((j, child.clone()));
                    }
                    if k + 1 == from.len() {
                        return Ok(Some(child));
                    }
                    w.frames.push(GatherFrame { node: child, next: 0, last: None });
                }
                // Nothing more down this way, so the step above moves on.
                None => {
                    w.frames.pop();
                    if let Some(up) = w.frames.last_mut() {
                        up.next += 1;
                    }
                }
            }
        }
    }

    /// Where step `k` goes from `node`, trying its candidates from `from` on:
    /// the index of the child it takes, and the path it lands on. Nothing when
    /// it has no candidate there.
    #[allow(clippy::too_many_arguments)]
    fn walk_step<S: Source>(
        &mut self,
        doc: &Document<S>,
        list: &[usize],
        step: &Step,
        k: usize,
        node: &[usize],
        from: usize,
        landing: Landing,
    ) -> R<Option<(usize, Vec<usize>)>> {
        // The first step starts the walk, and only a field declared before the
        // gather, or the record that placed the element around it, can be
        // where it starts.
        if k == 0 {
            if from > 0 {
                return Ok(None);
            }
            if let Step::Placer = step {
                let Some((outer, idx)) = self.gathered_in(list) else { return Ok(None) };
                let record = self.gathered_record(doc, &outer, idx)?;
                // Opened again on the way down, since the outer walk may have
                // given some of them back since it stood there.
                for d in 0..=record.len() {
                    self.resolve(doc, &record[..d])?;
                }
                return Ok(Some((0, record)));
            }
            let Step::Field(name) = step else { return fail("a gather starts at a field declared before it") };
            let Some(mut p) = self.find_field(list, name) else { return Ok(None) };
            // `find_field` steps through an `At` the declaration shows. One a
            // switch or a `When` chose shows nothing until it is read, and an
            // HDF5 dataset keeps the copy of its datatype that way, placed
            // only for the classes whose elements ask it anything. The same
            // rule `within_path` applies to the first name of a path.
            self.through_at(doc, &mut p)?;
            return Ok(Some((0, p)));
        }
        // A step with one candidate stands on it until the walk has moved past
        // it, and the frame holds the candidate's own index rather than a
        // count: a field is child 10 of its structure, not the first of one.
        // So "moved past" is past that index. Asking whether anything has been
        // taken at all was the same answer while only the step above a record
        // was ever asked again, and the wrong one once a go could run out on
        // the record itself: the walk skipped it.
        match step {
            Step::Field(name) => {
                // A name taken from a stream is a field of what the stream
                // holds, as a name taken from a field that points elsewhere is
                // a field of what it points at.
                let mut node = node.to_vec();
                self.into_contents(doc, &mut node)?;
                let Some(j) = self.child_index(doc, &node, name)? else { return Ok(None) };
                if from > j {
                    return Ok(None);
                }
                let mut p = node;
                p.push(j);
                self.through_at(doc, &mut p)?;
                Ok(Some((j, p)))
            }
            // Taken by `deep_step`, which needs the frame's place as well as
            // its count.
            Step::Deep(_) => Ok(None),
            // One candidate, found by looking rather than by name, and stood
            // on the way a field is: as index nought, until the walk has moved
            // past it.
            Step::Stream => {
                if from > 0 {
                    return Ok(None);
                }
                Ok(self.stream_under(doc, node)?.map(|p| (0, p)))
            }
            Step::Tagged { key, tag, .. } => {
                if !self.is_list(doc, node)? {
                    return Ok(None);
                }
                // A label worked out rather than written down is worked out
                // where the gather is, since no one record is asking.
                let (key, tag) = (key.clone(), tag.clone());
                let tag = self.tag_now(doc, list, &tag, None)?;
                let n = self.child_count(doc, node)?;
                let mut p = node.to_vec();
                for i in 0..n as usize {
                    p.push(i);
                    if self.tag_matches(doc, &p, &key, &tag)? {
                        return Ok((from <= i).then_some((i, p)));
                    }
                    p.pop();
                }
                Ok(None)
            }
            Step::Each => {
                if !self.is_list(doc, node)? {
                    return Ok(None);
                }
                // A list of plain numbers holds no record: nothing in one has a
                // field to read an offset from. Asked of the type rather than
                // of every element, so a column of a million floats is passed
                // in one step. A list of runs is what a stitched stream is
                // looking for, so it is not passed at all.
                if let Ty::Array { elem, .. } | Ty::Repeat { elem, .. } = &self.memo[node].ty {
                    if landing == Landing::Records && self.holds_no_fields(elem) {
                        return Ok(None);
                    }
                }
                if from as u64 >= self.child_count(doc, node)? {
                    return Ok(None);
                }
                let mut p = node.to_vec();
                p.push(from);
                Ok(Some((from, p)))
            }
            Step::Placer => fail("only the first step of a walk can start at the record that placed it"),
            Step::Fields(names) => {
                let mut node = node.to_vec();
                self.into_contents(doc, &mut node)?;
                let Ty::Struct(s) = self.memo[&node].ty.base().clone() else { return Ok(None) };
                let Some(j) = (from..s.fields.len()).find(|&j| names.iter().any(|n| **n == *s.fields[j].name)) else {
                    return Ok(None);
                };
                let mut p = node;
                p.push(j);
                self.through_at(doc, &mut p)?;
                Ok(Some((j, p)))
            }
        }
    }

    /// Where a [`Step::Deep`] goes from `root`: the landing it stands on when
    /// `from` is the count it last landed at, and otherwise the next field
    /// called `name` after that landing, in the order a walk down through
    /// `root` meets them. Nothing when there are no more.
    ///
    /// The search carries on from the place it last landed rather than from
    /// the top, since the walk it is part of asks for one landing at a time
    /// and a tree of a thousand branches would otherwise be searched a
    /// thousand times. Opening a node for the first time is charged against
    /// the go, so a search that runs out stops where it can start again: at
    /// the landing before, with what it opened on the way still open.
    fn deep_step<S: Source>(
        &mut self,
        doc: &Document<S>,
        name: &str,
        root: &[usize],
        from: usize,
        last: Option<(usize, Vec<usize>)>,
    ) -> R<Option<(usize, Vec<usize>)>> {
        let (count, mut stack) = match last {
            Some((ord, p)) if from == ord => return Ok(Some((ord, p))),
            Some((ord, p)) if from == ord + 1 && p.starts_with(root) && p.len() > root.len() => {
                (ord + 1, (root.len()..p.len()).map(|k| (p[..k].to_vec(), p[k] + 1)).collect::<Vec<_>>())
            }
            None if from == 0 => (0, vec![(root.to_vec(), 0)]),
            _ => return Ok(None),
        };
        while let Some((parent, next)) = stack.last().cloned() {
            let (children, field) = self.deep_children(doc, &parent, next)?;
            if next >= children {
                stack.pop();
                continue;
            }
            if let Some(top) = stack.last_mut() {
                top.1 += 1;
            }
            let mut child = parent;
            child.push(next);
            if field.as_deref() == Some(name) {
                return Ok(Some((count, child)));
            }
            // Only a node nothing has opened before is charged: a go that ran
            // out starts the search again from the landing before, and paying
            // again for the nodes it already went through would spend every
            // go getting back to where the last one stopped.
            if !self.memo.contains_key(&child) {
                self.spend(0)?;
            }
            match self.resolve(doc, &child) {
                Ok(()) => stack.push((child, 0)),
                Err(e) if e.interrupted() => return Err(e),
                Err(_) => {}
            }
        }
        Ok(None)
    }

    /// How many children of `parent` a search at any depth goes into, and the
    /// name of child `idx` where `parent` is a structure. A list of plain
    /// numbers holds no fields to find, a list placed from records elsewhere
    /// is not the search's to walk, and of a stream only what it holds is
    /// looked in: its second child is what the decoder read.
    fn deep_children<S: Source>(&mut self, doc: &Document<S>, parent: &[usize], idx: usize) -> R<(usize, Option<String>)> {
        match self.resolve(doc, parent) {
            Ok(()) => {}
            Err(e) if e.interrupted() => return Err(e),
            Err(_) => return Ok((0, None)),
        }
        let ty = self.memo[parent].ty.clone();
        let counted = |ev: &mut Self| -> R<usize> {
            match ev.child_count(doc, parent) {
                Ok(n) => Ok(n as usize),
                Err(e) if e.interrupted() => Err(e),
                Err(_) => Ok(0),
            }
        };
        Ok(match ty.base() {
            Ty::Struct(s) => (s.fields.len(), s.fields.get(idx).map(|f| f.name.to_string())),
            Ty::At { .. } => (1, None),
            Ty::Decoded { .. } | Ty::Stitched { .. } => (counted(self)?.min(1), None),
            Ty::Array { elem, .. } | Ty::Repeat { elem, .. } => {
                if self.holds_no_fields(elem) {
                    (0, None)
                } else {
                    (counted(self)?, None)
                }
            }
            _ => (0, None),
        })
    }

    /// Whether the node at `path` is a list of the template's own, which is
    /// what a step that fans out over elements can fan out over.
    fn is_list<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<bool> {
        self.resolve(doc, path)?;
        Ok(matches!(
            self.memo[path].ty,
            Ty::Array { .. } | Ty::Repeat { .. } | Ty::PointerList { .. } | Ty::Chain { .. } | Ty::Gather { .. }
        ))
    }

    /// Whether an element of this type is a number or a run of bytes, looking
    /// through names and windows to what the element is. A switch could be
    /// anything, so it is not judged here.
    fn holds_no_fields(&self, ty: &Ty) -> bool {
        let mut ty = ty;
        for _ in 0..64 {
            match ty {
                Ty::Named(n) => match self.template.types.get(&**n) {
                    Some(t) => ty = t,
                    None => return false,
                },
                Ty::Sized { inner, .. } | Ty::SizedBits { inner, .. } | Ty::When { inner, .. } => ty = inner,
                other => return listing::plain(other) || matches!(other, Ty::Str { .. }),
            }
        }
        false
    }

    /// A field whose contents are elsewhere is its contents, here as in every
    /// path: naming it means what it points at.
    pub(super) fn through_at<S: Source>(&mut self, doc: &Document<S>, path: &mut Vec<usize>) -> R<()> {
        self.resolve(doc, path)?;
        if matches!(self.memo[path.as_slice()].ty, Ty::At { .. }) {
            path.push(0);
        }
        Ok(())
    }

    /// Where an expression is worked out when it is asked of a record: one past
    /// the record's last field, so that every field of it counts as written
    /// before, and the record's own start and room.
    ///
    /// The same frame a check is worked out in (see `check.rs`). Every node on
    /// the way down is opened first: the record was reached by a walk that may
    /// since have given some of them back.
    pub(super) fn record_frame<S: Source>(&mut self, doc: &Document<S>, record: &[usize]) -> R<(Vec<usize>, Option<(u64, u64)>)> {
        for k in 0..=record.len() {
            self.resolve(doc, &record[..k])?;
        }
        let r = &self.memo[record];
        let fields = match r.ty.base() {
            Ty::Struct(s) => s.fields.len(),
            _ => 0,
        };
        let here = Some((r.offset, r.limit));
        let mut end = record.to_vec();
        end.push(fields);
        Ok((end, here))
    }

    /// The gathered list `at` sits in, and which of its children `at` is or is
    /// inside: the nearest one, for a gather inside a gathered child.
    pub(super) fn gathered_in(&self, at: &[usize]) -> Option<(Vec<usize>, usize)> {
        (0..at.len())
            .rev()
            .find(|&k| matches!(self.memo.get(&at[..k]).map(|r| &r.ty), Some(Ty::Gather { .. })))
            .map(|k| (at[..k].to_vec(), at[k]))
    }

    /// The record that placed child `idx` of the gather at `list`.
    pub(super) fn gathered_record<S: Source>(&mut self, doc: &Document<S>, list: &[usize], idx: usize) -> R<Vec<usize>> {
        self.extend_gather_to(doc, list, idx)?;
        match self.list(list).gather.as_deref().and_then(|g| g.records.get(idx)) {
            Some(record) => Ok(record.clone()),
            None => fail("no descriptor placed this element"),
        }
    }

    /// Where [`Expr::Placer`] is worked out from `at`: the frame of the record
    /// that placed the gathered element `at` is, or is inside.
    pub(super) fn placer_frame<S: Source>(&mut self, doc: &Document<S>, at: &[usize]) -> R<(Vec<usize>, Option<(u64, u64)>)> {
        let Some((list, idx)) = self.gathered_in(at) else {
            return fail("no descriptor placed this field, so it has none to ask");
        };
        let record = self.gathered_record(doc, &list, idx)?;
        self.record_frame(doc, &record)
    }

    /// How many children the gather at `list` has, which is only known by
    /// walking to every record.
    pub(super) fn gather_count<S: Source>(&mut self, doc: &Document<S>, list: &[usize]) -> R<u64> {
        self.extend_gather_to(doc, list, usize::MAX)?;
        Ok(self.list(list).gather.as_deref().map_or(0, |g| g.starts.len() as u64))
    }

    /// Every child start of the gather at `list`, sorted, with its child. Sorted
    /// once, when the walk is done, and shared after that.
    pub(super) fn gather_sorted<S: Source>(&mut self, doc: &Document<S>, list: &[usize]) -> R<Arc<Vec<(u64, usize)>>> {
        self.extend_gather_to(doc, list, usize::MAX)?;
        let g = self.gather_mut(list);
        if let Some(sorted) = &g.sorted {
            return Ok(sorted.clone());
        }
        let mut sorted: Vec<(u64, usize)> = g.starts.iter().enumerate().map(|(i, s)| (*s, i)).collect();
        sorted.sort_unstable();
        let sorted = Arc::new(sorted);
        g.sorted = Some(sorted.clone());
        Ok(sorted)
    }

    /// The record that placed child `idx` of the gather at `list`, as a reader
    /// would name it: each step's name, with the index each fanning step took.
    /// `rows[3].col1[0]`, which is the descriptor a FITS heap array came from.
    pub(super) fn gathered_label<S: Source>(&mut self, doc: &Document<S>, list: &[usize], record: &[usize]) -> R<String> {
        let Ty::Gather { from, .. } = self.memo[list].ty.clone() else { return fail("not a gathered list") };
        self.walk_label(doc, list, &from, record)
    }

    /// The place a walk from `at` down `from` landed, named the way a reader
    /// would name it: each step's name, with the index each fanning step took.
    /// The same for a gather's record and a stitched stream's part.
    pub(super) fn walk_label<S: Source>(&mut self, doc: &Document<S>, at: &[usize], from: &[Step], record: &[usize]) -> R<String> {
        // Everything the label reads is a node on the way down to the record,
        // so those are opened first and the naming itself reads only the memo.
        for k in 0..=record.len() {
            self.resolve(doc, &record[..k])?;
        }
        Ok(self.walk_label_here(at, from, record))
    }

    /// The same, from what the memo already holds, for a caller with no
    /// document to open anything with: a read that fails in a stitched stream
    /// names the part it failed in. A step whose node has been given back is
    /// named by what can still be said about it.
    pub(super) fn walk_label_here(&self, at: &[usize], from: &[Step], record: &[usize]) -> String {
        let list = at;
        let mut label = String::new();
        let mut p: Vec<usize> = Vec::new();
        for (k, step) in from.iter().enumerate() {
            let dot = if label.is_empty() { "" } else { "." };
            if k == 0 {
                // Named the way the outer gather names that record, so the
                // label reads from the same place the walk started.
                if let Step::Placer = step {
                    let Some((outer, idx)) = self.gathered_in(list) else { break };
                    let Some(start) = self.list(&outer).gather.as_deref().and_then(|g| g.records.get(idx)).cloned() else { break };
                    let Some(Ty::Gather { from: outer_from, .. }) = self.memo.get(&outer).map(|r| r.ty.clone()) else { break };
                    label = self.walk_label_here(&outer, &outer_from, &start);
                    p = start;
                    continue;
                }
                let Step::Field(name) = step else { break };
                let Some(start) = self.find_field(list, name) else { break };
                label.push_str(name);
                p = start;
                continue;
            }
            // What a stream holds adds no name: the step before it named the
            // stream, and a field of its contents is a field of that.
            if matches!(step, Step::Field(_) | Step::Fields(_))
                && record.len() > p.len()
                && matches!(self.memo.get(&p).map(|r| &r.ty), Some(Ty::Decoded { .. } | Ty::Stitched { .. }))
            {
                p.push(0);
            }
            // A field found at any depth, named by every field on the way down
            // to it, since the step's own name says which field and not
            // which of the hundred it was.
            if let Step::Deep(want) = step {
                while record.len() > p.len() {
                    let Some(r) = self.memo.get(&p) else { break };
                    let j = record[p.len()];
                    let dot = if label.is_empty() { "" } else { "." };
                    match r.ty.base() {
                        Ty::Struct(s) => {
                            let name = s.fields.get(j).map(|f| f.name.to_string()).unwrap_or_default();
                            label.push_str(&format!("{dot}{name}"));
                            p.push(j);
                            if *name == **want {
                                break;
                            }
                        }
                        Ty::Array { .. } | Ty::Repeat { .. } => {
                            label.push_str(&format!("[{j}]"));
                            p.push(j);
                        }
                        _ => p.push(j),
                    }
                }
                continue;
            }
            // The run a stream step found, named by the fields on the way down
            // to it, since no one name in the template says where it is.
            if let Step::Stream = step {
                while record.len() > p.len() {
                    let Some(r) = self.memo.get(&p) else { break };
                    if matches!(r.ty, Ty::Decoded { .. } | Ty::Stitched { .. }) {
                        break;
                    }
                    let j = record[p.len()];
                    if let Ty::Struct(s) = r.ty.base() {
                        let name = s.fields.get(j).map(|f| f.name.to_string()).unwrap_or_default();
                        let dot = if label.is_empty() { "" } else { "." };
                        label.push_str(&format!("{dot}{name}"));
                    }
                    p.push(j);
                }
                continue;
            }
            let Some(&j) = record.get(p.len()) else { break };
            p.push(j);
            match step {
                Step::Stream | Step::Deep(_) => {}
                Step::Field(name) => label.push_str(&format!("{dot}{name}")),
                Step::Tagged { shown, .. } => label.push_str(&format!("{dot}{shown}")),
                Step::Each => label.push_str(&format!("[{j}]")),
                Step::Placer => {}
                Step::Fields(_) => {
                    let name = match self.memo.get(&p[..p.len() - 1]).map(|r| r.ty.base()) {
                        Some(Ty::Struct(s)) => s.fields.get(j).map(|f| f.name.to_string()).unwrap_or_default(),
                        _ => String::new(),
                    };
                    label.push_str(&format!("{dot}{name}"));
                }
            }
            if matches!(step, Step::Field(_) | Step::Fields(_))
                && record.len() > p.len()
                && matches!(self.memo.get(&p).map(|r| &r.ty), Some(Ty::At { .. }))
            {
                p.push(0);
            }
        }
        label
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::MemSource;
    use crate::template::{Expr as E, Ty as T};

    /// A node holding nodes, as deep as the file likes, each with a list of
    /// hits; and a gather placing one byte per hit, wherever the hit is.
    fn nested() -> Template {
        let node = T::structure(
            "Node",
            vec![
                ("n", T::u8()),
                ("kids", T::array(T::Named("Node".into()), E::field("n"))),
                ("hits_n", T::u8()),
                ("hits", T::array(T::structure("Hit", vec![("off", T::u8())]), E::field("hits_n"))),
            ],
        );
        let root = T::structure(
            "Root",
            vec![
                ("tree", T::Named("Node".into())),
                ("found", T::gather(vec![Step::field("tree"), Step::deep("hits"), Step::each()], E::field("off"), Anchor::File, E::lit(0), T::u8())),
            ],
        );
        Template::new("t", root).with_type("Node", node)
    }

    /// A root with two kids, the first of which has a kid of its own. Five
    /// hits between them, each pointing at a byte from 30 on.
    fn bytes() -> Vec<u8> {
        let mut b = vec![2];
        b.extend([1, 0, 1, 31, 1, 32]); // kid 0: one kid with one hit, and a hit
        b.extend([0, 2, 33, 34]); // kid 1: no kids, two hits
        b.extend([1, 30]); // the root's own hit
        b.resize(40, 0xee);
        b
    }

    #[test]
    fn a_walk_finds_a_field_at_any_depth() {
        let d = Document::new(MemSource(bytes()));
        let mut ev = Evaluator::new(nested());
        let found = ev.node(&d, &[1]).unwrap();
        assert_eq!(found.child_count, 5);
        // In the order a walk down through the tree meets them: a node's kids
        // before its own hits, since the kids come first in it.
        let at: Vec<u64> = (0..5).map(|i| ev.node(&d, &[1, i]).unwrap().offset_bits / 8).collect();
        assert_eq!(at, [31, 32, 33, 34, 30]);
        // Each named by every field on the way down to the record.
        let label = |ev: &mut Evaluator, i: usize| {
            let record = ev.gathered_record(&d, &[1], i).unwrap();
            ev.gathered_label(&d, &[1], &record).unwrap()
        };
        assert_eq!(label(&mut ev, 0), "tree.kids[0].kids[0].hits[0]");
        assert_eq!(label(&mut ev, 3), "tree.kids[1].hits[1]");
        assert_eq!(label(&mut ev, 4), "tree.hits[0]");
    }

    #[test]
    fn a_walk_at_any_depth_carries_on_across_goes() {
        let d = Document::new(MemSource(bytes()));
        let mut whole = Evaluator::new(nested());
        let want: Vec<u64> = (0..5).map(|i| whole.node(&d, &[1, i]).unwrap().offset_bits).collect();
        let mut ev = Evaluator::new(nested());
        ev.set_slice(Some(1));
        let mut goes = 0;
        let n = loop {
            goes += 1;
            assert!(goes < 100, "the walk is not getting any further");
            ev.begin_slice();
            match ev.node(&d, &[1]) {
                Ok(info) => break info.child_count,
                Err(EvalError::Busy { .. }) => {}
                Err(e) => panic!("{e:?}"),
            }
        };
        assert!(goes > 2, "{goes}");
        assert_eq!(n, 5);
        ev.set_slice(None);
        let got: Vec<u64> = (0..5).map(|i| ev.node(&d, &[1, i]).unwrap().offset_bits).collect();
        assert_eq!(got, want);
    }
}
