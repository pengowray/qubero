//! Where the fields that read their contents somewhere else put them.
//!
//! A field declared with [`Ty::At`](crate::template::Ty::At) costs no bytes
//! where it stands and covers a stretch of the file that may be nowhere near
//! it. For most formats that is a detail: a WAD's directory sits inside the
//! region the lumps cover, so a reader landing on a byte is already inside
//! something the walk from the root reaches by ordinary descent.
//!
//! HDF5 is the format where that stopped being true. Every object in one is
//! reached by address, so the root structure ends ninety-six bytes into the
//! file and the other hundred megabytes are covered only by what `At` placed.
//! Asking what byte ten million is used to answer "outside the template",
//! which is wrong in the way that matters: the template knows exactly what is
//! there, and only the walk to it was missing.
//!
//! So this keeps an index of every stretch an `At` placed, and of the field
//! that placed it. `locate` asks the index when the bit it was given is
//! outside what the root covers, and the descent carries on from there as it
//! always did.
//!
//! Four things make it affordable.
//!
//! The walk is pruned by the template rather than by the file: a type holding
//! no `At`, directly or through anything it contains, cannot place anything,
//! and a whole branch is skipped without reading a byte of it. The same
//! question is asked of a structure's fields one by one, so a variable-length
//! element costs the one field that points somewhere and not the three that
//! do not.
//!
//! A list is judged by what its first element turns out to be rather than by
//! what its element type could be. That type is a switch over every datatype
//! the format has and one of those cases does place something, so the type
//! alone always says yes; the element says no, and thirteen million numbers
//! are skipped in one step.
//!
//! A stretch already in the index is not walked into again. A column of a
//! hundred thousand strings points every element at a handful of heap
//! collections, and the second element to name one has nothing to add.
//!
//! The walk is resumable. It keeps its own stack rather than the call stack,
//! does a bounded number of steps per go, and stops with that stack where it
//! was, so a file with a hundred thousand chunks is indexed over several
//! questions rather than freezing the first one. A bit covered by nothing
//! indexed yet reads as a gap, which is what every byte outside the root read
//! as before any of this existed, and becomes a field as the walk reaches it.
//! The index goes when the memo goes, since a placement is where a resolved
//! node turned out to be.

use super::*;

/// A stretch of the file that a field placed there, and the field.
#[derive(Debug, Clone)]
pub(super) struct Placement {
    pub start: u64,
    pub end: u64,
    pub path: Vec<usize>,
}

/// One node of the walk that has been opened and not finished with.
#[derive(Debug, Clone)]
pub(super) struct Frame {
    path: Vec<usize>,
    /// How many children it has, and which of them is next.
    count: u64,
    next: u64,
    /// Which of a structure's fields could place anything. None for a list,
    /// whose elements are all the same.
    fields: Option<Vec<usize>>,
    /// How many of its elements in a row have added nothing to the index.
    stale: usize,
}

/// How many nodes one go of the walk may open. Enough that a file of a few
/// hundred megabytes is indexed in one go, and few enough that a larger one
/// comes back rather than holding the frame.
const STEP: usize = 60_000;

/// How many nodes the index may open in all. What is not reached reads as a
/// gap; the alternative is a walk with no end on a file with no end of chunks.
const BUDGET: usize = 2_000_000;

/// How many elements of one list may point at nothing new before the rest of
/// it is left alone. A column of strings fills one heap collection before it
/// starts the next, so a run this long saying nothing new means the collection
/// it names was indexed a while ago.
const SAME_ANSWER: usize = 64;

impl Evaluator {
    /// The narrowest placed stretch covering `bit`, and the field that placed
    /// it. Narrowest because placements nest: a link's name sits inside the
    /// heap's data segment, and both were placed by an `At`.
    pub(super) fn placement_at<S: Source>(&mut self, doc: &Document<S>, bit: u64) -> R<Option<Vec<usize>>> {
        self.index_placements(doc)?;
        Ok(self
            .placements
            .iter()
            .filter(|p| p.start <= bit && bit < p.end)
            .min_by_key(|p| p.end - p.start)
            .map(|p| p.path.clone()))
    }

    /// Where the next placed stretch after `bit` begins, which is how far a
    /// stretch nothing covers runs.
    pub(super) fn placement_after<S: Source>(&mut self, doc: &Document<S>, bit: u64) -> R<Option<u64>> {
        self.index_placements(doc)?;
        Ok(self.placements.iter().map(|p| p.start).filter(|&s| s > bit).min())
    }

    /// Carry the walk on for one go.
    fn index_placements<S: Source>(&mut self, doc: &Document<S>) -> R<()> {
        if self.placements_done {
            return Ok(());
        }
        if self.frontier.is_empty() && self.placed_opened == 0 {
            match self.frame(doc, Vec::new())? {
                Some(frame) => self.frontier.push(frame),
                None => {
                    self.placements_done = true;
                    return Ok(());
                }
            }
        }
        let stop = self.placed_opened + STEP;
        while self.placed_opened < stop {
            if self.placed_opened >= BUDGET {
                self.placements_done = true;
                break;
            }
            let Some(top) = self.frontier.last_mut() else {
                self.placements_done = true;
                break;
            };
            if top.next >= top.count || top.stale >= SAME_ANSWER {
                self.frontier.pop();
                continue;
            }
            let i = top.next;
            top.next += 1;
            let listy = top.fields.is_none();
            let depth = top.path.len();
            if top.fields.as_ref().is_some_and(|keep| !keep.contains(&(i as usize))) {
                continue;
            }
            let mut child = self.frontier[self.frontier.len() - 1].path.clone();
            child.push(i as usize);
            self.placed_opened += 1;
            let before = self.placements.len();
            // Charged against this go's allowance like any other element, so
            // a caller drawing frames gets `Busy` and its screen back rather
            // than a walk that holds the thread. The stack below is what makes
            // coming back cheap.
            let charge = self.spend(0).and_then(|()| self.open(doc, child));
            match charge {
                Ok(Some(frame)) => self.frontier.push(frame),
                Ok(None) => {}
                Err(e) if e.interrupted() => {
                    // Not read yet, or this go is over. Put the child back, so
                    // the next go opens it rather than stepping over it.
                    if let Some(f) = self.frontier.iter_mut().rev().find(|f| f.path.len() == depth) {
                        f.next -= 1;
                    }
                    self.placements.sort_by_key(|p| (p.start, p.end));
                    return Err(e);
                }
                // A branch that will not parse says nothing about the rest of
                // the file, and an index missing one stretch beats none.
                Err(_) => {}
            }
            if listy {
                let grew = self.placements.len() > before;
                if let Some(f) = self.frontier.iter_mut().rev().find(|f| f.path.len() == depth) {
                    f.stale = if grew { 0 } else { f.stale + 1 };
                }
            }
        }
        self.placements.sort_by_key(|p| (p.start, p.end));
        Ok(())
    }

    /// Look at one node: record what it places, and say whether the walk
    /// should carry on into it.
    fn open<S: Source>(&mut self, doc: &Document<S>, path: Vec<usize>) -> R<Option<Frame>> {
        self.resolve(doc, &path)?;
        let ty = self.memo[&path].ty.clone();
        if !matches!(ty, Ty::At { .. }) {
            return self.frame(doc, path);
        }
        let mut inner = path.clone();
        inner.push(0);
        self.resolve(doc, &inner)?;
        let start = self.memo[&inner].offset;
        let size = self.size_of(doc, &inner)?;
        if size == 0 {
            return Ok(None);
        }
        if !self.placed_ranges.insert((start, start + size)) {
            return Ok(None);
        }
        self.placements.push(Placement { start, end: start + size, path });
        self.frame(doc, inner)
    }

    /// A node to carry on into, where there is anything inside it worth
    /// carrying on into.
    fn frame<S: Source>(&mut self, doc: &Document<S>, path: Vec<usize>) -> R<Option<Frame>> {
        self.resolve(doc, &path)?;
        let ty = self.memo[&path].ty.clone();
        if !self.may_place(&ty) {
            return Ok(None);
        }
        let count = self.child_count(doc, &path)?;
        if count == 0 {
            return Ok(None);
        }
        let fields = match ty.base() {
            Ty::Struct(def) => {
                Some((0..def.fields.len()).filter(|&i| self.may_place(&def.fields[i].ty)).collect())
            }
            _ => None,
        };
        // What one element of a list turns out to be settles the whole run.
        if fields.is_none() && !matches!(ty, Ty::At { .. }) {
            let mut first = path.clone();
            first.push(0);
            self.resolve(doc, &first)?;
            let settled = self.memo[&first].ty.clone();
            if !self.may_place(&settled) {
                return Ok(None);
            }
        }
        Ok(Some(Frame { path, count, next: 0, fields, stale: 0 }))
    }

    /// Whether anything inside this type places its contents elsewhere. False
    /// prunes the whole branch without reading a byte of it.
    fn may_place(&self, ty: &Ty) -> bool {
        let mut seen = Vec::new();
        self.places(ty, &mut seen)
    }

    fn places(&self, ty: &Ty, seen: &mut Vec<String>) -> bool {
        match ty {
            Ty::At { .. } => true,
            Ty::Named(name) => {
                // A type that refers to itself is answered by its other
                // fields: saying no for the loop is safe, since a type whose
                // only way to an `At` is through itself never reaches one.
                if seen.iter().any(|s| s == &**name) {
                    return false;
                }
                seen.push(name.to_string());
                let answer = self.template.types.get(&**name).is_some_and(|t| self.places(t, seen));
                seen.pop();
                answer
            }
            Ty::Struct(s) => s.fields.iter().any(|f| self.places(&f.ty, seen)),
            Ty::Array { elem, .. } | Ty::Repeat { elem, .. } | Ty::PointerList { elem, .. } => {
                self.places(elem, seen)
            }
            Ty::Sized { inner, .. } => self.places(inner, seen),
            Ty::Switch { cases, default, .. } => {
                cases.iter().any(|(_, t)| self.places(t, seen)) || self.places(default, seen)
            }
            Ty::Enum { inner, .. } | Ty::Flags { inner, .. } => self.places(inner, seen),
            _ => false,
        }
    }
}
