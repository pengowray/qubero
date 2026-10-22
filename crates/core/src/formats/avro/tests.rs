use super::*;
use crate::document::Document;
use crate::eval::{Evaluator, Value};
use crate::source::MemSource;

/// A long as Avro writes it: zigzagged, then seven bits a byte, low first.
fn long(v: i64) -> Vec<u8> {
    let mut n = ((v << 1) ^ (v >> 63)) as u64;
    let mut out = Vec::new();
    loop {
        let b = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(b);
            return out;
        }
        out.push(b | 0x80);
    }
}

fn text(s: &str) -> Vec<u8> {
    [long(s.len() as i64), s.as_bytes().to_vec()].concat()
}

const SYNC: [u8; 16] = *b"0123456789abcdef";

/// A container file of one block holding `records`, with this schema and
/// codec.
fn container(schema: &str, codec: &str, count: i64, records: &[u8]) -> Vec<u8> {
    let mut b = MAGIC.to_vec();
    b.extend(long(2));
    b.extend(text("avro.schema"));
    b.extend(text(schema));
    b.extend(text("avro.codec"));
    b.extend(text(codec));
    b.extend(long(0));
    b.extend(SYNC);
    b.extend(long(count));
    b.extend(long(records.len() as i64));
    b.extend(records);
    b.extend(SYNC);
    b
}

fn eval(bytes: Vec<u8>) -> (Document<MemSource>, Evaluator) {
    (Document::new(MemSource(bytes)), Evaluator::new(avro()))
}

/// The node down this run of field names and list indices.
fn at(ev: &mut Evaluator, d: &Document<MemSource>, names: &[&str]) -> Vec<usize> {
    let mut path = Vec::new();
    for n in names {
        match n.parse::<usize>() {
            Ok(i) => path.push(i),
            Err(_) => path = ev.child_named(d, &path, n).unwrap().unwrap_or_else(|| panic!("no {n} under {path:?}")),
        }
    }
    path
}

#[test]
fn a_block_with_a_negative_count_has_its_size_after_it() {
    let schema = r#"{"type": "record", "name": "r", "fields": [{"name": "xs", "type": {"type": "array", "items": "int"}}]}"#;
    // Two blocks of items: two written plainly, then three with a negative
    // count and their size in bytes, then the zero that ends the array.
    let record = [long(2), long(5), long(-6), long(-3), long(3), long(1), long(2), long(300), long(0)].concat();
    let (d, mut ev) = eval(container(schema, "null", 1, &record));
    let xs = at(&mut ev, &d, &["blocks", "0", "data", "0", "xs"]);
    assert_eq!(ev.node(&d, &xs).unwrap().child_count, 3);
    let second = [xs.as_slice(), &[1]].concat();
    assert_eq!(ev.node(&d, &[second.as_slice(), &[0]].concat()).unwrap().value, Value::Int(-3));
    assert_eq!(ev.node(&d, &[second.as_slice(), &[1]].concat()).unwrap().value, Value::Int(3));
    let items = [second.as_slice(), &[2]].concat();
    assert_eq!(ev.node(&d, &items).unwrap().child_count, 3);
    assert_eq!(ev.node(&d, &[items.as_slice(), &[2]].concat()).unwrap().value, Value::Int(300));
}

#[test]
fn a_type_that_names_itself_reads_to_where_it_stops() {
    let schema = r#"{"type": "record", "name": "Link", "fields": [{"name": "v", "type": "int"}, {"name": "next", "type": ["null", "Link"]}]}"#;
    let record = [long(1), long(1), long(2), long(1), long(3), long(0)].concat();
    let (d, mut ev) = eval(container(schema, "null", 1, &record));
    let third = at(&mut ev, &d, &["blocks", "0", "data", "0", "next", "value", "next", "value"]);
    assert_eq!(ev.node(&d, &third).unwrap().type_name, "Link");
    let v = ev.child_named(&d, &third, "v").unwrap().unwrap();
    assert_eq!(ev.node(&d, &v).unwrap().value, Value::Int(3));
    let end = at(&mut ev, &d, &["blocks", "0", "data", "0", "next", "value", "next", "value", "next", "value"]);
    assert!(ev.node(&d, &end).unwrap().absent);
}

#[test]
fn a_codec_qubero_cannot_unpack_leaves_the_block_as_bytes_and_says_so() {
    let schema = r#""long""#;
    let (d, mut ev) = eval(container(schema, "lzo", 2, &[1, 2, 3, 4]));
    let data = at(&mut ev, &d, &["blocks", "0", "data"]);
    let n = ev.node(&d, &data).unwrap();
    assert_eq!(n.size_bits, 32);
    assert!(n.doc.as_deref().is_some_and(|d| d.contains("lzo")), "{:?}", n.doc);
    // The block after it is still where its size says.
    let sync = at(&mut ev, &d, &["blocks", "0", "sync"]);
    assert_eq!(ev.node(&d, &sync).unwrap().offset_bits, (d.len_bytes() - 16) * 8);
}

#[test]
fn a_framed_record_with_an_id_qubero_has_no_schema_for_reads_its_header() {
    let mut b = vec![0];
    b.extend(1205u32.to_be_bytes());
    b.extend([1, 2, 3, 4, 5]);
    let (d, mut ev) = eval(b);
    assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::UInt(1205));
    assert_eq!(ev.node(&d, &[2]).unwrap().value, Value::Int(12));
    assert_eq!(ev.node(&d, &[3]).unwrap().value, Value::Int(5));
    let record = ev.node(&d, &[4]).unwrap();
    assert_eq!(record.size_bits, 40);
    assert!(record.doc.as_deref().is_some_and(|d| d.contains("3.0 to 11.1")), "{:?}", record.doc);
}

#[test]
fn a_framed_record_reads_through_the_bundled_schema_its_id_names() {
    // The front of schema 7.0's alert: alertId, then the diaSource record,
    // whose first field is diaSourceId. The rest of the record is cut off, so
    // only the fields that are there are asked about.
    let mut b = vec![0];
    b.extend(700u32.to_be_bytes());
    b.extend(long(42));
    b.extend(long(-7));
    let (d, mut ev) = eval(b);
    assert_eq!(ev.node(&d, &[4, 0]).unwrap().name, "alertId");
    assert_eq!(ev.node(&d, &[4, 0]).unwrap().value, Value::Int(42));
    let first = ev.node(&d, &[4, 1, 0]).unwrap();
    assert_eq!((first.name.as_str(), first.value), ("diaSourceId", Value::Int(-7)));
}

#[test]
fn the_bundled_schemas_are_from_the_commit_the_script_names() {
    let script = include_str!("../../../../../tools/alert_schemas.mjs");
    assert!(script.contains(&format!("const COMMIT = \"{}\";", super::super::avro_alerts::COMMIT)));
}
