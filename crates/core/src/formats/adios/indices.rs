//! The records BP3 and BP4 index their blocks with: characteristic sets, and
//! the process group, variable and attribute indices that hold them.

use std::sync::Arc;

use super::shared::*;
use crate::template::{Endian, Expr as E, Tag, TaggedRef, Ty as T, Until};

// ---------------------------------------------------------------------------
// Characteristics

/// `BPBase::CharacteristicID`, by ADIOS's names.
pub(super) const CHARACTERISTIC: &[(i128, &str)] = &[
    (VALUE, "value"),
    (MIN, "min"),
    (MAX, "max"),
    (OFFSET, "offset"),
    (DIMENSIONS, "dimensions"),
    (5, "var id"),
    (PAYLOAD_OFFSET, "payload offset"),
    (FILE_INDEX, "subfile index"),
    (TIME_INDEX, "time index"),
    (BITMAP, "statistics bitmap"),
    (STATISTICS, "statistics"),
    (TRANSFORM, "transform type"),
    (MINMAX, "minmax"),
];

pub(super) const VALUE: i128 = 0;
pub(super) const MIN: i128 = 1;
pub(super) const MAX: i128 = 2;
pub(super) const OFFSET: i128 = 3;
pub(super) const DIMENSIONS: i128 = 4;
pub(super) const PAYLOAD_OFFSET: i128 = 6;
pub(super) const FILE_INDEX: i128 = 7;
pub(super) const TIME_INDEX: i128 = 8;
pub(super) const BITMAP: i128 = 9;
pub(super) const STATISTICS: i128 = 10;
pub(super) const TRANSFORM: i128 = 11;
pub(super) const MINMAX: i128 = 12;

/// `field` of the characteristic before this one in its set whose ID is `id`,
/// and nothing when there is none.
pub(super) fn earlier_characteristic(id: i128, field: &[&str]) -> E {
    E::Tagged(Arc::new(TaggedRef {
        array: None,
        key: path(&["id"]),
        tag: Tag::Int(id),
        field: path(field),
    }))
}

/// `field` of the characteristic in the set `characteristics` names whose ID
/// is `id`, and nothing when there is none.
pub(super) fn characteristic_of(id: i128, field: &[&str]) -> E {
    E::tagged("characteristics", &["id"], id, field)
}

pub(super) fn path(names: &[&str]) -> Arc<[String]> {
    names.iter().map(|s| s.to_string()).collect()
}

/// A set of characteristics: how many, how many bytes, and the records.
///
/// A set in an index gains a few fields worked out from its records, since
/// the questions a reader has about a block, which step and how many values,
/// are answered by records in whatever order the writer put them. In a BP3
/// file that holds its own data, the set also reaches the values it describes;
/// see [`indexed_values`].
pub(super) fn characteristic_set(e: Endian, kind: Kind, side: Side, v: Version) -> T {
    let mut fields = vec![
        ("count", T::u8()),
        ("length", T::u32(e)),
        ("characteristics", T::sized(clamp(E::field("length")), T::repeat(characteristic(e, kind, side), Until::End))),
    ];
    if side == Side::Index {
        fields.push(("step", T::computed(characteristic_of(TIME_INDEX, &["body"]))));
    }
    let placed = side == Side::Index && kind == Kind::Variable && v == Bp3;
    if placed {
        fields.extend(indexed_values(e));
    }
    let set = T::structure_named("CharacteristicSet", "step", "characteristics", fields);
    match placed {
        true => set.machinery(&["payload_offset", "element_count", "row_length", "width", "compressed"]).field_aside("values"),
        false => set,
    }
}

/// The values of a block, placed from its set in a BP3 index.
///
/// The process groups before the index hold these bytes already, in the order
/// they were written, and read there without the index. Placed again here they
/// are reached by the variable's name, which is what an index is for, the way
/// a device tree's property reaches its name in the strings block it is
/// already read in, and are a second reading that nothing counts twice. Only
/// when the file holds its data: a BP3 file whose
/// footer says the data is in subfiles has offsets into those.
///
/// A block an operator transformed is not placed, since the offset is to the
/// transformed bytes and not to values of the type.
pub(super) fn indexed_values(e: Endian) -> Vec<(&'static str, T)> {
    let in_file = E::within(&["footer", "flags"]).bit(0).negate();
    let count = || E::field("element_count");
    let row = || E::field("row_length");
    let place = in_file
        .both(E::field("compressed").equal_to(E::lit(0)))
        .both(E::lit(0).less_than(E::field("payload_offset")));
    vec![
        ("payload_offset", T::computed(characteristic_of(PAYLOAD_OFFSET, &["body"]))),
        ("element_count", T::computed(characteristic_of(DIMENSIONS, &["body", "element_count"]))),
        ("row_length", T::computed(characteristic_of(DIMENSIONS, &["body", "row_length"]))),
        ("width", width()),
        ("compressed", T::computed(characteristic_of(TRANSFORM, &["id"]).not_equal(E::lit(0)))),
        (
            "values",
            T::when(
                place,
                T::switch(
                    E::field("data_type"),
                    vec![(STRING, T::at(E::field("payload_offset"), bp_string(e)))],
                    T::when(
                        E::lit(0).less_than(E::field("width")),
                        T::at(E::field("payload_offset"), T::sized(count().mul(E::field("width")), block_values(e, count(), row()))),
                    ),
                ),
            ),
        ),
    ]
}

/// One characteristic: its ID, and a body as wide as the ID and the declared
/// type make it. An ID this does not know has no width anyone has written
/// down, so the rest of the set is its body.
pub(super) fn characteristic(e: Endian, kind: Kind, side: Side) -> T {
    let body = T::switch(
        E::field("id"),
        vec![
            (VALUE, value(e, kind, side)),
            (MIN, scalar(e)),
            (MAX, scalar(e)),
            (OFFSET, T::u64(e)),
            (DIMENSIONS, dimensions(e)),
            (PAYLOAD_OFFSET, T::u64(e)),
            (FILE_INDEX, T::u32(e)),
            (TIME_INDEX, T::u32(e)),
            (BITMAP, T::flags("StatisticsBitmap", T::u32(e), STATISTIC_BITS)),
            (STATISTICS, statistics(e)),
            (TRANSFORM, transform(e)),
            (MINMAX, minmax(e, side)),
        ],
        T::bytes(E::Remaining),
    );
    T::structure_named(
        "Characteristic",
        "id",
        "body",
        vec![("id", T::enumeration("CharacteristicId", T::u8(), CHARACTERISTIC)), ("body", body)],
    )
}

/// A value record. A string is a BP string; a string array attribute is as
/// many of them as its dimensions record says. A number is one value, or for
/// an attribute as many as its dimensions record says.
///
/// In a BP3 data file a single value is written with a two-byte length in
/// front, which ADIOS2's writer calls a special case for `bpdump`.
pub(super) fn value(e: Endian, kind: Kind, side: Side) -> T {
    if side == Side::Data {
        return T::structure("DataValue", vec![("length", T::u16(e)), ("value", T::sized(clamp(E::field("length")), scalar(e)))]);
    }
    let one = || {
        let mut cases = scalars(e);
        cases.push((STRING, bp_string(e)));
        T::switch(E::field("data_type"), cases, T::bytes(E::Remaining))
    };
    match kind {
        Kind::Variable => one(),
        Kind::Attribute => {
            let count = || earlier_characteristic(DIMENSIONS, &["body", "first_count"]).at_least(E::lit(1));
            let mut cases: Vec<(i128, T)> =
                scalars(e).into_iter().map(|(k, t)| (k, T::switch(E::lit(1).less_than(count()), vec![(1, T::array(t.clone(), count()))], t))).collect();
            cases.push((STRING, bp_string(e)));
            cases.push((STRING_ARRAY, T::array(bp_string(e), count())));
            T::switch(E::field("data_type"), cases, T::bytes(E::Remaining))
        }
    }
}

/// A dimensions record: how many, their length in bytes, and for each the
/// block's count, the variable's shape and where the block starts in it.
///
/// A shape of all ones but two is a single value per writer, and all ones but
/// one is an array joined along that dimension; any other shape is a length,
/// so the two are not an enumeration that would flag every length as a value
/// nobody named. After the fields the file
/// writes come three it does not: how many values the block holds, the first
/// count, which is how long an attribute's value is, and the last, which is
/// how long a row is.
pub(super) fn dimensions(e: Endian) -> T {
    let dim = T::structure(
        "Dimension",
        vec![
            ("count", T::u64(e)),
            ("shape", T::u64(e)),
            ("start", T::u64(e)),
        ],
    );
    let nth = |i: E| E::elem_field("dimensions", i, &["count"]);
    let some = || E::lit(0).less_than(E::field("count"));
    T::structure(
        "Dimensions",
        vec![
            ("count", T::u8()),
            ("length", T::u16(e)),
            ("dimensions", T::array(dim, E::field("count"))),
            ("counts", T::array(T::computed(nth(E::idx())), E::field("count"))),
            ("element_count", T::computed(E::product_of("counts"))),
            ("first_count", T::computed(E::cond(some(), nth(E::lit(0)), E::lit(0)))),
            ("row_length", T::computed(E::cond(some(), nth(E::field("count").sub(E::lit(1))), E::lit(1)))),
        ],
    )
    .machinery(&["counts", "element_count", "first_count", "row_length"])
}

/// The minimum and maximum of a block, and when the writer divided the block
/// into sub-blocks, how, and the minimum and maximum of each. The division is
/// one number per dimension, and a set in an index says how many dimensions
/// in a record of its own, while a data file's header says it beside the set.
pub(super) fn minmax(e: Endian, side: Side) -> T {
    let ndims = match side {
        Side::Index => earlier_characteristic(DIMENSIONS, &["body", "count"]),
        Side::Data => E::field("dims_count"),
    };
    let blocks = || E::field("sub_blocks");
    T::structure(
        "MinMax",
        vec![
            ("sub_blocks", T::u16(e)),
            ("min", scalar(e)),
            ("max", scalar(e)),
            (
                "division",
                T::when(
                    E::lit(1).less_than(blocks()),
                    T::structure(
                        "BlockDivision",
                        vec![
                            ("method", T::enumeration("BlockDivisionMethod", T::u8(), &[(0, "contiguous"), (1, "mesh")])),
                            ("sub_block_size", T::u64(e)),
                            ("divisions", T::array(T::u16(e), ndims)),
                            ("min_max", T::array(scalar(e), E::lit(2).mul(blocks()))),
                        ],
                    ),
                ),
            ),
        ],
    )
}

/// `BPBase::VariableStatistics`, the bits of an ADIOS 1 statistics bitmap.
pub(super) const STATISTIC_BITS: &[(u32, &str)] =
    &[(0, "min"), (1, "max"), (2, "count"), (3, "sum"), (4, "sum of squares"), (5, "histogram"), (6, "finite")];

/// ADIOS 1's statistics record: one value for each bit the bitmap record
/// before it sets, in the order of the bits. A histogram has no width ADIOS2
/// reads, and ends what can be read of the set.
pub(super) fn statistics(e: Endian) -> T {
    let bit = |n: u32| earlier_characteristic(BITMAP, &["body"]).bit(n);
    T::structure(
        "Statistics",
        vec![
            ("min", T::when(bit(0), scalar(e))),
            ("max", T::when(bit(1), scalar(e))),
            ("count", T::when(bit(2), T::u32(e))),
            ("sum", T::when(bit(3), T::F64(e))),
            ("sum_of_squares", T::when(bit(4), T::F64(e))),
            ("histogram", T::when(bit(5), T::bytes(E::Remaining))),
            ("finite", T::when(bit(6), T::u8())),
        ],
    )
}

/// An operator applied to the block: its name, the type and dimensions the
/// block had before, and what the operator kept about it. ADIOS2 keeps sixteen
/// bytes, the size in and the size out.
pub(super) fn transform(e: Endian) -> T {
    let dim = T::structure("Dimension", vec![("count", T::u64(e)), ("shape", T::u64(e)), ("start", T::u64(e))]);
    T::structure(
        "Transform",
        vec![
            ("name_length", T::u8()),
            ("name", T::utf8(E::field("name_length"))),
            ("data_type", data_type()),
            ("dims_count", T::u8()),
            ("dims_length", T::u16(e)),
            ("dimensions", T::sized(clamp(E::field("dims_length")), T::array(dim, E::field("dims_count")))),
            ("metadata_length", T::u16(e)),
            (
                "metadata",
                T::sized(
                    clamp(E::field("metadata_length")),
                    T::switch(
                        E::field("metadata_length"),
                        vec![(16, T::structure("TransformSizes", vec![("input_size", T::u64(e)), ("output_size", T::u64(e))]))],
                        T::bytes(E::Remaining),
                    ),
                ),
            ),
        ],
    )
}

// ---------------------------------------------------------------------------
// Indices

/// A process group index: how many entries, their length, and the entries,
/// each a group's IO name, array order, rank, step and where the group starts
/// in the data. `entries` is how many bytes the entries take, which a BP3
/// footer gives by where the next index starts rather than trusting the
/// length: ADIOS2's reader says ADIOS 1 wrote that length too small.
pub(super) fn pg_index(e: Endian, entries: E) -> T {
    let entry = T::structure_named(
        "PgIndexEntry",
        "step_name",
        "",
        vec![
            ("length", T::u16(e)),
            ("io_name", bp_string(e)),
            ("array_order", T::enumeration("BpArrayOrder", T::u8(), ARRAY_ORDER)),
            ("rank", T::u32(e)),
            ("step_name", bp_string(e)),
            ("step", T::u32(e)),
            ("data_offset", T::u64(e)),
            ("rest", T::when(E::lit(0).less_than(E::Remaining), T::bytes(E::Remaining))),
        ],
    );
    T::structure(
        "PgIndex",
        vec![
            ("count", T::u64(e)),
            ("length", T::u64(e)),
            ("entries", T::sized(clamp(entries), T::repeat(T::sized(clamp(E::peek(16, e).add(E::lit(2))), entry), Until::End))),
        ],
    )
}

/// A variable or attribute index: how many entries, their length, and the
/// entries. See [`pg_index`] for `entries`.
pub(super) fn element_index(e: Endian, v: Version, kind: Kind, entries: E) -> T {
    let name = match kind {
        Kind::Variable => "VariablesIndex",
        Kind::Attribute => "AttributesIndex",
    };
    let entry = T::sized(clamp(E::peek(32, e).add(E::lit(4))), index_entry(e, v, kind));
    T::structure(
        name,
        vec![
            ("count", T::u32(e)),
            ("length", T::u64(e)),
            ("entries", T::sized(clamp(entries), T::repeat(entry, Until::End))),
        ],
    )
}

/// One variable's or attribute's entry: its member ID, group, name, type, and
/// one characteristic set per block.
pub(super) fn index_entry(e: Endian, v: Version, kind: Kind) -> T {
    let mut fields = vec![
        ("length", T::u32(e)),
        ("member_id", T::u32(e)),
        ("group", bp_string(e)),
        ("name", bp_string(e)),
    ];
    match v {
        Bp3 => fields.push(("path", bp_string(e))),
        Bp4 => fields.extend([("array_order", T::enumeration("BpLayout", T::u8(), LAYOUT)), ("unused", T::u8())]),
    }
    fields.extend([
        ("data_type", data_type()),
        ("set_count", T::u64(e)),
        // Five bytes is the least a set can be, which is what keeps a count
        // that is not one from asking for sets the bytes cannot hold.
        ("sets", T::array(characteristic_set(e, kind, Side::Index, v), E::field("set_count").at_most(E::Remaining.div(E::lit(5))))),
        ("rest", T::when(E::lit(0).less_than(E::Remaining), T::bytes(E::Remaining))),
    ]);
    let name = match kind {
        Kind::Variable => "VariableEntry",
        Kind::Attribute => "AttributeEntry",
    };
    T::structure_named(name, "name", "sets", fields)
}

/// How a BP4 variable says its arrays are laid out, which NumPy's letters. An
/// attribute entry writes nothing there.
pub(super) const LAYOUT: &[(i128, &str)] = &[(b'K' as i128, "writer's default"), (b'C' as i128, "row-major"), (b'F' as i128, "column-major"), (0, "not given")];
