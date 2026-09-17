//! What `table_shape` replies for a file whose template declares one, as the
//! web asks for it.
//!
//! The samples live in the collection outside this repository; point
//! `QUBERO_SAMPLES` at it, or keep it beside the repository as
//! `qubero-samples`. Without it the test says so and passes.

use std::path::PathBuf;

use qubero_wasm::Editor;
use serde_json::Value;

fn samples() -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(set) = std::env::var("QUBERO_SAMPLES") {
        roots.extend(set.split(';').filter(|s| !s.is_empty()).map(PathBuf::from));
    }
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../qubero-samples"));
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../qubero-samples"));
    roots.into_iter().find(|p| p.exists())
}

fn editor(sample: &str) -> Option<Editor> {
    let Some(root) = samples() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return None;
    };
    let Ok(bytes) = std::fs::read(root.join(sample)) else {
        eprintln!("skipped: no {sample} in the sample collection");
        return None;
    };
    let chunk = 64 * 1024;
    let mut ed = Editor::new(bytes.len() as f64, chunk, 1024);
    for (i, part) in bytes.chunks(chunk as usize).enumerate() {
        ed.feed_chunk(i as f64, part);
    }
    let head = &bytes[..bytes.len().min(ed.sniff_window() as usize)];
    let name = ed.sniff_template(head, bytes.len() as f64, sample);
    assert!(ed.set_template(&name), "no template {name}");
    Some(ed)
}

fn json(reply: &str) -> Value {
    let v: Value = serde_json::from_str(reply).unwrap();
    assert_eq!(v["status"], "ok", "{v}");
    v["node"].clone()
}

#[test]
fn a_stereo_wave_replies_with_its_channels_its_rate_and_the_fields_that_say_so() {
    let Some(mut ed) = editor("wav/pcm-s16le-stereo-44100.wav") else { return };
    // The chunks, and the `data` one among them by name rather than by index.
    let chunks = json(&ed.template_node(0, &[3]));
    let n = chunks["child_count"].as_f64().unwrap() as u32;
    let data = (0..n)
        .find(|&i| json(&ed.template_node(0, &[3, i]))["name"].as_str().is_some_and(|s| s.ends_with("data")))
        .expect("a data chunk");
    let samples = [3, data, 2, 0];

    assert_eq!(json(&ed.template_node(0, &samples))["table"], true);
    let reply = ed.table_shape(0, &samples);
    eprintln!("{reply}");
    let shape = json(&reply);
    assert_eq!(shape["columns"], 2.0);
    assert_eq!(shape["rate"], 44100.0);
    assert_eq!(shape["names"], serde_json::json!(["left", "right"]));
    assert_eq!(shape["row_word"], "sample");
    assert_eq!(shape["column_word"], "channel");
    let facts = shape["facts"].as_array().expect("facts");
    assert!(!facts.is_empty());
    for f in facts {
        assert!(!f["path"].as_array().expect("a path").is_empty(), "{f} has nowhere to go");
    }

    // A field with no shape answers null rather than an error.
    assert!(json(&ed.table_shape(0, &[0])).is_null());
}
