//! Apache Avro: records written as bare values, read through a schema.
//!
//! Two shapes are on disk. An object container file opens with `Obj` and the
//! byte 1, then a metadata map holding the schema as JSON (`avro.schema`) and
//! the codec (`avro.codec`), then a 16-byte sync marker. After that come
//! blocks: how many records, how many bytes, the records, and the sync marker
//! again. A block is compressed whole when the codec says so.
//!
//! The other is one record behind the Confluent wire format: a zero byte, a
//! big-endian schema ID, and the record, with no schema in the file at all.
//! That is how the Vera C. Rubin Observatory sends its alerts, and its IDs are
//! the alert schema's major version times 100 plus its minor. Those schemas
//! are bundled, so an alert with one of their IDs reads in full; any other ID
//! reads its header and leaves the record as bytes.
//!
//! What a record holds is a [`Ty::Schema`](crate::template::Ty::Schema) built
//! from the schema (see `avro_schema.rs`).

use std::sync::Arc;

use crate::template::{Endian::*, Expr as E, KeyPart, Template, Ty as T, Until, Valid};

use super::avro_schema::{self, AlertTypes, Blocks, FileTypes, Reach, KIND, KIND_ALERT, KIND_BLOCK};

/// What an object container file opens with.
pub const MAGIC: &[u8] = b"Obj\x01";

/// Whether these are the first bytes of a Rubin alert in the Confluent wire
/// format: a zero byte, then the ID of an alert schema Qubero has. The zero
/// byte alone says nothing, so this is asked only of a file named `.avro`.
pub fn is_alert(head: &[u8], _len: u64) -> bool {
    match head {
        [0, a, b, c, d, ..] => avro_schema::is_alert_id(i128::from(u32::from_be_bytes([*a, *b, *c, *d]))),
        _ => false,
    }
}

/// A metadata value: the schema as the JSON it is, the codec as text, and
/// anything else as the bytes the map says it is.
fn meta_value() -> T {
    let json = T::structure_named(
        "schema",
        "",
        "json",
        vec![("length", T::zigzag()), ("json", T::sized(E::field("length"), T::json()))],
    );
    let text = Reach::file().ty(&avro_schema::Avro::String);
    T::matches(E::within(&["key", "text"]), vec![("avro.schema", json), ("avro.codec", text)], Reach::file().ty(&avro_schema::Avro::Bytes))
}

/// The header's metadata map, in blocks as any map is.
fn meta() -> T {
    let entry = T::structure("entry", vec![("key", Reach::file().ty(&avro_schema::Avro::String)), ("value", meta_value())]).named_by("key");
    let count = || E::field("count");
    let items = E::cond(count().less_than(E::lit(0)), E::lit(0).sub(count()), count());
    let block = T::structure_named(
        "block",
        "",
        "items",
        vec![("count", T::zigzag()), ("size", T::when(count().less_than(E::lit(0)), T::zigzag())), ("items", T::array(entry, items))],
    )
    .counted_as("block");
    T::repeat(block, Until::FieldValue { field: "count".into(), value: 0 })
}

/// What a block whose sync marker is not the header's is told.
const SYNC_DIFFERS: &str = "Differs from the sync marker in the file header";

/// Whether a block's sync marker is the header's: sixteen bytes, which an
/// expression reads as two halves of eight.
fn same_sync() -> E {
    let half = |field: E, n: i128| E::PeekIn { at: Box::new(E::start_of(field).mul(E::lit(8)).add(E::lit(n * 64))), bits: 64, endian: Big };
    let same = |n| half(E::field("sync"), n).equal_to(half(E::within(&["header", "sync"]), n));
    same(0).both(same(1))
}

fn block() -> T {
    T::structure(
        "Block",
        vec![
            ("count", T::zigzag()),
            ("size", T::zigzag()),
            ("data", T::sized(E::field("size"), T::schema(KIND_BLOCK, avro_schema::meta_entries(), Vec::new()))),
            ("sync", T::bytes(E::lit(16))),
        ],
    )
    .field_valid("sync", Valid::Expr { expr: same_sync(), msg: Some(SYNC_DIFFERS.into()) })
    .contents("data")
    .counted_as("block")
}

fn container() -> T {
    T::structure(
        "ObjectContainer",
        vec![
            (
                "header",
                T::structure("Header", vec![("magic", T::magic(MAGIC)), ("meta", meta()), ("sync", T::bytes(E::lit(16)))]),
            ),
            ("blocks", T::repeat(T::Named("Block".into()), Until::End)),
        ],
    )
}

/// One record in the Confluent wire format, typed by the bundled alert schema
/// its ID names.
fn framed() -> T {
    let record = T::sized(E::Remaining, T::schema(KIND_ALERT, Vec::new(), vec![KeyPart::Int(E::field("schema_id")), KeyPart::TextLit("".into())]));
    T::structure(
        "FramedRecord",
        vec![
            ("magic", T::magic(&[0])),
            ("schema_id", T::u32(Big)),
            ("major", T::computed(E::field("schema_id").div(E::lit(100)))),
            ("minor", T::computed(E::field("schema_id").modulo(E::lit(100)))),
            ("record", record),
        ],
    )
    .field_doc(
        "schema_id",
        "ID of the schema this record was written with, as numbered by the schema registry. Rubin numbers its alert schemas as major × 100 + minor: ID 900 is schema 9.0 and ID 1101 is schema 11.1. The major and minor fields beside this one show the two parts.",
    )
}

pub fn avro() -> Template {
    let root = T::switch(E::peek(8, Big), vec![(0, framed())], container());
    Template::new("avro", root)
        .with_type("Block", block())
        .with_schema(KIND, Arc::new(FileTypes::default()))
        .with_schema(KIND_BLOCK, Arc::new(Blocks))
        .with_schema(KIND_ALERT, Arc::new(AlertTypes::default()))
}

#[cfg(test)]
mod tests;
