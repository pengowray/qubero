//! SEG-Y: seismic reflection data, the format the oil and gas industry and the
//! academic surveys that borrow its ships have written since the Society of
//! Exploration Geophysicists fixed it in 1975. A survey's traces arrive in one.
//!
//! A file opens with 3600 bytes of file header and then the traces. The first
//! 3200 bytes are text: forty lines of eighty columns, `C 1` to `C40`, card
//! images from when a header was a deck of punched cards. The 1975 standard
//! made them EBCDIC and revision 1 allowed ASCII as well. What says which is
//! the first byte, which is `C` either way: 0xC3 in EBCDIC and 0x43 in ASCII.
//! segyio tells them apart the same way, and there is nothing else to go on.
//!
//! The next 400 bytes are binary and hold what the traces need: the sample
//! interval, how many samples a trace has and how each is written. Revision 1
//! added a flag saying whether every trace is the same length and a count of
//! extended textual headers, 3200-byte blocks of stanzas between the binary
//! header and the first trace. Those two are honoured whatever revision the
//! file claims, because segyio writes both into files it labels revision 0.
//! Revision 2 used bytes that had been unassigned for 32-bit versions of
//! counts that outgrew sixteen bits, an integer saying which way round the
//! file is, the offset of the first trace, and the number of 240-byte headers
//! each trace carries beyond its own. Those are read only when the revision
//! byte says 2 or later: in an older file the same bytes are unassigned, and a
//! writer was free to leave anything there.
//!
//! The revision is two bytes, major and minor, and revision 2 says so. Revision
//! 1 called the same two bytes one 16-bit number, so a little-endian writer
//! that swapped every number swapped this one too, and its revision 1.0 reads
//! here as 0.1. segyio 1.9 reads a little-endian file's revision that way
//! round, and segyio's own C library reads the two bytes as they lie, as this
//! does.
//!
//! *Which way round the numbers are.* SEG-Y is big-endian, and revision 1 said
//! a little-endian file was not SEG-Y at all. Plenty are written anyway.
//! Revision 2 put 0x01020304 at byte 3297 so that a reader could tell, and
//! when that word reads either way round it settles it. It is zero in most
//! files, including every little-endian sample here, so what settles it then
//! is the sample format code: a number from 1 to 16 read one way is 256 or
//! more read the other. The template peeks at both and lays the whole file out
//! in whichever answers, as SAC does with its header version.
//!
//! *How long a trace is.* Each trace is a 240-byte header, the extension
//! headers revision 2 allows, and then its samples, and nothing marks where
//! one ends. How many samples there are is in the trace's own header; how wide
//! each one is is in the binary header. When the fixed length flag is set the
//! binary header's count is the only one that matters, every trace is the same
//! size, and the traces are placed by arithmetic, so the millionth trace of a
//! survey is found without reading the 999,999 before it. Without the flag
//! each trace is measured from its own header, which means walking to reach
//! one. A trace header whose count is zero, as segyio's own test files write,
//! takes the binary header's, which is what every reader does.
//!
//! *What a sample is.* Format code 1 is the IBM System/360 hexadecimal float,
//! which is what seismic processing wrote when its computers were IBM
//! mainframes: see [`Ty::IbmF32`](crate::template::Ty::IbmF32). Codes 2, 3 and 8 are two's
//! complement integers, 5 and 6 IEEE floats, and 7, 9 to 12, 15 and 16 are the
//! widths revision 2 added. Code 4, fixed point with a gain byte, has been
//! obsolete since 2002 and its samples stay four bytes each.
//!
//! What is not read: an extended textual header count of -1 with no first
//! trace offset beside it ends at the block that opens `((SEG: EndText))` or
//! `((EndText))` spelled exactly so, in capitals, in either encoding. The
//! standard lets that stanza name be written in any case with spaces anywhere,
//! and a spelling other than those two reads on to the end of the file. The
//! count of extension headers is the binary header's for every trace: a trace
//! may say in its first extension that it has fewer, and that is not followed.
//! The scalars beside the coordinates, elevations and times are read and not
//! applied, as GRIB's are, since a scalar of -100 is a division and the answer
//! is not an integer. The byte positions revision 2 added were checked against
//! segyio, whose test files are the samples.
//!
//! Seismic Unix's `.su` files are these traces with no file header at all, in
//! the byte order of whatever wrote them and always as IEEE floats. They are
//! not read here. Nothing in one says what it is, so it could not be
//! recognised, and its trace header is not this one past byte 180: Seismic
//! Unix keeps its own sampling and plotting fields where revision 1 put the
//! CDP position and the in-line and cross-line numbers.

use crate::template::{Encoding, Endian, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};
use crate::text::{encode_settled, CodePage, Settled};

mod tables;
use tables::{AMPLITUDE_RECOVERY, COORDINATE_UNITS, DATA_USE, FIXED_LENGTH, FORMAT, GAIN_TYPE, IMPULSE_POLARITY, MEASUREMENT_SYSTEM, NO_YES, OVER_TRAVEL, SORTING, SWEEP_TYPE, TAPER_TYPE, TIME_BASIS, TRACE_ID, UNIT, VIBRATORY_POLARITY, YES_NO};

/// The textual header, and each extended textual header and data trailer
/// block after it.
const TEXT: i128 = 3200;

/// Where the traces start in a file with no extended textual headers: the
/// textual header and the 400-byte binary header.
pub const FILE_HEADER: usize = 3600;

/// A trace header, and each extension of one.
const TRACE_HEADER: i128 = 240;

/// Where the sample interval is written, in microseconds.
pub const INTERVAL_AT: usize = 3216;

/// Where the data sample format code is written. Reading it both ways round is
/// what tells a big-endian file from a little-endian one when the byte order
/// integer is not set.
pub const FORMAT_AT: usize = 3224;

/// Where revision 2 writes 0x01020304 in the file's own byte order.
const BYTE_ORDER_AT: i128 = 3296;

/// Where the major revision number is written. One byte, so it has no order.
const MAJOR_AT: i128 = 3500;

/// How many bytes a sample takes, by format code. The ones missing are codes
/// nobody has defined, and a file using one has traces nothing can measure.
const WIDTH: &[(i128, i128)] =
    &[(1, 4), (2, 4), (3, 2), (4, 4), (5, 4), (6, 8), (7, 3), (8, 1), (9, 8), (10, 4), (11, 2), (12, 8), (15, 3), (16, 1)];

pub fn segy() -> Template {
    // Too short to hold the file header is too short to ask anything of: the
    // byte order and the encoding are both peeks past the first 3200 bytes.
    let short = T::structure("SEGY", vec![("bytes", T::bytes(E::Remaining))]);
    let whole = T::switch(big_endian(), vec![(0, file(Little))], file(Big));
    Template::new("segy", T::switch(E::Remaining.less_than(E::lit(FILE_HEADER as i128)), vec![(1, short)], whole))
}

/// One when the file is big-endian, zero when it is little-endian.
///
/// The byte order integer first, since when it is set it is the file saying so
/// outright, and then the format code, which is a small number read the right
/// way round and a multiple of 256 read the wrong way. A file where neither
/// answers is read big-endian, which is what the standard says it is.
fn big_endian() -> E {
    let order = E::peek_at(E::lit(BYTE_ORDER_AT * 8), 32, Big);
    let format_fits = |e| {
        let code = E::peek_at(E::lit(FORMAT_AT as i128 * 8), 16, e);
        E::lit(0).less_than(code.clone()).both(code.less_or_equal(E::lit(16)))
    };
    E::cond(
        order.clone().equal_to(E::lit(0x0403_0201)),
        E::lit(0),
        E::cond(
            order.equal_to(E::lit(0x0102_0304)),
            E::lit(1),
            E::cond(format_fits(Big), E::lit(1), E::cond(format_fits(Little), E::lit(0), E::lit(1))),
        ),
    )
}

/// A field of the binary header, by its path from there.
fn binary(path: &[&str]) -> E {
    let mut whole = vec!["binary_header"];
    whole.extend_from_slice(path);
    E::within(&whole)
}

/// One when the binary header says revision 2 or later.
fn revision_2() -> E {
    E::lit(1).less_than(binary(&["major_revision"]))
}

/// A field revision 2 added, or zero in a file older than that, where the
/// field is not there to ask.
fn rev2_field(path: &[&str]) -> E {
    E::cond(revision_2(), binary(path), E::lit(0))
}

/// The whole file, for one byte order.
///
/// The fields of no bits between the headers and the traces are what the
/// traces are measured with, copied out of the binary header so that a trace's
/// size names a field beside the list rather than a path into another
/// structure. That is what lets a run of fixed-length traces be counted by
/// division.
fn file(e: Endian) -> T {
    let ebcdic = E::peek_at(E::lit(0), 8, Big).equal_to(E::lit(0xc3));
    // A revision 2 file may say outright where its first trace is, and when it
    // does, whatever sits between the last header and there belongs to
    // neither.
    let offset = rev2_field(&["rev2_layout", "first_trace_offset"]);
    let gap = E::cond(E::lit(0).less_than(offset.clone()), offset.sub(E::Pos).at_least(E::lit(0)), E::lit(0))
        .at_most(E::Remaining);
    let width = WIDTH.iter().rev().fold(E::lit(0), |rest, (code, bytes)| {
        E::cond(E::field("format").equal_to(E::lit(*code)), E::lit(*bytes), rest)
    });
    // Revision 2's 32-bit count replaces the 16-bit one when it is set.
    let extended = rev2_field(&["rev2_counts", "extended_samples_per_trace"]);
    let samples = E::cond(
        E::lit(0).less_than(extended.clone()),
        extended,
        binary(&["samples_per_trace"]),
    );
    T::structure(
        "SEGY",
        vec![
            ("ebcdic", T::computed(ebcdic)),
            ("textual_header", textual_header()),
            ("binary_header", binary_header(e)),
            ("extended_textual_headers", extended_textual_headers()),
            ("to_first_trace", T::bytes(gap)),
            ("format", T::computed(binary(&["format"]))),
            ("sample_bytes", T::computed(width)),
            ("samples_per_trace", T::computed(samples)),
            ("additional_trace_headers", T::computed(additional_trace_headers())),
            (
                "trace_bytes",
                T::computed(
                    E::field("additional_trace_headers")
                        .add(E::lit(1))
                        .mul(E::lit(TRACE_HEADER))
                        .add(E::field("samples_per_trace").mul(E::field("sample_bytes"))),
                ),
            ),
            ("traces", traces(e)),
            ("data_trailer", data_trailer()),
        ],
    )
    .machinery(&["ebcdic", "to_first_trace", "format", "sample_bytes", "samples_per_trace", "additional_trace_headers", "trace_bytes"])
}

/// How many 240-byte headers each trace has after its own, which only
/// revision 2 has any of.
///
/// Revision 2.0 gives the count four bytes at 3507 and revision 2.1 gives it
/// two, spending the other two on a survey type. segyio reads two bytes in
/// either revision and writes a 2.0 file that way too, so a 2.0 count that
/// reads as 65536 or more is a two-byte count followed by two zero bytes:
/// nobody puts sixty thousand 240-byte headers in front of every trace.
fn additional_trace_headers() -> E {
    let count = binary(&["rev2_layout", "max_additional_trace_headers"]);
    let wide = E::lit(65535).less_than(count.clone());
    E::cond(revision_2(), E::cond(wide, count.clone().shr(E::lit(16)), count), E::lit(0))
}

/// The 3200-byte textual header: forty card images of eighty columns.
///
/// Every card is read whole, spaces and all. A card is not a name with
/// padding after it: it is a line of text with spaces in the middle of it.
fn textual_header() -> T {
    let cards = |enc| T::array(T::text(StrLen::Fixed(E::lit(80)), enc), E::lit(40));
    T::switch(E::field("ebcdic"), vec![(1, cards(Encoding::Ebcdic))], cards(Encoding::Ascii))
}

/// A 3200-byte block of stanzas: an extended textual header, or a block of
/// the data trailer. Stanzas are not cards, and nothing obliges one to keep to
/// eighty columns, so a block is read as the one run of text it is.
///
/// Each block says its own encoding, since a file with an EBCDIC textual
/// header can still write its stanzas in ASCII, and segyio's own test files
/// do. A stanza opens with `((`, which is 0x4D in EBCDIC and 0x28 in ASCII,
/// and a block of cards opens with `C`; anything else takes the textual
/// header's encoding. `with_end` adds whether this is the block that ends the
/// run, for a count that does not say.
fn text_block(name: &str, with_end: bool) -> T {
    let first = E::peek_at(E::lit(0), 8, Big);
    let is = |b: i128| first.clone().equal_to(E::lit(b));
    let ebcdic = E::lit(0).less_than(E::Remaining).both(E::cond(
        is(0x4d).either(is(0xc3)),
        E::lit(1),
        E::cond(is(0x28).either(is(0x43)), E::lit(0), E::field("ebcdic")),
    ));
    let text = |enc| T::text(StrLen::Fixed(E::lit(TEXT)), enc);
    let mut fields = Vec::new();
    if with_end {
        fields.push(("end_text", T::computed(is_end_text())));
    }
    fields.push(("text", T::switch(ebcdic, vec![(1, text(Encoding::Ebcdic))], text(Encoding::Ascii))));
    T::structure_named(name, "", "text", fields).machinery(&["end_text"])
}

/// A list of `count` blocks of stanzas.
fn text_blocks(name: &str, count: E) -> T {
    T::array(text_block(name, false), count)
}

/// The binary header, 400 bytes.
fn binary_header(e: Endian) -> T {
    let i16 = || T::Int { bits: 16, endian: e };
    let major = |from: i128| E::peek_at(E::lit((MAJOR_AT - from) * 8), 8, Big);
    T::structure(
        "SegyBinaryHeader",
        vec![
            ("job_id", T::i32(e)),
            // For 3-D poststack data, usually the in-line number.
            ("line_number", T::i32(e)),
            ("reel_number", T::i32(e)),
            ("traces_per_ensemble", i16()),
            ("auxiliary_traces_per_ensemble", i16()),
            // Microseconds for time data, hertz for frequency data, metres or
            // feet for depth data.
            ("sample_interval", i16()),
            ("original_sample_interval", i16()),
            // Unsigned: a trace of 60,000 samples is one segyio tests with.
            ("samples_per_trace", T::u16(e)),
            ("original_samples_per_trace", T::u16(e)),
            ("format", T::enumeration("SegySampleFormat", i16(), FORMAT)),
            ("ensemble_fold", i16()),
            ("trace_sorting", T::enumeration("SegyTraceSorting", i16(), SORTING)),
            ("vertically_summed_traces", i16()),
            ("sweep_start_frequency", i16()),
            ("sweep_end_frequency", i16()),
            // In milliseconds, as the two taper lengths are.
            ("sweep_length", i16()),
            ("sweep_type", T::enumeration("SegySweepType", i16(), SWEEP_TYPE)),
            ("sweep_channel", i16()),
            ("sweep_start_taper_length", i16()),
            ("sweep_end_taper_length", i16()),
            ("taper_type", T::enumeration("SegyTaperType", i16(), TAPER_TYPE)),
            ("correlated", T::enumeration("SegyCorrelated", i16(), NO_YES)),
            ("binary_gain_recovered", T::enumeration("SegyBinaryGainRecovered", i16(), YES_NO)),
            ("amplitude_recovery", T::enumeration("SegyAmplitudeRecovery", i16(), AMPLITUDE_RECOVERY)),
            ("measurement_system", T::enumeration("SegyMeasurementSystem", i16(), MEASUREMENT_SYSTEM)),
            ("impulse_polarity", T::enumeration("SegyImpulsePolarity", i16(), IMPULSE_POLARITY)),
            ("vibratory_polarity", T::enumeration("SegyVibratoryPolarity", i16(), VIBRATORY_POLARITY)),
            // Unassigned before revision 2. The revision is written after
            // these bytes, so it is peeked at rather than named.
            ("rev2_counts", T::switch(E::lit(1).less_than(major(3260)), vec![(1, rev2_counts(e))], T::bytes(E::lit(240)))),
            // Revision 1 wrote these two bytes as one 16-bit number with the
            // point between them, 0x0100 for 1.0, which is the same bytes as
            // revision 2's major and minor numbers a byte each.
            ("major_revision", T::u8()),
            ("minor_revision", T::u8()),
            ("fixed_length_traces", T::enumeration("SegyFixedLength", i16(), FIXED_LENGTH)),
            // -1 says there are some and the last one says so.
            ("extended_textual_header_count", i16()),
            (
                "rev2_layout",
                T::switch(
                    E::lit(1).less_than(E::field("major_revision")),
                    vec![(1, T::switch(E::field("minor_revision"), vec![(0, rev2_layout(e, false))], rev2_layout(e, true)))],
                    T::bytes(E::lit(94)),
                ),
            ),
        ],
    )
}

/// Binary header bytes 3261 to 3500 in a revision 2 file: the counts that
/// outgrew their sixteen bits, and the byte order integer.
fn rev2_counts(e: Endian) -> T {
    let order_name = match e {
        Big => "big-endian",
        Little => "little-endian",
    };
    T::structure(
        "SegyRev2Counts",
        vec![
            ("extended_traces_per_ensemble", T::i32(e)),
            ("extended_auxiliary_traces_per_ensemble", T::i32(e)),
            ("extended_samples_per_trace", T::i32(e)),
            ("extended_sample_interval", T::F64(e)),
            ("extended_original_sample_interval", T::F64(e)),
            ("extended_original_samples_per_trace", T::i32(e)),
            ("extended_ensemble_fold", T::i32(e)),
            // Read in the order the file was laid out in, so it is 0x01020304
            // when that order is right.
            ("byte_order", T::enumeration_hex("SegyByteOrder", T::u32(e), &[(0x0102_0304, order_name), (0, "not set")])),
            ("unassigned", T::bytes(E::lit(200))),
        ],
    )
    .machinery(&["unassigned"])
}

/// Binary header bytes 3507 to 3600 in a revision 2 file. `two_one` is
/// revision 2.1's layout, which split the first four bytes in two.
fn rev2_layout(e: Endian, two_one: bool) -> T {
    let mut fields = match two_one {
        true => vec![("max_additional_trace_headers", T::u16(e)), ("survey_type", T::u16(e))],
        false => vec![("max_additional_trace_headers", T::u32(e))],
    };
    fields.extend([
        ("time_basis", T::enumeration("SegyTimeBasis", T::Int { bits: 16, endian: e }, TIME_BASIS)),
        ("trace_count", T::u64(e)),
        // From the start of the file. Zero means the traces follow the
        // extended textual headers.
        ("first_trace_offset", T::u64(e)),
        // 3200-byte blocks after the last trace.
        ("trailer_stanza_count", T::i32(e)),
        ("unassigned", T::bytes(E::lit(68))),
    ]);
    T::structure("SegyRev2Layout", fields).machinery(&["unassigned"])
}

/// The extended textual headers, as many as the binary header says.
///
/// A count of -1 says there are some and leaves finding the last one to the
/// reader. A revision 2 file that also gives the first trace's offset has
/// said where they stop; otherwise they run to the block that is only the
/// `EndText` stanza.
///
/// The count is honoured in a file of any revision. The 1975 standard left
/// those bytes unassigned, but a writer of that era left them zero, and
/// segyio writes a count there in files it labels revision 0.
fn extended_textual_headers() -> T {
    const NAME: &str = "SegyExtendedTextualHeader";
    let count = binary(&["extended_textual_header_count"]);
    let offset = rev2_field(&["rev2_layout", "first_trace_offset"]);
    let fits = |n: E| n.at_least(E::lit(0)).at_most(E::Remaining.div(E::lit(TEXT)));
    let counted = text_blocks(NAME, fits(count.clone()));
    let up_to_offset = text_blocks(NAME, fits(offset.clone().sub(E::lit(FILE_HEADER as i128)).div(E::lit(TEXT))));
    let to_end_text = T::repeat(text_block(NAME, true), Until::Cond(E::field("end_text")));
    let unknown = T::switch(E::lit(0).less_than(offset), vec![(1, up_to_offset)], to_end_text);
    T::switch(count.clone().equal_to(E::lit(-1)), vec![(1, unknown)], counted)
}

/// One when the block starting here opens with the stanza that ends the
/// extended textual headers, as revision 2 spells it or as revision 1 did.
fn is_end_text() -> E {
    let spellings = ["((SEG: EndText))", "((EndText))"];
    let ebcdic = Settled::SingleByte(CodePage::Ebcdic037);
    let mut any = E::lit(0);
    for s in spellings {
        let bits = s.len() as u32 * 8;
        for bytes in [s.as_bytes().to_vec(), encode_settled(ebcdic, s).expect("EBCDIC holds the stanza name")] {
            let word = bytes.iter().fold(0i128, |acc, b| (acc << 8) | i128::from(*b));
            let room = E::lit(s.len() as i128 - 1).less_than(E::Remaining);
            any = any.either(room.both(E::peek_at(E::lit(0), bits, Big).equal_to(E::lit(word))));
        }
    }
    any
}

/// The traces, up to the data trailer.
///
/// With the fixed length flag set, every trace is `trace_bytes` long and they
/// are placed by division. Without it each is measured by its own header as
/// it is reached, and the run stops when there is no longer room for a trace
/// header, leaving whatever is left over to nothing. A format code nothing
/// defines leaves every trace unmeasured, so the traces are left as the bytes
/// they are.
///
/// Every choice is made outside the list rather than inside each trace, so
/// the list has one element type and says what it is a list of.
fn traces(e: Endian) -> T {
    // Read in a file of any revision, for the reason the extended textual
    // header count is.
    let fixed = binary(&["fixed_length_traces"]).equal_to(E::lit(1));
    let room = E::Remaining.sub(trailer_blocks().mul(E::lit(TEXT))).at_least(E::lit(0));
    let run = |fixed: bool, rev2: bool| match fixed {
        true => T::repeat(T::sized(E::field("trace_bytes"), trace(e, true, rev2)), Until::End),
        false => T::repeat(trace(e, false, rev2), Until::Cond(E::Remaining.less_than(E::lit(TRACE_HEADER)))),
    };
    let by_revision = |fixed: bool| T::switch(revision_2(), vec![(1, run(fixed, true))], run(fixed, false));
    let unmeasured = E::field("sample_bytes").equal_to(E::lit(0)).either(E::Remaining.less_than(E::lit(TRACE_HEADER)));
    T::sized(
        room,
        T::switch(unmeasured, vec![(1, T::bytes(E::Remaining))], T::switch(fixed, vec![(1, by_revision(true))], by_revision(false))),
    )
}

/// How many 3200-byte trailer blocks a revision 2 file says follow its traces.
fn trailer_blocks() -> E {
    rev2_field(&["rev2_layout", "trailer_stanza_count"]).at_least(E::lit(0)).at_most(E::Remaining.div(E::lit(TEXT)))
}

fn data_trailer() -> T {
    text_blocks("SegyDataTrailer", trailer_blocks())
}

/// One trace: its header, the extensions revision 2 allows, and its samples.
///
/// `fixed` is a trace in a window the binary header measured, whose samples
/// fill what is left of it. Otherwise the trace's own header says how many
/// samples there are, and a count of zero takes the binary header's.
///
/// `rev2` is a trace in a revision 2 file, which has Extension 1 when the
/// first of its extra headers is named for it, and the rest of its extra
/// headers after that. A trace from an older file has neither field.
fn trace(e: Endian, fixed: bool, rev2: bool) -> T {
    let width = E::field("sample_bytes");
    let fits = E::Remaining.div(width.clone());
    let count = match fixed {
        true => fits,
        false => {
            let own = E::within(&["header", "sample_count"]);
            E::cond(own.clone().equal_to(E::lit(0)), E::field("samples_per_trace"), own).at_most(fits)
        }
    };
    let mut fields = vec![("header", trace_header(e, rev2))];
    if rev2 {
        let extensions = E::field("additional_trace_headers");
        let first = extensions.clone().greater_than(E::lit(0)).both(E::lit(TRACE_HEADER - 1).less_than(E::Remaining)).both(names_extension_1());
        let rest = extensions.sub(E::size_of("extension_1").div(E::lit(TRACE_HEADER)));
        fields.push(("extension_1", T::switch(first, vec![(1, extension_1(e))], T::bytes(E::lit(0)))));
        fields.push(("extensions", T::array(extension(), rest.at_least(E::lit(0)).at_most(E::Remaining.div(E::lit(TRACE_HEADER))))));
    }
    fields.push(("samples", samples(e, count)));
    T::structure_named("SegyTrace", "", "samples", fields).payload(&["samples"]).counted_as("trace")
}

/// The samples, typed by the binary header's format code. The switch is
/// outside the list so that the list has one element type and a stride.
fn samples(e: Endian, count: E) -> T {
    let run = |elem: T| T::array(elem, count.clone());
    T::switch(
        E::field("format"),
        vec![
            (1, run(T::IbmF32(e))),
            (2, run(T::i32(e))),
            (3, run(T::Int { bits: 16, endian: e })),
            // A zero byte, a gain exponent and a 16-bit mantissa, obsolete
            // since 2002 and left as the four bytes.
            (4, run(T::bytes(E::lit(4)))),
            (5, run(T::F32(e))),
            (6, run(T::F64(e))),
            (7, run(T::Int { bits: 24, endian: e })),
            (8, run(T::Int { bits: 8, endian: e })),
            (9, run(T::Int { bits: 64, endian: e })),
            (10, run(T::u32(e))),
            (11, run(T::u16(e))),
            (12, run(T::u64(e))),
            (15, run(T::UInt { bits: 24, endian: e })),
            (16, run(T::u8())),
        ],
        T::bytes(E::Remaining),
    )
}

/// Eight bytes of text in the textual header's encoding, ending at the first
/// zero byte, since a header with no name writes zeros there.
fn header_name() -> T {
    let name = |enc| T::text(StrLen::Padded { size: E::lit(8), pad: 0 }, enc);
    T::switch(E::field("ebcdic"), vec![(1, name(Encoding::Ebcdic))], name(Encoding::Ascii))
}

/// The 240-byte trace header, by revision 1's table. `rev2` names the last
/// eight bytes, which revision 2 made the header's name, `SEG00000`, and which
/// were unassigned before.
///
/// The elevations and depths take `elevation_scalar`, the coordinates and the
/// CDP position `coordinate_scalar`, the times from `source_uphole_time` to
/// `mute_end` take `time_scalar`, and `shotpoint` its own: a positive scalar
/// multiplies and a negative one divides.
fn trace_header(e: Endian, rev2: bool) -> T {
    let i16 = || T::Int { bits: 16, endian: e };
    let i32 = || T::i32(e);
    let mut fields = vec![
        ("trace_sequence_in_line", i32()),
        ("trace_sequence_in_file", i32()),
        ("field_record", i32()),
        ("trace_in_field_record", i32()),
        ("energy_source_point", i32()),
        ("ensemble", i32()),
        ("trace_in_ensemble", i32()),
        ("trace_id", T::enumeration("SegyTraceId", i16(), TRACE_ID)),
        ("vertically_summed_traces", i16()),
        ("horizontally_stacked_traces", i16()),
        ("data_use", T::enumeration("SegyDataUse", i16(), DATA_USE)),
        // From the source point to the receiver group, negative when the
        // receiver is behind the direction the line was shot in.
        ("source_receiver_offset", i32()),
        ("receiver_elevation", i32()),
        ("source_surface_elevation", i32()),
        ("source_depth", i32()),
        ("receiver_datum_elevation", i32()),
        ("source_datum_elevation", i32()),
        ("source_water_depth", i32()),
        ("receiver_water_depth", i32()),
        ("elevation_scalar", i16()),
        ("coordinate_scalar", i16()),
        ("source_x", i32()),
        ("source_y", i32()),
        ("group_x", i32()),
        ("group_y", i32()),
        ("coordinate_units", T::enumeration("SegyCoordinateUnits", i16(), COORDINATE_UNITS)),
        ("weathering_velocity", i16()),
        ("subweathering_velocity", i16()),
        // Milliseconds, all the way to `mute_end`.
        ("source_uphole_time", i16()),
        ("group_uphole_time", i16()),
        ("source_static_correction", i16()),
        ("group_static_correction", i16()),
        ("total_static_applied", i16()),
        ("lag_time_a", i16()),
        ("lag_time_b", i16()),
        // From the energy source going off to the first sample, and negative
        // for data recorded before time zero.
        ("delay_recording_time", i16()),
        ("mute_start", i16()),
        ("mute_end", i16()),
        ("sample_count", T::u16(e)),
        ("sample_interval", i16()),
        ("gain_type", T::enumeration("SegyGainType", i16(), GAIN_TYPE)),
        ("instrument_gain_constant", i16()),
        ("instrument_initial_gain", i16()),
        ("correlated", T::enumeration("SegyCorrelated", i16(), NO_YES)),
        ("sweep_start_frequency", i16()),
        ("sweep_end_frequency", i16()),
        ("sweep_length", i16()),
        ("sweep_type", T::enumeration("SegySweepType", i16(), SWEEP_TYPE)),
        ("sweep_start_taper_length", i16()),
        ("sweep_end_taper_length", i16()),
        ("taper_type", T::enumeration("SegyTaperType", i16(), TAPER_TYPE)),
        ("alias_filter_frequency", i16()),
        ("alias_filter_slope", i16()),
        ("notch_filter_frequency", i16()),
        ("notch_filter_slope", i16()),
        ("low_cut_frequency", i16()),
        ("high_cut_frequency", i16()),
        ("low_cut_slope", i16()),
        ("high_cut_slope", i16()),
        // Four digits from revision 1 on; the 1975 standard never said, and
        // two-digit years are out there.
        ("year", i16()),
        ("day_of_year", i16()),
        ("hour", i16()),
        ("minute", i16()),
        ("second", i16()),
        ("time_basis", T::enumeration("SegyTimeBasis", i16(), TIME_BASIS)),
        // The least significant bit is 2^-N volts.
        ("weighting_factor", i16()),
        ("group_at_roll_switch_one", i16()),
        ("group_of_first_trace", i16()),
        ("group_of_last_trace", i16()),
        ("gap_size", i16()),
        ("over_travel", T::enumeration("SegyOverTravel", i16(), OVER_TRAVEL)),
        ("cdp_x", i32()),
        ("cdp_y", i32()),
        ("inline", i32()),
        ("crossline", i32()),
        ("shotpoint", i32()),
        ("shotpoint_scalar", i16()),
        ("trace_value_unit", T::enumeration("SegyUnit", i16(), UNIT)),
        // The samples times mantissa times ten to the exponent are in
        // `transduction_unit`.
        ("transduction_mantissa", i32()),
        ("transduction_exponent", i16()),
        ("transduction_unit", T::enumeration("SegyUnit", i16(), UNIT)),
        ("device_id", i16()),
        ("time_scalar", i16()),
        ("source_type", i16()),
        // In tenths of a degree from the source orientation.
        ("source_energy_direction_vertical", i16()),
        ("source_energy_direction_crossline", i16()),
        ("source_energy_direction_inline", i16()),
        ("source_measurement_mantissa", i32()),
        ("source_measurement_exponent", i16()),
        ("source_measurement_unit", i16()),
    ];
    let machinery: &[&str] = match rev2 {
        true => {
            fields.push(("header_name", header_name()));
            &[]
        }
        false => {
            fields.push(("unassigned", T::bytes(E::lit(8))));
            &["unassigned"]
        }
    };
    T::structure("SegyTraceHeader", fields)
        .machinery(machinery)
        .payload(&["inline", "crossline", "sample_count"])
}

/// One when the 240 bytes starting here are named for Extension 1.
///
/// The last eight bytes of an extra trace header name it, and `SEG00001` is
/// Extension 1, whose layout the standard fixes and which comes first when a
/// trace has one. A header named nothing at all, eight zero bytes, is taken to
/// be Extension 1 as well, since a writer that leaves the names out still
/// writes that one first. Anything else is some other header, and the file's
/// XML stanza says what is in it, if anything does.
fn names_extension_1() -> E {
    let name = E::peek_at(E::lit((TRACE_HEADER - 8) * 8), 64, Big);
    let ebcdic = encode_settled(Settled::SingleByte(CodePage::Ebcdic037), "SEG00001").expect("EBCDIC holds the name");
    let word = |b: &[u8]| E::lit(i128::from(u64::from_be_bytes(b.try_into().expect("eight bytes"))));
    name.clone().equal_to(word(b"SEG00001")).either(name.clone().equal_to(word(&ebcdic))).either(name.equal_to(E::lit(0)))
}

/// An extra trace header whose layout the standard does not fix: 232 bytes and
/// the name that says what they are.
fn extension() -> T {
    T::structure_named(
        "SegyTraceHeaderExtension",
        "header_name",
        "contents",
        vec![("contents", T::bytes(E::lit(TRACE_HEADER - 8))), ("header_name", header_name())],
    )
}

/// Extension 1: 64-bit versions of the trace header's numbers, with the
/// coordinates, elevations and intervals as IEEE doubles so that they need no
/// scalar.
fn extension_1(e: Endian) -> T {
    let f64 = || T::F64(e);
    T::structure_named(
        "SegyTraceHeaderExtension1",
        "header_name",
        "",
        vec![
            ("trace_sequence_in_line", T::u64(e)),
            ("trace_sequence_in_file", T::u64(e)),
            ("field_record", T::Int { bits: 64, endian: e }),
            ("ensemble", T::Int { bits: 64, endian: e }),
            ("receiver_elevation", f64()),
            ("receiver_depth", f64()),
            ("source_surface_elevation", f64()),
            ("source_depth", f64()),
            ("receiver_datum_elevation", f64()),
            ("source_datum_elevation", f64()),
            ("source_water_depth", f64()),
            ("receiver_water_depth", f64()),
            ("source_x", f64()),
            ("source_y", f64()),
            ("group_x", f64()),
            ("group_y", f64()),
            ("source_receiver_offset", f64()),
            ("sample_count", T::u32(e)),
            // Added to the trace header's second.
            ("nanosecond_of_second", T::i32(e)),
            ("sample_interval", f64()),
            ("recording_device", T::i32(e)),
            // This trace's own count, where it has fewer than the binary
            // header's maximum. Not followed; see the module notes.
            ("additional_trace_header_count", T::u16(e)),
            ("last_trace_flag", T::Int { bits: 16, endian: e }),
            ("cdp_x", f64()),
            ("cdp_y", f64()),
            ("unassigned", T::bytes(E::lit(56))),
            ("header_name", header_name()),
        ],
    )
    .machinery(&["unassigned"])
}

/// A SEG-Y file, told by the front of its textual header and the plausibility
/// of the binary header after it.
///
/// There is no magic. What every file writes is its first card, `C 1` or
/// `C01`, in EBCDIC or ASCII, and three bytes of that is weak evidence on its
/// own: plenty of text files open with a C and a digit. The binary header is
/// what makes it strong. Its format code has to be one the standard defines
/// and its sample interval not zero, read one way round or the other.
pub fn is_segy(head: &[u8], len: u64) -> bool {
    if len < FILE_HEADER as u64 || head.len() < FILE_HEADER {
        return false;
    }
    let card = matches!(&head[..3], [0xc3, 0x40, 0xf1] | [0xc3, 0xf0, 0xf1] | b"C 1" | b"C01");
    let word = |at: usize, big: bool| {
        let b = [head[at], head[at + 1]];
        if big { i16::from_be_bytes(b) } else { i16::from_le_bytes(b) }
    };
    card && [true, false].into_iter().any(|big| {
        let format = i128::from(word(FORMAT_AT, big));
        WIDTH.iter().any(|(code, _)| *code == format) && word(INTERVAL_AT, big) != 0
    })
}

#[cfg(test)]
mod tests;
