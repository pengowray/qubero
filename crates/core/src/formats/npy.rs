//! NPY: one numpy array, written as a line of Python and then the numbers.
//! The format every science pipeline saves an intermediate in, XENON and
//! IceCube among them.
//!
//! Six bytes of magic, a version, and then a header: the length of a dict
//! written as text, and that dict. Version 1 counts the header in two bytes,
//! versions 2 and 3 in four, which is what a dtype with a few hundred named
//! fields needs. The dict is padded with spaces to a multiple of 64 bytes and
//! ends in a newline, so the numbers start on a boundary a memory map likes.
//!
//! The dict is Python rather than JSON: single quotes, `False` rather than
//! `false`, and a tuple for the shape. Nothing here parses Python, so the
//! three values it holds are read as the text between the keys that surround
//! them, which is exactly how numpy writes them:
//!
//! ```text
//! {'descr': '<f8', 'fortran_order': False, 'shape': (2, 3), }
//! ```
//!
//! `descr` is what types the numbers. A dtype of one kind reads as an array of
//! that type; a structured dtype, which is written as a list of tuples, does
//! not, and the numbers stay bytes.
//!
//! What is not read here:
//!
//! - The shape is text, not a structure, so a 2 by 3 array is one run of six
//!   numbers rather than two rows of three. `fortran_order` says which way
//!   round they are and is shown for the reader to see, since nothing in the
//!   IR reshapes a list.
//! - A structured dtype, `[('a', '<i8'), ('b', '<f4')]`, is a record layout
//!   this cannot turn into fields: reading it needs a parser for Python
//!   literals, of the kind [`Ty::Json`](crate::template::Ty::Json) is for JSON.
//!   The numbers stay bytes.
//! - Complex, datetime and string dtypes stay bytes for the same reason: a
//!   `<c16` is two f64s that belong together, and nothing here pairs them.
//! - A writer that puts the keys in another order, which the format allows and
//!   numpy has never done, gets the whole dict as one run of text and its
//!   numbers as bytes.
//! - An NPZ is a ZIP of these, and reads as the ZIP it is. Typing a member as
//!   an NPY would need a ZIP entry to take a template by the name it is stored
//!   under, and nothing in the IR says that.

use crate::template::{Encoding, Endian, Endian::Little, Expr as E, StrLen, Template, Ty as T};

pub const MAGIC: &[u8] = b"\x93NUMPY";

/// The key that opens the dict, and the two that separate its values. numpy
/// writes them exactly this way, and a header that does not is read as one
/// run of text rather than misread.
const DESCR: &[u8] = b"'descr': ";
const ORDER: &[u8] = b", 'fortran_order': ";
const SHAPE: &[u8] = b", 'shape': ";

/// A fixed run of text, never longer than what is left. A header whose keys
/// are not where they are expected leaves the field before this one running to
/// the end of the dict, and a fixed length would then read past it.
fn fixed(n: i128) -> T {
    T::text(StrLen::Fixed(E::lit(n).at_most(E::Remaining)), Encoding::Ascii)
}

/// Text up to the next place `needle` is written, or to the end of the dict
/// when it is not written again.
fn up_to(needle: &[u8]) -> T {
    T::text(StrLen::Fixed(E::to_bytes(needle)), Encoding::Ascii)
}

/// The dict, as the three values numpy writes and the punctuation between
/// them. The keys are the format's own machinery; the values are the point.
fn header() -> T {
    T::structure(
        "Header",
        vec![
            ("open", up_to(DESCR)),
            ("descr_key", fixed(DESCR.len() as i128)),
            ("descr", up_to(ORDER)),
            ("order_key", fixed(ORDER.len() as i128)),
            ("fortran_order", up_to(SHAPE)),
            ("shape_key", fixed(SHAPE.len() as i128)),
            ("shape", up_to(b", }")),
            // The `, }` and the spaces that pad the header to 64 bytes, and
            // the newline that ends it.
            ("close", T::text(StrLen::Fixed(E::Remaining), Encoding::Ascii)),
        ],
    )
    .machinery(&["open", "descr_key", "order_key", "shape_key", "close"])
    .payload(&["descr", "shape"])
}

/// The numbers, as the type `descr` names. The data runs to the end of the
/// file, so how many there are is that room divided by how wide one is, and
/// the shape does not have to be parsed to say it.
fn data() -> T {
    let of = |ty: T, width: i128| T::array(ty, E::Remaining.div(E::lit(width)));
    let int = |bits: u32, e: Endian| T::Int { bits, endian: e };
    let uint = |bits: u32, e: Endian| T::UInt { bits, endian: e };
    // Both byte orders of everything numpy writes with one, and the
    // byte-order-free spelling of the types one byte wide.
    let mut cases: Vec<(String, T)> = vec![
        // A boolean is a byte holding 0 or 1.
        ("'|b1'".into(), of(T::UInt { bits: 8, endian: Little }, 1)),
        ("'|i1'".into(), of(int(8, Little), 1)),
        ("'|u1'".into(), of(uint(8, Little), 1)),
        ("'<i1'".into(), of(int(8, Little), 1)),
        ("'<u1'".into(), of(uint(8, Little), 1)),
    ];
    for (mark, e) in [('<', Endian::Little), ('>', Endian::Big)] {
        for (kind, width) in [("i", 2), ("i", 4), ("i", 8), ("u", 2), ("u", 4), ("u", 8), ("f", 2), ("f", 4), ("f", 8)] {
            let bits = width as u32 * 8;
            let ty = match kind {
                "i" => int(bits, e),
                "u" => uint(bits, e),
                _ => match width {
                    2 => T::F16(e),
                    4 => T::F32(e),
                    _ => T::F64(e),
                },
            };
            cases.push((format!("'{mark}{kind}{width}'"), of(ty, width)));
        }
    }
    T::Match {
        on: E::within(&["header", "descr"]),
        cases: cases.into(),
        // A dtype of several types, or one whose numbers belong together:
        // the room is right and nothing here can say what is in it.
        default: std::sync::Arc::new(T::bytes(E::Remaining)),
    }
}

pub fn npy() -> Template {
    let root = T::structure(
        "NPY",
        vec![
            ("magic", T::magic(MAGIC)),
            ("major", T::u8()),
            ("minor", T::u8()),
            // Version 1 wrote the header length in two bytes. Versions 2 and 3
            // write four, for a dtype too long to say in 65535.
            ("header_len", T::switch(E::field("major"), vec![(1, T::u16(Little))], T::u32(Little))),
            ("header", T::sized(E::field("header_len"), header())),
            ("data", data()),
        ],
    )
    .machinery(&["header_len"]);
    Template::new("npy", root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// A file of the shape numpy writes: the dict padded with spaces to a
    /// multiple of 64 bytes counting the magic and the length, and a newline
    /// where the padding ends.
    fn file(major: u8, descr: &str, fortran: bool, shape: &str, data: &[u8]) -> Vec<u8> {
        let dict = format!("{{'descr': {descr}, 'fortran_order': {}, 'shape': {shape}, }}", if fortran { "True" } else { "False" });
        let front = if major == 1 { 10 } else { 12 };
        let len = (front + dict.len() + 1).div_ceil(64) * 64 - front;
        let mut header = dict.into_bytes();
        header.resize(len - 1, b' ');
        header.push(b'\n');
        let mut b = MAGIC.to_vec();
        b.extend_from_slice(&[major, 0]);
        if major == 1 {
            b.extend_from_slice(&(header.len() as u16).to_le_bytes());
        } else {
            b.extend_from_slice(&(header.len() as u32).to_le_bytes());
        }
        b.extend_from_slice(&header);
        b.extend_from_slice(data);
        b
    }

    fn doubles() -> Vec<u8> {
        let mut data = Vec::new();
        for v in [1.0f64, 2.5, -3.0, 4.0, 5.0, 6.0] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        file(1, "'<f8'", false, "(2, 3)", &data)
    }

    fn eval(bytes: Vec<u8>) -> (Document<MemSource>, Evaluator) {
        (Document::new(MemSource(bytes)), Evaluator::new(npy()))
    }

    #[test]
    fn the_header_is_as_long_as_the_number_before_it_and_starts_the_data_on_64() {
        let (d, mut ev) = eval(doubles());
        let len = ev.node(&d, &[3]).unwrap().value.as_int().unwrap();
        let header = ev.node(&d, &[4]).unwrap();
        assert_eq!(header.size_bits, len as u64 * 8);
        assert_eq!((header.offset_bits + header.size_bits) % (64 * 8), 0);
    }

    #[test]
    fn the_dict_reads_as_its_three_values() {
        let (d, mut ev) = eval(doubles());
        assert_eq!(ev.node(&d, &[4, 2]).unwrap().value, Value::Str("'<f8'".into()));
        assert_eq!(ev.node(&d, &[4, 4]).unwrap().value, Value::Str("False".into()));
        assert_eq!(ev.node(&d, &[4, 6]).unwrap().value, Value::Str("(2, 3)".into()));
    }

    #[test]
    fn the_numbers_are_read_as_the_dtype_names_them() {
        let (d, mut ev) = eval(doubles());
        let data = ev.node(&d, &[5]).unwrap();
        assert_eq!((data.type_name.as_str(), data.child_count), ("f64 le[]", 6));
        assert_eq!(ev.node(&d, &[5, 1]).unwrap().value, Value::Float(2.5));
        assert_eq!(data.offset_bits + data.size_bits, d.len_bits());
    }

    #[test]
    fn the_other_byte_order_is_read_the_other_way_round() {
        let mut data = Vec::new();
        for v in [1i32, -2, 3] {
            data.extend_from_slice(&v.to_be_bytes());
        }
        let (d, mut ev) = eval(file(1, "'>i4'", false, "(3,)", &data));
        assert_eq!(ev.node(&d, &[5]).unwrap().type_name, "i32 be[]");
        assert_eq!(ev.node(&d, &[5, 1]).unwrap().value, Value::Int(-2));
    }

    #[test]
    fn a_version_2_file_counts_its_header_in_four_bytes() {
        let long: Vec<String> = (0..40).map(|i| format!("('field_number_{i:03}', '<f4')")).collect();
        let descr = format!("[{}]", long.join(", "));
        let b = file(2, &descr, false, "(2,)", &[0; 320]);
        let (d, mut ev) = eval(b);
        assert_eq!(ev.node(&d, &[3]).unwrap().type_name, "u32 le");
        assert!(ev.node(&d, &[3]).unwrap().value.as_int().unwrap() > 1000);
        // A dtype of named fields is not one type, so the numbers stay bytes.
        assert_eq!(ev.node(&d, &[5]).unwrap().type_name, "bytes[]");
    }

    #[test]
    fn a_fortran_ordered_file_says_so_and_is_still_read() {
        let (d, mut ev) = eval(file(1, "'<f8'", true, "(2, 3)", &[0; 48]));
        assert_eq!(ev.node(&d, &[4, 4]).unwrap().value, Value::Str("True".into()));
        assert_eq!(ev.node(&d, &[5]).unwrap().child_count, 6);
    }
}
