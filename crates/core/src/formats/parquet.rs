//! Parquet: a columnar table, read from the back.
//!
//! The file opens with `PAR1` and closes with it, and everything that says
//! where anything is sits in between the two. The last eight bytes are how
//! long the footer is and the magic again; the footer ends where those eight
//! bytes begin, so the whole structure is found by measuring backwards from
//! the end of the file. That is the point of the layout: a reader on a network
//! store fetches the last kilobyte, learns where every column chunk of every
//! row group is, and then fetches only the columns it was asked for.
//!
//! The footer is a Thrift compact-protocol `FileMetaData`, and it reads as
//! one: the schema, every row group, every column chunk in one, and the
//! statistics kept for each. See [`crate::formats::thrift`] for how a compact
//! struct is walked, and [`SCHEMA`] for what the numbered fields are called.
//! The names, the enum cases and the field ids below are transcribed from
//! `parquet.thrift` in apache/parquet-format, read on 2026-09-05.
//!
//! Everything between the opening magic and the footer is the row groups: the
//! pages of every column, each with its own header and its own encoding. The
//! footer is what places them, and two gathers read it that way (see
//! [`row_groups`]). The first walks to every row group's `columns` in the
//! footer and places a region for each row group, from where its column chunks
//! start to where they end. The second is inside that region
//! and walks to the column chunk entries of that one row group, placing each
//! column chunk at its `dictionary_page_offset` or `data_page_offset` and
//! sizing it by its `total_compressed_size`. A column chunk reads as pages: a
//! header in the same Thrift, read with [`PAGE_SCHEMA`], and the payload it
//! counts. So a page sits under its row group and column chunk, where it is in
//! the file, and asks the entry that placed its column chunk for the codec and
//! the physical type.
//!
//! The chunk's offset index, column index and bloom filter are not in any row
//! group's region. A writer puts them after the last row group, or a bloom
//! filter between one row group and the next, and each is placed with an `At`
//! under the column chunk entry in the footer that names it.
//!
//! A payload opens. The column chunk's `codec` says what was run over it, and
//! the payload is declared as a run packed that way, so what is inside a page
//! is a space of its own with the levels and the values laid over it. Snappy,
//! gzip, zstd, brotli and the raw LZ4 of `LZ4_RAW` all open; UNCOMPRESSED
//! opens too, as a stored space, so that one reading applies to every page
//! whether or not anything was packed. Two do not: LZO has no decoder here,
//! and `LZ4`, codec 5, is the Hadoop-framed one rather than a raw block and no
//! file to hand holds one, so naming it and keeping the bytes is the honest
//! answer rather than a guess at a framing.
//!
//! A `DATA_PAGE_V2` payload is not one run. Its repetition and definition
//! levels are written in front of the packed part and are never packed
//! themselves, sized by two lengths in the header, so that a reader can decide
//! whether it wants the page at all without unpacking anything.
//!
//! Inside the unpacked bytes are the values. A dictionary page holds nothing
//! else, so it reads as the column's physical type outright: four-byte
//! integers, doubles, byte arrays each behind their own length. A
//! `DATA_PAGE_V2` says in its header which encoding its values are in, and
//! PLAIN, the two dictionary encodings and RLE booleans all read. RLE
//! booleans are behind a four-byte length, which is the one place in a page
//! the hybrid carries one. The
//! dictionary encodings and RLE are the same thing underneath, the RLE and
//! bit-packed hybrid, and it reads as its runs: a varint header whose low bit
//! says which kind, then either one value repeated or a group of eight packed
//! at the width. A bit-packed group keeps its bytes, because Parquet packs
//! from the low bit of a byte upwards and bits are addressed here from the
//! high bit down, so a field per value would name the right byte and the wrong
//! bits inside it.
//!
//! A v1 data page keeps its bytes, and the reason is worth stating rather than
//! leaving to be discovered. Its levels are written only where the column's
//! maximum level is above zero, and that maximum is the count of OPTIONAL and
//! REPEATED elements along the column's path through the schema. Nothing in
//! the page says it and nothing an expression here can reach says it either:
//! the schema is a flat depth-first list in the footer, and which of its
//! elements this column is, is a walk rather than a lookup. So the first four
//! bytes of a v1 page are a level run's length or they are the first value,
//! and only something that can walk the schema can tell which. The delta
//! encodings and BYTE_STREAM_SPLIT keep their bytes for a plainer reason: a
//! delta block's values are packed at a width that changes every miniblock,
//! which is a decoder rather than a declaration.
//!
//! What the template leaves as bytes, [`parquet_page`](super::parquet_page)
//! reads. Every `Page` is marked as packed, so the inspector asks that reader
//! for the page under the cursor, and it walks the schema for the column's
//! levels, undoes the codec, and reports each step and then the values. Every
//! page of every sample in the collection reads that way, bar two brotli pages
//! that claim two gigabytes.
//!
//! A sequential walk of that region would be wrong, which is why it is not
//! here: a writer may put a column index, an offset index and a bloom filter
//! between the last row group and the footer, and none of those is a page.
//! What places a page honestly is the footer's own `data_page_offset` and
//! `dictionary_page_offset`, one per column chunk.
//!
//! Encryption may also leave the footer readable, and then the magic stays
//! `PAR1` and a 28-byte signature is written after the `FileMetaData`, inside
//! what the length counts. Nothing here names it, since no file to hand has
//! one: what a reader sees is 28 bytes of gap after the footer's stop byte,
//! and a footer that has one also has `footer_signing_key_metadata` set.
//!
//! A file whose footer is encrypted says `PARE` at both ends rather than
//! `PAR1`, and the magic at the front is what picks between the two shapes.
//! The schema, the row groups and every column chunk are ciphertext there, so
//! nothing in it says where anything is. One structure is still in the clear,
//! and it is the one a reader needs: a `FileCryptoMetaData` sits just before
//! the encrypted footer and names the algorithm and the key. The four bytes
//! at the end count that structure and the footer together.

use crate::codec::Codec;
use crate::formats::thrift::{self, Field, Struct, What::{Enum, Plain, Struct as Sub, Text}};
use crate::template::{Anchor, Endian::{Big, Little}, Expr as E, Step, Template, Ty as T, Until};
use std::sync::Arc;

/// What one of these opens with, and what an unencrypted one closes with.
pub const MAGIC: &[u8] = b"PAR1";

/// What a file whose footer is encrypted says at both ends instead. The
/// magic is the whole of how the two are told apart: a reader that sees this
/// knows to look for a `FileCryptoMetaData` before the footer, and one that
/// does not know the word stops rather than reading ciphertext as a schema.
pub const ENCRYPTED: &[u8] = b"PARE";

/// The physical type of a column: what the bytes of a value are, before any
/// reading of them as a date or a decimal.
const TYPE: &[(i128, &str)] = &[
    (0, "BOOLEAN"),
    (1, "INT32"),
    (2, "INT64"),
    (3, "INT96"),
    (4, "FLOAT"),
    (5, "DOUBLE"),
    (6, "BYTE_ARRAY"),
    (7, "FIXED_LEN_BYTE_ARRAY"),
];

/// The older way of saying what a physical type means, kept beside
/// `LogicalType` for readers that predate it.
const CONVERTED_TYPE: &[(i128, &str)] = &[
    (0, "UTF8"),
    (1, "MAP"),
    (2, "MAP_KEY_VALUE"),
    (3, "LIST"),
    (4, "ENUM"),
    (5, "DECIMAL"),
    (6, "DATE"),
    (7, "TIME_MILLIS"),
    (8, "TIME_MICROS"),
    (9, "TIMESTAMP_MILLIS"),
    (10, "TIMESTAMP_MICROS"),
    (11, "UINT_8"),
    (12, "UINT_16"),
    (13, "UINT_32"),
    (14, "UINT_64"),
    (15, "INT_8"),
    (16, "INT_16"),
    (17, "INT_32"),
    (18, "INT_64"),
    (19, "JSON"),
    (20, "BSON"),
    (21, "INTERVAL"),
];

const REPETITION: &[(i128, &str)] = &[(0, "REQUIRED"), (1, "OPTIONAL"), (2, "REPEATED")];

/// How a column's values are packed. There is no 1: the first dictionary
/// encoding was numbered 2 and the number was never reused.
const ENCODING: &[(i128, &str)] = &[
    (0, "PLAIN"),
    (2, "PLAIN_DICTIONARY"),
    (3, "RLE"),
    (4, "BIT_PACKED"),
    (5, "DELTA_BINARY_PACKED"),
    (6, "DELTA_LENGTH_BYTE_ARRAY"),
    (7, "DELTA_BYTE_ARRAY"),
    (8, "RLE_DICTIONARY"),
    (9, "BYTE_STREAM_SPLIT"),
    (10, "ALP"),
];

const CODEC: &[(i128, &str)] = &[
    (0, "UNCOMPRESSED"),
    (1, "SNAPPY"),
    (2, "GZIP"),
    (3, "LZO"),
    (4, "BROTLI"),
    (5, "LZ4"),
    (6, "ZSTD"),
    (7, "LZ4_RAW"),
];

/// Which of the two kinds a hybrid run is, from the low bit of its header.
/// The format's own two words, so a reader with the encodings page open finds
/// the same ones there.
const HYBRID_RUN: &[(i128, &str)] = &[(0, "RLE"), (1, "BIT_PACKED")];

const PAGE_TYPE: &[(i128, &str)] =
    &[(0, "DATA_PAGE"), (1, "INDEX_PAGE"), (2, "DICTIONARY_PAGE"), (3, "DATA_PAGE_V2")];

const BOUNDARY_ORDER: &[(i128, &str)] = &[(0, "UNORDERED"), (1, "ASCENDING"), (2, "DESCENDING")];

const EDGE_ALGORITHM: &[(i128, &str)] =
    &[(0, "SPHERICAL"), (1, "VINCENTY"), (2, "THOMAS"), (3, "ANDOYER"), (4, "KARNEY")];

/// A struct with no fields at all. Thrift writes one as a lone stop byte, and
/// a union uses a run of them as the labels of an enumeration it can extend
/// later: which member of `LogicalType` is set is the whole of what it says.
const fn empty(name: &'static str) -> Struct {
    Struct { name, fields: &[] }
}

const fn f(id: i128, name: &'static str, what: thrift::What) -> Field {
    Field { id, name, what }
}

/// Every struct the footer reaches, by field id.
///
/// A union is a struct whose one set field is the answer, so it is written
/// here as a struct like any other and reads as one.
pub const SCHEMA: &[Struct] = &[
    Struct {
        name: "FileMetaData",
        fields: &[
            f(1, "version", Plain),
            f(2, "schema", Sub("SchemaElement")),
            f(3, "num_rows", Plain),
            f(4, "row_groups", Sub("RowGroup")),
            f(5, "key_value_metadata", Sub("KeyValue")),
            f(6, "created_by", Text),
            f(7, "column_orders", Sub("ColumnOrder")),
            f(8, "encryption_algorithm", Sub("EncryptionAlgorithm")),
            f(9, "footer_signing_key_metadata", Plain),
        ],
    },
    Struct {
        name: "SchemaElement",
        fields: &[
            f(1, "type", Enum("Type", TYPE)),
            f(2, "type_length", Plain),
            f(3, "repetition_type", Enum("FieldRepetitionType", REPETITION)),
            f(4, "name", Text),
            f(5, "num_children", Plain),
            f(6, "converted_type", Enum("ConvertedType", CONVERTED_TYPE)),
            f(7, "scale", Plain),
            f(8, "precision", Plain),
            f(9, "field_id", Plain),
            f(10, "logicalType", Sub("LogicalType")),
        ],
    },
    Struct {
        name: "LogicalType",
        fields: &[
            f(1, "STRING", Sub("StringType")),
            f(2, "MAP", Sub("MapType")),
            f(3, "LIST", Sub("ListType")),
            f(4, "ENUM", Sub("EnumType")),
            f(5, "DECIMAL", Sub("DecimalType")),
            f(6, "DATE", Sub("DateType")),
            f(7, "TIME", Sub("TimeType")),
            f(8, "TIMESTAMP", Sub("TimestampType")),
            f(10, "INTEGER", Sub("IntType")),
            f(11, "UNKNOWN", Sub("NullType")),
            f(12, "JSON", Sub("JsonType")),
            f(13, "BSON", Sub("BsonType")),
            f(14, "UUID", Sub("UUIDType")),
            f(15, "FLOAT16", Sub("Float16Type")),
            f(16, "VARIANT", Sub("VariantType")),
            f(17, "GEOMETRY", Sub("GeometryType")),
            f(18, "GEOGRAPHY", Sub("GeographyType")),
            f(19, "FILE", Sub("FileType")),
        ],
    },
    empty("StringType"),
    empty("MapType"),
    empty("ListType"),
    empty("EnumType"),
    empty("DateType"),
    empty("NullType"),
    empty("JsonType"),
    empty("BsonType"),
    empty("UUIDType"),
    empty("Float16Type"),
    empty("FileType"),
    Struct { name: "DecimalType", fields: &[f(1, "scale", Plain), f(2, "precision", Plain)] },
    Struct {
        name: "TimeType",
        fields: &[f(1, "isAdjustedToUTC", Plain), f(2, "unit", Sub("TimeUnit"))],
    },
    Struct {
        name: "TimestampType",
        fields: &[f(1, "isAdjustedToUTC", Plain), f(2, "unit", Sub("TimeUnit"))],
    },
    Struct {
        name: "TimeUnit",
        fields: &[
            f(1, "MILLIS", Sub("MilliSeconds")),
            f(2, "MICROS", Sub("MicroSeconds")),
            f(3, "NANOS", Sub("NanoSeconds")),
        ],
    },
    empty("MilliSeconds"),
    empty("MicroSeconds"),
    empty("NanoSeconds"),
    Struct { name: "IntType", fields: &[f(1, "bitWidth", Plain), f(2, "isSigned", Plain)] },
    Struct { name: "VariantType", fields: &[f(1, "specification_version", Plain)] },
    Struct { name: "GeometryType", fields: &[f(1, "crs", Text)] },
    Struct {
        name: "GeographyType",
        fields: &[f(1, "crs", Text), f(2, "algorithm", Enum("EdgeInterpolationAlgorithm", EDGE_ALGORITHM))],
    },
    Struct {
        name: "RowGroup",
        fields: &[
            f(1, "columns", Sub("ColumnChunk")),
            f(2, "total_byte_size", Plain),
            f(3, "num_rows", Plain),
            f(4, "sorting_columns", Sub("SortingColumn")),
            f(5, "file_offset", Plain),
            f(6, "total_compressed_size", Plain),
            f(7, "ordinal", Plain),
        ],
    },
    Struct {
        name: "ColumnChunk",
        fields: &[
            f(1, "file_path", Text),
            f(2, "file_offset", Plain),
            f(3, "meta_data", Sub("ColumnMetaData")),
            f(4, "offset_index_offset", Plain),
            f(5, "offset_index_length", Plain),
            f(6, "column_index_offset", Plain),
            f(7, "column_index_length", Plain),
            f(8, "crypto_metadata", Sub("ColumnCryptoMetaData")),
            f(9, "encrypted_column_metadata", Plain),
        ],
    },
    Struct {
        name: "ColumnMetaData",
        fields: &[
            f(1, "type", Enum("Type", TYPE)),
            f(2, "encodings", Enum("Encoding", ENCODING)),
            f(3, "path_in_schema", Text),
            f(4, "codec", Enum("CompressionCodec", CODEC)),
            f(5, "num_values", Plain),
            f(6, "total_uncompressed_size", Plain),
            f(7, "total_compressed_size", Plain),
            f(8, "key_value_metadata", Sub("KeyValue")),
            f(9, "data_page_offset", Plain),
            f(10, "index_page_offset", Plain),
            f(11, "dictionary_page_offset", Plain),
            f(12, "statistics", Sub("Statistics")),
            f(13, "encoding_stats", Sub("PageEncodingStats")),
            f(14, "bloom_filter_offset", Plain),
            f(15, "bloom_filter_length", Plain),
            f(16, "size_statistics", Sub("SizeStatistics")),
            f(17, "geospatial_statistics", Sub("GeospatialStatistics")),
        ],
    },
    // The bounds are the column's own bytes, in the column's own physical
    // type, so they stay bytes: a min of `00 00 00 07` is the number seven in
    // an INT32 column and four spaces of a fixed-width string in another.
    Struct {
        name: "Statistics",
        fields: &[
            f(1, "max", Plain),
            f(2, "min", Plain),
            f(3, "null_count", Plain),
            f(4, "distinct_count", Plain),
            f(5, "max_value", Plain),
            f(6, "min_value", Plain),
            f(7, "is_max_value_exact", Plain),
            f(8, "is_min_value_exact", Plain),
            f(9, "nan_count", Plain),
        ],
    },
    Struct {
        name: "SizeStatistics",
        fields: &[
            f(1, "unencoded_byte_array_data_bytes", Plain),
            f(2, "repetition_level_histogram", Plain),
            f(3, "definition_level_histogram", Plain),
        ],
    },
    Struct {
        name: "GeospatialStatistics",
        fields: &[f(1, "bbox", Sub("BoundingBox")), f(2, "geospatial_types", Plain)],
    },
    Struct {
        name: "BoundingBox",
        fields: &[
            f(1, "xmin", Plain),
            f(2, "xmax", Plain),
            f(3, "ymin", Plain),
            f(4, "ymax", Plain),
            f(5, "zmin", Plain),
            f(6, "zmax", Plain),
            f(7, "mmin", Plain),
            f(8, "mmax", Plain),
        ],
    },
    Struct {
        name: "PageEncodingStats",
        fields: &[
            f(1, "page_type", Enum("PageType", PAGE_TYPE)),
            f(2, "encoding", Enum("Encoding", ENCODING)),
            f(3, "count", Plain),
        ],
    },
    Struct { name: "KeyValue", fields: &[f(1, "key", Text), f(2, "value", Text)] },
    Struct {
        name: "SortingColumn",
        fields: &[f(1, "column_idx", Plain), f(2, "descending", Plain), f(3, "nulls_first", Plain)],
    },
    Struct {
        name: "ColumnOrder",
        fields: &[
            f(1, "TYPE_ORDER", Sub("TypeDefinedOrder")),
            f(2, "IEEE_754_TOTAL_ORDER", Sub("IEEE754TotalOrder")),
            f(3, "INT96_TIMESTAMP_ORDER", Sub("Int96TimestampOrder")),
        ],
    },
    empty("TypeDefinedOrder"),
    empty("IEEE754TotalOrder"),
    empty("Int96TimestampOrder"),
    Struct {
        name: "ColumnCryptoMetaData",
        fields: &[
            f(1, "ENCRYPTION_WITH_FOOTER_KEY", Sub("EncryptionWithFooterKey")),
            f(2, "ENCRYPTION_WITH_COLUMN_KEY", Sub("EncryptionWithColumnKey")),
        ],
    },
    empty("EncryptionWithFooterKey"),
    Struct {
        name: "EncryptionWithColumnKey",
        fields: &[f(1, "path_in_schema", Text), f(2, "key_metadata", Plain)],
    },
    Struct {
        name: "EncryptionAlgorithm",
        fields: &[f(1, "AES_GCM_V1", Sub("AesGcmV1")), f(2, "AES_GCM_CTR_V1", Sub("AesGcmCtrV1"))],
    },
    Struct {
        name: "AesGcmV1",
        fields: &[f(1, "aad_prefix", Plain), f(2, "aad_file_unique", Plain), f(3, "supply_aad_prefix", Plain)],
    },
    Struct {
        name: "AesGcmCtrV1",
        fields: &[f(1, "aad_prefix", Plain), f(2, "aad_file_unique", Plain), f(3, "supply_aad_prefix", Plain)],
    },
    // The one structure of an encrypted file that is not encrypted, and the
    // only reason such a file says anything at all. See [`encrypted`].
    Struct {
        name: "FileCryptoMetaData",
        fields: &[f(1, "encryption_algorithm", Sub("EncryptionAlgorithm")), f(2, "key_metadata", Plain)],
    },
];

/// The structs a page header is made of, and the offset index, column index
/// and bloom filter header a column chunk points at beside its pages. The same
/// schema and the same reader as the footer's.
pub const PAGE_SCHEMA: &[Struct] = &[
    Struct { name: "BloomFilterHeader", fields: &[
        f(1, "numBytes", Plain),
        f(2, "algorithm", Sub("BloomFilterAlgorithm")),
        f(3, "hash", Sub("BloomFilterHash")),
        f(4, "compression", Sub("BloomFilterCompression")),
    ] },
    Struct { name: "BloomFilterAlgorithm", fields: &[f(1, "BLOCK", Sub("SplitBlockAlgorithm"))] },
    Struct { name: "BloomFilterHash", fields: &[f(1, "XXHASH", Sub("XxHash"))] },
    Struct { name: "BloomFilterCompression", fields: &[f(1, "UNCOMPRESSED", Sub("Uncompressed"))] },
    empty("SplitBlockAlgorithm"),
    empty("XxHash"),
    empty("Uncompressed"),
    Struct {
        name: "PageHeader",
        fields: &[
            f(1, "type", Enum("PageType", PAGE_TYPE)),
            f(2, "uncompressed_page_size", Plain),
            f(3, "compressed_page_size", Plain),
            f(4, "crc", Plain),
            f(5, "data_page_header", Sub("DataPageHeader")),
            f(6, "index_page_header", Sub("IndexPageHeader")),
            f(7, "dictionary_page_header", Sub("DictionaryPageHeader")),
            f(8, "data_page_header_v2", Sub("DataPageHeaderV2")),
        ],
    },
    Struct {
        name: "DataPageHeader",
        fields: &[
            f(1, "num_values", Plain),
            f(2, "encoding", Enum("Encoding", ENCODING)),
            f(3, "definition_level_encoding", Enum("Encoding", ENCODING)),
            f(4, "repetition_level_encoding", Enum("Encoding", ENCODING)),
            f(5, "statistics", Sub("Statistics")),
        ],
    },
    empty("IndexPageHeader"),
    Struct {
        name: "DictionaryPageHeader",
        fields: &[
            f(1, "num_values", Plain),
            f(2, "encoding", Enum("Encoding", ENCODING)),
            f(3, "is_sorted", Plain),
        ],
    },
    Struct {
        name: "DataPageHeaderV2",
        fields: &[
            f(1, "num_values", Plain),
            f(2, "num_nulls", Plain),
            f(3, "num_rows", Plain),
            f(4, "encoding", Enum("Encoding", ENCODING)),
            f(5, "definition_levels_byte_length", Plain),
            f(6, "repetition_levels_byte_length", Plain),
            f(7, "is_compressed", Plain),
            f(8, "statistics", Sub("Statistics")),
        ],
    },
    Struct {
        name: "OffsetIndex",
        fields: &[f(1, "page_locations", Sub("PageLocation")), f(2, "unencoded_byte_array_data_bytes", Plain)],
    },
    Struct {
        name: "PageLocation",
        fields: &[f(1, "offset", Plain), f(2, "compressed_page_size", Plain), f(3, "first_row_index", Plain)],
    },
    Struct {
        name: "ColumnIndex",
        fields: &[
            f(1, "null_pages", Plain),
            f(2, "min_values", Plain),
            f(3, "max_values", Plain),
            f(4, "boundary_order", Enum("BoundaryOrder", BOUNDARY_ORDER)),
            f(5, "null_counts", Plain),
            f(6, "repetition_level_histograms", Plain),
            f(7, "definition_level_histograms", Plain),
            f(8, "nan_counts", Plain),
        ],
    },
];

/// The four bytes at the very front, as a number, without reading them. What
/// says which of the two shapes this file is.
fn leading_magic() -> E {
    E::peek(32, Big)
}

/// How long the footer is: the four bytes before the closing magic, which is
/// the only length in the file that is not itself in the footer.
fn footer_length() -> E {
    E::peek_at(E::lit(-64), 32, Little)
}

/// A numbered field of the current ColumnChunk, or of its nested metadata.
fn chunk_field(id: i128) -> E {
    E::tagged("fields", &["id"], id, &["value"])
}

fn metadata_field(id: i128) -> E {
    metadata_in(E::field("fields"), id)
}

/// A numbered field of the `ColumnMetaData` of the column chunk entry whose
/// list of fields `chunk` lands on, which need not be the entry asking.
fn metadata_in(chunk: E, id: i128) -> E {
    E::tagged_in(E::tagged_in(chunk, &["id"], 3, &["value", "fields"]), &["id"], id, &["value"])
}

fn header_field(id: i128) -> E {
    E::tagged_in(E::within(&["header", "fields"]), &["id"], id, &["value"])
}

/// A numbered field of the `DataPageHeaderV2` nested inside this page's
/// header, which is field 8 of the header and has numbered fields of its own.
///
/// A field nothing wrote reads as zero, and a boolean reads as 1 for true and
/// 2 for false, which is how the compact protocol writes one: the value is in
/// the type nibble and there is no body. So a switch on a boolean has three
/// cases to tell apart and not two, and absent is not the same as false.
fn v2_field(id: i128) -> E {
    let header = E::tagged_in(E::within(&["header", "fields"]), &["id"], 8, &["value", "fields"]);
    E::tagged_in(header, &["id"], id, &["value"])
}

/// What the column chunk said packed its pages. A page sits in a column chunk
/// in the file, and the codec is a field of the `ColumnMetaData` in the footer
/// entry that placed that column chunk, so it is asked of that entry.
fn codec_field() -> E {
    E::placer(metadata_field(4))
}

fn page() -> T {
    T::structure_named("Page", "type", "payload", vec![
        ("header", T::Named("parquet.PageHeader".into())),
        ("type", T::enumeration("PageType", T::computed(header_field(1)), PAGE_TYPE)),
        ("payload", payload()),
    ])
    // A page reads further than a template can take it: the levels of a v1
    // page, the delta encodings, and the byte stream split all need the
    // schema or a decoder. See [`super::parquet_page`].
    .packed_as(super::parquet_page::PACKING)
}

/// What a page's payload holds, which is not the same shape for every kind of
/// page.
///
/// A v1 data page and a dictionary page are one compressed run from the first
/// byte to the last. A `DATA_PAGE_V2` is not: its repetition and definition
/// levels are written in front of the compressed part and are never packed, so
/// that a reader deciding whether it wants a page at all can read its levels
/// without unpacking anything. The two lengths are in the header, which is the
/// other half of the same idea, and what is left after them is the run the
/// codec was run over.
fn payload() -> T {
    T::switch(header_field(1), vec![(3, payload_v2())], packed(header_field(3)))
}

/// A `DATA_PAGE_V2` payload: levels in the clear, then the values.
///
/// `is_compressed` is the writer's one way of saying that the codec was not
/// run over this page after all, which parquet-mr does when packing made the
/// page larger. It defaults to true, so a page that says nothing is packed.
fn payload_v2() -> T {
    let rep = v2_field(6);
    let def = v2_field(5);
    let values = header_field(3).sub(rep.clone()).sub(def.clone()).at_least(E::lit(0));
    T::structure("DataPageV2Payload", vec![
        ("repetition_levels", T::bytes(rep)),
        ("definition_levels", T::bytes(def)),
        ("values", T::switch(v2_field(7), vec![(2, stored(values.clone()))], packed(values))),
    ])
}

/// A compressed run of `len` bytes, opened by whatever the column chunk's
/// codec names.
///
/// Which codecs open and which do not is the part of this worth writing down.
/// UNCOMPRESSED opens as a stored space rather than staying plain bytes so
/// that one reader applies to every page: what is inside a page is the same
/// arrangement of levels and values whether or not anything was run over it,
/// and a space is where that arrangement is declared.
///
/// LZO and the older framed LZ4 are named and keep their bytes. LZO has no
/// decoder here, and its licence is most of why little else has one either.
/// LZ4 as codec 5 is not a raw LZ4 block: Hadoop wrapped each block in a pair
/// of big-endian lengths, writers disagreed for years about how a page with
/// more than one block in it was written, and no file in the sample collection
/// holds one, so what would go here would be a reading nothing has checked.
/// LZ4_RAW, codec 7, is the raw block and does open; it is what every writer
/// since 2020 produces and what the sample named for it holds.
fn packed(len: E) -> T {
    let pack = |codec| T::decoded(len.clone(), codec, page_values());
    T::switch(codec_field(), vec![
        (0, stored(len.clone())),
        (1, pack(Codec::Snappy)),
        (2, pack(Codec::Gzip)),
        (4, pack(Codec::Brotli)),
        (6, pack(Codec::Zstd)),
        (7, pack(Codec::Lz4Block)),
    ], T::bytes(len))
}

/// A run nothing was run over, opened as a space all the same. See
/// [`Codec::Stored`](crate::codec::Codec::Stored) for why that is a codec here
/// rather than a flag.
fn stored(len: E) -> T {
    T::decoded(len, Codec::Stored, page_values())
}

/// The column's physical type, as the number the footer wrote. Field 1 of the
/// `ColumnMetaData`, which is where a page finds out how wide one value is
/// without going anywhere near the schema. Asked of the entry that placed the
/// column chunk, as the codec is.
fn type_field() -> E {
    E::placer(metadata_field(1))
}

/// What the unpacked bytes of a page hold.
///
/// Two of the three kinds of page read as their values here, and the third
/// does not, for a reason worth stating plainly. A dictionary page is nothing
/// but values, so it reads. A `DATA_PAGE_V2` says in its header how many bytes
/// of levels it wrote, so what is left is the values and they read too. A v1
/// data page says neither: its repetition and definition levels are there only
/// when the column's maximum level is above zero, and that maximum comes from
/// the repetition types of every schema element along the column's path, which
/// is not a thing an expression here can reach. Four bytes at the front of a v1
/// page are a level run's length or they are the first value, and nothing in
/// the page tells the two apart. So a v1 page keeps its bytes and the side
/// reader, which can walk the schema, is what says what they hold.
fn page_values() -> T {
    T::switch(header_field(1), vec![(2, plain_values()), (3, v2_values())], T::bytes(E::Remaining))
}

/// The values of a `DATA_PAGE_V2`, by the encoding its header names.
///
/// `PLAIN_DICTIONARY` and `RLE_DICTIONARY` are the same bytes under two
/// numbers: the first was what parquet-mr wrote before the second was given a
/// number of its own, and a reader has to take both.
fn v2_values() -> T {
    T::switch(v2_field(4), vec![
        (0, plain_values()),
        (2, dictionary_indices()),
        (8, dictionary_indices()),
        (3, rle_booleans()),
    ], T::bytes(E::Remaining))
}

/// PLAIN: values back to back, in the column's physical type.
///
/// Every number is little-endian, which is the one thing Parquet fixes
/// everywhere. A byte array carries its own four-byte length; a fixed-length
/// one does not, and its width is in the schema rather than in the footer, so
/// it stays bytes here. So does a boolean, which PLAIN packs a bit at a time
/// from the low bit of each byte up: that is the opposite of how bits are
/// addressed here, and a field laid over them would point at the wrong bit of
/// the right byte.
fn plain_values() -> T {
    let rep = |ty| T::repeat(ty, Until::End);
    T::switch(type_field(), vec![
        (1, rep(T::i32(Little))),
        (2, rep(T::Int { bits: 64, endian: Little })),
        // INT96: twelve bytes Impala wrote a nanosecond timestamp into, as
        // three little-endian words. Deprecated since 2016 and still in every
        // file Impala ever wrote.
        (3, rep(T::bytes(E::lit(12)))),
        (4, rep(T::F32(Little))),
        (5, rep(T::F64(Little))),
        (6, rep(byte_array())),
    ], T::bytes(E::Remaining))
}

/// One PLAIN byte array: four bytes of length and then that many bytes.
fn byte_array() -> T {
    T::structure_named("ByteArray", "", "bytes", vec![
        ("length", T::u32(Little)),
        ("bytes", T::bytes(E::field("length"))),
    ])
}

/// RLE booleans: four bytes of length, and then the hybrid at one bit a value.
///
/// There is no width byte, because a boolean is one bit. There is a length,
/// in both page versions, and that is the odd one out: a v2 page's levels are
/// sized by the header and carry none, and the same runs holding dictionary
/// indices carry none either. The values are the one place it is written.
fn rle_booleans() -> T {
    T::structure("RleBooleans", vec![
        ("length", T::u32(Little)),
        ("runs", T::sized(E::field("length"), hybrid_runs(E::lit(1)))),
    ])
}

/// Dictionary indices: a byte saying how wide an index is, and then the
/// hybrid runs that hold them.
///
/// The width byte is part of the encoding rather than part of the hybrid: the
/// same runs appear for levels and for RLE booleans with the width coming from
/// somewhere else entirely.
fn dictionary_indices() -> T {
    T::structure("DictionaryIndices", vec![
        ("bit_width", T::u8()),
        ("runs", hybrid_runs(E::field("bit_width"))),
    ])
}

/// The RLE and bit-packed hybrid, at a width something outside it fixed.
fn hybrid_runs(width: E) -> T {
    T::repeat(hybrid_run(width), Until::End)
}

/// One run of the hybrid.
///
/// Every run starts with an unsigned varint whose low bit says which of the
/// two kinds this is; the rest of it is a count. An even header repeats a
/// single value, written in as many whole bytes as the width needs, and the
/// count is how many times. An odd header introduces groups of eight values
/// packed at the width, and the count is how many groups, so eight times the
/// count values follow in `count * width` bytes.
///
/// The packed values stay bytes rather than becoming fields. Parquet packs
/// them from the low bit of each byte upwards, which is the reverse of how
/// bits are addressed here, so a field per value would name the right byte and
/// the wrong bits of it. The side reader unpacks them and says what they are.
fn hybrid_run(width: E) -> T {
    let header = E::field("header");
    let count = header.clone().shr(E::lit(1));
    let rle = T::structure("RleRun", vec![
        ("count", T::computed(count.clone())),
        // A width of zero writes no bytes at all: every index is the same
        // index, and a column with one distinct value has nothing to say.
        ("value", T::switch(width.clone().equals(E::lit(0)), vec![(1, T::computed(E::lit(0)))],
            T::uint_expr(width.clone().div_ceil(E::lit(8)).mul(E::lit(8)), Little))),
    ]);
    let packed = T::structure("BitPackedRun", vec![
        ("count", T::computed(count.clone().mul(E::lit(8)))),
        ("values", T::bytes(count.mul(width))),
    ]);
    T::structure_named("HybridRun", "kind", "", vec![
        ("header", T::leb_u()),
        ("kind", T::enumeration("HybridRunKind", T::computed(header.and(E::lit(1))), HYBRID_RUN)),
        ("run", T::switch(E::field("kind"), vec![(1, packed)], rle)),
    ])
}

/// A pointer with a declared length. Never let its window include the footer.
/// The structure around a page repeat also prevents truncated-page recovery
/// from borrowing bytes beyond this explicitly sized column chunk.
fn pointed(at: E, length: E, inner: T) -> T {
    T::switch(at.clone().less_than(E::lit(4)), vec![(1, T::bytes(E::lit(0)))],
        T::at(at, T::sized(length.at_most(E::Remaining.sub(E::lit(8)).sub(footer_length()).at_least(E::lit(0))), inner)))
}

/// What a column chunk entry in the footer places outside the row groups: its
/// offset index, column index and bloom filter. Its pages are in its row
/// group's region, placed by [`row_groups`].
fn column_data() -> T {
    let bloom = T::structure("BloomFilter", vec![
        ("header", T::Named("parquet.BloomFilterHeader".into())),
        ("bitset", T::bytes(header_field(1))),
    ]);
    let refs = T::structure("ColumnData", vec![
        ("offset_index", pointed(chunk_field(4), chunk_field(5), T::Named("parquet.OffsetIndex".into()))),
        ("column_index", pointed(chunk_field(6), chunk_field(7), T::Named("parquet.ColumnIndex".into()))),
        // Older writers omit bloom_filter_length. Its own header still gives
        // the bitset size, so the file boundary is the fallback window.
        ("bloom_filter", pointed(metadata_field(14), metadata_field(15).or(E::Remaining), bloom)),
    ]);
    // A summary file points into other files. Encrypted column headers are
    // not compact Thrift. Keep their metadata without interpreting local bytes
    // as the pages of either kind of column.
    T::switch(elsewhere(E::field("fields")), vec![(0, refs)], T::bytes(E::lit(0)))
}

/// Nonzero when the column chunk entry whose fields `chunk` lands on keeps its
/// pages in another file (`file_path`, as a summary file does) or encrypts
/// them (`crypto_metadata`). Neither kind is read as pages here.
fn elsewhere(chunk: E) -> E {
    let external = E::tagged_in(chunk.clone(), &["id"], 1, &["id"]);
    let encrypted = E::tagged_in(chunk, &["id"], 8, &["id"]);
    external.or(encrypted)
}

/// Where the first page of a column chunk is, from its entry in the footer:
/// the dictionary page when it has one, which comes before the data pages,
/// and otherwise the first data page. Zero, which places nothing, for an entry
/// whose pages are not read here and for an offset inside the opening magic.
fn chunk_start(chunk: E) -> E {
    let data = metadata_in(chunk.clone(), 9);
    let first = metadata_in(chunk.clone(), 11).or(data.clone()).at_most(data);
    E::cond(elsewhere(chunk).or(first.clone().less_than(E::lit(4))), E::lit(0), first)
}

/// Where a column chunk ends, from its entry in the footer. Zero where it has
/// no start.
fn chunk_end(chunk: E) -> E {
    let start = chunk_start(chunk.clone());
    E::cond(start.clone(), start.add(metadata_in(chunk, 7)), E::lit(0))
}

/// The lower of two starts, where a start of zero is no start at all.
fn earliest(a: E, b: E) -> E {
    a.clone().or(b.clone()).at_most(b.or(a))
}

/// Every row group, each a region of the file holding its column chunks.
///
/// Two gathers, one inside the other. This one walks to each row group's
/// `columns` field in the footer and places a region from where its column
/// chunks start to where they end. A writer puts a row group's column chunks
/// one after another in the order it lists them, so the first and the last
/// listed are the two ends; both ends are asked of both, which also reads a
/// list written back to front. The region stops short of the footer whatever
/// the entries say.
///
/// Inside it, the second gather starts from the same `columns` field and walks
/// to each column chunk entry in it, placing that column chunk where its pages
/// start, sized by `total_compressed_size`. A chunk that starts outside its
/// row group's region is passed over, and one that runs past it is cut off
/// there, so no page reads bytes that belong to another row group or to the
/// indexes after them.
fn row_groups() -> T {
    let first = E::within(&["value", "elems", "0", "fields"]);
    let last = E::elem_within(&["value", "elems"], E::within(&["value", "count"]).sub(E::lit(1)), &["fields"]);
    let start = earliest(chunk_start(first.clone()), chunk_start(last.clone()));
    let end = chunk_end(first).at_least(chunk_end(last));
    let before_footer = E::Remaining.sub(E::lit(8)).sub(footer_length()).at_least(E::lit(0));
    let span = E::placer(end.sub(start.clone()));
    let chunks = vec![Step::placer(), Step::field("value"), Step::field("elems"), Step::each()];
    let columns = T::gather(chunks, chunk_start(E::field("fields")), Anchor::File, E::lit(0), column_chunk()).skipping_zero();
    let row_group = T::structure("RowGroup", vec![("columns", T::sized(E::Remaining, columns))]);
    let entries = vec![
        Step::field("footer"),
        Step::field("fields"),
        Step::tagged(&["id"], 4, "row_groups"),
        Step::field("value"),
        Step::field("elems"),
        Step::each(),
        Step::field("fields"),
        Step::tagged(&["id"], 1, "columns"),
    ];
    let region = T::sized(span.at_most(before_footer).at_least(E::lit(0)), row_group);
    T::gather(entries, start, Anchor::File, E::lit(0), region).skipping_zero()
}

/// One column chunk in the file: its pages, as many as its entry's
/// `total_compressed_size` holds, and never past the row group it is in.
///
/// A structure round the pages rather than the pages alone, so that a page
/// whose header claims more than is left reads as far as the chunk goes and
/// no further.
fn column_chunk() -> T {
    let size = E::placer(metadata_field(7)).at_most(E::Remaining).at_least(E::lit(0));
    T::sized(size, T::structure("ColumnChunk", vec![("pages", T::repeat(page(), Until::End))]))
}

/// The bytes of a plain file: the row groups, the footer, and the trailer that
/// found the footer.
///
/// Both lengths are floored at nothing. A file cut off in the middle, or one
/// whose footer length is larger than the file, would otherwise ask for a run
/// of bytes measured backwards past where it started, and refusing to place
/// the bytes that are there would hide the very thing that went wrong.
fn plain() -> T {
    T::structure(
        "Parquet",
        vec![
            ("magic", T::magic(MAGIC)),
            // The Thrift `FileMetaData`, which runs from here to the eight
            // bytes that measured it. Sized by what is left rather than by the
            // length field, so a length larger than the file still places the
            // bytes that are there.
            (
                "footer",
                T::at(E::Remaining.sub(E::lit(8)).sub(footer_length()).at_least(E::lit(0)).add(E::lit(4)),
                    T::sized(E::Remaining.sub(E::lit(8)).at_least(E::lit(0)), T::Named("parquet.FileMetaData".into()))),
            ),
            ("footer_length", T::at(E::Remaining.sub(E::lit(4)), T::u32(Little))),
            ("footer_magic", T::at(E::Remaining, T::magic(MAGIC))),
            // After the footer, since the walks start from it, though the row
            // groups come first in the file.
            ("row_groups", row_groups()),
        ],
    )
}

/// A file whose footer is encrypted. The schema, the row groups and every
/// column chunk are ciphertext, so nothing says where anything is.
///
/// One thing is still in the clear, and it is the one thing a reader needs:
/// the `FileCryptoMetaData` written just before the encrypted footer says
/// which algorithm was used and carries whatever the writer put there to
/// identify the key. It ends at its own stop byte, so what follows it runs
/// from there to the length.
///
/// The four bytes at the end count that structure and the encrypted footer
/// together, which is why neither is measured from it: the first is as long as
/// Thrift says and the second is the rest.
fn encrypted() -> T {
    T::structure(
        "ParquetEncrypted",
        vec![
            ("magic", T::magic(ENCRYPTED)),
            ("modules", T::bytes(E::Remaining.sub(E::lit(8)).sub(footer_length()).at_least(E::lit(0)))),
            ("crypto_metadata", T::Named("parquet.FileCryptoMetaData".into())),
            ("footer", T::bytes(E::Remaining.sub(E::lit(8)).at_least(E::lit(0)))),
            ("footer_length", T::u32(Little)),
            ("footer_magic", T::magic(ENCRYPTED)),
        ],
    )
}

pub fn parquet() -> Template {
    let root = T::switch(leading_magic(), vec![(0x5041_5245, encrypted())], plain());
    let mut t = Template::new("parquet", root);
    let mut structs: Vec<&Struct> = SCHEMA.iter().collect();
    structs.extend(PAGE_SCHEMA.iter());
    let owned: Vec<Struct> = structs.into_iter().map(|s| Struct { name: s.name, fields: s.fields }).collect();
    for (name, mut ty) in thrift::types("parquet", &owned) {
        if name == "parquet.ColumnChunk" {
            let T::Struct(ref mut definition) = ty else { unreachable!() };
            let T::Struct(additions) = T::structure("", vec![("data", column_data())]) else { unreachable!() };
            let definition = Arc::make_mut(definition);
            definition.fields.extend(additions.fields.iter().cloned());
            definition.contents = None;
        }
        t = t.with_type(&name, ty);
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, eval::{Evaluator, Value}, source::MemSource};

    /// A file of `rows` bytes of row group and `footer` bytes of footer,
    /// opened and closed with `end`.
    fn file(rows: usize, footer: usize, end: &[u8]) -> Vec<u8> {
        let mut v = end.to_vec();
        v.extend(std::iter::repeat(0xAB).take(rows));
        v.extend(std::iter::repeat(0x15).take(footer));
        v.extend_from_slice(&(footer as u32).to_le_bytes());
        v.extend_from_slice(end);
        v
    }

    #[test]
    fn the_footer_is_measured_back_from_the_end() {
        let d = Document::new(MemSource(file(40, 12, MAGIC)));
        let mut e = Evaluator::new(parquet());
        assert_eq!(e.node(&d, &[1, 0]).unwrap().offset_bits, 44 * 8);
        assert_eq!(e.node(&d, &[1, 0]).unwrap().size_bits, 12 * 8);
        assert_eq!(e.node(&d, &[2, 0]).unwrap().value, Value::UInt(12));
        assert_eq!(e.node(&d, &[3, 0]).unwrap().offset_bits, d.len_bits() - 32);
    }

    #[test]
    fn a_file_with_no_row_groups_still_reads() {
        let d = Document::new(MemSource(file(0, 4, MAGIC)));
        let mut e = Evaluator::new(parquet());
        assert_eq!(e.node(&d, &[1, 0]).unwrap().offset_bits, 4 * 8);
        assert_eq!(e.node(&d, &[1, 0]).unwrap().size_bits, 4 * 8);
    }

    #[test]
    fn a_footer_longer_than_the_file_places_what_is_there() {
        let mut bytes = file(8, 4, MAGIC);
        let n = bytes.len();
        bytes[n - 8..n - 4].copy_from_slice(&0xFFFF_u32.to_le_bytes());
        let d = Document::new(MemSource(bytes));
        let mut e = Evaluator::new(parquet());
        assert_eq!(e.node(&d, &[1, 0]).unwrap().offset_bits, 4 * 8);
        assert_eq!(e.node(&d, &[1, 0]).unwrap().size_bits, 12 * 8);
    }

    /// A footer holding the fields `bytes` says, wrapped in what finds it.
    fn footed(fields: &[u8]) -> Vec<u8> {
        let mut v = MAGIC.to_vec();
        v.extend_from_slice(fields);
        v.extend_from_slice(&(fields.len() as u32).to_le_bytes());
        v.extend_from_slice(MAGIC);
        v
    }

    fn varint(mut n: u64) -> Vec<u8> {
        let mut out = Vec::new();
        while n >= 128 { out.push(n as u8 | 128); n >>= 7; }
        out.push(n as u8);
        out
    }

    // Explicit ids (delta zero) let every level be permuted independently.
    fn field(id: u64, kind: u8, body: Vec<u8>) -> Vec<u8> {
        let mut out = vec![kind];
        out.extend(varint(id * 2)); out.extend(body); out
    }

    fn integer(id: u64, n: u64) -> Vec<u8> { field(id, 6, varint(n * 2)) }

    fn object(fields: Vec<Vec<u8>>, reverse: bool) -> Vec<u8> {
        let mut fields = fields;
        if reverse { fields.reverse(); }
        let mut out: Vec<u8> = fields.into_iter().flatten().collect();
        out.push(0); out
    }

    fn one(id: u64, element: Vec<u8>) -> Vec<u8> {
        let mut list = vec![0x1c]; list.extend(element); field(id, 9, list)
    }

    fn fixture(reverse: bool, external: bool, encrypted: bool, oversized: bool) -> (Vec<u8>, u64) {
        let page = |kind, oversized| {
            let mut bytes = object(vec![integer(1, kind), integer(2, 2), integer(3, if oversized { 1000 } else { 2 })], reverse);
            bytes.extend([0xab, 0xcd]); bytes
        };
        let dict = page(2, oversized);
        let data = page(0, false);
        let size = dict.len() + data.len();
        let metadata = object(vec![integer(9, 4 + dict.len() as u64), integer(11, 4), integer(7, size as u64)], reverse);
        let mut fields = vec![integer(2, 0), field(3, 12, metadata)];
        if external { fields.push(field(1, 8, vec![1, b'x'])); }
        if encrypted { fields.push(field(8, 12, vec![0])); }
        let column = object(fields, reverse);
        let row = object(vec![one(1, column), integer(3, 1)], reverse);
        let footer = object(vec![integer(1, 1), one(4, row), integer(3, 1)], reverse);
        let mut bytes = MAGIC.to_vec();
        bytes.extend(dict); bytes.extend(data);
        let end = bytes.len() as u64;
        // Not page headers: bytes outside the declared chunk must stay a gap.
        bytes.extend([0xff; 16]);
        bytes.extend_from_slice(&footer);
        bytes.extend_from_slice(&(footer.len() as u32).to_le_bytes());
        bytes.extend_from_slice(MAGIC);
        (bytes, end)
    }

    fn nodes_of(e: &mut Evaluator, d: &Document<MemSource>, ty: &str) -> Vec<(Vec<usize>, crate::eval::NodeInfo)> {
        let mut stack = vec![Vec::new()];
        let mut out = Vec::new();
        while let Some(path) = stack.pop() {
            let n = e.node(d, &path).unwrap();
            for i in (0..n.child_count as usize).rev() {
                let mut p = path.clone(); p.push(i); stack.push(p);
            }
            if n.type_name == ty { out.push((path, n)); }
        }
        out
    }

    #[test]
    fn reordered_footer_column_and_page_fields_keep_their_meaning() {
        for reverse in [false, true] {
            let (bytes, end) = fixture(reverse, false, false, false);
            let d = Document::new(MemSource(bytes));
            let mut e = Evaluator::new(parquet());
            let pages = nodes_of(&mut e, &d, "Page");
            assert_eq!(pages.len(), 2);
            assert_eq!(pages[0].1.offset_bits, 32);
            assert_eq!(pages[1].1.offset_bits + pages[1].1.size_bits, end * 8);
            for (path, page) in &pages {
                assert!(e.locate(&d, page.offset_bits).unwrap().starts_with(path));
            }
            let gap = e.spans(&d, end * 8, (end + 16) * 8, 8).unwrap();
            assert_eq!(gap.len(), 1);
            assert!(gap[0].gap);
            assert_eq!(gap[0].size_bits, 16 * 8);
        }
    }

    #[test]
    fn external_and_encrypted_columns_do_not_parse_local_pages() {
        for (external, encrypted) in [(true, false), (false, true)] {
            let (bytes, _) = fixture(true, external, encrypted, false);
            let d = Document::new(MemSource(bytes));
            let mut e = Evaluator::new(parquet());
            assert!(nodes_of(&mut e, &d, "Page").is_empty());
            assert!(e.spans(&d, 32, 40, 8).unwrap()[0].gap);
        }
    }

    #[test]
    fn a_page_cannot_borrow_bytes_past_its_column() {
        let (bytes, end) = fixture(true, false, false, true);
        let d = Document::new(MemSource(bytes));
        let mut e = Evaluator::new(parquet());
        assert!(e.locate(&d, 32).is_err());
        let spans = e.spans(&d, end * 8, (end + 16) * 8, 8).unwrap();
        assert!(spans.iter().all(|s| s.gap));
    }

    #[test]
    fn footer_directed_lookup_resumes_after_yielding() {
        let (bytes, _) = fixture(true, false, false, false);
        let d = Document::new(MemSource(bytes));
        let mut e = Evaluator::new(parquet());
        e.set_slice(Some(8));
        for _ in 0..1000 {
            e.begin_slice();
            match e.spans(&d, 32, 48, 16) {
                Ok(spans) => { assert!(!spans.is_empty()); assert!(!spans[0].gap); return; }
                Err(crate::eval::EvalError::Busy { .. }) => {}
                Err(e) => panic!("{e:?}"),
            }
        }
        panic!("page lookup failed to make progress");
    }

    /// A list of structs, however many.
    fn many(id: u64, elements: Vec<Vec<u8>>) -> Vec<u8> {
        let mut list = vec![((elements.len() as u8) << 4) | 12];
        list.extend(elements.into_iter().flatten());
        field(id, 9, list)
    }

    /// Two row groups of two column chunks, each chunk one uncompressed data
    /// page of two bytes, and eight bytes nothing names between the row
    /// groups, where a writer may put a bloom filter. The column chunks of
    /// the second row group are listed in the footer in the opposite order to
    /// the file. Returns the bytes and where each page starts.
    fn two_row_groups() -> (Vec<u8>, Vec<u64>) {
        row_groups_listed(true)
    }

    /// The same file, with the second row group's column chunks listed in the
    /// footer back to front or in file order.
    fn row_groups_listed(backwards: bool) -> (Vec<u8>, Vec<u64>) {
        let page = || {
            let mut bytes = object(vec![integer(1, 0), integer(2, 2), integer(3, 2)], false);
            bytes.extend([0xab, 0xcd]);
            bytes
        };
        let mut bytes = MAGIC.to_vec();
        let mut starts = Vec::new();
        for group in 0..2 {
            if group == 1 {
                bytes.extend([0xff; 8]);
            }
            for _ in 0..2 {
                starts.push(bytes.len() as u64);
                bytes.extend(page());
            }
        }
        let len = page().len() as u64;
        let chunk = |at: u64| {
            let metadata = object(vec![integer(9, at), integer(7, len)], false);
            object(vec![integer(2, 0), field(3, 12, metadata)], false)
        };
        let rows = |columns: Vec<Vec<u8>>| object(vec![many(1, columns), integer(3, 1)], false);
        let groups = vec![
            rows(vec![chunk(starts[0]), chunk(starts[1])]),
            if backwards { rows(vec![chunk(starts[3]), chunk(starts[2])]) } else { rows(vec![chunk(starts[2]), chunk(starts[3])]) },
        ];
        let footer = object(vec![integer(1, 1), many(4, groups), integer(3, 2)], false);
        bytes.extend_from_slice(&footer);
        bytes.extend_from_slice(&(footer.len() as u32).to_le_bytes());
        bytes.extend_from_slice(MAGIC);
        (bytes, starts)
    }

    /// Each row group is a region of its own, holding the column chunks the
    /// footer lists for it and nothing from the other row group.
    #[test]
    fn each_row_group_is_a_region_holding_its_column_chunks() {
        let (bytes, starts) = two_row_groups();
        let page = starts[1] - starts[0];
        let d = Document::new(MemSource(bytes));
        let mut e = Evaluator::new(parquet());
        let groups = e.node(&d, &[4]).unwrap();
        assert_eq!((groups.name.as_str(), groups.child_count), ("row_groups", 2));
        for (i, first) in [(0, starts[0]), (1, starts[2])] {
            let group = e.node(&d, &[4, i]).unwrap();
            assert_eq!(group.type_name, "RowGroup");
            assert_eq!((group.offset_bits, group.size_bits), (first * 8, 2 * page * 8), "row group {i}");
            assert_eq!(e.node(&d, &[4, i, 0]).unwrap().child_count, 2, "row group {i}");
        }
        // Numbered in the order the footer lists them, and placed where each
        // entry says.
        let chunk = |e: &mut Evaluator, i: usize, j: usize| e.node(&d, &[4, i, 0, j]).unwrap().offset_bits / 8;
        assert_eq!([chunk(&mut e, 0, 0), chunk(&mut e, 0, 1), chunk(&mut e, 1, 0), chunk(&mut e, 1, 1)], [starts[0], starts[1], starts[3], starts[2]]);
        // A page is found where it is, under its row group and column chunk.
        let pages = nodes_of(&mut e, &d, "Page");
        assert_eq!(pages.len(), 4);
        for (path, info) in &pages {
            assert_eq!(&path[..1], &[4]);
            assert!(e.locate(&d, info.offset_bits).unwrap().starts_with(path));
        }
        assert_eq!(e.locate(&d, starts[2] * 8).unwrap()[..4], [4, 1, 0, 1]);
        // What is between the row groups belongs to neither.
        let between = e.spans(&d, (starts[1] + page) * 8, starts[2] * 8, 8).unwrap();
        assert_eq!(between.len(), 1);
        assert!(between[0].gap);
        assert_eq!((between[0].offset_bits / 8, between[0].size_bits / 8), (starts[1] + page, 8));
        // The column chunk says which entry placed it, starting from the row
        // group's own entry: in Parquet's names, and as Thrift stores it.
        let placed = e.origins(&d, &[4, 1, 0, 1]).unwrap().swap_remove(0);
        assert_eq!(placed.label, "footer.row_groups[1].columns[1]");
        assert_eq!(placed.stored.as_deref(), Some("footer.fields.row_groups.value.elems[1].fields.columns.value.elems[1]"));
    }

    /// The footer is after every page it places, so an overwrite of a column
    /// chunk entry in it has to move the column chunk it placed, the way a
    /// file read afresh has it.
    #[test]
    fn an_overwrite_of_a_column_chunk_entry_moves_its_column_chunk() {
        let (bytes, _) = row_groups_listed(true);
        let (after, starts) = row_groups_listed(false);
        assert_eq!(bytes.len(), after.len());
        let first = bytes.iter().zip(&after).position(|(a, b)| a != b).unwrap();
        let last = bytes.iter().zip(&after).rposition(|(a, b)| a != b).unwrap();
        let mut d = Document::new(MemSource(bytes));
        let mut e = Evaluator::new(parquet());
        let before: Vec<Vec<usize>> = nodes_of(&mut e, &d, "Page").into_iter().map(|(p, _)| p).collect();
        assert_eq!(e.node(&d, &[4, 1, 0, 0]).unwrap().offset_bits / 8, starts[3]);
        d.overwrite_bytes(first as u64, &after[first..=last]);
        e.invalidate_from(first as u64 * 8);
        let mut fresh = Evaluator::new(parquet());
        assert_eq!(e.node(&d, &[4, 1, 0, 0]).unwrap().offset_bits / 8, starts[2]);
        for path in &before {
            for k in 0..=path.len() {
                assert_eq!(e.node(&d, &path[..k]), fresh.node(&d, &path[..k]), "{:?}", &path[..k]);
            }
        }
    }

    /// A file of one column chunk holding one page, with the column's physical
    /// type and codec written where the real thing writes them. `pages` is the
    /// bytes of every page, header and payload together.
    fn one_column(physical: u64, codec: u64, pages: Vec<u8>) -> Vec<u8> {
        let metadata = object(vec![
            integer(1, physical),
            integer(4, codec),
            integer(9, 4),
            integer(7, pages.len() as u64),
        ], false);
        let column = object(vec![integer(2, 0), field(3, 12, metadata)], false);
        let row = object(vec![one(1, column), integer(3, 1)], false);
        let footer = object(vec![integer(1, 1), one(4, row), integer(3, 1)], false);
        let mut bytes = MAGIC.to_vec();
        bytes.extend(pages);
        bytes.extend_from_slice(&footer);
        bytes.extend_from_slice(&(footer.len() as u32).to_le_bytes());
        bytes.extend_from_slice(MAGIC);
        bytes
    }

    /// A page of `kind` whose payload is `body`, with `header` giving whatever
    /// extra fields that kind of page carries.
    fn one_page(kind: u64, mut header: Vec<Vec<u8>>, body: &[u8]) -> Vec<u8> {
        let mut fields = vec![integer(1, kind), integer(2, body.len() as u64), integer(3, body.len() as u64)];
        fields.append(&mut header);
        let mut bytes = object(fields, false);
        bytes.extend_from_slice(body);
        bytes
    }

    /// A dictionary page of four-byte integers reads as those integers, from
    /// the physical type the column chunk wrote and nothing else.
    #[test]
    fn a_dictionary_page_reads_as_the_columns_physical_type() {
        let mut body = Vec::new();
        for v in [7i32, -1, 1000] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        let dict = object(vec![integer(1, 3), integer(2, 0)], false);
        let bytes = one_column(1, 0, one_page(2, vec![field(7, 12, dict)], &body));
        let d = Document::new(MemSource(bytes));
        let mut e = Evaluator::new(parquet());
        let page = nodes_of(&mut e, &d, "Page");
        assert_eq!(page.len(), 1);
        // payload -> the stored space -> the repeat of values.
        let mut at = page[0].0.clone();
        at.extend([2, 0]);
        assert_eq!(e.node(&d, &at).unwrap().child_count, 3);
        for (i, want) in [7i128, -1, 1000].into_iter().enumerate() {
            let mut p = at.clone();
            p.push(i);
            assert_eq!(e.node(&d, &p).unwrap().value.as_int(), Some(want), "value {i}");
        }
    }

    /// The RLE and bit-packed hybrid, on bytes written by hand: a width byte,
    /// a run that repeats one index five times, and a run of one group of
    /// eight packed at that width.
    #[test]
    fn dictionary_indices_read_as_hybrid_runs() {
        // width 3, then header 5 << 1 = 10 with the value 2, then header
        // (1 << 1) | 1 = 3 with three bytes holding eight three-bit values.
        let body = [3u8, 10, 2, 3, 0xaa, 0xbb, 0xcc];
        let v2 = object(vec![
            integer(1, 8),
            integer(2, 0),
            integer(3, 8),
            integer(4, 8),
            integer(5, 0),
            integer(6, 0),
        ], false);
        let bytes = one_column(1, 0, one_page(3, vec![field(8, 12, v2)], &body));
        let d = Document::new(MemSource(bytes));
        let mut e = Evaluator::new(parquet());
        let indices = nodes_of(&mut e, &d, "DictionaryIndices");
        assert_eq!(indices.len(), 1, "the values should read as dictionary indices");
        let mut at = indices[0].0.clone();
        at.push(0);
        assert_eq!(e.node(&d, &at).unwrap().value, Value::UInt(3), "the width byte");
        let runs = nodes_of(&mut e, &d, "HybridRun");
        assert_eq!(runs.len(), 2);
        // The first run repeats one value: five of index 2, in one byte.
        let rle = nodes_of(&mut e, &d, "RleRun");
        assert_eq!(rle.len(), 1);
        let mut p = rle[0].0.clone();
        p.push(0);
        assert_eq!(e.node(&d, &p).unwrap().value.as_int(), Some(5), "how many times");
        p.pop();
        p.push(1);
        assert_eq!(e.node(&d, &p).unwrap().value.as_int(), Some(2), "the repeated index");
        assert_eq!(e.node(&d, &p).unwrap().size_bits, 8, "a width of 3 takes one whole byte");
        // The second is one group of eight, three bits each, so three bytes.
        let packed = nodes_of(&mut e, &d, "BitPackedRun");
        assert_eq!(packed.len(), 1);
        let mut q = packed[0].0.clone();
        q.push(0);
        assert_eq!(e.node(&d, &q).unwrap().value.as_int(), Some(8), "how many values");
        q.pop();
        q.push(1);
        assert_eq!(e.node(&d, &q).unwrap().size_bits, 24, "eight values of three bits");
    }

    /// A width of zero writes no value byte at all: every index is the same
    /// index, and a run of them is the header and nothing else.
    #[test]
    fn a_zero_width_index_takes_no_bytes() {
        // width 0, then header 4 << 1 = 8, repeating the only index there is.
        let body = [0u8, 8];
        let v2 = object(vec![integer(1, 4), integer(4, 8), integer(5, 0), integer(6, 0)], false);
        let bytes = one_column(1, 0, one_page(3, vec![field(8, 12, v2)], &body));
        let d = Document::new(MemSource(bytes));
        let mut e = Evaluator::new(parquet());
        let rle = nodes_of(&mut e, &d, "RleRun");
        assert_eq!(rle.len(), 1);
        assert_eq!(e.node(&d, &rle[0].0).unwrap().size_bits, 0);
    }

    /// A v2 page keeps its levels out of the packed part, sized by the two
    /// lengths in its header, so the levels stay readable whatever the codec.
    #[test]
    fn a_v2_page_puts_its_levels_in_front_of_the_packed_run() {
        let body = [0xd0u8, 0xd1, 0xe0, 0xe1, 0xe2, 1, 2, 3, 4];
        let v2 = object(vec![
            integer(1, 2),
            integer(4, 0),
            integer(5, 3),
            integer(6, 2),
        ], false);
        let bytes = one_column(2, 0, one_page(3, vec![field(8, 12, v2)], &body));
        let d = Document::new(MemSource(bytes));
        let mut e = Evaluator::new(parquet());
        let payload = nodes_of(&mut e, &d, "DataPageV2Payload");
        assert_eq!(payload.len(), 1);
        let at = payload[0].0.clone();
        let part = |e: &mut Evaluator, i: usize| {
            let mut p = at.clone();
            p.push(i);
            e.node(&d, &p).unwrap()
        };
        assert_eq!(part(&mut e, 0).size_bits, 2 * 8, "repetition levels");
        assert_eq!(part(&mut e, 1).size_bits, 3 * 8, "definition levels");
        assert_eq!(part(&mut e, 2).size_bits, 4 * 8, "what is left is the values");
    }

    #[test]
    fn the_footer_reads_as_its_fields() {
        // version = 1, num_rows = 8 (delta 2 from field 1), created_by = "me".
        let d = Document::new(MemSource(footed(&[0x15, 0x02, 0x26, 0x10, 0x38, 0x02, b'm', b'e', 0x00])));
        let mut e = Evaluator::new(parquet());
        // footer.fields[0]: the id names it and the value is the number.
        let id = e.node(&d, &[1, 0, 0, 0, 2]).unwrap().value;
        assert!(matches!(id, Value::Enum { name: Some(ref n), .. } if n == "version"), "got {id:?}");
        assert_eq!(e.node(&d, &[1, 0, 0, 0, 3]).unwrap().value.as_int(), Some(1));
        let rows = e.node(&d, &[1, 0, 0, 1, 2]).unwrap().value;
        assert!(matches!(rows, Value::Enum { name: Some(ref n), .. } if n == "num_rows"), "got {rows:?}");
        assert_eq!(e.node(&d, &[1, 0, 0, 1, 3]).unwrap().value.as_int(), Some(8));
        assert_eq!(e.node(&d, &[1, 0, 0, 2, 3, 1]).unwrap().value, Value::Str("me".into()));
    }

    /// The footer's length is `remaining - 8`, worked out in the room the
    /// file had left. Once the field is placed, its own limit is the window
    /// that length set, so a reader shown the arithmetic has to be shown the
    /// room the arithmetic saw: `max(17 - 8, 0) = 9` for a nine-byte footer,
    /// not `max(9 - 8, 0) = 1`.
    #[test]
    fn the_footers_length_is_written_out_in_the_room_it_measured() {
        let d = Document::new(MemSource(footed(&[0x15, 0x02, 0x26, 0x10, 0x38, 0x02, b'm', b'e', 0x00])));
        let mut e = Evaluator::new(parquet());
        assert_eq!(e.node(&d, &[1, 0]).unwrap().size_bits, 9 * 8);
        let rel = e.relations(&d, &[1, 0]).unwrap();
        assert_eq!(rel[0].written, "max(remaining - 8, 0)");
        assert_eq!(rel[0].substituted, "max(17 - 8, 0)");
        assert_eq!(rel[0].result, "9");
    }

    #[test]
    fn an_encrypted_file_still_says_how_it_was_encrypted() {
        // A FileCryptoMetaData: field 1 is an AES_GCM_V1 whose field 2 is an
        // aad_file_unique of two bytes, and field 2 is a key called "kf".
        let crypto = [0x1c, 0x1c, 0x28, 0x02, 0xaa, 0xbb, 0x00, 0x00, 0x18, 0x02, b'k', b'f', 0x00];
        let cipher = [0u8; 6];
        let mut v = ENCRYPTED.to_vec();
        v.extend_from_slice(&[0xAB; 20]);
        v.extend_from_slice(&crypto);
        v.extend_from_slice(&cipher);
        v.extend_from_slice(&((crypto.len() + cipher.len()) as u32).to_le_bytes());
        v.extend_from_slice(ENCRYPTED);
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(parquet());
        // The magic at the front is the whole of what picks this shape.
        assert_eq!(e.node(&d, &[]).unwrap().type_name, "ParquetEncrypted");
        assert_eq!(e.node(&d, &[1]).unwrap().size_bits, 20 * 8, "the modules are what the length leaves");
        assert_eq!(e.node(&d, &[2]).unwrap().size_bits, crypto.len() as u64 * 8, "Thrift ends the structure");
        assert_eq!(e.node(&d, &[3]).unwrap().size_bits, cipher.len() as u64 * 8, "the rest is the footer");
        let alg = e.node(&d, &[2, 0, 0, 2]).unwrap().value;
        assert!(matches!(alg, Value::Enum { name: Some(ref n), .. } if n == "encryption_algorithm"), "got {alg:?}");
        // The key metadata is binary, whatever a writer chose to put in it,
        // so it stays bytes rather than becoming a word.
        assert_eq!(e.node(&d, &[2, 0, 1, 3, 1]).unwrap().value, Value::Bytes { len: 2, preview: b"kf".to_vec() });
    }
}
