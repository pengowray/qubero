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
    if adler != adler32(&out) {
        return Err(Refusal::Failed);
    }
    b.push(end, out.len() as u64, StepKind::Header(StepField::Wrapper, 0));
    b.finish_at(data.len() as u64 * 8, out.len() as u64);
    Ok((out, b.done()))
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

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &byte in chunk {
            a += byte as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
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
}
