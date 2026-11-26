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
//! Deflate's pass can be done a step at a time, carrying on from where the
//! step before stopped: see [`Indexing`]. zstd's pass is done in one go.
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

mod deflate;
mod seekable;
mod xz;
mod zstd;

pub use deflate::{Indexing, Progress, Stepped};
pub use seekable::Seekable;
use xz::index_xz;
use zstd::index_zstd;

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
            whole_steps: crate::codec::MAX_STEPS,
            block_marks: MAX_BLOCK_MARKS,
            span: CAP_BYTES,
        }
    }
}

/// Whether a run packed with `codec` can be read a piece at a time.
pub fn can_index(codec: Codec) -> bool {
    matches!(codec, Codec::Deflate | Codec::Zlib | Codec::Gzip | Codec::Zstd | Codec::Xz)
}

/// Whether the pass that indexes a run packed with `codec` can be done a step
/// at a time, by an [`Indexing`]: deflate, zlib and gzip. A zstd run is
/// decoded in one go to index it, and an xz stream is laid out from its own
/// index without decoding any of it, by [`decode_or_index`].
pub fn can_step(codec: Codec) -> bool {
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
/// For deflate, zlib, gzip and zstd this is one pass. The run is decoded as
/// [`crate::codec::decode_traced`] decodes it, with checkpoints taken on the
/// way; a run that ends under the cap comes back exactly as that function
/// gives it, and one that passes the cap carries on from its last checkpoint
/// with the trace off and the output dropped as soon as nothing can copy from
/// it. Deflate's pass is an [`Indexing`] run to the end; zstd's is made in
/// one go. An xz stream is laid out from its own index, without a
/// pass. Every other codec is decoded whole or refused, as it always was.
pub fn decode_or_index(codec: Codec, packed: Arc<Vec<u8>>) -> Result<Decoded, Refusal> {
    decode_or_index_with(codec, packed, Limits::default())
}

/// [`decode_or_index`] under limits other than the default ones.
pub fn decode_or_index_with(codec: Codec, packed: Arc<Vec<u8>>, limits: Limits) -> Result<Decoded, Refusal> {
    if packed.len() > CAP_BYTES {
        return Err(Refusal::TooLarge);
    }
    match codec {
        _ if can_step(codec) => {
            let mut pass = Indexing::new(codec, packed, limits)?;
            loop {
                if let Stepped::Done(decoded) = pass.step(u64::MAX)? {
                    return Ok(decoded);
                }
            }
        }
        Codec::Zstd => index_zstd(packed, limits),
        Codec::Xz => index_xz(packed, limits),
        _ => whole(codec, &packed),
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
        /// How many deflate blocks start before `in_bit`. This counts the
        /// block the checkpoint is inside, and does not count a block
        /// starting at `in_bit`.
        blocks: u64,
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

    /// How many deflate blocks start before it. None of a zstd or xz run's.
    fn blocks(&self) -> u64 {
        match self.start {
            Start::Deflate { blocks, .. } => blocks,
            Start::Unit(_) => 0,
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
    /// The checkpoint taken most recently, even when thinning dropped it from
    /// `list`: where the pass goes back to once the output passes the cap.
    latest: Option<Checkpoint>,
    spacing: u64,
    next: u64,
    max: usize,
}

impl Checkpoints {
    fn new(spacing: u64, max: usize) -> Checkpoints {
        Checkpoints { list: Vec::new(), latest: None, spacing: spacing.max(1), next: 0, max: max.max(2) }
    }

    /// Keep a checkpoint at `out`, if one is due there. Past the most there
    /// may be, every second one goes and the spacing doubles. The first is
    /// never one that goes: a span starting anywhere else would have nothing
    /// to start from.
    fn offer(&mut self, in_bit: u64, out: u64, start: impl FnOnce() -> Start) {
        if out < self.next {
            return;
        }
        let taken = Checkpoint { in_bit, out, start: start() };
        self.latest = Some(taken.clone());
        self.list.push(taken);
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

// Reachable from `eval::space`'s tests, which open runs like these through
// the templates.
#[cfg(test)]
pub(crate) mod tests;
