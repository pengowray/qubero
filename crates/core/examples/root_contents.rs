//! What a CERN ROOT file holds, in its own terms:
//! `cargo run --release --example root_contents -- path/to/file.root [values]`
//!
//! The class descriptions out of `StreamerInfo`, then every tree with its
//! branches, leaves and baskets. Add `values` and the first basket of each
//! branch is read as well, which is what checks the basket reader against a
//! file rather than against a hand-built one.

use std::cell::RefCell;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::time::Instant;

use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats::root;
use qubero_core::formats::root_tree::{self, Reading, Values};
use qubero_core::source::{Missing, Source};

struct FileSource {
    file: RefCell<File>,
    len: u64,
}

impl Source for FileSource {
    fn len_bytes(&self) -> u64 {
        self.len
    }
    fn read_bytes(&self, offset: u64, out: &mut [u8]) -> Vec<Missing> {
        let mut f = self.file.borrow_mut();
        if f.seek(SeekFrom::Start(offset)).is_err() || f.read_exact(out).is_err() {
            out.fill(0);
        }
        Vec::new()
    }
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: root_contents <file.root> [values]");
    let want_values = std::env::args().any(|a| a == "values");
    let file = File::open(&path).expect("open");
    let len = file.metadata().expect("metadata").len();
    let doc = Document::new(FileSource { file: RefCell::new(file), len });
    let mut ev = Evaluator::new(root());

    let start = Instant::now();
    let contents = root_tree::contents(&mut ev, &doc).expect("contents");
    println!("{path}: {len} bytes");
    if let Some(why) = &contents.trouble {
        println!("trouble: {why}");
    }

    println!("\n{} classes described", contents.classes.len());
    for class in &contents.classes {
        println!("  {} v{} checksum {:#x}", class.name, class.version, class.checksum);
        for member in &class.members {
            let dims = match member.dims.is_empty() {
                true => String::new(),
                false => format!("[{}]", member.dims.iter().map(|d| d.to_string()).collect::<Vec<_>>().join("][")),
            };
            println!(
                "      {:<24} {:<24} code {:<4} {:>4} bytes{}{}",
                member.name,
                member.type_name,
                member.code,
                member.size,
                dims,
                if member.base { "  (base class)" } else { "" }
            );
        }
    }

    for tree in &contents.trees {
        println!("\ntree {} \"{}\" at {} · {} entries", tree.name, tree.title, tree.at, tree.entries);
        if let Some(why) = &tree.trouble {
            println!("  trouble: {why}");
        }
        for branch in &tree.branches {
            let pad = "  ".repeat(branch.depth);
            let reading = match &branch.reading {
                Reading::Fixed { width, per_entry, floating, unsigned } => format!(
                    "{}{}{} × {per_entry}",
                    if *floating { "f" } else if *unsigned { "u" } else { "i" },
                    width * 8,
                    ""
                ),
                Reading::Not(why) => format!("not read: {why}"),
            };
            println!(
                "  {pad}{:<28} {:<16} {} baskets · {} entries · {}",
                branch.name,
                branch.class,
                branch.basket_total,
                branch.entries,
                reading
            );
            for leaf in &branch.leaves {
                println!(
                    "  {pad}    leaf {} ({}) {} × {} bytes{}",
                    leaf.name,
                    leaf.class,
                    leaf.len,
                    leaf.width,
                    match leaf.counted_by.is_empty() {
                        true => String::new(),
                        false => format!(", counted by {}", leaf.counted_by),
                    }
                );
            }
            for (i, basket) in branch.baskets.iter().take(4).enumerate() {
                println!(
                    "  {pad}    basket {i} at {} · {} bytes · entries {}..{}",
                    basket.at,
                    basket.bytes,
                    basket.first_entry,
                    basket.first_entry + basket.entries
                );
            }
            if !want_values || branch.baskets.is_empty() {
                continue;
            }
            match root_tree::branch_basket(&doc, branch, 0) {
                Err(why) => println!("  {pad}    basket 0 would not read: {why:?}"),
                Ok(data) => {
                    let shown = match &data.values {
                        Values::Ints(v) => {
                            format!("{:?}", &v[..v.len().min(5)])
                        }
                        Values::Floats(v) => format!("{:?}", &v[..v.len().min(5)]),
                        Values::None(why) => format!("not read: {why}"),
                    };
                    println!(
                        "  {pad}    basket 0: {} entries, {} bytes of values, {} offsets · {shown}",
                        data.entries,
                        data.border,
                        data.offsets.len()
                    );
                }
            }
        }
        if tree.branch_total > tree.branches.len() {
            println!("  {} branches, {} shown", tree.branch_total, tree.branches.len());
        }
    }
    println!("\nread in {:?}", start.elapsed());
}
