//! MiniSEED: the data records of SEED, which is what the seismometers of the
//! world send and what IRIS and the USGS hand back when you ask them for a
//! waveform.
//!
//! A file is a run of records and nothing else. There is no file header, no
//! index and no end marker: a record is 512 or 4096 bytes, the next one
//! starts where it stops, and a program reading the stream off a serial line
//! in 1990 could start anywhere and find its footing again. Each record has
//! forty-eight bytes of fixed header, then a chain of blockettes, then the
//! samples.
//!
//! Two things about a record are not written in its fixed header, and both
//! have to be worked out:
//!
//! *Which way round the numbers are.* Nothing says. What settles it is the
//! start time: the year is a 16-bit number two bytes into the time, and 2008
//! the wrong way round is 55,303. The template peeks at it both ways and lays
//! the record out in whichever gives a year somebody could have recorded in.
//! That is what libmseed does, and there is nothing better available.
//!
//! *How long the record is.* Blockette 1000 says, as a power of two, and
//! blockette 1000 is inside the record whose length it settles. The template
//! peeks at the first blockette in the chain and at the one after it, which
//! covers every writer in practice; a record that puts blockette 1000 third,
//! or leaves it out, is read as running to the end of the file, which is the
//! honest answer to a record that never said how long it was.
//!
//! The samples are exposed as far as their shape goes, not as far as their
//! values. Steim1 and Steim2 pack differences into 64-byte frames of sixteen
//! 32-bit words, where the first word is sixteen 2-bit codes saying what the
//! other fifteen hold, and this template reads those codes and types each
//! word by what its own code says: four 8-bit differences, two 16-bit, one
//! 32-bit, or for Steim2 a second code inside the word itself choosing
//! between seven 4-bit differences and one 30-bit. Word 1 and word 2 of the
//! first frame are the forward and reverse integration constants, which is
//! what turns differences back into samples. Undoing the differencing is not
//! done here.
//!
//! A little-endian record swaps less than it looks like it should, and this is
//! the part worth knowing. A Steim word that holds whole differences is not
//! swapped as a word at all: four 1-byte differences are the four bytes where
//! they lie, two 2-byte ones are each turned round on their own, and a 4-byte
//! one is the whole word turned round. All of Steim1 is that, and so is
//! Steim2's four-byte case, and all of it is a matter of naming fields at
//! their own width and byte order.
//!
//! Steim2's bit-packed words are the exception. Those are swapped whole and
//! then cut up, and the IR slices a run of *bits* in the order they are
//! written, so cutting one where it lies would name the wrong bits. Such a
//! word is read whole and the differences hang off it as computed fields,
//! worked out from its value with shifts. The word keeps all four of its bytes
//! and the differences take none, which is the honest picture: what is on disk
//! is a word, and the differences are something a reader works out.
//!
//! Blockettes read: 100, 200, 201, 300, 310, 320, 390, 395, 400, 405, 500,
//! 1000, 1001 and 2000, from libmseed's `libmseed.h`. The flag bytes of the
//! event and calibration blockettes are left as numbers rather than named:
//! what each bit means differs enough between them that a wrong name would be
//! worse than none.
//!
//! Encodings typed: 0 to 5, 10 and 11, 12 to 18, 30 and 32. The gain-ranged
//! ones, 12 to 18 and 30, are exposed at their sample width and no further;
//! which bits of a GEOSCOPE or a CDSN word are the gain differs by network.
//! 19 (Steim3) and 31 (HGLP) stay bytes, having no fixed width here to go on.

use crate::template::{Encoding, Endian, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};

/// How long the fixed header is, which is where the blockette chain starts
/// in every record anybody writes.
const FIXED: i128 = 48;

/// Where the start time's year is written. Reading it both ways round is what
/// tells a big-endian record from a little-endian one.
pub const YEAR_AT: usize = 20;

/// Where the offset of the first blockette is written.
const FIRST_BLOCKETTE_AT: i128 = 46;

/// Blockette 1000: the one that says how long the record is and how the
/// samples are packed. Every miniSEED record has one; a full SEED volume's
/// data records may not, which is what the fallback below is for.
const B1000: i128 = 1000;

/// The blockettes worth naming. The ones this template reads are 1000, 1001
/// and 100; the rest are named and left as bytes, since what is in them is
/// station metadata rather than anything the record's shape depends on.
const BLOCKETTE_TYPE: &[(i128, &str)] = &[
    (100, "sample rate"),
    (200, "generic event detection"),
    (201, "Murdock event detection"),
    (300, "step calibration"),
    (310, "sine calibration"),
    (320, "pseudo-random calibration"),
    (390, "generic calibration"),
    (395, "calibration abort"),
    (400, "beam"),
    (405, "beam delay"),
    (500, "timing"),
    (1000, "data only SEED"),
    (1001, "data extension"),
    (2000, "opaque data"),
];

/// How the samples are written, by the number blockette 1000 gives.
const ENCODING: &[(i128, &str)] = &[
    (0, "ASCII text"),
    (1, "16-bit integers"),
    (2, "24-bit integers"),
    (3, "32-bit integers"),
    (4, "32-bit floats"),
    (5, "64-bit floats"),
    (10, "Steim1"),
    (11, "Steim2"),
    (12, "GEOSCOPE 24-bit"),
    (13, "GEOSCOPE 16-bit gain 3"),
    (14, "GEOSCOPE 16-bit gain 4"),
    (15, "US National Network"),
    (16, "CDSN 16-bit gain"),
    (17, "Graefenberg 16-bit gain"),
    (18, "IPG Strasbourg 16-bit gain"),
    (19, "Steim3"),
    (30, "SRO"),
    (31, "HGLP"),
    (32, "DWWSSN"),
    (33, "RSTN 16-bit gain"),
];

const WORD_ORDER: &[(i128, &str)] = &[(0, "little-endian"), (1, "big-endian")];

/// What the record says was happening while it was recorded.
const ACTIVITY_FLAGS: &[(u32, &str)] = &[
    (0, "calibration signals present"),
    (1, "time correction applied"),
    (2, "beginning of an event"),
    (3, "end of an event"),
    (4, "positive leap second"),
    (5, "negative leap second"),
    (6, "event in progress"),
];

/// What happened to the record on its way here.
const IO_FLAGS: &[(u32, &str)] = &[
    (0, "station volume parity error"),
    (1, "long record read"),
    (2, "short record read"),
    (3, "start of time series"),
    (4, "end of time series"),
    (5, "clock locked"),
];

/// What is wrong with the samples, as the digitiser saw it. These are the
/// flags a seismologist throwing data out reads.
const QUALITY_FLAGS: &[(u32, &str)] = &[
    (0, "amplifier saturation"),
    (1, "digitiser clipping"),
    (2, "spikes"),
    (3, "glitches"),
    (4, "missing or padded data"),
    (5, "telemetry synchronisation error"),
    (6, "digital filter charging"),
    (7, "time tag questionable"),
];

pub fn mseed() -> Template {
    Template::new("mseed", T::structure("MiniSEED", vec![("records", T::repeat(record(), Until::End))]))
}

/// One record: big-endian, little-endian, or not a record at all.
///
/// The last of those three is the point. A file that ends part way through a
/// record, or that has a run of samples left over from a record whose header
/// never arrived, still shows what is there rather than reading a header out
/// of compressed data and failing on every field of it. A recording cut off
/// mid-transmission is exactly the file somebody opens a hex editor to look
/// at.
fn record() -> T {
    let tail = T::structure("MiniSEEDTail", vec![("bytes", T::bytes(E::Remaining))]);
    // A year somebody could have recorded in. The first digital seismic
    // networks are from the 1960s; the wrong byte order gives tens of
    // thousands, and so does a header that is not a header.
    let plausible = |e| {
        let year = E::peek_at(E::lit(YEAR_AT as i128 * 8), 16, e);
        year.clone().less_than(E::lit(2101)).mul(E::lit(1799).less_than(year))
    };
    let little = T::switch(plausible(Little), vec![(1, record_of(Little))], tail.clone());
    let whole = T::switch(plausible(Big), vec![(1, record_of(Big))], little);
    T::switch(E::Remaining.less_than(E::lit(FIXED)), vec![(1, tail)], whole)
}

/// Where blockette 1000 is, as an offset from the start of the record.
///
/// The chain is followed by hand for two links, because an expression cannot
/// loop: the first blockette, and the one its `next` points at. Every writer
/// puts blockette 1000 first or second. When neither is it, this lands on the
/// second link's offset and the type read there is not 1000, which is what
/// [`found_b1000`] asks and what sends the record to its fallback.
fn b1000_at(e: Endian) -> E {
    let first = peek16(E::lit(FIRST_BLOCKETTE_AT), e);
    let next = peek16(first.clone().add(E::lit(2)), e);
    let here = is_b1000(first.clone(), e);
    // `here * first + (1 - here) * next`: there is no conditional expression,
    // and a comparison multiplied by each side is what one is written as.
    here.clone().mul(first).add(E::lit(1).sub(here).mul(next))
}

/// Whether blockette 1000 was found at all, which decides whether the record
/// has a length or runs to the end of the file.
fn found_b1000(e: Endian) -> E {
    is_b1000(b1000_at(e), e)
}

fn is_b1000(at: E, e: Endian) -> E {
    let t = peek16(at, e);
    t.clone().less_than(E::lit(B1000 + 1)).mul(E::lit(B1000 - 1).less_than(t))
}

/// A 16-bit number at `at` bytes into the record, or 1 when the record does
/// not reach that far. One is not a blockette type and not an offset any
/// reader will act on, so a chain that points off the end reads as a chain
/// that ended.
fn peek16(at: E, e: Endian) -> E {
    let short = E::Remaining.less_than(at.clone().add(E::lit(2)));
    short.or(E::peek_at(at.mul(E::lit(8)), 16, e))
}

/// An 8-bit number at `at` bytes into the record, or zero when the record
/// does not reach that far.
///
/// Nothing here needs to tell an absent byte from a zero one: the caller has
/// already asked whether blockette 1000 was found at all.
fn peek8(at: E) -> E {
    let short = E::Remaining.less_than(at.clone().add(E::lit(1)));
    short.or(E::peek_at(at.mul(E::lit(8)), 8, Big))
}

/// The record, laid out for one byte order, with the samples typed by what
/// blockette 1000 says they are.
///
/// The encoding has to be settled here rather than at the data field, because
/// a peek is measured from the field it is written on and blockette 1000 is
/// behind the cursor by then.
fn record_of(e: Endian) -> T {
    let at = b1000_at(e);
    let encoding = peek8(at.add(E::lit(4)));
    T::switch(
        encoding,
        vec![
            (0, sized(e, T::text(StrLen::Fixed(E::Remaining), Encoding::Ascii))),
            (1, sized(e, samples(T::Int { bits: 16, endian: e }, 2))),
            (2, sized(e, samples(T::Int { bits: 24, endian: e }, 3))),
            (3, sized(e, samples(T::Int { bits: 32, endian: e }, 4))),
            (4, sized(e, samples(T::F32(e), 4))),
            (5, sized(e, samples(T::F64(e), 8))),
            (10, sized(e, steim(e, false))),
            (11, sized(e, steim(e, true))),
            // Three bytes a sample, a gain and a mantissa packed together in a
            // shape that is not a number of any width. Named, and left alone.
            (12, sized(e, gain_ranged(T::bytes(E::lit(3)), 3))),
            (13, sized(e, gain_ranged(T::u16(e), 2))),
            (14, sized(e, gain_ranged(T::u16(e), 2))),
            (15, sized(e, gain_ranged(T::u16(e), 2))),
            (16, sized(e, gain_ranged(T::u16(e), 2))),
            (17, sized(e, gain_ranged(T::u16(e), 2))),
            (18, sized(e, gain_ranged(T::u16(e), 2))),
            (30, sized(e, gain_ranged(T::u16(e), 2))),
            (32, sized(e, samples(T::Int { bits: 16, endian: e }, 2))),
        ],
        sized(e, T::bytes(E::Remaining)),
    )
}

/// The gain-ranged encodings: one sample is a fixed number of bytes holding a
/// mantissa and the gain the amplifier was on, packed differently by every
/// network that invented one.
///
/// The width is the same for all of them and that is what is exposed. Which
/// bits are the gain is not: it differs between GEOSCOPE, CDSN, SRO and the
/// rest, and a wrong split would read as numbers rather than as the words it
/// got wrong.
fn gain_ranged(elem: T, width: i128) -> T {
    samples(elem, width)
}

/// The record in the room blockette 1000 gives it: two to the power of the
/// exponent it holds, which is 9 for a 512-byte record and 12 for a 4096-byte
/// one. A record whose length nothing states runs to the end of the file.
fn sized(e: Endian, data: T) -> T {
    let exponent = peek8(b1000_at(e).add(E::lit(6))).at_most(E::lit(20));
    // Never shorter than the fixed header, and never past the end of the
    // file: a record cut off in transmission reads as far as it goes.
    let len = E::lit(1).shl(exponent).at_least(E::lit(FIXED)).at_most(E::Remaining);
    T::switch(found_b1000(e), vec![(1, T::sized(len, body(e, data.clone())))], T::sized(E::Remaining, body(e, data)))
}

fn body(e: Endian, data: T) -> T {
    // Where the blockette chain ends, which is where the samples start. A
    // record with no samples writes zero there and its blockettes run to the
    // end of the record.
    let has_data = E::lit(FIXED).less_than(E::field("data_offset"));
    let padding = has_data
        .mul(E::field("data_offset").sub(E::lit(FIXED)).sub(E::size_of("blockettes")))
        .at_least(E::lit(0));
    T::structure_named(
        "MiniSEEDRecord",
        "channel",
        "data",
        vec![
            // Six digits counting the records as they were written. Some
            // writers leave it blank, which is why it is text and not a
            // number.
            ("sequence_number", T::text(StrLen::Padded { size: E::lit(6), pad: b' ' }, Encoding::Ascii)),
            // How far the data has been checked: D for indeterminate, R
            // for raw, Q for quality controlled, M for merged.
            ("data_quality", T::text(StrLen::Fixed(E::lit(1)), Encoding::Ascii)),
            ("reserved", T::bytes(E::lit(1))),
            // The four names that say which channel of which station in which
            // network this is, each padded with spaces to its own width.
            ("station", name(5)),
            ("location", name(2)),
            ("channel", name(3)),
            ("network", name(2)),
            ("start_time", btime(e)),
            ("sample_count", T::u16(e)),
            // The sample rate as a pair: the two are multiplied when both are
            // positive, and divided when one is negative, which is how one
            // pair of small integers reaches both 100 Hz and one sample a
            // day.
            ("sample_rate_factor", T::Int { bits: 16, endian: e }),
            ("sample_rate_multiplier", T::Int { bits: 16, endian: e }),
            ("activity_flags", T::flags("ActivityFlags", T::u8(), ACTIVITY_FLAGS)),
            ("io_flags", T::flags("IOFlags", T::u8(), IO_FLAGS)),
            ("quality_flags", T::flags("QualityFlags", T::u8(), QUALITY_FLAGS)),
            ("blockette_count", T::u8()),
            // In units of 0.0001 seconds, to be added to the start time
            // unless the activity flags say it already has been.
            ("time_correction", T::Int { bits: 32, endian: e }),
            ("data_offset", T::u16(e)),
            ("first_blockette_offset", T::u16(e)),
            ("blockettes", T::array(blockette(e), E::field("blockette_count"))),
            ("header_padding", T::bytes(padding)),
            ("data", data),
        ],
    )
    .machinery(&["reserved", "first_blockette_offset", "header_padding"])
    .payload(&["station", "channel", "sample_count"])
    .counted_as("record")
}

fn name(bytes: i128) -> T {
    T::text(StrLen::Padded { size: E::lit(bytes), pad: b' ' }, Encoding::Ascii)
}

/// SEED's ten-byte time: the one place in the format where the fraction of a
/// second is a separate field, counted in ten-thousandths.
fn btime(e: Endian) -> T {
    T::inline_structure(
        "BTime",
        vec![
            ("year", T::u16(e)),
            ("day", T::u16(e)), // of the year, 1 to 366
            ("hour", T::u8()),
            ("minute", T::u8()),
            ("second", T::u8()), // 60 during a leap second
            ("unused", T::u8()),
            ("fraction", T::u16(e)), // ten-thousandths of a second
        ],
    )
}

/// One blockette. Every one opens with its type and where the next one
/// starts, and what is between those two offsets is the body, whether or not
/// anything here knows what the type means.
fn blockette(e: Endian) -> T {
    // Where this blockette starts, which the file writes only as the previous
    // one's forward pointer.
    let start = E::prev("next").or(E::field("first_blockette_offset"));
    let has_next = E::lit(0).less_than(E::field("next"));
    let to_next = E::field("next").sub(start).sub(E::lit(4)).at_least(E::lit(0));
    // A blockette nothing here reads runs to where the next one starts, or to
    // the end of the chain when it is the last.
    let rest = has_next.clone().mul(to_next.clone()).or(E::Remaining);
    // What a body this template does read leaves before the next blockette.
    let padding = has_next.mul(to_next.sub(E::size_of("body"))).at_least(E::lit(0));
    T::structure_named(
        "MiniSEEDBlockette",
        "type",
        "body",
        vec![
            ("type", T::enumeration("BlocketteType", T::u16(e), BLOCKETTE_TYPE)),
            // From the start of the record, not from here. Zero ends the chain.
            ("next", T::u16(e)),
            (
                "body",
                T::switch(
                    E::field("type"),
                    vec![
                        (100, b100_body(e)),
                        (200, b200_body(e)),
                        (201, b201_body(e)),
                        (300, b300_body(e)),
                        (310, b310_body(e)),
                        (320, b320_body(e)),
                        (390, b390_body(e)),
                        (395, b395_body(e)),
                        (400, b400_body(e)),
                        (405, b405_body(e, rest.clone())),
                        (500, b500_body(e)),
                        (1000, b1000_body()),
                        (1001, b1001_body()),
                        (2000, b2000_body(e)),
                    ],
                    T::bytes(rest),
                ),
            ),
            ("padding", T::bytes(padding)),
        ],
    )
    .counted_as("blockette")
}

/// A name padded with spaces, as every text field of a blockette is written.
fn padded(bytes: i128) -> T {
    T::text(StrLen::Padded { size: E::lit(bytes), pad: b' ' }, Encoding::Ascii)
}

/// Blockette 200: something crossed a threshold. What the detector saw, and
/// when it saw it.
///
/// The flag byte is left as a number. Its bits say whether the wave was a
/// compression or a dilatation, whether the amplitude is in counts or in
/// units, and whether this is the start of the event or its end; naming them
/// wrong would be worse than not naming them.
fn b200_body(e: Endian) -> T {
    T::structure(
        "GenericEventDetection",
        vec![
            ("amplitude", T::F32(e)),
            ("period", T::F32(e)),
            ("background_estimate", T::F32(e)),
            ("flags", T::u8()),
            ("reserved", T::u8()),
            ("time", btime(e)),
            ("detector", padded(24)),
        ],
    )
    .machinery(&["reserved"])
}

/// Blockette 201: the same, from a Murdock-Hutt detector, which records the
/// six signal-to-noise numbers it decided on and which of its pickers fired.
fn b201_body(e: Endian) -> T {
    T::structure(
        "MurdockEventDetection",
        vec![
            ("amplitude", T::F32(e)),
            ("period", T::F32(e)),
            ("background_estimate", T::F32(e)),
            ("flags", T::u8()),
            ("reserved", T::u8()),
            ("time", btime(e)),
            ("snr_values", T::array(T::u8(), E::lit(6))),
            ("loopback", T::u8()),
            ("pick_algorithm", T::u8()),
            ("detector", padded(24)),
        ],
    )
    .machinery(&["reserved"])
}

/// Blockette 300: a step calibration was injected. The station drives a known
/// signal into the channel so that what comes out can be compared with it.
fn b300_body(e: Endian) -> T {
    T::structure(
        "StepCalibration",
        vec![
            ("time", btime(e)),
            ("calibration_count", T::u8()),
            ("flags", T::u8()),
            // Both in ten-thousandths of a second.
            ("step_duration", T::u32(e)),
            ("interval_duration", T::u32(e)),
            ("amplitude", T::F32(e)),
            ("input_channel", padded(3)),
            ("reserved", T::u8()),
            ("reference_amplitude", T::u32(e)),
            ("coupling", padded(12)),
            ("rolloff", padded(12)),
        ],
    )
    .machinery(&["reserved"])
}

/// Blockette 310: a sine calibration, which states a period rather than a
/// step duration.
fn b310_body(e: Endian) -> T {
    T::structure(
        "SineCalibration",
        vec![
            ("time", btime(e)),
            ("reserved1", T::u8()),
            ("flags", T::u8()),
            ("duration", T::u32(e)),
            ("period", T::F32(e)),
            ("amplitude", T::F32(e)),
            ("input_channel", padded(3)),
            ("reserved2", T::u8()),
            ("reference_amplitude", T::u32(e)),
            ("coupling", padded(12)),
            ("rolloff", padded(12)),
        ],
    )
    .machinery(&["reserved1", "reserved2"])
}

/// Blockette 320: a pseudo-random calibration, which names the kind of noise
/// it drove in and gives its amplitude peak to peak.
fn b320_body(e: Endian) -> T {
    T::structure(
        "PseudoRandomCalibration",
        vec![
            ("time", btime(e)),
            ("reserved1", T::u8()),
            ("flags", T::u8()),
            ("duration", T::u32(e)),
            ("ptp_amplitude", T::F32(e)),
            ("input_channel", padded(3)),
            ("reserved2", T::u8()),
            ("reference_amplitude", T::u32(e)),
            ("coupling", padded(12)),
            ("rolloff", padded(12)),
            ("noise_type", padded(8)),
        ],
    )
    .machinery(&["reserved1", "reserved2"])
}

/// Blockette 390: a calibration of a kind none of the three above describes.
fn b390_body(e: Endian) -> T {
    T::structure(
        "GenericCalibration",
        vec![
            ("time", btime(e)),
            ("reserved1", T::u8()),
            ("flags", T::u8()),
            ("duration", T::u32(e)),
            ("amplitude", T::F32(e)),
            ("input_channel", padded(3)),
            ("reserved2", T::u8()),
        ],
    )
    .machinery(&["reserved1", "reserved2"])
}

/// Blockette 395: a calibration stopped before it finished, and this is when.
fn b395_body(e: Endian) -> T {
    T::structure("CalibrationAbort", vec![("time", btime(e)), ("reserved", T::bytes(E::lit(2)))])
        .machinery(&["reserved"])
}

/// Blockette 400: this record is a beam, formed by summing an array of
/// sensors, and this is where it was pointed.
fn b400_body(e: Endian) -> T {
    T::structure(
        "Beam",
        vec![
            ("azimuth", T::F32(e)),
            ("slowness", T::F32(e)),
            ("configuration", T::u16(e)),
            ("reserved", T::bytes(E::lit(2))),
        ],
    )
    .machinery(&["reserved"])
}

/// Blockette 405: the per-sensor delays the beam above was formed with. How
/// many there are is not written anywhere: the blockette runs to the next one,
/// and every two bytes of it is a delay.
fn b405_body(e: Endian, rest: E) -> T {
    T::structure("BeamDelay", vec![("delay_values", T::array(T::u16(e), rest.div(E::lit(2))))])
}

/// Blockette 500: what the clock was doing. The one blockette a seismologist
/// reads when two stations disagree about when something happened.
fn b500_body(e: Endian) -> T {
    T::structure(
        "Timing",
        vec![
            ("vco_correction", T::F32(e)),
            ("time", btime(e)),
            // The microseconds the ten-byte time has no room for, -50 to +49.
            ("microseconds", T::Int { bits: 8, endian: Big }),
            // 0 to 100, as the receiver rated its own fix.
            ("reception_quality", T::u8()),
            ("exception_count", T::u32(e)),
            ("exception_type", padded(16)),
            ("clock_model", padded(32)),
            ("clock_status", padded(128)),
        ],
    )
    .payload(&["exception_type", "clock_status"])
}

/// Blockette 2000: whatever the writer wanted to carry that SEED has no field
/// for. A run of `~`-terminated headers naming what it is, and then the bytes.
///
/// Both offsets are counted from the start of the blockette, so eleven bytes
/// of body and the four-byte header come off them here.
fn b2000_body(e: Endian) -> T {
    const HEAD: i128 = 15;
    let headers = E::field("data_offset").sub(E::lit(HEAD)).at_least(E::lit(0)).at_most(E::Remaining);
    let opaque = E::field("length").sub(E::field("data_offset")).at_least(E::lit(0)).at_most(E::Remaining);
    T::structure(
        "OpaqueData",
        vec![
            // The whole blockette, header included.
            ("length", T::u16(e)),
            ("data_offset", T::u16(e)),
            ("record_number", T::u32(e)),
            ("word_order", T::enumeration("WordOrder", T::u8(), WORD_ORDER)),
            ("flags", T::u8()),
            ("header_count", T::u8()),
            (
                "headers",
                T::sized(
                    headers,
                    T::repeat(T::text(StrLen::Terminated { end: b'~', or_end: true }, Encoding::Ascii), Until::End),
                ),
            ),
            ("opaque", T::bytes(opaque)),
        ],
    )
    .payload(&["headers"])
}

/// Blockette 1000, which is what makes a record readable on its own: how the
/// samples are packed, which way round their words are, and how long the
/// record is.
fn b1000_body() -> T {
    T::structure(
        "DataOnlySEED",
        vec![
            ("encoding", T::enumeration("Encoding", T::u8(), ENCODING)),
            ("word_order", T::enumeration("WordOrder", T::u8(), WORD_ORDER)),
            // A power of two: 9 is a 512-byte record, 12 is 4096.
            ("record_length", T::u8()),
            ("reserved", T::u8()),
        ],
    )
    .machinery(&["reserved"])
}

/// Blockette 1001, which a modern writer adds for the microseconds the fixed
/// header has no room for.
fn b1001_body() -> T {
    T::structure(
        "DataExtension",
        vec![
            // 0 to 100, as the clock rated itself.
            ("timing_quality", T::u8()),
            ("microseconds", T::Int { bits: 8, endian: Big }),
            ("reserved", T::u8()),
            // How many Steim frames the record holds, which the reader would
            // otherwise have to work out from the record length.
            ("frame_count", T::u8()),
        ],
    )
    .machinery(&["reserved"])
}

/// Blockette 100, which states the sample rate as a float for the rates the
/// factor and multiplier pair cannot express.
fn b100_body(e: Endian) -> T {
    T::structure(
        "SampleRate",
        vec![("sample_rate", T::F32(e)), ("flags", T::u8()), ("reserved", T::bytes(E::lit(3)))],
    )
    .machinery(&["reserved"])
}

/// Samples of a fixed width, as many as the header said and no more than the
/// record holds.
fn samples(elem: T, width: i128) -> T {
    let count = E::field("sample_count").at_most(E::Remaining.div(E::lit(width)));
    T::structure("MiniSEEDSamples", vec![("samples", T::array(elem, count)), ("padding", T::bytes(E::Remaining))])
}

/// Steim1 or Steim2 compressed differences, as the 64-byte frames they are
/// written in.
///
/// The first frame is the one that carries the integration constants: word 1
/// is the first sample of the record and word 2 the last, and every other
/// word in the record is a difference from its neighbour. A reader that has
/// those two and the differences has the samples; a reader that has only the
/// differences has nothing it can place.
fn steim(e: Endian, two: bool) -> T {
    let frames = T::array(frame(e, two, false), E::Remaining.div(E::lit(64)));
    T::structure(
        "SteimData",
        vec![
            // A frame is 64 bytes and nothing measures it, so the room has to
            // be asked for by hand: `if_room` would ask for one byte.
            ("frame0", T::present_if(E::lit(63).less_than(E::Remaining), frame(e, two, true))),
            ("frames", frames),
            ("slack", T::bytes(E::Remaining)),
        ],
    )
}

/// The names of the fifteen words after the code word, which the IR needs as
/// static text.
const WORDS: [&str; 15] =
    ["w1", "w2", "w3", "w4", "w5", "w6", "w7", "w8", "w9", "w10", "w11", "w12", "w13", "w14", "w15"];

/// One 64-byte frame: a word of sixteen 2-bit codes, and fifteen words the
/// codes describe. The code for the code word itself is always 0.
fn frame(e: Endian, two: bool, first: bool) -> T {
    let mut fields: Vec<(&str, T)> = vec![("nibbles", T::u32(e))];
    for (i, name) in WORDS.iter().enumerate() {
        let n = i as u32 + 1;
        // The first frame spends its first two words on the constants that
        // turn the differences back into samples, and codes both as 0.
        let ty = match (first, n) {
            (true, 1) => T::Int { bits: 32, endian: e },
            (true, 2) => T::Int { bits: 32, endian: e },
            _ => word(e, two, n),
        };
        let name = match (first, n) {
            (true, 1) => "x0",  // the first sample of the record
            (true, 2) => "xn",  // and the last, to check the sum against
            _ => name,
        };
        fields.push((name, ty));
    }
    T::structure("SteimFrame", fields).counted_as("frame")
}

/// One data word, typed by the two bits of the code word that describe it.
///
/// This is the whole reason the frames are worth showing: a word of a Steim2
/// record is four bytes that hold seven differences or one, and which it is
/// is written thirty bytes earlier. Reading the code and typing the word by
/// it is what makes that visible without decoding anything.
fn word(e: Endian, two: bool, n: u32) -> T {
    // The code for word `n` is bits 31-2n and 30-2n of the code word, with
    // the code for the code word itself in the top two bits.
    let high = E::field("nibbles").bit(31 - 2 * n);
    let low = E::field("nibbles").bit(30 - 2 * n);
    let code = high.mul(E::lit(2)).add(low);
    if e == Little {
        return little_word(code, two);
    }
    let packed = |name: &'static str, bits: u32, count: u32| {
        let mut fields: Vec<(&str, T)> = Vec::new();
        for i in 0..count {
            fields.push((DIFFS[i as usize], T::Int { bits, endian: Big }));
        }
        // What is left of the word once the differences have had their bits.
        let spare = 32 - bits * count;
        if spare > 0 {
            fields.push(("unused", T::UInt { bits: spare, endian: Big }));
        }
        T::inline_structure(name, fields)
    };
    // A second code inside the word, which is how Steim2 fits seven
    // differences into thirty bits when the signal is quiet enough.
    let sub = |name: &'static str, cases: Vec<(i128, u32, u32, &'static str)>| {
        let cases = cases
            .into_iter()
            .map(|(v, bits, count, shape)| {
                // What the differences leave of the thirty bits comes before
                // them, not after: the seven 4-bit case fills bits 27 to 0 and
                // leaves 29 and 28 alone. Putting the slack at the other end
                // reads every one of the seven off by four bits.
                let mut fields: Vec<(&str, T)> = Vec::new();
                let spare = 30 - bits * count;
                if spare > 0 {
                    fields.push(("unused", T::UInt { bits: spare, endian: Big }));
                }
                for i in 0..count {
                    fields.push((DIFFS[i as usize], T::Int { bits, endian: Big }));
                }
                (v, T::inline_structure(shape, fields))
            })
            .collect();
        T::inline_structure(
            name,
            vec![
                ("dnib", T::UInt { bits: 2, endian: Big }),
                ("differences", T::switch(E::field("dnib"), cases, T::UInt { bits: 30, endian: Big })),
            ],
        )
    };
    let cases = if two {
        vec![
            // Not data: the integration constants in frame 0, and nothing at
            // all in the words a record stops short of filling.
            (0, T::u32(Big)),
            (1, packed("Steim2x8", 8, 4)),
            (2, sub("Steim2Wide", vec![(1, 30, 1, "Steim2x30"), (2, 15, 2, "Steim2x15"), (3, 10, 3, "Steim2x10")])),
            (3, sub("Steim2Narrow", vec![(0, 6, 5, "Steim2x6"), (1, 5, 6, "Steim2x5"), (2, 4, 7, "Steim2x4")])),
        ]
    } else {
        vec![(0, T::u32(Big)), (1, packed("Steim1x8", 8, 4)), (2, packed("Steim1x16", 16, 2)), (3, packed("Steim1x32", 32, 1))]
    };
    T::switch(code, cases, T::u32(Big))
}

/// Names for the differences inside one word. Seven is as many as Steim2
/// packs into thirty bits.
const DIFFS: [&str; 7] = ["d0", "d1", "d2", "d3", "d4", "d5", "d6"];

/// The unsigned number in the `width` bits of `src` ending at bit `top`,
/// counting bits from the least significant.
///
/// The IR has a bit and a left shift and no right shift or mask, so the field
/// is added up a bit at a time. That is more expression than a shift and an
/// and would be, and it is the only way to name a bit field of a number rather
/// than of a run of bytes: a little-endian Steim word has to be read whole and
/// taken apart afterwards, because taking it apart in place would name the
/// bits of the byte-swapped word.
fn bits_of(src: &E, top: u32, width: u32) -> E {
    let mut total = E::lit(0);
    for j in 0..width {
        total = total.add(src.clone().bit(top - j).shl(E::lit((width - 1 - j) as i128)));
    }
    total
}

/// The same bits read as a signed number: the unsigned value less twice the
/// top bit's weight, which is two's complement written out.
fn signed_bits(src: &E, top: u32, width: u32) -> E {
    bits_of(src, top, width).sub(src.clone().bit(top).shl(E::lit(width as i128)))
}

/// One data word of a little-endian record.
///
/// A word that holds whole differences is not byte-swapped as a word at all:
/// four 1-byte differences are the four bytes where they lie, two 2-byte ones
/// are each swapped on their own, and a 4-byte one is the whole word swapped.
/// So all of Steim1 and half of Steim2 is a matter of naming the fields at
/// their own width and byte order, exactly as the big-endian side does.
///
/// The rest of Steim2 is not. A word that packs its differences into bit
/// fields is swapped whole and then cut up, and the IR slices a run of *bits*
/// in the order they are written, so cutting the word where it lies would name
/// the wrong bits. Those words are read whole and the fields worked out from
/// the value with shifts: the word keeps its four bytes and the differences
/// beside it take none.
fn little_word(code: E, two: bool) -> T {
    // Differences that are whole numbers of bytes, laid out where they are.
    let plain = |name: &'static str, bits: u32, count: u32| {
        let fields =
            (0..count).map(|i| (DIFFS[i as usize], T::Int { bits, endian: Little })).collect::<Vec<_>>();
        T::inline_structure(name, fields)
    };
    // The bit-packed ones, worked out from the swapped word's value. `from` is
    // the highest bit of the first difference, and what the differences leave
    // of the thirty bits sits above them.
    let w = E::field("word");
    let packed = |name: &'static str, bits: u32, count: u32| {
        let from = 29 - (30 - bits * count);
        let fields = (0..count)
            .map(|i| (DIFFS[i as usize], T::Computed(signed_bits(&w, from - i * bits, bits))))
            .collect();
        T::inline_structure(name, fields)
    };
    // Steim2's second code, in the top two bits of the word itself, which is
    // what decides how many differences the other thirty bits hold.
    let sub = |name: &'static str, cases: Vec<(i128, u32, u32, &'static str)>| {
        let cases = cases.into_iter().map(|(v, bits, count, shape)| (v, packed(shape, bits, count))).collect();
        T::inline_structure(
            name,
            vec![
                ("word", T::u32(Little)),
                ("dnib", T::Computed(bits_of(&w, 31, 2))),
                ("differences", T::switch(E::field("dnib"), cases, T::bytes(E::lit(0)))),
            ],
        )
    };
    let cases = if two {
        vec![
            (0, T::u32(Little)),
            (1, plain("Steim2x8", 8, 4)),
            (2, sub("Steim2Wide", vec![(1, 30, 1, "Steim2x30"), (2, 15, 2, "Steim2x15"), (3, 10, 3, "Steim2x10")])),
            (3, sub("Steim2Narrow", vec![(0, 6, 5, "Steim2x6"), (1, 5, 6, "Steim2x5"), (2, 4, 7, "Steim2x4")])),
        ]
    } else {
        vec![
            (0, T::u32(Little)),
            (1, plain("Steim1x8", 8, 4)),
            (2, plain("Steim1x16", 16, 2)),
            (3, plain("Steim1x32", 32, 1)),
        ]
    };
    T::switch(code, cases, T::u32(Little))
}

/// A miniSEED file, told by the shape of its first record. There is no magic:
/// what a record starts with is six characters of sequence number, which some
/// writers leave blank, and then a letter saying how far the data has been
/// checked.
///
/// The year and the day of the year are what carry the weight. A record whose
/// header reads as a plausible calendar date one way round and not the other
/// is a record, and a file of 512-byte blocks that happens to open with six
/// digits and a `D` is not.
pub fn is_mseed(head: &[u8], len: u64) -> bool {
    if len < FIXED as u64 || head.len() < FIXED as usize {
        return false;
    }
    // Six digits, or blanks: `reclen_1024_without_sequence_numbers.mseed` is
    // a real file from a real network.
    if !head[..6].iter().all(|b| b.is_ascii_digit() || *b == b' ') {
        return false;
    }
    if !matches!(head[6], b'D' | b'R' | b'Q' | b'M') {
        return false;
    }
    // The byte after the quality indicator is reserved and blank in every
    // writer's output.
    if head[7] != b' ' && head[7] != 0 {
        return false;
    }
    let year = |big: bool| {
        let b: [u8; 2] = head[YEAR_AT..YEAR_AT + 2].try_into().unwrap();
        if big { u16::from_be_bytes(b) } else { u16::from_le_bytes(b) }
    };
    let day = |big: bool| {
        let b: [u8; 2] = head[YEAR_AT + 2..YEAR_AT + 4].try_into().unwrap();
        if big { u16::from_be_bytes(b) } else { u16::from_le_bytes(b) }
    };
    [true, false].into_iter().any(|big| {
        (1800..=2100).contains(&year(big)) && (1..=366).contains(&day(big))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// Field indices in a record, which the tests read by name rather than by
    /// counting fields at the assertion.
    const STATION: usize = 3;
    const SAMPLE_COUNT: usize = 8;
    const BLOCKETTES: usize = 18;
    const DATA: usize = 20;

    /// A record: the fixed header, blockette 1000, and whatever data.
    fn record_bytes(big: bool, encoding: u8, exponent: u8, samples: &[u8]) -> Vec<u8> {
        let u16b = |v: u16| if big { v.to_be_bytes() } else { v.to_le_bytes() };
        let mut v = Vec::new();
        v.extend_from_slice(b"000001D ");
        v.extend_from_slice(b"BGLD ");
        v.extend_from_slice(b"  ");
        v.extend_from_slice(b"EHE");
        v.extend_from_slice(b"BW");
        v.extend_from_slice(&u16b(2008)); // year
        v.extend_from_slice(&u16b(1)); // day of the year
        v.extend_from_slice(&[0, 0, 0, 0]); // hour, minute, second, unused
        v.extend_from_slice(&u16b(0)); // fraction
        v.extend_from_slice(&u16b(7)); // sample count
        v.extend_from_slice(&u16b(200)); // rate factor
        v.extend_from_slice(&u16b(1)); // rate multiplier
        v.extend_from_slice(&[0, 0, 0, 1]); // three flag bytes and one blockette
        v.extend_from_slice(&[0, 0, 0, 0]); // time correction
        v.extend_from_slice(&u16b(64)); // data starts at 64
        v.extend_from_slice(&u16b(48)); // first blockette at 48
        assert_eq!(v.len(), FIXED as usize);
        v.extend_from_slice(&u16b(1000)); // blockette 1000
        v.extend_from_slice(&u16b(0)); // last in the chain
        v.extend_from_slice(&[encoding, u8::from(big), exponent, 0]);
        v.resize(64, 0); // the padding a data offset of 64 leaves
        v.extend_from_slice(samples);
        v.resize(1 << exponent, 0);
        v
    }

    #[test]
    fn a_record_is_read_either_way_round_and_sized_by_blockette_1000() {
        for big in [true, false] {
            let mut file = record_bytes(big, 3, 9, &[]);
            file.extend_from_slice(&record_bytes(big, 3, 9, &[]));
            let d = Document::new(MemSource(file));
            let mut ev = Evaluator::new(mseed());
            assert_eq!(ev.node(&d, &[0]).unwrap().child_count, 2, "two records, big={big}");
            let record = ev.node(&d, &[0, 1]).unwrap();
            assert_eq!(record.offset_bits, 512 * 8);
            assert_eq!(record.size_bits, 512 * 8);
            assert_eq!(ev.node(&d, &[0, 0, STATION]).unwrap().value, Value::Str("BGLD".into()));
            assert_eq!(ev.node(&d, &[0, 0, SAMPLE_COUNT]).unwrap().value.as_int(), Some(7));
            assert_eq!(ev.node(&d, &[0, 0, BLOCKETTES]).unwrap().child_count, 1);
        }
    }

    /// 32-bit integer samples are the plain case: as many as the header said.
    #[test]
    fn the_samples_are_typed_by_what_blockette_1000_says() {
        let mut data = Vec::new();
        for i in 0..7i32 {
            data.extend_from_slice(&i.to_be_bytes());
        }
        let d = Document::new(MemSource(record_bytes(true, 3, 9, &data)));
        let mut ev = Evaluator::new(mseed());
        assert_eq!(ev.node(&d, &[0, 0, DATA, 0]).unwrap().child_count, 7);
        assert_eq!(ev.node(&d, &[0, 0, DATA, 0, 3]).unwrap().value.as_int(), Some(3));
    }

    /// A Steim1 frame: the code word says word 1 and 2 are constants, word 3
    /// holds four 8-bit differences and word 4 two 16-bit ones.
    #[test]
    fn a_steim_word_is_typed_by_its_code() {
        // The code for word n sits in bits 31-2n and 30-2n, so word 3's is
        // bits 25 and 24 and word 4's is bits 23 and 22.
        let nibbles: u32 = (1 << 24) | (2 << 22);
        let mut data = nibbles.to_be_bytes().to_vec();
        data.extend_from_slice(&100i32.to_be_bytes()); // x0
        data.extend_from_slice(&107i32.to_be_bytes()); // xn
        data.extend_from_slice(&[1, 2, 3, 4]); // four 8-bit differences
        data.extend_from_slice(&[0, 5, 0, 6]); // two 16-bit ones
        let d = Document::new(MemSource(record_bytes(true, 10, 9, &data)));
        let mut ev = Evaluator::new(mseed());
        let frame0 = [0, 0, DATA, 0];
        // x0 and xn are the second and third fields of the first frame.
        assert_eq!(ev.node(&d, &[0, 0, DATA, 0, 1]).unwrap().value.as_int(), Some(100));
        assert_eq!(ev.node(&d, &[0, 0, DATA, 0, 2]).unwrap().value.as_int(), Some(107));
        assert_eq!(ev.node(&d, &frame0).unwrap().size_bits, 64 * 8);
        // Word 3 is four differences, word 4 is two.
        assert_eq!(ev.node(&d, &[0, 0, DATA, 0, 3]).unwrap().child_count, 4);
        assert_eq!(ev.node(&d, &[0, 0, DATA, 0, 3, 2]).unwrap().value.as_int(), Some(3));
        assert_eq!(ev.node(&d, &[0, 0, DATA, 0, 4]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[0, 0, DATA, 0, 4, 1]).unwrap().value.as_int(), Some(6));
    }

    /// A file that stops part way through its last record reads what is
    /// there rather than refusing the whole file. The second record says it
    /// is 512 bytes and only 188 of them arrived; every field that did arrive
    /// still reads, samples included.
    #[test]
    fn a_record_cut_off_short_reads_as_far_as_it_goes() {
        let mut data = Vec::new();
        for i in 0..7i32 {
            data.extend_from_slice(&i.to_be_bytes());
        }
        let mut file = record_bytes(true, 3, 9, &data);
        file.extend_from_slice(&record_bytes(true, 3, 9, &data));
        file.truncate(700); // one whole record and 188 bytes of the next
        let d = Document::new(MemSource(file));
        let mut ev = Evaluator::new(mseed());
        assert_eq!(ev.node(&d, &[0]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[0, 0]).unwrap().size_bits, 512 * 8);
        assert_eq!(ev.node(&d, &[0, 1]).unwrap().size_bits, 188 * 8);
        assert_eq!(ev.node(&d, &[0, 1, DATA, 0]).unwrap().child_count, 7);
    }

    /// The same frame as the test above, written the other way round.
    ///
    /// A Steim1 word is never swapped as a word: four 1-byte differences are
    /// the four bytes where they lie, and two 2-byte ones are each swapped on
    /// their own. So the bytes of word 3 are the same in both files, and only
    /// the pairs in word 4 turn round.
    #[test]
    fn a_little_endian_steim1_word_swaps_its_differences_and_not_the_word() {
        let nibbles: u32 = (1 << 24) | (2 << 22);
        let mut data = nibbles.to_le_bytes().to_vec();
        data.extend_from_slice(&100i32.to_le_bytes()); // x0
        data.extend_from_slice(&107i32.to_le_bytes()); // xn
        // Four 8-bit differences, in the order they are written: 1, 2, -3, 4.
        data.extend_from_slice(&[1, 2, 0xfd, 4]);
        // Two 16-bit ones, each little-endian: 5 and -6.
        data.extend_from_slice(&5i16.to_le_bytes());
        data.extend_from_slice(&(-6i16).to_le_bytes());
        let d = Document::new(MemSource(record_bytes(false, 10, 9, &data)));
        let mut ev = Evaluator::new(mseed());
        assert_eq!(ev.node(&d, &[0, 0, DATA, 0, 1]).unwrap().value.as_int(), Some(100));
        let w3 = ev.node(&d, &[0, 0, DATA, 0, 3]).unwrap();
        assert_eq!((w3.child_count, w3.size_bits), (4, 32));
        for (i, want) in [1i128, 2, -3, 4].into_iter().enumerate() {
            assert_eq!(ev.node(&d, &[0, 0, DATA, 0, 3, i]).unwrap().value.as_int(), Some(want), "d{i}");
        }
        let w4 = ev.node(&d, &[0, 0, DATA, 0, 4]).unwrap();
        assert_eq!(w4.child_count, 2);
        assert_eq!(ev.node(&d, &[0, 0, DATA, 0, 4, 0]).unwrap().value.as_int(), Some(5));
        assert_eq!(ev.node(&d, &[0, 0, DATA, 0, 4, 1]).unwrap().value.as_int(), Some(-6));
    }

    /// Steim2's second code: a word whose top two bits say how the other
    /// thirty are divided. Little-endian, so both codes are read from values.
    #[test]
    fn a_little_endian_steim2_word_reads_its_second_code() {
        // Words 1 and 2 of frame 0 are the constants, so the word under test
        // is word 3, whose code sits in bits 25 and 24. Code 3 sends it to the
        // narrow shapes.
        let nibbles: u32 = 3 << 24;
        let mut data = nibbles.to_le_bytes().to_vec();
        data.extend_from_slice(&100i32.to_le_bytes()); // x0
        data.extend_from_slice(&107i32.to_le_bytes()); // xn
        // dnib 2: seven 4-bit differences, 1 through 7 with the third
        // negative. They fill bits 27 to 0, leaving 29 and 28 unused.
        let mut word: u32 = 2 << 30;
        for (i, v) in [1u32, 2, 13, 4, 5, 6, 7].into_iter().enumerate() {
            word |= v << (24 - 4 * i as u32);
        }
        data.extend_from_slice(&word.to_le_bytes());
        let d = Document::new(MemSource(record_bytes(false, 11, 9, &data)));
        let mut ev = Evaluator::new(mseed());
        let w3 = ev.node(&d, &[0, 0, DATA, 0, 3]).unwrap();
        assert_eq!(w3.child_count, 3); // word, dnib, differences
        assert_eq!(ev.node(&d, &[0, 0, DATA, 0, 3, 0]).unwrap().size_bits, 32);
        assert_eq!(ev.node(&d, &[0, 0, DATA, 0, 3, 1]).unwrap().value.as_int(), Some(2));
        let diffs = ev.node(&d, &[0, 0, DATA, 0, 3, 2]).unwrap();
        assert_eq!(diffs.type_name, "Steim2x4");
        assert_eq!(diffs.child_count, 7);
        for (i, want) in [1i128, 2, -3, 4, 5, 6, 7].into_iter().enumerate() {
            assert_eq!(ev.node(&d, &[0, 0, DATA, 0, 3, 2, i]).unwrap().value.as_int(), Some(want), "d{i}");
        }
    }

    /// A blockette chain of three: the one that sizes the record, a timing
    /// blockette, and a calibration. Each is read from its own type, and the
    /// chain places them by the forward pointers rather than by their widths.
    #[test]
    fn the_blockettes_after_1000_are_read_by_type() {
        let mut extra = Vec::new();
        // Blockette 500, at offset 56, pointing at 320 at offset 256.
        extra.extend_from_slice(&500u16.to_be_bytes());
        extra.extend_from_slice(&256u16.to_be_bytes());
        extra.extend_from_slice(&0.25f32.to_be_bytes()); // vco_correction
        extra.extend_from_slice(&2008u16.to_be_bytes());
        extra.extend_from_slice(&1u16.to_be_bytes());
        extra.extend_from_slice(&[0, 0, 0, 0]);
        extra.extend_from_slice(&0u16.to_be_bytes());
        extra.push(0xf6); // microseconds, -10
        extra.push(90); // reception quality
        extra.extend_from_slice(&3u32.to_be_bytes()); // exception count
        let pad = |s: &str, n: usize| {
            let mut v = s.as_bytes().to_vec();
            v.resize(n, b' ');
            v
        };
        extra.extend(pad("VCO CORRECTION", 16));
        extra.extend(pad("GPS", 32));
        extra.extend(pad("LOCKED", 128));

        let mut record = record_bytes(true, 3, 9, &[]);
        record[46..48].copy_from_slice(&48u16.to_be_bytes()); // first blockette
        record[39] = 3; // three blockettes in the chain
        record[50..52].copy_from_slice(&56u16.to_be_bytes()); // 1000 points at 500
        record[44..46].copy_from_slice(&0u16.to_be_bytes()); // no samples in this one
        record[56..56 + extra.len()].copy_from_slice(&extra);
        // Blockette 320 at 256, last in the chain.
        let mut cal = Vec::new();
        cal.extend_from_slice(&320u16.to_be_bytes());
        cal.extend_from_slice(&0u16.to_be_bytes());
        cal.extend_from_slice(&2008u16.to_be_bytes());
        cal.extend_from_slice(&1u16.to_be_bytes());
        cal.extend_from_slice(&[0, 0, 0, 0]);
        cal.extend_from_slice(&0u16.to_be_bytes());
        cal.push(0); // reserved1
        cal.push(0x04); // flags
        cal.extend_from_slice(&600u32.to_be_bytes()); // duration
        cal.extend_from_slice(&1.5f32.to_be_bytes()); // ptp amplitude
        cal.extend(pad("BHZ", 3));
        cal.push(0);
        cal.extend_from_slice(&7u32.to_be_bytes());
        cal.extend(pad("RESISTIVE", 12));
        cal.extend(pad("3DB@10HZ", 12));
        cal.extend(pad("TELEDYNE", 8));
        record[256..256 + cal.len()].copy_from_slice(&cal);

        let d = Document::new(MemSource(record));
        let mut ev = Evaluator::new(mseed());
        assert_eq!(ev.node(&d, &[0, 0, BLOCKETTES]).unwrap().child_count, 3);
        let timing = ev.node(&d, &[0, 0, BLOCKETTES, 1, 2]).unwrap();
        assert_eq!(timing.type_name, "Timing");
        assert_eq!(ev.node(&d, &[0, 0, BLOCKETTES, 1, 2, 0]).unwrap().value, Value::Float(0.25));
        assert_eq!(ev.node(&d, &[0, 0, BLOCKETTES, 1, 2, 2]).unwrap().value.as_int(), Some(-10));
        assert_eq!(ev.node(&d, &[0, 0, BLOCKETTES, 1, 2, 6]).unwrap().value, Value::Str("GPS".into()));
        assert_eq!(ev.node(&d, &[0, 0, BLOCKETTES, 1, 2, 7]).unwrap().value, Value::Str("LOCKED".into()));
        let cal = ev.node(&d, &[0, 0, BLOCKETTES, 2, 2]).unwrap();
        assert_eq!(cal.type_name, "PseudoRandomCalibration");
        assert_eq!(ev.node(&d, &[0, 0, BLOCKETTES, 2, 2, 3]).unwrap().value.as_int(), Some(600));
        assert_eq!(ev.node(&d, &[0, 0, BLOCKETTES, 2, 2, 10]).unwrap().value, Value::Str("TELEDYNE".into()));
    }

    /// Blockette 2000 is the one that measures itself: its own length and
    /// where its opaque bytes start, both counted from its beginning.
    #[test]
    fn an_opaque_blockette_splits_its_headers_from_its_payload() {
        let mut b = Vec::new();
        b.extend_from_slice(&2000u16.to_be_bytes());
        b.extend_from_slice(&0u16.to_be_bytes()); // last in the chain
        b.extend_from_slice(&37u16.to_be_bytes()); // length, header included
        b.extend_from_slice(&30u16.to_be_bytes()); // where the payload starts
        b.extend_from_slice(&1u32.to_be_bytes()); // record number
        b.push(1); // big-endian
        b.push(0); // flags
        b.push(2); // two headers
        b.extend_from_slice(b"Format~Version~"); // 15 bytes, to offset 30
        b.extend_from_slice(b"opaque!"); // 7 bytes, to length 37

        let mut record = record_bytes(true, 3, 9, &[]);
        record[46..48].copy_from_slice(&48u16.to_be_bytes());
        record[39] = 2;
        record[50..52].copy_from_slice(&56u16.to_be_bytes());
        record[44..46].copy_from_slice(&0u16.to_be_bytes());
        record[56..56 + b.len()].copy_from_slice(&b);

        let d = Document::new(MemSource(record));
        let mut ev = Evaluator::new(mseed());
        let body = ev.node(&d, &[0, 0, BLOCKETTES, 1, 2]).unwrap();
        assert_eq!(body.type_name, "OpaqueData");
        let headers = ev.node(&d, &[0, 0, BLOCKETTES, 1, 2, 6]).unwrap();
        assert_eq!((headers.size_bits, headers.child_count), (15 * 8, 2));
        assert_eq!(ev.node(&d, &[0, 0, BLOCKETTES, 1, 2, 6, 0]).unwrap().value, Value::Str("Format".into()));
        let opaque = ev.node(&d, &[0, 0, BLOCKETTES, 1, 2, 7]).unwrap();
        assert_eq!((opaque.type_name.as_str(), opaque.size_bits), ("bytes[]", 7 * 8));
    }

    /// The narrower and wider fixed-width encodings: 24-bit integers, and the
    /// gain-ranged words that are two bytes each and nothing more here.
    #[test]
    fn the_other_fixed_width_encodings_are_typed_by_their_sample_width() {
        let mut data = Vec::new();
        for i in [1i32, -2, 3, -4, 5, -6, 7] {
            data.extend_from_slice(&i.to_be_bytes()[1..]);
        }
        let d = Document::new(MemSource(record_bytes(true, 2, 9, &data)));
        let mut ev = Evaluator::new(mseed());
        assert_eq!(ev.node(&d, &[0, 0, DATA, 0]).unwrap().child_count, 7);
        assert_eq!(ev.node(&d, &[0, 0, DATA, 0, 1]).unwrap().value.as_int(), Some(-2));
        assert_eq!(ev.node(&d, &[0, 0, DATA, 0, 6]).unwrap().value.as_int(), Some(7));

        // CDSN, 16-bit gain-ranged: seven words and no claim about their bits.
        let d = Document::new(MemSource(record_bytes(true, 16, 9, &[0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0, 7])));
        let mut ev = Evaluator::new(mseed());
        let s = ev.node(&d, &[0, 0, DATA, 0]).unwrap();
        assert_eq!((s.type_name.as_str(), s.child_count), ("u16 be[]", 7));
        assert_eq!(ev.node(&d, &[0, 0, DATA, 0, 3]).unwrap().value, Value::UInt(4));
    }

    #[test]
    fn recognised_by_its_sequence_number_and_a_plausible_date() {
        let r = record_bytes(true, 10, 9, &[]);
        assert!(is_mseed(&r, r.len() as u64));
        let r = record_bytes(false, 10, 9, &[]);
        assert!(is_mseed(&r, r.len() as u64));
        // Six digits and a D, and a date that is nobody's date.
        let mut not = r.clone();
        not[YEAR_AT..YEAR_AT + 4].copy_from_slice(&[0xff; 4]);
        assert!(!is_mseed(&not, not.len() as u64));
        assert!(!is_mseed(&[0u8; 512], 512));
    }
}
