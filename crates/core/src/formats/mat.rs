//! MAT: what MATLAB's `save` writes, in the three shapes it has had.
//!
//! Level 5, which every `.mat` from MATLAB 5 onwards is, opens with 128 bytes
//! of header: 116 of text saying which MATLAB wrote it and when, an offset to
//! the subsystem data, a version, and two letters that say which way round the
//! numbers are. `IM` is a little-endian writer and `MI` a big-endian one, and
//! they are the same two letters read either way, which is the only reason
//! they can be read before the answer is known.
//!
//! After the header is a stream of data elements, each an 8-byte tag and the
//! bytes it counts, padded out to a multiple of eight. A tag is a type and a
//! byte count, except when the count is small enough to fit beside the type:
//! then the count moves into the top half of the first word and four bytes of
//! data follow the tag instead of eight. Which form a tag is in is the top
//! half of its first word being nonzero, and that is the first thing read
//! here.
//!
//! Two element types are not data:
//!
//! - `miCOMPRESSED` holds a zlib stream, and what inflates out of it is
//!   another element. That is where a `.mat` written by MATLAB 7 puts every
//!   variable, so the tree under one of these is the file's actual contents.
//!   These are the one element that is not padded: the next tag begins the
//!   byte after the stream ends.
//! - `miMATRIX` is an array, and holds elements of its own: the flags that
//!   say what class it is, its dimensions, its name, and then its values.
//!
//! An array's values are typed by the element that carries them rather than
//! by the array's class. MATLAB writes a double array as bytes when every
//! value fits in one, so a `mxDOUBLE` whose numbers are 1 to 9 is a run of
//! `miUINT8` and reads as one here. The class says what MATLAB hands back;
//! the element type says what is on disk, and this reads the disk.
//!
//! Level 4, which is what `save -v4` still writes and what a `.mat` from
//! before MATLAB 5 is, has no header and no magic number. A file is a run of
//! matrices, each opening with five 32-bit integers: a packed description, the
//! rows, the columns, whether there is an imaginary part, and the length of
//! the name. The description is read as the decimal digits it is: thousands
//! for the machine that wrote it, tens for how wide a number is, units for
//! whether the matrix is text. Nothing marks the front of one of these, so
//! what recognises it is those five integers agreeing with each other and with
//! the length of the file.
//!
//! Level 7.3 is an HDF5 file with the same 128-byte header in front of it,
//! sitting in the 512-byte user block that HDF5 allows. The header is read
//! here and says so; the HDF5 inside it is not, because every address in an
//! HDF5 superblock counts from the base address the superblock names, and the
//! HDF5 template reads addresses from the front of the file. Reading one would
//! mean teaching that template a base to count from.
//!
//! What else is not read here:
//!
//! - The subsystem data the header points at, which is where MATLAB keeps
//!   objects of a class: a `mxOPAQUE` element names a class and an offset into
//!   it, and the offset lands in a region this reads as bytes.
//! - A structure's fields are the elements they are, in order, but they are
//!   not labelled with the names beside them. The names are in the file and
//!   read as text; joining each to its field would mean an array's rows taking
//!   a name from a run of fixed-width text in a sibling element.
//! - A sparse array's row indices and column starts read as the numbers they
//!   are, not as the positions they describe.
//! - Level 4 on a VAX or a Cray, whose floating point is neither of the two
//!   IEEE layouts. The machine digit is read and named; the numbers under it
//!   are read as IEEE and are wrong. No file like that has been seen this
//!   century.

use crate::codec::Codec;
use crate::template::{
    Encoding,
    Endian::{self, Big, Little},
    Expr as E, StrLen, Template, Ty as T, Until,
};

/// Where the two letters that say which way round the file is written sit,
/// counted from the front of the file.
pub const ENDIAN_MARKER_AT: usize = 126;

/// The first four letters of the word every header opens with, read as one
/// big-endian word: `MATL`.
const MATL: i128 = 0x4d41_544c;

/// `IM`, read big-endian: what a little-endian writer puts there.
const INTEL: i128 = 0x494d;
/// `MI`: what a big-endian writer puts there.
const MOTOROLA: i128 = 0x4d49;

/// The version a level 5 file writes, and the one a level 7.3 file writes.
const V5: i128 = 0x0100;
const V73: i128 = 0x0200;

/// The 116 bytes of text, the subsystem offset, the version and the marker.
const HEADER_LEN: i128 = 128;

/// The user block a level 7.3 file leaves in front of its HDF5 superblock.
pub const USER_BLOCK: u64 = 512;

/// The eight bytes MATLAB writes for "there is no subsystem data": spaces,
/// the same filler as the text above them.
const NO_SUBSYSTEM: i128 = 0x2020_2020_2020_2020;

/// What an element carries. The gaps are numbers the format reserved and
/// never used.
const DATA_TYPES: &[(i128, &str)] = &[
    (1, "int8"),
    (2, "uint8"),
    (3, "int16"),
    (4, "uint16"),
    (5, "int32"),
    (6, "uint32"),
    (7, "single"),
    (9, "double"),
    (12, "int64"),
    (13, "uint64"),
    (14, "array"),
    (15, "compressed"),
    (16, "utf-8"),
    (17, "utf-16"),
    (18, "utf-32"),
];

const MI_MATRIX: i128 = 14;
const MI_COMPRESSED: i128 = 15;

/// What MATLAB hands back for an array, which is not the same question as
/// what its numbers are written as.
const CLASSES: &[(i128, &str)] = &[
    (1, "cell"),
    (2, "struct"),
    (3, "object"),
    (4, "char"),
    (5, "sparse"),
    (6, "double"),
    (7, "single"),
    (8, "int8"),
    (9, "uint8"),
    (10, "int16"),
    (11, "uint16"),
    (12, "int32"),
    (13, "uint32"),
    (14, "int64"),
    (15, "uint64"),
    (16, "function"),
    (17, "opaque"),
    (18, "object"),
];

const MX_CELL: i128 = 1;
const MX_STRUCT: i128 = 2;
const MX_OBJECT: i128 = 3;
const MX_CHAR: i128 = 4;
const MX_SPARSE: i128 = 5;
const MX_OPAQUE: i128 = 17;
const MX_OBJECT_NEW: i128 = 18;

pub fn mat() -> Template {
    let mut t = Template::new("mat", root());
    for e in [Little, Big] {
        t = t.with_type(element_name(e), element(e, false));
        t = t.with_type(text_element_name(e), element(e, true));
    }
    t
}

/// Whether there is a header at all, and then which way round it says the
/// file is. The word at the front settles the first question, because a level
/// 4 file opens with a number below 4053 and cannot be mistaken for letters;
/// only once that is answered is it safe to look 126 bytes in for the second,
/// since a level 4 file may be shorter than that.
fn root() -> T {
    T::switch(E::peek(32, Big), vec![(MATL, level5_either())], level4())
}

/// A file with a header, read whichever way its two letters say. Neither is a
/// file this can read: the header is there and says something else.
fn level5_either() -> T {
    T::switch(
        E::peek_at(E::lit(ENDIAN_MARKER_AT as i128 * 8), 16, Big),
        vec![(INTEL, level5(Little)), (MOTOROLA, level5(Big))],
        T::bytes(E::Remaining),
    )
}

fn element_name(e: Endian) -> &'static str {
    match e {
        Little => "mat.Element.le",
        Big => "mat.Element.be",
    }
}

/// The same element, with a run of numbers read as the text it is. What a
/// `mxCHAR` array's values are: MATLAB writes them as UTF-16 code units, or
/// as bytes when they all fit in one.
fn text_element_name(e: Endian) -> &'static str {
    match e {
        Little => "mat.Text.le",
        Big => "mat.Text.be",
    }
}

// ---------------------------------------------------------------- level 5

fn level5(e: Endian) -> T {
    T::structure(
        "MAT-file",
        vec![
            ("header", header(e)),
            (
                "body",
                T::switch(
                    E::within(&["header", "version"]),
                    vec![(V73, hdf5_body())],
                    T::repeat(T::Named(element_name(e).into()), Until::End).counted_as("element"),
                ),
            ),
        ],
    )
}

fn header(e: Endian) -> T {
    T::structure(
        "Header",
        vec![
            // What wrote the file, which platform it ran on and when. MATLAB
            // pads it to its full width with spaces.
            ("description", T::text(StrLen::Fixed(E::lit(116)), Encoding::Ascii)),
            // Where the objects of a class live, for a file that has any.
            // Spaces mean it has none, which is nearly every file.
            ("subsystem_offset", T::unset_int(T::u64(e), NO_SUBSYSTEM)),
            ("version", T::enumeration_hex("Version", T::u16(e), &[(V5, "level 5"), (V73, "level 7.3, HDF5")])),
            ("endian_marker", T::text(StrLen::Fixed(E::lit(2)), Encoding::Ascii)),
        ],
    )
}

/// A level 7.3 file's contents: the rest of the user block, and then the HDF5
/// file that starts at 512. See the note at the top for why it stops there.
fn hdf5_body() -> T {
    T::structure(
        "HDF5",
        vec![
            ("user_block", T::bytes(E::lit(USER_BLOCK as i128 - HEADER_LEN).at_most(E::Remaining))),
            ("signature", T::magic(b"\x89HDF\r\n\x1a\n")),
            ("file", T::bytes(E::Remaining)),
        ],
    )
}

/// One element: a tag and what it counts.
///
/// The tag has two forms and the top half of its first word tells them apart.
/// Nonzero means the short one, where that half is the byte count and the
/// data is the four bytes after the tag; zero means the long one, where the
/// count is a word of its own and the data follows eight bytes in.
fn element(e: Endian, as_text: bool) -> T {
    T::switch(
        E::peek(32, e).shr(E::lit(16)).at_most(E::lit(1)),
        vec![(1, short_element(e, as_text))],
        long_element(e, as_text),
    )
}

fn long_element(e: Endian, as_text: bool) -> T {
    T::structure_named(
        "Element",
        "type",
        "data",
        vec![
            ("type", T::enumeration("DataType", T::u32(e), DATA_TYPES)),
            ("bytes", T::u32(e)),
            ("data", body(e, E::field("bytes"), as_text)),
            // Every element but a compressed one is padded out to a multiple
            // of eight, and the next tag starts after the padding.
            (
                "padding",
                T::switch(
                    E::field("type"),
                    vec![(MI_COMPRESSED, T::bytes(E::lit(0)))],
                    T::bytes(E::field("bytes").pad_to(8).at_most(E::Remaining)),
                ),
            ),
        ],
    )
    .machinery(&["padding"])
}

/// The short form: byte count and type packed into one word, and four bytes
/// of data after it. Four bytes is the whole of the form, so what is not
/// data is padding.
fn short_element(e: Endian, as_text: bool) -> T {
    T::structure_named(
        "Element",
        "type",
        "data",
        vec![
            ("type", T::enumeration("DataType", T::uint_expr(E::lit(16), e), DATA_TYPES)),
            ("bytes", T::u16(e)),
            ("data", body(e, E::field("bytes").at_most(E::lit(4)), as_text)),
            ("padding", T::bytes(E::lit(4).sub(E::field("bytes").at_most(E::lit(4))))),
        ],
    )
    .machinery(&["padding"])
}

/// What an element's bytes read as, by the type in its tag. `size` is the
/// byte count, which the two tag forms hold in different places.
fn body(e: Endian, size: E, as_text: bool) -> T {
    let n = |width: i128| size.clone().div(E::lit(width));
    let run = |elem: T, width: i128| T::array(elem, n(width));
    let chars = |enc: Encoding| T::text(StrLen::Fixed(size.clone()), enc);
    // A char array's numbers are the characters they stand for, and so are an
    // array's name and a structure's field names. Everything else is read as
    // the numbers it is.
    let (signed_eight, eight, sixteen) = match as_text {
        true => (chars(Encoding::Latin1), chars(Encoding::Latin1), chars(Encoding::Utf16(e))),
        false => (run(T::Int { bits: 8, endian: e }, 1), run(T::u8(), 1), run(T::u16(e), 2)),
    };
    T::switch(
        E::field("type"),
        vec![
            (1, signed_eight),
            (2, eight),
            (3, run(T::Int { bits: 16, endian: e }, 2)),
            (4, sixteen),
            (5, run(T::i32(e), 4)),
            (6, run(T::u32(e), 4)),
            (7, run(T::F32(e), 4)),
            (9, run(T::F64(e), 8)),
            (12, run(T::Int { bits: 64, endian: e }, 8)),
            (13, run(T::u64(e), 8)),
            (MI_MATRIX, T::sized(size.clone(), matrix(e))),
            // A zlib stream holding one element, which is where MATLAB 7 puts
            // every variable a file has.
            (MI_COMPRESSED, T::decoded(size.clone(), Codec::Zlib, T::Named(element_name(e).into()))),
            (16, chars(Encoding::Utf8)),
            (17, chars(Encoding::Utf16(e))),
            (18, run(T::u32(e), 4)),
        ],
        T::bytes(size),
    )
}

// ------------------------------------------------------------------ arrays

/// An array: the flags that say what it is, how big it is, what it is called,
/// and then whatever its class carries.
fn matrix(e: Endian) -> T {
    T::structure(
        "Array",
        vec![
            ("flags", array_flags(e)),
            // Every class but one says how big it is here. An opaque array
            // does not: what follows its flags is the name of the variable,
            // which is the field below.
            (
                "dimensions",
                T::switch(E::within(&["flags", "class"]), vec![(MX_OPAQUE, T::bytes(E::lit(0)))], dimensions(e)),
            ),
            ("name", T::Named(text_element_name(e).into())),
            (
                "contents",
                T::switch(
                    E::within(&["flags", "class"]),
                    vec![
                        (MX_CELL, cells(e)),
                        (MX_STRUCT, fields(e, false)),
                        (MX_OBJECT, fields(e, true)),
                        (MX_OBJECT_NEW, fields(e, true)),
                        (MX_CHAR, characters(e)),
                        (MX_SPARSE, sparse(e)),
                        (MX_OPAQUE, opaque(e)),
                    ],
                    numbers(e),
                ),
            ),
        ],
    )
}

/// The first subelement, which is always the long form and always two words:
/// one packing the class and the flags, and the count of stored values a
/// sparse array keeps.
fn array_flags(e: Endian) -> T {
    T::structure(
        "ArrayFlags",
        vec![
            ("type", T::enumeration("DataType", T::u32(e), DATA_TYPES)),
            ("bytes", T::u32(e)),
            // One word holding two things: the class in its low byte, and
            // three flags nine bits up. Both are read off it rather than
            // read again, so the word itself is the only thing with bytes.
            ("packed", T::u32(e)),
            ("class", T::enumeration("Class", T::computed(E::field("packed").and(E::lit(0xff))), CLASSES)),
            (
                "flags",
                T::flags(
                    "ArrayFlag",
                    T::computed(E::field("packed").shr(E::lit(9)).and(E::lit(7))),
                    &[(0, "logical"), (1, "global"), (2, "complex")],
                ),
            ),
            // Nonzero only for a sparse array, where it is how many values
            // were made room for.
            ("nzmax", T::u32(e)),
        ],
    )
    .machinery(&["type", "bytes", "packed"])
}

/// The second subelement: one 32-bit number per dimension. Always the long
/// form, and padded like any other element, which a matrix with an odd number
/// of dimensions needs.
fn dimensions(e: Endian) -> T {
    T::structure(
        "Dimensions",
        vec![
            ("type", T::enumeration("DataType", T::u32(e), DATA_TYPES)),
            ("bytes", T::u32(e)),
            ("sizes", T::array(T::i32(e), E::field("bytes").div(E::lit(4)))),
            ("padding", T::bytes(E::field("bytes").pad_to(8).at_most(E::Remaining))),
        ],
    )
    .machinery(&["type", "bytes", "padding"])
}

/// A whether-there-is-one field, for the imaginary part every numeric class
/// may carry. Complex is bit 11 of the flags word.
fn imaginary(e: Endian) -> T {
    T::switch(
        E::within(&["flags", "packed"]).shr(E::lit(11)).and(E::lit(1)),
        vec![(1, T::Named(element_name(e).into()))],
        T::bytes(E::lit(0)),
    )
}

/// The values of a numeric array: one element of them, and a second when the
/// array is complex. Both are typed by their own tags, not by the class.
fn numbers(e: Endian) -> T {
    T::structure("Values", vec![("real", T::Named(element_name(e).into())), ("imaginary", imaginary(e))])
}

/// A char array, whose values are the characters they stand for.
fn characters(e: Endian) -> T {
    T::structure("Text", vec![("characters", T::Named(text_element_name(e).into()))])
}

/// A cell array: one element per cell, each an array of its own.
fn cells(e: Endian) -> T {
    T::structure(
        "Cells",
        vec![("cells", T::repeat(T::Named(element_name(e).into()), Until::End).counted_as("cell"))],
    )
}

/// A structure, and an object, which is a structure with a class name in
/// front of it. The names are one element of fixed-width text; the values are
/// one element each, in the order the names are in.
fn fields(e: Endian, named_class: bool) -> T {
    let mut f: Vec<(&str, T)> = Vec::new();
    if named_class {
        f.push(("class_name", T::Named(text_element_name(e).into())));
    }
    f.push(("field_name_length", T::Named(element_name(e).into())));
    f.push(("field_names", T::Named(text_element_name(e).into())));
    f.push(("fields", T::repeat(T::Named(element_name(e).into()), Until::End).counted_as("field")));
    T::structure("Struct", f)
}

/// A sparse array. `row_index` holds a row for each stored value and
/// `column_start` where each column's values begin in it, which is the
/// compressed-column layout MATLAB keeps sparse matrices in.
fn sparse(e: Endian) -> T {
    T::structure(
        "Sparse",
        vec![
            ("row_index", T::Named(element_name(e).into())),
            ("column_start", T::Named(element_name(e).into())),
            ("real", T::Named(element_name(e).into())),
            ("imaginary", imaginary(e)),
        ],
    )
}

/// An object of a class MATLAB keeps in the subsystem data, which is what a
/// `string` is. Two names say which class it is: the system that stores it,
/// which is `MCOS` for everything MATLAB has written this decade, and the
/// class itself. The array after them holds the numbers that find the object
/// in the subsystem data, which this does not follow.
fn opaque(e: Endian) -> T {
    T::structure(
        "Opaque",
        vec![
            ("storage", T::Named(text_element_name(e).into())),
            ("class_name", T::Named(text_element_name(e).into())),
            ("reference", T::Named(element_name(e).into())),
        ],
    )
}

// ---------------------------------------------------------------- level 4

/// The thousands digit of the description, which says what wrote the file and
/// with it which way round its numbers are.
const MACHINES: &[(i128, &str)] = &[
    (0, "intel, little-endian"),
    (1, "motorola, big-endian"),
    (2, "vax d-float"),
    (3, "vax g-float"),
    (4, "cray"),
];

/// The tens digit: how wide one number is.
const PRECISIONS: &[(i128, &str)] = &[
    (0, "double"),
    (1, "single"),
    (2, "int32"),
    (3, "int16"),
    (4, "uint16"),
    (5, "uint8"),
];

/// How many bytes one value takes, by precision digit.
fn width_of(p: i128) -> i128 {
    match p {
        0 => 8,
        1 | 2 => 4,
        3 | 4 => 2,
        _ => 1,
    }
}

/// The largest a description can be and still decompose: machine 4, precision
/// 5, text 2. Anything above this is not one.
pub const MOPT_LIMIT: i128 = 4052;

/// A level 4 file, whichever way round it is written. Nothing in one says,
/// so what says it is the machine digit of the first description read
/// big-endian: a file written on a big-endian machine has a digit of 1 to 4
/// there, and one written little-endian has either a 0 or, once the bytes are
/// the wrong way round, a number in the millions.
fn level4() -> T {
    let big = || T::repeat(matrix4(Big), Until::End).counted_as("matrix");
    T::switch(
        E::peek(32, Big).div(E::lit(1000)),
        vec![(1, big()), (2, big()), (3, big()), (4, big())],
        T::repeat(matrix4(Little), Until::End).counted_as("matrix"),
    )
}

/// One level 4 matrix: five integers, the name, and the numbers.
fn matrix4(e: Endian) -> T {
    // The description is decimal digits, not a packed word: thousands for the
    // machine, tens for how wide a number is, units for what kind of matrix
    // it is. There is no remainder operator, so each digit is what is left
    // after the ones above it are taken away.
    let digit = |above: i128| {
        let d = E::field("description");
        d.clone().div(E::lit(above)).sub(d.div(E::lit(above * 10)).mul(E::lit(10)))
    };
    let precision = digit(10);
    let text = digit(1);
    T::structure_named(
        "Matrix",
        "name",
        "values",
        vec![
            ("description", T::u32(e)),
            ("machine", T::enumeration("Machine", T::computed(E::field("description").div(E::lit(1000))), MACHINES)),
            ("precision", T::enumeration("Precision", T::computed(precision.clone()), PRECISIONS)),
            ("kind", T::enumeration("Kind", T::computed(text), &[(0, "numeric"), (1, "text"), (2, "sparse")])),
            ("rows", T::i32(e)),
            ("columns", T::i32(e)),
            ("imaginary_part", T::enumeration("Imaginary", T::i32(e), &[(0, "no"), (1, "yes")])),
            ("name_length", T::i32(e)),
            // The length counts the NUL that ends the name, which is the
            // format's business rather than the name's.
            ("name", T::text(StrLen::Padded { size: E::field("name_length"), pad: 0 }, Encoding::Ascii)),
            ("values", values4(e, precision.clone())),
            (
                "imaginary",
                T::switch(E::field("imaginary_part"), vec![(1, values4(e, precision))], T::bytes(E::lit(0))),
            ),
        ],
    )
    .machinery(&["description", "name_length"])
}

/// One matrix's numbers, read as whatever the precision digit says they are.
fn values4(e: Endian, precision: E) -> T {
    let count = E::field("rows").mul(E::field("columns"));
    let run = |elem: T| T::array(elem, count.clone());
    T::switch(
        precision,
        vec![
            (0, run(T::F64(e))),
            (1, run(T::F32(e))),
            (2, run(T::i32(e))),
            (3, run(T::Int { bits: 16, endian: e })),
            (4, run(T::u16(e))),
            (5, run(T::u8())),
        ],
        T::bytes(E::lit(0)),
    )
}

/// A level 5 or 7.3 file: the word MATLAB, and the two letters that say which
/// way round it is where the header puts them.
pub fn is_mat5(head: &[u8]) -> bool {
    head.starts_with(b"MATLAB")
        && matches!(head.get(ENDIAN_MARKER_AT..ENDIAN_MARKER_AT + 2), Some(b"IM") | Some(b"MI"))
}

/// A level 4 file, which says nothing about itself and has to be reasoned
/// about: five integers that have to agree with each other and with the
/// length of the file, and a name that has to be a name.
///
/// The description decomposes or it does not, the dimensions are positive and
/// their product fits, and the name is printable ASCII ending in a NUL. Five
/// constrained integers and an identifier is stronger evidence than it looks,
/// but it is still weaker than a signature, so `sniff` asks this late.
pub fn is_mat4(head: &[u8], len: u64) -> bool {
    let word = |at: usize, e: Endian| -> Option<i64> {
        let b: [u8; 4] = head.get(at..at + 4)?.try_into().ok()?;
        Some(match e {
            Little => i32::from_le_bytes(b),
            Big => i32::from_be_bytes(b),
        } as i64)
    };
    [Little, Big].iter().any(|&e| {
        let (Some(mopt), Some(rows), Some(cols), Some(imag), Some(namlen)) =
            (word(0, e), word(4, e), word(8, e), word(12, e), word(16, e))
        else {
            return false;
        };
        // The description as its digits: machine, a zero the format has never
        // used for anything, precision, and what kind of matrix it is.
        if !(0..=MOPT_LIMIT as i64).contains(&mopt) {
            return false;
        }
        let (machine, order, precision, kind) = (mopt / 1000, mopt / 100 % 10, mopt / 10 % 10, mopt % 10);
        if machine > 4 || order != 0 || precision > 5 || kind > 2 {
            return false;
        }
        // A machine digit that disagrees with the way the word was read is
        // the other endianness answering, not this one.
        if (machine == 0) != (e == Little) {
            return false;
        }
        if !(1..=64).contains(&namlen) || rows < 0 || cols < 0 || imag < 0 || imag > 1 {
            return false;
        }
        let Some(name) = head.get(20..20 + namlen as usize) else { return false };
        if name.last() != Some(&0) || !name[..name.len() - 1].iter().all(|b| b.is_ascii_graphic()) {
            return false;
        }
        // The whole matrix, which has to fit in what is left of the file.
        let values = (rows as i128) * (cols as i128) * width_of(precision as i128) * (1 + imag as i128);
        20 + namlen as i128 + values <= len as i128
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::Evaluator;
    use crate::source::MemSource;

    /// The name and value of the node at `path`, read with this template.
    fn read(bytes: &[u8], path: &[usize]) -> (String, String) {
        let doc = Document::new(MemSource(bytes.to_vec()));
        let mut ev = Evaluator::new(mat());
        let n = ev.node(&doc, path).expect("node");
        (n.name, format!("{:?}", n.value))
    }

    /// A level 5 header for a little-endian writer, with nothing after it.
    fn header_bytes(marker: &[u8; 2], version: [u8; 2]) -> Vec<u8> {
        let mut v = b"MATLAB 5.0 MAT-file, Platform: TEST".to_vec();
        v.resize(116, b' ');
        v.extend([b' '; 8]);
        v.extend(version);
        v.extend(marker);
        v
    }

    #[test]
    fn the_two_letters_say_which_way_round_the_file_is() {
        assert!(is_mat5(&header_bytes(b"IM", [0, 1])));
        assert!(is_mat5(&header_bytes(b"MI", [1, 0])));
        // The right word in the wrong place is not a header.
        let mut wrong = header_bytes(b"IM", [0, 1]);
        wrong[126] = b'X';
        assert!(!is_mat5(&wrong));
    }

    #[test]
    fn a_level_5_file_reads_its_header_and_its_elements() {
        let mut v = header_bytes(b"IM", [0, 1]);
        // One miINT32 element of two numbers, which needs no padding.
        v.extend(5u32.to_le_bytes());
        v.extend(8u32.to_le_bytes());
        v.extend(7i32.to_le_bytes());
        v.extend(9i32.to_le_bytes());
        // The header's version, and then the one element after it.
        assert_eq!(read(&v, &[0, 2]).1, "Enum { raw: 256, name: Some(\"level 5\"), hex: true }");
        let (name, value) = read(&v, &[1, 0, 0]);
        assert_eq!(name, "type");
        assert_eq!(value, "Enum { raw: 5, name: Some(\"int32\"), hex: false }");
    }

    #[test]
    fn a_level_4_matrix_is_five_integers_that_agree() {
        // Big-endian, double, numeric, 1 by 2, real, name "ab\0".
        let mut v = Vec::new();
        v.extend(1000u32.to_be_bytes());
        v.extend(1i32.to_be_bytes());
        v.extend(2i32.to_be_bytes());
        v.extend(0i32.to_be_bytes());
        v.extend(3i32.to_be_bytes());
        v.extend(b"ab\0");
        v.extend(1.5f64.to_be_bytes());
        v.extend(2.5f64.to_be_bytes());
        assert!(is_mat4(&v, v.len() as u64));
        assert_eq!(crate::formats::sniff(&v, v.len() as u64), Some("mat"));
        // Truncated: the numbers no longer fit in what is there.
        assert!(!is_mat4(&v[..v.len() - 8], v.len() as u64 - 8));
    }

    #[test]
    fn ordinary_bytes_are_not_a_level_4_matrix() {
        assert!(!is_mat4(b"this is just a sentence, and a long one at that.", 48));
        assert!(!is_mat4(&[0u8; 64], 64));
    }
}
