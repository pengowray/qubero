//! Undoing what a frame library did to the numbers of an FrVect.
//!
//! A vector's `compress` field names a scheme in its low byte, and says in its
//! high byte which way round the machine that packed it wrote its words: 256
//! is added by a little-endian writer. The template opens the gzip scheme as a
//! space of its own, since that is the gzip codec and nothing else. Two
//! schemes are more than a codec, and the template leaves them to this:
//!
//! - **3, differences then gzip.** What comes out of the gzip stream is not
//!   the numbers but the difference of each from the one before, the first
//!   from nought. The template opens the stream and shows those differences;
//!   this adds them back up.
//! - **5, 8 and 10, zero suppression** of 2, 4 and 8-byte words. The numbers
//!   are differenced first, the same way, and the differences are then packed
//!   by how many bits each block of them needs.
//!
//! Every step is reported, not just the answer, which is the arrangement
//! [`hdf5_chunk`](super::hdf5_chunk) has: which step, how many bytes went in,
//! how many came out.
//!
//! **Zero suppression**, from appendix B of the specification (LIGO-T970130,
//! version 8). The run starts with a two-byte block size. Then, for each block
//! of that many differences, a few bits hold one less than the number of bits
//! every difference in the block fits in: 3 bits of it for 1-byte words, 4 for
//! 2-byte, 5 for 4-byte, 6 for 8-byte. Then each difference of the block in
//! that many bits, with `2^(nBits-1) - 1` added so that it is never negative.
//! Bits are taken from the low end of a word first, and the words are the
//! vector's own width, written the way round the `compress` field says. A
//! block that runs past the end of the vector stops at `nData`, which is why
//! the count has to come from outside the run: the last word is padded, and
//! padding read as bits is more differences of nought.
//!
//! The specification's own worked example, the 16-bit words
//! `0003 2D17 37F8 2963 0025`, reads back as the eight numbers it started
//! from; a test here holds it to that. The 2-byte scheme is also checked
//! against FrameL's test file, whose `fastProc` channel is written as twice
//! `fastAdc1`, uncompressed, in every frame: every one of the 2,000 packed
//! shorts in each of its ten frames comes back as half the float beside it.
//! The 4-byte and 8-byte schemes, differences then gzip, and complex numbers
//! under zero suppression have no sample here, so they follow the
//! specification's words and a test that packs by the same words.
//!
//! A complex vector is packed as all its real parts and then all its
//! imaginary parts, differenced as one run of integers; putting each real part
//! back beside its imaginary part is the last step. A float is differenced as
//! the integer with the same bits.

use super::hdf5_chunk::Step;
use crate::bits::LowBits;

/// What [`StructDef::packed`](crate::template::StructDef::packed) calls this,
/// so the template can mark a packed vector and the panel can find its way
/// back here.
pub const PACKING: &str = "gwf_vect";

/// The largest packed run this will unpack. Frames hold a second or a few
/// seconds of a channel; a claim far past that is a reason to stop.
pub const PACKED_LIMIT: usize = 64 << 20;
const DECODED_LIMIT: usize = 256 << 20;

/// How many numbers a panel shows, the same as for an HDF5 chunk.
pub const SHOWN: usize = 32;

/// What a vector turned out to hold, and what it took to get there.
#[derive(Debug, Clone, PartialEq)]
pub struct Unpacked {
    pub packed_bytes: usize,
    pub steps: Vec<Step>,
    /// The numbers' bytes, once every step that could be done was, the way
    /// round `little` says.
    pub bytes: Vec<u8>,
    pub little: bool,
    /// Why the unpacking stopped early, where it did.
    pub problem: Option<String>,
}

/// One number of each vector type: how many bytes, whether it is a pair of
/// floats, whether it is an integer, and what a panel calls it. `None` for a
/// string vector and for a type the specification does not list.
fn element(vect_type: u16) -> Option<(usize, bool, bool, &'static str)> {
    Some(match vect_type {
        0 => (1, false, true, "i8"),
        1 => (2, false, true, "i16"),
        2 => (8, false, false, "f64"),
        3 => (4, false, false, "f32"),
        4 => (4, false, true, "i32"),
        5 => (8, false, true, "i64"),
        6 => (8, true, false, "complex f32"),
        7 => (16, true, false, "complex f64"),
        9 => (2, false, true, "u16"),
        10 => (4, false, true, "u32"),
        11 => (8, false, true, "u64"),
        12 => (1, false, true, "u8"),
        _ => return None,
    })
}

/// The answer for a packed run of `packed_bytes` that is over
/// [`PACKED_LIMIT`], without the bytes: a run that large is not read at all.
pub fn refused(packed_bytes: usize, compress: u16) -> Unpacked {
    let mb = PACKED_LIMIT / (1 << 20);
    Unpacked {
        packed_bytes,
        steps: Vec::new(),
        bytes: Vec::new(),
        little: compress & 0x100 != 0,
        problem: Some(format!("Not unpacked: the vector is over this viewer's {mb} MB limit.")),
    }
}

/// Undo the packing. `data` is the whole of the vector's `data` field,
/// `compress` and `vect_type` its two fields of those names, and `n_data` how
/// many numbers it says it holds.
pub fn decode(data: &[u8], compress: u16, vect_type: u16, n_data: u64) -> Unpacked {
    if data.len() > PACKED_LIMIT {
        return refused(data.len(), compress);
    }
    let little = compress & 0x100 != 0;
    let mut out = Unpacked { packed_bytes: data.len(), steps: Vec::new(), bytes: Vec::new(), little, problem: None };
    let Some((width, complex, integer, name)) = element(vect_type) else {
        // The name the tree shows in the vector's `type` field, where it has one.
        let shown = if vect_type == 8 { "STRING".to_string() } else { vect_type.to_string() };
        out.problem = Some(format!("Not unpacked: type = {shown} is not a numeric type this viewer reads."));
        return out;
    };
    // A complex vector is two runs of its part's width, one of real parts and
    // one of imaginary parts.
    let (word, words) = if complex { (width / 2, n_data.saturating_mul(2)) } else { (width, n_data) };
    if words.saturating_mul(word as u64) > DECODED_LIMIT as u64 {
        let (mb, limit) = (words.saturating_mul(word as u64) >> 20, DECODED_LIMIT >> 20);
        let n = crate::encode::commas(n_data);
        out.problem =
            Some(format!("Not unpacked: {n} values as {name} would unpack to {mb} MB, over this viewer's {limit} MB limit."));
        return out;
    }
    let words = words as usize;
    match compress & 0xff {
        1 | 3 => {
            let Ok(inflated) = crate::codec::decode(crate::codec::Codec::Zlib, data) else {
                out.problem = Some("Stopped at gzip: the compressed data would not inflate.".into());
                return out;
            };
            out.steps.push(step("gzip", data.len(), inflated.len()));
            out.bytes = inflated;
            if compress & 0xff == 3 {
                if !integer {
                    out.problem = Some(format!(
                        "Stopped at differencing: the specification defines it for integers, and this vector holds {name}."
                    ));
                    return out;
                }
                sum(&mut out.bytes, word, little);
                out.steps.push(step("differencing", out.bytes.len(), out.bytes.len()));
            }
        }
        scheme @ (5 | 8 | 10) => {
            let packs = match scheme {
                5 => 2,
                8 => 4,
                _ => 8,
            };
            // The whole field in the message, 261 rather than 5, since that
            // is the number the tree shows beside it.
            if packs != word {
                let holds =
                    if complex { format!("{name} values have {word}-byte parts") } else { format!("values are {word}-byte {name}") };
                out.problem = Some(format!(
                    "Not unpacked: compress = {compress} is zero suppression of {packs}-byte words, and this vector's {holds}."
                ));
                return out;
            }
            match unsuppress(data, word, little, words, complex) {
                Ok(bytes) => {
                    out.steps.push(step("zero suppression", data.len(), bytes.len()));
                    out.bytes = bytes;
                }
                Err((bytes, why)) => {
                    out.bytes = bytes;
                    out.problem = Some(why);
                    return out;
                }
            }
            sum(&mut out.bytes, word, little);
            out.steps.push(step("differencing", out.bytes.len(), out.bytes.len()));
            if complex {
                out.bytes = interleave(&out.bytes, word);
                out.steps.push(step("interleave", out.bytes.len(), out.bytes.len()));
            }
        }
        0 => out.bytes = data.to_vec(),
        _ => {
            out.problem = Some(format!("Not unpacked: compress = {compress} is not a scheme this viewer undoes."));
            return out;
        }
    }
    out
}

fn step(what: &str, in_bytes: usize, out_bytes: usize) -> Step {
    Step { filter: what.to_string(), in_bytes, out_bytes, skipped: false }
}

/// Add the differences back up, in place, each word the running total of
/// itself and every word before it. The sums wrap, the way the subtraction
/// that made them did.
fn sum(bytes: &mut [u8], word: usize, little: bool) {
    let mask = if word == 8 { u64::MAX } else { (1u64 << (word * 8)) - 1 };
    let mut total = 0u64;
    for w in bytes.chunks_exact_mut(word) {
        total = total.wrapping_add(read(w, little)) & mask;
        write(w, total, little);
    }
}

fn read(b: &[u8], little: bool) -> u64 {
    let f = |acc: u64, x: &u8| acc << 8 | u64::from(*x);
    if little { b.iter().rev().fold(0, f) } else { b.iter().fold(0, f) }
}

fn write(b: &mut [u8], v: u64, little: bool) {
    let n = b.len();
    for (i, x) in b.iter_mut().enumerate() {
        let shift = if little { i } else { n - 1 - i } * 8;
        *x = (v >> shift) as u8;
    }
}

/// Unpack `count` zero-suppressed differences of `word` bytes each, which for a
/// complex vector are real and imaginary parts rather than values. On a run
/// that ends before `count` of them, the ones that were read come back with
/// the reason.
fn unsuppress(data: &[u8], word: usize, little: bool, count: usize, parts: bool) -> Result<Vec<u8>, (Vec<u8>, String)> {
    let mut out = vec![0u8; count * word];
    if count == 0 {
        return Ok(out);
    }
    if data.len() < 2 {
        return Err((Vec::new(), "Stopped at zero suppression: the data is under 2 bytes, too short for the block size that starts it.".into()));
    }
    let block = read(&data[..2], little) as usize;
    if block == 0 {
        return Err((Vec::new(), "Stopped at zero suppression: the block size is zero.".into()));
    }
    // The words the bits are packed in, turned into one little-endian run of
    // bytes, so that taking bits from the low end of each word is taking them
    // from the low end of each byte in order.
    let words = &data[2..];
    let bits: Vec<u8> = if little {
        words[..words.len() / word * word].to_vec()
    } else {
        words.chunks_exact(word).flat_map(|w| w.iter().rev().copied()).collect()
    };
    let mut reader = LowBits::low_first(&bits);
    let head = match word {
        1 => 3,
        2 => 4,
        4 => 5,
        _ => 6,
    };
    let width = word as u32 * 8;
    let mask = if width == 64 { u64::MAX } else { (1u64 << width) - 1 };
    let mut done = 0usize;
    while done < count {
        let Some(n) = reader.take(head) else { break };
        // The header is exactly wide enough to count to the word's width, so
        // no block can claim more bits than a word has.
        let n_bits = n as u32 + 1;
        let offset = (1u64 << (n_bits - 1)) - 1;
        for _ in 0..block.min(count - done) {
            let Some(code) = reader.take(n_bits) else { break };
            write(&mut out[done * word..(done + 1) * word], code.wrapping_sub(offset) & mask, little);
            done += 1;
        }
    }
    if done < count {
        out.truncate(done * word);
        let (done, of) = (crate::encode::commas(done as u64), crate::encode::commas(count as u64));
        let noun = if parts { "real and imaginary parts" } else { "values" };
        let why = format!("Stopped at zero suppression after {done} of {of} {noun}: the packed bits ran out.");
        return Err((out, why));
    }
    Ok(out)
}

/// All the real parts and then all the imaginary parts, put back as one pair
/// after another.
fn interleave(bytes: &[u8], part: usize) -> Vec<u8> {
    let half = bytes.len() / 2 / part * part;
    let (re, im) = bytes.split_at(half);
    let mut out = Vec::with_capacity(half * 2);
    for (r, i) in re.chunks_exact(part).zip(im.chunks_exact(part)) {
        out.extend_from_slice(r);
        out.extend_from_slice(i);
    }
    out
}

/// The first numbers of an unpacked vector, as text: what one is called, the
/// first [`SHOWN`] of them, and how many there are.
pub fn values(bytes: &[u8], vect_type: u16, little: bool) -> (String, Vec<String>, u64) {
    use crate::decode::{narrow_f32, read_int, read_uint};
    use crate::template::Endian;
    let Some((width, complex, integer, name)) = element(vect_type) else { return (String::new(), Vec::new(), 0) };
    let endian = if little { Endian::Little } else { Endian::Big };
    let signed = !matches!(vect_type, 9..=12);
    let one = |b: &[u8]| -> String {
        let bits = b.len() as u32 * 8;
        match (integer, bits) {
            (true, _) if signed => read_int(b, bits, endian).to_string(),
            (true, _) => read_uint(b, bits, endian).to_string(),
            (false, 32) => narrow_f32(f32::from_bits(read_uint(b, 32, endian) as u32)).to_string(),
            (false, _) => f64::from_bits(read_uint(b, 64, endian) as u64).to_string(),
        }
    };
    // A complex number as `re+imi`, the way it is written on paper.
    let pair = |b: &[u8]| {
        let (re, im) = (one(&b[..width / 2]), one(&b[width / 2..]));
        if im.starts_with('-') { format!("{re}{im}i") } else { format!("{re}+{im}i") }
    };
    let total = (bytes.len() / width) as u64;
    let shown = bytes.chunks_exact(width).take(SHOWN).map(|b| if complex { pair(b) } else { one(b) }).collect();
    (name.to_string(), shown, total)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Zero suppression the way the specification describes it, written out
    /// here from the same words so the unpacking can be checked for widths no
    /// sample holds. Not a copy of any library's packer.
    fn suppress(numbers: &[u64], word: usize, little: bool, block: usize) -> Vec<u8> {
        let width = word as u32 * 8;
        let mask = if width == 64 { u64::MAX } else { (1u64 << width) - 1 };
        let head = match word {
            2 => 4,
            4 => 5,
            _ => 6,
        };
        let mut previous = 0u64;
        let diffs: Vec<u64> = numbers
            .iter()
            .map(|&n| {
                let d = n.wrapping_sub(previous) & mask;
                previous = n;
                d
            })
            .collect();
        let mut bits: Vec<bool> = Vec::new();
        let put = |v: u64, n: u32, bits: &mut Vec<bool>| (0..n).for_each(|i| bits.push(v >> i & 1 == 1));
        for chunk in diffs.chunks(block) {
            // The fewest bits every difference in the block fits in, as a
            // signed number of that many bits with its offset added.
            let signed = |d: u64| if width == 64 { d as i64 as i128 } else { ((d << (64 - width)) as i64 >> (64 - width)) as i128 };
            let n_bits = (1..=width)
                .find(|&b| chunk.iter().all(|&d| {
                    let s = signed(d);
                    let lo = -((1i128 << (b - 1)) - 1);
                    let hi = 1i128 << (b - 1);
                    s >= lo && s <= hi
                }))
                .unwrap_or(width);
            put(u64::from(n_bits - 1), head, &mut bits);
            let offset = (1u64 << (n_bits - 1)) - 1;
            for &d in chunk {
                put(d.wrapping_add(offset) & mask, n_bits, &mut bits);
            }
        }
        let mut out = vec![0u8; 2];
        write(&mut out, block as u64, little);
        for w in bits.chunks(width as usize) {
            let v = w.iter().enumerate().fold(0u64, |acc, (i, b)| acc | (u64::from(*b) << i));
            let mut bytes = vec![0u8; word];
            write(&mut bytes, v, little);
            out.extend(bytes);
        }
        out
    }

    fn words(numbers: &[u64], word: usize, little: bool) -> Vec<u8> {
        let mut out = vec![0u8; numbers.len() * word];
        for (i, n) in numbers.iter().enumerate() {
            write(&mut out[i * word..(i + 1) * word], *n, little);
        }
        out
    }

    /// Appendix B's example, as the specification prints it: block size 3,
    /// then 16-bit words. The prose beside it says the first block is
    /// `(83 -3 -4)`, which is a typing slip for `(82 3 0)`: the bytes are
    /// what count, and they read back as the numbers it started from.
    #[test]
    fn the_specifications_own_example_reads_back_as_the_numbers_it_packed() {
        let packed: Vec<u8> = [0x0003u16, 0x2D17, 0x37f8, 0x2963, 0x0025].iter().flat_map(|w| w.to_le_bytes()).collect();
        let v = decode(&packed, 256 + 5, 1, 8);
        assert_eq!(v.problem, None);
        let got: Vec<i16> = v.bytes.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
        assert_eq!(got, vec![82, 85, 85, 81, 80, 82, 84, 85]);
        let steps: Vec<&str> = v.steps.iter().map(|s| s.filter.as_str()).collect();
        assert_eq!(steps, vec!["zero suppression", "differencing"]);
        // Written by a big-endian machine, the words are the other way round
        // and nothing else changes.
        let packed: Vec<u8> = [0x0003u16, 0x2D17, 0x37f8, 0x2963, 0x0025].iter().flat_map(|w| w.to_be_bytes()).collect();
        let v = decode(&packed, 5, 1, 8);
        let got: Vec<i16> = v.bytes.chunks_exact(2).map(|b| i16::from_be_bytes([b[0], b[1]])).collect();
        assert_eq!(got, vec![82, 85, 85, 81, 80, 82, 84, 85]);
    }

    #[test]
    fn four_and_eight_byte_words_come_back_as_they_went_in() {
        let ints: Vec<u64> = (0..1000u64).map(|i| (i * i * 7919 % 100_003) as u32 as u64 | if i % 97 == 0 { 0x8000_0000 } else { 0 }).collect();
        for little in [true, false] {
            let v = decode(&suppress(&ints, 4, little, 16), if little { 264 } else { 8 }, 10, ints.len() as u64);
            assert_eq!(v.problem, None);
            assert_eq!(v.bytes, words(&ints, 4, little));
        }
        let doubles: Vec<u64> = (0..500).map(|i| (f64::from(i) * 0.01).sin().to_bits()).collect();
        let v = decode(&suppress(&doubles, 8, true, 12), 266, 2, doubles.len() as u64);
        assert_eq!(v.problem, None);
        assert_eq!(v.bytes, words(&doubles, 8, true));
    }

    /// A complex vector goes in as every real part and then every imaginary
    /// part, and comes out as pairs.
    #[test]
    fn a_complex_vector_is_paired_back_up_after_it_is_unpacked() {
        let pairs = [(1.5f32, -2.0f32), (0.25, 8.0), (-3.0, 0.5)];
        let mut run: Vec<u64> = pairs.iter().map(|p| u64::from(p.0.to_bits())).collect();
        run.extend(pairs.iter().map(|p| u64::from(p.1.to_bits())));
        let v = decode(&suppress(&run, 4, true, 4), 264, 6, 3);
        assert_eq!(v.problem, None);
        let want: Vec<u8> = pairs.iter().flat_map(|p| [p.0.to_le_bytes(), p.1.to_le_bytes()].concat()).collect();
        assert_eq!(v.bytes, want);
        assert_eq!(v.steps.last().unwrap().filter, "interleave");
        let (name, shown, total) = values(&v.bytes, 6, true);
        assert_eq!((name.as_str(), total), ("complex f32", 3));
        assert_eq!(shown[0], "1.5-2i");
    }

    #[test]
    fn differences_then_gzip_is_inflated_and_then_summed() {
        let numbers: Vec<u64> = (0..300u64).map(|i| (1000 + i * 3) & 0xffff).collect();
        let mut previous = 0u64;
        let diffs: Vec<u64> = numbers.iter().map(|&n| { let d = n.wrapping_sub(previous) & 0xffff; previous = n; d }).collect();
        let packed = miniz_oxide::deflate::compress_to_vec_zlib(&words(&diffs, 2, true), 6);
        let v = decode(&packed, 259, 1, 300);
        assert_eq!(v.problem, None);
        assert_eq!(v.bytes, words(&numbers, 2, true));
        let steps: Vec<(&str, usize, usize)> = v.steps.iter().map(|s| (s.filter.as_str(), s.in_bytes, s.out_bytes)).collect();
        assert_eq!(steps, vec![("gzip", packed.len(), 600), ("differencing", 600, 600)]);
    }

    /// The count comes from outside the run, and the run's last word is
    /// padded: a run that ends early says how far it got.
    #[test]
    fn a_run_that_ends_before_its_count_says_how_far_it_got() {
        let numbers: Vec<u64> = (0..40).collect();
        let packed = suppress(&numbers, 2, true, 8);
        let v = decode(&packed, 261, 1, 400);
        let problem = v.problem.expect("a problem");
        assert!(problem.contains("of 400 values"), "{problem}");
        assert!(v.bytes.len() >= 80);
    }

    #[test]
    fn a_scheme_for_one_width_on_a_vector_of_another_is_not_unpacked() {
        let v = decode(&[3, 0, 0, 0], 261, 2, 1);
        assert!(v.problem.expect("a problem").contains("compress = 261 is zero suppression of 2-byte words, and this vector's values are 8-byte f64"));
        assert!(v.steps.is_empty());
    }
}
