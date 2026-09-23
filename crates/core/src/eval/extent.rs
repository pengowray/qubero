//! Whether each part of a file ends where the field giving its length says.
//!
//! Nearly every format writes a part's length or count in front of it, and
//! nearly every damaged file is damaged there: a size that overshoots the end
//! of the file because the file was cut short, a size a recorder forgot to
//! update after it appended a chunk, a count of entries that the room holds
//! two of. The reader already copes with most of these one at a time. A run
//! whose element overruns its room is stretched to take the element in
//! (`stretch_to` in `walk.rs`), a run that meets an element it cannot read
//! stops and says why (`repeat_trouble`), a template caps a size at the room
//! left with `min(size, remaining)`. What nothing did was say, for the whole
//! file, which lengths the reader had to overrule.
//!
//! So for every field another field's length or count depends on (see
//! [`crate::machinery::measurers`]), this compares four numbers: what the
//! length field says, what the template made of it before any cap
//! (`stated`), what the part was read as (`read`), and how far the part's own
//! fields reached (`content`), against the room its parent had and the length
//! of the file. A part that would not read at all is looked into until the
//! field that would not fit is found, and the same four numbers are given for
//! that one.
//!
//! The verdicts:
//!
//! - `fits`: the part ends where the length says, and its fields fill it.
//! - `short`: it ends where the length says, and its fields stop before that.
//!   The bits in between are a gap in the ledger.
//! - `stretched`: the part runs further than the length says. The reader made
//!   room for an element that overran it, or the template adds bytes the
//!   length leaves out.
//! - `past-parent` and `past-file`: the length says the part runs past the end
//!   of what it is in, or past the end of the file, and the reader stopped it
//!   there or could not read it.
//! - `unreadable`: the part would not read, for a reason that is not its
//!   length. `why` has the reader's own reason.

use super::*;

/// One length checked against the part it sizes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtentCheck {
    /// The field that gives the length or count, and what it holds. Empty
    /// path, empty name and no value when the length comes from no field of
    /// its own: a peek at the bytes ahead, or the room left.
    pub length_path: Vec<usize>,
    pub length_name: String,
    pub length_value: Option<i128>,
    /// True when the length field breaks a constraint the template declares
    /// for it, which is the listing's red mark on the same field.
    pub length_invalid: bool,
    /// The part the length sizes.
    pub part_path: Vec<usize>,
    pub part_name: String,
    /// `length` or `count`.
    pub role: &'static str,
    /// Where the part starts.
    pub offset_bits: u64,
    /// For a length: how long the template says the part is before any cap.
    /// For a count: how many elements. None when it would not work out.
    pub stated: Option<u64>,
    /// The same, as the reader read it. None when the part would not read.
    pub read: Option<u64>,
    /// How far the part's own fields reached from its start, for a part that
    /// has fields. None for a part that is one value.
    pub content_bits: Option<u64>,
    /// How much room the part had, from its start to the end of what it is
    /// in, and how long the file is. `stated` past the one is `past-parent`,
    /// past the other `past-file`.
    pub room_bits: u64,
    pub space_bits: u64,
    /// True when the template's length reads more than the length field: it
    /// adds a header the length leaves out, or a block a recorder forgot. The
    /// length field's own value is then not the part's length in any unit.
    pub adjusted: bool,
    pub verdict: &'static str,
    /// The reader's own words for why the part would not read. Empty when it
    /// read.
    pub why: String,
}

/// The whole audit so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtentAudit {
    /// Every check that is not `fits`, in the order the walk met them. Capped
    /// at [`KEEP`]; `counts` counts all of them.
    pub checks: Vec<ExtentCheck>,
    /// How many checks came to each verdict, `fits` included.
    pub counts: Vec<(&'static str, u64)>,
    /// Where the root's own fields end, and how long the file is. The bits
    /// between are after the root: often a trailer the template does not
    /// describe, sometimes placed fields, which the ledger tells apart.
    pub root_end_bits: u64,
    pub space_bits: u64,
    /// True once the whole file has been walked.
    pub done: bool,
    /// The reader's reason, when the root itself would not read and so
    /// nothing could be walked. The checks then say where the root failed.
    pub root_failed: Option<String>,
}

/// How many checks that are not `fits` are kept.
pub const KEEP: usize = 500;

/// How a declaration gets its length from a field, and in what unit.
#[derive(Debug, Clone)]
pub(super) struct Measure {
    pub role: &'static str,
    pub expr: Expr,
    /// How many bits one of what the expression counts is: 8 for bytes, 1 for
    /// bits, and 0 for elements, which have no size of their own here.
    pub unit: u64,
}

/// What an audit found out, kept between goes.
#[derive(Debug, Default)]
pub(super) struct Tally {
    checks: Vec<ExtentCheck>,
    counts: std::collections::BTreeMap<&'static str, u64>,
    pub root_end_bits: u64,
    pub space_bits: u64,
    pub root_failed: Option<String>,
}

impl Tally {
    pub(super) fn add(&mut self, c: ExtentCheck, times: u64) {
        *self.counts.entry(c.verdict).or_default() += times;
        if c.verdict != "fits" && self.checks.len() < KEEP {
            self.checks.push(c);
        }
    }

    pub(super) fn audit(&self, done: bool) -> ExtentAudit {
        ExtentAudit {
            checks: self.checks.clone(),
            counts: self.counts.iter().map(|(k, v)| (*k, *v)).collect(),
            root_end_bits: self.root_end_bits,
            space_bits: self.space_bits,
            done,
            root_failed: self.root_failed.clone(),
        }
    }
}

/// Where in a field's declaration its length or count is read from the field
/// called `name`, looking through names, windows, choices and conditions.
/// Nothing inside an element of a list: that is the element's own length.
pub(super) fn find_measure(t: &Template, ty: &Ty, name: Option<&str>, depth: u32) -> Option<Measure> {
    if depth > 16 {
        return None;
    }
    let names = |e: &Expr| match name {
        Some(n) => machinery::names_in(e).iter().any(|m| &**m == n),
        None => !matches!(e, Expr::Lit(_)),
    };
    let length = |e: &Expr, unit: u64| Some(Measure { role: "length", expr: e.clone(), unit });
    match ty {
        Ty::Named(n) => t.types.get(&**n).and_then(|inner| find_measure(t, inner, name, depth + 1)),
        Ty::Sized { size, inner } => {
            if names(size) {
                length(size, 8)
            } else {
                find_measure(t, inner, name, depth + 1)
            }
        }
        Ty::SizedBits { bits, inner } => {
            if names(bits) {
                length(bits, 1)
            } else {
                find_measure(t, inner, name, depth + 1)
            }
        }
        Ty::Bytes(e) if names(e) => length(e, 8),
        Ty::Str { len, .. } | Ty::TextInt { len, .. } => match len {
            crate::template::StrLen::Fixed(e) | crate::template::StrLen::Padded { size: e, .. } if names(e) => length(e, 8),
            _ => None,
        },
        Ty::Array { count, .. } if names(count) => Some(Measure { role: "count", expr: count.clone(), unit: 0 }),
        Ty::Switch { cases, default, .. } => cases
            .iter()
            .map(|(_, c)| c)
            .chain(std::iter::once(&**default))
            .find_map(|c| find_measure(t, c, name, depth + 1)),
        Ty::Match { cases, default, .. } => cases
            .iter()
            .map(|(_, c)| c)
            .chain(std::iter::once(&**default))
            .find_map(|c| find_measure(t, c, name, depth + 1)),
        Ty::When { inner, .. } | Ty::Origin { inner } | Ty::Enum { inner, .. } | Ty::Nullable { inner, .. } | Ty::Flags { inner, .. } => {
            find_measure(t, inner, name, depth + 1)
        }
        _ => None,
    }
}

/// The length an expression gives before the template caps it at the room
/// left. `min(size, remaining)` is how a template says "believe the size, but
/// not past the end", and what the size said is the number this audit is for.
pub(super) fn unclamped(e: &Expr) -> Expr {
    let b = |x: &Expr| Box::new(unclamped(x));
    match e {
        Expr::Min(a, c) if room(c) && !room(a) => unclamped(a),
        Expr::Min(a, c) if room(a) && !room(c) => unclamped(c),
        Expr::Add(a, c) => Expr::Add(b(a), b(c)),
        Expr::Sub(a, c) => Expr::Sub(b(a), b(c)),
        Expr::Mul(a, c) => Expr::Mul(b(a), b(c)),
        Expr::Div(a, c) => Expr::Div(b(a), b(c)),
        Expr::DivCeil(a, c) => Expr::DivCeil(b(a), b(c)),
        Expr::Or(a, c) => Expr::Or(b(a), c.clone()),
        Expr::Max(a, c) => Expr::Max(b(a), b(c)),
        Expr::Min(a, c) => Expr::Min(b(a), b(c)),
        Expr::Cond { when, then, otherwise } => Expr::Cond { when: when.clone(), then: b(then), otherwise: b(otherwise) },
        other => other.clone(),
    }
}

/// Whether an expression is a measure of the room left rather than something
/// the file says: the room itself, the window, the space's length, and any
/// arithmetic on those alone.
fn room(e: &Expr) -> bool {
    match e {
        Expr::Remaining | Expr::WindowSize | Expr::SpaceSize => true,
        Expr::Lit(_) => false,
        Expr::Add(a, c) | Expr::Sub(a, c) | Expr::Mul(a, c) | Expr::Div(a, c) | Expr::Min(a, c) | Expr::Max(a, c) | Expr::DivCeil(a, c) => {
            (room(a) || room(c)) && !reads(a) && !reads(c)
        }
        _ => false,
    }
}

/// Whether an expression reads a field or the bytes, rather than only the
/// room and literals.
fn reads(e: &Expr) -> bool {
    match e {
        Expr::Lit(_) | Expr::Remaining | Expr::WindowSize | Expr::SpaceSize | Expr::Real(_) => false,
        Expr::Add(a, c) | Expr::Sub(a, c) | Expr::Mul(a, c) | Expr::Div(a, c) | Expr::Min(a, c) | Expr::Max(a, c) | Expr::DivCeil(a, c) | Expr::Or(a, c) => {
            reads(a) || reads(c)
        }
        _ => true,
    }
}

/// Whether a length reads anything besides the field `name`: another field,
/// or the bytes themselves. The room left does not count, since a cap on the
/// room is what `unclamped` takes off.
pub(super) fn reads_besides(e: &Expr, name: &str) -> bool {
    if machinery::names_in(e).iter().any(|n| &**n != name) {
        return true;
    }
    peeks(e)
}

/// Whether an expression looks at the bytes directly.
fn peeks(e: &Expr) -> bool {
    match e {
        Expr::Peek { .. } | Expr::PeekAt { .. } | Expr::PeekIn { .. } | Expr::Find { .. } | Expr::Run { .. } | Expr::ToMarker { .. } | Expr::StreamLen(_) => true,
        Expr::Add(a, c)
        | Expr::Sub(a, c)
        | Expr::Mul(a, c)
        | Expr::Div(a, c)
        | Expr::Mod(a, c)
        | Expr::DivCeil(a, c)
        | Expr::Or(a, c)
        | Expr::Either(a, c)
        | Expr::Both(a, c)
        | Expr::And(a, c)
        | Expr::BitOr(a, c)
        | Expr::BitXor(a, c)
        | Expr::Less(a, c)
        | Expr::Eq(a, c)
        | Expr::Ne(a, c)
        | Expr::Le(a, c)
        | Expr::Gt(a, c)
        | Expr::Ge(a, c)
        | Expr::Shl(a, c)
        | Expr::Shr(a, c)
        | Expr::Min(a, c)
        | Expr::Max(a, c) => peeks(a) || peeks(c),
        Expr::Cond { when, then, otherwise } => peeks(when) || peeks(then) || peeks(otherwise),
        Expr::Not(a) | Expr::BitNot(a) | Expr::Log2(a) | Expr::Pow2(a) | Expr::Pow10(a) | Expr::Trunc(a) | Expr::Bit(a, _) => peeks(a),
        Expr::PadTo { n, .. } => peeks(n),
        _ => false,
    }
}

/// Which verdict a length comes to, given where the part starts, how long
/// the template says it is, and how long it was read as.
pub(super) fn length_verdict(offset: u64, stated: u64, read: u64, space_bits: u64) -> &'static str {
    if stated > read {
        if offset.saturating_add(stated) > space_bits { "past-file" } else { "past-parent" }
    } else if stated < read {
        "stretched"
    } else {
        "fits"
    }
}

impl Evaluator {
    /// The check for a part that read, sized by the sibling at index `by` of
    /// the structure it is in. The verdict is `fits` where only its fields
    /// can say more, which the walk settles when it closes the part.
    ///
    /// None when the length cannot be worked out apart from the part: the
    /// expression names something that is not there any more.
    pub(super) fn check_part<S: Source>(&mut self, doc: &Document<S>, path: &[usize], by: usize) -> R<Option<ExtentCheck>> {
        let Some((&idx, parent)) = path.split_last() else { return Ok(None) };
        let (decl, name, pr_limit, pr_space) = {
            let Some(pr) = self.memo.get(parent) else { return Ok(None) };
            let Ty::Struct(s) = pr.ty.base() else { return Ok(None) };
            let (Some(f), Some(g)) = (s.fields.get(idx), s.fields.get(by)) else { return Ok(None) };
            (f.ty.clone(), g.name.to_string(), pr.limit, pr.space)
        };
        let r = self.memo[path].clone();
        // A field declared as a choice is measured by the case it took: a
        // SQLite cell whose payload spilled is not the size its length says,
        // and was never meant to be.
        let decl = self.case_declared(&decl, &r.ty);
        let Some(m) = find_measure(&self.template, &decl, Some(&name), 0) else { return Ok(None) };
        let stated = match self.eval_expr_at(doc, path, &unclamped(&m.expr), Some((r.cursor, pr_limit))) {
            Ok(v) => u64::try_from(v).ok(),
            Err(e) if e.interrupted() => return Err(e),
            Err(_) => None,
        };
        let Some(stated) = stated else { return Ok(None) };
        let space_bits = self.space_room(doc, pr_space);
        let (stated, read) = match m.role {
            "count" => (stated, self.child_count(doc, path)?),
            _ => {
                let read = match r.declared_size {
                    Some(bits) if matches!(decl_base(&self.template, &decl), Ty::Sized { .. } | Ty::SizedBits { .. }) => bits,
                    _ => self.size_of(doc, path)?,
                };
                (stated.saturating_mul(m.unit), read)
            }
        };
        let verdict = match m.role {
            "count" if stated > read => "past-parent",
            "count" if stated < read => "stretched",
            "count" => "fits",
            _ => length_verdict(r.offset, stated, read, space_bits),
        };
        let mut length = parent.to_vec();
        length.push(by);
        let check = self.length_check(doc, &length, path, m, (stated, read), (r.offset, pr_limit, space_bits), verdict, "")?;
        Ok(Some(check))
    }

    /// Why child `idx` of `parent` would not read, looked into until the
    /// field that would not fit is found. `at` is where the child starts.
    ///
    /// What was placed to find out is given back afterwards unless it was
    /// already held: the walk does not come back to a part that failed.
    pub(super) fn diagnose<S: Source>(&mut self, doc: &Document<S>, parent: &[usize], idx: usize, at: u64, why: &str, depth: u32) -> R<Option<ExtentCheck>> {
        let mut child = parent.to_vec();
        child.push(idx);
        let held = self.memo.contains_key(&child);
        let found = self.diagnose_placed(doc, parent, &child, at, why, depth);
        if !held && !matches!(&found, Err(e) if e.interrupted()) {
            self.memo.forget_under(&child);
        }
        found
    }

    fn diagnose_placed<S: Source>(&mut self, doc: &Document<S>, parent: &[usize], child: &[usize], at: u64, why: &str, depth: u32) -> R<Option<ExtentCheck>> {
        let idx = *child.last().expect("a child");
        match self.resolve(doc, child) {
            Err(e) if e.interrupted() => return Err(e),
            Err(_) => return self.failed_length(doc, parent, idx, at, why),
            Ok(()) => {}
        }
        match self.size_of(doc, child) {
            Err(e) if e.interrupted() => return Err(e),
            // It reads now: nothing to say.
            Ok(_) => return Ok(None),
            Err(_) => {}
        }
        // Placed, and something inside it will not read. Only a structure is
        // looked into: its fields are a handful, where a list's elements may
        // be a walk to the end of the file.
        let fields = match self.memo[child].ty.base() {
            Ty::Struct(s) if !s.overlap && depth < 8 => s.fields.len(),
            _ => return self.failed_length(doc, parent, idx, at, why),
        };
        let mut from = self.memo[child].offset;
        for k in 0..fields {
            let mut g = child.to_vec();
            g.push(k);
            match self.resolve(doc, &g).and_then(|()| self.size_of(doc, &g)) {
                Ok(size) => from = self.memo[&g].cursor + size,
                Err(e) if e.interrupted() => return Err(e),
                Err(EvalError::Failed(w)) => return self.diagnose(doc, child, k, from, &w, depth + 1),
                Err(e) => return Err(e),
            }
        }
        self.failed_length(doc, parent, idx, at, why)
    }

    /// The check for child `idx` of `parent`, which would not be placed at
    /// all: its own declaration failed, usually by a length past its room.
    fn failed_length<S: Source>(&mut self, doc: &Document<S>, parent: &[usize], idx: usize, at: u64, why: &str) -> R<Option<ExtentCheck>> {
        let mut child = parent.to_vec();
        child.push(idx);
        let Some(pr) = self.memo.get(parent).cloned() else { return Ok(None) };
        let space_bits = self.space_room(doc, pr.space);
        // Which sibling gives the length, when the child is a field of a
        // structure, and what the declaration makes of it.
        let (decl, by) = match pr.ty.base() {
            Ty::Struct(s) => {
                let Some(f) = s.fields.get(idx) else { return Ok(None) };
                let measured = machinery::measurers(s);
                let by = measured.iter().position(|m| *m == Some(idx));
                (f.ty.clone(), by.map(|j| (j, s.fields[j].name.to_string())))
            }
            _ => match self.declared_ty(&child) {
                Ok(t) => (t, None),
                Err(_) => return Ok(None),
            },
        };
        let m = match &by {
            Some((_, name)) => find_measure(&self.template, &decl, Some(name), 0),
            None => find_measure(&self.template, &decl, None, 0),
        };
        let stated = match &m {
            Some(m) => match self.eval_expr_at(doc, &child, &unclamped(&m.expr), Some((at, pr.limit))) {
                Ok(v) => u64::try_from(v).ok().map(|v| if m.role == "count" { v } else { v.saturating_mul(m.unit) }),
                Err(e) if e.interrupted() => return Err(e),
                Err(_) => None,
            },
            None => None,
        };
        let verdict = match (&m, stated) {
            (Some(m), Some(s)) if m.role == "length" && at.saturating_add(s) > space_bits => "past-file",
            (Some(m), Some(s)) if m.role == "length" && at.saturating_add(s) > pr.limit => "past-parent",
            _ => "unreadable",
        };
        let m = m.unwrap_or(Measure { role: "length", expr: Expr::Lit(0), unit: 8 });
        let length = match &by {
            Some((j, _)) => {
                let mut p = parent.to_vec();
                p.push(*j);
                p
            }
            None => Vec::new(),
        };
        let check = self.length_check(doc, &length, &child, m, (stated.unwrap_or(0), 0), (at, pr.limit, space_bits), verdict, why)?;
        Ok(Some(ExtentCheck { stated, read: None, ..check }))
    }

    /// Put a check together from what is known about it. `sizes` is the
    /// stated and read size, `place` where the part starts, the end of its
    /// room, and the length of the space.
    #[allow(clippy::too_many_arguments)]
    fn length_check<S: Source>(
        &mut self,
        doc: &Document<S>,
        length: &[usize],
        part: &[usize],
        m: Measure,
        sizes: (u64, u64),
        place: (u64, u64, u64),
        verdict: &'static str,
        why: &str,
    ) -> R<ExtentCheck> {
        let (length_name, length_value, length_invalid, adjusted) = if length.is_empty() {
            (String::new(), None, false, true)
        } else {
            let value = match self.value_of(doc, length) {
                Ok(v) => v.value.as_int(),
                Err(e) if e.interrupted() => return Err(e),
                Err(_) => None,
            };
            let invalid = match self.valid_of(doc, length) {
                Ok(Some(v)) => !v.ok,
                Err(e) if e.interrupted() => return Err(e),
                _ => false,
            };
            let name = self.memo.get(length).map(|r| r.name.text()).unwrap_or_default();
            let adjusted = reads_besides(&m.expr, &name);
            (name, value, invalid, adjusted)
        };
        let part_name = self.memo.get(part).map(|r| r.name.text()).unwrap_or_else(|| match part.split_last() {
            Some((&i, parent)) => self.field_name(parent, i).unwrap_or_else(|| format!("[{i}]")),
            None => String::new(),
        });
        Ok(ExtentCheck {
            length_path: length.to_vec(),
            length_name,
            length_value,
            length_invalid,
            part_path: part.to_vec(),
            part_name,
            role: m.role,
            offset_bits: place.0,
            stated: Some(sizes.0),
            read: Some(sizes.1),
            content_bits: None,
            room_bits: place.1.saturating_sub(place.0),
            space_bits: place.2,
            adjusted,
            verdict,
            why: why.to_string(),
        })
    }

    /// The case of a choice a node took, as declared, where its declaration
    /// is a choice; the declaration itself otherwise, and where the case
    /// cannot be told.
    fn case_declared(&self, decl: &Ty, resolved: &Ty) -> Ty {
        let t = &self.template;
        let mut ty = decl;
        for _ in 0..16 {
            ty = match ty {
                Ty::Named(n) => match t.types.get(&**n) {
                    Some(inner) => inner,
                    None => return decl.clone(),
                },
                Ty::When { inner, .. } | Ty::Origin { inner } => inner,
                Ty::Switch { .. } | Ty::Match { .. } => {
                    let cases: Vec<&Ty> = match ty {
                        Ty::Switch { cases, default, .. } => cases.iter().map(|(_, c)| c).chain(std::iter::once(&**default)).collect(),
                        Ty::Match { cases, default, .. } => cases.iter().map(|(_, c)| c).chain(std::iter::once(&**default)).collect(),
                        _ => unreachable!(),
                    };
                    return match self.case_taken(ty, resolved).and_then(|i| cases.get(i)) {
                        Some(c) => (*c).clone(),
                        None => decl.clone(),
                    };
                }
                _ => return decl.clone(),
            };
        }
        decl.clone()
    }

    /// The name field `i` of the structure at `parent` is declared with.
    fn field_name(&self, parent: &[usize], i: usize) -> Option<String> {
        match self.memo.get(parent)?.ty.base() {
            Ty::Struct(s) => s.fields.get(i).map(|f| f.name.to_string()),
            _ => None,
        }
    }
}

/// A declaration with names and conditions looked through, down to what
/// decides its size.
fn decl_base<'a>(t: &'a Template, ty: &'a Ty) -> &'a Ty {
    let mut ty = ty;
    for _ in 0..16 {
        ty = match ty {
            Ty::Named(n) => match t.types.get(&**n) {
                Some(inner) => inner,
                None => return ty,
            },
            Ty::When { inner, .. } | Ty::Origin { inner } => inner,
            _ => return ty,
        };
    }
    ty
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::Expr as E;

    #[test]
    fn a_cap_at_the_room_left_comes_off() {
        // The WAV template's room for its chunks: the RIFF size less four,
        // the rest of the file when it is zero, and never past the end.
        let room = E::field("size").at_least(E::lit(4)).sub(E::lit(4)).or(E::Remaining).at_most(E::Remaining);
        let e = unclamped(&room);
        assert!(!matches!(e, Expr::Min(..)), "{e:?}");
        assert!(matches!(e, Expr::Or(..)), "a zero size still means the rest: {e:?}");
        // A cap on something that is not the room stays.
        let cap = E::field("a").at_most(E::field("b"));
        assert!(matches!(unclamped(&cap), Expr::Min(..)));
    }

    #[test]
    fn a_length_reading_only_its_field_is_not_adjusted() {
        assert!(!reads_besides(&E::field("size"), "size"));
        assert!(!reads_besides(&E::field("size").sub(E::lit(8)), "size"));
        assert!(reads_besides(&E::field("size").add(E::field("extra")), "size"));
        assert!(reads_besides(&E::field("size").add(E::peek(32, crate::template::Endian::Little)), "size"));
    }

    #[test]
    fn verdicts() {
        assert_eq!(length_verdict(0, 100, 100, 1000), "fits");
        assert_eq!(length_verdict(0, 100, 50, 1000), "past-parent");
        assert_eq!(length_verdict(960, 100, 40, 1000), "past-file");
        assert_eq!(length_verdict(0, 50, 100, 1000), "stretched");
    }
}
