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
//! An xz span is decoded only as far as a read reaches, a chunk of LZMA2 at a
//! time, and a later read carries on from where the last one stopped.
//! The steps of a deflate stream name literals by their byte and matches by
//! their length and distance, so they can be read off any stretch of the
//! stream without the output before it; a zstd frame and an xz block are
//! traced from their own start.
//!
//! A zstd frame or an xz block is decoded from its start, so one larger than
//! [`Limits::span`] is refused as [`Refusal::TooLargeUnit`]. That is what
//! keeps a file of one frame or one block refused past [`CAP_BYTES`]: what a
//! decoder needs to start inside one is its whole window or dictionary, which
//! can be as large as the output.
//!
//! The pass over a deflate or a zstd run can be done a step at a time,
//! carrying on from where the step before stopped: see [`Indexing`].
//!
//! Deflate's pass also records where each deflate block starts, in the run
//! and in the output. That is enough to list the blocks, and to trace any one
//! block when asked: like a span, a block can be traced from the compressed
//! run alone. A hostile run can hold tens of millions of tiny blocks, so past
//! [`MAX_BLOCK_MARKS`] the index keeps the start of every second block,
//! halving again each time the list fills, as it does with checkpoints. A
//! dropped start is found again by decoding from the kept start before it.
//!
//! The guard against a zip bomb is [`LAZY_CAP_BYTES`], a limit on the output.
//! Memory is bounded by the limits below whatever the output comes to, so a
//! bomb can only cost the time of one pass up to that limit.

use std::sync::Arc;

use crate::codec::inflate::{Resume, Wrapper};
use crate::codec::{Codec, Refusal, Trace, CAP_BYTES};
use crate::source::Missing;

mod deflate;
mod input;
mod seekable;
mod xz;
mod zstd;

use deflate::DeflatePass;
pub use input::{refused_as_none, ReadError, RunInput};
pub use seekable::Seekable;
use xz::index_xz;
use zstd::ZstdPass;

/// The most a run read a piece at a time may come to. Past this it is refused
/// as [`Refusal::TooLargeLazy`].
pub const LAZY_CAP_BYTES: u64 = 1 << 30;

/// The most compressed input a run may take up when its index pass reads it
/// through a handle onto its source, a window at a time: 512 MiB. A trace
/// keeps the bit each step starts at, counted from the run's first bit, in
/// 32 bits (`RawStep.in_start`), and 2^32 bits is 512 MiB. A longer run is
/// refused as [`Refusal::TooLargeInput`] before any of it is read.
///
/// A pass that reads a copy of its run is held to [`CAP_BYTES`] instead
/// ([`Limits::copy_input`]), since the copy is in memory for as long as the
/// pass runs. See [`too_long_to_index`].
pub const INPUT_CAP_BYTES: u64 = 1 << 29;

/// The most checkpoints one run keeps. When there would be more, every second
/// one is dropped and the spacing doubles, so 256 × 32 KiB = 8 MiB of windows
/// is the most a run's checkpoints hold.
pub const MAX_CHECKPOINTS: usize = 256;

/// How much output comes between two checkpoints, until there are too many.
pub const SPACING_START: u64 = 1 << 20;

/// How much of the compressed run comes between two checkpoints, until there
/// are too many: a checkpoint is due when either this much input or
/// [`SPACING_START`] of output has passed since the last one. Without it a
/// span's input has no bound, since an empty deflate block is 10 bits and
/// makes nothing, and a hostile run can be nothing else.
///
/// Thinning doubles it with the output spacing. Output and input checkpoints
/// share one list of [`MAX_CHECKPOINTS`], so a run has about `out / s + in / s`
/// of them, `s` being the spacing both rules use. At the limits, 1 GiB of
/// output and 512 MiB of input, that is 1.5 GiB / `s`, which is 256 or fewer
/// once `s` is 8 MiB: both spacings reach at most 8 MiB, three doublings on.
pub const IN_SPACING_START: u64 = 1 << 20;

/// The most compressed input one zstd frame or one xz block of an indexed
/// run may take, and the most one stretch between two kept deflate block
/// starts may take, when the run is read from a source that keeps only part
/// of the file at once. Each is read whole when a span or a block list is
/// read, so one larger is refused as [`Refusal::TooMuchAtOnce`].
pub const UNIT_INPUT: usize = 16 << 20;

/// The most compressed input one read of a zstd or xz span may gather at
/// once, when the run is read from a source that keeps only part of the file.
/// A span over it, which only thinning can make, is refused when read, as
/// [`Refusal::TooMuchAtOnce`]. A deflate span never reaches it: its input is at
/// most the input spacing and one stored block for each thinning, 8.5 MiB.
pub const SPAN_INPUT: usize = 16 << 20;

/// How far past where it starts one step of a deflate or zstd index pass may
/// stop, in bytes of the run: the step reads its run a window at a time,
/// from the byte its first bit is in to this much past it and the decoder's
/// margin beyond that ([`DEFLATE_MARGIN`] or [`ZSTD_MARGIN`]), and stops
/// before the window runs out. As many bytes as a step's share of output
/// has, so that a run whose output is at least as large as the run still
/// ends each step at its share of output.
///
/// [`DEFLATE_MARGIN`]: crate::codec::inflate::DEFLATE_MARGIN
/// [`ZSTD_MARGIN`]: crate::codec::frames::ZSTD_MARGIN
pub const STEP_INPUT: usize = 256 << 10;

/// How much decoded output one run keeps, in spans, before the span read
/// longest ago is dropped. One span is kept whatever its size. An xz span
/// counts for as much of it as is decoded.
pub const SPAN_CACHE_BYTES: usize = 16 << 20;

/// How many steps a deflate span's trace may hold before it names blocks
/// rather than codes, as [`crate::codec::MAX_STEPS`] does for a whole run.
///
/// A zstd or xz span's trace may hold [`crate::codec::MAX_STEPS`], as many as
/// a whole run's. Such a span is one or more frames or blocks of up to
/// [`Limits::span`] (64 MiB), which is as large as a run decoded whole, and an
/// `xz -T0` block of 24 MiB of text takes over two million steps. A deflate
/// span is a few megabytes at most.
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
    /// [`IN_SPACING_START`].
    pub in_spacing: u64,
    /// [`MAX_CHECKPOINTS`].
    pub checkpoints: usize,
    /// [`SPAN_CACHE_BYTES`].
    pub span_cache: usize,
    /// [`SPAN_TRACE_STEPS`]. Also the most steps one block's trace holds
    /// before its codes become one opaque step, and the most steps the cached
    /// block traces of one run hold in total.
    pub span_steps: usize,
    /// How many steps the trace of a run held whole may hold before it names
    /// blocks rather than codes: [`crate::codec::MAX_STEPS`].
    pub whole_steps: usize,
    /// [`MAX_BLOCK_MARKS`].
    pub block_marks: usize,
    /// The most one zstd frame or one xz block may come to, since it is
    /// decoded from its start: [`CAP_BYTES`]. A limit of its own and not
    /// [`Limits::whole`] because the tools that write these choose their
    /// size: `pzstd` writes frames four windows long, and `xz -T0` writes
    /// blocks three dictionaries long, megabytes at the smallest.
    pub span: usize,
    /// The most output a deflate stream is counted to when it is only being
    /// measured, to find where it ends ([`crate::template::Expr::StreamLen`]):
    /// [`CAP_BYTES`]. A limit of its own and not [`Limits::whole`], so that a
    /// test can make the runs held whole small and still measure a gzip
    /// member past them, and can reach this one without 64 MiB of output.
    pub measure: u64,
    /// [`UNIT_INPUT`]. Held to only when the run is read from a source with
    /// a resident limit; see [`RunInput::resident_limit`].
    pub unit_input: usize,
    /// [`SPAN_INPUT`]. Held to only when the run is read from a source with
    /// a resident limit, as `unit_input` is.
    pub span_input: usize,
    /// [`STEP_INPUT`]. A limit of its own so that a test can read a run of a
    /// few hundred kilobytes in windows of every size.
    pub step_input: usize,
    /// [`INPUT_CAP_BYTES`]: the most compressed input a run may take up when
    /// its pass reads it through a handle. A limit of its own so that a test
    /// can reach it with a run of a few hundred kilobytes.
    pub input: u64,
    /// The most compressed input a run may take up when its pass reads a
    /// copy of it: [`CAP_BYTES`]. A limit of its own, so that a test can show
    /// a run past it read through a handle without 64 MiB of run.
    pub copy_input: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            whole: CAP_BYTES,
            lazy: LAZY_CAP_BYTES,
            spacing: SPACING_START,
            in_spacing: IN_SPACING_START,
            checkpoints: MAX_CHECKPOINTS,
            span_cache: SPAN_CACHE_BYTES,
            span_steps: SPAN_TRACE_STEPS,
            whole_steps: crate::codec::MAX_STEPS,
            block_marks: MAX_BLOCK_MARKS,
            span: CAP_BYTES,
            measure: CAP_BYTES as u64,
            unit_input: UNIT_INPUT,
            span_input: SPAN_INPUT,
            step_input: STEP_INPUT,
            input: INPUT_CAP_BYTES,
            copy_input: CAP_BYTES,
        }
    }
}

impl Limits {
    /// The most of the run one step of the pass over a run packed with
    /// `codec` reads at once: [`Limits::step_input`] and the decoder's
    /// margin past where the step may stop.
    fn step_window(&self, codec: Codec) -> usize {
        let margin = match codec {
            Codec::Zstd => crate::codec::frames::ZSTD_MARGIN,
            _ => crate::codec::inflate::DEFLATE_MARGIN,
        };
        self.step_input.max(1).saturating_add(margin)
    }
}

/// Whether the pass over a run packed with `codec` can read the run through
/// `input` a window at a time, holding no copy of it: the pass is one that
/// steps ([`can_step`]), and the source answers any read whole or keeps at
/// least twice what one step reads at once, so that a step's window fits
/// beside the other reads of a go. In the browser the file's store keeps
/// 32 MiB and a step reads at most about 384 KiB. A pass that cannot is
/// made over a copy of the run ([`Indexing::from_copy`]).
pub fn reads_in_windows(codec: Codec, input: &RunInput, limits: &Limits) -> bool {
    can_step(codec) && input.fits(limits.step_window(codec))
}

/// Why a run of `len` bytes is too long for an index pass, if it is, said
/// before any of it is read: past [`Limits::input`] for a pass that reads it
/// through a handle (`through_handle`), as [`Refusal::TooLargeInput`], and
/// past [`Limits::copy_input`] for a pass that reads a copy of it, as
/// [`Refusal::TooLargeCopy`]. Each pass is held to the one limit of the way it
/// reads, so a copy never shows the handle's refusal.
pub fn too_long_to_index(len: u64, through_handle: bool, limits: &Limits) -> Option<Refusal> {
    match through_handle {
        true => (len > limits.input).then_some(Refusal::TooLargeInput),
        false => (len > limits.copy_input as u64).then_some(Refusal::TooLargeCopy),
    }
}

/// Whether a run packed with `codec` can be read a piece at a time.
pub fn can_index(codec: Codec) -> bool {
    matches!(codec, Codec::Deflate | Codec::Zlib | Codec::Gzip | Codec::Zstd | Codec::Xz)
}

/// Whether the pass that indexes a run packed with `codec` can be done a step
/// at a time, by an [`Indexing`]: deflate, zlib, gzip and zstd. An xz stream
/// is laid out from its own index without decoding any of it, by
/// [`decode_or_index`].
pub fn can_step(codec: Codec) -> bool {
    wrapper(codec).is_some() || codec == Codec::Zstd
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
/// it. The pass is an [`Indexing`] stepped to the end in
/// one go. An xz stream is laid out from its own index, without a
/// pass. Every other codec is decoded whole or refused, as it always was.
pub fn decode_or_index(codec: Codec, packed: Arc<Vec<u8>>) -> Result<Decoded, Refusal> {
    decode_or_index_with(codec, packed, Limits::default())
}

/// [`decode_or_index`] under limits other than the default ones.
pub fn decode_or_index_with(codec: Codec, packed: Arc<Vec<u8>>, limits: Limits) -> Result<Decoded, Refusal> {
    let input = RunInput::held(packed.clone());
    decode_or_index_through(codec, packed, input, limits)
}

/// [`decode_or_index_with`], for an index that reads the run through `input`
/// once it is made: the same bytes as `packed`, which the pass reads now.
/// When `input`'s source has a resident limit, the index is held to
/// [`Limits::unit_input`] and [`Limits::span_input`].
///
/// An index over a handle onto the file holds no copy of the run once the
/// pass is over: `packed` goes with the pass. See
/// `docs/DESIGN-lazy-compressed-input.md`, "Where a `RunInput` comes from".
/// The evaluator steps the pass over a deflate, zlib, gzip or zstd run
/// itself, reading it through `input` with no copy when it can
/// ([`reads_in_windows`]); it calls this for xz, whose layout still reads a
/// copy of the whole stream.
pub fn decode_or_index_through(codec: Codec, packed: Arc<Vec<u8>>, input: RunInput, limits: Limits) -> Result<Decoded, Refusal> {
    if let Some(why) = too_long_to_index(packed.len() as u64, false, &limits) {
        return Err(why);
    }
    match codec {
        _ if can_step(codec) => {
            let mut pass = Indexing::from_copy(codec, packed, input, limits)?;
            loop {
                match pass.step(u64::MAX, u64::MAX)? {
                    Stepped::Done(decoded) => return Ok(decoded),
                    Stepped::Working => {}
                    // A copy in memory never finds a chunk missing.
                    Stepped::NeedsInput(_) => return Err(Refusal::Failed),
                }
            }
        }
        Codec::Xz => index_xz(packed, input, limits),
        _ => whole(codec, &packed),
    }
}

/// The pass [`decode_or_index`] makes over a deflate, zlib, gzip or zstd run,
/// done a slice at a time: each [`Indexing::step`] decodes about as much
/// output, or reads about as many bits of the run, as it is told to, stops at
/// a place the decoder can carry on from, and the next step carries on from
/// exactly there. A browser calls it between frames, so that indexing 500 MB
/// of output does not stop the page for seconds. What it comes to is what the
/// pass in one go comes to.
///
/// A deflate step stops between two codes or at the start of a block, and a
/// zstd step between two zstd blocks, two frames or two skippable frames.
/// The bits are what bound a step over blocks or frames that make no output,
/// such as deflate's empty blocks, ten bits each. See [`DeflatePass`] and
/// [`ZstdPass`].
pub struct Indexing(Pass);

enum Pass {
    Deflate(DeflatePass),
    Zstd(ZstdPass),
}

/// What one [`Indexing::step`] came to.
pub enum Stepped {
    /// Not finished yet. Step again to carry on.
    Working,
    /// The step's window of the run is missing these chunks of the file, and
    /// the step did nothing. Step again once they are loaded.
    NeedsInput(Vec<Missing>),
    Done(Decoded),
}

/// How far an [`Indexing`] has got: `read_bits` of the run's `run_bits`, and
/// `unpacked` bytes of output so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub read_bits: u64,
    pub run_bits: u64,
    pub unpacked: u64,
}

impl Indexing {
    /// Start indexing `packed`, a run held in memory. `codec` is one
    /// [`can_step`] says yes to; any other, xz included, is refused as
    /// [`Refusal::Failed`]. Nothing is decoded until the first step.
    pub fn new(codec: Codec, packed: Arc<Vec<u8>>, limits: Limits) -> Result<Indexing, Refusal> {
        Indexing::through(codec, RunInput::held(packed), limits)
    }

    /// Start indexing the run `input` reads, which the pass reads a window
    /// at a time and the index reads once it is made. Nothing is read until
    /// the first step: that step reads the run's last bytes, which hold the
    /// trailer, and its head until the wrapper's header ends. A step whose
    /// window has chunks missing answers [`Stepped::NeedsInput`].
    ///
    /// So a pass over a handle onto the file holds only its window, at most
    /// [`Limits::step_input`] and the decoder's margin, besides what any pass
    /// holds: its checkpoints and their copies of block headers, and up to
    /// the cap of output while the run might still be held whole. When
    /// `input`'s source has a resident limit, the index is held to
    /// [`Limits::unit_input`] and [`Limits::span_input`].
    pub fn through(codec: Codec, input: RunInput, limits: Limits) -> Result<Indexing, Refusal> {
        let reads = input.clone();
        Indexing::reading(codec, reads, input, limits)
    }

    /// [`Indexing::through`], for a pass that reads `packed`, a copy of the
    /// run, while the index reads the run through `input` once it is made:
    /// for a run the pass cannot read through `input` a window at a time
    /// (see [`reads_in_windows`]). The pass lets go of the copy when it ends.
    pub fn from_copy(codec: Codec, packed: Arc<Vec<u8>>, input: RunInput, limits: Limits) -> Result<Indexing, Refusal> {
        if packed.len() as u64 != input.len() {
            return Err(Refusal::Failed);
        }
        Indexing::reading(codec, RunInput::held(packed), input, limits)
    }

    /// The pass reads the run through `reads`, and the index through `input`.
    /// A pass that reads a copy is held to [`Limits::copy_input`], and one
    /// that reads a handle to [`Limits::input`]: see [`too_long_to_index`].
    fn reading(codec: Codec, reads: RunInput, input: RunInput, limits: Limits) -> Result<Indexing, Refusal> {
        if let Some(why) = too_long_to_index(input.len(), reads.held_copy().is_none(), &limits) {
            return Err(why);
        }
        Ok(Indexing(match codec {
            Codec::Zstd => Pass::Zstd(ZstdPass::new(reads, input, limits)),
            _ => Pass::Deflate(DeflatePass::new(codec, reads, input, limits)?),
        }))
    }

    /// The same pass for a run the caller knows comes to more than the cap: a
    /// stitched stream's part whose length the file gives. It starts with the
    /// trace off, as a pass that has passed the cap carries on, and so skips
    /// decoding the first 64 MiB with the trace on, which is the slowest part
    /// of the pass and is thrown away at the cap. What it comes to is the
    /// index the pass would have made. A run that turns out to fit under the
    /// cap after all comes back indexed too, and a caller that knew its
    /// length finds it is not the one it was told.
    pub fn known_large(self) -> Indexing {
        Indexing(match self.0 {
            Pass::Deflate(p) => Pass::Deflate(p.known_large()),
            Pass::Zstd(p) => Pass::Zstd(p.known_large()),
        })
    }

    /// How far the pass has got: the bits of the run read and the bytes of
    /// output let go of, at the furthest each has been, so that neither goes
    /// back when the pass goes back at the cap.
    pub fn progress(&self) -> Progress {
        match &self.0 {
            Pass::Deflate(p) => p.progress(),
            Pass::Zstd(p) => p.progress(),
        }
    }

    /// Decode about `out_bytes` more output, or read about `in_bits` more
    /// bits of the run, whichever comes first, and stop; or finish. A deflate
    /// step goes past either by at most one stored block (64 KiB), and a zstd
    /// step by at most one zstd block (128 KiB). A step also stops
    /// [`Limits::step_input`] past where it started, before its window of
    /// the run runs out; or does nothing and answers
    /// [`Stepped::NeedsInput`] with the chunks its window is missing.
    pub fn step(&mut self, out_bytes: u64, in_bits: u64) -> Result<Stepped, Refusal> {
        match &mut self.0 {
            Pass::Deflate(p) => p.step(out_bytes, in_bits),
            Pass::Zstd(p) => p.step(out_bytes, in_bits),
        }
    }

    /// How many bytes of the run the pass holds between steps, for tests:
    /// its window and, for deflate, its trailer and the copy of the header
    /// of the block it stopped in.
    #[cfg(test)]
    fn holds(&self) -> usize {
        match &self.0 {
            Pass::Deflate(p) => p.holds(),
            Pass::Zstd(p) => p.holds(),
        }
    }

    /// Deflate's pass, for tests that look at where its steps stopped.
    #[cfg(test)]
    fn deflate(&self) -> &DeflatePass {
        match &self.0 {
            Pass::Deflate(p) => p,
            Pass::Zstd(_) => panic!("a zstd pass"),
        }
    }
}

fn whole(codec: Codec, packed: &[u8]) -> Result<Decoded, Refusal> {
    crate::codec::decode_traced(codec, packed).map(|(bytes, trace)| Decoded::Whole(bytes, trace))
}

/// A place a decode can start again from: for deflate, see
/// [`Sink::boundary`]; for zstd and xz, the start of a frame or a block.
///
/// [`Sink::boundary`]: crate::codec::inflate::Sink::boundary
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
        /// A copy of that header and its tables: the bytes of the run from
        /// the one `block` is in, [`MAX_HEAD_BYTES`] of them or to the run's
        /// end. So a decode starting here reads only the run from here on,
        /// however far back the block starts. Made when the pass finishes,
        /// for the checkpoints it keeps; nothing until then.
        ///
        /// [`MAX_HEAD_BYTES`]: crate::codec::inflate::MAX_HEAD_BYTES
        head: Option<Arc<[u8]>>,
        /// How many deflate blocks start before `in_bit`. This counts the
        /// block the checkpoint is inside, and does not count a block
        /// starting at `in_bit`.
        blocks: u64,
        /// The last 32 KiB of output before it, or all of it when there is
        /// less.
        window: Arc<[u8]>,
    },
    /// The start of a zstd frame, which copies nothing from before it, or of
    /// the skippable frames in front of it. `own` is the byte the span's
    /// first frame starts at itself, after those skippable frames, or the
    /// span's end when it holds no frame: a read of the span's bytes starts
    /// there, and reads none of the skippable frames.
    Frame { own: usize },
    /// The start of an xz block, which copies nothing from before it. The
    /// number is the block's, counted in the stream's index.
    Unit(usize),
}

impl Checkpoint {
    fn window(&self) -> &[u8] {
        match &self.start {
            Start::Deflate { window, .. } => window,
            Start::Frame { .. } | Start::Unit(_) => &[],
        }
    }

    fn block(&self) -> Option<u64> {
        match self.start {
            Start::Deflate { block, .. } => block,
            Start::Frame { .. } | Start::Unit(_) => None,
        }
    }

    /// The copy of the header of the block it is inside, and the byte of
    /// the run the copy starts at.
    fn head(&self) -> Option<(&[u8], usize)> {
        match &self.start {
            Start::Deflate { block: Some(h), head: Some(bytes), .. } => Some((bytes, (h / 8) as usize)),
            _ => None,
        }
    }

    /// How many deflate blocks start before it. None of a zstd or xz run's.
    fn blocks(&self) -> u64 {
        match self.start {
            Start::Deflate { blocks, .. } => blocks,
            Start::Frame { .. } | Start::Unit(_) => 0,
        }
    }

    fn resume(&self) -> Resume {
        Resume { in_bit: self.in_bit, block: self.block() }
    }

    fn unit(&self) -> usize {
        match self.start {
            Start::Unit(n) => n,
            Start::Deflate { .. } | Start::Frame { .. } => 0,
        }
    }

    /// Where a zstd span's first frame starts itself. Nothing for deflate
    /// and xz.
    fn own(&self) -> Option<usize> {
        match self.start {
            Start::Frame { own } => Some(own),
            Start::Deflate { .. } | Start::Unit(_) => None,
        }
    }
}

/// The checkpoints an index pass has taken, and where it takes the next.
struct Checkpoints {
    list: Vec<Checkpoint>,
    /// The checkpoint taken most recently, even when thinning dropped it from
    /// `list`: where the pass goes back to once the output passes the cap.
    latest: Option<Checkpoint>,
    /// How much output, and how many bits of the run, come between two
    /// checkpoints, and where the next is due by each.
    spacing: u64,
    in_spacing: u64,
    next: u64,
    next_in: u64,
    max: usize,
}

impl Checkpoints {
    fn new(limits: &Limits) -> Checkpoints {
        Checkpoints {
            list: Vec::new(),
            latest: None,
            spacing: limits.spacing.max(1),
            in_spacing: limits.in_spacing.saturating_mul(8).max(1),
            next: 0,
            next_in: 0,
            max: limits.checkpoints.max(2),
        }
    }

    /// Keep a checkpoint at bit `in_bit` of the run and byte `out` of the
    /// output, if one is due there: the output since the last one has come
    /// to the spacing, or the run up to bit `reach` has come to the input
    /// spacing. `reach` is `in_bit`, except for a zstd frame with skippable
    /// frames in front of it, where it is the frame's own start: the
    /// skippable frames are counted with the frame they are in front of, so
    /// that long ones start a span, which reads past them.
    fn offer(&mut self, in_bit: u64, reach: u64, out: u64, start: impl FnOnce() -> Start) {
        if out < self.next && reach < self.next_in {
            return;
        }
        self.take(in_bit, out, start);
    }

    /// Whether a checkpoint is due by the input, at bit `reach` of the run.
    fn due_by_input(&self, reach: u64) -> bool {
        reach >= self.next_in
    }

    /// Keep a checkpoint at the start of a zstd frame or an xz block `len`
    /// bits long, at `in_bit` and `out`, when it is the input spacing long or
    /// longer, so that it is a span of its own: the next checkpoint is then
    /// due by the input where it ends.
    fn take_if_long(&mut self, in_bit: u64, len: u64, out: u64, start: impl FnOnce() -> Start) {
        if len >= self.in_spacing {
            self.take(in_bit, out, start);
        }
    }

    /// Keep a checkpoint at `in_bit` and `out` whether one is due or not,
    /// unless the one taken last is there already. Past the most there may
    /// be, every second one goes and both spacings double. The first is never
    /// one that goes: a span starting anywhere else would have nothing to
    /// start from.
    fn take(&mut self, in_bit: u64, out: u64, start: impl FnOnce() -> Start) {
        // A pass that goes back at the cap, or a step that carries on from
        // where it stopped, offers the same place again.
        if self.latest.as_ref().is_some_and(|c| c.in_bit == in_bit) {
            return;
        }
        let taken = Checkpoint { in_bit, out, start: start() };
        self.latest = Some(taken.clone());
        self.list.push(taken);
        self.next = out.saturating_add(self.spacing);
        self.next_in = in_bit.saturating_add(self.in_spacing);
        if self.list.len() > self.max {
            let mut i = 0;
            self.list.retain(|_| {
                i += 1;
                (i - 1) % 2 == 0
            });
            self.spacing = self.spacing.saturating_mul(2);
            self.in_spacing = self.in_spacing.saturating_mul(2);
            let last = self.list.last().expect("the first is kept");
            self.next = last.out.saturating_add(self.spacing);
            self.next_in = last.in_bit.saturating_add(self.in_spacing);
        }
    }
}

// Reachable from `eval::space`'s tests, which open runs like these through
// the templates.
#[cfg(test)]
pub(crate) mod tests;
