use super::super::{Computed, EvalError, Evaluator, Value};
use crate::document::Document;
use crate::source::MemSource;
use crate::template::{Anchor, Endian::Big, Expr as E, Step, Template, Ty as T};

fn doc(bytes: &[u8]) -> Document<MemSource> {
    Document::new(MemSource(bytes.to_vec()))
}

/// Computed text is kept on its node like a computed number, and goes
/// when an edit could have changed what it read: an edit after it leaves
/// it, and an edit to the text it copies drops it and it reads again.
#[test]
fn computed_text_is_kept_until_an_edit_could_change_it() {
    let t = Template::new(
        "t",
        T::structure("S", vec![("name", T::utf8(E::lit(3))), ("copy", T::computed_text(E::field("name"))), ("tail", T::u8())]),
    );
    let mut d = Document::new(MemSource(b"abc!".to_vec()));
    let mut ev = Evaluator::new(t);
    let kept = |ev: &Evaluator| match ev.memo.get(&[1]).and_then(|r| r.computed.clone()) {
        Some(Computed::Text(s)) => Some(s.to_string()),
        _ => None,
    };
    assert_eq!(kept(&ev), None);
    assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::Str("abc".into()));
    assert_eq!(kept(&ev).as_deref(), Some("abc"));

    d.overwrite_bits(3 * 8, b"?", 8);
    ev.invalidate_from(3 * 8);
    assert_eq!(kept(&ev).as_deref(), Some("abc"));
    assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::Str("abc".into()));

    d.overwrite_bits(0, b"x", 8);
    ev.invalidate_from(0);
    assert_eq!(kept(&ev), None);
    assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::Str("xbc".into()));
    assert_eq!(kept(&ev).as_deref(), Some("xbc"));
}

/// Check that `ev`, which read the file before the edit and was then told
/// about it, answers each of `paths` the way an evaluator reading the edited
/// bytes for the first time does.
fn agrees_with_a_fresh_read(ev: &mut Evaluator, d: &Document<MemSource>, t: &Template, paths: &[&[usize]]) {
    assert_eq!(ev.memo.without_parent(), Vec::<Vec<usize>>::new());
    let mut fresh = Evaluator::new(t.clone());
    for path in paths {
        assert_eq!(ev.node(d, path), fresh.node(d, path), "{path:?}");
    }
}

/// Check that asking for each of `paths` again takes no walk: with an
/// allowance of nothing, the first element a walk reaches stops it.
fn answered_from_memory(ev: &mut Evaluator, d: &Document<MemSource>, paths: &[&[usize]]) {
    ev.set_slice(Some(0));
    ev.begin_slice();
    for path in paths {
        let got = ev.node(d, path);
        assert!(!matches!(got, Err(EvalError::Busy { .. })), "{path:?} was walked to again");
    }
    ev.set_slice(None);
    ev.begin_slice();
}

/// A record: where the next one is, and a byte of its own.
fn linked() -> T {
    T::structure("Rec", vec![("next", T::u16(Big)), ("value", T::u8())])
}

fn chain() -> Template {
    Template::new(
        "t",
        T::structure("Root", vec![("head", T::u16(Big)), ("recs", T::chain(E::field("head"), &["next"], Anchor::File, linked()))]),
    )
}

/// The header points at the record at 2, which points at the one at 8, which
/// points back at the one at 5, which ends the chain. Nothing points at the
/// record at 11 yet, and nothing reads the two bytes after it.
fn chain_bytes() -> Vec<u8> {
    vec![0, 2, /* @2 */ 0, 8, 0xaa, /* @5 */ 0, 0, 0xbb, /* @8 */ 0, 5, 0xcc, /* @11 */ 0, 5, 0xdd, /* @14 */ 0xee, 0xee]
}

#[test]
fn an_overwrite_of_a_link_drops_the_elements_it_placed() {
    let t = chain();
    let mut d = doc(&chain_bytes());
    let mut ev = Evaluator::new(t.clone());
    assert_eq!(ev.node(&d, &[1]).unwrap().child_count, 3);
    assert_eq!(ev.node(&d, &[1, 2]).unwrap().offset_bits, 5 * 8);
    assert_eq!(ev.node(&d, &[1, 2, 1]).unwrap().value, Value::UInt(0xbb));

    // The second record points at the one at 11 now. The third element ended
    // before the edit, but where it is was read from after it.
    d.overwrite_bytes(8, &[0, 11]);
    ev.invalidate_from(8 * 8);
    // The first element was placed from the header, which is before the edit.
    assert!(ev.memo.contains_key(&[1, 0]));
    agrees_with_a_fresh_read(&mut ev, &d, &t, &[&[1], &[1, 0], &[1, 1], &[1, 2], &[1, 2, 1], &[1, 3], &[1, 3, 1]]);
    assert_eq!(ev.node(&d, &[1, 2]).unwrap().offset_bits, 11 * 8);
    assert_eq!(ev.node(&d, &[1]).unwrap().child_count, 4);
}

#[test]
fn an_overwrite_after_a_chain_and_its_links_keeps_the_walk() {
    let t = chain();
    let mut d = doc(&chain_bytes());
    let mut ev = Evaluator::new(t.clone());
    let everything: &[&[usize]] = &[&[1], &[1, 0, 1], &[1, 1, 1], &[1, 2, 1]];
    for path in everything {
        ev.node(&d, path).unwrap();
    }
    let held = ev.memo_len();

    // A byte nothing reads.
    d.overwrite_bytes(14, &[0x11]);
    ev.invalidate_from(14 * 8);
    assert_eq!(ev.memo_len(), held);
    assert_eq!((ev.memo.list(&[1]).chain_starts.len(), ev.memo.list(&[1]).chain_done), (3, true));
    answered_from_memory(&mut ev, &d, everything);
    agrees_with_a_fresh_read(&mut ev, &d, &t, everything);
}

#[test]
fn an_overwrite_inside_a_chain_element_keeps_what_the_links_placed() {
    let t = chain();
    let mut d = doc(&chain_bytes());
    let mut ev = Evaluator::new(t.clone());
    let everything: &[&[usize]] = &[&[1], &[1, 0, 1], &[1, 1, 1], &[1, 2, 1]];
    for path in everything {
        ev.node(&d, path).unwrap();
    }

    // The second record's own byte. Every link is outside it, so the chain
    // stands, and what goes is the record the edit is in.
    d.overwrite_bytes(10, &[0x77]);
    ev.invalidate_from(10 * 8);
    for kept in [&[1, 0][..], &[1, 0, 1], &[1, 1, 0], &[1, 2], &[1, 2, 1]] {
        assert!(ev.memo.contains_key(kept), "{kept:?}");
    }
    assert!(!ev.memo.contains_key(&[1, 1, 1]));
    assert_eq!((ev.memo.list(&[1]).chain_starts.len(), ev.memo.list(&[1]).chain_done), (3, true));
    answered_from_memory(&mut ev, &d, everything);
    agrees_with_a_fresh_read(&mut ev, &d, &t, everything);
    assert_eq!(ev.node(&d, &[1, 1, 1]).unwrap().value, Value::UInt(0x77));
}

/// A record that places one gathered element: where it is, and how long.
fn placing() -> T {
    T::structure("Rec", vec![("off", T::u8()), ("len", T::u8())])
}

/// A byte saying where the records are, then a region of seven bytes they
/// place their elements in, with the records after the region: the way an
/// Arrow footer places the batches before it.
fn records_after() -> Template {
    let recs = T::structure("Recs", vec![("n", T::u8()), ("list", T::array(placing(), E::field("n")))]);
    let from = vec![Step::field("recs"), Step::field("list"), Step::each()];
    let region = T::gather(from, E::field("off"), Anchor::File, E::lit(0), T::bytes(E::placer(E::field("len"))));
    Template::new(
        "t",
        T::structure("Root", vec![("at", T::u8()), ("recs", T::at(E::field("at"), recs)), ("region", T::sized(E::lit(7), region))]),
    )
}

/// Two records at 8, placing two bytes at 1 and one at 4, and two bytes after
/// them that nothing reads.
fn records_after_bytes() -> Vec<u8> {
    vec![8, /* region */ 0xa0, 0xa1, 0, 0xb0, 0, 0xc0, 0, /* @8 */ 2, 1, 2, 4, 1, /* @13 */ 0xee, 0xee]
}

#[test]
fn an_overwrite_of_a_record_drops_what_the_gather_placed_from_it() {
    let t = records_after();
    let mut d = doc(&records_after_bytes());
    let mut ev = Evaluator::new(t.clone());
    let everything: &[&[usize]] = &[&[2], &[2, 0], &[2, 1]];
    for path in everything {
        ev.node(&d, path).unwrap();
    }
    assert_eq!(ev.node(&d, &[2, 1]).unwrap().offset_bits, 4 * 8);

    // The second record places its byte at 6 now. The element it placed ended
    // before the edit, but where it is was read from after it.
    d.overwrite_bytes(11, &[6]);
    ev.invalidate_from(11 * 8);
    // The first record ended before the edit, and so did everything the walk
    // read on the way to it.
    assert!(ev.memo.contains_key(&[2, 0]));
    agrees_with_a_fresh_read(&mut ev, &d, &t, everything);
    assert_eq!(ev.node(&d, &[2, 1]).unwrap().offset_bits, 6 * 8);

    // The first record's element is one byte long now. How long an element is
    // was read from its record too.
    d.overwrite_bytes(10, &[1]);
    ev.invalidate_from(10 * 8);
    agrees_with_a_fresh_read(&mut ev, &d, &t, everything);
    assert_eq!(ev.node(&d, &[2, 0]).unwrap().size_bits, 8);
}

#[test]
fn an_overwrite_after_a_gather_and_its_records_keeps_the_walk() {
    let t = records_after();
    let mut d = doc(&records_after_bytes());
    let mut ev = Evaluator::new(t.clone());
    let everything: &[&[usize]] = &[&[2], &[2, 0], &[2, 1]];
    for path in everything {
        ev.node(&d, path).unwrap();
    }
    let held = ev.memo_len();

    // A byte nothing reads.
    d.overwrite_bytes(13, &[0x11]);
    ev.invalidate_from(13 * 8);
    assert_eq!(ev.memo_len(), held);
    assert!(ev.memo.list(&[2]).gather.as_ref().is_some_and(|g| g.done));
    answered_from_memory(&mut ev, &d, everything);
    agrees_with_a_fresh_read(&mut ev, &d, &t, everything);
}

/// Two records, then the region they place their elements in: a byte at 6
/// and two at 4, and a byte no record places.
fn records_before() -> Template {
    let from = vec![Step::field("list"), Step::each()];
    let region = T::gather(from, E::field("off"), Anchor::File, E::lit(0), T::bytes(E::placer(E::field("len"))));
    Template::new("t", T::structure("Root", vec![("list", T::array(placing(), E::lit(2))), ("region", T::sized(E::Remaining, region))]))
}

#[test]
fn an_overwrite_inside_a_gathered_element_keeps_the_walk() {
    let t = records_before();
    let mut d = doc(&[6, 1, 4, 2, /* @4 */ 0xa0, 0xa1, /* @6 */ 0xb0, /* @7 */ 0xee]);
    let mut ev = Evaluator::new(t.clone());
    let everything: &[&[usize]] = &[&[1], &[1, 0], &[1, 1]];
    for path in everything {
        ev.node(&d, path).unwrap();
    }

    // The first element's own byte. Every record is before it, so what the
    // walk found stands, and what goes is the element the edit is in.
    d.overwrite_bytes(6, &[0x55]);
    ev.invalidate_from(6 * 8);
    assert!(ev.memo.contains_key(&[1, 1]));
    assert!(!ev.memo.contains_key(&[1, 0]));
    assert!(ev.memo.list(&[1]).gather.as_ref().is_some_and(|g| g.done));
    answered_from_memory(&mut ev, &d, everything);
    agrees_with_a_fresh_read(&mut ev, &d, &t, everything);
}

#[test]
fn an_overwrite_that_moves_a_gather_walks_to_its_records_again() {
    // Two records, a length and that many bytes, then a region whose elements
    // are placed from its own start. The records are before the length, but
    // where the region starts is not.
    let from = vec![Step::field("list"), Step::each()];
    let region = T::gather(from, E::field("off"), Anchor::SelfAligned(1), E::lit(0), T::bytes(E::placer(E::field("len"))));
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("list", T::array(placing(), E::lit(2))),
                ("pad", T::u8()),
                ("skip", T::bytes(E::field("pad"))),
                ("region", T::sized(E::Remaining, region)),
            ],
        ),
    );
    let mut d = doc(&[0, 1, 2, 1, /* pad */ 1, 0xee, /* @6 */ 0xa0, 0xa1, 0xb0, 0xb1, 0xc0]);
    let mut ev = Evaluator::new(t.clone());
    let everything: &[&[usize]] = &[&[3], &[3, 0], &[3, 1]];
    for path in everything {
        ev.node(&d, path).unwrap();
    }
    assert_eq!(ev.node(&d, &[3, 1]).unwrap().offset_bits, 8 * 8);

    // One more byte to skip moves the region, and every element with it.
    d.overwrite_bytes(4, &[2]);
    ev.invalidate_from(4 * 8);
    assert!(ev.memo.list(&[3]).gather.is_none());
    agrees_with_a_fresh_read(&mut ev, &d, &t, everything);
    assert_eq!(ev.node(&d, &[3, 1]).unwrap().offset_bits, 9 * 8);
}

/// A block of a chain: records, then where the next block is.
fn block() -> T {
    T::structure("Block", vec![("n", T::u8()), ("recs", T::array(placing(), E::field("n"))), ("next", T::u8())])
}

/// A chain of blocks holding records, and a region of three bytes the records
/// place their elements in.
fn gathered_through_a_chain() -> Template {
    let from = vec![Step::field("blocks"), Step::each(), Step::field("recs"), Step::each()];
    let region = T::gather(from, E::field("off"), Anchor::File, E::lit(0), T::bytes(E::placer(E::field("len"))));
    Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("head", T::u8()),
                ("blocks", T::chain(E::field("head"), &["next"], Anchor::File, block())),
                ("region", T::sized(E::lit(3), region)),
            ],
        ),
    )
}

#[test]
fn an_overwrite_of_a_link_moves_what_a_gather_found_along_the_chain() {
    let t = gathered_through_a_chain();
    // The header points at the block at 8, which points back at the one at 4,
    // which ends the chain. They place a byte at 1 and a byte at 2. The block
    // at 12 would place one at 3, if anything pointed at it.
    let mut d = doc(&[8, /* region */ 0xa0, 0xb0, 0xc0, /* @4 */ 1, 2, 1, 0, /* @8 */ 1, 1, 1, 4, /* @12 */ 1, 3, 1, 0]);
    let mut ev = Evaluator::new(t.clone());
    let everything: &[&[usize]] = &[&[1], &[1, 1], &[2], &[2, 0], &[2, 1]];
    for path in everything {
        ev.node(&d, path).unwrap();
    }
    assert_eq!(ev.node(&d, &[2, 1]).unwrap().offset_bits, 2 * 8);

    // The block at 8 points at the one at 12 now. The record that placed the
    // second element ended before the edit, and so did the block it is in,
    // but where that block is was read from after it.
    d.overwrite_bytes(11, &[12]);
    ev.invalidate_from(11 * 8);
    // The first block's record ended before the edit, and nothing read on the
    // way to it was after it.
    assert!(ev.memo.contains_key(&[2, 0]));
    agrees_with_a_fresh_read(&mut ev, &d, &t, everything);
    assert_eq!(ev.node(&d, &[2, 1]).unwrap().offset_bits, 3 * 8);
}

/// A file from the sample collection, when there is one to read.
fn sample(name: &str) -> Option<Vec<u8>> {
    let dir = std::env::var_os("QUBERO_SAMPLES")?;
    std::fs::read(std::path::Path::new(&dir).join(name)).ok()
}

/// Every node of a file, as far as a few thousand of them and a couple of
/// thousand children a node go, in the order a reader opening each in turn
/// reaches them. A node that does not read is a leaf.
fn every_path(ev: &mut Evaluator, d: &Document<MemSource>) -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    let mut stack = vec![Vec::new()];
    while let Some(path) = stack.pop() {
        if out.len() >= 20_000 {
            break;
        }
        let children = ev.node(d, &path).map_or(0, |n| n.child_count.min(2_000));
        for i in (0..children as usize).rev() {
            stack.push([&path[..], &[i]].concat());
        }
        out.push(path);
    }
    out
}

/// `agrees_with_a_fresh_read` over every node a fresh read of the edited
/// bytes finds.
fn agrees_everywhere(ev: &mut Evaluator, d: &Document<MemSource>, t: &Template) {
    let paths = every_path(&mut Evaluator::new(t.clone()), d);
    let paths: Vec<&[usize]> = paths.iter().map(Vec::as_slice).collect();
    agrees_with_a_fresh_read(ev, d, t, &paths);
}

/// The child of the node at `path` with this name.
fn child(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], name: &str) -> Vec<usize> {
    let n = ev.node(d, path).unwrap().child_count as usize;
    for i in 0..n {
        let p = [path, &[i]].concat();
        if ev.node(d, &p).unwrap().name == name {
            return p;
        }
    }
    panic!("nothing called {name} in {path:?}")
}

#[test]
fn an_overwrite_of_a_link_in_an_hdf4_file_agrees_with_a_fresh_read() {
    let t = crate::formats::builtin("hdf4").unwrap();
    // Files of three descriptor blocks, written by the HDF4 library.
    for name in ["hdf4/litend.hdf", "hdf4/ntcheck.hdf", "hdf4/tvattr.hdf"] {
        let Some(bytes) = sample(name) else { continue };
        let mut d = doc(&bytes);
        let mut ev = Evaluator::new(t.clone());
        every_path(&mut ev, &d);
        // The second block's link, which the chain of blocks and the chain of
        // their tables both follow, and the index gathers its entries along.
        // Nought ends both chains there.
        let blocks = child(&mut ev, &d, &[], "blocks");
        assert_eq!(ev.node(&d, &blocks).unwrap().child_count, 3, "{name}");
        let next = child(&mut ev, &d, &[&blocks[..], &[1]].concat(), "next");
        let at = ev.node(&d, &next).unwrap().offset_bits;
        d.overwrite_bytes(at / 8, &[0, 0, 0, 0]);
        ev.invalidate_from(at);
        agrees_everywhere(&mut ev, &d, &t);
        assert_eq!(ev.node(&d, &blocks).unwrap().child_count, 2, "{name}");
    }
}

#[test]
fn an_overwrite_of_an_arrow_footer_moves_the_batches_it_places() {
    let Some(bytes) = sample("arrow/columns-uncompressed.arrow") else { return };
    let t = crate::formats::builtin("arrow").unwrap();
    let mut d = doc(&bytes);
    let mut ev = Evaluator::new(t.clone());
    every_path(&mut ev, &d);
    let batches = child(&mut ev, &d, &[], "batches");
    let element = |i: usize| [&batches[..], &[i]].concat();
    let was: Vec<u64> = (0..2).map(|i| ev.node(&d, &element(i)).unwrap().offset_bits).collect();

    // The footer's first two blocks, each saying where a batch is and how
    // long, trade places. The footer is after every batch, so the batches
    // ended before the edit, and the first two trade places too.
    let blocks: Vec<(usize, usize)> = (0..2)
        .map(|i| {
            let record = ev.gathered_record(&d, &batches, i).unwrap();
            let n = ev.node(&d, &record).unwrap();
            ((n.offset_bits / 8) as usize, (n.size_bits / 8) as usize)
        })
        .collect();
    let ((a, len), (b, other)) = (blocks[0], blocks[1]);
    assert_eq!(len, other);
    assert!(a.min(b) as u64 * 8 > was[0].max(was[1]));
    d.overwrite_bytes(a as u64, &bytes[b..b + len]);
    d.overwrite_bytes(b as u64, &bytes[a..a + len]);
    ev.invalidate_from(a.min(b) as u64 * 8);
    agrees_everywhere(&mut ev, &d, &t);
    let now: Vec<u64> = (0..2).map(|i| ev.node(&d, &element(i)).unwrap().offset_bits).collect();
    assert_eq!(now, vec![was[1], was[0]]);
}

#[test]
fn an_overwrite_in_a_fits_heap_keeps_the_walk_to_its_descriptors() {
    let Some(bytes) = sample("fits/rice.fits") else { return };
    let t = crate::formats::builtin("fits").unwrap();
    let mut d = doc(&bytes);
    let mut ev = Evaluator::new(t.clone());
    let paths = every_path(&mut ev, &d);
    // The first heap, and the first array in it.
    let heap = paths.iter().find(|p| ev.node(&d, p).unwrap().type_name.starts_with("descriptors")).unwrap().clone();
    let at = ev.node(&d, &[&heap[..], &[0]].concat()).unwrap().offset_bits / 8;

    // A byte of that array. Every descriptor is in the rows before the heap,
    // so the walk to them stands, and nothing has to walk to them again.
    d.overwrite_bytes(at, &[!bytes[at as usize]]);
    ev.invalidate_from(at * 8);
    assert!(ev.memo.list(&heap).gather.as_ref().is_some_and(|g| g.done));
    answered_from_memory(&mut ev, &d, &[&heap]);
    agrees_everywhere(&mut ev, &d, &t);
}
