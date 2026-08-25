//! What a type knows beyond the value in front of it: the values an enum
//! names, the bytes a magic field wanted, what each bit of a flags field means,
//! and which bits of a float are which.

use super::*;
use crate::formats::ggml_quant::{self, Group, Offset, Quant, Weight};

/// What a type permits, as opposed to what this file happens to hold.
///
/// Four field kinds know more than their value shows: an enum knows the other
/// values it would accept, a magic field knows the bytes it wanted, a flags
/// field knows what each bit means, and a float knows which of its bits are
/// which. This is one answer for all of them, because they are one question:
/// what does this type say, beyond the number.
#[derive(Debug, Clone, PartialEq)]
pub enum Explain {
    /// The bytes the format requires, and the bytes that are there. They are
    /// equal when the field matches, and worth comparing when it does not.
    Magic { expected: Vec<u8>, actual: Vec<u8> },
    /// Every value the enum names, and the one the file holds. `current` is not
    /// always among them: a file is free to hold a value nobody named.
    Enum { name: String, hex: bool, cases: Vec<(i128, String)>, current: i128 },
    /// Every bit of the field, from bit 0 up, whether it is set and what it is
    /// called. A bit with no name is still a bit, and is still listed.
    Flags { name: String, raw: u128, bits: Vec<FlagBit> },
    /// A binary float, as its bits: 16, 32 or 64 of them, in value order with
    /// the byte order already resolved, so a reader can take the sign, the
    /// exponent and the significand apart without knowing how it was stored.
    /// A float, by the name of its layout rather than only its width: two
    /// sixteen-bit floats are in use and they divide their bits differently.
    Float { format: &'static str, width: u32, bits: u64 },
    /// A block of packed weights, taken apart: the block's shared scale, what
    /// it pairs with the scale, and every weight the block stands for, in the
    /// order the tensor reads them. Shown for the cursor anywhere in the block,
    /// because the packing crosses the fields: a `q4_k` weight is four bits of
    /// `qs` scaled by six bits of `scales` and two half floats at the front.
    Quant {
        /// The block layout, as ggml's own struct is named: `Q4_K`.
        kind: &'static str,
        /// How many bits one weight is worth.
        bits: u32,
        /// The block's shared scale, and what it pairs with: the `m` a `q4_1`
        /// adds, the `dmin` a K type takes away.
        d: f64,
        second: Option<Offset>,
        /// Where the block starts, so that a weight's bits can be found from
        /// the offset it carries.
        block_bits: u64,
        /// The scales the block keeps per group of weights, where it has them,
        /// and how many weights one group covers. What a K type spends twelve
        /// or sixteen bytes on, which read as bytes say nothing.
        groups: Vec<Group>,
        group_weights: u32,
        /// Taken off the packed value to get the stored one, and whether that
        /// value is read signed instead.
        bias: i32,
        signed: bool,
        weights: Vec<Weight>,
        /// Which weight the cursor is inside, where it is on one of them
        /// rather than on the block's scales.
        at: Option<usize>,
    },
    /// The type has nothing to add: its value already says everything.
    Plain,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FlagBit {
    pub bit: u32,
    pub name: Option<String>,
    pub set: bool,
}

impl Evaluator {
    /// What the type at `path` permits. See [`Explain`].
    /// `at_bits` is where the cursor is, which only a packed block uses: it
    /// says which of the block's weights the reader is standing on.
    pub fn explain<S: Source>(&mut self, doc: &Document<S>, path: &[usize], at_bits: Option<u64>) -> R<Explain> {
        self.resolve(doc, path)?;
        let size = self.size_of(doc, path)?;
        let r = self.memo.get(path).expect("resolved").clone();
        Ok(match &r.ty {
            Ty::Magic(want) => {
                // A short read is not a failure to explain: the expected bytes
                // are known whatever the file turned out to hold.
                let actual = self.read(doc, &r, r.offset, size).unwrap_or_default();
                Explain::Magic { expected: want.clone(), actual }
            }
            Ty::Enum { def, .. } => {
                let current = self.value_at(doc, path)?.as_int().unwrap_or(0);
                Explain::Enum {
                    name: def.name.clone(),
                    hex: def.hex,
                    cases: def.cases.clone(),
                    current,
                }
            }
            Ty::Flags { def, .. } => {
                let raw = match self.value_at(doc, path)? {
                    Value::Flags { raw, .. } => raw,
                    other => other.as_int().and_then(|v| u128::try_from(v).ok()).unwrap_or(0),
                };
                let bits = (0..size.min(64) as u32)
                    .map(|bit| FlagBit {
                        bit,
                        name: def.label(bit).map(str::to_string),
                        set: raw >> bit & 1 == 1,
                    })
                    .collect();
                Explain::Flags { name: def.name.clone(), raw, bits }
            }
            // An eight-bit float is one byte, so there is no order to it.
            Ty::F8 { e4m3 } => {
                let raw = self.read(doc, &r, r.offset, 8)?;
                Explain::Float { format: if *e4m3 { "e4m3" } else { "e5m2" }, width: 8, bits: raw[0] as u64 }
            }
            Ty::F16(e) | Ty::BF16(e) | Ty::F32(e) | Ty::F64(e) => {
                let (format, width): (&'static str, u32) = match r.ty {
                    Ty::F16(_) => ("binary16", 16),
                    Ty::BF16(_) => ("bfloat16", 16),
                    Ty::F32(_) => ("binary32", 32),
                    _ => ("binary64", 64),
                };
                let raw = self.read(doc, &r, r.offset, u64::from(width))?;
                Explain::Float { format, width, bits: crate::decode::read_uint(&raw, width, *e) as u64 }
            }
            _ => return self.explain_packed(doc, path, at_bits),
        })
    }

    /// A packed block, from the cursor being on it or on one of its fields.
    ///
    /// Both are asked because the fields are where a reader lands: the cursor
    /// is almost always inside `qs`, and a panel that only answered for the
    /// block itself would be blank exactly when it is wanted.
    fn explain_packed<S: Source>(&mut self, doc: &Document<S>, path: &[usize], at_bits: Option<u64>) -> R<Explain> {
        for len in [path.len(), path.len().wrapping_sub(1)] {
            if len > path.len() {
                break;
            }
            let at = &path[..len];
            self.resolve(doc, at)?;
            let r = self.memo.get(at).expect("resolved").clone();
            let Ty::Struct(def) = &r.ty else { continue };
            let Some(packing) = def.packed.clone() else { continue };
            let Some(kind) = ggml_quant::by_name(&packing) else { continue };
            let bytes = self.read(doc, &r, r.offset, kind.block_bytes() as u64 * 8)?;
            let Some(block) = ggml_quant::unpack(kind, &bytes) else { continue };
            return Ok(quant_of(kind, block, r.offset, at_bits));
        }
        Ok(Explain::Plain)
    }

    fn value_at<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Value> {
        Ok(self.node(doc, path)?.value)
    }
}

/// One unpacked block as an answer, with the cursor matched to the weight whose
/// bits it is inside.
fn quant_of(kind: Quant, block: ggml_quant::Block, block_bits: u64, at_bits: Option<u64>) -> Explain {
    // Either run of a split weight identifies it: the cursor is a bit, so the
    // one bit of `qh` a five-bit weight keeps there belongs to it and nothing
    // else.
    let holds = |p: &ggml_quant::Part, rel: u64| u64::from(p.bit) <= rel && rel < u64::from(p.bit + p.width);
    let at = at_bits.and_then(|c| c.checked_sub(block_bits)).and_then(|rel| {
        block
            .weights
            .iter()
            .position(|w| holds(&w.bits, rel) || w.high.is_some_and(|h| holds(&h, rel)))
    });
    Explain::Quant {
        kind: kind.name(),
        bits: kind.bits(),
        d: block.d,
        second: block.second,
        block_bits,
        groups: block.groups,
        group_weights: block.group_weights,
        bias: block.bias,
        signed: block.signed,
        weights: block.weights,
        at,
    }
}
