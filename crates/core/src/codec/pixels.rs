//! Two steps that are not compression but stand between a stream and the data
//! somebody hid in it.
//!
//! PNG never stores a row of pixels as it is. Each row is preceded by a filter
//! byte saying how the encoder predicted that row, and the decoder has to undo
//! the prediction before there are pixels at all (RFC 2083 section 6). So the
//! bytes coming out of an IDAT's zlib stream are not the image: they are the
//! image plus one byte a row, still folded. [`unfilter`] unfolds them.
//!
//! And a PICO-8 cartridge is a 160x205 PNG with a program hidden in the low
//! bits of the picture. Two bits of each of the four channels of a pixel carry
//! one byte of the cart, which is what [`low_bits_argb`] pulls back out. The
//! picture stays a picture; the cart is what the picture is carrying.
//!
//! Picotron does the same thing at a different width. A `.p64.png` is 512 by
//! 384 and every pixel carries eleven bits rather than eight, which is why
//! [`low_bits_rgba11`] hands back a bit stream and not one byte a pixel.

use crate::codec::{BlockKind, Refusal, StepField, StepKind, Trace, TraceBuilder, CAP_BYTES};

/// Undo PNG's per-row filtering, RFC 2083 section 6.
///
/// `stride` is the bytes in one unfiltered row, and `bpp` the bytes in one
/// pixel, which is how far back the "left" neighbour is. The input is rows of
/// `1 + stride` bytes, a filter byte and then the filtered row; the output is
/// the rows with the filter bytes gone, so it is exactly `stride` times the
/// number of rows.
///
/// What the trace says: one block a row, and inside it two steps, the filter
/// byte and the row. The row is one step rather than one per byte because a
/// filtered byte is a function of its neighbours in both spaces, and a step
/// per byte would claim a precision the filters do not have. The filter byte
/// is recorded as a [`StepField::Filter`], which names the five filters it can
/// hold: none, sub, up, average and paeth, numbered 0 to 4.
pub fn unfilter(data: &[u8], stride: u32, bpp: u8) -> Result<(Vec<u8>, Trace), Refusal> {
    let stride = stride as usize;
    let bpp = bpp as usize;
    if stride == 0 || bpp == 0 || bpp > stride {
        return Err(Refusal::Failed);
    }
    // A row is a filter byte and the row itself, and nothing else is in there:
    // a run that does not divide is not a PNG image's scanlines.
    if data.len() % (stride + 1) != 0 {
        return Err(Refusal::Failed);
    }
    let rows = data.len() / (stride + 1);
    if rows * stride > CAP_BYTES {
        return Err(Refusal::TooLarge);
    }
    let mut b = TraceBuilder::default();
    let mut out: Vec<u8> = Vec::with_capacity(rows * stride);
    for row in 0..rows {
        let at = row * (stride + 1);
        let filter = data[at];
        if filter > 4 {
            return Err(Refusal::Failed);
        }
        let out_start = (row * stride) as u64;
        b.open_block(at as u64 * 8, out_start);
        b.push(at as u64 * 8, out_start, StepKind::Header(StepField::Filter, filter as u32));
        b.push((at + 1) as u64 * 8, out_start, StepKind::Opaque);
        let src = &data[at + 1..at + 1 + stride];
        for i in 0..stride {
            // The row above, at the same column, and the pixel to the left.
            // Both read zero where there is no such byte, which is what the
            // spec says a decoder does at the edges.
            let up = match row {
                0 => 0u8,
                _ => out[(row - 1) * stride + i],
            };
            let left = match i >= bpp {
                true => out[row * stride + i - bpp],
                false => 0,
            };
            let up_left = match (row, i >= bpp) {
                (0, _) | (_, false) => 0u8,
                _ => out[(row - 1) * stride + i - bpp],
            };
            let x = src[i];
            let value = match filter {
                0 => x,
                1 => x.wrapping_add(left),
                2 => x.wrapping_add(up),
                3 => x.wrapping_add(((left as u16 + up as u16) / 2) as u8),
                _ => x.wrapping_add(paeth(left, up, up_left)),
            };
            out.push(value);
        }
        b.close_block((at + 1 + stride) as u64 * 8, out.len() as u64, BlockKind::Opaque, row + 1 == rows);
    }
    b.finish_at(data.len() as u64 * 8, out.len() as u64);
    Ok((out, b.done()))
}

/// PNG's Paeth predictor: whichever of the three neighbours the linear
/// estimate `a + b - c` comes nearest to, ties going to the left one.
fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i32 + b as i32 - c as i32;
    let (pa, pb, pc) = ((p - a as i32).abs(), (p - b as i32).abs(), (p - c as i32).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// Pull one byte out of the low two bits of each channel of an RGBA pixel,
/// the way a PICO-8 cartridge hides a program inside a picture.
///
/// The input is pixels as PNG stores them, four bytes of red, green, blue and
/// alpha. One output byte comes from one pixel: alpha carries bits 7 and 6,
/// red 5 and 4, green 3 and 2, blue 1 and 0. Everything above the low two bits
/// of a channel is the picture and is dropped here.
///
/// The trace names every byte: four input bytes in, one out, recorded as the
/// literal it came to. No coarsening, since the largest run this is ever asked
/// about is a 160x205 image and 32,800 steps.
pub fn low_bits_argb(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    if data.len() % 4 != 0 {
        return Err(Refusal::Failed);
    }
    let count = data.len() / 4;
    if count > CAP_BYTES {
        return Err(Refusal::TooLarge);
    }
    let mut b = TraceBuilder::default();
    let mut out: Vec<u8> = Vec::with_capacity(count);
    b.open_block(0, 0);
    for i in 0..count {
        let p = &data[i * 4..i * 4 + 4];
        let byte = (p[3] & 3) << 6 | (p[0] & 3) << 4 | (p[1] & 3) << 2 | (p[2] & 3);
        b.push(i as u64 * 32, i as u64, StepKind::Literal(byte));
        out.push(byte);
    }
    b.close_block(data.len() as u64 * 8, out.len() as u64, BlockKind::Pixels, true);
    b.finish_at(data.len() as u64 * 8, out.len() as u64);
    Ok((out, b.done()))
}

/// Pull eleven bits out of one RGBA pixel, the way a Picotron cartridge hides
/// a ROM inside a picture.
///
/// The input is pixels as PNG stores them, four bytes of red, green, blue and
/// alpha. One pixel carries eleven bits: the low three of red are the lowest
/// three, then the low three of green, then the low three of blue, then the
/// low two of alpha. Those elevens are laid end to end into a byte stream,
/// least significant bit first, so a byte of output straddles two pixels more
/// often than not. Everything above the bits named here is the picture.
///
/// Whatever bits are left over at the end are dropped: eight pixels come to
/// eleven whole bytes, and a Picotron image is 512 by 384, which divides.
///
/// What the trace says: one step a pixel, four bytes of input to the one or
/// two bytes of output that pixel completed. Never zero, since eleven bits are
/// more than a byte. A step is [`StepKind::Stored`] rather than a literal
/// because a literal is one byte and three steps in every eight are two, and
/// the bytes did come through as they were written.
pub fn low_bits_rgba11(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    if data.len() % 4 != 0 {
        return Err(Refusal::Failed);
    }
    let count = data.len() / 4;
    if count * 11 / 8 > CAP_BYTES {
        return Err(Refusal::TooLarge);
    }
    let mut b = TraceBuilder::default();
    let mut out: Vec<u8> = Vec::with_capacity(count * 11 / 8);
    b.open_block(0, 0);
    // The bits read and not yet written out, low bits first, and how many.
    let mut held: u32 = 0;
    let mut bits: u32 = 0;
    for i in 0..count {
        let p = &data[i * 4..i * 4 + 4];
        let word = (p[0] as u32 & 7) | (p[1] as u32 & 7) << 3 | (p[2] as u32 & 7) << 6 | (p[3] as u32 & 3) << 9;
        let at = out.len() as u64;
        held |= word << bits;
        bits += 11;
        while bits >= 8 {
            out.push(held as u8);
            held >>= 8;
            bits -= 8;
        }
        b.push(i as u64 * 32, at, StepKind::Stored);
    }
    b.close_block(data.len() as u64 * 8, out.len() as u64, BlockKind::Pixels, true);
    b.finish_at(data.len() as u64 * 8, out.len() as u64);
    Ok((out, b.done()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every filter type against a row worked out by hand, on a stride of 4
    /// and a pixel of 2 bytes so that "left" is a real distance.
    #[test]
    fn each_filter_type_puts_the_row_back() {
        // Row 0, filter none: the bytes as they are.
        let row0 = [10u8, 20, 30, 40];
        // Row 1, filter sub: each byte plus the one two back.
        // 1,2,3,4 -> 1,2,4,6.
        // Row 2, filter up: each byte plus the one above.
        // 5,5,5,5 over 1,2,4,6 -> 6,7,9,11.
        // Row 3, filter average: x + floor((left + up) / 2), over 6,7,9,11
        // with a left two back: 3, 3, then (3+9)/2 and (3+11)/2.
        // Row 4, filter paeth over row 3: x is zero, so each byte is whichever
        // neighbour the predictor picked, which here is the one above.
        let mut data = vec![0u8];
        data.extend_from_slice(&row0);
        data.extend_from_slice(&[1, 1, 2, 3, 4]);
        data.extend_from_slice(&[2, 5, 5, 5, 5]);
        data.extend_from_slice(&[3, 0, 0, 0, 0]);
        data.extend_from_slice(&[4, 0, 0, 0, 0]);
        let (out, trace) = unfilter(&data, 4, 2).unwrap();
        trace.check_tiles().unwrap();
        assert_eq!(&out[0..4], &[10, 20, 30, 40]);
        assert_eq!(&out[4..8], &[1, 2, 4, 6]);
        assert_eq!(&out[8..12], &[6, 7, 9, 11]);
        assert_eq!(&out[12..16], &[3, 3, 6, 7]);
        assert_eq!(&out[16..20], &[3, 3, 6, 7]);
        assert_eq!(out.len(), 20);
        assert_eq!(trace.blocks().len(), 5);
        assert!(trace.blocks().last().unwrap().last);
        assert_eq!(trace.out_bytes(), 20);
        assert_eq!(trace.in_bits(), data.len() as u64 * 8);
    }

    #[test]
    fn a_run_that_is_not_whole_rows_is_refused() {
        assert_eq!(unfilter(&[0, 1, 2, 3], 4, 4).err(), Some(Refusal::Failed));
        // Filter type 5 does not exist.
        assert_eq!(unfilter(&[5, 1, 2, 3, 4], 4, 4).err(), Some(Refusal::Failed));
        assert_eq!(unfilter(&[0, 1], 0, 1).err(), Some(Refusal::Failed));
    }

    #[test]
    fn a_cart_byte_is_the_low_bits_of_one_pixel_in_argb_order() {
        // r=0b01, g=0b00, b=0b11, a=0b10 makes 0b10_01_00_11.
        let px = [0xfd, 0xfc, 0xff, 0xfe];
        let (out, trace) = low_bits_argb(&px).unwrap();
        assert_eq!(out, vec![0b1001_0011]);
        trace.check_tiles().unwrap();
        assert_eq!(trace.len(), 1);
        assert_eq!(trace.step(0).unwrap().in_bits, 0..32);
        assert_eq!(trace.map_out(0).unwrap().kind, StepKind::Literal(0b1001_0011));
    }

    #[test]
    fn pixels_that_do_not_divide_into_four_are_refused() {
        assert_eq!(low_bits_argb(&[1, 2, 3]).err(), Some(Refusal::Failed));
        assert_eq!(low_bits_rgba11(&[1, 2, 3]).err(), Some(Refusal::Failed));
    }

    /// Eight pixels come to eleven bytes, and the bits run r, g, b, a from the
    /// bottom of the stream up. The first pixel here is 0b01_101_010_011, so
    /// the first byte out is 0b0101_0011 and the next three bits are 0b011.
    #[test]
    fn eleven_bits_a_pixel_are_laid_end_to_end_low_bits_first() {
        let mut px = vec![0xfb, 0xf2, 0xfd, 0xf9]; // r=3, g=2, b=5, a=1
        px.extend(std::iter::repeat(0xf8).take(28)); // seven pixels of nothing
        let (out, trace) = low_bits_rgba11(&px).unwrap();
        trace.check_tiles().unwrap();
        assert_eq!(out.len(), 11);
        assert_eq!(out[0], 0b0101_0011);
        assert_eq!(out[1], 0b0000_0011);
        assert_eq!(&out[2..], &[0; 9]);
        // One step a pixel, four bytes of input each, and one or two bytes
        // out: the third pixel is the first to carry enough for two.
        assert_eq!(trace.len(), 8);
        assert_eq!(trace.step(0).unwrap().in_bits, 0..32);
        assert_eq!(trace.step(0).unwrap().out_bytes, 0..1);
        assert_eq!(trace.step(1).unwrap().out_bytes, 1..2);
        assert_eq!(trace.step(2).unwrap().out_bytes, 2..4);
        assert_eq!(trace.blocks()[0].kind, BlockKind::Pixels);
    }
}
