//! Indexing a zstd run of several frames, a step at a time, and decoding and
//! tracing its spans.

use std::sync::Arc;

use super::input::RunInput;
use super::seekable::{seekable, Kind};
use super::{Checkpoints, Decoded, Limits, Progress, Start, Stepped};
use crate::codec::frames::{self, Walked, ZstdWalk};
use crate::codec::{Codec, Refusal, SpanUnit, Trace, TraceBuilder, Window, MAX_STEPS};

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
pub(super) struct ZstdPass {
    packed: Arc<Vec<u8>>,
    limits: Limits,
    marks: Checkpoints,
    phase: Phase,
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
    /// `largest` the largest frame it has finished. A frame may come to as
    /// much as the whole run may here, and the frames this part finished are
    /// held to the span limit only if the run turns out not to fit.
    Whole { walk: ZstdWalk, out: Vec<u8>, trace: TraceBuilder, last: (usize, u64), largest: u64 },
    /// Past the cap: the output is counted, to `count`, and none of it kept.
    Index { walk: ZstdWalk, count: u64 },
    /// Finished, or refused.
    Over,
}

impl ZstdPass {
    pub(super) fn new(packed: Arc<Vec<u8>>, limits: Limits) -> ZstdPass {
        let walk = ZstdWalk::new(0..packed.len(), 0, limits.span.max(limits.whole) as u64);
        let phase = Phase::Whole { walk, out: Vec::new(), trace: TraceBuilder::default(), last: (0, 0), largest: 0 };
        let marks = Checkpoints::new(limits.spacing, limits.checkpoints);
        ZstdPass { packed, limits, marks, phase, furthest: 0, most: 0 }
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
        let mark = |at: usize, out: u64, marks: &mut Checkpoints| marks.offer(at as u64 * 8, out, || Start::Unit(0));
        match std::mem::replace(&mut self.phase, Phase::Over) {
            Phase::Whole { mut walk, mut out, mut trace, mut last, mut largest } => {
                let mut over = false;
                let walked = walk.run(
                    Window::whole(data),
                    out_bytes,
                    reach,
                    &mut trace,
                    &mut |at, o| {
                        largest = largest.max(o - last.1);
                        last = (at, o);
                        mark(at, o, marks);
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
                        self.phase = Phase::Whole { walk, out, trace, last, largest };
                        Ok(Stepped::Working)
                    }
                    Ok(Some(end)) => Ok(Stepped::Done(Decoded::Whole(out, frames::zstd_done(trace, data.len(), end)))),
                    Err(Refusal::TooLarge) if over => {
                        drop((out, trace));
                        if largest > limits.span as u64 {
                            return Err(Refusal::TooLargeUnit(SpanUnit::ZstdFrame));
                        }
                        // Past the cap: on from the start of the frame the
                        // walk stopped in, counting the output and keeping
                        // none of it.
                        let (at, before) = last;
                        let walk = ZstdWalk::new(at..data.len(), before, limits.span as u64);
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
                let walked = walk.run(Window::whole(data), out_bytes, reach, &mut TraceBuilder::off(), &mut |at, o| mark(at, o, marks), &mut |bytes| {
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
                        let list = std::mem::take(&mut marks.list);
                        Ok(Stepped::Done(Decoded::Indexed(seekable(Codec::Zstd, RunInput::held(self.packed.clone()), Kind::Zstd, len, list, limits))))
                    }
                }
            }
            Phase::Over => Err(Refusal::Failed),
        }
    }
}

/// The output of the frames in bytes `range` of the run, read from `window`,
/// which holds them, and which come after `out` bytes of output and come to
/// `want` bytes. The same walk over the same bytes as the pass that indexed
/// them, so a span that comes out another length is the decoder disagreeing
/// with itself.
pub(super) fn decode_span(window: Window<'_>, range: std::ops::Range<usize>, out: u64, want: usize, frame_cap: usize) -> Result<Vec<u8>, Refusal> {
    let mut bytes = Vec::with_capacity(want);
    frames::zstd_walk(window, range, out, frame_cap as u64, &mut TraceBuilder::off(), &mut |_, _| {}, &mut |b| {
        if bytes.len() + b.len() > want {
            return Err(Refusal::Failed);
        }
        bytes.extend_from_slice(b);
        Ok(())
    })?;
    Ok(bytes)
}

/// The trace of the frames in bytes `range` of the run, read from `window`,
/// which come after `out` bytes of output: the steps a whole run's trace has
/// over the same frames, counted from the front of the run and of the output.
pub(super) fn span_trace(window: Window<'_>, range: std::ops::Range<usize>, out: u64, frame_cap: usize) -> Result<Trace, Refusal> {
    let mut b = TraceBuilder::with_budget(MAX_STEPS).starting_at(range.start as u64 * 8, out);
    let end = frames::zstd_walk(window, range.clone(), out, frame_cap as u64, &mut b, &mut |_, _| {}, &mut |_| Ok(()))?;
    b.finish_at(range.end as u64 * 8, end);
    Ok(b.done())
}
