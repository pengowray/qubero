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

/// Whether a field of type `ty` placed at bit `offset` is read low bit first.
///
/// Every integer [`packed_int`] answers for, and the floats as well, since a
/// format that packs from the bottom of a byte writes those the same way: a
/// JPEG XL header holds an f16 as sixteen bits read low bit first, from
/// wherever the bits before it ended. A [`Ty::UIntExpr`] says `Little` before
/// its width is known, so it is always read this way; at whole bytes on a byte
/// boundary that comes to the ordinary little-endian number.
pub fn low_bit_first(ty: &Ty, offset: u64) -> bool {
    match ty {
        Ty::Enum { inner, .. } | Ty::Flags { inner, .. } | Ty::Nullable { inner, .. } => low_bit_first(inner, offset),
        Ty::UIntExpr { endian, .. } => *endian == Endian::Little,
        Ty::F16(e) | Ty::BF16(e) => lsb_packed(16, *e, offset),
        Ty::F32(e) => lsb_packed(32, *e, offset),
        Ty::F64(e) => lsb_packed(64, *e, offset),
        _ => packed_int(ty).is_some_and(|(bits, endian)| lsb_packed(bits, endian, offset)),
    }
}

/// The bit at the other end of its byte: bit 1 from the top is bit 1 from the
/// bottom.
///
/// A field read low bit first is placed by the count of bits laid down before
/// it, with each byte's bits counted from the bottom: the first field in a
/// byte is at its lowest bit, and a twelve-bit field starting there is that
/// whole byte and the low nibble of the next. That is the order the format
/// counts in, and in it every such field is one run, however many bytes it
/// touches. The hex view and a cursor number bits from the top of each byte,
/// so a bit asked about from there is turned round with this before it is
/// compared with one of these fields; at whole bytes the two orders agree.
pub fn mirror_in_byte(bit: u64) -> u64 {
    bit ^ 7
}

/// Whether a type holds numbers read low bit first, so that a node of it
/// starting or ending partway through a byte counts that byte's bits from the
/// bottom, as the numbers inside it do. Looked for in a structure's own
/// fields, a list's elements, and through the wrappers that only choose or
/// bound what is inside, `depth` levels down; a named type is not followed.
///
/// A node whose ends are both on byte boundaries counts the same in either
/// order, so this is only asked of one that is not.
pub fn packs_low_bit_first(ty: &Ty, depth: u32) -> bool {
    let leaf = |t: &Ty| match t.without_sentinel() {
        Ty::UIntExpr { endian, .. } => *endian == Endian::Little,
        t => packed_int(t).is_some_and(|(bits, endian)| endian == Endian::Little && bits % 8 != 0),
    };
    let inner = |t: &Ty| leaf(t) || (depth > 0 && packs_low_bit_first(t, depth - 1));
    match ty {
        Ty::Struct(s) => s.fields.iter().any(|f| inner(&f.ty)),
        Ty::Array { elem, .. } | Ty::Repeat { elem, .. } => inner(elem),
        Ty::When { inner: t, .. } | Ty::Sized { inner: t, .. } | Ty::SizedBits { inner: t, .. } => inner(t),
        Ty::Switch { cases, default, .. } => cases.iter().any(|(_, t)| inner(t)) || inner(default),
        _ => false,
    }
}

/// Where an LSB-first field that fits in one byte sits when the bits are
/// numbered from the top of the byte, which is where a write puts it.
///
/// Two fields of three and five bits in one byte are at count 0 and 3, and
/// from the top they are at bit 5 and bit 0, which is `0bBBBBBAAA` written
/// out. Turned round like this the field is a run of bits high bit first, the
/// way every other field is written.
///
/// `None` for a field that crosses a byte boundary, which has no single run
/// numbered from the top: a twelve-bit field from the bottom of a byte is all
/// of that byte and the low nibble of the next. Such a field is read and shown
/// but not written; see [`low_bit_first`].
pub fn lsb_offset(bits: u32, offset: u64) -> Option<u64> {
    let start = offset % 8;
    let bits = bits as u64;
    (start + bits <= 8).then(|| offset - start + (8 - start - bits))
}

/// `bits` bits taken low bit first, starting `skip` bits from the bottom of
/// `raw[0]`, laid out the way the field's own decoder reads a number: whole
/// bytes as little-endian bytes, anything narrower as a run of bits high bit
/// first. [`read_uint`] with `Little` then gives the number the format wrote,
/// and so does every reading built on it: signed, floats, fixed point.
///
/// The first bit taken is the number's lowest, which is what packing from the
/// bottom of a byte means, so a field crossing into the next byte carries on
/// from that byte's lowest bit.
pub(crate) fn gather_lsb(raw: &[u8], skip: u32, bits: u32) -> Vec<u8> {
    let mut v: u128 = 0;
    for i in 0..bits.min(128) {
        let at = (skip + i) as usize;
        let bit = (raw[at / 8] >> (at % 8)) & 1;
        v |= (bit as u128) << i;
    }
    let n = bytes_for(bits as u64);
    if bits % 8 == 0 {
        v.to_le_bytes()[..n].to_vec()
    } else {
        (v << (n as u32 * 8 - bits)).to_be_bytes()[16 - n..].to_vec()
    }
}

/// The width and byte order of an integer field, looked through the wrappers
/// that only name its values. What decides how the field is packed.
pub fn packed_int(ty: &Ty) -> Option<(u32, Endian)> {
    match ty {
        Ty::UInt { bits, endian }
        | Ty::Int { bits, endian }
        | Ty::SignMagnitude { bits, endian }
        | Ty::Fixed { bits, endian, .. } => Some((*bits, *endian)),
        Ty::Enum { inner, .. } | Ty::Flags { inner, .. } | Ty::Nullable { inner, .. } => packed_int(inner),
        // How wide it is, is not known until it is read. Nothing that places
        // a field needs to know: see [`low_bit_first`].
        Ty::UIntExpr { .. } => None,
        _ => None,
    }
}

/// Size in bits if it does not depend on data.
pub fn fixed_bits(ty: &Ty) -> Option<u64> {
    Some(match ty {
        Ty::UInt { bits, .. } | Ty::Int { bits, .. } | Ty::SignMagnitude { bits, .. } => *bits as u64,
        // Never fixed, not even for a width written as a number: a field whose
        // width is an expression is placed without that width being known,
        // and one rule for all of them is simpler than two. A literal width
        // still costs nothing to read, because `stride` asks for it once for
        // the whole run.
        Ty::UIntExpr { .. } => return None,
        Ty::Nullable { inner, .. } => fixed_bits(inner)?,
        Ty::F16(_) | Ty::BF16(_) => 16,
        Ty::F8 { .. } => 8,
        Ty::Fixed { bits, .. } => *bits as u64,
        Ty::F32(_) => 32,
        Ty::F64(_) => 64,
        Ty::F80(_) => 80,
        Ty::IbmF32(_) => 32,
        Ty::Magic(b) => b.len() as u64 * 8,
        Ty::Bytes(Expr::Lit(n)) => (*n).max(0) as u64 * 8,
        // Text is fixed-size only when its length does not depend on the bytes.
        Ty::Str { len: StrLen::Fixed(Expr::Lit(n)), .. }
        | Ty::Str { len: StrLen::Padded { size: Expr::Lit(n), .. }, .. }
        | Ty::TextInt { len: StrLen::Fixed(Expr::Lit(n)), .. } => (*n).max(0) as u64 * 8,
        // A union is as wide as its widest field: every field of one starts
        // where the record does. See `StructDef::overlap`.
        Ty::Struct(s) if s.overlap => {
            s.fields.iter().map(|f| fixed_bits(&f.ty)).collect::<Option<Vec<u64>>>()?.into_iter().max().unwrap_or(0)
        }
        Ty::Struct(s) => s.fields.iter().map(|f| fixed_bits(&f.ty)).sum::<Option<u64>>()?,
        Ty::Array { elem, count: Expr::Lit(n) } => fixed_bits(elem)? * (*n).max(0) as u64,
        Ty::Sized { size: Expr::Lit(n), .. } => (*n).max(0) as u64 * 8,
        Ty::Enum { inner, .. } | Ty::Flags { inner, .. } => fixed_bits(inner)?,
        // A computed field is a value and no bits, whether that value is a
        // number or a word.
        Ty::Computed(_) | Ty::ComputedText(_) | Ty::ComputedReal(_) => 0,
        // A field pointing somewhere else is a place and no bits, so a
        // structure holding one is still as fixed as the rest of it. A chain
        // is the same: it is a run of places, and the places are elsewhere. So
        // is a gather, unless a `Sized` round it gives it a region to cover.
        Ty::At { .. } | Ty::Chain { .. } | Ty::Gather { .. } => 0,
        // A stream joined from runs elsewhere is no bits where it is declared:
        // the runs are fields of their own, and what it reads is in a space
        // of its own.
        Ty::Stitched { .. } => 0,
        // A field that may not be there is nought bytes or the width of what
        // is inside it, and which it is, is what the file says. Nothing fixed
        // about that, so a structure holding one has no fixed size either.
        Ty::When { .. } => return None,
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

/// Interpret a sign-and-magnitude integer of `bits` bits: the top bit is the
/// sign and the rest are how far from zero, read as an ordinary unsigned
/// number rather than as a complement. Negative zero comes back as zero, which
/// is the number it is. See [`Ty::SignMagnitude`].
pub(crate) fn read_sign_magnitude(buf: &[u8], bits: u32, endian: Endian) -> i128 {
    let u = read_uint(buf, bits, endian);
    if bits == 0 {
        return 0;
    }
    let magnitude = (u & ((1u128 << (bits - 1)) - 1)) as i128;
    if (u >> (bits - 1)) & 1 == 1 { -magnitude } else { magnitude }
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

/// An IBM hexadecimal float: the top bit is the sign, the next seven an
/// exponent of sixteen biased by 64, and the low twenty-four a fraction with
/// the point in front of it. See [`Ty::IbmF32`].
///
/// Nothing is special. A zero fraction is zero whatever the exponent says, and
/// an exponent of all ones is just a large number: the largest this can hold
/// is a whisker under 16^63, and the smallest with its top nibble set is
/// 16^-65. Both are well inside an f64, and so is every step between them.
pub(crate) fn ibm32_to_f64(word: u32) -> f64 {
    let sign = if word >> 31 == 1 { -1.0 } else { 1.0 };
    let exp = ((word >> 24) & 0x7f) as i32 - 64;
    let fraction = f64::from(word & 0x00ff_ffff) / f64::from(1u32 << 24);
    sign * fraction * 16f64.powi(exp)
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
