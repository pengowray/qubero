//! Tracker music modules: ProTracker MOD, Scream Tracker S3M, FastTracker XM,
//! and Impulse Tracker IT.
//!
//! MOD and XM are sequential. S3M and IT use offset tables, so their pointed
//! records are represented as placed fields. Packed event streams retain each
//! variable event where its mask is self-contained; IT's channel-mask reuse
//! remains raw bytes because it carries separate state for every channel.

use crate::template::{Anchor, Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};

const MOD_SIGNATURE: &[(i128, &str)] = &[
    (0x4d2e4b2e, "M.K."),
    (0x4d214b21, "M!K!"),
    (0x4d264b21, "M&K!"),
    (0x464c5434, "FLT4"),
    (0x464c5438, "FLT8"),
    (0x3443484e, "4CHN"),
    (0x3643484e, "6CHN"),
    (0x3843484e, "8CHN"),
    (0x4f4b5441, "OKTA"),
    (0x43443831, "CD81"),
];

fn ascii(size: i128) -> T {
    T::text(
        StrLen::Padded {
            size: E::lit(size),
            pad: 0,
        },
        Encoding::Latin1,
    )
}

/// One bit of an integer without requiring a bitwise expression operator.
fn bit(value: E, mask: i128) -> E {
    value
        .clone()
        .div(E::lit(mask))
        .sub(value.div(E::lit(mask * 2)).mul(E::lit(2)))
}

pub fn mod_file() -> Template {
    let mut channel_cases = vec![
        (0x4d2e4b2e, T::computed(E::lit(4))),
        (0x4d214b21, T::computed(E::lit(4))),
        (0x4d264b21, T::computed(E::lit(4))),
        (0x464c5434, T::computed(E::lit(4))),
        (0x3443484e, T::computed(E::lit(4))),
        (0x3643484e, T::computed(E::lit(6))),
        (0x464c5438, T::computed(E::lit(8))),
        (0x3843484e, T::computed(E::lit(8))),
        (0x4f4b5441, T::computed(E::lit(8))),
        (0x43443831, T::computed(E::lit(8))),
    ];
    for channels in 1..=9i128 {
        let signature = ((b'0' as i128 + channels) << 24) | 0x0043_484e;
        channel_cases.push((signature, T::computed(E::lit(channels))));
    }
    for channels in 10..=99i128 {
        let signature = ((b'0' as i128 + channels / 10) << 24)
            | ((b'0' as i128 + channels % 10) << 16)
            | 0x4348;
        channel_cases.push((signature, T::computed(E::lit(channels))));
        channel_cases.push((signature + 6, T::computed(E::lit(channels)))); // xxCN
    }
    Template::new(
        "mod",
        T::structure(
            "ProTrackerModule",
            vec![
                ("title", ascii(20)),
                ("samples", T::array(mod_sample_header(), E::lit(31))),
                ("song_length", T::u8()),
                ("restart_position", T::u8()),
                ("orders", T::array(T::u8(), E::field("song_length"))),
                (
                    "unused_orders",
                    T::bytes(E::lit(128).sub(E::field("song_length"))),
                ),
                (
                    "signature",
                    T::enumeration_hex("ModSignature", T::u32(Big), MOD_SIGNATURE),
                ),
                (
                    "channel_count",
                    T::switch(E::field("signature"), channel_cases, T::computed(E::lit(4))),
                ),
                (
                    "patterns",
                    T::array(
                        mod_pattern(E::field("channel_count")),
                        E::max_of("orders").add(E::lit(1)),
                    ),
                ),
                (
                    "sample_data",
                    T::array(
                        T::bytes(
                            E::elem_field("samples", E::idx(), &["length_words"]).mul(E::lit(2)),
                        ),
                        E::lit(31),
                    ),
                ),
            ],
        ),
    )
}

fn mod_sample_header() -> T {
    T::structure_named(
        "SampleHeader",
        "name",
        "",
        vec![
            ("name", ascii(22)),
            ("length_words", T::u16(Big)),
            ("finetune", T::u8()),
            ("volume", T::u8()),
            ("loop_start_words", T::u16(Big)),
            ("loop_length_words", T::u16(Big)),
        ],
    )
    .counted_as("sample")
}

fn mod_pattern(channels: E) -> T {
    T::array(
        T::array(
            T::inline_structure(
                "Cell",
                vec![
                    (
                        "sample_high",
                        T::UInt {
                            bits: 4,
                            endian: Big,
                        },
                    ),
                    (
                        "period",
                        T::UInt {
                            bits: 12,
                            endian: Big,
                        },
                    ),
                    (
                        "sample_low",
                        T::UInt {
                            bits: 4,
                            endian: Big,
                        },
                    ),
                    (
                        "effect",
                        T::UInt {
                            bits: 4,
                            endian: Big,
                        },
                    ),
                    ("parameter", T::u8()),
                ],
            ),
            channels,
        ),
        E::lit(64),
    )
}

pub fn s3m() -> Template {
    Template::new(
        "s3m",
        T::structure(
            "ScreamTrackerModule",
            vec![
                ("title", ascii(28)),
                ("marker", T::u8()),
                ("file_type", T::u8()),
                ("reserved", T::u16(Little)),
                ("order_count", T::u16(Little)),
                ("instrument_count", T::u16(Little)),
                ("pattern_count", T::u16(Little)),
                ("flags", T::u16(Little)),
                ("created_with", T::u16(Little)),
                ("sample_format", T::u16(Little)),
                ("magic", T::magic(b"SCRM")),
                ("global_volume", T::u8()),
                ("initial_speed", T::u8()),
                ("initial_tempo", T::u8()),
                ("master_volume", T::u8()),
                ("ultraclick", T::u8()),
                ("default_pan", T::u8()),
                ("reserved2", T::bytes(E::lit(8))),
                ("special", T::u16(Little)),
                ("channel_settings", T::array(T::u8(), E::lit(32))),
                ("orders", T::array(T::u8(), E::field("order_count"))),
                (
                    "instrument_offsets",
                    T::array(paragraph_pointer(), E::field("instrument_count")),
                ),
                (
                    "pattern_offsets",
                    T::array(paragraph_pointer(), E::field("pattern_count")),
                ),
                (
                    "panning",
                    T::switch(
                        E::field("default_pan"),
                        vec![(252, T::array(T::u8(), E::lit(32)))],
                        T::bytes(E::lit(0)),
                    ),
                ),
                (
                    "instruments",
                    T::at(
                        E::lit(0),
                        T::pointer_list_records(
                            "instrument_offsets",
                            &["offset"],
                            Anchor::File,
                            E::lit(0),
                            s3m_instrument(),
                        )
                        .skipping_zero(),
                    ),
                ),
                (
                    "patterns",
                    T::at(
                        E::lit(0),
                        T::pointer_list_records(
                            "pattern_offsets",
                            &["offset"],
                            Anchor::File,
                            E::lit(0),
                            s3m_pattern(),
                        )
                        .skipping_zero(),
                    ),
                ),
            ],
        ),
    )
}

fn paragraph_pointer() -> T {
    T::inline_structure(
        "ParagraphPointer",
        vec![
            ("paragraph", T::u16(Little)),
            ("offset", T::computed(E::field("paragraph").mul(E::lit(16)))),
        ],
    )
}

fn s3m_instrument() -> T {
    T::structure_named(
        "S3mInstrument",
        "name",
        "",
        vec![
            ("kind", T::u8()),
            ("filename", ascii(12)),
            ("memseg_high", T::u8()),
            ("memseg_low", T::u16(Little)),
            (
                "data_offset",
                T::computed(
                    E::field("memseg_high")
                        .mul(E::lit(65536))
                        .add(E::field("memseg_low"))
                        .mul(E::lit(16)),
                ),
            ),
            ("length", T::u32(Little)),
            ("loop_begin", T::u32(Little)),
            ("loop_end", T::u32(Little)),
            ("volume", T::u8()),
            ("reserved", T::u8()),
            ("packing", T::u8()),
            (
                "flags",
                T::flags(
                    "S3mSampleFlags",
                    T::u8(),
                    &[(0, "loop"), (1, "stereo"), (2, "16-bit")],
                ),
            ),
            ("c2_speed", T::u32(Little)),
            ("reserved2", T::bytes(E::lit(12))),
            ("name", ascii(28)),
            ("magic", T::magic(b"SCRS")),
            (
                "data",
                T::at(
                    E::field("data_offset"),
                    T::bytes(
                        E::field("length")
                            .mul(bit(E::field("flags"), 4).add(E::lit(1)))
                            .mul(bit(E::field("flags"), 2).add(E::lit(1))),
                    ),
                ),
            ),
        ],
    )
}

fn s3m_pattern() -> T {
    T::structure(
        "S3mPattern",
        vec![
            ("packed_length", T::u16(Little)),
            (
                "events",
                T::sized(
                    E::field("packed_length"),
                    T::repeat(s3m_event(), Until::End),
                ),
            ),
        ],
    )
}

fn s3m_event() -> T {
    T::inline_structure(
        "S3mEvent",
        vec![
            ("control", T::u8()),
            (
                "note_instrument",
                T::switch(
                    bit(E::field("control"), 32),
                    vec![(1, T::bytes(E::lit(2)))],
                    T::bytes(E::lit(0)),
                ),
            ),
            (
                "volume",
                T::switch(
                    bit(E::field("control"), 64),
                    vec![(1, T::u8())],
                    T::bytes(E::lit(0)),
                ),
            ),
            (
                "effect",
                T::switch(
                    bit(E::field("control"), 128),
                    vec![(1, T::bytes(E::lit(2)))],
                    T::bytes(E::lit(0)),
                ),
            ),
        ],
    )
}

pub fn xm() -> Template {
    Template::new(
        "xm",
        T::structure(
            "FastTrackerModule",
            vec![
                ("magic", T::magic(b"Extended Module: ")),
                ("module_name", ascii(20)),
                ("separator", T::u8()),
                ("tracker_name", ascii(20)),
                ("version", T::u16(Little)),
                ("header_size", T::u32(Little)),
                (
                    "header",
                    T::sized(
                        E::field("header_size"),
                        T::structure(
                            "SongHeader",
                            vec![
                                ("song_length", T::u16(Little)),
                                ("restart_position", T::u16(Little)),
                                ("channel_count", T::u16(Little)),
                                ("pattern_count", T::u16(Little)),
                                ("instrument_count", T::u16(Little)),
                                ("flags", T::u16(Little)),
                                ("default_tempo", T::u16(Little)),
                                ("default_bpm", T::u16(Little)),
                                ("orders", T::array(T::u8(), E::lit(256))),
                                ("extra", T::bytes(E::Remaining)),
                            ],
                        ),
                    ),
                ),
                (
                    "patterns",
                    T::array(xm_pattern(), E::within(&["header", "pattern_count"])),
                ),
                (
                    "instruments",
                    T::array(xm_instrument(), E::within(&["header", "instrument_count"])),
                ),
            ],
        ),
    )
}

fn xm_pattern() -> T {
    T::structure(
        "XmPattern",
        vec![
            ("header_length", T::u32(Little)),
            ("packing_type", T::u8()),
            ("row_count", T::u16(Little)),
            ("packed_size", T::u16(Little)),
            (
                "header_extra",
                T::bytes(E::field("header_length").sub(E::lit(9))),
            ),
            (
                "data",
                T::sized(E::field("packed_size"), T::repeat(xm_event(), Until::End)),
            ),
        ],
    )
}

fn xm_event() -> T {
    let field = |mask| {
        T::switch(
            bit(E::field("control"), 128).mul(bit(E::field("control"), mask)),
            vec![(1, T::u8())],
            T::bytes(E::lit(0)),
        )
    };
    T::inline_structure(
        "XmEvent",
        vec![
            ("control", T::u8()),
            (
                "unpacked_tail",
                T::switch(
                    E::field("control").div(E::lit(128)),
                    vec![(0, T::bytes(E::lit(4)))],
                    T::bytes(E::lit(0)),
                ),
            ),
            ("note", field(1)),
            ("instrument", field(2)),
            ("volume", field(4)),
            ("effect", field(8)),
            ("parameter", field(16)),
        ],
    )
}

fn xm_instrument() -> T {
    T::structure_named(
        "XmInstrument",
        "name",
        "",
        vec![
            ("header_size", T::u32(Little)),
            ("name", ascii(22)),
            ("kind", T::u8()),
            ("sample_count", T::u16(Little)),
            (
                "details",
                T::sized(
                    E::field("header_size").sub(E::lit(29)),
                    T::switch(
                        E::field("sample_count"),
                        vec![(0, T::bytes(E::lit(0)))],
                        xm_instrument_details(),
                    ),
                ),
            ),
            (
                "sample_headers",
                T::array(
                    T::sized(
                        E::within(&["details", "sample_header_size"]),
                        xm_sample_header(),
                    ),
                    E::field("sample_count"),
                ),
            ),
            (
                "sample_data",
                T::array(
                    T::bytes(E::elem_field("sample_headers", E::idx(), &["length"])),
                    E::field("sample_count"),
                ),
            ),
        ],
    )
}

fn xm_instrument_details() -> T {
    T::structure(
        "XmInstrumentDetails",
        vec![
            ("sample_header_size", T::u32(Little)),
            ("sample_map", T::array(T::u8(), E::lit(96))),
            ("volume_envelope", T::array(T::u16(Little), E::lit(24))),
            ("panning_envelope", T::array(T::u16(Little), E::lit(24))),
            ("volume_points", T::u8()),
            ("panning_points", T::u8()),
            ("volume_sustain", T::u8()),
            ("volume_loop_start", T::u8()),
            ("volume_loop_end", T::u8()),
            ("panning_sustain", T::u8()),
            ("panning_loop_start", T::u8()),
            ("panning_loop_end", T::u8()),
            ("volume_type", T::u8()),
            ("panning_type", T::u8()),
            ("vibrato_type", T::u8()),
            ("vibrato_sweep", T::u8()),
            ("vibrato_depth", T::u8()),
            ("vibrato_rate", T::u8()),
            ("volume_fadeout", T::u16(Little)),
            ("reserved", T::u16(Little)),
            ("extra", T::bytes(E::Remaining)),
        ],
    )
}

fn xm_sample_header() -> T {
    T::structure_named(
        "XmSample",
        "name",
        "",
        vec![
            ("length", T::u32(Little)),
            ("loop_start", T::u32(Little)),
            ("loop_length", T::u32(Little)),
            ("volume", T::u8()),
            (
                "finetune",
                T::Int {
                    bits: 8,
                    endian: Little,
                },
            ),
            (
                "type",
                T::flags(
                    "XmSampleType",
                    T::u8(),
                    &[(0, "forward loop"), (1, "ping-pong loop"), (4, "16-bit")],
                ),
            ),
            ("panning", T::u8()),
            (
                "relative_note",
                T::Int {
                    bits: 8,
                    endian: Little,
                },
            ),
            ("reserved", T::u8()),
            ("name", ascii(22)),
        ],
    )
}

pub fn it() -> Template {
    Template::new(
        "it",
        T::structure(
            "ImpulseTrackerModule",
            vec![
                ("magic", T::magic(b"IMPM")),
                ("song_name", ascii(26)),
                ("pattern_highlight", T::u16(Little)),
                ("order_count", T::u16(Little)),
                ("instrument_count", T::u16(Little)),
                ("sample_count", T::u16(Little)),
                ("pattern_count", T::u16(Little)),
                ("created_with", T::u16(Little)),
                ("compatible_with", T::u16(Little)),
                ("flags", T::u16(Little)),
                ("special", T::u16(Little)),
                ("global_volume", T::u8()),
                ("mix_volume", T::u8()),
                ("initial_speed", T::u8()),
                ("initial_tempo", T::u8()),
                ("pan_separation", T::u8()),
                ("pitch_wheel_depth", T::u8()),
                ("message_length", T::u16(Little)),
                ("message_offset", T::u32(Little)),
                ("reserved", T::u32(Little)),
                ("channel_pan", T::array(T::u8(), E::lit(64))),
                ("channel_volume", T::array(T::u8(), E::lit(64))),
                ("orders", T::array(T::u8(), E::field("order_count"))),
                (
                    "instrument_offsets",
                    T::array(T::u32(Little), E::field("instrument_count")),
                ),
                (
                    "sample_offsets",
                    T::array(T::u32(Little), E::field("sample_count")),
                ),
                (
                    "pattern_offsets",
                    T::array(T::u32(Little), E::field("pattern_count")),
                ),
                (
                    "message",
                    T::at(
                        E::field("message_offset"),
                        T::text(StrLen::Fixed(E::field("message_length")), Encoding::Cp437),
                    ),
                ),
                (
                    "instruments",
                    T::at(
                        E::lit(0),
                        T::pointer_list(
                            "instrument_offsets",
                            Anchor::File,
                            E::lit(0),
                            it_instrument(),
                        )
                        .skipping_zero(),
                    ),
                ),
                (
                    "samples",
                    T::at(
                        E::lit(0),
                        T::pointer_list("sample_offsets", Anchor::File, E::lit(0), it_sample())
                            .skipping_zero(),
                    ),
                ),
                (
                    "patterns",
                    T::at(
                        E::lit(0),
                        T::pointer_list("pattern_offsets", Anchor::File, E::lit(0), it_pattern())
                            .skipping_zero(),
                    ),
                ),
            ],
        ),
    )
}

fn it_instrument() -> T {
    T::structure_named(
        "ItInstrument",
        "name",
        "",
        vec![
            ("magic", T::magic(b"IMPI")),
            ("filename", ascii(12)),
            ("zero", T::u8()),
            ("new_note_action", T::u8()),
            ("duplicate_check", T::u8()),
            ("duplicate_action", T::u8()),
            ("fadeout", T::u16(Little)),
            (
                "pitch_pan_separation",
                T::Int {
                    bits: 8,
                    endian: Little,
                },
            ),
            ("pitch_pan_center", T::u8()),
            ("global_volume", T::u8()),
            ("default_pan", T::u8()),
            ("random_volume", T::u8()),
            ("random_pan", T::u8()),
            ("tracker_version", T::u16(Little)),
            ("sample_count", T::u8()),
            ("reserved", T::u8()),
            ("name", ascii(26)),
            ("initial_filter_cutoff", T::u8()),
            ("initial_filter_resonance", T::u8()),
            ("midi_channel", T::u8()),
            ("midi_program", T::u8()),
            ("midi_bank", T::u16(Little)),
            ("note_sample_map", T::bytes(E::lit(240))),
            ("volume_envelope", T::bytes(E::lit(82))),
            ("panning_envelope", T::bytes(E::lit(82))),
            ("pitch_envelope", T::bytes(E::lit(82))),
            ("reserved2", T::bytes(E::lit(4))),
        ],
    )
}

fn it_sample() -> T {
    T::structure_named(
        "ItSample",
        "name",
        "",
        vec![
            ("magic", T::magic(b"IMPS")),
            ("filename", ascii(12)),
            ("zero", T::u8()),
            ("global_volume", T::u8()),
            (
                "flags",
                T::flags(
                    "ItSampleFlags",
                    T::u8(),
                    &[
                        (0, "data"),
                        (1, "16-bit"),
                        (2, "stereo"),
                        (3, "compressed"),
                        (4, "loop"),
                        (5, "sustain loop"),
                        (6, "ping-pong loop"),
                        (7, "ping-pong sustain"),
                    ],
                ),
            ),
            ("default_volume", T::u8()),
            ("name", ascii(26)),
            ("convert", T::u8()),
            ("default_pan", T::u8()),
            ("length", T::u32(Little)),
            ("loop_begin", T::u32(Little)),
            ("loop_end", T::u32(Little)),
            ("c5_speed", T::u32(Little)),
            ("sustain_begin", T::u32(Little)),
            ("sustain_end", T::u32(Little)),
            ("data_offset", T::u32(Little)),
            ("vibrato_speed", T::u8()),
            ("vibrato_depth", T::u8()),
            ("vibrato_rate", T::u8()),
            ("vibrato_waveform", T::u8()),
            (
                "data",
                T::at(
                    E::field("data_offset"),
                    T::bytes(
                        E::field("length")
                            .mul(bit(E::field("flags"), 2).add(E::lit(1)))
                            .mul(bit(E::field("flags"), 4).add(E::lit(1))),
                    ),
                ),
            ),
        ],
    )
}

fn it_pattern() -> T {
    T::structure(
        "ItPattern",
        vec![
            ("packed_length", T::u16(Little)),
            ("row_count", T::u16(Little)),
            ("reserved", T::u32(Little)),
            ("data", T::bytes(E::field("packed_length"))),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, eval::Evaluator, source::MemSource};

    #[test]
    fn mod_order_table_sets_pattern_count() {
        let mut b = vec![0u8; 1084 + 2 * 1024];
        b[950] = 2;
        b[952] = 0;
        b[953] = 1;
        b[1080..1084].copy_from_slice(b"M.K.");
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(mod_file());
        assert_eq!(ev.node(&d, &[8]).unwrap().child_count, 2);
    }

    #[test]
    fn tracker_signatures_are_sniffed() {
        let mut m = vec![0; 1084];
        m[1080..].copy_from_slice(b"M.K.");
        assert_eq!(crate::formats::sniff(&m, m.len() as u64), Some("mod"));
        assert_eq!(
            crate::formats::sniff(b"Extended Module: test", 1000),
            Some("xm")
        );
        let mut s = vec![0; 48];
        s[28] = 0x1a;
        s[29] = 16;
        s[44..48].copy_from_slice(b"SCRM");
        assert_eq!(crate::formats::sniff(&s, 1000), Some("s3m"));
        assert_eq!(crate::formats::sniff(b"IMPMtest", 1000), Some("it"));
    }

    #[test]
    fn empty_xm_s3m_and_it_headers_evaluate() {
        let mut xm_bytes = b"Extended Module: ".to_vec();
        xm_bytes.resize(37, 0);
        xm_bytes.push(0x1a);
        xm_bytes.resize(58, 0);
        xm_bytes.extend_from_slice(&0x0104u16.to_le_bytes());
        xm_bytes.extend_from_slice(&276u32.to_le_bytes());
        xm_bytes.resize(340, 0);
        let xm_doc = Document::new(MemSource(xm_bytes));
        assert_eq!(
            Evaluator::new(xm())
                .node(&xm_doc, &[7])
                .unwrap()
                .child_count,
            0
        );

        let mut s3m_bytes = vec![0; 96];
        s3m_bytes[28] = 0x1a;
        s3m_bytes[29] = 16;
        s3m_bytes[44..48].copy_from_slice(b"SCRM");
        let s3m_doc = Document::new(MemSource(s3m_bytes));
        assert_eq!(
            Evaluator::new(s3m())
                .node(&s3m_doc, &[19])
                .unwrap()
                .child_count,
            32
        );

        let mut it_bytes = b"IMPM".to_vec();
        it_bytes.resize(192, 0);
        let it_doc = Document::new(MemSource(it_bytes));
        assert_eq!(
            Evaluator::new(it())
                .node(&it_doc, &[20])
                .unwrap()
                .child_count,
            64
        );
    }
}
