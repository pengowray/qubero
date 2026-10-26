//! Compressed runs too large to hold whole, read a piece at a time.
//!
//! Every other codec here decodes a run into one buffer and refuses once the
//! output passes [`CAP_BYTES`]. Three kinds of run past that are read through
//! checkpoints instead: places a decoder can start again from. A read decodes
//! from the checkpoint before it, and only as far as the next one. See
//! `docs/DESIGN-lazy-unpacking.md`.
//!
//! - A deflate run (raw, zlib or gzip) is decoded once from front to back,
//!   and a checkpoint keeps the 32 KiB of output before it, which is as far
//!   back as a deflate match can copy from.
//! - A zstd run of several frames is decoded once the same way, and a
//!   checkpoint is the start of a frame, which copies from nothing before it.
//! - An xz stream of several blocks is not decoded at all to index it: its own
//!   index says where every block is and what it comes to, and a checkpoint
//!   is the start of a block, which copies from nothing before it either.
//!
//! The output between two checkpoints is a span. Spans are what is decoded
//! and what is cached, and each has a trace of its own, made when asked for.
//! The steps of a deflate stream name literals by their byte and matches by
//! their length and distance, so they can be read off any stretch of the
//! stream without the output before it; a zstd frame and an xz block are
//! traced from their own start.
//!
//! A zstd frame or an xz block is decoded whole, so one larger than
//! [`Limits::span`] is refused as [`Refusal::TooLarge`]. That is what keeps a
//! file of one frame or one block refused past [`CAP_BYTES`]: what a decoder
//! needs to start inside one is its whole window or dictionary, which can be
//! as large as the output.
//!
//! The guard against a zip bomb is [`LAZY_CAP_BYTES`], a limit on the output.
//! Memory is bounded by the limits below whatever the output comes to, so a
//! bomb can only cost the time of one pass up to that limit.

use std::ops::ControlFlow;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::checksum::{adler32, adler32_update, crc32, crc32_update};
use crate::codec::inflate::{self, Ended, Framing, Resume, Sink, Wrapper};
use crate::codec::{frames, xz, Codec, Member, Refusal, Step, StepField, StepKind, Trace, TraceBuilder, CAP_BYTES};

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
    /// The most one zstd frame or one xz block may come to, since it is
    /// decoded whole: [`CAP_BYTES`]. A limit of its own and not
    /// [`Limits::whole`] because the tools that write these choose their
    /// size: `pzstd` writes frames four windows long, and `xz -T0` writes
    /// blocks three dictionaries long, megabytes at the smallest.
    pub span: usize,
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
            span: CAP_BYTES,
        }
    }
}

/// Whether a run packed with `codec` can be read a piece at a time.
pub fn can_index(codec: Codec) -> bool {
    matches!(codec, Codec::Deflate | Codec::Zlib | Codec::Gzip | Codec::Zstd | Codec::Xz)
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
/// For deflate, zlib, gzip and zstd this is one pass. The run is decoded as
/// [`crate::codec::decode_traced`] decodes it, with checkpoints taken on the
/// way; a run that ends under the cap comes back exactly as that function
/// gives it, and one that passes the cap carries on from its last checkpoint
/// with the trace off and the output dropped as soon as nothing can copy from
/// it. An xz stream is laid out from its own index, without a pass. Every
/// other codec is decoded whole or refused, as it always was.
pub fn decode_or_index(codec: Codec, packed: Arc<Vec<u8>>) -> Result<Decoded, Refusal> {
    decode_or_index_with(codec, packed, Limits::default())
}

/// [`decode_or_index`] under limits other than the default ones.
pub fn decode_or_index_with(codec: Codec, packed: Arc<Vec<u8>>, limits: Limits) -> Result<Decoded, Refusal> {
    if packed.len() > CAP_BYTES {
        return Err(Refusal::TooLarge);
    }
    match (codec, wrapper(codec)) {
        (_, Some(wrapper)) => index_deflate(codec, wrapper, packed, limits),
        (Codec::Zstd, None) => index_zstd(packed, limits),
        (Codec::Xz, None) => index_xz(packed, limits),
        _ => whole(codec, &packed),
    }
}

fn whole(codec: Codec, packed: &[u8]) -> Result<Decoded, Refusal> {
    crate::codec::decode_traced(codec, packed).map(|(bytes, trace)| Decoded::Whole(bytes, trace))
}

/// What a run of no checkpoints but those it was made with comes to: no
/// spans cached and no span trace made yet.
fn seekable(codec: Codec, packed: Arc<Vec<u8>>, kind: Kind, len: u64, checkpoints: Vec<Checkpoint>, limits: Limits) -> Seekable {
    Seekable {
        codec,
        packed,
        kind,
        len,
        checkpoints,
        crc32: None,
        adler32: None,
        limits,
        spans: Mutex::new(SpanCache::new(limits.span_cache)),
        traced: Mutex::new(None),
    }
}

fn index_deflate(codec: Codec, wrapper: Wrapper, packed: Arc<Vec<u8>>, limits: Limits) -> Result<Decoded, Refusal> {
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
    let window = last.window();
    let mut index = WindowSink::new(window.to_vec(), last.out - window.len() as u64, limits.lazy, true);
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
    let mut s = seekable(codec, packed, Kind::Deflate(f), len, checkpoints, limits);
    s.crc32 = Some(sums.crc);
    s.adler32 = Some(sums.adler);
    Ok(Decoded::Indexed(s))
}

/// A zstd run, in one pass as deflate is. Frames copy nothing from the ones
/// before them, so a checkpoint is the start of a frame and keeps no window,
/// and the pass past the cap starts again at the start of the frame the first
/// pass was in, which costs at most one frame decoded twice.
///
/// A frame larger than [`Limits::span`] is refused as too large however many
/// frames there are, since a span is decoded whole and a frame cannot be
/// entered anywhere but at its start. A file of one frame is the usual case:
/// `zstd` writes one frame whatever `-T` and `-B` say, and only `pzstd`, or
/// files joined end to end, have several.
fn index_zstd(packed: Arc<Vec<u8>>, limits: Limits) -> Result<Decoded, Refusal> {
    let data = &packed[..];
    let frame_cap = limits.span as u64;
    let mut marks = Checkpoints::new(limits.spacing, limits.checkpoints);
    let mark = |at: usize, out: u64, marks: &mut Checkpoints| marks.offer(at as u64 * 8, out, || Start::Unit(0));

    // The pass that may come to a whole run, traced, with checkpoints taken
    // in case it does not. A frame may come to as much as the whole run may
    // here, and the frames this pass finished are held to the span limit
    // only if the run turns out not to fit.
    let mut last = (0usize, 0u64);
    let mut largest = 0u64;
    let mut over = false;
    let mut out = Vec::new();
    let mut b = TraceBuilder::default();
    let first = frames::zstd_walk(
        data,
        0..data.len(),
        0,
        frame_cap.max(limits.whole as u64),
        &mut b,
        &mut |at, o| {
            largest = largest.max(o - last.1);
            last = (at, o);
            mark(at, o, &mut marks);
        },
        &mut |bytes| {
            if out.len() + bytes.len() > limits.whole {
                over = true;
                return Err(Refusal::TooLarge);
            }
            out.extend_from_slice(bytes);
            Ok(())
        },
    );
    match first {
        Ok(end) => return Ok(Decoded::Whole(out, frames::zstd_done(b, data.len(), end))),
        Err(Refusal::TooLarge) if over => {}
        Err(other) => return Err(other),
    }
    drop((out, b));
    if largest > frame_cap {
        return Err(Refusal::TooLarge);
    }

    // Past the cap: on from the start of the frame the first pass stopped in,
    // counting the output and keeping none of it.
    let (at, before) = last;
    let mut count = before;
    let len = frames::zstd_walk(
        data,
        at..data.len(),
        before,
        frame_cap,
        &mut TraceBuilder::off(),
        &mut |at, o| mark(at, o, &mut marks),
        &mut |bytes| {
            count += bytes.len() as u64;
            match count > limits.lazy {
                true => Err(Refusal::TooLargeLazy),
                false => Ok(()),
            }
        },
    )?;
    Ok(Decoded::Indexed(seekable(Codec::Zstd, packed, Kind::Zstd, len, marks.list, limits)))
}

/// An xz stream, indexed from the index at its end without decoding any of
/// it: that index lists every block's packed and unpacked size, and an xz
/// block never copies from an earlier block, so a checkpoint is the start of
/// a block.
///
/// A stream whose index adds up to no more than [`Limits::whole`], and a
/// stream [`xz::layout`] cannot read, go to the whole decoder as before, which
/// decodes them or refuses them. A larger stream is refused as too large when
/// its blocks use a filter our LZMA2 decoder does not run, because `lzma-rs`,
/// which runs those filters, reads only whole streams. It is refused as too
/// large too when any block unpacks to more than [`Limits::span`], which is
/// what a file of one block, as plain `xz` writes, is.
///
/// Nothing is decoded here, so a block whose unpacked size differs from the
/// index is found only when its span is read, and that read is refused.
fn index_xz(packed: Arc<Vec<u8>>, limits: Limits) -> Result<Decoded, Refusal> {
    let Some(l) = xz::layout(&packed).filter(|l| l.total_out > limits.whole as u64) else {
        return whole(Codec::Xz, &packed);
    };
    if !l.plain || l.blocks.iter().any(|b| b.unpacked > limits.span as u64) {
        return Err(Refusal::TooLarge);
    }
    if l.total_out > limits.lazy {
        return Err(Refusal::TooLargeLazy);
    }
    let mut marks = Checkpoints::new(limits.spacing, limits.checkpoints);
    let mut out = 0u64;
    for (i, blk) in l.blocks.iter().enumerate() {
        // The first span starts with the stream header, which is no block's.
        let at = if i == 0 { 0 } else { blk.at as u64 * 8 };
        marks.offer(at, out, || Start::Unit(i));
        out += blk.unpacked;
    }
    let len = l.total_out;
    Ok(Decoded::Indexed(seekable(Codec::Xz, packed, Kind::Xz(l), len, marks.list, limits)))
}

/// A place a decode can start again from: for deflate, see
/// [`Sink::boundary`]; for zstd and xz, the start of a frame or a block.
#[derive(Debug, Clone)]
struct Checkpoint {
    /// The bit of the run the next code starts at, or the first bit of the
    /// frame or block. A span's bits run from its checkpoint's to the next.
    in_bit: u64,
    /// How many bytes of output come before it.
    out: u64,
    start: Start,
}

/// What a decode starting at a checkpoint needs besides where it is.
#[derive(Debug, Clone)]
enum Start {
    Deflate {
        /// The header of the Huffman-coded block it is inside, whose tables
        /// a decode starting here reads again. Nothing at the start of a
        /// block.
        block: Option<u64>,
        /// The last 32 KiB of output before it, or all of it when there is
        /// less.
        window: Arc<[u8]>,
    },
    /// The start of a zstd frame or an xz block, which copies nothing from
    /// before it. The number is the xz block's, counted in the stream's index.
    Unit(usize),
}

impl Checkpoint {
    fn window(&self) -> &[u8] {
        match &self.start {
            Start::Deflate { window, .. } => window,
            Start::Unit(_) => &[],
        }
    }

    fn block(&self) -> Option<u64> {
        match self.start {
            Start::Deflate { block, .. } => block,
            Start::Unit(_) => None,
        }
    }

    fn resume(&self) -> Resume {
        Resume { in_bit: self.in_bit, block: self.block() }
    }

    fn unit(&self) -> usize {
        match self.start {
            Start::Unit(n) => n,
            Start::Deflate { .. } => 0,
        }
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
    fn offer(&mut self, in_bit: u64, out: u64, start: impl FnOnce() -> Start) {
        if out < self.next {
            return;
        }
        self.list.push(Checkpoint { in_bit, out, start: start() });
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
            let keep = (WINDOW as u64).min(out) as usize;
            let window = &self.hist[self.hist.len() - keep..];
            marks.offer(in_bit, out, || Start::Deflate { block, window: window.into() });
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
    kind: Kind,
    len: u64,
    checkpoints: Vec<Checkpoint>,
    crc32: Option<u32>,
    adler32: Option<u32>,
    limits: Limits,
    spans: Mutex<SpanCache>,
    traced: Mutex<Option<(usize, Arc<Trace>)>>,
}

/// Which decoder a run's spans go through, and what it needs to know about the
/// run as a whole.
enum Kind {
    /// Where the deflate data starts and ends inside the run's wrapper.
    Deflate(Framing),
    Zstd,
    /// Every block, from the stream's index.
    Xz(xz::Layout),
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
            let bytes = self.span(k)?;
            let take = (range.end.min(end) - pos) as usize;
            let from = (pos - range.start) as usize;
            let to = (pos - at) as usize;
            out[to..to + take].copy_from_slice(&bytes[from..from + take]);
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
        if let Some(bytes) = lock(&self.spans).get(k) {
            return Ok(bytes);
        }
        let bytes = Arc::new(self.decode_span(k)?);
        lock(&self.spans).put(k, bytes.clone());
        Ok(bytes)
    }

    /// Decode span `k` from its own checkpoint to the next one.
    ///
    /// For deflate and zstd it is the same decoder over the same bits as the
    /// pass that indexed it, so a span that comes out another length is a
    /// decoder disagreeing with itself. For xz it is the first time the
    /// span's blocks have been decoded, and a block that does not come to
    /// what the index says is refused here.
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
                let mut bytes = Vec::with_capacity(want);
                let frame_cap = self.limits.span as u64;
                frames::zstd_walk(&self.packed, self.span_in(k), c.out, frame_cap, &mut TraceBuilder::off(), &mut |_, _| {}, &mut |b| {
                    match bytes.len() + b.len() > want {
                        true => Err(Refusal::Failed),
                        false => Ok(bytes.extend_from_slice(b)),
                    }
                })?;
                bytes
            }
            Kind::Xz(l) => xz::blocks(&self.packed, l, self.span_units(k, l), c.out, 0, &mut TraceBuilder::off())?,
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
                let range = self.span_in(k);
                let mut b = TraceBuilder::with_budget(self.limits.span_steps).starting_at(range.start as u64 * 8, c.out);
                let frame_cap = self.limits.span as u64;
                let end = frames::zstd_walk(&self.packed, range.clone(), c.out, frame_cap, &mut b, &mut |_, _| {}, &mut |_| Ok(()))?;
                b.finish_at(range.end as u64 * 8, end);
                b.done()
            }
            Kind::Xz(l) => {
                let range = self.span_in(k);
                let units = self.span_units(k, l);
                let last = units.end == l.blocks.len();
                let mut b = TraceBuilder::with_budget(self.limits.span_steps).starting_at(range.start as u64 * 8, c.out);
                if k == 0 {
                    b.push(0, 0, StepKind::Header(StepField::FrameHeader, 0));
                }
                let end = c.out + xz::blocks(&self.packed, l, units, c.out, self.limits.span_steps, &mut b)?.len() as u64;
                if last {
                    xz::push_index(l, &mut b, end);
                }
                b.finish_at(range.end as u64 * 8, end);
                b.done()
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
        let mut sink = CountSink { len: c.out, stop_at: self.checkpoints.get(k + 1).map(|n| n.in_bit) };
        let ended = inflate::run(&self.packed, f.start, f.end, &mut sink, &mut b, Some(c.resume()))?;
        if ended == Ended::Stream && wrapped {
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
    /// whole run.
    pub fn map_in(&self, bit: u64) -> Option<Step> {
        self.trace_at_bit(bit)?.map_in(bit)
    }

    /// How many bytes the checkpoints' windows and the span cache hold, for
    /// tests and for saying what a run costs.
    pub fn held_bytes(&self) -> usize {
        self.checkpoints.iter().map(|c| c.window().len()).sum::<usize>() + lock(&self.spans).bytes
    }

    /// Where each checkpoint is, as the bit of the run and the byte of the
    /// output, for tests.
    #[cfg(test)]
    fn marks(&self) -> Vec<(u64, u64, Option<u64>)> {
        self.checkpoints.iter().map(|c| (c.in_bit, c.out, c.block())).collect()
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

// Reachable from `eval::space`'s tests, which open runs like these through
// the templates.
#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Small enough that a few hundred kilobytes reach every limit: the run is
    /// indexed past 64 KiB, checkpoints are 8 KiB apart until there are more
    /// than 12 of them, and the cache holds 40 KiB.
    fn small() -> Limits {
        Limits { whole: 64 << 10, lazy: 4 << 20, spacing: 8 << 10, checkpoints: 12, span_cache: 40 << 10, span_steps: 1_000_000, span: 64 << 10 }
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
    pub(crate) fn mixed(len: usize, seed: u64) -> Vec<u8> {
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
                assert_eq!(s.crc32(), Some(crc32(&data)));
                assert_eq!(s.adler32(), Some(adler32(&data)));
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

    /// zstd frames of `chunk` bytes each, packed by `ruzstd`, one after
    /// another. With `skippable`, each has a skippable frame in front of it
    /// holding its length, which is what `pzstd` writes.
    pub(crate) fn zstd_of(data: &[u8], chunk: usize, skippable: bool) -> Vec<u8> {
        use ruzstd::encoding::{compress_to_vec, CompressionLevel};
        let mut out = Vec::new();
        for (i, part) in data.chunks(chunk).enumerate() {
            let level = if i % 3 == 2 { CompressionLevel::Uncompressed } else { CompressionLevel::Fastest };
            let frame = compress_to_vec(part, level);
            if skippable {
                out.extend_from_slice(&0x184d_2a50u32.to_le_bytes());
                out.extend_from_slice(&4u32.to_le_bytes());
                out.extend_from_slice(&(frame.len() as u32).to_le_bytes());
            }
            out.extend_from_slice(&frame);
        }
        out
    }

    /// An xz stream of blocks of `chunk` bytes each, packed by `lzma-rust2`.
    pub(crate) fn xz_of(data: &[u8], chunk: usize) -> Vec<u8> {
        use crate::codec::xz::tests::{pack, wrap};
        let blocks: Vec<(Vec<u8>, Vec<u8>)> = data.chunks(chunk).map(|c| (pack(c, 3, 0, 2, 1 << 16, None), c.to_vec())).collect();
        wrap(&blocks, true)
    }

    /// Reads of every length from everywhere, across span joins, give what
    /// went in, and so does one read of everything.
    fn reads_back(s: &Seekable, data: &[u8], seed: u64) {
        assert_eq!(s.len(), data.len() as u64);
        let mut all = vec![0u8; data.len()];
        s.read(0, &mut all).expect("reads");
        assert!(all == data, "{:?}: the whole output differs", s.codec());
        let mut rand = rand_from(seed);
        for _ in 0..120 {
            let at = rand() % data.len() as u64;
            let n = (rand() % 50_000).min(data.len() as u64 - at) as usize;
            let mut got = vec![0u8; n];
            s.read(at, &mut got).expect("reads");
            assert!(got == data[at as usize..at as usize + n], "{:?}: {n} bytes at {at} differ", s.codec());
        }
    }

    /// Every span's trace tiles its own bits and bytes, and every byte and bit
    /// maps to the same step through the spans as through the trace of the
    /// whole run.
    fn traces_agree(s: &Seekable, whole: &Trace, seed: u64) {
        let packed_bits = s.packed().len() as u64 * 8;
        for k in 0..s.spans() {
            let t = s.span_trace(k).expect("traces");
            t.check_tiles().unwrap_or_else(|e| panic!("{:?} span {k}: {e}", s.codec()));
            assert_eq!(t.start().1, s.span_bytes(k).start);
            assert_eq!(t.out_bytes(), s.span_bytes(k).end);
        }
        assert_eq!(s.span_trace(s.spans() - 1).unwrap().in_bits(), packed_bits);
        let mut rand = rand_from(seed);
        let mut bytes: Vec<u64> = (0..2000).map(|_| rand() % s.len()).collect();
        let mut bits: Vec<u64> = (0..2000).map(|_| rand() % packed_bits).collect();
        bytes.sort_unstable();
        bits.sort_unstable();
        for byte in bytes {
            assert_eq!(s.map_out(byte), whole.map_out(byte), "{:?}: byte {byte}", s.codec());
        }
        for bit in bits {
            assert_eq!(s.map_in(bit), whole.map_in(bit), "{:?}: bit {bit}", s.codec());
        }
        for (in_bit, _, _) in s.marks() {
            for bit in [in_bit.saturating_sub(1), in_bit, in_bit + 1] {
                assert_eq!(s.map_in(bit), whole.map_in(bit), "{:?}: bit {bit} by checkpoint {in_bit}", s.codec());
            }
        }
    }

    /// A zstd run of several frames, with and without `pzstd`'s skippable
    /// frames, past the cap: read back from anywhere, a checkpoint at the
    /// start of a frame, and a trace per span that says what the whole
    /// trace says.
    #[test]
    fn a_zstd_run_of_several_frames_reads_back_from_anywhere() {
        let data = mixed(400_000, 29);
        for skippable in [false, true] {
            let packed = zstd_of(&data, 30_000, skippable);
            let s = indexed(Codec::Zstd, &packed, small());
            reads_back(&s, &data, 31);
            let marks = s.marks();
            assert_eq!(marks[0], (0, 0, None));
            assert!(marks.len() > 4 && marks.len() <= small().checkpoints, "{} checkpoints", marks.len());
            assert!(s.crc32().is_none() && s.adler32().is_none());
            let (bytes, whole) = crate::codec::decode_traced(Codec::Zstd, &packed).expect("decodes whole");
            assert!(bytes == data);
            traces_agree(&s, &whole, 37);
        }
    }

    /// Under the cap a zstd run comes back whole, exactly as the whole
    /// decoder gives it, and that is every frame and not only the first.
    #[test]
    fn a_zstd_run_under_the_cap_comes_back_whole_with_every_frame() {
        let data = mixed(50_000, 41);
        let packed = zstd_of(&data, 20_000, true);
        let (bytes, trace) = crate::codec::decode_traced(Codec::Zstd, &packed).expect("decodes");
        assert!(bytes == data, "{} bytes of {}", bytes.len(), data.len());
        trace.check_tiles().expect("tiles");
        // Three frames, each behind a skippable one.
        let headers = trace.steps().filter(|s| s.kind == StepKind::Header(StepField::FrameHeader, 0)).count();
        assert_eq!(headers, 6);
        let Ok(Decoded::Whole(ours, t)) = decode_or_index_with(Codec::Zstd, Arc::new(packed.clone()), small()) else {
            panic!("did not come back whole");
        };
        assert!(ours == bytes);
        assert!(t.steps().eq(trace.steps()));
    }

    /// A frame past the most one span may come to is refused as too large,
    /// which is what a file of one frame is. Past the limit for a run read a
    /// piece at a time, a run of small frames is refused with the refusal that
    /// says so.
    #[test]
    fn a_zstd_frame_past_the_span_limit_is_refused() {
        let data = mixed(300_000, 43);
        let one = zstd_of(&data, data.len(), false);
        assert_eq!(decode_or_index_with(Codec::Zstd, Arc::new(one), small()).err(), Some(Refusal::TooLarge));
        let mut sizes = zstd_of(&data, 30_000, false);
        sizes.extend(zstd_of(&data[..100_000], 100_000, false));
        assert_eq!(decode_or_index_with(Codec::Zstd, Arc::new(sizes), small()).err(), Some(Refusal::TooLarge));
        let many = zstd_of(&data, 30_000, true);
        let limits = Limits { lazy: 200_000, ..small() };
        assert_eq!(decode_or_index_with(Codec::Zstd, Arc::new(many), limits).err(), Some(Refusal::TooLargeLazy));
    }

    /// An xz stream of several blocks past the cap, laid out from its index:
    /// read back from anywhere, a checkpoint at the start of a block, a trace
    /// per span that says what the whole trace says, and members that are the
    /// whole trace's.
    #[test]
    fn an_xz_stream_of_several_blocks_reads_back_from_anywhere() {
        let data = mixed(400_000, 47);
        let packed = xz_of(&data, 30_000);
        let s = indexed(Codec::Xz, &packed, small());
        reads_back(&s, &data, 53);
        let marks = s.marks();
        assert_eq!(marks[0], (0, 0, None));
        assert!(marks.len() > 4 && marks.len() <= small().checkpoints, "{} checkpoints", marks.len());
        let (bytes, whole) = crate::codec::decode_traced(Codec::Xz, &packed).expect("decodes whole");
        assert!(bytes == data);
        assert!(!whole.coarse());
        traces_agree(&s, &whole, 59);
        assert_eq!(s.members(), whole.members());
    }

    /// One block past the most one span may come to is refused as too large,
    /// which is what a file of one block is; past the limit for a run read a
    /// piece at a time, so is a stream of small blocks, with the refusal that
    /// says so. Under the cap the stream comes back whole as it always did.
    #[test]
    fn an_xz_block_past_the_span_limit_is_refused() {
        let data = mixed(300_000, 61);
        let one = xz_of(&data, data.len());
        assert_eq!(decode_or_index_with(Codec::Xz, Arc::new(one), small()).err(), Some(Refusal::TooLarge));
        let limits = Limits { lazy: 200_000, ..small() };
        assert_eq!(decode_or_index_with(Codec::Xz, Arc::new(xz_of(&data, 30_000)), limits).err(), Some(Refusal::TooLargeLazy));
        let under = xz_of(&data[..50_000], 20_000);
        let (bytes, trace) = crate::codec::decode_traced(Codec::Xz, &under).expect("decodes");
        let Ok(Decoded::Whole(ours, t)) = decode_or_index_with(Codec::Xz, Arc::new(under), small()) else {
            panic!("did not come back whole");
        };
        assert!(ours == bytes);
        assert!(t.steps().eq(trace.steps()));
    }

    /// Nothing decodes an xz stream to index it, so a block that does not
    /// come to what the index says opens, and its span is refused when read.
    /// The spans either side still read.
    #[test]
    fn an_xz_block_the_index_is_wrong_about_is_refused_when_read() {
        use crate::codec::xz::tests::{pack, wrap};
        let data = mixed(300_000, 67);
        let mut blocks: Vec<(Vec<u8>, Vec<u8>)> = data.chunks(30_000).map(|c| (pack(c, 3, 0, 2, 1 << 16, None), c.to_vec())).collect();
        // The fifth block's packed data is the sixth's, so it decodes to a
        // stream of its own and not to what its index record says.
        blocks[4].0 = blocks[5].0.clone();
        blocks[4].1.truncate(29_000);
        let s = indexed(Codec::Xz, &wrap(&blocks, false), Limits { spacing: 1, ..small() });
        let k = s.span_of_byte(4 * 30_000).unwrap();
        let at = s.span_bytes(k).start;
        assert_eq!(s.read(at, &mut [0u8; 10]).err(), Some(Refusal::Failed));
        let mut first = vec![0u8; 30_000];
        s.read(0, &mut first).expect("the first block reads");
        assert!(first == data[..30_000]);
    }

    /// Past the first window of a frame each block is a step of its own, and
    /// the step says the bytes that block made: a raw block's step reads
    /// exactly the bytes it writes. Before that the blocks are one step. The
    /// same through the spans of a run indexed with frames this long.
    #[test]
    fn zstd_blocks_past_the_window_are_steps_of_their_own() {
        let data = mixed(1_200_000, 71);
        let packed = zstd_of(&data, 400_000, true);
        let (bytes, whole) = crate::codec::decode_traced(Codec::Zstd, &packed).expect("decodes");
        assert!(bytes == data);
        let mut raw = 0;
        let mut merged = 0;
        for step in whole.steps().filter(|s| s.kind == StepKind::Block) {
            let at = (step.in_bits.start / 8) as usize;
            let h = u32::from_le_bytes([packed[at - 3], packed[at - 2], packed[at - 1], 0]);
            let (kind, size) = ((h >> 1) & 3, (h >> 3) as u64);
            let reads = step.in_bits.end / 8 - step.in_bits.start / 8;
            let writes = step.out_bytes.end - step.out_bytes.start;
            if kind == 0 && reads == size {
                assert_eq!(writes, size, "raw block at byte {at}");
                assert!(packed[at..at + size as usize] == bytes[step.out_bytes.start as usize..step.out_bytes.end as usize]);
                raw += 1;
            }
            // No block may come to more than 128 KiB, so a step that does is
            // more than one block.
            merged += (writes > 128 << 10) as usize;
        }
        assert!(raw >= 2, "{raw} raw blocks checked");
        assert_eq!(merged, 3, "one step over the first window of each frame");
        let s = indexed(Codec::Zstd, &packed, Limits { span: 1 << 20, ..small() });
        reads_back(&s, &data, 73);
        traces_agree(&s, &whole, 79);
    }
}
