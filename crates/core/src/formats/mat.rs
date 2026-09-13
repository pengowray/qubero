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
//! A structure writes its field names in one element, each padded to the same
//! width, and then one element per field in the same order. Each of those is
//! labelled with its name, `[0] stringfield`, while its path stays
//! `fields[0]`.
//!
//! A sparse array writes a row index for each value it stores, then where
//! each column's values start in that list, then the values. The first two
//! read as the numbers they are written as, and the values are read by them:
//! a list of columns, each a list of entries, each a value with its row beside
//! it. Column `k` is entries `column_start[k]` up to `column_start[k + 1]`,
//! and an entry's row is the row index at the same place, counted from nought
//! as it is written. The imaginary part of a complex one is read the same way.
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
//! sitting in the 512-byte user block that HDF5 allows, and it is read as the
//! HDF5 file it is. Every address in an HDF5 superblock counts from the base
//! address the superblock names, which is 512 here and nought in a file that
//! is only HDF5; the one layout serves both because the HDF5 is placed in an
//! origin of its own and its addresses are counted from there. See
//! [`Anchor::Origin`](crate::template::Anchor::Origin).
//!
//! The subsystem data is where MATLAB keeps objects of a class, which is what
//! a `string`, a `table` and a `datetime` all are. A variable holding one is an
//! opaque array with nothing in it but the class's name and a column of
//! numbers saying which objects it holds. The objects themselves are in the
//! subsystem: an element written after the variables, found by the offset in
//! the header, holding a run of bytes that are a small MAT file of their own.
//! That file is a structure with a field called `MCOS`, and in the field one
//! more opaque array, of class `FileWrapper__`, whose cells are a table of
//! every class and object in the file followed by the values of their
//! properties. MathWorks has never described any of it; the layout followed
//! here, and how it was checked, is written above [`file_wrapper`].
//!
//! So each class in the table is labelled with its name, each object with its
//! class, and each property with its name, and a variable's object ids are
//! read as the numbers the objects are listed by. The link goes that one way:
//! a variable is written before the table and cannot read anything in it, so
//! its row shows which object it is and not what the object holds.
//!
//! What else is not read here:
//!
//! - A property's value is read as the array it is written as, and no
//!   further. A `string` keeps its text in a column of 64-bit words, its
//!   dimensions and the length of each string first and then the UTF-16 code
//!   units packed eight bytes to a word, and it reads as those words. Nothing
//!   in a value's cell says which object's property it is; the property says
//!   which cell, by number, and the cell is written after it.
//! - An object held in a property is not an opaque array but a bare `uint32`
//!   column with the same marker, and reads as numbers.
//! - What the writeups do not know either: the two words after each class's
//!   name and after each object's class, regions 6 and 7 of the table, and
//!   the first of the three cells shared by a class.
//! - A table of a version before 4. Only version 4 has been seen, and an
//!   older table keeps every cell after the second as a plain value.
//! - The subsystem of a level 7.3 file, which is HDF5 and read as that.
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

/// Where the header keeps the subsystem offset and the version, counted from
/// the front of the file.
const SUBSYSTEM_OFFSET_AT: i128 = 116;
const VERSION_AT: i128 = 124;

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
const MX_FUNCTION: i128 = 16;
const MX_OPAQUE: i128 = 17;
const MX_OBJECT_NEW: i128 = 18;

pub fn mat() -> Template {
    // A level 7.3 file is an HDF5 one behind the header, and an HDF5 file is
    // nothing but names: every object in it is reached by address, through a
    // type that refers to itself. So the vocabulary travels with the type.
    //
    // Built once and named, since which of its shapes a level 5 file takes is
    // decided in more than one place and the HDF5 reading is most of the
    // cost of building this template.
    let hdf5 = super::hdf5::hdf5_part();
    let mut t = Template::new("mat", root()).with_part(&hdf5).with_type(HDF5_BODY, hdf5_body(hdf5.root.clone()));
    for e in [Little, Big] {
        t = t.with_type(element_name(e), element(e, false));
        t = t.with_type(text_element_name(e), element(e, true));
        t = t.with_type(file_wrapper_name(e), file_wrapper(e));
        t = t.with_type(object_reference_name(e), object_reference(e));
        t = t.with_type(array_name(e), matrix(e));
        t = t.with_type(sparse_values_name(e), by_column(e));
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

/// A level 5 file, and where its subsystem data is when it has any.
///
/// The subsystem is an element like any other, written after the variables,
/// and nothing about its tag says what it is: the header's offset is the only
/// thing that does. So the variables are read in a window that ends where the
/// offset says, and the element there is read as the subsystem. The offset is
/// looked at before the header is read, since which fields the file has
/// depends on it, and it is trusted only when it lands past the header, leaves
/// room for a tag, and finds an array or a compressed element there. Anything
/// else, spaces and nought included, is a file with no subsystem, or one this
/// cannot place, and reads as a run of elements the way it always has.
fn level5(e: Endian) -> T {
    let offset = || E::peek_at(E::lit(SUBSYSTEM_OFFSET_AT * 8), 64, e);
    let inside = E::lit(HEADER_LEN - 1).less_than(offset()).mul(offset().add(E::lit(7)).less_than(E::Remaining));
    let not_hdf5 = E::lit(1).sub(E::peek_at(E::lit(VERSION_AT * 8), 16, e).equals(E::lit(V73)));
    T::switch(inside.mul(not_hdf5), vec![(1, placed_subsystem(e))], level5_file(e, Subsystem::None))
}

/// Which of the three shapes a level 5 file's fields take.
#[derive(Clone, Copy, PartialEq)]
enum Subsystem {
    None,
    /// The subsystem is the last element, which is where MATLAB writes it.
    Last,
    /// Something follows it.
    Followed,
}

/// The element the subsystem offset lands on, when it is one: its tag says
/// how long it is, which says whether anything is written after it.
fn placed_subsystem(e: Endian) -> T {
    let offset = || E::peek_at(E::lit(SUBSYSTEM_OFFSET_AT * 8), 64, e);
    let count = || E::peek_at(offset().add(E::lit(4)).mul(E::lit(8)), 32, e);
    // A compressed element is not padded; an array is, to a multiple of eight.
    let ends = |padded: bool| {
        let end = offset().add(E::lit(8)).add(count());
        if padded { end.add(count().pad_to(8)) } else { end }
    };
    let shape = |padded: bool| {
        T::switch(
            ends(padded).less_than(E::Remaining),
            vec![(1, level5_file(e, Subsystem::Followed))],
            level5_file(e, Subsystem::Last),
        )
    };
    T::switch(
        E::peek_at(offset().mul(E::lit(8)), 32, e),
        vec![(MI_COMPRESSED, shape(false)), (MI_MATRIX, shape(true))],
        level5_file(e, Subsystem::None),
    )
}

fn level5_file(e: Endian, subsystem: Subsystem) -> T {
    let elements = || T::repeat(T::Named(element_name(e).into()), Until::End);
    let mut fields = vec![("header", header(e))];
    match subsystem {
        Subsystem::None => fields.push((
            "body",
            T::switch(E::within(&["header", "version"]), vec![(V73, T::Named(HDF5_BODY.into()))], elements()),
        )),
        Subsystem::Last | Subsystem::Followed => {
            let before = E::within(&["header", "subsystem_offset"]).sub(E::lit(HEADER_LEN));
            fields.push(("body", T::sized(before, elements())));
            fields.push(("subsystem", subsystem_element(e)));
            if subsystem == Subsystem::Followed {
                fields.push(("after_subsystem", elements()));
            }
        }
    }
    T::structure("MAT-file", fields)
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
/// file that starts where it ends.
///
/// Read as the HDF5 file it is, in an origin of its own. Every address in an
/// HDF5 superblock counts from the base address the superblock names, which is
/// 512 here and nought in a file that is only HDF5; the one layout serves both
/// because the addresses are counted from the origin rather than from the
/// front of whatever holds it.
fn hdf5_body(hdf5: T) -> T {
    T::structure(
        "MAT-file",
        vec![
            ("user_block", T::bytes(E::lit(USER_BLOCK as i128 - HEADER_LEN).at_most(E::Remaining))),
            ("hdf5", T::origin(hdf5)),
        ],
    )
}

const HDF5_BODY: &str = "mat.HDF5";

/// One element: a tag and what it counts.
///
/// The tag has two forms and the top half of its first word tells them apart.
/// Nonzero means the short one, where that half is the byte count and the
/// data is the four bytes after the tag; zero means the long one, where the
/// count is a word of its own and the data follows eight bytes in.
fn element(e: Endian, as_text: bool) -> T {
    tagged(e, &|size| body(e, size, as_text))
}

/// Either form of tag, and `data` for what its bytes read as, handed the byte
/// count wherever that form keeps it. Every element reads its bytes by
/// [`body`] except a structure's field names, which are the same tag around a
/// different reading; see [`field_names`].
fn tagged(e: Endian, data: &dyn Fn(E) -> T) -> T {
    T::switch(
        E::peek(32, e).shr(E::lit(16)).at_most(E::lit(1)),
        vec![(1, short_element(e, data))],
        long_element(e, data),
    )
}

fn long_element(e: Endian, data: &dyn Fn(E) -> T) -> T {
    T::structure_named(
        "Element",
        "type",
        "data",
        vec![
            ("type", T::enumeration("DataType", T::u32(e), DATA_TYPES)),
            ("bytes", T::u32(e)),
            ("data", data(E::field("bytes"))),
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

/// The short form: the byte count and the type packed into one word, and four
/// bytes of data after it. Which half of that word comes first is which way
/// round the file is, since the count is the top half of it: a little-endian
/// writer puts the type first and a big-endian one the count.
///
/// Four bytes is the whole of the form, so whatever the count does not reach
/// is padding, and an element of this shape is eight bytes either way.
fn short_element(e: Endian, data: &dyn Fn(E) -> T) -> T {
    let kind = || T::enumeration("DataType", T::u16(e), DATA_TYPES);
    let count = || T::u16(e);
    let mut fields = match e {
        Little => vec![("type", kind()), ("bytes", count())],
        Big => vec![("bytes", count()), ("type", kind())],
    };
    let size = E::field("bytes").at_most(E::lit(4));
    fields.push(("data", data(size.clone())));
    fields.push(("padding", T::bytes(E::lit(4).sub(size))));
    T::structure_named("Element", "type", "data", fields).machinery(&["padding"])
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
            (MI_MATRIX, nonempty(size.clone(), T::Named(array_name(e).into()))),
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
    array_of(e, contents(e, vec![], numbers(e)))
}

/// An array whose contents are `contents`, for the few places that know more
/// about an array than its class says: the subsystem, and the arrays inside
/// it that are a table rather than a variable.
fn array_of(e: Endian, contents: T) -> T {
    T::structure(
        "Array",
        vec![
            ("array_flags", array_flags(e)),
            // Every class but one says how big it is here. An opaque array
            // does not: what follows its flags is the name of the variable,
            // which is the field below.
            (
                "dimensions",
                T::switch(E::within(&["array_flags", "class"]), vec![(MX_OPAQUE, T::bytes(E::lit(0)))], dimensions(e)),
            ),
            ("name", T::Named(text_element_name(e).into())),
            ("contents", contents),
        ],
    )
}

/// What an array's class carries after its name. `special` reads a class some
/// other way than it would be read anywhere else, and `numeric` is what every
/// class this does not name reads as.
fn contents(e: Endian, special: Vec<(i128, T)>, numeric: T) -> T {
    let mut cases = special;
    for (class, reading) in [
        (MX_CELL, cells(e)),
        (MX_STRUCT, fields(e, false)),
        (MX_OBJECT, fields(e, true)),
        (MX_OBJECT_NEW, fields(e, true)),
        (MX_CHAR, characters(e)),
        (MX_SPARSE, sparse(e)),
        (MX_FUNCTION, function(e)),
        (MX_OPAQUE, opaque(e)),
    ] {
        if !cases.iter().any(|(c, _)| *c == class) {
            cases.push((class, reading));
        }
    }
    T::switch(E::within(&["array_flags", "class"]), cases, numeric)
}

/// An element that is an array, read by `array`, and anything else read the
/// way any element is.
fn array_element(e: Endian, array: &dyn Fn() -> T) -> T {
    tagged(e, &|size: E| T::switch(E::field("type"), vec![(MI_MATRIX, nonempty(size.clone(), array()))], body(e, size, false)))
}

/// An array in `size` bytes, or no array at all when there are none. MATLAB
/// writes an `miMATRIX` of no bytes where a cell holds nothing, which is what
/// the second cell of the subsystem's table always is.
fn nonempty(size: E, array: T) -> T {
    T::switch(size.clone(), vec![(0, T::bytes(E::lit(0)))], T::sized(size, array))
}

/// The first subelement, which is always the long form and always two words:
/// one holding the class and the flags, and the count of stored values a
/// sparse array keeps.
///
/// The word is a class in its low byte and three flags in the byte above it,
/// so which of those two bytes comes first is which way round the file is.
/// Read as two bytes rather than as one word and a pair of shifts, so that
/// each row covers the byte it is talking about.
fn array_flags(e: Endian) -> T {
    let class = || T::enumeration("Class", T::u8(), CLASSES);
    let flags = || T::flags("ArrayFlag", T::u8(), &[(1, "logical"), (2, "global"), (3, "complex")]);
    let mut fields = vec![
        ("type", T::enumeration("DataType", T::u32(e), DATA_TYPES)),
        ("bytes", T::u32(e)),
    ];
    match e {
        Little => fields.extend([("class", class()), ("flags", flags()), ("reserved", T::u16(e))]),
        Big => fields.extend([("reserved", T::u16(e)), ("flags", flags()), ("class", class())]),
    }
    // Nonzero only for a sparse array, where it is how many values were made
    // room for.
    fields.push(("nzmax", T::u32(e)));
    T::structure("ArrayFlags", fields).machinery(&["type", "bytes", "reserved"])
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
        E::within(&["array_flags", "flags"]).shr(E::lit(3)).and(E::lit(1)),
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
///
/// Each value is labelled with its name, `[1] doublefield`, and is still
/// `fields[1]` to anything that reaches it. A structure array writes every
/// field of its first structure, then every field of its second, and so on,
/// so the name of element `i` is name `i` counted round the list of names:
/// a 1 by 2 array of `one` and `two` has four elements, named `one`, `two`,
/// `one`, `two`. There is no remainder in an expression, so it is `i` less
/// the whole lists of names before it.
fn fields(e: Endian, named_class: bool) -> T {
    let mut f: Vec<(&str, T)> = Vec::new();
    if named_class {
        f.push(("class_name", T::Named(text_element_name(e).into())));
    }
    f.push(("field_name_length", T::Named(element_name(e).into())));
    f.push(("field_names", field_names(e)));
    f.push(("fields", T::repeat(T::Named(element_name(e).into()), Until::End).counted_as("field")));
    let names = || E::within(&["field_names", "data"]).at_least(E::lit(1));
    let which = E::idx().sub(E::idx().div(names()).mul(names()));
    T::structure("Struct", f).field_elem_named_from("fields", E::elem_within(&["field_names", "data"], which, &[]))
}

/// A structure's field names: one element of text, each name padded with NULs
/// to the width the element before it gives, and read as that many names
/// rather than as one run. One run would read `one\0two\0`, and a label has to
/// be able to reach name `i` on its own.
///
/// MATLAB writes the names as `miINT8`. Bytes and UTF-8 are read the same way
/// in case a writer picks one of those; anything else keeps its bytes, and the
/// fields keep their bare indices.
fn field_names(e: Endian) -> T {
    // The width is the one number in the element before, and never nothing, so
    // a file that says nought divides by one rather than failing.
    let width = E::elem_within(&["field_name_length", "data"], E::lit(0), &[]).at_least(E::lit(1));
    tagged(e, &|size: E| {
        let names = |enc: Encoding| {
            let name = T::text(StrLen::Padded { size: width.clone(), pad: 0 }, enc);
            // Sized, so that a count of bytes that is not a whole number of
            // names still leaves the padding after it where the tag says.
            T::sized(size.clone(), T::array(name, size.clone().div(width.clone())))
        };
        T::switch(
            E::field("type"),
            vec![(1, names(Encoding::Latin1)), (2, names(Encoding::Latin1)), (16, names(Encoding::Utf8))],
            T::bytes(size),
        )
    })
}

/// A sparse array. `row_index` holds a row for each stored value and
/// `column_start` where each column's values begin in it, which is the
/// compressed-column layout MATLAB keeps sparse matrices in.
///
/// The values are read a column at a time, the way the two lists before them
/// divide them up: column `k` is entries `column_start[k]` up to
/// `column_start[k + 1]`, and each entry is its value with its row beside it,
/// read out of `row_index` at the same place. A row is counted from nought,
/// as it is written. There is one more column start than there are columns,
/// and the number of columns is the array's second dimension.
fn sparse(e: Endian) -> T {
    T::structure(
        "Sparse",
        vec![
            ("row_index", T::Named(element_name(e).into())),
            ("column_start", T::Named(element_name(e).into())),
            ("real", T::Named(sparse_values_name(e).into())),
            (
                "imaginary",
                T::switch(
                    E::within(&["array_flags", "flags"]).shr(E::lit(3)).and(E::lit(1)),
                    vec![(1, T::Named(sparse_values_name(e).into()))],
                    T::bytes(E::lit(0)),
                ),
            ),
        ],
    )
}

/// The element types a value can be written as, and what each reads as.
fn number_types(e: Endian) -> Vec<(i128, T)> {
    vec![
        (1, T::Int { bits: 8, endian: e }),
        (2, T::u8()),
        (3, T::Int { bits: 16, endian: e }),
        (4, T::u16(e)),
        (5, T::i32(e)),
        (6, T::u32(e)),
        (7, T::F32(e)),
        (9, T::F64(e)),
        (12, T::Int { bits: 64, endian: e }),
        (13, T::u64(e)),
    ]
}

/// A sparse array's values, as columns of entries. An element of a type that
/// is not a number, which no writer produces, is bytes: reading it as an
/// element would build an array, which builds this again. Values left over
/// past the last column's end stay unread in the element.
fn by_column(e: Endian) -> T {
    let start = |k: E| E::elem_within(&["column_start", "data"], k, &[]);
    let columns = |value: T| {
        let entry = T::structure(
            "Entry",
            vec![
                ("row", T::computed(E::elem_within(&["row_index", "data"], E::field("first_entry").add(E::idx()), &[]))),
                ("value", value),
            ],
        );
        let column = T::structure(
            "Column",
            vec![
                ("first_entry", T::computed(start(E::idx()))),
                ("entries", T::array(entry, start(E::idx().add(E::lit(1))).sub(E::field("first_entry")))),
            ],
        );
        T::array(column, E::elem_within(&["dimensions", "sizes"], E::lit(1), &[]))
    };
    tagged(e, &|size: E| {
        let cases = number_types(e).into_iter().map(|(code, value)| (code, T::sized(size.clone(), columns(value)))).collect();
        T::switch(E::field("type"), cases, T::bytes(size))
    })
}

/// A function handle, which holds one array: the structure MATLAB keeps the
/// function's name, its workspace and where it was defined in.
fn function(e: Endian) -> T {
    T::structure("Function", vec![("definition", T::Named(element_name(e).into()))])
}

/// An object of a class MATLAB keeps in the subsystem data, which is what a
/// `string` is. Two names say which class it is: the system that stores it,
/// which is `MCOS` for everything MATLAB has written this decade, and the
/// class itself. The array after them holds the numbers that find the object
/// in the subsystem data.
///
/// For `MCOS` those numbers are read as what they are, an object id for each
/// object in the array and the id of its class; see [`object_reference`]. The
/// one opaque array whose class is `FileWrapper__` is the subsystem's own
/// table of every object in the file, and its array is read as that table;
/// see [`file_wrapper`]. A `java` object, or anything else, keeps its array
/// as an ordinary element.
fn opaque(e: Endian) -> T {
    let reference = T::matches(
        E::within(&["class_name", "data"]),
        vec![("FileWrapper__", T::Named(file_wrapper_name(e).into()))],
        T::matches(
            E::within(&["storage", "data"]),
            vec![("MCOS", T::Named(object_reference_name(e).into()))],
            T::Named(element_name(e).into()),
        ),
    );
    T::structure(
        "Opaque",
        vec![
            ("storage", T::Named(text_element_name(e).into())),
            ("class_name", T::Named(text_element_name(e).into())),
            ("reference", reference),
        ],
    )
}

// --------------------------------------------------------------- subsystem
//
// MathWorks has never described any of what follows. The layout is the one
// written up by the `matio` Python package (foreverallama/matio, BSD-3, its
// `docs/subsystem_data_format.md`) and read the same way by the C `matio`
// library's `mcos.c` (tbeu/matio, BSD-2), checked here against two files
// byte for byte: every region offset lands inside the table and they run in
// order, every block in a region is as long as its count says, the regions
// come out exactly as long as the offsets make them, and every index into
// the names lands on a name. What neither writeup knows is left as numbers.

fn file_wrapper_name(e: Endian) -> &'static str {
    match e {
        Little => "mat.FileWrapper.le",
        Big => "mat.FileWrapper.be",
    }
}

/// An array, which is what every `miMATRIX` element holds. Named rather than
/// spelt out where it is used: an element reads its bytes one of two ways by
/// the form of its tag, an array holds elements, and spelling the array out in
/// each place builds it over and over before a byte has been read.
fn array_name(e: Endian) -> &'static str {
    match e {
        Little => "mat.Array.le",
        Big => "mat.Array.be",
    }
}

/// A sparse array's values read a column at a time, named for the same
/// reason, and because the real and imaginary parts are the same reading.
fn sparse_values_name(e: Endian) -> &'static str {
    match e {
        Little => "mat.SparseValues.le",
        Big => "mat.SparseValues.be",
    }
}

fn object_reference_name(e: Endian) -> &'static str {
    match e {
        Little => "mat.ObjectReference.le",
        Big => "mat.ObjectReference.be",
    }
}

/// The word an `MCOS` object reference opens with.
const REFERENCE_MARKER: i128 = 0xdd00_0000;

/// The subsystem element: an array of bytes, compressed or not, whose bytes
/// are a small MAT file of their own.
fn subsystem_element(e: Endian) -> T {
    let array = || array_of(e, contents(e, vec![], subsystem_values(e)));
    tagged(e, &|size: E| {
        T::switch(
            E::field("type"),
            vec![
                (MI_COMPRESSED, T::decoded(size.clone(), Codec::Zlib, array_element(e, &array))),
                (MI_MATRIX, nonempty(size.clone(), array())),
            ],
            body(e, size, false),
        )
    })
}

/// The subsystem array's values. MATLAB writes them as `miUINT8` whatever
/// class the array says, and a run of them long enough to hold the header is
/// read as the file inside; anything else is the numbers it is.
fn subsystem_values(e: Endian) -> T {
    let run = tagged(e, &|size: E| {
        let file = E::field("type").equals(E::lit(2)).mul(E::lit(7).less_than(size.clone()));
        T::switch(file, vec![(1, T::sized(size.clone(), subsystem_file()))], body(e, size, false))
    });
    T::structure("Values", vec![("real", run), ("imaginary", imaginary(e))])
}

/// The file inside the subsystem: the last four bytes of a MAT header, which
/// are the version and the two letters that say which way round, padded to
/// eight, and then elements. MATLAB writes a structure with a field for each
/// type system the file uses, `MCOS` being the one there always is, and after
/// it, sometimes, a second subsystem file with nothing in it.
fn subsystem_file() -> T {
    let file = |e: Endian| {
        T::structure(
            "Subsystem",
            vec![
                ("version", T::enumeration_hex("Version", T::u16(e), &[(V5, "level 5")])),
                ("endian_marker", T::text(StrLen::Fixed(E::lit(2)), Encoding::Ascii)),
                ("padding", T::bytes(E::lit(4).at_most(E::Remaining))),
                ("elements", T::repeat(T::Named(element_name(e).into()), Until::End)),
            ],
        )
        .machinery(&["padding"])
    };
    T::switch(
        E::peek_at(E::lit(16), 16, Big),
        vec![(INTEL, file(Little)), (MOTOROLA, file(Big))],
        T::bytes(E::Remaining),
    )
}

/// An `MCOS` object's array: a `uint32` column whose words are the marker
/// 0xDD000000, the number of dimensions, the dimensions, one object id for
/// each object in the array, and the id of the class. An id is the object's
/// index in the subsystem's table of objects, and a class id its index in the
/// table of classes, both counted from one; see [`linking`].
///
/// A column that does not open with the marker keeps its numbers.
fn object_reference(e: Endian) -> T {
    let words = || {
        T::structure(
            "ObjectReference",
            vec![
                ("marker", T::enumeration_hex("ReferenceMarker", T::u32(e), &[(REFERENCE_MARKER, "object reference")])),
                ("dimension_count", T::u32(e)),
                ("dimensions", T::array(T::u32(e), E::field("dimension_count"))),
                ("object_ids", T::array(T::u32(e), E::product_of("dimensions"))),
                ("class_id", T::u32(e)),
            ],
        )
    };
    let run = tagged(e, &|size: E| {
        let marked = T::switch(E::peek(32, e), vec![(REFERENCE_MARKER, T::sized(size.clone(), words()))], body(e, size.clone(), false));
        // Twelve bytes is the least a reference can be, and fewer cannot be
        // peeked at.
        let room = E::field("type").equals(E::lit(6)).mul(E::lit(11).less_than(size.clone()));
        T::switch(room, vec![(1, marked)], body(e, size, false))
    });
    let values = T::structure("Values", vec![("real", run), ("imaginary", imaginary(e))]);
    array_element(e, &|| array_of(e, contents(e, vec![], values.clone())))
}

/// The `FileWrapper__` object's array, which is a cell array and the whole of
/// what the subsystem knows: a table linking every object to its class and
/// its properties in the first cell, an empty cell, one cell for each
/// property value any object has, and, in version 4 of the table, three cells
/// shared by every object of a class.
///
/// Of those three, the last holds each class's default property values and
/// the one before it each class's alias; what the first is for, neither
/// writeup knows. Only version 4 has been seen. The C library reads one shared
/// cell rather than three in the versions before it, and nothing here has
/// been checked against such a file, so an older table keeps every cell after
/// the second in `values` rather than naming cells it cannot place.
fn file_wrapper(e: Endian) -> T {
    let size = |i: i128| E::elem_within(&["dimensions", "sizes"], E::lit(i), &[]);
    let version4 = || E::within(&["linking", "data", "contents", "real", "data", "version"]).equals(E::lit(4));
    let element = || T::Named(element_name(e).into());
    let shared = || T::switch(version4(), vec![(1, element())], T::bytes(E::lit(0)));
    let cells = T::structure(
        "FileWrapper",
        vec![
            ("linking", array_element(e, &|| array_of(e, contents(e, vec![], linking_values(e))))),
            ("reserved", element()),
            ("values", T::array(element(), size(0).mul(size(1)).sub(E::lit(2)).sub(version4().mul(E::lit(3))).at_least(E::lit(0)))),
            ("unknown", shared()),
            ("class_aliases", shared()),
            ("defaults", shared()),
        ],
    );
    array_element(e, &|| array_of(e, contents(e, vec![(MX_CELL, cells.clone())], numbers(e))))
}

/// The first cell's values: `miUINT8`, and read as the table they are once
/// there are enough of them for its first forty bytes.
fn linking_values(e: Endian) -> T {
    let run = tagged(e, &|size: E| {
        let table = E::field("type").equals(E::lit(2)).mul(E::lit(39).less_than(size.clone()));
        T::switch(table, vec![(1, T::sized(size.clone(), linking(e)))], body(e, size, false))
    });
    T::structure("Values", vec![("real", run), ("imaginary", imaginary(e))])
}

/// The table in the first cell. A version, how many names there are, eight
/// offsets counted from the front of the table, the names, and then the
/// regions the offsets mark out, which MATLAB writes in the order they are
/// listed. Each region opens with an entry of zeros standing for id nought,
/// so an entry's index in its list is its id.
///
/// Every index into the names counts from one, with nought meaning none, and
/// each entry that holds one is shown with the name beside it.
fn linking(e: Endian) -> T {
    let at = |region: &str| E::within(&["offsets", region]);
    let span = |from: &str, to: &str| at(to).sub(at(from)).at_least(E::lit(0)).at_most(E::Remaining);
    let offsets = T::structure(
        "RegionOffsets",
        ["classes", "saveobj_properties", "objects", "properties", "dynamic_properties", "region_6", "region_7", "end"]
            .into_iter()
            .map(|n| (n, T::u32(e)))
            .collect(),
    );
    let name = T::text(StrLen::Terminated { end: 0, or_end: false }, Encoding::Latin1);
    T::structure(
        "LinkingMetadata",
        vec![
            ("version", T::u32(e)),
            ("name_count", T::u32(e)),
            ("offsets", offsets),
            ("names", T::array(name, E::field("name_count"))),
            (
                "names_padding",
                T::bytes(at("classes").sub(E::lit(40)).sub(E::size_of("names")).at_least(E::lit(0)).at_most(E::Remaining)),
            ),
            // Region 1: the namespace and name of each class.
            (
                "classes",
                T::sized(span("classes", "saveobj_properties"), T::array(class_entry(e), span("classes", "saveobj_properties").div(E::lit(16)))),
            ),
            // Region 2: the properties of the objects whose class saves them
            // through a `saveobj` method, which is what a `string` does, under
            // a single property called `any`.
            ("saveobj_properties", T::sized(span("saveobj_properties", "objects"), T::repeat(property_list(e), Until::End))),
            // Region 3: each object's class, and which of the two lists of
            // properties its own are in.
            ("objects", T::sized(span("objects", "properties"), T::array(object_entry(e), span("objects", "properties").div(E::lit(24))))),
            // Region 4: the properties of every other object.
            ("properties", T::sized(span("properties", "dynamic_properties"), T::repeat(property_list(e), Until::End))),
            // Region 5: the objects that are an object's dynamic properties.
            ("dynamic_properties", T::sized(span("dynamic_properties", "region_6"), T::repeat(dynamic_list(e), Until::End))),
            // Regions 6 and 7 have only ever been seen empty or as zeros.
            ("region_6", T::bytes(span("region_6", "region_7"))),
            ("region_7", T::bytes(span("region_7", "end"))),
        ],
    )
    .machinery(&["names_padding"])
}

/// The name an index into the table's names stands for, or nothing for
/// nought.
fn name_at(index: &str) -> T {
    T::switch(
        E::field(index),
        vec![(0, T::text(StrLen::Fixed(E::lit(0)), Encoding::Ascii))],
        T::computed_text(E::elem_within(&["names"], E::field(index).sub(E::lit(1)), &[])),
    )
}

fn class_entry(e: Endian) -> T {
    T::structure_named(
        "Class",
        "name",
        "",
        vec![
            ("namespace_index", T::u32(e)),
            ("name_index", T::u32(e)),
            ("unknown", T::array(T::u32(e), E::lit(2))),
            ("namespace", name_at("namespace_index")),
            ("name", name_at("name_index")),
        ],
    )
}

/// One object. Only one of the two property ids is set: `saveobj_id` counts
/// through the saved-object properties and `normal_id` through the others.
/// `dependency_id` is the last object id this one depends on, which is how a
/// nested object is found.
fn object_entry(e: Endian) -> T {
    let class = T::switch(
        E::field("class_id"),
        vec![(0, T::text(StrLen::Fixed(E::lit(0)), Encoding::Ascii))],
        T::computed_text(E::elem_within(
            &["names"],
            E::elem_within(&["classes"], E::field("class_id"), &["name_index"]).sub(E::lit(1)),
            &[],
        )),
    );
    T::structure_named(
        "Object",
        "class",
        "",
        vec![
            ("class_id", T::u32(e)),
            ("unknown", T::array(T::u32(e), E::lit(2))),
            ("saveobj_id", T::u32(e)),
            ("normal_id", T::u32(e)),
            ("dependency_id", T::u32(e)),
            ("class", class),
        ],
    )
}

/// One object's properties: how many, each as a name, a kind and a value, and
/// padding to a multiple of eight.
fn property_list(e: Endian) -> T {
    T::structure(
        "PropertyList",
        vec![
            ("count", T::u32(e)),
            ("properties", T::array(property(e), E::field("count"))),
            ("padding", T::bytes(E::size_of("properties").add(E::lit(4)).pad_to(8).at_most(E::Remaining))),
        ],
    )
    .machinery(&["padding"])
}

/// One property. What `value` is depends on the kind: for a property it is
/// the index of the cell holding the value, counted from the first cell of
/// `values`; for an attribute it is the value itself; and for an enumeration
/// member it is an index into the names.
fn property(e: Endian) -> T {
    T::structure_named(
        "Property",
        "name",
        "",
        vec![
            ("name_index", T::u32(e)),
            ("kind", T::enumeration("PropertyKind", T::u32(e), &[(0, "enumeration member"), (1, "property"), (2, "attribute")])),
            ("value", T::u32(e)),
            ("name", name_at("name_index")),
        ],
    )
}

/// One object's dynamic properties, as the ids of the objects that hold them,
/// padded to a multiple of eight.
fn dynamic_list(e: Endian) -> T {
    T::structure(
        "DynamicProperties",
        vec![
            ("count", T::u32(e)),
            ("object_ids", T::array(T::u32(e), E::field("count"))),
            ("padding", T::bytes(E::size_of("object_ids").add(E::lit(4)).pad_to(8).at_most(E::Remaining))),
        ],
    )
    .machinery(&["padding"])
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

    /// One little-endian array element: its flags, a 1 by 1 size, a name, and
    /// whatever `contents` holds.
    fn array_bytes(class: u8, name: &[u8], contents: &[u8]) -> Vec<u8> {
        array_bytes_sized(class, [1, 1], name, contents)
    }

    /// The same, `rows` by `columns`.
    fn array_bytes_sized(class: u8, [rows, columns]: [i32; 2], name: &[u8], contents: &[u8]) -> Vec<u8> {
        let mut a = Vec::new();
        a.extend(6u32.to_le_bytes());
        a.extend(8u32.to_le_bytes());
        a.extend([class, 0, 0, 0]);
        a.extend(0u32.to_le_bytes());
        a.extend(5u32.to_le_bytes());
        a.extend(8u32.to_le_bytes());
        a.extend(rows.to_le_bytes());
        a.extend(columns.to_le_bytes());
        // A short name element, four bytes of room.
        a.extend(1u16.to_le_bytes());
        a.extend((name.len() as u16).to_le_bytes());
        let mut room = name.to_vec();
        room.resize(4, 0);
        a.extend(room);
        a.extend(contents);
        let mut v = 14u32.to_le_bytes().to_vec();
        v.extend((a.len() as u32).to_le_bytes());
        v.extend(a);
        v
    }

    #[test]
    fn a_structure_labels_each_field_with_its_name() {
        // A double of one byte, which is how MATLAB writes a small whole number.
        let double = |n: u8| {
            let mut real = 2u16.to_le_bytes().to_vec();
            real.extend(1u16.to_le_bytes());
            real.extend([n, 0, 0, 0]);
            array_bytes(6, b"", &real)
        };
        let mut s = Vec::new();
        // Names four bytes wide, then `a` and `bb` padded to that.
        s.extend(5u16.to_le_bytes());
        s.extend(4u16.to_le_bytes());
        s.extend(4i32.to_le_bytes());
        s.extend(1u32.to_le_bytes());
        s.extend(8u32.to_le_bytes());
        s.extend(b"a\0\0\0bb\0\0");
        s.extend(double(7));
        s.extend(double(9));
        let mut v = header_bytes(b"IM", [0, 1]);
        v.extend(array_bytes(2, b"s", &s));
        let fields = [1, 0, 2, 3, 2];
        assert_eq!(read(&v, &[fields.as_slice(), &[0]].concat()).0, "[0] a");
        assert_eq!(read(&v, &[fields.as_slice(), &[1]].concat()).0, "[1] bb");
        // The names are read one at a time, not as `a\0\0\0bb`.
        assert_eq!(read(&v, &[1, 0, 2, 3, 1, 2, 1]).1, "Str(\"bb\")");
    }

    #[test]
    fn a_sparse_array_reads_its_values_a_column_at_a_time() {
        // 3 by 2, with 1.5 and 2.5 in rows 0 and 2 of the first column and
        // 4 in row 1 of the second.
        let element = |kind: u32, bytes: Vec<u8>| {
            let mut v = kind.to_le_bytes().to_vec();
            v.extend((bytes.len() as u32).to_le_bytes());
            let padded = bytes.len().div_ceil(8) * 8;
            v.extend(&bytes);
            v.resize(8 + padded, 0);
            v
        };
        let ints = |ns: &[i32]| ns.iter().flat_map(|n| n.to_le_bytes()).collect::<Vec<u8>>();
        let mut s = element(5, ints(&[0, 2, 1]));
        s.extend(element(5, ints(&[0, 2, 3])));
        s.extend(element(9, [1.5f64, 2.5, 4.0].iter().flat_map(|f| f.to_le_bytes()).collect()));
        let mut v = header_bytes(b"IM", [0, 1]);
        v.extend(array_bytes_sized(5, [3, 2], b"s", &s));
        let real = [1, 0, 2, 3, 2, 2];
        let at = |rest: &[usize]| read(&v, &[real.as_slice(), rest].concat());
        assert_eq!(at(&[]).1, "Composite { count: 2 }", "two columns");
        assert_eq!(at(&[0, 1]).1, "Composite { count: 2 }", "two entries in the first");
        assert_eq!(at(&[0, 1, 1, 0]).1, "Int(2)", "the second entry is in row 2");
        assert_eq!(at(&[0, 1, 1, 1]).1, "Float(2.5)");
        assert_eq!(at(&[1, 0]).1, "Int(2)", "the second column starts at entry 2");
        assert_eq!(at(&[1, 1]).1, "Composite { count: 1 }");
        assert_eq!(at(&[1, 1, 0, 0]).1, "Int(1)");
        assert_eq!(at(&[1, 1, 0, 1]).1, "Float(4.0)");
    }

    /// A long-form `miINT32` element of one number, padded to sixteen bytes.
    fn int32_bytes(n: i32) -> Vec<u8> {
        let mut v = 5u32.to_le_bytes().to_vec();
        v.extend(4u32.to_le_bytes());
        v.extend(n.to_le_bytes());
        v.extend([0; 4]);
        v
    }

    #[test]
    fn the_subsystem_offset_splits_the_elements_where_it_lands() {
        // A variable, the subsystem, and a variable written after it. The
        // subsystem is an array of bytes holding the eight-byte header of a
        // MAT file with nothing after it.
        let mut run = 2u32.to_le_bytes().to_vec();
        run.extend(8u32.to_le_bytes());
        run.extend([0, 1, b'I', b'M', 0, 0, 0, 0]);
        let mut v = header_bytes(b"IM", [0, 1]);
        let offset = (v.len() + 16) as u64;
        v[116..124].copy_from_slice(&offset.to_le_bytes());
        v.extend(int32_bytes(7));
        v.extend(array_bytes(9, b"", &run));
        v.extend(int32_bytes(9));
        let fields = |v: &[u8]| {
            let doc = Document::new(MemSource(v.to_vec()));
            let mut ev = Evaluator::new(mat());
            let n = ev.node(&doc, &[]).expect("root").child_count as usize;
            (0..n).map(|i| ev.node(&doc, &[i]).expect("field").name).collect::<Vec<_>>()
        };
        assert_eq!(fields(&v), ["header", "body", "subsystem array", "after_subsystem"]);
        assert_eq!(read(&v, &[1]).1, "Composite { count: 1 }", "one variable before it");
        assert_eq!(read(&v, &[2, 2, 3, 0, 2, 1]).1, "Str(\"IM\")");
        // An offset past the end of the file places nothing.
        let mut past = v.clone();
        past[116..124].copy_from_slice(&(v.len() as u64).to_le_bytes());
        assert_eq!(fields(&past), ["header", "body"]);
        // Nor does one that lands on an element that is not an array.
        let mut number = v.clone();
        number[116..124].copy_from_slice(&128u64.to_le_bytes());
        assert_eq!(fields(&number), ["header", "body"]);
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
