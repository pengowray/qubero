//! Built-in templates. These double as the test-bed for the IR: anything a
//! format needs that the IR cannot say is a gap in the IR, not in the format.

mod png;
mod wasm;
mod wasm_opcodes;

pub use png::png;
pub use wasm::wasm;

use crate::template::Template;

pub fn builtin_names() -> &'static [&'static str] {
    &["png", "wasm"]
}

pub fn builtin(name: &str) -> Option<Template> {
    match name {
        "png" => Some(png()),
        "wasm" => Some(wasm()),
        _ => None,
    }
}

/// Pick a built-in template from the first bytes of a file.
pub fn sniff(head: &[u8]) -> Option<&'static str> {
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if head.starts_with(b"\0asm") {
        Some("wasm")
    } else {
        None
    }
}
