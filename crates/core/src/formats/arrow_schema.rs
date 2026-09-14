//! The FlatBuffers schema of an Arrow IPC file: every table and struct that
//! `Schema.fbs`, `Message.fbs` and `File.fbs` declare, with the enums their
//! fields name, transcribed into the form
//! [`flatbuf`](crate::formats::flatbuf) reads.
//!
//! Apart from [`arrow`](super::arrow) because it is Arrow's declarations
//! rather than this reader's decisions, and is checked against those files and
//! not against the template.

use crate::formats::flatbuf::{self, Field, Of, Scalar::*, Struct, Table, What as W};

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
pub(super) const TABLES: &[Table] = &[
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

pub(super) const STRUCTS: &[Struct] = &[
    Struct { name: "FieldNode", fields: &[("length", Long), ("null_count", Long)] },
    Struct { name: "Buffer", fields: &[("offset", Long), ("length", Long)] },
    Struct { name: "Block", fields: &[("offset", Long), ("metaDataLength", Int), ("bodyLength", Long)] },
];

pub(super) fn table(name: &str) -> &'static Table {
    TABLES.iter().find(|t| t.name == name).unwrap_or_else(|| panic!("no table {name}"))
}
