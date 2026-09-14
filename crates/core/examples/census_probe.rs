//! How long the Diagram view's count of a file takes, and what it holds on to.
//!
//! `cargo run --release -p qubero-core --example census_probe -- <file>...`
//!
//! Each file is counted twice. Once in one go with no allowance, which is the
//! whole cost of the walk; and once in goes of `WORK_SLICE` elements, the way
//! the web app asks, which is what a reader waits through. `CENSUS_LIMIT` caps
//! the nodes walked, the way the app caps a large file.

use std::time::Instant;

use qubero_core::document::Document;
use qubero_core::eval::{CensusState, CensusWalk, Evaluator};
use qubero_core::source::MemSource;

/// What the wasm crate gives one go.
const WORK_SLICE: u64 = 5_000;

fn main() {
    let limit: usize = std::env::var("CENSUS_LIMIT").ok().and_then(|s| s.parse().ok()).unwrap_or(usize::MAX);
    for arg in std::env::args().skip(1) {
        let bytes = match std::fs::read(&arg) {
            Ok(b) => b,
            Err(e) => {
                println!("{arg}: {e}");
                continue;
            }
        };
        let head = &bytes[..bytes.len().min(qubero_core::formats::SNIFF_WINDOW)];
        let Some(name) = qubero_core::formats::sniff(head, bytes.len() as u64) else {
            println!("{arg}: no template");
            continue;
        };
        let Some(t) = qubero_core::formats::builtin(name) else {
            println!("{arg}: no built-in {name}");
            continue;
        };
        println!("{arg}: {} bytes, read as {name}", bytes.len());
        let doc = Document::new(MemSource(bytes));

        let mut ev = Evaluator::new(t.clone());
        let started = Instant::now();
        let c = ev.census(&doc, limit);
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        match c {
            Ok(c) => println!(
                "  one go:  {ms:>9.1} ms, {:>8} fields, {:>4} boxes, {:?}, {} nodes kept",
                c.walked,
                c.boxes.len(),
                c.state,
                ev.memo_len()
            ),
            Err(e) => println!("  one go:  {ms:>9.1} ms, {e:?}"),
        }

        let mut ev = Evaluator::new(t);
        ev.set_slice(Some(WORK_SLICE));
        let mut walk = CensusWalk::new(doc.len_bits());
        let started = Instant::now();
        let mut goes = 0u32;
        let mut slowest = 0.0f64;
        let mut peak = 0usize;
        loop {
            goes += 1;
            ev.begin_slice();
            let go = Instant::now();
            let got = ev.census_step(&doc, &mut walk, limit);
            slowest = slowest.max(go.elapsed().as_secs_f64() * 1000.0);
            peak = peak.max(ev.memo_len());
            match got {
                Ok(c) if c.state == CensusState::Working && goes < 1_000_000 => continue,
                Ok(c) => {
                    println!(
                        "  in goes: {:>9.1} ms, {:>8} fields, {:>4} boxes, {:?}, {goes} goes, slowest {slowest:.1} ms, peak {peak} nodes kept",
                        started.elapsed().as_secs_f64() * 1000.0,
                        c.walked,
                        c.boxes.len(),
                        c.state,
                    );
                    break;
                }
                Err(e) => {
                    println!("  in goes: {e:?} after {goes} goes");
                    break;
                }
            }
        }
    }
}
