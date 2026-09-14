//! Bit-level copying with a byte-aligned fast path.
//!
//! And [`Bits`], a reader that takes numbers of any width off the front of a
//! run of bits, most significant first, for the formats that pack that way.

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

/// A place in a run of bits, most significant first, which is how GRIB and
/// BUFR pack everything narrower than a byte. `at` is how many bits have been
/// taken, which a BUFR reading keeps as where each value starts.
pub(crate) struct Bits<'a> {
    buf: &'a [u8],
    pub(crate) at: usize,
}

impl<'a> Bits<'a> {
    pub(crate) fn new(buf: &'a [u8]) -> Self {
        Bits { buf, at: 0 }
    }

    /// The next `n` bits, or nothing when there are not that many left. Zero
    /// bits is zero, which is what a group of no width means and not an error.
    pub(crate) fn take(&mut self, n: u32) -> Option<u64> {
        if n > 64 || self.at + n as usize > self.buf.len() * 8 {
            return None;
        }
        let mut v = 0u64;
        let mut left = n as usize;
        // A byte at a time where the reader is on a byte boundary, which is
        // most of the time for text and wide values.
        while left >= 8 && self.at % 8 == 0 {
            v = (v << 8) | u64::from(self.buf[self.at / 8]);
            self.at += 8;
            left -= 8;
        }
        for _ in 0..left {
            let byte = self.buf[self.at >> 3];
            v = (v << 1) | u64::from((byte >> (7 - (self.at & 7))) & 1);
            self.at += 1;
        }
        Some(v)
    }

    /// The next bit on its own, which is what a walk down a Huffman tree
    /// takes: one per branch, with the branch known only once it is taken.
    #[inline]
    pub(crate) fn bit(&mut self) -> Option<bool> {
        let &byte = self.buf.get(self.at >> 3)?;
        let set = (byte >> (7 - (self.at & 7))) & 1 == 1;
        self.at += 1;
        Some(set)
    }

    /// On to the next byte boundary, which is where each of the tables in
    /// front of GRIB's complexly packed values begins.
    pub(crate) fn align(&mut self) {
        self.at = (self.at + 7) & !7;
    }

    /// The next `n` bytes' worth of bits, which need not start on a byte.
    pub(crate) fn bytes(&mut self, n: usize) -> Option<Vec<u8>> {
        if self.at + n * 8 > self.buf.len() * 8 {
            return None;
        }
        (0..n).map(|_| self.take(8).map(|b| b as u8)).collect()
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
}
