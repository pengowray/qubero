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
//!
//! What this does owe that check is where to look. A block's check covers what
//! that block unpacked to, and where one block's bytes stop and the next
//! block's begin is not written anywhere in the file: it is the running sum of
//! the index's uncompressed sizes. So both paths below write a
//! [`Member`](crate::codec::Member) per block, which is the one thing that can
//! answer it, and the fallback writes them too: a stream read by the crate has
//! blocks whether or not anybody read inside them.

use crate::codec::{frames, lzma, Refusal, StepField, StepKind, Trace, TraceBuilder, Window, CAP_BYTES, MAX_STEPS};

/// What a stream starts with, and how long its header is: the magic, two
/// bytes of flags, and a CRC32 of the flags.
const MAGIC: &[u8] = b"\xfd7zXZ\x00";
pub(super) const HEADER: usize = 12;

/// The last twelve bytes: a CRC32, the size of the index, the flags again,
/// and these two.
pub(super) const FOOTER: usize = 12;
const FOOTER_MAGIC: &[u8] = b"YZ";

/// The longest a block header can be: its first byte gives its length in
/// units of four, less one.
pub(super) const MAX_BLOCK_HEADER: usize = 1024;

/// The filter that is a compressor. Everything else the format names is a
/// transform in front of it. See `formats::xz`, which lists them all.
const LZMA2: u64 = 0x21;

/// The largest dictionary size code LZMA2's one properties byte can spell.
/// Above it liblzma refuses the stream rather than allocating something, so a
/// block claiming more is one the crate should turn away rather than one this
/// reads with a decoder that would not have noticed.
const MAX_DICT_CODE: u8 = 40;

/// Where one block is and what the file says about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BlockAt {
    /// Its first byte, which is the first byte of its header.
    pub(super) at: usize,
    /// How long that header is, which its first byte gives in units of four.
    header: usize,
    /// The compressed data: what is left of the block once the header and the
    /// check have come off it, before the padding that is not part of either.
    packed: usize,
    /// What the index says it comes to.
    pub(super) unpacked: u64,
}

impl BlockAt {
    /// The block's LZMA2 data, as bytes of the stream: after its header, and
    /// before its padding and its check.
    pub(super) fn lzma2(&self) -> std::ops::Range<usize> {
        let from = self.at + self.header;
        from..from + self.packed
    }
}

/// A stream laid out from its index: every block placed, and the two numbers
/// the blocks are measured against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Layout {
    /// How many bytes the integrity check after every block takes, which the
    /// stream header's check type says and every block obeys.
    pub(super) check: usize,
    pub(super) blocks: Vec<BlockAt>,
    /// Where the index starts, which is where the last block stops.
    pub(super) index_at: usize,
    /// What the index says the whole stream unpacks to.
    pub(super) total_out: u64,
    /// Whether every block's chain is one LZMA2 filter. All of them or none of
    /// them: see the module doc for why this is not asked per block.
    pub(super) plain: bool,
}

impl Layout {
    /// How many bytes of the stream block `i` takes: its header, its
    /// compressed data, its padding and its check, up to where the next
    /// block or the index starts.
    pub(super) fn block_len(&self, i: usize) -> usize {
        self.blocks.get(i + 1).map_or(self.index_at, |b| b.at) - self.blocks[i].at
    }
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
            Err(Refusal::TooLargeOutput) => return Err(Refusal::TooLargeOutput),
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
            Err(Refusal::TooLargeOutput) => return Err(Refusal::TooLargeOutput),
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
        Ok(()) if out.len() > CAP_BYTES => Err(Refusal::TooLargeOutput),
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
        return Err(Refusal::TooLargeOutput);
    }
    let mut b = TraceBuilder::default();
    // The stream header, which the next step's start gives the width of.
    b.push(0, 0, StepKind::Header(StepField::FrameHeader, 0));
    let out = blocks(Window::whole(data), l, 0..l.blocks.len(), 0, budget, &mut b)?;
    if out.len() as u64 != l.total_out {
        return Err(Refusal::Failed);
    }
    push_index(l, &mut b, out.len() as u64);
    b.finish_at(data.len() as u64 * 8, out.len() as u64);
    Ok((out, b.done()))
}

/// Blocks `which` of a stream laid out as `l`, through our own LZMA2, with
/// their steps pushed onto `b` after `before` bytes of the stream's output.
/// What [`unpack`] reads a whole stream with, and what a stream too large to
/// hold whole reads a few of its blocks with: see [`crate::codec::lazy`].
/// The blocks are read from `window`, which has to hold their LZMA2 data;
/// the steps count from the stream's front whatever part of it that is.
///
/// `budget` is how many steps `b` may hold in all, as [`unpack`] says.
pub(super) fn blocks(
    window: Window<'_>,
    l: &Layout,
    which: std::ops::Range<usize>,
    before: u64,
    budget: usize,
    b: &mut TraceBuilder,
) -> Result<Vec<u8>, Refusal> {
    let mut out: Vec<u8> = Vec::new();
    for blk in &l.blocks[which] {
        let from = blk.at + blk.header;
        let end = from + blk.packed;
        let Some(chunk) = window.get(from..end) else { return Err(Refusal::Failed) };
        let here = before + out.len() as u64;
        b.push(blk.at as u64 * 8, here, StepKind::Header(StepField::BlockHeader, blk.header as u32));

        let left = budget.saturating_sub(b.steps());
        let (mut got, t) = lzma::lzma2_within(chunk, left)?;
        // What the index said this block holds. A block that decoded to
        // something else is a reading nobody should be shown, whatever the
        // bytes look like.
        if got.len() as u64 != blk.unpacked {
            return Err(Refusal::Failed);
        }
        b.absorb(&t, from as u64 * 8, here);
        // Which bytes of the stream's output this block is, which is what its
        // own integrity check covers and what nothing else can say: the offset
        // is the running sum of every earlier block, and no field of the file
        // holds it. See `formats::xz`, where the check is declared over this.
        b.member(from as u64 * 8..end as u64 * 8, here..here + got.len() as u64);
        out.append(&mut got);
        let here = before + out.len() as u64;

        // A block is padded out to a multiple of four bytes. The header is
        // already one, so what pads is the compressed data.
        let pad = blk.packed.next_multiple_of(4) - blk.packed;
        if pad > 0 {
            b.push(end as u64 * 8, here, StepKind::Header(StepField::Padding, pad as u32));
        }
        if l.check > 0 {
            b.push((end + pad) as u64 * 8, here, StepKind::Header(StepField::Footer, l.check as u32));
        }
    }
    Ok(out)
}

/// One step over the index and the stream footer, which say again what the
/// blocks did: what a trace reaching the end of the stream ends with.
pub(super) fn push_index(l: &Layout, b: &mut TraceBuilder, out: u64) {
    b.push(l.index_at as u64 * 8, out, StepKind::Header(StepField::Footer, 0));
}

/// Each block's share of the output, the way [`blocks`] declares it, read off
/// the index alone.
pub(super) fn members(l: &Layout) -> Vec<crate::codec::Member> {
    let mut out = 0u64;
    l.blocks
        .iter()
        .map(|blk| {
            let from = (blk.at + blk.header) as u64 * 8;
            let m = crate::codec::Member { in_bits: from..from + blk.packed as u64 * 8, out_bytes: out..out + blk.unpacked };
            out += blk.unpacked;
            m
        })
        .collect()
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
        // The same member the symbol-by-symbol path writes, and it says the
        // same thing here: this block's share of the output, from the index
        // rather than from a decoding of it. The crate produced the bytes and
        // the index says how they divide, and the two are held together by the
        // total being checked above. A block's check is over its own share
        // whichever decoder read the stream, so the declaration in
        // `formats::xz` has to be answerable on this path too.
        b.member(
            (blk.at + blk.header) as u64 * 8..(blk.at + blk.header + blk.packed) as u64 * 8,
            produced..produced + blk.unpacked,
        );
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
///
/// Made of the pieces a stream too large to hold whole is laid out with a
/// step at a time, from only the bytes each piece needs: [`ends`] from the
/// stream's first and last twelve bytes, [`Records`] from the index, and
/// [`block_at`] from each block's header. See [`crate::codec::lazy`].
pub(super) fn layout(data: &[u8]) -> Option<Layout> {
    if data.len() < HEADER + FOOTER {
        return None;
    }
    let ends = ends(&data[..HEADER], &data[data.len() - FOOTER..], data.len())?;
    let index = &data[ends.index_at..ends.footer];
    let mut records = Records::new(index, index.len(), ends.index_at)?;
    let mut blocks = Vec::new();
    let mut plain = true;
    while let Some(r) = records.next(index) {
        let (blk, p) = block_at(&r, &data[r.header_bytes()], ends.check)?;
        plain &= p;
        blocks.push(blk);
    }
    records.filled().then(|| Layout { check: ends.check, blocks, index_at: ends.index_at, total_out: records.total_out(), plain })
}

/// What a stream's header and footer say: how long each block's check is,
/// and where the index is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Ends {
    pub(super) check: usize,
    /// Where the index starts, which is where the last block stops.
    pub(super) index_at: usize,
    /// Where the footer starts, which is where the index stops.
    pub(super) footer: usize,
}

impl Ends {
    /// How many bytes the index takes.
    pub(super) fn index_len(&self) -> usize {
        self.footer - self.index_at
    }
}

/// The ends of a stream of `len` bytes, read from its first twelve bytes,
/// `head`, and its last twelve, `foot`. Nothing for a stream this does not
/// lay out.
pub(super) fn ends(head: &[u8], foot: &[u8], len: usize) -> Option<Ends> {
    if len < HEADER + FOOTER || head.len() < HEADER || foot.len() < FOOTER || !head.starts_with(MAGIC) {
        return None;
    }
    // The stream flags: a byte nobody has assigned, then four more bits of the
    // same and the check type. A stream setting any of them was written to a
    // version of the format this does not know, so the blocks under it are not
    // read on the strength of a chain that may not mean what it says.
    if head[6] != 0 || head[7] & 0xf0 != 0 {
        return None;
    }
    let check = check_size(head[7] & 0x0f);

    let footer = len - FOOTER;
    if &foot[FOOTER - 2..FOOTER] != FOOTER_MAGIC {
        return None;
    }
    // The backward size: how long the index is, in units of four bytes and
    // written one short. The one measurement no field in front of it gives.
    let backward = u32::from_le_bytes(foot.get(4..8)?.try_into().ok()?);
    let index_len = (backward as usize).checked_add(1)?.checked_mul(4)?;
    let index_at = footer.checked_sub(index_len)?;
    if index_at < HEADER {
        return None;
    }
    Some(Ends { check, index_at, footer })
}

/// Whether the last bytes of a run, `foot`, end in stream padding: four zero
/// bytes, which the format allows after a stream's footer, in multiples of
/// four. [`layout`] reads a single stream that ends at its footer, so a
/// padded stream is read by the decoder of whole streams.
pub(super) fn padded(foot: &[u8]) -> bool {
    foot.len() >= 4 && foot[foot.len() - 4..].iter().all(|&b| b == 0)
}

/// Whether `bytes` start the way a stream starts. A run of several streams
/// joined end to end ends with the last stream's footer and index, whose
/// blocks start after that stream's own header and not after the run's
/// first twelve bytes, so [`layout`] does not lay it out, and the decoder of
/// whole streams reads it.
pub(super) fn starts_stream(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

/// One record of a stream's index: where its block starts, which the records
/// before it give, and the two sizes the record says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Record {
    pub(super) at: usize,
    /// The block without its padding: header, data and check.
    pub(super) unpadded: usize,
    pub(super) unpacked: u64,
}

impl Record {
    /// The bytes of the stream that hold the block's header: its first
    /// [`MAX_BLOCK_HEADER`] bytes, or all of the block when it is shorter.
    /// [`block_at`] reads them.
    pub(super) fn header_bytes(&self) -> std::ops::Range<usize> {
        self.at..self.at + self.unpadded.min(MAX_BLOCK_HEADER)
    }
}

/// The records of an index, read one at a time from the index's bytes, and
/// where they have placed the blocks so far. Each record is checked as it is
/// read: a block that runs into the index, or a record that does not parse,
/// ends the records there, and [`Records::filled`] says no.
#[derive(Debug, Clone, Copy)]
pub(super) struct Records {
    /// The byte of the index the next record starts at.
    i: usize,
    /// How many records the index says it has, and how many are still to
    /// read.
    count: u64,
    left: u64,
    /// Where the next block starts.
    at: usize,
    index_at: usize,
    total_out: u64,
    broken: bool,
}

/// The most bytes one record of an index takes: two variable-length
/// integers of at most nine bytes each.
pub(super) const MAX_RECORD: usize = 18;

impl Records {
    /// The records of `index`, the first bytes of a stream's index of
    /// `index_len` bytes, which starts at byte `index_at` of the stream.
    /// `index` has to hold the count, and every record [`Records::next`]
    /// reads: [`Records::bytes_needed`] says how many bytes that is. Nothing
    /// for an index that does not start the way an index does.
    pub(super) fn new(index: &[u8], index_len: usize, index_at: usize) -> Option<Records> {
        if *index.first()? != 0x00 {
            return None;
        }
        let mut i = 1usize;
        let count = vli(index, &mut i)?;
        // A record is two bytes at the very least, so a count larger than the
        // index could hold is not a count.
        if count > index_len as u64 {
            return None;
        }
        Some(Records { i, count, left: count, at: HEADER, index_at, total_out: 0, broken: false })
    }

    /// How many records the index says it has.
    pub(super) fn count(&self) -> u64 {
        self.count
    }

    /// How many bytes from the index's first byte its records can take at
    /// most, which is all of them [`Records::next`] ever reads: the bytes
    /// after them are the index's padding and its CRC32, which nothing here
    /// reads.
    pub(super) fn bytes_needed(&self) -> usize {
        usize::try_from(self.count).unwrap_or(usize::MAX).saturating_mul(MAX_RECORD).saturating_add(self.i)
    }

    /// Where the blocks the records place end, once every record has been
    /// read and none was refused: where the index starts, for a stream the
    /// records fill. Nothing while there are records left, or once one did
    /// not hold together.
    pub(super) fn end(&self) -> Option<usize> {
        (self.left == 0 && !self.broken).then_some(self.at)
    }

    /// The next record of `index`, the bytes these records were made from.
    /// Nothing once they are all read, or once one does not hold together.
    pub(super) fn next(&mut self, index: &[u8]) -> Option<Record> {
        if self.left == 0 || self.broken {
            return None;
        }
        self.left -= 1;
        let record = self.read(index);
        self.broken = record.is_none();
        record
    }

    fn read(&mut self, index: &[u8]) -> Option<Record> {
        let unpadded = vli(index, &mut self.i)? as usize;
        let unpacked = vli(index, &mut self.i)?;
        if unpadded == 0 || self.at.checked_add(unpadded)? > self.index_at {
            return None;
        }
        self.total_out = self.total_out.checked_add(unpacked)?;
        let record = Record { at: self.at, unpadded, unpacked };
        self.at += unpadded.next_multiple_of(4);
        Some(record)
    }

    /// Whether every record has been read and the blocks they place fill
    /// the space between the stream header and the index exactly. Anything
    /// else means the index is describing a different file from the one in
    /// front of it. No blocks at all is a stream of nothing, which is what
    /// packing an empty file writes and is laid out like any other: an index
    /// saying zero, and zero bytes of output to agree with it.
    pub(super) fn filled(&self) -> bool {
        self.left == 0 && !self.broken && self.at == self.index_at
    }

    /// What the records read so far say their blocks come to.
    pub(super) fn total_out(&self) -> u64 {
        self.total_out
    }
}

/// The block record `r` places, read from `head`, the stream's bytes
/// [`Record::header_bytes`] names, and whether its chain is one LZMA2 filter
/// and nothing else. `check` is how long each block's check is. Nothing for
/// a block this does not lay out.
pub(super) fn block_at(r: &Record, head: &[u8], check: usize) -> Option<(BlockAt, bool)> {
    let header = (usize::from(*head.first()?) + 1) * 4;
    let packed = r.unpadded.checked_sub(header)?.checked_sub(check)?;
    // A block with no data in it is not a block. Even an empty file packs
    // to an LZMA2 stream of one byte.
    if packed == 0 {
        return None;
    }
    let plain = head.get(..header).is_some_and(|h| plain_lzma2(h, packed, r.unpacked));
    Some((BlockAt { at: r.at, header, packed, unpacked: r.unpacked }, plain))
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
///
/// `header` is the header's bytes, as long as its first byte says.
fn plain_lzma2(header: &[u8], packed: usize, unpacked: u64) -> bool {
    // Everything in the header but its own CRC32, which is the template's to
    // check and is not read here.
    let Some(body) = header.len().checked_sub(4).map(|n| &header[..n]) else {
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

// Reachable from `formats::xz`'s own tests, which need a stream that really is
// one: a template's tests can build a header, and only something that packs
// LZMA2 can build a block whose check has anything to be right about.
#[cfg(test)]
pub(crate) mod tests;
