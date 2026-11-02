//! The Blackfin boot stream inside Tascam's DR-05X V1.16 firmware update,
//! read with the `bfldr` template and checked against a walk of it by hand.
//!
//! The update file is a 32-byte header, the scrambled stream, and 4 more
//! scrambled bytes. This test undoes the scrambling with Tascam's table and
//! reads the stream as a file of its own, the way `bfldr` would read an
//! `.ldr` file.
//!
//! The file lives in the sample collection rather than here. Point
//! `QUBERO_SAMPLES` at it, or keep it beside the repository as
//! `qubero-samples`. With neither, the test says so and passes.

use std::collections::BTreeMap;
use std::time::Instant;

use qubero_core::codec::tascam::UNSCRAMBLE;
use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, NodeInfo, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

const FILE: &str = "DR05X_071.116";

/// The update file's header, before the scrambled stream.
const UPDATE_HEADER_BYTES: usize = 32;

/// The stream and the 0xff after it: the update file less its header and the
/// 4 bytes at the end.
const STREAM_BYTES: u64 = 0x1f_2ffc;

/// Where the final block ends, and the erased flash starts.
const STREAM_END: u64 = 0x1f_2ab0;

/// The unscrambled stream, or nothing when the sample collection is not to
/// hand.
fn stream() -> Option<Vec<u8>> {
    let path = qubero_samples::roots().into_iter().map(|r| r.join("firmware").join("tascam").join(FILE)).find(|p| p.exists());
    let Some(path) = path else {
        eprintln!("{}", qubero_samples::missing());
        return None;
    };
    let bytes = std::fs::read(path).unwrap();
    Some(bytes[UPDATE_HEADER_BYTES..bytes.len() - 4].iter().map(|&b| UNSCRAMBLE[b as usize]).collect())
}

/// The node a path of names reaches. A number steps to that element of a list.
fn named(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], names: &[&str]) -> Vec<usize> {
    let mut p = path.to_vec();
    for name in names {
        if let Ok(i) = name.parse::<usize>() {
            p.push(i);
            continue;
        }
        p = ev.child_named(d, &p, name).unwrap().unwrap_or_else(|| panic!("no {name} under {p:?}"));
    }
    p
}

fn node(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], names: &[&str]) -> NodeInfo {
    let p = named(ev, d, path, names);
    ev.node(d, &p).unwrap()
}

fn int(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], names: &[&str]) -> i128 {
    let v = node(ev, d, path, names).value;
    v.as_int().unwrap_or_else(|| panic!("{names:?} is not a number: {v:?}"))
}

/// Offset and size in bytes.
fn extent(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], names: &[&str]) -> (u64, u64) {
    let n = node(ev, d, path, names);
    (n.offset_bits / 8, n.size_bits / 8)
}

fn holds(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], names: &[&str]) -> bool {
    let p = named(ev, d, path, names);
    ev.valid_of(d, &p).unwrap().unwrap_or_else(|| panic!("{names:?} has no constraint")).ok
}

/// The names of the flags set in a block's header.
fn flags(ev: &mut Evaluator, d: &Document<MemSource>, block: &[usize]) -> Vec<String> {
    match node(ev, d, block, &["header", "flags"]).value {
        Value::Flags { set, .. } => set,
        other => panic!("not flags: {other:?}"),
    }
}

#[test]
fn the_dr05x_stream_reads_as_an_initcode_and_an_application() {
    let Some(bytes) = stream() else { return };
    assert_eq!(bytes.len() as u64, STREAM_BYTES);
    let head = &bytes[..bytes.len().min(formats::SNIFF_WINDOW)];
    assert_eq!(formats::sniff(head, STREAM_BYTES), Some("bfldr"));
    let d = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(formats::template("bfldr").expect("the bfldr template"));
    let ev = &mut ev;
    let d = &d;
    assert_eq!(ev.node(d, &[]).unwrap().size_bits, STREAM_BYTES * 8);
    let programs = named(ev, d, &[], &["programs"]);
    assert_eq!(ev.node(d, &programs).unwrap().child_count, 2);

    // The initcode: a first block whose argument reaches the next first
    // block, two blocks of code and data, and an init block that runs it.
    let init = [&programs[..], &[0]].concat();
    assert_eq!(extent(ev, d, &init, &[]), (0, 0x748));
    let first = named(ev, d, &init, &["first_block"]);
    assert_eq!(flags(ev, d, &first), ["ignore", "first"]);
    assert_eq!(int(ev, d, &init, &["first_block", "header", "argument"]), 0x738);
    assert_eq!(int(ev, d, &init, &["first_block", "header", "target_address"]), 0xffa0_0000);
    assert!(holds(ev, d, &init, &["first_block"]), "the first block's argument is the length of the blocks after it");
    let blocks = named(ev, d, &init, &["blocks"]);
    assert_eq!(ev.node(d, &blocks).unwrap().child_count, 3);
    let data = [&blocks[..], &[0]].concat();
    assert_eq!(extent(ev, d, &data, &[]), (0x10, 16 + 16));
    assert_eq!(int(ev, d, &data, &["header", "target_address"]), 0xff80_4000);
    let code = [&blocks[..], &[1]].concat();
    assert_eq!(extent(ev, d, &code, &["payload"]), (0x40, 1784));
    assert_eq!(int(ev, d, &code, &["header", "target_address"]), 0xffa0_0000);
    let run = [&blocks[..], &[2]].concat();
    assert_eq!(extent(ev, d, &run, &[]), (0x738, 16));
    assert_eq!(flags(ev, d, &run), ["init"]);
    assert!(node(ev, d, &init, &["after_last_block"]).absent);

    // The application, from the second first block to the final block.
    let app = [&programs[..], &[1]].concat();
    assert_eq!(extent(ev, d, &app, &[]), (0x748, STREAM_END - 0x748));
    assert_eq!(int(ev, d, &app, &["first_block", "header", "argument"]), 0x1f_2358);
    assert!(holds(ev, d, &app, &["first_block"]));
    let blocks = named(ev, d, &app, &["blocks"]);
    let count = ev.node(d, &blocks).unwrap().child_count;
    assert_eq!(count, 7826 - 2 - 3);
    let last = [&blocks[..], &[count as usize - 1]].concat();
    assert_eq!(extent(ev, d, &last, &[]), (0x1f_2aa0, 16));
    assert_eq!(flags(ev, d, &last), ["final"]);
    assert_eq!(int(ev, d, &last, &["header", "target_address"]), 0xffa0_0000);
    assert!(node(ev, d, &app, &["after_last_block"]).absent);

    // After the stream, 1,356 bytes of erased flash and nothing else.
    assert!(node(ev, d, &[], &["blocks"]).absent);
    assert_eq!(extent(ev, d, &[], &["erased"]), (STREAM_END, 1356));
    assert!(node(ev, d, &[], &["rest"]).absent);
}

/// Every node under `info`, depth first, with what is wrong with any of them
/// and how many blocks have each set of flags.
fn walk(ev: &mut Evaluator, d: &Document<MemSource>, info: &NodeInfo, kinds: &mut BTreeMap<String, usize>, flagged: &mut Vec<String>) {
    if let Some(p) = &info.problem {
        flagged.push(format!("{:?} {} at 0x{:x}: {}", info.path, info.name, info.offset_bits / 8, p.text));
    }
    if info.name == "flags" {
        if let Value::Flags { set, .. } = &info.value {
            *kinds.entry(set.join("|")).or_default() += 1;
        }
    }
    if info.child_count > 0 {
        for child in ev.children(d, &info.path, 0, info.child_count).unwrap() {
            walk(ev, d, &child, kinds, flagged);
        }
    }
}

#[test]
fn nothing_in_the_dr05x_stream_is_flagged_as_a_wrong_value() {
    let Some(bytes) = stream() else { return };
    let d = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(formats::template("bfldr").expect("the bfldr template"));
    let root = ev.node(&d, &[]).unwrap();
    let mut kinds = BTreeMap::new();
    let mut flagged = Vec::new();
    walk(&mut ev, &d, &root, &mut kinds, &mut flagged);
    assert!(flagged.is_empty(), "{} flagged, first: {:?}", flagged.len(), &flagged[..flagged.len().min(5)]);
    // 3,834 blocks of code or data, 3,988 fill blocks, and four with other
    // flags.
    let want: BTreeMap<String, usize> =
        [("", 3834), ("fill", 3988), ("ignore|first", 2), ("init", 1), ("final", 1)].into_iter().map(|(k, n)| (k.to_string(), n)).collect();
    assert_eq!(kinds, want);
    assert_eq!(ev.node(&d, &[]).unwrap().problems_within, (0, 0));
}

/// Every row of the stream down to the lists of blocks, which have to be
/// counted, and then every block's row. Prints how long each part took, but
/// asserts only the count of block rows, since the time depends on the
/// machine.
#[test]
fn the_dr05x_stream_lists_every_block_row() {
    let Some(bytes) = stream() else { return };
    let d = Document::new(MemSource(bytes));
    let mut ev = Evaluator::new(formats::template("bfldr").expect("the bfldr template"));
    let started = Instant::now();
    let mut rows = 0;
    let mut level = vec![ev.node(&d, &[]).unwrap()];
    for _ in 0..3 {
        let mut next = Vec::new();
        for n in &level {
            next.extend(ev.children(&d, &n.path, 0, n.child_count).unwrap());
        }
        rows += next.len();
        level = next;
    }
    let to_depth_3 = started.elapsed();
    let mut blocks = 0;
    for n in level.iter().filter(|n| n.name == "blocks") {
        blocks += ev.children(&d, &n.path, 0, n.child_count).unwrap().len();
    }
    eprintln!("{rows} rows to depth 3 in {to_depth_3:?}; {blocks} block rows by {:?}", started.elapsed());
    assert_eq!(blocks, 3 + 7821);
}
