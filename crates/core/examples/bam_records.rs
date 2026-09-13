//! Every alignment record of a BAM, found block by block the way the cursor
//! would find them: `bam_records <file> [block]`.
//!
//! One line a record, tab-separated: the virtual offset, the fields in the
//! order BAM writes them, the qualities as numbers, and the tags sorted by
//! name with the type letter BAM stored. That is the shape of the dump the
//! cross-check against bamnostic writes, so the two can be compared with
//! `diff`. With a block number, only that block's records, and a line about
//! the block first.

use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats;
use qubero_core::formats::bam_records::{Record, TagValue};
use qubero_core::source::MemSource;

fn main() {
    let path = std::env::args().nth(1).expect("usage: bam_records <file> [block]");
    let only: Option<usize> = std::env::args().nth(2).and_then(|b| b.parse().ok());
    let doc = Document::new(MemSource(std::fs::read(&path).unwrap()));
    let mut ev = Evaluator::new(formats::template("bgzf").unwrap());
    let blocks = ev.node(&doc, &[0]).unwrap().child_count as usize;
    for i in 0..blocks {
        if only.is_some_and(|b| b != i) {
            continue;
        }
        let block = ev.bam_block(&doc, &[0, i]).unwrap().expect("a BGZF block");
        if only.is_some() {
            println!(
                "block {} at {}: {} bytes, unpacks to {}, {} of header, {} carried in, {} records, walked {} blocks ({} bytes), problem {:?}",
                block.index,
                block.block_offset,
                block.packed_bytes,
                block.decoded_bytes,
                block.header_bytes,
                block.carried,
                block.records.len(),
                block.blocks_walked,
                block.bytes_walked,
                block.problem
            );
        } else if let Some(p) = &block.problem {
            eprintln!("block {i}: {p}");
        }
        for r in &block.records {
            println!("{}", line(r));
        }
    }
}

fn line(r: &Record) -> String {
    let mut tags: Vec<_> = r.tags.iter().collect();
    tags.sort_by(|a, b| a.tag.cmp(&b.tag));
    let tags: Vec<String> = tags
        .iter()
        .map(|t| {
            let value = match &t.value {
                TagValue::Char(c) => c.to_string(),
                TagValue::Int(n) => n.to_string(),
                TagValue::Float(f) => f64::from(*f).to_string(),
                TagValue::Text(s) | TagValue::Hex(s) => s.clone(),
                TagValue::Ints(_, v) => v.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(","),
                TagValue::Floats(v) => v.iter().map(|n| f64::from(*n).to_string()).collect::<Vec<_>>().join(","),
            };
            format!("{}:{}:{}", t.tag, t.type_code, value)
        })
        .collect();
    let qual: Vec<String> = r.qual.iter().map(|q| q.to_string()).collect();
    [
        r.virtual_offset().to_string(),
        r.read_name.clone(),
        r.flag.to_string(),
        r.ref_id.to_string(),
        r.pos.to_string(),
        r.mapq.to_string(),
        r.bin.to_string(),
        r.cigar.clone(),
        r.next_ref_id.to_string(),
        r.next_pos.to_string(),
        r.tlen.to_string(),
        r.seq.clone(),
        qual.join(","),
        tags.join(" "),
    ]
    .join("\t")
}
