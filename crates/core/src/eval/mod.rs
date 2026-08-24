//! Lazy template evaluation over a document.
//!
//! Nodes are addressed by path (child indices from the root). Offsets and sizes
//! are memoised per path and thrown away when the document changes. A read that
//! touches an unloaded chunk yields `EvalError::Pending` rather than a value, so
//! zero-filled bytes can never be mistaken for data.

use rustc_hash::FxHashMap;

use crate::bits::bytes_for;
use crate::decode::{be_int, f16_to_f64, fixed_bits, read_int, read_uint};
use crate::document::Document;
use crate::encode;
use crate::source::{Missing, Source};
use crate::template::{Anchor, Encoding, Expr, StrLen, Template, Ty, Until};
use crate::text::{self, Settled};

mod explain;
mod listing;
mod read;
mod walk;
#[cfg(test)]
mod tests;

pub use explain::{Explain, FlagBit};
pub use listing::Span;

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

/// What a list has learned about itself as it is walked. This is kept apart
/// from `Resolved` because resolving a child clones its parent, and a list
/// that has been walked a million elements holds a thousand checkpoints:
/// cloning those per child is what turns reading a long list into a crawl.
#[derive(Debug, Default, Clone)]
struct ListState {
    /// For `Repeat`: how many elements the walk has reached, where the last
    /// of them ends, and whether the walk reached its terminating
    /// condition. A count and one offset rather than one offset per
    /// element, since a repeat can run to millions of them.
    repeat_len: usize,
    repeat_end: Option<u64>,
    repeat_done: bool,
    /// How far a walk over this list has got: an element index and where it
    /// starts. Walking on from here is what keeps reading a list in order
    /// one step per element rather than one walk per element.
    walk_at: Option<(usize, u64)>,
    /// The offset of every thousandth element of a long list, so one can be
    /// reached without walking from the start. See `walk.rs`.
    checkpoints: Vec<(usize, u64)>,
    /// For `PointerList` with `to_next`: every child's start, sorted, worked
    /// out once so each child can find the one after it without a walk.
    pointer_starts: Option<Vec<u64>>,
    /// Children `0..seq_end` are resolved and sized, so child `seq_end` can
    /// be placed without walking back. Keeps sibling resolution iterative.
    seq_end: usize,
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
}

pub struct Evaluator {
    template: Template,
    memo: FxHashMap<Vec<usize>, Resolved>,
    /// What each list has learned about itself, for the few nodes that are
    /// lists. Kept apart from `memo` so resolving a child stays cheap.
    lists: FxHashMap<Vec<usize>, ListState>,
    /// What each guarded walk has added to the memo, so it can drop the nodes
    /// it has moved past. One entry per walk, since a list can hold a list.
    journals: Vec<walk::WalkJournal>,
}

impl Evaluator {
    pub fn new(template: Template) -> Self {
        Self {
            template,
            memo: FxHashMap::default(),
            lists: FxHashMap::default(),
            journals: Vec::new(),
        }
    }

    pub fn template(&self) -> &Template {
        &self.template
    }

    /// How many nodes are currently memoised. What a walk over a long list
    /// costs in memory is measured here rather than guessed at.
    pub fn memo_len(&self) -> usize {
        self.memo.len()
    }

    /// Drop what an overwrite at `bit` could have changed, and keep the rest.
    /// Call instead of `invalidate` when an edit replaced bits in place: an
    /// insertion or a deletion moves everything behind it, and needs the whole
    /// memo thrown away.
    ///
    /// Every expression a template can write looks backwards: a length, a
    /// count or a switch reads fields before the one it places, and the bytes
    /// after `bit` cannot have been what it read. So a node that ends at or
    /// before `bit` still starts where it did and is still the size it was,
    /// and what a list learned about its own first elements still holds.
    ///
    /// This is what makes editing a large file bearable: a byte changed in a
    /// GGUF's weights leaves the walk over its two million metadata elements
    /// standing, where throwing everything away would do that walk again.
    pub fn invalidate_from(&mut self, bit: u64) {
        self.journals.clear();
        // A node with no size worked out yet is dropped: nothing says where it
        // ends, so nothing says it ended before the edit.
        self.memo.retain(|_, r| r.size.is_some_and(|size| r.offset + size <= bit));
        self.lists.retain(|path, l| {
            l.checkpoints.retain(|(_, at)| *at <= bit);
            if l.walk_at.is_some_and(|(_, at)| at > bit) {
                l.walk_at = None;
            }
            // A repeat's count is only as good as the walk that reached it.
            if l.repeat_end.is_none_or(|end| end > bit) {
                l.repeat_len = 0;
                l.repeat_end = None;
                l.repeat_done = false;
            }
            // Where a pointer list's children start was read from a field that
            // may be anywhere, and where a sequential walk had got to counts
            // children some of which have just gone. Both are cheap to redo.
            l.pointer_starts = None;
            l.seq_end = 0;
            let empty = l.checkpoints.is_empty()
                && l.walk_at.is_none()
                && l.repeat_len == 0
                && !l.repeat_done;
            !empty || self.memo.contains_key(path)
        });
    }

    /// Drop every cached offset/size. Call after any document change that is
    /// not an overwrite, and whenever the template changes.
    pub fn invalidate(&mut self) {
        self.memo.clear();
        self.lists.clear();
        self.journals.clear();
    }

    /// What the list at `path` has learned about itself. A node that is not
    /// a list, or one nothing has been learned about yet, has learned
    /// nothing, which is what the default says.
    fn list(&self, path: &[usize]) -> ListState {
        self.lists.get(path).cloned().unwrap_or_default()
    }

    fn list_mut(&mut self, path: &[usize]) -> &mut ListState {
        self.lists.entry(path.to_vec()).or_default()
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
            self.remember(&[], r);
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
            // Place after the previous sibling, walking the elements in
            // between. A long list drops what the walk moves past, so this
            // stays bounded in memory; see `walk.rs`.
            // Note: asking for an element past a Repeat's terminating condition
            // is not prevented here; `children()` clamps, direct callers must too.
            self.walk_to(doc, parent, idx)?
        };
        if offset > pr.limit {
            return fail("runs past the end of its container");
        }
        // A pointer-list child with no size of its own runs to the next child
        // above it: its limit is that child's start.
        let mut limit = pr.limit;
        if let Ty::PointerList { to_next: true, .. } = &pr.ty {
            let starts = self.pointer_starts(doc, parent, &pr)?;
            if let Some(next) = starts.get(starts.partition_point(|s| *s <= offset)) {
                limit = limit.min(*next);
            }
        }
        let r = self.effective(doc, path, name, ty, offset, limit)?;
        self.remember(path, r);
        Ok(())
    }

    /// Where child `idx` of a pointer list starts. The offsets are bytes from
    /// the anchor, so a child can sit anywhere in the list's stretch, in any
    /// order. One that points outside it is an error for that child alone.
    fn pointer_offset<S: Source>(&mut self, doc: &Document<S>, list: &[usize], lr: &Resolved, idx: usize) -> R<u64> {
        let Ty::PointerList { offsets, field, anchor, adjust, .. } = &lr.ty else {
            return fail("not a pointer list");
        };
        let (offsets, field, anchor, adjust) = (offsets.clone(), field.clone(), *anchor, adjust.clone());
        let base = match anchor {
            Anchor::File => 0,
            // The nearest enclosing window, which is the page or the table the
            // offsets are counted inside.
            Anchor::Window => (0..list.len())
                .rev()
                .find_map(|k| self.memo.get(&list[..k]).filter(|r| r.declared_size.is_some()))
                .map(|r| r.offset)
                .unwrap_or(0),
            // The list's own start, aligned. `align` is bytes; offsets are bits.
            Anchor::SelfAligned(align) => {
                let a = u64::from(align) * 8;
                if a == 0 { lr.offset } else { lr.offset.div_ceil(a) * a }
            }
        };
        let at = self.eval_expr(
            doc,
            list,
            &Expr::Elem {
                array: offsets,
                index: Box::new(Expr::Lit(idx as i128)),
                field: field.into_iter().collect(),
            },
        )?;
        let adj = self.eval_expr(doc, list, &adjust)?;
        let bits = base as i128 + (at + adj) * 8;
        if bits < lr.offset as i128 || bits >= lr.limit as i128 {
            return fail(format!("offset {at} points outside {}", lr.name));
        }
        Ok(bits as u64)
    }

    /// Every child start of a pointer list, sorted, worked out once and kept.
    /// A child whose offset does not parse is left out rather than taking the
    /// list with it; a child whose bytes are not loaded yet is still an answer
    /// the caller has to wait for.
    fn pointer_starts<S: Source>(&mut self, doc: &Document<S>, list: &[usize], lr: &Resolved) -> R<Vec<u64>> {
        if let Some(starts) = self.lists.get(list).and_then(|l| l.pointer_starts.clone()) {
            return Ok(starts);
        }
        let n = self.child_count(doc, list)?;
        let mut starts = Vec::with_capacity(n as usize);
        for i in 0..n as usize {
            match self.pointer_offset(doc, list, lr, i) {
                Ok(off) => starts.push(off),
                Err(e @ EvalError::Pending(_)) => return Err(e),
                Err(_) => {}
            }
        }
        starts.sort_unstable();
        self.list_mut(list).pointer_starts = Some(starts.clone());
        Ok(starts)
    }

    /// Resolve and size children `0..idx` of `parent`, in order, without recursion.
    fn resolve_upto<S: Source>(&mut self, doc: &Document<S>, parent: &[usize], idx: usize) -> R<()> {
        let mut j = self.list(parent).seq_end;
        let mut p = parent.to_vec();
        while j < idx {
            p.push(j);
            self.resolve(doc, &p)?;
            self.size_of(doc, &p)?;
            p.pop();
            j += 1;
            self.list_mut(parent).seq_end = j;
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
                    match self.template.types.get(&*n) {
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
                    // Only the case this file takes is cloned; the others
                    // stay shared.
                    ty = match cases.iter().find(|(k, _)| *k == v) {
                        Some((_, t)) => t.clone(),
                        None => (*default).clone(),
                    };
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
                // Fixed-stride elements: the whole array is count × stride,
                // with no element resolved. An array of a billion samples is
                // sized by arithmetic, not by a walk.
                Ty::Array { elem, count } if fixed_bits(elem).is_some() => {
                    let n = self.eval_expr(doc, path, count)?;
                    if n < 0 {
                        return fail("negative count");
                    }
                    n as u64 * fixed_bits(elem).expect("checked")
                }
                Ty::Array { .. } | Ty::Repeat { .. } => {
                    let n = self.child_count(doc, path)?;
                    if n == 0 {
                        0
                    } else {
                        let mut last = path.to_vec();
                        last.push(n as usize - 1);
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
                let state = self.list(path);
                if state.repeat_done {
                    return Ok(state.repeat_len as u64);
                }
                let mut p = path.to_vec();
                loop {
                    let (ends, done) = {
                        let l = self.list(path);
                        (l.repeat_len, l.repeat_done)
                    };
                    if done {
                        return Ok(ends as u64);
                    }
                    let start = self.list(path).repeat_end.unwrap_or(r.offset);
                    if start >= r.limit {
                        self.list_mut(path).repeat_done = true;
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
                    let m = self.list_mut(path);
                    m.repeat_len += 1;
                    m.repeat_end = Some(end);
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
            Expr::Elem { array, index, field } => {
                let i = self.eval_expr_at(doc, at, index, here)?;
                if i < 0 {
                    return fail("negative index");
                }
                let Some(mut p) = self.find_field(at, array) else {
                    return fail(format!("unknown field {array}"));
                };
                p.push(i as usize);
                // Walk named fields into the element: `tensors[i].offset`.
                for name in field {
                    self.resolve(doc, &p)?;
                    let Ty::Struct(s) = self.memo[&p].ty.base() else {
                        return fail(format!("{array}[{i}] has no fields to look in"));
                    };
                    let Some(j) = s.fields.iter().position(|f| &f.name == name) else {
                        return fail(format!("{array}[{i}] has no field named {name}"));
                    };
                    p.push(j);
                }
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
            Expr::Sibling(field) => self.sibling_field(doc, at, &field.clone())?,
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

    /// The value at `field` in the nearest earlier element of the enclosing
    /// list that has one. Elements between are passed over: a WAVE file can put
    /// `fact` or `LIST` between `fmt ` and `data`, and the samples are still
    /// the width `fmt ` gave them.
    ///
    /// This reads earlier elements, which in a list long enough to be walked
    /// with its middle dropped means placing them again. Formats that need it
    /// have tens of elements, not millions; it does not belong in a long list.
    fn sibling_field<S: Source>(&mut self, doc: &Document<S>, at: &[usize], field: &[String]) -> R<i128> {
        let mut cur = at.to_vec();
        while let Some(idx) = cur.pop() {
            let listy = matches!(
                self.memo.get(&cur).map(|r| &r.ty),
                Some(Ty::Array { .. } | Ty::Repeat { .. } | Ty::PointerList { .. })
            );
            if !listy {
                continue;
            }
            for earlier in (0..idx).rev() {
                let mut elem = cur.clone();
                elem.push(earlier);
                if let Some(v) = self.field_in(doc, &mut elem, field)? {
                    return Ok(v);
                }
            }
            // Nothing in this list has it, so ask the list this one sits in.
            // A WAVE sample is inside a frame, inside the samples of a chunk,
            // and what declared its width is a chunk two levels out.
        }
        Ok(0)
    }

    /// Follow `field` down from the node at `path`, through whatever the
    /// template resolved it to, and read the number at the end. None when this
    /// node has no such field, which is how a search over siblings passes over
    /// the ones that are something else.
    fn field_in<S: Source>(&mut self, doc: &Document<S>, path: &mut Vec<usize>, field: &[String]) -> R<Option<i128>> {
        for name in field {
            if self.resolve(doc, path).is_err() {
                return Ok(None);
            }
            let Ty::Struct(s) = self.memo[path.as_slice()].ty.base() else { return Ok(None) };
            let Some(j) = s.fields.iter().position(|f| &f.name == name) else { return Ok(None) };
            path.push(j);
        }
        Ok(match self.node(doc, path) {
            Ok(info) => info.value.as_int(),
            // A field that cannot be read yet is not an answer, and must not be
            // taken for the absence of one.
            Err(e @ EvalError::Pending(_)) => return Err(e),
            Err(_) => None,
        })
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

}

fn child_elem(ty: &Ty) -> &Ty {
    match ty {
        Ty::Array { elem, .. } | Ty::Repeat { elem, .. } => elem,
        other => other,
    }
}
