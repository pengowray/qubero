//! Built-in templates. These double as the test-bed for the IR: anything a
//! format needs that the IR cannot say is a gap in the IR, not in the format.

mod dos;
mod id3;
mod midi;
mod mp4;
mod pe;
pub mod pe_tables;
mod png;
mod sqlite;
mod w4v;
mod wav;
mod wasm;
pub mod wasm_disasm;
mod wasm_opcodes;

pub use dos::dos;
pub use id3::id3;
pub use midi::midi;
pub use mp4::mp4;
pub use pe::pe;
pub use png::png;
pub use sqlite::sqlite;
pub use w4v::w4v;
pub use wav::wav;
pub use wasm::wasm;
pub use wasm_disasm::Module as WasmModule;

use crate::template::Template;

pub fn builtin_names() -> &'static [&'static str] {
    &["png", "wasm", "mp4", "id3", "wav", "w4v", "midi", "sqlite", "pe", "msdos"]
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
        "pe" => Some(pe()),
        "msdos" => Some(dos()),
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
    } else if is_pe(head) {
        Some("pe")
    } else if is_dos(head) {
        Some("msdos")
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

/// Whether these leading bytes are a DOS executable and nothing newer.
///
/// Everything in the `MZ` family opens the same way, and what says whether a
/// header of a later format follows is `relocation_table` at 0x18: a DOS
/// program's relocations start before 0x40, which is where the pointer to such
/// a header would have to be. A file that leaves room for one is claimed here
/// only once the bytes it points at have been seen and are none of `PE`, `NE`,
/// `LE` or `LX`. A pointer past what has been read leaves the file unclaimed, which is
/// the same answer `is_pe` gives to a short read and for the same reason.
fn is_dos(head: &[u8]) -> bool {
    if !head.starts_with(b"MZ") || head.len() < 0x1c {
        return false;
    }
    if u16::from_le_bytes([head[0x18], head[0x19]]) < 0x40 {
        return true;
    }
    let Some(b) = head.get(0x3c..0x40) else { return false };
    let at = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize;
    match head.get(at..at + 2) {
        Some(sig) => !matches!(sig, b"PE" | b"NE" | b"LE" | b"LX"),
        None => false,
    }
}

/// Whether these leading bytes are a Windows executable rather than a DOS one.
///
/// Both open with `MZ`. What separates them is a PE signature at the offset
/// held at 0x3c, so this needs to see that far into the file: on the files
/// Windows ships that is 0x80 to 0x100, but nothing fixes it. A file whose
/// header sits past what has been read is left unclaimed rather than guessed
/// at, since claiming it would put a template on every DOS program too.
fn is_pe(head: &[u8]) -> bool {
    if !head.starts_with(b"MZ") || head.len() < 0x40 {
        return false;
    }
    let at = u32::from_le_bytes([head[0x3c], head[0x3d], head[0x3e], head[0x3f]]) as usize;
    match at.checked_add(4) {
        Some(end) if end <= head.len() => &head[at..end] == b"PE\0\0",
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An `MZ` file that leaves room for a header of a later format, as
    /// everything from a Windows executable to a DOS extender does: its
    /// relocations start at 0x40, past the pointer at 0x3c.
    fn mz(pe_at: u32, len: usize, signature: bool) -> Vec<u8> {
        let mut v = vec![0u8; len];
        v[0..2].copy_from_slice(b"MZ");
        v[0x18..0x1a].copy_from_slice(&0x40u16.to_le_bytes());
        v[0x3c..0x40].copy_from_slice(&pe_at.to_le_bytes());
        if signature {
            let at = pe_at as usize;
            v[at..at + 4].copy_from_slice(b"PE\0\0");
        }
        v
    }

    #[test]
    fn a_windows_executable_is_a_pe() {
        assert_eq!(sniff(&mz(0x80, 0x100, true)), Some("pe"));
    }

    #[test]
    fn a_dos_executable_is_not() {
        // Relocations before 0x40, so there is no room for a later header.
        let mut v = vec![0u8; 0x100];
        v[0..2].copy_from_slice(b"MZ");
        v[0x18..0x1a].copy_from_slice(&0x1eu16.to_le_bytes());
        assert_eq!(sniff(&v), Some("msdos"));
        // Room left for one, and nothing in it.
        assert_eq!(sniff(&mz(0x80, 0x100, false)), Some("msdos"));
    }

    #[test]
    fn a_header_past_what_was_read_is_not_claimed() {
        // It may be a Windows executable, and reading it as a DOS one would
        // describe the stub that exists to say the program needs Windows.
        assert_eq!(sniff(&mz(0x400, 0x100, false)), None);
        // Even a short read of a real PE: better unclaimed than wrong.
        assert_eq!(sniff(&mz(0x80, 0x40, false)), None);
    }

    #[test]
    fn a_later_format_with_no_template_is_left_to_the_rules() {
        let mut v = mz(0x80, 0x100, false);
        // Windows 3.x, which is neither a PE nor a DOS program.
        v[0x80..0x82].copy_from_slice(b"NE");
        assert_eq!(sniff(&v), None);
    }

    #[test]
    fn the_other_formats_still_answer() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n"), Some("png"));
        assert_eq!(sniff(b"\0asm\x01\0\0\0"), Some("wasm"));
        assert_eq!(sniff(b"SQLite format 3\0"), Some("sqlite"));
    }
}
