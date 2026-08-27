//! Built-in templates. These double as the test-bed for the IR: anything a
//! format needs that the IR cannot say is a gap in the IR, not in the format.

mod aiff;
mod appledouble;
mod au;
mod bmp;
mod bards_tale;
mod cbor;
mod dos;
mod ggml;
pub mod ggml_quant;
mod gguf;
mod git;
mod gif;
mod gzip;
mod hdf5;
pub mod h5ad;
pub mod hdf5_chunk;
mod id3;
mod iff;
mod ilbm;
mod jpeg;
mod lha;
mod lnk;
mod mca;
mod midi;
mod mp4;
mod nes;
mod old_mac;
mod pak;
mod pcx;
mod pdf;
pub mod pdf_objstm;
pub mod pdf_xref;
mod pe;
pub mod pe_tables;
mod pi1;
mod pnm;
mod qoi;
mod png;
mod safetensors;
mod sqlite;
mod swf;
mod tap;
mod tga;
mod tiff;
mod tracker;
mod vpk;
mod w4v;
mod wad;
mod wav;
mod whisper;
mod zip;
mod wasm;
pub mod wasm_disasm;
mod wasm_opcodes;

pub use aiff::aiff;
pub use appledouble::{appledouble, applesingle};
pub use au::au;
pub use bmp::bmp;
pub use bards_tale::bards_tale;
pub use cbor::cbor;
pub use dos::dos;
pub use gguf::gguf;
pub use git::{git_index, git_pack_index};
pub use gif::gif;
pub use gzip::gzip;
pub use hdf5::hdf5;
pub use id3::id3;
pub use ilbm::ilbm;
pub use jpeg::jpeg;
pub use lha::lha;
pub use lnk::lnk;
pub use mca::mca;
pub use midi::midi;
pub use mp4::mp4;
pub use nes::nes;
pub use old_mac::{binhex, compactpro, macbinary, stuffit};
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
pub use swf::swf;
pub use tap::tap;
pub use tga::tga;
pub use tiff::{camera_raw, tiff};
pub use tracker::{it, mod_file, s3m, xm};
pub use vpk::vpk;
pub use w4v::w4v;
pub use wad::wad;
pub use wav::wav;
pub use whisper::whisper;
pub use zip::zip;
pub use wasm::wasm;
pub use wasm_disasm::Module as WasmModule;

use crate::template::{Template, Ty};

/// A file that is JSON and nothing else. The values inside it are the
/// structure, and every one of them says where in the file it is written.
pub fn json() -> Template {
    Template::new("json", Ty::json())
}

/// OME-Zarr metadata is JSON. Keeping it as its own template makes the
/// store's root metadata identifiable without inventing a binary layout for
/// the separately stored Zarr chunks.
pub fn omezarr() -> Template {
    Template::new("omezarr", Ty::json())
}

pub fn builtin_names() -> &'static [&'static str] {
    &["png", "swf", "zip", "wasm", "mp4", "id3", "wav", "w4v", "midi", "mod", "s3m", "xm", "it", "sqlite", "pe", "msdos", "gguf", "whisper", "safetensors", "json", "omezarr", "bmp", "pcx", "tga", "au", "pi1", "nes", "gzip", "gif", "aiff", "ilbm", "pnm", "wad", "pak", "vpk", "mca", "tap", "lha", "lnk", "cbor", "gitindex", "gitpackidx", "qoi", "tiff", "dng", "nef", "cr2", "arw", "orf", "rw2", "pef", "srw", "jpeg", "pdf", "hdf5", "appledouble", "applesingle", "macbinary", "binhex", "stuffit", "compactpro", "bardstale"]
}

pub fn builtin(name: &str) -> Option<Template> {
    match name {
        "png" => Some(png()),
        "swf" => Some(swf()),
        "zip" => Some(zip()),
        "wasm" => Some(wasm()),
        "mp4" => Some(mp4()),
        "id3" => Some(id3()),
        "wav" => Some(wav()),
        "w4v" => Some(w4v()),
        "midi" => Some(midi()),
        "mod" => Some(mod_file()),
        "s3m" => Some(s3m()),
        "xm" => Some(xm()),
        "it" => Some(it()),
        "sqlite" => Some(sqlite()),
        "pe" => Some(pe()),
        "msdos" => Some(dos()),
        "gguf" => Some(gguf()),
        "whisper" => Some(whisper()),
        "safetensors" => Some(safetensors()),
        "json" => Some(json()),
        "omezarr" => Some(omezarr()),
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
        "lnk" => Some(lnk()),
        "cbor" => Some(cbor()),
        "gitindex" => Some(git_index()),
        "gitpackidx" => Some(git_pack_index()),
        "qoi" => Some(qoi()),
        "tiff" => Some(tiff()),
        "dng" | "nef" | "cr2" | "arw" | "orf" | "rw2" | "pef" | "srw" => Some(camera_raw(name)),
        "jpeg" => Some(jpeg()),
        "pdf" => Some(pdf()),
        "hdf5" => Some(hdf5()),
        "appledouble" => Some(appledouble()),
        "applesingle" => Some(applesingle()),
        "macbinary" => Some(macbinary()),
        "binhex" => Some(binhex()),
        "stuffit" => Some(stuffit()),
        "compactpro" => Some(compactpro()),
        "bardstale" => Some(bards_tale()),
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
    (b"FWS", "swf"),
    (b"CWS", "swf"),
    (b"ZWS", "swf"),
    (b"PK\x03\x04", "zip"),
    (b"PK\x05\x06", "zip"),
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
    // An `.h5ad` single-cell dataset, a Keras model, a NASA product: all of
    // them are this container and nothing in the first bytes says which.
    (b"\x89HDF\r\n\x1a\n", "hdf5"),
    (b"ID3", "id3"),
    (b"\x00\x05\x16\x07", "appledouble"),
    (b"\x00\x05\x16\x00", "applesingle"),
    (b"{\"", "json"),
];

/// Pick a built-in template from the first bytes of a file. `len` is the
/// length of the whole file, which a format whose header is a table of
/// offsets needs in order to weigh what the table points at: the head alone
/// cannot say whether the offsets reach past the end.
///
/// The careful tests go first and the table of signatures second. That is the
/// wrong way round from how it reads, and it is deliberate: a prefix of two or
/// three bytes is weaker evidence than a test that looks at several things and
/// weighs them, so the tests get first refusal. `{"` is a JSON file and it is
/// also the size and checksum an LHA archive could open with, and only one of
/// the two knows enough to say so.
pub fn sniff(head: &[u8], len: u64) -> Option<&'static str> {
    if head.starts_with(b"Extended Module: ") {
        Some("xm")
    } else if head.starts_with(b"IMPM") {
        Some("it")
    } else if is_s3m(head) {
        Some("s3m")
    } else if is_mod(head) {
        Some("mod")
    } else if is_macbinary(head, len) {
        Some("macbinary")
    } else if is_binhex(head) {
        Some("binhex")
    } else if is_stuffit(head) {
        Some("stuffit")
    } else if is_compactpro(head, len) {
        Some("compactpro")
    } else if is_bards_tale(head, len) {
        Some("bardstale")
    } else if is_mca(head, len) {
        Some("mca")
    } else if is_whisper(head) {
        Some("whisper")
    } else if is_safetensors(head) {
        Some("safetensors")
    } else if let Some(raw) = camera_raw_format(head) {
        Some(raw)
    } else if head.len() >= 8 && &head[4..8] == b"ftyp" {
        Some("mp4")
    } else if is_pe(head) {
        Some("pe")
    } else if is_dos(head) {
        Some("msdos")
    } else if is_lha(head) {
        Some("lha")
    } else if is_lnk(head) {
        Some("lnk")
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

fn is_s3m(head: &[u8]) -> bool {
    head.get(28) == Some(&0x1a) && head.get(29) == Some(&16) && head.get(44..48) == Some(b"SCRM")
}

fn is_mod(head: &[u8]) -> bool {
    let Some(sig) = head.get(1080..1084) else { return false };
    matches!(sig, b"M.K." | b"M!K!" | b"M&K!" | b"FLT4" | b"FLT8" | b"4CHN" | b"6CHN" | b"8CHN" | b"OKTA" | b"CD81")
        || (sig[0].is_ascii_digit() && sig[1..] == *b"CHN")
        || (sig[..2].iter().all(u8::is_ascii_digit) && matches!(&sig[2..], b"CH" | b"CN"))
}

/// Identify the TIFF-based camera RAW formats whose bytes distinguish them.
/// Extension names are deliberately not involved: dropped files can be
/// renamed, and a normal TIFF should not become a NEF merely because of its
/// filename. DNG has its own version tag; CR2, ORF, and RW2 have fixed header
/// markers; the remaining TIFF variants require both a camera make and RAW
/// sensor metadata.
fn camera_raw_format(head: &[u8]) -> Option<&'static str> {
    if head.get(8..12) == Some(b"CR\x02\0") && (head.starts_with(b"II*\0") || head.starts_with(b"MM\0*")) {
        return Some("cr2");
    }
    if head.starts_with(b"IIRO") || head.starts_with(b"MMOR") {
        return Some("orf");
    }
    if head.starts_with(b"IIU\0") {
        return Some("rw2");
    }

    let little = if head.starts_with(b"II*\0") {
        true
    } else if head.starts_with(b"MM\0*") {
        false
    } else {
        return None;
    };
    let u16_at = |at: usize| -> Option<u16> {
        let b: [u8; 2] = head.get(at..at + 2)?.try_into().ok()?;
        Some(if little { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let b: [u8; 4] = head.get(at..at + 4)?.try_into().ok()?;
        Some(if little { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    };
    let ifd = usize::try_from(u32_at(4)?).ok()?;
    let count = usize::from(u16_at(ifd)?);
    let mut make: Option<&[u8]> = None;
    let mut raw_sensor = false;
    for i in 0..count.min(4096) {
        let at = ifd.checked_add(2)?.checked_add(i.checked_mul(12)?)?;
        let tag = u16_at(at)?;
        let kind = u16_at(at + 2)?;
        let values = usize::try_from(u32_at(at + 4)?).ok()?;
        if tag == 50706 {
            return Some("dng");
        }
        raw_sensor |= matches!(tag, 33421 | 33422 | 34713 | 37398 | 50710..=50741 | 50778..=50834);
        if tag == 271 && kind == 2 && values > 0 {
            let start = if values <= 4 { at + 8 } else { usize::try_from(u32_at(at + 8)?).ok()? };
            make = head.get(start..start.checked_add(values.min(64))?);
        }
    }
    if !raw_sensor {
        return None;
    }
    let make = make?;
    if starts_ascii_case_insensitive(make, b"NIKON") {
        Some("nef")
    } else if starts_ascii_case_insensitive(make, b"SONY") {
        Some("arw")
    } else if starts_ascii_case_insensitive(make, b"PENTAX") || starts_ascii_case_insensitive(make, b"RICOH") {
        Some("pef")
    } else if starts_ascii_case_insensitive(make, b"SAMSUNG") {
        Some("srw")
    } else {
        None
    }
}

fn starts_ascii_case_insensitive(value: &[u8], prefix: &[u8]) -> bool {
    value.get(..prefix.len()).is_some_and(|s| s.eq_ignore_ascii_case(prefix))
}

fn is_macbinary(head: &[u8], len: u64) -> bool {
    if head.len() < 128 || head[0] != 0 || head[74] != 0 || !(1..=63).contains(&head[1]) { return false; }
    let be32 = |at: usize| u32::from_be_bytes(head[at..at + 4].try_into().expect("four bytes")) as u64;
    let be16 = |at: usize| u16::from_be_bytes(head[at..at + 2].try_into().expect("two bytes")) as u64;
    let blocks = |n: u64| n.saturating_add(127) / 128 * 128;
    128u64.saturating_add(blocks(be16(120))).saturating_add(blocks(be32(83))).saturating_add(blocks(be32(87))).saturating_add(blocks(be16(99))) <= len
}

fn is_binhex(head: &[u8]) -> bool {
    head.windows(40).any(|w| w == b"(This file must be converted with BinHex")
}

fn is_stuffit(head: &[u8]) -> bool {
    const SIGS: [&[u8; 4]; 9] = [b"SIT!", b"ST46", b"ST50", b"ST60", b"ST65", b"STin", b"STi2", b"STi3", b"STi4"];
    let classic = head.get(10..14) == Some(b"rLau") && head.get(..4).is_some_and(|s| SIGS.iter().any(|magic| s == *magic));
    let sit5 = head.starts_with(b"StuffIt (c)1997-") && head.get(20..78) == Some(b" Aladdin Systems, Inc., http://www.aladdinsys.com/StuffIt/");
    classic || sit5
}

fn is_compactpro(head: &[u8], len: u64) -> bool {
    let Some(raw) = head.get(4..8) else { return false };
    let directory = u32::from_be_bytes(raw.try_into().expect("four bytes")) as u64;
    head.get(..2) == Some(&[1, 1]) && (8..len).contains(&directory)
}

/// Whether these bytes are a Bard's Tale I DOS `.TPW` file.
///
/// The format has no magic number. Its two record sizes, the discriminator at
/// byte 16, a printable NUL-padded name and the zero high bytes of the first
/// three character enums together make a much narrower signature than size or
/// extension alone.
fn is_bards_tale(head: &[u8], len: u64) -> bool {
    if head.len() < 17 || !matches!((len, head[16]), (109, 1) | (113, 2)) {
        return false;
    }
    let name = &head[..16];
    let Some(end) = name.iter().position(|&b| b == 0) else { return false };
    if end == 0 || !name[..end].iter().all(|&b| b.is_ascii_graphic() || b == b' ') || name[end..].iter().any(|&b| b != 0) {
        return false;
    }
    if head[16] == 2 {
        return true;
    }
    head.len() >= 23
        && head[18] == 0
        && head[20] == 0
        && head[22] == 0
        && head[17] & 1 == 0
        && head[19] <= 6
        && head[21] <= 9
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

/// Whether these leading bytes are a Shell Link (`.lnk`).  Its header size
/// and LinkCLSID together are fixed by the format, which is strong enough to
/// identify a shortcut without relying on its filename extension.
fn is_lnk(head: &[u8]) -> bool {
    head.len() >= 20
        && head[..4] == [0x4c, 0, 0, 0]
        && head[4..20] == [
            0x01, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00,
            0xc0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
        ]
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

/// Whether these leading bytes are a Minecraft Anvil region.
///
/// The format has no magic number: the file opens with its two tables, 1024
/// entries of four bytes each, and then the chunks the first table points at.
/// Nothing about one entry distinguishes it from noise, so the file is told by
/// the shape of all of them at once. Most entries of a real region are all
/// zeroes, the chunks never having been generated, and every one that is not
/// points past both tables, stays inside the file, and carries a length of at
/// least one sector. A single entry against that is a table of something
/// else, so it turns the file away rather than being averaged out.
///
/// The second table backs the first: entries there are seconds since 1970,
/// and a chunk a world has saved has a stamp between 2010, the years the game
/// is from, and 2100. The counts are weighed apart, one entry to the two
/// tables or more, because a world is saved a chunk at a time and stamps
/// outlive their entries and entries their stamps.
///
/// The head must hold both tables whole: an 8 KiB read is the smallest that
/// says anything about this format, and a shorter one is no evidence at all.
/// The length of the file is the other witness, since the pointers are only
/// worth weighing against the size they address.
fn is_mca(head: &[u8], len: u64) -> bool {
    if head.len() < 8192 || len < 12288 || len % 4096 != 0 {
        return false;
    }
    let mut present = 0;
    for i in 0..1024 {
        let at = i * 4;
        let sector = u32::from_be_bytes([0, head[at], head[at + 1], head[at + 2]]);
        let sectors = u32::from(head[at + 3]);
        if sector | sectors == 0 {
            continue;
        }
        let within = sector >= 2 && (u64::from(sector) + u64::from(sectors)) * 4096 <= len;
        if !within {
            return false;
        }
        present += 1;
    }
    if present == 0 {
        return false;
    }
    let mut dated = 0;
    for i in 0..1024 {
        let at = 4096 + i * 4;
        let stamp = u32::from_be_bytes([head[at], head[at + 1], head[at + 2], head[at + 3]]);
        // Seconds since 1970 between the start of 2010 and the start of 2100.
        if (1262304000..4102444800).contains(&stamp) {
            dated += 1;
        } else if stamp != 0 {
            return false;
        }
    }
    dated > 0 && dated * 2 >= present
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

    /// `sniff` over a file that is exactly these bytes and no more.
    ///
    /// The length is a real argument, not a formality: a format with no
    /// signature is recognised by whether the table in its head points inside
    /// the file, which the head alone cannot say. Where a test cares what that
    /// length is, it calls `sniff` and gives one; where it does not, saying
    /// "the file is what you see" is the honest stand-in for a number nobody
    /// chose.
    fn sniffed(head: &[u8]) -> Option<&'static str> {
        sniff(head, head.len() as u64)
    }

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
        assert_eq!(sniff(&mz(0x80, 0x100, true), 0x100), Some("pe"));
    }

    #[test]
    fn a_dos_executable_is_not() {
        // Relocations before 0x40, so there is no room for a later header.
        let mut v = vec![0u8; 0x100];
        v[0..2].copy_from_slice(b"MZ");
        v[0x18..0x1a].copy_from_slice(&0x1eu16.to_le_bytes());
        assert_eq!(sniff(&v, v.len() as u64), Some("msdos"));
        // Room left for one, and nothing in it.
        assert_eq!(sniff(&mz(0x80, 0x100, false), 0x100), Some("msdos"));
    }

    #[test]
    fn a_header_past_what_was_read_is_not_claimed() {
        // It may be a Windows executable, and reading it as a DOS one would
        // describe the stub that exists to say the program needs Windows.
        assert_eq!(sniff(&mz(0x400, 0x100, false), 0x100), None);
        // Even a short read of a real PE: better unclaimed than wrong.
        assert_eq!(sniff(&mz(0x80, 0x40, false), 0x40), None);
    }

    #[test]
    fn a_later_format_with_no_template_is_left_to_the_rules() {
        let mut v = mz(0x80, 0x100, false);
        // Windows 3.x, which is neither a PE nor a DOS program.
        v[0x80..0x82].copy_from_slice(b"NE");
        assert_eq!(sniff(&v, v.len() as u64), None);
    }

    #[test]
    fn a_whisper_model_is_told_from_the_other_ggml_files_by_its_mel_bands() {
        let mut v = b"lmgg".to_vec();
        v.resize(0x2c, 0);
        v[0x28..0x2c].copy_from_slice(&80u32.to_le_bytes());
        assert_eq!(sniff(&v, v.len() as u64), Some("whisper"));
        // A language model of the same era, which this cannot read.
        v[0x28..0x2c].copy_from_slice(&11008u32.to_le_bytes());
        assert_eq!(sniff(&v, v.len() as u64), None);
    }

    #[test]
    fn a_safetensors_file_is_told_by_its_header_length_and_the_json_after_it() {
        let mut v = 1024u64.to_le_bytes().to_vec();
        v.extend_from_slice(br#"{"a.weight":{"dtype":"F16""#);
        assert_eq!(sniff(&v, v.len() as u64), Some("safetensors"));
        // A length no header could have, whatever follows it.
        let mut v = u64::MAX.to_le_bytes().to_vec();
        v.extend_from_slice(br#"{"a":1}"#);
        assert_eq!(sniff(&v, v.len() as u64), None);
    }

    /// The front of a region: `present` chunks in consecutive sectors after
    /// the two tables, a saved stamp apiece, and every other entry zero, as
    /// most of the 1024 in a real region are.
    fn region(present: usize) -> Vec<u8> {
        let mut v = vec![0u8; 8192];
        for i in 0..present {
            let at = i * 4;
            let sector = 2u32 + i as u32;
            v[at..at + 3].copy_from_slice(&sector.to_be_bytes()[1..]);
            v[at + 3] = 1;
            v[4096 + at..4096 + at + 4].copy_from_slice(&1_700_000_000u32.to_be_bytes());
        }
        v
    }

    #[test]
    fn an_anvil_region_is_told_by_the_shape_of_its_tables() {
        assert_eq!(sniff(&region(182), (2 + 182) * 4096), Some("mca"));
        // Two chunks in the smallest region the tables allow.
        assert_eq!(sniff(&region(2), 4 * 4096), Some("mca"));
    }

    #[test]
    fn a_region_table_with_a_pointer_past_the_end_is_not_one() {
        let mut v = region(182);
        let at = 5 * 4;
        v[at..at + 3].copy_from_slice(&9_000u32.to_be_bytes()[1..]);
        assert_eq!(sniff(&v, (2 + 182) * 4096), None);
        // As are stamps that are seconds from no year a world was saved in.
        let mut v = region(182);
        let at = 4096 + 5 * 4;
        v[at..at + 4].copy_from_slice(&31_536_000u32.to_be_bytes());
        assert_eq!(sniff(&v, (2 + 182) * 4096), None);
    }

    #[test]
    fn tables_that_say_too_little_say_nothing() {
        let len = (2 + 182) * 4096;
        // Nothing but zeroes is not a region, whatever its length.
        assert_eq!(sniff(&region(0), 4 * 4096), None);
        // Less than both tables arriving is no evidence at all.
        assert_eq!(sniff(&region(182)[..8191], len), None);
        // A length that is not a count of sectors is not a region either.
        assert_eq!(sniff(&region(182), len + 1), None);
    }

    #[test]
    fn the_other_formats_still_answer() {
        assert_eq!(sniffed(b"\x89PNG\r\n\x1a\n"), Some("png"));
        assert_eq!(sniffed(b"\0asm\x01\0\0\0"), Some("wasm"));
        assert_eq!(sniffed(b"SQLite format 3\0"), Some("sqlite"));
        assert_eq!(sniffed(b"qoif\0\0\x01\0\0\0\x01\0\x04\0"), Some("qoi"));
        assert_eq!(sniffed(b"GIF89a"), Some("gif"));
        // Both ways round, and the 42 after the letters is written the way
        // the letters just said it would be.
        assert_eq!(sniffed(b"II*\x00\x08\x00\x00\x00"), Some("tiff"));
        assert_eq!(sniffed(b"MM\x00*\x00\x00\x00\x08"), Some("tiff"));
    }

    fn little_tiff(entries: &[(u16, u16, u32, [u8; 4])], tail: &[u8]) -> Vec<u8> {
        let mut v = b"II*\0\x08\0\0\0".to_vec();
        v.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        for (tag, kind, count, value) in entries {
            v.extend_from_slice(&tag.to_le_bytes());
            v.extend_from_slice(&kind.to_le_bytes());
            v.extend_from_slice(&count.to_le_bytes());
            v.extend_from_slice(value);
        }
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(tail);
        v
    }

    #[test]
    fn dng_is_distinguished_from_an_ordinary_tiff_by_its_version_tag() {
        let dng = little_tiff(&[(50706, 1, 4, [1, 6, 0, 0])], &[]);
        assert_eq!(sniffed(&dng), Some("dng"));

        // A camera make by itself does not prove that an otherwise ordinary
        // TIFF is the maker's proprietary RAW format.
        let make_at = 8 + 2 + 12 + 4;
        let tiff = little_tiff(&[(271, 2, 6, (make_at as u32).to_le_bytes())], b"NIKON\0");
        assert_eq!(sniffed(&tiff), Some("tiff"));
    }

    #[test]
    fn tiff_raw_variants_need_both_sensor_metadata_and_the_camera_make() {
        for (make, want) in [(b"NIKON\0".as_slice(), "nef"), (b"SONY\0", "arw"), (b"PENTAX\0", "pef"), (b"SAMSUNG\0", "srw")] {
            let make_at = 8 + 2 + 2 * 12 + 4;
            let raw = little_tiff(
                &[
                    (271, 2, make.len() as u32, (make_at as u32).to_le_bytes()),
                    (33421, 3, 2, [2, 0, 2, 0]),
                ],
                make,
            );
            assert_eq!(sniffed(&raw), Some(want), "{}", String::from_utf8_lossy(make));
        }
    }

    #[test]
    fn raw_formats_with_header_markers_are_recognised_directly() {
        assert_eq!(sniffed(b"II*\0\x10\0\0\0CR\x02\0"), Some("cr2"));
        assert_eq!(sniffed(b"IIRO\x08\0\0\0"), Some("orf"));
        assert_eq!(sniffed(b"MMOR\0\0\0\x08"), Some("orf"));
        assert_eq!(sniffed(b"IIU\0\x08\0\0\0"), Some("rw2"));
    }

    #[test]
    fn a_test_that_weighs_several_things_beats_a_two_byte_prefix() {
        // An LHA archive whose header size and checksum happen to be the two
        // characters a JSON file opens with. The method at offset 2 is what
        // settles it, and the table of prefixes never gets asked.
        assert_eq!(sniffed(b"{\"-lh5-\0\0\0\0"), Some("lha"));
        // And an ordinary JSON file is still JSON.
        assert_eq!(sniffed(b"{\"name\": 1}"), Some("json"));
    }

    #[test]
    fn one_magic_number_covering_several_formats_is_settled_by_what_follows() {
        assert_eq!(sniffed(b"FORM\0\0\0\x10AIFF"), Some("aiff"));
        assert_eq!(sniffed(b"FORM\0\0\0\x10ILBM"), Some("ilbm"));
        // A JPEG opens with the start of image and then the first marker,
        // which is three bytes; the two on their own are not enough.
        assert_eq!(sniffed(b"\xff\xd8\xff\xe0\x00\x10JFIF\x00"), Some("jpeg"));
        assert_eq!(sniffed(b"\xff\xd8\xff\xdb\x00\x43\x00"), Some("jpeg"));
        assert_eq!(sniffed(b"\xff\xd8hello"), None);
        // An IFF file holding something with no template here is left alone
        // rather than read as one of the two that do.
        assert_eq!(sniffed(b"FORM\0\0\0\x108SVX"), None);
    }
}
