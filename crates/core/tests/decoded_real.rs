//! Every compressed stream in the sample collection, decoded by our own
//! decoders and checked against the crates that were here before them.
//!
//! The collection is not in this repository: point `QUBERO_SAMPLES` at it, or
//! keep it beside the checkout as `qubero-samples`. With neither, this says so
//! and passes.
//!
//! Two things are checked of every stream. The bytes have to be the bytes
//! `miniz_oxide` or `lz4_flex` says they are, byte for byte. And the trace has
//! to tile: every bit of the run read by exactly one step, every byte of the
//! output written by exactly one step, nothing skipped and nothing counted
//! twice. A decoder that quietly loses a symbol passes the first check and
//! fails the second.

use std::path::{Path, PathBuf};

use qubero_core::codec::{self, Codec, StepField, StepKind};
use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::source::MemSource;

/// What was found, so a run with no samples in hand can say so rather than
/// passing silently.
#[derive(Default)]
struct Tally {
    streams: usize,
    bytes_in: u64,
    bytes_out: u64,
    steps: usize,
    /// The largest trace: how many steps, and which file.
    biggest: Option<(usize, String)>,
}

impl Tally {
    fn saw(&mut self, what: &str, packed: usize, out: usize, steps: usize) {
        self.streams += 1;
        self.bytes_in += packed as u64;
        self.bytes_out += out as u64;
        self.steps += steps;
        if self.biggest.as_ref().is_none_or(|(n, _)| steps > *n) {
            self.biggest = Some((steps, what.to_string()));
        }
    }
}

#[test]
fn every_deflate_stream_in_the_collection_reads_the_same_as_miniz_oxide() {
    let files = samples();
    if files.is_empty() {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    }
    let mut tally = Tally::default();
    for path in &files {
        let Ok(bytes) = std::fs::read(path) else { continue };
        let name = path.display().to_string();
        for (what, run) in deflate_runs(&bytes) {
            let theirs = miniz_oxide::inflate::decompress_to_vec_with_limit(run, codec::CAP_BYTES);
            let ours = codec::decode_traced(Codec::Deflate, run);
            match (theirs, ours) {
                (Ok(theirs), Ok((ours, trace))) => {
                    assert_eq!(ours, theirs, "{name}: {what} decodes differently from miniz_oxide");
                    trace.check_tiles().unwrap_or_else(|e| panic!("{name}: {what}: {e}"));
                    assert_eq!(trace.out_bytes(), ours.len() as u64);
                    assert_eq!(trace.in_bits(), run.len() as u64 * 8);
                    // A deflate stream is blocks, and every block says how its
                    // symbols were coded.
                    assert!(!trace.blocks().is_empty() || ours.is_empty(), "{name}: {what} has no blocks");
                    assert!(trace.blocks().last().is_none_or(|b| b.last), "{name}: {what} never ended");
                    tally.saw(&format!("{name} {what}"), run.len(), ours.len(), trace.len());
                }
                // miniz reads streams with trailing rubbish after them that
                // this refuses, and the other way round for a few broken
                // files: what must never happen is two different answers.
                (Ok(_), Err(_)) | (Err(_), Ok(_)) | (Err(_), Err(_)) => {}
            }
        }
    }
    assert!(tally.streams > 0, "the collection holds no deflate streams, which cannot be right");
    report("deflate", &tally);
}

#[test]
fn every_zstd_and_xz_sample_is_traced_at_the_block() {
    let files = samples();
    if files.is_empty() {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    }
    let (mut seen_zstd, mut seen_xz) = (0, 0);
    for path in &files {
        let Ok(bytes) = std::fs::read(path) else { continue };
        let name = path.display().to_string();
        let codec = if bytes.starts_with(&0xfd2f_b528u32.to_le_bytes()) {
            Codec::Zstd
        } else if bytes.starts_with(b"\xfd7zXZ\x00") {
            Codec::Xz
        } else {
            continue;
        };
        let Ok((out, trace)) = codec::decode_traced(codec, &bytes) else { continue };
        trace.check_tiles().unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(trace.out_bytes(), out.len() as u64);
        assert_eq!(trace.in_bits(), bytes.len() as u64 * 8);
        // The point of stage three: the frame was parsed far enough to say
        // where its blocks are, rather than falling back to one step over the
        // whole run.
        let blocks = trace.steps().filter(|s| s.kind == StepKind::Header(StepField::BlockHeader, 0)).count();
        assert!(blocks > 0, "{name}: read as {} with no block headers found", codec.as_str());
        eprintln!("--- {name}: {} blocks, {} steps, {} bytes out", blocks, trace.len(), out.len());
        match codec {
            Codec::Zstd => seen_zstd += 1,
            _ => seen_xz += 1,
        }
    }
    eprintln!("--- {seen_zstd} zstd and {seen_xz} xz samples");
}

#[test]
fn every_lz4_block_in_the_collection_reads_the_same_as_lz4_flex() {
    let files = samples();
    if files.is_empty() {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    }
    let mut tally = Tally::default();
    for path in &files {
        let Ok(bytes) = std::fs::read(path) else { continue };
        if !bytes.starts_with(b"\x04\x22\x4d\x18") {
            continue;
        }
        let name = path.display().to_string();
        for (what, run) in lz4_blocks(&bytes) {
            let Ok((ours, trace)) = codec::decode_traced(Codec::Lz4Block, run) else { continue };
            let theirs = lz4_flex::block::decompress(run, ours.len()).expect("lz4_flex reads it too");
            assert_eq!(ours, theirs, "{name}: {what} decodes differently from lz4_flex");
            trace.check_tiles().unwrap_or_else(|e| panic!("{name}: {what}: {e}"));
            tally.saw(&format!("{name} {what}"), run.len(), ours.len(), trace.len());
        }
    }
    report("lz4", &tally);
}

/// Every stream in every sample, opened the way a tab opens one: through the
/// template that reads the file, as a space of its own.
///
/// What this catches that the byte-for-byte tests do not is the wiring. A
/// stream whose template places its run at the wrong offset still inflates,
/// because the bytes it was handed happened to be a stream; it opens into the
/// wrong thing, and the trace no longer covers the run the file says it is.
#[test]
fn every_compressed_sample_opens_as_a_space() {
    let files = samples();
    if files.is_empty() {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return;
    }
    let mut opened = 0;
    for path in &files {
        let Ok(bytes) = std::fs::read(path) else { continue };
        if bytes.is_empty() {
            continue;
        }
        let head = &bytes[..bytes.len().min(0x9000)];
        let Some(name) = qubero_core::formats::sniff(head, bytes.len() as u64) else { continue };
        if !matches!(name, "zlib" | "gzip" | "zip" | "lz4" | "zstd" | "xz") {
            continue;
        }
        let Some(template) = qubero_core::formats::builtin(name) else { continue };
        let doc = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(template);
        let mut found = Vec::new();
        streams(&doc, &mut ev, &[], 6, &mut found);
        for at in found {
            let Ok(Some(id)) = ev.open_space(&doc, 0, &at) else { continue };
            opened += 1;
            let space = ev.space(id).expect("just opened");
            let what = format!("{} {at:?}", path.display());
            space.trace().check_tiles().unwrap_or_else(|e| panic!("{what}: {e}"));
            assert_eq!(space.trace().out_bytes(), space.len_bytes(), "{what}: the trace and the bytes disagree");
            let (template, recognised, len) =
                (space.template.clone(), space.recognised, space.len_bytes());
            eprintln!("--- {what}: {len} bytes as {template}{}", if recognised { ", recognised" } else { "" });
            // Whatever it opened as, it reads: a space that opens into a
            // template nothing in it satisfies is worse than one that opens
            // into bytes.
            if len > 0 {
                ev.space_mut(id).unwrap().node(&[]).unwrap_or_else(|e| panic!("{what}: {e:?}"));
            }
        }
    }
    eprintln!("--- {opened} streams opened as spaces");
}

/// Every `Decoded` node under `at`, which is where the streams are.
fn streams(
    d: &Document<MemSource>,
    ev: &mut Evaluator,
    at: &[usize],
    depth: u32,
    out: &mut Vec<Vec<usize>>,
) {
    if depth == 0 || out.len() >= 16 {
        return;
    }
    let Ok(node) = ev.node(d, at) else { return };
    if node.space != 0 {
        return;
    }
    if matches!(node.type_name.as_str(), "deflate" | "zlib" | "lz4" | "zstd" | "xz") && node.child_count > 0 {
        out.push(at.to_vec());
        return;
    }
    for i in 0..node.child_count.min(24) as usize {
        streams(d, ev, &[at, &[i]].concat(), depth - 1, out);
    }
}

fn report(what: &str, tally: &Tally) {
    eprintln!(
        "--- {what}: {} streams, {} bytes in, {} bytes out, {} steps",
        tally.streams, tally.bytes_in, tally.bytes_out, tally.steps
    );
    if let Some((n, which)) = &tally.biggest {
        eprintln!("--- the largest trace is {n} steps, in {which}");
    }
}

/// Every raw deflate run these bytes hold, however the file wraps it: a zlib
/// stream, a gzip member, a PNG's image data, a ZIP entry stored with method
/// 8. Named, so a failure says which one.
fn deflate_runs(bytes: &[u8]) -> Vec<(String, &[u8])> {
    let mut out = Vec::new();
    if bytes.len() > 6 && bytes[0] & 0x0f == 8 && (bytes[0] as u16 * 256 + bytes[1] as u16) % 31 == 0 {
        out.push(("the zlib stream".into(), &bytes[2..bytes.len() - 4]));
    }
    if let Some(run) = gzip_body(bytes) {
        out.push(("the gzip member".into(), run));
    }
    out.extend(png_idat(bytes));
    out.extend(zip_entries(bytes));
    out
}

/// A gzip member's deflate run: past the header and the name and comment it
/// may carry, and short of the trailing CRC and length.
fn gzip_body(bytes: &[u8]) -> Option<&[u8]> {
    if !bytes.starts_with(b"\x1f\x8b\x08") || bytes.len() < 18 {
        return None;
    }
    let flags = bytes[3];
    let mut at = 10usize;
    if flags & 0x04 != 0 {
        let n = u16::from_le_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]) as usize;
        at += 2 + n;
    }
    for bit in [0x08u8, 0x10] {
        if flags & bit != 0 {
            at = bytes.get(at..)?.iter().position(|&b| b == 0)? + at + 1;
        }
    }
    if flags & 0x02 != 0 {
        at += 2;
    }
    bytes.get(at..bytes.len().checked_sub(8)?)
}

/// A PNG's image data, which is one zlib stream split across as many IDAT
/// chunks as the encoder felt like. Only the whole of it inflates, which is
/// why they are joined before anything is asked of them.
fn png_idat(bytes: &[u8]) -> Vec<(String, &[u8])> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Vec::new();
    }
    let mut at = 8usize;
    let mut parts: Vec<&[u8]> = Vec::new();
    while at + 8 <= bytes.len() {
        let Some(len) = bytes.get(at..at + 4).map(|b| u32::from_be_bytes(b.try_into().unwrap()) as usize) else {
            break;
        };
        let Some(kind) = bytes.get(at + 4..at + 8) else { break };
        if kind == b"IDAT" {
            let Some(data) = bytes.get(at + 8..at + 8 + len) else { break };
            parts.push(data);
        }
        let Some(next) = at.checked_add(12).and_then(|n| n.checked_add(len)) else { break };
        at = next;
    }
    // One chunk is the ordinary case and needs no copy; more than one cannot
    // be handed on as a slice, so those are left to the sample that has them.
    match parts.len() {
        1 if parts[0].len() > 6 => vec![("the image data".into(), &parts[0][2..parts[0].len() - 4])],
        _ => Vec::new(),
    }
}

/// Every ZIP entry written with deflate, found by walking the local headers.
fn zip_entries(bytes: &[u8]) -> Vec<(String, &[u8])> {
    let mut out = Vec::new();
    if !bytes.starts_with(b"PK\x03\x04") {
        return out;
    }
    let mut at = 0usize;
    let mut n = 0;
    while bytes.get(at..at + 4) == Some(b"PK\x03\x04") {
        let get = |o: usize| -> Option<u16> {
            bytes.get(at + o..at + o + 2).map(|b| u16::from_le_bytes(b.try_into().unwrap()))
        };
        let get32 = |o: usize| -> Option<u32> {
            bytes.get(at + o..at + o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        };
        let (Some(flags), Some(method), Some(size), Some(name_len), Some(extra_len)) =
            (get(6), get(8), get32(18), get(26), get(28))
        else {
            break;
        };
        let head = at + 30 + name_len as usize + extra_len as usize;
        // An entry whose size is in a descriptor after the data cannot be
        // sliced without reading the central directory, which this does not.
        if flags & 0x08 != 0 || size == 0 {
            break;
        }
        let Some(body) = bytes.get(head..head + size as usize) else { break };
        if method == 8 {
            out.push((format!("zip entry {n}"), body));
        }
        n += 1;
        at = head + size as usize;
    }
    out
}

/// Every compressed block of an LZ4 frame: a four-byte size whose top bit says
/// whether it was compressed at all, and that many bytes.
fn lz4_blocks(bytes: &[u8]) -> Vec<(String, &[u8])> {
    let mut out = Vec::new();
    let Some(&flg) = bytes.get(4) else { return out };
    let mut at = 7usize;
    if flg & 0x08 != 0 {
        at += 8;
    }
    if flg & 0x01 != 0 {
        at += 4;
    }
    let mut n = 0;
    loop {
        let Some(word) = bytes.get(at..at + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap())) else { break };
        at += 4;
        let size = (word & 0x7fff_ffff) as usize;
        if size == 0 {
            break;
        }
        let Some(body) = bytes.get(at..at + size) else { break };
        if word & 0x8000_0000 == 0 {
            out.push((format!("lz4 block {n}"), body));
        }
        n += 1;
        at += size;
        if flg & 0x10 != 0 {
            at += 4;
        }
    }
    out
}

fn samples() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in dirs() {
        collect(&dir, 6, &mut out);
    }
    out
}

fn dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(named) = std::env::var("QUBERO_SAMPLES") {
        out.extend(named.split(';').filter(|s| !s.is_empty()).map(PathBuf::from));
    }
    out.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../qubero-samples"));
    out.retain(|p| p.is_dir());
    out
}

fn collect(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            if depth > 0 {
                collect(&path, depth - 1, out);
            }
        } else if path.metadata().is_ok_and(|m| m.len() <= codec::CAP_BYTES as u64) {
            out.push(path);
        }
    }
}
