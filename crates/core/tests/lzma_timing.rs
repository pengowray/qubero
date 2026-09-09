//! How long the LZMA samples take to open, so a change to the decoder can be
//! held against what it replaced.
//!
//! Ignored by default: a timing is not a pass or a fail, and a test reporting
//! one on every run is noise. Run it by name, in release, since a decoder
//! timed in a debug build says more about the build than about the decoder:
//!
//! ```text
//! QUBERO_SAMPLES=... cargo test -p qubero-core --release --test lzma_timing -- --ignored --nocapture
//! ```

use std::path::{Path, PathBuf};
use std::time::Instant;

use qubero_core::codec::{decode, Codec};
use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats;
use qubero_core::source::MemSource;

#[test]
#[ignore = "a timing, not a pass"]
fn how_long_the_lzma_samples_take() {
    // An lzip member, which is LZMA1 read to an end-of-stream marker.
    if let Some(path) = find("hello.lz") {
        let data = std::fs::read(&path).expect("reads");
        time("hello.lz", Codec::Lzip, &data, 500);
    }

    // Something long enough to say what the rate is rather than what starting
    // costs. The only encoder that ships here writes literals and no matches,
    // so this times the literal path; the samples above and below carry the
    // matches.
    let text: Vec<u8> = "the quick brown fox jumps over the lazy dog. ".repeat(20_000).into_bytes();
    let mut alone = Vec::new();
    lzma_rs::lzma_compress(&mut &text[..], &mut alone).expect("packs");
    let props = alone[0];
    let dict = u32::from_le_bytes(alone[1..5].try_into().expect("four bytes"));
    let codec = Codec::Lzma1 { props, dict_size: dict, unpacked: None };
    time("880 KB of literals", codec, &alone[13..], 50);

    // The 7z archives: one whose header is packed with LZMA1, and two whose
    // file streams are packed with LZMA2.
    for (name, path) in [("nested-dirs-header-lzma.7z", &[7usize, 2, 0][..])] {
        time_space(name, path, 200);
    }
    time_space("nested-dirs-solid.7z", &[7, 2, 0], 200);
    time_space("nested-dirs-nonsolid.7z", &[7, 2, 0], 200);
}

/// Open a run enough times to time it, and say how long one open took.
fn time(name: &str, codec: Codec, data: &[u8], rounds: u32) {
    let out = match decode(codec, data) {
        Ok(out) => out,
        Err(e) => return println!("{name}: refused, {e:?}"),
    };
    let start = Instant::now();
    for _ in 0..rounds {
        std::hint::black_box(decode(codec, data).expect("reads"));
    }
    let each = start.elapsed() / rounds;
    let rate = out.len() as f64 / (1 << 20) as f64 / each.as_secs_f64();
    println!("{name}: {each:?} for {} bytes out, {rate:.1} MiB/s", out.len());
}

/// The same for a stream a 7z archive holds, opened through the template the
/// way the interface opens it. The archive read around the stream is the same
/// work either way, so what differs between two runs of this is the decoder.
fn time_space(name: &str, path: &[usize], rounds: u32) {
    let Some(file) = find(name) else { return };
    let bytes = std::fs::read(&file).expect("reads");
    let open = |bytes: Vec<u8>| {
        let d = Document::new(MemSource(bytes));
        let mut e = Evaluator::new(formats::builtin("7z").expect("7z"));
        let id = e.open_space(&d, 0, path).expect("no error").expect("it opens");
        e.space(id).expect("it is there").len_bytes()
    };
    let n = open(bytes.clone());
    let start = Instant::now();
    for _ in 0..rounds {
        std::hint::black_box(open(bytes.clone()));
    }
    println!("{name}{path:?}: {:?} for {n} bytes out", start.elapsed() / rounds);
}

fn find(name: &str) -> Option<PathBuf> {
    for dir in dirs() {
        let mut found = None;
        collect(&dir, 3, name, &mut found);
        if found.is_some() {
            return found;
        }
    }
    None
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

fn collect(dir: &Path, depth: u32, name: &str, found: &mut Option<PathBuf>) {
    if found.is_some() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            if depth > 0 {
                collect(&path, depth - 1, name, found);
            }
        } else if path.file_name().is_some_and(|f| f == name) {
            *found = Some(path);
            return;
        }
    }
}
