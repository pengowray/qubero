//! The format as boxes and arrows, read off the template rather than off a file.
//!
//! `graph.rs` answers about one file: these are the fields it turned out to
//! have, and this one decided that one's length. That is a picture of a
//! reading. A reader asking what a format *is* wants the other picture: the
//! types the format declares, the fields of each, and which field of which type
//! settles the shape of which other. Nothing in it depends on a file being
//! open, and nothing in it is read from one.
//!
//! So this walks a [`Template`] and hands back one box per type, one row per
//! field, and one edge per connection. No coordinates and no colours: the row's
//! `kind` is the same word [`crate::eval::value_kind`] gives the listing, so a
//! view can colour a row the way it colours the same field everywhere else, and
//! that is the whole of what is decided here.
//!
//! Two kinds of edge, which are the two questions a structure diagram answers:
//!
//! - **What is in this field**, which is [`Role::Type`]: the field is read as
//!   a type that has a box of its own, or as a switch whose cases have boxes.
//!   A switch is a box too (its rows are its cases), so the fan-out is drawn
//!   once rather than once per case out of one row.
//! - **What decided this field**, which is every other role: the expression
//!   that sized it, counted it, placed it or named it, with an edge from the
//!   row of every field the expression reads. Written with the same renderer
//!   the relations panel uses, so the diagram and the panel say the same thing.
//!
//! What is deliberately not here: any walk that needs bytes. A name an
//! expression uses that no row in scope answers produces no edge rather than a
//! guess, the same rule `graph.rs` follows for an arrow whose far end is
//! outside the subtree.

use std::collections::HashMap;
use std::sync::Arc;

use super::graph::value_kind;
use super::origin::Role;
use super::relate::write_expr;
use crate::template::{Expr, Packing, StrLen, StructDef, Template, Ty, Until};
use crate::template_text::tag_lit;

/// How many boxes one diagram may hold. A format whose switch cases are all
/// written inline can declare hundreds of shapes, and past a few hundred boxes
/// the picture is a wall rather than a diagram. What is left out is counted.
pub const BOX_CAP: usize = 300;

/// What a box stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxKind {
    /// A structure: its rows are its fields, in the order the format writes
    /// them.
    Seq,
    /// Fields worked out rather than written, which a format declares apart
    /// from the ones it lays out. Nothing produces one yet: the IR has no
    /// instances of its own, and the converter that will bring them (a Kaitai
    /// `instances` block) is a later wave.
    // TODO: emit these once `instances` lowers to something the walk can tell
    // apart from a trailing `Computed` or `At` field.
    Instances,
    /// A choice: its rows are the cases, and each case names the type taken
    /// when the switch reads that value.
    Switch,
}

impl BoxKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BoxKind::Seq => "seq",
            BoxKind::Instances => "instances",
            BoxKind::Switch => "switch",
        }
    }
}

/// One row of a box: one field of a structure, or one case of a switch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub name: String,
    /// The type as the listing's type column writes it. See
    /// [`Ty::display_name`].
    pub type_text: String,
    /// How long the field runs, when the template says: `4 bytes`, or the
    /// expression that decides it. Empty when only reading the file settles it.
    pub size_text: String,
    /// Where the field starts, counted from the front of this type, when every
    /// field before it is a fixed width. The expression instead for a field
    /// read at an address. Empty when neither.
    pub pos_text: String,
    /// True when the field holds a list of elements: an array, a repeat, or
    /// one of the lists whose elements are placed. The type column writes those
    /// four ways, so a view drawing a list as one asks this instead. See
    /// [`lists`].
    pub list: bool,
    /// The word [`crate::eval::value_kind`] gives this field, so a view colours
    /// it the way it colours the same field in the listing: `uint`, `str`,
    /// `magic`, `enum`, `composite` and the rest.
    pub kind: &'static str,
}

/// One type of the format, and its fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeBox {
    /// What the type is called. A type declared inside a field rather than in
    /// the template's table is named for where it was found: `Header.entry`.
    pub name: String,
    /// Where the walk first reached it: `png.chunks.data.'IHDR'`. The name is
    /// what the type is called and is what a box is labelled with; this is how
    /// to get to it, which is a different question and belongs on a second
    /// line. A type read in nine places has one of these, the first.
    pub path: String,
    /// What tells this type from every other: the structural key the walk
    /// shares boxes by. A caller that has counted the open file's nodes by the
    /// same key can say how many of each box the file holds; see
    /// [`crate::eval::census`].
    pub key: String,
    pub kind: BoxKind,
    /// The type this one was declared inside, for a box that has no name of its
    /// own in the template. None for a named type, which may be used from
    /// anywhere.
    pub parent: Option<String>,
    pub rows: Vec<Row>,
}

/// One connection, drawn from the row that decides to the row it decides about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagramEdge {
    /// The box and the row the edge leaves: the field that decided, or the
    /// field whose type is being named.
    pub from: (usize, usize),
    /// The box the edge arrives at, and the row in it when the edge is about
    /// one row. None for an edge to the box as a whole, which is what naming a
    /// type is.
    pub to: (usize, Option<usize>),
    pub role: Role,
    /// The expression the edge stands for, as the template writes it. Empty for
    /// an edge that is a declaration rather than an expression: a field read as
    /// a named type, or a switch case.
    pub label: String,
}

/// The whole picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagram {
    pub types: Vec<TypeBox>,
    pub edges: Vec<DiagramEdge>,
    /// Named types of the template with no box here: the ones nothing reachable
    /// from the root refers to, the ones that are not structures, and the ones
    /// past [`BOX_CAP`].
    pub omitted: u32,
}

/// The template as boxes and arrows.
///
/// The walk starts at the root and follows every type it can reach: a field's
/// declared type, the element type of a list, the cases of a switch. A named
/// type gets its box the first time it is reached and is never drawn twice, so
/// a type that refers to itself is one box with an edge back to it.
pub fn diagram(t: &Template) -> Diagram {
    let mut w = Walk {
        t,
        boxes: Vec::new(),
        defs: Vec::new(),
        by_text: HashMap::new(),
        by_path: HashMap::new(),
        edges: Vec::new(),
        named_drawn: 0,
        capped: 0,
    };
    // A root that is one of the named types is that type's box, not a second
    // copy of it under another name.
    match root_name(&t.root) {
        Some(name) => {
            w.named_box(&name);
        }
        // A root that is a choice is a choice: an HDF5 file is a superblock or
        // a user block and then a superblock, and the template says so at the
        // root. Drawn as a structure it drew nothing at all.
        None => match as_switch(t, &t.root) {
            Some(sw) => {
                let sw = sw.clone();
                let key = switch_key(&sw);
                w.switch_box(t.name.clone(), None, &sw, key);
            }
            None => {
                if let Some(sd) = as_struct(t, &t.root) {
                    let sd = sd.clone();
                    w.struct_box(t.name.clone(), None, &sd);
                }
            }
        },
    }
    w.link();
    let omitted = (t.types.len() as u32).saturating_sub(w.named_drawn) + w.capped;
    Diagram { types: w.boxes, edges: w.edges, omitted }
}

/// The name of the template's root, when the root is just a reference into the
/// type table. Wrappers that say where the root sits or how big it is do not
/// change which type it is.
fn root_name(ty: &Ty) -> Option<String> {
    match ty {
        Ty::Named(n) => Some(n.to_string()),
        Ty::Sized { inner, .. } | Ty::SizedBits { inner, .. } | Ty::Origin { inner } | Ty::At { inner, .. } => {
            root_name(inner)
        }
        _ => None,
    }
}

/// The structure inside a type, past the wrappers that say where it is, how big
/// it is, or how it is packed. None for a type that is not one: a number, a
/// string, a list of numbers.
///
/// The `Arc` rather than the `StructDef` behind it, because which structure
/// this is, is the pointer: a format that reads the same record in nine places
/// clones one `Arc` nine times, and that is what says the nine are one type and
/// not nine that happen to have the same fields.
pub(crate) fn struct_of<'a>(t: &'a Template, ty: &'a Ty) -> Option<&'a Arc<StructDef>> {
    as_struct(t, ty)
}

fn as_struct<'a>(t: &'a Template, ty: &'a Ty) -> Option<&'a Arc<StructDef>> {
    match ty {
        Ty::Struct(sd) => Some(sd),
        Ty::Sized { inner, .. } | Ty::SizedBits { inner, .. } | Ty::Origin { inner } | Ty::At { inner, .. } => {
            as_struct(t, inner)
        }
        Ty::Nullable { inner, .. } | Ty::Decoded { inner, .. } | Ty::When { inner, .. } => as_struct(t, inner),
        Ty::Stitched { inner, .. } => as_struct(t, inner),
        Ty::Array { elem, .. }
        | Ty::Repeat { elem, .. }
        | Ty::PointerList { elem, .. }
        | Ty::Chain { elem, .. }
        | Ty::Gather { elem, .. } => as_struct(t, elem),
        Ty::Named(n) => t.types.get(&**n).and_then(|inner| as_struct(t, inner)),
        _ => None,
    }
}

/// The switch inside a type, past the same wrappers. Told apart from a
/// structure because a switch is drawn as a box of its own whose rows are the
/// cases, which is the one shape a structure's rows cannot say.
/// What a switch is called: the question it asks. A switch has no name of its
/// own in the IR, and what a reader wants to know about a choice is what
/// decides it, so the box and the row that reaches it say the same words.
///
/// Only a switch written here, not one reached through a named type or a list:
/// those have a name already, and `foo` is a better answer than the question
/// asked inside it. None when the expression has no reading.
fn switch_title(ty: &Ty) -> Option<String> {
    match ty {
        Ty::Switch { on, .. } | Ty::Match { on, .. } => write_expr(on).map(|e| format!("switch on {e}")),
        _ => None,
    }
}

fn as_switch<'a>(t: &'a Template, ty: &'a Ty) -> Option<&'a Ty> {
    match ty {
        Ty::Switch { .. } | Ty::Match { .. } => Some(ty),
        Ty::Sized { inner, .. } | Ty::SizedBits { inner, .. } | Ty::Origin { inner } | Ty::At { inner, .. } => {
            as_switch(t, inner)
        }
        Ty::Nullable { inner, .. } | Ty::Decoded { inner, .. } | Ty::When { inner, .. } => as_switch(t, inner),
        Ty::Stitched { inner, .. } => as_switch(t, inner),
        Ty::Array { elem, .. }
        | Ty::Repeat { elem, .. }
        | Ty::PointerList { elem, .. }
        | Ty::Chain { elem, .. }
        | Ty::Gather { elem, .. } => as_switch(t, elem),
        _ => None,
    }
}

/// The named type a field is read as, when it is read as one. A list of a named
/// type answers with the element's name: what the arrow should reach is the
/// type the elements are, and that a hundred of them are read changes the row's
/// own type column rather than where the arrow goes.
fn named_target(ty: &Ty) -> Option<String> {
    match ty {
        Ty::Named(n) => Some(n.to_string()),
        Ty::Sized { inner, .. } | Ty::SizedBits { inner, .. } | Ty::Origin { inner } | Ty::At { inner, .. } => {
            named_target(inner)
        }
        Ty::Nullable { inner, .. } | Ty::Decoded { inner, .. } | Ty::When { inner, .. } => named_target(inner),
        Ty::Stitched { inner, .. } => named_target(inner),
        Ty::Array { elem, .. }
        | Ty::Repeat { elem, .. }
        | Ty::PointerList { elem, .. }
        | Ty::Chain { elem, .. }
        | Ty::Gather { elem, .. } => named_target(elem),
        _ => None,
    }
}

/// How many bits a type takes whatever file it is read in, for the types whose
/// width the template fixes outright. None as soon as anything about it is read
/// from the file, which is what stops a running offset once one field's length
/// depends on the bytes.
fn static_bits(t: &Template, ty: &Ty, depth: u32) -> Option<u64> {
    if depth > 16 {
        return None;
    }
    match ty {
        Ty::UInt { bits, .. } | Ty::Int { bits, .. } | Ty::SignMagnitude { bits, .. } => Some(u64::from(*bits)),
        Ty::Fixed { bits, .. } => Some(u64::from(*bits)),
        Ty::F16(_) | Ty::BF16(_) => Some(16),
        Ty::F32(_) => Some(32),
        Ty::F64(_) => Some(64),
        Ty::F80(_) => Some(80),
        Ty::IbmF32(_) => Some(32),
        Ty::F8 { .. } => Some(8),
        Ty::Magic(b) => Some(b.len() as u64 * 8),
        // Worked out rather than read, so it covers nothing at all.
        Ty::Computed(_) | Ty::ComputedText(_) | Ty::ComputedReal(_) => Some(0),
        // The field itself is nothing; its contents are somewhere else.
        Ty::At { .. } => Some(0),
        // Through `try_from` rather than `as`: a negative literal is not a
        // length, and taken as one it is eighteen million terabytes and a
        // multiplication that panics. No builtin writes one; a converted format
        // may.
        Ty::Bytes(Expr::Lit(n)) => u64::try_from(*n).ok()?.checked_mul(8),
        Ty::Str { len: StrLen::Fixed(Expr::Lit(n)) | StrLen::Padded { size: Expr::Lit(n), .. }, .. }
        | Ty::TextInt { len: StrLen::Fixed(Expr::Lit(n)) | StrLen::Padded { size: Expr::Lit(n), .. }, .. } => {
            u64::try_from(*n).ok()?.checked_mul(8)
        }
        Ty::Sized { size: Expr::Lit(n), .. } => u64::try_from(*n).ok()?.checked_mul(8),
        Ty::SizedBits { bits: Expr::Lit(n), .. } => u64::try_from(*n).ok(),
        Ty::Nullable { inner, .. } | Ty::Enum { inner, .. } | Ty::Flags { inner, .. } | Ty::Origin { inner } => {
            static_bits(t, inner, depth + 1)
        }
        // A field that may not be there has no width the template fixes: what
        // it costs depends on what the file says. Saying its inner width would
        // put every field after it at an offset no file need agree with.
        Ty::When { .. } => None,
        Ty::Array { elem, count: Expr::Lit(n) } => {
            static_bits(t, elem, depth + 1).and_then(|b| b.checked_mul(u64::try_from(*n).ok()?))
        }
        Ty::Struct(sd) => {
            let mut total = 0u64;
            for f in &sd.fields {
                // A second reading of bytes another field already covers
                // advances nothing.
                if f.aside {
                    continue;
                }
                let bits = static_bits(t, &f.ty, depth + 1)?;
                // A union is as wide as its widest field: every field of one
                // starts where the record does, so a sibling after it would
                // be drawn at an offset no file agrees with if these added
                // up. See `StructDef::overlap`.
                total = match sd.overlap {
                    true => total.max(bits),
                    false => total.checked_add(bits)?,
                };
            }
            Some(total)
        }
        Ty::Named(n) => t.types.get(&**n).and_then(|inner| static_bits(t, inner, depth + 1)),
        _ => None,
    }
}

/// A width in the words a size column uses. Bits only where the field is not a
/// whole number of bytes, since that is the only time the distinction matters
/// and a column of `32 bits` reads worse than one of `4 bytes`.
fn bits_text(bits: u64) -> String {
    if bits % 8 == 0 {
        let bytes = bits / 8;
        if bytes == 1 { "1 byte".to_string() } else { format!("{bytes} bytes") }
    } else if bits == 1 {
        "1 bit".to_string()
    } else {
        format!("{bits} bits")
    }
}

/// The size column for one field: what the template fixes, or the expression
/// that decides it, written the way the relations panel writes it.
fn size_text(t: &Template, ty: &Ty) -> String {
    if let Some(bits) = static_bits(t, ty, 0) {
        return bits_text(bits);
    }
    match ty {
        Ty::Bytes(e) | Ty::Sized { size: e, .. } => {
            write_expr(e).map_or(String::new(), |s| format!("{s} bytes"))
        }
        Ty::Str { len: StrLen::Fixed(e) | StrLen::Padded { size: e, .. }, .. }
        | Ty::TextInt { len: StrLen::Fixed(e) | StrLen::Padded { size: e, .. }, .. } => {
            write_expr(e).map_or(String::new(), |s| format!("{s} bytes"))
        }
        Ty::SizedBits { bits: e, .. } => write_expr(e).map_or(String::new(), |s| format!("{s} bits")),
        Ty::UIntExpr { bits: e, .. } => write_expr(e).map_or(String::new(), |s| format!("{s} bits")),
        // A run is as long as its count times its element, which is a size and
        // says where the count came from in the same breath. The element's own
        // width goes in where the template fixes it; where it does not, the
        // word stands in for it rather than a number no file would agree with.
        Ty::Array { count, elem } => write_expr(count).map_or(String::new(), |s| {
            match static_bits(t, elem, 0) {
                Some(b) => format!("{s} \u{d7} {}", bits_text(b)),
                None => format!("{s} \u{d7} element"),
            }
        }),
        Ty::Nullable { inner, .. } | Ty::Enum { inner, .. } | Ty::Flags { inner, .. } | Ty::Origin { inner } => {
            size_text(t, inner)
        }
        // Nothing at all, or whatever is inside it. The type column already
        // says `optional`, so the size says what it is when it is there.
        Ty::When { inner, .. } => size_text(t, inner),
        // Where a run stops, which is the nearest thing it has to a length. The
        // two that compare one field say which field; a condition is written
        // out the way every other expression here is.
        Ty::Repeat { until, .. } => match until {
            Until::End => "to the end".to_string(),
            Until::FieldValue { field, value } => format!("until {field} = {}", tag_lit(*value)),
            Until::FieldBytes { field, bytes } => match std::str::from_utf8(bytes) {
                Ok(text) if text.chars().all(|c| c.is_ascii_graphic()) => format!("until {field} = '{text}'"),
                _ => format!("until {field} matches"),
            },
            Until::Cond(e) => write_expr(e).map_or(String::new(), |s| format!("until {s}")),
            // Read before each element rather than after it, and the word
            // says which: the run carries on while this holds.
            Until::While(e) => write_expr(e).map_or(String::new(), |s| format!("while {s}")),
        },
        _ => String::new(),
    }
}

/// The address a field reads its contents at, for a field that reads them
/// somewhere else. Empty for every field that sits where it is declared.
fn at_text(ty: &Ty) -> Option<String> {
    match ty {
        Ty::At { at, .. } => write_expr(at),
        Ty::Sized { inner, .. }
        | Ty::SizedBits { inner, .. }
        | Ty::Origin { inner }
        | Ty::Nullable { inner, .. }
        | Ty::When { inner, .. } => at_text(inner),
        _ => None,
    }
}

/// What tells one type definition from another: its name and everything it
/// says.
///
/// Two structures with the same key are the same type however many times the
/// template built one, and are drawn once. Two with different keys are two
/// types however alike they look, and are drawn twice. The name is in the key
/// as well as the body because a format may give two shapes the same fields and
/// different names, and a box is labelled by its name: sharing them would put
/// one name on bytes the format calls something else.
fn struct_key(sd: &Arc<StructDef>) -> String {
    format!("struct\u{0}{}\u{0}{}", sd.name, crate::template_text::ty_text(&Ty::Struct(sd.clone())))
}

/// The same question for a switch, which the IR does not put behind an `Arc`:
/// what it reads and what each case picks.
fn switch_key(sw: &Ty) -> String {
    format!("switch\u{0}{}", crate::template_text::ty_text(sw))
}

/// Which box a type is drawn as, named the way the walk names it, or nothing
/// for a type that is drawn as a row rather than a box.
///
/// The one place the question is answered, because two callers ask it and a
/// second answer would be a second opinion: the walk uses it to decide whether
/// a type it has reached is one it has already drawn, and
/// [`crate::eval::census`] uses it to say which box a node of the open file
/// belongs to. A census keyed even slightly differently would count real
/// fields against boxes that are not there.
///
/// A switch first, for the reason `box_for` takes one first: a switch whose
/// cases are structures is both, and the choice is what the reader has to see.
/// Whether a type is a run of something rather than one of it.
///
/// [`box_key`] answers for a field's *contents*, so a list of chunks answers
/// with the chunk's box: that is the box the field's arrow points at, which is
/// what the drawing wants. A census counting nodes wants the other reading. The
/// list node itself is not a chunk; its elements are, and counting it as one
/// would make every run one longer than the file.
pub(crate) fn is_run(t: &Template, ty: &Ty) -> bool {
    match ty {
        Ty::Array { .. } | Ty::Repeat { .. } | Ty::PointerList { .. } | Ty::Chain { .. } | Ty::Gather { .. } => true,
        Ty::Sized { inner, .. }
        | Ty::SizedBits { inner, .. }
        | Ty::Origin { inner }
        | Ty::At { inner, .. }
        | Ty::Nullable { inner, .. }
        | Ty::Decoded { inner, .. }
        | Ty::When { inner, .. } => is_run(t, inner),
        Ty::Named(n) => t.types.get(&**n).is_some_and(|inner| is_run(t, inner)),
        _ => false,
    }
}

/// Whether a row stands for a list, which is [`is_run`] short of a stream.
///
/// A row is the field as declared, so it is read past what the declaration
/// wraps round the list: a size, an origin, a name, a condition, and an address
/// to read it at, which the row's position column already says. Not past a
/// stream. A compressed field holding a list is the stream, and its type column
/// names the codec; a census counting what the stream holds wants the other
/// answer, which is why this is not `is_run`. A stream joined from parts is not
/// looked into by either.
fn lists(t: &Template, ty: &Ty) -> bool {
    match ty {
        Ty::Decoded { .. } => false,
        Ty::Sized { inner, .. }
        | Ty::SizedBits { inner, .. }
        | Ty::Origin { inner }
        | Ty::At { inner, .. }
        | Ty::Nullable { inner, .. }
        | Ty::When { inner, .. } => lists(t, inner),
        Ty::Named(n) => t.types.get(&**n).is_some_and(|inner| lists(t, inner)),
        other => is_run(t, other),
    }
}

/// Whether a type is a choice, so that what one of it turns out to be is not
/// settled by the declaration alone.
pub(crate) fn is_choice(t: &Template, ty: &Ty) -> bool {
    as_switch(t, ty).is_some()
}

/// A cheap stand-in for [`box_key`], for a caller asking the same question of
/// thousands of nodes.
///
/// The key is the type written out, which costs what writing a type out costs;
/// asked once per type that is nothing, asked once per node of a file it is the
/// whole cost of a census. So this answers "which type is this" by the pointer
/// the IR already shares: a structure by its `Arc`, a switch by the `Arc` its
/// cases live in. Two types with the same identity have the same key, which is
/// what a cache needs; two with different identities may still share a key, and
/// the cache simply computes it twice.
///
/// Nothing for a type that is neither, which is the case a caller has to
/// compute without the cache.
pub(crate) fn box_identity(t: &Template, ty: &Ty) -> Option<usize> {
    if let Some(sw) = as_switch(t, ty) {
        return match sw {
            Ty::Switch { cases, .. } => Some(Arc::as_ptr(cases) as *const u8 as usize),
            Ty::Match { cases, .. } => Some(Arc::as_ptr(cases) as *const u8 as usize),
            _ => None,
        };
    }
    as_struct(t, ty).map(|sd| Arc::as_ptr(sd) as usize)
}

pub(crate) fn box_key(t: &Template, ty: &Ty) -> Option<String> {
    if let Some(sw) = as_switch(t, ty) {
        return Some(switch_key(sw));
    }
    as_struct(t, ty).map(struct_key)
}

/// One expression a row holds, and what it decides about the row.
struct Source {
    expr: Expr,
    role: Role,
    /// True when the expression is worked out inside one element of this row
    /// rather than beside the row: a run's stopping condition reads the
    /// element's own fields, and looking them up among this structure's would
    /// find the wrong field or none.
    inside: bool,
}

/// Every expression a field's declaration holds, with the role each plays.
/// The same list `origin.rs` walks, read off a declared type rather than off a
/// resolved node: this is the template, and nothing here has been resolved.
fn sources(ty: &Ty, out: &mut Vec<Source>, depth: u32) {
    if depth > 32 {
        return;
    }
    let add = |e: &Expr, role: Role, out: &mut Vec<Source>| out.push(Source { expr: e.clone(), role, inside: false });
    match ty {
        Ty::Sized { size, inner } => {
            add(size, Role::Length, out);
            sources(inner, out, depth + 1);
        }
        Ty::SizedBits { bits, inner } => {
            add(bits, Role::Width, out);
            sources(inner, out, depth + 1);
        }
        Ty::At { at, inner, .. } => {
            add(at, Role::Position, out);
            sources(inner, out, depth + 1);
        }
        Ty::Origin { inner } | Ty::Nullable { inner, .. } | Ty::Enum { inner, .. } | Ty::Flags { inner, .. } => {
            sources(inner, out, depth + 1)
        }
        // Whether the field is there at all, which is its own question
        // wherever it is shown: the panel and the origins say `condition` too.
        // See [`Role::Condition`].
        Ty::When { cond, inner } => {
            add(cond, Role::Condition, out);
            sources(inner, out, depth + 1);
        }
        Ty::Switch { on, .. } | Ty::Match { on, .. } => add(on, Role::Type, out),
        Ty::Bytes(e) => add(e, Role::Length, out),
        Ty::Str { len: StrLen::Fixed(e) | StrLen::Padded { size: e, .. }, .. }
        | Ty::TextInt { len: StrLen::Fixed(e) | StrLen::Padded { size: e, .. }, .. } => add(e, Role::Length, out),
        Ty::UIntExpr { bits, .. } => add(bits, Role::Width, out),
        Ty::Computed(e) | Ty::ComputedText(e) | Ty::ComputedReal(e) => add(e, Role::Value, out),
        Ty::Array { count, elem } => {
            add(count, Role::Count, out);
            sources(elem, out, depth + 1);
        }
        // Where the run stops, which is how many elements it has. Read inside
        // the element: `until type = 'IEND'` names the element's `type`, and
        // the arrow belongs on that row of the element's box.
        Ty::Repeat { elem, until } => {
            match until {
                Until::Cond(e) => out.push(Source { expr: e.clone(), role: Role::Count, inside: true }),
                // Asked beside the list, not inside the element: the arrow
                // belongs on the row of a sibling of the list, since that is
                // what the names in it reach. See `Until::While`.
                Until::While(e) => out.push(Source { expr: e.clone(), role: Role::Count, inside: false }),
                Until::FieldValue { field, .. } | Until::FieldBytes { field, .. } => {
                    out.push(Source { expr: Expr::field(field), role: Role::Count, inside: true })
                }
                Until::End => {}
            }
            sources(elem, out, depth + 1);
        }
        Ty::PointerList { offsets, adjust, elem, .. } => {
            add(&Expr::Ref(offsets.clone()), Role::Position, out);
            add(adjust, Role::Position, out);
            sources(elem, out, depth + 1);
        }
        Ty::Chain { first, adjust, elem, .. } => {
            add(first, Role::Position, out);
            add(adjust, Role::Position, out);
            sources(elem, out, depth + 1);
        }
        Ty::Gather { offset, adjust, elem, .. } => {
            add(offset, Role::Position, out);
            add(adjust, Role::Position, out);
            sources(elem, out, depth + 1);
        }
        Ty::Decoded { codec, inner } => {
            match codec {
                Packing::Fixed(_) => {}
                Packing::Lzma1 { props, dict_size, unpacked } => {
                    add(props, Role::Type, out);
                    add(dict_size, Role::Length, out);
                    if let Some(e) = unpacked {
                        add(e, Role::Length, out);
                    }
                }
                Packing::Rar5 { dictionary, unpacked } => {
                    add(dictionary, Role::Length, out);
                    add(unpacked, Role::Length, out);
                }
            }
            sources(inner, out, depth + 1);
        }
        // The two lengths a stitched stream may be given, and what it holds.
        // The walk's steps are names of places rather than expressions, the
        // way a gather's are.
        Ty::Stitched { part_len, len, inner, .. } => {
            if let Some(e) = part_len {
                add(e, Role::Length, out);
            }
            if let Some(e) = len {
                add(e, Role::Length, out);
            }
            sources(inner, out, depth + 1);
        }
        _ => {}
    }
}

/// Every field an expression reads, by name, in the order it reads them.
///
/// The same walk `origin.rs::from_expr` makes, without a file to resolve the
/// names against: what comes back is what the expression says, and whether a
/// row of that name is in scope is settled by the caller.
fn names_in(e: &Expr, out: &mut Vec<String>) {
    match e {
        Expr::Ref(n) | Expr::SizeOf(n) | Expr::BitsOf(n) | Expr::ProductOf(n) | Expr::SumOf(n) | Expr::MaxOf(n)
        | Expr::PopCount(n) | Expr::Prev(n) | Expr::LenOf(n) => out.push(n.to_string()),
        // A path into an earlier field. The field is its first name; the rest
        // are that field's own, and belong to no row of this box.
        Expr::Within(f) | Expr::Sibling(f) => {
            if let Some(first) = f.first() {
                out.push(first.clone());
            }
        }
        Expr::Elem { array, index, .. } | Expr::Product { array, index, .. } => {
            out.push(array.to_string());
            names_in(index, out);
        }
        Expr::ElemWithin { path, index, .. } => {
            if let Some(first) = path.first() {
                out.push(first.clone());
            }
            names_in(index, out);
        }
        // The records of an archive, and what says which entry of them.
        Expr::EntryOf { records, name } => {
            if let Some(first) = records.first() {
                out.push(first.clone());
            }
            names_in(name, out);
        }
        Expr::Tagged(t) => {
            if let Some(array) = &t.array {
                names_in(array, out);
            }
            if let crate::template::Tag::Computed(inner) | crate::template::Tag::ComputedText(inner) = &t.tag {
                names_in(inner, out);
            }
        }
        Expr::Placer(inner)
        | Expr::Log2(inner)
        | Expr::Bit(inner, _)
        | Expr::PadTo { n: inner, .. }
        | Expr::Not(inner)
        | Expr::BitNot(inner)
        | Expr::StartOf(inner)
        | Expr::RealText(inner)
        | Expr::Pow2(inner)
        | Expr::Pow10(inner)
        | Expr::Trunc(inner) => names_in(inner, out),
        Expr::PeekAt { skip, .. } => names_in(skip, out),
        Expr::PeekIn { at, .. } => names_in(at, out),
        Expr::Cond { when, then, otherwise } => {
            names_in(when, out);
            names_in(then, out);
            names_in(otherwise, out);
        }
        Expr::Or(a, b)
        | Expr::Add(a, b)
        | Expr::Sub(a, b)
        | Expr::Mul(a, b)
        | Expr::Div(a, b)
        | Expr::DivCeil(a, b)
        | Expr::Mod(a, b)
        | Expr::Less(a, b)
        | Expr::Eq(a, b)
        | Expr::Ne(a, b)
        | Expr::Le(a, b)
        | Expr::Gt(a, b)
        | Expr::Ge(a, b)
        | Expr::Both(a, b)
        | Expr::Either(a, b)
        | Expr::Shl(a, b)
        | Expr::Shr(a, b)
        | Expr::And(a, b)
        | Expr::BitOr(a, b)
        | Expr::BitXor(a, b)
        | Expr::Min(a, b)
        | Expr::Max(a, b) => {
            names_in(a, out);
            names_in(b, out);
        }
        // What is left names no field: a literal, an index, a peek at bits, a
        // search for a pattern, where the cursor is, how big the window is.
        // `Pos`, `WindowSize`, `SpacePos` and `SpaceSize` are facts about the
        // frame rather than about any row, so there is nothing to draw an
        // arrow from.
        _ => {}
    }
}

struct Walk<'a> {
    t: &'a Template,
    boxes: Vec<TypeBox>,
    /// The structure each box was drawn from, in step with `boxes`. Kept rather
    /// than looked up again: a box named for where it was found (`Outer.field`)
    /// has no entry in the type table, and finding it by walking the name back
    /// down the template is a second answer to a question already answered.
    /// None for a switch box, whose rows are cases rather than fields.
    defs: Vec<Option<StructDef>>,
    /// Box index by what the type *says*, so one type written in nine places is
    /// one box.
    ///
    /// This is what makes the picture the size of the format rather than the
    /// size of the walk. An ID3 tag's `switch on id` names `TextFrame` in
    /// thirteen cases and the template builds a fresh `StructDef` for each, so
    /// a WAV carrying one drew ninety-six identical `TextFrame` boxes and a
    /// hundred and four identical `switch on encoding` boxes beside them: a
    /// column of the same picture over and over, which says nothing thirteen
    /// times.
    ///
    /// Keyed on the structure's name and its rendering by
    /// [`crate::template_text::ty_text`] rather than on the `Arc` it came in.
    /// Pointer identity is the wrong question: it says whether two fields were
    /// handed the same object, and what a reader wants to know is whether they
    /// are the same type. The rendering answers that exactly, and it keeps
    /// apart what should be kept apart — an ELF's four class-and-endianness
    /// section headers print `u32 le` against `u32 be` and stay four boxes.
    ///
    /// Filled in before the box's own fields are walked, so a type holding
    /// itself stops.
    by_text: HashMap<String, usize>,
    /// Box index by the path it was first reached down, for the boxes neither
    /// key reaches.
    by_path: HashMap<String, usize>,
    edges: Vec<DiagramEdge>,
    /// How many of the template's named types got a box.
    named_drawn: u32,
    /// Boxes the cap refused.
    capped: u32,
}

impl<'a> Walk<'a> {
    /// The box for a named type, drawing it if this is the first time it has
    /// been reached. None for a name the template does not define, and for one
    /// that is not a structure or a switch: a named integer is a type column
    /// entry, not a box.
    fn named_box(&mut self, name: &str) -> Option<usize> {
        if let Some(&at) = self.by_path.get(name) {
            return Some(at);
        }
        let ty = self.t.types.get(name)?;
        if let Some(sd) = as_struct(self.t, ty) {
            let sd = sd.clone();
            // Already drawn where it was written inline, or under another of
            // its names. One type, one box, whichever way the walk got here
            // first; the table's name for it is recorded all the same so a
            // second `Named` lookup is answered without another search.
            if let Some(&at) = self.by_text.get(&struct_key(&sd)) {
                self.by_path.insert(name.to_string(), at);
                self.named_drawn += 1;
                return Some(at);
            }
            if self.boxes.len() >= BOX_CAP {
                self.capped += 1;
                return None;
            }
            self.named_drawn += 1;
            return Some(self.struct_box(name.to_string(), None, &sd));
        }
        if let Some(sw) = as_switch(self.t, ty) {
            if self.boxes.len() >= BOX_CAP {
                self.capped += 1;
                return None;
            }
            self.named_drawn += 1;
            let sw = sw.clone();
            let key = switch_key(&sw);
            if let Some(&at) = self.by_text.get(&key) {
                self.by_path.insert(name.to_string(), at);
                return Some(at);
            }
            return Some(self.switch_box(name.to_string(), None, &sw, key));
        }
        None
    }

    /// One structure as a box: a row per field, then the boxes its fields reach.
    ///
    /// `path` is where the walk found it. What the box is *called* is the
    /// structure's own name, which is what the format calls the thing:
    /// `IHDR`, not `png.chunks.data.'IHDR'`. The path stays, on the box, for
    /// the reader who wants to know how they would get there.
    fn struct_box(&mut self, path: String, parent: Option<String>, sd: &Arc<StructDef>) -> usize {
        let here = self.boxes.len();
        let key = struct_key(sd);
        self.by_text.insert(key.clone(), here);
        self.by_path.insert(path.clone(), here);
        let name = if sd.name.is_empty() { path.clone() } else { sd.name.clone() };
        let sd = (**sd).clone();
        self.boxes.push(TypeBox {
            name,
            path: path.clone(),
            key,
            kind: BoxKind::Seq,
            parent,
            rows: Vec::new(),
        });
        self.defs.push(Some(sd.clone()));
        // The running offset, which stops for good at the first field whose
        // width the file rather than the template settles. A position that
        // carried on past one would be a number nothing in any file agrees
        // with.
        let mut at: Option<u64> = Some(0);
        for f in &sd.fields {
            let pos = match at_text(&f.ty) {
                Some(e) => e,
                None => match at {
                    Some(bits) if bits % 8 == 0 => format!("0x{:x}", bits / 8),
                    Some(bits) => format!("0x{:x}+{}", bits / 8, bits % 8),
                    None => String::new(),
                },
            };
            self.boxes[here].rows.push(Row {
                name: f.name.to_string(),
                type_text: f.ty.display_name(),
                size_text: size_text(self.t, &f.ty),
                pos_text: pos,
                list: lists(self.t, &f.ty),
                kind: value_kind(self.t, &f.ty),
            });
            at = match (at, static_bits(self.t, &f.ty, 0)) {
                // Every field of a union is drawn at the record's own start,
                // so the running offset never moves on. See
                // `StructDef::overlap`.
                (Some(a), _) if sd.overlap => Some(a),
                (Some(a), Some(b)) if !f.aside => Some(a + b),
                (Some(a), Some(_)) => Some(a),
                _ => None,
            };
        }
        // The types the fields reach, once every row of this box exists: a
        // field whose type is this very structure has to find the box already
        // there. Named down the path rather than down the title, so two types
        // the format happens to call the same thing do not claim each other's
        // children.
        for (i, f) in sd.fields.iter().enumerate() {
            self.field_target(here, i, &path, &f.name, &f.ty);
        }
        here
    }

    /// Where one field's type edge goes, and what the edge says.
    fn field_target(&mut self, here: usize, row: usize, owner: &str, field: &str, ty: &Ty) {
        let Some((to, label)) = self.box_for(owner, field, ty) else { return };
        self.edges.push(DiagramEdge { from: (here, row), to: (to, None), role: Role::Type, label });
    }

    /// The box a type is drawn as, making it if this is the first time it has
    /// been reached: a switch's own box, a named type's box, or a box made here
    /// for a structure written inline. The label is the expression a switch
    /// reads, and empty for everything else.
    ///
    /// `owner` and `label` name the inline box when one is made, so a type the
    /// template never named is named for where it was found. Nothing for a type
    /// that is not one of the three, which is every number and every run of
    /// bytes: those say what they are in their own type column.
    fn box_for(&mut self, owner: &str, label: &str, ty: &Ty) -> Option<(usize, String)> {
        // A switch first, since a switch whose cases are structures is both and
        // the choice is what the reader has to see: skipping to the structure
        // would draw one case as though it were the only one.
        if let Some(sw) = as_switch(self.t, ty) {
            let sw = sw.clone();
            let name = format!("{owner}.{label}");
            let on = match &sw {
                Ty::Switch { on, .. } | Ty::Match { on, .. } => write_expr(on).unwrap_or_default(),
                _ => String::new(),
            };
            // One choice, one box, however many fields make it. Not scoped to
            // the type it was found in: two fields that read the same value and
            // pick between the same shapes are making one choice, and the box
            // says what that choice is rather than who is making it.
            let key = switch_key(&sw);
            if let Some(&at) = self.by_text.get(&key) {
                return Some((at, on));
            }
            if self.boxes.len() >= BOX_CAP {
                self.capped += 1;
                return None;
            }
            let to = self.switch_box(name, Some(owner.to_string()), &sw, key);
            return Some((to, on));
        }
        if let Some(target) = named_target(ty) {
            return self.named_box(&target).map(|to| (to, String::new()));
        }
        // A structure written where it is used rather than declared in the
        // table. It is still a type and still worth a box; it is named for
        // where it was found, since the template gave it no name of its own.
        let sd = as_struct(self.t, ty)?.clone();
        // The same structure reached a second time is the same box, whether it
        // was reached by another name or from another case.
        if let Some(&at) = self.by_text.get(&struct_key(&sd)) {
            return Some((at, String::new()));
        }
        let name = format!("{owner}.{label}");
        if self.boxes.len() >= BOX_CAP {
            self.capped += 1;
            return None;
        }
        Some((self.struct_box(name, Some(owner.to_string()), &sd), String::new()))
    }

    /// One switch as a box: a row per case, and an edge from each case to the
    /// type it picks.
    fn switch_box(&mut self, name: String, parent: Option<String>, sw: &Ty, key: String) -> usize {
        let here = self.boxes.len();
        self.by_text.insert(key.clone(), here);
        self.by_path.insert(name.clone(), here);
        // A switch has no name of its own in the IR, so it is called what it
        // reads. Not the last step of the path: a switch reached from a case of
        // another switch would then be titled by that case's value, and an ELF
        // would have three boxes called `1`. What the reader wants to know
        // about a choice is what decides it.
        let title = switch_title(sw);
        self.boxes.push(TypeBox {
            name: title.unwrap_or_else(|| name.rsplit_once('.').map_or(name.clone(), |(_, l)| l.to_string())),
            path: name.clone(),
            key,
            kind: BoxKind::Switch,
            parent,
            rows: Vec::new(),
        });
        self.defs.push(None);
        let cases: Vec<(String, Ty)> = match sw {
            Ty::Switch { cases, default, .. } => cases
                .iter()
                .map(|(k, ty)| (tag_lit(*k), ty.clone()))
                .chain(std::iter::once(("_".to_string(), (**default).clone())))
                .collect(),
            // Quoted, so a case a format keys on the word `F32` is not read as
            // a number, and so an empty key is visible as one.
            Ty::Match { cases, default, .. } => cases
                .iter()
                .map(|(k, ty)| (format!("{k:?}"), ty.clone()))
                .chain(std::iter::once(("_".to_string(), (**default).clone())))
                .collect(),
            _ => Vec::new(),
        };
        for (key, ty) in &cases {
            self.boxes[here].rows.push(Row {
                // A case that picks another switch says which question that
                // one asks, in the words its own box is titled with. Bare
                // `switch` in the type column would leave an ELF's four cases
                // saying the same word four times.
                type_text: switch_title(ty).unwrap_or_else(|| ty.display_name()),
                name: key.clone(),
                size_text: size_text(self.t, ty),
                pos_text: String::new(),
                list: lists(self.t, ty),
                kind: value_kind(self.t, ty),
            });
        }
        for (i, (key, ty)) in cases.iter().enumerate() {
            // A case that picks a type with a box of its own fans out to it;
            // one that picks a number or a run of bytes says so in its own
            // type column and reaches nothing. A case that is itself a switch
            // gets its own box too: an ELF picks a width and then picks an
            // endianness, and stopping at the first choice would leave the
            // whole of the format behind one unopened row.
            if let Some((to, label)) = self.box_for(&name, key, ty) {
                self.edges.push(DiagramEdge { from: (here, i), to: (to, None), role: Role::Case, label });
            }
        }
        here
    }

    /// The dependency edges: for every expression in every row, an edge from
    /// the row of each field it names to the row that reads it.
    ///
    /// Drawn the way the reader follows it, which is the way `graph.rs` draws
    /// the same connection: from the field that decided to the field it decided
    /// about. A name no row in scope answers produces nothing.
    fn link(&mut self) {
        // The declared types again, box by box, since the rows kept only what
        // is shown.
        let mut owed: Vec<(usize, usize, Vec<Source>)> = Vec::new();
        for (bi, b) in self.boxes.iter().enumerate() {
            let Some(sd) = self.defs[bi].as_ref() else { continue };
            for (ri, f) in sd.fields.iter().enumerate() {
                if ri >= b.rows.len() {
                    break;
                }
                let mut found = Vec::new();
                sources(&f.ty, &mut found, 0);
                if let Some(e) = &f.name_from {
                    found.push(Source { expr: e.clone(), role: Role::Name, inside: false });
                }
                if let Some(e) = &f.elem_name_from {
                    found.push(Source { expr: e.clone(), role: Role::Name, inside: false });
                }
                if !found.is_empty() {
                    owed.push((bi, ri, found));
                }
            }
        }
        for (bi, ri, found) in owed {
            for s in found {
                let Some(label) = write_expr(&s.expr) else { continue };
                let mut names = Vec::new();
                names_in(&s.expr, &mut names);
                for name in names {
                    // A run's stopping condition is worked out inside the
                    // element, so its names are the element's fields and not
                    // this structure's. Resolved from anywhere in the element's
                    // box, which then climbs out to this one exactly as a name
                    // read there would.
                    let from = match s.inside {
                        true => self.elem_box(bi, ri).and_then(|eb| self.find_row(eb, usize::MAX, &name)),
                        false => self.find_row(bi, ri, &name),
                    };
                    let Some((fb, fr)) = from else { continue };
                    self.edges.push(DiagramEdge {
                        from: (fb, fr),
                        to: (bi, Some(ri)),
                        role: s.role,
                        label: label.clone(),
                    });
                }
            }
        }
        self.edges.sort_by_key(|e| (e.from, e.to, e.role.as_str(), e.label.clone()));
        self.edges.dedup();
    }

    /// The box of what one row's elements are, for a row that is a list. That
    /// is the box its own `Type` edge reaches, which is already drawn by the
    /// time the dependency edges are worked out.
    fn elem_box(&self, from_box: usize, from_row: usize) -> Option<usize> {
        self.edges
            .iter()
            .find(|e| e.from == (from_box, from_row) && e.role == Role::Type && e.to.1.is_none())
            .map(|e| e.to.0)
    }

    /// The row a name means, seen from one row of one box: an earlier field of
    /// the same type first, then the types this one was declared inside.
    ///
    /// The same order `Expr::Ref` resolves in. A later field of the same box is
    /// taken as well, because a template may read a field declared after this
    /// one from inside a list, and an edge saying so is worth more than none.
    fn find_row(&self, from_box: usize, from_row: usize, name: &str) -> Option<(usize, usize)> {
        let mut at = Some(from_box);
        let mut row_cap = from_row;
        let mut hops = 0;
        while let Some(b) = at {
            if hops > 16 {
                break;
            }
            let boxed = &self.boxes[b];
            // Earlier in this type, which is where the format wrote it.
            if let Some(i) = boxed.rows[..row_cap.min(boxed.rows.len())].iter().rposition(|r| r.name == name) {
                return Some((b, i));
            }
            // Then anywhere in it: a list's element may read a field of its
            // container declared after the list.
            if hops > 0 {
                if let Some(i) = boxed.rows.iter().position(|r| r.name == name) {
                    return Some((b, i));
                }
            }
            at = boxed.parent.as_ref().and_then(|p| self.by_path.get(p)).copied();
            row_cap = usize::MAX;
            hops += 1;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::{Endian, Expr as E, Ty as T};

    fn strukt(name: &str, fields: Vec<(&str, T)>) -> T {
        T::structure(name, fields)
    }

    #[test]
    fn a_length_draws_an_arrow_to_the_run_it_sizes() {
        let root = strukt("root", vec![("len", T::u32(Endian::Big)), ("data", T::bytes(E::field("len")))]);
        let d = diagram(&Template::new("test", root));
        assert_eq!(d.types.len(), 1);
        assert_eq!(d.types[0].rows.len(), 2);
        assert_eq!(d.types[0].rows[0].pos_text, "0x0");
        assert_eq!(d.types[0].rows[0].size_text, "4 bytes");
        assert_eq!(d.types[0].rows[0].kind, "uint");
        assert_eq!(d.types[0].rows[1].pos_text, "0x4");
        assert_eq!(d.types[0].rows[1].size_text, "len bytes");
        assert_eq!(d.edges.len(), 1);
        let e = &d.edges[0];
        assert_eq!((e.from, e.to, e.role), ((0, 0), (0, Some(1)), Role::Length));
        // The label is what the relations panel writes for the same expression.
        assert_eq!(e.label, "len");
    }

    #[test]
    fn a_type_that_holds_itself_is_one_box_with_an_arrow_back_to_it() {
        let node = strukt("node", vec![("tag", T::u8()), ("child", T::Named("node".into()))]);
        let t = Template::new("tree", T::Named("node".into())).with_type("node", node);
        let d = diagram(&t);
        assert_eq!(d.types.len(), 1, "{:?}", d.types);
        assert_eq!(d.types[0].name, "node");
        assert_eq!(d.edges.len(), 1);
        assert_eq!((d.edges[0].from, d.edges[0].to, d.edges[0].role), ((0, 1), (0, None), Role::Type));
        assert_eq!(d.omitted, 0);
    }

    #[test]
    fn a_switch_is_a_box_of_its_cases() {
        let body = strukt("body", vec![("x", T::u16(Endian::Little))]);
        let root = strukt(
            "root",
            vec![
                ("kind", T::u8()),
                (
                    "payload",
                    T::Switch {
                        on: E::field("kind"),
                        cases: vec![(1i128, T::Named("body".into()))].into(),
                        default: std::sync::Arc::new(T::bytes(E::Remaining)),
                    },
                ),
            ],
        );
        let t = Template::new("test", root).with_type("body", body);
        let d = diagram(&t);
        let switch = d.types.iter().position(|b| b.kind == BoxKind::Switch).expect("a switch box");
        assert_eq!(d.types[switch].name, "switch on kind");
        assert_eq!(d.types[switch].path, "test.payload");
        assert_eq!(d.types[switch].parent.as_deref(), Some("test"));
        assert_eq!(d.types[switch].rows.len(), 2);
        assert_eq!(d.types[switch].rows[0].name, "1");
        assert_eq!(d.types[switch].rows[1].name, "_");
        // The field names the switch, the switch's case names the type, and
        // the value the switch reads names the field.
        let body_box = d.types.iter().position(|b| b.name == "body").expect("a body box");
        assert!(d.edges.iter().any(|e| e.to == (switch, None) && e.role == Role::Type && e.label == "kind"));
        assert!(d.edges.iter().any(|e| e.from == (switch, 0) && e.to == (body_box, None) && e.role == Role::Case));
        assert!(d.edges.iter().any(|e| e.to == (0, Some(1)) && e.role == Role::Type && e.label == "kind"));
    }

    /// A case whose type is another switch says which question that one asks,
    /// in the words its own box is titled with. `switch` on its own would be
    /// the same word in every such row, and the reader could not tell an ELF's
    /// choice of width from its choice of endianness.
    #[test]
    fn a_case_that_picks_another_switch_says_what_that_one_reads() {
        let inner = T::Switch {
            on: E::field("endian"),
            cases: vec![(1i128, T::u16(Endian::Little))].into(),
            default: std::sync::Arc::new(T::bytes(E::Remaining)),
        };
        let root = strukt(
            "root",
            vec![
                ("width", T::u8()),
                ("endian", T::u8()),
                (
                    "rest",
                    T::Switch {
                        on: E::field("width"),
                        cases: vec![(1i128, inner)].into(),
                        default: std::sync::Arc::new(T::bytes(E::Remaining)),
                    },
                ),
            ],
        );
        let d = diagram(&Template::new("test", root));
        let outer = d.types.iter().position(|b| b.name == "switch on width").expect("the outer switch");
        assert_eq!(d.types[outer].rows[0].type_text, "switch on endian");
        // And the box that case reaches is titled the same way, so the row and
        // the box it points at say one thing.
        assert!(d.types.iter().any(|b| b.name == "switch on endian"));
    }

    #[test]
    fn a_structure_written_inside_a_field_is_named_for_where_it_was_found() {
        let inner = strukt("inner", vec![("a", T::u8()), ("b", T::u8())]);
        let root = strukt("root", vec![("count", T::u8()), ("items", T::Array { elem: Box::new(inner), count: E::field("count") })]);
        let d = diagram(&Template::new("test", root));
        assert_eq!(d.types.len(), 2);
        assert_eq!(d.types[1].name, "inner");
        assert_eq!(d.types[1].path, "test.items");
        assert_eq!(d.types[1].parent.as_deref(), Some("test"));
        // The count is a field of the container and the element is a box of its
        // own, so both edges exist and they run in different directions.
        assert!(d.edges.iter().any(|e| e.from == (0, 0) && e.to == (0, Some(1)) && e.role == Role::Count));
        assert!(d.edges.iter().any(|e| e.from == (0, 1) && e.to == (1, None) && e.role == Role::Type));
    }

    #[test]
    fn an_element_reaching_its_containers_field_finds_it() {
        let inner = strukt("inner", vec![("body", T::bytes(E::field("width")))]);
        let root = strukt(
            "root",
            vec![("width", T::u8()), ("rows", T::Array { elem: Box::new(inner), count: E::lit(4) })],
        );
        let d = diagram(&Template::new("test", root));
        let elem = d.types.iter().position(|b| b.path == "test.rows").expect("an element box");
        assert!(d.edges.iter().any(|e| e.from == (0, 0) && e.to == (elem, Some(0)) && e.role == Role::Length));
    }

    #[test]
    fn the_same_template_draws_the_same_diagram_twice() {
        for name in crate::formats::builtin_names() {
            let Some(t) = crate::formats::builtin(name) else { continue };
            let a = diagram(&t);
            let b = diagram(&t);
            assert_eq!(a, b, "{name} drew two different diagrams");
        }
    }

    #[test]
    fn a_signature_case_reads_as_the_signature_it_is() {
        // Through the same function the IR text uses, so a case is spelled one
        // way across the whole interface: the letters where a tag spells
        // something, hex where it does not.
        let png = strukt(
            "root",
            vec![
                ("tag", T::u32(Endian::Big)),
                (
                    "body",
                    T::Switch {
                        on: E::field("tag"),
                        cases: vec![(0x4948_4452i128, T::u8()), (3i128, T::u8())].into(),
                        default: std::sync::Arc::new(T::bytes(E::Remaining)),
                    },
                ),
            ],
        );
        let d = diagram(&Template::new("test", png));
        let sw = d.types.iter().find(|b| b.kind == BoxKind::Switch).expect("a switch box");
        assert_eq!(sw.rows[0].name, "'IHDR'");
        assert_eq!(sw.rows[1].name, "3");
    }

    #[test]
    fn one_type_read_in_two_places_is_one_box() {
        let record = strukt("Record", vec![("a", T::u8())]);
        let root = strukt("root", vec![("head", record.clone()), ("tail", record)]);
        let d = diagram(&Template::new("test", root));
        // Two fields, one type, one box: `Ty::Struct` holds an `Arc`, and
        // cloning the type shares it.
        assert_eq!(d.types.len(), 2, "{:?}", d.types.iter().map(|b| &b.name).collect::<Vec<_>>());
        assert_eq!(d.types[1].name, "Record");
        let to_record: Vec<_> = d.edges.iter().filter(|e| e.to == (1, None) && e.role == Role::Type).collect();
        assert_eq!(to_record.len(), 2, "both fields point at it");
    }

    #[test]
    fn a_box_is_titled_by_the_structures_own_name_and_keeps_the_path() {
        let inner = strukt("IHDR", vec![("width", T::u32(Endian::Big))]);
        let root = strukt("png", vec![("head", inner)]);
        let d = diagram(&Template::new("test", root));
        assert_eq!(d.types[1].name, "IHDR");
        assert_eq!(d.types[1].path, "test.head");
    }

    #[test]
    fn an_optional_field_says_so_and_names_what_decides_it() {
        let root = strukt(
            "root",
            vec![("flags", T::u8()), ("extra", T::when(E::field("flags").and(E::lit(1)), T::u16(Endian::Little)))],
        );
        let d = diagram(&Template::new("test", root));
        assert_eq!(d.types[0].rows[1].type_text, "optional u16 le");
        let e = d.edges.iter().find(|e| e.role == Role::Condition).expect("a condition edge");
        assert_eq!((e.from, e.to), ((0, 0), (0, Some(1))));
        assert_eq!(e.label, "flags & 1");
    }

    #[test]
    fn a_run_that_stops_at_a_value_points_from_the_field_it_reads() {
        let elem = strukt("Chunk", vec![("kind", T::u8()), ("body", T::u8())]);
        let root = strukt(
            "root",
            vec![("chunks", T::Repeat { elem: Box::new(elem), until: Until::FieldValue { field: "kind".into(), value: 0 } })],
        );
        let d = diagram(&Template::new("test", root));
        let chunk = d.types.iter().position(|b| b.name == "Chunk").expect("an element box");
        // The field the run stops on is the element's, not the container's, so
        // the arrow leaves the element's box.
        assert!(d.edges.iter().any(|e| e.from == (chunk, 0) && e.to == (0, Some(0)) && e.role == Role::Count));
    }

    #[test]
    fn a_type_written_out_once_per_case_is_drawn_once() {
        // ID3 builds a fresh `StructDef` for each of the thirteen cases that
        // name a text frame, so nothing about the objects says they are one
        // type. What they say does.
        let Some(t) = crate::formats::builtin("id3") else { return };
        let d = diagram(&t);
        let frames: Vec<_> = d.types.iter().filter(|b| b.name == "TextFrame").collect();
        assert_eq!(frames.len(), 1, "one text frame, not {}", frames.len());
        // And the choice inside it, which used to be drawn once per copy.
        let under = frames[0].path.clone();
        let inside: Vec<_> =
            d.types.iter().filter(|b| b.kind == BoxKind::Switch && b.parent.as_deref() == Some(&under)).collect();
        assert_eq!(inside.len(), 1, "one choice inside it, not {}", inside.len());
    }

    #[test]
    fn an_elf_keeps_the_headers_it_reads_two_ways_apart() {
        // The opposite case, and the one sharing must not break: an ELF's four
        // section headers are one shape read at two widths and two byte
        // orders, and `u32 le` is not `u32 be`. Four boxes, and their rows say
        // why.
        let Some(t) = crate::formats::builtin("elf") else { return };
        let d = diagram(&t);
        let heads: Vec<_> = d.types.iter().filter(|b| b.name == "SectionHeader").collect();
        assert!(heads.len() > 1, "the endianness variants were merged into {}", heads.len());
        let spellings: std::collections::HashSet<String> =
            heads.iter().map(|b| b.rows.iter().map(|r| r.type_text.clone()).collect::<Vec<_>>().join(",")).collect();
        assert_eq!(spellings.len(), heads.len(), "two of them say the same thing and should have been shared");
    }

    /// A row says whether its field holds a list, past what the declaration
    /// wraps round it, and without reading the type column, which writes a
    /// placed list with an arrow and a run of bytes with brackets.
    #[test]
    fn a_row_says_whether_its_field_holds_a_list() {
        let root = strukt(
            "root",
            vec![
                ("n", T::u8()),
                ("nums", T::array(T::u8(), E::field("n"))),
                ("rest", T::repeat(T::u8(), crate::template::Until::End)),
                ("far", T::at(E::lit(0), T::array(T::u8(), E::lit(2)))),
                ("named", T::Named("Nums".into())),
                ("maybe", T::when(E::field("n"), T::array(T::u8(), E::lit(2)))),
                ("recs", T::chain(E::field("n"), &["n"], crate::template::Anchor::File, strukt("rec", vec![("n", T::u8())]))),
                ("data", T::bytes(E::field("n"))),
                ("packed", T::decoded(E::lit(4), crate::codec::Codec::Zlib, T::array(T::u8(), E::lit(2)))),
                ("one", strukt("one", vec![("a", T::u8())])),
            ],
        );
        let t = Template::new("test", root).with_type("Nums", T::array(T::u8(), E::lit(3)));
        let d = diagram(&t);
        let rows: Vec<(&str, bool)> = d.types[0].rows.iter().map(|r| (r.name.as_str(), r.list)).collect();
        assert_eq!(
            rows,
            vec![
                ("n", false),
                ("nums", true),
                ("rest", true),
                ("far", true),
                ("named", true),
                ("maybe", true),
                ("recs", true),
                ("data", false),
                ("packed", false),
                ("one", false),
            ]
        );
        assert_eq!(d.types[0].rows[7].type_text, "bytes[]");
    }

    #[test]
    fn every_builtin_draws_something() {
        for name in crate::formats::builtin_names() {
            let Some(t) = crate::formats::builtin(name) else { continue };
            let d = diagram(&t);
            // Every edge lands on a row that exists: a view indexes straight
            // into these, and an index past the end is a crash in the browser
            // rather than a missing arrow.
            for e in &d.edges {
                let from = d.types.get(e.from.0).unwrap_or_else(|| panic!("{name}: edge from a box that is not there"));
                assert!(e.from.1 < from.rows.len(), "{name}: edge from row {} of {}", e.from.1, from.name);
                let to = d.types.get(e.to.0).unwrap_or_else(|| panic!("{name}: edge to a box that is not there"));
                if let Some(r) = e.to.1 {
                    assert!(r < to.rows.len(), "{name}: edge to row {r} of {}", to.name);
                }
            }
        }
    }
}
