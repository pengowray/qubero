//! The types a ROOT file describes, built from the descriptions it carries.
//!
//! A streamed object is a run of members in the order its class wrote them,
//! and the order is written down in the file's `StreamerInfo` record: one
//! `TStreamerInfo` per class and version, holding one `TStreamerElement` per
//! member. So an object is a [`Ty::Schema`] node, and this is the builder that
//! reads those descriptions and makes a structure of them.
//!
//! The same circle [`root_streamer`](super::super::root_streamer) cuts, cut the
//! same way. The descriptions are streamed objects too, of a handful of classes
//! ROOT wrote streamers for by hand and has not changed: `TObject`, `TNamed`,
//! `TString`, the `TArray` family, the two collections, and `TStreamerInfo`
//! with its elements. Those are written out here and answered without looking
//! at the file. Everything else is looked up among the descriptions, matched by
//! class name and version the way `Schema::find` matches them, and laid out
//! member by member the way `read_element` reads one.
//!
//! Two kinds of key. `[class]` is an object of that class as it is written in
//! place: a `TObject` or a `TString` as their streamers write them, anything
//! else as a byte count, a version and its members. `[class, version]` is those
//! members. The version is only known once the object's own two bytes have
//! been read, which is why the second is a node of its own inside the first.
//!
//! A member whose type code this does not read ends the members there, as
//! bytes, the way the side reader stops at it: the byte count in front of the
//! object is what puts everything after the object back where it belongs.

use std::sync::Arc;

use crate::eval::{Descriptions, EvalError, R};
use crate::formats::root_streamer::{bootstrap, Element};
use crate::template::{Built, Endian::Big, Expr as E, KeyPart, KeyValue, SchemaBuilder, Step, Ty as T};

/// The name the template's schema nodes give this builder.
pub(super) const KIND: &str = "root";

/// The tag a pointer writes in front of a class name it spells out, because
/// this is the first object of the class in the record.
const NEW_CLASS: i128 = 0xFFFF_FFFF;

/// How many levels of base classes a count is looked for through. A class
/// description names its bases and each base names its own; a real file is
/// three or four deep, and a file that is not stops here.
const BASE_DEPTH: usize = 8;

/// The walk from any object of the file to the class descriptions: the
/// `StreamerInfo` record's object, which is a `TList`, and the members of each
/// `TStreamerInfo` in it.
///
/// Every step is a name the template gives, so every description it lands on
/// is a place in the field tree a reader can go to.
pub(super) fn table() -> Vec<Step> {
    vec![
        Step::field("streamer_info"),
        Step::field("object"),
        Step::field("members"),
        Step::field("elements"),
        Step::each(),
        Step::field("object"),
        Step::field("members"),
    ]
}

/// An object whose class is the text `class` names, written where it stands.
pub(super) fn object_of(class: E) -> T {
    T::schema(KIND, table(), vec![KeyPart::Text(class)])
}

/// The builder the template registers for [`KIND`].
#[derive(Debug, Default)]
pub(super) struct Streamers;

impl SchemaBuilder for Streamers {
    fn build(&self, key: &[KeyValue], table: &mut dyn Descriptions) -> R<Built> {
        match key {
            [KeyValue::Text(class)] => Ok(Built::by_heart(in_place(class))),
            [KeyValue::Text(class), KeyValue::Int(version)] => members(table, class, *version as i32),
            _ => Err(EvalError::Failed("a ROOT object is keyed by its class, or its class and version".into())),
        }
    }

    fn key_text(&self, key: &[KeyValue]) -> String {
        match key {
            [KeyValue::Text(class)] => class.to_string(),
            [KeyValue::Text(class), KeyValue::Int(version)] => format!("{class} v{version}"),
            _ => String::new(),
        }
    }
}

/// An object of `class` as it is written in place.
///
/// Three kinds of class write neither a byte count nor a version, and each is
/// its own shape. Everything else opens with four bytes whose bit 30 says they
/// are a count, and two of version, and its members are in the window the
/// count gives them.
fn in_place(class: &str) -> T {
    match class {
        "TObject" => tobject(),
        "TString" => super::tstring(),
        _ => match array_elem(class) {
            Some(elem) => tarray(class, elem),
            None => counted(class),
        },
    }
}

/// `TObject`: a version, two counters, and two places it can be longer. It
/// writes no byte count of its own, so every one of those has to be read.
fn tobject() -> T {
    T::structure(
        "TObject",
        vec![
            ("version", T::u16(Big)),
            // Bit 14 says four more bytes were written after the version.
            ("extra", T::when(E::field("version").bit(14), T::bytes(E::lit(4)))),
            ("fUniqueID", T::u32(Big)),
            ("fBits", T::u32(Big)),
            // Bit 4 of the bits: something else holds a reference to this
            // object, and the file it is in wrote two bytes to say which.
            ("pidf", T::when(E::field("fBits").and(E::lit(0x10)), T::u16(Big))),
        ],
    )
    .machinery(&["extra", "pidf"])
}

/// What one element of a `TArray` is, by the letter the class ends with.
fn array_elem(class: &str) -> Option<T> {
    Some(match class {
        "TArrayC" => T::Int { bits: 8, endian: Big },
        "TArrayS" => T::Int { bits: 16, endian: Big },
        "TArrayI" => T::Int { bits: 32, endian: Big },
        "TArrayL" | "TArrayL64" => T::Int { bits: 64, endian: Big },
        "TArrayF" => T::F32(Big),
        "TArrayD" => T::F64(Big),
        _ => return None,
    })
}

/// A count and then that many numbers, with nothing in front of the count.
fn tarray(class: &str, elem: T) -> T {
    T::structure(class, vec![("fN", T::i32(Big)), ("fArray", T::array(elem, E::field("fN")))])
}

/// An object that says how long it is: a byte count with bit 30 set, the
/// version of the class, and the members in the window the count leaves.
///
/// An object whose version has bit 14 set was written one member at a time
/// across a whole collection, which nothing here reads, so its members stay
/// bytes. One whose four bytes are not a count is an older writer's, with the
/// version and nothing to say where it ends.
fn counted(class: &str) -> T {
    let version = || E::field("version");
    let members = T::switch(
        version().bit(14),
        vec![(1, T::bytes(E::Remaining))],
        T::schema(KIND, table(), vec![KeyPart::TextLit(class.into()), KeyPart::Int(version().and(E::lit(0x3fff)))]),
    );
    let with_count = T::structure_named(
        class,
        "",
        "members",
        vec![
            ("count_raw", T::u32(Big)),
            ("byte_count", T::computed(E::field("count_raw").and(E::lit(0x3fff_ffff)))),
            ("version", T::u16(Big)),
            ("members", T::sized(E::field("byte_count").sub(E::lit(2)), members.clone())),
        ],
    )
    .machinery(&["count_raw", "byte_count"]);
    let without = T::structure_named(class, "", "members", vec![("version", T::u16(Big)), ("members", members)]);
    T::switch(E::peek(32, Big).bit(30), vec![(1, with_count)], without)
}

/// The fields of a pointer to an object: nothing, a reference to an object
/// read earlier, or a class and the object.
///
/// The class is spelled out the first time a record holds one of it, and every
/// later object of that class writes where that spelling is instead. ROOT
/// counts that place from the start of the key rather than the start of the
/// object's bytes, and adds two, so the name a tag `t` means is at
/// `(t & 0x7fffffff) + 2 - fKeylen` from where the record's object begins. That
/// is an ordinary field placed from an origin, and needs nothing remembered.
///
/// A reference to an object is its four bytes and nothing more. Placing what
/// is in a record never needs one followed.
fn pointer_fields() -> Vec<(&'static str, T)> {
    let first = || E::field("first");
    let counted = || first().bit(30).both(first().not_equal(E::lit(NEW_CLASS)));
    let tag = || E::cond(counted(), E::field("tag"), first());
    vec![
        ("first", T::u32(Big)),
        ("tag", T::when(counted(), T::u32(Big))),
        (
            "class_name",
            T::when(
                tag().bit(31),
                T::switch(
                    tag().equal_to(E::lit(NEW_CLASS)),
                    vec![(1, T::cstr())],
                    T::at_origin(tag().and(E::lit(0x7fff_ffff)).add(E::lit(2)).sub(E::field("fKeylen")), T::cstr()),
                ),
            ),
        ),
        ("object", T::when(tag().bit(31), object_of(E::within(&["class_name"])))),
        // A count in front of a reference to something this record never
        // spelled out: the side reader steps over what the count covers, and
        // so does this. Nothing a writer makes has one, since a reference is
        // written as its four bytes alone.
        (
            "skipped",
            T::when(
                counted().both(tag().bit(31).equal_to(E::lit(0))).both(E::lit(1).less_than(tag())),
                T::bytes(first().and(E::lit(0x3fff_ffff)).sub(E::lit(4))),
            ),
        ),
    ]
}

/// A pointer to an object, named by the class it points at.
fn pointer() -> T {
    T::structure_named("ObjectRef", "class_name", "object", pointer_fields()).machinery(&["first", "tag"])
}

/// One entry of a `TList`: a pointer, and the option string every entry
/// carries after it, which nothing anyone writes has ever used.
fn list_entry() -> T {
    let mut fields = pointer_fields();
    fields.push(("option_len", T::u8()));
    fields.push(("option", T::utf8(E::field("option_len"))));
    T::structure_named("ListEntry", "class_name", "object", fields).machinery(&["first", "tag", "option_len"])
}

/// The members of a class written in place, which is a structure named by the
/// class and version.
fn members(table: &mut dyn Descriptions, class: &str, version: i32) -> R<Built> {
    let name = format!("{class} v{version}");
    // The collections and `TNamed` write their own streamers, which is not
    // what their descriptions say: a description lists the members of the C++
    // class, and a collection's streamer writes its elements.
    let tobject = || ("TObject", tobject());
    let fixed = match class {
        "TNamed" => Some(vec![tobject(), ("fName", super::tstring()), ("fTitle", super::tstring())]),
        "TObjArray" => Some(vec![
            tobject(),
            ("fName", super::tstring()),
            ("fSize", T::i32(Big)),
            ("fLowerBound", T::i32(Big)),
            ("elements", T::array(pointer(), E::field("fSize"))),
        ]),
        "TList" | "THashList" => Some(vec![
            tobject(),
            ("fName", super::tstring()),
            ("fSize", T::i32(Big)),
            ("elements", T::array(list_entry(), E::field("fSize"))),
        ]),
        _ => None,
    };
    if let Some(fields) = fixed {
        return Ok(Built::by_heart(T::structure(&name, fields)));
    }
    let (elements, from) = match described(table, class, version)? {
        Some((record, elements)) => (elements, Some(record)),
        None => match bootstrap().find(class, version) {
            Some(info) => (info.elements.iter().map(|e| (e.clone(), None)).collect(), None),
            None => return Err(EvalError::Failed(format!("no description of {class} in the file's StreamerInfo"))),
        },
    };
    let mut fields: Vec<(String, T)> = Vec::new();
    let mut members_from = Vec::new();
    for (i, (el, path)) in elements.iter().enumerate() {
        let before = &elements[..i];
        match member_ty(table, el, before)? {
            Ok(ty) => {
                fields.push((el.name.clone(), ty));
                members_from.push(path.clone());
            }
            // The rest of the object is a run of bytes the count still ends.
            Err(why) => {
                fields.push((el.name.clone(), T::bytes(E::Remaining)));
                members_from.push(path.clone());
                let ty = T::structure(&name, fields.iter().map(|(n, t)| (n.as_str(), t.clone())).collect())
                    .field_doc(&el.name, &why);
                return Ok(Built { ty, from, members_from });
            }
        }
    }
    // A branch's baskets, one record each, so that a walk over a tree's
    // branches can place every basket from the three arrays they are listed
    // in. No bytes: every number here is one of those arrays' elements.
    if class == "TBranch" && fields.iter().any(|(n, _)| n == "fBasketSeek") {
        let at = |array: &str| T::computed(E::elem_within(&[array, "values"], E::idx(), &[]));
        let entry = T::inline_structure(
            "BasketRef",
            vec![("seek", at("fBasketSeek")), ("bytes", at("fBasketBytes")), ("first_entry", at("fBasketEntry"))],
        );
        let written = E::field("fWriteBasket").at_most(E::field("fMaxBaskets")).at_least(E::lit(0));
        fields.push(("baskets".into(), T::array(entry, written)));
    }
    let ty = T::structure(&name, fields.iter().map(|(n, t)| (n.as_str(), t.clone())).collect());
    Ok(Built { ty, from, members_from })
}

/// What a member is read as, by the type code its element gives it. The inner
/// error is why it cannot be read, which ends the members there; the outer is
/// a description that could not be read at all.
fn member_ty(table: &mut dyn Descriptions, el: &Element, before: &[(Element, Option<Vec<usize>>)]) -> R<Result<T, String>> {
    if el.is_base() {
        return Ok(Ok(in_place(&el.name)));
    }
    let code = el.etype;
    let unread = |what: String| Ok(Err(what));
    let (basic, shape) = match code {
        0..=19 => (code, 0),
        20..=39 => (code - 20, 1),
        40..=59 => (code - 40, 2),
        _ => (-1, 0),
    };
    if basic >= 0 {
        let Some(one) = basic_ty(basic) else {
            return unread(format!("{} is basic type {basic} ({}), which is not read here", el.name, el.type_name));
        };
        return Ok(Ok(match shape {
            0 => one,
            1 => T::array(one, E::lit(el.array_length.max(0))),
            _ => {
                let Some(count) = count_of(table, &el.count_name, before)? else {
                    return unread(format!("{} is counted by {}, which is not a member before it", el.name, el.count_name));
                };
                T::inline_structure(&el.type_name, vec![("marker", T::u8()), ("values", T::array(one, count.at_least(E::lit(0))))])
                    .machinery(&["marker"])
            }
        }));
    }
    Ok(match code {
        // An object written where it stands, with no pointer in front of it.
        61 | 62 | 63 | 68 => Ok(in_place(el.type_name.trim_end_matches('*').trim())),
        // A pointer, which may be nothing and may be a reference back.
        64 | 69 | 70 => Ok(pointer()),
        65 => Ok(super::tstring()),
        _ => Err(format!("{} has type code {code} ({}), which is not read here", el.name, el.type_name)),
    })
}

/// A basic type, by its code less any array offset: the same widths
/// `read_basic` reads, a truncated double as the double it is stored as and a
/// half-width float as the float.
fn basic_ty(code: i32) -> Option<T> {
    let int = |bits| T::Int { bits, endian: Big };
    let uint = |bits| T::UInt { bits, endian: Big };
    Some(match code {
        1 | 10 => int(8),
        2 => int(16),
        3 | 6 => int(32),
        4 | 16 => int(64),
        5 | 19 => T::F32(Big),
        8 | 9 => T::F64(Big),
        11 | 18 => uint(8),
        12 => uint(16),
        13 | 15 => uint(32),
        14 | 17 => uint(64),
        _ => return None,
    })
}

/// Where the member that counts a pointer is, as an expression asked from
/// inside the pointer: a member of this class written before it, or one of a
/// base class, which is a structure of its own here and is reached through it.
fn count_of(table: &mut dyn Descriptions, name: &str, before: &[(Element, Option<Vec<usize>>)]) -> R<Option<E>> {
    if before.iter().any(|(e, _)| !e.is_base() && e.name == name) {
        return Ok(Some(E::field(name)));
    }
    for (base, _) in before.iter().filter(|(e, _)| e.is_base()) {
        if let Some(mut path) = in_base(table, &base.name, base.base_version, name, BASE_DEPTH)? {
            path.insert(0, base.name.clone());
            let names: Vec<&str> = path.iter().map(String::as_str).collect();
            return Ok(Some(E::within(&names)));
        }
    }
    Ok(None)
}

/// The path from a base class's field to a member called `name` somewhere in
/// it, through its own bases.
fn in_base(table: &mut dyn Descriptions, class: &str, version: i32, name: &str, depth: usize) -> R<Option<Vec<String>>> {
    if depth == 0 {
        return Ok(None);
    }
    let elements = match described(table, class, version)? {
        Some((_, elements)) => elements.into_iter().map(|(e, _)| e).collect::<Vec<_>>(),
        None => match bootstrap().find(class, version) {
            Some(info) => info.elements.clone(),
            None => return Ok(None),
        },
    };
    if elements.iter().any(|e| !e.is_base() && e.name == name) {
        return Ok(Some(vec!["members".into(), name.into()]));
    }
    for base in elements.iter().filter(|e| e.is_base()) {
        if let Some(mut path) = in_base(table, &base.name, base.base_version, name, depth - 1)? {
            path.insert(0, base.name.clone());
            path.insert(0, "members".into());
            return Ok(Some(path));
        }
    }
    Ok(None)
}

/// The description of `class` at `version` among the file's, and its elements
/// with where each is: the one with that version, or the only one there is of
/// the class, or the first where there are several and none matches. Nothing
/// when the file does not describe the class, or when its descriptions are out
/// of sight from where the object is.
fn described(table: &mut dyn Descriptions, class: &str, version: i32) -> R<Option<(Vec<usize>, Vec<(Element, Option<Vec<usize>>)>)>> {
    let mut first = None;
    let mut exact = None;
    for record in table.records()? {
        let Some(name) = text_at(table, &record, &["TNamed", "members", "fName", "text"])? else { continue };
        if name != class {
            continue;
        }
        let v = int_at(table, &record, &["fClassVersion"])?;
        if v == Some(version as i128) {
            exact = Some(record);
            break;
        }
        if first.is_none() {
            first = Some(record);
        }
    }
    let Some(record) = exact.or(first) else { return Ok(None) };
    let elements = elements_of(table, &record)?;
    Ok(Some((record, elements)))
}

/// Every element of one `TStreamerInfo`, read into the shape the side reader
/// reads them into, with the path of each.
fn elements_of(table: &mut dyn Descriptions, record: &[usize]) -> R<Vec<(Element, Option<Vec<usize>>)>> {
    let Some(list) = found(table.find(record, &["fElements", "object", "members", "elements"]))? else { return Ok(Vec::new()) };
    let n = table.count(&list)?;
    let mut out = Vec::new();
    for i in 0..n {
        let i = i.to_string();
        let Some(item) = found(table.find(&list, &[&i]))? else { continue };
        let Some(kind) = text_at(table, &item, &["class_name"])? else { continue };
        let Some(m) = found(table.find(&item, &["object", "members"]))? else { continue };
        out.push((element(table, &kind, &m)?, Some(item)));
    }
    Ok(out)
}

/// One element, from the members of its object. The members a subclass adds
/// are its own; the rest are in its `TStreamerElement` base, and the name in
/// that one's `TNamed`.
fn element(table: &mut dyn Descriptions, kind: &str, m: &[usize]) -> R<Element> {
    let element_base: &[&str] = match kind {
        "TStreamerElement" => &[],
        "TStreamerSTLstring" => &["TStreamerSTL", "members", "TStreamerElement", "members"],
        _ => &["TStreamerElement", "members"],
    };
    let at = |names: &[&str]| -> Vec<String> { element_base.iter().chain(names).map(|s| s.to_string()).collect() };
    let own = |p: Vec<String>| p;
    let mut max_index = [0i32; 5];
    for (k, slot) in max_index.iter_mut().enumerate() {
        let k = k.to_string();
        *slot = int_path(table, m, &at(&["fMaxIndex", &k]))?.unwrap_or(0) as i32;
    }
    Ok(Element {
        kind: kind.to_string(),
        name: text_path(table, m, &at(&["TNamed", "members", "fName", "text"]))?.unwrap_or_default(),
        title: text_path(table, m, &at(&["TNamed", "members", "fTitle", "text"]))?.unwrap_or_default(),
        etype: int_path(table, m, &at(&["fType"]))?.unwrap_or(-1) as i32,
        size: int_path(table, m, &at(&["fSize"]))?.unwrap_or(0) as i32,
        array_length: int_path(table, m, &at(&["fArrayLength"]))?.unwrap_or(0) as i32,
        array_dim: int_path(table, m, &at(&["fArrayDim"]))?.unwrap_or(0) as i32,
        max_index,
        type_name: text_path(table, m, &at(&["fTypeName", "text"]))?.unwrap_or_default(),
        base_version: int_path(table, m, &own(vec!["fBaseVersion".into()]))?.unwrap_or(0) as i32,
        count_name: text_path(table, m, &own(vec!["fCountName".into(), "text".into()]))?.unwrap_or_default(),
        count_class: text_path(table, m, &own(vec!["fCountClass".into(), "text".into()]))?.unwrap_or_default(),
    })
}

/// A path that may not be there, where a node on the way that will not read
/// counts as not there. Waiting for bytes is still waiting.
fn found(got: R<Option<Vec<usize>>>) -> R<Option<Vec<usize>>> {
    match got {
        Ok(p) => Ok(p),
        Err(e) if e.interrupted() => Err(e),
        Err(_) => Ok(None),
    }
}

fn text_at(table: &mut dyn Descriptions, from: &[usize], names: &[&str]) -> R<Option<String>> {
    let Some(p) = found(table.find(from, names))? else { return Ok(None) };
    match table.text(&p) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.interrupted() => Err(e),
        Err(_) => Ok(None),
    }
}

fn int_at(table: &mut dyn Descriptions, from: &[usize], names: &[&str]) -> R<Option<i128>> {
    let Some(p) = found(table.find(from, names))? else { return Ok(None) };
    match table.int(&p) {
        Ok(v) => Ok(v),
        Err(e) if e.interrupted() => Err(e),
        Err(_) => Ok(None),
    }
}

fn text_path(table: &mut dyn Descriptions, from: &[usize], names: &[String]) -> R<Option<String>> {
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    text_at(table, from, &names)
}

fn int_path(table: &mut dyn Descriptions, from: &[usize], names: &[String]) -> R<Option<i128>> {
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    int_at(table, from, &names)
}

/// Shared rather than made per template, since it holds nothing.
pub(super) fn builder() -> Arc<dyn SchemaBuilder> {
    Arc::new(Streamers)
}
