//! The address spaces a file opens inside itself.
//!
//! The file is space 0, and every offset in the IR is a bit of it. A
//! [`Ty::Decoded`](crate::template::Ty::Decoded) field opens another: the
//! bytes its compressed run comes to, numbered from zero, with the fields
//! declared over them counting from there. A ROOT record's object is at `+0x0`
//! of the record's stream and at no address in the file at all.
//!
//! A space belongs to the node that opened it, so it is keyed by that node's
//! path, and it is thrown away when the memo is. Nothing here maps a decoded
//! byte back to a byte of the file: a byte of deflate output is a function of
//! every byte before it, and the honest answer to "which file byte is this" is
//! the whole run.
//!
//! Opening one is refused rather than attempted when the run is past the cap
//! or does not start on a byte, and refused after the fact when the decoder
//! will not read it. All three read as the bytes that are there, with the node
//! saying which happened; see [`crate::codec::Refusal`]. A deflate, zlib or
//! gzip run whose output is past the cap is the exception: it is indexed
//! instead, up to a larger limit.
//!
//! A [`Ty::Stitched`](crate::template::Ty::Stitched) field opens a space too,
//! and that one is not a buffer: it is a table of the parts it is joined from,
//! read a part at a time, with the parts that have to be unpacked kept in a
//! small cache rather than all at once. See [`Backing`] and `stitch.rs`.
//!
//! A deflate run whose output is past the cap is not a buffer either: it is an
//! index of checkpoints, read a span at a time. See [`crate::codec::lazy`].

use std::sync::Arc;

use rustc_hash::FxHashMap;

use crate::codec::lazy::{self, Decoded, Limits, Seekable, Stepped};
use crate::codec::{Codec, Refusal, Step, StepKind, Trace};
use crate::document::Document;
use crate::source::{Missing, Source};
use crate::template::Template;

/// Which address space something is a bit of. 0 is the file.
pub type SpaceId = u32;

/// What came of asking a `Decoded` node to open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Opened {
    /// The space it opened, which is never 0.
    Space(u32),
    Refused(Refusal),
}

/// Every space this reading has opened, and what each `Decoded` node came to.
/// A stream opened as a document of its own.
///
/// The difference between this and the nesting a `Decoded` node already does
/// in the listing: that reads the stream's fields inside the file's reading,
/// under the node that opened it. This is the stream *as a file*, with its own
/// template, its own reading and its own cursor, which is what a tab is. The
/// two coexist; a reader who only wants to see what a ROOT record holds never
/// opens one of these.
///
/// It stays connected to where it came from by the trace: `map_out` says which
/// bits of the run made a byte of this, and `map_in` the other way. The trace
/// counts its bits from the start of the run, and the run is wherever the
/// stream's field is, after a gzip's header or inside a PNG's IDAT, so `run`
/// says where that is: a step's place in the file is the run's place plus the
/// step's bits, and `map_in` takes a bit of the file and takes the run's place
/// off before it asks the trace.
///
/// Its fields are read where the stream was declared, unless the template came
/// from looking at the bytes. What a stream holds is often sized, counted or
/// picked by fields outside it: a PDB stream's length is an entry in a table at
/// the front of the file, and which element of the list the stream is decides
/// what its type is. A reading of the stream's bytes alone has none of those,
/// so the tab reads the fields under the stream in the reading it was declared
/// in, and [`Tab`](super::Tab) presents them as the tab's own. A stream whose
/// bytes were recognised, a gzip of a tar, is read by a template that needs
/// nothing outside it, and has a reading of its own. See [`View`].
///
/// A stream joined from several runs opens as one of these too. When it is
/// small enough to hold whole, its trace is every part's laid end to end, with
/// each part's bits counted along an axis of the trace's own rather than at
/// any place in a file, since the runs are scattered and a PDB's run 3 can be
/// before its run 1. `runs` says where each part's run really is, and the two
/// maps answer through it: a step is given in bits of its own part's run, as a
/// step of a stream unpacked from one run is. Too large to hold whole, it is
/// read a part at a time through a [`Joined`], which keeps no trace laid end
/// to end and asks each part for its own steps.
pub struct Space {
    /// This space's own number, which is what everything outside calls it by.
    pub id: SpaceId,
    /// The space the run was unpacked from. 0 is the file.
    pub parent: SpaceId,
    /// The `Decoded` or `Stitched` node in `parent` that opened it.
    pub path: Vec<usize>,
    /// What the run was unpacked with. `Stored` for a joined stream, which was
    /// made by no one decoding: each part says what it was.
    pub codec: Codec,
    /// What the decoded bytes turned out to be, which is either what the
    /// stream's own template said or, when that said only bytes, what
    /// `recognise` made of them.
    pub template: String,
    /// True when the template came from looking at the decoded bytes rather
    /// than from what the stream's own template declared. A gzip of a tar
    /// opens as a tar, and this is what says so.
    pub recognised: bool,
    doc: Document<SpaceSource>,
    reading: Reading,
    /// The trace of the whole decoding, for a stream held whole. Empty for
    /// one read a piece at a time, whose traces are made a span at a time:
    /// see [`Space::trace_at_bit`].
    trace: Arc<Trace>,
    /// For a joined stream, where each part's run is, in the order the parts
    /// go. Empty for a stream unpacked from one run.
    runs: Vec<JoinedRun>,
    /// For a stream unpacked from one run, where that run is. Nothing for a
    /// joined stream, whose parts each say in `runs`.
    run: Option<SingleRun>,
}

/// Where the one run a stream was unpacked from is.
///
/// The trace a decoder keeps counts bits from the front of what it was handed,
/// which is the run and not the file: byte 0 of a gzip's contents is a literal
/// three bits into the deflate, and the deflate starts after the gzip header.
/// This is what turns the one count into the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SingleRun {
    /// The space the run is a field of, and where in it the run starts and
    /// how many bits it is. The space is numbered the way the reading that
    /// unpacked the stream numbers them, as a joined stream's runs are: 0 is
    /// that reading's own document, which is the file for a stream the file's
    /// reading opened, and anything else is a stream the run sits inside.
    pub run_space: SpaceId,
    pub run_offset_bits: u64,
    pub run_bits: u64,
}

/// What reads a space's fields.
enum Reading {
    /// A reading of its own, over the space's bytes, for a template that came
    /// from looking at them.
    Own(Box<super::Evaluator>),
    /// The reading the stream was declared in, under the stream, for the
    /// template the stream declared. The template is kept for what is asked
    /// of the template rather than of the file: the diagram of what the
    /// stream holds.
    Declared { template: Template, view: View },
}

/// Where the fields of a stream opened as a tab are read, when its template is
/// the one the stream declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    /// The space whose reading holds the stream: 0 for the file, or a space
    /// whose bytes were recognised and so has a reading of its own. Never a
    /// space read this way itself: a stream declared inside one of those is
    /// read in the same reading, further down.
    pub reading: SpaceId,
    /// Where the stream's contents are in that reading. Every path of the tab
    /// is a path under this one.
    pub root: Vec<usize>,
}

/// Where one part of a joined stream opened as a document came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinedRun {
    /// Which bytes of the document the part gives.
    pub out_bytes: std::ops::Range<u64>,
    /// The field the part is: a PDB page, a BGZF block's compressed run.
    pub path: Vec<usize>,
    /// Where that run is, in the space it is a field of, and how many bits it
    /// is. 0 is the file.
    pub run_space: SpaceId,
    pub run_offset_bits: u64,
    pub run_bits: u64,
    /// Whether the run was unpacked to give the part, rather than read as it
    /// sits.
    pub packed: bool,
    /// Where the part's bits start on the trace's own axis: every earlier
    /// part's run, end to end. No place in any file.
    pub(super) in_start: u64,
}

impl Space {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        id: SpaceId,
        parent: SpaceId,
        path: Vec<usize>,
        codec: Codec,
        source: SpaceSource,
        trace: Trace,
        runs: Vec<JoinedRun>,
        run: Option<SingleRun>,
        template: Template,
        view: Option<View>,
    ) -> Space {
        Space {
            id,
            parent,
            path,
            codec,
            template: template.name.clone(),
            recognised: view.is_none(),
            doc: Document::new(source),
            reading: match view {
                None => Reading::Own(Box::new(super::Evaluator::new(template))),
                Some(view) => Reading::Declared { template, view },
            },
            trace: Arc::new(trace),
            runs,
            run,
        }
    }

    /// What this space's bytes are read from: a buffer, or a run read a
    /// piece at a time. Cloning it is cheap, and a clone reads the same
    /// bytes through the same caches.
    pub fn source(&self) -> &SpaceSource {
        self.doc.source()
    }

    /// The bytes this space holds, when it holds them all. Nothing for a
    /// stream read a piece at a time: see [`Space::source`].
    pub fn held(&self) -> Option<&[u8]> {
        match self.doc.source() {
            SpaceSource::Whole(bytes) => Some(bytes),
            _ => None,
        }
    }

    /// The bytes this space holds, for a space known to be held whole.
    ///
    /// # Panics
    ///
    /// For a stream read a piece at a time. Code that does not know which it
    /// has asks [`Space::held`] or reads through [`Space::source`].
    pub fn bytes(&self) -> &[u8] {
        self.held().expect("a space held whole")
    }

    pub fn len_bytes(&self) -> u64 {
        self.doc.len_bytes()
    }

    /// The trace of the whole decoding. Empty for a stream read a piece at a
    /// time, which keeps none: see [`Space::trace_at_bit`].
    pub fn trace(&self) -> &Trace {
        &self.trace
    }

    /// The trace that names bit `bit` of the run this space was unpacked
    /// from, counted from the run's start: the whole trace for a stream held
    /// whole, and for one read a piece at a time, the trace of the block that
    /// holds that bit, or of the span that read it for a bit outside every
    /// block. Its steps count from the run's start either way, so
    /// [`crate::codec::inflate::decode_step`] reads them against the run.
    ///
    /// It uses the block's trace because the listing lays out the block's
    /// codes from that trace, so every code the listing shows can be taken
    /// apart here. A span's trace replaces its codes with one opaque step once
    /// it passes [`crate::codec::lazy::SPAN_TRACE_STEPS`] steps, which a span
    /// of 1 MiB of literals does.
    pub fn trace_at_bit(&self, bit: u64) -> Option<Arc<Trace>> {
        match self.doc.source() {
            SpaceSource::Lazy(index) => match index.block_of_bit(bit) {
                Some(i) => index.block_trace(i).ok(),
                None => index.trace_at_bit(bit),
            },
            _ => Some(self.trace.clone()),
        }
    }

    /// Which step of the decoding produced a byte of this space, with its bits
    /// counted from the start of the run it read them from.
    ///
    /// For a stream unpacked from one run that is the trace's own count, and
    /// [`Space::run`] says where the run is. For a joined stream, the step of
    /// the part the byte is in, counted from the start of that part's run
    /// rather than along the trace's own axis: a byte of what a BGZF block
    /// unpacks to names bits of that block's deflate, and [`Space::run_at`]
    /// says where the block's deflate is. A byte of a stored part is the one
    /// step over that part's bytes.
    pub fn map_out(&self, byte: u64) -> Option<Step> {
        match self.doc.source() {
            SpaceSource::Lazy(index) => return index.map_out(byte),
            SpaceSource::Joined(joined) => return joined.map_out(byte),
            SpaceSource::Whole(_) => {}
        }
        let step = self.trace.map_out(byte)?;
        if self.runs.is_empty() {
            return Some(step);
        }
        Some(in_run(step, self.run_at(byte)?))
    }

    /// Which step read a bit of the run this space was unpacked from, and so
    /// which bytes of this space that bit produced.
    ///
    /// `bit` is a bit of the space the run is a field of, not a bit of the run:
    /// for every stream the file's reading opened from a field of its own, a
    /// bit of the file. The run's place is taken off before the trace is
    /// asked, and a bit before the run or past its end is nothing of this
    /// space's. The step's bits count from the run's start, as
    /// [`Space::map_out`] counts them.
    ///
    /// A joined stream has no one run, so there the part whose run holds the
    /// bit answers, with the step's bits counted from that run's start. Nothing
    /// for a bit no part was read from.
    pub fn map_in(&self, bit: u64) -> Option<Step> {
        if self.runs.is_empty() {
            let run = self.run?;
            let in_run = bit.checked_sub(run.run_offset_bits).filter(|&at| at < run.run_bits)?;
            return match self.doc.source() {
                SpaceSource::Lazy(index) => index.map_in(in_run),
                _ => self.trace.map_in(in_run),
            };
        }
        let run = self.run_holding(bit)?;
        if let SpaceSource::Joined(joined) = self.doc.source() {
            let i = self.runs.iter().position(|r| std::ptr::eq(r, run))?;
            return joined.map_in(i, bit - run.run_offset_bits);
        }
        Some(in_run(self.trace.map_in(run.in_start + (bit - run.run_offset_bits))?, run))
    }

    /// Which part of a joined stream was read from bit `bit` of the space the
    /// stream was declared in, which is the run [`Space::map_in`] counts its
    /// step's bits from. Asked by the bit and not by the step's output, since a
    /// step that read bits and made nothing, the end of a deflate block, has
    /// its output at the start of the next part.
    pub fn run_holding(&self, bit: u64) -> Option<&JoinedRun> {
        self.runs
            .iter()
            .find(|r| r.run_space == self.parent && (r.run_offset_bits..r.run_offset_bits + r.run_bits).contains(&bit))
    }

    /// Which part of a joined stream byte `byte` is in, and where that part's
    /// run is. Nothing for a stream unpacked from one run, and past the end.
    pub fn run_at(&self, byte: u64) -> Option<&JoinedRun> {
        let i = self.runs.partition_point(|r| r.out_bytes.end <= byte);
        self.runs.get(i).filter(|r| r.out_bytes.contains(&byte))
    }

    /// Every part of a joined stream, in the order the parts go. Empty for a
    /// stream unpacked from one run.
    pub fn runs(&self) -> &[JoinedRun] {
        &self.runs
    }

    /// Where the run of a stream unpacked from one run is, which is what its
    /// steps' bits count from. Nothing for a joined stream: see
    /// [`Space::run_at`].
    pub fn run(&self) -> Option<SingleRun> {
        self.run
    }

    /// The reading over this space's own bytes, lent with its document, since
    /// one is no use without the other. Nothing for a stream read where it was
    /// declared, which has none: see [`Space::view`] and
    /// [`Evaluator::tab_node`](super::Evaluator::tab_node).
    pub fn reading(&mut self) -> Option<(&mut super::Evaluator, &Document<SpaceSource>)> {
        match &mut self.reading {
            Reading::Own(ev) => Some((ev, &self.doc)),
            Reading::Declared { .. } => None,
        }
    }

    /// Where this space's fields are read, for a stream read where it was
    /// declared. Nothing for one with a reading of its own.
    pub fn view(&self) -> Option<&View> {
        match &self.reading {
            Reading::Own(_) => None,
            Reading::Declared { view, .. } => Some(view),
        }
    }

    /// What this space's bytes are read as: the template the stream declared,
    /// with the file's named types beside it, or the one its bytes were
    /// recognised as.
    pub fn read_as(&self) -> &Template {
        match &self.reading {
            Reading::Own(ev) => ev.template(),
            Reading::Declared { template, .. } => template,
        }
    }
}

/// What the document of a space opened as a tab of its own reads its bytes
/// from.
///
/// Never reports a chunk missing: every byte is either held or decoded when
/// it is read. A read the decoder fails on, which a run that indexed cleanly
/// should never give, reads as zeroes.
#[derive(Clone)]
pub enum SpaceSource {
    /// Every byte, held.
    Whole(Arc<Vec<u8>>),
    /// One run too large to hold whole, read through its index.
    Lazy(Arc<Seekable>),
    /// A stream joined from parts and too large to hold whole, read a part at
    /// a time.
    Joined(Arc<Joined>),
}

impl SpaceSource {
    /// Bytes `at..at + out.len()`. False when they could not be read, which
    /// leaves `out` as zeroes.
    pub fn read(&self, at: u64, out: &mut [u8]) -> bool {
        let ok = match self {
            SpaceSource::Whole(bytes) => match bytes.get(at as usize..at as usize + out.len()) {
                Some(have) => {
                    out.copy_from_slice(have);
                    true
                }
                None => false,
            },
            SpaceSource::Lazy(index) => index.read(at, out).is_ok(),
            SpaceSource::Joined(joined) => joined.read(at, out).is_ok(),
        };
        if !ok {
            out.fill(0);
        }
        ok
    }
}

impl Source for SpaceSource {
    fn len_bytes(&self) -> u64 {
        match self {
            SpaceSource::Whole(bytes) => bytes.len() as u64,
            SpaceSource::Lazy(index) => index.len(),
            SpaceSource::Joined(joined) => joined.len,
        }
    }

    fn read_bytes(&self, offset: u64, out: &mut [u8]) -> Vec<Missing> {
        self.read(offset, out);
        Vec::new()
    }
}

/// A stream joined from parts, too large to hold whole, read a part at a
/// time. What [`SpaceSource::Joined`] reads.
///
/// Everything it is read from is held here, so it needs nothing of the file
/// once it is made: the bytes of each stored part, and the compressed run of
/// each packed one. A packed part small enough to decode whole is decoded when
/// it is read and kept in a cache like a stitched space's; a larger one is
/// read through its index.
pub struct Joined {
    parts: Vec<(std::ops::Range<u64>, JoinedBytes)>,
    len: u64,
    cache: std::sync::Mutex<PartCache>,
    /// The trace of the packed part asked about most recently, which is
    /// decoded again to make it.
    traced: std::sync::Mutex<Option<(usize, Arc<Trace>)>>,
}

/// Where one part of a [`Joined`] stream's bytes come from.
pub(super) enum JoinedBytes {
    /// A part stored as it is, copied out of the file.
    Held(Arc<Vec<u8>>),
    /// A part small enough to decode whole: its compressed run.
    Packed { run: Arc<Vec<u8>>, codec: Codec },
    /// A part too large to decode whole.
    Indexed(Arc<Seekable>),
}

fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Joined {
    pub(super) fn new(parts: Vec<(std::ops::Range<u64>, JoinedBytes)>, len: u64, cache: usize) -> Joined {
        Joined { parts, len, cache: std::sync::Mutex::new(PartCache::new(cache)), traced: std::sync::Mutex::new(None) }
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn part_at(&self, byte: u64) -> Option<usize> {
        let i = self.parts.partition_point(|p| p.0.end <= byte);
        self.parts.get(i).filter(|p| p.0.contains(&byte)).map(|_| i)
    }

    /// Bytes `at..at + out.len()`, from as many parts as they cover.
    pub fn read(&self, at: u64, out: &mut [u8]) -> Result<(), Refusal> {
        let end = at + out.len() as u64;
        let mut pos = at;
        while pos < end {
            let i = self.part_at(pos).ok_or(Refusal::Failed)?;
            let range = self.parts[i].0.clone();
            let take = (range.end.min(end) - pos) as usize;
            let from = pos - range.start;
            let to = (pos - at) as usize;
            match &self.parts[i].1 {
                JoinedBytes::Held(bytes) => out[to..to + take].copy_from_slice(&bytes[from as usize..from as usize + take]),
                JoinedBytes::Indexed(index) => index.read(from, &mut out[to..to + take])?,
                JoinedBytes::Packed { run, codec } => {
                    let cached = lock(&self.cache).get(i);
                    let bytes = match cached {
                        Some(bytes) => bytes,
                        None => {
                            let bytes = Arc::new(crate::codec::decode(*codec, run)?);
                            if bytes.len() as u64 != range.end - range.start {
                                return Err(Refusal::Failed);
                            }
                            lock(&self.cache).put(i, bytes.clone());
                            bytes
                        }
                    };
                    out[to..to + take].copy_from_slice(&bytes[from as usize..from as usize + take]);
                }
            }
            pos += take as u64;
        }
        Ok(())
    }

    /// The trace of packed part `i`, decoded again with its trace on.
    fn part_trace(&self, i: usize, run: &[u8], codec: Codec) -> Option<Arc<Trace>> {
        if let Some((held, t)) = lock(&self.traced).as_ref() {
            if *held == i {
                return Some(t.clone());
            }
        }
        let (_, t) = crate::codec::decode_traced(codec, run).ok()?;
        let t = Arc::new(t);
        *lock(&self.traced) = Some((i, t.clone()));
        Some(t)
    }

    /// A step of part `i` with its output moved to where the part is in the
    /// joined stream. Its bits stay counted from the start of the part's own
    /// run, as a joined stream held whole gives them.
    fn shifted(&self, i: usize, step: Step) -> Step {
        let start = self.parts[i].0.start;
        Step { out_bytes: step.out_bytes.start + start..step.out_bytes.end + start, ..step }
    }

    /// Which step of which part produced byte `byte`, as
    /// [`Space::map_out`] gives it for a joined stream held whole.
    pub fn map_out(&self, byte: u64) -> Option<Step> {
        let i = self.part_at(byte)?;
        let (range, bytes) = &self.parts[i];
        let at = byte - range.start;
        let step = match bytes {
            JoinedBytes::Held(_) => {
                Step { in_bits: 0..(range.end - range.start) * 8, out_bytes: 0..range.end - range.start, kind: StepKind::Stored }
            }
            JoinedBytes::Indexed(index) => index.map_out(at)?,
            JoinedBytes::Packed { run, codec } => self.part_trace(i, run, *codec)?.map_out(at)?,
        };
        Some(self.shifted(i, step))
    }

    /// Which step of part `i` read bit `bit` of that part's run.
    pub fn map_in(&self, i: usize, bit: u64) -> Option<Step> {
        let (range, bytes) = self.parts.get(i)?;
        let step = match bytes {
            JoinedBytes::Held(_) => {
                let bits = (range.end - range.start) * 8;
                (bit < bits).then(|| Step { in_bits: 0..bits, out_bytes: 0..range.end - range.start, kind: StepKind::Stored })?
            }
            JoinedBytes::Indexed(index) => index.map_in(bit)?,
            JoinedBytes::Packed { run, codec } => self.part_trace(i, run, *codec)?.map_in(bit)?,
        };
        Some(self.shifted(i, step))
    }
}

/// A step of a joined stream's trace with its bits counted from the start of
/// its part's run, which is a place a reader can go, rather than along the
/// trace's own axis, which is not.
fn in_run(step: Step, run: &JoinedRun) -> Step {
    let back = |bit: u64| bit.saturating_sub(run.in_start);
    Step { in_bits: back(step.in_bits.start)..back(step.in_bits.end), ..step }
}

/// What a space's bytes are kept as.
///
/// A `Decoded` node's space is one buffer, unpacked whole the first time
/// anything inside it is asked for. A `Stitched` node's is a table of where
/// its parts are, and nothing more until a read reaches one: a BAM of a
/// gigabyte is sixteen thousand parts, and holding them all unpacked is the
/// thing a stitched space exists not to do.
pub(super) enum Backing {
    Whole { bytes: Arc<Vec<u8>>, trace: Trace },
    Stitched(Box<Stitch>),
    /// A run too large to hold whole, read through its checkpoints.
    Lazy(Arc<Seekable>),
}

/// Where a compressed run is: the space it is a field of, the bit it starts
/// at there and how many bits it is. What an index is kept by, so that every
/// node reading the same run reads it through the same index.
pub(super) type RunKey = (u32, u64, u64);

/// The indexes of runs too large to hold whole, by where each run is, so a
/// run is indexed once however many things read it: the `Decoded` node over
/// it, a stitched stream with it as a part, and the tab.
///
/// Each index holds its compressed run, so the total of those is capped, and
/// the index used longest ago goes first.
#[derive(Default)]
pub(super) struct Indexes {
    entries: Vec<(RunKey, Codec, Arc<Seekable>, u64)>,
    clock: u64,
}

impl Indexes {
    fn get(&mut self, key: RunKey, codec: Codec) -> Option<Arc<Seekable>> {
        self.clock += 1;
        let clock = self.clock;
        let e = self.entries.iter_mut().find(|e| e.0 == key && e.1 == codec)?;
        e.3 = clock;
        Some(e.2.clone())
    }

    fn put(&mut self, key: RunKey, codec: Codec, index: Arc<Seekable>) {
        let held = |entries: &[(RunKey, Codec, Arc<Seekable>, u64)]| entries.iter().map(|e| e.2.packed().len()).sum::<usize>();
        while !self.entries.is_empty() && held(&self.entries) + index.packed().len() > INDEXED_RUN_BYTES {
            let oldest = (0..self.entries.len()).min_by_key(|&i| self.entries[i].3).expect("not empty");
            self.entries.swap_remove(oldest);
        }
        self.clock += 1;
        self.entries.push((key, codec, index, self.clock));
    }
}

/// The most compressed bytes the indexes hold between them. The same as the
/// most one run may be.
const INDEXED_RUN_BYTES: usize = crate::codec::CAP_BYTES;

/// Runs whose indexing pass has started and not finished, by where each run
/// is, carried on a step at a time on each go until they are done. Kept apart
/// from [`Indexes`], which holds only finished indexes: those are read with no
/// evaluator behind them, by a tab's source, and a half-made one must never
/// get there.
///
/// A pass that might still come to a run held whole holds up to the cap of
/// output, so only a couple are kept, and the one stepped longest ago goes
/// first. A reading stepping two runs by turns keeps both.
#[derive(Default)]
pub(super) struct Unfinished {
    entries: Vec<(RunKey, Codec, lazy::Indexing, u64)>,
    clock: u64,
}

/// How many unfinished passes are kept at once.
const UNFINISHED_PASSES: usize = 2;

impl Unfinished {
    /// The pass over the run at `key`, taken out to be stepped.
    fn take(&mut self, key: RunKey, codec: Codec) -> Option<lazy::Indexing> {
        let i = self.entries.iter().position(|e| e.0 == key && e.1 == codec)?;
        Some(self.entries.swap_remove(i).2)
    }

    /// Put a pass back to be carried on next go.
    fn put(&mut self, key: RunKey, codec: Codec, pass: lazy::Indexing) {
        while self.entries.len() >= UNFINISHED_PASSES {
            let oldest = (0..self.entries.len()).min_by_key(|&i| self.entries[i].3).expect("not empty");
            self.entries.swap_remove(oldest);
        }
        self.clock += 1;
        self.entries.push((key, codec, pass, self.clock));
    }

    /// How far the pass stepped most recently has got.
    fn latest(&self) -> Option<(RunKey, lazy::Progress)> {
        self.entries.iter().max_by_key(|e| e.3).map(|e| (e.0, e.2.progress()))
    }
}

/// How much output one step of an indexing pass decodes, and what it is
/// charged against a go's allowance. 512 bytes an element: a go of 5,000
/// elements, which is what the web app asks for, decodes about 2.5 MiB. In
/// wasm that took about 18 ms on 2026-10-26, with the trace on or off: 1.25
/// MiB took 9 ms. Less than the twentieth of a second a go is meant to take,
/// since a listing drawing a frame asks several things that each carry the
/// same pass on.
const INDEX_STEP_BYTES: u64 = 256 << 10;
const INDEX_STEP_ELEMENTS: u64 = INDEX_STEP_BYTES / 512;

/// A space made of parts that are elsewhere, joined end to end.
pub(super) struct Stitch {
    /// The `Stitched` node that opened it, which is what knows the walk its
    /// parts were found by.
    pub(super) path: Vec<usize>,
    /// Every part, by where it starts in the joined bytes. Complete before
    /// anything inside is placed, so the total is known.
    pub(super) parts: Vec<Part>,
    pub(super) len_bytes: u64,
    /// The packed parts unpacked so far, as many as fit under the cap.
    pub(super) cache: std::cell::RefCell<PartCache>,
}

impl Stitch {
    /// The part holding byte `at` of the joined bytes. A part of no bytes
    /// holds nothing, so the answer is the next part along that has any.
    pub(super) fn part_at(&self, at: u64) -> Option<usize> {
        let i = self.parts.partition_point(|p| p.start + p.len <= at);
        self.parts.get(i).filter(|p| p.start <= at && at < p.start + p.len).map(|_| i)
    }
}

/// One run of a stitched space, and where in the joined bytes it goes.
#[derive(Debug, Clone)]
pub(super) struct Part {
    /// Where the part starts in the joined bytes, and how many it gives.
    pub(super) start: u64,
    pub(super) len: u64,
    /// The field the walk landed on, which is the run in the file (or in the
    /// space the run is in). Kept so a byte of the joined stream can name the
    /// field it came from, and so an unpacked part already open as a stream of
    /// its own is read from there rather than unpacked twice.
    pub(super) path: Vec<usize>,
    pub(super) source: PartSource,
}

/// Where a part's bytes come from.
#[derive(Debug, Clone)]
pub(super) enum PartSource {
    /// Bytes as they sit: a PDB page. `at_bits` is where they start in
    /// `space`.
    Stored { space: u32, at_bits: u64 },
    /// A run that has to be unpacked first: a BGZF member's deflate. The
    /// codec was worked out when the walk reached the run, so a read never
    /// needs to ask the template anything. `Err` is a codec that could not be
    /// worked out, which fails a read of this part and nothing else.
    Packed { space: u32, at_bits: u64, size_bits: u64, codec: Result<Codec, Refusal> },
}

/// How many unpacked bytes a stitched space keeps at once, across all its
/// parts. Two hundred and fifty-six BGZF members, which is several thousand
/// short reads either side of wherever the reader is.
pub(super) const STITCH_CACHE_BYTES: usize = 16 << 20;

/// The parts of a stitched space that have been unpacked, kept until the cap
/// says one has to go, and then the one read longest ago goes first.
pub(super) struct PartCache {
    entries: FxHashMap<usize, (Arc<Vec<u8>>, u64)>,
    /// How many bytes the entries hold, and the most they have held at once.
    bytes: usize,
    pub(super) peak: usize,
    /// A count of reads, so the entry read longest ago is the one with the
    /// smallest.
    clock: u64,
    cap: usize,
}

impl PartCache {
    pub(super) fn new(cap: usize) -> PartCache {
        PartCache { entries: FxHashMap::default(), bytes: 0, peak: 0, clock: 0, cap }
    }

    /// Which parts are being kept, in no order, and how many bytes they hold.
    #[cfg(test)]
    pub(super) fn held(&self) -> (Vec<usize>, usize) {
        (self.entries.keys().copied().collect(), self.bytes)
    }

    /// Part `i`, if it is being kept, marked as read now.
    pub(super) fn get(&mut self, i: usize) -> Option<Arc<Vec<u8>>> {
        self.clock += 1;
        let clock = self.clock;
        self.entries.get_mut(&i).map(|(bytes, read)| {
            *read = clock;
            bytes.clone()
        })
    }

    /// Keep part `i`, putting back the parts read longest ago until it fits.
    ///
    /// A part larger than the whole cap is still kept, beside the others, and
    /// only another part that large puts it back. A stream of one large part and
    /// some small ones is read by turns: a BP5 dataset joined from a ZIP reads a
    /// block's counts from `md.0` and its values from `data.0`, one after the
    /// other, and a `data.0` put back for every read of `md.0` would be unpacked
    /// again for every value. So the cap bounds the parts within it, and at most
    /// one part past it is held as well.
    pub(super) fn put(&mut self, i: usize, bytes: Arc<Vec<u8>>) {
        let large = bytes.len() > self.cap;
        loop {
            let small: usize = self.entries.values().map(|(b, _)| b.len()).filter(|&n| n <= self.cap).sum();
            let over = if large {
                self.entries.values().any(|(b, _)| b.len() > self.cap)
            } else {
                small + bytes.len() > self.cap
            };
            if !over {
                break;
            }
            let oldest = self
                .entries
                .iter()
                .filter(|(_, (b, _))| (b.len() > self.cap) == large)
                .min_by_key(|(_, (_, read))| *read)
                .map(|(k, _)| *k);
            let Some(oldest) = oldest else { break };
            if let Some((gone, _)) = self.entries.remove(&oldest) {
                self.bytes -= gone.len();
            }
        }
        self.clock += 1;
        self.bytes += bytes.len();
        self.peak = self.peak.max(self.bytes);
        self.entries.insert(i, (bytes, self.clock));
    }
}

pub(super) struct Spaces {
    /// What space `i + 1` is made of. Space 0 is the file and is not here.
    backings: Vec<Backing>,
    /// What each `Decoded` or `Stitched` node came to, so a stream is opened
    /// once however many times its children are asked for.
    opened: FxHashMap<Vec<usize>, Opened>,
    /// The cap each stitched space opened from here on is given.
    cache_cap: usize,
    /// Why a stitched stream would not open as a document of its own, for the
    /// ones that were asked to and would not. Kept apart from `opened`, which
    /// says the stream itself opened: one too long to hold whole still reads a
    /// part at a time in the listing.
    whole_refused: FxHashMap<Vec<usize>, Refusal>,
    /// The most a stitched stream may come to and still be held whole.
    whole_cap: usize,
    /// The limits a run too large to hold whole is indexed under.
    lazy_limits: Limits,
    /// Every index made so far that is still kept. Asked for while reading,
    /// which borrows the spaces shared, so it is a cell.
    indexes: std::cell::RefCell<Indexes>,
    /// Indexes still being made. A cell for the same reason.
    unfinished: std::cell::RefCell<Unfinished>,
}

impl Default for Spaces {
    fn default() -> Spaces {
        Spaces {
            backings: Vec::new(),
            opened: FxHashMap::default(),
            cache_cap: STITCH_CACHE_BYTES,
            whole_refused: FxHashMap::default(),
            whole_cap: crate::codec::CAP_BYTES,
            lazy_limits: Limits::default(),
            indexes: Default::default(),
            unfinished: Default::default(),
        }
    }
}

impl Spaces {
    pub(crate) fn get(&self, path: &[usize]) -> Option<Opened> {
        self.opened.get(path).copied()
    }

    /// Keep a decoded buffer and its trace, and hand back the space it became.
    pub(super) fn add(&mut self, path: &[usize], bytes: Vec<u8>, trace: Trace) -> u32 {
        self.backings.push(Backing::Whole { bytes: Arc::new(bytes), trace });
        let id = self.backings.len() as u32;
        self.opened.insert(path.to_vec(), Opened::Space(id));
        id
    }

    /// Keep the index of a run too large to hold whole, and hand back the
    /// space it became.
    pub(super) fn add_lazy(&mut self, path: &[usize], index: Arc<Seekable>) -> u32 {
        self.backings.push(Backing::Lazy(index));
        let id = self.backings.len() as u32;
        self.opened.insert(path.to_vec(), Opened::Space(id));
        id
    }

    /// The index of the run at `key`, packed with `codec`, if one was made
    /// and is still kept.
    pub(super) fn indexed(&self, key: RunKey, codec: Codec) -> Option<Arc<Seekable>> {
        self.indexes.borrow_mut().get(key, codec)
    }

    /// Keep an index of the run at `key`.
    pub(super) fn keep_index(&self, key: RunKey, codec: Codec, index: Arc<Seekable>) {
        self.indexes.borrow_mut().put(key, codec, index);
    }

    /// The limits a run too large to hold whole is indexed under.
    pub(super) fn lazy_limits(&self) -> Limits {
        self.lazy_limits
    }

    /// Those limits made small, so that a test reaches them with a few
    /// hundred kilobytes rather than sixty-four megabytes.
    #[cfg(test)]
    pub(super) fn set_lazy_limits(&mut self, limits: Limits) {
        self.lazy_limits = limits;
    }

    /// The index behind a space read a piece at a time. Nothing for any
    /// other space.
    pub(super) fn lazy(&self, space: u32) -> Option<&Arc<Seekable>> {
        match self.backings.get(space.checked_sub(1)? as usize)? {
            Backing::Lazy(index) => Some(index),
            _ => None,
        }
    }

    /// Keep the part table of a stitched space, and hand back the space it
    /// became.
    pub(super) fn add_stitched(&mut self, path: &[usize], parts: Vec<Part>, len_bytes: u64) -> u32 {
        let cache = std::cell::RefCell::new(PartCache::new(self.cache_cap));
        self.backings.push(Backing::Stitched(Box::new(Stitch { path: path.to_vec(), parts, len_bytes, cache })));
        let id = self.backings.len() as u32;
        self.opened.insert(path.to_vec(), Opened::Space(id));
        id
    }

    /// How many unpacked bytes a stitched space opened after this may keep.
    /// For a test that wants to see the cap at work without a gigabyte file.
    #[cfg(test)]
    pub(super) fn set_cache_cap(&mut self, bytes: usize) {
        self.cache_cap = bytes;
    }

    /// The most a stitched stream may come to and still open as a document of
    /// its own. [`crate::codec::CAP_BYTES`], the cap a `Decoded` stream has.
    pub(super) fn whole_cap(&self) -> usize {
        self.whole_cap
    }

    /// The same cap made small, for a test that wants a stream past it without
    /// sixty-four megabytes of file.
    #[cfg(test)]
    pub(super) fn set_whole_cap(&mut self, bytes: usize) {
        self.whole_cap = bytes;
    }

    pub(super) fn refuse_whole(&mut self, path: &[usize], why: Refusal) {
        self.whole_refused.insert(path.to_vec(), why);
    }

    /// Why the stitched stream at `path` would not open as a document of its
    /// own, when it was asked to and would not.
    pub(super) fn whole_refusal(&self, path: &[usize]) -> Option<Refusal> {
        self.whole_refused.get(path).copied()
    }

    /// The trace of the decoding that made a space. Nothing for a stitched
    /// space, which was not made by one decoding.
    pub(super) fn trace(&self, space: u32) -> Option<&Trace> {
        match self.backings.get(space.checked_sub(1)? as usize)? {
            Backing::Whole { trace, .. } => Some(trace),
            Backing::Stitched(_) | Backing::Lazy(_) => None,
        }
    }

    pub(super) fn refuse(&mut self, path: &[usize], why: Refusal) {
        self.opened.insert(path.to_vec(), Opened::Refused(why));
    }

    /// The bytes of a space held whole. `space` is never 0 here: the file is
    /// read through the document, not through this. Nothing for a stitched
    /// space, whose bytes are nowhere in one piece.
    pub(super) fn buf(&self, space: u32) -> Option<&Arc<Vec<u8>>> {
        match self.backings.get(space.checked_sub(1)? as usize)? {
            Backing::Whole { bytes, .. } => Some(bytes),
            Backing::Stitched(_) | Backing::Lazy(_) => None,
        }
    }

    /// The part table of a stitched space.
    pub(super) fn stitch(&self, space: u32) -> Option<&Stitch> {
        match self.backings.get(space.checked_sub(1)? as usize)? {
            Backing::Stitched(s) => Some(s),
            Backing::Whole { .. } | Backing::Lazy(_) => None,
        }
    }

    /// How many bits a space holds.
    pub(super) fn len_bits(&self, space: u32) -> u64 {
        match space.checked_sub(1).and_then(|i| self.backings.get(i as usize)) {
            Some(Backing::Whole { bytes, .. }) => bytes.len() as u64 * 8,
            Some(Backing::Stitched(s)) => s.len_bytes * 8,
            Some(Backing::Lazy(index)) => index.len() * 8,
            None => 0,
        }
    }

    /// Whether any stream has been opened at all. Most files hold none, and
    /// the sweep that drops decoded nodes should cost them nothing.
    pub(super) fn any(&self) -> bool {
        !self.opened.is_empty()
    }

    /// Start again from nothing. A decoded buffer is worked out from bytes of
    /// the file, so any change to the file or the template drops it: see
    /// `Memo::forget`.
    pub(super) fn forget(&mut self) {
        self.backings.clear();
        self.opened.clear();
        self.whole_refused.clear();
        // Kept by where a run is, and an edit can change what is there.
        self.indexes.borrow_mut().entries.clear();
        self.unfinished.borrow_mut().entries.clear();
    }

    /// The run being indexed most recently, and how far its pass has got,
    /// while it has not finished.
    pub(super) fn unpacking(&self) -> Option<(RunKey, lazy::Progress)> {
        self.unfinished.borrow().latest()
    }
}

impl super::Evaluator {
    /// The run at `key`, packed with `codec`, decoded whole when it fits under
    /// the cap and indexed when it does not: [`lazy::decode_or_index`], done
    /// a step at a time. Each step is charged to this go's allowance, and a go
    /// that runs out hands back `Busy` with the pass kept, to be carried on
    /// from where it stopped the next time anything asks for the same run.
    /// `Ok(Err)` is the run refusing, as it would refuse in one go.
    /// `known_large` is a caller that knows the run comes to more than the
    /// cap, which lets the pass skip the part that might have been a run held
    /// whole: see [`lazy::Indexing::known_large`].
    ///
    /// Takes the evaluator shared, since a read of a stitched stream reaches
    /// here, and reads do.
    pub(super) fn index_run<S: Source>(&self, doc: &Document<S>, key: RunKey, codec: Codec, known_large: bool) -> super::R<Result<Decoded, Refusal>> {
        let started = self.spaces.unfinished.borrow_mut().take(key, codec);
        let mut pass = match started {
            Some(pass) => pass,
            None => {
                if key.2 / 8 > crate::codec::CAP_BYTES as u64 {
                    return Ok(Err(Refusal::TooLarge));
                }
                let packed = Arc::new(self.read_in(doc, key.0, key.1, key.2)?);
                match lazy::Indexing::new(codec, packed, self.spaces.lazy_limits) {
                    Ok(pass) if known_large => pass.known_large(),
                    Ok(pass) => pass,
                    Err(why) => return Ok(Err(why)),
                }
            }
        };
        loop {
            if let Err(busy) = self.go.charge(INDEX_STEP_ELEMENTS) {
                self.spaces.unfinished.borrow_mut().put(key, codec, pass);
                return Err(busy);
            }
            match pass.step(INDEX_STEP_BYTES) {
                Ok(Stepped::Working) => {}
                Ok(Stepped::Done(decoded)) => return Ok(Ok(decoded)),
                Err(why) => return Ok(Err(why)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::codec::StepKind;
    use crate::document::Document;
    use crate::eval::space::{Opened, SpaceSource};
    use crate::eval::{Evaluator, SingleRun, Value};
    use crate::formats;
    use crate::source::MemSource;

    /// A run whose codec is three numbers the file wrote down, which is what
    /// [`Packing::Lzma1`](crate::template::Packing::Lzma1) exists for.
    ///
    /// A made-up container: the five bytes an LZMA header spends on its
    /// settings, the size that comes out, and then the stream. No format is
    /// laid out quite like this; what is being tested is that the numbers
    /// reach the decoder from *fields*, which is the whole of the change, and
    /// a real archive's are in another part of the file entirely.
    #[test]
    fn a_codec_takes_its_settings_from_fields_of_the_file() {
        use crate::template::{Expr as E, Packing, Template, Ty as T};

        let text = b"what a 7z coder writes down in its header and nowhere else";
        // The stream alone, taken from the `alone` file lzma writes: thirteen
        // bytes of header and then the range-coded bits.
        let alone = {
            let mut out = Vec::new();
            lzma_rs::lzma_compress(&mut &text[..], &mut out).expect("packs");
            out
        };
        let (props, dict) = (alone[0], u32::from_le_bytes(alone[1..5].try_into().unwrap()));
        let mut bytes = vec![props];
        bytes.extend_from_slice(&dict.to_le_bytes());
        bytes.extend_from_slice(&(text.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&alone[13..]);

        let make_named = |props: &str| Template::new(
            "made-up",
            T::structure(
                "Packed",
                vec![
                    ("props", T::u8()),
                    ("dict_size", T::u32(crate::template::Endian::Little)),
                    ("unpacked_size", T::u32(crate::template::Endian::Little)),
                    (
                        "stream",
                        T::decoded_as(
                            E::Remaining,
                            Packing::Lzma1 {
                                props: E::field(props),
                                dict_size: E::field("dict_size"),
                                unpacked: Some(E::field("unpacked_size")),
                            },
                            T::bytes(E::Remaining),
                        ),
                    ),
                ],
            ),
        );
        let d = Document::new(MemSource(bytes));
        let mut e = Evaluator::new(make_named("props"));
        let id = e.open_space(&d, 0, &[3]).unwrap().expect("the stream opens");
        assert_eq!(e.space(id).expect("it is there").bytes(), text);

        // The same run with the properties byte changed stays bytes. A codec
        // worked out from the file can be worked out wrongly, and the answer
        // for that is the answer for any stream that will not unpack: the run
        // is what it is and the node says why, rather than other bytes.
        let mut wrong = d.source().0.clone();
        wrong[0] = 0xff;
        let wrong = Document::new(MemSource(wrong));
        let mut e = Evaluator::new(make_named("props"));
        assert_eq!(e.open_space(&wrong, 0, &[3]).unwrap(), None, "a stream packed a way this cannot read stays bytes");

        // And a template naming a field this file has none of answers the same
        // way. Which field holds a codec's settings is often chosen by a switch
        // on what the file says the codec is, so a template that is right for
        // the files that have them names nothing in the files that do not: a
        // 7z header packed with `copy` writes no properties at all. The run
        // stays bytes, and the node it sits in still reads.
        let mut e = Evaluator::new(make_named("no_such_field"));
        assert!(e.node(&d, &[3]).is_ok(), "a run that will not open is still a field");
        assert_eq!(e.open_space(&d, 0, &[3]).unwrap(), None, "a name that is not there is not an error");

        // The two say different things, which is the point of telling them
        // apart. Nothing read the bytes in the second case, so nothing is
        // claiming they are wrong: a reader told unpacking failed there would
        // go looking for damage in a file that has none.
        let mut e = Evaluator::new(make_named("no_such_field"));
        assert_eq!(e.node(&d, &[3]).unwrap().refused.as_deref(), Some("settings"));
        let mut e = Evaluator::new(make_named("props"));
        assert_eq!(e.node(&wrong, &[3]).unwrap().refused.as_deref(), Some("failed"));
    }

    /// A part past the cap is kept beside the small parts read by turns with it,
    /// and put back only for another part past the cap: one large data file and
    /// the small metadata that points into it are not each unpacked again for
    /// every read of the other.
    #[test]
    fn a_part_larger_than_the_cap_is_kept_beside_the_small_ones() {
        use super::PartCache;
        use std::sync::Arc;
        let part = |n: usize| Arc::new(vec![0u8; n]);
        let held = |c: &PartCache| {
            let (mut parts, bytes) = c.held();
            parts.sort();
            (parts, bytes)
        };
        let mut c = PartCache::new(100);
        c.put(0, part(300));
        c.put(1, part(50));
        c.put(2, part(40));
        assert_eq!(held(&c), (vec![0, 1, 2], 390), "the small parts do not put back the large one");
        // Past the cap among the small ones, the oldest small part goes.
        c.put(3, part(30));
        assert_eq!(held(&c), (vec![0, 2, 3], 370));
        // Another large part puts back the first, and no small part.
        c.put(4, part(200));
        assert_eq!(held(&c), (vec![2, 3, 4], 270));
    }

    /// A file that is one zlib stream, over whatever is handed in.
    fn zlib_over(content: &[u8]) -> Document<MemSource> {
        Document::new(MemSource(miniz_oxide::deflate::compress_to_vec_zlib(content, 6)))
    }

    /// The `compressed` field of the zlib template.
    const RUN: &[usize] = &[6];

    #[test]
    fn a_stream_opens_as_a_document_of_its_own() {
        let d = zlib_over(b"hello, this is the text inside the stream");
        let mut e = Evaluator::new(formats::builtin("zlib").unwrap());
        let id = e.open_space(&d, 0, RUN).unwrap().expect("the stream opens");
        assert_eq!(id, 1);
        let space = e.space(id).expect("it is there");
        assert_eq!(space.parent, 0);
        assert_eq!(space.path, RUN);
        assert_eq!(space.bytes(), b"hello, this is the text inside the stream");
        // Asking again is the same space, not another copy of it.
        assert_eq!(e.open_space(&d, 0, RUN).unwrap(), Some(id));
        // And it reads: the template the stream declared says text.
        let node = e.tab_node(&d, id, &[0]).unwrap();
        assert_eq!(node.value, Value::Str("hello, this is the text inside the stream".into()));
    }

    /// A stream whose template says only "bytes" opens as whatever the bytes
    /// turn out to be. A gzip of a tar is a tar.
    #[test]
    fn bytes_that_know_what_they_are_open_as_that()
    {
        let d = zlib_over(&tar(b"notes.txt", b"a file inside a tar inside a stream"));
        let mut e = Evaluator::new(formats::builtin("zlib").unwrap());
        let id = e.open_space(&d, 0, RUN).unwrap().expect("the stream opens");
        let space = e.space(id).unwrap();
        assert_eq!(space.template, "tar");
        assert!(space.recognised, "the template came from the template, not from the bytes");
    }

    /// A stream inside a stream is a space beside its parent, and says which
    /// one it came out of.
    #[test]
    fn a_stream_inside_a_stream_opens_beside_it() {
        let inner = miniz_oxide::deflate::compress_to_vec_zlib(b"two deep", 6);
        let d = zlib_over(&inner);
        let mut e = Evaluator::new(formats::builtin("zlib").unwrap());
        let first = e.open_space(&d, 0, RUN).unwrap().expect("the outer stream opens");
        assert_eq!(e.space(first).unwrap().template, "zlib");
        let second = e.open_space(&d, first, RUN).unwrap().expect("the inner stream opens");
        assert_eq!(second, 2);
        assert_eq!(e.space(second).unwrap().parent, first);
        assert_eq!(e.space(second).unwrap().bytes(), b"two deep");
    }

    /// Every byte of a space came from a step, and every bit of the run it was
    /// unpacked from was read by one.
    ///
    /// The step's bits count from the start of the run, and the run is where
    /// the stream's field is: two bytes into a zlib file, after the header. So
    /// the bit of the file that leads back to a step is the run's place plus
    /// the step's bits, and the header's bits lead nowhere.
    /// A stream whose output is past the cap is indexed rather than refused:
    /// its fields read, a read anywhere in it decodes only the span it is in,
    /// and the Adler-32 after it is checked against the sum the indexing pass
    /// took. The limits are made small, so a few megabytes are past them.
    #[test]
    fn a_stream_past_the_cap_reads_a_piece_at_a_time() {
        use crate::codec::lazy::Limits;
        use crate::eval::space::Opened;
        let text: Vec<u8> = (0..60_000).flat_map(|i| format!("line {i} of a stream too large to hold\n").into_bytes()).collect();
        let d = zlib_over(&text);
        let mut e = Evaluator::new(formats::builtin("zlib").unwrap());
        e.spaces.set_lazy_limits(Limits { whole: 64 << 10, spacing: 16 << 10, checkpoints: 32, span_cache: 64 << 10, ..Limits::default() });
        let inner = e.node(&d, &[6, 0]).unwrap();
        assert_eq!(inner.size_bits, text.len() as u64 * 8);
        let Some(Opened::Space(id)) = e.spaces.get(RUN) else { panic!("the stream did not open") };
        let index = e.spaces.lazy(id).expect("indexed, not held whole").clone();
        assert!(index.spans() > 10);
        for at in [0usize, 1000, 777_777, text.len() - 100] {
            let got = e.read_in(&d, id, at as u64 * 8, 800).unwrap();
            assert_eq!(got, &text[at..at + 100], "100 bytes at {at}");
        }
        // Four reads far apart, and what the index holds is its checkpoints
        // and a few spans.
        assert!(index.held_bytes() < 2 << 20, "{} bytes held", index.held_bytes());
        let verdict = e.run_check(&d, &[7]).unwrap().expect("the Adler-32 is a check");
        assert!(verdict.ok, "{verdict:?}");
        // The blocks are listed from the index, which found where each one
        // starts on the same pass.
        assert_eq!(e.node(&d, RUN).unwrap().child_count, 2);
        // A second node over the same run is read through the same index.
        assert_eq!(e.spaces.indexed((0, 16, (d.len_bytes() - 6) * 8), crate::codec::Codec::Deflate).map(|i| std::sync::Arc::ptr_eq(&i, &index)), Some(true));
    }

    /// Limits small enough that a few megabytes of text are past the cap and
    /// take several goes to index.
    fn small_limits() -> crate::codec::lazy::Limits {
        crate::codec::lazy::Limits { whole: 64 << 10, spacing: 16 << 10, checkpoints: 32, span_cache: 64 << 10, ..Default::default() }
    }

    fn many_lines() -> Vec<u8> {
        (0..60_000).flat_map(|i| format!("line {i} of a stream opened a go at a time\n").into_bytes()).collect()
    }

    /// Indexing a stream past the cap is charged to the work allowed for a
    /// go like any other reading: a go that runs out hands back `Busy` and
    /// says how far the pass has got, and the next go carries on from there.
    /// What it comes to is the index one go makes.
    #[test]
    fn a_stream_past_the_cap_opens_a_go_at_a_time() {
        use crate::eval::EvalError;
        let text = many_lines();
        let d = zlib_over(&text);
        let mut one = Evaluator::new(formats::builtin("zlib").unwrap());
        one.spaces.set_lazy_limits(small_limits());
        let id = one.open_space(&d, 0, RUN).unwrap().expect("opens in one go");
        let SpaceSource::Lazy(whole) = one.space(id).unwrap().source().clone() else { panic!("held whole") };

        let mut e = Evaluator::new(formats::builtin("zlib").unwrap());
        e.spaces.set_lazy_limits(small_limits());
        e.set_slice(Some(600));
        let (mut goes, mut read) = (0, 0);
        let id = loop {
            e.begin_slice();
            match e.open_space(&d, 0, RUN) {
                Ok(Some(id)) => break id,
                Err(EvalError::Busy { .. }) => {
                    goes += 1;
                    let p = e.unpacking().expect("a pass is under way");
                    assert_eq!((p.run_space, p.run_offset_bits), (0, 16));
                    assert!(p.read_bits >= read && p.read_bits <= p.run_bits, "{read} then {p:?}");
                    read = p.read_bits;
                }
                other => panic!("{other:?}"),
            }
        };
        assert!(goes >= 3, "{goes} goes");
        assert_eq!(e.unpacking(), None);
        let SpaceSource::Lazy(index) = e.space(id).unwrap().source().clone() else { panic!("held whole") };
        assert_eq!((index.len(), index.crc32(), index.spans()), (whole.len(), whole.crc32(), whole.spans()));
        assert!((0..index.spans()).all(|k| index.span_bytes(k) == whole.span_bytes(k)));
        let mut got = vec![0u8; text.len()];
        assert!(index.read(0, &mut got).is_ok() && got == text);
    }

    /// The same for a gzip's joined stream, which is what the gzip template
    /// reads its member through, and which the listing reaches by reading
    /// the stream's first bytes rather than by opening it.
    #[test]
    fn a_gzip_past_the_cap_reads_a_go_at_a_time() {
        use crate::eval::EvalError;
        let text = many_lines();
        let mut gzip = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3];
        gzip.extend(miniz_oxide::deflate::compress_to_vec(&text, 6));
        gzip.extend(crate::checksum::crc32(&text).to_le_bytes());
        gzip.extend((text.len() as u32).to_le_bytes());
        let d = Document::new(MemSource(gzip));
        let mut e = Evaluator::new(formats::builtin("gzip").unwrap());
        e.spaces.set_lazy_limits(small_limits());
        e.set_slice(Some(600));
        let mut goes = 0;
        let node = loop {
            e.begin_slice();
            match e.node(&d, &[1, 0, 0]) {
                Ok(node) => break node,
                Err(EvalError::Busy { .. }) => {
                    goes += 1;
                    assert!(e.unpacking().is_some());
                    assert!(goes < 1000);
                }
                Err(other) => panic!("{other:?}"),
            }
        };
        assert!(goes >= 3, "{goes} goes");
        assert_eq!(node.size_bits, text.len() as u64 * 8);
        let Some(Opened::Space(id)) = e.spaces.get(&[1]) else { panic!("the joined stream did not open") };
        e.begin_slice();
        for at in [0usize, 777_777, text.len() - 100] {
            assert_eq!(e.read_in(&d, id, at as u64 * 8, 800).unwrap(), &text[at..at + 100], "100 bytes at {at}");
        }
    }

    #[test]
    fn the_map_runs_both_ways() {
        let text = "map me both ways. ".repeat(50).into_bytes();
        let d = zlib_over(&text);
        let mut e = Evaluator::new(formats::builtin("zlib").unwrap());
        let field = e.node(&d, RUN).unwrap();
        assert_eq!(field.offset_bits, 16, "the run starts after the two header bytes");
        let id = e.open_space(&d, 0, RUN).unwrap().unwrap();
        let space = e.space(id).unwrap();
        let run = SingleRun { run_space: 0, run_offset_bits: field.offset_bits, run_bits: field.size_bits };
        assert_eq!(space.run(), Some(run));
        for byte in 0..text.len() as u64 {
            let step = space.map_out(byte).unwrap_or_else(|| panic!("byte {byte} came from nowhere"));
            assert!(step.out_bytes.contains(&byte));
            assert!(matches!(step.kind, StepKind::Literal(_) | StepKind::Match { .. } | StepKind::Stored | StepKind::Pixel));
            assert!(step.in_bits.end <= field.size_bits, "{:?} is past the run's {} bits", step.in_bits, field.size_bits);
            // And the bits it read, as bits of the file, lead back to it.
            assert_eq!(space.map_in(field.offset_bits + step.in_bits.start), Some(step));
        }
        assert_eq!(space.map_out(text.len() as u64), None);
        for bit in 0..field.offset_bits {
            assert_eq!(space.map_in(bit), None, "bit {bit} is the zlib header's, not the run's");
        }
        assert_eq!(space.map_in(field.offset_bits + field.size_bits), None, "the Adler-32 after the run");
    }

    /// Editing the file drops the spaces: a decoded byte is worked out from
    /// bytes of the file, and a tab over one that has changed is stale.
    #[test]
    fn an_edit_to_the_file_closes_the_spaces() {
        let mut d = zlib_over(b"before");
        let mut e = Evaluator::new(formats::builtin("zlib").unwrap());
        assert!(e.open_space(&d, 0, RUN).unwrap().is_some());
        assert_eq!(e.spaces_open().count(), 1);
        d.overwrite_bytes(3, &[0x00]);
        e.invalidate_from(3 * 8);
        assert_eq!(e.spaces_open().count(), 0);
        assert!(e.space(1).is_none());
    }

    /// One tar member, which is a 512-byte header and the file rounded up to
    /// the next 512, then two empty blocks to end the archive.
    fn tar(name: &[u8], body: &[u8]) -> Vec<u8> {
        let mut head = [0u8; 512];
        head[..name.len()].copy_from_slice(name);
        head[100..107].copy_from_slice(b"0000644");
        head[108..115].copy_from_slice(b"0000000");
        head[116..123].copy_from_slice(b"0000000");
        let size = format!("{:011o} ", body.len());
        head[124..136].copy_from_slice(size.as_bytes());
        head[136..148].copy_from_slice(b"00000000000 ");
        head[156] = b'0';
        head[257..263].copy_from_slice(b"ustar\0");
        head[263..265].copy_from_slice(b"00");
        // The checksum is worked out with its own field read as spaces.
        head[148..156].copy_from_slice(b"        ");
        let sum: u32 = head.iter().map(|&b| b as u32).sum();
        head[148..154].copy_from_slice(format!("{sum:06o}").as_bytes());
        head[154] = 0;
        head[155] = b' ';
        let mut out = head.to_vec();
        out.extend_from_slice(body);
        out.resize(512 + body.len().next_multiple_of(512), 0);
        out.resize(out.len() + 1024, 0);
        out
    }
}
