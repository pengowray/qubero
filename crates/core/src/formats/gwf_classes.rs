//! The bodies of the IGWD frame classes: FrSH and FrSE, which are the file's
//! own dictionary, and the rest of the twenty the specification defines, each
//! in its version 6 and its version 8 layout.
//!
//! Apart from [`gwf`](super::gwf), which reads the file header and the stream
//! of structures and decides which body each structure gets. Those decisions
//! are short and these layouts are long, and the dispatch is easier to follow
//! without eight hundred lines of fields between its pieces.

use super::gwf::Version::{self, Eight, Six};
use crate::codec::Codec;
use crate::template::{Endian, Endian::*, Expr as E, Ty as T};

/// How the numbers in an FrVect are packed, from FrameL 8.30's
/// `FrVectCompData` and `FrVectExpand`.
///
/// The scheme is the low byte. A writer adds 256 when its own machine is
/// little-endian, so the top byte says which way round the words inside the
/// packed bytes are, and a reader on the other kind of machine knows to swap
/// them before unpacking. Every vector in the GWOSC sample is 257.
const SCHEMES: &[(i128, &str)] = &[
    (0, "none"),
    (1, "gzip"),
    (3, "differences, then gzip"),
    (5, "differences, then zero-suppressed 2-byte words"),
    (6, "differences, then zero-suppressed 2-byte words, gzip for other widths"),
    (7, "differences, then zero-suppressed, floats as 2-byte integers"),
    (8, "differences, then zero-suppressed 4-byte words"),
    (10, "differences, then zero-suppressed 8-byte words"),
    (255, "writer's own scheme"),
];

/// The same list twice: as written by a big-endian machine, and with 256 added
/// as written by a little-endian one.
fn compression(e: Endian) -> T {
    let mut owned: Vec<(i128, String)> = SCHEMES.iter().map(|(k, n)| (*k, (*n).to_string())).collect();
    owned.extend(SCHEMES.iter().map(|(k, n)| (256 + *k, format!("{n} (little-endian words)"))));
    let cases: Vec<(i128, &str)> = owned.iter().map(|(k, n)| (*k, n.as_str())).collect();
    T::enumeration("FrCompression", T::u16(e), &cases)
}

/// What one number of an FrVect is.
const VECT_TYPE: &[(i128, &str)] = &[
    (0, "CHAR"),
    (1, "INT_2S"),
    (2, "REAL_8"),
    (3, "REAL_4"),
    (4, "INT_4S"),
    (5, "INT_8S"),
    (6, "COMPLEX_8"),
    (7, "COMPLEX_16"),
    (8, "STRING"),
    (9, "INT_2U"),
    (10, "INT_4U"),
    (11, "INT_8U"),
    (12, "CHAR_U"),
];

/// A length-counted string: two bytes of count, and that many bytes with a
/// terminating nul among them.
fn string(e: Endian) -> T {
    T::inline_structure(
        "FrString",
        vec![("len", T::u16(e)), ("text", T::utf8_padded(E::field("len"), 0))],
    )
    .contents("text")
}

/// A reference to another structure in the same file, by class and instance.
/// Class zero points at nothing.
///
/// The class is this file's own number, and it stays a number. The dictionary
/// entry that names it is often further on than the pointer is, since a frame
/// header points at the channels written after it, so no search back through
/// the stream can name it; and a table of one library's numbers would name
/// the other library's classes wrongly. See [`CLASSES`](super::gwf::CLASSES).
fn pointer(e: Endian) -> T {
    T::inline_structure(
        "FrPtr",
        vec![("class", T::enumeration("FrPtrClass", T::u16(e), &[(0, "null")])), ("instance", T::u32(e))],
    )
}

/// Every body this reader knows, by the name the specification gives the
/// class. This is the list both routes below choose from: the file's own
/// dictionary picks by the name it declares, and the constant table picks by
/// the number a library would have used.
pub(super) fn bodies(e: Endian, v: Version) -> Vec<(&'static str, T)> {
    vec![
        ("FrSH", frsh(e, v)),
        ("FrSE", frse(e, v)),
        ("FrameH", frame_h(e, v)),
        ("FrAdcData", adc_data(e, v)),
        ("FrDetector", detector(e, v)),
        ("FrEndOfFile", end_of_file(e, v)),
        ("FrEndOfFrame", end_of_frame(e, v)),
        ("FrEvent", event(e, v)),
        ("FrHistory", history(e, v)),
        ("FrMsg", msg(e, v)),
        ("FrProcData", proc_data(e, v)),
        ("FrRawData", raw_data(e, v)),
        ("FrSerData", ser_data(e, v)),
        ("FrSimData", sim_data(e, v)),
        ("FrSimEvent", sim_event(e, v)),
        ("FrStatData", stat_data(e, v)),
        ("FrSummary", summary(e, v)),
        ("FrTable", table(e, v)),
        ("FrTOC", toc(e, v)),
        ("FrVect", vect(e, v)),
    ]
}

/// A body's own fields, and in version 8 the checksum every structure there
/// ends with. Version 6 has no checksum on a structure; its only checksums
/// are the frame's and the file's, which have fields of their own.
fn sealed(e: Endian, v: Version, mut fields: Vec<(&'static str, T)>) -> Vec<(&'static str, T)> {
    if v == Eight {
        fields.push(("chkSum", T::u32(e)));
    }
    fields
}

/// The four fields that grew after version 6: `sampleRate` of an FrSerData
/// and an FrSimData, and the parameters of an FrEvent and an FrSimEvent. Four
/// bytes in version 6 and eight in version 8. FrameL's version 6 file says
/// REAL_4 for all four in its dictionary, and FrAdcData's `sampleRate`, which
/// a loose reading of the same change could take in, is REAL_8 in both version
/// 6 samples.
fn widened(e: Endian, v: Version) -> T {
    match v {
        Six => T::F32(e),
        Eight => T::F64(e),
    }
}

/// A dictionary entry: this file calls class `class` by this name.
pub(super) fn frsh(e: Endian, v: Version) -> T {
    T::structure_named(
        "FrSH",
        "name",
        "",
        sealed(e, v, vec![
            ("name", string(e)),
            ("class", T::u16(e)),
            ("comment", string(e)),
        ]),
    )
    // The two together are the entry: this file calls class 4 `FrAdcData`.
    .payload(&["name", "class"])
}

/// One field of the class the FrSH before it named: what it is called and what
/// it is. The type is written as text, `INT_4U` or `REAL_8[nDim]`.
pub(super) fn frse(e: Endian, v: Version) -> T {
    T::structure_named(
        "FrSE",
        "name",
        "",
        sealed(e, v, vec![
            ("name", string(e)),
            ("type", string(e)),
            ("comment", string(e)),
        ]),
    )
}

fn frame_h(e: Endian, v: Version) -> T {
    T::structure_named(
        "FrameH",
        "name",
        "",
        sealed(e, v, vec![
            ("name", string(e)),
            ("run", T::i32(e)),
            ("frame", T::u32(e)),
            ("dataQuality", T::u32(e)),
            ("GTimeS", T::u32(e)),
            ("GTimeN", T::u32(e)),
            ("ULeapS", T::u16(e)),
            ("dt", T::F64(e)),
            ("type", pointer(e)),
            ("user", pointer(e)),
            ("detectSim", pointer(e)),
            ("detectProc", pointer(e)),
            ("history", pointer(e)),
            ("rawData", pointer(e)),
            ("procData", pointer(e)),
            ("simData", pointer(e)),
            ("event", pointer(e)),
            ("simEvent", pointer(e)),
            ("summaryData", pointer(e)),
            ("auxData", pointer(e)),
            ("auxTable", pointer(e)),
        ]),
    )
    .payload(&["GTimeS", "dt"])
}

fn detector(e: Endian, v: Version) -> T {
    T::structure_named(
        "FrDetector",
        "name",
        "",
        sealed(e, v, vec![
            ("name", string(e)),
            ("prefix", T::utf8(E::lit(2))),
            ("longitude", T::F64(e)),
            ("latitude", T::F64(e)),
            ("elevation", T::F32(e)),
            ("armXazimuth", T::F32(e)),
            ("armYazimuth", T::F32(e)),
            ("armXaltitude", T::F32(e)),
            ("armYaltitude", T::F32(e)),
            ("armXmidpoint", T::F32(e)),
            ("armYmidpoint", T::F32(e)),
            ("localTime", T::i32(e)),
            ("aux", pointer(e)),
            ("table", pointer(e)),
            ("next", pointer(e)),
        ]),
    )
}

fn proc_data(e: Endian, v: Version) -> T {
    T::structure_named(
        "FrProcData",
        "name",
        "",
        sealed(e, v, vec![
            ("name", string(e)),
            ("comment", string(e)),
            ("type", T::u16(e)),
            ("subType", T::u16(e)),
            ("timeOffset", T::F64(e)),
            ("tRange", T::F64(e)),
            ("fShift", T::F64(e)),
            ("phase", T::F32(e)),
            ("fRange", T::F64(e)),
            ("BW", T::F64(e)),
            ("nAuxParam", T::u16(e)),
            ("auxParam", T::array(T::F64(e), E::field("nAuxParam"))),
            ("auxParamNames", T::array(string(e), E::field("nAuxParam"))),
            ("data", pointer(e)),
            ("aux", pointer(e)),
            ("table", pointer(e)),
            ("history", pointer(e)),
            ("next", pointer(e)),
        ]),
    )
}

/// One channel of the digitiser, as it came off the hardware: what it was
/// called, what one count of it is worth, and a pointer to the numbers.
///
/// `bias` and `slope` are what turn a count back into volts, and `nBits` how
/// many of the bits of a sample the converter actually set.
fn adc_data(e: Endian, v: Version) -> T {
    T::structure_named(
        "FrAdcData",
        "name",
        "",
        sealed(e, v, vec![
            ("name", string(e)),
            ("comment", string(e)),
            ("channelGroup", T::u32(e)),
            ("channelNumber", T::u32(e)),
            ("nBits", T::u32(e)),
            ("bias", T::F32(e)),
            ("slope", T::F32(e)),
            ("units", string(e)),
            ("sampleRate", T::F64(e)),
            ("timeOffset", T::F64(e)),
            ("fShift", T::F64(e)),
            ("phase", T::F32(e)),
            // Zero when the channel was known to be bad while it recorded.
            ("dataValid", T::u16(e)),
            ("data", pointer(e)),
            ("aux", pointer(e)),
            ("next", pointer(e)),
        ]),
    )
    .payload(&["sampleRate", "slope", "units"])
}

/// Something a trigger found in the data: when, how big, and how sure.
///
/// The parameters are a list of doubles with a list of names beside it, which
/// is how one structure carries whatever a given search wanted to record.
fn event(e: Endian, v: Version) -> T {
    let n = E::field("nParam");
    T::structure_named(
        "FrEvent",
        "name",
        "",
        sealed(e, v, vec![
            ("name", string(e)),
            ("comment", string(e)),
            ("inputs", string(e)),
            ("GTimeS", T::u32(e)),
            ("GTimeN", T::u32(e)),
            ("timeBefore", T::F32(e)),
            ("timeAfter", T::F32(e)),
            ("eventStatus", T::u32(e)),
            ("amplitude", T::F32(e)),
            ("probability", T::F32(e)),
            ("statistics", string(e)),
            ("nParam", T::u16(e)),
            ("parameters", T::array(widened(e, v), n.clone())),
            ("parameterNames", T::array(string(e), n)),
            ("data", pointer(e)),
            ("table", pointer(e)),
            ("next", pointer(e)),
        ]),
    )
    .payload(&["GTimeS", "amplitude"])
}

/// A line of the record of what was done to this frame: a program, when it
/// ran, and what it said about itself.
fn history(e: Endian, v: Version) -> T {
    T::structure_named(
        "FrHistory",
        "name",
        "",
        sealed(e, v, vec![
            ("name", string(e)),
            ("time", T::u32(e)),
            ("comment", string(e)),
            ("next", pointer(e)),
        ]),
    )
}

/// One line of the detector's log, kept in the frame so that what the
/// instrument was complaining about arrives with the data it was recording.
fn msg(e: Endian, v: Version) -> T {
    T::structure_named(
        "FrMsg",
        "alarm",
        "",
        sealed(e, v, vec![
            ("alarm", string(e)),
            ("message", string(e)),
            ("severity", T::u32(e)),
            ("GTimeS", T::u32(e)),
            ("GTimeN", T::u32(e)),
            ("next", pointer(e)),
        ]),
    )
    .payload(&["message"])
}

/// The head of the raw data: five pointers at the lists of everything the
/// instrument itself wrote, and nothing of its own but a name.
fn raw_data(e: Endian, v: Version) -> T {
    T::structure_named(
        "FrRawData",
        "name",
        "",
        sealed(e, v, vec![
            ("name", string(e)),
            ("firstSer", pointer(e)),
            ("firstAdc", pointer(e)),
            ("firstTable", pointer(e)),
            ("logMsg", pointer(e)),
            ("more", pointer(e)),
        ]),
    )
}

/// A slow channel that arrives as text: the station keeping, read off a serial
/// line, with the whole line kept as it came.
fn ser_data(e: Endian, v: Version) -> T {
    T::structure_named(
        "FrSerData",
        "name",
        "",
        sealed(e, v, vec![
            ("name", string(e)),
            ("timeSec", T::u32(e)),
            ("timeNsec", T::u32(e)),
            ("sampleRate", widened(e, v)),
            ("data", string(e)),
            ("serial", pointer(e)),
            ("table", pointer(e)),
            ("next", pointer(e)),
        ]),
    )
    .payload(&["timeSec", "data"])
}

/// A channel that was made up rather than recorded: an injected signal, kept
/// beside the real data it was added to.
fn sim_data(e: Endian, v: Version) -> T {
    T::structure_named(
        "FrSimData",
        "name",
        "",
        sealed(e, v, vec![
            ("name", string(e)),
            ("comment", string(e)),
            ("sampleRate", widened(e, v)),
            ("timeOffset", T::F64(e)),
            ("fShift", T::F64(e)),
            ("phase", T::F32(e)),
            ("data", pointer(e)),
            ("input", pointer(e)),
            ("table", pointer(e)),
            ("next", pointer(e)),
        ]),
    )
}

/// An event that was injected rather than found. The same shape as an FrEvent
/// without the fields that only mean something for a trigger.
fn sim_event(e: Endian, v: Version) -> T {
    let n = E::field("nParam");
    T::structure_named(
        "FrSimEvent",
        "name",
        "",
        sealed(e, v, vec![
            ("name", string(e)),
            ("comment", string(e)),
            ("inputs", string(e)),
            ("GTimeS", T::u32(e)),
            ("GTimeN", T::u32(e)),
            ("timeBefore", T::F32(e)),
            ("timeAfter", T::F32(e)),
            ("amplitude", T::F32(e)),
            ("nParam", T::u16(e)),
            ("parameters", T::array(widened(e, v), n.clone())),
            ("parameterNames", T::array(string(e), n)),
            ("data", pointer(e)),
            ("table", pointer(e)),
            ("next", pointer(e)),
        ]),
    )
    .payload(&["GTimeS", "amplitude"])
}

/// Something about the detector that does not change every frame: a
/// calibration, valid between two times, with a version so a later one can
/// replace it.
fn stat_data(e: Endian, v: Version) -> T {
    T::structure_named(
        "FrStatData",
        "name",
        "",
        sealed(e, v, vec![
            ("name", string(e)),
            ("comment", string(e)),
            ("representation", string(e)),
            ("timeStart", T::u32(e)),
            ("timeEnd", T::u32(e)),
            ("version", T::u32(e)),
            ("detector", pointer(e)),
            ("data", pointer(e)),
            ("table", pointer(e)),
        ]),
    )
    .payload(&["timeStart", "timeEnd", "version"])
}

/// A number worked out about a stretch of data rather than sampled from it:
/// what the test was, and a vector of what it came to.
fn summary(e: Endian, v: Version) -> T {
    T::structure_named(
        "FrSummary",
        "name",
        "",
        sealed(e, v, vec![
            ("name", string(e)),
            ("comment", string(e)),
            ("test", string(e)),
            ("GTimeS", T::u32(e)),
            ("GTimeN", T::u32(e)),
            ("moments", pointer(e)),
            ("table", pointer(e)),
            ("next", pointer(e)),
        ]),
    )
}

/// A table: the column names here, and the columns themselves in the FrVect
/// chain `column` points at, one vector per column.
fn table(e: Endian, v: Version) -> T {
    T::structure_named(
        "FrTable",
        "name",
        "",
        sealed(e, v, vec![
            ("name", string(e)),
            ("comment", string(e)),
            ("nColumn", T::u16(e)),
            ("nRow", T::u32(e)),
            ("columnName", T::array(string(e), E::field("nColumn"))),
            ("column", pointer(e)),
            ("next", pointer(e)),
        ]),
    )
    .payload(&["nRow", "nColumn"])
}

/// The end of a frame. Version 6 keeps the frame's checksum here, and what
/// kind of checksum it is; version 8 checksums each structure instead, and
/// repeats the frame's start time.
fn end_of_frame(e: Endian, v: Version) -> T {
    let mut fields = vec![("run", T::i32(e)), ("frame", T::u32(e))];
    match v {
        Six => fields.extend([("chkType", T::u32(e)), ("chkSum", T::u32(e))]),
        Eight => fields.extend([("GTimeS", T::u32(e)), ("GTimeN", T::u32(e)), ("chkSum", T::u32(e))]),
    }
    T::structure("FrEndOfFrame", fields)
}

/// The end of the file: how many frames, how many bytes, and where the table
/// of contents is. The same three things in both versions, in a different
/// order around different checksums.
fn end_of_file(e: Endian, v: Version) -> T {
    let mut fields = vec![("nFrames", T::u32(e)), ("nBytes", T::u64(e))];
    match v {
        Six => fields.extend([("chkType", T::u32(e)), ("chkSum", T::u32(e)), ("seekTOC", T::u64(e))]),
        Eight => fields.extend([
            ("seekTOC", T::u64(e)),
            ("chkSumFrHeader", T::u32(e)),
            ("chkSum", T::u32(e)),
            ("chkSumFile", T::u32(e)),
        ]),
    }
    T::structure("FrEndOfFile", fields).payload(&["nFrames", "nBytes"])
}

/// A vector: a name, how its numbers are packed, and the packed bytes. The
/// dimensions come after the data, which is why `nBytes` has to say how far it
/// runs rather than the shape working it out.
fn vect(e: Endian, v: Version) -> T {
    let dims = E::field("nDim");
    T::structure_named(
        "FrVect",
        "name",
        "data",
        sealed(e, v, vec![
            ("name", string(e)),
            ("compress", compression(e)),
            ("type", T::enumeration("FrVectType", T::u16(e), VECT_TYPE)),
            ("nData", T::u64(e)),
            ("nBytes", T::u64(e)),
            ("data", T::sized(E::field("nBytes").at_most(E::Remaining), vect_data(e))),
            ("nDim", T::u32(e)),
            ("nx", T::array(T::u64(e), dims.clone())),
            ("dx", T::array(T::F64(e), dims.clone())),
            ("startX", T::array(T::F64(e), dims.clone())),
            ("unitX", T::array(string(e), dims)),
            ("unitY", string(e)),
            ("next", pointer(e)),
        ]),
    )
    .payload(&["nData"])
}

/// What is inside a vector: the numbers themselves when nothing was packed,
/// and what the packing made of them otherwise.
///
/// Nothing packed is 0 or 256, the same scheme written by a big-endian and a
/// little-endian machine, and both read the way round the file's header says.
/// FrameL writes 256 on every vector it leaves alone, so reading only 0 left
/// every vector of a FrameL file as bytes.
///
/// Every packed scheme has the same two numbers, one for each kind of machine,
/// and inside the packed run the words are the way round the number says
/// rather than the way the file's header does: a vector copied still packed
/// from one file into another keeps the byte order it was packed in.
///
/// - gzip opens as a space of its own, typed by the vector's `type` and
///   `nData`, since it is the zlib codec and nothing more.
/// - Differences then gzip opens the same way, and what is in the space is
///   named for what it is: the difference of each number from the one before.
///   Adding them back up is [`gwf_vect`](super::gwf_vect)'s, which reports
///   the steps.
/// - Zero suppression is a bit packing no codec here reads, and it needs
///   `nData` from outside the run, so its block size is read as the field it
///   is and the rest is left to the same side reader.
///
/// Any other number keeps its bytes.
fn vect_data(e: Endian) -> T {
    let gzip = |ce: Endian| T::decoded(E::Remaining, Codec::Zlib, numbers(ce));
    let differences = |ce: Endian| {
        T::structure("DifferencesThenGzip", vec![("differences", gzip(ce))]).packed_as(super::gwf_vect::PACKING)
    };
    let suppressed = |ce: Endian| {
        T::structure("ZeroSuppressed", vec![("block_size", T::u16(ce)), ("packed", T::bytes(E::Remaining))])
            .machinery(&["block_size"])
            .packed_as(super::gwf_vect::PACKING)
    };
    T::switch(
        E::field("compress"),
        vec![
            (0, numbers(e)),
            (256, numbers(e)),
            (1, gzip(Big)),
            (257, gzip(Little)),
            (3, differences(Big)),
            (259, differences(Little)),
            (5, suppressed(Big)),
            (261, suppressed(Little)),
            (8, suppressed(Big)),
            (264, suppressed(Little)),
            (10, suppressed(Big)),
            (266, suppressed(Little)),
        ],
        T::bytes(E::Remaining),
    )
}

/// The `nData` numbers of a vector, as its `type` names them, read from the
/// front of however many bytes there are.
///
/// A complex number is its real part and then its imaginary part, one pair
/// after another. A string vector is `nData` counted strings, which is how an
/// FrTable keeps a column of channel names. The count is held to what the
/// bytes could hold, so a vector whose `nData` is wrong reads as many numbers
/// as are there rather than asking for billions.
fn numbers(e: Endian) -> T {
    let each = |t: T, width: i128| T::array(t, E::field("nData").at_most(E::Remaining.div(E::lit(width))));
    let complex = |part: T| T::inline_structure("Complex", vec![("re", part.clone()), ("im", part)]);
    T::switch(
        E::field("type"),
        vec![
            (0, each(T::Int { bits: 8, endian: e }, 1)),
            (1, each(T::Int { bits: 16, endian: e }, 2)),
            (2, each(T::F64(e), 8)),
            (3, each(T::F32(e), 4)),
            (4, each(T::i32(e), 4)),
            (5, each(T::Int { bits: 64, endian: e }, 8)),
            (6, each(complex(T::F32(e)), 8)),
            (7, each(complex(T::F64(e)), 16)),
            (8, each(string(e), 2)),
            (9, each(T::u16(e), 2)),
            (10, each(T::u32(e), 4)),
            (11, each(T::u64(e), 8)),
            (12, each(T::u8(), 1)),
        ],
        T::bytes(E::Remaining),
    )
}

/// A count that may be written as -1 for a table this file has none of, which
/// is what FrameCPP does. Read as a count, that is four billion entries.
fn count(name: &str) -> E {
    E::field(name).less_than(E::lit(0xffff_ffffu32)).mul(E::field(name))
}

/// The table of contents: where every channel of every frame is, so a reader
/// after one channel need not walk the file. Every table in it is a count and
/// then that many of each column, and a count of 0xffffffff means the table is
/// not there at all.
///
/// Version 6 has no totals. Its static data is a run of groups, one per type,
/// each with its own count of instances; and its event columns are as long as
/// the per-type counts before them add up to, which `nTotalEvent` says outright
/// from version 8. That is how FrameL 6.24's `FrTOCWrite` lays it out and what
/// the version 6 specification's table says. FrameL's version 6 file checks
/// the events, with 30 in two types; no version 6 sample has static data, so
/// the groups are the specification's and FrameL's word.
fn toc(e: Endian, v: Version) -> T {
    let (u32a, u64a, f64a, i32a) = (
        |n: E| T::array(T::u32(e), n),
        |n: E| T::array(T::u64(e), n),
        |n: E| T::array(T::F64(e), n),
        |n: E| T::array(T::i32(e), n),
    );
    let strings = |n: E| T::array(string(e), n);
    // A column of positions, one per name and per frame.
    let per_frame = |n: E| T::array(T::array(T::u64(e), count("nFrame")), n);
    let (nf, nsh, ndet) = (count("nFrame"), count("nSH"), count("nDetector"));
    let (nstat, ntotal, nadc) = (count("nStatType"), count("nTotalStat"), count("nADC"));
    if v == Six {
        let group = T::structure_named(
            "FrTOCStat",
            "nameStat",
            "",
            vec![
                ("nameStat", string(e)),
                ("detector", string(e)),
                ("nStatInstance", T::u32(e)),
                ("tStart", u32a(count("nStatInstance"))),
                ("tEnd", u32a(count("nStatInstance"))),
                ("version", u32a(count("nStatInstance"))),
                ("positionStat", u64a(count("nStatInstance"))),
            ],
        );
        // A column as long as the counts in `counts` add up to, held to what
        // the bytes left could hold.
        let summed = |counts: &str, width: i128| E::sum_of(counts).at_most(E::Remaining.div(E::lit(width)));
        return T::structure(
            "FrTOC",
            vec![
                ("ULeapS", T::Int { bits: 16, endian: e }),
                ("nFrame", T::u32(e)),
                ("dataQuality", u32a(nf.clone())),
                ("GTimeS", u32a(nf.clone())),
                ("GTimeN", u32a(nf.clone())),
                ("dt", f64a(nf.clone())),
                ("runs", i32a(nf.clone())),
                ("frame", u32a(nf.clone())),
                ("positionH", u64a(nf.clone())),
                ("nFirstADC", u64a(nf.clone())),
                ("nFirstSer", u64a(nf.clone())),
                ("nFirstTable", u64a(nf.clone())),
                ("nFirstMsg", u64a(nf)),
                ("nSH", T::u32(e)),
                ("SHid", T::array(T::u16(e), nsh.clone())),
                ("SHname", strings(nsh)),
                ("nDetector", T::u32(e)),
                ("nameDetector", strings(ndet.clone())),
                ("positionDetector", u64a(ndet)),
                ("nStatType", T::u32(e)),
                ("stat_types", T::array(group, nstat)),
                ("nADC", T::u32(e)),
                ("name", strings(nadc.clone())),
                ("channelID", u32a(nadc.clone())),
                ("groupID", u32a(nadc.clone())),
                ("positionADC", per_frame(nadc)),
                ("nProc", T::u32(e)),
                ("nameProc", strings(count("nProc"))),
                ("positionProc", per_frame(count("nProc"))),
                ("nSim", T::u32(e)),
                ("nameSim", strings(count("nSim"))),
                ("positionSim", per_frame(count("nSim"))),
                ("nSer", T::u32(e)),
                ("nameSer", strings(count("nSer"))),
                ("positionSer", per_frame(count("nSer"))),
                ("nSummary", T::u32(e)),
                ("nameSum", strings(count("nSummary"))),
                ("positionSum", per_frame(count("nSummary"))),
                ("nEventType", T::u32(e)),
                ("nameEvent", strings(count("nEventType"))),
                ("nEvent", u32a(count("nEventType"))),
                ("GTimeSEvent", u32a(summed("nEvent", 4))),
                ("GTimeNEvent", u32a(summed("nEvent", 4))),
                ("amplitudeEvent", T::array(T::F32(e), summed("nEvent", 4))),
                ("positionEvent", u64a(summed("nEvent", 8))),
                ("nSimEventType", T::u32(e)),
                ("nameSimEvent", strings(count("nSimEventType"))),
                ("nSimEvent", u32a(count("nSimEventType"))),
                ("GTimeSSim", u32a(summed("nSimEvent", 4))),
                ("GTimeNSim", u32a(summed("nSimEvent", 4))),
                ("amplitudeSimEvent", T::array(T::F32(e), summed("nSimEvent", 4))),
                ("positionSimEvent", u64a(summed("nSimEvent", 8))),
            ],
        );
    }
    T::structure(
        "FrTOC",
        vec![
            ("ULeapS", T::Int { bits: 16, endian: e }),
            ("nFrame", T::u32(e)),
            ("dataQuality", u32a(nf.clone())),
            ("GTimeS", u32a(nf.clone())),
            ("GTimeN", u32a(nf.clone())),
            ("dt", f64a(nf.clone())),
            ("runs", i32a(nf.clone())),
            ("frame", u32a(nf.clone())),
            ("positionH", u64a(nf.clone())),
            ("nFirstADC", u64a(nf.clone())),
            ("nFirstSer", u64a(nf.clone())),
            ("nFirstTable", u64a(nf.clone())),
            ("nFirstMsg", u64a(nf)),
            ("nSH", T::u32(e)),
            ("SHid", T::array(T::u16(e), nsh.clone())),
            ("SHname", strings(nsh)),
            ("nDetector", T::u32(e)),
            ("nameDetector", strings(ndet.clone())),
            ("positionDetector", u64a(ndet)),
            ("nStatType", T::u32(e)),
            ("nameStat", strings(nstat.clone())),
            ("detector", strings(nstat.clone())),
            ("nStatInstance", u32a(nstat)),
            ("nTotalStat", T::u32(e)),
            ("tStart", u32a(ntotal.clone())),
            ("tEnd", u32a(ntotal.clone())),
            ("version", u32a(ntotal.clone())),
            ("positionStat", u64a(ntotal)),
            ("nADC", T::u32(e)),
            ("name", strings(nadc.clone())),
            ("channelID", u32a(nadc.clone())),
            ("groupID", u32a(nadc.clone())),
            ("positionADC", per_frame(nadc)),
            ("nProc", T::u32(e)),
            ("nameProc", strings(count("nProc"))),
            ("positionProc", per_frame(count("nProc"))),
            ("nSim", T::u32(e)),
            ("nameSim", strings(count("nSim"))),
            ("positionSim", per_frame(count("nSim"))),
            ("nSer", T::u32(e)),
            ("nameSer", strings(count("nSer"))),
            ("positionSer", per_frame(count("nSer"))),
            ("nSummary", T::u32(e)),
            ("nameSum", strings(count("nSummary"))),
            ("positionSum", per_frame(count("nSummary"))),
            ("nEventType", T::u32(e)),
            ("nameEvent", strings(count("nEventType"))),
            ("nEvent", u32a(count("nEventType"))),
            ("nTotalEvent", T::u32(e)),
            ("GTimeSEvent", u32a(count("nTotalEvent"))),
            ("GTimeNEvent", u32a(count("nTotalEvent"))),
            ("amplitudeEvent", T::array(T::F32(e), count("nTotalEvent"))),
            ("positionEvent", u64a(count("nTotalEvent"))),
            ("nSimEventType", T::u32(e)),
            ("nameSimEvent", strings(count("nSimEventType"))),
            ("nSimEvent", u32a(count("nSimEventType"))),
            ("nTotalSEvent", T::u32(e)),
            ("GTimeSSim", u32a(count("nTotalSEvent"))),
            ("GTimeNSim", u32a(count("nTotalSEvent"))),
            ("amplitudeSimEvent", T::array(T::F32(e), count("nTotalSEvent"))),
            ("positionSimEvent", u64a(count("nTotalSEvent"))),
            ("chkSum", T::u32(e)),
        ],
    )
}
