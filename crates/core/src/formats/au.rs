//! Sun/NeXT audio: six big-endian numbers, an optional line of text, and the
//! samples. The oldest audio format still in use, and the one that puts its
//! numbers the way the machine that invented it did.
//!
//! A data size of 0xffffffff means "to the end of the file", which is what a
//! program piping audio out writes when it does not know how much there will
//! be. The samples run to the end here either way, so nothing has to special
//! case it.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, TableShape, Template, Ty as T};

/// The encodings, which is really a list of every way a telephone company has
/// written a sample. 1 is mu-law, the one an `.au` almost always is.
const ENCODING: &[(i128, &str)] = &[
    (1, "8-bit mu-law"),
    (2, "8-bit pcm"),
    (3, "16-bit pcm"),
    (4, "24-bit pcm"),
    (5, "32-bit pcm"),
    (6, "32-bit float"),
    (7, "64-bit float"),
    (23, "4-bit adpcm g721"),
    (24, "8-bit adpcm g722"),
    (25, "3-bit adpcm g723"),
    (26, "5-bit adpcm g723"),
    (27, "8-bit a-law"),
];

pub fn au() -> Template {
    Template::new(
        "au",
        T::structure(
            "AU",
            vec![
                ("magic", T::magic(b".snd")),
                ("data_offset", T::u32(Big)),
                ("data_size", T::u32(Big)),
                ("encoding", T::enumeration("Encoding", T::u32(Big), ENCODING)),
                ("sample_rate", T::u32(Big)),
                ("channels", T::u32(Big)),
                // The header is at least 24 bytes and the rest of it up to
                // data_offset is a comment, NUL padded.
                ("annotation", T::text(StrLen::Padded { size: E::field("data_offset").sub(E::lit(24)), pad: 0 }, Encoding::Ascii)),
                ("samples", sample_table()),
            ],
        ),
    )
    // The two companded encodings are a byte each, and the byte is not the
    // number the sample is: 0xff is the quietest mu-law code, not -1. Nothing
    // here decodes them, so the run says which companding it is rather than
    // calling them plain bytes and letting a reader take them for amplitudes.
    .with_type("MuLaw", T::u8())
    .with_type("ALaw", T::u8())
}

/// The samples, read as what `encoding` says they are.
///
/// The header is the enclosing structure here rather than an earlier chunk, so
/// a plain field name reaches it from inside the wrapper: the walk that
/// answers one climbs out of `Samples` into `AU` and finds the fields declared
/// before `samples`.
///
/// Samples are interleaved, one of each channel in turn, and the width is
/// settled once above the list so the millionth sample can be reached without
/// reading the ones before it. An encoding nobody here reads keeps its bytes
/// as bytes rather than being read as something it was never said to be:
/// that is every ADPCM, whose samples are not a whole number of bytes and are
/// worth nothing until the state machine in front of them has run.
fn samples() -> T {
    let raw = || T::bytes(E::Remaining);
    let run = |width: i128, elem: T| T::array(elem, E::Remaining.div(E::lit(width)));
    // Linear PCM is signed at every width here, eight bits included, which is
    // where this differs from WAV.
    let pcm = |bits: u32| T::Int { bits, endian: Big };
    T::switch(
        E::field("encoding"),
        vec![
            (1, run(1, T::Named("MuLaw".into()))),
            (2, run(1, pcm(8))),
            (3, run(2, pcm(16))),
            (4, run(3, pcm(24))),
            (5, run(4, pcm(32))),
            (6, run(4, T::F32(Big))),
            (7, run(8, T::F64(Big))),
            (27, run(1, T::Named("ALaw".into()))),
        ],
        raw(),
    )
}

/// The samples, and what makes a table of them: the channels interleaved into
/// rows, the rate those rows come at, and the header fields a reader wants to
/// see above the table.
///
/// A structure of one field, for the reason `formats::wav` gives: the run is a
/// switch on the encoding with an array at the end of each branch, and a
/// switch has no field for the shape to sit on.
fn sample_table() -> T {
    let shape = TableShape {
        // Interleaved: one row is one sample of each channel.
        columns: Some(E::field("channels")),
        names: vec!["left".into(), "right".into()],
        units: vec!["".into(), "".into()],
        column_word: Some("channel".into()),
        row_word: Some("sample".into()),
        rate: Some(E::field("sample_rate")),
        facts: vec![E::field("encoding"), E::field("sample_rate"), E::field("channels")],
    };
    T::structure("Samples", vec![("samples", samples())]).field_table("samples", shape)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    #[test]
    fn the_annotation_fills_the_room_the_offset_leaves() {
        let mut v = b".snd".to_vec();
        v.extend_from_slice(&32u32.to_be_bytes()); // data starts at 32
        v.extend_from_slice(&u32::MAX.to_be_bytes()); // size unknown
        v.extend_from_slice(&1u32.to_be_bytes()); // mu-law
        v.extend_from_slice(&8000u32.to_be_bytes());
        v.extend_from_slice(&1u32.to_be_bytes());
        v.extend_from_slice(b"hi\0\0\0\0\0\0");
        v.extend_from_slice(&[0xff; 16]);

        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(au());
        assert_eq!(ev.node(&d, &[4]).unwrap().value, Value::UInt(8000));
        assert_eq!(ev.node(&d, &[6]).unwrap().value, Value::Str("hi".into()));
        assert_eq!(ev.node(&d, &[6]).unwrap().size_bits, 8 * 8);
        assert_eq!(ev.node(&d, &[7]).unwrap().size_bits, 16 * 8);
        // Mu-law is a byte a sample, and the run says which companding rather
        // than calling the bytes amplitudes.
        assert_eq!(ev.node(&d, &[7, 0]).unwrap().type_name, "MuLaw[]");
        assert_eq!(ev.node(&d, &[7, 0]).unwrap().child_count, 16);
    }

    /// A header of the six numbers with no annotation room, followed by the
    /// bytes given.
    fn file(encoding: u32, rate: u32, channels: u32, data: &[u8]) -> Vec<u8> {
        let mut v = b".snd".to_vec();
        v.extend_from_slice(&24u32.to_be_bytes());
        v.extend_from_slice(&(data.len() as u32).to_be_bytes());
        v.extend_from_slice(&encoding.to_be_bytes());
        v.extend_from_slice(&rate.to_be_bytes());
        v.extend_from_slice(&channels.to_be_bytes());
        v.extend_from_slice(data);
        v
    }

    #[test]
    fn the_samples_read_as_a_table_of_a_sample_of_each_channel() {
        // Two channels of 16-bit PCM: four rows of two.
        let mut data = Vec::new();
        for s in [1i16, -1, 2, -2, 3, -3, 4, -4] {
            data.extend_from_slice(&s.to_be_bytes());
        }
        let d = Document::new(MemSource(file(3, 22050, 2, &data)));
        let mut ev = Evaluator::new(au());

        let samples = [7, 0];
        let node = ev.node(&d, &samples).unwrap();
        assert!(node.table, "the samples should read as a table");
        assert_eq!(node.type_name, "i16 be[]");
        assert_eq!(node.child_count, 8);
        assert_eq!(ev.node(&d, &[7, 0, 1]).unwrap().value, Value::Int(-1));

        // The channel count and the rate are fields of the structure around
        // the wrapper, which a plain field name reaches from inside it.
        let shape = ev.table_shape(&d, &samples).unwrap().expect("a shape");
        assert_eq!(shape.columns, Some(2));
        assert_eq!(shape.rate, Some(22050));
        assert_eq!(shape.row_word.as_deref(), Some("sample"));
        assert_eq!(shape.column_word.as_deref(), Some("channel"));
        let labels: Vec<&str> = shape.facts.iter().map(|f| f.label.as_str()).collect();
        assert_eq!(labels, ["encoding", "sample_rate", "channels"]);
        for f in &shape.facts {
            assert!(!f.path.is_empty(), "{} has nowhere to go", f.label);
        }

        // One sample of the run is not a table of its own.
        assert!(!ev.node(&d, &[7, 0, 0]).unwrap().table);
        assert_eq!(ev.table_shape(&d, &[7, 0, 0]).unwrap(), None);
    }

    #[test]
    fn each_encoding_reads_its_own_width_and_an_unknown_one_stays_bytes() {
        let data = [0u8; 24];
        for (encoding, name, children) in [
            (1u32, "MuLaw[]", 24),
            (2, "i8[]", 24),
            (3, "i16 be[]", 12),
            (4, "i24 be[]", 8),
            (5, "i32 be[]", 6),
            (6, "f32 be[]", 6),
            (7, "f64 be[]", 3),
            (27, "ALaw[]", 24),
        ] {
            let d = Document::new(MemSource(file(encoding, 8000, 1, &data)));
            let mut ev = Evaluator::new(au());
            let node = ev.node(&d, &[7, 0]).unwrap();
            assert_eq!(node.type_name, name, "encoding {encoding}");
            assert_eq!(node.child_count, children, "encoding {encoding}");
        }
        // G.721 ADPCM is four bits a sample behind a state machine, so the
        // bytes stay bytes.
        let d = Document::new(MemSource(file(23, 8000, 1, &data)));
        let mut ev = Evaluator::new(au());
        assert_eq!(ev.node(&d, &[7, 0]).unwrap().type_name, "bytes[]");
    }
}
