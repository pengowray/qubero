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
//! The guard against a zip bomb is [`LAZY_CAP_BYTES`], a limit on the output.
//! Memory is bounded by the limits below whatever the output comes to, so a
//! bomb can only cost the time of one pass up to that limit.

use std::ops::ControlFlow;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::checksum::{adler32, adler32_update, crc32, crc32_update};
use crate::codec::inflate::{self, Ended, Framing, Resume, Sink, Wrapper};
use crate::codec::{Codec, Refusal, Step, StepField, StepKind, Trace, TraceBuilder, CAP_BYTES};

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
    /// [`SPAN_TRACE_STEPS`].
    pub span_steps: usize,
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
    drop(whole);
    let mut index = WindowSink::new(last.window.to_vec(), last.out - last.window.len() as u64, limits.lazy, true);
    index.marks = Some(marks);
    index.sums = Some(Sums { crc, adler, upto: last.out });
    match inflate::run(data, f.start, f.end, &mut index, &mut TraceBuilder::off(), Some(last.resume())) {
        Ok(_) => {}
        Err(Refusal::TooLarge) => return Err(Refusal::TooLargeLazy),
        Err(other) => return Err(other),
    }
    let len = index.len();
    index.sum_to(len);
    let sums = index.sums.expect("set above");
    if !inflate::trailer_agrees(&f, data, len, || sums.crc, || sums.adler) {
        return Err(Refusal::Failed);
    }
    let checkpoints = index.marks.take().expect("set above").list;
    Ok(Decoded::Indexed(Seekable {
        codec,
        packed,
        framing: f,
        len,
        checkpoints,
        crc32: sums.crc,
        adler32: sums.adler,
        limits,
        spans: Mutex::new(SpanCache::new(limits.span_cache)),
        traced: Mutex::new(None),
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
    fn offer(&mut self, in_bit: u64, out: u64, block: Option<u64>, window: &[u8]) {
        if out < self.next {
            return;
        }
        self.list.push(Checkpoint { in_bit, out, block, window: window.into() });
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
    /// Sum the output as it leaves.
    sums: Option<Sums>,
    /// Stop at the first boundary at or past this bit.
    stop_at: Option<u64>,
}

impl WindowSink {
    fn new(hist: Vec<u8>, base: u64, cap: u64, slide: bool) -> WindowSink {
        WindowSink { hist, base, cap, slide, marks: None, sums: None, stop_at: None }
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
                marks.offer(in_bit, out, block, &self.hist[self.hist.len() - keep..]);
            }
        }
        ControlFlow::Continue(())
    }
}

/// Output counted and not kept, for a decode that wants only its trace. A
/// deflate step says what it copied and from how far back, not what the
/// bytes were, so the trace of a span needs none of the output before it.
struct CountSink {
    len: u64,
    stop_at: Option<u64>,
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
    fn boundary(&mut self, in_bit: u64, _: Option<u64>) -> ControlFlow<()> {
        match self.stop_at.is_some_and(|stop| in_bit >= stop) {
            true => ControlFlow::Break(()),
            false => ControlFlow::Continue(()),
        }
    }
}

/// A compressed run that comes to more than can be held whole, read through
/// its checkpoints.
///
/// Holds the run itself, which is at most [`CAP_BYTES`]; the checkpoints; the
/// spans decoded most recently, up to [`Limits::span_cache`]; and the trace of
/// the span asked about most recently. Reads take `&self`, as a
/// [`Source`](crate::source::Source) is read, so the caches are behind locks.
pub struct Seekable {
    codec: Codec,
    packed: Arc<Vec<u8>>,
    framing: Framing,
    len: u64,
    checkpoints: Vec<Checkpoint>,
    crc32: u32,
    adler32: u32,
    limits: Limits,
    spans: Mutex<SpanCache>,
    traced: Mutex<Option<(usize, Arc<Trace>)>>,
}

impl std::fmt::Debug for Seekable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Seekable")
            .field("codec", &self.codec)
            .field("packed", &self.packed.len())
            .field("len", &self.len)
            .field("checkpoints", &self.checkpoints.len())
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
        if let Some(bytes) = lock(&self.spans).get(k) {
            return Ok(bytes);
        }
        let bytes = Arc::new(self.decode_span(k)?);
        lock(&self.spans).put(k, bytes.clone());
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
        let mut sink = CountSink { len: c.out, stop_at: self.checkpoints.get(k + 1).map(|n| n.in_bit) };
        let ended = inflate::run(&self.packed, f.start, f.end, &mut sink, &mut b, Some(c.resume()))?;
        if ended == Ended::Stream && wrapped {
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

    /// How many bytes the checkpoints' windows and the span cache hold, for
    /// tests and for saying what a run costs.
    pub fn held_bytes(&self) -> usize {
        self.checkpoints.iter().map(|c| c.window.len()).sum::<usize>() + lock(&self.spans).bytes
    }

    /// Where each checkpoint is, as the bit of the run and the byte of the
    /// output, for tests.
    #[cfg(test)]
    fn marks(&self) -> Vec<(u64, u64, Option<u64>)> {
        self.checkpoints.iter().map(|c| (c.in_bit, c.out, c.block)).collect()
    }
}

/// The spans of one run decoded most recently, up to a number of bytes, with
/// the one read longest ago going first. At least one is kept whatever its
/// size.
struct SpanCache {
    entries: Vec<(usize, Arc<Vec<u8>>, u64)>,
    bytes: usize,
    cap: usize,
    clock: u64,
    #[cfg(test)]
    peak: usize,
}

impl SpanCache {
    fn new(cap: usize) -> SpanCache {
        SpanCache {
            entries: Vec::new(),
            bytes: 0,
            cap,
            clock: 0,
            #[cfg(test)]
            peak: 0,
        }
    }

    fn get(&mut self, k: usize) -> Option<Arc<Vec<u8>>> {
        self.clock += 1;
        let clock = self.clock;
        let e = self.entries.iter_mut().find(|e| e.0 == k)?;
        e.2 = clock;
        Some(e.1.clone())
    }

    fn put(&mut self, k: usize, bytes: Arc<Vec<u8>>) {
        while !self.entries.is_empty() && self.bytes + bytes.len() > self.cap {
            let oldest = (0..self.entries.len()).min_by_key(|&i| self.entries[i].2).expect("not empty");
            let gone = self.entries.swap_remove(oldest);
            self.bytes -= gone.1.len();
        }
        self.clock += 1;
        self.bytes += bytes.len();
        #[cfg(test)]
        {
            self.peak = self.peak.max(self.bytes);
        }
        self.entries.push((k, bytes, self.clock));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Small enough that a few hundred kilobytes reach every limit: the run is
    /// indexed past 64 KiB, checkpoints are 8 KiB apart until there are more
    /// than 12 of them, and the cache holds 40 KiB.
    fn small() -> Limits {
        Limits { whole: 64 << 10, lazy: 4 << 20, spacing: 8 << 10, checkpoints: 12, span_cache: 40 << 10, span_steps: 1_000_000 }
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
        let mut out = vec![0x1f, 0x8b, 8, 0x08, 0, 0, 0, 0, 0, 3];
        out.extend_from_slice(b"name.fits\0");
        out.extend_from_slice(&miniz_oxide::deflate::compress_to_vec(data, level));
        out.extend_from_slice(&crc32(data).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
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
