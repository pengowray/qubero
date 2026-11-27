//! How long an index pass over a gzip takes, and the most heap it holds,
//! when the run is read through a handle onto the file as the evaluator
//! reads a run of the file.
//!
//! `cargo run --release --example pass_memory -- <file.gz> [tries]`
//!
//! The file is one gzip member. Each try makes the pass, steps it to its end
//! with the evaluator's step sizes (256 KiB of output or 2 Mibit of input),
//! and prints the time and the peak of heap allocated over what was
//! allocated before the try, which counts the pass, anything it copies and
//! the index it makes. The file itself is allocated before.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use qubero_core::codec::lazy::{Decoded, Indexing, Limits, RunInput, Stepped};
use qubero_core::codec::Codec;
use qubero_core::source::{ArcSource, Source};

struct Counting;

static NOW: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn grew(n: usize) {
    let now = NOW.fetch_add(n, Ordering::Relaxed) + n;
    PEAK.fetch_max(now, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        NOW.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            NOW.fetch_sub(layout.size(), Ordering::Relaxed);
            grew(new_size);
        }
        p
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("a gzip file");
    let tries: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5);
    let bytes = Arc::new(std::fs::read(path).expect("reads"));
    let source: Arc<dyn Source + Send + Sync> = Arc::new(ArcSource(bytes.clone()));
    let mut best = (f64::MAX, usize::MAX);
    for _ in 0..tries {
        let base = NOW.load(Ordering::Relaxed);
        PEAK.store(base, Ordering::Relaxed);
        let t = Instant::now();
        let input = RunInput::new(source.clone(), 0, bytes.len() as u64);
        let mut pass = make(input, &bytes);
        let mut steps = 0usize;
        let decoded = loop {
            steps += 1;
            match pass.step(256 << 10, (256 << 10) * 8) {
                Ok(Stepped::Done(d)) => break d,
                Ok(Stepped::Working) => {}
                Ok(Stepped::NeedsInput(m)) => panic!("an ArcSource missing {m:?}"),
                Err(why) => panic!("refused: {why:?}"),
            }
        };
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let peak = PEAK.load(Ordering::Relaxed) - base;
        let (kind, held) = match &decoded {
            Decoded::Indexed(s) => ("indexed", s.held_bytes()),
            Decoded::Whole(b, _) => ("whole", b.len()),
        };
        drop(decoded);
        println!("{kind}, {steps} steps: {ms:.1} ms, peak {peak} bytes over the file, index holds {held}");
        best = (best.0.min(ms), best.1.min(peak));
    }
    println!("fastest {:.1} ms, least peak {} bytes", best.0, best.1);
}

/// The pass as the evaluator makes it for a run of the file: reading the run
/// through the handle a window at a time. Before part 7b of
/// `docs/DESIGN-lazy-compressed-input.md`, the pass read a copy of the run
/// that the evaluator gathered first, `Arc::new(bytes.to_vec())`.
fn make(input: RunInput, _bytes: &Arc<Vec<u8>>) -> Indexing {
    Indexing::through(Codec::Gzip, input, Limits::default()).expect("starts")
}
