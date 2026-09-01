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

/// Whether a field of `bits` bits starting at `offset` is packed from the
/// bottom of its byte rather than the top.
///
/// Only fields with a byte boundary somewhere other than at both ends of them
/// are: a number of whole bytes on a byte boundary is ordered by `endian` the
/// ordinary way, and that path is untouched.
pub fn lsb_packed(bits: u32, endian: Endian, offset: u64) -> bool {
    endian == Endian::Little && (bits % 8 != 0 || offset % 8 != 0)
}

/// Where an LSB-first field's bits actually sit.
///
/// A field's offset is a count of the bits laid down before it, and for an
/// MSB-first field that count is also an address: bit 0 of a byte is its top
/// bit, so the first field in a byte is at the top. LSB-first stacks the other
/// way, so the first field in a byte is at the bottom and the count has to be
/// turned around inside the byte to say where the bits are. Two fields of
/// three and five bits in one byte are at bit 5 and bit 0 of it, in that
/// order, which is `0bBBBBBAAA` written out.
///
/// Turned around, the bits are contiguous and in MSB order, so everything
/// downstream — reading, writing, highlighting, the gutter — takes them as it
/// takes any other field, and only the address changed.
///
/// `None` for a field that would straddle a byte boundary, because that one
/// cannot be turned around: a twelve-bit LSB-first field is the whole of one
/// byte and the low nibble of the next, and a bit address space numbered from
/// the top of each byte has no single range that means those bits. Such a
/// field is refused rather than placed somewhere it is not.
pub fn lsb_offset(bits: u32, offset: u64) -> Option<u64> {
    let start = offset % 8;
    let bits = bits as u64;
    (start + bits <= 8).then(|| offset - start + (8 - start - bits))
}

/// The width and byte order of an integer field, looked through the wrappers
/// that only name its values. What decides how the field is packed.
pub fn packed_int(ty: &Ty) -> Option<(u32, Endian)> {
    match ty {
        Ty::UInt { bits, endian } | Ty::Int { bits, endian } | Ty::Fixed { bits, endian, .. } => Some((*bits, *endian)),
        Ty::Enum { inner, .. } | Ty::Flags { inner, .. } => packed_int(inner),
        _ => None,
    }
}

/// Size in bits if it does not depend on data.
pub fn fixed_bits(ty: &Ty) -> Option<u64> {
    Some(match ty {
        Ty::UInt { bits, .. } | Ty::Int { bits, .. } => *bits as u64,
        Ty::F16(_) | Ty::BF16(_) => 16,
        Ty::F8 { .. } => 8,
        Ty::Fixed { bits, .. } => *bits as u64,
        Ty::F32(_) => 32,
        Ty::F64(_) => 64,
        Ty::F80(_) => 80,
        Ty::Magic(b) => b.len() as u64 * 8,
        Ty::Bytes(Expr::Lit(n)) => (*n).max(0) as u64 * 8,
        // Text is fixed-size only when its length does not depend on the bytes.
        Ty::Str { len: StrLen::Fixed(Expr::Lit(n)), .. }
        | Ty::Str { len: StrLen::Padded { size: Expr::Lit(n), .. }, .. }
        | Ty::TextInt { len: StrLen::Fixed(Expr::Lit(n)), .. } => (*n).max(0) as u64 * 8,
        Ty::Struct(s) => s.fields.iter().map(|f| fixed_bits(&f.ty)).sum::<Option<u64>>()?,
        Ty::Array { elem, count: Expr::Lit(n) } => fixed_bits(elem)? * (*n).max(0) as u64,
        Ty::Sized { size: Expr::Lit(n), .. } => (*n).max(0) as u64 * 8,
        Ty::Enum { inner, .. } | Ty::Flags { inner, .. } => fixed_bits(inner)?,
        // A computed field is a value and no bits.
        Ty::Computed(_) => 0,
        // A field pointing somewhere else is a place and no bits, so a
        // structure holding one is still as fixed as the rest of it.
        Ty::At { .. } => 0,
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

/// What an eight-bit float is worth. `e4m3` has four bits of exponent and
/// three of fraction, with the top exponent still holding numbers rather than
/// infinities: only an exponent and a fraction of all ones is not a number.
/// `e5m2` has five and two and works like every other IEEE float, infinities
/// and all.
///
/// Every value either form can hold is a float exactly, so nothing is rounded
/// here and nothing needs shortening afterwards.
/// An eighty-bit extended float as the nearest f64.
///
/// Unlike every other float here, the significand carries its leading one
/// rather than assuming it, so the value is the significand scaled by the
/// exponent with nothing to put back. f64 holds 53 of those 64 bits: a value
/// that uses all of them rounds, and the sample rates and round numbers these
/// are written for do not.
pub(crate) fn f80_to_f64(bits: u128) -> f64 {
    let sign = if bits >> 79 & 1 == 1 { -1.0 } else { 1.0 };
    let exp = ((bits >> 64) & 0x7fff) as i32;
    let significand = (bits & u64::MAX as u128) as u64;
    if exp == 0x7fff {
        // The leading one is written out here, so what says infinity from not
        // a number is the sixty-three bits below it.
        return if significand << 1 == 0 { sign * f64::INFINITY } else { f64::NAN };
    }
    // Zero and the denormals need no case of their own: the scaling takes
    // them to zero on its own.
    //
    // The two halves of the scaling are applied apart. Together they would be
    // 2 to the power of the exponent less 16446, and for a small number that
    // is past what an f64 can hold even though the answer is not: the
    // significand brings it back up, but only if it is still there to.
    sign * (significand as f64 * 2f64.powi(-63)) * 2f64.powi(exp - 16383)
}

pub(crate) fn f8_to_f64(b: u8, e4m3: bool) -> f64 {
    let (exp_bits, frac_bits) = if e4m3 { (4u32, 3u32) } else { (5u32, 2u32) };
    let bias = (1i32 << (exp_bits - 1)) - 1;
    let sign = if b & 0x80 != 0 { -1.0 } else { 1.0 };
    let exp = ((b >> frac_bits) & ((1 << exp_bits) - 1) as u8) as i32;
    let frac = (b & ((1 << frac_bits) - 1) as u8) as f64;
    let top = (1 << exp_bits) - 1;
    let scale = (1u32 << frac_bits) as f64;
    if exp == top {
        if e4m3 {
            // The one value this form spends on not being a number.
            if frac as u32 == (1 << frac_bits) - 1 {
                return f64::NAN;
            }
        } else if frac == 0.0 {
            return sign * f64::INFINITY;
        } else {
            return f64::NAN;
        }
    }
    if exp == 0 {
        // Subnormal: no leading one, and the exponent the smallest normal has.
        return sign * (frac / scale) * 2f64.powi(1 - bias);
    }
    sign * (1.0 + frac / scale) * 2f64.powi(exp - bias)
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
