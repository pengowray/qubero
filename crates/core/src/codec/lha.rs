//! LHA's `-lh4-` through `-lh7-`: LZSS against a window, under Huffman codes
//! rebuilt every few thousand symbols.
//!
//! Written against <https://github.com/fragglet/lhasa> (ISC), whose
//! `lib/lh_new_decoder.c` is the same algorithm for all four methods with two
//! numbers changed, and checked against `src/huf.c` of LHa for UNIX, which
//! reads the same stream through a different table structure. Where the two
//! differ it is only in how much nonsense they tolerate, and that is noted
//! below.
//!
//! ## The stream
//!
//! A run of blocks and nothing else: no signature, no end marker, and no count
//! of them. A block is a sixteen-bit count of the symbols in it, three code
//! tables, and then that many symbols. A symbol under 256 is the byte it is,
//! and anything higher is a match: its length is the symbol less 253, so the
//! shortest is three, and how far back it reaches is read from the third
//! table.
//!
//! The literal/length alphabet is one space of 510 symbols, 0 to 255 the bytes
//! and 256 to 509 the lengths 3 to 256. The offset alphabet does not hold
//! distances; it holds *widths*. Symbol `w` above zero means a distance of
//! `2^(w-1)` plus `w-1` more bits, so the code says how big the number is and
//! the number follows it, which is how one small alphabet names a distance
//! anywhere in a 64K window.
//!
//! ## Three tables, each written in the one before it
//!
//! The three are read in this order, and the order matters because each is
//! written using the one before:
//!
//! 1. The **code-length alphabet**, 19 symbols, its lengths written out flat,
//!    three bits each. A three-bit value of 7 is not 7: it means seven and
//!    more, and one bits follow until a zero, each adding one. There is one
//!    oddity, and it is genuinely arbitrary: right after the third length two
//!    bits say how many of the next lengths to skip as unused. lhasa's comment
//!    on it reads "Not sure of the reason for this".
//! 2. The **literal/length alphabet**, 510 symbols, its lengths written under
//!    the alphabet above. A symbol of 3 or more is a length of that less two;
//!    0, 1 and 2 are runs of unused symbols, of one, of 3 to 18, and of 20 to
//!    531.
//! 3. The **offset alphabet**, its lengths written flat like the first.
//!
//! Each of the three has the same special case: a count of zero does not mean
//! an empty table, it means a table of one symbol, whose number follows and
//! whose code is *no bits at all*. Every read of such a table returns that
//! symbol and moves nothing on. A file of one repeated byte is written this
//! way, and a decoder that gives a zero-bit code one bit hangs on the first.
//!
//! ## What the window is worth
//!
//! Less than the brief for this file assumed, and this is worth saying plainly.
//! The four methods are named for their windows, 4K, 8K, 32K and 64K, but the
//! window never enters the arithmetic here. The one number the decoder takes
//! from it is how wide the offset table's count is written: four bits for a
//! window of 8K or less, five above it. So `-lh4-` and `-lh5-` decode by the
//! same code exactly, and so do `-lh6-` and `-lh7-`; lhasa says as much by
//! building `-lh4-` from the `-lh5-` decoder with one field changed. The window
//! bounds how far back an *encoder* may look, and a distance past it is caught
//! here only by the stronger test that it must not reach back past the start of
//! the output.
//!
//! ## Where the stream stops
//!
//! Nothing says. The count in a block header counts symbols, not blocks, and no
//! block says it is the last; a decoder is expected to already know how long
//! the file was, because the archive's header says so, and to stop when it has
//! that many bytes. This one is not told: [`Codec::Lha`](crate::codec::Codec)
//! carries the window and nothing else, and the run it is handed is the packed
//! bytes alone.
//!
//! So it stops at a block boundary, on either of the two things that can only
//! be the end: fewer than sixteen bits left, which cannot hold another header,
//! or a count of zero, which no encoder writes. The bits an encoder pads its
//! last byte out with are zeroes, so both cases are the same case in practice.
//! Every symbol of every block is decoded, and since the encoder wrote every
//! byte of the file as a symbol, that comes to the file. Running out of input
//! part way through a block is a stream cut short, and is refused rather than
//! handed back short: the archive knows the length and the check over this run
//! would catch it, but a decoder that returns half a file quietly is the wrong
//! place to find that out.
//!
//! ## `-lh1-` is not this
//!
//! `-lh1-` packs against a 4K window with an adaptive Huffman tree reordered
//! after every symbol, and `-lh2-` and `-lh3-` are two more schemes again.
//! None of them is this decoder and none is written here; those methods stay
//! bytes, which is the template's business and is what it says.
//!
//! ## Bits
//!
//! Most significant first, which is not how deflate reads them and *is* how
//! Qubero addresses them. Unlike [`inflate`](crate::codec::inflate) and
//! [`pico8`](crate::codec::pico8), a step's bit range here needs no
//! reinterpreting: a highlight narrower than a byte lands where a reader
//! reading left to right expects it.

use crate::codec::{BlockKind, Refusal, StepField, StepKind, TableField, Trace, TraceBuilder, CAP_BYTES};

/// The literal/length alphabet: 256 bytes, then the lengths 3 to 256.
const NC: usize = 510;

/// The code-length alphabet, whose symbols are lengths 1 to 16 and the three
/// run codes below them.
const NT: usize = 19;

/// Bits the code-length table's count is written in.
const TBIT: u32 = 5;

/// Bits the literal/length table's count, and its run-of-20 extension, are
/// written in.
const CBIT: u32 = 9;

/// The shortest match, which is what the length alphabet counts up from.
const THRESHOLD: u32 = 3;

/// The longest code any of the three tables may give a symbol.
const MAX_CODE_LEN: u32 = 16;

/// Reading a bit at a time, most significant first.
struct Bits<'a> {
    data: &'a [u8],
    /// How many bits have been read, which is the position in the run.
    at: u64,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Bits<'a> {
        Bits { data, at: 0 }
    }

    fn left(&self) -> u64 {
        self.data.len() as u64 * 8 - self.at
    }

    fn bit(&mut self) -> Result<u32, Refusal> {
        if self.at >= self.data.len() as u64 * 8 {
            return Err(Refusal::Failed);
        }
        let byte = self.data[(self.at / 8) as usize];
        let set = byte >> (7 - self.at % 8) & 1;
        self.at += 1;
        Ok(set as u32)
    }

    /// `bits` bits as a number, the first one read being the highest.
    fn val(&mut self, bits: u32) -> Result<u32, Refusal> {
        let mut val = 0u32;
        for _ in 0..bits {
            val = val << 1 | self.bit()?;
        }
        Ok(val)
    }

    /// The next sixteen bits without reading them, for the one question asked
    /// between blocks: whether what is left is another block or the zero bits
    /// the last byte was padded out with.
    fn peek16(&self) -> u32 {
        let mut val = 0u32;
        for i in 0..16 {
            let at = self.at + i;
            let bit = match at < self.data.len() as u64 * 8 {
                true => self.data[(at / 8) as usize] >> (7 - at % 8) & 1,
                false => 0,
            };
            val = val << 1 | bit as u32;
        }
        val
    }
}

/// One of the three tables, as the decoder reads from it.
///
/// Canonical, exactly as deflate's are: sorted by code length and then by
/// symbol, shortest first. lhasa builds a real tree and LHa for UNIX builds a
/// lookup table, and both assign codes in that order, so a canonical decoder
/// reads what either wrote.
enum Code {
    /// A table of one symbol whose code is no bits. Reading it moves nothing
    /// on, which is the whole point of it.
    Single(u16),
    Canonical {
        /// How many symbols have each code length, indexed by the length.
        counts: [u16; MAX_CODE_LEN as usize + 1],
        /// The symbols, by length and then by symbol.
        symbols: Vec<u16>,
    },
}

impl Code {
    /// The table `lengths` describes, or a refusal if it is not a table: over
    /// -subscribed is a stream that is not one, and so is a length past 16.
    ///
    /// An under-subscribed table is allowed, since one symbol given a code of
    /// one bit is exactly how a two-symbol alphabet with one symbol used comes
    /// out, and both references build it without complaint.
    fn new(lengths: &[u8]) -> Result<Code, Refusal> {
        let mut counts = [0u16; MAX_CODE_LEN as usize + 1];
        for &len in lengths {
            if len as u32 > MAX_CODE_LEN {
                return Err(Refusal::Failed);
            }
            counts[len as usize] += 1;
        }
        // That the code lengths describe a tree: each level has room for twice
        // what the level above left over.
        let mut room = 1i64;
        for len in 1..=MAX_CODE_LEN as usize {
            room = room * 2 - counts[len] as i64;
            if room < 0 {
                return Err(Refusal::Failed);
            }
        }
        // Where each length's symbols start, and then the symbols in order.
        let mut offsets = [0u16; MAX_CODE_LEN as usize + 2];
        for len in 1..=MAX_CODE_LEN as usize {
            offsets[len + 1] = offsets[len] + counts[len];
        }
        let mut symbols = vec![0u16; offsets[MAX_CODE_LEN as usize + 1] as usize];
        for (sym, &len) in lengths.iter().enumerate() {
            if len > 0 {
                symbols[offsets[len as usize] as usize] = sym as u16;
                offsets[len as usize] += 1;
            }
        }
        Ok(Code::Canonical { counts, symbols })
    }

    /// The symbol the next bits name. A single-symbol table reads no bits.
    fn decode(&self, bits: &mut Bits) -> Result<u16, Refusal> {
        let (counts, symbols) = match self {
            Code::Single(sym) => return Ok(*sym),
            Code::Canonical { counts, symbols } => (counts, symbols),
        };
        // Walking the canonical code: at each length, the codes of that length
        // run from `first`, and `index` is where their symbols start.
        let mut code = 0u32;
        let mut first = 0u32;
        let mut index = 0u32;
        for len in 1..=MAX_CODE_LEN as usize {
            code |= bits.bit()?;
            let count = counts[len] as u32;
            if code < first + count {
                return Ok(symbols[(index + code - first) as usize]);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err(Refusal::Failed)
    }
}

/// A length written flat: three bits, and if that says seven it says seven and
/// more, with one bits following until a zero and each adding one.
fn length_value(bits: &mut Bits) -> Result<u8, Refusal> {
    let mut len = bits.val(3)?;
    if len == 7 {
        while bits.bit()? == 1 {
            len += 1;
            if len > MAX_CODE_LEN {
                return Err(Refusal::Failed);
            }
        }
    }
    Ok(len as u8)
}

/// The first and third tables, whose lengths are written out flat.
///
/// `count_bits` is how wide the count is, and `skip_at_three` is whether the
/// two-bit skip after the third length is there, which it is for the
/// code-length table and is not for the offset table. `dist` picks which of the
/// two the trace calls the entries.
fn flat_table(
    bits: &mut Bits,
    b: &mut TraceBuilder,
    out_len: u64,
    alphabet: usize,
    count_bits: u32,
    skip_at_three: bool,
    dist: bool,
) -> Result<Code, Refusal> {
    let at = bits.at;
    let n = bits.val(count_bits)? as usize;
    if n == 0 {
        // One symbol, whose code is no bits. The count and the symbol are one
        // step: neither says anything without the other.
        let sym = bits.val(count_bits)? as u16;
        if sym as usize >= alphabet {
            return Err(Refusal::Failed);
        }
        b.push(at, out_len, table_step(dist, sym, 0));
        return Ok(Code::Single(sym));
    }
    if n > alphabet {
        // LHa for UNIX would write past the end of its array here and lhasa
        // clamps the count; neither is a stream any encoder writes, so this
        // says so instead of guessing which half of it to believe.
        return Err(Refusal::Failed);
    }
    let mut lengths = vec![0u8; alphabet];
    let mut i = 0usize;
    while i < n {
        let at = bits.at;
        let len = length_value(bits)?;
        lengths[i] = len;
        b.push(at, out_len, table_step(dist, i as u16, len));
        i += 1;
        if skip_at_three && i == 3 {
            // The arbitrary one: two bits of how many of the next lengths are
            // unused. They stay zero, so this only moves the cursor on.
            let at = bits.at;
            let skip = bits.val(2)? as usize;
            b.push(at, out_len, StepKind::Table(TableField::Repeat { code: 0, count: skip as u16, len: 0, dist }));
            i = (i + skip).min(alphabet);
        }
    }
    Code::new(&lengths)
}

/// The entry a flat table's step is, which differs only in what the interface
/// calls it: the code-length alphabet's own lengths, or the offset alphabet's.
fn table_step(dist: bool, sym: u16, len: u8) -> StepKind {
    match dist {
        true => StepKind::Table(TableField::Dist { sym, len }),
        false => StepKind::Table(TableField::CodeLen { sym: sym as u8, len }),
    }
}

/// The second table: 510 code lengths, themselves written under the
/// code-length alphabet read just before.
fn code_table(bits: &mut Bits, b: &mut TraceBuilder, out_len: u64, temp: &Code) -> Result<Code, Refusal> {
    let at = bits.at;
    let n = bits.val(CBIT)? as usize;
    if n == 0 {
        let sym = bits.val(CBIT)? as u16;
        if sym as usize >= NC {
            return Err(Refusal::Failed);
        }
        b.push(at, out_len, StepKind::Table(TableField::LitLen { sym, len: 0 }));
        return Ok(Code::Single(sym));
    }
    if n > NC {
        return Err(Refusal::Failed);
    }
    let mut lengths = vec![0u8; NC];
    let mut i = 0usize;
    while i < n {
        let at = bits.at;
        let code = temp.decode(bits)?;
        if code <= 2 {
            // A run of symbols nothing uses: one, or a count that follows.
            let skip = match code {
                0 => 1u32,
                1 => bits.val(4)? + 3,
                _ => bits.val(CBIT)? + 20,
            };
            let took = (skip as usize).min(n - i);
            b.push(at, out_len, StepKind::Table(TableField::Repeat { code: code as u8, count: took as u16, len: 0, dist: false }));
            i += took;
        } else {
            let len = code as u8 - 2;
            lengths[i] = len;
            b.push(at, out_len, StepKind::Table(TableField::LitLen { sym: i as u16, len }));
            i += 1;
        }
    }
    Code::new(&lengths)
}

/// One entry's packed data. `window_bits` is how far back a match may reach:
/// 12 for `-lh4-`, 13 for `-lh5-`, 15 for `-lh6-`, 16 for `-lh7-`.
///
/// All it settles is how wide the offset table's count is written, which is
/// four bits up to a window of 8K and five above it. See the module doc: the
/// window is a bound on the encoder, and what holds a match inside the output
/// here is that it may not reach back past the start of it.
pub fn entry(data: &[u8], window_bits: u8) -> Result<(Vec<u8>, Trace), Refusal> {
    run(data, window_bits, TraceBuilder::default())
}

/// The decoding, over a builder the caller made. Only the test that has to
/// reach the coarsening path passes anything but a default one.
fn run(data: &[u8], window_bits: u8, mut b: TraceBuilder) -> Result<(Vec<u8>, Trace), Refusal> {
    // How wide the offset table's count is written, and how many symbols that
    // table may hold. LHa for UNIX calls these `pbit` and `np` and sets them
    // exactly here, in `decode_start_st1`; the alphabet is one past the widest
    // distance the window holds, since symbol `w` names distances up to 2^w,
    // except that `-lh4-` shares `-lh5-`'s and so is one wider than it needs.
    //
    // lhasa instead allows `(1 << pbit) - 1` symbols, which is 15 or 31: looser
    // than this, and no different for any file an encoder writes, since the
    // count comes from the stream and no encoder writes a symbol it cannot use.
    let (count_bits, offset_alphabet): (u32, usize) = match window_bits {
        0..=13 => (4, 14),
        14..=15 => (5, 16),
        _ => (5, 17),
    };

    let mut bits = Bits::new(data);
    let mut out: Vec<u8> = Vec::new();
    let mut coarse = false;

    loop {
        // Another block, or the end. Sixteen bits that will not fit, or a
        // count of zero, is the end: see the module doc.
        if bits.left() < 16 || bits.peek16() == 0 {
            break;
        }
        let block_in = bits.at;
        let block_out = out.len() as u64;
        b.open_block(block_in, block_out);

        let at = bits.at;
        let mut left = bits.val(16)?;
        b.push(at, out.len() as u64, StepKind::Header(StepField::BlockHeader, left));

        let temp = flat_table(&mut bits, &mut b, out.len() as u64, NT, TBIT, true, false)?;
        let lit = code_table(&mut bits, &mut b, out.len() as u64, &temp)?;
        let offsets = flat_table(&mut bits, &mut b, out.len() as u64, offset_alphabet, count_bits, false, true)?;

        // The symbols. Every one of them produces output, so the count is what
        // holds this loop rather than anything in the stream.
        let sym_start = b.steps();
        let sym_in = bits.at;
        let sym_out = out.len() as u64;
        if coarse {
            b.push(sym_in, sym_out, StepKind::Opaque);
        }
        while left > 0 {
            // Too many symbols to name one at a time: keep the map at the
            // block, and say in the trace that this is what happened.
            if !coarse && b.over_budget() {
                coarse = true;
                b.coarsen();
                b.truncate(sym_start);
                b.push(sym_in, sym_out, StepKind::Opaque);
            }
            left -= 1;
            let at = bits.at;
            let sym = lit.decode(&mut bits)?;
            if (sym as usize) < 256 {
                if out.len() >= CAP_BYTES {
                    return Err(Refusal::TooLarge);
                }
                out.push(sym as u8);
                if !coarse {
                    b.push(at, out.len() as u64 - 1, StepKind::Literal(sym as u8));
                }
                continue;
            }
            if sym as usize >= NC {
                return Err(Refusal::Failed);
            }
            let len = sym as u32 - 256 + THRESHOLD;
            // The offset code is a width, and the distance follows it in that
            // many bits less one.
            let width = offsets.decode(&mut bits)? as u32;
            let dist = match width {
                0 => 1u32,
                w => {
                    if w > 31 {
                        return Err(Refusal::Failed);
                    }
                    (1u32 << (w - 1)) + bits.val(w - 1)? + 1
                }
            };
            if dist as usize > out.len() {
                return Err(Refusal::Failed);
            }
            if out.len() + len as usize > CAP_BYTES {
                return Err(Refusal::TooLarge);
            }
            if !coarse {
                b.push(at, out.len() as u64, StepKind::Match { len, dist });
            }
            let from = out.len() - dist as usize;
            // An overlapping copy is ordinary: a distance of one fills.
            for k in 0..len as usize {
                let byte = out[from + k];
                out.push(byte);
            }
        }
        // Whether this was the last block, which is the same question as the
        // one at the top of the loop.
        let last = bits.left() < 16 || bits.peek16() == 0;
        b.close_block(bits.at, out.len() as u64, BlockKind::Dynamic, last);
    }

    // What is left is the zero bits the last byte was padded out with, and is
    // not a symbol of any block, so it sits outside them.
    let end = data.len() as u64 * 8;
    if bits.at < end {
        b.push(bits.at, out.len() as u64, StepKind::Header(StepField::Padding, 0));
    }
    b.finish_at(end, out.len() as u64);
    Ok((out, b.done()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stream written the way an encoder would, so a test can say what it
    /// means rather than what the bits come to.
    ///
    /// Only the shapes the tests need, and deliberately not a general encoder:
    /// every table it writes is either the single-symbol kind or a flat one of
    /// equal lengths, which is enough to exercise both and is a great deal less
    /// code than a Huffman encoder that would only ever be checked against this
    /// decoder anyway.
    #[derive(Default)]
    struct Writer {
        data: Vec<u8>,
        bits: u32,
    }

    impl Writer {
        fn bit(&mut self, set: bool) {
            if self.bits % 8 == 0 {
                self.data.push(0);
            }
            if set {
                let last = self.data.len() - 1;
                self.data[last] |= 1 << (7 - self.bits % 8);
            }
            self.bits += 1;
        }

        /// `n` bits of `val`, the highest first.
        fn val(&mut self, val: u32, n: u32) {
            for i in (0..n).rev() {
                self.bit(val >> i & 1 != 0);
            }
        }

        /// A length written the way a flat table writes one.
        fn length(&mut self, len: u32) {
            match len < 7 {
                true => self.val(len, 3),
                false => {
                    self.val(7, 3);
                    for _ in 7..len {
                        self.bit(true);
                    }
                    self.bit(false);
                }
            }
        }

        /// A table of one symbol, whose code is no bits.
        fn single(&mut self, sym: u32, count_bits: u32) {
            self.val(0, count_bits);
            self.val(sym, count_bits);
        }

        /// A block whose three tables are all the single-symbol kind, so every
        /// symbol of it costs nothing and the whole block is its header.
        fn single_block(&mut self, count: u32, sym: u32, offset_sym: u32, count_bits: u32) {
            self.val(count, 16);
            self.single(0, TBIT); // the code-length table, unused
            self.single(sym, CBIT);
            self.single(offset_sym, count_bits);
        }
    }

    /// A whole archive-less run of one block whose literal table names one
    /// byte: the shortest thing this format can say, and the case a decoder
    /// that gives a zero-bit code one bit hangs on.
    #[test]
    fn a_table_of_one_symbol_reads_no_bits_at_all() {
        let mut w = Writer::default();
        w.single_block(5, b'z' as u32, 0, 4);
        let (out, trace) = entry(&w.data, 13).expect("reads");
        assert_eq!(out, b"zzzzz");
        trace.check_tiles().expect("tiles");
        assert_eq!(trace.blocks().len(), 1);
        assert!(trace.blocks()[0].last);
        let kinds: Vec<_> = trace.steps().map(|s| s.kind).collect();
        assert!(kinds.contains(&StepKind::Header(StepField::BlockHeader, 5)));
        assert!(kinds.contains(&StepKind::Literal(b'z')));
        // Every one of the five literals read nothing, and the map still says
        // which bits produced each: the block header's, since that is all
        // there is.
        for byte in 0..5 {
            assert_eq!(trace.map_out(byte).unwrap().kind, StepKind::Literal(b'z'));
        }
    }

    /// A single-symbol literal table naming a match symbol, which is how a
    /// long run of one byte is really written: one literal and then matches.
    #[test]
    fn a_single_symbol_table_may_name_a_match() {
        let mut w = Writer::default();
        // One literal first, in its own block, then a block of matches.
        w.single_block(1, b'a' as u32, 0, 4);
        // Symbol 256 is a match of three, and offset symbol 0 is a distance
        // of one, which fills from the byte before.
        w.single_block(4, 256, 0, 4);
        let (out, trace) = entry(&w.data, 13).expect("reads");
        assert_eq!(out, b"a".repeat(13));
        trace.check_tiles().expect("tiles");
        assert_eq!(trace.blocks().len(), 2);
        assert!(!trace.blocks()[0].last);
        assert!(trace.blocks()[1].last);
        let kinds: Vec<_> = trace.steps().map(|s| s.kind).collect();
        assert!(kinds.contains(&StepKind::Match { len: 3, dist: 1 }));
        // The tenth byte came from a match, and the map says which one.
        let m = trace.map_out(10).unwrap();
        assert_eq!(m.kind, StepKind::Match { len: 3, dist: 1 });
        assert_eq!(m.out_bytes, 10..13);
    }

    /// The three-bit length code says seven and more, not seven.
    #[test]
    fn a_length_of_seven_is_a_length_of_seven_and_more() {
        for len in 1..=MAX_CODE_LEN {
            let mut w = Writer::default();
            w.length(len);
            let mut bits = Bits::new(&w.data);
            assert_eq!(length_value(&mut bits).expect("reads"), len as u8, "length {len}");
        }
        // And a run of ones that never stops is not a length.
        let mut w = Writer::default();
        w.val(7, 3);
        for _ in 0..40 {
            w.bit(true);
        }
        let mut bits = Bits::new(&w.data);
        assert_eq!(length_value(&mut bits).err(), Some(Refusal::Failed));
    }

    /// Codes are canonical: sorted by length and then by symbol, shortest
    /// first, which is the order both references assign them in. This is the
    /// one thing a decoder can get subtly wrong and still decode *something*,
    /// so it is asserted against codes worked out by hand.
    #[test]
    fn codes_are_assigned_shortest_first_and_then_in_symbol_order() {
        // Two symbols of one bit: 0 -> "0", 1 -> "1".
        let code = Code::new(&[1, 1]).expect("a table");
        let mut bits = Bits::new(&[0b0100_0000]);
        assert_eq!(code.decode(&mut bits).unwrap(), 0);
        assert_eq!(code.decode(&mut bits).unwrap(), 1);
        // The shorter code wins the lower number whatever order the symbols
        // came in: symbol 1 has the only one-bit code, so it is "0", and the
        // two-bit codes go to symbols 0 and 2 in that order.
        let code = Code::new(&[2, 1, 2]).expect("a table");
        let mut bits = Bits::new(&[0b0101_1000]);
        assert_eq!(code.decode(&mut bits).unwrap(), 1); // "0"
        assert_eq!(code.decode(&mut bits).unwrap(), 0); // "10"
        assert_eq!(code.decode(&mut bits).unwrap(), 2); // "11"
        // A symbol skipped over entirely takes no code at all.
        let code = Code::new(&[1, 0, 2, 2]).expect("a table");
        let mut bits = Bits::new(&[0b0101_1000]);
        assert_eq!(code.decode(&mut bits).unwrap(), 0); // "0"
        assert_eq!(code.decode(&mut bits).unwrap(), 2); // "10"
        assert_eq!(code.decode(&mut bits).unwrap(), 3); // "11"
    }

    /// The two-bit skip sits after the third length of the code-length table
    /// and nowhere else, and the offset table does not have it at all. An
    /// off-by-one here reads every later table one field out of step.
    #[test]
    fn the_two_bit_skip_comes_after_the_third_length() {
        let mut w = Writer::default();
        // Seven entries, and the skipped ones are counted among them: three
        // lengths, two skipped, two more lengths.
        w.val(7, TBIT);
        w.length(1); // symbol 0
        w.length(0); // symbol 1
        w.length(0); // symbol 2
        w.val(2, 2); // and then skip symbols 3 and 4
        w.length(2); // symbol 5
        w.length(2); // symbol 6
        let mut bits = Bits::new(&w.data);
        let mut b = TraceBuilder::default();
        let code = flat_table(&mut bits, &mut b, 0, NT, TBIT, true, false).expect("a table");
        // Symbol 0 got one bit, and symbols 5 and 6 two each: the skip landed
        // on 3 and 4, so those took no code.
        let mut read = Bits::new(&[0b0101_1000]);
        assert_eq!(code.decode(&mut read).unwrap(), 0); // "0"
        assert_eq!(code.decode(&mut read).unwrap(), 5); // "10"
        assert_eq!(code.decode(&mut read).unwrap(), 6); // "11"
        // The trace named the entries it read, and the skip among them.
        let kinds: Vec<_> = b.done().steps().map(|s| s.kind).collect();
        assert!(kinds.contains(&StepKind::Table(TableField::CodeLen { sym: 0, len: 1 })));
        assert!(kinds.contains(&StepKind::Table(TableField::CodeLen { sym: 5, len: 2 })));
        assert!(kinds.contains(&StepKind::Table(TableField::Repeat { code: 0, count: 2, len: 0, dist: false })));
    }

    /// A set of lengths that describes no tree is refused rather than decoded
    /// into whatever it happens to come to.
    #[test]
    fn lengths_that_are_not_a_tree_are_refused() {
        // Three symbols of one bit each: two is all a bit holds.
        assert_eq!(Code::new(&[1, 1, 1]).err(), Some(Refusal::Failed));
        // A length past what any of the three tables may give.
        assert_eq!(Code::new(&[17]).err(), Some(Refusal::Failed));
        // Under-subscribed is fine: one symbol of one bit is what an alphabet
        // with one used symbol and a count above zero comes to.
        assert!(Code::new(&[1]).is_ok());
    }

    /// The window settles how wide the offset table's count is written, and
    /// nothing else, so the four methods are two decoders and not four:
    /// `-lh4-` reads exactly as `-lh5-` does, and `-lh6-` exactly as `-lh7-`.
    /// lhasa says the same thing by building its `-lh4-` decoder out of the
    /// `-lh5-` one with only the dictionary size changed.
    #[test]
    fn the_four_methods_are_two_decoders() {
        // A four-bit count, which is what a window of 8K or less writes.
        let mut narrow = Writer::default();
        narrow.single_block(3, b'q' as u32, 0, 4);
        let four = entry(&narrow.data, 12).expect("reads as -lh4-");
        let five = entry(&narrow.data, 13).expect("reads as -lh5-");
        assert_eq!(four.0, b"qqq");
        assert_eq!(five.0, four.0);
        assert_eq!(five.1.len(), four.1.len());

        // A five-bit count, which is what a larger window writes.
        let mut wide = Writer::default();
        wide.single_block(3, b'q' as u32, 0, 5);
        let six = entry(&wide.data, 15).expect("reads as -lh6-");
        let seven = entry(&wide.data, 16).expect("reads as -lh7-");
        assert_eq!(six.0, b"qqq");
        assert_eq!(seven.0, six.0);
        assert_eq!(seven.1.len(), six.1.len());
    }

    /// A distance reaching back past the start of the output is refused: there
    /// is nothing there, and a window is not an excuse to invent it.
    #[test]
    fn a_match_reaching_before_the_start_is_refused() {
        let mut w = Writer::default();
        // A block of one match, with nothing written before it.
        w.single_block(1, 256, 0, 4);
        assert_eq!(entry(&w.data, 13).err(), Some(Refusal::Failed));
    }

    /// Nothing at all is nothing, not a failure, and neither is a run too
    /// short to hold a block header.
    #[test]
    fn a_run_with_no_block_in_it_reads_as_nothing() {
        for data in [&[][..], &[0][..], &[0, 0][..], &[0, 0, 0, 0][..]] {
            let (out, trace) = entry(data, 13).expect("reads");
            assert!(out.is_empty(), "{data:?}");
            trace.check_tiles().expect("tiles");
            assert_eq!(trace.in_bits(), data.len() as u64 * 8);
        }
    }

    /// A stream cut short part way through a block is refused rather than
    /// handed back as however much of a file had arrived.
    #[test]
    fn a_stream_cut_short_is_refused() {
        let mut w = Writer::default();
        w.single_block(200, b'x' as u32, 0, 4);
        w.single_block(200, 256, 0, 4);
        let full = w.data.clone();
        // Every prefix either reads as a shorter stream or refuses, and none
        // may panic.
        for n in 0..full.len() {
            let _ = entry(&full[..n], 13);
        }
        // A block header saying more symbols than there are bits for them.
        let mut w = Writer::default();
        w.val(3, 16);
        w.single(0, TBIT);
        w.val(2, CBIT); // a real table, whose lengths run off the end
        assert_eq!(entry(&w.data, 13).err(), Some(Refusal::Failed));
    }

    /// Too many symbols to name one at a time: the trace stops naming them,
    /// says so, and still tiles both spaces.
    #[test]
    fn a_long_run_coarsens_rather_than_naming_every_symbol() {
        let mut w = Writer::default();
        w.single_block(5000, b'w' as u32, 0, 4);
        let (out, trace) = entry(&w.data, 13).expect("reads");
        assert_eq!(out.len(), 5000);
        assert!(!trace.coarse());
        trace.check_tiles().expect("tiles");
        // The same run with a budget too small to name them. Reaching
        // `MAX_STEPS` for real takes a hundred megabytes of input, and this is
        // the one path that can leave a trace not tiling, so it has to be
        // reachable from a test.
        let (coarse_out, coarse) = run(&w.data, 13, TraceBuilder::with_budget(100)).expect("reads");
        assert_eq!(coarse_out, out, "coarsening changed the bytes");
        assert!(coarse.coarse());
        assert!(coarse.len() < trace.len(), "coarsening kept as many steps as naming them");
        assert_eq!(coarse.blocks().len(), trace.blocks().len(), "coarsening lost a block");
        coarse.check_tiles().expect("tiles");
    }
}
