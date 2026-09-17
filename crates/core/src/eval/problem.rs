//! What is wrong with a value, said in one line.
//!
//! The words are built here rather than in each view, so the listing, the hex
//! chip's tooltip, the inspector and the value table say the same thing about
//! the same bytes. Two tiers: [`Tier::Invalid`] where the format itself rules
//! the value out, and [`Tier::Undefined`] where Qubero merely has no name for
//! it. See `docs/DESIGN-wrong-values.md`.

use crate::eval::{Problem, Tier, Value};
use crate::template::Ty;

/// How a float's bits are laid out: the exponent and fraction widths, whether
/// the leading one is written out rather than assumed (x87 alone), and whether
/// the top exponent holds infinities (every form but `e4m3`, which spends it
/// on numbers and keeps one pattern back for NaN).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Layout {
    pub(crate) width: u32,
    pub(crate) exp: u32,
    pub(crate) sig: u32,
    pub(crate) explicit: bool,
    pub(crate) infinities: bool,
}

/// The double the rest of Qubero carries a float as, for a value that reached
/// here without bits of its own: a computed real, a JSON number.
pub(crate) const BINARY64: Layout = Layout { width: 64, exp: 11, sig: 52, explicit: false, infinities: true };

/// Which layout a field's bits are in, for the float types that have one.
/// None for a field whose value is a float without a stored pattern, and for
/// IBM hexadecimal, which has no NaN and no infinity to name.
pub(crate) fn layout(ty: &Ty) -> Option<Layout> {
    Some(match ty.without_sentinel() {
        Ty::F16(_) => Layout { width: 16, exp: 5, sig: 10, explicit: false, infinities: true },
        Ty::BF16(_) => Layout { width: 16, exp: 8, sig: 7, explicit: false, infinities: true },
        Ty::F32(_) => Layout { width: 32, exp: 8, sig: 23, explicit: false, infinities: true },
        Ty::F64(_) => BINARY64,
        Ty::F80(_) => Layout { width: 80, exp: 15, sig: 64, explicit: true, infinities: true },
        Ty::F8 { e4m3 } => {
            if *e4m3 {
                Layout { width: 8, exp: 4, sig: 3, explicit: false, infinities: false }
            } else {
                Layout { width: 8, exp: 5, sig: 2, explicit: false, infinities: true }
            }
        }
        _ => return None,
    })
}

/// What is wrong with this value, when something is. `float` is the field's
/// own bit pattern and layout, for a float whose bytes are in the file: a
/// signalling NaN read as an f32 and widened to a double comes back quiet, so
/// the classification is made on the bits the file holds rather than on the
/// double they were read into.
pub(super) fn of(value: &Value, ty: &Ty, float: Option<(u128, Layout)>) -> Option<Problem> {
    match value {
        Value::Magic { ok: false, expected, .. } => Some(Problem {
            tier: Tier::Invalid,
            text: format!("Does not match: expected {}", crate::text::c_string(expected)),
        }),
        Value::Enum { name: None, .. } => Some(Problem {
            tier: Tier::Undefined,
            text: format!("Undefined in {}", enum_name(ty).unwrap_or("this enum")),
        }),
        Value::Flags { unnamed, .. } if *unnamed > 0 => Some(Problem {
            tier: Tier::Undefined,
            text: if *unnamed == 1 { "1 unnamed bit set".to_string() } else { format!("{unnamed} unnamed bits set") },
        }),
        Value::Float(v) if !v.is_finite() => {
            let (bits, l) = float.unwrap_or_else(|| (u128::from(v.to_bits()), BINARY64));
            float_text(bits, l).map(|text| Problem { tier: Tier::Undefined, text })
        }
        _ => None,
    }
}

/// The enum's own name, for a field that is one. `base` looks through an enum
/// to the integer under it, so this does not use it.
fn enum_name(ty: &Ty) -> Option<&str> {
    match ty {
        Ty::Enum { def, .. } => Some(&def.name),
        Ty::Nullable { inner, .. } => enum_name(inner),
        _ => None,
    }
}

/// What a bit pattern that is not a number reads as: the same classification
/// the type panel's bit layout draws, in one line. None for a pattern that is
/// an ordinary number, which is every pattern whose exponent is not all ones.
pub(crate) fn float_text(bits: u128, l: Layout) -> Option<String> {
    let top = (1u128 << l.exp) - 1;
    let stored = (bits >> l.sig) & top;
    if stored != top {
        return None;
    }
    // x87 writes the leading one out as the top bit of its significand, so
    // what the other layouts call the fraction is the bits below it.
    let frac_bits = if l.explicit { l.sig - 1 } else { l.sig };
    let frac = bits & ((1u128 << frac_bits) - 1);
    let negative = (bits >> (l.width - 1)) & 1 == 1;
    if !l.infinities {
        // Every pattern under the top exponent is a number here except the one
        // this layout keeps back.
        return (frac == (1u128 << frac_bits) - 1).then(|| "Not a number (quiet NaN)".to_string());
    }
    if frac == 0 {
        return Some(if negative { "Negative infinity".to_string() } else { "Infinity".to_string() });
    }
    let quiet_bit = 1u128 << (frac_bits - 1);
    let quiet = frac & quiet_bit != 0;
    let payload = frac & !quiet_bit;
    Some(match (quiet, payload) {
        (true, 0) => "Not a number (quiet NaN)".to_string(),
        (true, p) => format!("Not a number (quiet NaN, payload 0x{p:x})"),
        (false, p) => format!("Not a number (signalling NaN, payload 0x{p:x})"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const F32: Layout = Layout { width: 32, exp: 8, sig: 23, explicit: false, infinities: true };
    const F16: Layout = Layout { width: 16, exp: 5, sig: 10, explicit: false, infinities: true };
    const E4M3: Layout = Layout { width: 8, exp: 4, sig: 3, explicit: false, infinities: false };
    const X87: Layout = Layout { width: 80, exp: 15, sig: 64, explicit: true, infinities: true };

    fn f32_text(bits: u32) -> Option<String> {
        float_text(u128::from(bits), F32)
    }

    #[test]
    fn ordinary_numbers_say_nothing() {
        assert_eq!(f32_text(0), None);
        assert_eq!(f32_text(1.5f32.to_bits()), None);
        assert_eq!(float_text(u128::from(1.0f64.to_bits()), BINARY64), None);
        // A subnormal is a number, not a problem.
        assert_eq!(f32_text(1), None);
    }

    #[test]
    fn infinities_are_named_by_sign() {
        assert_eq!(f32_text(0x7f80_0000).as_deref(), Some("Infinity"));
        assert_eq!(f32_text(0xff80_0000).as_deref(), Some("Negative infinity"));
        assert_eq!(float_text(u128::from(f64::INFINITY.to_bits()), BINARY64).as_deref(), Some("Infinity"));
        assert_eq!(float_text(u128::from(f64::NEG_INFINITY.to_bits()), BINARY64).as_deref(), Some("Negative infinity"));
    }

    #[test]
    fn the_canonical_quiet_nan_has_no_payload() {
        assert_eq!(f32_text(0x7fc0_0000).as_deref(), Some("Not a number (quiet NaN)"));
        assert_eq!(float_text(u128::from(f64::NAN.to_bits()), BINARY64).as_deref(), Some("Not a number (quiet NaN)"));
    }

    #[test]
    fn a_payload_beyond_the_quiet_bit_is_worth_saying() {
        assert_eq!(f32_text(0x7fc0_0001).as_deref(), Some("Not a number (quiet NaN, payload 0x1)"));
        assert_eq!(f32_text(0x7fd0_0000).as_deref(), Some("Not a number (quiet NaN, payload 0x100000)"));
    }

    #[test]
    fn a_signalling_nan_keeps_the_quiet_bit_clear() {
        assert_eq!(f32_text(0x7f80_0001).as_deref(), Some("Not a number (signalling NaN, payload 0x1)"));
        assert_eq!(f32_text(0xff80_00ff).as_deref(), Some("Not a number (signalling NaN, payload 0xff)"));
    }

    #[test]
    fn half_precision_reads_by_its_own_widths() {
        assert_eq!(float_text(0x7c00, F16).as_deref(), Some("Infinity"));
        assert_eq!(float_text(0xfc00, F16).as_deref(), Some("Negative infinity"));
        assert_eq!(float_text(0x7e00, F16).as_deref(), Some("Not a number (quiet NaN)"));
        assert_eq!(float_text(0x7c01, F16).as_deref(), Some("Not a number (signalling NaN, payload 0x1)"));
        assert_eq!(float_text(0x7bff, F16), None);
    }

    #[test]
    fn e4m3_spends_its_top_exponent_on_numbers() {
        // 0x78 is 1111 000: the top exponent with an empty fraction, which
        // this layout reads as a number rather than as an infinity.
        assert_eq!(float_text(0x78, E4M3), None);
        assert_eq!(float_text(0x7f, E4M3).as_deref(), Some("Not a number (quiet NaN)"));
        assert_eq!(float_text(0xff, E4M3).as_deref(), Some("Not a number (quiet NaN)"));
    }

    #[test]
    fn x87_counts_its_fraction_under_the_written_leading_one() {
        // Sign 0, exponent all ones, integer bit set, fraction 0.
        assert_eq!(float_text(0x7fff_8000_0000_0000_0000, X87).as_deref(), Some("Infinity"));
        assert_eq!(float_text(0xffff_8000_0000_0000_0000, X87).as_deref(), Some("Negative infinity"));
        // Integer bit set, quiet bit set: the usual quiet NaN.
        assert_eq!(float_text(0x7fff_c000_0000_0000_0000, X87).as_deref(), Some("Not a number (quiet NaN)"));
        assert_eq!(float_text(0x7fff_8000_0000_0000_0001, X87).as_deref(), Some("Not a number (signalling NaN, payload 0x1)"));
    }

    #[test]
    fn magic_says_what_was_wanted() {
        let v = Value::Magic { ok: false, bytes: b"PK\x03\x04".to_vec(), expected: b"\x89PNG".to_vec() };
        let p = of(&v, &Ty::u8(), None).expect("a problem");
        assert_eq!(p.tier, Tier::Invalid);
        assert_eq!(p.text, format!("Does not match: expected {}", crate::text::c_string(b"\x89PNG")));
    }

    #[test]
    fn a_matching_magic_is_not_a_problem() {
        let v = Value::Magic { ok: true, bytes: b"\x89PNG".to_vec(), expected: b"\x89PNG".to_vec() };
        assert_eq!(of(&v, &Ty::u8(), None), None);
    }

    #[test]
    fn unnamed_bits_are_counted_in_words() {
        let one = Value::Flags { raw: 1, set: Vec::new(), unnamed: 1 };
        assert_eq!(of(&one, &Ty::u8(), None).expect("a problem").text, "1 unnamed bit set");
        let three = Value::Flags { raw: 7, set: Vec::new(), unnamed: 3 };
        let p = of(&three, &Ty::u8(), None).expect("a problem");
        assert_eq!(p.tier, Tier::Undefined);
        assert_eq!(p.text, "3 unnamed bits set");
        let none = Value::Flags { raw: 0, set: Vec::new(), unnamed: 0 };
        assert_eq!(of(&none, &Ty::u8(), None), None);
    }
}
