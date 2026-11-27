//! Indexing an xz stream of several blocks, from the index at its end, and
//! reading its spans.

use std::ops::Range;
use std::sync::{Arc, Mutex};

use super::input::{ReadError, RunInput};
use super::seekable::{lock, seekable, Kind, Recent};
use super::{whole, Checkpoints, Decoded, Limits, Start};
use crate::codec::lzma::{self, Lzma2, LZMA2_CHUNK_MAX};
use crate::codec::{xz, Codec, Refusal, SpanUnit, StepField, StepKind, Trace, TraceBuilder, Window, MAX_STEPS};

/// An xz stream, indexed from the index at its end without decoding any of
/// it: that index lists every block's packed and unpacked size, and an xz
/// block never copies from an earlier block, so a checkpoint is the start of
/// a block.
///
/// A stream whose index adds up to no more than [`Limits::whole`], and a
/// stream [`xz::layout`] cannot read, go to the whole decoder as before, which
/// decodes them or refuses them. A larger stream is refused as too large when
/// its blocks use a filter our LZMA2 decoder does not run, because `lzma-rs`,
/// which runs those filters, reads only whole streams. It is refused as
/// [`Refusal::TooLargeUnit`] when any block unpacks to more than
/// [`Limits::span`], which is what a file of one block, as plain `xz` writes,
/// is.
///
/// Nothing is decoded here, so a block whose unpacked size differs from the
/// index is found only when a read reaches the block, and that read is
/// refused. See [`Prefix`].
pub(super) fn index_xz(packed: Arc<Vec<u8>>, input: RunInput, limits: Limits) -> Result<Decoded, Refusal> {
    let Some(l) = xz::layout(&packed).filter(|l| l.total_out > limits.whole as u64) else {
        return whole(Codec::Xz, &packed);
    };
    if !l.plain {
        return Err(Refusal::TooLarge);
    }
    if l.blocks.iter().any(|b| b.unpacked > limits.span as u64) {
        return Err(Refusal::TooLargeUnit(SpanUnit::XzBlock));
    }
    if l.total_out > limits.lazy {
        return Err(Refusal::TooLargeLazy);
    }
    let mut marks = Checkpoints::new(&limits);
    let mut out = 0u64;
    for (i, blk) in l.blocks.iter().enumerate() {
        // The first span starts with the stream header, which is no block's.
        let at = if i == 0 { 0 } else { blk.at as u64 * 8 };
        // A block of the input spacing or more is a span of its own: the
        // block after it starts the spacing on, so is due by the input.
        marks.take_if_long(at, l.block_len(i) as u64 * 8, out, || Start::Unit(i));
        marks.offer(at, at, out, || Start::Unit(i));
        out += blk.unpacked;
    }
    let len = l.total_out;
    Ok(Decoded::Indexed(seekable(Codec::Xz, input, Kind::Xz(l), len, marks.list, limits)))
}

/// Bytes `from..from + dst.len()` of span `k`, whose blocks are `units`,
/// decoded only as far as the read reaches.
///
/// What a span has been decoded to is kept in `cache`, with the decoder that
/// carries it on, so a read further into a span carries on from where the
/// last read of it stopped, and a read of bytes already decoded decodes
/// nothing. The cache weighs each span by the memory it holds and drops the
/// span read longest ago, as the cache of whole spans does.
///
/// A read that fails drops what was decoded of its span, and the next read of
/// the span starts again from the span's first block.
///
/// The blocks are read from `window`, a window of the stream that holds the
/// LZMA2 data of every block the read decodes.
#[cfg(test)]
pub(super) fn read_span(
    cache: &Mutex<Recent<Prefix>>,
    window: Window<'_>,
    l: &xz::Layout,
    units: Range<usize>,
    k: usize,
    from: usize,
    dst: &mut [u8],
) -> Result<(), Refusal> {
    let held = lock(cache).take(k as u64);
    let mut p = held.unwrap_or_else(|| Prefix::new(l, units.clone()));
    p.reach(window, l, units, from + dst.len())?;
    p.copy(from, dst);
    let weight = p.weight();
    lock(cache).put(k as u64, p, weight);
    Ok(())
}

/// [`read_span`], with the blocks read through `input`: only the blocks the
/// read decodes, each one's LZMA2 data whole (see [`Prefix::needs`]). A read
/// that finds chunks of the input missing decodes nothing, and keeps what was
/// decoded of the span for the read made again once they are there.
pub(super) fn read_span_through(
    cache: &Mutex<Recent<Prefix>>,
    input: &RunInput,
    l: &xz::Layout,
    units: Range<usize>,
    k: usize,
    from: usize,
    dst: &mut [u8],
) -> Result<(), ReadError> {
    let upto = from + dst.len();
    let held = lock(cache).take(k as u64);
    let mut p = held.unwrap_or_else(|| Prefix::new(l, units.clone()));
    let copy = match p.needs(l, units.clone(), upto).map(|range| input.copy(range)).transpose() {
        Ok(copy) => copy,
        Err(missing) => {
            let weight = p.weight();
            lock(cache).put(k as u64, p, weight);
            return Err(ReadError::Missing(missing));
        }
    };
    let run_len = input.len() as usize;
    let window = match &copy {
        Some(copy) => copy.window(input.len()),
        None => Window::of(&[], 0, run_len).expect("an empty window"),
    };
    p.reach(window, l, units, upto)?;
    p.copy(from, dst);
    let weight = p.weight();
    lock(cache).put(k as u64, p, weight);
    Ok(())
}

/// A span decoded from its start as far as reads have reached, and the
/// decoder that carries it on from there.
///
/// Decoding stops between two LZMA2 chunks, so a read decodes up to the end
/// of the chunk its last byte is in. A chunk comes to at most 2 MiB, and an
/// encoder also ends one at 64 KiB of packed data, so on text a read decodes
/// a megabyte or two past its end at most.
///
/// Each block decodes into a vector of its own, which is the block's
/// dictionary and its output, as when the whole stream is decoded. No vector
/// is given more room than its block comes to, so the bytes held for a span
/// never come to more than the span's length.
pub(super) struct Prefix {
    /// What the span comes to, from the index.
    len: usize,
    /// The span's first block, counted in the stream's index.
    first: usize,
    /// Each block's output, from the span's first block on. All of them are
    /// decoded to their end except the last while `decoder` is there.
    blocks: Vec<Vec<u8>>,
    /// The last block's decoder, between two of its chunks.
    decoder: Option<Lzma2>,
    /// How many bytes `blocks` hold between them.
    decoded: usize,
}

impl Prefix {
    fn new(l: &xz::Layout, units: Range<usize>) -> Prefix {
        let len = l.blocks[units.clone()].iter().map(|b| b.unpacked as usize).sum();
        Prefix { len, first: units.start, blocks: Vec::new(), decoder: None, decoded: 0 }
    }

    /// How many bytes of the span are decoded.
    #[cfg(test)]
    pub(super) fn decoded(&self) -> usize {
        self.decoded
    }

    /// The memory this holds: the blocks' bytes and the room made for the
    /// next chunk's, and the decoder's models.
    pub(super) fn weight(&self) -> usize {
        self.blocks.iter().map(Vec::capacity).sum::<usize>()
            + self.blocks.capacity() * std::mem::size_of::<Vec<u8>>()
            + self.decoder.as_ref().map_or(0, Lzma2::held_bytes)
    }

    /// The bytes of the stream that [`Prefix::reach`] reads to decode the
    /// first `upto` bytes of the span: the LZMA2 data of every block from the
    /// one being decoded, or the next to start, to the one holding byte
    /// `upto - 1` of the span, or to the span's last block for a read that
    /// reaches the span's end. A block's data is read whole, since its chunk
    /// headers are all read before its first chunk is decoded. Nothing when
    /// the read decodes nothing.
    fn needs(&self, l: &xz::Layout, units: Range<usize>, upto: usize) -> Option<Range<usize>> {
        if self.decoded >= upto && upto < self.len {
            return None;
        }
        let from = self.first + self.blocks.len() - usize::from(self.decoder.is_some());
        let to = match upto >= self.len {
            true => units.end,
            false => {
                let mut end = 0usize;
                let holding = l.blocks[units.clone()].iter().position(|b| {
                    end += b.unpacked as usize;
                    end >= upto
                });
                holding.map_or(units.end, |i| units.start + i + 1)
            }
        };
        (from < to).then(|| l.blocks[from].lzma2().start..l.blocks[to - 1].lzma2().end)
    }

    /// Decode on, a chunk at a time, until the first `upto` bytes of the span
    /// are decoded. A read that reaches the end of the span finishes every
    /// block in it, even blocks that come to nothing.
    ///
    /// Before a block's first chunk is decoded, its chunk headers are read up
    /// to the byte that ends its LZMA2, and they have to add up to what the
    /// index says the block comes to. So a block the index is wrong about is
    /// refused before any of it is read. A chunk whose data is damaged is
    /// found only by a read that reaches it.
    fn reach(&mut self, window: Window<'_>, l: &xz::Layout, units: Range<usize>, upto: usize) -> Result<(), Refusal> {
        let mut off = TraceBuilder::off();
        while self.decoded < upto || upto >= self.len {
            if self.decoder.is_none() {
                let next = self.first + self.blocks.len();
                if next >= units.end {
                    break;
                }
                let blk = &l.blocks[next];
                if lzma::lzma2_len_in(lzma2_of(window, blk)?)? != blk.unpacked {
                    return Err(Refusal::Failed);
                }
                self.blocks.push(Vec::new());
                self.decoder = Some(Lzma2::new());
            }
            let blk = &l.blocks[self.first + self.blocks.len() - 1];
            let data = lzma2_of(window, blk)?;
            let size = blk.unpacked as usize;
            let (Some(dec), Some(out)) = (self.decoder.as_mut(), self.blocks.last_mut()) else { return Err(Refusal::Failed) };
            let before = out.len();
            grow(out, before + LZMA2_CHUNK_MAX, size);
            let mut ended = dec.chunk_in(data, out, &mut off)?;
            // At the block's last byte, read the byte that ends its LZMA2, which
            // the headers said comes next, so that a block read to its end is
            // finished and its decoder let go.
            if !ended && out.len() == size {
                ended = dec.chunk_in(data, out, &mut off)?;
            }
            if ended != (out.len() == size) || out.len() > size {
                return Err(Refusal::Failed);
            }
            self.decoded += out.len() - before;
            if ended {
                self.decoder = None;
            }
        }
        match self.decoded < upto {
            true => Err(Refusal::Failed),
            false => Ok(()),
        }
    }

    /// Bytes `from..from + dst.len()` of the span, which are decoded.
    fn copy(&self, from: usize, dst: &mut [u8]) {
        let mut at = from;
        let mut start = 0usize;
        let mut filled = 0usize;
        for b in &self.blocks {
            if filled == dst.len() {
                break;
            }
            let end = start + b.len();
            if at < end {
                let n = (end - at).min(dst.len() - filled);
                dst[filled..filled + n].copy_from_slice(&b[at - start..at - start + n]);
                filled += n;
                at += n;
            }
            start = end;
        }
    }
}

/// The window over block `blk`'s LZMA2 data, with positions counted from that
/// data's first byte, as an LZMA2 decoder counts them: as much of it as
/// `window` holds. Refused for a block whose data runs past the stream's end.
fn lzma2_of<'a>(window: Window<'a>, blk: &xz::BlockAt) -> Result<Window<'a>, Refusal> {
    let data = blk.lzma2();
    match data.end <= window.run_len() {
        true => Ok(window.within(data)),
        false => Err(Refusal::Failed),
    }
}

/// Make room in `v` for `need` bytes, but never for more than `most`, which
/// is what it holds once it is done. Room grows by doubling, so a vector
/// filled a chunk at a time is copied a few times rather than once a chunk.
fn grow(v: &mut Vec<u8>, need: usize, most: usize) {
    let need = need.min(most);
    if v.capacity() < need {
        let room = need.max(v.capacity() * 2).min(most);
        v.reserve_exact(room.saturating_sub(v.len()));
    }
}

/// The trace of span `k`, whose bytes of the run are `range`, whose blocks are
/// `units` and whose output starts at byte `out`: the trace a whole stream has
/// over the same blocks, with the stream's header in span 0 and its index and
/// footer in the last span.
pub(super) fn span_trace(window: Window<'_>, l: &xz::Layout, k: usize, range: Range<usize>, units: Range<usize>, out: u64) -> Result<Trace, Refusal> {
    let last = units.end == l.blocks.len();
    let mut b = TraceBuilder::with_budget(MAX_STEPS).starting_at(range.start as u64 * 8, out);
    if k == 0 {
        b.push(0, 0, StepKind::Header(StepField::FrameHeader, 0));
    }
    let end = out + xz::blocks(window, l, units, out, MAX_STEPS, &mut b)?.len() as u64;
    if last {
        xz::push_index(l, &mut b, end);
    }
    b.finish_at(range.end as u64 * 8, end);
    Ok(b.done())
}
