//! What a type knows beyond the value in front of it: the values an enum
//! names, the bytes a magic field wanted, what each bit of a flags field means,
//! and which bits of a float are which.

use super::*;

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
    Float { width: u32, bits: u64 },
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
    pub fn explain<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Explain> {
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
            Ty::F16(e) | Ty::F32(e) | Ty::F64(e) => {
                let width: u32 = match r.ty {
                    Ty::F16(_) => 16,
                    Ty::F32(_) => 32,
                    _ => 64,
                };
                let raw = self.read(doc, &r, r.offset, u64::from(width))?;
                Explain::Float { width, bits: crate::decode::read_uint(&raw, width, *e) as u64 }
            }
            _ => Explain::Plain,
        })
    }

    fn value_at<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Value> {
        Ok(self.node(doc, path)?.value)
    }
}
