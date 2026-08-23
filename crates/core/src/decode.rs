//! Reading typed values out of packed bits: the half that `encode.rs` inverts.
//!
//! Nothing here touches the document or the source. It takes the bits a field
//! occupies (MSB-first, as `Document::read_bits` produces them) and turns them
//! into a number, plus the static size questions the evaluator asks of a type.

use crate::bits::bytes_for;
use crate::template::{Endian, Expr, Ty};

/// A short text or byte field used in an expression is its bytes as a
/// big-endian number, so a switch can key on e.g. "IHDR".
pub(crate) fn be_int(b: &[u8]) -> i128 {
    b.iter().fold(0i128, |acc, &x| (acc << 8) | x as i128)
}

/// Size in bits if it does not depend on data.
pub fn fixed_bits(ty: &Ty) -> Option<u64> {
    Some(match ty {
        Ty::UInt { bits, .. } | Ty::Int { bits, .. } => *bits as u64,
        Ty::F16(_) => 16,
        Ty::Fixed { bits, .. } => *bits as u64,
        Ty::F32(_) => 32,
        Ty::F64(_) => 64,
        Ty::Magic(b) => b.len() as u64 * 8,
        Ty::Bytes(Expr::Lit(n)) | Ty::Utf8(Expr::Lit(n)) => (*n).max(0) as u64 * 8,
        Ty::Struct(s) => s.fields.iter().map(|f| fixed_bits(&f.ty)).sum::<Option<u64>>()?,
        Ty::Array { elem, count: Expr::Lit(n) } => fixed_bits(elem)? * (*n).max(0) as u64,
        Ty::Sized { size: Expr::Lit(n), .. } => (*n).max(0) as u64 * 8,
        Ty::Enum { inner, .. } => fixed_bits(inner)?,
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
