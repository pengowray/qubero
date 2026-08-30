//! Lazy template evaluation over a document.
//!
//! Nodes are addressed by path (child indices from the root). Offsets and sizes
//! are memoised per path and thrown away when the document changes. A read that
//! touches an unloaded chunk yields `EvalError::Pending` rather than a value, so
//! zero-filled bytes can never be mistaken for data.

use rustc_hash::FxHashMap;

use crate::bits::bytes_for;
use crate::decode::{be_int, f8_to_f64, f80_to_f64, fixed_bits, narrow_bf16, narrow_f16, narrow_f32, read_int, read_uint};
use crate::document::Document;
use crate::encode;
use crate::machinery;
use crate::source::{Missing, Source};
use crate::template::{Anchor, Encoding, Expr, StrLen, Template, Ty, Until};
use crate::text::{self, Settled};

mod explain;
mod jsontree;
mod listing;
mod origin;
mod placed;
mod expr;
mod read;
mod size;
mod walk;
#[cfg(test)]
mod tests;

pub use explain::{Explain, FlagBit};
pub use listing::{Span, SpanPart};

/// A bounded walk over variable-size array elements that has enough samples to
/// project the array's eventual extent. This is deliberately a projection,
/// not a node: the exact node remains unavailable until the walk finishes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtentEstimate {
    pub path: Vec<usize>,
    pub measured_items: u64,
    pub total_items: u64,
    pub measured_bits: u64,
    pub estimated_bits: u64,
}
pub use origin::{Origin, Role};

#[derive(Debug, Clone, PartialEq)]
pub enum EvalError {
    Pending(Vec<Missing>),
    /// The work allowed for one go ran out before this could be worked out.
    /// Asking again carries on from where it stopped. `reached_bits` is how far
    /// into the file the reading has got, which is what someone watching it
    /// wants to know.
    Busy { reached_bits: u64 },
    Failed(String),
}

impl EvalError {
    /// The work is not finished rather than not possible: bytes are on their
    /// way, or the time allowed for one go ran out. Either way the answer is to
    /// ask again, and neither means the field is wrong.
    pub fn interrupted(&self) -> bool {
        matches!(self, EvalError::Pending(_) | EvalError::Busy { .. })
    }
}

pub type R<T> = Result<T, EvalError>;

fn fail<T>(msg: impl Into<String>) -> R<T> {
    Err(EvalError::Failed(msg.into()))
}

/// How far down a node may be before reading it is refused, counted in path
/// components. Components, not levels of the format: one level of CBOR is two
/// of these, one level of bencode about six, so this is sixty-odd of the first
/// and twenty of the second, which is past anything a file means by nesting.
///
/// The number is what the stack affords, less a third. Measured against a
/// 1 MiB stack, which is what wasm is given and what a thread on Windows
/// starts with: a debug build runs out at 195 components, a release build with
/// this workspace's settings at about 1400. The debug build is the one that
/// binds, and shrinking what a frame holds is what would raise this.
///
/// This is also the ceiling on `no_ring`'s `DEEPEST`, which it will now never
/// reach: following a pointer adds components too, so a chain of them stops
/// here first. Real nesting stops at three.
const DEEPEST_PATH: usize = 128;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    UInt(u128),
    Int(i128),
    Float(f64),
    Bytes { len: u64, preview: Vec<u8> },
    Str(String),
    /// The bytes a format fixes, and whether the file has them. The bytes
    /// are kept so the value can be read rather than only judged: a
    /// signature is a name as often as it is a number.
    Magic { ok: bool, bytes: Vec<u8> },
    Composite { count: u64 },
    /// A named integer. `name` is None when the file holds a value the enum
    /// does not list. `hex` is how the number should be shown.
    Enum { raw: i128, name: Option<String>, hex: bool },
    /// A run of bytes whose first few have not arrived yet. The field's place
    /// and length are known; only what it holds is still coming.
    Unread { len: u64 },
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
            // A field whose bytes have not arrived is not a number yet.
            Value::Unread { .. } => None,
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
    /// What one child of this list is called, for counting them: `value` for a
    /// run of numbers, whatever the format calls them for a run it has a word
    /// for, and nothing at all when the honest answer is `item`.
    pub unit: Option<String>,
    /// The sibling whose length, count, type or position this field settles,
    /// as an index among the parent's children. This is the fact behind
    /// folding a structure's machinery away: what a field is machinery *for*.
    /// Whether it is folded is the view's to decide, since that depends on
    /// where the two of them end up on screen. See [`crate::machinery`].
    pub consumed_by: Option<usize>,
    /// What the template says about this field regardless of the shapes:
    /// `Some(true)` for machinery, `Some(false)` for payload, `None` when it
    /// has no opinion.
    pub machinery: Option<bool>,
    /// True when this field is only its parent's contents: a name that says
    /// nothing the parent has not already said. A ZIP entry is a signature and
    /// a `body`, and a view that gives `body` a heading of its own has spent a
    /// level of structure on the word "body". See `StructDef::contents`.
    pub contents: bool,
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
    /// An array's declared count. Repeats do not know theirs until they finish,
    /// so only arrays can project a total extent while being walked.
    expected_count: Option<u64>,
    /// The offset of every thousandth element of a long list, so one can be
    /// reached without walking from the start. See `walk.rs`.
    checkpoints: Vec<(usize, u64)>,
    /// For `PointerList` with `to_next`: every child's start, sorted, worked
    /// out once so each child can find the one after it without a walk.
    /// Where each child of a pointer list starts, in order of where rather
    /// than in order of which, with the child it belongs to. Sorted, so the
    /// child covering a bit is a halving rather than a walk through all of them.
    pointer_starts: Option<Vec<(u64, usize)>>,
    /// Children `0..seq_end` are resolved and sized, so child `seq_end` can
    /// be placed without walking back. Keeps sibling resolution iterative.
    seq_end: usize,
}

/// What to call a node: the name of the field it is, or where it sits in the
/// list it belongs to. An index is kept as a number and spelled `[7]` only
/// when something asks for it, because placing a million elements would
/// otherwise spell a million names that nobody reads.
#[derive(Debug, Clone)]
enum Name {
    Field(std::sync::Arc<str>),
    Index(usize),
}

impl Name {
    fn text(&self) -> String {
        match self {
            Name::Field(s) => s.to_string(),
            Name::Index(i) => format!("[{i}]"),
        }
    }
}

#[derive(Debug, Clone)]
struct Resolved {
    name: Name,
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
    /// The text of each `Ty::Json` field, parsed, kept beside the memo. Read
    /// once however many values are asked for, and thrown away with the memo
    /// entry it belongs to.
    json: FxHashMap<Vec<usize>, std::sync::Arc<crate::json::Val>>,
    /// What each guarded walk has added to the memo, so it can drop the nodes
    /// it has moved past. One entry per walk, since a list can hold a list.
    journals: Vec<walk::WalkJournal>,
    /// Elements left before this go has to hand back, and how many each go is
    /// allowed. None works to the end, which is what a caller with nothing to
    /// draw meanwhile wants.
    left: Option<u64>,
    slice: Option<u64>,
    /// How far into the file the reading has got, at its furthest.
    reached_bits: u64,
    /// Bytes an answer was given without: previews that have not arrived. The
    /// caller fetches them and asks again, and meanwhile has its rows.
    wanted: Vec<Missing>,
    /// Every stretch of the file a field placed somewhere other than where it
    /// was declared, so a bit outside what the root covers can still be named.
    /// See [`placed`].
    placements: Vec<placed::Placement>,
    /// The stretches already in that index, so the same one reached from a
    /// hundred thousand places is walked into once.
    placed_ranges: rustc_hash::FxHashSet<(u64, u64)>,
    /// Whether that index is everything there is, or as far as the walk got.
    placements_done: bool,
    /// The walk's own stack, so it can stop after a bounded number of nodes
    /// and carry on from where it was when the next question comes.
    frontier: Vec<placed::Frame>,
    /// How many nodes that walk has opened, over all its goes.
    placed_opened: usize,
}

impl Evaluator {
    pub fn new(template: Template) -> Self {
        Self {
            template,
            memo: FxHashMap::default(),
            lists: FxHashMap::default(),
            json: FxHashMap::default(),
            journals: Vec::new(),
            left: None,
            slice: None,
            reached_bits: 0,
            wanted: Vec::new(),
            placements: Vec::new(),
            placed_ranges: rustc_hash::FxHashSet::default(),
            placements_done: false,
            frontier: Vec::new(),
            placed_opened: 0,
        }
    }

    pub fn template(&self) -> &Template {
        &self.template
    }

    /// Work in goes of `elements` at a time, handing back `EvalError::Busy` in
    /// between so the caller can draw what it has and say how far it has got.
    /// None, the default, works until the answer is ready however long that
    /// takes.
    pub fn set_slice(&mut self, elements: Option<u64>) {
        self.slice = elements;
        self.left = elements;
    }

    /// Start another go. What was worked out already is kept; only the
    /// allowance is refilled.
    pub fn begin_slice(&mut self) {
        self.left = self.slice;
        self.wanted.clear();
    }

    /// Bytes wanted for previews that were answered without them, since the
    /// last `begin_slice`. Fetching these and asking again fills them in.
    pub fn wanted(&self) -> Vec<Missing> {
        let mut out = self.wanted.clone();
        out.sort_by_key(|m| m.chunk);
        out.dedup();
        out
    }

    pub(super) fn want(&mut self, missing: Vec<Missing>) {
        self.wanted.extend(missing);
    }

    /// How far into the file the reading has got, at its furthest.
    pub fn reached_bits(&self) -> u64 {
        self.reached_bits
    }

    /// The most advanced unfinished variable-size array walk. Its average
    /// element extent is projected over the declared count; callers mark the
    /// result approximate until the ordinary node succeeds.
    pub fn extent_estimate(&self) -> Option<ExtentEstimate> {
        self.lists
            .iter()
            .filter_map(|(path, state)| {
                let total = state.expected_count?;
                let (measured, at) = state.walk_at?;
                if measured == 0 || measured as u64 >= total {
                    return None;
                }
                let start = self.memo.get(path)?.offset;
                let measured_bits = at.saturating_sub(start);
                if measured_bits == 0 {
                    return None;
                }
                let estimated_bits = measured_bits
                    .saturating_mul(total)
                    .checked_div(measured as u64)?;
                Some(ExtentEstimate {
                    path: path.clone(),
                    measured_items: measured as u64,
                    total_items: total,
                    measured_bits,
                    estimated_bits,
                })
            })
            .max_by_key(|estimate| (estimate.measured_items, estimate.measured_bits))
    }

    /// Charge one element against this go's allowance, and note how far the
    /// reading has reached.
    pub(super) fn spend(&mut self, at_bits: u64) -> R<()> {
        self.reached_bits = self.reached_bits.max(at_bits);
        let Some(left) = self.left.as_mut() else { return Ok(()) };
        if *left == 0 {
            return Err(EvalError::Busy { reached_bits: self.reached_bits });
        }
        *left -= 1;
        Ok(())
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
        // Every placement is where a resolved node turned out to be, and this
        // is about to drop some of those nodes. Coarse, like the memo's own
        // invalidation, and cheap to build again.
        self.placements.clear();
        self.placed_ranges.clear();
        self.placements_done = false;
        self.frontier.clear();
        self.placed_opened = 0;
        // A node with no size worked out yet is dropped: nothing says where it
        // ends, so nothing says it ended before the edit.
        self.memo.retain(|_, r| r.size.is_some_and(|size| r.offset + size <= bit));
        // The parsed text of a JSON field goes when the field itself does.
        self.json.retain(|path, _| self.memo.contains_key(path));
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
        self.placements.clear();
        self.placed_ranges.clear();
        self.placements_done = false;
        self.frontier.clear();
        self.placed_opened = 0;
        self.lists.clear();
        self.json.clear();
        self.journals.clear();
        self.left = self.slice;
        self.reached_bits = 0;
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

    /// The child of `path` that a structure calls `name`, if it has one.
    ///
    /// A reader that walks a file in the format's own terms rather than the
    /// template's needs this: the field indices of a template are its own
    /// business and change when a field is added, but a format's names do not.
    pub fn child_named<S: Source>(&mut self, doc: &Document<S>, path: &[usize], name: &str) -> R<Option<Vec<usize>>> {
        Ok(self.child_index(doc, path, name)?.map(|i| {
            let mut p = path.to_vec();
            p.push(i);
            p
        }))
    }

    pub fn node<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<NodeInfo> {
        self.resolve(doc, path)?;
        let size = self.size_of(doc, path)?;
        let r = self.memo.get(path).expect("resolved").clone();
        let (value, child_count, composite) = match &r.ty {
            Ty::Struct(s) => (Value::Composite { count: s.fields.len() as u64 }, s.fields.len() as u64, true),
            Ty::Array { .. } | Ty::Repeat { .. } | Ty::PointerList { .. } | Ty::At { .. } => {
                let n = self.child_count(doc, path)?;
                (Value::Composite { count: n }, n, true)
            }
            Ty::Json(shape) if shape.composite() => {
                let n = self.child_count(doc, path)?;
                (Value::Composite { count: n }, n, true)
            }
            _ => (self.primitive_value(doc, path, &r, &r.ty, size)?, 0, false),
        };
        let reading = self.reading(doc, &r, size)?;
        let (consumed_by, machinery, contents) = self.in_parent(path);
        Ok(NodeInfo {
            path: path.to_vec(),
            editable: !composite && encode::editable(&r.ty, size) && self.padding_is_clean(doc, &r, size)? && !reading.1,
            value_offset_bits: reading.0 .0,
            value_bytes: reading.0 .1,
            read_as: reading.2,
            name: self.label(doc, path, &r)?,
            type_name: r.ty.display_name(),
            unit: self.unit_of(path, &r.ty).map(str::to_string),
            offset_bits: r.offset,
            size_bits: size,
            value,
            child_count,
            composite,
            consumed_by,
            machinery,
            contents,
        })
    }

    /// What the field at `path` is machinery for, what its structure says about
    /// it, and whether it is only that structure's contents. All three are
    /// properties of the parent structure's declaration, so a child of a list
    /// has none of them: the elements of a list are all the same shape and none
    /// of them places another.
    ///
    /// Worked out afresh for each child rather than once for the structure.
    /// Listing a structure's children makes that quadratic, in a walk of a
    /// handful of type enums over the few dozen fields a structure has; a run
    /// long enough to care about is a list, and lists stop at the first line
    /// here.
    fn in_parent(&self, path: &[usize]) -> (Option<usize>, Option<bool>, bool) {
        let Some((&last, parent)) = path.split_last() else { return (None, None, false) };
        let Some(Ty::Struct(s)) = self.memo.get(parent).map(|r| &r.ty) else { return (None, None, false) };
        let name = s.fields.get(last).map(|f| &f.name);
        let contents = match (&s.contents, name) {
            (Some(c), Some(n)) => **n == **c,
            _ => false,
        };
        (machinery::consumers(s).get(last).copied().flatten(), machinery::hint(s, last), contents)
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
        let mut missing: Vec<Missing> = Vec::new();
        let mut p = path.to_vec();
        for i in from..to.min(n) {
            p.push(i as usize);
            let got = self.node(doc, &p);
            p.pop();
            match got {
                Ok(info) => out.push(info),
                // Children placed by offsets sit apart from one another, so the
                // bytes one of them needs say nothing about the next. Asking
                // for all of them together is one wait; stopping at the first
                // is one wait per child, and a page of two hundred tensors
                // would trickle in over two hundred goes.
                Err(EvalError::Pending(m)) => missing.extend(m),
                Err(e) => return Err(e),
            }
        }
        if !missing.is_empty() {
            missing.sort_by_key(|m| m.chunk);
            missing.dedup();
            return Err(EvalError::Pending(missing));
        }
        Ok(out)
    }

    // ----- resolution -----

    /// What one child of this list is called. A run of numbers holds values; a
    /// run of anything the format has a word for holds those; everything else
    /// holds items, which is what saying nothing here means. A list of a named
    /// type is looked up, since that is how a format that reaches its own types
    /// by name writes one.
    fn unit_of<'a>(&'a self, path: &[usize], ty: &'a Ty) -> Option<&'a str> {
        // JSON counts what it holds: an object holds entries, an array holds
        // values. The whole text holds whichever of the two it turned out to
        // be, which is known once it has been read.
        if let Ty::Json(shape) = ty {
            let shape = match shape {
                crate::json::Shape::Doc => self.json.get(path)?.kind.shape(),
                other => *other,
            };
            return match shape {
                crate::json::Shape::Object => Some("entry"),
                crate::json::Shape::Array => Some("value"),
                _ => None,
            };
        }
        let mut elem = match ty {
            Ty::Array { elem, .. } | Ty::Repeat { elem, .. } | Ty::PointerList { elem, .. } => elem.base(),
            _ => return None,
        };
        for _ in 0..8 {
            let Ty::Named(n) = elem else { break };
            elem = self.template.types.get(&**n)?.base();
        }
        if let Ty::Struct(s) = elem {
            if let Some(unit) = &s.unit {
                return Some(unit);
            }
        }
        listing::plain(elem).then_some("value")
    }

    fn resolve<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<()> {
        if self.memo.contains_key(path) {
            return Ok(());
        }
        // Placing a node opens the line of ancestors above it, one call deep
        // for each, and measuring a list opens its elements the same way. So
        // the length of the path is how deep the stack goes, and a format that
        // can hold itself has no length it stops at: bencode nested 908 deep
        // is a real file someone has written down. Past this the answer is an
        // error rather than the stack running out under it.
        if path.len() > DEEPEST_PATH {
            return fail(format!("nested more than {DEEPEST_PATH} fields deep"));
        }
        if path.is_empty() {
            let limit = doc.len_bits();
            let root = self.template.root.clone();
            let r = self.effective(doc, &[], Name::Field("file".into()), root, 0, limit)?;
            self.remember(&[], r);
            return Ok(());
        }
        let (parent, idx) = (&path[..path.len() - 1], path[path.len() - 1]);
        self.resolve(doc, parent)?;
        let pr = self.memo.get(parent).expect("parent resolved").clone();
        // A value inside JSON is placed where its text is, which the parse
        // already knows. Nothing below applies to it.
        if matches!(pr.ty, Ty::Json(_)) {
            return self.resolve_json_child(doc, path);
        }
        let (name, ty) = match &pr.ty {
            Ty::Struct(s) => match s.fields.get(idx) {
                Some(f) => (Name::Field(f.name.clone()), f.ty.clone()),
                None => return fail("no such field"),
            },
            Ty::Array { elem, .. } | Ty::Repeat { elem, .. } | Ty::PointerList { elem, .. } => {
                (Name::Index(idx), (**elem).clone())
            }
            // The one thing it points at keeps the field's own name: a row
            // saying `directory` twice says nothing the once did not.
            Ty::At { inner, .. } => (pr.name.clone(), (**inner).clone()),
            _ => return fail("not a composite"),
        };
        // A field that reads its contents from somewhere else in the file is
        // not bounded by the structure it was declared in: an object header
        // message is sixteen bytes long and the heap it names is half a
        // kilobyte further on. What such a child is bounded by is the file.
        let mut escapes = None;
        // Offset: read from the pointer array, or after the previous sibling,
        // or at the parent's start.
        let offset = if matches!(pr.ty, Ty::PointerList { .. }) {
            match self.pointer_offset(doc, parent, &pr, idx)? {
                Some(at) => at,
                // An entry that points at nothing, in a list that allows for
                // one. It keeps its place among the children and covers no
                // bytes: a safetensors header holds the file's own metadata
                // among the tensors, and no weights belong to it.
                None => {
                    let r = Resolved {
                        name,
                        ty: Ty::Bytes(Expr::Lit(0)),
                        offset: pr.offset,
                        limit: pr.offset,
                        declared_size: Some(0),
                        size: Some(0),
                        computed: None,
                    };
                    self.remember(path, r);
                    return Ok(());
                }
            }
        } else if let Ty::At { anchor, at, inner } = &pr.ty {
            // Bytes from the anchor, which is how a header names the place it
            // keeps a table: from the start of the file, or from the start of
            // the copy of the format this one is written inside.
            let (anchor, at, what) = (*anchor, at.clone(), self.settled_name(inner));
            let n = self.eval_expr(doc, parent, &at)?;
            if n < 0 {
                return fail("negative offset");
            }
            let to = self.anchor_base(parent, pr.offset, anchor) + n as u64 * 8;
            self.no_ring(parent, to, &what)?;
            if anchor == Anchor::File {
                escapes = Some(doc.len_bits());
            }
            to
        } else if idx == 0 {
            pr.offset
        } else if let Some(stride) = self.stride(doc, parent, &pr.ty)? {
            pr.offset + idx as u64 * stride
        } else {
            // Place after the previous sibling, walking the elements in
            // between. A long list drops what the walk moves past, so this
            // stays bounded in memory; see `walk.rs`.
            // Note: asking for an element past a Repeat's terminating condition
            // is not prevented here; `children()` clamps, direct callers must too.
            self.walk_to(doc, parent, idx)?
        };
        let mut limit = escapes.unwrap_or(pr.limit);
        if offset > limit {
            return fail("runs past the end of its container");
        }
        // A pointer-list child with no size of its own runs to the next child
        // above it: its limit is that child's start.
        if let Ty::PointerList { to_next: true, .. } = &pr.ty {
            let starts = self.pointer_starts(doc, parent, &pr)?;
            if let Some((next, _)) = starts.get(starts.partition_point(|(s, _)| *s <= offset)) {
                limit = limit.min(*next);
            }
        }
        let r = self.effective(doc, path, name, ty, offset, limit)?;
        self.remember(path, r);
        Ok(())
    }

    /// Refuse an offset that points back at something already open above it.
    ///
    /// Every other way of placing a child moves forward, so a type that refers
    /// to itself is bounded by whatever contains it and must end. An `At` is
    /// the one that does not: it names a place, and a file is free to name a
    /// place that is already being read. A directory whose entry points at
    /// that directory is a ring, and following one is not slow but endless,
    /// since the cursor asking what covers a byte would go round it forever.
    ///
    /// A ring has to come back to something it has already opened, and what
    /// makes two nodes the same node is being the same type in the same
    /// place. So the line of ancestors is asked whether any of them is that,
    /// and one that is means the pointer closes a ring. Only ancestors: two
    /// entries pointing at the same string are not a ring, they are two
    /// entries pointing at the same string, and that is allowed and common.
    ///
    /// The count of pointers above is kept as well, for the file that is not a
    /// ring but is trying to be difficult: a chain of a million distinct
    /// offsets ends, but not soon enough to be worth waiting for. The limit is
    /// far past any real nesting, which in practice is a JPEG holding a TIFF
    /// holding a sub-directory and stops at three.
    /// What a type is called once a name has been looked up. A ring is
    /// recognised by what is at the far end of the pointer, and the pointer
    /// says `tiff.Ifd` while the node already open there remembers the
    /// structure that name stands for. Comparing the two as written would
    /// have the guard quietly never fire on the one shape that needs it.
    fn settled_name(&self, ty: &Ty) -> String {
        let mut ty = ty;
        for _ in 0..64 {
            let Ty::Named(n) = ty else { break };
            match self.template.types.get(&**n) {
                Some(t) => ty = t,
                None => break,
            }
        }
        ty.display_name()
    }

    fn no_ring(&self, parent: &[usize], to: u64, what: &str) -> R<()> {
        const DEEPEST: usize = 1024;
        let mut jumps = 0;
        for k in (0..=parent.len()).rev() {
            let Some(r) = self.memo.get(&parent[..k]) else { continue };
            if matches!(r.ty, Ty::At { .. }) {
                jumps += 1;
            }
            if r.offset == to && r.ty.display_name() == what {
                return fail("points back at something already being read");
            }
        }
        if jumps >= DEEPEST {
            return fail(format!("pointers nested more than {DEEPEST} deep"));
        }
        Ok(())
    }

    /// What an offset is counted from, in bits. Shared by the two types that
    /// place something away from where it is declared: a pointer list and an
    /// `At`. `own` is the start of the field doing the pointing, which only
    /// the aligned anchor uses.
    fn anchor_base(&self, path: &[usize], own: u64, anchor: Anchor) -> u64 {
        match anchor {
            Anchor::File => 0,
            // The nearest window around it, which is the page or the table or
            // the embedded copy the offsets are counted inside. The field's
            // own path is not searched: a list that is itself a window counts
            // from the one outside it, not from itself.
            Anchor::Window => (0..path.len())
                .rev()
                .find_map(|k| self.memo.get(&path[..k]).filter(|r| r.declared_size.is_some()))
                .map(|r| r.offset)
                .unwrap_or(0),
            // Its own start, aligned. `align` is bytes; offsets are bits.
            Anchor::SelfAligned(align) => {
                let a = u64::from(align) * 8;
                if a == 0 { own } else { own.div_ceil(a) * a }
            }
        }
    }

    /// Where child `idx` of a pointer list starts. The offsets are bytes from
    /// the anchor, so a child can sit anywhere in the list's stretch, in any
    /// order. One that points outside it is an error for that child alone.
    fn pointer_offset<S: Source>(&mut self, doc: &Document<S>, list: &[usize], lr: &Resolved, idx: usize) -> R<Option<u64>> {
        let Ty::PointerList { offsets, field, anchor, adjust, skip_missing, skip_zero, .. } = &lr.ty else {
            return fail("not a pointer list");
        };
        let (offsets, field, anchor, adjust) = (offsets.clone(), field.clone(), *anchor, adjust.clone());
        let (skip_missing, skip_zero) = (*skip_missing, *skip_zero);
        let base = self.anchor_base(list, lr.offset, anchor);
        let e = Expr::Elem { array: offsets, index: Box::new(Expr::Lit(idx as i128)), field };
        let at = match self.eval_expr(doc, list, &e) {
            Ok(at) => at,
            // Nothing to read there. In a list that allows for it that is an
            // entry pointing at nothing rather than a broken file.
            Err(err) if skip_missing && !err.interrupted() => return Ok(None),
            Err(err) => return Err(err),
        };
        // Zero before anything is added to it: what the table holds is what
        // says the entry points at nothing, not where the arithmetic lands.
        if skip_zero && at == 0 {
            return Ok(None);
        }
        let adj = self.eval_expr(doc, list, &adjust)?;
        let bits = base as i128 + (at + adj) * 8;
        // The end of the list is a place a child may start: an entry holding
        // nothing sits there, and a safetensors file writes one. A child that
        // starts there and reads anything fails when it reads.
        if bits < lr.offset as i128 || bits > lr.limit as i128 {
            return fail(format!("offset {at} points outside {}", lr.name.text()));
        }
        Ok(Some(bits as u64))
    }

    /// Every child start of a pointer list, sorted, worked out once and kept.
    /// A child whose offset does not parse is left out rather than taking the
    /// list with it; a child whose bytes are not loaded yet is still an answer
    /// the caller has to wait for.
    fn pointer_starts<S: Source>(&mut self, doc: &Document<S>, list: &[usize], lr: &Resolved) -> R<Vec<(u64, usize)>> {
        if let Some(starts) = self.lists.get(list).and_then(|l| l.pointer_starts.clone()) {
            return Ok(starts);
        }
        let n = self.child_count(doc, list)?;
        let mut starts = Vec::with_capacity(n as usize);
        for i in 0..n as usize {
            match self.pointer_offset(doc, list, lr, i) {
                Ok(Some(off)) => starts.push((off, i)),
                Ok(None) => {}
                Err(e) if e.interrupted() => return Err(e),
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
        name: Name,
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
                Ty::Match { on, cases, default } => {
                    let v = self.text_at(doc, path, &on, Some((offset, limit)))?;
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

    /// Raw bytes of a named field directly inside the struct at `path`.
    fn child_raw_bytes<S: Source>(&mut self, doc: &Document<S>, path: &[usize], field: &str) -> R<Vec<u8>> {
        let idx = match &self.memo[path].ty {
            Ty::Struct(s) => s.fields.iter().position(|f| *f.name == *field),
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
}
