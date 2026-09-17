//! The table shapes the templates declare, read against real files.
//!
//! A stereo WAV, whose channel count and rate are in a chunk before the
//! samples and are the reason the shape is in the IR at all; a dBase file,
//! whose records are one row each and need nothing worked out; and the two
//! other formats that read their samples as a table, AU, whose header is the
//! structure around the run rather than an earlier chunk, and AIFF, whose rate
//! is an 80-bit extended float.
//!
//! The files live in the sample collection rather than here. Point
//! `QUBERO_SAMPLES` at it, or keep it beside the repository as
//! `qubero-samples`. With neither, the test says so and passes.

use std::path::PathBuf;

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, TableShapeInfo, Value};
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

/// The run of samples inside the chunk with this id, by name for the reason
/// [`wav_samples`] gives: the chunks may come in any order. One step further
/// down than the WAV walk, because AIFF's sound chunk has an offset and a
/// block size in front of the wrapper rather than being the wrapper.
fn iff_samples(ev: &mut Evaluator, doc: &Document<MemSource>, want: &str) -> Vec<usize> {
    let chunks = ev.child_named(doc, &[], "chunks").unwrap().expect("chunks");
    let n = ev.node(doc, &chunks).unwrap().child_count;
    for i in 0..n as usize {
        let mut chunk = chunks.clone();
        chunk.push(i);
        let id = ev.child_named(doc, &chunk, "id").unwrap().expect("an id");
        match ev.node(doc, &id).unwrap().value {
            Value::Str(s) if s.trim() == want => {}
            _ => continue,
        }
        let body = ev.child_named(doc, &chunk, "body").unwrap().expect("a body");
        let wrapper = ev.child_named(doc, &body, "samples").unwrap().expect("the wrapper");
        return ev.child_named(doc, &wrapper, "samples").unwrap().expect("the samples");
    }
    panic!("no {want} chunk");
}

/// The run an `.au` wraps in its `Samples` structure.
fn au_samples(ev: &mut Evaluator, doc: &Document<MemSource>) -> Vec<usize> {
    let wrapper = ev.child_named(doc, &[], "samples").unwrap().expect("the wrapper");
    ev.child_named(doc, &wrapper, "samples").unwrap().expect("the samples")
}

/// What every run of samples should be able to say: how wide a row is, how
/// fast the rows come, what a row and a column are called, and where each of
/// the facts above the table is stored.
fn a_run_of_samples(shape: &TableShapeInfo, columns: u64, rate: u64) {
    assert_eq!(shape.columns, Some(columns));
    assert_eq!(shape.rate, Some(rate));
    assert_eq!(shape.row_word.as_deref(), Some("sample"));
    assert_eq!(shape.column_word.as_deref(), Some("channel"));
    assert!(!shape.facts.is_empty(), "no facts");
    for f in &shape.facts {
        assert!(!f.path.is_empty(), "{} has nowhere to go", f.label);
    }
}

#[test]
fn a_stereo_au_says_how_many_channels_a_row_is_and_how_fast_the_rows_come() {
    let Some((doc, mut ev)) = open("au", "pcm-s16be-stereo-22050.au", "au") else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let samples = au_samples(&mut ev, &doc);
    let node = ev.node(&doc, &samples).unwrap();
    assert!(node.table, "the samples should read as a table");
    assert_eq!(node.type_name, "i16 be[]");
    assert!(node.child_count > 0, "no samples");
    a_run_of_samples(&ev.table_shape(&doc, &samples).unwrap().expect("a shape"), 2, 22050);
}

#[test]
fn an_au_of_floats_and_one_of_mu_law_read_as_what_their_encoding_says() {
    for (name, type_name, rate) in
        [("float32-mono-44100.au", "f32 be[]", 44100), ("mulaw-mono-8000.au", "MuLaw[]", 8000)]
    {
        let Some((doc, mut ev)) = open("au", name, "au") else {
            eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
            return;
        };
        let samples = au_samples(&mut ev, &doc);
        assert_eq!(ev.node(&doc, &samples).unwrap().type_name, type_name, "{name}");
        a_run_of_samples(&ev.table_shape(&doc, &samples).unwrap().expect("a shape"), 1, rate);
    }
}

#[test]
fn an_aiff_reads_the_rate_its_common_chunk_wrote_as_an_eighty_bit_float() {
    let Some((doc, mut ev)) = open("aiff", "pcm-s16be-mono-id3.aiff", "aiff") else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    };
    let samples = iff_samples(&mut ev, &doc, "SSND");
    let node = ev.node(&doc, &samples).unwrap();
    assert!(node.table, "the samples should read as a table");
    assert_eq!(node.type_name, "i16 be[]");
    assert!(node.child_count > 0, "no samples");
    a_run_of_samples(&ev.table_shape(&doc, &samples).unwrap().expect("a shape"), 1, 22050);
}

#[test]
fn an_aifc_reads_its_samples_the_way_its_compression_id_says() {
    for (name, type_name, columns, rate) in
        [("aifc-sowt-s16le-stereo.aifc", "i16 le[]", 2, 22050), ("aifc-fl32-mono.aifc", "f32 be[]", 1, 48000)]
    {
        let Some((doc, mut ev)) = open("aiff", name, "aiff") else {
            eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
            return;
        };
        let samples = iff_samples(&mut ev, &doc, "SSND");
        assert_eq!(ev.node(&doc, &samples).unwrap().type_name, type_name, "{name}");
        a_run_of_samples(&ev.table_shape(&doc, &samples).unwrap().expect("a shape"), columns, rate);
    }
}
