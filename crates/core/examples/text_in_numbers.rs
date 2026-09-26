//! How much text the report finds inside runs of numbers, over many files at
//! several minimum lengths: what `TEXT_MIN_CHARS` was chosen with.
//!
//! ```text
//! cargo run -p qubero-core --example text_in_numbers -- [--min 8,12,16,24,32] [path...]
//! ```
//!
//! With no path, the whole sample collection. A file is walked once at the
//! smallest minimum, and again at each larger one only if that found
//! anything, since a longer minimum never finds more. Each line is a file
//! with text found, and each column a minimum: how many runs of text were
//! found inside runs of numbers. Minimums are in characters, which for ASCII
//! is bytes. `--detail` lists the runs of numbers and their first texts.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, ReportWalk, TextInNumbers};
use qubero_core::formats;
use qubero_core::source::MemSource;

/// How long one walk of one file may take before the file is left out.
const PATIENCE: Duration = Duration::from_secs(60);

fn main() {
    let mut args = std::env::args().skip(1);
    let mut mins: Vec<usize> = vec![8, 12, 16, 24, 32];
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut detail = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--min" => mins = args.next().unwrap_or_default().split(',').filter_map(|n| n.trim().parse().ok()).collect(),
            "--detail" => detail = true,
            _ => paths.push(PathBuf::from(a)),
        }
    }
    mins.sort_unstable();
    if paths.is_empty() {
        paths.extend(qubero_samples::root());
    }
    let mut files = Vec::new();
    for p in &paths {
        gather(p, &mut files);
    }
    files.sort();
    let mut read = 0;
    let mut numeric = 0;
    let mut slow = Vec::new();
    // Files with any hit, and text runs in all, per minimum.
    let mut with = vec![(0usize, 0u64); mins.len()];
    print!("{:<72} {:<12}", "file", "template");
    for m in &mins {
        print!(" {:>6}", format!("{m}"));
    }
    println!();
    for file in &files {
        let Ok(bytes) = std::fs::read(file) else { continue };
        let Some(name) = formats::sniff(&bytes[..bytes.len().min(formats::SNIFF_WINDOW)], bytes.len() as u64) else { continue };
        let shown = file.strip_prefix(paths.first().map(|p| p.as_path()).unwrap_or(Path::new(""))).unwrap_or(file).display().to_string();
        let mut row: Vec<u64> = Vec::new();
        let mut first: Option<TextInNumbers> = None;
        for (i, &m) in mins.iter().enumerate() {
            if i > 0 && row.last() == Some(&0) {
                row.push(0);
                continue;
            }
            let Some(t) = walk(&bytes, name, m) else {
                slow.push(shown.clone());
                break;
            };
            if i == 0 {
                read += 1;
                if t.numeric_bytes > 0 {
                    numeric += 1;
                }
            }
            row.push(t.runs.iter().map(|r| r.texts).sum());
            if first.is_none() {
                first = Some(t);
            }
        }
        if row.len() < mins.len() || row[0] == 0 {
            continue;
        }
        for (i, n) in row.iter().enumerate() {
            with[i].0 += (*n > 0) as usize;
            with[i].1 += n;
        }
        print!("{:<72} {:<12}", shown, name);
        for n in &row {
            print!(" {n:>6}");
        }
        println!();
        if detail {
            if let Some(t) = first {
                for r in &t.runs {
                    println!(
                        "    {} ({}, {} bits each) at {:#x}, {} bytes: {} text runs, {} bytes",
                        r.name,
                        r.what,
                        r.element_bits,
                        r.offset_bits / 8,
                        r.size_bits / 8,
                        r.texts,
                        r.text_bytes
                    );
                    for x in &r.first {
                        println!("      {:#x} {} {:?}", x.offset_bits / 8, x.encoding, x.text);
                    }
                }
            }
        }
    }
    println!();
    println!("{} files read with a template, {} with runs of numbers, {} left out as too slow", read, numeric, slow.len());
    print!("files with text in runs of numbers, and runs of text in all:");
    for (i, m) in mins.iter().enumerate() {
        print!("  {m}: {} files, {} runs", with[i].0, with[i].1);
    }
    println!();
    for s in &slow {
        println!("  too slow: {s}");
    }
}

/// The report's walk over one file, to the end, with text counted from `min`
/// characters. None if it took too long.
fn walk(bytes: &[u8], name: &str, min: usize) -> Option<TextInNumbers> {
    let t = formats::template(name)?;
    let len = bytes.len() as u64 * 8;
    let doc = Document::new(MemSource(bytes.to_vec()));
    let mut ev = Evaluator::new(t);
    ev.set_slice(Some(50_000));
    let mut w = ReportWalk::new(len).with_text_min(min);
    let started = Instant::now();
    loop {
        ev.begin_slice();
        if ev.report_step(&doc, &mut w).is_err() {
            break;
        }
        if w.done() {
            break;
        }
        if started.elapsed() > PATIENCE {
            return None;
        }
    }
    Some(w.text_in_numbers())
}

fn gather(p: &Path, out: &mut Vec<PathBuf>) {
    if p.is_dir() {
        let Ok(entries) = std::fs::read_dir(p) else { return };
        for e in entries.flatten() {
            gather(&e.path(), out);
        }
    } else if p.is_file() {
        out.push(p.to_path_buf());
    }
}
