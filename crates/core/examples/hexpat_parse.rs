//! Parse every pattern and every include of an ImHex-Patterns checkout and say
//! how many of each the parser gets through.
//!
//! This is the syntax oracle for the `.hexpat` converter: the parser is done
//! when it reads all of them, and not before. Every failure prints one line,
//! `file:line:col: message`, so a run diffs against the last one.
//!
//! Usage: `cargo run -p qubero-core --example hexpat_parse -- <dir>`, or set
//! `IMHEX_PATTERNS` to the checkout instead. `<dir>` is the repository root,
//! the one holding `patterns/` and `includes/`.
//!
//! `__IMHEX__` is defined, because that is the host the corpus is written for
//! and it is what decides which branch of an `#ifdef` a pattern means.

use qubero_core::hexpat::{parse_with, HexpatError, Resolved, Resolver};
use std::path::{Path, PathBuf};

/// Finds an include the way `resolvers.cpp` does: try each include directory,
/// add `.hexpat` then `.pat` to a path with no extension, and read `pattern`
/// inside a directory.
struct FileResolver {
    roots: Vec<PathBuf>,
}

impl Resolver for FileResolver {
    fn resolve(&self, path: &str) -> Option<Resolved> {
        for root in &self.roots {
            let full = root.join(path);
            let candidates: Vec<PathBuf> = if full.extension().is_some() {
                vec![full]
            } else {
                let base = if full.is_dir() { full.join("pattern") } else { full };
                vec![base.with_extension("hexpat"), base.with_extension("pat")]
            };
            for candidate in candidates {
                if !candidate.is_file() {
                    continue;
                }
                let text = std::fs::read_to_string(&candidate).ok()?;
                return Some(Resolved {
                    name: candidate.to_string_lossy().replace('\\', "/"),
                    text: text.replace("\r\n", "\n"),
                });
            }
        }
        None
    }
}

fn main() {
    let dir = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("IMHEX_PATTERNS").ok())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            eprintln!("usage: hexpat_parse <ImHex-Patterns dir>   (or set IMHEX_PATTERNS)");
            std::process::exit(2);
        });

    let resolver = FileResolver { roots: vec![dir.join("includes"), dir.join("patterns"), dir.clone()] };
    let defines = vec!["__IMHEX__".to_string()];

    let mut patterns = Vec::new();
    walk(&dir.join("patterns"), "hexpat", &mut patterns);
    patterns.sort();

    let mut includes = Vec::new();
    walk(&dir.join("includes"), "pat", &mut includes);
    includes.sort();

    let patterns_ok = run(&patterns, &resolver, &defines);
    let includes_ok = run(&includes, &resolver, &defines);

    println!("{patterns_ok}/{} patterns, {includes_ok}/{} includes", patterns.len(), includes.len());
}

fn run(files: &[PathBuf], resolver: &FileResolver, defines: &[String]) -> usize {
    let mut ok = 0;
    for path in files {
        let name = path.to_string_lossy().replace('\\', "/");
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text.replace("\r\n", "\n"),
            Err(error) => {
                println!("{name}:0:0: {error}");
                continue;
            }
        };
        match parse_with(&name, &text, resolver, defines) {
            Ok(_) => ok += 1,
            Err(error) => println!("{}", report(&error)),
        }
    }
    ok
}

fn report(error: &HexpatError) -> String {
    format!("{}:{}:{}: {}", error.file, error.pos.line, error.pos.col, error.message)
}

fn walk(dir: &Path, extension: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, extension, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some(extension) {
            out.push(path);
        }
    }
}
