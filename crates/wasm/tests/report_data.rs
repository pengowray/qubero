//! What the report's data calls reply, as the web asks for them: the format's
//! description, the template's profile, and the four answers of the report's
//! walk, carried on a go at a time until `done`.
//!
//! The samples live in the collection outside this repository; point
//! `QUBERO_SAMPLES` at it, or keep it beside the repository as
//! `qubero-samples`. Without it the test says so and passes.

use qubero_wasm::Editor;
use serde_json::Value;

fn editor(sample: &str) -> Option<Editor> {
    let Some(root) = qubero_samples::root() else {
        eprintln!("{}", qubero_samples::missing());
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

/// Ask `step` until its answer says it is done.
fn until_done(mut step: impl FnMut() -> String) -> Value {
    for _ in 0..100_000 {
        let node = json(&step());
        if node["done"] == true {
            return node;
        }
    }
    panic!("never done");
}

#[test]
fn a_riff_file_replies_with_its_profile_ledger_audit_and_directories() {
    let Some(mut ed) = editor("wav/xc1060673-kuhls-pipistrelle-data-size-short.wav") else { return };

    let about: Value = serde_json::from_str(&ed.format_about("wav")).unwrap();
    assert_eq!(about["name"], "WAV");
    assert_eq!(about["wikipedia"], "WAV");
    assert_eq!(ed.format_about("ksy:nothing"), "null");

    let declared = json(&ed.template_profile(0));
    assert_eq!(declared["done"], true);
    assert!(declared["rows"].as_array().unwrap().iter().any(|r| r["category"] == "number"));

    let profile = until_done(|| ed.format_profile_step(0));
    let n16 = profile["rows"].as_array().unwrap().iter().find(|r| r["category"] == "number" && r["width"] == 16.0 && r["kind"] == "signed");
    assert!(n16.is_some_and(|r| r["order"] == "little" && r["fields"].as_f64().unwrap() >= 150_000.0), "{profile}");

    let ledger = until_done(|| ed.byte_ledger_step(0));
    assert_eq!(ledger["counted_bits"], ledger["file_bits"]);
    let data = ledger["rows"].as_array().unwrap().iter().find(|r| r["group"] == "data" && r["role"] == "content").expect("the data chunk");
    assert_eq!(data["part"], serde_json::json!([3]));

    let audit = until_done(|| ed.extent_audit_step(0));
    let riff = &audit["checks"][0];
    assert_eq!((riff["verdict"].as_str(), riff["length_name"].as_str()), (Some("past-file"), Some("size")));
    assert_eq!(riff["length_path"], serde_json::json!([1]));

    let dirs = until_done(|| ed.directories_step(0));
    assert!(dirs["lists"].as_array().unwrap().is_empty());
}
