//! IGWD frame files: the raw and processed data of the LIGO, Virgo and KAGRA
//! gravitational wave detectors. Specification LIGO-T970130, versions 6 and 8
//! of the format.
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
//! its own schema, and the first hundred and fifty structures of the GWOSC
//! sample are nothing but that schema.
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
//! The constant tables below are what is left when that question has no
//! answer: a file with no dictionary at all, or one whose dictionary does not
//! cover the class in hand. For version 8 it is the numbering FrameCPP
//! assigns, which is the order the specification lists the structures in; for
//! version 6 it is the numbering both version 6 samples use. A file that both
//! declares a class and calls it something this reader has never heard of
//! gets its bytes, which is the honest answer.
//!
//! There is no standard numbering to fall back on, which the samples showed.
//! In version 8, FrameCPP numbers `FrameH` 3; FrameL 8.30 numbers it 4 and its
//! `FrAdcData` 5; FrameL 8.20 numbers classes in the order it first wrote one,
//! so in the gwpy sample `FrVect` is 5. So the class byte names only the two
//! classes the
//! specification fixes, and `class_name` beside it reads as what this file's
//! own dictionary calls the number. The structure is labelled by that, and a
//! pointer's class stays a number: an enum of one library's numbering would
//! label a FrameL frame header as an ADC channel, and nothing on screen would
//! say that it had.
//!
//! What is read: the header, the structure stream, and every class the
//! specification defines, each checked field for field against a file's own
//! dictionary. `gwf_real.rs` walks each sample's FrSH and FrSE structures off
//! the bytes, without the template, and asks that every structure the
//! dictionary covers reads as the class it names, with the fields it lists in
//! that order, the last of them ending where the structure's length says.
//! The GWOSC file covers FrameH, FrDetector, FrProcData, FrVect, FrEndOfFrame,
//! FrTOC and FrEndOfFile. The other eleven (FrAdcData, FrEvent, FrHistory,
//! FrMsg, FrRawData, FrSerData, FrSimData, FrSimEvent, FrStatData, FrSummary,
//! FrTable) are covered by the test file FrameL ships, written by its
//! `exampleFull.c`, and their values are checked against what that program
//! says it put in them. Five files in all: GWOSC's, FrameL 8.30's, a FrameL
//! 8.20 frame from gwpy's tests, and two in version 6.
//!
//! **Vectors.** An FrVect's numbers are read as its `type` names them, all
//! thirteen types, complex pairs and counted strings included. A packed vector
//! is opened as far as a template can open it: gzip as a space of the numbers,
//! differences then gzip as a space of the differences, and zero suppression
//! as its block size and the packed bits. The last two are finished by
//! [`gwf_vect`](super::gwf_vect), which the value panel asks, and which
//! reports each step it took. The GWOSC strain opens as the same 131,072
//! doubles GWOSC's HDF5 file of the same 32 seconds holds; FrameL's
//! zero-suppressed shorts unpack to half the floats its example wrote beside
//! them. Only those two schemes have samples.
//!
//! **Versions.** Version 6 is read as fully as version 8, from two samples:
//! FrameL 6.24's copy of the same test file, which has every class but
//! FrStatData, and a 2003 LIGO calibration frame FrameCPP wrote, which
//! LALSuite keeps for its tests. What differs was found in their bytes and
//! their dictionaries and checked against both, not carried over from the
//! version 8 layout:
//!
//! - the file header ends in the letters `AZ` where version 8 has the library
//!   and the checksum scheme;
//! - a structure's class is two bytes, where version 8 has a checksum kind and
//!   a one-byte class, so the header is fourteen bytes in both;
//! - no body ends in a checksum, and FrEndOfFrame and FrEndOfFile carry the
//!   frame's and the file's instead, in a different order;
//! - `sampleRate` of FrSerData and FrSimData, and the parameters of FrEvent and
//!   FrSimEvent, are 4-byte floats;
//! - the table of contents has no totals, so its event columns are as long as
//!   the per-type counts add up to, and its static data is a list of groups.
//!
//! FrStatData in version 6 has no sample. Its fields are the specification's
//! version 6 table, which is version 8's without the checksum, and the table
//! of contents' static data groups are the specification's and FrameL 6.24's
//! writer's; neither has been read against bytes.
//!
//! Version 7 is bytes past the check words. The specification's revision
//! history lists the changes after version 6 without saying which of them
//! came with 7 and which with 8, one of them moves a byte into the structure
//! header, and no version 7 file is here. Versions before 6 are bytes the
//! same way.
//!
//! What else stays bytes: the body of a class this reader has no layout for,
//! and a vector packed with a scheme the specification does not list.

use crate::codec::Codec;
use crate::template::{Endian, Endian::*, Expr as E, Template, Ty as T, Until};

/// Which library wrote the file, from byte 38 of the header.
const LIBRARY: &[(i128, &str)] = &[(0, "unknown"), (1, "FrameLib"), (2, "FrameCPP")];

/// How the checksums in the file were worked out, from byte 39.
const CHECKSUM_SCHEME: &[(i128, &str)] = &[(0, "none"), (1, "CRC")];

/// What a structure's own checksum covers.
const CHECKSUM_KIND: &[(i128, &str)] = &[(0, "none"), (1, "structure")];

/// The two class numbers the specification fixes. Every other number is the
/// writer's to assign, so a name for it can only come from the file.
const FIXED_CLASSES: &[(i128, &str)] = &[(1, "FrSH"), (2, "FrSE")];

/// The class numbers FrameCPP assigns in a version 8 file, which is the order
/// the specification lists the structures in. Used only for a structure whose
/// class no dictionary entry in the file covers; see the note at the top.
///
/// Not FrameL's numbering, which is why it names nothing on screen. FrameL
/// 8.30 calls class 4 `FrameH` where FrameCPP calls it `FrAdcData`, and FrameL
/// 8.20 numbers classes in the order it first wrote one, so its `FrVect` is 5.
/// A row that read class 4 as `FrAdcData` above a frame header would be the
/// kind of wrong a reader has no way to catch.
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

/// The class numbers of a version 6 file, for the same fallback. Both version
/// 6 samples use them, one written by FrameL 6.24 and one by FrameCPP, for
/// every class either declares; `FrStatData` is in neither and has FrameL
/// 8.30's number, which uses this same numbering in version 8.
const V6_CLASSES: &[(i128, &str)] = &[
    (1, "FrSH"),
    (2, "FrSE"),
    (4, "FrameH"),
    (5, "FrAdcData"),
    (6, "FrDetector"),
    (7, "FrEndOfFrame"),
    (8, "FrEvent"),
    (9, "FrMsg"),
    (10, "FrHistory"),
    (11, "FrRawData"),
    (12, "FrProcData"),
    (13, "FrSimData"),
    (14, "FrSimEvent"),
    (15, "FrSerData"),
    (16, "FrStatData"),
    (17, "FrSummary"),
    (18, "FrTable"),
    (19, "FrTOC"),
    (20, "FrVect"),
    (21, "FrEndOfFile"),
];

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

/// The two layouts of the structures this reader has samples of.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Version {
    Six,
    Eight,
}

use Version::{Eight, Six};

/// Everything past the widths, read the way round the check word says, and
/// laid out the way the version says.
///
/// Version 7 is bytes past the check words. The specification's revision
/// history puts the move from 6 to 7 and the move to 8 in one list without
/// saying which change was which version's, and one of them moved a byte into
/// the structure header; no version 7 file is here to settle it.
fn rest(e: Endian) -> T {
    let checks = || {
        vec![
            ("check_int2", T::u16(e)),
            ("check_int4", T::u32(e)),
            ("check_int8", T::u64(e)),
            ("check_real4", T::F32(e)),
            ("check_real8", T::F64(e)),
        ]
    };
    let machinery = ["check_int2", "check_int4", "check_int8", "check_real4", "check_real8"];
    let mut eight = checks();
    eight.extend([
        ("library", T::enumeration("FrameLibrary", T::u8(), LIBRARY)),
        ("checksum_scheme", T::enumeration("ChecksumScheme", T::u8(), CHECKSUM_SCHEME)),
        ("structures", T::repeat(structure(e, Eight), Until::End)),
    ]);
    // Version 6 ends its header with the letters A and Z, for a reader to
    // check that the machine that wrote it spoke ASCII. Version 8 gave the
    // two bytes to the library and the checksum scheme.
    let mut six = checks();
    six.extend([("ascii_check", T::magic(b"AZ")), ("structures", T::repeat(structure(e, Six), Until::End))]);
    let mut other = checks();
    other.push(("rest", T::bytes(E::Remaining)));
    T::switch(
        E::field("version"),
        vec![
            (6, T::structure("FrameFileBody", six).machinery(&machinery)),
            (8, T::structure("FrameFileBody", eight).machinery(&machinery)),
        ],
        T::structure("FrameFileBody", other).machinery(&machinery),
    )
}

/// One structure: fourteen bytes of header and a body as long as the length
/// says.
///
/// Version 8 split the two-byte class of version 6 into a checksum kind and a
/// one-byte class, which is why the header is the same length in both.
fn structure(e: Endian, v: Version) -> T {
    let mut fields = vec![("length", T::u64(e))];
    match v {
        Eight => fields.extend([
            ("checksum_kind", T::enumeration("ChecksumKind", T::u8(), CHECKSUM_KIND)),
            ("class", T::enumeration("FrClass", T::u8(), FIXED_CLASSES)),
        ]),
        Six => fields.push(("class", T::enumeration("FrClass", T::u16(e), FIXED_CLASSES))),
    }
    fields.extend([
        ("class_name", class_name()),
        ("instance", T::u32(e)),
        ("body", T::sized(body_size(14), class_body(e, v))),
    ]);
    let machinery: &[&str] = match v {
        Eight => &["length", "checksum_kind", "instance"],
        Six => &["length", "instance"],
    };
    T::structure_named("FrStructure", "class_name", "body", fields).machinery(machinery)
}

/// What this file calls a structure's class. The number is in the file and the
/// word is in the file, in a dictionary entry further back, and before this
/// the reader was shown the number and left to go and find the entry
/// themselves. A row of no bytes beside the class says it, and it is what the
/// structure is labelled by.
///
/// FrSH and FrSE are the two classes no dictionary entry describes, since they
/// are the dictionary, so their names are the specification's and read off
/// the number.
fn class_name() -> T {
    let fixed = T::enumeration("FrClass", T::computed(E::field("class")), FIXED_CLASSES);
    T::switch(E::field("class"), vec![(1, fixed.clone()), (2, fixed)], T::computed_text(declared_name()))
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
///
/// The class is this file's own number, and it stays a number. The dictionary
/// entry that names it is often further on than the pointer is, since a frame
/// header points at the channels written after it, so no search back through
/// the stream can name it; and a table of one library's numbers would name
/// the other library's classes wrongly. See [`CLASSES`].
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
fn bodies(e: Endian, v: Version) -> Vec<(&'static str, T)> {
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

/// The body of a structure, chosen by what this file says its class is.
///
/// Classes 1 and 2 are the two the specification fixes, and they are taken
/// first for a reason beyond the spec: FrSH is the structure the search below
/// reads, and letting a dictionary entry's own body be chosen by a search
/// through dictionary entries would make every one of them ask about every one
/// before it. In the GWOSC sample that is 151 of the 162 structures answered
/// without a search at all.
fn class_body(e: Endian, v: Version) -> T {
    T::switch(E::field("class"), vec![(1, frsh(e, v)), (2, frse(e, v))], declared_body(e, v))
}

/// The name this file gives the class the asking structure carries: the `FrSH`
/// earlier in the stream that numbered itself with this structure's class byte,
/// and the name written in it. Empty when the file declared no such class.
fn declared_name() -> E {
    E::sibling_tagged(&["body", "class"], E::field("class"), &["body", "name", "text"])
}

/// The body of everything else: whatever the FrSH earlier in this stream that
/// numbered itself with this structure's class byte calls it.
///
/// A file with no dictionary entry for the class answers with no name at all,
/// and the empty name is a case here rather than the default: it takes the
/// constant table. A file that names a class something this reader has no
/// layout for falls to the default and keeps its bytes.
fn declared_body(e: Endian, v: Version) -> T {
    let declared = declared_name();
    let mut cases = bodies(e, v);
    cases.push(("", by_class_number(e, v)));
    T::matches(declared, cases, T::bytes(E::Remaining))
}

/// The fallback: a library's class numbers, for a file that never said. See
/// the note at the top.
fn by_class_number(e: Endian, v: Version) -> T {
    let named = bodies(e, v);
    let table = match v {
        Six => V6_CLASSES,
        Eight => CLASSES,
    };
    let cases = table
        .iter()
        .filter_map(|(n, name)| named.iter().find(|(k, _)| k == name).map(|(_, t)| (*n, t.clone())))
        .collect();
    T::switch(E::field("class"), cases, T::bytes(E::Remaining))
}

/// A dictionary entry: this file calls class `class` by this name.
fn frsh(e: Endian, v: Version) -> T {
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
fn frse(e: Endian, v: Version) -> T {
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
                ("statTypes", T::array(group, nstat)),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Role, Value};
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
        assert_eq!(ev.node(&d, &at(&[3, 5, 5, 1])).unwrap().value, Value::Float(-2.5));
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
        // The two classes the specification fixes are named off the number,
        // since no dictionary entry describes the dictionary.
        assert_eq!(ev.node(&d, &at(&[0, 3])).unwrap().value, Value::Enum { raw: 1, name: Some("FrSH".into()), hex: false });
        assert_eq!(ev.node(&d, &at(&[1, 3])).unwrap().value, Value::Enum { raw: 2, name: Some("FrSE".into()), hex: false });
        // And every other class by what the dictionary entry before it says.
        assert_eq!(ev.node(&d, &at(&[2, 3])).unwrap().value, Value::Str("FrameH".into()));
        // name, then the class number this file gave it.
        assert_eq!(ev.node(&d, &at(&[0, 5, 0, 1])).unwrap().value, Value::Str("FrameH".into()));
        assert_eq!(ev.node(&d, &at(&[0, 5, 1])).unwrap().value, Value::UInt(3));
        // And an element of it: what one field of a FrameH is called.
        assert_eq!(ev.node(&d, &at(&[1, 5, 1, 1])).unwrap().value, Value::Str("STRING".into()));
    }

    #[test]
    fn a_frame_header_reads_its_time_and_its_pointers() {
        let d = Document::new(MemSource(file(true)));
        let mut ev = Evaluator::new(gwf());
        assert_eq!(ev.node(&d, &at(&[2, 5, 4])).unwrap().value, Value::UInt(1_126_259_447));
        assert_eq!(ev.node(&d, &at(&[2, 5, 7])).unwrap().value, Value::Float(32.0));
        // A pointer is six bytes: a class and an instance.
        let aux = ev.node(&d, &at(&[2, 5, 19])).unwrap();
        assert_eq!(aux.size_bits, 48);
        // The file's own number for the class, left as a number: this file's
        // vector is class 20, and another writer's could be 5.
        assert_eq!(ev.node(&d, &at(&[2, 5, 19, 0])).unwrap().value, Value::Enum { raw: 20, name: None, hex: false });
        // Nothing pointed at is the one class a pointer names outright.
        assert_eq!(
            ev.node(&d, &at(&[2, 5, 8, 0])).unwrap().value,
            Value::Enum { raw: 0, name: Some("null".into()), hex: false }
        );
    }

    #[test]
    fn an_uncompressed_vector_reads_as_the_numbers_its_type_names() {
        let d = Document::new(MemSource(file(true)));
        let mut ev = Evaluator::new(gwf());
        let data = ev.node(&d, &at(&[3, 5, 5])).unwrap();
        assert_eq!(data.type_name, "f64 le[]");
        assert_eq!(data.child_count, 2);
        assert_eq!(ev.node(&d, &at(&[3, 5, 5, 0])).unwrap().value, Value::Float(1.5));
        // The dimensions come after the data, so nBytes is what placed them.
        assert_eq!(ev.node(&d, &at(&[3, 5, 6])).unwrap().value, Value::UInt(1));
        assert_eq!(ev.node(&d, &at(&[3, 5, 10, 0, 1])).unwrap().value, Value::Str("s".into()));
    }

    /// FrameL writes 256 on a vector it did not pack, which is the same
    /// nothing as 0 written by a little-endian machine.
    #[test]
    fn a_vector_marked_unpacked_by_a_little_endian_writer_reads_as_numbers() {
        let w = W(true);
        let mut b = file(true);
        let mut fv = w.str("names");
        fv.extend(w.u16(256));
        fv.extend(w.u16(8)); // STRING
        fv.extend(w.u64(2));
        let strings = [w.str("H1"), w.str("L1")].concat();
        fv.extend(w.u64(strings.len() as u64));
        fv.extend_from_slice(&strings);
        fv.extend(w.u32(0)); // nDim
        fv.extend(w.str("")); // unitY
        fv.extend(w.ptr(0, 0));
        fv.extend(w.u32(0));
        b.extend(w.structure(20, 1, &fv));
        let mut cx = w.str("response");
        cx.extend(w.u16(256));
        cx.extend(w.u16(6)); // COMPLEX_8
        cx.extend(w.u64(2));
        cx.extend(w.u64(16));
        for v in [0.5f32, -1.0, 2.0, 0.25] {
            cx.extend(w.f32(v));
        }
        cx.extend(w.u32(0));
        cx.extend(w.str(""));
        cx.extend(w.ptr(0, 0));
        cx.extend(w.u32(0));
        b.extend(w.structure(20, 2, &cx));
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(gwf());
        assert_eq!(ev.node(&d, &at(&[6, 5, 5, 1, 1])).unwrap().value, Value::Str("L1".into()));
        // A complex number is its real part and then its imaginary part.
        assert_eq!(ev.node(&d, &at(&[7, 5, 5])).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &at(&[7, 5, 5, 1, 0])).unwrap().value, Value::Float(2.0));
        assert_eq!(ev.node(&d, &at(&[7, 5, 5, 1, 1])).unwrap().value, Value::Float(0.25));
    }

    /// A vector whose data is `packed`, as the seventh structure of the file
    /// above, the whole file big-endian when `big` is set.
    fn packed_vector(big: bool, compress: u16, vect_type: u16, n: u64, packed: &[u8]) -> Vec<u8> {
        let w = W(!big);
        let mut b = file(!big);
        let mut fv = w.str("strain");
        fv.extend(w.u16(compress));
        fv.extend(w.u16(vect_type));
        fv.extend(w.u64(n));
        fv.extend(w.u64(packed.len() as u64));
        fv.extend_from_slice(packed);
        fv.extend(w.u32(0)); // nDim
        fv.extend(w.str("strain")); // unitY
        fv.extend(w.ptr(0, 0));
        fv.extend(w.u32(0));
        b.extend(w.structure(20, 1, &fv));
        b
    }

    /// Gzip, which is what every GWOSC vector holds, opens as a space of the
    /// numbers the vector's type names. The words inside are the way round the
    /// scheme says, which in a big-endian file packed on a little-endian
    /// machine is not the way round the file is.
    #[test]
    fn a_gzip_vector_opens_as_the_numbers_inside_it() {
        let plain: Vec<u8> = [1.5f64, -2.5].iter().flat_map(|v| v.to_le_bytes()).collect();
        let packed = miniz_oxide::deflate::compress_to_vec_zlib(&plain, 6);
        let d = Document::new(MemSource(packed_vector(true, 257, 2, 2, &packed)));
        let mut ev = Evaluator::new(gwf());
        let kind = ev.node(&d, &at(&[6, 5, 1])).unwrap();
        assert_eq!(kind.value, Value::Enum { raw: 257, name: Some("gzip (little-endian words)".into()), hex: false });
        let data = ev.node(&d, &at(&[6, 5, 5])).unwrap();
        assert_eq!(data.size_bits, packed.len() as u64 * 8);
        assert_eq!(ev.node(&d, &at(&[6, 5, 5, 0, 1])).unwrap().value, Value::Float(-2.5));
        // And the fields after the data are where nBytes put them.
        assert_eq!(ev.node(&d, &at(&[6, 5, 11, 1])).unwrap().value, Value::Str("strain".into()));
    }

    /// Zero suppression is marked for the side reader, which the panel asks.
    /// Its block size is the one field of it the template reads.
    #[test]
    fn a_zero_suppressed_vector_is_marked_for_the_side_reader() {
        let packed: Vec<u8> = [0x0003u16, 0x2D17, 0x37f8, 0x2963, 0x0025].iter().flat_map(|w| w.to_le_bytes()).collect();
        let d = Document::new(MemSource(packed_vector(false, 261, 1, 8, &packed)));
        let mut ev = Evaluator::new(gwf());
        let data = ev.node(&d, &at(&[6, 5, 5])).unwrap();
        assert_eq!(data.type_name, "ZeroSuppressed");
        assert_eq!(ev.node(&d, &at(&[6, 5, 5, 0])).unwrap().value, Value::UInt(3));
        match ev.explain(&d, &at(&[6, 5, 5, 1]), None).unwrap() {
            crate::eval::Explain::Hdf5Chunk { values, total, element_type, problem, .. } => {
                assert_eq!(problem, None);
                assert_eq!((element_type.as_str(), total), ("i16", 8));
                assert_eq!(values, ["82", "85", "85", "81", "80", "82", "84", "85"]);
            }
            other => panic!("{other:?}"),
        }
    }

    /// A stream that will not inflate is the bytes it is, not a guess.
    #[test]
    fn a_gzip_vector_that_will_not_inflate_keeps_its_bytes() {
        let d = Document::new(MemSource(packed_vector(false, 257, 2, 2, &[1, 2, 3, 4, 5, 6])));
        let mut ev = Evaluator::new(gwf());
        let data = ev.node(&d, &at(&[6, 5, 5])).unwrap();
        assert_eq!(data.size_bits, 6 * 8);
        assert_eq!(ev.node(&d, &at(&[6, 5, 6])).unwrap().value, Value::UInt(0));
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
        let b = ev.node(&d, &at(&[1, 5])).unwrap();
        assert_eq!(b.type_name, "FrHistory");
        assert_eq!(ev.node(&d, &at(&[1, 5, 0, 1])).unwrap().value, Value::Str("myProgram".into()));
        assert_eq!(ev.node(&d, &at(&[1, 5, 1])).unwrap().value, Value::UInt(1_126_259_447));
        // The class byte reads as the standard numbering, and the row beside it
        // reads as what this file actually calls that number: 40 is FrHistory
        // here and FrHistory is not 40 anywhere else.
        let class = ev.node(&d, &at(&[1, 2])).unwrap();
        assert_eq!(class.value.as_int(), Some(40));
        let named = ev.node(&d, &at(&[1, 3])).unwrap();
        assert_eq!(named.value, Value::Str("FrHistory".into()));
        // A field of no bits: a reading of what is already there, not a claim
        // that a byte exists.
        assert_eq!(named.size_bits, 0);
        assert_eq!(named.type_name, "computed text");
        // And the row says where the word came from: the dictionary entry that
        // numbered itself 40, not the class byte on its own.
        let seen: Vec<_> = ev.origins(&d, &at(&[1, 3])).unwrap().into_iter().map(|o| (o.role, o.label)).collect();
        // Both ends of it: the class byte that sent the search off, and the
        // dictionary entry it landed on.
        assert!(seen.contains(&(Role::Value, "class".to_string())), "{seen:?}");
        assert!(seen.iter().any(|(r, l)| *r == Role::Value && l.ends_with("].body.name.text")), "{seen:?}");
    }

    /// The other half of the same rule: a name this reader has no layout for
    /// keeps its bytes rather than falling back to whatever the number would
    /// have meant. Class 9 is `FrHistory` by the standard numbering, and this
    /// file says it is something else.
    #[test]
    fn a_class_named_something_this_reader_does_not_know_keeps_its_bytes() {
        let d = Document::new(MemSource(declared("FrGizmo", 9, &[0xaa; 8])));
        let mut ev = Evaluator::new(gwf());
        let body = ev.node(&d, &at(&[1, 5])).unwrap();
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
        assert_eq!(s.value, Value::Enum { raw: 9, name: None, hex: false });
        assert_eq!(ev.node(&d, &at(&[4, 5])).unwrap().type_name, "FrHistory");
        // No name is claimed for the class: the table is a guess at what the
        // number meant, and the body is the only place the guess shows.
        assert_eq!(ev.node(&d, &at(&[4, 3])).unwrap().value, Value::Str(String::new()));
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
        assert_eq!(ev.node(&d, &at(&[1, 5])).unwrap().type_name, "FrAdcData");
        assert_eq!(ev.node(&d, &at(&[1, 5, 6])).unwrap().value, Value::Float(2.5));
        assert_eq!(ev.node(&d, &at(&[1, 5, 8])).unwrap().value, Value::Float(16384.0));
        // The last field ends exactly where the structure does: every width in
        // the list between is right.
        let last = ev.node(&d, &at(&[1, 5, 16])).unwrap();
        let whole = ev.node(&d, &at(&[1])).unwrap();
        assert_eq!(last.offset_bits + last.size_bits, whole.offset_bits + whole.size_bits);
    }

    /// A version 6 file: the header ends in `AZ`, a structure's class is two
    /// bytes where version 8 has a checksum kind and a one-byte class, and no
    /// body ends in a checksum.
    fn six(serial: &[u8]) -> Vec<u8> {
        let w = W(true);
        let mut b = b"IGWD\0".to_vec();
        b.extend_from_slice(&[6, 9, 2, 4, 8, 4, 8]);
        b.extend(w.u16(0x1234));
        b.extend(w.u32(0x1234_5678));
        b.extend(w.u64(0x0123_4567_89ab_cdef));
        b.extend(w.f32(std::f32::consts::PI));
        b.extend(w.f64(std::f64::consts::PI));
        b.extend_from_slice(b"AZ");
        let structure = |class: u16, body: &[u8]| {
            let mut v = w.u64(body.len() as u64 + 14);
            v.extend(w.u16(class));
            v.extend(w.u32(0));
            v.extend_from_slice(body);
            v
        };
        let mut sh = w.str("FrSerData");
        sh.extend(w.u16(15));
        sh.extend(w.str(""));
        b.extend(structure(1, &sh));
        b.extend(structure(15, serial));
        b
    }

    #[test]
    fn a_version_6_file_reads_its_narrower_fields_and_no_checksums() {
        let w = W(true);
        let mut ser = w.str("sms1");
        ser.extend(w.u32(600_000_000)); // timeSec
        ser.extend(w.u32(0)); // timeNsec
        ser.extend(w.f32(1.0)); // sampleRate, four bytes wide here
        ser.extend(w.str("sms data are here"));
        for _ in 0..3 {
            ser.extend(w.ptr(0, 0));
        }
        let d = Document::new(MemSource(six(&ser)));
        let mut ev = Evaluator::new(gwf());
        assert_eq!(ev.node(&d, &[8, 5]).unwrap().name, "ascii_check");
        assert_eq!(ev.node(&d, &[8, 6]).unwrap().child_count, 2);
        // The dictionary entry is three fields, with no checksum after them.
        assert_eq!(ev.node(&d, &[8, 6, 0, 4]).unwrap().child_count, 3);
        let body = [8, 6, 1, 4];
        assert_eq!(ev.node(&d, &body).unwrap().type_name, "FrSerData");
        assert_eq!(ev.node(&d, &[8, 6, 1, 2]).unwrap().value, Value::Str("FrSerData".into()));
        assert_eq!(ev.node(&d, &[8, 6, 1, 4, 3]).unwrap().size_bits, 32);
        assert_eq!(ev.node(&d, &[8, 6, 1, 4, 4, 1]).unwrap().value, Value::Str("sms data are here".into()));
        assert_eq!(ev.node(&d, &body).unwrap().child_count, 8);
    }

    /// Nothing here settles version 7's layout, so past the check words it is
    /// the bytes it is.
    #[test]
    fn a_version_7_file_is_bytes_past_its_check_words() {
        let mut b = six(&[]);
        b[5] = 7;
        let len = b.len() as u64;
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(gwf());
        let rest = ev.node(&d, &[8, 5]).unwrap();
        assert_eq!((rest.name.as_str(), rest.offset_bits + rest.size_bits), ("rest", len * 8));
    }

    #[test]
    fn the_end_of_file_record_says_how_many_frames_there_were() {
        let d = Document::new(MemSource(file(true)));
        let mut ev = Evaluator::new(gwf());
        assert_eq!(ev.node(&d, &at(&[5, 5, 0])).unwrap().value, Value::UInt(1));
        assert_eq!(ev.node(&d, &at(&[5, 5, 2])).unwrap().value, Value::UInt(40));
    }
}
