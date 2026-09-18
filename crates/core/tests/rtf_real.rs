//! A real rich text file, read all the way down.
//!
//! One Word document of a table, 51 KB of it, which is the shape that matters:
//! a template that reads `{\rtf1 hi}` and not a real file has read nothing.
//! What it checks is that the whole file is one group, that every item in it
//! resolves, and that the group structure closes where the file does.
//!
//! The file lives in the sample collection rather than here. Point
//! `QUBERO_SAMPLES` at it, or keep it beside the repository as
//! `qubero-samples`. With neither, the test says so and passes.

use std::path::PathBuf;

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

fn sample(name: &str) -> Option<PathBuf> {
    qubero_samples::roots().into_iter().map(|r| r.join("rtf").join(name)).find(|p| p.exists())
}

fn read(name: &str) -> Option<(Document<MemSource>, Evaluator)> {
    let path = sample(name)?;
    let doc = Document::new(MemSource(std::fs::read(&path).unwrap()));
    Some((doc, Evaluator::new(formats::builtin("rtf").unwrap())))
}

/// Resolve every node under `path`, and say how many there were and how deep
/// they went. The first that does not read is the answer, said rather than
/// thrown: what a reader wants to know is which item stopped it.
fn walk(ev: &mut Evaluator, d: &Document<MemSource>, path: &mut Vec<usize>, nodes: &mut usize, deepest: &mut usize) -> Result<(), String> {
    let node = ev.node(d, path).map_err(|e| format!("{path:?} does not read: {e:?}"))?;
    *nodes += 1;
    *deepest = (*deepest).max(path.len());
    for i in 0..node.child_count as usize {
        path.push(i);
        walk(ev, d, path, nodes, deepest)?;
        path.pop();
    }
    Ok(())
}

#[test]
fn a_real_word_file_is_one_group_covering_all_of_it() {
    let Some((d, mut ev)) = read("word16-table.rtf") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let root = ev.node(&d, &[]).unwrap();
    // The outer group and nothing beside it: no gap before the brace and none
    // after the one that closes it.
    assert_eq!(root.size_bits, d.len_bits());
    assert_eq!(
        ev.node(&d, &[0]).unwrap().value,
        Value::Enum { raw: b'{' as i128, name: Some("group".into()), hex: false }
    );
    // `{\rtf1` is the first five bytes: the brace, and a control word of a
    // backslash, three letters and the version.
    assert_eq!(ev.node(&d, &[1, 0, 1, 0]).unwrap().value, Value::Str("rtf".into()));
    assert_eq!(ev.node(&d, &[1, 0, 1, 1]).unwrap().value, Value::Int(1));
    // The character set, which is a control word with no parameter at all.
    assert_eq!(ev.node(&d, &[1, 2, 1, 0]).unwrap().value, Value::Str("ansi".into()));
    assert_eq!(ev.node(&d, &[1, 2, 1, 1]).unwrap().size_bits, 0);
}

#[test]
fn every_item_of_a_real_word_file_reads() {
    let Some((d, mut ev)) = read("word16-table.rtf") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let (mut nodes, mut deepest) = (0, 0);
    if let Err(why) = walk(&mut ev, &d, &mut Vec::new(), &mut nodes, &mut deepest) {
        panic!("{why}");
    }
    // Twenty thousand items in fifty kilobytes, which is what a file of
    // control words looks like. The numbers are what this file holds rather
    // than anything the format fixes, so they are checked as orders of
    // magnitude: a template that quietly stopped reading early would fall
    // well short of them.
    assert!(nodes > 10_000, "{nodes} nodes");
    // Word nests a table cell about this deep, and each group costs two
    // segments of path: the item and the run of items under it.
    assert!((8..=16).contains(&deepest), "deepest path {deepest}");
}
