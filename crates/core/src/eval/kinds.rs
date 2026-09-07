//! Where a file's bytes went, totalled by kind and type over the whole of it.
//!
//! The listing answers this a screen at a time: `spans` says what covers each
//! stretch, and a view adding those up learns what a screenful is made of. A
//! treemap asks the same question of the whole file at once, which `spans`
//! cannot answer: it resolves every field it reports, and a file whose tensor
//! table holds two million entries is two million nodes before the first
//! rectangle is drawn.
//!
//! So this walks the tree itself, in goes small enough that the page stays
//! usable between them, and keeps a total per (kind, type) pair rather than a
//! node per field. Two things make that affordable:
//!
//! - **A run of same-sized elements is walked once and multiplied.** A grid of
//!   a million floats, or a database of a million pages, costs one element's
//!   walk times a count. `scale` on the frame is how: everything under a
//!   scaled frame is added that many times over, so the multiplication costs
//!   nothing and needs no second set of totals to hold the element's own
//!   answer in.
//! - **What the walk has gone past is dropped.** A frame gives its children
//!   back to the memo when it closes, and an element of a long list is dropped
//!   one element behind the walk, which is the window `walk.rs` keeps and for
//!   the same reason: a million elements remembered is more memory than the
//!   file.
//!
//! What the totals mean, precisely:
//!
//! - A **leaf** contributes all of its bits. That includes a compressed run,
//!   whose contents are bits of another address space entirely and so are
//!   never walked from here, and a decoder's trace, whose symbols are a second
//!   reading of bits the run has already accounted for.
//! - A **structure or a list** contributes only what its children leave over,
//!   and only when the type says those leftovers are its own punctuation --
//!   the braces of a JSON object, the brackets of an array, which is what
//!   `framed` marks. Anything else a composite fails to cover is a gap. Either
//!   way a composite's bits are never added to its children's, so nothing is
//!   counted twice.
//! - **Unmapped** is bits inside the walked region that no field covers: the
//!   slack at the end of a structure, padding between records, and the tail of
//!   a file whose template describes only its header.
//! - What has **not been walked yet** is simply absent. There is no bucket for
//!   it here; the caller has `reached_bits` and the file's length and can
//!   subtract.

use rustc_hash::FxHashMap;

use super::*;

/// One (kind, type) pair and what the file spends on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindTotal {
    /// The word the listing gives a field of this type: `uint`, `str`,
    /// `magic`, `composite`. See [`super::value_kind`].
    pub kind: &'static str,
    /// The type as the type column writes it: `u16 le`, `utf8[]`, `ZipRecord`.
    pub type_name: String,
    pub bits: u64,
    pub count: u64,
}

/// What the walk has found so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindTotals {
    /// True once the whole file has been accounted for. Until then, ask again.
    pub done: bool,
    /// How far into the file the walk has reached, at its furthest. What is
    /// past this has not been looked at and appears in none of the numbers
    /// below, so the caller derives what is left as the file's length less
    /// this.
    pub reached_bits: u64,
    /// Bits some field covers, which is the sum of `totals`.
    pub covered_bits: u64,
    /// Bits inside the reached region that no field covers.
    pub unmapped_bits: u64,
    /// Biggest first, then by type, so two runs over the same file answer the
    /// same way and a view need not sort what it is handed.
    pub totals: Vec<KindTotal>,
}

/// One node the walk is inside, and how far through its children it has got.
struct Frame {
    path: Vec<usize>,
    /// Where the node ends. For the root this is the end of the file rather
    /// than the end of what the root covers: a template that describes a
    /// header and stops has the rest of the file to account for, and saying so
    /// is most of the point of the question.
    end: u64,
    /// Where the node starts, which the uniform shortcut needs to place the
    /// end of a run without walking to it.
    offset: u64,
    /// Where the children have got to. Bits between this and the next child's
    /// start are a gap.
    cursor: u64,
    /// How many children there are, or `u64::MAX` for a run whose length only
    /// walking it settles.
    count: u64,
    next: u64,
    /// How many times over the file holds this subtree, from the runs of
    /// same-sized elements above it. Never zero.
    scale: u64,
    /// Whether the bits the children leave over are the node's own punctuation
    /// rather than bytes nothing describes.
    framed: bool,
    /// What those leftovers are counted as when they are.
    type_name: String,
    /// Whether the children are placed one after another. The children of a
    /// pointer list, a chain or an `At` are wherever an offset read from the
    /// file put them, so they move no cursor: letting them would call
    /// everything they skipped over a gap and then count the siblings that
    /// fill it as well.
    sequential: bool,
    /// Children of this node that place *their* children by offset, kept back
    /// until the ones laid out in order are done.
    ///
    /// A format sometimes describes one stretch twice. npy writes its dtype as
    /// text and then, over the same text, a record view of the fields it
    /// names; xz lists its header, blocks, index and footer and then points at
    /// the whole stream to say what it unpacks to. Both are answers to "what
    /// does this come to" rather than claims on bytes, and counting them would
    /// say the file is bigger than it is. What tells them from a field the
    /// format keeps elsewhere -- an AppleDouble's data fork, an ELF section --
    /// is whether they land inside what the fields laid out in order already
    /// cover, and that is only known once those fields are done. So they wait,
    /// and are then judged against the cursor's final resting place.
    deferred: Vec<u64>,
    /// How many of `deferred` have been taken.
    taking: usize,
    /// The stretch the run that placed this node had already covered, for a
    /// node whose children an offset puts somewhere. A child that lands wholly
    /// inside it is a second view of bytes that run has described, and is
    /// passed over. `(0, 0)` for everything else, which lets nothing through.
    already: (u64, u64),
    /// Direct children this frame has resolved, to give back when it closes.
    /// Empty for a guarded list, which drops as it goes instead.
    born: Vec<Vec<usize>>,
    /// The element before the one being placed, for a guarded list. Placing an
    /// element asks the one before it where it ends, so the walk lets go one
    /// element behind itself; dropping any sooner would have every element
    /// sized twice.
    prev: Option<Vec<usize>>,
    /// True when this is a list long enough that `walk.rs` drops its middle,
    /// which is also when this can afford to.
    guarded: bool,
    /// The stride of a run whose elements are all the same shape as well as
    /// the same size, so element 0 stands for all of them. See
    /// [`Evaluator::exact_stride`].
    uniform: Option<u64>,
}

/// What opening a composite turned out to say about it, before the walk has
/// written anything down. Kept apart from [`Frame`] so that the questions
/// reading the file can interrupt are all asked first: see
/// [`Evaluator::opening`].
struct Opening {
    path: Vec<usize>,
    count: u64,
    guarded: bool,
    uniform: Option<u64>,
}

impl Opening {
    fn frame(self, r: &Resolved, scale: u64, end: u64, already: (u64, u64)) -> Frame {
        Frame {
            path: self.path,
            end,
            offset: r.offset,
            cursor: r.offset,
            count: self.count,
            next: 0,
            scale,
            // The rule the node itself carries: the braces of a JSON object
            // and the brackets of an array are the node's own, and its members
            // tile what is between them. Everything else that fails to cover
            // itself has left a gap.
            framed: matches!(&r.ty, Ty::Json(shape, _) if shape.composite()) && self.count > 0,
            type_name: r.ty.display_name(),
            sequential: !places(&r.ty),
            deferred: Vec::new(),
            taking: 0,
            already,
            born: Vec::new(),
            prev: None,
            guarded: self.guarded,
            uniform: self.uniform,
        }
    }
}

/// Whether this node's children are wherever an offset read from the file put
/// them, rather than one after another.
fn places(ty: &Ty) -> bool {
    matches!(ty, Ty::PointerList { .. } | Ty::Chain { .. } | Ty::At { .. })
}

/// The walk, kept between goes. Held beside the evaluator rather than inside
/// it, the way the byte-class scan is held beside the document: it is one
/// question someone asked, not something the reading needs to know.
///
/// Thrown away whenever the document changes, for the same reason the scan is:
/// every offset in it describes bytes that may no longer be there.
pub struct KindWalk {
    stack: Vec<Frame>,
    totals: FxHashMap<(&'static str, String), (u64, u64)>,
    covered_bits: u64,
    unmapped_bits: u64,
    /// The furthest bit anything accounted for reaches. Kept here rather than
    /// taken from [`Evaluator::reached_bits`], which counts what was *read*: a
    /// run of a million same-sized records is accounted for without reading
    /// any of it past the first, and a progress line drawn from what was read
    /// would sit at nothing while the walk finished.
    reached_bits: u64,
    /// True once the root has been placed, which is what tells a walk that has
    /// not begun from one that has finished. Both have an empty stack.
    started: bool,
    done: bool,
    /// The file's length in bits, which is where the root frame ends.
    file_bits: u64,
}

impl KindWalk {
    pub fn new(file_bits: u64) -> KindWalk {
        KindWalk {
            stack: Vec::new(),
            totals: FxHashMap::default(),
            covered_bits: 0,
            unmapped_bits: 0,
            reached_bits: 0,
            started: false,
            done: false,
            file_bits,
        }
    }

    /// The length this walk was built for, so a caller can tell a walk that is
    /// still about the document in hand from one that is not.
    pub fn file_bits(&self) -> u64 {
        self.file_bits
    }

    pub fn done(&self) -> bool {
        self.done
    }

    /// What has been found so far, ready to hand over.
    pub fn totals(&self) -> KindTotals {
        let mut totals: Vec<KindTotal> = self
            .totals
            .iter()
            .map(|((kind, type_name), (bits, count))| KindTotal {
                kind,
                type_name: type_name.clone(),
                bits: *bits,
                count: *count,
            })
            .collect();
        totals.sort_by(|a, b| b.bits.cmp(&a.bits).then_with(|| a.type_name.cmp(&b.type_name)).then(a.kind.cmp(b.kind)));
        KindTotals {
            done: self.done,
            reached_bits: self.reached_bits,
            covered_bits: self.covered_bits,
            // A field placed by address covers bits outside whatever structure
            // declared it, so one stretch of the file can be counted both as
            // covered, by that field, and as a gap in the run of fields it
            // fell in the middle of. The totals are what they are; the gap is
            // held down to what is left over, so a view drawing covered and
            // unmapped side by side never draws more than has been reached.
            unmapped_bits: self.unmapped_bits.min(self.reached_bits.saturating_sub(self.covered_bits)),
            totals,
        }
    }

    fn add(&mut self, kind: &'static str, type_name: String, bits: u64, count: u64) {
        let e = self.totals.entry((kind, type_name)).or_insert((0, 0));
        e.0 = e.0.saturating_add(bits);
        e.1 = e.1.saturating_add(count);
        self.covered_bits = self.covered_bits.saturating_add(bits);
    }

    fn gap(&mut self, bits: u64) {
        self.unmapped_bits = self.unmapped_bits.saturating_add(bits);
    }

    fn reach(&mut self, bit: u64) {
        self.reached_bits = self.reached_bits.max(bit.min(self.file_bits));
    }
}

impl Evaluator {
    /// Carry the walk on for one go, and say what it has found.
    ///
    /// The go ends when the evaluator's allowance runs out, which is the
    /// mechanism every other stepped reading here uses: the caller refills it
    /// with `begin_slice` and asks again. `done` on the answer says when to
    /// stop asking. Bytes that have not arrived come back as `Pending` and are
    /// the caller's to fetch, exactly as they are for a node.
    ///
    /// Interrupted part-way through a field, the walk redoes that one field on
    /// the next go: a frame does not step past a child until the child is
    /// fully accounted for, so nothing is counted twice by resuming.
    pub fn kind_totals_step<S: Source>(&mut self, doc: &Document<S>, walk: &mut KindWalk) -> R<KindTotals> {
        match self.kind_totals_run(doc, walk) {
            Ok(()) | Err(EvalError::Busy { .. }) => Ok(walk.totals()),
            Err(e) => Err(e),
        }
    }

    /// The walk itself, one child of one frame per turn.
    fn kind_totals_run<S: Source>(&mut self, doc: &Document<S>, walk: &mut KindWalk) -> R<()> {
        if walk.done {
            return Ok(());
        }
        if !walk.started {
            // Only once the root is placed, and not before: sizing the root of
            // a file that is one long run is a walk to the end of that run,
            // and it can run out of the allowance. Marked started too early,
            // the next go would find an empty stack and call a walk that had
            // done nothing finished.
            self.open_root(doc, walk)?;
            walk.started = true;
        }
        loop {
            let Some(f) = walk.stack.last() else {
                walk.done = true;
                return Ok(());
            };
            // A run whose length only walking it settles stops when its room
            // runs out; everything else stops when its children do. The ones
            // held back for later come after all of them: see `deferred`.
            let in_order = f.next < f.count && !(f.count == u64::MAX && f.cursor >= f.end);
            let ends = !in_order && f.taking >= f.deferred.len();
            let at = f.cursor;
            if ends {
                self.close_frame(walk);
                continue;
            }
            // One element of the allowance per child, which is what gives the
            // page a turn part-way through a long run rather than freezing it.
            self.spend(at)?;
            self.one_child(doc, walk, in_order)?;
        }
    }

    /// Place the root, and open a frame over it when it holds anything.
    fn open_root<S: Source>(&mut self, doc: &Document<S>, walk: &mut KindWalk) -> R<()> {
        self.resolve(doc, &[])?;
        let size = self.size_of(doc, &[])?;
        let r = self.memo[&Vec::new()].clone();
        if !descends(&r.ty) {
            // A template that is one field, with the rest of the file left
            // over. Answered here rather than given a case in the loop below.
            let kind = super::value_kind(&self.template, &r.ty);
            walk.add(kind, r.ty.display_name(), size, 1);
            walk.gap(walk.file_bits.saturating_sub(r.offset + size));
            walk.reach(walk.file_bits);
            walk.done = true;
            return Ok(());
        }
        let opening = self.opening(doc, &[], &r)?;
        walk.stack.push(opening.frame(&r, 1, walk.file_bits, (0, 0)));
        Ok(())
    }

    /// Account for the next child of the frame on top, and open a frame over
    /// it when it holds anything.
    fn one_child<S: Source>(&mut self, doc: &Document<S>, walk: &mut KindWalk, in_order: bool) -> R<()> {
        let top = walk.stack.len() - 1;
        let (path, idx, scale, sequential) = {
            let f = &walk.stack[top];
            let i = if in_order { f.next } else { f.deferred[f.taking] };
            let mut p = f.path.clone();
            p.push(i as usize);
            // A child taken from the queue is one that puts its own children
            // somewhere: it is not the next thing along, so it moves no cursor
            // and leaves no gap behind it.
            (p, i, f.scale, f.sequential && in_order)
        };
        let sized = self.resolve(doc, &path).and_then(|()| self.size_of(doc, &path));
        let size = match sized {
            Ok(size) => size,
            Err(e) if e.interrupted() => return Err(e),
            // A field that will not read ends the run it is in rather than the
            // walk: a structure places its fields one after another, so
            // nothing after the one that failed can be placed either. What is
            // left of the parent is a stretch the template could not read, and
            // reads here as a gap, which is what the annotation column says
            // about the same bytes. Fields it had put aside are still reached:
            // those are placed by an offset and do not depend on this one.
            Err(_) => {
                walk.stack[top].next = walk.stack[top].count;
                return Ok(());
            }
        };
        let r = self.memo[path.as_slice()].clone();
        // Nothing in another address space belongs to this file: it is a
        // reading of bytes unpacked from it, and its offsets count from the
        // front of those. Reached only if a stream were descended into, which
        // `descends` refuses; kept here so that it stays refused.
        if r.space != 0 {
            self.step_past(walk, top, in_order);
            return Ok(());
        }
        // A field that puts its own children somewhere waits until the ones
        // laid out in order are done, so that where they landed can be judged
        // against what those cover. See `deferred`.
        if in_order && places(&r.ty) {
            let f = &mut walk.stack[top];
            f.deferred.push(idx);
            f.next += 1;
            return Ok(());
        }
        // A second view of bytes the run that placed this has already
        // described. See `already`.
        let already = walk.stack[top].already;
        if already.1 > already.0 && r.offset >= already.0 && r.offset + size <= already.1 {
            self.note_born(walk, top, &path);
            self.step_past(walk, top, in_order);
            return Ok(());
        }
        // Everything that can want bytes or run out of the allowance happens
        // before anything is written down. Counting a run that ends on what it
        // reads is a walk of its own and either can happen in the middle of
        // it; interrupted after the frame had been stepped past its child, the
        // next go would skip that child and its bits would be in none of the
        // answers.
        let opening = if descends(&r.ty) { Some(self.opening(doc, &path, &r)?) } else { None };
        if sequential {
            let cursor = walk.stack[top].cursor;
            if r.offset > cursor {
                walk.gap((r.offset - cursor).saturating_mul(scale));
            }
            let now = cursor.max(r.offset + size);
            walk.stack[top].cursor = now;
            walk.reach(now);
            // A run whose length only the walk settles ends here if this
            // element covered nothing: every element after it starts where
            // this one did, so the run would ask the same question of the same
            // offset for ever. `count_from` refuses the same thing.
            if walk.stack[top].count == u64::MAX && now == cursor {
                walk.stack[top].next = u64::MAX;
                return Ok(());
            }
        } else {
            walk.reach(r.offset + size);
        }
        // A run of same-sized elements of the same shape: element 0 is walked
        // and counted as many times as there are elements, and the rest of the
        // run is never touched. This is the difference between opening a file
        // of a million pages and reading one.
        let scale = match walk.stack[top].uniform {
            Some(stride) if idx == 0 => {
                let f = &mut walk.stack[top];
                let n = f.count;
                let end = f.offset.saturating_add(n.saturating_mul(stride));
                f.next = n;
                f.cursor = f.cursor.max(end);
                // The whole run is accounted for the moment element 0 is, so
                // the progress line has to say so before the totals do.
                // Otherwise coverage would run ahead of what the caller thinks
                // has been reached and "not yet walked" would go negative.
                walk.reach(end);
                scale.saturating_mul(n).max(1)
            }
            _ => {
                self.step_past(walk, top, in_order);
                scale
            }
        };
        self.note_born(walk, top, &path);
        let Some(opening) = opening else {
            let kind = super::value_kind(&self.template, &r.ty);
            walk.add(kind, r.ty.display_name(), size.saturating_mul(scale), scale);
            return Ok(());
        };
        // What the run that placed this has already covered, for judging where
        // its children land. Only a field held back until the end has one: by
        // then the cursor has stopped moving, and anything inside it is a
        // second view rather than more bytes.
        let already = if in_order { (0, 0) } else { (walk.stack[top].offset, walk.stack[top].cursor) };
        walk.stack.push(opening.frame(&r, scale, r.offset + size, already));
        Ok(())
    }

    /// Step the frame past the child just dealt with, whichever queue it came
    /// from.
    fn step_past(&mut self, walk: &mut KindWalk, top: usize, in_order: bool) {
        let f = &mut walk.stack[top];
        if in_order {
            f.next += 1;
        } else {
            f.taking += 1;
        }
    }

    /// Note a child so that the frame it belongs to can give it back.
    ///
    /// A long list gives its elements back as it goes instead, one behind the
    /// walk, which is what keeps a run of a million from being a million
    /// nodes; see `prev` on the frame.
    fn note_born(&mut self, walk: &mut KindWalk, top: usize, path: &[usize]) {
        let f = &mut walk.stack[top];
        if !f.guarded {
            f.born.push(path.to_vec());
            return;
        }
        if let Some(gone) = f.prev.replace(path.to_vec()) {
            self.memo.forget_node(&gone);
        }
    }

    /// Everything about opening a node that reading the file could interrupt:
    /// how many children it has, whether it is long enough to be walked with
    /// its middle dropped, and whether its elements are alike enough to be
    /// counted once and multiplied. Asked before the walk writes anything
    /// down, so that an interruption leaves it where it was.
    fn opening<S: Source>(&mut self, doc: &Document<S>, path: &[usize], r: &Resolved) -> R<Opening> {
        let count = match self.count_unless_walk(doc, path) {
            Ok(Some(n)) => n,
            // A run that fills its container with elements whose length their
            // own bytes give: counting it means decoding all of it, which is
            // what this walk is about to do anyway. It stops when the room
            // runs out or an element will not read.
            Ok(None) => u64::MAX,
            Err(e) if e.interrupted() => return Err(e),
            Err(_) => 0,
        };
        // Dropped as the walk goes past, either because `walk.rs` already
        // treats this list that way, or because its children are placed one
        // per offset and so do not need each other: a zip's central directory
        // is sixty thousand entries and remembering all of them to say what
        // they came to is the memory this walk exists to not spend.
        let guarded = self.guarded(doc, path, r)? || places(&r.ty);
        let uniform = if count > 1 && count != u64::MAX { self.exact_stride(doc, path, &r.ty)? } else { None };
        Ok(Opening { path: path.to_vec(), count, guarded, uniform })
    }

    /// Close the frame on top: account for what its children left over, and
    /// give the children back.
    fn close_frame(&mut self, walk: &mut KindWalk) {
        let f = walk.stack.pop().expect("called with a frame open");
        // A node whose children are wherever an offset put them tiles nothing
        // and so leaves nothing over. The stretch it was declared across is
        // the enclosing run's to account for; calling it a gap here would
        // count the same bytes as missing and as covered at once.
        let leftover = if f.sequential { f.end.saturating_sub(f.cursor) } else { 0 };
        if leftover > 0 {
            let bits = leftover.saturating_mul(f.scale);
            if f.framed {
                walk.add("composite", f.type_name.clone(), bits, f.scale);
            } else {
                walk.gap(bits);
            }
        }
        walk.reach(f.end);
        for path in f.born {
            self.memo.forget_node(&path);
        }
        if let Some(path) = f.prev {
            self.memo.forget_node(&path);
        }
    }

    /// The stride of a run whose elements are not only the same size but the
    /// same shape, so that walking one and multiplying says what walking all
    /// of them would.
    ///
    /// Not [`Evaluator::stride`] itself, which is a weaker claim: it answers
    /// for a run of `Sized` elements whose *window* is uniform, which is how a
    /// database of 4 KiB pages is written. Every page is 4 KiB and no two
    /// pages hold the same fields, so multiplying the first page's breakdown
    /// by two hundred thousand would invent a split the file does not have.
    /// What holds is either that the element is a fixed number of bits, which
    /// pins every field in it, or that it holds one value and nothing inside
    /// it, in which case there is nothing to get wrong.
    fn exact_stride<S: Source>(&mut self, doc: &Document<S>, path: &[usize], ty: &Ty) -> R<Option<u64>> {
        let elem = match ty {
            Ty::Array { elem, .. } | Ty::Repeat { elem, until: Until::End } => (**elem).clone(),
            _ => return Ok(None),
        };
        let Some(stride) = self.stride(doc, path, ty)? else { return Ok(None) };
        if stride == 0 {
            return Ok(None);
        }
        if fixed_bits(&elem).is_some() {
            return Ok(Some(stride));
        }
        // A run of numbers packed to a width the header named, or of text, or
        // of raw bytes: one value each, so every element is the same kind and
        // the same type however wide the file made it.
        let mut settled = elem.base();
        for _ in 0..8 {
            let Ty::Named(n) = settled else { break };
            match self.template.types.get(&**n) {
                Some(t) => settled = t.base(),
                None => return Ok(None),
            }
        }
        Ok(super::listing::plain(settled).then_some(stride))
    }
}

/// Whether the walk goes inside this type.
///
/// Everything that holds fields of this file does. The two that hold fields
/// which are not of this file do not: a compressed run, whose contents are at
/// offsets of the bytes it unpacked to and are no part of the file's layout,
/// and a decoder's trace, whose blocks and symbols are a second reading of
/// bits the run has already accounted for. Both are leaves here and contribute
/// every bit they cover, which is the honest answer: a gzip is as many bytes
/// of the file as it is, whatever is inside it.
fn descends(ty: &Ty) -> bool {
    match ty {
        Ty::Struct(_)
        | Ty::Array { .. }
        | Ty::Repeat { .. }
        | Ty::PointerList { .. }
        | Ty::Chain { .. }
        | Ty::At { .. } => true,
        Ty::Json(shape, _) => shape.composite(),
        _ => false,
    }
}
