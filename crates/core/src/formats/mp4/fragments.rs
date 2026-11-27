//! Movie fragments: a movie written in pieces, each a `moof` saying what is in
//! the `mdat` after it, so that a recording can be read before it is finished
//! and a stream can start anywhere.
//!
//! The `moov` of a fragmented file has empty sample tables and an `mvex` with
//! a `trex` per track, giving the defaults. Each `moof` has a `traf` per track:
//! `tfhd` says which track and where its data counts from, `tfdt` when the
//! fragment starts, and each `trun` lists a run of samples, with whatever of
//! duration, size, flags and composition offset is not left to a default.
//!
//! Each `trun` places its samples, as `sample_data`. A run's samples are one
//! after another in the file, so the run is one stretch starting at its data
//! offset, and each sample in it is as long as the run or the defaults say.

use super::tables::dependency_fields;
use super::{bits, by_version, cc, full_box, full_box_flags, i32be, media, u16be, u32be};
use crate::template::{Endian::*, Expr as E, Ty as T};

pub(super) fn boxes() -> Vec<(&'static str, T)> {
    vec![
        // The data offsets of a track run count from the start of the moof,
        // and the origin is how a run inside it finds that start.
        ("moof", T::origin(super::child_boxes())),
        ("mehd", mehd()),
        ("trex", trex()),
        ("mfhd", mfhd()),
        ("tfhd", tfhd()),
        ("tfdt", tfdt()),
        ("trun", trun()),
        ("tfra", tfra()),
        ("mfro", mfro()),
        ("sidx", sidx()),
        ("prft", prft()),
        ("emsg", emsg()),
    ]
}

/// The 32 bits a fragment uses to say what kind of sample something is: the
/// four dependency fields `sdtp` has, a padding count, whether it is a sync
/// sample, and a priority.
fn sample_flags() -> T {
    let mut fields = vec![("reserved", bits(4))];
    fields.extend(dependency_fields());
    fields.extend(vec![("padding_value", bits(3)), ("is_non_sync_sample", bits(1)), ("degradation_priority", bits(16))]);
    T::structure("SampleFlags", fields)
        .field_doc("is_non_sync_sample", "0: a sync sample, which a player can start decoding from. 1: any other sample.")
}

fn mehd() -> T {
    let mut fields = full_box();
    fields.push(("fragment_duration", by_version(u32be(), T::u64(Big))));
    T::structure("MovieExtendsHeader", fields).field_doc("fragment_duration", "How long the whole movie is, including every fragment, in the movie's timescale.")
}

fn trex() -> T {
    let mut fields = full_box();
    fields.extend(vec![
        ("track_id", u32be()),
        ("default_sample_description_index", u32be()),
        ("default_sample_duration", u32be()),
        ("default_sample_size", u32be()),
        ("default_sample_flags", sample_flags()),
    ]);
    T::structure("TrackExtends", fields)
        .doc("The defaults a track's fragments use for anything their tfhd and trun boxes leave out.")
}

fn mfhd() -> T {
    let mut fields = full_box();
    fields.push(("sequence_number", u32be()));
    T::structure("MovieFragmentHeader", fields).field_doc("sequence_number", "This fragment's number. Fragments are numbered in order, from 1.")
}

const TFHD_FLAGS: &[(u32, &str)] = &[
    (0, "base data offset present"),
    (1, "sample description index present"),
    (3, "default sample duration present"),
    (4, "default sample size present"),
    (5, "default sample flags present"),
    (16, "duration is empty"),
    (17, "default base is moof"),
];

fn tfhd() -> T {
    let flag = |b: u32| E::field("flags").bit(b);
    let mut fields = full_box_flags("TrackFragmentFlags", TFHD_FLAGS);
    fields.extend(vec![
        ("track_id", u32be()),
        ("base_data_offset", T::when(flag(0), T::u64(Big))),
        ("sample_description_index", T::when(flag(1), u32be())),
        ("default_sample_duration", T::when(flag(3), u32be())),
        ("default_sample_size", T::when(flag(4), u32be())),
        ("default_sample_flags", T::when(flag(5), sample_flags())),
    ]);
    T::structure("TrackFragmentHeader", fields)
        .positions(&["base_data_offset"])
        .field_doc("base_data_offset", "Where in the file this fragment's data offsets count from. Without it they count from the start of the moof.")
}

fn tfdt() -> T {
    let mut fields = full_box();
    fields.push(("base_media_decode_time", by_version(u32be(), T::u64(Big))));
    T::structure("TrackFragmentDecodeTime", fields)
        .field_doc("base_media_decode_time", "When the first sample of this fragment is decoded, in the media's timescale: the durations of every earlier sample of the track added up.")
}

const TRUN_FLAGS: &[(u32, &str)] = &[
    (0, "data offset present"),
    (2, "first sample flags present"),
    (8, "sample duration present"),
    (9, "sample size present"),
    (10, "sample flags present"),
    (11, "sample composition time offset present"),
];

/// A field of this traf's `tfhd`.
fn tfhd_field(field: &str) -> E {
    E::sibling_tagged(&["type"], E::lit(cc("tfhd")), &["body", field])
}

/// The boxes of the `moov` this fragment belongs to, searched from inside a
/// fragment: `moov` is an earlier box of the file.
fn moov() -> E {
    E::sibling_tagged(&["type"], E::lit(cc("moov")), &["body"])
}

/// `field` of the first sample entry of the track this fragment is about,
/// found by its track ID: the trak whose tkhd has it, then down to stsd.
fn track_entry(field: &[&str]) -> E {
    let trak = E::tagged_in_by(moov(), &["body", "0", "body", "times", "track_id"], tfhd_field("track_id"), &["body"]);
    let mdia = E::tagged_in(trak, &["type"], cc("mdia"), &["body"]);
    let minf = E::tagged_in(mdia, &["type"], cc("minf"), &["body"]);
    let stbl = E::tagged_in(minf, &["type"], cc("stbl"), &["body", "boxes"]);
    let mut path = vec!["body", "entries", "0"];
    path.extend_from_slice(field);
    E::tagged_in(stbl, &["type"], cc("stsd"), &path)
}

/// The track's default sample size from its `trex`, for a run whose `tfhd`
/// gives none.
fn trex_default_size() -> E {
    let mvex = E::tagged_in(moov(), &["type"], cc("mvex"), &["body"]);
    E::tagged_in_by(mvex, &["body", "track_id"], tfhd_field("track_id"), &["body", "default_sample_size"])
}

fn trun() -> T {
    let flag = |b: u32| E::field("flags").bit(b);
    let sample = T::structure(
        "TrackRunSample",
        vec![
            ("duration", T::when(flag(8), u32be())),
            ("size", T::when(flag(9), u32be())),
            ("sample_flags", T::when(flag(10), sample_flags())),
            ("composition_time_offset", T::when(flag(11), by_version(u32be(), i32be()))),
        ],
    );
    let mut fields = full_box_flags("TrackRunFlags", TRUN_FLAGS);
    fields.extend(vec![
        ("sample_count", u32be()),
        ("data_offset", T::when(flag(0), i32be())),
        ("first_sample_flags", T::when(flag(2), sample_flags())),
        ("samples", T::array(sample, E::field("sample_count"))),
        ("sample_data", run_data()),
    ]);
    T::structure("TrackRun", fields)
        .field_doc("data_offset", "Where this run's sample data starts, from the tfhd's base data offset or else from the start of the moof.")
        .field_doc("first_sample_flags", "Flags for the first sample only, so that a run can start with a sync sample and use the default flags for the rest.")
        .field_named_from("sample_data", E::words("track {}", tfhd_field("track_id")))
        .field_doc("sample_data", "This run's samples, in the mdat at data_offset.")
}

/// The run's samples where they are: from the base the `tfhd` gives, or from
/// the start of the `moof` when it gives none, plus the run's data offset.
///
/// Counting from the `moof` is what `default-base-is-moof` says, and what ISO
/// 14496-12 says for the first track fragment of a `moof` without a base. A
/// second track fragment without either counts from where the first one's
/// data ends, which no writer in the sample collection does and which is not
/// followed here.
fn run_data() -> T {
    let flag = |b: u32| E::field("flags").bit(b);
    let offset = E::cond(flag(0), E::field("data_offset"), E::lit(0));
    let has_base = tfhd_field("flags").bit(0);
    let config = |kind: &str| {
        E::tagged_in(track_entry(&["body", "boxes"]), &["type"], cc(kind), &["body", "length_size_minus_one"])
    };
    let size = E::cond(flag(9), E::elem_field("samples", E::idx(), &["size"]), E::field("default_size"));
    let data = T::structure(
        "TrackRunData",
        vec![
            ("format", T::enumeration_hex("SampleFormat", T::computed(track_entry(&["format"])), media::FORMATS)),
            (
                "nal_length_size",
                T::when(media::is_nal_format(E::field("format")), T::computed(config("avcC").or(config("hvcC")).add(E::lit(1)))),
            ),
            ("channel_count", T::when(media::is_pcm_format(E::field("format")), T::computed(track_entry(&["body", "channel_count"])))),
            (
                "default_size",
                T::when(
                    flag(9).negate(),
                    T::computed(E::cond(tfhd_field("flags").bit(4), tfhd_field("default_sample_size"), trex_default_size())),
                ),
            ),
            ("data", T::array(T::sized(size, T::Named("SampleContent".into())), E::field("sample_count"))),
        ],
    )
    .contents("data")
    .machinery(&["nal_length_size", "channel_count", "default_size"])
    .field_elem_named_from("data", E::words("sample {}", E::idx().add(E::lit(1))))
    .field_doc("format", "This track's sample entry format, from its stsd in the moov. Each sample is read according to this format.")
    .field_doc("default_size", "The size in bytes of every sample in this run, from the tfhd or else the track's trex in the moov.");
    // Where the moof starts in the file. The moof's boxes are read inside an
    // origin, which is the first byte after its eight-byte header, and an
    // offset counted from an origin cannot reach past the moof. So the origin
    // is turned into a file offset: this field starts where `samples` ends,
    // and `samples` starts `start_of(samples)` bytes after the origin.
    let moof = E::SpacePos.sub(E::size_of("samples")).sub(E::start_of(E::field("samples"))).sub(E::lit(8));
    let base = E::cond(has_base, tfhd_field("base_data_offset"), moof);
    T::at(base.add(offset), data)
}

fn tfra() -> T {
    let width = |field: &str| T::uint_expr(E::field(field).add(E::lit(1)).mul(E::lit(8)), Big);
    let entry = T::structure(
        "RandomAccessEntry",
        vec![
            ("time", by_version(u32be(), T::u64(Big))),
            ("moof_offset", by_version(u32be(), T::u64(Big))),
            ("traf_number", width("length_size_of_traf_num")),
            ("trun_number", width("length_size_of_trun_num")),
            ("sample_number", width("length_size_of_sample_num")),
        ],
    )
    .positions(&["moof_offset"])
    .field_doc("time", "When the sample plays, in the media's timescale.")
    .field_doc("moof_offset", "The file offset of the moof that holds the sample.")
    .field_doc("traf_number", "Which traf of the moof at moof_offset, counting from 1. trun_number and sample_number then pick the trun and the sample, also counting from 1.");
    let mut fields = full_box();
    fields.extend(vec![
        ("track_id", u32be()),
        ("reserved", bits(26)),
        ("length_size_of_traf_num", bits(2)),
        ("length_size_of_trun_num", bits(2)),
        ("length_size_of_sample_num", bits(2)),
        ("number_of_entry", u32be()),
        ("entries", T::array(entry, E::field("number_of_entry"))),
    ]);
    T::structure("TrackFragmentRandomAccess", fields)
        .doc("Where the sync samples of one track are, for seeking in a fragmented file.")
        .field_doc("length_size_of_traf_num", "How many bytes each traf_number has, less one. The next two fields give the same for trun_number and sample_number.")
}

fn mfro() -> T {
    let mut fields = full_box();
    fields.push(("size", u32be()));
    T::structure("MovieFragmentRandomAccessOffset", fields)
        .field_doc("size", "How many bytes the mfra box has, this box included, so a reader can find the mfra from the end of the file.")
}

/// A segment index: how a DASH or HLS segment divides into subsegments.
fn sidx() -> T {
    let reference = T::structure(
        "SegmentReference",
        vec![
            ("reference_type", T::enumeration("ReferenceType", bits(1), &[(0, "media"), (1, "segment index")])),
            ("referenced_size", bits(31)),
            ("subsegment_duration", u32be()),
            ("starts_with_sap", bits(1)),
            ("sap_type", bits(3)),
            ("sap_delta_time", bits(28)),
        ],
    )
    .field_doc("referenced_size", "The size in bytes of this subsegment. It starts where the previous one ends.");
    let mut fields = full_box();
    fields.extend(vec![
        ("reference_id", u32be()),
        ("timescale", u32be()),
        ("earliest_presentation_time", by_version(u32be(), T::u64(Big))),
        ("first_offset", by_version(u32be(), T::u64(Big))),
        ("reserved", u16be()),
        ("reference_count", u16be()),
        ("references", T::array(reference, E::field("reference_count"))),
    ]);
    T::structure("SegmentIndex", fields)
        .positions(&["first_offset"])
        .field_doc("first_offset", "How many bytes after the end of this box the first subsegment starts.")
}

/// Producer reference time: the wall clock time at which a sample was made.
fn prft() -> T {
    let mut fields = full_box();
    fields.extend(vec![
        ("reference_track_id", u32be()),
        ("ntp_timestamp", T::u64(Big)),
        ("media_time", by_version(u32be(), T::u64(Big))),
    ]);
    T::structure("ProducerReferenceTime", fields)
        .field_doc("ntp_timestamp", "Seconds since 1900 in the high 32 bits and a fraction of a second in the low 32, the way NTP writes a time.")
}

/// An event message, such as an ad marker in a live stream.
fn emsg() -> T {
    let v0 = T::structure(
        "EventMessageV0",
        vec![
            ("scheme_id_uri", T::cstr()),
            ("value", T::cstr()),
            ("timescale", u32be()),
            ("presentation_time_delta", u32be()),
            ("event_duration", u32be()),
            ("id", u32be()),
            ("message_data", T::bytes(E::Remaining)),
        ],
    );
    let v1 = T::structure(
        "EventMessageV1",
        vec![
            ("timescale", u32be()),
            ("presentation_time", T::u64(Big)),
            ("event_duration", u32be()),
            ("id", u32be()),
            ("scheme_id_uri", T::cstr()),
            ("value", T::cstr()),
            ("message_data", T::bytes(E::Remaining)),
        ],
    );
    let mut fields = full_box();
    fields.push(("message", by_version(v0, v1)));
    T::structure("EventMessage", fields)
}

#[cfg(test)]
mod tests {
    use super::super::mp4;
    use super::super::tests::{boxed, cat, full, value, walk};
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn u32s(values: &[u32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_be_bytes()).collect()
    }

    /// A moov for one track, ID 1, whose samples are in a format nothing
    /// reads further, with a trex giving `default_size`.
    fn moov(default_size: u32) -> Vec<u8> {
        let mut tkhd = u32s(&[0, 0, 1, 0, 0]);
        tkhd.extend_from_slice(&[0; 60]);
        let tkhd = full(b"tkhd", 0, 3, &tkhd);
        let entry = boxed(b"zzzz", &[0, 0, 0, 0, 0, 0, 0, 1]);
        let stsd = full(b"stsd", 0, 0, &cat(&[&u32s(&[1]), &entry]));
        let stbl = boxed(b"stbl", &stsd);
        let trak = boxed(b"trak", &cat(&[&tkhd, &boxed(b"mdia", &boxed(b"minf", &stbl))]));
        let trex = full(b"trex", 0, 0, &u32s(&[1, 1, 0, default_size, 0]));
        boxed(b"moov", &cat(&[&trak, &boxed(b"mvex", &trex)]))
    }

    fn moof(tfhd: &[u8], trun: &[u8]) -> Vec<u8> {
        let mfhd = full(b"mfhd", 0, 0, &u32s(&[1]));
        boxed(b"moof", &cat(&[&mfhd, &boxed(b"traf", &cat(&[tfhd, trun]))]))
    }

    #[test]
    fn a_run_counts_its_data_offset_from_the_moof_and_reads_its_own_sizes() {
        let moov = moov(0);
        // default-base-is-moof, and a run with a data offset and a size per
        // sample. The moof is the same length whatever the offset says.
        let tfhd = full(b"tfhd", 0, 0x02_0000, &u32s(&[1]));
        let run = |offset: u32| full(b"trun", 0, 0x201, &u32s(&[2, offset, 3, 5]));
        let moof_len = moof(&tfhd, &run(0)).len() as u32;
        let moof = moof(&tfhd, &run(moof_len + 8));
        let mdat = boxed(b"mdat", &[1, 2, 3, 4, 5, 6, 7, 8]);
        let moof_at = moov.len() as u64;
        let file = cat(&[&moov, &moof, &mdat]);
        let d = Document::new(MemSource(file));
        let mut ev = Evaluator::new(mp4());

        let second = walk(&mut ev, &d, &["1", "body", "1", "body", "1", "body", "sample_data", "0", "data", "1"]);
        let node = ev.node(&d, &second).unwrap();
        assert_eq!(node.offset_bits / 8, moof_at + moof_len as u64 + 8 + 3);
        assert_eq!(node.size_bits, 5 * 8);
        let p = ev.locate(&d, (moof_at + moof_len as u64 + 8 + 4) * 8).unwrap();
        assert!(p.starts_with(&second), "{p:?} is not in {second:?}");
    }

    #[test]
    fn a_random_access_entry_reads_numbers_as_wide_as_the_box_says() {
        // Version 1, track 1; traf numbers one byte, trun numbers two, sample
        // numbers four.
        let mut body = u32s(&[1, 0b00_01_11]);
        body.extend_from_slice(&u32s(&[1]));
        body.extend_from_slice(&9000u64.to_be_bytes());
        body.extend_from_slice(&1239u64.to_be_bytes());
        body.extend_from_slice(&[1, 0, 2]);
        body.extend_from_slice(&u32s(&[7]));
        let d = Document::new(MemSource(full(b"tfra", 1, 0, &body)));
        let mut ev = Evaluator::new(mp4());
        let entry = |name: &'static str| -> Vec<&'static str> { vec!["0", "body", "entries", "0", name] };
        assert_eq!(value(&mut ev, &d, &entry("time")), Value::UInt(9000));
        assert_eq!(value(&mut ev, &d, &entry("moof_offset")), Value::UInt(1239));
        assert_eq!(value(&mut ev, &d, &entry("traf_number")), Value::UInt(1));
        assert_eq!(value(&mut ev, &d, &entry("trun_number")), Value::UInt(2));
        assert_eq!(value(&mut ev, &d, &entry("sample_number")), Value::UInt(7));
    }

    #[test]
    fn a_run_without_sizes_takes_the_default_from_trex_and_counts_from_the_base() {
        let moov = moov(4);
        let trun = full(b"trun", 0, 0, &u32s(&[2]));
        // The base data offset is the mdat's first byte of data; the moof is
        // as long whatever the offset's value.
        let probe = moof(&full(b"tfhd", 0, 1, &cat(&[&u32s(&[1]), &0u64.to_be_bytes()])), &trun);
        let base = (moov.len() + probe.len() + 8) as u64;
        let tfhd = full(b"tfhd", 0, 1, &cat(&[&u32s(&[1]), &base.to_be_bytes()]));
        let file = cat(&[&moov, &moof(&tfhd, &trun), &boxed(b"mdat", &[9; 8])]);
        let d = Document::new(MemSource(file));
        let mut ev = Evaluator::new(mp4());

        let data = ["1", "body", "1", "body", "1", "body", "sample_data", "0"];
        let at = |names: &[&'static str]| -> Vec<&'static str> { data.iter().chain(names).copied().collect() };
        assert_eq!(value(&mut ev, &d, &at(&["default_size"])), Value::Int(4));
        let second = walk(&mut ev, &d, &at(&["data", "1"]));
        let node = ev.node(&d, &second).unwrap();
        assert_eq!(node.offset_bits / 8, base + 4);
        assert_eq!(node.size_bits, 32);
    }
}
