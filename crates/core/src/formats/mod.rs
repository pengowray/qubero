//! Built-in templates. These double as the test-bed for the IR: anything a
//! format needs that the IR cannot say is a gap in the IR, not in the format.

mod id3;
mod midi;
mod mp4;
mod png;
mod sqlite;
mod w4v;
mod wav;
mod wasm;
mod wasm_opcodes;

pub use id3::id3;
pub use midi::midi;
pub use mp4::mp4;
pub use png::png;
pub use sqlite::sqlite;
pub use w4v::w4v;
pub use wav::wav;
pub use wasm::wasm;

use crate::template::Template;

pub fn builtin_names() -> &'static [&'static str] {
    &["png", "wasm", "mp4", "id3", "wav", "w4v", "midi", "sqlite"]
}

pub fn builtin(name: &str) -> Option<Template> {
    match name {
        "png" => Some(png()),
        "wasm" => Some(wasm()),
        "mp4" => Some(mp4()),
        "id3" => Some(id3()),
        "wav" => Some(wav()),
        "w4v" => Some(w4v()),
        "midi" => Some(midi()),
        "sqlite" => Some(sqlite()),
        _ => None,
    }
}

/// Pick a built-in template from the first bytes of a file.
pub fn sniff(head: &[u8]) -> Option<&'static str> {
    if head.starts_with(b"SQLite format 3\0") {
        Some("sqlite")
    } else if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if head.starts_with(b"\0asm") {
        Some("wasm")
    } else if head.len() >= 8 && &head[4..8] == b"ftyp" {
        Some("mp4")
    } else if head.starts_with(b"MThd") {
        Some("midi")
    } else if head.starts_with(b"ID3") {
        Some("id3")
    } else if head.starts_with(b"RIFF") && head.len() >= 12 && &head[8..12] == b"WAVE" {
        // The only thing that marks a W4V is the format tag inside `fmt `, so
        // this needs a few more bytes than a magic number would.
        if head.len() >= 22 && &head[12..16] == b"fmt " && &head[20..22] == b"AW" {
            Some("w4v")
        } else {
            Some("wav")
        }
    } else {
        None
    }
}
