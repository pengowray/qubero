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
use crate::template_text;

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
                // The one thing a joined stream holds is as long as the parts
                // it is joined from, cut where the stream says it ends. Both
                // lengths are written against where they were worked out: the
                // total where the stream is declared, and a part's length in
                // the structure its run is a field of, for which the first
                // part stands, since every part is measured the same way.
                Some(Ty::Stitched { part_len, len, .. }) => {
                    if let Some(len) = len {
                        self.relation(doc, parent, &len, Role::Length, None, &mut out);
                    }
                    if let Some(part_len) = part_len {
                        match self.first_part_frame(doc, parent) {
                            Ok(Some(frame)) => self.relation(doc, &frame.end, &part_len, Role::Length, frame.here, &mut out),
                            Err(e) if e.interrupted() => return Err(e),
                            _ => {}
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
                // Whether the field is here: `flags & 8` reading as `12 & 8 =
                // 8` is the whole of why a row is there or is not. Its own
                // question, not the one a switch asks, and the panel says so.
                // See [`Role::Condition`].
                Ty::When { cond, inner } => {
                    self.relation(doc, path, &cond, Role::Condition, None, &mut out);
                    ty = *inner;
                }
                Ty::Switch { on, .. } | Ty::Match { on, .. } => {
                    self.relation(doc, path, &on, Role::Type, None, &mut out);
                    break;
                }
                // A switch whose cases are in the file: the key as written,
                // as read, and the description it named. See `schema.rs`.
                Ty::Schema { .. } => {
                    out.extend(self.schema_relation(doc, path));
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
            // The same relationship, and what it comes to is the real it
            // works out rather than a whole number it would fail to be.
            Ty::ComputedReal(e) => self.relation_as(doc, path, &e.clone(), Role::Value, None, true, &mut out),
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
        self.relation_as(doc, at, e, role, here, false, out)
    }

    /// The same, saying whether the expression is worked out as reals, which
    /// is the field's to say and not the expression's: see
    /// [`Ty::ComputedReal`]. Only what it comes to depends on it. The leaves
    /// are written in as each reads, a float as the float it is and a count
    /// as the count, whichever way the whole is worked out.
    #[allow(clippy::too_many_arguments)]
    fn relation_as<S: Source>(
        &mut self,
        doc: &Document<S>,
        at: &[usize],
        e: &Expr,
        role: Role,
        here: Option<(u64, u64)>,
        real: bool,
        out: &mut Vec<Relation>,
    ) {
        let Some(written) = write_expr(e) else { return };
        let mut named = false;
        let here = here.or_else(|| self.memo.get(at).map(|r| (r.offset, r.limit)));
        let Ok(Some(substituted)) = self.substitute(doc, at, e, here, &mut named) else { return };
        if !named || substituted == written {
            return;
        }
        let result = match real {
            true => match self.eval_real_at(doc, at, e, here) {
                Ok(v) => real_text(v),
                Err(_) => return,
            },
            false => match self.eval_expr_at(doc, at, e, here) {
                Ok(v) => v.to_string(),
                Err(_) => return,
            },
        };
        // A substitution that already is the answer says the same thing twice.
        if substituted == result {
            return;
        }
        out.push(Relation { role, written, substituted, result });
    }

    /// The same expression with every leaf that reads the file replaced by
    /// what it reads. `named` comes back true when at least one was.
    ///
    /// Everything between the leaves is written by the writer that writes the
    /// IR text, so the two forms of one relationship differ in the leaves and
    /// nowhere else: the same words, the same brackets, the same reading. See
    /// [`crate::template_text::with_leaves`].
    fn substitute<S: Source>(
        &mut self,
        doc: &Document<S>,
        at: &[usize],
        e: &Expr,
        here: Option<(u64, u64)>,
        named: &mut bool,
    ) -> R<Option<String>> {
        // A leaf the writer cannot write and a read that failed both stop the
        // writer, and they are not the same answer: the first is a
        // relationship not worth showing, the second is a file that would not
        // answer. So the error is held here and raised once the writer is
        // done with it.
        let mut failed = None;
        let written = template_text::with_leaves(e, &mut |leaf| match self.leaf_value(doc, at, leaf, here, named) {
            Ok(s) => s,
            Err(e) => {
                failed = Some(e);
                None
            }
        });
        match failed {
            Some(e) => Err(e),
            None => Ok(written),
        }
    }

    /// One leaf with what it read in its place.
    fn leaf_value<S: Source>(
        &mut self,
        doc: &Document<S>,
        at: &[usize],
        e: &Expr,
        here: Option<(u64, u64)>,
        named: &mut bool,
    ) -> R<Option<String>> {
        // A search over a list, where the value alone would hide the half
        // of it this record contributed. `earlier[class_num = 9].name`
        // says what was looked for and where; `"trce"` says only what came
        // back, and leaves the reader to guess which element answered.
        if let Expr::Tagged(t) = e {
            let tag = match &t.tag {
                Tag::Computed(e) => self.substitute(doc, at, &e.clone(), here, named)?,
                // A label that is text: substituting the expression that
                // works it out means the text it came to, since that is
                // what the search was actually given. Leaving it as
                // written would make the two forms the same and the whole
                // relationship would be dropped as saying nothing.
                Tag::ComputedText(e) => {
                    *named = true;
                    Some(format!("{:?}", self.text_at(doc, at, &e.clone(), here)?))
                }
                // A constant the template fixed, which reads the same in both
                // forms and is written the same way in both.
                other => template_text::tag_text(other, false),
            };
            let Some(tag) = tag else { return Ok(None) };
            let field = if t.field.is_empty() { String::new() } else { format!(".{}", t.field.join(".")) };
            *named = true;
            let array = t.array.as_ref().and_then(write_expr).unwrap_or_else(|| "earlier".into());
            return Ok(Some(format!("{array}[{} = {tag}]{field}", t.key.join("."))));
        }
        // Everything else this can write at all reads the file. What it reads
        // is the whole of what substituting it means, so one evaluation covers
        // all of them.
        if write_expr(e).is_none() {
            return Ok(None);
        }
        *named = true;
        // The number a card's text spells, rather than the text or the search
        // that found the card: `2.5 or else 1.0` is the working, and the
        // search is written out in the form above it.
        if let Expr::RealText(_) = e {
            return Ok(Some(real_text(self.eval_real_at(doc, at, e, here)?)));
        }
        // A field that holds a float is written in as the float, which is what
        // its own row shows. Looked at before it is asked as a whole number,
        // since three of the leaves that name a field do not fail on a float
        // there: a walk back and a search pass over one and answer nought,
        // which would write a GRIB reference value into its formula as 0.
        if matches!(
            e,
            Expr::Ref(_) | Expr::Within(_) | Expr::Elem { .. } | Expr::ElemWithin { .. } | Expr::Placer(_) | Expr::Sibling(_) | Expr::Prev(_)
        ) {
            match self.field_value(doc, at, e, here) {
                Ok(super::expr::Leaf::Value(v, _)) if v.as_int().is_none() => {
                    if let Some(f) = super::expr::real_reading(&v) {
                        return Ok(Some(real_text(f)));
                    }
                }
                Err(err) if err.interrupted() => return Err(err),
                // Nothing found, a whole number, or a question asked of a
                // descriptor that is arithmetic rather than a field: the
                // reading below says each of those as it always has.
                _ => {}
            }
        }
        // Everything else as a whole number, so a count keeps every digit an
        // i128 has and a double would round. What will not read as one is
        // tried as a real before it is given up on.
        match self.eval_expr_at(doc, at, e, here) {
            Ok(v) => Ok(Some(v.to_string())),
            Err(err) if err.interrupted() => Err(err),
            Err(err) => match self.eval_real_at(doc, at, e, here) {
                Ok(v) => Ok(Some(real_text(v))),
                Err(_) => Err(err),
            },
        }
    }
}

/// A real as the relations panel writes one in: the shortest digits that read
/// back as the same double, and no point on a whole one, the way a float row
/// reads. The IR text keeps its point on a literal, because there the point is
/// what says a real was written; here every number is a value the file gave.
fn real_text(v: f64) -> String {
    v.to_string()
}

/// The expression as the template writes it. None for the expressions with no
/// reading in this notation: a search for a byte pattern, or a peek at bits
/// that are not a field.
///
/// The IR text, this panel and the diagram's labels are one writer, so an
/// expression reads one way wherever the reader meets it. The words and the
/// brackets are settled in [`crate::template_text`], beside the grammar the
/// rest of the IR is written in.
///
/// Public because a type can hold an expression too: a field as wide as
/// another field says names that field in the type column, and the notation it
/// is named in should be the one every other connection is written in. See
/// [`crate::template::Ty::UIntExpr`].
pub fn write_expr(e: &Expr) -> Option<String> {
    template_text::readable(e)
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

    /// One expression, one reading. The panel, the diagram's labels and the IR
    /// text are one writer, and this is what says so: anything it catches is
    /// two writers that have drifted apart again.
    #[test]
    fn the_panel_writes_what_the_ir_text_writes() {
        let a = || E::field("a");
        let each = [
            E::field("n").pad_to(4),
            E::max_of("rows"),
            E::pop_count("mask"),
            E::prev("length"),
            E::sibling(&["head", "count"]),
            E::within(&["head", "count"]),
            // A mask beside a comparison, which is the case the two writers
            // used to read two ways.
            a().and(E::lit(8)).less_than(E::lit(4)),
            a().and(a().less_than(E::lit(4))),
            a().shr(E::lit(3)).and(E::lit(0x3f)),
            a().equal_to(E::lit(0x4948_4452)).negate(),
            E::cond(a(), E::lit(1), E::cond(a(), E::lit(2), E::lit(3))),
            E::tagged_by_expr("structures", &["class_num"], E::field("class"), &["name"]),
            E::field("n").at_most(E::Remaining).add(E::lit(1)),
        ];
        for e in each {
            assert_eq!(write_expr(&e), Some(crate::template_text::expr(&e)), "{e:?}");
        }
    }

    /// The words themselves, so that changing one changes this line too.
    #[test]
    fn a_call_is_named_the_way_the_ir_names_it() {
        assert_eq!(write_expr(&E::field("n").pad_to(4)).as_deref(), Some("padding(n, 4)"));
        assert_eq!(write_expr(&E::max_of("rows")).as_deref(), Some("largest(rows)"));
        assert_eq!(write_expr(&E::pop_count("mask")).as_deref(), Some("setbits(mask)"));
        assert_eq!(write_expr(&E::prev("length")).as_deref(), Some("previous(length)"));
        // A field of the element before this one, told apart from a field of
        // this one: `count` is here and `earlier(count)` is in the last record.
        assert_eq!(write_expr(&E::sibling(&["count"])).as_deref(), Some("earlier(count)"));
        assert_eq!(write_expr(&E::within(&["head", "count"])).as_deref(), Some("head.count"));
    }

    /// A mask binds tighter than the comparison it is written beside, as it
    /// does in the IR text and in Kaitai, so a bit field beside a number needs
    /// no brackets and a comparison inside a mask keeps them.
    #[test]
    fn a_mask_binds_tighter_than_a_comparison() {
        let a = || E::field("a");
        assert_eq!(write_expr(&a().and(E::lit(8)).less_than(E::lit(4))).as_deref(), Some("a & 8 < 4"));
        assert_eq!(write_expr(&a().and(a().less_than(E::lit(4)))).as_deref(), Some("a & (a < 4)"));
        // And a mask is written in hex, which is the form the bits are read in.
        assert_eq!(write_expr(&a().shr(E::lit(3)).and(E::lit(0x3f))).as_deref(), Some("a >> 3 & 0x3f"));
    }

    #[test]
    fn an_expression_with_no_reading_is_not_half_written() {
        let e = E::Add(Box::new(E::field("a")), Box::new(E::Find { needle: vec![0], last: false }));
        assert_eq!(write_expr(&e), None);
    }
}
