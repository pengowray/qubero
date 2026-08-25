//! safetensors: a JSON header saying what the tensors are, and then the
//! tensors, with nothing in between.
//!
//! The file opens with the length of the header, then that much JSON. Every
//! member of it is one tensor: what type its weights are, what shape they
//! have, and where its bytes start and end. Those two offsets count from the
//! end of the header rather than from the start of the file.
//!
//! The header may also hold `__metadata__`, which is the file's own notes
//! rather than a tensor and points at no weights.
//!
//! There is no magic number. What marks the format is a length that makes
//! sense followed by the `{` a JSON object opens with.

use crate::template::{Anchor, Endian::*, Expr as E, Template, Ty as T};

/// How the weights of one tensor are read: what its `dtype` says they are, as
/// many of them as its `shape` comes to.
///
/// Every type here is what the safetensors reference implementation writes.
/// One it does not, or one added since, leaves the weights as the bytes the
/// header's own two offsets make room for, which is still the right size and
/// the right place.
fn weights() -> T {
    // A shape of `[]` multiplies to one, which is a tensor holding a single
    // number; a shape of `[0]` multiplies to none, which files do write.
    let count = || E::product("header", E::idx(), &["shape"]);
    let int = |bits: u32| T::Int { bits, endian: Little };
    let uint = |bits: u32| T::UInt { bits, endian: Little };
    let of = |ty: T| T::array(ty, count());
    let room = E::elem_field("header", E::idx(), &["data_offsets", "1"])
        .sub(E::elem_field("header", E::idx(), &["data_offsets", "0"]));
    T::matches(
        E::elem_field("header", E::idx(), &["dtype"]),
        vec![
            ("F64", of(T::F64(Little))),
            ("F32", of(T::F32(Little))),
            ("F16", of(T::F16(Little))),
            ("BF16", of(T::BF16(Little))),
            ("F8_E4M3", of(T::f8(true))),
            ("F8_E5M2", of(T::f8(false))),
            ("I64", of(int(64))),
            ("I32", of(int(32))),
            ("I16", of(int(16))),
            ("I8", of(int(8))),
            ("U64", of(uint(64))),
            ("U32", of(uint(32))),
            ("U16", of(uint(16))),
            ("U8", of(uint(8))),
            // One byte each, and only 0 or 1 in it.
            ("BOOL", of(uint(8))),
        ],
        T::bytes(room),
    )
}

pub fn safetensors() -> Template {
    let root = T::structure(
        "Safetensors",
        vec![
            ("header_len", T::u64(Little)),
            ("header", T::sized(E::field("header_len"), T::json())),
            // The offsets in the header count from the end of it, which is
            // where this field starts.
            (
                "tensors",
                T::pointer_list_sized(
                    "header",
                    &["data_offsets", "0"],
                    Anchor::File,
                    E::field("header_len").add(E::lit(8)),
                    T::Named("Weights".into()),
                ),
            ),
        ],
    );
    Template::new("safetensors", root).with_type("Weights", weights())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// A file of the shape safetensors writes: notes of its own, one tensor of
    /// four halves, and one of six bytes.
    fn file() -> Vec<u8> {
        let header = concat!(
            r#"{"__metadata__":{"format":"pt"},"#,
            r#""a.weight":{"dtype":"F16","shape":[2,2],"data_offsets":[0,8]},"#,
            r#""a.scale":{"dtype":"F8_E4M3","shape":[6],"data_offsets":[8,14]}}"#,
        );
        let mut b = (header.len() as u64).to_le_bytes().to_vec();
        b.extend_from_slice(header.as_bytes());
        // 1.0, 2.0, -1.0, 0.5 as halves, then six e4m3 weights.
        for h in [0x3c00u16, 0x4000, 0xbc00, 0x3800] {
            b.extend_from_slice(&h.to_le_bytes());
        }
        b.extend_from_slice(&[0x38, 0x40, 0xb8, 0x00, 0x7e, 0x77]);
        b
    }

    fn eval() -> (Document<MemSource>, Evaluator) {
        (Document::new(MemSource(file())), Evaluator::new(safetensors()))
    }

    #[test]
    fn the_header_is_as_long_as_the_number_before_it() {
        let (d, mut ev) = eval();
        let len = ev.node(&d, &[0]).unwrap();
        let header = ev.node(&d, &[1]).unwrap();
        assert_eq!(header.offset_bits, 64);
        assert_eq!(header.size_bits, len.value.as_int().unwrap() as u64 * 8);
        assert_eq!(header.type_name, "json");
    }

    #[test]
    fn every_member_of_the_header_is_a_row_of_its_own() {
        let (d, mut ev) = eval();
        assert_eq!(ev.node(&d, &[1]).unwrap().child_count, 3);
        let tensor = ev.node(&d, &[1, 1]).unwrap();
        assert_eq!(tensor.name, "a.weight");
        assert_eq!(tensor.type_name, "object");
        // Its type, its shape and where its weights are.
        assert_eq!(tensor.child_count, 3);
        assert_eq!(ev.node(&d, &[1, 1, 0]).unwrap().value, Value::Str("F16".into()));
        assert_eq!(ev.node(&d, &[1, 1, 1]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[1, 1, 2, 1]).unwrap().value, Value::Int(8));
    }

    #[test]
    fn a_value_in_the_header_sits_where_its_text_does() {
        let (d, mut ev) = eval();
        let dtype = ev.node(&d, &[1, 1, 0]).unwrap();
        let at = (dtype.offset_bits / 8) as usize;
        // The quotes are part of the value's text, so this is `"F16"`.
        assert_eq!(&file()[at..at + (dtype.size_bits / 8) as usize], b"\"F16\"");
    }

    #[test]
    fn the_weights_are_read_as_the_type_the_header_names() {
        let (d, mut ev) = eval();
        let first = ev.node(&d, &[2, 1]).unwrap();
        assert_eq!(first.name, "a.weight");
        assert_eq!(first.type_name, "f16 le[]");
        // Two by two, which is four halves and eight bytes.
        assert_eq!((first.child_count, first.size_bits), (4, 64));
        assert_eq!(ev.node(&d, &[2, 1, 1]).unwrap().value, Value::Float(2.0));
        let second = ev.node(&d, &[2, 2]).unwrap();
        assert_eq!((second.type_name.as_str(), second.child_count), ("f8 e4m3[]", 6));
        assert_eq!(ev.node(&d, &[2, 2, 0]).unwrap().value, Value::Float(1.0));
        // The largest an e4m3 weight goes, and the one value it spends on
        // not being a number.
        assert_eq!(ev.node(&d, &[2, 2, 4]).unwrap().value, Value::Float(448.0));
    }

    #[test]
    fn the_weights_start_where_the_header_ends() {
        let (d, mut ev) = eval();
        let header = ev.node(&d, &[1]).unwrap();
        let end = header.offset_bits + header.size_bits;
        assert_eq!(ev.node(&d, &[2, 1]).unwrap().offset_bits, end);
        // And the last of them ends where the file does.
        let last = ev.node(&d, &[2, 2]).unwrap();
        assert_eq!(last.offset_bits + last.size_bits, d.len_bits());
    }

    #[test]
    fn the_files_own_notes_point_at_no_weights() {
        let (d, mut ev) = eval();
        let meta = ev.node(&d, &[2, 0]).unwrap();
        assert_eq!(meta.size_bits, 0);
        // And they are still there to read, in the header.
        assert_eq!(ev.node(&d, &[1, 0, 0]).unwrap().value, Value::Str("pt".into()));
    }

    #[test]
    fn a_tensor_says_which_of_the_headers_entries_placed_it() {
        use crate::eval::Role;
        let (d, mut ev) = eval();
        let o = ev.origins(&d, &[2, 2]).unwrap();
        let seen: Vec<_> = o.iter().map(|x| (x.role, x.label.as_str(), x.value.as_str())).collect();
        assert!(seen.contains(&(Role::Position, "header[2].data_offsets.0", "8")), "{seen:?}");
        assert!(seen.contains(&(Role::Type, "header[2].dtype", "F8_E4M3")), "{seen:?}");
        assert!(seen.contains(&(Role::Count, "header[2].shape", "6")), "{seen:?}");
    }

    #[test]
    fn a_tensor_holding_nothing_at_the_very_end_is_still_placed() {
        let header = concat!(
            r#"{"a":{"dtype":"F16","shape":[2],"data_offsets":[0,4]},"#,
            r#""scaled_fp8":{"dtype":"F8_E4M3","shape":[0],"data_offsets":[4,4]}}"#,
        );
        let mut b = (header.len() as u64).to_le_bytes().to_vec();
        b.extend_from_slice(header.as_bytes());
        b.extend_from_slice(&[0; 4]);
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(safetensors());
        let empty = ev.node(&d, &[2, 1]).expect("placed");
        assert_eq!((empty.child_count, empty.size_bits), (0, 0));
        assert_eq!(empty.offset_bits, d.len_bits());
    }

    #[test]
    fn a_type_nobody_here_knows_still_takes_the_room_it_was_given() {
        let header = r#"{"x":{"dtype":"F4_E2M1","shape":[8],"data_offsets":[0,4]}}"#;
        let mut b = (header.len() as u64).to_le_bytes().to_vec();
        b.extend_from_slice(header.as_bytes());
        b.extend_from_slice(&[0; 4]);
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(safetensors());
        let t = ev.node(&d, &[2, 0]).unwrap();
        assert_eq!((t.type_name.as_str(), t.size_bits), ("bytes[]", 32));
    }
}
