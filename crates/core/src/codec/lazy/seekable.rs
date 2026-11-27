//! Reading an indexed run: its bytes, the traces of its spans and blocks, and
//! the caches that hold them.

use std::sync::{Arc, Mutex, MutexGuard};

use super::deflate::{BlockMarks, WindowSink};
use super::input::{refused_as_none, Copied, ReadError, RunInput};
use super::{Checkpoint, Limits, GROUPS_KEPT};
use crate::codec::inflate::{self, BlockStart, CountSink, Ended, Framing, Input, Resume, Sink, Wrapper, MAX_HEAD_BYTES};
use crate::codec::{xz, Block, BlockKind, Codec, Member, Refusal, Step, StepField, StepKind, Trace, TraceBuilder, Window};
use crate::source::Missing;

/// A compressed run that comes to more than can be held whole, read through
/// its checkpoints.
///
/// Holds the checkpoints; where the blocks start; the spans decoded most
/// recently, up to [`Limits::span_cache`] (for xz, as much of each as reads
/// have reached); the trace of the span asked about most recently; and the
/// traces of the blocks asked about most recently, up to
/// [`Limits::span_steps`] steps between them. Reads take `&self`, as a
/// [`Source`](crate::source::Source) is read, so the caches are behind locks.
///
/// The run's compressed bytes are read through a [`RunInput`], by each read
/// as it needs them: a span's, a block's, or the stretch between two kept
/// block starts. A read that finds chunks of them missing answers
/// [`ReadError::Missing`] and caches nothing. The input is a handle onto
/// the source the run is in, the file's chunk store in the browser, when one
/// reaches the run, and otherwise a copy of the run, at most [`CAP_BYTES`],
/// which never finds a chunk missing. The index pass reads the run through
/// the same input a window at a time, or reads a copy of its own when the
/// input's source keeps too little for a window ([`super::reads_in_windows`]).
/// See `docs/DESIGN-lazy-compressed-input.md`.
///
/// [`CAP_BYTES`]: crate::codec::CAP_BYTES
pub struct Seekable {
    codec: Codec,
    /// Where the run's compressed bytes are read from. The bits of every
    /// trace count from its first byte.
    input: RunInput,
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
    groups: Mutex<Recent<Arc<[BlockStart]>>>,
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
pub(super) fn seekable(codec: Codec, input: RunInput, kind: Kind, len: u64, checkpoints: Vec<Checkpoint>, limits: Limits) -> Seekable {
    Seekable {
        codec,
        input,
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
            .field("input", &self.input.len())
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

    /// Where the compressed run is read from. Its first byte is what the
    /// bits of every trace count from.
    pub fn input(&self) -> &RunInput {
        &self.input
    }

    /// The most output this index keeps decoded at once, when it reads its
    /// run from a source that keeps only part of the file: its span cache.
    /// A read of more output than that in one call decodes spans that push
    /// out the ones before them, whose compressed bytes the store may have
    /// pushed out too, and would ask for them again for ever. `None` when
    /// the run is read from a source that answers any read whole.
    pub fn resident_limit(&self) -> Option<u64> {
        self.input.resident_limit().map(|_| self.limits.span_cache as u64)
    }

    /// Read the run through `input` from now on, in place of the input the
    /// index pass gave it: for a test over a source that loses chunks.
    #[cfg(test)]
    pub(crate) fn set_input(&mut self, input: RunInput) {
        assert_eq!(input.len(), self.input.len(), "the same run");
        self.input = input;
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
        if bit >= self.input.len() * 8 {
            return None;
        }
        Some(self.checkpoints.partition_point(|c| c.in_bit <= bit).saturating_sub(1))
    }

    /// Bytes `at..at + out.len()` of the output.
    ///
    /// Each span the read reaches is decoded from its compressed bytes, read
    /// through the run's [`RunInput`] first. When chunks of them are not
    /// loaded, the read stops at that span and names them all, and nothing
    /// of the span is decoded or cached; spans decoded before it stay in the
    /// cache, so the read made again once the chunks are there does not
    /// decode them again.
    pub fn read(&self, at: u64, out: &mut [u8]) -> Result<(), ReadError> {
        let end = at.checked_add(out.len() as u64).ok_or(Refusal::Failed)?;
        if end > self.len {
            return Err(Refusal::Failed.into());
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
                Kind::Xz(l) => {
                    let most = match self.input.resident_limit() {
                        Some(_) => self.limits.span_input,
                        None => usize::MAX,
                    };
                    super::xz::read_span_through(&self.prefixes, &self.input, l, self.span_units(k, l), k, from, dst, most)?
                }
                _ => dst.copy_from_slice(&self.span(k)?[from..from + take]),
            }
            pos += take as u64;
        }
        Ok(())
    }

    /// The bytes of the run span `k` reads, for a span that starts on a byte:
    /// from its checkpoint's bit to the next one's, and the last to the end.
    pub(super) fn span_in(&self, k: usize) -> std::ops::Range<usize> {
        let end = self.checkpoints.get(k + 1).map_or(self.input.len() as usize, |c| (c.in_bit / 8) as usize);
        (self.checkpoints[k].in_bit / 8) as usize..end
    }

    /// The bytes of the run zstd span `k`'s frames take: from where its first
    /// frame starts itself, past the skippable frames in front of it, to the
    /// next checkpoint. What a read of its bytes reads.
    pub(super) fn frames_in(&self, k: usize) -> std::ops::Range<usize> {
        let range = self.span_in(k);
        self.checkpoints[k].own().unwrap_or(range.start).min(range.end)..range.end
    }

    /// Refused as [`Refusal::TooMuchAtOnce`] when a span read would gather
    /// more than [`Limits::span_input`] of the run at once from a source that
    /// keeps only part of the file. Only thinning makes a zstd or xz span
    /// that large; a deflate span never is.
    fn at_once(&self, bytes: usize) -> Result<(), Refusal> {
        match self.input.too_much(bytes, self.limits.span_input) {
            true => Err(Refusal::TooMuchAtOnce),
            false => Ok(()),
        }
    }

    /// The xz blocks span `k` holds, as numbers in the stream's index.
    pub(super) fn span_units(&self, k: usize, l: &xz::Layout) -> std::ops::Range<usize> {
        let end = self.checkpoints.get(k + 1).map_or(l.blocks.len(), |c| c.unit());
        self.checkpoints[k].unit()..end
    }

    /// Everything span `k` comes to, from the cache or decoded now.
    fn span(&self, k: usize) -> Result<Arc<Vec<u8>>, ReadError> {
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
    /// as the read reaches, through `prefixes`. See
    /// [`super::xz::read_span_through`].
    fn decode_span(&self, k: usize) -> Result<Vec<u8>, ReadError> {
        let range = self.span_bytes(k);
        let want = (range.end - range.start) as usize;
        let bytes = match &self.kind {
            Kind::Deflate(_) => {
                let copies = self.deflate_span_copies(k)?;
                self.deflate_span(k, copies.input(self.input.len()))?
            }
            Kind::Zstd => {
                let (range, out) = (self.frames_in(k), self.checkpoints[k].out);
                self.at_once(range.len())?;
                let copy = self.input.copy(range.clone())?;
                super::zstd::decode_span(copy.window(self.input.len()), range, out, want, self.limits.span)?
            }
            Kind::Xz(_) => return Err(Refusal::Failed.into()),
        };
        if bytes.len() != want {
            return Err(Refusal::Failed.into());
        }
        Ok(bytes)
    }

    /// What a decode or a trace of deflate span `k` reads, copied through
    /// the run's input: from the byte its checkpoint's bit is in to the byte
    /// holding the bit before the next checkpoint's (or to the run's end, for
    /// the last span). The header of the block the checkpoint is inside is
    /// the checkpoint's own copy.
    fn deflate_span_copies(&self, k: usize) -> Result<DeflateCopies<'_>, Vec<Missing>> {
        let c = &self.checkpoints[k];
        let to = self.checkpoints.get(k + 1).map_or(self.input.len() as usize, |n| n.in_bit.div_ceil(8) as usize);
        let code = self.input.copy((c.in_bit / 8) as usize..to)?;
        Ok(DeflateCopies { code, head: c.head() })
    }

    /// Deflate span `k`, decoded from `input`: from checkpoint `k`'s bit to
    /// the next checkpoint's, and the header of the block the checkpoint is
    /// inside. See [`inflate::run`] for what `input` has to hold.
    pub(super) fn deflate_span(&self, k: usize, input: Input<'_>) -> Result<Vec<u8>, Refusal> {
        let f = self.framing()?;
        let c = &self.checkpoints[k];
        let range = self.span_bytes(k);
        let want = (range.end - range.start) as usize;
        let window = c.window();
        let mut hist = Vec::with_capacity(window.len() + want);
        hist.extend_from_slice(window);
        let mut sink = WindowSink::new(hist, c.out - window.len() as u64, range.end, false);
        sink.stop_at = self.checkpoints.get(k + 1).map(|n| n.in_bit);
        inflate::run(input, f.start, f.end, &mut sink, &mut TraceBuilder::off(), Some(c.resume()))?;
        if sink.len() != range.end {
            return Err(Refusal::Failed);
        }
        let mut bytes = sink.hist;
        bytes.drain(..window.len());
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
    ///
    /// The span's compressed bytes are read through the run's input first,
    /// and a span missing chunks of them is traced once they are there.
    pub fn span_trace(&self, k: usize) -> Result<Arc<Trace>, ReadError> {
        if let Some((held, t)) = lock(&self.traced).as_ref() {
            if *held == k {
                return Ok(t.clone());
            }
        }
        let c = self.checkpoints.get(k).ok_or(Refusal::Failed)?;
        let len = self.input.len();
        let t = Arc::new(match &self.kind {
            Kind::Deflate(_) => self.deflate_span_trace(k, self.deflate_span_copies(k)?.input(len))?,
            Kind::Zstd => {
                // The skippable frames in front of the span's first frame are
                // read a header at a time, and their first piece with the
                // frames, so that a span behind a short one reads at once.
                let (front, frames) = (self.span_in(k).start, self.frames_in(k));
                self.at_once(frames.len())?;
                let first = (front < frames.start).then(|| front..frames.start.min(front + super::zstd::FRONT_PIECE));
                let [copy, first] = self.input.copies([Some(frames.clone()), first])?;
                let skips = super::zstd::skippable_starts(&self.input, front, frames.start, first)?;
                super::zstd::span_trace(copy.expect("asked for").window(len), front, &skips, frames, c.out, self.limits.span)?
            }
            Kind::Xz(l) => {
                let range = self.span_in(k);
                self.at_once(range.len())?;
                let copy = self.input.copy(range.clone())?;
                super::xz::span_trace(copy.window(len), l, k, range, self.span_units(k, l), c.out)?
            }
        });
        *lock(&self.traced) = Some((k, t.clone()));
        Ok(t)
    }

    /// The trace of deflate span `k`, decoded from `input`, which holds what
    /// [`Seekable::deflate_span`] needs.
    pub(super) fn deflate_span_trace(&self, k: usize, input: Input<'_>) -> Result<Trace, Refusal> {
        let f = self.framing()?;
        let c = &self.checkpoints[k];
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
        let ended = inflate::run(input, f.start, f.end, &mut sink, &mut b, Some(c.resume()))?;
        if matches!(ended, Ended::Stream(_)) && wrapped {
            b.push(f.end, sink.len, StepKind::Header(StepField::Wrapper, 0));
            b.finish_at(input.code().run_len() as u64 * 8, sink.len);
        }
        Ok(b.done())
    }

    /// The trace that holds output byte `byte`: its span's. Nothing past the
    /// end, or when the decoder refuses the span; the chunks of the file the
    /// span's compressed bytes are missing, when they are.
    pub fn trace_at_byte(&self, byte: u64) -> Result<Option<Arc<Trace>>, Vec<Missing>> {
        match self.span_of_byte(byte) {
            Some(k) => refused_as_none(self.span_trace(k)),
            None => Ok(None),
        }
    }

    /// The trace that names bit `bit` of the run: the span's that read it.
    /// Nothing and missing chunks as for [`Seekable::trace_at_byte`].
    pub fn trace_at_bit(&self, bit: u64) -> Result<Option<Arc<Trace>>, Vec<Missing>> {
        match self.span_of_bit(bit) {
            Some(k) => refused_as_none(self.span_trace(k)),
            None => Ok(None),
        }
    }

    /// Which step produced byte `byte` of the output, as
    /// [`Trace::map_out`] says for a whole run.
    pub fn map_out(&self, byte: u64) -> Result<Option<Step>, Vec<Missing>> {
        Ok(self.trace_at_byte(byte)?.and_then(|t| t.map_out(byte)))
    }

    /// Which step read bit `bit` of the run, as [`Trace::map_in`] says for a
    /// whole run. Asked of the trace of the block holding the bit, which is
    /// the trace the listing and the inspector take a code from, and of the
    /// span's trace for a bit outside every block.
    pub fn map_in(&self, bit: u64) -> Result<Option<Step>, Vec<Missing>> {
        let trace = match self.block_of_bit(bit)? {
            Some(i) => refused_as_none(self.block_trace(i))?,
            None => self.trace_at_bit(bit)?,
        };
        Ok(trace.and_then(|t| t.map_in(bit)))
    }

    /// How many blocks the run holds.
    pub fn blocks(&self) -> u64 {
        self.blocks.count
    }

    /// Block `i`: where it is in the run and in the output, how it is coded
    /// and whether it says it is the last. Its `steps` are empty; they are in
    /// [`Seekable::block_trace`].
    ///
    /// The pass kept what each kept start's header says, so this reads none
    /// of the run, unless block starts were thinned and the start of block
    /// `i` or `i + 1` is found again by decoding (see [`Seekable::group`]).
    pub fn block(&self, i: u64) -> Result<Block, ReadError> {
        let start = self.block_start(i)?;
        let (in_end, out_end) = match i + 1 < self.blocks.count {
            true => self.block_start(i + 1).map(|s| (s.bit(), s.out))?,
            false => (self.blocks_end, self.len),
        };
        let (last, btype) = start.head();
        let kind = match btype {
            0 => BlockKind::Stored,
            1 => BlockKind::Fixed,
            2 => BlockKind::Dynamic,
            _ => return Err(Refusal::Failed.into()),
        };
        Ok(Block { steps: 0..0, kind, last, in_bits: start.bit()..in_end, out_bytes: start.out..out_end })
    }

    /// The block that bit `bit` of the run belongs to. None for a bit of the
    /// gzip or zlib wrapper, or of the padding after the last block, and
    /// when the decoder refuses the stretch of the run the bit is in. Once
    /// block starts are thinned, finding the block reads that stretch, and
    /// the chunks of the file it is missing are the error.
    pub fn block_of_bit(&self, bit: u64) -> Result<Option<u64>, Vec<Missing>> {
        let list = &self.blocks.list;
        match list.first() {
            Some(first) if bit >= first.bit() && bit < self.blocks_end => {}
            _ => return Ok(None),
        }
        let g = list.partition_point(|m| m.bit() <= bit) - 1;
        if self.blocks.stride == 1 {
            return Ok(Some(g as u64));
        }
        let Some(starts) = refused_as_none(self.group(g))? else { return Ok(None) };
        Ok(Some(g as u64 * self.blocks.stride + starts.partition_point(|m| m.bit() <= bit) as u64 - 1))
    }

    /// Where the deflate data is inside the run's wrapper. Refused for a zstd
    /// or xz run, which has no deflate blocks.
    fn framing(&self) -> Result<&Framing, Refusal> {
        match &self.kind {
            Kind::Deflate(f) => Ok(f),
            Kind::Zstd | Kind::Xz(_) => Err(Refusal::Failed),
        }
    }

    /// Where block `i` starts, and what its header says.
    fn block_start(&self, i: u64) -> Result<BlockStart, ReadError> {
        if i >= self.blocks.count {
            return Err(Refusal::Failed.into());
        }
        let stride = self.blocks.stride;
        let (g, j) = ((i / stride) as usize, (i % stride) as usize);
        if j == 0 {
            return Ok(self.blocks.list[g]);
        }
        Ok(self.group(g)?.get(j).copied().ok_or(Refusal::Failed)?)
    }

    /// The start of every block from kept start `g` up to the next kept start,
    /// found by decoding from start `g` and counting the output. The run is
    /// read through its input from start `g` to the next kept start, or to
    /// the run's end. A stretch past [`Limits::unit_input`] is refused as
    /// [`Refusal::TooMuchAtOnce`] when the source keeps only part of the
    /// file.
    fn group(&self, g: usize) -> Result<Arc<[BlockStart]>, ReadError> {
        if let Some(starts) = lock(&self.groups).get(g as u64) {
            return Ok(starts);
        }
        let in_bit = self.blocks.list.get(g).ok_or(Refusal::Failed)?.bit();
        let to = self.blocks.list.get(g + 1).map_or(self.input.len() as usize, |m| m.bit().div_ceil(8) as usize);
        // Only a hostile run, of more blocks than are kept, has a long
        // stretch between two kept starts.
        if self.input.too_much(to - (in_bit / 8) as usize, self.limits.unit_input) {
            return Err(Refusal::TooMuchAtOnce.into());
        }
        let code = self.input.copy((in_bit / 8) as usize..to)?;
        let starts: Arc<[BlockStart]> = self.group_from(g, Input::new(code.window(self.input.len()), None))?.into();
        lock(&self.groups).put(g as u64, starts.clone(), 1);
        Ok(starts)
    }

    /// [`Seekable::group`] decoded from `input`, which has to hold the run
    /// from kept start `g` to the next kept start. The list is not cached.
    pub(super) fn group_from(&self, g: usize, input: Input<'_>) -> Result<Vec<BlockStart>, Refusal> {
        let &kept = self.blocks.list.get(g).ok_or(Refusal::Failed)?;
        let (in_bit, out) = (kept.bit(), kept.out);
        let mut sink = CountSink { len: out, cap: u64::MAX, stop_at: self.blocks.list.get(g + 1).map(|m| m.bit()), starts: Some(Vec::new()), stopped: None };
        let f = self.framing()?;
        inflate::run(input, f.start, f.end, &mut sink, &mut TraceBuilder::off(), Some(Resume { in_bit, block: None }))?;
        let starts = sink.starts.take().unwrap_or_default();
        // The same decoder read these bits on the index pass, so a different
        // count is the decoder disagreeing with itself.
        let want = self.blocks.stride.min(self.blocks.count - g as u64 * self.blocks.stride);
        if starts.len() as u64 != want || starts.first() != Some(&kept) {
            return Err(Refusal::Failed);
        }
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
    /// [`crate::codec::MAX_STEPS`]. The decode stops where the trace turns
    /// coarse, and the opaque step runs on to the block's end, which the
    /// index knows.
    ///
    /// The block is read through the run's input, from its first byte to the
    /// byte holding its last bit, or for a Huffman-coded block, as far as
    /// [`Limits::span_steps`] codes reach at most: [`MAX_HEAD_BYTES`] for its
    /// header and tables, and 48 bits a code. So under the default limits a
    /// block's trace reads at most about 6 MB of the run.
    pub fn block_trace(&self, i: u64) -> Result<Arc<Trace>, ReadError> {
        if let Some(t) = lock(&self.block_traces).get(i) {
            return Ok(t);
        }
        let block = self.block(i)?;
        let from = (block.in_bits.start / 8) as usize;
        let mut to = block.in_bits.end.div_ceil(8) as usize;
        if block.kind != BlockKind::Stored {
            to = to.min(from + MAX_HEAD_BYTES + self.limits.span_steps.saturating_mul(6));
        }
        let code = self.input.copy(from..to)?;
        let t = Arc::new(self.block_trace_from(i, Input::new(code.window(self.input.len()), None), true)?);
        lock(&self.block_traces).put(i, t.clone(), t.len());
        Ok(t)
    }

    /// [`Seekable::block_trace`] decoded from `input`, which has to hold
    /// block `i`, or with `stop`, as much of it as is decoded before the
    /// trace turns coarse. The trace is not cached. Without `stop`, a coarse
    /// block is decoded to its end, which gives the same trace.
    pub(super) fn block_trace_from(&self, i: u64, input: Input<'_>, stop: bool) -> Result<Trace, ReadError> {
        let block = self.block(i)?;
        let (in_bit, out) = (block.in_bits.start, block.out_bytes.start);
        let next = (i + 1 < self.blocks.count).then_some(block.in_bits.end);
        let mut sink = CountSink { len: out, cap: u64::MAX, stop_at: next, starts: None, stopped: None };
        let mut b = TraceBuilder::with_budget(self.limits.span_steps).starting_at(in_bit, out);
        if stop {
            b = b.stopping_when_coarse();
        }
        let f = self.framing()?;
        if let Ended::Coarse(_) = inflate::run(input, f.start, f.end, &mut sink, &mut b, Some(Resume { in_bit, block: None }))? {
            // What a decode to the block's end would have done: the opaque
            // step runs to the block's end, and so does the trace, or for the
            // last block, on to the end of the deflate data.
            b.close_block(block.in_bits.end, block.out_bytes.end, block.kind, block.last);
            match next {
                Some(end) => b.finish_at(end, block.out_bytes.end),
                None => inflate::stream_tail(&mut b, block.in_bits.end, f.end, block.out_bytes.end),
            }
        }
        let t = b.done();
        match t.blocks() {
            [one] if one.in_bits == block.in_bits && one.out_bytes == block.out_bytes => Ok(t),
            _ => Err(Refusal::Failed.into()),
        }
    }

    /// How many bytes the checkpoints' windows and header copies, the block
    /// starts and the span cache hold, for tests and for saying what a run
    /// costs. Checkpoints inside one block share a header copy, which counts
    /// for each of them.
    pub fn held_bytes(&self) -> usize {
        self.checkpoints.iter().map(|c| c.window().len() + c.head().map_or(0, |h| h.0.len())).sum::<usize>()
            + self.blocks.list.len() * std::mem::size_of::<BlockStart>()
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

/// What a deflate decode reads: its codes, copied through the run's input,
/// and the checkpoint's copy of the header and tables of the block it starts
/// inside, when it starts inside one, with the byte of the run that starts
/// at.
struct DeflateCopies<'a> {
    code: Copied,
    head: Option<(&'a [u8], usize)>,
}

impl DeflateCopies<'_> {
    /// The copies as the input [`inflate::run`] reads, in a run `run_len`
    /// bytes long.
    fn input(&self, run_len: u64) -> Input<'_> {
        let head = self.head.and_then(|(bytes, at)| Window::of(bytes, at, run_len as usize));
        Input::new(self.code.window(run_len), head)
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
