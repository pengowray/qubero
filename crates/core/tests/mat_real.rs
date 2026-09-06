//! Real MATLAB files, one per shape the format has taken.
//!
//! What it checks is what the header and the element tags decide rather than
//! the numbers: which level a file is, which way round it is written, that a
//! compressed element inflates into another element, and that an array's class
//! picks the right thing to read under it.
//!
//! The files come from scipy's own test corpus and live in the sample
//! collection rather than here. Point `QUBERO_SAMPLES` at it, or keep it
//! beside the repository as `qubero-samples`. With neither, each test says so
//! and passes.

use std::path::PathBuf;

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

fn sample(name: &str) -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(set) = std::env::var("QUBERO_SAMPLES") {
        roots.extend(set.split(';').filter(|s| !s.is_empty()).map(PathBuf::from));
    }
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../qubero-samples"));
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../qubero-samples"));
    roots.into_iter().map(|r| r.join("mat").join(name)).find(|p| p.exists())
}

fn read(name: &str) -> Option<(Document<MemSource>, Evaluator)> {
    let path = sample(name)?;
    let bytes = std::fs::read(&path).unwrap();
    // What the file is called plays no part: every one of these is recognised
    // by what is in it.
    let sniffed = formats::sniff(&bytes[..bytes.len().min(formats::SNIFF_WINDOW)], bytes.len() as u64);
    assert_eq!(sniffed, Some("mat"), "{name} is not recognised as a MATLAB file");
    let doc = Document::new(MemSource(bytes));
    Some((doc, Evaluator::new(formats::builtin("mat").unwrap())))
}

macro_rules! open {
    ($name:expr) => {
        match read($name) {
            Some(pair) => pair,
            None => {
                eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
                return;
            }
        }
    };
}

/// The name of the field at `path`, and what it reads as.
fn at(d: &Document<MemSource>, ev: &mut Evaluator, path: &[usize]) -> (String, Value) {
    let n = ev.node(d, path).unwrap_or_else(|e| panic!("{path:?}: {e:?}"));
    (n.name, n.value)
}

fn text(d: &Document<MemSource>, ev: &mut Evaluator, path: &[usize]) -> String {
    match at(d, ev, path).1 {
        Value::Str(s) => s,
        other => panic!("{path:?} is {other:?}, not text"),
    }
}

fn number(d: &Document<MemSource>, ev: &mut Evaluator, path: &[usize]) -> i128 {
    match at(d, ev, path).1 {
        Value::UInt(v) => v as i128,
        Value::Int(v) => v,
        Value::Enum { raw, .. } => raw,
        other => panic!("{path:?} is {other:?}, not a number"),
    }
}

/// Where an array's own fields sit inside a file whose one variable is written
/// as a single compressed element: the file's body, its first element, the
/// inflated stream, the array element in it, and the array.
const IN_ZLIB: &[usize] = &[1, 0, 2, 0, 2];

#[test]
fn a_level_4_file_has_no_header_at_all() {
    let (d, mut ev) = open!("testmatrix_4.2c_SOL2.mat");
    // Straight into the matrices: five integers, a name, and the numbers.
    assert_eq!(text(&d, &mut ev, &[0, 8]), "testmatrix");
    assert_eq!(number(&d, &mut ev, &[0, 1]), 1, "written on a big-endian machine");
    assert_eq!(number(&d, &mut ev, &[0, 2]), 0, "eight bytes to a number");
    assert_eq!((number(&d, &mut ev, &[0, 4]), number(&d, &mut ev, &[0, 5])), (3, 5));
    let values = ev.node(&d, &[0, 9]).unwrap();
    assert_eq!((values.type_name.as_str(), values.child_count), ("f64 be[]", 15));
}

/// A description of 0 reads as 0 whichever way round it is taken, so the
/// machine digit is the only thing that can settle the endianness.
#[test]
fn a_level_4_file_written_little_endian_is_read_that_way() {
    let (d, mut ev) = open!("test_mat4_le_floats.mat");
    assert_eq!(text(&d, &mut ev, &[0, 8]), "a");
    assert_eq!(number(&d, &mut ev, &[0, 1]), 0, "written on a little-endian machine");
    let values = ev.node(&d, &[0, 9]).unwrap();
    assert_eq!(values.type_name, "f64 le[]");
}

/// More than one variable in a level 4 file is more than one matrix, one after
/// the other, with nothing between them.
#[test]
fn a_level_4_file_holds_its_variables_one_after_another() {
    let (d, mut ev) = open!("testvec_4_GLNX86.mat");
    let file = ev.node(&d, &[]).unwrap();
    assert_eq!(file.child_count, 2);
    assert_eq!(text(&d, &mut ev, &[0, 8]), "fit_params");
    assert_eq!(text(&d, &mut ev, &[1, 8]), "xdot_filt");
}

/// A level 5 file written on a big-endian machine says `MI`, and everything
/// after the header is read that way round.
#[test]
fn a_big_endian_level_5_file_reads_the_other_way_round() {
    let (d, mut ev) = open!("testdouble_6.1_SOL2.mat");
    assert_eq!(text(&d, &mut ev, &[0, 3]), "MI");
    assert_eq!(number(&d, &mut ev, &[0, 2]), 0x0100, "level 5");
    // Uncompressed, so the array element sits in the file rather than in a
    // stream: the class is read from the far byte of the flags word.
    assert_eq!(number(&d, &mut ev, &[1, 0, 2, 0, 4]), 6, "double");
    assert_eq!(text(&d, &mut ev, &[1, 0, 2, 2, 2]), "testdouble");
}

/// What MATLAB 7 writes: one zlib stream per variable, and an array element
/// inside it.
#[test]
fn a_compressed_element_inflates_into_another_element() {
    let (d, mut ev) = open!("testdouble_7.1_GLNX86.mat");
    assert_eq!(number(&d, &mut ev, &[1, 0, 0]), 15, "compressed");
    let stream = ev.node(&d, &[1, 0, 2]).unwrap();
    assert_eq!(stream.type_name, "zlib");
    // Inside the stream, the array and its name.
    assert_eq!(number(&d, &mut ev, &[1, 0, 2, 0, 0]), 14, "array");
    assert_eq!(text(&d, &mut ev, &[IN_ZLIB, &[2, 2]].concat()), "testdouble");
    let values = ev.node(&d, &[IN_ZLIB, &[3, 0, 2]].concat()).unwrap();
    assert_eq!((values.type_name.as_str(), values.child_count), ("f64 le[]", 9));
}

/// A dimensions element is padded like any other, which a matrix with an odd
/// number of dimensions is the only thing that shows.
#[test]
fn three_dimensions_leave_four_bytes_of_padding() {
    let (d, mut ev) = open!("test3dmatrix_7.4_GLNX86.mat");
    let sizes = ev.node(&d, &[IN_ZLIB, &[1, 2]].concat()).unwrap();
    assert_eq!(sizes.child_count, 3);
    let padding = ev.node(&d, &[IN_ZLIB, &[1, 3]].concat()).unwrap();
    assert_eq!(padding.size_bits, 32);
}

/// Complex numbers are two elements, and what says there is a second one is a
/// flag in the array's first word.
#[test]
fn a_complex_array_carries_a_second_run_of_numbers() {
    let (d, mut ev) = open!("testcomplex_7.4_GLNX86.mat");
    let flags = ev.node(&d, &[IN_ZLIB, &[0, 3]].concat()).unwrap();
    assert!(matches!(&flags.value, Value::Flags { set, .. } if set.iter().any(|f| f == "complex")), "{:?}", flags.value);
    let real = ev.node(&d, &[IN_ZLIB, &[3, 0, 2]].concat()).unwrap();
    let imaginary = ev.node(&d, &[IN_ZLIB, &[3, 1, 2]].concat()).unwrap();
    assert_eq!(real.type_name, "f64 le[]");
    assert_eq!(imaginary.type_name, "f64 le[]");
    assert_eq!(real.child_count, imaginary.child_count);
}

/// A logical array is stored as bytes and says it is logical in the same word
/// that says what class it is.
#[test]
fn a_logical_array_is_bytes_with_a_flag_on_it() {
    let (d, mut ev) = open!("testbool_8_WIN64.mat");
    assert_eq!(number(&d, &mut ev, &[IN_ZLIB, &[0, 2]].concat()), 9, "uint8");
    let flags = ev.node(&d, &[IN_ZLIB, &[0, 3]].concat()).unwrap();
    assert!(matches!(&flags.value, Value::Flags { set, .. } if set.iter().any(|f| f == "logical")), "{:?}", flags.value);
}

/// A structure's fields are elements of their own, with the names beside them.
#[test]
fn a_structure_names_its_fields_before_it_writes_them() {
    let (d, mut ev) = open!("teststructnest_7.4_GLNX86.mat");
    assert_eq!(number(&d, &mut ev, &[IN_ZLIB, &[0, 2]].concat()), 2, "struct");
    assert_eq!(number(&d, &mut ev, &[IN_ZLIB, &[3, 0, 2, 0]].concat()), 4, "four bytes to a name");
    assert_eq!(text(&d, &mut ev, &[IN_ZLIB, &[3, 1, 2]].concat()), "one\0two\0");
    let fields = ev.node(&d, &[IN_ZLIB, &[3, 2]].concat()).unwrap();
    assert_eq!(fields.child_count, 2);
}

/// A short tag packs the byte count and the type into one word, the count in
/// its top half. Which of the two comes first is the byte order, so a
/// big-endian file is the only thing that catches a reading with them the
/// wrong way round: the field name length below is always written this way.
#[test]
fn a_short_tag_swaps_its_halves_with_the_byte_order() {
    let (d, mut ev) = open!("teststruct_6.1_SOL2.mat");
    let length = &[1, 0, 2, 3, 0];
    // The row takes its name from the field and its type together.
    assert_eq!(at(&d, &mut ev, length).0, "field_name_length int32");
    // The count is first and the type second, which is only true this way
    // round: read the other way it says four bytes of uint16.
    assert_eq!(number(&d, &mut ev, &[length.as_slice(), &[0]].concat()), 4);
    assert_eq!(number(&d, &mut ev, &[length.as_slice(), &[1]].concat()), 5, "int32");
    assert_eq!(ev.node(&d, &[length.as_slice(), &[2]].concat()).unwrap().type_name, "i32 be[]");
}

/// A sparse array keeps its values in the compressed-column layout: a row for
/// each value, and where each column's values begin.
#[test]
fn a_sparse_array_writes_its_positions_before_its_values() {
    let (d, mut ev) = open!("testsparsecomplex_7.4_GLNX86.mat");
    assert_eq!(number(&d, &mut ev, &[IN_ZLIB, &[0, 2]].concat()), 5, "sparse");
    let rows = ev.node(&d, &[IN_ZLIB, &[3, 0, 2]].concat()).unwrap();
    let starts = ev.node(&d, &[IN_ZLIB, &[3, 1, 2]].concat()).unwrap();
    assert_eq!(rows.type_name, "i32 le[]");
    assert_eq!(starts.type_name, "i32 le[]");
    assert_eq!(ev.node(&d, &[IN_ZLIB, &[3, 3, 2]].concat()).unwrap().type_name, "f64 le[]");
}

/// A MATLAB string is an opaque array: no dimensions after its flags, and two
/// names saying where the object it stands for is kept.
#[test]
fn an_opaque_array_names_the_class_it_stands_for() {
    let (d, mut ev) = open!("testmatlabstring_7_WIN64.mat");
    assert_eq!(number(&d, &mut ev, &[IN_ZLIB, &[0, 2]].concat()), 17, "opaque");
    assert_eq!(ev.node(&d, &[IN_ZLIB, &[1]].concat()).unwrap().size_bits, 0, "no dimensions");
    assert_eq!(text(&d, &mut ev, &[IN_ZLIB, &[2, 2]].concat()), "matstring1");
    assert_eq!(text(&d, &mut ev, &[IN_ZLIB, &[3, 0, 2]].concat()), "MCOS");
    assert_eq!(text(&d, &mut ev, &[IN_ZLIB, &[3, 1, 2]].concat()), "string");
}

/// Level 7.3 is the same header with an HDF5 file behind it, in the user block
/// HDF5 allows, and it is read as the HDF5 file it is.
///
/// Every address in it counts from the superblock rather than from the front
/// of the file, which is what the origin the HDF5 is placed in says. Read the
/// other way the root group's header would land 512 bytes early, on bytes that
/// are not a header at all.
#[test]
fn a_level_7_3_file_is_hdf5_behind_the_same_header() {
    let (d, mut ev) = open!("testhdf5_7.4_GLNX86.mat");
    assert_eq!(number(&d, &mut ev, &[0, 2]), 0x0200, "level 7.3");
    let signature = ev.node(&d, &[1, 1, 0]).unwrap();
    assert_eq!(signature.offset_bits / 8, 512);
    assert!(matches!(&signature.value, Value::Magic { ok: true, .. }), "{:?}", signature.value);
    // The root group's object header, which the superblock puts at 928: 512
    // bytes of user block further in than a reading from the front would.
    let root_group = &[1, 1, 2, 14, 5, 0];
    assert_eq!(ev.node(&d, root_group).unwrap().offset_bits / 8, 512 + 928);
    // And the one variable in the file, whose name is a byte offset into the
    // local heap the root group's symbol table names.
    let name = [root_group.as_slice(), &[6, 0, 4, 2, 0, 7, 0, 6, 0, 3, 0, 4, 0, 5, 0]].concat();
    assert_eq!(text(&d, &mut ev, &name), "testdouble");
}

/// A file whose element runs past the end of it is refused rather than read
/// as far as it goes. It lives in `does-not-read` for that reason.
#[test]
fn an_element_longer_than_the_file_is_refused() {
    let Some(path) = sample("does-not-read/malformed1.mat") else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let doc = Document::new(MemSource(std::fs::read(&path).unwrap()));
    let mut ev = Evaluator::new(formats::builtin("mat").unwrap());
    assert!(ev.node(&doc, &[]).is_err(), "a file that cannot be read should say so");
}
