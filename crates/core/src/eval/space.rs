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
//! gzip run, a zstd run of several frames or an xz stream of several blocks
//! whose output is past the cap is the exception: it is indexed instead, up
//! to a larger limit.
//!
//! A [`Ty::Stitched`](crate::template::Ty::Stitched) field opens a space too,
//! and that one is not a buffer: it is a table of the parts it is joined from,
//! read a part at a time, with the parts that have to be unpacked kept in a
//! small cache rather than all at once. See [`Backing`] and `stitch.rs`.
//!
//! Such a run is not a buffer either: it is an index of checkpoints, read a
//! span at a time. See [`crate::codec::lazy`].

use std::sync::Arc;

use rustc_hash::FxHashMap;

use crate::codec::lazy::{self, Decoded, Limits, ReadError, Seekable, Stepped};
use crate::codec::inflate;
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
    ///
    /// Such a trace is made from the run's compressed bytes, which may be
    /// missing chunks of the file: the error is those chunks. Nothing when
    /// the decoder refuses the run there.
    pub fn trace_at_bit(&self, bit: u64) -> Result<Option<Arc<Trace>>, Vec<Missing>> {
        match self.doc.source() {
            SpaceSource::Lazy(index) => match index.block_of_bit(bit)? {
                Some(i) => lazy::refused_as_none(index.block_trace(i)),
                None => index.trace_at_bit(bit),
            },
            _ => Ok(Some(self.trace.clone())),
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
    ///
    /// A stream read through an index makes the step's trace from its run,
    /// and the error is the chunks of the file the run is missing.
    pub fn map_out(&self, byte: u64) -> Result<Option<Step>, Vec<Missing>> {
        match self.doc.source() {
            SpaceSource::Lazy(index) => return index.map_out(byte),
            SpaceSource::Joined(joined) => return joined.map_out(byte),
            SpaceSource::Whole(_) => {}
        }
        let Some(step) = self.trace.map_out(byte) else { return Ok(None) };
        if self.runs.is_empty() {
            return Ok(Some(step));
        }
        Ok(self.run_at(byte).map(|run| in_run(step, run)))
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
    /// for a bit no part was read from. Missing chunks as for
    /// [`Space::map_out`].
    pub fn map_in(&self, bit: u64) -> Result<Option<Step>, Vec<Missing>> {
        if self.runs.is_empty() {
            let Some(run) = self.run else { return Ok(None) };
            let Some(in_run) = bit.checked_sub(run.run_offset_bits).filter(|&at| at < run.run_bits) else { return Ok(None) };
            return match self.doc.source() {
                SpaceSource::Lazy(index) => index.map_in(in_run),
                _ => Ok(self.trace.map_in(in_run)),
            };
        }
        let Some(run) = self.run_holding(bit) else { return Ok(None) };
        if let SpaceSource::Joined(joined) = self.doc.source() {
            let Some(i) = self.runs.iter().position(|r| std::ptr::eq(r, run)) else { return Ok(None) };
            return joined.map_in(i, bit - run.run_offset_bits);
        }
        Ok(self.trace.map_in(run.in_start + (bit - run.run_offset_bits)).map(|step| in_run(step, run)))
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
/// Every byte is either held or decoded when it is read. A run read through
/// its index reads its compressed bytes as it goes, so a read can find chunks
/// of the file missing: it reads as zeroes and reports them, and those are
/// chunks of the file, whichever document asks. A read the decoder fails on,
/// which a run that indexed cleanly should never give, reads as zeroes and
/// reports nothing.
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
    /// Bytes `at..at + out.len()`. When they could not be read, `out` is
    /// left as zeroes, and the error says whether chunks of the file are
    /// missing or the decoder refused.
    pub fn read(&self, at: u64, out: &mut [u8]) -> Result<(), ReadError> {
        let read = match self {
            SpaceSource::Whole(bytes) => match bytes.get(at as usize..at as usize + out.len()) {
                Some(have) => {
                    out.copy_from_slice(have);
                    Ok(())
                }
                None => Err(ReadError::Refused(Refusal::Failed)),
            },
            SpaceSource::Lazy(index) => index.read(at, out),
            SpaceSource::Joined(joined) => joined.read(at, out),
        };
        if read.is_err() {
            out.fill(0);
        }
        read
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
        match self.read(offset, out) {
            Err(ReadError::Missing(missing)) => missing,
            Ok(()) | Err(ReadError::Refused(_)) => Vec::new(),
        }
    }

    /// For a stream read through an index whose run is read from a store
    /// that keeps part of the file, as much output as the index keeps
    /// decoded: see [`Seekable::resident_limit`]. A reading over the stream,
    /// a recognised tab's, then reads a long run a window at a time and
    /// refuses a read that can never fit, as a reading of the file does.
    /// Nothing for bytes held whole.
    fn resident_limit(&self) -> Option<u64> {
        match self {
            SpaceSource::Whole(_) => None,
            SpaceSource::Lazy(index) => index.resident_limit(),
            SpaceSource::Joined(joined) => joined.resident_limit(),
        }
    }

    /// Another over the same bytes, which copies none of them: what an index
    /// over a run inside this stream reads the run through.
    fn share(&self) -> Option<Arc<dyn Source + Send + Sync>> {
        Some(Arc::new(self.clone()))
    }
}

/// A stream joined from parts, too large to hold whole, read a part at a
/// time. What [`SpaceSource::Joined`] reads.
///
/// What it is read from is held here: the bytes of each stored part, and the
/// compressed run of each packed one. A packed part small enough to decode
/// whole is decoded when it is read and kept in a cache like a stitched
/// space's; a larger one is read through its index, which reads its run
/// through its [`lazy::RunInput`] and may find chunks of the file missing.
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

    /// The least that any part read through its index keeps decoded at once,
    /// when such a part reads its run from a store that keeps part of the
    /// file. The other parts are held, or decoded from a copy of their run,
    /// and never wait for bytes.
    pub fn resident_limit(&self) -> Option<u64> {
        let indexed = self.parts.iter().filter_map(|(_, bytes)| match bytes {
            JoinedBytes::Indexed(index) => index.resident_limit(),
            JoinedBytes::Held(_) | JoinedBytes::Packed { .. } => None,
        });
        indexed.min()
    }

    fn part_at(&self, byte: u64) -> Option<usize> {
        let i = self.parts.partition_point(|p| p.0.end <= byte);
        self.parts.get(i).filter(|p| p.0.contains(&byte)).map(|_| i)
    }

    /// Bytes `at..at + out.len()`, from as many parts as they cover. A part
    /// read through its index that finds chunks of the file missing stops
    /// the read there and names them.
    pub fn read(&self, at: u64, out: &mut [u8]) -> Result<(), ReadError> {
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
                                return Err(Refusal::Failed.into());
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
    /// [`Space::map_out`] gives it for a joined stream held whole. A part
    /// read through its index may be missing chunks of the file, and the
    /// error is those chunks.
    pub fn map_out(&self, byte: u64) -> Result<Option<Step>, Vec<Missing>> {
        let Some(i) = self.part_at(byte) else { return Ok(None) };
        let (range, bytes) = &self.parts[i];
        let at = byte - range.start;
        let step = match bytes {
            JoinedBytes::Held(_) => {
                Some(Step { in_bits: 0..(range.end - range.start) * 8, out_bytes: 0..range.end - range.start, kind: StepKind::Stored })
            }
            JoinedBytes::Indexed(index) => index.map_out(at)?,
            JoinedBytes::Packed { run, codec } => self.part_trace(i, run, *codec).and_then(|t| t.map_out(at)),
        };
        Ok(step.map(|step| self.shifted(i, step)))
    }

    /// Which step of part `i` read bit `bit` of that part's run. Missing
    /// chunks as for [`Joined::map_out`].
    pub fn map_in(&self, i: usize, bit: u64) -> Result<Option<Step>, Vec<Missing>> {
        let Some((range, bytes)) = self.parts.get(i) else { return Ok(None) };
        let step = match bytes {
            JoinedBytes::Held(_) => {
                let bits = (range.end - range.start) * 8;
                (bit < bits).then(|| Step { in_bits: 0..bits, out_bytes: 0..range.end - range.start, kind: StepKind::Stored })
            }
            JoinedBytes::Indexed(index) => index.map_in(bit)?,
            JoinedBytes::Packed { run, codec } => self.part_trace(i, run, *codec).and_then(|t| t.map_in(bit)),
        };
        Ok(step.map(|step| self.shifted(i, step)))
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
/// An index whose run no handle reaches reads it through a copy of it, so
/// the total of those copies is capped, and the index used longest ago goes
/// first. An index over a handle owns no copy, and weighs nothing here (see
/// [`lazy::RunInput::owned_bytes`]).
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
        let held = |entries: &[(RunKey, Codec, Arc<Seekable>, u64)]| entries.iter().map(|e| e.2.input().owned_bytes()).sum::<u64>();
        while !self.entries.is_empty() && held(&self.entries) + index.input().owned_bytes() > INDEXED_RUN_BYTES as u64 {
            let oldest = (0..self.entries.len()).min_by_key(|&i| self.entries[i].3).expect("not empty");
            self.entries.swap_remove(oldest);
        }
        self.clock += 1;
        self.entries.push((key, codec, index, self.clock));
    }
}

/// The most the copies of compressed runs that indexes read through may
/// hold between them. The same as the most one run may be.
const INDEXED_RUN_BYTES: usize = crate::codec::CAP_BYTES;

/// Runs whose indexing pass has started and not finished, by where each run
/// is, carried on a step at a time on each go until they are done. Kept apart
/// from [`Indexes`], which holds only finished indexes: those are read with no
/// evaluator behind them, by a tab's source, and a half-made one must never
/// get there.
///
/// A pass that might still come to a run held whole holds up to the cap of
/// output, so there are places for only two. A pass that has started is never
/// let go of to make room for another: a call whose pass finds both places
/// taken carries on one of the passes holding them instead, and starts its
/// own once a place comes free. See [`super::Evaluator::wait_for_a_place`].
#[derive(Default)]
pub(super) struct Unfinished {
    /// What each place holds, with the stamp it was put there with (see
    /// [`Spaces::stamp`]).
    entries: Vec<(RunKey, Codec, Held, u64)>,
}

/// What one of the places in [`Unfinished`] holds.
enum Held {
    /// The compressed run of a pass that has not started yet, read so far a
    /// window at a time (see [`super::Evaluator::gather_into`]), and whether
    /// the pass is to start knowing the run is large. The pass takes its
    /// place before reading any of the run, so that it never finishes
    /// reading with nowhere to go, and the place holds the copy, as the pass
    /// would hold it. Only a run that needs more of the file than one window
    /// is read this way, and only for a pass that cannot read the run
    /// through a handle a window at a time ([`lazy::reads_in_windows`]); any
    /// other is read in one call.
    Gathering { bytes: Vec<u8>, known_large: bool },
    /// A pass that has started and not finished.
    Pass(lazy::Indexing),
    /// A pass that another call carried to its end, which came to a run held
    /// whole. Its bytes and trace wait here for a call over the same run to
    /// take them, since nothing else keeps a run held whole by where the run
    /// is: a `Decoded` node keeps one by its own path. They take a place as a
    /// pass does, since they are as large as a pass in its traced phase. A
    /// pass that comes to an index or a refusal leaves no such thing behind:
    /// those go to [`Indexes`] and to `refused_runs`, kept by the run.
    Whole(Vec<u8>, Trace),
}

/// How many places there are for unfinished passes.
const UNFINISHED_PASSES: usize = 2;

impl Unfinished {
    /// What the place for the run at `key` holds, taken out to be stepped or
    /// handed back.
    fn take(&mut self, key: RunKey, codec: Codec) -> Option<Held> {
        let i = self.entries.iter().position(|e| e.0 == key && e.1 == codec)?;
        Some(self.entries.swap_remove(i).2)
    }

    fn has_room(&self) -> bool {
        self.entries.len() < UNFINISHED_PASSES
    }

    /// Put a pass back to be carried on next go, or a run another call
    /// finished to be taken. A place has to be free: whatever took a pass out
    /// puts it back in the place it left, and a pass only starts in a free
    /// place.
    fn put(&mut self, key: RunKey, codec: Codec, held: Held, stamp: u64) {
        debug_assert!(self.has_room(), "no free place to put a pass in");
        self.entries.push((key, codec, held, stamp));
    }

    /// The pass with the fewest compressed bits left to read, taken out to be
    /// carried on by a call waiting for a place, since it is the one that
    /// frees a place soonest. A pass still reading its run has all of the
    /// run's bits to decode after what it has to read. None when every place
    /// holds a finished run.
    fn take_nearest_its_end(&mut self) -> Option<(RunKey, Codec, Held)> {
        let left = |key: RunKey, held: &Held| match held {
            Held::Gathering { bytes, .. } => Some(key.2.saturating_sub(bytes.len() as u64 * 8) + key.2),
            Held::Pass(pass) => Some(pass.progress().run_bits.saturating_sub(pass.progress().read_bits)),
            Held::Whole(..) => None,
        };
        let i = (0..self.entries.len()).filter(|&i| left(self.entries[i].0, &self.entries[i].2).is_some()).min_by_key(|&i| left(self.entries[i].0, &self.entries[i].2))?;
        let (key, codec, held, _) = self.entries.swap_remove(i);
        Some((key, codec, held))
    }

    /// Let go of the finished run put in its place longest ago, when every
    /// place holds one and no call has come for them: a call waiting for a
    /// place would otherwise have nothing to carry on and nothing to wait
    /// for. The run is unpacked again if it is asked for.
    fn drop_oldest_whole(&mut self) {
        let oldest = (0..self.entries.len()).filter(|&i| matches!(self.entries[i].2, Held::Whole(..))).min_by_key(|&i| self.entries[i].3);
        let i = oldest.expect("a place holding a finished run");
        self.entries.swap_remove(i);
    }
}

/// Deflate streams being measured to find where they end, for
/// [`Expr::StreamLen`](crate::template::Expr::StreamLen): the walks started
/// and not finished, carried on a step at a time on each go, and the ends
/// found. Each is kept by where the bytes measured are: the space, the bit
/// the stream starts at, and how many bits there are from there to the end
/// of the container or to the cap on a run's size, whichever is nearer.
///
/// Kept apart from [`Unfinished`], so that measuring one gzip member never
/// puts back the indexing pass of another, or the other way round.
///
/// The ends are kept as well as the walks, as a guard. If a go ran out after a
/// walk had finished and before the field it sizes was placed, the next go
/// would ask again, and without the end kept it would start the walk again
/// from the front: for a stream that takes more than one go to measure, every
/// go could then run out before the walk finished. The gzip template places
/// the field as soon as the walk finishes, but a template that puts
/// `StreamLen` inside a larger expression might not.
#[derive(Default)]
pub(super) struct Measures {
    /// Each walk with the bytes it has read so far, which grow a window at a
    /// time as it reaches them: see [`super::Evaluator::measure_steps`].
    walking: Vec<(RunKey, Codec, Vec<u8>, inflate::Measuring)>,
    /// Where each stream ended, in bytes from its start, or why it has no end
    /// to give. Oldest first.
    ended: std::collections::VecDeque<(RunKey, Codec, Result<u64, Refusal>)>,
}

/// How many places there are for unfinished measuring walks. Each holds the
/// bytes it has read, up to 64 MiB. As with passes, a walk that has started is
/// never let go of to make room for another: a call whose walk finds both
/// places taken carries one of those walks on instead.
const MEASURING_WALKS: usize = 2;

/// How many ends found are kept, the oldest going first. A few bytes each.
const ENDS_KEPT: usize = 64;

impl Measures {
    fn ended(&self, key: RunKey, codec: Codec) -> Option<Result<u64, Refusal>> {
        self.ended.iter().find(|e| e.0 == key && e.1 == codec).map(|e| e.2)
    }

    fn end(&mut self, key: RunKey, codec: Codec, found: Result<u64, Refusal>) {
        if self.ended.len() >= ENDS_KEPT {
            self.ended.pop_front();
        }
        self.ended.push_back((key, codec, found));
    }

    /// The walk over the stream at `key`, taken out to be stepped.
    fn take(&mut self, key: RunKey, codec: Codec) -> Option<(Vec<u8>, inflate::Measuring)> {
        let i = self.walking.iter().position(|e| e.0 == key && e.1 == codec)?;
        let (_, _, data, walk) = self.walking.swap_remove(i);
        Some((data, walk))
    }

    fn has_room(&self) -> bool {
        self.walking.len() < MEASURING_WALKS
    }

    /// Put a walk back to be carried on next go. As in [`Unfinished::put`],
    /// a place has to be free.
    fn put(&mut self, key: RunKey, codec: Codec, data: Vec<u8>, walk: inflate::Measuring) {
        debug_assert!(self.has_room(), "no free place to put a walk in");
        self.walking.push((key, codec, data, walk));
    }

    /// The walk with the fewest bits left to read, taken out to be carried on
    /// by a call waiting for a place. A walk stops where its stream ends,
    /// which may be well before the end of the bytes it measures, so this is
    /// only a guess at which walk frees a place soonest.
    fn take_nearest_its_end(&mut self) -> Option<(RunKey, Codec, Vec<u8>, inflate::Measuring)> {
        let i = (0..self.walking.len()).min_by_key(|&i| self.walking[i].0 .2.saturating_sub(self.walking[i].3.read_bits()))?;
        Some(self.walking.swap_remove(i))
    }

    fn is_empty(&self) -> bool {
        self.walking.is_empty() && self.ended.is_empty()
    }
}

/// Copies of runs being read a window at a time, kept from one go to the
/// next, for the reads of a whole run that are not an index pass's: a run of
/// a codec that cannot be indexed, a stitched stream's packed part, the run
/// a checksum covers, a joined stream's stored part, and an index pass's run
/// in a go with no allowance or of a codec whose pass is not stepped. See
/// [`super::Evaluator::gather_run`]. An index pass stepped across goes reads
/// its run into its own place in [`Unfinished`] instead.
///
/// Each copy is kept by where its run is. When a gather starts, the run's
/// whole length is set aside against [`GATHER_BYTES`], so a gather that has
/// started can always finish and is never let go of to make room for
/// another. A gather that would start past the total waits, carrying on the
/// others, as an index pass waits for a place.
#[derive(Default)]
pub(super) struct Gathers {
    /// The copies not being read into right now, each with the stamp it was
    /// put back with. One that is complete was finished by a call waiting
    /// for room, and waits for its own call to take it.
    entries: Vec<(RunKey, Vec<u8>, u64)>,
    /// Every gather started and not yet handed over: those in `entries`, and
    /// those being read into further up the stack.
    started: Vec<RunKey>,
    clock: u64,
}

/// The most bytes the gathers started and not yet handed over may come to
/// between them: the cap, which is the most one run read whole may be.
const GATHER_BYTES: u64 = crate::codec::CAP_BYTES as u64;

/// The most bytes one window of a gather reads, in one call. A quarter of
/// what the file's source keeps when that is less, so that a window always
/// fits beside what else a go reads.
const GATHER_WINDOW: u64 = 1 << 20;

/// How many bytes copied out of the file are charged as one element of a
/// go's allowance: 64 elements a mebibyte. A go of 5,000 elements, which is
/// what the web app asks for, copies at most about 78 MiB this way, more
/// than one run may be, so a gather whose chunks are all loaded ends in one
/// go. Copying a mebibyte out of the chunk store took about 1.4 ms native on
/// 2026-11-26 (41 MB in 56 ms), against about 4 ms for the 512 elements of
/// one index step.
pub(super) const GATHER_BYTES_PER_ELEMENT: u64 = 16 << 10;

/// The bytes past where a measuring step starts that it may read: its own
/// 1 Mibit, a stored block it is not stopped in (65,541 bytes with its
/// header, padding and lengths), and one more code. A walk whose bytes reach
/// that far never runs out of them in a step.
const MEASURE_AHEAD_BYTES: u64 = MEASURE_STEP_BITS / 8 + 65_541 + 1024;

fn target(key: RunKey) -> u64 {
    key.2.div_ceil(8)
}

impl Gathers {
    fn take(&mut self, key: RunKey) -> Option<Vec<u8>> {
        let i = self.entries.iter().position(|e| e.0 == key)?;
        Some(self.entries.swap_remove(i).1)
    }

    fn put(&mut self, key: RunKey, bytes: Vec<u8>) {
        self.clock += 1;
        self.entries.push((key, bytes, self.clock));
    }

    /// Whether a gather of the run at `key` may start beside the others
    /// within `cap` bytes, and if so, set its length aside.
    fn admit(&mut self, key: RunKey, cap: u64) -> bool {
        let set_aside: u64 = self.started.iter().map(|&k| target(k)).sum();
        if set_aside + target(key) > cap {
            return false;
        }
        self.started.push(key);
        true
    }

    /// The gather at `key` handed over or given up: its length is no longer
    /// set aside.
    fn release(&mut self, key: RunKey) {
        if let Some(i) = self.started.iter().position(|&k| k == key) {
            self.started.swap_remove(i);
        }
        if let Some(i) = self.entries.iter().position(|e| e.0 == key) {
            self.entries.swap_remove(i);
        }
    }

    /// The unfinished copy with the fewest bytes left to read, taken out to
    /// be carried on by a gather waiting for room.
    fn take_nearest_its_end(&mut self) -> Option<(RunKey, Vec<u8>)> {
        let left = |e: &(RunKey, Vec<u8>, u64)| target(e.0) - (e.1.len() as u64).min(target(e.0));
        let i = (0..self.entries.len()).filter(|&i| left(&self.entries[i]) > 0).min_by_key(|&i| left(&self.entries[i]))?;
        let (key, bytes, _) = self.entries.swap_remove(i);
        Some((key, bytes))
    }

    /// Let go of the complete copy put back longest ago, which a call waiting
    /// for room finished and its own call has not come for. Its run is read
    /// again if it is asked for. False when there is none.
    fn drop_oldest_complete(&mut self) -> bool {
        let oldest = (0..self.entries.len()).filter(|&i| self.entries[i].1.len() as u64 >= target(self.entries[i].0)).min_by_key(|&i| self.entries[i].2);
        let Some(i) = oldest else { return false };
        let key = self.entries[i].0;
        self.release(key);
        true
    }

    fn is_empty(&self) -> bool {
        self.started.is_empty()
    }
}

/// How much output one step of an indexing pass decodes, and what it is
/// charged against a go's allowance. 512 bytes an element: a go of 5,000
/// elements, which is what the web app asks for, decodes about 2.5 MiB. In
/// wasm that took about 18 ms on 2026-10-26, with the trace on or off: 1.25
/// MiB took 9 ms. Less than the twentieth of a second a go is meant to take,
/// since a listing drawing a frame asks several things that each carry the
/// same pass on.
pub(super) const INDEX_STEP_BYTES: u64 = 256 << 10;
pub(super) const INDEX_STEP_ELEMENTS: u64 = INDEX_STEP_BYTES / 512;

/// The most bits of the run one step of an indexing pass reads, whatever
/// output it has made. Deflate's empty blocks are ten bits each and make no
/// output, so without this a run of them is one step however long it is.
///
/// As many bits as the step's share of output has, 2 Mibit (256 KiB of the
/// run), so a step over a run whose output is at least as large as the run
/// still ends at its share of output, as before: stored blocks and packed
/// photos as well as text. Only a run that makes less output than it takes up
/// is stepped by its bits. Measured native on 2026-11-26, a step of 2 Mibit
/// of empty fixed blocks took up to 31 ms and a go of ten steps up to 250 ms,
/// most of it the trace each block adds while the run may still be held
/// whole; past the cap, with the trace off, 9 ms and 90 ms. Half as many bits
/// would halve those, and would double the steps over random bytes, which
/// are about 0.5 ms each.
const INDEX_STEP_BITS: u64 = INDEX_STEP_BYTES * 8;

/// How many bits of a deflate stream one step of measuring it reads, and how
/// many bits are charged as one element of a go's allowance.
///
/// Counted in bits read, where the indexing pass counts output, because a
/// decoder that only counts the output does about the same work for each code
/// it reads, whether the code is a literal or a match copying 258 bytes. So
/// 64 MiB of the Rubin raw's long matches count in 5 ms native, and 64 MiB of
/// CSV-like text in about 500 ms. A code is at least one bit long, so a step of
/// 1 Mibit reads at most about a million codes: 3.5 ms native on CSV-like
/// text, 8 ms on literals of one or two bits each. A step is charged after it
/// is done, for the bits it read, so that measuring a short stream costs a go
/// next to nothing.
const MEASURE_STEP_BITS: u64 = 1 << 20;
const MEASURE_BITS_PER_ELEMENT: u64 = 1024;

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
    /// Why each packed part that failed did so, by part index: it would not
    /// decode, or it decoded to another length than the file says. A read
    /// that reaches one of these parts again fails with the same message
    /// without decoding it again, so a listing scrolling over a damaged part
    /// decodes it once. Dropped with the space when the spaces are forgotten.
    pub(super) troubles: std::cell::RefCell<FxHashMap<usize, super::stitch::PartTrouble>>,
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
    /// Why the index pass over a run refused it, by where the run is and its
    /// codec, so that a run past `Limits::lazy` or one that will not decode
    /// is not decoded again, up to a gigabyte of output, by every read that
    /// reaches it. Kept beside the indexes, and dropped with them. A cell for
    /// the same reason.
    refused_runs: std::cell::RefCell<FxHashMap<(RunKey, Codec), Refusal>>,
    /// How many index passes have been started, for a test that wants to see
    /// a pass is not made twice.
    #[cfg(test)]
    pub(super) passes: std::cell::Cell<u32>,
    /// How many packed runs have been decoded whole outside an index pass, for
    /// a test that wants to see a part is not decoded twice.
    #[cfg(test)]
    pub(super) decodes: std::cell::Cell<u32>,
    /// Deflate streams being measured to find where they end, and the ends
    /// found.
    measures: Measures,
    /// A count of what has been put in `Unfinished`, so that when both its
    /// places hold a finished run, the one put there first is let go of.
    stamps: std::cell::Cell<u64>,
    /// Runs being read a window at a time. A cell, since reads reach it.
    gathers: std::cell::RefCell<Gathers>,
    /// The most the gathers may set aside between them: [`GATHER_BYTES`],
    /// made small by a test.
    gather_cap: u64,
    /// Joined streams being made ready to open as documents of their own, a
    /// part at a time. A cell, since the walk is carried on from inside
    /// reads of other streams waiting for a place.
    pub(super) joins: std::cell::RefCell<super::stitch::Joins>,
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
            refused_runs: Default::default(),
            #[cfg(test)]
            passes: Default::default(),
            #[cfg(test)]
            decodes: Default::default(),
            measures: Default::default(),
            stamps: Default::default(),
            gathers: Default::default(),
            gather_cap: GATHER_BYTES,
            joins: Default::default(),
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

    /// Why the index pass over the run at `key`, packed with `codec`, refused
    /// it, if one did.
    pub(super) fn run_refusal(&self, key: RunKey, codec: Codec) -> Option<Refusal> {
        self.refused_runs.borrow().get(&(key, codec)).copied()
    }

    /// How many runs have a refusal kept, for tests that check a run waiting
    /// for bytes never has one.
    #[cfg(test)]
    pub(super) fn refusals(&self) -> usize {
        self.refused_runs.borrow().len()
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
        self.backings.push(Backing::Stitched(Box::new(Stitch { path: path.to_vec(), parts, len_bytes, cache, troubles: Default::default() })));
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

    /// Whether any stream has been opened, indexed or measured at all. Most
    /// files hold none, and the sweep that drops decoded nodes should cost them
    /// nothing. A gzip member is measured before anything opens it, and an
    /// index pass can be under way before its stream has opened, so those
    /// count too: an edit has to drop them.
    pub(super) fn any(&self) -> bool {
        !self.opened.is_empty()
            || !self.measures.is_empty()
            || !self.unfinished.borrow().entries.is_empty()
            || !self.indexes.borrow().entries.is_empty()
            || !self.refused_runs.borrow().is_empty()
            || !self.gathers.borrow().is_empty()
            || !self.joins.borrow().is_empty()
    }

    /// The most the gathers may set aside between them, made small for a
    /// test that wants runs too long to gather beside each other.
    #[cfg(test)]
    pub(super) fn set_gather_cap(&mut self, bytes: u64) {
        self.gather_cap = bytes;
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
        self.refused_runs.borrow_mut().clear();
        self.measures = Measures::default();
        *self.gathers.borrow_mut() = Gathers::default();
        *self.joins.borrow_mut() = Default::default();
    }

    /// A number larger than any handed out before, for the pass being put
    /// back now.
    fn stamp(&self) -> u64 {
        let stamp = self.stamps.get() + 1;
        self.stamps.set(stamp);
        stamp
    }
}

impl super::Evaluator {
    /// The run at `key`, packed with `codec`, decoded whole when it fits under
    /// the cap and indexed when it does not: [`lazy::decode_or_index`], done
    /// a step at a time. Each step is charged to this go's allowance, and a go
    /// that runs out hands back `Busy` with the pass kept, to be carried on
    /// from where it stopped the next time anything asks for the same run.
    /// `Ok(Err)` is the run refusing, as it would refuse in one go. A refusal
    /// is kept by where the run is, and handed back from then on without
    /// another pass.
    /// `known_large` is a caller that knows the run comes to more than the
    /// cap, which lets the pass skip the part that might have been a run held
    /// whole: see [`lazy::Indexing::known_large`].
    ///
    /// Deflate, zlib, gzip and zstd are indexed a step at a time. An xz run is
    /// decoded or laid out in one go, with nothing charged, as
    /// [`lazy::decode_or_index`] does it: see [`lazy::can_step`].
    ///
    /// Takes the evaluator shared, since a read of a stitched stream reaches
    /// here, and reads do.
    pub(super) fn index_run<S: Source>(&self, doc: &Document<S>, key: RunKey, codec: Codec, known_large: bool) -> super::R<Result<Decoded, Refusal>> {
        if let Some(why) = self.spaces.run_refusal(key, codec) {
            return Ok(Err(why));
        }
        let done = self.index_pass(doc, key, codec, known_large)?;
        if let Err(why) = done {
            self.spaces.refused_runs.borrow_mut().insert((key, codec), why);
        }
        Ok(done)
    }

    /// The pass behind [`Self::index_run`], for a run with no refusal kept.
    ///
    /// A deflate, zlib, gzip or zstd pass reads its run through a handle onto
    /// the source the run is in, a window at a time, when a handle reaches
    /// the run and the source keeps enough for a window
    /// ([`lazy::reads_in_windows`]). It holds no copy of the run, and starts
    /// at once: a step whose window is missing chunks hands back `Pending`
    /// with them, and the pass waits in its place in `Unfinished`.
    ///
    /// Any other pass reads a copy of the run. A run that needs more of the
    /// file than one window ([`Self::gathers`]) is read a window at a time
    /// across goes before such a pass starts. A pass stepped across goes
    /// takes its place in `Unfinished` first and reads its run into that
    /// place ([`Held::Gathering`]); any other reads it through
    /// [`Self::gather_run`].
    fn index_pass<S: Source>(&self, doc: &Document<S>, key: RunKey, codec: Codec, known_large: bool) -> super::R<Result<Decoded, Refusal>> {
        let held = self.spaces.unfinished.borrow_mut().take(key, codec);
        if let Some(held) = held {
            return self.carry_on(doc, key, codec, held, true);
        }
        if key.2 / 8 > crate::codec::CAP_BYTES as u64 {
            return Ok(Err(Refusal::TooLarge));
        }
        let limits = self.spaces.lazy_limits;
        let windowed = self.shared_input(doc, key).filter(|input| lazy::reads_in_windows(codec, input, &limits));
        // A go with no allowance carries its pass to the end before it hands
        // back, so it never puts one back for that and needs no place,
        // unless the pass reads a source that can find chunks missing. A go
        // that has spent its allowance already starts nothing: the pass would
        // take a place having done nothing.
        let placed = lazy::can_step(codec) && (self.go.limited() || windowed.as_ref().is_some_and(|input| input.resident_limit().is_some()));
        if placed {
            self.go.charge(0)?;
            self.wait_for_a_place(doc)?;
            if windowed.is_none() && self.gathers(doc, key.0, key.1, key.2) {
                return self.carry_on(doc, key, codec, Held::Gathering { bytes: Vec::new(), known_large }, true);
            }
        }
        if let Some(input) = windowed {
            return self.start_pass(key, codec, lazy::Indexing::through(codec, input, limits), known_large);
        }
        // What the index will read the run through once it is made, and the
        // pass's own copy of the run, which goes when the pass does. Where
        // no handle reaches the run, the input is that copy.
        let input = match self.run_input(doc, key)? {
            Ok(input) => input,
            Err(why) => return Ok(Err(why)),
        };
        let packed = match input.held_copy() {
            Some(copy) => copy.clone(),
            None => match self.gather_run(doc, key.0, key.1, key.2)? {
                Ok(packed) => Arc::new(packed),
                Err(why) => return Ok(Err(why)),
            },
        };
        if !lazy::can_step(codec) {
            #[cfg(test)]
            self.spaces.passes.set(self.spaces.passes.get() + 1);
            return Ok(lazy::decode_or_index_through(codec, packed, input, limits));
        }
        self.start_pass(key, codec, lazy::Indexing::from_copy(codec, packed, input, limits), known_large)
    }

    /// Step a pass just made over the run at `key`, told the run is large
    /// when `known_large`, or hand back the refusal it was made with.
    fn start_pass(&self, key: RunKey, codec: Codec, made: Result<lazy::Indexing, Refusal>, known_large: bool) -> super::R<Result<Decoded, Refusal>> {
        #[cfg(test)]
        self.spaces.passes.set(self.spaces.passes.get() + 1);
        let pass = match made {
            Ok(pass) if known_large => pass.known_large(),
            Ok(pass) => pass,
            Err(why) => return Ok(Err(why)),
        };
        self.index_steps(key, codec, pass, true)
    }

    /// Where an index of the run at `key` reads the run from once it is
    /// made: a handle that copies nothing when one reaches the run, and
    /// otherwise a copy of the run. See `docs/DESIGN-lazy-compressed-input.md`,
    /// "Where a `RunInput` comes from".
    ///
    /// - A run of the file, in a stretch no edit touched, is read through a
    ///   handle onto the file's source ([`Source::share`]), from where the
    ///   stretch is in the source ([`Document::source_offset_of`]). In the
    ///   browser that is the file's chunk store, which the file's tab keeps
    ///   anyway.
    /// - A run in a space held whole is read from the space's bytes.
    /// - A run in a space read through an index is read through that space's
    ///   source, which decodes it as it is read.
    /// - Any other run is gathered whole ([`Self::gather_run`]) and the copy
    ///   kept: a run in a stitched space, in a stretch an edit touched, or in
    ///   a source that gives no handle, such as a [`MemSource`]. A run longer
    ///   than the cap is refused as [`Refusal::TooLarge`] before anything of
    ///   it is read. A handle is never refused here.
    ///
    /// A handle made before an edit reads the source it was given, at the
    /// place it was given, and never the edited bytes. Every edit drops the
    /// indexes that hold such handles ([`Spaces::forget`]).
    ///
    /// [`MemSource`]: crate::source::MemSource
    pub(super) fn run_input<S: Source>(&self, doc: &Document<S>, key: RunKey) -> super::R<Result<lazy::RunInput, Refusal>> {
        if let Some(input) = self.shared_input(doc, key) {
            return Ok(Ok(input));
        }
        if key.2 / 8 > crate::codec::CAP_BYTES as u64 {
            return Ok(Err(Refusal::TooLarge));
        }
        Ok(self.gather_run(doc, key.0, key.1, key.2)?.map(|bytes| lazy::RunInput::held(Arc::new(bytes))))
    }

    /// The handle [`Self::run_input`] makes for the run at `key`, when one
    /// reaches it. Reads nothing.
    #[inline(never)]
    fn shared_input<S: Source>(&self, doc: &Document<S>, (space, at, n): RunKey) -> Option<lazy::RunInput> {
        if n % 8 != 0 {
            return None;
        }
        let (source, from): (Arc<dyn Source + Send + Sync>, u64) = match space {
            0 => {
                let from = doc.source_offset_of(at, n)?;
                (doc.source().share()?, from)
            }
            _ if at % 8 != 0 => return None,
            _ => match self.spaces.buf(space) {
                Some(bytes) => (Arc::new(crate::source::ArcSource(bytes.clone())), at / 8),
                None => (Arc::new(SpaceSource::Lazy(self.spaces.lazy(space)?.clone())), at / 8),
            },
        };
        Some(lazy::RunInput::new(source, from, n / 8))
    }

    /// Carry on what the place for the run at `key` holds, taken out of it:
    /// read the rest of a run being gathered and start its pass, then step
    /// the pass, until it finishes or this go hands back. Whatever has not
    /// finished goes back in its place. `own` is as in [`Self::index_steps`].
    fn carry_on<S: Source>(&self, doc: &Document<S>, key: RunKey, codec: Codec, held: Held, own: bool) -> super::R<Result<Decoded, Refusal>> {
        let pass = match held {
            Held::Pass(pass) => pass,
            Held::Whole(bytes, trace) => return Ok(Ok(Decoded::Whole(bytes, trace))),
            Held::Gathering { mut bytes, known_large } => {
                if let Err(e) = self.gather_into(doc, key, &mut bytes, own) {
                    // A run that will not read frees its place, and the call
                    // that reads it meets the same failure.
                    if e.interrupted() {
                        let stamp = self.spaces.stamp();
                        self.spaces.unfinished.borrow_mut().put(key, codec, Held::Gathering { bytes, known_large }, stamp);
                    }
                    return Err(e);
                }
                #[cfg(test)]
                self.spaces.passes.set(self.spaces.passes.get() + 1);
                // As in `index_pass`: the index reads a handle when one
                // reaches the run, and otherwise the copy just gathered,
                // which the pass reads either way.
                let packed = Arc::new(bytes);
                let input = self.shared_input(doc, key).unwrap_or_else(|| lazy::RunInput::held(packed.clone()));
                match lazy::Indexing::from_copy(codec, packed, input, self.spaces.lazy_limits) {
                    Ok(pass) if known_large => pass.known_large(),
                    Ok(pass) => pass,
                    Err(why) => return Ok(Err(why)),
                }
            }
        };
        self.index_steps(key, codec, pass, own)
    }

    /// Step `pass`, over the run at `key`, until it finishes, this go runs
    /// out, or a step's window of the run is missing chunks of the file. A go
    /// that runs out puts the pass back in its place and hands back `Busy`,
    /// and a step missing chunks puts it back and hands back `Pending` with
    /// them, which is never a refusal. Either way it notes the pass as the
    /// one the call waits on when the pass is the call's own (`own`), so
    /// that [`super::Evaluator::unpacking`] says how far it has got, and
    /// otherwise notes that the call waits for a place with no progress of
    /// its own to give.
    fn index_steps(&self, key: RunKey, codec: Codec, mut pass: lazy::Indexing, own: bool) -> super::R<Result<Decoded, Refusal>> {
        loop {
            let waits = match self.go.charge(INDEX_STEP_ELEMENTS) {
                Err(busy) => busy,
                Ok(()) => match pass.step(INDEX_STEP_BYTES, INDEX_STEP_BITS) {
                    Ok(Stepped::Working) => continue,
                    Ok(Stepped::NeedsInput(missing)) => super::EvalError::Pending(missing),
                    Ok(Stepped::Done(decoded)) => return Ok(Ok(decoded)),
                    Err(why) => return Ok(Err(why)),
                },
            };
            if own {
                self.go.ran_out_in(super::Unpacking::of(key, pass.progress()));
            } else {
                self.go.ran_out_waiting();
            }
            let stamp = self.spaces.stamp();
            self.spaces.unfinished.borrow_mut().put(key, codec, Held::Pass(pass), stamp);
            return Err(waits);
        }
    }

    /// Wait for a free place in `Unfinished`, for a pass about to start.
    ///
    /// While both places are taken, this go carries on the pass nearest its
    /// end, which is another call's, until the go runs out and hands back
    /// `Busy`, until the pass waits for bytes of the file (`Pending`, naming
    /// that pass's chunks), or until that pass finishes. What a pass finished
    /// here comes to is kept where its own call will find it: an index in
    /// `Indexes` and a refusal in `refused_runs`, which free its place, or a
    /// run held whole in the place itself. Every go that reaches a pass so
    /// moves one of the unfinished passes on, so they all finish, and a pass
    /// nobody asks for any more is finished by the calls waiting behind it.
    ///
    /// When both places hold runs finished this way and not yet taken, the
    /// one put there first is let go of, since there is then nothing to carry
    /// on and nothing would ever free a place.
    fn wait_for_a_place<S: Source>(&self, doc: &Document<S>) -> super::R<()> {
        loop {
            if self.spaces.unfinished.borrow().has_room() {
                return Ok(());
            }
            let nearest = self.spaces.unfinished.borrow_mut().take_nearest_its_end();
            let Some((other, codec, held)) = nearest else {
                self.spaces.unfinished.borrow_mut().drop_oldest_whole();
                continue;
            };
            match self.carry_on(doc, other, codec, held, false) {
                Ok(Ok(Decoded::Indexed(index))) => self.spaces.keep_index(other, codec, Arc::new(index)),
                Ok(Ok(Decoded::Whole(bytes, trace))) => {
                    let stamp = self.spaces.stamp();
                    self.spaces.unfinished.borrow_mut().put(other, codec, Held::Whole(bytes, trace), stamp);
                }
                Ok(Err(why)) => {
                    self.spaces.refused_runs.borrow_mut().insert((other, codec), why);
                }
                Err(e) if e.interrupted() => return Err(e),
                // The other run would not read. Its own call is told it would
                // not unpack, rather than this call being failed for it.
                Err(_) => {
                    self.spaces.refused_runs.borrow_mut().insert((other, codec), Refusal::Failed);
                }
            }
        }
    }

    /// How many bytes one window of a gather reads at once: [`GATHER_WINDOW`],
    /// or a quarter of what the file's source keeps when that is less.
    pub(super) fn gather_window<S: Source>(&self, doc: &Document<S>) -> u64 {
        doc.resident_limit().map_or(GATHER_WINDOW, |keeps| (keeps / 4).clamp(1, GATHER_WINDOW))
    }

    /// Whether a read of `n` bits at bit `at` of `space` is read a window at
    /// a time rather than in one call: it needs more of the file at once than
    /// one window, and the file's source keeps only some of the file. A
    /// source that answers any read whole is read in one call, as always.
    pub(super) fn gathers<S: Source>(&self, doc: &Document<S>, space: u32, at: u64, n: u64) -> bool {
        doc.resident_limit().is_some() && self.file_bytes_at_once(space, at, n) > self.gather_window(doc)
    }

    /// Read on into `bytes`, a copy of the run at `key` read so far, a window
    /// at a time, until it holds the whole run. Each window is charged to the
    /// go before it is read, and a window whose bytes have not arrived hands
    /// back `Pending` with them. `bytes` keeps everything read before that, so
    /// the next go carries on from there. `own` is as in
    /// [`Self::index_steps`]: a go that runs out reading another call's run
    /// notes that its call waits with no progress of its own to give.
    ///
    /// Windows end on multiples of the window in the space the run is in, so
    /// that in the file each window covers whole chunks of the chunk store
    /// and no chunk is asked for by two windows.
    pub(super) fn gather_into<S: Source>(&self, doc: &Document<S>, (space, at, n): RunKey, bytes: &mut Vec<u8>, own: bool) -> super::R<()> {
        let window = self.gather_window(doc);
        loop {
            let have = bytes.len() as u64 * 8;
            if have >= n {
                return Ok(());
            }
            let from = at + have;
            let take = match from % 8 {
                0 => ((from / 8 / window + 1) * window * 8 - from).min(n - have),
                _ => (window * 8).min(n - have),
            };
            if let Err(busy) = self.go.charge(take.div_ceil(8).div_ceil(GATHER_BYTES_PER_ELEMENT)) {
                if !own {
                    self.go.ran_out_waiting();
                }
                return Err(busy);
            }
            bytes.extend_from_slice(&self.read_in(doc, space, from, take)?);
        }
    }

    /// `n` bits at bit `at` of `space`, for a caller that reads a whole run:
    /// in one call when that needs no more of the file than one window, and
    /// otherwise a window at a time across goes, into a copy kept in
    /// [`Gathers`] until it is complete and handed over. A go that runs out
    /// or waits for bytes hands back `Busy` or `Pending` with the copy kept,
    /// and the next call for the same run carries on from where it stopped.
    ///
    /// `Ok(Err)` is [`Refusal::TooMuchAtOnce`]: the run cannot be gathered
    /// beside what other gathers have set aside, and nothing can make room.
    /// That is only ever a run longer than the gathers may set aside
    /// together, or one read inside the read of another run that has set
    /// aside the rest. No run up to the cap is refused that way on its own.
    pub(super) fn gather_run<S: Source>(&self, doc: &Document<S>, space: u32, at: u64, n: u64) -> super::R<Result<Vec<u8>, Refusal>> {
        if !self.gathers(doc, space, at, n) {
            return Ok(Ok(self.read_in(doc, space, at, n)?));
        }
        let key = (space, at, n);
        let started = self.spaces.gathers.borrow_mut().take(key);
        let mut bytes = match started {
            Some(bytes) => bytes,
            None => match self.room_to_gather(doc, key)? {
                Ok(()) => Vec::new(),
                Err(why) => return Ok(Err(why)),
            },
        };
        match self.gather_into(doc, key, &mut bytes, true) {
            Ok(()) => {
                self.spaces.gathers.borrow_mut().release(key);
                Ok(Ok(bytes))
            }
            Err(e) => {
                let mut gathers = self.spaces.gathers.borrow_mut();
                if e.interrupted() {
                    gathers.put(key, bytes);
                } else {
                    gathers.release(key);
                }
                Err(e)
            }
        }
    }

    /// Set the run at `key`'s length aside for a gather about to start,
    /// waiting for room as an index pass waits for a place
    /// ([`Self::wait_for_a_place`]): this go carries on the unfinished gather
    /// nearest its end, which stays complete in `Gathers` for its own call to
    /// take, and when every copy there is complete, the one put back longest
    /// ago is let go of.
    fn room_to_gather<S: Source>(&self, doc: &Document<S>, key: RunKey) -> super::R<Result<(), Refusal>> {
        loop {
            if self.spaces.gathers.borrow_mut().admit(key, self.spaces.gather_cap) {
                return Ok(Ok(()));
            }
            let nearest = self.spaces.gathers.borrow_mut().take_nearest_its_end();
            if let Some((other, mut bytes)) = nearest {
                match self.gather_into(doc, other, &mut bytes, false) {
                    Ok(()) => self.spaces.gathers.borrow_mut().put(other, bytes),
                    Err(e) if e.interrupted() => {
                        self.spaces.gathers.borrow_mut().put(other, bytes);
                        return Err(e);
                    }
                    // The other run would not read. Its own call meets that
                    // when it reads it again.
                    Err(_) => self.spaces.gathers.borrow_mut().release(other),
                }
                continue;
            }
            if !self.spaces.gathers.borrow_mut().drop_oldest_complete() {
                return Ok(Err(Refusal::TooMuchAtOnce));
            }
        }
    }

    /// How many bytes the stream packed with `codec` at the front of the
    /// `key.2` bits at bit `key.1` of space `key.0` takes, which is what
    /// [`crate::codec::stream_len`] says of those bytes. `Ok(Err)` is the
    /// refusal it gives.
    ///
    /// A deflate stream is measured a step at a time ([`inflate::Measuring`]),
    /// each step charged to this go's allowance, and a go that runs out hands
    /// back `Busy` with the walk kept, to be carried on from where it stopped
    /// the next time the same stream is measured. What it finds is kept as
    /// well. The walk reads its bytes as it reaches them, a window at a time
    /// (see [`Self::measure_steps`]), so a short stream costs a short read
    /// however far the bytes it is measured in go on. Any other codec is
    /// answered in one go, from the bytes gathered whole.
    ///
    /// A walk that has started is never let go of to make room for another.
    /// A deflate stream whose walk finds both places taken waits for one, as
    /// an index pass does in [`Self::wait_for_a_place`]: this go carries on
    /// the walk nearest its end, whose end is kept in `Measures` for its own
    /// call. A walk that is never put back needs no place: one in a go with
    /// no allowance, over a source that answers any read whole.
    pub(super) fn stream_end<S: Source>(&mut self, doc: &Document<S>, key: RunKey, codec: Codec) -> super::R<Result<u64, Refusal>> {
        if let Some(found) = self.spaces.measures.ended(key, codec) {
            return Ok(found);
        }
        if let Some((data, walk)) = self.spaces.measures.take(key, codec) {
            return self.measure_steps(doc, key, codec, data, walk, true);
        }
        if codec != Codec::Deflate {
            let data = match self.gather_run(doc, key.0, key.1, key.2)? {
                Ok(data) => data,
                Err(why) => return Ok(Err(why)),
            };
            return Ok(crate::codec::stream_len(codec, &data));
        }
        if self.go.limited() || doc.resident_limit().is_some() {
            self.go.charge(0)?;
            while !self.spaces.measures.has_room() {
                let (other, other_codec, data, walk) = self.spaces.measures.take_nearest_its_end().expect("both places hold a walk");
                // Whatever it finds is kept in `Measures` for its own call.
                let _ = self.measure_steps(doc, other, other_codec, data, walk, false)?;
            }
        }
        let walk = inflate::Measuring::new(self.spaces.lazy_limits.measure);
        self.measure_steps(doc, key, codec, Vec::new(), walk, true)
    }

    /// Step the walk measuring the stream at `key` until it finds where the
    /// stream ends, or why it has no end to give, or until this go runs out
    /// or waits for bytes. What it finds is kept in `Measures`. A go that
    /// hands back puts the walk back, noting it as the one it ran out in when
    /// the walk is the call's own (`own`), as `index_steps` does.
    ///
    /// `data` is the bytes from the stream's start read so far. Before each
    /// step they are read on, a window at a time and each window charged,
    /// until they reach [`MEASURE_AHEAD_BYTES`] past where the step starts,
    /// or the end of what is measured. Those bytes are always enough for the
    /// step. As a guard, a step refused as failed while there are bytes still
    /// to read is taken once more with a window more: a step that ran out of
    /// bytes would then succeed, and one over a stream that will not decode
    /// fails again. Any other refusal, such as output past the cap, is final
    /// at once. So a walk reads about a step past where its stream ends, or
    /// past where it was refused.
    fn measure_steps<S: Source>(&mut self, doc: &Document<S>, key: RunKey, codec: Codec, mut data: Vec<u8>, mut walk: inflate::Measuring, own: bool) -> super::R<Result<u64, Refusal>> {
        let total = key.2 / 8;
        let window = self.gather_window(doc);
        // A step that failed with bytes still to read, to be taken again.
        let mut short = false;
        loop {
            let reach = match short {
                true => data.len() as u64 + window,
                false => walk.read_bits() / 8 + MEASURE_AHEAD_BYTES,
            };
            let reach = reach.min(total);
            let mut stopped = self.go.charge(0).err();
            while stopped.is_none() && (data.len() as u64) < reach {
                let have = data.len() as u64;
                // Ending on a multiple of the window, as a gather's windows
                // do. The stream starts on a byte.
                let at = key.1 / 8 + have;
                let take = (reach - have).min((at / window + 1) * window - at);
                if let Err(busy) = self.go.charge(take.div_ceil(GATHER_BYTES_PER_ELEMENT)) {
                    stopped = Some(busy);
                    break;
                }
                match self.read_in(doc, key.0, key.1 + have * 8, take * 8) {
                    Ok(got) => data.extend_from_slice(&got),
                    Err(e) if e.interrupted() => stopped = Some(e),
                    Err(e) => return Err(e),
                }
            }
            if let Some(e) = stopped {
                if matches!(e, super::EvalError::Busy { .. }) {
                    if own {
                        // The share read is of the bytes the walk measures,
                        // which run from the start of the stream to the end
                        // of the container or the cap.
                        let progress = lazy::Progress { read_bits: walk.read_bits(), run_bits: key.2, unpacked: walk.unpacked() };
                        self.go.ran_out_in(super::Unpacking::of(key, progress));
                    } else {
                        self.go.ran_out_waiting();
                    }
                }
                self.spaces.measures.put(key, codec, data, walk);
                return Err(e);
            }
            let before = walk;
            let from = walk.read_bits();
            let stepped = walk.step(&data, MEASURE_STEP_BITS);
            // Taken again once, and only when the step failed: output past
            // the cap is refused the same with any number of bytes more.
            if stepped == Err(Refusal::Failed) && !short && (data.len() as u64) < total {
                walk = before;
                short = true;
                continue;
            }
            short = false;
            let to = match stepped {
                Ok(inflate::Measured::Ends(len)) => len * 8,
                Ok(inflate::Measured::Working) => walk.read_bits(),
                // A refused step does not say how far it read, so it is
                // charged in full.
                Err(_) => from + MEASURE_STEP_BITS,
            };
            self.go.spent(to.saturating_sub(from) / MEASURE_BITS_PER_ELEMENT);
            let found = match stepped {
                Ok(inflate::Measured::Working) => continue,
                Ok(inflate::Measured::Ends(len)) => Ok(len),
                Err(why) => Err(why),
            };
            self.spaces.measures.end(key, codec, found);
            return Ok(found);
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
    use crate::source::{ArcSource, MemSource};

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

    /// A file that is one zlib stream, over whatever is handed in. Held in an
    /// `ArcSource`, which gives a handle, so that an index over the stream
    /// reads it through that handle, as an index in the browser reads the
    /// file's store.
    fn zlib_over(content: &[u8]) -> Document<ArcSource> {
        Document::new(ArcSource(std::sync::Arc::new(miniz_oxide::deflate::compress_to_vec_zlib(content, 6))))
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
        assert_eq!(index.input().owned_bytes(), 0, "the index reads the file, and holds no copy of the run");
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

    /// Ask `f` until it stops answering `Pending`, feeding `lossy` the
    /// chunks each answer names, and say how many times it waited. Every
    /// chunk named is one of `run`'s, the bytes of the file the run is.
    fn until_fed<T>(lossy: &crate::codec::lazy::tests::Lossy, run: std::ops::Range<usize>, mut f: impl FnMut() -> crate::eval::R<T>) -> (T, usize) {
        use crate::eval::EvalError;
        let inside = lossy.chunks(run);
        for waits in 0..50 {
            match f() {
                Ok(found) => return (found, waits),
                Err(EvalError::Pending(missing)) => {
                    assert!(!missing.is_empty() && missing.iter().all(|m| inside.contains(m)), "{missing:?} is not in {inside:?}");
                    lossy.feed(&missing);
                }
                Err(other) => panic!("{other:?}"),
            }
        }
        panic!("waited 50 times")
    }

    /// A stream read through an index whose run is missing chunks of the file
    /// waits for them wherever it is read: the template looked for in its
    /// first bytes, its bytes read by a field and by a tab, a step of the map
    /// both ways, a block and a block's codes. Each answer is `Pending`,
    /// naming chunks of the file inside the run, and once they are fed it
    /// reads as the stream does. Nothing is kept as the run's refusal.
    #[test]
    fn a_stream_whose_run_is_missing_chunks_waits_for_them() {
        use crate::codec::lazy::tests::Lossy;
        use crate::codec::lazy::{decode_or_index_with, Decoded, RunInput};
        use crate::codec::Codec;
        use crate::eval::EvalError;
        use crate::source::Source;
        let text = many_lines();
        let d = zlib_over(&text);
        let file = d.source().0.to_vec();
        let run = 2..file.len() - 4;
        let key = (0, run.start as u64 * 8, run.len() as u64 * 8);
        let Ok(Decoded::Indexed(mut index)) = decode_or_index_with(Codec::Deflate, std::sync::Arc::new(file[run.clone()].to_vec()), small_limits()) else {
            panic!("indexed")
        };
        let lossy = Lossy::new(file.clone(), 1024);
        index.set_input(RunInput::new(lossy.clone(), run.start as u64, run.len() as u64));
        let mut e = Evaluator::new(formats::builtin("zlib").unwrap());
        e.spaces.set_lazy_limits(small_limits());
        e.spaces.keep_index(key, Codec::Deflate, std::sync::Arc::new(index));
        let refused = |e: &Evaluator| e.spaces.run_refusal(key, Codec::Deflate).is_some() || !e.spaces.refused_runs.borrow().is_empty();

        // The stream's template says only bytes, so its first bytes are
        // looked at to see what they are, and those wait.
        let (id, waits) = until_fed(&lossy, run.clone(), || e.open_space(&d, 0, RUN));
        let id = id.expect("the stream opens");
        assert!(waits > 0 && !refused(&e));
        let SpaceSource::Lazy(_) = e.space(id).unwrap().source() else { panic!("held whole") };

        // Not the stream's first bytes, whose span was read to see what the
        // stream is, and is still held.
        for at in [400_000usize, 1_234_567, text.len() - 100] {
            lossy.drop_all();
            let (got, waits) = until_fed(&lossy, run.clone(), || e.read_in(&d, id, at as u64 * 8, 800));
            // Once for the span the bytes are in, and once more if they run
            // on into the next.
            assert!(got == text[at..at + 100] && (1..=2).contains(&waits), "100 bytes at {at}, after {waits} waits");
        }
        assert!(!refused(&e));

        // A tab reads the stream through its source, which names the
        // chunks of the file and reads zeroes until they are fed.
        lossy.drop_all();
        let mut got = vec![0u8; 100];
        let missing = e.space(id).unwrap().source().read_bytes(500_000, &mut got);
        assert!(!missing.is_empty() && got == [0u8; 100]);
        let (_, waits) = until_fed(&lossy, run.clone(), || match e.space(id).unwrap().source().read_bytes(500_000, &mut got) {
            missing if missing.is_empty() => Ok(()),
            missing => Err(EvalError::Pending(missing)),
        });
        assert!(waits >= 1 && got == text[500_000..500_100]);

        // The map from a byte of the stream to its step, and back.
        lossy.drop_all();
        let (step, _) = until_fed(&lossy, run.clone(), || e.map_out(id, 350_000));
        let step = step.expect("a step made it");
        lossy.drop_all();
        let back = until_fed(&lossy, run.clone(), || e.map_in(id, run.start as u64 * 8 + step.in_bits.start)).0;
        assert_eq!(back, Some(step));

        // The listing's blocks, read from the index's block starts and the
        // run's bits.
        lossy.drop_all();
        let block = until_fed(&lossy, run.clone(), || e.node(&d, &[6, 1, 3])).0;
        assert!(block.size_bits > 0 && block.child_count > 0, "{block:?}");
        lossy.drop_all();
        let code = until_fed(&lossy, run.clone(), || e.node(&d, &[6, 1, 3, block.child_count as usize - 1, 0])).0;
        assert!(code.size_bits > 0, "{code:?}");
        assert!(!refused(&e));
        let verdict = e.run_check(&d, &[7]).unwrap().expect("the Adler-32 is a check");
        assert!(verdict.ok, "{verdict:?}");
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

    /// The same for a zstd run of several frames, opened through the zstd
    /// template: its pass is charged to the go too, says how far it has got
    /// between goes, and comes to the index one go makes.
    #[test]
    fn a_zstd_run_past_the_cap_opens_a_go_at_a_time() {
        use crate::codec::lazy::tests::zstd_of;
        use crate::eval::EvalError;
        let text = many_lines();
        let d = Document::new(ArcSource(std::sync::Arc::new(zstd_of(&text, 100_000, true))));
        let open = |e: &mut Evaluator| -> crate::eval::R<Option<crate::eval::SpaceId>> {
            let decoded = e.child_named(&d, &[], "decoded")?.expect("a decoded field");
            e.open_space(&d, 0, &[decoded[0], 0])
        };
        let mut one = Evaluator::new(formats::builtin("zstd").unwrap());
        one.spaces.set_lazy_limits(small_limits());
        let id = open(&mut one).unwrap().expect("opens in one go");
        let SpaceSource::Lazy(whole) = one.space(id).unwrap().source().clone() else { panic!("held whole") };

        let mut e = Evaluator::new(formats::builtin("zstd").unwrap());
        e.spaces.set_lazy_limits(small_limits());
        e.set_slice(Some(600));
        let (mut goes, mut read, mut unpacked) = (0, 0, 0);
        let id = loop {
            e.begin_slice();
            match open(&mut e) {
                Ok(Some(id)) => break id,
                Err(EvalError::Busy { .. }) => {
                    goes += 1;
                    let p = e.unpacking().expect("a pass is under way");
                    assert!(p.read_bits >= read && p.read_bits <= p.run_bits, "{read} then {p:?}");
                    assert!(p.unpacked_bytes >= unpacked, "{unpacked} then {p:?}");
                    (read, unpacked) = (p.read_bits, p.unpacked_bytes);
                    assert!(goes < 1000);
                }
                other => panic!("{other:?}"),
            }
        };
        assert!(goes >= 5, "{goes} goes");
        assert_eq!(e.unpacking(), None);
        let SpaceSource::Lazy(index) = e.space(id).unwrap().source().clone() else { panic!("held whole") };
        assert_eq!((index.len(), index.spans()), (whole.len(), whole.spans()));
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
        let d = Document::new(ArcSource(std::sync::Arc::new(gzip_member(&text))));
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

    /// Lines of a count and a number that looks random, which pack to about
    /// half their size, so that a stream of a megabyte or two takes several
    /// steps to measure.
    fn numbered_lines(lines: u64, salt: u64) -> Vec<u8> {
        (0..lines).flat_map(|i| format!("{i} {}\n", i.wrapping_mul(0x9e37_79b9_7f4a_7c15 ^ salt) >> 40).into_bytes()).collect()
    }

    /// One gzip member holding `text`, packed at level 6, with no name.
    fn gzip_member(text: &[u8]) -> Vec<u8> {
        let mut gzip = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3];
        gzip.extend(miniz_oxide::deflate::compress_to_vec(text, 6));
        gzip.extend(crate::checksum::crc32(text).to_le_bytes());
        gzip.extend((text.len() as u32).to_le_bytes());
        gzip
    }

    /// Every gzip member's fields, as a name, where the field starts and how
    /// long it is, read in goes of `slice` elements under `limits`; the
    /// evaluator that read them; and how many goes handed back while a
    /// member's stream was being measured.
    fn gzip_members_in_goes(d: &Document<MemSource>, limits: crate::codec::lazy::Limits, slice: Option<u64>) -> (Vec<(String, u64, u64)>, Evaluator, usize) {
        use crate::eval::EvalError;
        let mut e = Evaluator::new(formats::builtin("gzip").unwrap());
        e.spaces.set_lazy_limits(limits);
        e.set_slice(slice);
        let (mut goes, mut measuring, mut at) = (0, 0, (0, 0));
        let fields = loop {
            e.begin_slice();
            let read = (|| {
                let members = e.node(d, &[0])?.child_count as usize;
                let mut fields = Vec::new();
                for m in 0..members {
                    for f in e.children(d, &[0, m], 0, 20)? {
                        fields.push((f.name, f.offset_bits, f.size_bits));
                    }
                }
                Ok(fields)
            })();
            match read {
                Ok(fields) => break fields,
                Err(EvalError::Busy { .. }) => {
                    goes += 1;
                    assert!(goes < 2000, "no end to the goes");
                    // A member's start is known only once the member before
                    // it is measured, so one walk at a time is under way.
                    assert!(e.spaces.measures.walking.len() <= 1, "{} walks under way", e.spaces.measures.walking.len());
                    if !e.spaces.measures.walking.is_empty() {
                        measuring += 1;
                        // The walk stepped last is the one reported, and it
                        // only moves forward.
                        let p = e.unpacking().expect("a walk is under way");
                        let walk = (p.run_offset_bits, p.read_bits);
                        assert!(walk.0 != at.0 || walk.1 > at.1, "{at:?} then {walk:?}");
                        assert!(p.read_bits <= p.run_bits);
                        at = walk;
                    }
                }
                Err(other) => panic!("{other:?}"),
            }
        };
        e.set_slice(None);
        (fields, e, measuring)
    }

    /// Measuring where a gzip member's stream ends is charged to the work
    /// allowed for a go, and a go that runs out part-way carries on next time
    /// from where it stopped. Two members, both past the cap on a run held
    /// whole, come to the same fields in goes as in one go, each run as long
    /// as its deflate stream, and the joined stream reads across the join.
    #[test]
    fn gzip_members_are_measured_a_go_at_a_time() {
        let (first, second) = (numbered_lines(140_000, 1), numbered_lines(90_000, 2));
        let d = Document::new(MemSource([gzip_member(&first), gzip_member(&second)].concat()));
        let (one, _, measured) = gzip_members_in_goes(&d, small_limits(), None);
        assert_eq!(measured, 0);
        let (stepped, mut e, measured) = gzip_members_in_goes(&d, small_limits(), Some(600));
        assert_eq!(stepped, one);
        // Each member's stream takes several steps to measure.
        assert!(measured >= 8, "{measured} goes measuring");
        let runs: Vec<u64> = stepped.iter().filter(|f| f.0 == "compressed").map(|f| f.2).collect();
        let deflate = |text: &[u8]| miniz_oxide::deflate::compress_to_vec(text, 6).len() as u64 * 8;
        assert_eq!(runs, [deflate(&first), deflate(&second)]);
        assert!(runs.iter().all(|&bits| bits > 4 * super::MEASURE_STEP_BITS), "each run is several steps long: {runs:?}");

        let joined = [&first[..], &second[..]].concat();
        assert_eq!(e.node(&d, &[1, 0]).unwrap().size_bits, joined.len() as u64 * 8);
        let Some(Opened::Space(id)) = e.spaces.get(&[1]) else { panic!("the joined stream did not open") };
        let at = first.len() - 50;
        assert_eq!(e.read_in(&d, id, at as u64 * 8, 800).unwrap(), &joined[at..at + 100]);
    }

    /// A member whose stream comes to more than the measuring cap has no end
    /// to give, and its run falls back to everything before the last trailer,
    /// in goes as in one go. That is right for a file of one member, and for
    /// a file of two it takes in the second, as it did before measuring went
    /// in goes.
    #[test]
    fn a_gzip_member_past_the_measuring_cap_falls_back_in_goes_as_in_one() {
        let limits = crate::codec::lazy::Limits { measure: 1 << 20, ..small_limits() };
        let text = numbered_lines(140_000, 3);
        for members in [1, 2] {
            let file: Vec<u8> = (0..members).flat_map(|_| gzip_member(&text)).collect();
            let d = Document::new(MemSource(file.clone()));
            let (one, _, _) = gzip_members_in_goes(&d, limits, None);
            let (stepped, e, measured) = gzip_members_in_goes(&d, limits, Some(600));
            assert_eq!(stepped, one, "{members} members");
            assert!(measured >= 2, "{members} members: {measured} goes measuring");
            let runs: Vec<u64> = stepped.iter().filter(|f| f.0 == "compressed").map(|f| f.2).collect();
            assert_eq!(runs, [(file.len() as u64 - 10 - 8) * 8], "{members} members");
            let key = (0, 80, (file.len() as u64 - 10) * 8);
            assert_eq!(e.spaces.measures.ended(key, crate::codec::Codec::Deflate), Some(Err(crate::codec::Refusal::TooLarge)));
        }
    }

    /// An edit drops the ends measured, even when no stream has been opened:
    /// a member measured before the edit is measured again after it, and
    /// reads as a fresh reading of the edited file does.
    #[test]
    fn an_edit_drops_the_ends_measured() {
        let mut file = [gzip_member(b"the first member, which an edit breaks"), gzip_member(b"and a second one")].concat();
        let d = Document::new(MemSource(file.clone()));
        let mut e = Evaluator::new(formats::builtin("gzip").unwrap());
        assert_eq!(e.node(&d, &[0]).unwrap().child_count, 2);
        assert!(e.spaces.get(&[0, 0, 10]).is_none(), "nothing has been opened");
        // The first byte of the first member's deflate, now a block of type
        // 3, which no stream has.
        file[10] = 0xff;
        let d = Document::new(MemSource(file));
        e.invalidate_from(80);
        let fresh = Evaluator::new(formats::builtin("gzip").unwrap()).node(&d, &[0]).unwrap().child_count;
        assert_eq!(fresh, 1, "the broken member's run takes in the second");
        assert_eq!(e.node(&d, &[0]).unwrap().child_count, fresh);
    }

    /// A field's name, where it starts and how long it is, for each node.
    fn placed(nodes: Vec<crate::eval::NodeInfo>) -> Vec<(String, u64, u64)> {
        nodes.into_iter().map(|n| (n.name, n.offset_bits, n.size_bits)).collect()
    }

    /// While a gzip member's stream is unpacked a go at a time, the member's
    /// header fields are ready. A slice of the member's children gives them
    /// and stops at the stream, which waits on the pass, and the go says how
    /// far that pass over the member's run has got.
    #[test]
    fn a_gzip_members_header_is_ready_while_its_stream_is_unpacked() {
        use crate::eval::EvalError;
        let text = numbered_lines(300_000, 4);
        let d = Document::new(MemSource(gzip_member(&text)));
        let mut one = Evaluator::new(formats::builtin("gzip").unwrap());
        one.spaces.set_lazy_limits(small_limits());
        let all = placed(one.children(&d, &[0, 0], 0, 20).unwrap());
        let stream = all.iter().position(|f| f.0 == "compressed").expect("a stream");
        assert_eq!((stream, all.len()), (10, 13));

        let mut e = Evaluator::new(formats::builtin("gzip").unwrap());
        e.spaces.set_lazy_limits(small_limits());
        e.set_slice(Some(600));
        let (mut goes, mut indexing) = (0, 0);
        let got = loop {
            e.begin_slice();
            let (ready, rest) = e.children_ready(&d, &[0, 0], 0, 20);
            let ready = placed(ready);
            match rest {
                Ok(()) => break ready,
                Err(EvalError::Busy { .. }) => {
                    assert!(ready.len() <= stream, "a field after the stream came before it: {ready:?}");
                    assert_eq!(ready[..], all[..ready.len()]);
                    if ready.len() == stream {
                        let p = e.unpacking().expect("the stream waits on a pass");
                        assert_eq!((p.run_space, p.run_offset_bits), (0, 80));
                        // The index pass reads the run, which stops at the
                        // trailer. The walk measuring it reads to the end of
                        // the file.
                        if p.run_bits == all[stream].2 {
                            indexing += 1;
                        }
                    }
                }
                Err(other) => panic!("{other:?}"),
            }
            goes += 1;
            assert!(goes < 1000, "no end to the goes");
        };
        assert_eq!(got, all);
        assert!(indexing >= 3, "the header was ready in {indexing} goes of the index pass, of {goes}");
    }

    /// Two streams carried on by turns, as a listing reading one member and
    /// Open unpacked on the other do in the browser. Each call that runs out
    /// says how far the pass it ran out in has got, which is its own
    /// stream's. A go says nothing until it has run out in a pass, though the
    /// other stream's is unfinished, and a go that runs out in one pass and
    /// then reaches the other, with nothing left to step it with, says the
    /// first.
    #[test]
    fn each_call_says_how_far_its_own_stream_has_got() {
        use crate::eval::EvalError;
        let (first, second) = (gzip_member(&numbered_lines(200_000, 5)), gzip_member(&numbered_lines(300_000, 6)));
        let starts = (80, (first.len() as u64 + 10) * 8);
        let d = Document::new(MemSource([first, second].concat()));
        let mut e = Evaluator::new(formats::builtin("gzip").unwrap());
        e.spaces.set_lazy_limits(small_limits());
        // Both members measured, so what is left is the two streams' passes.
        assert_eq!(e.node(&d, &[0]).unwrap().child_count, 2);
        e.set_slice(Some(600));
        let at = |e: &Evaluator| e.unpacking().map(|p| (p.run_space, p.run_offset_bits));

        e.begin_slice();
        assert!(matches!(e.open_space(&d, 0, &[0, 1, 10]), Err(EvalError::Busy { .. })));
        assert_eq!(at(&e), Some((0, starts.1)));
        assert!(matches!(e.children_ready(&d, &[0, 0], 0, 20).1, Err(EvalError::Busy { .. })));
        assert_eq!(at(&e), Some((0, starts.1)), "the first member's pass was reached with nothing left");

        let (mut listed, mut opened) = (None, None);
        let (mut listing, mut opening) = (0, 0);
        while listed.is_none() || opened.is_none() {
            if listed.is_none() {
                e.begin_slice();
                assert_eq!(e.unpacking(), None, "a go that has not run out waits on nothing");
                match e.children_ready(&d, &[0, 0], 0, 20) {
                    (ready, Ok(())) => listed = Some(ready.len()),
                    (ready, Err(EvalError::Busy { .. })) => {
                        listing += 1;
                        assert_eq!(ready.len(), 10, "the header is ready");
                        assert_eq!(at(&e), Some((0, starts.0)), "listing go {listing}");
                    }
                    (_, Err(other)) => panic!("{other:?}"),
                }
            }
            if opened.is_none() {
                e.begin_slice();
                assert_eq!(e.unpacking(), None, "a go that has not run out waits on nothing");
                match e.open_space(&d, 0, &[0, 1, 10]) {
                    Ok(id) => opened = Some(id),
                    Err(EvalError::Busy { .. }) => {
                        opening += 1;
                        assert_eq!(at(&e), Some((0, starts.1)), "opening go {opening}");
                    }
                    Err(other) => panic!("{other:?}"),
                }
            }
            assert!(listing + opening < 2000, "no end to the goes");
        }
        assert!(listing >= 3 && opening >= 3, "{listing} goes listing, {opening} opening");
        assert_eq!(listed, Some(13));
        assert!(opened.flatten().is_some(), "the second member's stream opened");
    }

    /// A ZIP of `entries`, each deflated at level 6, and where each entry's
    /// deflate run starts, in bits. Only the local records and the end record
    /// are written: the template reads the records in order and needs nothing
    /// from a central directory.
    fn zip_of(entries: &[Vec<u8>]) -> (Vec<u8>, Vec<u64>) {
        let (mut file, mut starts) = (Vec::new(), Vec::new());
        for (i, text) in entries.iter().enumerate() {
            let packed = miniz_oxide::deflate::compress_to_vec(text, 6);
            let name = format!("{i}.txt");
            file.extend(b"PK\x03\x04");
            file.extend(20u16.to_le_bytes()); // version needed
            file.extend(0u16.to_le_bytes()); // flags
            file.extend(8u16.to_le_bytes()); // deflate
            file.extend([0u8; 4]); // time and date
            file.extend(crate::checksum::crc32(text).to_le_bytes());
            file.extend((packed.len() as u32).to_le_bytes());
            file.extend((text.len() as u32).to_le_bytes());
            file.extend((name.len() as u16).to_le_bytes());
            file.extend(0u16.to_le_bytes()); // no extra field
            file.extend(name.as_bytes());
            starts.push(file.len() as u64 * 8);
            file.extend(packed);
        }
        let directory = file.len() as u32;
        file.extend(b"PK\x05\x06");
        file.extend([0u8; 12]); // disks, entries and the directory's size: none
        file.extend(directory.to_le_bytes());
        file.extend(0u16.to_le_bytes()); // no comment
        (file, starts)
    }

    /// Where entry `i`'s data is in the ZIP template: each record is its
    /// signature and its body, and the body's fifteenth field is the data.
    fn zip_data(i: usize) -> [usize; 4] {
        [0, i, 1, 14]
    }

    /// Every byte of an opened space, whether it is held whole or read
    /// through an index.
    fn bytes_of(e: &Evaluator, id: crate::eval::SpaceId) -> Vec<u8> {
        match e.space(id).expect("a space").source().clone() {
            SpaceSource::Whole(bytes) => bytes.to_vec(),
            SpaceSource::Lazy(index) => {
                let mut got = vec![0u8; index.len() as usize];
                assert!(index.read(0, &mut got).is_ok());
                got
            }
            SpaceSource::Joined(_) => panic!("a joined stream"),
        }
    }

    /// Open the data of the ZIP entries in `asks`, one go each, by turns,
    /// until each of them has opened. An entry named more than once in `asks`
    /// is asked that many times a round. Each go that hands back `Busy` names
    /// its own entry's run or none, each entry's pass never goes back, and no
    /// more passes are held unfinished than there are places for. Gives each
    /// entry's space, by entry.
    fn open_by_turns(e: &mut Evaluator, d: &Document<MemSource>, starts: &[u64], asks: &[usize]) -> Vec<Option<crate::eval::SpaceId>> {
        use crate::eval::EvalError;
        let mut opened = vec![None; starts.len()];
        let mut read = vec![0u64; starts.len()];
        let wanted: std::collections::BTreeSet<usize> = asks.iter().copied().collect();
        let mut rounds = 0;
        while wanted.iter().any(|&i| opened[i].is_none()) {
            for &i in asks {
                if opened[i].is_some() {
                    continue;
                }
                e.begin_slice();
                match e.open_space(d, 0, &zip_data(i)) {
                    Ok(id) => opened[i] = Some(id.expect("the entry's data opens")),
                    Err(EvalError::Busy { .. }) => {
                        if let Some(p) = e.unpacking() {
                            assert_eq!((p.run_space, p.run_offset_bits), (0, starts[i]), "round {rounds}: entry {i} named another entry's pass");
                            assert!(p.read_bits >= read[i], "round {rounds}: entry {i}'s pass went back from {} to {}", read[i], p.read_bits);
                            read[i] = p.read_bits;
                        }
                    }
                    Err(other) => panic!("{other:?}"),
                }
                assert!(e.spaces.unfinished.borrow().entries.len() <= super::UNFINISHED_PASSES);
            }
            rounds += 1;
            assert!(rounds < 300, "no end to the rounds, the passes at {read:?}");
        }
        opened
    }

    /// Three large streams carried on by turns, one go each, as a listing
    /// showing three ZIP entries at once carries them on. There are places
    /// for two passes that have not finished. A third stream used to push out
    /// the pass stepped longest ago, which then started again from the front
    /// on its next go, so that none of them ever finished. Now a stream that
    /// finds both places taken spends its go carrying one of those passes on,
    /// and starts its own once one of them has finished. Each pass is made
    /// once, also when one stream is asked four times a round, and each
    /// stream reads back its own text.
    #[test]
    fn three_streams_carried_on_by_turns_all_open() {
        let texts: Vec<Vec<u8>> = (0..3).map(|i| numbered_lines(140_000, 10 + i)).collect();
        let (file, starts) = zip_of(&texts);
        let d = Document::new(MemSource(file));
        for asks in [&[0, 1, 2][..], &[0, 1, 2, 2, 2, 2], &[2, 2, 2, 2, 0, 1]] {
            let mut e = Evaluator::new(formats::builtin("zip").unwrap());
            e.spaces.set_lazy_limits(small_limits());
            e.set_slice(Some(600));
            let opened = open_by_turns(&mut e, &d, &starts, asks);
            assert_eq!(e.spaces.passes.get(), 3, "asked {asks:?}");
            e.set_slice(None);
            for (i, id) in opened.into_iter().enumerate() {
                let id = id.expect("opened");
                assert!(e.spaces.lazy(id).is_some(), "entry {i} is past the cap");
                assert!(bytes_of(&e, id) == texts[i], "entry {i}'s bytes, asked {asks:?}");
            }
        }
    }

    /// Two streams started and never asked for again, then a third: the
    /// third opens, by carrying the other two to their ends first, and the
    /// first two are there for the asking afterwards. Past the cap they come
    /// to indexes, kept with the others. Under it they come to runs held
    /// whole, which wait in their places for a call to take them; with both
    /// places holding one, the third lets go of the one put there first, and
    /// that stream is unpacked again when it is asked for.
    #[test]
    fn a_stream_opens_behind_two_that_nobody_asks_for_any_more() {
        use crate::eval::EvalError;
        let texts: Vec<Vec<u8>> = (0..3).map(|i| numbered_lines(140_000, 30 + i)).collect();
        let (file, starts) = zip_of(&texts);
        let d = Document::new(MemSource(file));
        let under = crate::codec::lazy::Limits { whole: 4 << 20, ..small_limits() };
        for (limits, passes) in [(small_limits(), 3), (under, 4)] {
            let mut e = Evaluator::new(formats::builtin("zip").unwrap());
            e.spaces.set_lazy_limits(limits);
            e.set_slice(Some(600));
            for i in [0, 1] {
                e.begin_slice();
                assert!(matches!(e.open_space(&d, 0, &zip_data(i)), Err(EvalError::Busy { .. })), "entry {i} opened in one go");
                assert_eq!(e.unpacking().map(|p| p.run_offset_bits), Some(starts[i]));
            }
            let third = open_by_turns(&mut e, &d, &starts, &[2]);
            let opened = open_by_turns(&mut e, &d, &starts, &[0, 1]);
            assert_eq!(e.spaces.passes.get(), passes, "{} passes, the cap at {} bytes", e.spaces.passes.get(), limits.whole);
            e.set_slice(None);
            for (i, id) in [opened[0], opened[1], third[2]].into_iter().enumerate() {
                let id = id.expect("opened");
                assert_eq!(e.spaces.lazy(id).is_some(), limits.whole < texts[i].len(), "entry {i}");
                assert!(bytes_of(&e, id) == texts[i], "entry {i}'s bytes, the cap at {} bytes", limits.whole);
            }
        }
    }

    /// A call with an unlimited allowance, as the wasm crate makes where it
    /// works to the end in one call, opens a third stream while both places
    /// are taken, without waiting and without touching the two passes in
    /// them. Those carry on afterwards from where they were.
    #[test]
    fn a_call_with_no_limit_opens_a_stream_beside_two_unfinished() {
        use crate::eval::EvalError;
        let texts: Vec<Vec<u8>> = (0..3).map(|i| numbered_lines(140_000, 50 + i)).collect();
        let (file, starts) = zip_of(&texts);
        let d = Document::new(MemSource(file));
        let mut e = Evaluator::new(formats::builtin("zip").unwrap());
        e.spaces.set_lazy_limits(small_limits());
        e.set_slice(Some(600));
        for i in [0, 1] {
            e.begin_slice();
            assert!(matches!(e.open_space(&d, 0, &zip_data(i)), Err(EvalError::Busy { .. })), "entry {i} opened in one go");
        }
        let held = |e: &Evaluator| -> Vec<(u64, u64)> {
            let mut held: Vec<(u64, u64)> = e.spaces.unfinished.borrow().entries.iter().map(|(key, _, h, _)| match h {
                super::Held::Pass(pass) => (key.1, pass.progress().read_bits),
                super::Held::Whole(..) => panic!("a run held whole"),
                super::Held::Gathering { .. } => panic!("a run being read a window at a time, from memory"),
            }).collect();
            held.sort();
            held
        };
        let before = held(&e);
        assert_eq!(before.iter().map(|h| h.0).collect::<Vec<_>>(), [starts[0], starts[1]]);
        e.set_slice(None);
        e.begin_slice();
        let third = e.open_space(&d, 0, &zip_data(2)).unwrap().expect("the third entry opens in one call");
        assert!(bytes_of(&e, third) == texts[2]);
        assert_eq!(held(&e), before, "the two passes held were touched");
        e.set_slice(Some(600));
        let opened = open_by_turns(&mut e, &d, &starts, &[0, 1]);
        assert_eq!(e.spaces.passes.get(), 3, "a held pass started again");
        e.set_slice(None);
        for i in [0, 1] {
            assert!(bytes_of(&e, opened[i].expect("opened")) == texts[i], "entry {i}'s bytes");
        }
    }

    /// The same for the walks measuring where deflate streams end: three
    /// streams measured by turns each carry on from where they stopped, and
    /// each comes to its own end. No template has two walks under way today:
    /// only gzip's measures streams, and a member's start is known only once
    /// the member before it has been measured. A template measuring streams
    /// that are read independently of each other would.
    #[test]
    fn three_streams_measured_by_turns_all_end() {
        use crate::codec::Codec;
        use crate::eval::EvalError;
        let texts: Vec<Vec<u8>> = (0..3).map(|i| numbered_lines(90_000, 40 + i)).collect();
        let members: Vec<Vec<u8>> = texts.iter().map(|t| gzip_member(t)).collect();
        let file = members.concat();
        // Each member's deflate, which starts after ten bytes of header, and
        // the bits from there to the end of the file, which is what a walk
        // measuring it reads.
        let mut keys = Vec::new();
        let mut at = 0u64;
        for m in &members {
            keys.push((0, (at + 10) * 8, (file.len() as u64 - at - 10) * 8));
            at += m.len() as u64;
        }
        let d = Document::new(MemSource(file));
        let mut e = Evaluator::new(formats::builtin("gzip").unwrap());
        e.spaces.set_lazy_limits(small_limits());
        e.set_slice(Some(600));
        let (mut ended, mut read, mut rounds) = (vec![None; 3], vec![0u64; 3], 0);
        while ended.iter().any(Option::is_none) {
            for i in 0..3 {
                if ended[i].is_some() {
                    continue;
                }
                e.begin_slice();
                match e.stream_end(&d, keys[i], Codec::Deflate) {
                    Ok(found) => ended[i] = Some(found),
                    Err(EvalError::Busy { .. }) => {
                        if let Some(p) = e.unpacking() {
                            assert_eq!(p.run_offset_bits, keys[i].1, "round {rounds}: stream {i} named another stream's walk");
                            assert!(p.read_bits >= read[i], "round {rounds}: stream {i}'s walk went back from {} to {}", read[i], p.read_bits);
                            read[i] = p.read_bits;
                        }
                    }
                    Err(other) => panic!("{other:?}"),
                }
                assert!(e.spaces.measures.walking.len() <= super::MEASURING_WALKS);
            }
            rounds += 1;
            assert!(rounds < 300, "no end to the rounds, the walks at {read:?}");
        }
        let deflate = |text: &[u8]| miniz_oxide::deflate::compress_to_vec(text, 6).len() as u64;
        let ends: Vec<_> = texts.iter().map(|t| Some(Ok(deflate(t)))).collect();
        assert_eq!(ended, ends);
        assert!(read.iter().all(|&r| r > 0), "each walk took more than one go: {read:?}");
    }

    /// An xz stream of several blocks and a zstd run of several frames, past
    /// the cap, open through their templates as spaces read a block or a
    /// frame at a time. An xz block's check is taken over that block's bytes,
    /// read through the index.
    #[test]
    fn xz_blocks_and_zstd_frames_past_the_cap_read_one_at_a_time() {
        use crate::codec::lazy::tests::{mixed, xz_of, zstd_of};
        use crate::codec::lazy::Limits;
        let data = mixed(400_000, 89);
        let limits = Limits { whole: 64 << 10, span: 64 << 10, spacing: 16 << 10, checkpoints: 32, span_cache: 64 << 10, ..Limits::default() };
        let open = |template: &str, packed: Vec<u8>| {
            let d = Document::new(MemSource(packed));
            let mut e = Evaluator::new(formats::builtin(template).unwrap());
            e.spaces.set_lazy_limits(limits);
            let decoded = e.child_named(&d, &[], "decoded").unwrap().expect("a decoded field");
            let id = e.open_space(&d, 0, &[decoded[0], 0]).unwrap().expect("opens");
            (d, e, id)
        };
        for (template, packed) in [("xz", xz_of(&data, 30_000)), ("zstd", zstd_of(&data, 30_000, true))] {
            let (d, e, id) = open(template, packed);
            assert!(e.spaces.lazy(id).is_some(), "{template}: held whole");
            for at in [0usize, 12_345, 299_999, data.len() - 100] {
                let got = e.read_in(&d, id, at as u64 * 8, 800).unwrap();
                assert_eq!(got, &data[at..at + 100], "{template}: 100 bytes at {at}");
            }
            let space = e.space(id).unwrap();
            assert_eq!(space.len_bytes(), data.len() as u64);
            let step = space.map_out(350_000).unwrap().expect("a step made it");
            assert!(step.out_bytes.contains(&350_000), "{template}: {step:?}");
        }
        let (d, mut e, _) = open("xz", xz_of(&data, 30_000));
        let blocks = e.child_named(&d, &[], "blocks").unwrap().unwrap();
        for i in [0, 5, 13] {
            let check = e.child_named(&d, &[blocks[0], i], "check").unwrap().unwrap();
            let v = e.run_check(&d, &check).unwrap().expect("the block's CRC-32 is a check");
            assert!(v.ok, "block {i}: {v:?}");
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
            let step = space.map_out(byte).unwrap().unwrap_or_else(|| panic!("byte {byte} came from nowhere"));
            assert!(step.out_bytes.contains(&byte));
            assert!(matches!(step.kind, StepKind::Literal(_) | StepKind::Match { .. } | StepKind::Stored | StepKind::Pixel));
            assert!(step.in_bits.end <= field.size_bits, "{:?} is past the run's {} bits", step.in_bits, field.size_bits);
            // And the bits it read, as bits of the file, lead back to it.
            assert_eq!(space.map_in(field.offset_bits + step.in_bits.start), Ok(Some(step)));
        }
        assert_eq!(space.map_out(text.len() as u64), Ok(None));
        for bit in 0..field.offset_bits {
            assert_eq!(space.map_in(bit), Ok(None), "bit {bit} is the zlib header's, not the run's");
        }
        assert_eq!(space.map_in(field.offset_bits + field.size_bits), Ok(None), "the Adler-32 after the run");
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
