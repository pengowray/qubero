//! Every object stream in a PDF and what it holds, found by scanning the file
//! for `obj` rather than by reading the table:
//! `cargo run --example scan_objstm -- path/to/file.pdf`
//!
//! The table is how a reader is meant to find these, and for a file that keeps
//! its table in a cross-reference stream the objects do not reach the tree at
//! all yet. Scanning is how the decoder gets exercised against real files in
//! the meantime.

use std::fs;

use qubero_core::formats::pdf_objstm;

fn main() {
    let path = std::env::args().nth(1).expect("usage: scan_objstm <file.pdf>");
    let d = fs::read(&path).expect("read");
    println!("{path}  {} bytes", d.len());

    let mut streams = 0;
    let mut held = 0;
    let mut at = 0;
    while let Some(i) = d[at..].windows(4).position(|w| w == b" obj") {
        let start = at + i + 4;
        at = start;
        let end = d[start..].windows(6).position(|w| w == b"endobj").map_or(d.len(), |n| start + n);
        let body = &d[start..end];
        let Some((dict, data)) = pdf_objstm::split_body(body) else { continue };
        if !pdf_objstm::is_object_stream(dict) {
            continue;
        }
        streams += 1;
        match pdf_objstm::decode(dict, data) {
            Err(p) => println!("  at {start}: {}", p.as_str()),
            Ok(s) => {
                held += s.objects.len();
                println!(
                    "  at {start}: {} packed bytes, {} decoded, {} of {} objects{}",
                    data.len(),
                    s.decoded_bytes,
                    s.objects.len(),
                    s.claimed,
                    s.extends.map_or(String::new(), |e| format!(", extends {e}")),
                );
                for o in s.objects.iter().take(2) {
                    let text: String = o.text.chars().take(76).collect();
                    println!("    {:>6}  {:>5} bytes  {text}", o.number, o.len);
                }
            }
        }
    }
    println!("  {streams} object streams holding {held} objects");
}
