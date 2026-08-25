//! A smoke test over real safetensors models. None is in the repository, and
//! the ones people have are far too big to read into memory, so this reads
//! from the file as the editor does and skips unless there is one to hand:
//! any `*.safetensors` in `web/public`, or under a directory `QUBERO_SAMPLES`
//! names (several, separated by `;`).

use std::cell::RefCell;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats::safetensors;
use qubero_core::source::{Missing, Source};

/// The file itself, read where it is asked for. A model is up to twenty
/// gigabytes, so nothing here is held.
struct FileSource {
    file: RefCell<File>,
    len: u64,
}

impl Source for FileSource {
    fn len_bytes(&self) -> u64 {
        self.len
    }
    fn read_bytes(&self, offset: u64, out: &mut [u8]) -> Vec<Missing> {
        let mut f = self.file.borrow_mut();
        if f.seek(SeekFrom::Start(offset)).is_err() || f.read_exact(out).is_err() {
            out.fill(0);
        }
        Vec::new()
    }
}

#[test]
fn reads_real_models_end_to_end() {
    let mut dirs = vec![PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../web/public"))];
    if let Ok(extra) = std::env::var("QUBERO_SAMPLES") {
        dirs.extend(extra.split(';').filter(|s| !s.is_empty()).map(PathBuf::from));
    }
    let mut found = Vec::new();
    for dir in &dirs {
        collect(dir, 3, &mut found);
    }
    if found.is_empty() {
        eprintln!("skipped: no safetensors model in {dirs:?}. Put one there, or set QUBERO_SAMPLES.");
        return;
    }
    found.sort();
    for path in found {
        check(&path);
    }
}

fn collect(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            if depth > 0 {
                collect(&path, depth - 1, out);
            }
        } else if path.extension().is_some_and(|x| x == "safetensors") {
            out.push(path);
        }
    }
}

fn check(path: &Path) {
    let file = File::open(path).expect("opens");
    let len = file.metadata().expect("has a size").len();
    eprintln!("--- {}: {len} bytes", path.display());
    let d = Document::new(FileSource { file: RefCell::new(file), len });
    let mut ev = Evaluator::new(safetensors());

    let header = ev.node(&d, &[1]).expect("the header parses");
    let n = header.child_count as usize;
    eprintln!("header {} bytes, {n} entries", header.size_bits / 8);

    // Every entry of the header places one run of weights, in the order the
    // header wrote them. Reading all of them is the test: a shape or a type
    // read wrongly anywhere leaves a run the wrong size, which shows up as a
    // gap before the next one or as an end that is not the end of the file.
    let mut at = header.offset_bits + header.size_bits;
    let mut kinds: Vec<(String, usize)> = Vec::new();
    let mut skipped = 0;
    let mut bytes = 0u64;
    for i in 0..n {
        let t = ev.node(&d, &[2, i]).unwrap_or_else(|e| panic!("tensor {i}: {e:?}"));
        if t.size_bits == 0 && t.name.starts_with("__") {
            // The file's own notes, which point at no weights.
            skipped += 1;
            continue;
        }
        assert_eq!(t.offset_bits, at, "{} starts {} bits after the one before it", t.name, t.offset_bits as i64 - at as i64);
        at += t.size_bits;
        bytes += t.size_bits / 8;
        match kinds.iter_mut().find(|(k, _)| *k == t.type_name) {
            Some((_, c)) => *c += 1,
            None => kinds.push((t.type_name.clone(), 1)),
        }
    }
    assert_eq!(at, d.len_bits(), "the last run of weights does not end at the end of the file");
    eprintln!("{} tensors, {skipped} not weights, {bytes} bytes of them", n - skipped, );
    eprintln!("  types: {kinds:?}");

    // The first tensor, in full: what the header says, and what was read.
    let first = (0..n).find(|i| ev.node(&d, &[2, *i]).expect("read").size_bits > 0).expect("a tensor");
    let t = ev.node(&d, &[2, first]).expect("read");
    let entry = ev.node(&d, &[1, first]).expect("read");
    let dtype = ev.node(&d, &[1, first, 0]).expect("read").value;
    let shape: Vec<i128> = (0..ev.node(&d, &[1, first, 1]).expect("read").child_count as usize)
        .map(|k| ev.node(&d, &[1, first, 1, k]).expect("read").value.as_int().unwrap_or(-1))
        .collect();
    eprintln!("  [{first}] {} {dtype:?} {shape:?} at 0x{:x}, {} of {}", entry.name, t.offset_bits / 8, t.child_count, t.type_name);
    assert_eq!(t.child_count as i128, shape.iter().product::<i128>().max(1));

    // And one weight from the middle of it, which is the read the editor does
    // when someone clicks there.
    let mid = t.child_count / 2;
    if t.child_count > 0 {
        let w = ev.node(&d, &[2, first, mid as usize]).expect("read");
        assert!(matches!(w.value, Value::Float(_) | Value::Int(_) | Value::UInt(_)), "{:?}", w.value);
        eprintln!("  weight {mid} of it: {:?}", w.value);
    }
}
