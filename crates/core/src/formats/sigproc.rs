//! SIGPROC filterbank (`.fil`): the format pulsar and SETI searches write
//! their spectra in, a header of keyword records and then the samples. Every
//! Breakthrough Listen filterbank product is one, beside the HDF5 copy.
//!
//! The header is a run of strings, each a little-endian `u32` length and that
//! many bytes, between `HEADER_START` and `HEADER_END`. After each keyword
//! string comes its value, whose type the keyword fixes: a little-endian
//! `int32`, a little-endian `double`, one byte, or another string. Nothing in
//! the file says how long a value is, so a keyword this reader does not know
//! ends the header there, and everything after it is left unread rather than
//! guessed at.
//!
//! The samples follow `HEADER_END` with nothing between: one time sample after
//! another, each `nifs` runs of `nchans` values, `nbits` each, as many as the
//! file holds. So a file has no sample count of its own, and a part of a time
//! sample at the end is a file cut short.
//!
//! What is read and what is not:
//!
//! - 8, 16 and 32-bit samples read as numbers: unsigned bytes (signed when the
//!   header says `signed = 1`), unsigned 16-bit, and 32-bit floats, which is
//!   what sigproc writes at each width. Breakthrough Listen writes 32 and 8.
//! - 1, 2 and 4-bit samples are left as bytes, under `Packed samples`. No
//!   file of that width was at hand to check the order of the samples inside
//!   a byte against.
//! - `src_raj` and `src_dej` are the doubles the file holds, which sigproc
//!   writes as `hhmmss.s` and `ddmmss.s`, not as degrees.

use crate::template::{Encoding, Endian::Little, Expr as E, StrLen, TableFact, TableShape, Template, Ty as T, Until};

/// The first sixteen bytes of every filterbank file: the length of the string
/// `HEADER_START`, and the string.
pub const MAGIC: &[u8] = b"\x0c\x00\x00\x00HEADER_START";

/// The keywords whose value is a little-endian `int32`.
const INTS: &[&str] = &[
    "telescope_id", "machine_id", "data_type", "nchans", "nbits", "nifs", "nbeams", "ibeam", "nsamples", "barycentric",
    "pulsarcentric", "nbins",
];

/// The keywords whose value is a little-endian `double`.
const DOUBLES: &[&str] =
    &["tstart", "tsamp", "fch1", "foff", "src_raj", "src_dej", "az_start", "za_start", "refdm", "period", "fchannel"];

/// The keywords whose value is a string, written like a keyword.
const STRINGS: &[&str] = &["source_name", "rawdatafile"];

/// The keywords that mark a place and have no value. `FREQUENCY_START` and
/// `FREQUENCY_END` hold a list of `fchannel` records between them, for a file
/// whose channels are not evenly spaced.
const MARKS: &[&str] = &["HEADER_START", "HEADER_END", "FREQUENCY_START", "FREQUENCY_END"];

/// A string as the header writes one: its length, and its bytes.
fn string() -> T {
    T::structure_named("String", "", "text", vec![("length", T::u32(Little)), ("text", T::text(StrLen::Fixed(E::field("length")), Encoding::Ascii))])
        .machinery(&["length"])
}

/// What one record of the header says it ended: 1 for `HEADER_END`, 2 for a
/// keyword this reader does not know, after which nothing can be read, and
/// nothing for every other.
const ENDS_HEADER: i128 = 1;
const UNKNOWN: i128 = 2;

/// One record of the header: a keyword, and the value its keyword says it has.
fn record() -> T {
    let mut values: Vec<(&str, T)> = Vec::new();
    values.extend(INTS.iter().map(|k| (*k, T::i32(Little))));
    values.extend(DOUBLES.iter().map(|k| (*k, T::F64(Little))));
    values.extend(STRINGS.iter().map(|k| (*k, string())));
    values.push(("signed", T::u8()));
    let mut ends: Vec<(&str, T)> = values.iter().map(|(k, _)| (*k, T::computed(E::lit(0)))).collect();
    ends.extend(MARKS.iter().map(|k| (*k, T::computed(E::lit(0)))));
    ends.retain(|(k, _)| *k != "HEADER_END");
    ends.push(("HEADER_END", T::computed(E::lit(ENDS_HEADER))));
    // A mark has no value, so it has no bytes after its keyword.
    let marks = MARKS.iter().map(|k| (*k, T::bytes(E::lit(0))));
    let value = T::matches(E::field("keyword"), values.into_iter().chain(marks).collect(), T::bytes(E::lit(0)));
    T::structure_named(
        "Record",
        "keyword",
        "value",
        vec![
            ("length", T::u32(Little)),
            ("keyword", T::text(StrLen::Fixed(E::field("length")), Encoding::Ascii)),
            ("value", value),
            ("ends", T::matches(E::field("keyword"), ends, T::computed(E::lit(UNKNOWN)))),
        ],
    )
    .machinery(&["length", "ends"])
    .counted_as("record")
}

/// The value of the header record whose keyword is `name`, or nothing when
/// the header has none.
fn header(name: &str) -> E {
    E::tagged_text("header", &["keyword"], name, &["value"])
}

/// How many values make one time sample: `nchans` for each of `nifs`.
fn per_sample() -> E {
    header("nchans").mul(header("nifs").or(E::lit(1)))
}

/// The samples, read as what `nbits` says they are, and then the part of a
/// time sample a file cut short ends with.
fn samples() -> T {
    let run = |bits: i128, elem: T| {
        // Whole time samples only: the bytes after the last one are a part
        // row, and laying values over them would put the start of a time
        // sample where its end should be.
        let row_bytes = per_sample().mul(E::lit(bits)).div(E::lit(8)).at_least(E::lit(1));
        let rows = E::Remaining.div(row_bytes);
        T::array(elem, rows.mul(per_sample()))
    };
    let bytes = T::switch(header("signed"), vec![(1, run(8, T::Int { bits: 8, endian: Little }))], run(8, T::u8()));
    let shape = TableShape {
        columns: Some(per_sample()),
        column_word: Some("channel".into()),
        row_word: Some("sample".into()),
        rate: Some(E::real(1.0).div(header("tsamp"))),
        facts: vec![
            TableFact::named("nchans", header("nchans")),
            TableFact::named("nifs", header("nifs")),
            TableFact::named("nbits", header("nbits")),
            TableFact::named("tsamp", header("tsamp")).in_unit("s"),
            TableFact::named("fch1", header("fch1")).in_unit("MHz"),
            TableFact::named("foff", header("foff")).in_unit("MHz"),
            TableFact::named("tstart", header("tstart")).in_unit("MJD"),
        ],
        ..Default::default()
    };
    let table = |values: T| {
        T::structure("Samples", vec![("samples", values), ("part_row", T::bytes(E::Remaining))])
            .payload(&["samples"])
            .field_table("samples", shape.clone())
    };
    T::switch(
        header("nbits"),
        vec![(8, table(bytes)), (16, table(run(16, T::u16(Little)))), (32, table(run(32, T::F32(Little))))],
        // 1, 2 and 4 bits, or a width sigproc does not write: the bytes, as
        // bytes, since the order of the samples inside one is not known here.
        T::structure("Packed samples", vec![("bytes", T::bytes(E::Remaining))]),
    )
}

/// What follows the header: the samples when the header ended where it should,
/// and the rest of the file, unread, after a keyword this reader does not know.
fn after_header() -> T {
    let ended = E::tagged_text("header", &["keyword"], "HEADER_END", &["ends"]);
    T::switch(ended, vec![(ENDS_HEADER, samples())], T::structure("Unread", vec![("rest", T::bytes(E::Remaining))]))
}

pub fn sigproc() -> Template {
    Template::new(
        "sigproc",
        T::structure(
            "Filterbank",
            vec![
                ("header", T::repeat(record(), Until::Cond(E::field("ends")))),
                ("data", after_header()),
            ],
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn string(out: &mut Vec<u8>, s: &str) {
        out.extend_from_slice(&(s.len() as u32).to_le_bytes());
        out.extend_from_slice(s.as_bytes());
    }

    /// A header with these records, each a keyword and its value's bytes.
    fn file(records: &[(&str, Vec<u8>)], samples: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        string(&mut out, "HEADER_START");
        for (k, v) in records {
            string(&mut out, k);
            out.extend_from_slice(v);
        }
        string(&mut out, "HEADER_END");
        out.extend_from_slice(samples);
        out
    }

    fn int(v: i32) -> Vec<u8> {
        v.to_le_bytes().to_vec()
    }
    fn double(v: f64) -> Vec<u8> {
        v.to_le_bytes().to_vec()
    }
    fn text(s: &str) -> Vec<u8> {
        let mut out = Vec::new();
        string(&mut out, s);
        out
    }

    fn common(nbits: i32) -> Vec<(&'static str, Vec<u8>)> {
        vec![
            ("source_name", text("B0329+54")),
            ("nchans", int(3)),
            ("nifs", int(2)),
            ("nbits", int(nbits)),
            ("tsamp", double(0.9)),
            ("fch1", double(1500.0)),
            ("foff", double(-0.5)),
        ]
    }

    #[test]
    fn the_file_opens_with_the_magic() {
        let b = file(&common(8), &[]);
        assert!(b.starts_with(MAGIC));
        assert_eq!(crate::formats::sniff(&b, b.len() as u64), Some("sigproc"));
    }

    #[test]
    fn the_header_is_its_records_and_the_samples_fill_the_rest() {
        let mut samples = Vec::new();
        for v in [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0] {
            samples.extend_from_slice(&v.to_le_bytes());
        }
        // Two whole time samples of 3 channels and 2 IFs, and 3 bytes of a third.
        samples.extend_from_slice(&[0xaa, 0xbb, 0xcc]);
        let b = file(&common(32), &samples);
        let d = Document::new(MemSource(b.clone()));
        let mut ev = Evaluator::new(sigproc());
        // HEADER_START, seven records, HEADER_END.
        assert_eq!(ev.node(&d, &[0]).unwrap().child_count, 9);
        let name = ev.node(&d, &[0, 1, 2, 1]).unwrap().value;
        assert_eq!(name, Value::Str("B0329+54".into()));
        assert_eq!(ev.node(&d, &[0, 5, 2]).unwrap().value, Value::Float(0.9));
        let run = ev.node(&d, &[1, 0]).unwrap();
        assert_eq!((run.type_name.as_str(), run.child_count), ("f32 le[]", 12));
        assert_eq!(ev.node(&d, &[1, 0, 11]).unwrap().value, Value::Float(12.0));
        let part = ev.node(&d, &[1, 1]).unwrap();
        assert_eq!((part.offset_bits / 8, part.size_bits / 8), (b.len() as u64 - 3, 3));
        // A table of 6 columns, 1/0.9 rows a second, with its unit-bearing facts.
        let shape = ev.table_shape(&d, &[1, 0]).unwrap().expect("a shape");
        assert_eq!(shape.columns, Some(6));
        assert_eq!(shape.rate, Some(1.0 / 0.9));
        let facts: Vec<(&str, &str, &str)> = shape.facts.iter().map(|f| (f.label.as_str(), f.value.as_str(), f.unit.as_str())).collect();
        assert_eq!(facts[0], ("nchans", "3", ""));
        assert_eq!(facts[3], ("tsamp", "0.9", "s"));
        assert_eq!(facts[5], ("foff", "-0.5", "MHz"));
    }

    #[test]
    fn eight_bit_samples_are_bytes_and_signed_when_the_header_says() {
        let b = file(&common(8), &[1, 2, 3, 250, 5, 6]);
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(sigproc());
        assert_eq!(ev.node(&d, &[1, 0]).unwrap().type_name, "u8[]");
        assert_eq!(ev.node(&d, &[1, 0, 3]).unwrap().value, Value::UInt(250));
        let mut records = common(8);
        records.push(("signed", vec![1]));
        let b = file(&records, &[1, 2, 3, 250, 5, 6]);
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(sigproc());
        assert_eq!(ev.node(&d, &[1, 0, 3]).unwrap().value.as_int(), Some(-6));
    }

    #[test]
    fn a_keyword_nobody_knows_ends_the_header_and_leaves_the_rest_unread() {
        let mut records = common(8);
        records.insert(2, ("new_keyword", int(7)));
        let b = file(&records, &[1, 2, 3, 4, 5, 6]);
        let d = Document::new(MemSource(b.clone()));
        let mut ev = Evaluator::new(sigproc());
        // HEADER_START, source_name, nchans, and the keyword it stopped at.
        assert_eq!(ev.node(&d, &[0]).unwrap().child_count, 4);
        let rest = ev.node(&d, &[1]).unwrap();
        assert_eq!(rest.type_name, "Unread");
        assert_eq!(rest.offset_bits + rest.size_bits, b.len() as u64 * 8);
    }

    #[test]
    fn samples_narrower_than_a_byte_stay_bytes() {
        let b = file(&common(2), &[0x1b; 6]);
        let d = Document::new(MemSource(b.clone()));
        let mut ev = Evaluator::new(sigproc());
        let data = ev.node(&d, &[1]).unwrap();
        assert_eq!(data.type_name, "Packed samples");
        let bytes = ev.node(&d, &[1, 0]).unwrap();
        assert_eq!((bytes.size_bits, bytes.offset_bits + bytes.size_bits), (48, b.len() as u64 * 8));
        assert_eq!(ev.table_shape(&d, &[1, 0]).unwrap(), None);
    }

    #[test]
    fn a_header_with_no_samples_has_an_empty_table() {
        let b = file(&common(32), &[]);
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(sigproc());
        assert_eq!(ev.node(&d, &[1, 0]).unwrap().child_count, 0);
    }
}
