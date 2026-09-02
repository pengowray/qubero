//! IGWD frame files: the raw and processed data of the LIGO, Virgo and KAGRA
//! gravitational wave detectors. Specification LIGO-T970130, version 8.
//!
//! A frame file is forty bytes of header and then a flat stream of
//! self-describing structures. The header is the only part with a fixed
//! layout: a magic, the format version, how wide this writer made each of its
//! five number types, and then the same three integers and two copies of pi
//! every writer puts there so a reader can tell which way round the numbers
//! are. 0x1234 read as 0x3412 is a file from the other kind of machine, and
//! everything after the check word is read the other way about. That is the
//! same trick an ELF plays with its data byte, and it is answered here the
//! same way: the switch has one arm per byte order, each holding the whole
//! rest of the file built with that endianness.
//!
//! Every structure after the header is a length, a checksum kind, a class, an
//! instance number, and a body. Two classes are fixed by the specification:
//! class 1 is FrSH, a dictionary entry naming a class, and class 2 is FrSE,
//! one field of the class the FrSH before it named. So a frame file carries
//! its own schema, and the first hundred and fifty structures of the sample
//! this was written against are nothing but that schema.
//!
//! **How a body is chosen.** Every class number other than 1 and 2 is assigned
//! by the writer, and what it means is only knowable by reading the FrSH
//! structures earlier in the same stream. That is the question
//! [`Expr::sibling_tagged`] asks: among the structures before this one, find
//! the FrSH whose `class` field holds this structure's class byte, and read the
//! name it declares. The body is then picked by that name, not by a number,
//! so a file that numbers `FrAdcData` 4 and a file that numbers it 40 both
//! read as an `FrAdcData`.
//!
//! The constant table below is what is left when that question has no answer:
//! a file with no dictionary at all, or one whose dictionary does not cover
//! the class in hand. It is the numbering FrameL and FrameCPP assign, which is
//! the order the specification lists the structures in. A file that both
//! declares a class and calls it something this reader has never heard of gets
//! its bytes, which is the honest answer.
//!
//! Two names, then, for a file that numbers its classes its own way: the class
//! byte reads as an enum of the standard numbering, and the body reads as the
//! structure the file said it was. A computed label is a number in the IR and
//! never text, so the class byte cannot be made to say what the dictionary
//! calls it. Where the two disagree, the body is the one to believe, and
//! seeing both is how a reader knows they disagreed at all.
//!
//! What is read: the header, the structure stream, and every class the
//! specification defines. FrameH, FrDetector, FrProcData, FrVect, FrEndOfFrame,
//! FrTOC and FrEndOfFile are checked field for field against the GWOSC
//! sample's own dictionary, all 162 structures of it. The other eleven are
//! from FrameL 8.30's `Fr*Def()` functions, which are what writes the
//! dictionary a file carries; no sample here declares them, so nothing has
//! checked them against bytes.
//!
//! What stays bytes: the contents of a compressed FrVect, which is named but
//! not unpacked; the body of a class this reader has no layout for; and the
//! whole stream of a version 6 or 7 file past its structure headers, which
//! nothing here has a sample of. Version 6 differs in more than its header:
//! `sampleRate` and the event parameters are 4-byte floats there and 8-byte
//! ones from version 8, and no field is written out on a guess.

use crate::template::{Endian, Endian::*, Expr as E, Template, Ty as T, Until};

/// Which library wrote the file, from byte 38 of the header.
const LIBRARY: &[(i128, &str)] = &[(0, "unknown"), (1, "FrameLib"), (2, "FrameCPP")];

/// How the checksums in the file were worked out, from byte 39.
const CHECKSUM_SCHEME: &[(i128, &str)] = &[(0, "none"), (1, "CRC")];

/// What a structure's own checksum covers.
const CHECKSUM_KIND: &[(i128, &str)] = &[(0, "none"), (1, "structure")];

/// The class numbers as FrameL and FrameCPP assign them, which is the order
/// the specification lists the structures in. Only 1 and 2 are fixed by the
/// format; see the note at the top about the rest.
const CLASSES: &[(i128, &str)] = &[
    (1, "FrSH"),
    (2, "FrSE"),
    (3, "FrameH"),
    (4, "FrAdcData"),
    (5, "FrDetector"),
    (6, "FrEndOfFile"),
    (7, "FrEndOfFrame"),
    (8, "FrEvent"),
    (9, "FrHistory"),
    (10, "FrMsg"),
    (11, "FrProcData"),
    (12, "FrRawData"),
    (13, "FrSerData"),
    (14, "FrSimData"),
    (15, "FrSimEvent"),
    (16, "FrStatData"),
    (17, "FrSummary"),
    (18, "FrTable"),
    (19, "FrTOC"),
    (20, "FrVect"),
];

/// The same numbers as a pointer writes them, where nothing pointed at is
/// class zero.
fn pointer_classes() -> Vec<(i128, &'static str)> {
    let mut v = vec![(0i128, "null")];
    v.extend_from_slice(CLASSES);
    v
}

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

pub fn gwf() -> Template {
    Template::new("gwf", header())
}

/// The forty bytes every frame file opens with. Nothing before the check word
/// depends on byte order: a magic, two version bytes, and five widths.
fn header() -> T {
    T::structure_named(
        "FrameFile",
        "",
        "contents",
        vec![
            ("magic", T::magic(b"IGWD\0")),
            ("version", T::u8()),
            ("minor_version", T::u8()),
            ("size_int2", T::u8()),
            ("size_int4", T::u8()),
            ("size_int8", T::u8()),
            ("size_real4", T::u8()),
            ("size_real8", T::u8()),
            (
                "contents",
                // 0x1234 written by a machine of the other kind reads as
                // 0x3412, and says the rest of the file is the other way
                // round. Anything else is not a frame file's header.
                T::switch(
                    E::peek(16, Little),
                    vec![(0x1234, rest(Little)), (0x3412, rest(Big))],
                    T::bytes(E::Remaining),
                ),
            ),
        ],
    )
    .machinery(&["size_int2", "size_int4", "size_int8", "size_real4", "size_real8"])
}

/// Everything past the widths, read the way round the check word says.
fn rest(e: Endian) -> T {
    T::structure(
        "FrameFileBody",
        vec![
            ("check_int2", T::u16(e)),
            ("check_int4", T::u32(e)),
            ("check_int8", T::u64(e)),
            ("check_real4", T::F32(e)),
            ("check_real8", T::F64(e)),
            ("library", T::enumeration("FrameLibrary", T::u8(), LIBRARY)),
            ("checksum_scheme", T::enumeration("ChecksumScheme", T::u8(), CHECKSUM_SCHEME)),
            (
                "structures",
                // Version 8 grew the length to eight bytes and gained the
                // checksum byte. Nothing here has a version 6 or 7 file to
                // check the older header against, so its bodies stay bytes.
                T::switch(
                    E::field("version").less_than(E::lit(8)),
                    vec![(1, T::repeat(old_structure(e), Until::End))],
                    T::repeat(structure(e), Until::End),
                ),
            ),
        ],
    )
    .machinery(&["check_int2", "check_int4", "check_int8", "check_real4", "check_real8"])
}

/// One structure of a version 8 file: fourteen bytes of header and a body as
/// long as the length says.
fn structure(e: Endian) -> T {
    T::structure_named(
        "FrStructure",
        "class",
        "body",
        vec![
            ("length", T::u64(e)),
            ("checksum_kind", T::enumeration("ChecksumKind", T::u8(), CHECKSUM_KIND)),
            ("class", T::enumeration("FrClass", T::u8(), CLASSES)),
            ("instance", T::u32(e)),
            ("body", T::sized(body_size(14), class_body(e))),
        ],
    )
    .machinery(&["length", "checksum_kind", "instance"])
}

/// One structure of a version 6 or 7 file. The header the specification gives
/// for those, and the body left as bytes: no file of either is here to check a
/// layout against, and a guessed one would read as words rather than as the
/// numbers it got wrong.
fn old_structure(e: Endian) -> T {
    T::structure_named(
        "FrStructure",
        "class",
        "body",
        vec![
            ("length", T::u32(e)),
            ("class", T::enumeration("FrClass", T::u16(e), CLASSES)),
            ("instance", T::u32(e)),
            ("body", T::sized(body_size(10), T::bytes(E::Remaining))),
        ],
    )
    .machinery(&["length", "instance"])
}

/// How much of a structure is body: its length less its header, never below
/// zero and never past the end of the file. A length that lies is a file that
/// was cut short or written wrong, and reading what is there says so.
fn body_size(header: i128) -> E {
    E::field("length").sub(E::lit(header)).at_least(E::lit(0)).at_most(E::Remaining)
}

/// A length-counted string: two bytes of count, and that many bytes with a
/// terminating nul among them.
fn string(e: Endian) -> T {
    T::inline_structure(
        "FrString",
        vec![("len", T::u16(e)), ("text", T::utf8_padded(E::field("len"), 0))],
    )
}

/// A reference to another structure in the same file, by class and instance.
/// Class zero points at nothing.
fn pointer(e: Endian) -> T {
    T::inline_structure(
        "FrPtr",
        vec![
            ("class", T::enumeration("FrPtrClass", T::u16(e), &pointer_classes())),
            ("instance", T::u32(e)),
        ],
    )
}

/// Every body this reader knows, by the name the specification gives the
/// class. This is the list both routes below choose from: the file's own
/// dictionary picks by the name it declares, and the constant table picks by
/// the number FrameL would have used.
fn bodies(e: Endian) -> Vec<(&'static str, T)> {
    vec![
        ("FrSH", frsh(e)),
        ("FrSE", frse(e)),
        ("FrameH", frame_h(e)),
        ("FrAdcData", adc_data(e)),
        ("FrDetector", detector(e)),
        ("FrEndOfFile", end_of_file(e)),
        ("FrEndOfFrame", end_of_frame(e)),
        ("FrEvent", event(e)),
        ("FrHistory", history(e)),
        ("FrMsg", msg(e)),
        ("FrProcData", proc_data(e)),
        ("FrRawData", raw_data(e)),
        ("FrSerData", ser_data(e)),
        ("FrSimData", sim_data(e)),
        ("FrSimEvent", sim_event(e)),
        ("FrStatData", stat_data(e)),
        ("FrSummary", summary(e)),
        ("FrTable", table(e)),
        ("FrTOC", toc(e)),
        ("FrVect", vect(e)),
    ]
}

/// The body of a structure, chosen by what this file says its class is.
///
/// Classes 1 and 2 are the two the specification fixes, and they are taken
/// first for a reason beyond the spec: FrSH is the structure the search below
/// reads, and letting a dictionary entry's own body be chosen by a search
/// through dictionary entries would make every one of them ask about every one
/// before it. In the GWOSC sample that is 151 of the 162 structures answered
/// without a search at all.
fn class_body(e: Endian) -> T {
    T::switch(E::field("class"), vec![(1, frsh(e)), (2, frse(e))], declared_body(e))
}

/// The body of everything else: whatever the FrSH earlier in this stream that
/// numbered itself with this structure's class byte calls it.
///
/// A file with no dictionary entry for the class answers with no name at all,
/// and the empty name is a case here rather than the default: it takes the
/// constant table. A file that names a class something this reader has no
/// layout for falls to the default and keeps its bytes.
fn declared_body(e: Endian) -> T {
    let declared = E::sibling_tagged(&["body", "class"], E::field("class"), &["body", "name", "text"]);
    let mut cases = bodies(e);
    cases.push(("", by_class_number(e)));
    T::matches(declared, cases, T::bytes(E::Remaining))
}

/// The fallback: the class numbers FrameL and FrameCPP assign, for a file that
/// never said. See the note at the top.
fn by_class_number(e: Endian) -> T {
    let named = bodies(e);
    let cases = CLASSES
        .iter()
        .filter_map(|(n, name)| named.iter().find(|(k, _)| k == name).map(|(_, t)| (*n, t.clone())))
        .collect();
    T::switch(E::field("class"), cases, T::bytes(E::Remaining))
}

/// A dictionary entry: this file calls class `class` by this name.
fn frsh(e: Endian) -> T {
    T::structure_named(
        "FrSH",
        "name",
        "",
        vec![
            ("name", string(e)),
            ("class", T::u16(e)),
            ("comment", string(e)),
            ("chkSum", T::u32(e)),
        ],
    )
    // The two together are the entry: this file calls class 4 `FrAdcData`.
    .payload(&["name", "class"])
}

/// One field of the class the FrSH before it named: what it is called and what
/// it is. The type is written as text, `INT_4U` or `REAL_8[nDim]`.
fn frse(e: Endian) -> T {
    T::structure_named(
        "FrSE",
        "name",
        "",
        vec![
            ("name", string(e)),
            ("type", string(e)),
            ("comment", string(e)),
            ("chkSum", T::u32(e)),
        ],
    )
}

fn frame_h(e: Endian) -> T {
    T::structure_named(
        "FrameH",
        "name",
        "",
        vec![
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
            ("chkSum", T::u32(e)),
        ],
    )
    .payload(&["GTimeS", "dt"])
}

fn detector(e: Endian) -> T {
    T::structure_named(
        "FrDetector",
        "name",
        "",
        vec![
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
            ("chkSum", T::u32(e)),
        ],
    )
}

fn proc_data(e: Endian) -> T {
    T::structure_named(
        "FrProcData",
        "name",
        "",
        vec![
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
            ("chkSum", T::u32(e)),
        ],
    )
}

/// One channel of the digitiser, as it came off the hardware: what it was
/// called, what one count of it is worth, and a pointer to the numbers.
///
/// `bias` and `slope` are what turn a count back into volts, and `nBits` how
/// many of the bits of a sample the converter actually set.
fn adc_data(e: Endian) -> T {
    T::structure_named(
        "FrAdcData",
        "name",
        "",
        vec![
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
            ("chkSum", T::u32(e)),
        ],
    )
    .payload(&["sampleRate", "slope", "units"])
}

/// Something a trigger found in the data: when, how big, and how sure.
///
/// The parameters are a list of doubles with a list of names beside it, which
/// is how one structure carries whatever a given search wanted to record.
fn event(e: Endian) -> T {
    let n = E::field("nParam");
    T::structure_named(
        "FrEvent",
        "name",
        "",
        vec![
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
            ("parameters", T::array(T::F64(e), n.clone())),
            ("parameterNames", T::array(string(e), n)),
            ("data", pointer(e)),
            ("table", pointer(e)),
            ("next", pointer(e)),
            ("chkSum", T::u32(e)),
        ],
    )
    .payload(&["GTimeS", "amplitude"])
}

/// A line of the record of what was done to this frame: a program, when it
/// ran, and what it said about itself.
fn history(e: Endian) -> T {
    T::structure_named(
        "FrHistory",
        "name",
        "",
        vec![
            ("name", string(e)),
            ("time", T::u32(e)),
            ("comment", string(e)),
            ("next", pointer(e)),
            ("chkSum", T::u32(e)),
        ],
    )
}

/// One line of the detector's log, kept in the frame so that what the
/// instrument was complaining about arrives with the data it was recording.
fn msg(e: Endian) -> T {
    T::structure_named(
        "FrMsg",
        "alarm",
        "",
        vec![
            ("alarm", string(e)),
            ("message", string(e)),
            ("severity", T::u32(e)),
            ("GTimeS", T::u32(e)),
            ("GTimeN", T::u32(e)),
            ("next", pointer(e)),
            ("chkSum", T::u32(e)),
        ],
    )
    .payload(&["message"])
}

/// The head of the raw data: five pointers at the lists of everything the
/// instrument itself wrote, and nothing of its own but a name.
fn raw_data(e: Endian) -> T {
    T::structure_named(
        "FrRawData",
        "name",
        "",
        vec![
            ("name", string(e)),
            ("firstSer", pointer(e)),
            ("firstAdc", pointer(e)),
            ("firstTable", pointer(e)),
            ("logMsg", pointer(e)),
            ("more", pointer(e)),
            ("chkSum", T::u32(e)),
        ],
    )
}

/// A slow channel that arrives as text: the station keeping, read off a serial
/// line, with the whole line kept as it came.
fn ser_data(e: Endian) -> T {
    T::structure_named(
        "FrSerData",
        "name",
        "",
        vec![
            ("name", string(e)),
            ("timeSec", T::u32(e)),
            ("timeNsec", T::u32(e)),
            ("sampleRate", T::F64(e)),
            ("data", string(e)),
            ("serial", pointer(e)),
            ("table", pointer(e)),
            ("next", pointer(e)),
            ("chkSum", T::u32(e)),
        ],
    )
    .payload(&["timeSec", "data"])
}

/// A channel that was made up rather than recorded: an injected signal, kept
/// beside the real data it was added to.
fn sim_data(e: Endian) -> T {
    T::structure_named(
        "FrSimData",
        "name",
        "",
        vec![
            ("name", string(e)),
            ("comment", string(e)),
            ("sampleRate", T::F64(e)),
            ("timeOffset", T::F64(e)),
            ("fShift", T::F64(e)),
            ("phase", T::F32(e)),
            ("data", pointer(e)),
            ("input", pointer(e)),
            ("table", pointer(e)),
            ("next", pointer(e)),
            ("chkSum", T::u32(e)),
        ],
    )
}

/// An event that was injected rather than found. The same shape as an FrEvent
/// without the fields that only mean something for a trigger.
fn sim_event(e: Endian) -> T {
    let n = E::field("nParam");
    T::structure_named(
        "FrSimEvent",
        "name",
        "",
        vec![
            ("name", string(e)),
            ("comment", string(e)),
            ("inputs", string(e)),
            ("GTimeS", T::u32(e)),
            ("GTimeN", T::u32(e)),
            ("timeBefore", T::F32(e)),
            ("timeAfter", T::F32(e)),
            ("amplitude", T::F32(e)),
            ("nParam", T::u16(e)),
            ("parameters", T::array(T::F64(e), n.clone())),
            ("parameterNames", T::array(string(e), n)),
            ("data", pointer(e)),
            ("table", pointer(e)),
            ("next", pointer(e)),
            ("chkSum", T::u32(e)),
        ],
    )
    .payload(&["GTimeS", "amplitude"])
}

/// Something about the detector that does not change every frame: a
/// calibration, valid between two times, with a version so a later one can
/// replace it.
fn stat_data(e: Endian) -> T {
    T::structure_named(
        "FrStatData",
        "name",
        "",
        vec![
            ("name", string(e)),
            ("comment", string(e)),
            ("representation", string(e)),
            ("timeStart", T::u32(e)),
            ("timeEnd", T::u32(e)),
            ("version", T::u32(e)),
            ("detector", pointer(e)),
            ("data", pointer(e)),
            ("table", pointer(e)),
            ("chkSum", T::u32(e)),
        ],
    )
    .payload(&["timeStart", "timeEnd", "version"])
}

/// A number worked out about a stretch of data rather than sampled from it:
/// what the test was, and a vector of what it came to.
fn summary(e: Endian) -> T {
    T::structure_named(
        "FrSummary",
        "name",
        "",
        vec![
            ("name", string(e)),
            ("comment", string(e)),
            ("test", string(e)),
            ("GTimeS", T::u32(e)),
            ("GTimeN", T::u32(e)),
            ("moments", pointer(e)),
            ("table", pointer(e)),
            ("next", pointer(e)),
            ("chkSum", T::u32(e)),
        ],
    )
}

/// A table: the column names here, and the columns themselves in the FrVect
/// chain `column` points at, one vector per column.
fn table(e: Endian) -> T {
    T::structure_named(
        "FrTable",
        "name",
        "",
        vec![
            ("name", string(e)),
            ("comment", string(e)),
            ("nColumn", T::u16(e)),
            ("nRow", T::u32(e)),
            ("columnName", T::array(string(e), E::field("nColumn"))),
            ("column", pointer(e)),
            ("next", pointer(e)),
            ("chkSum", T::u32(e)),
        ],
    )
    .payload(&["nRow", "nColumn"])
}

fn end_of_frame(e: Endian) -> T {
    T::structure(
        "FrEndOfFrame",
        vec![
            ("run", T::i32(e)),
            ("frame", T::u32(e)),
            ("GTimeS", T::u32(e)),
            ("GTimeN", T::u32(e)),
            ("chkSum", T::u32(e)),
        ],
    )
}

fn end_of_file(e: Endian) -> T {
    T::structure(
        "FrEndOfFile",
        vec![
            ("nFrames", T::u32(e)),
            ("nBytes", T::u64(e)),
            ("seekTOC", T::u64(e)),
            ("chkSumFrHeader", T::u32(e)),
            ("chkSum", T::u32(e)),
            ("chkSumFile", T::u32(e)),
        ],
    )
    .payload(&["nFrames", "nBytes"])
}

/// A vector: a name, how its numbers are packed, and the packed bytes. The
/// dimensions come after the data, which is why `nBytes` has to say how far it
/// runs rather than the shape working it out.
fn vect(e: Endian) -> T {
    let dims = E::field("nDim");
    T::structure_named(
        "FrVect",
        "name",
        "data",
        vec![
            ("name", string(e)),
            ("compress", compression(e)),
            ("type", T::enumeration("FrVectType", T::u16(e), VECT_TYPE)),
            ("nData", T::u64(e)),
            ("nBytes", T::u64(e)),
            ("data", T::sized(E::field("nBytes").at_most(E::Remaining), numbers(e))),
            ("nDim", T::u32(e)),
            ("nx", T::array(T::u64(e), dims.clone())),
            ("dx", T::array(T::F64(e), dims.clone())),
            ("startX", T::array(T::F64(e), dims.clone())),
            ("unitX", T::array(string(e), dims)),
            ("unitY", string(e)),
            ("next", pointer(e)),
            ("chkSum", T::u32(e)),
        ],
    )
    .payload(&["nData"])
}

/// What is inside a vector: the numbers themselves when nothing was packed,
/// and the packed bytes otherwise. Unpacking gzip or a zero-suppressed run is
/// not done here, so a compressed vector says what it is and keeps its bytes.
fn numbers(e: Endian) -> T {
    let n = E::field("nData");
    let each = |t: T| T::array(t, n.clone());
    T::switch(
        E::field("compress"),
        vec![(
            0,
            T::switch(
                E::field("type"),
                vec![
                    (0, each(T::Int { bits: 8, endian: e })),
                    (1, each(T::Int { bits: 16, endian: e })),
                    (2, each(T::F64(e))),
                    (3, each(T::F32(e))),
                    (4, each(T::i32(e))),
                    (5, each(T::Int { bits: 64, endian: e })),
                    (9, each(T::u16(e))),
                    (10, each(T::u32(e))),
                    (11, each(T::u64(e))),
                    (12, each(T::u8())),
                ],
                T::bytes(E::Remaining),
            ),
        )],
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
fn toc(e: Endian) -> T {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// Which way round a hand-built file writes its numbers.
    #[derive(Clone, Copy)]
    struct W(bool);

    impl W {
        fn u16(self, v: u16) -> Vec<u8> {
            if self.0 { v.to_le_bytes().into() } else { v.to_be_bytes().into() }
        }
        fn u32(self, v: u32) -> Vec<u8> {
            if self.0 { v.to_le_bytes().into() } else { v.to_be_bytes().into() }
        }
        fn u64(self, v: u64) -> Vec<u8> {
            if self.0 { v.to_le_bytes().into() } else { v.to_be_bytes().into() }
        }
        fn f32(self, v: f32) -> Vec<u8> {
            if self.0 { v.to_le_bytes().into() } else { v.to_be_bytes().into() }
        }
        fn f64(self, v: f64) -> Vec<u8> {
            if self.0 { v.to_le_bytes().into() } else { v.to_be_bytes().into() }
        }
        /// A string as the format writes one: a count that includes the nul.
        fn str(self, s: &str) -> Vec<u8> {
            let mut v = self.u16(s.len() as u16 + 1);
            v.extend_from_slice(s.as_bytes());
            v.push(0);
            v
        }
        fn ptr(self, class: u16, instance: u32) -> Vec<u8> {
            let mut v = self.u16(class);
            v.extend(self.u32(instance));
            v
        }
        /// A structure: its body wrapped in the fourteen-byte header.
        fn structure(self, class: u8, instance: u32, body: &[u8]) -> Vec<u8> {
            let mut v = self.u64(body.len() as u64 + 14);
            v.push(1); // checksummed
            v.push(class);
            v.extend(self.u32(instance));
            v.extend_from_slice(body);
            v
        }
    }

    /// A whole small frame file: the header, a dictionary entry and one of its
    /// fields, a frame header, an uncompressed vector of two doubles, a
    /// structure of a class nothing here knows, and the end of file record.
    fn file(little: bool) -> Vec<u8> {
        let w = W(little);
        let mut b = b"IGWD\0".to_vec();
        b.extend_from_slice(&[8, 1, 2, 4, 8, 4, 8]);
        b.extend(w.u16(0x1234));
        b.extend(w.u32(0x1234_5678));
        b.extend(w.u64(0x0123_4567_89ab_cdef));
        b.extend(w.f32(std::f32::consts::PI));
        b.extend(w.f64(std::f64::consts::PI));
        b.extend_from_slice(&[2, 1]); // FrameCPP, CRC

        let mut sh = w.str("FrameH");
        sh.extend(w.u16(3));
        sh.extend(w.str("Frame Header Structure"));
        sh.extend(w.u32(0));
        b.extend(w.structure(1, 0, &sh));

        let mut se = w.str("name");
        se.extend(w.str("STRING"));
        se.extend(w.str("Name of project"));
        se.extend(w.u32(0));
        b.extend(w.structure(2, 0, &se));

        let mut fh = w.str("H1:TEST");
        fh.extend(w.u32(-1i32 as u32)); // run
        fh.extend(w.u32(0)); // frame
        fh.extend(w.u32(0)); // dataQuality
        fh.extend(w.u32(1_126_259_447)); // GTimeS
        fh.extend(w.u32(0)); // GTimeN
        fh.extend(w.u16(36)); // ULeapS
        fh.extend(w.f64(32.0)); // dt
        for _ in 0..11 {
            fh.extend(w.ptr(0, 0));
        }
        fh.extend(w.ptr(20, 0)); // auxData points at the vector
        fh.extend(w.ptr(0, 0));
        fh.extend(w.u32(0));
        b.extend(w.structure(3, 0, &fh));

        let mut fv = w.str("strain");
        fv.extend(w.u16(0)); // uncompressed
        fv.extend(w.u16(2)); // REAL_8
        fv.extend(w.u64(2)); // nData
        fv.extend(w.u64(16)); // nBytes
        fv.extend(w.f64(1.5));
        fv.extend(w.f64(-2.5));
        fv.extend(w.u32(1)); // nDim
        fv.extend(w.u64(2)); // nx
        fv.extend(w.f64(0.000244140625)); // dx
        fv.extend(w.f64(0.0)); // startX
        fv.extend(w.str("s")); // unitX
        fv.extend(w.str("strain")); // unitY
        fv.extend(w.ptr(0, 0));
        fv.extend(w.u32(0));
        b.extend(w.structure(20, 0, &fv));

        // A class no dictionary here named: eight bytes that stay bytes.
        b.extend(w.structure(9, 0, &[0xaa; 8]));

        let mut eof = w.u32(1); // nFrames
        eof.extend(w.u64(0)); // nBytes
        eof.extend(w.u64(40)); // seekTOC
        eof.extend(w.u32(0));
        eof.extend(w.u32(0));
        eof.extend(w.u32(0));
        b.extend(w.structure(6, 0, &eof));
        b
    }

    /// The stream of structures, wherever the endianness switch put it.
    const STREAM: &[usize] = &[8, 7];

    fn at(path: &[usize]) -> Vec<usize> {
        STREAM.iter().copied().chain(path.iter().copied()).collect()
    }

    #[test]
    fn the_header_says_the_version_and_which_way_round_the_numbers_are() {
        let d = Document::new(MemSource(file(true)));
        let mut ev = Evaluator::new(gwf());
        assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::UInt(8));
        // The check words read as themselves, which is what makes them checks.
        assert_eq!(ev.node(&d, &[8, 0]).unwrap().value, Value::UInt(0x1234));
        assert_eq!(ev.node(&d, &[8, 2]).unwrap().value, Value::UInt(0x0123_4567_89ab_cdef));
        assert_eq!(ev.node(&d, &[8, 4]).unwrap().value, Value::Float(std::f64::consts::PI));
    }

    #[test]
    fn a_file_from_the_other_kind_of_machine_reads_the_same_way() {
        // Same file, every number the other way round. The check word is what
        // picks the arm, and everything under it agrees again.
        let d = Document::new(MemSource(file(false)));
        let mut ev = Evaluator::new(gwf());
        assert_eq!(ev.node(&d, &[8, 2]).unwrap().value, Value::UInt(0x0123_4567_89ab_cdef));
        assert_eq!(ev.node(&d, &at(&[3, 4, 5, 1])).unwrap().value, Value::Float(-2.5));
        assert_eq!(ev.node(&d, STREAM).unwrap().child_count, 6);
    }

    #[test]
    fn every_structure_in_the_stream_is_placed() {
        let bytes = file(true);
        let d = Document::new(MemSource(bytes.clone()));
        let mut ev = Evaluator::new(gwf());
        let stream = ev.node(&d, STREAM).unwrap();
        assert_eq!(stream.child_count, 6);
        // The last one ends exactly at the end of the file: every length in
        // the stream added up to the bytes that are there.
        let last = ev.node(&d, &at(&[5])).unwrap();
        assert_eq!(last.offset_bits + last.size_bits, bytes.len() as u64 * 8);
    }

    #[test]
    fn a_dictionary_entry_names_the_class_it_describes() {
        let d = Document::new(MemSource(file(true)));
        let mut ev = Evaluator::new(gwf());
        let sh = ev.node(&d, &at(&[0, 2])).unwrap();
        assert_eq!(sh.value, Value::Enum { raw: 1, name: Some("FrSH".into()), hex: false });
        // name, then the class number this file gave it.
        assert_eq!(ev.node(&d, &at(&[0, 4, 0, 1])).unwrap().value, Value::Str("FrameH".into()));
        assert_eq!(ev.node(&d, &at(&[0, 4, 1])).unwrap().value, Value::UInt(3));
        // And an element of it: what one field of a FrameH is called.
        assert_eq!(ev.node(&d, &at(&[1, 4, 1, 1])).unwrap().value, Value::Str("STRING".into()));
    }

    #[test]
    fn a_frame_header_reads_its_time_and_its_pointers() {
        let d = Document::new(MemSource(file(true)));
        let mut ev = Evaluator::new(gwf());
        assert_eq!(ev.node(&d, &at(&[2, 4, 4])).unwrap().value, Value::UInt(1_126_259_447));
        assert_eq!(ev.node(&d, &at(&[2, 4, 7])).unwrap().value, Value::Float(32.0));
        // A pointer is six bytes: a class and an instance.
        let aux = ev.node(&d, &at(&[2, 4, 19])).unwrap();
        assert_eq!(aux.size_bits, 48);
        assert_eq!(
            ev.node(&d, &at(&[2, 4, 19, 0])).unwrap().value,
            Value::Enum { raw: 20, name: Some("FrVect".into()), hex: false }
        );
    }

    #[test]
    fn an_uncompressed_vector_reads_as_the_numbers_its_type_names() {
        let d = Document::new(MemSource(file(true)));
        let mut ev = Evaluator::new(gwf());
        let data = ev.node(&d, &at(&[3, 4, 5])).unwrap();
        assert_eq!(data.type_name, "f64 le[]");
        assert_eq!(data.child_count, 2);
        assert_eq!(ev.node(&d, &at(&[3, 4, 5, 0])).unwrap().value, Value::Float(1.5));
        // The dimensions come after the data, so nBytes is what placed them.
        assert_eq!(ev.node(&d, &at(&[3, 4, 6])).unwrap().value, Value::UInt(1));
        assert_eq!(ev.node(&d, &at(&[3, 4, 10, 0, 1])).unwrap().value, Value::Str("s".into()));
    }

    #[test]
    fn a_compressed_vector_is_named_and_left_alone() {
        // The same vector with gzip written by the other kind of machine,
        // which is what every GWOSC file holds. The bytes stay bytes.
        let w = W(true);
        let mut b = file(true);
        let start = b.len();
        let mut fv = w.str("strain");
        fv.extend(w.u16(257));
        fv.extend(w.u16(2));
        fv.extend(w.u64(2));
        fv.extend(w.u64(6));
        fv.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        fv.extend(w.u32(0)); // nDim
        fv.extend(w.str("strain")); // unitY
        fv.extend(w.ptr(0, 0));
        fv.extend(w.u32(0));
        b.extend(w.structure(20, 1, &fv));
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(gwf());
        assert!(ev.node(&d, &at(&[6])).unwrap().offset_bits == start as u64 * 8);
        let kind = ev.node(&d, &at(&[6, 4, 1])).unwrap();
        assert_eq!(
            kind.value,
            Value::Enum { raw: 257, name: Some("gzip (little-endian words)".into()), hex: false }
        );
        let data = ev.node(&d, &at(&[6, 4, 5])).unwrap();
        assert_eq!((data.type_name.as_str(), data.size_bits), ("bytes[]", 6 * 8));
    }

    /// A dictionary entry: this file calls class `class` by this name.
    fn dictionary(w: W, name: &str, class: u16) -> Vec<u8> {
        let mut sh = w.str(name);
        sh.extend(w.u16(class));
        sh.extend(w.str(""));
        sh.extend(w.u32(0));
        w.structure(1, 0, &sh)
    }

    /// A file of three structures: a dictionary entry, and then one structure
    /// of the class it numbers holding `body`.
    fn declared(name: &str, class: u8, body: &[u8]) -> Vec<u8> {
        let w = W(true);
        let mut b = b"IGWD\0".to_vec();
        b.extend_from_slice(&[8, 1, 2, 4, 8, 4, 8]);
        b.extend(w.u16(0x1234));
        b.extend(w.u32(0));
        b.extend(w.u64(0));
        b.extend(w.f32(0.0));
        b.extend(w.f64(0.0));
        b.extend_from_slice(&[2, 1]);
        b.extend(dictionary(w, name, class as u16));
        b.extend(w.structure(class, 0, body));
        b
    }

    /// The point of reading the dictionary rather than a table of numbers.
    /// This file numbers `FrHistory` 40, which no library does, and the
    /// structure of class 40 reads as a history all the same.
    #[test]
    fn a_class_the_file_numbered_itself_reads_by_the_name_the_file_gave_it() {
        let w = W(true);
        let mut body = w.str("myProgram");
        body.extend(w.u32(1_126_259_447));
        body.extend(w.str("ran once"));
        body.extend(w.ptr(0, 0));
        body.extend(w.u32(0));
        let d = Document::new(MemSource(declared("FrHistory", 40, &body)));
        let mut ev = Evaluator::new(gwf());
        let b = ev.node(&d, &at(&[1, 4])).unwrap();
        assert_eq!(b.type_name, "FrHistory");
        assert_eq!(ev.node(&d, &at(&[1, 4, 0, 1])).unwrap().value, Value::Str("myProgram".into()));
        assert_eq!(ev.node(&d, &at(&[1, 4, 1])).unwrap().value, Value::UInt(1_126_259_447));
    }

    /// The other half of the same rule: a name this reader has no layout for
    /// keeps its bytes rather than falling back to whatever the number would
    /// have meant. Class 9 is `FrHistory` by the standard numbering, and this
    /// file says it is something else.
    #[test]
    fn a_class_named_something_this_reader_does_not_know_keeps_its_bytes() {
        let d = Document::new(MemSource(declared("FrGizmo", 9, &[0xaa; 8])));
        let mut ev = Evaluator::new(gwf());
        let body = ev.node(&d, &at(&[1, 4])).unwrap();
        assert_eq!((body.type_name.as_str(), body.size_bits), ("bytes[]", 8 * 8));
    }

    /// A file with no dictionary entry for the class falls back to the numbers
    /// FrameL assigns. `file()` declares only `FrameH`, so its class 9
    /// structure is read as the `FrHistory` that numbering makes it.
    #[test]
    fn a_class_no_dictionary_entry_covers_falls_back_to_the_standard_numbering() {
        let d = Document::new(MemSource(file(true)));
        let mut ev = Evaluator::new(gwf());
        let s = ev.node(&d, &at(&[4, 2])).unwrap();
        assert_eq!(s.value, Value::Enum { raw: 9, name: Some("FrHistory".into()), hex: false });
        assert_eq!(ev.node(&d, &at(&[4, 4])).unwrap().type_name, "FrHistory");
    }

    /// One of the eleven classes no sample here declares, read from the field
    /// list FrameL writes into a dictionary.
    #[test]
    fn an_adc_channel_reads_its_calibration() {
        let w = W(true);
        let mut body = w.str("H1:LSC-DARM");
        body.extend(w.str("darm"));
        body.extend(w.u32(3)); // channelGroup
        body.extend(w.u32(7)); // channelNumber
        body.extend(w.u32(16)); // nBits
        body.extend(w.f32(0.5)); // bias
        body.extend(w.f32(2.5)); // slope
        body.extend(w.str("counts"));
        body.extend(w.f64(16384.0)); // sampleRate
        body.extend(w.f64(0.0)); // timeOffset
        body.extend(w.f64(0.0)); // fShift
        body.extend(w.f32(0.0)); // phase
        body.extend(w.u16(0)); // dataValid
        for _ in 0..3 {
            body.extend(w.ptr(0, 0));
        }
        body.extend(w.u32(0));
        let d = Document::new(MemSource(declared("FrAdcData", 4, &body)));
        let mut ev = Evaluator::new(gwf());
        assert_eq!(ev.node(&d, &at(&[1, 4])).unwrap().type_name, "FrAdcData");
        assert_eq!(ev.node(&d, &at(&[1, 4, 6])).unwrap().value, Value::Float(2.5));
        assert_eq!(ev.node(&d, &at(&[1, 4, 8])).unwrap().value, Value::Float(16384.0));
        // The last field ends exactly where the structure does: every width in
        // the list between is right.
        let last = ev.node(&d, &at(&[1, 4, 16])).unwrap();
        let whole = ev.node(&d, &at(&[1])).unwrap();
        assert_eq!(last.offset_bits + last.size_bits, whole.offset_bits + whole.size_bits);
    }

    #[test]
    fn the_end_of_file_record_says_how_many_frames_there_were() {
        let d = Document::new(MemSource(file(true)));
        let mut ev = Evaluator::new(gwf());
        assert_eq!(ev.node(&d, &at(&[5, 4, 0])).unwrap().value, Value::UInt(1));
        assert_eq!(ev.node(&d, &at(&[5, 4, 2])).unwrap().value, Value::UInt(40));
    }
}
