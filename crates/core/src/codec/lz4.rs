//! One LZ4 block, read a sequence at a time and writing down what it read.
//!
//! An LZ4 block is nothing but sequences, and a sequence is nothing but a
//! token, a run of literals, and a match: no headers, no tables, no bit
//! packing. Which makes it the format where a trace is worth the most per line
//! of decoder, and where the map is exact down to the byte.
//!
//! The last sequence has no match: it is literals and then the block ends,
//! which is how a block that does not divide evenly finishes.

use crate::codec::{BlockKind, Refusal, StepField, StepKind, Trace, TraceBuilder, CAP_BYTES};

/// The last five bytes of a block are always literals, and a match may not
/// come nearer than twelve bytes to the end. Not checked here: a file nobody
/// vouched for may say anything, and refusing it for a rule about how it was
/// written would refuse blocks that decode.
const MIN_MATCH: u32 = 4;

/// One LZ4 block: the bytes it comes to and what was read where.
pub fn block(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    let mut b = TraceBuilder::default();
    let mut out: Vec<u8> = Vec::new();
    let mut at = 0usize;
    let mut coarse = false;
    // One block, holding every sequence. LZ4 has no block structure of its
    // own inside a block, and a trace with no blocks in it has no rows for the
    // reader to open.
    b.open_block(0, 0);
    while at < data.len() {
        if !coarse && b.over_budget() {
            coarse = true;
            b.coarsen();
            b.push(at as u64 * 8, out.len() as u64, StepKind::Opaque);
        }
        let token = data[at];
        if !coarse {
            b.push(at as u64 * 8, out.len() as u64, StepKind::Header(StepField::Token, token as u32));
        }
        at += 1;
        // The literal run's length: the token's high nibble, and if that is
        // full, bytes of 255 until one is not.
        let mut lit = (token >> 4) as usize;
        if lit == 15 {
            let start = at;
            lit += extra(data, &mut at)?;
            if !coarse {
                b.push(start as u64 * 8, out.len() as u64, StepKind::Header(StepField::LengthExtra, lit as u32));
            }
        }
        if at + lit > data.len() {
            return Err(Refusal::Failed);
        }
        if out.len() + lit > CAP_BYTES {
            return Err(Refusal::TooLarge);
        }
        if lit > 0 {
            if !coarse {
                b.push(at as u64 * 8, out.len() as u64, StepKind::Stored);
            }
            out.extend_from_slice(&data[at..at + lit]);
            at += lit;
        }
        // The block may end here, and the last sequence always does.
        if at == data.len() {
            break;
        }
        if at + 2 > data.len() {
            return Err(Refusal::Failed);
        }
        let offset = u16::from_le_bytes([data[at], data[at + 1]]);
        if !coarse {
            b.push(at as u64 * 8, out.len() as u64, StepKind::Header(StepField::Offset, offset as u32));
        }
        at += 2;
        if offset == 0 || offset as usize > out.len() {
            return Err(Refusal::Failed);
        }
        let match_in = at;
        let mut len = (token & 0x0f) as u32 + MIN_MATCH;
        if token & 0x0f == 15 {
            let more = extra(data, &mut at)?;
            len = len.checked_add(u32::try_from(more).map_err(|_| Refusal::Failed)?).ok_or(Refusal::Failed)?;
        }
        if out.len() + len as usize > CAP_BYTES {
            return Err(Refusal::TooLarge);
        }
        let from = out.len() - offset as usize;
        let was = out.len();
        // Overlapping copies are the ordinary case: an offset of one fills.
        for k in 0..len as usize {
            let byte = out[from + k];
            out.push(byte);
        }
        if !coarse {
            b.push(match_in as u64 * 8, was as u64, StepKind::Match { len, dist: offset as u32 });
        }
    }
    b.close_block(data.len() as u64 * 8, out.len() as u64, BlockKind::Sequences, true);
    b.finish_at(data.len() as u64 * 8, out.len() as u64);
    Ok((out, b.done()))
}

/// A length that did not fit its nibble: bytes of 255 and then the remainder.
fn extra(data: &[u8], at: &mut usize) -> Result<usize, Refusal> {
    let mut total = 0usize;
    loop {
        let &byte = data.get(*at).ok_or(Refusal::Failed)?;
        *at += 1;
        total = total.checked_add(byte as usize).ok_or(Refusal::Failed)?;
        if byte != 255 {
            return Ok(total);
        }
        if total > CAP_BYTES {
            return Err(Refusal::TooLarge);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bytes and the map together: what came out has to be what `lz4_flex`
    /// says came out, and the steps have to account for every byte both ways.
    fn agrees(data: &[u8]) {
        let packed = lz4_flex::block::compress(data);
        let (ours, trace) = block(&packed).expect("we read it");
        let theirs = lz4_flex::block::decompress(&packed, data.len()).expect("lz4_flex reads it");
        assert_eq!(ours, theirs, "the bytes differ from lz4_flex's");
        assert_eq!(ours, data);
        trace.check_tiles().expect("the trace tiles");
        assert_eq!(trace.in_bits(), packed.len() as u64 * 8);
        assert_eq!(trace.out_bytes(), data.len() as u64);
    }

    #[test]
    fn a_block_reads_as_what_went_in() {
        agrees(b"");
        agrees(b"hello");
        agrees(b"hello hello hello hello hello lz4");
        agrees(&[0u8; 100_000]);
        agrees(&"the quick brown fox. ".repeat(4000).into_bytes());
    }

    #[test]
    fn every_shape_of_data() {
        let mut seed = 0x9e37_79b9u64;
        let mut rand = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let noise: Vec<u8> = (0..90_000).map(|_| rand() as u8).collect();
        let runs: Vec<u8> = (0..60_000u32).map(|i| (i / 900) as u8).collect();
        agrees(&noise);
        agrees(&runs);
        for n in [1, 2, 5, 12, 13, 14, 15, 16, 17, 254, 255, 256, 270, 4096] {
            agrees(&noise[..n]);
            agrees(&runs[..n]);
        }
    }

    /// A token, a literal run, an offset and a match, each named and placed.
    #[test]
    fn a_sequence_is_a_token_then_literals_then_a_match() {
        let source = b"abcabcabc".repeat(30);
        let packed = lz4_flex::block::compress(&source);
        let (out, trace) = block(&packed).expect("reads");
        assert_eq!(out, source);
        trace.check_tiles().expect("tiles");
        let kinds: Vec<_> = trace.steps().map(|s| s.kind).collect();
        assert!(matches!(kinds[0], StepKind::Header(StepField::Token, _)));
        assert!(kinds.contains(&StepKind::Stored));
        // Three bytes of `abc` repeated, so the match reads three bytes back.
        assert!(kinds.contains(&StepKind::Header(StepField::Offset, 3)));
        assert!(kinds.iter().any(|k| matches!(k, StepKind::Match { .. })));
        // The first bytes came through as literals; a later one was copied.
        assert_eq!(trace.map_out(0).unwrap().kind, StepKind::Stored);
        assert!(matches!(trace.map_out(60).unwrap().kind, StepKind::Match { .. }));
        let step = trace.map_out(60).unwrap();
        assert_eq!(trace.map_in(step.in_bits.start).map(|s| s.kind), Some(step.kind));
    }

    #[test]
    fn broken_blocks_are_refused_rather_than_panicking() {
        // A token asking for more literals than there are.
        assert!(block(&[0xf0]).is_err());
        // A match before there is anything to copy from.
        assert!(block(&[0x0f, 0x01, 0x00]).is_err());
        // An offset of zero, which no encoder writes and no decoder can use.
        assert!(block(&[0x10, b'a', 0x00, 0x00]).is_err());
        let packed = lz4_flex::block::compress(b"a block cut short somewhere in the middle of it");
        for n in 0..packed.len() {
            let _ = block(&packed[..n]);
        }
    }
}
