//! What the web reads about a value its format rules out, or has no name for.
//!
//! The bytes are built here rather than taken from the sample collection: a
//! file with a signature that does not match and a colour type PNG rules out
//! is exactly the file nobody keeps. The PNG template is applied by name for
//! the same reason, since bytes with the wrong signature sniff as nothing.

use qubero_wasm::Editor;
use serde_json::Value;

/// One PNG chunk: its length, its type, its data and a CRC of zero. Nothing
/// here sums a chunk, so the sum is not what is under test.
fn chunk(kind: &[u8], data: &[u8]) -> Vec<u8> {
    let mut out = (data.len() as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&[0; 4]);
    out
}

/// A one-pixel PNG with `signature` in front of it and `color_type` in its
/// header, so either can be made wrong on its own.
fn png(signature: &[u8], color_type: u8) -> Vec<u8> {
    let mut b = signature.to_vec();
    let mut ihdr = 1u32.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&1u32.to_be_bytes());
    ihdr.extend_from_slice(&[8, color_type, 0, 0, 0]);
    b.extend_from_slice(&chunk(b"IHDR", &ihdr));
    b.extend_from_slice(&chunk(b"IEND", b""));
    b
}

fn editor(bytes: Vec<u8>) -> Editor {
    let chunk = 64 * 1024;
    let mut ed = Editor::new(bytes.len() as f64, chunk, 1024);
    for (i, part) in bytes.chunks(chunk as usize).enumerate() {
        ed.feed_chunk(i as f64, part);
    }
    assert!(ed.set_template("png"), "no png template");
    ed
}

fn node(ed: &mut Editor, path: &[u32]) -> Value {
    let reply: Value = serde_json::from_str(&ed.template_node(0, path)).unwrap();
    assert_eq!(reply["status"], "ok", "{path:?}: {reply}");
    reply["node"].clone()
}

#[test]
fn a_signature_that_does_not_match_is_invalid_and_says_what_was_wanted() {
    let mut ed = editor(png(b"PK\x03\x04\r\n\x1a\n", 6));
    let sig = node(&mut ed, &[0]);
    assert_eq!(sig["problem"]["tier"], "invalid", "{sig}");
    assert_eq!(sig["problem"]["text"], "Does not match: expected \"\\x89PNG\\r\\n\\x1a\\n\"", "{sig}");
    // The value column says what the bytes are and nothing about what they
    // are not: the reason lives in `problem` now.
    assert!(!sig["value"].as_str().unwrap().contains("does not match"), "{sig}");
    // And `ok` is gone, so nothing in the web is reading a field that is not
    // sent any more.
    assert!(sig["ok"].is_null(), "{sig}");
    // The root counts it, without having read the rest of the file to do so.
    let root = node(&mut ed, &[]);
    assert_eq!(root["problems_within"][0], 1, "{root}");
    assert!(root["problem"].is_null(), "{root}");
}

#[test]
fn a_signature_that_matches_carries_no_problem() {
    let mut ed = editor(png(b"\x89PNG\r\n\x1a\n", 6));
    let sig = node(&mut ed, &[0]);
    assert!(sig["problem"].is_null(), "{sig}");
    let color = node(&mut ed, &[1, 0, 2, 3]);
    assert!(color["problem"].is_null(), "{color}");
    assert_eq!(color["value"], "rgba (6)", "{color}");
}

#[test]
fn a_colour_type_the_format_rules_out_is_invalid_and_lists_the_allowed_ones() {
    // PNG declares which colour types exist, so a ninth is not a value
    // Qubero has no name for: the format rules it out, and the constraint
    // outranks the missing name.
    let mut ed = editor(png(b"\x89PNG\r\n\x1a\n", 9));
    let color = node(&mut ed, &[1, 0, 2, 3]);
    assert_eq!(color["problem"]["tier"], "invalid", "{color}");
    assert_eq!(color["problem"]["text"], "Unknown or invalid: must be one of 0, 2, 3, 4, 6", "{color}");
    assert_eq!(color["value"], "9 (unknown)", "{color}");
    // The header the field sits in counts it, and so does the file.
    let ihdr = node(&mut ed, &[1, 0, 2]);
    assert_eq!(ihdr["problems_within"], serde_json::json!([1, 0]), "{ihdr}");
    let root = node(&mut ed, &[]);
    assert_eq!(root["problems_within"], serde_json::json!([1, 0]), "{root}");
}

#[test]
fn a_span_carries_the_verdict_the_node_carries() {
    let mut ed = editor(png(b"PK\x03\x04\r\n\x1a\n", 6));
    let reply: Value = serde_json::from_str(&ed.spans(0, 0.0, 512.0, 64)).unwrap();
    assert_eq!(reply["status"], "ok", "{reply}");
    let spans = reply["node"].as_array().expect("spans");
    let sig = spans.iter().find(|s| s["path"] == serde_json::json!([0])).unwrap_or_else(|| panic!("no signature span: {reply}"));
    assert_eq!(sig["problem"]["tier"], "invalid", "{sig}");
    assert_eq!(sig["problem"]["text"], "Does not match: expected \"\\x89PNG\\r\\n\\x1a\\n\"", "{sig}");
}
