//! Built-in templates. These double as the test-bed for the IR: anything a
//! format needs that the IR cannot say is a gap in the IR, not in the format.

mod aiff;
mod au;
mod bmp;
mod cbor;
mod dos;
mod ggml;
pub mod ggml_quant;
mod gguf;
mod git;
mod gif;
mod gzip;
mod id3;
mod iff;
mod ilbm;
mod jpeg;
mod lha;
mod mca;
mod midi;
mod mp4;
mod nes;
mod pak;
mod pcx;
mod pdf;
pub mod pdf_xref;
mod pe;
pub mod pe_tables;
mod pi1;
mod pnm;
mod qoi;
mod png;
mod safetensors;
mod sqlite;
mod tap;
mod tga;
mod tiff;
mod vpk;
mod w4v;
mod wad;
mod wav;
mod whisper;
mod wasm;
pub mod wasm_disasm;
mod wasm_opcodes;

pub use aiff::aiff;
pub use au::au;
pub use bmp::bmp;
pub use cbor::cbor;
pub use dos::dos;
pub use gguf::gguf;
pub use git::{git_index, git_pack_index};
pub use gif::gif;
pub use gzip::gzip;
pub use id3::id3;
pub use ilbm::ilbm;
pub use jpeg::jpeg;
pub use lha::lha;
pub use mca::mca;
pub use midi::midi;
pub use mp4::mp4;
pub use nes::nes;
pub use pe::pe;
pub use pak::pak;
pub use pcx::pcx;
pub use pdf::pdf;
pub use pi1::pi1;
pub use pnm::pnm;
pub use qoi::qoi;
pub use png::png;
pub use safetensors::safetensors;
pub use sqlite::sqlite;
pub use tap::tap;
pub use tga::tga;
pub use tiff::tiff;
pub use vpk::vpk;
pub use w4v::w4v;
pub use wad::wad;
pub use wav::wav;
pub use whisper::whisper;
pub use wasm::wasm;
pub use wasm_disasm::Module as WasmModule;

use crate::template::{Template, Ty};

/// A file that is JSON and nothing else. The values inside it are the
/// structure, and every one of them says where in the file it is written.
pub fn json() -> Template {
    Template::new("json", Ty::json())
}

pub fn builtin_names() -> &'static [&'static str] {
    &["png", "wasm", "mp4", "id3", "wav", "w4v", "midi", "sqlite", "pe", "msdos", "gguf", "whisper", "safetensors", "json", "bmp", "pcx", "tga", "au", "pi1", "nes", "gzip", "gif", "aiff", "ilbm", "pnm", "wad", "pak", "vpk", "mca", "tap", "lha", "cbor", "gitindex", "gitpackidx", "qoi", "tiff", "jpeg", "pdf"]
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
        "gguf" => Some(gguf()),
        "whisper" => Some(whisper()),
        "safetensors" => Some(safetensors()),
        "json" => Some(json()),
        "bmp" => Some(bmp()),
        "pcx" => Some(pcx()),
        "tga" => Some(tga()),
        "au" => Some(au()),
        "pi1" => Some(pi1()),
        "nes" => Some(nes()),
        "gzip" => Some(gzip()),
        "gif" => Some(gif()),
        "aiff" => Some(aiff()),
        "ilbm" => Some(ilbm()),
        "pnm" => Some(pnm()),
        "wad" => Some(wad()),
        "pak" => Some(pak()),
        "vpk" => Some(vpk()),
        "mca" => Some(mca()),
        "tap" => Some(tap()),
        "lha" => Some(lha()),
        "cbor" => Some(cbor()),
        "gitindex" => Some(git_index()),
        "gitpackidx" => Some(git_pack_index()),
        "qoi" => Some(qoi()),
        "tiff" => Some(tiff()),
        "jpeg" => Some(jpeg()),
        "pdf" => Some(pdf()),
        _ => None,
    }
}

/// Formats a file announces by its first bytes and nothing more. Read in
/// order, so a longer signature that starts with a shorter one goes above it.
///
/// A format that needs more than a prefix is not here: those are the functions
/// below, which `sniff` asks first. Several formats in the tree are in neither,
/// because nothing marks the front of the file at all: a TGA, a Degas screen
/// and a CBOR document are templates to pick rather than templates to guess at.
const MAGIC: &[(&[u8], &str)] = &[
    (b"SQLite format 3\0", "sqlite"),
    (b"\x89PNG\r\n\x1a\n", "png"),
    (b"\0asm", "wasm"),
    (b"GGUF", "gguf"),
    (b"MThd", "midi"),
    (b"\x1f\x8b", "gzip"),
    (b"DIRC", "gitindex"),
    (b"\xfftOc", "gitpackidx"),
    (b"IWAD", "wad"),
    (b"PWAD", "wad"),
    // `PACK` also opens a git packfile, which is a different thing with no
    // template here. A pack writes 2 as a big-endian version next; a Quake
    // archive writes an offset that would have to be under twelve for the two
    // to be confused.
    (b"PACK", "pak"),
    (b"\x34\x12\xaa\x55", "vpk"),
    (b"NES\x1a", "nes"),
    (b"GIF8", "gif"),
    (b"qoif", "qoi"),
    // Three bytes rather than two: the marker after the start-of-image is
    // the first segment, and every JPEG has one.
    (b"\xff\xd8\xff", "jpeg"),
    (b"II*\x00", "tiff"),
    (b"MM\x00*", "tiff"),
    (b".snd", "au"),
    (b"%PDF-", "pdf"),
    (b"ID3", "id3"),
    (b"{\"", "json"),
];

/// Pick a built-in template from the first bytes of a file.
///
/// The careful tests go first and the table of signatures second. That is the
/// wrong way round from how it reads, and it is deliberate: a prefix of two or
/// three bytes is weaker evidence than a test that looks at several things and
/// weighs them, so the tests get first refusal. `{"` is a JSON file and it is
/// also the size and checksum an LHA archive could open with, and only one of
/// the two knows enough to say so.
pub fn sniff(head: &[u8]) -> Option<&'static str> {
    if is_whisper(head) {
        Some("whisper")
    } else if is_safetensors(head) {
        Some("safetensors")
    } else if head.len() >= 8 && &head[4..8] == b"ftyp" {
        Some("mp4")
    } else if is_pe(head) {
        Some("pe")
    } else if is_dos(head) {
        Some("msdos")
    } else if is_lha(head) {
        Some("lha")
    } else if is_bmp(head) {
        Some("bmp")
    } else if is_pnm(head) {
        Some("pnm")
    } else if is_pcx(head) {
        Some("pcx")
    } else if let Some((_, name)) = MAGIC.iter().find(|(magic, _)| head.starts_with(magic)) {
        Some(name)
    } else if head.starts_with(b"FORM") && head.len() >= 12 {
        // The Amiga container, whose form type says which format it holds.
        match &head[8..12] {
            b"AIFF" | b"AIFC" => Some("aiff"),
            b"ILBM" | b"PBM " => Some("ilbm"),
            _ => None,
        }
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

/// Whether these leading bytes are a Windows bitmap.
///
/// `BM` on its own is two letters a text file could open with, so this also
/// wants the DIB header after it to be one of the sizes the five versions of
/// that header have.
fn is_bmp(head: &[u8]) -> bool {
    let Some(b) = head.get(14..18) else { return false };
    head.starts_with(b"BM")
        && matches!(u32::from_le_bytes([b[0], b[1], b[2], b[3]]), 12 | 40 | 52 | 56 | 64 | 108 | 124)
}

/// Whether these leading bytes are a PCX.
///
/// The first byte is 0x0A and nothing else, but one byte is not enough on its
/// own, so the version and the encoding after it have to be values the format
/// defines too. Version 1 was never used and 0x0A is a newline, so a text file
/// starting with a blank line is turned away by the encoding byte.
fn is_pcx(head: &[u8]) -> bool {
    head.first() == Some(&0x0a) && matches!(head.get(1), Some(0 | 2 | 3 | 4 | 5)) && head.get(2) == Some(&1)
}

/// Whether these leading bytes are an LHA archive.
///
/// Nothing marks the front of the file: the first two bytes are a header size
/// and a checksum, which can be anything. What is fixed is the method at
/// offset 2, five characters of `-lh`, a digit or letter, and `-`. That is the
/// signature every tool that identifies these files uses.
fn is_lha(head: &[u8]) -> bool {
    matches!(head.get(2..7), Some([b'-', b'l', b'h' | b'z', _, b'-']))
}

/// Whether these leading bytes are a netpbm file.
///
/// P and a digit from 1 to 6, and then whitespace. Two characters alone would
/// claim any text file that happened to start with them, so the byte after the
/// digit has to be one that could separate the magic from the width.
fn is_pnm(head: &[u8]) -> bool {
    head.first() == Some(&b'P')
        && matches!(head.get(1), Some(b'1'..=b'6'))
        && matches!(head.get(2), Some(b' ' | b'\t' | b'\n' | b'\r'))
}

/// Whether these leading bytes are a whisper.cpp model.
///
/// `lmgg` is `ggml` written as a 32-bit number, and every model file ggml wrote
/// before GGUF opens with it, including the llama.cpp ones of the same era.
/// What tells a whisper model apart is `n_mels` at 0x28: an audio model has 80
/// mel bands, or 128 for large-v3, and a language model has something else
/// there entirely.
fn is_whisper(head: &[u8]) -> bool {
    if !head.starts_with(b"lmgg") || head.len() < 0x2c {
        return false;
    }
    matches!(u32::from_le_bytes([head[0x28], head[0x29], head[0x2a], head[0x2b]]), 80 | 128)
}

/// Whether these leading bytes are a safetensors file.
///
/// The format has no magic number: it opens with the length of its JSON
/// header, and then the header. So what is looked for is a length that could
/// be one, followed by the two characters an object whose first key is a
/// string starts with. A header is tens of kilobytes on the smallest real
/// model and a few megabytes on the largest.
fn is_safetensors(head: &[u8]) -> bool {
    let Some(len) = head.get(..8) else { return false };
    let len = u64::from_le_bytes(len.try_into().expect("eight bytes"));
    (2..=64 << 20).contains(&len) && head.get(8..10) == Some(b"{\"".as_slice())
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
    fn a_whisper_model_is_told_from_the_other_ggml_files_by_its_mel_bands() {
        let mut v = b"lmgg".to_vec();
        v.resize(0x2c, 0);
        v[0x28..0x2c].copy_from_slice(&80u32.to_le_bytes());
        assert_eq!(sniff(&v), Some("whisper"));
        // A language model of the same era, which this cannot read.
        v[0x28..0x2c].copy_from_slice(&11008u32.to_le_bytes());
        assert_eq!(sniff(&v), None);
    }

    #[test]
    fn a_safetensors_file_is_told_by_its_header_length_and_the_json_after_it() {
        let mut v = 1024u64.to_le_bytes().to_vec();
        v.extend_from_slice(br#"{"a.weight":{"dtype":"F16""#);
        assert_eq!(sniff(&v), Some("safetensors"));
        // A length no header could have, whatever follows it.
        let mut v = u64::MAX.to_le_bytes().to_vec();
        v.extend_from_slice(br#"{"a":1}"#);
        assert_eq!(sniff(&v), None);
    }

    #[test]
    fn the_other_formats_still_answer() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n"), Some("png"));
        assert_eq!(sniff(b"\0asm\x01\0\0\0"), Some("wasm"));
        assert_eq!(sniff(b"SQLite format 3\0"), Some("sqlite"));
        assert_eq!(sniff(b"qoif\0\0\x01\0\0\0\x01\0\x04\0"), Some("qoi"));
        assert_eq!(sniff(b"GIF89a"), Some("gif"));
        // Both ways round, and the 42 after the letters is written the way
        // the letters just said it would be.
        assert_eq!(sniff(b"II*\x00\x08\x00\x00\x00"), Some("tiff"));
        assert_eq!(sniff(b"MM\x00*\x00\x00\x00\x08"), Some("tiff"));
    }

    #[test]
    fn a_test_that_weighs_several_things_beats_a_two_byte_prefix() {
        // An LHA archive whose header size and checksum happen to be the two
        // characters a JSON file opens with. The method at offset 2 is what
        // settles it, and the table of prefixes never gets asked.
        assert_eq!(sniff(b"{\"-lh5-\0\0\0\0"), Some("lha"));
        // And an ordinary JSON file is still JSON.
        assert_eq!(sniff(b"{\"name\": 1}"), Some("json"));
    }

    #[test]
    fn one_magic_number_covering_several_formats_is_settled_by_what_follows() {
        assert_eq!(sniff(b"FORM\0\0\0\x10AIFF"), Some("aiff"));
        assert_eq!(sniff(b"FORM\0\0\0\x10ILBM"), Some("ilbm"));
        // A JPEG opens with the start of image and then the first marker,
        // which is three bytes; the two on their own are not enough.
        assert_eq!(sniff(b"\xff\xd8\xff\xe0\x00\x10JFIF\x00"), Some("jpeg"));
        assert_eq!(sniff(b"\xff\xd8\xff\xdb\x00\x43\x00"), Some("jpeg"));
        assert_eq!(sniff(b"\xff\xd8hello"), None);
        // An IFF file holding something with no template here is left alone
        // rather than read as one of the two that do.
        assert_eq!(sniff(b"FORM\0\0\0\x108SVX"), None);
    }
}
