//! What a PDF's table says and where it puts the objects:
//! `cargo run --example scan_pdf -- path/to/file.pdf`

use std::fs;

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats::pdf;
use qubero_core::source::MemSource;

/// The value of a field, or why it could not be read.
fn show(ev: &mut Evaluator, d: &Document<MemSource>, at: &[usize]) -> String {
    match ev.node(d, at) {
        Ok(n) => match n.value {
            Value::Str(s) => format!("{s:?}"),
            other => format!("{other:?}"),
        },
        Err(e) => format!("<{e:?}>"),
    }
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: scan_pdf <file.pdf>");
    let d = Document::new(MemSource(fs::read(&path).expect("read")));
    let mut ev = Evaluator::new(pdf());

    println!("{path}");
    println!("  version      {}", show(&mut ev, &d, &[0]));
    println!("  startxref at {}", show(&mut ev, &d, &[1]));
    println!("  table at     {}", show(&mut ev, &d, &[2, 0]));
    println!("  xref         {}", show(&mut ev, &d, &[3, 0]));
    println!("  trailer      {}", show(&mut ev, &d, &[5, 0]));
    println!("  eof          {}", show(&mut ev, &d, &[7, 0]));

    // The subsections the table is written in, one for a file saved once.
    match ev.node(&d, &[4, 0]) {
        Ok(lines) => {
            println!("  {} lines in the table", lines.child_count);
            for i in 0..lines.child_count as usize {
                let Ok(l) = ev.node(&d, &[4, 0, i]) else { continue };
                if l.type_name != "Subsection" {
                    continue;
                }
                println!(
                    "    subsection from object {} for {} entries",
                    show(&mut ev, &d, &[4, 0, i, 0]),
                    show(&mut ev, &d, &[4, 0, i, 1]),
                );
            }
        }
        Err(e) => println!("  the table did not read: {e:?}"),
    }

    let Ok(objects) = ev.node(&d, &[8]) else {
        println!("  no objects: the table did not read");
        return;
    };
    println!("  {} objects", objects.child_count);
    let mut covered = 0u64;
    for i in 0..objects.child_count as usize {
        let o = match ev.node(&d, &[8, i]) {
            Ok(o) => o,
            Err(e) => {
                println!("    [{i}] did not read: {e:?}");
                continue;
            }
        };
        covered += o.size_bits / 8;
        if o.size_bits == 0 {
            if i < 5 {
                println!("    [{i}] points at nothing");
            }
            continue;
        }
        if i < 5 || i + 3 > objects.child_count as usize {
            println!(
                "    [{i}] at {:>9}  {:>8} bytes  {} {} {}",
                o.offset_bits / 8,
                o.size_bits / 8,
                show(&mut ev, &d, &[8, i, 0]),
                show(&mut ev, &d, &[8, i, 1]),
                show(&mut ev, &d, &[8, i, 2]),
            );
        }
    }
    println!("  {covered} bytes of {} are inside an object", d.len_bits() / 8);
}
