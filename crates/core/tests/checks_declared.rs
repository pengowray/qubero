//! Every checksum a template declares has to name things that are there.
//!
//! A [`Check`](qubero_core::template::Check) names fields: the one holding the
//! bytes it covers, and whichever ones its offsets and its guard are worked out
//! from. A name no structure has is a typo, and a typo does not fail loudly: the
//! query answers "no check here", the panel shows nothing where it used to show
//! a verdict, and nothing anywhere says why. So the names are checked here,
//! against the type tree, with no file in hand at all.
//!
//! This is the whole of the static side. Whether the *coverage* is right is a
//! question about the format and can only be answered against a real file; that
//! is `checks_real`.

use std::collections::{HashMap, HashSet};

use qubero_core::formats;
use qubero_core::template::{Check, Covers, Expr, Ty};

#[test]
fn every_check_names_fields_that_exist() {
    let mut declared = 0;
    for name in formats::builtin_names() {
        let Some(t) = formats::builtin(name) else { continue };
        let mut w = Walk { name, seen: HashSet::new(), scope: Vec::new(), found: 0 };
        w.ty(&t.root, &t.types);
        declared += w.found;
    }
    // A guard against the walk quietly stopping finding anything: the seven
    // the inspector used to hand-write are the floor.
    assert!(declared >= 7, "only {declared} checks found across every template");
    eprintln!("{declared} checks declared");
}

struct Walk<'a> {
    name: &'a str,
    /// Named types already stepped into, so a format that nests inside itself
    /// is walked once rather than for ever.
    seen: HashSet<&'a str>,
    /// The field names in view, innermost last: this structure's, then the ones
    /// it sits inside. A check may name any of them. Pushed and popped rather
    /// than copied, since a template nests a hundred deep and the copy is what
    /// made this take five minutes.
    scope: Vec<&'a [String]>,
    found: usize,
}

impl<'a> Walk<'a> {
    fn ty(&mut self, ty: &'a Ty, types: &'a HashMap<String, Ty>) {
        match ty {
            Ty::Struct(s) => {
                let names: Vec<String> = s.fields.iter().map(|f| f.name.to_string()).collect();
                // Leaked so the scope can hold a slice of it: the walk outlives
                // the loop that builds it, and a test that runs once is not
                // where an arena belongs.
                self.scope.push(Box::leak(names.into_boxed_slice()));
                for f in &s.fields {
                    if let Some(check) = &f.check {
                        self.found += 1;
                        self.check(&f.name, check);
                    }
                    self.ty(&f.ty, types);
                }
                self.scope.pop();
            }
            Ty::Array { elem, .. } | Ty::Repeat { elem, .. } | Ty::Chain { elem, .. } => self.ty(elem, types),
            Ty::PointerList { elem, .. } => self.ty(elem, types),
            Ty::Nullable { inner, .. }
            | Ty::At { inner, .. }
            | Ty::Sized { inner, .. }
            | Ty::SizedBits { inner, .. }
            | Ty::Origin { inner }
            | Ty::Decoded { inner, .. }
            | Ty::Enum { inner, .. }
            | Ty::Flags { inner, .. } => self.ty(inner, types),
            Ty::Switch { cases, default, .. } => {
                for (_, t) in cases.iter() {
                    self.ty(t, types);
                }
                self.ty(default, types);
            }
            Ty::Match { cases, default, .. } => {
                for (_, t) in cases.iter() {
                    self.ty(t, types);
                }
                self.ty(default, types);
            }
            Ty::Named(n) => {
                if let Some((key, t)) = types.get_key_value(&**n) {
                    if self.seen.insert(key) {
                        self.ty(t, types);
                        self.seen.remove(key.as_str());
                    }
                }
            }
            _ => {}
        }
    }

    fn check(&self, field: &str, check: &Check) {
        let at = format!("{}: the check on `{field}`", self.name);
        if let Some(when) = &check.when {
            self.expr(when, &format!("{at}, in its guard"));
        }
        match &check.over {
            Covers::UpToHere => {}
            Covers::Run { at: start, len } => {
                self.expr(start, &format!("{at}, in where the run starts"));
                self.expr(len, &format!("{at}, in how long the run is"));
            }
            Covers::Field { name } => self.names(name, &at),
            Covers::Unpacked { name, len } => {
                self.names(name, &at);
                if let Some(len) = len {
                    self.expr(len, &format!("{at}, in the unpacked length"));
                }
            }
        }
    }

    fn names(&self, name: &str, at: &str) {
        assert!(
            self.scope.iter().any(|level| level.iter().any(|f| f == name)),
            "{at} covers a field called `{name}`, and no structure it sits in has one"
        );
    }

    /// Every field an expression reads has to be in view too. Only the forms
    /// that name a field are looked at; the arithmetic around them is walked
    /// through to reach them.
    fn expr(&self, e: &Expr, at: &str) {
        let two = |a: &Expr, b: &Expr| {
            self.expr(a, at);
            self.expr(b, at);
        };
        match e {
            Expr::Ref(n) | Expr::SizeOf(n) | Expr::BitsOf(n) | Expr::ProductOf(n) | Expr::SumOf(n) | Expr::MaxOf(n) => {
                self.names(n, at)
            }
            Expr::Elem { array, index, .. } | Expr::Product { array, index, .. } => {
                self.names(array, at);
                self.expr(index, at);
            }
            Expr::Within(path) | Expr::Sibling(path) => {
                if let Some(head) = path.first() {
                    self.names(head, at);
                }
            }
            Expr::Or(a, b)
            | Expr::Add(a, b)
            | Expr::Sub(a, b)
            | Expr::Mul(a, b)
            | Expr::Div(a, b)
            | Expr::Less(a, b)
            | Expr::Shl(a, b)
            | Expr::Shr(a, b)
            | Expr::And(a, b)
            | Expr::Min(a, b)
            | Expr::Max(a, b) => two(a, b),
            Expr::PadTo { n, .. } => self.expr(n, at),
            Expr::Bit(a, _) => self.expr(a, at),
            Expr::PeekAt { skip, .. } => self.expr(skip, at),
            // `Remaining` has nothing to measure from where a check's
            // expressions are worked out, which is past the last field of the
            // structure rather than at one of them.
            Expr::Remaining => panic!("{at} measures what is left, which a check cannot ask"),
            _ => {}
        }
    }
}

/// The lint has to be able to fail. A check naming a field nothing declares is
/// the mistake it exists to catch, and a walker that quietly found nothing
/// would pass every template for ever.
#[test]
#[should_panic(expected = "no structure it sits in has one")]
fn a_name_nothing_declares_is_caught() {
    walk_one(Covers::Field { name: "nowhere".into() }, None);
}

/// And so does a guard reading a field that is not there, which is the same
/// mistake one level in.
#[test]
#[should_panic(expected = "in its guard")]
fn a_guard_reading_a_field_that_is_not_there_is_caught() {
    walk_one(Covers::Field { name: "a".into() }, Some(Expr::field("method").equals(Expr::lit(1))));
}

/// A structure of two fields with one check on it, walked the way a real
/// template is.
fn walk_one(over: Covers, when: Option<Expr>) {
    use qubero_core::template::{Checksum, Ty as T};
    let root = T::structure("Made", vec![("a", T::u8()), ("sum", T::u8())])
        .field_check("sum", Check { algorithm: Checksum::Sum8, over, when });
    let types = HashMap::new();
    let mut w = Walk { name: "made-up", seen: HashSet::new(), scope: Vec::new(), found: 0 };
    w.ty(&root, &types);
    assert_eq!(w.found, 1);
}
