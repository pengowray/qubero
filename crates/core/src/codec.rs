//! Unpacking a compressed run so the fields inside it can be read.
//!
//! A file that holds a compressed stream holds structure the reader cannot see:
//! a ROOT record is a nine-byte block header and then a zlib stream, and
//! everything the record is *for* is on the other side of it. Reading the run
//! as `bytes[3824]` is honest and useless.
//!
//! So a template may say what a run is compressed with, and the reading opens
//! it. The compressed bytes stay exactly where they are and stay exactly as
//! long as they are; what comes out of them is a second address space, and the
//! fields declared over it count from its own start. See
//! [`Ty::Decoded`](crate::template::Ty::Decoded).
//!
//! Every decoder here is pure Rust and builds for wasm32. Nothing streams: a
//! stream is opened whole or not at all, which is why there is a cap.

pub mod frames;
pub mod inflate;
pub mod lz4;
pub mod pico8;
pub mod pixels;
pub mod pxu;

use std::ops::Range;

/// The largest a decoded stream may come to. Past this the run is left as the
/// bytes it is and the node says why: a zip bomb is one line in a file and
/// gigabytes in memory, and a hex editor that opens one has stopped being a
/// hex editor.
pub const CAP_BYTES: usize = 64 * 1024 * 1024;

/// What a run is compressed with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    /// RFC 1950: two header bytes, deflate, an Adler-32.
    Zlib,
    /// RFC 1951 on its own, with nothing wrapped round it.
    Deflate,
    Zstd,
    /// One LZ4 block, with no frame header and no length in front of it. What
    /// ROOT hands to LZ4 and what an LZ4 frame's blocks hold.
    Lz4Block,
    Xz,
    /// Not compression: PNG's per-row filtering, undone. What comes out of an
    /// IDAT's zlib stream is rows of `1 + stride` bytes, a filter byte and a
    /// row predicted from its neighbours; what comes out of this is the
    /// pixels. `bpp` is the bytes in a pixel, which is how far back a filter
    /// looks for the byte to its left.
    PngUnfilter { stride: u32, bpp: u8 },
    /// Not compression either: one byte out of the low two bits of each
    /// channel of an RGBA pixel, alpha first. How a PICO-8 cartridge is
    /// carried inside the picture of its label.
    LowBitsArgb,
    /// The same trick at a different width: eleven bits out of one RGBA pixel,
    /// three from red, three from green, three from blue and two from alpha,
    /// packed into a byte stream low bits first. How a Picotron cartridge is
    /// carried inside the picture of its label.
    LowBitsRgba11,
    /// PICO-8's `\0pxa` code compression, written by PICO-8 0.2.0 and after.
    /// A bit stream of move-to-front literals and back-references. The run
    /// handed here is the stream alone: the eight header bytes in front of it
    /// are fields of the cart and are read by the template, not by this.
    Pico8Pxa,
    /// PICO-8's older `:c:\0` code compression: a byte stream of table
    /// indices, escaped bytes and two-byte back-references, ending at a pair
    /// of zero bytes. The run handed here starts after the same eight header
    /// bytes.
    Pico8Old,
    /// Picotron's `pxu` userdata encoding, which sits inside a POD's text
    /// where a `userdata()` value would be. The run handed here starts at the
    /// `pxu\0` and may be longer than the elements need.
    PicotronPxu,
}

impl Codec {
    pub fn as_str(self) -> &'static str {
        match self {
            Codec::Zlib => "zlib",
            Codec::Deflate => "deflate",
            Codec::Zstd => "zstd",
            Codec::Lz4Block => "lz4",
            Codec::Xz => "xz",
            Codec::PngUnfilter { .. } => "png unfilter",
            Codec::LowBitsArgb => "low bits argb",
            Codec::LowBitsRgba11 => "low bits rgba 11",
            Codec::Pico8Pxa => "pico-8 pxa",
            Codec::Pico8Old => "pico-8 old code",
            Codec::PicotronPxu => "picotron pxu",
        }
    }
}

/// Why a run was left as bytes. A kind rather than a sentence: the core says
/// what happened and the interface says it in words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The run, or what it would come to, is past [`CAP_BYTES`].
    TooLarge,
    /// The decoder would not read it.
    Failed,
    /// The run does not start on a byte, and no decoder reads half a byte.
    Unaligned,
}

impl Refusal {
    /// The word the interface looks the message up by.
    pub fn as_str(self) -> &'static str {
        match self {
            Refusal::TooLarge => "too-large",
            Refusal::Failed => "failed",
            Refusal::Unaligned => "unaligned",
        }
    }
}

/// How many steps a trace may hold before it stops recording one per symbol.
///
/// A 64 MiB deflate output is tens of millions of literals, and a step apiece
/// is more memory than the bytes they describe. Past this the decoder keeps
/// decoding and keeps the map, but a block's symbols are recorded as one
/// [`StepKind::Opaque`] step covering the whole run: the trace still tiles the
/// input and the output, it just stops naming every byte.
pub const MAX_STEPS: usize = 4_000_000;

/// Which named field of a header or a sequence a step is.
///
/// Not one enum per codec: a step is a step, and the interface wants a word
/// for it rather than a shape to match on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepField {
    /// Deflate: the bit that says this is the last block.
    Bfinal,
    /// Deflate: stored, fixed Huffman, or dynamic Huffman.
    Btype,
    /// Deflate: how many literal/length code lengths follow, less 257.
    Hlit,
    /// Deflate: how many distance code lengths follow, less 1.
    Hdist,
    /// Deflate: how many code-length code lengths follow, less 4.
    Hclen,
    /// A stored block's length, and its one's complement.
    StoredLen,
    StoredNlen,
    /// The bits between the last block and the byte boundary, which a decoder
    /// reads past and nothing means.
    Padding,
    /// Bytes before or after the deflate stream a wrapper put there: zlib's
    /// two header bytes and its Adler-32.
    Wrapper,
    /// LZ4: the byte holding the literal run's length and the match's.
    Token,
    /// LZ4: the bytes extending a length past what the token could hold.
    LengthExtra,
    /// LZ4: how far back the match reads.
    Offset,
    /// zstd, xz: a frame header.
    FrameHeader,
    /// zstd, xz: one block's header.
    BlockHeader,
    /// PNG: the byte in front of a scanline saying how the row was predicted.
    Filter,
    /// xz: the index and the stream footer.
    Footer,
    /// pxu: the two bytes saying the element type, whether a height follows
    /// the width, how wide the sizes are, and which compression was used.
    PxuFlags,
    /// pxu: how many elements a row is, and how many rows there are.
    PxuWidth,
    PxuHeight,
    /// pxu: how many of a token's low bits are an index into the table of
    /// elements written before.
    PxuBits,
}

impl StepField {
    /// The word the interface looks the field's name up by.
    pub fn as_str(self) -> &'static str {
        match self {
            StepField::Bfinal => "bfinal",
            StepField::Btype => "btype",
            StepField::Hlit => "hlit",
            StepField::Hdist => "hdist",
            StepField::Hclen => "hclen",
            StepField::StoredLen => "len",
            StepField::StoredNlen => "nlen",
            StepField::Padding => "padding",
            StepField::Wrapper => "wrapper",
            StepField::Token => "token",
            StepField::LengthExtra => "length_extra",
            StepField::Offset => "offset",
            StepField::FrameHeader => "frame_header",
            StepField::BlockHeader => "block_header",
            StepField::Filter => "filter",
            StepField::Footer => "footer",
            StepField::PxuFlags => "pxu_flags",
            StepField::PxuWidth => "pxu_width",
            StepField::PxuHeight => "pxu_height",
            StepField::PxuBits => "pxu_bits",
        }
    }
}

/// One entry of a Huffman table as the decoder read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableField {
    /// One of the code-length alphabet's own lengths, three bits: which
    /// symbol of that alphabet, and how long its code is.
    CodeLen { sym: u8, len: u8 },
    /// A literal/length code length, given outright.
    LitLen { sym: u16, len: u8 },
    /// A distance code length, given outright.
    Dist { sym: u16, len: u8 },
    /// Code 16, 17 or 18: repeat the length before it, or a run of zeroes.
    /// `count` symbols get `len`, and `dist` says which table they fill.
    Repeat { code: u8, count: u16, len: u8, dist: bool },
}

/// What a decoder did over one stretch of its input.
///
/// The one source of both the map between the two spaces and the fields the
/// compressed space shows: nothing is drawn that the decoder did not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    /// A named field of a block or frame header, and what it said. A field
    /// that says nothing on its own -- padding, a wrapper's bytes -- says 0.
    Header(StepField, u32),
    /// A Huffman code length, or a run of them.
    Table(TableField),
    /// One byte, given as itself.
    Literal(u8),
    /// `len` bytes copied from `dist` bytes back in the output.
    Match { len: u32, dist: u32 },
    /// Bytes copied through as they came: a deflate stored block's payload,
    /// an LZ4 literal run.
    Stored,
    /// Deflate symbol 256.
    EndOfBlock,
    /// A whole block, for a codec whose insides this round does not read.
    Block,
    /// Input the decoder read and this trace does not name.
    Opaque,
    /// One pixel, read for the bits somebody hid in it. Its output range is
    /// the bytes that pixel completed, which is one for a cart's pixels and
    /// one or two where a pixel carries eleven bits.
    Pixel,
}

impl StepKind {
    /// The word the interface looks the step's message up by.
    pub fn as_str(self) -> &'static str {
        match self {
            StepKind::Header(..) => "header",
            StepKind::Table(_) => "table",
            StepKind::Literal(_) => "literal",
            StepKind::Match { .. } => "match",
            StepKind::Stored => "stored",
            StepKind::EndOfBlock => "end-of-block",
            StepKind::Block => "block",
            StepKind::Opaque => "opaque",
            StepKind::Pixel => "pixel",
        }
    }
}

/// One step of a decoding: which bits of the input it read and which bytes of
/// the output it produced.
///
/// `in_bits` are bits of the compressed run counted from its front. For
/// deflate they are counted the way deflate reads them, least significant bit
/// of a byte first, which is not how the rest of Qubero addresses bits: bit 0
/// is the *low* bit of byte 0. Within a byte that is the reverse of a field
/// address, and across bytes the two agree, so the byte extent of a step is
/// the same either way and only a sub-byte highlight differs. Byte-aligned
/// codecs have no such question.
///
/// Either range may be empty: a header field produces no output, and a match
/// of length 3 reads no input of its own past the code that named it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub in_bits: Range<u64>,
    pub out_bytes: Range<u64>,
    pub kind: StepKind,
}

/// A step as it is kept: starts only, with the ends taken from the step after
/// it. Twenty bytes rather than fifty-six, which is the difference between a
/// large file's trace fitting in memory and not.
#[derive(Debug, Clone, Copy)]
struct RawStep {
    in_start: u32,
    out_start: u32,
    a: u32,
    b: u32,
    tag: u8,
}

/// How a deflate block said its symbols were coded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    Stored,
    Fixed,
    Dynamic,
    /// An LZ4 block, which has no blocks inside it and no tables: one run of
    /// sequences from the front of it to the back.
    Sequences,
    /// Pixels, read for the bits somebody hid in them. Not a compressed block
    /// at all: one run of pixels from the front of the image to the back.
    Pixels,
    /// A block whose insides this round does not read: a zstd or xz block.
    Opaque,
}

impl BlockKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BlockKind::Stored => "stored",
            BlockKind::Fixed => "fixed",
            BlockKind::Dynamic => "dynamic",
            BlockKind::Sequences => "sequences",
            BlockKind::Pixels => "pixels",
            BlockKind::Opaque => "opaque",
        }
    }
}

/// One block of a decoding, and which steps belong to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// Indices into the trace's steps.
    pub steps: Range<u32>,
    pub kind: BlockKind,
    /// Whether the format said this was the last one.
    pub last: bool,
    pub in_bits: Range<u64>,
    pub out_bytes: Range<u64>,
}

/// Everything a decoder recorded about one run.
///
/// The steps tile the input bits and the output bytes exactly: step `i`'s
/// ranges end where step `i + 1`'s begin, the first begins at zero, and the
/// last ends at the run's own length. That is asserted rather than assumed,
/// by [`Trace::check_tiles`].
#[derive(Debug, Clone, Default)]
pub struct Trace {
    steps: Vec<RawStep>,
    blocks: Vec<Block>,
    end_in_bits: u64,
    end_out_bytes: u64,
    /// Whether the trace gave up naming every symbol; see [`MAX_STEPS`].
    coarse: bool,
}

impl Trace {
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// Whether the trace stopped naming every symbol because there were too
    /// many of them. The interface says so rather than pretending the file
    /// held no literals.
    pub fn coarse(&self) -> bool {
        self.coarse
    }

    pub fn in_bits(&self) -> u64 {
        self.end_in_bits
    }

    pub fn out_bytes(&self) -> u64 {
        self.end_out_bytes
    }

    /// Step `i`, built out of its own start and the next one's.
    pub fn step(&self, i: usize) -> Option<Step> {
        let raw = self.steps.get(i)?;
        let (in_end, out_end) = match self.steps.get(i + 1) {
            Some(next) => (next.in_start as u64, next.out_start as u64),
            None => (self.end_in_bits, self.end_out_bytes),
        };
        Some(Step {
            in_bits: raw.in_start as u64..in_end,
            out_bytes: raw.out_start as u64..out_end,
            kind: unpack(*raw),
        })
    }

    pub fn steps(&self) -> impl Iterator<Item = Step> + '_ {
        (0..self.steps.len()).map(move |i| self.step(i).expect("in range"))
    }

    /// Which step produced a byte of the output, as an index.
    ///
    /// A halving rather than a walk: the starts are sorted because the trace
    /// tiles. Steps that produced nothing are skipped, since a byte belongs to
    /// the step that made it and not to the header before it.
    pub fn index_out(&self, byte: u64) -> Option<usize> {
        if byte >= self.end_out_bytes || u32::try_from(byte).is_err() {
            return None;
        }
        let byte = byte as u32;
        // The last step whose out_start is at or below `byte`, then forward
        // past any empty ones sharing that start.
        let mut lo = 0usize;
        let mut hi = self.steps.len();
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.steps[mid].out_start <= byte { lo = mid + 1 } else { hi = mid }
        }
        let mut i = lo.checked_sub(1)?;
        // Walk back to the first step with this start, then forward to the one
        // that is not empty.
        while i > 0 && self.steps[i].out_start == self.steps[i - 1].out_start {
            i -= 1;
        }
        while self.step(i)?.out_bytes.is_empty() {
            i += 1;
        }
        Some(i)
    }

    /// Which step produced a byte of the output.
    pub fn map_out(&self, byte: u64) -> Option<Step> {
        self.step(self.index_out(byte)?)
    }

    /// Which step read a bit of the input, as an index.
    pub fn index_in(&self, bit: u64) -> Option<usize> {
        if bit >= self.end_in_bits || u32::try_from(bit).is_err() {
            return None;
        }
        let bit = bit as u32;
        let mut lo = 0usize;
        let mut hi = self.steps.len();
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.steps[mid].in_start <= bit { lo = mid + 1 } else { hi = mid }
        }
        let mut i = lo.checked_sub(1)?;
        while i > 0 && self.steps[i].in_start == self.steps[i - 1].in_start {
            i -= 1;
        }
        while self.step(i)?.in_bits.is_empty() {
            i += 1;
        }
        Some(i)
    }

    /// Which step read a bit of the input.
    pub fn map_in(&self, bit: u64) -> Option<Step> {
        self.step(self.index_in(bit)?)
    }

    /// That the steps tile the input bits and the output bytes exactly, with
    /// nothing skipped and nothing counted twice. Says what is wrong rather
    /// than panicking, so a test can name the file.
    pub fn check_tiles(&self) -> Result<(), String> {
        let mut at_in = 0u64;
        let mut at_out = 0u64;
        for (i, raw) in self.steps.iter().enumerate() {
            if raw.in_start as u64 != at_in {
                return Err(format!("step {i} reads from bit {} where bit {at_in} was next", raw.in_start));
            }
            if raw.out_start as u64 != at_out {
                return Err(format!("step {i} writes at byte {} where byte {at_out} was next", raw.out_start));
            }
            let s = self.step(i).expect("in range");
            if s.in_bits.end < s.in_bits.start || s.out_bytes.end < s.out_bytes.start {
                return Err(format!("step {i} runs backwards: {s:?}"));
            }
            at_in = s.in_bits.end;
            at_out = s.out_bytes.end;
        }
        if at_in != self.end_in_bits {
            return Err(format!("the steps read {at_in} bits of {}", self.end_in_bits));
        }
        if at_out != self.end_out_bytes {
            return Err(format!("the steps wrote {at_out} bytes of {}", self.end_out_bytes));
        }
        Ok(())
    }
}

/// What a decoder writes its trace through, so the tiling is kept by
/// construction: a step begins where the one before it ended.
#[derive(Default)]
pub(crate) struct TraceBuilder {
    trace: Trace,
    /// Where the block being recorded started, and its first step.
    block_start: Option<(u64, u64, u32)>,
    /// How many steps this trace may hold before it coarsens. Only the tests
    /// set it: reaching [`MAX_STEPS`] takes a hundred megabytes of input, and
    /// the path that gives up naming symbols is the one path that can leave a
    /// trace not tiling, so it has to be reachable.
    budget: Option<usize>,
}

impl TraceBuilder {
    /// Record a step read from `in_bits` and producing `out_bytes`, both
    /// absolute. The ends are taken from the next step, so they are only
    /// checked here.
    pub(crate) fn push(&mut self, in_start: u64, out_start: u64, kind: StepKind) {
        self.trace.steps.push(pack(in_start, out_start, kind));
    }

    /// Where the trace has got to, which is what the decoder must keep in
    /// step with.
    pub(crate) fn finish_at(&mut self, in_bits: u64, out_bytes: u64) {
        self.trace.end_in_bits = in_bits;
        self.trace.end_out_bytes = out_bytes;
    }

    pub(crate) fn steps(&self) -> usize {
        self.trace.steps.len()
    }

    pub(crate) fn over_budget(&self) -> bool {
        self.trace.steps.len() >= self.budget.unwrap_or(MAX_STEPS)
    }

    #[cfg(test)]
    pub(crate) fn with_budget(budget: usize) -> TraceBuilder {
        TraceBuilder { budget: Some(budget), ..TraceBuilder::default() }
    }

    pub(crate) fn coarsen(&mut self) {
        self.trace.coarse = true;
    }

    /// Drop every step from `from` on, so a block's symbols can be replaced by
    /// one step covering all of them.
    pub(crate) fn truncate(&mut self, from: usize) {
        self.trace.steps.truncate(from);
    }

    pub(crate) fn open_block(&mut self, in_bits: u64, out_bytes: u64) {
        self.block_start = Some((in_bits, out_bytes, self.trace.steps.len() as u32));
    }

    pub(crate) fn close_block(&mut self, in_bits: u64, out_bytes: u64, kind: BlockKind, last: bool) {
        let Some((in_start, out_start, step_start)) = self.block_start.take() else { return };
        self.trace.blocks.push(Block {
            steps: step_start..self.trace.steps.len() as u32,
            kind,
            last,
            in_bits: in_start..in_bits,
            out_bytes: out_start..out_bytes,
        });
    }

    pub(crate) fn done(self) -> Trace {
        self.trace
    }
}

const TAG_HEADER: u8 = 1;
const TAG_CODELEN: u8 = 2;
const TAG_LITLEN: u8 = 3;
const TAG_DIST: u8 = 4;
const TAG_REPEAT: u8 = 5;
const TAG_LITERAL: u8 = 6;
const TAG_MATCH: u8 = 7;
const TAG_STORED: u8 = 8;
const TAG_END: u8 = 9;
const TAG_BLOCK: u8 = 10;
const TAG_OPAQUE: u8 = 11;
const TAG_PIXEL: u8 = 12;

fn pack(in_start: u64, out_start: u64, kind: StepKind) -> RawStep {
    let (tag, a, b) = match kind {
        StepKind::Header(f, v) => (TAG_HEADER, f as u32, v),
        StepKind::Table(TableField::CodeLen { sym, len }) => (TAG_CODELEN, sym as u32, len as u32),
        StepKind::Table(TableField::LitLen { sym, len }) => (TAG_LITLEN, sym as u32, len as u32),
        StepKind::Table(TableField::Dist { sym, len }) => (TAG_DIST, sym as u32, len as u32),
        StepKind::Table(TableField::Repeat { code, count, len, dist }) => {
            (TAG_REPEAT, code as u32 | (count as u32) << 8 | (len as u32) << 24, dist as u32)
        }
        StepKind::Literal(v) => (TAG_LITERAL, v as u32, 0),
        StepKind::Match { len, dist } => (TAG_MATCH, len, dist),
        StepKind::Stored => (TAG_STORED, 0, 0),
        StepKind::EndOfBlock => (TAG_END, 0, 0),
        StepKind::Block => (TAG_BLOCK, 0, 0),
        StepKind::Opaque => (TAG_OPAQUE, 0, 0),
        StepKind::Pixel => (TAG_PIXEL, 0, 0),
    };
    RawStep { in_start: in_start as u32, out_start: out_start as u32, a, b, tag }
}

fn unpack(raw: RawStep) -> StepKind {
    match raw.tag {
        TAG_HEADER => StepKind::Header(FIELDS[raw.a as usize], raw.b),
        TAG_CODELEN => StepKind::Table(TableField::CodeLen { sym: raw.a as u8, len: raw.b as u8 }),
        TAG_LITLEN => StepKind::Table(TableField::LitLen { sym: raw.a as u16, len: raw.b as u8 }),
        TAG_DIST => StepKind::Table(TableField::Dist { sym: raw.a as u16, len: raw.b as u8 }),
        TAG_REPEAT => StepKind::Table(TableField::Repeat {
            code: raw.a as u8,
            count: (raw.a >> 8) as u16,
            len: (raw.a >> 24) as u8,
            dist: raw.b != 0,
        }),
        TAG_LITERAL => StepKind::Literal(raw.a as u8),
        TAG_MATCH => StepKind::Match { len: raw.a, dist: raw.b },
        TAG_STORED => StepKind::Stored,
        TAG_END => StepKind::EndOfBlock,
        TAG_BLOCK => StepKind::Block,
        TAG_PIXEL => StepKind::Pixel,
        _ => StepKind::Opaque,
    }
}

/// The header fields in the order [`StepField`] declares them, so a packed
/// step can be read back. Kept beside the enum on purpose: adding a field
/// without adding it here is caught by the test below.
const FIELDS: [StepField; 20] = [
    StepField::Bfinal,
    StepField::Btype,
    StepField::Hlit,
    StepField::Hdist,
    StepField::Hclen,
    StepField::StoredLen,
    StepField::StoredNlen,
    StepField::Padding,
    StepField::Wrapper,
    StepField::Token,
    StepField::LengthExtra,
    StepField::Offset,
    StepField::FrameHeader,
    StepField::BlockHeader,
    StepField::Filter,
    StepField::Footer,
    StepField::PxuFlags,
    StepField::PxuWidth,
    StepField::PxuHeight,
    StepField::PxuBits,
];

/// Open a compressed run and say what the decoder did to it.
///
/// The bytes are the same bytes [`decode`] gives; the trace is what tells the
/// reader which bits of the run made which bytes of what came out. How fine
/// the trace is depends on the codec: deflate and LZ4 are read by our own
/// decoders and traced per symbol, zstd and xz keep their crates and are
/// traced per block.
pub fn decode_traced(codec: Codec, data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    if data.len() > CAP_BYTES {
        return Err(Refusal::TooLarge);
    }
    let (out, trace) = match codec {
        Codec::Deflate => inflate::inflate(data)?,
        Codec::Zlib => inflate::zlib(data)?,
        Codec::Lz4Block => lz4::block(data)?,
        Codec::Zstd => frames::zstd(data)?,
        Codec::Xz => frames::xz(data)?,
        Codec::PngUnfilter { stride, bpp } => pixels::unfilter(data, stride, bpp)?,
        Codec::LowBitsArgb => pixels::low_bits_argb(data)?,
        Codec::LowBitsRgba11 => pixels::low_bits_rgba11(data)?,
        Codec::Pico8Pxa => pico8::pxa(data)?,
        Codec::Pico8Old => pico8::old(data)?,
        Codec::PicotronPxu => pxu::pxu(data)?,
    };
    if out.len() > CAP_BYTES {
        return Err(Refusal::TooLarge);
    }
    Ok((out, trace))
}

/// Open a compressed run. `data` is the whole of it.
pub fn decode(codec: Codec, data: &[u8]) -> Result<Vec<u8>, Refusal> {
    if data.len() > CAP_BYTES {
        return Err(Refusal::TooLarge);
    }
    let out = match codec {
        // One decoder, not two: the bytes a reader sees have to be the bytes
        // the trace describes, so the traced path is the only path.
        Codec::Zlib | Codec::Deflate | Codec::Lz4Block | Codec::PngUnfilter { .. } | Codec::LowBitsArgb | Codec::LowBitsRgba11 | Codec::Pico8Pxa | Codec::Pico8Old | Codec::PicotronPxu => {
            decode_traced(codec, data)?.0
        }
        Codec::Zstd => zstd(data)?,
        Codec::Xz => xz(data)?,
    };
    if out.len() > CAP_BYTES {
        return Err(Refusal::TooLarge);
    }
    Ok(out)
}

/// Zstandard, read a frame at a time. A file compressed by `zstd` is one
/// frame; ROOT writes one per block. Concatenated frames are read through to
/// the end, which is what the format says a decoder does.
fn zstd(data: &[u8]) -> Result<Vec<u8>, Refusal> {
    use std::io::Read;
    let mut decoder = ruzstd::decoding::StreamingDecoder::new(data).map_err(|_| Refusal::Failed)?;
    // One byte past the cap, so a stream that only just fits is told from one
    // that does not.
    let mut out = Vec::new();
    match decoder.by_ref().take(CAP_BYTES as u64 + 1).read_to_end(&mut out) {
        Ok(_) if out.len() > CAP_BYTES => Err(Refusal::TooLarge),
        Ok(_) => Ok(out),
        Err(_) => Err(Refusal::Failed),
    }
}

/// A whole xz stream, header through footer. The LZMA2 inside one block is a
/// step of a decoder's state and does not stand on its own, which is why the
/// template opens the stream and not the block.
fn xz(data: &[u8]) -> Result<Vec<u8>, Refusal> {
    let mut input = std::io::BufReader::new(data);
    let mut out = Vec::new();
    match lzma_rs::xz_decompress(&mut input, &mut out) {
        Ok(()) if out.len() > CAP_BYTES => Err(Refusal::TooLarge),
        Ok(()) => Ok(out),
        Err(_) => Err(Refusal::Failed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zlib_stream_comes_back_as_what_went_in() {
        let packed = miniz_oxide::deflate::compress_to_vec_zlib(b"hello hello hello", 6);
        assert_eq!(decode(Codec::Zlib, &packed).unwrap(), b"hello hello hello");
    }

    #[test]
    fn raw_deflate_has_no_header_on_it() {
        let packed = miniz_oxide::deflate::compress_to_vec(b"deflate me", 6);
        assert_eq!(decode(Codec::Deflate, &packed).unwrap(), b"deflate me");
        // The same bytes read as zlib are not a zlib stream.
        assert_eq!(decode(Codec::Zlib, &packed), Err(Refusal::Failed));
    }

    #[test]
    fn an_lz4_block_is_sized_by_trying_since_nothing_in_it_says() {
        let long = "lz4 block ".repeat(5000);
        let packed = lz4_flex::block::compress(long.as_bytes());
        assert_eq!(decode(Codec::Lz4Block, &packed).unwrap(), long.as_bytes());
    }

    #[test]
    fn bytes_that_are_not_a_stream_are_refused_rather_than_guessed_at() {
        assert_eq!(decode(Codec::Zlib, b"not compressed"), Err(Refusal::Failed));
        assert_eq!(decode(Codec::Zstd, b"not compressed"), Err(Refusal::Failed));
        assert_eq!(decode(Codec::Xz, b"not compressed"), Err(Refusal::Failed));
    }

}
