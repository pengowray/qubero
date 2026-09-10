//! xz, read down to the symbols where the file lets it be.
//!
//! An xz stream is a twelve-byte header, some blocks, an index of the blocks,
//! and a footer. The index is what makes the shape free: it lists, for every
//! block, how many bytes the block takes and how many it comes to, so the
//! blocks can be walked without decoding any of them. That is what a reader
//! seeking into a `.xz` uses, and it was for a while all this module said.
//!
//! It is not all a reader wants. A block holds a filter chain, and the last
//! filter of a chain is nearly always LZMA2, which is the one shape of LZMA
//! that carries its own settings; [`crate::codec::lzma::lzma2`] already reads
//! one to its literals and its matches for 7z and lzip. So when a stream's
//! blocks hold LZMA2 and nothing else, the blocks are read here, the per-block
//! traces are stitched into one over the whole stream, and clicking a byte of
//! the text lands on the symbol that wrote it. That is the same thing deflate
//! has done all along, and the reason it is worth the parse below.
//!
//! What forces the other path is a chain with a transform in it: delta, or one
//! of the branch filters, or a filter identifier nobody here knows. Those run
//! over what LZMA2 produced, so reading only the LZMA2 would give bytes that
//! are not the file's bytes, and the failure would be silent: a delta filter
//! does not change how long the data is, so the index would agree with the
//! wrong answer. Such a stream goes to `lzma-rs` whole, and its trace is the
//! block map from the index.
//!
//! The fallback is all-or-nothing across the stream, not per block. Two
//! reasons. The bytes of a stream are one run of output, so a stream whose
//! second block needs a filter this cannot run has no first half worth showing
//! on its own; and the decision has to be the same for the bytes and for the
//! trace, or the two doors of [`crate::codec`] would answer differently. So
//! every block's chain is read before any block is decoded, and one chain this
//! cannot run sends the whole stream to the crate.
//!
//! Nothing here verifies a block's integrity check. That is the template's
//! job and `formats::xz` does it: this reads a chain to decide what decoder to
//! use, and does not re-check what has already been checked elsewhere.

use crate::codec::{frames, lzma, Refusal, StepField, StepKind, Trace, TraceBuilder, CAP_BYTES, MAX_STEPS};

/// What a stream starts with, and how long its header is: the magic, two
/// bytes of flags, and a CRC32 of the flags.
const MAGIC: &[u8] = b"\xfd7zXZ\x00";
const HEADER: usize = 12;

/// The last twelve bytes: a CRC32, the size of the index, the flags again,
/// and these two.
const FOOTER: usize = 12;
const FOOTER_MAGIC: &[u8] = b"YZ";

/// The filter that is a compressor. Everything else the format names is a
/// transform in front of it. See `formats::xz`, which lists them all.
const LZMA2: u64 = 0x21;

/// The largest dictionary size code LZMA2's one properties byte can spell.
/// Above it liblzma refuses the stream rather than allocating something, so a
/// block claiming more is one the crate should turn away rather than one this
/// reads with a decoder that would not have noticed.
const MAX_DICT_CODE: u8 = 40;

/// Where one block is and what the file says about it.
struct BlockAt {
    /// Its first byte, which is the first byte of its header.
    at: usize,
    /// How long that header is, which its first byte gives in units of four.
    header: usize,
    /// The compressed data: what is left of the block once the header and the
    /// check have come off it, before the padding that is not part of either.
    packed: usize,
    /// What the index says it comes to.
    unpacked: u64,
}

/// A stream laid out from its index: every block placed, and the two numbers
/// the blocks are measured against.
struct Layout {
    /// How many bytes the integrity check after every block takes, which the
    /// stream header's check type says and every block obeys.
    check: usize,
    blocks: Vec<BlockAt>,
    /// Where the index starts, which is where the last block stops.
    index_at: usize,
    /// What the index says the whole stream unpacks to.
    total_out: u64,
    /// Whether every block's chain is one LZMA2 filter. All of them or none of
    /// them: see the module doc for why this is not asked per block.
    plain: bool,
}

/// An xz stream: the bytes, and a trace over them.
///
/// The trace is per symbol when every block is plain LZMA2 and per block when
/// it is not, and a stream this cannot lay out at all gets one step over the
/// whole of it. Every one of those tiles the run, which is what the reader
/// needs; what differs is how much it says.
pub fn stream(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    let plan = layout(data);
    if let Some(l) = plan.as_ref().filter(|l| l.plain) {
        match unpack(data, l, MAX_STEPS) {
            Ok((out, t)) if t.check_tiles().is_ok() => return Ok((out, t)),
            // Our own bytes passed every count the index states, so they are
            // the bytes; it is the map that did not hold together, and a map
            // that does not tile is one no listing can draw. The block map
            // stands in for it, and the bytes do not change with it: the two
            // doors of this module have to agree whatever the trace does.
            Ok((out, _)) => {
                let n = out.len();
                return Ok((out, shape(plan.as_ref(), data.len(), n)));
            }
            // Past the cap is past the cap whichever decoder is asked, so
            // there is nothing to fall back to.
            Err(Refusal::TooLarge) => return Err(Refusal::TooLarge),
            Err(_) => {}
        }
    }
    let out = borrowed(data)?;
    let n = out.len();
    Ok((out, shape(plan.as_ref(), data.len(), n)))
}

/// The same bytes with no trace kept.
///
/// The decision and the decoding are the ones [`stream`] makes, so the two
/// cannot answer differently. What is left out is the recording: the trace
/// builder handed to each block is given no budget at all, so LZMA2 gives up
/// naming symbols before it names the first one and the whole stream costs a
/// handful of steps instead of one per byte. Building the map and throwing it
/// away would have been simpler to write and would have made every plain
/// decode pay for a map nobody asked for, which for a stream at the cap is
/// eighty megabytes of steps.
pub fn bytes(data: &[u8]) -> Result<Vec<u8>, Refusal> {
    if let Some(l) = layout(data).filter(|l| l.plain) {
        match unpack(data, &l, 0) {
            Ok((out, _)) => return Ok(out),
            Err(Refusal::TooLarge) => return Err(Refusal::TooLarge),
            Err(_) => {}
        }
    }
    borrowed(data)
}

/// The whole stream through `lzma-rs`, which is the one call to that crate
/// left in this codebase.
///
/// It is what reads a chain this cannot run: a delta or a branch filter over
/// LZMA2, or a filter nobody here names. It gives bytes and says nothing about
/// where they came from, which is why it is the fallback and not the path.
fn borrowed(data: &[u8]) -> Result<Vec<u8>, Refusal> {
    let mut input = std::io::BufReader::new(data);
    let mut out = Vec::new();
    match lzma_rs::xz_decompress(&mut input, &mut out) {
        Ok(()) if out.len() > CAP_BYTES => Err(Refusal::TooLarge),
        Ok(()) => Ok(out),
        Err(_) => Err(Refusal::Failed),
    }
}

/// A map of the blocks and nothing inside them, or one step over the run when
/// even that could not be drawn.
fn shape(plan: Option<&Layout>, len: usize, total_out: usize) -> Trace {
    plan.and_then(|l| block_trace(l, len, total_out))
        .filter(|t| t.check_tiles().is_ok())
        .unwrap_or_else(|| frames::whole(len, total_out))
}

/// Every block through our own LZMA2, stitched into one run and one trace.
///
/// `budget` is how many steps the trace may hold in all. It is spent across
/// the blocks rather than per block: each block is given what is left, so a
/// stream of a hundred blocks cannot record a hundred times the cap. A block
/// that runs out gives up naming its symbols, which is what a trace says with
/// [`Trace::coarse`], and the stitched trace carries that up.
///
/// Two things are checked against the index before the bytes are handed back,
/// because a trace that disagrees with the file is worse than no trace: each
/// block has to produce what its own index record says, and the whole has to
/// come to what the records add up to.
fn unpack(data: &[u8], l: &Layout, budget: usize) -> Result<(Vec<u8>, Trace), Refusal> {
    if l.total_out > CAP_BYTES as u64 {
        return Err(Refusal::TooLarge);
    }
    let mut out: Vec<u8> = Vec::with_capacity(l.total_out as usize);
    let mut b = TraceBuilder::default();
    // The stream header, which the next step's start gives the width of.
    b.push(0, 0, StepKind::Header(StepField::FrameHeader, 0));

    for blk in &l.blocks {
        let from = blk.at + blk.header;
        let end = from + blk.packed;
        let Some(chunk) = data.get(from..end) else { return Err(Refusal::Failed) };
        b.push(blk.at as u64 * 8, out.len() as u64, StepKind::Header(StepField::BlockHeader, blk.header as u32));

        let left = budget.saturating_sub(b.steps());
        let (mut got, t) = lzma::lzma2_within(chunk, left)?;
        // What the index said this block holds. A block that decoded to
        // something else is a reading nobody should be shown, whatever the
        // bytes look like.
        if got.len() as u64 != blk.unpacked {
            return Err(Refusal::Failed);
        }
        b.absorb(&t, from as u64 * 8, out.len() as u64);
        out.append(&mut got);

        // A block is padded out to a multiple of four bytes. The header is
        // already one, so what pads is the compressed data.
        let pad = blk.packed.next_multiple_of(4) - blk.packed;
        if pad > 0 {
            b.push(end as u64 * 8, out.len() as u64, StepKind::Header(StepField::Padding, pad as u32));
        }
        if l.check > 0 {
            b.push((end + pad) as u64 * 8, out.len() as u64, StepKind::Header(StepField::Footer, l.check as u32));
        }
    }

    if out.len() as u64 != l.total_out {
        return Err(Refusal::Failed);
    }
    // The index and the stream footer, which say again what the blocks did.
    b.push(l.index_at as u64 * 8, out.len() as u64, StepKind::Header(StepField::Footer, 0));
    b.finish_at(data.len() as u64 * 8, out.len() as u64);
    Ok((out, b.done()))
}

/// The blocks of a stream and nothing inside them: a step for the header, one
/// per block, and one for the index and footer together.
///
/// What the reader gets when a chain sent the stream to `lzma-rs`, and what
/// this module gave for every stream before it read any of them itself.
fn block_trace(l: &Layout, len: usize, total_out: usize) -> Option<Trace> {
    if l.total_out != total_out as u64 {
        return None;
    }
    let mut b = TraceBuilder::default();
    b.push(0, 0, StepKind::Header(StepField::FrameHeader, 0));
    let mut produced = 0u64;
    for blk in &l.blocks {
        b.push(blk.at as u64 * 8, produced, StepKind::Header(StepField::BlockHeader, blk.header as u32));
        // The data, its padding and its check as one step: nothing here read
        // them, so nothing here may tell them apart.
        b.push((blk.at + blk.header) as u64 * 8, produced, StepKind::Block);
        produced += blk.unpacked;
    }
    b.push(l.index_at as u64 * 8, produced, StepKind::Header(StepField::Footer, 0));
    b.finish_at(len as u64 * 8, produced);
    Some(b.done())
}

/// Where every block of a stream is, read from the index at the end of it and
/// from the blocks' own headers.
///
/// One stream, not several. Streams may be concatenated, and a file of two of
/// them has a footer in the middle that this does not go looking for; such a
/// file lays out as nothing at all and is opened by the crate, which is what
/// happened to it before this module read any block itself.
fn layout(data: &[u8]) -> Option<Layout> {
    if data.len() < HEADER + FOOTER || !data.starts_with(MAGIC) {
        return None;
    }
    // The stream flags: a byte nobody has assigned, then four more bits of the
    // same and the check type. A stream setting any of them was written to a
    // version of the format this does not know, so the blocks under it are not
    // read on the strength of a chain that may not mean what it says.
    if data[6] != 0 || data[7] & 0xf0 != 0 {
        return None;
    }
    let check = check_size(data[7] & 0x0f);

    let footer = data.len() - FOOTER;
    if &data[data.len() - 2..] != FOOTER_MAGIC {
        return None;
    }
    // The backward size: how long the index is, in units of four bytes and
    // written one short. The one measurement no field in front of it gives.
    let backward = u32::from_le_bytes(data.get(footer + 4..footer + 8)?.try_into().ok()?);
    let index_len = (backward as usize).checked_add(1)?.checked_mul(4)?;
    let index_at = footer.checked_sub(index_len)?;
    if index_at < HEADER {
        return None;
    }

    let index = &data[index_at..footer];
    let mut i = 0usize;
    if *index.first()? != 0x00 {
        return None;
    }
    i += 1;
    let count = vli(index, &mut i)?;
    // A record is two bytes at the very least, so a count larger than the
    // index could hold is not a count.
    if count > index.len() as u64 {
        return None;
    }

    let mut blocks = Vec::new();
    let mut at = HEADER;
    let mut total_out = 0u64;
    let mut plain = true;
    for _ in 0..count {
        // The block without its padding: header, data and check.
        let unpadded = vli(index, &mut i)? as usize;
        let unpacked = vli(index, &mut i)?;
        if unpadded == 0 || at.checked_add(unpadded)? > index_at {
            return None;
        }
        let header = (usize::from(*data.get(at)?) + 1) * 4;
        let packed = unpadded.checked_sub(header)?.checked_sub(check)?;
        // A block with no data in it is not a block. Even an empty file packs
        // to an LZMA2 stream of one byte.
        if packed == 0 {
            return None;
        }
        plain &= plain_lzma2(data, at, header, packed, unpacked);
        blocks.push(BlockAt { at, header, packed, unpacked });
        total_out = total_out.checked_add(unpacked)?;
        at += unpadded.next_multiple_of(4);
    }
    // The blocks have to fill the space between the header and the index
    // exactly. Anything else means the index is describing a different file
    // from the one in front of it. No blocks at all is a stream of nothing,
    // which is what packing an empty file writes and is laid out like any
    // other: an index saying zero, and zero bytes of output to agree with it.
    if at != index_at {
        return None;
    }
    Some(Layout { check, blocks, index_at, total_out, plain })
}

/// How many bytes the check after every block takes, from the four bits that
/// name it. All sixteen IDs have a length whether or not they have a name: 0
/// is no check, and the rest run in groups of three that double from four.
/// The same arithmetic `formats::xz` does, for the same reason.
fn check_size(id: u8) -> usize {
    if id == 0 {
        0
    } else {
        4 << ((id - 1) / 3)
    }
}

/// Whether a block's filter chain is one LZMA2 filter and nothing else.
///
/// Strict on purpose, and every clause below earns its place. The chain is the
/// only thing that says whether reading the LZMA2 alone gives the file's
/// bytes, and a wrong yes here is not caught by anything downstream: a delta
/// filter leaves the length alone, so a stream read without it comes to
/// exactly what the index says and is wrong in every byte.
///
/// So a header is taken as plain only when it says so completely: no reserved
/// flag set, one filter, that filter LZMA2 with the one properties byte the
/// filter has, a dictionary size code the format can spell, whichever sizes it
/// wrote agreeing with the index, and zeroes in what is left. Anything else,
/// including a header this cannot walk to the end of, is somebody else's to
/// read.
fn plain_lzma2(data: &[u8], at: usize, header: usize, packed: usize, unpacked: u64) -> bool {
    // Everything in the header but its own CRC32, which is the template's to
    // check and is not read here.
    let Some(body) = data.get(at..at + header).and_then(|h| h.get(..header.checked_sub(4)?)) else {
        return false;
    };
    let Some(&flags) = body.get(1) else { return false };
    // Four bits nobody has assigned, and two saying how many filters there
    // are less one. More than one filter is a chain with a transform in it.
    if flags & 0x3c != 0 || flags & 0x03 != 0 {
        return false;
    }

    let mut i = 2usize;
    // The two sizes the encoder may write. `xz -T1` writes neither and the
    // threaded encoder writes both; either way they have to say what the
    // index already said.
    if flags & 0x40 != 0 && vli(body, &mut i) != Some(packed as u64) {
        return false;
    }
    if flags & 0x80 != 0 && vli(body, &mut i) != Some(unpacked) {
        return false;
    }

    if vli(body, &mut i) != Some(LZMA2) || vli(body, &mut i) != Some(1) {
        return false;
    }
    let Some(&props) = body.get(i) else { return false };
    i += 1;
    // Two bits nobody has used, and six holding a dictionary size code. Our
    // own decoder allocates whatever the stream turns out to need and would
    // happily read a code liblzma refuses, so a code past the encoding is
    // handed to the crate rather than read by something that would not have
    // noticed.
    if props & 0xc0 != 0 || props & 0x3f > MAX_DICT_CODE {
        return false;
    }
    // Zeroes out to the length the header's first byte gave.
    body[i..].iter().all(|&byte| byte == 0)
}

/// xz's variable-length integer: seven bits a byte, least significant first,
/// the high bit set on every byte but the last. Nine bytes at most, and no
/// trailing zero byte, since a number has one spelling.
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
mod tests;
