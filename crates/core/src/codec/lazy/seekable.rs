//! Reading an indexed run: its bytes, the traces of its spans and blocks, and
//! the caches that hold them.

use std::sync::{Arc, Mutex, MutexGuard};

use super::deflate::{BlockMarks, WindowSink};
use super::{Checkpoint, Limits, GROUPS_KEPT};
use crate::codec::inflate::{self, CountSink, Ended, Framing, Resume, Sink, Wrapper};
use crate::codec::{xz, Block, BlockKind, Codec, Member, Refusal, Step, StepField, StepKind, Trace, TraceBuilder};

/// A compressed run that comes to more than can be held whole, read through
/// its checkpoints.
///
/// Holds the run itself, which is at most [`CAP_BYTES`]; the checkpoints; where
/// the blocks start; the spans decoded most recently, up to
/// [`Limits::span_cache`] (for xz, as much of each as reads have reached);
/// the trace of the span asked about most recently;
/// and the traces of the blocks asked about most recently, up to
/// [`Limits::span_steps`] steps between them. Reads take `&self`, as a
/// [`Source`](crate::source::Source) is read, so the caches are behind locks.
///
/// [`CAP_BYTES`]: crate::codec::CAP_BYTES
pub struct Seekable {
    codec: Codec,
    packed: Arc<Vec<u8>>,
    kind: Kind,
    len: u64,
    pub(super) checkpoints: Vec<Checkpoint>,
    /// Where each deflate block starts. Empty for zstd and xz.
    pub(super) blocks: BlockMarks,
    /// Where the last deflate block ends, as a bit of the run.
    pub(super) blocks_end: u64,
    pub(super) crc32: Option<u32>,
    pub(super) adler32: Option<u32>,
    limits: Limits,
    pub(super) spans: Mutex<Recent<Arc<Vec<u8>>>>,
    /// An xz run's spans, each decoded as far as reads have reached it, up to
    /// [`Limits::span_cache`]. Empty for deflate and zstd, whose spans are
    /// decoded whole into `spans`.
    pub(super) prefixes: Mutex<Recent<super::xz::Prefix>>,
    traced: Mutex<Option<(usize, Arc<Trace>)>>,
    /// Block starts found again by decoding: one list for each kept start,
    /// keyed by that start's place in `blocks.list`.
    groups: Mutex<Recent<Arc<[(u64, u64)]>>>,
    block_traces: Mutex<Recent<Arc<Trace>>>,
}

/// Which decoder a run's spans go through, and what it needs to know about the
/// run as a whole.
pub(super) enum Kind {
    /// Where the deflate data starts and ends inside the run's wrapper.
    Deflate(Framing),
    Zstd,
    /// Every block, from the stream's index.
    Xz(xz::Layout),
}

/// What a run of no checkpoints but those it was made with comes to: no
/// spans cached and no span trace made yet, and no deflate blocks.
pub(super) fn seekable(codec: Codec, packed: Arc<Vec<u8>>, kind: Kind, len: u64, checkpoints: Vec<Checkpoint>, limits: Limits) -> Seekable {
    Seekable {
        codec,
        packed,
        kind,
        len,
        checkpoints,
        blocks: BlockMarks::new(limits.block_marks),
        blocks_end: 0,
        crc32: None,
        adler32: None,
        limits,
        spans: Mutex::new(Recent::new(limits.span_cache)),
        prefixes: Mutex::new(Recent::new(limits.span_cache)),
        traced: Mutex::new(None),
        groups: Mutex::new(Recent::new(GROUPS_KEPT)),
        block_traces: Mutex::new(Recent::new(limits.span_steps)),
    }
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
pub(super) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
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

    /// The CRC-32 of the whole output, worked out on the pass that indexed
    /// it. Only a deflate run's pass sums its output, since gzip's trailer
    /// holds this sum; nothing for zstd and xz.
    pub fn crc32(&self) -> Option<u32> {
        self.crc32
    }

    /// The Adler-32 of the whole output, the same way, which zlib's trailer
    /// holds.
    pub fn adler32(&self) -> Option<u32> {
        self.adler32
    }

    /// The container's own pieces of the run, as a whole trace would declare
    /// them: an xz stream's blocks, which the index gives without decoding
    /// any. See [`Member`]. Empty for every other codec.
    pub fn members(&self) -> Vec<Member> {
        match &self.kind {
            Kind::Xz(l) => xz::members(l),
            Kind::Deflate(_) | Kind::Zstd => Vec::new(),
        }
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
            let take = (range.end.min(end) - pos) as usize;
            let from = (pos - range.start) as usize;
            let to = (pos - at) as usize;
            let dst = &mut out[to..to + take];
            match &self.kind {
                // Decoded only as far as this read reaches.
                Kind::Xz(l) => super::xz::read_span(&self.prefixes, &self.packed, l, self.span_units(k, l), k, from, dst)?,
                _ => dst.copy_from_slice(&self.span(k)?[from..from + take]),
            }
            pos += take as u64;
        }
        Ok(())
    }

    /// The bytes of the run span `k` reads, for a span that starts on a byte:
    /// from its checkpoint's bit to the next one's, and the last to the end.
    fn span_in(&self, k: usize) -> std::ops::Range<usize> {
        let end = self.checkpoints.get(k + 1).map_or(self.packed.len(), |c| (c.in_bit / 8) as usize);
        (self.checkpoints[k].in_bit / 8) as usize..end
    }

    /// The xz blocks span `k` holds, as numbers in the stream's index.
    fn span_units(&self, k: usize, l: &xz::Layout) -> std::ops::Range<usize> {
        let end = self.checkpoints.get(k + 1).map_or(l.blocks.len(), |c| c.unit());
        self.checkpoints[k].unit()..end
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

    /// Decode deflate or zstd span `k` from its own checkpoint to the next
    /// one. It is the same decoder over the same bits as the pass that
    /// indexed it, so a span that comes out another length is a decoder
    /// disagreeing with itself.
    ///
    /// An xz span is never decoded whole: a read of one decodes only as far
    /// as the read reaches, through `prefixes`. See [`super::xz::read_span`].
    fn decode_span(&self, k: usize) -> Result<Vec<u8>, Refusal> {
        let c = &self.checkpoints[k];
        let range = self.span_bytes(k);
        let want = (range.end - range.start) as usize;
        let bytes = match &self.kind {
            Kind::Deflate(f) => {
                let window = c.window();
                let mut hist = Vec::with_capacity(window.len() + want);
                hist.extend_from_slice(window);
                let mut sink = WindowSink::new(hist, c.out - window.len() as u64, range.end, false);
                sink.stop_at = self.checkpoints.get(k + 1).map(|n| n.in_bit);
                inflate::run(&self.packed, f.start, f.end, &mut sink, &mut TraceBuilder::off(), Some(c.resume()))?;
                if sink.len() != range.end {
                    return Err(Refusal::Failed);
                }
                let mut bytes = sink.hist;
                bytes.drain(..window.len());
                bytes
            }
            Kind::Zstd => {
                super::zstd::decode_span(&self.packed, self.span_in(k), c.out, want, self.limits.span)?
            }
            Kind::Xz(_) => return Err(Refusal::Failed),
        };
        if bytes.len() != want {
            return Err(Refusal::Failed);
        }
        Ok(bytes)
    }

    /// The trace of span `k`: every step from where its decode starts to
    /// where the next span's does, counted from the front of the run and of
    /// the output as a whole trace counts. Kept until another span's is asked
    /// for.
    ///
    /// A deflate decode starting inside a block reads the block's header and
    /// tables again, so the trace starts at that header and has them, and
    /// then one opaque step over the codes between the tables and the
    /// checkpoint, which made bytes of the span before. A zstd or xz span's
    /// trace is the one a whole run's has over the same frames or blocks,
    /// with the stream's header in span 0 and its index and footer in the
    /// last span.
    pub fn span_trace(&self, k: usize) -> Result<Arc<Trace>, Refusal> {
        if let Some((held, t)) = lock(&self.traced).as_ref() {
            if *held == k {
                return Ok(t.clone());
            }
        }
        let c = self.checkpoints.get(k).ok_or(Refusal::Failed)?;
        let t = Arc::new(match &self.kind {
            Kind::Deflate(f) => self.deflate_span_trace(k, c, f)?,
            Kind::Zstd => {
                super::zstd::span_trace(&self.packed, self.span_in(k), c.out, self.limits.span)?
            }
            Kind::Xz(l) => {
                super::xz::span_trace(&self.packed, l, k, self.span_in(k), self.span_units(k, l), c.out)?
            }
        });
        *lock(&self.traced) = Some((k, t.clone()));
        Ok(t)
    }

    fn deflate_span_trace(&self, k: usize, c: &Checkpoint, f: &Framing) -> Result<Trace, Refusal> {
        let wrapped = f.wrapper != Wrapper::Raw;
        let first = match (k, wrapped) {
            (0, true) => 0,
            _ => c.block().unwrap_or(c.in_bit),
        };
        let mut b = TraceBuilder::with_budget(self.limits.span_steps).starting_at(first, c.out);
        if k == 0 && wrapped {
            b.push(0, 0, StepKind::Header(StepField::Wrapper, 0));
        }
        let mut sink = CountSink { len: c.out, cap: u64::MAX, stop_at: self.checkpoints.get(k + 1).map(|n| n.in_bit), starts: None, stopped: None };
        let ended = inflate::run(&self.packed, f.start, f.end, &mut sink, &mut b, Some(c.resume()))?;
        if matches!(ended, Ended::Stream(_)) && wrapped {
            b.push(f.end, sink.len, StepKind::Header(StepField::Wrapper, 0));
            b.finish_at(self.packed.len() as u64 * 8, sink.len);
        }
        Ok(b.done())
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
    /// whole run. Asked of the trace of the block holding the bit, which is
    /// the trace the listing and the inspector take a code from, and of the
    /// span's trace for a bit outside every block.
    pub fn map_in(&self, bit: u64) -> Option<Step> {
        match self.block_of_bit(bit) {
            Some(i) => self.block_trace(i).ok()?.map_in(bit),
            None => self.trace_at_bit(bit)?.map_in(bit),
        }
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

    /// Where the deflate data is inside the run's wrapper. Refused for a zstd
    /// or xz run, which has no deflate blocks.
    fn framing(&self) -> Result<&Framing, Refusal> {
        match &self.kind {
            Kind::Deflate(f) => Ok(f),
            Kind::Zstd | Kind::Xz(_) => Err(Refusal::Failed),
        }
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
        let mut sink = CountSink { len: out, cap: u64::MAX, stop_at: self.blocks.list.get(g + 1).map(|m| m.0), starts: Some(Vec::new()), stopped: None };
        let f = self.framing()?;
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
        let mut sink = CountSink { len: out, cap: u64::MAX, stop_at: next, starts: None, stopped: None };
        let mut b = TraceBuilder::with_budget(self.limits.span_steps).starting_at(in_bit, out);
        let f = self.framing()?;
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
        self.checkpoints.iter().map(|c| c.window().len()).sum::<usize>()
            + self.blocks.list.len() * std::mem::size_of::<(u64, u64)>()
            + lock(&self.spans).weight
            + lock(&self.prefixes).weight
    }

    /// Where each checkpoint is, as the bit of the run and the byte of the
    /// output, for tests.
    #[cfg(test)]
    pub(super) fn marks(&self) -> Vec<(u64, u64, Option<u64>)> {
        self.checkpoints.iter().map(|c| (c.in_bit, c.out, c.block())).collect()
    }
}

/// A cache of what was made most recently for one run, up to a total weight,
/// that drops the entry used longest ago first. Spans are weighed in bytes,
/// block traces in steps, and each list of block starts counts as 1. The
/// newest entry is kept whatever it weighs.
pub(super) struct Recent<T> {
    entries: Vec<(u64, T, usize, u64)>,
    weight: usize,
    cap: usize,
    clock: u64,
    #[cfg(test)]
    pub(super) peak: usize,
}

impl<T: Clone> Recent<T> {
    fn get(&mut self, k: u64) -> Option<T> {
        self.clock += 1;
        let clock = self.clock;
        let e = self.entries.iter_mut().find(|e| e.0 == k)?;
        e.3 = clock;
        Some(e.1.clone())
    }
}

impl<T> Recent<T> {
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

    /// Take entry `k` out, to change it and put it back.
    pub(super) fn take(&mut self, k: u64) -> Option<T> {
        let i = self.entries.iter().position(|e| e.0 == k)?;
        let gone = self.entries.swap_remove(i);
        self.weight -= gone.2;
        Some(gone.1)
    }

    /// Keep `value` as entry `k`, in place of any entry `k` there is.
    pub(super) fn put(&mut self, k: u64, value: T, weight: usize) {
        self.take(k);
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
