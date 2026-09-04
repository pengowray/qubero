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
/// is recorded as a [`StepField::BlockHeader`], which is what it is. The five
/// filters it can name are none, sub, up, average and paeth, numbered 0 to 4.
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
        b.push(at as u64 * 8, out_start, StepKind::Header(StepField::BlockHeader, filter as u32));
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
    b.close_block(data.len() as u64 * 8, out.len() as u64, BlockKind::Sequences, true);
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
    }
}
