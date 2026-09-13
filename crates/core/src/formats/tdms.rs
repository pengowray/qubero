//! NI TDMS: the file LabVIEW, DAQmx and the rest of National Instruments'
//! software write measurements into. Read from NI's "TDMS File Format
//! Internal Structure", checked against npTDMS.
//!
//! A file is a run of segments, each a 28-byte lead-in and then as much as the
//! lead-in says follows it. Nothing else holds the file together: no header,
//! no directory, no end record. A writer appends a segment every time it
//! flushes, so a long recording is thousands of them.
//!
//! **The lead-in.** Four letters, `TDSm` in a data file and `TDSh` in the
//! `.tdms_index` file written beside one; a mask of flags NI calls the table of
//! contents; a version; and two offsets, both counted from the end of the
//! lead-in: where the next segment starts and where this one's raw data does.
//! The mask is always little-endian, and one of its bits says whether
//! everything after it is. NI's document says so in as many words, "including
//! the lead in", and the big-endian file npTDMS keeps for its tests is written
//! that way: its mask reads `4e 00 00 00` and its version `00 00 12 69`. So the
//! mask is read first and the rest of the segment is a switch with one arm per
//! byte order, the way a GWF frame file's check word picks its arm.
//!
//! A next-segment offset of all ones is how a segment the writer never came
//! back to finish is marked. Clamping the offset to what is left of the file
//! reads that case with no special arm: the segment runs to the end.
//!
//! An index file is a copy of the data file with the raw data left out, so its
//! segments keep the data file's offsets and each one ends where its metadata
//! does.

use std::sync::Arc;

use crate::template::{Anchor, Endian, Endian::*, Expr as E, Step, Tag, TaggedRef, Template, Time, Ty as T, Until};

/// The bits of the table of contents that mean something, by NI's names less
/// the `kToc`. Bit 0 and bit 4 are unused.
const TOC: &[(u32, &str)] = &[
    (1, "metadata"),
    (2, "new object list"),
    (3, "raw data"),
    (5, "interleaved"),
    (6, "big-endian"),
    (7, "DAQmx raw data"),
];

/// The two versions NI has written. 4713 is TDMS 2.0, which added interleaved
/// data, the big-endian flag and DAQmx raw data.
const VERSION: &[(i128, &str)] = &[(4712, "TDMS 1.0"), (4713, "TDMS 2.0")];

/// A next-segment offset of every bit set.
const UNFINISHED: i128 = 0xFFFF_FFFF_FFFF_FFFF;

/// `TDSm` and `TDSh`, the four letters read as one big-endian number.
const DATA_TAG: i128 = 0x5444_536D;
const INDEX_TAG: i128 = 0x5444_5368;

pub fn tdms() -> Template {
    Template::new("tdms", T::repeat(segment(), Until::End))
}

/// One segment: the tag, the table of contents, and everything the table says
/// about the byte order of the rest.
fn segment() -> T {
    T::structure_named(
        "Segment",
        "",
        "contents",
        vec![
            // Four letters, read as the number they spell so that the rest of
            // the segment can ask which of the two it is. Anything else is
            // not a segment, and reads as a number nothing named.
            ("tag", T::enumeration("SegmentTag", T::u32(Big), &[(DATA_TAG, "TDSm"), (INDEX_TAG, "TDSh")])),
            ("toc", T::flags("TableOfContents", T::u32(Little), TOC)),
            (
                "contents",
                T::switch(E::field("toc").bit(6), vec![(1, contents(Big))], contents(Little)),
            ),
        ],
    )
}

/// The rest of a segment, every number in it read the way `e` says.
fn contents(e: Endian) -> T {
    let offset = E::field("raw_data_offset");
    // How much of the file this segment's raw data takes. An index file holds
    // none of it, though its offsets still count it. What the next-segment
    // offset claims past the metadata, never below nothing and never past the
    // end of the file: a writer that stopped wrote all ones there, and the
    // clamp is what reads its segment to the end of the file.
    let raw_size = || E::field("next_segment_offset").sub(offset.clone()).at_least(E::lit(0)).at_most(E::Remaining);
    let is_data = || E::field("tag").not_equal(E::lit(INDEX_TAG));
    T::structure(
        "SegmentContents",
        vec![
            ("version", T::enumeration("TdmsVersion", T::u32(e), VERSION)),
            (
                "next_segment_offset",
                T::enumeration("NextSegmentOffset", T::u64(e), &[(UNFINISHED, "unfinished")]),
            ),
            ("raw_data_offset", T::u64(e)),
            // As long as the raw data offset says whether or not the flag is
            // set, so that the raw data starts where the offset puts it. A
            // segment without the flag writes an offset of nothing.
            (
                "metadata",
                T::sized(
                    offset.clone().at_least(E::lit(0)).at_most(E::Remaining),
                    T::when(E::field("toc").bit(1), metadata(e)),
                ),
            ),
            ("raw_data_layout", T::enumeration("RawDataLayout", T::computed(raw_data_layout()), LAYOUT)),
            (
                "raw_data",
                T::when(
                    is_data().both(E::lit(0).less_than(raw_size())),
                    T::sized(
                        raw_size(),
                        T::switch(
                            E::field("raw_data_layout"),
                            vec![
                                (CONTIGUOUS, contiguous(e, Here)),
                                (INTERLEAVED, interleaved(e, Here)),
                                (CONTIGUOUS_EARLIER, contiguous(e, Earlier)),
                                (INTERLEAVED_EARLIER, interleaved(e, Earlier)),
                            ],
                            T::bytes(E::Remaining),
                        ),
                    ),
                ),
            ),
        ],
    )
    .machinery(&["next_segment_offset", "raw_data_offset", "raw_data_layout"])
}

/// How a segment's raw data is laid out, and what says so.
const UNKNOWN: i128 = 0;
const CONTIGUOUS: i128 = 1;
const INTERLEAVED: i128 = 2;
const CONTIGUOUS_EARLIER: i128 = 3;
const INTERLEAVED_EARLIER: i128 = 4;
const DAQMX: i128 = 5;

const LAYOUT: &[(i128, &str)] = &[
    (UNKNOWN, "not known"),
    (CONTIGUOUS, "contiguous"),
    (INTERLEAVED, "interleaved"),
    (CONTIGUOUS_EARLIER, "contiguous, laid out by an earlier segment"),
    (INTERLEAVED_EARLIER, "interleaved, laid out by an earlier segment"),
    (DAQMX, "DAQmx"),
];

/// Where the channel list a segment's raw data is laid out by comes from.
#[derive(Clone, Copy)]
enum LaidOut {
    /// This segment's own metadata.
    Here,
    /// The metadata of the nearest segment before this one that has any,
    /// which is what a segment without the metadata flag reuses whole.
    Earlier,
}

use LaidOut::{Earlier, Here};

/// A field of the metadata the raw data is laid out by.
fn laid_out_by(from: LaidOut, field: &str) -> E {
    match from {
        Here => E::within(&["metadata", field]),
        Earlier => E::sibling(&["contents", "metadata", field]),
    }
}

/// Which of the layouts this segment's raw data has, or that none of them can
/// be placed.
///
/// DAQmx data is left as bytes: its values are interleaved by acquisition card
/// and cut out by scaler, which the index describes and the raw data does not
/// repeat.
///
/// The data is placed when the channel list is one the template holds whole.
/// That is a segment with metadata that starts a new list, or a segment with
/// no metadata after one of those, which reuses it. A segment with metadata
/// that adds to the list before it is not placed: the list it adds to is the
/// sum of every segment since the last new one, and a field can hold a number
/// or some text but not a list carried from one segment to the next.
///
/// Interleaved data needs every channel to have a width, since a sample is one
/// value of each. npTDMS reads a segment flagged interleaved that holds a
/// single string channel as contiguous, and so does this.
fn raw_data_layout() -> E {
    let toc = || E::field("toc");
    let kind = |from: LaidOut, contiguous: i128, interleaved: i128| {
        let flagged = toc().bit(5);
        E::cond(
            flagged.clone().both(laid_out_by(from, "no_width_count").equal_to(E::lit(0))),
            E::lit(interleaved),
            E::cond(
                flagged
                    .negate()
                    .either(laid_out_by(from, "channel_count").equal_to(E::lit(1)))
                    .both(laid_out_by(from, "unknown_layout_count").equal_to(E::lit(0))),
                E::lit(contiguous),
                E::lit(UNKNOWN),
            ),
        )
    };
    E::cond(
        toc().bit(7),
        E::lit(DAQMX),
        E::cond(
            toc().bit(1),
            E::cond(laid_out_by(Here, "new_list"), kind(Here, CONTIGUOUS, INTERLEAVED), E::lit(UNKNOWN)),
            E::cond(
                laid_out_by(Earlier, "new_list"),
                kind(Earlier, CONTIGUOUS_EARLIER, INTERLEAVED_EARLIER),
                E::lit(UNKNOWN),
            ),
        ),
    )
}

/// A field of one channel of the list the raw data is laid out by, for the
/// channel at the index this is asked from.
fn of_channel(from: LaidOut, field: &str) -> E {
    match from {
        Here => E::elem_within(&["metadata", "channels"], E::idx(), &[field]),
        Earlier => E::elem_field("channels", E::idx(), &[field]),
    }
}

/// The channels a segment with no metadata reads its data by, copied out of
/// the earlier segment that has them. Asking the earlier segment once per
/// channel here is what saves asking it once per chunk below.
fn copied_channels(from: LaidOut) -> Option<(&'static str, T)> {
    let Earlier = from else { return None };
    let found = |field: &str| {
        E::Tagged(Arc::new(TaggedRef {
            array: Some(E::sibling(&["contents", "metadata", "channels"])),
            key: path(&["number"]),
            tag: Tag::Computed(Arc::new(E::idx())),
            field: path(&[field]),
        }))
    };
    let copy = T::structure_named(
        "Channel",
        "path",
        "",
        vec![
            ("path", T::computed_text(found("path"))),
            ("data_type", T::enumeration_hex("DataType", T::computed(found("data_type")), DATA_TYPE)),
            ("value_count", T::computed(found("value_count"))),
            ("data_size", T::computed(found("data_size"))),
        ],
    );
    Some(("channels", T::array(copy, E::field("channel_count"))))
}

/// Contiguous raw data: chunk after chunk, each holding every channel's values
/// one channel after another. The chunk repeats as many times as fit, and a
/// writer that stopped partway through one leaves the rest as bytes.
fn contiguous(e: Endian, from: LaidOut) -> T {
    let chunk = T::structure_named(
        "Chunk",
        "",
        "values",
        vec![("values", T::array(channel_values(e, from), E::field("channel_count")))],
    )
    .field_elem_named_from("values", of_channel(from, "path"));
    let mut fields = vec![
        ("channel_count", T::computed(laid_out_by(from, "channel_count"))),
        ("chunk_size", T::computed(laid_out_by(from, "chunk_size"))),
        ("chunk_count", T::computed(whole(E::field("chunk_size")))),
    ];
    fields.extend(copied_channels(from));
    fields.extend([
        ("chunks", T::array(T::sized(E::field("chunk_size"), chunk), E::field("chunk_count"))),
        ("unfinished_chunk", T::when(E::lit(0).less_than(E::Remaining), T::bytes(E::Remaining))),
    ]);
    T::structure("ContiguousData", fields).machinery(&["channel_count", "chunk_size", "chunk_count", "channels"])
}

/// How many whole runs of `size` bytes fit in what is left, and none of a run
/// of nothing.
fn whole(size: E) -> E {
    E::cond(E::lit(0).less_than(size.clone()), E::Remaining.div(size), E::lit(0))
}

/// Interleaved raw data: sample after sample, each one value of every
/// channel. How many samples a channel's index says a chunk holds does not
/// matter to the layout, since chunk after chunk of samples is only more
/// samples.
fn interleaved(e: Endian, from: LaidOut) -> T {
    let sample = T::structure_named(
        "Sample",
        "",
        "values",
        vec![("values", T::array(T::switch(of_channel(from, "data_type"), scalars(e), T::bytes(E::lit(0))), E::field("channel_count")))],
    )
    .field_elem_named_from("values", of_channel(from, "path"));
    let mut fields = vec![
        ("channel_count", T::computed(laid_out_by(from, "channel_count"))),
        ("sample_size", T::computed(laid_out_by(from, "frame_size"))),
        ("sample_count", T::computed(whole(E::field("sample_size")))),
    ];
    fields.extend(copied_channels(from));
    fields.extend([
        ("samples", T::array(T::sized(E::field("sample_size"), sample), E::field("sample_count"))),
        ("unfinished_sample", T::when(E::lit(0).less_than(E::Remaining), T::bytes(E::Remaining))),
    ]);
    T::structure("InterleavedData", fields).machinery(&["channel_count", "sample_size", "sample_count", "channels"])
}

/// One channel's values in one chunk, as its type says.
fn channel_values(e: Endian, from: LaidOut) -> T {
    let count = || of_channel(from, "value_count");
    let mut cases: Vec<(i128, T)> = scalars(e).into_iter().map(|(k, t)| (k, T::array(t, count()))).collect();
    cases.push((STRING, strings(e, count(), of_channel(from, "data_size"))));
    T::switch(of_channel(from, "data_type"), cases, T::bytes(of_channel(from, "data_size").at_least(E::lit(0))))
}

/// A string channel's values: where each string ends, and then all of them
/// written together. NI's document calls the numbers offsets and says they
/// are where each string starts; every writer puts where each one ends, the
/// first string starting at nothing, and npTDMS reads them that way.
fn strings(e: Endian, count: E, size: E) -> T {
    let end = E::elem("offsets", E::idx());
    let start = E::cond(E::idx().equal_to(E::lit(0)), E::lit(0), E::elem("offsets", E::idx().sub(E::lit(1))));
    T::structure(
        "Strings",
        vec![
            ("offsets", T::array(T::u32(e), count.clone())),
            (
                "text",
                T::sized(
                    size.sub(E::size_of("offsets")).at_least(E::lit(0)).at_most(E::Remaining),
                    T::array(T::utf8(end.sub(start)), count),
                ),
            ),
        ],
    )
}

/// A length and that many bytes of UTF-8. Every name, path and text value in
/// the file is written this way, with nothing to end it.
fn string(e: Endian) -> T {
    T::inline_structure("TdmsString", vec![("length", T::u32(e)), ("text", T::utf8(E::field("length")))])
}

/// The metadata: how many objects the segment describes, and each of them.
///
/// Nothing says the objects fill the space the raw data offset leaves them.
/// The first segment of npTDMS's `raw1.tdms`, which DAQmx wrote, ends its
/// objects 2,341 bytes into a metadata section of 4,068, and what is after
/// them reads as the padding it is.
///
/// The fields after that are not in the file. `channels` is the objects that
/// have raw data in this segment, in the order their data is written, which
/// is what the raw data is a run of; the counts and sizes below it are what
/// that list adds up to. They are here rather than beside the raw data so that
/// a later segment with no metadata of its own can ask for them.
fn metadata(e: Endian) -> T {
    // One number per channel, for the sums below. There is no way to add up a
    // field of a list of records, so each channel's share is copied out first,
    // the way NetCDF's `record_bytes` is.
    let per_channel = |value: E| T::array(T::computed(value), E::field("channel_count"));
    let of_channel = |field: &str| E::elem_field("channels", E::idx(), &[field]);
    T::structure(
        "Metadata",
        vec![
            ("object_count", T::u32(e)),
            ("objects", T::array(object(e), E::field("object_count"))),
            ("padding", T::when(E::lit(0).less_than(E::Remaining), T::bytes(E::Remaining))),
            // Every object with raw data, and none without. An object with
            // none gives an offset before the list's own start, which is what
            // passes a record over; the channels take no bytes, so the offset
            // of every other one is nothing.
            (
                "channels",
                T::gather(
                    vec![Step::field("objects"), Step::each()],
                    E::cond(E::field("has_data"), E::lit(0), E::lit(-1)),
                    Anchor::Window,
                    E::lit(0),
                    channel(),
                ),
            ),
            ("channel_count", T::computed(E::len_of("channels"))),
            ("data_sizes", per_channel(of_channel("data_size"))),
            ("chunk_size", T::computed(E::sum_of("data_sizes"))),
            ("widths", per_channel(of_channel("width"))),
            ("frame_size", T::computed(E::sum_of("widths"))),
            ("unknown_layouts", per_channel(E::lit(1).sub(of_channel("layout_known")))),
            ("unknown_layout_count", T::computed(E::sum_of("unknown_layouts"))),
            ("no_widths", per_channel(of_channel("width").equal_to(E::lit(0)))),
            ("no_width_count", T::computed(E::sum_of("no_widths"))),
            // Whether this list is all there is: the flag says a new list
            // starts here, and the first segment has nothing to add to.
            // Without it, the objects are added to the list the segment
            // before ended with, which is not something a field can hold.
            ("new_list", T::computed(E::field("toc").bit(2).either(E::idx().equal_to(E::lit(0))))),
        ],
    )
    .machinery(&[
        "channel_count",
        "data_sizes",
        "chunk_size",
        "widths",
        "frame_size",
        "unknown_layouts",
        "unknown_layout_count",
        "no_widths",
        "no_width_count",
        "new_list",
    ])
}

/// One channel with raw data in this segment: the object's layout, read again
/// from the object that placed it. `number` is where it is in the list, which
/// is what a later segment finds it by.
fn channel() -> T {
    let placer = |field: &str| E::placer(E::field(field));
    T::structure_named(
        "Channel",
        "path",
        "",
        vec![
            ("path", T::computed_text(E::placer(E::within(&["path", "text"])))),
            ("data_type", T::enumeration_hex("DataType", T::computed(placer("data_type")), DATA_TYPE)),
            ("value_count", T::computed(placer("value_count"))),
            ("width", T::computed(placer("width"))),
            ("data_size", T::computed(placer("data_size"))),
            ("layout_known", T::computed(placer("layout_known"))),
            ("number", T::computed(E::idx())),
        ],
    )
}

/// What an object's raw data index can say instead of how long it is.
const NO_DATA: i128 = 0xFFFF_FFFF;
const SAME_AS_BEFORE: i128 = 0;
/// A DAQmx channel's index, one per kind of scaler it holds. NI's document
/// spells them `0x69120000` and `0x69130000`, which is the bytes a
/// little-endian file writes read the other way round. npTDMS finds the
/// digital line scaler written as `0x0000126A` in real files, and says NI's
/// `0x1369` is wrong; both are read here as the digital line scaler they mean.
const FORMAT_CHANGING: i128 = 0x1269;
const DIGITAL_LINE: i128 = 0x126A;
const DIGITAL_LINE_AS_DOCUMENTED: i128 = 0x1369;

const RAW_DATA_INDEX: &[(i128, &str)] = &[
    (NO_DATA, "no raw data"),
    (SAME_AS_BEFORE, "same as before"),
    (FORMAT_CHANGING, "DAQmx format changing scaler"),
    (DIGITAL_LINE, "DAQmx digital line scaler"),
    (DIGITAL_LINE_AS_DOCUMENTED, "DAQmx digital line scaler"),
];

/// One object: a root, a group or a channel, by its path. `/` is the file,
/// `/'group'` a group and `/'group'/'channel'` a channel in it, with a quote
/// inside a name written twice.
///
/// The raw data index is a number that is either how many bytes of index
/// follow it or one of four values that mean something else. The index is
/// read by its type and not by that length: npTDMS's writer puts 20 in front
/// of a string channel's index and then writes the 28 bytes a string's index
/// takes, and a reader that trusted the 20 would read the properties eight
/// bytes early. npTDMS reads it by type too.
///
/// After the fields the file writes come six it does not: the layout this
/// object's raw data has in this segment, worked out once here so that the raw
/// data and later segments can ask for it by name. See [`layout_of`].
fn object(e: Endian) -> T {
    let index = E::field("raw_data_index");
    let has_index = || index.clone().not_equal(E::lit(NO_DATA)).both(index.clone().not_equal(E::lit(SAME_AS_BEFORE)));
    let is_daqmx = || {
        index.clone().equal_to(E::lit(FORMAT_CHANGING))
            .either(index.clone().equal_to(E::lit(DIGITAL_LINE)))
            .either(index.clone().equal_to(E::lit(DIGITAL_LINE_AS_DOCUMENTED)))
    };
    // Written here, or carried from the last segment that wrote it.
    let own_or_before = |own: E, field: &str| E::cond(has_index(), own, layout_of(field));
    let kind = E::field("data_type");
    let count = E::field("value_count");
    let width = E::field("width");
    // A string channel says how many bytes it takes; every other type is its
    // width times its count. DAQmx data is laid out by its scalers and not by
    // anything a template can add up.
    let own_size = E::cond(
        is_daqmx(),
        E::lit(0),
        E::cond(kind.clone().equal_to(E::lit(STRING)), E::within(&["index", "total_size"]), width.clone().mul(count.clone())),
    );
    let own_known = is_daqmx()
        .negate()
        .both(kind.equal_to(E::lit(STRING)).either(E::lit(0).less_than(width)).either(count.equal_to(E::lit(0))));
    let widths = WIDTH.iter().map(|(k, w)| (*k, T::computed(E::lit(*w)))).collect();
    T::structure_named(
        "Object",
        "path",
        "",
        vec![
            ("path", string(e)),
            (
                "raw_data_index",
                T::enum_ranged("RawDataIndex", T::u32(e), RAW_DATA_INDEX, &[(0, 1, "{n}-byte index")]),
            ),
            (
                "index",
                T::when(
                    has_index(),
                    T::switch(
                        index.clone(),
                        vec![
                            (FORMAT_CHANGING, daqmx_index(e, false)),
                            (DIGITAL_LINE, daqmx_index(e, true)),
                            (DIGITAL_LINE_AS_DOCUMENTED, daqmx_index(e, true)),
                        ],
                        raw_data_index(e),
                    ),
                ),
            ),
            ("property_count", T::u32(e)),
            ("properties", T::array(property(e), E::field("property_count"))),
            ("has_data", T::computed(index.clone().not_equal(E::lit(NO_DATA)))),
            (
                "data_type",
                T::enumeration_hex("DataType", T::computed(own_or_before(E::within(&["index", "data_type"]), "data_type")), DATA_TYPE),
            ),
            ("value_count", T::computed(own_or_before(E::within(&["index", "value_count"]), "value_count"))),
            ("width", T::switch(E::field("data_type"), widths, T::computed(E::lit(0)))),
            ("data_size", T::computed(own_or_before(own_size, "data_size"))),
            ("layout_known", T::computed(own_or_before(own_known, "layout_known"))),
        ],
    )
    .machinery(&["has_data", "data_type", "value_count", "width", "data_size", "layout_known"])
}

/// `field` of this object as the most recent earlier segment with metadata
/// has it, found by path. Nothing when that segment does not list the path.
///
/// What a raw data index of 0 means, and what one of all ones leaves standing:
/// the layout is whatever it was last time. npTDMS keeps the last layout of
/// every path it has seen, from however far back. A template cannot keep a
/// table, so this asks the one place it can reach, the object list of the
/// nearest segment behind this one that has metadata, and asks that object's
/// own `field` in turn, which asks the segment before that if it too said
/// "same as before". LabVIEW lists every channel again in each segment it
/// writes with metadata, so the chain is unbroken in the files it writes. A
/// path the nearest such segment left out is where the chain stops, and the
/// layout is not known.
///
/// Each link is a question asked inside the one before, so a channel that has
/// said "same as before" for thousands of segments running is thousands of
/// questions deep when its last segment is read first. Reading the segments in
/// order answers each from the one before it; reading the last one cold may
/// run out of room and say so.
fn layout_of(field: &str) -> E {
    E::Tagged(Arc::new(TaggedRef {
        array: Some(E::sibling(&["contents", "metadata", "objects"])),
        key: path(&["path", "text"]),
        tag: Tag::ComputedText(Arc::new(E::within(&["path", "text"]))),
        field: path(&[field]),
    }))
}

fn path(names: &[&str]) -> Arc<[String]> {
    names.iter().map(|s| s.to_string()).collect()
}

/// How an ordinary channel's values are laid out in this segment's raw data:
/// their type, how many of them there are in one chunk, and for strings how
/// many bytes that comes to. The dimension is always 1 in TDMS 2.0.
fn raw_data_index(e: Endian) -> T {
    T::structure(
        "RawDataIndex",
        vec![
            ("data_type", data_type(e)),
            ("dimension", T::u32(e)),
            ("value_count", T::u64(e)),
            ("total_size", T::when(E::field("data_type").equal_to(E::lit(STRING)), T::u64(e))),
        ],
    )
}

/// A DAQmx channel's index. The values are not the channel's own: they are
/// raw readings from the acquisition card, one buffer per card and several
/// channels interleaved in each, and a scaler says where in a buffer's sample
/// this channel's reading is and what type it is. `value_count` is how many
/// samples one chunk of the buffer holds, and `widths` how wide one sample of
/// each buffer is.
///
/// A digital line scaler says where its reading is in bits rather than bytes,
/// and its sample format is one byte where the other scaler's is four, which
/// is why the two records are 17 and 20 bytes long.
fn daqmx_index(e: Endian, digital_line: bool) -> T {
    let scaler = match digital_line {
        false => T::structure(
            "FormatChangingScaler",
            vec![
                ("daqmx_data_type", T::enumeration("DaqmxDataType", T::u32(e), DAQMX_TYPE)),
                ("raw_buffer_index", T::u32(e)),
                ("raw_byte_offset", T::u32(e)),
                ("sample_format_bitmap", T::u32(e)),
                ("scale_id", T::u32(e)),
            ],
        ),
        true => T::structure(
            "DigitalLineScaler",
            vec![
                ("daqmx_data_type", T::enumeration("DaqmxDataType", T::u32(e), DAQMX_TYPE)),
                ("raw_buffer_index", T::u32(e)),
                ("raw_bit_offset", T::u32(e)),
                ("sample_format_bitmap", T::u8()),
                ("scale_id", T::u32(e)),
            ],
        ),
    };
    T::structure(
        "DaqmxIndex",
        vec![
            ("data_type", data_type(e)),
            ("dimension", T::u32(e)),
            ("value_count", T::u64(e)),
            ("scaler_count", T::u32(e)),
            ("scalers", T::array(scaler, E::field("scaler_count"))),
            ("width_count", T::u32(e)),
            ("widths", T::array(T::u32(e), E::field("width_count"))),
        ],
    )
}

/// A DAQmx scaler's type codes, which are not the numbers of `tdsDataType`.
/// From npTDMS's `DAQMX_TYPES`; NI's document does not list them.
const DAQMX_TYPE: &[(i128, &str)] = &[
    (0, "U8"),
    (1, "I8"),
    (2, "U16"),
    (3, "I16"),
    (4, "U32"),
    (5, "I32"),
    (6, "U64"),
    (7, "I64"),
    (8, "SingleFloat"),
    (9, "DoubleFloat"),
    (0xFFFF_FFFF, "TimeStamp"),
];

const STRING: i128 = 0x20;
const TIMESTAMP: i128 = 0x44;

/// `tdsDataType`, by NI's names less the `tdsType`.
const DATA_TYPE: &[(i128, &str)] = &[
    (0, "Void"),
    (1, "I8"),
    (2, "I16"),
    (3, "I32"),
    (4, "I64"),
    (5, "U8"),
    (6, "U16"),
    (7, "U32"),
    (8, "U64"),
    (9, "SingleFloat"),
    (10, "DoubleFloat"),
    (11, "ExtendedFloat"),
    (0x19, "SingleFloatWithUnit"),
    (0x1A, "DoubleFloatWithUnit"),
    (0x1B, "ExtendedFloatWithUnit"),
    (STRING, "String"),
    (0x21, "Boolean"),
    (TIMESTAMP, "TimeStamp"),
    (0x4F, "FixedPoint"),
    (0x08_000C, "ComplexSingleFloat"),
    (0x10_000D, "ComplexDoubleFloat"),
    (0xFFFF_FFFF, "DAQmxRawData"),
];

/// In hex, because the two complex types and the DAQmx one are numbers
/// nobody reads in decimal.
fn data_type(e: Endian) -> T {
    T::enumeration_hex("DataType", T::u32(e), DATA_TYPE)
}

/// One value of each type that has a width, the way a property holds one and
/// a channel holds a run of them. A float "with unit" is the same float, its
/// unit written in a property beside it.
///
/// Two types have no width anyone has written down. `ExtendedFloat` is
/// LabVIEW's extended precision, which is eighty bits on one platform and a
/// hundred and twenty-eight on another, and `FixedPoint` is a LabVIEW
/// fixed-point number whose word length is part of its type and not of the
/// file. npTDMS reads neither, and no sample here has one.
fn scalars(e: Endian) -> Vec<(i128, T)> {
    let complex = |name: &str, part: T| {
        T::inline_structure(name, vec![("real", part.clone()), ("imaginary", part)])
    };
    vec![
        (1, T::Int { bits: 8, endian: e }),
        (2, T::Int { bits: 16, endian: e }),
        (3, T::i32(e)),
        (4, T::Int { bits: 64, endian: e }),
        (5, T::u8()),
        (6, T::u16(e)),
        (7, T::u32(e)),
        (8, T::u64(e)),
        (9, T::F32(e)),
        (10, T::F64(e)),
        (0x19, T::F32(e)),
        (0x1A, T::F64(e)),
        (0x21, T::enumeration("Boolean", T::u8(), &[(0, "false"), (1, "true")])),
        (TIMESTAMP, timestamp(e)),
        (0x08_000C, complex("ComplexSingleFloat", T::F32(e))),
        (0x10_000D, complex("ComplexDoubleFloat", T::F64(e))),
    ]
}

/// Bytes per value for each type in [`scalars`], and nothing for the rest.
const WIDTH: &[(i128, i128)] = &[
    (1, 1),
    (2, 2),
    (3, 4),
    (4, 8),
    (5, 1),
    (6, 2),
    (7, 4),
    (8, 8),
    (9, 4),
    (10, 8),
    (0x19, 4),
    (0x1A, 8),
    (0x21, 1),
    (TIMESTAMP, 16),
    (0x08_000C, 8),
    (0x10_000D, 16),
];

/// A moment: whole seconds since 1904-01-01 UTC, and a fraction of a second
/// counted in 2^-64ths.
///
/// The two are one 128-bit number, so which comes first follows the byte
/// order: the fraction in a little-endian file and the seconds in a big-endian
/// one. npTDMS reads them that way round, and so does the big-endian sample.
///
/// The seconds are the moment. The fraction is kept as the number it is, with
/// what it comes to in nanoseconds beside it, since 9223372036854775808 is
/// half a second and nobody reads it as one.
fn timestamp(e: Endian) -> T {
    let seconds = ("seconds", T::Int { bits: 64, endian: e });
    let fraction = ("fraction", T::u64(e));
    let fields = match e {
        Little => vec![fraction, seconds],
        Big => vec![seconds, fraction],
    };
    let mut fields = fields;
    fields.push((
        "nanoseconds",
        T::computed(E::field("fraction").mul(E::lit(1_000_000_000)).div(E::lit(1i128 << 64))),
    ));
    T::structure("TimeStamp", fields).field_time("seconds", Time::mac()).machinery(&["fraction"])
}

/// A property: a name, the type of its value, and the value.
///
/// A value of a type with no known width ends what can be read of the object:
/// the rest of the metadata reads as bytes rather than as properties placed
/// at a guess.
fn property(e: Endian) -> T {
    let mut cases = scalars(e);
    cases.push((STRING, string(e)));
    T::structure_named(
        "Property",
        "name",
        "value",
        vec![
            ("name", string(e)),
            ("data_type", data_type(e)),
            ("value", T::switch(E::field("data_type"), cases, T::bytes(E::Remaining))),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// A lead-in and what follows it, `big` deciding every number after the
    /// mask.
    fn segment(toc: u32, big: bool, meta: &[u8], raw: &[u8], next: Option<u64>) -> Vec<u8> {
        let mut b = b"TDSm".to_vec();
        b.extend_from_slice(&(toc | if big { 1 << 6 } else { 0 }).to_le_bytes());
        let w = |v: u64, n: usize| -> Vec<u8> {
            let bytes = if big { v.to_be_bytes() } else { v.to_le_bytes() };
            if big { bytes[8 - n..].to_vec() } else { bytes[..n].to_vec() }
        };
        b.extend(w(4713, 4));
        b.extend(w(next.unwrap_or((meta.len() + raw.len()) as u64), 8));
        b.extend(w(meta.len() as u64, 8));
        b.extend_from_slice(meta);
        b.extend_from_slice(raw);
        b
    }

    #[test]
    fn a_segment_is_as_long_as_its_lead_in_says() {
        let mut file = segment(0b1110, false, &[0; 12], &[7; 16], None);
        file.extend(segment(0b1000, true, &[], &[9; 8], None));
        let d = Document::new(MemSource(file.clone()));
        let mut ev = Evaluator::new(tdms());
        assert_eq!(ev.node(&d, &[]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[0, 2, 0]).unwrap().value, Value::Enum { raw: 4713, name: Some("TDMS 2.0".into()), hex: false });
        assert_eq!(ev.node(&d, &[0, 2, 3]).unwrap().size_bits, 12 * 8);
        assert_eq!(ev.node(&d, &[0, 2, 5]).unwrap().size_bits, 16 * 8);
        // The second is big-endian after its mask, and reads the same.
        assert_eq!(ev.node(&d, &[1, 2, 1]).unwrap().value.as_int(), Some(8));
        let last = ev.node(&d, &[1]).unwrap();
        assert_eq!(last.offset_bits + last.size_bits, file.len() as u64 * 8);
    }

    #[test]
    fn a_segment_the_writer_never_finished_runs_to_the_end_of_the_file() {
        let file = segment(0b1000, false, &[], &[1; 40], Some(u64::MAX));
        let d = Document::new(MemSource(file.clone()));
        let mut ev = Evaluator::new(tdms());
        assert_eq!(
            ev.node(&d, &[0, 2, 1]).unwrap().value,
            Value::Enum { raw: UNFINISHED, name: Some("unfinished".into()), hex: false }
        );
        assert_eq!(ev.node(&d, &[0, 2, 5]).unwrap().size_bits, 40 * 8);
    }

    #[test]
    fn an_index_file_segment_ends_with_its_metadata() {
        // The offsets are the data file's: eight bytes of metadata and a
        // hundred of raw data that the index file does not hold.
        let mut file = segment(0b1110, false, &[0; 8], &[], Some(108));
        file.extend(segment(0b1110, false, &[0; 8], &[], Some(108)));
        file[..4].copy_from_slice(b"TDSh");
        file[36..40].copy_from_slice(b"TDSh");
        let n = file.len();
        let d = Document::new(MemSource(file));
        let mut ev = Evaluator::new(tdms());
        assert_eq!(ev.node(&d, &[]).unwrap().child_count, 2);
        assert!(ev.node(&d, &[0, 2, 5]).unwrap().absent);
        let last = ev.node(&d, &[1]).unwrap();
        assert_eq!(last.offset_bits + last.size_bits, n as u64 * 8);
    }

    /// Numbers and strings written the way round a segment says.
    #[derive(Clone, Copy)]
    pub(super) struct P(pub bool);

    impl P {
        pub fn u32(self, v: u32) -> Vec<u8> {
            if self.0 { v.to_be_bytes().into() } else { v.to_le_bytes().into() }
        }
        pub fn u64(self, v: u64) -> Vec<u8> {
            if self.0 { v.to_be_bytes().into() } else { v.to_le_bytes().into() }
        }
        pub fn str(self, s: &str) -> Vec<u8> {
            let mut v = self.u32(s.len() as u32);
            v.extend_from_slice(s.as_bytes());
            v
        }
        /// An index for `count` values of `kind`, with the length NI's writer
        /// puts in front of it.
        pub fn index(self, kind: u32, count: u64, total: Option<u64>) -> Vec<u8> {
            let mut v = self.u32(if total.is_some() { 28 } else { 20 });
            v.extend(self.u32(kind));
            v.extend(self.u32(1));
            v.extend(self.u64(count));
            if let Some(t) = total {
                v.extend(self.u64(t));
            }
            v
        }
        /// An object: its path, its index (or the four bytes that stand for
        /// one) and its properties, each already packed.
        pub fn object(self, path: &str, index: &[u8], props: &[Vec<u8>]) -> Vec<u8> {
            let mut v = self.str(path);
            v.extend_from_slice(index);
            v.extend(self.u32(props.len() as u32));
            for p in props {
                v.extend_from_slice(p);
            }
            v
        }
        pub fn prop(self, name: &str, kind: u32, value: &[u8]) -> Vec<u8> {
            let mut v = self.str(name);
            v.extend(self.u32(kind));
            v.extend_from_slice(value);
            v
        }
        pub fn metadata(self, objects: &[Vec<u8>]) -> Vec<u8> {
            let mut v = self.u32(objects.len() as u32);
            for o in objects {
                v.extend_from_slice(o);
            }
            v
        }
    }

    /// The objects of the first segment of `file`.
    const OBJECTS: &[usize] = &[0, 2, 3, 1];

    fn at(path: &[usize]) -> Vec<usize> {
        OBJECTS.iter().copied().chain(path.iter().copied()).collect()
    }

    #[test]
    fn an_object_reads_its_index_by_type_and_its_properties_after_it() {
        let p = P(false);
        let meta = p.metadata(&[
            p.object("/", &p.u32(0xFFFF_FFFF), &[p.prop("title", 0x20, &p.str("run 4")), p.prop("gain", 10, &2.5f64.to_le_bytes())]),
            // npTDMS's writer says 20 for a string channel's index and then
            // writes 28 bytes of it. The property after it is where the
            // type says, not where the length does.
            p.object("/'g'/'names'", &[p.u32(20), p.u32(0x20), p.u32(1), p.u64(3), p.u64(20)].concat(), &[p.prop("n", 3, &p.u32(7))]),
            p.object("/'g'/'v'", &p.u32(0), &[]),
        ]);
        let d = Document::new(MemSource(segment(0b1110, false, &meta, &[], None)));
        let mut ev = Evaluator::new(tdms());
        assert_eq!(ev.node(&d, OBJECTS).unwrap().child_count, 3);
        assert_eq!(ev.node(&d, &at(&[0, 0, 1])).unwrap().value, Value::Str("/".into()));
        assert!(ev.node(&d, &at(&[0, 2])).unwrap().absent);
        assert_eq!(ev.node(&d, &at(&[0, 4, 0, 2, 1])).unwrap().value, Value::Str("run 4".into()));
        assert_eq!(ev.node(&d, &at(&[0, 4, 1, 2])).unwrap().value, Value::Float(2.5));
        let names = at(&[1]);
        assert_eq!(ev.node(&d, &[&names[..], &[2, 3]].concat()).unwrap().value, Value::UInt(20));
        assert_eq!(ev.node(&d, &[&names[..], &[4, 0, 2]].concat()).unwrap().value, Value::Int(7));
        let v = ev.node(&d, &at(&[2, 1])).unwrap();
        assert_eq!(v.value, Value::Enum { raw: 0, name: Some("same as before".into()), hex: false });
    }

    /// The same moment written each way round: the fraction first when the
    /// segment is little-endian, the seconds first when it is big-endian.
    #[test]
    fn a_timestamp_puts_its_seconds_where_the_byte_order_does() {
        use crate::eval::Moment;
        // 2025-09-06T10:40:00.5Z, in seconds from 1904 and 2^-64ths.
        let (seconds, fraction) = (3_840_000_000u64, 1u64 << 63);
        for big in [false, true] {
            let p = P(big);
            let value = match big {
                false => [p.u64(fraction), p.u64(seconds)].concat(),
                true => [p.u64(seconds), p.u64(fraction)].concat(),
            };
            let meta = p.metadata(&[p.object("/", &p.u32(0xFFFF_FFFF), &[p.prop("created", 0x44, &value)])]);
            let d = Document::new(MemSource(segment(0b1110, big, &meta, &[], None)));
            let mut ev = Evaluator::new(tdms());
            let stamp = at(&[0, 4, 0, 2]);
            let names: Vec<String> = (0..3).map(|i| ev.node(&d, &[&stamp[..], &[i]].concat()).unwrap().name).collect();
            let seconds_at = names.iter().position(|n| n == "seconds").unwrap();
            assert_eq!(seconds_at, if big { 0 } else { 1 });
            let s = [&stamp[..], &[seconds_at]].concat();
            let time = ev.time_of(&d, &s).unwrap().unwrap();
            assert_eq!(time.moment, Moment::At { unix_seconds: 3_840_000_000 - 2_082_844_800, nanos: 0 });
            assert_eq!(ev.node(&d, &[&stamp[..], &[2]].concat()).unwrap().value, Value::Int(500_000_000));
        }
    }

    #[test]
    fn a_daqmx_index_reads_its_scalers_and_widths() {
        let p = P(false);
        let scaler = [p.u32(3), p.u32(0), p.u32(4), p.u32(0), p.u32(1)].concat();
        let line = [p.u32(0), p.u32(0), p.u32(9), vec![0], p.u32(2)].concat();
        let daqmx = |header: u32, scalers: &[Vec<u8>]| {
            let mut v = [p.u32(header), p.u32(0xFFFF_FFFF), p.u32(1), p.u64(1000), p.u32(scalers.len() as u32)].concat();
            for s in scalers {
                v.extend_from_slice(s);
            }
            v.extend([p.u32(1), p.u32(6)].concat());
            v
        };
        let meta = p.metadata(&[
            p.object("/'dev'/'ai0'", &daqmx(0x1269, &[scaler]), &[]),
            p.object("/'dev'/'port0'", &daqmx(0x126A, &[line.clone(), line]), &[p.prop("x", 5, &[9])]),
        ]);
        let d = Document::new(MemSource(segment(0b1000_1110, false, &meta, &[], None)));
        let mut ev = Evaluator::new(tdms());
        let ai0 = at(&[0, 2]);
        assert_eq!(ev.node(&d, &ai0).unwrap().type_name, "DaqmxIndex");
        assert_eq!(ev.node(&d, &[&ai0[..], &[4, 0, 0]].concat()).unwrap().value.as_int(), Some(3));
        assert_eq!(ev.node(&d, &[&ai0[..], &[6, 0]].concat()).unwrap().value, Value::UInt(6));
        // A digital line scaler is 17 bytes, and the property after two of
        // them lands where it was written.
        let port = at(&[1, 2]);
        assert_eq!(ev.node(&d, &[&port[..], &[4, 1]].concat()).unwrap().size_bits, 17 * 8);
        assert_eq!(ev.node(&d, &at(&[1, 4, 0, 2])).unwrap().value, Value::UInt(9));
    }

    /// A property whose type has no width anyone wrote down takes the rest of
    /// the metadata as bytes, rather than placing what follows at a guess.
    #[test]
    fn a_property_of_no_known_width_ends_what_can_be_read() {
        let p = P(false);
        let meta = p.metadata(&[
            p.object("/", &p.u32(0xFFFF_FFFF), &[p.prop("ext", 11, &[0; 16]), p.prop("after", 3, &p.u32(1))]),
        ]);
        let len = meta.len();
        let d = Document::new(MemSource(segment(0b1110, false, &meta, &[], None)));
        let mut ev = Evaluator::new(tdms());
        let value = ev.node(&d, &at(&[0, 4, 0, 2])).unwrap();
        assert_eq!(value.type_name, "bytes[]");
        assert_eq!(value.offset_bits / 8 + value.size_bits / 8, 28 + len as u64);
    }
}
