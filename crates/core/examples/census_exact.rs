//! Whether the Diagram view's count of a file is the count walking every
//! element gives.
//!
//! `cargo run --release -p qubero-core --example census_exact -- <file or folder>...`
//!
//! Each file is counted twice: the way the view counts, walking the first
//! element of a run whose elements the template says are all the same shape
//! and counting it once per element, and element by element. The two should
//! agree on every box and every row. A file whose element-by-element count
//! passes `CENSUS_LIMIT` fields (a million unless set) is skipped rather than
//! walked for minutes.
//!
//! One line per file: `same` or `DIFFERENT`, the fields each count walked, the
//! boxes and rows, and the sum of every box's count, which is a number to
//! compare between two builds. Each difference is printed under its file.

use std::path::{Path, PathBuf};

use qubero_core::document::Document;
use qubero_core::eval::{Census, CensusState, CensusWalk, Evaluator};
use qubero_core::formats;
use qubero_core::source::MemSource;

fn main() {
    // A debug build's frames are several times a release build's, and a deep
    // file reads deep. See `DEEPEST_PATH` in `eval`.
    let t = std::thread::Builder::new().stack_size(256 << 20).spawn(sweep).expect("a thread to count on");
    t.join().expect("the count finishes");
}

fn sweep() {
    let limit: usize = std::env::var("CENSUS_LIMIT").ok().and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let mut files = Vec::new();
    for arg in std::env::args().skip(1) {
        collect(Path::new(&arg), &mut files);
    }
    files.sort();
    let (mut same, mut different, mut skipped) = (0, 0, 0);
    for path in files {
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let head = &bytes[..bytes.len().min(formats::SNIFF_WINDOW)];
        let Some(name) = formats::sniff(head, bytes.len() as u64) else { continue };
        let Some(template) = formats::template(name) else { continue };
        let doc = Document::new(MemSource(bytes));
        let fast = count(&doc, &template, false, limit);
        let slow = count(&doc, &template, true, limit);
        let shown = path.display();
        let (fast, slow) = match (fast, slow) {
            (Ok(f), Ok(s)) if s.state == CensusState::Done && f.state == CensusState::Done => (f, s),
            (Ok(_), Ok(s)) => {
                skipped += 1;
                println!("skipped   {shown} as {name}: element by element stopped {:?} after {} fields", s.state, s.walked);
                continue;
            }
            (f, s) => {
                skipped += 1;
                println!("skipped   {shown} as {name}: {:?} / {:?}", f.err(), s.err());
                continue;
            }
        };
        let total: u64 = fast.boxes.iter().map(|b| b.count).sum();
        let diffs = differences(&fast, &slow);
        let word = if diffs.is_empty() { "same     " } else { "DIFFERENT" };
        println!(
            "{word} {shown} as {name}: {} fields multiplied, {} walked, {} boxes, {} rows, box counts sum to {total}",
            fast.walked,
            slow.walked,
            fast.boxes.len(),
            fast.rows.len()
        );
        for d in &diffs {
            println!("            {d}");
        }
        if diffs.is_empty() {
            same += 1;
        } else {
            different += 1;
        }
    }
    println!("{same} the same, {different} different, {skipped} skipped");
}

fn count(doc: &Document<MemSource>, template: &qubero_core::template::Template, every: bool, limit: usize) -> Result<Census, String> {
    let mut ev = Evaluator::new(template.clone());
    let mut walk = CensusWalk::new(doc.len_bits());
    if every {
        walk = walk.element_by_element();
    }
    ev.census_step(doc, &mut walk, limit).map_err(|e| format!("{e:?}"))
}

/// Every box and row the two counts disagree on, as a line each.
fn differences(fast: &Census, slow: &Census) -> Vec<String> {
    let mut out = Vec::new();
    let boxes = |c: &Census| c.boxes.iter().map(|b| (b.key.clone(), (b.count, b.first_path.clone(), b.space))).collect::<std::collections::BTreeMap<_, _>>();
    let rows = |c: &Census| c.rows.iter().map(|r| ((r.key.clone(), r.row), (r.count, r.first_path.clone(), r.space))).collect::<std::collections::BTreeMap<_, _>>();
    let (fb, sb) = (boxes(fast), boxes(slow));
    for key in fb.keys().chain(sb.keys()).collect::<std::collections::BTreeSet<_>>() {
        let (f, s) = (fb.get(key), sb.get(key));
        if f != s {
            out.push(format!("box {}: multiplied {f:?}, walked {s:?}", short(key)));
        }
    }
    let (fr, sr) = (rows(fast), rows(slow));
    for key in fr.keys().chain(sr.keys()).collect::<std::collections::BTreeSet<_>>() {
        let (f, s) = (fr.get(key), sr.get(key));
        if f != s {
            out.push(format!("row {} of {}: multiplied {f:?}, walked {s:?}", key.1, short(&key.0)));
        }
    }
    out
}

/// A box's key is the type written out, which can be pages long.
fn short(key: &str) -> String {
    let flat: String = key.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(80) {
        Some((i, _)) => format!("{}...", &flat[..i]),
        None => flat,
    }
}

fn collect(at: &Path, out: &mut Vec<PathBuf>) {
    if at.is_file() {
        out.push(at.to_path_buf());
        return;
    }
    let Ok(entries) = std::fs::read_dir(at) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "tools" || n == ".git" || n == "hexdump" || n == "does-not-read") {
                continue;
            }
            collect(&path, out);
        } else if path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
            continue;
        } else if path.extension().is_none_or(|e| e != "md" && e != "tsv" && e != "py" && e != "dis") {
            out.push(path);
        }
    }
}
