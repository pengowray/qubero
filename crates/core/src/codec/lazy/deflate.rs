//! Indexing a deflate, zlib or gzip run: the pass that can be done a step at a
//! time, the sink it decodes into, and the block starts and sums it records.

use std::ops::ControlFlow;
use std::sync::Arc;

use super::input::{PassWindow, RunInput};
use super::seekable::{seekable, Kind};
use super::{wrapper, Checkpoints, Decoded, Limits, Progress, Start, Stepped, SLIDE_AT, WINDOW};
use crate::checksum::{adler32_update, crc32_update};
use crate::codec::inflate::{self, note_head, BlockStart, Ended, Framed, Framing, Input, Resume, Sink, Wrapper, DEFLATE_MARGIN, MAX_HEAD_BYTES};
use crate::codec::{Codec, Refusal, StepField, StepKind, TraceBuilder, Window};
use crate::source::Missing;

/// A deflate, zlib or gzip run's index pass, done a step at a time: each
/// [`DeflatePass::step`] decodes about as much output, or reads about as many
/// bits, as it is told to, stops at a place the decoder can carry on from,
/// and the next step carries on from exactly there. See [`super::Indexing`].
///
/// The pass reads the run a window at a time. A step reads from the byte
/// its first bit is in to [`Limits::step_input`] and [`DEFLATE_MARGIN`] past
/// it, stops by the margin before the window ends, and keeps the bytes from
/// where it stopped on for the next step. A step that starts inside a
/// Huffman-coded block reads the block's header and tables again from a copy
/// the pass made when it stopped in the block, since the block can start
/// anywhere before. The first step reads the run's last bytes, which hold
/// the trailer and a gzip's ISIZE, and its head, until the wrapper's header
/// ends.
///
/// Besides that window, it holds what the pass would hold if it ran in one
/// go: up to the cap of output and its trace while the run might still come
/// to a whole one, and after that the window of output, the checkpoints and
/// their copies of block headers, the block starts and the running sums.
pub(super) struct DeflatePass {
    codec: Codec,
    wrapper: Wrapper,
    /// Where the pass reads the run from: `input`, or a copy of the run.
    reads: RunInput,
    /// Where the index reads the run from once it is made.
    input: RunInput,
    /// Where the deflate data is inside the wrapper. Read by the first step.
    framing: Option<Framing>,
    /// The run's last bytes, up to eight: the trailer, and a gzip's ISIZE.
    /// Read by the first step. Nothing for a raw run.
    tail: Option<Vec<u8>>,
    /// The bytes of the run the pass holds between steps.
    held: PassWindow,
    /// A copy of the header and tables of the Huffman-coded block the last
    /// step stopped inside, and the bit the header starts at: what the next
    /// step reads them from.
    head: Option<(u64, Arc<[u8]>)>,
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
    /// Nothing read yet. The first step reads the wrapper's header and the
    /// trailer, and starts as [`Phase::Whole`], or as [`Phase::Index`] when
    /// `large`: a caller that knows the run comes to more than the cap, or a
    /// gzip trailer that says so.
    Start { large: bool },
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
    /// Start indexing a run packed with deflate, zlib or gzip, which the pass
    /// reads through `reads` and the index through `input`; any other codec
    /// is refused as [`Refusal::Failed`]. Nothing is read until the first
    /// step.
    ///
    /// A gzip run whose trailer says it comes to more than the cap starts as
    /// [`DeflatePass::known_large`] does. The trailer gives the size modulo
    /// 4 GiB, so a size past the cap there is past it in fact; a trailer that
    /// is wrong about it fails the check at the end either way.
    pub(super) fn new(codec: Codec, reads: RunInput, input: RunInput, limits: Limits) -> Result<DeflatePass, Refusal> {
        let wrapper = wrapper(codec).ok_or(Refusal::Failed)?;
        let phase = Phase::Start { large: false };
        Ok(DeflatePass { codec, wrapper, reads, input, framing: None, tail: None, held: PassWindow::default(), head: None, limits, phase, furthest: 0, most: 0 })
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
        if matches!(self.phase, Phase::Start { .. }) {
            self.phase = Phase::Start { large: true };
        } else if let (Phase::Whole { from: None, .. }, Some(f)) = (&self.phase, self.framing) {
            self.phase = self.started(f, true);
        }
        self
    }

    /// The phase a pass starts in, once the first step has read where the
    /// deflate data is.
    fn started(&self, f: Framing, large: bool) -> Phase {
        let fresh = |cap: u64, slide: bool| {
            let mut sink = WindowSink::new(Vec::new(), 0, cap, slide);
            sink.marks = Some(Checkpoints::new(&self.limits));
            sink.blocks = Some(BlockMarks::new(self.limits.block_marks));
            sink.sums = Some(Sums { crc: 0, adler: 1, upto: 0 });
            sink.mid_block = true;
            sink
        };
        if large {
            return Phase::Index { sink: fresh(self.limits.lazy, true), from: Resume { in_bit: f.start, block: None } };
        }
        let mut trace = TraceBuilder::with_budget(self.limits.whole_steps).carried_on();
        if self.wrapper != Wrapper::Raw {
            trace.push(0, 0, StepKind::Header(StepField::Wrapper, 0));
        }
        Phase::Whole { sink: fresh(self.limits.whole as u64, false), trace, from: None }
    }

    /// How far the pass has got: the bits of the run read and the bytes of
    /// output written, at the furthest it has been, so that neither goes
    /// back when the pass does.
    pub(super) fn progress(&self) -> Progress {
        let run_bits = self.input.len() * 8;
        let unpacked = match &self.phase {
            Phase::Whole { sink, .. } | Phase::Index { sink, .. } => sink.len(),
            Phase::Start { .. } | Phase::Over => 0,
        };
        Progress { read_bits: self.furthest, run_bits, unpacked: self.most.max(unpacked) }
    }

    /// How many bytes of the run the pass holds between steps: its window,
    /// the trailer and the copy of the header of the block it stopped in.
    #[cfg(test)]
    pub(super) fn holds(&self) -> usize {
        self.held.len() + self.tail.as_ref().map_or(0, Vec::len) + self.head.as_ref().map_or(0, |h| h.1.len())
    }

    /// Decode about `out_bytes` more output, or read about `in_bits` more
    /// bits of the run, whichever comes first, and stop at the next code or
    /// block; or finish. A stored block is not stopped in, so a step can
    /// overrun by up to one stored block, 64 KiB. A step also stops
    /// [`Limits::step_input`] past where it starts, and one whose window is
    /// missing chunks of the file does nothing and names them.
    pub(super) fn step(&mut self, out_bytes: u64, in_bits: u64) -> Result<Stepped, Refusal> {
        let stepped = self.step_on(out_bytes, in_bits);
        let (at, out) = match &self.phase {
            Phase::Start { .. } => (0, 0),
            Phase::Whole { sink, from, .. } => (from.map_or(0, |r| r.in_bit), sink.len()),
            Phase::Index { sink, from } => (from.in_bit, sink.len()),
            Phase::Over => (self.input.len() * 8, 0),
        };
        self.furthest = self.furthest.max(at);
        self.most = self.most.max(out);
        stepped
    }

    fn step_on(&mut self, out_bytes: u64, in_bits: u64) -> Result<Stepped, Refusal> {
        let f = match self.framing {
            Some(f) => f,
            None => {
                if let Some(missing) = self.start()? {
                    return Ok(Stepped::NeedsInput(missing));
                }
                self.framing.expect("found by the first step")
            }
        };
        let from = match &self.phase {
            Phase::Whole { from, .. } => *from,
            Phase::Index { from, .. } => Some(*from),
            Phase::Start { .. } | Phase::Over => return Err(Refusal::Failed),
        };
        // The step's window, from the byte its first bit is in. The step
        // stops at the first place it can at or past its share of input,
        // the margin before the window ends, so that no stretch between two
        // such places runs out of the window. A window that reaches the end
        // of the deflate data reaches the end of the run, so that a copy of
        // a block header near the end is the copy a pass over the whole run
        // makes.
        let run_len = self.reads.len() as usize;
        let at = (from.map_or(f.start, |r| r.in_bit) / 8) as usize;
        let share = at.saturating_add(self.limits.step_input.max(1));
        let mut to = share.saturating_add(DEFLATE_MARGIN);
        if (to as u64).saturating_mul(8) >= f.end {
            to = run_len;
        }
        let stop = (share as u64).saturating_mul(8);
        if let Err(missing) = self.held.reach(&self.reads, at, to) {
            return Ok(Stepped::NeedsInput(missing));
        }
        let held = std::mem::take(&mut self.held);
        let stepped = self.decode(&held, f, from, stop, out_bytes, in_bits);
        self.held = held;
        stepped
    }

    /// Read what the first step reads before it decodes: the run's last
    /// bytes, and its head, read further until the wrapper's header ends
    /// inside it. Nothing when both are read and the pass has started; the
    /// chunks of the file they are missing, all at once, otherwise.
    ///
    /// A header longer than half of what the source keeps, which only a
    /// gzip name or comment can make, is refused as [`Refusal::TooMuchAtOnce`],
    /// since it is read in one call.
    fn start(&mut self) -> Result<Option<Vec<Missing>>, Refusal> {
        let run_len = self.reads.len() as usize;
        let mut missing = Vec::new();
        if self.tail.is_none() && self.wrapper != Wrapper::Raw {
            match self.reads.copy(run_len.saturating_sub(8)..run_len) {
                Ok(copy) => self.tail = Some(copy.bytes),
                Err(m) => missing = m,
            }
        }
        // As much as the first step decodes from, so that the head is read
        // with the first window, in one round.
        let mut want = match self.wrapper {
            Wrapper::Raw => 0,
            _ => self.limits.step_input.max(1).saturating_add(DEFLATE_MARGIN).min(run_len),
        };
        let found = loop {
            if let Err(m) = self.held.reach(&self.reads, 0, want) {
                missing.extend(m);
                break None;
            }
            if !missing.is_empty() {
                break None;
            }
            match inflate::framing(self.wrapper, self.held.bytes(), run_len)? {
                Framed::At(f) => break Some(f),
                Framed::NeedsHead(n) => {
                    if n <= self.held.len() {
                        return Err(Refusal::Failed);
                    }
                    if !self.reads.fits(n) {
                        return Err(Refusal::TooMuchAtOnce);
                    }
                    // Twice as much each time, as much as one read may take.
                    want = n.max(want.saturating_mul(2).min(self.reads.most_at_once())).min(run_len);
                }
            }
        };
        let Some(f) = found else {
            missing.sort_unstable_by_key(|m| m.chunk);
            missing.dedup();
            return Ok(Some(missing));
        };
        let says = match (self.wrapper, self.tail.as_deref()) {
            (Wrapper::Gzip, Some(t)) if t.len() >= 4 => u32::from_le_bytes(t[t.len() - 4..].try_into().expect("four bytes")),
            _ => 0,
        };
        let large = matches!(self.phase, Phase::Start { large: true }) || says as usize > self.limits.whole;
        self.framing = Some(f);
        self.phase = self.started(f, large);
        Ok(None)
    }

    /// One step's decode, from `held`, which holds its window, starting at
    /// `from` and stopping at the first place to stop at or past bit `stop`
    /// at the latest.
    fn decode(&mut self, held: &PassWindow, f: Framing, from: Option<Resume>, stop: u64, out_bytes: u64, in_bits: u64) -> Result<Stepped, Refusal> {
        let run_len = self.reads.len() as usize;
        // The header of the block the step starts inside, from the copy.
        let head = match from.and_then(|r| r.block) {
            None => None,
            Some(h) => match &self.head {
                Some((at, copy)) if *at == h => Some((h, copy.clone())),
                _ => return Err(Refusal::Failed),
            },
        };
        let input = Input::new(held.window(run_len), head.as_ref().and_then(|(h, copy)| Window::of(copy, (h / 8) as usize, run_len)));
        // Counted from where the step starts, not from the header of the
        // block it starts in, which it reads again: a step has to get past
        // its first code however long the block's tables are.
        let bits_end = |at: u64| at.saturating_add(in_bits.max(1)).min(stop);
        match std::mem::replace(&mut self.phase, Phase::Over) {
            Phase::Whole { mut sink, mut trace, from } => {
                sink.slice_end = sink.len().saturating_add(out_bytes.max(1));
                sink.slice_bits = bits_end(from.map_or(f.start, |r| r.in_bit));
                let ran = inflate::run(input, f.start, f.end, &mut sink, &mut trace, from);
                self.copy_heads(&mut sink, held)?;
                match ran {
                    Ok(Ended::Stopped(_)) => {
                        let from = sink.stopped.take();
                        // Summed this far and no further: past the cap the
                        // pass goes back to the last checkpoint, and the sums
                        // have to go back with it.
                        if let Some(at) = sink.marks.as_ref().and_then(|m| m.latest.as_ref()).map(|c| c.out) {
                            sink.sum_to(at);
                        }
                        self.keep_head(from, &sink, held)?;
                        self.phase = Phase::Whole { sink, trace, from };
                        Ok(Stepped::Working)
                    }
                    Ok(Ended::Stream(_)) => {
                        let sums = sink.sums.expect("set when the pass started");
                        let out = sink.hist;
                        let rest = &out[sums.upto as usize..];
                        let crc = || crc32_update(sums.crc, rest);
                        let adler = || adler32_update(sums.adler, rest);
                        if !inflate::trailer_agrees(&f, self.trailer(&f), out.len() as u64, crc, adler) {
                            return Err(Refusal::Failed);
                        }
                        if f.wrapper != Wrapper::Raw {
                            trace.push(f.end, out.len() as u64, StepKind::Header(StepField::Wrapper, 0));
                            trace.finish_at(run_len as u64 * 8, out.len() as u64);
                        }
                        Ok(Stepped::Done(Decoded::Whole(out, trace.done())))
                    }
                    Err(Refusal::TooLarge) => {
                        self.phase = self.past_the_cap(sink);
                        Ok(Stepped::Working)
                    }
                    // The pass's trace goes on to the end of a block when
                    // it turns coarse.
                    Ok(Ended::Coarse(_)) => Err(Refusal::Failed),
                    Err(other) => Err(other),
                }
            }
            Phase::Index { mut sink, from } => {
                sink.slice_end = sink.len().saturating_add(out_bytes.max(1));
                sink.slice_bits = bits_end(from.in_bit);
                let ran = inflate::run(input, f.start, f.end, &mut sink, &mut TraceBuilder::off(), Some(from));
                self.copy_heads(&mut sink, held)?;
                match ran {
                    Ok(Ended::Stopped(_)) => {
                        let from = sink.stopped.take().expect("only a step's end stops this decode");
                        self.keep_head(Some(from), &sink, held)?;
                        self.phase = Phase::Index { sink, from };
                        Ok(Stepped::Working)
                    }
                    Ok(Ended::Stream(blocks_end)) => self.finish(sink, blocks_end).map(Stepped::Done),
                    Ok(Ended::Coarse(_)) => Err(Refusal::Failed),
                    Err(Refusal::TooLarge) => Err(Refusal::TooLargeLazy),
                    Err(other) => Err(other),
                }
            }
            Phase::Start { .. } | Phase::Over => Err(Refusal::Failed),
        }
    }

    /// The output has passed the cap. Go back to the last checkpoint, which
    /// is at most the spacing before where the decode stopped, and carry on
    /// from there keeping the window and nothing more, with the sums taken
    /// over the output as it leaves the window. The next step reads its
    /// window again from there, and the block header from the checkpoint's
    /// copy.
    fn past_the_cap(&mut self, mut whole: WindowSink) -> Phase {
        let marks = whole.marks.take().expect("the first pass keeps checkpoints");
        let last = marks.latest.clone().expect("checkpoint 0 is taken before any output");
        whole.sum_to(last.out);
        let sums = whole.sums;
        // The decode from the checkpoint records again every block that
        // starts at or after it, so those are dropped here.
        let mut blocks = whole.blocks.take().expect("the first pass records blocks");
        blocks.truncate(last.blocks());
        drop(whole);
        self.head = match &last.start {
            Start::Deflate { block: Some(h), head: Some(copy), .. } => Some((*h, copy.clone())),
            _ => None,
        };
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
        let f = self.framing.ok_or(Refusal::Failed)?;
        let len = sink.len();
        sink.sum_to(len);
        let sums = sink.sums.expect("set when the pass started");
        if !inflate::trailer_agrees(&f, self.trailer(&f), len, || sums.crc, || sums.adler) {
            return Err(Refusal::Failed);
        }
        let checkpoints = sink.marks.take().expect("set when the pass started").list;
        let mut s = seekable(self.codec, self.input.clone(), Kind::Deflate(f), len, checkpoints, self.limits);
        s.blocks = sink.blocks.take().expect("set when the pass started");
        s.blocks_end = blocks_end;
        s.crc32 = Some(sums.crc);
        s.adler32 = Some(sums.adler);
        Ok(Decoded::Indexed(s))
    }

    /// The bytes of the run the wrapper's trailer is, from the run's last
    /// bytes, which the first step read. Nothing for a raw run.
    fn trailer(&self, f: &Framing) -> &[u8] {
        let tail = self.tail.as_deref().unwrap_or(&[]);
        let range = f.trailer();
        let base = self.reads.len() as usize - tail.len();
        range.start.checked_sub(base).and_then(|from| tail.get(from..from + range.len())).unwrap_or(&[])
    }
}

impl DeflatePass {
    /// Give each checkpoint the step took inside a Huffman-coded block a copy
    /// of the block's header and tables, which a decode starting there reads
    /// again, so that it need not read the run as far back as the block's
    /// start. The checkpoints inside one block share one copy: the one the
    /// pass holds for the block the step started inside, or one made from
    /// the step's window, which holds every block that started in the step,
    /// header, tables and the bytes after them up to [`MAX_HEAD_BYTES`].
    fn copy_heads(&self, sink: &mut WindowSink, held: &PassWindow) -> Result<(), Refusal> {
        let Some(marks) = sink.marks.as_mut() else { return Ok(()) };
        let run_len = self.reads.len() as usize;
        let mut last = self.head.clone();
        for c in marks.list.iter_mut().chain(marks.latest.iter_mut()) {
            let Start::Deflate { block: Some(h), head: copy @ None, .. } = &mut c.start else { continue };
            let made = match &last {
                Some((at, made)) if *at == *h => made.clone(),
                _ => {
                    let made = head_copy(held, *h, run_len)?;
                    last = Some((*h, made.clone()));
                    made
                }
            };
            *copy = Some(made);
        }
        Ok(())
    }

    /// Keep a copy of the header of the block the step stopped inside, `from`
    /// says which, for the next step: the copy the pass holds already, or
    /// that of the checkpoint taken last, or one made from the window.
    fn keep_head(&mut self, from: Option<Resume>, sink: &WindowSink, held: &PassWindow) -> Result<(), Refusal> {
        let Some(h) = from.and_then(|r| r.block) else {
            self.head = None;
            return Ok(());
        };
        if self.head.as_ref().is_some_and(|(at, _)| *at == h) {
            return Ok(());
        }
        let marked = sink.marks.as_ref().and_then(|m| m.latest.as_ref()).and_then(|c| match &c.start {
            Start::Deflate { block: Some(b), head: Some(copy), .. } if *b == h => Some(copy.clone()),
            _ => None,
        });
        let copy = match marked {
            Some(copy) => copy,
            None => head_copy(held, h, self.reads.len() as usize)?,
        };
        self.head = Some((h, copy));
        Ok(())
    }
}

/// A copy of the header and tables of the block whose header starts at bit
/// `h` of the run: the bytes from the one `h` is in, [`MAX_HEAD_BYTES`] of
/// them or to the run's end, from `held`, which has to hold them.
fn head_copy(held: &PassWindow, h: u64, run_len: usize) -> Result<Arc<[u8]>, Refusal> {
    let from = (h / 8) as usize;
    held.window(run_len).get(from..(from + MAX_HEAD_BYTES).min(run_len)).map(Arc::from).ok_or(Refusal::Failed)
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
