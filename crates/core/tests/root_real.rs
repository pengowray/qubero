//! The ROOT files in the sample collection, which is not in this repository:
//! every `*.root` under a directory `QUBERO_SAMPLES` names (several, separated
//! by `;`), or under `qubero-samples` beside the checkout. Skips when there is
//! none.
//!
//! What a made-up file cannot show is that the compressed stream inside a
//! record is the stream the compressor really wrote. These five were written
//! by five releases of ROOT with four different compressors, so a block header
//! that named the algorithm and then handed on the wrong bytes shows up here:
//! an xz stream is placed by reading its footer backwards from the end of the
//! window the block gave it, and a window one byte out reads as nothing.

use std::path::{Path, PathBuf};

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats::root;
use qubero_core::source::MemSource;

/// The header's fields, and the record each of the last three places.
const DIRECTORY: [usize; 2] = [16, 0];
/// A record's own fields: twelve of key, and then whatever it holds.
const K_FIELDS: usize = 12;

#[test]
fn reads_the_real_files_and_what_is_inside_their_blocks() {
    let mut found = Vec::new();
    for dir in dirs() {
        collect(&dir, 3, &mut found);
    }
    if found.is_empty() {
        eprintln!("skipped: no ROOT file in hand. Set QUBERO_SAMPLES to a directory holding one.");
        return;
    }
    found.sort();
    let mut algorithms = Vec::new();
    let mut nested = 0;
    for path in &found {
        let (algorithm, dirs) = check(path);
        algorithms.push(algorithm);
        nested += dirs;
    }
    // Between them the samples cover every algorithm ROOT writes today, and a
    // file with no compression at all.
    eprintln!("{} files read, {nested} subdirectories walked: {algorithms:?}", found.len());
    assert!(nested > 0, "no sample here has a subdirectory, so nothing tested the walk");
}

/// Reads one file, and answers with the algorithm of the first compressed
/// block found in it and how many subdirectories the walk went into.
fn check(path: &Path) -> (Option<String>, usize) {
    let bytes = std::fs::read(path).expect("reads");
    let d = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(root());
    let say = |what: &str| format!("{}: {what}", path.display());

    // The header, and the top directory's record at the offset it gives.
    assert!(
        matches!(ev.node(&d, &[0]).unwrap().value, Value::Magic { ok: true, .. }),
        "{}",
        say("not a ROOT file")
    );
    let dir = ev.node(&d, &DIRECTORY).unwrap();
    assert_eq!(dir.offset_bits / 8, 100, "{}", say("the top record is not at fBEGIN"));

    // The directory's key list, and every key in it.
    let keys = [DIRECTORY.as_slice(), &[K_FIELDS + 2, 11, 0, K_FIELDS + 1]].concat();
    let n = ev.node(&d, &keys).unwrap().child_count;
    assert!(n > 0, "{}", say("a file with no keys in its top directory"));

    let mut dirs = 0;
    let algorithm = walk(&d, &mut ev, &keys, path, &mut dirs);
    eprintln!("--- {}: {n} keys, first block {algorithm:?}", path.display());
    (algorithm, dirs)
}

/// Every key of one key list, and every key of every directory under it. The
/// depth is the template's own: past [`MAX_DEPTH`] a directory key is a plain
/// record and has no key list to find, which is what stops this.
fn walk(
    d: &Document<MemSource>,
    ev: &mut Evaluator,
    keys: &[usize],
    path: &Path,
    dirs: &mut usize,
) -> Option<String> {
    let say = |what: &str| format!("{}: {what}", path.display());
    let n = ev.node(d, keys).unwrap().child_count;
    let mut algorithm = None;
    for i in 0..n as usize {
        let entry = [keys, &[i]].concat();
        // Every key names a class and an object, which is what makes the
        // listing readable.
        let class = ev.node(d, &[entry.as_slice(), &[9, 1]].concat()).unwrap().value;
        let Value::Str(class) = class else { panic!("{}", say("a key with no class name")) };
        assert!(!class.is_empty(), "{}", say("a key whose class name is empty"));
        let name = ev.node(d, &[entry.as_slice(), &[10, 1]].concat()).unwrap().value;
        assert!(matches!(name, Value::Str(_)), "{}", say("a key with no object name"));

        // What the key points at. A directory is walked into; anything else
        // is a record whose body is blocks or bytes.
        let body = [entry.as_slice(), &[K_FIELDS, 0, K_FIELDS]].concat();
        let body_node = ev.node(d, &body).unwrap();
        if class.starts_with("TDirectory") {
            // A directory record holds the sixty bytes of a directory, and
            // its key list is read the same way the top one was.
            assert_eq!(body_node.size_bits, 60 * 8, "{}", say("a directory record that is not sixty bytes"));
            *dirs += 1;
            let inner = [body.as_slice(), &[11, 0, K_FIELDS + 1]].concat();
            let m = ev.node(d, &inner).unwrap().child_count;
            eprintln!("--- {}: {class} {name:?} holds {m} keys", path.display());
            algorithm = algorithm.or_else(|| walk(d, ev, &inner, path, dirs));
            continue;
        }
        if class.contains("RNTuple") {
            algorithm = algorithm.or_else(|| anchor(d, ev, &body, path));
            continue;
        }
        for b in 0..body_node.child_count as usize {
            algorithm = algorithm.or_else(|| block(d, ev, &[body.as_slice(), &[b]].concat(), path));
        }
    }
    algorithm
}

/// One compressed block: the algorithm it names, and the stream inside it read
/// by that format's own template.
fn block(d: &Document<MemSource>, ev: &mut Evaluator, at: &[usize], path: &Path) -> Option<String> {
    let value = ev.node(d, &[at, &[0]].concat()).ok()?.value;
    let Value::Enum { name: Some(name), .. } = value else { return None };
    let stream = [at, &[4]].concat();
    let node = ev.node(d, &stream).unwrap();
    let compressed = ev.node(d, &[at, &[2]].concat()).unwrap().value.as_int().unwrap();
    assert_eq!(node.size_bits / 8, compressed as u64, "{}: the stream is not as long as the block says", path.display());
    // The first field of each of these is what says the template found the
    // stream it was handed: a magic number, or zlib's own two bytes.
    match name.as_str() {
        // An xz stream begins with its own magic; a zstd stream is a list of
        // frames, and the magic is the first field of the first of them.
        "xz" | "zstd" => {
            let to_magic: &[usize] = match name.as_str() {
                "zstd" => &[0, 0, 0],
                _ => &[0],
            };
            let magic = ev.node(d, &[stream.as_slice(), to_magic].concat()).unwrap().value;
            assert!(
                matches!(magic, Value::Magic { ok: true, .. }),
                "{}: an {name} block whose stream does not start with {name}",
                path.display()
            );
            // An xz stream is placed by reading its footer backwards from the
            // end of the window the block gave it, so a window a byte out
            // reads as nothing. The footer is the ninth field and is twelve
            // bytes wherever it lands.
            if name == "xz" {
                let footer = ev.node(d, &[stream.as_slice(), &[8]].concat()).unwrap();
                assert_eq!(footer.size_bits, 12 * 8, "{}: an xz footer that is not twelve bytes", path.display());
                let end = (footer.offset_bits + footer.size_bits) / 8;
                assert_eq!(
                    end,
                    (node.offset_bits + node.size_bits) / 8,
                    "{}: the xz stream does not end where the block does",
                    path.display()
                );
            }
        }
        "zlib" => {
            let method = ev.node(d, &[stream.as_slice(), &[1]].concat()).unwrap().value;
            assert_eq!(
                method,
                Value::Enum { raw: 8, name: Some("deflate".into()), hex: false },
                "{}: a ZL block that is not a zlib stream",
                path.display()
            );
        }
        "lz4" => {
            // ROOT's lz4 is a checksum and a bare block, so what proves it was
            // read right is the arithmetic: eight bytes of hash, and the rest
            // of the block after it.
            ev.node(d, &[stream.as_slice(), &[0]].concat()).unwrap();
            let raw = ev.node(d, &[stream.as_slice(), &[1]].concat()).unwrap();
            assert_eq!(
                raw.size_bits / 8,
                compressed as u64 - 8,
                "{}: an L4 block whose raw block is not what is left after the hash",
                path.display()
            );
        }
        _ => {}
    }
    Some(name)
}

/// The anchor of an RNTuple, and the two envelopes it points at. When the
/// anchor is itself compressed there are no numbers to read, only blocks, and
/// the envelopes cannot be placed: that is the answer, not a failure.
fn anchor(d: &Document<MemSource>, ev: &mut Evaluator, at: &[usize], path: &Path) -> Option<String> {
    let node = ev.node(d, at).unwrap();
    if node.size_bits != 78 * 8 || node.child_count != 17 {
        eprintln!("--- {}: the anchor is compressed, so its envelopes stay unplaced", path.display());
        return (0..node.child_count as usize).find_map(|b| block(d, ev, &[at, &[b]].concat(), path));
    }
    let field = |ev: &mut Evaluator, i: usize| ev.node(d, &[at, &[i]].concat()).unwrap().value.as_int().unwrap();
    assert_eq!(field(ev, 3), 1, "{}: an anchor whose epoch is not 1", path.display());
    for (i, envelope) in [(7usize, 15usize), (10, 16)] {
        let seek = field(ev, i);
        let nbytes = field(ev, i + 1);
        let placed = ev.node(d, &[at, &[envelope, 0]].concat()).unwrap();
        assert_eq!(placed.offset_bits / 8, seek as u64, "{}: an envelope is not where the anchor says", path.display());
        assert_eq!(placed.size_bits / 8, nbytes as u64, "{}: an envelope is not as long as the anchor says", path.display());
    }
    eprintln!("--- {}: an RNTuple anchor, both envelopes placed", path.display());
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

fn collect(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            if depth > 0 {
                collect(&path, depth - 1, out);
            }
        } else if path.extension().is_some_and(|x| x == "root") {
            out.push(path);
        }
    }
}
