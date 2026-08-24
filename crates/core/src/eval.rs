//! Lazy template evaluation over a document.
//!
//! Nodes are addressed by path (child indices from the root). Offsets and sizes
//! are memoised per path and thrown away when the document changes. A read that
//! touches an unloaded chunk yields `EvalError::Pending` rather than a value, so
//! zero-filled bytes can never be mistaken for data.

use std::collections::HashMap;

use crate::bits::bytes_for;
use crate::decode::{be_int, f16_to_f64, fixed_bits, read_int, read_uint};
use crate::document::Document;
use crate::encode;
use crate::source::{Missing, Source};
use crate::template::{Anchor, Encoding, Expr, StrLen, Template, Ty, Until};
use crate::text::{self, Settled};

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
    /// A named integer. `name` is None when the file holds a value the enum
    /// does not list. `hex` is how the number should be shown.
    Enum { raw: i128, name: Option<String>, hex: bool },
    /// An integer read as independent bits. `set` names the bits that are on,
    /// in bit order; `unnamed` counts the bits that are on and have no name,
    /// which is worth saying rather than hiding.
    Flags { raw: u128, set: Vec<String>, unnamed: u32 },
}

impl Value {
    pub fn as_int(&self) -> Option<i128> {
        match self {
            Value::UInt(v) => i128::try_from(*v).ok(),
            Value::Int(v) => Some(*v),
            Value::Composite { count } => Some(*count as i128),
            Value::Enum { raw, .. } => Some(*raw),
            Value::Flags { raw, .. } => i128::try_from(*raw).ok(),
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
    /// How many bytes of the field the value occupies. Same as the field's size
    /// except for padded and terminated text, where the padding and the
    /// terminator are the format's business, not the value's.
    pub value_bytes: u64,
    /// Where the value starts, which is past a byte-order mark if there is one.
    pub value_offset_bits: u64,
    /// How the encoding was settled when the template did not say outright, or
    /// that the bytes do not fit the encoding the template named.
    pub read_as: Option<String>,
}

/// One entry in the annotation column: a field, a run of them, or a stretch
/// the template does not describe. Produced by `Evaluator::spans`.
#[derive(Debug, Clone)]
pub struct Span {
    pub path: Vec<usize>,
    pub offset_bits: u64,
    pub size_bits: u64,
    pub name: String,
    /// What it sits inside, outermost first.
    pub trail: Vec<String>,
    pub type_name: String,
    pub value: Value,
    /// No field covers these bits.
    pub gap: bool,
    /// How many fields this entry stands for, when a run of numbers is shown
    /// as one. Zero for a single field.
    pub count: u64,
    /// A structure marked to read on one row, already joined: `local.get 0`
    /// rather than an `op` row and an `imm` row. None for everything else,
    /// which reads as its own value.
    pub line: Option<String>,
    /// The first few values of a run shown as one entry. `512 values` says how
    /// many and nothing about what, and a run of zeroes and a run of samples
    /// are worth telling apart without opening either.
    pub sample: Vec<String>,
}

/// Values from the front of a collapsed run, at most this many.
const SAMPLE: u64 = 4;

/// One value on a shared row, which is terser than the same value on a row of
/// its own: a named number gives its name and drops the number behind it,
/// because the row already has several values competing for the eye.
fn brief(v: &Value) -> String {
    match v {
        Value::UInt(n) => n.to_string(),
        Value::Int(n) => n.to_string(),
        Value::Float(n) => format!("{n}"),
        Value::Str(s) => s.clone(),
        Value::Enum { raw, name, hex } => match name {
            Some(n) => n.clone(),
            None if *hex => format!("0x{raw:x}"),
            None => raw.to_string(),
        },
        Value::Flags { set, unnamed, .. } => {
            let mut s = set.join("|");
            if *unnamed > 0 {
                if !s.is_empty() {
                    s.push('|');
                }
                let _ = std::fmt::Write::write_fmt(&mut s, format_args!("{unnamed} more"));
            }
            s
        }
        Value::Bytes { len, preview } => {
            let s: String = preview.iter().map(|b| format!("{b:02x} ")).collect();
            let s = s.trim_end().to_string();
            if *len as usize > preview.len() { format!("{s}…") } else { s }
        }
        // Nothing to say when the bytes are what the format asked for. The
        // mismatch is the only half worth a reader's attention.
        Value::Magic { ok } => (if *ok { "" } else { "does not match" }).to_string(),
        Value::Composite { .. } => String::new(),
    }
}

/// A run of these is worth one entry rather than one each.
const COLLAPSE_RUN: u64 = 8;

/// A type that holds one number or one run of bytes, and nothing inside it.
fn plain(ty: &Ty) -> bool {
    match ty {
        Ty::Enum { inner, .. } | Ty::Flags { inner, .. } => plain(inner),
        Ty::UInt { .. }
        | Ty::Int { .. }
        | Ty::F16(_)
        | Ty::F32(_)
        | Ty::F64(_)
        | Ty::Fixed { .. }
        | Ty::Leb128 { .. }
        | Ty::Vlq
        | Ty::SqliteVarint
        | Ty::Magic(_)
        | Ty::Bytes(_) => true,
        _ => false,
    }
}

/// Bits to write, and where. Produced by `Evaluator::prepare_write`.
#[derive(Debug, Clone, PartialEq)]
pub struct Write {
    pub offset_bits: u64,
    /// MSB-first packed, `n_bits` long.
    pub data: Vec<u8>,
    pub n_bits: u64,
}

/// Where a text field's value sits inside it, and what its bytes mean.
struct StrSpan {
    /// Bytes before the value: a byte-order mark, or none.
    start: u64,
    /// Bytes of value, before any padding or terminator.
    len: u64,
    settled: Settled,
    /// The padding holds something other than padding.
    dirty: bool,
    /// How the encoding was decided, when the template did not say outright.
    note: Option<String>,
}

/// Index of `term` in `hay`, aligned to whole units of its length.
fn find_unit(hay: &[u8], term: &[u8]) -> Option<usize> {
    let unit = term.len();
    (0..hay.len().saturating_sub(unit - 1)).step_by(unit).find(|i| hay[*i..*i + unit] == *term)
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
    /// A computed field's value, once worked out. Element `n` of a list asks
    /// element `n - 1` for its value, so without this a track of ten thousand
    /// events is ten thousand deep rather than one.
    computed: Option<i128>,
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
            Ty::Array { .. } | Ty::Repeat { .. } | Ty::PointerList { .. } => {
                let n = self.child_count(doc, path)?;
                (Value::Composite { count: n }, n, true)
            }
            _ => (self.primitive_value(doc, path, &r, &r.ty, size)?, 0, false),
        };
        let reading = self.reading(doc, &r, size)?;
        Ok(NodeInfo {
            path: path.to_vec(),
            editable: !composite && encode::editable(&r.ty, size) && self.padding_is_clean(doc, &r, size)? && !reading.1,
            value_offset_bits: reading.0 .0,
            value_bytes: reading.0 .1,
            read_as: reading.2,
            name: self.label(doc, path, &r)?,
            type_name: r.ty.display_name(),
            offset_bits: r.offset,
            size_bits: size,
            value,
            child_count,
            composite,
        })
    }

    /// The deepest field containing `bit`, as a path from the root. Its
    /// ancestors are the prefixes of that path, so one call gives the whole
    /// chain the hex cursor is standing in.
    ///
    /// Walking a repeat has to resolve its elements, so on a large templated
    /// file this costs what displaying it costs; the memo makes the second call
    /// cheap until the next edit.
    /// What to call this node: its field name, and what the structure says
    /// names it. `[9]` alone does not say which section it is; `[9] code`
    /// does, and keeping the index says which of the two custom ones this is.
    ///
    /// This is worked out here rather than when the node is resolved, because
    /// it means reading a sibling and resolving has to stay cheap.
    fn label<S: Source>(&mut self, doc: &Document<S>, path: &[usize], r: &Resolved) -> R<String> {
        let Ty::Struct(s) = r.ty.base() else { return Ok(r.name.clone()) };
        let Some(by) = s.named_by.clone() else { return Ok(r.name.clone()) };
        let Some(i) = s.fields.iter().position(|f| f.name == by) else { return Ok(r.name.clone()) };
        let mut child = path.to_vec();
        child.push(i);
        // A field that cannot be read yet leaves the node with the name it had.
        let Ok(info) = self.node(doc, &child) else { return Ok(r.name.clone()) };
        let text = brief(&info.value);
        let text = text.trim_end();
        Ok(if text.is_empty() { r.name.clone() } else { format!("{} {text}", r.name) })
    }

    pub fn locate<S: Source>(&mut self, doc: &Document<S>, bit: u64) -> R<Vec<usize>> {
        let mut path: Vec<usize> = Vec::new();
        self.resolve(doc, &path)?;
        let size = self.size_of(doc, &path)?;
        let root = self.memo[&path].clone();
        if bit < root.offset || bit >= root.offset + size {
            return fail("outside the template");
        }
        loop {
            let n = self.child_count(doc, &path)?;
            if n == 0 {
                return Ok(path);
            }
            match self.child_at(doc, &path, n, bit)? {
                Some(i) => path.push(i),
                // Inside the parent but in none of its children: padding, or a
                // struct whose fields do not fill it.
                None => return Ok(path),
            }
        }
    }

    /// The outermost enclosing structure that reads as one row, or the field
    /// itself when nothing on the way to it is marked that way. `locate` has
    /// already resolved every step, so this only reads what it left behind.
    fn inline_ancestor(&self, path: &[usize]) -> Vec<usize> {
        for n in 0..path.len() {
            let prefix = &path[..n];
            if let Some(r) = self.memo.get(prefix) {
                if matches!(r.ty.base(), Ty::Struct(s) if s.inline) {
                    return prefix.to_vec();
                }
            }
        }
        path.to_vec()
    }

    /// Every field across a stretch of the file, in order, for the annotation
    /// column. One call covers what is on screen rather than one field, so the
    /// column can be drawn without a round trip per byte.
    ///
    /// Two things are not one field each. A stretch no field covers, which is
    /// the slack at the end of a structure, comes back as a gap. A long run of
    /// plain numbers, such as W4V's 512 codes, comes back as the run itself:
    /// several hundred entries reading `[0]`, `[1]`, `[2]` would fill the
    /// column with less than one entry saying what the run is.
    pub fn spans<S: Source>(&mut self, doc: &Document<S>, from: u64, to: u64, max: usize) -> R<Vec<Span>> {
        self.resolve(doc, &[])?;
        let root_size = self.size_of(doc, &[])?;
        let root_offset = self.memo[&Vec::new()].offset;
        let end = to.min(root_offset + root_size);
        let mut at = from.max(root_offset);
        let mut out: Vec<Span> = Vec::new();
        while at < end && out.len() < max {
            let path = self.locate(doc, at)?;
            // A structure marked to read on one row stands for its fields here.
            let path = self.inline_ancestor(&path);
            let inline = matches!(self.memo[&path].ty.base(), Ty::Struct(s) if s.inline);
            let info = self.node(doc, &path)?;
            let mut span = self.span_of(doc, &path, &info)?;
            if inline {
                let mut parts = Vec::new();
                self.one_line(doc, &path, &mut parts)?;
                span.line = Some(parts.join(" "));
            } else if info.composite {
                // Inside it, but in none of its children: the template has
                // nothing to say about these bytes.
                span.gap = true;
                span.offset_bits = at;
                let mut ends = info.offset_bits + info.size_bits;
                if matches!(self.memo[&path].ty, Ty::PointerList { .. }) {
                    if let Some(next) = self.next_child_start(doc, &path, at)? {
                        ends = ends.min(next);
                    }
                }
                span.size_bits = ends - at;
                span.count = 0;
            } else if let Some((run, count)) = self.collapsible(doc, &path)? {
                let run_info = self.node(doc, &run)?;
                span = self.span_of(doc, &run, &run_info)?;
                span.count = count;
                for i in 0..count.min(SAMPLE) {
                    let mut elem = run.clone();
                    elem.push(i as usize);
                    span.sample.push(brief(&self.node(doc, &elem)?.value));
                }
                // A run of values that each read as nothing, such as matching
                // signatures, is better left to say only how many there are.
                if span.sample.iter().all(|s| s.is_empty()) {
                    span.sample.clear();
                }
            }
            let next = span.offset_bits + span.size_bits;
            at = if next > at { next } else { at + 8 };
            if span.size_bits > 0 {
                out.push(span);
            }
        }
        Ok(out)
    }

    fn span_of<S: Source>(&mut self, doc: &Document<S>, path: &[usize], info: &NodeInfo) -> R<Span> {
        let mut trail = Vec::new();
        for k in 1..path.len() {
            self.resolve(doc, &path[..k])?;
            // A field a structure calls its contents adds a step to the trail
            // and nothing to what it says.
            if self.is_contents(&path[..k]) {
                continue;
            }
            let r = self.memo[&path[..k]].clone();
            trail.push(self.label(doc, &path[..k], &r)?);
        }
        Ok(Span {
            path: path.to_vec(),
            offset_bits: info.offset_bits,
            size_bits: info.size_bits,
            name: info.name.clone(),
            trail,
            type_name: info.type_name.clone(),
            value: info.value.clone(),
            gap: false,
            count: 0,
            line: None,
            sample: Vec::new(),
        })
    }

    /// Whether this node is the field its parent calls its own contents.
    fn is_contents(&self, path: &[usize]) -> bool {
        let Some((&last, parent)) = path.split_last() else { return false };
        let Some(r) = self.memo.get(parent) else { return false };
        let Ty::Struct(s) = r.ty.base() else { return false };
        let Some(by) = &s.contents else { return false };
        s.fields.get(last).is_some_and(|f| &f.name == by)
    }

    /// A structure that reads on one row, as its fields' values in order. A
    /// field that is itself a structure contributes its own fields, so a wasm
    /// instruction whose immediate has two parts still reads as one line.
    fn one_line<S: Source>(&mut self, doc: &Document<S>, path: &[usize], out: &mut Vec<String>) -> R<()> {
        let info = self.node(doc, path)?;
        if !info.composite {
            // A field of no bits is an absence, not an empty value: the switch
            // for an opcode with no immediate selects one.
            if info.size_bits > 0 {
                let text = brief(&info.value);
                if !text.is_empty() {
                    out.push(text);
                }
            }
            return Ok(());
        }
        for i in 0..info.child_count as usize {
            let mut child = path.to_vec();
            child.push(i);
            self.one_line(doc, &child, out)?;
        }
        Ok(())
    }

    /// The nearest run of plain numbers `path` sits in, if it is long enough to
    /// be worth showing as one entry.
    fn collapsible<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Option<(Vec<usize>, u64)>> {
        for k in 0..path.len() {
            let ty = self.memo[&path[..k]].ty.clone();
            let elem = match &ty {
                Ty::Array { elem, .. } | Ty::Repeat { elem, .. } => (**elem).clone(),
                _ => continue,
            };
            // Text stays one entry per line: GUANO lines are each worth reading.
            if !plain(&elem) {
                continue;
            }
            let n = self.child_count(doc, &path[..k])?;
            if n >= COLLAPSE_RUN {
                return Ok(Some((path[..k].to_vec(), n)));
            }
        }
        Ok(None)
    }

    /// Which child of `path` covers `bit`, if any.
    fn child_at<S: Source>(&mut self, doc: &Document<S>, path: &[usize], n: u64, bit: u64) -> R<Option<usize>> {
        let r = self.memo[path].clone();
        // Fixed-size elements: go straight to the one that covers the bit.
        if let Ty::Array { elem, .. } | Ty::Repeat { elem, .. } = &r.ty {
            if let Some(each) = fixed_bits(elem) {
                if each > 0 {
                    let i = (bit - r.offset) / each;
                    return Ok(if i < n { Some(i as usize) } else { None });
                }
            }
        }
        // Children of a pointer list are in the order their offsets are in,
        // not the order they sit in, so every one has to be looked at, and one
        // that does not parse is passed over rather than taking the page with it.
        let scattered = matches!(r.ty, Ty::PointerList { .. });
        let mut p = path.to_vec();
        for i in 0..n as usize {
            p.push(i);
            let placed = match self.resolve(doc, &p) {
                Ok(()) => self.size_of(doc, &p).map(|size| (self.memo[&p].offset, size)),
                Err(e) => Err(e),
            };
            p.pop();
            let (off, size) = match placed {
                Ok(v) => v,
                Err(e) if scattered && !matches!(e, EvalError::Pending(_)) => continue,
                Err(e) => return Err(e),
            };
            if bit < off && !scattered {
                return Ok(None);
            }
            if bit >= off && bit < off + size {
                return Ok(Some(i));
            }
        }
        Ok(None)
    }

    /// The first child of a pointer list that starts after `bit`. What is
    /// between them belongs to no field, and saying so needs to know where the
    /// next one begins: free space inside a page sits between cells, not after
    /// all of them.
    fn next_child_start<S: Source>(&mut self, doc: &Document<S>, path: &[usize], bit: u64) -> R<Option<u64>> {
        let n = self.child_count(doc, path)?;
        let mut best: Option<u64> = None;
        let mut p = path.to_vec();
        for i in 0..n as usize {
            p.push(i);
            let placed = self.resolve(doc, &p).map(|()| self.memo[&p].offset);
            p.pop();
            match placed {
                Ok(off) if off > bit => best = Some(best.map_or(off, |b: u64| b.min(off))),
                Ok(_) => {}
                Err(e @ EvalError::Pending(_)) => return Err(e),
                Err(_) => {}
            }
        }
        Ok(best)
    }

    /// The whole text of a text field, decoded in its own encoding, up to the
    /// length that can be edited. The node's value is only a preview.
    pub fn text_value<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<(String, bool)> {
        self.resolve(doc, path)?;
        let size = self.size_of(doc, path)?;
        let r = self.memo[path].clone();
        let Some(span) = self.str_span(doc, &r, size)? else { return fail("not a text field") };
        let shown = span.len.min(crate::encode::EDIT_LIMIT_BYTES);
        let bytes = self.read(doc, &r, r.offset + span.start * 8, shown * 8)?;
        let (text, _) = text::decode_settled(span.settled, &bytes);
        Ok((text, span.len > shown))
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
                Ty::Bytes(_) | Ty::Str { .. } => format!(
                    "Too long to edit: {} bytes; the limit is {}. Use the hex view.",
                    encode::commas(size / 8),
                    encode::commas(encode::EDIT_LIMIT_BYTES)
                ),
                _ => "This field can't be edited here. Use the hex view.".to_string(),
            });
        }
        if !self.padding_is_clean(doc, &r, size)? {
            return fail(match &r.ty {
                Ty::Str { len: StrLen::Terminated { end, .. }, .. } => format!(
                    "This text has no 0x{end:02x} to end it; writing would add one and make the field longer. Use the hex view."
                ),
                Ty::Str { len: StrLen::Padded { pad, .. }, .. } => format!(
                    "Bytes after the first 0x{pad:02x} aren't shown here; writing would overwrite them. Use the hex view."
                ),
                _ => "This field can't be edited here. Use the hex view.".to_string(),
            });
        }
        // A text field is written back in the encoding it was read in, with the
        // byte-order mark it already had.
        let state = match self.str_span(doc, &r, size)? {
            Some(span) => encode::StrState {
                settled: Some(span.settled),
                bom: if span.start == 0 { Vec::new() } else { self.read(doc, &r, r.offset, span.start * 8)? },
            },
            None => encode::StrState::default(),
        };
        let data = encode::encode(&r.ty, text, size, &state).map_err(EvalError::Failed)?;
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
            Ty::Array { elem, .. } | Ty::Repeat { elem, .. } | Ty::PointerList { elem, .. } => {
                (format!("[{idx}]"), (**elem).clone())
            }
            _ => return fail("not a composite"),
        };
        // Offset: read from the pointer array, or after the previous sibling,
        // or at the parent's start.
        let offset = if matches!(pr.ty, Ty::PointerList { .. }) {
            self.pointer_offset(doc, parent, &pr, idx)?
        } else if idx == 0 {
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

    /// Where child `idx` of a pointer list starts. The offsets are bytes from
    /// the anchor, so a child can sit anywhere in the list's stretch, in any
    /// order. One that points outside it is an error for that child alone.
    fn pointer_offset<S: Source>(&mut self, doc: &Document<S>, list: &[usize], lr: &Resolved, idx: usize) -> R<u64> {
        let Ty::PointerList { offsets, anchor, adjust, .. } = &lr.ty else {
            return fail("not a pointer list");
        };
        let (offsets, anchor, adjust) = (offsets.clone(), *anchor, adjust.clone());
        let base = match anchor {
            Anchor::File => 0,
            // The nearest enclosing window, which is the page or the table the
            // offsets are counted inside.
            Anchor::Window => (0..list.len())
                .rev()
                .find_map(|k| self.memo.get(&list[..k]).filter(|r| r.declared_size.is_some()))
                .map(|r| r.offset)
                .unwrap_or(0),
        };
        let at = self.eval_expr(doc, list, &Expr::Elem { array: offsets, index: Box::new(Expr::Lit(idx as i128)) })?;
        let adj = self.eval_expr(doc, list, &adjust)?;
        let bits = base as i128 + (at + adj) * 8;
        if bits < lr.offset as i128 || bits >= lr.limit as i128 {
            return fail(format!("offset {at} points outside {}", lr.name));
        }
        Ok(bits as u64)
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
        let mut hops = 0;
        loop {
            match ty {
                Ty::Named(n) => {
                    hops += 1;
                    if hops > 64 {
                        return fail(format!("type {n} refers to itself with nothing in between"));
                    }
                    match self.template.types.get(&n) {
                        Some(t) => ty = t.clone(),
                        None => return fail(format!("no type named {n} in this template")),
                    }
                }
                Ty::Sized { size, inner } => {
                    let bytes = self.eval_expr_at(doc, path, &size, Some((offset, limit)))?;
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
                    // The node is not in the memo yet, so where it starts has
                    // to be handed over: a switch that looks at the byte it is
                    // about to read needs to know where that byte is.
                    let v = self.eval_expr_at(doc, path, &on, Some((offset, limit)))?;
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
                        computed: None,
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
                Ty::Bytes(e) => {
                    let n = self.eval_expr(doc, path, e)?;
                    if n < 0 {
                        return fail("negative length");
                    }
                    n as u64 * 8
                }
                Ty::Str { len, enc } => match len {
                    StrLen::Fixed(e) | StrLen::Padded { size: e, .. } => {
                        let n = self.eval_expr(doc, path, e)?;
                        if n < 0 {
                            return fail("negative length");
                        }
                        n as u64 * 8
                    }
                    StrLen::Terminated { end, or_end } => {
                        let (settled, bom) = self.str_head(doc, &r, enc)?;
                        let term = text::unit_bytes(settled, *end);
                        match self.read_terminated(doc, &r, &term, bom) {
                            Ok((_, n)) => n * 8,
                            // No terminator: the field runs to the end of its
                            // container, if the format allows for that.
                            Err(e) => {
                                if *or_end && !matches!(e, EvalError::Pending(_)) {
                                    r.limit - r.offset
                                } else {
                                    return Err(e);
                                }
                            }
                        }
                    }
                },
                Ty::Leb128 { .. } => {
                    let (_, n) = self.read_leb(doc, &r)?;
                    n * 8
                }
                Ty::Vlq => {
                    let (_, n) = self.read_vlq(doc, &r)?;
                    n * 8
                }
                Ty::Enum { inner, .. } | Ty::Flags { inner, .. } => match **inner {
                    Ty::Leb128 { .. } => {
                        let (_, n) = self.read_leb(doc, &r)?;
                        n * 8
                    }
                    Ty::Vlq => {
                        let (_, n) = self.read_vlq(doc, &r)?;
                        n * 8
                    }
                    Ty::SqliteVarint => self.read_sqlite_varint(doc, &r)?.1 * 8,
                    _ => return fail("enum over a type with no fixed size"),
                },
                // A pointer list holds the stretch its offsets point into,
                // which runs to the end of its container.
                Ty::PointerList { .. } => r.limit - r.offset,
                Ty::SqliteVarint => self.read_sqlite_varint(doc, &r)?.1 * 8,
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
            // As many children as the array of offsets has entries.
            Ty::PointerList { offsets, .. } => {
                let n = self.eval_expr(doc, path, &Expr::Ref(offsets.clone()))?;
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
        let here = self.memo.get(at).map(|r| (r.offset, r.limit));
        self.eval_expr_at(doc, at, e, here)
    }

    /// `here` is the field's own start and its container's limit, which is what
    /// `Remaining` measures. It has to be passed in while a node is still being
    /// resolved, since it is not in the memo yet.
    fn eval_expr_at<S: Source>(
        &mut self,
        doc: &Document<S>,
        at: &[usize],
        e: &Expr,
        here: Option<(u64, u64)>,
    ) -> R<i128> {
        Ok(match e {
            Expr::Lit(v) => *v,
            Expr::Remaining => match here {
                Some((offset, limit)) if limit >= offset => ((limit - offset) / 8) as i128,
                _ => return fail("nothing to measure the rest of"),
            },
            // The index of the element this sits in, which is what a field
            // whose type comes from a list read earlier needs.
            Expr::Idx => {
                let mut cur = at.to_vec();
                let mut found = 0;
                while let Some(idx) = cur.pop() {
                    let listy = self
                        .memo
                        .get(&cur)
                        .map(|r| matches!(r.ty, Ty::Array { .. } | Ty::Repeat { .. } | Ty::PointerList { .. }));
                    if listy == Some(true) {
                        found = idx as i128;
                        break;
                    }
                }
                found
            }
            Expr::Elem { array, index } => {
                let i = self.eval_expr_at(doc, at, index, here)?;
                if i < 0 {
                    return fail("negative index");
                }
                let Some(mut p) = self.find_field(at, array) else {
                    return fail(format!("unknown field {array}"));
                };
                p.push(i as usize);
                match self.node(doc, &p)?.value.as_int() {
                    Some(v) => v,
                    None => return fail(format!("{array}[{i}] is not a number")),
                }
            }
            Expr::Ref(name) => match self.lookup(doc, at, name)? {
                (Some(v), _) => v,
                (None, _) => return fail(format!("{name} is not a number")),
            },
            Expr::SizeOf(name) => self.lookup(doc, at, name)?.1,
            // Read where this field starts without taking the bits: what a
            // field that exists only when the byte says so has to ask.
            Expr::Peek(bits) => {
                let Some((offset, limit)) = here else { return fail("nothing to look at") };
                if offset + u64::from(*bits) > limit {
                    return fail("looks past the end of its container");
                }
                let mut buf = vec![0u8; bytes_for(u64::from(*bits))];
                let missing = doc.read_bits(offset, u64::from(*bits), &mut buf);
                if !missing.is_empty() {
                    return Err(EvalError::Pending(missing));
                }
                read_uint(&buf, *bits, crate::template::Endian::Big) as i128
            }
            Expr::Prev(name) => self.prev_field(doc, at, name)?,
            Expr::Or(a, b) => match self.eval_expr_at(doc, at, a, here)? {
                0 => self.eval_expr_at(doc, at, b, here)?,
                v => v,
            },
            Expr::Add(a, b) => self.eval_expr_at(doc, at, a, here)? + self.eval_expr_at(doc, at, b, here)?,
            Expr::Sub(a, b) => self.eval_expr_at(doc, at, a, here)? - self.eval_expr_at(doc, at, b, here)?,
            Expr::Mul(a, b) => self.eval_expr_at(doc, at, a, here)? * self.eval_expr_at(doc, at, b, here)?,
            Expr::Div(a, b) => {
                let d = self.eval_expr_at(doc, at, b, here)?;
                if d == 0 {
                    return fail("division by zero");
                }
                self.eval_expr_at(doc, at, a, here)? / d
            }
        })
    }

    /// Field `name` of the element before this one, in the nearest enclosing
    /// list. Zero for the first element and outside a list, which is what lets
    /// `Or` fall through to the case for a message with no state behind it.
    ///
    /// The elements of a list are resolved in order, so by the time element `n`
    /// asks, element `n - 1` is already in the memo: this is a lookup, not a
    /// walk back to the start.
    fn prev_field<S: Source>(&mut self, doc: &Document<S>, at: &[usize], name: &str) -> R<i128> {
        let mut cur = at.to_vec();
        while let Some(idx) = cur.pop() {
            let listy = matches!(
                self.memo.get(&cur).map(|r| &r.ty),
                Some(Ty::Array { .. } | Ty::Repeat { .. } | Ty::PointerList { .. })
            );
            if !listy {
                continue;
            }
            if idx == 0 {
                return Ok(0);
            }
            let mut elem = cur.clone();
            elem.push(idx - 1);
            self.resolve(doc, &elem)?;
            let Ty::Struct(s) = self.memo[&elem].ty.base() else { return Ok(0) };
            let Some(j) = s.fields.iter().position(|f| f.name == name) else { return Ok(0) };
            elem.push(j);
            return Ok(self.node(doc, &elem)?.value.as_int().unwrap_or(0));
        }
        Ok(0)
    }

    /// The path of the field named `name`, found the way `lookup` finds it.
    fn find_field(&self, at: &[usize], name: &str) -> Option<Vec<usize>> {
        let mut cur = at.to_vec();
        while let Some(idx) = cur.pop() {
            if let Some(Ty::Struct(s)) = self.memo.get(&cur).map(|r| &r.ty) {
                if let Some(j) = s.fields.iter().take(idx).position(|f| f.name == name) {
                    let mut p = cur.clone();
                    p.push(j);
                    return Some(p);
                }
            }
        }
        None
    }

    /// Find `name` among the fields before `at` in its struct, then in
    /// enclosing structs. Returns its value and its size in bytes.
    fn lookup<S: Source>(&mut self, doc: &Document<S>, at: &[usize], name: &str) -> R<(Option<i128>, i128)> {
        let mut cur = at.to_vec();
        while !cur.is_empty() {
            let idx = cur.pop().expect("non-empty");
            let parent = cur.clone();
            if let Ty::Struct(s) = &self.memo[&parent].ty {
                if let Some(j) = s.fields[..idx].iter().position(|f| f.name == name) {
                    let mut p = parent;
                    p.push(j);
                    let info = self.node(doc, &p)?;
                    // A field with no numeric reading can still be measured.
                    return Ok((info.value.as_int(), (info.size_bits / 8) as i128));
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

    /// Where the value sits inside a text field, and how the bytes read.
    ///
    /// A byte-order mark belongs to the field but not to the value, and the
    /// padding or terminator ends it. Everything is measured in whole code
    /// units, so UTF-16LE text does not stop at the first zero byte of "H".
    fn str_span<S: Source>(&self, doc: &Document<S>, r: &Resolved, size: u64) -> R<Option<StrSpan>> {
        let Ty::Str { len, enc } = &r.ty else { return Ok(None) };
        let n = size / 8;
        let cap = n.min(crate::encode::EDIT_LIMIT_BYTES);
        let bytes = if cap == 0 { Vec::new() } else { self.read(doc, r, r.offset, cap * 8)? };
        let (settled, bom, note) = text::settle(enc, &bytes);
        let bom = (bom as u64).min(cap);
        let body = &bytes[bom as usize..];
        let unit = settled.unit();
        let rest = cap - bom;
        let (text_len, dirty) = match len {
            StrLen::Fixed(_) => (rest, false),
            StrLen::Padded { pad, .. } => {
                let term = text::unit_bytes(settled, *pad);
                match find_unit(body, &term) {
                    None => (rest, false),
                    Some(i) => {
                        let tail = &body[i..];
                        // Anything in the padding that is not padding would be
                        // lost by writing back only what is shown.
                        let dirty = !tail.chunks(unit).all(|u| u == term);
                        (i as u64, dirty)
                    }
                }
            }
            StrLen::Terminated { end, .. } => {
                let term = text::unit_bytes(settled, *end);
                match find_unit(body, &term) {
                    Some(i) => (i as u64, false),
                    // No terminator to write back, so this one is read-only.
                    None => (rest, true),
                }
            }
        };
        Ok(Some(StrSpan { start: bom, len: text_len, settled, dirty, note }))
    }

    /// Where a field's value sits, whether the bytes fit the encoding, and how
    /// the encoding was decided. Everything a node needs beyond its value.
    #[allow(clippy::type_complexity)]
    fn reading<S: Source>(&self, doc: &Document<S>, r: &Resolved, size: u64) -> R<((u64, u64), bool, Option<String>)> {
        let Some(span) = self.str_span(doc, r, size)? else { return Ok(((r.offset, size / 8), false, None)) };
        let shown = span.len.min(crate::encode::EDIT_LIMIT_BYTES);
        let bytes = self.read(doc, r, r.offset + span.start * 8, shown * 8)?;
        let (_, lossy) = text::decode_settled(span.settled, &bytes);
        let note = if lossy {
            Some(format!(
                "Not valid {}; the bad bytes show as \u{fffd}. Edit it in the hex view.",
                span.settled.name()
            ))
        } else {
            span.note
        };
        Ok(((r.offset + span.start * 8, span.len), lossy, note))
    }

    /// A padded text field shows only what is before its first pad byte. If the
    /// rest is not all padding, writing back what is shown would drop bytes the
    /// reader never saw, so such a field is not editable here.
    fn padding_is_clean<S: Source>(&self, doc: &Document<S>, r: &Resolved, size: u64) -> R<bool> {
        if size > crate::encode::EDIT_LIMIT_BYTES * 8 {
            return Ok(true); // too long to edit anyway
        }
        Ok(!self.str_span(doc, r, size)?.map(|s| s.dirty).unwrap_or(false))
    }

    /// How a text field reads before its length is known: the encoding the
    /// scanner should step in, and the bytes any byte-order mark takes.
    fn str_head<S: Source>(&self, doc: &Document<S>, r: &Resolved, enc: &Encoding) -> R<(Settled, u64)> {
        let want = 4u64.min((r.limit - r.offset) / 8);
        let head = if want == 0 { Vec::new() } else { self.read(doc, r, r.offset, want * 8)? };
        let (settled, bom, _) = text::settle(enc, &head);
        Ok((settled, bom as u64))
    }

    /// Scan for the terminator, whole code units at a time, and return the
    /// bytes of text and the bytes of the whole field. Read in blocks: a long
    /// string should not be one call per unit.
    fn read_terminated<S: Source>(&self, doc: &Document<S>, r: &Resolved, term: &[u8], bom: u64) -> R<(u64, u64)> {
        const BLOCK: u64 = 256;
        /// A file with no terminator in it must fail rather than walk to the end.
        const CAP: u64 = 64 * 1024;
        let unit = term.len() as u64;
        let start = r.offset + bom * 8;
        let stop = r.limit.min(start + CAP * 8);
        let mut at = start;
        let mut text_bytes = 0u64;
        while at < stop {
            let mut n = BLOCK.min((stop - at) / 8);
            n -= n % unit;
            if n == 0 {
                break;
            }
            let block = self.read(doc, r, at, n * 8)?;
            for i in (0..block.len()).step_by(unit as usize) {
                if block[i..i + unit as usize] == *term {
                    let len = text_bytes + i as u64;
                    return Ok((len, bom + len + unit));
                }
            }
            text_bytes += n;
            at += n * 8;
        }
        fail(format!("no 0x{:02x} terminator within {} bytes", term[0], (stop - start) / 8))
    }

    fn read_leb<S: Source>(&self, doc: &Document<S>, r: &Resolved) -> R<(u128, u64)> {
        let mut value: u128 = 0;
        let mut shift = 0;
        for i in 0..10u64 {
            let b = self.read(doc, r, r.offset + i * 8, 8)?[0];
            value |= ((b & 0x7f) as u128) << shift;
            shift += 7;
            if b & 0x80 == 0 {
                if let Ty::Leb128 { signed: true } = r.ty.base() {
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

    /// A variable-length quantity: the high bit says another byte follows, and
    /// the seven bits below it are the next group down. Four bytes is the most
    /// a Standard MIDI File is allowed to use.
    fn read_vlq<S: Source>(&self, doc: &Document<S>, r: &Resolved) -> R<(u128, u64)> {
        let mut value: u128 = 0;
        for i in 0..4u64 {
            let b = self.read(doc, r, r.offset + i * 8, 8)?[0];
            value = (value << 7) | (b & 0x7f) as u128;
            if b & 0x80 == 0 {
                return Ok((value, i + 1));
            }
        }
        fail("variable-length number longer than 4 bytes")
    }

    /// SQLite's varint: seven bits per byte, most significant group first, and
    /// a ninth byte that contributes all eight of its bits. The result is
    /// 64-bit two's complement, so a negative row id reads as one.
    fn read_sqlite_varint<S: Source>(&self, doc: &Document<S>, r: &Resolved) -> R<(i128, u64)> {
        let mut value: u64 = 0;
        for i in 0..8u64 {
            let b = self.read(doc, r, r.offset + i * 8, 8)?[0];
            value = (value << 7) | (b & 0x7f) as u64;
            if b & 0x80 == 0 {
                return Ok((value as i64 as i128, i + 1));
            }
        }
        let last = self.read(doc, r, r.offset + 64, 8)?[0];
        value = (value << 8) | last as u64;
        Ok((value as i64 as i128, 9))
    }

    fn primitive_value<S: Source>(&mut self, doc: &Document<S>, at: &[usize], r: &Resolved, ty: &Ty, size: u64) -> R<Value> {
        Ok(match ty {
            Ty::UInt { bits, endian } => Value::UInt(read_uint(&self.read(doc, r, r.offset, size)?, *bits, *endian)),
            Ty::Int { bits, endian } => Value::Int(read_int(&self.read(doc, r, r.offset, size)?, *bits, *endian)),
            Ty::Fixed { bits, frac, endian, signed } => {
                let buf = self.read(doc, r, r.offset, size)?;
                let raw = if *signed { read_int(&buf, *bits, *endian) as f64 } else { read_uint(&buf, *bits, *endian) as f64 };
                Value::Float(raw / (1u64 << frac) as f64)
            }
            Ty::F16(e) => Value::Float(f16_to_f64(read_uint(&self.read(doc, r, r.offset, 16)?, 16, *e) as u16)),
            Ty::F32(e) => Value::Float(f32::from_bits(read_uint(&self.read(doc, r, r.offset, 32)?, 32, *e) as u32) as f64),
            Ty::F64(e) => Value::Float(f64::from_bits(read_uint(&self.read(doc, r, r.offset, 64)?, 64, *e) as u64)),
            Ty::Leb128 { signed } => {
                let (v, _) = self.read_leb(doc, r)?;
                if *signed { Value::Int(v as i128) } else { Value::UInt(v) }
            }
            Ty::Vlq => Value::UInt(self.read_vlq(doc, r)?.0),
            Ty::Computed(e) => {
                if let Some(v) = self.memo.get(at).and_then(|m| m.computed) {
                    return Ok(Value::Int(v));
                }
                let v = self.eval_expr_at(doc, at, e, Some((r.offset, r.limit)))?;
                if let Some(m) = self.memo.get_mut(at) {
                    m.computed = Some(v);
                }
                Value::Int(v)
            }
            Ty::SqliteVarint => Value::Int(self.read_sqlite_varint(doc, r)?.0),
            Ty::Magic(want) => Value::Magic { ok: self.read(doc, r, r.offset, size)? == *want },
            Ty::Bytes(_) => {
                let len = size / 8;
                let preview = self.read(doc, r, r.offset, len.min(16) * 8)?;
                Value::Bytes { len, preview }
            }
            Ty::Str { .. } => {
                let span = self.str_span(doc, r, size)?.expect("text field");
                let shown = span.len.min(256);
                let bytes = self.read(doc, r, r.offset + span.start * 8, shown * 8)?;
                let (mut text, _) = text::decode_settled(span.settled, &bytes);
                if span.len > shown {
                    text.push('\u{2026}');
                }
                Value::Str(text)
            }
            Ty::Enum { inner, def } => {
                let raw = match self.primitive_value(doc, at, r, inner, size)? {
                    Value::UInt(v) => i128::try_from(v).unwrap_or(i128::MAX),
                    Value::Int(v) => v,
                    _ => return fail("an enum must sit on an integer"),
                };
                Value::Enum { raw, name: def.label(raw).map(str::to_string), hex: def.hex }
            }
            Ty::Flags { inner, def } => {
                let raw = match self.primitive_value(doc, at, r, inner, size)? {
                    Value::UInt(v) => v,
                    Value::Int(v) => v as u128,
                    _ => return fail("flags must sit on an integer"),
                };
                let mut set: Vec<String> = Vec::new();
                let mut unnamed = 0u32;
                for bit in 0..size.min(128) as u32 {
                    if raw >> bit & 1 == 0 {
                        continue;
                    }
                    match def.label(bit) {
                        Some(n) => set.push(n.to_string()),
                        None => unnamed += 1,
                    }
                }
                Value::Flags { raw, set, unnamed }
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
    use crate::template::{Anchor, Endian::*, Expr as E, Ty as T};

    fn doc(bytes: &[u8]) -> Document<MemSource> {
        Document::new(MemSource(bytes.to_vec()))
    }

    #[test]
    fn spans_cover_a_stretch_without_a_call_per_field() {
        // A header, a run of numbers too long to list, and a window with room
        // left over at the end of it.
        let t = Template::new(
            "t",
            T::structure(
                "Root",
                vec![
                    ("tag", T::u16(Big)),
                    ("codes", T::array(T::u8(), E::lit(12))),
                    ("window", T::sized(E::lit(4), T::structure("Inner", vec![("a", T::u16(Big))]))),
                ],
            ),
        );
        let d = doc(&[0xab, 0xcd, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 0, 7, 0, 0]);
        let mut ev = Evaluator::new(t);

        let all = ev.spans(&d, 0, 18 * 8, 100).unwrap();
        assert_eq!(all.len(), 4);
        assert_eq!((all[0].name.as_str(), all[0].size_bits), ("tag", 16));
        assert!(all[0].trail.is_empty());

        // Twelve numbers as one entry, saying how many it stands for.
        assert_eq!(all[1].name, "codes");
        assert_eq!(all[1].count, 12);
        assert_eq!(all[1].size_bits, 12 * 8);

        assert_eq!(all[2].name, "a");
        assert_eq!(all[2].trail, vec!["window"]);
        assert_eq!(all[2].value, Value::UInt(7));

        // The two bytes the window leaves over are a gap, not a field.
        assert!(all[3].gap);
        assert_eq!(all[3].offset_bits, 16 * 8);
        assert_eq!(all[3].size_bits, 2 * 8);

        // Asking for part of the file starts at the field covering that bit,
        // whether or not the field starts there.
        let part = ev.spans(&d, 5 * 8, 8 * 8, 100).unwrap();
        assert_eq!(part.len(), 1);
        assert_eq!(part[0].name, "codes");
        assert_eq!(part[0].offset_bits, 2 * 8);

        // A shorter run stays one entry per field.
        let t2 = Template::new("t", T::array(T::u8(), E::lit(4)));
        let mut ev2 = Evaluator::new(t2);
        let each = ev2.spans(&d, 0, 4 * 8, 100).unwrap();
        assert_eq!(each.len(), 4);
        assert_eq!(each[3].name, "[3]");

        // The count is a limit, not a target.
        assert_eq!(ev2.spans(&d, 0, 4 * 8, 2).unwrap().len(), 2);
    }

    #[test]
    fn struct_with_count_driven_array() {
        let t = Template::new(
            "t",
            T::structure("Root", vec![("n", T::u8()), ("items", T::array(T::u16(Little), E::field("n")))]),
        );
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
        let t = Template::new(
            "t",
            T::repeat(
                T::structure("Rec", vec![("len", T::leb_u()), ("data", T::bytes(E::field("len")))]),
                Until::End,
            ),
        );
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
        let t = Template::new(
            "t",
            T::structure(
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
        );
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
        let t = Template::new(
            "t",
            T::structure("Root", vec![("n", T::leb_u()), ("xs", T::array(T::leb_u(), E::field("n")))]),
        );
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
        let t = Template::new(
            "t",
            T::structure("B", vec![("a", T::UInt { bits: 3, endian: Big }), ("b", T::UInt { bits: 5, endian: Big })]),
        );
        let d = doc(&[0b101_01100]);
        let mut ev = Evaluator::new(t);
        assert_eq!(ev.node(&d, &[0]).unwrap().value, Value::UInt(0b101));
        assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::UInt(0b01100));
        assert_eq!(ev.node(&d, &[1]).unwrap().offset_bits, 3);
    }

    #[test]
    fn writing_a_field_hits_only_its_own_bits() {
        let t = Template::new(
            "t",
            T::structure(
                "B",
                vec![
                    ("a", T::UInt { bits: 3, endian: Big }),
                    ("b", T::UInt { bits: 5, endian: Big }),
                    ("n", T::u16(Little)),
                    ("tag", T::utf8(E::lit(4))),
                ],
            ),
        );
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

    #[test]
    fn locate_finds_the_field_under_a_bit() {
        let t = Template::new(
            "t",
            T::structure(
                "B",
                vec![
                    ("a", T::UInt { bits: 3, endian: Big }),
                    ("b", T::UInt { bits: 5, endian: Big }),
                    ("items", T::array(T::u16(Big), E::lit(3))),
                ],
            ),
        );
        let d = doc(&[0b101_01100, 0, 1, 0, 2, 0, 3]);
        let mut ev = Evaluator::new(t);
        assert_eq!(ev.locate(&d, 0).unwrap(), vec![0]);
        assert_eq!(ev.locate(&d, 2).unwrap(), vec![0]);
        assert_eq!(ev.locate(&d, 3).unwrap(), vec![1]);
        assert_eq!(ev.locate(&d, 7).unwrap(), vec![1]);
        // Into the array: element 1 starts at byte 3.
        assert_eq!(ev.locate(&d, 8).unwrap(), vec![2, 0]);
        assert_eq!(ev.locate(&d, 3 * 8 + 4).unwrap(), vec![2, 1]);
        assert_eq!(ev.locate(&d, 6 * 8).unwrap(), vec![2, 2]);
        assert!(ev.locate(&d, 7 * 8).is_err());
    }

    #[test]
    fn text_is_read_and_written_in_its_own_encoding() {
        use crate::template::{Encoding, StrLen};
        let t = Template::new(
            "t",
            T::structure(
                "R",
                vec![
                    ("dos", T::text(StrLen::Padded { size: E::lit(8), pad: 0 }, Encoding::Cp437)),
                    ("wide", T::text(StrLen::Padded { size: E::lit(10), pad: 0 }, Encoding::Bom { fallback: Box::new(Encoding::Latin1) })),
                ],
            ),
        );
        // CP437 0xE1 is the sharp s; the rest of the field is padding.
        let mut bytes = vec![b'D', b'O', b'S', 0xe1, 0, 0, 0, 0];
        // UTF-16 LE with a byte-order mark: "Hi", then NUL units.
        bytes.extend_from_slice(&[0xff, 0xfe, b'H', 0, b'i', 0, 0, 0, 0, 0]);
        let mut d = doc(&bytes);
        let mut ev = Evaluator::new(t);

        let dos = ev.node(&d, &[0]).unwrap();
        assert_eq!(dos.value, Value::Str("DOS\u{00df}".into()));
        assert_eq!(dos.value_bytes, 4);
        assert_eq!(dos.type_name, "cp437 nul-pad");

        let wide = ev.node(&d, &[1]).unwrap();
        assert_eq!(wide.value, Value::Str("Hi".into()));
        // The mark is part of the field, not of the value.
        assert_eq!(wide.value_offset_bits, wide.offset_bits + 16);
        assert_eq!(wide.value_bytes, 4);
        assert_eq!(wide.read_as.as_deref(), Some("Read as UTF-16 LE, from a byte-order mark"));

        // Writing keeps the encoding and the mark, and pads in whole units.
        let w = ev.prepare_write(&d, &[1], "Sun").unwrap();
        assert_eq!(w.data, vec![0xff, 0xfe, b'S', 0, b'u', 0, b'n', 0, 0, 0]);
        d.overwrite_bits(w.offset_bits, &w.data, w.n_bits);
        ev.invalidate();
        assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::Str("Sun".into()));

        // A character CP437 does not have is refused, not mangled.
        assert!(matches!(ev.prepare_write(&d, &[0], "\u{20ac}"), Err(EvalError::Failed(_))));
        let w = ev.prepare_write(&d, &[0], "\u{00df}\u{00df}").unwrap();
        assert_eq!(w.data, vec![0xe1, 0xe1, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn an_enum_is_written_by_name() {
        let t = Template::new(
            "t",
            T::structure("R", vec![("kind", T::enumeration("Kind", T::u8(), &[(1, "one"), (2, "two")]))]),
        );
        let d = doc(&[1]);
        let mut ev = Evaluator::new(t);
        assert_eq!(ev.prepare_write(&d, &[0], "two").unwrap().data, vec![2]);
        assert_eq!(ev.prepare_write(&d, &[0], "9").unwrap().data, vec![9]);
        assert!(matches!(ev.prepare_write(&d, &[0], "three"), Err(EvalError::Failed(_))));
    }

    #[test]
    fn remaining_measures_to_the_end_of_the_container() {
        use crate::template::{Encoding, StrLen};
        let t = Template::new(
            "t",
            T::structure(
                "R",
                vec![
                    ("n", T::u8()),
                    ("head", T::bytes(E::field("n"))),
                    ("rest", T::bytes(E::Remaining)),
                ],
            ),
        );
        let d = doc(&[2, 0xaa, 0xbb, 1, 2, 3]);
        let mut ev = Evaluator::new(t);
        assert_eq!(ev.node(&d, &[2]).unwrap().size_bits, 3 * 8);

        // Inside a Sized window it stops at the window, not at the file.
        let t2 = Template::new(
            "t",
            T::structure(
                "R",
                vec![
                    ("win", T::sized(E::lit(3), T::structure("W", vec![("a", T::u8()), ("b", T::bytes(E::Remaining))]))),
                    ("after", T::u8()),
                ],
            ),
        );
        let mut ev2 = Evaluator::new(t2);
        assert_eq!(ev2.node(&d, &[0, 1]).unwrap().size_bits, 2 * 8);
        assert_eq!(ev2.node(&d, &[1]).unwrap().offset_bits, 3 * 8);

        // A repeat whose element takes the rest has exactly one element.
        let t3 = Template::new("t", T::repeat(T::sized(E::Remaining, T::bytes(E::Remaining)), Until::End));
        let mut ev3 = Evaluator::new(t3);
        assert_eq!(ev3.node(&d, &[]).unwrap().child_count, 1);
        let _ = StrLen::Fixed(E::lit(0));
        let _ = Encoding::Utf8;
    }

    #[test]
    fn a_last_line_without_a_terminator_still_reads() {
        use crate::template::{Encoding, StrLen};
        let line = T::text(StrLen::Terminated { end: b'\n', or_end: true }, Encoding::Utf8);
        let t = Template::new("t", T::repeat(line, Until::End));
        let d = doc(b"one\ntwo");
        let mut ev = Evaluator::new(t);
        assert_eq!(ev.node(&d, &[]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[0]).unwrap().value, Value::Str("one".into()));
        assert_eq!(ev.node(&d, &[0]).unwrap().size_bits, 4 * 8);
        let last = ev.node(&d, &[1]).unwrap();
        assert_eq!(last.value, Value::Str("two".into()));
        assert_eq!(last.size_bits, 3 * 8);
        // Nothing to write the terminator back into, so the tail is read-only.
        assert!(!last.editable);
        assert!(ev.node(&d, &[0]).unwrap().editable);

        // Without `or_end` the same bytes are an error, not a guess.
        let strict = T::text(StrLen::Terminated { end: b'\n', or_end: false }, Encoding::Utf8);
        let mut ev2 = Evaluator::new(Template::new("t", T::repeat(strict, Until::End)));
        assert!(ev2.node(&d, &[1]).is_err());
    }

    /// A template whose items sit at offsets held in an earlier array, in the
    /// order the offsets are in rather than the order they sit in.
    fn pointer_template() -> Template {
        let item = T::structure("Item", vec![("len", T::u8()), ("text", T::utf8(E::field("len")))]);
        Template::new(
            "t",
            T::structure(
                "Root",
                vec![
                    ("count", T::u8()),
                    ("ptrs", T::array(T::u16(Big), E::field("count"))),
                    ("items", T::pointer_list("ptrs", Anchor::Window, E::lit(0), item)),
                ],
            ),
        )
    }

    // count, two offsets, a byte belonging to nothing, then the two items with
    // the later one pointed at first.
    const POINTED: &[u8] = &[2, 0, 10, 0, 6, 0xff, 3, b'b', b'e', b'e', 2, b'o', b'k'];

    #[test]
    fn pointed_at_items_read_in_offset_order() {
        let d = doc(POINTED);
        let mut ev = Evaluator::new(pointer_template());
        assert_eq!(ev.node(&d, &[2]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[2, 0]).unwrap().offset_bits, 10 * 8);
        assert_eq!(ev.node(&d, &[2, 0, 1]).unwrap().value, Value::Str("ok".into()));
        assert_eq!(ev.node(&d, &[2, 1, 1]).unwrap().value, Value::Str("bee".into()));
        // The cursor finds the item that covers a byte, wherever it is in the list.
        assert_eq!(ev.locate(&d, 7 * 8).unwrap(), vec![2, 1, 1]);
        assert_eq!(ev.locate(&d, 11 * 8).unwrap(), vec![2, 0, 1]);
    }

    #[test]
    fn space_between_pointed_at_items_is_a_gap_of_its_own() {
        let d = doc(POINTED);
        let mut ev = Evaluator::new(pointer_template());
        let spans = ev.spans(&d, 5 * 8, 13 * 8, 20).unwrap();
        // The byte no offset points at, then the earlier item, then the later.
        assert!(spans[0].gap);
        assert_eq!((spans[0].offset_bits, spans[0].size_bits), (5 * 8, 8));
        assert_eq!(spans[1].name, "len");
        assert_eq!(spans[2].value, Value::Str("bee".into()));
        assert_eq!(spans[4].value, Value::Str("ok".into()));
    }

    #[test]
    fn an_offset_outside_the_list_fails_only_that_item() {
        let mut b = POINTED.to_vec();
        b[2] = 200; // the first offset now points past the end
        let d = doc(&b);
        let mut ev = Evaluator::new(pointer_template());
        assert!(ev.node(&d, &[2, 0]).is_err());
        assert_eq!(ev.node(&d, &[2, 1, 1]).unwrap().value, Value::Str("bee".into()));
        assert_eq!(ev.locate(&d, 7 * 8).unwrap(), vec![2, 1, 1]);
    }

    #[test]
    fn a_field_takes_its_type_from_a_list_read_earlier() {
        let t = Template::new(
            "t",
            T::structure(
                "Root",
                vec![
                    ("n", T::u8()),
                    ("types", T::array(T::u8(), E::field("n"))),
                    (
                        "vals",
                        T::array(
                            T::switch(E::elem("types", E::idx()), vec![(1, T::u8()), (2, T::u16(Big))], T::bytes(E::lit(0))),
                            E::field("n"),
                        ),
                    ),
                ],
            ),
        );
        let d = doc(&[2, 2, 1, 0, 5, 7]);
        let mut ev = Evaluator::new(t);
        assert_eq!(ev.node(&d, &[2, 0]).unwrap().value, Value::UInt(5));
        assert_eq!(ev.node(&d, &[2, 1]).unwrap().value, Value::UInt(7));
    }

    #[test]
    fn sqlite_varints_read_and_write_at_their_own_size() {
        let t = Template::new(
            "t",
            T::structure("Root", vec![("a", T::sqlite_varint()), ("b", T::sqlite_varint())]),
        );
        // 128 in two bytes, then -1 in the nine-byte form.
        let d = doc(&[0x81, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
        let mut ev = Evaluator::new(t);
        let a = ev.node(&d, &[0]).unwrap();
        assert_eq!((a.value.clone(), a.size_bits), (Value::Int(128), 16));
        let b = ev.node(&d, &[1]).unwrap();
        assert_eq!((b.value.clone(), b.size_bits), (Value::Int(-1), 72));
        // Writing keeps the size: 3 pads out to two bytes, -2 to nine.
        let w = ev.prepare_write(&d, &[0], "3").unwrap();
        assert_eq!((w.data, w.n_bits), (vec![0x80, 0x03], 16));
        let w = ev.prepare_write(&d, &[1], "-2").unwrap();
        assert_eq!(w.n_bits, 72);
        let d2 = doc(&w.data);
        let mut ev2 = Evaluator::new(Template::new("t", T::structure("R", vec![("v", T::sqlite_varint())])));
        assert_eq!(ev2.node(&d2, &[0]).unwrap().value, Value::Int(-2));
    }
}

/// What a type permits, as opposed to what this file happens to hold.
///
/// Three field kinds know more than their value shows: an enum knows the other
/// values it would accept, a magic field knows the bytes it wanted, and a flags
/// field knows what each bit means. This is one answer for all three, because
/// they are one question: what does this type say, beyond the number.
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
