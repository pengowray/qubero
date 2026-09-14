//! Which schema field each node of a record batch stands for, and which node
//! and purpose each buffer has: the walk that names a batch's buffers, since
//! nothing in the batch says.
//!
//! Apart from [`arrow`](super::arrow) because it is the one part of the reader
//! that works something out rather than reading it: the column layouts, the
//! depth-first walk over the nodes, and the buffers' roles by layout.
//! `arrow.rs` splices these fields into the schema's `Field`, `FieldNode` and
//! `Buffer` tables, and reads each buffer by the `reading` the walk picks.

use super::arrow::{
    BYTES, BITS, I8, U8, I16, U16, I32, U32, I64, U64, F16, F32, F64, DAYS, DATE_MILLIS, INSTANT, WALL_CLOCK, YEAR_MONTH, DAY_TIME,
    MONTH_DAY_NANO, FIXED_WIDTH, TEXT, BINARY, TEXT_VIEWS, BINARY_VIEWS, VALIDITY_BITMAP, ENDS32, ENDS64,
};
use super::arrow_schema::{table, FIELD, RECORD_BATCH, SCHEMA};
use crate::formats::flatbuf;
use crate::template::{Expr as E, Ty as T};

// How a column's buffers are laid out, which is what the schema decides and
// the record batch never repeats. The numbers are this reader's own, and the
// names are the "Layout Type" column of the table in Columnar.rst headed
// "Buffer Listing for Each Layout", with a large variant, 64-bit offsets
// rather than 32, carried separately.
const NULL: i128 = 0;
const PRIMITIVE: i128 = 1;
const VARIABLE_BINARY: i128 = 2;
const VARIABLE_BINARY_VIEW: i128 = 3;
const LIST: i128 = 4;
const LIST_VIEW: i128 = 5;
const FIXED_SIZE_LIST: i128 = 6;
const STRUCT: i128 = 7;
const SPARSE_UNION: i128 = 8;
const DENSE_UNION: i128 = 9;
const DICTIONARY_ENCODED: i128 = 10;
const RUN_END_ENCODED: i128 = 11;
const UNPARSED: i128 = 12;

const LAYOUT: &[(i128, &str)] = &[
    (NULL, "Null"),
    (PRIMITIVE, "Primitive"),
    (VARIABLE_BINARY, "Variable Binary"),
    (VARIABLE_BINARY_VIEW, "Variable Binary View"),
    (LIST, "List"),
    (LIST_VIEW, "List View"),
    (FIXED_SIZE_LIST, "Fixed-size List"),
    (STRUCT, "Struct"),
    (SPARSE_UNION, "Sparse Union"),
    (DENSE_UNION, "Dense Union"),
    (DICTIONARY_ENCODED, "Dictionary-encoded"),
    (RUN_END_ENCODED, "Run-end encoded"),
    (UNPARSED, "unparsed"),
];

// What one buffer is for, in the words of the same table.
const VALIDITY: i128 = 1;
const DATA: i128 = 2;
const OFFSETS: i128 = 3;
const VIEWS: i128 = 4;
const SIZES: i128 = 5;
const TYPE_IDS: i128 = 6;
const INDICES: i128 = 7;

pub(super) const ROLE: &[(i128, &str)] = &[
    (0, "unparsed"),
    (VALIDITY, "validity"),
    (DATA, "data"),
    (OFFSETS, "offsets"),
    (VIEWS, "views"),
    (SIZES, "sizes"),
    (TYPE_IDS, "type ids"),
    (INDICES, "indices"),
];

/// Which buffer is for what, by layout and then by position among the
/// column's own buffers. A binary view's data buffers are as many as the
/// batch says, so every position from the third on is data.
const ROLES: &[(i128, &[i128])] = &[
    (PRIMITIVE, &[VALIDITY, DATA]),
    (VARIABLE_BINARY, &[VALIDITY, OFFSETS, DATA]),
    (VARIABLE_BINARY_VIEW, &[VALIDITY, VIEWS, DATA, DATA, DATA, DATA, DATA, DATA]),
    (LIST, &[VALIDITY, OFFSETS]),
    (LIST_VIEW, &[VALIDITY, OFFSETS, SIZES]),
    (FIXED_SIZE_LIST, &[VALIDITY]),
    (STRUCT, &[VALIDITY]),
    (SPARSE_UNION, &[TYPE_IDS]),
    (DENSE_UNION, &[TYPE_IDS, OFFSETS]),
    (DICTIONARY_ENCODED, &[VALIDITY, INDICES]),
];

/// A chain of conditions over one number, for a choice of two or three.
/// `on` is repeated in every test, so it should be a field rather than a sum.
fn pick(on: &E, cases: &[(i128, E)], default: E) -> E {
    cases.iter().rev().fold(default, |rest, (k, v)| E::cond(on.clone().equal_to(E::lit(*k)), v.clone(), rest))
}

fn pick_lit(on: &E, cases: &[(i128, i128)], default: i128) -> E {
    let cases: Vec<(i128, E)> = cases.iter().map(|(k, v)| (*k, E::lit(*v))).collect();
    pick(on, &cases, E::lit(default))
}

/// A lookup table as a field: the number `on` holds picks the case.
///
/// A switch over computed fields rather than [`pick`], because a switch is
/// settled in one step and a chain of twenty-six conditions is twenty-six
/// calls deep. A buffer asks its node, which asks the schema, which asks its
/// type table, all inside one read, and a chain that long at the bottom of
/// that was most of the stack a read is allowed.
fn lookup(on: E, cases: Vec<(i128, E)>, default: E) -> T {
    T::switch(on, cases.into_iter().map(|(k, v)| (k, T::computed(v))).collect(), T::computed(default))
}

/// The same, for a table whose answers are layouts, so that the row reads as
/// the layout's name.
fn lookup_layout(on: E, cases: Vec<(i128, E)>, default: E) -> T {
    let layout = |e: E| T::enumeration("Layout", T::computed(e), LAYOUT);
    T::switch(on, cases.into_iter().map(|(k, v)| (k, layout(v))).collect(), layout(default))
}

/// A signed or unsigned integer of `bits`, as a reading.
fn integer(bits: &E, signed: E) -> E {
    E::cond(
        signed,
        pick_lit(bits, &[(8, I8), (16, I16), (32, I32), (64, I64)], BYTES),
        pick_lit(bits, &[(8, U8), (16, U16), (32, U32), (64, U64)], BYTES),
    )
}

/// What the schema says about one column, worked out once where the column is
/// declared, so that a record batch can ask for it by position.
///
/// Nothing here is in the file. It is the reading of the column's `type` and
/// `dictionary` that the buffers depend on: which layout, whether its offsets
/// are 64-bit, what one value is, how wide a fixed-width one is, which
/// dictionary it is encoded against and with what width of index. Declared at
/// the end of every `Field` table, children's included, and read from there by
/// the nodes of every batch.
///
/// `index` is where the field sits among its siblings. A child is found from a
/// batch by a search for it rather than by position, because what reaches it
/// is a list inside an element of another list, and a path in an expression
/// names one position and not two.
pub(super) fn field_layout() -> T {
    let type_id = E::field("type_id");
    let of = |table_name: &str, field: &str, default: i128| {
        flatbuf::read_or(&["type", "table"], table(table_name), field, default)
    };
    let kind = lookup_layout(
        type_id.clone(),
        vec![
            (1, E::lit(NULL)),
            (2, E::lit(PRIMITIVE)),
            (3, E::lit(PRIMITIVE)),
            (4, E::lit(VARIABLE_BINARY)),
            (5, E::lit(VARIABLE_BINARY)),
            (6, E::lit(PRIMITIVE)),
            (7, E::lit(PRIMITIVE)),
            (8, E::lit(PRIMITIVE)),
            (9, E::lit(PRIMITIVE)),
            (10, E::lit(PRIMITIVE)),
            (11, E::lit(PRIMITIVE)),
            (12, E::lit(LIST)),
            (13, E::lit(STRUCT)),
            (14, E::cond(of("Union", "mode", 0).equal_to(E::lit(1)), E::lit(DENSE_UNION), E::lit(SPARSE_UNION))),
            (15, E::lit(PRIMITIVE)),
            (16, E::lit(FIXED_SIZE_LIST)),
            (17, E::lit(LIST)),
            (18, E::lit(PRIMITIVE)),
            (19, E::lit(VARIABLE_BINARY)),
            (20, E::lit(VARIABLE_BINARY)),
            (21, E::lit(LIST)),
            (22, E::lit(RUN_END_ENCODED)),
            (23, E::lit(VARIABLE_BINARY_VIEW)),
            (24, E::lit(VARIABLE_BINARY_VIEW)),
            (25, E::lit(LIST_VIEW)),
            (26, E::lit(LIST_VIEW)),
        ],
        E::lit(UNPARSED),
    );
    let time_unit = |table_name: &str| of(table_name, "unit", 0);
    let values = lookup(
        type_id.clone(),
        vec![
            (2, integer(&of("Int", "bitWidth", 0), of("Int", "is_signed", 0))),
            (3, pick_lit(&of("FloatingPoint", "precision", 0), &[(0, F16), (1, F32), (2, F64)], BYTES)),
            (4, E::lit(BINARY)),
            (5, E::lit(TEXT)),
            (6, E::lit(BITS)),
            (7, E::lit(FIXED_WIDTH)),
            (8, pick_lit(&of("Date", "unit", 1), &[(0, DAYS), (1, DATE_MILLIS)], BYTES)),
            (9, pick_lit(&of("Time", "bitWidth", 32), &[(32, I32), (64, I64)], BYTES)),
            (
                10,
                E::cond(
                    flatbuf::is_written(&["type", "table"], table("Timestamp"), "timezone"),
                    E::lit(INSTANT).add(time_unit("Timestamp")),
                    E::lit(WALL_CLOCK).add(time_unit("Timestamp")),
                ),
            ),
            (11, pick_lit(&of("Interval", "unit", 0), &[(0, YEAR_MONTH), (1, DAY_TIME), (2, MONTH_DAY_NANO)], BYTES)),
            (15, E::lit(FIXED_WIDTH)),
            (18, E::lit(I64)),
            (19, E::lit(BINARY)),
            (20, E::lit(TEXT)),
            (23, E::lit(BINARY)),
            (24, E::lit(TEXT)),
        ],
        E::lit(BYTES),
    );
    let width = lookup(type_id.clone(), vec![(7, of("Decimal", "bitWidth", 128).div(E::lit(8))), (15, of("FixedSizeBinary", "byteWidth", 0))], E::lit(0));
    let encoding = ["dictionary", "table"];
    let dictionary = flatbuf::is_written(&[], &FIELD, "dictionary");
    let index_type = ["dictionary", "table", "indexType", "table"];
    let index = E::cond(
        flatbuf::is_written(&encoding, table("DictionaryEncoding"), "indexType"),
        integer(&flatbuf::read_or(&index_type, table("Int"), "bitWidth", 0), flatbuf::read_or(&index_type, table("Int"), "is_signed", 0)),
        // No index type is a signed 32-bit index, which Schema.fbs says in
        // as many words.
        E::lit(I32),
    );
    // Everything a batch's node needs, in one number, so that a node reads the
    // schema once rather than six times: the layout in the low four bits, then
    // whether offsets are 64-bit, how a value reads, how an index reads,
    // whether the column is dictionary encoded, a fixed width, and how many
    // children. See `unpack`.
    let summary = E::field("kind")
        .add(E::field("large").shl(E::lit(4)))
        .add(E::field("values").shl(E::lit(5)))
        .add(E::field("index_values").shl(E::lit(11)))
        .add(E::field("dictionary_id").greater_or_equal(E::lit(0)).shl(E::lit(17)))
        .add(E::field("width").and(E::lit(0xFFFF_FFFF_u32)).shl(E::lit(18)))
        .add(E::field("child_count").and(E::lit(0xFFFF_FFFF_u32)).mul(E::lit(1i128 << 50)));
    T::structure(
        "ColumnLayout",
        vec![
            ("type_id", T::computed(flatbuf::read_or(&[], &FIELD, "type_type", 0))),
            ("index", T::computed(E::Idx)),
            ("child_count", T::computed(E::cond(flatbuf::is_written(&[], &FIELD, "children"), E::within(&["children", "vector", "count"]), E::lit(0)))),
            ("kind", kind),
            ("large", lookup(type_id, vec![(19, E::lit(1)), (20, E::lit(1)), (21, E::lit(1)), (26, E::lit(1))], E::lit(0))),
            ("values", values),
            ("width", width),
            ("dictionary_id", T::computed(E::cond(dictionary.clone(), flatbuf::read_or(&encoding, table("DictionaryEncoding"), "id", 0), E::lit(-1)))),
            ("index_values", T::computed(E::cond(dictionary, index, E::lit(BYTES)))),
            ("summary", T::computed(summary)),
        ],
    )
    .machinery(&["type_id", "index", "child_count", "large", "values", "width", "dictionary_id", "index_values", "summary"])
}

/// The top-level fields of the schema, from anywhere in a batch: the schema
/// message at the front of the stream, which a file repeats in its footer and
/// a stream has nowhere else.
pub(super) const SCHEMA_TABLE: &[&str] = &["schema", "metadata", "root", "table", "header", "table"];
pub(super) const SCHEMA_FIELDS: &[&str] = &["schema", "metadata", "root", "table", "header", "table", "fields", "vector", "elements"];

/// A field of no bytes whose type is chosen when it is read, declared so that
/// a list of the records it sits in stays a list of fixed-size records.
///
/// A switch could be any size until it is read, so a record holding a bare one
/// has no size of its own, and a list of such records is placed by walking
/// every element before the one asked for instead of by multiplying. The walk
/// forgets what it walks past, and a node forgotten has to work its whole
/// chain of nodes out again from the first. Sized to nothing, the switch
/// costs the record no bytes and the record keeps its sixteen.
fn fixed(ty: T) -> T {
    T::sized(E::lit(0), ty)
}

/// Bits `shift` up of the packed `summary`, `width` of them. See
/// [`field_layout`].
fn unpack(shift: i128, width: u32) -> E {
    E::field("summary").shr(E::lit(shift)).and(E::lit((1i128 << width) - 1))
}

/// The layout value `name` of the schema field a node stands for, wherever in
/// the tree of fields that is: a top-level field by position, a child by a
/// search of its parent's children, a grandchild by a search of the child's.
fn of_field(name: &str) -> E {
    let layout = ["table", "layout", name];
    let children = ["table", "children", "vector", "elements"];
    let key = ["table", "layout", "index"];
    let top = E::elem_within(SCHEMA_FIELDS, E::field("top"), &layout);
    let child_list = E::elem_within(SCHEMA_FIELDS, E::field("top"), &children);
    let child = E::tagged_in_by(child_list.clone(), &key, E::field("child"), &layout);
    let grandchild_list = E::tagged_in_by(child_list, &key, E::field("child"), &children);
    let grandchild = E::tagged_in_by(grandchild_list, &key, E::field("grandchild"), &layout);
    let depth = E::field("depth");
    pick(&depth, &[(1, top), (2, child), (3, grandchild)], E::lit(0))
}

/// Which schema field a node of a batch stands for, and how its buffers are
/// laid out, as fields of the node that nothing in the file holds.
///
/// The nodes of a batch are every field of the schema, children and all, in
/// the order a depth-first walk of the schema meets them, and nothing in a
/// node says which field it is. So the walk is made again here, one node at a
/// time, from what the node before worked out: the position among the
/// top-level fields, among that field's children, among the child's children,
/// how deep that is, and how many children each of those has. A field with
/// children is followed by its first child; a field without is followed by its
/// next sibling, or by its parent's next sibling when it was the last.
///
/// Three levels deep, which covers a list of structs, a map, and a list of
/// lists. A fourth level ends the walk: that node and every node after it is
/// `unparsed`, and so is every buffer of theirs, and they still read as bytes
/// where their `Buffer` puts them. A walk that guessed past that point would
/// put the wrong column's name on every buffer after it.
///
/// A dictionary batch holds one column, the dictionary of whichever field
/// names it by id, so its walk starts at that field and ends with that
/// field's own children.
///
/// What a node's buffers are is then the layout of that field, and the first
/// of them is where the node before's ended: `first_buffer` is a running
/// count, and `starts_at` is that count only for a node with buffers of its
/// own, so that a search for the node whose buffers begin at a given buffer
/// finds exactly one.
///
/// Every step reads the node before with `prev`, which is why these are
/// fields of the node and not of a structure inside it, and every step reads
/// the schema once, for the packed `summary`. A buffer asks its node, which
/// asks the node before and the schema, all inside one read, and the reader
/// allows a read only so much stack.
pub(super) fn node_walk() -> Vec<(&'static str, T)> {
    let first = E::Idx.equal_to(E::lit(0));
    let depth = E::prev("depth");
    let not = |e: E| E::lit(1).sub(e);
    let descend = depth.clone().equal_to(E::lit(1)).either(depth.clone().equal_to(E::lit(2))).mul(E::prev("own").greater_than(E::lit(0)));
    let leaf = E::prev("own").equal_to(E::lit(0));
    let next_grandchild = depth
        .clone()
        .equal_to(E::lit(3))
        .mul(leaf.clone())
        .mul(E::prev("grandchild").add(E::lit(1)).less_than(E::prev("grandchildren")));
    let next_child = depth
        .clone()
        .greater_or_equal(E::lit(2))
        .mul(leaf.clone())
        .mul(not(E::field("next_grandchild")))
        .mul(E::prev("child").add(E::lit(1)).less_than(E::prev("children")));
    let next_top = depth
        .clone()
        .not_equal(E::lit(0))
        .mul(leaf)
        .mul(not(E::field("next_grandchild")))
        .mul(not(E::field("next_child")));
    let fields = {
        let mut count: Vec<&str> = SCHEMA_TABLE.to_vec();
        count.extend(["fields", "vector", "count"]);
        E::cond(flatbuf::is_written(SCHEMA_TABLE, &SCHEMA, "fields"), E::within(&count), E::lit(0))
    };
    let dictionary = E::field("in_dictionary");
    let start_top = E::cond(dictionary.clone(), E::field("schema_field"), E::lit(0));
    let start_depth = E::cond(
        dictionary.clone(),
        E::field("schema_field").greater_or_equal(E::lit(0)),
        fields.clone().greater_than(E::lit(0)),
    );
    let back_at_top = E::field("next_top").mul(not(dictionary.clone())).mul(E::field("top").less_than(fields));
    let new_depth = E::cond(
        first.clone(),
        start_depth,
        E::cond(
            E::field("descend"),
            depth.clone().add(E::lit(1)),
            E::cond(E::field("next_grandchild"), E::lit(3), E::cond(E::field("next_child"), E::lit(2), back_at_top)),
        ),
    );
    let kind = E::field("layout");
    let buffers = {
        let variadic = E::elem_within(&["variadicBufferCounts", "vector", "elements"], E::field("views").sub(E::lit(1)), &[]);
        let variadic = E::cond(flatbuf::is_written(&[], &RECORD_BATCH, "variadicBufferCounts"), variadic, E::lit(0));
        let counts: Vec<(i128, E)> = ROLES
            .iter()
            .map(|(k, roles)| (*k, if *k == VARIABLE_BINARY_VIEW { E::lit(2).add(variadic.clone()) } else { E::lit(roles.len() as i128) }))
            .collect();
        lookup(kind.clone(), counts, E::lit(0))
    };
    let descended_from = |level: i128| E::field("descend").mul(depth.clone().equal_to(E::lit(level)));
    let dictionary_encoded = unpack(17, 1).mul(not(dictionary.mul(E::field("depth").equal_to(E::lit(1)))));
    vec![
        ("in_dictionary", T::computed(E::within(&["header_type"]).equal_to(E::lit(2)))),
        ("descend", T::computed(descend)),
        ("next_grandchild", T::computed(next_grandchild)),
        ("next_child", T::computed(next_child)),
        ("next_top", T::computed(next_top)),
        ("top", T::computed(E::cond(first, start_top, E::prev("top").add(E::field("next_top"))))),
        ("child", T::computed(E::cond(descended_from(1), E::lit(0), E::prev("child").add(E::field("next_child"))))),
        ("grandchild", T::computed(E::cond(descended_from(2), E::lit(0), E::prev("grandchild").add(E::field("next_grandchild"))))),
        ("children", T::computed(E::cond(descended_from(1), E::prev("own"), E::prev("children")))),
        ("grandchildren", T::computed(E::cond(descended_from(2), E::prev("own"), E::prev("grandchildren")))),
        ("depth", T::computed(new_depth)),
        ("summary", T::computed(E::cond(E::field("depth").greater_than(E::lit(0)), of_field("summary"), E::lit(0)))),
        ("own", T::computed(unpack(50, 32))),
        (
            "layout",
            T::enumeration(
                "Layout",
                T::computed(E::cond(
                    E::field("depth").equal_to(E::lit(0)),
                    E::lit(UNPARSED),
                    E::cond(dictionary_encoded, E::lit(DICTIONARY_ENCODED), unpack(0, 4)),
                )),
                LAYOUT,
            ),
        ),
        ("large", T::computed(unpack(4, 1))),
        ("values", T::computed(E::cond(kind.clone().equal_to(E::lit(DICTIONARY_ENCODED)), unpack(11, 6), unpack(5, 6)))),
        ("width", T::computed(unpack(18, 32))),
        ("views", T::computed(E::prev("views").add(kind.equal_to(E::lit(VARIABLE_BINARY_VIEW))))),
        ("buffers", fixed(buffers)),
        ("first_buffer", T::computed(E::prev("first_buffer").add(E::prev("buffers")))),
        ("starts_at", T::computed(E::cond(E::field("buffers").greater_than(E::lit(0)), E::field("first_buffer"), E::lit(-1)))),
        ("number", T::computed(E::Idx.add(E::lit(1)))),
        ("column", fixed(node_column())),
    ]
}

/// The name of the field a node stands for, which is the name a reader looks
/// for: `tags`, and then `item` for the list's values.
fn node_column() -> T {
    let name = ["table", "name", "string", "text"];
    let children = ["table", "children", "vector", "elements"];
    let key = ["table", "layout", "index"];
    let child_list = E::elem_within(SCHEMA_FIELDS, E::field("top"), &children);
    T::switch(
        E::field("depth"),
        vec![
            (1, T::computed_text(E::elem_within(SCHEMA_FIELDS, E::field("top"), &name))),
            (2, T::computed_text(E::tagged_in_by(child_list.clone(), &key, E::field("child"), &name))),
            (3, T::computed_text(E::tagged_in_by(E::tagged_in_by(child_list, &key, E::field("child"), &children), &key, E::field("grandchild"), &name))),
        ],
        T::when(E::lit(0), T::computed_text(E::lit(0))),
    )
}

/// Which node a buffer belongs to and what it is for, as fields of the buffer.
///
/// A buffer whose position is where some node's buffers start belongs to that
/// node, and is its first. Any other belongs to the node the buffer before it
/// belonged to, one position further on, as long as that node has that many.
/// A buffer that fits neither is past where the walk of the nodes stopped.
pub(super) fn buffer_walk() -> Vec<(&'static str, T)> {
    let nodes = ["nodes", "vector", "elements"];
    let node = |f: &str| E::elem_within(&nodes, E::field("node"), &[f]);
    let found = E::field("found");
    let role = {
        let slot = E::field("slot").at_most(E::lit(7));
        let code = E::cond(E::field("known"), E::field("kind").mul(E::lit(8)).add(slot), E::lit(-1));
        let table: Vec<(i128, E)> = ROLES
            .iter()
            .flat_map(|(k, roles)| roles.iter().enumerate().map(move |(s, r)| (k * 8 + s as i128, E::lit(*r))))
            .collect();
        let role = |e: E| T::enumeration("BufferRole", T::computed(e), ROLE);
        T::switch(code, table.into_iter().map(|(k, v)| (k, role(v))).collect(), role(E::lit(0)))
    };
    // An offset of a dense union says where in its child one value is, so
    // there is one per value; the offsets of a list or a string run one past
    // the last value, to say where it ends.
    let offsets = E::cond(
        E::field("kind").equal_to(E::lit(DENSE_UNION)),
        E::lit(I32),
        E::cond(node("large"), E::lit(ENDS64), E::lit(ENDS32)),
    );
    let sizes = E::cond(node("large"), E::lit(I64), E::lit(I32));
    let reading = lookup(
        E::field("role"),
        vec![
            (VALIDITY, E::lit(VALIDITY_BITMAP)),
            (DATA, node("values")),
            (OFFSETS, offsets),
            (SIZES, sizes),
            (VIEWS, E::cond(node("values").equal_to(E::lit(TEXT)), E::lit(TEXT_VIEWS), E::lit(BINARY_VIEWS))),
            (TYPE_IDS, E::lit(I8)),
            (INDICES, node("values")),
        ],
        E::lit(BYTES),
    );
    vec![
        ("found", T::computed(E::tagged_in_by(E::within(&nodes), &["starts_at"], E::Idx, &["number"]))),
        ("node", T::computed(E::cond(found.clone().not_equal(E::lit(0)), found.clone().sub(E::lit(1)), E::prev("node")))),
        ("slot", T::computed(E::cond(found.clone().not_equal(E::lit(0)), E::lit(0), E::prev("slot").add(E::lit(1))))),
        ("kind", T::enumeration("Layout", T::computed(node("layout")), LAYOUT)),
        (
            "known",
            T::computed(E::cond(found.not_equal(E::lit(0)), E::lit(1), E::prev("known").mul(E::field("slot").less_than(node("buffers"))))),
        ),
        ("column", fixed(T::when(E::field("known"), T::computed_text(node("column"))))),
        ("role", fixed(role)),
        ("reading", fixed(reading)),
        ("width", T::computed(node("width"))),
        ("count", T::computed(node("length"))),
    ]
}
