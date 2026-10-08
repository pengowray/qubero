//! What `hardening` replies, as the web asks for it: pending with the chunks
//! to fetch while the bytes it needs are still on their way, then the rows,
//! and a null node for a file that is not a program.
//!
//! The samples live in the collection outside this repository; point
//! `QUBERO_SAMPLES` at it, or keep it beside the repository as
//! `qubero-samples`. Without it the test says so and passes.

use qubero_wasm::Editor;
use serde_json::Value;

const CHUNK: usize = 64 * 1024;

/// An editor over `sample` with only its first chunk loaded, and the file's
/// bytes to feed it the rest from.
fn editor(sample: &str) -> Option<(Editor, Vec<u8>)> {
    let Some(root) = qubero_samples::root() else {
        eprintln!("{}", qubero_samples::missing());
        return None;
    };
    let Ok(bytes) = std::fs::read(root.join(sample)) else {
        eprintln!("skipped: no {sample} in the sample collection");
        return None;
    };
    let mut ed = Editor::new(bytes.len() as f64, CHUNK as u32, 1024);
    ed.feed_chunk(0.0, &bytes[..bytes.len().min(CHUNK)]);
    let head = &bytes[..bytes.len().min(ed.sniff_window() as usize)];
    let name = ed.sniff_template(head, bytes.len() as f64, sample);
    assert!(ed.set_template(&name), "no template {name}");
    Some((ed, bytes))
}

/// Ask until the answer is not pending, feeding the chunks each reply asks
/// for. How many replies were pending comes back with the answer.
fn answer(ed: &mut Editor, bytes: &[u8]) -> (Value, usize) {
    for pending in 0..1000 {
        let v: Value = serde_json::from_str(&ed.hardening(0)).unwrap();
        match v["status"].as_str() {
            Some("pending") => {
                for c in v["chunks"].as_array().unwrap() {
                    let i = c.as_f64().unwrap() as usize;
                    ed.feed_chunk(i as f64, &bytes[i * CHUNK..((i + 1) * CHUNK).min(bytes.len())]);
                }
            }
            Some("ok") => return (v["node"].clone(), pending),
            _ => panic!("{v}"),
        }
    }
    panic!("still pending");
}

#[test]
fn a_program_replies_with_its_rows_once_its_bytes_are_in() {
    let Some((mut ed, bytes)) = editor("elf/busybox-aarch64") else { return };
    let (node, pending) = answer(&mut ed, &bytes);
    // The section headers are at the far end of the file, past the first
    // chunk, so the first answer has to wait for them.
    assert!(pending > 0, "answered from the first chunk alone");
    assert_eq!(node["format"], "elf");
    let part = &node["parts"][0];
    assert_eq!(part["name"], "");
    assert!(!part["path"].as_array().unwrap().is_empty());
    let rows = part["rows"].as_array().unwrap();
    let relro = rows.iter().find(|r| r["key"] == "relro").unwrap();
    assert_eq!((relro["state"].as_str(), relro["verdict"].as_str()), (Some("full"), Some("good")));
    for r in rows {
        for key in ["key", "state", "verdict", "count", "total", "items", "more", "evidence"] {
            assert!(r.get(key).is_some(), "{key} missing from {r}");
        }
        for e in r["evidence"].as_array().unwrap() {
            for key in ["path", "offset_bits", "size_bits", "what", "name"] {
                assert!(e.get(key).is_some(), "{key} missing from {e}");
            }
        }
    }
    let fortify = rows.iter().find(|r| r["key"] == "fortify").unwrap();
    assert_eq!((fortify["count"].as_f64(), fortify["total"].as_f64()), (Some(0.0), Some(38.0)));
    let relro_count = &relro["count"];
    assert!(relro_count.is_null(), "a row with no count says null: {relro_count}");
}

#[test]
fn a_file_that_is_not_a_program_has_no_rows() {
    let Some((mut ed, bytes)) = editor("wav/xc1060673-kuhls-pipistrelle-data-size-short.wav") else { return };
    let (node, _) = answer(&mut ed, &bytes);
    assert!(node.is_null(), "{node}");
}
