//! FFS, the serialization BP5 writes its metadata in: format IDs, encoded
//! records, and the formats themselves as `mmd.0` keeps them.

use super::bp5::plausible_order;
use super::shared::*;
use crate::template::{Endian, Endian::*, Expr as E, Template, Ty as T, Until};

/// An FFS format ID: the version of the ID, the length of the format it names
/// divided by four, and two hashes of that format. Versions 2 and 3 are
/// twelve bytes and are what BP5 writes; version 3 spends the byte version 2
/// left unused on the top of the length.
pub(super) fn format_id() -> T {
    T::structure(
        "FfsFormatId",
        vec![
            ("version", T::u8()),
            ("rep_length_top", T::u8()),
            ("rep_length", T::u16(Big)),
            ("hash1", T::bytes(E::lit(4))),
            ("hash2", T::bytes(E::lit(4))),
            ("format_length", T::computed(E::field("rep_length_top").mul(E::lit(65536)).add(E::field("rep_length")).mul(E::lit(4)))),
        ],
    )
}

/// One FFS-encoded record: the ID of the format it is written in, eight bytes
/// saying how much follows the header, padding to eight, and the record.
///
/// The record is a C structure laid out as the writer's compiler would, and
/// the format with this ID says how: see [`ffs_schema`](super::ffs_schema),
/// which builds it from `mmd.0`. `kind` is which of that builder's two kinds
/// the record is read as. A record read where `mmd.0` is not stays bytes,
/// saying why.
///
/// Only a format with variable parts writes the length, and every format BP5
/// writes has them. A record whose length does not fit the block is left as
/// bytes after its ID.
pub(super) fn ffs_record(e: Endian, kind: &str) -> T {
    let fits = E::lit(24).less_or_equal(E::Remaining).both(E::peek_at(E::lit(96), 64, e).add(E::lit(24)).less_or_equal(E::Remaining));
    let body = T::structure(
        "FfsRecord",
        vec![
            ("format_id", format_id()),
            ("data_length", T::u64(e)),
            ("padding", T::bytes(E::lit(4))),
            ("data", T::sized(clamp(E::field("data_length")), super::ffs_schema::record_data(kind))),
            ("alignment", T::when(E::lit(0).less_than(E::Remaining), T::bytes(E::Remaining))),
        ],
    );
    let version = E::peek(8, Big);
    T::switch(
        version.clone().equal_to(E::lit(2)).either(version.equal_to(E::lit(3))).both(fits),
        vec![(1, body)],
        T::bytes(E::Remaining),
    )
}

/// A BP5 `mmd.0`: the FFS formats the records in `md.0` are written in, each a
/// length for its ID, a length for its description, the ID and the
/// description.
///
/// The description is FFS's server representation: a header, then for each
/// subformat, the structure and the structures it contains, a header of its
/// own, its fields, and the names and types those fields point at. That much
/// is documented in FFS's `fm_internal.h` and read here, so what a step's
/// metadata holds reads as field names and types even while the metadata
/// itself is bytes.
pub fn adios_bp5_metametadata() -> Template {
    Template::new("adiosbp5mmd", metametadata_root())
}

/// What [`adios_bp5_metametadata`] reads, for a template that reads `md.0`
/// with it.
pub(super) fn metametadata_root() -> T {
    plausible_order(|e| {
        let block = T::structure(
            "Bp5MetaMetadata",
            vec![
                ("id_length", T::u64(e)),
                ("description_length", T::u64(e)),
                ("id", T::sized(clamp(E::field("id_length")), T::switch(E::field("id_length"), vec![(12, format_id())], T::bytes(E::Remaining)))),
                ("description", T::sized(clamp(E::field("description_length")), format_rep())),
            ],
        );
        T::repeat(block, Until::End)
    })
}

/// FFS's server representation of a format, wire format 1: its length, byte
/// order and representation version, how many subformats, and the format and
/// each of them, the format first and not counted. The lengths are in network
/// byte order; everything inside a subformat is in the byte order it says, and
/// what follows the last is padding to eight.
pub(super) fn format_rep() -> T {
    let subformat = T::sized(
        clamp(E::peek(16, Big).add(E::peek_at(E::lit(192), 16, Big).mul(E::lit(65536)))),
        T::structure_named(
            "FfsSubformat",
            "",
            "body",
            vec![
                ("rep_length", T::u16(Big)),
                ("rep_version", T::u8()),
                ("byte_order", T::enumeration("FfsByteOrder", T::u8(), ENDIANNESS)),
                ("body", by_byte_order(E::field("byte_order"), subformat_body)),
            ],
        ),
    );
    T::structure(
        "FfsFormatRep",
        vec![
            ("rep_length", T::u16(Big)),
            ("byte_order", T::enumeration("FfsByteOrder", T::u8(), ENDIANNESS)),
            ("rep_version", T::u8()),
            ("subformat_count", T::u8()),
            ("recursive", T::u8()),
            ("rep_length_top", T::u16(Big)),
            (
                "subformats",
                T::switch(
                    E::lit(0).less_than(E::field("rep_version")),
                    vec![(1, T::array(subformat, E::field("subformat_count").add(E::lit(1))))],
                    T::bytes(E::Remaining),
                ),
            ),
            ("rest", T::when(E::lit(0).less_than(E::Remaining), T::bytes(E::Remaining))),
        ],
    )
}

/// A subformat after its first four bytes: where its name is, how many fields
/// and how long its structure is, pointer size, header size, float format,
/// where its optional information is, array order and alignment, the top
/// bytes of the two lengths, and then the fields, the strings they name, and
/// the optional information. Every offset counts from the subformat's start.
pub(super) fn subformat_body(e: Endian) -> T {
    let field = T::structure_named(
        "FfsField",
        "name",
        "",
        vec![
            ("name_offset", T::u32(e)),
            ("type_offset", T::u32(e)),
            ("size", T::i32(e)),
            ("offset", T::i32(e)),
            ("name", T::at_in_window(E::field("name_offset"), T::cstr())),
            ("type", T::at_in_window(E::field("type_offset"), T::cstr())),
        ],
    )
    // Readings of the strings the subformat body lists after its fields, the
    // way an ELF section header reads its name out of the table of them.
    .field_aside("name")
    .field_aside("type");
    // The strings run from the end of the fields to the optional information,
    // or to the end when there is none.
    let strings = E::cond(
        E::lit(0).less_than(E::field("opt_info_offset")),
        E::field("opt_info_offset").sub(E::Pos),
        E::Remaining,
    );
    T::structure_named(
        "FfsSubformatBody",
        "name",
        "fields",
        vec![
            ("name_offset", T::u32(e)),
            ("field_count", T::u32(e)),
            ("record_length", T::i32(e)),
            ("pointer_size", T::u8()),
            ("header_size", T::u8()),
            ("float_format", T::u16(e)),
            ("opt_info_offset", T::u16(e)),
            ("column_major", T::u8()),
            ("alignment", T::u8()),
            ("rep_length_top", T::u16(Big)),
            ("opt_info_offset_top", T::u16(e)),
            ("fields", T::array(field, E::field("field_count").at_most(E::Remaining.div(E::lit(16))))),
            ("name", T::at_in_window(E::field("name_offset"), T::cstr())),
            ("strings", T::sized(clamp(strings), T::repeat(T::cstr(), Until::End))),
            ("optional_info", T::when(E::lit(0).less_than(E::Remaining), T::bytes(E::Remaining))),
        ],
    )
    .field_aside("name")
}
