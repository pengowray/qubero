//! Excel 97–2003 BIFF8 records, inside an OLE Workbook stream or standalone.
//! Record sizes bound every interpretation; unknown records stay as bytes.
//! MS-XLS: https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-xls/
use crate::codec::Codec;
use crate::template::{
    Deduce, Encoding, Endian::Little, Expr as E, StrLen, Template, Ty as T, Until,
};

const RECORDS: &[(i128, &str)] = &[
    (0x0809, "BOF"),
    (0x000a, "EOF"),
    (0x0085, "BoundSheet8"),
    (0x0200, "Dimensions"),
    (0x0203, "Number"),
    (0x027e, "RK"),
    (0x00bd, "MulRK"),
    (0x0205, "BoolErr"),
    (0x0201, "Blank"),
    (0x00be, "MulBlank"),
    (0x00fd, "LabelSst"),
    (0x0204, "Label"),
    (0x00fc, "SST"),
    (0x003c, "Continue"),
    (0x0006, "Formula"),
    (0x0207, "String"),
    (0x04bc, "ShrFmla"),
    (0x0221, "Array"),
    (0x002f, "FilePass (encrypted)"),
    (0x0042, "CodePage"),
    (0x00e0, "XF"),
    (0x041e, "Format"),
    (0x0208, "Row"),
];

pub fn xls() -> Template {
    Template::new(
        "xls",
        T::switch(
            E::peek_at(E::lit(0), 16, Little),
            vec![(
                0xcfd0,
                T::structure(
                    "ExcelCompoundDocument",
                    vec![
                        (
                            "container",
                            T::at(
                                E::lit(0),
                                super::thumbsdb::compound_file("CompoundDocument"),
                            ),
                        ),
                        (
                            "workbook",
                            T::decoded(E::Remaining, Codec::CfbWorkbook, workbook()),
                        ),
                    ],
                ),
            )],
            workbook(),
        ),
    )
    .deduced_by(super::xls_cells::Cells)
}

fn workbook() -> T {
    T::switch(
        E::peek_at(E::lit(32), 16, Little),
        vec![(
            0x0600,
            T::structure(
                "Workbook",
                vec![
                    (
                        "records",
                        T::repeat(
                            record(),
                            Until::FieldValue {
                                field: "record_type".into(),
                                value: 0x002f,
                            },
                        ),
                    ),
                    ("encrypted_or_unread_data", T::bytes(E::Remaining)),
                ],
            ),
        )],
        T::bytes(E::Remaining),
    )
}
fn record() -> T {
    T::structure_named(
        "BiffRecord",
        "record_type",
        "",
        vec![
            ("cell", T::computed_text(E::Deduced(Deduce::Builds))),
            (
                "record_type",
                T::enumeration("BiffRecordType", T::u16(Little), RECORDS),
            ),
            ("size", T::u16(Little)),
            (
                "body",
                T::sized(
                    E::field("size"),
                    T::switch(
                        E::field("record_type"),
                        vec![
                            (
                                0x0809,
                                T::structure(
                                    "BOF",
                                    vec![
                                        ("version", T::u16(Little)),
                                        (
                                            "substream_type",
                                            T::enumeration(
                                                "Substream",
                                                T::u16(Little),
                                                &[
                                                    (5, "workbook globals"),
                                                    (16, "worksheet"),
                                                    (32, "chart"),
                                                ],
                                            ),
                                        ),
                                        ("build", T::u16(Little)),
                                        ("year", T::u16(Little)),
                                        ("flags", T::bytes(E::Remaining)),
                                    ],
                                ),
                            ),
                            (
                                0x0085,
                                T::structure(
                                    "BoundSheet8",
                                    vec![
                                        ("stream_offset", T::u32(Little)),
                                        ("visibility", T::u8()),
                                        ("sheet_type", T::u8()),
                                        ("name", unicode(true)),
                                    ],
                                ),
                            ),
                            (0x0203, cell("Number", vec![("value", T::F64(Little))])),
                            (0x027e, cell("RK", vec![("encoded_number", T::u32(Little))])),
                            (
                                0x00fd,
                                cell("LabelSst", vec![("string_index", T::u32(Little))]),
                            ),
                            (0x0204, cell("Label", vec![("value", unicode(false))])),
                            (0x0201, cell("Blank", vec![])),
                            (
                                0x0205,
                                cell("BoolErr", vec![("value", T::u8()), ("is_error", T::u8())]),
                            ),
                            (
                                0x00bd,
                                T::structure(
                                    "MulRK",
                                    vec![
                                        ("row", T::u16(Little)),
                                        ("first_column", T::u16(Little)),
                                        (
                                            "cells",
                                            T::array(
                                                T::structure(
                                                    "RK",
                                                    vec![
                                                        ("format_index", T::u16(Little)),
                                                        ("encoded_number", T::u32(Little)),
                                                    ],
                                                ),
                                                E::Remaining.sub(E::lit(2)).div(E::lit(6)),
                                            ),
                                        ),
                                        ("last_column", T::u16(Little)),
                                    ],
                                ),
                            ),
                            (
                                0x0006,
                                cell(
                                    "Formula",
                                    vec![
                                        (
                                            "cached_result",
                                            T::switch(
                                                E::peek_at(E::lit(48), 16, Little),
                                                vec![(
                                                    0xffff,
                                                    T::structure(
                                                        "SpecialResult",
                                                        vec![
                                                            (
                                                                "type",
                                                                T::enumeration(
                                                                    "ResultType",
                                                                    T::u8(),
                                                                    &[
                                                                        (0, "string follows"),
                                                                        (1, "boolean"),
                                                                        (2, "error"),
                                                                        (3, "empty"),
                                                                    ],
                                                                ),
                                                            ),
                                                            ("reserved", T::u8()),
                                                            ("value", T::u8()),
                                                            ("padding", T::bytes(E::lit(5))),
                                                        ],
                                                    ),
                                                )],
                                                T::F64(Little),
                                            ),
                                        ),
                                        ("flags", T::u16(Little)),
                                        ("calculation_cache", T::u32(Little)),
                                        ("token_size", T::u16(Little)),
                                        ("tokens", T::bytes(E::field("token_size"))),
                                        ("extra_data", T::bytes(E::Remaining)),
                                    ],
                                ),
                            ),
                            (0x0207, unicode(false)),
                            (
                                0x00fc,
                                T::structure(
                                    "SharedStringTable",
                                    vec![
                                        ("total_count", T::u32(Little)),
                                        ("unique_count", T::u32(Little)),
                                        ("strings_and_formatting", T::bytes(E::Remaining)),
                                    ],
                                ),
                            ),
                        ],
                        T::bytes(E::Remaining),
                    ),
                ),
            ),
        ],
    )
}
fn cell(name: &str, tail: Vec<(&str, T)>) -> T {
    let mut fields = vec![
        ("row", T::u16(Little)),
        ("column", T::u16(Little)),
        ("format_index", T::u16(Little)),
    ];
    fields.extend(tail);
    T::structure(name, fields)
}
fn unicode(short: bool) -> T {
    T::structure(
        "UnicodeString",
        vec![
            (
                "character_count",
                if short { T::u8() } else { T::u16(Little) },
            ),
            ("flags", T::u8()),
            (
                "text",
                T::switch(
                    E::field("flags").and(E::lit(1)),
                    vec![(
                        1,
                        T::text(
                            StrLen::Fixed(E::field("character_count").mul(E::lit(2))),
                            Encoding::Utf16(Little),
                        ),
                    )],
                    T::text(StrLen::Fixed(E::field("character_count")), Encoding::Latin1),
                ),
            ),
        ],
    )
}
