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
        // The walk starts where the gather is declared: the first step names a
        // field before it, the way the first name of a path does.
        {
            let g = self.gather_mut(list);
            if !g.started {
                g.frames = vec![GatherFrame { node: list.to_vec(), next: 0 }];
                g.started = true;
            }
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
            let Some(record) = self.gather_record(doc, list, &from)? else {
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
            if let Some(top) = g.frames.last_mut() {
                top.next += 1;
            }
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

    /// The record the walk stands on, or the next one after the frames have
    /// moved past everything that holds none. Nothing when the walk is over.
    ///
    /// Moving the frames on is not moving past a record: the walk stands on a
    /// record until the caller has read it, so a read that has to wait for
    /// bytes asks this again and is handed the same one.
    fn gather_record<S: Source>(&mut self, doc: &Document<S>, list: &[usize], from: &[Step]) -> R<Option<Vec<usize>>> {
        loop {
            let (k, node, next) = {
                let Some(g) = self.list(list).gather.as_deref() else { return Ok(None) };
                let Some(top) = g.frames.last() else { return Ok(None) };
                (g.frames.len() - 1, top.node.clone(), top.next)
            };
            let Some(step) = from.get(k) else { return Ok(None) };
            let got = match self.gather_step(doc, list, step, k, &node, next) {
                Err(e) if !e.interrupted() => None,
                other => other?,
            };
            let g = self.gather_mut(list);
            match got {
                Some((j, child)) => {
                    g.frames[k].next = j;
                    if k + 1 == from.len() {
                        return Ok(Some(child));
                    }
                    g.frames.push(GatherFrame { node: child, next: 0 });
                }
                // Nothing more down this way, so the step above moves on.
                None => {
                    g.frames.pop();
                    if let Some(up) = g.frames.last_mut() {
                        up.next += 1;
                    }
                }
            }
        }
    }

    /// Where step `k` goes from `node`, trying its candidates from `from` on:
    /// the index of the child it takes, and the path it lands on. Nothing when
    /// it has no candidate there.
    fn gather_step<S: Source>(
        &mut self,
        doc: &Document<S>,
        list: &[usize],
        step: &Step,
        k: usize,
        node: &[usize],
        from: usize,
    ) -> R<Option<(usize, Vec<usize>)>> {
        // The first step starts the walk, and only a field declared before the
        // gather can be where it starts.
        if k == 0 {
            let Step::Field(name) = step else { return fail("a gather starts at a field declared before it") };
            return Ok(if from == 0 { self.find_field(list, name).map(|p| (0, p)) } else { None });
        }
        match step {
            Step::Field(name) => {
                if from > 0 {
                    return Ok(None);
                }
                let Some(j) = self.child_index(doc, node, name)? else { return Ok(None) };
                let mut p = node.to_vec();
                p.push(j);
                self.through_at(doc, &mut p)?;
                Ok(Some((j, p)))
            }
            Step::Tagged { key, tag, .. } => {
                if from > 0 || !self.is_list(doc, node)? {
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
                        return Ok(Some((i, p)));
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
                // in one step.
                if let Ty::Array { elem, .. } | Ty::Repeat { elem, .. } = &self.memo[node].ty {
                    if self.holds_no_fields(elem) {
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
            Step::Fields(names) => {
                self.resolve(doc, node)?;
                let Ty::Struct(s) = self.memo[node].ty.base().clone() else { return Ok(None) };
                let Some(j) = (from..s.fields.len()).find(|&j| names.iter().any(|n| **n == *s.fields[j].name)) else {
                    return Ok(None);
                };
                let mut p = node.to_vec();
                p.push(j);
                self.through_at(doc, &mut p)?;
                Ok(Some((j, p)))
            }
        }
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
                Ty::Sized { inner, .. } | Ty::SizedBits { inner, .. } => ty = inner,
                other => return listing::plain(other) || matches!(other, Ty::Str { .. }),
            }
        }
        false
    }

    /// A field whose contents are elsewhere is its contents, here as in every
    /// path: naming it means what it points at.
    fn through_at<S: Source>(&mut self, doc: &Document<S>, path: &mut Vec<usize>) -> R<()> {
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
            None => fail("nothing in the gathered list placed this"),
        }
    }

    /// Where [`Expr::Placer`] is worked out from `at`: the frame of the record
    /// that placed the gathered element `at` is, or is inside.
    pub(super) fn placer_frame<S: Source>(&mut self, doc: &Document<S>, at: &[usize]) -> R<(Vec<usize>, Option<(u64, u64)>)> {
        let Some((list, idx)) = self.gathered_in(at) else {
            return fail("nothing placed this: only an element of a gathered list has a record to ask");
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
        let mut label = String::new();
        let mut p: Vec<usize> = Vec::new();
        for (k, step) in from.iter().enumerate() {
            let dot = if label.is_empty() { "" } else { "." };
            if k == 0 {
                let Step::Field(name) = step else { break };
                let Some(start) = self.find_field(list, name) else { break };
                label.push_str(name);
                p = start;
                continue;
            }
            let Some(&j) = record.get(p.len()) else { break };
            p.push(j);
            match step {
                Step::Field(name) => label.push_str(&format!("{dot}{name}")),
                Step::Tagged { shown, .. } => label.push_str(&format!("{dot}{shown}")),
                Step::Each => label.push_str(&format!("[{j}]")),
                Step::Fields(_) => {
                    self.resolve(doc, &p[..p.len() - 1])?;
                    let name = match self.memo[&p[..p.len() - 1]].ty.base() {
                        Ty::Struct(s) => s.fields.get(j).map(|f| f.name.to_string()).unwrap_or_default(),
                        _ => String::new(),
                    };
                    label.push_str(&format!("{dot}{name}"));
                }
            }
            if matches!(step, Step::Field(_) | Step::Fields(_)) && record.len() > p.len() {
                self.resolve(doc, &p)?;
                if matches!(self.memo[&p].ty, Ty::At { .. }) {
                    p.push(0);
                }
            }
        }
        Ok(label)
    }
}
