//! The whole web of connections under one node, rather than one field's worth.
//!
//! `origin.rs` answers for a single field: which other fields settled its
//! length, its count, its type or its place. That is what an inspector panel
//! wants, because a panel is showing one row. It is not what a reader wants who
//! is asking what this format *is*: a GGUF header is a hundred fields of which
//! a dozen decide the rest, and the shape of that is not visible one row at a
//! time. Twelve arrows drawn at once say in a glance what twelve visits to
//! twelve panels say in a minute.
//!
//! So this asks the same question of every field in a subtree and hands back
//! the nodes and the arrows between them, ready to be laid out. Nothing here
//! decides how it is drawn: no coordinates, no colours, no grouping. The `kind`
//! on each node is the one concession, and it is only the resolved type said
//! coarsely, so that a view can group by it without reading types itself.
//!
//! Two things are deliberately not done. Values are not read: what a field says
//! is what makes `origins` expensive, and a graph that draws a thousand nodes
//! would pay that a thousand times over for text no arrow shows. And an arrow
//! whose far end is not in the subtree is dropped rather than pointed at
//! nothing, since a graph with edges hanging off it reads as a graph that is
//! missing nodes.

use super::*;
use super::origin::Role;

/// One field of the subtree, as much of it as an arrow needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphNode {
    pub path: Vec<usize>,
    pub name: String,
    /// The type as a short kind the view can group by: `u16`, `u32`, `f32`,
    /// `bytes`, `str`, `struct`, `array`, `repeat`, and the rest of what
    /// [`kind_of`] names. Derived from the resolved `Ty`, coarse, and stable:
    /// a view that colours by it should not have the colours move because a
    /// template started spelling a type another way.
    pub kind: String,
    pub offset_bits: u64,
    pub size_bits: u64,
    /// Index into the returned node list, or `usize::MAX` for the root.
    pub parent: usize,
    pub child_count: u64,
    /// True when this node has children that were not walked because the cap
    /// was reached.
    pub truncated: bool,
}

/// One field deciding something about another, as an arrow between two nodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphEdge {
    /// Index into the node list. The field that decided.
    pub from: usize,
    /// Index into the node list. The field it decided about.
    pub to: usize,
    /// [`Role::as_str`] of the origin that produced it: `length`, `count`,
    /// `type`, `position`, `value`, `name`, `width`, `points`.
    pub role: &'static str,
}

/// The subtree under one node, and what its fields decide about each other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Graph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    /// How many nodes existed under `path` beyond the cap, as far as is known.
    /// A run whose count would take a walk of the whole file to settle
    /// contributes what the walk reached and no more, so this is a floor
    /// rather than a total.
    pub omitted: u64,
}

/// What the parent index says when there is no parent: the node the graph was
/// asked for, which is the one node in it with nothing above it.
pub const NO_PARENT: usize = usize::MAX;

impl Evaluator {
    /// Every field under `path`, and every connection between two of them.
    ///
    /// Breadth-first, and stopping at `limit` nodes. Breadth-first because a
    /// cap has to fall somewhere, and a cap on a depth-first walk keeps one
    /// deep spine of a format and none of its siblings, which is the opposite
    /// of what a reader wanting the shape of the thing asked for. Cut across,
    /// what is left is the top of the format: the sections, then their fields.
    ///
    /// A field that will not read is passed over rather than taking the graph
    /// with it, the same way a run carries on past an element it cannot place.
    /// Bytes that have not arrived are collected across the whole walk and
    /// asked for together, so a page of scattered records is one wait and not
    /// one per record. The node the caller asked about is the exception to
    /// both: if that will not resolve there is no subtree to answer with.
    pub fn graph<S: Source>(&mut self, doc: &Document<S>, path: &[usize], limit: usize) -> R<Graph> {
        self.resolve(doc, path)?;
        let mut g = Graph { nodes: Vec::new(), edges: Vec::new(), omitted: 0 };
        // Bytes an answer was given without. Gathered rather than returned at
        // the first miss: see `Evaluator::children`, which waits the same way
        // and for the same reason.
        let mut missing: Vec<Missing> = Vec::new();
        // What is waiting to be walked, oldest first, each with the index of
        // the node it hangs under.
        let mut queue: std::collections::VecDeque<(Vec<usize>, usize)> = std::collections::VecDeque::new();
        queue.push_back((path.to_vec(), NO_PARENT));
        while let Some((at, parent)) = queue.pop_front() {
            if g.nodes.len() >= limit {
                // Only reachable for a cap of nothing at all: a child is
                // queued only where there is room left for it to become a
                // node. What is still waiting is what the cap costs.
                g.omitted += queue.len() as u64 + 1;
                break;
            }
            match self.add_node(doc, &at, parent, &mut g, &mut queue, limit) {
                Ok(()) => {}
                Err(EvalError::Busy { reached_bits }) => return Err(EvalError::Busy { reached_bits }),
                // The bytes are on their way. The rest of the subtree does not
                // depend on them, so it is walked meanwhile and the caller is
                // told at the end what to fetch.
                Err(EvalError::Pending(m)) => missing.extend(m),
                // A field the template cannot read here. Its siblings still
                // read, and a graph missing one node says more than an error
                // in place of the whole subtree. The node the caller asked
                // about is not one of those: with that unreadable there is no
                // subtree to answer with.
                Err(e @ EvalError::Failed(_)) => {
                    if parent == NO_PARENT {
                        return Err(e);
                    }
                }
            }
        }
        // The arrows, once every node is in and can be looked up. Doing this
        // per node as the walk went would drop every edge pointing forwards.
        self.add_edges(doc, &mut g, &mut missing)?;
        if !missing.is_empty() {
            missing.sort_by_key(|m| m.chunk);
            missing.dedup();
            return Err(EvalError::Pending(missing));
        }
        Ok(g)
    }

    /// One field of the walk: what it is, where it is, and what hangs under it.
    ///
    /// Everything here comes from the memo. Nothing reads the field's bytes: a
    /// name that the file rather than the template spells, which is what
    /// `listing::label` goes and looks up, is a read per node and is worth
    /// nothing to an arrow.
    fn add_node<S: Source>(
        &mut self,
        doc: &Document<S>,
        at: &[usize],
        parent: usize,
        g: &mut Graph,
        queue: &mut std::collections::VecDeque<(Vec<usize>, usize)>,
        limit: usize,
    ) -> R<()> {
        self.resolve(doc, at)?;
        let r = self.memo[at].clone();
        // Charged like any element of any other walk, so that a graph asked of
        // a file with millions of fields hands the caller its screen back
        // instead of holding the thread.
        self.spend(r.offset)?;
        let size = self.size_of(doc, at)?;
        let here = g.nodes.len();
        g.nodes.push(GraphNode {
            path: at.to_vec(),
            name: r.name.text(),
            kind: kind_of(&self.template, &r.ty),
            offset_bits: r.offset,
            size_bits: size,
            parent,
            child_count: 0,
            truncated: false,
        });
        // Room for what is already in and what is already queued: a child
        // queued now is a node later, and counting only the nodes would queue
        // far more than the cap allows and then drop most of them.
        let room = limit.saturating_sub(g.nodes.len() + queue.len());
        // Counting the children is a walk of its own, and it can want bytes or
        // run out of the go's allowance. The node stands either way, saying
        // that what is under it was not reached.
        let (count, all) = match self.graph_child_count(doc, at, room) {
            Ok(v) => v,
            Err(e) => {
                g.nodes[here].truncated = true;
                return if e.interrupted() { Err(e) } else { Ok(()) };
            }
        };
        g.nodes[here].child_count = count;
        let taken = room.min(usize::try_from(count).unwrap_or(usize::MAX));
        for i in 0..taken {
            let mut child = at.to_vec();
            child.push(i);
            queue.push_back((child, here));
        }
        // A count that stopped short is still a count that stopped short: the
        // node says so whether the cap or the walk was what stopped it.
        if (taken as u64) < count || !all {
            g.nodes[here].truncated = true;
            g.omitted += count - taken as u64;
        }
        Ok(())
    }

    /// How many children to walk under this node, and whether that is all of
    /// them.
    ///
    /// A run that fills its container with elements only their own bytes
    /// measure has no count without decoding every one, which for a code
    /// section is a minute of work for a number the graph would then throw most
    /// of away. So such a run is asked for as many children as there is `room`
    /// for and no more, one at a time, and stops at the first that will not
    /// place. `false` says the answer is what was reached rather than the
    /// whole. See [`Evaluator::count_unless_walk`].
    fn graph_child_count<S: Source>(&mut self, doc: &Document<S>, at: &[usize], room: usize) -> R<(u64, bool)> {
        if let Some(n) = self.count_unless_walk(doc, at)? {
            return Ok((n, true));
        }
        let mut child = at.to_vec();
        for i in 0..room {
            child.push(i);
            let placed = self.resolve(doc, &child);
            child.pop();
            match placed {
                Ok(()) => {}
                Err(e) if e.interrupted() => return Err(e),
                // The run ended on something that would not read, which is what
                // the bytes after the last whole element look like, and is
                // where counting it would have stopped too.
                Err(_) => return Ok((i as u64, true)),
            }
        }
        Ok((room as u64, false))
    }

    /// Every connection between two nodes of the walk.
    ///
    /// Asked of each node in turn, because that is the direction the template
    /// is written in: a field names what decided it, not what it decides. The
    /// arrow is drawn the other way round, from the field that decided to the
    /// field it decided about, since that is the direction the reader follows:
    /// this number is why that run is 40 bytes long.
    ///
    /// An arrow whose far end is outside the subtree, or past the cap, is
    /// dropped. So is a repeated one: `a + a` names the same field twice and
    /// means one connection.
    fn add_edges<S: Source>(&mut self, doc: &Document<S>, g: &mut Graph, missing: &mut Vec<Missing>) -> R<()> {
        let index: rustc_hash::FxHashMap<Vec<usize>, usize> =
            g.nodes.iter().enumerate().map(|(i, n)| (n.path.clone(), i)).collect();
        for to in 0..g.nodes.len() {
            let at = g.nodes[to].path.clone();
            // Charged separately from placing the node: asking what decided a
            // field means following every expression its declaration holds,
            // which is work of its own and is worth as much of the allowance.
            self.spend(g.nodes[to].offset_bits)?;
            match self.origins_no_values(doc, &at) {
                Ok(found) => {
                    for o in found {
                        let Some(&from) = index.get(&o.path) else { continue };
                        g.edges.push(GraphEdge { from, to, role: o.role.as_str() });
                    }
                }
                Err(EvalError::Busy { reached_bits }) => return Err(EvalError::Busy { reached_bits }),
                Err(EvalError::Pending(m)) => missing.extend(m),
                Err(EvalError::Failed(_)) => {}
            }
            // Where this field points, when it is one of a pointer list's
            // offsets. Asked apart from the rest because the answer is a node
            // rather than a field that decided one, and because it is the one
            // arrow that runs from this field outward.
            match self.points_at(doc, &at) {
                Ok(Some((_, _, target))) => {
                    if let Some(&pointed) = index.get(&target) {
                        g.edges.push(GraphEdge { from: to, to: pointed, role: Role::Points.as_str() });
                    }
                }
                Ok(None) => {}
                Err(EvalError::Busy { reached_bits }) => return Err(EvalError::Busy { reached_bits }),
                Err(EvalError::Pending(m)) => missing.extend(m),
                Err(EvalError::Failed(_)) => {}
            }
        }
        g.edges.sort_by_key(|e| (e.from, e.to, e.role));
        g.edges.dedup();
        Ok(())
    }
}

/// What to call this type, coarsely, so a view can group by it.
///
/// Not [`Ty::display_name`]: that is written for a type column a reader reads,
/// and it says `u bits_per_value be` and `offsets → Tensor`, which are facts
/// about one template rather than kinds anything can be grouped into. This
/// answers the question a legend asks instead: is it a number, and how wide;
/// is it text; is it a list, and of which sort.
///
/// The words are part of the interface. A view keys colours and filters off
/// them, so they are chosen to stay put: a template that starts spelling a
/// width differently must not move a node into another group.
pub fn kind_of(template: &Template, ty: &Ty) -> String {
    match ty {
        // A sentinel says how one value reads and nothing about the shape, so
        // the kind is the kind of what is under it.
        Ty::Nullable { inner, .. } => kind_of(template, inner),
        // A resolved node has these unwrapped already, and they are answered
        // anyway so that a caller holding a declared type gets the same answer.
        Ty::Sized { inner, .. } | Ty::SizedBits { inner, .. } | Ty::Origin { inner } => kind_of(template, inner),
        Ty::Named(n) => match template.types.get(&**n) {
            Some(t) => kind_of(template, t),
            None => "named".to_string(),
        },
        Ty::UInt { bits, .. } => format!("u{bits}"),
        // Two's complement and sign-and-magnitude are the same question to a
        // reader grouping by kind: a signed number that wide.
        Ty::Int { bits, .. } | Ty::SignMagnitude { bits, .. } => format!("i{bits}"),
        // How wide it is, is what the file says rather than what the template
        // does, so there is no width to put in the name.
        Ty::UIntExpr { .. } => "uint".to_string(),
        // Every number written a byte at a time until it says it has finished.
        // Which of the spellings is a fact about the format, not a kind.
        Ty::Leb128 { .. } | Ty::Zigzag | Ty::Vlq | Ty::SqliteVarint | Ty::SevenZipNumber | Ty::EbmlVint { .. } => {
            "varint".to_string()
        }
        Ty::F16(_) => "f16".to_string(),
        Ty::BF16(_) => "bf16".to_string(),
        Ty::F32(_) => "f32".to_string(),
        Ty::F64(_) => "f64".to_string(),
        Ty::F80(_) => "f80".to_string(),
        Ty::F8 { .. } => "f8".to_string(),
        Ty::Fixed { .. } => "fixed".to_string(),
        // A number the file wrote out in words: a FITS keyword's value, a
        // decimal length in a tar header.
        Ty::TextInt { .. } => "digits".to_string(),
        Ty::Magic(_) => "magic".to_string(),
        Ty::Bytes(_) => "bytes".to_string(),
        Ty::Str { .. } => "str".to_string(),
        // Both kept apart from the numbers they are read as: what a reader
        // does with a named value is not what they do with a measurement.
        Ty::Enum { .. } => "enum".to_string(),
        Ty::Flags { .. } => "flags".to_string(),
        // Worked out rather than read. Text or number, it has no bytes of its
        // own and a view showing where the file's weight is should not colour
        // it as though it had.
        Ty::Computed(_) | Ty::ComputedText(_) => "computed".to_string(),
        Ty::Insn { .. } => "insn".to_string(),
        Ty::Json(..) => "json".to_string(),
        Ty::Struct(_) => "struct".to_string(),
        Ty::Array { .. } => "array".to_string(),
        Ty::Repeat { .. } => "repeat".to_string(),
        // Told apart from an array because the children are somewhere else in
        // the file, which is the fact a reader of the graph most wants: the
        // arrows out of a pointer list go across the file, not down it.
        Ty::PointerList { .. } => "pointers".to_string(),
        Ty::Chain { .. } => "chain".to_string(),
        Ty::At { .. } => "at".to_string(),
        Ty::Decoded { .. } => "stream".to_string(),
        // The parts of a decoder's trace are one kind here, the codes in a
        // block included: what a reader wants to know is that these nodes came
        // out of a decoding rather than out of the file.
        Ty::Traced { .. } | Ty::CodeBits { .. } => "trace".to_string(),
        // A resolved node has taken a case already; this is only reachable
        // from a declared type.
        Ty::Switch { .. } | Ty::Match { .. } => "switch".to_string(),
    }
}

/// The word a field's *value* would be given, worked out from its type rather
/// than by reading it.
///
/// The listing already labels every row this way, but it gets the word out of
/// the `Value` a read produced. Totalling a whole file by kind cannot afford
/// that: reading the value of every field means decoding every string and
/// every packed number in the file to learn something the type already says.
/// So this asks the type the same question, and answers in exactly the
/// vocabulary the rows use, because a view colouring a treemap and a view
/// colouring a listing have to agree about what a field is.
///
/// Two of the words a value can carry are facts about the bytes rather than
/// about the type and so never come out of here: `unread`, which means the
/// bytes have not arrived, and `unset`, which means a slot holds its format's
/// "nobody filled this in" value. A `Nullable` answers as the number under it.
pub fn value_kind(template: &Template, ty: &Ty) -> &'static str {
    match ty {
        // Wrappers that say where a field is or how wide its window is, and
        // nothing about what it holds.
        Ty::Nullable { inner, .. } => value_kind(template, inner),
        Ty::Sized { inner, .. } | Ty::SizedBits { inner, .. } | Ty::Origin { inner } => value_kind(template, inner),
        Ty::Named(n) => match template.types.get(&**n) {
            // A name nothing defines is a template that cannot be read, and
            // the bytes under it are all anyone can say about it.
            Some(t) => value_kind(template, t),
            None => "bytes",
        },
        Ty::UInt { .. } | Ty::UIntExpr { .. } | Ty::Vlq | Ty::EbmlVint { .. } | Ty::SevenZipNumber => "uint",
        Ty::Leb128 { signed } => {
            if *signed { "int" } else { "uint" }
        }
        // SQLite's varint is signed, a zigzag is signed by construction, and a
        // number the file wrote out in digits is read into an `i128` whichever
        // way it was written. A computed field is a number worked out rather
        // than read, and `Value::Int` is what carries it.
        Ty::Int { .. } | Ty::SignMagnitude { .. } | Ty::Zigzag | Ty::SqliteVarint | Ty::TextInt { .. } | Ty::Computed(_) => "int",
        // A fixed-point number reads as the fraction it stands for, not as the
        // integer it is stored as.
        Ty::F16(_) | Ty::BF16(_) | Ty::F32(_) | Ty::F64(_) | Ty::F80(_) | Ty::F8 { .. } | Ty::Fixed { .. } => "float",
        Ty::Magic(_) => "magic",
        Ty::Enum { .. } => "enum",
        Ty::Flags { .. } => "flags",
        // Text found in the file, and text a template worked out, read as
        // that text.
        Ty::Str { .. } | Ty::ComputedText(_) => "str",
        // An instruction *reads* as the line the disassembler wrote, and used
        // to be counted as text for that reason. It is not text. Six hundred
        // kilobytes of a busybox binary are machine code, and a picture of
        // what the file is made of that called them strings was answering a
        // question about how they are displayed rather than what they are.
        Ty::Insn { .. } => "insn",
        // A JSON number may be whole or not, which only the digits say; both
        // words mean the same thing to a view, and the type column carries the
        // distinction for anyone who wants it. `true`, `false` and `null` are
        // shown as the words the file wrote.
        Ty::Json(shape, _) if shape.composite() => "composite",
        Ty::Json(crate::json::Shape::Number, _) => "int",
        Ty::Json(..) => "str",
        // A Huffman code reads as a string of noughts and ones, and is not
        // text, the same distinction an instruction is kept apart for: a
        // picture of what a file is made of should count a compressed block's
        // codes as the compressed bytes they are. No word of its own, because
        // the answer to "what is this file made of" is already `stream` a
        // level up and these are what is inside one.
        Ty::Bytes(_) | Ty::CodeBits { .. } => "bytes",
        // Everything that holds other fields, including the two that hold
        // fields which are not bits of this file: a stream's contents are at
        // offsets of the bytes it unpacked to, and a trace's symbols are what
        // it read to produce them.
        Ty::Struct(_)
        | Ty::Array { .. }
        | Ty::Repeat { .. }
        | Ty::PointerList { .. }
        | Ty::Chain { .. }
        | Ty::At { .. }
        | Ty::Decoded { .. }
        | Ty::Traced { .. } => "composite",
        // A resolved node has taken a case already; reached only from a
        // declared type, where nothing yet says which shape it will be.
        Ty::Switch { .. } | Ty::Match { .. } => "bytes",
    }
}
