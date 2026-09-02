//! The one-stream files in the sample collection, which is not in this
//! repository: `hello.zz`, `hello.txt.xz`, `hello.txt.zst` and `hello.lz4`
//! under a directory `QUBERO_SAMPLES` names (several, separated by `;`), or
//! under `qubero-samples` beside the checkout. Skips when there is none.
//!
//! What a made-up file cannot show is that the stream is the one the real
//! compressor wrote. Each of these was written by its own tool over the same
//! sentence, so a decoder handed the wrong window, or the wrong bytes, or the
//! bytes in the wrong order, comes out with something other than that
//! sentence and says so here.

use std::path::{Path, PathBuf};

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

/// What every one of these files holds, whatever it was compressed with.
const SENTENCE: &str = "Qubero reads the shape of a compressed file without decompressing it.";

#[test]
fn a_file_that_is_one_stream_reads_as_what_the_stream_holds() {
    let names = [("hello.zz", "zlib"), ("hello.txt.xz", "xz"), ("hello.txt.zst", "zstd"), ("hello.lz4", "lz4")];
    let mut read = 0;
    for (name, template) in names {
        let Some(path) = find(name) else { continue };
        read += 1;
        let bytes = std::fs::read(&path).expect("reads");
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(formats::builtin(template).expect("a template of that name"));
        let found = text_inside(&d, &mut ev, &[], 8);
        let Some((at, text)) = found else {
            // A frame whose blocks were all stored, which is what an encoder
            // writes when compressing did not help. There is nothing to open,
            // and the text is bytes of the file: the sample collection has one
            // of these and it is not a failure.
            let text = String::from_utf8_lossy(ev_bytes(&d)).to_string();
            assert!(
                text.contains(SENTENCE),
                "{}: a {template} file with nothing compressed and nothing in the clear either",
                path.display()
            );
            eprintln!("--- {}: {template}, stored rather than compressed, so nothing to open", path.display());
            continue;
        };
        assert!(
            text.starts_with(SENTENCE),
            "{}: a {template} stream that opened into {text:?} rather than the sentence",
            path.display()
        );
        // The text is bytes of the stream, not of the file, and it starts at
        // the front of them.
        let node = ev.node(&d, &at).unwrap();
        assert_ne!(node.space, 0, "{}: text out of a stream, still in the file's space", path.display());
        assert_eq!(node.offset_bits, 0, "{}: the stream's text does not start at the front of it", path.display());
        // Nothing decoded is written back: there is nowhere in the file to put
        // it.
        assert!(!node.editable, "{}: a decoded field offered for editing", path.display());
        eprintln!("--- {}: {template}, {} bytes of text", path.display(), node.size_bits / 8);
    }
    if read == 0 {
        eprintln!("skipped: no one-stream sample in hand. Set QUBERO_SAMPLES to a directory holding one.");
    }
}

/// The whole file, for the one sample whose blocks were stored rather than
/// compressed.
fn ev_bytes(d: &Document<MemSource>) -> &[u8] {
    &d.source().0
}

/// The first text field anywhere under `at` that came out of a stream. Which
/// path reaches it depends on the format, so it is looked for rather than
/// spelled.
fn text_inside(
    d: &Document<MemSource>,
    ev: &mut Evaluator,
    at: &[usize],
    depth: u32,
) -> Option<(Vec<usize>, String)> {
    if depth == 0 {
        return None;
    }
    let node = ev.node(d, at).ok()?;
    if node.space != 0 {
        // An empty one is a stream of nothing, which an LZ4 frame ends with:
        // real, and not the text this is looking for.
        if let Value::Str(s) = &node.value {
            if !s.is_empty() {
                return Some((at.to_vec(), s.clone()));
            }
        }
    }
    for i in 0..node.child_count.min(24) as usize {
        if let Some(found) = text_inside(d, ev, &[at, &[i]].concat(), depth - 1) {
            return Some(found);
        }
    }
    None
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

fn collect(dir: &Path, depth: u32, name: &str, out: &mut Option<PathBuf>) {
    if out.is_some() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            if depth > 0 {
                collect(&path, depth - 1, name, out);
            }
        } else if path.file_name().is_some_and(|f| f == name) {
            *out = Some(path);
            return;
        }
    }
}
