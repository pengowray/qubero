//! One LZ4 block, read a sequence at a time and writing down what it read.
//!
//! An LZ4 block is nothing but sequences, and a sequence is nothing but a
//! token, a run of literals, and a match: no headers, no tables, no bit
//! packing. Which makes it the format where a trace is worth the most per line
//! of decoder, and where the map is exact down to the byte.
//!
//! The last sequence has no match: it is literals and then the block ends,
//! which is how a block that does not divide evenly finishes.
//!
//! **Frames.** What a `.lz4` file holds, and what Arrow compresses each buffer
//! into, is the frame format around those blocks: a magic number, a
//! descriptor, blocks each behind a four-byte size, a size of zero to end on,
//! and checksums wherever the descriptor asked for them. See [`frame`]. The
//! `lz4` template lays that out as fields and opens each block on its own,
//! which is the better reading of a file that is nothing but a frame; this is
//! for a frame somewhere nothing lays fields over it, and for the one thing a
//! block at a time cannot do, which is a frame whose blocks are linked.

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
    // One block, holding every sequence. LZ4 has no block structure of its
    // own inside a block, and a trace with no blocks in it has no rows for the
    // reader to open.
    b.open_block(0, 0);
    sequences(data, 0, &mut out, 0, &mut b)?;
    b.close_block(data.len() as u64 * 8, out.len() as u64, BlockKind::Sequences, true);
    b.finish_at(data.len() as u64 * 8, out.len() as u64);
    Ok((out, b.done()))
}

/// The sequences of one block, written onto the end of `out` and recorded in
/// `b` at where they are in the whole run: `base` is the byte of the run the
/// block's first byte is.
///
/// `floor` is the first byte of `out` a match may copy from. A block read on
/// its own, or an independent block of a frame, starts where it starts and a
/// match reaching before that is a broken block; a linked block of a frame may
/// reach back into the blocks before it, as far as the frame's own first byte.
fn sequences(data: &[u8], base: usize, out: &mut Vec<u8>, floor: usize, b: &mut TraceBuilder) -> Result<(), Refusal> {
    let mut at = 0usize;
    let mut coarse = false;
    let bit = |at: usize| (base + at) as u64 * 8;
    while at < data.len() {
        if !coarse && b.over_budget() {
            coarse = true;
            b.coarsen();
            b.push(bit(at), out.len() as u64, StepKind::Opaque);
        }
        let token = data[at];
        if !coarse {
            b.push(bit(at), out.len() as u64, StepKind::Header(StepField::Token, token as u32));
        }
        at += 1;
        // The literal run's length: the token's high nibble, and if that is
        // full, bytes of 255 until one is not.
        let mut lit = (token >> 4) as usize;
        if lit == 15 {
            let start = at;
            lit += extra(data, &mut at)?;
            if !coarse {
                b.push(bit(start), out.len() as u64, StepKind::Header(StepField::LengthExtra, lit as u32));
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
                b.push(bit(at), out.len() as u64, StepKind::Stored);
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
            b.push(bit(at), out.len() as u64, StepKind::Header(StepField::Offset, offset as u32));
        }
        at += 2;
        if offset == 0 || offset as usize > out.len() - floor {
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
            b.push(bit(match_in), was as u64, StepKind::Match { len, dist: offset as u32 });
        }
    }
    Ok(())
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

/// What a frame starts with, as the little-endian number it is.
const FRAME_MAGIC: u32 = 0x184d_2204;

/// The first of the sixteen magics a skippable frame may have. The low nibble
/// is the application's to choose and means nothing to a decoder.
const SKIPPABLE_MAGIC: u32 = 0x184d_2a50;

/// The top bit of a block's size word, set when the block was stored as it
/// came because compressing it made it larger.
const STORED_BIT: u32 = 1 << 31;

/// One or more LZ4 frames, one after another: the bytes they come to, and a
/// step for every part of every frame.
///
/// **What a frame is.** Four bytes of magic, then a descriptor: a flags byte,
/// a byte giving the largest block the encoder would write, the content size
/// when the flags say one is there, a dictionary id when they say that, and a
/// byte of checksum over the descriptor. Then blocks, each a four-byte size
/// and that many bytes, the size's top bit saying the bytes are stored rather
/// than compressed, and each followed by a four-byte checksum when the flags
/// ask for one. A size of zero is the end mark. After it, a checksum of
/// everything the frame came to, when the flags ask for that too.
///
/// **Linked blocks.** The flags say whether each block stands on its own. When
/// it does not, a match may reach back past the start of its block into the
/// ones before it, up to the 64 KiB an offset can say, which is why this reads
/// every block onto one output rather than handing each to [`block`]: a linked
/// block read on its own is a block whose first match has nothing to copy.
/// Nothing reaches back past the frame's own start either way, since the frame
/// is where a decoder starts again.
///
/// **What the trace says.** The magic and descriptor are one
/// [`StepField::FrameHeader`] step, each size word a
/// [`StepField::BlockHeader`] saying the word it holds (so the end mark says
/// nought), and each checksum a [`StepField::BlockChecksum`] or
/// [`StepField::ContentChecksum`] saying the word written there. A compressed
/// block's sequences are traced as [`block`] traces them, and a stored block
/// is one [`StepKind::Stored`] step. Each block, from its size word to the end
/// of its data, is a [`Block`](crate::codec::Block) of the trace; its checksum
/// comes after it, outside, as the frame's own header and end mark do.
///
/// **What is not checked.** The checksums are named and not computed. They
/// are xxHash-32, which nothing else here needs, and the zstd frames beside
/// these are not checked either; a frame with a wrong checksum reads as the
/// bytes its blocks say. A content size that disagrees with what the blocks
/// came to is refused, since that is the frame contradicting itself rather
/// than a sum left unverified.
///
/// A skippable frame is stepped over as one header step, the way zstd's are. A
/// legacy frame, the format of the first releases with a magic of its own, is
/// refused: it has no descriptor and no end mark, and nothing that embeds LZ4
/// frames writes one. So is a dictionary this cannot have been handed, when a
/// match reaches into it; a frame that names a dictionary and never uses it
/// reads.
pub fn frame(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    let mut b = TraceBuilder::default();
    let mut out: Vec<u8> = Vec::new();
    let mut at = 0usize;
    let word = |at: usize| -> Result<u32, Refusal> {
        let bytes = data.get(at..at + 4).ok_or(Refusal::Failed)?;
        Ok(u32::from_le_bytes(bytes.try_into().expect("four bytes")))
    };
    if data.is_empty() {
        return Err(Refusal::Failed);
    }
    while at < data.len() {
        let magic = word(at)?;
        if magic & !0xf == SKIPPABLE_MAGIC {
            let size = word(at + 4)? as usize;
            let end = (at + 8).checked_add(size).filter(|e| *e <= data.len()).ok_or(Refusal::Failed)?;
            b.push(at as u64 * 8, out.len() as u64, StepKind::Header(StepField::FrameHeader, 0));
            at = end;
            continue;
        }
        if magic != FRAME_MAGIC {
            return Err(Refusal::Failed);
        }
        let (&flags, &bd) = (data.get(at + 4).ok_or(Refusal::Failed)?, data.get(at + 5).ok_or(Refusal::Failed)?);
        // Version 01 is the only one there has been; the reserved bits are
        // there to be zero, and a decoder that finds them set is told the
        // frame is from a version it does not know.
        if flags >> 6 != 1 || flags & 0x02 != 0 || bd & 0x8f != 0 {
            return Err(Refusal::Failed);
        }
        let independent = flags & 0x20 != 0;
        let block_checksums = flags & 0x10 != 0;
        let has_size = flags & 0x08 != 0;
        let content_checksum = flags & 0x04 != 0;
        let has_dictionary = flags & 0x01 != 0;
        // 64 KiB, 256 KiB, 1 MiB or 4 MiB; below four is not a size.
        let largest = match bd >> 4 {
            n @ 4..=7 => 1usize << (8 + 2 * n),
            _ => return Err(Refusal::Failed),
        };
        let mut descriptor = at + 6;
        let content_size = match has_size {
            true => {
                let bytes = data.get(descriptor..descriptor + 8).ok_or(Refusal::Failed)?;
                descriptor += 8;
                Some(u64::from_le_bytes(bytes.try_into().expect("eight bytes")))
            }
            false => None,
        };
        if has_dictionary {
            descriptor += 4;
        }
        // The descriptor's checksum byte.
        descriptor += 1;
        if descriptor > data.len() {
            return Err(Refusal::Failed);
        }
        b.push(at as u64 * 8, out.len() as u64, StepKind::Header(StepField::FrameHeader, 0));
        at = descriptor;
        let start = out.len();
        loop {
            let size_word = word(at)?;
            if size_word == 0 {
                b.push(at as u64 * 8, out.len() as u64, StepKind::Header(StepField::BlockHeader, 0));
                at += 4;
                break;
            }
            b.open_block(at as u64 * 8, out.len() as u64);
            b.push(at as u64 * 8, out.len() as u64, StepKind::Header(StepField::BlockHeader, size_word));
            at += 4;
            let stored = size_word & STORED_BIT != 0;
            let size = (size_word & !STORED_BIT) as usize;
            if size > largest || at + size > data.len() {
                return Err(Refusal::Failed);
            }
            let body = &data[at..at + size];
            if stored {
                if out.len() + size > CAP_BYTES {
                    return Err(Refusal::TooLarge);
                }
                if size > 0 {
                    b.push(at as u64 * 8, out.len() as u64, StepKind::Stored);
                }
                out.extend_from_slice(body);
            } else {
                let floor = if independent { out.len() } else { start };
                sequences(body, at, &mut out, floor, &mut b)?;
            }
            at += size;
            let kind = if stored { BlockKind::Stored } else { BlockKind::Sequences };
            // The last block is the one the end mark follows, which nothing
            // in the block itself says. Closed before its checksum, which is
            // a word about the block rather than one of its sequences.
            let mark = at + if block_checksums { 4 } else { 0 };
            b.close_block(at as u64 * 8, out.len() as u64, kind, word(mark).is_ok_and(|w| w == 0));
            if block_checksums {
                let sum = word(at)?;
                b.push(at as u64 * 8, out.len() as u64, StepKind::Header(StepField::BlockChecksum, sum));
                at += 4;
            }
        }
        if content_checksum {
            let sum = word(at)?;
            b.push(at as u64 * 8, out.len() as u64, StepKind::Header(StepField::ContentChecksum, sum));
            at += 4;
        }
        if content_size.is_some_and(|n| n != (out.len() - start) as u64) {
            return Err(Refusal::Failed);
        }
    }
    b.finish_at(data.len() as u64 * 8, out.len() as u64);
    Ok((out, b.done()))
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

    use crate::codec::Step;
    use lz4_flex::frame::{BlockMode, BlockSize, FrameDecoder, FrameEncoder, FrameInfo};

    /// A frame written by `lz4_flex`'s own encoder with the settings given.
    fn framed(data: &[u8], info: FrameInfo) -> Vec<u8> {
        let mut w = FrameEncoder::with_frame_info(info, Vec::new());
        std::io::Write::write_all(&mut w, data).expect("packs");
        w.finish().expect("finishes")
    }

    /// A frame read here against the same frame read by `lz4_flex`, with the
    /// trace accounting for every byte of both.
    fn frame_agrees(data: &[u8], info: FrameInfo) -> Trace {
        let packed = framed(data, info);
        let (ours, trace) = frame(&packed).expect("we read it");
        let mut theirs = Vec::new();
        std::io::Read::read_to_end(&mut FrameDecoder::new(&packed[..]), &mut theirs).expect("lz4_flex reads it");
        assert_eq!(ours, theirs, "the bytes differ from lz4_flex's");
        assert_eq!(ours, data);
        trace.check_tiles().expect("the trace tiles");
        assert_eq!(trace.in_bits(), packed.len() as u64 * 8);
        assert_eq!(trace.out_bytes(), data.len() as u64);
        trace
    }

    /// Text that repeats across far more than one 64 KiB block, so a linked
    /// block has something in the block before it worth reaching for.
    fn long_text() -> Vec<u8> {
        (0..30_000u32).flat_map(|i| format!("row {} of the table, value {}; ", i % 97, i % 13).into_bytes()).collect()
    }

    #[test]
    fn a_frame_reads_as_what_went_in_whatever_its_flags_say() {
        let text = long_text();
        let noise: Vec<u8> = {
            let mut seed = 0x2545_f491u64;
            (0..200_000).map(|_| { seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17; seed as u8 }).collect()
        };
        for data in [&b""[..], b"hello lz4 frame", &text, &noise] {
            for mode in [BlockMode::Independent, BlockMode::Linked] {
                for (blocks, content) in [(false, false), (true, false), (false, true), (true, true)] {
                    let info = FrameInfo::new()
                        .block_size(BlockSize::Max64KB)
                        .block_mode(mode)
                        .block_checksums(blocks)
                        .content_checksum(content)
                        .content_size(Some(data.len() as u64));
                    frame_agrees(data, info);
                }
            }
        }
    }

    /// Linked blocks are the reason this reads a whole frame at once. Each one
    /// read on its own is refused, because its first match reaches into the
    /// block before it; the frame read whole gives the text back.
    #[test]
    fn a_linked_block_copies_from_the_block_before_it() {
        let text = long_text();
        let packed = framed(&text, FrameInfo::new().block_size(BlockSize::Max64KB).block_mode(BlockMode::Linked));
        let trace = frame_agrees(&text, FrameInfo::new().block_size(BlockSize::Max64KB).block_mode(BlockMode::Linked));
        assert!(trace.blocks().len() > 3, "several blocks: {}", trace.blocks().len());
        let second = &trace.blocks()[1];
        let body = (second.in_bits.start / 8 + 4) as usize..(second.in_bits.end / 8) as usize;
        assert!(block(&packed[body]).is_err(), "the second block leans on the first");
        // And a match in it that reaches back past its own first byte.
        let reaches_back = trace.steps().any(|s| match s.kind {
            StepKind::Match { dist, .. } => s.out_bytes.start >= second.out_bytes.start
                && s.out_bytes.start < second.out_bytes.end
                && s.out_bytes.start - (dist as u64) < second.out_bytes.start,
            _ => false,
        });
        assert!(reaches_back);
    }

    /// The frame's parts, each where it is: the descriptor, a size word, the
    /// sequences, a block checksum, the end mark and the content checksum.
    #[test]
    fn every_part_of_a_frame_is_a_step() {
        let text = b"a frame of one block, one block of a frame. ".repeat(40);
        let info = FrameInfo::new().block_checksums(true).content_checksum(true).content_size(Some(text.len() as u64));
        let packed = framed(&text, info.clone());
        let trace = frame_agrees(&text, info);
        let steps: Vec<Step> = trace.steps().collect();
        // Magic, flags, block size, eight bytes of content size, checksum.
        assert_eq!(steps[0].kind, StepKind::Header(StepField::FrameHeader, 0));
        assert_eq!(steps[0].in_bits, 0..15 * 8);
        assert!(matches!(steps[1].kind, StepKind::Header(StepField::BlockHeader, w) if w & STORED_BIT == 0));
        assert!(matches!(steps[2].kind, StepKind::Header(StepField::Token, _)));
        let n = steps.len();
        let word = |at: usize| u32::from_le_bytes(packed[at..at + 4].try_into().unwrap());
        let (end, block_sum) = (packed.len() - 8, packed.len() - 12);
        assert_eq!(steps[n - 3].kind, StepKind::Header(StepField::BlockChecksum, word(block_sum)));
        assert_eq!(steps[n - 2].kind, StepKind::Header(StepField::BlockHeader, 0));
        assert_eq!(steps[n - 2].in_bits, end as u64 * 8..(end as u64 + 4) * 8);
        assert_eq!(steps[n - 1].kind, StepKind::Header(StepField::ContentChecksum, word(packed.len() - 4)));
        assert_eq!(steps[n - 1].in_bits, (packed.len() as u64 - 4) * 8..packed.len() as u64 * 8);
        // The block runs from its size word to the end of its data, and its
        // checksum is outside it.
        assert_eq!(trace.blocks().len(), 1);
        assert!(trace.blocks()[0].last);
        assert_eq!(trace.blocks()[0].in_bits, steps[1].in_bits.start..block_sum as u64 * 8);
    }

    /// Noise does not compress, so the encoder stores its blocks, and a stored
    /// block is one step of bytes that came through as they were.
    #[test]
    fn a_block_that_did_not_compress_is_stored() {
        let mut seed = 7u64;
        let noise: Vec<u8> = (0..1000).map(|_| { seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1); (seed >> 33) as u8 }).collect();
        let trace = frame_agrees(&noise, FrameInfo::new());
        assert_eq!(trace.blocks()[0].kind, BlockKind::Stored);
        assert!(trace.steps().any(|s| s.kind == StepKind::Stored && s.out_bytes == (0..1000)));
    }

    /// Two frames end to end are one stream, with a skippable frame between
    /// them that nothing reads.
    #[test]
    fn frames_follow_one_another_and_a_skippable_one_is_stepped_over() {
        let mut v = framed(b"the first frame", FrameInfo::new());
        v.extend_from_slice(&(SKIPPABLE_MAGIC + 3).to_le_bytes());
        v.extend_from_slice(&5u32.to_le_bytes());
        v.extend_from_slice(b"notes");
        v.extend_from_slice(&framed(b", and the second", FrameInfo::new()));
        let (out, trace) = frame(&v).expect("reads");
        assert_eq!(out, b"the first frame, and the second");
        trace.check_tiles().expect("tiles");
    }

    #[test]
    fn broken_frames_are_refused_rather_than_panicking() {
        assert!(frame(b"").is_err());
        assert!(frame(b"not a frame at all").is_err());
        // A content size the blocks do not come to.
        let text = long_text();
        let mut packed = framed(&text[..1000], FrameInfo::new().content_size(Some(1000)));
        packed[6] = 0xe7;
        assert_eq!(frame(&packed).err(), Some(Refusal::Failed));
        // A legacy frame, which is not this format.
        assert!(frame(&[0x02, 0x21, 0x4c, 0x18, 0, 0, 0, 0]).is_err());
        let packed = framed(&text, FrameInfo::new().block_size(BlockSize::Max64KB).block_mode(BlockMode::Linked).block_checksums(true));
        for n in (0..packed.len()).step_by(97) {
            let _ = frame(&packed[..n]);
        }
        assert!(frame(&packed[..packed.len() - 1]).is_err());
    }
}
