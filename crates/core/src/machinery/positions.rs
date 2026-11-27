//! Which fields hold a position, worked out once per template.
//!
//! [`super::locators`] answers this for one structure on its own, from what
//! its later fields read. A position is often read from somewhere else: a
//! `PointerList` reads `offset` inside every element of a table of section
//! headers, a `Chain` reads `next` inside every record it walks, a `Gather`
//! reads `offset` inside every record its steps reach. The field is in one
//! structure and the reader is in another, so this walks the whole template
//! and marks the structure the field is in.
//!
//! The answer is kept per structure, keyed by the `Arc` that holds it, so a
//! node read asks one hash lookup. A structure the walk did not reach, which
//! is one a `Ty::Schema` builds while the file is read, is walked the first
//! time it is asked about.

use std::sync::Arc;

use rustc_hash::FxHashMap;

use crate::template::{Expr, Step, StructDef, Template, Ty};

/// The field's own value is a position.
pub const AT: u8 = 1;
/// The field is a list and each of its elements is a position.
pub const EACH: u8 = 2;

/// How deep [`structs_here`] and [`elems_here`] follow named types into named
/// types. A template that names a type as itself would otherwise never end.
const NAMED_DEPTH: u32 = 16;

/// How many structures built while the file is read are kept before they are
/// all dropped. Some are built once per node, such as one `Symbol` for every
/// symbol of a traced deflate stream, and keeping each one would hold every
/// one of them for as long as the reading lasts.
const BUILT_CAP: usize = 4096;

struct Marks {
    /// Held so that the key, the address of this structure, cannot be reused
    /// by another structure while the entry is here.
    _def: Arc<StructDef>,
    flags: Box<[u8]>,
    /// Whether this structure's own fields have been read for what they
    /// place. A structure can be marked from outside before that.
    walked: bool,
    /// [`super::consumers`] for this structure, worked out the first time a
    /// node read asks.
    consumers: Option<Box<[Option<usize>]>>,
    /// Whether the walk of the whole template reached this structure. The
    /// others were built while the file was read, and are dropped in bulk
    /// past [`BUILT_CAP`].
    kept: bool,
}

/// The positions in one template's structures. See the module note.
#[derive(Default)]
pub struct Positions {
    by_def: FxHashMap<usize, Marks>,
    filled: bool,
    /// How many entries the walk of the whole template made.
    kept: usize,
}

fn key(def: &Arc<StructDef>) -> usize {
    Arc::as_ptr(def) as usize
}

impl Positions {
    /// Every structure the template reaches, walked now.
    pub fn of(t: &Template) -> Positions {
        let mut p = Positions::default();
        p.fill(t);
        p
    }

    fn fill(&mut self, t: &Template) {
        if self.filled {
            return;
        }
        self.filled = true;
        let mut w = Walk { t, p: self, scope: Vec::new(), placers: Vec::new(), named: Default::default() };
        w.ty(&t.root, 0);
        let mut names: Vec<&String> = t.types.keys().collect();
        names.sort();
        for n in names {
            w.ty(&t.types[n], 0);
        }
        for m in self.by_def.values_mut() {
            m.kept = true;
        }
        self.kept = self.by_def.len();
    }

    /// [`AT`] and [`EACH`] for field `i` of `def`.
    pub fn flags(&mut self, t: &Template, def: &Arc<StructDef>, i: usize) -> u8 {
        self.ensure(t, def).flags.get(i).copied().unwrap_or(0)
    }

    /// [`super::consumers`] for field `i` of `def`, worked out once per
    /// structure.
    pub fn consumer(&mut self, t: &Template, def: &Arc<StructDef>, i: usize) -> Option<usize> {
        let m = self.ensure(t, def);
        m.consumers.get_or_insert_with(|| super::consumers(def).into_boxed_slice()).get(i).copied().flatten()
    }

    fn ensure(&mut self, t: &Template, def: &Arc<StructDef>) -> &mut Marks {
        self.fill(t);
        if !self.by_def.get(&key(def)).is_some_and(|m| m.walked) {
            if self.by_def.len() > self.kept + BUILT_CAP {
                self.by_def.retain(|_, m| m.kept);
            }
            let mut w = Walk { t, p: self, scope: Vec::new(), placers: Vec::new(), named: Default::default() };
            w.def(def);
        }
        self.by_def.get_mut(&key(def)).expect("walked")
    }

    fn entry(&mut self, def: &Arc<StructDef>) -> &mut Marks {
        self.by_def.entry(key(def)).or_insert_with(|| Marks {
            _def: def.clone(),
            flags: vec![0; def.fields.len()].into_boxed_slice(),
            walked: false,
            consumers: None,
            kept: false,
        })
    }

    fn mark(&mut self, def: &Arc<StructDef>, i: usize, flag: u8) {
        if let Some(f) = self.entry(def).flags.get_mut(i) {
            *f |= flag;
        }
    }
}

/// A position named from a field: `head` is a field the reader can see,
/// `rest` goes down into it, and `elem`, when there is one, says the field
/// landed on is a list whose elements hold the position, at that path inside
/// each element (empty when the elements are the numbers).
struct Term<'a> {
    head: &'a str,
    rest: &'a [String],
    elem: Option<&'a [String]>,
}

/// The terms a position expression is made of by adding them. See
/// [`super::locators`] for what is left out and why.
fn terms<'a>(e: &'a Expr, out: &mut Vec<Term<'a>>) {
    match e {
        Expr::Ref(n) => out.push(Term { head: n, rest: &[], elem: None }),
        // `offsets[i]` and `tensors[i].offset`.
        Expr::Elem { array, field, .. } => out.push(Term { head: array, rest: &[], elem: Some(field) }),
        Expr::ElemWithin { path, field, .. } if !path.is_empty() => {
            out.push(Term { head: &path[0], rest: &path[1..], elem: Some(field) })
        }
        // A path into an earlier field, down to the number.
        Expr::Within(path) if !path.is_empty() => out.push(Term { head: &path[0], rest: &path[1..], elem: None }),
        // A path of one step is read as a sibling's name. A longer one starts
        // in an earlier element of the enclosing list, which is not a field of
        // this structure, and is left out.
        Expr::Sibling(path) if path.len() == 1 => out.push(Term { head: &path[0], rest: &[], elem: None }),
        // A field of one element added to another field of the same element
        // is that element's position plus a length: Arrow's last block ends
        // at `offset + metaDataLength + bodyLength` of that block. So only the
        // first field read from an element counts, and the ones added after
        // it are lengths.
        Expr::Add(a, b) => {
            let from = out.len();
            terms(a, out);
            let left = out.len();
            terms(b, out);
            let mut k = left;
            while k < out.len() {
                let same = |t: &Term, u: &Term| t.head == u.head && t.rest == u.rest;
                let added = &out[k];
                if added.elem.is_some_and(|p| !p.is_empty()) && out[from..left].iter().any(|t| t.elem.is_some_and(|p| !p.is_empty()) && same(t, added)) {
                    out.remove(k);
                } else {
                    k += 1;
                }
            }
        }
        Expr::Min(a, b) | Expr::Max(a, b) | Expr::Or(a, b) => {
            terms(a, out);
            terms(b, out);
        }
        Expr::Sub(a, _) => terms(a, out),
        Expr::Cond { then, otherwise, .. } => {
            terms(then, out);
            terms(otherwise, out);
        }
        _ => {}
    }
}

/// The structures a value of this type is, past the wrappers that say where
/// it is, how big it is or how it is packed, and through every case of a
/// switch. Not into a list: a step into a list is a step of its own.
fn structs_here<'a>(t: &'a Template, ty: &'a Ty, depth: u32, out: &mut Vec<&'a Arc<StructDef>>) {
    match ty {
        Ty::Struct(s) => out.push(s),
        Ty::Sized { inner, .. }
        | Ty::SizedBits { inner, .. }
        | Ty::Origin { inner }
        | Ty::At { inner, .. }
        | Ty::Nullable { inner, .. }
        | Ty::Decoded { inner, .. }
        | Ty::When { inner, .. }
        | Ty::Enum { inner, .. }
        | Ty::Flags { inner, .. } => structs_here(t, inner, depth, out),
        Ty::Switch { cases, default, .. } => {
            for (_, c) in cases.iter() {
                structs_here(t, c, depth, out);
            }
            structs_here(t, default, depth, out);
        }
        Ty::Match { cases, default, .. } => {
            for (_, c) in cases.iter() {
                structs_here(t, c, depth, out);
            }
            structs_here(t, default, depth, out);
        }
        Ty::Named(n) if depth < NAMED_DEPTH => {
            if let Some(inner) = t.types.get(&**n) {
                structs_here(t, inner, depth + 1, out);
            }
        }
        _ => {}
    }
}

/// The element types of the lists a value of this type is, past the same
/// wrappers as [`structs_here`].
fn elems_here<'a>(t: &'a Template, ty: &'a Ty, depth: u32, out: &mut Vec<&'a Ty>) {
    match ty {
        Ty::Array { elem, .. }
        | Ty::Repeat { elem, .. }
        | Ty::PointerList { elem, .. }
        | Ty::Chain { elem, .. }
        | Ty::Gather { elem, .. } => out.push(elem),
        Ty::Sized { inner, .. }
        | Ty::SizedBits { inner, .. }
        | Ty::Origin { inner }
        | Ty::At { inner, .. }
        | Ty::Nullable { inner, .. }
        | Ty::Decoded { inner, .. }
        | Ty::When { inner, .. } => elems_here(t, inner, depth, out),
        Ty::Switch { cases, default, .. } => {
            for (_, c) in cases.iter() {
                elems_here(t, c, depth, out);
            }
            elems_here(t, default, depth, out);
        }
        Ty::Match { cases, default, .. } => {
            for (_, c) in cases.iter() {
                elems_here(t, c, depth, out);
            }
            elems_here(t, default, depth, out);
        }
        Ty::Named(n) if depth < NAMED_DEPTH => {
            if let Some(inner) = t.types.get(&**n) {
                elems_here(t, inner, depth + 1, out);
            }
        }
        _ => {}
    }
}

/// The type of field `name` in each structure `ty` is.
fn field_tys<'a>(t: &'a Template, ty: &'a Ty, name: &str, out: &mut Vec<&'a Ty>) {
    let mut ss = Vec::new();
    structs_here(t, ty, 0, &mut ss);
    for s in ss {
        if let Some(f) = s.fields.iter().find(|f| &*f.name == name) {
            out.push(&f.ty);
        }
    }
}

struct Walk<'a, 'p> {
    t: &'a Template,
    p: &'p mut Positions,
    /// The structures around the one being walked, outermost first, for the
    /// first step of a gather, which may name a field of any of them.
    scope: Vec<&'a Arc<StructDef>>,
    /// The record types each gather around the one being walked reads its
    /// offsets from, innermost last, for a gather that starts at
    /// [`Step::Placer`].
    placers: Vec<Vec<&'a Ty>>,
    /// The named types this walk has gone into already. Everything under one
    /// is walked the first time, so going in again finds nothing new.
    named: rustc_hash::FxHashSet<&'a str>,
}

impl<'a> Walk<'a, '_> {
    /// Every structure under `ty`, walked once each.
    fn ty(&mut self, ty: &'a Ty, depth: u32) {
        match ty {
            Ty::Struct(s) => {
                if !self.p.by_def.get(&key(s)).is_some_and(|m| m.walked) {
                    self.def(s);
                }
            }
            Ty::Named(n) if depth < NAMED_DEPTH && self.named.insert(&**n) => {
                if let Some(inner) = self.t.types.get(&**n) {
                    self.ty(inner, depth + 1);
                }
            }
            Ty::Gather { elem, .. } => self.ty(elem, depth),
            Ty::Array { elem, .. } | Ty::Repeat { elem, .. } | Ty::PointerList { elem, .. } | Ty::Chain { elem, .. } => {
                self.ty(elem, depth)
            }
            Ty::Sized { inner, .. }
            | Ty::SizedBits { inner, .. }
            | Ty::Origin { inner }
            | Ty::At { inner, .. }
            | Ty::Nullable { inner, .. }
            | Ty::Decoded { inner, .. }
            | Ty::When { inner, .. }
            | Ty::Enum { inner, .. }
            | Ty::Flags { inner, .. }
            | Ty::Stitched { inner, .. } => self.ty(inner, depth),
            Ty::Raster { pixel, .. } => self.ty(pixel, depth),
            Ty::Switch { cases, default, .. } => {
                for (_, c) in cases.iter() {
                    self.ty(c, depth);
                }
                self.ty(default, depth);
            }
            Ty::Match { cases, default, .. } => {
                for (_, c) in cases.iter() {
                    self.ty(c, depth);
                }
                self.ty(default, depth);
            }
            _ => {}
        }
    }

    /// Marks what the fields of `def` read as positions, then walks every
    /// structure inside them.
    fn def(&mut self, def: &'a Arc<StructDef>) {
        self.p.entry(def).walked = true;
        for n in &def.positions {
            if let Some(i) = def.fields.iter().position(|f| f.name == *n) {
                // A declared list of numbers, such as the chunk offsets of an
                // MP4 `stco`, holds a position in each element.
                let mut es = Vec::new();
                elems_here(self.t, &def.fields[i].ty, 0, &mut es);
                let numbers = !es.is_empty() && es.iter().all(|e| {
                    let (mut ss, mut inner) = (Vec::new(), Vec::new());
                    structs_here(self.t, e, 0, &mut ss);
                    elems_here(self.t, e, 0, &mut inner);
                    ss.is_empty() && inner.is_empty()
                });
                self.p.mark(def, i, if numbers { AT | EACH } else { AT });
            }
        }
        let mut ts = Vec::new();
        for (i, f) in def.fields.iter().enumerate() {
            ts.clear();
            self.reads(def, i, &f.ty, &mut ts);
            for term in &ts {
                // A field is written before the fields that read it, so only
                // earlier siblings are candidates.
                if let Some(j) = def.fields[..i].iter().position(|g| *g.name == *term.head) {
                    self.land_sibling(def, j, term);
                }
            }
        }
        self.scope.push(def);
        for f in &def.fields {
            self.ty(&f.ty, 0);
        }
        self.scope.pop();
    }

    /// The sibling terms field `i`'s type reads as positions, and the marks
    /// it makes inside other structures directly: a chain's `next` inside its
    /// elements, a gather's offset inside its records.
    fn reads(&mut self, def: &'a Arc<StructDef>, i: usize, ty: &'a Ty, out: &mut Vec<Term<'a>>) {
        match ty {
            Ty::At { at, inner, .. } => {
                terms(at, out);
                self.reads(def, i, inner, out);
            }
            Ty::PointerList { offsets, field, adjust, elem, .. } => {
                out.push(Term { head: offsets, rest: &[], elem: Some(field) });
                terms(adjust, out);
                self.reads(def, i, elem, out);
            }
            Ty::Chain { first, next, adjust, elem, .. } => {
                terms(first, out);
                terms(adjust, out);
                self.mark_path(elem, next, AT);
                self.reads(def, i, elem, out);
            }
            Ty::Gather { from, offset, adjust, elem, .. } => {
                terms(adjust, out);
                let records = self.records(def, i, from);
                let mut ot = Vec::new();
                terms(offset, &mut ot);
                for r in &records {
                    let mut ss = Vec::new();
                    structs_here(self.t, r, 0, &mut ss);
                    for s in ss {
                        for term in &ot {
                            if let Some(j) = s.fields.iter().position(|g| *g.name == *term.head) {
                                self.land_sibling(s, j, term);
                            }
                        }
                    }
                }
                // The records are what a `Placer` step inside the children
                // starts from.
                self.placers.push(records);
                self.reads(def, i, elem, out);
                self.ty(elem, 0);
                self.placers.pop();
            }
            Ty::Array { elem, .. } | Ty::Repeat { elem, .. } => self.reads(def, i, elem, out),
            Ty::Sized { inner, .. }
            | Ty::Origin { inner }
            | Ty::When { inner, .. }
            | Ty::Enum { inner, .. }
            | Ty::Flags { inner, .. }
            | Ty::Nullable { inner, .. } => self.reads(def, i, inner, out),
            Ty::Switch { cases, default, .. } => {
                for (_, c) in cases.iter() {
                    self.reads(def, i, c, out);
                }
                self.reads(def, i, default, out);
            }
            Ty::Match { cases, default, .. } => {
                for (_, c) in cases.iter() {
                    self.reads(def, i, c, out);
                }
                self.reads(def, i, default, out);
            }
            // A structure's fields name their own siblings, and are walked
            // when the structure is.
            _ => {}
        }
    }

    /// Marks what `term` names, starting at field `j` of `def`.
    fn land_sibling(&mut self, def: &'a Arc<StructDef>, j: usize, term: &Term<'a>) {
        let ty = &def.fields[j].ty;
        if term.rest.is_empty() {
            let each = term.elem.is_some_and(|p| p.is_empty());
            self.p.mark(def, j, if each { AT | EACH } else { AT });
            if let Some(path) = term.elem {
                self.in_elems(ty, path);
            }
            return;
        }
        let mut ss = Vec::new();
        structs_here(self.t, ty, 0, &mut ss);
        for s in ss {
            if let Some(k) = s.fields.iter().position(|g| *g.name == *term.rest[0]) {
                let inner = Term { head: &s.fields[k].name, rest: &term.rest[1..], elem: term.elem };
                self.land_sibling(s, k, &inner);
            }
        }
    }

    /// Marks the elements of the list `ty` is: each element itself when
    /// `path` is empty, and the field at `path` inside each one otherwise.
    fn in_elems(&mut self, ty: &'a Ty, path: &'a [String]) {
        if path.is_empty() {
            // The elements are the numbers, and [`EACH`] on the list's own
            // field says so.
            return;
        }
        let mut es = Vec::new();
        elems_here(self.t, ty, 0, &mut es);
        for e in es {
            self.mark_path(e, path, AT);
        }
    }

    /// Marks the field at `path` inside each structure `ty` is.
    fn mark_path(&mut self, ty: &'a Ty, path: &'a [String], flag: u8) {
        let Some((first, rest)) = path.split_first() else { return };
        let mut ss = Vec::new();
        structs_here(self.t, ty, 0, &mut ss);
        for s in ss {
            if let Some(k) = s.fields.iter().position(|g| *g.name == **first) {
                if rest.is_empty() {
                    self.p.mark(s, k, flag);
                } else {
                    self.mark_path(&s.fields[k].ty, rest, flag);
                }
            }
        }
    }

    /// The types of the records a gather declared as field `i` of `def`
    /// reaches by `from`, as far as the template says what they are. A step
    /// whose landing only the file can say, `Stream`, ends the walk with
    /// nothing.
    fn records(&self, def: &'a Arc<StructDef>, i: usize, from: &'a [Step]) -> Vec<&'a Ty> {
        let Some((first, rest)) = from.split_first() else { return Vec::new() };
        let mut here: Vec<&'a Ty> = match first {
            // A field declared before the gather, here or in a structure
            // around it, the way the walk itself looks for one.
            Step::Field(name) => {
                let found = def.fields[..i].iter().find(|f| *f.name == **name).map(|f| &f.ty).or_else(|| {
                    self.scope.iter().rev().find_map(|s| s.fields.iter().find(|f| *f.name == **name).map(|f| &f.ty))
                });
                found.into_iter().collect()
            }
            Step::Placer => self.placers.last().cloned().unwrap_or_default(),
            _ => Vec::new(),
        };
        for step in rest {
            let mut next = Vec::new();
            for ty in &here {
                match step {
                    Step::Field(name) => field_tys(self.t, ty, name, &mut next),
                    Step::Fields(names) => {
                        for n in names.iter() {
                            field_tys(self.t, ty, n, &mut next);
                        }
                    }
                    Step::Each | Step::Elements(_) | Step::Tagged { .. } => elems_here(self.t, ty, 0, &mut next),
                    Step::Deep(name) => deep(self.t, ty, name, &mut Vec::new(), &mut next),
                    Step::Stream | Step::Placer => {}
                }
            }
            here = next;
        }
        here
    }
}

/// The type of every field called `name` anywhere under `ty`: into
/// structures and the elements of lists, never into a field it has landed on.
fn deep<'a>(t: &'a Template, ty: &'a Ty, name: &str, seen: &mut Vec<usize>, out: &mut Vec<&'a Ty>) {
    let mut ss = Vec::new();
    structs_here(t, ty, 0, &mut ss);
    let mut es = Vec::new();
    elems_here(t, ty, 0, &mut es);
    for e in es {
        structs_here(t, e, 0, &mut ss);
    }
    for s in ss {
        if seen.contains(&key(s)) {
            continue;
        }
        seen.push(key(s));
        for f in &s.fields {
            if &*f.name == name {
                out.push(&f.ty);
            } else {
                deep(t, &f.ty, name, seen, out);
            }
        }
    }
}

/// Asserts that in every structure called `structure` that template `t`
/// reaches, the fields in `marked` hold a position and the fields in
/// `unmarked` do not. Each name has to be a field of at least one of them.
#[cfg(test)]
pub(crate) fn assert_marks(t: &Template, structure: &str, marked: &[&str], unmarked: &[&str]) {
    let p = Positions::of(t);
    let defs: Vec<&Marks> = p.by_def.values().filter(|m| m._def.name == structure).collect();
    assert!(!defs.is_empty(), "{}: no structure {structure}", t.name);
    for (names, want) in [(marked, true), (unmarked, false)] {
        for n in names {
            let mut found = false;
            for m in &defs {
                if let Some(i) = m._def.fields.iter().position(|f| &*f.name == *n) {
                    found = true;
                    assert_eq!(m.flags[i] & AT != 0, want, "{}: {structure}.{n}", t.name);
                }
            }
            assert!(found, "{}: {structure} has no field {n}", t.name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::{Endian::Big, Expr as E, Ty as T};

    /// A structure built for every node read, the way a traced stream builds
    /// one per symbol, is not kept for the whole reading.
    #[test]
    fn structures_built_while_reading_are_not_all_kept() {
        let t = Template::new("t", T::structure("Root", vec![("n", T::u8())]));
        let mut p = Positions::of(&t);
        for _ in 0..3 * BUILT_CAP {
            let Ty::Struct(s) = T::structure("Built", vec![("at", T::u32(Big)), ("x", T::at(E::field("at"), T::u8()))]) else { panic!() };
            assert_eq!(p.flags(&t, &s, 0), AT);
        }
        assert!(p.by_def.len() <= p.kept + BUILT_CAP + 1, "{}", p.by_def.len());
    }

    /// A declared list of numbers holds a position in each element. A
    /// declared list of structures or of lists does not: the elements are
    /// not numbers.
    #[test]
    fn a_declared_list_of_numbers_is_a_position_in_each_element() {
        let rec = T::structure("Rec", vec![("n", T::u8())]);
        let root = T::structure(
            "Root",
            vec![
                ("count", T::u8()),
                ("offsets", T::array(T::u32(Big), E::field("count"))),
                ("records", T::array(rec, E::field("count"))),
                ("first", T::u32(Big)),
                ("table", T::array(T::array(T::u32(Big), E::field("count")), E::field("count"))),
            ],
        )
        .positions(&["offsets", "records", "first", "table"]);
        let t = Template::new("t", root.clone());
        let Ty::Struct(s) = &root else { panic!() };
        let mut p = Positions::of(&t);
        assert_eq!(p.flags(&t, s, 0), 0);
        assert_eq!(p.flags(&t, s, 1), AT | EACH);
        assert_eq!(p.flags(&t, s, 2), AT);
        assert_eq!(p.flags(&t, s, 3), AT);
        assert_eq!(p.flags(&t, s, 4), AT);
    }
}

