//! Which files in a folder read as a hex dump, and what the reader made of
//! each. A census, for judging how often it says yes to something that is not
//! one: point it at a tree of anything and every line it prints is either a
//! dump or a mistake.
//!
//! `cargo run -p qubero-core --example dump_census -- <path>...`

use qubero_core::hexdump;

fn main() {
    let mut yes = 0usize;
    let mut looked = 0usize;
    for arg in std::env::args().skip(1) {
        let mut stack = vec![std::path::PathBuf::from(arg)];
        while let Some(p) = stack.pop() {
            if p.is_dir() {
                stack.extend(std::fs::read_dir(&p).into_iter().flatten().flatten().map(|e| e.path()));
                continue;
            }
            let Ok(bytes) = std::fs::read(&p) else { continue };
            if bytes.is_empty() || bytes.len() > hexdump::LIMIT {
                continue;
            }
            looked += 1;
            let Some(dump) = hexdump::read(&bytes, 0) else { continue };
            yes += 1;
            let l = &dump.layout;
            let address = match &l.address {
                Some(a) => format!("{} to {} digits", a.base.name(), a.digits.map_or(0, |d| d)),
                None => "none".to_string(),
            };
            println!(
                "{:>9} bytes over {:>4} lines of {:>2} in groups of {:>2}, address {:<24} strays {:>4}, {} {}",
                dump.byte_count(),
                dump.byte_count().div_ceil(l.bytes_per_line.max(1) as u64),
                l.bytes_per_line,
                l.group,
                address,
                strays(&dump, &bytes),
                l.looks_like().unwrap_or("unnamed"),
                p.display(),
            );
        }
    }
    eprintln!("read as a dump: {yes} of {looked}");
}

/// Lines with something on them that the reader did not take as part of the
/// dump. The number the reader itself weighs, printed so a file that only just
/// passed can be told from one that sailed through.
fn strays(dump: &hexdump::Dump<'_>, bytes: &[u8]) -> usize {
    dump.skipped
        .iter()
        .filter(|at| {
            let from = **at as usize;
            let end = bytes[from..].iter().position(|b| *b == b'\n').map_or(bytes.len(), |i| from + i);
            bytes[from..end].iter().any(|b| !b.is_ascii_whitespace())
        })
        .count()
}
