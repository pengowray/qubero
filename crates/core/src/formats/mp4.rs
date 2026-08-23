//! MP4 (ISO base media file format), and as much H.264 as the container holds.
//!
//! Every box is size, four-character type, payload. Container boxes hold more
//! boxes, which is why the IR grew `Ty::Named`: `Box` refers to itself through
//! the template's type table.
//!
//! What is here: the box tree, ftyp, the movie/track/media headers in both
//! their 32-bit and 64-bit versions, hdlr, the sample description down to
//! avc1, and the AVC decoder configuration record with its SPS and PPS, each
//! NAL unit split into its header bits and payload.
//!
//! What is not: the contents of an SPS or PPS, which are exp-golomb coded bit
//! fields the IR cannot describe yet; sample tables beyond stsd; and a box with
//! size 0, meaning "to the end of the file", which needs a "rest of the
//! container" expression.

use crate::template::{Endian::*, Expr as E, Template, Ty as T, Until};

/// A four-character box type as the big-endian number the IR compares against.
fn cc(s: &str) -> i128 {
    s.bytes().fold(0i128, |acc, b| (acc << 8) | b as i128)
}

fn u32be() -> T {
    T::u32(Big)
}

fn u16be() -> T {
    T::u16(Big)
}

/// Boxes whose payload is nothing but more boxes.
const CONTAINERS: &[&str] =
    &["moov", "trak", "mdia", "minf", "stbl", "edts", "dinf", "udta", "mvex", "moof", "traf", "mfra"];

/// H.264 profiles, as they appear in the decoder configuration record.
const PROFILE: &[(i128, &str)] = &[
    (66, "baseline"),
    (77, "main"),
    (88, "extended"),
    (100, "high"),
    (110, "high 10"),
    (122, "high 4:2:2"),
    (244, "high 4:4:4"),
];

/// NAL unit types from the H.264 spec, table 7-1.
const NAL_TYPE: &[(i128, &str)] = &[
    (1, "non-IDR slice"),
    (2, "partition A"),
    (3, "partition B"),
    (4, "partition C"),
    (5, "IDR slice"),
    (6, "SEI"),
    (7, "SPS"),
    (8, "PPS"),
    (9, "access unit delimiter"),
    (10, "end of sequence"),
    (11, "end of stream"),
    (12, "filler"),
    (13, "SPS extension"),
    (19, "auxiliary slice"),
];

pub fn mp4() -> Template {
    Template::new("mp4", T::repeat(T::Named("Box".into()), Until::End)).with_type("Box", boxes())
}

fn boxes() -> T {
    // A box is 8 bytes of header, or 16 when size is 1 and the real size is a
    // 64-bit field after the type.
    let short = E::field("size").sub(E::lit(8));
    let long = E::field("largesize").sub(E::lit(16));
    T::structure(
        "Box",
        vec![
            ("size", u32be()),
            ("type", T::utf8(E::lit(4))),
            (
                "body",
                T::switch(
                    E::field("size"),
                    vec![(
                        1,
                        T::structure(
                            "LargeBox",
                            vec![
                                ("largesize", T::u64(Big)),
                                ("payload", T::sized(long.clone(), payload(long))),
                            ],
                        ),
                    )],
                    T::sized(short.clone(), payload(short)),
                ),
            ),
        ],
    )
}

/// What is inside a box of `len` bytes, chosen by its type.
fn payload(len: E) -> T {
    let mut cases: Vec<(i128, T)> = CONTAINERS.iter().map(|c| (cc(c), T::repeat(T::Named("Box".into()), Until::End))).collect();
    cases.push((cc("ftyp"), ftyp(len.clone())));
    cases.push((cc("mvhd"), mvhd()));
    cases.push((cc("tkhd"), tkhd()));
    cases.push((cc("mdhd"), mdhd()));
    cases.push((cc("hdlr"), hdlr(len.clone())));
    cases.push((cc("stsd"), stsd()));
    cases.push((cc("avcC"), avcc()));
    T::switch(E::field("type"), cases, T::bytes(len))
}

fn ftyp(len: E) -> T {
    T::structure(
        "FileType",
        vec![
            ("major_brand", T::utf8(E::lit(4))),
            ("minor_version", u32be()),
            ("compatible_brands", T::array(T::utf8(E::lit(4)), len.sub(E::lit(8)).div(E::lit(4)))),
        ],
    )
}

/// version and flags, the first four bytes of every "full box".
fn full_box() -> Vec<(&'static str, T)> {
    vec![("version", T::u8()), ("flags", T::bytes(E::lit(3)))]
}

fn mvhd() -> T {
    let v0 = T::structure(
        "MovieTimes32",
        vec![
            ("creation_time", u32be()),
            ("modification_time", u32be()),
            ("timescale", u32be()),
            ("duration", u32be()),
        ],
    );
    let v1 = T::structure(
        "MovieTimes64",
        vec![
            ("creation_time", T::u64(Big)),
            ("modification_time", T::u64(Big)),
            ("timescale", u32be()),
            ("duration", T::u64(Big)),
        ],
    );
    let mut fields = full_box();
    fields.extend(vec![
        ("times", T::switch(E::field("version"), vec![(1, v1)], v0)),
        ("rate", T::fixed(32, 16, Big)),
        ("volume", T::fixed(16, 8, Big)),
        ("reserved", T::bytes(E::lit(10))),
        ("matrix", T::bytes(E::lit(36))),
        ("pre_defined", T::bytes(E::lit(24))),
        ("next_track_id", u32be()),
    ]);
    T::structure("MovieHeader", fields)
}

fn tkhd() -> T {
    let v0 = T::structure(
        "TrackTimes32",
        vec![
            ("creation_time", u32be()),
            ("modification_time", u32be()),
            ("track_id", u32be()),
            ("reserved", u32be()),
            ("duration", u32be()),
        ],
    );
    let v1 = T::structure(
        "TrackTimes64",
        vec![
            ("creation_time", T::u64(Big)),
            ("modification_time", T::u64(Big)),
            ("track_id", u32be()),
            ("reserved", u32be()),
            ("duration", T::u64(Big)),
        ],
    );
    let mut fields = full_box();
    fields.extend(vec![
        ("times", T::switch(E::field("version"), vec![(1, v1)], v0)),
        ("reserved2", T::bytes(E::lit(8))),
        ("layer", T::Int { bits: 16, endian: Big }),
        ("alternate_group", T::Int { bits: 16, endian: Big }),
        ("volume", T::fixed(16, 8, Big)),
        ("reserved3", u16be()),
        ("matrix", T::bytes(E::lit(36))),
        ("width", T::fixed(32, 16, Big)),
        ("height", T::fixed(32, 16, Big)),
    ]);
    T::structure("TrackHeader", fields)
}

fn mdhd() -> T {
    let v0 = T::structure(
        "MediaTimes32",
        vec![
            ("creation_time", u32be()),
            ("modification_time", u32be()),
            ("timescale", u32be()),
            ("duration", u32be()),
        ],
    );
    let v1 = T::structure(
        "MediaTimes64",
        vec![
            ("creation_time", T::u64(Big)),
            ("modification_time", T::u64(Big)),
            ("timescale", u32be()),
            ("duration", T::u64(Big)),
        ],
    );
    let mut fields = full_box();
    // The language is three five-bit letters, each offset from 0x60.
    fields.extend(vec![
        ("times", T::switch(E::field("version"), vec![(1, v1)], v0)),
        ("pad", T::UInt { bits: 1, endian: Big }),
        ("language", T::array(T::UInt { bits: 5, endian: Big }, E::lit(3))),
        ("pre_defined", u16be()),
    ]);
    T::structure("MediaHeader", fields)
}

fn hdlr(len: E) -> T {
    let mut fields = full_box();
    fields.extend(vec![
        ("pre_defined", u32be()),
        // The handler is four characters, and they read as themselves.
        ("handler_type", T::utf8(E::lit(4))),
        ("reserved", T::bytes(E::lit(12))),
        // Fills the rest of the box and ends at a NUL, which some writers leave
        // out; padded rather than terminated so a box without one still reads.
        ("name", T::utf8_padded(len.sub(E::lit(24)), 0)),
    ]);
    T::structure("Handler", fields)
}

fn stsd() -> T {
    let mut fields = full_box();
    fields.extend(vec![
        ("entry_count", u32be()),
        ("entries", T::array(sample_entry(), E::field("entry_count"))),
    ]);
    T::structure("SampleDescription", fields)
}

fn sample_entry() -> T {
    let rest = E::field("size").sub(E::lit(16));
    T::structure(
        "SampleEntry",
        vec![
            ("size", u32be()),
            ("format", T::utf8(E::lit(4))),
            ("reserved", T::bytes(E::lit(6))),
            ("data_ref_index", u16be()),
            (
                "body",
                T::sized(
                    rest.clone(),
                    T::switch(E::field("format"), vec![(cc("avc1"), visual_sample_entry()), (cc("hvc1"), visual_sample_entry())], T::bytes(rest)),
                ),
            ),
        ],
    )
}

fn visual_sample_entry() -> T {
    T::structure(
        "VisualSampleEntry",
        vec![
            ("pre_defined", u16be()),
            ("reserved", u16be()),
            ("pre_defined2", T::bytes(E::lit(12))),
            ("width", u16be()),
            ("height", u16be()),
            ("horiz_resolution", T::fixed(32, 16, Big)),
            ("vert_resolution", T::fixed(32, 16, Big)),
            ("reserved2", u32be()),
            ("frame_count", u16be()),
            // 32 bytes: a length byte, then that many characters, then padding.
            (
                "compressor_name",
                T::structure(
                    "CompressorName",
                    vec![("length", T::u8()), ("text", T::utf8_padded(E::lit(31), 0))],
                ),
            ),
            ("depth", u16be()),
            ("pre_defined3", T::Int { bits: 16, endian: Big }),
            ("boxes", T::repeat(T::Named("Box".into()), Until::End)),
        ],
    )
}

/// AVCDecoderConfigurationRecord: what a decoder needs before the first frame.
fn avcc() -> T {
    let param_set = T::structure(
        "ParameterSet",
        vec![("length", u16be()), ("nal", T::sized(E::field("length"), nal_unit(E::field("length"))))],
    );
    T::structure(
        "AVCConfig",
        vec![
            ("configuration_version", T::u8()),
            ("profile", T::enumeration("H264Profile", T::u8(), PROFILE)),
            ("profile_compatibility", T::u8()),
            ("level", T::u8()),
            ("reserved", T::UInt { bits: 6, endian: Big }),
            ("length_size_minus_one", T::UInt { bits: 2, endian: Big }),
            ("reserved2", T::UInt { bits: 3, endian: Big }),
            ("sps_count", T::UInt { bits: 5, endian: Big }),
            ("sps", T::array(param_set.clone(), E::field("sps_count"))),
            ("pps_count", T::u8()),
            ("pps", T::array(param_set, E::field("pps_count"))),
        ],
    )
}

/// One NAL unit: a byte of header bits, then the payload the IR cannot read
/// into (exp-golomb coded).
fn nal_unit(len: E) -> T {
    T::structure(
        "NalUnit",
        vec![
            ("forbidden_zero", T::UInt { bits: 1, endian: Big }),
            ("ref_idc", T::UInt { bits: 2, endian: Big }),
            ("unit_type", T::enumeration("NalType", T::UInt { bits: 5, endian: Big }, NAL_TYPE)),
            ("rbsp", T::bytes(len.sub(E::lit(1)))),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(body);
        v
    }

    fn sample() -> Vec<u8> {
        let mut ftyp = b"isom".to_vec();
        ftyp.extend_from_slice(&512u32.to_be_bytes());
        ftyp.extend_from_slice(b"isomavc1");
        let mut mvhd = vec![0u8, 0, 0, 0]; // version 0, no flags
        mvhd.extend_from_slice(&0u32.to_be_bytes()); // creation
        mvhd.extend_from_slice(&0u32.to_be_bytes()); // modification
        mvhd.extend_from_slice(&600u32.to_be_bytes()); // timescale
        mvhd.extend_from_slice(&1200u32.to_be_bytes()); // duration
        mvhd.extend_from_slice(&0x0001_0000u32.to_be_bytes()); // rate
        mvhd.extend_from_slice(&0x0100u16.to_be_bytes()); // volume
        mvhd.extend_from_slice(&[0; 10]);
        mvhd.extend_from_slice(&[0; 36]);
        mvhd.extend_from_slice(&[0; 24]);
        mvhd.extend_from_slice(&2u32.to_be_bytes()); // next track id

        let sps = [0x67u8, 0x42, 0xc0, 0x1e]; // NAL header 0x67 = SPS, ref_idc 3
        let mut avcc = vec![1u8, 0x42, 0xc0, 0x1e, 0xff, 0xe1];
        avcc.extend_from_slice(&(sps.len() as u16).to_be_bytes());
        avcc.extend_from_slice(&sps);
        avcc.push(0); // no PPS

        let mut hdlr = vec![0u8, 0, 0, 0]; // version, flags
        hdlr.extend_from_slice(&0u32.to_be_bytes()); // pre_defined
        hdlr.extend_from_slice(b"vide");
        hdlr.extend_from_slice(&[0; 12]); // reserved
        hdlr.extend_from_slice(b"VideoHandler\0");

        let mut moov = boxed(b"mvhd", &mvhd);
        moov.extend_from_slice(&boxed(b"hdlr", &hdlr));
        moov.extend_from_slice(&boxed(b"avcC", &avcc));

        let mut out = boxed(b"ftyp", &ftyp);
        out.extend_from_slice(&boxed(b"moov", &moov));
        out.extend_from_slice(&boxed(b"mdat", &[0xaa; 16]));
        out
    }

    #[test]
    fn box_tree_recurses_and_reads_headers() {
        let d = Document::new(MemSource(sample()));
        let mut ev = Evaluator::new(mp4());
        assert_eq!(ev.node(&d, &[]).unwrap().child_count, 3);
        assert_eq!(ev.node(&d, &[0, 1]).unwrap().value, Value::Str("ftyp".into()));
        assert_eq!(ev.node(&d, &[0, 2, 0]).unwrap().value, Value::Str("isom".into()));
        assert_eq!(ev.node(&d, &[0, 2, 2]).unwrap().child_count, 2);

        // moov holds boxes, reached through Ty::Named.
        assert_eq!(ev.node(&d, &[1, 2]).unwrap().child_count, 3);
        assert_eq!(ev.node(&d, &[1, 2, 0, 1]).unwrap().value, Value::Str("mvhd".into()));
        assert_eq!(ev.node(&d, &[1, 2, 0, 2, 2, 2]).unwrap().value, Value::UInt(600));
        assert_eq!(ev.node(&d, &[1, 2, 0, 2, 2, 3]).unwrap().value, Value::UInt(1200));

        // hdlr's name is padded text: the trailing NUL is not part of the value.
        let name = ev.node(&d, &[1, 2, 1, 2, 5]).unwrap();
        assert_eq!(name.value, Value::Str("VideoHandler".into()));
        assert_eq!(name.size_bits, 13 * 8);
        assert!(name.editable);
        let w = ev.prepare_write(&d, &[1, 2, 1, 2, 5], "Cam").unwrap();
        assert_eq!(w.data, b"Cam\0\0\0\0\0\0\0\0\0\0".to_vec());

        // avcC, down to the bits of the SPS NAL header.
        let avcc = &[1usize, 2, 2, 2];
        assert_eq!(ev.node(&d, &[1, 2, 2, 1]).unwrap().value, Value::Str("avcC".into()));
        let profile = ev.node(&d, &[avcc[0], avcc[1], avcc[2], avcc[3], 1]).unwrap();
        assert_eq!(profile.value, Value::Enum { raw: 66, name: Some("baseline".into()), hex: false });
        assert_eq!(ev.node(&d, &[1, 2, 2, 2, 5]).unwrap().value, Value::UInt(3)); // length_size_minus_one
        assert_eq!(ev.node(&d, &[1, 2, 2, 2, 7]).unwrap().value, Value::UInt(1)); // sps_count
        let nal_type = ev.node(&d, &[1, 2, 2, 2, 8, 0, 1, 2]).unwrap();
        assert_eq!(nal_type.value, Value::Enum { raw: 7, name: Some("SPS".into()), hex: false });
        assert_eq!(ev.node(&d, &[1, 2, 2, 2, 8, 0, 1, 3]).unwrap().size_bits, 24);

        // hdlr's name is padded, so it reads without the trailing NUL.
        // (The sample's compressor_name is all zeros, so its length byte reads 0.)

        // mdat is left as bytes.
        assert_eq!(ev.node(&d, &[2, 2]).unwrap().size_bits, 16 * 8);
    }
}
