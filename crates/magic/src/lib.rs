//! File identification for formats Qubero has no template for.
//!
//! This is a separate wasm module, not part of `qubero-wasm`: it carries the
//! `file(1)` rule database and the engine that runs it, which together weigh
//! more than the rest of the editor. The web side imports it only after
//! `formats::sniff` has already come up empty, so a file the editor knows
//! never pays for it.
//!
//! What comes back is a label, not a template. Nothing here can lay out
//! fields; it says what the bytes are so the hex view is not silent about a
//! format the editor cannot yet describe.

use serde::Serialize;
use wasm_bindgen::prelude::*;

/// One identification, as the rule that matched described it.
#[derive(Serialize)]
struct MatchDto {
    /// The rule's own sentence: `PNG image data, 1280 x 720, 8-bit/color RGBA`.
    message: String,
    /// Media type, or "" where the rule carries none.
    mime: String,
    /// Extensions the rule lists, in the order it lists them.
    ext: Vec<String>,
    /// The rule's strength. Higher wins when several match; carried through so
    /// a caller can tell a signature match from a weak guess.
    strength: f64,
    /// Which rule file it came from, for tracking a wrong answer back.
    source: String,
}

/// Identify the bytes at the start of a file. Returns JSON, or "" for no match.
///
/// `head` should be the file's first bytes and nothing else: rules count from
/// the start of what they are given, so a window taken from anywhere else
/// reads as a file that happens to begin there. Rules that search rather than
/// test a fixed offset only see as far as the window reaches.
#[wasm_bindgen]
pub fn identify(head: &[u8]) -> String {
    let db = match magic_db::global() {
        Ok(db) => db,
        Err(_) => return String::new(),
    };
    let m = match db.best_magic_slice(head) {
        Ok(m) => m,
        Err(_) => return String::new(),
    };
    // `is_default` is the database's own last resort ("data", "ASCII text"),
    // which tells the reader nothing the hex view is not already showing.
    if m.is_default() {
        return String::new();
    }
    let message = m.message();
    if message.is_empty() {
        return String::new();
    }
    let mut ext: Vec<String> = m.extensions().iter().map(|e| e.to_string()).collect();
    ext.sort();
    let dto = MatchDto {
        message,
        mime: m.mime_type().to_string(),
        ext,
        strength: m.strength() as f64,
        source: m.source().unwrap_or_default().to_string(),
    };
    serde_json::to_string(&dto).unwrap_or_default()
}
