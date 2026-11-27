//! Indexing a zstd run of several frames, a step at a time, and decoding and
//! tracing its spans.

use std::ops::Range;
use std::sync::Arc;

use super::input::{Copied, ReadError, RunInput};
use super::seekable::{seekable, Kind};
use super::{Checkpoints, Decoded, Limits, Progress, Start, Stepped};
use crate::codec::frames::{self, FrameAt, Walked, ZstdWalk};
use crate::codec::{Codec, Refusal, SpanUnit, StepField, StepKind, Trace, TraceBuilder, Window, MAX_STEPS};

/// A zstd run's index pass, done a step at a time as deflate's is. Frames
/// copy nothing from the ones before them, so a checkpoint is the start of a
/// frame and keeps no window, and the pass past the cap starts again at the
/// start of the frame the first part was in, which costs at most one frame
/// decoded twice.
///
/// A step stops between two zstd blocks, not only between frames, since the
/// walk keeps the frame's decoder from one step to the next; see
/// [`ZstdWalk::run`] for how far past its share of output, and of the run, a
/// step goes.
///
/// A frame larger than [`Limits::span`] is refused as
/// [`Refusal::TooLargeUnit`] however many frames there are, since a span is
/// decoded whole and a frame cannot be entered anywhere but at its start. A
/// frame whose header asks for a window over 128 MiB is refused before it is
/// decoded, as [`Refusal::TooLarge`], the refusal the whole decoder gives it,
/// since nothing says yet how large the frame is. A file of one frame is the
/// usual case: `zstd` writes one frame whatever `-T` and `-B` say, and only
/// `pzstd`, or files joined end to end, have several.
///
/// A checkpoint is due at a frame by the output, or by the input up to where
/// the frame starts itself, past the skippable frames in front of it. A frame
/// whose own input is the input spacing or more is a span of its own: the
/// pass learns its size when it ends, and takes a checkpoint at its start
/// then. Skippable frames after the last frame are a span of their own too
/// when they come to the input spacing, so that no span's read of its bytes
/// reads them.
///
/// A span is read whole, so when the index will read the run from a source
/// that keeps only part of the file, a frame whose own input passes
/// [`Limits::unit_input`] is refused as [`Refusal::TooMuchAtOnce`]: a frame the
/// pass has finished once it is past the cap, and one finished while the run
/// might still be held whole once the run turns out not to fit.
pub(super) struct ZstdPass {
    packed: Arc<Vec<u8>>,
    /// Where the index reads the run from once it is made.
    input: RunInput,
    limits: Limits,
    marks: Checkpoints,
    phase: Phase,
    /// The frame the walk is in, or the one it announced last: none since
    /// the walk last started.
    open: Option<FrameAt>,
    /// The furthest bit of the run a step has stopped at, and the most output
    /// let go of, kept apart from the walk, which goes back a frame at the
    /// cap.
    furthest: u64,
    most: u64,
}

enum Phase {
    /// The part that may come to a whole run: traced, and with the output
    /// kept, and checkpoints taken in case it does not. `last` is the frame
    /// it is in, as the byte it starts at and the output before it, and
    /// `largest` the most output a frame it has finished came to, and
    /// `widest` the most input one took. A frame may come to as much as the
    /// whole run may here, and the frames this part finished are held to the
    /// span limit, and to the limit on input, only if the run turns out not
    /// to fit.
    Whole { walk: ZstdWalk, out: Vec<u8>, trace: TraceBuilder, last: (usize, u64), largest: u64, widest: usize },
    /// Past the cap: the output is counted, to `count`, and none of it kept.
    Index { walk: ZstdWalk, count: u64 },
    /// Finished, or refused.
    Over,
}

impl ZstdPass {
    pub(super) fn new(packed: Arc<Vec<u8>>, input: RunInput, limits: Limits) -> ZstdPass {
        let walk = ZstdWalk::new(0..packed.len(), 0, limits.span.max(limits.whole) as u64);
        let phase = Phase::Whole { walk, out: Vec::new(), trace: TraceBuilder::default(), last: (0, 0), largest: 0, widest: 0 };
        let marks = Checkpoints::new(&limits);
        ZstdPass { packed, input, limits, marks, phase, open: None, furthest: 0, most: 0 }
    }

    /// The same pass for a run the caller knows comes to more than the cap,
    /// which starts as the pass carries on past the cap: with the trace off
    /// and the output counted, from the first frame. It comes to the same
    /// checkpoints, and a run that fits under the cap after all comes back
    /// indexed too.
    pub(super) fn known_large(mut self) -> ZstdPass {
        if matches!(&self.phase, Phase::Whole { walk, .. } if walk.reached() == 0) {
            let walk = ZstdWalk::new(0..self.packed.len(), 0, self.limits.span as u64);
            self.phase = Phase::Index { walk, count: 0 };
        }
        self
    }

    pub(super) fn progress(&self) -> Progress {
        let run_bits = self.packed.len() as u64 * 8;
        Progress { read_bits: self.furthest, run_bits, unpacked: self.most }
    }

    pub(super) fn step(&mut self, out_bytes: u64, in_bits: u64) -> Result<Stepped, Refusal> {
        let stepped = self.step_on(out_bytes, usize::try_from(in_bits / 8).unwrap_or(usize::MAX));
        let (at, out) = match &self.phase {
            Phase::Whole { walk, .. } | Phase::Index { walk, .. } => (walk.reached() as u64 * 8, walk.let_go()),
            Phase::Over => (self.packed.len() as u64 * 8, 0),
        };
        self.furthest = self.furthest.max(at);
        self.most = self.most.max(out);
        stepped
    }

    fn step_on(&mut self, out_bytes: u64, reach: usize) -> Result<Stepped, Refusal> {
        let data = &self.packed[..];
        let limits = self.limits;
        let marks = &mut self.marks;
        let open = &mut self.open;
        let input = &self.input;
        // The frame before ends where this one's skippable frames start. How
        // much input it took, when there was one.
        let mark = |f: FrameAt, marks: &mut Checkpoints, open: &mut Option<FrameAt>| {
            let took = open.replace(f).map(|was| ended(marks, was, f.front));
            marks.offer(f.front as u64 * 8, f.own as u64 * 8, f.out, || Start::Frame { own: f.own });
            took
        };
        let too_much = |took: Option<usize>| match took {
            Some(n) if input.too_much(n, limits.unit_input) => Err(Refusal::TooMuchAtOnce),
            _ => Ok(()),
        };
        // The last frame ends where the walk says, and any skippable frames
        // after it are a span of their own once they come to the spacing.
        let finish = |walk: &ZstdWalk, marks: &mut Checkpoints, open: &mut Option<FrameAt>, len: u64| {
            if let Some(was) = open.take() {
                too_much(Some(ended(marks, was, walk.frames_end())))?;
            }
            let rest = walk.frames_end();
            if rest < data.len() && marks.due_by_input(data.len() as u64 * 8) {
                marks.take(rest as u64 * 8, len, || Start::Frame { own: data.len() });
            }
            Ok(())
        };
        match std::mem::replace(&mut self.phase, Phase::Over) {
            Phase::Whole { mut walk, mut out, mut trace, mut last, mut largest, mut widest } => {
                let mut over = false;
                let walked = walk.run(
                    Window::whole(data),
                    out_bytes,
                    reach,
                    &mut trace,
                    &mut |f| {
                        largest = largest.max(f.out - last.1);
                        last = (f.front, f.out);
                        widest = widest.max(mark(f, marks, open).unwrap_or(0));
                        Ok(())
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
                match walked.and_then(Walked::of_whole) {
                    Ok(None) => {
                        self.phase = Phase::Whole { walk, out, trace, last, largest, widest };
                        Ok(Stepped::Working)
                    }
                    Ok(Some(end)) => Ok(Stepped::Done(Decoded::Whole(out, frames::zstd_done(trace, data.len(), end)))),
                    Err(Refusal::TooLarge) if over => {
                        drop((out, trace));
                        if largest > limits.span as u64 {
                            return Err(Refusal::TooLargeUnit(SpanUnit::ZstdFrame));
                        }
                        too_much(Some(widest))?;
                        // Past the cap: on from the start of the frame the
                        // walk stopped in, counting the output and keeping
                        // none of it.
                        let (at, before) = last;
                        let walk = ZstdWalk::new(at..data.len(), before, limits.span as u64);
                        *open = None;
                        self.phase = Phase::Index { walk, count: before };
                        Ok(Stepped::Working)
                    }
                    // A frame past the walk's cap, which is at least the span
                    // limit.
                    Err(Refusal::TooLarge) if walk.frame_too_large() => Err(Refusal::TooLargeUnit(SpanUnit::ZstdFrame)),
                    Err(other) => Err(other),
                }
            }
            Phase::Index { mut walk, mut count } => {
                let walked = walk.run(Window::whole(data), out_bytes, reach, &mut TraceBuilder::off(), &mut |f| too_much(mark(f, marks, open)), &mut |bytes| {
                    count += bytes.len() as u64;
                    match count > limits.lazy {
                        true => Err(Refusal::TooLargeLazy),
                        false => Ok(()),
                    }
                });
                let walked = match walked.and_then(Walked::of_whole) {
                    // A frame past the walk's cap, which is the span limit.
                    Err(Refusal::TooLarge) if walk.frame_too_large() => return Err(Refusal::TooLargeUnit(SpanUnit::ZstdFrame)),
                    w => w?,
                };
                match walked {
                    None => {
                        self.phase = Phase::Index { walk, count };
                        Ok(Stepped::Working)
                    }
                    Some(len) => {
                        finish(&walk, marks, open, len)?;
                        let list = std::mem::take(&mut marks.list);
                        Ok(Stepped::Done(Decoded::Indexed(seekable(Codec::Zstd, self.input.clone(), Kind::Zstd, len, list, limits))))
                    }
                }
            }
            Phase::Over => Err(Refusal::Failed),
        }
    }
}

/// A frame announced as `f` has ended at byte `end`: one whose own input is
/// the input spacing or more gets a checkpoint at its start, if it has none,
/// so that it is a span of its own. The next frame then starts a span as
/// well, since it starts the spacing past that checkpoint or more. How many
/// bytes of input the frame took, without the skippable frames in front.
fn ended(marks: &mut Checkpoints, f: FrameAt, end: usize) -> usize {
    let took = end - f.own;
    marks.take_if_long(f.front as u64 * 8, took as u64 * 8, f.out, || Start::Frame { own: f.own });
    took
}

/// The output of the frames in bytes `range` of the run, read from `window`,
/// which holds them, and which come after `out` bytes of output and come to
/// `want` bytes. The same walk over the same bytes as the pass that indexed
/// them, so a span that comes out another length is the decoder disagreeing
/// with itself.
pub(super) fn decode_span(window: Window<'_>, range: Range<usize>, out: u64, want: usize, frame_cap: usize) -> Result<Vec<u8>, Refusal> {
    let mut bytes = Vec::with_capacity(want);
    frames::zstd_walk(window, range, out, frame_cap as u64, &mut TraceBuilder::off(), &mut |b| {
        if bytes.len() + b.len() > want {
            return Err(Refusal::Failed);
        }
        bytes.extend_from_slice(b);
        Ok(())
    })?;
    Ok(bytes)
}

/// The trace of a span that starts at byte `front` of the run, after `out`
/// bytes of output: a step for each skippable frame at `skips`, which are in
/// front of the span's first frame, and then the frames in bytes `frames` of
/// the run, read from `window`, which holds them. The steps a whole run's
/// trace has over the same bytes, counted from the front of the run and of
/// the output. `window` may hold skippable frames too, which the walk reads
/// as a whole run's walk does; `skips` is for those it does not hold.
pub(super) fn span_trace(window: Window<'_>, front: usize, skips: &[usize], frames: Range<usize>, out: u64, frame_cap: usize) -> Result<Trace, Refusal> {
    let mut b = TraceBuilder::with_budget(MAX_STEPS).starting_at(front as u64 * 8, out);
    for &at in skips {
        b.push(at as u64 * 8, out, StepKind::Header(StepField::FrameHeader, 0));
    }
    let end = frames::zstd_walk(window, frames.clone(), out, frame_cap as u64, &mut b, &mut |_| Ok(()))?;
    b.finish_at(frames.end as u64 * 8, end);
    Ok(b.done())
}

/// How much of the skippable frames in front of a span's first frame one read
/// takes, when a span's trace looks for where each starts. The contents of a
/// longer one are skipped and not read.
pub(super) const FRONT_PIECE: usize = 64 << 10;

/// Where each skippable frame in bytes `front..own` of the run starts, read
/// from their headers: from `first`, a copy of the run from `front`, and then
/// through `input` a piece of up to [`FRONT_PIECE`] at a time. They have to
/// end at `own`, where the frame they are in front of starts.
pub(super) fn skippable_starts(input: &RunInput, front: usize, own: usize, first: Option<Copied>) -> Result<Vec<usize>, ReadError> {
    const HEADER: usize = 8;
    let (mut starts, mut piece, mut at) = (Vec::new(), first, front);
    while at < own {
        if !piece.as_ref().is_some_and(|p| p.at <= at && at + HEADER <= p.at + p.bytes.len()) {
            piece = Some(input.copy(at..own.min(at + FRONT_PIECE))?);
        }
        let p = piece.as_ref().expect("read just now if not before");
        let size = frames::skippable(&p.bytes[at - p.at..], own - at).ok_or(Refusal::Failed)?;
        starts.push(at);
        at += size;
    }
    Ok(starts)
}
