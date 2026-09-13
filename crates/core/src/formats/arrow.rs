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

use crate::formats::flatbuf::{self, Field, Of, Scalar::*, Struct, Table, What as W};
use crate::template::{Anchor, Endian::{Big, Little}, Expr as E, Step, Template, Ty as T};

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
}

/// What a message's body holds, once the header says what the message is.
fn body() -> T {
    T::bytes(E::Remaining)
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

fn with_types(mut t: Template) -> Template {
    for (name, ty) in flatbuf::types("arrow", TABLES, STRUCTS) {
        t = t.with_type(&name, ty);
    }
    t.with_type("arrow.EncapsulatedMessage", message())
}

pub fn arrow() -> Template {
    with_types(Template::new("arrow", file()))
}
