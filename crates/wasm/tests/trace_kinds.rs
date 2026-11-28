//! What `trace_kinds` replies: every family's kinds and their shapes, as the
//! web would ask for them.

use serde_json::Value;

#[test]
fn every_family_lists_its_kinds_with_their_shapes() {
    let all: Value = serde_json::from_str(&qubero_wasm::trace_kinds("")).unwrap();
    let families = all.as_array().unwrap();
    assert_eq!(families.len(), qubero_core::codec::tracekind::Family::ALL.len());
    for f in families {
        let prefix = f["family"].as_str().unwrap();
        for k in f["kinds"].as_array().unwrap() {
            let kind = k["kind"].as_str().unwrap();
            assert!(kind.starts_with(&format!("{prefix}.")), "{kind} in {prefix}");
            assert!(!k["about"].as_str().unwrap().is_empty(), "{kind}");
            assert!(!k["forms"].as_array().unwrap().is_empty(), "{kind}");
            for c in k["children"].as_array().unwrap() {
                assert!(c.as_str().unwrap().starts_with(&format!("{prefix}.")), "{kind} has child {c}");
            }
        }
    }
}

#[test]
fn one_family_is_asked_for_by_its_prefix() {
    let deflate: Value = serde_json::from_str(&qubero_wasm::trace_kinds("deflate")).unwrap();
    let kinds = deflate[0]["kinds"].as_array().unwrap();
    let literal = kinds.iter().find(|k| k["kind"] == "deflate.literal").unwrap();
    assert_eq!(literal["forms"][0]["reads"], serde_json::json!({ "type": "code", "name": "Huffman code" }));
    assert_eq!(literal["forms"][0]["sized"], "fixed");
    assert_eq!(literal["forms"][1]["sized"], "table");
    let bfinal = kinds.iter().find(|k| k["kind"] == "deflate.bfinal").unwrap();
    assert_eq!(bfinal["forms"][0]["reads"]["type"], "enum");
    assert_eq!(bfinal["forms"][0]["reads"]["cases"][1], serde_json::json!({ "value": 1.0, "name": "the last block" }));
    let codes = kinds.iter().find(|k| k["kind"] == "deflate.codes").unwrap();
    assert!(codes["children"].as_array().unwrap().iter().any(|c| c == "deflate.match"));
    let stored = kinds.iter().find(|k| k["kind"] == "deflate.stored_data").unwrap();
    assert_eq!(
        stored["forms"][0]["reads"],
        serde_json::json!({
            "type": "record",
            "name": "Symbol",
            "kind": { "value": 3.0, "name": "stored" },
            "fields": [{ "name": "kind", "type": "SymbolKind" }, { "name": "length", "type": "computed" }],
        })
    );
    assert_eq!(qubero_wasm::trace_kinds("no such family"), "[]");
}
