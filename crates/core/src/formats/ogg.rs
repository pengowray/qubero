//! Ogg: pages that carry one or more logical streams, such as Vorbis or Opus
//! audio, Theora video, or FLAC.
//!
//! A page is 27 bytes of header, a table of segment lengths called lacing
//! values, and the segments those values measure. A packet is split into
//! segments: a lacing value of 255 means the packet goes on into the next
//! segment, and any smaller value ends it. So each packet's lacing is a run of
//! 255s and one smaller value, and a packet whose length is a multiple of 255
//! ends with a 0. The segment table is read here as one entry per packet, and
//! the segments as the packets they make up, each as long as its entry says.
//!
//! **A packet may continue onto the next page.** Its last lacing value on this
//! page is then 255, and the next page of the stream sets its `continued
//! packet` flag. The pieces on either side stay runs of bytes: joining them
//! would need a run made of pieces chosen by stream and page, and
//! [`Ty::Stitched`](crate::template::Ty::Stitched) chooses its pieces by field
//! name only.
//!
//! **Header packets are read by their signatures.** A packet that starts and
//! ends on one page is read as what its first bytes say it is: a Vorbis
//! identification, comment or setup header (`\x01vorbis`, `\x03vorbis`,
//! `\x05vorbis`), `OpusHead`, `OpusTags`, or the first packet of an Ogg FLAC
//! stream (`\x7fFLAC`). Ogg FLAC's other header packets have no signature:
//! each holds one FLAC metadata block, and is recognised by a block header
//! whose length is exactly the packet's length minus 4, on a page whose
//! granule position is 0. Ogg FLAC audio packets are one frame each and start
//! with the frame sync code. Vorbis and Opus audio packets, Theora packets,
//! and the packets of any other codec are runs of bytes.
//!
//! **A page seals itself.** The CRC-32 sits inside the run it covers, and
//! the writer summed four zeroes where the number was going to go, so the
//! check reads those bytes as zeroes too. That is [`Check::blanking`], and
//! this is the second format to need it after tar; the sum itself is the
//! unreflected CRC-32, which is a different number from the reflected one
//! every other format here writes. See [`Checksum::Crc32Ogg`].
//!
//! **Streams are interleaved.** Several logical streams may share one file,
//! each with its own serial number, and a player picks the pages it wants by
//! that number. Pages of different streams alternate, so the pages are read
//! in the order they were written rather than gathered by serial: the file is
//! what it is, and grouping it would put the reading somewhere the bytes are
//! not.

use crate::template::{Check, Checksum, Covers, Endian::*, Expr as E, Template, Until, Ty as T};

/// What every page starts with.
pub const MAGIC: &[u8] = b"OggS";

/// The header before the lacing table: the magic, the version, the flags, the
/// granule position, the serial, the sequence, the sum, and the count.
const HEADER: i128 = 27;

/// What the flags byte says about the page, one bit each.
const FLAGS: &[(u32, &str)] = &[
    // The first packet here is the tail of one that began on an earlier page.
    (0, "continued packet"),
    // The first page of this logical stream.
    (1, "first page"),
    // The last one.
    (2, "last page"),
];

/// Which reading a packet gets, by what its first bytes are. See
/// [`packet_kind`].
const VORBIS_IDENTIFICATION: i128 = 1;
const VORBIS_COMMENT: i128 = 2;
const VORBIS_SETUP: i128 = 3;
const OPUS_HEAD: i128 = 4;
const OPUS_TAGS: i128 = 5;
const FLAC_MAPPING: i128 = 6;
const FLAC_FRAME: i128 = 7;
const FLAC_BLOCK: i128 = 8;

pub fn ogg() -> Template {
    // An Ogg FLAC stream's PICTURE block brings the PNG and JPEG readers, and
    // the names they refer to, with it.
    let flac = super::flac::part();
    Template::new("ogg", T::structure("OggStream", vec![("pages", T::repeat(page(), Until::End))])).with_part(&flac)
}

fn page() -> T {
    T::structure_named(
        "OggPage",
        "sequence",
        "packets",
        vec![
            ("magic", T::magic(MAGIC)),
            ("version", T::u8()),
            ("header_type", T::flags("OggHeaderType", T::u8(), FLAGS)),
            // How far into the stream the last packet finishing on this page
            // reaches, in whatever unit the codec counts in: samples for
            // Vorbis and Opus, frames for Theora. -1 says no packet finishes
            // here, which is why it is signed.
            ("granule_position", T::Int { bits: 64, endian: Little }),
            // Which logical stream this page belongs to. Several share a file
            // and a player follows one of them.
            ("serial_number", T::u32(Little)),
            ("page_sequence", T::u32(Little)),
            ("crc32", T::u32(Little)),
            ("page_segments", T::u8()),
            ("segment_table", T::sized(E::field("page_segments"), T::repeat(lacing(), Until::End))),
            ("packets", T::array(packet(), E::len_of("segment_table"))),
        ],
    )
    .counted_as("page")
    .field_check("crc32", page_crc())
    .field_doc("page_segments", "Number of lacing values in the segment table.")
    .field_doc("segment_table", "The lacing values, grouped by packet.")
}

/// The sum a page writes about itself: every byte of the page, with the four
/// its own checksum sits in read as zeroes.
///
/// The writer had to put something there while adding up, and zeroes are what
/// the format says to put, so a reader adding those bytes up as they stand
/// gets another number entirely. The run is the page from its first byte,
/// since offsets in a check count from the structure the field sits in.
fn page_crc() -> Check {
    let length = E::lit(HEADER).add(E::field("page_segments")).add(E::size_of("packets"));
    Check::of(Checksum::Crc32Ogg, Covers::Run { at: E::lit(0), len: length }).blanking(0)
}

/// The lacing values of one packet: any number of 255s and the value that
/// ends the packet, or only 255s when the packet goes on past this page.
fn lacing() -> T {
    let ends_here = E::sum_of("segments").less_than(E::lit(255).mul(E::len_of("segments")));
    T::structure(
        "Lacing",
        vec![
            ("segments", T::array(T::u8(), E::run(&[0xff]).add(E::lit(1)).at_most(E::Remaining))),
            ("packet_length", T::computed(E::sum_of("segments"))),
            ("packet_ends", T::enumeration("PacketEnds", T::computed(ends_here), &[(0, "on a later page"), (1, "on this page")])),
        ],
    )
    .counted_as("packet")
    .reads_as(&[("packet_length", "{} bytes", ""), ("packet_ends", "", "on this page")])
    .field_doc("segments", "Lacing values: the length in bytes of each segment of this packet. 255 means the packet continues in the next segment.")
    .field_doc("packet_length", "The packet's length on this page in bytes: the sum of its lacing values.")
    .field_doc("packet_ends", "Whether the packet ends on this page or on a later page of the same logical stream.")
}

/// One packet, as long as its lacing says, read by what its first bytes are.
fn packet() -> T {
    let length = E::elem_field("segment_table", E::idx(), &["packet_length"]);
    T::sized(
        length,
        T::switch(
            packet_kind(),
            vec![
                (VORBIS_IDENTIFICATION, vorbis_identification()),
                (VORBIS_COMMENT, vorbis_comment()),
                (VORBIS_SETUP, vorbis_setup()),
                (OPUS_HEAD, opus_head()),
                (OPUS_TAGS, opus_tags()),
                (FLAC_MAPPING, flac_mapping()),
                (FLAC_FRAME, super::flac::frame()),
                (FLAC_BLOCK, super::flac::metadata_block()),
            ],
            T::bytes(E::Remaining),
        ),
    )
}

/// Whether the packet starts with `bytes`, asked without reading past its end.
fn starts_with(bytes: &[u8]) -> E {
    let value = bytes.iter().fold(0i128, |a, &b| (a << 8) | i128::from(b));
    let n = bytes.len() as u32;
    E::Remaining.greater_or_equal(E::lit(i128::from(n))).both(E::peek(8 * n, Big).equal_to(E::lit(value)))
}

/// Which reading a packet gets. Only a packet that starts and ends on this
/// page is read as anything but bytes, since a piece of one cannot be read on
/// its own.
fn packet_kind() -> E {
    let starts_here = E::idx().greater_than(E::lit(0)).either(E::field("header_type").bit(0).equal_to(E::lit(0)));
    let ends_here = E::elem_field("segment_table", E::idx(), &["packet_ends"]).equal_to(E::lit(1));
    // A frame starts with the 15-bit sync code and the blocking strategy bit.
    // An Opus packet whose first two bytes are FF F8 or FF F9 would ask for
    // more frames than a packet can hold, and a Vorbis or Theora audio packet
    // starts with a 0 bit, so this is a FLAC frame or nothing.
    let flac_frame = E::Remaining.greater_or_equal(E::lit(8)).both(E::peek(16, Big).and(E::lit(0xfffe)).equal_to(E::lit(0xfff8)));
    // A metadata block on a header page: its type is not the forbidden 127,
    // and its length is the rest of the packet.
    let head = || E::peek(32, Big);
    let flac_block = E::field("granule_position")
        .equal_to(E::lit(0))
        .both(E::Remaining.greater_or_equal(E::lit(4)))
        .both(head().shr(E::lit(24)).and(E::lit(0x7f)).not_equal(E::lit(127)))
        .both(head().and(E::lit(0xff_ffff)).equal_to(E::Remaining.sub(E::lit(4))));
    let tests = [
        (starts_with(b"\x01vorbis"), VORBIS_IDENTIFICATION),
        (starts_with(b"\x03vorbis"), VORBIS_COMMENT),
        (starts_with(b"\x05vorbis"), VORBIS_SETUP),
        (starts_with(b"OpusHead"), OPUS_HEAD),
        (starts_with(b"OpusTags"), OPUS_TAGS),
        (starts_with(b"\x7fFLAC"), FLAC_MAPPING),
        (flac_frame, FLAC_FRAME),
        (flac_block, FLAC_BLOCK),
    ];
    let mut kind = E::lit(0);
    for (test, k) in tests.into_iter().rev() {
        kind = E::cond(test, E::lit(k), kind);
    }
    E::cond(starts_here.both(ends_here), kind, E::lit(0))
}

/// The first seven bytes of every Vorbis header: the packet type and the word
/// `vorbis`.
fn vorbis_start() -> Vec<(&'static str, T)> {
    vec![
        ("packet_type", T::enumeration("VorbisPacketType", T::u8(), &[(1, "identification"), (3, "comment"), (5, "setup")])),
        ("signature", T::magic(b"vorbis")),
    ]
}

/// Bytes after the end of a header that the header's own layout does not
/// cover.
fn extra() -> T {
    T::when(E::Remaining.greater_than(E::lit(0)), T::bytes(E::Remaining))
}

/// The Vorbis identification header: the first packet of a Vorbis stream.
/// Vorbis packs its fields from the low bit of each byte, so the two block
/// size exponents are the low and then the high half of one byte.
fn vorbis_identification() -> T {
    let low_bits = |n: u32| T::UInt { bits: n, endian: Little };
    let mut fields = vorbis_start();
    fields.extend(vec![
        ("vorbis_version", T::u32(Little)),
        ("audio_channels", T::u8()),
        ("audio_sample_rate", T::u32(Little)),
        ("bitrate_maximum", T::i32(Little)),
        ("bitrate_nominal", T::i32(Little)),
        ("bitrate_minimum", T::i32(Little)),
        ("blocksize_0_exponent", low_bits(4)),
        ("blocksize_1_exponent", low_bits(4)),
        ("blocksize_0", T::computed(E::pow2(E::field("blocksize_0_exponent")))),
        ("blocksize_1", T::computed(E::pow2(E::field("blocksize_1_exponent")))),
        ("framing_flag", low_bits(1)),
        ("unused_bits", low_bits(7)),
        ("extra", extra()),
    ]);
    T::structure("VorbisIdentification", fields)
        .reads_as(&[("audio_sample_rate", "{} Hz", ""), ("audio_channels", "{} channels", ""), ("bitrate_nominal", "{} bit/s", "0")])
        .field_doc("vorbis_version", "Must be 0.")
        .field_doc("audio_sample_rate", "Samples per second, per channel.")
        .field_doc("bitrate_maximum", "Highest bitrate in bits per second, as a hint. 0 or less means not set.")
        .field_doc("bitrate_nominal", "Average bitrate in bits per second, as a hint. 0 or less means not set.")
        .field_doc("bitrate_minimum", "Lowest bitrate in bits per second, as a hint. 0 or less means not set.")
        .field_doc("blocksize_0_exponent", "Exponent of the short block size: the size in samples is 2 to the power of this value.")
        .field_doc("blocksize_1_exponent", "Exponent of the long block size: the size in samples is 2 to the power of this value.")
        .field_doc("blocksize_0", "The short block size in samples.")
        .field_doc("blocksize_1", "The long block size in samples.")
        .field_doc("framing_flag", "Must be 1.")
}

/// The Vorbis comment header: the stream's tags, in the layout FLAC's
/// VORBIS_COMMENT block uses, then a framing bit.
fn vorbis_comment() -> T {
    let mut fields = vorbis_start();
    fields.extend(super::flac::comment_fields());
    fields.extend(vec![("framing_flag", T::UInt { bits: 1, endian: Little }), ("unused_bits", T::UInt { bits: 7, endian: Little }), ("extra", extra())]);
    T::structure("VorbisCommentHeader", fields).field_doc("framing_flag", "Must be 1.")
}

/// The Vorbis setup header. Everything after the codebook count is packed
/// bit by bit with no byte boundaries, so it is one run.
fn vorbis_setup() -> T {
    let mut fields = vorbis_start();
    fields.extend(vec![
        ("codebook_count_minus1", T::u8()),
        ("codebook_count", T::computed(E::field("codebook_count_minus1").add(E::lit(1)))),
        ("setup_data", T::bytes(E::Remaining)),
    ]);
    T::structure("VorbisSetup", fields)
        .field_doc("codebook_count_minus1", "Number of codebooks minus 1.")
        .field_doc(
            "setup_data",
            "The codebooks, then the time domain transforms, floors, residues, mappings and modes, bit-packed with no byte alignment. Shown as one run of bytes.",
        )
}

/// OpusHead, RFC 7845 section 5.1: the first packet of an Opus stream.
fn opus_head() -> T {
    let mapped = || E::field("channel_mapping_family").not_equal(E::lit(0));
    T::structure(
        "OpusHead",
        vec![
            ("signature", T::magic(b"OpusHead")),
            ("version", T::u8()),
            ("output_channel_count", T::u8()),
            ("pre_skip", T::u16(Little)),
            ("input_sample_rate", T::u32(Little)),
            ("output_gain", T::Fixed { bits: 16, frac: 8, endian: Little, signed: true }),
            (
                "channel_mapping_family",
                T::enumeration("ChannelMappingFamily", T::u8(), &[(0, "mono or stereo"), (1, "Vorbis channel order"), (255, "unidentified channels")]),
            ),
            ("stream_count", T::when(mapped(), T::u8())),
            ("coupled_count", T::when(mapped(), T::u8())),
            ("channel_mapping", T::when(mapped(), T::array(T::u8(), E::field("output_channel_count")))),
            ("extra", extra()),
        ],
    )
    .reads_as(&[("output_channel_count", "{} channels", ""), ("input_sample_rate", "input {} Hz", "")])
    .field_doc("version", "Format version, 1 in RFC 7845. The high 4 bits are the major version.")
    .field_doc("pre_skip", "Samples at 48 kHz to discard from the start of the decoded audio.")
    .field_doc("input_sample_rate", "The sample rate the audio had before it was encoded, for information only; playback normally uses 48 kHz.")
    .field_doc("output_gain", "Gain in dB to apply to the decoded audio.")
    .field_doc("stream_count", "Number of Opus streams in each packet.")
    .field_doc("coupled_count", "Number of the Opus streams in each packet that are coupled (stereo).")
    .field_doc("channel_mapping", "For each output channel, the index of the decoded channel to use. 255 means the output channel is silent.")
}

/// OpusTags, RFC 7845 section 5.2: the stream's tags, in the layout FLAC's
/// VORBIS_COMMENT block uses.
fn opus_tags() -> T {
    let mut fields = vec![("signature", T::magic(b"OpusTags"))];
    fields.extend(super::flac::comment_fields());
    fields.push(("extra", extra()));
    T::structure("OpusTags", fields).field_doc(
        "extra",
        "Padding or other data after the comments. Tag editors keep it if the lowest bit of its first byte is 1, and may drop it if that bit is 0.",
    )
}

/// The first packet of an Ogg FLAC stream: a signature, the mapping's
/// version, how many header packets follow, and the native FLAC signature and
/// STREAMINFO block.
fn flac_mapping() -> T {
    T::structure(
        "OggFlacHeader",
        vec![
            ("signature", T::magic(b"\x7fFLAC")),
            ("major_version", T::u8()),
            ("minor_version", T::u8()),
            ("header_packet_count", T::unset_int(T::u16(Big), 0)),
            ("native_signature", T::magic(super::flac::MAGIC)),
            ("stream_info", super::flac::metadata_block()),
        ],
    )
    .field_doc("header_packet_count", "Number of header packets after this one, each containing one metadata block. Unset (0) if unknown.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{checksum::crc32_msb, document::Document, eval::Evaluator, eval::Value, source::MemSource};

    /// A page carrying `segments`, with the sum the format asks for.
    fn page_bytes(serial: u32, sequence: u32, flags: u8, segments: &[&[u8]]) -> Vec<u8> {
        page_with(serial, sequence, flags, 0, &segments.iter().map(|s| (s.to_vec(), true)).collect::<Vec<_>>())
    }

    /// A page carrying whole packets and pieces of them: each is laced as a
    /// run of 255s and, when `ends` is set, the value that ends it.
    fn page_with(serial: u32, sequence: u32, flags: u8, granule: i64, packets: &[(Vec<u8>, bool)]) -> Vec<u8> {
        let mut table = Vec::new();
        let mut body = Vec::new();
        for (p, ends) in packets {
            let mut left = p.len();
            while left >= 255 {
                table.push(255);
                left -= 255;
            }
            if *ends {
                table.push(left as u8);
            } else {
                assert_eq!(left, 0, "a packet that goes on past the page fills whole segments");
            }
            body.extend_from_slice(p);
        }
        let mut v = MAGIC.to_vec();
        v.push(0);
        v.push(flags);
        v.extend_from_slice(&granule.to_le_bytes());
        v.extend_from_slice(&serial.to_le_bytes());
        v.extend_from_slice(&sequence.to_le_bytes());
        // Zeroes while the sum runs, which is what the format says to write.
        v.extend_from_slice(&0u32.to_le_bytes());
        v.push(table.len() as u8);
        v.extend_from_slice(&table);
        v.extend_from_slice(&body);
        let sum = crc32_msb(&v, 0, 0);
        v[22..26].copy_from_slice(&sum.to_le_bytes());
        v
    }

    fn at(e: &mut Evaluator, d: &Document<MemSource>, n: usize, name: &str) -> Vec<usize> {
        e.child_named(d, &[0, n], name).unwrap().unwrap_or_else(|| panic!("no field called {name}"))
    }

    /// The path of `names` under the root, where a name that is a number is
    /// an index.
    fn path(e: &mut Evaluator, d: &Document<MemSource>, names: &[&str]) -> Vec<usize> {
        let mut p = Vec::new();
        for name in names {
            let next = match name.parse::<usize>() {
                Ok(i) => i,
                Err(_) => e.child_named(d, &p, name).unwrap().unwrap_or_else(|| panic!("no field called {name} under {p:?}"))[p.len()],
            };
            p.push(next);
        }
        p
    }

    fn int(e: &mut Evaluator, d: &Document<MemSource>, names: &[&str]) -> i128 {
        let p = path(e, d, names);
        e.node(d, &p).unwrap().value.as_int().unwrap_or_else(|| panic!("{names:?} is not a number"))
    }

    fn text(e: &mut Evaluator, d: &Document<MemSource>, names: &[&str]) -> String {
        let p = path(e, d, names);
        match e.node(d, &p).unwrap().value {
            Value::Str(s) => s,
            other => panic!("{names:?} is not text: {other:?}"),
        }
    }

    fn type_name(e: &mut Evaluator, d: &Document<MemSource>, names: &[&str]) -> String {
        let p = path(e, d, names);
        e.node(d, &p).unwrap().type_name
    }

    fn crc_ok(e: &mut Evaluator, d: &Document<MemSource>, page: usize) -> bool {
        let sum = at(e, d, page, "crc32");
        e.run_check(d, &sum).unwrap().expect("a verdict").ok
    }

    #[test]
    fn a_page_is_its_header_its_lacing_table_and_the_packets_the_table_measures() {
        let bytes = page_bytes(0x1234, 0, 0x02, &[b"one", b"two and a bit"]);
        let d = Document::new(MemSource(bytes.clone()));
        let mut e = Evaluator::new(ogg());
        let count = at(&mut e, &d, 0, "page_segments");
        assert_eq!(e.node(&d, &count).unwrap().value.as_int(), Some(2));
        let packets = at(&mut e, &d, 0, "packets");
        assert_eq!(e.node(&d, &packets).unwrap().size_bits, (3 + 13) * 8);
        assert_eq!(e.node(&d, &packets).unwrap().child_count, 2);
        assert_eq!(int(&mut e, &d, &["pages", "0", "segment_table", "1", "packet_length"]), 13);
        let second = path(&mut e, &d, &["pages", "0", "packets", "1"]);
        assert_eq!(e.node(&d, &second).unwrap().size_bits, 13 * 8);
        let serial = at(&mut e, &d, 0, "serial_number");
        assert_eq!(e.node(&d, &serial).unwrap().value.as_int(), Some(0x1234));
        // The whole page and nothing after it.
        assert_eq!(e.node(&d, &[0, 0]).unwrap().size_bits, bytes.len() as u64 * 8);
    }

    /// The sum covers the page including the four bytes it sits in, read as
    /// zeroes. Adding those bytes up as they stand gives another number, which
    /// is the whole reason `blanking` exists.
    #[test]
    fn a_page_sums_to_what_it_wrote_down_with_its_own_four_bytes_read_as_zeroes() {
        let bytes = page_bytes(7, 3, 0, &[b"a packet of some length"]);
        let d = Document::new(MemSource(bytes.clone()));
        let mut e = Evaluator::new(ogg());
        let sum = at(&mut e, &d, 0, "crc32");
        let info = e.check_of(&d, &sum).unwrap().expect("a page checks itself");
        assert_eq!(info.algorithm, "crc32");
        assert_eq!(info.over, Some((0, bytes.len() as u64)));
        let blanked = info.blanked.expect("its own four bytes are read as something else");
        assert_eq!((blanked.at, blanked.len, blanked.byte), (22, 4, 0));
        let v = e.run_check(&d, &sum).unwrap().expect("the sum can be taken");
        assert!(v.ok, "computed {}, stored {}", v.computed, v.stored);
        // And the sum taken over the bytes as they sit there is a different
        // number, so the substitution is doing the work.
        assert_ne!(crc32_msb(&bytes, 0, 0), crc32_msb(&{ let mut z = bytes.clone(); z[22..26].fill(0); z }, 0, 0));
    }

    #[test]
    fn a_page_somebody_edited_no_longer_sums() {
        let mut bytes = page_bytes(7, 3, 0, &[b"a packet of some length"]);
        let last = bytes.len() - 1;
        bytes[last] ^= 0x20;
        let d = Document::new(MemSource(bytes));
        let mut e = Evaluator::new(ogg());
        assert!(!crc_ok(&mut e, &d, 0));
    }

    /// Several logical streams share a file and their pages alternate, so the
    /// reading follows the file rather than gathering by serial number.
    #[test]
    fn pages_of_two_streams_read_in_the_order_they_were_written() {
        let mut v = page_bytes(0xaa, 0, 0x02, &[b"stream one"]);
        v.extend_from_slice(&page_bytes(0xbb, 0, 0x02, &[b"stream two"]));
        v.extend_from_slice(&page_bytes(0xaa, 1, 0x00, &[b"one again"]));
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(ogg());
        assert_eq!(e.node(&d, &[0]).unwrap().child_count, 3);
        for (n, serial) in [(0usize, 0xaa), (1, 0xbb), (2, 0xaa)] {
            let p = at(&mut e, &d, n, "serial_number");
            assert_eq!(e.node(&d, &p).unwrap().value.as_int(), Some(serial), "page {n}");
            assert!(crc_ok(&mut e, &d, n), "page {n}");
        }
    }

    /// A packet of 255 bytes is laced as 255 and then 0, and the 0 is part of
    /// that packet's lacing rather than a packet of its own. One of 300 bytes
    /// is 255 and 45.
    #[test]
    fn a_lacing_run_and_its_closing_zero_measure_one_packet() {
        let long = vec![b'x'; 255];
        let longer = vec![b'y'; 300];
        let bytes = page_bytes(1, 0, 0, &[&long, &longer, b"tail"]);
        let d = Document::new(MemSource(bytes));
        let mut e = Evaluator::new(ogg());
        let table = path(&mut e, &d, &["pages", "0", "segment_table"]);
        assert_eq!(e.node(&d, &table).unwrap().child_count, 3);
        assert_eq!(int(&mut e, &d, &["pages", "0", "segment_table", "0", "packet_length"]), 255);
        let zero = path(&mut e, &d, &["pages", "0", "segment_table", "0", "segments"]);
        assert_eq!(e.node(&d, &zero).unwrap().child_count, 2);
        assert_eq!(int(&mut e, &d, &["pages", "0", "segment_table", "1", "packet_length"]), 300);
        assert_eq!(int(&mut e, &d, &["pages", "0", "segment_table", "2", "packet_length"]), 4);
        let packets = at(&mut e, &d, 0, "packets");
        assert_eq!(e.node(&d, &packets).unwrap().size_bits, (255 + 300 + 4) * 8);
        assert!(crc_ok(&mut e, &d, 0));
    }

    fn vorbis_id_bytes() -> Vec<u8> {
        let mut v = b"\x01vorbis".to_vec();
        v.extend_from_slice(&0u32.to_le_bytes());
        v.push(2);
        v.extend_from_slice(&44_100u32.to_le_bytes());
        v.extend_from_slice(&0i32.to_le_bytes());
        v.extend_from_slice(&128_000i32.to_le_bytes());
        v.extend_from_slice(&0i32.to_le_bytes());
        // 256 and 2048 samples: the exponents 8 and 11, low half first.
        v.push(0xb8);
        v.push(1);
        v
    }

    #[test]
    fn a_vorbis_identification_header_reads_its_fields_and_the_block_size_nibbles_low_first() {
        let bytes = page_with(5, 0, 0x02, 0, &[(vorbis_id_bytes(), true)]);
        let d = Document::new(MemSource(bytes));
        let mut e = Evaluator::new(ogg());
        let id = |rest: &'static str| -> Vec<&'static str> { vec!["pages", "0", "packets", "0", rest] };
        assert_eq!(type_name(&mut e, &d, &["pages", "0", "packets", "0"]), "VorbisIdentification");
        assert_eq!(int(&mut e, &d, &id("packet_type")), 1);
        assert_eq!(int(&mut e, &d, &id("audio_channels")), 2);
        assert_eq!(int(&mut e, &d, &id("audio_sample_rate")), 44_100);
        assert_eq!(int(&mut e, &d, &id("bitrate_nominal")), 128_000);
        assert_eq!(int(&mut e, &d, &id("blocksize_0")), 256);
        assert_eq!(int(&mut e, &d, &id("blocksize_1")), 2048);
        assert_eq!(int(&mut e, &d, &id("framing_flag")), 1);
        let packet = path(&mut e, &d, &["pages", "0", "packets", "0"]);
        assert_eq!(e.node(&d, &packet).unwrap().size_bits, 30 * 8);
    }

    fn comment_list(vendor: &str, comments: &[&str]) -> Vec<u8> {
        let mut v = (vendor.len() as u32).to_le_bytes().to_vec();
        v.extend_from_slice(vendor.as_bytes());
        v.extend_from_slice(&(comments.len() as u32).to_le_bytes());
        for c in comments {
            v.extend_from_slice(&(c.len() as u32).to_le_bytes());
            v.extend_from_slice(c.as_bytes());
        }
        v
    }

    /// The comment and setup headers usually share the second page, and the
    /// first audio packet starts the third.
    #[test]
    fn vorbis_comment_and_setup_headers_share_a_page_and_audio_packets_stay_bytes() {
        let mut comment = b"\x03vorbis".to_vec();
        comment.extend_from_slice(&comment_list("Xiph.Org libVorbis I 20200704", &["TITLE=Tone", "ENCODER=Lavc"]));
        comment.push(1);
        let mut setup = b"\x05vorbis".to_vec();
        setup.push(3);
        setup.extend_from_slice(b"BCV\x01\x00\x08\x00some packed codebooks");
        let mut v = page_with(5, 0, 0x02, 0, &[(vorbis_id_bytes(), true)]);
        v.extend_from_slice(&page_with(5, 1, 0, 0, &[(comment.clone(), true), (setup.clone(), true)]));
        v.extend_from_slice(&page_with(5, 2, 0x04, 1024, &[(vec![0x00, 0x11, 0x22], true)]));
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(ogg());
        assert_eq!(type_name(&mut e, &d, &["pages", "1", "packets", "0"]), "VorbisCommentHeader");
        assert_eq!(text(&mut e, &d, &["pages", "1", "packets", "0", "vendor"]), "Xiph.Org libVorbis I 20200704");
        assert_eq!(text(&mut e, &d, &["pages", "1", "packets", "0", "comments", "0", "text", "name"]), "TITLE");
        assert_eq!(text(&mut e, &d, &["pages", "1", "packets", "0", "comments", "1", "text", "value"]), "Lavc");
        assert_eq!(int(&mut e, &d, &["pages", "1", "packets", "0", "framing_flag"]), 1);
        assert_eq!(type_name(&mut e, &d, &["pages", "1", "packets", "1"]), "VorbisSetup");
        assert_eq!(int(&mut e, &d, &["pages", "1", "packets", "1", "codebook_count"]), 4);
        let setup_data = path(&mut e, &d, &["pages", "1", "packets", "1", "setup_data"]);
        assert_eq!(e.node(&d, &setup_data).unwrap().size_bits, (setup.len() as u64 - 8) * 8);
        assert!(type_name(&mut e, &d, &["pages", "2", "packets", "0"]).starts_with("bytes"));
        for page in 0..3 {
            assert!(crc_ok(&mut e, &d, page), "page {page}");
        }
    }

    /// A header that goes on to the next page is two pieces, and neither is
    /// read as a header: the first piece stops partway through it.
    #[test]
    fn a_header_split_across_pages_stays_bytes_on_both_pages() {
        let mut comment = b"\x03vorbis".to_vec();
        comment.extend_from_slice(&comment_list("vendor", &["COMMENT=".to_string().as_str(), &"x".repeat(600)]));
        comment.push(1);
        let (first, rest) = comment.split_at(510);
        let mut v = page_with(5, 1, 0, 0, &[(first.to_vec(), false)]);
        v.extend_from_slice(&page_with(5, 2, 0x01, 0, &[(rest.to_vec(), true)]));
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(ogg());
        assert_eq!(int(&mut e, &d, &["pages", "0", "segment_table", "0", "packet_ends"]), 0);
        assert!(type_name(&mut e, &d, &["pages", "0", "packets", "0"]).starts_with("bytes"));
        assert!(type_name(&mut e, &d, &["pages", "1", "packets", "0"]).starts_with("bytes"));
        assert!(crc_ok(&mut e, &d, 0) && crc_ok(&mut e, &d, 1));
    }

    #[test]
    fn opus_head_and_opus_tags_read_with_little_endian_numbers() {
        let mut head = b"OpusHead".to_vec();
        head.extend_from_slice(&[1, 2]);
        head.extend_from_slice(&312u16.to_le_bytes());
        head.extend_from_slice(&48_000u32.to_le_bytes());
        head.extend_from_slice(&(-256i16).to_le_bytes());
        head.push(0);
        let mut tags = b"OpusTags".to_vec();
        tags.extend_from_slice(&comment_list("libopus 1.4", &["ARTIST=Someone"]));
        tags.extend_from_slice(&[0; 3]);
        let mut v = page_with(9, 0, 0x02, 0, &[(head, true)]);
        v.extend_from_slice(&page_with(9, 1, 0, 0, &[(tags, true)]));
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(ogg());
        let h = |rest: &'static str| -> Vec<&'static str> { vec!["pages", "0", "packets", "0", rest] };
        assert_eq!(type_name(&mut e, &d, &["pages", "0", "packets", "0"]), "OpusHead");
        assert_eq!(int(&mut e, &d, &h("output_channel_count")), 2);
        assert_eq!(int(&mut e, &d, &h("pre_skip")), 312);
        assert_eq!(int(&mut e, &d, &h("input_sample_rate")), 48_000);
        let gain = path(&mut e, &d, &h("output_gain"));
        assert_eq!(e.node(&d, &gain).unwrap().value, Value::Float(-1.0));
        assert_eq!(type_name(&mut e, &d, &["pages", "1", "packets", "0"]), "OpusTags");
        assert_eq!(text(&mut e, &d, &["pages", "1", "packets", "0", "vendor"]), "libopus 1.4");
        assert_eq!(text(&mut e, &d, &["pages", "1", "packets", "0", "comments", "0", "text", "value"]), "Someone");
        let extra = path(&mut e, &d, &["pages", "1", "packets", "0", "extra"]);
        assert_eq!(e.node(&d, &extra).unwrap().size_bits, 3 * 8);
    }

    /// An Ogg FLAC stream: the mapping header with STREAMINFO, a header packet
    /// holding a VORBIS_COMMENT block, and an audio packet holding a frame.
    #[test]
    fn ogg_flac_reads_its_mapping_header_its_metadata_blocks_and_its_frames() {
        let mut si = Vec::new();
        si.extend_from_slice(&4096u16.to_be_bytes());
        si.extend_from_slice(&4096u16.to_be_bytes());
        si.extend_from_slice(&[0; 6]);
        let packed: u64 = (16_000u64 << 44) | (15 << 36) | 16_000;
        si.extend_from_slice(&packed.to_be_bytes());
        si.extend_from_slice(&[0; 16]);
        let mut mapping = b"\x7fFLAC\x01\x00\x00\x01fLaC".to_vec();
        mapping.push(0x00);
        mapping.extend_from_slice(&(si.len() as u32).to_be_bytes()[1..]);
        mapping.extend_from_slice(&si);
        let vc = comment_list("Lavf", &["TITLE=Tone"]);
        let mut block = vec![0x84];
        block.extend_from_slice(&(vc.len() as u32).to_be_bytes()[1..]);
        block.extend_from_slice(&vc);
        // A frame: fixed blocking, 4096 samples, 16 kHz, mono, 16 bits,
        // frame number 0.
        let mut frame = vec![0xff, 0xf8, 0xc5, 0x08, 0x00];
        frame.push(crate::checksum::crc8_flac(&frame));
        frame.extend_from_slice(&[0x55; 9]);
        let sum = crate::checksum::crc16_flac(&frame);
        frame.extend_from_slice(&sum.to_be_bytes());
        let mut v = page_with(3, 0, 0x02, 0, &[(mapping, true)]);
        v.extend_from_slice(&page_with(3, 1, 0, 0, &[(block, true)]));
        v.extend_from_slice(&page_with(3, 2, 0x04, 4096, &[(frame, true)]));
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(ogg());
        assert_eq!(type_name(&mut e, &d, &["pages", "0", "packets", "0"]), "OggFlacHeader");
        assert_eq!(int(&mut e, &d, &["pages", "0", "packets", "0", "header_packet_count"]), 1);
        assert_eq!(int(&mut e, &d, &["pages", "0", "packets", "0", "stream_info", "data", "sample_rate"]), 16_000);
        assert_eq!(int(&mut e, &d, &["pages", "1", "packets", "0", "block_type"]), 4);
        assert_eq!(int(&mut e, &d, &["pages", "1", "packets", "0", "last_block"]), 1);
        assert_eq!(text(&mut e, &d, &["pages", "1", "packets", "0", "data", "comments", "0", "text", "value"]), "Tone");
        assert_eq!(type_name(&mut e, &d, &["pages", "2", "packets", "0"]), "Frame");
        assert_eq!(int(&mut e, &d, &["pages", "2", "packets", "0", "header", "block_size"]), 4096);
        for check in ["crc8", "crc16"] {
            let p = path(&mut e, &d, &["pages", "2", "packets", "0", check]);
            assert!(e.run_check(&d, &p).unwrap().expect("a verdict").ok, "{check}");
        }
        for page in 0..3 {
            assert!(crc_ok(&mut e, &d, page), "page {page}");
        }
    }

    /// An audio packet on a page with a granule position can look like a
    /// metadata block by chance, so a block is only read on a header page.
    #[test]
    fn a_packet_shaped_like_a_metadata_block_on_an_audio_page_stays_bytes() {
        let packet = vec![0x04, 0x00, 0x00, 0x02, 0xaa, 0xbb];
        let bytes = page_with(3, 5, 0, 8192, &[(packet, true)]);
        let d = Document::new(MemSource(bytes));
        let mut e = Evaluator::new(ogg());
        assert!(type_name(&mut e, &d, &["pages", "0", "packets", "0"]).starts_with("bytes"));
    }
}
