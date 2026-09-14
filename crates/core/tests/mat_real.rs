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

/// A structure's fields are elements of their own, with the names beside them,
/// and each is labelled with its name. A structure inside a field names its own
/// fields from its own list, not from the one around it.
#[test]
fn a_structure_names_its_fields_before_it_writes_them() {
    let (d, mut ev) = open!("teststructnest_7.4_GLNX86.mat");
    assert_eq!(number(&d, &mut ev, &[IN_ZLIB, &[0, 2]].concat()), 2, "struct");
    assert_eq!(number(&d, &mut ev, &[IN_ZLIB, &[3, 0, 2, 0]].concat()), 4, "four bytes to a name");
    // The names are one element, read as a name per four bytes rather than as
    // `one\0two\0`.
    assert_eq!(text(&d, &mut ev, &[IN_ZLIB, &[3, 1, 2, 0]].concat()), "one");
    assert_eq!(text(&d, &mut ev, &[IN_ZLIB, &[3, 1, 2, 1]].concat()), "two");
    let fields = ev.node(&d, &[IN_ZLIB, &[3, 2]].concat()).unwrap();
    assert_eq!(fields.child_count, 2);
    assert_eq!(at(&d, &mut ev, &[IN_ZLIB, &[3, 2, 0]].concat()).0, "[0] one");
    assert_eq!(at(&d, &mut ev, &[IN_ZLIB, &[3, 2, 1]].concat()).0, "[1] two");
    // `two` is a structure of one field, `three`.
    let inner = [IN_ZLIB, &[3, 2, 1, 2, 3, 2, 0]].concat();
    assert_eq!(at(&d, &mut ev, &inner).0, "[0] three");
}

/// The names as scipy reads them, on a big-endian file whose names are 32
/// bytes wide, and the path to a field is still its index.
#[test]
fn every_field_of_a_structure_is_labelled_with_its_name() {
    let (d, mut ev) = open!("teststruct_6.1_SOL2.mat");
    let fields = [1, 0, 2, 3, 2];
    let labels: Vec<String> = (0..3).map(|i| at(&d, &mut ev, &[fields.as_slice(), &[i]].concat()).0).collect();
    assert_eq!(labels, ["[0] stringfield", "[1] doublefield", "[2] complexfield"]);
    assert_eq!(ev.child_named(&d, &fields, "1").unwrap(), Some([fields.as_slice(), &[1]].concat()));
    assert_eq!(ev.child_named(&d, &fields, "doublefield").unwrap(), None);
}

/// A 1 by 2 structure array writes both fields of its first structure and then
/// both of its second, so the names go round again.
#[test]
fn a_structure_array_repeats_its_names_for_each_structure() {
    let (d, mut ev) = open!("teststructarr_7.4_GLNX86.mat");
    let fields = [IN_ZLIB, &[3, 2]].concat();
    let labels: Vec<String> = (0..4).map(|i| at(&d, &mut ev, &[fields.as_slice(), &[i]].concat()).0).collect();
    assert_eq!(labels, ["[0] one", "[1] two", "[2] one", "[3] two"]);
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
    assert_eq!(ev.node(&d, &[IN_ZLIB, &[3, 3, 2]].concat()).unwrap().type_name, "Column[]");
}

/// The values of a sparse array read a column at a time, each beside its row,
/// and put back where they belong they are the matrix scipy's `loadmat` reads:
///
/// ```text
/// [[1+1j, 2, 3, 4, 5],
///  [2,    0, 0, 0, 0],
///  [3,    0, 0, 0, 0]]
/// ```
#[test]
fn a_sparse_array_is_the_matrix_scipy_reads() {
    let (d, mut ev) = open!("testsparsecomplex_7.4_GLNX86.mat");
    let dense = |ev: &mut Evaluator, part: usize| {
        let mut m = [[0.0f64; 5]; 3];
        let columns = [IN_ZLIB, &[3, part, 2]].concat();
        assert_eq!(ev.node(&d, &columns).unwrap().child_count, 5);
        for k in 0..5 {
            let entries = [columns.as_slice(), &[k, 1]].concat();
            for i in 0..ev.node(&d, &entries).unwrap().child_count as usize {
                let row = number(&d, ev, &[entries.as_slice(), &[i, 0]].concat()) as usize;
                let Value::Float(v) = at(&d, ev, &[entries.as_slice(), &[i, 1]].concat()).1 else { panic!("not a float") };
                m[row][k] = v;
            }
        }
        m
    };
    assert_eq!(dense(&mut ev, 2), [[1.0, 2.0, 3.0, 4.0, 5.0], [2.0, 0.0, 0.0, 0.0, 0.0], [3.0, 0.0, 0.0, 0.0, 0.0]]);
    assert_eq!(dense(&mut ev, 3), [[1.0, 0.0, 0.0, 0.0, 0.0], [0.0; 5], [0.0; 5]]);
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

/// Where a path goes, by the names along it rather than by the numbers: a
/// field by the name it was declared with, and an element of a list as `#i`.
/// The subsystem is twenty levels down, and a path of twenty numbers says
/// nothing about which of them is wrong.
fn find(d: &Document<MemSource>, ev: &mut Evaluator, names: &[&str]) -> Vec<usize> {
    let mut path = Vec::new();
    for name in names {
        if let Some(i) = name.strip_prefix('#') {
            path.push(i.parse().unwrap());
            continue;
        }
        let count = ev.node(d, &path).unwrap_or_else(|e| panic!("{path:?}: {e:?}")).child_count as usize;
        let spaced = format!("{name} ");
        let found = (0..count).find(|&j| {
            let label = ev.node(d, &[path.as_slice(), &[j]].concat()).map(|n| n.name).unwrap_or_default();
            label == *name || label.starts_with(&spaced)
        });
        path.push(found.unwrap_or_else(|| panic!("nothing called {name} under {path:?}")));
    }
    path
}

fn count_at(d: &Document<MemSource>, ev: &mut Evaluator, names: &[&str]) -> usize {
    let path = find(d, ev, names);
    ev.node(d, &path).unwrap().child_count as usize
}

fn number_at(d: &Document<MemSource>, ev: &mut Evaluator, names: &[&str]) -> i128 {
    let path = find(d, ev, names);
    number(d, ev, &path)
}

fn text_at(d: &Document<MemSource>, ev: &mut Evaluator, names: &[&str]) -> String {
    let path = find(d, ev, names);
    text(d, ev, &path)
}

/// What the row is labelled with after its index: `[1] BasicClass` is
/// `BasicClass`, and a row with nothing after its index is empty.
fn label_at(d: &Document<MemSource>, ev: &mut Evaluator, names: &[&str]) -> String {
    let path = find(d, ev, names);
    let name = at(d, ev, &path).0;
    name.split_once("] ").map_or(String::new(), |(_, rest)| rest.to_string())
}

/// From the subsystem element to the file inside it.
const SUBSYSTEM: &[&str] = &["subsystem", "data", "data", "data", "contents", "real", "data"];
/// From there to the `FileWrapper__` object's cells: the first element is the
/// structure with a field for each type system, and `MCOS` is its first field.
const WRAPPER: &[&str] = &["elements", "#0", "data", "contents", "fields", "#0", "data", "contents", "reference", "data", "contents"];
/// From the cells to the table in the first of them.
const LINKING: &[&str] = &["linking", "data", "contents", "real", "data"];
/// From a variable to the words of its object reference.
const REFERENCE: &[&str] = &["data", "data", "data", "contents", "reference", "data", "contents", "real", "data"];

/// A path into the table, from the root.
fn table(names: &[&'static str]) -> Vec<&'static str> {
    [SUBSYSTEM, WRAPPER, LINKING, names].concat()
}

/// The subsystem is the element the header's offset lands on, after the
/// variables, and it is read as the small MAT file its bytes are rather than
/// as 1280 numbers.
#[test]
fn the_subsystem_is_the_element_the_header_points_at() {
    let (d, mut ev) = open!("testmatlabstring_7_WIN64.mat");
    assert_eq!(ev.node(&d, &[]).unwrap().child_count, 3, "header, body, subsystem");
    assert_eq!(ev.node(&d, &[1]).unwrap().child_count, 2, "two variables and no more");
    let offset = number(&d, &mut ev, &[0, 1]);
    assert_eq!(offset, 306);
    let subsystem = find(&d, &mut ev, &["subsystem"]);
    assert_eq!(ev.node(&d, &subsystem).unwrap().offset_bits / 8, offset as u64);
    assert_eq!(number_at(&d, &mut ev, &[SUBSYSTEM, &["version"]].concat()), 0x0100);
    assert_eq!(text_at(&d, &mut ev, &[SUBSYSTEM, &["endian_marker"]].concat()), "IM");
    // One field, for the one type system the file uses, holding the object
    // whose class is the subsystem's own.
    assert_eq!(label_at(&d, &mut ev, &[SUBSYSTEM, &WRAPPER[..6]].concat()), "MCOS");
    assert_eq!(text_at(&d, &mut ev, &[SUBSYSTEM, &WRAPPER[..8], &["class_name", "data"]].concat()), "FileWrapper__");
}

/// An `MCOS` variable holds no value, only which object it is: the marker
/// 0xDD000000, its dimensions, an object id for each object, and a class id.
/// These are the numbers scipy's `loadmat` hands back as `_ObjectMetadata`.
#[test]
fn an_mcos_variable_is_an_object_id_and_a_class_id() {
    let (d, mut ev) = open!("testmatlabstring_7_WIN64.mat");
    let word = |ev: &mut Evaluator, names: &[&str]| number_at(&d, ev, &[&["body", "#1"][..], REFERENCE, names].concat());
    let words = find(&d, &mut ev, &[&["body", "#1"][..], REFERENCE].concat());
    assert_eq!(ev.node(&d, &words).unwrap().type_name, "ObjectReference");
    assert_eq!(word(&mut ev, &["marker"]), 0xdd00_0000);
    assert_eq!(word(&mut ev, &["dimension_count"]), 2);
    assert_eq!(word(&mut ev, &["object_ids", "#0"]), 2, "matstring2 is the second object");
    assert_eq!(word(&mut ev, &["class_id"]), 1);
}

/// The table in the subsystem's first cell, for two strings: two names, one
/// class, two objects that each keep their text under a property called `any`,
/// and a cell for each of the two values.
#[test]
fn the_subsystem_table_names_the_classes_and_properties() {
    let (d, mut ev) = open!("testmatlabstring_7_WIN64.mat");
    assert_eq!(number_at(&d, &mut ev, &table(&["version"])), 4);
    assert_eq!(text_at(&d, &mut ev, &table(&["names", "#1"])), "string");
    assert_eq!(count_at(&d, &mut ev, &table(&["classes"])), 2, "the empty class nought, and string");
    assert_eq!(label_at(&d, &mut ev, &table(&["classes", "#1"])), "string");
    assert_eq!(count_at(&d, &mut ev, &table(&["objects"])), 3);
    assert_eq!(label_at(&d, &mut ev, &table(&["objects", "#2"])), "string");
    assert_eq!(number_at(&d, &mut ev, &table(&["objects", "#2", "saveobj_id"])), 2);
    assert_eq!(label_at(&d, &mut ev, &table(&["saveobj_properties", "#2", "properties", "#0"])), "any");
    assert_eq!(number_at(&d, &mut ev, &table(&["saveobj_properties", "#2", "properties", "#0", "value"])), 1);
    let values = [SUBSYSTEM, WRAPPER, &["values"]].concat();
    assert_eq!(count_at(&d, &mut ev, &values), 2, "seven cells, less the two before and three after");
}

/// Objects of classes with properties of their own: each class named with its
/// namespace, each object with its class, and each property with its name and
/// the cell its value is in.
#[test]
fn every_property_of_every_object_is_named() {
    let (d, mut ev) = open!("test_user_defined_v7.mat");
    let classes: Vec<String> = ["#1", "#2", "#3", "#4"].iter().map(|i| label_at(&d, &mut ev, &table(&["classes", *i]))).collect();
    assert_eq!(classes, ["BasicClass", "DefaultClass", "string", "HandleClass"]);
    assert_eq!(text_at(&d, &mut ev, &table(&["classes", "#1", "namespace"])), "TestClasses");
    assert_eq!(text_at(&d, &mut ev, &table(&["classes", "#3", "namespace"])), "", "string has none");
    assert_eq!(count_at(&d, &mut ev, &table(&["objects"])), 14);
    // obj_with_vals is object 2, whose properties are the second list, and
    // its `a` was set to 10.
    assert_eq!(number_at(&d, &mut ev, &table(&["objects", "#2", "normal_id"])), 2);
    let names: Vec<String> =
        ["#0", "#1", "#2"].iter().map(|i| label_at(&d, &mut ev, &table(&["properties", "#2", "properties", *i]))).collect();
    assert_eq!(names, ["a", "b", "c"]);
    assert_eq!(number_at(&d, &mut ev, &table(&["properties", "#2", "properties", "#0", "value"])), 3);
    let ten = find(&d, &mut ev, &[SUBSYSTEM, WRAPPER, &["values", "#3", "data", "contents", "real", "data", "#0"]].concat());
    assert!(matches!(at(&d, &mut ev, &ten).1, Value::Float(v) if v == 10.0));
    // A 2 by 2 array of objects is four object ids, and two variables holding
    // one handle hold the same one.
    let ids = |ev: &mut Evaluator, variable: &'static str| {
        let n = count_at(&d, ev, &[&["body", variable][..], REFERENCE, &["object_ids"]].concat());
        (0..n).map(|i| number_at(&d, ev, &[&["body", variable][..], REFERENCE, &["object_ids", format!("#{i}").as_str()]].concat())).collect::<Vec<_>>()
    };
    assert_eq!(ids(&mut ev, "#4"), [9, 10, 11, 12]);
    assert_eq!(ids(&mut ev, "#5"), ids(&mut ev, "#6"));
    // Every property whose value is in a cell names a cell there is.
    let cells = count_at(&d, &mut ev, &[SUBSYSTEM, WRAPPER, &["values"]].concat()) as i128;
    assert_eq!(cells, 32);
    let lists = find(&d, &mut ev, &table(&["properties"]));
    for l in 0..ev.node(&d, &lists).unwrap().child_count as usize {
        let props = [lists.as_slice(), &[l, 1]].concat();
        for p in 0..ev.node(&d, &props).unwrap().child_count as usize {
            let entry = [props.as_slice(), &[p]].concat();
            if number(&d, &mut ev, &[entry.as_slice(), &[1]].concat()) == 1 {
                assert!(number(&d, &mut ev, &[entry.as_slice(), &[2]].concat()) < cells, "{entry:?}");
            }
        }
    }
}

/// A MATLAB table is an object, and everything in it is in the subsystem.
#[test]
fn a_table_is_an_object_like_any_other() {
    let (d, mut ev) = open!("test_tables_v7.mat");
    assert_eq!(ev.node(&d, &[]).unwrap().child_count, 3, "nothing after the subsystem");
    assert_eq!(ev.node(&d, &[1]).unwrap().child_count, 32);
    assert_eq!(label_at(&d, &mut ev, &table(&["classes", "#1"])), "table");
    assert_eq!(label_at(&d, &mut ev, &table(&["objects", "#2"])), "table", "table_numeric");
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

/// The HDF5 panels are offered on whether the reading holds an HDF5 file, not
/// on the template's name, since `mat` reads both kinds of file. A level 7.3
/// file does and a level 5 one does not.
#[test]
fn only_a_level_7_3_file_holds_hdf5() {
    use qubero_core::formats::h5ad::holds_hdf5;
    let (d, mut ev) = open!("testhdf5_7.4_GLNX86.mat");
    assert!(holds_hdf5(&mut ev, &d).unwrap(), "level 7.3");
    for name in ["testdouble_7.1_GLNX86.mat", "teststructnest_7.4_GLNX86.mat", "testdouble_6.1_SOL2.mat", "testvec_4_GLNX86.mat"] {
        let (d, mut ev) = open!(name);
        assert!(!holds_hdf5(&mut ev, &d).unwrap(), "{name}");
    }
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
