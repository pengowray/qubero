//! Lazy template evaluation over a document.
//!
//! Nodes are addressed by path (child indices from the root). Offsets and sizes
//! are memoised per path and thrown away when the document changes. A read that
//! touches an unloaded chunk yields `EvalError::Pending` rather than a value, so
//! zero-filled bytes can never be mistaken for data.

use crate::bits::bytes_for;
use crate::codec::Refusal;
use crate::decode::{be_int, f8_to_f64, f80_to_f64, fixed_bits, lsb_offset, lsb_packed, narrow_bf16, narrow_f16, narrow_f32, packed_int, read_int, read_sign_magnitude, read_uint};
use crate::document::Document;
use crate::encode;
use crate::machinery;
use crate::source::{Missing, Source};
use crate::template::{Anchor, Encoding, Expr, StrLen, Tag, TaggedRef, Template, TracedPart, Ty, Until};
use crate::text::{self, Settled};

mod explain;
mod go;
mod jsontree;
mod listing;
mod memo;
mod origin;
mod placed;
mod expr;
mod read;
mod relate;
mod size;
mod space;
mod traced;
mod walk;
#[cfg(test)]
mod tests;

pub use explain::{Explain, FlagBit};
pub use space::{Space, SpaceId};
pub use listing::{magic_reading, Span, SpanPart};
pub use relate::write_expr;

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
pub use relate::Relation;

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

/// Whether a stream's declared contents say nothing about what they are.
///
/// A template that says a run unpacks into bytes, or into a wrapper holding
/// one field of bytes or of text, has no opinion worth keeping: the bytes
/// themselves know better, and a gzip of a tar should open as a tar. A
/// template that says anything else is believed.
fn says_only_bytes(ty: &Ty) -> bool {
    match ty {
        Ty::Bytes(_) | Ty::Str { .. } => true,
        Ty::Sized { inner, .. } | Ty::SizedBits { inner, .. } => says_only_bytes(inner),
        Ty::Struct(s) => s.fields.len() == 1 && says_only_bytes(&s.fields[0].ty),
        _ => false,
    }
}

/// How far down a node may be before reading it is refused, counted in path
/// components. Components, not levels of the format: one level of CBOR is two
/// of these, one level of bencode about six, so this is thirty of the first
/// and ten of the second: deeper than a file means anything by, and nowhere
/// near what one can be made to say.
///
/// Measured rather than picked, in a debug build, whose frames are several
/// times a release build's. A megabyte of stack, which is what wasm is given
/// and what a thread on Windows starts with, carries about 280 components of
/// a run that stops on what it reads (how bencode nests) and about 390 of a
/// list of lists. So this is under half of the smaller of those, on the
/// smallest stack any of it runs on, and there is several times the room in
/// the build that ships.
///
/// Per component, not per level: the two shapes are much further apart per
/// level than they are here, because a level of the first is four or five
/// components and a level of the second is two.
///
/// This is also the ceiling on `no_ring`'s `DEEPEST`, which it will now never
/// reach: following a pointer adds components too, so a chain of them stops
/// here first. Real nesting stops at three.
///
/// `cargo run --example stack_probe -- <levels> <array|repeat> <MiB>` is where
/// these numbers come from, and is how to take them again after a change.
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
    /// signature is a name as often as it is a number. `expected` is what the
    /// template asked for, carried along so that a file which does not have
    /// it can be told what it was, which is the whole of what a reader wants
    /// from a signature that is wrong.
    Magic { ok: bool, bytes: Vec<u8>, expected: Vec<u8> },
    Composite { count: u64 },
    /// A named integer. `name` is None when the file holds a value the enum
    /// does not list. `hex` is how the number should be shown.
    Enum { raw: i128, name: Option<String>, hex: bool },
    /// A run of bytes whose first few have not arrived yet. The field's place
    /// and length are known; only what it holds is still coming.
    Unread { len: u64 },
    /// A slot holding the value its format writes to mean nobody filled it in.
    /// The number is kept rather than thrown away: it is what the bytes say,
    /// it is what an expression reading this field gets, and a reader who
    /// wants to see -12345 can still be shown it. See [`crate::template::Ty::Nullable`].
    Unset(Box<Value>),
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
            // An unset slot is still the number the file holds. A count that
            // nobody filled in is -12345, and the clamp in front of it is
            // there to deal with exactly that.
            Value::Unset(inner) => inner.as_int(),
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
    /// Which address space the offsets above are counted in. 0 is the file,
    /// which is what all but a decoded stream's contents are. See
    /// [`crate::template::Ty::Decoded`].
    pub space: u32,
    /// For a `Decoded` field whose stream would not open: which of the three
    /// ways it would not, as [`crate::codec::Refusal::as_str`] words it. None
    /// for every other field, and for a stream that opened.
    pub refused: Option<String>,
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
    /// For `Chain`: where each element found so far starts, in the order the
    /// pointers were followed, and whether the walk reached its end. A chain
    /// cannot say how long it is without being walked, and element `n` is
    /// found by reading element `n - 1`, so the walk is kept here and carried
    /// on rather than started again per element.
    chain_starts: Vec<u64>,
    chain_done: bool,
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
    /// Where the count of bits laid down had reached when this field was
    /// placed, which is `offset` for every field but an LSB-first one: that
    /// one sits at the other end of its byte from where the count reached, so
    /// the next field after it goes at `cursor + size` and not at
    /// `offset + size`. See [`crate::decode::lsb_offset`].
    cursor: u64,
    /// Exclusive bit limit this node may not read past.
    limit: u64,
    /// Size fixed by an enclosing `Sized`, if any.
    declared_size: Option<u64>,
    size: Option<u64>,
    /// A computed field's value, once worked out. Element `n` of a list asks
    /// element `n - 1` for its value, so without this a track of ten thousand
    /// events is ten thousand deep rather than one.
    computed: Option<i128>,
    /// Which address space `offset` and `limit` are bits of. 0 is the file.
    space: u32,
}

/// Where a child sits before its type is unwrapped: what it is called, what
/// the template says it is, and the stretch of the file it may read.
struct Place {
    name: Name,
    ty: Ty,
    offset: u64,
    limit: u64,
    space: u32,
}

pub struct Evaluator {
    template: Template,
    memo: memo::Memo,
    /// What each guarded walk has added to the memo, so it can drop the nodes
    /// it has moved past. One entry per walk, since a list can hold a list.
    journals: Vec<walk::WalkJournal>,
    /// What this go of reading may spend and has spent, which is the one
    /// thing here that does not outlive the answer being asked for.
    go: go::Go,
    /// Every stretch of the file a field placed somewhere other than where it
    /// was declared, so a bit outside what the root covers can still be named,
    /// and how far the walk that finds them has got. See [`placed`].
    placed: placed::Index,
    /// The decoded streams this reading has opened, and what each one came to.
    spaces: space::Spaces,
    /// The streams opened as documents of their own, which is what a tab is.
    /// Numbered across every level of nesting, so a stream inside a stream is
    /// a space beside its parent rather than inside it: the interface names
    /// one number and this says which one it means. A slot is taken out while
    /// something is being asked of it, which is why they are options.
    open: Vec<Option<Box<space::Space>>>,
}

impl Evaluator {
    pub fn new(template: Template) -> Self {
        Self {
            template,
            memo: memo::Memo::default(),
            journals: Vec::new(),
            go: go::Go::default(),
            placed: placed::Index::default(),
            spaces: space::Spaces::default(),
            open: Vec::new(),
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
        self.go.set_slice(elements);
    }

    /// Start another go. What was worked out already is kept; only the
    /// allowance is refilled.
    pub fn begin_slice(&mut self) {
        self.go.begin();
    }

    /// Bytes wanted for previews that were answered without them, since the
    /// last `begin_slice`. Fetching these and asking again fills them in.
    pub fn wanted(&self) -> Vec<Missing> {
        self.go.wanted()
    }

    pub(super) fn want(&mut self, missing: Vec<Missing>) {
        self.go.want(missing);
    }

    /// How far into the file the reading has got, at its furthest.
    pub fn reached_bits(&self) -> u64 {
        self.go.reached_bits()
    }

    /// The most advanced unfinished variable-size array walk. Its average
    /// element extent is projected over the declared count; callers mark the
    /// result approximate until the ordinary node succeeds.
    pub fn extent_estimate(&self) -> Option<ExtentEstimate> {
        self.memo
            .lists()
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
        self.go.spend(at_bits)
    }

    /// Read something with one more read open, and close it again afterwards.
    ///
    /// The pair has to be kept exactly: a read left open would have the next
    /// one think the stack was further down than it is. So this is the only
    /// place that keeps it, and whatever `f` does or answers, the read is
    /// closed on the way out.
    pub(super) fn deeper<T>(&mut self, depth: usize, f: impl FnOnce(&mut Self) -> R<T>) -> R<T> {
        self.go.enter(depth)?;
        let out = f(self);
        self.go.leave();
        out
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
        self.placed.forget();
        // Only when there is something to drop: this is the path that exists
        // to keep an edit to a large file cheap, and most files hold no stream
        // at all.
        if self.spaces.any() {
            self.spaces.forget();
            self.memo.forget_decoded();
            // A space is a reading of bytes worked out from the file, so an
            // edit anywhere in the file drops every one of them. A tab over a
            // stream that is gone opens it again, which is one inflate.
            self.open.clear();
        }
        self.memo.forget_after(bit);
    }

    /// Drop every cached offset/size. Call after any document change that is
    /// not an overwrite, and whenever the template changes.
    pub fn invalidate(&mut self) {
        self.memo.forget();
        self.placed.forget();
        self.spaces.forget();
        self.open.clear();
        self.journals.clear();
        self.go.restart();
    }

    /// What the list at `path` has learned about itself. A node that is not
    /// a list, or one nothing has been learned about yet, has learned
    /// nothing, which is what the default says.
    ///
    /// Lent rather than handed over. The walk asks this once an element, and
    /// a list it has walked a million elements into holds a thousand
    /// checkpoints: a copy an element is the crawl this type was split out of
    /// `Resolved` to avoid.
    fn list(&self, path: &[usize]) -> &ListState {
        self.memo.list(path)
    }

    fn list_mut(&mut self, path: &[usize]) -> &mut ListState {
        self.memo.list_mut(path)
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
            Ty::Array { .. } | Ty::Repeat { .. } | Ty::PointerList { .. } | Ty::Chain { .. } | Ty::At { .. } => {
                let n = self.child_count(doc, path)?;
                (Value::Composite { count: n }, n, true)
            }
            Ty::Json(shape) if shape.composite() => {
                let n = self.child_count(doc, path)?;
                (Value::Composite { count: n }, n, true)
            }
            // A stream holds one thing when it opens and nothing when it does
            // not, so asking about the node opens it. That is a read of the
            // whole run, done once and kept: what stops it from being a read
            // per row is that `locate` stops at the run, so only a stream
            // something is actually drawing is ever unpacked, and the row has
            // to say whether it opened.
            Ty::Decoded { .. } | Ty::Traced { .. } => {
                let n = self.child_count(doc, path)?;
                (Value::Composite { count: n }, n, true)
            }
            _ => (self.primitive_value(doc, path, &r, &r.ty, size)?, 0, false),
        };
        let reading = self.reading(doc, &r, size)?;
        let (consumed_by, machinery, contents) = self.in_parent(path);
        Ok(NodeInfo {
            path: path.to_vec(),
            space: r.space,
            refused: match self.spaces.get(path) {
                Some(space::Opened::Refused(why)) => Some(why.as_str().to_string()),
                _ => None,
            },
            // Nothing inside a decoded stream is written back: there is no
            // mapping from a decoded byte to a byte of the file, so a change
            // made there has nowhere to go.
            editable: r.space == 0
                && !composite && encode::editable(&r.ty, size) && self.padding_is_clean(doc, &r, size)? && !reading.1,
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

    /// The first `limit` bytes of the field at `path`, read where the field
    /// actually is.
    ///
    /// A caller with the node in hand knows its offset and could read the
    /// document itself, and that is exactly the mistake this exists to stop: a
    /// field inside a decoded stream is at an offset of that stream, and the
    /// file at the same offset is other bytes entirely. `true` says the field
    /// runs on past what came back.
    pub fn field_bytes<S: Source>(&mut self, doc: &Document<S>, path: &[usize], limit: u64) -> R<(Vec<u8>, bool)> {
        self.resolve(doc, path)?;
        let size = self.size_of(doc, path)?;
        let r = self.memo[path].clone();
        // The value rather than the whole field, so padding and a terminator
        // are left out the way the node's own reading leaves them out.
        let (at, len) = match self.str_span(doc, &r, size)? {
            Some(span) => (r.offset + span.start * 8, span.len),
            None => (r.offset, size / 8),
        };
        let want = len.min(limit);
        Ok((self.read(doc, &r, at, want * 8)?, len > want))
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
        // A decoded byte is a function of every compressed byte before it, so
        // there is no run of the file this text could be written to. `editable`
        // already says so; this is the same answer where it cannot be ignored,
        // since the offset below would otherwise be a bit of the stream used as
        // a bit of the file.
        if r.space != 0 {
            return fail("Bytes read out of a compressed stream can't be edited: they aren't in the file.");
        }
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
                crate::json::Shape::Doc => self.memo.json(path)?.kind.shape(),
                other => *other,
            };
            return match shape {
                crate::json::Shape::Object => Some("entry"),
                crate::json::Shape::Array => Some("value"),
                _ => None,
            };
        }
        // What a trace holds at each level: blocks, and then symbols. An LZ4
        // block has one run of sequences rather than blocks, and counting
        // those as blocks would say something the format does not.
        if let Ty::Traced { part } = ty {
            return match part {
                TracedPart::Blocks => Some(traced::blocks_unit(self.trace_for(path).and_then(|(_, t)| t.blocks().first().map(|b| b.kind)))),
                TracedPart::Block(_) => None,
                TracedPart::Symbols(_) => Some("symbol"),
            };
        }
        let mut elem = match ty {
            Ty::Array { elem, .. } | Ty::Repeat { elem, .. } | Ty::PointerList { elem, .. } | Ty::Chain { elem, .. } => elem.base(),
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
            let r = self.effective(doc, &[], Name::Field("file".into()), root, 0, limit, 0)?;
            self.remember(&[], r);
            return Ok(());
        }
        let (parent, idx) = (&path[..path.len() - 1], path[path.len() - 1]);
        self.resolve(doc, parent)?;
        // Where the child goes is worked out in a call of its own, so that
        // what it took to work out is off the stack before the child is read.
        // Reading the child is what goes deeper, and a file that nests pays
        // for every frame still open above it.
        let Some(place) = self.place_child(doc, path, parent, idx)? else { return Ok(()) };
        let r = self.effective(doc, path, place.name, place.ty, place.offset, place.limit, place.space)?;
        self.remember(path, r);
        Ok(())
    }

    /// Where child `idx` of the node at `parent` goes: what it is called, what
    /// the template says it is, and the stretch of the file it may read.
    ///
    /// `None` when the child has been settled here and there is nothing left
    /// to read: a value inside JSON, whose place the parse already knows, or
    /// an entry in a pointer list that points at nothing.
    fn place_child<S: Source>(&mut self, doc: &Document<S>, path: &[usize], parent: &[usize], idx: usize) -> R<Option<Place>> {
        let pr = self.memo.get(parent).expect("parent resolved").clone();
        // A value inside JSON is placed where its text is, which the parse
        // already knows. Nothing below applies to it.
        if matches!(pr.ty, Ty::Json(_)) {
            self.resolve_json_child(doc, path)?;
            return Ok(None);
        }
        let (name, ty) = match &pr.ty {
            Ty::Struct(s) => match s.fields.get(idx) {
                Some(f) => (Name::Field(f.name.clone()), f.ty.clone()),
                None => return fail("no such field"),
            },
            Ty::Array { elem, .. } | Ty::Repeat { elem, .. } | Ty::PointerList { elem, .. } | Ty::Chain { elem, .. } => {
                (Name::Index(idx), (**elem).clone())
            }
            // The one thing it points at keeps the field's own name: a row
            // saying `directory` twice says nothing the once did not.
            Ty::At { inner, .. } => (pr.name.clone(), (**inner).clone()),
            // Same again for what a stream holds: the field is the stream, and
            // its first child is what came out of it. Its second, when the
            // codec keeps a trace with blocks in it, is what the decoder read
            // to get there: payload first, machinery second.
            Ty::Decoded { inner, .. } if idx == 0 => (pr.name.clone(), (**inner).clone()),
            Ty::Decoded { .. } => {
                // The trace comes of opening the stream, so a reader who asks
                // for the blocks before asking what came out opens it here.
                self.open_space_at(doc, parent)?;
                let ty = Ty::Traced { part: TracedPart::Blocks };
                return Ok(Some(Place {
                    name: Name::Field("blocks".into()),
                    ty,
                    offset: pr.offset,
                    limit: pr.limit,
                    space: pr.space,
                }));
            }
            Ty::Traced { part } => return self.place_traced(parent, &pr, *part, idx),
            _ => return fail("not a composite"),
        };
        // What a stream holds is read over the bytes it came to, not over the
        // file. The space is opened here, once, and every field below this one
        // inherits it; a field that counts from the start of the file goes
        // back to the file, which is how an RNTuple anchor inside a compressed
        // record still finds its envelopes. See [`space`].
        if let Ty::Decoded { .. } = &pr.ty {
            let space = match self.open_space_at(doc, parent)? {
                space::Opened::Space(id) => id,
                // The stream would not open. `child_count` already said it has
                // nothing inside, so nothing should be asking; answering with
                // an error rather than a wrong place is what is left.
                space::Opened::Refused(_) => return fail("this stream did not open"),
            };
            let limit = self.spaces.len_bits(space);
            return Ok(Some(Place { name, ty, offset: 0, limit, space }));
        }
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
                        cursor: pr.offset,
                        limit: pr.offset,
                        declared_size: Some(0),
                        size: Some(0),
                        computed: None,
                        space: pr.space,
                    };
                    self.remember(path, r);
                    return Ok(None);
                }
            }
        } else if let Ty::Chain { anchor, .. } = &pr.ty {
            // Where the walk found this element. A chain's elements are placed
            // like an `At`'s child rather than one after another, so a file
            // counting from its own start lets them go anywhere in it.
            let anchor = *anchor;
            self.extend_chain_to(doc, parent, idx)?;
            let Some(&at) = self.list(parent).chain_starts.get(idx) else {
                return fail("past the end of the chain");
            };
            if anchor == Anchor::File {
                escapes = Some(doc.len_bits());
            }
            at
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
            let into = if anchor == Anchor::File { 0 } else { pr.space };
            self.no_ring(parent, to, into, &what)?;
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
        // An offset counted from the start of the file means the file, whatever
        // space the field naming it was read in. This is what an RNTuple anchor
        // needs: the anchor is inside a compressed record and its two envelopes
        // are at file offsets it names.
        let space = match &pr.ty {
            Ty::At { anchor: Anchor::File, .. } | Ty::PointerList { anchor: Anchor::File, .. } => 0,
            _ => pr.space,
        };
        if space != pr.space {
            limit = doc.len_bits();
            if offset > limit {
                return fail("runs past the end of the file");
            }
        }
        Ok(Some(Place { name, ty, offset, limit, space }))
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

    fn no_ring(&self, parent: &[usize], to: u64, into: u32, what: &str) -> R<()> {
        const DEEPEST: usize = 1024;
        let mut jumps = 0;
        for k in (0..=parent.len()).rev() {
            let Some(r) = self.memo.get(&parent[..k]) else { continue };
            if matches!(r.ty, Ty::At { .. }) {
                jumps += 1;
            }
            if r.offset == to && r.space == into && r.ty.display_name() == what {
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
            // Only a window in the same space: a stream's own field sits in
            // the file, and counting a decoded offset from where the
            // compressed run happens to start would be counting from a number
            // that means nothing here.
            Anchor::Window => {
                let space = self.memo.get(path).map_or(0, |r| r.space);
                (0..path.len())
                    .rev()
                    .find_map(|k| {
                        self.memo.get(&path[..k]).filter(|r| r.declared_size.is_some() && r.space == space)
                    })
                    .map(|r| r.offset)
                    .unwrap_or(0)
            }
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
        if let Some(starts) = self.list(list).pointer_starts.clone() {
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

    /// Follow the chain at `list` until element `want` has been found, or the
    /// walk has ended. See [`Ty::Chain`] for what ends it.
    ///
    /// Iterative, and it has to be: element `n` is found by reading element
    /// `n - 1`, and doing that by recursion would put a chain of a thousand
    /// records a thousand frames deep. Each step reads one field of the
    /// element before it, which is already resolved because this put it there.
    fn extend_chain_to<S: Source>(&mut self, doc: &Document<S>, list: &[usize], want: usize) -> R<()> {
        loop {
            let state = self.list(list);
            if state.chain_done || state.chain_starts.len() > want {
                return Ok(());
            }
            let n = state.chain_starts.len();
            let lr = self.memo[list].clone();
            let Ty::Chain { first, next, anchor, .. } = &lr.ty else { return fail("not a chain") };
            let (first, next, anchor) = (first.clone(), next.clone(), *anchor);
            let base = self.anchor_base(list, lr.offset, anchor);
            // Where the first element is, or where the one before this said
            // the next one is. An element with no such field ends the chain
            // rather than failing: a record too short to hold its own pointer
            // is a file cut off, and the records before it are still worth
            // showing.
            let at = if n == 0 {
                self.eval_expr(doc, list, &first)?
            } else {
                let mut prev = list.to_vec();
                prev.push(n - 1);
                self.resolve(doc, &prev)?;
                if !self.descend(doc, &mut prev, &next)? {
                    self.list_mut(list).chain_done = true;
                    return Ok(());
                }
                let info = self.node(doc, &prev)?;
                let v = info.value.as_int().unwrap_or(0);
                // All ones for the width of the field it was read from: the
                // other way a format writes "no more". Judged by that field's
                // width, since 0xffff is a terminator in a 16-bit field and an
                // ordinary offset in a 32-bit one.
                if info.size_bits > 0 && info.size_bits < 127 && v == (1i128 << info.size_bits) - 1 {
                    self.list_mut(list).chain_done = true;
                    return Ok(());
                }
                v
            };
            let bits = base as i128 + at * 8;
            let ends = at <= 0
                || bits < 0
                || bits as u64 >= doc.len_bits()
                || n >= crate::template::CHAIN_CAP
                || self.list(list).chain_starts.contains(&(bits as u64));
            if ends {
                self.list_mut(list).chain_done = true;
                return Ok(());
            }
            // Charged like any other element, so a chain long enough to be
            // worth watching hands the caller its screen back.
            self.spend(bits as u64)?;
            self.list_mut(list).chain_starts.push(bits as u64);
        }
    }

    /// Every element of the chain at `list`, walked to the end.
    fn chain_starts<S: Source>(&mut self, doc: &Document<S>, list: &[usize]) -> R<Vec<u64>> {
        self.extend_chain_to(doc, list, usize::MAX)?;
        Ok(self.list(list).chain_starts.clone())
    }

    /// Where every child of a list whose children are not laid out one after
    /// another starts, sorted, with the child it belongs to. Both kinds answer
    /// it: a pointer list from its table of offsets, a chain by following it.
    /// What asks is the search for the child covering a bit, which for a list
    /// in this shape is a halving rather than a walk through all of them.
    fn scattered_starts<S: Source>(
        &mut self,
        doc: &Document<S>,
        list: &[usize],
        lr: &Resolved,
    ) -> R<Vec<(u64, usize)>> {
        if matches!(lr.ty, Ty::Chain { .. }) {
            let mut starts: Vec<(u64, usize)> =
                self.chain_starts(doc, list)?.into_iter().enumerate().map(|(i, s)| (s, i)).collect();
            starts.sort_unstable();
            return Ok(starts);
        }
        self.pointer_starts(doc, list, lr)
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
        space: u32,
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
                Ty::SizedBits { bits, inner } => {
                    let n = self.eval_expr_at(doc, path, &bits, Some((offset, limit)))?;
                    if n < 0 {
                        return fail("negative size");
                    }
                    let bits = n as u64;
                    if offset + bits > limit {
                        return fail(format!("{bits} bits run past the end of the container"));
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
                    // An LSB-first field is at the other end of its byte from
                    // where the cursor counted to, and this is the one place
                    // that knows both the type and where the count reached.
                    // See `decode::lsb_offset`.
                    let placed = match packed_int(&other) {
                        Some((bits, endian)) if lsb_packed(bits, endian, offset) => match lsb_offset(bits, offset) {
                            Some(at) => at,
                            None => return fail(format!(
                                "{bits} bits packed low-bit-first would cross a byte boundary, which has no single range of bits"
                            )),
                        },
                        _ => offset,
                    };
                    return Ok(Resolved {
                        name,
                        ty: other,
                        offset: placed,
                        cursor: offset,
                        limit,
                        declared_size,
                        size: None,
                        computed: None,
                        space,
                    });
                }
            }
        }
    }

    /// Open the stream at `path`, or say why it would not open. Done once per
    /// node: a stream is unpacked when something first asks what is inside it,
    /// and the answer is kept until the memo is thrown away.
    ///
    /// The bytes have to be read before anything can be decided, so this
    /// reports `Pending` like any other read. Everything after that is an
    /// answer rather than an error: a stream that will not open is a fact
    /// about the file, and it should not take the listing down with it.
    pub(super) fn open_space_at<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<space::Opened> {
        if let Some(known) = self.spaces.get(path) {
            return Ok(known);
        }
        self.resolve(doc, path)?;
        let size = self.size_of(doc, path)?;
        let r = self.memo[path].clone();
        let Ty::Decoded { codec, .. } = &r.ty else { return fail("not a decoded stream") };
        let codec = *codec;
        // No decoder reads half a byte, and a compressed run that does not
        // start on one is a template saying something it cannot mean.
        if r.offset % 8 != 0 || size % 8 != 0 {
            self.spaces.refuse(path, Refusal::Unaligned);
            return Ok(space::Opened::Refused(Refusal::Unaligned));
        }
        // A run of no bytes opens into no bytes. Not a refusal: an LZ4 frame's
        // end mark is a block of nothing, and saying it would not unpack would
        // be saying something went wrong where nothing did.
        if size == 0 {
            return Ok(space::Opened::Space(self.spaces.add(path, Vec::new(), crate::codec::Trace::default())));
        }
        if size / 8 > crate::codec::CAP_BYTES as u64 {
            self.spaces.refuse(path, Refusal::TooLarge);
            return Ok(space::Opened::Refused(Refusal::TooLarge));
        }
        let packed = self.read(doc, &r, r.offset, size)?;
        Ok(match crate::codec::decode_traced(codec, &packed) {
            Ok((bytes, trace)) => space::Opened::Space(self.spaces.add(path, bytes, trace)),
            Err(why) => {
                self.spaces.refuse(path, why);
                space::Opened::Refused(why)
            }
        })
    }

    /// Open the `Decoded` node at `path` of space `space` as a document of its
    /// own, and say which space that is. Nothing when the stream would not
    /// open, which is a fact about the file rather than an error.
    ///
    /// Done once per node: asking again for a stream already open hands back
    /// the space it opened. `space` is 0 for a node of the file, and the id of
    /// an already-open space for a stream inside a stream, which is how a zip
    /// inside a gzip is reached.
    pub fn open_space<S: Source>(
        &mut self,
        doc: &Document<S>,
        space: SpaceId,
        path: &[usize],
    ) -> R<Option<SpaceId>> {
        if let Some(id) =
            self.open.iter().flatten().find(|s| s.parent == space && s.path == path).map(|s| s.id)
        {
            return Ok(Some(id));
        }
        // The run is unpacked by whichever reading holds it: the file's own,
        // or the one over the space it sits in. The space is taken out of the
        // registry for the length of the call, since it holds a reading that
        // is about to be asked to do work.
        let unpacked = match space {
            0 => self.unpack(doc, path)?,
            id => {
                let mut held = match self.take(id) {
                    Some(held) => held,
                    None => return fail("no such space"),
                };
                let (ev, sub) = held.reading();
                let got = ev.unpack(sub, path);
                self.put(held);
                got?
            }
        };
        let Some((bytes, trace, codec, inner)) = unpacked else { return Ok(None) };
        let (template, recognised) = self.template_for(&inner, &bytes);
        let id = self.open.len() as SpaceId + 1;
        self.open.push(Some(Box::new(space::Space::new(
            id,
            space,
            path.to_vec(),
            codec,
            bytes,
            trace,
            template,
            recognised,
        ))));
        Ok(Some(id))
    }

    /// The bytes and the trace of the stream at `path`, or nothing when it
    /// would not open. The step every `open_space` starts with, whichever
    /// reading is being asked.
    #[allow(clippy::type_complexity)]
    fn unpack<S: Source>(
        &mut self,
        doc: &Document<S>,
        path: &[usize],
    ) -> R<Option<(std::sync::Arc<Vec<u8>>, crate::codec::Trace, crate::codec::Codec, Ty)>> {
        let id = match self.open_space_at(doc, path)? {
            space::Opened::Space(id) => id,
            space::Opened::Refused(_) => return Ok(None),
        };
        let Ty::Decoded { codec, inner } = self.memo[path].ty.clone() else {
            return fail("not a decoded stream");
        };
        let (Some(bytes), Some(trace)) = (self.spaces.buf(id), self.spaces.trace(id)) else {
            return fail("this stream is no longer open");
        };
        Ok(Some((bytes.clone(), trace.clone(), codec, (*inner).clone())))
    }

    /// What a space's bytes read as: what the stream's own template said, or,
    /// when that said only that they are bytes, whatever they turn out to be.
    ///
    /// This is what makes a gzip of a tar open as a tar. A template that says
    /// something more than "bytes" is believed: a ROOT record's object is an
    /// object whatever a sniffer would make of the first four bytes of it.
    fn template_for(&self, inner: &Ty, bytes: &[u8]) -> (Template, bool) {
        if says_only_bytes(inner) {
            let head = &bytes[..bytes.len().min(0x9000)];
            if let Some(found) = crate::formats::sniff(head, bytes.len() as u64) {
                if let Some(t) = crate::formats::builtin(found) {
                    return (t, true);
                }
            }
        }
        let mut t = Template::new(&self.template.name, inner.clone());
        // The stream's fields may name types the file's template declared, and
        // a reading that does not know them cannot place them.
        t.types = self.template.types.clone();
        (t, false)
    }

    /// A space this reading has opened.
    pub fn space(&self, id: SpaceId) -> Option<&Space> {
        self.open.get(id.checked_sub(1)? as usize)?.as_deref()
    }

    /// A space this reading has opened, to ask something of.
    pub fn space_mut(&mut self, id: SpaceId) -> Option<&mut Space> {
        self.open.get_mut(id.checked_sub(1)? as usize)?.as_deref_mut()
    }

    /// Every space open, in the order they were opened.
    pub fn spaces_open(&self) -> impl Iterator<Item = &Space> {
        self.open.iter().flatten().map(|s| &**s)
    }

    /// Which step of a decoding produced a byte of `space`.
    pub fn map_out(&self, space: SpaceId, byte: u64) -> Option<crate::codec::Step> {
        self.space(space)?.map_out(byte)
    }

    /// Which step read a bit of the run `space` was unpacked from.
    pub fn map_in(&self, space: SpaceId, bit: u64) -> Option<crate::codec::Step> {
        self.space(space)?.map_in(bit)
    }

    fn take(&mut self, id: SpaceId) -> Option<Box<Space>> {
        self.open.get_mut(id.checked_sub(1)? as usize)?.take()
    }

    fn put(&mut self, space: Box<Space>) {
        let at = space.id as usize - 1;
        self.open[at] = Some(space);
    }

    /// Where a `Traced` node's child sits, which is where the decoder said it
    /// read it. Nothing is walked and nothing is measured: a step knows its
    /// own bits, so element a million of a symbol run is one lookup.
    fn place_traced(
        &mut self,
        parent: &[usize],
        pr: &Resolved,
        part: TracedPart,
        idx: usize,
    ) -> R<Option<Place>> {
        let Some((base, trace)) = self.trace_for(parent) else {
            return fail("this stream is no longer open");
        };
        let place = |name: String, ty: Ty, at: u64| {
            Ok(Some(Place { name: Name::Field(name.into()), ty, offset: base + at, limit: pr.limit, space: pr.space }))
        };
        match part {
            TracedPart::Blocks => {
                let Some(block) = trace.blocks().get(idx) else { return fail("no such block") };
                let at = block.in_bits.start;
                place(traced::block_name(block), Ty::Traced { part: TracedPart::Block(idx as u32) }, at)
            }
            TracedPart::Block(i) => {
                let Some(view) = traced::BlockView::of(trace, i) else { return fail("no such block") };
                let head = view.head.len();
                if idx < head {
                    let step = trace.step(view.head.start as usize + idx).expect("in range");
                    let (name, ty) = traced::head_field(&step);
                    place(name, ty, step.in_bits.start)
                } else if idx == head && !view.symbols.is_empty() {
                    let at = view.symbols_at(trace);
                    place("symbols".into(), Ty::Traced { part: TracedPart::Symbols(i) }, at)
                } else {
                    fail("no such field")
                }
            }
            TracedPart::Symbols(i) => {
                let Some(view) = traced::BlockView::of(trace, i) else { return fail("no such block") };
                let k = view.symbols.start as usize + idx;
                if k >= view.symbols.end as usize {
                    return fail("no such symbol");
                }
                let step = trace.step(k).expect("in range");
                let (name, ty) = traced::symbol_ty(&step);
                place(name, ty, step.in_bits.start)
            }
        }
    }

    /// The trace behind a `Traced` node, and where the run it describes starts.
    ///
    /// Found by walking up to the stream that opened it, the same way a space
    /// is: a `Traced` node is always inside one, and the trace belongs to the
    /// stream rather than to the node.
    pub(super) fn trace_for(&self, path: &[usize]) -> Option<(u64, &crate::codec::Trace)> {
        for k in (0..=path.len()).rev() {
            let Some(r) = self.memo.get(&path[..k]) else { continue };
            if matches!(r.ty, Ty::Decoded { .. }) {
                let space::Opened::Space(id) = self.spaces.get(&path[..k])? else { return None };
                return Some((r.offset, self.spaces.trace(id)?));
            }
        }
        None
    }

    /// Whether a stream's trace has anything to show, which decides whether
    /// the stream has a `blocks` child at all. zstd and xz are traced at the
    /// block and have no blocks to open, so they do not get one.
    pub(super) fn has_blocks(&self, path: &[usize]) -> bool {
        self.trace_for(path).is_some_and(|(_, t)| !t.blocks().is_empty())
    }

    /// Which address space a read at `path` belongs to.
    ///
    /// The node's own, when it has been resolved. While it is still being
    /// placed it is not in the memo yet and the answer comes from the nearest
    /// ancestor that is, by the same rule `place_child` follows: below a
    /// stream the space is the one that stream opened, and an offset counted
    /// from the start of the file means the file.
    pub(super) fn space_at(&self, path: &[usize]) -> u32 {
        for k in (0..=path.len()).rev() {
            let Some(r) = self.memo.get(&path[..k]) else { continue };
            if k == path.len() {
                return r.space;
            }
            if matches!(r.ty, Ty::Decoded { .. }) {
                // The stream's second child is what the decoder read, which is
                // bits of the run and so of the space the run is in. Only the
                // first child is on the other side of the codec.
                if path.get(k) == Some(&1) {
                    return r.space;
                }
                return match self.spaces.get(&path[..k]) {
                    Some(space::Opened::Space(id)) => id,
                    _ => r.space,
                };
            }
            if matches!(
                r.ty,
                Ty::At { anchor: Anchor::File, .. } | Ty::PointerList { anchor: Anchor::File, .. }
            ) {
                return 0;
            }
            return r.space;
        }
        0
    }

    /// The value of a named field directly inside the struct at `path`, as a
    /// number, or nothing when it has no numeric reading.
    fn child_int<S: Source>(&mut self, doc: &Document<S>, path: &[usize], field: &str) -> R<Option<i128>> {
        let idx = match &self.memo[path].ty {
            Ty::Struct(s) => s.fields.iter().position(|f| *f.name == *field),
            _ => None,
        };
        let Some(idx) = idx else { return fail(format!("no field named {field}")) };
        let mut p = path.to_vec();
        p.push(idx);
        Ok(self.node(doc, &p)?.value.as_int())
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
