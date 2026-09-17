//! AIFF, and the compressed AIFC that grew out of it. Apple's answer to what
//! became WAV, on the IFF frame the Amiga had already published.
//!
//! The sample rate is the reason this format is interesting: it is an 80-bit
//! extended float, the native long double of the 68881 the machine had, which
//! nothing else in common use writes. The IR reads one now, so the field says
//! 44100 rather than sitting there as ten bytes nobody can spend.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, TableShape, Template, Time, Ty as T};

use super::iff::{cc, chunk_text, iff};

pub fn aiff() -> Template {
    iff("aiff", body())
}

fn body() -> T {
    T::switch(
        E::field("id"),
        vec![
            (cc("COMM"), comm()),
            (cc("SSND"), ssnd()),
            (cc("FVER"), fver()),
            (cc("NAME"), chunk_text()),
            (cc("AUTH"), chunk_text()),
            (cc("ANNO"), chunk_text()),
            (cc("(c) "), chunk_text()),
            (cc("MARK"), markers()),
        ],
        T::bytes(E::Remaining),
    )
}

/// What the samples are. Everything after the rate is AIFC only, and an AIFF
/// chunk simply ends before it.
fn comm() -> T {
    T::structure(
        "Common",
        vec![
            ("channels", T::u16(Big)),
            ("frames", T::u32(Big)),
            ("sample_size", T::u16(Big)),
            // 80-bit extended: a sign, fifteen bits of exponent and a
            // sixty-four bit significand with its leading one written out.
            ("sample_rate", T::F80(Big)),
            // AIFC adds a four-character compression id and then a Pascal
            // string naming it for a person. An AIFF chunk ends before both,
            // so the id is as much of four bytes as the chunk has left, which
            // there is none: a switch reading it gets nought, which is the
            // same answer as no chunk at all and takes the same branch.
            ("compression", T::text(StrLen::Fixed(E::Remaining.at_most(E::lit(4))), Encoding::Ascii)),
            ("compression_name", T::bytes(E::Remaining)),
        ],
    )
}

/// Where the samples are. `offset` is the room left before them so a player
/// can align a block, and it is almost always zero.
fn ssnd() -> T {
    T::structure(
        "SoundData",
        vec![
            ("offset", T::u32(Big)),
            ("block_size", T::u32(Big)),
            ("samples", sample_table()),
        ],
    )
}

/// The samples, read as what the `COMM` chunk earlier in the file said they
/// are. `COMM` is a sibling chunk rather than a field of this one, and a
/// `FVER`, a `NAME` or a `MARK` can sit between the two, so the width is asked
/// of the nearest earlier chunk that declares one.
///
/// AIFF samples are two's-complement signed at every width, eight bits
/// included, which is where this differs from WAV: a WAV's 8-bit samples are
/// unsigned with 128 for silence.
///
/// AIFC keeps the same frame and names a compression in front of the samples.
/// Two of the four in common use are not compression at all: `sowt` is the
/// same PCM with its bytes the other way round, which is what a little-endian
/// machine writes, and `fl32`/`fl64` are floats. A compression nobody here
/// reads keeps its bytes as bytes.
fn samples() -> T {
    let raw = || T::bytes(E::Remaining);
    let run = |width: i128, elem: T| T::array(elem, E::Remaining.div(E::lit(width)));
    let by_width = |endian| {
        T::switch(
            E::sibling(&["body", "sample_size"]),
            vec![
                (8, run(1, T::Int { bits: 8, endian })),
                (16, run(2, T::Int { bits: 16, endian })),
                (24, run(3, T::Int { bits: 24, endian })),
                (32, run(4, T::Int { bits: 32, endian })),
            ],
            raw(),
        )
    };
    T::switch(
        E::sibling(&["body", "compression"]),
        vec![
            // No compression field at all, which is what an AIFF has.
            (0, by_width(Big)),
            (cc("NONE"), by_width(Big)),
            (cc("sowt"), by_width(Little)),
            (cc("fl32"), run(4, T::F32(Big))),
            (cc("FL32"), run(4, T::F32(Big))),
            (cc("fl64"), run(8, T::F64(Big))),
            (cc("FL64"), run(8, T::F64(Big))),
        ],
        raw(),
    )
}

/// The samples, and what makes a table of them: the channels interleaved into
/// rows, the rate those rows come at, and the `COMM` fields a reader wants to
/// see above the table.
///
/// A structure of one field, for the reason `formats::wav` gives: the run is a
/// switch with an array at the end of each branch, and a switch has no field
/// for the shape to sit on.
fn sample_table() -> T {
    let shape = TableShape {
        // Interleaved: one row is one sample of each channel, which AIFF
        // calls a frame.
        columns: Some(E::sibling(&["body", "channels"])),
        names: vec!["left".into(), "right".into()],
        units: vec!["".into(), "".into()],
        column_word: Some("channel".into()),
        row_word: Some("sample".into()),
        // An 80-bit extended float, and 44100.0 of them a second is a rate.
        rate: Some(E::sibling(&["body", "sample_rate"])),
        facts: vec![
            E::sibling(&["body", "channels"]),
            E::sibling(&["body", "sample_size"]),
            E::sibling(&["body", "sample_rate"]),
            E::sibling(&["body", "compression"]),
        ],
    };
    T::structure("Samples", vec![("samples", samples())]).field_table("samples", shape)
}

/// AIFC's version, written as a date: 0xA2805140 is 23 May 1990, and it is the
/// only value the format has ever had.
///
/// The date is seconds from 1904 like every other time a Mac format writes,
/// so it reads as one rather than only as a number a table names. It was
/// wrong here in two places and in two different ways, saying 1991 in the
/// enumeration and 22 May 1991 in this comment; the arithmetic says
/// 1990-05-23 14:40:00 and the AIFF-C specification says May 23, 1990.
fn fver() -> T {
    T::structure(
        "Version",
        vec![("timestamp", T::enumeration("AifcVersion", T::u32(Big), &[(0xa280_5140, "AIFC version 1")]))],
    )
    .field_time("timestamp", Time::mac().local())
}

/// Named points in the sound, which is what a sampler loops between.
fn markers() -> T {
    let marker = T::structure(
        "Marker",
        vec![
            ("id", T::u16(Big)),
            ("position", T::u32(Big)),
            ("name_length", T::u8()),
            ("name", T::utf8(E::field("name_length"))),
            // A pstring is padded to an even total, and the length byte
            // counts towards it: one pad byte when the name is even, none
            // when it is odd.
            ("pad", T::bytes(E::field("name_length").add(E::lit(1)).pad_to(2))),
        ],
    )
    .counted_as("marker");
    T::structure("Markers", vec![("count", T::u16(Big)), ("markers", T::array(marker, E::field("count")))])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = id.to_vec();
        v.extend_from_slice(&(body.len() as u32).to_be_bytes());
        v.extend_from_slice(body);
        if body.len() % 2 == 1 {
            v.push(0);
        }
        v
    }

    fn file() -> Vec<u8> {
        let mut comm = 2u16.to_be_bytes().to_vec();
        comm.extend_from_slice(&1000u32.to_be_bytes());
        comm.extend_from_slice(&16u16.to_be_bytes());
        // 44100 as an 80-bit extended float.
        comm.extend_from_slice(&[0x40, 0x0e, 0xac, 0x44, 0, 0, 0, 0, 0, 0]);

        let mut chunks = chunk(b"COMM", &comm);
        chunks.extend_from_slice(&chunk(b"NAME", b"Sample"));
        chunks.extend_from_slice(&chunk(b"SSND", &[0u8; 8 + 16]));

        let mut v = b"FORM".to_vec();
        v.extend_from_slice(&((4 + chunks.len()) as u32).to_be_bytes());
        v.extend_from_slice(b"AIFF");
        v.extend_from_slice(&chunks);
        v
    }

    #[test]
    fn the_chunks_read_and_the_sizes_are_big_endian() {
        let d = Document::new(MemSource(file()));
        let mut ev = Evaluator::new(aiff());
        assert_eq!(ev.node(&d, &[3]).unwrap().child_count, 3);
        assert_eq!(ev.node(&d, &[3, 0, 2, 0]).unwrap().value, Value::UInt(2));
        assert_eq!(ev.node(&d, &[3, 0, 2, 1]).unwrap().value, Value::UInt(1000));
        let rate = ev.node(&d, &[3, 0, 2, 3]).unwrap();
        assert_eq!(rate.size_bits, 10 * 8);
        assert_eq!(rate.value, Value::Float(44100.0));
        assert_eq!(rate.type_name, "f80 be");
        // Eighty bits is the first float width here that is not a power of
        // two, so writing one is worth proving rather than assuming.
        assert!(rate.editable);
        let w = ev.prepare_write(&d, &[3, 0, 2, 3], "48000").unwrap();
        assert_eq!(w.n_bits, 80);
        assert_eq!(w.data, vec![0x40, 0x0e, 0xbb, 0x80, 0, 0, 0, 0, 0, 0]);
        assert_eq!(ev.node(&d, &[3, 1, 2]).unwrap().value, Value::Str("Sample".into()));
        assert_eq!(ev.node(&d, &[3, 2, 2, 2]).unwrap().size_bits, 16 * 8);
    }

    /// A FORM of a COMM and an SSND, with the COMM's AIFC tail and the sound
    /// bytes given. The tail is empty for a plain AIFF.
    fn sound(channels: u16, bits: u16, tail: &[u8], data: &[u8]) -> Vec<u8> {
        let mut comm = channels.to_be_bytes().to_vec();
        comm.extend_from_slice(&((data.len() / 2) as u32).to_be_bytes());
        comm.extend_from_slice(&bits.to_be_bytes());
        comm.extend_from_slice(&[0x40, 0x0e, 0xac, 0x44, 0, 0, 0, 0, 0, 0]); // 44100
        comm.extend_from_slice(tail);

        let mut ssnd = vec![0u8; 8];
        ssnd.extend_from_slice(data);

        let mut chunks = chunk(b"COMM", &comm);
        chunks.extend_from_slice(&chunk(b"SSND", &ssnd));
        let mut v = b"FORM".to_vec();
        v.extend_from_slice(&((4 + chunks.len()) as u32).to_be_bytes());
        v.extend_from_slice(if tail.is_empty() { b"AIFF" } else { b"AIFC" });
        v.extend_from_slice(&chunks);
        v
    }

    /// Where the run of samples sits: the SSND is the second chunk, its body's
    /// third field is the wrapper, and the run is the wrapper's only field.
    const SAMPLES: [usize; 5] = [3, 1, 2, 2, 0];

    #[test]
    fn the_samples_read_as_a_table_of_a_sample_of_each_channel() {
        let mut data = Vec::new();
        for s in [1i16, -1, 2, -2, 3, -3, 4, -4] {
            data.extend_from_slice(&s.to_be_bytes());
        }
        let d = Document::new(MemSource(sound(2, 16, &[], &data)));
        let mut ev = Evaluator::new(aiff());

        let node = ev.node(&d, &SAMPLES).unwrap();
        assert!(node.table, "the samples should read as a table");
        assert_eq!(node.type_name, "i16 be[]");
        assert_eq!(node.child_count, 8);
        assert_eq!(ev.node(&d, &[3, 1, 2, 2, 0, 1]).unwrap().value, Value::Int(-1));

        let shape = ev.table_shape(&d, &SAMPLES).unwrap().expect("a shape");
        assert_eq!(shape.columns, Some(2));
        // The rate is the 80-bit extended float the COMM holds, and 44100.0
        // of anything a second is a rate of 44100.
        assert_eq!(shape.rate, Some(44100));
        assert_eq!(shape.row_word.as_deref(), Some("sample"));
        assert_eq!(shape.column_word.as_deref(), Some("channel"));
        let labels: Vec<&str> = shape.facts.iter().map(|f| f.label.as_str()).collect();
        assert_eq!(labels, ["body.channels", "body.sample_size", "body.sample_rate", "body.compression"]);
        for f in &shape.facts {
            assert!(!f.path.is_empty(), "{} has nowhere to go", f.label);
        }

        // One sample of the run is not a table of its own.
        assert!(!ev.node(&d, &[3, 1, 2, 2, 0, 0]).unwrap().table);
        assert_eq!(ev.table_shape(&d, &[3, 1, 2, 2, 0, 0]).unwrap(), None);
    }

    #[test]
    fn an_aifc_reads_its_samples_as_the_compression_says() {
        // A Pascal string naming the compression follows the four-character
        // id, padded to an even length, and it is empty in all of these.
        let tail = |id: &[u8; 4]| {
            let mut t = id.to_vec();
            t.extend_from_slice(&[0, 0]);
            t
        };
        for (id, bits, name, children) in [
            (b"NONE", 16u16, "i16 be[]", 8),
            (b"sowt", 16, "i16 le[]", 8),
            (b"fl32", 32, "f32 be[]", 4),
            (b"FL32", 32, "f32 be[]", 4),
            (b"fl64", 64, "f64 be[]", 2),
            (b"FL64", 64, "f64 be[]", 2),
        ] {
            let d = Document::new(MemSource(sound(1, bits, &tail(id), &[0u8; 16])));
            let mut ev = Evaluator::new(aiff());
            let what = String::from_utf8_lossy(id).to_string();
            let node = ev.node(&d, &SAMPLES).unwrap();
            assert_eq!(node.type_name, name, "{what}");
            assert_eq!(node.child_count, children, "{what}");
            assert_eq!(ev.table_shape(&d, &SAMPLES).unwrap().expect("a shape").columns, Some(1), "{what}");
        }
        // Real compression, which nothing here decodes: the bytes stay bytes.
        let d = Document::new(MemSource(sound(1, 16, &tail(b"ima4"), &[0u8; 16])));
        let mut ev = Evaluator::new(aiff());
        assert_eq!(ev.node(&d, &SAMPLES).unwrap().type_name, "bytes[]");
    }

    #[test]
    fn a_width_the_common_chunk_names_is_the_width_the_samples_are_read_at() {
        for (bits, name, children) in [(8u16, "i8[]", 16), (24, "i24 be[]", 5), (32, "i32 be[]", 4)] {
            let d = Document::new(MemSource(sound(1, bits, &[], &[0u8; 16])));
            let mut ev = Evaluator::new(aiff());
            let node = ev.node(&d, &SAMPLES).unwrap();
            assert_eq!(node.type_name, name, "{bits} bits");
            assert_eq!(node.child_count, children, "{bits} bits");
        }
    }

    #[test]
    fn a_marker_name_is_padded_to_an_even_total_with_its_length_byte() {
        // Two markers: one four-letter name, which needs a pad byte, and one
        // five-letter name, which does not.
        let mut body = 2u16.to_be_bytes().to_vec();
        for (id, at, name) in [(1u16, 0u32, "loop"), (2, 44100, "start")] {
            body.extend_from_slice(&id.to_be_bytes());
            body.extend_from_slice(&at.to_be_bytes());
            body.push(name.len() as u8);
            body.extend_from_slice(name.as_bytes());
            if name.len() % 2 == 0 {
                body.push(0);
            }
        }
        let chunks = chunk(b"MARK", &body);
        let mut v = b"FORM".to_vec();
        v.extend_from_slice(&((4 + chunks.len()) as u32).to_be_bytes());
        v.extend_from_slice(b"AIFF");
        v.extend_from_slice(&chunks);

        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(aiff());
        assert_eq!(ev.node(&d, &[3, 0, 2, 1]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[3, 0, 2, 1, 0, 3]).unwrap().value, Value::Str("loop".into()));
        assert_eq!(ev.node(&d, &[3, 0, 2, 1, 0, 4]).unwrap().size_bits, 8);
        assert_eq!(ev.node(&d, &[3, 0, 2, 1, 1, 3]).unwrap().value, Value::Str("start".into()));
        assert_eq!(ev.node(&d, &[3, 0, 2, 1, 1, 4]).unwrap().size_bits, 0);
        assert_eq!(ev.node(&d, &[3, 0, 2, 1, 1, 1]).unwrap().value, Value::UInt(44100));
    }

    #[test]
    fn an_odd_sized_chunk_is_followed_by_a_pad_byte() {
        let mut chunks = chunk(b"ANNO", b"odd");
        chunks.extend_from_slice(&chunk(b"AUTH", b"me"));
        let mut v = b"FORM".to_vec();
        v.extend_from_slice(&((4 + chunks.len()) as u32).to_be_bytes());
        v.extend_from_slice(b"AIFF");
        v.extend_from_slice(&chunks);

        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(aiff());
        assert_eq!(ev.node(&d, &[3]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[3, 0, 3]).unwrap().size_bits, 8);
        assert_eq!(ev.node(&d, &[3, 1, 2]).unwrap().value, Value::Str("me".into()));
    }
}
