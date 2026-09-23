//! A real SQLite database with a freelist: `sqlite/page512-overflow-freelist.db`
//! from the sample collection, written by `tools/make_sqlite_samples.py`. One
//! table of twelve rows, four of them deleted, 512-byte pages, and every row
//! too big for its page.
//!
//! What makes it worth a test is what the DELETE left behind. Four of the
//! pages it freed were table leaves, and SQLite does not rewrite a page it
//! frees, so they still start with the table-leaf type byte. Read by that byte
//! they add four leaves and four rows to a table `sqlite3` reads as five
//! leaves and eight rows. The numbers below are what `sqlite3` and a walk of
//! the B-tree from its root page say.

use std::collections::{BTreeMap, BTreeSet};

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

/// The one freelist trunk, and the 26 pages it lists, in the order it lists
/// them, which is the order the DELETE freed them.
const TRUNK: u32 = 9;
const FREE_LEAVES: [u32; 26] =
    [10, 11, 12, 24, 25, 26, 27, 28, 35, 43, 44, 45, 46, 47, 48, 49, 57, 66, 67, 68, 69, 70, 71, 72, 73, 74];

fn sample() -> Option<Vec<u8>> {
    let path = qubero_samples::dir("sqlite")?.join("page512-overflow-freelist.db");
    path.is_file().then(|| std::fs::read(&path).unwrap())
}

fn open() -> Option<(Document<MemSource>, Evaluator)> {
    let Some(bytes) = sample() else {
        eprintln!("{}", qubero_samples::missing_dir("sqlite"));
        return None;
    };
    Some((Document::new(MemSource(bytes)), Evaluator::new(formats::sqlite())))
}

fn child(ev: &mut Evaluator, doc: &Document<MemSource>, at: &[usize], name: &str) -> Vec<usize> {
    ev.child_named(doc, at, name).unwrap().unwrap_or_else(|| panic!("no {name} under {at:?}"))
}

fn uint(ev: &mut Evaluator, doc: &Document<MemSource>, at: &[usize]) -> u32 {
    match ev.node(doc, at).unwrap().value {
        Value::UInt(n) => n as u32,
        Value::Int(n) => n as u32,
        other => panic!("{at:?} is {other:?}, not a number"),
    }
}

/// Every page from 2 on, by the type it read as. Page 1 is the schema.
fn pages_by_type(ev: &mut Evaluator, doc: &Document<MemSource>) -> BTreeMap<String, Vec<u32>> {
    let pages = child(ev, doc, &[], "pages");
    let n = ev.node(doc, &pages).unwrap().child_count as usize;
    assert_eq!(n, 73, "74 pages, less page 1");
    let mut out: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    for i in 0..n {
        let page = [pages.as_slice(), &[i]].concat();
        out.entry(ev.node(doc, &page).unwrap().type_name).or_default().push(i as u32 + 2);
    }
    out
}

#[test]
fn a_page_on_the_freelist_is_free_whatever_its_first_byte_says() {
    let Some((doc, mut ev)) = open() else { return };
    let got = pages_by_type(&mut ev, &doc);
    let leaves: Vec<u32> = BTreeSet::from(FREE_LEAVES).into_iter().collect();
    assert_eq!(got["TableInterior"], [2]);
    // Not 35, 49, 57 and 74, which start with 13 and are on the freelist.
    assert_eq!(got["TableLeaf"], [17, 18, 29, 42, 65]);
    assert_eq!(got["FreelistTrunk"], [TRUNK]);
    assert_eq!(got["FreelistLeaf"], leaves);
    // Everything else is an overflow page, and nothing is left as bytes.
    assert_eq!(got["Overflow"].len(), 40);
    assert_eq!(got.len(), 5, "{:?}", got.keys().collect::<Vec<_>>());

    // Eight rows, which is what `SELECT` finds.
    let pages = child(&mut ev, &doc, &[], "pages");
    let mut rows = 0;
    for p in &got["TableLeaf"] {
        let page = [pages.as_slice(), &[*p as usize - 2]].concat();
        let cells = child(&mut ev, &doc, &page, "cells");
        rows += ev.node(&doc, &cells).unwrap().child_count;
    }
    assert_eq!(rows, 8);
}

#[test]
fn the_trunk_reads_as_the_pages_it_lists() {
    let Some((doc, mut ev)) = open() else { return };
    let freelist = child(&mut ev, &doc, &[], "freelist");
    assert_eq!(ev.node(&doc, &freelist).unwrap().child_count, 1);
    let trunk = [freelist.as_slice(), &[0]].concat();
    // Placed at page 9, found from the header's pointer to it.
    assert_eq!(ev.node(&doc, &trunk).unwrap().offset_bits, u64::from(TRUNK - 1) * 512 * 8);
    let next = child(&mut ev, &doc, &trunk, "next_trunk");
    assert_eq!(uint(&mut ev, &doc, &next), 0);
    let count = child(&mut ev, &doc, &trunk, "leaf_count");
    assert_eq!(uint(&mut ev, &doc, &count), 26);
    let list = child(&mut ev, &doc, &trunk, "leaves");
    let listed: Vec<u32> = (0..26)
        .map(|i| {
            let entry = [list.as_slice(), &[i]].concat();
            let page = child(&mut ev, &doc, &entry, "page");
            uint(&mut ev, &doc, &page)
        })
        .collect();
    assert_eq!(listed, FREE_LEAVES);

    // The same 26, as the one list every page is looked up in.
    let free = child(&mut ev, &doc, &[], "free_pages");
    assert_eq!(ev.node(&doc, &free).unwrap().child_count, 26);
    let flat: Vec<u32> = (0..26).map(|i| uint(&mut ev, &doc, &[free.as_slice(), &[i]].concat())).collect();
    assert_eq!(flat, FREE_LEAVES);

    // The trunk is also page 9 of the run, and reads the same there.
    let pages = child(&mut ev, &doc, &[], "pages");
    let page = [pages.as_slice(), &[TRUNK as usize - 2]].concat();
    let count = child(&mut ev, &doc, &page, "leaf_count");
    assert_eq!(uint(&mut ev, &doc, &count), 26);
}

/// The template types an overflow page by ruling everything else out, since
/// it cannot see the cell that points at it. The rows can: following every
/// live row's chain has to land on exactly the pages the template called
/// overflow pages, no more and no fewer.
#[test]
fn the_overflow_pages_are_the_ones_the_live_rows_reach() {
    let Some((doc, mut ev)) = open() else { return };
    let got = pages_by_type(&mut ev, &doc);
    let pages = child(&mut ev, &doc, &[], "pages");
    let mut reached = BTreeSet::new();
    for p in &got["TableLeaf"] {
        let page = [pages.as_slice(), &[*p as usize - 2]].concat();
        let cells = child(&mut ev, &doc, &page, "cells");
        for c in 0..ev.node(&doc, &cells).unwrap().child_count as usize {
            let cell = [cells.as_slice(), &[c]].concat();
            let found = formats::sqlite_payload(&mut ev, &doc, &cell).unwrap();
            assert!(found.complete(), "page {p} cell {c}: {:?}", found.problem);
            for q in found.pages {
                assert!(reached.insert(q), "page {q} is on two rows' chains");
            }
        }
    }
    assert_eq!(reached.into_iter().collect::<Vec<_>>(), got["Overflow"]);
}
