//! Google Draco: a compressed 3D mesh or point cloud.
//!
//! The header is plain bytes and the optional metadata section is varints and
//! length-prefixed strings, so both are decoded in full. What follows is
//! connectivity and then attributes, almost all of it rANS entropy-coded with
//! no lengths written anywhere a template could reach, so after decoding the
//! few plain-integer fields at the front of the connectivity section the rest
//! is left as one labelled block.
//!
//! Draco's varints are unsigned LEB128, which the IR already reads.

use crate::template::{Endian::*, Expr as E, Template, Ty as T};

/// What kind of geometry the file holds.
const ENCODER_TYPE: &[(i128, &str)] = &[(0, "point cloud"), (1, "triangular mesh")];

/// How the geometry was encoded. The number means one thing for a mesh and
/// another for a point cloud; both readings are shown.
const ENCODER_METHOD: &[(i128, &str)] = &[
    (0, "sequential"),
    (1, "edgebreaker (mesh) / kd-tree (point cloud)"),
];

/// Edgebreaker traversal decoders. 1 existed once and was removed.
const TRAVERSAL: &[(i128, &str)] = &[
    (0, "standard"),
    (1, "predictive (deprecated)"),
    (2, "valence"),
];

/// One name/value pair of a metadata block. The value is raw bytes: Draco
/// writes strings, integers and doubles into it without saying which.
fn entry() -> T {
    T::structure_named(
        "Entry",
        "name",
        "value",
        vec![
            ("name_len", T::u8()),
            ("name", T::utf8(E::field("name_len"))),
            ("value_len", T::u8()),
            ("value", T::bytes(E::field("value_len"))),
        ],
    )
}

/// A metadata block: entries, then named sub-blocks of the same shape.
fn metadata() -> T {
    T::structure(
        "Metadata",
        vec![
            ("num_entries", T::leb_u()),
            ("entries", T::array(entry(), E::field("num_entries"))),
            ("num_sub_metadata", T::leb_u()),
            (
                "sub_metadata",
                T::array(
                    T::structure_named(
                        "SubMetadata",
                        "name",
                        "metadata",
                        vec![
                            ("name_len", T::u8()),
                            ("name", T::utf8(E::field("name_len"))),
                            ("metadata", T::Named("draco.Metadata".into())),
                        ],
                    ),
                    E::field("num_sub_metadata"),
                ),
            ),
        ],
    )
}

/// The whole metadata section: per-attribute blocks, then the geometry's own.
fn metadata_section() -> T {
    T::structure(
        "MetadataSection",
        vec![
            ("num_att_metadata", T::leb_u()),
            (
                "att_metadata",
                T::array(
                    T::structure(
                        "AttributeMetadata",
                        vec![
                            ("att_unique_id", T::leb_u()),
                            ("metadata", T::Named("draco.Metadata".into())),
                        ],
                    ),
                    E::field("num_att_metadata"),
                ),
            ),
            ("file_metadata", T::Named("draco.Metadata".into())),
        ],
    )
}

pub fn draco() -> Template {
    // A sequentially encoded mesh opens its connectivity with two counts and
    // a method byte written in the clear; everything after them is the index
    // data (raw or symbol-coded) and then the attribute sections.
    let sequential_mesh = T::structure(
        "SequentialConnectivity",
        vec![
            ("num_faces", T::leb_u()),
            ("num_points", T::leb_u()),
            (
                "connectivity_method",
                T::enumeration("ConnectivityMethod", T::u8(), &[(0, "symbol-coded indices"), (1, "raw indices")]),
            ),
            ("data", T::bytes(E::Remaining)),
        ],
    );
    // Edgebreaker says which traversal decoder it used and then everything
    // else is entropy coded.
    let edgebreaker = T::structure(
        "EdgebreakerConnectivity",
        vec![
            ("traversal_decoder", T::enumeration("TraversalDecoder", T::u8(), TRAVERSAL)),
            ("data", T::bytes(E::Remaining)),
        ],
    );
    let body = T::switch(
        E::field("encoder_type"),
        vec![(
            1,
            T::switch(
                E::field("encoder_method"),
                vec![(0, sequential_mesh), (1, edgebreaker)],
                T::bytes(E::Remaining),
            ),
        )],
        // Point cloud connectivity and attributes: entropy coded throughout.
        T::bytes(E::Remaining),
    );
    Template::new(
        "draco",
        T::structure(
            "Draco",
            vec![
                ("magic", T::magic(b"DRACO")),
                ("major_version", T::u8()),
                ("minor_version", T::u8()),
                ("encoder_type", T::enumeration("EncoderType", T::u8(), ENCODER_TYPE)),
                ("encoder_method", T::enumeration("EncoderMethod", T::u8(), ENCODER_METHOD)),
                ("flags", T::flags("HeaderFlags", T::u16(Little), &[(15, "metadata")])),
                ("metadata", T::present_if(E::field("flags").bit(15), metadata_section())),
                ("body", body),
            ],
        ),
    )
    .with_type("draco.Metadata", metadata())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn header(encoder_type: u8, method: u8, flags: u16) -> Vec<u8> {
        let mut b = b"DRACO".to_vec();
        b.extend_from_slice(&[2, 2, encoder_type, method]);
        b.extend_from_slice(&flags.to_le_bytes());
        b
    }

    #[test]
    fn edgebreaker_header_reads_and_names_its_fields() {
        let mut b = header(1, 1, 0);
        b.push(2); // valence traversal
        b.extend_from_slice(&[0xaa; 6]); // entropy coded tail
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(draco());
        let ty = ev.node(&d, &[3]).unwrap();
        assert_eq!(ty.value, Value::Enum { raw: 1, name: Some("triangular mesh".into()), hex: false });
        let method = ev.node(&d, &[4]).unwrap();
        assert_eq!(method.type_name, "EncoderMethod");
        // No metadata flag: the metadata field covers no bytes.
        let meta = ev.node(&d, &[6]).unwrap();
        assert_eq!(meta.size_bits, 0);
        let traversal = ev.node(&d, &[7, 0]).unwrap();
        assert_eq!(traversal.value, Value::Enum { raw: 2, name: Some("valence".into()), hex: false });
        let data = ev.node(&d, &[7, 1]).unwrap();
        assert_eq!(data.size_bits, 6 * 8);
    }

    #[test]
    fn sequential_mesh_reads_its_counts() {
        let mut b = header(1, 0, 0);
        b.extend_from_slice(&[0x80, 0x02]); // num_faces = 256, as a varint
        b.push(12); // num_points
        b.push(1); // raw indices
        b.extend_from_slice(&[0; 4]);
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(draco());
        assert_eq!(ev.node(&d, &[7, 0]).unwrap().value, Value::UInt(256));
        assert_eq!(ev.node(&d, &[7, 1]).unwrap().value, Value::UInt(12));
        let method = ev.node(&d, &[7, 2]).unwrap();
        assert_eq!(method.value, Value::Enum { raw: 1, name: Some("raw indices".into()), hex: false });
    }

    #[test]
    fn metadata_section_decodes_entries_and_sub_blocks() {
        let mut b = header(1, 1, 0x8000);
        // One attribute's metadata: id 3, one entry, no sub-blocks.
        b.push(1); // num_att_metadata
        b.push(3); // att_unique_id
        b.push(1); // num_entries
        b.extend_from_slice(b"\x04name\x05chair");
        b.push(0); // num_sub_metadata
        // File metadata: no entries, one sub-block with one entry.
        b.push(0); // num_entries
        b.push(1); // num_sub_metadata
        b.extend_from_slice(b"\x03gen"); // sub-block name
        b.push(1); // its num_entries
        b.extend_from_slice(b"\x02by\x03exp");
        b.push(0); // its num_sub_metadata
        // Connectivity afterwards still lands where it should.
        b.push(0); // standard traversal
        b.extend_from_slice(&[0xbb; 3]);
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(draco());
        let name = ev.node(&d, &[6, 1, 0, 1, 1, 0, 1]).unwrap();
        assert_eq!(name.value, Value::Str("name".into()));
        let value = ev.node(&d, &[6, 1, 0, 1, 1, 0, 3]).unwrap();
        assert_eq!(value.size_bits, 5 * 8);
        let sub_name = ev.node(&d, &[6, 2, 3, 0, 1]).unwrap();
        assert_eq!(sub_name.value, Value::Str("gen".into()));
        let sub_entry_name = ev.node(&d, &[6, 2, 3, 0, 2, 1, 0, 1]).unwrap();
        assert_eq!(sub_entry_name.value, Value::Str("by".into()));
        let traversal = ev.node(&d, &[7, 0]).unwrap();
        assert_eq!(traversal.value, Value::Enum { raw: 0, name: Some("standard".into()), hex: false });
        assert_eq!(ev.node(&d, &[7, 1]).unwrap().size_bits, 3 * 8);
    }

    #[test]
    fn point_cloud_body_is_one_opaque_block() {
        let mut b = header(0, 1, 0);
        b.extend_from_slice(&[0xcc; 10]);
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(draco());
        let body = ev.node(&d, &[7]).unwrap();
        assert_eq!(body.size_bits, 10 * 8);
    }
}
