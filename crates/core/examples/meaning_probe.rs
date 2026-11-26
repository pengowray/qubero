//! How many bytes of a file the hex view leaves without a meaning:
//! `cargo run --example meaning_probe -- <file-or-dir>...`
//!
//! The annotation column's spans are asked for the whole file, and each byte
//! is put in one of four piles:
//!
//! - **named**: a field with a value, or a run of them.
//! - **gap**: no field covers it.
//! - **opaque**: a field that is only bytes, so the column can name it but not
//!   say what is in it.
//! - **one row**: a structure drawn as one chip over more than `WIDE` bytes,
//!   which hides every field inside it from the column.
//!
//! A directory sweeps every file under it, one line each. `MEANING_LIST=1`
//! also lists the stretches in the last three piles, largest first, so a
//! template can be worked through from the top.

use std::fs;
use std::path::Path;

use qubero_core::document::Document;
use qubero_core::eval::{EvalError, Evaluator, Span};
use qubero_core::source::MemSource;

/// A one-row chip wider than this many bytes is counted as hiding fields.
const WIDE: u64 = 32;

fn template_of(bytes: &[u8]) -> Option<&'static str> {
    let head = &bytes[..bytes.len().min(qubero_core::formats::SNIFF_WINDOW)];
    qubero_core::formats::sniff(head, bytes.len() as u64)
}

fn spans_of(bytes: Vec<u8>, name: &str) -> Result<Vec<Span>, String> {
    let len = bytes.len() as u64;
    let d = Document::new(MemSource(bytes));
    let Some(t) = qubero_core::formats::template(name) else { return Err("no template".into()) };
    let mut ev = Evaluator::new(t);
    ev.set_slice(Some(5_000));
    for _ in 0..400 {
        ev.begin_slice();
        match ev.spans(&d, 0, len * 8, 200_000) {
            Ok(v) => return Ok(v),
            Err(EvalError::Busy { .. }) => {}
            Err(e) => return Err(format!("{e:?}")),
        }
    }
    Err("never settled".into())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Pile {
    Named,
    Gap,
    Opaque,
    OneRow,
}

fn pile(s: &Span) -> Pile {
    let bytes = s.size_bits / 8;
    if s.gap {
        Pile::Gap
    } else if s.line.is_some() && bytes > WIDE {
        Pile::OneRow
    } else if s.count == 0 && !s.opens && (s.type_name.starts_with("bytes") || s.type_name == "unread") {
        Pile::Opaque
    } else {
        Pile::Named
    }
}

fn describe(s: &Span) -> String {
    let mut where_ = s.trail.join(" › ");
    if !where_.is_empty() {
        where_.push_str(" › ");
    }
    format!("{where_}{} ({})", s.name, s.type_name)
}

fn one(rel: &str, bytes: Vec<u8>, list: bool, out: &mut Vec<String>) {
    let len = bytes.len() as u64;
    let Some(name) = template_of(&bytes) else {
        out.push(format!("{rel}\t-\t{len}\tno template"));
        return;
    };
    let spans = match spans_of(bytes, name) {
        Ok(v) => v,
        Err(e) => {
            out.push(format!("{rel}\t{name}\t{len}\terr {e}"));
            return;
        }
    };
    let mut sum = [0u64; 4];
    let mut worst: Vec<(u64, u64, Pile, String)> = Vec::new();
    let mut covered = 0u64;
    for s in &spans {
        let b = s.size_bits / 8;
        covered += b;
        let p = pile(s);
        sum[p as usize] += b;
        if p != Pile::Named {
            worst.push((b, s.offset_bits / 8, p, describe(s)));
        }
    }
    // Bytes no span came back for at all are gaps too.
    sum[Pile::Gap as usize] += len.saturating_sub(covered);
    let pct = |n: u64| if len == 0 { 0.0 } else { n as f64 * 100.0 / len as f64 };
    out.push(format!(
        "{rel}\t{name}\t{len}\tgap {} ({:.0}%)\topaque {} ({:.0}%)\tone-row {} ({:.0}%)",
        sum[1],
        pct(sum[1]),
        sum[2],
        pct(sum[2]),
        sum[3],
        pct(sum[3]),
    ));
    if list {
        worst.sort_by(|a, b| b.0.cmp(&a.0));
        for (b, at, p, d) in worst.iter().take(12) {
            out.push(format!("    {at:#08x} +{b:<7} {p:?}\t{d}"));
        }
    }
}

fn sweep(root: &Path, dir: &Path, list: bool, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            sweep(root, &path, list, out);
            continue;
        }
        let Ok(bytes) = fs::read(&path) else { continue };
        let rel = path.strip_prefix(root).unwrap_or(&path).display().to_string().replace('\\', "/");
        one(&rel, bytes, list, out);
    }
}

fn main() {
    let list = std::env::var_os("MEANING_LIST").is_some();
    let mut out = Vec::new();
    for arg in std::env::args().skip(1) {
        let path = Path::new(&arg);
        if path.is_dir() {
            sweep(path, path, list, &mut out);
        } else if let Ok(bytes) = fs::read(path) {
            one(&arg, bytes, list, &mut out);
        }
    }
    for line in out {
        println!("{line}");
    }
}
