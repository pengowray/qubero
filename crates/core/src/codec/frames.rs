//! zstd and xz, traced at the block rather than at the symbol.
//!
//! Both keep the crate that reads them. What is written here is only the map:
//! how the run divides into blocks, which bytes of it each block is, and how
//! much of the output each one produced. Inside a block there is nothing but
//! bytes this round, which is an honest stop rather than a missing feature:
//! a zstd block is FSE and Huffman over three interleaved streams, and an xz
//! block is a range coder whose state is the whole of the block before it.
//!
//! xz needs no decoding at all for its map: the index at the end of a stream
//! lists every block's compressed and uncompressed size, which is what an xz
//! reader seeking into a file uses and what a reader looking at one wants.
//!
//! When the headers do not read the way this expects, the run still opens: the
//! trace becomes one step over all of it. A map nobody can draw is better than
//! a stream nobody can open.

use crate::codec::{Refusal, StepField, StepKind, Trace, TraceBuilder};

/// A trace of one step over the whole run, for a decoder that gave the bytes
/// but not the shape.
fn whole(input: usize, output: usize) -> Trace {
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

/// An xz stream: the bytes, and a step per block, taken from the index.
///
/// Nothing is decoded twice. The index at the end of the stream says, for
/// every block in it, how many bytes it takes and how many it comes to, which
/// is all a map at this granularity needs.
pub fn xz(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    let out = super::xz(data)?;
    let trace = xz_trace(data, out.len()).filter(|t| t.check_tiles().is_ok());
    Ok(match trace {
        Some(t) => (out, t),
        None => {
            let n = out.len();
            (out, whole(data.len(), n))
        }
    })
}

/// The blocks of an xz stream, read out of its index.
fn xz_trace(data: &[u8], total_out: usize) -> Option<Trace> {
    // Header: six bytes of magic, two of flags, four of CRC32.
    if data.len() < 12 + 12 || data.get(..6)? != b"\xfd7zXZ\x00" {
        return None;
    }
    // Footer: a CRC32, the size of the index in units of four bytes less one,
    // the flags again, and `YZ`.
    let footer = data.len() - 12;
    if data.get(data.len() - 2..)? != b"YZ" {
        return None;
    }
    let backward = u32::from_le_bytes(data.get(footer + 4..footer + 8)?.try_into().ok()?);
    let index_len = (backward as usize).checked_add(1)?.checked_mul(4)?;
    let index_at = footer.checked_sub(index_len)?;
    if index_at < 12 {
        return None;
    }

    let index = &data[index_at..footer];
    let mut i = 0usize;
    if *index.first()? != 0x00 {
        return None;
    }
    i += 1;
    let count = vli(index, &mut i)?;
    if count > u32::MAX as u64 {
        return None;
    }

    let mut b = TraceBuilder::default();
    b.push(0, 0, StepKind::Header(StepField::FrameHeader, 0));
    let mut at = 12usize;
    let mut produced = 0u64;
    for _ in 0..count {
        let unpadded = vli(index, &mut i)? as usize;
        let uncompressed = vli(index, &mut i)?;
        if unpadded == 0 || at + unpadded > index_at {
            return None;
        }
        b.push(at as u64 * 8, produced, StepKind::Header(StepField::BlockHeader, 0));
        // A block header is one byte of size in units of four, and that many
        // bytes; what is left of the block is the compressed data.
        let head = (data[at] as usize + 1) * 4;
        if head >= unpadded {
            return None;
        }
        b.push((at + head) as u64 * 8, produced, StepKind::Block);
        produced += uncompressed;
        // Every block is padded out to a multiple of four bytes.
        at += unpadded.next_multiple_of(4);
    }
    if at != index_at || produced != total_out as u64 {
        return None;
    }
    // The index and the footer, which say the same thing the blocks did.
    b.push(index_at as u64 * 8, produced, StepKind::Header(StepField::Footer, 0));
    b.finish_at(data.len() as u64 * 8, produced);
    Some(b.done())
}

/// xz's variable-length integer: seven bits a byte, least significant first,
/// the high bit set on every byte but the last. Nine bytes at most.
fn vli(data: &[u8], at: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for shift in 0..9u32 {
        let byte = *data.get(*at)?;
        *at += 1;
        v |= ((byte & 0x7f) as u64) << (shift * 7);
        if byte & 0x80 == 0 {
            return (shift == 0 || byte != 0).then_some(v);
        }
    }
    None
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

    #[test]
    fn a_variable_length_integer_reads_seven_bits_a_byte() {
        let mut at = 0;
        assert_eq!(vli(&[0x00], &mut at), Some(0));
        let mut at = 0;
        assert_eq!(vli(&[0x7f], &mut at), Some(127));
        let mut at = 0;
        assert_eq!(vli(&[0x80, 0x01], &mut at), Some(128));
        // Nine bytes that never end is not a number.
        let mut at = 0;
        assert_eq!(vli(&[0x80; 9], &mut at), None);
    }

    /// Bytes that are not a stream give nothing rather than a wrong shape.
    #[test]
    fn nonsense_has_no_shape() {
        assert!(xz_trace(b"not an xz stream at all, not even close", 10).is_none());
        assert!(zstd_trace(b"not a zstd frame", 10).is_none());
        assert_eq!(crate::codec::decode_traced(Codec::Xz, b"nope").err(), Some(Refusal::Failed));
    }
}
