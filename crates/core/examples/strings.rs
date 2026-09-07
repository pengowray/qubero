//! Run the string scanner over files, the way the Strings view does.
//!
//! The view is the place to judge how the list reads; this is the place to
//! judge what is in it. A hundred files at once is how a rule that looks
//! principled turns out to report a page of gibberish on one format and
//! nothing at all on another, and neither shows up in a unit test written by
//! the person who wrote the rule.
//!
//! ```text
//! cargo run -p qubero-core --example strings -- <path>...      # a summary per file
//! cargo run -p qubero-core --example strings -- --list <path>  # every string found
//! cargo run -p qubero-core --example strings -- --odd <path>...# only the odd-looking ones
//! ```
//!
//! `--min N` sets the shortest run reported. `--enc` takes a comma-separated
//! list out of `ascii`, `utf16le` and `utf16be`.

use std::path::{Path, PathBuf};

use qubero_core::source::MemSource;
use qubero_core::stringscan::{scan, Enc, Hit, Opts};

fn main() {
    let mut args = std::env::args().skip(1);
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut opts = Opts::default();
    let mut mode = Mode::Summary;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--list" => mode = Mode::List,
            "--odd" => mode = Mode::Odd,
            "--stats" => mode = Mode::Stats,
            "--min" => opts.min_chars = args.next().and_then(|n| n.parse().ok()).unwrap_or(4),
            "--enc" => {
                let want = args.next().unwrap_or_default();
                let has = |name: &str| want.split(',').any(|e| e.trim() == name);
                opts.ascii = has("ascii");
                opts.utf16le = has("utf16le");
                opts.utf16be = has("utf16be");
            }
            _ => paths.push(PathBuf::from(arg)),
        }
    }
    if paths.is_empty() {
        eprintln!("usage: strings [--list|--odd] [--min N] [--enc a,b] <path>...");
        return;
    }
    let mut files = Vec::new();
    for p in &paths {
        gather(p, &mut files);
    }
    files.sort();
    let mut totals = Totals::default();
    for file in &files {
        report(file, opts, mode, &mut totals);
    }
    if files.len() > 1 {
        println!();
        println!("{} files, {} strings", files.len(), totals.all);
        println!(
            "  ascii {}  utf8 {}  utf16le {}  utf16be {}  prefixed {}  cut {}  wtf16 {}",
            totals.ascii, totals.utf8, totals.le, totals.be, totals.prefixed, totals.cut, totals.wtf
        );
        if mode == Mode::Stats {
            println!("  wide strings by character count (all / two thirds words):");
            for n in 0..32 {
                if totals.wide_by_len[n] > 0 {
                    println!("    {:>3}{} {:>6} {:>6}", n, if n == 31 { "+" } else { " " }, totals.wide_by_len[n], totals.wordish[n]);
                }
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Summary,
    List,
    /// Only the strings with something to say about them, which is where a
    /// rule that has gone wrong shows itself.
    Odd,
    /// How long the wide strings are and how much of them is words, which is
    /// what says whether a rule about either would pay for itself.
    Stats,
}

#[derive(Default)]
struct Totals {
    all: usize,
    ascii: usize,
    utf8: usize,
    le: usize,
    be: usize,
    prefixed: usize,
    cut: usize,
    wtf: usize,
    /// Wide strings by character count, capped at thirty-one.
    wide_by_len: [usize; 32],
    /// How many of those are two thirds words and ASCII punctuation.
    wordish: [usize; 32],
}

/// A character a string is made of, rather than one that fell out of a number:
/// anything printable ASCII, and any letter or digit in any script.
fn wordish(c: char) -> bool {
    (' '..='~').contains(&c) || c.is_alphanumeric()
}

fn gather(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_dir() {
        let Ok(entries) = std::fs::read_dir(path) else { return };
        for entry in entries.flatten() {
            gather(&entry.path(), out);
        }
    } else if path.is_file() {
        out.push(path.to_path_buf());
    }
}

fn report(path: &Path, opts: Opts, mode: Mode, totals: &mut Totals) {
    let Ok(bytes) = std::fs::read(path) else { return };
    let size = bytes.len();
    let src = MemSource(bytes);
    let start = std::time::Instant::now();
    let mut hits: Vec<Hit> = Vec::new();
    let mut from = 0u64;
    // The same loop the view runs, so what this prints is what the view holds.
    for _ in 0..100_000 {
        let step = scan(&src, from, 5000, opts);
        hits.extend(step.hits);
        if step.next <= from || step.next as usize >= size {
            break;
        }
        from = step.next;
    }
    let took = start.elapsed();
    let mut mine = Totals::default();
    for h in &hits {
        mine.all += 1;
        match h.enc {
            Enc::Ascii => mine.ascii += 1,
            Enc::Utf8 => mine.utf8 += 1,
            Enc::Utf16Le => mine.le += 1,
            Enc::Utf16Be => mine.be += 1,
        }
        if !h.prefix.is_empty() {
            mine.prefixed += 1;
        }
        if h.cut {
            mine.cut += 1;
        }
        if h.lone_surrogates {
            mine.wtf += 1;
        }
    }
    println!(
        "{}  {} bytes  {} strings  ({} ascii, {} utf8, {} le, {} be; {} prefixed, {} wtf16)  {:?}",
        path.display(),
        size,
        mine.all,
        mine.ascii,
        mine.utf8,
        mine.le,
        mine.be,
        mine.prefixed,
        mine.wtf,
        took
    );
    match mode {
        Mode::Summary => {}
        Mode::List => {
            for h in &hits {
                println!("{}", one(h));
            }
        }
        Mode::Odd => {
            for h in hits.iter().filter(|h| interesting(h)) {
                println!("{}", one(h));
            }
        }
        Mode::Stats => {}
    }
    if mode == Mode::Stats {
        for h in hits.iter().filter(|h| h.enc == Enc::Utf16Le || h.enc == Enc::Utf16Be) {
            let n = h.text.chars().count().min(31);
            totals.wide_by_len[n] += 1;
            let words = h.text.chars().filter(|c| wordish(*c)).count();
            if words * 3 >= h.text.chars().count() * 2 {
                totals.wordish[n] += 1;
            }
        }
    }
    totals.all += mine.all;
    totals.ascii += mine.ascii;
    totals.utf8 += mine.utf8;
    totals.le += mine.le;
    totals.be += mine.be;
    totals.prefixed += mine.prefixed;
    totals.cut += mine.cut;
    totals.wtf += mine.wtf;
}

/// A string with something about it worth a second look.
fn interesting(h: &Hit) -> bool {
    !h.prefix.is_empty() || h.lone_surrogates || h.cut || h.enc != Enc::Ascii
}

fn one(h: &Hit) -> String {
    let mut out = format!("  {:#010x} {:<10} {:?}", h.at, h.enc.name(), clip(&h.text, 60));
    for p in &h.prefix {
        let hex: Vec<String> = p.raw.iter().map(|b| format!("{b:02x}")).collect();
        out += &format!(
            "  [{} {} = {} {}{}]",
            p.kind.name(),
            hex.join(" "),
            p.value,
            p.counts.name(),
            if p.with_terminator { " +term" } else { "" }
        );
        if p.weak {
            out += "?";
        }
    }
    if let Some(t) = h.term {
        out += match t {
            qubero_core::stringscan::Term::Nul => "  [nul]",
            qubero_core::stringscan::Term::NulNul => "  [nulnul]",
        };
    }
    if h.lone_surrogates {
        out += "  [wtf16]";
    }
    if h.cut {
        out += "  [cut]";
    }
    out
}

fn clip(text: &str, n: usize) -> String {
    if text.chars().count() <= n {
        return text.to_string();
    }
    let head: String = text.chars().take(n).collect();
    format!("{head}\u{2026}")
}
