//! HCOMPRESS_1: a tile's H-transform coefficients read out of their quadtrees,
//! scaled back up, and transformed back into pixels.
//!
//! Richard White's HCOMPRESS, as CFITSIO's `fits_hdecompress.c` has it. The
//! encoder takes a tile of `nx` rows of `ny` pixels (`ny` is `ZTILE1`, so the
//! pixels are in the order FITS keeps them) through the H-transform, a
//! two-dimensional Haar wavelet: each 2 by 2 block becomes its sum and three
//! differences, and the sums are transformed again, until one sum is left. The
//! coefficients are divided by `scale` and rounded, which is where a scale
//! over 1 loses information. Then each bit plane of each quadrant of the
//! coefficients is written as a quadtree, and the signs after.
//!
//! **The stream.** Two bytes `DD 99`; `nx`, `ny` and `scale` as big-endian
//! 32-bit integers; the sum of every pixel as a big-endian 64-bit integer,
//! which is the one coefficient not coded in bit planes; and three bytes, how
//! many bit planes the first quadrant, the second and third, and the fourth
//! have. Bits after that are read from the top of each byte down.
//!
//! **Quadrants.** The coefficients are split in four: rows up to `(nx+1)/2`
//! and columns up to `(ny+1)/2`, which holds the coarse sums; the rest of
//! those rows; the rest of those columns; and the rest of both. Quadrants are
//! read in that order, and within one, bit planes from the top.
//!
//! **A bit plane** opens with four bits. 0 says the plane is written directly,
//! four bits for each 2 by 2 block, row by row. 15 says it is a quadtree: one
//! code for the whole plane, then for each level down each non-zero code of
//! the level above becomes four bits for its 2 by 2 block, and every one of
//! those that is non-zero is replaced by a code read from the stream, taken
//! from the last block to the first. The last level's codes are the 2 by 2
//! blocks of the plane. Any other four bits is a broken stream. In a code of
//! four bits, 8 is the block's top left, 4 its top right, 2 its bottom left and
//! 1 its bottom right, and a block at an odd edge drops what is off it.
//!
//! A code is read by a fixed Huffman table: three bits `000` to `011` are 1,
//! 2, 4 and 8; four bits `1000` to `1100` are 3, 5, 10, 12 and 15; five bits
//! `11010` to `11110` are 6, 7, 9, 11 and 13; and six bits `111110` and
//! `111111` are 0 and 14.
//!
//! After the last plane four bits of 0 end the planes. Signs start on the
//! next byte: one bit for each non-zero coefficient in order, 1 for negative.
//! The first coefficient is then the sum from the header.
//!
//! **Undigitizing** multiplies every coefficient by `scale`, when `scale` is
//! over 1.
//!
//! **The inverse transform** runs from the coarsest level to the finest.
//! Coefficients are rounded to the precision their level kept, which the
//! encoder's own rounding needs, and the low bits of the three differences
//! are carried into the sum so that a lossless tile comes back exactly. With
//! `SMOOTH` set in the header and a scale of 2 or more, each level's
//! differences are nudged, by at most half the scale, toward the slopes the
//! neighbouring sums imply, where that would not make the image less smooth;
//! that is what takes the blockiness off a lossy tile.
//!
//! Everything is done in 64-bit integers and each pixel kept as the 32-bit
//! integer it comes to, which is what CFITSIO's `fits_hdecompress64` does and
//! what astropy calls. Where a broken stream would take a coefficient past 64
//! bits, the tile is refused rather than wrapped.

use super::{commas, pixel_word, plural, stopped, Image, Step, Tile, Values};
use crate::bits::Bits;

/// The two bytes a stream opens with.
const MAGIC: [u8; 2] = [0xdd, 0x99];

/// How many bytes the stream's header is.
const HEADER: usize = 25;

/// The most bit planes a quadrant may have: a coefficient is a 64-bit
/// integer, and its sign is written apart.
const MOST_PLANES: u8 = 63;

/// HCOMPRESS: the coefficients read, undigitized and transformed back.
pub(super) fn step(tile: &mut Tile, image: &Image, data: &[u8], pixels: usize) -> Option<Values> {
    let name = image.algorithm.as_str();
    let mut coefficients = match decode(data, pixels) {
        Ok(c) => c,
        Err(why) => {
            stopped(tile, name, data.len());
            tile.problem = Some(problem(name, &why, data.len(), pixels));
            return None;
        }
    };
    let Coefficients { nx, ny, scale, sum, planes, quadtree, direct, signs, .. } = coefficients;
    tile.steps.push(Step {
        what: name.into(),
        in_bytes: data.len(),
        out_bytes: 0,
        note: format!(
            "{ny} × {nx} coefficients, scale {scale}, sum of all pixels {sum}; bit planes by quadrant {}, {}, {}: {} as quadtrees, {} written directly; {} sign bits",
            planes[0],
            planes[1],
            planes[2],
            commas(quadtree as u64),
            commas(direct as u64),
            commas(signs as u64)
        ),
    });
    if scale > 1 {
        let scaled = undigitize(&mut coefficients.values, i64::from(scale));
        tile.steps.push(Step {
            what: "undigitize".into(),
            in_bytes: 0,
            out_bytes: 0,
            note: format!("each coefficient × {scale}, the scale it was divided by"),
        });
        if scaled.is_err() {
            tile.problem = Some(format!("Stopped at undigitize: a coefficient times {scale} is more than a 64-bit integer holds."));
            return None;
        }
    }
    let smooth = image.smooth != 0 && scale >= 2;
    let levels = ceil_log2(nx.max(ny));
    let mut note = format!("inverse H-transform over {}", plural(u64::from(levels), "level", "levels"));
    if smooth {
        note.push_str(&format!(", smoothed as it went (SMOOTH = {}), each difference moved by at most {}", image.smooth, scale >> 1));
    } else if image.smooth != 0 {
        note.push_str(&format!("; SMOOTH = {} does nothing at a scale of {scale}", image.smooth));
    }
    tile.steps.push(Step { what: "H-transform".into(), in_bytes: 0, out_bytes: 0, note });
    if hinv(&mut coefficients.values, nx, ny, smooth, i64::from(scale)).is_err() {
        tile.problem = Some("Stopped at H-transform: a coefficient came to more than a 64-bit integer holds.".into());
        return None;
    }
    // What `fits_hdecompress64` hands back: each pixel as a C `int`.
    Some(Values::Ints(coefficients.values.iter().map(|v| i64::from(*v as i32)).collect()))
}

/// Why a stream could not be read, in words.
fn problem(name: &str, why: &Broken, bytes: usize, pixels: usize) -> String {
    match why {
        Broken::Short => format!(
            "Stopped at {name}: the tile is {}, too short for the {HEADER}-byte header.",
            plural(bytes as u64, "byte", "bytes")
        ),
        Broken::Magic(m) => format!("Stopped at {name}: the tile starts with {:02X} {:02X}, not DD 99.", m[0], m[1]),
        Broken::Size { nx, ny } => format!(
            "Stopped at {name}: the header says {ny} × {nx} pixels, and the tile is {}.",
            pixel_word(pixels as u64)
        ),
        Broken::TooSmall => format!("Stopped at {name}: the tile is 1 × 1 pixel, and the H-transform needs 2 pixels along one axis at least."),
        Broken::Planes(p) => format!("Stopped at {name}: a quadrant has {p} bit planes, and a 64-bit coefficient holds at most {MOST_PLANES}."),
        Broken::Code { quadrant, plane, code } => format!(
            "Stopped at {name}: bit plane {plane} of quadrant {} opens with {code}, and only 0 and 15 are allowed.",
            quadrant + 1
        ),
        Broken::End(code) => format!("Stopped at {name}: after the last bit plane comes {code}, not the 0 that ends them."),
        Broken::RanOut => format!("Stopped at {name}: the data ran out before every coefficient was read."),
    }
}

/// Why a stream could not be read.
#[derive(Debug, PartialEq)]
enum Broken {
    /// Shorter than the header.
    Short,
    /// Not opening with `DD 99`.
    Magic([u8; 2]),
    /// A size that is not the tile's.
    Size { nx: i32, ny: i32 },
    /// A tile of one pixel, which has no H-transform.
    TooSmall,
    /// A quadrant of more bit planes than a coefficient holds.
    Planes(u8),
    /// A bit plane opening with neither 0 nor 15. The quadrant counts from 0,
    /// the plane from the lowest.
    Code { quadrant: usize, plane: u32, code: u64 },
    /// Something other than 0 after the last bit plane.
    End(u64),
    /// Fewer bits than the stream needed.
    RanOut,
}

/// A coefficient that came to more than an `i64` holds.
#[derive(Debug, PartialEq)]
struct Overflow;

/// A stream's header and coefficients, and how they were written.
#[derive(Debug)]
struct Coefficients {
    nx: usize,
    ny: usize,
    scale: i32,
    sum: i64,
    planes: [u8; 3],
    /// The coefficients, `ny` to a row.
    values: Vec<i64>,
    /// How many bit planes were quadtrees, and how many written directly.
    quadtree: usize,
    direct: usize,
    /// How many sign bits were read, one for each non-zero coefficient.
    signs: usize,
}

/// `decode` and `dodecode`: the header, every bit plane of every quadrant, and
/// the signs. See the module doc.
fn decode(data: &[u8], pixels: usize) -> Result<Coefficients, Broken> {
    let Some(head) = data.get(..HEADER) else { return Err(Broken::Short) };
    if head[..2] != MAGIC {
        return Err(Broken::Magic([head[0], head[1]]));
    }
    let int = |at: usize| i32::from_be_bytes([head[at], head[at + 1], head[at + 2], head[at + 3]]);
    let (nx, ny, scale) = (int(2), int(6), int(10));
    let sum = i64::from_be_bytes([head[14], head[15], head[16], head[17], head[18], head[19], head[20], head[21]]);
    let planes = [head[22], head[23], head[24]];
    if nx <= 0 || ny <= 0 || u64::from(nx.unsigned_abs()) * u64::from(ny.unsigned_abs()) != pixels as u64 {
        return Err(Broken::Size { nx, ny });
    }
    // The transform needs two pixels along one side at least: CFITSIO's works
    // out its first mask from half the longer side.
    if nx.max(ny) < 2 {
        return Err(Broken::TooSmall);
    }
    if let Some(p) = planes.iter().find(|p| **p > MOST_PLANES) {
        return Err(Broken::Planes(*p));
    }
    let (nx, ny) = (nx as usize, ny as usize);
    let mut out = Coefficients { nx, ny, scale, sum, planes, values: vec![0; pixels], quadtree: 0, direct: 0, signs: 0 };
    let mut bits = Bits::new(data);
    bits.at = HEADER * 8;
    let (nx2, ny2) = (nx.div_ceil(2), ny.div_ceil(2));
    let quadrants = [(0, nx2, ny2, planes[0]), (ny2, nx2, ny / 2, planes[1]), (ny * nx2, nx / 2, ny2, planes[1]), (ny * nx2 + ny2, nx / 2, ny / 2, planes[2])];
    for (q, (start, nqx, nqy, n)) in quadrants.into_iter().enumerate() {
        qtree_decode(&mut bits, &mut out, Quadrant { index: q, start, nqx, nqy, planes: n })?;
    }
    match bits.take(4) {
        Some(0) => {}
        Some(code) => return Err(Broken::End(code)),
        None => return Err(Broken::RanOut),
    }
    // The signs start on a byte of their own.
    bits.align();
    for v in out.values.iter_mut().filter(|v| **v != 0) {
        out.signs += 1;
        if bits.bit().ok_or(Broken::RanOut)? {
            // A coefficient of at most 63 bit planes is under 2^63, so its
            // negative is an i64 too.
            *v = -*v;
        }
    }
    out.values[0] = sum;
    Ok(out)
}

/// One quadrant of the coefficients: which, where its first coefficient is,
/// how many rows and columns it has, and how many bit planes.
struct Quadrant {
    index: usize,
    start: usize,
    nqx: usize,
    nqy: usize,
    planes: u8,
}

/// `qtree_decode`: each bit plane of one quadrant, from the top.
fn qtree_decode(bits: &mut Bits, out: &mut Coefficients, q: Quadrant) -> Result<(), Broken> {
    let Quadrant { index, start, nqx, nqy, planes } = q;
    let log2n = ceil_log2(nqx.max(nqy));
    let (nqx2, nqy2) = (nqx.div_ceil(2), nqy.div_ceil(2));
    // The last level is `nqx2` by `nqy2` codes, and no level is larger. A
    // quadrant of no rows still takes one code, read and thrown away.
    let mut scratch = vec![0u8; nqx2.max(1) * nqy2.max(1)];
    for plane in (0..u32::from(planes)).rev() {
        match bits.take(4).ok_or(Broken::RanOut)? {
            0 => {
                for s in scratch.iter_mut().take(nqx2 * nqy2) {
                    *s = bits.take(4).ok_or(Broken::RanOut)? as u8;
                }
                out.direct += 1;
            }
            0xf => {
                scratch[0] = huffman(bits)?;
                let (mut nx, mut ny, mut nfx, mut nfy) = (1usize, 1usize, nqx, nqy);
                let mut c = 1usize << log2n;
                for _ in 1..log2n {
                    // The sizes of each level, halving up from the quadrant's.
                    c >>= 1;
                    nx <<= 1;
                    ny <<= 1;
                    if nfx <= c { nx -= 1 } else { nfx -= c }
                    if nfy <= c { ny -= 1 } else { nfy -= c }
                    expand(bits, &mut scratch, nx, ny)?;
                }
                out.quadtree += 1;
            }
            code => return Err(Broken::Code { quadrant: index, plane, code }),
        }
        insert(&scratch, nqx, nqy, &mut out.values, start, out.ny, plane);
    }
    Ok(())
}

/// `qtree_expand` and `qtree_copy`: the codes of the level above, `(nx+1)/2`
/// by `(ny+1)/2` at the front of `b`, spread over `nx` by `ny` as the four bits
/// of each, and each of those that is non-zero read again from the stream,
/// the last first.
fn expand(bits: &mut Bits, b: &mut [u8], nx: usize, ny: usize) -> Result<(), Broken> {
    let (nx2, ny2) = (nx.div_ceil(2), ny.div_ceil(2));
    // Each code to the top left of its block, from the last, so that none is
    // written over before it is moved.
    for i in (0..nx2).rev() {
        for j in (0..ny2).rev() {
            b[2 * (ny * i + j)] = b[ny2 * i + j];
        }
    }
    for i in (0..nx).step_by(2) {
        for j in (0..ny).step_by(2) {
            let s00 = ny * i + j;
            let v = b[s00];
            if i + 1 < nx {
                b[s00 + ny] = (v >> 1) & 1;
                if j + 1 < ny {
                    b[s00 + ny + 1] = v & 1;
                }
            }
            if j + 1 < ny {
                b[s00 + 1] = (v >> 2) & 1;
            }
            b[s00] = (v >> 3) & 1;
        }
    }
    for s in b[..nx * ny].iter_mut().rev() {
        if *s != 0 {
            *s = huffman(bits)?;
        }
    }
    Ok(())
}

/// `qtree_bitins`: the four-bit codes of `a`, `(nx+1)/2` to a row, set as bit
/// `plane` of the `nx` by `ny` coefficients at `start`, `n` to a row.
fn insert(a: &[u8], nx: usize, ny: usize, b: &mut [i64], start: usize, n: usize, plane: u32) {
    let value = 1i64 << plane;
    let mut k = 0;
    for i in (0..nx).step_by(2) {
        for j in (0..ny).step_by(2) {
            let (v, s00) = (a[k], start + n * i + j);
            let (down, right) = (i + 1 < nx, j + 1 < ny);
            if v & 8 != 0 {
                b[s00] |= value;
            }
            if v & 4 != 0 && right {
                b[s00 + 1] |= value;
            }
            if v & 2 != 0 && down {
                b[s00 + n] |= value;
            }
            if v & 1 != 0 && down && right {
                b[s00 + n + 1] |= value;
            }
            k += 1;
        }
    }
}

/// `input_huffman`: one four-bit code. See the module doc for the table.
fn huffman(bits: &mut Bits) -> Result<u8, Broken> {
    let c = bits.take(3).ok_or(Broken::RanOut)?;
    let mut bit = || bits.bit().map(u64::from).ok_or(Broken::RanOut);
    if c < 4 {
        return Ok(1 << c);
    }
    let c = (c << 1) | bit()?;
    let four = match c {
        8 => Some(3),
        9 => Some(5),
        10 => Some(10),
        11 => Some(12),
        12 => Some(15),
        _ => None,
    };
    if let Some(v) = four {
        return Ok(v);
    }
    let c = (c << 1) | bit()?;
    let five = match c {
        26 => Some(6),
        27 => Some(7),
        28 => Some(9),
        29 => Some(11),
        30 => Some(13),
        _ => None,
    };
    if let Some(v) = five {
        return Ok(v);
    }
    let c = (c << 1) | bit()?;
    Ok(if c == 62 { 0 } else { 14 })
}

/// `undigitize`: every coefficient times the scale.
fn undigitize(values: &mut [i64], scale: i64) -> Result<(), Overflow> {
    for v in values {
        *v = v.checked_mul(scale).ok_or(Overflow)?;
    }
    Ok(())
}

/// How many times 2 must be doubled to reach `n`: 0 for 1, 1 for 2, 2 for 3
/// and 4, 3 for 5 to 8. CFITSIO works it out in floats and corrects the
/// rounding, which comes to the same.
fn ceil_log2(n: usize) -> u32 {
    if n <= 1 { 0 } else { usize::BITS - (n - 1).leading_zeros() }
}

/// A value worked out in 128 bits, back in the 64 a coefficient has.
fn narrow(v: i128) -> Result<i64, Overflow> {
    i64::try_from(v).map_err(|_| Overflow)
}

/// A coefficient rounded to a multiple of the bit `mask` keeps: half up for a
/// positive one and half down for a negative one, as `prnd` and `nrnd` say.
fn round(v: i128, prnd: i128, nrnd: i128, mask: i128) -> i128 {
    (v + if v >= 0 { prnd } else { nrnd }) & mask
}

/// `hinv`: the inverse H-transform, in place, from the coarsest level down.
///
/// The arithmetic is CFITSIO's, done in 128 bits so that nothing on the way
/// can overflow, and every value put back is checked to fit an `i64`.
fn hinv(a: &mut [i64], nx: usize, ny: usize, smooth: bool, scale: i64) -> Result<(), Overflow> {
    let nmax = nx.max(ny);
    // At least 1: `decode` refuses a tile of fewer than two pixels a side.
    let log2n = ceil_log2(nmax).max(1);
    let mut tmp = vec![0i64; nmax.div_ceil(2)];
    let mut shift = 1u32;
    let mut bit0: i128 = 1 << (log2n - 1);
    let mut bit1 = bit0 << 1;
    let bit2 = bit0 << 2;
    let mut mask0 = -bit0;
    let mut mask1 = mask0 << 1;
    let mask2 = mask0 << 2;
    let mut prnd0 = bit0 >> 1;
    let mut prnd1 = bit1 >> 1;
    let prnd2 = bit2 >> 1;
    let mut nrnd0 = prnd0 - 1;
    let mut nrnd1 = prnd1 - 1;
    let nrnd2 = prnd2 - 1;
    // The sum is rounded to a multiple of bit2.
    a[0] = narrow(round(i128::from(a[0]), prnd2, nrnd2, mask2))?;
    let (mut nxtop, mut nytop, mut nxf, mut nyf) = (1usize, 1usize, nx, ny);
    let mut c = 1usize << log2n;
    for k in (0..log2n).rev() {
        c >>= 1;
        nxtop <<= 1;
        nytop <<= 1;
        if nxf <= c { nxtop -= 1 } else { nxf -= c }
        if nyf <= c { nytop -= 1 } else { nyf -= c }
        // The last level divides by 4 rather than 2, and rounds a negative
        // coefficient of the finest bit as it does a positive one.
        if k == 0 {
            nrnd0 = 0;
            shift = 2;
        }
        // Interleave the coefficients of this level along each axis.
        for i in 0..nxtop {
            unshuffle(&mut a[ny * i..], nytop, 1, &mut tmp);
        }
        for j in 0..nytop {
            unshuffle(&mut a[j..], nxtop, ny, &mut tmp);
        }
        if smooth {
            hsmooth(a, nxtop, nytop, ny, scale)?;
        }
        let (oddx, oddy) = (nxtop % 2, nytop % 2);
        let at = |a: &[i64], i: usize| i128::from(a[i]);
        let mut i = 0;
        while i < nxtop - oddx {
            let mut s00 = ny * i;
            let mut s10 = s00 + ny;
            let mut j = 0;
            while j < nytop - oddy {
                let h0 = at(a, s00);
                let hx = round(at(a, s10), prnd1, nrnd1, mask1);
                let hy = round(at(a, s00 + 1), prnd1, nrnd1, mask1);
                let hc = round(at(a, s10 + 1), prnd0, nrnd0, mask0);
                // Bit 0 of hc into hx and hy, and bits 0 and 1 of all three
                // into h0.
                let lowbit0 = hc & bit0;
                let hx = if hx >= 0 { hx - lowbit0 } else { hx + lowbit0 };
                let hy = if hy >= 0 { hy - lowbit0 } else { hy + lowbit0 };
                let lowbit1 = (hc ^ hx ^ hy) & bit1;
                let h0 = if h0 >= 0 { h0 + lowbit0 - lowbit1 } else { h0 + if lowbit0 == 0 { lowbit1 } else { lowbit0 - lowbit1 } };
                a[s10 + 1] = narrow((h0 + hx + hy + hc) >> shift)?;
                a[s10] = narrow((h0 + hx - hy - hc) >> shift)?;
                a[s00 + 1] = narrow((h0 - hx + hy - hc) >> shift)?;
                a[s00] = narrow((h0 - hx - hy + hc) >> shift)?;
                s00 += 2;
                s10 += 2;
                j += 2;
            }
            if oddy == 1 {
                // The last of a row of odd length, which has no hy or hc.
                let h0 = at(a, s00);
                let hx = round(at(a, s10), prnd1, nrnd1, mask1);
                let lowbit1 = hx & bit1;
                let h0 = if h0 >= 0 { h0 - lowbit1 } else { h0 + lowbit1 };
                a[s10] = narrow((h0 + hx) >> shift)?;
                a[s00] = narrow((h0 - hx) >> shift)?;
            }
            i += 2;
        }
        if oddx == 1 {
            // The last row of a column of odd length, which has no hx or hc.
            let mut s00 = ny * i;
            let mut j = 0;
            while j < nytop - oddy {
                let h0 = at(a, s00);
                let hy = round(at(a, s00 + 1), prnd1, nrnd1, mask1);
                let lowbit1 = hy & bit1;
                let h0 = if h0 >= 0 { h0 - lowbit1 } else { h0 + lowbit1 };
                a[s00 + 1] = narrow((h0 + hy) >> shift)?;
                a[s00] = narrow((h0 - hy) >> shift)?;
                s00 += 2;
                j += 2;
            }
            if oddy == 1 {
                a[s00] = narrow(at(a, s00) >> shift)?;
            }
        }
        // The next level down keeps one bit more.
        bit1 = bit0;
        bit0 >>= 1;
        mask1 = mask0;
        mask0 >>= 1;
        prnd1 = prnd0;
        prnd0 >>= 1;
        nrnd1 = nrnd0;
        nrnd0 = prnd0 - 1;
    }
    Ok(())
}

/// `unshuffle`: the first `(n+1)/2` of every `n2`th value of `a` to the even
/// places and the rest to the odd places, in order.
fn unshuffle(a: &mut [i64], n: usize, n2: usize, tmp: &mut [i64]) {
    let nhalf = n.div_ceil(2);
    for (t, i) in (nhalf..n).enumerate() {
        tmp[t] = a[n2 * i];
    }
    for i in (0..nhalf).rev() {
        a[2 * n2 * i] = a[n2 * i];
    }
    for (t, i) in (1..n).step_by(2).enumerate() {
        a[n2 * i] = tmp[t];
    }
}

/// `hsmooth`: each difference of the level `nxtop` by `nytop` moved toward
/// the slope its neighbouring sums imply, where that keeps the sums in order,
/// by at most half the scale. The coefficients at the edges are left as they
/// are.
fn hsmooth(a: &mut [i64], nxtop: usize, nytop: usize, ny: usize, scale: i64) -> Result<(), Overflow> {
    let smax = i128::from(scale >> 1);
    if smax <= 0 {
        return Ok(());
    }
    let at = |a: &[i64], i: usize| i128::from(a[i]);
    let ny2 = ny << 1;
    // How far a difference moves: toward `diff` held inside what the
    // neighbours allow, divided by `unit` rounding toward zero, and no further
    // than `smax`.
    let nudge = |diff: i128, dmin: i128, dmax: i128, now: i128, unit: u32| -> i128 {
        let diff = diff.min(dmax).max(dmin);
        let s = diff - (now << unit);
        let s = if s >= 0 { s >> unit } else { (s + (1 << unit) - 1) >> unit };
        s.min(smax).max(-smax)
    };
    // The x differences, hx, in every row but the first and last pair.
    for i in (2..nxtop.saturating_sub(2)).step_by(2) {
        for j in (0..nytop).step_by(2) {
            let s00 = ny * i + j;
            let s10 = s00 + ny;
            let (hm, h0, hp) = (at(a, s00 - ny2), at(a, s00), at(a, s00 + ny2));
            let dmax = (hp - h0).min(h0 - hm).max(0) << 2;
            let dmin = (hp - h0).max(h0 - hm).min(0) << 2;
            if dmin < dmax {
                let now = at(a, s10);
                a[s10] = narrow(now + nudge(hp - hm, dmin, dmax, now, 3))?;
            }
        }
    }
    // The y differences, hy, in every column but the first and last pair.
    for i in (0..nxtop).step_by(2) {
        for j in (2..nytop.saturating_sub(2)).step_by(2) {
            let s00 = ny * i + j;
            let (hm, h0, hp) = (at(a, s00 - 2), at(a, s00), at(a, s00 + 2));
            let dmax = (hp - h0).min(h0 - hm).max(0) << 2;
            let dmin = (hp - h0).max(h0 - hm).min(0) << 2;
            if dmin < dmax {
                let now = at(a, s00 + 1);
                a[s00 + 1] = narrow(now + nudge(hp - hm, dmin, dmax, now, 3))?;
            }
        }
    }
    // The cross differences, hc, away from every edge.
    for i in (2..nxtop.saturating_sub(2)).step_by(2) {
        for j in (2..nytop.saturating_sub(2)).step_by(2) {
            let s00 = ny * i + j;
            let s10 = s00 + ny;
            let hmm = at(a, s00 - ny2 - 2);
            let hpm = at(a, s00 + ny2 - 2);
            let hmp = at(a, s00 - ny2 + 2);
            let hpp = at(a, s00 + ny2 + 2);
            let h0 = at(a, s00);
            let diff = hpp + hmm - hmp - hpm;
            let hx2 = at(a, s10) << 1;
            let hy2 = at(a, s00 + 1) << 1;
            let m1 = ((hpp - h0).max(0) - hx2 - hy2).min((h0 - hpm).max(0) + hx2 - hy2);
            let m2 = ((h0 - hmp).max(0) - hx2 + hy2).min((hmm - h0).max(0) + hx2 + hy2);
            let dmax = m1.min(m2) << 4;
            let m1 = ((hpp - h0).min(0) - hx2 - hy2).max((h0 - hpm).min(0) + hx2 - hy2);
            let m2 = ((h0 - hmp).min(0) - hx2 + hy2).max((hmm - h0).min(0) + hx2 + hy2);
            let dmin = m1.max(m2) << 4;
            if dmin < dmax {
                let now = at(a, s10 + 1);
                a[s10 + 1] = narrow(now + nudge(diff, dmin, dmax, now, 6))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::tests::cards;
    use super::super::{decode as tile, Place, Row, Stored};
    use super::*;

    /// A stream's header: its size, scale and sum, and its bit planes by
    /// quadrant.
    fn header(nx: i32, ny: i32, scale: i32, sum: i64, planes: [u8; 3]) -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        for v in [nx, ny, scale] {
            b.extend_from_slice(&v.to_be_bytes());
        }
        b.extend_from_slice(&sum.to_be_bytes());
        b.extend_from_slice(&planes);
        b
    }

    /// A 2 by 2 tile whose sum is 400 and whose x and y differences are 1,
    /// one bit plane each: one written as a quadtree, one directly, and the y
    /// difference negative.
    fn two_by_two() -> Vec<u8> {
        let mut b = header(2, 2, 0, 400, [0, 1, 0]);
        // 1111 opens quadrant 2's plane as a quadtree, and 011 is its one code,
        // 8. 0000 opens quadrant 3's plane written directly, and 1000 is its
        // code. 0000 ends the planes, and five bits pad the byte.
        b.extend_from_slice(&[0b1111_0110, 0b0001_0000, 0b0000_0000]);
        // Signs, one for each coefficient that is not zero: minus, then plus.
        b.push(0b1000_0000);
        b
    }

    /// A tile of an image of 32-bit integers, `nx` rows of `ny`, compressed
    /// with HCOMPRESS as `data`, decoded the way the panel decodes it.
    fn decoded(data: &[u8], nx: u64, ny: u64, smooth: i64) -> Tile {
        let lines = [
            "ZBITPIX =                   32".to_string(),
            "ZNAXIS  =                    2".into(),
            format!("ZNAXIS1 = {ny:20}"),
            format!("ZNAXIS2 = {nx:20}"),
            format!("ZTILE1  = {ny:20}"),
            format!("ZTILE2  = {nx:20}"),
            "ZCMPTYPE= 'HCOMPRESS_1'".into(),
            "ZNAME1  = 'SCALE   '".into(),
            "ZVAL1   =                    0".into(),
            "ZNAME2  = 'SMOOTH  '".into(),
            format!("ZVAL2   = {smooth:20}"),
        ];
        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let image = Image::from_cards(&cards(&refs)).unwrap();
        let row = Row {
            place: Some(Place { stored: Stored::Compressed, count: data.len() as u64, offset: 0, elem: b'B' }),
            has_data_column: true,
            zscale: None,
            zzero: None,
            zblank: None,
            quantized: false,
        };
        tile(&image, 0, &row, data)
    }

    /// Bits written from the top of each byte down.
    #[derive(Default)]
    struct Writer {
        bytes: Vec<u8>,
        bits: usize,
    }

    impl Writer {
        fn put(&mut self, v: u64, n: u32) {
            for i in (0..n).rev() {
                if self.bits % 8 == 0 {
                    self.bytes.push(0);
                }
                let last = self.bytes.len() - 1;
                self.bytes[last] |= (((v >> i) & 1) as u8) << (7 - self.bits % 8);
                self.bits += 1;
            }
        }

        /// On to the next byte.
        fn align(&mut self) {
            self.bits = self.bytes.len() * 8;
        }
    }

    #[test]
    fn a_small_tile_worked_by_hand_comes_back() {
        let c = decode(&two_by_two(), 4).unwrap();
        assert_eq!((c.nx, c.ny, c.scale, c.sum, c.planes), (2, 2, 0, 400, [0, 1, 0]));
        assert_eq!((c.quadtree, c.direct, c.signs), (1, 1, 2));
        assert_eq!(c.values, [400, -1, 1, 0]);
        // The x difference rounds to 2 and the y difference to -2, and 400 is
        // shared out between the four pixels by them.
        let t = decoded(&two_by_two(), 2, 2, 0);
        assert_eq!((t.pixels, t.problem), (vec![100.0, 99.0, 101.0, 100.0], None));
        let whats: Vec<&str> = t.steps.iter().map(|s| s.what.as_str()).collect();
        assert_eq!(whats, ["HCOMPRESS_1", "H-transform"]);
    }

    #[test]
    fn the_smallest_power_of_two_is_worked_out_as_cfitsio_does() {
        let want = [(1, 0), (2, 1), (3, 2), (4, 2), (5, 3), (8, 3), (9, 4), (4096, 12), (4097, 13)];
        for (n, log2n) in want {
            assert_eq!(ceil_log2(n), log2n, "{n}");
        }
    }

    #[test]
    fn a_stream_cut_short_anywhere_is_refused() {
        let whole = two_by_two();
        for n in 0..whole.len() {
            let want = if n < HEADER { Broken::Short } else { Broken::RanOut };
            assert_eq!(decode(&whole[..n], 4).unwrap_err(), want, "{n} bytes");
            let t = decoded(&whole[..n], 2, 2, 0);
            assert!(t.pixels.is_empty() && t.problem.is_some(), "{n} bytes");
        }
    }

    #[test]
    fn a_broken_header_is_refused() {
        let mut bad = two_by_two();
        bad[1] = 0x98;
        assert_eq!(decode(&bad, 4).unwrap_err(), Broken::Magic([0xdd, 0x98]));
        assert_eq!(decode(&header(3, 2, 0, 0, [0; 3]), 4).unwrap_err(), Broken::Size { nx: 3, ny: 2 });
        assert_eq!(decode(&header(-2, -2, 0, 0, [0; 3]), 4).unwrap_err(), Broken::Size { nx: -2, ny: -2 });
        // 2^32 pixels, which a tile of none is not.
        assert_eq!(decode(&header(65536, 65536, 0, 0, [0; 3]), 0).unwrap_err(), Broken::Size { nx: 65536, ny: 65536 });
        assert_eq!(decode(&header(1, 1, 0, 0, [0; 3]), 1).unwrap_err(), Broken::TooSmall);
        assert_eq!(decode(&header(2, 2, 0, 0, [0, 64, 0]), 4).unwrap_err(), Broken::Planes(64));
    }

    #[test]
    fn a_bad_quadtree_code_or_end_mark_is_refused() {
        // Quadrant 2's plane opening with 0111 rather than 1111.
        let mut bad = two_by_two();
        bad[HEADER] = 0b0111_0110;
        assert_eq!(decode(&bad, 4).unwrap_err(), Broken::Code { quadrant: 1, plane: 0, code: 7 });
        // The four bits after the last plane reading 1000.
        let mut bad = two_by_two();
        bad[HEADER + 1] = 0b0001_0001;
        assert_eq!(decode(&bad, 4).unwrap_err(), Broken::End(8));
        let t = decoded(&bad, 2, 2, 0);
        assert!(t.pixels.is_empty() && t.problem.is_some());
    }

    #[test]
    fn a_coefficient_past_64_bits_is_refused_rather_than_wrapped() {
        let refused_at = |t: &Tile, step: &str| t.pixels.is_empty() && t.problem.as_deref().is_some_and(|p| p.starts_with(&format!("Stopped at {step}")));
        // A sum a scale of 3 takes past i64::MAX.
        let mut big = header(2, 2, 3, i64::MAX / 2, [0; 3]);
        big.push(0);
        let t = decoded(&big, 2, 2, 0);
        assert!(refused_at(&t, "undigitize"), "{:?}", t.problem);
        // A sum that rounding to a multiple of 4 takes past it.
        let mut big = header(2, 2, 0, i64::MAX, [0; 3]);
        big.push(0);
        let t = decoded(&big, 2, 2, 0);
        assert!(refused_at(&t, "H-transform"), "{:?}", t.problem);
        // Every coefficient of a tile large enough to smooth as large as 61
        // bit planes make it, written directly, at a scale of 2.
        let (nx, ny) = (9usize, 10usize);
        let mut b = header(nx as i32, ny as i32, 2, 1 << 61, [61; 3]);
        let mut w = Writer::default();
        for (qx, qy) in [(5usize, 5usize), (5, 5), (4, 5), (4, 5)] {
            for _ in 0..61 {
                w.put(0, 4);
                for _ in 0..qx.div_ceil(2) * qy.div_ceil(2) {
                    w.put(15, 4);
                }
            }
        }
        w.put(0, 4);
        w.align();
        for _ in 0..nx * ny {
            w.put(0, 1);
        }
        b.extend_from_slice(&w.bytes);
        let c = decode(&b, nx * ny).unwrap();
        assert!(c.values[1..].iter().all(|v| *v == (1 << 61) - 1));
        let t = decoded(&b, nx as u64, ny as u64, 1);
        assert!(refused_at(&t, "H-transform"), "{:?}", t.problem);
    }

    /// A random number generator for the two tests after, the same numbers
    /// every run.
    fn numbers(mut seed: u64) -> impl FnMut() -> u64 {
        move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        }
    }

    /// Streams from a valid header and random bytes after it, smoothed and
    /// not, at scales that lose and that do not. None may panic, and none may
    /// give a tile of the wrong number of pixels.
    #[test]
    fn random_bytes_after_a_header_never_panic() {
        let mut next = numbers(0x2545_f491_4f6c_dd1d);
        for round in 0..3000 {
            let nx = 1 + (next() % 13) as i32;
            let ny = 2 + (next() % 13) as i32;
            let planes = [(next() % 40) as u8, (next() % 40) as u8, (next() % 40) as u8];
            let scale = (next() % 40) as i32;
            let mut b = header(nx, ny, scale, (next() % 100_000) as i64 - 50_000, planes);
            let tail = (next() % 300) as usize;
            // Mostly the codes that open a plane, so the walk goes deep.
            for _ in 0..tail {
                b.push(match next() % 4 {
                    0 => 0xff,
                    1 => 0x0f,
                    _ => next() as u8,
                });
            }
            let t = decoded(&b, nx as u64, ny as u64, (round % 2) as i64);
            assert!(t.problem.is_some() || t.pixels.len() == (nx * ny) as usize, "round {round}");
        }
    }

    /// Streams of random coefficients, every bit plane written directly, of
    /// every shape up to 16 by 16, smoothed and not. Every one decodes, so the
    /// transform and the smoothing see odd sizes and every sign.
    #[test]
    fn random_coefficients_of_every_shape_come_back_as_pixels() {
        let mut next = numbers(0x9e37_79b9_7f4a_7c15);
        for round in 0..2000 {
            let (nx, ny) = (1 + (next() % 16) as usize, 2 + (next() % 15) as usize);
            let planes = [(next() % 41) as u8, (next() % 41) as u8, (next() % 41) as u8];
            let scale = (next() % 40) as i32;
            let mut b = header(nx as i32, ny as i32, scale, (next() % 100_000) as i64 - 50_000, planes);
            let (nx2, ny2) = (nx.div_ceil(2), ny.div_ceil(2));
            let mut w = Writer::default();
            for (q, (qx, qy)) in [(nx2, ny2), (nx2, ny / 2), (nx / 2, ny2), (nx / 2, ny / 2)].into_iter().enumerate() {
                for _ in 0..planes[[0, 1, 1, 2][q]] {
                    w.put(0, 4);
                    for _ in 0..qx.div_ceil(2) * qy.div_ceil(2) {
                        w.put(next() % 16, 4);
                    }
                }
            }
            w.put(0, 4);
            w.align();
            for _ in 0..nx * ny {
                w.put(next() & 1, 1);
            }
            b.extend_from_slice(&w.bytes);
            let t = decoded(&b, nx as u64, ny as u64, (round % 2) as i64);
            assert_eq!((t.problem.as_deref(), t.pixels.len()), (None, nx * ny), "round {round}");
        }
    }
}
