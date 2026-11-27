//! The kinds of the nodes a decoder's trace lays down, over the sample
//! collection, and two ignored tools that check the kinds cost nothing: a
//! dump of whole trees to diff before and after, and a timing.
//!
//! The collection is not in this repository: point `QUBERO_SAMPLES` at it, or
//! keep it beside the checkout as `qubero-samples`. With neither, this says so
//! and passes.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, NodeInfo};
use qubero_core::codec::tracekind::Role;
use qubero_core::formats;
use qubero_core::source::MemSource;

/// The template a file reads with, picked the way `samples_real` picks it.
fn template_for(path: &Path, bytes: &[u8]) -> Option<qubero_core::template::Template> {
    let head = &bytes[..bytes.len().min(0x9000)];
    let name = formats::sniff_named(head, bytes.len() as u64, &path.to_string_lossy())?;
    formats::template(name)
}

fn open(path: &Path) -> Option<(Document<MemSource>, Evaluator)> {
    let bytes = std::fs::read(path).ok()?;
    let template = template_for(path, &bytes)?;
    Some((Document::new(MemSource(bytes)), Evaluator::new(template)))
}

/// One line per node: everything the listing shows of it, and nothing that
/// is new.
fn line(n: &NodeInfo) -> String {
    format!(
        "{:?}\t{}\t{}\t{:?}\t{}\t{}\t{}\t{}\t{:?}\t{}",
        n.path, n.name, n.type_name, n.value, n.offset_bits, n.size_bits, n.child_count, n.lsb_first, n.unit, n.space
    )
}

/// Every node of the tree, depth first, up to `budget` of them.
fn dump(ev: &mut Evaluator, doc: &Document<MemSource>, path: &mut Vec<usize>, out: &mut Vec<String>, budget: usize) {
    if out.len() >= budget {
        return;
    }
    let Ok(n) = ev.node(doc, path) else {
        out.push(format!("{path:?}\tdoes not read"));
        return;
    };
    out.push(line(&n));
    for i in 0..n.child_count as usize {
        path.push(i);
        dump(ev, doc, path, out, budget);
        path.pop();
    }
}

/// Writes the trees of a few files with deflate streams and a JPEG scan to
/// `target-scratch/trees-<label>.txt`, `label` from `TRACE_DUMP_LABEL`, so a
/// change that should not show can be diffed against the build before it.
#[test]
#[ignore = "a tool, not a check"]
fn dump_decoded_trees() {
    let Some(root) = qubero_samples::root() else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let label = std::env::var("TRACE_DUMP_LABEL").unwrap_or_else(|_| "now".into());
    let files = [
        "png/png11-iccp.png",
        "gzip/gnu-gzip-9-with-name.gz",
        "gzip/eight-members-split-tar.tgz",
        "zip/streamed-data-descriptors.zip",
        "jpeg/libjpeg-turbo-testorig-baseline.jpg",
        "jpeg/pillow-cmyk-adobe-restart.jpg",
    ];
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target-scratch");
    std::fs::create_dir_all(&dir).unwrap();
    let mut f = std::fs::File::create(dir.join(format!("trees-{label}.txt"))).unwrap();
    for file in files {
        let (doc, mut ev) = open(&root.join(file)).unwrap_or_else(|| panic!("{file} does not open"));
        let mut out = Vec::new();
        dump(&mut ev, &doc, &mut Vec::new(), &mut out, 2_000_000);
        writeln!(f, "=== {file}: {} nodes", out.len()).unwrap();
        for l in out {
            writeln!(f, "{l}").unwrap();
        }
    }
}

/// The first `Decoded` node of a tree, searched breadth first a few levels
/// down.
fn first_decoded(ev: &mut Evaluator, doc: &Document<MemSource>) -> Option<Vec<usize>> {
    let mut level = vec![Vec::new()];
    for _ in 0..8 {
        let mut next = Vec::new();
        for path in level {
            let n = ev.node(doc, &path).ok()?;
            if n.decoded {
                return Some(path);
            }
            for i in 0..n.child_count.min(4096) as usize {
                let mut p = path.clone();
                p.push(i);
                next.push(p);
            }
        }
        level = next;
    }
    None
}

/// Lists every node under `path`, a whole level at a time, and counts them.
fn list_all(ev: &mut Evaluator, doc: &Document<MemSource>, path: &mut Vec<usize>) -> usize {
    let n = ev.node(doc, path).unwrap().child_count;
    let kids = ev.children(doc, path, 0, n).unwrap();
    let mut seen = kids.len();
    for (i, k) in kids.iter().enumerate() {
        if k.child_count > 0 {
            path.push(i);
            seen += list_all(ev, doc, path);
            path.pop();
        }
    }
    seen
}

/// How long a large deflate stream and a large JPEG scan take to open and to
/// list every node their traces lay down. Build it optimised and run it by
/// name; see the report this came with for the command.
#[test]
#[ignore = "a timing, not a pass"]
fn how_long_listing_a_trace_takes() {
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut rand = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let words = ["the", "quick", "brown", "fox", "jumps", "over", "lazy", "dog", "deflate", "stream", "bits", "of"];
    let mut data = Vec::with_capacity(3 << 20);
    while data.len() < 2 << 20 {
        let w = words[(rand() % words.len() as u64) as usize];
        data.extend_from_slice(w.as_bytes());
        data.push(if rand() % 11 == 0 { b'\n' } else { b' ' });
    }
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&data, 6);
    let rounds = 3;
    let start = Instant::now();
    for _ in 0..rounds {
        std::hint::black_box(qubero_core::codec::decode_traced(qubero_core::codec::Codec::Zlib, &packed).unwrap());
    }
    println!("deflate, 2 MiB out: decode and trace {:?} a round", start.elapsed() / rounds);
    let start = Instant::now();
    let mut nodes = 0;
    for _ in 0..rounds {
        let doc = Document::new(MemSource(packed.clone()));
        let mut ev = Evaluator::new(formats::builtin("zlib").unwrap());
        nodes = list_all(&mut ev, &doc, &mut vec![6, 1]);
    }
    println!("deflate, 2 MiB out: open and list {nodes} nodes {:?} a round", start.elapsed() / rounds);

    let jpeg = std::env::var("TRACE_TIMING_JPEG").map(PathBuf::from).unwrap_or_else(|_| {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target-scratch/large.jpg")
    });
    let Ok(bytes) = std::fs::read(&jpeg) else {
        println!("no JPEG at {} to time", jpeg.display());
        return;
    };
    let start = Instant::now();
    let mut nodes = 0;
    for _ in 0..rounds {
        let doc = Document::new(MemSource(bytes.clone()));
        let mut ev = Evaluator::new(template_for(&jpeg, &bytes).unwrap());
        let mut scan = first_decoded(&mut ev, &doc).expect("a scan");
        scan.push(1);
        ev.node(&doc, &scan).unwrap();
        nodes = list_all(&mut ev, &doc, &mut scan);
    }
    println!("jpeg {} bytes: open and list {nodes} nodes {:?} a round", bytes.len(), start.elapsed() / rounds);
    let start = Instant::now();
    for _ in 0..rounds {
        let doc = Document::new(MemSource(bytes.clone()));
        let mut ev = Evaluator::new(template_for(&jpeg, &bytes).unwrap());
        let mut scan = first_decoded(&mut ev, &doc).expect("a scan");
        scan.push(1);
        std::hint::black_box(ev.node(&doc, &scan).unwrap());
    }
    println!("jpeg {} bytes: open the scan alone {:?} a round", bytes.len(), start.elapsed() / rounds);
}

/// What the walk below saw: per family, how many nodes of each kind, and the
/// streams whose blocks have no kind at all, by the type of their stream.
#[derive(Default)]
struct Seen {
    kinds: std::collections::BTreeMap<&'static str, std::collections::BTreeMap<&'static str, usize>>,
    unnamed: std::collections::BTreeMap<String, usize>,
    streams: usize,
    failures: Vec<String>,
}

/// How many nodes of one stream's `blocks` row are walked, at most.
const STREAM_BUDGET: usize = 300_000;

/// Walks the nodes a template declared, a few levels and a few children
/// deep, checking none has a kind, and hands each stream's `blocks` row to
/// [`walk_blocks`].
fn walk(ev: &mut Evaluator, doc: &Document<MemSource>, path: &mut Vec<usize>, depth: usize, file: &str, seen: &mut Seen) {
    let Ok(n) = ev.node(doc, path) else { return };
    if let Some(k) = n.trace_kind {
        seen.failures.push(format!("{file} {path:?} {}: a node the template declared has kind {}", n.name, k.as_str()));
    }
    if n.decoded && n.refused.is_none() && n.child_count >= 2 {
        seen.streams += 1;
        path.push(1);
        if let Ok(blocks) = ev.node(doc, path) {
            match blocks.trace_kind {
                Some(k) => {
                    let prefix = k.as_str().split('.').next().unwrap();
                    *seen.kinds.entry(prefix).or_default().entry(k.as_str()).or_default() += 1;
                    let mut budget = STREAM_BUDGET;
                    walk_blocks(ev, doc, path, Some(k.family()), file, seen, &mut budget);
                }
                None => {
                    *seen.unnamed.entry(n.type_name.clone()).or_default() += 1;
                    let mut budget = 20_000;
                    walk_blocks(ev, doc, path, None, file, seen, &mut budget);
                }
            }
        }
        path.pop();
        path.push(0);
        walk(ev, doc, path, depth + 1, file, seen);
        path.pop();
        return;
    }
    if depth >= 12 {
        return;
    }
    let count = n.child_count as usize;
    let picks: Vec<usize> = if count <= 48 { (0..count).collect() } else { (0..32).chain(count - 16..count).collect() };
    for i in picks {
        path.push(i);
        walk(ev, doc, path, depth + 1, file, seen);
        path.pop();
    }
}

/// Walks every node under a stream's `blocks` row, up to `budget` of them.
/// With a family, every one has a kind of that family and from its table;
/// without one, none has a kind: a codec names all its nodes or none.
fn walk_blocks(
    ev: &mut Evaluator,
    doc: &Document<MemSource>,
    path: &mut Vec<usize>,
    family: Option<qubero_core::codec::tracekind::Family>,
    file: &str,
    seen: &mut Seen,
    budget: &mut usize,
) {
    let Ok(n) = ev.node(doc, path) else { return };
    let Ok(kids) = ev.children(doc, path, 0, n.child_count) else { return };
    for (i, k) in kids.iter().enumerate() {
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        match (family, k.trace_kind) {
            (Some(f), Some(kind)) if kind.family() == f && f.kinds().iter().any(|e| e.kind == kind.as_str()) => {
                let prefix = kind.as_str().split('.').next().unwrap();
                *seen.kinds.entry(prefix).or_default().entry(kind.as_str()).or_default() += 1;
            }
            (None, None) => {}
            (f, kind) => seen.failures.push(format!(
                "{file} {path:?}/{i} {}: kind {:?} under a stream of family {f:?}",
                k.name,
                kind.map(|k| k.as_str())
            )),
        }
        if k.child_count == 0 {
            continue;
        }
        path.push(i);
        match k.trace_kind.map(|t| t.role()) {
            // A step that is not one code reads as a record, `kind` and
            // `length` or `value`, the same record for every codec. Its fields
            // are the record's, named by the field, and have no kind.
            Some(r) if !matches!(r, Role::Blocks | Role::Block | Role::Codes | Role::Unit) => {
                for (j, f) in ev.children(doc, path, 0, k.child_count).unwrap_or_default().iter().enumerate() {
                    if let Some(t) = f.trace_kind {
                        seen.failures.push(format!("{file} {path:?}/{j} {}: a field of a step's record has kind {}", f.name, t.as_str()));
                    }
                }
            }
            _ => walk_blocks(ev, doc, path, family, file, seen, budget),
        }
        path.pop();
    }
}

/// The folders of the collection whose files hold deflate streams or JPEG
/// scans, or streams of other codecs inside them.
const FOLDERS: &[&str] = &[
    "png", "apng", "gzip", "zip", "compressed", "jpeg", "lha", "docx", "odt", "epub", "pico8", "picotron", "cdf", "parquet",
    "arrow", "godot", "root", "eml", "xlsx", "pptx", "avi", "tar",
];

/// Every node under a stream's `blocks` row has a kind from its codec's
/// table, for the codecs that name their nodes; none does for the rest; and
/// no node a template declared has one. Prints the kinds no sample reached.
#[test]
fn every_node_a_trace_lays_down_has_a_kind_from_its_codecs_table() {
    let Some(root) = qubero_samples::root() else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let mut files: Vec<PathBuf> = Vec::new();
    for folder in FOLDERS {
        let Ok(dir) = std::fs::read_dir(root.join(folder)) else { continue };
        files.extend(dir.flatten().map(|e| e.path()).filter(|p| p.is_file()));
    }
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/jpeg");
    if let Ok(dir) = std::fs::read_dir(&fixtures) {
        files.extend(dir.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "jpg")));
    }
    files.sort();
    let mut seen = Seen::default();
    for file in &files {
        let Some((doc, mut ev)) = open(file) else { continue };
        let name = file.strip_prefix(&root).unwrap_or(file).display().to_string();
        let started = Instant::now();
        let walked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut s = Seen::default();
            walk(&mut ev, &doc, &mut Vec::new(), 0, &name, &mut s);
            s
        }));
        match walked {
            Ok(s) => {
                seen.streams += s.streams;
                seen.failures.extend(s.failures);
                for (k, v) in s.unnamed {
                    *seen.unnamed.entry(k).or_default() += v;
                }
                for (f, m) in s.kinds {
                    for (k, v) in m {
                        *seen.kinds.entry(f).or_default().entry(k).or_default() += v;
                    }
                }
            }
            Err(_) => eprintln!("{name}: the walk panicked, which samples_real reports"),
        }
        if started.elapsed().as_secs() > 5 {
            eprintln!("{name}: {:?}", started.elapsed());
        }
    }
    eprintln!("{} files, {} streams", files.len(), seen.streams);
    for family in qubero_core::codec::tracekind::Family::ALL {
        let prefix = family.kinds()[0].kind.split('.').next().unwrap();
        let counts = seen.kinds.get(prefix).cloned().unwrap_or_default();
        eprintln!("{prefix}: {counts:?}");
        assert!(!counts.is_empty(), "no {prefix} stream in the samples was named");
        let never: Vec<_> = family.kinds().iter().map(|e| e.kind).filter(|k| !counts.contains_key(k)).collect();
        if !never.is_empty() {
            eprintln!("{prefix}: kinds in the table no sample has: {never:?}");
        }
    }
    eprintln!("streams whose nodes have no kind yet, by stream type: {:?}", seen.unnamed);
    assert!(seen.failures.is_empty(), "{} failures:\n{}", seen.failures.len(), seen.failures.join("\n"));
}
