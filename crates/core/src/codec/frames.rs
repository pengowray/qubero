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
//! When the headers do not read the way this expects, the run still opens: the
//! trace becomes one step over all of it. A map nobody can draw is better than
//! a stream nobody can open.

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

/// A zstd run: the bytes, and a step per frame header and per block.
///
/// The blocks are stepped through one at a time rather than parsed twice: the
/// decoder is the only thing that knows how much output a compressed block
/// produced, so it is asked, between blocks, how far it has got.
pub fn zstd(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    let out = super::zstd(data)?;
    let trace = zstd_trace(data, out.len()).filter(|t| t.check_tiles().is_ok());
    Ok(match trace {
        Some(t) => (out, t),
        None => {
            let n = out.len();
            (out, whole(data.len(), n))
        }
    })
}

/// The shape of a zstd run, or nothing when the headers do not read.
fn zstd_trace(data: &[u8], total_out: usize) -> Option<Trace> {
    use ruzstd::decoding::{BlockDecodingStrategy, FrameDecoder};

    let mut b = TraceBuilder::default();
    let mut at = 0usize;
    let mut produced = 0usize;
    while at < data.len() {
        // A skippable frame: a magic in `0x184d2a50..=0x184d2a5f`, a length,
        // and bytes an application put there that no decoder reads.
        if let Some(size) = skippable(&data[at..]) {
            b.push(at as u64 * 8, produced as u64, StepKind::Header(StepField::FrameHeader, 0));
            at += size;
            continue;
        }
        let mut src = &data[at..];
        let mut dec = FrameDecoder::new();
        dec.init(&mut src).ok()?;
        let checksum = frame_has_checksum(&data[at..])?;
        let header = dec.bytes_read_from_source() as usize;
        b.push(at as u64 * 8, produced as u64, StepKind::Header(StepField::FrameHeader, 0));
        let mut read = header;
        while !dec.is_finished() {
            let block_at = at + read;
            dec.decode_blocks(&mut src, BlockDecodingStrategy::UptoBlocks(1)).ok()?;
            let now = dec.bytes_read_from_source() as usize;
            if now == read {
                return None;
            }
            b.push(block_at as u64 * 8, produced as u64, StepKind::Header(StepField::BlockHeader, 0));
            // The three-byte block header, and then the block itself.
            b.push((block_at + 3) as u64 * 8, produced as u64, StepKind::Block);
            produced += dec.can_collect();
            let _ = dec.collect();
            read = now;
        }
        // The decoder counts the frame's checksum as bytes it read, so it is
        // already inside `read`; what is left is to name it.
        if checksum {
            if read < 4 {
                return None;
            }
            b.push((at + read - 4) as u64 * 8, produced as u64, StepKind::Header(StepField::Footer, 0));
        }
        if read == 0 || at + read > data.len() {
            return None;
        }
        at += read;
    }
    if produced != total_out {
        return None;
    }
    b.finish_at(data.len() as u64 * 8, total_out as u64);
    Some(b.done())
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

    /// Bytes that are not a stream give nothing rather than a wrong shape.
    #[test]
    fn nonsense_has_no_shape() {
        assert!(zstd_trace(b"not a zstd frame", 10).is_none());
        assert_eq!(crate::codec::decode_traced(Codec::Zstd, b"nope").err(), Some(Refusal::Failed));
    }
}
