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
//! [`ZstdWalk`]. Those blocks are one step.
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

/// The largest window a zstd frame may ask for: 128 MiB, which is the most
/// the `zstd` command line decodes without being told to use more memory.
///
/// The decoder keeps a frame's last window of output until the frame ends, and
/// lets go of nothing before then, so the window is how much it may hold that
/// nothing here has seen yet. A frame asking for more is refused as too large
/// before any of it is decoded.
const MAX_WINDOW: u64 = 128 << 20;

/// A zstd run: the bytes, and a step per frame header and per block.
///
/// Every frame of the run, not only the first: `pzstd` writes one frame per
/// few megabytes, each behind a skippable frame saying how long it is, and a
/// file made by joining two `.zst` files is two frames. See [`ZstdWalk`].
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

/// Decode the frames of `data[span]` in one go: a [`ZstdWalk`] that does not
/// stop until the end of `span`. Returns how much output there is there.
pub(super) fn zstd_walk(
    data: &[u8],
    span: std::ops::Range<usize>,
    out: u64,
    frame_cap: u64,
    b: &mut TraceBuilder,
    on_frame: &mut dyn FnMut(usize, u64),
    sink: &mut dyn FnMut(&[u8]) -> Result<(), Refusal>,
) -> Result<u64, Refusal> {
    ZstdWalk::new(span, out, frame_cap).run(data, u64::MAX, usize::MAX, b, on_frame, sink)?.ok_or(Refusal::Failed)
}

/// The most output one zstd block may make: 128 KiB, or the frame's window
/// when that is smaller.
const BLOCK_MAX: u64 = 128 << 10;

/// A walk over the frames of `data[span]`, which decodes them and says which
/// bits of each made which bytes: a step for each frame header, each block
/// header and each block, and one for the checksum at the end of a frame that
/// has one. It can stop between two blocks and carry on from there, which is
/// what the index pass does a step at a time; [`zstd_walk`] is one that does
/// not stop.
///
/// A walk stopped inside a frame keeps that frame's decoder. `ruzstd` keeps
/// everything a frame needs between two blocks in the decoder, the window
/// included, and is handed the input afresh for each block, so carrying on
/// later decodes exactly what carrying on at once would have.
///
/// Bytes go to the sink as the decoder lets go of them, and every frame is
/// announced before it is decoded, with the byte it starts at and how much
/// output comes before it. The skippable frames in front of a frame are
/// counted as part of it. A frame that comes to more than `frame_cap`, or asks
/// for a window larger than [`MAX_WINDOW`], is [`Refusal::TooLarge`]. A frame
/// whose checksum does not match what it decoded is [`Refusal::Failed`].
///
/// The decoder keeps the frame's window, the last bytes it may copy from, and
/// lets go only of what has left it. So for the first window's worth of a
/// frame nothing says where one block's output stopped and the next one's
/// began, and those blocks are one step. From the block that fills the window
/// on, each block's output is what the decoder lets go of after it, and each
/// block is its own step. A frame of no more than its window, which is any
/// frame whose header gives its size in place of a window, is one step.
pub(super) struct ZstdWalk {
    end: usize,
    /// Where the next frame, or the skippable frames in front of it, start,
    /// while no frame is part of the way through. A walk that stopped among
    /// those skippable frames is at the next one.
    at: usize,
    /// Where the skippable frames in front of the next frame start, for a
    /// walk that stopped among them: the next frame is announced as starting
    /// there, as it would be by a walk that did not stop.
    front: Option<usize>,
    /// How much output comes before `at`, or before the frame part of the way
    /// through. Where the steps' bytes count from.
    out: u64,
    frame_cap: u64,
    frame: Option<Frame>,
}

/// A frame decoded part of the way, between two blocks.
struct Frame {
    dec: ruzstd::decoding::FrameDecoder,
    /// Where the frame's own magic is, after any skippable frames.
    at: usize,
    /// How many bytes of the frame the decoder has read.
    read: usize,
    /// How much of the frame's output the decoder has let go of.
    collected: u64,
    /// Where the next block's output starts, when that is known.
    next: Option<u64>,
    window: u64,
    checksum: bool,
}

impl ZstdWalk {
    /// A walk over `span`, with `out` bytes of output before it.
    pub(super) fn new(span: std::ops::Range<usize>, out: u64, frame_cap: u64) -> ZstdWalk {
        ZstdWalk { end: span.end, at: span.start, front: None, out, frame_cap, frame: None }
    }

    /// How far into the run the decoder has read.
    pub(super) fn reached(&self) -> usize {
        self.frame.as_ref().map_or(self.at, |f| f.at + f.read)
    }

    /// How much output the decoder has let go of so far, which is what has
    /// gone to the sink.
    pub(super) fn let_go(&self) -> u64 {
        self.out + self.frame.as_ref().map_or(0, |f| f.collected)
    }

    /// How much output has been decoded so far. Between frames, and once a
    /// frame has let go of anything, that is exact, since what the decoder
    /// keeps then is one window. Before that each block counts as the most
    /// one may make, up to the window.
    fn decoded(out: u64, frame: Option<&Frame>) -> u64 {
        match frame {
            None => out,
            Some(f) if f.collected > 0 => out + f.collected + f.window,
            Some(f) => out + f.window.min(f.dec.blocks_decoded() as u64 * BLOCK_MAX),
        }
    }

    /// Decode on to the end of the span, or until about `share` more bytes of
    /// output have been decoded, and stop between two blocks or two frames.
    /// The output at the end of the span, or nothing when the walk stopped
    /// first and is to be carried on.
    ///
    /// The walk stops at the first block boundary past its share, so it goes
    /// past it by at most one block, 128 KiB. Until a frame's output first
    /// passes its window the decoder does not say how much it holds, so there
    /// a block counts as 128 KiB, which is what a full block makes. A walk
    /// over smaller blocks there stops early, and the next one decodes the
    /// difference as well as its own share.
    ///
    /// The walk also stops at the first place past `reach` more bytes of the
    /// run, between two blocks, two frames or two skippable frames, so that a
    /// walk over frames that make no output still stops. A block is at most
    /// 128 KiB of the run, so the walk goes past `reach` by at most that.
    pub(super) fn run(
        &mut self,
        data: &[u8],
        share: u64,
        reach: usize,
        b: &mut TraceBuilder,
        on_frame: &mut dyn FnMut(usize, u64),
        sink: &mut dyn FnMut(&[u8]) -> Result<(), Refusal>,
    ) -> Result<Option<u64>, Refusal> {
        use ruzstd::decoding::BlockDecodingStrategy;

        let until = ZstdWalk::decoded(self.out, self.frame.as_ref()).saturating_add(share.max(1));
        let read_until = self.reached().saturating_add(reach.max(1));
        loop {
            let f = match &mut self.frame {
                Some(f) => f,
                None if self.out >= until || self.at >= read_until => return Ok(None),
                None => {
                    let starts = *self.front.get_or_insert(self.at);
                    // A skippable frame: a magic in `0x184d2a50..=0x184d2a5f`,
                    // a length, and bytes an application put there that no
                    // decoder reads.
                    while let Some(size) = skippable(&data[self.at..self.end]) {
                        if self.at >= read_until {
                            return Ok(None);
                        }
                        b.push(self.at as u64 * 8, self.out, StepKind::Header(StepField::FrameHeader, 0));
                        self.at += size;
                    }
                    self.front = None;
                    if self.at == self.end {
                        return Ok(Some(self.out));
                    }
                    on_frame(starts, self.out);
                    let frame = self.start_frame(data)?;
                    b.push(self.at as u64 * 8, self.out, StepKind::Header(StepField::FrameHeader, 0));
                    self.frame.insert(frame)
                }
            };
            while !f.dec.is_finished() {
                if ZstdWalk::decoded(self.out, Some(&*f)) >= until || f.at + f.read >= read_until {
                    return Ok(None);
                }
                let block_at = (f.at + f.read) as u64 * 8;
                match f.next {
                    Some(_) if b.over_budget() => b.coarsen(),
                    Some(o) => {
                        b.push(block_at, o, StepKind::Header(StepField::BlockHeader, 0));
                        // The three-byte block header, and then the block itself.
                        b.push(block_at + 24, o, StepKind::Block);
                    }
                    None => {}
                }
                let mut src = &data[f.at + f.read..self.end];
                f.dec.decode_blocks(&mut src, BlockDecodingStrategy::UptoBlocks(1)).map_err(|_| Refusal::Failed)?;
                let now = f.dec.bytes_read_from_source() as usize;
                if now == f.read {
                    return Err(Refusal::Failed);
                }
                f.read = now;
                if let Some(bytes) = f.dec.collect().filter(|v| !v.is_empty()) {
                    f.collected += bytes.len() as u64;
                    if f.collected > self.frame_cap {
                        return Err(Refusal::TooLarge);
                    }
                    sink(&bytes)?;
                }
                // Once the decoder has let go of anything, what it keeps is
                // exactly one window, so the output so far is that and what it
                // let go of. Before then it is somewhere under one window.
                f.next = match (f.dec.is_finished(), f.collected) {
                    (true, _) => Some(self.out + f.collected),
                    (false, 0) => None,
                    (false, _) => Some(self.out + f.collected + f.window),
                };
            }
            // The last `collect` let go of the rest of the frame, so the sum
            // the decoder took is over all of it.
            if f.checksum && f.dec.get_calculated_checksum() != f.dec.get_checksum_from_data() {
                return Err(Refusal::Failed);
            }
            self.out += f.collected;
            // The decoder counts the frame's checksum as bytes it read, so it
            // is already inside `read`; what is left is to name it.
            if f.checksum {
                let Some(sum_at) = (f.at + f.read).checked_sub(4) else { return Err(Refusal::Failed) };
                b.push(sum_at as u64 * 8, self.out, StepKind::Header(StepField::Footer, 0));
            }
            self.at = f.at + f.read;
            self.frame = None;
        }
    }

    /// Read the header of the frame at `self.at` and set up its decoder.
    fn start_frame(&self, data: &[u8]) -> Result<Frame, Refusal> {
        let frame = &data[self.at..self.end];
        let checksum = frame_has_checksum(frame).ok_or(Refusal::Failed)?;
        let mut src = frame;
        // A decoder of its own for every frame. `ruzstd` refuses a window
        // over 100 MiB, and reserves the whole window, only when a decoder is
        // used for a second frame, so one decoder for every frame would read
        // the first of two `zstd --long` files joined end to end and refuse
        // the second.
        let mut dec = ruzstd::decoding::FrameDecoder::new();
        dec.init(&mut src).map_err(|_| Refusal::Failed)?;
        let window = window_size(frame, dec.content_size()).ok_or(Refusal::Failed)?;
        if window > MAX_WINDOW {
            return Err(Refusal::TooLarge);
        }
        let read = dec.bytes_read_from_source() as usize;
        Ok(Frame { dec, at: self.at, read, collected: 0, next: Some(self.out), window, checksum })
    }
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

    /// A frame of one raw block whose header asks for a 128 MiB window, which
    /// is what `zstd --long` writes when it does not know the input's size.
    fn long_window_frame(content: &[u8]) -> Vec<u8> {
        let mut f = vec![0x28, 0xb5, 0x2f, 0xfd, 0x00, (27 - 10) << 3];
        // The last block, raw, and how long it is.
        let header = 1 | (content.len() as u32) << 3;
        f.extend_from_slice(&header.to_le_bytes()[..3]);
        f.extend_from_slice(content);
        f
    }

    /// Every frame of a run is read, the second as well as the first, and
    /// each has a step for its header. Two `zstd --long` files joined end to
    /// end are the case that once lost its second frame.
    #[test]
    fn every_frame_of_a_run_is_read() {
        let mut run = long_window_frame(b"first frame, ");
        run.extend(long_window_frame(b"second frame"));
        let (bytes, trace) = zstd(&run).expect("reads");
        assert_eq!(bytes, b"first frame, second frame");
        trace.check_tiles().expect("tiles");
        let headers = trace.steps().filter(|s| s.kind == StepKind::Header(StepField::FrameHeader, 0)).count();
        assert_eq!(headers, 2);
    }

    /// A frame asking for a window past 128 MiB is refused before any of it
    /// is decoded, whatever it holds.
    #[test]
    fn a_frame_asking_for_too_large_a_window_is_refused() {
        let mut huge = long_window_frame(b"tiny");
        huge[5] = (40 - 10) << 3;
        assert_eq!(zstd(&huge).err(), Some(Refusal::TooLarge));
    }
}
