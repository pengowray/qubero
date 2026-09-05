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
//! footer is what places them, and placing them is not done here yet, so this
//! reads as one region. What is written down already is [`PAGE_SCHEMA`], the
//! structs a page header is made of, since they are the other Thrift in the
//! file and the same schema reads them.
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

use crate::formats::thrift::{self, Field, Struct, What::{Enum, Plain, Struct as Sub, Text}};
use crate::template::{Endian::{Big, Little}, Expr as E, Template, Ty as T};

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

/// The structs a page header is made of.
///
/// Nothing places one yet, so nothing reads one yet either. They are here
/// because they are the same schema and the same reader, and because what is
/// missing for the pages is arithmetic over the footer rather than any of
/// this.
pub const PAGE_SCHEMA: &[Struct] = &[
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
            // Every page of every column, undecoded: the footer is what says
            // where one row group ends and the next begins.
            ("row_groups", T::bytes(E::Remaining.sub(E::lit(8)).sub(footer_length()).at_least(E::lit(0)))),
            // The Thrift `FileMetaData`, which runs from here to the eight
            // bytes that measured it. Sized by what is left rather than by the
            // length field, so a length larger than the file still places the
            // bytes that are there.
            (
                "footer",
                T::sized(E::Remaining.sub(E::lit(8)).at_least(E::lit(0)), T::Named("parquet.FileMetaData".into())),
            ),
            ("footer_length", T::u32(Little)),
            ("footer_magic", T::magic(MAGIC)),
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
    for (name, ty) in thrift::types("parquet", &owned) {
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
        assert_eq!(e.node(&d, &[1]).unwrap().size_bits, 40 * 8);
        assert_eq!(e.node(&d, &[2]).unwrap().size_bits, 12 * 8);
        assert_eq!(e.node(&d, &[3]).unwrap().value, Value::UInt(12));
    }

    #[test]
    fn a_file_with_no_row_groups_still_reads() {
        let d = Document::new(MemSource(file(0, 4, MAGIC)));
        let mut e = Evaluator::new(parquet());
        assert_eq!(e.node(&d, &[1]).unwrap().size_bits, 0);
        assert_eq!(e.node(&d, &[2]).unwrap().size_bits, 4 * 8);
    }

    #[test]
    fn a_footer_longer_than_the_file_places_what_is_there() {
        let mut bytes = file(8, 4, MAGIC);
        let n = bytes.len();
        bytes[n - 8..n - 4].copy_from_slice(&0xFFFF_u32.to_le_bytes());
        let d = Document::new(MemSource(bytes));
        let mut e = Evaluator::new(parquet());
        assert_eq!(e.node(&d, &[1]).unwrap().size_bits, 0);
        assert_eq!(e.node(&d, &[2]).unwrap().size_bits, 12 * 8);
    }

    /// A footer holding the fields `bytes` says, wrapped in what finds it.
    fn footed(fields: &[u8]) -> Vec<u8> {
        let mut v = MAGIC.to_vec();
        v.extend_from_slice(fields);
        v.extend_from_slice(&(fields.len() as u32).to_le_bytes());
        v.extend_from_slice(MAGIC);
        v
    }

    #[test]
    fn the_footer_reads_as_its_fields() {
        // version = 1, num_rows = 8 (delta 2 from field 1), created_by = "me".
        let d = Document::new(MemSource(footed(&[0x15, 0x02, 0x26, 0x10, 0x38, 0x02, b'm', b'e', 0x00])));
        let mut e = Evaluator::new(parquet());
        // footer.fields[0]: the id names it and the value is the number.
        let id = e.node(&d, &[2, 0, 0, 2]).unwrap().value;
        assert!(matches!(id, Value::Enum { name: Some(ref n), .. } if n == "version"), "got {id:?}");
        assert_eq!(e.node(&d, &[2, 0, 0, 3]).unwrap().value.as_int(), Some(1));
        let rows = e.node(&d, &[2, 0, 1, 2]).unwrap().value;
        assert!(matches!(rows, Value::Enum { name: Some(ref n), .. } if n == "num_rows"), "got {rows:?}");
        assert_eq!(e.node(&d, &[2, 0, 1, 3]).unwrap().value.as_int(), Some(8));
        assert_eq!(e.node(&d, &[2, 0, 2, 3, 1]).unwrap().value, Value::Str("me".into()));
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
        assert_eq!(e.node(&d, &[2]).unwrap().size_bits, 9 * 8);
        let rel = e.relations(&d, &[2]).unwrap();
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
