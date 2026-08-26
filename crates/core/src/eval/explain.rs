//! What a type knows beyond the value in front of it: the values an enum
//! names, the bytes a magic field wanted, what each bit of a flags field means,
//! and which bits of a float are which.

use super::*;
use crate::formats::ggml_quant::{self, Group, Offset, Quant, Weight};
use crate::formats::{pdf_objstm, pdf_xref};

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
    /// `named` is what the value goes by, which is not always in `cases`: a
    /// format that stops naming values and starts counting them names it by
    /// the run it falls in instead.
    Enum { name: String, hex: bool, cases: Vec<(i128, String)>, current: i128, named: Option<String> },
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
    /// A cross-reference stream, decompressed and split into the rows it
    /// stands for. Shown for the cursor anywhere in the object, because the
    /// rows are not bytes of the file and there is nothing to land on: what
    /// the reader is standing on is the packed run they came out of.
    XrefRows {
        /// The widths from `/W` and the predictor from `/DecodeParms`, which
        /// say how the run was taken apart.
        widths: [u32; 3],
        predictor: Option<u32>,
        /// How many bytes the run is in the file, and how many it came to once
        /// decompressed.
        packed_bytes: u64,
        decoded_bytes: u64,
        /// How many rows there are of each kind, over the whole table rather
        /// than over the ones listed. `unknown` counts rows whose type the
        /// spec does not define, which are kept apart so the four add up to
        /// the total rather than quietly landing in one of the other three.
        free: usize,
        in_file: usize,
        in_stream: usize,
        unknown: usize,
        /// The rows themselves, up to [`XREF_ROWS_SHOWN`] of them, and how
        /// many there are altogether. A table with more says so rather than
        /// looking complete.
        rows: Vec<pdf_xref::Row>,
        total: usize,
        /// Why there are no rows, where there are none.
        problem: Option<String>,
    },
    /// An object stream, opened into the objects it holds. Shown for the
    /// cursor anywhere in the object, because the objects inside are not bytes
    /// of the file: what the reader is standing on is the compressed run they
    /// came out of.
    ObjStm {
        /// How many bytes the run is in the file, and how many it came to once
        /// decompressed.
        packed_bytes: u64,
        decoded_bytes: u64,
        /// Where the objects begin in the decompressed bytes, from `/First`.
        first: u64,
        /// The object number in `/Extends`: the object stream this one is a
        /// continuation of, where it is one. Not followed.
        extends: Option<u64>,
        /// The objects, up to [`OBJSTM_SHOWN`] of them, and how many the
        /// dictionary said there were.
        objects: Vec<pdf_objstm::Object>,
        total: usize,
        /// Why there are no objects, where there are none.
        problem: Option<String>,
    },
    /// The type has nothing to add: its value already says everything.
    Plain,
}

/// How many objects of an object stream are handed to a reader at once.
pub const OBJSTM_SHOWN: usize = 256;

/// How much of an object is read to find out whether it is an object stream.
/// Its dictionary comes first and no real one runs longer than this, so an
/// image object several megabytes long is passed over for the price of a page.
const OBJSTM_DICT_PREFIX: u64 = 8 << 10;

/// The largest compressed run this will open for a panel that is redrawn every
/// time the cursor moves.
const OBJSTM_PACKED_LIMIT: u64 = 4 << 20;

/// How many rows of a cross-reference stream are handed to a reader at once.
/// A table runs to one row an object, and a panel is not the place to read a
/// hundred thousand of them.
pub const XREF_ROWS_SHOWN: usize = 512;

/// The largest packed run this will decompress for a panel that is redrawn
/// every time the cursor moves. Four megabytes of compressed table is a file
/// with millions of objects; nothing real reaches it.
const XREF_PACKED_LIMIT: u64 = 4 << 20;

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
                    named: def.name_of(current),
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
            if &*packing == pdf_xref::PACKING {
                return self.explain_xref(doc, at, &r);
            }
            if &*packing == pdf_objstm::PACKING {
                return self.explain_objstm(doc, at, &r);
            }
            let Some(kind) = ggml_quant::by_name(&packing) else { continue };
            let bytes = self.read(doc, &r, r.offset, kind.block_bytes() as u64 * 8)?;
            let Some(block) = ggml_quant::unpack(kind, &bytes) else { continue };
            return Ok(quant_of(kind, block, r.offset, at_bits));
        }
        Ok(Explain::Plain)
    }

    /// A cross-reference stream taken apart. The dictionary beside the packed
    /// run says how, so both are read from the object the cursor is in.
    ///
    /// A run that will not decode is still an answer: what the dictionary said
    /// is worth showing next to the reason, since between them they are how a
    /// reader works out whether the file is odd or this is.
    fn explain_xref<S: Source>(&mut self, doc: &Document<S>, at: &[usize], r: &Resolved) -> R<Explain> {
        let Ty::Struct(def) = &r.ty else { return Ok(Explain::Plain) };
        let field = |name: &str| def.fields.iter().position(|f| &*f.name == name);
        let (Some(d), Some(p)) = (field("dictionary"), field("rows")) else { return Ok(Explain::Plain) };

        let mut dict_path = at.to_vec();
        dict_path.push(d);
        let Value::Str(dict) = self.node(doc, &dict_path)?.value else { return Ok(Explain::Plain) };

        let mut rows_path = at.to_vec();
        rows_path.push(p);
        self.resolve(doc, &rows_path)?;
        let packed_bits = self.size_of(doc, &rows_path)?;
        let rr = self.memo.get(&rows_path).expect("resolved").clone();
        let packed_bytes = packed_bits / 8;

        let answer = |problem: Option<String>, t: Option<pdf_xref::Table>| {
            let t = t.unwrap_or(pdf_xref::Table {
                rows: Vec::new(),
                widths: [0, 0, 0],
                predictor: None,
                decoded_bytes: 0,
                trailing_bytes: 0,
            });
            let mut counts = (0usize, 0usize, 0usize, 0usize);
            for row in &t.rows {
                match row.kind {
                    pdf_xref::Kind::Free => counts.0 += 1,
                    pdf_xref::Kind::InFile => counts.1 += 1,
                    pdf_xref::Kind::InStream => counts.2 += 1,
                    pdf_xref::Kind::Other(_) => counts.3 += 1,
                }
            }
            Explain::XrefRows {
                widths: t.widths,
                predictor: t.predictor,
                packed_bytes,
                decoded_bytes: t.decoded_bytes as u64,
                free: counts.0,
                in_file: counts.1,
                in_stream: counts.2,
                unknown: counts.3,
                total: t.rows.len(),
                rows: t.rows.into_iter().take(XREF_ROWS_SHOWN).collect(),
                problem,
            }
        };

        if packed_bytes > XREF_PACKED_LIMIT {
            let mb = XREF_PACKED_LIMIT / (1 << 20);
            let msg = format!("The compressed data is over the {mb} MB limit and was not decompressed.");
            return Ok(answer(Some(msg), None));
        }
        let bytes = self.read(doc, &rr, rr.offset, packed_bits)?;
        Ok(match pdf_xref::decode(&dict, &bytes) {
            Ok(t) => answer(None, Some(t)),
            Err(p) => answer(Some(p.as_str()), None),
        })
    }

    /// An object stream opened. Every object in the file arrives here, because
    /// only the dictionary inside one says whether it is an object stream, so
    /// the first thing this does is read enough of the body to find out and
    /// hand back `Plain` for the objects that are not.
    fn explain_objstm<S: Source>(&mut self, doc: &Document<S>, at: &[usize], r: &Resolved) -> R<Explain> {
        let Ty::Struct(def) = &r.ty else { return Ok(Explain::Plain) };
        let Some(b) = def.fields.iter().position(|f| &*f.name == "body") else { return Ok(Explain::Plain) };

        let mut body_path = at.to_vec();
        body_path.push(b);
        self.resolve(doc, &body_path)?;
        let body_bits = self.size_of(doc, &body_path)?;
        let br = self.memo.get(&body_path).expect("resolved").clone();

        // The dictionary and no more of the object than that. A body with no
        // `stream` keyword in its first few kilobytes is not an object stream,
        // and neither is one whose dictionary says it is something else.
        let head = self.read(doc, &br, br.offset, body_bits.min(OBJSTM_DICT_PREFIX * 8))?;
        let Some((dict, _)) = pdf_objstm::split_body(&head) else { return Ok(Explain::Plain) };
        if !pdf_objstm::is_object_stream(dict) {
            return Ok(Explain::Plain);
        }

        let packed_bytes = body_bits / 8;
        let answer = |problem: Option<String>, s: Option<pdf_objstm::Stream>| {
            let s = s.unwrap_or(pdf_objstm::Stream {
                objects: Vec::new(),
                claimed: 0,
                first: 0,
                decoded_bytes: 0,
                extends: None,
            });
            Explain::ObjStm {
                packed_bytes,
                decoded_bytes: s.decoded_bytes as u64,
                first: s.first as u64,
                extends: s.extends,
                total: s.objects.len(),
                objects: s.objects.into_iter().take(OBJSTM_SHOWN).collect(),
                problem,
            }
        };

        if packed_bytes > OBJSTM_PACKED_LIMIT {
            let mb = OBJSTM_PACKED_LIMIT / (1 << 20);
            let msg = format!("The compressed data is over the {mb} MB limit and was not decompressed.");
            return Ok(answer(Some(msg), None));
        }
        let body = self.read(doc, &br, br.offset, body_bits)?;
        let Some((dict, data)) = pdf_objstm::split_body(&body) else { return Ok(Explain::Plain) };
        Ok(match pdf_objstm::decode(dict, data) {
            Ok(s) => answer(None, Some(s)),
            Err(p) => answer(Some(p.as_str()), None),
        })
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
