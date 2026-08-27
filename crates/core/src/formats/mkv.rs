//! Matroska: an EBML tree whose element IDs define the media container.
//!
//! EBML's variable-size integers are the essential part of the layout. IDs
//! keep their marker bit; sizes remove it, and an all-ones size means the
//! element continues to the end of its parent. Known master elements recurse
//! into the tree, common scalar elements get their real type, and unfamiliar
//! extensions remain bounded bytes.

use crate::template::{Endian::*, Expr as E, Template, Ty as T, Until};

const IDS: &[(i128, &str)] = &[
    (0x1a45dfa3, "EBML"),
    (0x18538067, "Segment"),
    (0x114d9b74, "SeekHead"),
    (0x1549a966, "Info"),
    (0x1654ae6b, "Tracks"),
    (0xae, "TrackEntry"),
    (0xe0, "Video"),
    (0xe1, "Audio"),
    (0x1f43b675, "Cluster"),
    (0x1c53bb6b, "Cues"),
    (0x1043a770, "Chapters"),
    (0x1941a469, "Attachments"),
    (0x1254c367, "Tags"),
    (0xec, "Void"),
    (0xbf, "CRC-32"),
    (0x4282, "DocType"),
    (0x4287, "DocTypeVersion"),
    (0x4285, "DocTypeReadVersion"),
    (0xd7, "TrackNumber"),
    (0x73c5, "TrackUID"),
    (0x83, "TrackType"),
    (0x86, "CodecID"),
    (0x258688, "CodecName"),
    (0xb0, "PixelWidth"),
    (0xba, "PixelHeight"),
    (0xb5, "SamplingFrequency"),
    (0x9f, "Channels"),
    (0xe7, "Timestamp"),
    (0xa3, "SimpleBlock"),
];

const MASTERS: &[i128] = &[
    0x1a45dfa3, 0x18538067, 0x114d9b74, 0x4dbb, 0x1549a966, 0x1654ae6b, 0xae, 0xe0, 0xe1,
    0x1f43b675, 0x1c53bb6b, 0xbb, 0xb7, 0x1043a770, 0x45b9, 0xb6, 0x1941a469, 0x61a7, 0x1254c367,
    0x7373, 0x67c8,
];

const UINTS: &[i128] = &[
    0x4286, 0x42f7, 0x42f2, 0x42f3, 0x4287, 0x4285, 0x2ad7b1, 0xd7, 0x73c5, 0x83, 0xb0, 0xba,
    0x54b0, 0x54ba, 0x9f, 0x6264, 0xe7,
];

const STRINGS: &[i128] = &[
    0x4282, 0x4d80, 0x5741, 0x7ba9, 0x536e, 0x22b59c, 0x86, 0x258688,
];
const FLOATS: &[i128] = &[0x4489, 0xb5, 0x78b5];

pub fn mkv() -> Template {
    Template::new("mkv", T::repeat(T::Named("Element".into()), Until::End))
        .with_type("Element", element())
}

fn element() -> T {
    T::structure_named(
        "Element",
        "id",
        "data",
        vec![
            ("id", T::enumeration_hex("ElementID", T::ebml_id(), IDS)),
            ("size", T::ebml_size()),
            ("data", data()),
        ],
    )
    .counted_as("element")
}

fn data() -> T {
    // Every all-ones VINT is the unknown-size sentinel, regardless of width.
    let unknown = [
        0x7f,
        0x3fff,
        0x1f_ffff,
        0x0fff_ffff,
        0x07_ffff_ffff,
        0x03ff_ffff_ffff,
        0x01ff_ffff_ffff_ffff,
        0x00ff_ffff_ffff_ffff,
    ];
    let cases = unknown
        .into_iter()
        .map(|n| (n, payload(E::Remaining)))
        .collect();
    T::switch(
        E::field("size"),
        cases,
        T::sized(E::field("size"), payload(E::field("size"))),
    )
}

fn payload(len: E) -> T {
    let mut cases: Vec<(i128, T)> = MASTERS
        .iter()
        .map(|&id| (id, T::repeat(T::Named("Element".into()), Until::End)))
        .collect();
    cases.extend(UINTS.iter().map(|&id| (id, uint(len.clone()))));
    cases.extend(STRINGS.iter().map(|&id| (id, T::utf8(len.clone()))));
    cases.extend(FLOATS.iter().map(|&id| (id, float(len.clone()))));
    T::switch(E::field("id"), cases, T::bytes(len))
}

fn uint(len: E) -> T {
    T::switch(
        len,
        (1..=8)
            .map(|n| {
                (
                    n,
                    T::UInt {
                        bits: n as u32 * 8,
                        endian: Big,
                    },
                )
            })
            .collect(),
        T::bytes(E::Remaining),
    )
}

fn float(len: E) -> T {
    T::switch(
        len,
        vec![(4, T::F32(Big)), (8, T::F64(Big))],
        T::bytes(E::Remaining),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    #[test]
    fn ebml_header_and_unknown_sized_segment_form_a_tree() {
        let bytes = vec![
            0x1a, 0x45, 0xdf, 0xa3, 0x8c, // EBML, 12 bytes
            0x42, 0x82, 0x40, 0x08, b'm', b'a', b't', b'r', b'o', b's', b'k', b'a', 0x18, 0x53,
            0x80, 0x67, 0xff, // Segment, unknown size
            0x15, 0x49, 0xa9, 0x66, 0x85, 0x2a, 0xd7, 0xb1, 0x81, 0x01,
        ];
        let doc = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(mkv());
        assert_eq!(
            ev.node(&doc, &[0, 0]).unwrap().value,
            Value::Enum {
                raw: 0x1a45dfa3,
                name: Some("EBML".into()),
                hex: true
            }
        );
        assert_eq!(
            ev.node(&doc, &[0, 2, 0, 2]).unwrap().value,
            Value::Str("matroska".into())
        );
        assert_eq!(ev.node(&doc, &[1, 1]).unwrap().value, Value::UInt(0x7f));
        assert_eq!(
            ev.node(&doc, &[1, 2, 0, 2, 0, 2]).unwrap().value,
            Value::UInt(1)
        );
    }
}
