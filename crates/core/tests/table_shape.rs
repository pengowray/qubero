//! The table shapes the templates declare, read against real files.
//!
//! Two of them: a stereo WAV, whose channel count and rate are in a chunk
//! before the samples and are the reason the shape is in the IR at all, and a
//! dBase file, whose records are one row each and need nothing worked out.
//!
//! The files live in the sample collection rather than here. Point
//! `QUBERO_SAMPLES` at it, or keep it beside the repository as
//! `qubero-samples`. With neither, the test says so and passes.

use std::path::PathBuf;

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

fn sample(kind: &str, name: &str) -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(set) = std::env::var("QUBERO_SAMPLES") {
        roots.extend(set.split(';').filter(|s| !s.is_empty()).map(PathBuf::from));
    }
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../qubero-samples"));
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../qubero-samples"));
    roots.into_iter().map(|r| r.join(kind).join(name)).find(|p| p.exists())
}

fn open(kind: &str, name: &str, template: &str) -> Option<(Document<MemSource>, Evaluator)> {
    let path = sample(kind, name)?;
    let doc = Document::new(MemSource(std::fs::read(&path).unwrap()));
    Some((doc, Evaluator::new(formats::template(template).expect("a template"))))
}

/// The `samples` field of the first `data` chunk, by name rather than by
/// index: a file is free to put its chunks in any order, and a hard-coded
/// index would be testing this file's order rather than the template.
fn wav_samples(ev: &mut Evaluator, doc: &Document<MemSource>) -> Vec<usize> {
    let chunks = ev.child_named(doc, &[], "chunks").unwrap().expect("chunks");
    let n = ev.node(doc, &chunks).unwrap().child_count;
    for i in 0..n as usize {
        let mut chunk = chunks.clone();
        chunk.push(i);
        let id = ev.child_named(doc, &chunk, "id").unwrap().expect("an id");
        match ev.node(doc, &id).unwrap().value {
            Value::Str(s) if s.trim() == "data" => {}
            _ => continue,
        }
        let body = ev.child_named(doc, &chunk, "body").unwrap().expect("a body");
        return ev.child_named(doc, &body, "samples").unwrap().expect("the samples");
    }
    panic!("no data chunk");
}

#[test]
fn a_stereo_wave_says_how_many_channels_a_row_is_and_how_fast_the_rows_come() {
    let Some((doc, mut ev)) = open("wav", "pcm-s16le-stereo-44100.wav", "wav") else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let samples = wav_samples(&mut ev, &doc);
    let node = ev.node(&doc, &samples).unwrap();
    assert!(node.table, "the samples should read as a table");
    assert_eq!(node.type_name, "i16 le[]");
    assert!(node.child_count > 0, "no samples");

    let shape = ev.table_shape(&doc, &samples).unwrap().expect("a shape");
    assert_eq!(shape.columns, Some(2));
    assert_eq!(shape.rate, Some(44100));
    assert_eq!(shape.names, ["left", "right"]);
    assert_eq!(shape.row_word.as_deref(), Some("sample"));
    assert_eq!(shape.column_word.as_deref(), Some("channel"));
    // Every fact is a field somewhere in the file, so every one of them has a
    // path a reader can be sent to.
    assert!(!shape.facts.is_empty(), "no facts");
    for f in &shape.facts {
        assert!(!f.path.is_empty(), "{} has nowhere to go", f.label);
    }
    let rate = shape.facts.iter().find(|f| f.label.ends_with("sample_rate")).expect("the rate among the facts");
    assert_eq!(rate.value, "44100");

    // The run itself is a table; one sample of it is not.
    let mut one = samples.clone();
    one.push(0);
    assert!(!ev.node(&doc, &one).unwrap().table);
    assert_eq!(ev.table_shape(&doc, &one).unwrap(), None);
}

#[test]
fn a_dbase_files_records_are_a_table_of_one_record_a_row() {
    let Some((doc, mut ev)) = open("dbf", "libreoffice7-dbase.dbf", "dbf") else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let records = ev.child_named(&doc, &[], "records").unwrap().expect("the records");
    assert!(ev.node(&doc, &records).unwrap().table);

    let shape = ev.table_shape(&doc, &records).unwrap().expect("a shape");
    // One record is one row, so nothing says how many elements make one.
    assert_eq!(shape.columns, None);
    assert_eq!(shape.rate, None);
    assert_eq!(shape.row_word.as_deref(), Some("record"));
    let count = shape.facts.iter().find(|f| f.label == "record_count").expect("the count among the facts");
    assert!(!count.path.is_empty());
}
