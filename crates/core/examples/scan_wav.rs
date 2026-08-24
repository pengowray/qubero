//! What a WAVE file's samples read as:
//! `cargo run --example scan_wav -- path/to/file.wav`

use std::fs;

use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats::wav;
use qubero_core::source::MemSource;

fn main() {
    let path = std::env::args().nth(1).expect("usage: scan_wav <file.wav>");
    let d = Document::new(MemSource(fs::read(&path).expect("read")));
    let mut ev = Evaluator::new(wav());
    let chunks = ev.node(&d, &[3]).expect("chunks");
    println!("{path}: {} chunks", chunks.child_count);
    for i in 0..chunks.child_count as usize {
        let c = ev.node(&d, &[3, i]).expect("chunk");
        let body = ev.node(&d, &[3, i, 2]).expect("body");
        println!("  {:<14} {:>12} bytes  body {} ({})", c.name, c.size_bits / 8, body.type_name, body.child_count);
        if c.name.contains("data") && body.composite {
            let spans = ev.spans(&d, body.offset_bits, body.offset_bits + 512 * 8, 8).expect("spans");
            println!("    annotation column: {} entries over the first 512 bytes", spans.len());
            for s in spans.iter().take(3) {
                println!(
                    "      {:<10} {:>6} bytes  count {}  sample {:?}",
                    s.name,
                    s.size_bits / 8,
                    s.count,
                    s.sample
                );
            }
            for f in 0..body.child_count.min(3) {
                let sample = ev.node(&d, &[3, i, 2, f as usize]).expect("sample");
                println!("    sample {f} at 0x{:x}: {:?}", sample.offset_bits / 8, sample.value);
            }
        }
    }
}
