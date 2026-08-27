//! Shockwave Flash (SWF): its movie header and stream of tagged records.
//!
//! `FWS` is described through its tags. In `CWS` and `ZWS`, everything after
//! the common header is compressed, so it remains one payload: decompressed
//! fields do not have honest offsets in the source file.

use crate::template::{Endian::*, Expr as E, Template, Ty as T, Until};

const COMPRESSION: &[(i128, &str)] = &[(70, "none"), (67, "zlib"), (90, "lzma")];
const TAGS: &[(i128, &str)] = &[
    (0, "End"),
    (1, "ShowFrame"),
    (2, "DefineShape"),
    (4, "PlaceObject"),
    (5, "RemoveObject"),
    (6, "DefineBits"),
    (7, "DefineButton"),
    (8, "JPEGTables"),
    (9, "SetBackgroundColor"),
    (10, "DefineFont"),
    (11, "DefineText"),
    (12, "DoAction"),
    (14, "DefineSound"),
    (15, "StartSound"),
    (18, "SoundStreamHead"),
    (19, "SoundStreamBlock"),
    (20, "DefineBitsLossless"),
    (21, "DefineBitsJPEG2"),
    (22, "DefineShape2"),
    (26, "PlaceObject2"),
    (28, "RemoveObject2"),
    (32, "DefineShape3"),
    (33, "DefineText2"),
    (34, "DefineButton2"),
    (35, "DefineBitsJPEG3"),
    (36, "DefineBitsLossless2"),
    (37, "DefineEditText"),
    (39, "DefineSprite"),
    (43, "FrameLabel"),
    (46, "DefineMorphShape"),
    (48, "DefineFont2"),
    (56, "ExportAssets"),
    (57, "ImportAssets"),
    (59, "DoInitAction"),
    (60, "DefineVideoStream"),
    (61, "VideoFrame"),
    (62, "DefineFontInfo2"),
    (65, "ScriptLimits"),
    (69, "FileAttributes"),
    (70, "PlaceObject3"),
    (71, "ImportAssets2"),
    (73, "DefineFontAlignZones"),
    (74, "CSMTextSettings"),
    (75, "DefineFont3"),
    (76, "SymbolClass"),
    (77, "Metadata"),
    (78, "DefineScalingGrid"),
    (82, "DoABC"),
    (83, "DefineShape4"),
    (84, "DefineMorphShape2"),
    (86, "DefineSceneAndFrameLabelData"),
    (87, "DefineBinaryData"),
    (88, "DefineFontName"),
    (89, "StartSound2"),
    (90, "DefineBitsJPEG4"),
    (91, "DefineFont4"),
    (93, "EnableTelemetry"),
];

pub fn swf() -> Template {
    Template::new(
        "swf",
        T::structure(
            "SWF",
            vec![
                (
                    "compression",
                    T::enumeration("Compression", T::u8(), COMPRESSION),
                ),
                ("magic", T::magic(b"WS")),
                ("version", T::u8()),
                ("file_length", T::u32(Little)),
                (
                    "movie",
                    T::switch(E::field("compression"), vec![(70, movie())], compressed()),
                ),
            ],
        ),
    )
}

fn compressed() -> T {
    T::structure("CompressedMovie", vec![("data", T::bytes(E::Remaining))])
}

fn movie() -> T {
    T::structure(
        "Movie",
        vec![
            ("frame_size", rect()),
            ("frame_rate", T::fixed(16, 8, Little)),
            ("frame_count", T::u16(Little)),
            (
                "tags",
                T::repeat(
                    tag(),
                    Until::FieldBytes {
                        field: "tag_and_length".into(),
                        bytes: vec![0, 0],
                    },
                ),
            ),
        ],
    )
}

fn rect() -> T {
    let cases = (1..=31)
        .map(|n| {
            let padding = (8 - (5 + 4 * n) % 8) % 8;
            (
                n as i128,
                T::structure(
                    "RectCoordinates",
                    vec![
                        (
                            "x_min",
                            T::Int {
                                bits: n,
                                endian: Big,
                            },
                        ),
                        (
                            "x_max",
                            T::Int {
                                bits: n,
                                endian: Big,
                            },
                        ),
                        (
                            "y_min",
                            T::Int {
                                bits: n,
                                endian: Big,
                            },
                        ),
                        (
                            "y_max",
                            T::Int {
                                bits: n,
                                endian: Big,
                            },
                        ),
                        (
                            "padding",
                            T::UInt {
                                bits: padding,
                                endian: Big,
                            },
                        ),
                    ],
                ),
            )
        })
        .collect();
    T::structure(
        "Rect",
        vec![
            (
                "bits_per_coordinate",
                T::UInt {
                    bits: 5,
                    endian: Big,
                },
            ),
            (
                "coordinates",
                T::switch(E::field("bits_per_coordinate"), cases, T::bytes(E::lit(0))),
            ),
        ],
    )
}

fn tag() -> T {
    let header = E::field("tag_and_length");
    let short = header
        .clone()
        .sub(header.clone().div(E::lit(64)).mul(E::lit(64)));
    T::structure_named(
        "Tag",
        "code",
        "body",
        vec![
            ("tag_and_length", T::u16(Little)),
            (
                "code",
                T::enumeration("TagCode", T::computed(header.div(E::lit(64))), TAGS),
            ),
            (
                "body",
                T::switch(
                    short.clone(),
                    vec![(
                        63,
                        T::structure(
                            "LongTagBody",
                            vec![
                                ("length", T::u32(Little)),
                                ("data", T::bytes(E::field("length"))),
                            ],
                        ),
                    )],
                    T::bytes(short),
                ),
            ),
        ],
    )
    .counted_as("tag")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::Document,
        eval::{Evaluator, Value},
        source::MemSource,
    };

    fn sample() -> Vec<u8> {
        let mut v = b"FWS\x09".to_vec();
        v.extend_from_slice(&26u32.to_le_bytes());
        v.extend_from_slice(&[0x08, 0x00]);
        v.extend_from_slice(&(24u16 << 8).to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&(9u16 << 6 | 3).to_le_bytes());
        v.extend_from_slice(&[0x12, 0x34, 0x56]);
        v.extend_from_slice(&(82u16 << 6 | 63).to_le_bytes());
        v.extend_from_slice(&2u32.to_le_bytes());
        v.extend_from_slice(&[0xaa, 0xbb]);
        v.extend_from_slice(&0u16.to_le_bytes());
        v
    }

    #[test]
    fn reads_movie_header_short_and_long_tags() {
        let d = Document::new(MemSource(sample()));
        let mut e = Evaluator::new(swf());
        assert_eq!(e.node(&d, &[4, 0, 0]).unwrap().value, Value::UInt(1));
        assert_eq!(e.node(&d, &[4, 1]).unwrap().value, Value::Float(24.0));
        assert_eq!(e.node(&d, &[4, 3]).unwrap().child_count, 3);
        assert!(matches!(
            e.node(&d, &[4, 3, 0, 1]).unwrap().value,
            Value::Enum { raw: 9, .. }
        ));
        assert_eq!(e.node(&d, &[4, 3, 1, 2, 0]).unwrap().value, Value::UInt(2));
    }

    #[test]
    fn compressed_payload_stays_at_its_source_offset() {
        let mut bytes = b"CWS\x0d".to_vec();
        bytes.extend_from_slice(&100u32.to_le_bytes());
        bytes.extend_from_slice(&[1, 2, 3, 4]);
        let d = Document::new(MemSource(bytes));
        let mut e = Evaluator::new(swf());
        assert_eq!(e.node(&d, &[4, 0]).unwrap().size_bits, 32);
    }
}
