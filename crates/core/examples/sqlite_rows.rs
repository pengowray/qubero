//! Follow every row of a SQLite database that did not fit on its page.
//!
//! Prints one line per spilled row: how long it claims to be, how much the
//! chain reached, how many pages it took, and a checksum of the bytes, which
//! is what a separate reader of the same file can be compared against.
//!
//! Given a second argument, writes each row's assembled bytes there as
//! `page-cell.row`, so that another reader of the same database can be held
//! against them byte for byte.
//!
//! Usage: `cargo run --example sqlite_rows -- <file.sqlite> [directory]`

use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats::{sqlite, sqlite_payload};
use qubero_core::gather::Gathered;
use qubero_core::source::{MemSource, Source as _};

fn main() {
    let path = std::env::args().nth(1).expect("a database");
    let into = std::env::args().nth(2);
    let doc = Document::new(MemSource(std::fs::read(&path).expect("readable")));
    let mut ev = Evaluator::new(sqlite());

    // The pages, and within a page its cells. Both are found by name so that a
    // change to the template does not silently walk somewhere else.
    let pages = ev.child_named(&doc, &[], "pages").expect("pages").expect("a pages field");
    let page_count = ev.node(&doc, &pages).expect("pages").child_count as usize;
    let (mut spilled, mut broken) = (0usize, 0usize);

    for p in 0..page_count {
        let page = [pages.as_slice(), &[p]].concat();
        let Ok(Some(cells)) = ev.child_named(&doc, &page, "cells") else { continue };
        let Ok(node) = ev.node(&doc, &cells) else { continue };
        for c in 0..node.child_count as usize {
            let cell = [cells.as_slice(), &[c]].concat();
            // A cell with no payload is an interior page's pointer to a child.
            if ev.child_named(&doc, &cell, "payload").ok().flatten().is_none() {
                continue;
            }
            let Ok(found) = sqlite_payload(&mut ev, &doc, &cell) else { continue };
            if found.pages.is_empty() {
                continue;
            }
            spilled += 1;
            if !found.complete() {
                broken += 1;
            }
            let gathered = Gathered::new(doc.source(), found.extents.iter().copied());
            let mut bytes = vec![0u8; gathered.len_bytes() as usize];
            gathered.read_bytes(0, &mut bytes);
            if let Some(dir) = &into {
                let name = std::path::Path::new(dir).join(format!("{}-{c}.row", p + 2));
                std::fs::write(name, &bytes).expect("writable");
            }
            println!(
                "page {:<5} cell {:<3} {:>9} bytes claimed, {:>9} found over {:>4} pages  {:016x}{}",
                p + 2,
                c,
                found.declared,
                found.found,
                found.pages.len(),
                checksum(&bytes),
                found.problem.clone().map(|p| format!("  {p}")).unwrap_or_default(),
            );
            // The row read as the columns it holds, which is what the panel
            // shows when the cursor is on a payload that spilled.
            let row = qubero_core::formats::sqlite_overflow::read(&doc, &found, 1);
            for (i, column) in row.columns.iter().enumerate() {
                println!("    column {i}: {:<20} {:?}", column.type_name, column.value);
            }
            if let Some(problem) = row.problem {
                println!("    {problem}");
            }
        }
    }
    println!("{spilled} rows spilled, {broken} of them incomplete");
}

/// A checksum simple enough that another language can compute the same one in
/// three lines, which is the point: the answer has to be comparable against a
/// reader that shares no code with this.
fn checksum(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        h ^= *byte as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}
