//! NASA CDF: the file a space physics mission is published as.
//!
//! MMS, Cluster, Voyager, Parker Solar Probe: what comes down from CDAWeb is
//! one of these. It shares the letters CDF with NetCDF classic and nothing
//! else; that one is `netcdf`, and it announces itself with `CDF` and a
//! version byte rather than with the number below.
//!
//! Eight bytes of signature, and then records. Every record says how long it
//! is and what kind it is, and the kinds refer to each other by offset from
//! the start of the file rather than by lying next to each other. So the
//! structure is a walk: the descriptor record at the front points at the
//! global descriptor, which points at the head of three chains, and each chain
//! is a record holding the offset of the next one of its kind.
//!
//! The chains are the variables, twice over, and the attributes. A variable
//! descriptor names a variable, says what type its values are and how they are
//! shaped, and points at the index that finds its values. There are two lists
//! of them because CDF has two kinds of variable: the rVariables, which all
//! share one shape declared once in the global descriptor, and the zVariables,
//! which each carry their own. Everything written this century is a zVariable.
//! An attribute descriptor names an attribute and heads a chain of entries,
//! one per variable it has been set on, or one for the file as a whole.
//!
//! **The values.** A variable points at an index of index records, each entry
//! of which says which records of the variable it covers and where their bytes
//! are. Those bytes are read here as the numbers they are: one row per record
//! of the variable, and inside it that record's values, so a record of a
//! three-vector of floats reads as three floats. How many values that is, is
//! the block's own room divided by how many records it holds and by how wide
//! one value is, rather than the variable's dimensions multiplied together: a
//! dimension a variable says it does not vary along is one value repeated and
//! is not written at all, so the shape has more numbers in it than the block
//! has values. An attribute entry's value and a variable's pad value are read
//! the same way, by the type each of them names.
//!
//! How to read any of them depends on the encoding named in the descriptor
//! record at the front of the file, which may be any of a dozen machines'
//! byte orders, and reading them the wrong way round would answer with numbers
//! no instrument measured. So the whole set of value types is switched on that
//! field, one arm per byte order, the way `gwf` and `elf` switch on theirs. The
//! records themselves, and every offset in them, are big-endian whatever the
//! encoding says.
//!
//! Five of those encodings are VAX floating point rather than IEEE: VAX itself
//! and the four Alpha and Itanium VMS ones, which are the same D and G formats
//! the VAX had. They are read here at the right width and the wrong layout, so
//! the encoding is named and the numbers under it are wrong. `mat` says the
//! same about level 4 written on a VAX, and for the same reason: no file like
//! that has been seen this century. CDF has no Cray encoding at all.
//!
//! A block of values may be compressed, in which case the block is a record of
//! its own holding a stream. CDF squeezes with gzip, with a run-length coding
//! of zeroes, or with one of two Huffman codings (see
//! [`crate::codec::cdfhuff`]). Which of the four is in the variable's own
//! compression parameters rather than in the block, so what is asked here is
//! the stream: a gzip member opens with two bytes that say so, and the other
//! three open with nothing in particular. A block packed one of those other
//! ways keeps its bytes.
//!
//! A compressed file says so in its second word and holds one compressed
//! record, which is the whole of the uncompressed file squeezed. That is
//! unpacked and the chains inside it are walked, so such a file reads as the
//! file it holds. What comes out of the stream is everything after the eight
//! bytes of signature, while every offset inside it still counts from the
//! file's first byte, so the walk is taken back by those eight bytes: that is
//! what `bytes_before` is, and it is nought everywhere else. The compression
//! parameters are a record of their own that the compressed record points at,
//! and they are what says which codec to open it with.
//!
//! Version 2.x is the same format at half the width: every offset and every
//! record size is 32 bits rather than 64 and every name is 64 bytes rather
//! than 256, and not one field moved otherwise. So the layouts here are
//! written once and built twice, and a file from the 1990s reads the same way
//! a file from last week does, values and all. A version 2.6 file signs itself;
//! anything older opens with the word that means "not compressed", twice over,
//! and says nothing about what it is until the record behind that.
//!
//! One thing did move. A file written before version 2.5 leaves 128 bytes of
//! nothing in the middle of every variable descriptor, which the library calls
//! wasted space; the release in the descriptor record at the front is what
//! says whether it is there.
//!
//! One more thing belongs to no record: a file whose flags say it is checksummed
//! keeps sixteen bytes of MD5 at the very end, after everything the chains
//! reach.
//!
//! What is still not read. A whole file opens with any of the four codings,
//! because its compression parameters are in hand where the stream starts; a
//! block of values squeezed with anything but gzip keeps its bytes, since the
//! run-length and both Huffman codings open with no signature to be told by
//! and the parameters that name them belong to the variable, not the block.
//! Nor are a sparse variable's missing records worked out from its pad value
//! or the record before. A record's values are a flat run, in the order the
//! file wrote them: how to fold them into the variable's shape is what the
//! majority flag and the dimension variances say, and doing that folding is a
//! reader's job rather than this one's.
//!
//! **Times.** CDF has three, and each is declared a moment wherever a value of
//! it is read: a variable's values, its pad value, and an attribute entry such
//! as the FILLVAL or VALIDMIN beside it.
//!
//! A CDF_EPOCH is milliseconds from 0000-01-01 in a double, every day 86,400
//! seconds long. A CDF_EPOCH16 is two doubles, whole seconds from the same
//! instant and picoseconds within the second, and its seconds are the moment,
//! with the picoseconds on the row beside them. Both types' fill value,
//! -1.0E31, and pad value, 0.0, read as no time.
//!
//! A CDF_TIME_TT2000 is nanoseconds in an `int8` from noon on 2000-01-01 in
//! Terrestrial Time, and it counts the leap seconds UTC inserts, so it becomes
//! a UTC date only through the table of them: counted as if there were none,
//! from the right zero, a time from after 2016 would be five seconds out, and
//! from noon on 2000-01-01 in UTC, 69.184 seconds. So it is an
//! [`Atomic`](crate::template::Atomic) epoch, a moment inside a leap second
//! reads as the `23:59:60` it was, and a moment after the table's last day
//! says so. Its fill value, the most negative `int8`, and its pad value, the
//! one after, read as no time; as counts both would be dates in 1707.
//!
//! The global descriptor records which table the writing library had:
//! `leap_second_last_updated` is the day it was last changed, or -1 from a
//! library older than 3.6. It is read and not yet used. A library whose table
//! stopped before a leap second wrote every later time a second away from what
//! the table here reads it as, and nothing yet says so.

use crate::codec::Codec;
use crate::template::{
    Anchor, Encoding,
    Endian::{self, Big, Little},
    Expr as E, StrLen, Template, Time, Ty as T,
};

/// What a version 3 file starts with.
pub const MAGIC: &[u8] = &[0xCD, 0xF3, 0x00, 0x01];

/// What a version 2.5 file and anything older starts with: the word that means
/// "not compressed", written where the version 3 signature goes and again
/// after it. Nothing in those eight bytes says the file is a CDF at all.
pub const MAGIC_V2: &[u8] = &[0x00, 0x00, 0xFF, 0xFF, 0x00, 0x00, 0xFF, 0xFF];

/// What a version 2.6 file starts with, which is the first release to give the
/// format a signature of its own.
pub const MAGIC_V26: &[u8] = &[0xCD, 0xF2, 0x60, 0x02];

/// The kinds of record. Every one of them is a fixed layout after the size and
/// the type, and the ones this reads are the ones that say where something is.
const RECORD_TYPE: &[(i128, &str)] = &[
    (-1, "unused"),
    (1, "descriptor"),
    (2, "global descriptor"),
    (3, "rVariable descriptor"),
    (4, "attribute descriptor"),
    (5, "attribute g/rEntry"),
    (6, "variable index"),
    (7, "variable values"),
    (8, "zVariable descriptor"),
    (9, "attribute zEntry"),
    (10, "compressed"),
    (11, "compression parameters"),
    (12, "sparseness parameters"),
    (13, "compressed variable values"),
];

/// What the numbers in a variable or an attribute entry are. Every one of
/// these is read as the number it names; see [`number`] for the widths.
const DATA_TYPE: &[(i128, &str)] = &[
    (1, "int1"),
    (2, "int2"),
    (4, "int4"),
    (8, "int8"),
    (11, "uint1"),
    (12, "uint2"),
    (14, "uint4"),
    (21, "real4"),
    (22, "real8"),
    (31, "epoch"),
    (32, "epoch16"),
    (33, "tt2000"),
    (41, "byte"),
    (44, "float"),
    (45, "double"),
    (51, "char"),
    (52, "uchar"),
];

/// Whose byte order and floating-point format the values are in. This is the
/// one thing in the file that is not big-endian: the records are, and the
/// numbers inside a variable are whatever this says.
const ENCODING: &[(i128, &str)] = &[
    (1, "network"),
    (2, "Sun"),
    (3, "VAX"),
    (4, "DECstation"),
    (5, "SGi"),
    (6, "IBM PC"),
    (7, "IBM RS"),
    (8, "host"),
    (9, "PPC"),
    (11, "HP"),
    (12, "NeXT"),
    (13, "Alpha OSF/1"),
    (14, "Alpha VMS d"),
    (15, "Alpha VMS g"),
    (16, "Alpha VMS i"),
    (17, "ARM little"),
    (18, "ARM big"),
    (19, "IA64 VMS i"),
    (20, "IA64 VMS d"),
    (21, "IA64 VMS g"),
];

/// Who an attribute belongs to. An assumed scope is one the writer did not say
/// outright and the library worked out from what it was set on.
const SCOPE: &[(i128, &str)] = &[
    (1, "global"),
    (2, "variable"),
    (3, "global assumed"),
    (4, "variable assumed"),
];

/// The encodings whose numbers are written most significant byte first.
const BIG_ENDIAN: &[i128] = &[1, 2, 5, 7, 9, 10, 11, 12, 18];

/// The encodings whose numbers are written least significant byte first. The
/// five VAX ones are here too: their integers really are this way round, and
/// their floats are a layout nothing here reads. See the module doc.
const LITTLE_ENDIAN: &[i128] = &[3, 4, 6, 13, 14, 15, 16, 17, 19, 20, 21];

/// How a compressed record was squeezed.
const COMPRESSION: &[(i128, &str)] = &[(1, "run-length"), (2, "Huffman"), (3, "adaptive Huffman"), (5, "gzip")];

/// What a gzip member starts with, which is how a compressed block of values
/// says it is one. The record that names the codec belongs to the variable and
/// not to the block, so this is what the block itself has to be asked.
const GZIP_MAGIC: i128 = 0x1f8b;

fn i32be() -> T {
    T::i32(Big)
}

fn data_type() -> T {
    T::enumeration("CdfDataType", i32be(), DATA_TYPE)
}

/// Which of the two shapes a file's records are written in.
///
/// Version 3 widened every offset and every record size to sixty-four bits and
/// every name to 256 bytes; before that they were thirty-two bits and 64. Not
/// one field moved otherwise, so the layouts below are written once and built
/// twice, which is also how it reads: a version 2 file is the same format on a
/// machine with less disk.
#[derive(Clone, Copy)]
struct Shape {
    /// What the record type is called in the template's table.
    record: &'static str,
    /// Whether offsets, record sizes and names are the wide ones.
    wide: bool,
}

/// Version 3, which is everything written since 2004.
const V3: Shape = Shape { record: "CdfRecord", wide: true };
/// Version 2.x, which is everything before it.
const V2: Shape = Shape { record: "Cdf2Record", wide: false };

impl Shape {
    /// An offset into the file, which is what every link in this format is.
    /// Zero means there is nothing there, and so does all ones.
    ///
    /// Signed in both widths, because that is how the library declares them
    /// and how it writes the two sentinels: a version 2 offset is an `Int32`
    /// and the one that means nothing is -1.
    fn offset(self) -> T {
        T::Int { bits: if self.wide { 64 } else { 32 }, endian: Big }
    }

    /// How many bytes one of those offsets takes.
    fn offset_bytes(self) -> i128 {
        if self.wide { 8 } else { 4 }
    }

    /// A name, in the fixed room the version gives one, padded with nuls.
    fn name(self) -> T {
        T::text(StrLen::Padded { size: E::lit(if self.wide { 256 } else { 64 }), pad: 0 }, Encoding::Ascii)
    }

    /// A count of bytes: how long a record is, how long a stream is, how large
    /// something was before it was squeezed.
    fn size(self) -> T {
        if self.wide { T::u64(Big) } else { T::u32(Big) }
    }

    /// How long a record's own header is: its size and its type.
    fn header(self) -> i128 {
        if self.wide { 12 } else { 8 }
    }

    /// The 128 bytes of nothing in the middle of a variable descriptor written
    /// before version 2.5, and no bytes at all anywhere else.
    ///
    /// The library calls it wasted space and reads past it by the version in
    /// the descriptor record at the front of the file, which is what this asks
    /// too. Without it every field after it in such a file, the variable's own
    /// name included, is read 128 bytes early.
    fn wasted(self) -> T {
        match self.wide {
            true => T::bytes(E::lit(0)),
            false => T::switch(
                E::field("release").less_than(E::lit(5)),
                vec![(1, T::bytes(E::lit(128)))],
                T::bytes(E::lit(0)),
            ),
        }
    }

    /// The chain of records that starts at `field` and runs on through the
    /// `next` pointer inside each record.
    ///
    /// Every list in a CDF is written this way: no count, no table, just a
    /// head offset and a forward pointer in every record. Written as a record
    /// holding the record after it, which is the only shape there was before
    /// [`Ty::Chain`](crate::template::Ty::Chain), a file with two hundred
    /// attributes is a tree two hundred levels deep, and the two hundredth
    /// attribute sits behind two hundred rows the reader has to open one at a
    /// time. It is a list, and this says so.
    fn chain_of(self, field: &str, next: &str) -> T {
        T::chain_adjusted(
            E::field(field),
            &["body", next],
            Anchor::Origin,
            E::lit(0).sub(E::field("bytes_before")),
            T::Named(self.record.into()),
        )
    }

    /// The record `field` points at, or nothing where it holds no offset.
    /// Every chain and every pointer in the file ends this way rather than
    /// with a count.
    ///
    /// Nought is how the format says there is nothing there, and the library
    /// also writes all ones in a field it has no use for: a variable that is
    /// neither compressed nor sparse leaves -1 where the parameters would be
    /// pointed at.
    fn at_record(self, field: &str) -> T {
        T::switch(
            E::field(field),
            vec![(0, T::bytes(E::lit(0))), (-1, T::bytes(E::lit(0)))],
            T::at_origin(from_file_start(field), T::Named(self.record.into())),
        )
    }
}

/// Where in the bytes to hand an offset lands. Every offset in a CDF counts
/// from the first byte of the file, and the bytes being read are not always
/// the file: the whole of a compressed one unpacks into a run whose first byte
/// is the file's ninth. `bytes_before` is how many of the file's bytes come in
/// front of these, and it is nought for a file read where it lies.
fn from_file_start(field: &str) -> E {
    E::field(field).sub(E::field("bytes_before"))
}

/// One value of the type numbered `code`, read the way `e` says. Nothing for
/// the two character types, which are text and are shaped by whoever is asking
/// rather than by the type alone, and nothing for a number no version of CDF
/// has.
///
/// `real4` and `float` are the same thing under two names, as are `real8` and
/// `double`, and `byte` is `int1`: the second name of each pair is what the
/// 1990s library called it and both are still written. An epoch is a count of
/// milliseconds in a float and an epoch16 is a pair of them, seconds and
/// picoseconds; a TT2000 is a count of nanoseconds in an `int8`.
///
/// The number is the same whether or not it is a time, so the times are
/// declared where the numbers are used rather than here, which is a field
/// holding them: see [`time_of_type`]. The epoch16 is the exception, being a
/// structure of its own, so its seconds carry their declaration with them.
fn number(code: i128, e: Endian) -> Option<T> {
    Some(match code {
        1 | 41 => T::Int { bits: 8, endian: e },
        2 => T::Int { bits: 16, endian: e },
        4 => T::Int { bits: 32, endian: e },
        8 | 33 => T::Int { bits: 64, endian: e },
        11 => T::UInt { bits: 8, endian: e },
        12 => T::UInt { bits: 16, endian: e },
        14 => T::UInt { bits: 32, endian: e },
        21 | 44 => T::F32(e),
        22 | 31 | 45 => T::F64(e),
        32 => T::structure("CdfEpoch16", vec![("seconds", T::F64(e)), ("picoseconds", T::F64(e))])
            .field_time("seconds", Time::cdf_epoch16_seconds()),
        _ => return None,
    })
}

/// The data types whose numbers are moments, other than the epoch16, which
/// declares its own. See [`time_of_type`].
const TIME_TYPES: &[i128] = &[31, 33];

/// The moment a value of this data type is, if it is one.
///
/// A CDF_EPOCH is milliseconds from the year 0 in a double; see
/// [`Time::cdf_epoch`]. A CDF_TIME_TT2000 is nanoseconds from J2000 on a clock
/// that counts leap seconds; see [`Time::tt2000`]. Both with the fill and pad
/// values CDF gives them.
fn time_of_type(code: i128) -> Option<Time> {
    match code {
        31 => Some(Time::cdf_epoch()),
        33 => Some(Time::tt2000()),
        _ => None,
    }
}

/// A record whose value is typed by its own `data_type`, built once for each
/// data type that is a moment and once for everything else.
///
/// A time is declared on a field, and the field that holds an attribute
/// entry's value or a variable's pad value is the same field whatever the
/// type: which of a dozen numbers it holds is a switch inside it. So the choice
/// is made one level out, on the record, by the four bytes of `data_type`
/// `at` bytes into it, and the copies differ in nothing but the declaration.
/// The other way to say it, a structure of one field around each time-typed
/// value, would put a row in the field tree above every FILLVAL and VALIDMIN
/// that nothing in the file corresponds to.
fn by_time_type(at: i128, build: impl Fn(Option<Time>) -> T) -> T {
    let cases = TIME_TYPES.iter().map(|code| (*code, build(time_of_type(*code)))).collect();
    T::switch(E::peek_at(E::lit(at * 8), 32, Big), cases, build(None))
}

/// Whether this data type is characters rather than numbers.
fn is_text(code: i128) -> bool {
    code == 51 || code == 52
}

/// A switch over every data type, with `shape` saying what a field of that
/// type looks like: the element for a number, the width for text.
///
/// The three places values are read all have the same list of types and
/// differently shaped fields around them, which is what this collects. A type
/// number no version of CDF has keeps its bytes rather than reading as
/// nothing.
fn by_data_type(e: Endian, shape: impl Fn(i128, Option<T>) -> T) -> T {
    let cases = DATA_TYPE.iter().map(|(code, _)| (*code, shape(*code, number(*code, e)))).collect();
    T::switch(E::field("data_type"), cases, T::bytes(E::Remaining))
}

/// The same set of value types twice over, once for each byte order, picked by
/// the encoding the descriptor record at the front of the file names.
///
/// This is the shape `gwf` and `elf` use for the same question: one arm per
/// byte order, each holding the whole of what depends on it. The encoding is
/// reached outwards from wherever the values are, which is however many
/// records away the chains led: a variable's numbers are in a block pointed at
/// by an index pointed at by a descriptor, and the byte order is still the
/// file's.
///
/// An encoding this has never heard of, and `host`, which is a file saying its
/// numbers are in the order of a machine it does not name, read as network
/// order: that is what the format's own default is and the only answer left.
fn by_encoding(build: fn(Endian) -> T) -> T {
    let (big, little) = (build(Big), build(Little));
    let mut cases: Vec<(i128, T)> = BIG_ENDIAN.iter().map(|c| (*c, big.clone())).collect();
    cases.extend(LITTLE_ENDIAN.iter().map(|c| (*c, little.clone())));
    T::switch(E::field("encoding"), cases, big)
}

/// The value an attribute entry ends with: `num_elements` numbers, or that
/// many characters where the type is text.
fn entry_value(e: Endian) -> T {
    by_data_type(e, |code, number| match (is_text(code), number) {
        // A string attribute is one run of characters as wide as the record
        // has room for, which is the same as the count it declares.
        (true, _) => T::text(StrLen::Padded { size: E::Remaining, pad: 0 }, Encoding::Ascii),
        (_, Some(n)) => T::array(n, E::field("num_elements").at_least(E::lit(0))),
        (_, None) => T::bytes(E::Remaining),
    })
}

/// A variable's pad value: one value of its type, and nothing at all where the
/// flags said there is no pad value and the record stops.
fn pad_value(e: Endian) -> T {
    let one = by_data_type(e, |code, number| match (is_text(code), number) {
        (true, _) => T::text(StrLen::Padded { size: E::Remaining, pad: 0 }, Encoding::Ascii),
        (_, Some(n)) => n,
        (_, None) => T::bytes(E::Remaining),
    });
    T::switch(E::Remaining, vec![(0, T::bytes(E::lit(0)))], one)
}

/// Every value one block holds: a row per record of the variable, and inside
/// it the values of that record.
///
/// How many records the block covers is in the index entry that pointed at it,
/// which wrote the first and the last it holds. How many values are in one of
/// them is then the room divided out, and that is deliberate rather than the
/// obvious thing, which would be to multiply the variable's dimensions
/// together: a dimension a variable says it does not vary along is one value
/// repeated and is not written at all, so the shape has more numbers in it
/// than the block has values. `cacsst2.cdf` is a file of exactly that, four
/// two-dimensional rVariables of which three vary along one dimension each,
/// and multiplying the shape out reads ninety-one values as eighteen thousand
/// and off the end of the record.
///
/// So what the block is asked is how much room it has, which is a fact about
/// the block, and the shape stays what it is: a fact about the variable, in
/// the variable's own descriptor, for a reader to fold these values into.
/// A block with room left over after the division has that room as a gap,
/// which is the honest answer to bytes no value covers.
fn value_records(e: Endian) -> T {
    // At least one, because the division below is by this. An index entry
    // covering no records is a broken entry, not a reason to fail.
    let records = E::field("to_record").sub(E::field("from_record")).add(E::lit(1)).at_least(E::lit(1));
    by_data_type(e, move |code, number| {
        let (one, width) = match (is_text(code), number) {
            // A character variable's value is `num_elems` characters, which is
            // what makes a string an element rather than a dimension.
            (true, _) => (
                T::text(StrLen::Padded { size: E::field("num_elems"), pad: 0 }, Encoding::Ascii),
                E::field("num_elems").at_least(E::lit(1)),
            ),
            (_, Some(n)) => (n, E::lit(width_of(code))),
            (_, None) => return T::bytes(E::Remaining),
        };
        // Worked out once, at the front of the block, and read from there by
        // every record. Asked again where the second record starts, the same
        // expression would see a block one record shorter and answer with
        // fewer values each time.
        let per_record = E::Remaining.div(records.clone()).div(width).at_least(E::lit(0));
        let values = T::structure(
            "CdfValues",
            vec![("per_record", T::computed(per_record)), ("records", T::array(T::array(one, E::field("per_record")), records.clone()))],
        )
        .machinery(&["per_record"]);
        // Declared on the rows, which is every value in every row: a list of
        // lists lends its declaration to the numbers at the bottom of it.
        match time_of_type(code) {
            Some(time) => values.field_time("records", time),
            None => values,
        }
    })
}

/// How many bytes one value of this type takes. The companion of [`number`],
/// which says what those bytes mean; this is what divides a block of them up.
fn width_of(code: i128) -> i128 {
    match code {
        1 | 11 | 41 | 51 | 52 => 1,
        2 | 12 => 2,
        4 | 14 | 21 | 44 => 4,
        32 => 16,
        _ => 8,
    }
}

/// The descriptor record: which release of the library wrote the file, how its
/// numbers are encoded, and where the global descriptor is. Always the first
/// record, at offset eight.
fn cdr(s: Shape) -> T {
    T::structure(
        "CdfDescriptor",
        vec![
            ("gdr_offset", s.offset()),
            ("version", i32be()),
            ("release", i32be()),
            ("encoding", T::enumeration("CdfEncoding", i32be(), ENCODING)),
            (
                "flags",
                T::flags(
                    "CdfFlags",
                    i32be(),
                    &[(0, "row-major"), (1, "single file"), (2, "checksum"), (3, "MD5 checksum")],
                ),
            ),
            ("rfu_a", i32be()),
            ("rfu_b", i32be()),
            ("increment", i32be()),
            ("rfu_d", i32be()),
            ("rfu_e", i32be()),
            // 256 bytes of the notice every CDF carries, nul-padded.
            ("copyright", T::text(StrLen::Padded { size: E::Remaining, pad: 0 }, Encoding::Ascii)),
            ("gdr", s.at_record("gdr_offset")),
        ],
    )
}

/// The global descriptor: the head of each of the three chains, where the file
/// ends, and the shape every rVariable shares.
fn gdr(s: Shape) -> T {
    T::structure(
        "CdfGlobalDescriptor",
        vec![
            ("r_vdr_head", s.offset()),
            ("z_vdr_head", s.offset()),
            ("adr_head", s.offset()),
            // Where the records stop. A checksummed file has sixteen more
            // bytes after this, and nothing else should.
            ("eof", s.offset()),
            ("n_r_vars", i32be()),
            ("num_attr", i32be()),
            ("r_max_rec", i32be()),
            // The shape every rVariable shares. Called what a zVariable calls
            // its own, because a value block asks for it by name and has to
            // find whichever of the two its variable has: a zVariable's is
            // beside it, and an rVariable has none and finds this one.
            ("num_dims", i32be()),
            ("n_z_vars", i32be()),
            // The head of the free list: records the file has finished with
            // and would write over before it grew.
            ("uir_head", s.offset()),
            ("rfu_c", i32be()),
            // The day the leap second table the file was written against was
            // last changed, as YYYYMMDD. Reserved until 3.6, which is why a
            // file older than that writes -1 here.
            (if s.wide { "leap_second_last_updated" } else { "rfu_d" }, i32be()),
            ("rfu_e", i32be()),
            ("dim_sizes", T::array(i32be(), E::field("num_dims").at_least(E::lit(0)))),
            ("r_variables", s.chain_of("r_vdr_head", "vdr_next")),
            ("z_variables", s.chain_of("z_vdr_head", "vdr_next")),
            ("attributes", s.chain_of("adr_head", "adr_next")),
            ("unused", s.chain_of("uir_head", "uir_next")),
        ],
    )
}

/// One attribute: its name, who it belongs to, and the heads of its two lists
/// of entries. An attribute has an entry per rVariable and an entry per
/// zVariable, and a global attribute keeps its values in the first list.
fn adr(s: Shape) -> T {
    T::structure_named(
        "CdfAttribute",
        "name",
        "",
        vec![
            ("adr_next", s.offset()),
            ("agr_edr_head", s.offset()),
            ("scope", T::enumeration("CdfScope", i32be(), SCOPE)),
            ("num", i32be()),
            ("n_gr_entries", i32be()),
            ("max_gr_entry", i32be()),
            ("rfu_a", i32be()),
            ("az_edr_head", s.offset()),
            ("n_z_entries", i32be()),
            ("max_z_entry", i32be()),
            ("rfu_e", i32be()),
            ("name", s.name()),
            ("g_entries", s.chain_of("agr_edr_head", "aedr_next")),
            ("z_entries", s.chain_of("az_edr_head", "aedr_next")),
        ],
    )
}

/// One entry of an attribute: which variable it is set on, what type its value
/// is, and the value itself, read as that type.
///
/// An entry of a time type holds moments, which is what a time variable's
/// FILLVAL, VALIDMIN and VALIDMAX are, and is declared so: its data type is
/// the second field after the link to the next entry.
fn aedr(s: Shape) -> T {
    by_time_type(s.offset_bytes() + 4, |time| {
        let entry = aedr_layout(s);
        match time {
            Some(t) => entry.field_time("value", t),
            None => entry,
        }
    })
}

fn aedr_layout(s: Shape) -> T {
    T::structure(
        "CdfAttributeEntry",
        vec![
            ("aedr_next", s.offset()),
            ("attr_num", i32be()),
            ("data_type", data_type()),
            // Which variable this entry is about, by its number in the list.
            ("num", i32be()),
            ("num_elements", i32be()),
            // How many strings are packed into a character entry, which is a
            // question version 3 added: before it the slot was reserved.
            (if s.wide { "num_strings" } else { "rfu_a" }, i32be()),
            ("rfu_b", i32be()),
            ("rfu_c", i32be()),
            ("rfu_d", i32be()),
            ("rfu_e", i32be()),
            // The value, read by the type this record named and the byte order
            // the file's descriptor record did.
            ("value", by_encoding(entry_value)),
        ],
    )
}

/// One variable. `z` says which of the two kinds: a zVariable writes its own
/// shape, and an rVariable takes the shape the global descriptor declared and
/// writes only which of those dimensions it varies along.
///
/// A variable of a time type has a pad value that is a moment, or more often
/// the value that means none, and is declared so: its data type is the field
/// straight after the link to the next variable.
fn vdr(s: Shape, z: bool) -> T {
    by_time_type(s.offset_bytes(), |time| {
        let variable = vdr_layout(s, z);
        match time {
            Some(t) => variable.field_time("pad_value", t),
            None => variable,
        }
    })
}

fn vdr_layout(s: Shape, z: bool) -> T {
    let dims = || E::field("num_dims").at_least(E::lit(0));
    let mut fields = vec![
        ("vdr_next", s.offset()),
        ("data_type", data_type()),
        // The highest record number written, counting from zero, so -1 is a
        // variable with nothing in it yet.
        ("max_rec", i32be()),
        ("vxr_head", s.offset()),
        ("vxr_tail", s.offset()),
        (
            "flags",
            T::flags("CdfVariableFlags", i32be(), &[(0, "record variance"), (1, "pad value"), (2, "compressed")]),
        ),
        // How the records this variable has not been given are stored: not at
        // all, filled with the pad value, or filled with the previous record.
        ("s_records", T::enumeration("CdfSparseness", i32be(), &[(0, "none"), (1, "padded"), (2, "previous")])),
        ("rfu_b", i32be()),
        ("rfu_c", i32be()),
        ("rfu_f", i32be()),
    ];
    // A file older than version 2.5 leaves 128 bytes of nothing here, and
    // every field below it is 128 bytes further on. Nothing since does, so the
    // field is not there to be seen at all rather than there and empty.
    if !s.wide {
        fields.push(("wasted", s.wasted()));
    }
    fields.extend([
        ("num_elems", i32be()),
        ("num", i32be()),
        // Where the compression or sparseness parameters are, when the flags
        // say there are any.
        ("cpr_or_spr_offset", s.offset()),
        ("blocking_factor", i32be()),
        ("name", s.name()),
    ]);
    if z {
        fields.push(("num_dims", i32be()));
        fields.push(("dim_sizes", T::array(i32be(), dims())));
    }
    // Which of the dimensions the values actually vary along. A dimension that
    // does not vary is one value repeated, and is not stored.
    fields.push(("dim_varys", T::array(i32be(), dims())));
    fields.push(("pad_value", by_encoding(pad_value)));
    // The index of where the values are, which is a chain of its own: one
    // index record per few thousand records of the variable.
    fields.push(("values_index", s.chain_of("vxr_head", "vxr_next")));
    // Whichever of the two the flags said: how a compressed variable was
    // squeezed, or how a sparse one stores the records it was not given.
    fields.push(("parameters", s.at_record("cpr_or_spr_offset")));
    T::structure_named(if z { "CdfZVariable" } else { "CdfRVariable" }, "name", "", fields)
}

/// An index of the blocks of one variable's values: which records each block
/// covers, and where it is. A variable with more blocks than an index record
/// holds writes another and points at it from here, so these are a chain.
///
/// The three arrays are parallel and only the first `n_used_entries` of them
/// mean anything; what is written above that is room the writer left itself.
/// `blocks` is those three read as what they are, an entry each, with the
/// values at the far end of every one. It covers no bytes of its own.
fn vxr(s: Shape) -> T {
    let n = || E::field("n_entries").at_least(E::lit(0));
    T::structure(
        "CdfValueIndex",
        vec![
            ("vxr_next", s.offset()),
            ("n_entries", i32be()),
            ("n_used_entries", i32be()),
            ("first_record", T::array(i32be(), n())),
            ("last_record", T::array(i32be(), n())),
            ("block_offset", T::array(s.offset(), n())),
            ("blocks", T::array(vxr_entry(s), E::field("n_used_entries").at_least(E::lit(0)))),
        ],
    )
}

/// One entry of that index: the records it covers and the block that holds
/// them, read out of the three arrays above it.
///
/// The three numbers are worked out rather than read, because they are already
/// in the file: they are element `i` of three arrays, and writing them here is
/// what lets the block below say how many records it has to lay out. What the
/// entry points at is usually a block of values and may be another index, for
/// a variable with more blocks than one index record holds.
///
/// The `i` is [`Expr::Idx`](crate::template::Expr::Idx), which is this entry's
/// place in the nearest list around it, and the nearest list has to be the
/// `blocks` array. Nothing between this structure and that array may be a list
/// of its own, or every entry would read the first element of all three.
fn vxr_entry(s: Shape) -> T {
    T::structure(
        "CdfValueBlock",
        vec![
            ("from_record", T::computed(E::elem("first_record", E::Idx))),
            ("to_record", T::computed(E::elem("last_record", E::Idx))),
            ("at", T::computed(E::elem("block_offset", E::Idx))),
            ("values", s.at_record("at")),
        ],
    )
}

/// A block of values that was compressed: how long the stream is, and the
/// values inside it.
///
/// Which codec is in the variable's own compression parameters and not here,
/// so what is asked is the stream: a gzip member starts with two bytes that
/// say so, and the other three codings CDF has start with nothing in
/// particular. A block packed one of those other ways keeps its bytes.
fn cvvr(s: Shape) -> T {
    T::structure(
        "CdfCompressedValues",
        vec![
            ("rfu_a", i32be()),
            ("c_size", s.size()),
            (
                "data",
                T::switch(
                    E::peek(16, Big),
                    vec![(GZIP_MAGIC, T::decoded(E::field("c_size"), Codec::Gzip, by_encoding(value_records)))],
                    T::bytes(E::field("c_size").at_least(E::lit(0))),
                ),
            ),
        ],
    )
}

/// How a sparse variable stores the records nobody gave it, and with what
/// settings. The companion of the compression parameters, in the same slot of
/// the variable descriptor.
fn spr() -> T {
    T::structure(
        "CdfSparsenessParameters",
        vec![
            ("s_arrays_type", i32be()),
            ("rfu_a", i32be()),
            ("p_count", i32be()),
            ("s_arrays_parms", T::array(i32be(), E::field("p_count").at_least(E::lit(0)))),
        ],
    )
}

/// The whole of an uncompressed file, squeezed into one record, and the file
/// read out of it.
///
/// The parameters are read before the stream because they are what says how to
/// open it: they are a record of their own somewhere else in the file, and a
/// field that points at one takes no room, so reading it first costs nothing
/// and puts the codec in hand while the cursor is still here.
fn ccr(s: Shape) -> T {
    let inside = |codec| T::decoded(E::Remaining, codec, inside_a_compressed_file(s));
    T::structure(
        "CdfCompressed",
        vec![
            ("cpr_offset", s.offset()),
            // How large it was before, which is what a reader allocates.
            ("u_size", s.size()),
            ("rfu_a", i32be()),
            ("parameters", T::at_origin(from_file_start("cpr_offset"), T::Named(s.record.into()))),
            (
                "data",
                T::switch(
                    E::within(&["parameters", "body", "c_type"]),
                    vec![
                        (1, inside(Codec::CdfRle)),
                        (2, inside(Codec::CdfHuffman)),
                        (3, inside(Codec::CdfAhuff)),
                        (5, inside(Codec::Gzip)),
                    ],
                    T::bytes(E::Remaining),
                ),
            ),
        ],
    )
}

/// The file a compressed one holds, read over the bytes that come out of the
/// stream.
///
/// What is squeezed is everything after the eight-byte signature, so the first
/// byte out of the stream is the file's ninth and every offset written inside
/// it is eight larger than where it lands here. `bytes_before` is that eight,
/// and every offset in this template is taken back by it; at the front of a
/// file read where it lies the same field is nought and nothing moves.
///
/// The marker round it is what the offsets count from. `Anchor::File` would
/// mean the file these bytes came out of, which is a different set of bytes
/// with a different length, and the walk would end wherever the packing
/// happened to leave off.
fn inside_a_compressed_file(s: Shape) -> T {
    T::origin(
        T::structure(
            "CdfInsideCompressed",
            vec![("bytes_before", T::computed(E::lit(8))), ("first_record", T::Named(s.record.into()))],
        )
        .machinery(&["bytes_before"]),
    )
}

/// How something was compressed, and with what settings: the gzip level, or
/// nothing at all for the other three.
fn cpr() -> T {
    T::structure(
        "CdfCompressionParameters",
        vec![
            ("c_type", T::enumeration("CdfCompression", i32be(), COMPRESSION)),
            ("rfu_a", i32be()),
            ("p_count", i32be()),
            ("c_parms", T::array(i32be(), E::field("p_count").at_least(E::lit(0)))),
        ],
    )
}

/// A record the file has finished with, and the two it sits between in the
/// free list. Not walked: what it holds is whatever was written there before.
fn uir(s: Shape) -> T {
    T::structure("CdfUnused", vec![("uir_next", s.offset()), ("uir_prev", s.offset()), ("free", T::bytes(E::Remaining))])
}

/// Any record: how long it is, what it is, and that many bytes read as the
/// layout its type names.
///
/// Sizing the body from the record's own size is what makes the trailing
/// fields safe. An attribute entry's value, a variable's pad value and the
/// notice in the descriptor record all run to the end of their record and
/// nothing else says how long they are.
fn record(s: Shape) -> T {
    let body = T::switch(
        E::field("type"),
        vec![
            (-1, uir(s)),
            (1, cdr(s)),
            (2, gdr(s)),
            (3, vdr(s, false)),
            (4, adr(s)),
            (5, aedr(s)),
            (6, vxr(s)),
            (7, by_encoding(value_records)),
            (8, vdr(s, true)),
            (9, aedr(s)),
            (10, ccr(s)),
            (11, cpr()),
            (12, spr()),
            (13, cvvr(s)),
        ],
        // A record type no version of CDF has: sized, named, and not opened.
        T::bytes(E::Remaining),
    );
    T::structure_named(
        s.record,
        "type",
        "body",
        vec![
            ("size", s.size()),
            ("type", T::enumeration("CdfRecordType", i32be(), RECORD_TYPE)),
            ("body", T::sized(E::field("size").sub(E::lit(s.header())).at_least(E::lit(0)), body)),
        ],
    )
}

pub fn cdf() -> Template {
    let root = T::structure(
        "Cdf",
        vec![
            // Nought, because these bytes are the file. See `from_file_start`,
            // and `inside_a_compressed_file` for where it is not.
            ("bytes_before", T::computed(E::lit(0))),
            (
                "magic",
                T::enumeration_hex(
                    "CdfMagic",
                    T::u32(Big),
                    // Version 2.6 gave the format a signature of its own.
                    // Before that a file opened with the word that means "not
                    // compressed", twice over, and the only thing saying what
                    // it was, was the descriptor record behind it.
                    &[(0xCDF3_0001, "CDF 3"), (0xCDF2_6002, "CDF 2.6"), (0x0000_FFFF, "CDF 2.x")],
                ),
            ),
            (
                "compression",
                T::enumeration_hex(
                    "CdfCompressed",
                    T::u32(Big),
                    &[(0x0000_FFFF, "uncompressed"), (0xCCCC_0001, "compressed")],
                ),
            ),
            // The first record, which is the descriptor record of an
            // uncompressed file and the compressed record of a squeezed one.
            // Everything else in the file is reached from it.
            (
                "first_record",
                T::switch(
                    E::field("magic"),
                    vec![
                        (0x0000_FFFF, T::Named(V2.record.into())),
                        (0xCDF2_6002u32 as i128, T::Named(V2.record.into())),
                    ],
                    T::Named(V3.record.into()),
                ),
            ),
        ],
    )
    .machinery(&["bytes_before"]);
    Template::new("cdf", root).with_type(V3.record, record(V3)).with_type(V2.record, record(V2))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, eval::{Evaluator, Moment, Value}, source::MemSource};

    fn be32(v: i32) -> Vec<u8> {
        v.to_be_bytes().to_vec()
    }
    fn be64(v: i64) -> Vec<u8> {
        v.to_be_bytes().to_vec()
    }

    /// A record: its size, its type, and its body.
    fn rec(kind: i32, body: Vec<u8>) -> Vec<u8> {
        let mut v = ((body.len() + 12) as u64).to_be_bytes().to_vec();
        v.extend(be32(kind));
        v.extend(body);
        v
    }

    /// A name in the 256 bytes one gets.
    fn nm(s: &str) -> Vec<u8> {
        let mut v = s.as_bytes().to_vec();
        v.resize(256, 0);
        v
    }

    /// A small version 3 file: a descriptor record, a global descriptor with
    /// one dimension declared for the rVariables, one rVariable, one
    /// zVariable, and one global attribute with one entry.
    ///
    /// Written twice over, once to measure the records and once with the
    /// offsets that measuring settled, which is what a writer does.
    fn file() -> Vec<u8> {
        let cdr_at = 8i64;
        let build = |o: [i64; 5]| {
            let [gdr_at, rvdr_at, zvdr_at, adr_at, aedr_at] = o;
            let mut cdr = be64(gdr_at);
            cdr.extend(be32(3)); // version
            cdr.extend(be32(8)); // release
            cdr.extend(be32(1)); // network encoding
            cdr.extend(be32(3)); // row-major, single file
            cdr.extend(be32(0));
            cdr.extend(be32(0));
            cdr.extend(be32(0)); // increment
            cdr.extend(be32(-1));
            cdr.extend(be32(-1));
            let mut notice = b"Common Data Format (CDF)".to_vec();
            notice.resize(256, 0);
            cdr.extend(notice);

            let mut gdr = be64(rvdr_at);
            gdr.extend(be64(zvdr_at));
            gdr.extend(be64(adr_at));
            gdr.extend(be64(0)); // eof, which nothing here reads back
            gdr.extend(be32(1)); // one rVariable
            gdr.extend(be32(1)); // one attribute
            gdr.extend(be32(0)); // rMaxRec
            gdr.extend(be32(2)); // two rVariable dimensions
            gdr.extend(be32(1)); // one zVariable
            gdr.extend(be64(0)); // no free list
            gdr.extend(be32(0));
            gdr.extend(be32(-1));
            gdr.extend(be32(-1));
            gdr.extend(be32(4)); // rDimSizes
            gdr.extend(be32(5));

            let vdr = |z: bool, name: &str, dims: &[i32]| {
                let mut v = be64(0); // no next of this kind
                v.extend(be32(45)); // double
                v.extend(be32(-1)); // nothing written yet
                v.extend(be64(0)); // no index
                v.extend(be64(0));
                v.extend(be32(1)); // record variance, no pad value
                v.extend(be32(0));
                v.extend(be32(0));
                v.extend(be32(-1));
                v.extend(be32(-1));
                v.extend(be32(1)); // one element
                v.extend(be32(0)); // number zero
                v.extend(be64(0));
                v.extend(be32(0));
                v.extend(nm(name));
                if z {
                    v.extend(be32(dims.len() as i32));
                    for d in dims {
                        v.extend(be32(*d));
                    }
                }
                for _ in dims {
                    v.extend(be32(1)); // varies
                }
                v
            };

            let mut adr = be64(0); // one attribute only
            adr.extend(be64(aedr_at));
            adr.extend(be32(1)); // global
            adr.extend(be32(0));
            adr.extend(be32(1)); // one entry
            adr.extend(be32(0));
            adr.extend(be32(0));
            adr.extend(be64(0)); // no zEntries
            adr.extend(be32(0));
            adr.extend(be32(0));
            adr.extend(be32(-1));
            adr.extend(nm("TITLE"));

            let mut aedr = be64(0);
            aedr.extend(be32(0)); // attribute zero
            aedr.extend(be32(51)); // char
            aedr.extend(be32(0));
            aedr.extend(be32(5)); // five of them
            aedr.extend(be32(1));
            for _ in 0..4 {
                aedr.extend(be32(0));
            }
            aedr.extend_from_slice(b"depth");

            let mut b = MAGIC.to_vec();
            b.extend_from_slice(&[0x00, 0x00, 0xFF, 0xFF]);
            b.extend(rec(1, cdr));
            b.extend(rec(2, gdr));
            b.extend(rec(3, vdr(false, "r_field", &[1, 1])));
            b.extend(rec(8, vdr(true, "sea_temp", &[3])));
            b.extend(rec(4, adr));
            b.extend(rec(5, aedr));
            b
        };
        // One pass to measure, one to write. Every record but the first is
        // reached by an offset, so the offsets have to be known first.
        let zeros = build([0; 5]);
        let sizes: Vec<i64> = {
            let mut at = cdr_at;
            let mut out = Vec::new();
            for _ in 0..6 {
                out.push(at);
                let size = i64::from_be_bytes(zeros[at as usize..at as usize + 8].try_into().unwrap());
                at += size;
            }
            out
        };
        build([sizes[1], sizes[2], sizes[3], sizes[4], sizes[5]])
    }

    #[test]
    fn the_first_record_says_where_the_global_descriptor_is() {
        let d = Document::new(MemSource(file()));
        let mut e = Evaluator::new(cdf());
        assert_eq!(
            e.node(&d, &[1]).unwrap().value,
            Value::Enum { raw: 0xCDF3_0001, name: Some("CDF 3".into()), hex: true }
        );
        assert_eq!(
            e.node(&d, &[2]).unwrap().value,
            Value::Enum { raw: 0x0000_FFFF, name: Some("uncompressed".into()), hex: true }
        );
        let cdr = e.node(&d, &[3, 2]).unwrap();
        assert_eq!(cdr.type_name, "CdfDescriptor");
        assert_eq!(e.node(&d, &[3, 2, 1]).unwrap().value, Value::Int(3));
        assert_eq!(e.node(&d, &[3, 2, 10]).unwrap().value, Value::Str("Common Data Format (CDF)".into()));
        // The global descriptor, reached by the offset rather than by lying
        // next to it.
        let gdr = e.node(&d, &[3, 2, 11, 0, 2]).unwrap();
        assert_eq!(gdr.type_name, "CdfGlobalDescriptor");
    }

    #[test]
    fn a_records_body_is_as_long_as_the_record_says() {
        let d = Document::new(MemSource(file()));
        let mut e = Evaluator::new(cdf());
        let size = e.node(&d, &[3, 0]).unwrap().value.as_int().unwrap();
        assert_eq!(e.node(&d, &[3, 2]).unwrap().size_bits as i128, (size - 12) * 8);
        // The notice runs to the end of the record and nothing says how long
        // it is but the record's own size.
        assert_eq!(e.node(&d, &[3, 2, 10]).unwrap().size_bits, 256 * 8);
    }

    #[test]
    fn a_z_variable_carries_its_own_shape() {
        let d = Document::new(MemSource(file()));
        let mut e = Evaluator::new(cdf());
        let z = e.node(&d, &[3, 2, 11, 0, 2, 15, 0, 2]).unwrap();
        assert_eq!(z.type_name, "CdfZVariable");
        assert_eq!(z.name, "body sea_temp");
        assert_eq!(e.node(&d, &[3, 2, 11, 0, 2, 15, 0, 2, 1]).unwrap().value.as_int(), Some(45));
        assert_eq!(e.node(&d, &[3, 2, 11, 0, 2, 15, 0, 2, 15]).unwrap().value, Value::Int(1));
        assert_eq!(e.node(&d, &[3, 2, 11, 0, 2, 15, 0, 2, 16, 0]).unwrap().value, Value::Int(3));
    }

    #[test]
    fn an_r_variable_takes_the_shape_the_global_descriptor_declared() {
        // Two dimensions, declared once in the global descriptor, so the
        // rVariable writes two variances and no sizes of its own.
        let d = Document::new(MemSource(file()));
        let mut e = Evaluator::new(cdf());
        let r = e.node(&d, &[3, 2, 11, 0, 2, 14, 0, 2]).unwrap();
        assert_eq!(r.type_name, "CdfRVariable");
        assert_eq!(r.name, "body r_field");
        assert_eq!(e.node(&d, &[3, 2, 11, 0, 2, 14, 0, 2, 15]).unwrap().child_count, 2);
    }

    #[test]
    fn an_attribute_heads_a_chain_of_entries() {
        let d = Document::new(MemSource(file()));
        let mut e = Evaluator::new(cdf());
        let adr = e.node(&d, &[3, 2, 11, 0, 2, 16, 0, 2]).unwrap();
        assert_eq!(adr.name, "body TITLE");
        assert_eq!(e.node(&d, &[3, 2, 11, 0, 2, 16, 0, 2, 2]).unwrap().value.as_int(), Some(1));
        let entry = e.node(&d, &[3, 2, 11, 0, 2, 16, 0, 2, 12, 0, 2]).unwrap();
        assert_eq!(entry.type_name, "CdfAttributeEntry");
        // Five characters of value, which stay bytes: how to read them is the
        // file's encoding to say, not this record's.
        assert_eq!(e.node(&d, &[3, 2, 11, 0, 2, 16, 0, 2, 12, 0, 2, 10]).unwrap().size_bits, 5 * 8);
    }

    #[test]
    fn every_chain_is_a_flat_list_however_long_it_is() {
        let d = Document::new(MemSource(file()));
        let mut e = Evaluator::new(cdf());
        let gdr = [3, 2, 11, 0, 2];
        let at = |e: &mut Evaluator, i: usize| {
            let mut p = gdr.to_vec();
            p.push(i);
            e.node(&d, &p).unwrap()
        };
        // Each list is one row with its elements under it, rather than a
        // record that holds the next record that holds the next record.
        assert_eq!(at(&mut e, 14).child_count, 1); // rVariables
        assert_eq!(at(&mut e, 15).child_count, 1); // zVariables
        let attrs = at(&mut e, 16);
        assert_eq!(attrs.child_count, 1);
        assert_eq!(attrs.type_name, "chain \u{2192} CdfRecord");
        // The list covers no bytes where it stands; the records it found do.
        assert_eq!(attrs.size_bits, 0);
        // A chain whose head is zero is a list of nothing, not a broken file.
        assert_eq!(at(&mut e, 17).child_count, 0); // the free list
        // The attribute's own entries are a list too.
        assert_eq!(e.node(&d, &[3, 2, 11, 0, 2, 16, 0, 2, 12]).unwrap().child_count, 1);
    }

    /// CDF's run-length coding: a zero byte escapes one less than the number
    /// of zeroes to write, and every other byte is itself.
    fn rle(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut at = 0;
        while at < data.len() {
            if data[at] != 0 {
                out.push(data[at]);
                at += 1;
                continue;
            }
            let run = data[at..].iter().take_while(|b| **b == 0).count().min(256);
            out.push(0);
            out.push((run - 1) as u8);
            at += run;
        }
        out
    }

    /// A compressed file: the whole of [`file`] after its signature, squeezed,
    /// and the parameters that say how.
    ///
    /// The point of the test is that the walk inside lands in the right place.
    /// Every offset in there counts from the front of the file it was, and what
    /// comes out of the stream is that file from its ninth byte on, so a reader
    /// that does not take the eight back reads every record eight bytes late.
    fn compressed() -> Vec<u8> {
        let plain = file();
        let mut ccr = be64(0); // the parameters, filled in below
        ccr.extend(be64((plain.len() - 8) as i64));
        ccr.extend(be32(0));
        ccr.extend(rle(&plain[8..]));
        let cpr_at = (8 + 12 + ccr.len()) as i64;
        ccr[0..8].copy_from_slice(&cpr_at.to_be_bytes());
        let mut cpr = be32(1); // run-length
        cpr.extend(be32(0));
        cpr.extend(be32(1));
        cpr.extend(be32(0)); // its one parameter, which means nothing
        let mut b = MAGIC.to_vec();
        b.extend_from_slice(&[0xCC, 0xCC, 0x00, 0x01]);
        b.extend(rec(10, ccr));
        b.extend(rec(11, cpr));
        b
    }

    #[test]
    fn a_compressed_file_holds_one_record_and_the_settings_that_made_it() {
        let d = Document::new(MemSource(compressed()));
        let mut e = Evaluator::new(cdf());
        assert_eq!(
            e.node(&d, &[2]).unwrap().value,
            Value::Enum { raw: 0xCCCC_0001, name: Some("compressed".into()), hex: true }
        );
        let body = e.node(&d, &[3, 2]).unwrap();
        assert_eq!(body.type_name, "CdfCompressed");
        assert_eq!(e.node(&d, &[3, 2, 1]).unwrap().value, Value::UInt(file().len() as u128 - 8));
        let parms = e.node(&d, &[3, 2, 3, 0, 2]).unwrap();
        assert_eq!(parms.type_name, "CdfCompressionParameters");
        assert_eq!(
            e.node(&d, &[3, 2, 3, 0, 2, 0]).unwrap().value,
            Value::Enum { raw: 1, name: Some("run-length".into()), hex: false }
        );
    }

    /// The file inside the compressed one, read as a file: the same records in
    /// the same order, found by the same offsets taken back by the eight bytes
    /// of signature that are not in the stream.
    #[test]
    fn a_compressed_file_reads_as_the_file_it_holds() {
        let d = Document::new(MemSource(compressed()));
        let mut e = Evaluator::new(cdf());
        // What comes out is longer than the file it came out of, which is the
        // whole point of squeezing it and the reason the walk inside cannot be
        // bounded by the length of the file.
        let inside = e.node(&d, &[3, 2, 4, 0]).unwrap();
        assert_eq!(inside.type_name, "CdfInsideCompressed");
        // Every offset in this template is taken back by this field, found by
        // its name from wherever the offset is. A space whose root does not
        // declare it would find the file's own nought instead and read every
        // record eight bytes late, so the name is checked and not only the
        // number.
        let base = e.node(&d, &[3, 2, 4, 0, 0]).unwrap();
        assert_eq!(base.name, "bytes_before");
        assert_eq!(base.value, Value::Int(8));
        // The descriptor record is the first byte of the stream, and the global
        // descriptor is at the offset it names less those eight.
        let cdr = e.node(&d, &[3, 2, 4, 0, 1, 2]).unwrap();
        assert_eq!(cdr.type_name, "CdfDescriptor");
        assert_eq!(e.node(&d, &[3, 2, 4, 0, 1, 2, 11, 0, 2]).unwrap().type_name, "CdfGlobalDescriptor");
        let gdr = e.node(&d, &[3, 2, 4, 0, 1, 2, 11, 0]).unwrap();
        assert_eq!(gdr.offset_bits, (e.node(&d, &[3, 2, 4, 0, 1, 2, 0]).unwrap().value.as_int().unwrap() - 8) as u64 * 8);
        // And the chains below it, which are the reason for all of it: the
        // zVariable is the same one, with the same name.
        let z = e.node(&d, &[3, 2, 4, 0, 1, 2, 11, 0, 2, 15, 0, 2]).unwrap();
        assert_eq!(z.name, "body sea_temp");
        let a = e.node(&d, &[3, 2, 4, 0, 1, 2, 11, 0, 2, 16, 0, 2]).unwrap();
        assert_eq!(a.name, "body TITLE");
        assert_eq!(e.node(&d, &[3, 2, 4, 0, 1, 2, 11, 0, 2, 16, 0, 2, 12, 0, 2, 10]).unwrap().value, Value::Str("depth".into()));
    }

    /// A file with one zVariable and one block of its values: the smallest
    /// thing that exercises the walk from a descriptor to numbers.
    ///
    /// `encoding` is the machine the numbers are in, `data_type` what they
    /// are, `dim` how many of them are in one record, and `block` the bytes of
    /// the block, already in that byte order. With `compressed` the block is a
    /// compressed one, and `block` is the stream rather than the values.
    fn with_values(encoding: i32, data_type: i32, dim: i32, records: i32, block: &[u8], compressed: bool) -> Vec<u8> {
        // Every record here is a fixed size given the arguments, so the
        // offsets are worked out rather than measured.
        let (gdr_at, vdr_at) = (8 + 312, 8 + 312 + 84);
        // A descriptor with one dimension in it, and an index with one entry.
        let vxr_at = vdr_at + 352;
        let block_at = vxr_at + 44;

        let mut cdr = be64(gdr_at);
        cdr.extend(be32(3)); // version
        cdr.extend(be32(8)); // release
        cdr.extend(be32(encoding));
        cdr.extend(be32(3)); // row-major, single file
        for _ in 0..3 {
            cdr.extend(be32(0));
        }
        cdr.extend(be32(-1));
        cdr.extend(be32(-1));
        cdr.resize(cdr.len() + 256, 0);

        let mut gdr = be64(0); // no rVariables
        gdr.extend(be64(vdr_at));
        gdr.extend(be64(0)); // no attributes
        gdr.extend(be64(0)); // eof, which nothing here reads back
        for v in [0, 0, -1, 0, 1] {
            gdr.extend(be32(v)); // no rVariables, no attributes, no rDims, one zVariable
        }
        gdr.extend(be64(0)); // no free list
        for v in [0, -1, -1] {
            gdr.extend(be32(v));
        }

        let mut vdr = be64(0); // the only variable of its kind
        vdr.extend(be32(data_type));
        vdr.extend(be32(records - 1)); // the highest record number written
        vdr.extend(be64(vxr_at));
        vdr.extend(be64(vxr_at));
        vdr.extend(be32(if compressed { 5 } else { 1 })); // record variance, and compressed
        for v in [0, 0, -1, -1] {
            vdr.extend(be32(v));
        }
        vdr.extend(be32(1)); // one element per value
        vdr.extend(be32(0)); // variable number zero
        vdr.extend(be64(-1)); // no parameters
        vdr.extend(be32(0));
        vdr.extend(nm("measured"));
        vdr.extend(be32(1)); // one dimension
        vdr.extend(be32(dim));
        vdr.extend(be32(1)); // which varies

        let mut vxr = be64(0); // no second index record
        vxr.extend(be32(1)); // one entry, and it is used
        vxr.extend(be32(1));
        vxr.extend(be32(0)); // covering records nought to the last
        vxr.extend(be32(records - 1));
        vxr.extend(be64(block_at));

        let mut b = MAGIC.to_vec();
        b.extend_from_slice(&[0x00, 0x00, 0xFF, 0xFF]);
        b.extend(rec(1, cdr));
        b.extend(rec(2, gdr));
        b.extend(rec(8, vdr));
        b.extend(rec(6, vxr));
        match compressed {
            false => b.extend(rec(7, block.to_vec())),
            true => {
                let mut cvvr = be32(0);
                cvvr.extend(be64(block.len() as i64));
                cvvr.extend_from_slice(block);
                b.extend(rec(13, cvvr));
            }
        }
        assert_eq!(b.len() as i64, block_at + 12 + block.len() as i64 + if compressed { 12 } else { 0 });
        b
    }

    /// Where the records of the one variable in [`with_values`] are.
    const VALUES: &[usize] = &[3, 2, 11, 0, 2, 15, 0, 2, 19, 0, 2, 6, 0, 3, 0, 2, 1];

    /// The same three numbers written by a big-endian machine and by a little-
    /// endian one, which is what the encoding field is for. Both come out as
    /// the numbers somebody measured; read the one way round they would be
    /// four orders of magnitude apart.
    #[test]
    fn the_encoding_says_which_way_round_the_values_are() {
        let numbers: [f32; 3] = [1.5, -2.25, 3.75];
        let mut network = Vec::new();
        let mut ibm_pc = Vec::new();
        for v in numbers {
            network.extend(v.to_be_bytes());
            ibm_pc.extend(v.to_le_bytes());
        }
        // 1 is network order and 6 is an IBM PC, which is the pair nearly every
        // CDF in the world is one of.
        for (encoding, block) in [(1, network), (6, ibm_pc)] {
            let d = Document::new(MemSource(with_values(encoding, 21, 3, 1, &block, false)));
            let mut e = Evaluator::new(cdf());
            let values = e.node(&d, VALUES).unwrap();
            assert_eq!(values.child_count, 1, "one record, encoding {encoding}");
            for (i, want) in numbers.iter().enumerate() {
                let mut p = VALUES.to_vec();
                p.extend([0, i]);
                assert_eq!(e.node(&d, &p).unwrap().value, Value::Float(*want as f64), "encoding {encoding}");
            }
        }
    }

    /// A block that was squeezed. CDF hands the whole block to gzip, so what is
    /// in the record is a member with its own header and its own check, and
    /// what the values are read over is what comes out of it.
    #[test]
    fn a_compressed_block_of_values_is_unpacked() {
        let numbers: [i32; 4] = [7, -1, 1000, 0];
        let mut plain = Vec::new();
        for v in numbers {
            plain.extend(v.to_be_bytes());
        }
        let d = Document::new(MemSource(with_values(1, 4, 2, 2, &gzip_member(&plain), true)));
        let mut e = Evaluator::new(cdf());
        // The record's body is the stream and what it says about itself; the
        // values are what came out of it.
        let mut values_at = VALUES[..VALUES.len() - 1].to_vec();
        values_at.extend([2, 0, 1]);
        let values = e.node(&d, &values_at).unwrap();
        // Two records of two numbers each, which is what the index entry and
        // the variable's one dimension say between them.
        assert_eq!(values.child_count, 2);
        for (i, want) in numbers.iter().enumerate() {
            let mut p = values_at.clone();
            p.extend([i / 2, i % 2]);
            assert_eq!(e.node(&d, &p).unwrap().value, Value::Int(*want as i128));
        }
    }

    fn path(at: &[usize], more: &[usize]) -> Vec<usize> {
        let mut p = at.to_vec();
        p.extend(more);
        p
    }

    /// A CDF_EPOCH variable's values are moments, and its fill value is no
    /// time. The number is `cacsst2.cdf`'s, 1982-01-01 by `cdflib`.
    #[test]
    fn an_epoch_variables_values_are_moments() {
        let mut block = Vec::new();
        for ms in [62545910400000.0f64, -1.0e31] {
            block.extend(ms.to_be_bytes());
        }
        let d = Document::new(MemSource(with_values(1, 31, 1, 2, &block, false)));
        let mut e = Evaluator::new(cdf());
        let first = e.time_of(&d, &path(VALUES, &[0, 0])).unwrap().expect("an epoch is a time");
        assert_eq!(first.moment, Moment::At { unix_seconds: 378_691_200, nanos: 0 });
        assert_eq!(e.time_of(&d, &path(VALUES, &[1, 0])).unwrap().unwrap().moment, Moment::Unset);

        // The same eight bytes in a double variable are a double.
        let d = Document::new(MemSource(with_values(1, 45, 1, 2, &block, false)));
        let mut e = Evaluator::new(cdf());
        assert!(e.time_of(&d, &path(VALUES, &[0, 0])).unwrap().is_none());
    }

    /// A CDF_TIME_TT2000 variable's values go through the table of leap
    /// seconds. The first is half a second into the one at the end of 2016,
    /// by `cdflib`'s `compute_tt2000`; the second is the pad value.
    #[test]
    fn a_tt2000_variables_values_count_leap_seconds() {
        let mut block = Vec::new();
        for n in [536_500_868_684_000_000i64, i64::MIN + 1] {
            block.extend(n.to_be_bytes());
        }
        let d = Document::new(MemSource(with_values(1, 33, 1, 2, &block, false)));
        let mut e = Evaluator::new(cdf());
        let leap = e.time_of(&d, &path(VALUES, &[0, 0])).unwrap().expect("a TT2000 is a time");
        assert_eq!(leap.moment, Moment::LeapSecond { unix_seconds: 1_483_228_799, nanos: 500_000_000 });
        assert_eq!(e.time_of(&d, &path(VALUES, &[1, 0])).unwrap().unwrap().moment, Moment::Unset);
    }

    /// A CDF_EPOCH16's seconds are the moment and its picoseconds are not.
    #[test]
    fn an_epoch16s_seconds_are_the_moment() {
        let mut block = 62545910400.0f64.to_le_bytes().to_vec();
        block.extend(123456789012.0f64.to_le_bytes());
        let d = Document::new(MemSource(with_values(6, 32, 1, 1, &block, false)));
        let mut e = Evaluator::new(cdf());
        let seconds = e.time_of(&d, &path(VALUES, &[0, 0, 0])).unwrap().expect("the seconds are a time");
        assert_eq!(seconds.moment, Moment::At { unix_seconds: 378_691_200, nanos: 0 });
        assert!(e.time_of(&d, &path(VALUES, &[0, 0, 1])).unwrap().is_none(), "the picoseconds are a count");
    }

    /// A gzip member holding `data`, written as one stored deflate block, which
    /// is the shape that can be built by hand.
    fn gzip_member(data: &[u8]) -> Vec<u8> {
        let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff];
        out.push(1); // the last block, and stored
        out.extend((data.len() as u16).to_le_bytes());
        out.extend((!(data.len() as u16)).to_le_bytes());
        out.extend_from_slice(data);
        out.extend(crate::checksum::crc32(data).to_le_bytes());
        out.extend((data.len() as u32).to_le_bytes());
        out
    }

    #[test]
    fn a_version_two_file_reads_its_header_and_its_32_bit_offsets() {
        // The GDR sits straight after the CDR: eight bytes of signature, then
        // the CDR's own eight-byte header and its body.
        let gdr_at = 8 + 8 + 4 + 9 * 4 + 5;
        let mut cdr = (gdr_at as u32).to_be_bytes().to_vec();
        cdr.extend(be32(2));
        cdr.extend(be32(7));
        cdr.extend(be32(1));
        cdr.extend(be32(1));
        for _ in 0..5 {
            cdr.extend(be32(0));
        }
        cdr.extend_from_slice(b"NSSDC");
        let mut b = MAGIC_V2.to_vec();
        let mut r = ((cdr.len() + 8) as u32).to_be_bytes().to_vec();
        r.extend(be32(1));
        r.extend(cdr);
        b.extend(r);
        // A global descriptor this does not open, at the offset the CDR gave.
        b.extend(16u32.to_be_bytes());
        b.extend(be32(2));
        b.extend(be64(0));
        let d = Document::new(MemSource(b));
        let mut e = Evaluator::new(cdf());
        assert_eq!(
            e.node(&d, &[1]).unwrap().value,
            Value::Enum { raw: 0x0000_FFFF, name: Some("CDF 2.x".into()), hex: true }
        );
        // The same layout as a version 3 file, at half the width: the record
        // header is eight bytes rather than twelve and every offset is four.
        let cdr = e.node(&d, &[3, 2]).unwrap();
        assert_eq!(cdr.type_name, "CdfDescriptor");
        assert_eq!(e.node(&d, &[3, 2, 0]).unwrap().value, Value::Int(gdr_at as i128));
        assert_eq!(e.node(&d, &[3, 2, 0]).unwrap().size_bits, 32);
        assert_eq!(e.node(&d, &[3, 2, 1]).unwrap().value, Value::Int(2));
        assert_eq!(e.node(&d, &[3, 2, 10]).unwrap().value, Value::Str("NSSDC".into()));
        // The record it points at, read as far as its size and its type.
        assert_eq!(e.node(&d, &[3, 2, 11, 0, 0]).unwrap().value, Value::UInt(16));
    }
}
