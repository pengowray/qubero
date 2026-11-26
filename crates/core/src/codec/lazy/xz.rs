//! Indexing an xz stream of several blocks, from the index at its end, and
//! reading its spans.

use std::ops::Range;
use std::sync::Arc;

use super::seekable::{seekable, Kind};
use super::{whole, Checkpoints, Decoded, Limits, Start};
use crate::codec::{xz, Codec, Refusal, StepField, StepKind, Trace, TraceBuilder, MAX_STEPS};

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
pub(super) fn index_xz(packed: Arc<Vec<u8>>, limits: Limits) -> Result<Decoded, Refusal> {
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

/// Everything blocks `units` of the stream come to, one after another: a
/// span's bytes. This is the first time those blocks are decoded, and a block
/// that does not come to what the index says is refused here.
pub(super) fn decode_span(packed: &[u8], l: &xz::Layout, units: Range<usize>, out: u64) -> Result<Vec<u8>, Refusal> {
    xz::blocks(packed, l, units, out, 0, &mut TraceBuilder::off())
}

/// The trace of span `k`, whose bytes of the run are `range`, whose blocks are
/// `units` and whose output starts at byte `out`: the trace a whole stream has
/// over the same blocks, with the stream's header in span 0 and its index and
/// footer in the last span.
pub(super) fn span_trace(packed: &[u8], l: &xz::Layout, k: usize, range: Range<usize>, units: Range<usize>, out: u64) -> Result<Trace, Refusal> {
    let last = units.end == l.blocks.len();
    let mut b = TraceBuilder::with_budget(MAX_STEPS).starting_at(range.start as u64 * 8, out);
    if k == 0 {
        b.push(0, 0, StepKind::Header(StepField::FrameHeader, 0));
    }
    let end = out + xz::blocks(packed, l, units, out, MAX_STEPS, &mut b)?.len() as u64;
    if last {
        xz::push_index(l, &mut b, end);
    }
    b.finish_at(range.end as u64 * 8, end);
    Ok(b.done())
}
