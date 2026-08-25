//! Taking a ggml block apart into the numbers it stands for.
//!
//! [`ggml`](super::ggml) says where every block and every scale is, and leaves
//! the packed weights as bytes, because that is what a template can say. This
//! is the other half: given one block's bytes, the weights that come out of it,
//! each as the small integer the file holds and as the number the model reads.
//!
//! The packing is not a run of bit-fields, which is why no template describes
//! it. A `q4_0` block holds weight 0 in the *low* nibble of `qs[0]` and weight
//! 16 in the high nibble of the same byte, so the file's order and the tensor's
//! order are not the same order. The K types are stranger still: `q4_k` packs
//! eight six-bit scales and eight six-bit minimums into twelve bytes, four bits
//! of each in one place and two in another.
//!
//! Everything here is transcribed from ggml's own `dequantize_row_*`, which is
//! the only definition of these layouts there is.

use crate::decode::f16_to_f64;

/// A block layout whose weights this module can unpack. The IQ types and the
/// ternary ones are left out: their weights come from lookup tables that only
/// ggml has, so a block of one of those is the right size and opaque inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// ggml's own names for these, which is what the file and the field tree say.
#[allow(non_camel_case_types)]
pub enum Quant {
    Q4_0,
    Q4_1,
    Q5_0,
    Q5_1,
    Q8_0,
    Q2_K,
    Q3_K,
    Q4_K,
    Q5_K,
    Q6_K,
    Q8_K,
}

impl Quant {
    /// The name ggml's own block struct goes by, which is what the field tree
    /// shows for the block.
    pub fn name(self) -> &'static str {
        match self {
            Quant::Q4_0 => "Q4_0",
            Quant::Q4_1 => "Q4_1",
            Quant::Q5_0 => "Q5_0",
            Quant::Q5_1 => "Q5_1",
            Quant::Q8_0 => "Q8_0",
            Quant::Q2_K => "Q2_K",
            Quant::Q3_K => "Q3_K",
            Quant::Q4_K => "Q4_K",
            Quant::Q5_K => "Q5_K",
            Quant::Q6_K => "Q6_K",
            Quant::Q8_K => "Q8_K",
        }
    }

    /// How many weights one block holds.
    pub fn weights(self) -> usize {
        match self {
            Quant::Q4_0 | Quant::Q4_1 | Quant::Q5_0 | Quant::Q5_1 | Quant::Q8_0 => 32,
            _ => 256,
        }
    }

    /// How many bytes one block takes, which is `sizeof` ggml's struct.
    pub fn block_bytes(self) -> usize {
        match self {
            Quant::Q4_0 => 18,
            Quant::Q4_1 => 20,
            Quant::Q5_0 => 22,
            Quant::Q5_1 => 24,
            Quant::Q8_0 => 34,
            Quant::Q2_K => 84,
            Quant::Q3_K => 110,
            Quant::Q4_K => 144,
            Quant::Q5_K => 176,
            Quant::Q6_K => 210,
            Quant::Q8_K => 292,
        }
    }

    /// How many bits one weight is worth, not counting the block's share of
    /// the scales. The name of the type says it.
    pub fn bits(self) -> u32 {
        match self {
            Quant::Q2_K => 2,
            Quant::Q3_K => 3,
            Quant::Q4_0 | Quant::Q4_1 | Quant::Q4_K => 4,
            Quant::Q5_0 | Quant::Q5_1 | Quant::Q5_K => 5,
            Quant::Q6_K => 6,
            Quant::Q8_0 | Quant::Q8_K => 8,
        }
    }
}

/// The layout of that name, for the block structures the ggml template builds.
/// Anything else is a block this module leaves alone.
pub fn by_name(name: &str) -> Option<Quant> {
    Some(match name {
        "Q4_0" => Quant::Q4_0,
        "Q4_1" => Quant::Q4_1,
        "Q5_0" => Quant::Q5_0,
        "Q5_1" => Quant::Q5_1,
        "Q8_0" => Quant::Q8_0,
        "Q2_K" => Quant::Q2_K,
        "Q3_K" => Quant::Q3_K,
        "Q4_K" => Quant::Q4_K,
        "Q5_K" => Quant::Q5_K,
        "Q6_K" => Quant::Q6_K,
        "Q8_K" => Quant::Q8_K,
        _ => return None,
    })
}

/// One run of bits that makes up part of a packed weight.
///
/// A five-bit weight is not five bits in a row. Four of them are a nibble of
/// `qs` and the fifth is one bit of `qh`, a long way off, and a reader looking
/// at the nibble has no way to see where the fifth came from. This says where
/// each part is and what it contributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Part {
    /// The block's own field these bits are in, as the file names it: `qs`,
    /// `qh`, `ql`, `hmask`.
    pub field: &'static str,
    /// Where they are, counted in bits from the start of the block.
    pub bit: u32,
    pub width: u32,
    /// Where they sit in the packed value: 0 for the low part, 4 for the fifth
    /// bit of a `q5_1`.
    pub shift: u32,
}

/// One weight, as the file holds it and as the model reads it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weight {
    /// The stored integer, after whatever the type subtracts from it: a `q4_0`
    /// nibble runs 0 to 15 in the file and −8 to 7 here, because that is the
    /// number the scale multiplies.
    pub q: i32,
    /// The number the model reads: the stored integer through this block's
    /// scale and minimum.
    pub value: f64,
    /// The run holding the low bits, which is the one a reader lands on.
    pub bits: Part,
    /// The rest of the packed value, for a type that keeps its top bits
    /// somewhere else in the block.
    pub high: Option<Part>,
}

/// A run of weights inside a block that share a scale of their own. Only the K
/// types have these: they spend twelve or sixteen bytes on six-bit scales, one
/// per sixteen or thirty-two weights, and the block's own `d` is what those are
/// measured in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Group {
    /// The scale as stored, after whatever bias the type takes off it.
    pub scale: i32,
    /// The minimum taken off every weight in the group, where the type has one.
    pub min: Option<i32>,
}

/// What a layout pairs with its scale, and how it applies it. Enough to write
/// out how a stored integer becomes the number the model reads, rather than
/// leaving that to be inferred from the name.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Offset {
    /// The file's own name for it: `m`, `dmin`.
    pub name: &'static str,
    pub value: f64,
    /// Taken away from the scaled weight rather than added to it.
    pub subtract: bool,
    /// Multiplied by the group's own minimum first, which is what a K type
    /// does; without this it applies to every weight in the block alike.
    pub per_group: bool,
}

/// One block's numbers: its shared scale, whatever it pairs with the scale, and
/// the weights.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub kind: Quant,
    /// The scale every weight in the block is multiplied by. For the K types
    /// this is the block's own scale, which the six-bit per-group scales are
    /// themselves measured in.
    pub d: f64,
    /// What the type pairs with the scale, where it has one: the `m` a `q4_1`
    /// adds, or the `dmin` a K type takes away.
    pub second: Option<Offset>,
    /// The per-group scales, in the order the groups run, or empty for a type
    /// that has one scale for the whole block. Group `i` covers the weights
    /// from `i * group_weights`.
    pub groups: Vec<Group>,
    pub group_weights: u32,
    /// Taken off the packed value to get the stored one: 8 for a `q4_0`, 32 for
    /// a `q6_k`, and 0 for a type that reads its weights as they are.
    pub bias: i32,
    /// The packed value is read signed, which is what the eight-bit types do
    /// instead of biasing.
    pub signed: bool,
    pub weights: Vec<Weight>,
}

/// The half-precision number at `b[i..i + 2]`, which is how every one of these
/// blocks writes its scale.
fn f16(b: &[u8], i: usize) -> f64 {
    f16_to_f64(u16::from_le_bytes([b[i], b[i + 1]]))
}

fn f32le(b: &[u8], i: usize) -> f64 {
    f32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as f64
}

/// The bit run of the low nibble of block byte `i`, counting bits from the top
/// of each byte the way the rest of the editor does.
fn lo_nibble(i: usize) -> (u32, u32) {
    low_bits(i, 0, 4)
}

fn hi_nibble(i: usize) -> (u32, u32) {
    low_bits(i, 4, 4)
}

/// The `width` bits of block byte `i` whose lowest is `lsb` places up from the
/// bottom of the byte. ggml counts bits from the bottom; the editor counts them
/// from the top, so this is where the two meet.
fn low_bits(i: usize, lsb: u32, width: u32) -> (u32, u32) {
    (i as u32 * 8 + (8 - lsb - width), width)
}

/// Bit `k` of the little-endian `u32` at block byte `base`, which is how the
/// five-bit types keep their fifth bits.
fn u32_bit(base: usize, k: u32) -> (u32, u32) {
    low_bits(base + (k / 8) as usize, k % 8, 1)
}

/// One block's weights, or `None` if `b` is shorter than the block.
///
/// Transcribed from ggml's `dequantize_row_*`. Where ggml writes into `y` in
/// tensor order, this pushes in the same order, so the index of a weight here
/// is its index in the tensor's row.
pub fn unpack(kind: Quant, b: &[u8]) -> Option<Block> {
    if b.len() < kind.block_bytes() {
        return None;
    }
    Some(match kind {
        Quant::Q4_0 => {
            let d = f16(b, 0);
            let qs = &b[2..18];
            let mut w = vec![blank(); 32];
            for j in 0..16 {
                let (q0, q1) = ((qs[j] & 0x0F) as i32 - 8, (qs[j] >> 4) as i32 - 8);
                w[j] = weight(q0, q0 as f64 * d, part("qs", lo_nibble(2 + j), 0));
                w[j + 16] = weight(q1, q1 as f64 * d, part("qs", hi_nibble(2 + j), 0));
            }
            Block { kind, d, second: None, groups: Vec::new(), group_weights: 0, bias: 8, signed: false, weights: w }
        }
        Quant::Q4_1 => {
            let (d, m) = (f16(b, 0), f16(b, 2));
            let qs = &b[4..20];
            let mut w = vec![blank(); 32];
            for j in 0..16 {
                let (q0, q1) = ((qs[j] & 0x0F) as i32, (qs[j] >> 4) as i32);
                w[j] = weight(q0, q0 as f64 * d + m, part("qs", lo_nibble(4 + j), 0));
                w[j + 16] = weight(q1, q1 as f64 * d + m, part("qs", hi_nibble(4 + j), 0));
            }
            Block { kind, d, second: Some(Offset { name: "m", value: m, subtract: false, per_group: false }), groups: Vec::new(), group_weights: 0, bias: 0, signed: false, weights: w }
        }
        Quant::Q5_0 => {
            let d = f16(b, 0);
            let qh = u32::from_le_bytes([b[2], b[3], b[4], b[5]]);
            let qs = &b[6..22];
            let mut w = vec![blank(); 32];
            for j in 0..16 {
                let h0 = ((qh >> j) << 4) as u8 & 0x10;
                let h1 = (qh >> (j + 12)) as u8 & 0x10;
                let q0 = ((qs[j] & 0x0F) | h0) as i32 - 16;
                let q1 = ((qs[j] >> 4) | h1) as i32 - 16;
                let k = j as u32;
                w[j] = split(q0, q0 as f64 * d, part("qs", lo_nibble(6 + j), 0), part("qh", u32_bit(2, k), 4));
                w[j + 16] = split(q1, q1 as f64 * d, part("qs", hi_nibble(6 + j), 0), part("qh", u32_bit(2, k + 16), 4));
            }
            Block { kind, d, second: None, groups: Vec::new(), group_weights: 0, bias: 16, signed: false, weights: w }
        }
        Quant::Q5_1 => {
            let (d, m) = (f16(b, 0), f16(b, 2));
            let qh = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
            let qs = &b[8..24];
            let mut w = vec![blank(); 32];
            for j in 0..16 {
                let h0 = ((qh >> j) << 4) as u8 & 0x10;
                let h1 = (qh >> (j + 12)) as u8 & 0x10;
                let q0 = ((qs[j] & 0x0F) | h0) as i32;
                let q1 = ((qs[j] >> 4) | h1) as i32;
                let k = j as u32;
                w[j] = split(q0, q0 as f64 * d + m, part("qs", lo_nibble(8 + j), 0), part("qh", u32_bit(4, k), 4));
                w[j + 16] = split(q1, q1 as f64 * d + m, part("qs", hi_nibble(8 + j), 0), part("qh", u32_bit(4, k + 16), 4));
            }
            Block { kind, d, second: Some(Offset { name: "m", value: m, subtract: false, per_group: false }), groups: Vec::new(), group_weights: 0, bias: 0, signed: false, weights: w }
        }
        Quant::Q8_0 => {
            let d = f16(b, 0);
            let w = (0..32)
                .map(|j| {
                    let q = b[2 + j] as i8 as i32;
                    weight(q, q as f64 * d, part("qs", ((2 + j) as u32 * 8, 8), 0))
                })
                .collect();
            Block { kind, d, second: None, groups: Vec::new(), group_weights: 0, bias: 0, signed: true, weights: w }
        }
        Quant::Q8_K => {
            let d = f32le(b, 0);
            let w = (0..256)
                .map(|j| {
                    let q = b[4 + j] as i8 as i32;
                    weight(q, q as f64 * d, part("qs", ((4 + j) as u32 * 8, 8), 0))
                })
                .collect();
            Block { kind, d, second: None, groups: Vec::new(), group_weights: 0, bias: 0, signed: true, weights: w }
        }
        Quant::Q2_K => {
            // scales[16], qs[64], d, dmin
            let (d, dmin) = (f16(b, 80), f16(b, 82));
            let scales = &b[0..16];
            let qs = &b[16..80];
            let mut w = Vec::with_capacity(256);
            let mut is = 0usize;
            for n in [0usize, 128] {
                let q = &qs[n / 4..];
                let qbase = 16 + n / 4;
                for j in 0..4u32 {
                    let shift = 2 * j;
                    for half in 0..2usize {
                        let sc = scales[is];
                        is += 1;
                        let (dl, ml) = (d * (sc & 0xF) as f64, dmin * (sc >> 4) as f64);
                        for l in 0..16usize {
                            let at = half * 16 + l;
                            let qv = ((q[at] >> shift) & 3) as i32;
                            w.push(weight(qv, dl * qv as f64 - ml, part("qs", low_bits(qbase + at, shift, 2), 0)));
                        }
                    }
                }
            }
            let groups = scales
                .iter()
                .map(|&sc| Group { scale: i32::from(sc & 0xF), min: Some(i32::from(sc >> 4)) })
                .collect();
            Block { kind, d, second: Some(Offset { name: "dmin", value: dmin, subtract: true, per_group: true }), groups, group_weights: 16, bias: 0, signed: false, weights: w }
        }
        Quant::Q3_K => {
            // hmask[32], qs[64], scales[12], d
            let d = f16(b, 108);
            let hm = &b[0..32];
            let qs = &b[32..96];
            let scales = k3_scales(&b[96..108]);
            let mut w = Vec::with_capacity(256);
            let mut is = 0usize;
            let mut m: u8 = 1;
            let mut mbit: u32 = 0;
            for n in [0usize, 128] {
                let q = &qs[n / 4..];
                let qbase = 32 + n / 4;
                for j in 0..4u32 {
                    let shift = 2 * j;
                    for half in 0..2usize {
                        let dl = d * (scales[is] as f64 - 32.0);
                        is += 1;
                        for l in 0..16usize {
                            let at = half * 16 + l;
                            let low = ((q[at] >> shift) & 3) as i32;
                            // A mask bit set means nothing is taken off, so it
                            // is the third bit of a value biased by four.
                            let qv = low - if hm[at] & m != 0 { 0 } else { 4 };
                            w.push(split(
                                qv,
                                dl * qv as f64,
                                part("qs", low_bits(qbase + at, shift, 2), 0),
                                part("hmask", low_bits(at, mbit, 1), 2),
                            ));
                        }
                    }
                    m <<= 1;
                    mbit += 1;
                }
            }
            let groups = scales.iter().map(|&sc| Group { scale: i32::from(sc) - 32, min: None }).collect();
            Block { kind, d, second: None, groups, group_weights: 16, bias: 4, signed: false, weights: w }
        }
        Quant::Q4_K => {
            // d, dmin, scales[12], qs[128]
            let (d, dmin) = (f16(b, 0), f16(b, 2));
            let scales = &b[4..16];
            let qs = &b[16..144];
            let mut w = Vec::with_capacity(256);
            for g in 0..4usize {
                let q = &qs[32 * g..32 * g + 32];
                let qbase = 16 + 32 * g;
                let (sc1, m1) = scale_min_k4(2 * g, scales);
                let (sc2, m2) = scale_min_k4(2 * g + 1, scales);
                let (d1, off1) = (d * sc1 as f64, dmin * m1 as f64);
                let (d2, off2) = (d * sc2 as f64, dmin * m2 as f64);
                for l in 0..32usize {
                    let qv = (q[l] & 0xF) as i32;
                    w.push(weight(qv, d1 * qv as f64 - off1, part("qs", lo_nibble(qbase + l), 0)));
                }
                for l in 0..32usize {
                    let qv = (q[l] >> 4) as i32;
                    w.push(weight(qv, d2 * qv as f64 - off2, part("qs", hi_nibble(qbase + l), 0)));
                }
            }
            Block { kind, d, second: Some(Offset { name: "dmin", value: dmin, subtract: true, per_group: true }), groups: k4_groups(scales), group_weights: 32, bias: 0, signed: false, weights: w }
        }
        Quant::Q5_K => {
            // d, dmin, scales[12], qh[32], qs[128]
            let (d, dmin) = (f16(b, 0), f16(b, 2));
            let scales = &b[4..16];
            let qh = &b[16..48];
            let ql = &b[48..176];
            let mut w = Vec::with_capacity(256);
            for g in 0..4usize {
                let q = &ql[32 * g..32 * g + 32];
                let qbase = 48 + 32 * g;
                let (sc1, m1) = scale_min_k4(2 * g, scales);
                let (sc2, m2) = scale_min_k4(2 * g + 1, scales);
                let (d1, off1) = (d * sc1 as f64, dmin * m1 as f64);
                let (d2, off2) = (d * sc2 as f64, dmin * m2 as f64);
                let (u1, u2) = (1u8 << (2 * g), 2u8 << (2 * g));
                for l in 0..32usize {
                    let qv = (q[l] & 0xF) as i32 + if qh[l] & u1 != 0 { 16 } else { 0 };
                    w.push(split(
                        qv,
                        d1 * qv as f64 - off1,
                        part("qs", lo_nibble(qbase + l), 0),
                        part("qh", low_bits(16 + l, 2 * g as u32, 1), 4),
                    ));
                }
                for l in 0..32usize {
                    let qv = (q[l] >> 4) as i32 + if qh[l] & u2 != 0 { 16 } else { 0 };
                    w.push(split(
                        qv,
                        d2 * qv as f64 - off2,
                        part("qs", hi_nibble(qbase + l), 0),
                        part("qh", low_bits(16 + l, 2 * g as u32 + 1, 1), 4),
                    ));
                }
            }
            Block { kind, d, second: Some(Offset { name: "dmin", value: dmin, subtract: true, per_group: true }), groups: k4_groups(scales), group_weights: 32, bias: 0, signed: false, weights: w }
        }
        Quant::Q6_K => {
            // ql[128], qh[64], scales[16] (signed), d
            let d = f16(b, 208);
            let ql = &b[0..128];
            let qh = &b[128..192];
            let sc = &b[192..208];
            let mut w = vec![blank(); 256];
            for n in 0..2usize {
                let (qlb, a, c) = (64 * n, &ql[64 * n..], 128 * n);
                for l in 0..32usize {
                    let is = l / 16;
                    let h = qh[32 * n + l];
                    // Four weights share one byte of `qh`, two high bits each,
                    // and they are 32 apart in the row.
                    let hbyte = 128 + 32 * n + l;
                    let parts = [
                        ((a[l] & 0xF) as i32 | ((h & 3) as i32) << 4, is, 0usize, lo_nibble(qlb + l), 0u32),
                        ((a[l + 32] & 0xF) as i32 | (((h >> 2) & 3) as i32) << 4, is + 2, 32, lo_nibble(qlb + l + 32), 2),
                        ((a[l] >> 4) as i32 | (((h >> 4) & 3) as i32) << 4, is + 4, 64, hi_nibble(qlb + l), 4),
                        ((a[l + 32] >> 4) as i32 | (((h >> 6) & 3) as i32) << 4, is + 6, 96, hi_nibble(qlb + l + 32), 6),
                    ];
                    for (raw, si, at, src, hlsb) in parts {
                        let q = raw - 32;
                        let s = sc[8 * n + si] as i8 as f64;
                        w[c + at + l] =
                            split(q, d * s * q as f64, part("ql", src, 0), part("qh", low_bits(hbyte, hlsb, 2), 4));
                    }
                }
            }
            let groups = sc.iter().map(|&s| Group { scale: i32::from(s as i8), min: None }).collect();
            Block { kind, d, second: None, groups, group_weights: 16, bias: 32, signed: false, weights: w }
        }
    })
}

/// The eight scales and eight minimums a `q4_k` or `q5_k` packs into twelve
/// bytes, in the order the groups run.
fn k4_groups(scales: &[u8]) -> Vec<Group> {
    (0..8)
        .map(|j| {
            let (sc, m) = scale_min_k4(j, scales);
            Group { scale: i32::from(sc), min: Some(i32::from(m)) }
        })
        .collect()
}

fn part(field: &'static str, (bit, width): (u32, u32), shift: u32) -> Part {
    Part { field, bit, width, shift }
}

fn weight(q: i32, value: f64, bits: Part) -> Weight {
    Weight { q, value, bits, high: None }
}

/// A weight whose bits are in two places at once.
fn split(q: i32, value: f64, bits: Part, high: Part) -> Weight {
    Weight { q, value, bits, high: Some(high) }
}

/// A placeholder for the arms that fill their weights out of order.
fn blank() -> Weight {
    Weight { q: 0, value: 0.0, bits: Part { field: "", bit: 0, width: 0, shift: 0 }, high: None }
}

/// ggml's `get_scale_min_k4`: the six-bit scale and six-bit minimum of group
/// `j`, out of the twelve bytes a K block spends on all sixteen of them. The
/// first four of each fit in a byte; the last four are split, four bits in the
/// low half of one byte and two in the top of another.
fn scale_min_k4(j: usize, q: &[u8]) -> (u8, u8) {
    if j < 4 {
        (q[j] & 63, q[j + 4] & 63)
    } else {
        ((q[j + 4] & 0xF) | ((q[j - 4] >> 6) << 4), (q[j + 4] >> 4) | ((q[j] >> 6) << 4))
    }
}

/// ggml's unpacking of a `q3_k` block's twelve scale bytes into sixteen
/// six-bit scales, which are read biased by 32.
fn k3_scales(s: &[u8]) -> [u8; 16] {
    const KMASK1: u32 = 0x0303_0303;
    const KMASK2: u32 = 0x0f0f_0f0f;
    let word = |i: usize| u32::from_le_bytes([s[4 * i], s[4 * i + 1], s[4 * i + 2], s[4 * i + 3]]);
    let (a0, a1, tmp) = (word(0), word(1), word(2));
    let aux = [
        (a0 & KMASK2) | ((tmp & KMASK1) << 4),
        (a1 & KMASK2) | (((tmp >> 2) & KMASK1) << 4),
        ((a0 >> 4) & KMASK2) | (((tmp >> 4) & KMASK1) << 4),
        ((a1 >> 4) & KMASK2) | (((tmp >> 6) & KMASK1) << 4),
    ];
    let mut out = [0u8; 16];
    for (i, v) in aux.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The half-precision bits of 1.0, so a block's weights come out as the
    /// integers themselves and the arithmetic is checkable by eye.
    const ONE: [u8; 2] = [0x00, 0x3C];

    #[test]
    fn q4_0_splits_a_byte_into_two_weights() {
        let mut b = vec![0u8; 18];
        b[0..2].copy_from_slice(&ONE);
        // Low nibble is weight 0, high nibble is weight 16, both biased by 8.
        b[2] = 0x9A; // low 0xA = 10 -> 2, high 0x9 = 9 -> 1
        let block = unpack(Quant::Q4_0, &b).unwrap();
        assert_eq!(block.d, 1.0);
        assert_eq!(block.weights[0].q, 2);
        assert_eq!(block.weights[16].q, 1);
        assert_eq!(block.weights[0].value, 2.0);
        // Every other nibble is zero, which is −8 once the bias comes off.
        assert_eq!(block.weights[1].q, -8);
        // The low nibble is the bottom half of the byte, so its bits start
        // four in.
        assert_eq!((block.weights[0].bits.bit, block.weights[0].bits.width), (2 * 8 + 4, 4));
        assert_eq!(block.weights[16].bits.bit, 2 * 8);
    }

    #[test]
    fn q5_1_takes_its_fifth_bit_from_qh() {
        let mut b = vec![0u8; 24];
        b[0..2].copy_from_slice(&ONE); // d
        b[2..4].copy_from_slice(&[0, 0]); // m = 0
        // qh bit 0 is weight 0's fifth bit; bit 16 is weight 16's.
        b[4..8].copy_from_slice(&0x0001_0001u32.to_le_bytes());
        b[8] = 0x21; // low 1, high 2
        let block = unpack(Quant::Q5_1, &b).unwrap();
        assert_eq!(block.weights[0].q, 1 + 16);
        assert_eq!(block.weights[16].q, 2 + 16);
        assert_eq!(block.weights[1].q, 0);
        assert_eq!(block.second.map(|o| (o.name, o.value, o.subtract)), Some(("m", 0.0, false)));
        // The fifth bit is bit 0 of the u32 at block byte 4, which is the low
        // bit of that byte and so the last of its eight.
        let h = block.weights[0].high.expect("a fifth bit");
        assert_eq!((h.field, h.bit, h.width, h.shift), ("qh", 4 * 8 + 7, 1, 4));
        // Weight 16's is bit 16 of the same u32: two bytes along.
        let h16 = block.weights[16].high.expect("a fifth bit");
        assert_eq!(h16.bit, 6 * 8 + 7);
        // And weight 13's, which is bit 13: byte 5, five places up from the
        // bottom, so third from the top.
        let mut b13 = vec![0u8; 24];
        b13[0..2].copy_from_slice(&ONE);
        b13[4..8].copy_from_slice(&(1u32 << 13).to_le_bytes());
        b13[8 + 13] = 0x0B;
        let block13 = unpack(Quant::Q5_1, &b13).unwrap();
        assert_eq!(block13.weights[13].q, 0x0B + 16);
        assert_eq!(block13.weights[13].high.unwrap().bit, 5 * 8 + 2);
    }

    #[test]
    fn q4_k_reads_a_group_scale_and_minimum() {
        let mut b = vec![0u8; 144];
        b[0..2].copy_from_slice(&ONE); // d
        b[2..4].copy_from_slice(&ONE); // dmin
        // Group 0's scale is scales[0] & 63 and its minimum scales[4] & 63.
        b[4] = 3;
        b[8] = 1;
        b[16] = 0x05; // weight 0 is the low nibble
        let block = unpack(Quant::Q4_K, &b).unwrap();
        assert_eq!(block.weights.len(), 256);
        assert_eq!(block.weights[0].q, 5);
        assert_eq!(block.weights[0].value, 5.0 * 3.0 - 1.0);
        // Weight 32 is the high nibble of the same byte, on group 0's second
        // scale, which is zero here.
        assert_eq!(block.weights[32].q, 0);
        assert_eq!(block.weights[32].bits.bit, 16 * 8);
    }

    #[test]
    fn k4_scale_unpacking_matches_ggml() {
        // The split layout: group 4's scale is the low nibble of byte 8 with
        // the top two bits of byte 0 above it.
        let mut q = [0u8; 12];
        q[0] = 0b1100_0000;
        q[8] = 0x0A;
        assert_eq!(scale_min_k4(4, &q).0, 0x0A | (0b11 << 4));
    }

    #[test]
    fn q6_k_biases_by_thirty_two() {
        let mut b = vec![0u8; 210];
        b[208..210].copy_from_slice(&ONE);
        b[192] = 1; // scales[0]
        b[0] = 0x0F; // ql[0] low nibble
        let block = unpack(Quant::Q6_K, &b).unwrap();
        assert_eq!(block.weights[0].q, 15 - 32);
        assert_eq!(block.weights[0].value, -17.0);
    }

    /// The weights of a block tile the field that holds them: every bit of it
    /// belongs to exactly one weight, and none of them reaches outside. An
    /// index off by one anywhere in the unpacking shows up here.
    #[test]
    fn the_weights_of_a_block_tile_its_quants() {
        for kind in ALL {
            let b = vec![0x5Au8; kind.block_bytes()];
            let block = unpack(kind, &b).unwrap();
            let mut claimed = vec![0u8; kind.block_bytes() * 8];
            for w in &block.weights {
                for bit in w.bits.bit..w.bits.bit + w.bits.width {
                    claimed[bit as usize] += 1;
                }
            }
            let first = claimed.iter().position(|&c| c > 0).expect("some bits");
            let last = claimed.iter().rposition(|&c| c > 0).expect("some bits");
            assert!(claimed[first..=last].iter().all(|&c| c == 1), "{} claims a bit twice or leaves a hole", kind.name());
            // The run is as wide as the weights are: four bits apiece for the
            // types that keep a fifth or sixth bit elsewhere.
            let per = match kind {
                Quant::Q5_0 | Quant::Q5_1 | Quant::Q5_K | Quant::Q6_K => 4,
                Quant::Q3_K => 2,
                other => other.bits(),
            };
            assert_eq!(last + 1 - first, kind.weights() * per as usize, "{}", kind.name());
        }
    }

    /// Every group covers the same number of weights, and they run in the same
    /// order the weights do, so group `i` is the scale of the weights starting
    /// at `i * group_weights`.
    #[test]
    fn the_groups_of_a_k_block_cover_its_weights() {
        for kind in ALL {
            let block = unpack(kind, &vec![0x5Au8; kind.block_bytes()]).unwrap();
            if block.groups.is_empty() {
                assert_eq!(block.group_weights, 0, "{}", kind.name());
                assert!(!kind.name().ends_with("_K") || kind == Quant::Q8_K, "{} should group", kind.name());
                continue;
            }
            assert_eq!(block.groups.len() * block.group_weights as usize, kind.weights(), "{}", kind.name());
        }
    }

    /// A `q4_k` group past the fourth takes four bits of its scale from one
    /// byte and two from another, which is the part of `get_scale_min_k4` most
    /// worth pinning down.
    #[test]
    fn a_split_q4_k_group_reads_both_halves() {
        let mut b = vec![0u8; 144];
        b[0..2].copy_from_slice(&ONE);
        b[2..4].copy_from_slice(&ONE);
        // scales[] starts at byte 4. Group 5: scale is the low nibble of
        // scales[9] with the top two bits of scales[1] above it; the minimum is
        // the high nibble of scales[9] with the top two bits of scales[5].
        b[4 + 9] = 0xA3;
        b[4 + 1] = 0b0100_0000;
        b[4 + 5] = 0b1000_0000;
        let block = unpack(Quant::Q4_K, &b).unwrap();
        let g = block.groups[5];
        assert_eq!(g.scale, 0x03 | (0b01 << 4));
        assert_eq!(g.min, Some(0x0A | (0b10 << 4)));
        // The first four groups sit whole in one byte each.
        assert_eq!(block.groups[1].scale, 0);
        assert_eq!(block.groups[1].min, Some(0));
    }

    #[test]
    fn a_short_block_is_no_block() {
        assert!(unpack(Quant::Q4_K, &[0u8; 100]).is_none());
    }

    const ALL: [Quant; 11] = [
        Quant::Q4_0,
        Quant::Q4_1,
        Quant::Q5_0,
        Quant::Q5_1,
        Quant::Q8_0,
        Quant::Q2_K,
        Quant::Q3_K,
        Quant::Q4_K,
        Quant::Q5_K,
        Quant::Q6_K,
        Quant::Q8_K,
    ];

    #[test]
    fn every_kind_fills_its_weights() {
        for kind in ALL {
            let b = vec![0x5Au8; kind.block_bytes()];
            let block = unpack(kind, &b).unwrap();
            assert_eq!(block.weights.len(), kind.weights(), "{}", kind.name());
            // Every weight says where it came from, and inside the block.
            for w in &block.weights {
                assert!(w.bits.bit + w.bits.width <= kind.block_bytes() as u32 * 8, "{}", kind.name());
                if let Some(h) = w.high {
                    assert!(h.bit + h.width <= kind.block_bytes() as u32 * 8, "{}", kind.name());
                }
            }
        }
    }
}
