//! RAR 5's compression: LZSS against a window, under five Huffman tables, with
//! three byte-transforming filters layered on top of the result.
//!
//! Written against libarchive's `archive_read_support_format_rar5.c`, which is
//! BSD 2-clause, copyright 2018 Grzegorz Antoniak, and is a self-contained
//! implementation owing nothing to unRAR:
//! <https://github.com/libarchive/libarchive/blob/master/libarchive/archive_read_support_format_rar5.c>
//!
//! That provenance is the whole reason this file exists at all. unRAR's licence
//! forbids using its sources to re-create the RAR compression algorithm, and
//! 7-Zip carves its RAR handler out of its own LGPL for that reason, so neither
//! could be read here without putting that restriction on this project.
//! libarchive's is a clean-room reimplementation under a licence that asks only
//! for the notice above. See `THIRD-PARTY-NOTICES.md`.
//!
//! ## The stream
//!
//! An entry's data area is a chain of blocks, each of which is a short header
//! and then a bit stream. The header is a flags byte, a checksum byte, and one
//! to three bytes of length; the flags say how many length bytes there are, how
//! many bits of the block's *last* byte mean anything, whether the block
//! carries new Huffman tables, and whether it is the last block of the entry.
//!
//! Nothing in the bit stream says where it ends. A block ends when the bit
//! cursor reaches the last byte's meaningful bits, which is why `bit_size` is
//! in the header, and the entry ends at the block whose flags say so. Neither
//! is a marker in the stream: both are counted.
//!
//! ## Five tables, one written in the next
//!
//! A block that carries tables writes 430 code lengths, and the alphabet those
//! lengths are themselves written in is written first, in a way that is not
//! Huffman at all:
//!
//! 1. The **code-length alphabet**, 20 symbols, one nibble each, read from the
//!    front of the block before any bit reading starts. A nibble of 15 is an
//!    escape: the nibble after it is 0 for a literal 15, and anything else for
//!    that many plus two zeroes. This is the only part of the block read a
//!    nibble at a time, and the bit cursor takes over from wherever it stopped,
//!    which may be mid-byte.
//! 2. The **430 lengths**, under the alphabet above, run-length coded: 0 to 15
//!    is a length outright, 16 and 17 repeat the length before, and 18 and 19
//!    write runs of zeroes. The four tables are cut out of that one run in
//!    order: 306 literal/length codes, 64 distance slots, 16 low-distance bits,
//!    44 repeat-match lengths.
//!
//! A block whose flags say it carries no tables keeps the ones before it, which
//! is what lets an encoder split a file across blocks without paying for the
//! tables twice.
//!
//! ## Symbols
//!
//! A symbol under 256 is the byte it is. 256 defines a filter, which reads bits
//! and writes nothing. 257 repeats the last match at the last distance. 258 to
//! 261 repeat a match at one of the four most recent distances, moved to the
//! front as it is used. 262 and up is a match: the symbol names a length, a
//! second table names a distance slot, and how many more bits follow the slot
//! depends on the slot. A distance over 0x100 buys a byte of length, over
//! 0x2000 another, over 0x40000 another again, which is the format paying for
//! the fact that a long reach is only worth coding for a long match.
//!
//! ## What the window is worth
//!
//! Less than its name suggests, the same as it is in [`lha`](super::lha). RAR
//! writes into a circular buffer of the declared dictionary size and masks
//! every index by it; nothing here allocates one, because the output *is* the
//! history and a match reads back into it. The window reaches the arithmetic
//! only as a bound: a distance may not reach past the start of what has been
//! written, nor past the dictionary the header declared, and a filter may not
//! cover more than half of it. A file whose distances overrun any of those is
//! not one an encoder wrote, and saying so is better than masking the index
//! round into bytes that mean nothing.
//!
//! That equivalence is exactly what makes this decoder a *non-solid* one, and
//! it is the reason the template hands it only non-solid entries. A solid
//! entry's first match may reach back into the entry before it, where the
//! output of this run holds nothing at all. See the module doc of
//! [`formats::rar5`](crate::formats::rar5) for the rest of what is turned away.
//!
//! ## Filters
//!
//! Symbol 256 does not produce a byte. It names a range of the output and one
//! of three transforms to run over it once the range has been unpacked: Delta
//! for interleaved channels, E8/E8E9 for x86 call and jump targets, ARM for BL
//! branch targets. Every one of them rewrites bytes in place and none changes a
//! length.
//!
//! Filters read the *unfiltered* output and write to a copy, which is what
//! RAR's own unpacker does: the window a later match copies from holds what
//! LZSS produced, not what a filter made of it. So this keeps the two apart,
//! decodes the whole entry into `raw`, and lays the filtered ranges over a copy
//! of it at the end. Filters may not overlap and must be given in order, which
//! is checked rather than assumed.
//!
//! ## Where the trace is exact and where it is not
//!
//! Per symbol, and the map from input bits to output *positions* is exact
//! everywhere: every literal, every match, every code length and the filter
//! definitions all tile the run. Inside a filtered range the byte *values* a
//! reader sees are the filter's, while the step that covers them names the
//! literal or the match that put the pre-filter byte there. That is a real
//! limit and not a rounding: the honest reading of such a step is "this is
//! where these bytes came from before the filter ran", and the filter's own
//! definition is a step of its own a few thousand symbols earlier. Nothing else
//! in the trace is approximate.
//!
//! ## Bits
//!
//! Most significant first, which is how Qubero addresses them, so a step's bit
//! range needs no reinterpreting the way [`inflate`](super::inflate)'s does.
//! The nibbles of the first table are the high half of a byte and then the low
//! half, which is the same order.

use crate::codec::{BlockKind, Refusal, StepField, StepKind, TableField, Trace, TraceBuilder, CAP_BYTES};

/// The code-length alphabet, written a nibble at a time in front of everything.
const HUFF_BC: usize = 20;
/// Literals and match lengths: 0 to 255 the bytes, 256 a filter, 257 to 261 the
/// repeats, 262 and up the lengths.
const HUFF_NC: usize = 306;
/// Distance slots.
const HUFF_DC: usize = 64;
/// The low four bits of a long distance, coded rather than written flat.
const HUFF_LDC: usize = 16;
/// Lengths for a match that reuses a remembered distance.
const HUFF_RC: usize = 44;
/// The lengths all four are cut out of, in one run.
const HUFF_TABLE_SIZE: usize = HUFF_NC + HUFF_DC + HUFF_LDC + HUFF_RC;

/// The transforms symbol 256 may ask for. The other four numbers the field can
/// hold were never used by RAR 5 and are refused rather than guessed at.
const FILTER_DELTA: u16 = 0;
const FILTER_E8: u16 = 1;
const FILTER_E8E9: u16 = 2;
const FILTER_ARM: u16 = 3;

/// The x86 filter's idea of how large the program it is rewriting is. Not a
/// measurement of anything: it is a constant both ends agree on, and an address
/// is rewritten only when it lands inside it.
const E8_FILE_SIZE: u32 = 0x0100_0000;

/// Reading a block's bits, most significant first.
///
/// `in_addr` and `bit_addr` are libarchive's, kept rather than folded into one
/// count because the table reader hands the cursor over as a byte and a nibble
/// and the arithmetic below is easier to check against the original this way.
struct Bits<'a> {
    /// The block's bytes, and nothing past them.
    data: &'a [u8],
    /// Where `data[0]` sits in the whole run, in bits, so a step can be
    /// recorded against the run rather than against the block.
    base: u64,
    in_addr: usize,
    bit_addr: u32,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8], base: u64) -> Bits<'a> {
        Bits { data, base, in_addr: 0, bit_addr: 0 }
    }

    /// Where the cursor is in the whole run.
    fn at(&self) -> u64 {
        self.base + self.in_addr as u64 * 8 + self.bit_addr as u64
    }

    /// A byte of the block, or zero past the end of it.
    ///
    /// The peeks below read up to five bytes to answer about the next sixteen
    /// or thirty-two bits, so near the end of a block they reach past it.
    /// libarchive reads the bytes that follow in its buffer; zeroes are the
    /// honest answer here, and no code an encoder wrote needs those bits: the
    /// block ends at a counted position and everything past it is padding.
    fn byte(&self, i: usize) -> u32 {
        self.data.get(i).copied().unwrap_or(0) as u32
    }

    /// That the cursor is still inside the block. libarchive's two read
    /// functions refuse past this, and so does this.
    fn inside(&self) -> Result<(), Refusal> {
        match self.in_addr < self.data.len() {
            true => Ok(()),
            false => Err(Refusal::Failed),
        }
    }

    /// The next sixteen bits, without consuming them.
    fn peek16(&self) -> u32 {
        let v = self.byte(self.in_addr) << 16 | self.byte(self.in_addr + 1) << 8 | self.byte(self.in_addr + 2);
        v >> (8 - self.bit_addr) & 0xffff
    }

    /// The next thirty-two bits, without consuming them. Only the long distance
    /// codes need this many.
    fn peek32(&self) -> u32 {
        let v = self.byte(self.in_addr) << 24
            | self.byte(self.in_addr + 1) << 16
            | self.byte(self.in_addr + 2) << 8
            | self.byte(self.in_addr + 3);
        // A shift of eight on a byte is zero, which is what C's promotion to
        // int gives here and what the `bit_addr == 0` case needs.
        (v << self.bit_addr) | (self.byte(self.in_addr + 4) >> (8 - self.bit_addr))
    }

    fn skip(&mut self, bits: u32) {
        let n = self.bit_addr + bits;
        self.in_addr += (n >> 3) as usize;
        self.bit_addr = n & 7;
    }

    /// `n` bits as a number, the first one read being the highest. At most
    /// sixteen, which is all any field in this format is.
    fn val(&mut self, n: u32) -> Result<u32, Refusal> {
        self.inside()?;
        let v = self.peek16() >> (16 - n);
        self.skip(n);
        Ok(v)
    }
}

/// One of the five tables, in the form libarchive decodes from.
///
/// Not the canonical walk [`lha`](super::lha) uses. RAR's tables are canonical
/// too, but the decoder reaches a symbol by comparing sixteen bits against a
/// per-length upper bound, and a short lookup answers the common lengths in one
/// step. Ported as it stands rather than rewritten: the arithmetic is what
/// decides which symbol a bit pattern is, and a rewrite that agreed on every
/// well-formed file could still disagree on a damaged one.
struct Table {
    size: usize,
    /// The largest sixteen-bit prefix that can still be a code of each length.
    decode_len: [i64; 16],
    /// Where each length's symbols start in `decode_num`.
    decode_pos: [u32; 16],
    quick_bits: u32,
    quick_len: Vec<u8>,
    quick_num: Vec<u16>,
    /// The symbols, by code length and then by symbol.
    decode_num: Vec<u16>,
}

impl Table {
    /// The table `bit_length` describes.
    ///
    /// Over-subscribed is refused, which is the one check that stops a stream
    /// of noise decoding into a plausible-looking file: a set of lengths whose
    /// codes need more than a sixteen-bit space is not a prefix code and
    /// nothing an encoder wrote looks like it. Under-subscribed is allowed, the
    /// same as everywhere: an alphabet with one symbol in use is written that
    /// way.
    fn new(bit_length: &[u8], size: usize) -> Result<Table, Refusal> {
        let mut lc = [0i64; 16];
        for i in 0..size {
            lc[(bit_length[i] & 15) as usize] += 1;
        }
        lc[0] = 0;

        let mut decode_len = [0i64; 16];
        let mut decode_pos = [0u32; 16];
        let mut upper_limit = 0i64;
        for i in 1..16 {
            upper_limit += lc[i];
            decode_len[i] = upper_limit << (16 - i);
            decode_pos[i] = decode_pos[i - 1] + lc[i - 1] as u32;
            upper_limit <<= 1;
        }
        if upper_limit > 65536 {
            return Err(Refusal::Failed);
        }

        let mut decode_num = vec![0u16; size.max(1)];
        let mut fill = decode_pos;
        for i in 0..size {
            let clen = (bit_length[i] & 15) as usize;
            if clen > 0 {
                let at = fill[clen] as usize;
                if at < size {
                    decode_num[at] = i as u16;
                }
                fill[clen] += 1;
            }
        }

        // The short cut: for every prefix this many bits wide, which length its
        // code is and which symbol, worked out once so the common case is one
        // lookup rather than a walk down the lengths.
        let quick_bits = if size == HUFF_NC { 10 } else { 7 };
        let quick_size = 1usize << quick_bits;
        let mut quick_len = vec![0u8; quick_size];
        let mut quick_num = vec![0u16; quick_size];
        let mut cur_len = 1usize;
        for code in 0..quick_size {
            let bit_field = (code << (16 - quick_bits)) as i64;
            while cur_len < 16 && bit_field >= decode_len[cur_len] {
                cur_len += 1;
            }
            quick_len[code] = cur_len as u8;
            let dist = (bit_field - decode_len[cur_len - 1]) >> (16 - cur_len);
            let pos = decode_pos[cur_len & 15] as i64 + dist;
            if cur_len < 16 && pos >= 0 && (pos as usize) < size {
                quick_num[code] = decode_num[pos as usize];
            }
        }

        Ok(Table { size, decode_len, decode_pos, quick_bits, quick_len, quick_num, decode_num })
    }

    /// The symbol the next bits name.
    fn decode(&self, bits: &mut Bits) -> Result<u16, Refusal> {
        bits.inside()?;
        // The low bit is dropped before the comparison: a code is at most
        // fifteen bits, so the sixteenth never decides which one it is.
        let bitfield = (bits.peek16() & 0xfffe) as i64;
        if bitfield < self.decode_len[self.quick_bits as usize] {
            let code = (bitfield >> (16 - self.quick_bits)) as usize;
            bits.skip(self.quick_len[code] as u32);
            return Ok(self.quick_num[code]);
        }
        let mut len = 15usize;
        for i in self.quick_bits as usize + 1..15 {
            if bitfield < self.decode_len[i] {
                len = i;
                break;
            }
        }
        bits.skip(len as u32);
        let dist = (bitfield - self.decode_len[len - 1]) >> (16 - len);
        let pos = self.decode_pos[len] as i64 + dist;
        let pos = match pos >= 0 && (pos as usize) < self.size {
            true => pos as usize,
            // A prefix no symbol claims. libarchive answers with symbol zero
            // rather than failing, and the bits are consumed either way, so the
            // stream stays in step and whatever it decodes to fails its check.
            false => 0,
        };
        Ok(self.decode_num[pos])
    }
}

/// The five tables a block decodes under, kept across blocks that carry none.
struct Tables {
    /// Literals, the filter code, the repeats, and the match lengths.
    lit: Table,
    /// Distance slots.
    dist: Table,
    /// The low four bits of a distance far enough back to code them.
    low_dist: Table,
    /// Lengths for a match reusing a remembered distance.
    repeat: Table,
}

/// A transform symbol 256 asked for, and the run of output it covers.
#[derive(Clone, Copy)]
struct Filter {
    kind: u16,
    /// Where the run starts in the output, counted from the start of the entry.
    start: u64,
    len: u32,
    /// Delta only: how many interleaved streams the bytes are.
    channels: usize,
}

/// One entry's packed data.
///
/// `window_bits` is the dictionary the header declared, as a power of two, and
/// `unpacked` is what the header says comes out. Neither is in the stream: RAR
/// writes no end-of-stream marker and no dictionary size, so a decoder that was
/// not told is a decoder that cannot tell a finished file from a truncated one.
/// See [`Codec::Rar5`](crate::codec::Codec::Rar5).
pub fn entry(data: &[u8], window_bits: u8, unpacked: u64) -> Result<(Vec<u8>, Trace), Refusal> {
    run(data, window_bits, unpacked, TraceBuilder::default())
}

fn run(data: &[u8], window_bits: u8, unpacked: u64, mut b: TraceBuilder) -> Result<(Vec<u8>, Trace), Refusal> {
    if unpacked > CAP_BYTES as u64 {
        return Err(Refusal::TooLarge);
    }
    if !(17..=32).contains(&window_bits) {
        // 128K is the smallest dictionary RAR 5 has a code for and 4G the
        // largest. Anything else is a number this cannot have come from the
        // four bits the header spends on it.
        return Err(Refusal::Settings);
    }
    let unpacked = unpacked as usize;
    let window = 1u64 << window_bits;
    // A file of no bytes is packed into no blocks. Nothing to read, and nothing
    // wrong with that.
    if data.is_empty() {
        let mut b = b;
        b.finish_at(0, 0);
        return match unpacked == 0 {
            true => Ok((Vec::new(), b.done())),
            false => Err(Refusal::Failed),
        };
    }

    // What LZSS produced, which is what a match copies from and what a filter
    // reads. The filtered bytes are laid over a copy of it at the end.
    let mut raw: Vec<u8> = Vec::with_capacity(unpacked);
    let mut filters: Vec<Filter> = Vec::new();
    // The four most recent distances, most recent first.
    let mut dist_cache = [0u64; 4];
    let mut last_len = 0u32;
    let mut tables: Option<Tables> = None;
    let mut coarse = false;
    // Where the next block header is, in bytes from the start of the run.
    let mut pos = 0usize;

    loop {
        // A block header is two bytes and then one to three of length. Running
        // out here is an entry whose blocks did not add up.
        if pos + 3 > data.len() {
            return Err(Refusal::Failed);
        }
        let flags = data[pos];
        let byte_count = (flags >> 3 & 7) as usize;
        if byte_count > 2 {
            return Err(Refusal::Failed);
        }
        let bit_size = 1 + (flags & 7) as u32;
        let last_block = flags >> 6 & 1 == 1;
        let table_present = flags >> 7 & 1 == 1;
        let header_len = 3 + byte_count;
        if pos + header_len > data.len() {
            return Err(Refusal::Failed);
        }
        let mut block_size = 0usize;
        for i in 0..=byte_count {
            block_size |= (data[pos + 2 + i] as usize) << (i * 8);
        }
        // The header's own seal: a byte, over the flags and the length.
        let sum = 0x5a
            ^ flags
            ^ (block_size & 0xff) as u8
            ^ (block_size >> 8 & 0xff) as u8
            ^ (block_size >> 16 & 0xff) as u8;
        if sum != data[pos + 1] {
            return Err(Refusal::Failed);
        }
        let body = pos + header_len;
        // A block longer than what is left is an entry split across volumes, or
        // one cut short. Either way the bytes to finish it are not here, and
        // half a file handed back quietly is the wrong answer to both.
        if block_size == 0 || body + block_size > data.len() {
            return Err(Refusal::Failed);
        }
        let block_end = (body + block_size) as u64 * 8;

        b.open_block(pos as u64 * 8, raw.len() as u64);
        b.push(pos as u64 * 8, raw.len() as u64, StepKind::Header(StepField::BlockHeader, block_size as u32));

        let mut bits = Bits::new(&data[body..body + block_size], body as u64 * 8);
        if table_present {
            tables = Some(parse_tables(&mut bits, &mut b, raw.len() as u64)?);
        }
        // A block that carries no tables and follows none is a stream starting
        // in the middle of itself, which is what a solid entry looks like from
        // here. The template does not send those, and this says so if one
        // arrives.
        let Some(t) = tables.as_ref() else { return Err(Refusal::Failed) };

        let sym_start = b.steps();
        let sym_in = bits.at();
        let sym_out = raw.len() as u64;
        if coarse {
            b.push(sym_in, sym_out, StepKind::Opaque);
        }

        loop {
            // The block ends at a counted position: past the last byte, or at
            // it with the meaningful bits used up.
            if bits.in_addr + 1 > block_size || (bits.in_addr + 1 == block_size && bits.bit_addr >= bit_size) {
                break;
            }
            // Too many symbols to name one at a time: keep the map at the
            // block, and say in the trace that this is what happened.
            if !coarse && b.over_budget() {
                coarse = true;
                b.coarsen();
                b.truncate(sym_start);
                b.push(sym_in, sym_out, StepKind::Opaque);
            }
            let at = bits.at();
            let sym = t.lit.decode(&mut bits)?;

            if sym < 256 {
                if raw.len() >= unpacked {
                    return Err(Refusal::Failed);
                }
                raw.push(sym as u8);
                if !coarse {
                    b.push(at, raw.len() as u64 - 1, StepKind::Literal(sym as u8));
                }
                continue;
            }
            if sym == 256 {
                let f = parse_filter(&mut bits, raw.len() as u64, window, filters.last())?;
                filters.push(f);
                if !coarse {
                    b.push(at, raw.len() as u64, StepKind::Header(StepField::FilterDef, f.kind as u32));
                }
                continue;
            }
            // A match, in one of three shapes: the last one again, one of the
            // four remembered distances with a fresh length, or both read out
            // of the stream.
            let (len, dist) = if sym == 257 {
                if last_len == 0 {
                    // Nothing to repeat yet. libarchive skips the symbol
                    // rather than failing, and so does this; the bits it took
                    // are still recorded, or the step before would appear to
                    // have read them.
                    if !coarse {
                        b.push(at, raw.len() as u64, StepKind::Opaque);
                    }
                    continue;
                }
                (last_len, dist_cache[0])
            } else if sym < 262 {
                let idx = sym as usize - 258;
                let dist = dist_cache[idx];
                dist_cache.copy_within(0..idx, 1);
                dist_cache[0] = dist;
                let slot = t.repeat.decode(&mut bits)?;
                (code_length(&mut bits, slot)?, dist)
            } else {
                let mut len = code_length(&mut bits, sym - 262)?;
                let dist = read_distance(&mut bits, &t.dist, &t.low_dist)?;
                // A long reach earns a longer match: the format spends the
                // extra distance bits only where a match is worth them.
                if dist > 0x100 {
                    len += 1;
                    if dist > 0x2000 {
                        len += 1;
                        if dist > 0x40000 {
                            len += 1;
                        }
                    }
                }
                dist_cache.copy_within(0..3, 1);
                dist_cache[0] = dist;
                (len, dist)
            };

            last_len = len;
            if dist == 0 || dist > raw.len() as u64 || dist > window {
                // Past the start of what has been written, or past the
                // dictionary the header declared. See the module doc: this is
                // where a solid entry, and a damaged one, both give themselves
                // away.
                return Err(Refusal::Failed);
            }
            if raw.len() + len as usize > unpacked {
                return Err(Refusal::Failed);
            }
            if !coarse {
                b.push(at, raw.len() as u64, StepKind::Match { len, dist: dist as u32 });
            }
            let from = raw.len() - dist as usize;
            // An overlapping copy is ordinary: a distance of one fills.
            for k in 0..len as usize {
                let byte = raw[from + k];
                raw.push(byte);
            }
        }

        // Whatever is left of the block after its last symbol. Reading past the
        // end of a block is a stream that does not fit the length it declared.
        if bits.at() > block_end {
            return Err(Refusal::Failed);
        }
        if bits.at() < block_end {
            b.push(bits.at(), raw.len() as u64, StepKind::Header(StepField::Padding, 0));
        }
        b.close_block(block_end, raw.len() as u64, BlockKind::Dynamic, last_block);

        pos = body + block_size;
        if last_block {
            break;
        }
    }

    // What the header said comes out. A decoder that produced anything else has
    // not decoded this entry, and handing the bytes over anyway would put a
    // checksum mismatch in front of a reader as if the file were the broken
    // thing.
    if raw.len() != unpacked {
        return Err(Refusal::Failed);
    }
    // Any bytes after the last block, which no block claimed.
    let end = data.len() as u64 * 8;
    if (pos as u64) < data.len() as u64 {
        b.push(pos as u64 * 8, raw.len() as u64, StepKind::Header(StepField::Padding, 0));
    }
    b.finish_at(end, raw.len() as u64);

    // Nothing to lay over it is the ordinary case, and the bytes are already
    // the answer: copying a hundred megabytes to say so would be the most
    // expensive thing this file does.
    let out = match filters.is_empty() {
        true => raw,
        false => apply_filters(&raw, &filters)?,
    };
    Ok((out, b.done()))
}

/// A match length, from the symbol that named it and the extra bits it asks
/// for. Shared by the two tables that code one: the literal alphabet above 262,
/// and the repeat alphabet.
fn code_length(bits: &mut Bits, code: u16) -> Result<u32, Refusal> {
    if code >= HUFF_RC as u16 {
        return Err(Refusal::Failed);
    }
    let code = code as u32;
    let (extra, mut len) = match code < 8 {
        true => (0, 2 + code),
        false => (code / 4 - 1, 2 + ((4 | code & 3) << (code / 4 - 1))),
    };
    if extra > 0 {
        len += bits.val(extra)?;
    }
    Ok(len)
}

/// How far back a match reads: a slot from one table, and then the bits the
/// slot says follow it. The four lowest bits of a long distance are themselves
/// coded, under a table of their own, which is what `low_dist` is.
fn read_distance(bits: &mut Bits, dist: &Table, low_dist: &Table) -> Result<u64, Refusal> {
    let slot = dist.decode(bits)? as u32;
    if slot >= HUFF_DC as u32 {
        return Err(Refusal::Failed);
    }
    if slot < 4 {
        return Ok(1 + slot as u64);
    }
    let extra = slot / 2 - 1;
    let mut d = 1u64 + (((2 | (slot & 1)) as u64) << extra);
    if extra < 4 {
        d += bits.val(extra)? as u64;
        return Ok(d);
    }
    if extra > 4 {
        // Everything above the low four bits, read flat.
        bits.inside()?;
        let add = bits.peek32();
        bits.skip(extra - 4);
        d += ((add >> (36 - extra)) << 4) as u64;
    }
    let low = low_dist.decode(bits)?;
    if low as usize >= HUFF_LDC {
        return Err(Refusal::Failed);
    }
    Ok(d + low as u64)
}

/// The five tables at the front of a block.
fn parse_tables(bits: &mut Bits, b: &mut TraceBuilder, out: u64) -> Result<Tables, Refusal> {
    // The code-length alphabet, a nibble at a time from the front of the block.
    // The bit cursor has not started yet and takes over from where this stops.
    let mut bit_length = [0u8; HUFF_BC];
    let mut nib = 0usize;
    // A nibble of the block, high half of a byte first. Running out is a block
    // that stops in the middle of its own tables.
    let nibble = |data: &[u8], nib: &mut usize| -> Result<u32, Refusal> {
        let byte = *data.get(*nib / 2).ok_or(Refusal::Failed)? as u32;
        let v = match *nib % 2 == 0 {
            true => byte >> 4,
            false => byte & 0xf,
        };
        *nib += 1;
        Ok(v)
    };
    let mut w = 0usize;
    while w < HUFF_BC {
        let at = bits.base + nib as u64 * 4;
        let value = nibble(bits.data, &mut nib)?;
        if value != 15 {
            bit_length[w] = value as u8;
            b.push(at, out, StepKind::Table(TableField::CodeLen { sym: w as u8, len: value as u8 }));
            w += 1;
            continue;
        }
        // An escape: a zero after it means the length really is fifteen, and
        // anything else means that many plus two lengths of zero.
        let value = nibble(bits.data, &mut nib)?;
        if value == 0 {
            bit_length[w] = 15;
            b.push(at, out, StepKind::Table(TableField::CodeLen { sym: w as u8, len: 15 }));
            w += 1;
            continue;
        }
        let run = ((value + 2) as usize).min(HUFF_BC - w);
        for k in 0..run {
            bit_length[w + k] = 0;
        }
        b.push(at, out, StepKind::Table(TableField::Repeat { code: 15, count: run as u16, len: 0, dist: false }));
        w += run;
    }
    bits.in_addr = nib / 2;
    bits.bit_addr = if nib % 2 == 0 { 0 } else { 4 };

    let code_lengths = Table::new(&bit_length, HUFF_BC)?;

    // The 430 lengths the four real tables are cut out of, under the alphabet
    // just read.
    let mut table = [0u8; HUFF_TABLE_SIZE];
    let mut i = 0usize;
    while i < HUFF_TABLE_SIZE {
        let at = bits.at();
        let num = code_lengths.decode(bits)?;
        // Which of the four tables this length belongs to, which is all the
        // trace needs to call it by the right name.
        let dist = i >= HUFF_NC;
        if num < 16 {
            table[i] = num as u8;
            b.push(at, out, step_for(dist, i, num as u8));
            i += 1;
            continue;
        }
        if num >= 20 {
            return Err(Refusal::Failed);
        }
        // 16 and 17 repeat the length before; 18 and 19 write zeroes. The pair
        // in each case differ only in how wide the count is.
        let short = num == 16 || num == 18;
        let n = match short {
            true => bits.val(3)? + 3,
            false => bits.val(7)? + 11,
        } as usize;
        let repeat = num < 18;
        if repeat && i == 0 {
            // Nothing to repeat: the first length cannot be "the one before".
            return Err(Refusal::Failed);
        }
        let took = n.min(HUFF_TABLE_SIZE - i);
        let len = match repeat {
            true => table[i - 1],
            false => 0,
        };
        for k in 0..took {
            table[i + k] = len;
        }
        b.push(at, out, StepKind::Table(TableField::Repeat { code: num as u8, count: took as u16, len, dist }));
        i += took;
    }

    let mut at = 0usize;
    let lit = Table::new(&table[at..], HUFF_NC)?;
    at += HUFF_NC;
    let dist = Table::new(&table[at..], HUFF_DC)?;
    at += HUFF_DC;
    let low_dist = Table::new(&table[at..], HUFF_LDC)?;
    at += HUFF_LDC;
    let repeat = Table::new(&table[at..], HUFF_RC)?;
    Ok(Tables { lit, dist, low_dist, repeat })
}

/// What the trace calls one of the 430 lengths: the literal alphabet's, or one
/// of the three that follow it. The three are all about distances and share a
/// name, which is as fine as [`TableField`] draws it.
fn step_for(dist: bool, i: usize, len: u8) -> StepKind {
    let sym = match dist {
        true => (i - HUFF_NC) as u16,
        false => i as u16,
    };
    match dist {
        true => StepKind::Table(TableField::Dist { sym, len }),
        false => StepKind::Table(TableField::LitLen { sym, len }),
    }
}

/// A number written as a count of bytes and then that many bytes, which is how
/// a filter says where it starts and how long it is.
fn filter_number(bits: &mut Bits) -> Result<u32, Refusal> {
    let bytes = bits.val(2)? + 1;
    let mut v = 0u32;
    for i in 0..bytes {
        bits.inside()?;
        let byte = bits.peek16() >> 8;
        bits.skip(8);
        v = v.wrapping_add(byte << (i * 8));
    }
    Ok(v)
}

/// The filter symbol 256 defines: where it starts, how long it is, and which
/// transform.
///
/// `previous` is the last one defined, because filters have to arrive in order
/// and may not overlap. libarchive checks that, and it is worth keeping: a
/// filter reaching back into a range already handed out is a stream this cannot
/// carry out faithfully, and running it anyway would rewrite bytes that have
/// already been read.
fn parse_filter(bits: &mut Bits, write_ptr: u64, window: u64, previous: Option<&Filter>) -> Result<Filter, Refusal> {
    let start = filter_number(bits)? as u64;
    let len = filter_number(bits)?;
    let kind = bits.val(3)? as u16;
    let start = write_ptr + start;
    if len < 4 || len > 0x40_0000 || len as u64 > window >> 1 {
        return Err(Refusal::Failed);
    }
    if let Some(p) = previous {
        if start < p.start + p.len as u64 {
            return Err(Refusal::Failed);
        }
    }
    let channels = match kind {
        FILTER_DELTA => bits.val(5)? as usize + 1,
        FILTER_E8 | FILTER_E8E9 | FILTER_ARM => 0,
        // Four more numbers the field can hold, none of which RAR 5 ever
        // wrote. Refused rather than ignored: a filter left unrun is a file
        // handed back wrong with nothing saying so.
        _ => return Err(Refusal::Failed),
    };
    Ok(Filter { kind, start, len, channels })
}

/// The filtered output: a copy of what LZSS produced, with each filter's range
/// rewritten. Reads are all from `raw`, which is what the format means by the
/// window, so a filter never sees another filter's work.
fn apply_filters(raw: &[u8], filters: &[Filter]) -> Result<Vec<u8>, Refusal> {
    let mut out = raw.to_vec();
    for f in filters {
        let s = usize::try_from(f.start).map_err(|_| Refusal::Failed)?;
        let n = f.len as usize;
        let end = s.checked_add(n).ok_or(Refusal::Failed)?;
        if end > raw.len() {
            // A filter over bytes the entry never produced. libarchive waits
            // for more data here; there is no more, so the stream is wrong.
            return Err(Refusal::Failed);
        }
        let src = &raw[s..end];
        let dst = match f.kind {
            FILTER_DELTA => delta(src, f.channels),
            FILTER_E8 => e8e9(src, f.start, false),
            FILTER_E8E9 => e8e9(src, f.start, true),
            FILTER_ARM => arm(src, f.start),
            _ => return Err(Refusal::Failed),
        };
        out[s..end].copy_from_slice(&dst);
    }
    Ok(out)
}

/// Delta: the bytes are `channels` interleaved streams, each written as the
/// difference from the one before it in the same stream. Undoing it walks each
/// stream in turn, which is why the source is read straight through while the
/// destination steps by the channel count.
fn delta(src: &[u8], channels: usize) -> Vec<u8> {
    let mut out = vec![0u8; src.len()];
    let mut at = 0usize;
    for ch in 0..channels {
        let mut prev = 0u8;
        let mut to = ch;
        while to < src.len() {
            prev = prev.wrapping_sub(src[at]);
            out[to] = prev;
            at += 1;
            to += channels;
        }
    }
    out
}

/// x86 call and jump targets, written as absolute addresses by the packer and
/// put back to relative here. `start` is where the run sits in the file, which
/// is what an address is relative *to*.
fn e8e9(src: &[u8], start: u64, extended: bool) -> Vec<u8> {
    let mut out = src.to_vec();
    let mut i = 0usize;
    while i < src.len().saturating_sub(4) {
        let b = src[i];
        i += 1;
        if b == 0xe8 || (extended && b == 0xe9) {
            let offset = ((i as u64 + start) % E8_FILE_SIZE as u64) as u32;
            let addr = u32::from_le_bytes(src[i..i + 4].try_into().expect("four bytes"));
            if addr & 0x8000_0000 != 0 {
                if addr.wrapping_add(offset) & 0x8000_0000 == 0 {
                    out[i..i + 4].copy_from_slice(&addr.wrapping_add(E8_FILE_SIZE).to_le_bytes());
                }
            } else if addr.wrapping_sub(E8_FILE_SIZE) & 0x8000_0000 != 0 {
                out[i..i + 4].copy_from_slice(&addr.wrapping_sub(offset).to_le_bytes());
            }
            i += 4;
        }
    }
    out
}

/// ARM branch targets. Every instruction is four bytes and a BL is the ones
/// whose top byte is 0xEB, so unlike x86 this steps a fixed four at a time and
/// only has to look at one byte of each.
fn arm(src: &[u8], start: u64) -> Vec<u8> {
    let mut out = src.to_vec();
    let mut i = 0usize;
    while i + 3 < src.len() {
        if src[i + 3] == 0xeb {
            let v = u32::from_le_bytes(src[i..i + 4].try_into().expect("four bytes")) & 0x00ff_ffff;
            let off = v.wrapping_sub(((i as u64 + start) / 4) as u32);
            out[i..i + 4].copy_from_slice(&((off & 0x00ff_ffff) | 0xeb00_0000).to_le_bytes());
        }
        i += 4;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the decoder is actually checked against is in
    /// `crates/core/tests/rar5_real.rs`: archives WinRAR packed, against the
    /// CRC-32 each entry carries. Nothing here writes a RAR stream, because a
    /// stream written by this file's own understanding of the format would
    /// agree with this file's own understanding of the format.
    ///
    /// What is worth testing here is the parts that are arithmetic rather than
    /// agreement: the three filters, and the ways a run is turned away.
    #[test]
    fn delta_walks_one_channel_at_a_time_through_the_source() {
        // One channel: each byte out is minus the running total of the bytes
        // in, so a source of -1 counts up.
        assert_eq!(delta(&[255, 255, 255], 1), [1, 2, 3]);
        // Two channels: the source is read straight through while the output
        // steps by two, so the second and third bytes in are the second and
        // third bytes *of different streams*.
        let (a, b, c, d) = (10u8, 20u8, 30u8, 40u8);
        assert_eq!(
            delta(&[a, b, c, d], 2),
            [a.wrapping_neg(), c.wrapping_neg(), a.wrapping_add(b).wrapping_neg(), c.wrapping_add(d).wrapping_neg()]
        );
    }

    #[test]
    fn e8_rewrites_a_call_target_and_leaves_everything_else_alone() {
        // 0xE8 and then an address inside the megabyte the filter works in:
        // the packer made it absolute and this puts it back to relative, which
        // is the position of the byte after the opcode.
        let mut src = vec![0xe8, 0x00, 0x01, 0x00, 0x00];
        src.extend_from_slice(&[0; 4]);
        let out = e8e9(&src, 0, false);
        assert_eq!(&out[1..5], &0x0000_00ffu32.to_le_bytes(), "0x100 at offset 1 is 0xff away");
        assert_eq!(out[0], 0xe8, "the opcode itself is not touched");

        // 0xE9 is a jump, and only the extended filter claims it.
        let src = [0xe9, 0x00, 0x01, 0x00, 0x00, 0, 0, 0, 0];
        assert_eq!(e8e9(&src, 0, false), src, "plain E8 leaves a jump alone");
        assert_ne!(e8e9(&src, 0, true), src, "E8E9 does not");
    }

    #[test]
    fn arm_rewrites_a_branch_by_the_instruction_it_stands_at() {
        // A BL is the top byte 0xEB, and the offset it carries counts
        // instructions rather than bytes, which is the /4 below.
        let src = [0x10, 0x00, 0x00, 0xeb];
        assert_eq!(arm(&src, 0), src, "at the start of the file nothing moves");
        assert_eq!(arm(&src, 16), [0x0c, 0x00, 0x00, 0xeb], "sixteen bytes in is four instructions");
        // Anything whose top byte is not 0xEB is not a branch.
        let src = [0x10, 0x00, 0x00, 0xea];
        assert_eq!(arm(&src, 16), src);
    }

    /// A `Trace` does not compare, so the refusal is what these look at.
    fn refusal(data: &[u8], window_bits: u8, unpacked: u64) -> Option<Refusal> {
        entry(data, window_bits, unpacked).err()
    }

    #[test]
    fn bytes_that_are_not_a_rar_stream_are_refused_rather_than_guessed_at() {
        assert_eq!(refusal(b"not compressed at all", 17, 21), Some(Refusal::Failed));
        // Nothing at all, and a length that says there should have been.
        assert_eq!(refusal(b"", 17, 9), Some(Refusal::Failed));
        // A file of no bytes is packed into no blocks, which is not an error.
        assert_eq!(entry(b"", 17, 0).expect("an empty entry").0, Vec::<u8>::new());
    }

    #[test]
    fn a_dictionary_the_header_cannot_have_held_is_a_settings_refusal_not_a_failure() {
        // 128K to 4G is what four bits of the file header can say. A number
        // outside that did not come from a RAR header, and telling a reader
        // the stream is broken would send them looking for damage that is not
        // there. See [`Refusal::Settings`].
        assert_eq!(refusal(b"anything", 16, 8), Some(Refusal::Settings));
        assert_eq!(refusal(b"anything", 33, 8), Some(Refusal::Settings));
    }

    #[test]
    fn an_entry_larger_than_the_cap_is_not_opened() {
        assert_eq!(refusal(b"anything", 17, CAP_BYTES as u64 + 1), Some(Refusal::TooLarge));
    }
}
