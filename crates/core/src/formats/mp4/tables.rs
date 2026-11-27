//! The sample table, `stbl`: where each sample of a track is, how big it is,
//! when it plays, and which samples a player can start from.
//!
//! The tables never say "sample 37 is at byte X". They say it in three parts.
//! `stco` (or `co64` past four gigabytes) lists where each chunk starts in the
//! file, a chunk being a run of samples written one after another. `stsc` says
//! how many samples each chunk holds, as runs of chunks that hold the same
//! number. `stsz` gives each sample's size. A sample's place is its chunk's
//! offset plus the sizes of the samples before it in that chunk.
//!
//! [`sample_table`] works that out and places every sample over the `mdat` it
//! is in, as `sample_data`. The three tables are read a second time inside it,
//! because what an element of a list can name is the fields declared before
//! it, and the tables are elements of the `stbl` box list rather than fields.

use super::{bits, by_version, cc, child_boxes, fourcc, full_box, full_box_flags, i16be, i32be, media, u16be, u32be};
use crate::template::{Endian::*, Expr as E, Ty as T, Until};

/// Every table box this module reads, by type.
pub(super) fn boxes() -> Vec<(&'static str, T)> {
    vec![
        ("stts", time_to_sample()),
        ("ctts", composition_offsets()),
        ("cslg", composition_shift()),
        ("stsc", sample_to_chunk()),
        ("stsz", sample_sizes()),
        ("stz2", compact_sample_sizes()),
        ("stco", chunk_offsets(false)),
        ("co64", chunk_offsets(true)),
        ("stss", sync_samples("SyncSamples")),
        ("stps", sync_samples("PartialSyncSamples")),
        ("stsh", shadow_sync()),
        ("sdtp", sample_dependencies()),
        ("sgpd", sample_group_descriptions()),
        ("sbgp", sample_to_group()),
        ("subs", sub_samples()),
        ("saiz", aux_info_sizes()),
        ("saio", aux_info_offsets()),
    ]
}

pub(super) fn counted_full_box(name: &str, entry: T) -> T {
    let mut fields = full_box();
    fields.extend(vec![("entry_count", u32be()), ("entries", T::array(entry, E::field("entry_count")))]);
    T::structure(name, fields)
}

fn time_to_sample() -> T {
    counted_full_box(
        "TimeToSample",
        T::structure("TimeToSampleEntry", vec![("sample_count", u32be()), ("sample_delta", u32be())])
            .field_doc("sample_delta", "How long each of these samples lasts, in the media's timescale (mdhd)."),
    )
}

fn composition_offsets() -> T {
    counted_full_box(
        "CompositionOffsets",
        T::structure("CompositionOffsetEntry", vec![("sample_count", u32be()), ("sample_offset", by_version(u32be(), i32be()))])
            .field_doc("sample_offset", "Composition time minus decode time for these samples, in the media's timescale."),
    )
}

fn composition_shift() -> T {
    let n = || by_version(i32be(), T::Int { bits: 64, endian: Big });
    let mut fields = full_box();
    fields.extend(vec![
        ("composition_to_dts_shift", n()),
        ("least_decode_to_display_delta", n()),
        ("greatest_decode_to_display_delta", n()),
        ("composition_start_time", n()),
        ("composition_end_time", n()),
    ]);
    T::structure("CompositionShift", fields)
}

fn sample_to_chunk() -> T {
    counted_full_box(
        "SampleToChunk",
        T::structure(
            "SampleToChunkEntry",
            vec![("first_chunk", u32be()), ("samples_per_chunk", u32be()), ("sample_description_index", u32be())],
        )
        .field_doc("first_chunk", "The first chunk of a run of chunks that hold the same number of samples, counting from 1. The run ends at the chunk before the next entry's first_chunk.")
        .field_doc("sample_description_index", "Which entry of stsd describes the samples in this run of chunks, counting from 1."),
    )
}

fn sample_sizes() -> T {
    let mut fields = full_box();
    fields.extend(vec![
        ("sample_size", u32be()),
        ("sample_count", u32be()),
        ("entry_sizes", T::when(E::field("sample_size").equal_to(E::lit(0)), T::array(u32be(), E::field("sample_count")))),
    ]);
    T::structure("SampleSizes", fields)
        .field_doc("sample_size", "The size of every sample in bytes, when they are all the same size. 0 means each size is listed in entry_sizes.")
}

/// `stz2`: the same table with each size in 4, 8 or 16 bits.
fn compact_sample_sizes() -> T {
    let mut fields = full_box();
    fields.extend(vec![
        ("reserved", bits(24)),
        ("field_size", T::u8()),
        ("sample_count", u32be()),
        ("entry_sizes", T::array(T::uint_expr(E::field("field_size"), Big), E::field("sample_count"))),
    ]);
    T::structure("CompactSampleSizes", fields).field_doc("field_size", "How many bits each entry size has: 4, 8 or 16.")
}

fn chunk_offsets(wide: bool) -> T {
    counted_full_box(if wide { "ChunkOffsets64" } else { "ChunkOffsets32" }, if wide { T::u64(Big) } else { u32be() }).positions(&["entries"])
}

fn sync_samples(name: &str) -> T {
    counted_full_box(name, u32be())
}

fn shadow_sync() -> T {
    counted_full_box(
        "ShadowSync",
        T::structure("ShadowSyncEntry", vec![("shadowed_sample_number", u32be()), ("sync_sample_number", u32be())]),
    )
}

const LEADING: &[(i128, &str)] = &[(0, "unknown"), (1, "leading, not decodable"), (2, "not leading"), (3, "leading, decodable")];
const DEPENDS_ON: &[(i128, &str)] = &[(0, "unknown"), (1, "depends on other samples"), (2, "independent"), (3, "reserved")];
const DEPENDED_ON: &[(i128, &str)] = &[(0, "unknown"), (1, "other samples depend on it"), (2, "disposable"), (3, "reserved")];
const REDUNDANCY: &[(i128, &str)] = &[(0, "unknown"), (1, "has redundant coding"), (2, "no redundant coding"), (3, "reserved")];

/// The four two-bit fields `sdtp` and the sample flags of a fragment share.
pub(super) fn dependency_fields() -> Vec<(&'static str, T)> {
    vec![
        ("is_leading", T::enumeration("Leading", bits(2), LEADING)),
        ("depends_on", T::enumeration("DependsOn", bits(2), DEPENDS_ON)),
        ("is_depended_on", T::enumeration("DependedOn", bits(2), DEPENDED_ON)),
        ("has_redundancy", T::enumeration("Redundancy", bits(2), REDUNDANCY)),
    ]
}

/// `sdtp`: a byte per sample, for as many samples as the track has.
fn sample_dependencies() -> T {
    let mut fields = full_box();
    fields.push((
        "samples",
        T::repeat(
            T::structure("SampleDependency", dependency_fields())
                .field_doc("depends_on", "2 (independent): an I-frame, decodable on its own. 1 (depends on other samples): a P- or B-frame."),
            Until::End,
        ),
    ));
    T::structure("SampleDependencies", fields)
}

/// `sgpd`: what each group of a grouping means. Version 1 says how long each
/// entry is, and version 0 left that to the grouping type, so an entry of a
/// type not read here runs to the end of the box in version 0.
fn sample_group_descriptions() -> T {
    let version = E::field("version");
    let entry = T::switch(
        E::field("grouping_type"),
        vec![
            (cc("roll"), roll_entry("RollRecovery")),
            (cc("prol"), roll_entry("PreRoll")),
            (cc("sync"), T::structure("SyncSampleEntry", vec![("reserved", bits(2)), ("nal_unit_type", bits(6))])),
            (
                cc("rap "),
                T::structure("RandomAccessPoint", vec![("num_leading_samples_known", bits(1)), ("num_leading_samples", bits(7))]),
            ),
            (cc("tele"), T::structure("TemporalLevel", vec![("level_independently_decodable", bits(1)), ("reserved", bits(7))])),
            (cc("seig"), encryption_entry()),
        ],
        T::bytes(E::Remaining),
    );
    let sized_entry = T::structure(
        "SampleGroupEntry",
        vec![
            ("description_length", T::when(E::field("default_length").equal_to(E::lit(0)), u32be())),
            (
                "entry",
                T::sized(
                    E::cond(E::field("default_length").equal_to(E::lit(0)), E::field("description_length"), E::field("default_length")).at_most(E::Remaining),
                    entry.clone(),
                ),
            ),
        ],
    );
    let mut fields = full_box();
    fields.extend(vec![
        ("grouping_type", fourcc()),
        ("default_length", T::when(version.clone().equal_to(E::lit(1)), u32be())),
        ("default_sample_description_index", T::when(version.clone().greater_or_equal(E::lit(2)), u32be())),
        ("entry_count", u32be()),
        ("entries", T::array(T::switch(version.equal_to(E::lit(1)), vec![(1, sized_entry)], entry), E::field("entry_count"))),
    ]);
    T::structure("SampleGroupDescription", fields)
        .field_doc("grouping_type", "What kind of grouping this is. roll: roll recovery, such as audio pre-roll. sync: the NAL unit type of sync samples. rap: random access points. seig: encryption.")
        .field_doc("default_length", "How long every entry is, in bytes. 0 means each entry starts with its own length.")
}

fn roll_entry(name: &str) -> T {
    T::structure(name, vec![("roll_distance", i16be())])
        .field_doc("roll_distance", "How many samples before (negative) or after (positive) a sample of this group must be decoded for the output to be correct.")
}

/// Common encryption's default key for a group of samples.
fn encryption_entry() -> T {
    T::structure(
        "EncryptionGroup",
        vec![
            ("reserved", T::u8()),
            ("crypt_byte_block", bits(4)),
            ("skip_byte_block", bits(4)),
            ("is_protected", T::u8()),
            ("per_sample_iv_size", T::u8()),
            ("kid", T::bytes(E::lit(16))),
            ("constant_iv_size", T::when(E::field("is_protected").equal_to(E::lit(1)).both(E::field("per_sample_iv_size").equal_to(E::lit(0))), T::u8())),
            ("constant_iv", T::when(E::field("is_protected").equal_to(E::lit(1)).both(E::field("per_sample_iv_size").equal_to(E::lit(0))), T::bytes(E::field("constant_iv_size")))),
        ],
    )
    .field_doc("kid", "The ID of the key the samples are encrypted with.")
}

/// `sbgp`: which group each run of samples belongs to.
fn sample_to_group() -> T {
    let mut fields = full_box();
    fields.extend(vec![
        ("grouping_type", fourcc()),
        ("grouping_type_parameter", T::when(E::field("version").equal_to(E::lit(1)), u32be())),
        ("entry_count", u32be()),
        (
            "entries",
            T::array(
                T::structure("SampleToGroupEntry", vec![("sample_count", u32be()), ("group_description_index", u32be())])
                    .field_doc("group_description_index", "The entry, counting from 1, of the sgpd with the same grouping type that describes these samples. 0: no group. From 0x10001: an entry of the sgpd in this movie fragment, 0x10001 being its entry 1."),
                E::field("entry_count"),
            ),
        ),
    ]);
    T::structure("SampleToGroup", fields)
}

/// `subs`: how samples divide into sub-samples, such as the NAL units of a
/// video sample.
fn sub_samples() -> T {
    let sub = T::structure(
        "SubSample",
        vec![
            ("subsample_size", by_version(u16be(), u32be())),
            ("subsample_priority", T::u8()),
            ("discardable", T::u8()),
            ("codec_specific_parameters", u32be()),
        ],
    );
    let entry = T::structure(
        "SubSampleEntry",
        vec![("sample_delta", u32be()), ("subsample_count", u16be()), ("subsamples", T::array(sub, E::field("subsample_count")))],
    )
    .field_doc("sample_delta", "This entry's sample number minus the previous entry's sample number. In the first entry, the sample number itself, counting from 1.");
    counted_full_box("SubSampleInformation", entry)
}

fn aux_info_type() -> Vec<(&'static str, T)> {
    vec![
        ("aux_info_type", T::when(E::field("flags").bit(0), fourcc())),
        ("aux_info_type_parameter", T::when(E::field("flags").bit(0), u32be())),
    ]
}

fn aux_info_sizes() -> T {
    let mut fields = full_box_flags("AuxInfoFlags", &[(0, "type present")]);
    fields.extend(aux_info_type());
    fields.extend(vec![
        ("default_sample_info_size", T::u8()),
        ("sample_count", u32be()),
        ("sample_info_sizes", T::when(E::field("default_sample_info_size").equal_to(E::lit(0)), T::array(T::u8(), E::field("sample_count")))),
    ]);
    T::structure("AuxInfoSizes", fields)
}

fn aux_info_offsets() -> T {
    let mut fields = full_box_flags("AuxInfoFlags", &[(0, "type present")]);
    fields.extend(aux_info_type());
    fields.extend(vec![("entry_count", u32be()), ("offsets", T::array(by_version(u32be(), T::u64(Big)), E::field("entry_count")))]);
    // Each offset counts from the start of the file in a moov, and from the
    // fragment's base data offset in a traf.
    T::structure("AuxInfoOffsets", fields).positions(&["offsets"])
}

/// Whether this sample table has a box of type `kind`.
fn has(kind: &str) -> E {
    E::tagged("boxes", &["type"], cc(kind), &["type"]).equal_to(E::lit(cc(kind)))
}

/// The body of the box of type `kind`, read again as `ty` where it is.
fn view(kind: &str, ty: T) -> T {
    T::at_origin(E::start_of(E::tagged("boxes", &["type"], cc(kind), &["body", "version"])), ty)
}

/// `stbl`: the tables as boxes, then every sample placed where the tables say.
pub(super) fn sample_table() -> T {
    let complete = has("stsd")
        .both(has("stsc"))
        .both(has("stsz").either(has("stz2")))
        .both(has("stco").either(has("co64")));
    T::structure("SampleTable", vec![("boxes", child_boxes()), ("sample_data", T::when(complete, sample_data()))])
        .contents("boxes")
        .field_named_from("sample_data", E::words("track {}", E::sibling_tagged(&["type"], E::lit(cc("tkhd")), &["body", "times", "track_id"])))
        .field_doc("sample_data", "Every sample of the track, at the file offsets that stco, stsc and stsz give.")
}

/// The tables read again so that each sample can look itself up, then the
/// samples, a run of chunks at a time.
fn sample_data() -> T {
    let entry = |field: &[&str]| {
        let mut path = vec!["body", "entries", "0"];
        path.extend_from_slice(field);
        E::tagged("boxes", &["type"], cc("stsd"), &path)
    };
    let config = |kind: &str| {
        E::tagged_in(
            E::tagged("boxes", &["type"], cc("stsd"), &["body", "entries", "0", "body", "boxes"]),
            &["type"],
            cc(kind),
            &["body", "length_size_minus_one"],
        )
    };
    T::structure(
        "SampleData",
        vec![
            ("format", T::enumeration_hex("SampleFormat", T::computed(entry(&["format"])), media::FORMATS)),
            (
                "nal_length_size",
                T::when(
                    media::is_nal_format(E::field("format")),
                    T::computed(config("avcC").or(config("hvcC")).add(E::lit(1))),
                ),
            ),
            ("channel_count", T::when(media::is_pcm_format(E::field("format")), T::computed(entry(&["body", "channel_count"])))),
            (
                "sample_sizes",
                T::switch(has("stsz"), vec![(1, view("stsz", sample_sizes()))], view("stz2", compact_sample_sizes())),
            ),
            ("chunk_offsets", T::switch(has("stco"), vec![(1, view("stco", chunk_offsets(false)))], view("co64", chunk_offsets(true)))),
            ("sample_to_chunk", view("stsc", sample_to_chunk())),
            ("constant_size", T::computed(E::cond(has("stsz"), E::within(&["sample_sizes", "sample_size"]), E::lit(0)))),
            ("runs", T::array(chunk_run(), E::within(&["sample_to_chunk", "entry_count"]))),
        ],
    )
    .field_aside("sample_sizes")
    .field_aside("chunk_offsets")
    .field_aside("sample_to_chunk")
    .machinery(&["sample_sizes", "chunk_offsets", "sample_to_chunk", "constant_size", "nal_length_size", "channel_count"])
    .field_doc("format", "The sample entry format from stsd. Each sample is read according to this format.")
    .field_doc("nal_length_size", "The size in bytes of the length field before each NAL unit, from the decoder configuration.")
    .field_doc("constant_size", "The size of every sample in bytes, when stsz gives one size for all of them; otherwise 0.")
    .field_doc("sample_sizes", "The sample sizes from stsz (or stz2), used to find each sample.")
    .field_doc("chunk_offsets", "The chunk offsets from stco (or co64), used to find each chunk.")
    .field_doc("sample_to_chunk", "The entries of stsc, used to group the chunks into runs.")
    .field_doc("runs", "One run of chunks for each stsc entry. Every chunk in a run holds the same number of samples.")
}

/// One entry of `stsc` worked out: which chunks it covers, how many samples
/// each holds, and which sample its first chunk starts with.
///
/// The first sample of a run is the first sample of the run before plus every
/// sample that run held, which is the one thing no entry says and the reason
/// the runs are read in order.
fn chunk_run() -> T {
    let stsc = |index: E, field: &str| E::elem_within(&["sample_to_chunk", "entries"], index, &[field]);
    let next = E::idx().add(E::lit(1));
    let chunks = E::within(&["chunk_offsets", "entry_count"]);
    let ends_before = E::cond(
        next.clone().less_than(E::within(&["sample_to_chunk", "entry_count"])),
        stsc(next, "first_chunk"),
        chunks.clone().add(E::lit(1)),
    );
    let first_sample = E::cond(
        E::idx().equal_to(E::lit(0)),
        E::lit(0),
        E::prev("first_sample").add(E::prev("chunk_count").mul(E::prev("samples_per_chunk"))),
    );
    T::structure(
        "ChunkRun",
        vec![
            ("first_chunk", T::computed(stsc(E::idx(), "first_chunk"))),
            ("chunk_count", T::computed(ends_before.sub(E::field("first_chunk")).at_least(E::lit(0)).at_most(chunks))),
            ("samples_per_chunk", T::computed(stsc(E::idx(), "samples_per_chunk"))),
            ("first_sample", T::computed(first_sample)),
            ("chunks", T::array(chunk(), E::field("chunk_count"))),
        ],
    )
    .field_elem_named_from("chunks", E::words("chunk {}", E::field("first_chunk").add(E::idx())))
    .field_doc("first_chunk", "The first chunk of this run, counting from 1.")
    .field_doc("first_sample", "How many samples of the track come before this run.")
}

/// One chunk: placed at its offset from `stco`, and as many samples as the
/// run says, one after another.
fn chunk() -> T {
    let offset = E::elem_within(&["chunk_offsets", "entries"], E::field("first_chunk").sub(E::lit(1)).add(E::idx()), &[]);
    T::at(
        offset,
        T::structure(
            "Chunk",
            vec![
                ("first_sample", T::computed(E::field("first_sample").add(E::idx().mul(E::field("samples_per_chunk"))))),
                ("samples", T::array(sample(), E::field("samples_per_chunk"))),
            ],
        )
        .contents("samples")
        .machinery(&["first_sample"])
        .field_elem_named_from("samples", E::words("sample {}", E::field("first_sample").add(E::idx()).add(E::lit(1))))
        .field_doc("first_sample", "How many samples of the track come before this chunk."),
    )
}

/// One sample, as long as `stsz` says, read as what the track's format holds.
fn sample() -> T {
    let listed = E::elem_within(&["sample_sizes", "entry_sizes"], E::field("first_sample").add(E::idx()), &[]);
    let size = E::cond(E::field("constant_size").not_equal(E::lit(0)), E::field("constant_size"), listed);
    T::sized(size, T::Named("SampleContent".into()))
}

#[cfg(test)]
mod tests {
    use super::super::tests::{boxed, cat, full, value, walk};
    use super::super::mp4;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn u32s(values: &[u32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_be_bytes()).collect()
    }

    /// A movie with one track whose samples are split over two chunks, with a
    /// chunk of another kind of bytes between them.
    fn movie(sizes: &[u32], per_chunk: &[(u32, u32)], chunk_at: &[u32]) -> Vec<u8> {
        // Six reserved bytes and a data reference index, and nothing else.
        let entry = boxed(b"zzzz", &[0, 0, 0, 0, 0, 0, 0, 1]);
        let stsz_body: Vec<u32> = [0, sizes.len() as u32].into_iter().chain(sizes.iter().copied()).collect();
        track(&entry, &u32s(&stsz_body), per_chunk, chunk_at, false)
    }

    /// A moov with one track: this sample entry, these stsz fields, and the
    /// chunks at these offsets, in a co64 when `wide`.
    fn track(entry: &[u8], stsz: &[u8], per_chunk: &[(u32, u32)], chunk_at: &[u32], wide: bool) -> Vec<u8> {
        let stsd = full(b"stsd", 0, 0, &cat(&[&u32s(&[1]), entry]));
        let stsc_body: Vec<u32> = std::iter::once(per_chunk.len() as u32).chain(per_chunk.iter().flat_map(|(f, n)| [*f, *n, 1])).collect();
        let stsc = full(b"stsc", 0, 0, &u32s(&stsc_body));
        let stsz = full(b"stsz", 0, 0, stsz);
        let mut offsets = u32s(&[chunk_at.len() as u32]);
        for at in chunk_at {
            if wide {
                offsets.extend_from_slice(&(*at as u64).to_be_bytes());
            } else {
                offsets.extend_from_slice(&at.to_be_bytes());
            }
        }
        let stco = full(if wide { b"co64" } else { b"stco" }, 0, 0, &offsets);
        let stbl = boxed(b"stbl", &cat(&[&stsd, &stsc, &stsz, &stco]));
        boxed(b"moov", &boxed(b"trak", &boxed(b"mdia", &boxed(b"minf", &stbl))))
    }

    /// The path to `sample_data` in a file made by `track`, then `names`.
    fn data(names: &[&'static str]) -> Vec<&'static str> {
        ["0", "body", "0", "body", "0", "body", "0", "body", "0", "body", "sample_data"].iter().chain(names).copied().collect()
    }

    /// A file of a moov made by `make` for chunks at the given offsets into
    /// the mdat, then the mdat holding `media`.
    fn with_mdat(make: impl Fn(&[u32]) -> Vec<u8>, chunk_at: &[u32], media: &[u8]) -> Vec<u8> {
        let mdat_at = make(&vec![0; chunk_at.len()]).len() as u32 + 8;
        let moved: Vec<u32> = chunk_at.iter().map(|at| at + mdat_at).collect();
        cat(&[&make(&moved), &boxed(b"mdat", media)])
    }

    /// A visual sample entry of the given format with these boxes after it.
    fn visual_entry(format: &[u8; 4], boxes: &[u8]) -> Vec<u8> {
        let mut body = vec![0, 0, 0, 0, 0, 0, 0, 1];
        body.extend_from_slice(&[0; 16]);
        body.extend_from_slice(&[0, 64, 0, 48, 0, 72, 0, 0, 0, 72, 0, 0, 0, 0, 0, 0, 0, 1]);
        body.extend_from_slice(&[0; 32]);
        body.extend_from_slice(&[0, 24, 0xff, 0xff]);
        body.extend_from_slice(boxes);
        boxed(format, &body)
    }

    #[test]
    fn h264_samples_read_as_nal_units_with_their_sei_and_slice_header() {
        // avcC with four-byte lengths and no parameter sets.
        let avcc = boxed(b"avcC", &[1, 0x42, 0xc0, 0x1e, 0xff, 0xe0, 0]);
        let entry = visual_entry(b"avc1", &avcc);
        // An SEI of unregistered user data, then an IDR slice whose header is
        // first_mb_in_slice 0, slice_type 7 (I, all slices), pps 0.
        let mut sei = vec![0x06, 5, 18];
        sei.extend_from_slice(&[0x11; 16]);
        sei.extend_from_slice(b"hi");
        sei.push(0x80);
        let idr = [0x65u8, 0x88, 0x80, 0xaa, 0xbb];
        let mut sample = (sei.len() as u32).to_be_bytes().to_vec();
        sample.extend_from_slice(&sei);
        sample.extend_from_slice(&(idr.len() as u32).to_be_bytes());
        sample.extend_from_slice(&idr);
        let stsz = u32s(&[0, 1, sample.len() as u32]);
        let file = with_mdat(|at| track(&entry, &stsz, &[(1, 1)], at, false), &[0], &sample);
        let d = Document::new(MemSource(file));
        let mut ev = Evaluator::new(mp4());
        let first = data(&["runs", "0", "chunks", "0", "0", "samples", "0"]);
        let at = |names: &[&'static str]| -> Vec<&'static str> { first.iter().chain(names).copied().collect() };
        assert_eq!(value(&mut ev, &d, &at(&["0", "nal", "rbsp", "messages", "0", "payload", "text"])), Value::Str("hi".into()));
        let slice_type = value(&mut ev, &d, &at(&["1", "nal", "rbsp", "header", "slice_type", "value"]));
        assert_eq!(slice_type, Value::Enum { raw: 7, name: Some("I (all)".into()), hex: false });
        assert_eq!(value(&mut ev, &d, &at(&["1", "nal", "rbsp", "rest_bits"])), Value::UInt(0));
        let rest = walk(&mut ev, &d, &at(&["1", "nal", "rbsp", "rest"]));
        assert_eq!(ev.node(&d, &rest).unwrap().size_bits, 16);
    }

    #[test]
    fn pcm_samples_of_one_size_read_as_a_number_per_channel_from_a_co64() {
        // A big-endian PCM entry with two channels of 16 bits.
        let mut sound = vec![0, 0, 0, 0, 0, 0, 0, 1];
        sound.extend_from_slice(&[0; 8]);
        sound.extend_from_slice(&[0, 2, 0, 16, 0, 0, 0, 0, 0x1f, 0x40, 0, 0]);
        let entry = boxed(b"twos", &sound);
        let stsz = u32s(&[4, 2]);
        let media = [0x00u8, 0x01, 0xff, 0xfe, 0x7f, 0xff, 0x80, 0x00];
        let file = with_mdat(|at| track(&entry, &stsz, &[(1, 2)], at, true), &[0], &media);
        let d = Document::new(MemSource(file));
        let mut ev = Evaluator::new(mp4());
        let sample = |i: &'static str, ch: &'static str| data(&["runs", "0", "chunks", "0", "0", "samples", i, ch]);
        assert_eq!(value(&mut ev, &d, &sample("0", "1")), Value::Int(-2));
        assert_eq!(value(&mut ev, &d, &sample("1", "1")), Value::Int(-32768));
    }

    #[test]
    fn group_tables_dependency_bytes_and_compact_sizes_read_their_entries() {
        // sgpd version 1 with one default length, and one with a length per
        // entry for a grouping type nothing reads further.
        let mut roll = b"roll".to_vec();
        roll.extend_from_slice(&u32s(&[2, 1]));
        roll.extend_from_slice(&(-1i16).to_be_bytes());
        let mut other = b"abcd".to_vec();
        other.extend_from_slice(&u32s(&[0, 2, 3]));
        other.extend_from_slice(&[1, 2, 3]);
        other.extend_from_slice(&u32s(&[1]));
        other.push(9);
        let mut sbgp = b"roll".to_vec();
        sbgp.extend_from_slice(&u32s(&[1, 23, 1]));
        // sdtp: an independent sample, then one that depends on others.
        let sdtp = [0x20u8, 0x10];
        // stz2 with 4-bit sizes 1, 2 and 3.
        let mut stz2 = vec![0, 0, 0, 4];
        stz2.extend_from_slice(&u32s(&[3]));
        stz2.extend_from_slice(&[0x12, 0x30]);
        let file = cat(&[
            &full(b"sgpd", 1, 0, &roll),
            &full(b"sgpd", 1, 0, &other),
            &full(b"sbgp", 0, 0, &sbgp),
            &full(b"sdtp", 0, 0, &sdtp),
            &full(b"stz2", 0, 0, &stz2),
        ]);
        let d = Document::new(MemSource(file));
        let mut ev = Evaluator::new(mp4());
        assert_eq!(value(&mut ev, &d, &["0", "body", "entries", "0", "entry", "roll_distance"]), Value::Int(-1));
        let second = walk(&mut ev, &d, &["1", "body", "entries", "1", "entry"]);
        assert_eq!(ev.node(&d, &second).unwrap().size_bits, 8);
        assert_eq!(value(&mut ev, &d, &["2", "body", "entries", "0", "sample_count"]), Value::UInt(23));
        let independent = value(&mut ev, &d, &["3", "body", "samples", "0", "depends_on"]);
        assert_eq!(independent, Value::Enum { raw: 2, name: Some("independent".into()), hex: false });
        let dependent = value(&mut ev, &d, &["3", "body", "samples", "1", "depends_on"]);
        assert_eq!(dependent, Value::Enum { raw: 1, name: Some("depends on other samples".into()), hex: false });
        for (i, want) in [("0", 1u128), ("1", 2), ("2", 3)] {
            assert_eq!(value(&mut ev, &d, &["4", "body", "entry_sizes", i]), Value::UInt(want), "stz2 entry {i}");
        }
    }

    #[test]
    fn samples_are_placed_over_mdat_from_the_three_tables() {
        // Five samples: 3 in chunk 1, then 2 in chunk 2. The mdat holds chunk
        // 1, four bytes of something else, then chunk 2.
        let sizes = [2u32, 3, 1, 4, 2];
        let probe = movie(&sizes, &[(1, 3), (2, 2)], &[0, 0]);
        let mdat_at = probe.len() as u32 + 8;
        let file_moov = movie(&sizes, &[(1, 3), (2, 2)], &[mdat_at, mdat_at + 6 + 4]);
        let media: Vec<u8> = (1u8..=6).chain([0xee; 4]).chain(7u8..=12).collect();
        let file = cat(&[&file_moov, &boxed(b"mdat", &media)]);
        let d = Document::new(MemSource(file));
        let mut ev = Evaluator::new(mp4());

        let data = ["0", "body", "0", "body", "0", "body", "0", "body", "0", "body", "sample_data"];
        let at = |names: &[&'static str]| -> Vec<&'static str> { data.iter().chain(names).copied().collect() };
        assert_eq!(value(&mut ev, &d, &at(&["runs", "1", "first_sample"])), Value::Int(3));
        assert_eq!(value(&mut ev, &d, &at(&["runs", "1", "chunk_count"])), Value::Int(1));

        // Sample 4 (the first of chunk 2) is four bytes, after the gap.
        let fourth = walk(&mut ev, &d, &at(&["runs", "1", "chunks", "0", "0", "samples", "0"]));
        let node = ev.node(&d, &fourth).unwrap();
        assert_eq!(node.offset_bits / 8, (mdat_at + 10) as u64);
        assert_eq!(node.size_bits, 32);

        // A byte of mdat reads as the sample over it, and the bytes between the
        // chunks read as the mdat's own data.
        let p = ev.locate(&d, (mdat_at as u64 + 11) * 8).unwrap();
        assert!(p.starts_with(&fourth), "{p:?} is not in {fourth:?}");
        let gap = ev.locate(&d, (mdat_at as u64 + 7) * 8).unwrap();
        assert!(ev.node(&d, &gap).unwrap().name.starts_with("body"), "{:?}", ev.node(&d, &gap).unwrap().name);
    }

    /// Five thousand stsc entries, which is past where a list is kept whole in
    /// memory, and the first sample of each run is worked out from the run
    /// before. A fresh evaluator asked about the last byte of the file still
    /// has to find the last sample.
    #[test]
    fn a_long_sample_to_chunk_table_still_places_its_last_sample() {
        let runs = 5000u32;
        // Chunk n holds one sample when n is odd and two when it is even, so no
        // two entries in a row are alike and none can be merged.
        let per_chunk: Vec<(u32, u32)> = (1..=runs).map(|n| (n, 1 + (n + 1) % 2)).collect();
        let total: u32 = per_chunk.iter().map(|(_, n)| n).sum();
        let sizes = vec![1u32; total as usize];
        let probe = movie(&sizes, &per_chunk, &vec![0; runs as usize]);
        let mdat_at = probe.len() as u32 + 8;
        let mut starts = Vec::new();
        let mut at = mdat_at;
        for (_, n) in &per_chunk {
            starts.push(at);
            at += n;
        }
        let moov = movie(&sizes, &per_chunk, &starts);
        let media: Vec<u8> = (0..total).map(|i| i as u8).collect();
        let file = cat(&[&moov, &boxed(b"mdat", &media)]);
        let last_bit = (file.len() as u64 - 1) * 8;
        let d = Document::new(MemSource(file));
        let mut ev = Evaluator::new(mp4());
        // The cursor lands on the frame inside the sample, and the sample is
        // the structure holding it.
        let p = ev.locate(&d, last_bit).unwrap();
        let node = ev.node(&d, &p).unwrap();
        assert_eq!(node.offset_bits, last_bit);
        assert_eq!(node.size_bits, 8);
        let sample = ev.node(&d, &p[..p.len() - 1]).unwrap();
        assert!(sample.name.contains(&format!("sample {total}")), "{}", sample.name);
    }

    /// Each chunk offset of an `stco` or a `co64` is a position, so a view
    /// writes it in the base its addresses are in. The entry count is not.
    #[test]
    fn each_chunk_offset_is_a_position() {
        let entry = boxed(b"zzzz", &[0, 0, 0, 0, 0, 0, 0, 1]);
        for wide in [false, true] {
            let file = with_mdat(|at| track(&entry, &u32s(&[0, 2, 2, 2]), &[(1, 1)], at, wide), &[0, 2], &[1, 2, 3, 4]);
            let d = Document::new(MemSource(file));
            let mut ev = Evaluator::new(mp4());
            let stco = ["0", "body", "0", "body", "0", "body", "0", "body", "0", "body", "boxes", "3", "body"];
            let at = |names: &[&'static str]| -> Vec<&'static str> { stco.iter().chain(names).copied().collect() };
            let first = walk(&mut ev, &d, &at(&["entries", "0"]));
            assert!(ev.node(&d, &first).unwrap().locates, "wide {wide}");
            let count = walk(&mut ev, &d, &at(&["entry_count"]));
            assert!(!ev.node(&d, &count).unwrap().locates, "wide {wide}");
        }
        let t = mp4();
        crate::machinery::assert_marks(&t, "AuxInfoOffsets", &["offsets"], &["entry_count"]);
        crate::machinery::assert_marks(&t, "ItemLocation", &["base_offset"], &["extent_count"]);
        crate::machinery::assert_marks(&t, "ExifItem", &["tiff_header_offset"], &[]);
        crate::machinery::assert_marks(&t, "TrackFragmentHeader", &["base_data_offset"], &["default_sample_size"]);
        crate::machinery::assert_marks(&t, "RandomAccessEntry", &["moof_offset"], &["time"]);
        crate::machinery::assert_marks(&t, "SegmentIndex", &["first_offset"], &["reference_count"]);
    }
}
