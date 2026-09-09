//! Every timestamp a template declares has to be something the query can read.
//!
//! Two mistakes are possible in a [`Time`](qubero_core::template::Time)
//! declaration, and neither fails loudly. An MS-DOS pair naming a field no
//! structure has answers "no time here", and the panel shows nothing where it
//! ought to show a date. A time declared on a field that is not a number, a
//! text field or a run of bytes, answers nothing for the same reason, or worse
//! reads four letters as a count of seconds. So both are checked here, against
//! the type tree, with no file in hand at all.
//!
//! This is the whole of the static side. Whether an *epoch* is the right one is
//! a question about the format, and no walk over the types can answer it: a
//! wrong epoch shows a confident, wrong date, and only a real file and its
//! documentation settle that. The sibling of `checks_real` for time would be
//! that, and it is not here.

use std::collections::{HashMap, HashSet};

use qubero_core::formats;
use qubero_core::template::{Epoch, Time, Ty};

#[test]
fn every_declared_time_is_readable() {
    let mut declared = 0;
    for name in formats::builtin_names() {
        let Some(t) = formats::builtin(name) else { continue };
        // Most templates declare no times at all, and the walk below is scoped:
        // a named type reachable by five paths is walked five times, because
        // what a name resolves to depends on which path it came by. This pass
        // is not scoped and steps into each named type once, so it settles "is
        // there anything here" for the other two hundred templates in the time
        // the scoped walk takes for one.
        if !any_time(&t.root, &t.types, &mut HashSet::new()) {
            continue;
        }
        let mut w = Walk { name, seen: HashSet::new(), scope: Vec::new(), found: 0 };
        w.ty(&t.root, &t.types);
        declared += w.found;
    }
    // A guard against the walk quietly stopping finding anything. The eight
    // cases the inspector used to hand-write are the floor: gzip, MP4 and BRAW,
    // utmp, cpio, ar and deb, a journal, a region file, a ZIP and a cabinet.
    //
    // The number counts places, not declarations: a time inside a type two
    // formats share is validated once for each way of reaching it, since what
    // its names resolve to depends on which way that was.
    assert!(declared >= 8, "only {declared} time sites found across every template");
    eprintln!("{declared} time sites validated");
}

/// Whether a type tree declares a time anywhere in it, scope disregarded. Each
/// named type is stepped into once and never again, which is what makes this
/// cheap and is why it cannot answer the question the walk below answers.
fn any_time<'a>(ty: &'a Ty, types: &'a HashMap<String, Ty>, seen: &mut HashSet<&'a str>) -> bool {
    match ty {
        Ty::Struct(s) => s.fields.iter().any(|f| f.time.is_some() || any_time(&f.ty, types, seen)),
        Ty::Array { elem, .. } | Ty::Repeat { elem, .. } | Ty::Chain { elem, .. } => any_time(elem, types, seen),
        Ty::PointerList { elem, .. } => any_time(elem, types, seen),
        Ty::Nullable { inner, .. }
        | Ty::At { inner, .. }
        | Ty::Sized { inner, .. }
        | Ty::SizedBits { inner, .. }
        | Ty::Origin { inner }
        | Ty::Decoded { inner, .. }
        | Ty::Enum { inner, .. }
        | Ty::Flags { inner, .. } => any_time(inner, types, seen),
        Ty::Switch { cases, default, .. } => {
            cases.iter().any(|(_, t)| any_time(t, types, seen)) || any_time(default, types, seen)
        }
        Ty::Match { cases, default, .. } => {
            cases.iter().any(|(_, t)| any_time(t, types, seen)) || any_time(default, types, seen)
        }
        Ty::Named(n) => match types.get_key_value(&**n) {
            Some((key, t)) => seen.insert(key) && any_time(t, types, seen),
            None => false,
        },
        _ => false,
    }
}

struct Walk<'a> {
    name: &'a str,
    /// Named types already stepped into, so a format that nests inside itself
    /// is walked once rather than for ever.
    seen: HashSet<&'a str>,
    /// The field names in view, innermost last: this structure's, then the ones
    /// it sits inside. An MS-DOS half may name any of them, since the query
    /// resolves outwards exactly as a check does.
    scope: Vec<&'a [String]>,
    found: usize,
}

impl<'a> Walk<'a> {
    fn ty(&mut self, ty: &'a Ty, types: &'a HashMap<String, Ty>) {
        match ty {
            Ty::Struct(s) => {
                let names: Vec<String> = s.fields.iter().map(|f| f.name.to_string()).collect();
                // Leaked so the scope can hold a slice of it, as in
                // `checks_declared`: a test that runs once is not where an arena
                // belongs.
                self.scope.push(Box::leak(names.into_boxed_slice()));
                for f in &s.fields {
                    if let Some(time) = &f.time {
                        self.found += 1;
                        self.time(&f.name, &f.ty, time);
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

    fn time(&self, field: &str, ty: &Ty, time: &Time) {
        let at = format!("{}: the time on `{field}`", self.name);
        assert!(reads_as_a_number(ty), "{at} is on a field that holds no number");
        if let Epoch::DosHalves { date, time } = &time.epoch {
            self.names(date, &at);
            self.names(time, &at);
        }
    }

    fn names(&self, name: &str, at: &str) {
        assert!(
            self.scope.iter().any(|level| level.iter().any(|f| f == name)),
            "{at} pairs with a field called `{name}`, and no structure it sits in has one"
        );
    }
}

/// Whether the query will get a number out of a field of this type.
///
/// The wrappers are stepped through, since a time on a `present_if` field or on
/// an enumeration is still a time and the number underneath is what is read. A
/// list is allowed and means its elements: see `Field::time`. What is refused
/// is a field the query would read as something other than a count, above all a
/// text or byte field, whose value is its bytes as a big-endian number and
/// would turn four letters into a confident date.
fn reads_as_a_number(ty: &Ty) -> bool {
    match ty {
        Ty::UInt { .. } | Ty::Int { .. } | Ty::SignMagnitude { .. } | Ty::TextInt { .. } => true,
        Ty::UIntExpr { .. } => true,
        Ty::Leb128 { .. } | Ty::Vlq | Ty::Zigzag | Ty::SqliteVarint | Ty::EbmlVint { .. } => true,
        Ty::Computed(_) => true,
        // A list of them, which is what a declaration on an array means.
        Ty::Array { elem, .. } | Ty::Repeat { elem, .. } | Ty::Chain { elem, .. } => reads_as_a_number(elem),
        Ty::PointerList { elem, .. } => reads_as_a_number(elem),
        Ty::Nullable { inner, .. }
        | Ty::At { inner, .. }
        | Ty::Sized { inner, .. }
        | Ty::SizedBits { inner, .. }
        | Ty::Origin { inner }
        | Ty::Enum { inner, .. }
        | Ty::Flags { inner, .. } => reads_as_a_number(inner),
        // One reading of it is enough, and requiring all of them was wrong. A
        // `tar` writes a field it has nothing to put in as zero bytes rather
        // than as digits, and switches on the first byte to say so; an `ar`
        // does the same with spaces. Those cases are genuinely not numbers and
        // the query answers nothing for them, which is the right answer for a
        // field nobody filled in. What matters is that a field with a time on
        // it can be a number at all.
        //
        // The two are not one arm: a `Switch` keys on numbers and a `Match` on
        // strings, so the bindings have different types.
        Ty::Switch { cases, default, .. } => {
            cases.iter().any(|(_, t)| reads_as_a_number(t)) || reads_as_a_number(default)
        }
        Ty::Match { cases, default, .. } => {
            cases.iter().any(|(_, t)| reads_as_a_number(t)) || reads_as_a_number(default)
        }
        _ => false,
    }
}

/// The lint has to be able to fail. A pair naming a field nothing declares is
/// one of the two mistakes it exists to catch, and a walker that quietly found
/// nothing would pass every template for ever.
#[test]
#[should_panic(expected = "no structure it sits in has one")]
fn a_dos_half_naming_a_field_nothing_declares_is_caught() {
    walk(Ty::structure("Made", vec![("date", Ty::u16(qubero_core::template::Endian::Little))])
        .field_time("date", Time::dos_halves("date", "nowhere")));
}

/// And the other: a time on a field that holds no number. Bytes read as a
/// big-endian number, so this one would not error anywhere, it would show a
/// date worked out from four letters.
#[test]
#[should_panic(expected = "holds no number")]
fn a_time_on_a_field_that_is_not_a_number_is_caught() {
    walk(
        Ty::structure("Made", vec![("when", Ty::bytes(qubero_core::template::Expr::lit(4)))])
            .field_time("when", Time::unix()),
    )
}

fn walk(root: Ty) {
    let types = HashMap::new();
    let mut w = Walk { name: "made-up", seen: HashSet::new(), scope: Vec::new(), found: 0 };
    w.ty(&root, &types);
    assert_eq!(w.found, 1);
}
