//! The relationship behind a field's shape, written out.
//!
//! `origin.rs` answers which other fields decided a field's length, count or
//! type, and that is a list of names. It does not say what was done with them.
//! A reader looking at a run of 3,824 bytes and a row saying it depends on
//! `cell_content_start` still has to work out the arithmetic in their head.
//!
//! So this writes the expression the template holds, twice: as the template
//! writes it, and with each field's value put in its place. The quantised
//! weights panel already does exactly this for one formula, `d * scale *
//! stored - dmin * min`; this is the same move for every length and count in
//! the IR.
//!
//! The rule it follows is the one the design settled: the core describes the
//! relationship and the UI renders it. Nothing here decides how it is laid
//! out, and nothing in the UI infers a relationship from a field's name.
//!
//! An expression this cannot write out produces nothing rather than a partial
//! reading. Half a formula is worse than none: `x + …` invites the reader to
//! believe the half they can see is the whole story.

use super::origin::Role;
use super::*;

/// One relationship, written both ways.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relation {
    /// What the expression decides about the field.
    pub role: Role,
    /// The expression as the template writes it: `cell_content_start - 100`.
    pub written: String,
    /// The same with every field's value in its place: `3936 - 100`.
    pub substituted: String,
    /// What it comes to.
    pub result: String,
}

/// How tightly an operator binds, so that only the brackets a reader needs are
/// written. Zero is a leaf or a call, which never needs any.
fn prec(e: &Expr) -> u32 {
    match e {
        // A question about a question, so everything in it binds tighter.
        Expr::Cond { .. } => 1,
        // The value-or, which takes its right side only when the left comes to
        // nothing. Loosest of the binary operators, which is where it was
        // before the boolean set arrived beside it.
        Expr::Or(..) => 2,
        Expr::Either(..) => 3,
        Expr::Both(..) => 4,
        Expr::Not(..) => 5,
        // Between the booleans and a comparison, as it is in every language
        // that writes these: `a & b < c` is the comparison of `a & b`, and a
        // mask written beside an addition binds looser than the addition.
        Expr::And(..) => 6,
        Expr::Less(..) | Expr::Eq(..) | Expr::Ne(..) | Expr::Le(..) | Expr::Gt(..) | Expr::Ge(..) => 7,
        Expr::Shl(..) | Expr::Shr(..) => 8,
        Expr::Add(..) | Expr::Sub(..) => 9,
        Expr::Mul(..) | Expr::Div(..) | Expr::Mod(..) => 10,
        _ => 0,
    }
}

/// An outer precedence no operator reaches, so that whatever is written at it
/// is bracketed unless it is a leaf. What `not` writes its operand at: `not a
/// == b` has two readings and only one of them is this one, and a bracket is
/// cheaper than being right for the wrong reason.
const ALWAYS: u32 = u32::MAX;

impl Evaluator {
    /// The relationships behind the field at `path`: what decided its length,
    /// how many children it has, which type it was read as, or what it says.
    /// Empty for a field the template placed and sized outright, and for one
    /// whose expression this cannot write out.
    pub fn relations<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Vec<Relation>> {
        self.resolve(doc, path)?;
        let mut out = Vec::new();
        // The address that put this node here, for the contents of an `At`.
        // Written out against the field that declared it, because that is the
        // frame the address was worked out in: `e_shoff` is a field of the
        // header, and reading it from inside the table it places would find
        // nothing of that name.
        if let Some((&idx, parent)) = path.split_last() {
            match self.memo.get(parent).map(|r| r.ty.clone()) {
                Some(Ty::At { at, .. }) => self.relation(doc, parent, &at, Role::Position, None, &mut out),
                // The same for a gathered element, against the record that
                // placed it: `offset` is a field of a descriptor in some row,
                // and only from there does the name mean anything.
                Some(Ty::Gather { offset, .. }) => {
                    if let Ok(record) = self.gathered_record(doc, parent, idx) {
                        if let Ok((end, frame)) = self.record_frame(doc, &record) {
                            self.relation(doc, &end, &offset, Role::Position, frame, &mut out);
                        }
                    }
                }
                _ => {}
            }
        }
        if let Some(from) = self.name_from(path) {
            self.relation(doc, path, &from, Role::Name, None, &mut out);
        }
        let declared = self.declared_ty(path)?;
        let mut ty = declared;
        for _ in 0..64 {
            match ty {
                Ty::Named(n) => match self.template.types.get(&*n) {
                    Some(t) => ty = t.clone(),
                    None => break,
                },
                // Measured in the room the container had left, which is not
                // the room this node has: resolving a `Sized` narrows the
                // limit to the window the size itself set. Asking here would
                // answer with the window, and write out a length short by
                // whatever the expression takes off the end.
                Ty::Sized { size, inner } => {
                    let room = self.before_window(path);
                    self.relation(doc, path, &size, Role::Length, room, &mut out);
                    ty = *inner;
                }
                // The address is where this field's contents are, not where
                // the field is, so it is written out against the contents. See
                // the `At` parent handled above.
                Ty::At { inner, .. } => ty = *inner,
                Ty::Origin { inner } => ty = *inner,
                Ty::Switch { on, .. } | Ty::Match { on, .. } => {
                    self.relation(doc, path, &on, Role::Type, None, &mut out);
                    break;
                }
                _ => break,
            }
        }
        let base = self.memo[path].ty.without_sentinel().clone();
        match &base {
            Ty::Bytes(e) => self.relation(doc, path, &e.clone(), Role::Length, None, &mut out),
            // How wide the number is, which is the one thing about it the
            // template did not fix. Told apart from a byte length because it
            // is bits and because a reader asking "why is this eleven bits"
            // wants the field that said eleven.
            Ty::UIntExpr { bits, .. } => self.relation(doc, path, &(**bits).clone(), Role::Width, None, &mut out),
            Ty::Str { len: StrLen::Fixed(e) | StrLen::Padded { size: e, .. }, .. } => {
                self.relation(doc, path, &e.clone(), Role::Length, None, &mut out)
            }
            Ty::Array { count, .. } => self.relation(doc, path, &count.clone(), Role::Count, None, &mut out),
            Ty::Computed(e) | Ty::ComputedText(e) => self.relation(doc, path, &e.clone(), Role::Value, None, &mut out),
            _ => {}
        }
        Ok(out)
    }

    /// The frame a `Sized` node's own size expression was worked out in:
    /// where the field starts, and how far its container let it go.
    ///
    /// Resolving a `Sized` narrows the node's limit to the window the size
    /// expression set, so by the time there is a node to ask, the memo no
    /// longer holds the room the expression saw. `Remaining` read from the
    /// narrowed frame answers with the window, and a length written out as
    /// `max(403 - 8, 0) = 395` for a field of 403 bytes is a number nothing
    /// in the file agrees with.
    ///
    /// Nothing for a `Sized` at the root, which has no container to ask; the
    /// caller then falls back to the node's own frame, as every other
    /// relationship does.
    fn before_window(&self, path: &[usize]) -> Option<(u64, u64)> {
        let (_, parent) = path.split_last()?;
        Some((self.memo.get(path)?.offset, self.memo.get(parent)?.limit))
    }

    /// One expression written both ways, if it can be. Dropped rather than
    /// reported when it names no field, since `4096` explains nothing that the
    /// size column does not already say, and dropped when reading it would
    /// have to wait for bytes: a relationship is worth showing when it is
    /// known and worth nothing guessed.
    fn relation<S: Source>(
        &mut self,
        doc: &Document<S>,
        at: &[usize],
        e: &Expr,
        role: Role,
        here: Option<(u64, u64)>,
        out: &mut Vec<Relation>,
    ) {
        let Some(written) = write_expr(e) else { return };
        let mut named = false;
        let here = here.or_else(|| self.memo.get(at).map(|r| (r.offset, r.limit)));
        let Ok(Some(substituted)) = self.substitute(doc, at, e, 0, here, &mut named) else { return };
        if !named || substituted == written {
            return;
        }
        let Ok(result) = self.eval_expr_at(doc, at, e, here) else { return };
        let result = result.to_string();
        // A substitution that already is the answer says the same thing twice.
        if substituted == result {
            return;
        }
        out.push(Relation { role, written, substituted, result });
    }

    /// The same expression with every leaf that reads the file replaced by
    /// what it reads. `named` comes back true when at least one was.
    fn substitute<S: Source>(
        &mut self,
        doc: &Document<S>,
        at: &[usize],
        e: &Expr,
        outer: u32,
        here: Option<(u64, u64)>,
        named: &mut bool,
    ) -> R<Option<String>> {
        let here_prec = prec(e);
        let wrap = |s: String| if here_prec > 0 && here_prec < outer { format!("({s})") } else { s };
        let two = |a: &Expr, b: &Expr, op: &str, ev: &mut Self, named: &mut bool| -> R<Option<String>> {
            let (Some(l), Some(r)) = (ev.substitute(doc, at, a, here_prec, here, named)?, ev.substitute(doc, at, b, here_prec + 1, here, named)?)
            else {
                return Ok(None);
            };
            Ok(Some(format!("{l} {op} {r}")))
        };
        let s = match e {
            Expr::Lit(_) => write_at(e, outer),
            Expr::Or(a, b) => two(a, b, "or else", self, named)?.map(wrap),
            Expr::Either(a, b) => two(a, b, "or", self, named)?.map(wrap),
            Expr::Both(a, b) => two(a, b, "and", self, named)?.map(wrap),
            Expr::Less(a, b) => two(a, b, "<", self, named)?.map(wrap),
            Expr::Eq(a, b) => two(a, b, "==", self, named)?.map(wrap),
            Expr::Ne(a, b) => two(a, b, "!=", self, named)?.map(wrap),
            Expr::Le(a, b) => two(a, b, "<=", self, named)?.map(wrap),
            Expr::Gt(a, b) => two(a, b, ">", self, named)?.map(wrap),
            Expr::Ge(a, b) => two(a, b, ">=", self, named)?.map(wrap),
            Expr::Mod(a, b) => two(a, b, "%", self, named)?.map(wrap),
            Expr::Not(a) => match self.substitute(doc, at, a, ALWAYS, here, named)? {
                Some(inner) => Some(wrap(format!("not {inner}"))),
                None => return Ok(None),
            },
            // Both branches, whichever one was taken. Writing out only the
            // branch the condition chose would say the reader was shown the
            // whole question, and leave them unable to check the answer
            // against the case that did not come up.
            Expr::Cond { when, then, otherwise } => {
                let (Some(c), Some(t), Some(f)) = (
                    self.substitute(doc, at, when, here_prec + 1, here, named)?,
                    self.substitute(doc, at, then, here_prec + 1, here, named)?,
                    self.substitute(doc, at, otherwise, here_prec + 1, here, named)?,
                ) else {
                    return Ok(None);
                };
                Some(wrap(format!("{c} ? {t} : {f}")))
            }
            Expr::Shl(a, b) => two(a, b, "<<", self, named)?.map(wrap),
            Expr::Shr(a, b) => two(a, b, ">>", self, named)?.map(wrap),
            Expr::And(a, b) => two(a, b, "&", self, named)?.map(wrap),
            Expr::Add(a, b) => two(a, b, "+", self, named)?.map(wrap),
            Expr::Sub(a, b) => two(a, b, "-", self, named)?.map(wrap),
            Expr::Mul(a, b) => two(a, b, "*", self, named)?.map(wrap),
            Expr::Div(a, b) => two(a, b, "/", self, named)?.map(wrap),
            Expr::Min(a, b) | Expr::Max(a, b) => {
                let name = if matches!(e, Expr::Min(..)) { "min" } else { "max" };
                let (Some(l), Some(r)) =
                    (self.substitute(doc, at, a, 0, here, named)?, self.substitute(doc, at, b, 0, here, named)?)
                else {
                    return Ok(None);
                };
                Some(format!("{name}({l}, {r})"))
            }
            Expr::DivCeil(a, b) => {
                let (Some(l), Some(r)) =
                    (self.substitute(doc, at, a, 0, here, named)?, self.substitute(doc, at, b, 0, here, named)?)
                else {
                    return Ok(None);
                };
                Some(format!("ceil({l} / {r})"))
            }
            Expr::Log2(a) => {
                let Some(inner) = self.substitute(doc, at, a, 0, here, named)? else { return Ok(None) };
                Some(format!("log2({inner})"))
            }
            Expr::PadTo { n, align } => {
                let Some(inner) = self.substitute(doc, at, n, 0, here, named)? else { return Ok(None) };
                Some(format!("align({inner}, {align})"))
            }
            Expr::Bit(a, i) => {
                let Some(inner) = self.substitute(doc, at, a, 0, here, named)? else { return Ok(None) };
                Some(format!("bit({inner}, {i})"))
            }
            // A search over a list, where the value alone would hide the half
            // of it this record contributed. `earlier[class_num = 9].name`
            // says what was looked for and where; `"trce"` says only what came
            // back, and leaves the reader to guess which element answered.
            Expr::Tagged(t) => {
                let tag = match &t.tag {
                    Tag::Computed(e) => self.substitute(doc, at, &e.clone(), 0, here, named)?,
                    // A label that is text: substituting the expression that
                    // works it out means the text it came to, since that is
                    // what the search was actually given. Leaving it as
                    // written would make the two forms the same and the whole
                    // relationship would be dropped as saying nothing.
                    Tag::ComputedText(e) => {
                        *named = true;
                        Some(format!("{:?}", self.text_at(doc, at, &e.clone(), here)?))
                    }
                    other => other.written(),
                };
                let Some(tag) = tag else { return Ok(None) };
                let field = if t.field.is_empty() { String::new() } else { format!(".{}", t.field.join(".")) };
                *named = true;
                let array = t.array.as_ref().and_then(write_expr).unwrap_or_else(|| "earlier".into());
                Some(format!("{array}[{} = {tag}]{field}", t.key.join(".")))
            }
            // Everything left that this can write at all is a leaf that reads
            // the file. What it reads is the whole of what substituting it
            // means, so one evaluation covers all of them.
            _ => {
                if write_expr(e).is_none() {
                    return Ok(None);
                }
                *named = true;
                Some(self.eval_expr_at(doc, at, e, here)?.to_string())
            }
        };
        Ok(s)
    }
}

/// The expression as the template writes it. None for the expressions with no
/// reading in this notation: a search for a byte pattern, or a peek at bits
/// that are not a field.
///
/// Public because a type can hold an expression too: a field as wide as
/// another field says names that field in the type column, and the notation it
/// is named in should be the one every other connection is written in. See
/// [`crate::template::Ty::UIntExpr`].
pub fn write_expr(e: &Expr) -> Option<String> {
    write_at(e, 0)
}

fn write_at(e: &Expr, outer: u32) -> Option<String> {
    let here = prec(e);
    let wrap = |s: String| if here > 0 && here < outer { format!("({s})") } else { s };
    let two = |a: &Expr, b: &Expr, op: &str| -> Option<String> {
        Some(wrap(format!("{} {op} {}", write_at(a, here)?, write_at(b, here + 1)?)))
    };
    let path = |array: &str, index: &Expr, field: &[String]| -> Option<String> {
        let mut s = format!("{array}[{}]", write_at(index, 0)?);
        for f in field {
            s.push('.');
            s.push_str(f);
        }
        Some(s)
    };
    Some(match e {
        Expr::Lit(n) => n.to_string(),
        Expr::Ref(n) => n.to_string(),
        Expr::Remaining => "remaining".to_string(),
        Expr::SizeOf(n) => format!("sizeof({n})"),
        Expr::BitsOf(n) => format!("bitsof({n})"),
        Expr::Idx => "index".to_string(),
        // Nothing to point at: the answer comes from running the file, not
        // from a field a reader could go and look at.
        Expr::Deduced(_) => return None,
        Expr::Elem { array, index, field } => path(array, index, field)?,
        Expr::ElemWithin { path: into, index, field } => path(&into.join("."), index, field)?,
        Expr::Product { array, index, field } => format!("product({})", path(array, index, field)?),
        Expr::ProductOf(n) => format!("product({n})"),
        Expr::SumOf(n) => format!("sum({n})"),
        Expr::MaxOf(n) => format!("max({n})"),
        // "set bits" rather than "popcount": the panel writes this beside a
        // length, where a reader wants what was counted and not the name of
        // the machine instruction that counts it.
        Expr::PopCount(n) => format!("set bits in {n}"),
        Expr::Prev(n) => format!("previous {n}"),
        Expr::Sibling(f) | Expr::Within(f) => f.join("."),
        // A question for another record, so it says whose: the names inside
        // are that record's fields, and written bare they would read as fields
        // beside this one. A name or a path reads as a path into the
        // descriptor, `descriptor.count`; anything longer is bracketed whole,
        // since qualifying only its first name would claim the rest were
        // fields beside this one.
        Expr::Placer(e) => match &**e {
            Expr::Ref(n) => format!("descriptor.{n}"),
            Expr::Within(f) => format!("descriptor.{}", f.join(".")),
            other => format!("descriptor.({})", write_at(other, 0)?),
        },
        // The list, the question asked of each element, and what is read from
        // the one that answers. A search over the elements before this one has
        // no field to name, so it is named for what it searches: `earlier`.
        Expr::Tagged(t) => {
            let key = t.key.join(".");
            let field = if t.field.is_empty() { String::new() } else { format!(".{}", t.field.join(".")) };
            let array = match &t.array { Some(array) => write_expr(array)?, None => "earlier".into() };
            format!("{array}[{key} = {}]{field}", t.tag.written()?)
        }
        // "or else" rather than "or", which the boolean one below is. The two
        // answer different things and there is one English word between them:
        // `flags or 4` is 12 under this and 1 under the other, and a reader
        // shown the same word for both cannot tell which they are looking at.
        // This is the fallback: "the length in this record, or else the last
        // record that had one".
        Expr::Or(a, b) => two(a, b, "or else")?,
        Expr::Either(a, b) => two(a, b, "or")?,
        Expr::Both(a, b) => two(a, b, "and")?,
        Expr::Less(a, b) => two(a, b, "<")?,
        Expr::Eq(a, b) => two(a, b, "==")?,
        Expr::Ne(a, b) => two(a, b, "!=")?,
        Expr::Le(a, b) => two(a, b, "<=")?,
        Expr::Gt(a, b) => two(a, b, ">")?,
        Expr::Ge(a, b) => two(a, b, ">=")?,
        Expr::Mod(a, b) => two(a, b, "%")?,
        // Bracketed unless what it negates is a leaf: `not a == b` reads two
        // ways and only one of them is what this means.
        Expr::Not(a) => wrap(format!("not {}", write_at(a, ALWAYS)?)),
        Expr::Cond { when, then, otherwise } => wrap(format!(
            "{} ? {} : {}",
            write_at(when, here + 1)?,
            write_at(then, here + 1)?,
            write_at(otherwise, here + 1)?
        )),
        Expr::Shl(a, b) => two(a, b, "<<")?,
        Expr::Shr(a, b) => two(a, b, ">>")?,
        Expr::And(a, b) => two(a, b, "&")?,
        Expr::Add(a, b) => two(a, b, "+")?,
        Expr::Sub(a, b) => two(a, b, "-")?,
        Expr::Mul(a, b) => two(a, b, "*")?,
        Expr::Div(a, b) => two(a, b, "/")?,
        Expr::Min(a, b) => format!("min({}, {})", write_at(a, 0)?, write_at(b, 0)?),
        Expr::Max(a, b) => format!("max({}, {})", write_at(a, 0)?, write_at(b, 0)?),
        Expr::DivCeil(a, b) => format!("ceil({} / {})", write_at(a, 10)?, write_at(b, 11)?),
        Expr::Log2(a) => format!("log2({})", write_at(a, 0)?),
        Expr::PadTo { n, align } => format!("align({}, {align})", write_at(n, 0)?),
        Expr::Bit(a, i) => format!("bit({}, {i})", write_at(a, 0)?),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::Expr as E;

    #[test]
    fn an_expression_reads_as_the_template_writes_it() {
        let e = E::Sub(Box::new(E::field("cell_content_start")), Box::new(E::lit(100)));
        assert_eq!(write_expr(&e).as_deref(), Some("cell_content_start - 100"));
    }

    #[test]
    fn brackets_appear_only_where_they_change_the_reading() {
        let sum = E::Add(Box::new(E::field("a")), Box::new(E::field("b")));
        let mul = E::Mul(Box::new(sum.clone()), Box::new(E::lit(2)));
        assert_eq!(write_expr(&mul).as_deref(), Some("(a + b) * 2"));
        let plain = E::Add(Box::new(E::Mul(Box::new(E::field("a")), Box::new(E::lit(2)))), Box::new(E::field("b")));
        assert_eq!(write_expr(&plain).as_deref(), Some("a * 2 + b"));
        // Subtraction does not associate, so the right side keeps its brackets.
        let right = E::Sub(Box::new(E::field("a")), Box::new(sum));
        assert_eq!(write_expr(&right).as_deref(), Some("a - (a + b)"));
    }

    #[test]
    fn a_lookup_by_computed_key_reads_as_the_question_it_asks() {
        // The list named beside the field, keyed on a number this record holds.
        let named = E::tagged_by_expr("structures", &["class_num"], E::field("class"), &["name"]);
        assert_eq!(write_expr(&named).as_deref(), Some("structures[class_num = class].name"));
        // The same over the elements before this one, which have no field name
        // to be reached by.
        let earlier = E::sibling_tagged(&["class_num"], E::field("class"), &["name"]);
        assert_eq!(write_expr(&earlier).as_deref(), Some("earlier[class_num = class].name"));
    }

    #[test]
    fn a_question_for_the_descriptor_says_whose_fields_it_names() {
        let count = E::placer(E::field("count"));
        assert_eq!(write_expr(&count).as_deref(), Some("descriptor.count"));
        // Inside something larger it is still one term.
        let at_most = count.at_most(E::Remaining);
        assert_eq!(write_expr(&at_most).as_deref(), Some("min(descriptor.count, remaining)"));
        // And a longer question is bracketed whole, so the second name is not
        // read as a field beside the element.
        let product = E::placer(E::field("count").mul(E::field("width")));
        assert_eq!(write_expr(&product).as_deref(), Some("descriptor.(count * width)"));
    }

    #[test]
    fn the_arithmetic_and_the_comparisons_read_as_they_are_written() {
        let a = || E::field("a");
        let b = || E::field("b");
        assert_eq!(write_expr(&a().modulo(E::lit(4))).as_deref(), Some("a % 4"));
        assert_eq!(write_expr(&a().equal_to(b())).as_deref(), Some("a == b"));
        assert_eq!(write_expr(&a().not_equal(b())).as_deref(), Some("a != b"));
        assert_eq!(write_expr(&a().less_or_equal(b())).as_deref(), Some("a <= b"));
        assert_eq!(write_expr(&a().greater_than(b())).as_deref(), Some("a > b"));
        assert_eq!(write_expr(&a().greater_or_equal(b())).as_deref(), Some("a >= b"));
        // A modulo binds as tightly as the division it is written beside.
        assert_eq!(write_expr(&a().add(b()).modulo(E::lit(4))).as_deref(), Some("(a + b) % 4"));
        assert_eq!(write_expr(&a().modulo(E::lit(4)).add(b())).as_deref(), Some("a % 4 + b"));
        // A comparison binds looser than the arithmetic in it.
        assert_eq!(write_expr(&a().add(E::lit(1)).equal_to(b())).as_deref(), Some("a + 1 == b"));
    }

    /// Two operators that both mean something like "or", so they cannot both
    /// be written "or": one answers a truth and the other answers a value.
    #[test]
    fn the_two_kinds_of_or_are_told_apart_in_words() {
        let a = || E::field("a");
        let b = || E::field("b");
        assert_eq!(write_expr(&a().either(b())).as_deref(), Some("a or b"));
        assert_eq!(write_expr(&a().or(b())).as_deref(), Some("a or else b"));
        assert_eq!(write_expr(&a().both(b())).as_deref(), Some("a and b"));
        // A comparison inside a boolean needs no brackets; a value-or does,
        // since the reader would otherwise have to know which binds tighter.
        assert_eq!(
            write_expr(&a().equal_to(E::lit(1)).both(b().greater_than(E::lit(2)))).as_deref(),
            Some("a == 1 and b > 2")
        );
        assert_eq!(write_expr(&a().both(b().or(E::lit(3)))).as_deref(), Some("a and (b or else 3)"));
        // `or` is looser than `and`, as it is everywhere these words are used.
        assert_eq!(
            write_expr(&a().either(b().both(E::lit(1)))).as_deref(),
            Some("a or b and 1")
        );
        assert_eq!(
            write_expr(&a().either(b()).both(E::lit(1))).as_deref(),
            Some("(a or b) and 1")
        );
    }

    /// `not a == b` has two readings, so it is never written: what is negated
    /// is bracketed unless it is a single name or number.
    #[test]
    fn a_negation_brackets_whatever_is_not_a_leaf() {
        assert_eq!(write_expr(&E::field("a").negate()).as_deref(), Some("not a"));
        assert_eq!(write_expr(&E::Remaining.negate()).as_deref(), Some("not remaining"));
        assert_eq!(
            write_expr(&E::field("a").equal_to(E::lit(1)).negate()).as_deref(),
            Some("not (a == 1)")
        );
        assert_eq!(
            write_expr(&E::field("a").negate().both(E::field("b"))).as_deref(),
            Some("not a and b")
        );
    }

    #[test]
    fn a_ternary_reads_as_a_ternary_and_nests_in_brackets() {
        let e = E::cond(E::field("wide"), E::field("long"), E::field("short"));
        assert_eq!(write_expr(&e).as_deref(), Some("wide ? long : short"));
        // Inside arithmetic it is one term.
        assert_eq!(write_expr(&e.clone().add(E::lit(1))).as_deref(), Some("(wide ? long : short) + 1"));
        // And a ternary inside a ternary is bracketed on either side, so
        // there is nothing to work out about which colon belongs to which.
        let nested = E::cond(E::field("a"), e.clone(), E::lit(0));
        assert_eq!(write_expr(&nested).as_deref(), Some("a ? (wide ? long : short) : 0"));
        let tail = E::cond(E::field("a"), E::lit(0), e);
        assert_eq!(write_expr(&tail).as_deref(), Some("a ? 0 : (wide ? long : short)"));
    }

    #[test]
    fn an_expression_with_no_reading_is_not_half_written() {
        let e = E::Add(Box::new(E::field("a")), Box::new(E::Find { needle: vec![0], last: false }));
        assert_eq!(write_expr(&e), None);
    }
}
