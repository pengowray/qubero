//! How long opening a joined stream as a document takes, how much heap the
//! opened stream holds, and how long reading all of it takes, when the file
//! is read through a chunk store, as the web holds it, and when it is held
//! in memory. For measuring what reading a joined stream's parts through
//! handles onto the store costs.
//!
//! `cargo run --release --example joined_timing -- <file> [path] [tries] [whole cap]`
//!
//! The stream at `path` (`1` unless given, which is a BGZF file's joined
//! stream) is opened, and then read from its first byte to its last in
//! reads of 64 KiB, as a hex view scrolled from end to end reads it. The
//! open is timed whole and in two: the walk to the stream's parts, which
//! reading the node at `path` makes, and the rest of opening it as a
//! document. The cap on a joined stream held whole is 64 MiB unless given, so a smaller
//! cap makes a smaller file's stream open a part at a time. Each try is a
//! fresh reading over a fresh document, and the sources take turns. The
//! fastest of `tries` (5 unless given) is printed for each source, with the
//! heap the opened stream held, which is the same in every try.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use qubero_core::codec::lazy::Limits;
use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats;
use qubero_core::source::{ChunkStore, MemSource, Source};

struct Counting;

static NOW: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            NOW.fetch_add(layout.size(), Ordering::Relaxed);
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
            NOW.fetch_add(new_size, Ordering::Relaxed);
        }
        p
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

const CHUNK: u64 = 64 << 10;

/// What one try came to.
struct Try {
    open: Duration,
    walk: Duration,
    read: Duration,
    held: usize,
    len: u64,
}

/// One try: a fresh reading of `d`, the stream at `path` opened and then
/// read end to end. The error is why the stream would not open.
fn once<S: Source>(d: &Document<S>, template: &str, path: &[usize], whole_cap: usize) -> Result<Try, String> {
    let before = NOW.load(Ordering::Relaxed);
    let started = Instant::now();
    let mut ev = Evaluator::new(formats::template(template).expect("a template"));
    ev.set_stream_limits(Limits::default(), whole_cap);
    ev.node(d, path).expect("the stream's node");
    let walk = started.elapsed();
    let Some(id) = ev.open_space(d, 0, path).expect("opens") else { return Err(format!("{:?}", ev.open_refusal(path))) };
    let open = started.elapsed();
    let held = NOW.load(Ordering::Relaxed).saturating_sub(before);
    let source = ev.space(id).expect("the space").source().clone();
    let len = source.len_bytes();
    let started = Instant::now();
    let mut buf = vec![0u8; CHUNK as usize];
    let mut sum = 0u64;
    for at in (0..len).step_by(CHUNK as usize) {
        let n = CHUNK.min(len - at) as usize;
        source.read(at, &mut buf[..n]).expect("reads");
        sum = sum.wrapping_add(buf[..n].iter().map(|&b| b as u64).sum::<u64>());
    }
    std::hint::black_box(sum);
    Ok(Try { open, walk, read: started.elapsed(), held, len })
}

fn report(what: &str, tries: &[Result<Try, String>]) {
    let tries: Vec<&Try> = match tries.iter().map(Result::as_ref).collect::<Result<Vec<_>, _>>() {
        Ok(tries) => tries,
        Err(why) => return println!("{what}: refused as {why}"),
    };
    let ms = |t: Duration| t.as_secs_f64() * 1000.0;
    let open = tries.iter().map(|t| t.open).min().unwrap();
    let walk = tries.iter().map(|t| t.walk).min().unwrap();
    let rest = tries.iter().map(|t| t.open - t.walk).min().unwrap();
    let read = tries.iter().map(|t| t.read).min().unwrap();
    let held = tries.iter().map(|t| t.held).max().unwrap();
    println!(
        "{what}: {} bytes, open fastest {:.1} ms (walk to the parts {:.1} ms, the rest {:.1} ms), read fastest {:.1} ms, held {held} bytes after opening",
        tries[0].len,
        ms(open),
        ms(walk),
        ms(rest),
        ms(read)
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let file = args.get(1).expect("usage: joined_timing <file> [path] [tries] [whole cap]");
    let path: Vec<usize> = args.get(2).map_or(vec![1], |a| a.split('.').map(|i| i.parse().expect("a path of numbers")).collect());
    let tries: usize = args.get(3).and_then(|a| a.parse().ok()).unwrap_or(5);
    let whole_cap: usize = args.get(4).and_then(|a| a.parse().ok()).unwrap_or(64 << 20);
    let bytes = std::fs::read(file).expect("reads");
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
    println!("{file}: {template}, {} bytes", bytes.len());
    let (mut in_store, mut in_memory) = (Vec::new(), Vec::new());
    for _ in 0..tries {
        let d = stored();
        in_store.push(once(&d, template, &path, whole_cap));
        let d = held();
        in_memory.push(once(&d, template, &path, whole_cap));
    }
    report("chunk store", &in_store);
    report("in memory  ", &in_memory);
}
