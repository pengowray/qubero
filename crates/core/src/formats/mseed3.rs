//! miniSEED 3: the FDSN's 2023 replacement for the record SEED handed down
//! from the 1980s, and a different layout from the 2.4 one `mseed` reads.
//!
//! The same idea as before. A file is a run of records and nothing else: no
//! file header, no index, no end marker, and a record that stands on its own
//! so a reader can start anywhere and find its footing. What changed is
//! everything a reader had to guess at.
//!
//! *Which way round the numbers are* is stated rather than deduced. Every
//! binary field of the header is little-endian, always, so nothing here peeks
//! at a year to work out the byte order. The data payload is the exception,
//! and a deliberate one: the integer and float encodings are little-endian
//! like the header, and the Steim encodings are big-endian, because a Steim
//! frame was defined big-endian in 1991 and the FDSN chose not to redefine it.
//!
//! *How long the record is* is stated too, and it is not a power of two and
//! not in a blockette. A record is the forty-byte fixed header plus the three
//! lengths written at the end of it: the source identifier, the extra headers
//! and the data. That is the whole of it, so records in one file may be any
//! length and each one says its own.
//!
//! The blockette chain is gone. What it carried is now two things. The three
//! headers a reader needs are fields of the fixed header: the encoding, the
//! sample rate and the number of samples, which in 2.4 were spread between
//! blockette 1000, a factor-and-multiplier pair and a 16-bit count. Everything
//! else is JSON, in the extra headers, under names the FDSN registers; a
//! record's calibration, its timing exceptions and its clock model are objects
//! in there rather than blockettes 300, 500 and 1001. So this template reads
//! them as JSON and the reader sees the names the writer wrote.
//!
//! The channel is one string rather than four space-padded fields. A source
//! identifier is a URI, and in practice always `FDSN:NET_STA_LOC_B_S_SS`:
//! network, station, location, and the band, source and subsource codes that
//! used to be the three characters of a channel name.
//!
//! The samples are exposed as far as their shape goes and no further, exactly
//! as in 2.4, and by the same code: the encodings share their numbers with the
//! older format, so the Steim frames and the fixed-width runs are `mseed`'s
//! own and the differences are not undone into samples here.
//!
//! The record carries a CRC-32C of itself with the CRC field zeroed, and that
//! is not declared as a check. Two things are missing for it: the arithmetic,
//! since nothing here computes Castagnoli's polynomial, and a way to say
//! "these bytes with those four of them read as zero", which no `Covers` can.
//! The field is read and named, and whether it matches is left unanswered
//! rather than answered wrongly.

use crate::formats::mseed;
use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};

/// The fixed header, which is where the source identifier starts in every
/// record. Forty bytes, and the only length in the format that is a constant.
const FIXED: i128 = 40;

/// Where the three lengths that decide how long a record is are written.
const ID_LEN_AT: i128 = 33;
const EXTRA_LEN_AT: i128 = 34;
const DATA_LEN_AT: i128 = 36;

/// How the samples are written. The numbers are the 2.4 ones, and the FDSN
/// carried over only the encodings anybody still writes: the text one, the
/// three fixed-width numeric ones, Steim1 and Steim2. 2 (24-bit integers) and
/// the gain-ranged encodings a 1980s digitiser produced are not in miniSEED 3
/// at all, and 100 is new: bytes the writer wants carried and will not say
/// anything about.
const ENCODING: &[(i128, &str)] = &[
    (0, "text"),
    (1, "16-bit integers"),
    (3, "32-bit integers"),
    (4, "32-bit floats"),
    (5, "64-bit floats"),
    (10, "Steim1"),
    (11, "Steim2"),
    (19, "Steim3"),
    (100, "opaque"),
];

/// What the record says about itself. Three bits of the eight are used, and
/// the rest are reserved: the FDSN moved everything else the 2.4 flag bytes
/// held into the extra headers, where a name says what it means.
const FLAGS: &[(u32, &str)] = &[(0, "calibration signals present"), (1, "time tag questionable"), (2, "clock locked")];

pub fn mseed3() -> Template {
    Template::new("mseed3", T::structure("MiniSEED3", vec![("records", T::repeat(record(), Until::End))]))
}

/// One record, or what is left of a file that stops in the middle of one.
///
/// The last of those is why this is a switch rather than the record itself. A
/// recording cut off in transmission, or a run of samples whose header never
/// arrived, is exactly the file somebody opens a hex editor to look at, and
/// reading a header out of the middle of a Steim frame would fill the panel
/// with fields that are not there.
fn record() -> T {
    let tail = T::structure("MiniSEED3Tail", vec![("bytes", T::bytes(E::Remaining))]);
    // `MS` and a format version of 3, which is the whole of the signature.
    let header = E::peek_at(E::lit(0), 24, Big);
    let is_record = header.clone().less_than(E::lit(0x4D5304)).mul(E::lit(0x4D5302).less_than(header));
    let short = E::Remaining.less_than(E::lit(FIXED));
    T::switch(short.or(E::lit(1).sub(is_record)), vec![(1, tail)], sized())
}

/// A number `bits` wide at `at` bytes into the record, or zero when the record
/// does not reach that far. The caller has already asked whether there are
/// forty bytes to read, so every one of these is inside the header.
fn peek(at: i128, bits: u32) -> E {
    E::peek_at(E::lit(at * 8), bits, Little)
}

/// The record in the room its own header gives it: the fixed forty bytes and
/// the three lengths written at the end of them.
///
/// Asked by peeking rather than by naming the fields, because a size is
/// settled before what is inside it is read. Never past the end of the file,
/// so a record cut off part way reads as far as it goes rather than failing.
fn sized() -> T {
    let length = E::lit(FIXED)
        .add(peek(ID_LEN_AT, 8))
        .add(peek(EXTRA_LEN_AT, 16))
        .add(peek(DATA_LEN_AT, 32))
        .at_most(E::Remaining);
    T::sized(length, body())
}

fn body() -> T {
    T::structure_named(
        "MiniSEED3Record",
        "source_identifier",
        "data",
        vec![
            ("magic", T::magic(b"MS")),
            ("format_version", T::u8()),
            ("flags", T::flags("MiniSEED3Flags", T::u8(), FLAGS)),
            // The start time of the first sample, written as fields rather
            // than as a count from an epoch. The nanosecond comes first
            // because that is where the format puts it, not because anything
            // reads it first.
            ("start_time", start_time()),
            ("encoding", T::enumeration("MiniSEED3Encoding", T::u8(), ENCODING)),
            // Samples per second when positive, and seconds per sample when
            // negative, which is how a rate of one sample a day keeps its
            // precision. Zero when the record carries no time series at all.
            ("sample_rate", T::F64(Little)),
            ("sample_count", T::u32(Little)),
            // Of the whole record with these four bytes read as zero. Not
            // checked here; see the module notes.
            ("crc", T::u32(Little)),
            // Which revision of the data this is, counting from 1 for raw.
            // Only comparable against other records from the same data centre.
            ("publication_version", T::u8()),
            ("identifier_length", T::u8()),
            ("extra_length", T::u16(Little)),
            ("data_length", T::u32(Little)),
            // A URI, and in practice `FDSN:NET_STA_LOC_B_S_SS`.
            ("source_identifier", T::text(StrLen::Fixed(E::field("identifier_length")), Encoding::Ascii)),
            ("extra_headers", extra_headers()),
            ("data", T::sized(E::field("data_length"), data())),
        ],
    )
    .machinery(&["magic", "identifier_length", "extra_length", "data_length"])
    .payload(&["source_identifier", "sample_count"])
    .counted_as("record")
}

/// UTC as its own fields. A second of 60 is a positive leap second, which is
/// the reason the format writes the parts rather than a count.
fn start_time() -> T {
    T::inline_structure(
        "MiniSEED3Time",
        vec![
            ("nanosecond", T::u32(Little)), // 0 to 999,999,999
            ("year", T::u16(Little)),
            ("day", T::u16(Little)), // of the year, 1 to 366
            ("hour", T::u8()),
            ("minute", T::u8()),
            ("second", T::u8()),
        ],
    )
}

/// Whatever the writer had to say that the fixed header has no field for,
/// as JSON. Most records carry none, and a length of zero is not an empty
/// document but no document: read as the nothing it is rather than handed to
/// a parser that would have to fail on it.
fn extra_headers() -> T {
    T::switch(E::field("extra_length"), vec![(0, T::bytes(E::lit(0)))], T::sized(E::field("extra_length"), T::json()))
}

/// The samples, typed by the encoding the header named.
///
/// Read inside the room `data_length` gave, so `Remaining` here is the payload
/// and the shared readers from `mseed` measure against the right end.
///
/// The byte order is the encoding's rather than the record's. Everything the
/// FDSN defined as a number is little-endian like the header; the two Steim
/// encodings keep the big-endian frames they were defined with, which is the
/// one place a miniSEED 3 record is not little-endian throughout.
fn data() -> T {
    T::switch(
        E::field("encoding"),
        vec![
            (0, T::text(StrLen::Fixed(E::Remaining), Encoding::Utf8)),
            (1, mseed::samples(T::Int { bits: 16, endian: Little }, 2)),
            (3, mseed::samples(T::Int { bits: 32, endian: Little }, 4)),
            (4, mseed::samples(T::F32(Little), 4)),
            (5, mseed::samples(T::F64(Little), 8)),
            (10, mseed::steim(Big, false)),
            (11, mseed::steim(Big, true)),
        ],
        // Steim3, which has no fixed width here to go on, and the opaque
        // encoding, which is bytes by definition.
        T::bytes(E::Remaining),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// One record's bytes: the fixed header, the identifier, the extra
    /// headers and the payload, with the three lengths filled in from what was
    /// handed over. Nothing here rounds anything up, which is the point of the
    /// format: a record is as long as its parts add up to.
    fn record_bytes(encoding: u8, id: &str, extra: &str, data: &[u8], samples: u32) -> Vec<u8> {
        let mut b = b"MS\x03".to_vec();
        b.push(0); // flags
        b.extend_from_slice(&123_456_789u32.to_le_bytes()); // nanosecond
        b.extend_from_slice(&2026u16.to_le_bytes());
        b.extend_from_slice(&256u16.to_le_bytes()); // day of the year
        b.extend_from_slice(&[13, 45, 7]); // hour, minute, second
        b.push(encoding);
        b.extend_from_slice(&40.0f64.to_le_bytes()); // samples per second
        b.extend_from_slice(&samples.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes()); // the CRC, left unset
        b.push(1); // publication version
        b.push(id.len() as u8);
        b.extend_from_slice(&(extra.len() as u16).to_le_bytes());
        b.extend_from_slice(&(data.len() as u32).to_le_bytes());
        b.extend_from_slice(id.as_bytes());
        b.extend_from_slice(extra.as_bytes());
        b.extend_from_slice(data);
        b
    }

    fn one_record() -> Vec<u8> {
        let data: Vec<u8> = [1i32, -2, 3].iter().flat_map(|v| v.to_le_bytes()).collect();
        record_bytes(3, "FDSN:XX_TEST__B_H_Z", "", &data, 3)
    }

    #[test]
    fn the_fixed_header_reads_as_forty_bytes_of_little_endian_fields() {
        let d = Document::new(MemSource(one_record()));
        let mut ev = Evaluator::new(mseed3());
        let record = ev.node(&d, &[0, 0]).unwrap();
        assert_eq!(record.type_name, "MiniSEED3Record");
        assert_eq!(ev.node(&d, &[0, 0, 1]).unwrap().value, Value::UInt(3));
        // The time, whose nanosecond is written before its year.
        assert_eq!(ev.node(&d, &[0, 0, 3, 0]).unwrap().value, Value::UInt(123_456_789));
        assert_eq!(ev.node(&d, &[0, 0, 3, 1]).unwrap().value, Value::UInt(2026));
        assert_eq!(ev.node(&d, &[0, 0, 3, 2]).unwrap().value, Value::UInt(256));
        let encoding = ev.node(&d, &[0, 0, 4]).unwrap();
        assert_eq!(encoding.value, Value::Enum { raw: 3, name: Some("32-bit integers".into()), hex: false });
        assert_eq!(ev.node(&d, &[0, 0, 5]).unwrap().value, Value::Float(40.0));
        assert_eq!(ev.node(&d, &[0, 0, 6]).unwrap().value, Value::UInt(3));
        // The identifier, which is the one string a reader looks for.
        assert_eq!(ev.node(&d, &[0, 0, 12]).unwrap().value, Value::Str("FDSN:XX_TEST__B_H_Z".into()));
        assert_eq!(record.name, "[0] FDSN:XX_TEST__B_H_Z");
        // And the samples, which are little-endian here and not a run of bytes.
        let samples = ev.node(&d, &[0, 0, 14, 0]).unwrap();
        assert_eq!((samples.type_name.as_str(), samples.child_count), ("i32 le[]", 3));
        assert_eq!(ev.node(&d, &[0, 0, 14, 0, 1]).unwrap().value, Value::Int(-2));
    }

    #[test]
    fn a_records_length_is_its_three_lengths_and_nothing_rounded_up() {
        // Two records of different lengths, one after the other, which is what
        // no 2.4 file could do: the second only reads if the first was
        // measured from its own header rather than from a power of two.
        let extra = r#"{"FDSN":{"Clock":{"Model":"Acme 3"}}}"#;
        let first = record_bytes(3, "FDSN:XX_TEST__B_H_Z", extra, &[0u8; 12], 3);
        let second = record_bytes(1, "FDSN:YY_OTHER_00_S_H_N", "", &[0u8; 6], 3);
        assert_eq!(first.len(), 40 + 19 + extra.len() + 12);
        let mut b = first.clone();
        b.extend_from_slice(&second);
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(mseed3());
        assert_eq!(ev.node(&d, &[0]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[0, 0]).unwrap().size_bits, first.len() as u64 * 8);
        assert_eq!(ev.node(&d, &[0, 1]).unwrap().size_bits, second.len() as u64 * 8);
        assert_eq!(ev.node(&d, &[0, 1, 12]).unwrap().value, Value::Str("FDSN:YY_OTHER_00_S_H_N".into()));
        // The extra headers of the first, read as the JSON they are.
        let headers = ev.node(&d, &[0, 0, 13]).unwrap();
        assert_eq!(headers.size_bits, extra.len() as u64 * 8);
        assert!(headers.child_count > 0, "the extra headers read as no document");
        // And none at all on the second, which is what most records carry.
        assert_eq!(ev.node(&d, &[0, 1, 13]).unwrap().size_bits, 0);
    }

    #[test]
    fn what_is_not_a_record_is_left_as_bytes() {
        // A whole record and then eleven bytes of something else, which is a
        // file that stopped in the middle of a transmission.
        let mut b = one_record();
        b.extend_from_slice(b"not a recor");
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(mseed3());
        assert_eq!(ev.node(&d, &[0]).unwrap().child_count, 2);
        let tail = ev.node(&d, &[0, 1]).unwrap();
        assert_eq!(tail.type_name, "MiniSEED3Tail");
        assert_eq!(tail.size_bits, 11 * 8);
        // And a version byte that is not 3 is not one of these records either.
        let mut other = one_record();
        other[2] = 2;
        let d = Document::new(MemSource(other));
        let mut ev = Evaluator::new(mseed3());
        assert_eq!(ev.node(&d, &[0, 0]).unwrap().type_name, "MiniSEED3Tail");
    }

    #[test]
    fn a_steim_payload_keeps_the_big_endian_frames_it_was_defined_with() {
        // One 64-byte frame: the two integration constants, and then a word of
        // four 8-bit differences. Word 3's code is bits 25 and 24 of the code
        // word, and the codes for words 1 and 2 are zero because the constants
        // are not differences.
        let mut frame = Vec::new();
        frame.extend_from_slice(&(1u32 << 24).to_be_bytes());
        frame.extend_from_slice(&7i32.to_be_bytes()); // x0
        frame.extend_from_slice(&11i32.to_be_bytes()); // xn
        frame.extend_from_slice(&[1, 2, 255, 0]); // four differences, one of them -1
        frame.resize(64, 0);
        let d = Document::new(MemSource(record_bytes(10, "FDSN:XX_TEST__B_H_Z", "", &frame, 4)));
        let mut ev = Evaluator::new(mseed3());
        // frame0 of the Steim data, and the constants read big-endian.
        assert_eq!(ev.node(&d, &[0, 0, 14, 0, 1]).unwrap().value, Value::Int(7));
        assert_eq!(ev.node(&d, &[0, 0, 14, 0, 2]).unwrap().value, Value::Int(11));
        let word = ev.node(&d, &[0, 0, 14, 0, 3]).unwrap();
        assert_eq!((word.type_name.as_str(), word.child_count), ("Steim1x8", 4));
        assert_eq!(ev.node(&d, &[0, 0, 14, 0, 3, 2]).unwrap().value, Value::Int(-1));
    }
}
