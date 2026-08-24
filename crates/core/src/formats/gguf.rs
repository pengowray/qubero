//! GGUF: the file a llama.cpp model is loaded from.
//!
//! A header, then two lists: the metadata, which is where a model says what
//! architecture it is and how it was trained, and one entry per tensor saying
//! where in the file its weights start. The weights themselves are the rest of
//! the file, and read as whatever the tensor's own record says they are: a run
//! of floats, or a run of quantised blocks of the size that type packs. See
//! [`weights`].
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

/// What one tensor's numbers actually are, by the type its record names.
///
/// A quantised tensor is not a run of numbers: it is a run of blocks, each
/// holding one scale (sometimes two) shared by the weights packed after it,
/// four or five or six bits apiece. Which block, and how big, is what the
/// type says, so `q4_k` is 256 weights in 144 bytes and `q8_0` is 32 in 34.
/// The blocks are all one size, so the run is counted by division and a block
/// in the middle of a tensor of a hundred thousand is one step away.
///
/// The packed weights are left as bytes. Unpacking them would mean shifting
/// four-bit fields out of a byte and scaling each by a six-bit scale that is
/// itself packed six to a byte, and no template says that; what is here says
/// where every block and every scale is, which is what a file this size is
/// usually opened to check.
///
/// Sizes and layouts are those of ggml's own block structs. The IQ types and
/// the ternary ones pack their weights in ways nothing outside ggml reads, so
/// those blocks are the right size and opaque inside.
fn weights() -> T {
    let f16 = || T::F16(Little);
    let raw = |n: i128| T::bytes(E::lit(n));
    // How many numbers this tensor holds, which is what its shape says and
    // not what the room before the next tensor says: a small tensor is
    // followed by padding out to the next 32-byte boundary, and that padding
    // is not weights.
    let count = |per_block: i128| E::product("tensors", E::idx(), &["dims"]).div(E::lit(per_block));
    // A run of plain numbers is one per weight; a run of blocks is one per
    // however many weights that type packs together.
    let run = |ty: T| T::array(ty, count(1));
    let blocks = |per_block: i128, name: &str, fields: Vec<(&str, T)>| {
        T::array(T::inline_structure(name, fields).counted_as("block"), count(per_block))
    };
    // A block nothing outside ggml unpacks: the right size, and no claim
    // about what is inside it.
    let opaque = |per_block: i128, size: i128, name: &str| blocks(per_block, name, vec![("packed", raw(size))]);
    T::switch(
        E::elem_field("tensors", E::idx(), &["type"]),
        vec![
            (0, run(T::F32(Little))),
            (1, run(f16())),
            (2, blocks(32, "Q4_0", vec![("d", f16()), ("qs", raw(16))])),
            (3, blocks(32, "Q4_1", vec![("d", f16()), ("m", f16()), ("qs", raw(16))])),
            (6, blocks(32, "Q5_0", vec![("d", f16()), ("qh", T::u32(Little)), ("qs", raw(16))])),
            (7, blocks(32, "Q5_1", vec![("d", f16()), ("m", f16()), ("qh", T::u32(Little)), ("qs", raw(16))])),
            (8, blocks(32, "Q8_0", vec![("d", f16()), ("qs", T::array(T::Int { bits: 8, endian: Little }, E::lit(32)))])),
            (9, blocks(32, "Q8_1", vec![("d", f16()), ("s", f16()), ("qs", raw(36))])),
            (10, blocks(256, "Q2_K", vec![("scales", raw(16)), ("qs", raw(64)), ("d", f16()), ("dmin", f16())])),
            (11, blocks(256, "Q3_K", vec![("hmask", raw(32)), ("qs", raw(64)), ("scales", raw(12)), ("d", f16())])),
            (12, blocks(256, "Q4_K", vec![("d", f16()), ("dmin", f16()), ("scales", raw(12)), ("qs", raw(128))])),
            (13, blocks(256, "Q5_K", vec![("d", f16()), ("dmin", f16()), ("scales", raw(12)), ("qh", raw(32)), ("qs", raw(128))])),
            (14, blocks(256, "Q6_K", vec![("ql", raw(128)), ("qh", raw(64)), ("scales", raw(16)), ("d", f16())])),
            (15, blocks(256, "Q8_K", vec![("d", T::F32(Little)), ("qs", raw(256)), ("bsums", raw(32))])),
            (16, opaque(256, 66, "IQ2_XXS")),
            (17, opaque(256, 74, "IQ2_XS")),
            (18, opaque(256, 98, "IQ3_XXS")),
            (19, opaque(256, 50, "IQ1_S")),
            (20, blocks(32, "IQ4_NL", vec![("d", f16()), ("qs", raw(16))])),
            (21, opaque(256, 110, "IQ3_S")),
            (22, opaque(256, 82, "IQ2_S")),
            (23, blocks(256, "IQ4_XS", vec![("d", f16()), ("scales_h", T::u16(Little)), ("scales_l", raw(4)), ("qs", raw(128))])),
            (24, run(T::Int { bits: 8, endian: Little })),
            (25, run(T::Int { bits: 16, endian: Little })),
            (26, run(T::i32(Little))),
            (27, run(T::Int { bits: 64, endian: Little })),
            (28, run(T::F64(Little))),
            (29, opaque(256, 56, "IQ1_M")),
            (30, run(T::BF16(Little))),
            (34, opaque(256, 54, "TQ1_0")),
            (35, opaque(256, 66, "TQ2_0")),
            (39, blocks(32, "MXFP4", vec![("e", T::u8()), ("qs", raw(16))])),
            (40, opaque(64, 36, "NVFP4")),
            (41, opaque(128, 18, "Q1_0")),
        ],
        // A type added to ggml since this was written: where it is and how
        // much of it there is, and nothing invented about the rest.
        T::bytes(E::Remaining),
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
            ("data", T::pointer_list_records("tensors", "offset", Anchor::SelfAligned(32), E::lit(0), T::Named("Weights".into()))),
        ],
    );
    Template::new("gguf", root)
        .with_type("Value", value())
        .with_type("Array", array())
        .with_type("Metadata", metadata())
        .with_type("Tensor", tensor())
        .with_type("Weights", weights())
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

    #[test]
    fn a_tensor_of_floats_reads_as_floats() {
        let bytes = two_tensor_file();
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(gguf());
        let first = ev.node(&d, &[6, 0]).unwrap();
        // Sixteen f32s, because that is what the record says this tensor is.
        assert_eq!(first.type_name, "f32 le[]");
        assert_eq!(first.child_count, 16);
        assert_eq!(ev.node(&d, &[6, 0, 3]).unwrap().size_bits, 32);
    }

    #[test]
    fn the_padding_after_a_small_tensor_is_not_part_of_it() {
        // Two tensors of two floats each. The second is placed 32 bytes along,
        // because that is where the next 32-byte boundary is, and the 24 bytes
        // between them are padding rather than eight more weights.
        let mut b = b"GGUF".to_vec();
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(&2u64.to_le_bytes()); // tensors
        b.extend_from_slice(&0u64.to_le_bytes()); // metadata entries
        for (name, offset) in [("a.weight", 0u64), ("b.weight", 32)] {
            b.extend_from_slice(&gstr(name));
            b.extend_from_slice(&1u32.to_le_bytes());
            b.extend_from_slice(&2u64.to_le_bytes()); // two numbers
            b.extend_from_slice(&0u32.to_le_bytes()); // f32
            b.extend_from_slice(&offset.to_le_bytes());
        }
        b.resize(b.len().div_ceil(32) * 32 + 40, 0);
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(gguf());
        let first = ev.node(&d, &[6, 0]).unwrap();
        assert_eq!((first.child_count, first.size_bits), (2, 8 * 8));
        // The padding belongs to no field, and reads as a gap.
        let after = first.offset_bits + first.size_bits;
        let gap = ev.spans(&d, after, after + 24 * 8, 2).unwrap();
        assert!(gap[0].gap);
        assert_eq!(gap[0].size_bits, 24 * 8);
    }

    /// One tensor of the named ggml type, holding `payload` as its weights.
    fn one_tensor_file(ty: u32, payload: &[u8]) -> Vec<u8> {
        let mut b = b"GGUF".to_vec();
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(&1u64.to_le_bytes()); // tensors
        b.extend_from_slice(&0u64.to_le_bytes()); // metadata entries
        b.extend_from_slice(&gstr("blk.0.ffn_up.weight"));
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&64u64.to_le_bytes());
        b.extend_from_slice(&ty.to_le_bytes());
        b.extend_from_slice(&0u64.to_le_bytes());
        b.resize(b.len().div_ceil(32) * 32, 0);
        b.extend_from_slice(payload);
        b
    }

    #[test]
    fn a_quantised_tensor_reads_as_the_blocks_its_type_packs() {
        // Two q8_0 blocks: a scale and thirty-two weights apiece, 34 bytes each.
        let mut payload = Vec::new();
        for block in 0..2u8 {
            payload.extend_from_slice(&[0x00, 0x3c]); // f16 1.0
            payload.extend((0..32).map(|i| i + block));
        }
        let d = Document::new(MemSource(one_tensor_file(8, &payload)));
        let mut ev = Evaluator::new(gguf());
        let t = ev.node(&d, &[6, 0]).unwrap();
        assert_eq!((t.type_name.as_str(), t.child_count), ("Q8_0[]", 2));
        // The scale of the second block, and the weight after the last one it
        // holds, which is where the block after it starts.
        assert_eq!(ev.node(&d, &[6, 0, 1, 0]).unwrap().value, Value::Float(1.0));
        assert_eq!(ev.node(&d, &[6, 0, 1, 1, 31]).unwrap().value, Value::Int(32));
        assert_eq!(ev.node(&d, &[6, 0, 1]).unwrap().size_bits, 34 * 8);
    }

    #[test]
    fn a_list_says_what_its_children_are_called() {
        // A run of numbers holds values and a run of quantised weights holds
        // blocks. Only the format knows the second word.
        let mut payload = Vec::new();
        for _ in 0..2 {
            payload.extend_from_slice(&[0x00, 0x3c]);
            payload.extend(std::iter::repeat_n(0u8, 32));
        }
        let d = Document::new(MemSource(one_tensor_file(8, &payload)));
        let mut ev = Evaluator::new(gguf());
        assert_eq!(ev.node(&d, &[6, 0]).unwrap().unit.as_deref(), Some("block"));
        // The weights inside one of those blocks are numbers, and so values.
        assert_eq!(ev.node(&d, &[6, 0, 0, 1]).unwrap().unit.as_deref(), Some("value"));
        // A tensor of plain floats is values too, and the tensor table itself
        // is a list of records the format has no word for.
        let d = Document::new(MemSource(one_tensor_file(0, &[0; 64 * 4])));
        let mut ev = Evaluator::new(gguf());
        assert_eq!(ev.node(&d, &[6, 0]).unwrap().unit.as_deref(), Some("value"));
        assert_eq!(ev.node(&d, &[5]).unwrap().unit, None);
    }

    #[test]
    fn a_bf16_tensor_reads_as_brain_floats() {
        // Two brain floats, which are two bytes each like a half float and
        // hold quite different numbers in them.
        let mut payload = 0x3f80u16.to_le_bytes().to_vec();
        payload.extend_from_slice(&0xbe59u16.to_le_bytes());
        payload.resize(64 * 2, 0); // the record says sixty-four of them
        let d = Document::new(MemSource(one_tensor_file(30, &payload)));
        let mut ev = Evaluator::new(gguf());
        let t = ev.node(&d, &[6, 0]).unwrap();
        assert_eq!(t.type_name, "bf16 le[]");
        assert_eq!(ev.node(&d, &[6, 0, 0]).unwrap().value, Value::Float(1.0));
        assert_eq!(ev.node(&d, &[6, 0, 1]).unwrap().value, Value::Float(-0.212));
    }

    #[test]
    fn a_type_this_reader_does_not_know_is_still_bytes() {
        // A ggml type from after this was written: how much of it there is,
        // and nothing invented about what is inside it.
        let d = Document::new(MemSource(one_tensor_file(200, &[0; 64])));
        let mut ev = Evaluator::new(gguf());
        let t = ev.node(&d, &[6, 0]).unwrap();
        assert_eq!((t.type_name.as_str(), t.size_bits), ("bytes[]", 64 * 8));
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
        b.resize(aligned + 64 + 64, 0);
        b
    }

    #[test]
    fn a_tensors_weights_say_which_record_shaped_them() {
        use crate::eval::Role;
        let bytes = two_tensor_file();
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(gguf());
        let o = ev.origins(&d, &[6, 1]).unwrap();
        let roles: Vec<_> = o.iter().map(|x| (x.role, x.label.as_str(), x.value.as_str())).collect();
        // Placed by the offset in its record, read as the type that record
        // names, and as many numbers as its shape multiplies out to.
        assert!(roles.contains(&(Role::Position, "tensors[1].offset", "64")), "{roles:?}");
        assert!(roles.contains(&(Role::Type, "tensors[1].type", "f32")), "{roles:?}");
        assert!(roles.contains(&(Role::Count, "tensors[1].dims", "16")), "{roles:?}");
    }

    #[test]
    fn a_tensor_offset_points_at_the_weights_it_places() {
        use crate::eval::Role;
        let bytes = two_tensor_file();
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(gguf());
        let weights = ev.node(&d, &[6, 1]).unwrap().offset_bits;
        // tensors[1].offset is the `offset` field of the second record.
        let o = ev.origins(&d, &[5, 1, 4]).unwrap();
        let points = o.iter().find(|x| x.role == Role::Points).expect("points somewhere");
        assert_eq!(points.target_bits, Some(weights));
        assert_eq!(points.label, "data[1]");
    }

    #[test]
    fn a_strings_bytes_are_as_long_as_the_count_before_them() {
        use crate::eval::Role;
        let d = Document::new(MemSource(file()));
        let mut ev = Evaluator::new(gguf());
        // metadata[0].key.text is `len` bytes long.
        let o = ev.origins(&d, &[4, 0, 0, 1]).unwrap();
        assert_eq!(o.len(), 1);
        assert_eq!((o[0].role, o[0].label.as_str(), o[0].value.as_str()), (Role::Length, "len", "20"));
        assert_eq!(o[0].path, vec![4, 0, 0, 0]);
    }

    #[test]
    fn a_field_the_template_placed_outright_came_from_nowhere_else() {
        let d = Document::new(MemSource(file()));
        let mut ev = Evaluator::new(gguf());
        assert_eq!(ev.origins(&d, &[1]).unwrap(), Vec::new());
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
        // Sixteen floats each, which is what their shapes say.
        assert_eq!(first.size_bits, 64 * 8);
        assert_eq!(second.offset_bits + second.size_bits, bytes.len() as u64 * 8);
        // Named by the records whose offsets placed them.
        assert_eq!(first.name, "[0] output.weight");
        assert_eq!(second.name, "[1] blk.0.attn_q.weight");
    }
}
