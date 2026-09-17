//! Whether a field's value is one its format allows, and what to say when it
//! is not.
//!
//! The constraint itself is [`Valid`](crate::template::Valid), declared on the
//! field by whoever wrote the template or by a converter carrying over a
//! Kaitai `valid:` or an ImHex `std::assert`. This works it out against the
//! bytes in front of it.
//!
//! Three rules the whole file keeps.
//!
//! Reading is never affected. The field is read as declared and the value is
//! what the bytes say; the verdict is a second fact about it, the way a magic
//! number's `ok` is. Nothing here refuses to read, and no constraint changes
//! which case a switch takes.
//!
//! A verdict that cannot be worked out is not a verdict. Bytes that have not
//! arrived are `Pending` and the caller asks again, exactly as everywhere
//! else; a bound naming a field this file does not have is no answer and comes
//! back as the refusal it is. What must never happen is `ok: false` because a
//! question could not be asked.
//!
//! A constraint about the wrong kind of value says nothing. `Finite` on an
//! integer, `InEnum` on a field that is not an enum: those are templates
//! saying something they cannot mean, so a debug build says so and a release
//! build passes the value. Marking every integer in a file red because a
//! converter lowered a bound onto the wrong field would be far worse than not
//! checking it.
//!
//! Every expression is worked out where a [`Check`](crate::template::Check)'s
//! expressions are: at the *end* of the structure the field sits in, so a
//! bound may name a field written after it. See
//! [`Covers`](crate::template::Covers).

use std::sync::Arc;

use super::*;
use crate::template::Valid;

/// What came of a field's constraint: whether it holds, and the one line that
/// says why it does not.
///
/// `text` is empty when `ok`. There is nothing for an interface to say about a
/// value the format allows, and a green line beside every field in a file is a
/// line nobody reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidVerdict {
    pub ok: bool,
    /// The reason, state first: `Out of range: must be 1 to 255`. Built here
    /// rather than in an interface so the listing, the chip tooltip, the
    /// inspector and the table say the same words about the same bytes.
    pub text: String,
}

/// How many of a set are listed before the rest are counted. Four names and a
/// count fit a listing row; nine names are a paragraph in a column an inch
/// wide, and a reader counting them has stopped reading the file.
const LISTED: usize = 4;

impl Evaluator {
    /// Whether the value at `path` is one its format allows.
    ///
    /// Nothing when the field declares no constraint, which is nearly every
    /// field. The value is read to answer this, so it costs what showing the
    /// field costs and no more: no constraint reads bytes the field does not
    /// cover, which is what keeps this cheap enough to ask about every row a
    /// listing draws. Checksums are the ones that do, and they are
    /// [`Evaluator::run_check`]'s business, not this one's.
    pub fn valid_of<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Option<ValidVerdict>> {
        let Some((valid, end)) = self.constraint(doc, path)? else { return Ok(None) };
        // The field a bound's `This` means, put back afterwards rather than
        // cleared: a bound may name a field whose own reading asks another
        // question, and an outer `this` has to survive it.
        let was = self.this.replace(path.to_vec());
        let out = self.verdict(doc, path, &end, &valid);
        self.this = was;
        out.map(Some)
    }

    /// The constraint on the field at `path`, and the frame its expressions
    /// are worked out in.
    ///
    /// Either a field of a structure, or an element of a list whose constraint
    /// is declared on the list one level further out: the two are told apart
    /// by what the parent is, the way `check.rs` tells a checksum from a list
    /// of checksums. A declaration on a list is about its elements, since a
    /// run of a million samples has no `Field` per sample to hang one on. See
    /// [`Field::valid`](crate::template::Field::valid).
    fn constraint<S: Source>(
        &mut self,
        doc: &Document<S>,
        path: &[usize],
    ) -> R<Option<(Arc<Valid>, Vec<usize>)>> {
        let Some((&idx, parent)) = path.split_last() else { return Ok(None) };
        self.resolve(doc, path)?;
        let (valid, frame, fields) = match self.memo.get(parent).map(|r| r.ty.clone()) {
            Some(Ty::Struct(s)) => {
                let Some(v) = s.fields.get(idx).and_then(|f| f.valid.clone()) else { return Ok(None) };
                // A declaration on a list is about the elements, so the list
                // itself has no verdict: `these samples are real numbers` says
                // nothing about the run that holds them, and asking the run
                // would compare a count of elements with a bound meant for one
                // of them.
                if matches!(self.memo[path].ty, Ty::Array { .. } | Ty::Repeat { .. }) {
                    return Ok(None);
                }
                (v, parent.to_vec(), s.fields.len())
            }
            // The list is a field of the structure holding it, and the
            // constraint is declared there, so its expressions are worked out
            // at the end of *that* structure: what a bound on an element can
            // name is what the record holding the list can name, and the index
            // the element sits at, which `Expr::Idx` answers.
            Some(Ty::Array { .. } | Ty::Repeat { .. }) => {
                let Some((&list, grandparent)) = parent.split_last() else { return Ok(None) };
                let Some(Ty::Struct(s)) = self.memo.get(grandparent).map(|r| r.ty.clone()) else {
                    return Ok(None);
                };
                let Some(v) = s.fields.get(list).and_then(|f| f.valid.clone()) else { return Ok(None) };
                (v, grandparent.to_vec(), s.fields.len())
            }
            _ => return Ok(None),
        };
        // One past the last field of the structure, so that a name in a bound
        // reaches the whole structure and not only what was written before the
        // field. No node is ever resolved there; it is a place to ask
        // questions from.
        Ok(Some((valid, [&frame[..], &[fields][..]].concat())))
    }

    fn verdict<S: Source>(
        &mut self,
        doc: &Document<S>,
        path: &[usize],
        end: &[usize],
        valid: &Valid,
    ) -> R<ValidVerdict> {
        // `Expr` and `InEnum` are the two that do not compare the value with a
        // number, so they are answered before it is read as one.
        match valid {
            Valid::Expr { expr, msg } => {
                let held = self.eval_expr(doc, end, expr)? != 0;
                return Ok(match held {
                    true => holds(),
                    false => fails(format!("Fails the check: {}", said(expr, msg.as_deref()))),
                });
            }
            Valid::InEnum => return self.in_enum(doc, path),
            Valid::Finite => return self.finite(doc, path),
            _ => {}
        }
        let value = self.value_of(doc, path)?.value;
        // A float is compared as a float. Reading 1.5 as a whole number to
        // compare it would be a bound about a different value, and `1.5 is at
        // least 2` is the kind of answer this file exists to prevent.
        match float_of(&value) {
            Some(f) => self.real_bound(doc, end, valid, f),
            // A structure or a list, whose `as_int` is how many children it
            // has. Comparing that with a bound meant for a value would answer
            // `ok: false` about a count nobody wrote a bound about, which is
            // the one answer this file exists to prevent. `constraint` already
            // keeps a plain list out; this catches a list behind a condition
            // or a window, and a structure a converter put the bound on.
            None if matches!(value, Value::Composite { .. }) => {
                debug_assert!(false, "a bound on {value:?}, which is a count of children rather than a value");
                Ok(holds())
            }
            None => match value.as_int() {
                Some(v) => self.whole_bound(doc, end, valid, v),
                // Text, a run of bytes too long to be a number, a field whose
                // bytes have not arrived. Nothing here is a comparison, so
                // there is nothing to compare.
                None => {
                    debug_assert!(false, "a bound on {value:?}, which is not a number");
                    Ok(holds())
                }
            },
        }
    }

    fn whole_bound<S: Source>(&mut self, doc: &Document<S>, end: &[usize], valid: &Valid, v: i128) -> R<ValidVerdict> {
        Ok(match valid {
            Valid::Eq(e) => {
                let want = self.eval_expr(doc, end, e)?;
                yes_or(v == want, || format!("Must be {want}"))
            }
            Valid::Min(e) => {
                let min = self.eval_expr(doc, end, e)?;
                yes_or(v >= min, || format!("Out of range: must be at least {min}"))
            }
            Valid::Max(e) => {
                let max = self.eval_expr(doc, end, e)?;
                yes_or(v <= max, || format!("Out of range: must be at most {max}"))
            }
            Valid::Range { min, max } => {
                let min = self.eval_expr(doc, end, min)?;
                let max = self.eval_expr(doc, end, max)?;
                yes_or((min..=max).contains(&v), || format!("Out of range: must be {min} to {max}"))
            }
            Valid::AnyOf(items) => {
                let mut allowed = Vec::with_capacity(items.len());
                for e in items {
                    allowed.push(self.eval_expr(doc, end, e)?);
                }
                let ok = allowed.contains(&v);
                yes_or(ok, || one_of(allowed.iter().map(i128::to_string)))
            }
            // Answered before the value was read.
            Valid::Expr { .. } | Valid::InEnum | Valid::Finite => holds(),
        })
    }

    /// The same bounds over a float field. The numbers a bound is written with
    /// are still whole ones in the IR unless the template wrote `Expr::Real`,
    /// so each side is worked out as a real and compared as one.
    fn real_bound<S: Source>(&mut self, doc: &Document<S>, end: &[usize], valid: &Valid, v: f64) -> R<ValidVerdict> {
        Ok(match valid {
            Valid::Eq(e) => {
                let want = self.eval_real_at(doc, end, e, None)?;
                yes_or(v == want, || format!("Must be {want}"))
            }
            Valid::Min(e) => {
                let min = self.eval_real_at(doc, end, e, None)?;
                yes_or(v >= min, || format!("Out of range: must be at least {min}"))
            }
            Valid::Max(e) => {
                let max = self.eval_real_at(doc, end, e, None)?;
                yes_or(v <= max, || format!("Out of range: must be at most {max}"))
            }
            Valid::Range { min, max } => {
                let min = self.eval_real_at(doc, end, min, None)?;
                let max = self.eval_real_at(doc, end, max, None)?;
                yes_or(v >= min && v <= max, || format!("Out of range: must be {min} to {max}"))
            }
            Valid::AnyOf(items) => {
                let mut allowed = Vec::with_capacity(items.len());
                for e in items {
                    allowed.push(self.eval_real_at(doc, end, e, None)?);
                }
                let ok = allowed.contains(&v);
                yes_or(ok, || one_of(allowed.iter().map(f64::to_string)))
            }
            Valid::Expr { .. } | Valid::InEnum | Valid::Finite => holds(),
        })
    }

    /// Whether the value is one the field's enum names.
    ///
    /// The enum already knows: `Value::Enum` carries the name it found, or
    /// none. What this adds is the tier. Without a constraint an unnamed value
    /// is something Qubero has no name for and is marked quietly; with one the
    /// format itself rules it out.
    fn in_enum<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<ValidVerdict> {
        let value = self.value_of(doc, path)?.value;
        let Value::Enum { name, .. } = &value else {
            debug_assert!(false, "`valid in enum` on {value:?}, which is not an enum");
            return Ok(holds());
        };
        if name.is_some() {
            return Ok(holds());
        }
        // The enum's own name, which is what makes the line unambiguous about
        // who does not know the value: `Undefined in ColorType`, not
        // `unknown`. The type is behind whatever wraps it, the way every other
        // question about what a field really is looks through a sentinel.
        let named = match enum_name(&self.memo[path].ty) {
            Some(n) => format!("Unknown or invalid: undefined in {n}"),
            None => "Unknown or invalid: undefined in this enum".to_string(),
        };
        Ok(fails(named))
    }

    /// Whether a float field holds a real number rather than a NaN or an
    /// infinity.
    ///
    /// The two are told apart in the words, because they are different things
    /// to find in a file: a NaN is usually a calculation that went wrong or a
    /// slot nobody filled in, and an infinity is usually a number that
    /// overflowed on its way in.
    fn finite<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<ValidVerdict> {
        let value = self.value_of(doc, path)?.value;
        let Some(f) = float_of(&value) else {
            debug_assert!(false, "`valid finite` on {value:?}, which is not a float");
            return Ok(holds());
        };
        Ok(match (f.is_nan(), f.is_infinite()) {
            (true, _) => fails("Unknown or invalid: not a number".to_string()),
            (_, true) => fails("Unknown or invalid: infinite".to_string()),
            _ => holds(),
        })
    }
}

/// The value read as a float, and only when the field really holds one. An
/// integer is not turned into a double here: a bound on an integer is
/// arithmetic a double would round, and `Finite` said of one is a template
/// bug that has to be caught rather than answered.
fn float_of(v: &Value) -> Option<f64> {
    match v {
        Value::Float(f) => Some(*f),
        Value::Unset(inner) => float_of(inner),
        _ => None,
    }
}

fn enum_name(ty: &Ty) -> Option<String> {
    match ty {
        Ty::Enum { def, .. } => Some(def.name.clone()),
        Ty::Nullable { inner, .. } => enum_name(inner),
        _ => None,
    }
}

/// What a failed check says, which is the message the source wrote down when
/// it wrote one and the expression in the template's own notation otherwise.
/// An expression that notation cannot write is named rather than half written,
/// for the reason `relate` gives: half a formula invites a reader to believe
/// the half they can see.
fn said(e: &Expr, msg: Option<&str>) -> String {
    match msg {
        Some(m) => m.to_string(),
        None => super::write_expr(e).unwrap_or_else(|| "the condition the format states".to_string()),
    }
}

/// The set a value has to be in, listing the first few and counting the rest.
/// A set one longer than the cap is listed whole: `and 1 more` is longer than
/// the name it stands in for.
fn one_of(items: impl ExactSizeIterator<Item = String>) -> String {
    let n = items.len();
    let shown = if n == LISTED + 1 { n } else { LISTED };
    let mut listed: Vec<String> = items.take(shown).collect();
    if n > shown {
        listed.push(format!("and {} more", n - shown));
    }
    format!("Unknown or invalid: must be one of {}", listed.join(", "))
}

fn holds() -> ValidVerdict {
    ValidVerdict { ok: true, text: String::new() }
}

fn fails(text: String) -> ValidVerdict {
    ValidVerdict { ok: false, text }
}

fn yes_or(ok: bool, why: impl FnOnce() -> String) -> ValidVerdict {
    match ok {
        true => holds(),
        false => fails(why()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::MemSource;
    use crate::template::{Endian::Little, Expr as E, Ty as T};

    /// One structure carrying every shape of constraint at once, so each test
    /// reads one field of the same file and the bytes say which case is being
    /// asked about. `limit` is written last on purpose: `ceiling` is bounded
    /// by it, and a bound that could only look backwards could not say so.
    fn template() -> Template {
        let ty = T::structure(
            "Root",
            vec![
                ("exact", T::u8()),
                ("floor", T::u8()),
                ("ceiling_lit", T::u8()),
                ("span", T::u8()),
                ("set", T::u8()),
                ("big_set", T::u8()),
                ("colour", T::enumeration("ColorType", T::u8(), &[(0, "grey"), (2, "rgb")])),
                ("bare", T::u8()),
                ("spoken", T::u8()),
                ("sample", T::F32(Little)),
                ("ceiling", T::u8()),
                ("limit", T::u8()),
            ],
        )
        .field_valid("exact", Valid::Eq(E::lit(5)))
        .field_valid("floor", Valid::Min(E::lit(10)))
        .field_valid("ceiling_lit", Valid::Max(E::lit(10)))
        .field_valid("span", Valid::Range { min: E::lit(1), max: E::lit(9) })
        .field_valid("set", Valid::AnyOf(vec![E::lit(0), E::lit(2), E::lit(3)]))
        .field_valid(
            "big_set",
            Valid::AnyOf(vec![E::lit(0), E::lit(1), E::lit(2), E::lit(3), E::lit(4), E::lit(6)]),
        )
        .field_valid("colour", Valid::InEnum)
        .field_valid("bare", Valid::Expr { expr: E::This.greater_than(E::lit(3)), msg: None })
        .field_valid(
            "spoken",
            Valid::Expr { expr: E::This.greater_than(E::lit(3)), msg: Some("the count starts at four".into()) },
        )
        .field_valid("sample", Valid::Finite)
        .field_valid("ceiling", Valid::Max(E::field("limit")));
        Template::new("t", ty)
    }

    /// A file where every constraint holds. The float is 1.0.
    fn good() -> Vec<u8> {
        let mut v = vec![5, 10, 10, 9, 3, 6, 2, 4, 4];
        v.extend_from_slice(&1.0f32.to_le_bytes());
        v.extend_from_slice(&[7, 7]);
        v
    }

    /// The same file with every value out of bounds, and the float a NaN.
    fn bad() -> Vec<u8> {
        let mut v = vec![6, 9, 11, 10, 1, 5, 1, 3, 3];
        v.extend_from_slice(&f32::NAN.to_le_bytes());
        v.extend_from_slice(&[8, 7]);
        v
    }

    fn verdict(bytes: &[u8], field: usize) -> ValidVerdict {
        let d = Document::new(MemSource(bytes.to_vec()));
        let mut ev = Evaluator::new(template());
        ev.valid_of(&d, &[field]).unwrap().expect("a constraint on this field")
    }

    fn why(bytes: &[u8], field: usize) -> String {
        let v = verdict(bytes, field);
        assert!(!v.ok, "this value should not be allowed");
        v.text
    }

    #[test]
    fn a_value_the_format_allows_passes_and_says_nothing() {
        for field in 0..=10 {
            let v = verdict(&good(), field);
            assert!(v.ok, "field {field}: {}", v.text);
            assert_eq!(v.text, "");
        }
    }

    #[test]
    fn a_field_with_no_constraint_has_no_verdict() {
        let d = Document::new(MemSource(good()));
        let mut ev = Evaluator::new(template());
        assert_eq!(ev.valid_of(&d, &[11]).unwrap(), None);
    }

    #[test]
    fn a_bound_says_which_way_it_was_missed() {
        assert_eq!(why(&bad(), 0), "Must be 5");
        assert_eq!(why(&bad(), 1), "Out of range: must be at least 10");
        assert_eq!(why(&bad(), 2), "Out of range: must be at most 10");
        assert_eq!(why(&bad(), 3), "Out of range: must be 1 to 9");
    }

    /// A set is listed while it is short enough to read and counted after
    /// that, so a row says what is allowed rather than filling with numbers.
    #[test]
    fn a_set_lists_what_is_allowed_and_counts_the_rest() {
        assert_eq!(why(&bad(), 4), "Unknown or invalid: must be one of 0, 2, 3");
        assert_eq!(why(&bad(), 5), "Unknown or invalid: must be one of 0, 1, 2, 3, and 2 more");
        // One past the cap is listed whole: PNG's five colour types.
        assert_eq!(one_of(["0", "2", "3", "4", "6"].into_iter().map(String::from)), "Unknown or invalid: must be one of 0, 2, 3, 4, 6");
    }

    /// The enum's own name, which is what says who has no name for the value.
    #[test]
    fn an_enum_bound_names_the_enum() {
        assert_eq!(why(&bad(), 6), "Unknown or invalid: undefined in ColorType");
    }

    /// A condition is shown as the sentence its source wrote when it wrote
    /// one, and as the expression in the template's own notation otherwise.
    #[test]
    fn a_condition_is_shown_as_what_it_says() {
        assert_eq!(why(&bad(), 7), "Fails the check: this > 3");
        assert_eq!(why(&bad(), 8), "Fails the check: the count starts at four");
    }

    #[test]
    fn a_float_that_is_not_a_number_is_said_to_be_one() {
        assert_eq!(why(&bad(), 9), "Unknown or invalid: not a number");

        let mut infinite = bad();
        infinite[9..13].copy_from_slice(&f32::INFINITY.to_le_bytes());
        assert_eq!(why(&infinite, 9), "Unknown or invalid: infinite");
    }

    /// Worked out at the end of the structure, so a bound may name a field
    /// written after the one it bounds, the way a checksum's coverage may.
    #[test]
    fn a_bound_may_name_a_later_field() {
        assert_eq!(why(&bad(), 10), "Out of range: must be at most 7");
    }

    /// Declared on a list, the constraint is about the elements: a run of
    /// samples has no field per sample to hang one on.
    #[test]
    fn a_constraint_on_a_list_is_about_its_elements() {
        let ty = T::structure("Root", vec![("samples", T::array(T::F32(Little), E::lit(2)))])
            .field_valid("samples", Valid::Finite);
        let mut bytes = 1.5f32.to_le_bytes().to_vec();
        bytes.extend_from_slice(&f32::NAN.to_le_bytes());
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(Template::new("t", ty));
        assert!(ev.valid_of(&d, &[0, 0]).unwrap().unwrap().ok);
        assert_eq!(ev.valid_of(&d, &[0, 1]).unwrap().unwrap().text, "Unknown or invalid: not a number");
        // The list itself is not what the claim is about.
        assert_eq!(ev.valid_of(&d, &[0]).unwrap(), None);
    }

    /// A list behind a condition or a window is still a list, and a structure
    /// is not a value either. Neither is compared with a bound: what
    /// `as_int` answers for both is how many children they have, and a
    /// verdict about that is a verdict about something nobody declared.
    #[test]
    fn a_count_of_children_is_never_what_a_bound_is_compared_with() {
        let ty = T::structure("Root", vec![("run", T::when(E::lit(1), T::array(T::u8(), E::lit(2))))])
            .field_valid("run", Valid::Min(E::lit(5)));
        let d = Document::new(MemSource(vec![5, 6]));
        let mut ev = Evaluator::new(Template::new("t", ty));
        // The elements are what the claim is about, and both hold.
        assert!(ev.valid_of(&d, &[0, 0]).unwrap().unwrap().ok);
        assert!(ev.valid_of(&d, &[0, 1]).unwrap().unwrap().ok);
        // The run itself holds two children, and two is not the value.
        assert!(ev.valid_of(&d, &[0]).unwrap().is_none_or(|v| v.ok));
    }

    /// A value that cannot be read is no verdict at all, and passes the
    /// refusal up the way every other query does. What must never come back
    /// is `ok: false` because the question could not be asked.
    #[test]
    fn a_value_that_cannot_be_read_is_not_a_failure() {
        let d = Document::new(MemSource(Vec::new()));
        let mut ev = Evaluator::new(template());
        assert!(ev.valid_of(&d, &[0]).is_err());
    }
}
