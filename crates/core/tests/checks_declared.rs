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
use qubero_core::template::{Check, Checksum, Covers, Expr, Named, Ty, Ty as T};

#[test]
fn every_check_names_fields_that_exist() {
    let mut declared = 0;
    for name in formats::builtin_names() {
        let Some(t) = formats::builtin(name) else { continue };
        // Most templates declare nothing to check, and the walk below is
        // scoped: a named type reachable by five paths is walked five times,
        // because what a check may name depends on which path it came by. This
        // pass is not scoped and steps into each named type once, so it settles
        // "is there anything here" for the other two hundred templates in the
        // time the scoped walk takes for one.
        if !any_check(&t.root, &t.types, &mut HashSet::new()) {
            continue;
        }
        let mut names = HashSet::new();
        every_name(&t.root, &t.types, &mut HashSet::new(), &mut names);
        let mut w = Walk { name, declared: names, seen: HashSet::new(), scope: Vec::new(), found: 0 };
        w.ty(&t.root, &t.types);
        declared += w.found;
    }
    // A guard against the walk quietly stopping finding anything: the seven
    // the inspector used to hand-write are the floor.
    //
    // The number counts places, not declarations: a check inside a type two
    // formats share is validated once for each way of reaching it, since what
    // its names resolve to depends on which way that was. That is the point of
    // the scoped walk and is why the number is far larger than the count of
    // `field_check` calls in the templates.
    assert!(declared >= 7, "only {declared} check sites found across every template");
    eprintln!("{declared} check sites validated");
}

/// Every field name a type tree declares, scope disregarded. Each named type
/// is stepped into once, as in [`any_check`]: a name is a name wherever the
/// walk reached it from.
fn every_name<'a>(
    ty: &'a Ty,
    types: &'a HashMap<String, Ty>,
    seen: &mut HashSet<&'a str>,
    out: &mut HashSet<&'a str>,
) {
    match ty {
        Ty::Struct(s) => {
            for f in &s.fields {
                out.insert(&f.name);
                every_name(&f.ty, types, seen, out);
            }
        }
        Ty::Array { elem, .. } | Ty::Repeat { elem, .. } | Ty::Chain { elem, .. } => every_name(elem, types, seen, out),
        Ty::PointerList { elem, .. } => every_name(elem, types, seen, out),
        Ty::Nullable { inner, .. }
        | Ty::At { inner, .. }
        | Ty::Sized { inner, .. }
        | Ty::SizedBits { inner, .. }
        | Ty::Origin { inner }
        | Ty::Decoded { inner, .. }
        | Ty::Enum { inner, .. }
        | Ty::Flags { inner, .. } => every_name(inner, types, seen, out),
        Ty::Switch { cases, default, .. } => {
            for (_, t) in cases.iter() {
                every_name(t, types, seen, out);
            }
            every_name(default, types, seen, out);
        }
        Ty::Named(n) => {
            if let Some((key, t)) = types.get_key_value(&**n)
                && seen.insert(key)
            {
                every_name(t, types, seen, out);
            }
        }
        _ => {}
    }
}

/// Whether a type tree holds a check anywhere in it, scope disregarded. Each
/// named type is stepped into once and never again, which is what makes this
/// cheap and is why it cannot answer the question the walk below answers.
fn any_check<'a>(ty: &'a Ty, types: &'a HashMap<String, Ty>, seen: &mut HashSet<&'a str>) -> bool {
    match ty {
        Ty::Struct(s) => s.fields.iter().any(|f| !f.checks.is_empty() || any_check(&f.ty, types, seen)),
        Ty::Array { elem, .. } | Ty::Repeat { elem, .. } | Ty::Chain { elem, .. } => any_check(elem, types, seen),
        Ty::PointerList { elem, .. } => any_check(elem, types, seen),
        Ty::Nullable { inner, .. }
        | Ty::At { inner, .. }
        | Ty::Sized { inner, .. }
        | Ty::SizedBits { inner, .. }
        | Ty::Origin { inner }
        | Ty::Decoded { inner, .. }
        | Ty::Enum { inner, .. }
        | Ty::Flags { inner, .. } => any_check(inner, types, seen),
        Ty::Switch { cases, default, .. } => {
            cases.iter().any(|(_, t)| any_check(t, types, seen)) || any_check(default, types, seen)
        }
        Ty::Match { cases, default, .. } => {
            cases.iter().any(|(_, t)| any_check(t, types, seen)) || any_check(default, types, seen)
        }
        Ty::Named(n) => match types.get_key_value(&**n) {
            Some((key, t)) => seen.insert(key) && any_check(t, types, seen),
            None => false,
        },
        _ => false,
    }
}

struct Walk<'a> {
    name: &'a str,
    /// Every field name the template declares, anywhere. Only a backwards
    /// reach uses it: see [`Walk::named`].
    declared: HashSet<&'a str>,
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
                    // Every way the field can be checked, not just the first:
                    // a field that holds whichever sum the header named
                    // declares one per algorithm, and a typo in the third is
                    // as invisible as a typo in the first.
                    for check in &f.checks {
                        self.found += 1;
                        self.check(&f.name, check);
                    }
                    // A list of sums declares one check for all its elements,
                    // and it is validated in the same scope: what an element
                    // can name is what the record holding the list can name.
                    if let Some(check) = &f.elem_check {
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
        // A blank is the check field's own bytes read as something else while
        // the sum runs, so it means nothing unless those bytes are among the
        // summed ones. Only a run of the enclosing structure can hold them:
        // `UpToHere` stops where the field starts, and `Field` and `Unpacked`
        // are over some other field entirely. The evaluator refuses those, and
        // refusing is how a check disappears with nothing said anywhere.
        assert!(
            check.blank.is_none() || matches!(check.over, Covers::Run { .. }),
            "{at} reads its own bytes as a blank, over something that cannot hold them"
        );
        match &check.over {
            Covers::UpToHere => {}
            Covers::Run { at: start, len } => {
                self.expr(start, &format!("{at}, in where the run starts"));
                self.expr(len, &format!("{at}, in how long the run is"));
            }
            Covers::Field { name } => self.named(name, &at),
            Covers::Unpacked { name, len } => {
                self.named(name, &at);
                if let Some(len) = len {
                    self.expr(len, &format!("{at}, in the unpacked length"));
                }
            }
            // Two names, and both have to be there: the run that gets opened,
            // and the member's own packed bytes that a reader is sent to.
            Covers::UnpackedMember { name, packed } => {
                self.named(name, &at);
                self.named(packed, &format!("{at}, in the member's own run"));
            }
        }
    }

    /// A name a check covers, checked as far as where it looks allows.
    ///
    /// A name resolved outwards is checked against the scope this walk is
    /// carrying, which is exactly what the evaluator will search. One resolved
    /// *backwards* cannot be: the field is in another element of the enclosing
    /// list, and which element that is depends on the file. So each segment is
    /// checked against every field name the template declares anywhere, which
    /// still catches the mistake this file exists to catch, a typo, and does
    /// not pretend to catch more.
    fn named(&self, name: &Named, at: &str) {
        match name {
            Named::Here(name) => self.names(name, at),
            Named::Earlier(path) => {
                assert!(!path.is_empty(), "{at} reaches back to a field with no name");
                for seg in path.iter() {
                    assert!(
                        self.declared.contains(seg.as_str()),
                        "{at} reaches back through `{seg}`, and no structure in this template declares one"
                    );
                }
            }
            // The first name is reached the way `Within` reaches one, so it
            // has to be in view here; the rest go down inside it, where this
            // walk cannot follow, and are checked against every name the
            // template declares as a backwards reach is.
            Named::Elem { array, index } => {
                let Some((first, rest)) = array.split_first() else {
                    panic!("{at} covers an element of a list with no name");
                };
                self.names(first, at);
                for seg in rest {
                    assert!(
                        self.declared.contains(seg.as_str()),
                        "{at} covers an element reached through `{seg}`, and no structure in this template declares one"
                    );
                }
                self.expr(index, &format!("{at}, in which element it covers"));
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
            | Expr::DivCeil(a, b)
            | Expr::Less(a, b)
            | Expr::Shl(a, b)
            | Expr::Shr(a, b)
            | Expr::And(a, b)
            | Expr::Min(a, b)
            | Expr::Max(a, b) => two(a, b),
            Expr::PadTo { n, .. } => self.expr(n, at),
            Expr::Log2(a) => self.expr(a, at),
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
    walk_one(Covers::Field { name: Named::here("nowhere") }, None);
}

/// And so does a guard reading a field that is not there, which is the same
/// mistake one level in.
#[test]
#[should_panic(expected = "in its guard")]
fn a_guard_reading_a_field_that_is_not_there_is_caught() {
    walk_one(Covers::Field { name: Named::here("a") }, Some(Expr::field("method").equals(Expr::lit(1))));
}

/// And so does a blank over coverage that cannot hold the check field. That
/// one the evaluator refuses in silence, which is exactly the shape of mistake
/// this file exists to make loud.
#[test]
#[should_panic(expected = "over something that cannot hold them")]
fn a_blank_over_coverage_that_cannot_hold_the_field_is_caught() {
    walk(Check::of(Checksum::Sum8, Covers::Field { name: Named::here("a") }).blanking(0));
}

/// And a backwards reach through a name nothing in the template declares.
#[test]
#[should_panic(expected = "and no structure in this template declares one")]
fn a_backwards_reach_through_a_name_nothing_declares_is_caught() {
    walk(Check::of(Checksum::Sum8, Covers::Field { name: Named::earlier(&["nowhere"]) }));
}

/// And an element check reaching a list nothing declares.
#[test]
#[should_panic(expected = "no structure it sits in has one")]
fn an_element_check_over_a_list_nothing_declares_is_caught() {
    walk(Check::of(Checksum::Sum8, Covers::Field { name: Named::elem(&["nowhere"], Expr::Idx) }));
}

fn walk_one(over: Covers, when: Option<Expr>) {
    let check = Check::of(Checksum::Sum8, over);
    walk(match when {
        Some(when) => check.only_when(when),
        None => check,
    });
}

/// A structure of two fields with one check on it, walked the way a real
/// template is.
fn walk(check: Check) {
    let root = T::structure("Made", vec![("a", T::u8()), ("sum", T::u8())]).field_check("sum", check);
    let types = HashMap::new();
    let mut names = HashSet::new();
    every_name(&root, &types, &mut HashSet::new(), &mut names);
    let mut w = Walk { name: "made-up", declared: names, seen: HashSet::new(), scope: Vec::new(), found: 0 };
    w.ty(&root, &types);
    assert_eq!(w.found, 1);
}
