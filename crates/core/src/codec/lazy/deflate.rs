//! Indexing a deflate, zlib or gzip run: the pass that can be done a step at a
//! time, the sink it decodes into, and the block starts and sums it records.

use std::ops::ControlFlow;
use std::sync::Arc;

use super::input::RunInput;
use super::seekable::{seekable, Kind};
use super::{wrapper, Checkpoint, Checkpoints, Decoded, Limits, Progress, Start, Stepped, SLIDE_AT, WINDOW};
use crate::checksum::{adler32_update, crc32_update};
use crate::codec::inflate::{self, note_head, BlockStart, Ended, Framing, Input, Resume, Sink, Wrapper, MAX_HEAD_BYTES};
use crate::codec::{Codec, Refusal, StepField, StepKind, TraceBuilder, CAP_BYTES};

/// A deflate, zlib or gzip run's index pass, done a step at a time: each
/// [`DeflatePass::step`] decodes about as much output, or reads about as many
/// bits, as it is told to, stops at a place the decoder can carry on from,
/// and the next step carries on from exactly there. See [`super::Indexing`].
///
/// Holds what the pass would hold if it ran in one go: up to the cap of output
/// and its trace while the run might still come to a whole one, and after
/// that the window, the checkpoints, the block starts and the running sums.
pub(super) struct DeflatePass {
    codec: Codec,
    packed: Arc<Vec<u8>>,
    /// Where the index reads the run from once it is made.
    input: RunInput,
    framing: Framing,
    limits: Limits,
    pub(super) phase: Phase,
    /// The furthest bit of the run a step has stopped at. Kept apart from
    /// where the next step starts, which goes back a little once the output
    /// passes the cap. The same for the output, in `most`.
    furthest: u64,
    most: u64,
}

/// How far a [`DeflatePass`] has got.
pub(super) enum Phase {
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

impl DeflatePass {
    /// Start indexing `packed`, packed with deflate, zlib or gzip; any other
    /// codec is refused as [`Refusal::Failed`]. Nothing is decoded until the
    /// first step.
    ///
    /// A gzip run whose trailer says it comes to more than the cap starts as
    /// [`DeflatePass::known_large`] does. The trailer gives the size modulo
    /// 4 GiB, so a size past the cap there is past it in fact; a trailer that
    /// is wrong about it fails the check at the end either way.
    pub(super) fn new(codec: Codec, packed: Arc<Vec<u8>>, input: RunInput, limits: Limits) -> Result<DeflatePass, Refusal> {
        let wrapper = wrapper(codec).ok_or(Refusal::Failed)?;
        if packed.len() > CAP_BYTES {
            return Err(Refusal::TooLarge);
        }
        let framing = inflate::framing(wrapper, &packed, packed.len())?.found()?;
        let mut trace = TraceBuilder::with_budget(limits.whole_steps).carried_on();
        if wrapper != Wrapper::Raw {
            trace.push(0, 0, StepKind::Header(StepField::Wrapper, 0));
        }
        let mut sink = WindowSink::new(Vec::new(), 0, limits.whole as u64, false);
        sink.marks = Some(Checkpoints::new(&limits));
        sink.blocks = Some(BlockMarks::new(limits.block_marks));
        sink.sums = Some(Sums { crc: 0, adler: 1, upto: 0 });
        sink.mid_block = true;
        let phase = Phase::Whole { sink, trace, from: None };
        let pass = DeflatePass { codec, packed, input, framing, limits, phase, furthest: 0, most: 0 };
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
    pub(super) fn known_large(mut self) -> DeflatePass {
        if matches!(self.phase, Phase::Whole { from: None, .. }) {
            let mut sink = WindowSink::new(Vec::new(), 0, self.limits.lazy, true);
            sink.marks = Some(Checkpoints::new(&self.limits));
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
    pub(super) fn progress(&self) -> Progress {
        let run_bits = self.packed.len() as u64 * 8;
        let unpacked = match &self.phase {
            Phase::Whole { sink, .. } | Phase::Index { sink, .. } => sink.len(),
            Phase::Over => 0,
        };
        Progress { read_bits: self.furthest, run_bits, unpacked: self.most.max(unpacked) }
    }

    /// Decode about `out_bytes` more output, or read about `in_bits` more
    /// bits of the run, whichever comes first, and stop at the next code or
    /// block; or finish. A stored block is not stopped in, so a step can
    /// overrun by up to one stored block, 64 KiB.
    pub(super) fn step(&mut self, out_bytes: u64, in_bits: u64) -> Result<Stepped, Refusal> {
        let stepped = self.step_on(out_bytes, in_bits);
        let (at, out) = match &self.phase {
            Phase::Whole { sink, from, .. } => (from.map_or(0, |r| r.in_bit), sink.len()),
            Phase::Index { sink, from } => (from.in_bit, sink.len()),
            Phase::Over => (self.packed.len() as u64 * 8, 0),
        };
        self.furthest = self.furthest.max(at);
        self.most = self.most.max(out);
        stepped
    }

    fn step_on(&mut self, out_bytes: u64, in_bits: u64) -> Result<Stepped, Refusal> {
        let data = &self.packed[..];
        let f = self.framing;
        // Counted from where the step starts, not from the header of the
        // block it starts in, which it reads again: a step has to get past
        // its first code however long the block's tables are.
        let bits_end = |from: u64| from.saturating_add(in_bits.max(1));
        match std::mem::replace(&mut self.phase, Phase::Over) {
            Phase::Whole { mut sink, mut trace, from } => {
                sink.slice_end = sink.len().saturating_add(out_bytes.max(1));
                sink.slice_bits = bits_end(from.map_or(f.start, |r| r.in_bit));
                match inflate::run(Input::whole(data), f.start, f.end, &mut sink, &mut trace, from) {
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
                        if !inflate::trailer_agrees(&f, &data[f.trailer()], out.len() as u64, crc, adler) {
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
                sink.slice_bits = bits_end(from.in_bit);
                match inflate::run(Input::whole(data), f.start, f.end, &mut sink, &mut TraceBuilder::off(), Some(from)) {
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
        if !inflate::trailer_agrees(&self.framing, &self.packed[self.framing.trailer()], len, || sums.crc, || sums.adler) {
            return Err(Refusal::Failed);
        }
        let mut checkpoints = sink.marks.take().expect("set when the pass started").list;
        self.copy_heads(&mut checkpoints);
        let mut s = seekable(self.codec, self.input.clone(), Kind::Deflate(self.framing), len, checkpoints, self.limits);
        s.blocks = sink.blocks.take().expect("set when the pass started");
        s.blocks_end = blocks_end;
        s.crc32 = Some(sums.crc);
        s.adler32 = Some(sums.adler);
        Ok(Decoded::Indexed(s))
    }
}

impl DeflatePass {
    /// Give each checkpoint inside a block a copy of the block's header and
    /// tables, which a decode starting there reads again, so that it need
    /// not read the run as far back as the block's start. The checkpoints
    /// inside one block share one copy.
    fn copy_heads(&self, checkpoints: &mut [Checkpoint]) {
        let mut last: Option<(u64, Arc<[u8]>)> = None;
        for c in checkpoints {
            let Start::Deflate { block: Some(h), head, .. } = &mut c.start else { continue };
            let copy = match &last {
                Some((at, copy)) if *at == *h => copy.clone(),
                _ => {
                    let from = (*h / 8) as usize;
                    let copy: Arc<[u8]> = self.packed[from..(from + MAX_HEAD_BYTES).min(self.packed.len())].into();
                    last = Some((*h, copy.clone()));
                    copy
                }
            };
            *head = Some(copy);
        }
    }
}

/// Where each block starts, as a bit of the run and a byte of the output, and
/// what its header says, recorded by the index pass. Holds every block's
/// start until there are more than `max`, and from then on the start of every
/// `stride`th block. Block 0's start is always kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BlockMarks {
    /// The start of blocks 0, `stride`, 2 × `stride` and so on.
    pub(super) list: Vec<BlockStart>,
    /// How many blocks have started, kept or not.
    pub(super) count: u64,
    pub(super) stride: u64,
    max: usize,
}

impl BlockMarks {
    pub(super) fn new(max: usize) -> BlockMarks {
        BlockMarks { list: Vec::new(), count: 0, stride: 1, max: max.max(2) }
    }

    /// Record a block that starts at `in_bit`, with `out` bytes of output
    /// before it. When the list grows past `max`, every second start is
    /// dropped and the stride doubles, so the list keeps the starts of the
    /// blocks whose number is a multiple of the stride.
    fn record(&mut self, in_bit: u64, out: u64) {
        if self.count % self.stride == 0 {
            self.list.push(BlockStart::new(in_bit, out));
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
pub(super) struct WindowSink {
    pub(super) hist: Vec<u8>,
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
    pub(super) stop_at: Option<u64>,
    /// Stop at the first boundary once the output has come to this, which is
    /// where a step of a [`DeflatePass`] ends, and say where in `stopped`.
    slice_end: u64,
    /// The same, at the first boundary at or past this bit of the run, so
    /// that a step over blocks that make no output still ends.
    slice_bits: u64,
    /// Whether that may be between two codes of a block, or only at the start
    /// of one.
    mid_block: bool,
    stopped: Option<Resume>,
}

impl WindowSink {
    pub(super) fn new(hist: Vec<u8>, base: u64, cap: u64, slide: bool) -> WindowSink {
        WindowSink { hist, base, cap, slide, marks: None, blocks: None, sums: None, stop_at: None, slice_end: u64::MAX, slice_bits: u64::MAX, mid_block: false, stopped: None }
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
            marks.offer(in_bit, in_bit, out, || Start::Deflate { block, head: None, blocks, window: window.into() });
        }
        // After the checkpoint, so that a step ending here takes the one a
        // pass in one go would have taken, and the step after does not take
        // it again.
        if (out >= self.slice_end || in_bit >= self.slice_bits) && (block.is_none() || self.mid_block) {
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

    /// Kept with the block's start, when it is kept. A step carrying on
    /// inside a block reads its header again, and records the same.
    #[inline]
    fn head(&mut self, in_bit: u64, last: bool, btype: u32) {
        if let Some(blocks) = &mut self.blocks {
            note_head(&mut blocks.list, in_bit, last, btype);
        }
    }
}
