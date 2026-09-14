//! Quantized floats: a tile's integers turned back into the floats they were
//! made from, with a scale, a zero point, and the standard's fixed sequence of
//! random numbers for a dithered tile. See the module doc of
//! [`fits_tile`](super) for the arithmetic.
//!
//! Apart from [`fits_tile`](super) because it comes after every algorithm
//! alike: whatever made the integers, Rice or gzip or none, they are
//! unquantized the same way, and nothing here knows how they were stored.

use std::sync::OnceLock;

use super::{pixel_word, stopped, Image, Row, Step, Tile, Values};

/// How many random numbers the dithering sequence has.
const N_RANDOM: usize = 10_000;

/// The stored value `SUBTRACTIVE_DITHER_2` writes for a pixel that was
/// exactly zero.
const ZERO_VALUE: i64 = -2_147_483_646;

/// The standard's random numbers, made once.
pub fn randoms() -> &'static [f32] {
    static TABLE: OnceLock<Vec<f32>> = OnceLock::new();
    TABLE.get_or_init(|| seeds().map(|seed| (seed / 2147483647.0) as f32).collect())
}

/// The generator's seeds, in order, the way the standard writes it out: in
/// 64-bit floats, which hold every product exactly.
fn seeds() -> impl Iterator<Item = f64> {
    let a = 16807.0f64;
    let m = 2147483647.0f64;
    let mut seed = 1.0f64;
    (0..N_RANDOM).map(move |_| {
        let t = a * seed;
        seed = t - m * (t / m).trunc();
        seed
    })
}

/// The tile's integers turned back into the floats they were quantized from.
pub(super) fn unquantize(tile: &mut Tile, image: &Image, row: &Row, index: u64, values: &Values) -> Vec<f64> {
    let ints: &[i64] = match values {
        Values::Ints(v) => v,
        Values::Floats(_) => return Vec::new(),
    };
    let scale = row.zscale.unwrap_or(1.0);
    let zero = row.zzero.unwrap_or(0.0);
    let f32_image = image.zbitpix == -32;
    let keep = |v: f64| if f32_image { f64::from(v as f32) } else { v };
    let method = if image.quantize.is_empty() { "NO_DITHER" } else { image.quantize.as_str() };
    let dither = match method {
        "NO_DITHER" => None,
        "SUBTRACTIVE_DITHER_1" => Some(false),
        "SUBTRACTIVE_DITHER_2" => Some(true),
        other => {
            stopped(tile, other, 0);
            tile.problem = Some(format!(
                "Stopped at {other}: ZQUANTIZ names a method this viewer does not know, so the integers could not be turned back into floats."
            ));
            return Vec::new();
        }
    };
    // The zero point with its sign as the operator, so that a negative one
    // reads as a subtraction rather than as `+ -45.6`.
    let plus = if zero.is_sign_negative() { format!("\u{2212} {}", -zero) } else { format!("+ {zero}") };
    let keywords = "(ZSCALE and ZZERO for this tile)";
    let mut blanks = 0usize;
    let mut zeros = 0usize;
    let mut out = Vec::with_capacity(ints.len());
    let Some(two) = dither else {
        tile.steps.push(Step { what: method.into(), in_bytes: 0, out_bytes: 0, note: format!("float = integer × {scale} {plus} {keywords}") });
        for &q in ints {
            if Some(q) == row.zblank {
                blanks += 1;
                out.push(f64::NAN);
            } else {
                out.push(keep(q as f64 * scale + zero));
            }
        }
        blank_rows(tile, row, blanks, None);
        return out;
    };
    tile.steps.push(Step {
        what: method.into(),
        in_bytes: 0,
        out_bytes: 0,
        note: format!("float = (integer \u{2212} r + 0.5) × {scale} {plus} {keywords}"),
    });
    let Some(dither0) = image.dither0 else {
        tile.problem = Some(format!("Stopped at {method}: this dither needs a ZDITHER0 keyword, and the header has none."));
        return Vec::new();
    };
    let rand = randoms();
    // Summed wider than an i64, since ZDITHER0 can be any i64.
    let mut iseed = (i128::from(index) + i128::from(dither0) - 1).rem_euclid(N_RANDOM as i128) as usize;
    let mut next = (rand[iseed] * 500.0) as usize;
    tile.steps.push(Step {
        what: "dither".into(),
        in_bytes: 0,
        out_bytes: 0,
        note: format!("r from the standard's fixed sequence of 10,000 random numbers; ZDITHER0 = {dither0}, this tile starting at r[{next}]"),
    });
    for &q in ints {
        if Some(q) == row.zblank {
            blanks += 1;
            out.push(f64::NAN);
        } else if two && q == ZERO_VALUE {
            zeros += 1;
            out.push(0.0);
        } else {
            out.push(keep((q as f64 - f64::from(rand[next]) + 0.5) * scale + zero));
        }
        next += 1;
        if next == N_RANDOM {
            iseed = (iseed + 1) % N_RANDOM;
            next = (rand[iseed] * 500.0) as usize;
        }
    }
    blank_rows(tile, row, blanks, two.then_some(zeros));
    out
}

/// A row for the pixels that were blank, and for the method that keeps zeros
/// exact, one for the pixels that were zero. Neither when there were none.
fn blank_rows(tile: &mut Tile, row: &Row, blanks: usize, zeros: Option<usize>) {
    if let (true, Some(v)) = (blanks > 0, row.zblank) {
        let n = blanks as u64;
        let note = format!("{} stored as ZBLANK = {v}, which means blank, read as NaN", pixel_word(n));
        tile.steps.push(Step { what: "blank".into(), in_bytes: 0, out_bytes: 0, note });
    }
    if let Some(z) = zeros.filter(|z| *z > 0) {
        let note = format!("{} stored as {ZERO_VALUE}, which SUBTRACTIVE_DITHER_2 reserves for exactly 0.0", pixel_word(z as u64));
        tile.steps.push(Step { what: "zero".into(), in_bytes: 0, out_bytes: 0, note });
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::cards;
    use super::super::{decode, Place, Stored};
    use super::*;

    #[test]
    fn the_random_numbers_are_the_standards_sequence() {
        // The standard's own check: the 10,000th seed is 1043618065.
        assert_eq!(seeds().last(), Some(1043618065.0));
        let r = randoms();
        assert_eq!(r.len(), 10_000);
        assert_eq!(r[0], (16807.0 / 2147483647.0) as f32);
    }

    /// `ZDITHER0` can be any 64-bit number, and the first random number a tile
    /// takes is counted from it and from the tile's number, round the sequence.
    #[test]
    fn the_dither_seed_wraps_round_the_sequence_whatever_it_is() {
        let r = randoms();
        let want = |iseed: usize| f64::from(((4.0 - f64::from(r[(r[iseed] * 500.0) as usize]) + 0.5) * 0.5 + 10.0) as f32);
        // i64::MAX is 5807 more than a multiple of 10,000; one less than
        // i64::MIN is 4191 more.
        assert_eq!(quantized("SUBTRACTIVE_DITHER_1", &[4], i64::MAX, 1).pixels, [want(5807)]);
        assert_eq!(quantized("SUBTRACTIVE_DITHER_1", &[4], i64::MIN, 0).pixels, [want(4191)]);
    }

    /// A one-row image of `pixels` quantized with this method, from a row
    /// whose scale and zero point are 0.5 and 10.
    fn quantized(method: &str, ints: &[i32], dither0: i64, index: u64) -> Tile {
        let mut lines = vec![
            "ZBITPIX =                  -32".to_string(),
            "ZNAXIS  =                    1".to_string(),
            format!("ZNAXIS1 = {:20}", ints.len()),
            "ZCMPTYPE= 'NOCOMPRESS'".to_string(),
            format!("ZDITHER0= {dither0:20}"),
            "ZSCALE  =                  0.5".to_string(),
            "ZZERO   =                 10.0".to_string(),
            "ZBLANK  =          -2147483648".to_string(),
        ];
        if !method.is_empty() {
            lines.push(format!("ZQUANTIZ= '{method}'"));
        }
        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let image = Image::from_cards(&cards(&refs)).unwrap();
        let mut data = Vec::new();
        for i in ints {
            data.extend_from_slice(&i.to_be_bytes());
        }
        let row = Row {
            place: Some(Place { stored: Stored::Compressed, count: data.len() as u64, offset: 0, elem: b'B' }),
            has_data_column: true,
            zscale: image.zscale,
            zzero: image.zzero,
            zblank: image.zblank,
            quantized: true,
        };
        decode(&image, index, &row, &data)
    }

    #[test]
    fn no_dither_is_a_scale_and_a_zero_point() {
        let t = quantized("", &[0, 4, i32::MIN], 1, 0);
        assert_eq!(t.pixels[..2], [10.0, 12.0]);
        assert!(t.pixels[2].is_nan());
        let whats: Vec<&str> = t.steps.iter().map(|s| s.what.as_str()).collect();
        assert_eq!(whats, ["stored", "read", "NO_DITHER", "blank"]);
        assert_eq!(t.steps[2].note, "float = integer × 0.5 + 10 (ZSCALE and ZZERO for this tile)");
        assert_eq!(t.steps[3].note, "1 pixel stored as ZBLANK = -2147483648, which means blank, read as NaN");
    }

    #[test]
    fn dithering_takes_its_random_numbers_from_where_the_tile_and_the_seed_say() {
        let r = randoms();
        // Tile 2 with ZDITHER0 5 starts at iseed 6.
        let t = quantized("SUBTRACTIVE_DITHER_1", &[4, 4, -2147483646], 5, 2);
        let start = (r[6] * 500.0) as usize;
        let want = |q: f64, k: usize| f64::from(((q - f64::from(r[start + k]) + 0.5) * 0.5 + 10.0) as f32);
        assert_eq!(t.pixels[0], want(4.0, 0));
        assert_eq!(t.pixels[1], want(4.0, 1));
        // Only the second method keeps a zero.
        assert_eq!(t.pixels[2], want(-2147483646.0, 2));
        let t = quantized("SUBTRACTIVE_DITHER_2", &[4, -2147483646, 4], 5, 2);
        assert_eq!((t.pixels[1], t.pixels[2]), (0.0, want(4.0, 2)));
    }

    #[test]
    fn the_random_numbers_wrap_to_the_next_seed_when_they_run_out() {
        let r = randoms();
        // A tile long enough to run off the end of the sequence.
        let start = (r[0] * 500.0) as usize;
        let n = N_RANDOM - start + 3;
        let ints = vec![0i32; n];
        let t = quantized("SUBTRACTIVE_DITHER_1", &ints, 1, 0);
        let restart = (r[1] * 500.0) as usize;
        let want = |k: usize| f64::from(((0.0 - f64::from(r[k]) + 0.5) * 0.5 + 10.0) as f32);
        assert_eq!(t.pixels[N_RANDOM - start - 1], want(N_RANDOM - 1));
        assert_eq!(t.pixels[N_RANDOM - start], want(restart));
    }
}
