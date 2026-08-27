//! Raw DV: a stream of fixed 80-byte DIF blocks.
//!
//! Each block starts with a three-byte position ID. Its section type separates
//! headers, subcode, video auxiliary data, audio, and compressed video. Audio
//! blocks expose their five-byte AAUX pack; frame payloads stay compressed.

use crate::template::{Endian::*, Expr as E, Template, Ty as T, Until};

const SECTION: &[(i128, &str)] = &[
    (0, "header"),
    (1, "subcode"),
    (2, "VAUX"),
    (3, "audio"),
    (4, "video"),
];

pub fn dv() -> Template {
    Template::new("dv", T::repeat(block(), Until::End))
}

fn block() -> T {
    T::structure_named(
        "DIFBlock",
        "section_type",
        "body",
        vec![
            (
                "section_type",
                T::enumeration(
                    "SectionType",
                    T::UInt {
                        bits: 3,
                        endian: Big,
                    },
                    SECTION,
                ),
            ),
            (
                "reserved",
                T::UInt {
                    bits: 1,
                    endian: Big,
                },
            ),
            (
                "arbitrary",
                T::UInt {
                    bits: 4,
                    endian: Big,
                },
            ),
            (
                "sequence",
                T::UInt {
                    bits: 4,
                    endian: Big,
                },
            ),
            (
                "channel",
                T::UInt {
                    bits: 1,
                    endian: Big,
                },
            ),
            (
                "channel_pair",
                T::UInt {
                    bits: 1,
                    endian: Big,
                },
            ),
            (
                "reserved2",
                T::UInt {
                    bits: 2,
                    endian: Big,
                },
            ),
            ("block_number", T::u8()),
            (
                "body",
                T::switch(
                    E::field("section_type"),
                    vec![(0, header()), (3, audio())],
                    T::bytes(E::lit(77)),
                ),
            ),
        ],
    )
    .counted_as("DIF block")
}

fn header() -> T {
    T::structure(
        "DIFHeader",
        vec![
            (
                "system",
                T::enumeration(
                    "VideoSystem",
                    T::UInt {
                        bits: 1,
                        endian: Big,
                    },
                    &[(0, "525/60"), (1, "625/50")],
                ),
            ),
            (
                "reserved",
                T::UInt {
                    bits: 7,
                    endian: Big,
                },
            ),
            (
                "reserved2",
                T::UInt {
                    bits: 5,
                    endian: Big,
                },
            ),
            (
                "track_application_id",
                T::UInt {
                    bits: 3,
                    endian: Big,
                },
            ),
            ("rest", T::bytes(E::lit(75))),
        ],
    )
}

fn audio() -> T {
    T::structure(
        "AudioDIF",
        vec![
            ("AAUX", T::bytes(E::lit(5))),
            ("samples", T::bytes(E::lit(72))),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    #[test]
    fn dif_id_splits_position_from_payload() {
        let mut bytes = vec![0x60, 0x17, 3]; // audio, sequence 1, first channel
        bytes.extend_from_slice(&[0x50, 1, 2, 3, 4]);
        bytes.resize(80, 0xaa);
        let doc = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(dv());
        assert_eq!(
            ev.node(&doc, &[0, 0]).unwrap().value,
            Value::Enum {
                raw: 3,
                name: Some("audio".into()),
                hex: false
            }
        );
        assert_eq!(ev.node(&doc, &[0, 3]).unwrap().value, Value::UInt(1));
        assert_eq!(ev.node(&doc, &[0, 7]).unwrap().value, Value::UInt(3));
        assert_eq!(ev.node(&doc, &[0, 8, 0]).unwrap().size_bits, 40);
    }
}
