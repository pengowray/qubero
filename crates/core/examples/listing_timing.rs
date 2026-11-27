//! How long listing a file takes when the file is read through a chunk store,
//! as the web holds it, and when it is held in memory. For measuring what
//! reading through the store costs a listing.
//!
//! `cargo run --release --example listing_timing -- <file> [nodes] [tries] [tab path]`
//!
//! The tree is listed depth first, the first 64 children of each node, until
//! `nodes` rows are listed (20,000 unless given). With a tab path, such as
//! `1` for a gzip's `decoded`, that stream is opened as a tab and its own
//! reading is listed the same way, when its bytes were recognised. Each try
//! is a fresh reading over a fresh document. The fastest and the median of
//! `tries` (7 unless given) are printed for each source.

use std::time::{Duration, Instant};

use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats;
use qubero_core::source::{ChunkStore, MemSource, Source};

const CHUNK: u64 = 64 << 10;

/// List the tree under `root` depth first, up to `budget` rows.
fn walk<S: Source>(ev: &mut Evaluator, d: &Document<S>, budget: usize) -> usize {
    let mut stack = vec![Vec::new()];
    let mut rows = 0;
    while let Some(path) = stack.pop() {
        let Ok(kids) = ev.children(d, &path, 0, 64) else { continue };
        rows += kids.len();
        if rows >= budget {
            break;
        }
        for k in kids.iter().rev() {
            if k.child_count > 0 {
                stack.push(k.path.clone());
            }
        }
    }
    rows
}

/// One try: a fresh reading of `d`, listed, and the tab at `tab` opened and
/// its reading listed.
fn once<S: Source>(d: &Document<S>, template: &str, budget: usize, tab: &Option<Vec<usize>>) -> (Duration, usize) {
    let started = Instant::now();
    let mut ev = Evaluator::new(formats::template(template).expect("a template"));
    let mut rows = walk(&mut ev, d, budget);
    if let Some(path) = tab {
        let id = ev.open_space(d, 0, path).expect("opens").expect("a space");
        let space = ev.space_mut(id).expect("the space");
        if let Some((tab_ev, tab_doc)) = space.reading() {
            rows += walk(tab_ev, tab_doc, budget);
        }
    }
    (started.elapsed(), rows)
}

fn report(what: &str, mut times: Vec<Duration>, rows: usize) {
    times.sort();
    let ms = |t: Duration| t.as_secs_f64() * 1000.0;
    println!("{what}: {rows} rows, fastest {:.1} ms, median {:.1} ms", ms(times[0]), ms(times[times.len() / 2]));
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: listing_timing <file> [nodes] [tries] [tab path]");
    let budget: usize = std::env::args().nth(2).and_then(|a| a.parse().ok()).unwrap_or(20_000);
    let tries: usize = std::env::args().nth(3).and_then(|a| a.parse().ok()).unwrap_or(7);
    let tab: Option<Vec<usize>> = std::env::args().nth(4).map(|a| a.split('.').map(|i| i.parse().expect("a path of numbers")).collect());
    let bytes = std::fs::read(&path).expect("reads");
    let template = formats::sniff(&bytes[..bytes.len().min(formats::SNIFF_WINDOW)], bytes.len() as u64).expect("sniffs");
    let chunks = bytes.len().div_ceil(CHUNK as usize);
    let stored = || {
        let mut store = ChunkStore::new(bytes.len() as u64, CHUNK, chunks.max(1));
        for (i, part) in bytes.chunks(CHUNK as usize).enumerate() {
            store.insert(i as u64, part.into());
        }
        Document::new(store)
    };
    let held = || Document::new(MemSource(bytes.clone()));
    println!("{path}: {template}, {} bytes", bytes.len());
    let (mut in_store, mut in_memory, mut rows) = (Vec::new(), Vec::new(), 0);
    // By turns, so that the load on the machine falls on both alike.
    for _ in 0..tries {
        let d = stored();
        let (t, n) = once(&d, template, budget, &tab);
        in_store.push(t);
        rows = n;
        let d = held();
        let (t, _) = once(&d, template, budget, &tab);
        in_memory.push(t);
    }
    report("chunk store", in_store, rows);
    report("in memory  ", in_memory, rows);
}
