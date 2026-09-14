//! ADIOS2 BP files: what the large simulations of fusion, climate and
//! cosmology write their output as. Read from the ADIOS2 sources' serializers
//! (`toolkit/format/bp`, `bp5` and the BP4 and BP5 writers, release 2.12.1),
//! and checked against files that release wrote.
//!
//! Three versions, and only the oldest is one file. **BP3**, the format of
//! ADIOS 1 carried into ADIOS2, is a run of process groups and then a footer
//! that indexes them. **BP4** splits the same records into a directory:
//! `data.0` to `data.N` hold the process groups, `md.0` holds every step's
//! index, and `md.idx` says where in `md.0` each step starts. **BP5** keeps the
//! directory and replaces the indices with FFS, a self-describing binary
//! serialization of its own: `md.idx` again, `md.0` holding each step's FFS
//! records, `mmd.0` holding the FFS formats those records are written in, and
//! `data.N` holding nothing but the bytes the records point at.
//!
//! Qubero opens one file, so every file of a directory is a template of its
//! own. A record in one file that points into another, a step in `md.0` naming
//! a block in `data.0`, is read as the number it is and not followed.
//!
//! **Process groups.** What a writer's rank puts down in one step: the IO's
//! name, whether its arrays are column-major, the step, the transports, and
//! then every variable block it wrote, each a header with the variable's name,
//! type and dimensions and then its values, and last the attributes, each
//! written once in the first group that had it. BP4 brackets the group, each
//! variable and each attribute with four letters, `[PGI` and `PGI]` and the
//! like, which BP3 does not have. The header carries the type and the
//! dimensions, so a data file is read to its values without its index.
//!
//! **Indices.** Three per step: one entry per process group saying where it
//! starts, one per variable and one per attribute. A variable's entry is its
//! name and type and then one characteristic set per block, and a set is a
//! list of tagged records: the step, the file, the dimensions, the minimum and
//! maximum, where the block starts. Which records a set holds, and in what
//! order, is up to the writer, so each is read by its tag and a record whose
//! width depends on another asks for it by tag: an attribute's value is as
//! many numbers as its dimensions record says, and a sub-block minimum table
//! as wide as the dimensions are many.
//!
//! **BP5.** The index file is records of a kind and a length, so a record of a
//! kind this does not know is passed over by its length. A step in `md.0` is
//! a total and then, per writer, the size of its metadata block and of its
//! attribute block, and the writer count is in the index file, not here. So a
//! step is read into its blocks only when one writer's two sizes and the eight
//! bytes each takes add up to the total, which is a check a step written by two
//! writers cannot pass. What is inside a block is an FFS record, see
//! [`ffs_record`] for how far that reads and what it would take to go further.

use std::sync::Arc;

use crate::template::{Endian, Endian::*, Expr as E, Tag, TaggedRef, Template, Time, Ty as T, Until};

/// The two layouts of the records BP3 and BP4 share. BP4 added the bracketing
/// letters, and in an index entry it spends the two bytes BP3 keeps for a path
/// on an array order and a byte nobody uses.
#[derive(Clone, Copy, PartialEq)]
enum Version {
    Bp3,
    Bp4,
}

use Version::{Bp3, Bp4};

/// Whether an index entry is a variable's or an attribute's. The two are laid
/// out alike and differ in one record: an attribute's value can be an array,
/// and says how long in a dimensions record written before it.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Variable,
    Attribute,
}

/// Where a characteristic set is written. A set in an index is the whole
/// description of a block; a set in a variable's header in a data file holds
/// only its bounds, and its dimensions are in the header beside it.
#[derive(Clone, Copy, PartialEq)]
enum Side {
    Index,
    Data,
}

/// `BPBase::DataTypes`, named by the C++ type ADIOS2 maps to each rather
/// than by ADIOS 1's names: what BP calls `long` is eight bytes.
const DATA_TYPE: &[(i128, &str)] = &[
    (0, "int8"),
    (1, "int16"),
    (2, "int32"),
    (4, "int64"),
    (5, "float"),
    (6, "double"),
    (7, "long double"),
    (STRING, "string"),
    (10, "complex float"),
    (11, "complex double"),
    (STRING_ARRAY, "string array"),
    (50, "uint8"),
    (51, "uint16"),
    (52, "uint32"),
    (54, "uint64"),
    (55, "char"),
];

const STRING: i128 = 9;
const STRING_ARRAY: i128 = 12;

/// Bytes per value for the types in [`scalars`]. A long double is sixteen
/// bytes wherever ADIOS2 is built for x86-64 and is left as those bytes, since
/// what they hold differs by platform.
const WIDTH: &[(i128, i128)] =
    &[(0, 1), (1, 2), (2, 4), (4, 8), (5, 4), (6, 8), (7, 16), (10, 8), (11, 16), (50, 1), (51, 2), (52, 4), (54, 8), (55, 1)];

/// One value of each type with a width.
fn scalars(e: Endian) -> Vec<(i128, T)> {
    let complex = |name: &str, part: T| T::inline_structure(name, vec![("real", part.clone()), ("imaginary", part)]);
    vec![
        (0, T::Int { bits: 8, endian: e }),
        (1, T::Int { bits: 16, endian: e }),
        (2, T::i32(e)),
        (4, T::Int { bits: 64, endian: e }),
        (5, T::F32(e)),
        (6, T::F64(e)),
        (7, T::bytes(E::lit(16))),
        (10, complex("ComplexFloat", T::F32(e))),
        (11, complex("ComplexDouble", T::F64(e))),
        (50, T::u8()),
        (51, T::u16(e)),
        (52, T::u32(e)),
        (54, T::u64(e)),
        // Signed, as `char` is where ADIOS2 is built, and as its Python
        // binding writes an `int8` array: with this type rather than 0.
        (55, T::Int { bits: 8, endian: e }),
    ]
}

/// One value of the type the entry or record around this field declares. A
/// type with no width says nothing about how long its value is, so the rest
/// of the set is bytes.
fn scalar(e: Endian) -> T {
    T::switch(E::field("data_type"), scalars(e), T::bytes(E::Remaining))
}

/// How wide one value of the declared type is, and nothing for a type with no
/// fixed width.
fn width() -> T {
    T::switch(E::field("data_type"), WIDTH.iter().map(|(k, w)| (*k, T::computed(E::lit(*w)))).collect(), T::computed(E::lit(0)))
}

fn data_type() -> T {
    T::enumeration("BpDataType", T::u8(), DATA_TYPE)
}

/// A length in two bytes and that many bytes of text: every name BP writes.
fn bp_string(e: Endian) -> T {
    T::structure_named("BpString", "", "text", vec![("length", T::u16(e)), ("text", T::utf8(E::field("length")))])
}

/// `n`, never below nothing and never past what is left.
fn clamp(n: E) -> E {
    n.at_least(E::lit(0)).at_most(E::Remaining)
}

/// `count` values of `elem`, as rows of `row` when there is more than one
/// row. The innermost dimension is the row, since ADIOS2 writes its arrays in
/// the order C does unless the process group says otherwise.
fn shaped(elem: T, count: E, row: E) -> T {
    let row = || row.clone().at_least(E::lit(1));
    T::switch(
        count.clone().less_or_equal(row()),
        vec![(1, T::array(elem.clone(), count.clone()))],
        T::array(T::array(elem, row()), count.div(row())),
    )
}

/// The values of a block, read as the declared type when the bytes in this
/// window are exactly as many as the dimensions and the type make, and as
/// bytes otherwise. A block an operator compressed is the case this is for:
/// a data file does not say it was compressed, the index does, and the size
/// is what gives it away.
fn block_values(e: Endian, count: E, row: E) -> T {
    let mut cases: Vec<(i128, T)> = scalars(e)
        .into_iter()
        .map(|(k, t)| {
            let w = WIDTH.iter().find(|(x, _)| *x == k).map_or(1, |(_, w)| *w);
            (k, T::switch(E::Remaining.equal_to(count.clone().mul(E::lit(w))), vec![(1, shaped(t, count.clone(), row.clone()))], T::bytes(E::Remaining)))
        })
        .collect();
    cases.push((STRING, bp_string(e)));
    T::switch(E::field("data_type"), cases, T::bytes(E::Remaining))
}

/// The byte order the first byte says, for a file whose every number after it
/// is read that way: `0` little-endian, `1` big-endian.
const ENDIANNESS: &[(i128, &str)] = &[(0, "little-endian"), (1, "big-endian")];

/// `make`, read with the byte order a flag byte at `flag` gives.
fn by_byte_order(flag: E, make: impl Fn(Endian) -> T) -> T {
    T::switch(flag.equal_to(E::lit(1)), vec![(1, make(Big))], make(Little))
}

/// The 64-byte header that opens every BP4 file and a BP5 index. A version
/// string saying which release wrote it and what the file is, the release
/// again as three characters, and flags. The minor release is written as a
/// character counted up from `0`, so 2.12 writes `<`.
fn header(v5: bool) -> T {
    let digit = |name: &str| T::computed(E::field(name).sub(E::lit(b'0' as i128)));
    let mut fields = vec![
        ("version_tag", T::utf8_padded(E::lit(32), 0)),
        ("major_char", T::u8()),
        ("minor_char", T::u8()),
        ("patch_char", T::u8()),
        ("adios_major", digit("major_char")),
        ("adios_minor", digit("minor_char")),
        ("adios_patch", digit("patch_char")),
        ("unused", T::u8()),
        ("byte_order", T::enumeration("BpByteOrder", T::u8(), ENDIANNESS)),
        ("bp_version", T::u8()),
    ];
    let active = || T::enumeration("BpActive", T::u8(), &[(0, "finished"), (1, "being written")]);
    if v5 {
        fields.extend([
            ("bp_minor_version", T::u8()),
            ("active", active()),
            ("array_order", T::enumeration("BpArrayOrder", T::u8(), ARRAY_ORDER)),
            ("flatten_steps", T::u8()),
            ("reserved", T::bytes(E::lit(22))),
        ]);
    } else {
        fields.extend([("active", active()), ("reserved", T::bytes(E::lit(25)))]);
    }
    T::structure("BpHeader", fields)
}

/// `y` for Fortran's order and `n` for C's, which is how a process group says
/// it too.
const ARRAY_ORDER: &[(i128, &str)] = &[(b'y' as i128, "column-major"), (b'n' as i128, "row-major")];

// ---------------------------------------------------------------------------
// Characteristics

/// `BPBase::CharacteristicID`, by ADIOS's names.
const CHARACTERISTIC: &[(i128, &str)] = &[
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

const VALUE: i128 = 0;
const MIN: i128 = 1;
const MAX: i128 = 2;
const OFFSET: i128 = 3;
const DIMENSIONS: i128 = 4;
const PAYLOAD_OFFSET: i128 = 6;
const FILE_INDEX: i128 = 7;
const TIME_INDEX: i128 = 8;
const BITMAP: i128 = 9;
const STATISTICS: i128 = 10;
const TRANSFORM: i128 = 11;
const MINMAX: i128 = 12;

/// `field` of the characteristic before this one in its set whose ID is `id`,
/// and nothing when there is none.
fn earlier_characteristic(id: i128, field: &[&str]) -> E {
    E::Tagged(Arc::new(TaggedRef {
        array: None,
        key: path(&["id"]),
        tag: Tag::Int(id),
        field: path(field),
    }))
}

/// `field` of the characteristic in the set `characteristics` names whose ID
/// is `id`, and nothing when there is none.
fn characteristic_of(id: i128, field: &[&str]) -> E {
    E::tagged("characteristics", &["id"], id, field)
}

fn path(names: &[&str]) -> Arc<[String]> {
    names.iter().map(|s| s.to_string()).collect()
}

/// A set of characteristics: how many, how many bytes, and the records.
///
/// A set in an index gains a few fields worked out from its records, since
/// the questions a reader has about a block, which step and how many values,
/// are answered by records in whatever order the writer put them. In a BP3
/// file that holds its own data, the set also reaches the values it describes;
/// see [`indexed_values`].
fn characteristic_set(e: Endian, kind: Kind, side: Side, v: Version) -> T {
    let mut fields = vec![
        ("count", T::u8()),
        ("length", T::u32(e)),
        ("characteristics", T::sized(clamp(E::field("length")), T::repeat(characteristic(e, kind, side), Until::End))),
    ];
    if side == Side::Index {
        fields.push(("step", T::computed(characteristic_of(TIME_INDEX, &["body"]))));
    }
    let mut machinery = vec![];
    if side == Side::Index && kind == Kind::Variable && v == Bp3 {
        fields.extend(indexed_values(e));
        machinery.extend(["payload_offset", "element_count", "row_length", "width", "compressed"]);
    }
    T::structure_named("CharacteristicSet", "step", "characteristics", fields).machinery(&machinery)
}

/// The values of a block, placed from its set in a BP3 index.
///
/// The process groups before the index hold these bytes already, in the order
/// they were written, and read there without the index. Placed again here they
/// are reached by the variable's name, which is what an index is for, the way
/// a device tree's property reaches its name in the strings block it is
/// already read in. Only when the file holds its data: a BP3 file whose
/// footer says the data is in subfiles has offsets into those.
///
/// A block an operator transformed is not placed, since the offset is to the
/// transformed bytes and not to values of the type.
fn indexed_values(e: Endian) -> Vec<(&'static str, T)> {
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
fn characteristic(e: Endian, kind: Kind, side: Side) -> T {
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
fn value(e: Endian, kind: Kind, side: Side) -> T {
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
fn dimensions(e: Endian) -> T {
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
fn minmax(e: Endian, side: Side) -> T {
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
const STATISTIC_BITS: &[(u32, &str)] =
    &[(0, "min"), (1, "max"), (2, "count"), (3, "sum"), (4, "sum of squares"), (5, "histogram"), (6, "finite")];

/// ADIOS 1's statistics record: one value for each bit the bitmap record
/// before it sets, in the order of the bits. A histogram has no width ADIOS2
/// reads, and ends what can be read of the set.
fn statistics(e: Endian) -> T {
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
fn transform(e: Endian) -> T {
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
fn pg_index(e: Endian, entries: E) -> T {
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
fn element_index(e: Endian, v: Version, kind: Kind, entries: E) -> T {
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
fn index_entry(e: Endian, v: Version, kind: Kind) -> T {
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
const LAYOUT: &[(i128, &str)] = &[(b'K' as i128, "writer's default"), (b'C' as i128, "row-major"), (b'F' as i128, "column-major"), (0, "not given")];

// ---------------------------------------------------------------------------
// Process groups

/// One process group, bracketed in BP4.
///
/// Each count of variables is how many blocks the group holds. The length
/// written beside it is not used: BP3 writes it four bytes short of what the
/// variables take. The attribute count is not used either: it is how many
/// attributes the IO has, and a group writes only the ones no group before it
/// did, so the attributes are read by their length.
fn process_group(e: Endian, v: Version) -> T {
    let mut fields = vec![];
    if v == Bp4 {
        fields.push(("begin", T::magic(b"[PGI")));
    }
    fields.extend([
        ("length", T::u64(e)),
        ("array_order", T::enumeration("BpArrayOrder", T::u8(), ARRAY_ORDER)),
        ("io_name", bp_string(e)),
        ("coordination_var", T::u32(e)),
        ("step_name", bp_string(e)),
        ("step", T::u32(e)),
        ("transport_count", T::u8()),
        ("transports_length", T::u16(e)),
        ("transports", T::sized(clamp(E::field("transports_length")), T::array(transport(e), E::field("transport_count")))),
        ("variable_count", T::u32(e)),
        ("variables_length", T::u64(e)),
        ("variables", T::array(variable_record(e, v), E::field("variable_count").at_most(E::Remaining.div(E::lit(8))))),
        ("attribute_count", T::u32(e)),
        ("attributes_length", T::u64(e)),
        ("attributes", T::sized(clamp(E::field("attributes_length").sub(E::lit(8))), T::repeat(attribute_record(e, v), Until::End))),
    ]);
    let tail = if v == Bp4 { 4 } else { 0 };
    fields.push(("rest", T::when(E::lit(tail).less_than(E::Remaining), T::bytes(E::Remaining.sub(E::lit(tail))))));
    if v == Bp4 {
        fields.push(("end", T::magic(b"PGI]")));
    }
    // A BP4 group's length runs from the length to the end of `PGI]`; a BP3
    // group's does not count its own eight bytes.
    let size = match v {
        Bp4 => E::peek_at(E::lit(32), 64, e).add(E::lit(4)),
        Bp3 => E::peek(64, e).add(E::lit(8)),
    };
    T::sized(clamp(size), T::structure_named("ProcessGroup", "step_name", "variables", fields))
}

/// ADIOS 1's transport method IDs, of which ADIOS2 writes three.
const TRANSPORT: &[(i128, &str)] = &[(0xFF, "null (discards output)"), (0xFE, "unknown"), (2, "POSIX"), (26, "fstream"), (27, "stdio"), (28, "ZeroMQ")];

fn transport(e: Endian) -> T {
    T::structure(
        "Transport",
        vec![
            ("method", T::enumeration("BpTransport", T::u8(), TRANSPORT)),
            ("params_length", T::u16(e)),
            ("params", T::bytes(clamp(E::field("params_length")))),
        ],
    )
}

/// One block of a variable in a process group: its header, and then its
/// values.
///
/// The header has the type and each dimension's count, shape and start, each
/// behind a letter saying whether a variable gives it, which ADIOS 1 used and
/// ADIOS2 never does; ADIOS 1 wrote a four-byte variable ID there instead of
/// the number. The characteristic set after the dimensions holds the bounds.
/// A BP4 block then has a padding length and that much padding ending in
/// `VMD]`, which a writer given a span uses to align the values.
fn variable_record(e: Endian, v: Version) -> T {
    let mut fields = vec![];
    if v == Bp4 {
        fields.push(("begin", T::magic(b"[VMD")));
    }
    fields.extend([("length", T::u64(e)), ("member_id", T::u32(e)), ("name", bp_string(e))]);
    match v {
        Bp3 => fields.push(("path", bp_string(e))),
        Bp4 => fields.extend([("array_order", T::enumeration("BpLayout", T::u8(), LAYOUT)), ("unused", T::u8())]),
    }
    let nth = |i: E| E::elem_field("dimensions", i, &["count"]);
    fields.extend([
        ("data_type", data_type()),
        ("is_dimension", T::enumeration("BpYesNo", T::u8(), YES_NO)),
        ("dims_count", T::u8()),
        ("dims_length", T::u16(e)),
        ("dimensions", T::sized(clamp(E::field("dims_length")), T::array(data_dimension(e), E::field("dims_count")))),
        ("characteristics", characteristic_set(e, Kind::Variable, Side::Data, v)),
    ]);
    if v == Bp4 {
        fields.extend([
            ("padding_length", T::u8()),
            ("padding", T::bytes(clamp(E::field("padding_length").sub(E::lit(4))))),
            ("end", T::magic(b"VMD]")),
        ]);
    }
    let some = || E::lit(0).less_than(E::field("dims_count"));
    fields.extend([
        ("counts", T::array(T::computed(nth(E::idx())), E::field("dims_count"))),
        ("element_count", T::computed(E::product_of("counts"))),
        ("row_length", T::computed(E::cond(some(), nth(E::field("dims_count").sub(E::lit(1))), E::lit(1)))),
        ("values", T::sized(E::Remaining, block_values(e, E::field("element_count"), E::field("row_length")))),
    ]);
    // Both versions count the length from the length field to the end of the
    // values.
    let size = match v {
        Bp4 => E::peek_at(E::lit(32), 64, e).add(E::lit(4)),
        Bp3 => E::peek(64, e),
    };
    T::sized(
        clamp(size),
        T::structure_named("VariableBlock", "name", "values", fields).machinery(&["counts", "element_count", "row_length"]),
    )
}

const YES_NO: &[(i128, &str)] = &[(b'y' as i128, "yes"), (b'n' as i128, "no")];

/// One dimension in a data file's block header. See [`variable_record`].
fn data_dimension(e: Endian) -> T {
    let part = |flag: &'static str| T::switch(E::field(flag).equal_to(E::lit(b'y' as i128)), vec![(1, T::u32(e))], T::u64(e));
    let flag = || T::enumeration("BpYesNo", T::u8(), YES_NO);
    T::structure(
        "DataDimension",
        vec![
            ("count_is_var", flag()),
            ("count", part("count_is_var")),
            ("shape_is_var", flag()),
            ("shape", part("shape_is_var")),
            ("start_is_var", flag()),
            ("start", part("start_is_var")),
        ],
    )
}

/// One attribute in a process group: its name, whether a variable owns it,
/// its type and its value, bracketed in BP4. A string's value is a length and
/// the text; a string array is a count and that many, each a length and text
/// with its nul; anything else is a byte count and that many bytes of values.
fn attribute_record(e: Endian, v: Version) -> T {
    let mut fields = vec![];
    if v == Bp4 {
        fields.push(("begin", T::magic(b"[AMD")));
    }
    let sized_text = |name: &str| T::structure_named(name, "", "text", vec![("size", T::u32(e)), ("text", T::utf8_padded(clamp(E::field("size")), 0))]);
    let arrays = scalars(e)
        .into_iter()
        .map(|(k, t)| {
            let w = WIDTH.iter().find(|(x, _)| *x == k).map_or(1, |(_, w)| *w);
            (k, T::array(t, E::Remaining.div(E::lit(w))))
        })
        .collect();
    let numbers = T::structure(
        "AttributeValues",
        vec![
            ("size", T::u32(e)),
            ("values", T::sized(clamp(E::field("size")), T::switch(E::field("data_type"), arrays, T::bytes(E::Remaining)))),
        ],
    );
    fields.extend([
        ("length", T::u32(e)),
        ("member_id", T::u32(e)),
        ("name", bp_string(e)),
        ("path", bp_string(e)),
        ("is_variable", T::enumeration("BpYesNo", T::u8(), YES_NO)),
        ("variable_id", T::when(E::field("is_variable").equal_to(E::lit(b'y' as i128)), T::u32(e))),
        ("data_type", data_type()),
        (
            "value",
            T::switch(
                E::field("data_type"),
                vec![
                    (STRING, sized_text("AttributeString")),
                    (
                        STRING_ARRAY,
                        T::structure(
                            "AttributeStrings",
                            vec![("count", T::u32(e)), ("strings", T::array(sized_text("AttributeString"), E::field("count").at_most(E::Remaining.div(E::lit(4)))))],
                        ),
                    ),
                ],
                numbers,
            ),
        ),
    ]);
    let tail = if v == Bp4 { 4 } else { 0 };
    fields.push(("rest", T::when(E::lit(tail).less_than(E::Remaining), T::bytes(E::Remaining.sub(E::lit(tail))))));
    if v == Bp4 {
        fields.push(("end", T::magic(b"AMD]")));
    }
    let size = match v {
        Bp4 => E::peek_at(E::lit(32), 32, e).add(E::lit(4)),
        Bp3 => E::peek(32, e),
    };
    T::sized(clamp(size), T::structure_named("AttributeRecord", "name", "value", fields))
}

// ---------------------------------------------------------------------------
// BP3

/// A BP3 file: process groups, the three indices, and the 56 bytes at the end
/// that say where each index starts. Read from the back, as Parquet is: the
/// byte order is the fourth byte from the end, and the footer is placed first
/// so that everything before it can be measured by it.
///
/// Each index is measured by where the next one starts rather than by the
/// length written in it, which is what ADIOS2's reader does, since ADIOS 1
/// wrote those lengths too small.
///
/// A writer of more than one rank puts the data in subfiles and writes this
/// file with the indices alone, starting at nought; its offsets are into the
/// subfile its subfile index names. A subfile is itself a BP3 file with its own
/// footer.
pub fn adios_bp3() -> Template {
    let root = by_byte_order(E::peek_at(E::lit(-32), 8, Big), |e| {
        let at = |name: &str| E::within(&["footer", name]);
        // The whole file, since the fields after the process groups are
        // declared past them and what is left from there is not the file.
        let footer_at = || E::WindowSize.sub(E::lit(FOOTER)).at_least(E::lit(0));
        let between = |from: E, to: E| to.sub(from.clone()).at_least(E::lit(0)).at_most(footer_at().sub(from).at_least(E::lit(0)));
        T::structure(
            "Bp3",
            vec![
                ("footer", T::at(footer_at(), minifooter(e))),
                ("process_groups", T::sized(clamp(at("pg_index_offset")), T::repeat(process_group(e, Bp3), Until::End))),
                (
                    "pg_index",
                    T::at(
                        at("pg_index_offset"),
                        T::sized(between(at("pg_index_offset"), at("variables_index_offset")), pg_index(e, E::Remaining)),
                    ),
                ),
                (
                    "variables_index",
                    T::at(
                        at("variables_index_offset"),
                        T::sized(
                            between(at("variables_index_offset"), at("attributes_index_offset")),
                            element_index(e, Bp3, Kind::Variable, E::Remaining),
                        ),
                    ),
                ),
                (
                    "attributes_index",
                    T::at(
                        at("attributes_index_offset"),
                        T::sized(between(at("attributes_index_offset"), footer_at()), element_index(e, Bp3, Kind::Attribute, E::Remaining)),
                    ),
                ),
            ],
        )
    });
    Template::new("adiosbp3", root)
}

/// How long a BP3 footer is: a 28-byte version and the 28 ADIOS 1 wrote.
const FOOTER: i128 = 56;

/// The footer: the version string of the release that wrote the file, the
/// release again as characters, where each index starts, the byte order, two
/// flags, and the BP version.
///
/// The flags are the byte ADIOS 1 kept its version flags in, one bit for data
/// in subfiles and one for a time index characteristic in every set. ADIOS2
/// writes 3 in a file of indices alone and nothing in a file holding its data,
/// and its reader takes 3 as subfiles and 0 or 2 as none, which is the low bit.
fn minifooter(e: Endian) -> T {
    let digit = |name: &str| T::computed(E::field(name).sub(E::lit(b'0' as i128)));
    T::structure(
        "Bp3Footer",
        vec![
            ("version_tag", T::utf8_padded(E::lit(24), 0)),
            ("major_char", T::u8()),
            ("minor_char", T::u8()),
            ("patch_char", T::u8()),
            ("adios_major", digit("major_char")),
            ("adios_minor", digit("minor_char")),
            ("adios_patch", digit("patch_char")),
            ("unused", T::u8()),
            ("pg_index_offset", T::u64(e)),
            ("variables_index_offset", T::u64(e)),
            ("attributes_index_offset", T::u64(e)),
            ("byte_order", T::enumeration("BpByteOrder", T::u8(), ENDIANNESS)),
            ("reserved", T::u8()),
            ("flags", T::flags("BpFooterFlags", T::u8(), &[(0, "data in subfiles"), (1, "time index")])),
            ("bp_version", T::u8()),
        ],
    )
}

// ---------------------------------------------------------------------------
// BP4

/// A BP4 `md.idx`: the header, and a 64-byte record for each step saying which
/// rank wrote it, where in `md.0` its three indices start and its end is, and
/// when it was written.
pub fn adios_bp4_index() -> Template {
    let record = |e: Endian| {
        T::structure(
            "Bp4StepIndex",
            vec![
                ("step", T::u64(e)),
                ("rank", T::u64(e)),
                ("pg_index_offset", T::u64(e)),
                ("variables_index_offset", T::u64(e)),
                ("attributes_index_offset", T::u64(e)),
                ("step_end_offset", T::u64(e)),
                ("timestamp", T::u64(e)),
                ("reserved", T::u64(e)),
            ],
        )
        .field_time("timestamp", Time::unix())
    };
    let root = T::structure(
        "Bp4Index",
        vec![
            ("header", header(false)),
            (
                "steps",
                by_byte_order(E::within(&["header", "byte_order"]), |e| T::array(record(e), E::Remaining.div(E::lit(64)))),
            ),
            ("rest", T::when(E::lit(0).less_than(E::Remaining), T::bytes(E::Remaining))),
        ],
    );
    Template::new("adiosbp4idx", root)
}

/// A BP4 `md.0`: the header, and each step's three indices one after another.
/// Every offset in them is into a data file of the same directory, the one a
/// set's file index names, and is not followed.
pub fn adios_bp4_metadata() -> Template {
    let step = |e: Endian| {
        T::structure_named(
            "Bp4StepMetadata",
            "step",
            "",
            vec![
                ("pg_index", pg_index(e, E::field("length"))),
                (
                    "step",
                    T::computed(E::cond(
                        E::lit(0).less_than(E::within(&["pg_index", "count"])),
                        E::elem_within(&["pg_index", "entries"], E::lit(0), &["step"]),
                        E::lit(0),
                    )),
                ),
                ("variables_index", element_index(e, Bp4, Kind::Variable, E::field("length"))),
                ("attributes_index", element_index(e, Bp4, Kind::Attribute, E::field("length"))),
            ],
        )
    };
    let root = T::structure(
        "Bp4Metadata",
        vec![("header", header(false)), ("steps", by_byte_order(E::within(&["header", "byte_order"]), |e| T::repeat(step(e), Until::End)))],
    );
    Template::new("adiosbp4md", root)
}

/// A BP4 `data.N`: the header, and process groups to the end.
pub fn adios_bp4_data() -> Template {
    let root = T::structure(
        "Bp4Data",
        vec![
            ("header", header(false)),
            ("process_groups", by_byte_order(E::within(&["header", "byte_order"]), |e| T::repeat(process_group(e, Bp4), Until::End))),
        ],
    );
    Template::new("adiosbp4data", root)
}

// ---------------------------------------------------------------------------
// BP5

/// The kinds of record in a BP5 index, by the letter each is written as.
const RECORD_KIND: &[(i128, &str)] = &[(b's' as i128, "step"), (b'w' as i128, "writer map")];

/// A BP5 `md.idx`: the header, and records to the end, each a kind, a length
/// and a body.
///
/// A step record says where its metadata is in `md.0` and how long, how many
/// times each writer flushed, and for each writer where each flush and the
/// rest of its data went in its data file. How many writers there are is in
/// the writer map record before it, and is worked out here from the length
/// instead, so a step record reads without that one.
pub fn adios_bp5_index() -> Template {
    let step = |e: Endian| {
        let flush = T::structure("Flush", vec![("offset", T::u64(e)), ("size", T::u64(e))]);
        let per_writer = E::field("flush_count").mul(E::lit(16)).add(E::lit(8));
        T::structure(
            "Bp5StepRecord",
            vec![
                ("metadata_offset", T::u64(e)),
                ("metadata_size", T::u64(e)),
                ("flush_count", T::u64(e)),
                ("writer_count", T::computed(E::Remaining.div(per_writer))),
                (
                    "writers",
                    T::array(
                        T::structure(
                            "WriterData",
                            vec![("flushes", T::array(flush, E::field("flush_count"))), ("data_offset", T::u64(e))],
                        ),
                        E::field("writer_count"),
                    ),
                ),
                ("rest", T::when(E::lit(0).less_than(E::Remaining), T::bytes(E::Remaining))),
            ],
        )
    };
    let writers = |e: Endian| {
        T::structure(
            "Bp5WriterMap",
            vec![
                ("writer_count", T::u64(e)),
                ("aggregator_count", T::u64(e)),
                ("subfile_count", T::u64(e)),
                ("subfile_of_writer", T::array(T::u64(e), E::field("writer_count").at_most(E::Remaining.div(E::lit(8))))),
                ("rest", T::when(E::lit(0).less_than(E::Remaining), T::bytes(E::Remaining))),
            ],
        )
    };
    let record = |e: Endian| {
        T::structure_named(
            "Bp5IndexRecord",
            "kind",
            "body",
            vec![
                ("kind", T::enumeration("Bp5RecordKind", T::u8(), RECORD_KIND)),
                ("length", T::u64(e)),
                (
                    "body",
                    T::sized(
                        clamp(E::field("length")),
                        T::switch(E::field("kind"), vec![(b's' as i128, step(e)), (b'w' as i128, writers(e))], T::bytes(E::Remaining)),
                    ),
                ),
            ],
        )
    };
    let root = T::structure(
        "Bp5Index",
        vec![
            ("header", header(true)),
            ("records", by_byte_order(E::within(&["header", "byte_order"]), |e| T::repeat(record(e), Until::End))),
        ],
    );
    Template::new("adiosbp5idx", root)
}

/// The byte order of a BP5 file with no header, which is written in the
/// writer's own. Little-endian unless the first eight bytes only make sense
/// the other way round as a length that fits in the file.
fn plausible_order(make: impl Fn(Endian) -> T) -> T {
    let fits = |e: Endian| E::lit(8).less_or_equal(E::Remaining).both(E::peek(64, e).less_or_equal(E::Remaining));
    T::switch(fits(Little).negate().both(fits(Big)), vec![(1, make(Big))], make(Little))
}

/// A BP5 `md.0`: steps one after another, each a total and then its blocks.
///
/// The blocks are, for each writer, the size of its metadata block, then for
/// each writer the size of its attribute block, then the metadata blocks and
/// then the attribute blocks that are not empty. How many writers is in
/// `md.idx`. One writer's step is recognised by its two sizes and their
/// sixteen bytes coming to the total, and read into its two blocks; a step of
/// several writers is left as bytes.
pub fn adios_bp5_metadata() -> Template {
    let root = plausible_order(|e| {
        let one_writer = E::lit(16)
            .less_or_equal(E::Remaining)
            .both(E::peek(64, e).add(E::peek_at(E::lit(64), 64, e)).add(E::lit(16)).equal_to(E::Remaining));
        let blocks = T::structure(
            "Bp5WriterBlocks",
            vec![
                ("metadata_size", T::u64(e)),
                ("attributes_size", T::u64(e)),
                ("metadata", T::sized(clamp(E::field("metadata_size")), ffs_record(e))),
                (
                    "attributes",
                    T::when(E::lit(0).less_than(E::field("attributes_size")), T::sized(clamp(E::field("attributes_size")), ffs_record(e))),
                ),
            ],
        );
        let step = T::structure(
            "Bp5StepMetadata",
            vec![
                ("total_size", T::u64(e)),
                ("blocks", T::sized(clamp(E::field("total_size")), T::switch(one_writer, vec![(1, blocks)], T::bytes(E::Remaining)))),
            ],
        );
        T::repeat(step, Until::End)
    });
    Template::new("adiosbp5md", root)
}

/// An FFS format ID: the version of the ID, the length of the format it names
/// divided by four, and two hashes of that format. Versions 2 and 3 are
/// twelve bytes and are what BP5 writes; version 3 spends the byte version 2
/// left unused on the top of the length.
fn format_id() -> T {
    T::structure(
        "FfsFormatId",
        vec![
            ("version", T::u8()),
            ("rep_length_top", T::u8()),
            ("rep_length", T::u16(Big)),
            ("hash1", T::bytes(E::lit(4))),
            ("hash2", T::bytes(E::lit(4))),
            ("format_length", T::computed(E::field("rep_length_top").mul(E::lit(65536)).add(E::field("rep_length")).mul(E::lit(4)))),
        ],
    )
}

/// One FFS-encoded record: the ID of the format it is written in, eight bytes
/// saying how much follows the header, padding to eight, and the record.
///
/// The record stays bytes, and this is what reading it would take. It is a
/// C structure laid out as the writer's compiler would: the format with this
/// ID, found in `mmd.0` of the same directory, lists its fields with a name, a
/// type, a size and an offset, and the base structure is that format's record
/// length. A field typed `integer[BitFieldCount]` or `MetaArrayMM8` is a
/// pointer, written as an offset to its array in the variable part after the
/// base, the array as long as another field of the record says and made of
/// the subformat of that name. BP5 then encodes what each field is in its
/// name: `BPG_8_10_temperature` is a global array of eight-byte values of
/// ADIOS type 10 called `temperature`, whose count, shape, offsets and data
/// block location follow in that subformat. Three things are missing from the
/// IR for it: a format looked up in another file by its ID, a field placed at
/// an offset read from a structure whose layout is itself data, and a name
/// split into the parts a variable is described by.
///
/// Only a format with variable parts writes the length, and every format BP5
/// writes has them. A record whose length does not fit the block is left as
/// bytes after its ID.
fn ffs_record(e: Endian) -> T {
    let fits = E::lit(24).less_or_equal(E::Remaining).both(E::peek_at(E::lit(96), 64, e).add(E::lit(24)).less_or_equal(E::Remaining));
    let body = T::structure(
        "FfsRecord",
        vec![
            ("format_id", format_id()),
            ("data_length", T::u64(e)),
            ("padding", T::bytes(E::lit(4))),
            ("data", T::bytes(clamp(E::field("data_length")))),
            ("alignment", T::when(E::lit(0).less_than(E::Remaining), T::bytes(E::Remaining))),
        ],
    );
    let version = E::peek(8, Big);
    T::switch(
        version.clone().equal_to(E::lit(2)).either(version.equal_to(E::lit(3))).both(fits),
        vec![(1, body)],
        T::bytes(E::Remaining),
    )
}

/// A BP5 `mmd.0`: the FFS formats the records in `md.0` are written in, each a
/// length for its ID, a length for its description, the ID and the
/// description.
///
/// The description is FFS's server representation: a header, then for each
/// subformat, the structure and the structures it contains, a header of its
/// own, its fields, and the names and types those fields point at. That much
/// is documented in FFS's `fm_internal.h` and read here, so what a step's
/// metadata holds reads as field names and types even while the metadata
/// itself is bytes.
pub fn adios_bp5_metametadata() -> Template {
    let root = plausible_order(|e| {
        let block = T::structure(
            "Bp5MetaMetadata",
            vec![
                ("id_length", T::u64(e)),
                ("description_length", T::u64(e)),
                ("id", T::sized(clamp(E::field("id_length")), T::switch(E::field("id_length"), vec![(12, format_id())], T::bytes(E::Remaining)))),
                ("description", T::sized(clamp(E::field("description_length")), format_rep())),
            ],
        );
        T::repeat(block, Until::End)
    });
    Template::new("adiosbp5mmd", root)
}

/// FFS's server representation of a format, wire format 1: its length, byte
/// order and representation version, how many subformats, and the format and
/// each of them, the format first and not counted. The lengths are in network
/// byte order; everything inside a subformat is in the byte order it says, and
/// what follows the last is padding to eight.
fn format_rep() -> T {
    let subformat = T::sized(
        clamp(E::peek(16, Big).add(E::peek_at(E::lit(192), 16, Big).mul(E::lit(65536)))),
        T::structure_named(
            "FfsSubformat",
            "",
            "body",
            vec![
                ("rep_length", T::u16(Big)),
                ("rep_version", T::u8()),
                ("byte_order", T::enumeration("FfsByteOrder", T::u8(), ENDIANNESS)),
                ("body", by_byte_order(E::field("byte_order"), subformat_body)),
            ],
        ),
    );
    T::structure(
        "FfsFormatRep",
        vec![
            ("rep_length", T::u16(Big)),
            ("byte_order", T::enumeration("FfsByteOrder", T::u8(), ENDIANNESS)),
            ("rep_version", T::u8()),
            ("subformat_count", T::u8()),
            ("recursive", T::u8()),
            ("rep_length_top", T::u16(Big)),
            (
                "subformats",
                T::switch(
                    E::lit(0).less_than(E::field("rep_version")),
                    vec![(1, T::array(subformat, E::field("subformat_count").add(E::lit(1))))],
                    T::bytes(E::Remaining),
                ),
            ),
            ("rest", T::when(E::lit(0).less_than(E::Remaining), T::bytes(E::Remaining))),
        ],
    )
}

/// A subformat after its first four bytes: where its name is, how many fields
/// and how long its structure is, pointer size, header size, float format,
/// where its optional information is, array order and alignment, the top
/// bytes of the two lengths, and then the fields, the strings they name, and
/// the optional information. Every offset counts from the subformat's start.
fn subformat_body(e: Endian) -> T {
    let field = T::structure_named(
        "FfsField",
        "name",
        "",
        vec![
            ("name_offset", T::u32(e)),
            ("type_offset", T::u32(e)),
            ("size", T::i32(e)),
            ("offset", T::i32(e)),
            ("name", T::at_in_window(E::field("name_offset"), T::cstr())),
            ("type", T::at_in_window(E::field("type_offset"), T::cstr())),
        ],
    );
    // The strings run from the end of the fields to the optional information,
    // or to the end when there is none.
    let strings = E::cond(
        E::lit(0).less_than(E::field("opt_info_offset")),
        E::field("opt_info_offset").sub(E::Pos),
        E::Remaining,
    );
    T::structure_named(
        "FfsSubformatBody",
        "name",
        "fields",
        vec![
            ("name_offset", T::u32(e)),
            ("field_count", T::u32(e)),
            ("record_length", T::i32(e)),
            ("pointer_size", T::u8()),
            ("header_size", T::u8()),
            ("float_format", T::u16(e)),
            ("opt_info_offset", T::u16(e)),
            ("column_major", T::u8()),
            ("alignment", T::u8()),
            ("rep_length_top", T::u16(Big)),
            ("opt_info_offset_top", T::u16(e)),
            ("fields", T::array(field, E::field("field_count").at_most(E::Remaining.div(E::lit(16))))),
            ("name", T::at_in_window(E::field("name_offset"), T::cstr())),
            ("strings", T::sized(clamp(strings), T::repeat(T::cstr(), Until::End))),
            ("optional_info", T::when(E::lit(0).less_than(E::Remaining), T::bytes(E::Remaining))),
        ],
    )
}

// ---------------------------------------------------------------------------
// Recognition
//
// Which ADIOS2 template a file is, from its first bytes and its length, is
// asked twice, since the two kinds of evidence belong to different bands of
// `PROBES`. What each file can be told by:
//
// - Every BP4 file and a BP5 `md.idx` open with `ADIOS-BP v`, a release, and a
//   word saying which file it is, with the BP version at byte 37. That is a
//   signature and it is sound.
// - A BP5 `md.0` has no header. A step written by one writer is a total and
//   two sizes that add up to it with their sixteen bytes, followed by an FFS
//   format ID; that is claimed. A step of several writers is not.
// - A BP5 `mmd.0` has no header either. It opens with an ID length of twelve, a
//   description length that fits the file, an FFS ID of version 2 or 3, and a
//   description whose length, in its own first two bytes, is four times the
//   length the ID gives. Four fields agreeing is claimed.
// - A BP5 `data.N` is bytes the metadata points at, and nothing marks it.
// - A BP3 file keeps its version at the end, which a sniff only sees when the
//   whole file is in the window. Its front is a process group, or for a file
//   whose data is in subfiles a process group index, and either names its step
//   twice, as digits and as a number; that agreeing, with the rest of the
//   header in bounds, is claimed.

/// Both questions, in the order `PROBES` asks them.
#[cfg(test)]
fn sniff(head: &[u8], len: u64) -> Option<&'static str> {
    sniff_signed(head, len).or_else(|| sniff_agreeing(head, len))
}

/// The files that sign themselves: every BP4 file and a BP5 index.
pub fn sniff_signed(head: &[u8], _len: u64) -> Option<&'static str> {
    headed(head)
}

/// The files recognised by fields that agree with each other rather than by a
/// signature: a BP5 `md.0` or `mmd.0`, and a BP3 file.
pub fn sniff_agreeing(head: &[u8], len: u64) -> Option<&'static str> {
    if is_bp5_metametadata(head, len) {
        return Some("adiosbp5mmd");
    }
    if is_bp5_metadata(head, len) {
        return Some("adiosbp5md");
    }
    if is_bp3(head, len) {
        return Some("adiosbp3");
    }
    None
}


fn headed(head: &[u8]) -> Option<&'static str> {
    if !head.starts_with(b"ADIOS-BP v") || head.len() < 64 || head[36] > 1 {
        return None;
    }
    let tag = &head[10..32];
    let word = tag.iter().position(|&b| b == b' ').map(|i| &tag[i + 1..])?;
    let is = |w: &[u8]| word.starts_with(w) && word[w.len()..].iter().all(|&b| b == 0);
    match head[37] {
        4 if is(b"Index Table") => Some("adiosbp4idx"),
        4 if is(b"Metadata") => Some("adiosbp4md"),
        4 if is(b"Data") => Some("adiosbp4data"),
        5 if is(b"Index Table") => Some("adiosbp5idx"),
        _ => None,
    }
}

fn u64_le(b: &[u8], at: usize) -> Option<u64> {
    b.get(at..at + 8).map(|s| u64::from_le_bytes(s.try_into().unwrap()))
}

fn u32_le(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap()))
}

fn u16_le(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2).map(|s| u16::from_le_bytes(s.try_into().unwrap()))
}

fn u16_be(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2).map(|s| u16::from_be_bytes(s.try_into().unwrap()))
}

fn is_bp5_metametadata(head: &[u8], len: u64) -> bool {
    let (Some(12), Some(info)) = (u64_le(head, 0), u64_le(head, 8)) else { return false };
    let (Some(&version), Some(&top), Some(rep)) = (head.get(16), head.get(17), u16_be(head, 18)) else { return false };
    let (Some(length), Some(&rep_version)) = (u16_be(head, 28), head.get(31)) else { return false };
    let ideal = (top as u64) << 18 | (rep as u64) << 2;
    let written = length as u64 | (u16_be(head, 34).unwrap_or(0) as u64) << 16;
    (version == 2 || version == 3)
        && rep != 0
        && rep_version > 0
        && info.checked_add(28).is_some_and(|n| n <= len)
        && written >= 8
        && written <= info
        && written >> 2 == ideal >> 2
}

fn is_bp5_metadata(head: &[u8], len: u64) -> bool {
    let (Some(total), Some(meta), Some(attr)) = (u64_le(head, 0), u64_le(head, 8), u64_le(head, 16)) else { return false };
    let Some(&version) = head.get(24) else { return false };
    let adds_up = meta.checked_add(attr).and_then(|n| n.checked_add(16)) == Some(total);
    adds_up && total.checked_add(8).is_some_and(|n| n <= len) && meta >= 24 && (version == 2 || version == 3) && u16_be(head, 26).is_some_and(|r| r != 0)
}

fn is_bp3(head: &[u8], len: u64) -> bool {
    bp3_footer(head, len) || bp3_group_front(head, len) || bp3_index_front(head, len)
}

/// The footer of a BP3 file the sniff can see all of, little-endian or not.
fn bp3_footer(head: &[u8], len: u64) -> bool {
    if head.len() as u64 != len || head.len() < 56 {
        return false;
    }
    let f = &head[head.len() - 56..];
    if !f.starts_with(b"ADIOS-BP v") || f[55] != 3 || f[52] > 1 {
        return false;
    }
    let read = |at: usize| {
        let b: [u8; 8] = f[at..at + 8].try_into().unwrap();
        if f[52] == 1 { u64::from_be_bytes(b) } else { u64::from_le_bytes(b) }
    };
    let (pg, vars, attrs) = (read(28), read(36), read(44));
    pg <= vars && vars <= attrs && attrs <= len - 56
}

/// A step name, which is the step written as digits, and the step after it.
fn step_named_twice(head: &[u8], at: usize) -> Option<usize> {
    let n = u16_le(head, at)? as usize;
    let digits = head.get(at + 2..at + 2 + n)?;
    if n == 0 || n > 10 || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let text: u64 = std::str::from_utf8(digits).ok()?.parse().ok()?;
    (u32_le(head, at + 2 + n)? as u64 == text).then_some(at + 2 + n + 4)
}

/// A name of printable characters, and where it ends.
fn printable_name(head: &[u8], at: usize) -> Option<usize> {
    let n = u16_le(head, at)? as usize;
    let name = head.get(at + 2..at + 2 + n)?;
    name.iter().all(|b| (0x20..0x7f).contains(b)).then_some(at + 2 + n)
}

/// A BP3 file that starts with its first process group, little-endian.
fn bp3_group_front(head: &[u8], len: u64) -> bool {
    let Some(length) = u64_le(head, 0) else { return false };
    if length.checked_add(8).is_none_or(|n| n > len) || !matches!(head.get(8), Some(b'y' | b'n')) {
        return false;
    }
    let Some(after_name) = printable_name(head, 9) else { return false };
    let Some(after_step) = step_named_twice(head, after_name + 4) else { return false };
    let (Some(&count), Some(methods)) = (head.get(after_step), u16_le(head, after_step + 1)) else { return false };
    methods as usize == 3 * count as usize
}

/// A BP3 file of indices alone, which starts with its process group index.
fn bp3_index_front(head: &[u8], len: u64) -> bool {
    let (Some(count), Some(length)) = (u64_le(head, 0), u64_le(head, 8)) else { return false };
    if count == 0 || length.checked_add(16).is_none_or(|n| n > len) {
        return false;
    }
    let Some(entry) = u16_le(head, 16) else { return false };
    let Some(after_name) = printable_name(head, 18) else { return false };
    if !matches!(head.get(after_name), Some(b'y' | b'n')) {
        return false;
    }
    let Some(after_step) = step_named_twice(head, after_name + 5) else { return false };
    after_step + 8 == 18 + entry as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, NodeInfo, Value};
    use crate::source::MemSource;

    /// Writes the records BP3 and BP4 share, little-endian.
    #[derive(Default)]
    struct W(Vec<u8>);

    impl W {
        fn u8(&mut self, v: u8) -> &mut Self {
            self.0.push(v);
            self
        }
        fn u16(&mut self, v: u16) -> &mut Self {
            self.0.extend(v.to_le_bytes());
            self
        }
        fn u32(&mut self, v: u32) -> &mut Self {
            self.0.extend(v.to_le_bytes());
            self
        }
        fn u64(&mut self, v: u64) -> &mut Self {
            self.0.extend(v.to_le_bytes());
            self
        }
        fn bytes(&mut self, v: &[u8]) -> &mut Self {
            self.0.extend_from_slice(v);
            self
        }
        fn name(&mut self, s: &str) -> &mut Self {
            self.u16(s.len() as u16).bytes(s.as_bytes())
        }
        fn len(&self) -> usize {
            self.0.len()
        }
    }

    fn header(kind: &str, version: u8) -> Vec<u8> {
        let mut h = format!("ADIOS-BP v2.<.1 {kind}").into_bytes();
        h.resize(32, 0);
        h.extend(b"2<1\0");
        h.extend([0, version, 0, 0]);
        h.resize(64, 0);
        h
    }

    /// A characteristic set, from records already packed.
    fn set(records: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = records.concat();
        let mut w = W::default();
        w.u8(records.len() as u8).u32(body.len() as u32).bytes(&body);
        w.0
    }

    fn dims(d: &[(u64, u64, u64)]) -> Vec<u8> {
        let mut w = W::default();
        w.u8(DIMENSIONS as u8).u8(d.len() as u8).u16(24 * d.len() as u16);
        for (c, s, o) in d {
            w.u64(*c).u64(*s).u64(*o);
        }
        w.0
    }

    fn rec(id: i128, body: &[u8]) -> Vec<u8> {
        [vec![id as u8], body.to_vec()].concat()
    }

    /// An index entry: the header BP4 writes and the sets.
    fn entry(name: &str, data_type: u8, sets: &[Vec<u8>]) -> Vec<u8> {
        let mut body = W::default();
        body.u32(0).u16(0).name(name).u8(b'K').u8(0).u8(data_type).u64(sets.len() as u64);
        for s in sets {
            body.bytes(s);
        }
        let mut w = W::default();
        w.u32(body.len() as u32).bytes(&body.0);
        w.0
    }

    fn node(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize]) -> NodeInfo {
        ev.node(d, path).unwrap()
    }

    /// Finds a child by name, so a test says which field it means.
    fn child(ev: &mut Evaluator, d: &Document<MemSource>, parent: &[usize], name: &str) -> Vec<usize> {
        let n = node(ev, d, parent).child_count as usize;
        for i in 0..n {
            let p = [parent, &[i]].concat();
            let info = node(ev, d, &p);
            if info.name == name || info.name.ends_with(&format!(" {name}")) {
                return p;
            }
        }
        panic!("no {name} under {parent:?}");
    }

    fn at(ev: &mut Evaluator, d: &Document<MemSource>, names: &[&str]) -> Vec<usize> {
        names.iter().fold(vec![], |p, n| match n.parse::<usize>() {
            Ok(i) => [&p[..], &[i]].concat(),
            Err(_) => child(ev, d, &p, n),
        })
    }

    #[test]
    fn a_bp4_index_is_a_header_and_a_record_a_step() {
        let mut f = header("Index Table", 4);
        for step in 1..=2u64 {
            let mut w = W::default();
            w.u64(step).u64(0).u64(64).u64(111).u64(200).u64(300).u64(1_780_000_000).u64(0);
            f.extend(w.0);
        }
        assert_eq!(sniff(&f, f.len() as u64), Some("adiosbp4idx"));
        let d = Document::new(MemSource(f));
        let mut ev = Evaluator::new(adios_bp4_index());
        let steps = at(&mut ev, &d, &["steps"]);
        assert_eq!(node(&mut ev, &d, &steps).child_count, 2);
        let step = at(&mut ev, &d, &["steps", "1", "step"]);
        assert_eq!(node(&mut ev, &d, &step).value.as_int(), Some(2));
        let bp = at(&mut ev, &d, &["header", "bp_version"]);
        assert_eq!(node(&mut ev, &d, &bp).value.as_int(), Some(4));
        let minor = at(&mut ev, &d, &["header", "adios_minor"]);
        assert_eq!(node(&mut ev, &d, &minor).value.as_int(), Some(12));
    }

    /// An attribute's value is as many numbers as the dimensions record before
    /// it says, and a variable's sets read their records by tag whatever order
    /// they are in.
    #[test]
    fn characteristics_are_read_by_their_tags() {
        let var = entry(
            "temperature",
            6,
            &[set(&[
                rec(TIME_INDEX, &2u32.to_le_bytes()),
                rec(FILE_INDEX, &0u32.to_le_bytes()),
                dims(&[(2, 4, 0), (3, 3, 0)]),
                rec(MINMAX, &[1u16.to_le_bytes().to_vec(), 0.5f64.to_le_bytes().to_vec(), 9.5f64.to_le_bytes().to_vec()].concat()),
                rec(OFFSET, &77u64.to_le_bytes()),
                rec(PAYLOAD_OFFSET, &99u64.to_le_bytes()),
            ])],
        );
        let attr = entry(
            "grid",
            2,
            &[set(&[
                rec(TIME_INDEX, &1u32.to_le_bytes()),
                dims(&[(2, 0, 0)]),
                rec(VALUE, &[4i32.to_le_bytes(), 3i32.to_le_bytes()].concat()),
            ])],
        );
        let mut w = W::default();
        // A process group index of one entry.
        let mut pg = W::default();
        pg.name("sim").u8(b'n').u32(0).name("2").u32(2).u64(64);
        w.u64(1).u64(pg.len() as u64 + 2).u16(pg.len() as u16).bytes(&pg.0);
        w.u32(1).u64(var.len() as u64).bytes(&var);
        w.u32(1).u64(attr.len() as u64).bytes(&attr);
        let f = [header("Metadata", 4), w.0].concat();
        assert_eq!(sniff(&f, f.len() as u64), Some("adiosbp4md"));
        let d = Document::new(MemSource(f));
        let mut ev = Evaluator::new(adios_bp4_metadata());
        let steps = at(&mut ev, &d, &["steps"]);
        assert_eq!(node(&mut ev, &d, &steps).child_count, 1);
        let sets = at(&mut ev, &d, &["steps", "0", "variables_index", "entries", "0", "sets"]);
        let s = [&sets[..], &[0]].concat();
        let step = child(&mut ev, &d, &s, "step");
        assert_eq!(node(&mut ev, &d, &step).value.as_int(), Some(2));
        let records = child(&mut ev, &d, &s, "characteristics");
        assert_eq!(node(&mut ev, &d, &records).child_count, 6);
        let max = at(&mut ev, &d, &["steps", "0", "variables_index", "entries", "0", "sets", "0", "characteristics", "3", "body", "max"]);
        assert_eq!(node(&mut ev, &d, &max).value, Value::Float(9.5));
        let value = at(&mut ev, &d, &["steps", "0", "attributes_index", "entries", "0", "sets", "0", "characteristics", "2", "body"]);
        let v = node(&mut ev, &d, &value);
        assert_eq!(v.child_count, 2);
        assert_eq!(node(&mut ev, &d, &[&value[..], &[1]].concat()).value, Value::Int(3));
    }

    /// A process group in a data file reads each block's values as its type
    /// and dimensions say, and a block whose size does not match them as bytes.
    #[test]
    fn a_data_file_reads_blocks_by_their_headers() {
        let block = |name: &str, data_type: u8, counts: &[u64], values: &[u8]| {
            let mut body = W::default();
            body.u32(0).name(name).u8(b'K').u8(0).u8(data_type).u8(b'n').u8(counts.len() as u8).u16(27 * counts.len() as u16);
            for c in counts {
                body.u8(b'n').u64(*c).u8(b'n').u64(*c).u8(b'n').u64(0);
            }
            body.bytes(&set(&[])).u8(4).bytes(b"VMD]").bytes(values);
            let mut w = W::default();
            w.bytes(b"[VMD").u64(body.len() as u64 + 8).bytes(&body.0);
            w.0
        };
        let doubles: Vec<u8> = (0..6).flat_map(|i| (i as f64).to_le_bytes()).collect();
        let blocks = [block("grid", 6, &[2, 3], &doubles), block("packed", 6, &[4], &[1, 2, 3])].concat();
        let mut pg = W::default();
        pg.u8(b'n').name("sim").u32(0).name("1").u32(1).u8(1).u16(3).u8(2).u16(0);
        pg.u32(2).u64(blocks.len() as u64).bytes(&blocks).u32(0).u64(0).bytes(b"PGI]");
        let mut w = W::default();
        w.bytes(b"[PGI").u64(pg.len() as u64 + 8).bytes(&pg.0);
        let f = [header("Data", 4), w.0].concat();
        assert_eq!(sniff(&f, f.len() as u64), Some("adiosbp4data"));
        let n = f.len() as u64;
        let d = Document::new(MemSource(f));
        let mut ev = Evaluator::new(adios_bp4_data());
        let groups = at(&mut ev, &d, &["process_groups"]);
        let g = node(&mut ev, &d, &groups);
        assert_eq!((g.child_count, g.offset_bits + g.size_bits), (1, n * 8));
        let grid = at(&mut ev, &d, &["process_groups", "0", "variables", "0", "values"]);
        assert_eq!(node(&mut ev, &d, &grid).child_count, 2);
        assert_eq!(node(&mut ev, &d, &[&grid[..], &[1, 2]].concat()).value, Value::Float(5.0));
        let packed = at(&mut ev, &d, &["process_groups", "0", "variables", "1", "values"]);
        assert_eq!(node(&mut ev, &d, &packed).type_name, "bytes[]");
    }

    #[test]
    fn a_bp5_index_reads_records_by_kind_and_length() {
        let mut w = W::default();
        w.u8(b'w').u64(32).u64(1).u64(1).u64(1).u64(0);
        w.u8(b's').u64(32).u64(0).u64(1216).u64(0).u64(0);
        w.u8(b'x').u64(3).bytes(&[1, 2, 3]);
        let f = [header("Index Table", 5), w.0].concat();
        assert_eq!(sniff(&f, f.len() as u64), Some("adiosbp5idx"));
        let d = Document::new(MemSource(f));
        let mut ev = Evaluator::new(adios_bp5_index());
        let records = at(&mut ev, &d, &["records"]);
        assert_eq!(node(&mut ev, &d, &records).child_count, 3);
        let writers = at(&mut ev, &d, &["records", "1", "body", "writer_count"]);
        assert_eq!(node(&mut ev, &d, &writers).value.as_int(), Some(1));
        let size = at(&mut ev, &d, &["records", "1", "body", "metadata_size"]);
        assert_eq!(node(&mut ev, &d, &size).value.as_int(), Some(1216));
    }

    /// One writer's sizes adding up to the total is what reads a step into
    /// its blocks; two writers' cannot, and stay bytes.
    #[test]
    fn a_bp5_step_of_one_writer_reads_its_ffs_blocks() {
        let record = |len: usize| {
            let mut w = W::default();
            w.bytes(&[2, 0, 0, 5, 1, 2, 3, 4, 5, 6, 7, 8]).u64(len as u64 - 24).u32(0).bytes(&vec![0xAB; len - 24]);
            w.0
        };
        let (meta, attrs) = (record(64), record(40));
        let mut w = W::default();
        w.u64(16 + 64 + 40).u64(64).u64(40).bytes(&meta).bytes(&attrs);
        assert_eq!(sniff(&w.0, w.0.len() as u64), Some("adiosbp5md"));
        // Two writers: metadata of 64 and 9 bytes, attributes of 40 and none.
        let two = [32u64 + 64 + 9 + 40, 64, 9, 40, 0].iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>();
        w.bytes(&two).bytes(&meta).bytes(&[0; 9]).bytes(&attrs);
        let d = Document::new(MemSource(w.0));
        let mut ev = Evaluator::new(adios_bp5_metadata());
        assert_eq!(node(&mut ev, &d, &[]).child_count, 2);
        let data = at(&mut ev, &d, &["0", "blocks", "metadata", "data"]);
        assert_eq!(node(&mut ev, &d, &data).size_bits, 40 * 8);
        let second = at(&mut ev, &d, &["1", "blocks"]);
        assert_eq!(node(&mut ev, &d, &second).type_name, "bytes[]");
    }

    /// A BP3 file of one process group, one variable of three int32s and one
    /// double attribute, with its indices and footer.
    fn bp3_file() -> Vec<u8> {
        let characteristics = |records: &[Vec<u8>]| set(records);
        let values: Vec<u8> = [7i32, 8, 9].iter().flat_map(|v| v.to_le_bytes()).collect();
        let mut var = W::default();
        var.u32(0).name("v").name("").u8(2).u8(b'n').u8(1).u16(27);
        var.u8(b'n').u64(3).u8(b'n').u64(3).u8(b'n').u64(0);
        var.bytes(&characteristics(&[dims(&[(3, 3, 0)]), rec(MIN, &7i32.to_le_bytes()), rec(MAX, &9i32.to_le_bytes())]));
        let header_len = 8 + var.len();
        let mut attr = W::default();
        attr.u32(0).name("a").name("").u8(b'n').u8(6).u32(8).bytes(&2.5f64.to_le_bytes());
        let mut group = W::default();
        group.u8(b'n').name("io").u32(0).name("1").u32(1).u8(1).u16(3).u8(2).u16(0);
        let variables_at = 8 + group.len() + 12;
        group.u32(1).u64(0).u64((header_len + values.len()) as u64).bytes(&var.0).bytes(&values);
        group.u32(1).u64(8 + 4 + attr.len() as u64).u32(4 + attr.len() as u32).bytes(&attr.0);
        let attribute_at = 8 + group.len() - attr.len() - 4;
        let mut f = W::default();
        f.u64(group.len() as u64).bytes(&group.0);
        let pg_start = f.len() as u64;
        let mut pg = W::default();
        pg.name("io").u8(b'n').u32(0).name("1").u32(1).u64(0);
        f.u64(1).u64(pg.len() as u64 + 2).u16(pg.len() as u16).bytes(&pg.0);
        let vars_start = f.len() as u64;
        let placed = |at: usize, payload: usize| [rec(OFFSET, &(at as u64).to_le_bytes()), rec(PAYLOAD_OFFSET, &(payload as u64).to_le_bytes())];
        let [off, pay] = placed(variables_at, variables_at + header_len);
        let var_set = characteristics(&[
            rec(TIME_INDEX, &1u32.to_le_bytes()),
            rec(FILE_INDEX, &0u32.to_le_bytes()),
            rec(MIN, &7i32.to_le_bytes()),
            rec(MAX, &9i32.to_le_bytes()),
            dims(&[(3, 3, 0)]),
            off,
            pay,
        ]);
        let index_entry = |name: &str, data_type: u8, s: &[u8]| {
            let mut body = W::default();
            body.u32(0).u16(0).name(name).u16(0).u8(data_type).u64(1).bytes(s);
            let mut w = W::default();
            w.u32(body.len() as u32).bytes(&body.0);
            w.0
        };
        let entry = index_entry("v", 2, &var_set);
        f.u32(1).u64(entry.len() as u64).bytes(&entry);
        let attrs_start = f.len() as u64;
        let [off, pay] = placed(attribute_at, attribute_at + 4 + 4 + 3 + 2 + 1 + 1);
        let attr_set = characteristics(&[
            rec(TIME_INDEX, &1u32.to_le_bytes()),
            rec(FILE_INDEX, &0u32.to_le_bytes()),
            dims(&[(1, 0, 0)]),
            rec(VALUE, &2.5f64.to_le_bytes()),
            off,
            pay,
        ]);
        let entry = index_entry("a", 6, &attr_set);
        f.u32(1).u64(entry.len() as u64).bytes(&entry);
        let mut tag = b"ADIOS-BP v2.<.1".to_vec();
        tag.resize(24, 0);
        f.bytes(&tag).bytes(b"2<1\0").u64(pg_start).u64(vars_start).u64(attrs_start).bytes(&[0, 0, 0, 3]);
        f.0
    }

    /// The values a BP3 index places by payload offset are the ones the
    /// process group before it reads in order, and the file is recognised
    /// by its footer when the whole of it is seen and by its front when not.
    #[test]
    fn a_bp3_index_places_the_values_its_process_group_holds() {
        let f = bp3_file();
        assert_eq!(sniff(&f, f.len() as u64), Some("adiosbp3"));
        assert_eq!(sniff(&f[..64], f.len() as u64 + 4096), Some("adiosbp3"));
        let n = f.len() as u64;
        let d = Document::new(MemSource(f));
        let mut ev = Evaluator::new(adios_bp3());
        let walked = at(&mut ev, &d, &["process_groups", "0", "variables", "0", "values"]);
        let walked = node(&mut ev, &d, &walked);
        assert_eq!(walked.child_count, 3);
        let placed = at(&mut ev, &d, &["variables_index", "variables_index", "entries", "0", "sets", "0", "values", "values"]);
        let placed_info = node(&mut ev, &d, &placed);
        assert_eq!((placed_info.offset_bits, placed_info.child_count), (walked.offset_bits, 3));
        assert_eq!(node(&mut ev, &d, &[&placed[..], &[2]].concat()).value, Value::Int(9));
        let attribute = at(&mut ev, &d, &["attributes_index", "attributes_index", "entries", "0", "sets", "0", "characteristics", "3", "body"]);
        assert_eq!(node(&mut ev, &d, &attribute).value, Value::Float(2.5));
        let in_group = at(&mut ev, &d, &["process_groups", "0", "attributes", "0", "value", "values", "0"]);
        assert_eq!(node(&mut ev, &d, &in_group).value, Value::Float(2.5));
        let footer = at(&mut ev, &d, &["footer", "footer"]);
        let footer = node(&mut ev, &d, &footer);
        assert_eq!(footer.offset_bits + footer.size_bits, n * 8);
    }

    #[test]
    fn nothing_else_is_claimed() {
        assert_eq!(sniff(b"ADIOS-BP v2.9.0 Something\0\0\0\0\0\0\0", 32), None);
        let mut zeros = vec![0u8; 256];
        assert_eq!(sniff(&zeros, 256), None);
        zeros[0] = 12;
        assert_eq!(sniff(&zeros, 256), None);
    }
}
