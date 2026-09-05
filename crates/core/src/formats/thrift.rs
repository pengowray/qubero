//! Thrift's compact protocol: the wire says the shape, a schema says the names.
//!
//! A compact struct is a run of fields ending at a zero byte, and every field
//! opens with one byte holding two numbers: how far its id is from the id of
//! the field before it, and what kind of value follows. That is the whole of
//! what a reader needs to walk one, and it is why a file written against a
//! newer version of a schema still reads: a field nobody has heard of still
//! says how long it is.
//!
//! So the parse comes off the wire and the names come off a schema, and the
//! two are kept apart here. [`Struct`] is a schema: which id is called what,
//! which ids hold a struct of their own and which one, which hold an integer
//! whose values have names. Nothing in it decides how many bytes anything
//! takes. An id the schema does not mention is read exactly as correctly as
//! one it does, and shown by its number.
//!
//! Every integer but the byte is zigzagged (see [`Ty::Zigzag`]), which is the
//! part that cannot be guessed: read as a plain LEB128 the numbers come out
//! doubled, and read as a sign-extended one they come out doubled and half of
//! them negative. A double is eight bytes little-endian, unlike the same
//! protocol family's binary flavour, where it is big-endian.
//!
//! ## Where the running id comes from
//!
//! The delta is measured from the previous field of the same struct, so the
//! id is a running sum and not something written down. [`Expr::Prev`] is that
//! sum: field `n`'s id is field `n - 1`'s id plus this delta, and `Prev` is
//! zero for the first element of a list, which is where Thrift's own counter
//! starts. A nested struct is a list of its own, so the counter resets when
//! one opens and picks up again when it closes, without anything having to
//! say so.
//!
//! A delta of zero means the id is written out instead, as a zigzag varint
//! after the header byte. The switch that reads it is keyed on the whole
//! header byte rather than on the delta, because the stop byte is a delta of
//! zero as well and there is no id after it.
//!
//! ## What is not read
//!
//! A union is a struct with one field set, and reads as one. Which member of
//! a union a struct is, is not called out.
//!
//! A type nibble above 13 is not a type, and a field carrying one takes the
//! rest of its container as bytes. Nothing after such a byte can be placed:
//! the length of every field is the type's to say, and an unknown type says
//! nothing. Stopping loudly beats walking on into the middle of a number.

use crate::template::{Endian::{Big, Little}, Expr as E, Ty as T, Until};

/// The name the generic struct is registered under: what a field whose id no
/// schema mentions reads as, and what the schema's own gaps fall back to.
pub const GENERIC: &str = "thrift.Struct";

/// The generic list and map, registered under these names for the same reason.
///
/// A list of lists cannot be written out, because writing it out is what does
/// not stop: the type of an element is a switch that has a list in it. So a
/// collection one level down is named rather than built, and the name is what
/// closes the loop. A schema still reaches the first level, which is where the
/// lists a schema knows about are: a row group's columns, a schema's elements.
const GENERIC_LIST: &str = "thrift.List";
const GENERIC_MAP: &str = "thrift.Map";

/// What one struct of a schema is called and what its fields are.
///
/// Only the fields worth naming need be here. A field left out is still read.
pub struct Struct {
    pub name: &'static str,
    pub fields: &'static [Field],
}

/// One numbered field of a schema: its id, what to call it, and anything the
/// wire cannot say about it.
pub struct Field {
    pub id: i128,
    pub name: &'static str,
    pub what: What,
}

/// What a schema knows about a field beyond its name.
pub enum What {
    /// Nothing: the wire's own type is the whole story. A count, a length, a
    /// flag.
    Plain,
    /// Binary holding text. Thrift has one type for both, so only a schema can
    /// tell a name from a blob.
    Text,
    /// An integer whose values have names, given as they are written in the
    /// schema: the enum's name and its cases.
    Enum(&'static str, &'static [(i128, &'static str)]),
    /// A struct, by the name of another [`Struct`] in the same schema. The
    /// wire says whether there is one of them or a list of them, so this
    /// covers a field, a list, a set and the values of a map alike.
    Struct(&'static str),
}

/// Every named type a schema needs, ready for `Template::with_type`.
///
/// Struct `Foo` of the schema is registered as `<prefix>.Foo`, so two formats
/// that both have a `Header` can sit in one table. The generic struct is
/// registered too, under [`GENERIC`], since every schema falls back to it.
pub fn types(prefix: &str, structs: &[Struct]) -> Vec<(String, T)> {
    let mut out: Vec<(String, T)> = structs
        .iter()
        .map(|s| (format!("{prefix}.{}", s.name), struct_ty(prefix, s.name, Some(s))))
        .collect();
    out.push((GENERIC.to_string(), struct_ty(prefix, "ThriftStruct", None)));
    out.push((GENERIC_LIST.to_string(), list_ty(prefix, None)));
    out.push((GENERIC_MAP.to_string(), map_ty(prefix, None)));
    out
}

/// One struct: the fields it holds, up to and including the byte that ends it.
fn struct_ty(prefix: &str, name: &str, schema: Option<&Struct>) -> T {
    T::structure_named(
        name,
        "",
        "fields",
        vec![("fields", T::repeat(field_ty(prefix, schema), Until::FieldValue { field: "kind".into(), value: 0 }))],
    )
}

/// One field: its header byte, what that byte says, and its value.
///
/// `hdr` and `kind` are the structure's own machinery, but `id` is not: it is
/// the number the schema names the field by, and a reader checking a footer
/// against a specification is reading exactly that.
fn field_ty(prefix: &str, schema: Option<&Struct>) -> T {
    // A header byte of 0x01 through 0x0F is a delta of zero with a real type
    // in it, which is the one shape that writes its id out. A byte of zero is
    // the stop byte, and everything else carries its delta in the top nibble.
    let explicit: Vec<(i128, T)> = (1..=15).map(|k| (k, id_ty(schema, T::zigzag()))).collect();
    let running = id_ty(schema, T::computed(E::prev("id").add(E::field("hdr").shr(E::lit(4)))));
    T::structure_named(
        "ThriftField",
        "id",
        "value",
        vec![
            ("hdr", T::u8()),
            ("kind", T::computed(E::field("hdr").and(E::lit(15)))),
            ("id", T::switch(E::field("hdr"), explicit, running)),
            ("value", value_ty(prefix, schema, E::field("kind"), false, false)),
        ],
    )
    .machinery(&["hdr", "kind"])
}

/// The field id, named by the schema where the schema has a name for it.
fn id_ty(schema: Option<&Struct>, inner: T) -> T {
    let Some(s) = schema else { return inner };
    let cases: Vec<(i128, &str)> = s.fields.iter().map(|f| (f.id, f.name)).collect();
    T::enumeration(&format!("{} field", s.name), inner, &cases)
}

/// The value of a field, or of one element of a list, set or map.
///
/// `kind` is where the type nibble is: in a field's header byte, or in a
/// collection's. `in_collection` says which, because a boolean is the one type
/// whose value is in the nibble when it is a field and in a byte of its own
/// when it is an element. `nested` says this is already inside a collection,
/// so a collection found here is named rather than built. See [`GENERIC_LIST`].
fn value_ty(prefix: &str, schema: Option<&Struct>, kind: E, in_collection: bool, nested: bool) -> T {
    let bools: Vec<(i128, T)> = if in_collection {
        // Every element is a byte, and the nibble said only that they are
        // booleans, so both nibble values lead to the same type.
        vec![(1, boolean(T::u8())), (2, boolean(T::u8()))]
    } else {
        vec![(1, boolean(T::computed(E::lit(1)))), (2, boolean(T::computed(E::lit(2))))]
    };
    let mut cases: Vec<(i128, T)> = bools;
    cases.push((0, T::bytes(E::lit(0))));
    cases.push((3, T::Int { bits: 8, endian: Big }));
    cases.push((4, T::zigzag()));
    cases.push((5, by_id(schema, T::zigzag(), &|f| match &f.what {
        What::Enum(name, cases) => Some(T::enumeration(name, T::zigzag(), cases)),
        _ => None,
    })));
    cases.push((6, T::zigzag()));
    cases.push((7, T::F64(Little)));
    cases.push((8, by_id(schema, binary(false), &|f| matches!(f.what, What::Text).then(|| binary(true)))));
    for k in [9, 10] {
        cases.push((k, if nested { T::Named(GENERIC_LIST.into()) } else { list_ty(prefix, schema) }));
    }
    cases.push((11, if nested { T::Named(GENERIC_MAP.into()) } else { map_ty(prefix, schema) }));
    cases.push((12, by_id(schema, T::Named(GENERIC.into()), &|f| match f.what {
        What::Struct(n) => Some(T::Named(format!("{prefix}.{n}").into())),
        _ => None,
    })));
    // A UUID is sixteen bytes and nothing about it is a number worth reading
    // as one.
    cases.push((13, T::bytes(E::lit(16))));
    // Not a type. See the note at the top about why this takes everything.
    T::switch(kind, cases, T::bytes(E::Remaining))
}

/// A value whose type the schema refines for some ids and not others: a switch
/// on the field id, with `generic` for every id the schema says nothing about.
fn by_id(schema: Option<&Struct>, generic: T, refine: &dyn Fn(&Field) -> Option<T>) -> T {
    let Some(s) = schema else { return generic };
    let cases: Vec<(i128, T)> = s.fields.iter().filter_map(|f| refine(f).map(|t| (f.id, t))).collect();
    if cases.is_empty() { generic } else { T::switch(E::field("id"), cases, generic) }
}

/// True and false, by name. The value is in the type nibble for a field and in
/// a byte for an element, and either way it is 1 or 2 rather than 1 or 0.
fn boolean(inner: T) -> T {
    T::enumeration("bool", inner, &[(1, "true"), (2, "false")])
}

/// Binary: a length and then that many bytes. `text` is what the schema says
/// the bytes mean; Thrift itself does not distinguish them.
fn binary(text: bool) -> T {
    let body = if text { T::utf8(E::field("len")) } else { T::bytes(E::field("len")) };
    T::inline_structure("Binary", vec![("len", T::leb_u()), ("bytes", body)]).machinery(&["len"])
}

/// A list or a set: a header nibble each for how many and of what, with the
/// count written out separately when it does not fit in four bits.
fn list_ty(prefix: &str, schema: Option<&Struct>) -> T {
    T::structure_named(
        "ThriftList",
        "",
        "elems",
        vec![
            ("hdr", T::u8()),
            ("elem_kind", T::computed(E::field("hdr").and(E::lit(15)))),
            ("short_count", T::computed(E::field("hdr").shr(E::lit(4)))),
            // Fifteen is not a count: it says the count follows.
            ("long_count", T::switch(E::field("short_count"), vec![(15, T::leb_u())], T::bytes(E::lit(0)))),
            (
                "count",
                T::switch(
                    E::field("short_count"),
                    vec![(15, T::computed(E::field("long_count")))],
                    T::computed(E::field("short_count")),
                ),
            ),
            ("elems", T::array(value_ty(prefix, schema, E::field("elem_kind"), true, true), E::field("count"))),
        ],
    )
    .machinery(&["hdr", "elem_kind", "short_count", "long_count"])
}

/// A map: how many pairs, then one byte holding both types, then the pairs.
/// An empty map writes the count and stops, so the type byte is not there to
/// be read.
fn map_ty(prefix: &str, schema: Option<&Struct>) -> T {
    let entry = T::structure(
        "ThriftEntry",
        vec![
            ("key", value_ty(prefix, None, E::field("key_kind"), true, true)),
            ("value", value_ty(prefix, schema, E::field("value_kind"), true, true)),
        ],
    );
    T::structure_named(
        "ThriftMap",
        "",
        "entries",
        vec![
            ("count", T::leb_u()),
            ("kinds", T::switch(E::field("count"), vec![(0, T::bytes(E::lit(0)))], T::u8())),
            ("key_kind", T::computed(E::field("kinds").shr(E::lit(4)))),
            ("value_kind", T::computed(E::field("kinds").and(E::lit(15)))),
            ("entries", T::array(entry, E::field("count"))),
        ],
    )
    .machinery(&["kinds", "key_kind", "value_kind"])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, eval::{Evaluator, Value}, source::MemSource, template::Template};

    const ROOT: Struct = Struct {
        name: "Root",
        fields: &[
            Field { id: 1, name: "count", what: What::Plain },
            Field { id: 3, name: "label", what: What::Text },
            Field { id: 20, name: "big", what: What::Plain },
            Field { id: 21, name: "nums", what: What::Plain },
            Field { id: 22, name: "inner", what: What::Struct("Inner") },
            Field { id: 23, name: "codec", what: What::Enum("Codec", &[(0, "none"), (1, "snappy")]) },
        ],
    };
    const INNER: Struct = Struct { name: "Inner", fields: &[Field { id: 1, name: "flag", what: What::Plain }] };

    fn template() -> Template {
        let mut t = Template::new("test", T::structure("Test", vec![("meta", T::Named("t.Root".into()))]));
        for (name, ty) in types("t", &[ROOT, INNER]) {
            t = t.with_type(&name, ty);
        }
        t
    }

    /// The fields of the struct at the front of `bytes`, as (id, value).
    fn fields(bytes: Vec<u8>) -> Vec<(i128, Value)> {
        let d = Document::new(MemSource(bytes));
        let mut e = Evaluator::new(template());
        let n = e.node(&d, &[0, 0]).expect("the field list reads").child_count as usize;
        (0..n)
            .map(|i| {
                let id = e.node(&d, &[0, 0, i, 2]).expect("an id reads").value;
                let raw = match id {
                    Value::Enum { raw, .. } => raw,
                    Value::Int(v) => v,
                    other => panic!("an id is a number, not {other:?}"),
                };
                (raw, e.node(&d, &[0, 0, i, 3]).expect("a value reads").value)
            })
            .collect()
    }

    /// One field of every shape, in one struct: a delta, a longer delta, an id
    /// written out because the gap is over fifteen, a list, and a nested
    /// struct whose own ids start again from zero.
    fn sampler() -> Vec<u8> {
        vec![
            0x15, 0x0e, // field 1, i32, zigzag 14 = 7
            0x28, 0x02, b'h', b'i', // field 3 (delta 2), binary "hi"
            0x06, 0x28, 0x01, // field 20: delta 0, so the id is written; i64 -1
            0x19, 0x25, 0x02, 0x04, // field 21, list of two i32: 1 and 2
            0x1c, 0x11, 0x00, // field 22, struct: its field 1 is true, then stop
            0x15, 0x02, // field 23, i32, zigzag 2 = 1
            0x00, // stop
        ]
    }

    #[test]
    fn the_ids_are_the_deltas_added_up() {
        let got: Vec<i128> = fields(sampler()).iter().map(|(id, _)| *id).collect();
        // The last is the stop byte, whose type nibble is zero and whose id is
        // whatever the running sum reached.
        assert_eq!(&got[..6], &[1, 3, 20, 21, 22, 23]);
    }

    #[test]
    fn an_id_past_a_gap_of_fifteen_is_written_out() {
        // The third field's id is 20 and the one before it is 3, so the delta
        // nibble cannot hold it: the header is a bare type and the id follows
        // as a zigzag varint, which is three bytes rather than one plus a
        // value.
        let d = Document::new(MemSource(sampler()));
        let mut e = Evaluator::new(template());
        assert_eq!(e.node(&d, &[0, 0, 2, 2]).unwrap().size_bits, 8, "the id takes a byte of its own");
        assert_eq!(e.node(&d, &[0, 0, 0, 2]).unwrap().size_bits, 0, "a delta leaves the id unwritten");
    }

    #[test]
    fn the_numbers_are_unzigzagged() {
        let got = fields(sampler());
        assert_eq!(got[0].1.as_int(), Some(7));
        assert_eq!(got[2].1.as_int(), Some(-1));
    }

    #[test]
    fn a_schema_names_the_fields_and_the_values() {
        let d = Document::new(MemSource(sampler()));
        let mut e = Evaluator::new(template());
        let id = e.node(&d, &[0, 0, 1, 2]).unwrap().value;
        assert!(matches!(id, Value::Enum { name: Some(ref n), .. } if n == "label"), "got {id:?}");
        // Field 23 is an i32 the schema says holds a codec.
        let codec = e.node(&d, &[0, 0, 5, 3]).unwrap().value;
        assert!(matches!(codec, Value::Enum { name: Some(ref n), .. } if n == "snappy"), "got {codec:?}");
    }

    #[test]
    fn text_a_schema_names_reads_as_text() {
        let d = Document::new(MemSource(sampler()));
        let mut e = Evaluator::new(template());
        // Binary is a length and then its bytes, so the text is the second of
        // the two. Thrift has one type for a name and for a blob, and only the
        // schema saying `Text` is what makes this a word rather than 68 69.
        assert_eq!(e.node(&d, &[0, 0, 1, 3, 1]).unwrap().value, Value::Str("hi".into()));
        assert_eq!(e.node(&d, &[0, 0, 1, 3, 0]).unwrap().value.as_int(), Some(2), "the length is read as one");
    }

    #[test]
    fn a_nested_struct_starts_its_ids_again() {
        let d = Document::new(MemSource(sampler()));
        let mut e = Evaluator::new(template());
        // meta.fields[4].value is an Inner, whose own fields[0] has id 1
        // rather than 23: the delta is measured inside the struct it is in.
        let id = e.node(&d, &[0, 0, 4, 3, 0, 0, 2]).unwrap().value;
        assert!(matches!(id, Value::Enum { raw: 1, name: Some(ref n), .. } if n == "flag"), "got {id:?}");
        let flag = e.node(&d, &[0, 0, 4, 3, 0, 0, 3]).unwrap().value;
        assert!(matches!(flag, Value::Enum { raw: 1, name: Some(ref n), .. } if n == "true"), "got {flag:?}");
    }

    #[test]
    fn a_list_of_fifteen_or_more_writes_its_count_separately() {
        // Field 1, a list of sixteen i32s, each the number one.
        let mut b = vec![0x19, 0xf5, 0x10];
        b.extend(std::iter::repeat(0x02).take(16));
        b.push(0x00);
        let d = Document::new(MemSource(b));
        let mut e = Evaluator::new(template());
        // meta.fields[0].value is the list; its elems field is the fifth.
        let elems = e.node(&d, &[0, 0, 0, 3, 5]).expect("the elements read");
        assert_eq!(elems.child_count, 16);
        assert_eq!(e.node(&d, &[0, 0, 0, 3, 5, 15]).unwrap().value.as_int(), Some(1));
    }

    #[test]
    fn an_empty_map_writes_no_type_byte() {
        // Field 1, a map of no pairs: the count is zero and nothing follows.
        let d = Document::new(MemSource(vec![0x1b, 0x00, 0x00]));
        let mut e = Evaluator::new(template());
        let map = e.node(&d, &[0, 0, 0, 3]).expect("the map reads");
        assert_eq!(map.size_bits, 8, "a count of zero is the whole of an empty map");
    }

    #[test]
    fn a_map_reads_its_pairs() {
        // Field 1, one pair: an i32 key of 1 and a binary value of "hi".
        let d = Document::new(MemSource(vec![0x1b, 0x01, 0x58, 0x02, 0x02, b'h', b'i', 0x00]));
        let mut e = Evaluator::new(template());
        assert_eq!(e.node(&d, &[0, 0, 0, 3, 4, 0, 0]).unwrap().value.as_int(), Some(1));
        let v = e.node(&d, &[0, 0, 0, 3, 4, 0, 1]).unwrap();
        assert_eq!(v.size_bits, 3 * 8, "a length and two bytes");
    }
}
