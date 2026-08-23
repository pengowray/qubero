//! Lazy template evaluation over a document.
//!
//! Nodes are addressed by path (child indices from the root). Offsets and sizes
//! are memoised per path and thrown away when the document changes. A read that
//! touches an unloaded chunk yields `EvalError::Pending` rather than a value, so
//! zero-filled bytes can never be mistaken for data.

use std::collections::HashMap;

use crate::bits::bytes_for;
use crate::decode::{be_int, f16_to_f64, fixed_bits, read_uint};
use crate::document::Document;
use crate::encode;
use crate::source::{Missing, Source};
use crate::template::{Expr, Template, Ty, Until};

#[derive(Debug, Clone, PartialEq)]
pub enum EvalError {
    Pending(Vec<Missing>),
    Failed(String),
}

pub type R<T> = Result<T, EvalError>;

fn fail<T>(msg: impl Into<String>) -> R<T> {
    Err(EvalError::Failed(msg.into()))
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    UInt(u128),
    Int(i128),
    Float(f64),
    Bytes { len: u64, preview: Vec<u8> },
    Str(String),
    Magic { ok: bool },
    Composite { count: u64 },
}

impl Value {
    pub fn as_int(&self) -> Option<i128> {
        match self {
            Value::UInt(v) => i128::try_from(*v).ok(),
            Value::Int(v) => Some(*v),
            Value::Composite { count } => Some(*count as i128),
            // Short text/byte fields used in expressions are their bytes as a
            // big-endian number, so a switch can key on e.g. "IHDR".
            Value::Bytes { len, preview } if *len <= 15 => Some(be_int(preview)),
            Value::Str(s) if s.len() <= 15 => Some(be_int(s.as_bytes())),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodeInfo {
    pub path: Vec<usize>,
    pub name: String,
    pub type_name: String,
    pub offset_bits: u64,
    pub size_bits: u64,
    pub value: Value,
    pub child_count: u64,
    /// True for structs, arrays and repeats.
    pub composite: bool,
    /// True when `write` accepts text for this field.
    pub editable: bool,
}

/// Bits to write, and where. Produced by `Evaluator::prepare_write`.
#[derive(Debug, Clone, PartialEq)]
pub struct Write {
    pub offset_bits: u64,
    /// MSB-first packed, `n_bits` long.
    pub data: Vec<u8>,
    pub n_bits: u64,
}

#[derive(Debug, Clone)]
struct Resolved {
    name: String,
    /// Effective type after unwrapping `Sized` and `Switch`.
    ty: Ty,
    offset: u64,
    /// Exclusive bit limit this node may not read past.
    limit: u64,
    /// Size fixed by an enclosing `Sized`, if any.
    declared_size: Option<u64>,
    size: Option<u64>,
    /// For `Repeat`: end offsets of elements resolved so far, and whether the
    /// walk reached its terminating condition.
    repeat_ends: Vec<u64>,
    repeat_done: bool,
    /// Children `0..seq_end` are resolved and sized, so child `seq_end` can be
    /// placed without walking back. Keeps sibling resolution iterative.
    seq_end: usize,
}

pub struct Evaluator {
    template: Template,
    memo: HashMap<Vec<usize>, Resolved>,
}

impl Evaluator {
    pub fn new(template: Template) -> Self {
        Self { template, memo: HashMap::new() }
    }

    pub fn template(&self) -> &Template {
        &self.template
    }

    /// Drop every cached offset/size. Call after any document change.
    pub fn invalidate(&mut self) {
        self.memo.clear();
    }

    pub fn node<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<NodeInfo> {
        self.resolve(doc, path)?;
        let size = self.size_of(doc, path)?;
        let r = self.memo.get(path).expect("resolved").clone();
        let (value, child_count, composite) = match &r.ty {
            Ty::Struct(s) => (Value::Composite { count: s.fields.len() as u64 }, s.fields.len() as u64, true),
            Ty::Array { .. } | Ty::Repeat { .. } => {
                let n = self.child_count(doc, path)?;
                (Value::Composite { count: n }, n, true)
            }
            _ => (self.primitive_value(doc, &r, size)?, 0, false),
        };
        Ok(NodeInfo {
            path: path.to_vec(),
            editable: !composite && encode::editable(&r.ty, size),
            name: r.name,
            type_name: r.ty.display_name(),
            offset_bits: r.offset,
            size_bits: size,
            value,
            child_count,
            composite,
        })
    }

    /// Encode `text` for the field at `path`, ready to be written.
    ///
    /// Resolving a field can touch unloaded chunks, so this reports the same
    /// tri-state as a read: the caller fetches the chunks and asks again.
    /// Nothing is written here. The caller applies the result to the document
    /// and then calls `invalidate`.
    pub fn prepare_write<S: Source>(&mut self, doc: &Document<S>, path: &[usize], text: &str) -> R<Write> {
        self.resolve(doc, path)?;
        let size = self.size_of(doc, path)?;
        let r = self.memo.get(path).expect("resolved").clone();
        if !encode::editable(&r.ty, size) {
            return fail(match &r.ty {
                Ty::Magic(_) => "Magic bytes are fixed by the format.".to_string(),
                Ty::Bytes(_) | Ty::Utf8(_) => format!("Too long to edit here: {} bytes. Use the hex view.", size / 8),
                _ => "This field can't be edited here. Use the hex view.".to_string(),
            });
        }
        let data = encode::encode(&r.ty, text, size).map_err(EvalError::Failed)?;
        Ok(Write { offset_bits: r.offset, data, n_bits: size })
    }

    /// Children `from..to` of the node at `path` (clamped to the child count).
    pub fn children<S: Source>(&mut self, doc: &Document<S>, path: &[usize], from: u64, to: u64) -> R<Vec<NodeInfo>> {
        let n = self.child_count(doc, path)?;
        let mut out = Vec::new();
        let mut p = path.to_vec();
        for i in from..to.min(n) {
            p.push(i as usize);
            out.push(self.node(doc, &p)?);
            p.pop();
        }
        Ok(out)
    }

    // ----- resolution -----

    fn resolve<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<()> {
        if self.memo.contains_key(path) {
            return Ok(());
        }
        if path.is_empty() {
            let limit = doc.len_bits();
            let root = self.template.root.clone();
            let r = self.effective(doc, &[], "file".into(), root, 0, limit)?;
            self.memo.insert(vec![], r);
            return Ok(());
        }
        let (parent, idx) = (&path[..path.len() - 1], path[path.len() - 1]);
        self.resolve(doc, parent)?;
        let pr = self.memo.get(parent).expect("parent resolved").clone();
        let (name, ty) = match &pr.ty {
            Ty::Struct(s) => match s.fields.get(idx) {
                Some(f) => (f.name.clone(), f.ty.clone()),
                None => return fail("no such field"),
            },
            Ty::Array { elem, .. } | Ty::Repeat { elem, .. } => (format!("[{idx}]"), (**elem).clone()),
            _ => return fail("not a composite"),
        };
        // Offset: after the previous sibling, or at the parent's start.
        let offset = if idx == 0 {
            pr.offset
        } else if let (Ty::Array { elem, .. }, Some(fs)) = (&pr.ty, fixed_bits(child_elem(&pr.ty))) {
            let _ = elem;
            pr.offset + idx as u64 * fs
        } else {
            // Place after the previous sibling. Siblings are resolved in order,
            // iteratively, so deep arrays do not recurse element by element.
            // Note: asking for an element past a Repeat's terminating condition
            // is not prevented here; `children()` clamps, direct callers must too.
            self.resolve_upto(doc, parent, idx)?;
            let mut prev = parent.to_vec();
            prev.push(idx - 1);
            self.memo[&prev].offset + self.size_of(doc, &prev)?
        };
        if offset > pr.limit {
            return fail("runs past the end of its container");
        }
        let r = self.effective(doc, path, name, ty, offset, pr.limit)?;
        self.memo.insert(path.to_vec(), r);
        Ok(())
    }

    /// Resolve and size children `0..idx` of `parent`, in order, without recursion.
    fn resolve_upto<S: Source>(&mut self, doc: &Document<S>, parent: &[usize], idx: usize) -> R<()> {
        let mut j = self.memo[parent].seq_end;
        let mut p = parent.to_vec();
        while j < idx {
            p.push(j);
            self.resolve(doc, &p)?;
            self.size_of(doc, &p)?;
            p.pop();
            j += 1;
            self.memo.get_mut(parent).expect("resolved").seq_end = j;
        }
        Ok(())
    }

    /// Unwrap `Sized` and `Switch` wrappers into the type actually parsed here.
    fn effective<S: Source>(
        &mut self,
        doc: &Document<S>,
        path: &[usize],
        name: String,
        mut ty: Ty,
        offset: u64,
        mut limit: u64,
    ) -> R<Resolved> {
        let mut declared_size = None;
        loop {
            match ty {
                Ty::Sized { size, inner } => {
                    let bytes = self.eval_expr(doc, path, &size)?;
                    if bytes < 0 {
                        return fail("negative size");
                    }
                    let bits = bytes as u64 * 8;
                    if offset + bits > limit {
                        return fail(format!("size {bytes} runs past the end of its container"));
                    }
                    limit = offset + bits;
                    declared_size = Some(bits);
                    ty = *inner;
                }
                Ty::Switch { on, cases, default } => {
                    let v = self.eval_expr(doc, path, &on)?;
                    ty = cases.into_iter().find(|(k, _)| *k == v).map(|(_, t)| t).unwrap_or(*default);
                }
                other => {
                    return Ok(Resolved {
                        name,
                        ty: other,
                        offset,
                        limit,
                        declared_size,
                        size: None,
                        repeat_ends: Vec::new(),
                        repeat_done: false,
                        seq_end: 0,
                    });
                }
            }
        }
    }

    fn size_of<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<u64> {
        self.resolve(doc, path)?;
        let r = self.memo[path].clone();
        if let Some(s) = r.size {
            return Ok(s);
        }
        let size = if let Some(d) = r.declared_size {
            d
        } else if let Some(f) = fixed_bits(&r.ty) {
            f
        } else {
            match &r.ty {
                Ty::Bytes(e) | Ty::Utf8(e) => {
                    let n = self.eval_expr(doc, path, e)?;
                    if n < 0 {
                        return fail("negative length");
                    }
                    n as u64 * 8
                }
                Ty::Leb128 { .. } => {
                    let (_, n) = self.read_leb(doc, &r)?;
                    n * 8
                }
                Ty::Struct(s) => {
                    if s.fields.is_empty() {
                        0
                    } else {
                        let mut last = path.to_vec();
                        last.push(s.fields.len() - 1);
                        self.resolve_upto(doc, path, s.fields.len() - 1)?;
                        self.resolve(doc, &last)?;
                        let end = self.memo[&last].offset + self.size_of(doc, &last)?;
                        end - r.offset
                    }
                }
                Ty::Array { .. } | Ty::Repeat { .. } => {
                    let n = self.child_count(doc, path)?;
                    if n == 0 {
                        0
                    } else {
                        let mut last = path.to_vec();
                        last.push(n as usize - 1);
                        self.resolve_upto(doc, path, n as usize - 1)?;
                        self.resolve(doc, &last)?;
                        let end = self.memo[&last].offset + self.size_of(doc, &last)?;
                        end - r.offset
                    }
                }
                _ => unreachable!("fixed-size types handled above"),
            }
        };
        if r.offset + size > r.limit {
            return fail("runs past the end of its container");
        }
        self.memo.get_mut(path).expect("resolved").size = Some(size);
        Ok(size)
    }

    fn child_count<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<u64> {
        self.resolve(doc, path)?;
        let r = self.memo[path].clone();
        match &r.ty {
            Ty::Struct(s) => Ok(s.fields.len() as u64),
            Ty::Array { count, .. } => {
                let n = self.eval_expr(doc, path, count)?;
                if n < 0 {
                    return fail("negative count");
                }
                Ok(n as u64)
            }
            Ty::Repeat { until, .. } => {
                if r.repeat_done {
                    return Ok(r.repeat_ends.len() as u64);
                }
                let mut p = path.to_vec();
                loop {
                    let (ends, done) = {
                        let m = &self.memo[path];
                        (m.repeat_ends.len(), m.repeat_done)
                    };
                    if done {
                        return Ok(ends as u64);
                    }
                    let start = self.memo[path].repeat_ends.last().copied().unwrap_or(r.offset);
                    if start >= r.limit {
                        self.memo.get_mut(path).expect("resolved").repeat_done = true;
                        return Ok(ends as u64);
                    }
                    p.push(ends);
                    self.resolve(doc, &p)?;
                    let size = self.size_of(doc, &p)?;
                    let end = self.memo[&p].offset + size;
                    if size == 0 {
                        return fail("repeated element has zero size");
                    }
                    let stop = match until {
                        Until::End => false,
                        Until::FieldBytes { field, bytes } => {
                            let want = bytes.clone();
                            let got = self.child_raw_bytes(doc, &p, field)?;
                            got == want
                        }
                    };
                    p.pop();
                    let m = self.memo.get_mut(path).expect("resolved");
                    m.repeat_ends.push(end);
                    if stop {
                        m.repeat_done = true;
                    }
                }
            }
            _ => Ok(0),
        }
    }

    /// Raw bytes of a named field directly inside the struct at `path`.
    fn child_raw_bytes<S: Source>(&mut self, doc: &Document<S>, path: &[usize], field: &str) -> R<Vec<u8>> {
        let idx = match &self.memo[path].ty {
            Ty::Struct(s) => s.fields.iter().position(|f| f.name == field),
            _ => None,
        };
        let Some(idx) = idx else { return fail(format!("no field named {field}")) };
        let mut p = path.to_vec();
        p.push(idx);
        self.resolve(doc, &p)?;
        let size = self.size_of(doc, &p)?;
        let r = self.memo[&p].clone();
        self.read(doc, &r, r.offset, size)
    }

    // ----- expressions -----

    fn eval_expr<S: Source>(&mut self, doc: &Document<S>, at: &[usize], e: &Expr) -> R<i128> {
        Ok(match e {
            Expr::Lit(v) => *v,
            Expr::Ref(name) => self.lookup(doc, at, name)?,
            Expr::Add(a, b) => self.eval_expr(doc, at, a)? + self.eval_expr(doc, at, b)?,
            Expr::Sub(a, b) => self.eval_expr(doc, at, a)? - self.eval_expr(doc, at, b)?,
            Expr::Mul(a, b) => self.eval_expr(doc, at, a)? * self.eval_expr(doc, at, b)?,
            Expr::Div(a, b) => {
                let d = self.eval_expr(doc, at, b)?;
                if d == 0 {
                    return fail("division by zero");
                }
                self.eval_expr(doc, at, a)? / d
            }
        })
    }

    /// Find `name` among the fields before `at` in its struct, then in enclosing structs.
    fn lookup<S: Source>(&mut self, doc: &Document<S>, at: &[usize], name: &str) -> R<i128> {
        let mut cur = at.to_vec();
        while !cur.is_empty() {
            let idx = cur.pop().expect("non-empty");
            let parent = cur.clone();
            if let Ty::Struct(s) = &self.memo[&parent].ty {
                if let Some(j) = s.fields[..idx].iter().position(|f| f.name == name) {
                    let mut p = parent;
                    p.push(j);
                    let info = self.node(doc, &p)?;
                    return match info.value.as_int() {
                        Some(v) => Ok(v),
                        None => fail(format!("{name} is not a number")),
                    };
                }
            }
        }
        fail(format!("unknown field {name}"))
    }

    // ----- reading -----

    fn read<S: Source>(&self, doc: &Document<S>, r: &Resolved, at: u64, n: u64) -> R<Vec<u8>> {
        if at + n > r.limit {
            return fail("runs past the end of its container");
        }
        let mut buf = vec![0u8; bytes_for(n)];
        let missing = doc.read_bits(at, n, &mut buf);
        if missing.is_empty() {
            Ok(buf)
        } else {
            Err(EvalError::Pending(missing))
        }
    }

    fn read_leb<S: Source>(&self, doc: &Document<S>, r: &Resolved) -> R<(u128, u64)> {
        let mut value: u128 = 0;
        let mut shift = 0;
        for i in 0..10u64 {
            let b = self.read(doc, r, r.offset + i * 8, 8)?[0];
            value |= ((b & 0x7f) as u128) << shift;
            shift += 7;
            if b & 0x80 == 0 {
                if let Ty::Leb128 { signed: true } = r.ty {
                    if b & 0x40 != 0 {
                        let v = value as i128 - (1i128 << shift);
                        return Ok((v as u128, i + 1));
                    }
                }
                return Ok((value, i + 1));
            }
        }
        fail("LEB128 longer than 10 bytes")
    }

    fn primitive_value<S: Source>(&mut self, doc: &Document<S>, r: &Resolved, size: u64) -> R<Value> {
        Ok(match &r.ty {
            Ty::UInt { bits, endian } => Value::UInt(read_uint(&self.read(doc, r, r.offset, size)?, *bits, *endian)),
            Ty::Int { bits, endian } => {
                let u = read_uint(&self.read(doc, r, r.offset, size)?, *bits, *endian);
                let v = if *bits < 128 && u >> (bits - 1) & 1 == 1 { u as i128 - (1i128 << bits) } else { u as i128 };
                Value::Int(v)
            }
            Ty::F16(e) => Value::Float(f16_to_f64(read_uint(&self.read(doc, r, r.offset, 16)?, 16, *e) as u16)),
            Ty::F32(e) => Value::Float(f32::from_bits(read_uint(&self.read(doc, r, r.offset, 32)?, 32, *e) as u32) as f64),
            Ty::F64(e) => Value::Float(f64::from_bits(read_uint(&self.read(doc, r, r.offset, 64)?, 64, *e) as u64)),
            Ty::Leb128 { signed } => {
                let (v, _) = self.read_leb(doc, r)?;
                if *signed { Value::Int(v as i128) } else { Value::UInt(v) }
            }
            Ty::Magic(want) => Value::Magic { ok: self.read(doc, r, r.offset, size)? == *want },
            Ty::Bytes(_) => {
                let len = size / 8;
                let preview = self.read(doc, r, r.offset, len.min(16) * 8)?;
                Value::Bytes { len, preview }
            }
            Ty::Utf8(_) => {
                let len = size / 8;
                let bytes = self.read(doc, r, r.offset, len.min(256) * 8)?;
                let mut s = String::from_utf8_lossy(&bytes).into_owned();
                if len > 256 {
                    s.push('…');
                }
                Value::Str(s)
            }
            _ => unreachable!("composite handled by caller"),
        })
    }
}

fn child_elem(ty: &Ty) -> &Ty {
    match ty {
        Ty::Array { elem, .. } | Ty::Repeat { elem, .. } => elem,
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::MemSource;
    use crate::template::{Endian::*, Expr as E, Ty as T};

    fn doc(bytes: &[u8]) -> Document<MemSource> {
        Document::new(MemSource(bytes.to_vec()))
    }

    #[test]
    fn struct_with_count_driven_array() {
        let t = Template {
            name: "t".into(),
            root: T::structure("Root", vec![("n", T::u8()), ("items", T::array(T::u16(Little), E::field("n")))]),
        };
        let d = doc(&[3, 1, 0, 2, 0, 3, 0, 99]);
        let mut ev = Evaluator::new(t);
        let root = ev.node(&d, &[]).unwrap();
        assert_eq!(root.size_bits, 7 * 8);
        assert_eq!(root.child_count, 2);
        let items = ev.node(&d, &[1]).unwrap();
        assert_eq!(items.child_count, 3);
        assert_eq!(items.offset_bits, 8);
        let third = ev.node(&d, &[1, 2]).unwrap();
        assert_eq!(third.value, Value::UInt(3));
        assert_eq!(third.offset_bits, 5 * 8);
    }

    #[test]
    fn repeat_until_end_and_leb128() {
        // Records: leb128 length, then bytes. Three records.
        let t = Template {
            name: "t".into(),
            root: T::repeat(
                T::structure("Rec", vec![("len", T::leb_u()), ("data", T::bytes(E::field("len")))]),
                Until::End,
            ),
        };
        let mut bytes = vec![2, 0xAA, 0xBB, 0, 0x80, 0x01];
        bytes.extend(std::iter::repeat_n(7u8, 128));
        let d = doc(&bytes);
        let mut ev = Evaluator::new(t);
        let root = ev.node(&d, &[]).unwrap();
        assert_eq!(root.child_count, 3);
        assert_eq!(root.size_bits, bytes.len() as u64 * 8);
        let third_len = ev.node(&d, &[2, 0]).unwrap();
        assert_eq!(third_len.value, Value::UInt(128));
        assert_eq!(third_len.size_bits, 16);
    }

    #[test]
    fn sized_switch_and_pending() {
        use crate::source::ChunkStore;
        let t = Template {
            name: "t".into(),
            root: T::structure(
                "Root",
                vec![
                    ("kind", T::u8()),
                    ("size", T::u8()),
                    (
                        "body",
                        T::sized(
                            E::field("size"),
                            T::switch(E::field("kind"), vec![(1, T::u32(Big))], T::bytes(E::field("size"))),
                        ),
                    ),
                ],
            ),
        };
        let mut d = Document::new(ChunkStore::new(6, 4, 8));
        let mut ev = Evaluator::new(t.clone());
        assert!(matches!(ev.node(&d, &[]), Err(EvalError::Pending(_))));
        d.source_mut().insert(0, vec![1, 4, 0, 0].into_boxed_slice());
        assert!(matches!(ev.node(&d, &[2]), Err(EvalError::Pending(_))));
        d.source_mut().insert(1, vec![1, 2].into_boxed_slice());
        let body = ev.node(&d, &[2]).unwrap();
        assert_eq!(body.type_name, "u32 be");
        assert_eq!(body.value, Value::UInt(0x0102));
        assert_eq!(body.size_bits, 32);
        // A size that overruns the file is an error, not a zero.
        let d2 = doc(&[9, 40, 0]);
        let mut ev2 = Evaluator::new(t);
        assert!(matches!(ev2.node(&d2, &[2]), Err(EvalError::Failed(_))));
    }

    #[test]
    fn huge_variable_size_array_does_not_recurse() {
        // 50k LEB128 elements; the count itself is a 3-byte LEB128.
        let n = 50_000u32;
        let mut bytes = vec![(n & 0x7f) as u8 | 0x80, ((n >> 7) & 0x7f) as u8 | 0x80, (n >> 14) as u8];
        for i in 0..n {
            let v = i % 300;
            if v < 128 {
                bytes.push(v as u8);
            } else {
                bytes.push((v & 0x7f) as u8 | 0x80);
                bytes.push((v >> 7) as u8);
            }
        }
        let t = Template {
            name: "t".into(),
            root: T::structure("Root", vec![("n", T::leb_u()), ("xs", T::array(T::leb_u(), E::field("n")))]),
        };
        let d = doc(&bytes);
        let mut ev = Evaluator::new(t);
        // Size of the array first, before any element is resolved.
        let xs = ev.node(&d, &[1]).unwrap();
        assert_eq!(xs.child_count, 50_000);
        assert_eq!(xs.size_bits, (bytes.len() as u64 - 3) * 8);
        assert_eq!(ev.node(&d, &[1, 49_999]).unwrap().value, Value::UInt(49_999 % 300));
        // Fresh evaluator, jump straight to the last element.
        let mut ev2 = Evaluator::new(ev.template().clone());
        assert_eq!(ev2.node(&d, &[1, 49_999]).unwrap().value, Value::UInt(49_999 % 300));
    }

    #[test]
    fn bitfields_read_msb_first() {
        let t = Template {
            name: "t".into(),
            root: T::structure("B", vec![("a", T::UInt { bits: 3, endian: Big }), ("b", T::UInt { bits: 5, endian: Big })]),
        };
        let d = doc(&[0b101_01100]);
        let mut ev = Evaluator::new(t);
        assert_eq!(ev.node(&d, &[0]).unwrap().value, Value::UInt(0b101));
        assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::UInt(0b01100));
        assert_eq!(ev.node(&d, &[1]).unwrap().offset_bits, 3);
    }

    #[test]
    fn writing_a_field_hits_only_its_own_bits() {
        let t = Template {
            name: "t".into(),
            root: T::structure(
                "B",
                vec![
                    ("a", T::UInt { bits: 3, endian: Big }),
                    ("b", T::UInt { bits: 5, endian: Big }),
                    ("n", T::u16(Little)),
                    ("tag", T::utf8(E::lit(4))),
                ],
            ),
        };
        let mut d = doc(&[0b101_01100, 0x34, 0x12, b'I', b'H', b'D', b'R']);
        let mut ev = Evaluator::new(t);
        assert!(ev.node(&d, &[0]).unwrap().editable);
        assert!(!ev.node(&d, &[]).unwrap().editable);

        for (path, text) in [(vec![1], "31"), (vec![2], "0xbeef"), (vec![3], "iend")] {
            let w = ev.prepare_write(&d, &path, text).unwrap();
            d.overwrite_bits(w.offset_bits, &w.data, w.n_bits);
            ev.invalidate();
        }
        assert_eq!(ev.node(&d, &[0]).unwrap().value, Value::UInt(0b101));
        assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::UInt(31));
        assert_eq!(ev.node(&d, &[2]).unwrap().value, Value::UInt(0xbeef));
        assert_eq!(ev.node(&d, &[3]).unwrap().value, Value::Str("iend".into()));

        let mut out = [0u8; 7];
        d.read_bytes(0, &mut out);
        assert_eq!(out, [0b101_11111, 0xef, 0xbe, b'i', b'e', b'n', b'd']);

        // Rejections carry a reason and leave the document alone.
        assert!(matches!(ev.prepare_write(&d, &[1], "32"), Err(EvalError::Failed(_))));
        assert!(matches!(ev.prepare_write(&d, &[3], "toolong"), Err(EvalError::Failed(_))));
        assert!(matches!(ev.prepare_write(&d, &[], "1"), Err(EvalError::Failed(_))));
    }
}
