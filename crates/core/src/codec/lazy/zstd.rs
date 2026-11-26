//! Indexing a zstd run of several frames, in one pass.

use std::sync::Arc;

use super::seekable::{seekable, Kind};
use super::{Checkpoints, Decoded, Limits, Start};
use crate::codec::{frames, Codec, Refusal, TraceBuilder};

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
pub(super) fn index_zstd(packed: Arc<Vec<u8>>, limits: Limits) -> Result<Decoded, Refusal> {
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
