//! MP4 (ISO base media file format), and as much H.264 as the container holds.
//!
//! Every box is size, four-character type, payload. Container boxes hold more
//! boxes, which is why the IR grew `Ty::Named`: `Box` refers to itself through
//! the template's type table.
//!
//! What is here: the box tree, ftyp, the movie/track/media headers in both
//! their 32-bit and 64-bit versions, metadata keys and typed values, timing,
//! size and chunk indexes, the sample description down to visual entries, and
//! the AVC decoder configuration record with its SPS and PPS. Blackmagic RAW
//! adds known per-frame camera metadata, picture headers, and motion samples.
//!
//! What is not: the contents of an SPS or PPS, which are exp-golomb coded bit
//! fields the IR cannot describe yet, nor proprietary compressed BRAW essence.

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
    template_named("mp4")
}

pub(crate) fn template_named(name: &str) -> Template {
    Template::new(name, T::repeat(T::Named("Box".into()), Until::End)).with_type("Box", boxes())
}

fn boxes() -> T {
    // A box is 8 bytes of header, or 16 when size is 1 and the real size is a
    // 64-bit field after the type.
    let short = E::field("size").sub(E::lit(8));
    let long = E::field("largesize").sub(E::lit(16));
    T::structure_named(
        "Box",
        "type",
        "body",
        vec![
            ("size", u32be()),
            ("type", T::utf8(E::lit(4))),
            (
                "body",
                T::switch(
                    E::field("size"),
                    vec![
                        // Size 0 means "to the end of the file". The spec allows
                        // it only for the last box; a file that uses it anywhere
                        // else really does swallow what follows.
                        (0, T::sized(E::Remaining, payload(E::Remaining))),
                        (
                            1,
                            T::structure(
                                "LargeBox",
                                vec![
                                    ("largesize", T::u64(Big)),
                                    ("payload", T::sized(long.clone(), payload(long))),
                                ],
                            ),
                        ),
                    ],
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
    cases.push((cc("stts"), time_to_sample()));
    cases.push((cc("ctts"), composition_offsets()));
    cases.push((cc("stsc"), sample_to_chunk()));
    cases.push((cc("stsz"), sample_sizes()));
    cases.push((cc("stco"), chunk_offsets(false)));
    cases.push((cc("co64"), chunk_offsets(true)));
    cases.push((cc("stss"), sync_samples()));
    cases.push((cc("meta"), metadata()));
    cases.push((cc("keys"), metadata_keys()));
    cases.push((cc("ilst"), metadata_items()));
    cases.push((cc("mogy"), motion_vector("Gyroscope")));
    cases.push((cc("moac"), motion_vector("Accelerometer")));
    cases.push((cc("mdat"), media_data(len.clone())));
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
                    T::switch(
                        E::field("format"),
                        vec![
                            (cc("avc1"), visual_sample_entry()),
                            (cc("hvc1"), visual_sample_entry()),
                            (cc("brlt"), visual_sample_entry()),
                            (cc("brxq"), visual_sample_entry()),
                            (cc("brst"), visual_sample_entry()),
                            (cc("brvm"), visual_sample_entry()),
                            (cc("brhq"), visual_sample_entry()),
                            (cc("brvl"), visual_sample_entry()),
                            (cc("brvn"), visual_sample_entry()),
                            (cc("brvo"), visual_sample_entry()),
                        ],
                        T::bytes(rest),
                    ),
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

fn counted_full_box(name: &'static str, entry: T) -> T {
    let mut fields = full_box();
    fields.extend(vec![("entry_count", u32be()), ("entries", T::array(entry, E::field("entry_count")))]);
    T::structure(name, fields)
}

fn time_to_sample() -> T {
    counted_full_box(
        "TimeToSample",
        T::structure("TimeToSampleEntry", vec![("sample_count", u32be()), ("sample_delta", u32be())]),
    )
}

fn composition_offsets() -> T {
    let offset = T::switch(
        E::field("version"),
        vec![(1, T::Int { bits: 32, endian: Big })],
        u32be(),
    );
    counted_full_box(
        "CompositionOffsets",
        T::structure("CompositionOffsetEntry", vec![("sample_count", u32be()), ("sample_offset", offset)]),
    )
}

fn sample_to_chunk() -> T {
    counted_full_box(
        "SampleToChunk",
        T::structure(
            "SampleToChunkEntry",
            vec![("first_chunk", u32be()), ("samples_per_chunk", u32be()), ("sample_description_index", u32be())],
        ),
    )
}

fn sample_sizes() -> T {
    let mut fields = full_box();
    fields.extend(vec![
        ("sample_size", u32be()),
        ("sample_count", u32be()),
        (
            "entry_sizes",
            T::switch(
                E::field("sample_size"),
                vec![(0, T::array(u32be(), E::field("sample_count")))],
                T::bytes(E::lit(0)),
            ),
        ),
    ]);
    T::structure("SampleSizes", fields)
}

fn chunk_offsets(wide: bool) -> T {
    counted_full_box(if wide { "ChunkOffsets64" } else { "ChunkOffsets32" }, if wide { T::u64(Big) } else { u32be() })
}

fn sync_samples() -> T {
    counted_full_box("SyncSamples", u32be())
}

/// QuickTime metadata is a full `meta` box whose children are regular boxes.
fn metadata() -> T {
    let mut fields = full_box();
    fields.push(("boxes", T::repeat(T::Named("Box".into()), Until::End)));
    T::structure("Metadata", fields)
}

fn metadata_keys() -> T {
    let entry = T::structure(
        "MetadataKey",
        vec![
            ("size", u32be()),
            ("namespace", T::utf8(E::lit(4))),
            ("name", T::utf8(E::field("size").sub(E::lit(8)))),
        ],
    );
    let mut fields = full_box();
    fields.extend(vec![("entry_count", u32be()), ("entries", T::array(entry, E::field("entry_count")))]);
    T::structure("MetadataKeys", fields)
}

fn metadata_items() -> T {
    T::repeat(
        T::structure(
            "MetadataItem",
            vec![
                ("size", u32be()),
                ("key_index", u32be()),
                ("value", T::sized(E::field("size").sub(E::lit(8)), metadata_data())),
            ],
        ),
        Until::End,
    )
}

fn metadata_data() -> T {
    T::structure(
        "MetadataData",
        vec![
            ("size", u32be()),
            ("magic", T::magic(b"data")),
            ("type", T::enumeration("MetadataType", u32be(), METADATA_TYPES)),
            ("locale", u32be()),
            (
                "value",
                T::sized(
                    E::field("size").sub(E::lit(16)),
                    T::switch(
                        E::field("type"),
                        vec![
                            (1, T::utf8(E::Remaining)),
                            (23, T::F32(Big)),
                            (24, T::F64(Big)),
                            (65, T::Int { bits: 8, endian: Big }),
                            (66, T::Int { bits: 16, endian: Big }),
                            (67, T::Int { bits: 32, endian: Big }),
                            (70, T::array(T::F32(Big), E::lit(2))),
                            (71, T::array(T::F32(Big), E::lit(2))),
                            (74, T::Int { bits: 64, endian: Big }),
                            (75, T::u8()),
                            (76, u16be()),
                            (77, u32be()),
                            (78, T::u64(Big)),
                        ],
                        T::bytes(E::Remaining),
                    ),
                ),
            ),
        ],
    )
}

const METADATA_TYPES: &[(i128, &str)] = &[
    (1, "UTF-8"),
    (23, "float32"),
    (24, "float64"),
    (65, "signed int8"),
    (66, "signed int16"),
    (67, "signed int32"),
    (70, "point/size (2 × float32)"),
    (71, "dimensions (2 × float32)"),
    (74, "signed int64"),
    (75, "unsigned int8"),
    (76, "unsigned int16"),
    (77, "unsigned int32"),
    (78, "unsigned int64"),
];

fn motion_vector(name: &'static str) -> T {
    T::structure(
        name,
        vec![("x", T::F32(Little)), ("y", T::F32(Little)), ("z", T::F32(Little))],
    )
}

/// Decode the first BRAW sample when an `mdat` starts with its `bmdf` marker.
/// Sample tables still expose every subsequent sample's offset and size.
fn media_data(len: E) -> T {
    let too_short = len.clone().less_than(E::lit(8));
    let marker = E::peek_at(E::lit(4 * 8), 32, Big);
    T::switch(
        too_short.or(marker),
        vec![
            (cc("bmdf"), first_braw_sample(len.clone())),
            (cc("mogy"), motion_samples()),
            (cc("moac"), motion_samples()),
        ],
        T::bytes(len),
    )
}

fn motion_samples() -> T {
    let body_len = E::field("size").sub(E::lit(8));
    T::repeat(
        T::structure(
            "BlackmagicMotionSample",
            vec![
                ("size", u32be()),
                ("type", T::utf8(E::lit(4))),
                (
                    "value",
                    T::sized(
                        body_len.clone(),
                        T::switch(
                            E::field("type"),
                            vec![(cc("mogy"), motion_vector("Gyroscope")), (cc("moac"), motion_vector("Accelerometer"))],
                            T::bytes(body_len),
                        ),
                    ),
                ),
            ],
        ),
        Until::End,
    )
}

fn first_braw_sample(len: E) -> T {
    T::structure(
        "BrawMediaData",
        vec![
            ("metadata_size", u32be()),
            ("metadata_magic", T::magic(b"bmdf")),
            (
                "frame_metadata",
                T::sized(
                    E::field("metadata_size").sub(E::lit(8)),
                    T::repeat(frame_metadata_atom(), Until::End),
                ),
            ),
            ("picture_magic", T::magic(b"braw")),
            ("picture_size", u32be()),
            (
                "picture",
                T::sized(
                    E::field("picture_size").sub(E::lit(8)),
                    T::structure(
                        "BrawPicture",
                        vec![
                            ("sample_header", T::bytes(E::lit(12))),
                            ("width", u16be()),
                            ("height", u16be()),
                            ("slice_height", u16be()),
                            ("compressed_essence", T::bytes(E::Remaining)),
                        ],
                    ),
                ),
            ),
            (
                "remaining_samples",
                T::bytes(
                    len.sub(E::field("metadata_size"))
                        .sub(E::field("picture_size")),
                ),
            ),
        ],
    )
}

fn frame_metadata_atom() -> T {
    let body_len = E::field("size").sub(E::lit(8));
    T::structure(
        "BrawFrameMetadata",
        vec![
            ("size", u32be()),
            ("type", T::utf8(E::lit(4))),
            (
                "value",
                T::sized(
                    body_len.clone(),
                    T::switch(
                        E::field("type"),
                        vec![
                            (
                                cc("srte"),
                                T::structure("SensorRate", vec![("numerator", u32be()), ("denominator", u32be())]),
                            ),
                            (cc("innd"), T::F32(Big)),
                            (cc("agpf"), T::F32(Big)),
                            (cc("expo"), T::F32(Big)),
                            (cc("isoe"), u32be()),
                            (cc("wkel"), u32be()),
                            (cc("wtin"), u16be()),
                            (cc("asct"), u32be()),
                            (cc("asti"), u16be()),
                            (cc("shtv"), T::utf8_padded(body_len.clone(), 0)),
                            (cc("aptr"), T::utf8_padded(body_len.clone(), 0)),
                            (cc("dsnc"), T::utf8_padded(body_len.clone(), 0)),
                            (cc("fcln"), T::utf8_padded(body_len.clone(), 0)),
                        ],
                        T::bytes(body_len),
                    ),
                ),
            ),
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

    #[test]
    fn a_box_of_size_zero_runs_to_the_end_of_the_file() {
        let mut b = boxed(b"ftyp", b"isom\0\0\0\0");
        // A size of 0 means the box takes everything that is left.
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(b"mdat");
        b.extend_from_slice(&[0x11; 20]);
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(mp4());
        assert_eq!(ev.node(&d, &[]).unwrap().child_count, 2);
        let mdat = ev.node(&d, &[1]).unwrap();
        assert_eq!(mdat.size_bits, (8 + 20) * 8);
        assert_eq!(ev.node(&d, &[1, 2]).unwrap().size_bits, 20 * 8);
    }

    #[test]
    fn braw_media_exposes_frame_metadata_and_picture_header() {
        let mut metadata_atom = 12u32.to_be_bytes().to_vec();
        metadata_atom.extend_from_slice(b"expo");
        metadata_atom.extend_from_slice(&1.5f32.to_be_bytes());

        let mut media = 20u32.to_be_bytes().to_vec();
        media.extend_from_slice(b"bmdf");
        media.extend_from_slice(&metadata_atom);
        media.extend_from_slice(b"braw");
        media.extend_from_slice(&29u32.to_be_bytes());
        media.extend_from_slice(&[0x11; 12]);
        media.extend_from_slice(&4096u16.to_be_bytes());
        media.extend_from_slice(&2160u16.to_be_bytes());
        media.extend_from_slice(&270u16.to_be_bytes());
        media.extend_from_slice(&[0xaa, 0xbb, 0xcc]);

        let mut file = boxed(b"wide", b"");
        file.extend_from_slice(&boxed(b"mdat", &media));
        let d = Document::new(MemSource(file));
        let mut ev = Evaluator::new(template_named("braw"));

        assert_eq!(ev.node(&d, &[1, 2, 0]).unwrap().value, Value::UInt(20));
        assert_eq!(ev.node(&d, &[1, 2, 2, 0, 1]).unwrap().value, Value::Str("expo".into()));
        assert_eq!(ev.node(&d, &[1, 2, 5, 1]).unwrap().value, Value::UInt(4096));
        assert_eq!(ev.node(&d, &[1, 2, 5, 2]).unwrap().value, Value::UInt(2160));
        assert_eq!(ev.node(&d, &[1, 2, 5, 4]).unwrap().size_bits, 24);
    }

    #[test]
    fn quicktime_metadata_keys_and_typed_values_are_visible() {
        let mut keys = vec![0; 4];
        keys.extend_from_slice(&1u32.to_be_bytes());
        keys.extend_from_slice(&26u32.to_be_bytes());
        keys.extend_from_slice(b"mdta");
        keys.extend_from_slice(b"com.blackmagic.iso");

        let mut data = 20u32.to_be_bytes().to_vec();
        data.extend_from_slice(b"data");
        data.extend_from_slice(&77u32.to_be_bytes());
        data.extend_from_slice(&0u32.to_be_bytes());
        data.extend_from_slice(&800u32.to_be_bytes());
        let mut item = 28u32.to_be_bytes().to_vec();
        item.extend_from_slice(&1u32.to_be_bytes());
        item.extend_from_slice(&data);

        let mut file = boxed(b"keys", &keys);
        file.extend_from_slice(&boxed(b"ilst", &item));
        let d = Document::new(MemSource(file));
        let mut ev = Evaluator::new(template_named("braw"));

        assert_eq!(
            ev.node(&d, &[0, 2, 3, 0, 2]).unwrap().value,
            Value::Str("com.blackmagic.iso".into())
        );
        assert_eq!(ev.node(&d, &[1, 2, 0, 1]).unwrap().value, Value::UInt(1));
        assert_eq!(ev.node(&d, &[1, 2, 0, 2, 4]).unwrap().value, Value::UInt(800));
    }

    #[test]
    fn blackmagic_motion_samples_decode_three_axes() {
        let mut sample = 20u32.to_be_bytes().to_vec();
        sample.extend_from_slice(b"mogy");
        sample.extend_from_slice(&1.0f32.to_le_bytes());
        sample.extend_from_slice(&2.0f32.to_le_bytes());
        sample.extend_from_slice(&3.0f32.to_le_bytes());
        let d = Document::new(MemSource(boxed(b"mdat", &sample)));
        let mut ev = Evaluator::new(template_named("braw"));

        assert_eq!(ev.node(&d, &[0, 2, 0, 1]).unwrap().value, Value::Str("mogy".into()));
        assert_eq!(ev.node(&d, &[0, 2, 0, 2]).unwrap().child_count, 3);
    }
}
