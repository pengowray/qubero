//! Bit-level copying with a byte-aligned fast path.
//!
//! And [`Bits`], a reader that takes numbers of any width off the front of a
//! run of bits, in whichever of the two orders a format packs them.

use crate::codec::Refusal;

/// Copy `n` bits from `src` starting at bit `src_bit` into `dst` starting at bit `dst_bit`.
/// Bits outside the written range in `dst` are preserved.
pub fn copy_bits(src: &[u8], src_bit: u64, dst: &mut [u8], dst_bit: u64, n: u64) {
    if n == 0 {
        return;
    }
    if src_bit % 8 == 0 && dst_bit % 8 == 0 {
        let sb = (src_bit / 8) as usize;
        let db = (dst_bit / 8) as usize;
        let whole = (n / 8) as usize;
        dst[db..db + whole].copy_from_slice(&src[sb..sb + whole]);
        let rem = (n % 8) as u32;
        if rem != 0 {
            let mask: u8 = !(0xFFu8 >> rem);
            dst[db + whole] = (dst[db + whole] & !mask) | (src[sb + whole] & mask);
        }
        return;
    }
    // General path: one bit at a time. Rare in practice (only unaligned pieces).
    for i in 0..n {
        let b = get_bit(src, src_bit + i);
        set_bit(dst, dst_bit + i, b);
    }
}

#[inline]
pub fn get_bit(buf: &[u8], bit: u64) -> bool {
    let byte = buf[(bit / 8) as usize];
    (byte >> (7 - (bit % 8))) & 1 == 1
}

#[inline]
pub fn set_bit(buf: &mut [u8], bit: u64, value: bool) {
    let idx = (bit / 8) as usize;
    let mask = 1u8 << (7 - (bit % 8));
    if value {
        buf[idx] |= mask;
    } else {
        buf[idx] &= !mask;
    }
}

/// Number of bytes needed to hold `bits` bits.
#[inline]
pub fn bytes_for(bits: u64) -> usize {
    bits.div_ceil(8) as usize
}

/// A place in a run of bits: the one reader every format and codec here that
/// packs things narrower than a byte takes them off with.
///
/// **Which end of a byte comes first.** Two orders are in use and each format
/// keeps to one. Most take the high bit of a byte first and build a number
/// with the first bit read as its highest: GRIB, BUFR, FITS's Rice coding,
/// LHA, RAR 5 and CDF's Huffman codings. Deflate, PICO-8's `pxa` and the zero
/// suppression in a GWF vector take the low bit first and build a number with
/// the first bit read as its lowest. The order is the type, `Bits` for the
/// first and [`LowBits`] for the second, so a reader made for one cannot be
/// handed to code written for the other, and the branch between the two is
/// settled when the code is compiled rather than on every bit.
///
/// Deflate's Huffman codes are the one place the two orders meet: the bits of
/// a code come low bit first off each byte, and the code is built from them
/// highest first. That is [`Self::bit`] called in a loop, which reads the bits
/// in the reader's order and leaves the building to the caller.
///
/// `at` is how many bits have been taken, which a BUFR reading keeps as where
/// each value starts and a codec's trace records as where each step starts.
/// It may be set outright to move the reader, and may run past the end: what
/// is past the end reads as nothing, or as nought where [`Self::peek`] says so.
#[derive(Clone, Copy)]
pub(crate) struct Bits<'a, const LOW_FIRST: bool = false> {
    buf: &'a [u8],
    pub(crate) at: usize,
    /// One past the last bit that may be taken: the end of `buf`, unless
    /// [`Self::until`] brought it in.
    end: usize,
}

/// Bits taken from the low end of each byte first. See [`Bits`].
pub(crate) type LowBits<'a> = Bits<'a, true>;

impl<'a> Bits<'a> {
    /// A reader at the first bit of `buf`, taking the high bit of each byte
    /// first.
    pub(crate) fn new(buf: &'a [u8]) -> Self {
        Bits { buf, at: 0, end: buf.len() * 8 }
    }
}

impl<'a> LowBits<'a> {
    /// A reader at the first bit of `buf`, taking the low bit of each byte
    /// first.
    pub(crate) fn low_first(buf: &'a [u8]) -> Self {
        Bits { buf, at: 0, end: buf.len() * 8 }
    }
}

impl<'a, const LOW_FIRST: bool> Bits<'a, LOW_FIRST> {
    /// The same reader, stopping at bit `end` of `buf` rather than at its last
    /// byte. A deflate stream inside a zlib wrapper ends four bytes early, and
    /// a step read again ends wherever the step did, which need not be on a
    /// byte.
    pub(crate) fn until(mut self, end: usize) -> Self {
        self.end = end.min(self.buf.len() * 8);
        self
    }

    /// The bytes the bits are in, all of them, from the first.
    pub(crate) fn buf(&self) -> &'a [u8] {
        self.buf
    }

    /// One past the last bit that may be taken.
    pub(crate) fn end(&self) -> usize {
        self.end
    }

    /// How many bits are left to take.
    pub(crate) fn left(&self) -> usize {
        self.end.saturating_sub(self.at)
    }

    /// The next bit on its own, which is what a walk down a Huffman tree
    /// takes: one per branch, with the branch known only once it is taken.
    #[inline]
    pub(crate) fn bit(&mut self) -> Option<bool> {
        if self.at >= self.end {
            return None;
        }
        let byte = self.buf[self.at >> 3];
        let shift = if LOW_FIRST { self.at & 7 } else { 7 - (self.at & 7) };
        self.at += 1;
        Some((byte >> shift) & 1 == 1)
    }

    /// The next `n` bits, or nothing when there are not that many left. Zero
    /// bits is zero, which is what a group of no width means and not an error.
    #[inline]
    pub(crate) fn take(&mut self, n: u32) -> Option<u64> {
        if n > 64 || self.left() < n as usize {
            return None;
        }
        let v = self.gather(n);
        self.at += n as usize;
        Some(v)
    }

    /// The next `n` bits without taking them, with every bit past the end
    /// read as nought. At most 64.
    ///
    /// What a decoder that looks ahead by a fixed width wants near the end of
    /// its input: LHA and RAR 5 both look at the next sixteen bits to find a
    /// code of however many it turns out to be, and a code in the last byte of
    /// a stream is still a code with fewer than sixteen bits after it.
    #[inline]
    pub(crate) fn peek(&self, n: u32) -> u64 {
        self.gather(n.min(64))
    }

    /// Past the next `n` bits without reading them.
    #[inline]
    pub(crate) fn skip(&mut self, n: u32) {
        self.at += n as usize;
    }

    /// How many nought bits come before the next one bit, with that one bit
    /// taken too. Nothing when the end comes first.
    pub(crate) fn unary(&mut self) -> Option<u64> {
        let mut zeros = 0u64;
        loop {
            let avail = (8 - (self.at & 7)).min(self.left()) as u32;
            if avail == 0 {
                return None;
            }
            let (byte, used) = (self.buf[self.at >> 3], (self.at & 7) as u32);
            // The bits of this byte not yet read, lined up at the end they are
            // read from, with noughts filling the other end.
            let lead = match LOW_FIRST {
                true => u32::from(byte >> used).trailing_zeros().min(8),
                false => (u32::from(byte << used) & 0xff).leading_zeros().saturating_sub(24),
            };
            if lead >= avail {
                zeros += u64::from(avail);
                self.at += avail as usize;
                continue;
            }
            zeros += u64::from(lead);
            self.at += lead as usize + 1;
            return Some(zeros);
        }
    }

    /// On to the next byte boundary, which is where each of the tables in
    /// front of GRIB's complexly packed values begins and where a deflate
    /// stored block's length is. Says how many bits that passed over.
    pub(crate) fn align(&mut self) -> usize {
        let skip = (8 - self.at % 8) % 8;
        self.at += skip;
        skip
    }

    /// The next `n` bytes' worth of bits, which need not start on a byte.
    pub(crate) fn bytes(&mut self, n: usize) -> Option<Vec<u8>> {
        if self.left() < n * 8 {
            return None;
        }
        (0..n).map(|_| self.take(8).map(|b| b as u8)).collect()
    }

    /// Where the reader is, as the bit a codec's trace records a step at.
    pub(crate) fn pos(&self) -> u64 {
        self.at as u64
    }

    /// The next bit as a number, or the refusal a decoder gives when its
    /// input stops in the middle of something: for the codecs, where every
    /// read that runs out is the stream being refused.
    #[inline]
    pub(crate) fn read_bit(&mut self) -> Result<u32, Refusal> {
        self.bit().map(u32::from).ok_or(Refusal::Failed)
    }

    /// The next `n` bits, at most 32, as a number, or the same refusal.
    #[inline]
    pub(crate) fn read(&mut self, n: u32) -> Result<u32, Refusal> {
        match n <= 32 {
            true => self.take(n).map(|v| v as u32).ok_or(Refusal::Failed),
            false => Err(Refusal::Failed),
        }
    }

    /// `n` bits from `at` on, as many of them at a time as one byte holds,
    /// with any bit at or past `end` read as nought.
    #[inline]
    fn gather(&self, n: u32) -> u64 {
        let mut v = 0u64;
        let mut got = 0u32;
        let mut at = self.at;
        while got < n {
            let used = (at & 7) as u32;
            let here = (8 - used).min(n - got);
            let byte = u32::from(self.buf.get(at >> 3).copied().unwrap_or(0));
            let mask = (1u32 << here) - 1;
            // How many of these bits come before the end, the rest being read
            // as noughts. They are the ones read last.
            let real = self.end.saturating_sub(at).min(here as usize) as u32;
            if LOW_FIRST {
                let part = (byte >> used) & mask & ((1u32 << real) - 1);
                v |= u64::from(part) << got;
            } else {
                let part = (byte >> (8 - used - here)) & mask;
                let part = (part >> (here - real)) << (here - real);
                v = (v << here) | u64::from(part);
            }
            got += here;
            at += here as usize;
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligned_copy_with_tail() {
        let src = [0xAB, 0xCD, 0xFF];
        let mut dst = [0u8; 3];
        copy_bits(&src, 0, &mut dst, 0, 20);
        assert_eq!(dst, [0xAB, 0xCD, 0xF0]);
    }

    #[test]
    fn unaligned_copy() {
        let src = [0b1010_1010, 0b1111_0000];
        let mut dst = [0u8; 2];
        copy_bits(&src, 3, &mut dst, 1, 9);
        // src bits 3..12 = 0 1 0 1 0 | 1 1 1 1
        // dst bits 1..10
        assert_eq!(dst, [0b0010_1011, 0b1100_0000]);
    }

    /// The same two bytes read both ways: from the top of each byte with the
    /// first bit highest, and from the bottom with the first bit lowest.
    #[test]
    fn a_number_is_built_in_the_order_its_bits_are_read() {
        let buf = [0b1011_0010, 0b0110_1101];
        let mut high = Bits::new(&buf);
        assert_eq!(high.take(3), Some(0b101));
        assert_eq!(high.take(7), Some(0b10010_01));
        assert_eq!(high.bit(), Some(true));
        assert_eq!(high.take(5), Some(0b01101));
        assert_eq!(high.take(1), None);
        let mut low = Bits::low_first(&buf);
        // The low three bits of the first byte, 010, the first read lowest.
        assert_eq!(low.take(3), Some(0b010));
        // The other five, 10110, and then the low two of the second, 01.
        assert_eq!(low.take(7), Some(0b01_10110));
        assert_eq!(low.bit(), Some(true));
        assert_eq!(low.take(5), Some(0b01101));
        assert_eq!(low.take(1), None);
    }

    /// Every width at every starting bit, against the same bits read one at a
    /// time, in both orders.
    #[test]
    fn a_wide_take_is_the_bits_one_at_a_time() {
        let buf: Vec<u8> = (0..24u32).map(|i| (i.wrapping_mul(0x9e37_79b9) >> 13) as u8).collect();
        for start in 0..16 {
            for n in 0..=64u32 {
                let (mut hi, mut lo) = (Bits::new(&buf), Bits::low_first(&buf));
                hi.at = start;
                lo.at = start;
                let (mut one_hi, mut one_lo) = (hi, lo);
                let (mut want_hi, mut want_lo) = (0u64, 0u64);
                for i in 0..n {
                    want_hi = (want_hi << 1) | u64::from(one_hi.bit().unwrap());
                    want_lo |= u64::from(one_lo.bit().unwrap()) << i;
                }
                assert_eq!(hi.peek(n), want_hi, "high first, {n} bits from {start}");
                assert_eq!(hi.take(n), Some(want_hi), "high first, {n} bits from {start}");
                assert_eq!(lo.take(n), Some(want_lo), "low first, {n} bits from {start}");
                assert_eq!((hi.at, lo.at), (start + n as usize, start + n as usize));
            }
        }
    }

    /// A reader brought in to end mid-byte stops there, and a look past its
    /// end sees noughts rather than the bits that are really there.
    #[test]
    fn the_end_can_be_anywhere_and_past_it_is_nought() {
        let buf = [0xff, 0xff];
        let mut high = Bits::new(&buf).until(11);
        assert_eq!(high.left(), 11);
        assert_eq!(high.peek(16), 0xffe0);
        assert_eq!(high.take(12), None);
        assert_eq!(high.take(11), Some(0x7ff));
        assert_eq!(high.bit(), None);
        let low = Bits::low_first(&buf).until(11);
        assert_eq!(low.peek(16), 0x07ff);
        let past = Bits { at: 20, ..Bits::new(&buf) };
        assert_eq!((past.peek(8), past.left()), (0, 0));
    }

    #[test]
    fn unary_counts_noughts_across_bytes_in_either_order() {
        let buf = [0b0000_0000, 0b0001_0000, 0b0000_1000];
        let mut high = Bits::new(&buf);
        assert_eq!(high.unary(), Some(11));
        assert_eq!(high.at, 12);
        assert_eq!(high.unary(), Some(8));
        assert_eq!(high.unary(), None);
        let mut low = Bits::low_first(&buf);
        assert_eq!(low.unary(), Some(12));
        assert_eq!(low.unary(), Some(6));
        assert_eq!(low.unary(), None);
    }
}
