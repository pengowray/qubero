//! Compressed runs too large to hold whole, read a piece at a time.
//!
//! Every other codec here decodes a run into one buffer and refuses once the
//! output passes [`CAP_BYTES`]. A deflate run (raw, zlib or gzip) past that
//! is decoded once from front to back instead, keeping only checkpoints: the
//! places the decoder could start again from, each with the 32 KiB of output
//! before it, which is as far back as a deflate match can copy from. A read
//! then decodes from the checkpoint before it, and only as far as the next
//! one. See `docs/DESIGN-lazy-unpacking.md`.
//!
//! The output between two checkpoints is a span. Spans are what is decoded
//! and what is cached, and each has a trace of its own, made when asked for:
//! the steps of a deflate stream name literals by their byte and matches by
//! their length and distance, so they can be read off any stretch of the
//! stream without the output before it.
//!
//! The same pass records where each block starts, in the run and in the
//! output. That is enough to list the blocks, and to trace any one block when
//! asked: like a span, a block can be traced from the compressed run alone.
//! A hostile run can hold tens of millions of tiny blocks, so past
//! [`MAX_BLOCK_MARKS`] the index keeps the start of every second block,
//! halving again each time the list fills, as it does with checkpoints. A
//! dropped start is found again by decoding from the kept start before it.
//!
//! The guard against a zip bomb is [`LAZY_CAP_BYTES`], a limit on the output.
//! Memory is bounded by the limits below whatever the output comes to, so a
//! bomb can only cost the time of one pass up to that limit.

use std::ops::ControlFlow;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::checksum::{adler32, adler32_update, crc32, crc32_update};
use crate::codec::inflate::{self, Ended, Framing, Resume, Sink, Wrapper};
use crate::codec::{Block, BlockKind, Codec, Refusal, Step, StepField, StepKind, Trace, TraceBuilder, CAP_BYTES};

/// The most a run read a piece at a time may come to. Past this it is refused
/// as [`Refusal::TooLargeLazy`].
pub const LAZY_CAP_BYTES: u64 = 1 << 30;

/// The most checkpoints one run keeps. When there would be more, every second
/// one is dropped and the spacing doubles, so 256 × 32 KiB = 8 MiB of windows
/// is the most a run's checkpoints hold.
pub const MAX_CHECKPOINTS: usize = 256;

/// How much output comes between two checkpoints, until there are too many.
pub const SPACING_START: u64 = 1 << 20;

/// How much decoded output one run keeps, in spans, before the span read
/// longest ago is dropped. One span is kept whatever its size.
pub const SPAN_CACHE_BYTES: usize = 16 << 20;

/// How many steps a span's trace may hold before it names blocks rather than
/// codes, as [`crate::codec::MAX_STEPS`] does for a whole run.
pub const SPAN_TRACE_STEPS: usize = 1_000_000;

/// The most block starts one run keeps. Past this, every second start is
/// dropped, and a dropped start is found again by decoding from the kept start
/// before it. So a run's block starts take at most 65,536 × 16 bytes = 1 MiB.
/// Ordinary encoders write far fewer blocks: zlib ends a block every 16,384
/// codes and a stored block holds at most 64 KiB, so 1 GiB of output is tens
/// of thousands of blocks. A hostile run can start a new block every 10 bits,
/// since an empty fixed-Huffman block is 10 bits long.
pub const MAX_BLOCK_MARKS: usize = 1 << 16;

/// How many lists of dropped block starts one run keeps once it has found
/// them again by decoding. Each list covers the blocks from one kept start to
/// the next, at most 1,024 blocks under the default limits.
const GROUPS_KEPT: usize = 16;

/// As far back as a deflate match can copy from.
const WINDOW: usize = 32 * 1024;

/// How much output the index pass holds before dropping all but the last
/// [`WINDOW`] of it. Larger means fewer copies of the window and more memory.
const SLIDE_AT: usize = 256 * 1024;

/// The limits a run is decoded under. [`Limits::default`] is the constants
/// above; tests make them small, so that a few hundred kilobytes of data
/// reach every one of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The most a run decoded whole may come to: [`CAP_BYTES`].
    pub whole: usize,
    /// [`LAZY_CAP_BYTES`].
    pub lazy: u64,
    /// [`SPACING_START`].
    pub spacing: u64,
    /// [`MAX_CHECKPOINTS`].
    pub checkpoints: usize,
    /// [`SPAN_CACHE_BYTES`].
    pub span_cache: usize,
    /// [`SPAN_TRACE_STEPS`]. Also the most steps one block's trace holds
    /// before its codes become one opaque step, and the most steps the cached
    /// block traces of one run hold in total.
    pub span_steps: usize,
    /// [`MAX_BLOCK_MARKS`].
    pub block_marks: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            whole: CAP_BYTES,
            lazy: LAZY_CAP_BYTES,
            spacing: SPACING_START,
            checkpoints: MAX_CHECKPOINTS,
            span_cache: SPAN_CACHE_BYTES,
            span_steps: SPAN_TRACE_STEPS,
            block_marks: MAX_BLOCK_MARKS,
        }
    }
}

/// Whether a run packed with `codec` can be read a piece at a time.
pub fn can_index(codec: Codec) -> bool {
    wrapper(codec).is_some()
}

fn wrapper(codec: Codec) -> Option<Wrapper> {
    match codec {
        Codec::Deflate => Some(Wrapper::Raw),
        Codec::Zlib => Some(Wrapper::Zlib),
        Codec::Gzip => Some(Wrapper::Gzip),
        _ => None,
    }
}

/// What a run came to: its bytes and trace, held whole as every run under the
/// cap is, or an index to read it through.
pub enum Decoded {
    Whole(Vec<u8>, Trace),
    Indexed(Seekable),
}

/// Open a run whole if it fits under the cap, and index it otherwise.
///
/// For deflate, zlib and gzip this is one pass. The run is decoded as
/// [`crate::codec::decode_traced`] decodes it, with checkpoints taken on the
/// way; a run that ends under the cap comes back exactly as that function
/// gives it, and one that passes the cap carries on from its last checkpoint
/// with the trace off and the output dropped as it leaves the window. Every
/// other codec is decoded whole or refused, as it always was.
pub fn decode_or_index(codec: Codec, packed: Arc<Vec<u8>>) -> Result<Decoded, Refusal> {
    decode_or_index_with(codec, packed, Limits::default())
}

/// [`decode_or_index`] under limits other than the default ones.
pub fn decode_or_index_with(codec: Codec, packed: Arc<Vec<u8>>, limits: Limits) -> Result<Decoded, Refusal> {
    let Some(wrapper) = wrapper(codec) else {
        return crate::codec::decode_traced(codec, &packed).map(|(bytes, trace)| Decoded::Whole(bytes, trace));
    };
    if packed.len() > CAP_BYTES {
        return Err(Refusal::TooLarge);
    }
    let data = &packed[..];
    let f = inflate::framing(wrapper, data)?;
    let marks = Checkpoints::new(limits.spacing, limits.checkpoints);

    // The pass that may come to a whole run, traced, with checkpoints taken
    // in case it does not.
    let mut b = TraceBuilder::default();
    if wrapper != Wrapper::Raw {
        b.push(0, 0, StepKind::Header(StepField::Wrapper, 0));
    }
    let mut whole = WindowSink::new(Vec::new(), 0, limits.whole as u64, false);
    whole.marks = Some(marks);
    whole.blocks = Some(BlockMarks::new(limits.block_marks));
    match inflate::run(data, f.start, f.end, &mut whole, &mut b, None) {
        Ok(_) => {
            let out = whole.hist;
            if !inflate::trailer_agrees(&f, data, out.len() as u64, || crc32(&out), || adler32(&out)) {
                return Err(Refusal::Failed);
            }
            if wrapper != Wrapper::Raw {
                b.push(f.end, out.len() as u64, StepKind::Header(StepField::Wrapper, 0));
                b.finish_at(data.len() as u64 * 8, out.len() as u64);
            }
            return Ok(Decoded::Whole(out, b.done()));
        }
        Err(Refusal::TooLarge) => {}
        Err(other) => return Err(other),
    }
    drop(b);

    // Past the cap: on from the last checkpoint, which is at most the spacing
    // before where the first pass stopped, keeping the window and nothing
    // more. The sums are taken over the output as it leaves the window.
    let marks = whole.marks.take().expect("the first pass keeps checkpoints");
    let last = marks.list.last().cloned().expect("checkpoint 0 is taken before any output");
    let upto = &whole.hist[..last.out as usize];
    let (crc, adler) = (crc32(upto), adler32(upto));
    // The second pass records again every block that starts at or after the
    // checkpoint, so those are dropped here.
    let mut blocks = whole.blocks.take().expect("the first pass records blocks");
    blocks.truncate(last.blocks);
    drop(whole);
    let mut index = WindowSink::new(last.window.to_vec(), last.out - last.window.len() as u64, limits.lazy, true);
    index.marks = Some(marks);
    index.blocks = Some(blocks);
    index.sums = Some(Sums { crc, adler, upto: last.out });
    let blocks_end = match inflate::run(data, f.start, f.end, &mut index, &mut TraceBuilder::off(), Some(last.resume())) {
        Ok(Ended::Stream(at)) => at,
        // This sink never asks the decoder to stop, so a stop is a fault.
        Ok(Ended::Stopped(_)) => return Err(Refusal::Failed),
        Err(Refusal::TooLarge) => return Err(Refusal::TooLargeLazy),
        Err(other) => return Err(other),
    };
    let len = index.len();
    index.sum_to(len);
    let sums = index.sums.expect("set above");
    if !inflate::trailer_agrees(&f, data, len, || sums.crc, || sums.adler) {
        return Err(Refusal::Failed);
    }
    let checkpoints = index.marks.take().expect("set above").list;
    let blocks = index.blocks.take().expect("set above");
    Ok(Decoded::Indexed(Seekable {
        codec,
        packed,
        framing: f,
        len,
        checkpoints,
        blocks,
        blocks_end,
        crc32: sums.crc,
        adler32: sums.adler,
        limits,
        spans: Mutex::new(Recent::new(limits.span_cache)),
        traced: Mutex::new(None),
        groups: Mutex::new(Recent::new(GROUPS_KEPT)),
        block_traces: Mutex::new(Recent::new(limits.span_steps)),
    }))
}

/// A place a decode can start again from. See [`Sink::boundary`].
#[derive(Debug, Clone)]
struct Checkpoint {
    /// The bit of the run the next code starts at.
    in_bit: u64,
    /// How many bytes of output come before it.
    out: u64,
    /// The header of the Huffman-coded block it is inside, whose tables a
    /// decode starting here reads again. Nothing at the start of a block.
    block: Option<u64>,
    /// How many blocks start before `in_bit`. This counts the block the
    /// checkpoint is inside, and does not count a block starting at `in_bit`.
    blocks: u64,
    /// The last 32 KiB of output before it, or all of it when there is less.
    window: Arc<[u8]>,
}

impl Checkpoint {
    fn resume(&self) -> Resume {
        Resume { in_bit: self.in_bit, block: self.block }
    }
}

/// The checkpoints an index pass has taken, and where it takes the next.
struct Checkpoints {
    list: Vec<Checkpoint>,
    spacing: u64,
    next: u64,
    max: usize,
}

impl Checkpoints {
    fn new(spacing: u64, max: usize) -> Checkpoints {
        Checkpoints { list: Vec::new(), spacing: spacing.max(1), next: 0, max: max.max(2) }
    }

    /// Keep a checkpoint at `out`, if one is due there. Past the most there
    /// may be, every second one goes and the spacing doubles. The first is
    /// never one that goes: a span starting anywhere else would have nothing
    /// to start from.
    fn offer(&mut self, in_bit: u64, out: u64, block: Option<u64>, blocks: u64, window: &[u8]) {
        if out < self.next {
            return;
        }
        self.list.push(Checkpoint { in_bit, out, block, blocks, window: window.into() });
        self.next = out + self.spacing;
        if self.list.len() > self.max {
            let mut i = 0;
            self.list.retain(|_| {
                i += 1;
                (i - 1) % 2 == 0
            });
            self.spacing *= 2;
            self.next = self.list.last().map_or(0, |c| c.out) + self.spacing;
        }
    }
}

/// Where each block starts, as a bit of the run and a byte of the output,
/// recorded by the index pass. Holds every block's start until there are more
/// than `max`, and from then on the start of every `stride`th block. Block 0's
/// start is always kept.
#[derive(Debug, Clone)]
struct BlockMarks {
    /// The start of blocks 0, `stride`, 2 × `stride` and so on.
    list: Vec<(u64, u64)>,
    /// How many blocks have started, kept or not.
    count: u64,
    stride: u64,
    max: usize,
}

impl BlockMarks {
    fn new(max: usize) -> BlockMarks {
        BlockMarks { list: Vec::new(), count: 0, stride: 1, max: max.max(2) }
    }

    /// Record a block that starts at `in_bit`, with `out` bytes of output
    /// before it. When the list grows past `max`, every second start is
    /// dropped and the stride doubles, so the list keeps the starts of the
    /// blocks whose number is a multiple of the stride.
    fn record(&mut self, in_bit: u64, out: u64) {
        if self.count % self.stride == 0 {
            self.list.push((in_bit, out));
            if self.list.len() > self.max {
                let mut i = 0;
                self.list.retain(|_| {
                    i += 1;
                    (i - 1) % 2 == 0
                });
                self.stride *= 2;
            }
        }
        self.count += 1;
    }

    /// Keep only the first `count` blocks, for a second pass that starts again
    /// from a checkpoint after them and records the later blocks again.
    fn truncate(&mut self, count: u64) {
        self.list.truncate(count.div_ceil(self.stride) as usize);
        self.count = count;
    }
}

/// The running CRC-32 and Adler-32 of the output, taken up to `upto`.
#[derive(Debug, Clone, Copy)]
struct Sums {
    crc: u32,
    adler: u32,
    upto: u64,
}

/// Output kept from some point of the stream on: all of it, or once `slide`
/// is set, the last [`WINDOW`] of it and whatever has come since the last
/// slide.
struct WindowSink {
    hist: Vec<u8>,
    /// Where `hist[0]` is in the whole stream's output.
    base: u64,
    /// The most the output may come to.
    cap: u64,
    slide: bool,
    /// Take checkpoints on the way.
    marks: Option<Checkpoints>,
    /// Record where blocks start on the way.
    blocks: Option<BlockMarks>,
    /// Sum the output as it leaves.
    sums: Option<Sums>,
    /// Stop at the first boundary at or past this bit.
    stop_at: Option<u64>,
}

impl WindowSink {
    fn new(hist: Vec<u8>, base: u64, cap: u64, slide: bool) -> WindowSink {
        WindowSink { hist, base, cap, slide, marks: None, blocks: None, sums: None, stop_at: None }
    }

    /// Bring the sums up to `upto`, from what `hist` still holds.
    fn sum_to(&mut self, upto: u64) {
        let Some(s) = &mut self.sums else { return };
        if upto <= s.upto {
            return;
        }
        let bytes = &self.hist[(s.upto - self.base) as usize..(upto - self.base) as usize];
        s.crc = crc32_update(s.crc, bytes);
        s.adler = adler32_update(s.adler, bytes);
        s.upto = upto;
    }

    #[inline]
    fn slid(&mut self) {
        if self.slide && self.hist.len() >= SLIDE_AT {
            let cut = self.hist.len() - WINDOW;
            self.sum_to(self.base + cut as u64);
            self.hist.drain(..cut);
            self.base += cut as u64;
        }
    }
}

impl Sink for WindowSink {
    #[inline]
    fn len(&self) -> u64 {
        self.base + self.hist.len() as u64
    }

    #[inline]
    fn literal(&mut self, byte: u8) -> Result<(), Refusal> {
        if self.len() >= self.cap {
            return Err(Refusal::TooLarge);
        }
        self.hist.push(byte);
        self.slid();
        Ok(())
    }

    #[inline]
    fn copy(&mut self, dist: u32, len: u32) -> Result<(), Refusal> {
        if dist as usize > self.hist.len() {
            return Err(Refusal::Failed);
        }
        if self.len() + len as u64 > self.cap {
            return Err(Refusal::TooLarge);
        }
        let from = self.hist.len() - dist as usize;
        for k in 0..len as usize {
            let byte = self.hist[from + k];
            self.hist.push(byte);
        }
        self.slid();
        Ok(())
    }

    fn stored(&mut self, bytes: &[u8]) -> Result<(), Refusal> {
        if self.len() + bytes.len() as u64 > self.cap {
            return Err(Refusal::TooLarge);
        }
        self.hist.extend_from_slice(bytes);
        self.slid();
        Ok(())
    }

    #[inline]
    fn boundary(&mut self, in_bit: u64, block: Option<u64>) -> ControlFlow<()> {
        if self.stop_at.is_some_and(|stop| in_bit >= stop) {
            return ControlFlow::Break(());
        }
        let out = self.len();
        if let Some(marks) = &mut self.marks {
            if out >= marks.next {
                let keep = (WINDOW as u64).min(out) as usize;
                let started = self.blocks.as_ref().map_or(0, |b| b.count);
                marks.offer(in_bit, out, block, started, &self.hist[self.hist.len() - keep..]);
            }
        }
        if let (None, Some(blocks)) = (block, &mut self.blocks) {
            blocks.record(in_bit, out);
        }
        ControlFlow::Continue(())
    }
}

/// Output counted and not kept, for a decode that wants only its trace or
/// where its blocks start. A deflate step says what it copied and from how
/// far back, not what the bytes were, so the trace of a span needs none of
/// the output before it.
struct CountSink {
    len: u64,
    stop_at: Option<u64>,
    /// Where each block starts, as the bit of the run and the byte of the
    /// output, when asked for.
    starts: Option<Vec<(u64, u64)>>,
}

impl Sink for CountSink {
    #[inline]
    fn len(&self) -> u64 {
        self.len
    }

    #[inline]
    fn literal(&mut self, _: u8) -> Result<(), Refusal> {
        self.len += 1;
        Ok(())
    }

    #[inline]
    fn copy(&mut self, dist: u32, len: u32) -> Result<(), Refusal> {
        if u64::from(dist) > self.len {
            return Err(Refusal::Failed);
        }
        self.len += u64::from(len);
        Ok(())
    }

    fn stored(&mut self, bytes: &[u8]) -> Result<(), Refusal> {
        self.len += bytes.len() as u64;
        Ok(())
    }

    #[inline]
    fn boundary(&mut self, in_bit: u64, block: Option<u64>) -> ControlFlow<()> {
        if self.stop_at.is_some_and(|stop| in_bit >= stop) {
            return ControlFlow::Break(());
        }
        if let (None, Some(starts)) = (block, &mut self.starts) {
            starts.push((in_bit, self.len));
        }
        ControlFlow::Continue(())
    }
}

/// A compressed run that comes to more than can be held whole, read through
/// its checkpoints.
///
/// Holds the run itself, which is at most [`CAP_BYTES`]; the checkpoints; where
/// the blocks start; the spans decoded most recently, up to
/// [`Limits::span_cache`]; the trace of the span asked about most recently;
/// and the traces of the blocks asked about most recently, up to
/// [`Limits::span_steps`] steps between them. Reads take `&self`, as a
/// [`Source`](crate::source::Source) is read, so the caches are behind locks.
pub struct Seekable {
    codec: Codec,
    packed: Arc<Vec<u8>>,
    framing: Framing,
    len: u64,
    checkpoints: Vec<Checkpoint>,
    blocks: BlockMarks,
    /// Where the last block ends, as a bit of the run.
    blocks_end: u64,
    crc32: u32,
    adler32: u32,
    limits: Limits,
    spans: Mutex<Recent<Arc<Vec<u8>>>>,
    traced: Mutex<Option<(usize, Arc<Trace>)>>,
    /// Block starts found again by decoding: one list for each kept start,
    /// keyed by that start's place in `blocks.list`.
    groups: Mutex<Recent<Arc<[(u64, u64)]>>>,
    block_traces: Mutex<Recent<Arc<Trace>>>,
}

impl std::fmt::Debug for Seekable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Seekable")
            .field("codec", &self.codec)
            .field("packed", &self.packed.len())
            .field("len", &self.len)
            .field("checkpoints", &self.checkpoints.len())
            .field("blocks", &self.blocks.count)
            .finish()
    }
}

/// A lock that a panic while it was held does not make unusable: what it
/// guards is a cache, and a cache in any state is still a cache.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Seekable {
    pub fn codec(&self) -> Codec {
        self.codec
    }

    /// How many bytes the run comes to.
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The compressed run, from its first byte, which is what the bits of
    /// every trace count from.
    pub fn packed(&self) -> &Arc<Vec<u8>> {
        &self.packed
    }

    /// The CRC-32 of the whole output, worked out on the pass that indexed it.
    pub fn crc32(&self) -> u32 {
        self.crc32
    }

    /// The Adler-32 of the whole output, the same way.
    pub fn adler32(&self) -> u32 {
        self.adler32
    }

    /// How many spans the output is divided into, one per checkpoint.
    pub fn spans(&self) -> usize {
        self.checkpoints.len()
    }

    /// The output bytes span `k` covers.
    pub fn span_bytes(&self, k: usize) -> std::ops::Range<u64> {
        let end = self.checkpoints.get(k + 1).map_or(self.len, |c| c.out);
        self.checkpoints[k].out..end
    }

    /// The span holding output byte `byte`. A span of no bytes holds none, so
    /// the answer is the last span starting at or before it.
    pub fn span_of_byte(&self, byte: u64) -> Option<usize> {
        if byte >= self.len {
            return None;
        }
        Some(self.checkpoints.partition_point(|c| c.out <= byte).saturating_sub(1))
    }

    /// The span that read bit `bit` of the run: the one whose checkpoints
    /// either side hold it. A span's trace reaches back over the header of
    /// the block its checkpoint is inside, and those bits are still the span
    /// before's, which read them as codes. The wrapper's header is span 0's
    /// and its trailer is the last span's.
    pub fn span_of_bit(&self, bit: u64) -> Option<usize> {
        if bit >= self.packed.len() as u64 * 8 {
            return None;
        }
        Some(self.checkpoints.partition_point(|c| c.in_bit <= bit).saturating_sub(1))
    }

    /// Bytes `at..at + out.len()` of the output.
    pub fn read(&self, at: u64, out: &mut [u8]) -> Result<(), Refusal> {
        let end = at.checked_add(out.len() as u64).ok_or(Refusal::Failed)?;
        if end > self.len {
            return Err(Refusal::Failed);
        }
        let mut pos = at;
        while pos < end {
            let k = self.span_of_byte(pos).ok_or(Refusal::Failed)?;
            let range = self.span_bytes(k);
            let bytes = self.span(k)?;
            let take = (range.end.min(end) - pos) as usize;
            let from = (pos - range.start) as usize;
            let to = (pos - at) as usize;
            out[to..to + take].copy_from_slice(&bytes[from..from + take]);
            pos += take as u64;
        }
        Ok(())
    }

    /// Everything span `k` comes to, from the cache or decoded now.
    fn span(&self, k: usize) -> Result<Arc<Vec<u8>>, Refusal> {
        if let Some(bytes) = lock(&self.spans).get(k as u64) {
            return Ok(bytes);
        }
        let bytes = Arc::new(self.decode_span(k)?);
        lock(&self.spans).put(k as u64, bytes.clone(), bytes.len());
        Ok(bytes)
    }

    /// Decode span `k` from its own checkpoint to the next one.
    fn decode_span(&self, k: usize) -> Result<Vec<u8>, Refusal> {
        let c = &self.checkpoints[k];
        let range = self.span_bytes(k);
        let mut hist = Vec::with_capacity(c.window.len() + (range.end - range.start) as usize);
        hist.extend_from_slice(&c.window);
        let mut sink = WindowSink::new(hist, c.out - c.window.len() as u64, range.end, false);
        sink.stop_at = self.checkpoints.get(k + 1).map(|n| n.in_bit);
        let f = &self.framing;
        inflate::run(&self.packed, f.start, f.end, &mut sink, &mut TraceBuilder::off(), Some(c.resume()))?;
        // The same decoder over the same bits as the pass that indexed it, so
        // a span that comes out another length is a decoder disagreeing with
        // itself.
        if sink.len() != range.end {
            return Err(Refusal::Failed);
        }
        let mut bytes = sink.hist;
        bytes.drain(..c.window.len());
        Ok(bytes)
    }

    /// The trace of span `k`: every step from where its decode starts to
    /// where the next span's does, counted from the front of the run and of
    /// the output as a whole trace counts. Kept until another span's is asked
    /// for.
    ///
    /// A decode starting inside a block reads the block's header and tables
    /// again, so the trace starts at that header and has them, and then one
    /// opaque step over the codes between the tables and the checkpoint,
    /// which made bytes of the span before.
    pub fn span_trace(&self, k: usize) -> Result<Arc<Trace>, Refusal> {
        if let Some((held, t)) = lock(&self.traced).as_ref() {
            if *held == k {
                return Ok(t.clone());
            }
        }
        let c = self.checkpoints.get(k).ok_or(Refusal::Failed)?;
        let f = &self.framing;
        let wrapped = f.wrapper != Wrapper::Raw;
        let first = match (k, wrapped) {
            (0, true) => 0,
            _ => c.block.unwrap_or(c.in_bit),
        };
        let mut b = TraceBuilder::with_budget(self.limits.span_steps).starting_at(first, c.out);
        if k == 0 && wrapped {
            b.push(0, 0, StepKind::Header(StepField::Wrapper, 0));
        }
        let mut sink = CountSink { len: c.out, stop_at: self.checkpoints.get(k + 1).map(|n| n.in_bit), starts: None };
        let ended = inflate::run(&self.packed, f.start, f.end, &mut sink, &mut b, Some(c.resume()))?;
        if matches!(ended, Ended::Stream(_)) && wrapped {
            b.push(f.end, sink.len, StepKind::Header(StepField::Wrapper, 0));
            b.finish_at(self.packed.len() as u64 * 8, sink.len);
        }
        let t = Arc::new(b.done());
        *lock(&self.traced) = Some((k, t.clone()));
        Ok(t)
    }

    /// The trace that holds output byte `byte`: its span's.
    pub fn trace_at_byte(&self, byte: u64) -> Option<Arc<Trace>> {
        self.span_trace(self.span_of_byte(byte)?).ok()
    }

    /// The trace that names bit `bit` of the run: the span's that read it.
    pub fn trace_at_bit(&self, bit: u64) -> Option<Arc<Trace>> {
        self.span_trace(self.span_of_bit(bit)?).ok()
    }

    /// Which step produced byte `byte` of the output, as
    /// [`Trace::map_out`] says for a whole run.
    pub fn map_out(&self, byte: u64) -> Option<Step> {
        self.trace_at_byte(byte)?.map_out(byte)
    }

    /// Which step read bit `bit` of the run, as [`Trace::map_in`] says for a
    /// whole run.
    pub fn map_in(&self, bit: u64) -> Option<Step> {
        self.trace_at_bit(bit)?.map_in(bit)
    }

    /// How many blocks the run holds.
    pub fn blocks(&self) -> u64 {
        self.blocks.count
    }

    /// Block `i`: where it is in the run and in the output, how it is coded
    /// and whether it says it is the last. Its `steps` are empty; they are in
    /// [`Seekable::block_trace`].
    pub fn block(&self, i: u64) -> Result<Block, Refusal> {
        let (in_start, out_start) = self.block_start(i)?;
        let (in_end, out_end) = match i + 1 < self.blocks.count {
            true => self.block_start(i + 1)?,
            false => (self.blocks_end, self.len),
        };
        // BFINAL and BTYPE, the first three bits of the block header, low bit
        // first, read from the run, which is in memory.
        let bit = |k: u64| self.packed.get((k / 8) as usize).map_or(0, |b| (b >> (k % 8)) & 1);
        let kind = match bit(in_start + 1) | bit(in_start + 2) << 1 {
            0 => BlockKind::Stored,
            1 => BlockKind::Fixed,
            2 => BlockKind::Dynamic,
            _ => return Err(Refusal::Failed),
        };
        Ok(Block { steps: 0..0, kind, last: bit(in_start) == 1, in_bits: in_start..in_end, out_bytes: out_start..out_end })
    }

    /// The block that bit `bit` of the run belongs to. None for a bit of the
    /// gzip or zlib wrapper, or of the padding after the last block.
    pub fn block_of_bit(&self, bit: u64) -> Option<u64> {
        let list = &self.blocks.list;
        if bit < list.first()?.0 || bit >= self.blocks_end {
            return None;
        }
        let g = list.partition_point(|m| m.0 <= bit) - 1;
        if self.blocks.stride == 1 {
            return Some(g as u64);
        }
        let starts = self.group(g).ok()?;
        Some(g as u64 * self.blocks.stride + starts.partition_point(|m| m.0 <= bit) as u64 - 1)
    }

    /// Where block `i` starts: the bit of its header and how much output
    /// comes before it.
    fn block_start(&self, i: u64) -> Result<(u64, u64), Refusal> {
        if i >= self.blocks.count {
            return Err(Refusal::Failed);
        }
        let stride = self.blocks.stride;
        let (g, j) = ((i / stride) as usize, (i % stride) as usize);
        if j == 0 {
            return Ok(self.blocks.list[g]);
        }
        self.group(g)?.get(j).copied().ok_or(Refusal::Failed)
    }

    /// The start of every block from kept start `g` up to the next kept start,
    /// found by decoding from start `g` and counting the output.
    fn group(&self, g: usize) -> Result<Arc<[(u64, u64)]>, Refusal> {
        if let Some(starts) = lock(&self.groups).get(g as u64) {
            return Ok(starts);
        }
        let &(in_bit, out) = self.blocks.list.get(g).ok_or(Refusal::Failed)?;
        let mut sink = CountSink { len: out, stop_at: self.blocks.list.get(g + 1).map(|m| m.0), starts: Some(Vec::new()) };
        let f = &self.framing;
        inflate::run(&self.packed, f.start, f.end, &mut sink, &mut TraceBuilder::off(), Some(Resume { in_bit, block: None }))?;
        let starts: Arc<[(u64, u64)]> = sink.starts.take().unwrap_or_default().into();
        // The same decoder read these bits on the index pass, so a different
        // count is the decoder disagreeing with itself.
        let want = self.blocks.stride.min(self.blocks.count - g as u64 * self.blocks.stride);
        if starts.len() as u64 != want || starts.first() != Some(&(in_bit, out)) {
            return Err(Refusal::Failed);
        }
        lock(&self.groups).put(g as u64, starts.clone(), 1);
        Ok(starts)
    }

    /// The trace of block `i` alone: its header, its tables and its codes,
    /// with bits and bytes counted from the start of the run and of the
    /// output, as in every trace here. [`Trace::blocks`] has this block as its
    /// only entry. The last block's trace also covers the padding after it, up
    /// to the end of the deflate data.
    ///
    /// In a block of more than [`Limits::span_steps`] codes, the codes are one
    /// opaque step, as they are in a whole run's trace past
    /// [`crate::codec::MAX_STEPS`].
    pub fn block_trace(&self, i: u64) -> Result<Arc<Trace>, Refusal> {
        if let Some(t) = lock(&self.block_traces).get(i) {
            return Ok(t);
        }
        let block = self.block(i)?;
        let (in_bit, out) = (block.in_bits.start, block.out_bytes.start);
        let next = (i + 1 < self.blocks.count).then_some(block.in_bits.end);
        let mut sink = CountSink { len: out, stop_at: next, starts: None };
        let mut b = TraceBuilder::with_budget(self.limits.span_steps).starting_at(in_bit, out);
        let f = &self.framing;
        inflate::run(&self.packed, f.start, f.end, &mut sink, &mut b, Some(Resume { in_bit, block: None }))?;
        let t = b.done();
        match t.blocks() {
            [one] if one.in_bits == block.in_bits && one.out_bytes == block.out_bytes => {}
            _ => return Err(Refusal::Failed),
        }
        let t = Arc::new(t);
        lock(&self.block_traces).put(i, t.clone(), t.len());
        Ok(t)
    }

    /// How many bytes the checkpoints' windows, the block starts and the span
    /// cache hold, for tests and for saying what a run costs.
    pub fn held_bytes(&self) -> usize {
        self.checkpoints.iter().map(|c| c.window.len()).sum::<usize>()
            + self.blocks.list.len() * std::mem::size_of::<(u64, u64)>()
            + lock(&self.spans).weight
    }

    /// Where each checkpoint is, as the bit of the run and the byte of the
    /// output, for tests.
    #[cfg(test)]
    fn marks(&self) -> Vec<(u64, u64, Option<u64>)> {
        self.checkpoints.iter().map(|c| (c.in_bit, c.out, c.block)).collect()
    }
}

/// A cache of what was made most recently for one run, up to a total weight,
/// that drops the entry used longest ago first. Spans are weighed in bytes,
/// block traces in steps, and each list of block starts counts as 1. The
/// newest entry is kept whatever it weighs.
struct Recent<T> {
    entries: Vec<(u64, T, usize, u64)>,
    weight: usize,
    cap: usize,
    clock: u64,
    #[cfg(test)]
    peak: usize,
}

impl<T: Clone> Recent<T> {
    fn new(cap: usize) -> Recent<T> {
        Recent {
            entries: Vec::new(),
            weight: 0,
            cap,
            clock: 0,
            #[cfg(test)]
            peak: 0,
        }
    }

    fn get(&mut self, k: u64) -> Option<T> {
        self.clock += 1;
        let clock = self.clock;
        let e = self.entries.iter_mut().find(|e| e.0 == k)?;
        e.3 = clock;
        Some(e.1.clone())
    }

    fn put(&mut self, k: u64, value: T, weight: usize) {
        while !self.entries.is_empty() && self.weight + weight > self.cap {
            let oldest = (0..self.entries.len()).min_by_key(|&i| self.entries[i].3).expect("not empty");
            let gone = self.entries.swap_remove(oldest);
            self.weight -= gone.2;
        }
        self.clock += 1;
        self.weight += weight;
        #[cfg(test)]
        {
            self.peak = self.peak.max(self.weight);
        }
        self.entries.push((k, value, weight, self.clock));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Small enough that a few hundred kilobytes reach every limit: the run is
    /// indexed past 64 KiB, checkpoints are 8 KiB apart until there are more
    /// than 12 of them, and the cache holds 40 KiB.
    fn small() -> Limits {
        Limits { whole: 64 << 10, lazy: 4 << 20, spacing: 8 << 10, checkpoints: 12, span_cache: 40 << 10, span_steps: 1_000_000, block_marks: MAX_BLOCK_MARKS }
    }

    fn rand_from(mut seed: u64) -> impl FnMut() -> u64 {
        move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        }
    }

    /// Words, runs of one byte and noise, in stretches, so that an encoder
    /// writes literals, short and long matches, and blocks of every kind.
    fn mixed(len: usize, seed: u64) -> Vec<u8> {
        let mut rand = rand_from(seed);
        let words = ["deflate ", "window ", "checkpoint ", "span ", "the ", "of ", "Rubin ", "FITS "];
        let mut out = Vec::with_capacity(len);
        while out.len() < len {
            match rand() % 4 {
                0 => {
                    for _ in 0..rand() % 400 {
                        out.extend_from_slice(words[(rand() % 8) as usize].as_bytes());
                    }
                }
                1 => out.extend(std::iter::repeat_n((rand() % 256) as u8, (rand() % 5000) as usize)),
                2 => out.extend((0..rand() % 3000).map(|_| rand() as u8)),
                _ => out.extend((0..rand() % 2000).map(|i| (i / 7) as u8)),
            }
        }
        out.truncate(len);
        out
    }

    fn gzip_of(data: &[u8], level: u8) -> Vec<u8> {
        gzip_around(data, &miniz_oxide::deflate::compress_to_vec(data, level))
    }

    fn gzip_around(data: &[u8], raw: &[u8]) -> Vec<u8> {
        let mut out = vec![0x1f, 0x8b, 8, 0x08, 0, 0, 0, 0, 0, 3];
        out.extend_from_slice(b"name.fits\0");
        out.extend_from_slice(raw);
        out.extend_from_slice(&crc32(data).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out
    }

    /// Raw deflate flushed every `every` bytes of input, as a writer that
    /// flushes each line does: each flush ends a block and adds an empty
    /// stored one, so a few hundred kilobytes come to hundreds of blocks.
    fn flushed(data: &[u8], level: u8, every: usize) -> Vec<u8> {
        use miniz_oxide::deflate::core::{compress, create_comp_flags_from_zip_params, CompressorOxide, TDEFLFlush};
        let mut c = CompressorOxide::new(create_comp_flags_from_zip_params(level as i32, -15, 0));
        let mut out = Vec::new();
        let mut buf = vec![0u8; every * 2 + 4096];
        let chunks: Vec<&[u8]> = data.chunks(every).collect();
        for (k, chunk) in chunks.iter().enumerate() {
            let flush = if k + 1 == chunks.len() { TDEFLFlush::Finish } else { TDEFLFlush::Sync };
            let mut input = *chunk;
            loop {
                let (_, read, wrote) = compress(&mut c, input, &mut buf, flush);
                out.extend_from_slice(&buf[..wrote]);
                input = &input[read..];
                if input.is_empty() && wrote < buf.len() {
                    break;
                }
            }
        }
        out
    }

    fn indexed(codec: Codec, packed: &[u8], limits: Limits) -> Seekable {
        match decode_or_index_with(codec, Arc::new(packed.to_vec()), limits) {
            Ok(Decoded::Indexed(s)) => s,
            Ok(Decoded::Whole(bytes, _)) => panic!("{} bytes came back whole", bytes.len()),
            Err(why) => panic!("refused: {why:?}"),
        }
    }

    /// Every encoder level and every wrapper, read back a piece at a time:
    /// the whole output in one read, and reads of every length across the
    /// span boundaries, in an order that keeps the cache turning over.
    #[test]
    fn a_large_run_reads_back_as_what_went_in_from_anywhere() {
        let data = mixed(300_000, 7);
        for level in [0u8, 1, 6, 9] {
            let raw = miniz_oxide::deflate::compress_to_vec(&data, level);
            let zlib = miniz_oxide::deflate::compress_to_vec_zlib(&data, level);
            let gzip = gzip_of(&data, level);
            for (codec, packed) in [(Codec::Deflate, &raw), (Codec::Zlib, &zlib), (Codec::Gzip, &gzip)] {
                let s = indexed(codec, packed, small());
                assert_eq!(s.len(), data.len() as u64);
                assert_eq!(s.crc32(), crc32(&data));
                assert_eq!(s.adler32(), adler32(&data));
                let mut all = vec![0u8; data.len()];
                s.read(0, &mut all).expect("reads");
                assert!(all == data, "{codec:?} level {level}: the whole output differs");
                let mut rand = rand_from(level as u64 + 99);
                for _ in 0..80 {
                    let at = rand() % data.len() as u64;
                    let n = (rand() % 30_000).min(data.len() as u64 - at) as usize;
                    let mut got = vec![0u8; n];
                    s.read(at, &mut got).expect("reads");
                    assert!(got == data[at as usize..at as usize + n], "{codec:?} level {level}: {n} bytes at {at} differ");
                }
                let peak = lock(&s.spans).peak;
                let largest = (0..s.spans()).map(|k| s.span_bytes(k)).map(|r| r.end - r.start).max().unwrap() as usize;
                assert!(peak <= small().span_cache + largest, "{codec:?} level {level}: the cache held {peak} bytes");
            }
        }
    }

    /// Past the most checkpoints there may be, every second one goes, and the
    /// first stays: it is the only place span 0 can start from.
    #[test]
    fn checkpoints_thin_out_and_the_first_stays() {
        let data = mixed(600_000, 3);
        let s = indexed(Codec::Deflate, &miniz_oxide::deflate::compress_to_vec(&data, 6), small());
        let marks = s.marks();
        assert!(marks.len() <= small().checkpoints, "{} checkpoints", marks.len());
        assert_eq!(marks[0], (0, 0, None));
        assert!(marks.windows(2).all(|w| w[0].0 < w[1].0 && w[0].1 <= w[1].1));
        // Thinned at least once: 600 KB at 8 KiB apart is far more than 12.
        assert!(marks.windows(2).any(|w| w[1].1 - w[0].1 >= 16 << 10));
        // And some of them are inside a block, which is the case that reads
        // a block's tables again.
        assert!(marks.iter().any(|m| m.2.is_some()), "no checkpoint inside a block");
    }

    /// A run under the cap comes back whole, with the same bytes and the same
    /// trace the whole decoders give, step for step.
    #[test]
    fn a_run_under_the_cap_comes_back_as_it_always_did() {
        let data = mixed(50_000, 5);
        let raw = miniz_oxide::deflate::compress_to_vec(&data, 6);
        let zlib = miniz_oxide::deflate::compress_to_vec_zlib(&data, 6);
        let gzip = gzip_of(&data, 6);
        for (codec, packed) in [(Codec::Deflate, &raw), (Codec::Zlib, &zlib), (Codec::Gzip, &gzip)] {
            let (bytes, trace) = crate::codec::decode_traced(codec, packed).expect("decodes");
            let Ok(Decoded::Whole(ours, t)) = decode_or_index_with(codec, Arc::new(packed.clone()), small()) else {
                panic!("{codec:?} did not come back whole");
            };
            assert_eq!(ours, bytes);
            assert_eq!(t.len(), trace.len());
            assert!(t.steps().zip(trace.steps()).all(|(a, b)| a == b), "{codec:?}: the steps differ");
            assert_eq!(t.blocks(), trace.blocks());
            assert_eq!((t.in_bits(), t.out_bytes()), (trace.in_bits(), trace.out_bytes()));
        }
    }

    /// Each span's trace tiles what its decode read and wrote, and every byte
    /// and bit maps to the same step through the spans as through the trace
    /// of the whole run.
    #[test]
    fn span_traces_agree_with_the_whole_trace() {
        let data = mixed(300_000, 11);
        for (codec, packed) in [
            (Codec::Deflate, miniz_oxide::deflate::compress_to_vec(&data, 6)),
            (Codec::Gzip, gzip_of(&data, 9)),
            (Codec::Zlib, miniz_oxide::deflate::compress_to_vec_zlib(&data, 1)),
        ] {
            let (_, whole) = crate::codec::decode_traced(codec, &packed).expect("decodes whole");
            assert!(!whole.coarse());
            let s = indexed(codec, &packed, small());
            for k in 0..s.spans() {
                let t = s.span_trace(k).expect("traces");
                t.check_tiles().unwrap_or_else(|e| panic!("{codec:?} span {k}: {e}"));
                assert_eq!(t.start().1, s.span_bytes(k).start);
                assert_eq!(t.out_bytes(), s.span_bytes(k).end);
            }
            // In order, so that the one span trace kept is the one asked for
            // next nearly every time.
            let mut rand = rand_from(17);
            let mut bytes: Vec<u64> = (0..2000).map(|_| rand() % data.len() as u64).collect();
            let mut bits: Vec<u64> = (0..2000).map(|_| rand() % (packed.len() as u64 * 8)).collect();
            bytes.sort_unstable();
            bits.sort_unstable();
            for byte in bytes {
                assert_eq!(s.map_out(byte), whole.map_out(byte), "{codec:?}: byte {byte}");
            }
            for bit in bits {
                assert_eq!(s.map_in(bit), whole.map_in(bit), "{codec:?}: bit {bit}");
            }
            // Every checkpoint's own bit, and the bits either side, which is
            // where a span's trace and the one before it meet.
            for (in_bit, _, block) in s.marks() {
                for bit in [in_bit.saturating_sub(1), in_bit, in_bit + 1].into_iter().chain(block) {
                    assert_eq!(s.map_in(bit), whole.map_in(bit), "{codec:?}: bit {bit} by checkpoint {in_bit}");
                }
            }
        }
    }

    /// The sidebar's take on one code, rebuilt from a span's trace, says what
    /// it says when rebuilt from the whole trace.
    #[test]
    fn a_code_taken_apart_through_a_span_reads_as_through_the_whole() {
        let data = mixed(300_000, 13);
        let packed = miniz_oxide::deflate::compress_to_vec(&data, 6);
        let (_, whole) = crate::codec::decode_traced(Codec::Deflate, &packed).expect("decodes whole");
        let s = indexed(Codec::Deflate, &packed, small());
        let mut rand = rand_from(23);
        let mut bits: Vec<u64> = (0..3000).map(|_| rand() % (packed.len() as u64 * 8)).collect();
        bits.sort_unstable();
        let mut compared = 0;
        for bit in bits {
            let t = s.trace_at_bit(bit).expect("a span names it");
            let (Some(ours), Some(theirs)) = (t.index_in(bit), whole.index_in(bit)) else { continue };
            let a = inflate::decode_step(&packed, &t, ours);
            let b = inflate::decode_step(&packed, &whole, theirs);
            // The block number and the step that declared a code length are
            // counted in the trace they came from, and the two traces start
            // in different places. The rest, the step's place in its block
            // included, is the same code read the same way.
            let same = |d: inflate::DecodedStep| {
                let plain = |c: inflate::DecodedCode| inflate::DecodedCode { entry: None, ..c };
                (plain(d.symbol), d.distance.map(plain), d.block_kind)
            };
            assert_eq!(a.map(same), b.map(same), "bit {bit}");
            compared += a.is_some() as usize;
        }
        assert!(compared > 500, "only {compared} codes compared");
    }

    /// The blocks found on the index pass are the blocks of the whole trace,
    /// one for one: where each starts and ends in the run and in the output,
    /// how it is coded and whether it is the last. And each block's own trace
    /// holds the same steps as the whole trace does over that block.
    #[test]
    fn the_blocks_of_an_index_are_the_blocks_of_the_whole_trace() {
        let data = mixed(300_000, 29);
        // Level 0 writes stored blocks, so checkpoints land at block starts;
        // the others put them inside Huffman-coded blocks. Flushing makes
        // hundreds of blocks of every kind.
        let mut most = 0;
        for level in [0u8, 1, 6] {
            for (codec, packed) in [
                (Codec::Deflate, miniz_oxide::deflate::compress_to_vec(&data, level)),
                (Codec::Zlib, miniz_oxide::deflate::compress_to_vec_zlib(&data, level)),
                (Codec::Deflate, flushed(&data, level, 3000)),
                (Codec::Gzip, gzip_around(&data, &flushed(&data, level, 5000))),
            ] {
                let (_, whole) = crate::codec::decode_traced(codec, &packed).expect("decodes whole");
                let s = indexed(codec, &packed, small());
                assert_eq!(s.blocks(), whole.blocks().len() as u64, "{codec:?} level {level}");
                most = most.max(s.blocks());
                for (i, want) in whole.blocks().iter().enumerate() {
                    let got = s.block(i as u64).expect("a block");
                    assert_eq!(
                        (got.kind, got.last, &got.in_bits, &got.out_bytes),
                        (want.kind, want.last, &want.in_bits, &want.out_bytes),
                        "{codec:?} level {level}: block {i}"
                    );
                    for bit in [want.in_bits.start, want.in_bits.end - 1] {
                        assert_eq!(s.block_of_bit(bit), Some(i as u64), "{codec:?} level {level}: bit {bit}");
                    }
                    let t = s.block_trace(i as u64).expect("traces");
                    t.check_tiles().unwrap_or_else(|e| panic!("{codec:?} level {level} block {i}: {e}"));
                    let ours: Vec<Step> = t.steps().take(t.blocks()[0].steps.len()).collect();
                    let theirs: Vec<Step> = want.steps.clone().map(|k| whole.step(k as usize).unwrap()).collect();
                    assert!(ours == theirs, "{codec:?} level {level}: block {i}'s steps differ");
                }
                assert_eq!(s.block(s.blocks()).err(), Some(Refusal::Failed));
                // Bits outside every block: the wrapper's, and the padding.
                let first = whole.blocks()[0].in_bits.start;
                if first > 0 {
                    assert_eq!(s.block_of_bit(first - 1), None);
                }
                assert_eq!(s.block_of_bit(whole.blocks().last().unwrap().in_bits.end), None);
            }
        }
        assert!(most > 100, "at most {most} blocks");
    }

    /// Past the most block starts there may be, every second one goes and
    /// the ones between are found again by decoding: every block still reads
    /// as the whole trace has it, and no more starts are kept than the limit.
    #[test]
    fn block_starts_thin_out_and_the_rest_are_found_again() {
        let data = mixed(300_000, 31);
        let packed = flushed(&data, 1, 2000);
        let (_, whole) = crate::codec::decode_traced(Codec::Deflate, &packed).expect("decodes whole");
        let n = whole.blocks().len();
        let limits = Limits { block_marks: 4, ..small() };
        let s = indexed(Codec::Deflate, &packed, limits);
        assert!(n > 16, "only {n} blocks");
        assert!(s.blocks.list.len() <= 4 && s.blocks.stride > 1, "{} starts kept {} apart", s.blocks.list.len(), s.blocks.stride);
        assert_eq!(s.blocks(), n as u64);
        // Backwards, so that the groups turn over.
        for (i, want) in whole.blocks().iter().enumerate().rev() {
            let got = s.block(i as u64).expect("a block");
            assert_eq!((&got.in_bits, &got.out_bytes, got.kind), (&want.in_bits, &want.out_bytes, want.kind), "block {i}");
            let mid = (want.in_bits.start + want.in_bits.end) / 2;
            assert_eq!(s.block_of_bit(mid), Some(i as u64), "bit {mid}");
        }
    }

    /// Past the limit for a run read a piece at a time, a run is refused with
    /// the refusal that says so, after a pass that kept no more than the
    /// checkpoints.
    #[test]
    fn a_run_past_the_limit_is_refused() {
        let zeros = vec![0u8; 5 << 20];
        let packed = miniz_oxide::deflate::compress_to_vec(&zeros, 9);
        let limits = Limits { lazy: 4 << 20, ..small() };
        assert_eq!(decode_or_index_with(Codec::Deflate, Arc::new(packed), limits).err(), Some(Refusal::TooLargeLazy));
    }

    /// A trailer that disagrees with what came out is refused, as it is for a
    /// run decoded whole, and so is a run cut short.
    #[test]
    fn a_wrong_trailer_or_a_short_run_is_refused() {
        let data = mixed(200_000, 19);
        let mut gzip = gzip_of(&data, 6);
        let n = gzip.len();
        gzip[n - 8] ^= 1;
        assert_eq!(decode_or_index_with(Codec::Gzip, Arc::new(gzip), small()).err(), Some(Refusal::Failed));
        let mut zlib = miniz_oxide::deflate::compress_to_vec_zlib(&data, 6);
        let n = zlib.len();
        zlib[n - 1] ^= 1;
        assert_eq!(decode_or_index_with(Codec::Zlib, Arc::new(zlib), small()).err(), Some(Refusal::Failed));
        let raw = miniz_oxide::deflate::compress_to_vec(&data, 6);
        let cut = raw[..raw.len() * 2 / 3].to_vec();
        assert_eq!(decode_or_index_with(Codec::Deflate, Arc::new(cut), small()).err(), Some(Refusal::Failed));
    }
}
