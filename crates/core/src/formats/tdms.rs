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
//!
//! **Metadata.** A list of objects, each a path, a raw data index saying how
//! the object's values are laid out in this segment, and properties, each a
//! name, a type and a value. A timestamp is whole seconds from 1904 and a
//! fraction in 2^-64ths, and the seconds are declared a moment. After what the
//! file writes, the metadata works out what the raw data needs: each object's
//! layout, the channels in the order their data is written, and what one chunk
//! of them comes to.
//!
//! **Raw data.** Chunks of every channel's values one channel after another,
//! repeated as often as they fit, or with the interleaved flag, samples of one
//! value of every channel. Strings are a run of end offsets and then the text.
//! DAQmx raw data is named and left as bytes.
//!
//! **What carries over.** The format is a stream: a segment can reuse the
//! whole of the one before, change its channel list without restating it, or
//! say a channel is laid out as before. Every one of those is followed here,
//! but only one link back, since a chain as long as the file cannot be worked
//! out without running out of stack. See [`prior_new_list`] for the rule and
//! the measurements behind it, and [`metadata`] for how a changed list is
//! kept. A segment whose layout cannot be reached that way says its layout is
//! not known, and its raw data is bytes.
//!
//! What was checked: every channel of ten files, counted, first, last and
//! summed against npTDMS 1.11, in `tdms_real.rs`. Four are npTDMS's own test
//! files: LabVIEW's big-endian example, a digital input log from LabVIEW
//! SignalExpress 2011, a DAQmx raw data log and a file of 128 doubles. Six are
//! from the sample collection's generator, which packs by hand the reuse,
//! interleaving, byte order and unfinished writes npTDMS's writer does not
//! produce.
//!
//! What is not read: values of `ExtendedFloat`, `ExtendedFloatWithUnit` and
//! `FixedPoint`, whose widths NI does not give and npTDMS does not read
//! either, and DAQmx samples, which need their scalers applied. One thing is
//! read differently from npTDMS: the tail of a finished segment that is not a
//! whole number of chunks, which npTDMS shares out between the channels by how
//! many values each has, and which this reads channel by channel as it does
//! the chunk a writer did not finish.

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
            (
                "raw_data",
                T::when(is_data().both(E::lit(0).less_than(raw_size())), T::sized(raw_size(), raw_data(e))),
            ),
        ],
    )
    .machinery(&["next_segment_offset", "raw_data_offset"])
}

/// A segment's raw data: which layout it has, and the data read that way.
///
/// A structure of its own rather than the switch that picks the layout, and
/// that is what keeps a long file quick to open. Where a segment ends decides
/// where every one after it starts, so placing the thousandth segment places
/// the nine hundred and ninety-nine before it, and a switch is decided when
/// its field is placed. The layout is worked out from channel lists, sums over
/// them and searches back through earlier segments, and none of that is
/// needed to know how long the raw data is. Behind a structure it is asked
/// only when the raw data is opened.
fn raw_data(e: Endian) -> T {
    T::structure_named(
        "RawData",
        "",
        "data",
        vec![
            ("layout", T::enumeration("RawDataLayout", T::computed(raw_data_layout()), LAYOUT)),
            (
                "data",
                T::switch(
                    E::field("layout"),
                    vec![
                        (CONTIGUOUS, contiguous(e, Here)),
                        (INTERLEAVED, interleaved(e, Here)),
                        (CONTIGUOUS_EARLIER, contiguous(e, Earlier)),
                        (INTERLEAVED_EARLIER, interleaved(e, Earlier)),
                    ],
                    T::bytes(E::Remaining),
                ),
            ),
        ],
    )
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
/// The data is placed when the channel list is known whole and every channel
/// in it has a layout: `list_known` and `unknown_layout_count` in the
/// metadata that lays it out, which is this segment's own or, for a segment
/// with no metadata, the nearest earlier one's.
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
            E::cond(laid_out_by(Here, "list_known"), kind(Here, CONTIGUOUS, INTERLEAVED), E::lit(UNKNOWN)),
            E::cond(
                laid_out_by(Earlier, "list_known"),
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
/// one channel after another. The chunk repeats as many times as fit.
///
/// A writer that stopped partway through a chunk leaves one that is not all
/// there, and what it did write is read the way npTDMS reads a segment the
/// writer never finished: each channel in turn, as many whole values as the
/// bytes left hold, until they run out. A string channel is read only when
/// all of it is there, since its offsets say nothing about strings not
/// written. npTDMS reads the tail of a finished segment whose data is not a
/// whole number of chunks another way, sharing it between channels by how
/// many values each has; no writer is known to leave one, and this reads it
/// the same way as an unfinished one.
fn contiguous(e: Endian, from: LaidOut) -> T {
    let chunk = |partial: bool| {
        T::structure_named(
            "Chunk",
            "",
            "values",
            vec![("values", T::array(channel_values(e, from, partial), E::field("channel_count")))],
        )
        .field_elem_named_from("values", of_channel(from, "path"))
    };
    let mut fields = vec![
        ("channel_count", T::computed(laid_out_by(from, "channel_count"))),
        ("chunk_size", T::computed(laid_out_by(from, "chunk_size"))),
        ("chunk_count", T::computed(whole(E::field("chunk_size")))),
    ];
    fields.extend(copied_channels(from));
    fields.extend([
        ("chunks", T::array(T::sized(E::field("chunk_size"), chunk(false)), E::field("chunk_count"))),
        ("unfinished_chunk", T::when(E::lit(0).less_than(E::Remaining), chunk(true))),
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

/// One channel's values in one chunk, as its type says. In a chunk the
/// writer did not finish, as many as the bytes left hold.
fn channel_values(e: Endian, from: LaidOut, partial: bool) -> T {
    let size = of_channel(from, "data_size");
    let count = |width: i128| match partial {
        false => of_channel(from, "value_count"),
        true => of_channel(from, "value_count").at_most(E::Remaining.div(E::lit(width))),
    };
    let mut cases: Vec<(i128, T)> = scalars(e)
        .into_iter()
        .map(|(k, t)| {
            let width = WIDTH.iter().find(|(w, _)| *w == k).map_or(1, |(_, n)| *n);
            (k, T::array(t, count(width)))
        })
        .collect();
    let text = strings(e, of_channel(from, "value_count"), size.clone());
    cases.push((STRING, if partial { T::when(size.clone().less_or_equal(E::Remaining), text) } else { text }));
    T::switch(of_channel(from, "data_type"), cases, T::bytes(size.at_least(E::lit(0)).at_most(E::Remaining)))
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
/// The fields after that are not in the file. They are the list of channels
/// this segment's raw data is a run of, and what that list adds up to, kept
/// here so that a later segment can ask for them by name.
///
/// **The list, and what carries it.** A segment that sets the new object list
/// flag starts the list over: its channels are its objects that have raw data,
/// in the order they are written. One that does not keeps the list the
/// segment before it ended with and changes it. An object already in that
/// list stays where it is, with the layout this segment gives it; an object
/// that is not is added at the end. That is how NI's writer says a channel's
/// count changed without writing the whole list again.
///
/// No field can hold a list, but a list of fields of no bytes can be worked out
/// from another one. So `channels` in such a segment is as long as the list
/// before it plus what this segment adds, and each entry reads its path and
/// layout from the entry at the same place in the nearest earlier segment
/// with metadata, or from the object here that lists that path, or from the
/// objects this segment adds. A segment after it with no metadata of its own
/// reads that list.
///
/// Two changes are not followed. An object in the list before that this
/// segment says has no raw data: npTDMS takes it out of this segment's list,
/// which moves every channel after it, and a list of entries worked out by
/// position cannot close a gap. And a change to a list that was itself a
/// change: see [`prior_new_list`] for why every question here reaches one
/// segment back and no further. Either way the list is not known, and the raw
/// data laid out by it is bytes.
fn metadata(e: Endian) -> T {
    // One number per channel, for the sums below. There is no way to add up a
    // field of a list of records, so each channel's share is copied out first,
    // the way NetCDF's `record_bytes` is.
    let per_channel = |value: E| T::array(T::computed(value), E::field("channel_count"));
    let of_channel = |field: &str| E::elem_field("channels", E::idx(), &[field]);
    let new_list = || E::field("new_list");
    let has_data = || E::field("has_data");
    let listed_before = || E::field("listed_before");
    T::structure(
        "Metadata",
        vec![
            ("object_count", T::u32(e)),
            ("objects", T::array(object(e), E::field("object_count"))),
            ("padding", T::when(E::lit(0).less_than(E::Remaining), T::bytes(E::Remaining))),
            // The flag, or the first segment, which has no list to change.
            ("new_list", T::computed(E::field("toc").bit(2).either(E::idx().equal_to(E::lit(0))))),
            (
                "carried_count",
                T::computed(E::cond(
                    prior_new_list(),
                    E::sibling(&["contents", "metadata", "channel_count"]),
                    E::lit(0),
                )),
            ),
            ("added_channels", T::when(new_list().negate(), objects_where(has_data().both(listed_before().negate())))),
            ("dropped_channels", T::when(new_list().negate(), objects_where(listed_before().both(has_data().negate())))),
            (
                "channels",
                T::switch(
                    new_list(),
                    vec![(1, objects_where(has_data()))],
                    T::array(kept_channel(), E::field("carried_count").add(E::len_of("added_channels"))),
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
            // Whether `channels` is the whole list: always for a new one, and
            // for a changed one when the list it changed was a new one and
            // nothing was taken out of it.
            (
                "list_known",
                T::computed(E::cond(
                    new_list(),
                    E::lit(1),
                    prior_new_list().both(E::len_of("dropped_channels").equal_to(E::lit(0))),
                )),
            ),
        ],
    )
    .machinery(&[
        "new_list",
        "carried_count",
        "added_channels",
        "dropped_channels",
        "channel_count",
        "data_sizes",
        "chunk_size",
        "widths",
        "frame_size",
        "unknown_layouts",
        "unknown_layout_count",
        "no_widths",
        "no_width_count",
        "list_known",
    ])
}

/// The objects of this segment for which `cond` holds, in the order they are
/// written. An object it does not hold for gives an offset before the list's
/// own start, which is what passes a record over; the channels take no bytes,
/// so the offset of every other one is nothing.
fn objects_where(cond: E) -> T {
    T::gather(
        vec![Step::field("objects"), Step::each()],
        E::cond(cond, E::lit(0), E::lit(-1)),
        Anchor::Window,
        E::lit(0),
        channel(),
    )
}

/// One channel, read again from the object that placed it. `object_at` is
/// where that object is written, in bytes from the start of the file, and
/// `number` is where the channel is in the list, which is what a later segment
/// finds it by.
fn channel() -> T {
    let placer = |field: &str| E::placer(E::field(field));
    T::structure_named(
        "Channel",
        "path",
        "",
        vec![
            ("path", T::computed_text(E::placer(E::within(&["path", "text"])))),
            ("data_type", computed_data_type(placer("data_type"))),
            ("value_count", T::computed(placer("value_count"))),
            ("width", T::computed(placer("width"))),
            ("data_size", T::computed(placer("data_size"))),
            ("layout_known", T::computed(placer("layout_known"))),
            ("object_at", T::computed(E::start_of(E::placer(E::field("path"))))),
            ("number", T::computed(E::idx())),
        ],
    )
}

/// One entry of a list this segment changed rather than started: the entry at
/// the same place in the list before, as this segment lists it if it does,
/// or one of the channels this segment adds after the ones it kept.
fn kept_channel() -> T {
    let kept = || E::idx().less_than(E::field("carried_count"));
    let before = |field: &str| {
        E::Tagged(Arc::new(TaggedRef {
            array: Some(E::sibling(&["contents", "metadata", "channels"])),
            key: path(&["number"]),
            tag: Tag::Computed(Arc::new(E::idx())),
            field: path(&[field]),
        }))
    };
    // The object in this segment with the same path as that entry, if any.
    let listed = |field: &[&str]| {
        E::Tagged(Arc::new(TaggedRef {
            array: Some(E::field("objects")),
            key: path(&["path", "text"]),
            tag: Tag::ComputedText(Arc::new(before("path"))),
            field: path(field),
        }))
    };
    // Every path is at least `/`, so a length of nothing is nothing found.
    let relisted = || listed(&["path", "length"]).not_equal(E::lit(0));
    let added = |field: &str| E::elem_field("added_channels", E::idx().sub(E::field("carried_count")), &[field]);
    let pick = |field: &str| E::cond(kept(), E::cond(relisted(), listed(&[field]), before(field)), added(field));
    T::structure_named(
        "Channel",
        "path",
        "",
        vec![
            (
                "path",
                T::switch(kept(), vec![(1, T::computed_text(before("path")))], T::computed_text(added("path"))),
            ),
            ("data_type", computed_data_type(pick("data_type"))),
            ("value_count", T::computed(pick("value_count"))),
            ("width", T::computed(pick("width"))),
            ("data_size", T::computed(pick("data_size"))),
            ("layout_known", T::computed(pick("layout_known"))),
            (
                "object_at",
                T::computed(E::cond(
                    kept(),
                    E::cond(relisted(), E::start_of(listed(&["path"])), before("object_at")),
                    added("object_at"),
                )),
            ),
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
/// After the fields the file writes come seven it does not: whether the
/// object was in the channel list this segment changes, and the layout its raw
/// data has in this segment, worked out once here so that the list and later
/// segments can ask for it by name.
///
/// A raw data index of 0 says the layout is what it was last time, and one of
/// all ones leaves the last layout standing for later. npTDMS keeps the last
/// layout of every path it has seen, however far back. Here the layout is
/// taken from the object for the same path in the nearest earlier segment with
/// metadata, when that object wrote an index out; see [`prior_new_list`].
fn object(e: Endian) -> T {
    let index = E::field("raw_data_index");
    let explicit = |index: E| index.clone().not_equal(E::lit(NO_DATA)).both(index.not_equal(E::lit(SAME_AS_BEFORE)));
    let daqmx = |index: E| {
        index
            .clone()
            .equal_to(E::lit(FORMAT_CHANGING))
            .either(index.clone().equal_to(E::lit(DIGITAL_LINE)))
            .either(index.equal_to(E::lit(DIGITAL_LINE_AS_DOCUMENTED)))
    };
    let before = || earlier_object(&["raw_data_index"]);
    // What the layout is read from: this object's index when it wrote one,
    // otherwise the earlier object's when that one did, otherwise nothing.
    let from_here = || explicit(index.clone());
    let from_before = || explicit(before());
    let pick = |field: &str, missing: i128| {
        E::cond(
            from_here(),
            E::within(&["index", field]),
            E::cond(from_before(), earlier_object(&["index", field]), E::lit(missing)),
        )
    };
    let kind = E::field("data_type");
    let count = E::field("value_count");
    let width = E::field("width");
    let is_daqmx = || E::cond(from_here(), daqmx(index.clone()), daqmx(before()));
    // A string channel says how many bytes it takes; every other type is its
    // width times its count. DAQmx data is laid out by its scalers and not by
    // anything a template can add up.
    let size = E::cond(
        is_daqmx(),
        E::lit(0),
        E::cond(kind.clone().equal_to(E::lit(STRING)), pick("total_size", 0), width.clone().mul(count.clone())),
    );
    let known = from_here()
        .either(from_before())
        .both(is_daqmx().negate())
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
                    from_here(),
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
            // In the list this segment changes, when it changes one rather
            // than starting a new one. Every entry of a list was written
            // somewhere past the first lead-in, so an entry not found is an
            // object at nothing.
            (
                "listed_before",
                T::computed(
                    E::field("toc")
                        .bit(2)
                        .negate()
                        .both(prior_new_list())
                        .both(in_earlier_list(&["object_at"]).not_equal(E::lit(0))),
                ),
            ),
            ("data_type", computed_data_type(pick("data_type", NOT_KNOWN))),
            ("value_count", T::computed(pick("value_count", 0))),
            ("width", T::switch(E::field("data_type"), widths, T::computed(E::lit(0)))),
            ("data_size", T::computed(size)),
            ("layout_known", T::computed(known)),
        ],
    )
    .machinery(&["has_data", "listed_before", "data_type", "value_count", "width", "data_size", "layout_known"])
}

/// A data type worked out rather than read, which may be one nobody wrote.
const NOT_KNOWN: i128 = -1;

fn computed_data_type(value: E) -> T {
    let mut cases = DATA_TYPE.to_vec();
    cases.push((NOT_KNOWN, "not known"));
    T::enumeration_hex("DataType", T::computed(value), &cases)
}

/// Every question this template asks of an earlier segment goes to the nearest
/// one with metadata, and takes its answer from what that segment wrote
/// rather than from what that segment worked out in turn.
///
/// The format itself carries state from segment to segment without limit. A
/// channel can say its layout is the same as before in a thousand segments
/// running, and a list can be changed a thousand times over. Following that
/// here means a question inside a question inside a question, one level for
/// every segment the state was carried through. Worked out cold from the end
/// of a long file, that is as deep as the file is long: a file of eighty
/// segments that each say "same as before" runs out of stack in a release
/// build, and three thousand segments that each change the list had not
/// finished after ten minutes, since every level asks for its paths again as
/// text, which is not kept. Nothing in the IR can walk back through earlier
/// segments by a path without asking each one's answer in turn.
///
/// So the chain stops at one link. A raw data index of 0 is followed to the
/// object for the same path in the nearest earlier segment with metadata, and
/// reads the index that object wrote; one that itself said "same as before"
/// is not followed further, and the layout is not known. A list changed by a
/// segment without the new list flag is known when the list it changed was a
/// new one. A segment with no metadata reads whatever list the nearest segment
/// with metadata has, which asks at most one segment further back again.
///
/// That covers what the samples hold: LabVIEW's big-endian example lists its
/// channels again in its second segment with an index of 0, and a segment with
/// no metadata reads the list however far back it is, three thousand segments
/// in well under a second. A file whose writer says "same as before" or
/// changes the list in segment after segment reads to the first link and is
/// bytes after it.
///
/// This asks the first of those questions: whether the nearest earlier
/// segment with metadata started a new list.
fn prior_new_list() -> E {
    E::sibling(&["contents", "metadata", "new_list"]).equal_to(E::lit(1))
}

/// `field` of the object for this object's path in the nearest earlier segment
/// with metadata, and nothing when that segment has no such object.
fn earlier_object(field: &[&str]) -> E {
    E::Tagged(Arc::new(TaggedRef {
        array: Some(E::sibling(&["contents", "metadata", "objects"])),
        key: path(&["path", "text"]),
        tag: Tag::ComputedText(Arc::new(E::within(&["path", "text"]))),
        field: path(field),
    }))
}

/// `field` of the entry for this object's path in the channel list of the
/// nearest earlier segment with metadata. Nothing when that list has no such
/// path, and `object_at` is never nothing for an entry that is there.
fn in_earlier_list(field: &[&str]) -> E {
    E::Tagged(Arc::new(TaggedRef {
        array: Some(E::sibling(&["contents", "metadata", "channels"])),
        key: path(&["path"]),
        tag: Tag::ComputedText(Arc::new(E::within(&["path", "text"]))),
        field: path(field),
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
        assert_eq!(ev.node(&d, &[0, 2, 4]).unwrap().size_bits, 16 * 8);
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
        assert_eq!(ev.node(&d, &[0, 2, 4]).unwrap().size_bits, 40 * 8);
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
        assert!(ev.node(&d, &[0, 2, 4]).unwrap().absent);
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

    /// The flags a segment's table of contents is built from.
    const META: u32 = 1 << 1;
    const NEW_LIST: u32 = 1 << 2;
    const RAW: u32 = 1 << 3;
    const INTERLEAVED_FLAG: u32 = 1 << 5;

    fn doubles(values: &[f64]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    /// A file of `segments`, each already packed.
    fn file(segments: &[Vec<u8>]) -> (Document<MemSource>, Evaluator) {
        (Document::new(MemSource(segments.concat())), Evaluator::new(tdms()))
    }

    /// Segment `k`'s raw data layout, by name.
    fn layout(ev: &mut Evaluator, d: &Document<MemSource>, k: usize) -> String {
        match ev.node(d, &[k, 2, 4, 0]).unwrap().value {
            Value::Enum { name: Some(name), .. } => name,
            other => panic!("{other:?}"),
        }
    }

    /// Segment `k`'s data, whatever it was read as.
    fn data(ev: &mut Evaluator, d: &Document<MemSource>, k: usize) -> crate::eval::NodeInfo {
        ev.node(d, &[k, 2, 4, 1]).unwrap()
    }

    /// A channel said to be laid out as before takes the index the segment
    /// before wrote for its path. One said to be laid out as before a second
    /// time is not followed back again, and its segment is bytes.
    #[test]
    fn same_as_before_is_followed_one_segment_back() {
        let p = P(false);
        let a = "/'g'/'a'";
        let (d, mut ev) = file(&[
            segment(META | NEW_LIST | RAW, false, &p.metadata(&[p.object(a, &p.index(10, 2, None), &[])]), &doubles(&[1.0, 2.0]), None),
            segment(META | NEW_LIST | RAW, false, &p.metadata(&[p.object(a, &p.u32(0), &[])]), &doubles(&[3.0, 4.0, 5.0, 6.0]), None),
            segment(META | NEW_LIST | RAW, false, &p.metadata(&[p.object(a, &p.u32(0), &[])]), &doubles(&[7.0, 8.0]), None),
        ]);
        assert_eq!(layout(&mut ev, &d, 1), "contiguous");
        // Two chunks of the two doubles the first segment's index said.
        let second = data(&mut ev, &d, 1);
        assert_eq!(ev.node(&d, &[&second.path[..], &[3]].concat()).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[1, 2, 4, 1, 3, 1, 0, 0, 1]).unwrap().value, Value::Float(6.0));
        // The object says what it read its layout from, and the third one
        // reads nothing.
        assert_eq!(ev.node(&d, &[1, 2, 3, 1, 0, 7]).unwrap().value.as_int(), Some(10));
        let third = [2, 2, 3, 1, 0, 7];
        assert_eq!(ev.node(&d, &third).unwrap().value, Value::Enum { raw: NOT_KNOWN, name: Some("not known".into()), hex: true });
        assert_eq!(layout(&mut ev, &d, 2), "not known");
        assert_eq!((data(&mut ev, &d, 2).type_name.as_str(), data(&mut ev, &d, 2).size_bits), ("bytes[]", 16 * 8));
    }

    /// A segment with no metadata reads the list of the nearest one that has
    /// some, however many segments back.
    #[test]
    fn a_segment_without_metadata_reads_the_nearest_list_before_it() {
        let p = P(false);
        let (a, b) = ("/'g'/'a'", "/'g'/'b'");
        let meta = p.metadata(&[p.object(a, &p.index(10, 1, None), &[]), p.object(b, &p.index(3, 2, None), &[])]);
        let chunk = [doubles(&[1.5]), 7i32.to_le_bytes().to_vec(), 8i32.to_le_bytes().to_vec()].concat();
        let (d, mut ev) = file(&[
            segment(META | NEW_LIST | RAW, false, &meta, &chunk, None),
            segment(RAW, false, &[], &chunk, None),
            segment(RAW, false, &[], &[chunk.clone(), chunk.clone()].concat(), None),
        ]);
        assert_eq!(layout(&mut ev, &d, 2), "contiguous, laid out by an earlier segment");
        let chunks = [2, 2, 4, 1, 4];
        assert_eq!(ev.node(&d, &chunks).unwrap().child_count, 2);
        let b1 = ev.node(&d, &[&chunks[..], &[1, 0, 1]].concat()).unwrap();
        assert_eq!((b1.name.as_str(), b1.type_name.as_str()), ("[1] /'g'/'b'", "i32 le[]"));
        assert_eq!(ev.node(&d, &[&chunks[..], &[1, 0, 1, 1]].concat()).unwrap().value, Value::Int(8));
    }

    /// A segment that changes the list rather than starting one keeps the
    /// channels it does not mention, gives the one it mentions its new layout
    /// in place, and adds a new one at the end.
    #[test]
    fn a_changed_list_keeps_its_order_and_adds_at_the_end() {
        let p = P(false);
        let (a, b, c) = ("/'g'/'a'", "/'g'/'b'", "/'g'/'c'");
        let first = p.metadata(&[p.object(a, &p.index(10, 1, None), &[]), p.object(b, &p.index(10, 1, None), &[])]);
        // b now holds two values, and c is new.
        let second = p.metadata(&[p.object(c, &p.index(5, 3, None), &[]), p.object(b, &p.index(10, 2, None), &[])]);
        let (d, mut ev) = file(&[
            segment(META | NEW_LIST | RAW, false, &first, &doubles(&[1.0, 2.0]), None),
            segment(META | RAW, false, &second, &[doubles(&[3.0, 4.0, 5.0]), vec![6, 7, 8]].concat(), None),
        ]);
        assert_eq!(layout(&mut ev, &d, 1), "contiguous");
        let channels = [1, 2, 3, 7];
        let names: Vec<String> = (0..3).map(|i| ev.node(&d, &[&channels[..], &[i]].concat()).unwrap().name).collect();
        assert_eq!(names, ["[0] /'g'/'a'", "[1] /'g'/'b'", "[2] /'g'/'c'"]);
        assert_eq!(ev.node(&d, &[1, 2, 4, 1, 3, 0, 0, 1, 1]).unwrap().value, Value::Float(5.0));
        assert_eq!(ev.node(&d, &[1, 2, 4, 1, 3, 0, 0, 2, 2]).unwrap().value, Value::UInt(8));
        // Where each entry's layout was written: a in the first segment, b and
        // c in this one.
        let at = |ev: &mut Evaluator, i: usize| ev.node(&d, &[&channels[..], &[i, 6]].concat()).unwrap().value.as_int().unwrap();
        assert!(at(&mut ev, 0) < 60 && at(&mut ev, 1) > 60 && at(&mut ev, 2) > 60);
    }

    /// Taking a channel out of a list changed rather than started moves every
    /// channel after it, which a list worked out by position cannot follow.
    #[test]
    fn a_channel_taken_out_of_a_changed_list_leaves_the_data_as_bytes() {
        let p = P(false);
        let (a, b) = ("/'g'/'a'", "/'g'/'b'");
        let first = p.metadata(&[p.object(a, &p.index(10, 1, None), &[]), p.object(b, &p.index(10, 1, None), &[])]);
        let second = p.metadata(&[p.object(a, &p.u32(0xFFFF_FFFF), &[])]);
        let (d, mut ev) = file(&[
            segment(META | NEW_LIST | RAW, false, &first, &doubles(&[1.0, 2.0]), None),
            segment(META | RAW, false, &second, &doubles(&[3.0]), None),
        ]);
        assert_eq!(ev.node(&d, &[1, 2, 3, 6]).unwrap().child_count, 1);
        assert_eq!(layout(&mut ev, &d, 1), "not known");
    }

    /// Interleaved data is a sample of one value per channel, which a string
    /// channel has no width to be part of.
    #[test]
    fn interleaved_data_with_a_string_channel_is_not_placed() {
        let p = P(false);
        let meta = p.metadata(&[p.object("/'g'/'n'", &p.index(3, 1, None), &[]), p.object("/'g'/'s'", &p.index(0x20, 1, Some(5)), &[])]);
        let raw = [7i32.to_le_bytes().to_vec(), p.u32(1), b"x".to_vec()].concat();
        let (d, mut ev) = file(&[segment(META | NEW_LIST | RAW | INTERLEAVED_FLAG, false, &meta, &raw, None)]);
        assert_eq!(layout(&mut ev, &d, 0), "not known");
        assert_eq!(data(&mut ev, &d, 0).type_name, "bytes[]");
    }
}
