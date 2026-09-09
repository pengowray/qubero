//! Deflate, RFC 1951, read a symbol at a time and writing down what it read.
//!
//! There are good inflaters on crates.io and this is not here to beat them. It
//! is here because none of them will say *where*: which bits held the block
//! header, which bits held the Huffman code lengths, which bit the literal `h`
//! came out of and which three bytes the match after it copied. That is the
//! whole of what a hex editor wants from a compressed run, and it is the one
//! thing a decoder built to be fast throws away.
//!
//! So the bytes are checked against `miniz_oxide` in the tests and the trace is
//! checked against itself: the steps tile the input bits and the output bytes
//! exactly, or the run is refused.
//!
//! Bits are read the way deflate reads them, least significant bit of a byte
//! first, and bit positions in the trace count that way too. See [`Step`].

use crate::codec::{
    BlockKind, Refusal, Step, StepField, StepKind, TableField, Trace, TraceBuilder, CAP_BYTES,
};

/// The longest a Huffman code may be in deflate.
const MAX_BITS: usize = 15;

/// What each length code past 256 starts at, and how many extra bits it reads.
const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] =
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];

const DIST_BASE: [u32; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145,
    8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] =
    [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

/// The order the code-length code's own lengths are written in, which puts the
/// ones a short table is likely to use first so the rest can be left out.
const CL_ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

/// Bits of the run, least significant first, with a hard end.
struct Bits<'a> {
    data: &'a [u8],
    /// Where the next bit comes from, counted from the front of `data`.
    pos: u64,
    /// One past the last bit that may be read.
    end: u64,
}

impl<'a> Bits<'a> {
    fn bit(&mut self) -> Result<u32, Refusal> {
        if self.pos >= self.end {
            return Err(Refusal::Failed);
        }
        let byte = self.data[(self.pos / 8) as usize];
        let b = (byte >> (self.pos % 8)) & 1;
        self.pos += 1;
        Ok(b as u32)
    }

    /// `n` bits as a number, the first one read being the least significant.
    fn bits(&mut self, n: u32) -> Result<u32, Refusal> {
        let mut v = 0u32;
        for i in 0..n {
            v |= self.bit()? << i;
        }
        Ok(v)
    }

    /// Forward to the next byte boundary, giving back how many bits that was.
    fn align(&mut self) -> u64 {
        let skip = (8 - self.pos % 8) % 8;
        self.pos += skip;
        skip
    }
}

/// A canonical Huffman code, kept as zlib's reference decoder keeps one: how
/// many codes there are of each length, and the symbols in canonical order.
/// Decoding walks a bit at a time, which is slower than a lookup table and
/// short enough to be obviously right.
struct Code {
    counts: [u16; MAX_BITS + 1],
    symbols: Vec<u16>,
}

impl Code {
    /// Build from one length per symbol. A code that does not use up all its
    /// space is refused, except the one case the format allows: a distance
    /// table with a single symbol in it, which a block using no matches or
    /// only one distance writes.
    fn build(lengths: &[u8], allow_incomplete: bool) -> Result<Code, Refusal> {
        let mut counts = [0u16; MAX_BITS + 1];
        for &l in lengths {
            if l as usize > MAX_BITS {
                return Err(Refusal::Failed);
            }
            counts[l as usize] += 1;
        }
        let used: u32 = counts[1..].iter().map(|&c| c as u32).sum();
        if used == 0 {
            // No codes at all: legal for the distance table of a block that
            // holds no matches, and never decodable, which is what it means.
            return Ok(Code { counts, symbols: Vec::new() });
        }
        // Kraft's inequality, counted in units of 2^-15.
        let mut left = 1i32;
        for len in 1..=MAX_BITS {
            left <<= 1;
            left -= counts[len] as i32;
            if left < 0 {
                return Err(Refusal::Failed);
            }
        }
        if left > 0 && !(allow_incomplete && used == 1) {
            return Err(Refusal::Failed);
        }
        let mut offs = [0u16; MAX_BITS + 2];
        for len in 1..=MAX_BITS {
            offs[len + 1] = offs[len] + counts[len];
        }
        let mut symbols = vec![0u16; used as usize];
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[offs[l as usize] as usize] = sym as u16;
                offs[l as usize] += 1;
            }
        }
        Ok(Code { counts, symbols })
    }

    fn decode(&self, bits: &mut Bits) -> Result<u16, Refusal> {
        let mut code = 0i32;
        let mut first = 0i32;
        let mut index = 0i32;
        for len in 1..=MAX_BITS {
            code |= bits.bit()? as i32;
            let count = self.counts[len] as i32;
            if code - count < first {
                return Ok(self.symbols[(index + (code - first)) as usize]);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err(Refusal::Failed)
    }
}

/// The fixed code every deflate stream may use without writing a table.
fn fixed_codes() -> (Code, Code) {
    let mut lit = [0u8; 288];
    for (i, l) in lit.iter_mut().enumerate() {
        *l = match i {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    // Thirty-two codes of five bits, of which the format leaves the last two
    // unused: a code that did not fill its space would not be a code.
    let dist = [5u8; 32];
    (Code::build(&lit, false).expect("the fixed code is a code"), Code::build(&dist, false).expect("likewise"))
}

/// A whole raw deflate stream: the bytes it comes to and what was read where.
pub fn inflate(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    let mut b = TraceBuilder::default();
    let mut out = Vec::new();
    run(data, 0, data.len() as u64 * 8, CAP_BYTES, &mut out, &mut b)?;
    Ok((out, b.done()))
}

/// The same, with a step budget the tests can reach. See [`crate::codec::MAX_STEPS`].
#[cfg(test)]
fn inflate_within(data: &[u8], budget: usize) -> Result<(Vec<u8>, Trace), Refusal> {
    let mut b = TraceBuilder::with_budget(budget);
    let mut out = Vec::new();
    run(data, 0, data.len() as u64 * 8, CAP_BYTES, &mut out, &mut b)?;
    Ok((out, b.done()))
}

/// A zlib stream, RFC 1950: two bytes of header, deflate, and an Adler-32.
///
/// The wrapper's own bytes are steps too, so the trace tiles the run the
/// template named rather than only the middle of it.
pub fn zlib(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    if data.len() < 6 {
        return Err(Refusal::Failed);
    }
    let (cmf, flg) = (data[0], data[1]);
    if cmf & 0x0f != 8 || (cmf as u16 * 256 + flg as u16) % 31 != 0 || flg & 0x20 != 0 {
        return Err(Refusal::Failed);
    }
    let mut b = TraceBuilder::default();
    b.push(0, 0, StepKind::Header(StepField::Wrapper, 0));
    let end = (data.len() as u64 - 4) * 8;
    let mut out = Vec::new();
    run(data, 16, end, CAP_BYTES, &mut out, &mut b)?;
    let adler = u32::from_be_bytes([data[data.len() - 4], data[data.len() - 3], data[data.len() - 2], data[data.len() - 1]]);
    if adler != crate::checksum::adler32(&out) {
        return Err(Refusal::Failed);
    }
    b.push(end, out.len() as u64, StepKind::Header(StepField::Wrapper, 0));
    b.finish_at(data.len() as u64 * 8, out.len() as u64);
    Ok((out, b.done()))
}

/// A whole gzip member, RFC 1952: the header and its optional name and
/// comment, deflate, and a CRC-32 with the length after it.
///
/// The same shape as [`zlib`] with a longer front and a different sum, and it
/// exists for the same reason [`Codec::Gzip`](crate::codec::Codec::Gzip) does:
/// `gzip.rs` reads a `.gz` file's wrapper as fields and hands the middle of it
/// to `Codec::Deflate`, which is right when the member is the file and no use
/// to a format that embeds a whole one, as Godot's third compression mode
/// does.
///
/// One member, not a file. A `.gz` may hold several written end to end, and a
/// run holding two is refused rather than half-read: the trailer this looks at
/// is the last eight bytes, which belong to the member this did not decode.
pub fn gzip(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    // The two magic bytes, the method, the flags, four of time, one of how
    // hard the packer tried and one of what wrote it.
    const FIXED: usize = 10;
    // The CRC-32 of what came out and how long it was, both little-endian,
    // unlike zlib's big-endian Adler.
    const TRAILER: usize = 8;
    if data.len() < FIXED + TRAILER || data[0] != 0x1f || data[1] != 0x8b || data[2] != 8 {
        return Err(Refusal::Failed);
    }
    let flags = data[3];
    // The top three flag bits are reserved and a decoder is told to refuse a
    // member that sets one.
    if flags & 0xe0 != 0 {
        return Err(Refusal::Failed);
    }
    let mut at = FIXED;
    if flags & 0x04 != 0 {
        // Extra fields, as a length and then that many bytes.
        let len = u16::from_le_bytes([byte(data, at)?, byte(data, at + 1)?]) as usize;
        at = at.checked_add(2 + len).ok_or(Refusal::Failed)?;
    }
    if flags & 0x08 != 0 {
        at = past_nul(data, at)?; // the name the file had before it was packed
    }
    if flags & 0x10 != 0 {
        at = past_nul(data, at)?; // a comment, which almost nothing writes
    }
    if flags & 0x02 != 0 {
        // Two bytes of CRC over the header. Not checked: nothing writes one,
        // and refusing a member for a sum this does not compute would be
        // refusing it for the wrong reason.
        at = at.checked_add(2).ok_or(Refusal::Failed)?;
    }
    if at + TRAILER > data.len() {
        return Err(Refusal::Failed);
    }
    let mut b = TraceBuilder::default();
    b.push(0, 0, StepKind::Header(StepField::Wrapper, 0));
    let end = (data.len() - TRAILER) as u64 * 8;
    let mut out = Vec::new();
    run(data, at as u64 * 8, end, CAP_BYTES, &mut out, &mut b)?;
    let tail = &data[data.len() - TRAILER..];
    let crc = u32::from_le_bytes([tail[0], tail[1], tail[2], tail[3]]);
    let size = u32::from_le_bytes([tail[4], tail[5], tail[6], tail[7]]);
    if crc != crate::checksum::crc32(&out) || size != out.len() as u32 {
        return Err(Refusal::Failed);
    }
    b.push(end, out.len() as u64, StepKind::Header(StepField::Wrapper, 0));
    b.finish_at(data.len() as u64 * 8, out.len() as u64);
    Ok((out, b.done()))
}

fn byte(data: &[u8], at: usize) -> Result<u8, Refusal> {
    data.get(at).copied().ok_or(Refusal::Failed)
}

/// Past a NUL-terminated string in the header, or a refusal when the member
/// ends before the terminator does.
fn past_nul(data: &[u8], at: usize) -> Result<usize, Refusal> {
    let rest = data.get(at..).ok_or(Refusal::Failed)?;
    let n = rest.iter().position(|&b| b == 0).ok_or(Refusal::Failed)?;
    Ok(at + n + 1)
}

/// As much of a zlib stream as the bytes on hand come to, with no complaint
/// about the bytes that are not there.
///
/// A sniff sees the front of a file and no more, so a zlib stream inside it is
/// nearly always cut off partway through a block. [`zlib`] calls that a
/// refusal, and rightly: the Adler-32 is missing and no block ever said it was
/// the last. This entry point runs the same decoder and hands back whatever the
/// stream had produced by the time it ran out, which is what a caller wanting
/// the first rows of an image is after. The trace is dropped; there is nothing
/// worth pointing at in a file only partly read.
///
/// `cap` is the most output to keep. Deflate packs a 258-byte match into two
/// bits, so a few tens of kilobytes of hostile input come to tens of megabytes
/// of output, and a caller which knows how big the whole thing should be says
/// so here rather than paying for that.
///
/// An empty vector comes back if the two header bytes are not a zlib header.
pub fn inflate_prefix(data: &[u8], cap: usize) -> Vec<u8> {
    if data.len() < 2 {
        return Vec::new();
    }
    let (cmf, flg) = (data[0], data[1]);
    if cmf & 0x0f != 8 || (cmf as u16 * 256 + flg as u16) % 31 != 0 || flg & 0x20 != 0 {
        return Vec::new();
    }
    let mut b = TraceBuilder::default();
    let mut out = Vec::new();
    let _ = run(data, 16, data.len() as u64 * 8, cap, &mut out, &mut b);
    out
}

/// The blocks between `start` and `end`, bits of `data`, written into `out`.
///
/// The bytes go into a vector the caller owns rather than one made here, so
/// that a caller reading a stream which stops early keeps what came out before
/// it stopped. `cap` is the most output the run may produce; past it the run is
/// refused as [`Refusal::TooLarge`].
fn run(
    data: &[u8],
    start: u64,
    end: u64,
    cap: usize,
    out: &mut Vec<u8>,
    b: &mut TraceBuilder,
) -> Result<(), Refusal> {
    if end > data.len() as u64 * 8 || start > end {
        return Err(Refusal::Failed);
    }
    let mut bits = Bits { data, pos: start, end };
    let mut coarse = false;
    loop {
        let block_in = bits.pos;
        let block_out = out.len() as u64;
        b.open_block(block_in, block_out);
        let at = bits.pos;
        let last = bits.bit()? == 1;
        b.push(at, out.len() as u64, StepKind::Header(StepField::Bfinal, last as u32));
        let at = bits.pos;
        let btype = bits.bits(2)?;
        b.push(at, out.len() as u64, StepKind::Header(StepField::Btype, btype));
        let kind = match btype {
            0 => {
                stored(&mut bits, out, cap, b)?;
                BlockKind::Stored
            }
            1 => {
                let (lit, dist) = fixed_codes();
                symbols(&mut bits, out, cap, b, &lit, &dist, &mut coarse)?;
                BlockKind::Fixed
            }
            2 => {
                let (lit, dist) = dynamic_tables(&mut bits, out.len() as u64, b)?;
                symbols(&mut bits, out, cap, b, &lit, &dist, &mut coarse)?;
                BlockKind::Dynamic
            }
            _ => return Err(Refusal::Failed),
        };
        b.close_block(bits.pos, out.len() as u64, kind, last);
        if last {
            break;
        }
    }
    // The bits between the last block and the byte boundary, which the format
    // does not use and a decoder reads past.
    let at = bits.pos;
    let skipped = bits.align();
    if skipped > 0 {
        b.push(at, out.len() as u64, StepKind::Header(StepField::Padding, 0));
    }
    // Anything after the stream inside the run the template named. A zlib
    // trailer arrives here as its own step; anything else is bytes nobody
    // claimed, and saying so is better than pretending the run ended early.
    if bits.pos < end {
        b.push(bits.pos, out.len() as u64, StepKind::Opaque);
    }
    b.finish_at(end.max(bits.pos), out.len() as u64);
    Ok(())
}

/// A stored block: the rest of the byte, a length, its complement, and that
/// many bytes as they are.
fn stored(bits: &mut Bits, out: &mut Vec<u8>, cap: usize, b: &mut TraceBuilder) -> Result<(), Refusal> {
    let at = bits.pos;
    if bits.align() > 0 {
        b.push(at, out.len() as u64, StepKind::Header(StepField::Padding, 0));
    }
    let at = bits.pos;
    let len = bits.bits(16)?;
    b.push(at, out.len() as u64, StepKind::Header(StepField::StoredLen, len));
    let at = bits.pos;
    let nlen = bits.bits(16)?;
    b.push(at, out.len() as u64, StepKind::Header(StepField::StoredNlen, nlen));
    if len ^ 0xffff != nlen {
        return Err(Refusal::Failed);
    }
    let start = (bits.pos / 8) as usize;
    let stop = start + len as usize;
    if stop as u64 * 8 > bits.end {
        return Err(Refusal::Failed);
    }
    if out.len() + len as usize > cap {
        return Err(Refusal::TooLarge);
    }
    if len > 0 {
        b.push(bits.pos, out.len() as u64, StepKind::Stored);
        out.extend_from_slice(&bits.data[start..stop]);
        bits.pos = stop as u64 * 8;
    }
    Ok(())
}

/// A dynamic block's two tables: how many lengths there are, the code-length
/// alphabet, and then the lengths themselves, run-length coded.
fn dynamic_tables(bits: &mut Bits, out_at: u64, b: &mut TraceBuilder) -> Result<(Code, Code), Refusal> {
    let at = bits.pos;
    let hlit = bits.bits(5)? as usize + 257;
    b.push(at, out_at, StepKind::Header(StepField::Hlit, hlit as u32));
    let at = bits.pos;
    let hdist = bits.bits(5)? as usize + 1;
    b.push(at, out_at, StepKind::Header(StepField::Hdist, hdist as u32));
    let at = bits.pos;
    let hclen = bits.bits(4)? as usize + 4;
    b.push(at, out_at, StepKind::Header(StepField::Hclen, hclen as u32));
    if hlit > 286 || hdist > 30 {
        return Err(Refusal::Failed);
    }

    let mut cl = [0u8; 19];
    for &slot in CL_ORDER.iter().take(hclen) {
        let at = bits.pos;
        let len = bits.bits(3)? as u8;
        cl[slot] = len;
        b.push(at, out_at, StepKind::Table(TableField::CodeLen { sym: slot as u8, len }));
    }
    let cl_code = Code::build(&cl, false)?;

    let mut lengths = vec![0u8; hlit + hdist];
    let mut i = 0usize;
    while i < lengths.len() {
        let at = bits.pos;
        let sym = cl_code.decode(bits)?;
        let in_dist = i >= hlit;
        match sym {
            0..=15 => {
                lengths[i] = sym as u8;
                let field = if in_dist {
                    TableField::Dist { sym: (i - hlit) as u16, len: sym as u8 }
                } else {
                    TableField::LitLen { sym: i as u16, len: sym as u8 }
                };
                b.push(at, out_at, StepKind::Table(field));
                i += 1;
            }
            16 | 17 | 18 => {
                let (extra, base, value) = match sym {
                    16 => (2u32, 3u16, *lengths.get(i.wrapping_sub(1)).ok_or(Refusal::Failed)?),
                    17 => (3, 3, 0),
                    _ => (7, 11, 0),
                };
                if sym == 16 && i == 0 {
                    return Err(Refusal::Failed);
                }
                let count = base + bits.bits(extra)? as u16;
                if i + count as usize > lengths.len() {
                    return Err(Refusal::Failed);
                }
                for slot in &mut lengths[i..i + count as usize] {
                    *slot = value;
                }
                b.push(
                    at,
                    out_at,
                    StepKind::Table(TableField::Repeat { code: sym as u8, count, len: value, dist: in_dist }),
                );
                i += count as usize;
            }
            _ => return Err(Refusal::Failed),
        }
    }
    let lit = Code::build(&lengths[..hlit], false)?;
    let dist = Code::build(&lengths[hlit..], true)?;
    Ok((lit, dist))
}

/// The symbols of one Huffman-coded block, up to and including its end mark.
fn symbols(
    bits: &mut Bits,
    out: &mut Vec<u8>,
    cap: usize,
    b: &mut TraceBuilder,
    lit: &Code,
    dist: &Code,
    coarse: &mut bool,
) -> Result<(), Refusal> {
    let sym_start = b.steps();
    let sym_in = bits.pos;
    let sym_out = out.len() as u64;
    if *coarse {
        b.push(sym_in, sym_out, StepKind::Opaque);
    }
    loop {
        // Too many symbols to name one at a time: keep the map at the block,
        // and say in the trace that this is what happened.
        if !*coarse && b.over_budget() {
            *coarse = true;
            b.coarsen();
            b.truncate(sym_start);
            b.push(sym_in, sym_out, StepKind::Opaque);
        }
        let at = bits.pos;
        let sym = lit.decode(bits)?;
        match sym {
            0..=255 => {
                if out.len() >= cap {
                    return Err(Refusal::TooLarge);
                }
                out.push(sym as u8);
                if !*coarse {
                    b.push(at, out.len() as u64 - 1, StepKind::Literal(sym as u8));
                }
            }
            256 => {
                if !*coarse {
                    b.push(at, out.len() as u64, StepKind::EndOfBlock);
                }
                return Ok(());
            }
            257..=285 => {
                let i = sym as usize - 257;
                let len = LEN_BASE[i] + bits.bits(LEN_EXTRA[i] as u32)? as u16;
                let dsym = dist.decode(bits)? as usize;
                if dsym >= DIST_BASE.len() {
                    return Err(Refusal::Failed);
                }
                let d = DIST_BASE[dsym] + bits.bits(DIST_EXTRA[dsym] as u32)?;
                if d as usize > out.len() {
                    return Err(Refusal::Failed);
                }
                if out.len() + len as usize > cap {
                    return Err(Refusal::TooLarge);
                }
                let from = out.len() - d as usize;
                let was = out.len();
                // A match may read what it is writing: `dist` of 1 fills with
                // one byte, and that is the format working as intended.
                for k in 0..len as usize {
                    let byte = out[from + k];
                    out.push(byte);
                }
                if !*coarse {
                    b.push(at, was as u64, StepKind::Match { len: len as u32, dist: d });
                }
            }
            _ => return Err(Refusal::Failed),
        }
    }
}

/// One Huffman-coded number as the block wrote it down.
///
/// Deflate never writes a number: it writes a symbol, and the symbol names a
/// base and how many bits follow it. A length of 11 is symbol 265 and one
/// extra bit; a length of 3 is symbol 257 and no extra bits at all. A reader
/// looking at thirteen bits of a file and told only "match 3 back 4" has been
/// given the answer and none of the working, and cannot check that thirteen is
/// the right number of bits for it. This is the working.
///
/// `code_bits + extra_bits` is what this number cost, and the widths of a
/// step's codes add up to the step's own `in_bits` exactly. See
/// [`DecodedStep::bits`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodedCode {
    /// Which symbol of the alphabet the Huffman code decoded to.
    pub symbol: u16,
    /// How wide that code was. Not stored anywhere: it is a fact about the
    /// table the block declared, so it comes back only by rebuilding the table
    /// and walking the bits again.
    pub code_bits: u8,
    /// How many bits followed the code outright, and what they read as. Both
    /// zero for a literal, for the end mark, and for the many lengths and
    /// distances a symbol names on its own.
    pub extra_bits: u8,
    pub extra: u32,
    /// What the symbol and its extra bits came to: the byte, for a literal;
    /// the length or the distance, for the two halves of a match; and 0 for
    /// the end mark, which stands for no number.
    pub value: u32,
    /// Which step of the block's head set this symbol's code length, as an
    /// index into the trace's steps, so a panel can send a reader to the bits
    /// that decided how wide this code would be.
    ///
    /// `None` when there is nowhere to send them: a fixed block's two tables
    /// are written in RFC 1951 section 3.2.6 rather than in the file, so no
    /// step of the run declared them and no link would be honest. Also `None`
    /// for a symbol whose length the head never mentioned, which cannot happen
    /// in a table that built.
    pub entry: Option<u32>,
    /// The same step counted from the front of its block, which is where it
    /// sits among the block node's children. The trace's steps and the
    /// listing's rows are two different numberings of the same head, and a
    /// link needs the second.
    pub entry_child: Option<u32>,
}

/// What one deflate symbol step was made of, worked out again from the
/// block's tables. See [`decode_step`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodedStep {
    /// Which block of the trace the step belongs to, as an index into
    /// [`Trace::blocks`].
    pub block: u32,
    /// How that block said its symbols were coded, which is the same thing as
    /// where the two tables came from: `Dynamic` wrote them into the run and
    /// `Fixed` did not.
    pub block_kind: BlockKind,
    /// The literal/length code, which every symbol step begins with. Its
    /// `symbol` says which of the three kinds of step this is: under 256 a
    /// literal, 256 the end of the block, over 256 the length of a match.
    pub symbol: DecodedCode,
    /// The distance code that followed the length. Nothing for a literal and
    /// nothing for the end mark, which read no second code.
    pub distance: Option<DecodedCode>,
}

/// What a literal/length symbol stands for. Derived from the symbol number
/// rather than stored beside it: there is one right answer and no room for the
/// two to disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolMeaning {
    /// Symbols 0 to 255, each one byte given as itself.
    Literal(u8),
    /// Symbol 256, which ends the block and produces nothing.
    EndOfBlock,
    /// Symbols 257 to 285, each a length or the front of one.
    Length,
}

impl DecodedStep {
    /// What the first code stood for.
    pub fn meaning(&self) -> SymbolMeaning {
        match self.symbol.symbol {
            0..=255 => SymbolMeaning::Literal(self.symbol.symbol as u8),
            256 => SymbolMeaning::EndOfBlock,
            _ => SymbolMeaning::Length,
        }
    }

    /// Every bit this step read, added up: the two codes and the two runs of
    /// extra bits. The same number as the step's own `in_bits` length, and the
    /// reason the four widths are worth showing at all. A reader who is told
    /// 8 + 0 + 5 + 0 and sees the step is 13 bits wide has checked the
    /// arithmetic; one who is told only "match 3 back 4" has not.
    pub fn bits(&self) -> u32 {
        let one = |c: &DecodedCode| c.code_bits as u32 + c.extra_bits as u32;
        one(&self.symbol) + self.distance.as_ref().map_or(0, one)
    }
}

/// Take one symbol step apart: which codes carried it, how wide they were, and
/// which entry of the block's tables set each width.
///
/// None of this is in the trace. A step is a packed twenty bytes and a
/// `StepKind::Match { len, dist }` is the decoded length and distance and
/// nothing else; which symbol named that length, whether extra bits followed
/// it and how many bits the whole thing cost are all thrown away, because
/// keeping them would cost every step of every stream in memory to answer a
/// question asked of one step at a time. So this asks it again, from the run
/// and the trace, and only when somebody clicks.
///
/// The work is rebuilding the block's two Huffman tables and re-decoding the
/// step's bits with them, which for a dynamic block means walking the head
/// steps the decoder already wrote down. It reads no more of the run than the
/// step it was asked about, so it is proportional to the block's head and not
/// to the block.
///
/// `data` is the whole compressed run, from the front, because that is what
/// the trace's bit positions count from: a zlib stream's first block starts at
/// bit 16 of the run and the trace says so. A prefix long enough to hold the
/// step is enough.
///
/// `None` rather than a panic for everything that does not answer: a step that
/// is not a symbol, a block whose tables cannot be rebuilt, a stream cut off
/// before the step it names, a symbol the format does not have.
pub fn decode_step(data: &[u8], trace: &Trace, step: usize) -> Option<DecodedStep> {
    let s = trace.step(step)?;
    // Stored payload and the one opaque step a coarsened block leaves behind
    // are steps of a block and are not symbols of one. Neither was read
    // through a Huffman table, so neither has a width to give back.
    if !matches!(s.kind, StepKind::Literal(_) | StepKind::Match { .. } | StepKind::EndOfBlock) {
        return None;
    }
    let (i, block) = trace.blocks().iter().enumerate().find(|(_, b)| b.steps.contains(&(step as u32)))?;
    // Where the block's machinery stops and its symbols start. The same split
    // the listing draws, found the same way: the first step that is neither a
    // header field nor a table entry.
    let head_end = (block.steps.start..block.steps.end)
        .find(|&k| !trace.step(k as usize).is_some_and(|s| matches!(s.kind, StepKind::Header(..) | StepKind::Table(_))))
        .unwrap_or(block.steps.end);
    let head = block.steps.start..head_end;
    let (lit, dist, owner, hlit) = match block.kind {
        BlockKind::Fixed => {
            let (lit, dist) = fixed_codes();
            (lit, dist, Vec::new(), 0)
        }
        BlockKind::Dynamic => rebuild(trace, head.clone())?,
        // A stored block holds no codes, and the rest of the kinds belong to
        // other codecs entirely.
        _ => return None,
    };
    // Where a symbol's code length was declared, in both numberings the caller
    // may want. `owner` is empty for a fixed block, whose tables no step of the
    // run wrote down.
    let entry = |at: usize| match owner.get(at).copied().flatten() {
        Some(k) => (Some(k), Some(k - block.steps.start)),
        None => (None, None),
    };

    // Read the step's bits again, bounded by the step: a decode that would run
    // past the bits the decoder said it read has gone wrong, and stopping is
    // better than reading a neighbour's.
    let end = s.in_bits.end.min(data.len() as u64 * 8);
    let mut bits = Bits { data, pos: s.in_bits.start, end };
    let start = bits.pos;
    let sym = lit.decode(&mut bits).ok()?;
    let code_bits = u8::try_from(bits.pos - start).ok()?;
    let (lit_entry, lit_child) = entry(sym as usize);
    let mut first = DecodedCode {
        symbol: sym,
        code_bits,
        extra_bits: 0,
        extra: 0,
        value: sym as u32,
        entry: lit_entry,
        entry_child: lit_child,
    };
    match sym {
        // A literal and the end mark are the symbol and nothing after it, so
        // the code's width is the whole step and there is no second half.
        0..=256 => {
            if sym == 256 {
                first.value = 0;
            }
            Some(DecodedStep { block: i as u32, block_kind: block.kind, symbol: first, distance: None })
        }
        257..=285 => {
            let k = sym as usize - 257;
            let extra_bits = LEN_EXTRA[k];
            let extra = bits.bits(extra_bits as u32).ok()?;
            first.extra_bits = extra_bits;
            first.extra = extra;
            first.value = LEN_BASE[k] as u32 + extra;

            let at = bits.pos;
            let dsym = dist.decode(&mut bits).ok()?;
            let dcode_bits = u8::try_from(bits.pos - at).ok()?;
            // Symbols 30 and 31 are decodable out of the fixed distance code
            // and mean nothing; the decoder refuses them and so does this.
            let d = dsym as usize;
            if d >= DIST_BASE.len() {
                return None;
            }
            let dextra_bits = DIST_EXTRA[d];
            let dextra = bits.bits(dextra_bits as u32).ok()?;
            let (dist_entry, dist_child) = entry(hlit + d);
            let second = DecodedCode {
                symbol: dsym,
                code_bits: dcode_bits,
                extra_bits: dextra_bits,
                extra: dextra,
                value: DIST_BASE[d] + dextra,
                entry: dist_entry,
                entry_child: dist_child,
            };
            Some(DecodedStep { block: i as u32, block_kind: block.kind, symbol: first, distance: Some(second) })
        }
        _ => None,
    }
}

/// A dynamic block's two tables, rebuilt from the head steps that declared
/// them, along with which step declared each code length and where the
/// distance table starts.
///
/// One run of lengths rather than two, because that is how the format writes
/// them and how [`dynamic_tables`] reads them: `hlit` literal/length lengths
/// and `hdist` distance lengths, end to end, with the repeat codes running
/// over the join. A repeat that starts on the literal side and reaches past
/// `hlit` fills distance lengths, and a rebuild keeping a counter per table
/// would put those in the wrong one. The `dist` flag on a
/// [`TableField::Repeat`] says which side the repeat *started*, which is what
/// the decoder recorded and not a second opinion about where the entries
/// landed.
///
/// The entries given outright carry their own symbol, so the running counter
/// is only ever trusted across a repeat, which is the one entry that does not
/// say where it begins.
#[allow(clippy::type_complexity)]
fn rebuild(trace: &Trace, head: std::ops::Range<u32>) -> Option<(Code, Code, Vec<Option<u32>>, usize)> {
    let mut hlit = None;
    let mut hdist = None;
    for k in head.clone() {
        match trace.step(k as usize)?.kind {
            StepKind::Header(StepField::Hlit, v) => hlit = Some(v as usize),
            StepKind::Header(StepField::Hdist, v) => hdist = Some(v as usize),
            _ => {}
        }
    }
    // A dynamic block that did not say how long its tables are is not one this
    // can rebuild. The decoder would not have got past the header.
    let (hlit, hdist) = (hlit?, hdist?);
    if hlit > 286 || hdist > 30 || hlit < 257 || hdist < 1 {
        return None;
    }
    let mut lengths = vec![0u8; hlit + hdist];
    let mut owner: Vec<Option<u32>> = vec![None; hlit + hdist];
    let mut at = 0usize;
    for k in head {
        let StepKind::Table(field) = trace.step(k as usize)?.kind else { continue };
        match field {
            // The code-length alphabet's own lengths, which named the codes
            // that wrote the two tables and are not entries of either.
            TableField::CodeLen { .. } => {}
            TableField::LitLen { sym, len } => {
                at = sym as usize;
                *lengths.get_mut(at)? = len;
                owner[at] = Some(k);
                at += 1;
            }
            TableField::Dist { sym, len } => {
                at = hlit.checked_add(sym as usize)?;
                *lengths.get_mut(at)? = len;
                owner[at] = Some(k);
                at += 1;
            }
            TableField::Repeat { count, len, .. } => {
                let stop = at.checked_add(count as usize)?;
                if stop > lengths.len() {
                    return None;
                }
                for slot in at..stop {
                    lengths[slot] = len;
                    owner[slot] = Some(k);
                }
                at = stop;
            }
        }
    }
    let lit = Code::build(&lengths[..hlit], false).ok()?;
    let dist = Code::build(&lengths[hlit..], true).ok()?;
    Some((lit, dist, owner, hlit))
}

/// Which table a symbol step was decoded with, for a field's origin: the one
/// the block it belongs to declared.
pub fn table_of(trace: &Trace, step: usize) -> Option<BlockKind> {
    trace.blocks().iter().find(|blk| blk.steps.contains(&(step as u32))).map(|blk| blk.kind)
}

/// Whether a step is one of the symbols of a block rather than its machinery.
pub fn is_symbol(step: &Step) -> bool {
    matches!(step.kind, StepKind::Literal(_) | StepKind::Match { .. } | StepKind::EndOfBlock | StepKind::Stored)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bytes and the map, checked together: what came out has to be what
    /// `miniz_oxide` says came out, and the steps have to account for every
    /// bit of the run and every byte of the result.
    fn agrees(raw: &[u8]) {
        let theirs = miniz_oxide::inflate::decompress_to_vec_with_limit(raw, CAP_BYTES)
            .expect("miniz reads it");
        let (ours, trace) = inflate(raw).expect("we read it");
        assert_eq!(ours, theirs, "the bytes differ from miniz_oxide's");
        trace.check_tiles().expect("the trace tiles");
        assert_eq!(trace.in_bits(), raw.len() as u64 * 8);
        assert_eq!(trace.out_bytes(), ours.len() as u64);
    }

    fn roundtrip(data: &[u8], level: u8) {
        agrees(&miniz_oxide::deflate::compress_to_vec(data, level));
    }

    #[test]
    fn a_short_stream_reads_as_what_went_in() {
        for level in 0..=10 {
            roundtrip(b"hello hello hello", level);
            roundtrip(b"", level);
            roundtrip(&[0u8; 5000], level);
        }
    }

    #[test]
    fn a_zlib_stream_carries_its_wrapper_as_steps() {
        let packed = miniz_oxide::deflate::compress_to_vec_zlib(b"hello hello hello", 6);
        let (out, trace) = zlib(&packed).expect("reads");
        assert_eq!(out, b"hello hello hello");
        trace.check_tiles().expect("tiles");
        assert_eq!(trace.in_bits(), packed.len() as u64 * 8);
        let first = trace.step(0).unwrap();
        assert_eq!(first.kind, StepKind::Header(StepField::Wrapper, 0));
        assert_eq!(first.in_bits, 0..16);
        let last = trace.step(trace.len() - 1).unwrap();
        assert_eq!(last.kind, StepKind::Header(StepField::Wrapper, 0));
        assert_eq!(last.in_bits.end, packed.len() as u64 * 8);
    }

    /// Every level, over data of every shape: text that compresses, bytes that
    /// do not, and runs that turn into long matches.
    #[test]
    fn every_level_over_every_shape_of_data() {
        let mut seed = 0x1234_5678u64;
        let mut rand = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let noise: Vec<u8> = (0..70_000).map(|_| rand() as u8).collect();
        let text: Vec<u8> = "the quick brown fox jumps over the lazy dog. ".repeat(2000).into_bytes();
        let runs: Vec<u8> = (0..40_000u32).map(|i| (i / 700) as u8).collect();
        for data in [&noise[..], &text[..], &runs[..], &[][..], &[7][..]] {
            for level in 0..=10 {
                roundtrip(data, level);
            }
        }
    }

    /// Fixed-Huffman and stored blocks, which the levels above mostly do not
    /// produce, written by hand so they are certainly tested.
    #[test]
    fn a_fixed_block_and_a_stored_block() {
        // An empty fixed block: BFINAL=1, BTYPE=01, then the 7-bit code for
        // symbol 256, which is all zeroes.
        let (out, trace) = inflate(&[0x03, 0x00]).expect("reads");
        assert!(out.is_empty());
        trace.check_tiles().expect("tiles");
        assert_eq!(trace.blocks().len(), 1);
        assert_eq!(trace.blocks()[0].kind, BlockKind::Fixed);

        // A stored block: BFINAL=1, BTYPE=00, padding, LEN, NLEN, the bytes.
        let mut stored = vec![0x01, 0x05, 0x00, 0xfa, 0xff];
        stored.extend_from_slice(b"there");
        let (out, trace) = inflate(&stored).expect("reads");
        assert_eq!(out, b"there");
        trace.check_tiles().expect("tiles");
        assert_eq!(trace.blocks()[0].kind, BlockKind::Stored);
        assert!(trace.steps().any(|s| s.kind == StepKind::Stored));
        // The five bytes came from the five bytes.
        let step = trace.map_out(2).expect("a step made byte 2");
        assert_eq!(step.kind, StepKind::Stored);
        assert_eq!(step.out_bytes, 0..5);
    }

    /// A literal and a match, named and placed. `abcabcabc` is three literals
    /// and a match of six at distance three, which is what an encoder writes
    /// and what the reader wants to see.
    #[test]
    fn a_literal_says_its_byte_and_a_match_says_how_far_back() {
        let packed = miniz_oxide::deflate::compress_to_vec(b"abcabcabcabcabcabc", 9);
        let (out, trace) = inflate(&packed).expect("reads");
        assert_eq!(out, b"abcabcabcabcabcabc");
        trace.check_tiles().expect("tiles");
        let first = trace.map_out(0).expect("a step made byte 0");
        assert_eq!(first.kind, StepKind::Literal(b'a'));
        assert_eq!(first.out_bytes, 0..1);
        let later = trace.map_out(10).expect("a step made byte 10");
        assert!(matches!(later.kind, StepKind::Match { .. }), "byte 10 came from {:?}", later.kind);
        // And the bits that step read map back to it.
        let back = trace.map_in(later.in_bits.start).expect("a step read that bit");
        assert_eq!(back, later);
    }

    /// A dynamic block writes its tables down, and the trace holds them.
    #[test]
    fn a_dynamic_block_shows_the_code_lengths_it_declared() {
        let text: Vec<u8> = "structure, and the shape of it, and the shape of the shape".repeat(200).into_bytes();
        let packed = miniz_oxide::deflate::compress_to_vec(&text, 9);
        let (_, trace) = inflate(&packed).expect("reads");
        assert_eq!(trace.blocks()[0].kind, BlockKind::Dynamic);
        let heads: Vec<_> = trace
            .steps()
            .take(5)
            .map(|s| match s.kind {
                StepKind::Header(f, v) => (f, v),
                other => panic!("a block that starts with {other:?}"),
            })
            .collect();
        let fields: Vec<_> = heads.iter().map(|&(f, _)| f).collect();
        assert_eq!(
            fields,
            [StepField::Bfinal, StepField::Btype, StepField::Hlit, StepField::Hdist, StepField::Hclen]
        );
        // And each says what it read: the last block, a dynamic one, and two
        // table sizes inside what the format allows.
        assert_eq!(heads[0].1, 1);
        assert_eq!(heads[1].1, 2);
        assert!((257..=286).contains(&heads[2].1), "hlit of {}", heads[2].1);
        assert!((1..=30).contains(&heads[3].1), "hdist of {}", heads[3].1);
        assert!((4..=19).contains(&heads[4].1), "hclen of {}", heads[4].1);
        assert!(trace.steps().any(|s| matches!(s.kind, StepKind::Table(TableField::CodeLen { .. }))));
        assert!(trace.steps().any(|s| matches!(s.kind, StepKind::Table(TableField::LitLen { .. }))));
        // A table step reads bits and writes nothing.
        for s in trace.steps() {
            if matches!(s.kind, StepKind::Table(_) | StepKind::Header(..)) {
                assert!(s.out_bytes.is_empty(), "{s:?} claims to have written bytes");
            }
        }
    }

    /// Bytes that are not a stream are refused, and nothing panics. A broken
    /// block must never take the listing down with it.
    #[test]
    fn broken_streams_are_refused_rather_than_panicking() {
        let cases: Vec<Vec<u8>> = vec![
            vec![],
            vec![0x00],
            // BTYPE 3, which no stream may use.
            vec![0x07, 0x00, 0x00],
            // A stored block whose NLEN does not match.
            vec![0x01, 0x05, 0x00, 0x00, 0x00, 1, 2, 3, 4, 5],
            // A stored block that runs off the end.
            vec![0x01, 0xff, 0x00, 0x00, 0xff],
            vec![0xff; 64],
            vec![0x78, 0x9c, 0xff, 0xff, 0xff, 0xff],
        ];
        for case in cases {
            let got = inflate(&case);
            assert!(got.is_err(), "{case:?} was read as {:?}", got.map(|(o, _)| o.len()));
        }
        // And every prefix of a real stream, which is the shape a truncated
        // file has.
        let packed = miniz_oxide::deflate::compress_to_vec(b"a stream cut short somewhere", 6);
        for n in 0..packed.len() {
            let _ = inflate(&packed[..n]);
        }
        // And every one-byte corruption of a real one. Some of these are still
        // streams, of something else; what matters is that none of them is a
        // panic, and that the ones that do read still tile.
        let packed = miniz_oxide::deflate::compress_to_vec(&"corrupt me byte by byte. ".repeat(40).into_bytes(), 6);
        for i in 0..packed.len() {
            for xor in [0x01u8, 0x40, 0xff] {
                let mut bad = packed.clone();
                bad[i] ^= xor;
                if let Ok((out, trace)) = inflate(&bad) {
                    trace.check_tiles().unwrap_or_else(|e| panic!("byte {i} ^ {xor:#x}: {e}"));
                    assert_eq!(trace.out_bytes(), out.len() as u64);
                }
            }
        }
    }

    /// A trace of a stream with more symbols than the budget keeps the map and
    /// says it stopped naming them.
    #[test]
    fn a_huge_stream_coarsens_rather_than_filling_memory() {
        // Not the real budget, which would need a hundred megabytes of input
        // to reach; what is checked here is that the flag and the tiling hold
        // together, so the real thing is exercised by the same code path.
        let text: Vec<u8> = (0..200_000u32).map(|i| (i.wrapping_mul(2654435761) >> 24) as u8).collect();
        let packed = miniz_oxide::deflate::compress_to_vec(&text, 6);
        let (out, trace) = inflate(&packed).expect("reads");
        assert_eq!(out, text);
        trace.check_tiles().expect("tiles");
        assert!(!trace.coarse(), "the budget is not reached by a stream this size");

        // The same stream with a budget it does reach. The bytes are the same
        // bytes, the trace still tiles, and what is lost is only the naming:
        // each block's symbols become one step covering all of them.
        let (coarse_out, coarse) = inflate_within(&packed, 50).expect("reads");
        assert_eq!(coarse_out, out, "coarsening changed the bytes");
        coarse.check_tiles().expect("a coarse trace still tiles");
        assert!(coarse.coarse(), "the budget of 50 was not reached");
        assert!(coarse.len() < trace.len(), "coarsening kept as many steps as naming them");
        assert_eq!(coarse.blocks().len(), trace.blocks().len(), "coarsening lost a block");
        // Every byte still maps to something; a block's symbols map to the one
        // step standing for all of them.
        for byte in (0..out.len() as u64).step_by(97) {
            assert!(coarse.map_out(byte).is_some(), "byte {byte} came from nowhere");
        }
        assert!(coarse.steps().any(|s| s.kind == StepKind::Opaque));
        assert!(!coarse.steps().any(|s| matches!(s.kind, StepKind::Literal(_))));
    }

    /// Every byte of the output belongs to exactly one step, and every bit of
    /// the input to at most one.
    #[test]
    fn the_map_agrees_with_the_tiling_at_every_byte() {
        let text: Vec<u8> = "map me both ways, byte by byte and bit by bit. ".repeat(60).into_bytes();
        let packed = miniz_oxide::deflate::compress_to_vec(&text, 6);
        let (out, trace) = inflate(&packed).expect("reads");
        for byte in 0..out.len() as u64 {
            let step = trace.map_out(byte).unwrap_or_else(|| panic!("byte {byte} came from nowhere"));
            assert!(step.out_bytes.contains(&byte), "byte {byte} mapped to {step:?}");
        }
        assert!(trace.map_out(out.len() as u64).is_none());
        for bit in 0..trace.in_bits() {
            let step = trace.map_in(bit).unwrap_or_else(|| panic!("bit {bit} was read by nobody"));
            assert!(step.in_bits.contains(&bit), "bit {bit} mapped to {step:?}");
        }
        assert!(trace.map_in(trace.in_bits()).is_none());
    }

    /// Every symbol step of a stream, taken apart and checked against itself.
    ///
    /// Four things have to hold at once, and each one catches a different way
    /// of being wrong. The widths add up to the step's own, which is the
    /// property that catches an off-by-one anywhere in the walk. The values
    /// come to what the decoder already recorded, which catches a table
    /// rebuilt out of the wrong lengths. Every entry link points at a step
    /// that declared a length, and declared the width the code turned out to
    /// have, which catches a running counter that drifted. And the link's two
    /// numberings agree, which is what the listing needs to find the row.
    fn every_symbol_adds_up(packed: &[u8], trace: &Trace) -> (usize, usize, usize) {
        let (mut literals, mut matches, mut from_repeat) = (0, 0, 0);
        for block in trace.blocks() {
            if block.kind == BlockKind::Stored {
                continue;
            }
            for k in block.steps.start..block.steps.end {
                let step = trace.step(k as usize).expect("in range");
                if !matches!(step.kind, StepKind::Literal(_) | StepKind::Match { .. } | StepKind::EndOfBlock) {
                    continue;
                }
                let d = decode_step(packed, trace, k as usize)
                    .unwrap_or_else(|| panic!("step {k}, a {:?}, would not come apart", step.kind));
                let width = step.in_bits.end - step.in_bits.start;
                assert_eq!(d.bits() as u64, width, "step {k} is {width} bits and came apart as {d:?}");
                assert_eq!(d.block_kind, block.kind);
                match step.kind {
                    StepKind::Literal(b) => {
                        literals += 1;
                        assert_eq!(d.meaning(), SymbolMeaning::Literal(b));
                        assert_eq!(d.symbol.value, b as u32);
                        assert!(d.distance.is_none(), "a literal read a distance code");
                    }
                    StepKind::Match { len, dist } => {
                        matches += 1;
                        assert_eq!(d.meaning(), SymbolMeaning::Length);
                        assert_eq!(d.symbol.value, len, "step {k} says {len} and came apart as {d:?}");
                        let second = d.distance.expect("a match reads a distance code");
                        assert_eq!(second.value, dist, "step {k} reaches back {dist} and came apart as {d:?}");
                    }
                    StepKind::EndOfBlock => {
                        assert_eq!(d.meaning(), SymbolMeaning::EndOfBlock);
                        assert_eq!(d.symbol.symbol, 256);
                        assert!(d.distance.is_none());
                    }
                    _ => unreachable!(),
                }
                for code in [Some(d.symbol), d.distance].into_iter().flatten() {
                    let Some(entry) = code.entry else {
                        assert_eq!(block.kind, BlockKind::Fixed, "a dynamic block left a code with no entry");
                        assert!(code.entry_child.is_none());
                        continue;
                    };
                    assert_eq!(block.kind, BlockKind::Dynamic, "a fixed block named an entry step");
                    assert!(block.steps.contains(&entry), "step {k} points outside its own block");
                    assert_eq!(code.entry_child, Some(entry - block.steps.start));
                    let declared = match trace.step(entry as usize).expect("in range").kind {
                        StepKind::Table(TableField::LitLen { len, .. } | TableField::Dist { len, .. }) => len,
                        StepKind::Table(TableField::Repeat { len, .. }) => {
                            from_repeat += 1;
                            len
                        }
                        other => panic!("step {k}'s code length came from a {other:?}"),
                    };
                    assert_eq!(declared, code.code_bits, "step {k}'s entry declares {declared} for a {} bit code", code.code_bits);
                }
            }
        }
        (literals, matches, from_repeat)
    }

    /// The four widths of a step add up to the step, over every shape of
    /// stream: dynamic blocks, a fixed one, and a zlib wrapper whose bits are
    /// counted from the front of the run rather than from the first block.
    #[test]
    fn a_step_taken_apart_accounts_for_every_bit_it_read() {
        let text: Vec<u8> = "the quick brown fox jumps over the lazy dog. ".repeat(2000).into_bytes();
        for (data, level) in [(&text[..], 9u8), (&text[..], 6), (b"abcabcab".as_slice(), 6), (&[9u8; 3000][..], 6)] {
            let packed = miniz_oxide::deflate::compress_to_vec(data, level);
            let (_, trace) = inflate(&packed).expect("reads");
            let (literals, matches, _) = every_symbol_adds_up(&packed, &trace);
            assert!(literals > 0 && matches > 0, "nothing was checked for level {level}");
        }
        // The same through a wrapper: the trace counts bits from the front of
        // the run, so a stream starting at bit 16 has to come apart there too.
        let packed = miniz_oxide::deflate::compress_to_vec_zlib(&text, 9);
        let (_, trace) = zlib(&packed).expect("reads");
        assert!(trace.blocks()[0].in_bits.start >= 16);
        every_symbol_adds_up(&packed, &trace);
    }

    /// A length symbol that does not say the whole length: symbol 281 covers
    /// 195 to 226 and reads five bits to say which. The zero-extra case is
    /// most of a stream, so a test that only saw those would pass with the
    /// extra bits never read and the widths still adding up.
    #[test]
    fn a_length_with_extra_bits_says_how_many_and_what_they_said() {
        let packed = miniz_oxide::deflate::compress_to_vec(&[9u8; 3000], 6);
        let (_, trace) = inflate(&packed).expect("reads");
        every_symbol_adds_up(&packed, &trace);
        let mut found = 0;
        for k in 0..trace.len() {
            let Some(d) = decode_step(&packed, &trace, k) else { continue };
            if d.symbol.extra_bits == 0 {
                continue;
            }
            found += 1;
            let base = LEN_BASE[d.symbol.symbol as usize - 257] as u32;
            assert_eq!(d.symbol.value, base + d.symbol.extra);
            assert!(d.symbol.extra < 1 << d.symbol.extra_bits, "{d:?} reads more than its bits hold");
        }
        assert!(found > 0, "a run of three thousand bytes held no length with extra bits");

        // And the distance half of the same question, which needs data with
        // matches reaching further back than four bytes.
        let text: Vec<u8> = "the quick brown fox jumps over the lazy dog. ".repeat(2000).into_bytes();
        let packed = miniz_oxide::deflate::compress_to_vec(&text, 9);
        let (_, trace) = inflate(&packed).expect("reads");
        let mut found = 0;
        for k in 0..trace.len() {
            let Some(second) = decode_step(&packed, &trace, k).and_then(|d| d.distance) else { continue };
            if second.extra_bits == 0 {
                continue;
            }
            found += 1;
            assert_eq!(second.value, DIST_BASE[second.symbol as usize] + second.extra);
        }
        assert!(found > 0, "no match in that text reached back far enough to read extra bits");
    }

    /// A fixed block's tables are in RFC 1951 and not in the file, so there is
    /// no step to send a reader to and the answer says so rather than pointing
    /// at whatever step happens to sit at index zero.
    #[test]
    fn a_fixed_block_has_no_table_entry_to_link_to() {
        let packed = miniz_oxide::deflate::compress_to_vec(b"abcabcab", 6);
        let (_, trace) = inflate(&packed).expect("reads");
        assert_eq!(trace.blocks()[0].kind, BlockKind::Fixed);
        let (literals, matches, _) = every_symbol_adds_up(&packed, &trace);
        assert!(literals > 0 && matches > 0, "the fixed block held no symbols to check");
        for k in 0..trace.len() {
            let Some(d) = decode_step(&packed, &trace, k) else { continue };
            assert_eq!(d.block_kind, BlockKind::Fixed);
            assert!(d.symbol.entry.is_none(), "{d:?} claims the file declared a fixed code");
            assert!(d.distance.is_none_or(|second| second.entry.is_none()));
            // The fixed literal code is eight bits up to 143 and nine after,
            // and the end mark is seven, which is what RFC 1951 says.
            let want = match d.symbol.symbol {
                0..=143 => 8,
                144..=255 => 9,
                256..=279 => 7,
                _ => 8,
            };
            assert_eq!(d.symbol.code_bits, want, "{d:?} is not the fixed code");
            assert!(d.distance.is_none_or(|second| second.code_bits == 5));
        }
        // The hand-built empty fixed block from the test above, which holds
        // nothing but the end mark.
        let (_, trace) = inflate(&[0x03, 0x00]).expect("reads");
        let end = (0..trace.len()).find(|&k| trace.step(k).unwrap().kind == StepKind::EndOfBlock).expect("an end mark");
        let d = decode_step(&[0x03, 0x00], &trace, end).expect("comes apart");
        assert_eq!(d.meaning(), SymbolMeaning::EndOfBlock);
        assert_eq!(d.bits(), 7);
    }

    /// A code length that came from a repeat entry rather than from one
    /// written out. Code 16 says "the length before this one, again", so the
    /// only thing that says which symbols it covered is a counter run over the
    /// head in the order the decoder read it, and a counter off by one puts
    /// every later symbol in the wrong place.
    #[test]
    fn a_code_length_from_a_repeat_names_the_step_that_repeated_it() {
        let text: Vec<u8> = "the quick brown fox jumps over the lazy dog. ".repeat(2000).into_bytes();
        let packed = miniz_oxide::deflate::compress_to_vec(&text, 9);
        let (_, trace) = inflate(&packed).expect("reads");
        assert_eq!(trace.blocks()[0].kind, BlockKind::Dynamic);
        // The head has repeats that give a length something uses: code 16
        // repeats the length before it, where 17 and 18 write runs of zero and
        // no symbol with a zero length is ever decoded.
        assert!(trace.steps().any(|s| matches!(s.kind, StepKind::Table(TableField::Repeat { len, .. }) if len > 0)));
        let (_, _, from_repeat) = every_symbol_adds_up(&packed, &trace);
        assert!(from_repeat > 0, "no symbol in that stream took its code length from a repeat");
    }

    /// Everything that is not a symbol of a Huffman-coded block, and every way
    /// a run can be broken. Nothing here answers, and nothing here panics: a
    /// panel asking about the step under the cursor must never take the
    /// listing down with it.
    #[test]
    fn a_step_that_is_not_a_symbol_is_refused_rather_than_guessed_at() {
        let text: Vec<u8> = "and now for something not a symbol at all. ".repeat(200).into_bytes();
        let packed = miniz_oxide::deflate::compress_to_vec(&text, 9);
        let (_, trace) = inflate(&packed).expect("reads");
        for k in 0..trace.len() {
            let kind = trace.step(k).expect("in range").kind;
            let got = decode_step(&packed, &trace, k);
            let symbol = matches!(kind, StepKind::Literal(_) | StepKind::Match { .. } | StepKind::EndOfBlock);
            assert_eq!(got.is_some(), symbol, "step {k}, a {kind:?}, came apart as {got:?}");
        }
        // Past the end, and on a run that is not the run the trace describes.
        assert!(decode_step(&packed, &trace, trace.len()).is_none());
        assert!(decode_step(&packed, &trace, usize::MAX).is_none());
        for n in 0..packed.len() {
            for k in 0..trace.len() {
                let _ = decode_step(&packed[..n], &trace, k);
            }
        }
        let noise: Vec<u8> = packed.iter().map(|b| !b).collect();
        for k in 0..trace.len() {
            let _ = decode_step(&noise, &trace, k);
        }
        // A stored block's payload is bytes, not a symbol, and a coarsened
        // block names its symbols with one opaque step that is not one either.
        let mut stored = vec![0x01, 0x05, 0x00, 0xfa, 0xff];
        stored.extend_from_slice(b"there");
        let (_, trace) = inflate(&stored).expect("reads");
        for k in 0..trace.len() {
            assert!(decode_step(&stored, &trace, k).is_none());
        }
        let (_, coarse) = inflate_within(&packed, 50).expect("reads");
        assert!(coarse.coarse());
        for k in 0..coarse.len() {
            assert!(decode_step(&packed, &coarse, k).is_none(), "a coarse trace named a symbol at step {k}");
        }
        // And every one-byte corruption of a real stream, which is where a
        // rebuilt table meets lengths nobody meant to write.
        let packed = miniz_oxide::deflate::compress_to_vec(&"corrupt me byte by byte. ".repeat(40).into_bytes(), 6);
        for i in 0..packed.len() {
            for xor in [0x01u8, 0x40, 0xff] {
                let mut bad = packed.clone();
                bad[i] ^= xor;
                let Ok((_, trace)) = inflate(&bad) else { continue };
                for k in 0..trace.len() {
                    let _ = decode_step(&bad, &trace, k);
                    let _ = decode_step(&packed, &trace, k);
                }
            }
        }
    }

    /// A gzip member built by hand: the fixed ten bytes, whatever the flags
    /// say follows them, the deflate stream, and the sum and the length.
    fn member(flags: u8, front: &[u8], data: &[u8]) -> Vec<u8> {
        let mut m = vec![0x1f, 0x8b, 8, flags, 0, 0, 0, 0, 0, 3];
        m.extend_from_slice(front);
        m.extend_from_slice(&miniz_oxide::deflate::compress_to_vec(data, 6));
        m.extend(crate::checksum::crc32(data).to_le_bytes());
        m.extend((data.len() as u32).to_le_bytes());
        m
    }

    /// The plain case, and the trace tiling the whole member rather than only
    /// the deflate in the middle of it.
    #[test]
    fn a_gzip_member_reads_as_what_went_in() {
        let text = b"a gzip member is a header, a deflate stream, and a sum.".repeat(40);
        let packed = member(0, &[], &text);
        let (out, trace) = gzip(&packed).expect("reads");
        assert_eq!(out, text);
        trace.check_tiles().expect("the trace tiles");
        assert_eq!(trace.in_bits(), packed.len() as u64 * 8);
        assert_eq!(trace.out_bytes(), text.len() as u64);
        // The header and the trailer are steps of their own, so nothing in the
        // run belongs to nobody.
        let wrappers: Vec<_> =
            trace.steps().filter(|s| matches!(s.kind, StepKind::Header(StepField::Wrapper, _))).collect();
        assert_eq!(wrappers.len(), 2);
        assert_eq!(wrappers[0].in_bits, 0..10 * 8);
        assert_eq!(wrappers[1].in_bits.end, packed.len() as u64 * 8);
    }

    /// Everything the flags can put between the header and the stream, which
    /// is what makes a gzip header a thing to parse rather than to skip.
    #[test]
    fn the_optional_parts_of_the_header_move_the_stream_along() {
        let text = b"named and commented".to_vec();
        // FEXTRA, FNAME, FCOMMENT and FHCRC together.
        let mut front = vec![4u8, 0, b'e', b'x', b't', b'r']; // two of length, four of it
        front.extend_from_slice(b"probe.res\0");
        front.extend_from_slice(b"written by hand\0");
        front.extend_from_slice(&[0, 0]); // the header sum, which nothing checks
        let packed = member(0x02 | 0x04 | 0x08 | 0x10, &front, &text);
        let (out, trace) = gzip(&packed).expect("reads");
        assert_eq!(out, text);
        trace.check_tiles().expect("tiles");
        // The first step covers all of it, however long the flags made it.
        assert_eq!(trace.step(0).unwrap().in_bits, 0..(10 + front.len() as u64) * 8);
        // And each flag on its own, since a header read four bytes short reads
        // the stream at the wrong bit and fails in a way that says nothing.
        for (flag, front) in
            [(0x08u8, &b"only-a-name\0"[..]), (0x10, b"only a comment\0"), (0x04, &[2, 0, b'h', b'i'])]
        {
            assert_eq!(gzip(&member(flag, front, &text)).expect("reads").0, text);
        }
    }

    #[test]
    fn a_member_that_does_not_add_up_is_refused_rather_than_returned() {
        let text = b"check me".to_vec();
        let good = member(0, &[], &text);
        assert!(gzip(&good).is_ok());
        // A sum that is not the sum of what came out.
        let mut bad = good.clone();
        bad[good.len() - 8] ^= 1;
        assert_eq!(gzip(&bad).err(), Some(Refusal::Failed));
        // A length that is not the length.
        let mut bad = good.clone();
        bad[good.len() - 4] ^= 1;
        assert_eq!(gzip(&bad).err(), Some(Refusal::Failed));
        // Not a member at all, and a zlib stream read as one.
        assert_eq!(gzip(b"not a gzip member at all").err(), Some(Refusal::Failed));
        assert_eq!(gzip(&miniz_oxide::deflate::compress_to_vec_zlib(&text, 6)).err(), Some(Refusal::Failed));
        // A reserved flag bit, which the format says to refuse.
        let mut bad = good.clone();
        bad[3] = 0x20;
        assert_eq!(gzip(&bad).err(), Some(Refusal::Failed));
        // A name with no terminator, and every prefix of a whole member.
        assert!(gzip(&member(0x08, b"no terminator here", &text)).is_err());
        for n in 0..good.len() {
            let _ = gzip(&good[..n]);
        }
        // Two members end to end, which a `.gz` file may be and this codec is
        // not: what it decodes is the first and what it checks is the last
        // one's trailer, so it refuses rather than handing back half a file.
        let second = member(0, &[], b"and me as well");
        assert_eq!(gzip(&[good, second].concat()).err(), Some(Refusal::Failed));
    }
}
