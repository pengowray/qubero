//! AIFF, and the compressed AIFC that grew out of it. Apple's answer to what
//! became WAV, on the IFF frame the Amiga had already published.
//!
//! The sample rate is the reason this format is interesting: it is an 80-bit
//! extended float, the 68881's native long double, which nothing else in
//! common use writes and which no type here can read. It stays ten bytes, and
//! that is a gap in the IR rather than in the format.

use crate::template::{Endian::*, Expr as E, Template, Ty as T};

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
            ("sample_rate", T::bytes(E::lit(10))),
            ("compression", T::bytes(E::Remaining)),
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
            ("samples", T::bytes(E::Remaining)),
        ],
    )
}

/// AIFC's version, written as a date: 0xA2805140 is 22 May 1991, and it is the
/// only value the format has ever had.
fn fver() -> T {
    T::structure(
        "Version",
        vec![("timestamp", T::enumeration("AifcVersion", T::u32(Big), &[(0xa280_5140, "1991-05-23")]))],
    )
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
            // when it is odd. There is no modulo, so it is written out.
            (
                "pad",
                T::bytes({
                    let used = E::field("name_length").add(E::lit(1));
                    used.clone().sub(used.div(E::lit(2)).mul(E::lit(2)))
                }),
            ),
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
        assert_eq!(ev.node(&d, &[3, 0, 2, 3]).unwrap().size_bits, 10 * 8);
        assert_eq!(ev.node(&d, &[3, 1, 2]).unwrap().value, Value::Str("Sample".into()));
        assert_eq!(ev.node(&d, &[3, 2, 2, 2]).unwrap().size_bits, 16 * 8);
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
