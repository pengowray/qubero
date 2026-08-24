//! Reading typed values out of packed bits: the half that `encode.rs` inverts.
//!
//! Nothing here touches the document or the source. It takes the bits a field
//! occupies (MSB-first, as `Document::read_bits` produces them) and turns them
//! into a number, plus the static size questions the evaluator asks of a type.

use crate::bits::bytes_for;
use crate::template::{Endian, Expr, StrLen, Ty};

/// A short text or byte field used in an expression is its bytes as a
/// big-endian number, so a switch can key on e.g. "IHDR".
pub(crate) fn be_int(b: &[u8]) -> i128 {
    b.iter().fold(0i128, |acc, &x| (acc << 8) | x as i128)
}

/// Size in bits if it does not depend on data.
pub fn fixed_bits(ty: &Ty) -> Option<u64> {
    Some(match ty {
        Ty::UInt { bits, .. } | Ty::Int { bits, .. } => *bits as u64,
        Ty::F16(_) | Ty::BF16(_) => 16,
        Ty::Fixed { bits, .. } => *bits as u64,
        Ty::F32(_) => 32,
        Ty::F64(_) => 64,
        Ty::Magic(b) => b.len() as u64 * 8,
        Ty::Bytes(Expr::Lit(n)) => (*n).max(0) as u64 * 8,
        // Text is fixed-size only when its length does not depend on the bytes.
        Ty::Str { len: StrLen::Fixed(Expr::Lit(n)), .. } | Ty::Str { len: StrLen::Padded { size: Expr::Lit(n), .. }, .. } => {
            (*n).max(0) as u64 * 8
        }
        Ty::Struct(s) => s.fields.iter().map(|f| fixed_bits(&f.ty)).sum::<Option<u64>>()?,
        Ty::Array { elem, count: Expr::Lit(n) } => fixed_bits(elem)? * (*n).max(0) as u64,
        Ty::Sized { size: Expr::Lit(n), .. } => (*n).max(0) as u64 * 8,
        Ty::Enum { inner, .. } | Ty::Flags { inner, .. } => fixed_bits(inner)?,
        // A computed field is a value and no bits.
        Ty::Computed(_) => 0,
        // A named type could be anything, including itself.
        Ty::Named(_) => return None,
        _ => return None,
    })
}

/// Interpret a two's-complement integer of `bits` bits.
pub(crate) fn read_int(buf: &[u8], bits: u32, endian: Endian) -> i128 {
    let u = read_uint(buf, bits, endian);
    if bits < 128 && (u >> (bits - 1)) & 1 == 1 {
        u as i128 - (1i128 << bits)
    } else {
        u as i128
    }
}

/// Interpret `bits` bits (MSB-first packed in `buf`) as an unsigned integer.
/// Little-endian applies byte order for whole-byte widths; narrower fields read
/// as packed big-endian bit strings.
pub(crate) fn read_uint(buf: &[u8], bits: u32, endian: Endian) -> u128 {
    let nbytes = bytes_for(bits as u64);
    let mut v: u128 = 0;
    if endian == Endian::Little && bits % 8 == 0 {
        for i in (0..nbytes).rev() {
            v = (v << 8) | buf[i] as u128;
        }
    } else {
        for &b in &buf[..nbytes] {
            v = (v << 8) | b as u128;
        }
        let extra = (nbytes as u32 * 8) - bits;
        v >>= extra;
    }
    v
}

/// The shortest decimal that reads back as this same half-precision number.
///
/// An f16 holds between three and four decimal digits. Widening one to an f64
/// and printing that spells out every digit of the wider number: a scale of
/// 0.00387 becomes 0.0038700103759765625, of which three digits are in the
/// file and the rest are an artefact of the widening. Rounding to the shortest
/// form that still reads back as the same sixteen bits loses nothing, and says
/// only what the file says.
pub(crate) fn narrow_f16(h: u16) -> f64 {
    let v = f16_to_f64(h);
    if !v.is_finite() {
        return v;
    }
    // Five significant digits always suffice for an f16; the loop stops at the
    // first that comes back as the same bits.
    (1..=5)
        .find_map(|digits| {
            let short: f64 = format!("{v:.*e}", digits - 1).parse().ok()?;
            (crate::encode::f64_to_f16(short) == h).then_some(short)
        })
        .unwrap_or(v)
}

/// The same for single precision, where the shortest form is what Rust prints
/// for an `f32` already; widening first is what spells out the rest.
pub(crate) fn narrow_f32(x: f32) -> f64 {
    if x.is_finite() { format!("{x}").parse().unwrap_or(x as f64) } else { x as f64 }
}

/// A brain float is the first sixteen bits of a single-precision float, so
/// reading one is putting the other sixteen back.
pub(crate) fn bf16_to_f64(h: u16) -> f64 {
    f32::from_bits((h as u32) << 16) as f64
}

/// The shortest decimal that reads back as this same brain float. Eight bits
/// of significand is between two and three decimal digits; four always say
/// which of them it is.
pub(crate) fn narrow_bf16(h: u16) -> f64 {
    let v = bf16_to_f64(h);
    if !v.is_finite() {
        return v;
    }
    (1..=4)
        .find_map(|digits| {
            let short: f64 = format!("{v:.*e}", digits - 1).parse().ok()?;
            (crate::encode::f64_to_bf16(short) == h).then_some(short)
        })
        .unwrap_or(v)
}

pub(crate) fn f16_to_f64(h: u16) -> f64 {
    let s = if h >> 15 == 1 { -1.0 } else { 1.0 };
    let e = ((h >> 10) & 0x1f) as i32;
    let f = (h & 0x3ff) as f64;
    match e {
        0 => s * 2f64.powi(-14) * (f / 1024.0),
        0x1f => {
            if f == 0.0 {
                s * f64::INFINITY
            } else {
                f64::NAN
            }
        }
        _ => s * 2f64.powi(e - 15) * (1.0 + f / 1024.0),
    }
}
