//! The files `joblib.dump` wrote, read as the arrays and frames that went in.
//!
//! Split out of `pickle_real.rs`, which is the same reading against the plain
//! pickles. What is here is what joblib adds: a wrapper in front of every
//! array and the array's bytes after it rather than in an opcode, a whole
//! pickle of its own where a column of objects goes, and a compressor round
//! the lot of it. `joblib_versions.rs` is the same claims across the releases
//! that wrote the wrapper differently.
//!
//! Like the rest of the collection this skips when the folder is not beside
//! the repository. Point `QUBERO_SAMPLES` at it to run it elsewhere.

use std::path::{Path, PathBuf};

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, FrameCell, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

/// One row of the tree as the listing shows it, which is what these tests
/// walk. The same shape `pickle_real.rs` walks; the two files read different
/// samples and neither is the other's helper.
struct Row {
    path: Vec<usize>,
    depth: usize,
    name: String,
    ty: String,
    at: u64,
    len: u64,
    value: Value,
}

fn walk_rows(doc: &Document<MemSource>, ev: &mut Evaluator, path: &[usize], depth: usize, out: &mut Vec<Row>) {
    let node = ev.node(doc, path).unwrap_or_else(|e| panic!("{path:?}: {e:?}"));
    out.push(Row {
        path: path.to_vec(),
        depth,
        name: node.name.clone(),
        ty: node.type_name.clone(),
        at: node.offset_bits / 8,
        len: node.size_bits / 8,
        value: node.value.clone(),
    });
    // A run of numbers is counted rather than walked: it is a value, not a
    // part of the file's shape.
    if node.type_name.ends_with("[]") {
        return;
    }
    for i in 0..node.child_count as usize {
        let mut next = path.to_vec();
        next.push(i);
        walk_rows(doc, ev, &next, depth + 1, out);
    }
}

fn row<'a>(rows: &'a [Row], name: &str) -> &'a Row {
    rows.iter().find(|r| r.name == name).unwrap_or_else(|| panic!("no {name} row"))
}

/// A cell as the interface shows it, which for a value the file has not got is
/// nothing at all.
fn cell_text(cell: &FrameCell) -> String {
    value_text(&cell.value)
}

/// The same for a value read off a node of the tree rather than out of a
/// table.
fn value_text(cell: &Option<Value>) -> String {
    match cell {
        None => String::new(),
        Some(Value::Int(n)) => n.to_string(),
        Some(Value::UInt(n)) => n.to_string(),
        Some(Value::Float(f)) => f.to_string(),
        Some(Value::Str(s)) => s.clone(),
        Some(other) => format!("{other:?}"),
    }
}

/// The plain pickles, for the frame that is dumped both ways.
fn folder() -> Option<PathBuf> {
    qubero_samples::dir("pickle")
}

/// A joblib file's nested pickle holds whatever a column of objects holds.
///
/// `joblib.dump` writes an array of objects as a whole pickle inside the
/// stream, read by the same grammar with the outer stream's memo and framing
/// put aside. So a column of dates follows from the widening rather than
/// needing anything of its own, and this is the file that says so.
#[test]
fn a_joblib_nested_pickle_holds_a_column_of_dates() {
    let Some(dir) = qubero_samples::dir("joblib") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let name = "pandas-frame-of-dates.joblib";
    let bytes = std::fs::read(dir.join(name)).unwrap();
    let found = formats::pickle::familiar::recognise(&bytes).unwrap_or_else(|| panic!("{name} matched no form"));
    assert_eq!(found.form, "mixed-values-p4-p5-v2");
    assert_eq!(found.extensions(), "builtins, stdlib, numpy, pandas, joblib");
    let doc = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(formats::builtin("picklefpf").unwrap());
    let shape = ev.table_shape(&doc, &[1]).unwrap().unwrap_or_else(|| panic!("{name}: no table"));
    assert_eq!(shape.names, ["index", "day", "reading"]);
    assert_eq!(shape.units, ["int64", "object", "float64"]);
    let read = ev.pickle_cells(&doc, &[1], 0, 3).unwrap();
    let said: Vec<Vec<String>> = read.iter().map(|row| row.iter().map(cell_text).collect()).collect();
    assert_eq!(said, [["0", "2026-09-17", "1.5"], ["1", "2026-09-18", "2.5"], ["2", "2026-09-19", "3.5"]]);
}

/// The files `joblib.dump` wrote, and which form each of them reads under.
///
/// Written out file by file, the way the pickle matrix is: a file that starts
/// matching is a form that grew without anyone saying so, and a file that
/// stops is a regression.
#[test]
fn every_joblib_sample_reads_as_the_form_it_was_dumped_under() {
    let Some(dir) = qubero_samples::dir("joblib") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    // Each file, the form that reads it, and the template it is opened with.
    // A compressed file is the compressor's; what is inside it is the form,
    // and `a_compressed_joblib_file_opens_as_the_joblib_file_it_holds` is that.
    let want: &[(&str, Option<&str>, &str)] = &[
        ("array-0d.joblib", Some("joblib-arrays-p4-p5-v1"), "joblib"),
        // An array of objects, whose values joblib cannot write as bytes: it
        // writes a whole pickle of its own where the numbers would go.
        ("array-of-objects.joblib", Some("joblib-arrays-p4-p5-v1"), "joblib"),
        ("array-big-endian.joblib", Some("joblib-arrays-p4-p5-v1"), "joblib"),
        ("array-empty.joblib", Some("joblib-arrays-p4-p5-v1"), "joblib"),
        ("array-float64.joblib", Some("joblib-arrays-p4-p5-v1"), "joblib"),
        ("array-fortran-order.joblib", Some("joblib-arrays-p4-p5-v1"), "joblib"),
        ("array-small.joblib", Some("joblib-arrays-p4-p5-v1"), "joblib"),
        ("list-of-arrays.joblib", Some("joblib-arrays-p4-p5-v1"), "joblib"),
        ("dict-of-arrays.joblib", Some("joblib-arrays-p4-p5-v1"), "joblib"),
        // The protocol is the caller's to choose, and 2 writes GLOBAL's two
        // lines and BINPUT's slot numbers where 4 writes STACK_GLOBAL and
        // MEMOIZE.
        ("dict-of-arrays-protocol2.joblib", Some("joblib-arrays-p2-p3-v1"), "joblib"),
        // An estimator saved the way scikit-learn's documentation says to,
        // which is the sklearn objects with their arrays wrapped this way.
        ("sklearn-linear-regression.joblib", Some("joblib-sklearn-p4-p5-v1"), "joblib"),
        ("sklearn-random-forest.joblib", Some("joblib-sklearn-p4-p5-v1"), "joblib"),
        // A classifier fitted on labels that are strings keeps them in
        // `classes_`, which is an array of objects and so a nested pickle.
        ("sklearn-string-labels.joblib", Some("joblib-sklearn-p4-p5-v1"), "joblib"),
        // What else goes into a joblib file beside the arrays. Each of these
        // is the wrapper and one more family in the one stream, which is the
        // mixed form: the two rows above are the two mixtures joblib was
        // given a name of its own for, and these are the rest.
        ("stdlib-and-arrays.joblib", Some("mixed-values-p4-p5-v2"), "joblib"),
        ("pandas-frame.joblib", Some("mixed-values-p4-p5-v2"), "joblib"),
        // The same frame with named columns and a text column, which are two
        // object arrays and so two nested pickles.
        ("pandas-frame-named-columns.joblib", Some("mixed-values-p4-p5-v2"), "joblib"),
        ("scipy-csr-matrix.joblib", Some("mixed-values-p4-p5-v2"), "joblib"),
        // A frame with a column of dates, so the nested pickle joblib writes
        // for that column holds dates rather than text.
        ("pandas-frame-of-dates.joblib", Some("mixed-values-p4-p5-v2"), "joblib"),
        // The breadth sweep: the estimators people actually save, each dumped
        // the way scikit-learn's own documentation says to save a model.
        // `tools/make_sklearn_breadth_samples.py` in the collection writes
        // them, and the same objects are in `pickle/` pickled plainly.
        ("sklearn-logistic-regression.joblib", Some("joblib-sklearn-p4-p5-v1"), "joblib"),
        ("sklearn-logistic-regression-string-labels.joblib", Some("joblib-sklearn-p4-p5-v1"), "joblib"),
        ("sklearn-ridge.joblib", Some("joblib-sklearn-p4-p5-v1"), "joblib"),
        ("sklearn-lasso.joblib", Some("joblib-sklearn-p4-p5-v1"), "joblib"),
        ("sklearn-sgd-classifier.joblib", Some("joblib-sklearn-p4-p5-v1"), "joblib"),
        ("sklearn-linear-svc.joblib", Some("joblib-sklearn-p4-p5-v1"), "joblib"),
        ("sklearn-svc.joblib", Some("joblib-sklearn-p4-p5-v1"), "joblib"),
        ("sklearn-k-neighbors-classifier.joblib", Some("joblib-sklearn-p4-p5-v1"), "joblib"),
        ("sklearn-gaussian-nb.joblib", Some("joblib-sklearn-p4-p5-v1"), "joblib"),
        ("sklearn-decision-tree-regressor.joblib", Some("joblib-sklearn-p4-p5-v1"), "joblib"),
        ("sklearn-random-forest-classifier.joblib", Some("joblib-sklearn-p4-p5-v1"), "joblib"),
        ("sklearn-gradient-boosting-classifier.joblib", Some("joblib-sklearn-p4-p5-v1"), "joblib"),
        ("sklearn-hist-gradient-boosting-classifier.joblib", Some("joblib-sklearn-p4-p5-v1"), "joblib"),
        ("sklearn-kmeans.joblib", Some("joblib-sklearn-p4-p5-v1"), "joblib"),
        ("sklearn-pca.joblib", Some("joblib-sklearn-p4-p5-v1"), "joblib"),
        ("sklearn-standard-scaler.joblib", Some("joblib-sklearn-p4-p5-v1"), "joblib"),
        ("sklearn-min-max-scaler.joblib", Some("joblib-sklearn-p4-p5-v1"), "joblib"),
        ("sklearn-one-hot-encoder.joblib", Some("joblib-sklearn-p4-p5-v1"), "joblib"),
        // A fitted `CountVectorizer` holds no array at all: its vocabulary is
        // a dictionary of words to positions. So there is no wrapper for
        // joblib to write and the file is byte for byte the plain pickle,
        // which is the form it reads as.
        ("sklearn-count-vectorizer.joblib", Some("sklearn-estimator-p4-p5-v1"), "picklefpf"),
        ("sklearn-tfidf-vectorizer.joblib", Some("joblib-sklearn-p4-p5-v1"), "joblib"),
        ("sklearn-label-encoder.joblib", Some("joblib-sklearn-p4-p5-v1"), "joblib"),
        ("sklearn-simple-imputer.joblib", Some("joblib-sklearn-p4-p5-v1"), "joblib"),
        ("sklearn-mlp-classifier.joblib", Some("joblib-sklearn-p4-p5-v1"), "joblib"),
        ("sklearn-pipeline-scaler-and-model.joblib", Some("joblib-sklearn-p4-p5-v1"), "joblib"),
        // Two of them hold a second family beside scikit-learn's, so the
        // joblib name is not the narrowest true one and the mixed form is.
        ("sklearn-column-transformer.joblib", Some("mixed-values-p4-p5-v2"), "joblib"),
        ("sklearn-k-neighbors-classifier-sparse-input.joblib", Some("mixed-values-p4-p5-v2"), "joblib"),
        // A `numpy.matrix`, which reaches the same wrapper every array does
        // and names `numpy.matrix` in its `subclass` where every other file
        // names `numpy.ndarray`. One of NumPy's own array classes, so it is
        // read, and the node says which class it is.
        ("v1.6-numpy-matrix.joblib", Some("joblib-arrays-p4-p5-v1"), "joblib"),
        // A fitted `GridSearchCV`, whose `cv_results_` is a dictionary of
        // masked arrays. Those are NumPy's, and the wrapped arrays beside
        // them are joblib's, so the narrowest true name is still the joblib
        // one.
        ("sklearn-grid-search-cv.joblib", Some("joblib-sklearn-p4-p5-v1"), "joblib"),
        // The compressors, each of which holds one of the files above.
        ("dict-of-arrays-zlib.joblib", None, "zlib"),
        ("dict-of-arrays-compress-true.joblib", None, "zlib"),
        ("dict-of-arrays-gzip.joblib", None, "gzip"),
        ("dict-of-arrays-bz2.joblib", None, "bzip2"),
        ("dict-of-arrays-xz.joblib", None, "xz"),
        // Raw LZMA, which is the same coder with no container round it.
        ("dict-of-arrays-lzma.joblib", None, "lzma"),
        // What joblib 1.1 and older wrote: the same wrapper with no
        // `numpy_array_alignment_bytes` key and no padding byte, so the
        // numbers begin at the BUILD. `joblib_versions.rs` reads them.
        ("v0.11-array-big-endian.joblib", Some("joblib-arrays-p2-p3-v1"), "joblib"),
        ("v0.11-array-float64.joblib", Some("joblib-arrays-p2-p3-v1"), "joblib"),
        ("v0.11-array-fortran-order.joblib", Some("joblib-arrays-p2-p3-v1"), "joblib"),
        ("v0.11-dict-of-arrays.joblib", Some("joblib-arrays-p2-p3-v1"), "joblib"),
        ("v1.1-dict-of-arrays.joblib", Some("joblib-arrays-p2-p3-v1"), "joblib"),
        ("v1.1-dict-of-arrays-zlib.joblib", None, "zlib"),
        // What joblib 0.9 wrote for an array of objects: no wrapper at all,
        // because there are no numbers to write beside the pickle, so it is
        // an ordinary NumPy pickle and reads as one.
        ("v0.9-array-of-objects.joblib", Some("numpy-array-p2-p3-v2"), "picklefpf"),
        // And what it wrote for a compressed file, which is joblib's own
        // container rather than the compressor's: `ZF`, the unpacked length
        // as text, and a zlib stream holding the whole pickle.
        ("v0.9-dict-of-arrays-zfile.joblib", None, "joblibzfile"),
        // The rest of what joblib wrote before 0.10 is in `v0.9-npy-files/`,
        // which this walk does not descend into: those pickles name their own `.npy` by
        // name, so the folder carries the version and the files keep theirs.
        // `joblib_versions.rs` reads them.
    ];
    let mut seen = Vec::new();
    for path in joblibs(&dir) {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let bytes = std::fs::read(&path).unwrap();
        let Some((_, form, template)) = want.iter().find(|(f, _, _)| *f == name) else {
            panic!("{name} is not in the matrix; add it with the form it matches, or None");
        };
        assert_eq!(formats::pickle::familiar::recognise(&bytes).map(|m| m.form), *form, "{name}");
        let window = &bytes[..bytes.len().min(0x9000)];
        assert_eq!(formats::sniff(window, bytes.len() as u64), Some(*template), "{name}");
        seen.push(name);
    }
    for (name, _, _) in want {
        assert!(seen.iter().any(|s| s == name), "{name} is in the matrix and not in the collection");
    }
}

/// The numbers a joblib file holds are the numbers that were dumped into it,
/// read from the run after the wrapper rather than from any opcode.
///
/// What `tools/make_torch_joblib_samples.py` dumped, in storage order: a
/// Fortran array's run goes down its first axis, so its numbers are not the
/// count in order.
#[test]
fn a_joblib_arrays_numbers_are_the_ones_it_was_dumped_with() {
    let Some(dir) = qubero_samples::dir("joblib") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let rows: Vec<Vec<u64>> = (0..4).map(|r: u64| (0..6).map(|c| r * 6 + c).collect()).collect();
    let down: Vec<u64> = (0..6).flat_map(|c: u64| (0..4).map(move |r| r * 6 + c)).collect();
    let cases: &[(&str, Vec<String>)] = &[
        ("array-float64.joblib", rows.concat().iter().map(u64::to_string).collect()),
        ("array-fortran-order.joblib", down.iter().map(u64::to_string).collect()),
        ("array-big-endian.joblib", (0..6).map(|n: u64| n.to_string()).collect()),
        ("array-small.joblib", (0..6).map(|n: u64| n.to_string()).collect()),
        // One value, and none at all.
        ("array-0d.joblib", vec!["2.5".into()]),
        ("array-empty.joblib", Vec::new()),
    ];
    for (name, want) in cases {
        let bytes = std::fs::read(dir.join(name)).unwrap();
        assert_eq!(&joblib_numbers(&bytes, name), want, "{name}");
    }
}

/// The numbers of the one array in a joblib file, read through the template
/// the sniffer names it with rather than the pickle one.
fn joblib_numbers(bytes: &[u8], where_: &str) -> Vec<String> {
    let doc = Document::new(MemSource(bytes.to_vec()));
    let mut ev = Evaluator::new(formats::builtin("joblib").unwrap());
    let mut rows = Vec::new();
    walk_rows(&doc, &mut ev, &[], 0, &mut rows);
    // The run of numbers, which below protocol 3 is a step under the node the
    // match placed: that node is the text the numbers were spelled as, and its
    // one child is the numbers themselves. Both are called `numbers`, so the
    // type is what tells them apart.
    let row = rows
        .iter()
        .find(|r| r.name == "numbers" && r.ty.ends_with("[]"))
        .unwrap_or_else(|| panic!("{where_}: no numbers row"));
    let node = ev.node(&doc, &row.path).unwrap();
    (0..node.child_count as usize)
        .map(|i| {
            let mut at = row.path.clone();
            at.push(i);
            value_text(&Some(ev.node(&doc, &at).unwrap().value))
        })
        .collect()
}

/// A compressed joblib file: the stream opens as a space, and the space reads
/// as the joblib file it holds rather than as text.
///
/// joblib writes the same bytes into a compressor as it writes into a plain
/// file: the same wrapper, the same `allow_mmap`, and the same padding counted
/// from the position in the unpacked stream. So there is one form, and the
/// only thing the compressor changes is where it is read.
#[test]
fn a_compressed_joblib_file_opens_as_the_joblib_file_it_holds() {
    let Some(dir) = qubero_samples::dir("joblib") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let plain = std::fs::read(dir.join("dict-of-arrays.joblib")).unwrap();
    for name in ["dict-of-arrays-zlib.joblib", "dict-of-arrays-gzip.joblib", "dict-of-arrays-bz2.joblib", "dict-of-arrays-xz.joblib", "dict-of-arrays-lzma.joblib", "dict-of-arrays-compress-true.joblib"] {
        let bytes = std::fs::read(dir.join(name)).unwrap();
        let window = &bytes[..bytes.len().min(0x9000)];
        let template = formats::sniff(window, bytes.len() as u64).unwrap_or_else(|| panic!("{name}: not recognised"));
        let doc = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(formats::builtin(template).unwrap());
        let run = stream_node(&doc, &mut ev).unwrap_or_else(|| panic!("{name}: no compressed run"));
        let id = ev.open_space(&doc, 0, &run).unwrap_or_else(|e| panic!("{name} at {run:?}: {e:?}")).unwrap_or_else(|| panic!("{name}: the stream does not open"));
        let space = ev.space(id).unwrap();
        assert_eq!(space.template, "joblib", "{name}");
        assert!(space.recognised, "{name}: the template came from the stream and not from the bytes");
        assert_eq!(space.bytes(), plain.as_slice(), "{name}: a compressor changes nothing about what joblib writes");
    }
}

/// The field holding the whole of what a compressed file unpacks to.
///
/// Named `decoded` where a format joins one stream from several runs, which
/// is what gzip does: a member is a piece of the file and only the join is the
/// file. Everywhere else it is the one field whose bytes are a stream.
fn stream_node(doc: &Document<MemSource>, ev: &mut Evaluator) -> Option<Vec<usize>> {
    let root = ev.node(doc, &[]).ok()?;
    for i in 0..root.child_count as usize {
        if ev.node(doc, &[i]).ok()?.name != "decoded" {
            continue;
        }
        // bzip2 and xz lay the field over the whole stream, so the run is one
        // level further down; gzip joins the members and the join is the run.
        return decoded_node(doc, ev, &[i], 0).or(Some(vec![i]));
    }
    decoded_node(doc, ev, &[], 0)
}

/// The first field of a reading whose bytes are a stream of their own.
fn decoded_node(doc: &Document<MemSource>, ev: &mut Evaluator, path: &[usize], depth: usize) -> Option<Vec<usize>> {
    if depth > 6 {
        return None;
    }
    let node = ev.node(doc, path).ok()?;
    if node.decoded {
        return Some(path.to_vec());
    }
    for i in 0..node.child_count as usize {
        let mut next = path.to_vec();
        next.push(i);
        if let Some(found) = decoded_node(doc, ev, &next, depth + 1) {
            return Some(found);
        }
    }
    None
}

fn joblibs(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "joblib"))
        .collect();
    out.sort();
    out
}

/// A frame dumped by joblib opens as the same table as the same frame pickled
/// plainly, cell for cell.
///
/// The two files are nothing alike. Plainly pickled, every column's numbers
/// are inside the stream; dumped by joblib, each is a wrapper and a run of
/// bytes after it, except the column names and the text column, which have no
/// bytes to write and are a whole pickle each. The table is the claim that
/// none of that reaches the reader.
#[test]
fn a_joblib_frame_opens_as_the_table_the_same_frame_pickled_plainly_does() {
    let (Some(dir), Some(plain)) = (qubero_samples::dir("joblib"), folder()) else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    // `tools/make_torch_joblib_samples.py` dumps the same frame
    // `tools/make_mixed_pickle_samples.py` pickles: three rows, two numbers
    // and a label.
    let want: &[&[&str]] = &[&["0", "1", "0.5", "a"], &["1", "2", "1.5", "b"], &["2", "3", "2.5", "c"]];
    let read = |path: &Path, at: &[usize]| {
        let bytes = std::fs::read(path).unwrap();
        let doc = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(formats::builtin("picklefpf").unwrap());
        let where_ = path.file_name().unwrap().to_string_lossy().into_owned();
        let shape = ev.table_shape(&doc, at).unwrap().unwrap_or_else(|| panic!("{where_}: no table"));
        let cells = ev.pickle_cells(&doc, at, 0, want.len() as u64 + 1).unwrap();
        let said: Vec<Vec<String>> = cells.iter().map(|row| row.iter().map(cell_text).collect()).collect();
        (shape.names, shape.units, said)
    };
    // The joblib file holds the frame and nothing else; the plain one holds a
    // list of two frames and this is the first of them.
    let dumped = read(&dir.join("pandas-frame-named-columns.joblib"), &[1]);
    let pickled = read(&plain.join("mixed-frames-sharing-placements-p4.pickle"), &[1, 3]);
    assert_eq!(dumped.0, ["index", "x", "y", "label"]);
    assert_eq!(dumped.1, ["int64", "int64", "float64", "str"]);
    let want: Vec<Vec<String>> = want.iter().map(|row| row.iter().map(|c| (*c).to_string()).collect()).collect();
    assert_eq!(dumped.2, want);
    assert_eq!(dumped, pickled, "the same frame, one dumped and one pickled");
}

/// An array of objects dumped by joblib reads as the values that were in it,
/// which is more than text: a pandas column of objects holds strings, numbers
/// and the missing entries between them.
#[test]
fn an_object_array_joblib_wrote_reads_as_the_values_it_holds() {
    let Some(dir) = qubero_samples::dir("joblib") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let bytes = std::fs::read(dir.join("array-of-objects.joblib")).unwrap();
    let doc = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(formats::builtin("joblib").unwrap());
    let mut rows = Vec::new();
    walk_rows(&doc, &mut ev, &[], 0, &mut rows);
    // The pickle joblib wrote where the numbers would go, with a protocol of
    // its own: the stream is protocol 4 and this one is protocol 5.
    let nested = row(&rows, "nested pickle");
    let protocols: Vec<&Row> = rows.iter().filter(|r| r.name == "protocol").collect();
    assert_eq!(protocols.len(), 2);
    assert_eq!(value_text(&Some(protocols[0].value.clone())), "4");
    assert_eq!(value_text(&Some(protocols[1].value.clone())), "5");
    assert!(protocols[1].at > nested.at && protocols[1].at < nested.at + nested.len);
    // `numpy.array(["a", None, 3], dtype=object)`, in order: a string, the
    // singleton and a number, which is more than the text and `None` an
    // object array held before.
    let values: Vec<(String, String)> = rows
        .iter()
        .filter(|r| r.depth == nested.depth + 1 && r.name.starts_with('['))
        .map(|r| (r.ty.clone(), value_text(&Some(r.value.clone()))))
        .collect();
    assert_eq!(values[0], ("utf8[]".to_string(), "a".to_string()));
    assert_eq!(values[1].0, "null");
    assert_eq!(values[2], ("u8".to_string(), "3".to_string()));
    assert_eq!(values.len(), 3);
}
