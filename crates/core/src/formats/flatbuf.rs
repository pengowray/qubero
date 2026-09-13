//! FlatBuffers: a table finds its own fields through a table of offsets.
//!
//! Thrift's compact protocol writes its fields one after another and says
//! what each one is as it goes, so a reader walks it. A FlatBuffers table does
//! the opposite, which is the whole reason the format exists: nothing in it is
//! walked. A table opens with a signed 32-bit number, and the table's *vtable*
//! is that many bytes before it (after it, when the number is negative). The
//! vtable is two 16-bit sizes, its own and the table's, and then one 16-bit
//! offset per field id, counted from the start of the table. An offset of zero
//! says the field was not written. So reading field 3 of a table is two reads
//! and an addition, whatever else the table holds, and that is what lets a
//! program use the bytes where they lie instead of unpacking them.
//!
//! That shape is said here with nothing new. A table is a structure whose
//! first field is that signed offset and whose second is the vtable, placed
//! with an [`At`](T::At) at `start_of(vtable_offset) - vtable_offset`. Every
//! field after those is placed the same way, at `start_of(vtable_offset)`
//! plus its own entry in the vtable, and is only there when the vtable is long
//! enough to have an entry for it and the entry is not zero: a
//! [`When`](T::When) over those two questions. The lookup by id is a lookup by
//! name, because the vtable is read as a structure too, one field per id,
//! named from the schema. `vtable.bodyLength` is the entry for `bodyLength`.
//!
//! A string, a vector, a table inside a table and the value of a union are all
//! an unsigned 32-bit offset, and that one is counted from where the offset
//! itself sits rather than from the table. [`Expr::StartOf`](E::StartOf) is
//! what names that place, so every such field reads as the offset and then
//! what it points at, placed at `start_of(offset) + offset`. Scalars and
//! structs are stored in the table itself.
//!
//! Every address here counts from the nearest [`Ty::Origin`](T::Origin), which
//! is how `start_of` counts too, so a FlatBuffer written partway into another
//! file reads the same as one on its own.
//!
//! ## What a schema says
//!
//! Nothing in the bytes says what a field is, which is the other half of the
//! difference from Thrift. A Thrift field carries a type nibble, so a reader
//! with no schema still knows how long it is. A FlatBuffers field is an offset
//! in a vtable and nothing more: without the schema it is not even known how
//! many bytes it takes. So the schema here is not a layer of names over a
//! parse the way [`crate::formats::thrift`]'s is. It is what the parse is
//! made of. [`Table`] lists the fields in the order the `.fbs` file declares
//! them, which is what numbers them, and [`What`] says how each is stored.
//!
//! A field the schema does not know is still visible: the vtable says it is
//! there and where, in `unknown_fields`, which is what an old reader of a
//! newer file sees. What it holds cannot be said.
//!
//! ## What is not read
//!
//! A vector of unions, and the `bool` of a struct wider than a byte, which no
//! schema read with this has. A buffer's optional four-byte file identifier is
//! not looked for either: Arrow, the one format that reads with this so far,
//! does not write one.

use crate::template::{Endian::Little, Expr as E, Ty as T};

/// A number stored where the table or struct is, rather than behind an offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scalar {
    Bool,
    Byte,
    UByte,
    Short,
    UShort,
    Int,
    UInt,
    Long,
    ULong,
    Float,
    Double,
}

impl Scalar {
    /// How many bytes one takes, which is also what a struct aligns it to.
    pub fn bytes(self) -> i128 {
        match self {
            Scalar::Bool | Scalar::Byte | Scalar::UByte => 1,
            Scalar::Short | Scalar::UShort => 2,
            Scalar::Int | Scalar::UInt | Scalar::Float => 4,
            Scalar::Long | Scalar::ULong | Scalar::Double => 8,
        }
    }

    fn ty(self) -> T {
        match self {
            Scalar::Bool => T::enumeration("bool", T::u8(), &[(0, "false"), (1, "true")]),
            Scalar::Byte => T::Int { bits: 8, endian: Little },
            Scalar::UByte => T::u8(),
            Scalar::Short => T::Int { bits: 16, endian: Little },
            Scalar::UShort => T::u16(Little),
            Scalar::Int => T::i32(Little),
            Scalar::UInt => T::u32(Little),
            Scalar::Long => T::Int { bits: 64, endian: Little },
            Scalar::ULong => T::u64(Little),
            Scalar::Float => T::F32(Little),
            Scalar::Double => T::F64(Little),
        }
    }
}

/// What the elements of a vector are.
pub enum Of {
    Scalar(Scalar),
    /// An enum, by its name in the schema and its cases.
    Enum(Scalar, &'static str, &'static [(i128, &'static str)]),
    String,
    /// Tables, by name: the vector holds an offset to each.
    Table(&'static str),
    /// Structs, by name: the vector holds the structs themselves, back to back.
    Struct(&'static str),
}

/// How one field of a table is stored.
pub enum What {
    Scalar(Scalar),
    Enum(Scalar, &'static str, &'static [(i128, &'static str)]),
    String,
    Table(&'static str),
    /// A struct, stored in the table rather than behind an offset.
    Struct(&'static str),
    Vector(Of),
    /// A union, by its name and its members. A union takes two ids: the first
    /// is a byte saying which member this is, called `<name>_type` the way
    /// `flatc` calls it, and the second is the offset to that member's table.
    /// Zero is `NONE` in every union and is not listed.
    Union(&'static str, &'static [(i128, &'static str)]),
}

/// One field of a table, as the `.fbs` file declares it.
pub struct Field {
    pub name: &'static str,
    pub what: What,
}

/// A table of a schema. Fields are listed in the order the schema declares
/// them, because that order is what numbers them: no id is written down.
pub struct Table {
    pub name: &'static str,
    pub fields: &'static [Field],
}

/// A struct of a schema: fixed fields at fixed places, padded to line each up
/// with its own size and the whole to its widest.
pub struct Struct {
    pub name: &'static str,
    pub fields: &'static [(&'static str, Scalar)],
}

impl Table {
    /// Every name that takes an id, in id order: a union gives its type byte
    /// and its value.
    fn slots(&self) -> Vec<String> {
        let mut out = Vec::new();
        for f in self.fields {
            if let What::Union(..) = f.what {
                out.push(format!("{}_type", f.name));
            }
            out.push(f.name.to_string());
        }
        out
    }

    /// The id of the field called `name`, including a union's `_type` byte.
    pub fn id(&self, name: &str) -> Option<usize> {
        self.slots().iter().position(|s| s == name)
    }
}

/// Every table and struct of a schema, ready for `Template::with_type`, each
/// registered as `<prefix>.<Name>` so that two schemas can share a template.
pub fn types(prefix: &str, tables: &[Table], structs: &[Struct]) -> Vec<(String, T)> {
    let mut out: Vec<(String, T)> =
        tables.iter().map(|t| (format!("{prefix}.{}", t.name), table_ty(prefix, t))).collect();
    out.extend(structs.iter().map(|s| (format!("{prefix}.{}", s.name), struct_ty(s))));
    out
}

/// A whole buffer: the offset to its root table, which is the first four
/// bytes, and the table it points at.
pub fn buffer(prefix: &str, root: &str) -> T {
    T::structure_named("FlatBuffer", "", "root", vec![("root", offset("table", named(prefix, root), root))])
}

fn named(prefix: &str, name: &str) -> T {
    T::Named(format!("{prefix}.{name}").into())
}

/// Where the table this field belongs to starts: the one place every other
/// address in it is counted from.
fn table_start() -> E {
    E::start_of(E::field("vtable_offset"))
}

/// Whether field `name`, with id `id`, was written: the vtable has an entry
/// that far along, and the entry is not zero.
///
/// Both halves are needed and in this order. A vtable is cut short after the
/// last field a writer set, so a table written against an older schema, or
/// one whose later fields are all defaults, has no entry at all to read for
/// the fields past that, and asking for one would be asking for bytes that
/// belong to something else.
fn written(vtable: &[&str], id: usize, name: &str) -> E {
    let mut size: Vec<&str> = vtable.to_vec();
    size.push("vtable_size");
    let mut entry: Vec<&str> = vtable.to_vec();
    entry.push(name);
    E::within(&size)
        .greater_than(E::lit(4 + 2 * id as i128))
        .both(E::within(&entry).not_equal(E::lit(0)))
}

/// Whether field `name` of the table reached by `path` was written, asked from
/// outside that table. `path` goes down to the table itself: to `table` inside
/// an offset, for a table behind one.
pub fn is_written(path: &[&str], table: &Table, name: &str) -> E {
    let id = table.id(name).unwrap_or_else(|| panic!("{} has no field {name}", table.name));
    let mut vtable = path.to_vec();
    vtable.push("vtable");
    written(&vtable, id, name)
}

/// The number in scalar field `name` of the table reached by `path`, or
/// `default` where the table does not write it.
///
/// That is how every FlatBuffers reader answers, and it is why a writer leaves
/// a default out: a field holding its default costs no bytes at all. So a field
/// that is not here still has a value, and anything counting with it has to be
/// told what the schema says that value is. The listing still says the field
/// is absent, because it is.
pub fn read_or(path: &[&str], table: &Table, name: &str, default: i128) -> E {
    let mut value = path.to_vec();
    value.push(name);
    E::cond(is_written(path, table, name), E::within(&value), E::lit(default))
}

/// A vtable, read with one field per id the schema names. The ids past those
/// are still counted, so a file from a newer schema shows how many fields it
/// has that this one does not know.
fn vtable_ty(t: &Table) -> T {
    let slots = t.slots();
    let mut fields: Vec<(String, T)> = vec![("vtable_size".into(), T::u16(Little)), ("table_size".into(), T::u16(Little))];
    for (id, name) in slots.iter().enumerate() {
        let there = E::field("vtable_size").greater_than(E::lit(4 + 2 * id as i128));
        fields.push((name.clone(), T::when(there, T::u16(Little))));
    }
    let known = slots.len() as i128;
    let more = E::field("vtable_size").sub(E::lit(4)).div(E::lit(2)).sub(E::lit(known)).at_least(E::lit(0));
    fields.push(("unknown_fields".into(), T::array(T::u16(Little), more)));
    let mut machinery: Vec<&str> = vec!["vtable_size", "table_size"];
    machinery.extend(slots.iter().map(String::as_str));
    structure("VTable", fields).machinery(&machinery)
}

fn structure(name: &str, fields: Vec<(String, T)>) -> T {
    T::structure(name, fields.iter().map(|(n, t)| (n.as_str(), t.clone())).collect())
}

/// A table: the offset to its vtable, the vtable, and then each field where
/// the vtable says it is.
fn table_ty(prefix: &str, t: &Table) -> T {
    let mut fields: Vec<(String, T)> = vec![
        ("vtable_offset".into(), T::i32(Little)),
        ("vtable".into(), T::at_origin(table_start().sub(E::field("vtable_offset")), vtable_ty(t))),
    ];
    let mut id = 0;
    for f in t.fields {
        if let What::Union(name, cases) = f.what {
            let kind = format!("{}_type", f.name);
            let mut labels: Vec<(i128, &str)> = vec![(0, "NONE")];
            labels.extend(cases.iter().copied());
            fields.push((kind.clone(), placed(id, &kind, T::enumeration(name, T::u8(), &labels))));
            let which = E::cond(written(&["vtable"], id, &kind), E::within(&[kind.as_str()]), E::lit(0));
            let member = T::switch(which, cases.iter().map(|(v, n)| (*v, named(prefix, n))).collect(), T::bytes(E::lit(0)));
            fields.push((f.name.into(), placed(id + 1, f.name, offset("table", member, name))));
            id += 2;
            continue;
        }
        fields.push((f.name.into(), placed(id, f.name, value_ty(prefix, &f.what))));
        id += 1;
    }
    structure(t.name, fields).machinery(&["vtable_offset", "vtable"])
}

/// Field `name`, with id `id`, placed where its vtable entry says, and there
/// only when the vtable says it was written.
fn placed(id: usize, name: &str, ty: T) -> T {
    let at = table_start().add(E::within(&["vtable", name]));
    T::when(written(&["vtable"], id, name), T::at_origin(at, ty))
}

fn value_ty(prefix: &str, what: &What) -> T {
    match what {
        What::Scalar(s) => s.ty(),
        What::Enum(s, name, cases) => T::enumeration(name, s.ty(), cases),
        What::String => offset("string", string_ty(), "string"),
        What::Table(n) => offset("table", named(prefix, n), n),
        What::Struct(n) => named(prefix, n),
        What::Vector(of) => {
            let (elem, called) = elem_ty(prefix, of);
            offset("vector", vector_ty(elem, &called), &format!("{called}[]"))
        }
        // Built by `table_ty`, which is the one place that knows both ids.
        What::Union(..) => unreachable!("a union is two fields, built with its table"),
    }
}

fn elem_ty(prefix: &str, of: &Of) -> (T, String) {
    match of {
        Of::Scalar(s) => (s.ty(), s.ty().display_name()),
        Of::Enum(s, name, cases) => (T::enumeration(name, s.ty(), cases), name.to_string()),
        Of::String => (offset("string", string_ty(), "string"), "string".into()),
        Of::Table(n) => (offset("table", named(prefix, n), n), n.to_string()),
        Of::Struct(n) => (named(prefix, n), n.to_string()),
    }
}

/// An unsigned offset and what it points at, counted from where the offset
/// sits. `target` is what the pointed-at thing is called in the tree: `table`,
/// `string` or `vector`. `shown` is what it is, for the type column.
fn offset(target: &str, inner: T, shown: &str) -> T {
    let at = E::start_of(E::field("offset")).add(E::field("offset"));
    T::structure_named(
        &format!("offset \u{2192} {shown}"),
        "",
        target,
        vec![("offset", T::u32(Little)), (target, T::at_origin(at, inner))],
    )
    .machinery(&["offset"])
}

/// A string: a length, that many bytes of UTF-8, and a zero byte after them
/// that the length does not count. The zero is there so that a C program can
/// use the bytes as they lie, and it is part of the file, so it is shown.
fn string_ty() -> T {
    T::inline_structure(
        "string",
        vec![("length", T::u32(Little)), ("text", T::utf8(E::field("length"))), ("terminator", T::u8())],
    )
    .machinery(&["length", "terminator"])
}

/// A vector: how many elements, and then the elements. A vector of tables or
/// strings holds an offset per element, each counted from itself; a vector of
/// scalars or structs holds the elements.
fn vector_ty(elem: T, called: &str) -> T {
    T::structure_named(
        &format!("{called}[]"),
        "",
        "elements",
        vec![("count", T::u32(Little)), ("elements", T::array(elem, E::field("count")))],
    )
    .machinery(&["count"])
}

/// A struct, laid out as a C compiler would: each field at the next multiple
/// of its own size, and the whole padded to a multiple of its widest field.
/// The padding is bytes of the file, so it is named.
fn struct_ty(s: &Struct) -> T {
    let mut fields: Vec<(String, T)> = Vec::new();
    let mut pads: Vec<String> = Vec::new();
    let mut at = 0i128;
    let mut widest = 1i128;
    let pad = |fields: &mut Vec<(String, T)>, pads: &mut Vec<String>, n: i128| {
        if n > 0 {
            let name = if pads.is_empty() { "padding".to_string() } else { format!("padding_{}", pads.len() + 1) };
            fields.push((name.clone(), T::bytes(E::lit(n))));
            pads.push(name);
        }
    };
    for (name, scalar) in s.fields {
        let size = scalar.bytes();
        widest = widest.max(size);
        pad(&mut fields, &mut pads, (size - at % size) % size);
        at += (size - at % size) % size;
        fields.push((name.to_string(), scalar.ty()));
        at += size;
    }
    pad(&mut fields, &mut pads, (widest - at % widest) % widest);
    let pads: Vec<&str> = pads.iter().map(String::as_str).collect();
    structure(s.name, fields).machinery(&pads)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, eval::{Evaluator, Value}, source::MemSource, template::Template};

    const COLOUR: &[(i128, &str)] = &[(0, "Red"), (1, "Green")];

    /// A small schema with one of every way a field is stored, and a union.
    const TABLES: &[Table] = &[
        Table {
            name: "Monster",
            fields: &[
                Field { name: "hp", what: What::Scalar(Scalar::Short) },
                Field { name: "name", what: What::String },
                Field { name: "colour", what: What::Enum(Scalar::Byte, "Colour", COLOUR) },
                Field { name: "pos", what: What::Struct("Vec2") },
                Field { name: "friends", what: What::Vector(Of::Table("Monster")) },
                Field { name: "equipped", what: What::Union("Equipment", &[(1, "Weapon")]) },
                Field { name: "mana", what: What::Scalar(Scalar::Short) },
            ],
        },
        Table { name: "Weapon", fields: &[Field { name: "damage", what: What::Scalar(Scalar::Int) }] },
    ];
    const STRUCTS: &[Struct] = &[Struct { name: "Vec2", fields: &[("x", Scalar::Byte), ("y", Scalar::Int)] }];

    fn template() -> Template {
        let mut t = Template::new("fb", buffer("t", "Monster"));
        for (name, ty) in types("t", TABLES, STRUCTS) {
            t = t.with_type(&name, ty);
        }
        t
    }

    /// Bytes laid down back to front would be how a real builder does it; a
    /// test is clearer written front to back with the offsets worked out by
    /// hand, which is what this does.
    struct Buf(Vec<u8>);
    impl Buf {
        fn at(&self) -> usize {
            self.0.len()
        }
        fn u16(&mut self, v: u16) {
            self.0.extend_from_slice(&v.to_le_bytes());
        }
        fn u32(&mut self, v: u32) {
            self.0.extend_from_slice(&v.to_le_bytes());
        }
        fn i32(&mut self, v: i32) {
            self.0.extend_from_slice(&v.to_le_bytes());
        }
        fn put_u32(&mut self, at: usize, v: u32) {
            self.0[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
        fn align(&mut self, n: usize) {
            while self.0.len() % n != 0 {
                self.0.push(0);
            }
        }
    }

    /// A Monster with hp, name, pos, two friends (each with only hp) and an
    /// equipped Weapon, but no colour and no mana. Returns the bytes.
    ///
    /// Layout, by offset:
    ///   0   root offset
    ///   4   Monster vtable: 18 bytes, 7 slots of 8 (ids 0 to 6)
    ///   24  Monster table
    ///   ... strings, vectors and the other tables after it
    fn monster() -> Vec<u8> {
        let mut b = Buf(Vec::new());
        b.u32(0); // root, patched below
        // Monster vtable. ids: hp 0, name 1, colour 2, pos 3, friends 4,
        // equipped_type 5, equipped 6, mana 7. mana is past the end.
        let vt = b.at();
        b.u16(4 + 2 * 7); // vtable_size: seven entries
        b.u16(28); // table_size
        b.u16(4); // hp
        b.u16(8); // name
        b.u16(0); // colour: not written
        b.u16(12); // pos
        b.u16(20); // friends
        b.u16(6); // equipped_type
        b.u16(24); // equipped
        b.align(4);
        let table = b.at();
        b.put_u32(0, table as u32);
        b.i32((table - vt) as i32); // vtable_offset
        b.u16(150); // hp at +4
        b.0.push(1); // equipped_type at +6: Weapon
        b.0.push(0);
        let name_at = b.at();
        b.u32(0); // name at +8, patched
        b.0.extend_from_slice(&[0xfe, 0, 0, 0]); // pos.x at +12, then padding
        b.i32(-3); // pos.y at +16
        let friends_at = b.at();
        b.u32(0); // friends at +20
        let equipped_at = b.at();
        b.u32(0); // equipped at +24
        // The name.
        let s = b.at();
        b.put_u32(name_at, (s - name_at) as u32);
        b.u32(3);
        b.0.extend_from_slice(b"orc\0");
        b.align(4);
        // The friends vector: two offsets.
        let v = b.at();
        b.put_u32(friends_at, (v - friends_at) as u32);
        b.u32(2);
        let first = b.at();
        b.u32(0);
        let second = b.at();
        b.u32(0);
        // A vtable shared by both friends, then the two tables: hp only.
        let fvt = b.at();
        b.u16(6);
        b.u16(8);
        b.u16(4);
        b.align(4);
        for (slot, hp) in [(first, 10u16), (second, 20u16)] {
            let t = b.at();
            b.put_u32(slot, (t - slot) as u32);
            b.i32((t - fvt) as i32);
            b.u16(hp);
            b.u16(0);
        }
        // The weapon, whose vtable comes after it: a negative vtable offset.
        let w = b.at();
        b.put_u32(equipped_at, (w - equipped_at) as u32);
        b.i32(0); // patched below
        b.i32(42);
        let wvt = b.at();
        b.u16(6);
        b.u16(8);
        b.u16(4);
        b.align(4);
        let back = w as i32 - wvt as i32;
        b.0[w..w + 4].copy_from_slice(&back.to_le_bytes());
        b.0
    }

    /// The node at `path` below the root table: root, then the table it
    /// points at, then the structure that placement holds.
    fn at(e: &mut Evaluator, d: &Document<MemSource>, path: &[usize]) -> crate::eval::NodeInfo {
        let mut p = vec![0, 1, 0];
        p.extend_from_slice(path);
        e.node(d, &p).unwrap_or_else(|err| panic!("{path:?}: {err:?}"))
    }

    // Field indices in the Monster structure: vtable_offset 0, vtable 1, hp 2,
    // name 3, colour 4, pos 5, friends 6, equipped_type 7, equipped 8, mana 9.

    #[test]
    fn a_scalar_is_where_the_vtable_says() {
        let d = Document::new(MemSource(monster()));
        let mut e = Evaluator::new(template());
        let hp = at(&mut e, &d, &[2, 0]);
        assert_eq!(hp.value.as_int(), Some(150));
        assert_eq!(hp.offset_bits, (24 + 4) * 8, "four bytes into the table, as the vtable said");
    }

    #[test]
    fn an_entry_of_zero_and_an_entry_past_the_end_are_both_absent() {
        let d = Document::new(MemSource(monster()));
        let mut e = Evaluator::new(template());
        assert!(at(&mut e, &d, &[4]).absent, "colour's entry is zero");
        assert!(at(&mut e, &d, &[9]).absent, "mana has no entry: the vtable stops before it");
    }

    #[test]
    fn a_string_is_counted_from_its_own_offset() {
        let d = Document::new(MemSource(monster()));
        let mut e = Evaluator::new(template());
        // name -> the offset structure -> string -> text.
        let text = at(&mut e, &d, &[3, 0, 1, 0, 1]);
        assert_eq!(text.value, Value::Str("orc".into()));
    }

    #[test]
    fn a_struct_is_padded_like_c() {
        let d = Document::new(MemSource(monster()));
        let mut e = Evaluator::new(template());
        // pos -> Vec2 { x, padding, y }
        assert_eq!(at(&mut e, &d, &[5, 0, 0]).value.as_int(), Some(-2));
        assert_eq!(at(&mut e, &d, &[5, 0, 1]).size_bits, 3 * 8, "three bytes to line y up on four");
        assert_eq!(at(&mut e, &d, &[5, 0, 2]).value.as_int(), Some(-3));
    }

    #[test]
    fn a_vector_of_tables_holds_an_offset_to_each() {
        let d = Document::new(MemSource(monster()));
        let mut e = Evaluator::new(template());
        // friends -> offset structure -> vector -> elements
        let elements = at(&mut e, &d, &[6, 0, 1, 0, 1]);
        assert_eq!(elements.child_count, 2);
        for (i, want) in [10, 20].into_iter().enumerate() {
            // element -> table -> Monster -> hp -> the number
            let hp = at(&mut e, &d, &[6, 0, 1, 0, 1, i, 1, 0, 2, 0]);
            assert_eq!(hp.value.as_int(), Some(want), "friend {i}");
        }
        // Both friends share one vtable, and both read it.
        let a = at(&mut e, &d, &[6, 0, 1, 0, 1, 0, 1, 0, 1, 0]).offset_bits;
        let b = at(&mut e, &d, &[6, 0, 1, 0, 1, 1, 1, 0, 1, 0]).offset_bits;
        assert_eq!(a, b);
    }

    #[test]
    fn a_union_is_a_type_byte_and_the_table_it_names() {
        let d = Document::new(MemSource(monster()));
        let mut e = Evaluator::new(template());
        let kind = at(&mut e, &d, &[7, 0]).value;
        assert!(matches!(kind, Value::Enum { raw: 1, name: Some(ref n), .. } if n == "Weapon"), "got {kind:?}");
        // equipped -> offset structure -> table, which the type byte made a
        // Weapon; its vtable is after it, so the vtable offset is negative.
        let weapon = at(&mut e, &d, &[8, 0, 1, 0]);
        assert_eq!(weapon.type_name, "Weapon");
        assert!(at(&mut e, &d, &[8, 0, 1, 0, 0]).value.as_int().unwrap() < 0);
        assert_eq!(at(&mut e, &d, &[8, 0, 1, 0, 2, 0]).value.as_int(), Some(42));
    }

    #[test]
    fn the_vtable_names_its_entries() {
        let d = Document::new(MemSource(monster()));
        let mut e = Evaluator::new(template());
        // vtable -> VTable { vtable_size, table_size, hp, name, ... }
        assert_eq!(at(&mut e, &d, &[1, 0, 3]).value.as_int(), Some(8), "name's entry");
        assert!(at(&mut e, &d, &[1, 0, 9]).absent, "mana is past the end of the vtable");
        assert_eq!(at(&mut e, &d, &[1, 0, 10]).child_count, 0, "no ids this schema does not know");
    }

    #[test]
    fn a_default_is_what_an_unwritten_scalar_reads_as() {
        let mut t = Template::new(
            "fb",
            T::structure(
                "Test",
                vec![
                    ("fb", buffer("t", "Monster")),
                    ("hp", T::computed(read_or(&["fb", "root", "table"], &TABLES[0], "hp", 100))),
                    ("mana", T::computed(read_or(&["fb", "root", "table"], &TABLES[0], "mana", 100))),
                ],
            ),
        );
        for (name, ty) in types("t", TABLES, STRUCTS) {
            t = t.with_type(&name, ty);
        }
        let d = Document::new(MemSource(monster()));
        let mut e = Evaluator::new(t);
        assert_eq!(e.node(&d, &[1]).unwrap().value.as_int(), Some(150), "written");
        assert_eq!(e.node(&d, &[2]).unwrap().value.as_int(), Some(100), "not written, so the default");
    }

    #[test]
    fn a_vtable_longer_than_the_schema_counts_what_it_does_not_know() {
        // A Weapon whose vtable has two more entries than the schema has ids.
        let mut b = Buf(Vec::new());
        b.u32(16); // root
        b.u16(10); // vtable at 4: three entries
        b.u16(8);
        b.u16(4); // damage
        b.u16(0);
        b.u16(0);
        b.u16(0); // padding up to the table
        b.i32(12); // table at 16, vtable twelve bytes before it
        b.i32(7); // damage
        let t = Template::new("fb", buffer("t", "Weapon")).with_type("t.Weapon", table_ty("t", &TABLES[1]));
        let d = Document::new(MemSource(b.0));
        let mut e = Evaluator::new(t);
        // root -> table -> Weapon -> vtable -> VTable -> unknown_fields
        assert_eq!(e.node(&d, &[0, 1, 0, 1, 0, 3]).unwrap().child_count, 2);
        assert_eq!(e.node(&d, &[0, 1, 0, 2, 0]).unwrap().value.as_int(), Some(7));
    }
}
