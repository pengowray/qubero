//! What the core gives the report view for a file: the format profile of the
//! template and of the file, the byte ledger, the extent audit and the
//! directories, printed as readable text.
//!
//! `cargo run -p qubero-core --example report_data -- <file> [template] ...`
//!
//! A file with a second argument that names a template is read as that; one
//! without is sniffed. Several files may be given, each with or without a
//! template after it. Set `REPORT_SLICE` to walk in goes of that many
//! elements, the way the web app asks, and check that the answer is the same.

use std::time::Instant;

use qubero_core::document::Document;
use qubero_core::eval::{template_profile, Evaluator, Profile, ReportWalk};
use qubero_core::formats;
use qubero_core::source::MemSource;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let slice: Option<u64> = std::env::var("REPORT_SLICE").ok().and_then(|s| s.parse().ok());
    let mut i = 0;
    while i < args.len() {
        let file = &args[i];
        i += 1;
        let named = args.get(i).filter(|a| formats::template(a).is_some()).cloned();
        if named.is_some() {
            i += 1;
        }
        report(file, named, slice);
    }
}

fn report(file: &str, named: Option<String>, slice: Option<u64>) {
    let bytes = match std::fs::read(file) {
        Ok(b) => b,
        Err(e) => {
            println!("{file}: {e}");
            return;
        }
    };
    let name = match named {
        Some(n) => n,
        None => match formats::sniff(&bytes[..bytes.len().min(formats::SNIFF_WINDOW)], bytes.len() as u64) {
            Some(n) => n.to_string(),
            None => {
                println!("{file}: no template");
                return;
            }
        },
    };
    let Some(t) = formats::template(&name) else {
        println!("{file}: no template {name}");
        return;
    };
    println!("==== {file}: {} bytes, read as {name}", bytes.len());
    if let Some(a) = formats::about::about(&name) {
        println!("about: {} ({})", a.name, a.wikipedia.unwrap_or("-"));
    }
    let declared = template_profile(&t);
    let len = bytes.len() as u64 * 8;
    let doc = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(t);
    ev.set_slice(slice);
    let mut walk = ReportWalk::new(len);
    let started = Instant::now();
    let mut goes = 0;
    loop {
        goes += 1;
        ev.begin_slice();
        if let Err(e) = ev.report_step(&doc, &mut walk) {
            println!("error: {e:?}");
            return;
        }
        if walk.done() {
            break;
        }
        if goes > 1_000_000 {
            println!("never finished");
            return;
        }
    }
    println!("walked in {:.1} ms, {goes} goes", started.elapsed().as_secs_f64() * 1000.0);

    println!("-- profile of the template");
    print_profile(&declared);
    println!("-- profile of the file");
    print_profile(&walk.profile());

    // The kind totals over the same file, which the ledger's walk follows:
    // covered bits past the file's length are a stretch the template reads
    // twice, and show in the ledger too.
    let mut kinds_ev = Evaluator::new(formats::template(&name).expect("the same template"));
    let mut kinds = qubero_core::eval::KindWalk::new(len);
    if let Ok(k) = kinds_ev.kind_totals_step(&doc, &mut kinds) {
        println!("-- kind totals: {} covered, {} unmapped, {} reached", k.covered_bits, k.unmapped_bits, k.reached_bits);
    }
    let l = walk.ledger();
    println!("-- ledger: {} of {} bits counted, done {}", l.counted_bits, l.file_bits, l.done);
    for r in &l.rows {
        let share = r.bits as f64 * 100.0 / l.file_bits.max(1) as f64;
        let zero = if r.role == "gap" || r.role == "padding" {
            format!(" zero {} unscanned {}", r.zero_bits / 8, r.unscanned_bits / 8)
        } else {
            String::new()
        };
        println!(
            "  {:>10} B {share:5.1}%  {:>6}x  part {:?} {:<16} group {:<24} {:<10}{zero}  first {:?} @{:#x}",
            r.bits / 8,
            r.count,
            r.part,
            r.part_name,
            format!("{} ({})", r.group, r.group_from),
            r.role,
            r.first_path,
            r.first_offset_bits / 8
        );
    }

    let a = walk.audit();
    println!(
        "-- extent audit: {:?}, root ends at {:#x} of {:#x}{}",
        a.counts,
        a.root_end_bits / 8,
        a.space_bits / 8,
        a.root_failed.as_ref().map(|w| format!(", root failed: {w}")).unwrap_or_default()
    );
    for c in &a.checks {
        println!(
            "  {:<11} {} {:?} ({}) = {:?}{} sizes {} {:?} ({}) at {:#x}: stated {:?} read {:?} content {:?} room {} {}{}",
            c.verdict,
            c.role,
            c.length_path,
            c.length_name,
            c.length_value,
            if c.length_invalid { " INVALID" } else { "" },
            "",
            c.part_path,
            c.part_name,
            c.offset_bits / 8,
            c.stated,
            c.read,
            c.content_bits,
            c.room_bits,
            if c.adjusted { "adjusted " } else { "" },
            c.why
        );
    }

    let d = walk.directories();
    println!("-- directories: {} (unexamined {}), done {}", d.lists.len(), d.unexamined, d.done);
    for dir in &d.lists {
        println!("  {:?} {} : {} elements, {} place something", dir.path, dir.name, dir.elements, dir.placing);
        for e in dir.entries.iter().take(6) {
            let targets: Vec<String> = e
                .targets
                .iter()
                .map(|t| format!("{} {} @{:#x}+{}{}", t.via, t.name, t.offset_bits / 8, t.size_bits / 8, if t.aside { " aside" } else { "" }))
                .collect();
            println!("    {:?} {} @{:#x}+{} -> {}", e.path, e.name, e.offset_bits / 8, e.size_bits / 8, targets.join("; "));
        }
        if dir.entries.len() > 6 {
            println!("    ... {} more", dir.entries.len() - 6);
        }
    }
}

fn print_profile(p: &Profile) {
    for r in &p.rows {
        let extra = if r.category == "codec" { format!(" unpacked {} of them to {} B", r.unpacked_fields, r.unpacked_bits / 8) } else { String::new() };
        println!(
            "  {:<10} {:<16} w{:<3} {:<6} {:<10} {:>8} fields {:>10} B{extra}",
            r.category, r.kind, r.width, r.order, r.detail, r.fields, r.bits / 8
        );
    }
    for c in &p.choices {
        println!("  choice {:<20} {} cases, took {} over {} fields", c.name, c.cases, c.taken, c.fields);
    }
    println!("  facts {:?}", p.facts);
}
