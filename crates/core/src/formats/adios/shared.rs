//! What every version of BP shares: the types a value is written as, the
//! header BP4 and BP5 open their files with, and byte order.

use crate::template::{Endian, Endian::*, Expr as E, Ty as T};

/// The two layouts of the records BP3 and BP4 share. BP4 added the bracketing
/// letters, and in an index entry it spends the two bytes BP3 keeps for a path
/// on an array order and a byte nobody uses.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Version {
    Bp3,
    Bp4,
}

pub(super) use Version::{Bp3, Bp4};

/// Whether an index entry is a variable's or an attribute's. The two are laid
/// out alike and differ in one record: an attribute's value can be an array,
/// and says how long in a dimensions record written before it.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Kind {
    Variable,
    Attribute,
}

/// Where a characteristic set is written. A set in an index is the whole
/// description of a block; a set in a variable's header in a data file holds
/// only its bounds, and its dimensions are in the header beside it.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Side {
    Index,
    Data,
}

/// `BPBase::DataTypes`, named by the C++ type ADIOS2 maps to each rather
/// than by ADIOS 1's names: what BP calls `long` is eight bytes.
pub(super) const DATA_TYPE: &[(i128, &str)] = &[
    (0, "int8"),
    (1, "int16"),
    (2, "int32"),
    (4, "int64"),
    (5, "float"),
    (6, "double"),
    (7, "long double"),
    (STRING, "string"),
    (10, "complex float"),
    (11, "complex double"),
    (STRING_ARRAY, "string array"),
    (50, "uint8"),
    (51, "uint16"),
    (52, "uint32"),
    (54, "uint64"),
    (55, "char"),
];

pub(super) const STRING: i128 = 9;
pub(super) const STRING_ARRAY: i128 = 12;

/// Bytes per value for the types in [`scalars`]. A long double is sixteen
/// bytes wherever ADIOS2 is built for x86-64 and is left as those bytes, since
/// what they hold differs by platform.
pub(super) const WIDTH: &[(i128, i128)] =
    &[(0, 1), (1, 2), (2, 4), (4, 8), (5, 4), (6, 8), (7, 16), (10, 8), (11, 16), (50, 1), (51, 2), (52, 4), (54, 8), (55, 1)];

/// One value of each type with a width.
pub(super) fn scalars(e: Endian) -> Vec<(i128, T)> {
    let complex = |name: &str, part: T| T::inline_structure(name, vec![("real", part.clone()), ("imaginary", part)]);
    vec![
        (0, T::Int { bits: 8, endian: e }),
        (1, T::Int { bits: 16, endian: e }),
        (2, T::i32(e)),
        (4, T::Int { bits: 64, endian: e }),
        (5, T::F32(e)),
        (6, T::F64(e)),
        (7, T::bytes(E::lit(16))),
        (10, complex("ComplexFloat", T::F32(e))),
        (11, complex("ComplexDouble", T::F64(e))),
        (50, T::u8()),
        (51, T::u16(e)),
        (52, T::u32(e)),
        (54, T::u64(e)),
        // Signed, as `char` is where ADIOS2 is built, and as its Python
        // binding writes an `int8` array: with this type rather than 0.
        (55, T::Int { bits: 8, endian: e }),
    ]
}

/// One value of the type the entry or record around this field declares. A
/// type with no width says nothing about how long its value is, so the rest
/// of the set is bytes.
pub(super) fn scalar(e: Endian) -> T {
    T::switch(E::field("data_type"), scalars(e), T::bytes(E::Remaining))
}

/// How wide one value of the declared type is, and nothing for a type with no
/// fixed width.
pub(super) fn width() -> T {
    T::switch(E::field("data_type"), WIDTH.iter().map(|(k, w)| (*k, T::computed(E::lit(*w)))).collect(), T::computed(E::lit(0)))
}

pub(super) fn data_type() -> T {
    T::enumeration("BpDataType", T::u8(), DATA_TYPE)
}

/// A length in two bytes and that many bytes of text: every name BP writes.
pub(super) fn bp_string(e: Endian) -> T {
    T::structure_named("BpString", "", "text", vec![("length", T::u16(e)), ("text", T::utf8(E::field("length")))])
}

/// `n`, never below nothing and never past what is left.
pub(super) fn clamp(n: E) -> E {
    n.at_least(E::lit(0)).at_most(E::Remaining)
}

/// `count` values of `elem`, as rows of `row` when there is more than one
/// row. The innermost dimension is the row, since ADIOS2 writes its arrays in
/// the order C does unless the process group says otherwise.
pub(super) fn shaped(elem: T, count: E, row: E) -> T {
    let row = || row.clone().at_least(E::lit(1));
    T::switch(
        count.clone().less_or_equal(row()),
        vec![(1, T::array(elem.clone(), count.clone()))],
        T::array(T::array(elem, row()), count.div(row())),
    )
}

/// The values of a block, read as the declared type when the bytes in this
/// window are exactly as many as the dimensions and the type make, and as
/// bytes otherwise. A block an operator compressed is the case this is for:
/// a data file does not say it was compressed, the index does, and the size
/// is what gives it away.
pub(super) fn block_values(e: Endian, count: E, row: E) -> T {
    let mut cases: Vec<(i128, T)> = scalars(e)
        .into_iter()
        .map(|(k, t)| {
            let w = WIDTH.iter().find(|(x, _)| *x == k).map_or(1, |(_, w)| *w);
            (k, T::switch(E::Remaining.equal_to(count.clone().mul(E::lit(w))), vec![(1, shaped(t, count.clone(), row.clone()))], T::bytes(E::Remaining)))
        })
        .collect();
    cases.push((STRING, bp_string(e)));
    T::switch(E::field("data_type"), cases, T::bytes(E::Remaining))
}

/// The byte order the first byte says, for a file whose every number after it
/// is read that way: `0` little-endian, `1` big-endian.
pub(super) const ENDIANNESS: &[(i128, &str)] = &[(0, "little-endian"), (1, "big-endian")];

/// `make`, read with the byte order a flag byte at `flag` gives.
pub(super) fn by_byte_order(flag: E, make: impl Fn(Endian) -> T) -> T {
    T::switch(flag.equal_to(E::lit(1)), vec![(1, make(Big))], make(Little))
}

/// The 64-byte header that opens every BP4 file and a BP5 index. A version
/// string saying which release wrote it and what the file is, the release
/// again as three characters, and flags. The minor release is written as a
/// character counted up from `0`, so 2.12 writes `<`.
pub(super) fn header(v5: bool) -> T {
    let digit = |name: &str| T::computed(E::field(name).sub(E::lit(b'0' as i128)));
    let mut fields = vec![
        ("version_tag", T::utf8_padded(E::lit(32), 0)),
        ("major_char", T::u8()),
        ("minor_char", T::u8()),
        ("patch_char", T::u8()),
        ("adios_major", digit("major_char")),
        ("adios_minor", digit("minor_char")),
        ("adios_patch", digit("patch_char")),
        ("unused", T::u8()),
        ("byte_order", T::enumeration("BpByteOrder", T::u8(), ENDIANNESS)),
        ("bp_version", T::u8()),
    ];
    let active = || T::enumeration("BpActive", T::u8(), &[(0, "finished"), (1, "being written")]);
    if v5 {
        fields.extend([
            ("bp_minor_version", T::u8()),
            ("active", active()),
            ("array_order", T::enumeration("BpArrayOrder", T::u8(), ARRAY_ORDER)),
            ("flatten_steps", T::u8()),
            ("reserved", T::bytes(E::lit(22))),
        ]);
    } else {
        fields.extend([("active", active()), ("reserved", T::bytes(E::lit(25)))]);
    }
    T::structure("BpHeader", fields)
}

/// `y` for Fortran's order and `n` for C's, which is how a process group says
/// it too.
pub(super) const ARRAY_ORDER: &[(i128, &str)] = &[(b'y' as i128, "column-major"), (b'n' as i128, "row-major")];
