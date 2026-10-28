//! What colour a structure of samples is. See [`crate::template::Colour`].

use super::*;

impl Evaluator {
    /// The colour the structure at `path` says it is, as CSS writes one:
    /// `#rrggbb`, or `#rrggbbaa` where it has an alpha. Every sample is
    /// scaled from its own range to eight bits, rounding to the nearest.
    ///
    /// None for a structure that says it is no colour, and for one whose
    /// samples are not all there, not numbers, or larger than their range
    /// allows. A tRNS grey of 300 in a four-bit image is a file that is wrong,
    /// and a swatch of the nearest colour that fits would say it is not.
    pub(super) fn colour_of<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Option<String>> {
        let Some(def) = self.struct_of(&self.memo[path].ty.clone()) else { return Ok(None) };
        let Some(colour) = def.colour.clone() else { return Ok(None) };
        let depth = match &colour.depth {
            Some(e) => match self.eval_expr(doc, path, e) {
                Ok(bits) => Some(bits),
                Err(EvalError::Failed(_)) => return Ok(None),
                Err(e) => return Err(e),
            },
            None => None,
        };
        let mut samples = Vec::with_capacity(4);
        for name in colour.channels.iter().chain(colour.alpha.iter()) {
            let Some(child) = self.child_named(doc, path, name)? else { return Ok(None) };
            let field = match self.value_of(doc, &child) {
                Ok(field) => field,
                Err(EvalError::Failed(_)) => return Ok(None),
                Err(e) => return Err(e),
            };
            if field.absent {
                return Ok(None);
            }
            let Some(v) = field.value.as_int() else { return Ok(None) };
            let Some(byte) = to_byte(v, depth.unwrap_or(field.size_bits as i128)) else { return Ok(None) };
            samples.push(byte);
        }
        // A grey is the same level in all three, and its alpha, if any, after.
        if colour.channels.len() == 1 {
            let grey = samples[0];
            samples.splice(0..0, [grey, grey]);
        }
        Ok(Some(samples.iter().fold(String::from("#"), |mut s, b| {
            let _ = std::fmt::Write::write_fmt(&mut s, format_args!("{b:02x}"));
            s
        })))
    }
}

/// A sample of `bits` bits as the eight-bit level it stands for: nought is
/// nought and the largest the bits hold is 255, whatever the width. None for a
/// sample past its range, or a width no colour is written in.
fn to_byte(v: i128, bits: i128) -> Option<u8> {
    if !(1..=32).contains(&bits) || v < 0 {
        return None;
    }
    let max = (1i128 << bits) - 1;
    if v > max {
        return None;
    }
    u8::try_from((v * 255 + max / 2) / max).ok()
}

#[cfg(test)]
mod tests {
    use super::to_byte;

    #[test]
    fn every_width_reaches_white() {
        for bits in [1, 2, 4, 8, 16] {
            assert_eq!(to_byte(0, bits), Some(0));
            assert_eq!(to_byte((1 << bits) - 1, bits), Some(255));
        }
        // A four-bit grey of 3 is a fifth of the way up, not nearly black.
        assert_eq!(to_byte(3, 4), Some(51));
        assert_eq!(to_byte(0x8000, 16), Some(128));
        assert_eq!(to_byte(16, 4), None);
        assert_eq!(to_byte(1, 0), None);
    }
}
