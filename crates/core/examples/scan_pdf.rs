//! What a PDF's table says and where it puts the objects:
//! `cargo run --example scan_pdf -- path/to/file.pdf`

use std::fs;

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Explain, Value};
use qubero_core::formats::{pdf, pdf_xref};
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
    println!("  at the offset {}", show(&mut ev, &d, &[3, 0]));
    println!("  trailer      {}", show(&mut ev, &d, &[5, 0]));
    println!("  eof          {}", show(&mut ev, &d, &[8, 0]));

    // The table may be a stream inside an object instead of lines of text.
    if let Ok(s) = ev.node(&d, &[6, 0]) {
        if s.size_bits > 0 {
            println!("  cross-reference stream at {}, {} bytes", s.offset_bits / 8, s.size_bits / 8);
            let dict = match ev.node(&d, &[6, 0, 3]).map(|n| n.value) {
                Ok(Value::Str(t)) => t,
                _ => String::new(),
            };
            println!("    dictionary {dict:?}");
            let rows = ev.node(&d, &[6, 0, 5]).expect("the packed rows");
            let mut bytes = vec![0u8; (rows.size_bits / 8) as usize];
            d.read_bytes(rows.offset_bits / 8, &mut bytes);
            match qubero_core::formats::pdf_xref::decode(&dict, &bytes) {
                Err(p) => println!("    rows       {} bytes: {}", bytes.len(), p.as_str()),
                Ok(t) => {
                    println!(
                        "    rows       {} packed bytes, {} decoded, /W {:?}, predictor {:?}",
                        bytes.len(),
                        t.decoded_bytes,
                        t.widths,
                        t.predictor
                    );
                    let (free, in_file, in_stream) = t.rows.iter().fold((0, 0, 0), |(f, o, s), r| match r.kind {
                        pdf_xref::Kind::Free => (f + 1, o, s),
                        pdf_xref::Kind::InFile => (f, o + 1, s),
                        _ => (f, o, s + 1),
                    });
                    println!("    {} rows: {free} free, {in_file} in the file, {in_stream} in object streams", t.rows.len());
                    for r in t.rows.iter().take(4).chain(t.rows.iter().rev().take(1)) {
                        let (a, b) = r.kind.field_names();
                        println!("      object {:>6}  {:<20} {a} {}, {b} {}", r.object, r.kind.as_str(), r.second, r.third);
                    }
                    let past = t.rows.iter().filter(|r| r.kind == pdf_xref::Kind::InFile && r.second >= d.len_bytes()).count();
                    println!("    {past} of the offsets are past the end of the file");
                }
            }
        }
    }

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

    let Ok(objects) = ev.node(&d, &[9]) else {
        println!("  no objects: the table did not read");
        return;
    };
    println!("  {} objects", objects.child_count);
    let mut covered = 0u64;
    for i in 0..objects.child_count as usize {
        let o = match ev.node(&d, &[9, i]) {
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
                show(&mut ev, &d, &[9, i, 0]),
                show(&mut ev, &d, &[9, i, 1]),
                show(&mut ev, &d, &[9, i, 2]),
            );
        }
    }
    println!("  {covered} bytes of {} are inside an object", d.len_bits() / 8);

    // The object streams among them, and what each one holds. Most of a modern
    // PDF's objects are in here rather than at an offset of their own.
    let mut streams = 0;
    let mut inside = 0;
    for i in 0..objects.child_count as usize {
        let Ok(Explain::ObjStm { objects: os, total, problem, packed_bytes, decoded_bytes, extends, .. }) =
            ev.explain(&d, &[9, i], None)
        else {
            continue;
        };
        streams += 1;
        inside += total;
        let number = show(&mut ev, &d, &[9, i, 0]);
        println!("  object stream {number}: {packed_bytes} packed bytes, {decoded_bytes} decoded, {total} objects");
        if let Some(p) = problem {
            println!("    {p}");
        }
        if let Some(e) = extends {
            println!("    extends object stream {e}");
        }
        for o in os.iter().take(3) {
            let text: String = o.text.chars().take(70).collect();
            println!("    object {:>6}  {} bytes  {text}", o.number, o.len);
        }
    }
    println!("  {streams} object streams holding {inside} objects");
}
