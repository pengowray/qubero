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
//! `descr` is what types the numbers, and `shape` is how they are grouped: a
//! 2 by 3 array reads as two rows of three, and a 3-D one as rows of rows.
//! Which dimension a row runs along is what `fortran_order` decides, since C
//! order runs along the last dimension and Fortran order along the first.
//!
//! A dtype of one kind reads as an array of that type, complex numbers as the
//! pairs of floats they are, a date as the count of the unit its dtype names,
//! and a fixed-width string as text. A structured dtype, which is written as a
//! list of tuples, reads as the list of names and formats it is, and the
//! numbers under it stay bytes.
//!
//! What is not read here:
//!
//! - What is in a record. A structured dtype, `[('a', '<i8'), ('b', '<f4')]`,
//!   says what its fields are called and what type each of them is, and both
//!   are in the file rather than in the template. A structure's field names
//!   are fixed when the template is built, and a type is picked by text in a
//!   field beside it rather than by text in one element of a list inside
//!   another field, so neither half of that can be followed. The list is read
//!   for the reader to see; the numbers stay bytes.
//! - Only the plain form of a structured dtype, a flat list of `('name',
//!   'format')` pairs. A field whose type is a dtype of its own,
//!   `('a', [('x', '<i4')])`, a field with a shape after its format,
//!   `('a', '<f4', (2, 2))`, a field whose name is a title and a name
//!   together, and the dict form with `offsets` and `itemsize`, are all
//!   written in the same brackets and are not taken apart here. A shape after
//!   the format leaves the tuple's own closing bracket where the next field
//!   would start, and the last of them reads as a field with nothing in it.
//! - Four dimensions, and then no more: each one is a field of a chain the
//!   template holds, so a shape with more of them reads the first four and
//!   leaves the rest of the tuple uncovered. The numbers are grouped for two
//!   dimensions and for three; a four-dimensional array says it has four and
//!   reads as one run all the same.
//! - A string dtype says its width in its own name, so widths up to 64 are
//!   read and a wider one stays bytes. A `U` is UTF-32, which is not among the
//!   encodings, and reads as the 32-bit code points it is written as.
//! - The unit of a date is in the name of the type its count is wrapped in,
//!   `datetime64[ns]`, because a field here carries no note of its own.
//! - A writer that puts the keys in another order, which the format allows and
//!   numpy has never done, gets the whole dict as one run of text and its
//!   numbers as bytes.
//! - An NPZ is a ZIP of these, and reads as the ZIP it is. Typing a member as
//!   an NPY would need a ZIP entry to take a template by the name it is stored
//!   under, and nothing in the IR says that.

use crate::template::{Encoding, Endian, Endian::Big, Endian::Little, Expr as E, StrLen, Template, Ty as T, Until};

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

/// The most dimensions a shape is read as. Each one is a field of its own in
/// a structure the template holds, so there has to be a last; a shape with
/// more than this reads the first few and leaves the rest of the tuple as
/// bytes nothing covered.
const DIMS: usize = 4;

/// The widest fixed-width string read as one. A string dtype says its width in
/// its own name, `'<U16'`, so every width is a keyword of its own here.
const WIDEST_STRING: i128 = 64;

/// The dict, as the three values numpy writes and the punctuation between
/// them. The keys are the format's own machinery; the values are the point.
fn header() -> T {
    // Where the dict's first value starts, counted from the front of the
    // header: what the record view of a structured dtype is placed at.
    let descr_at = E::size_of("open").add(E::size_of("descr_key"));
    let record = T::switch(E::peek(8, Big), vec![(b'[' as i128, T::at_in_window(descr_at, descr_list()))], T::bytes(E::lit(0)));
    T::structure(
        "Header",
        vec![
            ("open", up_to(DESCR)),
            ("descr_key", fixed(DESCR.len() as i128)),
            // A dtype of named fields, read as the fields it names. It covers
            // the same bytes `descr` does and takes none of its own, since
            // what types the numbers has to stay one run of text for the
            // switch below to read.
            ("record", record),
            ("descr", up_to(ORDER)),
            ("order_key", fixed(ORDER.len() as i128)),
            ("fortran_order", up_to(SHAPE)),
            ("shape_key", fixed(SHAPE.len() as i128)),
            ("shape", T::sized(E::to_bytes(b", }"), shape())),
            // The `, }` and the spaces that pad the header to 64 bytes, and
            // the newline that ends it.
            ("close", T::text(StrLen::Fixed(E::Remaining), Encoding::Ascii)),
        ],
    )
    .machinery(&["open", "descr_key", "order_key", "shape_key", "close"])
    .payload(&["descr", "shape"])
}

/// The shape, as the tuple of numbers it is written as: `(2, 3)` is two rows
/// of three rather than a run of text saying so.
///
/// The dimensions are a chain rather than a list, because how many there are
/// is not written anywhere: each one holds the rest of the tuple, and stops
/// when the next thing in it is the `)` that closes it. `ndim` counts back up
/// that chain, so the array below can ask how many dimensions there are
/// without anything having counted them into a field.
fn shape() -> T {
    T::structure(
        "Shape",
        vec![
            ("open", T::text(StrLen::Fixed(E::lit(1)), Encoding::Ascii)),
            // `()` is a zero-dimensional array, which is one number.
            ("dims", T::switch(next_in_tuple(), vec![(b')' as i128, no_dims())], dim(1))),
            ("close", T::text(StrLen::Fixed(E::Remaining), Encoding::Ascii)),
        ],
    )
    .machinery(&["open", "close"])
}

/// The next byte of the tuple, and the `)` that closes it when there is no
/// next byte. A shape written with nothing after the last dimension leaves the
/// chain at the end of what it may read, and looking there is an error rather
/// than an answer.
fn next_in_tuple() -> E {
    E::Remaining.less_than(E::lit(1)).mul(E::lit(b')' as i128)).or(E::peek(8, Big))
}

/// The end of the chain: no more dimensions, and nothing left to count.
fn no_dims() -> T {
    T::structure("Dims", vec![("ndim", T::computed(E::lit(0)))])
}

/// One dimension of the shape, and the rest of the tuple after it. A `)` where
/// the next number would be is the end, which is how `(3,)` says it has one
/// dimension rather than two.
fn dim(depth: usize) -> T {
    let rest = match depth == DIMS {
        true => no_dims(),
        false => T::switch(next_in_tuple(), vec![(b')' as i128, no_dims())], dim(depth + 1)),
    };
    T::inline_structure(
        "Dims",
        vec![
            ("dim", T::decimal(StrLen::token(&[b' '], &[b',', b')']))),
            ("rest", rest),
            ("ndim", T::computed(E::within(&["rest", "ndim"]).add(E::lit(1)))),
        ],
    )
    .machinery(&["rest", "ndim"])
    .payload(&["dim"])
}

/// A structured dtype, `[('a', '<i8'), ('b', '<f4')]`, as the pairs it is
/// written as. What each field is called and what type it is are both here;
/// what is not here is the record itself. See the module note.
fn descr_list() -> T {
    T::sized(E::to_bytes(b"]"), T::repeat(descr_field(), Until::End))
}

/// One `('name', 'format')` of a structured dtype. The quotes and the
/// punctuation between them are the format's own, and are stepped over.
fn descr_field() -> T {
    let quoted = |name: &'static str| (name, T::text(StrLen::Fixed(E::to_bytes(b"'").at_most(E::Remaining)), Encoding::Ascii));
    let step = |name: &'static str| (name, T::text(StrLen::Fixed(E::to_bytes(b"'").add(E::lit(1)).at_most(E::Remaining)), Encoding::Ascii));
    T::inline_structure(
        "Field",
        vec![
            // The `[(` of the first, or the `), (` of the ones after it, and
            // the quote that opens the name.
            step("before"),
            quoted("name"),
            // The quote that closes the name, and then the `, '` before the
            // format: two quotes along, since one of them is this.
            ("quote", T::text(StrLen::Fixed(E::lit(1).at_most(E::Remaining)), Encoding::Ascii)),
            step("between"),
            quoted("format"),
            // The closing quote, and the shape a field of several numbers
            // carries after it: `('a', '<f4', (2, 2))`.
            ("after", T::text(StrLen::Fixed(E::to_bytes(b")").add(E::lit(1)).at_most(E::Remaining)), Encoding::Ascii)),
        ],
    )
    .machinery(&["before", "quote", "between", "after"])
    .payload(&["name", "format"])
    .counted_as("field")
}

/// The numbers, in the shape the header gave them: a run for a one-dimensional
/// array, rows of the inner dimension for a two-dimensional one, and rows of
/// rows for a three. Which dimension is the inner one is what `fortran_order`
/// says: C order runs along the last, Fortran order along the first.
///
/// Every count is capped by the room there is, so a file cut short shows the
/// numbers it does have.
fn shaped(ty: T, width: i128) -> T {
    let flat = T::array(ty.clone(), E::Remaining.div(E::lit(width)));
    // The chain of dimensions, reached by name: the second is inside the
    // first, and the third inside that.
    let axis = |n: usize| {
        let mut path = vec!["header", "shape", "dims"];
        for _ in 0..n {
            path.push("rest");
        }
        path.push("dim");
        E::within(&path)
    };
    // Outermost first: how many of the next one down there are.
    let nest = |dims: Vec<E>| {
        let mut elem = ty.clone();
        let mut stride = E::lit(width);
        for d in dims.into_iter().rev() {
            elem = T::array(elem, d.clone().at_most(E::Remaining.div(stride.clone())));
            stride = stride.mul(d);
        }
        elem
    };
    let order = |c: Vec<E>, f: Vec<E>| T::matches(E::within(&["header", "fortran_order"]), vec![("True", nest(f))], nest(c));
    T::switch(
        E::within(&["header", "shape", "dims", "ndim"]),
        vec![
            (2, order(vec![axis(0), axis(1)], vec![axis(1), axis(0)])),
            (3, order(vec![axis(0), axis(1), axis(2)], vec![axis(2), axis(1), axis(0)])),
        ],
        // One dimension, none at all, or more than the shape is read to: one
        // run of numbers, which is how they are written whatever the shape.
        flat,
    )
}

/// The numbers, as the type `descr` names. The data runs to the end of the
/// file, so how many there are is that room divided by how wide one is, and
/// the shape says how they are grouped rather than how many there are.
fn data() -> T {
    let of = |ty: T, width: i128| shaped(ty, width);
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
        // A complex number is two floats that belong together, so it reads as
        // the pair it is rather than as twice as many numbers.
        for (width, part) in [(8, T::F32(e)), (16, T::F64(e))] {
            let pair = T::inline_structure("Complex", vec![("re", part.clone()), ("im", part)]);
            cases.push((format!("'{mark}c{width}'"), of(pair, width)));
        }
        // A date and a length of time are both a 64-bit count of a unit named
        // in the dtype. The unit is in the name of the type the count is
        // wrapped in, since a field here carries no note of its own.
        for (code, what) in [('M', "datetime64"), ('m', "timedelta64")] {
            for unit in ["Y", "M", "W", "D", "h", "m", "s", "ms", "us", "ns", "ps", "fs", "as"] {
                let count = T::inline_structure(&format!("{what}[{unit}]"), vec![("count", int(64, e))]);
                cases.push((format!("'{mark}{code}8[{unit}]'"), of(count, 8)));
            }
            // Written without a unit, which numpy reads as no unit at all.
            cases.push((format!("'{mark}{code}8'"), of(int(64, e), 8)));
        }
        // Text of a fixed width: bytes for `S`, and four bytes a character for
        // `U`, which is UTF-32 and reads here as the code points it is.
        for n in 1..=WIDEST_STRING {
            let word = T::UInt { bits: 32, endian: e };
            cases.push((format!("'{mark}U{n}'"), of(T::array(word, E::lit(n)), n * 4)));
            cases.push((format!("'{mark}S{n}'"), of(T::text(StrLen::Padded { size: E::lit(n), pad: 0 }, Encoding::Ascii), n)));
        }
    }
    for n in 1..=WIDEST_STRING {
        cases.push((format!("'|S{n}'"), of(T::text(StrLen::Padded { size: E::lit(n), pad: 0 }, Encoding::Ascii), n)));
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
        assert_eq!(ev.node(&d, &[4, 3]).unwrap().value, Value::Str("'<f8'".into()));
        assert_eq!(ev.node(&d, &[4, 5]).unwrap().value, Value::Str("False".into()));
        // The shape is the numbers it is written as: two of three.
        assert_eq!(ev.node(&d, &[4, 7, 1, 0]).unwrap().value, Value::Int(2));
        assert_eq!(ev.node(&d, &[4, 7, 1, 1, 0]).unwrap().value, Value::Int(3));
        assert_eq!(ev.node(&d, &[4, 7, 1, 2]).unwrap().value, Value::Int(2));
    }

    #[test]
    fn the_numbers_are_read_as_the_dtype_names_them() {
        let (d, mut ev) = eval(doubles());
        // Two rows of three, since that is the shape: C order runs along the
        // last dimension, so a row is three numbers.
        let data = ev.node(&d, &[5]).unwrap();
        assert_eq!(data.child_count, 2);
        let row = ev.node(&d, &[5, 0]).unwrap();
        assert_eq!((row.type_name.as_str(), row.child_count), ("f64 le[]", 3));
        assert_eq!(ev.node(&d, &[5, 0, 1]).unwrap().value, Value::Float(2.5));
        assert_eq!(ev.node(&d, &[5, 1, 0]).unwrap().value, Value::Float(4.0));
        assert_eq!(data.offset_bits + data.size_bits, d.len_bits());
    }

    #[test]
    fn a_fortran_ordered_array_runs_along_its_first_dimension() {
        let mut data = Vec::new();
        for v in [1.0f64, 2.0, 3.0, 4.0, 5.0, 6.0] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        // Two by three written column by column: a run is a column of two.
        let (d, mut ev) = eval(file(1, "'<f8'", true, "(2, 3)", &data));
        let data = ev.node(&d, &[5]).unwrap();
        assert_eq!(data.child_count, 3);
        assert_eq!(ev.node(&d, &[5, 0]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[5, 1, 0]).unwrap().value, Value::Float(3.0));
    }

    #[test]
    fn a_three_dimensional_array_reads_as_rows_of_rows() {
        let data: Vec<u8> = (0..24u8).collect();
        let (d, mut ev) = eval(file(1, "'|u1'", false, "(2, 3, 4)", &data));
        assert_eq!(ev.node(&d, &[5]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[5, 0]).unwrap().child_count, 3);
        assert_eq!(ev.node(&d, &[5, 0, 0]).unwrap().child_count, 4);
        assert_eq!(ev.node(&d, &[5, 1, 2, 3]).unwrap().value, Value::UInt(23));
    }

    #[test]
    fn a_zero_dimensional_array_is_one_number() {
        let (d, mut ev) = eval(file(1, "'<i4'", false, "()", &7i32.to_le_bytes()));
        assert_eq!(ev.node(&d, &[4, 7, 1, 0]).unwrap().value, Value::Int(0));
        let data = ev.node(&d, &[5]).unwrap();
        assert_eq!((data.type_name.as_str(), data.child_count), ("i32 le[]", 1));
    }

    #[test]
    fn a_date_is_a_count_of_the_unit_its_dtype_names() {
        let (d, mut ev) = eval(file(1, "'<M8[ns]'", false, "(2,)", &[0; 16]));
        assert_eq!(ev.node(&d, &[5, 0]).unwrap().type_name, "datetime64[ns]");
    }

    #[test]
    fn a_complex_number_is_the_pair_of_floats_it_is_written_as() {
        let mut data = Vec::new();
        for v in [1.5f32, -2.5, 0.0, 0.5] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        let (d, mut ev) = eval(file(1, "'<c8'", false, "(2,)", &data));
        assert_eq!(ev.node(&d, &[5]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[5, 0, 1]).unwrap().value, Value::Float(-2.5));
    }

    #[test]
    fn a_string_dtype_is_text_of_the_width_it_names() {
        let (d, mut ev) = eval(file(1, "'|S4'", false, "(2,)", b"abc\0defg"));
        let first = ev.node(&d, &[5, 0]).unwrap();
        assert_eq!(first.size_bits, 4 * 8);
        assert_eq!(first.value, Value::Str("abc".into()));
        assert_eq!(ev.node(&d, &[5, 1]).unwrap().value, Value::Str("defg".into()));
    }

    #[test]
    fn a_structured_dtype_reads_as_the_fields_it_names_and_no_further() {
        let b = file(1, "[('a', '<i8'), ('b', '<f4')]", false, "(2,)", &[0; 24]);
        let (d, mut ev) = eval(b);
        let record = ev.node(&d, &[4, 2]).unwrap();
        // The record view covers the same bytes the text does and takes none
        // of its own.
        assert_eq!(record.size_bits, 0);
        assert_eq!(ev.node(&d, &[4, 2, 0]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[4, 2, 0, 0, 1]).unwrap().value, Value::Str("a".into()));
        assert_eq!(ev.node(&d, &[4, 2, 0, 0, 4]).unwrap().value, Value::Str("<i8".into()));
        assert_eq!(ev.node(&d, &[4, 2, 0, 1, 1]).unwrap().value, Value::Str("b".into()));
        // What is in the record is still bytes: see the module note.
        assert_eq!(ev.node(&d, &[5]).unwrap().type_name, "bytes[]");
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
        assert_eq!(ev.node(&d, &[4, 5]).unwrap().value, Value::Str("True".into()));
        // Three columns of two, which is the same six numbers.
        assert_eq!(ev.node(&d, &[5]).unwrap().child_count, 3);
    }
}
