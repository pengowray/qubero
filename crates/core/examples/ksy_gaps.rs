//! Convert every `.ksy` in a directory tree and print what the conversion had
//! to say, as JSON on stdout.
//!
//! This is what decides whether a Kaitai format is worth bundling, and what
//! `tools/ksy_bundle.mjs` reads to write the generated half of `bundled.rs`
//! and the table in `crates/core/formats-ksy/README.md`. Run over
//! `crates/core/formats-ksy` it reports the bundled set; run over a Kaitai
//! checkout's `formats/` it reports every candidate there is.
//!
//! Imports resolve against the same tree, so `/common/bcd` finds the file the
//! importing format meant whichever directory it was called from.
//!
//! Usage: `cargo run -p qubero-core --example ksy_gaps -- <dir> [imports-dir]`

use qubero_core::ksy::{self, MapImports};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().expect("usage: ksy_gaps <dir> [imports-dir]"));
    let imports_dir = args.next().map(PathBuf::from).unwrap_or_else(|| dir.clone());

    let mut files = Vec::new();
    walk(&dir, &mut files);
    files.sort();

    let mut map = HashMap::new();
    let mut import_files = Vec::new();
    walk(&imports_dir, &mut import_files);
    for path in &import_files {
        let key = path
            .strip_prefix(&imports_dir)
            .unwrap_or(path)
            .with_extension("")
            .to_string_lossy()
            .replace('\\', "/");
        map.insert(key, std::fs::read_to_string(path).unwrap_or_default());
    }
    let imports = MapImports(map);

    println!("[");
    for (i, path) in files.iter().enumerate() {
        let rel = path.strip_prefix(&dir).unwrap_or(path).to_string_lossy().replace('\\', "/");
        let text = std::fs::read_to_string(path).unwrap_or_default();
        print!("{{\"file\":{}", quote(&rel));
        match ksy::parse(&text) {
            Err(e) => print!(",\"parse_error\":{}", quote(&format!("{e}"))),
            Ok(spec) => {
                let meta = &spec.meta;
                print!(",\"id\":{}", quote(meta.id.as_deref().unwrap_or("")));
                print!(",\"title\":{}", quote(meta.title.as_deref().unwrap_or("")));
                print!(",\"license\":{}", quote(meta.license.as_deref().unwrap_or("")));
                // A root type with parameters has no meaning without its
                // arguments, so nothing can open a file as it: it is in the
                // collection to be imported and nothing else.
                print!(",\"params\":{}", spec.params.len());
                print!(",\"extensions\":[");
                for (j, ext) in meta.file_extension.iter().enumerate() {
                    print!("{}{}", if j > 0 { "," } else { "" }, quote(&ext.trim_start_matches('.').to_ascii_lowercase()));
                }
                print!("]");
                print!(",\"imports\":[");
                for (j, name) in meta.imports.iter().enumerate() {
                    print!("{}{}", if j > 0 { "," } else { "" }, quote(name));
                }
                print!("]");
            }
        }
        match ksy::convert(&text, &imports) {
            Err(e) => print!(",\"error\":{}", quote(&format!("{e}"))),
            Ok(converted) => {
                print!(",\"fields\":{}", converted.report.fields.len());
                print!(",\"notes\":{}", converted.report.notes.len());
                print!(",\"magics\":[");
                for (j, (at, bytes)) in ksy::signature(&converted.template).iter().enumerate() {
                    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
                    print!("{}[{at},{}]", if j > 0 { "," } else { "" }, quote(&hex));
                }
                print!("]");
                print!(",\"gaps\":[");
                for (j, gap) in converted.report.gaps.iter().enumerate() {
                    if j > 0 {
                        print!(",");
                    }
                    print!(
                        "{{\"path\":{},\"source\":{},\"reason\":{}}}",
                        quote(&gap.path),
                        quote(&gap.source),
                        quote(&gap.reason)
                    );
                }
                print!("]");
            }
        }
        println!("}}{}", if i + 1 == files.len() { "" } else { "," });
    }
    println!("]");
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('_')) {
                continue;
            }
            walk(&path, out);
        } else if path.extension().is_some_and(|e| e == "ksy") {
            out.push(path);
        }
    }
}

fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
