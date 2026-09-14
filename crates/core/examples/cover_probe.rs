//! How much of a file the annotation column names, against how much the tree
//! covers: `cargo run --release --example cover_probe -- <file-or-dir>...`
//!
//! The tree is walked from the root the way the field tree opens it, and every
//! leaf in the file's own address space adds its extent. A decoded stream adds
//! its run as well, and is walked into for anything inside it that points back
//! out at the file. Overlaps count once. The spans are asked twice: once for
//! the whole file, and once in windows the size of a screenful, which is what
//! the hex view does. Where the tree names bytes the spans do not, something
//! placed those bytes that the spans never found.
//!
//! Three settings, as environment variables: `COVER_MAX_BYTES` passes over
//! larger files, `COVER_DIFF` lists the stretches the tree names and the spans
//! do not, and `COVER_TIMING` is for a large file (see `timing`).

use std::fs;
use std::path::Path;
use std::time::Instant;

use qubero_core::document::Document;
use qubero_core::eval::{EvalError, Evaluator};
use qubero_core::source::MemSource;

/// A screenful of the hex view, roughly.
const WINDOW: u64 = 2048;

fn template_of(bytes: &[u8]) -> Option<&'static str> {
    let head = &bytes[..bytes.len().min(qubero_core::formats::SNIFF_WINDOW)];
    qubero_core::formats::sniff(head, bytes.len() as u64)
}

/// How many bits the union of these stretches covers.
fn union(v: Vec<(u64, u64)>) -> u64 {
    merged(v).iter().map(|(a, b)| b - a).sum()
}

/// The same stretches, sorted, with the overlapping ones joined.
fn merged(mut v: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
    v.sort();
    let mut out: Vec<(u64, u64)> = Vec::new();
    for (a, b) in v {
        match out.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// The stretches of `named` that `spans` leaves out, in bytes, for finding
/// what is behind a difference. Asked for with `COVER_DIFF=1`.
fn missing(tree: Vec<(u64, u64)>, spans: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
    let spans = merged(spans);
    let mut out = Vec::new();
    for (a, b) in merged(tree) {
        let mut at = a;
        for &(c, d) in spans.iter().filter(|(c, d)| *d > a && *c < b) {
            if c > at {
                out.push((at / 8, c / 8));
            }
            at = at.max(d);
        }
        if at < b {
            out.push((at / 8, b / 8));
        }
    }
    out
}

fn tree(ev: &mut Evaluator, doc: &Document<MemSource>, path: &mut Vec<usize>, out: &mut Vec<(u64, u64)>, opened: &mut usize) {
    *opened += 1;
    if *opened > 3_000_000 {
        return;
    }
    let Ok(n) = ev.node(doc, path) else { return };
    // Not a composite with nothing in it: a list the file gave no elements
    // stretches over bytes it says nothing about, and the spans are right to
    // call those a gap.
    if n.space == 0 && n.size_bits > 0 && (n.decoded || !n.composite) {
        out.push((n.offset_bits, n.offset_bits + n.size_bits));
    }
    if !n.composite {
        return;
    }
    for i in 0..n.child_count as usize {
        path.push(i);
        tree(ev, doc, path, out, opened);
        path.pop();
    }
}

/// Whether this run is for timing a large file rather than measuring what is
/// named: `COVER_TIMING=1` skips the tree walk, lets the spans take as many
/// goes as they need, and times a window at the end of the file asked of a
/// fresh evaluator, which is a jump to the end of a file just opened.
fn timing() -> bool {
    std::env::var_os("COVER_TIMING").is_some()
}

/// The named stretches over one window, clipped to it, and how many goes of
/// 5,000 steps they took.
fn spans(ev: &mut Evaluator, doc: &Document<MemSource>, from: u64, to: u64) -> Result<(Vec<(u64, u64)>, usize), String> {
    let most = if timing() { 1_000_000 } else { 200 };
    for go in 1..=most {
        ev.begin_slice();
        match ev.spans(doc, from, to, 1_000_000) {
            Ok(v) => {
                let named = v
                    .iter()
                    .filter(|s| !s.gap)
                    .map(|s| (s.offset_bits.max(from), (s.offset_bits + s.size_bits).min(to)))
                    .filter(|(a, b)| b > a)
                    .collect();
                return Ok((named, go));
            }
            Err(EvalError::Busy { .. }) => {}
            Err(e) => return Err(format!("{e:?}")),
        }
    }
    Err("never settled".into())
}

fn probe(path: &Path, rel: &str) {
    // A cap on what is read at all, for sweeping the whole collection: the
    // tree walk opens every leaf, and a forty-megabyte archive is minutes.
    let cap: u64 = std::env::var("COVER_MAX_BYTES").ok().and_then(|s| s.parse().ok()).unwrap_or(u64::MAX);
    if fs::metadata(path).map_or(true, |m| m.len() > cap) {
        return;
    }
    let Ok(bytes) = fs::read(path) else { return };
    let len = bytes.len() as u64;
    let Some(name) = template_of(&bytes) else {
        println!("{rel}\t-");
        return;
    };
    let t = qubero_core::formats::template(name).unwrap();
    let doc = Document::new(MemSource(bytes));

    let mut leaves = Vec::new();
    if !timing() {
        let mut ev = Evaluator::new(t.clone());
        let mut opened = 0;
        tree(&mut ev, &doc, &mut Vec::new(), &mut leaves, &mut opened);
    }
    let tree_bytes = union(leaves.clone()) / 8;

    if timing() {
        for (which, from) in [("middle", len / 2), ("last", len.saturating_sub(WINDOW))] {
            let mut ev = Evaluator::new(t.clone());
            ev.set_slice(Some(5_000));
            let clock = Instant::now();
            match spans(&mut ev, &doc, from * 8, (from + WINDOW).min(len) * 8) {
                Ok((v, goes)) => println!(
                    "{rel}\t{which} window, fresh: {} bytes named in {} ms, {goes} goes",
                    union(v) / 8,
                    clock.elapsed().as_millis()
                ),
                Err(e) => println!("{rel}\t{which} window, fresh: {e}"),
            }
        }
        if std::env::var("COVER_TIMING").is_ok_and(|v| v == "last") {
            return;
        }
    }

    let mut ev = Evaluator::new(t.clone());
    ev.set_slice(Some(5_000));
    let clock = Instant::now();
    let whole = match spans(&mut ev, &doc, 0, len * 8) {
        Ok((v, goes)) => {
            let ms = clock.elapsed().as_millis();
            if std::env::var_os("COVER_DIFF").is_some() {
                for (a, b) in missing(leaves, v.clone()).into_iter().take(20) {
                    println!("  not named: {a:#x}..{b:#x} ({} bytes)", b - a);
                }
            }
            format!("{}\t{ms} ms {goes} goes", union(v) / 8)
        }
        Err(e) => format!("{e}\t{} ms", clock.elapsed().as_millis()),
    };

    // A fresh evaluator, as a file just opened has, so the first window pays
    // for whatever the spans have to find before they can name anything.
    let mut ev = Evaluator::new(t);
    ev.set_slice(Some(5_000));
    let mut windowed = Vec::new();
    let mut trouble = None;
    let mut slowest = 0;
    let mut most_goes = 0;
    let mut at = 0;
    while at < len {
        let clock = Instant::now();
        match spans(&mut ev, &doc, at * 8, (at + WINDOW).min(len) * 8) {
            Ok((v, goes)) => {
                windowed.extend(v);
                most_goes = most_goes.max(goes);
            }
            Err(e) => trouble = Some(e),
        }
        let ms = clock.elapsed().as_millis();
        if timing() && ms >= 200 {
            println!("{rel}\twindow at {at:#x}: {ms} ms");
        }
        slowest = slowest.max(ms);
        at += WINDOW;
    }
    let windowed = match trouble {
        Some(e) => e,
        None => (union(windowed) / 8).to_string(),
    };
    println!(
        "{rel}\t{name}\t{len}\ttree {tree_bytes}\tspans {whole}\twindows {windowed}\tslowest window {slowest} ms {most_goes} goes"
    );
}

fn sweep(root: &Path, dir: &Path, out: &mut Vec<(String, std::path::PathBuf)>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            sweep(root, &path, out);
            continue;
        }
        let rel = path.strip_prefix(root.parent().unwrap_or(root)).unwrap_or(&path).display().to_string().replace('\\', "/");
        out.push((rel, path));
    }
}

fn main() {
    for arg in std::env::args().skip(1) {
        let path = Path::new(&arg);
        let mut files = Vec::new();
        if path.is_dir() {
            sweep(path, path, &mut files);
        } else {
            files.push((arg.clone(), path.to_path_buf()));
        }
        files.sort();
        for (rel, p) in files {
            probe(&p, &rel);
        }
    }
}
