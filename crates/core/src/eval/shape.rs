//! How a field came to be where it is, and how long it turned out to be: one
//! word for each question.
//!
//! `origin.rs` answers with fields: which other field settled this one's
//! length, its count, its type or its place. That is the whole answer for the
//! fields a format works out, and no answer at all for the fields it does not,
//! which is most of them. A reader standing on a `u32` in a header gets an
//! empty list back, and an empty list does not say "the template says so": it
//! says nothing, which reads as a panel that failed rather than as a field with
//! a plain answer.
//!
//! So this answers the same two questions for every field, whether or not
//! another field is involved. Where does it start, and how long is it. The
//! answers are deliberately coarse: they are what a panel can print before the
//! reader has clicked anything, and what a reader wants from that line is which
//! of a handful of ways the format works. The field is where the one before it
//! ended, or where a number in the header said, or wherever the decoder put it.
//! Which *field* said so is `origins`, and the panel shows the two together.
//!
//! Nothing here reads bytes that placing and sizing the field has not already
//! read. Both answers come from the memo and from the template: the type the
//! field resolved to, the type its declaration gave, and the type of the thing
//! it sits inside. That matters because this is asked on every move of the
//! cursor.
//!
//! [`Placed::Unknown`] and [`Sizing::Unknown`] are real answers and are meant
//! to be given. A word invented for a shape this does not recognise would be a
//! guess printed in the same place, in the same voice, as a fact; the panel
//! prints nothing for Unknown instead.

use super::*;

/// How a field's start was settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placed {
    /// The whole file. It starts where the file starts, and nothing decided
    /// that.
    Root,
    /// The first field of what it sits in, so it starts where that does.
    First,
    /// After the field before it. The commonest answer in any format.
    Follows,
    /// One element of a run, at its own index.
    Element,
    /// Where an offset in a table of them put it. The offset is an origin of
    /// role [`Role::Position`](super::origin::Role::Position).
    Pointer,
    /// Where the element before it said the next one would be.
    Chain,
    /// At an address the file gave, which a header pointing at a table is. The
    /// expression is an origin of role
    /// [`Role::Position`](super::origin::Role::Position), so the panel can name
    /// the field the address was read from.
    Address,
    /// Wherever the decoder that produced it read it from. A symbol of a
    /// deflate stream is not at an offset anything wrote down; it is where the
    /// bit reader had got to.
    Trace,
    /// The front of what a compressed run unpacked to.
    Stream,
    /// None of the above. Say nothing rather than guess.
    Unknown,
}

impl Placed {
    /// The word that crosses the boundary. Part of the interface, like
    /// [`super::kind_of`]'s: a view keys wording off these, so they stay put.
    pub fn as_str(self) -> &'static str {
        match self {
            Placed::Root => "root",
            Placed::First => "first",
            Placed::Follows => "follows",
            Placed::Element => "element",
            Placed::Pointer => "pointer",
            Placed::Chain => "chain",
            Placed::Address => "address",
            Placed::Trace => "trace",
            Placed::Stream => "stream",
            Placed::Unknown => "unknown",
        }
    }
}

/// How a field's length was settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sizing {
    /// The type says so and nothing in the file can change it: a `u32` is four
    /// bytes, a magic is as long as the bytes it matches, a window declared as
    /// a number of bytes is that many.
    Fixed,
    /// Worked out, from an expression that reads the file. Which fields it
    /// reads are the `Length` origins, and the arithmetic is the `Length`
    /// relation.
    Expression,
    /// It ends where a terminator does: the byte that ends a C string, or the
    /// element a run was told to stop after.
    Terminated,
    /// It fills what is left of whatever it sits in.
    Remaining,
    /// As long as the fields inside it come to.
    Children,
    /// A list of places rather than a stretch of bytes: its elements are
    /// wherever the offsets it read said, and what it covers is what is left
    /// of the thing it was declared in, because that is as far as they can
    /// reach. Told apart from `Remaining` because a reader who is told a
    /// pointer list is "the rest of the header" will go looking in the header
    /// for it.
    Scattered,
    /// As many elements as the count says, each as long as its type.
    Count,
    /// Its own bytes say where it ends: a variable-length integer that marks
    /// its last byte, an instruction the disassembler measured.
    Encoded,
    /// As much as the decoder read to produce it.
    Trace,
    /// No bytes of its own. A field worked out rather than read, and a field
    /// that is a place rather than a thing: what covers bytes is what it points
    /// at.
    Nothing,
    /// None of the above. Say nothing rather than guess.
    Unknown,
}

impl Sizing {
    pub fn as_str(self) -> &'static str {
        match self {
            Sizing::Fixed => "fixed",
            Sizing::Expression => "expression",
            Sizing::Terminated => "terminated",
            Sizing::Remaining => "remaining",
            Sizing::Children => "children",
            Sizing::Scattered => "scattered",
            Sizing::Count => "count",
            Sizing::Encoded => "encoded",
            Sizing::Trace => "trace",
            Sizing::Nothing => "nothing",
            Sizing::Unknown => "unknown",
        }
    }
}

/// Both answers about one field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shape {
    pub placed: Placed,
    pub sized: Sizing,
}

impl Evaluator {
    /// How the field at `path` was placed and how it was sized.
    ///
    /// Cheap on purpose: the field has to be resolved, which the panel asking
    /// this has already paid for, and everything after that is a look at two
    /// types.
    pub fn shape<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Shape> {
        self.resolve(doc, path)?;
        Ok(Shape { placed: self.placed(path), sized: self.sizing(path) })
    }

    /// Which of the ways a format has of saying where something goes put this
    /// field where it is.
    ///
    /// The field's own declaration is asked before the thing it sits in,
    /// because a field declared at an address is at that address whatever the
    /// structure around it does. Everything else is a fact about the parent:
    /// what places a field is whatever holds it.
    fn placed(&self, path: &[usize]) -> Placed {
        let Some((&idx, parent)) = path.split_last() else { return Placed::Root };
        if matches!(self.settled_declaration(path), Some(Ty::At { .. })) {
            return Placed::Address;
        }
        let Some(pr) = self.memo.get(parent) else { return Placed::Unknown };
        match &pr.ty {
            Ty::PointerList { .. } => Placed::Pointer,
            Ty::Chain { .. } => Placed::Chain,
            // The one thing an `At` holds is what its address points at, and it
            // carries the field's own name: this is the node the cursor lands
            // on, since the `At` itself covers no bytes.
            Ty::At { .. } => Placed::Address,
            Ty::Traced { .. } => Placed::Trace,
            // What a stream holds starts at the front of the unpacked bytes.
            // Anything else under it is the trace of the decoding, which is
            // read off the compressed bits rather than placed in them.
            Ty::Decoded { .. } => {
                if idx == 0 {
                    Placed::Stream
                } else {
                    Placed::Trace
                }
            }
            Ty::Array { .. } | Ty::Repeat { .. } => Placed::Element,
            // A member of a JSON object is placed by the parse, but it is
            // placed after the member before it, which is what the reader is
            // being told.
            Ty::Struct(_) | Ty::Json(..) => {
                if idx == 0 {
                    Placed::First
                } else {
                    Placed::Follows
                }
            }
            _ => Placed::Unknown,
        }
    }

    /// Which of the ways a format has of saying how long something is settled
    /// this field's length.
    ///
    /// The order is [`Evaluator::size_of`]'s, and has to be: a window around a
    /// field settles its length before the field's own type gets to measure
    /// itself, so asking the type first would answer about a measurement that
    /// never happened.
    fn sizing(&self, path: &[usize]) -> Sizing {
        let Some(r) = self.memo.get(path) else { return Sizing::Unknown };
        // Asked before `fixed_bits`, which answers `Some(0)` for these: they
        // are fixed at no bits, which is true and is not what a reader wants
        // to be told about a field that is a place or a computation.
        if matches!(r.ty, Ty::At { .. } | Ty::Chain { .. } | Ty::Computed(_) | Ty::ComputedText(_)) {
            return Sizing::Nothing;
        }
        // A window around the field settles its length before the field's own
        // type gets to measure itself. How that window's size was arrived at
        // is worked out where the size is: by the time there is a node to ask,
        // the declaration has been walked past, and a `Sized` inside a case of
        // a switch cannot be found again from here.
        if r.declared_size.is_some() {
            return r.sized_how.unwrap_or(Sizing::Unknown);
        }
        if fixed_bits(&r.ty).is_some() {
            return Sizing::Fixed;
        }
        // A wrapper that names values or marks one as unset is as long as what
        // it wraps, which is how `read_size` measures it too.
        let mut ty = &r.ty;
        for _ in 0..64 {
            match ty {
                Ty::Enum { inner, .. } | Ty::Flags { inner, .. } | Ty::Nullable { inner, .. } => ty = inner,
                _ => break,
            }
        }
        match ty {
            Ty::Struct(_) => Sizing::Children,
            Ty::Json(shape, _) if shape.composite() => Sizing::Children,
            // A JSON scalar is as long as the text the parse gave it.
            Ty::Json(..) => Sizing::Encoded,
            Ty::Array { .. } => Sizing::Count,
            Ty::Repeat { until: Until::End, .. } => Sizing::Remaining,
            // A run told to stop after the element that says so: the same
            // answer as a terminated string, one element up.
            Ty::Repeat { .. } => Sizing::Terminated,
            Ty::Bytes(e) => expr_sizing(e),
            Ty::Str { len, .. } | Ty::TextInt { len, .. } => match len {
                StrLen::Fixed(e) | StrLen::Padded { size: e, .. } => expr_sizing(e),
                StrLen::Scan { .. } | StrLen::Terminated { .. } => Sizing::Terminated,
            },
            Ty::UIntExpr { bits, .. } => expr_sizing(bits),
            Ty::Leb128 { .. } | Ty::Zigzag | Ty::Vlq | Ty::SqliteVarint | Ty::EbmlVint { .. } | Ty::Insn { .. } => Sizing::Encoded,
            Ty::Traced { .. } => Sizing::Trace,
            Ty::PointerList { .. } => Sizing::Scattered,
            _ => Sizing::Unknown,
        }
    }

    /// What a field was declared as, with the names looked up. Used to tell a
    /// field placed at an address from one placed after the field before it,
    /// which the resolved type cannot answer: resolving an `At` hands back what
    /// it points at.
    fn settled_declaration(&self, path: &[usize]) -> Option<Ty> {
        let mut ty = self.declared_ty(path).ok()?;
        for _ in 0..64 {
            match ty {
                Ty::Named(n) => ty = self.template.types.get(&*n)?.clone(),
                Ty::Origin { inner } => ty = *inner,
                other => return Some(other),
            }
        }
        None
    }
}

/// What a length expression says about how the length was settled: it fills
/// the room left, it is worked out from the file, or it is a number the
/// template wrote down.
pub(super) fn expr_sizing(e: &Expr) -> Sizing {
    let (remaining, read) = expr_reads(e);
    if remaining {
        Sizing::Remaining
    } else if read {
        Sizing::Expression
    } else {
        Sizing::Fixed
    }
}

/// Whether an expression measures the room left, and whether any part of it is
/// worked out rather than written down. Both are needed at once: `remaining -
/// 4` is a field that fills what is left, and reporting it as an expression
/// would send a reader looking for a length field that does not exist.
fn expr_reads(e: &Expr) -> (bool, bool) {
    let two = |a: &Expr, b: &Expr| {
        let (ra, na) = expr_reads(a);
        let (rb, nb) = expr_reads(b);
        (ra || rb, na || nb)
    };
    match e {
        Expr::Lit(_) => (false, false),
        Expr::Remaining => (true, false),
        Expr::Add(a, b)
        | Expr::Sub(a, b)
        | Expr::Mul(a, b)
        | Expr::Div(a, b)
        | Expr::Or(a, b)
        | Expr::And(a, b)
        | Expr::Less(a, b)
        | Expr::Shl(a, b)
        | Expr::Shr(a, b)
        | Expr::Min(a, b)
        | Expr::Max(a, b) => two(a, b),
        Expr::PadTo { n, .. } => expr_reads(n),
        Expr::Bit(a, _) => expr_reads(a),
        // Everything left reads something: a field, an element of a list, a
        // peek at bytes ahead, a number the decoder deduced.
        _ => (false, true),
    }
}
