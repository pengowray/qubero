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

use crate::template::{Expr, StrLen, StructDef, Ty, Until};

mod positions;
pub use positions::{Positions, AT, EACH};

/// For each field of `def`, the first later sibling whose length, count, type
/// or position that field settles. `None` for a field no sibling reads, which
/// is most of them.
///
/// Only siblings count. A name reached from an enclosing structure belongs to
/// that structure's own answer, and a name used inside a nested structure is
/// that structure's business: descending into one would mark a field here
/// whose name a type further in happens to reuse.
pub fn consumers(def: &StructDef) -> Vec<Option<usize>> {
    refs(def, true)
}

/// The same, for the fields that settle how long, how many or where another
/// field is, leaving out the ones that settle only *which* of several shapes
/// it takes.
///
/// The two are machinery in different degrees. A length in front of a run of
/// bytes is arithmetic: a reading that leads with it opens with the extent the
/// row is already drawn at. A field that picks a shape is usually the word the
/// whole record is about, and dropping it leaves nothing to read: a wasm
/// opcode picks the type of its immediate, and `i32.const 42` without it is
/// `42`.
pub fn measurers(def: &StructDef) -> Vec<Option<usize>> {
    refs(def, false)
}

fn refs(def: &StructDef, selectors: bool) -> Vec<Option<usize>> {
    let mut out = vec![None; def.fields.len()];
    let mut names: Vec<Arc<str>> = Vec::new();
    for (i, f) in def.fields.iter().enumerate() {
        names.clear();
        ty_refs(&f.ty, &mut names, selectors);
        // What the field says it is a table of counts too: a run of samples
        // whose rows are as wide as `channels` says is settled by that field
        // as much as its length is, and a view of what depends on what that
        // left the shape out would draw the samples depending on nothing.
        if let Some(t) = &f.table {
            for e in t.columns.iter().chain(t.rate.iter()).chain(t.facts.iter().map(|f| &f.value)) {
                expr_refs(e, &mut names);
            }
        }
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

/// Every sibling an expression reads by name. What [`consumers`] is worked out
/// from, for a caller that has one expression in hand and wants to know
/// whether a particular field is in it: the offset a pointer is read from is
/// the field that pointer points from, and a reading that says so has to be
/// sure it is naming the right one.
pub fn names_in(e: &Expr) -> Vec<Arc<str>> {
    let mut out = Vec::new();
    expr_refs(e, &mut out);
    out
}

/// For each field of `def`, whether a later sibling uses its value as a
/// position: where an `At` reads its contents, the table a `PointerList`
/// takes its offsets from, where a `Chain` starts, and the base a
/// `PointerList`, `Chain` or `Gather` adds to every offset it reads. A field
/// named in [`StructDef::positions`] is one too.
///
/// A view uses this to write such a value in the same base as the addresses
/// beside it, so `program_header_offset` can be matched by eye to the address
/// of the table it points at.
///
/// Only the terms a position is made of by adding count. In `base + index *
/// stride` the index and the stride are a count and a length, and writing
/// them as addresses would say they point somewhere. A field named on the
/// right of a subtraction is left out for the same reason: `end - size` is a
/// position, `size` is not.
///
/// This is `def` on its own. A position read from inside another structure,
/// such as the `offset` a `PointerList` reads in each element of a table, is
/// marked by [`Positions`], which walks a whole template.
pub fn locators(def: &Arc<StructDef>) -> Vec<bool> {
    let t = crate::template::Template::new("", Ty::Struct(def.clone()));
    let mut p = Positions::default();
    (0..def.fields.len()).map(|i| p.flags(&t, def, i) & AT != 0).collect()
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
///
/// `selectors` says whether the name a switch picks its shape by counts. It
/// does for [`consumers`], which is about which field settled another's shape
/// at all, and does not for [`measurers`], which is about arithmetic.
fn ty_refs(ty: &Ty, out: &mut Vec<Arc<str>>, selectors: bool) {
    match ty {
        Ty::Bytes(e) => expr_refs(e, out),
        Ty::Str { len, .. } | Ty::TextInt { len, .. } => strlen_refs(len, out),
        Ty::Array { elem, count } => {
            expr_refs(count, out);
            ty_refs(elem, out, selectors);
        }
        // `Until::FieldBytes` names a field of the element, not a sibling of
        // the list, so there is nothing here to collect. `Until::Cond` is
        // asked from inside the element too, and a name in it is the
        // element's until it is not: the climb out of an element reaches the
        // list's siblings, and a sibling with the same name as a field of the
        // element would be marked as machinery for a run it has nothing to do
        // with. Leaving it unmarked shows it as an ordinary row, which is the
        // safe way to be wrong.
        //
        // `Until::While` is the one that is asked beside the list rather than
        // inside an element, so the names in it *are* siblings, and a field
        // that decides where a run stops is that run's machinery.
        Ty::Repeat { elem, until } => {
            if let Until::While(e) = until {
                expr_refs(e, out);
            }
            ty_refs(elem, out, selectors);
        }
        Ty::PointerList { offsets, adjust, elem, .. } => {
            out.push(offsets.clone());
            expr_refs(adjust, out);
            ty_refs(elem, out, selectors);
        }
        // The field that says where the chain starts is machinery for it, the
        // way a pointer list's table of offsets is. The `next` field is inside
        // an element and so is not a sibling of anything here.
        Ty::Chain { first, elem, .. } => {
            expr_refs(first, out);
            ty_refs(elem, out, selectors);
        }
        // Not the field a gather's walk starts from. That is where its records
        // are, and the records are a table of their own with a reading of
        // their own: a FITS heap walks the rows, and folding the rows away as
        // machinery for the heap would hide the table under what it points at.
        // What only places the children, the adjustment, is machinery.
        Ty::Gather { adjust, elem, .. } => {
            expr_refs(adjust, out);
            ty_refs(elem, out, selectors);
        }
        Ty::At { at, inner, .. } => {
            expr_refs(at, out);
            ty_refs(inner, out, selectors);
        }
        Ty::Sized { size, inner } => {
            expr_refs(size, out);
            ty_refs(inner, out, selectors);
        }
        Ty::Origin { inner } => ty_refs(inner, out, selectors),
        // Whether a field is there at all is the same kind of answer a switch
        // gives, so the field that decides it counts as a selector and not as
        // a measurer: a flag word that says which optional fields a record
        // carries is usually the word the record is about.
        Ty::When { cond, inner } => {
            if selectors {
                expr_refs(cond, out);
            }
            ty_refs(inner, out, selectors);
        }
        Ty::Switch { on, cases, default } => {
            if selectors {
                expr_refs(on, out);
            }
            for (_, t) in cases.iter() {
                ty_refs(t, out, selectors);
            }
            ty_refs(default, out, selectors);
        }
        Ty::Match { on, cases, default } => {
            if selectors {
                expr_refs(on, out);
            }
            for (_, t) in cases.iter() {
                ty_refs(t, out, selectors);
            }
            ty_refs(default, out, selectors);
        }
        Ty::Enum { inner, .. } | Ty::Flags { inner, .. } | Ty::Nullable { inner, .. } => ty_refs(inner, out, selectors),
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
        Expr::Ref(n) | Expr::SizeOf(n) | Expr::BitsOf(n) | Expr::ProductOf(n) | Expr::SumOf(n) | Expr::MaxOf(n) | Expr::PopCount(n) | Expr::LenOf(n) | Expr::Prev(n) => {
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
                expr_refs(array, out);
            }
            if let crate::template::Tag::Computed(e) | crate::template::Tag::ComputedText(e) = &t.tag {
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
        // The records of an archive, named the way a path into a sibling is,
        // and the expression that says which entry, which may name more.
        Expr::EntryOf { records, name } => {
            if let Some(first) = records.first() {
                out.push(Arc::from(first.as_str()));
            }
            expr_refs(name, out);
        }
        // The same, and the index is arithmetic that may name more.
        Expr::ElemWithin { path, index, .. } => {
            if let Some(first) = path.first() {
                out.push(Arc::from(first.as_str()));
            }
            expr_refs(index, out);
        }
        Expr::Or(a, b)
        | Expr::Either(a, b)
        | Expr::Both(a, b)
        | Expr::Add(a, b)
        | Expr::Sub(a, b)
        | Expr::Mul(a, b)
        | Expr::Div(a, b)
        | Expr::Mod(a, b)
        | Expr::DivCeil(a, b)
        | Expr::Less(a, b)
        | Expr::Eq(a, b)
        | Expr::Ne(a, b)
        | Expr::Le(a, b)
        | Expr::Gt(a, b)
        | Expr::Ge(a, b)
        | Expr::Shl(a, b)
        | Expr::Shr(a, b)
        | Expr::And(a, b)
        | Expr::BitOr(a, b)
        | Expr::BitXor(a, b)
        | Expr::Min(a, b)
        | Expr::Max(a, b) => {
            expr_refs(a, out);
            expr_refs(b, out);
        }
        // Every branch, not only the one this file takes: which field is
        // machinery for which is a fact about the template, and the template
        // reads all three.
        Expr::Cond { when, then, otherwise } => {
            expr_refs(when, out);
            expr_refs(then, out);
            expr_refs(otherwise, out);
        }
        Expr::Log2(a) | Expr::Not(a) | Expr::BitNot(a) => expr_refs(a, out),
        // Where a field is rather than what it says, but the field is named
        // the same way, and a field something is placed from is plumbing the
        // same as one something is sized from.
        Expr::StartOf(a) => expr_refs(a, out),
        // A field whose bytes are compared whole, which settles what it
        // settles the way a comparison with a number does.
        Expr::BytesEq { field, .. } => expr_refs(field, out),
        Expr::PeekAt { skip, .. } => expr_refs(skip, out),
        Expr::PeekIn { at, .. } => expr_refs(at, out),
        Expr::PadTo { n, .. } => expr_refs(n, out),
        Expr::Bit(a, _) => expr_refs(a, out),
        // A NIfTI-1 header's `vox_offset` places the voxels through its whole
        // part, and is plumbing for them the same as a length field is.
        Expr::Trunc(a) | Expr::Pow2(a) | Expr::Pow10(a) | Expr::RealText(a) => expr_refs(a, out),
        Expr::Words { value, .. } => expr_refs(value, out),
        Expr::Lower(a) => expr_refs(a, out),
        // A field whose text is searched for settles where the run it ends
        // stops, the way a length field settles it: a MIME message's header
        // is machinery for every part its boundary separates.
        Expr::FindText(f) => expr_refs(&f.text, out),
        // What a record says, and a record is somewhere else entirely: the
        // names inside are its fields, not siblings of this one.
        Expr::Placer(_) => {}
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

    /// How wide the numbers are packed settles the run the same way a length
    /// settles a run of bytes, so the field that said so folds behind it.
    #[test]
    fn a_width_belongs_to_the_run_it_packs() {
        let s = def(&T::structure(
            "Section",
            vec![
                ("bits_per_value", T::u8()),
                ("count", T::u16(Big)),
                ("values", T::array(T::uint_expr(E::field("bits_per_value"), Big), E::field("count"))),
            ],
        ));
        assert_eq!(consumers(&s), vec![Some(2), Some(2), None]);
    }

    /// A record that finds an earlier one by a number it carries is placed by
    /// that number, so the byte holding it is machinery for the field that
    /// used it. The list searched is the one the record is in, which is not a
    /// sibling of anything and so names nothing here.
    #[test]
    fn a_computed_label_belongs_to_the_lookup_that_used_it() {
        let s = def(&T::structure(
            "Rec",
            vec![
                ("class", T::u8()),
                ("body", T::sized(E::sibling_tagged(&["class_num"], E::field("class"), &["size"]), T::u8())),
            ],
        ));
        assert_eq!(consumers(&s), vec![Some(1), None]);
    }

    /// The offset an `At` reads from is a position; the count beside it, the
    /// index and stride multiplied into it, and a length subtracted from it
    /// are not.
    #[test]
    fn positions_are_the_terms_an_offset_is_added_from() {
        let s = def(&T::structure(
            "Header",
            vec![
                ("table_offset", T::u32(Big)),
                ("count", T::u16(Big)),
                ("index", T::u16(Big)),
                ("stride", T::u16(Big)),
                ("end", T::u32(Big)),
                ("size", T::u32(Big)),
                ("table", T::at(E::field("table_offset"), T::array(T::u8(), E::field("count")))),
                ("entry", T::at(E::field("table_offset").add(E::field("index").mul(E::field("stride"))), T::u8())),
                ("tail", T::at(E::field("end").sub(E::field("size")), T::u8())),
            ],
        ));
        assert_eq!(locators(&s), vec![true, false, false, false, true, false, false, false, false]);
    }

    #[test]
    fn an_offset_table_and_a_chain_start_are_positions() {
        let s = def(&T::structure(
            "Body",
            vec![
                ("count", T::u16(Big)),
                ("pointers", T::array(T::u16(Big), E::field("count"))),
                ("cells", T::pointer_list("pointers", crate::template::Anchor::Window, E::lit(0), T::u8())),
                ("first", T::u32(Big)),
                ("records", T::chain(E::field("first"), &["next"], crate::template::Anchor::File, T::u8())),
            ],
        ));
        assert_eq!(locators(&s), vec![false, true, false, true, false]);
    }

    /// The flags `Positions` gives each field of `s`, by name, in a template
    /// whose root is `root` and which `s` is reached from.
    fn flags_in(root: &Ty, s: &Arc<StructDef>) -> Vec<(String, u8)> {
        let t = crate::template::Template::new("test", root.clone());
        let mut p = Positions::of(&t);
        s.fields.iter().enumerate().map(|(i, f)| (f.name.to_string(), p.flags(&t, s, i))).collect()
    }

    fn marked(got: &[(String, u8)], flag: u8) -> Vec<&str> {
        got.iter().filter(|(_, f)| f & flag != 0).map(|(n, _)| n.as_str()).collect()
    }

    /// A pointer list that reads its offsets from inside each element of a
    /// table marks that field of the element, and not the size beside it.
    #[test]
    fn the_offset_a_pointer_list_reads_in_each_record_is_a_position() {
        use crate::template::Anchor;
        let header = T::structure("SectionHeader", vec![("offset", T::u64(Big)), ("size", T::u64(Big))]);
        let root = T::structure(
            "File",
            vec![
                ("count", T::u16(Big)),
                ("headers", T::array(header, E::field("count"))),
                ("sections", T::pointer_list_sized("headers", &["offset"], Anchor::File, E::lit(0), T::u8())),
            ],
        );
        // The header as it sits inside the root, which is the `Arc` a node
        // read holds.
        let Ty::Array { elem, .. } = &def(&root).fields[1].ty else { panic!() };
        assert_eq!(marked(&flags_in(&root, &def(elem)), AT), vec!["offset"]);
        // The table itself is marked as before, and its elements are records,
        // not numbers to mark.
        let top = flags_in(&root, &def(&root));
        assert_eq!(marked(&top, AT), vec!["headers"]);
        assert!(marked(&top, EACH).is_empty());
    }

    /// A pointer list over a plain array of numbers marks each element, which
    /// the array's own field carries as `EACH`.
    #[test]
    fn each_element_of_an_offsets_array_is_a_position() {
        let root = T::structure(
            "Body",
            vec![
                ("count", T::u16(Big)),
                ("pointers", T::array(T::u16(Big), E::field("count"))),
                ("cells", T::pointer_list("pointers", crate::template::Anchor::Window, E::lit(0), T::u8())),
                ("lengths", T::array(T::u16(Big), E::field("count"))),
            ],
        );
        let got = flags_in(&root, &def(&root));
        assert_eq!(marked(&got, EACH), vec!["pointers"]);
        assert_eq!(marked(&got, AT), vec!["pointers"]);
    }

    /// `offsets[i]` in an `At` marks each element of `offsets`, and
    /// `tensors[i].offset` marks `offset` inside each tensor record.
    #[test]
    fn an_element_read_as_a_position_is_one() {
        let tensor = T::structure("Tensor", vec![("dims", T::u32(Big)), ("offset", T::u64(Big))]);
        let at_field = E::Elem { array: "tensors".into(), index: Box::new(E::lit(0)), field: Arc::from(vec!["offset".to_string()]) };
        let first = E::Elem { array: "offsets".into(), index: Box::new(E::lit(0)), field: Arc::from(Vec::new()) };
        let root = T::structure(
            "File",
            vec![
                ("n", T::u32(Big)),
                ("offsets", T::array(T::u32(Big), E::field("n"))),
                ("tensors", T::array(tensor, E::field("n"))),
                ("first", T::at(first, T::u8())),
                ("data", T::at(at_field, T::u8())),
            ],
        );
        let r = def(&root);
        assert_eq!(marked(&flags_in(&root, &r), EACH), vec!["offsets"]);
        let Ty::Array { elem, .. } = &r.fields[2].ty else { panic!() };
        assert_eq!(marked(&flags_in(&root, &def(elem)), AT), vec!["offset"]);
    }

    /// Where an element ends is its offset plus its lengths, and the lengths
    /// are not positions.
    #[test]
    fn the_lengths_added_to_an_elements_offset_are_not_positions() {
        let block = T::structure("Block", vec![("offset", T::u64(Big)), ("head", T::u32(Big)), ("body", T::u64(Big))]);
        let part = |name: &str| E::Elem { array: "blocks".into(), index: Box::new(E::lit(0)), field: Arc::from(vec![name.to_string()]) };
        let root = T::structure(
            "File",
            vec![("blocks", T::array(block, E::lit(2))), ("end", T::at(part("offset").add(part("head")).add(part("body")), T::u8()))],
        );
        let Ty::Array { elem, .. } = &def(&root).fields[0].ty else { panic!() };
        assert_eq!(marked(&flags_in(&root, &def(elem)), AT), vec!["offset"]);
    }

    /// A chain's `next` is a position inside every record of the chain, along
    /// the path the chain names, through a switch on the record's kind.
    #[test]
    fn a_chain_next_is_a_position_in_each_record() {
        let a = T::structure("A", vec![("size", T::u32(Big)), ("next", T::u32(Big))]);
        let b = T::structure("B", vec![("next", T::u32(Big)), ("count", T::u32(Big))]);
        let record = T::structure("Record", vec![("kind", T::u8()), ("body", T::switch(E::field("kind"), vec![(1, a)], b))]);
        let root = T::structure(
            "File",
            vec![("first", T::u32(Big)), ("records", T::chain(E::field("first"), &["body", "next"], crate::template::Anchor::File, record))],
        );
        assert_eq!(marked(&flags_in(&root, &def(&root)), AT), vec!["first"]);
        let Ty::Chain { elem, .. } = &def(&root).fields[1].ty else { panic!() };
        let rec = def(elem);
        assert!(marked(&flags_in(&root, &rec), AT).is_empty());
        let Ty::Switch { cases, default, .. } = &rec.fields[1].ty else { panic!() };
        assert_eq!(marked(&flags_in(&root, &def(&cases[0].1)), AT), vec!["next"]);
        assert_eq!(marked(&flags_in(&root, &def(default)), AT), vec!["next"]);
    }

    /// A gather's offset is a position in each record its steps reach; the
    /// count beside it, which says how long the child is, is not.
    #[test]
    fn a_gather_offset_is_a_position_in_each_record() {
        use crate::template::Step;
        let desc = T::structure("Descriptor", vec![("count", T::u32(Big)), ("offset", T::u32(Big))]);
        let row = T::structure("Row", vec![("descriptors", T::array(desc, E::lit(2)))]);
        let root = T::structure(
            "Table",
            vec![
                ("rows", T::array(row, E::lit(3))),
                (
                    "heap",
                    T::gather(
                        vec![Step::field("rows"), Step::each(), Step::field("descriptors"), Step::each()],
                        E::field("offset"),
                        crate::template::Anchor::Window,
                        E::lit(0),
                        T::bytes(E::Placer(Box::new(E::field("count")))),
                    ),
                ),
            ],
        );
        let r = def(&root);
        // The walk starts at `rows`, which is where the records are and not
        // itself a position.
        assert!(marked(&flags_in(&root, &r), AT).is_empty());
        let Ty::Array { elem: row, .. } = &r.fields[0].ty else { panic!() };
        let Ty::Array { elem: desc, .. } = &def(row).fields[0].ty else { panic!() };
        assert_eq!(marked(&flags_in(&root, &def(desc)), AT), vec!["offset"]);
    }

    /// A path into an earlier field marks the field it lands on.
    #[test]
    fn a_path_into_an_earlier_field_marks_where_it_lands() {
        let inner = T::structure("Info", vec![("len", T::u32(Big)), ("at", T::u32(Big))]);
        let within = E::Within(Arc::from(vec!["info".to_string(), "at".to_string()]));
        let root = T::structure("File", vec![("info", inner), ("data", T::at(within, T::u8()))]);
        let r = def(&root);
        // `info` is not the number, and is left alone.
        assert!(marked(&flags_in(&root, &r), AT).is_empty());
        assert_eq!(marked(&flags_in(&root, &def(&r.fields[0].ty)), AT), vec!["at"]);
    }

    /// A field the template declares a position is one, though nothing reads it.
    #[test]
    fn a_declared_position_is_one() {
        let s = def(&T::structure("Segment", vec![("offset", T::u64(Big)), ("size", T::u64(Big))]).positions(&["offset"]));
        assert_eq!(locators(&s), vec![true, false]);
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
        // The page size in bytes settles every page below it. That makes the
        // pages its owner; whether it is folded is the view's business, since
        // the pages are not in the same part of the listing.
        assert_eq!(owner("page_bytes"), Some("page1".to_string()));
        // The field as written is read only by the one working out its size
        // in bytes, and a value worked out from a field does not make that
        // field machinery. It stays a row of the header.
        assert_eq!(owner("page_size"), None);
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
