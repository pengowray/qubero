//! RICE_1: a tile's bits taken apart into its integers. See the module doc of
//! [`fits_tile`](super) for how the coding works.
//!
//! Apart from [`fits_tile`](super) because a bit decoder is a job of its own:
//! nothing here knows about cards, rows or quantization, and the tile only
//! hands it the bytes, how many pixels to make, and the two parameters.

use super::{commas, pixel_word, plural, stopped, Image, Step, Tile, Values};
use crate::bits::Bits;

/// Rice: the bits taken apart into the tile's integers.
pub(super) fn step(tile: &mut Tile, image: &Image, data: &[u8], pixels: usize) -> Option<Values> {
    let name = image.algorithm.clone();
    let bytepix = image.bytepix as usize;
    if !matches!(bytepix, 1 | 2 | 4) {
        stopped(tile, &name, data.len());
        tile.problem = Some(format!(
            "Stopped at {name}: BYTEPIX is {bytepix}, and this viewer's Rice decoder reads 1, 2 or 4 bytes per pixel."
        ));
        return None;
    }
    let blocksize = image.blocksize as usize;
    if blocksize == 0 {
        stopped(tile, &name, data.len());
        tile.problem = Some(format!("Stopped at {name}: BLOCKSIZE is 0, and a block must hold at least 1 pixel."));
        return None;
    }
    let rice = rice(data, pixels, bytepix, blocksize);
    let made = rice.values.len();
    tile.decoded_bytes = made * bytepix;
    let blocks = rice.coded + rice.zero + rice.whole;
    tile.steps.push(Step {
        what: name.clone(),
        in_bytes: data.len(),
        out_bytes: made * bytepix,
        note: format!(
            "{} per pixel; first pixel stored whole, then {} of {}: {} Rice-coded, {} all-zero, {} uncoded at {} bits per difference",
            plural(bytepix as u64, "byte", "bytes"),
            plural(blocks as u64, "block", "blocks"),
            pixel_word(blocksize as u64),
            commas(rice.coded as u64),
            commas(rice.zero as u64),
            commas(rice.whole as u64),
            bytepix * 8
        ),
    });
    if let Some(code) = rice.bad_code {
        let largest = [0, 7, 15, 0, 26][bytepix];
        tile.problem = Some(format!(
            "Stopped at {name}: block code {code} after {} of {}; the largest for {bytepix}-byte pixels is {largest}.",
            commas(made as u64),
            pixel_word(pixels as u64)
        ));
    } else if made < pixels {
        tile.problem = Some(format!("Stopped at {name}: the data ran out after {} of {}.", commas(made as u64), pixel_word(pixels as u64)));
    }
    Some(Values::Ints(rice.values))
}

/// What a Rice tile came to, and how its blocks were written.
struct Rice {
    values: Vec<i64>,
    coded: usize,
    zero: usize,
    whole: usize,
    /// A block code past the largest the width has, where one stopped the
    /// walk.
    bad_code: Option<u64>,
}

/// Rice decoding, as the convention describes it. See the module doc of
/// [`fits_tile`](super).
fn rice(data: &[u8], pixels: usize, bytepix: usize, blocksize: usize) -> Rice {
    let width = 8 * bytepix as u32;
    let (code_bits, largest) = match bytepix {
        1 => (3, 7),
        2 => (4, 15),
        _ => (5, 26),
    };
    let mask: u64 = if width == 64 { u64::MAX } else { (1 << width) - 1 };
    let mut out = Rice { values: Vec::with_capacity(pixels), coded: 0, zero: 0, whole: 0, bad_code: None };
    if pixels == 0 {
        return out;
    }
    // Read from the top of each byte down.
    let mut bits = Bits::new(data);
    let Some(first) = bits.take(width) else { return out };
    let mut last = first;
    let signed = |v: u64| -> i64 {
        if bytepix == 1 {
            v as i64
        } else {
            let shift = 64 - width;
            ((v << shift) as i64) >> shift
        }
    };
    while out.values.len() < pixels {
        let Some(code) = bits.take(code_bits) else { return out };
        let n = blocksize.min(pixels - out.values.len());
        if code == 0 {
            out.zero += 1;
            out.values.extend(std::iter::repeat_n(signed(last), n));
            continue;
        }
        if code > largest {
            out.bad_code = Some(code);
            return out;
        }
        let whole = code == largest;
        if whole {
            out.whole += 1;
        } else {
            out.coded += 1;
        }
        let low = (code - 1) as u32;
        for _ in 0..n {
            let folded = if whole {
                let Some(v) = bits.take(width) else { return out };
                v
            } else {
                let Some(high) = bits.unary() else { return out };
                let Some(bottom) = bits.take(low) else { return out };
                (high << low) | bottom
            };
            // 0, 1, 2, 3 back to 0, -1, 1, -2.
            let difference = if folded & 1 == 0 { folded >> 1 } else { !(folded >> 1) };
            last = last.wrapping_add(difference) & mask;
            out.values.push(signed(last));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bits written from the top of each byte down, the way Rice reads them.
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
                let bit = (v >> i) & 1;
                let last = self.bytes.len() - 1;
                self.bytes[last] |= (bit as u8) << (7 - self.bits % 8);
                self.bits += 1;
            }
        }
    }

    /// A Rice encoder written from the description, for the decoder to be
    /// checked against. It picks its own code per block: all zero where every
    /// difference is, whole where `whole` says, and otherwise `k` low bits.
    fn rice_encode(pixels: &[i64], bytepix: usize, blocksize: usize, pick: impl Fn(usize, &[u64]) -> Option<u32>) -> Vec<u8> {
        let width = 8 * bytepix as u32;
        let (code_bits, largest) = match bytepix {
            1 => (3, 7),
            2 => (4, 15),
            _ => (5, 26),
        };
        let mask: u64 = (1u64 << width) - 1;
        let mut w = Writer::default();
        w.put(pixels[0] as u64 & mask, width);
        let mut last = pixels[0] as u64 & mask;
        let folded: Vec<u64> = pixels
            .iter()
            .map(|p| {
                let now = *p as u64 & mask;
                let d = now.wrapping_sub(last) & mask;
                last = now;
                // Sign-extend the difference at the pixel's width, then fold.
                let shift = 64 - width;
                let d = ((d << shift) as i64) >> shift;
                if d >= 0 { (d as u64) << 1 } else { (!(d as u64) << 1) | 1 }
            })
            .collect();
        for (b, block) in folded.chunks(blocksize).enumerate() {
            if block.iter().all(|f| *f == 0) {
                w.put(0, code_bits);
                continue;
            }
            match pick(b, block) {
                None => {
                    w.put(largest, code_bits);
                    for f in block {
                        w.put(*f, width);
                    }
                }
                Some(k) => {
                    w.put(u64::from(k) + 1, code_bits);
                    for f in block {
                        for _ in 0..(f >> k) {
                            w.put(0, 1);
                        }
                        w.put(1, 1);
                        w.put(f & ((1 << k) - 1), k);
                    }
                }
            }
        }
        w.bytes
    }

    #[test]
    fn rice_reads_every_kind_of_block_at_every_width() {
        for bytepix in [1usize, 2, 4] {
            let span = if bytepix == 1 { 255 } else if bytepix == 2 { 60_000 } else { 4_000_000_000 };
            let mut pixels = Vec::new();
            for i in 0..100i64 {
                // A ramp, then a flat run, then noise across the whole range.
                let v = match i {
                    0..=40 => 100 + i * 3,
                    41..=72 => 50,
                    _ => ((i * 7919) % span) - if bytepix == 1 { 0 } else { span / 2 },
                };
                pixels.push(v);
            }
            // As few low bits as keep the unary part short, and never so many
            // that the code would be the one that says whole.
            let most = [0, 5, 13, 0, 24][bytepix];
            let data = rice_encode(&pixels, bytepix, 16, |b, block| {
                let top = block.iter().max().map_or(0, |m| 64 - m.leading_zeros());
                if b == 5 { None } else { Some(top.saturating_sub(2).min(most)) }
            });
            let r = rice(&data, pixels.len(), bytepix, 16);
            let want: Vec<i64> = pixels
                .iter()
                .map(|p| match bytepix {
                    1 => i64::from(*p as u8),
                    2 => i64::from(*p as i16),
                    _ => i64::from(*p as i32),
                })
                .collect();
            assert_eq!(r.values, want, "bytepix {bytepix}");
            assert!(r.zero >= 1 && r.whole == 1 && r.coded >= 1, "bytepix {bytepix}");
        }
    }

    #[test]
    fn a_code_past_the_largest_stops_the_walk() {
        // A 4-byte pixel of 7, then the code 27, which five bits hold and no
        // block may have.
        let mut w = Writer::default();
        w.put(7, 32);
        w.put(27, 5);
        w.put(0, 3);
        let r = rice(&w.bytes, 4, 4, 32);
        assert_eq!((r.values.len(), r.bad_code), (0, Some(27)));
    }

    #[test]
    fn rice_that_runs_out_of_bits_stops_where_it_did() {
        let pixels: Vec<i64> = (0..64).map(|i| i * 5).collect();
        let data = rice_encode(&pixels, 4, 32, |_, _| Some(4));
        let r = rice(&data[..data.len() / 2], pixels.len(), 4, 32);
        assert!(!r.values.is_empty() && r.values.len() < 64);
        assert_eq!(r.values[..], pixels[..r.values.len()]);
    }
}
