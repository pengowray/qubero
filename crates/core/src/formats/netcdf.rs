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
//! A variable's record says where in the file its numbers start, so the data
//! is placed by those offsets rather than read in order. How they are shaped
//! is not in that record: the shape is in the dimension list, reached through
//! the variable's `dimids`, and following that chain is what makes a 2-D
//! variable read as rows. The innermost dimension is the row, because the
//! classic format writes its arrays in the order C does. A variable of one
//! dimension is one row and reads as the run it is.
//!
//! Three versions, told apart by the byte after `CDF`. The classic file counts
//! everything in 32 bits; the 64-bit offset file (CDF-2) writes each variable's
//! start as 64 bits, which is what lifted the 2 GB limit; the 64-bit data file
//! (CDF-5) widens the sizes as well and adds the unsigned and 64-bit types.
//! Everything is big-endian, in all three.
//!
//! A variable whose first dimension is the unlimited one is stored one record
//! at a time, and all such variables are interleaved: every record variable's
//! slab for record 0, then every one for record 1, and so on. Its `begin`
//! points at its slab of the first record, and record `k` is `k * recsize`
//! past that, where `recsize` is the `vsize` of every record variable added
//! up. Nothing in the file writes `recsize` down; it is worked out here.
//!
//! Two things about records are not handled. A file whose record count is all
//! ones was left open by a writer that never came back to say, and only the
//! first hundred thousand records of one are placed. And the classic format
//! has a special case this ignores: when a file has exactly one record
//! variable, its records are written with no padding between them, so a record
//! of an odd width is placed a little further on each time than this says.
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
///
/// The last two fields are worked out rather than read, and take up no bytes.
/// They are the two things the shape of the data depends on and that no field
/// says outright: how long a row is, and whether this variable is written a
/// record at a time. Both are questions about the dimension list, reached
/// through `dimids`, and they are asked here because this is where `dimids`
/// is: the data is placed by a list declared after every variable, and from
/// there one variable's dimensions are two lists away.
fn var() -> T {
    // dims[dimids[i]].size, for the first and the last of this variable's
    // dimensions. Only reached when there is one to reach: a negative index
    // is an error, so the guards in front of these must not be evaluated
    // away.
    let dim_size = |which: E| E::elem_field("dims", E::elem("dimids", which), &["size"]);
    let scalar = || E::field("ndims").less_than(E::lit(1));
    // How many values are in a row: the size of the innermost dimension. A
    // variable of no dimensions holds one value, and so does one whose only
    // dimension is the record dimension, whose size is written as zero.
    let row = scalar().or(dim_size(E::field("ndims").sub(E::lit(1))).or(E::lit(1)));
    // A variable is a record variable when its *first* dimension is the
    // unlimited one. Written as "not (no dimensions, or a first dimension
    // with a size)", so that the dimension is only looked up when there is
    // one: `Or` does not evaluate its right side unless it has to.
    let is_record = E::lit(1).sub(scalar().or(E::lit(0).less_than(dim_size(E::lit(0)))));
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
            ("row_length", T::computed(row)),
            ("is_record", T::computed(is_record)),
        ],
    )
    .machinery(&["row_length", "is_record"])
}

/// How many bytes one value of each type takes. A type this reader does not
/// know reads as bytes, so its values are one byte each.
fn width() -> T {
    let w = |n: i128| T::computed(E::lit(n));
    T::switch(
        E::field("nc_type"),
        vec![
            (1, w(1)),
            (2, w(1)),
            (3, w(2)),
            (4, w(4)),
            (5, w(4)),
            (6, w(8)),
            (7, w(1)),
            (8, w(2)),
            (9, w(4)),
            (10, w(8)),
            (11, w(8)),
        ],
        w(1),
    )
}

/// One slab of a variable's numbers, shaped by its dimensions: `rows` rows of
/// `row_length` values each. A variable of one dimension, or of none, is one
/// row and reads as the run it is rather than as a run inside a run.
fn shaped(elem: T) -> T {
    let row = T::array(elem, E::field("row_length"));
    T::switch(E::field("rows"), vec![(1, row.clone())], T::array(row, E::field("rows")))
}

/// The same for text. A character variable's innermost dimension is how long
/// one string is, which is how the classic format holds a list of names.
fn shaped_text() -> T {
    let row = T::utf8(E::field("row_length"));
    T::switch(E::field("rows"), vec![(1, row.clone())], T::array(row, E::field("rows")))
}

/// One record's worth of a variable, read as the type its record declared and
/// shaped by its dimensions.
fn slab() -> T {
    T::switch(
        E::field("nc_type"),
        vec![
            (1, shaped(T::Int { bits: 8, endian: Big })),
            (2, shaped_text()),
            (3, shaped(T::Int { bits: 16, endian: Big })),
            (4, shaped(T::i32(Big))),
            (5, shaped(T::F32(Big))),
            (6, shaped(T::F64(Big))),
            (7, shaped(T::u8())),
            (8, shaped(T::u16(Big))),
            (9, shaped(T::u32(Big))),
            (10, shaped(T::Int { bits: 64, endian: Big })),
            (11, shaped(T::u64(Big))),
        ],
        shaped(T::u8()),
    )
}

/// The numbers one variable holds.
///
/// Everything about the shape is copied into fields of no bits first. They
/// come from the variable's own record, which is `vars[i]` for the `i`th child
/// of the pointer list, and they are copied here because inside the arrays
/// below the index has become the row's or the record's rather than the
/// variable's.
///
/// A plain variable is `vsize` bytes at `begin` and that is all. A record
/// variable is one slab per record, and the slabs are not together: every
/// record variable's slab for record 0 is written, then every one for record
/// 1, and so on, so this variable's record `k` is `k * recsize` past its
/// `begin`. `recsize` is what one round of that comes to, which is the
/// `vsize` of every record variable added up.
///
/// The last few bytes of a slab may belong to no value: `vsize` is rounded up
/// to a multiple of four, so a variable of three-byte rows has a byte of
/// padding at the end of each record. That reads as what it is.
fn var_data() -> T {
    let mine = |field: &str| T::computed(E::elem_field("vars", E::idx(), &[field]));
    let values = || E::field("vsize").div(E::field("width"));
    let rows = values().div(E::field("row_length").at_least(E::lit(1)));
    // One record for a plain variable, and however many were written for a
    // record one. Clamped: a file left open by a writer that never came back
    // says its record count is all ones, and that is not a number of records
    // to place.
    let records = E::lit(1).sub(E::field("is_record")).or(E::field("numrecs").at_most(E::lit(RECORD_LIMIT)));
    let used = E::field("rows").mul(E::field("row_length")).mul(E::field("width"));
    T::structure(
        "VarData",
        vec![
            ("nc_type", mine("nc_type")),
            ("vsize", mine("vsize")),
            ("begin", mine("begin")),
            ("row_length", mine("row_length")),
            ("is_record", mine("is_record")),
            ("width", width()),
            ("rows", T::computed(rows)),
            ("records", T::computed(records)),
            ("values", slab()),
            ("padding", T::bytes(E::field("vsize").sub(used).at_least(E::lit(0)))),
            (
                "later_records",
                T::array(
                    T::at(E::field("begin").add(E::idx().add(E::lit(1)).mul(E::field("recsize"))), slab()),
                    E::field("records").sub(E::lit(1)).at_least(E::lit(0)),
                ),
            ),
        ],
    )
    .machinery(&["nc_type", "vsize", "begin", "row_length", "is_record", "width", "rows", "records"])
    .payload(&["values"])
}

/// How many records this will place, however many a file claims. A record
/// count of all ones means the writer never said, and every record after this
/// many is a claim about bytes nothing has counted.
const RECORD_LIMIT: i128 = 100_000;

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
            // The dimension list is the root's own fields rather than a
            // structure of its own, because every variable's shape is a
            // question about `dims` and a name reaches only the direct fields
            // of the structures a field sits inside.
            ("dims_tag", T::enumeration("NcTag", u32be(), TAG)),
            ("num_dims", size_t()),
            ("dims", T::array(T::Named("Dim".into()), E::field("num_dims"))),
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
    // How far apart one record is from the next: every record variable's
    // `vsize` added up. Nothing in the file writes it down, and it cannot be
    // asked of the variable list directly, because adding up a field of a
    // list of records is not a question the IR has. So each variable's
    // contribution is copied into a number of its own first, and those are
    // what is added up: a variable that is not a record variable contributes
    // nothing.
    fields.push((
        "record_bytes",
        T::array(
            T::computed(
                E::elem_field("vars", E::idx(), &["vsize"]).mul(E::elem_field("vars", E::idx(), &["is_record"])),
            ),
            E::field("nelems"),
        ),
    ));
    fields.push(("recsize", T::computed(E::sum_of("record_bytes"))));
    fields.push((
        "data",
        T::pointer_list_sized("vars", &["begin"], Anchor::File, E::lit(0), T::Named("VarData".into())),
    ));
    T::structure("VariableList", fields).machinery(&["record_bytes", "recsize"])
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

    /// A CDF-1 file with one variable over both dimensions, where neither of
    /// them is unlimited: two rows of three floats, written as one block.
    fn file_2d() -> Vec<u8> {
        let build = |begin: u32| {
            let mut b = b"CDF\x01".to_vec();
            b.extend_from_slice(&be32(0)); // no records
            b.extend_from_slice(&be32(0x0A));
            b.extend_from_slice(&be32(2));
            b.extend_from_slice(&nm(1, "time"));
            b.extend_from_slice(&be32(2)); // two of them, and not unlimited
            b.extend_from_slice(&nm(1, "cell"));
            b.extend_from_slice(&be32(3));
            b.extend_from_slice(&be32(0)); // no global attributes
            b.extend_from_slice(&be32(0));
            b.extend_from_slice(&be32(0x0B));
            b.extend_from_slice(&be32(1));
            b.extend_from_slice(&nm(1, "sea_temp"));
            b.extend_from_slice(&be32(2)); // over both dimensions
            b.extend_from_slice(&be32(0));
            b.extend_from_slice(&be32(1));
            b.extend_from_slice(&be32(0)); // no attributes
            b.extend_from_slice(&be32(0));
            b.extend_from_slice(&be32(5)); // float
            b.extend_from_slice(&be32(24));
            b.extend_from_slice(&be32(begin));
            b
        };
        let header = build(0).len() as u32;
        let mut b = build(header);
        for v in [1.5f32, 2.5, 3.5, 4.5, 5.5, 6.5] {
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
        assert_eq!(ev.node(&d, &[5]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[5, 0]).unwrap().name, "[0] time");
        assert_eq!(ev.node(&d, &[5, 0, 1]).unwrap().value, Value::UInt(0));
        assert_eq!(ev.node(&d, &[5, 1, 1]).unwrap().value, Value::UInt(3));
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
        assert_eq!(ev.node(&d, &[3]).unwrap().size_bits, 32);
        assert_eq!(ev.node(&d, &[4]).unwrap().size_bits, 64);
        assert_eq!(ev.node(&d, &[5, 1, 0, 0]).unwrap().size_bits, 64);
        assert_eq!(ev.node(&d, &[5, 1, 1]).unwrap().size_bits, 64);
        // And a variable's dimension ids, which are counts as well.
        assert_eq!(ev.node(&d, &[7, 2, 0, 2, 0]).unwrap().size_bits, 64);
        assert_eq!(ev.node(&d, &[5, 1, 1]).unwrap().value, Value::UInt(3));
        assert_eq!(ev.node(&d, &[7, 2, 0, 0, 1]).unwrap().value, Value::Str("sea_temp".into()));
    }

    #[test]
    fn an_attribute_reads_as_the_type_it_names() {
        let d = Document::new(MemSource(file(1)));
        let mut ev = Evaluator::new(netcdf());
        assert_eq!(ev.node(&d, &[6, 2, 0]).unwrap().name, "[0] title");
        let ty = ev.node(&d, &[6, 2, 0, 1]).unwrap();
        assert_eq!(ty.value, Value::Enum { raw: 2, name: Some("char".into()), hex: false });
        assert_eq!(ev.node(&d, &[6, 2, 0, 3]).unwrap().value, Value::Str("depth".into()));
        // Five bytes of text, and three of padding to the next boundary.
        assert_eq!(ev.node(&d, &[6, 2, 0, 4]).unwrap().size_bits, 3 * 8);
    }

    #[test]
    fn a_variable_says_which_dimensions_it_is_over() {
        let d = Document::new(MemSource(file(1)));
        let mut ev = Evaluator::new(netcdf());
        assert_eq!(ev.node(&d, &[7, 2]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[7, 2, 0]).unwrap().name, "[0] sea_temp");
        assert_eq!(ev.node(&d, &[7, 2, 0, 2, 0]).unwrap().value, Value::UInt(1));
        // Its own attribute, inside the variable rather than beside it.
        assert_eq!(ev.node(&d, &[7, 2, 0, 3, 2, 0, 3]).unwrap().value, Value::Str("celsius".into()));
        assert_eq!(ev.node(&d, &[7, 2, 0, 5]).unwrap().value, Value::UInt(12));
        // What the dimensions come to, worked out and costing no bytes: a row
        // of three, and not a record variable.
        let row = ev.node(&d, &[7, 2, 0, 7]).unwrap();
        assert_eq!((row.value, row.size_bits), (Value::Int(3), 0));
        assert_eq!(ev.node(&d, &[7, 2, 0, 8]).unwrap().value, Value::Int(0));
        // And `depth`, whose first dimension is the unlimited one.
        assert_eq!(ev.node(&d, &[7, 2, 1, 8]).unwrap().value, Value::Int(1));
        // One record variable of eight bytes, so a record is eight bytes.
        assert_eq!(ev.node(&d, &[7, 4]).unwrap().value, Value::Int(8));
    }

    #[test]
    fn each_variables_numbers_are_placed_where_its_record_says() {
        let d = Document::new(MemSource(file(1)));
        let mut ev = Evaluator::new(netcdf());
        let data = ev.node(&d, &[7, 5]).unwrap();
        assert_eq!(data.child_count, 2);
        // Three floats: the type is float, vsize is twelve bytes, and its one
        // dimension makes them a single row.
        let first = ev.node(&d, &[7, 5, 0, 8]).unwrap();
        assert_eq!((first.type_name.as_str(), first.child_count), ("f32 be[]", 3));
        assert_eq!(ev.node(&d, &[7, 5, 0, 8, 2]).unwrap().value, Value::Float(3.5));
        assert_eq!(ev.node(&d, &[7, 5, 0]).unwrap().name, "[0] sea_temp");
        // And the record variable's first record, which is one double.
        let second = ev.node(&d, &[7, 5, 1, 8]).unwrap();
        assert_eq!((second.child_count, second.size_bits), (1, 64));
        assert_eq!(ev.node(&d, &[7, 5, 1, 8, 0]).unwrap().value, Value::Float(10.0));
    }

    #[test]
    fn a_record_variable_reads_as_one_slab_per_record() {
        // Two records were written, so `depth` has the record its `begin`
        // points at and one more, a whole record further on.
        let d = Document::new(MemSource(file(1)));
        let mut ev = Evaluator::new(netcdf());
        let later = ev.node(&d, &[7, 5, 1, 10]).unwrap();
        assert_eq!(later.child_count, 1);
        // The slab is the one child of a field that takes up no room where it
        // is declared, and stands where the offset put it.
        let second = ev.node(&d, &[7, 5, 1, 10, 0, 0]).unwrap();
        assert_eq!(ev.node(&d, &[7, 5, 1, 10, 0, 0, 0]).unwrap().value, Value::Float(20.0));
        // Placed a record on from the first, and not by reading in order.
        let first = ev.node(&d, &[7, 5, 1, 8]).unwrap();
        assert_eq!(second.offset_bits, first.offset_bits + 8 * 8);
        // A plain variable has no records after its first.
        assert_eq!(ev.node(&d, &[7, 5, 0, 10]).unwrap().child_count, 0);
    }

    #[test]
    fn a_two_dimensional_variable_reads_as_rows() {
        // `sea_temp` over both dimensions: two records of three floats each,
        // and each record is one row rather than a run of six.
        let d = Document::new(MemSource(file_2d()));
        let mut ev = Evaluator::new(netcdf());
        let values = ev.node(&d, &[7, 5, 0, 8]).unwrap();
        assert_eq!((values.child_count, values.size_bits), (2, 2 * 3 * 32));
        let row = ev.node(&d, &[7, 5, 0, 8, 1]).unwrap();
        assert_eq!((row.type_name.as_str(), row.child_count), ("f32 be[]", 3));
        assert_eq!(ev.node(&d, &[7, 5, 0, 8, 1, 2]).unwrap().value, Value::Float(6.5));
    }

    #[test]
    fn a_variables_data_says_which_record_placed_it() {
        use crate::eval::Role;
        let d = Document::new(MemSource(file(2)));
        let mut ev = Evaluator::new(netcdf());
        let o = ev.origins(&d, &[7, 5, 0]).unwrap();
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
        let tag = ev.node(&d, &[3]).unwrap();
        assert_eq!(tag.value, Value::Enum { raw: 0, name: Some("absent".into()), hex: false });
        assert_eq!(ev.node(&d, &[7, 2]).unwrap().child_count, 0);
        // And nothing is a record variable, so a record is nothing.
        assert_eq!(ev.node(&d, &[7, 4]).unwrap().value, Value::Int(0));
    }
}
