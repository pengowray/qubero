//! zstd, traced at the block rather than at the symbol, and the one-step trace
//! every codec falls back on.
//!
//! zstd keeps the crate that reads it. What is written here is only the map:
//! how the run divides into frames and blocks, which bytes of it each block
//! is, and how much of the output each one produced. Inside a block there is
//! nothing but bytes this round, which is an honest stop rather than a missing
//! feature: a zstd block is FSE and Huffman over three interleaved streams,
//! and unpicking that is a decoder of its own.
//!
//! xz used to be read here on the same terms and is not any more. Its blocks
//! usually hold LZMA2, which we have a decoder for, so it moved to
//! [`crate::codec::xz`] and is traced to its symbols; what it kept from here
//! is [`whole`], for the streams whose filter chains it cannot run.
//!
//! How much output each block produced is what the decoder says as it goes,
//! and for the first window's worth of a frame it does not say: see
//! [`zstd_walk`]. Those blocks are one step.
//!
//! When the steps do not tile, the run still opens: the trace becomes one step
//! over all of it. A map nobody can draw is better than a stream nobody can
//! open.

use crate::codec::{Refusal, StepField, StepKind, Trace, TraceBuilder};

/// A trace of one step over the whole run, for a decoder that gave the bytes
/// but not the shape.
pub(super) fn whole(input: usize, output: usize) -> Trace {
    let mut b = TraceBuilder::default();
    if input > 0 || output > 0 {
        b.push(0, 0, StepKind::Block);
    }
    b.finish_at(input as u64 * 8, output as u64);
    b.done()
}

/// A Brotli stream: the bytes, and one step over the whole of it.
///
/// The step is as fine as the truth goes. Brotli's blocks are not delimited by
/// anything a second reader could find: the stream carries three interleaved
/// categories of block, each switching on a count the decoder maintains as it
/// goes, and the crate that reads it is not asked where the boundaries fell.
/// So this says these bits made those bytes, which is what a reader standing
/// on a byte of the output needs, and does not pretend to a map it has not
/// got.
///
/// The output is capped rather than trusted: a page header may claim any
/// uncompressed size it likes, and one of the Parquet samples claims two
/// gigabytes from three kilobytes of input.
pub fn brotli(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    use std::io::Read;
    let mut out = Vec::new();
    // One byte past the cap, so a stream that only just fits is told from one
    // that does not.
    let mut reader = brotli_decompressor::Decompressor::new(data, 4096);
    match reader.by_ref().take(super::CAP_BYTES as u64 + 1).read_to_end(&mut out) {
        Ok(_) if out.len() > super::CAP_BYTES => Err(Refusal::TooLarge),
        Ok(_) => {
            let n = out.len();
            Ok((out, whole(data.len(), n)))
        }
        Err(_) => Err(Refusal::Failed),
    }
}

/// A zstd run: the bytes, and a step per frame header and per block.
///
/// Every frame of the run, not only the first: `pzstd` writes one frame per
/// few megabytes, each behind a skippable frame saying how long it is, and a
/// file made by joining two `.zst` files is two frames. See [`zstd_walk`].
pub fn zstd(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    let cap = super::CAP_BYTES as u64;
    let mut out = Vec::new();
    let mut b = TraceBuilder::default();
    let end = zstd_walk(data, 0..data.len(), 0, cap, &mut b, &mut |_, _| {}, &mut |bytes| {
        if out.len() + bytes.len() > cap as usize {
            return Err(Refusal::TooLarge);
        }
        out.extend_from_slice(bytes);
        Ok(())
    })?;
    let trace = zstd_done(b, data.len(), end);
    Ok((out, trace))
}

/// The trace a walk over a whole run recorded, ended at the end of the run, or
/// one step over all of it when the steps do not tile.
pub(super) fn zstd_done(mut b: TraceBuilder, input: usize, output: u64) -> Trace {
    b.finish_at(input as u64 * 8, output);
    let t = b.done();
    match t.check_tiles() {
        Ok(()) => t,
        Err(_) => whole(input, output as usize),
    }
}

/// Decode the frames of `data[span]`, and say which bits of each made which
/// bytes: a step for each frame header, each block header and each block, and
/// one for the checksum at the end of a frame that has one.
///
/// `out` is how much output came before `span`, which is where the steps'
/// bytes count from. Bytes go to `sink` as the decoder lets go of them, and
/// every frame is announced to `on_frame` before it is decoded, with the byte
/// it starts at and how much output comes before it. The skippable frames in
/// front of a frame are counted as part of it. A frame that comes to more than
/// `frame_cap` is [`Refusal::TooLarge`]. Returns how much output there is at
/// the end of `span`.
///
/// The decoder keeps the frame's window, the last bytes it may copy from, and
/// lets go only of what has left it. So for the first window's worth of a
/// frame nothing says where one block's output stopped and the next one's
/// began, and those blocks are one step. From the block that fills the window
/// on, each block's output is what the decoder lets go of after it, and each
/// block is its own step. A frame of no more than its window, which is any
/// frame whose header gives its size in place of a window, is one step.
pub(super) fn zstd_walk(
    data: &[u8],
    span: std::ops::Range<usize>,
    mut out: u64,
    frame_cap: u64,
    b: &mut TraceBuilder,
    on_frame: &mut dyn FnMut(usize, u64),
    sink: &mut dyn FnMut(&[u8]) -> Result<(), Refusal>,
) -> Result<u64, Refusal> {
    use ruzstd::decoding::{BlockDecodingStrategy, FrameDecoder};

    let mut dec = FrameDecoder::new();
    let mut at = span.start;
    while at < span.end {
        let starts = at;
        // A skippable frame: a magic in `0x184d2a50..=0x184d2a5f`, a length,
        // and bytes an application put there that no decoder reads.
        while let Some(size) = skippable(&data[at..span.end]) {
            b.push(at as u64 * 8, out, StepKind::Header(StepField::FrameHeader, 0));
            at += size;
        }
        if at == span.end {
            break;
        }
        on_frame(starts, out);
        let frame = &data[at..span.end];
        let checksum = frame_has_checksum(frame).ok_or(Refusal::Failed)?;
        let mut src = frame;
        dec.init(&mut src).map_err(|_| Refusal::Failed)?;
        let window = window_size(frame, dec.content_size()).ok_or(Refusal::Failed)?;
        b.push(at as u64 * 8, out, StepKind::Header(StepField::FrameHeader, 0));
        let mut read = dec.bytes_read_from_source() as usize;
        // How much of the frame the decoder has let go of, and where the next
        // block's output starts, when that is known.
        let mut collected = 0u64;
        let mut next = Some(out);
        while !dec.is_finished() {
            let block_at = (at + read) as u64 * 8;
            match next {
                Some(_) if b.over_budget() => b.coarsen(),
                Some(o) => {
                    b.push(block_at, o, StepKind::Header(StepField::BlockHeader, 0));
                    // The three-byte block header, and then the block itself.
                    b.push(block_at + 24, o, StepKind::Block);
                }
                None => {}
            }
            dec.decode_blocks(&mut src, BlockDecodingStrategy::UptoBlocks(1)).map_err(|_| Refusal::Failed)?;
            let now = dec.bytes_read_from_source() as usize;
            if now == read {
                return Err(Refusal::Failed);
            }
            read = now;
            if let Some(bytes) = dec.collect().filter(|v| !v.is_empty()) {
                collected += bytes.len() as u64;
                if collected > frame_cap {
                    return Err(Refusal::TooLarge);
                }
                sink(&bytes)?;
            }
            // Once the decoder has let go of anything, what it keeps is
            // exactly one window, so the output so far is that and what it
            // let go of. Before then it is somewhere under one window.
            next = match (dec.is_finished(), collected) {
                (true, _) => Some(out + collected),
                (false, 0) => None,
                (false, _) => Some(out + collected + window),
            };
        }
        out += collected;
        // The decoder counts the frame's checksum as bytes it read, so it is
        // already inside `read`; what is left is to name it.
        if checksum {
            let Some(sum_at) = (at + read).checked_sub(4) else { return Err(Refusal::Failed) };
            b.push(sum_at as u64 * 8, out, StepKind::Header(StepField::Footer, 0));
        }
        at += read;
    }
    Ok(out)
}

/// How many bytes back a block of a zstd frame may copy from, which is what
/// the decoder keeps: the window its header gives, or the frame's own size
/// when the header gives that instead. The same arithmetic `ruzstd` does,
/// since what this says has to be what it keeps.
fn window_size(frame: &[u8], content_size: u64) -> Option<u64> {
    let descriptor = *frame.get(4)?;
    if descriptor & 0x20 != 0 {
        return Some(content_size);
    }
    let w = *frame.get(5)?;
    let base = 1u64 << (10 + u64::from(w >> 3));
    Some(base + base / 8 * u64::from(w & 7))
}

/// How long a skippable frame is, if these bytes start one.
fn skippable(data: &[u8]) -> Option<usize> {
    let magic = u32::from_le_bytes(data.get(..4)?.try_into().ok()?);
    if !(0x184d_2a50..=0x184d_2a5f).contains(&magic) {
        return None;
    }
    let size = u32::from_le_bytes(data.get(4..8)?.try_into().ok()?) as usize;
    size.checked_add(8).filter(|&n| n <= data.len())
}

/// Whether a zstd frame ends with a checksum of what it decoded, which the
/// frame header descriptor's third bit says.
fn frame_has_checksum(data: &[u8]) -> Option<bool> {
    if u32::from_le_bytes(data.get(..4)?.try_into().ok()?) != 0xfd2f_b528 {
        return None;
    }
    Some(data.get(4)? & 0x04 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{Codec, Step};

    /// A trace of a run this module could not shape still tiles it: one step
    /// over the whole of it, which is what "bytes, and nothing said about
    /// them" looks like.
    #[test]
    fn a_run_with_no_shape_is_one_step_over_all_of_it() {
        let t = whole(40, 100);
        t.check_tiles().expect("tiles");
        assert_eq!(
            t.step(0),
            Some(Step { in_bits: 0..320, out_bytes: 0..100, kind: StepKind::Block })
        );
    }

    /// Bytes that are not a stream are refused rather than given a shape.
    #[test]
    fn nonsense_has_no_shape() {
        assert_eq!(zstd(b"not a zstd frame").err(), Some(Refusal::Failed));
        assert_eq!(crate::codec::decode_traced(Codec::Zstd, b"nope").err(), Some(Refusal::Failed));
    }
}
