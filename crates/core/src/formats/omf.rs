//! Intel/Microsoft OMF relocatable objects: a run of checksummed records.
//!
//! The record byte says what follows, the little-endian length includes the
//! final checksum, and a module normally runs from THEADR to MODEND. Names in
//! the two header records and LNAMES are length-prefixed; record kinds whose
//! subrecord grammar depends on preceding indexes remain bounded payloads.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};

const RECORDS: &[(i128, &str)] = &[
    (0x80, "THEADR"),
    (0x82, "LHEADR"),
    (0x88, "COMENT"),
    (0x8a, "MODEND"),
    (0x8b, "MODEND32"),
    (0x8c, "EXTDEF"),
    (0x8e, "TYPDEF"),
    (0x90, "PUBDEF"),
    (0x91, "PUBDEF32"),
    (0x94, "LINNUM"),
    (0x95, "LINNUM32"),
    (0x96, "LNAMES"),
    (0x98, "SEGDEF"),
    (0x99, "SEGDEF32"),
    (0x9a, "GRPDEF"),
    (0x9c, "FIXUPP"),
    (0x9d, "FIXUPP32"),
    (0xa0, "LEDATA"),
    (0xa1, "LEDATA32"),
    (0xa2, "LIDATA"),
    (0xa3, "LIDATA32"),
    (0xb0, "COMDEF"),
    (0xb2, "BAKPAT"),
    (0xb4, "LEXTDEF"),
    (0xb6, "LPUBDEF"),
    (0xb7, "LPUBDEF32"),
    (0xb8, "LCOMDEF"),
    (0xbc, "CEXTDEF"),
    (0xc2, "COMDAT"),
    (0xc3, "COMDAT32"),
    (0xc4, "LINSYM"),
    (0xc5, "LINSYM32"),
    (0xc6, "ALIAS"),
    (0xc8, "NBKPAT"),
    (0xca, "LLNAMES"),
];

pub fn omf() -> Template {
    Template::new("omf", T::repeat(record(), Until::End))
}

fn record() -> T {
    T::structure_named(
        "Record",
        "record_type",
        "contents",
        vec![
            (
                "record_type",
                T::enumeration_hex("RecordType", T::u8(), RECORDS),
            ),
            ("record_length", T::u16(Little)),
            (
                "contents",
                T::sized(E::field("record_length").sub(E::lit(1)), contents()),
            ),
            ("checksum", T::u8()),
        ],
    )
    .counted_as("record")
}

fn contents() -> T {
    T::switch(
        E::field("record_type"),
        vec![
            (0x80, name_record("TranslatorHeader")),
            (0x82, name_record("LibraryModuleHeader")),
            (0x96, names()),
        ],
        T::bytes(E::Remaining),
    )
}

fn name_record(kind: &str) -> T {
    T::structure(
        kind,
        vec![
            ("name_length", T::u8()),
            (
                "name",
                T::text(StrLen::Fixed(E::field("name_length")), Encoding::Ascii),
            ),
            ("extra", T::bytes(E::Remaining)),
        ],
    )
}

fn names() -> T {
    let name = T::structure_named(
        "Name",
        "text",
        "",
        vec![
            ("length", T::u8()),
            (
                "text",
                T::text(StrLen::Fixed(E::field("length")), Encoding::Ascii),
            ),
        ],
    )
    .counted_as("name");
    T::repeat(name, Until::End)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    #[test]
    fn header_name_and_checksum_are_separate() {
        let bytes = vec![0x80, 5, 0, 3, b'F', b'O', b'O', 0x42];
        let doc = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(omf());
        assert_eq!(
            ev.node(&doc, &[0, 2, 1]).unwrap().value,
            Value::Str("FOO".into())
        );
        assert_eq!(ev.node(&doc, &[0, 3]).unwrap().value, Value::UInt(0x42));
    }
}
