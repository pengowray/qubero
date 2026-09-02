//! NetCDF classic: the file an ocean float, a satellite pass or a climate run
//! is published as.
//!
//! A header that describes the whole file, and then the numbers. The header is
//! three lists: the dimensions, the attributes that apply to the file as a
//! whole, and the variables. Every list is written the same way, a tag saying
//! which list it is and a count, so an empty one is two zero words and needs no
//! case of its own. Every name is a count and that many bytes, padded out to a
//! multiple of four.
//!
//! A variable's record says where in the file its numbers start, so the data is
//! placed by those offsets rather than read in order, and each one reads as an
//! array of the type the variable declared. What it does not say is how the
//! numbers are shaped: the shape is in the dimension list, reached through the
//! variable's `dimids`, and nothing here follows that chain, so a 2-D variable
//! reads as one long run rather than as rows.
//!
//! Three versions, told apart by the byte after `CDF`. The classic file counts
//! everything in 32 bits; the 64-bit offset file (CDF-2) writes each variable's
//! start as 64 bits, which is what lifted the 2 GB limit; the 64-bit data file
//! (CDF-5) widens the sizes as well and adds the unsigned and 64-bit types.
//! Everything is big-endian, in all three.
//!
//! The record variables are the gap. A variable whose first dimension is the
//! unlimited one is stored one record at a time, all such variables
//! interleaved, and its `begin` points at its slab of the first record only.
//! That slab is what reads here; the records after it belong to no field and
//! show as a gap, because saying otherwise would mean claiming a run of bytes
//! that holds every other record variable too.
//!
//! A `.nc` written by a modern library is often NetCDF-4, which is an HDF5 file
//! and starts with the HDF5 signature. That one is `hdf5`, not this.

use crate::template::{Anchor, Endian::*, Expr as E, Template, Ty as T};

/// The external types a value in the header, or a variable's data, can have.
/// The last five exist only in a CDF-5 file.
const NC_TYPE: &[(i128, &str)] = &[
    (1, "byte"),
    (2, "char"),
    (3, "short"),
    (4, "int"),
    (5, "float"),
    (6, "double"),
    (7, "ubyte"),
    (8, "ushort"),
    (9, "uint"),
    (10, "int64"),
    (11, "uint64"),
];

/// The tag that opens each of the three lists, and the one that says there is
/// no list at all. `ABSENT` is written as two zero words, so a list that is not
/// there reads as this tag and a count of nothing.
const TAG: &[(i128, &str)] = &[(0, "absent"), (0x0A, "dimensions"), (0x0B, "variables"), (0x0C, "attributes")];

const VERSION: &[(i128, &str)] = &[(1, "classic"), (2, "64-bit offset"), (5, "64-bit data")];

fn u32be() -> T {
    T::u32(Big)
}

/// Every count in the header. CDF-5 widened all of them at once: how many
/// dimensions there are, how long a name is, how many values an attribute has,
/// how many bytes a variable takes. Four bytes in a classic or 64-bit-offset
/// file, eight in a 64-bit-data one. The three list tags and `nc_type` did not
/// widen, because they are fixed 32-bit words rather than counts.
fn size_t() -> T {
    T::switch(E::field("version"), vec![(5, T::u64(Big))], u32be())
}

/// Where a variable's data starts. Four bytes in a classic file, which is the
/// limit CDF-2 was made to lift, and eight in both later versions.
fn begin() -> T {
    T::switch(E::field("version"), vec![(1, u32be())], T::u64(Big))
}

fn nc_type() -> T {
    T::enumeration("NcType", u32be(), NC_TYPE)
}

/// A count and that many bytes of name, padded out so that what follows starts
/// on a four-byte boundary. Every name in the file is written this way.
fn name() -> T {
    T::structure_named(
        "Name",
        "",
        "text",
        vec![
            ("len", size_t()),
            ("text", T::utf8(E::field("len"))),
            ("padding", T::bytes(E::size_of("text").pad_to(4))),
        ],
    )
}

/// One of the three lists: the tag that says which it is, a count, and that
/// many entries. `items` is named by the caller, because the pointer list that
/// places the data has to name the variable list's entries by that name.
fn list(items: &str, elem: T) -> Vec<(&str, T)> {
    vec![
        ("tag", T::enumeration("NcTag", u32be(), TAG)),
        ("nelems", size_t()),
        (items, T::array(elem, E::field("nelems"))),
    ]
}

/// A name and a length. A length of zero is the unlimited dimension, the one
/// the records are counted along.
fn dim() -> T {
    T::structure_named("Dim", "name", "", vec![("name", name()), ("size", size_t())])
}

/// One attribute: a name, a type, that many values, and padding to the next
/// four-byte boundary. The values read as whatever the type says, so a
/// `units` attribute is text and a `_FillValue` is one number of the
/// variable's own type.
fn attribute() -> T {
    T::structure_named(
        "Attribute",
        "name",
        "values",
        vec![
            ("name", name()),
            ("nc_type", nc_type()),
            ("nelems", size_t()),
            ("values", values(E::field("nelems"))),
            ("padding", T::bytes(E::size_of("values").pad_to(4))),
        ],
    )
}

/// `count` values of whatever `nc_type` says, for an attribute. A type this
/// reader does not know says nothing about how long the value is, so there is
/// nothing to read and nothing to guess at.
fn values(count: E) -> T {
    let n = || count.clone();
    T::switch(
        E::field("nc_type"),
        vec![
            (1, T::array(T::Int { bits: 8, endian: Big }, n())),
            (2, T::utf8(n())),
            (3, T::array(T::Int { bits: 16, endian: Big }, n())),
            (4, T::array(T::i32(Big), n())),
            (5, T::array(T::F32(Big), n())),
            (6, T::array(T::F64(Big), n())),
            (7, T::array(T::u8(), n())),
            (8, T::array(T::u16(Big), n())),
            (9, T::array(T::u32(Big), n())),
            (10, T::array(T::Int { bits: 64, endian: Big }, n())),
            (11, T::array(T::u64(Big), n())),
        ],
        T::bytes(E::lit(0)),
    )
}

fn attribute_list() -> T {
    T::structure("AttributeList", list("attrs", T::Named("Attribute".into())))
}

/// One variable: its name, which dimensions it is over, its own attributes,
/// its type, how many bytes it takes and where those bytes start. `vsize` is
/// the whole variable for a plain one and one record's worth for a record
/// variable, padded to four bytes either way.
fn var() -> T {
    T::structure_named(
        "Variable",
        "name",
        "",
        vec![
            ("name", name()),
            ("ndims", size_t()),
            ("dimids", T::array(size_t(), E::field("ndims"))),
            ("attributes", attribute_list()),
            ("nc_type", nc_type()),
            ("vsize", size_t()),
            ("begin", begin()),
        ],
    )
}

/// The numbers one variable holds, read as the type its record declared and as
/// many of them as its `vsize` divides into. `vsize` is rounded up to four
/// bytes, so up to three bytes at the end of a variable of a narrow type belong
/// to no value and read as a gap, which is what they are.
fn var_data() -> T {
    let ty = || E::elem_field("vars", E::idx(), &["nc_type"]);
    let size = || E::elem_field("vars", E::idx(), &["vsize"]);
    let count = |width: i128| size().div(E::lit(width));
    let of = |width: i128, elem: T| T::array(elem, count(width));
    T::switch(
        ty(),
        vec![
            (1, of(1, T::Int { bits: 8, endian: Big })),
            (2, T::utf8(count(1))),
            (3, of(2, T::Int { bits: 16, endian: Big })),
            (4, of(4, T::i32(Big))),
            (5, of(4, T::F32(Big))),
            (6, of(8, T::F64(Big))),
            (7, of(1, T::u8())),
            (8, of(2, T::u16(Big))),
            (9, of(4, T::u32(Big))),
            (10, of(8, T::Int { bits: 64, endian: Big })),
            (11, of(8, T::u64(Big))),
        ],
        T::bytes(size()),
    )
}

pub fn netcdf() -> Template {
    let root = T::structure(
        "NetCDF",
        vec![
            ("magic", T::magic(b"CDF")),
            ("version", T::enumeration("CdfVersion", T::u8(), VERSION)),
            // How many records have been written along the unlimited
            // dimension. A writer that did not know leaves it at all ones,
            // which is eight bytes of them in a 64-bit-data file.
            (
                "numrecs",
                T::switch(
                    E::field("version"),
                    vec![(5, T::enumeration("NumRecs", T::u64(Big), &[(0xFFFF_FFFF_FFFF_FFFF, "streaming")]))],
                    T::enumeration("NumRecs", u32be(), &[(0xFFFF_FFFF, "streaming")]),
                ),
            ),
            ("dimensions", T::structure("DimensionList", list("dims", T::Named("Dim".into())))),
            ("attributes", attribute_list()),
            ("variables", var_list()),
        ],
    );
    Template::new("netcdf", root)
        .with_type("Dim", dim())
        .with_type("Attribute", attribute())
        .with_type("Variable", var())
        .with_type("VarData", var_data())
}

/// The variable list, and the data it places. The pointer list is declared
/// last and inside this structure, because the offsets it reads are a field of
/// this structure: it names `vars`, which is its own sibling.
fn var_list() -> T {
    let mut fields = list("vars", T::Named("Variable".into()));
    fields.push((
        "data",
        T::pointer_list_sized("vars", &["begin"], Anchor::File, E::lit(0), T::Named("VarData".into())),
    ));
    T::structure("VariableList", fields)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn be32(v: u32) -> Vec<u8> {
        v.to_be_bytes().to_vec()
    }

    /// A count, as wide as the version writes one.
    fn count(version: u8, v: u64) -> Vec<u8> {
        match version == 5 {
            true => v.to_be_bytes().to_vec(),
            false => (v as u32).to_be_bytes().to_vec(),
        }
    }

    /// A name: its length, its bytes, and padding to four.
    fn nm(version: u8, s: &str) -> Vec<u8> {
        let mut v = count(version, s.len() as u64);
        let head = v.len();
        v.extend_from_slice(s.as_bytes());
        v.resize(head + s.len().div_ceil(4) * 4, 0);
        v
    }

    /// A whole small CDF-1 file: two dimensions, one of them unlimited, a
    /// global attribute, a 2-D float variable and a record variable.
    ///
    /// Written twice over: once to find out how long the header is, and once
    /// with the data offsets that length settles. That is what every writer of
    /// one of these does.
    fn file(version: u8) -> Vec<u8> {
        let size = |v: u64| count(version, v);
        let offset = |v: u64| if version == 1 { (v as u32).to_be_bytes().to_vec() } else { v.to_be_bytes().to_vec() };
        let build = |begins: [u64; 2]| {
            let mut b = b"CDF".to_vec();
            b.push(version);
            b.extend_from_slice(&size(2)); // two records written
            // Dimensions: time is unlimited, so its length is zero.
            b.extend_from_slice(&be32(0x0A));
            b.extend_from_slice(&size(2));
            b.extend_from_slice(&nm(version, "time"));
            b.extend_from_slice(&size(0));
            b.extend_from_slice(&nm(version, "cell"));
            b.extend_from_slice(&size(3));
            // One global attribute, which is text.
            b.extend_from_slice(&be32(0x0C));
            b.extend_from_slice(&size(1));
            b.extend_from_slice(&nm(version, "title"));
            b.extend_from_slice(&be32(2)); // char
            b.extend_from_slice(&size(5));
            b.extend_from_slice(b"depth\0\0\0"); // padded to four
            // Two variables.
            b.extend_from_slice(&be32(0x0B));
            b.extend_from_slice(&size(2));
            // sea_temp(cell): three floats, and one attribute of its own.
            b.extend_from_slice(&nm(version, "sea_temp"));
            b.extend_from_slice(&size(1));
            b.extend_from_slice(&size(1)); // dimid 1, cell
            b.extend_from_slice(&be32(0x0C));
            b.extend_from_slice(&size(1));
            b.extend_from_slice(&nm(version, "units"));
            b.extend_from_slice(&be32(2));
            b.extend_from_slice(&size(7));
            b.extend_from_slice(b"celsius\0");
            b.extend_from_slice(&be32(5)); // float
            b.extend_from_slice(&size(12));
            b.extend_from_slice(&offset(begins[0]));
            // depth(time): a record variable, one double per record.
            b.extend_from_slice(&nm(version, "depth"));
            b.extend_from_slice(&size(1));
            b.extend_from_slice(&size(0)); // dimid 0, the unlimited one
            b.extend_from_slice(&be32(0)); // no attributes
            b.extend_from_slice(&size(0));
            b.extend_from_slice(&be32(6)); // double
            b.extend_from_slice(&size(8));
            b.extend_from_slice(&offset(begins[1]));
            b
        };
        let header = build([0, 0]).len() as u64;
        let mut b = build([header, header + 12]);
        // Three floats, then two records of one double each.
        for v in [1.5f32, 2.5, 3.5] {
            b.extend_from_slice(&v.to_be_bytes());
        }
        for v in [10.0f64, 20.0] {
            b.extend_from_slice(&v.to_be_bytes());
        }
        b
    }

    #[test]
    fn the_version_byte_says_which_of_the_three_it_is() {
        for (version, name) in [(1u8, "classic"), (2, "64-bit offset"), (5, "64-bit data")] {
            let d = Document::new(MemSource(file(version)));
            let mut ev = Evaluator::new(netcdf());
            let v = ev.node(&d, &[1]).unwrap();
            assert_eq!(v.value, Value::Enum { raw: version as i128, name: Some(name.into()), hex: false });
            // Two records written, which is a number nothing names.
            assert_eq!(ev.node(&d, &[2]).unwrap().value, Value::Enum { raw: 2, name: None, hex: false });
        }
    }

    #[test]
    fn a_dimension_of_length_zero_is_the_unlimited_one() {
        let d = Document::new(MemSource(file(2)));
        let mut ev = Evaluator::new(netcdf());
        assert_eq!(ev.node(&d, &[3, 2]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[3, 2, 0]).unwrap().name, "[0] time");
        assert_eq!(ev.node(&d, &[3, 2, 0, 1]).unwrap().value, Value::UInt(0));
        assert_eq!(ev.node(&d, &[3, 2, 1, 1]).unwrap().value, Value::UInt(3));
    }

    #[test]
    fn a_wide_file_writes_its_counts_in_eight_bytes() {
        // CDF-5 widened every count in the header at once, not only the ones
        // that hold a size: the record count, how many dimensions there are,
        // how long a name is, and a dimension's length are all eight bytes.
        // The tags did not widen, because they are fixed words.
        let d = Document::new(MemSource(file(5)));
        let mut ev = Evaluator::new(netcdf());
        assert_eq!(ev.node(&d, &[2]).unwrap().size_bits, 64);
        assert_eq!(ev.node(&d, &[3, 0]).unwrap().size_bits, 32);
        assert_eq!(ev.node(&d, &[3, 1]).unwrap().size_bits, 64);
        assert_eq!(ev.node(&d, &[3, 2, 1, 0, 0]).unwrap().size_bits, 64);
        assert_eq!(ev.node(&d, &[3, 2, 1, 1]).unwrap().size_bits, 64);
        // And a variable's dimension ids, which are counts as well.
        assert_eq!(ev.node(&d, &[5, 2, 0, 2, 0]).unwrap().size_bits, 64);
        assert_eq!(ev.node(&d, &[3, 2, 1, 1]).unwrap().value, Value::UInt(3));
        assert_eq!(ev.node(&d, &[5, 2, 0, 0, 1]).unwrap().value, Value::Str("sea_temp".into()));
    }

    #[test]
    fn an_attribute_reads_as_the_type_it_names() {
        let d = Document::new(MemSource(file(1)));
        let mut ev = Evaluator::new(netcdf());
        assert_eq!(ev.node(&d, &[4, 2, 0]).unwrap().name, "[0] title");
        let ty = ev.node(&d, &[4, 2, 0, 1]).unwrap();
        assert_eq!(ty.value, Value::Enum { raw: 2, name: Some("char".into()), hex: false });
        assert_eq!(ev.node(&d, &[4, 2, 0, 3]).unwrap().value, Value::Str("depth".into()));
        // Five bytes of text, and three of padding to the next boundary.
        assert_eq!(ev.node(&d, &[4, 2, 0, 4]).unwrap().size_bits, 3 * 8);
    }

    #[test]
    fn a_variable_says_which_dimensions_it_is_over() {
        let d = Document::new(MemSource(file(1)));
        let mut ev = Evaluator::new(netcdf());
        assert_eq!(ev.node(&d, &[5, 2]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[5, 2, 0]).unwrap().name, "[0] sea_temp");
        assert_eq!(ev.node(&d, &[5, 2, 0, 2, 0]).unwrap().value, Value::UInt(1));
        // Its own attribute, inside the variable rather than beside it.
        assert_eq!(ev.node(&d, &[5, 2, 0, 3, 2, 0, 3]).unwrap().value, Value::Str("celsius".into()));
        assert_eq!(ev.node(&d, &[5, 2, 0, 5]).unwrap().value, Value::UInt(12));
    }

    #[test]
    fn each_variables_numbers_are_placed_where_its_record_says() {
        let d = Document::new(MemSource(file(1)));
        let mut ev = Evaluator::new(netcdf());
        let data = ev.node(&d, &[5, 3]).unwrap();
        assert_eq!(data.child_count, 2);
        // Three floats, because the type is float and vsize is twelve bytes.
        let first = ev.node(&d, &[5, 3, 0]).unwrap();
        assert_eq!((first.type_name.as_str(), first.child_count), ("f32 be[]", 3));
        assert_eq!(ev.node(&d, &[5, 3, 0, 2]).unwrap().value, Value::Float(3.5));
        assert_eq!(first.name, "[0] sea_temp");
        // And the record variable's first record, which is one double.
        let second = ev.node(&d, &[5, 3, 1]).unwrap();
        assert_eq!((second.child_count, second.size_bits), (1, 64));
        assert_eq!(ev.node(&d, &[5, 3, 1, 0]).unwrap().value, Value::Float(10.0));
    }

    #[test]
    fn the_records_after_the_first_belong_to_no_field() {
        // The second record of `depth` is eight bytes nothing covers: the
        // header says where the first record's slab is and no more.
        let d = Document::new(MemSource(file(1)));
        let mut ev = Evaluator::new(netcdf());
        let first = ev.node(&d, &[5, 3, 1]).unwrap();
        let after = first.offset_bits + first.size_bits;
        let gap = ev.spans(&d, after, after + 8 * 8, 2).unwrap();
        assert!(gap[0].gap);
        assert_eq!(gap[0].size_bits, 8 * 8);
    }

    #[test]
    fn a_variables_data_says_which_record_placed_it() {
        use crate::eval::Role;
        let d = Document::new(MemSource(file(2)));
        let mut ev = Evaluator::new(netcdf());
        let o = ev.origins(&d, &[5, 3, 0]).unwrap();
        let roles: Vec<_> = o.iter().map(|x| (x.role, x.label.as_str())).collect();
        assert!(roles.iter().any(|(r, l)| *r == Role::Position && *l == "vars[0].begin"), "{roles:?}");
    }

    #[test]
    fn a_file_with_no_variables_at_all_still_reads() {
        // Three absent lists, which is six zero words, and nothing else.
        let mut b = b"CDF\x01".to_vec();
        b.extend_from_slice(&be32(0));
        for _ in 0..6 {
            b.extend_from_slice(&be32(0));
        }
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(netcdf());
        let tag = ev.node(&d, &[3, 0]).unwrap();
        assert_eq!(tag.value, Value::Enum { raw: 0, name: Some("absent".into()), hex: false });
        assert_eq!(ev.node(&d, &[5, 2]).unwrap().child_count, 0);
    }
}
