//! Disassemble the program inside a UF2, across the blocks it is cut into.
//!
//! Usage: `cargo run --example uf2_code -- <file.uf2> [how many lines]`
use qubero_core::code::decode;
use qubero_core::formats::uf2_image;
use qubero_core::gather::Gathered;
use qubero_core::source::{MemSource, Source};

/// What the file fills, in one line.
fn describe(image: &qubero_core::formats::Uf2Image) -> String {
    let total: u64 = image.runs.iter().map(|r| r.len()).sum();
    format!("{} bytes over {} run(s)", total, image.runs.len())
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("a .uf2 file");
    let lines: usize = args.next().and_then(|n| n.parse().ok()).unwrap_or(20);
    let file = MemSource(std::fs::read(&path).expect("readable"));

    let image = uf2_image(&file);
    let Some(isa) = image.isa() else {
        println!("{path}: no chip named, so no decoder chosen");
        return;
    };
    println!("{path}: {} as {}", describe(&image), isa.name());
    let run = image.runs.iter().max_by_key(|r| r.len()).expect("a run");
    println!("  the largest run: {} bytes at {:#x}", run.len(), run.address);

    let code = Gathered::new(&file, run.extents.iter().copied());
    let mut bytes = vec![0u8; code.len_bytes() as usize];
    code.read_bytes(0, &mut bytes);

    // What the joins cost when they are not joined. Decoding each block on its
    // own is what this crate could do before: every instruction that begins in
    // one block and ends in the next is read as two wrong ones, and the count
    // below is how often that happens in this file.
    let mut straddling = 0usize;
    let mut at = 0usize;
    while at < bytes.len() {
        let insn = decode(isa, &bytes[at..(at + isa.longest()).min(bytes.len())]);
        if code.origin(at as u64, insn.len as u64).len() > 1 {
            straddling += 1;
        }
        at += insn.len.max(1);
    }
    println!("  {straddling} instructions sit across a join between two blocks");

    let mut at = 0usize;
    for _ in 0..lines {
        if at >= bytes.len() {
            break;
        }
        let insn = decode(isa, &bytes[at..(at + isa.longest()).min(bytes.len())]);
        // Where this instruction really is, which is two places when it sits
        // across the join between two blocks.
        let origin = code.origin(at as u64, insn.len as u64);
        let places: Vec<String> = origin.iter().map(|e| format!("{:x}+{}", e.at, e.len)).collect();
        println!("  {:#010x}  {:<28}  file {}", run.address + at as u64, insn.text, places.join(" and "));
        at += insn.len.max(1);
    }
}
