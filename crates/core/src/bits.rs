//! Bit-level copying with a byte-aligned fast path.

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
