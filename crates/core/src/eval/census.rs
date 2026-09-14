//! What the open file actually has, counted against the diagram's boxes.
//!
//! `diagram.rs` draws the format: every type the template declares, whether or
//! not the file in front of the reader holds one. That is the right picture and
//! it is half the question. A PNG's diagram has a box for every chunk type the
//! format defines, and a reader looking at one particular PNG wants to know
//! which of them this file has, how many, and where the first one is.
//!
//! So this walks the file the evaluator has already read and counts its nodes
//! against those boxes. Three things come back per box: how many the file
//! holds, which row of it each node stood on, and the path to the first one, so
//! the view can badge a box with a count, draw an unused one quietly, and take
//! a reader who asks to the bytes.
//!
//! **Keyed by the diagram's own key.** [`crate::eval::diagram::box_key`] is
//! what the drawing shares boxes by, and it is what this counts by. A census
//! keyed even slightly differently would put real counts on boxes that are not
//! there and leave the drawn ones reading empty, which is worse than no census:
//! a reader would trust it.
//!
//! **Values are not read**, the way [`super::graph`] does not read them: what a
//! field says is the expensive half of reading it and no badge shows it.
//!
//! **Kept between goes.** The walk is a [`CensusWalk`] the caller holds, the
//! way it holds a [`super::KindWalk`], and each [`Evaluator::census_step`]
//! carries it on until this go's allowance runs out, the bytes it needs have
//! not arrived, or it has counted as many nodes as it was allowed. Which of
//! those it was is [`CensusState`], because the caller does something
//! different for each: ask again at once, ask again when bytes land, or ask the
//! reader. The first version started from the root on every call and marked
//! all three the same way, as "stopped short": each go spent its allowance
//! walking the nodes the last go had already counted, and a view that only
//! asked again on a reply that was not `ok` never asked again.
//!
//! **Depth-first, and forgetting behind itself.** A count that runs to the end
//! of a file reads every node of it, and a breadth-first queue holds a whole
//! level of the tree at once: 184,000 nodes of a 200 KB ROOT file were all in
//! memory together. So this walks the way [`super::KindWalk`] does, one frame
//! per open node, and gives each node back to the memo once the walk is past
//! it. What a limit keeps is then the first part of the file in order rather
//! than the top of all of it, which is what "Counted the first N fields" says.
//!
//! **Exact, or it says it is not.** A run whose elements are all the same
//! shape, which the template settles, is counted by walking its first element
//! and counting what is in it once per element: a WAV's half a million samples
//! are one sample walked. Every other run is walked element by element, since
//! what one element holds says nothing about the next: sampling the first 32
//! of a journal's 156 objects badged its object box `×32`, and every object
//! type found only past the 32nd was drawn as one the file has none of. A count that finished is the file's; one that did not says so.

use rustc_hash::FxHashMap;

use super::diagram::{box_identity, box_key, is_choice, is_run};
use super::size::same_shape;
use super::*;

/// How many boxes of one kind the file holds, and where the first is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxCount {
    /// The diagram box this counts, by [`crate::eval::diagram::box_key`].
    pub key: String,
    pub count: u64,
    /// The path to the first one the walk met, which depth-first in file order
    /// is the first one in the file.
    pub first_path: Vec<usize>,
    /// Which reading the path is in: 0 is the file, anything else an unpacked
    /// stream. See [`super::space`].
    pub space: u32,
}

/// The same for one row of one box: one field of a type, counted over every
/// node of that type the file holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowCount {
    pub key: String,
    pub row: usize,
    pub count: u64,
    pub first_path: Vec<usize>,
    pub space: u32,
}

/// Where a count has got to, and so what the caller does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CensusState {
    /// Every node was counted. The counts are the file's.
    Done,
    /// This go's allowance ran out. Asking again carries on at once.
    Working,
    /// The bytes the next node needs have not arrived. They have been asked
    /// for (see [`Evaluator::wanted`]); asking again once they land carries on.
    Waiting,
    /// The walk counted as many nodes as it was allowed, with more to count.
    /// Asking again with a higher limit carries on; asking with the same one
    /// answers the same.
    Capped,
}

/// What the file holds, against what the format can hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Census {
    pub boxes: Vec<BoxCount>,
    pub rows: Vec<RowCount>,
    /// How many nodes the walk has looked at. A run counted from its first
    /// element counts as the nodes walked, not as the elements it stands for.
    pub walked: u64,
    /// Anything but [`CensusState::Done`] makes every count a floor rather
    /// than a total, and a view that does not say so is showing a number that
    /// looks like an answer.
    pub state: CensusState,
}

/// One node the walk is inside: its children, and how far through them it is.
struct Frame {
    path: Vec<usize>,
    /// The box this node's children are rows of. None for a run, whose
    /// children are its elements and stand on nobody's row: counting the run
    /// as one of them would make every list in the file one longer than it is.
    own: Option<String>,
    /// The next child to count.
    next: usize,
    /// How many children to count: all of them, or one for a run whose
    /// elements are all the same shape.
    count: u64,
    /// How many nodes of the file each child counted here stands for: one,
    /// or the length of every same-shaped run above it multiplied together.
    weight: u64,
    /// True for a run, and for a node whose children are placed one per
    /// offset. Those let their children go one behind the walk, since placing
    /// an element asks the one before it where it ends; everything else gives
    /// its children back when it closes, since one field's count or length can
    /// be read off an earlier one.
    guarded: bool,
    born: Vec<Vec<usize>>,
    prev: Option<Vec<usize>>,
}

/// The count, kept between goes.
///
/// Thrown away whenever the document or its template changes, for the reason a
/// [`super::KindWalk`] is: every path in it is a path through bytes and a
/// template that may no longer be there.
pub struct CensusWalk {
    stack: Vec<Frame>,
    boxes: FxHashMap<String, BoxCount>,
    rows: FxHashMap<(String, usize), RowCount>,
    /// The key of a type, written out once. The key is the type written out,
    /// and writing one out per node is what a census costs if nothing is
    /// remembered: a WAV of five thousand nodes took two minutes before this.
    /// Keyed by the pointer the IR already shares, with the type kept beside
    /// it so that the pointer cannot be freed and handed to another type while
    /// the walk still holds the key.
    keys: FxHashMap<usize, (Ty, Option<String>)>,
    walked: u64,
    /// True once the root has been counted, which tells a walk that has not
    /// begun from one that has finished: both have an empty stack.
    started: bool,
    done: bool,
    /// The document's length in bits when the walk began, so a caller can tell
    /// a walk about the document in hand from one that is not.
    file_bits: u64,
    /// Where the walk starts: the file's root, or the contents of a stream
    /// opened as a tab. See [`Tab`](super::Tab).
    root: Vec<usize>,
    /// False to walk every element of every run, however alike the template
    /// says they are. See [`CensusWalk::element_by_element`].
    multiply: bool,
}

impl CensusWalk {
    pub fn new(file_bits: u64) -> CensusWalk {
        CensusWalk::under(file_bits, Vec::new())
    }

    /// A count of the nodes under `root`, in a document of `file_bits` bits.
    pub fn under(file_bits: u64, root: Vec<usize>) -> CensusWalk {
        CensusWalk {
            stack: Vec::new(),
            boxes: FxHashMap::default(),
            rows: FxHashMap::default(),
            keys: FxHashMap::default(),
            walked: 0,
            started: false,
            done: false,
            file_bits,
            root,
            multiply: true,
        }
    }

    /// The same count with every element of every run walked, even where the
    /// template says the elements are all the same shape and the first could
    /// stand for the rest. Slower by the length of every such run, and what
    /// the count that multiplies is checked against.
    pub fn element_by_element(mut self) -> CensusWalk {
        self.multiply = false;
        self
    }

    pub fn file_bits(&self) -> u64 {
        self.file_bits
    }

    pub fn done(&self) -> bool {
        self.done
    }

    /// What has been counted so far, ready to hand over.
    pub fn census(&self, state: CensusState) -> Census {
        // Biggest first, so a view that shows some of them shows the ones worth
        // showing, and so two runs over one file answer in the same order.
        let mut boxes: Vec<BoxCount> = self.boxes.values().cloned().collect();
        boxes.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.key.cmp(&b.key)));
        let mut rows: Vec<RowCount> = self.rows.values().cloned().collect();
        rows.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| (&a.key, a.row).cmp(&(&b.key, b.row))));
        Census { boxes, rows, walked: self.walked, state }
    }

    /// The key of a type, written out once and remembered.
    fn key(&mut self, t: &Template, ty: &Ty) -> Option<String> {
        match box_identity(t, ty) {
            Some(id) => self.keys.entry(id).or_insert_with(|| (ty.clone(), box_key(t, ty))).1.clone(),
            None => box_key(t, ty),
        }
    }

    fn add_box(&mut self, key: &str, n: u64, path: &[usize], space: u32) {
        let e = self.boxes.entry(key.to_string()).or_insert_with(|| BoxCount {
            key: key.to_string(),
            count: 0,
            first_path: path.to_vec(),
            space,
        });
        e.count = e.count.saturating_add(n);
    }

    fn add_row(&mut self, key: &str, row: usize, n: u64, path: &[usize], space: u32) {
        let e = self.rows.entry((key.to_string(), row)).or_insert_with(|| RowCount {
            key: key.to_string(),
            row,
            count: 0,
            first_path: path.to_vec(),
            space,
        });
        e.count = e.count.saturating_add(n);
    }
}

/// Everything about one node that reading the file could interrupt, asked
/// before the walk writes anything down. An interruption part-way through a
/// node leaves the walk where it was, and the next go asks the same node again
/// from the memo: nothing is counted twice by resuming.
struct Seen {
    space: u32,
    declared: Ty,
    resolved: Ty,
    /// The type that makes this node a run, when it is one.
    run: Option<Ty>,
    /// How many children it has.
    children: u64,
    guarded: bool,
}

impl Evaluator {
    /// Count the open file's nodes against the diagram's boxes in one call,
    /// stopping after `limit` of them. For a caller with nothing to draw
    /// meanwhile; the web app holds a [`CensusWalk`] and steps it.
    pub fn census<S: Source>(&mut self, doc: &Document<S>, limit: usize) -> R<Census> {
        let mut walk = CensusWalk::new(doc.len_bits());
        self.census_step(doc, &mut walk, limit)
    }

    /// Carry the count on for one go, and say what it has found and why it
    /// stopped. See [`CensusState`].
    ///
    /// `limit` is how many nodes the walk may have looked at in all, not in
    /// this go, so raising it carries a capped walk on from where it stopped.
    ///
    /// A node that will not read is passed over rather than taking the count
    /// with it, the same way the graph carries on past a field it cannot
    /// place: a count that is short by one broken record is worth more than no
    /// count. The root not reading is an error, since then there is nothing.
    pub fn census_step<S: Source>(&mut self, doc: &Document<S>, walk: &mut CensusWalk, limit: usize) -> R<Census> {
        let state = match self.census_run(doc, walk, limit) {
            Ok(state) => state,
            Err(EvalError::Busy { .. }) => CensusState::Working,
            // What has been counted is true, and the bytes are on their way.
            // Answered with the counts rather than as pending, so the view has
            // them to draw while it waits.
            Err(EvalError::Pending(missing)) => {
                self.want(missing);
                CensusState::Waiting
            }
            Err(e) => return Err(e),
        };
        Ok(walk.census(state))
    }

    fn census_run<S: Source>(&mut self, doc: &Document<S>, walk: &mut CensusWalk, limit: usize) -> R<CensusState> {
        if walk.done {
            return Ok(CensusState::Done);
        }
        if !walk.started {
            if limit == 0 {
                return Ok(CensusState::Capped);
            }
            let root = walk.root.clone();
            let seen = self.see(doc, &root)?;
            self.count_seen(walk, &root, None, 1, seen);
            walk.started = true;
        }
        loop {
            let Some(top) = walk.stack.len().checked_sub(1) else {
                walk.done = true;
                return Ok(CensusState::Done);
            };
            let f = &walk.stack[top];
            if f.next as u64 >= f.count {
                // Its size, asked before its children go back, while they are
                // still in the memo to answer it. Placing the sibling after
                // this node asks where this one ends, and a node whose size was
                // never asked answers by reading its fields again, which by then
                // have been given back: they came back with nothing left to let
                // go of them, two nodes per record of a ten-thousand-record run.
                // Asked at the end rather than on arrival, since sizing a list
                // on arrival walks all of it before the first element is
                // counted.
                let path = f.path.clone();
                match self.size_of(doc, &path) {
                    Err(e) if e.interrupted() => return Err(e),
                    _ => {}
                }
                self.close_census_frame(walk);
                continue;
            }
            if walk.walked >= limit as u64 {
                return Ok(CensusState::Capped);
            }
            let idx = f.next;
            let mut path = f.path.clone();
            path.push(idx);
            let row = f.own.clone().map(|k| (k, idx));
            let weight = f.weight;
            match self.see(doc, &path) {
                Ok(seen) => {
                    walk.stack[top].next += 1;
                    self.note_census_child(walk, top, &path);
                    self.count_seen(walk, &path, row, weight, seen);
                }
                Err(e) if e.interrupted() => return Err(e),
                // The first child that will not place ends the node.
                Err(_) => walk.stack[top].count = idx as u64,
            }
        }
    }

    /// Ask everything about the node at `path` that could be interrupted.
    fn see<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Seen> {
        self.resolve(doc, path)?;
        let (space, offset, resolved) = {
            let r = &self.memo[path];
            (r.space, r.offset, r.ty.clone())
        };
        // Charged like every other walk, so a count of a file with millions of
        // fields hands the caller their screen back.
        self.spend(offset)?;
        // What the row this node stands on is: the field's own declaration, and
        // not what it turned out to be. The diagram drew the declaration, so a
        // switch is the switch box and not the case it took. A node with no
        // declaration of its own, a member of a parsed JSON value, is what it
        // turned out to be.
        let mut declared = self.declared_ty(path).unwrap_or_else(|_| resolved.clone());
        // A field declared as a named type is that type. A GGUF metadata
        // value is `Value`, which is a switch, and read only as the name it
        // was not a choice: the switch's box stayed uncounted over a file
        // whose every metadata entry takes it.
        for _ in 0..16 {
            let Ty::Named(n) = &declared else { break };
            match self.template.types.get(&**n) {
                Some(t) => declared = t.clone(),
                None => break,
            }
        }
        // Whether this node is a run of something rather than one of it.
        //
        // Asked of what it turned out to be as well as of its declaration,
        // because a switch that picks a list is a list: a WAV's `data` chunk is
        // declared as a choice and resolves to half a million samples.
        let run = if is_run(&self.template, &declared) {
            Some(declared.clone())
        } else if is_run(&self.template, &resolved) {
            Some(resolved.clone())
        } else {
            None
        };
        // How many children, even of a run only walking settles the length of.
        // The count is the listing's own, walked the way the listing walks a
        // long run, so this counts the elements the listing shows and stops
        // where it stops: a run that ends in a broken element ends before it.
        // Asking for elements until one would not place instead counted one
        // past the end, since a place at the end of the room is still a place.
        let children = match self.child_count(doc, path) {
            Ok(n) => n,
            Err(e) if e.interrupted() => return Err(e),
            // A node whose children cannot be counted is still a node: it is
            // counted, and nothing under it is.
            Err(_) => 0,
        };
        // Whether the children go back one behind the walk or all at once when
        // it closes, by the rule the kind totals use: a list `walk.rs` already
        // walks with its middle dropped, and a node whose children are placed
        // one per offset and so do not need each other. Dropping the elements
        // of a short list one behind would only have the list place each of
        // them twice, since it re-reads from the first one it no longer has.
        let guarded = match &resolved {
            Ty::Repeat { .. } | Ty::PointerList { .. } | Ty::Chain { .. } | Ty::Gather { .. } | Ty::At { .. } => true,
            Ty::Array { .. } => children > super::walk::GUARD_ABOVE as u64,
            _ => false,
        };
        Ok(Seen { space, declared, resolved, run, children, guarded })
    }

    /// Write down one node that has been seen, standing for `weight` nodes of
    /// the file, and open a frame over its children.
    fn count_seen(&mut self, walk: &mut CensusWalk, path: &[usize], row: Option<(String, usize)>, weight: u64, seen: Seen) {
        let Seen { space, declared, resolved, run, children, guarded } = seen;
        walk.walked += 1;
        if let Some((key, i)) = &row {
            walk.add_row(key, *i, weight, path, space);
        }
        // The choice this node made, for a node that is one: a switch is a box
        // of its own and its row is the case taken. Counted whether or not
        // which way can be named: counting it only when the case is known left
        // the box reading "this file has none of these" over a file that takes
        // it on every chunk.
        let choice = is_choice(&self.template, &declared);
        if choice {
            if let Some(key) = walk.key(&self.template, &declared) {
                walk.add_box(&key, weight, path, space);
                if let Some(case) = self.case_taken(&declared, &resolved) {
                    walk.add_row(&key, case, weight, path, space);
                }
            }
        }
        // Which box this node's own children are rows of. A node declared as a
        // choice *is* whatever it resolved to, whether or not the case it came
        // from could be named: its children are the fields of the shape the
        // choice picked, and standing them on the choice's rows put a PNG's
        // `width` on the row that says `'IHDR'`.
        let own = match &run {
            Some(_) => None,
            None if choice => walk.key(&self.template, &resolved),
            None => walk.key(&self.template, &declared),
        };
        if let Some(key) = &own {
            walk.add_box(key, weight, path, space);
        }
        if children == 0 {
            return;
        }
        // A run of elements that are all the same shape: the first one is
        // walked and stands for every one of them. Only where the node's
        // children are the elements themselves, which is a list and not
        // something wrapped round one: a stream holding a list has the list
        // as its child, not the list's elements.
        let same = walk.multiply && match &resolved {
            Ty::Array { elem, .. } | Ty::Repeat { elem, .. } => same_shape(&self.template, elem),
            _ => false,
        };
        let (count, weight) = match (children, same) {
            (n, true) => (1, weight.saturating_mul(n)),
            (n, false) => (n, weight),
        };
        walk.stack.push(Frame { path: path.to_vec(), own, next: 0, count, weight, guarded, born: Vec::new(), prev: None });
    }

    /// Note a child so that the frame it belongs to can give it back.
    fn note_census_child(&mut self, walk: &mut CensusWalk, top: usize, path: &[usize]) {
        let f = &mut walk.stack[top];
        if !f.guarded {
            f.born.push(path.to_vec());
            return;
        }
        if let Some(gone) = f.prev.replace(path.to_vec()) {
            self.memo.forget_node(&gone);
        }
    }

    /// Close the frame on top, giving its children back to the memo.
    fn close_census_frame(&mut self, walk: &mut CensusWalk) {
        let f = walk.stack.pop().expect("called with a frame open");
        for path in f.born {
            self.memo.forget_node(&path);
        }
        if let Some(path) = f.prev {
            self.memo.forget_node(&path);
        }
    }

    /// Which case of a switch a node took, as a row of the switch's box.
    ///
    /// The resolved type is what the case picked, so the case is the one it
    /// came from. Asked twice, cheaply, rather than once expensively:
    ///
    /// 1. **By pointer.** Resolving a switch clones the case's type, and a
    ///    `Ty::Struct` clone shares its `Arc`, so the case a node took is the
    ///    one whose structure is literally the same object. Exact, and free.
    /// 2. **By the type column's own words**, for the cases that are not
    ///    structures: `u16 le` is not `u16 be`, which is the whole of what
    ///    tells two numeric cases apart.
    ///
    /// Not by the rendering the boxes are keyed on, which was the first way
    /// this was written: that renders the whole of a resolved type per node,
    /// and a WAV of five thousand nodes took over two minutes to count.
    ///
    /// Nothing for a type that is not a switch. A switch whose default was
    /// taken answers with the last row, which is where the default is drawn.
    ///
    /// A switch in a window is still the switch: a PNG chunk's `data` is a
    /// switch with a size round it, and a node of it resolves to the case it
    /// took. Asked only of the declaration as written, every case row of that
    /// switch read as taken by no chunk at all and was drawn faded.
    fn case_taken(&self, declared: &Ty, resolved: &Ty) -> Option<usize> {
        let mut declared = declared;
        for _ in 0..16 {
            declared = match declared {
                Ty::Sized { inner, .. } | Ty::SizedBits { inner, .. } | Ty::Origin { inner } | Ty::When { inner, .. } => inner,
                Ty::Named(n) => match self.template.types.get(&**n) {
                    Some(t) => t,
                    None => break,
                },
                _ => break,
            };
        }
        let cases: Vec<&Ty> = match declared {
            Ty::Switch { cases, default, .. } => cases.iter().map(|(_, t)| t).chain(Some(&**default)).collect(),
            Ty::Match { cases, default, .. } => cases.iter().map(|(_, t)| t).chain(Some(&**default)).collect(),
            _ => return None,
        };
        if let Some(got) = super::diagram::struct_of(&self.template, resolved) {
            if let Some(i) = cases
                .iter()
                .position(|c| super::diagram::struct_of(&self.template, c).is_some_and(|c| std::sync::Arc::ptr_eq(c, got)))
            {
                return Some(i);
            }
        }
        let want = resolved.display_name();
        cases.iter().position(|c| c.display_name() == want)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::source::MemSource;
    use crate::template::{Expr as E, Ty as T};

    fn read(t: Template, bytes: &[u8]) -> (Evaluator, Document<MemSource>) {
        (Evaluator::new(t), Document::new(MemSource(bytes.to_vec())))
    }

    #[test]
    fn a_run_is_counted_once_per_element_and_the_first_is_named() {
        let elem = T::structure("Item", vec![("tag", T::u8()), ("value", T::u8())]);
        let root = T::structure(
            "root",
            vec![("count", T::u8()), ("items", T::Array { elem: Box::new(elem), count: E::field("count") })],
        );
        let t = Template::new("test", root);
        let d = crate::eval::diagram(&t);
        let item = d.types.iter().find(|b| b.name == "Item").expect("an Item box");
        let (mut ev, doc) = read(t, &[3, 1, 2, 3, 4, 5, 6]);
        let c = ev.census(&doc, 1000).expect("a census");
        let got = c.boxes.iter().find(|b| b.key == item.key).expect("Item counted");
        assert_eq!(got.count, 3, "three items");
        assert_eq!(got.first_path, vec![1, 0], "the first is the run's first element");
        // And each of its two fields, once per element.
        let tag = c.rows.iter().find(|r| r.key == item.key && r.row == 0).expect("the tag row");
        assert_eq!(tag.count, 3);
        assert_eq!(tag.first_path, vec![1, 0, 0]);
    }

    #[test]
    fn a_case_taken_inside_a_window_is_counted_on_its_row() {
        // A chunk whose body is a switch with a size round it, the way a PNG
        // chunk's data is: two chunks take the first case and one the default.
        let a = T::structure("Present", vec![("x", T::u8())]);
        let chunk = T::structure(
            "Chunk",
            vec![
                ("kind", T::u8()),
                ("body", T::sized(E::lit(1), T::switch(E::field("kind"), vec![(1, a)], T::bytes(E::lit(1))))),
            ],
        );
        let t = Template::new("test", T::repeat(chunk, Until::End));
        let d = crate::eval::diagram(&t);
        let sw = d.types.iter().find(|x| x.kind == crate::eval::BoxKind::Switch).expect("a switch box").key.clone();
        let (mut ev, doc) = read(t, &[1, 9, 2, 9, 1, 9]);
        let c = ev.census(&doc, usize::MAX).expect("a census");
        assert_eq!(c.boxes.iter().find(|b| b.key == sw).map(|b| b.count), Some(3), "three chunks made the choice");
        let (mut ev, doc) = read(ev.template().clone(), &[1, 9, 2, 9, 1, 9]);
        let c = ev.census(&doc, usize::MAX).expect("a census");
        assert_eq!(c.state, CensusState::Done);
        let taken: Vec<(usize, u64)> = {
            let mut v: Vec<(usize, u64)> = c.rows.iter().filter(|r| r.key == sw).map(|r| (r.row, r.count)).collect();
            v.sort();
            v
        };
        assert_eq!(taken, vec![(0, 2), (1, 1)]);
    }

    #[test]
    fn a_run_of_windows_is_multiplied_only_when_what_is_inside_them_is_the_same() {
        // Four records of a byte-sized window each: one around a fixed
        // record, which every element holds the same way, and one around a
        // choice, which the elements make differently. The count that walks
        // every element is what both should come to.
        let fixed = T::sized(E::lit(2), T::structure("Pair", vec![("a", T::u8()), ("b", T::u8())]));
        let present = T::structure("Present", vec![("x", T::u8())]);
        let chosen = T::structure(
            "Chunk",
            vec![("kind", T::u8()), ("body", T::sized(E::lit(1), T::switch(E::field("kind"), vec![(1, present)], T::bytes(E::lit(1)))))],
        );
        let bytes = [1, 9, 2, 9, 1, 9, 2, 9];
        for (elem, multiplied) in [(fixed, true), (chosen, false)] {
            let t = Template::new("test", T::structure("root", vec![("runs", T::array(elem, E::lit(4)))]));
            let (mut ev, doc) = read(t.clone(), &bytes);
            let fast = ev.census(&doc, usize::MAX).expect("a census");
            let mut ev = Evaluator::new(t);
            let mut walk = CensusWalk::new(doc.len_bits()).element_by_element();
            let slow = ev.census_step(&doc, &mut walk, usize::MAX).expect("a census");
            assert_eq!((fast.state, slow.state), (CensusState::Done, CensusState::Done));
            assert_eq!(fast.boxes, slow.boxes);
            assert_eq!(fast.rows, slow.rows);
            // Walked once and multiplied, or walked through.
            assert_eq!(fast.walked < slow.walked, multiplied, "{} walked against {}", fast.walked, slow.walked);
        }
    }

    #[test]
    fn a_switch_declared_by_name_is_counted_as_the_switch() {
        // A GGUF metadata value is the named type `Value`, which is a switch.
        let a = T::structure("Present", vec![("x", T::u8())]);
        let entry = T::structure("Entry", vec![("kind", T::u8()), ("value", T::Named("Value".into()))]);
        let t = Template::new("test", T::repeat(entry, Until::End))
            .with_type("Value", T::switch(E::field("kind"), vec![(1, a)], T::u8()));
        let d = crate::eval::diagram(&t);
        let Some(sw) = d.types.iter().find(|x| x.kind == crate::eval::BoxKind::Switch).map(|b| b.key.clone()) else {
            panic!("no switch box in {:?}", d.types.iter().map(|b| &b.name).collect::<Vec<_>>());
        };
        let (mut ev, doc) = read(t, &[1, 9, 2, 9, 1, 9]);
        let c = ev.census(&doc, usize::MAX).expect("a census");
        assert_eq!(c.boxes.iter().find(|b| b.key == sw).map(|b| b.count), Some(3));
        let mut taken: Vec<(usize, u64)> = c.rows.iter().filter(|r| r.key == sw).map(|r| (r.row, r.count)).collect();
        taken.sort();
        assert_eq!(taken, vec![(0, 2), (1, 1)]);
    }

    #[test]
    fn a_type_the_file_does_not_have_is_counted_nowhere() {
        let a = T::structure("Present", vec![("x", T::u8())]);
        let b = T::structure("Absent", vec![("y", T::u8())]);
        let root = T::structure(
            "root",
            vec![
                ("kind", T::u8()),
                ("body", T::switch(E::field("kind"), vec![(1, a), (2, b)], T::bytes(E::lit(0)))),
            ],
        );
        let t = Template::new("test", root);
        let d = crate::eval::diagram(&t);
        let present = d.types.iter().find(|x| x.name == "Present").expect("a Present box");
        let absent = d.types.iter().find(|x| x.name == "Absent").expect("an Absent box");
        let (mut ev, doc) = read(t, &[1, 9]);
        let c = ev.census(&doc, 1000).expect("a census");
        assert_eq!(c.boxes.iter().find(|x| x.key == present.key).map(|x| x.count), Some(1));
        assert!(c.boxes.iter().all(|x| x.key != absent.key), "a shape the file has none of is not counted");
        // The switch stands on the case it took, which is the first row.
        let sw = d.types.iter().find(|x| x.kind == crate::eval::BoxKind::Switch).expect("a switch box");
        let taken = c.rows.iter().find(|r| r.key == sw.key).expect("a case row");
        assert_eq!((taken.row, taken.count), (0, 1));
    }

    #[test]
    fn a_long_run_is_counted_in_full_without_being_walked() {
        // A thousand elements, and a walk allowed a hundred nodes. The count is
        // still a thousand, because every element of this run is the same
        // shape and the length says how many: walking them would only spend
        // the budget arriving at a number already known.
        let elem = T::structure("Sample", vec![("left", T::u8()), ("right", T::u8())]);
        let root = T::structure("root", vec![("samples", T::Array { elem: Box::new(elem), count: E::lit(1000) })]);
        let t = Template::new("test", root);
        let d = crate::eval::diagram(&t);
        let sample = d.types.iter().find(|b| b.name == "Sample").expect("a Sample box");
        let (mut ev, doc) = read(t, &vec![7u8; 2000]);
        let c = ev.census(&doc, 100).expect("a census");
        assert_eq!(c.boxes.iter().find(|b| b.key == sample.key).map(|b| b.count), Some(1000));
        assert!(c.walked <= 100, "walked {} nodes for a thousand samples", c.walked);
        assert_eq!(c.state, CensusState::Done);
        // And the first is still where it is, so a reader can be taken to it.
        assert_eq!(c.boxes.iter().find(|b| b.key == sample.key).map(|b| b.first_path.clone()), Some(vec![0, 0]));
    }

    #[test]
    fn a_chunk_of_a_cartridge_png_is_counted_as_the_shape_it_is() {
        // The PICO-8 cartridge reads a PNG's chunks, and each chunk's body is a
        // choice wrapped in a window. The node is whatever the choice picked,
        // and its fields are that shape's rows: counted as the choice instead,
        // a header's `width` stood on the row that says `'IHDR'`.
        let Some(dir) = std::env::var_os("QUBERO_SAMPLES") else { return };
        let path = std::path::Path::new(&dir).join("pico8/p8png-test.p8.png");
        let Ok(bytes) = std::fs::read(&path) else { return };
        let Some(t) = crate::formats::builtin("p8png") else { return };
        let d = crate::eval::diagram(&t);
        let (mut ev, doc) = read(t, &bytes);
        // The whole file: a count stopped part way is a floor, and depth-first
        // it can stop inside the code of the second chunk, before the rest.
        let c = ev.census(&doc, usize::MAX).expect("a census");
        assert_eq!(c.state, CensusState::Done);
        let by_key: std::collections::HashMap<&str, u64> =
            c.boxes.iter().map(|b| (b.key.as_str(), b.count)).collect();
        for name in ["IHDR", "tEXt"] {
            let Some(b) = d.types.iter().find(|b| b.name == name) else { continue };
            assert_eq!(by_key.get(b.key.as_str()).copied(), Some(1), "{name} is in this file exactly once");
        }
        // No box is counted more often than there are chunks to hold one. A
        // shape reached once per chunk is four; anything past that would be a
        // node counted twice over.
        let chunks = d
            .types
            .iter()
            .find(|b| b.name == "Chunk")
            .and_then(|b| by_key.get(b.key.as_str()).copied())
            .unwrap_or(0);
        for b in &d.types {
            let got = by_key.get(b.key.as_str()).copied().unwrap_or(0);
            assert!(got <= chunks.max(1), "{} counted {got} times in a file of {chunks} chunks", b.name);
        }
        // And the choice's own rows are its cases, counted once per chunk that
        // took one, never once per field of the shape it picked.
        let Some(sw) = d.types.iter().find(|b| b.kind == crate::eval::BoxKind::Switch && b.name.contains("type"))
        else {
            return;
        };
        let taken: u64 = c.rows.iter().filter(|r| r.key == sw.key).map(|r| r.count).sum();
        let chunks = by_key.get(sw.key.as_str()).copied().unwrap_or(0);
        assert!(taken <= chunks, "{taken} cases taken by {chunks} chunks");
    }

    #[test]
    fn a_png_is_counted_by_its_chunks() {
        let Some(dir) = std::env::var_os("QUBERO_SAMPLES") else { return };
        let path = std::path::Path::new(&dir).join("pico8/p8png-test.p8.png");
        let Ok(bytes) = std::fs::read(&path) else { return };
        let Some(t) = crate::formats::builtin("png") else { return };
        let d = crate::eval::diagram(&t);
        let (mut ev, doc) = read(t, &bytes);
        let c = ev.census(&doc, usize::MAX).expect("a census");
        assert_eq!(c.state, CensusState::Done);
        let chunk = d.types.iter().find(|b| b.name == "Chunk").expect("a Chunk box");
        let ihdr = d.types.iter().find(|b| b.name == "IHDR").expect("an IHDR box");
        let chunks = c.boxes.iter().find(|b| b.key == chunk.key).map(|b| b.count).unwrap_or(0);
        assert!(chunks >= 3, "a PNG has at least a header, some data and an end: {chunks}");
        assert_eq!(c.boxes.iter().find(|b| b.key == ihdr.key).map(|b| b.count), Some(1), "one header");
    }

    /// A record whose length its own first byte gives, so no two records of a
    /// run need be the same shape and the run is walked one by one.
    fn record() -> T {
        T::structure("Record", vec![("len", T::u8()), ("body", T::bytes(E::field("len")))])
    }

    /// `n` records of one byte of body each, in a run only walking settles the
    /// length of.
    fn records(n: usize) -> (Template, Vec<u8>) {
        let t = Template::new("test", T::repeat(record(), Until::End));
        let bytes = (0..n).flat_map(|i| [1u8, i as u8]).collect();
        (t, bytes)
    }

    fn count_of(c: &Census, d: &crate::eval::Diagram, name: &str) -> Option<u64> {
        let b = d.types.iter().find(|b| b.name == name)?;
        c.boxes.iter().find(|x| x.key == b.key).map(|x| x.count)
    }

    #[test]
    fn a_run_only_walking_settles_is_counted_to_its_end() {
        // Fifty records. The walk once looked inside the first 32 of a run
        // whose length it could not know, and badged a journal's object box
        // with 32 over a file of 156.
        let (t, bytes) = records(50);
        let d = crate::eval::diagram(&t);
        let (mut ev, doc) = read(t, &bytes);
        let c = ev.census(&doc, usize::MAX).expect("a census");
        assert_eq!(c.state, CensusState::Done);
        assert_eq!(count_of(&c, &d, "Record"), Some(50));
    }

    #[test]
    fn a_list_that_ends_early_or_holds_only_numbers_finishes() {
        // The two ways a finished count of a small JPEG called itself
        // unfinished: a run of two tables asked for 32 and marked short
        // before finding it held two, and 64 bytes of a quantisation table,
        // which hold no box and no row, walked 32 of.
        let table = T::structure("Table", vec![("id", T::u8()), ("values", T::array(T::u8(), E::lit(64)))]);
        let t = Template::new("test", T::structure("root", vec![("tables", T::repeat(table, Until::End))]));
        let d = crate::eval::diagram(&t);
        let bytes: Vec<u8> = (0..2).flat_map(|i| std::iter::once(i).chain(std::iter::repeat_n(7, 64))).collect();
        let (mut ev, doc) = read(t, &bytes);
        let c = ev.census(&doc, 20_000).expect("a census");
        assert_eq!(c.state, CensusState::Done, "walked {}", c.walked);
        assert_eq!(count_of(&c, &d, "Table"), Some(2));
    }

    #[test]
    fn a_same_shaped_run_counts_its_rows_once_per_element() {
        // Every element of a run of same-shaped records has the same fields,
        // so walking the first and counting it a thousand times is exact for
        // its rows as well as its box.
        let elem = T::structure("Sample", vec![("left", T::u8()), ("right", T::u8())]);
        let root = T::structure("root", vec![("samples", T::Array { elem: Box::new(elem), count: E::lit(1000) })]);
        let t = Template::new("test", root);
        let d = crate::eval::diagram(&t);
        let sample = d.types.iter().find(|b| b.name == "Sample").expect("a Sample box").key.clone();
        let (mut ev, doc) = read(t, &vec![7u8; 2000]);
        let c = ev.census(&doc, usize::MAX).expect("a census");
        assert_eq!(c.state, CensusState::Done);
        let rows: Vec<u64> = (0..2).map(|i| c.rows.iter().find(|r| r.key == sample && r.row == i).map_or(0, |r| r.count)).collect();
        assert_eq!(rows, vec![1000, 1000]);
    }

    #[test]
    fn a_go_that_runs_out_says_working_and_the_next_carries_on() {
        let (t, bytes) = records(400);
        let (mut whole, doc) = read(t.clone(), &bytes);
        let expected = whole.census(&doc, usize::MAX).expect("a census");

        let mut ev = Evaluator::new(t);
        ev.set_slice(Some(50));
        let mut walk = CensusWalk::new(doc.len_bits());
        let mut goes = 0;
        let got = loop {
            goes += 1;
            assert!(goes < 1000, "never finished");
            ev.begin_slice();
            let c = ev.census_step(&doc, &mut walk, usize::MAX).expect("a step");
            match c.state {
                CensusState::Working => continue,
                _ => break c,
            }
        };
        assert!(goes > 1, "one go of 50 counted 400 records");
        assert_eq!(got.state, CensusState::Done);
        // Resuming counts nothing twice and misses nothing.
        assert_eq!(got.boxes, expected.boxes);
        assert_eq!(got.rows, expected.rows);
        assert_eq!(got.walked, expected.walked);
    }

    #[test]
    fn a_count_that_reaches_its_limit_says_capped_and_a_higher_limit_carries_on() {
        let (t, bytes) = records(100);
        let d = crate::eval::diagram(&t);
        let (mut whole, doc) = read(t.clone(), &bytes);
        let expected = whole.census(&doc, usize::MAX).expect("a census");

        let mut ev = Evaluator::new(t);
        let mut walk = CensusWalk::new(doc.len_bits());
        let capped = ev.census_step(&doc, &mut walk, 30).expect("a step");
        assert_eq!(capped.state, CensusState::Capped);
        assert_eq!(capped.walked, 30);
        assert!(count_of(&capped, &d, "Record").is_some_and(|n| n < 100));
        // Asked again with the same limit, it stays where it is.
        assert_eq!(ev.census_step(&doc, &mut walk, 30).expect("a step"), capped);
        // And with more room it finishes, with the counts of a walk that was
        // never stopped.
        let rest = ev.census_step(&doc, &mut walk, usize::MAX).expect("a step");
        assert_eq!(rest.state, CensusState::Done);
        assert_eq!(rest.boxes, expected.boxes);
        assert_eq!(rest.rows, expected.rows);
    }

    #[test]
    fn a_count_waiting_on_bytes_says_waiting_and_names_them() {
        use crate::source::ChunkStore;
        let (t, bytes) = records(8);
        let d = crate::eval::diagram(&t);
        // Sixteen bytes in two chunks of eight, and only the first is here.
        let mut doc = Document::new(ChunkStore::new(16, 8, 4));
        doc.source_mut().insert(0, bytes[..8].to_vec().into_boxed_slice());
        let mut ev = Evaluator::new(t);
        let mut walk = CensusWalk::new(doc.len_bits());
        ev.begin_slice();
        let first = ev.census_step(&doc, &mut walk, usize::MAX).expect("a step");
        assert_eq!(first.state, CensusState::Waiting);
        assert!(ev.wanted().iter().any(|m| m.chunk == 1), "asked for {:?}", ev.wanted());
        doc.source_mut().insert(1, bytes[8..].to_vec().into_boxed_slice());
        ev.begin_slice();
        let done = ev.census_step(&doc, &mut walk, usize::MAX).expect("a step");
        assert_eq!(done.state, CensusState::Done);
        assert_eq!(count_of(&done, &d, "Record"), Some(8));
    }

    #[test]
    fn a_finished_count_gives_back_what_it_walked() {
        // Ten thousand records and every one of their fields. Kept, that is
        // thirty thousand nodes the memo holds for a count nobody reads again.
        let (t, bytes) = records(10_000);
        let (mut ev, doc) = read(t, &bytes);
        let c = ev.census(&doc, usize::MAX).expect("a census");
        assert_eq!(c.state, CensusState::Done);
        assert!(c.walked > 30_000, "walked {}", c.walked);
        assert!(ev.memo_len() < 1_000, "{} nodes kept after the count", ev.memo_len());
    }
}
