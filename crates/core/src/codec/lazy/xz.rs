//! Indexing an xz stream of several blocks, from the index at its end, a step
//! at a time, and reading its spans.

use std::ops::Range;
use std::sync::{Arc, Mutex};

use super::input::{Copied, ReadError, RunInput};
use super::seekable::{lock, seekable, Kind, Recent};
use super::{whole, Checkpoints, Decoded, Limits, Progress, Start, Stepped};
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
///
/// A stream whose index lists more than [`Limits::block_marks`] blocks is
/// refused as [`Refusal::TooManyXzBlocks`], whatever it unpacks to, before
/// any of its blocks are placed.
///
/// `packed` is the whole stream, held in memory. [`XzPass`] lays out a
/// stream it reads through a handle, a step at a time, and comes to the same
/// index.
pub(super) fn index_xz(packed: Arc<Vec<u8>>, input: RunInput, limits: Limits) -> Result<Decoded, Refusal> {
    if count_of(&packed).is_some_and(|n| n > limits.block_marks as u64) {
        return Err(Refusal::TooManyXzBlocks);
    }
    match xz::layout(&packed).filter(|l| l.total_out > limits.whole as u64) {
        Some(l) => laid_out(l, &input, limits),
        None => whole(Codec::Xz, &packed),
    }
}

/// How many blocks the index of `data`, a whole stream, says the stream has,
/// when its header, its footer and the start of its index hold together.
fn count_of(data: &[u8]) -> Option<u64> {
    if data.len() < xz::HEADER + xz::FOOTER {
        return None;
    }
    let ends = xz::ends(&data[..xz::HEADER], &data[data.len() - xz::FOOTER..], data.len())?;
    xz::Records::new(&data[ends.index_at..ends.footer], ends.index_len(), ends.index_at).map(|r| r.count())
}

/// The index of a stream laid out as `l`, past the cap, which the index reads
/// through `input`: refused when its blocks use a filter, when one comes to
/// more than [`Limits::span`], when the stream comes to more than
/// [`Limits::lazy`], or when one block is more than one read of `input` may
/// take, in that order.
fn laid_out(l: xz::Layout, input: &RunInput, limits: Limits) -> Result<Decoded, Refusal> {
    if !l.plain {
        return Err(Refusal::TooLargeOutput);
    }
    if l.blocks.iter().any(|b| b.unpacked > limits.span as u64) {
        return Err(Refusal::TooLargeUnit(SpanUnit::XzBlock));
    }
    if l.total_out > limits.lazy {
        return Err(Refusal::TooLargeLazy);
    }
    // A read of a span reads a block's data whole, so one too large to read
    // at once is refused.
    if (0..l.blocks.len()).any(|i| input.too_much(l.block_len(i), limits.unit_input)) {
        return Err(Refusal::TooMuchAtOnce);
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
    Ok(Decoded::Indexed(seekable(Codec::Xz, input.clone(), Kind::Xz(l), len, marks.list, limits)))
}

/// How many block headers one step of an [`XzPass`] reads at most. The
/// chunks those headers are missing are all named in one answer of
/// [`Stepped::NeedsInput`], so in the browser a stream of a thousand blocks
/// spread over 300 MiB loads its headers in about 20 rounds, not a thousand.
/// The span of the stream one step's headers cover is also bounded: see
/// [`XzPass`].
const HEADERS_PER_STEP: usize = 64;

/// How many times [`Limits::step_input`] one step reads, at most, when the
/// pass copies the stream whole for the decoder of whole streams. A copy
/// decodes nothing until it is complete, so a step can read more of it than
/// a step of a decode reads, and the copy takes fewer goes.
const COPY_STEPS: usize = 4;

/// An xz stream's index pass, done a step at a time, reading the stream
/// through its [`RunInput`] a piece at a time and holding no copy of it.
///
/// The layout of a stream is read from the end inwards: its header and
/// footer, then the index, whose length the footer gives, then the header of
/// each block, where the index puts it. Nothing between the block headers is
/// read, so a step never names a chunk of the file that lies wholly inside a
/// block's LZMA2 data. What it comes to is what [`index_xz`] comes to over a
/// copy of the same stream:
///
/// 1. The first step reads the stream's header and its footer, the first and
///    last 12 bytes.
/// 2. The index, whose length the footer gives, `step_input` bytes a step,
///    and none of the stream before it: in the browser the index is nearly
///    always in the footer's chunk of the file, which the first step loaded.
///    Once the index's count of blocks is read, only as much of the index as
///    that many records can take is read, and an index that lists more than
///    [`Limits::block_marks`] blocks is refused as
///    [`Refusal::TooManyXzBlocks`], since the blocks it lists are held for as
///    long as the index is kept. A stream whose index says it unpacks to the
///    cap or less, or whose index or footer does not hold together, goes to
///    the decoder of whole streams without its block headers being read, as
///    it would over a copy.
/// 3. The block headers, up to [`HEADERS_PER_STEP`] a step, each read where
///    the index puts it, and the missing chunks of all of them named in one
///    answer. The last header of a step ends no more than
///    [`RunInput::most_at_once`] bytes, half of what the source keeps, past
///    the start of the first, so that their chunks fit in the store
///    together. A header that is read is kept while the step waits for the
///    others, so that a store that cannot hold them all still gets further
///    each round.
/// 4. Once every block is placed, the same checks as over a copy, in the same
///    order (see [`laid_out`]).
///
/// A stream that goes to the decoder of whole streams is copied, a piece of
/// up to [`COPY_STEPS`] times `step_input` a step, and decoded once the copy
/// is complete. That copy is held to [`Limits::copy_input`]. A longer stream
/// is refused for what sent it there (see [`Whole`]): as
/// [`Refusal::TooLargeCopy`] when its size is the reason, and as
/// [`Refusal::Failed`] when part of its layout does not hold together, as
/// the decoder of whole streams refuses a damaged stream.
///
/// A pass over a copy in memory, and one over a stream no longer than one
/// step's piece, lays the stream out in one step from the whole stream, as
/// [`index_xz`] always has.
pub(super) struct XzPass {
    /// Where the pass reads the stream from: `input`, or a copy of it.
    reads: RunInput,
    /// Where the index reads the stream from once it is made.
    input: RunInput,
    limits: Limits,
    phase: Phase,
    /// The furthest bit of the stream the pass has reached, which does not go
    /// back when a stream whose block headers were being read is copied whole
    /// from its front.
    furthest: u64,
}

enum Phase {
    /// Nothing read yet.
    Start,
    /// The stream's ends are read, and its index is being read: `bytes` of it
    /// from its first byte on.
    Index { ends: xz::Ends, bytes: Vec<u8> },
    /// The index's records all read, and the blocks they place end before
    /// the index, as the last of several streams joined end to end would:
    /// whether a stream header is at byte `at`, where that last stream would
    /// start.
    Joined { at: usize },
    /// The index is read and holds together, and its blocks come to more than
    /// the cap: the block headers are being read.
    Headers(Box<Headers>),
    /// The stream is being copied whole for the decoder of whole streams:
    /// `bytes` of it from its first byte on.
    Copy { bytes: Vec<u8> },
    /// Finished, or refused.
    Over,
}

/// Why a stream goes to the decoder of whole streams, which is what a stream
/// too long to copy is refused for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Whole {
    /// Its size: it unpacks to the cap or less, or it is in a form that only
    /// the decoder of whole streams reads, with stream padding after its
    /// footer or joined end to end from several streams. Past the limit on
    /// a copy it is refused as [`Refusal::TooLargeCopy`].
    BySize,
    /// Its header, footer, index or a block header does not hold together.
    /// Past the limit on a copy it is refused as [`Refusal::Failed`], what
    /// the decoder of whole streams says of a stream it cannot read.
    Damaged,
}

/// The block headers of an [`XzPass`] read so far.
struct Headers {
    ends: xz::Ends,
    /// The index's bytes, which `records` reads.
    index: Vec<u8>,
    /// The records whose headers no step has asked for yet.
    records: xz::Records,
    total_out: u64,
    /// The blocks placed so far, and whether every chain among them is one
    /// LZMA2 filter.
    blocks: Vec<xz::BlockAt>,
    plain: bool,
    /// The records the step asked for, each with its block once its header
    /// is read.
    batch: Vec<(xz::Record, Option<(xz::BlockAt, bool)>)>,
}

impl XzPass {
    pub(super) fn new(reads: RunInput, input: RunInput, limits: Limits) -> XzPass {
        XzPass { reads, input, limits, phase: Phase::Start, furthest: 0 }
    }

    /// How far the pass has got: the bits of the stream up to the start of
    /// the furthest block header read, or the bits copied, and all of them
    /// once it is over. Nothing is unpacked, so no output.
    pub(super) fn progress(&self) -> Progress {
        Progress { read_bits: self.furthest, run_bits: self.input.len() * 8, unpacked: 0 }
    }

    /// How many bytes of the stream the pass holds between steps: the index
    /// while it reads it and the block headers, or the copy.
    #[cfg(test)]
    pub(super) fn holds(&self) -> usize {
        match &self.phase {
            Phase::Index { bytes, .. } | Phase::Copy { bytes } => bytes.len(),
            Phase::Headers(h) => h.index.len(),
            Phase::Start | Phase::Joined { .. } | Phase::Over => 0,
        }
    }

    /// Read the next piece of the stream's layout, or of its copy; or finish.
    /// A step whose piece is missing chunks of the file names them and keeps
    /// nothing it read, except the block headers it read. A step decodes
    /// nothing, except the last step of a stream copied whole, so it ignores
    /// the share of output and input its caller offers, which bounds a
    /// deflate or zstd step.
    pub(super) fn step(&mut self) -> Result<Stepped, Refusal> {
        let stepped = self.step_on();
        let at = match &self.phase {
            Phase::Start | Phase::Index { .. } | Phase::Joined { .. } => 0,
            // The headers of the step's batch read so far count, so that a
            // step that waits again having read some of them has got
            // further.
            Phase::Headers(h) => {
                let read = h.batch.iter().filter(|(_, got)| got.is_some()).map(|(r, _)| r.at);
                h.blocks.last().map(|b| b.at).into_iter().chain(read).max().map_or(0, |at| at as u64 * 8)
            }
            Phase::Copy { bytes } => bytes.len() as u64 * 8,
            Phase::Over => self.input.len() * 8,
        };
        self.furthest = self.furthest.max(at);
        stepped
    }

    fn step_on(&mut self) -> Result<Stepped, Refusal> {
        match std::mem::replace(&mut self.phase, Phase::Over) {
            Phase::Start => self.start(),
            Phase::Index { ends, bytes } => self.index(ends, bytes),
            Phase::Joined { at } => self.joined(at),
            Phase::Headers(h) => self.headers(h),
            Phase::Copy { bytes } => self.copy(bytes),
            Phase::Over => Err(Refusal::Failed),
        }
    }

    /// The first step: the whole stream laid out from a copy, or from one
    /// read of a stream no longer than a piece; or the stream's header and
    /// footer read, and the index read next.
    fn start(&mut self) -> Result<Stepped, Refusal> {
        let len = self.reads.len() as usize;
        if let Some(copy) = self.reads.held_copy() {
            let copy = copy.clone();
            return self.done(index_xz(copy, self.input.clone(), self.limits));
        }
        let piece = self.limits.step_input.max(1);
        if len <= piece {
            return match self.reads.copy(0..len) {
                Ok(c) => self.done(index_xz(Arc::new(c.bytes), self.input.clone(), self.limits)),
                Err(missing) => self.wait(Phase::Start, missing),
            };
        }
        let foot_at = len.saturating_sub(xz::FOOTER);
        let [head, foot] = match self.reads.copies([Some(0..xz::HEADER.min(len)), Some(foot_at..len)]) {
            Ok([Some(head), Some(foot)]) => [head, foot],
            Ok(_) => return Err(Refusal::Failed),
            Err(missing) => return self.wait(Phase::Start, missing),
        };
        match xz::ends(&head.bytes, &foot.bytes, len) {
            Some(ends) => {
                self.phase = Phase::Index { ends, bytes: Vec::new() };
                Ok(Stepped::Working)
            }
            // Stream padding after the footer is a stream as the format
            // allows it; anything else at the end is not a footer.
            None if xz::padded(&foot.bytes) => self.to_copy(Whole::BySize),
            None => self.to_copy(Whole::Damaged),
        }
    }

    /// The next piece of the index, and the index read once it is all there:
    /// until its count of blocks is read, as far as the footer, and after
    /// that only as far as that many records can take.
    fn index(&mut self, ends: xz::Ends, mut bytes: Vec<u8>) -> Result<Stepped, Refusal> {
        let wanted = |bytes: &[u8]| match xz::Records::new(bytes, ends.index_len(), ends.index_at) {
            Some(r) => ends.index_len().min(r.bytes_needed()),
            None => ends.index_len(),
        };
        let from = ends.index_at + bytes.len();
        // At least one record's worth, so that the first piece holds the count.
        let piece = self.limits.step_input.max(xz::MAX_RECORD);
        let to = (ends.index_at + wanted(&bytes)).min(from.saturating_add(piece));
        match self.reads.copy(from..to) {
            Ok(c) => bytes.extend_from_slice(&c.bytes),
            Err(missing) => return self.wait(Phase::Index { ends, bytes }, missing),
        }
        let Some(records) = xz::Records::new(&bytes, ends.index_len(), ends.index_at) else {
            return self.to_copy(Whole::Damaged);
        };
        if records.count() > self.limits.block_marks as u64 {
            return Err(Refusal::TooManyXzBlocks);
        }
        let want = wanted(&bytes);
        if bytes.len() < want {
            self.phase = Phase::Index { ends, bytes };
            return Ok(Stepped::Working);
        }
        bytes.truncate(want);
        self.read_index(ends, bytes, records)
    }

    /// The index's records, `records` read from `index`, checked from first
    /// to last, and the block headers read next when they hold together and
    /// come to more than the cap. Any other stream is decoded whole, as over
    /// a copy.
    fn read_index(&mut self, ends: xz::Ends, index: Vec<u8>, records: xz::Records) -> Result<Stepped, Refusal> {
        let mut all = records;
        while all.next(&index).is_some() {}
        if !all.filled() {
            // Blocks that end short of the index may be the last of several
            // streams joined end to end, whose own header comes before them.
            return match all.end().and_then(|end| ends.index_at.checked_sub(end)).filter(|&at| at > 0) {
                Some(at) => {
                    self.phase = Phase::Joined { at };
                    Ok(Stepped::Working)
                }
                None => self.to_copy(Whole::Damaged),
            };
        }
        if all.total_out() <= self.limits.whole as u64 {
            return self.to_copy(Whole::BySize);
        }
        let headers = Headers { ends, index, records, total_out: all.total_out(), blocks: Vec::new(), plain: true, batch: Vec::new() };
        self.phase = Phase::Headers(Box::new(headers));
        Ok(Stepped::Working)
    }

    /// Whether the last of several streams joined end to end starts at byte
    /// `at`, which the decoder of whole streams reads, or the index is
    /// damaged.
    fn joined(&mut self, at: usize) -> Result<Stepped, Refusal> {
        match self.reads.copy(at..at + xz::HEADER) {
            Ok(c) if xz::starts_stream(&c.bytes) => self.to_copy(Whole::BySize),
            Ok(_) => self.to_copy(Whole::Damaged),
            Err(missing) => self.wait(Phase::Joined { at }, missing),
        }
    }

    /// The next block headers, or the stream laid out once they are all read.
    fn headers(&mut self, mut h: Box<Headers>) -> Result<Stepped, Refusal> {
        if h.batch.is_empty() {
            let most = self.reads.most_at_once();
            while h.batch.len() < HEADERS_PER_STEP {
                let mut next = h.records;
                let Some(r) = next.next(&h.index) else { break };
                if h.batch.first().is_some_and(|(first, _)| r.header_bytes().end - first.at > most) {
                    break;
                }
                h.records = next;
                h.batch.push((r, None));
            }
            if h.batch.is_empty() {
                let Headers { ends, total_out, blocks, plain, .. } = *h;
                let l = xz::Layout { check: ends.check, blocks, index_at: ends.index_at, total_out, plain };
                return self.done(laid_out(l, &self.input, self.limits));
            }
        }
        let mut missing = Vec::new();
        for (r, got) in h.batch.iter_mut().filter(|(_, got)| got.is_none()) {
            match self.header_of(r) {
                // A block that does not hold together is one `xz::layout`
                // would not lay out either.
                Ok(head) => match xz::block_at(r, &head, h.ends.check) {
                    Some(blk) => *got = Some(blk),
                    None => return self.to_copy(Whole::Damaged),
                },
                Err(m) => missing.extend(m),
            }
        }
        if !missing.is_empty() {
            missing.sort_unstable_by_key(|m| m.chunk);
            missing.dedup();
            return self.wait(Phase::Headers(h), missing);
        }
        let Headers { batch, blocks, plain, .. } = &mut *h;
        for (_, got) in batch.drain(..) {
            let (blk, p) = got.expect("every header in the step was read");
            *plain &= p;
            blocks.push(blk);
        }
        self.phase = Phase::Headers(h);
        Ok(Stepped::Working)
    }

    /// The header of the block `r` places, and no more of the stream: its
    /// first byte, which gives its length, and then the rest of it. A header
    /// longer than its block is read only to the block's end, and
    /// [`xz::block_at`] does not lay that block out. So no read of a header
    /// names a chunk of the file that holds only the block's data.
    fn header_of(&self, r: &xz::Record) -> Result<Vec<u8>, Vec<crate::source::Missing>> {
        let first = self.reads.copy(r.at..r.at + 1)?;
        let len = first.bytes.first().map_or(1, |&b| (usize::from(b) + 1) * 4).min(r.unpadded);
        Ok(self.reads.copy(r.at..r.at + len)?.bytes)
    }

    /// Copy the stream whole, for the decoder of whole streams: a stream this
    /// does not lay out, or one that comes to no more than the cap. A stream
    /// too long to copy is refused for `why`.
    fn to_copy(&mut self, why: Whole) -> Result<Stepped, Refusal> {
        let len = self.reads.len();
        if len > self.limits.copy_input as u64 {
            return Err(match why {
                Whole::BySize => Refusal::TooLargeCopy,
                Whole::Damaged => Refusal::Failed,
            });
        }
        self.phase = Phase::Copy { bytes: Vec::with_capacity(len as usize) };
        Ok(Stepped::Working)
    }

    /// The next piece of the copy, and the stream decoded whole once the
    /// copy is complete.
    fn copy(&mut self, mut bytes: Vec<u8>) -> Result<Stepped, Refusal> {
        let len = self.reads.len() as usize;
        let step = self.limits.step_input.max(1);
        let piece = step.saturating_mul(COPY_STEPS).min((self.reads.most_at_once() / 2).max(step));
        let to = len.min(bytes.len().saturating_add(piece));
        match self.reads.copy(bytes.len()..to) {
            Ok(c) => bytes.extend_from_slice(&c.bytes),
            Err(missing) => return self.wait(Phase::Copy { bytes }, missing),
        }
        if bytes.len() < len {
            self.phase = Phase::Copy { bytes };
            return Ok(Stepped::Working);
        }
        self.done(whole(Codec::Xz, &bytes))
    }

    /// Wait in `phase` for the chunks `missing` names.
    fn wait(&mut self, phase: Phase, missing: Vec<crate::source::Missing>) -> Result<Stepped, Refusal> {
        self.phase = phase;
        Ok(Stepped::NeedsInput(missing))
    }

    fn done(&mut self, got: Result<Decoded, Refusal>) -> Result<Stepped, Refusal> {
        self.phase = Phase::Over;
        got.map(Stepped::Done)
    }
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
    if p.reach(window, l, units, from + dst.len())?.is_some() {
        return Err(Refusal::Failed);
    }
    p.copy(from, dst);
    let weight = p.weight();
    lock(cache).put(k as u64, p, weight);
    Ok(())
}

/// [`read_span`], with the blocks read through `input`: only the blocks the
/// read decodes, each one's LZMA2 data whole, as many at once as come to
/// `most` bytes of the stream and at least one (see [`Prefix::needs`]). A
/// read that finds chunks of the input missing decodes no further, and keeps
/// what was decoded of the span for the read made again once they are there.
#[allow(clippy::too_many_arguments)]
pub(super) fn read_span_through(
    cache: &Mutex<Recent<Prefix>>,
    input: &RunInput,
    l: &xz::Layout,
    units: Range<usize>,
    k: usize,
    from: usize,
    dst: &mut [u8],
    most: usize,
) -> Result<(), ReadError> {
    let upto = from + dst.len();
    let held = lock(cache).take(k as u64);
    let mut p = held.unwrap_or_else(|| Prefix::new(l, units.clone()));
    let run_len = input.len() as usize;
    let (mut copy, mut asked) = (None, None);
    loop {
        let window = match &copy {
            Some(copy) => Copied::window(copy, input.len()),
            None => Window::of(&[], 0, run_len).expect("an empty window"),
        };
        let Some(next) = p.reach(window, l, units.clone(), upto)? else { break };
        // If the copy made for block `next` still does not hold it, the
        // block's data runs past the stream's end, which `xz::layout` never
        // lays out. The read is refused, so that it does not copy the block
        // again for ever.
        if asked.replace(next) == Some(next) {
            return Err(Refusal::Failed.into());
        }
        match input.copy(p.needs(l, units.clone(), upto, next, most)) {
            Ok(more) => copy = Some(more),
            Err(missing) => {
                let weight = p.weight();
                lock(cache).put(k as u64, p, weight);
                return Err(ReadError::Missing(missing));
            }
        }
    }
    p.copy(from, dst);
    let weight = p.weight();
    lock(cache).put(k as u64, p, weight);
    Ok(())
}

/// The chunks of `input` that [`read_span_through`], asked for the first
/// `upto` bytes of span `k`, would name: those of the stretch it copies first,
/// worked out from how far the span is decoded already, without decoding.
/// Nothing when that much of the span is decoded already.
pub(super) fn span_missing(cache: &Mutex<Recent<Prefix>>, input: &RunInput, l: &xz::Layout, units: Range<usize>, k: usize, upto: usize, most: usize) -> Vec<crate::source::Missing> {
    let needs = {
        let cache = lock(cache);
        let fresh;
        let p = match cache.peek(k as u64) {
            Some(p) => p,
            None => {
                fresh = Prefix::new(l, units.clone());
                &fresh
            }
        };
        p.next_block(units.clone(), upto).map(|next| p.needs(l, units, upto, next, most))
    };
    needs.map_or_else(Vec::new, |range| input.missing(range))
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

    /// The block [`Prefix::reach`], given no bytes of the stream, says it
    /// needs first to decode the first `upto` bytes of the span: the block it
    /// stopped inside, or the one after the last it finished. Nothing when
    /// that much is decoded already.
    fn next_block(&self, units: Range<usize>, upto: usize) -> Option<usize> {
        if self.decoded >= upto && upto < self.len {
            return None;
        }
        match self.decoder {
            Some(_) => Some(self.first + self.blocks.len() - 1),
            None => Some(self.first + self.blocks.len()).filter(|&next| next < units.end),
        }
    }

    /// The bytes of the stream that [`Prefix::reach`] reads next to decode
    /// the first `upto` bytes of the span, once it has said it needs block
    /// `next`: the LZMA2 data of the blocks from `next` to the one holding
    /// byte `upto - 1` of the span, or to the span's last block for a read
    /// that reaches the span's end, as many as come to `most` bytes of the
    /// stream, and `next` whatever it comes to. A block's data is read whole,
    /// since its chunk headers are all read before its first chunk is
    /// decoded.
    fn needs(&self, l: &xz::Layout, units: Range<usize>, upto: usize, next: usize, most: usize) -> Range<usize> {
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
        let start = l.blocks[next].lzma2().start;
        let last = (next + 1..to).take_while(|&i| l.blocks[i].lzma2().end - start <= most).last().unwrap_or(next);
        start..l.blocks[last].lzma2().end
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
    ///
    /// `window` has to hold the LZMA2 data of a block whole for this to
    /// decode any of it. When it does not hold the next block the read
    /// decodes, this stops there and answers that block's number, for the
    /// caller to read it and carry on; nothing when the read is done.
    fn reach(&mut self, window: Window<'_>, l: &xz::Layout, units: Range<usize>, upto: usize) -> Result<Option<usize>, Refusal> {
        let mut off = TraceBuilder::off();
        let holds = |blk: &xz::BlockAt| window.start() <= blk.lzma2().start && blk.lzma2().end <= window.end();
        while self.decoded < upto || upto >= self.len {
            if self.decoder.is_none() {
                let next = self.first + self.blocks.len();
                if next >= units.end {
                    break;
                }
                let blk = &l.blocks[next];
                if !holds(blk) {
                    return Ok(Some(next));
                }
                if lzma::lzma2_len_in(lzma2_of(window, blk)?)? != blk.unpacked {
                    return Err(Refusal::Failed);
                }
                self.blocks.push(Vec::new());
                self.decoder = Some(Lzma2::new());
            }
            let at = self.first + self.blocks.len() - 1;
            let blk = &l.blocks[at];
            if !holds(blk) {
                return Ok(Some(at));
            }
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
            false => Ok(None),
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
