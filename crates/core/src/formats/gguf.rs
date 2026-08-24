//! GGUF: the file a llama.cpp model is loaded from.
//!
//! A header, then two lists: the metadata, which is where a model says what
//! architecture it is and how it was trained, and one entry per tensor saying
//! where in the file its weights start. The weights themselves are the rest of
//! the file, and are not described here beyond where they begin.
//!
//! Everything is little-endian. A big-endian GGUF exists for a few published
//! models, and is told apart by reading the version as a wildly wrong number
//! rather than by anything in the magic; nothing here reads one.

use crate::template::{Anchor, Endian::*, Expr as E, Template, Ty as T};

/// What a metadata value is. The number is written in the file; the name is
/// what the specification calls it.
const VALUE_TYPE: &[(i128, &str)] = &[
    (0, "uint8"),
    (1, "int8"),
    (2, "uint16"),
    (3, "int16"),
    (4, "uint32"),
    (5, "int32"),
    (6, "float32"),
    (7, "bool"),
    (8, "string"),
    (9, "array"),
    (10, "uint64"),
    (11, "int64"),
    (12, "float64"),
];

/// How one tensor's numbers are stored. The quantised types pack several
/// weights and a shared scale into a block, which is why there are so many of
/// them. 4 and 5 were a quantisation that was withdrawn, and no file that can
/// still be loaded holds one.
const GGML_TYPE: &[(i128, &str)] = &[
    (0, "f32"),
    (1, "f16"),
    (2, "q4_0"),
    (3, "q4_1"),
    (6, "q5_0"),
    (7, "q5_1"),
    (8, "q8_0"),
    (9, "q8_1"),
    (10, "q2_k"),
    (11, "q3_k"),
    (12, "q4_k"),
    (13, "q5_k"),
    (14, "q6_k"),
    (15, "q8_k"),
    (16, "iq2_xxs"),
    (17, "iq2_xs"),
    (18, "iq3_xxs"),
    (19, "iq1_s"),
    (20, "iq4_nl"),
    (21, "iq3_s"),
    (22, "iq2_s"),
    (23, "iq4_xs"),
    (24, "i8"),
    (25, "i16"),
    (26, "i32"),
    (27, "i64"),
    (28, "f64"),
    (29, "iq1_m"),
    (30, "bf16"),
];

const BOOL: &[(i128, &str)] = &[(0, "false"), (1, "true")];

/// A count of bytes and then that many bytes of UTF-8. Every name in the file
/// is written this way: keys, tensor names, and string values alike.
fn string() -> T {
    T::structure_named("String", "", "text", vec![("len", T::u64(Little)), ("text", T::utf8(E::field("len")))])
}

fn value_type() -> T {
    T::enumeration("GgufType", T::u32(Little), VALUE_TYPE)
}

/// One metadata value, whichever of the thirteen kinds `value_type` says it is.
/// An array holds its own element type, so this is reached again from inside
/// one: an array of strings is a list of the same String this uses.
fn value() -> T {
    T::switch(
        E::field("value_type"),
        vec![
            (0, T::u8()),
            (1, T::Int { bits: 8, endian: Little }),
            (2, T::u16(Little)),
            (3, T::Int { bits: 16, endian: Little }),
            (4, T::u32(Little)),
            (5, T::i32(Little)),
            (6, T::F32(Little)),
            (7, T::enumeration("Bool", T::u8(), BOOL)),
            (8, string()),
            (9, T::Named("Array".into())),
            (10, T::u64(Little)),
            (11, T::Int { bits: 64, endian: Little }),
            (12, T::F64(Little)),
        ],
        // A type the format does not define says nothing about how long the
        // value is, so there is nothing to read and nothing to guess.
        T::bytes(E::lit(0)),
    )
}

/// The element type, a count, and that many values of it.
fn array() -> T {
    T::structure(
        "Array",
        vec![
            ("value_type", value_type()),
            ("len", T::u64(Little)),
            ("items", T::array(T::Named("Value".into()), E::field("len"))),
        ],
    )
}

/// One key and its value. The key is what the row is called, since
/// `general.architecture` says far more than `[7]` does.
fn metadata() -> T {
    T::structure_named(
        "Metadata",
        "key",
        "value",
        vec![("key", string()), ("value_type", value_type()), ("value", T::Named("Value".into()))],
    )
}

/// Where one tensor is and what shape it has. `offset` counts from the start of
/// the tensor data rather than from the start of the file.
fn tensor() -> T {
    T::structure_named(
        "Tensor",
        "name",
        "",
        vec![
            ("name", string()),
            ("n_dims", T::u32(Little)),
            ("dims", T::array(T::u64(Little), E::field("n_dims"))),
            ("type", T::enumeration("GgmlType", T::u32(Little), GGML_TYPE)),
            ("offset", T::u64(Little)),
        ],
    )
}

pub fn gguf() -> Template {
    let root = T::structure(
        "GGUF",
        vec![
            ("magic", T::magic(b"GGUF")),
            ("version", T::u32(Little)),
            ("tensor_count", T::u64(Little)),
            ("metadata_count", T::u64(Little)),
            ("metadata", T::array(T::Named("Metadata".into()), E::field("metadata_count"))),
            ("tensors", T::array(T::Named("Tensor".into()), E::field("tensor_count"))),
            // The weights: one child per tensor, placed by the offsets the
            // tensor table holds and named by the records that hold them. A
            // tensor's bytes run to the start of the next one, since the file
            // stores no per-tensor size. Offsets count from here aligned to
            // `general.alignment`, which is a metadata value rather than a
            // field, so 32 is assumed; the padding before the first tensor
            // reads as a gap.
            ("data", T::pointer_list_records("tensors", "offset", Anchor::SelfAligned(32), E::lit(0), T::bytes(E::Remaining))),
        ],
    );
    Template::new("gguf", root)
        .with_type("Value", value())
        .with_type("Array", array())
        .with_type("Metadata", metadata())
        .with_type("Tensor", tensor())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn gstr(s: &str) -> Vec<u8> {
        let mut v = (s.len() as u64).to_le_bytes().to_vec();
        v.extend_from_slice(s.as_bytes());
        v
    }

    /// A whole small file: three metadata entries, one of them an array, and
    /// one tensor.
    fn file() -> Vec<u8> {
        let mut b = b"GGUF".to_vec();
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(&1u64.to_le_bytes()); // tensors
        b.extend_from_slice(&3u64.to_le_bytes()); // metadata entries
        b.extend_from_slice(&gstr("general.architecture"));
        b.extend_from_slice(&8u32.to_le_bytes());
        b.extend_from_slice(&gstr("llama"));
        b.extend_from_slice(&gstr("llama.block_count"));
        b.extend_from_slice(&4u32.to_le_bytes());
        b.extend_from_slice(&32u32.to_le_bytes());
        b.extend_from_slice(&gstr("tokenizer.ggml.tokens"));
        b.extend_from_slice(&9u32.to_le_bytes()); // an array
        b.extend_from_slice(&8u32.to_le_bytes()); // of strings
        b.extend_from_slice(&2u64.to_le_bytes());
        b.extend_from_slice(&gstr("<s>"));
        b.extend_from_slice(&gstr("</s>"));
        b.extend_from_slice(&gstr("token_embd.weight"));
        b.extend_from_slice(&2u32.to_le_bytes());
        b.extend_from_slice(&4096u64.to_le_bytes());
        b.extend_from_slice(&32000u64.to_le_bytes());
        b.extend_from_slice(&8u32.to_le_bytes()); // q8_0
        b.extend_from_slice(&0u64.to_le_bytes());
        b.extend_from_slice(&[0; 40]); // padding and the weights themselves
        b
    }

    #[test]
    fn the_header_says_how_many_of_each_there_are() {
        let d = Document::new(MemSource(file()));
        let mut ev = Evaluator::new(gguf());
        assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::UInt(3));
        assert_eq!(ev.node(&d, &[4]).unwrap().child_count, 3);
        assert_eq!(ev.node(&d, &[5]).unwrap().child_count, 1);
    }

    #[test]
    fn a_key_and_the_value_its_type_selects() {
        let d = Document::new(MemSource(file()));
        let mut ev = Evaluator::new(gguf());
        // metadata[0]: the key, then the string it selects.
        assert_eq!(ev.node(&d, &[4, 0, 0, 1]).unwrap().value, Value::Str("general.architecture".into()));
        assert_eq!(ev.node(&d, &[4, 0, 2, 1]).unwrap().value, Value::Str("llama".into()));
        // metadata[1]: a plain number, read as the type the entry names.
        let ty = ev.node(&d, &[4, 1, 1]).unwrap();
        assert_eq!(ty.value, Value::Enum { raw: 4, name: Some("uint32".into()), hex: false });
        assert_eq!(ev.node(&d, &[4, 1, 2]).unwrap().value, Value::UInt(32));
    }

    #[test]
    fn an_entry_is_named_by_its_key() {
        let d = Document::new(MemSource(file()));
        let mut ev = Evaluator::new(gguf());
        // The key is a length and then bytes, and the bytes are still the name.
        assert_eq!(ev.node(&d, &[4, 0]).unwrap().name, "[0] general.architecture");
        assert_eq!(ev.node(&d, &[5, 0]).unwrap().name, "[0] token_embd.weight");
    }

    #[test]
    fn an_array_holds_its_own_element_type() {
        let d = Document::new(MemSource(file()));
        let mut ev = Evaluator::new(gguf());
        let array = ev.node(&d, &[4, 2, 2]).unwrap();
        assert_eq!(array.type_name, "Array");
        assert_eq!(ev.node(&d, &[4, 2, 2, 1]).unwrap().value, Value::UInt(2));
        assert_eq!(ev.node(&d, &[4, 2, 2, 2]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[4, 2, 2, 2, 1, 1]).unwrap().value, Value::Str("</s>".into()));
    }

    #[test]
    fn a_tensor_says_its_shape_and_where_its_weights_start() {
        let d = Document::new(MemSource(file()));
        let mut ev = Evaluator::new(gguf());
        assert_eq!(ev.node(&d, &[5, 0, 0, 1]).unwrap().value, Value::Str("token_embd.weight".into()));
        assert_eq!(ev.node(&d, &[5, 0, 2]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[5, 0, 2, 1]).unwrap().value, Value::UInt(32000));
        let kind = ev.node(&d, &[5, 0, 3]).unwrap();
        assert_eq!(kind.value, Value::Enum { raw: 8, name: Some("q8_0".into()), hex: false });
    }

    #[test]
    fn the_weights_are_the_rest_of_the_file() {
        let d = Document::new(MemSource(file()));
        let mut ev = Evaluator::new(gguf());
        let data = ev.node(&d, &[6]).unwrap();
        assert_eq!(data.child_count, 1);
    }

    /// Two tensors: the data section has one child per tensor, placed at its
    /// offset from the aligned start, named by its record, running to the next.
    fn two_tensor_file() -> Vec<u8> {
        let mut b = b"GGUF".to_vec();
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(&2u64.to_le_bytes()); // tensors
        b.extend_from_slice(&0u64.to_le_bytes()); // metadata entries
        for (name, offset) in [("output.weight", 0u64), ("blk.0.attn_q.weight", 64)] {
            b.extend_from_slice(&gstr(name));
            b.extend_from_slice(&1u32.to_le_bytes());
            b.extend_from_slice(&16u64.to_le_bytes());
            b.extend_from_slice(&0u32.to_le_bytes()); // f32
            b.extend_from_slice(&offset.to_le_bytes());
        }
        let aligned = b.len().div_ceil(32) * 32;
        b.resize(aligned + 64 + 16, 0);
        b
    }

    #[test]
    fn each_tensors_bytes_are_a_child_of_the_data_section() {
        let bytes = two_tensor_file();
        let d = Document::new(MemSource(bytes.clone()));
        let mut ev = Evaluator::new(gguf());
        let data = ev.node(&d, &[6]).unwrap();
        assert_eq!(data.child_count, 2);

        let first = ev.node(&d, &[6, 0]).unwrap();
        let second = ev.node(&d, &[6, 1]).unwrap();
        // Placed at the aligned start of the data section, in offset order.
        assert_eq!(first.offset_bits % (32 * 8), 0);
        assert_eq!(second.offset_bits, first.offset_bits + 64 * 8);
        // The first runs to where the second starts; the last to the end.
        assert_eq!(first.size_bits, 64 * 8);
        assert_eq!(second.offset_bits + second.size_bits, bytes.len() as u64 * 8);
        // Named by the records whose offsets placed them.
        assert_eq!(first.name, "[0] output.weight");
        assert_eq!(second.name, "[1] blk.0.attn_q.weight");
    }
}
