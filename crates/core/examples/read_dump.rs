//! Read a hex dump back into the bytes it describes, and say what it took.
//!
//! `cargo run -p qubero-core --example read_dump -- <dump.txt> [out.bin]`

use qubero_core::hexdump::{self, Agreement, Note};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: read_dump <dump.txt> [out.bin]");
    let out = args.next();
    let bytes = std::fs::read(&path).expect("read");
    let Some(dump) = hexdump::read(&bytes, 0) else {
        println!("{path}: no dump found");
        return;
    };
    let l = &dump.layout;
    println!("{path}");
    println!("  read as    : {:?}", dump.tier());
    println!("  looks like : {}", l.looks_like().unwrap_or("nothing named here"));
    match &l.address {
        Some(a) => println!("  address    : {}, {} digits{}", a.base.name(), a.digits.map_or(0, |d| d), a.suffix.map_or(String::new(), |c| format!(", closed by {c:?}"))),
        None => println!("  address    : none"),
    }
    println!("  line       : {} bytes in groups of {}, {}", l.bytes_per_line, l.group, if l.upper { "upper case" } else { "lower case" });
    println!("  order      : {:?}", l.order);
    match &l.text {
        Some(t) => println!("  characters : {}, standing in with {:?}{}", t.glyphs.name(), t.placeholders, t.open.map_or(String::new(), |c| format!(", wrapped in {c:?}"))),
        None => println!("  characters : none"),
    }
    if !l.assumed.is_empty() {
        println!("  assumed    : {:?}", l.assumed);
    }
    for n in &dump.notes {
        match n {
            Note::Named(s) => println!("  names      : {s}"),
            Note::Length(v) => println!("  says length: {v}"),
            Note::Command(s) => println!("  command    : {s}"),
        }
    }
    println!("  covers     : {} bytes over {} stretches, {} lines skipped", dump.byte_count(), dump.extents().len(), dump.skipped.len());
    for e in dump.extents() {
        println!("               {:#x}..{:#x}", e.at, e.end());
    }
    let all = dump.span().map_or(Vec::new(), |(a, b)| dump.rows(a, b));
    let confirmed = all.iter().flat_map(|r| &r.agreement).filter(|a| **a == Agreement::Confirmed).count();
    let unverifiable = all.iter().flat_map(|r| &r.agreement).filter(|a| **a == Agreement::Unverifiable).count();
    let conflicts = dump.conflicts();
    println!("  columns    : {confirmed} confirmed, {unverifiable} unverifiable, {} in conflict", conflicts.len());
    for (at, wrote, digits) in conflicts.iter().take(5) {
        println!("               {at:#x}: digits say {digits:02x}, characters say {wrote:?}");
    }
    let _ = unverifiable;
    if let (Some(out), Some((from, to))) = (out, dump.span()) {
        let mut buf = vec![0u8; (to - from) as usize];
        let n = dump.read_at(from, &mut buf);
        std::fs::write(&out, &buf[..n]).expect("write");
        println!("  wrote      : {n} bytes to {out}");
    }
}
