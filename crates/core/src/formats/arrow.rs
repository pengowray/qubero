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
//! uncompressed length, -1 for one left as it was. ZSTD and LZ4 buffers open
//! and read as the plain ones do. Arrow's LZ4 is the frame format, magic and
//! descriptor and end mark included, and not the bare blocks a ROOT basket
//! or a Parquet LZ4_RAW page holds.

use super::arrow_schema::{table, DICTIONARY_BATCH, FOOTER, MESSAGE, RECORD_BATCH, SCHEMA, STRUCTS, TABLES};
use super::arrow_walk::{buffer_walk, field_layout, node_walk, ROLE, SCHEMA_FIELDS, SCHEMA_TABLE};
use crate::codec::Codec;
use crate::formats::flatbuf;
use crate::template::{Anchor, Endian::{Big, Little}, Expr as E, Step, Template, Time, Ty as T};
use std::sync::Arc;

/// What an Arrow file opens and closes with.
pub const MAGIC: &[u8] = b"ARROW1";

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

// How the bytes of a buffer read, once its column and its purpose are known.
// Internal to this reader; what a reader of the file sees is the type each
// one picks.
pub(super) const BYTES: i128 = 0;
pub(super) const BITS: i128 = 1;
pub(super) const I8: i128 = 2;
pub(super) const U8: i128 = 3;
pub(super) const I16: i128 = 4;
pub(super) const U16: i128 = 5;
pub(super) const I32: i128 = 6;
pub(super) const U32: i128 = 7;
pub(super) const I64: i128 = 8;
pub(super) const U64: i128 = 9;
pub(super) const F16: i128 = 10;
pub(super) const F32: i128 = 11;
pub(super) const F64: i128 = 12;
pub(super) const DAYS: i128 = 13;
pub(super) const DATE_MILLIS: i128 = 14;
/// A timestamp with a zone, in seconds, then milliseconds, microseconds and
/// nanoseconds: `TimeUnit` added to this.
pub(super) const INSTANT: i128 = 15;
/// A timestamp with no zone, which is wall-clock digits: the same four units.
pub(super) const WALL_CLOCK: i128 = 19;
pub(super) const YEAR_MONTH: i128 = 23;
pub(super) const DAY_TIME: i128 = 24;
pub(super) const MONTH_DAY_NANO: i128 = 25;
pub(super) const FIXED_WIDTH: i128 = 26;
pub(super) const TEXT: i128 = 27;
pub(super) const BINARY: i128 = 28;
pub(super) const TEXT_VIEWS: i128 = 29;
pub(super) const BINARY_VIEWS: i128 = 30;
pub(super) const VALIDITY_BITMAP: i128 = 31;
/// Offsets that run one past the last value, 32-bit and 64-bit.
pub(super) const ENDS32: i128 = 32;
pub(super) const ENDS64: i128 = 33;

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
        cases.push((INSTANT + i, timed(&format!("timestamp[{unit}, zoned]"), i64le.clone(), 8, time.clone())));
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
/// compressing it did not help. Either codec opens and reads as the buffer
/// would have: a codec of 0 is an LZ4 frame, which is also what a body that
/// leaves the codec unwritten means, and 1 is ZSTD.
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
        let opened = T::switch(
            codec.clone(),
            vec![
                (0, T::decoded(E::Remaining, Codec::Lz4Frame, plain(name))),
                (1, T::decoded(E::Remaining, Codec::Zstd, plain(name))),
            ],
            T::bytes(E::Remaining),
        );
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
