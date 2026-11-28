//! How long an index pass over a compressed file takes, and the most heap
//! it holds, when the run is read through a handle onto the file as the
//! evaluator reads a run of the file.
//!
//! `cargo run --release --example pass_memory -- <file> [tries] [codec] [copy]`
//!
//! The file is one run: a gzip member by default, or the codec named
//! (`deflate`, `zlib`, `gzip`, `zstd` or `xz`). Each try makes the pass,
//! steps it to its end with the evaluator's step sizes (256 KiB of output or
//! 2 Mibit of input), and prints the time and the peak of heap allocated
//! over what was allocated before the try, which counts the pass, anything
//! it copies and the index it makes, and what the index keeps for as long
//! as it is kept (`Seekable::kept_bytes`), or the length of a run held
//! whole. The file itself is allocated before.
//!
//! With `copy`, the pass reads a copy of the run made first, as the
//! evaluator's gather made one before the pass could read the run through
//! its handle: before part 7b of `docs/DESIGN-lazy-compressed-input.md` for
//! deflate and zstd, and before part 8 for xz, whose layout was then made in
//! one call of `decode_or_index_through`. A refusal is printed, not a
//! failure: a file of one xz block past 64 MiB is refused after its layout
//! is read.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use qubero_core::codec::lazy::{decode_or_index_through, Decoded, Indexing, Limits, RunInput, Stepped};
use qubero_core::codec::{Codec, Refusal};
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
    let path = args.get(1).expect("a compressed file");
    let tries: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5);
    let codec = match args.get(3).map(String::as_str) {
        None | Some("gzip") => Codec::Gzip,
        Some("deflate") => Codec::Deflate,
        Some("zlib") => Codec::Zlib,
        Some("zstd") => Codec::Zstd,
        Some("xz") => Codec::Xz,
        Some(other) => panic!("no codec {other}"),
    };
    let copy = args.get(4).is_some_and(|m| m == "copy");
    let bytes = Arc::new(std::fs::read(path).expect("reads"));
    let source: Arc<dyn Source + Send + Sync> = Arc::new(ArcSource(bytes.clone()));
    let mut best = (f64::MAX, usize::MAX);
    for _ in 0..tries {
        let base = NOW.load(Ordering::Relaxed);
        PEAK.store(base, Ordering::Relaxed);
        let t = Instant::now();
        let input = RunInput::new(source.clone(), 0, bytes.len() as u64);
        let (got, steps) = match copy {
            true => (decode_or_index_through(codec, Arc::new(bytes.to_vec()), input, Limits::default()), 1),
            false => stepped(Indexing::through(codec, input, Limits::default())),
        };
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let peak = PEAK.load(Ordering::Relaxed) - base;
        let (kind, held) = match &got {
            Ok(Decoded::Indexed(s)) => ("indexed".to_string(), s.kept_bytes() as usize),
            Ok(Decoded::Whole(b, _)) => ("whole".to_string(), b.len()),
            Err(why) => (format!("refused as {}", why.as_str()), 0),
        };
        drop(got);
        println!("{kind}, {steps} steps: {ms:.3} ms, peak {peak} bytes over the file, index keeps {held}");
        best = (best.0.min(ms), best.1.min(peak));
    }
    println!("fastest {:.3} ms, least peak {} bytes", best.0, best.1);
}

/// The pass as the evaluator makes it for a run of the file, reading the run
/// through the handle a window at a time, stepped to its end, and how many
/// steps that took.
fn stepped(made: Result<Indexing, Refusal>) -> (Result<Decoded, Refusal>, usize) {
    let mut pass = match made {
        Ok(pass) => pass,
        Err(why) => return (Err(why), 0),
    };
    let mut steps = 0usize;
    loop {
        steps += 1;
        match pass.step(256 << 10, (256 << 10) * 8) {
            Ok(Stepped::Done(d)) => return (Ok(d), steps),
            Ok(Stepped::Working) => {}
            Ok(Stepped::NeedsInput(m)) => panic!("an ArcSource missing {m:?}"),
            Err(why) => return (Err(why), steps),
        }
    }
}
