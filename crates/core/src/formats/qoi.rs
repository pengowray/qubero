//! QOI, the Quite OK Image format: a fourteen-byte header, a stream of
//! one-byte-tagged chunks, and eight bytes marking the end.
//!
//! The encoding is the whole point of the format, and it is small enough to
//! say outright. Every chunk starts with a two-bit tag, and what follows in
//! the same byte is the payload: an index into the sixty-four colours seen
//! recently, a difference of minus two to one on each channel, a larger
//! difference expressed against the green one, or a run of up to sixty-two
//! copies of the last colour. Two whole bytes are kept back, 0xfe and 0xff,
//! for a colour written out in full, which is why a run stops at sixty-two
//! rather than sixty-four.
//!
//! So the chunk is a switch on the whole first byte for those two, and a
//! switch on its top two bits for everything else. The bits below the tag are
//! fields of two, four and six bits, which is what the encoding is: nothing
//! here is padded out to a byte.
//!
//! The end marker is seven zero bytes and a one. Those bytes are legal chunks
//! too, so nothing tells them apart from the stream except being last: the
//! chunks are read in a window of everything but the final eight bytes.

use crate::template::{Endian::*, Expr as E, Template, Ty as T, Until};

/// What the two bits at the top of a chunk say the rest of it is.
const TAG: &[(i128, &str)] = &[(0, "index"), (1, "diff"), (2, "luma"), (3, "run")];

/// Whether the colours are sRGB or were written with no transfer curve at all.
/// The format says a decoder may ignore this, and every one of them does.
const COLOURSPACE: &[(i128, &str)] = &[(0, "srgb with linear alpha"), (1, "all linear")];

pub fn qoi() -> Template {
    Template::new(
        "qoi",
        T::structure(
            "QOI",
            vec![
                ("magic", T::magic(b"qoif")),
                ("width", T::u32(Big)),
                ("height", T::u32(Big)),
                ("channels", T::enumeration("Channels", T::u8(), &[(3, "rgb"), (4, "rgba")])),
                ("colourspace", T::enumeration("Colourspace", T::u8(), COLOURSPACE)),
                // Everything but the last eight bytes, which are the marker.
                ("chunks", T::sized(E::Remaining.sub(E::lit(8)), T::repeat(chunk(), Until::End))),
                ("end_marker", T::magic(&[0, 0, 0, 0, 0, 0, 0, 1])),
            ],
        ),
    )
}

/// One chunk. The two bytes that mean a colour written out in full are looked
/// for first, since both of them also carry the tag that means a run.
fn chunk() -> T {
    T::switch(E::peek(8, Big), vec![(0xfe, rgb()), (0xff, rgba())], tagged())
}

/// A colour in full, with the alpha of the one before it.
fn rgb() -> T {
    T::inline_structure(
        "Rgb",
        vec![("tag", T::magic(&[0xfe])), ("r", T::u8()), ("g", T::u8()), ("b", T::u8())],
    )
    .counted_as("chunk")
}

/// A colour in full, alpha included.
fn rgba() -> T {
    T::inline_structure(
        "Rgba",
        vec![("tag", T::magic(&[0xff])), ("r", T::u8()), ("g", T::u8()), ("b", T::u8()), ("a", T::u8())],
    )
    .counted_as("chunk")
}

/// The four chunks whose payload shares a byte with the tag that names it.
fn tagged() -> T {
    T::inline_structure(
        "Chunk",
        vec![
            ("tag", T::enumeration("Tag", T::UInt { bits: 2, endian: Big }, TAG)),
            (
                "body",
                T::switch(E::field("tag"), vec![(0, index()), (1, diff()), (2, luma())], run()),
            ),
        ],
    )
    .counted_as("chunk")
}

/// Which of the sixty-four colours seen recently this pixel is. The table is
/// not written down anywhere: both ends work out the same index from the
/// colour itself, so a hit costs one byte and a miss costs nothing.
fn index() -> T {
    T::UInt { bits: 6, endian: Big }
}

/// A small difference on each channel, held as two bits with a bias of two, so
/// the range is minus two to one. Alpha does not change.
fn diff() -> T {
    T::inline_structure(
        "Diff",
        vec![
            ("dr", T::UInt { bits: 2, endian: Big }),
            ("dg", T::UInt { bits: 2, endian: Big }),
            ("db", T::UInt { bits: 2, endian: Big }),
        ],
    )
}

/// A larger difference, written against green. Green moves by minus thirty-two
/// to thirty-one, and red and blue by how much further they moved than green
/// did. That is the whole idea of the format in one chunk: colours that change
/// together cost less than colours that change apart.
fn luma() -> T {
    T::inline_structure(
        "Luma",
        vec![
            ("dg", T::UInt { bits: 6, endian: Big }),
            ("dr_dg", T::UInt { bits: 4, endian: Big }),
            ("db_dg", T::UInt { bits: 4, endian: Big }),
        ],
    )
}

/// How many more times to write the last colour, with a bias of one. 62 and 63
/// are not runs: those are the two bytes that mean a colour in full.
fn run() -> T {
    T::UInt { bits: 6, endian: Big }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn qoi_bytes(body: &[u8]) -> Vec<u8> {
        let mut v = b"qoif".to_vec();
        v.extend_from_slice(&2u32.to_be_bytes());
        v.extend_from_slice(&2u32.to_be_bytes());
        v.extend_from_slice(&[4, 0]);
        v.extend_from_slice(body);
        v.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1]);
        v
    }

    #[test]
    fn the_header_reads_and_the_marker_ends_the_stream() {
        let d = Document::new(MemSource(qoi_bytes(&[0xff, 1, 2, 3, 4])));
        let mut ev = Evaluator::new(qoi());
        assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::UInt(2));
        assert_eq!(
            ev.node(&d, &[3]).unwrap().value,
            Value::Enum { raw: 4, name: Some("rgba".into()), hex: false }
        );
        let chunks = ev.node(&d, &[5]).unwrap();
        assert_eq!(chunks.offset_bits, 14 * 8);
        assert_eq!(chunks.size_bits, 5 * 8);
        assert_eq!(chunks.child_count, 1);
        let marker = ev.node(&d, &[6]).unwrap();
        assert_eq!(marker.value, Value::Magic { ok: true, bytes: vec![0, 0, 0, 0, 0, 0, 0, 1], expected: vec![0, 0, 0, 0, 0, 0, 0, 1] });
    }

    #[test]
    fn each_chunk_reads_as_what_its_first_two_bits_say() {
        // A colour in full, an index, a small difference, a luma and a run.
        let body = [
            0xfe, 0x11, 0x22, 0x33, // rgb
            0b00_001010, // index 10
            0b01_10_01_11, // diff: dr 2, dg 1, db 3
            0b10_100000, 0b0111_1001, // luma: dg 32, dr-dg 7, db-dg 9
            0b11_000100, // run of 5, written as 4
        ];
        let d = Document::new(MemSource(qoi_bytes(&body)));
        let mut ev = Evaluator::new(qoi());
        let chunks = ev.node(&d, &[5]).unwrap();
        assert_eq!(chunks.child_count, 5);

        // The colour in full, which is a whole byte rather than a tag.
        assert_eq!(ev.node(&d, &[5, 0]).unwrap().type_name, "Rgb");
        assert_eq!(ev.node(&d, &[5, 0, 2]).unwrap().value, Value::UInt(0x22));

        assert_eq!(
            ev.node(&d, &[5, 1, 0]).unwrap().value,
            Value::Enum { raw: 0, name: Some("index".into()), hex: false }
        );
        assert_eq!(ev.node(&d, &[5, 1, 1]).unwrap().value, Value::UInt(10));

        // Three fields of two bits, sharing a byte with the tag.
        assert_eq!(ev.node(&d, &[5, 2, 1]).unwrap().type_name, "Diff");
        assert_eq!(ev.node(&d, &[5, 2, 1, 0]).unwrap().value, Value::UInt(2));
        assert_eq!(ev.node(&d, &[5, 2, 1, 2]).unwrap().value, Value::UInt(3));

        // Luma takes two bytes, and the second is two nibbles.
        let l = ev.node(&d, &[5, 3, 1]).unwrap();
        assert_eq!(l.type_name, "Luma");
        assert_eq!(l.size_bits, 14);
        assert_eq!(ev.node(&d, &[5, 3, 1, 0]).unwrap().value, Value::UInt(32));
        assert_eq!(ev.node(&d, &[5, 3, 1, 2]).unwrap().value, Value::UInt(9));

        assert_eq!(ev.node(&d, &[5, 4, 1]).unwrap().value, Value::UInt(4));
        assert_eq!(ev.node(&d, &[5, 4]).unwrap().size_bits, 8);
    }
}
