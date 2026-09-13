//! Apache Arrow IPC files: columns in the layout a program keeps them in
//! memory, with FlatBuffers saying where each one is.
//!
//! An Arrow file is what `pyarrow.feather.write_feather` writes, and what
//! Feather version 2, `.arrow` and `.feather` all name. It opens with `ARROW1`
//! and two bytes of padding, and then it is an IPC stream: a run of messages,
//! each a FlatBuffers `Message` saying what it is and how long a body follows
//! it, and the body. The first message is the schema; after it come dictionary
//! batches and record batches, and then a marker saying the stream has ended.
//! Behind the stream is a footer, another FlatBuffer, holding the schema again
//! and a `Block` for every batch: where it starts, how long its metadata is,
//! how long its body is. The last ten bytes of the file are the footer's length
//! and `ARROW1` again, so the footer is found from the back, the way Parquet's
//! is, and a reader wanting the third record batch reads the footer and goes
//! straight there.
//!
//! So that is how this reads it. The footer is placed from the back, and the
//! batches are placed from the footer's blocks with a [`Ty::Gather`], rather
//! than by walking the stream from the front. The one message the footer does
//! not list is the schema at the front, which is always at byte 8, and the one
//! stretch it does not account for is the end-of-stream marker, which is
//! placed where the last block ends. Everything else in the file is a table
//! some other table points at, so the root covers only the magic and the
//! padding, and the cursor finds every other byte through what was placed
//! over it.
//!
//! Every name, field order and enum case below is transcribed from
//! `format/Schema.fbs`, `format/Message.fbs` and `format/File.fbs` in
//! apache/arrow (Apache-2.0), read on 2026-09-14, and the layout of a message
//! from `docs/source/format/Columnar.rst` of the same date. See
//! [`crate::formats::flatbuf`] for how a table is read.
//!
//! ## One message
//!
//! A message opens with four bytes of `FF` and then a four-byte length, the
//! length of the FlatBuffer that follows, padding included. The `FF`s are the
//! continuation marker Arrow 0.15 added (ARROW-6314); a file written before
//! it, or by a writer asked for the legacy format, has the length alone, and
//! a reader tells the two apart by whether the first four bytes are all ones,
//! which no length can be. A length of zero is the end-of-stream marker, so
//! that is eight bytes in a current file and four in a legacy one.
//!
//! In a file, a block's `metaDataLength` counts both of those numbers and the
//! FlatBuffer, so the body starts `metaDataLength` bytes after the block's
//! `offset`. The comment in `File.fbs` says the offset is past the message
//! header, and every file written by pyarrow 25 disagrees with it: the offset
//! is the `FF`s.
//!
//! ## Buffers
//!
//! A record batch's body is its columns' buffers back to back, and the
//! batch's `buffers` say where each one is and how long, counted from the
//! start of the body. Each is placed from there with a gather, so the body is
//! a list of buffers with gaps of padding between them.
//!
//! What a buffer holds is the part a template has to work for, because
//! nothing in the batch says. The batch lists a `FieldNode` for every field of
//! the schema, children included, in the order a depth-first walk meets them,
//! and a run of buffers per node whose count depends on the field's type: two
//! for an integer, three for a string, one for a struct, none for a null. So
//! each node walks the schema one step on from the node before it, and each
//! buffer is matched to the node whose buffers start where it is. See
//! [`node_walk`] and [`buffer_walk`]. The walk follows three levels of field,
//! which is a list of structs or a map; a fourth level stops it, and every
//! buffer from there to the end of the batch is placed and left as bytes.
//!
//! Then a buffer reads as what its column's layout says it is for. A validity
//! bitmap stays bytes; offsets are 32 or 64-bit integers, one more than the
//! rows; integers, floats, dates and timestamps are arrays of their type, the
//! dates and timestamps with the moment each one is; booleans are bits; a
//! decimal or a fixed-size binary is its bytes, one run per value; a string
//! column's data is one run of text and a binary one's one run of bytes; a
//! view is sixteen bytes that hold a short value or say where a long one is. A
//! union's type ids are 8-bit integers and a dense union's offsets 32-bit
//! ones. Only a batch's own rows are values: a buffer holding more than its
//! node's `length` has the rest as `beyond_length`.
//! Nested columns name their buffers and read them the same way, the list's
//! offsets and then its values under the child's own name.
//!
//! A dictionary batch holds one column, the dictionary of the field whose
//! `DictionaryEncoding.id` matches its own, and its buffers read as that
//! field's value type; in a record batch the same column's data buffer is
//! `indices`, of the index type.
//!
//! A compressed body compresses each buffer on its own behind an eight-byte
//! uncompressed length, -1 for one left as it was. ZSTD buffers open and read
//! as the plain ones do. LZ4 is the frame format, and the codec here reads the
//! blocks inside a frame and not the frame, so those buffers keep their bytes.

use crate::codec::Codec;
use crate::formats::flatbuf::{self, Field, Of, Scalar::*, Struct, Table, What as W};
use crate::template::{Anchor, Endian::{Big, Little}, Expr as E, Step, Template, Time, Ty as T};
use std::sync::Arc;

/// What an Arrow file opens and closes with.
pub const MAGIC: &[u8] = b"ARROW1";

const METADATA_VERSION: &[(i128, &str)] = &[(0, "V1"), (1, "V2"), (2, "V3"), (3, "V4"), (4, "V5")];
const FEATURE: &[(i128, &str)] = &[(0, "UNUSED"), (1, "DICTIONARY_REPLACEMENT"), (2, "COMPRESSED_BODY")];
const UNION_MODE: &[(i128, &str)] = &[(0, "Sparse"), (1, "Dense")];
const PRECISION: &[(i128, &str)] = &[(0, "HALF"), (1, "SINGLE"), (2, "DOUBLE")];
const DATE_UNIT: &[(i128, &str)] = &[(0, "DAY"), (1, "MILLISECOND")];
const TIME_UNIT: &[(i128, &str)] = &[(0, "SECOND"), (1, "MILLISECOND"), (2, "MICROSECOND"), (3, "NANOSECOND")];
const INTERVAL_UNIT: &[(i128, &str)] = &[(0, "YEAR_MONTH"), (1, "DAY_TIME"), (2, "MONTH_DAY_NANO")];
const DICTIONARY_KIND: &[(i128, &str)] = &[(0, "DenseArray")];
const ENDIANNESS: &[(i128, &str)] = &[(0, "Little"), (1, "Big")];
const COMPRESSION_TYPE: &[(i128, &str)] = &[(0, "LZ4_FRAME"), (1, "ZSTD")];
const BODY_COMPRESSION_METHOD: &[(i128, &str)] = &[(0, "BUFFER")];

/// The members of `union Type`, numbered as the union numbers them: from one,
/// in the order they are declared, with zero for `NONE`.
const TYPE: &[(i128, &str)] = &[
    (1, "Null"),
    (2, "Int"),
    (3, "FloatingPoint"),
    (4, "Binary"),
    (5, "Utf8"),
    (6, "Bool"),
    (7, "Decimal"),
    (8, "Date"),
    (9, "Time"),
    (10, "Timestamp"),
    (11, "Interval"),
    (12, "List"),
    (13, "Struct_"),
    (14, "Union"),
    (15, "FixedSizeBinary"),
    (16, "FixedSizeList"),
    (17, "Map"),
    (18, "Duration"),
    (19, "LargeBinary"),
    (20, "LargeUtf8"),
    (21, "LargeList"),
    (22, "RunEndEncoded"),
    (23, "BinaryView"),
    (24, "Utf8View"),
    (25, "ListView"),
    (26, "LargeListView"),
];

const MESSAGE_HEADER: &[(i128, &str)] =
    &[(1, "Schema"), (2, "DictionaryBatch"), (3, "RecordBatch"), (4, "Tensor"), (5, "SparseTensor")];

const fn f(name: &'static str, what: flatbuf::What) -> Field {
    Field { name, what }
}

const fn empty(name: &'static str) -> Table {
    Table { name, fields: &[] }
}

/// The schema of a column, from `Schema.fbs`.
pub const FIELD: Table = Table {
    name: "Field",
    fields: &[
        f("name", W::String),
        f("nullable", W::Scalar(Bool)),
        f("type", W::Union("Type", TYPE)),
        f("dictionary", W::Table("DictionaryEncoding")),
        f("children", W::Vector(Of::Table("Field"))),
        f("custom_metadata", W::Vector(Of::Table("KeyValue"))),
    ],
};

pub const SCHEMA: Table = Table {
    name: "Schema",
    fields: &[
        f("endianness", W::Enum(Short, "Endianness", ENDIANNESS)),
        f("fields", W::Vector(Of::Table("Field"))),
        f("custom_metadata", W::Vector(Of::Table("KeyValue"))),
        f("features", W::Vector(Of::Enum(Long, "Feature", FEATURE))),
    ],
};

pub const RECORD_BATCH: Table = Table {
    name: "RecordBatch",
    fields: &[
        f("length", W::Scalar(Long)),
        f("nodes", W::Vector(Of::Struct("FieldNode"))),
        f("buffers", W::Vector(Of::Struct("Buffer"))),
        f("compression", W::Table("BodyCompression")),
        f("variadicBufferCounts", W::Vector(Of::Scalar(Long))),
    ],
};

pub const DICTIONARY_BATCH: Table = Table {
    name: "DictionaryBatch",
    fields: &[f("id", W::Scalar(Long)), f("data", W::Table("RecordBatch")), f("isDelta", W::Scalar(Bool))],
};

pub const MESSAGE: Table = Table {
    name: "Message",
    fields: &[
        f("version", W::Enum(Short, "MetadataVersion", METADATA_VERSION)),
        f("header", W::Union("MessageHeader", MESSAGE_HEADER)),
        f("bodyLength", W::Scalar(Long)),
        f("custom_metadata", W::Vector(Of::Table("KeyValue"))),
    ],
};

pub const FOOTER: Table = Table {
    name: "Footer",
    fields: &[
        f("version", W::Enum(Short, "MetadataVersion", METADATA_VERSION)),
        f("schema", W::Table("Schema")),
        f("dictionaries", W::Vector(Of::Struct("Block"))),
        f("recordBatches", W::Vector(Of::Struct("Block"))),
        f("custom_metadata", W::Vector(Of::Table("KeyValue"))),
    ],
};

/// Every table the three schema files declare. `Tensor` and `SparseTensor`
/// are members of `MessageHeader` and are named so that the union can say so,
/// but their fields are not transcribed: no IPC file or stream writer puts one
/// in a stream of record batches, and a message holding one still shows its
/// vtable, which says how many fields it has and where they are.
const TABLES: &[Table] = &[
    empty("Null"),
    empty("Struct_"),
    empty("List"),
    empty("LargeList"),
    empty("ListView"),
    empty("LargeListView"),
    Table { name: "FixedSizeList", fields: &[f("listSize", W::Scalar(Int))] },
    Table { name: "Map", fields: &[f("keysSorted", W::Scalar(Bool))] },
    Table {
        name: "Union",
        fields: &[f("mode", W::Enum(Short, "UnionMode", UNION_MODE)), f("typeIds", W::Vector(Of::Scalar(Int)))],
    },
    Table { name: "Int", fields: &[f("bitWidth", W::Scalar(Int)), f("is_signed", W::Scalar(Bool))] },
    Table { name: "FloatingPoint", fields: &[f("precision", W::Enum(Short, "Precision", PRECISION))] },
    empty("Utf8"),
    empty("Binary"),
    empty("LargeUtf8"),
    empty("LargeBinary"),
    empty("Utf8View"),
    empty("BinaryView"),
    Table { name: "FixedSizeBinary", fields: &[f("byteWidth", W::Scalar(Int))] },
    empty("Bool"),
    empty("RunEndEncoded"),
    Table {
        name: "Decimal",
        fields: &[f("precision", W::Scalar(Int)), f("scale", W::Scalar(Int)), f("bitWidth", W::Scalar(Int))],
    },
    Table { name: "Date", fields: &[f("unit", W::Enum(Short, "DateUnit", DATE_UNIT))] },
    Table { name: "Time", fields: &[f("unit", W::Enum(Short, "TimeUnit", TIME_UNIT)), f("bitWidth", W::Scalar(Int))] },
    Table { name: "Timestamp", fields: &[f("unit", W::Enum(Short, "TimeUnit", TIME_UNIT)), f("timezone", W::String)] },
    Table { name: "Interval", fields: &[f("unit", W::Enum(Short, "IntervalUnit", INTERVAL_UNIT))] },
    Table { name: "Duration", fields: &[f("unit", W::Enum(Short, "TimeUnit", TIME_UNIT))] },
    Table { name: "KeyValue", fields: &[f("key", W::String), f("value", W::String)] },
    Table {
        name: "DictionaryEncoding",
        fields: &[
            f("id", W::Scalar(Long)),
            f("indexType", W::Table("Int")),
            f("isOrdered", W::Scalar(Bool)),
            f("dictionaryKind", W::Enum(Short, "DictionaryKind", DICTIONARY_KIND)),
        ],
    },
    FIELD,
    SCHEMA,
    Table {
        name: "BodyCompression",
        fields: &[
            f("codec", W::Enum(Byte, "CompressionType", COMPRESSION_TYPE)),
            f("method", W::Enum(Byte, "BodyCompressionMethod", BODY_COMPRESSION_METHOD)),
        ],
    },
    RECORD_BATCH,
    DICTIONARY_BATCH,
    MESSAGE,
    FOOTER,
    empty("Tensor"),
    empty("SparseTensor"),
];

const STRUCTS: &[Struct] = &[
    Struct { name: "FieldNode", fields: &[("length", Long), ("null_count", Long)] },
    Struct { name: "Buffer", fields: &[("offset", Long), ("length", Long)] },
    Struct { name: "Block", fields: &[("offset", Long), ("metaDataLength", Int), ("bodyLength", Long)] },
];

/// The footer's length: the four bytes ten from the end, in front of the
/// closing magic. Read without being placed, from wherever it is asked.
fn footer_length() -> E {
    E::peek_at(E::lit(-80), 32, Little)
}

/// Where the footer starts, counted from the front of the file, and never
/// before the end of the padding: a length larger than the file still places
/// what is there rather than nothing.
fn footer_start() -> E {
    E::WindowSize.sub(E::lit(10)).sub(footer_length()).at_least(E::lit(8))
}

/// One message: the marker, the length, the FlatBuffer, and the body.
///
/// The marker is there when the first four bytes are all ones, which a length
/// cannot be. A length of zero is the end of the stream, so a message of that
/// length has no FlatBuffer and no body.
///
/// The body follows the FlatBuffer, lined up on a multiple of eight. Writers
/// fold that padding into the length already, which is what Columnar.rst asks
/// for, so the padding here is nearly always absent; it is kept for a writer
/// that does not, since without it the body would be read four bytes early.
/// Counted from the front of the file, which is where an Arrow stream's eight
/// byte alignment is counted from: the file format's own magic and padding
/// are eight bytes too.
fn message() -> T {
    let size = E::field("metadata_size");
    let has_metadata = size.clone().greater_than(E::lit(0));
    let end_of_metadata = E::start_of(E::field("metadata")).add(size.clone());
    let padding = E::cond(has_metadata.clone(), end_of_metadata.pad_to(8), E::lit(0));
    let body_length = flatbuf::read_or(&["metadata", "root", "table"], &MESSAGE, "bodyLength", 0);
    T::structure(
        "EncapsulatedMessage",
        vec![
            ("continuation", T::when(E::peek(32, Little).equal_to(E::lit(0xFFFF_FFFF_u32)), T::u32(Little))),
            ("metadata_size", T::i32(Little)),
            ("metadata", T::when(has_metadata.clone(), T::sized(size, flatbuf::buffer("arrow", "Message")))),
            ("padding", T::when(padding.clone().greater_than(E::lit(0)), T::bytes(padding))),
            ("body", T::when(has_metadata.both(body_length.clone().greater_than(E::lit(0))), T::sized(body_length, body()))),
        ],
    )
    .machinery(&["continuation", "metadata_size", "padding"])
    // A buffer is called by the column it belongs to, which its descriptor in
    // the metadata worked out. What it is for, validity or offsets or data,
    // is the name of its type.
    .field_elem_named_from("body", E::placer(E::field("column")))
}

/// What a message's body holds, once the header says what the message is: the
/// buffers of a record batch, or of the record batch inside a dictionary
/// batch, each placed where its `Buffer` says.
fn body() -> T {
    let header = E::within(&["metadata", "root", "table", "header_type"]);
    T::switch(header, vec![(3, buffers(&["header", "table"])), (2, buffers(&["header", "table", "data", "table"]))], T::bytes(E::Remaining))
}

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

const ROLE: &[(i128, &str)] = &[
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

// How the bytes of a buffer read, once its column and its purpose are known.
// Internal to this reader; what a reader of the file sees is the type each
// one picks.
const BYTES: i128 = 0;
const BITS: i128 = 1;
const I8: i128 = 2;
const U8: i128 = 3;
const I16: i128 = 4;
const U16: i128 = 5;
const I32: i128 = 6;
const U32: i128 = 7;
const I64: i128 = 8;
const U64: i128 = 9;
const F16: i128 = 10;
const F32: i128 = 11;
const F64: i128 = 12;
const DAYS: i128 = 13;
const DATE_MILLIS: i128 = 14;
/// A timestamp with a zone, in seconds, then milliseconds, microseconds and
/// nanoseconds: `TimeUnit` added to this.
const INSTANT: i128 = 15;
/// A timestamp with no zone, which is wall-clock digits: the same four units.
const WALL_CLOCK: i128 = 19;
const YEAR_MONTH: i128 = 23;
const DAY_TIME: i128 = 24;
const MONTH_DAY_NANO: i128 = 25;
const FIXED_WIDTH: i128 = 26;
const TEXT: i128 = 27;
const BINARY: i128 = 28;
const TEXT_VIEWS: i128 = 29;
const BINARY_VIEWS: i128 = 30;
const VALIDITY_BITMAP: i128 = 31;
/// Offsets that run one past the last value, 32-bit and 64-bit.
const ENDS32: i128 = 32;
const ENDS64: i128 = 33;

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

fn table(name: &str) -> &'static Table {
    TABLES.iter().find(|t| t.name == name).unwrap_or_else(|| panic!("no table {name}"))
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
fn field_layout() -> T {
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
const SCHEMA_TABLE: &[&str] = &["schema", "metadata", "root", "table", "header", "table"];
const SCHEMA_FIELDS: &[&str] = &["schema", "metadata", "root", "table", "header", "table", "fields", "vector", "elements"];

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
fn node_walk() -> Vec<(&'static str, T)> {
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
fn buffer_walk() -> Vec<(&'static str, T)> {
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

/// The array one buffer holds, by what `reading` says it is.
///
/// As many values as the node's `length`, and never more than the buffer
/// holds. A buffer may hold more than its batch uses: pyarrow writing a table
/// in batches of three writes the first batch's buffers whole, five values
/// and all, and only the node says that two of them belong to no row of this
/// batch. Those bytes are `beyond_length` rather than values.
///
/// A validity bitmap is a bit per value, low bit first, and stays bytes: a
/// row of noughts and ones says less than the byte it is packed in, and the
/// values beside it say which rows are null anyway. A boolean column's data
/// is the same packing and reads as bits, because there the bits are the
/// values.
fn reading() -> T {
    let count = E::placer(E::field("count"));
    let width = E::placer(E::field("width"));
    let array = |ty: T, bytes: i128| T::array(ty, count.clone().at_most(E::Remaining.div(E::lit(bytes))));
    let ends = |ty: T, bytes: i128| T::array(ty, count.clone().add(E::lit(1)).at_most(E::Remaining.div(E::lit(bytes))));
    let timed = |name: &str, ty: T, bytes: i128, time: Time| {
        T::structure_named(name, "", "values", vec![("values", array(ty, bytes))]).field_time("values", time)
    };
    let i64le = T::Int { bits: 64, endian: Little };
    let mut cases = vec![
        (VALIDITY_BITMAP, T::bytes(count.clone().add(E::lit(7)).div(E::lit(8)).at_most(E::Remaining))),
        // In whole bytes, and read on one row where a row is a byte. A bit
        // packed from the low end sits at the far end of its byte from where
        // the byte is counted from, so a run of them read as fields covers
        // its bits in the order the values go and not in the order the byte
        // is laid out, and the bits of the last byte past the last value would
        // read as a gap between buffers.
        (
            BITS,
            T::sized(
                count.clone().add(E::lit(7)).div(E::lit(8)).at_most(E::Remaining),
                T::inline_structure(
                    "bits",
                    vec![(
                        "values",
                        T::array(T::enumeration("bool", T::UInt { bits: 1, endian: Little }, &[(0, "false"), (1, "true")]), count.clone().at_most(E::Remaining.mul(E::lit(8)))),
                    )],
                ),
            ),
        ),
        (ENDS32, ends(T::i32(Little), 4)),
        (ENDS64, ends(i64le.clone(), 8)),
        (I8, array(T::Int { bits: 8, endian: Little }, 1)),
        (U8, array(T::u8(), 1)),
        (I16, array(T::Int { bits: 16, endian: Little }, 2)),
        (U16, array(T::u16(Little), 2)),
        (I32, array(T::i32(Little), 4)),
        (U32, array(T::u32(Little), 4)),
        (I64, array(i64le.clone(), 8)),
        (U64, array(T::u64(Little), 8)),
        (F16, array(T::F16(Little), 2)),
        (F32, array(T::F32(Little), 4)),
        (F64, array(T::F64(Little), 8)),
        (DAYS, timed("date32[day]", T::i32(Little), 4, Time::counted(0, 86_400_000_000_000))),
        (DATE_MILLIS, timed("date64[ms]", i64le.clone(), 8, Time::unix_millis())),
        (YEAR_MONTH, array(T::i32(Little), 4)),
        (DAY_TIME, array(T::inline_structure("DayTime", vec![("days", T::i32(Little)), ("milliseconds", T::i32(Little))]), 8)),
        (
            MONTH_DAY_NANO,
            array(T::inline_structure("MonthDayNano", vec![("months", T::i32(Little)), ("days", T::i32(Little)), ("nanoseconds", i64le.clone())]), 16),
        ),
        (FIXED_WIDTH, T::array(T::bytes(width.clone()), E::cond(width.clone().greater_than(E::lit(0)), count.clone().at_most(E::Remaining.div(width)), E::lit(0)))),
        (TEXT, T::utf8(E::Remaining)),
        (BINARY, T::bytes(E::Remaining)),
        (TEXT_VIEWS, array(view(true), 16)),
        (BINARY_VIEWS, array(view(false), 16)),
    ];
    let units = [("s", Time::unix()), ("ms", Time::unix_millis()), ("us", Time::unix_micros()), ("ns", Time::unix_nanos())];
    for (i, (unit, time)) in units.iter().enumerate() {
        let i = i as i128;
        cases.push((INSTANT + i, timed(&format!("timestamp[{unit}, UTC]"), i64le.clone(), 8, time.clone())));
        cases.push((WALL_CLOCK + i, timed(&format!("timestamp[{unit}]"), i64le.clone(), 8, time.clone().zone_unknown())));
    }
    T::switch(E::placer(E::field("reading")), cases, T::bytes(E::Remaining))
}

/// One view of a binary view column: sixteen bytes that hold a short value
/// whole, or a long one's first four bytes and where the rest of it is.
///
/// The twelve bytes after the length are sized as twelve whatever they turn
/// out to hold, so that a column of a million views is still placed by
/// multiplying.
fn view(text: bool) -> T {
    let inlined = if text { T::utf8_padded(E::lit(12), 0) } else { T::bytes(E::lit(12)) };
    let elsewhere = T::inline_structure(
        "ViewReference",
        vec![("prefix", T::bytes(E::lit(4))), ("buffer_index", T::i32(Little)), ("offset", T::i32(Little))],
    );
    T::inline_structure(
        "View",
        vec![
            ("length", T::i32(Little)),
            ("value", T::sized(E::lit(12), T::switch(E::field("length").less_or_equal(E::lit(12)), vec![(1, inlined)], elsewhere))),
        ],
    )
}

/// Every buffer of the batch reached by `batch` from the message's `metadata`,
/// each placed where its `Buffer` says, counted from the start of the body.
///
/// A compressed body compresses each buffer on its own and writes, in front of
/// each, how long it is uncompressed, or -1 for one left as it was because
/// compressing it did not help. ZSTD opens and reads as the buffer would have;
/// an LZ4 frame keeps its bytes, since the codec here reads LZ4 blocks and
/// not the frame around them.
fn buffers(batch: &[&str]) -> T {
    let mut path: Vec<&str> = vec!["metadata", "root", "table"];
    path.extend_from_slice(batch);
    let mut compression: Vec<&str> = path.clone();
    compression.extend(["compression", "table"]);
    let compressed = flatbuf::is_written(&path, &RECORD_BATCH, "compression");
    let codec = flatbuf::read_or(&compression, table("BodyCompression"), "codec", 0);
    let mut steps: Vec<Step> = path.iter().map(|s| Step::field(s)).collect();
    steps.extend([Step::field("buffers"), Step::field("vector"), Step::field("elements"), Step::each()]);
    let role = E::placer(E::field("role"));
    let beyond_length = || T::when(E::Remaining.greater_than(E::lit(0)), T::bytes(E::Remaining));
    let plain = |name: &str| T::structure_named(name, "", "values", vec![("values", reading()), ("beyond_length", beyond_length())]);
    let i64le = || T::Int { bits: 64, endian: Little };
    let packed = |name: &str| {
        let stored = T::structure_named(
            name,
            "",
            "values",
            vec![("uncompressed_length", i64le()), ("values", reading()), ("beyond_length", beyond_length())],
        );
        let opened = T::switch(codec.clone(), vec![(1, T::decoded(E::Remaining, Codec::Zstd, plain(name)))], T::bytes(E::Remaining));
        let packed = T::structure_named(name, "", "values", vec![("uncompressed_length", i64le()), ("values", opened)]);
        T::switch(
            E::peek(64, Little).equal_to(E::lit(u64::MAX)),
            vec![(1, stored.machinery(&["uncompressed_length"]))],
            packed.machinery(&["uncompressed_length"]),
        )
    };
    let by_role = |make: &dyn Fn(&str) -> T| {
        T::switch(role.clone(), ROLE.iter().skip(1).map(|(code, name)| (*code, make(name))).collect(), T::bytes(E::Remaining))
    };
    let length = E::placer(E::field("length"));
    let contents = T::switch(compressed.both(length.clone().greater_than(E::lit(0))), vec![(1, by_role(&packed))], by_role(&plain));
    T::gather(steps, E::field("offset"), Anchor::SelfAligned(1), E::lit(0), T::sized(length, contents))
}

/// Where the last batch the footer lists ends, of the blocks in `list`: its
/// offset, its metadata and its body, added up. Nothing when there are none.
fn last_block_end(list: &str) -> E {
    let footer = ["footer", "root", "table"];
    let count: Vec<&str> = footer.iter().copied().chain([list, "vector", "count"]).collect();
    let elements: Vec<&str> = footer.iter().copied().chain([list, "vector", "elements"]).collect();
    let last = E::within(&count).sub(E::lit(1));
    let part = |name: &str| E::elem_within(&elements, last.clone(), &[name]);
    E::cond(
        flatbuf::is_written(&footer, &FOOTER, list).both(E::within(&count).greater_than(E::lit(0))),
        part("offset").add(part("metaDataLength")).add(part("bodyLength")),
        E::lit(0),
    )
}

/// Where the stream ends: after whichever of the schema, the last dictionary
/// and the last record batch comes last in the file.
fn stream_end() -> E {
    let schema = E::lit(8).add(E::size_of("schema"));
    schema.at_least(last_block_end("dictionaries")).at_least(last_block_end("recordBatches"))
}

/// Whether the end-of-stream marker is where the stream ends: four bytes of
/// zero in a legacy file, or all ones and then zero in a current one, and
/// room for it before the footer either way.
fn end_of_stream_here() -> E {
    let ahead = stream_end().sub(E::Pos).mul(E::lit(8));
    let room = stream_end().add(E::lit(4)).less_or_equal(footer_start());
    let legacy = E::peek_at(ahead.clone(), 32, Little).equal_to(E::lit(0));
    let current = E::peek_at(ahead, 64, Big).equal_to(E::lit(0xFFFF_FFFF_0000_0000_u64));
    room.both(legacy.either(current))
}

fn file() -> T {
    let blocks = vec![
        Step::field("footer"),
        Step::field("root"),
        Step::field("table"),
        Step::fields(&["dictionaries", "recordBatches"]),
        Step::field("vector"),
        Step::field("elements"),
        Step::each(),
    ];
    let footer_size = footer_length().at_most(E::WindowSize.sub(E::lit(18)).at_least(E::lit(0)));
    T::structure(
        "ArrowFile",
        vec![
            ("magic", T::magic(MAGIC)),
            ("padding", T::bytes(E::lit(2))),
            ("footer", T::at_origin(footer_start(), T::sized(footer_size, flatbuf::buffer("arrow", "Footer")))),
            ("footer_length", T::at_origin(E::WindowSize.sub(E::lit(10)), T::i32(Little))),
            ("footer_magic", T::at_origin(E::WindowSize.sub(E::lit(6)), T::magic(MAGIC))),
            ("schema", T::at_origin(E::lit(8), T::Named("arrow.EncapsulatedMessage".into()))),
            ("batches", T::gather(blocks, E::field("offset"), Anchor::Origin, E::lit(0), T::Named("arrow.EncapsulatedMessage".into()))),
            ("end_of_stream", T::when(end_of_stream_here(), T::at_origin(stream_end(), T::Named("arrow.EncapsulatedMessage".into())))),
        ],
    )
    .machinery(&["padding"])
}

/// The fields of `extra`, a structure built only to hold them, spliced into
/// the structure `ty` at `at`, or at the end.
fn splice(ty: &mut T, at: Option<usize>, extra: T) {
    let T::Struct(definition) = ty else { unreachable!("only a structure is extended") };
    let T::Struct(extra) = extra else { unreachable!() };
    let definition = Arc::make_mut(definition);
    let at = at.unwrap_or(definition.fields.len());
    definition.fields.splice(at..at, extra.fields.iter().cloned());
    definition.machinery.extend(extra.machinery.iter().cloned());
}

fn position(ty: &T, name: &str) -> usize {
    let T::Struct(definition) = ty else { unreachable!() };
    definition.fields.iter().position(|f| &*f.name == name).unwrap_or_else(|| panic!("no field {name}"))
}

/// The schema's own tables, and what this reader adds to five of them so that
/// a batch's buffers can be named from its schema. See [`node_walk`].
fn with_types(mut t: Template) -> Template {
    // Everything a walk adds is machinery but what a reader is looking for:
    // which column, and for a node how it is laid out, for a buffer what it
    // is for.
    let added = |fields: Vec<(&'static str, T)>, shown: &[&str]| {
        let hidden: Vec<&str> = fields.iter().map(|(n, _)| *n).filter(|n| !shown.contains(n)).collect();
        T::structure("", fields).machinery(&hidden)
    };
    for (name, mut ty) in flatbuf::types("arrow", TABLES, STRUCTS) {
        match name.as_str() {
            "arrow.Field" => splice(&mut ty, None, T::structure("", vec![("layout", field_layout())]).machinery(&["layout"])),
            "arrow.FieldNode" => splice(&mut ty, None, added(node_walk(), &["column", "layout"])),
            "arrow.Buffer" => splice(&mut ty, None, added(buffer_walk(), &["column", "role"])),
            // Which column a dictionary belongs to, found by its id among the
            // schema's fields, and declared before `data` so that the batch
            // inside can start its walk there.
            "arrow.DictionaryBatch" => {
                let id = flatbuf::read_or(&[], &DICTIONARY_BATCH, "id", 0);
                let found = E::tagged_in_by(E::within(SCHEMA_FIELDS), &["table", "layout", "dictionary_id"], id.clone(), &["table", "layout", "index"]);
                let check = E::elem_within(SCHEMA_FIELDS, found.clone(), &["table", "layout", "dictionary_id"]).equal_to(id);
                let fields = flatbuf::is_written(SCHEMA_TABLE, &SCHEMA, "fields");
                let index = E::cond(fields.both(check), found, E::lit(-1));
                let name = T::computed_text(E::elem_within(SCHEMA_FIELDS, E::field("schema_field"), &["table", "name", "string", "text"]));
                let extra = T::structure(
                    "",
                    vec![("schema_field", T::computed(index)), ("column", T::when(E::field("schema_field").greater_or_equal(E::lit(0)), name))],
                )
                .machinery(&["schema_field"]);
                let after = position(&ty, "id") + 1;
                splice(&mut ty, Some(after), extra);
            }
            // The count of data buffers each view column has in this batch,
            // moved in front of the nodes that count with it. Where its bytes
            // are is the vtable's to say, so only the order of the rows moves.
            "arrow.RecordBatch" => {
                let T::Struct(definition) = &mut ty else { unreachable!() };
                let definition = Arc::make_mut(definition);
                let from = definition.fields.iter().position(|f| &*f.name == "variadicBufferCounts").unwrap();
                let to = definition.fields.iter().position(|f| &*f.name == "nodes").unwrap();
                let moved = definition.fields.remove(from);
                definition.fields.insert(to, moved);
            }
            _ => {}
        }
        t = t.with_type(&name, ty);
    }
    t.with_type("arrow.EncapsulatedMessage", message())
}

pub fn arrow() -> Template {
    with_types(Template::new("arrow", file()))
}

/// An IPC stream: the same messages with nothing around them. The schema
/// first, then the dictionary and record batches one after another, then the
/// end-of-stream marker, which a writer may leave off by closing the stream
/// instead. So the run of messages stops at a message of no length or at the
/// end of the file, whichever comes first, and the marker is the last message
/// of the run when there is one.
///
/// Nothing indexes a stream, so there is nothing to place the batches from
/// but the lengths of the messages before them. Placed rather than laid out
/// from the root, for the reason the file's messages are: every table in them
/// is placed too, and the cursor finds a table's bytes through what was
/// placed over them only where the root does not already cover them.
fn stream() -> T {
    let message = || T::Named("arrow.EncapsulatedMessage".into());
    let messages = T::repeat(message(), crate::template::Until::FieldValue { field: "metadata_size".into(), value: 0 });
    T::structure(
        "ArrowStream",
        vec![("schema", T::at_origin(E::lit(0), message())), ("messages", T::at_origin(E::size_of("schema"), messages))],
    )
}

pub fn arrow_stream() -> Template {
    with_types(Template::new("arrowstream", stream()))
}

/// Whether a file is an IPC stream, which has no magic of its own.
///
/// What it has instead is a schema message at the front, and that is a lot of
/// structure to agree by chance. The first four bytes are all ones, the
/// continuation marker every writer since Arrow 0.15 puts there; the next
/// four are a length that is a multiple of eight and fits in the file; the
/// FlatBuffer that length counts has a root table inside it whose vtable is
/// inside it too, has room for a version, a header type and a header, says
/// version V1 to V5 where it says one, and says its header is a `Schema`.
///
/// A legacy stream, written without the marker, opens with the length alone
/// and is not recognised: four bytes of small number are what half the files
/// there are open with, and the rest of the evidence would be carrying all of
/// the weight. Such a file still opens with this template picked by hand.
pub fn is_stream(h: &[u8], len: u64) -> bool {
    let u32_at = |at: usize| h.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    let u16_at = |at: usize| h.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize);
    if u32_at(0) != Some(0xFFFF_FFFF) {
        return false;
    }
    let Some(size) = u32_at(4) else { return false };
    if size < 16 || size % 8 != 0 || size as u64 + 8 > len {
        return false;
    }
    let end = 8 + size;
    let Some(root) = u32_at(8) else { return false };
    let table = 8 + root;
    if root < 4 || table + 4 > end {
        return false;
    }
    let Some(back) = u32_at(table) else { return false };
    let vtable = table as i64 - i64::from(back as u32 as i32);
    if vtable < 12 || vtable as usize + 4 > end {
        return false;
    }
    let vtable = vtable as usize;
    let (Some(vtable_size), Some(table_size)) = (u16_at(vtable), u16_at(vtable + 2)) else { return false };
    if vtable_size < 10 || vtable_size % 2 != 0 || vtable + vtable_size > end || table_size < 8 || table + table_size > end {
        return false;
    }
    let entry = |id: usize| u16_at(vtable + 4 + 2 * id).filter(|&at| at != 0 && at < table_size);
    // `Message.version` is id 0, `header_type` id 1 and `header` id 2.
    if u16_at(vtable + 4).is_some_and(|at| at != 0) && !entry(0).and_then(|at| u16_at(table + at)).is_some_and(|v| v <= 4) {
        return false;
    }
    let schema = entry(1).and_then(|at| h.get(table + at)) == Some(&1);
    schema && entry(2).is_some_and(|at| at + 4 <= table_size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, eval::Evaluator, source::MemSource};

    /// The smallest stream there is: a schema message with no fields, and the
    /// end-of-stream marker.
    ///
    /// The FlatBuffer, by offset from where it starts: the root offset at 0, the
    /// message's vtable at 4, the message at 16 with its version at +4, its
    /// header type at +6 and the offset to its header at +8, the schema's
    /// vtable at 28 and the schema at 32.
    fn smallest(header_type: u8) -> Vec<u8> {
        let mut m = Vec::new();
        m.extend(16u32.to_le_bytes());
        m.extend([10, 0, 12, 0, 4, 0, 6, 0, 8, 0, 0, 0]);
        m.extend(12i32.to_le_bytes());
        m.extend([4, 0, header_type, 0]);
        m.extend(8u32.to_le_bytes());
        m.extend([4, 0, 4, 0]);
        m.extend(4i32.to_le_bytes());
        m.extend([0, 0, 0, 0]);
        let mut v = vec![0xff; 4];
        v.extend((m.len() as u32).to_le_bytes());
        v.extend(m);
        v.extend([0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0]);
        v
    }

    #[test]
    fn a_stream_is_recognised_by_its_schema_message() {
        let v = smallest(1);
        assert!(is_stream(&v, v.len() as u64));
        // A record batch first is not how a stream opens.
        let batch = smallest(3);
        assert!(!is_stream(&batch, batch.len() as u64));
        // Nor is the marker on its own, or a file too short for its length.
        assert!(!is_stream(&[0xff; 64], 64));
        assert!(!is_stream(&v, 20));
    }

    #[test]
    fn the_smallest_stream_reads_as_a_schema_and_an_end() {
        let d = Document::new(MemSource(smallest(1)));
        let mut e = Evaluator::new(arrow_stream());
        // schema -> message -> metadata -> root -> table -> Message -> header_type.
        let header = e.node(&d, &[0, 0, 2, 0, 1, 0, 3, 0]).unwrap().value;
        assert_eq!(header.as_int(), Some(1));
        let messages = e.node(&d, &[1, 0]).unwrap();
        assert_eq!(messages.offset_bits, 48 * 8, "after the schema message and its length");
        assert_eq!(messages.child_count, 1, "the end-of-stream marker");
        assert_eq!(e.node(&d, &[1, 0, 0, 1]).unwrap().value.as_int(), Some(0));
    }
}
