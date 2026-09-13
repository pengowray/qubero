//! Snappy, the raw block with no framing round it, read a tag at a time.
//!
//! What Parquet means by SNAPPY, and what every other format that says the
//! word without saying "framed" means too: a varint giving how many bytes come
//! out, and then tags to the end of the run. There is no magic, no stream
//! identifier, no checksum and no chunking; those belong to the *framing*
//! format, which is a different thing built on top of this one and is not what
//! a page payload holds.
//!
//! A tag is one byte whose low two bits say which of four things it is. Zero
//! is a run of literals, and the other three are a copy from further back in
//! what has already been written, differing only in how wide the offset is.
//! So the shape is LZ4's shape, and the trace is LZ4's trace: a token, then
//! either bytes that came through as they were or a length and a distance.
//!
//! Two details are worth writing down because they are where a decoder written
//! from memory goes wrong. A literal's length is *one more* than the number in
//! the tag's top six bits, and when that number is 60 or above it is not a
//! length at all: it says how many bytes of little-endian length follow, as
//! 60 meaning one byte through 63 meaning four. And a one-byte copy takes the
//! top three bits of its own tag as the high bits of the offset, which is the
//! only place in the format where a field is split across two bytes that are
//! not next to each other in the number they make.
//!
//! Copies may overlap what they read, and usually do: an offset of one is how
//! a run of the same byte is written. The copy is a byte at a time for that
//! reason, the way [`crate::codec::lz4`] does it.
//!
//! The preamble says how long the output will be. It is checked against what
//! came out rather than trusted: a decoder that allocates what a file claims
//! is a decoder a file can point at a gigabyte, and a run that does not come
//! to what it promised is a run that has gone wrong somewhere earlier.

use crate::codec::{BlockKind, Refusal, StepField, StepKind, Trace, TraceBuilder, CAP_BYTES};

/// A literal whose tag holds a length at all holds it in the top six bits, so
/// 59 is the largest that fits and 60 upwards count bytes instead.
const LITERAL_INLINE_MAX: u8 = 59;

/// One raw Snappy block: the bytes it comes to and what was read where.
pub fn block(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    let mut b = TraceBuilder::default();
    let mut at = 0usize;
    let declared = varint(data, &mut at)?;
    if declared > CAP_BYTES as u64 {
        return Err(Refusal::TooLarge);
    }
    b.push(0, 0, StepKind::Header(StepField::UnpackedSize, declared.min(u32::MAX as u64) as u32));
    let mut out: Vec<u8> = Vec::with_capacity(declared as usize);
    let mut coarse = false;
    // One block, holding every tag. Snappy has no block structure inside a
    // block, and a trace with no blocks in it has no rows for the reader to
    // open. The preamble is outside it: it is the run's header, not a symbol.
    b.open_block(at as u64 * 8, 0);
    while at < data.len() {
        if !coarse && b.over_budget() {
            coarse = true;
            b.coarsen();
            b.push(at as u64 * 8, out.len() as u64, StepKind::Opaque);
        }
        let tag = data[at];
        if !coarse {
            b.push(at as u64 * 8, out.len() as u64, StepKind::Header(StepField::Token, tag as u32));
        }
        at += 1;
        match tag & 3 {
            0 => {
                // The length is one more than the top six bits, unless those
                // bits are 60 or above, in which case they count the bytes of
                // length that follow.
                let head = tag >> 2;
                let len = if head <= LITERAL_INLINE_MAX {
                    head as usize + 1
                } else {
                    let width = head as usize - 59;
                    let start = at;
                    if at + width > data.len() {
                        return Err(Refusal::Failed);
                    }
                    let mut n = 0u64;
                    for (i, &byte) in data[at..at + width].iter().enumerate() {
                        n |= (byte as u64) << (8 * i);
                    }
                    at += width;
                    let len = n.checked_add(1).ok_or(Refusal::Failed)?;
                    if len > CAP_BYTES as u64 {
                        return Err(Refusal::TooLarge);
                    }
                    if !coarse {
                        b.push(start as u64 * 8, out.len() as u64, StepKind::Header(StepField::LengthExtra, len as u32));
                    }
                    len as usize
                };
                if at + len > data.len() {
                    return Err(Refusal::Failed);
                }
                if out.len() + len > CAP_BYTES {
                    return Err(Refusal::TooLarge);
                }
                if !coarse {
                    b.push(at as u64 * 8, out.len() as u64, StepKind::Stored);
                }
                out.extend_from_slice(&data[at..at + len]);
                at += len;
            }
            kind => {
                // A one-byte copy splits its offset: the top three bits of the
                // tag are the high bits of an eleven-bit number, and its
                // length is four larger than the three bits in between. The
                // wider two take their length from the whole top six bits and
                // their offset from the bytes that follow, whole.
                let start = at;
                let (len, offset) = match kind {
                    1 => {
                        let &low = data.get(at).ok_or(Refusal::Failed)?;
                        at += 1;
                        ((tag >> 2 & 7) as u32 + 4, ((tag as u32 >> 5) << 8) | low as u32)
                    }
                    2 => {
                        let bytes = data.get(at..at + 2).ok_or(Refusal::Failed)?;
                        at += 2;
                        ((tag >> 2) as u32 + 1, u16::from_le_bytes([bytes[0], bytes[1]]) as u32)
                    }
                    _ => {
                        let bytes = data.get(at..at + 4).ok_or(Refusal::Failed)?;
                        at += 4;
                        ((tag >> 2) as u32 + 1, u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
                    }
                };
                if !coarse {
                    b.push(start as u64 * 8, out.len() as u64, StepKind::Header(StepField::Offset, offset));
                }
                if offset == 0 || offset as usize > out.len() {
                    return Err(Refusal::Failed);
                }
                if out.len() + len as usize > CAP_BYTES {
                    return Err(Refusal::TooLarge);
                }
                let from = out.len() - offset as usize;
                let was = out.len();
                // Overlapping copies are the ordinary case: an offset of one
                // fills, and is how Snappy writes a run of one byte.
                for k in 0..len as usize {
                    let byte = out[from + k];
                    out.push(byte);
                }
                if !coarse {
                    b.push(at as u64 * 8, was as u64, StepKind::Match { len, dist: offset });
                }
            }
        }
    }
    // What the preamble promised against what the tags produced. A run that
    // does not come to its own declared length has been cut short or has been
    // read as the wrong thing, and either way the bytes are not the bytes.
    if out.len() as u64 != declared {
        return Err(Refusal::Failed);
    }
    b.close_block(data.len() as u64 * 8, out.len() as u64, BlockKind::Sequences, true);
    b.finish_at(data.len() as u64 * 8, out.len() as u64);
    Ok((out, b.done()))
}

/// The preamble: seven bits of length a byte, low group first, and the top bit
/// set on every byte but the last. Five bytes is as far as a 32-bit length
/// reaches, which is as long as a Snappy block may be.
fn varint(data: &[u8], at: &mut usize) -> Result<u64, Refusal> {
    let mut n = 0u64;
    for i in 0..5 {
        let &byte = data.get(*at).ok_or(Refusal::Failed)?;
        *at += 1;
        n |= ((byte & 0x7f) as u64) << (7 * i);
        if byte & 0x80 == 0 {
            return Ok(n);
        }
    }
    Err(Refusal::Failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bytes and the map together: what came out has to be what `snap`
    /// says came out, and the steps have to account for every byte both ways.
    fn agrees(data: &[u8]) {
        let packed = snap::raw::Encoder::new().compress_vec(data).expect("snap packs it");
        let (ours, trace) = block(&packed).expect("we read it");
        let theirs = snap::raw::Decoder::new().decompress_vec(&packed).expect("snap reads it");
        assert_eq!(ours, theirs, "the bytes differ from snap's");
        assert_eq!(ours, data);
        trace.check_tiles().expect("the trace tiles");
        assert_eq!(trace.in_bits(), packed.len() as u64 * 8);
        assert_eq!(trace.out_bytes(), data.len() as u64);
    }

    #[test]
    fn a_block_reads_as_what_went_in() {
        agrees(b"");
        agrees(b"hello");
        agrees(b"hello hello hello hello hello snappy");
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
        let runs: Vec<u8> = (0..90_000u32).map(|i| (i / 900) as u8).collect();
        agrees(&noise);
        agrees(&runs);
        // 59 and 60 are where a literal's length stops fitting its tag, and
        // 255 and 256 are where its byte count grows; every one of those is a
        // different arm of the reader above.
        for n in [1, 2, 5, 59, 60, 61, 62, 63, 64, 255, 256, 257, 65_535, 65_536] {
            agrees(&noise[..n]);
            agrees(&runs[..n]);
        }
    }

    /// A tag, a literal run, an offset and a match, each named and placed.
    #[test]
    fn a_tag_is_a_token_then_literals_or_a_copy() {
        let source = b"abcabcabc".repeat(30);
        let packed = snap::raw::Encoder::new().compress_vec(&source).expect("packs");
        let (out, trace) = block(&packed).expect("reads");
        assert_eq!(out, source);
        trace.check_tiles().expect("tiles");
        let kinds: Vec<_> = trace.steps().map(|s| s.kind).collect();
        // The preamble first, saying how many bytes the run comes to.
        assert_eq!(kinds[0], StepKind::Header(StepField::UnpackedSize, source.len() as u32));
        assert!(matches!(kinds[1], StepKind::Header(StepField::Token, _)));
        assert!(kinds.contains(&StepKind::Stored));
        assert!(kinds.iter().any(|k| matches!(k, StepKind::Header(StepField::Offset, _))));
        assert!(kinds.iter().any(|k| matches!(k, StepKind::Match { .. })));
        // The first bytes came through as literals; a later one was copied.
        assert_eq!(trace.map_out(0).unwrap().kind, StepKind::Stored);
        assert!(matches!(trace.map_out(60).unwrap().kind, StepKind::Match { .. }));
    }

    /// A run of one byte is written as a copy that reads what it is writing,
    /// which is the arm a decoder that copies in bulk gets wrong.
    #[test]
    fn an_overlapping_copy_fills() {
        agrees(&[0x5a; 4000]);
        agrees(&b"ab".repeat(2000));
    }

    #[test]
    fn a_run_that_does_not_come_to_its_preamble_is_refused() {
        // Says twenty bytes come out; writes five.
        let packed = [20u8, (4 << 2), b'h', b'e', b'l', b'l', b'o'];
        assert_eq!(block(&packed).err(), Some(Refusal::Failed));
    }

    #[test]
    fn a_copy_reaching_before_the_start_is_refused() {
        // Says one byte comes out, then copies from one byte back before
        // anything has been written.
        let packed = [1u8, 1 | (0 << 2), 1];
        assert_eq!(block(&packed).err(), Some(Refusal::Failed));
    }
}
