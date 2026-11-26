//! Indexing an xz stream of several blocks, from the index at its end.

use std::sync::Arc;

use super::seekable::{seekable, Kind};
use super::{whole, Checkpoints, Decoded, Limits, Start};
use crate::codec::{xz, Codec, Refusal};

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
