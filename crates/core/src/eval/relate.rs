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
        Expr::Or(..) => 1,
        // Between `or` and a comparison, as it is in every language that
        // writes these: `a & b < c` is the comparison of `a & b`, and a mask
        // written beside an addition binds looser than the addition.
        Expr::And(..) => 2,
        Expr::Less(..) => 3,
        Expr::Shl(..) | Expr::Shr(..) => 4,
        Expr::Add(..) | Expr::Sub(..) => 5,
        Expr::Mul(..) | Expr::Div(..) => 6,
        _ => 0,
    }
}

impl Evaluator {
    /// The relationships behind the field at `path`: what decided its length,
    /// how many children it has, which type it was read as, or what it says.
    /// Empty for a field the template placed and sized outright, and for one
    /// whose expression this cannot write out.
    pub fn relations<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Vec<Relation>> {
        self.resolve(doc, path)?;
        let mut out = Vec::new();
        if let Some(from) = self.name_from(path) {
            self.relation(doc, path, &from, Role::Name, &mut out);
        }
        let declared = self.declared_ty(path)?;
        let mut ty = declared;
        for _ in 0..64 {
            match ty {
                Ty::Named(n) => match self.template.types.get(&*n) {
                    Some(t) => ty = t.clone(),
                    None => break,
                },
                Ty::Sized { size, inner } => {
                    self.relation(doc, path, &size, Role::Length, &mut out);
                    ty = *inner;
                }
                Ty::Switch { on, .. } | Ty::Match { on, .. } => {
                    self.relation(doc, path, &on, Role::Type, &mut out);
                    break;
                }
                _ => break,
            }
        }
        let base = self.memo[path].ty.without_sentinel().clone();
        match &base {
            Ty::Bytes(e) => self.relation(doc, path, &e.clone(), Role::Length, &mut out),
            // How wide the number is, which is the one thing about it the
            // template did not fix. Told apart from a byte length because it
            // is bits and because a reader asking "why is this eleven bits"
            // wants the field that said eleven.
            Ty::UIntExpr { bits, .. } => self.relation(doc, path, &(**bits).clone(), Role::Width, &mut out),
            Ty::Str { len: StrLen::Fixed(e) | StrLen::Padded { size: e, .. }, .. } => {
                self.relation(doc, path, &e.clone(), Role::Length, &mut out)
            }
            Ty::Array { count, .. } => self.relation(doc, path, &count.clone(), Role::Count, &mut out),
            Ty::Computed(e) | Ty::ComputedText(e) => self.relation(doc, path, &e.clone(), Role::Value, &mut out),
            _ => {}
        }
        Ok(out)
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
        out: &mut Vec<Relation>,
    ) {
        let Some(written) = write_expr(e) else { return };
        let mut named = false;
        let Ok(Some(substituted)) = self.substitute(doc, at, e, 0, &mut named) else { return };
        if !named || substituted == written {
            return;
        }
        let Ok(result) = self.eval_expr(doc, at, e) else { return };
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
        named: &mut bool,
    ) -> R<Option<String>> {
        let here = prec(e);
        let wrap = |s: String| if here > 0 && here < outer { format!("({s})") } else { s };
        let two = |a: &Expr, b: &Expr, op: &str, ev: &mut Self, named: &mut bool| -> R<Option<String>> {
            let (Some(l), Some(r)) = (ev.substitute(doc, at, a, here, named)?, ev.substitute(doc, at, b, here + 1, named)?)
            else {
                return Ok(None);
            };
            Ok(Some(format!("{l} {op} {r}")))
        };
        let s = match e {
            Expr::Lit(_) => write_at(e, outer),
            Expr::Or(a, b) => two(a, b, "or", self, named)?.map(wrap),
            Expr::Less(a, b) => two(a, b, "<", self, named)?.map(wrap),
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
                    (self.substitute(doc, at, a, 0, named)?, self.substitute(doc, at, b, 0, named)?)
                else {
                    return Ok(None);
                };
                Some(format!("{name}({l}, {r})"))
            }
            Expr::PadTo { n, align } => {
                let Some(inner) = self.substitute(doc, at, n, 0, named)? else { return Ok(None) };
                Some(format!("align({inner}, {align})"))
            }
            Expr::Bit(a, i) => {
                let Some(inner) = self.substitute(doc, at, a, 0, named)? else { return Ok(None) };
                Some(format!("bit({inner}, {i})"))
            }
            // A search over a list, where the value alone would hide the half
            // of it this record contributed. `earlier[class_num = 9].name`
            // says what was looked for and where; `"trce"` says only what came
            // back, and leaves the reader to guess which element answered.
            Expr::Tagged(t) => {
                let tag = match &t.tag {
                    Tag::Computed(e) => self.substitute(doc, at, &e.clone(), 0, named)?,
                    // A label that is text: substituting the expression that
                    // works it out means the text it came to, since that is
                    // what the search was actually given. Leaving it as
                    // written would make the two forms the same and the whole
                    // relationship would be dropped as saying nothing.
                    Tag::ComputedText(e) => {
                        let here = self.memo.get(at).map(|r| (r.offset, r.limit));
                        *named = true;
                        Some(format!("{:?}", self.text_at(doc, at, &e.clone(), here)?))
                    }
                    other => other.written(),
                };
                let Some(tag) = tag else { return Ok(None) };
                let field = if t.field.is_empty() { String::new() } else { format!(".{}", t.field.join(".")) };
                *named = true;
                Some(format!("{}[{} = {tag}]{field}", t.array.as_deref().unwrap_or("earlier"), t.key.join(".")))
            }
            // Everything left that this can write at all is a leaf that reads
            // the file. What it reads is the whole of what substituting it
            // means, so one evaluation covers all of them.
            _ => {
                if write_expr(e).is_none() {
                    return Ok(None);
                }
                *named = true;
                Some(self.eval_expr(doc, at, e)?.to_string())
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
        Expr::Elem { array, index, field } => path(array, index, field)?,
        Expr::ElemWithin { path: into, index, field } => path(&into.join("."), index, field)?,
        Expr::Product { array, index, field } => format!("product({})", path(array, index, field)?),
        Expr::ProductOf(n) => format!("product({n})"),
        Expr::SumOf(n) => format!("sum({n})"),
        Expr::MaxOf(n) => format!("max({n})"),
        Expr::Prev(n) => format!("previous {n}"),
        Expr::Sibling(f) | Expr::Within(f) => f.join("."),
        // The list, the question asked of each element, and what is read from
        // the one that answers. A search over the elements before this one has
        // no field to name, so it is named for what it searches: `earlier`.
        Expr::Tagged(t) => {
            let key = t.key.join(".");
            let field = if t.field.is_empty() { String::new() } else { format!(".{}", t.field.join(".")) };
            let array = t.array.as_deref().unwrap_or("earlier");
            format!("{array}[{key} = {}]{field}", t.tag.written()?)
        }
        Expr::Or(a, b) => two(a, b, "or")?,
        Expr::Less(a, b) => two(a, b, "<")?,
        Expr::Shl(a, b) => two(a, b, "<<")?,
        Expr::Shr(a, b) => two(a, b, ">>")?,
        Expr::And(a, b) => two(a, b, "&")?,
        Expr::Add(a, b) => two(a, b, "+")?,
        Expr::Sub(a, b) => two(a, b, "-")?,
        Expr::Mul(a, b) => two(a, b, "*")?,
        Expr::Div(a, b) => two(a, b, "/")?,
        Expr::Min(a, b) => format!("min({}, {})", write_at(a, 0)?, write_at(b, 0)?),
        Expr::Max(a, b) => format!("max({}, {})", write_at(a, 0)?, write_at(b, 0)?),
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
    fn an_expression_with_no_reading_is_not_half_written() {
        let e = E::Add(Box::new(E::field("a")), Box::new(E::Find { needle: vec![0], last: false }));
        assert_eq!(write_expr(&e), None);
    }
}
