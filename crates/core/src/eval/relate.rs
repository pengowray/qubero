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
        Expr::Less(..) => 2,
        Expr::Shl(..) => 3,
        Expr::Add(..) | Expr::Sub(..) => 4,
        Expr::Mul(..) | Expr::Div(..) => 5,
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
        let base = self.memo[path].ty.clone();
        match &base {
            Ty::Bytes(e) => self.relation(doc, path, &e.clone(), Role::Length, &mut out),
            Ty::Str { len: StrLen::Fixed(e) | StrLen::Padded { size: e, .. }, .. } => {
                self.relation(doc, path, &e.clone(), Role::Length, &mut out)
            }
            Ty::Array { count, .. } => self.relation(doc, path, &count.clone(), Role::Count, &mut out),
            Ty::Computed(e) => self.relation(doc, path, &e.clone(), Role::Value, &mut out),
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
        let Some(written) = write_expr(e, 0) else { return };
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
            Expr::Lit(_) => write_expr(e, outer),
            Expr::Or(a, b) => two(a, b, "or", self, named)?.map(wrap),
            Expr::Less(a, b) => two(a, b, "<", self, named)?.map(wrap),
            Expr::Shl(a, b) => two(a, b, "<<", self, named)?.map(wrap),
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
            // Everything left that this can write at all is a leaf that reads
            // the file. What it reads is the whole of what substituting it
            // means, so one evaluation covers all of them.
            _ => {
                if write_expr(e, 0).is_none() {
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
fn write_expr(e: &Expr, outer: u32) -> Option<String> {
    let here = prec(e);
    let wrap = |s: String| if here > 0 && here < outer { format!("({s})") } else { s };
    let two = |a: &Expr, b: &Expr, op: &str| -> Option<String> {
        Some(wrap(format!("{} {op} {}", write_expr(a, here)?, write_expr(b, here + 1)?)))
    };
    let path = |array: &str, index: &Expr, field: &[String]| -> Option<String> {
        let mut s = format!("{array}[{}]", write_expr(index, 0)?);
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
        Expr::Product { array, index, field } => format!("product({})", path(array, index, field)?),
        Expr::ProductOf(n) => format!("product({n})"),
        Expr::SumOf(n) => format!("sum({n})"),
        Expr::MaxOf(n) => format!("max({n})"),
        Expr::Prev(n) => format!("previous {n}"),
        Expr::Sibling(f) | Expr::Within(f) => f.join("."),
        Expr::Tagged(t) => {
            let key = t.key.join(".");
            let field = if t.field.is_empty() { String::new() } else { format!(".{}", t.field.join(".")) };
            format!("{}[{key} = {}]{field}", t.array, t.tag)
        }
        Expr::Or(a, b) => two(a, b, "or")?,
        Expr::Less(a, b) => two(a, b, "<")?,
        Expr::Shl(a, b) => two(a, b, "<<")?,
        Expr::Add(a, b) => two(a, b, "+")?,
        Expr::Sub(a, b) => two(a, b, "-")?,
        Expr::Mul(a, b) => two(a, b, "*")?,
        Expr::Div(a, b) => two(a, b, "/")?,
        Expr::Min(a, b) => format!("min({}, {})", write_expr(a, 0)?, write_expr(b, 0)?),
        Expr::Max(a, b) => format!("max({}, {})", write_expr(a, 0)?, write_expr(b, 0)?),
        Expr::PadTo { n, align } => format!("align({}, {align})", write_expr(n, 0)?),
        Expr::Bit(a, i) => format!("bit({}, {i})", write_expr(a, 0)?),
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
        assert_eq!(write_expr(&e, 0).as_deref(), Some("cell_content_start - 100"));
    }

    #[test]
    fn brackets_appear_only_where_they_change_the_reading() {
        let sum = E::Add(Box::new(E::field("a")), Box::new(E::field("b")));
        let mul = E::Mul(Box::new(sum.clone()), Box::new(E::lit(2)));
        assert_eq!(write_expr(&mul, 0).as_deref(), Some("(a + b) * 2"));
        let plain = E::Add(Box::new(E::Mul(Box::new(E::field("a")), Box::new(E::lit(2)))), Box::new(E::field("b")));
        assert_eq!(write_expr(&plain, 0).as_deref(), Some("a * 2 + b"));
        // Subtraction does not associate, so the right side keeps its brackets.
        let right = E::Sub(Box::new(E::field("a")), Box::new(sum));
        assert_eq!(write_expr(&right, 0).as_deref(), Some("a - (a + b)"));
    }

    #[test]
    fn an_expression_with_no_reading_is_not_half_written() {
        let e = E::Add(Box::new(E::field("a")), Box::new(E::Find { needle: vec![0], last: false }));
        assert_eq!(write_expr(&e, 0), None);
    }
}
