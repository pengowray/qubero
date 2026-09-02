//! Which field settled the shape of which.
//!
//! Some fields exist so that another field can be read at all: the count in
//! front of a list, the length in front of a run of bytes, the array of
//! offsets a page's cells are found by. They are the structure's own
//! machinery. Read straight through, a listing gives them the same weight as
//! the thing they place, so a page of three rows reads as eleven fields of
//! arithmetic and three rows.
//!
//! Nothing in the IR marks them, and marking them by hand would mean going
//! through every template. So this reads the templates as they already are: a
//! field whose name another field's type uses to work out its length, its
//! count or where it starts is machinery for that field. The answer is the
//! sibling that used it, not a yes or no, because folding machinery away is
//! folding it behind something, and the view needs to know behind what.
//!
//! What this cannot work out for itself, the template says. A field nothing
//! references can still be plumbing: a reserved word, a fragment counter, the
//! free-block chain in a b-tree page header. And a field can decide a
//! sibling's shape and still be the point: a bitmap's width settles the stride
//! of every row and is also the first thing a reader wants to know. Those two
//! are `StructDef::machinery` and `StructDef::payload`, and they win over
//! anything worked out here.
//!
//! Deciding a *value* is a different relationship and is left out. A field
//! computed from two others is not placed by them, and folding its inputs away
//! would hide the sum's own working.

use std::sync::Arc;

use crate::template::{Expr, StrLen, StructDef, Ty};

/// For each field of `def`, the first later sibling whose length, count, type
/// or position that field settles. `None` for a field no sibling reads, which
/// is most of them.
///
/// Only siblings count. A name reached from an enclosing structure belongs to
/// that structure's own answer, and a name used inside a nested structure is
/// that structure's business: descending into one would mark a field here
/// whose name a type further in happens to reuse.
pub fn consumers(def: &StructDef) -> Vec<Option<usize>> {
    let mut out = vec![None; def.fields.len()];
    let mut names: Vec<Arc<str>> = Vec::new();
    for (i, f) in def.fields.iter().enumerate() {
        names.clear();
        ty_refs(&f.ty, &mut names);
        if names.is_empty() {
            continue;
        }
        // A field is written before the fields that read it, so only earlier
        // siblings are candidates, and the first reader is the owner it folds
        // behind when several read it.
        for (j, g) in def.fields.iter().enumerate().take(i) {
            if out[j].is_none() && names.iter().any(|n| *n == g.name) {
                out[j] = Some(i);
            }
        }
    }
    out
}

/// What the template itself says about field `i`: `Some(true)` for machinery,
/// `Some(false)` for payload, `None` when it has no opinion and [`consumers`]
/// is all there is to go on.
pub fn hint(def: &StructDef, i: usize) -> Option<bool> {
    let name = &def.fields.get(i)?.name;
    if def.machinery.iter().any(|n| n == name) {
        return Some(true);
    }
    if def.payload.iter().any(|n| n == name) {
        return Some(false);
    }
    None
}

/// Every sibling name this type reads, for length, count, type or position.
fn ty_refs(ty: &Ty, out: &mut Vec<Arc<str>>) {
    match ty {
        Ty::Bytes(e) => expr_refs(e, out),
        Ty::Str { len, .. } | Ty::TextInt { len, .. } => strlen_refs(len, out),
        Ty::Array { elem, count } => {
            expr_refs(count, out);
            ty_refs(elem, out);
        }
        // `Until::FieldBytes` names a field of the element, not a sibling of
        // the list, so there is nothing here to collect.
        Ty::Repeat { elem, .. } => ty_refs(elem, out),
        Ty::PointerList { offsets, adjust, elem, .. } => {
            out.push(offsets.clone());
            expr_refs(adjust, out);
            ty_refs(elem, out);
        }
        Ty::At { at, inner, .. } => {
            expr_refs(at, out);
            ty_refs(inner, out);
        }
        Ty::Sized { size, inner } => {
            expr_refs(size, out);
            ty_refs(inner, out);
        }
        Ty::Switch { on, cases, default } => {
            expr_refs(on, out);
            for (_, t) in cases.iter() {
                ty_refs(t, out);
            }
            ty_refs(default, out);
        }
        Ty::Match { on, cases, default } => {
            expr_refs(on, out);
            for (_, t) in cases.iter() {
                ty_refs(t, out);
            }
            ty_refs(default, out);
        }
        Ty::Enum { inner, .. } | Ty::Flags { inner, .. } | Ty::Nullable { inner, .. } => ty_refs(inner, out),
        // How wide the number is settles it the way a length settles a run of
        // bytes, so the field that said so is machinery for it: a GRIB's
        // `bits_per_value` belongs to the grid it packs.
        Ty::UIntExpr { bits, .. } => expr_refs(bits, out),
        // A value worked out from other fields, which is not the same as being
        // placed by them. See the module note.
        Ty::Computed(_) => {}
        // A structure's fields name their own siblings; a type from the table
        // is not here to look at. Stopping leaves a field unmarked, which
        // shows it as an ordinary row: the safe way to be wrong.
        Ty::Struct(_) | Ty::Named(_) => {}
        _ => {}
    }
}

fn strlen_refs(len: &StrLen, out: &mut Vec<Arc<str>>) {
    match len {
        StrLen::Fixed(e) => expr_refs(e, out),
        StrLen::Padded { size, .. } => expr_refs(size, out),
        StrLen::Scan { .. } | StrLen::Terminated { .. } => {}
    }
}

fn expr_refs(e: &Expr, out: &mut Vec<Arc<str>>) {
    match e {
        Expr::Ref(n) | Expr::SizeOf(n) | Expr::BitsOf(n) | Expr::ProductOf(n) | Expr::SumOf(n) | Expr::MaxOf(n) | Expr::Prev(n) => {
            out.push(n.clone())
        }
        Expr::Elem { array, index, .. } | Expr::Product { array, index, .. } => {
            out.push(array.clone());
            expr_refs(index, out);
        }
        // The list searched, when it is one named beside this field, and the
        // field the label is worked out from, when it is. A search over the
        // enclosing list names neither: what it looks at is elements, and an
        // element is not a sibling of the field asking.
        Expr::Tagged(t) => {
            if let Some(array) = &t.array {
                out.push(array.clone());
            }
            if let crate::template::Tag::Computed(e) = &t.tag {
                expr_refs(e, out);
            }
        }
        // A path starting at a sibling and going down into it. Only its first
        // step names something in this structure.
        Expr::Sibling(path) | Expr::Within(path) => {
            if let Some(first) = path.first() {
                out.push(Arc::from(first.as_str()));
            }
        }
        Expr::Or(a, b)
        | Expr::Add(a, b)
        | Expr::Sub(a, b)
        | Expr::Mul(a, b)
        | Expr::Div(a, b)
        | Expr::Less(a, b)
        | Expr::Shl(a, b)
        | Expr::Min(a, b)
        | Expr::Max(a, b) => {
            expr_refs(a, out);
            expr_refs(b, out);
        }
        Expr::PeekAt { skip, .. } => expr_refs(skip, out),
        Expr::PadTo { n, .. } => expr_refs(n, out),
        Expr::Bit(a, _) => expr_refs(a, out),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::{Endian::Big, Expr as E, Ty as T};

    fn def(ty: &Ty) -> Arc<StructDef> {
        match ty {
            Ty::Struct(s) => s.clone(),
            other => panic!("not a structure: {other:?}"),
        }
    }

    fn named(def: &StructDef, got: &[Option<usize>]) -> Vec<(String, Option<String>)> {
        def.fields
            .iter()
            .zip(got)
            .map(|(f, c)| (f.name.to_string(), c.map(|i| def.fields[i].name.to_string())))
            .collect()
    }

    #[test]
    fn a_length_prefix_belongs_to_what_it_measures() {
        let s = def(&T::structure("Chunk", vec![("len", T::u32(Big)), ("data", T::bytes(E::field("len")))]));
        assert_eq!(named(&s, &consumers(&s)), vec![("len".into(), Some("data".into())), ("data".into(), None)]);
    }

    #[test]
    fn a_count_belongs_to_the_list_it_counts() {
        let s = def(&T::structure(
            "Page",
            vec![("cell_count", T::u16(Big)), ("cell_pointers", T::array(T::u16(Big), E::field("cell_count")))],
        ));
        assert_eq!(consumers(&s), vec![Some(1), None]);
    }

    #[test]
    fn an_offset_array_belongs_to_the_children_it_places() {
        let s = def(&T::structure(
            "Body",
            vec![
                ("count", T::u16(Big)),
                ("pointers", T::array(T::u16(Big), E::field("count"))),
                ("cells", T::pointer_list("pointers", crate::template::Anchor::Window, E::lit(0), T::u8())),
            ],
        ));
        assert_eq!(consumers(&s), vec![Some(1), Some(2), None]);
    }

    #[test]
    fn a_field_nobody_reads_has_no_owner() {
        let s = def(&T::structure("Header", vec![("magic", T::magic(b"AB")), ("flags", T::u16(Big))]));
        assert_eq!(consumers(&s), vec![None, None]);
    }

    #[test]
    fn a_computed_value_does_not_own_its_inputs() {
        let s = def(&T::structure(
            "Sum",
            vec![("a", T::u16(Big)), ("b", T::u16(Big)), ("total", T::Computed(E::field("a").add(E::field("b"))))],
        ));
        assert_eq!(consumers(&s), vec![None, None, None]);
    }

    #[test]
    fn hints_say_what_the_shapes_cannot() {
        let ty = T::structure("Page", vec![("frag", T::u8()), ("width", T::u16(Big)), ("rows", T::bytes(E::field("width")))])
            .machinery(&["frag"])
            .payload(&["width"]);
        let s = def(&ty);
        assert_eq!(hint(&s, 0), Some(true));
        assert_eq!(hint(&s, 1), Some(false));
        assert_eq!(hint(&s, 2), None);
        // The hint sits over the shapes rather than changing them.
        assert_eq!(consumers(&s), vec![None, Some(2), None]);
    }

    #[test]
    fn sqlite_reads_as_the_listing_needs() {
        let t = crate::formats::sqlite();
        let s = def(&t.root);
        let got = named(&s, &consumers(&s));
        let owner = |name: &str| {
            got.iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no field {name}")).1.clone()
        };
        // The header's page size settles every page below it. That makes the
        // pages its owner; whether it is folded is the view's business, since
        // the pages are not in the same part of the listing.
        assert_eq!(owner("page_size"), Some("page1".to_string()));
        // Read by nothing: the file's own identity and its counters.
        assert_eq!(owner("magic"), None);
        assert_eq!(owner("change_counter"), None);
        assert_eq!(owner("user_version"), None);
        // The reserved space at the end of every page is read inside the page
        // structure, not by a sibling of the field. That is the limit written
        // down in `ty_refs`, and it fails the safe way: the field stays an
        // ordinary row of the header, which is where a reader wants it.
        assert_eq!(owner("reserved_space"), None);
    }
}
