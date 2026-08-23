//! Writing a typed field back into the document: the inverse of the readers in
//! `eval.rs`.
//!
//! Text goes in, MSB-first packed bits come out, ready for
//! `Document::overwrite_bits`. Every encoder produces exactly the field's
//! current size, so a write never shifts the bytes that follow it. Types whose
//! natural encoding is variable length (LEB128, text, bytes) are padded or
//! rejected rather than resized; growing a field is a structural edit and
//! belongs with the redundant-editing work, not here.

use crate::bits::bytes_for;
use crate::template::{Endian, StrLen, Ty};
use crate::text::{self, Settled};

/// The longest text or byte field that can be written as text. Past this a
/// value is not something anyone retypes, and the whole of it would have to be
/// held in the page to be edited at all.
pub const EDIT_LIMIT_BYTES: u64 = 4096;

/// Can a field of this type and current size be written from text?
///
/// A caller that only shows a preview of the value must apply its own, smaller
/// limit: writing back a value the user could not see would replace the part
/// the preview elided.
pub fn editable(ty: &Ty, size_bits: u64) -> bool {
    match ty {
        Ty::Enum { inner, .. } => editable(inner, size_bits),
        Ty::UInt { .. } | Ty::Int { .. } | Ty::F16(_) | Ty::F32(_) | Ty::F64(_) | Ty::Leb128 { .. } | Ty::Fixed { .. } => true,
        Ty::Bytes(_) | Ty::Str { .. } => size_bits <= EDIT_LIMIT_BYTES * 8,
        _ => false,
    }
}

/// What a text field currently holds, which decides how new text is written:
/// the encoding it was read as, and the byte-order mark to keep in front of it.
#[derive(Debug, Clone, Default)]
pub struct StrState {
    pub settled: Option<Settled>,
    pub bom: Vec<u8>,
}

/// Encode `text` as a value of `ty` occupying exactly `size_bits` bits.
///
/// `state` says how a text field was read; a field is written back in the
/// encoding it was read in, so a guess never silently flips on save.
pub fn encode(ty: &Ty, text: &str, size_bits: u64, state: &StrState) -> Result<Vec<u8>, String> {
    match ty {
        Ty::UInt { bits, endian } => {
            let v = parse_uint(text).ok_or_else(|| whole_number_msg(false))?;
            let max = mask(*bits);
            if v > max {
                return Err(range_msg(&ty.display_name(), "0", &max.to_string()));
            }
            Ok(write_uint(v, *bits, *endian))
        }
        Ty::Int { bits, endian } => {
            let v = parse_int(text).ok_or_else(|| whole_number_msg(true))?;
            let (min, max) = int_range(*bits);
            if v < min || v > max {
                return Err(range_msg(&ty.display_name(), &min.to_string(), &max.to_string()));
            }
            let u = (v as u128) & mask(*bits);
            Ok(write_uint(u, *bits, *endian))
        }
        Ty::Fixed { bits, frac, endian, signed } => {
            let x = parse_float(text)?;
            let scaled = (x * (1u64 << frac) as f64).round();
            let (min, max) = if *signed {
                let (lo, hi) = int_range(*bits);
                (lo as f64, hi as f64)
            } else {
                (0.0, mask(*bits) as f64)
            };
            if !scaled.is_finite() || scaled < min || scaled > max {
                // The true maximum is one step short of a power of two; printing
                // 65535.99998474121 helps nobody.
                let whole = bits - frac;
                let (lo, hi) = if *signed {
                    ((-(1i128 << (whole - 1))).to_string(), (1i128 << (whole - 1)).to_string())
                } else {
                    ("0".to_string(), (1i128 << whole).to_string())
                };
                return Err(format!("{} range is {lo} to just under {hi}.", ty.display_name()));
            }
            Ok(write_uint((scaled as i128 as u128) & mask(*bits), *bits, *endian))
        }
        Ty::F16(e) => {
            let x = parse_float(text)?;
            Ok(write_uint(f64_to_f16(x) as u128, 16, *e))
        }
        Ty::F32(e) => {
            let x = parse_float(text)?;
            Ok(write_uint((x as f32).to_bits() as u128, 32, *e))
        }
        Ty::F64(e) => {
            let x = parse_float(text)?;
            Ok(write_uint(x.to_bits() as u128, 64, *e))
        }
        Ty::Leb128 { signed } => {
            let room = (size_bits / 8) as usize;
            let bytes = if *signed {
                let v = parse_int(text).ok_or_else(|| whole_number_msg(true))?;
                leb_signed(v, room)
            } else {
                let v = parse_uint(text).ok_or_else(|| whole_number_msg(false))?;
                leb_unsigned(v, room)
            };
            bytes.ok_or_else(|| {
                let (min, max) = leb_limits(room, *signed);
                format!("{room}-byte {} range is {min} to {max}. Field sizes can't change yet.", ty.display_name())
            })
        }
        Ty::Enum { inner, def } => {
            let t = text.trim();
            match def.value_of(t) {
                Some(v) => encode(inner, &v.to_string(), size_bits, state),
                // Not a name: any number is still a legal value for the field.
                None => encode(inner, t, size_bits, state).map_err(|e| {
                    if parse_int(t).is_some() { e } else { enum_msg(&def.name, &def.cases) }
                }),
            }
        }
        Ty::Str { len, .. } => {
            let want = (size_bits / 8) as usize;
            let settled = state.settled.unwrap_or(Settled::Utf8);
            let unit = settled.unit();
            let bom = &state.bom;
            let body = text::encode_settled(settled, text).map_err(|c| cannot_hold_msg(settled, c))?;
            let room = want.saturating_sub(bom.len());
            // A character is not a byte in UTF-16, so both counts are given
            // when they differ; when they agree the extra count is noise.
            let chars = text.chars().count();
            match len {
                StrLen::Fixed(_) => {
                    if body.len() != room {
                        return Err(str_length_msg(body.len(), room, chars, settled));
                    }
                    Ok([bom.as_slice(), &body].concat())
                }
                StrLen::Padded { pad, .. } => {
                    if body.len() > room {
                        return Err(str_too_long_msg(body.len(), room, chars, settled));
                    }
                    let term = text::unit_bytes(settled, *pad);
                    if find_unit(&body, &term).is_some() {
                        return Err(no_pad_byte_msg(*pad));
                    }
                    if (room - body.len()) % unit != 0 {
                        return Err(odd_size_msg(settled, want));
                    }
                    let mut out = [bom.as_slice(), &body].concat();
                    while out.len() < want {
                        out.extend_from_slice(&term);
                    }
                    Ok(out)
                }
                StrLen::Terminated { end } => {
                    if room < unit {
                        return Err(format!("The field is {want} bytes; there's no room for text."));
                    }
                    if body.len() != room - unit {
                        return Err(str_length_msg(body.len(), room - unit, chars, settled));
                    }
                    let term = text::unit_bytes(settled, *end);
                    if find_unit(&body, &term).is_some() {
                        return Err(no_pad_byte_msg(*end));
                    }
                    Ok([bom.as_slice(), &body, &term].concat())
                }
            }
        }
        Ty::Bytes(_) => {
            let want = (size_bits / 8) as usize;
            let bytes = parse_hex(text).ok_or("Hex bytes only: 4a 2f 00.")?;
            if bytes.len() != want {
                return Err(length_msg(bytes.len(), want, "bytes"));
            }
            Ok(bytes)
        }
        Ty::Magic(_) => Err("Magic bytes are fixed by the format.".into()),
        _ => Err("This field can't be edited here. Use the hex view.".into()),
    }
}

/// Group a count for reading: 8487 -> "8,487". Sizes in messages are read by
/// people, not parsed.
pub(crate) fn commas(n: u64) -> String {
    let d = n.to_string();
    let mut out = String::with_capacity(d.len() + d.len() / 3);
    for (i, c) in d.chars().enumerate() {
        if i > 0 && (d.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn whole_number_msg(signed: bool) -> String {
    if signed {
        "Whole numbers only: -12, 0x1f, 0b1010.".into()
    } else {
        "Whole numbers only, 0 or higher: 12, 0x1f, 0b1010.".into()
    }
}

/// The names a field accepts, when the whole list fits on a status line.
/// A partial list of 200 opcodes would help nobody, so a long set is offered
/// the number formats instead.
fn enum_msg(name: &str, cases: &[(i128, String)]) -> String {
    let names: Vec<&str> = cases.iter().map(|(_, n)| n.as_str()).collect();
    let joined = names.join(", ");
    let a = article(name);
    if names.len() <= 8 && joined.len() <= 60 {
        format!("Not {a} {name} name or number. Names: {joined}.")
    } else {
        format!("Not {a} {name} name or number. Or type a number: 12, 0x1f, 0b1010.")
    }
}

/// Type names come from the template, so the article has to be worked out.
fn article(name: &str) -> &'static str {
    match name.chars().next() {
        Some(c) if "AEIOUaeiou".contains(c) => "an",
        _ => "a",
    }
}

fn range_msg(type_name: &str, min: &str, max: &str) -> String {
    format!("{type_name} range is {min} to {max}.")
}

/// Byte counts are what the field is measured in; character counts are what the
/// typist counted. In UTF-16 they differ, so both are given.
fn str_length_msg(got: usize, want: usize, chars: usize, settled: Settled) -> String {
    if got == chars {
        format!("Needs exactly {want} bytes of {}; got {got}. Field sizes can't change yet.", settled.name())
    } else {
        format!(
            "Needs exactly {want} bytes; got {got} ({chars} characters in {}). Field sizes can't change yet.",
            settled.name()
        )
    }
}

fn str_too_long_msg(got: usize, want: usize, chars: usize, settled: Settled) -> String {
    if got == chars {
        format!("Too long for this field: {got} bytes of {}; it holds {want}.", settled.name())
    } else {
        format!("Too long for this field: {got} bytes ({chars} characters in {}); it holds {want}.", settled.name())
    }
}

fn cannot_hold_msg(settled: Settled, c: char) -> String {
    format!("{} can't hold '{c}'.", settled.name())
}

fn odd_size_msg(settled: Settled, want: usize) -> String {
    format!("Odd size for {}: {want} bytes. The last character is incomplete.", settled.name())
}

/// Index of `term` in `hay`, aligned to whole units of its length.
fn find_unit(hay: &[u8], term: &[u8]) -> Option<usize> {
    let unit = term.len();
    (0..hay.len().saturating_sub(unit - 1)).step_by(unit).find(|i| hay[*i..*i + unit] == *term)
}

fn no_pad_byte_msg(pad: u8) -> String {
    format!("Can't contain 0x{pad:02x}; that's the byte that ends this text.")
}

fn length_msg(got: usize, want: usize, noun: &str) -> String {
    format!("Needs exactly {want} {noun}; got {got}. Field sizes can't change yet.")
}

/// What a LEB128 field of this many bytes can hold, for the message that says
/// a value will not fit.
fn leb_limits(room: usize, signed: bool) -> (String, String) {
    let bits = (room * 7).min(128) as u32;
    if !signed {
        return ("0".to_string(), mask(bits).to_string());
    }
    if bits >= 128 {
        return (i128::MIN.to_string(), i128::MAX.to_string());
    }
    ((-(1i128 << (bits - 1))).to_string(), ((1i128 << (bits - 1)) - 1).to_string())
}

fn mask(bits: u32) -> u128 {
    if bits >= 128 {
        u128::MAX
    } else {
        (1u128 << bits) - 1
    }
}

fn int_range(bits: u32) -> (i128, i128) {
    if bits >= 128 {
        (i128::MIN, i128::MAX)
    } else {
        (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
    }
}

/// The exact inverse of `decode::read_uint`: little-endian applies to whole-byte
/// widths only; narrower fields are packed big-endian, left-aligned in the buffer.
fn write_uint(v: u128, bits: u32, endian: Endian) -> Vec<u8> {
    let n = bytes_for(bits as u64);
    let mut out = vec![0u8; n];
    if endian == Endian::Little && bits % 8 == 0 {
        for (i, b) in out.iter_mut().enumerate() {
            *b = (v >> (8 * i)) as u8;
        }
    } else {
        let extra = (n as u32 * 8) - bits;
        let shifted = v << extra;
        for (i, b) in out.iter_mut().enumerate() {
            *b = (shifted >> (8 * (n - 1 - i))) as u8;
        }
    }
    out
}

fn strip(text: &str) -> String {
    text.trim().replace('_', "")
}

fn parse_uint(text: &str) -> Option<u128> {
    let t = strip(text);
    let t = t.strip_prefix('+').unwrap_or(&t);
    radix(t).and_then(|(digits, r)| u128::from_str_radix(digits, r).ok())
}

fn parse_int(text: &str) -> Option<i128> {
    let t = strip(text);
    let (neg, rest) = match t.strip_prefix('-') {
        Some(r) => (true, r.to_string()),
        None => (false, t.strip_prefix('+').unwrap_or(&t).to_string()),
    };
    let (digits, r) = radix(&rest)?;
    let v = i128::from_str_radix(digits, r).ok()?;
    Some(if neg { -v } else { v })
}

fn radix(t: &str) -> Option<(&str, u32)> {
    let lower = t.get(..2).map(|s| s.to_ascii_lowercase());
    let (digits, r) = match lower.as_deref() {
        Some("0x") => (&t[2..], 16),
        Some("0b") => (&t[2..], 2),
        Some("0o") => (&t[2..], 8),
        _ => (t, 10),
    };
    if digits.is_empty() {
        None
    } else {
        Some((digits, r))
    }
}

fn parse_float(text: &str) -> Result<f64, String> {
    let t = strip(text);
    t.parse::<f64>().map_err(|_| "Numbers only: 1.5, -2e10, inf, nan.".to_string())
}

fn parse_hex(text: &str) -> Option<Vec<u8>> {
    let t: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(&t).to_string();
    if t.is_empty() || t.len() % 2 != 0 || !t.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    (0..t.len() / 2).map(|i| u8::from_str_radix(&t[i * 2..i * 2 + 2], 16).ok()).collect()
}

/// LEB128 padded to exactly `room` bytes. Padding a value out with redundant
/// continuation bytes is legal LEB128, and it is what keeps the field's size
/// stable so the rest of the file does not shift.
fn leb_unsigned(mut v: u128, room: usize) -> Option<Vec<u8>> {
    if room == 0 || room > 19 {
        return None;
    }
    let mut out = Vec::with_capacity(room);
    while out.len() < room {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        let last = out.len() + 1 == room;
        out.push(if last { byte } else { byte | 0x80 });
    }
    if v != 0 {
        return None; // the value needs more bytes than the field has
    }
    Some(out)
}

fn leb_signed(v: i128, room: usize) -> Option<Vec<u8>> {
    if room == 0 || room > 19 {
        return None;
    }
    let mut x = v;
    let mut out = Vec::with_capacity(room);
    while out.len() < room {
        let byte = (x & 0x7f) as u8;
        x >>= 7; // arithmetic shift, so the sign extends into the padding
        let last = out.len() + 1 == room;
        out.push(if last { byte } else { byte | 0x80 });
    }
    // The final byte must carry the sign for the reader to recover the value.
    let last = *out.last()?;
    let ok = if v < 0 { x == -1 && last & 0x40 != 0 } else { x == 0 && last & 0x40 == 0 };
    if !ok {
        return None;
    }
    Some(out)
}

/// Inverse of `decode::f16_to_f64`.
fn f64_to_f16(x: f64) -> u16 {
    if x.is_nan() {
        return 0x7e00;
    }
    let sign: u16 = if x.is_sign_negative() { 0x8000 } else { 0 };
    let a = x.abs();
    if a.is_infinite() {
        return sign | 0x7c00;
    }
    if a == 0.0 {
        return sign;
    }
    let mut e = a.log2().floor() as i32;
    if a / 2f64.powi(e) >= 2.0 {
        e += 1; // log2 rounds up at powers of two
    }
    if e < -14 {
        // Subnormal. Rounding up to 1024 carries into the exponent field on its
        // own, giving the smallest normal, so there is nothing to clamp.
        return sign | (a / 2f64.powi(-24)).round() as u16;
    }
    if e > 15 {
        return sign | 0x7c00;
    }
    let m = a / 2f64.powi(e) - 1.0;
    let mut f = (m * 1024.0).round() as u32;
    if f == 1024 {
        f = 0;
        e += 1;
        if e > 15 {
            return sign | 0x7c00;
        }
    }
    sign | (((e + 15) as u16) << 10) | f as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::{f16_to_f64, read_uint};
    use crate::template::Expr;

    #[test]
    fn round_trips_through_the_reader() {
        for bits in [1u32, 3, 7, 8, 12, 16, 24, 32, 33, 64, 100, 128] {
            for endian in [Endian::Little, Endian::Big] {
                let v = mask(bits) / 3;
                let ty = Ty::UInt { bits, endian };
                let buf = encode(&ty, &v.to_string(), bits as u64, &StrState::default()).unwrap();
                assert_eq!(read_uint(&buf, bits, endian), v, "u{bits} {endian:?}");
            }
        }
    }

    #[test]
    fn signed_wraps_to_twos_complement() {
        let ty = Ty::Int { bits: 8, endian: Endian::Big };
        assert_eq!(encode(&ty, "-1", 8, &StrState::default()).unwrap(), vec![0xff]);
        assert_eq!(encode(&ty, "-128", 8, &StrState::default()).unwrap(), vec![0x80]);
        assert!(encode(&ty, "-129", 8, &StrState::default()).is_err());
        assert!(encode(&ty, "128", 8, &StrState::default()).is_err());
    }

    #[test]
    fn narrow_fields_are_left_aligned() {
        // Three bits of value 0b101 sit at the top of the byte.
        let ty = Ty::UInt { bits: 3, endian: Endian::Little };
        assert_eq!(encode(&ty, "0b101", 3, &StrState::default()).unwrap(), vec![0b1010_0000]);
    }

    #[test]
    fn little_endian_reverses_whole_bytes() {
        let ty = Ty::UInt { bits: 32, endian: Endian::Little };
        assert_eq!(encode(&ty, "0x01020304", 32, &StrState::default()).unwrap(), vec![4, 3, 2, 1]);
    }

    #[test]
    fn floats_round_trip() {
        for x in [0.0f64, 1.0, -2.5, 65504.0, -0.125] {
            assert_eq!(f16_to_f64(f64_to_f16(x)), x, "f16 {x}");
        }
        assert_eq!(f64_to_f16(f64::NEG_INFINITY), 0xfc00);
        // Subnormals, and a value just under the smallest normal (2^-14), which
        // rounds up into it rather than sticking at the largest subnormal.
        for x in [2f64.powi(-24), 2f64.powi(-16), 2f64.powi(-14) - 2f64.powi(-26), 2f64.powi(-14)] {
            let back = f16_to_f64(f64_to_f16(x));
            assert!((back - x).abs() <= 2f64.powi(-25), "f16 {x} -> {back}");
        }
        assert_eq!(encode(&Ty::F32(Endian::Big), "1.5", 32, &StrState::default()).unwrap(), vec![0x3f, 0xc0, 0, 0]);
        assert_eq!(encode(&Ty::F64(Endian::Little), "1.5", 64, &StrState::default()).unwrap(), vec![0, 0, 0, 0, 0, 0, 0xf8, 0x3f]);
        assert!(encode(&Ty::F64(Endian::Little), "nan", 64, &StrState::default()).is_ok());
        assert!(encode(&Ty::F32(Endian::Big), "one", 32, &StrState::default()).is_err());
    }

    #[test]
    fn leb128_pads_to_the_fields_current_size() {
        assert_eq!(leb_unsigned(1, 1).unwrap(), vec![0x01]);
        assert_eq!(leb_unsigned(1, 3).unwrap(), vec![0x81, 0x80, 0x00]);
        assert_eq!(leb_unsigned(624485, 3).unwrap(), vec![0xe5, 0x8e, 0x26]);
        assert!(leb_unsigned(624485, 2).is_none());
        assert_eq!(leb_signed(-1, 1).unwrap(), vec![0x7f]);
        assert_eq!(leb_signed(-1, 3).unwrap(), vec![0xff, 0xff, 0x7f]);
        assert_eq!(leb_signed(-123456, 3).unwrap(), vec![0xc0, 0xbb, 0x78]);
        assert_eq!(leb_signed(2, 2).unwrap(), vec![0x82, 0x00]);
        assert!(leb_signed(-123456, 2).is_none());
    }

    #[test]
    fn text_and_bytes_must_keep_their_length() {
        assert!(encode(&Ty::utf8(Expr::lit(4)), "IHDR", 32, &StrState::default()).is_ok());
        assert!(encode(&Ty::utf8(Expr::lit(4)), "IHD", 32, &StrState::default()).is_err());
        assert_eq!(encode(&Ty::Bytes(Expr::lit(2)), "de ad", 16, &StrState::default()).unwrap(), vec![0xde, 0xad]);
        assert!(encode(&Ty::Bytes(Expr::lit(2)), "dead be", 16, &StrState::default()).is_err());
        assert!(encode(&Ty::Magic(vec![1]), "01", 8, &StrState::default()).is_err());
    }

    #[test]
    fn counts_are_grouped_for_reading() {
        assert_eq!(commas(0), "0");
        assert_eq!(commas(999), "999");
        assert_eq!(commas(4096), "4,096");
        assert_eq!(commas(1234567), "1,234,567");
    }

    #[test]
    fn padded_text_keeps_the_field_size() {
        let ty = Ty::utf8_padded(Expr::lit(8), 0);
        assert_eq!(encode(&ty, "hi", 64, &StrState::default()).unwrap(), b"hi\0\0\0\0\0\0".to_vec());
        assert_eq!(encode(&ty, "12345678", 64, &StrState::default()).unwrap(), b"12345678".to_vec());
        // One byte too many, and a value that would look truncated when read back.
        assert!(encode(&ty, "123456789", 64, &StrState::default()).is_err());
        assert!(encode(&ty, "a\0b", 64, &StrState::default()).is_err());
        let spaces = Ty::utf8_padded(Expr::lit(4), b' ');
        assert_eq!(encode(&spaces, "ab", 32, &StrState::default()).unwrap(), b"ab  ".to_vec());
    }

    #[test]
    fn terminated_text_leaves_room_for_the_terminator() {
        let ty = Ty::cstr();
        // Five bytes of field: four of text, then the NUL.
        assert_eq!(encode(&ty, "abcd", 40, &StrState::default()).unwrap(), b"abcd\0".to_vec());
        assert!(encode(&ty, "abc", 40, &StrState::default()).is_err());
        assert!(encode(&ty, "abcde", 40, &StrState::default()).is_err());
        assert!(encode(&ty, "ab\0d", 40, &StrState::default()).is_err());
    }

    #[test]
    fn number_bases_and_junk() {
        let ty = Ty::UInt { bits: 16, endian: Endian::Big };
        assert_eq!(encode(&ty, " 0x1F ", 16, &StrState::default()).unwrap(), vec![0, 0x1f]);
        assert_eq!(encode(&ty, "1_000", 16, &StrState::default()).unwrap(), vec![0x03, 0xe8]);
        assert!(encode(&ty, "", 16, &StrState::default()).is_err());
        assert!(encode(&ty, "12abc", 16, &StrState::default()).is_err());
        assert!(encode(&ty, "-1", 16, &StrState::default()).is_err());
    }
}
