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

use rustc_hash::{FxHashMap, FxHashSet};

use super::size::same_shape;
use super::watch::{Closing, Watch};
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
    /// Whether this is an `At`, whose one child is whatever its address names.
    /// See [`KindWalk::reached_by_address`].
    at: bool,
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
    /// For a node that is a region of its own and whose children an offset
    /// puts somewhere inside it, which is a gather a `Sized` gave a length: how
    /// many of its bits its children have covered. A FITS heap is `PCOUNT`
    /// bytes and what no array claims is a gap, but its arrays move no cursor,
    /// so what they leave over is the region less what they came to. None for
    /// every other node.
    tiled: Option<u64>,
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
            at: matches!(r.ty, Ty::At { .. }),
            deferred: Vec::new(),
            taking: 0,
            already,
            born: Vec::new(),
            prev: None,
            guarded: self.guarded,
            uniform: self.uniform,
            tiled: region(r).then_some(0),
        }
    }
}

/// Whether this node's children are wherever an offset read from the file put
/// them, rather than one after another.
fn places(ty: &Ty) -> bool {
    matches!(ty, Ty::PointerList { .. } | Ty::Chain { .. } | Ty::Gather { .. } | Ty::At { .. })
}

/// Whether this node places its children by offset inside a region of its own,
/// which is a gather a `Sized` gave a length. Such a node is laid out in order
/// like any other field, since it covers its region where it is declared, and
/// it is only its children that are scattered.
fn region(r: &Resolved) -> bool {
    matches!(r.ty, Ty::Gather { .. }) && r.declared_size.is_some()
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
    /// Where each thing an `At` reached starts and how many bits it covers,
    /// so that a second address naming the same thing counts nothing.
    ///
    /// A file can be a graph rather than a tree. An HDF5 group links to an
    /// object by its address, and nothing stops two thousand links naming one
    /// dataset; the template follows each of them, since each is a way a
    /// reader gets there, and counted each time that one object header would
    /// be two thousand stretches of the file, which a test over the sample
    /// collection caught as more bits covered than the file has. Nothing in
    /// the template can mark one of those links the real one and the rest
    /// second readings, because none of them is: `Field::aside` is for a
    /// field that is always a view of somewhere else, and a link is the only
    /// reading of what it names until another link names it too.
    ///
    /// Only what an `At` reached, and only outside a multiplied run. A pointer
    /// list or a chain names many things, one each, and a set holding every
    /// chunk of a large dataset is memory this walk is meant not to spend. The
    /// same thing reached by exactly the same start and length is the case a
    /// graph makes; anything that overlaps some other way is left as it was.
    reached_by_address: FxHashSet<(u64, u64)>,
    /// Every stretch a node a parse placed has already been counted over,
    /// sorted by where it starts. See [`KindWalk::count_placed`].
    placed_runs: Vec<(u64, u64)>,
    /// True once the root has been placed, which is what tells a walk that has
    /// not begun from one that has finished. Both have an empty stack.
    started: bool,
    done: bool,
    /// The file's length in bits, which is where the root frame ends.
    file_bits: u64,
    /// Where the walk starts: the file's root, or the contents of a stream
    /// opened as a tab, read where the stream was declared. See
    /// [`Tab`](super::Tab).
    root: Vec<usize>,
    /// The space the root's fields are counted in, once the root is placed.
    /// Nothing in any other belongs to what is being totalled.
    space: SpaceId,
}

impl KindWalk {
    pub fn new(file_bits: u64) -> KindWalk {
        KindWalk::under(file_bits, Vec::new())
    }

    /// A walk over the fields under `root`, which hold `file_bits` bits of
    /// the space they are counted in.
    pub fn under(file_bits: u64, root: Vec<usize>) -> KindWalk {
        KindWalk {
            stack: Vec::new(),
            totals: FxHashMap::default(),
            covered_bits: 0,
            unmapped_bits: 0,
            reached_bits: 0,
            reached_by_address: FxHashSet::default(),
            placed_runs: Vec::new(),
            started: false,
            done: false,
            file_bits,
            root,
            space: 0,
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

    /// Whether this is the first reading of `from..to` by a node a parse
    /// placed, and take it down when it is.
    ///
    /// Two torch tensors can be two windows onto one storage, and one of them
    /// can be the whole of it. Each is the reading its own reader asked for
    /// and neither is a view of the other, so neither can be marked a second
    /// reading in advance the way [`crate::template::Field::aside`] marks
    /// one. What is true of the file is that the bytes are numbers once, so
    /// the first run over a stretch counts it and a run overlapping that one
    /// counts nothing.
    ///
    /// Kept sorted by where each run starts, so a checkpoint of a few hundred
    /// storages costs a binary search apiece rather than a scan.
    fn count_placed(&mut self, from: u64, to: u64) -> bool {
        let i = self.placed_runs.partition_point(|(start, _)| *start <= from);
        if i > 0 && self.placed_runs[i - 1].1 > from {
            return false;
        }
        if self.placed_runs.get(i).is_some_and(|(start, _)| *start < to) {
            return false;
        }
        self.placed_runs.insert(i, (from, to));
        true
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
        match self.kind_totals_run(doc, walk, &mut ()) {
            Ok(()) | Err(EvalError::Busy { .. }) => Ok(walk.totals()),
            Err(e) => Err(e),
        }
    }

    /// The walk itself, one child of one frame per turn, with `watch` told
    /// what it counts. See [`super::watch`].
    pub(super) fn kind_totals_run<S: Source, W: Watch<S>>(&mut self, doc: &Document<S>, walk: &mut KindWalk, watch: &mut W) -> R<()> {
        if walk.done {
            return Ok(());
        }
        if !walk.started {
            // Only once the root is placed, and not before: sizing the root of
            // a file that is one long run is a walk to the end of that run,
            // and it can run out of the allowance. Marked started too early,
            // the next go would find an empty stack and call a walk that had
            // done nothing finished.
            self.open_root(doc, walk, watch)?;
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
                let f = walk.stack.last().expect("a frame is open");
                watch.closing(self, doc, &closing_of(f))?;
                self.close_frame(walk, watch);
                continue;
            }
            // One element of the allowance per child, which is what gives the
            // page a turn part-way through a long run rather than freezing it.
            self.spend(at)?;
            self.one_child(doc, walk, in_order, watch)?;
        }
    }

    /// Place the root, and open a frame over it when it holds anything.
    fn open_root<S: Source, W: Watch<S>>(&mut self, doc: &Document<S>, walk: &mut KindWalk, watch: &mut W) -> R<()> {
        let root = walk.root.clone();
        self.resolve(doc, &root)?;
        let size = self.size_of(doc, &root)?;
        let r = self.memo[&root].clone();
        walk.space = r.space;
        if !descends(&r.ty) {
            // A template that is one field, with the rest of the file left
            // over. Answered here rather than given a case in the loop below.
            watch.ready(self, doc, &root, &r)?;
            let kind = super::value_kind(&self.template, &r.ty);
            walk.add(kind, r.ty.display_name(), size, 1);
            watch.leaf(self, &root, &r, size, 1);
            walk.gap(walk.file_bits.saturating_sub(r.offset + size));
            watch.gap(&root, r.offset + size, walk.file_bits.max(r.offset + size), 1);
            walk.reach(walk.file_bits);
            walk.done = true;
            return Ok(());
        }
        let opening = self.opening(doc, &root, &r)?;
        watch.ready(self, doc, &root, &r)?;
        walk.stack.push(opening.frame(&r, 1, walk.file_bits, (0, 0)));
        watch.open(self, &root, &r, 1);
        Ok(())
    }

    /// Account for the next child of the frame on top, and open a frame over
    /// it when it holds anything.
    fn one_child<S: Source, W: Watch<S>>(&mut self, doc: &Document<S>, walk: &mut KindWalk, in_order: bool, watch: &mut W) -> R<()> {
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
            Err(e) => {
                let why = match &e {
                    EvalError::Failed(why) => why.as_str(),
                    _ => "",
                };
                let (parent, at) = (walk.stack[top].path.clone(), walk.stack[top].cursor);
                watch.failed(self, doc, &parent, idx, at, why)?;
                walk.stack[top].next = walk.stack[top].count;
                return Ok(());
            }
        };
        let r = self.memo[path.as_slice()].clone();
        // Nothing in another address space belongs to this file: it is a
        // reading of bytes unpacked from it, and its offsets count from the
        // front of those. Reached only if a stream were descended into, which
        // `descends` refuses; kept here so that it stays refused.
        if r.space != walk.space {
            self.step_past(walk, top, in_order);
            return Ok(());
        }
        // A field that puts its own children somewhere waits until the ones
        // laid out in order are done, so that where they landed can be judged
        // against what those cover. See `deferred`. So does a field a parse
        // put somewhere itself, which is a torch tensor's numbers: laid out
        // in order it would move the cursor to another entry of the archive
        // and every byte in between would read as a gap. See
        // `Resolved::elsewhere`.
        if in_order && (places(&r.ty) || r.elsewhere) && !region(&r) {
            let f = &mut walk.stack[top];
            f.deferred.push(idx);
            f.next += 1;
            return Ok(());
        }
        // A field the template says is a second reading of bytes something
        // else describes: an ELF section header reads its own name out of the
        // section that holds every name. Counting it would count those bytes
        // twice, and a total that counts one stretch twice can say more of the
        // file is text than the file is long. See `Field::aside`.
        if self.aside(&path) {
            self.note_born(walk, top, &path);
            self.step_past(walk, top, in_order);
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
        watch.ready(self, doc, &path, &r)?;
        // The same thing reached again through another address. After the
        // last question that can be interrupted and before anything is written
        // down, so a go that stops short never finds its own child already
        // noted. See `reached_by_address`.
        if walk.stack[top].at && scale == 1 && !walk.reached_by_address.insert((r.offset, size)) {
            self.note_born(walk, top, &path);
            self.step_past(walk, top, in_order);
            return Ok(());
        }
        // A run a parse placed over bytes another one has already been counted
        // over: two torch tensors onto one storage. See `count_placed`.
        if r.elsewhere && size > 0 && !walk.count_placed(r.offset, r.offset + size) {
            self.note_born(walk, top, &path);
            self.step_past(walk, top, in_order);
            return Ok(());
        }
        if sequential {
            let cursor = walk.stack[top].cursor;
            if r.offset > cursor {
                walk.gap((r.offset - cursor).saturating_mul(scale));
                watch.gap(&walk.stack[top].path, cursor, r.offset, scale);
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
            // What a region's scattered children cover, counted once for each
            // time the region itself is: only the part inside the region, so a
            // child that runs out of it cannot make the gap come out negative.
            let f = &mut walk.stack[top];
            if let Some(tiled) = f.tiled.as_mut() {
                let (from, to) = (r.offset.max(f.offset), (r.offset + size).min(f.end));
                *tiled = tiled.saturating_add(to.saturating_sub(from));
            }
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
            watch.leaf(self, &path, &r, size.saturating_mul(scale), scale);
            return Ok(());
        };
        // What the run that placed this has already covered, for judging where
        // its children land. Only a field held back until the end has one: by
        // then the cursor has stopped moving, and anything inside it is a
        // second view rather than more bytes.
        let already = if in_order { (0, 0) } else { (walk.stack[top].offset, walk.stack[top].cursor) };
        walk.stack.push(opening.frame(&r, scale, r.offset + size, already));
        watch.open(self, &path, &r, scale);
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
    fn close_frame<S: Source, W: Watch<S>>(&mut self, walk: &mut KindWalk, watch: &mut W) {
        let f = walk.stack.pop().expect("called with a frame open");
        watch.close(self, &closing_of(&f));
        // A node whose children are wherever an offset put them tiles nothing
        // and so leaves nothing over. The stretch it was declared across is
        // the enclosing run's to account for; calling it a gap here would
        // count the same bytes as missing and as covered at once.
        // A region whose children are scattered inside it leaves over what they
        // did not cover. Two children over the same bytes count those bytes
        // twice here, which can only make the gap smaller than it is, never
        // larger than the region.
        let leftover = match f.tiled {
            Some(tiled) => f.end.saturating_sub(f.offset).saturating_sub(tiled),
            None if f.sequential => f.end.saturating_sub(f.cursor),
            None => 0,
        };
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
    /// What holds is either that the element is a fixed number of bits with
    /// nothing in it that points elsewhere or is chosen as it is read, which
    /// pins every field in it (see [`same_shape`]), or that it holds one value
    /// and nothing inside it, in which case there is nothing to get wrong.
    fn exact_stride<S: Source>(&mut self, doc: &Document<S>, path: &[usize], ty: &Ty) -> R<Option<u64>> {
        let elem = match ty {
            Ty::Array { elem, .. } | Ty::Repeat { elem, until: Until::End } => elem,
            _ => return Ok(None),
        };
        // A record written as the name of its type is that type, the same way
        // `stride` places it.
        let Some(elem) = self.through_names(elem) else { return Ok(None) };
        let Some(stride) = self.stride(doc, path, ty)? else { return Ok(None) };
        if stride == 0 {
            return Ok(None);
        }
        if same_shape(&self.template, &elem) {
            return Ok(Some(stride));
        }
        // A run of records whose fields are each a fixed number of bits or a
        // number as wide as a field outside the record says, which is how a
        // GRIB value is written: the packed integer and, taking no bits, what
        // it is worth. The width is asked of the list once, so every record
        // has the same fields at the same widths, and element 0's breakdown
        // times the count is what walking all of them would add up to.
        // Walked instead, a field of a million points is two million frames.
        if let Ty::Struct(s) = &elem {
            if same_in_every_record(&self.template, s) {
                return Ok(Some(stride));
            }
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

/// What a frame about to close left over, for whoever is following the walk.
/// Worked out the way `close_frame` works it out.
fn closing_of(f: &Frame) -> Closing<'_> {
    let (leftover, stretch) = match f.tiled {
        Some(tiled) => (f.end.saturating_sub(f.offset).saturating_sub(tiled), false),
        None if f.sequential => (f.end.saturating_sub(f.cursor), true),
        None => (0, false),
    };
    Closing {
        path: &f.path,
        offset: f.offset,
        end: f.end,
        cursor: f.cursor,
        leftover,
        stretch,
        framed: f.framed,
        scale: f.scale,
    }
}

/// Whether every record of a run of `s` has the same fields at the same widths,
/// whatever its bytes say: each field is the same in every record by
/// [`same_shape`], or a number
/// whose width is a literal or names a field that is not one of the record's
/// own, and so is answered by the field around the list for every record
/// alike.
///
/// The same test `Evaluator::stride` makes before it will place such a run by
/// arithmetic, asked again here rather than taken from its answer. A stride
/// says every record takes the same room; this says every record is the same
/// shape, which is the stronger claim the walk multiplies by, and it should
/// not start holding for some other record just because `stride` learns to
/// place one.
fn same_in_every_record(template: &Template, s: &crate::template::StructDef) -> bool {
    s.fields.iter().all(|f| {
        if same_shape(template, &f.ty) {
            return true;
        }
        let Ty::UIntExpr { bits, .. } = f.ty.without_sentinel() else { return false };
        match &**bits {
            Expr::Lit(_) => true,
            Expr::Ref(name) => !s.fields.iter().any(|g| *g.name == **name),
            _ => false,
        }
    })
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
        | Ty::Gather { .. }
        | Ty::At { .. } => true,
        Ty::Json(shape, _) => shape.composite(),
        Ty::Pickle(..) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use crate::document::Document;
    use crate::eval::{Evaluator, KindTotals, KindWalk};
    use crate::source::MemSource;
    use crate::template::{Endian, Expr as E, Template, Ty as T, Until};

    /// A grid of `n` values packed as GRIB packs them: a width named once in
    /// front, then each value the packed integer and, taking no bits, what it
    /// is worth. `walked` lays the same records out as a run that only
    /// walking settles, which is the element-by-element walk the shortcut
    /// stands in for.
    fn grid(n: u8, walked: bool) -> (Template, Vec<u8>) {
        let value = T::inline_structure(
            "Packed",
            vec![
                ("stored", T::uint_expr(E::field("bits_per_value"), Endian::Big)),
                ("worth", T::computed_real(E::field("stored").mul(E::real(0.5)))),
            ],
        );
        let values = match walked {
            true => T::sized(E::field("count"), T::repeat(value, Until::Cond(E::lit(0)))),
            false => T::array(value, E::field("count")),
        };
        let fields = vec![("bits_per_value", T::u8()), ("count", T::u8()), ("values", values)];
        let mut bytes = vec![8, n];
        bytes.extend(0..n);
        (Template::new("grid", T::structure("PackedData", fields)), bytes)
    }

    /// What the walk comes to, and how many goes of ten elements it took.
    fn totals((t, bytes): (Template, Vec<u8>)) -> (KindTotals, usize) {
        let len = bytes.len() as u64 * 8;
        let doc = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(t);
        ev.set_slice(Some(10));
        let mut walk = KindWalk::new(len);
        for goes in 1..10_000 {
            ev.begin_slice();
            let out = ev.kind_totals_step(&doc, &mut walk).unwrap();
            if out.done {
                return (out, goes);
            }
        }
        panic!("the walk did not finish");
    }

    #[test]
    fn a_run_of_packed_records_is_counted_once_and_multiplied() {
        let (fast, fast_goes) = totals(grid(200, false));
        let (slow, slow_goes) = totals(grid(200, true));
        assert_eq!(fast, slow);
        let find = |name: &str| fast.totals.iter().find(|t| t.type_name == name).map(|t| (t.bits, t.count));
        assert_eq!(find("u bits_per_value be"), Some((200 * 8, 200)));
        assert_eq!(find("computed real"), Some((0, 200)));
        assert_eq!((fast.covered_bits, fast.unmapped_bits, fast.reached_bits), (202 * 8, 0, 202 * 8));
        // One record walked rather than two hundred, each of which is three
        // steps: the record and its two fields.
        assert!(fast_goes <= 2 && slow_goes >= 60, "{fast_goes} goes against {slow_goes}");
    }

    #[test]
    fn a_run_of_records_written_as_a_type_name_is_multiplied_too() {
        // The records of the grid made fixed, eight bits and what they are
        // worth, and listed by the name of their type. Before a name was
        // looked through, this was walked record by record.
        let record = T::inline_structure(
            "Record",
            vec![("stored", T::u8()), ("worth", T::computed_real(E::field("stored").mul(E::real(0.5))))],
        );
        let run = |named: bool| {
            let elem = if named { T::Named("Record".into()) } else { record.clone() };
            let fields = vec![("count", T::u8()), ("values", T::array(elem, E::field("count")))];
            let t = Template::new("run", T::structure("Run", fields)).with_type("Record", record.clone());
            let mut bytes = vec![200];
            bytes.extend(0..200);
            totals((t, bytes))
        };
        let (named, named_goes) = run(true);
        let (inline, inline_goes) = run(false);
        assert_eq!(named, inline);
        assert_eq!((named.covered_bits, named.unmapped_bits, named.reached_bits), (201 * 8, 0, 201 * 8));
        assert!(named_goes <= 2 && inline_goes <= 2, "{named_goes} goes named, {inline_goes} written out");
    }

    #[test]
    fn a_run_of_fixed_records_that_differ_inside_is_walked_rather_than_multiplied() {
        // Three records of two bytes each, laid out as a list that divides and
        // again as one only walking settles. The walked one is what the totals
        // should come to.
        let run = |record: T, walked: bool, tail: &[u8]| {
            let records = match walked {
                true => T::sized(E::lit(6), T::repeat(record, Until::Cond(E::lit(0)))),
                false => T::array(record, E::lit(3)),
            };
            let t = Template::new("run", T::structure("Run", vec![("records", records), ("rest", T::bytes(E::Remaining))]));
            totals((t, tail.to_vec()))
        };
        // Each record points at bytes of its own, one, two and three long.
        let pointing = T::inline_structure("Pointing", vec![("len", T::u8()), ("off", T::u8()), ("to", T::at(E::field("off"), T::bytes(E::field("len"))))]);
        let bytes = [1, 6, 2, 7, 3, 9, 0xa, 0xb, 0xb, 0xc, 0xc, 0xc];
        let (fast, _) = run(pointing.clone(), false, &bytes);
        let (slow, _) = run(pointing, true, &bytes);
        assert_eq!(fast, slow);
        // Each record chooses what its field of no bytes is when it is read.
        let chosen = T::structure(
            "Chosen",
            vec![
                ("tag", T::u8()),
                ("what", T::sized(E::lit(0), T::switch(E::field("tag"), vec![(1, T::computed(E::lit(1)))], T::computed_real(E::real(0.5))))),
                ("pad", T::u8()),
            ],
        );
        let bytes = [1, 0, 0, 0, 0, 0];
        let (fast, _) = run(chosen.clone(), false, &bytes);
        let (slow, _) = run(chosen, true, &bytes);
        assert_eq!(fast, slow);
        let count = |name: &str| fast.totals.iter().find(|t| t.type_name == name).map(|t| t.count);
        assert_eq!((count("computed"), count("computed real")), (Some(1), Some(2)));
    }
}
