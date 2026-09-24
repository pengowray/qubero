//! PNG's scanlines, unfiltered with the image's own geometry.
//!
//! What comes out of a PNG's zlib stream is not the image. It is scanlines,
//! each a filter byte and then a row predicted from its neighbours (RFC 2083
//! section 6), and how long a row is depends on the header: the width, the bit
//! depth and the colour type. [`Codec::PngUnfilter`](crate::codec::Codec) is
//! handed one row length when the template is built, which is right for a
//! cartridge that is always 160 by 205 in RGBA and for nothing else.
//!
//! An interlaced image is further from it than that. Adam7 writes the image as
//! seven smaller images, one after the other, each with rows of its own width
//! and filtered only against the rows of its own pass. No one row length reads
//! the stream at all, and the unfilter that took one refused such a file whole,
//! correctly. This reads it pass by pass.
//!
//! What comes out is the unfiltered rows in the order they were written, pass
//! by pass, each row still padded to a whole byte. Putting the pixels back
//! where they go in the picture is not done here: that is a permutation, not a
//! decoding, and a trace has to run through its output in order. See
//! [`Ty::Raster`](crate::template::Ty::Raster), which places each pixel by the
//! same arithmetic, [`Geometry`].

use crate::codec::{BlockKind, Refusal, StepField, StepKind, Trace, TraceBuilder, CAP_BYTES};

/// Adam7's seven passes over each 8 by 8 tile of the image: the column and row
/// a pass starts at, and how far it steps across and down.
pub const ADAM7: [(u32, u32, u32, u32); 7] =
    [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)];

/// One pass of an image: which pixels it holds and where its rows are in the
/// unfiltered stream. An image that is not interlaced is one pass of every
/// pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pass {
    /// 1 to 7 for Adam7, and 0 for the one pass of an image that is not
    /// interlaced.
    pub number: u8,
    /// The first column and row this pass holds, and the step to the next.
    pub x0: u32,
    pub y0: u32,
    pub dx: u32,
    pub dy: u32,
    /// How many pixels a row of this pass holds, and how many rows it has.
    /// Either can be nought for a small image, and a pass with no pixels
    /// writes nothing at all, not even filter bytes.
    pub cols: u64,
    pub rows: u64,
    /// The bytes in one unfiltered row, the last one padded out to a whole
    /// byte.
    pub stride: u64,
    /// Where the pass's first unfiltered row starts, in bytes from the front
    /// of the unfiltered stream.
    pub start: u64,
}

impl Pass {
    /// The bytes this pass comes to once unfiltered.
    pub fn len(&self) -> u64 {
        self.rows * self.stride
    }

    pub fn is_empty(&self) -> bool {
        self.rows == 0 || self.cols == 0
    }
}

/// The shape of an image's scanlines, worked out once from its header, and the
/// one place the arithmetic for where a pixel is lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Geometry {
    pub width: u64,
    pub height: u64,
    pub bits_per_pixel: u64,
    pub passes: Vec<Pass>,
}

impl Geometry {
    /// The passes of a `width` by `height` image of `bits_per_pixel`, in the
    /// order they are written. Nothing when the numbers are nought or so large
    /// that the arithmetic would not fit, which no image a decoder could hold
    /// comes near.
    pub fn new(width: u64, height: u64, bits_per_pixel: u64, adam7: bool) -> Option<Geometry> {
        if width == 0 || height == 0 || bits_per_pixel == 0 || width > u32::MAX as u64 || height > u32::MAX as u64 {
            return None;
        }
        let shapes: Vec<(u8, (u32, u32, u32, u32))> = match adam7 {
            true => ADAM7.iter().enumerate().map(|(i, &p)| (i as u8 + 1, p)).collect(),
            false => vec![(0, (0, 0, 1, 1))],
        };
        let mut start = 0u64;
        let mut passes = Vec::with_capacity(shapes.len());
        for (number, (x0, y0, dx, dy)) in shapes {
            let cols = width.saturating_sub(x0 as u64).div_ceil(dx as u64);
            let rows = height.saturating_sub(y0 as u64).div_ceil(dy as u64);
            let stride = cols.checked_mul(bits_per_pixel)?.div_ceil(8);
            let pass = Pass { number, x0, y0, dx, dy, cols, rows, stride, start };
            start = start.checked_add(pass.len())?;
            passes.push(pass);
        }
        Some(Geometry { width, height, bits_per_pixel, passes })
    }

    /// Whether the rows are in seven passes rather than one.
    pub fn interlaced(&self) -> bool {
        self.passes.len() > 1
    }

    /// The bytes the unfiltered stream comes to.
    pub fn unfiltered_len(&self) -> u64 {
        self.passes.last().map_or(0, |p| p.start + p.len())
    }

    /// The bytes the filtered stream comes to: every row of every pass that
    /// has pixels, and a filter byte in front of each.
    pub fn filtered_len(&self) -> Option<u64> {
        self.passes.iter().try_fold(0u64, |n, p| match p.is_empty() {
            true => Some(n),
            false => n.checked_add(p.rows.checked_mul(p.stride.checked_add(1)?)?),
        })
    }

    /// Which pass holds the pixel at column `x` and row `y`, and where in the
    /// unfiltered stream its first bit is. Nothing for a pixel outside the
    /// image.
    pub fn pixel_bit(&self, x: u64, y: u64) -> Option<(&Pass, u64)> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let pass = match self.interlaced() {
            // Which pass a pixel is in depends only on where it sits in its
            // 8 by 8 tile, so the table is asked directly rather than searched.
            true => &self.passes[adam7_pass(x, y) as usize - 1],
            false => &self.passes[0],
        };
        let (row, col) = ((y - pass.y0 as u64) / pass.dy as u64, (x - pass.x0 as u64) / pass.dx as u64);
        Some((pass, (pass.start + row * pass.stride) * 8 + col * self.bits_per_pixel))
    }
}

/// How many bits a pixel of a PNG takes: the depth times the samples its
/// colour type has. Nothing for a depth the colour type does not allow, or a
/// colour type the specification does not define, which is a header no decoder
/// will read.
pub fn bits_per_pixel(bit_depth: i128, color_type: i128) -> Option<u8> {
    let (samples, depths): (u8, &[i128]) = match color_type {
        0 => (1, &[1, 2, 4, 8, 16]),
        2 => (3, &[8, 16]),
        3 => (1, &[1, 2, 4, 8]),
        4 => (2, &[8, 16]),
        6 => (4, &[8, 16]),
        _ => return None,
    };
    depths.contains(&bit_depth).then(|| samples * bit_depth as u8)
}

/// Which of Adam7's passes, 1 to 7, holds the pixel at `x`, `y`.
pub fn adam7_pass(x: u64, y: u64) -> u8 {
    // The 8 by 8 tile of RFC 2083's figure, row by row.
    const TILE: [[u8; 8]; 8] = [
        [1, 6, 4, 6, 2, 6, 4, 6],
        [7, 7, 7, 7, 7, 7, 7, 7],
        [5, 6, 5, 6, 5, 6, 5, 6],
        [7, 7, 7, 7, 7, 7, 7, 7],
        [3, 6, 4, 6, 3, 6, 4, 6],
        [7, 7, 7, 7, 7, 7, 7, 7],
        [5, 6, 5, 6, 5, 6, 5, 6],
        [7, 7, 7, 7, 7, 7, 7, 7],
    ];
    TILE[(y % 8) as usize][(x % 8) as usize]
}

/// Undo PNG's filtering over a whole image's scanlines, pass by pass.
///
/// `data` is everything the image's zlib stream came to. It has to be exactly
/// the rows the header describes: fewer is an image cut off, and more is bytes
/// after the image that no decoder would show, and both are refused rather
/// than read as far as they go, so that no pixel is ever read out of the wrong
/// place.
///
/// What the trace says: one block a scanline, [`BlockKind::Scanline`], and in
/// it the pass and the row within the pass, both steps of no width since the
/// file writes neither down, then the filter byte, then the row. The row is one
/// [`StepKind::Filtered`] step rather than one per byte because a filtered byte
/// is a function of its neighbours in both spaces, and a step per byte would
/// claim a precision the filters do not have. An image that is not interlaced
/// has no pass step.
pub fn unfilter_image(data: &[u8], g: &Geometry) -> Result<(Vec<u8>, Trace), Refusal> {
    if g.unfiltered_len() > CAP_BYTES as u64 {
        return Err(Refusal::TooLarge);
    }
    if g.filtered_len() != Some(data.len() as u64) {
        return Err(Refusal::Failed);
    }
    // How far back a filter looks for the byte to the left: one pixel, and one
    // byte for depths that pack several pixels into each byte.
    let bpp = g.bits_per_pixel.div_ceil(8) as usize;
    let mut b = TraceBuilder::default();
    let mut out: Vec<u8> = Vec::with_capacity(g.unfiltered_len() as usize);
    let mut at = 0usize;
    let rows_total: u64 = g.passes.iter().filter(|p| !p.is_empty()).map(|p| p.rows).sum();
    let mut done = 0u64;
    for pass in g.passes.iter().filter(|p| !p.is_empty()) {
        let stride = pass.stride as usize;
        for row in 0..pass.rows {
            let filter = data[at];
            if filter > 4 {
                return Err(Refusal::Failed);
            }
            let out_start = out.len() as u64;
            let bit = at as u64 * 8;
            b.open_block(bit, out_start);
            if g.interlaced() {
                b.push(bit, out_start, StepKind::Header(StepField::Pass, pass.number as u32));
            }
            b.push(bit, out_start, StepKind::Header(StepField::Row, row as u32));
            b.push(bit, out_start, StepKind::Header(StepField::Filter, filter as u32));
            b.push(bit + 8, out_start, StepKind::Filtered);
            let above = match row {
                0 => None,
                _ => Some(out.len() - stride),
            };
            unfilter_row(filter, &data[at + 1..at + 1 + stride], above, bpp, &mut out);
            at += 1 + stride;
            done += 1;
            b.close_block(at as u64 * 8, out.len() as u64, BlockKind::Scanline, done == rows_total);
        }
    }
    b.finish_at(data.len() as u64 * 8, out.len() as u64);
    Ok((out, b.done()))
}

/// Unfilter one row onto the end of `out`. `above` is where the row above
/// starts in `out`, or nothing for the first row of a pass, whose row above
/// reads as zeroes. `bpp` is how many bytes back the byte to the left is.
pub(crate) fn unfilter_row(filter: u8, src: &[u8], above: Option<usize>, bpp: usize, out: &mut Vec<u8>) {
    let start = out.len();
    for (i, &x) in src.iter().enumerate() {
        // The row above, at the same column, and the pixel to the left. Both
        // read zero where there is no such byte, which is what the spec says a
        // decoder does at the edges.
        let up = above.map_or(0, |a| out[a + i]);
        let left = if i >= bpp { out[start + i - bpp] } else { 0 };
        let up_left = match above {
            Some(a) if i >= bpp => out[a + i - bpp],
            _ => 0,
        };
        let value = match filter {
            0 => x,
            1 => x.wrapping_add(left),
            2 => x.wrapping_add(up),
            3 => x.wrapping_add(((left as u16 + up as u16) / 2) as u8),
            _ => x.wrapping_add(paeth(left, up, up_left)),
        };
        out.push(value);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The pass sizes of a 32 by 32 image, which is four whole tiles each way:
    /// every pass is a quarter of the one after it, give or take.
    #[test]
    fn adam7_splits_a_32_by_32_image_into_seven_smaller_ones() {
        let g = Geometry::new(32, 32, 64, true).unwrap();
        let shapes: Vec<(u64, u64)> = g.passes.iter().map(|p| (p.cols, p.rows)).collect();
        assert_eq!(shapes, [(4, 4), (4, 4), (8, 4), (8, 8), (16, 8), (16, 16), (32, 16)]);
        // Sixteen-bit RGBA is eight bytes a pixel, so this is the pixels'
        // bytes and one filter byte for each of the 60 scanlines.
        assert_eq!(g.unfiltered_len(), 32 * 32 * 8);
        assert_eq!(g.filtered_len(), Some(32 * 32 * 8 + 60));
    }

    /// A small image leaves some passes with no pixels, and those write no
    /// filter bytes at all.
    #[test]
    fn a_pass_with_no_pixels_takes_no_bytes() {
        let g = Geometry::new(1, 1, 8, true).unwrap();
        assert_eq!(g.passes.iter().filter(|p| !p.is_empty()).count(), 1);
        assert_eq!(g.filtered_len(), Some(2));
        let g = Geometry::new(3, 2, 8, true).unwrap();
        let shapes: Vec<(u64, u64)> = g.passes.iter().map(|p| (p.cols, p.rows)).collect();
        assert_eq!(shapes, [(1, 1), (0, 1), (1, 0), (1, 1), (2, 0), (1, 1), (3, 1)]);
    }

    /// Every pixel is in exactly one pass, at a bit no other pixel has.
    #[test]
    fn every_pixel_has_a_place_of_its_own() {
        for (w, h, bits) in [(13, 7, 1), (9, 17, 4), (32, 32, 16), (5, 3, 24)] {
            for adam7 in [false, true] {
                let g = Geometry::new(w, h, bits, adam7).unwrap();
                let mut seen = std::collections::HashSet::new();
                for y in 0..h {
                    for x in 0..w {
                        let (_, bit) = g.pixel_bit(x, y).unwrap();
                        assert!(bit + bits <= g.unfiltered_len() * 8, "{w}x{h} at {x},{y}");
                        assert!(seen.insert(bit), "{w}x{h} {bits} bits: {x},{y} shares bit {bit}");
                    }
                }
            }
        }
    }

    /// Sub-byte rows are padded to a whole byte, and each pass's rows are
    /// padded on their own.
    #[test]
    fn a_one_bit_row_is_padded_to_a_byte_in_every_pass() {
        let g = Geometry::new(10, 1, 1, false).unwrap();
        assert_eq!(g.passes[0].stride, 2);
        let g = Geometry::new(10, 8, 1, true).unwrap();
        // Pass 7 is every odd row, all ten pixels: two bytes a row.
        assert_eq!((g.passes[6].cols, g.passes[6].stride), (10, 2));
        // Pixel (3, 1) is in pass 7, row 0, column 3.
        let (pass, bit) = g.pixel_bit(3, 1).unwrap();
        assert_eq!(pass.number, 7);
        assert_eq!(bit, pass.start * 8 + 3);
    }

    #[test]
    fn a_stream_of_the_wrong_length_is_refused() {
        let g = Geometry::new(2, 2, 8, false).unwrap();
        assert_eq!(unfilter_image(&[0, 1, 2, 0, 3], &g).err(), Some(Refusal::Failed));
        assert_eq!(unfilter_image(&[0, 1, 2, 0, 3, 4, 9], &g).err(), Some(Refusal::Failed));
        assert_eq!(unfilter_image(&[5, 1, 2, 0, 3, 4], &g).err(), Some(Refusal::Failed));
        let (out, trace) = unfilter_image(&[0, 1, 2, 2, 3, 4], &g).unwrap();
        assert_eq!(out, [1, 2, 4, 6]);
        trace.check_tiles().unwrap();
    }

    /// Each scanline is a block that says which pass and row it is, and its
    /// filter, and its row is the one step that made its bytes.
    #[test]
    fn each_scanline_says_its_pass_row_and_filter() {
        // A 2 by 2 image interlaced: pass 1 is (0,0), pass 6 is (1,0), pass 7
        // is row 1. Each written with a different filter.
        let g = Geometry::new(2, 2, 8, true).unwrap();
        let data = [0, 10, 1, 20, 2, 5, 5];
        let (out, trace) = unfilter_image(&data, &g).unwrap();
        trace.check_tiles().unwrap();
        assert_eq!(out, [10, 20, 5, 5]);
        assert_eq!(trace.blocks().len(), 3);
        let kinds: Vec<Vec<StepKind>> =
            trace.blocks().iter().map(|b| (b.steps.start..b.steps.end).map(|i| trace.step(i as usize).unwrap().kind).collect()).collect();
        assert_eq!(
            kinds[1],
            [
                StepKind::Header(StepField::Pass, 6),
                StepKind::Header(StepField::Row, 0),
                StepKind::Header(StepField::Filter, 1),
                StepKind::Filtered
            ]
        );
        assert_eq!(kinds[2][0], StepKind::Header(StepField::Pass, 7));
        assert!(trace.blocks().iter().all(|b| b.kind == BlockKind::Scanline));
        assert!(trace.blocks()[2].last);
        // The byte at (1, 1) came from pass 7's row.
        let step = trace.map_out(3).unwrap();
        assert_eq!(step.kind, StepKind::Filtered);
        assert_eq!(step.in_bits, 5 * 8..7 * 8);
    }
}
