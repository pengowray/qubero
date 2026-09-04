//! The two ways a PICO-8 cartridge packs its Lua, read a symbol at a time.
//!
//! Neither is written down anywhere Lexaloffle publishes. Both decoders here
//! were written against two readings of the format that agree with each other:
//! `src/pxa.rs` of <https://github.com/shanecelis/pico8_decompress> (MIT), a
//! Rust port of the snippet Lexaloffle circulated, and `pico_compress.py` of
//! <https://github.com/thisismypassport/shrinko8> (MIT), which reads both
//! schemes and writes them back. Where the two differ in shape they agree in
//! result; the loop bounds here follow shrinko8's, which stop on the output
//! rather than on a byte count.
//!
//! ## What each is handed
//!
//! Both runs start after the code region's eight header bytes: four magic, a
//! big-endian 16-bit length of the text, and a big-endian 16-bit length of the
//! whole thing including those eight. Those are fields of the cart and the
//! template reads them, so neither decoder here sees a header and neither is
//! told how long its output should be. Each stops on its own terms: [`pxa`]
//! when fewer than eight bits of input are left, [`old`] at the pair of zero
//! bytes that ends it.
//!
//! ## How a pxa stream ends
//!
//! Nothing in the stream says it has ended. An encoder writes a whole number
//! of bytes and pads the last one with zero bits, and the compressed length in
//! the header counts exactly those bytes, so what is left over at the end is
//! at most seven zero bits.
//!
//! Seven zero bits cannot be a symbol. A one bit starts a literal, so a run of
//! zeroes is not one; a zero bit starts a back-reference, and the shortest
//! back-reference is eleven bits. So the stream ends where fewer than eight
//! bits remain and all of them are zero, and nowhere else. Six zero-and-one
//! bits at the end are a literal and are read as one. Running out of input
//! part way through a symbol is a stream that was cut short, and is refused.
//!
//! Bits are read from the low end of a byte upwards, the way deflate reads
//! them and not the way Qubero addresses bits. A step's byte extent is the
//! same either way; only a highlight narrower than a byte sits at the other
//! end of it. See [`crate::codec::Step`].

use crate::codec::{BlockKind, Refusal, StepField, StepKind, Trace, TraceBuilder, CAP_BYTES};

/// The shortest back-reference pxa writes, which is what its length chain
/// counts up from.
const PXA_MIN_MATCH: u32 = 3;

/// How many bits a link of the length chain holds. A link of all ones says
/// another link follows.
const PXA_LEN_LINK_BITS: u32 = 3;

/// How many bits the smallest literal index is written in, before the unary
/// prefix widens it.
const PXA_INDEX_BITS: u32 = 4;

/// A literal index may not be wider than this many extra bits. Twelve extra on
/// top of four would already be past the 256 entries there are, so a longer
/// prefix is a stream that is not a stream.
const PXA_MAX_EXTRA: u32 = 16;

/// Reading a bit at a time from the low end of each byte upwards.
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

    /// Whether what is left is the zero bits a last byte was padded out with:
    /// less than a byte of them, and every one a zero.
    fn at_padding(&self) -> bool {
        let left = self.left();
        left < 8 && (left == 0 || self.data[(self.at / 8) as usize] >> (self.at % 8) == 0)
    }

    fn bit(&mut self) -> Result<bool, Refusal> {
        if self.at >= self.data.len() as u64 * 8 {
            return Err(Refusal::Failed);
        }
        let byte = self.data[(self.at / 8) as usize];
        let set = byte >> (self.at % 8) & 1 != 0;
        self.at += 1;
        Ok(set)
    }

    /// `bits` bits as a number, the first one read being the lowest.
    fn val(&mut self, bits: u32) -> Result<u32, Refusal> {
        let mut val = 0u32;
        for i in 0..bits {
            if self.bit()? {
                val |= 1 << i;
            }
        }
        Ok(val)
    }
}

/// PICO-8's `\0pxa` code compression, the stream without its header.
///
/// One bit says which of two things follows. A one is a literal: a unary
/// prefix of one bits says how much wider than four bits its index is, the
/// index follows, and it names an entry of a 256-entry table that starts as
/// the bytes 0 to 255 in order. The byte named is written out and moved to the
/// front of the table, so the bytes used most lately are the cheapest to name.
///
/// A zero is a back-reference. Two more bits pick how wide its offset is: one
/// then one is five bits, one then zero is ten, and a zero on its own is
/// fifteen. The offset is that many bits plus one, and the length is three
/// plus a chain of three-bit groups, each group of seven saying another
/// follows.
///
/// One offset does not mean an offset. An offset of one written in ten bits is
/// the marker for a run of bytes stored as they are: eight bits each until a
/// zero byte ends the run. The same offset written in five bits is an ordinary
/// back-reference to the byte just written.
///
/// What the trace says: one step a symbol, from the bit that said which kind
/// it was through the last bit it read. A literal is the byte it named, a
/// back-reference is its length and how far back it reached, and a run stored
/// as it is comes out a byte a step, ending at the zero byte that ended it.
/// The move-to-front table is not recorded: it changes on every literal, and a
/// copy of it per step would be more memory than the cart.
pub fn pxa(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    let mut bits = Bits::new(data);
    let mut b = TraceBuilder::default();
    let mut out: Vec<u8> = Vec::new();
    // The table, most lately used first. It starts as every byte in order.
    let mut table: Vec<u8> = (0..=255u8).collect();
    b.open_block(0, 0);
    while !bits.at_padding() {
        let start = bits.at;
        if bits.bit()? {
            // A literal: how much wider than four bits its index is, in unary.
            let mut extra = 0u32;
            while bits.bit()? {
                extra += 1;
                if extra > PXA_MAX_EXTRA {
                    return Err(Refusal::Failed);
                }
            }
            // Every index the narrower widths could say is already spoken for,
            // so a wider one counts on from where they stopped.
            let index = bits.val(PXA_INDEX_BITS + extra)? as usize
                + ((1usize << PXA_INDEX_BITS) << extra) - (1usize << PXA_INDEX_BITS);
            if index > 255 {
                return Err(Refusal::Failed);
            }
            let byte = table[index];
            grow(&mut out, 1)?;
            b.push(start, out.len() as u64, StepKind::Literal(byte));
            out.push(byte);
            table.remove(index);
            table.insert(0, byte);
        } else {
            // A back-reference, or the marker that stands in for one.
            let width = match bits.bit()? {
                true => match bits.bit()? {
                    true => 5,
                    false => 10,
                },
                false => 15,
            };
            let offset = bits.val(width)? + 1;
            if offset == 1 && width == 10 {
                // The thirteen bits of the marker belong to the first byte of
                // the run, so that every step of a block is one of its symbols
                // and the run has no header standing on its own.
                let mut marker = Some(start);
                loop {
                    let at = marker.take().unwrap_or(bits.at);
                    let byte = bits.val(8)? as u8;
                    if byte == 0 {
                        b.push(at, out.len() as u64, StepKind::EndOfBlock);
                        break;
                    }
                    grow(&mut out, 1)?;
                    b.push(at, out.len() as u64, StepKind::Literal(byte));
                    out.push(byte);
                }
                continue;
            }
            let mut len = PXA_MIN_MATCH;
            loop {
                let part = bits.val(PXA_LEN_LINK_BITS)?;
                len = len.checked_add(part).ok_or(Refusal::Failed)?;
                if len as usize > CAP_BYTES {
                    return Err(Refusal::TooLarge);
                }
                if part != (1 << PXA_LEN_LINK_BITS) - 1 {
                    break;
                }
            }
            if offset as usize > out.len() {
                return Err(Refusal::Failed);
            }
            grow(&mut out, len as usize)?;
            b.push(start, out.len() as u64, StepKind::Match { len, dist: offset });
            let from = out.len() - offset as usize;
            // An overlapping copy is ordinary: an offset of one fills.
            for k in 0..len as usize {
                let byte = out[from + k];
                out.push(byte);
            }
        }
    }
    let end = data.len() as u64 * 8;
    // The padding goes outside the block, since it is not a symbol of one: it
    // is the zero bits the last byte was filled out with.
    b.close_block(bits.at, out.len() as u64, BlockKind::Sequences, true);
    if bits.left() > 0 {
        b.push(bits.at, out.len() as u64, StepKind::Header(StepField::Padding, 0));
    }
    b.finish_at(end, out.len() as u64);
    Ok((out, b.done()))
}

/// The 59 characters the older scheme can name in one byte, at the index that
/// names each. Index 0 is not a character: it escapes a byte written as it is.
/// Taken from `k_old_code_table` in shrinko8's `pico_compress.py`.
const OLD_TABLE: &[u8; 60] = b"\0\n 0123456789abcdefghijklmnopqrstuvwxyz!#%(){}[]<>+=/*:;.,~_";

/// The highest byte that names a character rather than starting a
/// back-reference.
const OLD_MAX_INDEX: u8 = 0x3b;

/// PICO-8's older `:c:\0` code compression, the stream without its header.
///
/// One byte a symbol, mostly. A zero escapes the byte after it, which is
/// written out as it is, except that two zeroes end the stream. A byte up to
/// 0x3b names a character of [`OLD_TABLE`], which is the newline, the space,
/// the digits, the lower-case letters and the punctuation Lua source is mostly
/// made of. Anything higher is the first of two bytes of a back-reference: the
/// offset is `(first - 0x3c) * 16` plus the low nibble of the second, and the
/// length is the second's high nibble plus two.
///
/// What the trace says: one step a symbol, and one step for whatever follows
/// the two zero bytes, which is the rest of the room the code region has and
/// is nothing.
pub fn old(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    let mut b = TraceBuilder::default();
    let mut out: Vec<u8> = Vec::new();
    let mut at = 0usize;
    b.open_block(0, 0);
    loop {
        let &first = data.get(at).ok_or(Refusal::Failed)?;
        let start = at as u64 * 8;
        if first == 0 {
            let &second = data.get(at + 1).ok_or(Refusal::Failed)?;
            at += 2;
            if second == 0 {
                b.push(start, out.len() as u64, StepKind::EndOfBlock);
                break;
            }
            grow(&mut out, 1)?;
            b.push(start, out.len() as u64, StepKind::Literal(second));
            out.push(second);
        } else if first <= OLD_MAX_INDEX {
            at += 1;
            let byte = OLD_TABLE[first as usize];
            grow(&mut out, 1)?;
            b.push(start, out.len() as u64, StepKind::Literal(byte));
            out.push(byte);
        } else {
            let &second = data.get(at + 1).ok_or(Refusal::Failed)?;
            at += 2;
            let offset = (first as u32 - 0x3c) * 16 + (second as u32 & 0xf);
            let len = (second as u32 >> 4) + 2;
            if offset == 0 || offset as usize > out.len() {
                return Err(Refusal::Failed);
            }
            grow(&mut out, len as usize)?;
            b.push(start, out.len() as u64, StepKind::Match { len, dist: offset });
            let from = out.len() - offset as usize;
            for k in 0..len as usize {
                let byte = out[from + k];
                out.push(byte);
            }
        }
    }
    // The code region is 0x3d00 bytes whatever the code comes to, so the run
    // handed here usually ends in thousands of bytes nobody wrote.
    let end = data.len() as u64 * 8;
    b.close_block(at as u64 * 8, out.len() as u64, BlockKind::Sequences, true);
    if at < data.len() {
        b.push(at as u64 * 8, out.len() as u64, StepKind::Header(StepField::Padding, 0));
    }
    b.finish_at(end, out.len() as u64);
    Ok((out, b.done()))
}

/// That the output has room for `more` bytes without going past the cap.
/// A cart's code is at most 65,535 bytes, so this never fires on a real one;
/// it is here because a run of bytes that is not a cart may say anything.
fn grow(out: &[u8], more: usize) -> Result<(), Refusal> {
    match out.len() + more > CAP_BYTES {
        true => Err(Refusal::TooLarge),
        false => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pxa stream built the way an encoder would, so a test can say what it
    /// means rather than what the bits come to. Only what the tests need: a
    /// literal by table index, a back-reference, and a stored run.
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
                self.data[last] |= 1 << (self.bits % 8);
            }
            self.bits += 1;
        }

        fn val(&mut self, val: u32, bits: u32) {
            for i in 0..bits {
                self.bit(val >> i & 1 != 0);
            }
        }

        /// A literal at `index` of the table as it stands.
        fn literal(&mut self, index: u32) {
            self.bit(true);
            // How much wider than four bits the index has to be written.
            let mut extra = 0u32;
            while index >= ((2u32 << PXA_INDEX_BITS) << extra) - (1 << PXA_INDEX_BITS) {
                extra += 1;
            }
            for _ in 0..extra {
                self.bit(true);
            }
            self.bit(false);
            let base = ((1u32 << PXA_INDEX_BITS) << extra) - (1 << PXA_INDEX_BITS);
            self.val(index - base, PXA_INDEX_BITS + extra);
        }

        fn matched(&mut self, offset: u32, len: u32) {
            self.bit(false);
            // The narrowest width that holds the offset, which is what an
            // encoder picks.
            let width = match offset - 1 {
                v if v < 32 => 5,
                v if v < 1024 => 10,
                _ => 15,
            };
            match width {
                5 => {
                    self.bit(true);
                    self.bit(true);
                }
                10 => {
                    self.bit(true);
                    self.bit(false);
                }
                _ => self.bit(false),
            }
            self.val(offset - 1, width);
            let mut left = len - PXA_MIN_MATCH;
            loop {
                let part = left.min(7);
                self.val(part, PXA_LEN_LINK_BITS);
                left -= part;
                if part != 7 {
                    break;
                }
            }
        }

        /// The marker and a run of bytes written as they are.
        fn stored(&mut self, bytes: &[u8]) {
            self.bit(false);
            self.bit(true);
            self.bit(false);
            self.val(0, 10);
            for &byte in bytes {
                self.val(byte as u32, 8);
            }
            self.val(0, 8);
        }
    }

    /// A literal names a table entry and moves it to the front, so the same
    /// byte a second time is index 0.
    #[test]
    fn a_literal_names_a_table_entry_and_the_table_moves() {
        let mut w = Writer::default();
        w.literal(b'h' as u32);
        w.literal(b'i' as u32);
        w.literal(0); // 'i' again, now at the front
        w.literal(1); // 'h', which 'i' pushed to second place
        let (out, trace) = pxa(&w.data).expect("reads");
        assert_eq!(out, b"hiih");
        trace.check_tiles().expect("tiles");
        assert_eq!(trace.map_out(0).unwrap().kind, StepKind::Literal(b'h'));
        assert_eq!(trace.map_out(3).unwrap().kind, StepKind::Literal(b'h'));
    }

    /// An index past what four bits hold is written wider, and the widths do
    /// not overlap: index 16 is the first that needs five.
    #[test]
    fn a_wide_literal_index_counts_on_from_where_the_narrow_one_stopped() {
        for index in [0u32, 15, 16, 17, 47, 48, 200, 255] {
            let mut w = Writer::default();
            w.literal(index);
            let (out, trace) = pxa(&w.data).expect("reads");
            assert_eq!(out, vec![index as u8], "index {index}");
            trace.check_tiles().expect("tiles");
        }
    }

    /// A back-reference is an offset and a length, and it may overlap what it
    /// is copying from.
    #[test]
    fn a_back_reference_copies_from_what_is_already_out() {
        let mut w = Writer::default();
        w.literal(b'a' as u32);
        w.literal(b'b' as u32);
        w.matched(2, 6); // abab ab
        let (out, trace) = pxa(&w.data).expect("reads");
        assert_eq!(out, b"abababab");
        trace.check_tiles().expect("tiles");
        let kinds: Vec<_> = trace.steps().map(|s| s.kind).collect();
        assert!(kinds.contains(&StepKind::Match { len: 6, dist: 2 }));
        let m = trace.map_out(4).unwrap();
        assert_eq!(m.kind, StepKind::Match { len: 6, dist: 2 });
        assert_eq!(m.out_bytes, 2..8);
        // An offset of one filling from a single byte.
        let mut w = Writer::default();
        w.literal(b'z' as u32);
        w.matched(1, 5);
        assert_eq!(pxa(&w.data).unwrap().0, b"zzzzzz");
    }

    /// A length past seven is a chain, and a length that lands on a multiple
    /// of seven still writes the link that says it stopped.
    #[test]
    fn a_long_match_writes_its_length_as_a_chain() {
        for len in [3u32, 9, 10, 11, 24, 100] {
            let mut w = Writer::default();
            w.literal(b'q' as u32);
            w.matched(1, len);
            let (out, trace) = pxa(&w.data).expect("reads");
            assert_eq!(out.len(), 1 + len as usize, "length {len}");
            assert!(out.iter().all(|&b| b == b'q'));
            trace.check_tiles().expect("tiles");
        }
    }

    /// An offset of one written in ten bits is not an offset: it is the marker
    /// for bytes stored as they are, ended by a zero byte.
    #[test]
    fn ten_bits_of_zero_mark_a_run_of_bytes_written_as_they_are() {
        let mut w = Writer::default();
        w.literal(b'x' as u32);
        w.stored(b"raw!");
        w.literal(b'y' as u32);
        let (out, trace) = pxa(&w.data).expect("reads");
        assert_eq!(out, b"xraw!y");
        trace.check_tiles().expect("tiles");
        let kinds: Vec<_> = trace.steps().map(|s| s.kind).collect();
        assert!(kinds.contains(&StepKind::EndOfBlock));
        assert_eq!(trace.map_out(1).unwrap().kind, StepKind::Literal(b'r'));
        // The marker's thirteen bits are counted with the first byte of the
        // run, so the run's first step is wider than the eight it read.
        assert_eq!(trace.map_out(1).unwrap().in_bits.end - trace.map_out(1).unwrap().in_bits.start, 21);
    }

    /// The same five bits are an ordinary back-reference to the byte before.
    #[test]
    fn the_marker_is_only_the_marker_at_ten_bits_wide() {
        let mut w = Writer::default();
        w.literal(b'k' as u32);
        w.matched(1, 3); // five bits of zero, which is offset 1
        assert_eq!(pxa(&w.data).unwrap().0, b"kkkk");
    }

    /// Padding is the zero bits the last byte was filled out with, and the
    /// trace says so rather than leaving a hole in the tiling.
    #[test]
    fn the_bits_after_the_last_symbol_are_padding() {
        let mut w = Writer::default();
        w.literal(b'a' as u32);
        let (out, trace) = pxa(&w.data).expect("reads");
        assert_eq!(out, b"a");
        trace.check_tiles().expect("tiles");
        assert!(trace
            .steps()
            .any(|s| s.kind == StepKind::Header(StepField::Padding, 0)));
        assert_eq!(trace.in_bits(), w.data.len() as u64 * 8);
    }

    /// A stream cut short in the middle of a symbol is refused rather than
    /// handing back whatever had been decoded so far.
    #[test]
    fn a_stream_cut_short_is_refused() {
        let mut w = Writer::default();
        for _ in 0..40 {
            w.literal(200);
        }
        // Every whole-byte prefix either reads as a shorter stream or refuses;
        // neither may panic.
        for n in 0..w.data.len() {
            let _ = pxa(&w.data[..n]);
        }
        // A literal whose unary prefix runs off the end of the run: the byte
        // is all ones, so the index it is widening for is never written.
        assert_eq!(pxa(&[0xff]).err(), Some(Refusal::Failed));
        // A back-reference reaching further back than anything written.
        let mut w = Writer::default();
        w.literal(b'a' as u32);
        w.matched(9, 3);
        assert_eq!(pxa(&w.data).err(), Some(Refusal::Failed));
    }

    /// Nothing at all is not a failure: an empty run is empty output.
    #[test]
    fn an_empty_run_reads_as_nothing() {
        let (out, trace) = pxa(&[]).expect("reads");
        assert!(out.is_empty());
        trace.check_tiles().expect("tiles");
    }

    /// The older scheme: a table index, an escaped byte, a back-reference and
    /// the two zeroes that end it.
    #[test]
    fn the_older_scheme_reads_its_three_kinds_of_byte() {
        // 0x0d is 'a', 0x0e 'b', 0x02 a space.
        let mut data = vec![0x0d, 0x0e, 0x02];
        // An escaped byte, for anything the table has no room for.
        data.extend_from_slice(&[0x00, b'A']);
        // A back-reference. The second byte's high nibble is the length less
        // two and its low nibble the offset's low four bits, so 0x14 is three
        // bytes from four back, which is everything written so far.
        data.extend_from_slice(&[0x3c, 0x14]);
        data.extend_from_slice(&[0x00, 0x00]);
        // And the room after it that a cart's code region always has.
        data.extend_from_slice(&[0; 16]);
        let (out, trace) = old(&data).expect("reads");
        assert_eq!(out, b"ab Aab ");
        trace.check_tiles().expect("tiles");
        let kinds: Vec<_> = trace.steps().map(|s| s.kind).collect();
        assert!(kinds.contains(&StepKind::Literal(b'a')));
        assert!(kinds.contains(&StepKind::Literal(b'A')));
        assert!(kinds.contains(&StepKind::Match { len: 3, dist: 4 }));
        assert!(kinds.contains(&StepKind::EndOfBlock));
        assert!(kinds.contains(&StepKind::Header(StepField::Padding, 0)));
        assert_eq!(trace.in_bits(), data.len() as u64 * 8);
    }

    /// Every character of the table names itself.
    #[test]
    fn the_table_covers_the_characters_lua_is_mostly_made_of() {
        let mut data: Vec<u8> = (1..=OLD_MAX_INDEX).collect();
        data.extend_from_slice(&[0, 0]);
        let (out, _) = old(&data).expect("reads");
        assert_eq!(out, &OLD_TABLE[1..]);
        assert_eq!(out.len(), 59);
    }

    /// A stream with no terminator, or cut in the middle of a pair, is
    /// refused.
    #[test]
    fn an_older_stream_cut_short_is_refused() {
        assert_eq!(old(&[0x0d, 0x0e]).err(), Some(Refusal::Failed));
        // A zero with nothing after it.
        assert_eq!(old(&[0x0d, 0x00]).err(), Some(Refusal::Failed));
        // A back-reference before there is anything to copy.
        assert_eq!(old(&[0x3c, 0x14, 0, 0]).err(), Some(Refusal::Failed));
        // An offset of zero, which names nothing.
        assert_eq!(old(&[0x0d, 0x3c, 0x10, 0, 0]).err(), Some(Refusal::Failed));
        let full = [0x0d, 0x0e, 0x00, b'A', 0x3c, 0x13, 0x00, 0x00];
        for n in 0..full.len() {
            let _ = old(&full[..n]);
        }
    }
}
