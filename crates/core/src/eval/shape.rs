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
    /// The type says so, and the type is written beside the length already: a
    /// `u32` is four bytes because it is a `u32`. Kept apart from `Fixed`
    /// because there is nothing to tell a reader here. They can see it.
    Type,
    /// A number the format fixes, which the type does not carry: a MAT-file's
    /// description is 116 bytes of text, and `ascii[]` says nothing about 116.
    /// Nothing in the file can change it, and that is worth saying, because a
    /// reader looking at 116 bytes of text has no way of knowing whether the
    /// file chose that or the format did.
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
            Sizing::Type => "type",
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
        // Measured, not only typed: a node a decoder laid out covers bits its
        // type says nothing about, and answering from the type alone would
        // print "no bytes of its own" beside a length of five bits. The panel
        // asking this has already paid for the measurement.
        let size = self.size_of(doc, path).unwrap_or(0);
        Ok(Shape { placed: self.placed(path), sized: self.sizing(path, size) })
    }

    /// Which of the ways a format has of saying where something goes put this
    /// field where it is.
    ///
    /// A fact about the thing it sits in: what places a field is whatever
    /// holds it. A field declared at an address is the one that looks like an
    /// exception and is not. The field itself sits where it was declared and
    /// covers no bytes there; what the address places is the one thing it
    /// holds, and that is the node this answers `Address` for.
    fn placed(&self, path: &[usize]) -> Placed {
        let Some((&idx, parent)) = path.split_last() else { return Placed::Root };
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
    fn sizing(&self, path: &[usize], size: u64) -> Sizing {
        let Some(r) = self.memo.get(path) else { return Sizing::Unknown };
        // Asked before `fixed_bits`, which answers `Some(0)` for these: they
        // are fixed at no bits, which is true and is not what a reader wants
        // to be told about a field that is a place or a computation.
        //
        // Unless it covers bits after all, which happens where a decoder laid
        // the node out: a deflate block's `hlit` is written as a computation
        // and is five bits of the compressed stream, because the trace says
        // where it starts and ends.
        if matches!(r.ty, Ty::At { .. } | Ty::Chain { .. } | Ty::Computed(_) | Ty::ComputedText(_)) {
            return if size == 0 { Sizing::Nothing } else { Sizing::Trace };
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
            return if width_in_name(&r.ty) { Sizing::Type } else { Sizing::Fixed };
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

/// Whether the width is already written in the name of the type.
///
/// Two fields can both be fixed and want different things said about them. A
/// `u64 le` is eight bytes, and the panel prints `u64 le` a line above the
/// eight, so a clause explaining that the 64 means 64 is a line the reader has
/// to read to learn nothing. A MAT-file's description is 116 bytes of `ascii[]`
/// and nothing on screen says where 116 came from, so a reader has no way to
/// tell whether the file chose it or the format did.
///
/// The split is exactly that: a number, a float or a magic carries its width in
/// its own name, and anything whose length is a constant written beside it does
/// not. An enum or a flags field is as wide as the number under it, so it
/// follows the number.
fn width_in_name(ty: &Ty) -> bool {
    match ty {
        Ty::UInt { .. }
        | Ty::Int { .. }
        | Ty::SignMagnitude { .. }
        | Ty::F16(_)
        | Ty::BF16(_)
        | Ty::F8 { .. }
        | Ty::F32(_)
        | Ty::F64(_)
        | Ty::F80(_)
        | Ty::Fixed { .. }
        | Ty::Magic(_) => true,
        Ty::Enum { inner, .. } | Ty::Flags { inner, .. } | Ty::Nullable { inner, .. } => width_in_name(inner),
        _ => false,
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
