//! Compressed runs too large to hold whole, read a piece at a time.
//!
//! Every other codec here decodes a run into one buffer and refuses once the
//! output passes [`CAP_BYTES`]. Three kinds of run past that are read through
//! checkpoints instead: places a decoder can start again from. A read decodes
//! from the checkpoint before it, and only as far as the next one. See
//! `docs/DESIGN-lazy-unpacking.md`.
//!
//! - A deflate run (raw, zlib or gzip) is decoded once from front to back,
//!   and a checkpoint keeps the 32 KiB of output before it, which is as far
//!   back as a deflate match can copy from.
//! - A zstd run of several frames is decoded once the same way, and a
//!   checkpoint is the start of a frame, which copies from nothing before it.
//! - An xz stream of several blocks is not decoded at all to index it: its own
//!   index says where every block is and what it comes to, and a checkpoint
//!   is the start of a block, which copies from nothing before it either.
//!
//! The output between two checkpoints is a span. Spans are what is decoded
//! and what is cached, and each has a trace of its own, made when asked for.
//! The steps of a deflate stream name literals by their byte and matches by
//! their length and distance, so they can be read off any stretch of the
//! stream without the output before it; a zstd frame and an xz block are
//! traced from their own start.
//!
//! A zstd frame or an xz block is decoded whole, so one larger than
//! [`Limits::span`] is refused as [`Refusal::TooLarge`]. That is what keeps a
//! file of one frame or one block refused past [`CAP_BYTES`]: what a decoder
//! needs to start inside one is its whole window or dictionary, which can be
//! as large as the output.
//!
//! Deflate's pass can be done a step at a time, carrying on from where the
//! step before stopped: see [`Indexing`]. zstd's pass is done in one go.
//!
//! Deflate's pass also records where each deflate block starts, in the run
//! and in the output. That is enough to list the blocks, and to trace any one
//! block when asked: like a span, a block can be traced from the compressed
//! run alone. A hostile run can hold tens of millions of tiny blocks, so past
//! [`MAX_BLOCK_MARKS`] the index keeps the start of every second block,
//! halving again each time the list fills, as it does with checkpoints. A
//! dropped start is found again by decoding from the kept start before it.
//!
//! The guard against a zip bomb is [`LAZY_CAP_BYTES`], a limit on the output.
//! Memory is bounded by the limits below whatever the output comes to, so a
//! bomb can only cost the time of one pass up to that limit.

use std::ops::ControlFlow;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::checksum::{adler32_update, crc32_update};
use crate::codec::inflate::{self, CountSink, Ended, Framing, Resume, Sink, Wrapper};
use crate::codec::{frames, xz, Block, BlockKind, Codec, Member, Refusal, Step, StepField, StepKind, Trace, TraceBuilder, CAP_BYTES, MAX_STEPS};

/// The most a run read a piece at a time may come to. Past this it is refused
/// as [`Refusal::TooLargeLazy`].
pub const LAZY_CAP_BYTES: u64 = 1 << 30;

/// The most checkpoints one run keeps. When there would be more, every second
/// one is dropped and the spacing doubles, so 256 × 32 KiB = 8 MiB of windows
/// is the most a run's checkpoints hold.
pub const MAX_CHECKPOINTS: usize = 256;

/// How much output comes between two checkpoints, until there are too many.
pub const SPACING_START: u64 = 1 << 20;

/// How much decoded output one run keeps, in spans, before the span read
/// longest ago is dropped. One span is kept whatever its size.
pub const SPAN_CACHE_BYTES: usize = 16 << 20;

/// How many steps a deflate span's trace may hold before it names blocks
/// rather than codes, as [`crate::codec::MAX_STEPS`] does for a whole run.
///
/// A zstd or xz span's trace may hold [`crate::codec::MAX_STEPS`], as many as
/// a whole run's. Such a span is one or more frames or blocks of up to
/// [`Limits::span`] (64 MiB), which is as large as a run decoded whole, and an
/// `xz -T0` block of 24 MiB of text takes over two million steps. A deflate
/// span is a few megabytes at most.
pub const SPAN_TRACE_STEPS: usize = 1_000_000;

/// The most block starts one run keeps. Past this, every second start is
/// dropped, and a dropped start is found again by decoding from the kept start
/// before it. So a run's block starts take at most 65,536 × 16 bytes = 1 MiB.
/// Ordinary encoders write far fewer blocks: zlib ends a block every 16,384
/// codes and a stored block holds at most 64 KiB, so 1 GiB of output is tens
/// of thousands of blocks. A hostile run can start a new block every 10 bits,
/// since an empty fixed-Huffman block is 10 bits long.
pub const MAX_BLOCK_MARKS: usize = 1 << 16;

/// How many lists of dropped block starts one run keeps once it has found
/// them again by decoding. Each list covers the blocks from one kept start to
/// the next, at most 1,024 blocks under the default limits.
const GROUPS_KEPT: usize = 16;

/// As far back as a deflate match can copy from.
const WINDOW: usize = 32 * 1024;

/// How much output the index pass holds before dropping all but the last
/// [`WINDOW`] of it. Larger means fewer copies of the window and more memory.
const SLIDE_AT: usize = 256 * 1024;

/// The limits a run is decoded under. [`Limits::default`] is the constants
/// above; tests make them small, so that a few hundred kilobytes of data
/// reach every one of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The most a run decoded whole may come to: [`CAP_BYTES`].
    pub whole: usize,
    /// [`LAZY_CAP_BYTES`].
    pub lazy: u64,
    /// [`SPACING_START`].
    pub spacing: u64,
    /// [`MAX_CHECKPOINTS`].
    pub checkpoints: usize,
    /// [`SPAN_CACHE_BYTES`].
    pub span_cache: usize,
    /// [`SPAN_TRACE_STEPS`]. Also the most steps one block's trace holds
    /// before its codes become one opaque step, and the most steps the cached
    /// block traces of one run hold in total.
    pub span_steps: usize,
    /// How many steps the trace of a run held whole may hold before it names
    /// blocks rather than codes: [`crate::codec::MAX_STEPS`].
    pub whole_steps: usize,
    /// [`MAX_BLOCK_MARKS`].
    pub block_marks: usize,
    /// The most one zstd frame or one xz block may come to, since it is
    /// decoded whole: [`CAP_BYTES`]. A limit of its own and not
    /// [`Limits::whole`] because the tools that write these choose their
    /// size: `pzstd` writes frames four windows long, and `xz -T0` writes
    /// blocks three dictionaries long, megabytes at the smallest.
    pub span: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            whole: CAP_BYTES,
            lazy: LAZY_CAP_BYTES,
            spacing: SPACING_START,
            checkpoints: MAX_CHECKPOINTS,
            span_cache: SPAN_CACHE_BYTES,
            span_steps: SPAN_TRACE_STEPS,
            whole_steps: crate::codec::MAX_STEPS,
            block_marks: MAX_BLOCK_MARKS,
            span: CAP_BYTES,
        }
    }
}

/// Whether a run packed with `codec` can be read a piece at a time.
pub fn can_index(codec: Codec) -> bool {
    matches!(codec, Codec::Deflate | Codec::Zlib | Codec::Gzip | Codec::Zstd | Codec::Xz)
}

/// Whether the pass that indexes a run packed with `codec` can be done a step
/// at a time, by an [`Indexing`]: deflate, zlib and gzip. A zstd run is
/// decoded in one go to index it, and an xz stream is laid out from its own
/// index without decoding any of it, by [`decode_or_index`].
pub fn can_step(codec: Codec) -> bool {
    wrapper(codec).is_some()
}

fn wrapper(codec: Codec) -> Option<Wrapper> {
    match codec {
        Codec::Deflate => Some(Wrapper::Raw),
        Codec::Zlib => Some(Wrapper::Zlib),
        Codec::Gzip => Some(Wrapper::Gzip),
        _ => None,
    }
}

/// What a run came to: its bytes and trace, held whole as every run under the
/// cap is, or an index to read it through.
pub enum Decoded {
    Whole(Vec<u8>, Trace),
    Indexed(Seekable),
}

/// Open a run whole if it fits under the cap, and index it otherwise.
///
/// For deflate, zlib, gzip and zstd this is one pass. The run is decoded as
/// [`crate::codec::decode_traced`] decodes it, with checkpoints taken on the
/// way; a run that ends under the cap comes back exactly as that function
/// gives it, and one that passes the cap carries on from its last checkpoint
/// with the trace off and the output dropped as soon as nothing can copy from
/// it. Deflate's pass is an [`Indexing`] run to the end; zstd's is made in
/// one go. An xz stream is laid out from its own index, without a
/// pass. Every other codec is decoded whole or refused, as it always was.
pub fn decode_or_index(codec: Codec, packed: Arc<Vec<u8>>) -> Result<Decoded, Refusal> {
    decode_or_index_with(codec, packed, Limits::default())
}

/// [`decode_or_index`] under limits other than the default ones.
pub fn decode_or_index_with(codec: Codec, packed: Arc<Vec<u8>>, limits: Limits) -> Result<Decoded, Refusal> {
    if packed.len() > CAP_BYTES {
        return Err(Refusal::TooLarge);
    }
    match codec {
        _ if can_step(codec) => {
            let mut pass = Indexing::new(codec, packed, limits)?;
            loop {
                if let Stepped::Done(decoded) = pass.step(u64::MAX)? {
                    return Ok(decoded);
                }
            }
        }
        Codec::Zstd => index_zstd(packed, limits),
        Codec::Xz => index_xz(packed, limits),
        _ => whole(codec, &packed),
    }
}

fn whole(codec: Codec, packed: &[u8]) -> Result<Decoded, Refusal> {
    crate::codec::decode_traced(codec, packed).map(|(bytes, trace)| Decoded::Whole(bytes, trace))
}

/// What a run of no checkpoints but those it was made with comes to: no
/// spans cached and no span trace made yet, and no deflate blocks.
fn seekable(codec: Codec, packed: Arc<Vec<u8>>, kind: Kind, len: u64, checkpoints: Vec<Checkpoint>, limits: Limits) -> Seekable {
    Seekable {
        codec,
        packed,
        kind,
        len,
        checkpoints,
        blocks: BlockMarks::new(limits.block_marks),
        blocks_end: 0,
        crc32: None,
        adler32: None,
        limits,
        spans: Mutex::new(Recent::new(limits.span_cache)),
        traced: Mutex::new(None),
        groups: Mutex::new(Recent::new(GROUPS_KEPT)),
        block_traces: Mutex::new(Recent::new(limits.span_steps)),
    }
}

/// A zstd run, in one pass as deflate is. Frames copy nothing from the ones
/// before them, so a checkpoint is the start of a frame and keeps no window,
/// and the pass past the cap starts again at the start of the frame the first
/// pass was in, which costs at most one frame decoded twice.
///
/// A frame larger than [`Limits::span`] is refused as too large however many
/// frames there are, since a span is decoded whole and a frame cannot be
/// entered anywhere but at its start. A file of one frame is the usual case:
/// `zstd` writes one frame whatever `-T` and `-B` say, and only `pzstd`, or
/// files joined end to end, have several.
fn index_zstd(packed: Arc<Vec<u8>>, limits: Limits) -> Result<Decoded, Refusal> {
    let data = &packed[..];
    let frame_cap = limits.span as u64;
    let mut marks = Checkpoints::new(limits.spacing, limits.checkpoints);
    let mark = |at: usize, out: u64, marks: &mut Checkpoints| marks.offer(at as u64 * 8, out, || Start::Unit(0));

    // The pass that may come to a whole run, traced, with checkpoints taken
    // in case it does not. A frame may come to as much as the whole run may
    // here, and the frames this pass finished are held to the span limit
    // only if the run turns out not to fit.
    let mut last = (0usize, 0u64);
    let mut largest = 0u64;
    let mut over = false;
    let mut out = Vec::new();
    let mut b = TraceBuilder::default();
    let first = frames::zstd_walk(
        data,
        0..data.len(),
        0,
        frame_cap.max(limits.whole as u64),
        &mut b,
        &mut |at, o| {
            largest = largest.max(o - last.1);
            last = (at, o);
            mark(at, o, &mut marks);
        },
        &mut |bytes| {
            if out.len() + bytes.len() > limits.whole {
                over = true;
                return Err(Refusal::TooLarge);
            }
            out.extend_from_slice(bytes);
            Ok(())
        },
    );
    match first {
        Ok(end) => return Ok(Decoded::Whole(out, frames::zstd_done(b, data.len(), end))),
        Err(Refusal::TooLarge) if over => {}
        Err(other) => return Err(other),
    }
    drop((out, b));
    if largest > frame_cap {
        return Err(Refusal::TooLarge);
    }

    // Past the cap: on from the start of the frame the first pass stopped in,
    // counting the output and keeping none of it.
    let (at, before) = last;
    let mut count = before;
    let len = frames::zstd_walk(
        data,
        at..data.len(),
        before,
        frame_cap,
        &mut TraceBuilder::off(),
        &mut |at, o| mark(at, o, &mut marks),
        &mut |bytes| {
            count += bytes.len() as u64;
            match count > limits.lazy {
                true => Err(Refusal::TooLargeLazy),
                false => Ok(()),
            }
        },
    )?;
    Ok(Decoded::Indexed(seekable(Codec::Zstd, packed, Kind::Zstd, len, marks.list, limits)))
}

/// An xz stream, indexed from the index at its end without decoding any of
/// it: that index lists every block's packed and unpacked size, and an xz
/// block never copies from an earlier block, so a checkpoint is the start of
/// a block.
///
/// A stream whose index adds up to no more than [`Limits::whole`], and a
/// stream [`xz::layout`] cannot read, go to the whole decoder as before, which
/// decodes them or refuses them. A larger stream is refused as too large when
/// its blocks use a filter our LZMA2 decoder does not run, because `lzma-rs`,
/// which runs those filters, reads only whole streams. It is refused as too
/// large too when any block unpacks to more than [`Limits::span`], which is
/// what a file of one block, as plain `xz` writes, is.
///
/// Nothing is decoded here, so a block whose unpacked size differs from the
/// index is found only when its span is read, and that read is refused.
fn index_xz(packed: Arc<Vec<u8>>, limits: Limits) -> Result<Decoded, Refusal> {
    let Some(l) = xz::layout(&packed).filter(|l| l.total_out > limits.whole as u64) else {
        return whole(Codec::Xz, &packed);
    };
    if !l.plain || l.blocks.iter().any(|b| b.unpacked > limits.span as u64) {
        return Err(Refusal::TooLarge);
    }
    if l.total_out > limits.lazy {
        return Err(Refusal::TooLargeLazy);
    }
    let mut marks = Checkpoints::new(limits.spacing, limits.checkpoints);
    let mut out = 0u64;
    for (i, blk) in l.blocks.iter().enumerate() {
        // The first span starts with the stream header, which is no block's.
        let at = if i == 0 { 0 } else { blk.at as u64 * 8 };
        marks.offer(at, out, || Start::Unit(i));
        out += blk.unpacked;
    }
    let len = l.total_out;
    Ok(Decoded::Indexed(seekable(Codec::Xz, packed, Kind::Xz(l), len, marks.list, limits)))
}

/// The pass [`decode_or_index`] makes, done a slice at a time: each
/// [`Indexing::step`] decodes about as much output as it is told to, stops at
/// a place the decoder can carry on from, and the next step carries on from
/// exactly there. A browser calls it between frames, so that indexing 500 MB
/// of output does not stop the page for seconds.
///
/// Holds what the pass would hold if it ran in one go: up to the cap of output
/// and its trace while the run might still come to a whole one, and after
/// that the window, the checkpoints, the block starts and the running sums.
pub struct Indexing {
    codec: Codec,
    packed: Arc<Vec<u8>>,
    framing: Framing,
    limits: Limits,
    phase: Phase,
    /// The furthest bit of the run a step has stopped at. Kept apart from
    /// where the next step starts, which goes back a little once the output
    /// passes the cap. The same for the output, in `most`.
    furthest: u64,
    most: u64,
}

/// How far an [`Indexing`] has got.
enum Phase {
    /// Decoding as a run held whole is decoded, traced, with checkpoints
    /// taken in case it comes to more than the cap. `from` is where the step
    /// before stopped. A step that stops inside a block leaves it open in the
    /// trace, and the next reads its header and tables again without
    /// recording them; see [`TraceBuilder::carried_on`].
    Whole { sink: WindowSink, trace: TraceBuilder, from: Option<Resume> },
    /// Past the cap: the trace is off and only the window is kept.
    Index { sink: WindowSink, from: Resume },
    /// Finished, or refused. A step asked of it is a caller's mistake.
    Over,
}

/// What one [`Indexing::step`] came to.
pub enum Stepped {
    /// Not finished yet. Step again to carry on.
    Working,
    Done(Decoded),
}

impl Indexing {
    /// Start indexing `packed`. `codec` is one [`can_step`] says yes to; any
    /// other, zstd and xz included, is refused as [`Refusal::Failed`]. Nothing
    /// is decoded until the first step.
    ///
    /// A gzip run whose trailer says it comes to more than the cap starts as
    /// [`Indexing::known_large`] does. The trailer gives the size modulo
    /// 4 GiB, so a size past the cap there is past it in fact; a trailer that
    /// is wrong about it fails the check at the end either way.
    pub fn new(codec: Codec, packed: Arc<Vec<u8>>, limits: Limits) -> Result<Indexing, Refusal> {
        let wrapper = wrapper(codec).ok_or(Refusal::Failed)?;
        if packed.len() > CAP_BYTES {
            return Err(Refusal::TooLarge);
        }
        let framing = inflate::framing(wrapper, &packed)?;
        let mut trace = TraceBuilder::with_budget(limits.whole_steps).carried_on();
        if wrapper != Wrapper::Raw {
            trace.push(0, 0, StepKind::Header(StepField::Wrapper, 0));
        }
        let mut sink = WindowSink::new(Vec::new(), 0, limits.whole as u64, false);
        sink.marks = Some(Checkpoints::new(limits.spacing, limits.checkpoints));
        sink.blocks = Some(BlockMarks::new(limits.block_marks));
        sink.sums = Some(Sums { crc: 0, adler: 1, upto: 0 });
        sink.mid_block = true;
        let phase = Phase::Whole { sink, trace, from: None };
        let pass = Indexing { codec, packed, framing, limits, phase, furthest: 0, most: 0 };
        let says = match pass.packed.len().checked_sub(4) {
            Some(at) if wrapper == Wrapper::Gzip => u32::from_le_bytes(pass.packed[at..].try_into().expect("four bytes")),
            _ => 0,
        };
        Ok(if says as usize > limits.whole { pass.known_large() } else { pass })
    }

    /// The same pass for a run the caller knows comes to more than the cap: a
    /// stitched stream's part whose length the file gives. It starts with the
    /// trace off, as a pass that has passed the cap carries on, and so skips
    /// decoding the first 64 MiB with the trace on, which is the slowest part
    /// of the pass and is thrown away at the cap. What it comes to is the
    /// index the pass would have made. A run that turns out to fit under the
    /// cap after all comes back indexed too, and a caller that knew its
    /// length finds it is not the one it was told.
    pub fn known_large(mut self) -> Indexing {
        if matches!(self.phase, Phase::Whole { from: None, .. }) {
            let mut sink = WindowSink::new(Vec::new(), 0, self.limits.lazy, true);
            sink.marks = Some(Checkpoints::new(self.limits.spacing, self.limits.checkpoints));
            sink.blocks = Some(BlockMarks::new(self.limits.block_marks));
            sink.sums = Some(Sums { crc: 0, adler: 1, upto: 0 });
            sink.mid_block = true;
            self.phase = Phase::Index { sink, from: Resume { in_bit: self.framing.start, block: None } };
        }
        self
    }

    /// How far the pass has got: the bits of the run read and the bytes of
    /// output written, at the furthest it has been, so that neither goes
    /// back when the pass does.
    pub fn progress(&self) -> Progress {
        let run_bits = self.packed.len() as u64 * 8;
        let unpacked = match &self.phase {
            Phase::Whole { sink, .. } | Phase::Index { sink, .. } => sink.len(),
            Phase::Over => 0,
        };
        Progress { read_bits: self.furthest, run_bits, unpacked: self.most.max(unpacked) }
    }

    /// Decode about `out_bytes` more output and stop at the next code, or
    /// finish. A stored block is not stopped in, so a step can overrun by up
    /// to one stored block, 64 KiB.
    pub fn step(&mut self, out_bytes: u64) -> Result<Stepped, Refusal> {
        let stepped = self.step_on(out_bytes);
        let (at, out) = match &self.phase {
            Phase::Whole { sink, from, .. } => (from.map_or(0, |r| r.in_bit), sink.len()),
            Phase::Index { sink, from } => (from.in_bit, sink.len()),
            Phase::Over => (self.packed.len() as u64 * 8, 0),
        };
        self.furthest = self.furthest.max(at);
        self.most = self.most.max(out);
        stepped
    }

    fn step_on(&mut self, out_bytes: u64) -> Result<Stepped, Refusal> {
        let data = &self.packed[..];
        let f = self.framing;
        match std::mem::replace(&mut self.phase, Phase::Over) {
            Phase::Whole { mut sink, mut trace, from } => {
                sink.slice_end = sink.len().saturating_add(out_bytes.max(1));
                match inflate::run(data, f.start, f.end, &mut sink, &mut trace, from) {
                    Ok(Ended::Stopped(_)) => {
                        let from = sink.stopped.take();
                        // Summed this far and no further: past the cap the
                        // pass goes back to the last checkpoint, and the sums
                        // have to go back with it.
                        if let Some(at) = sink.marks.as_ref().and_then(|m| m.latest.as_ref()).map(|c| c.out) {
                            sink.sum_to(at);
                        }
                        self.phase = Phase::Whole { sink, trace, from };
                        Ok(Stepped::Working)
                    }
                    Ok(Ended::Stream(_)) => {
                        let sums = sink.sums.expect("set when the pass started");
                        let out = sink.hist;
                        let rest = &out[sums.upto as usize..];
                        let crc = || crc32_update(sums.crc, rest);
                        let adler = || adler32_update(sums.adler, rest);
                        if !inflate::trailer_agrees(&f, data, out.len() as u64, crc, adler) {
                            return Err(Refusal::Failed);
                        }
                        if f.wrapper != Wrapper::Raw {
                            trace.push(f.end, out.len() as u64, StepKind::Header(StepField::Wrapper, 0));
                            trace.finish_at(data.len() as u64 * 8, out.len() as u64);
                        }
                        Ok(Stepped::Done(Decoded::Whole(out, trace.done())))
                    }
                    Err(Refusal::TooLarge) => {
                        self.phase = self.past_the_cap(sink);
                        Ok(Stepped::Working)
                    }
                    Err(other) => Err(other),
                }
            }
            Phase::Index { mut sink, from } => {
                sink.slice_end = sink.len().saturating_add(out_bytes.max(1));
                match inflate::run(data, f.start, f.end, &mut sink, &mut TraceBuilder::off(), Some(from)) {
                    Ok(Ended::Stopped(_)) => {
                        let from = sink.stopped.take().expect("only a step's end stops this decode");
                        self.phase = Phase::Index { sink, from };
                        Ok(Stepped::Working)
                    }
                    Ok(Ended::Stream(blocks_end)) => self.finish(sink, blocks_end).map(Stepped::Done),
                    Err(Refusal::TooLarge) => Err(Refusal::TooLargeLazy),
                    Err(other) => Err(other),
                }
            }
            Phase::Over => Err(Refusal::Failed),
        }
    }

    /// The output has passed the cap. Go back to the last checkpoint, which
    /// is at most the spacing before where the decode stopped, and carry on
    /// from there keeping the window and nothing more, with the sums taken
    /// over the output as it leaves the window.
    fn past_the_cap(&self, mut whole: WindowSink) -> Phase {
        let marks = whole.marks.take().expect("the first pass keeps checkpoints");
        let last = marks.latest.clone().expect("checkpoint 0 is taken before any output");
        whole.sum_to(last.out);
        let sums = whole.sums;
        // The decode from the checkpoint records again every block that
        // starts at or after it, so those are dropped here.
        let mut blocks = whole.blocks.take().expect("the first pass records blocks");
        blocks.truncate(last.blocks());
        drop(whole);
        let window = last.window();
        let mut sink = WindowSink::new(window.to_vec(), last.out - window.len() as u64, self.limits.lazy, true);
        sink.marks = Some(marks);
        sink.blocks = Some(blocks);
        sink.sums = sums;
        sink.mid_block = true;
        Phase::Index { sink, from: last.resume() }
    }

    /// The index, once the decode has reached the end of the stream, which
    /// its last block ends at bit `blocks_end` of.
    fn finish(&self, mut sink: WindowSink, blocks_end: u64) -> Result<Decoded, Refusal> {
        let len = sink.len();
        sink.sum_to(len);
        let sums = sink.sums.expect("set when the pass started");
        if !inflate::trailer_agrees(&self.framing, &self.packed, len, || sums.crc, || sums.adler) {
            return Err(Refusal::Failed);
        }
        let checkpoints = sink.marks.take().expect("set when the pass started").list;
        let mut s = seekable(self.codec, self.packed.clone(), Kind::Deflate(self.framing), len, checkpoints, self.limits);
        s.blocks = sink.blocks.take().expect("set when the pass started");
        s.blocks_end = blocks_end;
        s.crc32 = Some(sums.crc);
        s.adler32 = Some(sums.adler);
        Ok(Decoded::Indexed(s))
    }
}

/// How far an [`Indexing`] has got: `read_bits` of the run's `run_bits`, and
/// `unpacked` bytes of output so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub read_bits: u64,
    pub run_bits: u64,
    pub unpacked: u64,
}

/// A place a decode can start again from: for deflate, see
/// [`Sink::boundary`]; for zstd and xz, the start of a frame or a block.
#[derive(Debug, Clone)]
struct Checkpoint {
    /// The bit of the run the next code starts at, or the first bit of the
    /// frame or block. A span's bits run from its checkpoint's to the next.
    in_bit: u64,
    /// How many bytes of output come before it.
    out: u64,
    start: Start,
}

/// What a decode starting at a checkpoint needs besides where it is.
#[derive(Debug, Clone)]
enum Start {
    Deflate {
        /// The header of the Huffman-coded block it is inside, whose tables
        /// a decode starting here reads again. Nothing at the start of a
        /// block.
        block: Option<u64>,
        /// How many deflate blocks start before `in_bit`. This counts the
        /// block the checkpoint is inside, and does not count a block
        /// starting at `in_bit`.
        blocks: u64,
        /// The last 32 KiB of output before it, or all of it when there is
        /// less.
        window: Arc<[u8]>,
    },
    /// The start of a zstd frame or an xz block, which copies nothing from
    /// before it. The number is the xz block's, counted in the stream's index.
    Unit(usize),
}

impl Checkpoint {
    fn window(&self) -> &[u8] {
        match &self.start {
            Start::Deflate { window, .. } => window,
            Start::Unit(_) => &[],
        }
    }

    fn block(&self) -> Option<u64> {
        match self.start {
            Start::Deflate { block, .. } => block,
            Start::Unit(_) => None,
        }
    }

    /// How many deflate blocks start before it. None of a zstd or xz run's.
    fn blocks(&self) -> u64 {
        match self.start {
            Start::Deflate { blocks, .. } => blocks,
            Start::Unit(_) => 0,
        }
    }

    fn resume(&self) -> Resume {
        Resume { in_bit: self.in_bit, block: self.block() }
    }

    fn unit(&self) -> usize {
        match self.start {
            Start::Unit(n) => n,
            Start::Deflate { .. } => 0,
        }
    }
}

/// The checkpoints an index pass has taken, and where it takes the next.
struct Checkpoints {
    list: Vec<Checkpoint>,
    /// The checkpoint taken most recently, even when thinning dropped it from
    /// `list`: where the pass goes back to once the output passes the cap.
    latest: Option<Checkpoint>,
    spacing: u64,
    next: u64,
    max: usize,
}

impl Checkpoints {
    fn new(spacing: u64, max: usize) -> Checkpoints {
        Checkpoints { list: Vec::new(), latest: None, spacing: spacing.max(1), next: 0, max: max.max(2) }
    }

    /// Keep a checkpoint at `out`, if one is due there. Past the most there
    /// may be, every second one goes and the spacing doubles. The first is
    /// never one that goes: a span starting anywhere else would have nothing
    /// to start from.
    fn offer(&mut self, in_bit: u64, out: u64, start: impl FnOnce() -> Start) {
        if out < self.next {
            return;
        }
        let taken = Checkpoint { in_bit, out, start: start() };
        self.latest = Some(taken.clone());
        self.list.push(taken);
        self.next = out + self.spacing;
        if self.list.len() > self.max {
            let mut i = 0;
            self.list.retain(|_| {
                i += 1;
                (i - 1) % 2 == 0
            });
            self.spacing *= 2;
            self.next = self.list.last().map_or(0, |c| c.out) + self.spacing;
        }
    }
}

/// Where each block starts, as a bit of the run and a byte of the output,
/// recorded by the index pass. Holds every block's start until there are more
/// than `max`, and from then on the start of every `stride`th block. Block 0's
/// start is always kept.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BlockMarks {
    /// The start of blocks 0, `stride`, 2 × `stride` and so on.
    list: Vec<(u64, u64)>,
    /// How many blocks have started, kept or not.
    count: u64,
    stride: u64,
    max: usize,
}

impl BlockMarks {
    fn new(max: usize) -> BlockMarks {
        BlockMarks { list: Vec::new(), count: 0, stride: 1, max: max.max(2) }
    }

    /// Record a block that starts at `in_bit`, with `out` bytes of output
    /// before it. When the list grows past `max`, every second start is
    /// dropped and the stride doubles, so the list keeps the starts of the
    /// blocks whose number is a multiple of the stride.
    fn record(&mut self, in_bit: u64, out: u64) {
        if self.count % self.stride == 0 {
            self.list.push((in_bit, out));
            if self.list.len() > self.max {
                let mut i = 0;
                self.list.retain(|_| {
                    i += 1;
                    (i - 1) % 2 == 0
                });
                self.stride *= 2;
            }
        }
        self.count += 1;
    }

    /// Keep only the first `count` blocks, for a decode that starts again
    /// from a checkpoint after them and records the later blocks again.
    fn truncate(&mut self, count: u64) {
        self.list.truncate(count.div_ceil(self.stride) as usize);
        self.count = count;
    }
}

/// The running CRC-32 and Adler-32 of the output, taken up to `upto`.
#[derive(Debug, Clone, Copy)]
struct Sums {
    crc: u32,
    adler: u32,
    upto: u64,
}

/// Output kept from some point of the stream on: all of it, or once `slide`
/// is set, the last [`WINDOW`] of it and whatever has come since the last
/// slide.
struct WindowSink {
    hist: Vec<u8>,
    /// Where `hist[0]` is in the whole stream's output.
    base: u64,
    /// The most the output may come to.
    cap: u64,
    slide: bool,
    /// Take checkpoints on the way.
    marks: Option<Checkpoints>,
    /// Record where blocks start on the way.
    blocks: Option<BlockMarks>,
    /// Sum the output as it leaves.
    sums: Option<Sums>,
    /// Stop at the first boundary at or past this bit.
    stop_at: Option<u64>,
    /// Stop at the first boundary once the output has come to this, which is
    /// where a step of an [`Indexing`] ends, and say where in `stopped`.
    slice_end: u64,
    /// Whether that may be between two codes of a block, or only at the start
    /// of one.
    mid_block: bool,
    stopped: Option<Resume>,
}

impl WindowSink {
    fn new(hist: Vec<u8>, base: u64, cap: u64, slide: bool) -> WindowSink {
        WindowSink { hist, base, cap, slide, marks: None, blocks: None, sums: None, stop_at: None, slice_end: u64::MAX, mid_block: false, stopped: None }
    }

    /// Bring the sums up to `upto`, from what `hist` still holds.
    fn sum_to(&mut self, upto: u64) {
        let Some(s) = &mut self.sums else { return };
        if upto <= s.upto {
            return;
        }
        let bytes = &self.hist[(s.upto - self.base) as usize..(upto - self.base) as usize];
        s.crc = crc32_update(s.crc, bytes);
        s.adler = adler32_update(s.adler, bytes);
        s.upto = upto;
    }

    #[inline]
    fn slid(&mut self) {
        if self.slide && self.hist.len() >= SLIDE_AT {
            let cut = self.hist.len() - WINDOW;
            self.sum_to(self.base + cut as u64);
            self.hist.drain(..cut);
            self.base += cut as u64;
        }
    }
}

impl Sink for WindowSink {
    #[inline]
    fn len(&self) -> u64 {
        self.base + self.hist.len() as u64
    }

    #[inline]
    fn literal(&mut self, byte: u8) -> Result<(), Refusal> {
        if self.len() >= self.cap {
            return Err(Refusal::TooLarge);
        }
        self.hist.push(byte);
        self.slid();
        Ok(())
    }

    #[inline]
    fn copy(&mut self, dist: u32, len: u32) -> Result<(), Refusal> {
        if dist as usize > self.hist.len() {
            return Err(Refusal::Failed);
        }
        if self.len() + len as u64 > self.cap {
            return Err(Refusal::TooLarge);
        }
        let from = self.hist.len() - dist as usize;
        for k in 0..len as usize {
            let byte = self.hist[from + k];
            self.hist.push(byte);
        }
        self.slid();
        Ok(())
    }

    fn stored(&mut self, bytes: &[u8]) -> Result<(), Refusal> {
        if self.len() + bytes.len() as u64 > self.cap {
            return Err(Refusal::TooLarge);
        }
        self.hist.extend_from_slice(bytes);
        self.slid();
        Ok(())
    }

    #[inline]
    fn boundary(&mut self, in_bit: u64, block: Option<u64>) -> ControlFlow<()> {
        if self.stop_at.is_some_and(|stop| in_bit >= stop) {
            return ControlFlow::Break(());
        }
        let out = self.len();
        if let Some(marks) = &mut self.marks {
            let keep = (WINDOW as u64).min(out) as usize;
            let window = &self.hist[self.hist.len() - keep..];
            let blocks = self.blocks.as_ref().map_or(0, |b| b.count);
            marks.offer(in_bit, out, || Start::Deflate { block, blocks, window: window.into() });
        }
        // After the checkpoint, so that a step ending here takes the one a
        // pass in one go would have taken, and the step after does not take
        // it again.
        if out >= self.slice_end && (block.is_none() || self.mid_block) {
            self.stopped = Some(Resume { in_bit, block });
            return ControlFlow::Break(());
        }
        // After the stop, unlike the checkpoint: a step that stops at the
        // start of a block leaves the block to the step after, which is told
        // of the same start again and records it then. A step that stops
        // inside a block is not told of its start again, since the decode
        // carrying on reads the block's header without calling here.
        if let (None, Some(blocks)) = (block, &mut self.blocks) {
            blocks.record(in_bit, out);
        }
        ControlFlow::Continue(())
    }
}

/// A compressed run that comes to more than can be held whole, read through
/// its checkpoints.
///
/// Holds the run itself, which is at most [`CAP_BYTES`]; the checkpoints; where
/// the blocks start; the spans decoded most recently, up to
/// [`Limits::span_cache`]; the trace of the span asked about most recently;
/// and the traces of the blocks asked about most recently, up to
/// [`Limits::span_steps`] steps between them. Reads take `&self`, as a
/// [`Source`](crate::source::Source) is read, so the caches are behind locks.
pub struct Seekable {
    codec: Codec,
    packed: Arc<Vec<u8>>,
    kind: Kind,
    len: u64,
    checkpoints: Vec<Checkpoint>,
    /// Where each deflate block starts. Empty for zstd and xz.
    blocks: BlockMarks,
    /// Where the last deflate block ends, as a bit of the run.
    blocks_end: u64,
    crc32: Option<u32>,
    adler32: Option<u32>,
    limits: Limits,
    spans: Mutex<Recent<Arc<Vec<u8>>>>,
    traced: Mutex<Option<(usize, Arc<Trace>)>>,
    /// Block starts found again by decoding: one list for each kept start,
    /// keyed by that start's place in `blocks.list`.
    groups: Mutex<Recent<Arc<[(u64, u64)]>>>,
    block_traces: Mutex<Recent<Arc<Trace>>>,
}

/// Which decoder a run's spans go through, and what it needs to know about the
/// run as a whole.
enum Kind {
    /// Where the deflate data starts and ends inside the run's wrapper.
    Deflate(Framing),
    Zstd,
    /// Every block, from the stream's index.
    Xz(xz::Layout),
}

impl std::fmt::Debug for Seekable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Seekable")
            .field("codec", &self.codec)
            .field("packed", &self.packed.len())
            .field("len", &self.len)
            .field("checkpoints", &self.checkpoints.len())
            .field("blocks", &self.blocks.count)
            .finish()
    }
}

/// A lock that a panic while it was held does not make unusable: what it
/// guards is a cache, and a cache in any state is still a cache.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Seekable {
    pub fn codec(&self) -> Codec {
        self.codec
    }

    /// How many bytes the run comes to.
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The compressed run, from its first byte, which is what the bits of
    /// every trace count from.
    pub fn packed(&self) -> &Arc<Vec<u8>> {
        &self.packed
    }

    /// The CRC-32 of the whole output, worked out on the pass that indexed
    /// it. Only a deflate run's pass sums its output, since gzip's trailer
    /// holds this sum; nothing for zstd and xz.
    pub fn crc32(&self) -> Option<u32> {
        self.crc32
    }

    /// The Adler-32 of the whole output, the same way, which zlib's trailer
    /// holds.
    pub fn adler32(&self) -> Option<u32> {
        self.adler32
    }

    /// The container's own pieces of the run, as a whole trace would declare
    /// them: an xz stream's blocks, which the index gives without decoding
    /// any. See [`Member`]. Empty for every other codec.
    pub fn members(&self) -> Vec<Member> {
        match &self.kind {
            Kind::Xz(l) => xz::members(l),
            Kind::Deflate(_) | Kind::Zstd => Vec::new(),
        }
    }

    /// How many spans the output is divided into, one per checkpoint.
    pub fn spans(&self) -> usize {
        self.checkpoints.len()
    }

    /// The output bytes span `k` covers.
    pub fn span_bytes(&self, k: usize) -> std::ops::Range<u64> {
        let end = self.checkpoints.get(k + 1).map_or(self.len, |c| c.out);
        self.checkpoints[k].out..end
    }

    /// The span holding output byte `byte`. A span of no bytes holds none, so
    /// the answer is the last span starting at or before it.
    pub fn span_of_byte(&self, byte: u64) -> Option<usize> {
        if byte >= self.len {
            return None;
        }
        Some(self.checkpoints.partition_point(|c| c.out <= byte).saturating_sub(1))
    }

    /// The span that read bit `bit` of the run: the one whose checkpoints
    /// either side hold it. A span's trace reaches back over the header of
    /// the block its checkpoint is inside, and those bits are still the span
    /// before's, which read them as codes. The wrapper's header is span 0's
    /// and its trailer is the last span's.
    pub fn span_of_bit(&self, bit: u64) -> Option<usize> {
        if bit >= self.packed.len() as u64 * 8 {
            return None;
        }
        Some(self.checkpoints.partition_point(|c| c.in_bit <= bit).saturating_sub(1))
    }

    /// Bytes `at..at + out.len()` of the output.
    pub fn read(&self, at: u64, out: &mut [u8]) -> Result<(), Refusal> {
        let end = at.checked_add(out.len() as u64).ok_or(Refusal::Failed)?;
        if end > self.len {
            return Err(Refusal::Failed);
        }
        let mut pos = at;
        while pos < end {
            let k = self.span_of_byte(pos).ok_or(Refusal::Failed)?;
            let range = self.span_bytes(k);
            let bytes = self.span(k)?;
            let take = (range.end.min(end) - pos) as usize;
            let from = (pos - range.start) as usize;
            let to = (pos - at) as usize;
            out[to..to + take].copy_from_slice(&bytes[from..from + take]);
            pos += take as u64;
        }
        Ok(())
    }

    /// The bytes of the run span `k` reads, for a span that starts on a byte:
    /// from its checkpoint's bit to the next one's, and the last to the end.
    fn span_in(&self, k: usize) -> std::ops::Range<usize> {
        let end = self.checkpoints.get(k + 1).map_or(self.packed.len(), |c| (c.in_bit / 8) as usize);
        (self.checkpoints[k].in_bit / 8) as usize..end
    }

    /// The xz blocks span `k` holds, as numbers in the stream's index.
    fn span_units(&self, k: usize, l: &xz::Layout) -> std::ops::Range<usize> {
        let end = self.checkpoints.get(k + 1).map_or(l.blocks.len(), |c| c.unit());
        self.checkpoints[k].unit()..end
    }

    /// Everything span `k` comes to, from the cache or decoded now.
    fn span(&self, k: usize) -> Result<Arc<Vec<u8>>, Refusal> {
        if let Some(bytes) = lock(&self.spans).get(k as u64) {
            return Ok(bytes);
        }
        let bytes = Arc::new(self.decode_span(k)?);
        lock(&self.spans).put(k as u64, bytes.clone(), bytes.len());
        Ok(bytes)
    }

    /// Decode span `k` from its own checkpoint to the next one.
    ///
    /// For deflate and zstd it is the same decoder over the same bits as the
    /// pass that indexed it, so a span that comes out another length is a
    /// decoder disagreeing with itself. For xz it is the first time the
    /// span's blocks have been decoded, and a block that does not come to
    /// what the index says is refused here.
    fn decode_span(&self, k: usize) -> Result<Vec<u8>, Refusal> {
        let c = &self.checkpoints[k];
        let range = self.span_bytes(k);
        let want = (range.end - range.start) as usize;
        let bytes = match &self.kind {
            Kind::Deflate(f) => {
                let window = c.window();
                let mut hist = Vec::with_capacity(window.len() + want);
                hist.extend_from_slice(window);
                let mut sink = WindowSink::new(hist, c.out - window.len() as u64, range.end, false);
                sink.stop_at = self.checkpoints.get(k + 1).map(|n| n.in_bit);
                inflate::run(&self.packed, f.start, f.end, &mut sink, &mut TraceBuilder::off(), Some(c.resume()))?;
                if sink.len() != range.end {
                    return Err(Refusal::Failed);
                }
                let mut bytes = sink.hist;
                bytes.drain(..window.len());
                bytes
            }
            Kind::Zstd => {
                let mut bytes = Vec::with_capacity(want);
                let frame_cap = self.limits.span as u64;
                frames::zstd_walk(&self.packed, self.span_in(k), c.out, frame_cap, &mut TraceBuilder::off(), &mut |_, _| {}, &mut |b| {
                    if bytes.len() + b.len() > want {
                        return Err(Refusal::Failed);
                    }
                    bytes.extend_from_slice(b);
                    Ok(())
                })?;
                bytes
            }
            Kind::Xz(l) => xz::blocks(&self.packed, l, self.span_units(k, l), c.out, 0, &mut TraceBuilder::off())?,
        };
        if bytes.len() != want {
            return Err(Refusal::Failed);
        }
        Ok(bytes)
    }

    /// The trace of span `k`: every step from where its decode starts to
    /// where the next span's does, counted from the front of the run and of
    /// the output as a whole trace counts. Kept until another span's is asked
    /// for.
    ///
    /// A deflate decode starting inside a block reads the block's header and
    /// tables again, so the trace starts at that header and has them, and
    /// then one opaque step over the codes between the tables and the
    /// checkpoint, which made bytes of the span before. A zstd or xz span's
    /// trace is the one a whole run's has over the same frames or blocks,
    /// with the stream's header in span 0 and its index and footer in the
    /// last span.
    pub fn span_trace(&self, k: usize) -> Result<Arc<Trace>, Refusal> {
        if let Some((held, t)) = lock(&self.traced).as_ref() {
            if *held == k {
                return Ok(t.clone());
            }
        }
        let c = self.checkpoints.get(k).ok_or(Refusal::Failed)?;
        let t = Arc::new(match &self.kind {
            Kind::Deflate(f) => self.deflate_span_trace(k, c, f)?,
            Kind::Zstd => {
                let range = self.span_in(k);
                let mut b = TraceBuilder::with_budget(MAX_STEPS).starting_at(range.start as u64 * 8, c.out);
                let frame_cap = self.limits.span as u64;
                let end = frames::zstd_walk(&self.packed, range.clone(), c.out, frame_cap, &mut b, &mut |_, _| {}, &mut |_| Ok(()))?;
                b.finish_at(range.end as u64 * 8, end);
                b.done()
            }
            Kind::Xz(l) => {
                let range = self.span_in(k);
                let units = self.span_units(k, l);
                let last = units.end == l.blocks.len();
                let mut b = TraceBuilder::with_budget(MAX_STEPS).starting_at(range.start as u64 * 8, c.out);
                if k == 0 {
                    b.push(0, 0, StepKind::Header(StepField::FrameHeader, 0));
                }
                let end = c.out + xz::blocks(&self.packed, l, units, c.out, MAX_STEPS, &mut b)?.len() as u64;
                if last {
                    xz::push_index(l, &mut b, end);
                }
                b.finish_at(range.end as u64 * 8, end);
                b.done()
            }
        });
        *lock(&self.traced) = Some((k, t.clone()));
        Ok(t)
    }

    fn deflate_span_trace(&self, k: usize, c: &Checkpoint, f: &Framing) -> Result<Trace, Refusal> {
        let wrapped = f.wrapper != Wrapper::Raw;
        let first = match (k, wrapped) {
            (0, true) => 0,
            _ => c.block().unwrap_or(c.in_bit),
        };
        let mut b = TraceBuilder::with_budget(self.limits.span_steps).starting_at(first, c.out);
        if k == 0 && wrapped {
            b.push(0, 0, StepKind::Header(StepField::Wrapper, 0));
        }
        let mut sink = CountSink { len: c.out, cap: u64::MAX, stop_at: self.checkpoints.get(k + 1).map(|n| n.in_bit), starts: None };
        let ended = inflate::run(&self.packed, f.start, f.end, &mut sink, &mut b, Some(c.resume()))?;
        if matches!(ended, Ended::Stream(_)) && wrapped {
            b.push(f.end, sink.len, StepKind::Header(StepField::Wrapper, 0));
            b.finish_at(self.packed.len() as u64 * 8, sink.len);
        }
        Ok(b.done())
    }

    /// The trace that holds output byte `byte`: its span's.
    pub fn trace_at_byte(&self, byte: u64) -> Option<Arc<Trace>> {
        self.span_trace(self.span_of_byte(byte)?).ok()
    }

    /// The trace that names bit `bit` of the run: the span's that read it.
    pub fn trace_at_bit(&self, bit: u64) -> Option<Arc<Trace>> {
        self.span_trace(self.span_of_bit(bit)?).ok()
    }

    /// Which step produced byte `byte` of the output, as
    /// [`Trace::map_out`] says for a whole run.
    pub fn map_out(&self, byte: u64) -> Option<Step> {
        self.trace_at_byte(byte)?.map_out(byte)
    }

    /// Which step read bit `bit` of the run, as [`Trace::map_in`] says for a
    /// whole run. Asked of the trace of the block holding the bit, which is
    /// the trace the listing and the inspector take a code from, and of the
    /// span's trace for a bit outside every block.
    pub fn map_in(&self, bit: u64) -> Option<Step> {
        match self.block_of_bit(bit) {
            Some(i) => self.block_trace(i).ok()?.map_in(bit),
            None => self.trace_at_bit(bit)?.map_in(bit),
        }
    }

    /// How many blocks the run holds.
    pub fn blocks(&self) -> u64 {
        self.blocks.count
    }

    /// Block `i`: where it is in the run and in the output, how it is coded
    /// and whether it says it is the last. Its `steps` are empty; they are in
    /// [`Seekable::block_trace`].
    pub fn block(&self, i: u64) -> Result<Block, Refusal> {
        let (in_start, out_start) = self.block_start(i)?;
        let (in_end, out_end) = match i + 1 < self.blocks.count {
            true => self.block_start(i + 1)?,
            false => (self.blocks_end, self.len),
        };
        // BFINAL and BTYPE, the first three bits of the block header, low bit
        // first, read from the run, which is in memory.
        let bit = |k: u64| self.packed.get((k / 8) as usize).map_or(0, |b| (b >> (k % 8)) & 1);
        let kind = match bit(in_start + 1) | bit(in_start + 2) << 1 {
            0 => BlockKind::Stored,
            1 => BlockKind::Fixed,
            2 => BlockKind::Dynamic,
            _ => return Err(Refusal::Failed),
        };
        Ok(Block { steps: 0..0, kind, last: bit(in_start) == 1, in_bits: in_start..in_end, out_bytes: out_start..out_end })
    }

    /// The block that bit `bit` of the run belongs to. None for a bit of the
    /// gzip or zlib wrapper, or of the padding after the last block.
    pub fn block_of_bit(&self, bit: u64) -> Option<u64> {
        let list = &self.blocks.list;
        if bit < list.first()?.0 || bit >= self.blocks_end {
            return None;
        }
        let g = list.partition_point(|m| m.0 <= bit) - 1;
        if self.blocks.stride == 1 {
            return Some(g as u64);
        }
        let starts = self.group(g).ok()?;
        Some(g as u64 * self.blocks.stride + starts.partition_point(|m| m.0 <= bit) as u64 - 1)
    }

    /// Where the deflate data is inside the run's wrapper. Refused for a zstd
    /// or xz run, which has no deflate blocks.
    fn framing(&self) -> Result<&Framing, Refusal> {
        match &self.kind {
            Kind::Deflate(f) => Ok(f),
            Kind::Zstd | Kind::Xz(_) => Err(Refusal::Failed),
        }
    }

    /// Where block `i` starts: the bit of its header and how much output
    /// comes before it.
    fn block_start(&self, i: u64) -> Result<(u64, u64), Refusal> {
        if i >= self.blocks.count {
            return Err(Refusal::Failed);
        }
        let stride = self.blocks.stride;
        let (g, j) = ((i / stride) as usize, (i % stride) as usize);
        if j == 0 {
            return Ok(self.blocks.list[g]);
        }
        self.group(g)?.get(j).copied().ok_or(Refusal::Failed)
    }

    /// The start of every block from kept start `g` up to the next kept start,
    /// found by decoding from start `g` and counting the output.
    fn group(&self, g: usize) -> Result<Arc<[(u64, u64)]>, Refusal> {
        if let Some(starts) = lock(&self.groups).get(g as u64) {
            return Ok(starts);
        }
        let &(in_bit, out) = self.blocks.list.get(g).ok_or(Refusal::Failed)?;
        let mut sink = CountSink { len: out, cap: u64::MAX, stop_at: self.blocks.list.get(g + 1).map(|m| m.0), starts: Some(Vec::new()) };
        let f = self.framing()?;
        inflate::run(&self.packed, f.start, f.end, &mut sink, &mut TraceBuilder::off(), Some(Resume { in_bit, block: None }))?;
        let starts: Arc<[(u64, u64)]> = sink.starts.take().unwrap_or_default().into();
        // The same decoder read these bits on the index pass, so a different
        // count is the decoder disagreeing with itself.
        let want = self.blocks.stride.min(self.blocks.count - g as u64 * self.blocks.stride);
        if starts.len() as u64 != want || starts.first() != Some(&(in_bit, out)) {
            return Err(Refusal::Failed);
        }
        lock(&self.groups).put(g as u64, starts.clone(), 1);
        Ok(starts)
    }

    /// The trace of block `i` alone: its header, its tables and its codes,
    /// with bits and bytes counted from the start of the run and of the
    /// output, as in every trace here. [`Trace::blocks`] has this block as its
    /// only entry. The last block's trace also covers the padding after it, up
    /// to the end of the deflate data.
    ///
    /// In a block of more than [`Limits::span_steps`] codes, the codes are one
    /// opaque step, as they are in a whole run's trace past
    /// [`crate::codec::MAX_STEPS`].
    pub fn block_trace(&self, i: u64) -> Result<Arc<Trace>, Refusal> {
        if let Some(t) = lock(&self.block_traces).get(i) {
            return Ok(t);
        }
        let block = self.block(i)?;
        let (in_bit, out) = (block.in_bits.start, block.out_bytes.start);
        let next = (i + 1 < self.blocks.count).then_some(block.in_bits.end);
        let mut sink = CountSink { len: out, cap: u64::MAX, stop_at: next, starts: None };
        let mut b = TraceBuilder::with_budget(self.limits.span_steps).starting_at(in_bit, out);
        let f = self.framing()?;
        inflate::run(&self.packed, f.start, f.end, &mut sink, &mut b, Some(Resume { in_bit, block: None }))?;
        let t = b.done();
        match t.blocks() {
            [one] if one.in_bits == block.in_bits && one.out_bytes == block.out_bytes => {}
            _ => return Err(Refusal::Failed),
        }
        let t = Arc::new(t);
        lock(&self.block_traces).put(i, t.clone(), t.len());
        Ok(t)
    }

    /// How many bytes the checkpoints' windows, the block starts and the span
    /// cache hold, for tests and for saying what a run costs.
    pub fn held_bytes(&self) -> usize {
        self.checkpoints.iter().map(|c| c.window().len()).sum::<usize>()
            + self.blocks.list.len() * std::mem::size_of::<(u64, u64)>()
            + lock(&self.spans).weight
    }

    /// Where each checkpoint is, as the bit of the run and the byte of the
    /// output, for tests.
    #[cfg(test)]
    fn marks(&self) -> Vec<(u64, u64, Option<u64>)> {
        self.checkpoints.iter().map(|c| (c.in_bit, c.out, c.block())).collect()
    }
}

/// A cache of what was made most recently for one run, up to a total weight,
/// that drops the entry used longest ago first. Spans are weighed in bytes,
/// block traces in steps, and each list of block starts counts as 1. The
/// newest entry is kept whatever it weighs.
struct Recent<T> {
    entries: Vec<(u64, T, usize, u64)>,
    weight: usize,
    cap: usize,
    clock: u64,
    #[cfg(test)]
    peak: usize,
}

impl<T: Clone> Recent<T> {
    fn new(cap: usize) -> Recent<T> {
        Recent {
            entries: Vec::new(),
            weight: 0,
            cap,
            clock: 0,
            #[cfg(test)]
            peak: 0,
        }
    }

    fn get(&mut self, k: u64) -> Option<T> {
        self.clock += 1;
        let clock = self.clock;
        let e = self.entries.iter_mut().find(|e| e.0 == k)?;
        e.3 = clock;
        Some(e.1.clone())
    }

    fn put(&mut self, k: u64, value: T, weight: usize) {
        while !self.entries.is_empty() && self.weight + weight > self.cap {
            let oldest = (0..self.entries.len()).min_by_key(|&i| self.entries[i].3).expect("not empty");
            let gone = self.entries.swap_remove(oldest);
            self.weight -= gone.2;
        }
        self.clock += 1;
        self.weight += weight;
        #[cfg(test)]
        {
            self.peak = self.peak.max(self.weight);
        }
        self.entries.push((k, value, weight, self.clock));
    }
}

// Reachable from `eval::space`'s tests, which open runs like these through
// the templates.
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::checksum::{adler32, crc32};

    /// Small enough that a few hundred kilobytes reach every limit: the run is
    /// indexed past 64 KiB, checkpoints are 8 KiB apart until there are more
    /// than 12 of them, and the cache holds 40 KiB.
    fn small() -> Limits {
        Limits { whole: 64 << 10, lazy: 4 << 20, spacing: 8 << 10, checkpoints: 12, span_cache: 40 << 10, span_steps: 1_000_000, whole_steps: crate::codec::MAX_STEPS, block_marks: MAX_BLOCK_MARKS, span: 64 << 10 }
    }

    fn rand_from(mut seed: u64) -> impl FnMut() -> u64 {
        move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        }
    }

    /// Words, runs of one byte and noise, in stretches, so that an encoder
    /// writes literals, short and long matches, and blocks of every kind.
    pub(crate) fn mixed(len: usize, seed: u64) -> Vec<u8> {
        let mut rand = rand_from(seed);
        let words = ["deflate ", "window ", "checkpoint ", "span ", "the ", "of ", "Rubin ", "FITS "];
        let mut out = Vec::with_capacity(len);
        while out.len() < len {
            match rand() % 4 {
                0 => {
                    for _ in 0..rand() % 400 {
                        out.extend_from_slice(words[(rand() % 8) as usize].as_bytes());
                    }
                }
                1 => out.extend(std::iter::repeat_n((rand() % 256) as u8, (rand() % 5000) as usize)),
                2 => out.extend((0..rand() % 3000).map(|_| rand() as u8)),
                _ => out.extend((0..rand() % 2000).map(|i| (i / 7) as u8)),
            }
        }
        out.truncate(len);
        out
    }

    fn gzip_of(data: &[u8], level: u8) -> Vec<u8> {
        gzip_around(data, &miniz_oxide::deflate::compress_to_vec(data, level))
    }

    fn gzip_around(data: &[u8], raw: &[u8]) -> Vec<u8> {
        let mut out = vec![0x1f, 0x8b, 8, 0x08, 0, 0, 0, 0, 0, 3];
        out.extend_from_slice(b"name.fits\0");
        out.extend_from_slice(raw);
        out.extend_from_slice(&crc32(data).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out
    }

    /// Raw deflate flushed every `every` bytes of input, as a writer that
    /// flushes each line does: each flush ends a block and adds an empty
    /// stored one, so a few hundred kilobytes come to hundreds of blocks.
    fn flushed(data: &[u8], level: u8, every: usize) -> Vec<u8> {
        use miniz_oxide::deflate::core::{compress, create_comp_flags_from_zip_params, CompressorOxide, TDEFLFlush};
        let mut c = CompressorOxide::new(create_comp_flags_from_zip_params(level as i32, -15, 0));
        let mut out = Vec::new();
        let mut buf = vec![0u8; every * 2 + 4096];
        let chunks: Vec<&[u8]> = data.chunks(every).collect();
        for (k, chunk) in chunks.iter().enumerate() {
            let flush = if k + 1 == chunks.len() { TDEFLFlush::Finish } else { TDEFLFlush::Sync };
            let mut input = *chunk;
            loop {
                let (_, read, wrote) = compress(&mut c, input, &mut buf, flush);
                out.extend_from_slice(&buf[..wrote]);
                input = &input[read..];
                if input.is_empty() && wrote < buf.len() {
                    break;
                }
            }
        }
        out
    }

    /// Raw deflate of `data` with a block ended every `chunk` bytes, by a
    /// sync flush, which also writes an empty stored block. miniz writes 50 KB
    /// as one block, and a pass under the cap only stops between blocks.
    fn deflate_in_blocks(data: &[u8], chunk: usize) -> Vec<u8> {
        use miniz_oxide::deflate::core::{compress, create_comp_flags_from_zip_params, CompressorOxide, TDEFLFlush, TDEFLStatus};
        let mut c = CompressorOxide::new(create_comp_flags_from_zip_params(6, -15, 0));
        let mut buf = vec![0u8; 4 * chunk + 1024];
        let mut out = Vec::new();
        let pieces: Vec<&[u8]> = data.chunks(chunk).collect();
        for (i, piece) in pieces.iter().enumerate() {
            let flush = if i + 1 == pieces.len() { TDEFLFlush::Finish } else { TDEFLFlush::Sync };
            let (status, read, written) = compress(&mut c, piece, &mut buf, flush);
            assert!(read == piece.len() && matches!(status, TDEFLStatus::Okay | TDEFLStatus::Done), "{status:?}");
            out.extend_from_slice(&buf[..written]);
        }
        out
    }

    /// `raw` with the zlib or gzip wrapper round it, `data` being what it
    /// inflates to.
    fn wrapped(codec: Codec, raw: &[u8], data: &[u8]) -> Vec<u8> {
        match codec {
            Codec::Zlib => [&[0x78, 0x9c][..], raw, &adler32(data).to_be_bytes()].concat(),
            Codec::Gzip => {
                let head = [0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3];
                [&head[..], raw, &crc32(data).to_le_bytes(), &(data.len() as u32).to_le_bytes()].concat()
            }
            _ => raw.to_vec(),
        }
    }

    fn indexed(codec: Codec, packed: &[u8], limits: Limits) -> Seekable {
        match decode_or_index_with(codec, Arc::new(packed.to_vec()), limits) {
            Ok(Decoded::Indexed(s)) => s,
            Ok(Decoded::Whole(bytes, _)) => panic!("{} bytes came back whole", bytes.len()),
            Err(why) => panic!("refused: {why:?}"),
        }
    }

    /// Every encoder level and every wrapper, read back a piece at a time:
    /// the whole output in one read, and reads of every length across the
    /// span boundaries, in an order that keeps the cache turning over.
    #[test]
    fn a_large_run_reads_back_as_what_went_in_from_anywhere() {
        let data = mixed(300_000, 7);
        for level in [0u8, 1, 6, 9] {
            let raw = miniz_oxide::deflate::compress_to_vec(&data, level);
            let zlib = miniz_oxide::deflate::compress_to_vec_zlib(&data, level);
            let gzip = gzip_of(&data, level);
            for (codec, packed) in [(Codec::Deflate, &raw), (Codec::Zlib, &zlib), (Codec::Gzip, &gzip)] {
                let s = indexed(codec, packed, small());
                assert_eq!(s.len(), data.len() as u64);
                assert_eq!(s.crc32(), Some(crc32(&data)));
                assert_eq!(s.adler32(), Some(adler32(&data)));
                let mut all = vec![0u8; data.len()];
                s.read(0, &mut all).expect("reads");
                assert!(all == data, "{codec:?} level {level}: the whole output differs");
                let mut rand = rand_from(level as u64 + 99);
                for _ in 0..80 {
                    let at = rand() % data.len() as u64;
                    let n = (rand() % 30_000).min(data.len() as u64 - at) as usize;
                    let mut got = vec![0u8; n];
                    s.read(at, &mut got).expect("reads");
                    assert!(got == data[at as usize..at as usize + n], "{codec:?} level {level}: {n} bytes at {at} differ");
                }
                let peak = lock(&s.spans).peak;
                let largest = (0..s.spans()).map(|k| s.span_bytes(k)).map(|r| r.end - r.start).max().unwrap() as usize;
                assert!(peak <= small().span_cache + largest, "{codec:?} level {level}: the cache held {peak} bytes");
            }
        }
    }

    /// Past the most checkpoints there may be, every second one goes, and the
    /// first stays: it is the only place span 0 can start from.
    #[test]
    fn checkpoints_thin_out_and_the_first_stays() {
        let data = mixed(600_000, 3);
        let s = indexed(Codec::Deflate, &miniz_oxide::deflate::compress_to_vec(&data, 6), small());
        let marks = s.marks();
        assert!(marks.len() <= small().checkpoints, "{} checkpoints", marks.len());
        assert_eq!(marks[0], (0, 0, None));
        assert!(marks.windows(2).all(|w| w[0].0 < w[1].0 && w[0].1 <= w[1].1));
        // Thinned at least once: 600 KB at 8 KiB apart is far more than 12.
        assert!(marks.windows(2).any(|w| w[1].1 - w[0].1 >= 16 << 10));
        // And some of them are inside a block, which is the case that reads
        // a block's tables again.
        assert!(marks.iter().any(|m| m.2.is_some()), "no checkpoint inside a block");
    }

    /// A run under the cap comes back whole, with the same bytes and the same
    /// trace the whole decoders give, step for step.
    #[test]
    fn a_run_under_the_cap_comes_back_as_it_always_did() {
        let data = mixed(50_000, 5);
        let raw = miniz_oxide::deflate::compress_to_vec(&data, 6);
        let zlib = miniz_oxide::deflate::compress_to_vec_zlib(&data, 6);
        let gzip = gzip_of(&data, 6);
        for (codec, packed) in [(Codec::Deflate, &raw), (Codec::Zlib, &zlib), (Codec::Gzip, &gzip)] {
            let (bytes, trace) = crate::codec::decode_traced(codec, packed).expect("decodes");
            let Ok(Decoded::Whole(ours, t)) = decode_or_index_with(codec, Arc::new(packed.clone()), small()) else {
                panic!("{codec:?} did not come back whole");
            };
            assert_eq!(ours, bytes);
            assert_eq!(t.len(), trace.len());
            assert!(t.steps().zip(trace.steps()).all(|(a, b)| a == b), "{codec:?}: the steps differ");
            assert_eq!(t.blocks(), trace.blocks());
            assert_eq!((t.in_bits(), t.out_bytes()), (trace.in_bits(), trace.out_bytes()));
        }
    }

    /// Each span's trace tiles what its decode read and wrote, and every byte
    /// and bit maps to the same step through the spans as through the trace
    /// of the whole run.
    #[test]
    fn span_traces_agree_with_the_whole_trace() {
        let data = mixed(300_000, 11);
        for (codec, packed) in [
            (Codec::Deflate, miniz_oxide::deflate::compress_to_vec(&data, 6)),
            (Codec::Gzip, gzip_of(&data, 9)),
            (Codec::Zlib, miniz_oxide::deflate::compress_to_vec_zlib(&data, 1)),
        ] {
            let (_, whole) = crate::codec::decode_traced(codec, &packed).expect("decodes whole");
            assert!(!whole.coarse());
            let s = indexed(codec, &packed, small());
            for k in 0..s.spans() {
                let t = s.span_trace(k).expect("traces");
                t.check_tiles().unwrap_or_else(|e| panic!("{codec:?} span {k}: {e}"));
                assert_eq!(t.start().1, s.span_bytes(k).start);
                assert_eq!(t.out_bytes(), s.span_bytes(k).end);
            }
            // In order, so that the one span trace kept is the one asked for
            // next nearly every time.
            let mut rand = rand_from(17);
            let mut bytes: Vec<u64> = (0..2000).map(|_| rand() % data.len() as u64).collect();
            let mut bits: Vec<u64> = (0..2000).map(|_| rand() % (packed.len() as u64 * 8)).collect();
            bytes.sort_unstable();
            bits.sort_unstable();
            for byte in bytes {
                assert_eq!(s.map_out(byte), whole.map_out(byte), "{codec:?}: byte {byte}");
            }
            for bit in bits {
                assert_eq!(s.map_in(bit), whole.map_in(bit), "{codec:?}: bit {bit}");
            }
            // Every checkpoint's own bit, and the bits either side, which is
            // where a span's trace and the one before it meet.
            for (in_bit, _, block) in s.marks() {
                for bit in [in_bit.saturating_sub(1), in_bit, in_bit + 1].into_iter().chain(block) {
                    assert_eq!(s.map_in(bit), whole.map_in(bit), "{codec:?}: bit {bit} by checkpoint {in_bit}");
                }
            }
        }
    }

    /// The sidebar's take on one code, rebuilt from a span's trace, says what
    /// it says when rebuilt from the whole trace.
    #[test]
    fn a_code_taken_apart_through_a_span_reads_as_through_the_whole() {
        let data = mixed(300_000, 13);
        let packed = miniz_oxide::deflate::compress_to_vec(&data, 6);
        let (_, whole) = crate::codec::decode_traced(Codec::Deflate, &packed).expect("decodes whole");
        let s = indexed(Codec::Deflate, &packed, small());
        let mut rand = rand_from(23);
        let mut bits: Vec<u64> = (0..3000).map(|_| rand() % (packed.len() as u64 * 8)).collect();
        bits.sort_unstable();
        let mut compared = 0;
        for bit in bits {
            let t = s.trace_at_bit(bit).expect("a span names it");
            let (Some(ours), Some(theirs)) = (t.index_in(bit), whole.index_in(bit)) else { continue };
            let a = inflate::decode_step(&packed, &t, ours);
            let b = inflate::decode_step(&packed, &whole, theirs);
            // The block number and the step that declared a code length are
            // counted in the trace they came from, and the two traces start
            // in different places. The rest, the step's place in its block
            // included, is the same code read the same way.
            let same = |d: inflate::DecodedStep| {
                let plain = |c: inflate::DecodedCode| inflate::DecodedCode { entry: None, ..c };
                (plain(d.symbol), d.distance.map(plain), d.block_kind)
            };
            assert_eq!(a.map(same), b.map(same), "bit {bit}");
            compared += a.is_some() as usize;
        }
        assert!(compared > 500, "only {compared} codes compared");
    }

    /// The blocks found on the index pass are the blocks of the whole trace,
    /// one for one: where each starts and ends in the run and in the output,
    /// how it is coded and whether it is the last. And each block's own trace
    /// holds the same steps as the whole trace does over that block.
    #[test]
    fn the_blocks_of_an_index_are_the_blocks_of_the_whole_trace() {
        let data = mixed(300_000, 29);
        // Level 0 writes stored blocks, so checkpoints land at block starts;
        // the others put them inside Huffman-coded blocks. Flushing makes
        // hundreds of blocks of every kind.
        let mut most = 0;
        for level in [0u8, 1, 6] {
            for (codec, packed) in [
                (Codec::Deflate, miniz_oxide::deflate::compress_to_vec(&data, level)),
                (Codec::Zlib, miniz_oxide::deflate::compress_to_vec_zlib(&data, level)),
                (Codec::Deflate, flushed(&data, level, 3000)),
                (Codec::Gzip, gzip_around(&data, &flushed(&data, level, 5000))),
            ] {
                let (_, whole) = crate::codec::decode_traced(codec, &packed).expect("decodes whole");
                let s = indexed(codec, &packed, small());
                assert_eq!(s.blocks(), whole.blocks().len() as u64, "{codec:?} level {level}");
                most = most.max(s.blocks());
                for (i, want) in whole.blocks().iter().enumerate() {
                    let got = s.block(i as u64).expect("a block");
                    assert_eq!(
                        (got.kind, got.last, &got.in_bits, &got.out_bytes),
                        (want.kind, want.last, &want.in_bits, &want.out_bytes),
                        "{codec:?} level {level}: block {i}"
                    );
                    for bit in [want.in_bits.start, want.in_bits.end - 1] {
                        assert_eq!(s.block_of_bit(bit), Some(i as u64), "{codec:?} level {level}: bit {bit}");
                    }
                    let t = s.block_trace(i as u64).expect("traces");
                    t.check_tiles().unwrap_or_else(|e| panic!("{codec:?} level {level} block {i}: {e}"));
                    let ours: Vec<Step> = t.steps().take(t.blocks()[0].steps.len()).collect();
                    let theirs: Vec<Step> = want.steps.clone().map(|k| whole.step(k as usize).unwrap()).collect();
                    assert!(ours == theirs, "{codec:?} level {level}: block {i}'s steps differ");
                }
                assert_eq!(s.block(s.blocks()).err(), Some(Refusal::Failed));
                // Bits outside every block: the wrapper's, and the padding.
                let first = whole.blocks()[0].in_bits.start;
                if first > 0 {
                    assert_eq!(s.block_of_bit(first - 1), None);
                }
                assert_eq!(s.block_of_bit(whole.blocks().last().unwrap().in_bits.end), None);
            }
        }
        assert!(most > 100, "at most {most} blocks");
    }

    /// A block of more codes than a block's trace may hold has them as one
    /// opaque step, which still tiles the block, and which the inspector
    /// declines to take apart rather than misreading.
    #[test]
    fn a_block_of_too_many_codes_has_them_as_one_step() {
        let data = mixed(300_000, 37);
        let packed = flushed(&data, 6, 6000);
        let s = indexed(Codec::Deflate, &packed, Limits { span_steps: 200, ..small() });
        let mut coarse = 0;
        for i in 0..s.blocks() {
            let t = s.block_trace(i).expect("traces");
            t.check_tiles().unwrap_or_else(|e| panic!("block {i}: {e}"));
            let block = &t.blocks()[0];
            let payload: Vec<Step> = block.steps.clone().map(|k| t.step(k as usize).unwrap()).filter(|s| s.kind == StepKind::Opaque).collect();
            if let [one] = &payload[..] {
                coarse += 1;
                assert!(t.coarse());
                assert_eq!(one.in_bits.end, block.in_bits.end, "block {i}: the opaque step stops short of the block's end");
                let bit = (one.in_bits.start + one.in_bits.end) / 2;
                let k = t.index_in(bit).expect("a step reads it");
                assert_eq!(inflate::decode_step(&packed, &t, k), None, "block {i}: an opaque step taken apart");
            }
        }
        assert!(coarse > 10, "only {coarse} blocks over the limit");
    }

    /// Past the most block starts there may be, every second one goes and
    /// the ones between are found again by decoding: every block still reads
    /// as the whole trace has it, and no more starts are kept than the limit.
    #[test]
    fn block_starts_thin_out_and_the_rest_are_found_again() {
        let data = mixed(300_000, 31);
        let packed = flushed(&data, 1, 2000);
        let (_, whole) = crate::codec::decode_traced(Codec::Deflate, &packed).expect("decodes whole");
        let n = whole.blocks().len();
        let limits = Limits { block_marks: 4, ..small() };
        let s = indexed(Codec::Deflate, &packed, limits);
        assert!(n > 16, "only {n} blocks");
        assert!(s.blocks.list.len() <= 4 && s.blocks.stride > 1, "{} starts kept {} apart", s.blocks.list.len(), s.blocks.stride);
        assert_eq!(s.blocks(), n as u64);
        // Backwards, so that the groups turn over.
        for (i, want) in whole.blocks().iter().enumerate().rev() {
            let got = s.block(i as u64).expect("a block");
            assert_eq!((&got.in_bits, &got.out_bytes, got.kind), (&want.in_bits, &want.out_bytes, want.kind), "block {i}");
            let mid = (want.in_bits.start + want.in_bits.end) / 2;
            assert_eq!(s.block_of_bit(mid), Some(i as u64), "bit {mid}");
        }
    }

    /// Run `pass` to the end in steps of `out_bytes`, checking on the way that
    /// it only ever moves forward.
    fn in_steps(mut pass: Indexing, out_bytes: u64) -> (Result<Decoded, Refusal>, usize) {
        let mut steps = 0;
        let mut was = pass.progress();
        loop {
            steps += 1;
            let stepped = pass.step(out_bytes);
            let now = pass.progress();
            match stepped {
                Ok(Stepped::Working) => {
                    assert!(now.read_bits >= was.read_bits && now.read_bits <= now.run_bits, "{was:?} then {now:?}");
                    was = now;
                }
                Ok(Stepped::Done(d)) => return (Ok(d), steps),
                Err(why) => return (Err(why), steps),
            }
        }
    }

    /// The pass done in steps of every size, from smaller than one code's
    /// output to larger than the run, comes to what the pass in one go comes
    /// to: the same checkpoints and sums for a run past the cap, and for a run
    /// under it the same bytes and the same trace, step for step. Steps stop
    /// inside blocks too, so the runs are a block every 3,000 bytes and one
    /// block for the whole run, which a step of 100 bytes stops in hundreds
    /// of times.
    #[test]
    fn a_pass_in_steps_comes_to_what_one_pass_does() {
        let large = mixed(300_000, 29);
        let small_run = mixed(50_000, 31);
        for (data, raw) in [&large, &small_run]
            .into_iter()
            .flat_map(|data| [(data, deflate_in_blocks(data, 3000)), (data, miniz_oxide::deflate::compress_to_vec(data, 6))])
        {
            for codec in [Codec::Deflate, Codec::Gzip, Codec::Zlib] {
                let packed = Arc::new(wrapped(codec, &raw, data));
                let one = decode_or_index_with(codec, packed.clone(), small()).expect("decodes");
                for out_bytes in [100u64, 4096, 70_000, u64::MAX] {
                    let (got, steps) = in_steps(Indexing::new(codec, packed.clone(), small()).expect("starts"), out_bytes);
                    let at = format!("{codec:?}, {} bytes, steps of {out_bytes}", data.len());
                    if out_bytes == 4096 {
                        assert!(steps > 5, "{at}: {steps} steps");
                    }
                    match (&one, got.expect("decodes in steps")) {
                        (Decoded::Whole(bytes, trace), Decoded::Whole(ours, t)) => {
                            assert!(ours == *bytes, "{at}: the bytes differ");
                            assert!(t.steps().eq(trace.steps()), "{at}: the steps differ");
                            assert_eq!(t.blocks(), trace.blocks(), "{at}");
                            assert_eq!((t.in_bits(), t.out_bytes()), (trace.in_bits(), trace.out_bytes()), "{at}");
                        }
                        (Decoded::Indexed(s), Decoded::Indexed(ours)) => {
                            assert_eq!(ours.marks(), s.marks(), "{at}");
                            assert_eq!((&ours.blocks, ours.blocks_end), (&s.blocks, s.blocks_end), "{at}");
                            assert_eq!((ours.len(), ours.crc32(), ours.adler32()), (s.len(), s.crc32(), s.adler32()), "{at}");
                            let mut all = vec![0u8; data.len()];
                            ours.read(0, &mut all).expect("reads");
                            assert!(all == *data, "{at}: the output differs");
                        }
                        _ => panic!("{at}: one pass and the steps disagree about holding it whole"),
                    }
                }
            }
        }
    }

    /// A step that stops at the start of a block, or inside one, is carried
    /// on by the next step, which is told of that place again or reads the
    /// block's header again. Neither may record the block's start a second
    /// time. Steps of 100 bytes stop at dozens of block starts and inside
    /// dozens of blocks, before the cap and after it, and the block starts
    /// they record are the ones the pass in one go records: the same list,
    /// the same count, the same thinning, and each checkpoint counting the
    /// same blocks before it, which is what the pass counts back to at the
    /// cap. Both are the blocks of the whole trace.
    #[test]
    fn a_pass_in_steps_records_each_block_once() {
        let data = mixed(300_000, 53);
        // Nothing stops inside a stored block, so at level 0 every stop is at
        // the start of a block, and at level 6 most are inside one. Not gzip,
        // whose trailer says the run is large, so its pass starts past the
        // cap whether told or not.
        for (level, raw) in [(0u8, flushed(&data, 0, 3000)), (6, flushed(&data, 6, 3000)), (6, miniz_oxide::deflate::compress_to_vec(&data, 6))] {
            for codec in [Codec::Deflate, Codec::Zlib] {
                let packed = Arc::new(wrapped(codec, &raw, &data));
                let (_, whole) = crate::codec::decode_traced(codec, &packed).expect("decodes whole");
                for limits in [small(), Limits { block_marks: 4, ..small() }] {
                    for told in [false, true] {
                        let at = format!("{codec:?} level {level}, {} blocks, at most {} kept, told {told}", whole.blocks().len(), limits.block_marks);
                        let start = || {
                            let pass = Indexing::new(codec, packed.clone(), limits).expect("starts");
                            if told { pass.known_large() } else { pass }
                        };
                        let Ok(Decoded::Indexed(one)) = in_steps(start(), u64::MAX).0 else { panic!("{at}: not indexed in one go") };
                        // How many steps stopped at a block's start and how
                        // many inside a block, before the cap and after it.
                        let mut stops = [[0usize; 2]; 2];
                        let mut pass = start();
                        let ours = loop {
                            match pass.step(100) {
                                Ok(Stepped::Working) => {}
                                Ok(Stepped::Done(Decoded::Indexed(s))) => break s,
                                Ok(Stepped::Done(Decoded::Whole(..))) => panic!("{at}: came back whole"),
                                Err(why) => panic!("{at}: refused: {why:?}"),
                            }
                            match &pass.phase {
                                Phase::Whole { from: Some(r), .. } => stops[0][r.block.is_some() as usize] += 1,
                                Phase::Index { from, .. } => stops[1][from.block.is_some() as usize] += 1,
                                _ => {}
                            }
                        };
                        let inside = (level != 0) as usize;
                        if !told {
                            assert!(stops[0][inside] > 10, "{at}: stops before the cap {:?}", stops[0]);
                        }
                        assert!(stops[1][inside] > 10, "{at}: stops after the cap {:?}", stops[1]);
                        assert_eq!((&ours.blocks, ours.blocks_end), (&one.blocks, one.blocks_end), "{at}");
                        if limits.block_marks == 4 && whole.blocks().len() > 8 {
                            assert!(ours.blocks.stride > 1, "{at}: not thinned");
                        }
                        let counts = |s: &Seekable| s.checkpoints.iter().map(|c| (c.in_bit, c.blocks())).collect::<Vec<_>>();
                        assert_eq!(counts(&ours), counts(&one), "{at}");
                        for c in &ours.checkpoints {
                            let before = whole.blocks().partition_point(|b| b.in_bits.start < c.in_bit) as u64;
                            assert_eq!(c.blocks(), before, "{at}: the checkpoint at bit {}", c.in_bit);
                        }
                        assert_eq!(ours.blocks(), whole.blocks().len() as u64, "{at}");
                        for (i, want) in whole.blocks().iter().enumerate() {
                            let got = ours.block(i as u64).expect("a block");
                            assert_eq!(
                                (got.kind, got.last, &got.in_bits, &got.out_bytes),
                                (want.kind, want.last, &want.in_bits, &want.out_bytes),
                                "{at}: block {i}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// A trace that grows too long partway through a block names that
    /// block's symbols as one step, the ones from steps before as well, and
    /// carries on naming blocks in the steps after, as it would in one pass.
    #[test]
    fn a_trace_that_names_blocks_still_does_after_a_step() {
        let data = mixed(50_000, 37);
        for packed in [deflate_in_blocks(&data, 3000), miniz_oxide::deflate::compress_to_vec(&data, 6)] {
            let packed = Arc::new(packed);
            let limits = Limits { whole_steps: 2000, ..small() };
            let Ok(Decoded::Whole(_, trace)) = decode_or_index_with(Codec::Deflate, packed.clone(), limits) else { panic!("not whole") };
            assert!(trace.coarse());
            let (got, steps) = in_steps(Indexing::new(Codec::Deflate, packed, limits).expect("starts"), 100);
            assert!(steps > 100, "{steps} steps");
            let Ok(Decoded::Whole(_, t)) = got else { panic!("not whole in steps") };
            assert!(t.coarse());
            assert!(t.steps().eq(trace.steps()), "the steps differ");
            assert_eq!(t.blocks(), trace.blocks());
        }
    }

    /// A pass told the run is large starts with the trace off and comes to the
    /// same index as one that found out at the cap; one told so of a run
    /// under the cap indexes it all the same. A gzip run's trailer tells it.
    #[test]
    fn a_pass_told_the_run_is_large_comes_to_the_same_index() {
        let data = mixed(300_000, 43);
        let raw = Arc::new(deflate_in_blocks(&data, 3000));
        let found = in_steps(Indexing::new(Codec::Deflate, raw.clone(), small()).expect("starts"), 5000).0;
        let told = in_steps(Indexing::new(Codec::Deflate, raw, small()).expect("starts").known_large(), 5000).0;
        let (Ok(Decoded::Indexed(found)), Ok(Decoded::Indexed(told))) = (found, told) else { panic!("not indexed") };
        assert_eq!(told.marks(), found.marks());
        assert_eq!((&told.blocks, told.blocks_end), (&found.blocks, found.blocks_end));
        assert_eq!((told.len(), told.crc32(), told.adler32()), (found.len(), found.crc32(), found.adler32()));

        let short = mixed(50_000, 47);
        let raw = Arc::new(miniz_oxide::deflate::compress_to_vec(&short, 6));
        let told = in_steps(Indexing::new(Codec::Deflate, raw, small()).expect("starts").known_large(), 5000).0;
        let Ok(Decoded::Indexed(told)) = told else { panic!("not indexed") };
        assert_eq!(told.len(), short.len() as u64);

        // A gzip trailer saying 50 KB comes back whole; one saying 1 MB about
        // the same 50 KB is wrong, and refused as a wrong trailer is.
        let gzip = wrapped(Codec::Gzip, &deflate_in_blocks(&short, 3000), &short);
        let Ok(Decoded::Whole(..)) = decode_or_index_with(Codec::Gzip, Arc::new(gzip.clone()), small()) else { panic!("not whole") };
        let mut lying = gzip;
        let n = lying.len();
        lying[n - 4..].copy_from_slice(&(1u32 << 20).to_le_bytes());
        assert_eq!(decode_or_index_with(Codec::Gzip, Arc::new(lying), small()).err(), Some(Refusal::Failed));
    }

    /// Refusals come out of the pass in steps as they do in one go.
    #[test]
    fn a_pass_in_steps_refuses_what_one_pass_does() {
        let zeros = vec![0u8; 5 << 20];
        let packed = Arc::new(miniz_oxide::deflate::compress_to_vec(&zeros, 9));
        let limits = Limits { lazy: 4 << 20, ..small() };
        let (got, _) = in_steps(Indexing::new(Codec::Deflate, packed, limits).expect("starts"), 64 << 10);
        assert_eq!(got.err(), Some(Refusal::TooLargeLazy));
        let mut gzip = gzip_of(&mixed(200_000, 41), 6);
        let n = gzip.len();
        gzip[n - 8] ^= 1;
        let (got, _) = in_steps(Indexing::new(Codec::Gzip, Arc::new(gzip), small()).expect("starts"), 5000);
        assert_eq!(got.err(), Some(Refusal::Failed));
    }

    /// Past the limit for a run read a piece at a time, a run is refused with
    /// the refusal that says so, after a pass that kept no more than the
    /// checkpoints.
    #[test]
    fn a_run_past_the_limit_is_refused() {
        let zeros = vec![0u8; 5 << 20];
        let packed = miniz_oxide::deflate::compress_to_vec(&zeros, 9);
        let limits = Limits { lazy: 4 << 20, ..small() };
        assert_eq!(decode_or_index_with(Codec::Deflate, Arc::new(packed), limits).err(), Some(Refusal::TooLargeLazy));
    }

    /// A trailer that disagrees with what came out is refused, as it is for a
    /// run decoded whole, and so is a run cut short.
    #[test]
    fn a_wrong_trailer_or_a_short_run_is_refused() {
        let data = mixed(200_000, 19);
        let mut gzip = gzip_of(&data, 6);
        let n = gzip.len();
        gzip[n - 8] ^= 1;
        assert_eq!(decode_or_index_with(Codec::Gzip, Arc::new(gzip), small()).err(), Some(Refusal::Failed));
        let mut zlib = miniz_oxide::deflate::compress_to_vec_zlib(&data, 6);
        let n = zlib.len();
        zlib[n - 1] ^= 1;
        assert_eq!(decode_or_index_with(Codec::Zlib, Arc::new(zlib), small()).err(), Some(Refusal::Failed));
        let raw = miniz_oxide::deflate::compress_to_vec(&data, 6);
        let cut = raw[..raw.len() * 2 / 3].to_vec();
        assert_eq!(decode_or_index_with(Codec::Deflate, Arc::new(cut), small()).err(), Some(Refusal::Failed));
    }

    /// zstd frames of `chunk` bytes each, packed by `ruzstd`, one after
    /// another. With `skippable`, each has a skippable frame in front of it
    /// holding its length, which is what `pzstd` writes.
    pub(crate) fn zstd_of(data: &[u8], chunk: usize, skippable: bool) -> Vec<u8> {
        use ruzstd::encoding::{compress_to_vec, CompressionLevel};
        let mut out = Vec::new();
        for (i, part) in data.chunks(chunk).enumerate() {
            let level = if i % 3 == 2 { CompressionLevel::Uncompressed } else { CompressionLevel::Fastest };
            let frame = compress_to_vec(part, level);
            if skippable {
                out.extend_from_slice(&0x184d_2a50u32.to_le_bytes());
                out.extend_from_slice(&4u32.to_le_bytes());
                out.extend_from_slice(&(frame.len() as u32).to_le_bytes());
            }
            out.extend_from_slice(&frame);
        }
        out
    }

    /// An xz stream of blocks of `chunk` bytes each, packed by `lzma-rust2`.
    pub(crate) fn xz_of(data: &[u8], chunk: usize) -> Vec<u8> {
        use crate::codec::xz::tests::{pack, wrap};
        let blocks: Vec<(Vec<u8>, Vec<u8>)> = data.chunks(chunk).map(|c| (pack(c, 3, 0, 2, 1 << 16, None), c.to_vec())).collect();
        wrap(&blocks, true)
    }

    /// Reads of every length from everywhere, across span joins, give what
    /// went in, and so does one read of everything.
    fn reads_back(s: &Seekable, data: &[u8], seed: u64) {
        assert_eq!(s.len(), data.len() as u64);
        let mut all = vec![0u8; data.len()];
        s.read(0, &mut all).expect("reads");
        assert!(all == data, "{:?}: the whole output differs", s.codec());
        let mut rand = rand_from(seed);
        for _ in 0..120 {
            let at = rand() % data.len() as u64;
            let n = (rand() % 50_000).min(data.len() as u64 - at) as usize;
            let mut got = vec![0u8; n];
            s.read(at, &mut got).expect("reads");
            assert!(got == data[at as usize..at as usize + n], "{:?}: {n} bytes at {at} differ", s.codec());
        }
    }

    /// Every span's trace tiles its own bits and bytes, and every byte and bit
    /// maps to the same step through the spans as through the trace of the
    /// whole run.
    fn traces_agree(s: &Seekable, whole: &Trace, seed: u64) {
        let packed_bits = s.packed().len() as u64 * 8;
        for k in 0..s.spans() {
            let t = s.span_trace(k).expect("traces");
            t.check_tiles().unwrap_or_else(|e| panic!("{:?} span {k}: {e}", s.codec()));
            assert_eq!(t.start().1, s.span_bytes(k).start);
            assert_eq!(t.out_bytes(), s.span_bytes(k).end);
        }
        assert_eq!(s.span_trace(s.spans() - 1).unwrap().in_bits(), packed_bits);
        let mut rand = rand_from(seed);
        let mut bytes: Vec<u64> = (0..2000).map(|_| rand() % s.len()).collect();
        let mut bits: Vec<u64> = (0..2000).map(|_| rand() % packed_bits).collect();
        bytes.sort_unstable();
        bits.sort_unstable();
        for byte in bytes {
            assert_eq!(s.map_out(byte), whole.map_out(byte), "{:?}: byte {byte}", s.codec());
        }
        for bit in bits {
            assert_eq!(s.map_in(bit), whole.map_in(bit), "{:?}: bit {bit}", s.codec());
        }
        for (in_bit, _, _) in s.marks() {
            for bit in [in_bit.saturating_sub(1), in_bit, in_bit + 1] {
                assert_eq!(s.map_in(bit), whole.map_in(bit), "{:?}: bit {bit} by checkpoint {in_bit}", s.codec());
            }
        }
    }

    /// A zstd run of several frames, with and without `pzstd`'s skippable
    /// frames, past the cap: read back from anywhere, a checkpoint at the
    /// start of a frame, and a trace per span that says what the whole
    /// trace says.
    #[test]
    fn a_zstd_run_of_several_frames_reads_back_from_anywhere() {
        let data = mixed(400_000, 29);
        for skippable in [false, true] {
            let packed = zstd_of(&data, 30_000, skippable);
            let s = indexed(Codec::Zstd, &packed, small());
            reads_back(&s, &data, 31);
            let marks = s.marks();
            assert_eq!(marks[0], (0, 0, None));
            assert!(marks.len() > 4 && marks.len() <= small().checkpoints, "{} checkpoints", marks.len());
            assert!(s.crc32().is_none() && s.adler32().is_none());
            let (bytes, whole) = crate::codec::decode_traced(Codec::Zstd, &packed).expect("decodes whole");
            assert!(bytes == data);
            traces_agree(&s, &whole, 37);
        }
    }

    /// Under the cap a zstd run comes back whole, exactly as the whole
    /// decoder gives it, and that is every frame and not only the first.
    #[test]
    fn a_zstd_run_under_the_cap_comes_back_whole_with_every_frame() {
        let data = mixed(50_000, 41);
        let packed = zstd_of(&data, 20_000, true);
        let (bytes, trace) = crate::codec::decode_traced(Codec::Zstd, &packed).expect("decodes");
        assert!(bytes == data, "{} bytes of {}", bytes.len(), data.len());
        trace.check_tiles().expect("tiles");
        // Three frames, each behind a skippable one.
        let headers = trace.steps().filter(|s| s.kind == StepKind::Header(StepField::FrameHeader, 0)).count();
        assert_eq!(headers, 6);
        let Ok(Decoded::Whole(ours, t)) = decode_or_index_with(Codec::Zstd, Arc::new(packed.clone()), small()) else {
            panic!("did not come back whole");
        };
        assert!(ours == bytes);
        assert!(t.steps().eq(trace.steps()));
    }

    /// A frame past the most one span may come to is refused as too large,
    /// which is what a file of one frame is. Past the limit for a run read a
    /// piece at a time, a run of small frames is refused with the refusal that
    /// says so.
    #[test]
    fn a_zstd_frame_past_the_span_limit_is_refused() {
        let data = mixed(300_000, 43);
        let one = zstd_of(&data, data.len(), false);
        assert_eq!(decode_or_index_with(Codec::Zstd, Arc::new(one), small()).err(), Some(Refusal::TooLarge));
        let mut sizes = zstd_of(&data, 30_000, false);
        sizes.extend(zstd_of(&data[..100_000], 100_000, false));
        assert_eq!(decode_or_index_with(Codec::Zstd, Arc::new(sizes), small()).err(), Some(Refusal::TooLarge));
        let many = zstd_of(&data, 30_000, true);
        let limits = Limits { lazy: 200_000, ..small() };
        assert_eq!(decode_or_index_with(Codec::Zstd, Arc::new(many), limits).err(), Some(Refusal::TooLargeLazy));
    }

    /// An xz stream of several blocks past the cap, laid out from its index:
    /// read back from anywhere, a checkpoint at the start of a block, a trace
    /// per span that says what the whole trace says, and members that are the
    /// whole trace's.
    #[test]
    fn an_xz_stream_of_several_blocks_reads_back_from_anywhere() {
        let data = mixed(400_000, 47);
        let packed = xz_of(&data, 30_000);
        let s = indexed(Codec::Xz, &packed, small());
        reads_back(&s, &data, 53);
        let marks = s.marks();
        assert_eq!(marks[0], (0, 0, None));
        assert!(marks.len() > 4 && marks.len() <= small().checkpoints, "{} checkpoints", marks.len());
        let (bytes, whole) = crate::codec::decode_traced(Codec::Xz, &packed).expect("decodes whole");
        assert!(bytes == data);
        assert!(!whole.coarse());
        traces_agree(&s, &whole, 59);
        assert_eq!(s.members(), whole.members());
    }

    /// One block past the most one span may come to is refused as too large,
    /// which is what a file of one block is; past the limit for a run read a
    /// piece at a time, so is a stream of small blocks, with the refusal that
    /// says so. Under the cap the stream comes back whole as it always did.
    #[test]
    fn an_xz_block_past_the_span_limit_is_refused() {
        let data = mixed(300_000, 61);
        let one = xz_of(&data, data.len());
        assert_eq!(decode_or_index_with(Codec::Xz, Arc::new(one), small()).err(), Some(Refusal::TooLarge));
        let limits = Limits { lazy: 200_000, ..small() };
        assert_eq!(decode_or_index_with(Codec::Xz, Arc::new(xz_of(&data, 30_000)), limits).err(), Some(Refusal::TooLargeLazy));
        let under = xz_of(&data[..50_000], 20_000);
        let (bytes, trace) = crate::codec::decode_traced(Codec::Xz, &under).expect("decodes");
        let Ok(Decoded::Whole(ours, t)) = decode_or_index_with(Codec::Xz, Arc::new(under), small()) else {
            panic!("did not come back whole");
        };
        assert!(ours == bytes);
        assert!(t.steps().eq(trace.steps()));
    }

    /// Nothing decodes an xz stream to index it, so a block that does not
    /// come to what the index says opens, and its span is refused when read.
    /// The spans either side still read.
    #[test]
    fn an_xz_block_the_index_is_wrong_about_is_refused_when_read() {
        use crate::codec::xz::tests::{pack, wrap};
        let data = mixed(300_000, 67);
        let mut blocks: Vec<(Vec<u8>, Vec<u8>)> = data.chunks(30_000).map(|c| (pack(c, 3, 0, 2, 1 << 16, None), c.to_vec())).collect();
        // The fifth block's packed data is the sixth's, so it decodes to a
        // stream of its own and not to what its index record says.
        blocks[4].0 = blocks[5].0.clone();
        blocks[4].1.truncate(29_000);
        let s = indexed(Codec::Xz, &wrap(&blocks, false), Limits { spacing: 1, ..small() });
        let k = s.span_of_byte(4 * 30_000).unwrap();
        let at = s.span_bytes(k).start;
        assert_eq!(s.read(at, &mut [0u8; 10]).err(), Some(Refusal::Failed));
        let mut first = vec![0u8; 30_000];
        s.read(0, &mut first).expect("the first block reads");
        assert!(first == data[..30_000]);
    }

    /// Past the first window of a frame each block is a step of its own, and
    /// the step says the bytes that block made: a raw block's step reads
    /// exactly the bytes it writes. Before that the blocks are one step. The
    /// same through the spans of a run indexed with frames this long.
    #[test]
    fn zstd_blocks_past_the_window_are_steps_of_their_own() {
        let data = mixed(1_200_000, 71);
        let packed = zstd_of(&data, 400_000, true);
        let (bytes, whole) = crate::codec::decode_traced(Codec::Zstd, &packed).expect("decodes");
        assert!(bytes == data);
        let mut raw = 0;
        let mut merged = 0;
        for step in whole.steps().filter(|s| s.kind == StepKind::Block) {
            let at = (step.in_bits.start / 8) as usize;
            let h = u32::from_le_bytes([packed[at - 3], packed[at - 2], packed[at - 1], 0]);
            let (kind, size) = ((h >> 1) & 3, (h >> 3) as u64);
            let reads = step.in_bits.end / 8 - step.in_bits.start / 8;
            let writes = step.out_bytes.end - step.out_bytes.start;
            if kind == 0 && reads == size {
                assert_eq!(writes, size, "raw block at byte {at}");
                assert!(packed[at..at + size as usize] == bytes[step.out_bytes.start as usize..step.out_bytes.end as usize]);
                raw += 1;
            }
            // No block may come to more than 128 KiB, so a step that does is
            // more than one block.
            merged += (writes > 128 << 10) as usize;
        }
        assert!(raw >= 2, "{raw} raw blocks checked");
        assert_eq!(merged, 3, "one step over the first window of each frame");
        let s = indexed(Codec::Zstd, &packed, Limits { span: 1 << 20, ..small() });
        reads_back(&s, &data, 73);
        traces_agree(&s, &whole, 79);
    }
}
