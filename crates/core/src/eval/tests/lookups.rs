use super::*;

// ----- finding a sibling by a key this record works out -----

/// A stream of records where each one carries a class number, and what that
/// class is called is written in an earlier record of the same stream. This is
/// the shape GWF has, and the one no name can reach: the records are elements
/// of a list, not fields declared beside the field asking.
fn classed() -> Template {
    let record = T::structure(
        "Rec",
        vec![
            ("class", T::u8()),
            ("class_num", T::u8()),
            ("name", T::utf8(E::lit(4))),
            // The name of whichever earlier record numbered itself with this
            // record's class byte.
            ("of_class", T::Computed(E::sibling_tagged(&["class_num"], E::field("class"), &["class_num"]))),
        ],
    );
    Template::new("t", T::repeat(record, Until::End))
}

#[test]
fn a_record_finds_the_earlier_one_that_defines_its_class() {
    // Two definitions and then a record of class 9.
    let d = doc(b"\x00\x08dict\x00\x09trce\x09\x00data");
    let mut ev = Evaluator::new(classed());
    // The third record's class byte is 9, and the record that numbered itself
    // 9 is the second one.
    assert_eq!(ev.node(&d, &[2, 3]).unwrap().value.as_int(), Some(9));
    // Which is a connection, not a coincidence: the reader is pointed at the
    // element the answer came from, and at the byte that sent it there.
    let origins = ev.origins(&d, &[2, 3]).unwrap();
    let labels: Vec<&str> = origins.iter().map(|o| o.label.as_str()).collect();
    // Named for the list it was found in, which here is the whole file.
    assert_eq!(labels, vec!["class", "file[1].class_num"]);
    assert_eq!(origins[1].path, vec![1, 1]);
    // And the relationship is written out both ways.
    let rel = ev.relations(&d, &[2, 3]).unwrap();
    assert_eq!(rel[0].written, "earlier[class_num = class].class_num");
    assert_eq!(rel[0].substituted, "earlier[class_num = 9].class_num");
}

#[test]
fn a_lookup_that_finds_nothing_is_zero_rather_than_an_error() {
    // A record whose class nothing earlier defined. Zero, so `Or` can name
    // what to do without one, the same answer every other search here gives.
    let d = doc(b"\x07\x00data");
    let mut ev = Evaluator::new(classed());
    assert_eq!(ev.node(&d, &[0, 3]).unwrap().value.as_int(), Some(0));
}

#[test]
fn a_lookup_by_computed_key_reads_text_as_well_as_numbers() {
    // The same search, used to pick a type by the word an earlier record
    // holds rather than by a number. What a format that names its own record
    // types needs.
    let record = T::structure(
        "Rec",
        vec![
            ("class", T::u8()),
            ("class_num", T::u8()),
            ("name", T::utf8(E::lit(4))),
            (
                "body",
                T::matches(
                    E::sibling_tagged(&["class_num"], E::field("class"), &["name"]),
                    vec![("trce", T::u16(Big))],
                    T::bytes(E::lit(0)),
                ),
            ),
        ],
    );
    let d = doc(b"\x00\x08dict\x00\x09trce\x09\x00data\xbe\xef");
    let mut ev = Evaluator::new(Template::new("t", T::repeat(record, Until::End)));
    // The third record's class is 9, the record numbered 9 is called `trce`,
    // and a `trce` body is a sixteen-bit number.
    assert_eq!(ev.node(&d, &[2, 3]).unwrap().value, Value::UInt(0xbeef));
    // The first two are not, so their bodies are nothing at all.
    assert_eq!(ev.node(&d, &[0, 3]).unwrap().size_bits, 0);
}

/// A record: where the next one is, and a byte of its own.
fn linked() -> T {
    T::structure("Rec", vec![("next", T::u16(Big)), ("value", T::u8())])
}

#[test]
fn a_chain_of_pointers_is_a_flat_list() {
    // Three records, written in an order that has nothing to do with the order
    // they are read in: the header points at the last of them, which points
    // back at the first, which points at the middle one, which ends the chain.
    let t = T::structure(
        "Root",
        vec![("head", T::u16(Big)), ("recs", T::chain(E::field("head"), &["next"], Anchor::File, linked()))],
    );
    //          0..2 head=8      2..5 rec at 2      5..8 rec at 5     8..11 rec at 8
    let d = doc(&[0, 8, /* @2 */ 0, 5, 0xaa, /* @5 */ 0, 0, 0xbb, /* @8 */ 0, 2, 0xcc]);
    let mut ev = Evaluator::new(Template::new("t", t));
    let list = ev.node(&d, &[1]).unwrap();
    // Flat: three rows, not three levels.
    assert_eq!(list.child_count, 3);
    // The list itself covers no bytes; its elements are what cover them.
    assert_eq!(list.size_bits, 0);
    assert_eq!(list.type_name, "chain \u{2192} Rec");
    let value = |ev: &mut Evaluator, i: usize| ev.node(&d, &[1, i, 1]).unwrap().value.as_int();
    assert_eq!((value(&mut ev, 0), value(&mut ev, 1), value(&mut ev, 2)), (Some(0xcc), Some(0xaa), Some(0xbb)));
    // Read in the order the pointers give, not the order they sit in.
    assert_eq!(ev.node(&d, &[1, 0]).unwrap().offset_bits, 8 * 8);
    assert_eq!(ev.node(&d, &[1, 2]).unwrap().offset_bits, 5 * 8);
    // And the cursor finds them where they are, out of order and all. The
    // chain covers no bytes itself, so the search through the structure's
    // fields has to look inside it rather than at how long it is.
    assert_eq!(ev.locate(&d, 10 * 8).unwrap(), vec![1, 0, 1]);
    assert_eq!(ev.locate(&d, 3 * 8).unwrap(), vec![1, 1, 0]);
    assert_eq!(ev.locate(&d, 0).unwrap(), vec![0]);
}

#[test]
fn a_chain_stops_rather_than_going_round_for_ever() {
    let chain = |head: E| {
        T::structure("Root", vec![("head", T::u16(Big)), ("recs", T::chain(head, &["next"], Anchor::File, linked()))])
    };
    let count = |bytes: &[u8], head: E| {
        let d = doc(bytes);
        let mut ev = Evaluator::new(Template::new("t", chain(head)));
        ev.node(&d, &[1]).unwrap().child_count
    };
    // A record pointing at itself: one element, not for ever.
    assert_eq!(count(&[0, 2, 0, 2, 0xaa], E::field("head")), 1);
    // Two pointing at each other.
    assert_eq!(count(&[0, 2, 0, 5, 0xaa, 0, 2, 0xbb], E::field("head")), 2);
    // All ones for the width of the `next` field, which here is sixteen bits.
    assert_eq!(count(&[0, 2, 0xff, 0xff, 0xaa], E::field("head")), 1);
    // Past the end of the file.
    assert_eq!(count(&[0, 2, 0, 99, 0xaa], E::field("head")), 1);
    // A chain that starts nowhere is a list of nothing rather than an error.
    assert_eq!(count(&[0, 0, 0, 0, 0], E::field("head")), 0);
}

/// A chain whose offsets count from somewhere the bytes do not begin.
///
/// This is what a run of records unpacked out of the middle of a file needs:
/// every offset in a compressed CDF counts from the front of the file it was
/// before it was squeezed, and what comes out of the stream starts eight bytes
/// into that. The adjustment moves where each element is read and leaves the
/// tests that end the walk alone, so a nought still ends it.
#[test]
fn a_chain_reads_its_offsets_with_an_adjustment() {
    let t = T::structure(
        "Root",
        vec![
            ("head", T::u16(Big)),
            ("recs", T::chain_adjusted(E::field("head"), &["next"], Anchor::File, E::lit(-4), linked())),
        ],
    );
    // The offsets are written four too large: the head says 12 for the record
    // at 8, and that record says 6 for the one at 2.
    let d = doc(&[0, 12, /* @2 */ 0, 0, 0xaa, /* @5 */ 0, 0, 0, /* @8 */ 0, 6, 0xcc]);
    let mut ev = Evaluator::new(Template::new("t", t));
    assert_eq!(ev.node(&d, &[1]).unwrap().child_count, 2);
    assert_eq!(ev.node(&d, &[1, 0]).unwrap().offset_bits, 8 * 8);
    assert_eq!(ev.node(&d, &[1, 1]).unwrap().offset_bits, 2 * 8);
    assert_eq!(ev.node(&d, &[1, 1, 1]).unwrap().value, Value::UInt(0xaa));
    // A nought is still the end of the walk, and not the record four bytes
    // before the file: what the tests are made on is the offset as written.
    assert_eq!(ev.node(&d, &[1]).unwrap().child_count, 2);
}

/// A record that places one gathered element: where it is, and how long.
fn placing() -> T {
    T::structure("Rec", vec![("off", T::u8()), ("len", T::u8())])
}

/// Two groups of records, then a region the records place their elements in.
/// Each element is as long as the record that placed it says.
fn gathered(elem: T) -> T {
    let group = T::structure("Group", vec![("n", T::u8()), ("recs", T::array(placing(), E::field("n")))]);
    let from = vec![Step::field("groups"), Step::each(), Step::field("recs"), Step::each()];
    T::structure(
        "Root",
        vec![
            ("groups", T::array(group, E::lit(2))),
            ("region", T::sized(E::Remaining, T::gather(from, E::field("off"), Anchor::File, E::lit(0), elem))),
        ],
    )
}

/// Group 0 holds two records and group 1 one; the region runs from byte 8 to
/// byte 20, and the three elements sit in it out of order, with gaps between.
fn gathered_bytes() -> Vec<u8> {
    let mut b = vec![2, 14, 3, 10, 2, /* group 1 */ 1, 18, 1];
    //        8  9     10    11    12 13    14    15    16    17    18    19
    b.extend([0, 0, 0xa0, 0xa1, 0, 0, 0xb0, 0xb1, 0xb2, 0, 0xc0, 0]);
    b
}

#[test]
fn a_gather_places_elements_from_records_two_lists_deep() {
    let t = gathered(T::bytes(E::placer(E::field("len"))));
    let d = doc(&gathered_bytes());
    let mut ev = Evaluator::new(Template::new("t", t));
    let list = ev.node(&d, &[1]).unwrap();
    // Flat: one element per record, however deep the records were.
    assert_eq!(list.child_count, 3);
    assert_eq!(list.type_name, "descriptors \u{2192} bytes[]");
    // The region is what the window gave it, not what its elements come to.
    assert_eq!(list.size_bits, 12 * 8);
    // Numbered in the order the walk found the records, and placed where each
    // record said, which is not that order.
    let at = |ev: &mut Evaluator, i: usize| {
        let n = ev.node(&d, &[1, i]).unwrap();
        (n.offset_bits / 8, n.size_bits / 8)
    };
    assert_eq!((at(&mut ev, 0), at(&mut ev, 1), at(&mut ev, 2)), ((14, 3), (10, 2), (18, 1)));
    // The cursor finds each element where it is, and a byte of the region no
    // element claims is the region itself.
    assert_eq!(ev.locate(&d, 15 * 8).unwrap(), vec![1, 0]);
    assert_eq!(ev.locate(&d, 10 * 8).unwrap(), vec![1, 1]);
    assert_eq!(ev.locate(&d, 18 * 8).unwrap(), vec![1, 2]);
    assert_eq!(ev.locate(&d, 12 * 8).unwrap(), vec![1]);
    // A record is still where it was, and still a record.
    assert_eq!(ev.locate(&d, 3 * 8).unwrap(), vec![0, 0, 1, 1, 0]);
    assert_eq!(ev.shape(&d, &[1, 0]).unwrap().placed, Placed::Gathered);
    // The listing reads the region as the elements and the gaps between them.
    let spans = ev.spans(&d, 8 * 8, 20 * 8, 100).unwrap();
    let seen: Vec<(u64, u64, bool)> = spans.iter().map(|s| (s.offset_bits / 8, s.size_bits / 8, s.gap)).collect();
    assert_eq!(
        seen,
        vec![(8, 2, true), (10, 2, false), (12, 2, true), (14, 3, false), (17, 1, true), (18, 1, false), (19, 1, true)]
    );
}

/// Whether a node is a list, said the same way for all five kinds of list.
///
/// The type column writes them four ways: `X[]`, `offsets → X`, `chain → X`
/// and `descriptors → X`. A view reading the column took the last two for
/// structures, and a run of bytes, which is `bytes[]` there, for a list.
#[test]
fn a_node_says_it_is_a_list_whichever_way_its_type_is_written() {
    let list = |ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize]| ev.node(d, path).unwrap().list;

    // An array and a repeat, beside what is not a list: a number, a structure,
    // a switch that took a structure, and a pointer, whose one child is named
    // for it. A switch that took an array is a list, since the node is the
    // case the file took.
    let t = T::structure(
        "Root",
        vec![
            ("nums", T::array(T::u8(), E::lit(2))),
            ("kind", T::u8()),
            ("picked", T::switch(E::field("kind"), vec![(1, T::structure("One", vec![("a", T::u8())]))], T::bytes(E::lit(0)))),
            ("listed", T::switch(E::field("kind"), vec![(1, T::array(T::u8(), E::lit(2)))], T::bytes(E::lit(0)))),
            ("there", T::at(E::lit(0), T::structure("Pointee", vec![("b", T::u8())]))),
            ("rest", T::repeat(T::u8(), Until::End)),
        ],
    );
    let d = doc(&[1, 2, 1, 0xaa, 3, 4, 5, 6]);
    let mut ev = Evaluator::new(Template::new("t", t));
    assert!(!list(&mut ev, &d, &[]));
    assert!(list(&mut ev, &d, &[0]));
    assert!(!list(&mut ev, &d, &[1]));
    assert_eq!(ev.node(&d, &[2]).unwrap().type_name, "One");
    assert!(!list(&mut ev, &d, &[2]));
    assert_eq!(ev.node(&d, &[3]).unwrap().type_name, "u8[]");
    assert!(list(&mut ev, &d, &[3]));
    assert!(ev.node(&d, &[4]).unwrap().composite);
    assert!(!list(&mut ev, &d, &[4]));
    assert!(list(&mut ev, &d, &[5]));

    // A list placed by a table of offsets, and its elements, which are not.
    let d = doc(POINTED);
    let mut ev = Evaluator::new(pointer_template());
    assert!(list(&mut ev, &d, &[2]));
    assert!(!list(&mut ev, &d, &[2, 0]));

    // A chain of links.
    let t = T::structure("Root", vec![("head", T::u16(Big)), ("recs", T::chain(E::field("head"), &["next"], Anchor::File, linked()))]);
    let d = doc(&[0, 8, 0, 5, 0xaa, 0, 0, 0xbb, 0, 2, 0xcc]);
    let mut ev = Evaluator::new(Template::new("t", t));
    assert_eq!(ev.node(&d, &[1]).unwrap().type_name, "chain \u{2192} Rec");
    assert!(list(&mut ev, &d, &[1]));

    // A descriptor each. The elements are runs of bytes, which the type column
    // writes as `bytes[]`, and are not lists of anything.
    let d = doc(&gathered_bytes());
    let mut ev = Evaluator::new(Template::new("t", gathered(T::bytes(E::placer(E::field("len"))))));
    assert!(list(&mut ev, &d, &[1]));
    assert_eq!(ev.node(&d, &[1, 0]).unwrap().type_name, "bytes[]");
    assert!(!list(&mut ev, &d, &[1, 0]));
}

#[test]
fn a_gathered_element_says_which_record_placed_it() {
    let t = gathered(T::bytes(E::placer(E::field("len"))));
    let d = doc(&gathered_bytes());
    let mut ev = Evaluator::new(Template::new("t", t));
    // The record first, named the way the walk reached it, and then the field
    // of it the offset was read from.
    let origins = ev.origins(&d, &[1, 1]).unwrap();
    let seen: Vec<(Role, &str, &[usize])> = origins.iter().map(|o| (o.role, o.label.as_str(), o.path.as_slice())).collect();
    assert_eq!(seen[0], (Role::Position, "groups[0].recs[1]", &[0, 0, 1, 1][..]));
    assert_eq!(seen[1], (Role::Position, "off", &[0, 0, 1, 1, 0][..]));
    assert_eq!(origins[1].value, "10");
    // And the length names the record's field too, not a field beside the
    // element, since the element has none.
    assert!(origins.iter().any(|o| o.role == Role::Length && o.path == vec![0, 0, 1, 1, 1]), "{origins:?}");
}

#[test]
fn a_placer_reads_the_record_that_placed_its_element() {
    // Each element is a structure asking its record two things: how long its
    // bytes are, and where the record said it was.
    let elem = T::structure(
        "Piece",
        vec![("bytes", T::bytes(E::placer(E::field("len")))), ("was_at", T::computed(E::placer(E::field("off"))))],
    );
    let d = doc(&gathered_bytes());
    let mut ev = Evaluator::new(Template::new("t", gathered(elem)));
    assert_eq!(ev.node(&d, &[1, 2, 1]).unwrap().value.as_int(), Some(18));
    assert_eq!(ev.node(&d, &[1, 0, 0]).unwrap().size_bits, 3 * 8);
    // Asked anywhere that no gather placed, there is no record to ask.
    let t = T::structure("Root", vec![("n", T::u8()), ("v", T::computed(E::placer(E::field("n"))))]);
    let mut ev = Evaluator::new(Template::new("t", t));
    assert!(ev.node(&d, &[1]).is_err());
}

#[test]
fn a_gather_finds_a_tagged_list_wherever_it_is_written() {
    // Two tables, each a number saying what it is and records. Only the
    // records of table 7 place anything, whichever of the two comes first.
    let table = T::structure(
        "Table",
        vec![("id", T::u8()), ("n", T::u8()), ("recs", T::array(placing(), E::field("n")))],
    );
    let from = vec![Step::field("tables"), Step::tagged(&["id"], 7, "sevens"), Step::field("recs"), Step::each()];
    let t = T::structure(
        "Root",
        vec![
            ("tables", T::array(table, E::lit(2))),
            ("region", T::sized(E::Remaining, T::gather(from, E::field("off"), Anchor::File, E::lit(0), T::bytes(E::placer(E::field("len")))))),
        ],
    );
    //                  table 3 at 0: one record    table 7 at 4: two records
    let first_three = [3, 1, 12, 1, 7, 2, 13, 2, 15, 1, 0, 0, 0xee, 0xaa, 0xaa, 0xbb];
    let first_seven = [7, 2, 13, 2, 15, 1, 3, 1, 12, 1, 0, 0, 0xee, 0xaa, 0xaa, 0xbb];
    for bytes in [first_three, first_seven] {
        let d = doc(&bytes);
        let mut ev = Evaluator::new(Template::new("t", t.clone()));
        assert_eq!(ev.node(&d, &[1]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[1, 0]).unwrap().offset_bits, 13 * 8);
        assert_eq!(ev.node(&d, &[1, 1]).unwrap().offset_bits, 15 * 8);
        // The byte table 3 pointed at belongs to nothing.
        assert_eq!(ev.locate(&d, 12 * 8).unwrap(), vec![1]);
        // Named by the step, not by the question it asked.
        let label = ev.origins(&d, &[1, 1]).unwrap()[0].label.clone();
        assert_eq!(label, "tables.sevens.recs[1]");
    }
}

#[test]
fn a_gather_inside_a_gathered_element_walks_from_the_record_that_placed_it() {
    // Two groups, each saying where its region is and how long, and holding
    // records that place bytes inside that region. The regions are a gather,
    // and what is in each is a gather over that group's records alone.
    let group = T::structure(
        "Group",
        vec![("at", T::u8()), ("len", T::u8()), ("n", T::u8()), ("recs", T::array(placing(), E::field("n")))],
    );
    let inner = vec![Step::placer(), Step::field("recs"), Step::each()];
    let region = T::sized(
        E::placer(E::field("len")),
        T::gather(inner, E::field("off"), Anchor::File, E::lit(0), T::bytes(E::placer(E::field("len")))),
    );
    let outer = vec![Step::field("groups"), Step::each()];
    let t = T::structure(
        "Root",
        vec![("groups", T::array(group, E::lit(2))), ("regions", T::gather(outer, E::field("at"), Anchor::File, E::lit(0), region))],
    );
    // Group 0 is a region at 20 of six bytes, with records placing 21 and 24;
    // group 1 is a region at 12 of six bytes, with one record placing 13.
    let mut bytes = vec![20, 6, 2, 21, 2, 24, 1, /* group 1 */ 12, 6, 1, 13, 3];
    bytes.extend([0, 0xb0, 0xb1, 0xb2, 0, 0, /* 18 */ 0, 0, /* 20 */ 0, 0xa0, 0xa1, 0, 0xa2, 0]);
    let d = doc(&bytes);
    let mut ev = Evaluator::new(Template::new("t", t));
    let at = |ev: &mut Evaluator, path: &[usize]| {
        let n = ev.node(&d, path).unwrap();
        (n.offset_bits / 8, n.size_bits / 8, n.child_count)
    };
    assert_eq!(at(&mut ev, &[1, 0]), (20, 6, 2));
    assert_eq!(at(&mut ev, &[1, 1]), (12, 6, 1));
    // Each region holds only what its own group's records place.
    assert_eq!(at(&mut ev, &[1, 0, 0]), (21, 2, 0));
    assert_eq!(at(&mut ev, &[1, 0, 1]), (24, 1, 0));
    assert_eq!(at(&mut ev, &[1, 1, 0]), (13, 3, 0));
    assert_eq!(ev.locate(&d, 14 * 8).unwrap(), vec![1, 1, 0]);
    assert_eq!(ev.locate(&d, 24 * 8).unwrap(), vec![1, 0, 1]);
    assert_eq!(ev.locate(&d, 23 * 8).unwrap(), vec![1, 0]);
    // Named from the outer record on, the way the walk went.
    assert_eq!(ev.origins(&d, &[1, 1, 0]).unwrap()[0].label, "groups[1].recs[0]");
    assert_eq!(ev.origins(&d, &[1, 0, 1]).unwrap()[0].label, "groups[0].recs[1]");
}

#[test]
fn a_gather_covering_nothing_inside_a_structure_ends_the_gaps_around_its_elements() {
    // A structure of twelve bytes holding a byte and a gather with no region
    // of its own, whose two elements land inside the structure. The gaps in
    // the structure are the bytes between those elements, not the whole of
    // what is after the byte.
    let from = vec![Step::field("recs"), Step::each()];
    let items = T::gather(from, E::field("off"), Anchor::File, E::lit(0), T::bytes(E::placer(E::field("len"))));
    let t = T::structure(
        "Root",
        vec![
            ("recs", T::array(placing(), E::lit(2))),
            ("region", T::sized(E::lit(12), T::structure("Region", vec![("head", T::u8()), ("items", items)]))),
        ],
    );
    let bytes = [8, 2, 12, 1, /* 4 */ 0xee, 0, 0, 0, /* 8 */ 0xa0, 0xa1, 0, 0, /* 12 */ 0xb0, 0, 0, 0];
    let d = doc(&bytes);
    let mut ev = Evaluator::new(Template::new("t", t));
    let spans = ev.spans(&d, 4 * 8, 16 * 8, 100).unwrap();
    let seen: Vec<(u64, u64, bool)> = spans.iter().map(|s| (s.offset_bits / 8, s.size_bits / 8, s.gap)).collect();
    assert_eq!(seen, vec![(4, 1, false), (5, 3, true), (8, 2, false), (10, 2, true), (12, 1, false), (13, 3, true)]);
    // A window starting inside a gap still sees the gap end at the element.
    let spans = ev.spans(&d, 10 * 8, 16 * 8, 100).unwrap();
    assert_eq!((spans[0].offset_bits / 8, spans[0].size_bits / 8, spans[0].gap), (10, 2, true));
    assert_eq!(ev.locate(&d, 9 * 8).unwrap(), vec![1, 1, 0]);
    assert_eq!(ev.locate(&d, 12 * 8).unwrap(), vec![1, 1, 1]);
    assert_eq!(ev.locate(&d, 11 * 8).unwrap(), vec![1]);
}

#[test]
fn a_record_that_places_nothing_is_passed_over() {
    // A record holds an offset only when its kind says so. One of kind 1
    // points outside the region, which is passed over too, and so is an
    // offset of zero once the gather is told zero means nothing.
    let rec = T::structure(
        "Rec",
        vec![
            ("kind", T::u8()),
            ("body", T::switch(E::field("kind"), vec![(1, T::structure("Placed", vec![("off", T::u8())]))], T::bytes(E::lit(1)))),
        ],
    );
    let from = vec![Step::field("recs"), Step::each()];
    let region = |skip_zero: bool| {
        let g = T::gather(from.clone(), E::within(&["body", "off"]), Anchor::File, E::lit(0), T::u8());
        let g = if skip_zero { g.skipping_zero() } else { g };
        T::structure("Root", vec![("recs", T::array(rec.clone(), E::lit(5))), ("region", T::sized(E::Remaining, g))])
    };
    //          no offset   at 11     no offset  outside   at 0
    let bytes = [0, 9, 1, 11, 2, 9, 1, 99, 1, 0, 0xaa, 0xbb];
    let d = doc(&bytes);
    let mut ev = Evaluator::new(Template::new("t", region(false)));
    // Zero is a place in the file, though not one inside this region.
    assert_eq!(ev.node(&d, &[1]).unwrap().child_count, 1);
    assert_eq!(ev.node(&d, &[1, 0]).unwrap().value.as_int(), Some(0xbb));
    let mut ev = Evaluator::new(Template::new("t", region(true)));
    assert_eq!(ev.node(&d, &[1]).unwrap().child_count, 1);
}

/// Forty records, each placing a byte of the region after them, the last
/// record's first.
fn many_gathered() -> (T, Vec<u8>) {
    let from = vec![Step::field("recs"), Step::each()];
    let t = T::structure(
        "Root",
        vec![
            ("recs", T::array(placing(), E::lit(40))),
            ("region", T::sized(E::Remaining, T::gather(from, E::field("off"), Anchor::File, E::lit(0), T::bytes(E::placer(E::field("len")))))),
        ],
    );
    let mut b = Vec::new();
    for i in 0..40u8 {
        b.extend([80 + 2 * (39 - i), 1]);
    }
    b.extend((0..80u8).map(|i| if i % 2 == 0 { i } else { 0 }));
    (t, b)
}

#[test]
fn a_gather_carries_on_across_goes() {
    let (t, bytes) = many_gathered();
    let d = doc(&bytes);
    let mut whole = Evaluator::new(Template::new("t", t.clone()));
    let want: Vec<u64> = (0..40).map(|i| whole.node(&d, &[1, i]).unwrap().offset_bits).collect();
    let mut ev = Evaluator::new(Template::new("t", t));
    ev.set_slice(Some(8));
    let mut goes = 0;
    let n = loop {
        goes += 1;
        assert!(goes < 50, "the walk is not getting any further");
        ev.begin_slice();
        match ev.node(&d, &[1]) {
            Ok(info) => break info.child_count,
            Err(EvalError::Busy { .. }) => {}
            Err(e) => panic!("{e:?}"),
        }
    };
    // It did stop part way, and what it found in goes is what it finds in one.
    assert!(goes > 2, "{goes}");
    assert_eq!(n, 40);
    let got: Vec<u64> = (0..40).map(|i| ev.node(&d, &[1, i]).unwrap().offset_bits).collect();
    assert_eq!(got, want);
}

#[test]
fn a_whole_window_of_spans_over_a_gather_finishes_in_goes() {
    // `spans` starts again from the top of its window every go. A walk that
    // charged for going back over the records the last go reached would spend
    // each new go doing that, and never reach the end: bug B1, for a list walk.
    let (t, bytes) = many_gathered();
    let d = doc(&bytes);
    let mut ev = Evaluator::new(Template::new("t", t));
    ev.set_slice(Some(8));
    let mut goes = 0;
    let spans = loop {
        goes += 1;
        assert!(goes < 200, "spans over the gather never settled");
        ev.begin_slice();
        match ev.spans(&d, 0, d.len_bits(), 1000) {
            Ok(spans) => break spans,
            Err(EvalError::Busy { .. }) => {}
            Err(e) => panic!("{e:?}"),
        }
    };
    let region: Vec<&Span> = spans.iter().filter(|s| s.offset_bits >= 80 * 8).collect();
    // Forty bytes placed and forty between them, each its own entry.
    assert_eq!(region.iter().filter(|s| !s.gap).count(), 40);
    assert_eq!(region.iter().filter(|s| s.gap).count(), 40);
    assert_eq!(region.first().map(|s| s.path.clone()), Some(vec![1, 39]));
}

#[test]
fn a_lookup_can_be_keyed_by_text_found_somewhere_else() {
    // A table of definitions labelled in words, and records that say which
    // definition they follow by writing the word rather than a number. The key
    // field is padded to a fixed width and the pointing field is not, and the
    // two still match: what is compared is what the fields read as.
    let defs = T::array(
        T::structure("Def", vec![("name", T::utf8_padded(E::lit(6), b' ')), ("width", T::u8())]),
        E::lit(2),
    );
    let rec = T::structure(
        "Rec",
        vec![
            ("kind", T::utf8(E::lit(4))),
            // What that definition says one of these is worth.
            ("width", T::computed(E::tagged_by_text("defs", &["name"], E::field("kind"), &["width"]))),
            // And what it is called, read back as text.
            ("called", T::computed_text(E::tagged_by_text("defs", &["name"], E::field("kind"), &["name"]))),
        ],
    );
    let t = T::structure("Root", vec![("defs", defs), ("recs", T::repeat(rec, Until::End))]);
    let d = doc(b"flux  \x04time  \x08timefluxnope");
    let mut ev = Evaluator::new(Template::new("t", t));
    assert_eq!(ev.node(&d, &[1, 0, 1]).unwrap().value.as_int(), Some(8));
    assert_eq!(ev.node(&d, &[1, 1, 1]).unwrap().value.as_int(), Some(4));
    assert_eq!(ev.node(&d, &[1, 0, 2]).unwrap().value, Value::Str("time".into()));
    // A word nothing is labelled with finds nothing, which is an answer rather
    // than an error: zero for the number, and no text at all.
    assert_eq!(ev.node(&d, &[1, 2, 1]).unwrap().value.as_int(), Some(0));
    assert_eq!(ev.node(&d, &[1, 2, 2]).unwrap().value, Value::Str(String::new()));
    // Both ends of the search are connections: the field the word was read
    // from, and the definition it landed on.
    let seen: Vec<_> = ev.origins(&d, &[1, 0, 1]).unwrap().into_iter().map(|o| (o.role, o.label)).collect();
    assert!(seen.contains(&(Role::Value, "kind".to_string())), "{seen:?}");
    assert!(seen.iter().any(|(_, l)| l == "defs[1].width"), "{seen:?}");
    // And the question is written out with the word in its place, rather than
    // with the expression that found the word. `text` says the key is compared
    // as text, which is what tells this search from one keyed on a number.
    let rel = ev.relations(&d, &[1, 0, 1]).unwrap();
    assert_eq!(rel[0].written, "defs[name = text kind].width");
    assert_eq!(rel[0].substituted, "defs[name = \"time\"].width");
    assert_eq!(rel[0].result, "8");
}

#[test]
fn a_list_inside_a_sibling_can_be_indexed() {
    // The widths are in a table inside the header, and the values that are
    // those widths are the header's sibling. A name reaches only a field
    // beside the one asking, so a path is what gets there.
    let header = T::structure("Header", vec![("n", T::u8()), ("widths", T::array(T::u8(), E::field("n")))]);
    let value = T::structure("Value", vec![("v", T::uint_expr(E::elem_within(&["header", "widths"], E::idx(), &[]).mul(E::lit(8)), Big))]);
    let t = T::structure(
        "Root",
        vec![("header", header), ("values", T::array(value, E::within(&["header", "n"])))],
    );
    let d = doc(&[2, 1, 2, 0xaa, 0xbb, 0xcc]);
    let mut ev = Evaluator::new(Template::new("t", t));
    // One byte wide, then two.
    assert_eq!(ev.node(&d, &[1, 0, 0]).unwrap().value, Value::UInt(0xaa));
    assert_eq!(ev.node(&d, &[1, 1, 0]).unwrap().value, Value::UInt(0xbbcc));
    // The row says which entry of the table decided it, path and all.
    let seen: Vec<_> = ev.origins(&d, &[1, 1, 0]).unwrap().into_iter().map(|o| (o.role, o.label)).collect();
    assert!(seen.contains(&(Role::Width, "header.widths[1]".to_string())), "{seen:?}");
    let rel = ev.relations(&d, &[1, 1, 0]).unwrap();
    assert_eq!(rel[0].written, "header.widths[index] * 8");
}

/// A bit vector says which of a list of things has something, and the table
/// after it has one row per bit that was set. That count is written nowhere,
/// which is what `PopCount` is for.
#[test]
fn a_table_can_be_counted_by_the_bits_set_in_the_vector_before_it() {
    let t = T::structure(
        "Root",
        vec![
            ("present", T::bytes(E::lit(2))),
            ("values", T::array(T::u8(), E::pop_count("present"))),
        ],
    );
    // 0b1010_0001 0b0000_0110: five bits set, so five values follow.
    let d = doc(b"\xa1\x06\x0a\x0b\x0c\x0d\x0e");
    let mut ev = Evaluator::new(Template::new("t", t));
    assert_eq!(ev.node(&d, &[1]).unwrap().child_count, 5);
    assert_eq!(ev.node(&d, &[1, 4]).unwrap().value.as_int(), Some(0x0e));
}

/// Counted over the field's own bits, not its bytes. A vector of five items
/// lives in one byte, and whatever a format left in the three bits past the
/// end of it must not decide how long the table is.
#[test]
fn the_padding_at_the_end_of_a_bit_vector_is_not_counted() {
    let t = T::structure(
        "Root",
        vec![
            ("present", T::UInt { bits: 5, endian: crate::template::Endian::Big }),
            ("padding", T::UInt { bits: 3, endian: crate::template::Endian::Big }),
            ("values", T::array(T::u8(), E::pop_count("present"))),
        ],
    );
    // 0b1010_0111: two of the top five bits set, and the three padding bits
    // all set. Counting the byte would ask for five values.
    let d = doc(b"\xa7\x0a\x0b\x0c");
    let mut ev = Evaluator::new(Template::new("t", t));
    assert_eq!(ev.node(&d, &[2]).unwrap().child_count, 2);
}

#[test]
fn a_field_can_take_its_displayed_name_from_the_file() {
    // The name is written in a table earlier in the file, and the field it
    // names is a plain number the template calls `col1`.
    let t = T::structure(
        "Root",
        vec![
            ("labels", T::array(T::utf8(E::lit(4)), E::lit(2))),
            ("col1", T::u8()),
            ("col2", T::u8()),
        ],
    )
    .field_named_from("col1", E::elem_field("labels", E::lit(0), &[]))
    .field_named_from("col2", E::elem_field("labels", E::lit(1), &[]));
    let d = doc(b"fluxtime\x07\x09");
    let mut ev = Evaluator::new(Template::new("t", t));
    assert_eq!(ev.node(&d, &[1]).unwrap().name, "col1 flux");
    assert_eq!(ev.node(&d, &[2]).unwrap().name, "col2 time");
    // The declared name is still the name: a path and an expression are
    // written with it, and it does not move when the labels are edited.
    assert_eq!(ev.child_named(&d, &[], "col1").unwrap(), Some(vec![1]));
    // And the connection is exposed, as a name rather than as a value.
    let seen: Vec<_> = ev.origins(&d, &[1]).unwrap().into_iter().map(|o| (o.role, o.label)).collect();
    assert_eq!(seen, vec![(Role::Name, "labels[0]".to_string())]);
}

#[test]
fn the_elements_of_a_list_can_take_their_displayed_names_from_the_file() {
    // Two names in a table, and a list of three numbers named by position in
    // it: the third has no name written for it.
    let t = T::structure(
        "Root",
        vec![
            ("labels", T::array(T::utf8(E::lit(4)), E::lit(2))),
            ("vals", T::array(T::u8(), E::lit(3))),
            ("after", T::computed(E::elem_field("vals", E::lit(1), &[]))),
        ],
    )
    .field_elem_named_from("vals", E::elem_field("labels", E::idx(), &[]));
    let d = doc(b"fluxtime\x07\x09\x0b");
    let mut ev = Evaluator::new(Template::new("t", t));
    assert_eq!(ev.node(&d, &[1, 0]).unwrap().name, "[0] flux");
    assert_eq!(ev.node(&d, &[1, 1]).unwrap().name, "[1] time");
    // Nothing to read for the third, so it keeps its index and nothing fails.
    assert_eq!(ev.node(&d, &[1, 2]).unwrap().name, "[2]");
    // The list itself is not renamed: the declaration is about its elements.
    assert_eq!(ev.node(&d, &[1]).unwrap().name, "vals");
    // The index is still the name an expression reaches an element by.
    assert_eq!(ev.node(&d, &[2]).unwrap().value.as_int(), Some(9));
    // And the connection is exposed, as a name rather than as a value.
    let seen: Vec<_> = ev.origins(&d, &[1, 1]).unwrap().into_iter().map(|o| (o.role, o.label)).collect();
    assert_eq!(seen, vec![(Role::Name, "labels[1]".to_string())]);
}

#[test]
fn the_elements_of_a_list_can_be_named_by_a_number_written_in_words() {
    // A list numbered from 2 in the file's own count, the way SQLite's pages
    // after the first are, and a second list whose words carry a unit.
    let t = T::structure(
        "Root",
        vec![
            ("pages", T::array(T::u8(), E::lit(12))),
            ("runs", T::array(T::u8(), E::lit(2))),
            ("after", T::computed(E::elem_field("pages", E::lit(1), &[]))),
        ],
    )
    .field_elem_named_from("pages", E::words("page {}", E::idx().add(E::lit(2))))
    .field_elem_named_from("runs", E::words("{} bytes of data", E::idx().mul(E::lit(1000)).add(E::lit(1))));
    let d = doc(&[0, 9, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13]);
    let mut ev = Evaluator::new(Template::new("t", t));
    assert_eq!(ev.node(&d, &[0, 0]).unwrap().name, "[0] page 2");
    assert_eq!(ev.node(&d, &[0, 11]).unwrap().name, "[11] page 13");
    // A number with a unit after it reads as a record's line reads one:
    // grouped, and singular for one.
    assert_eq!(ev.node(&d, &[1, 0]).unwrap().name, "[0] 1 byte of data");
    assert_eq!(ev.node(&d, &[1, 1]).unwrap().name, "[1] 1,001 bytes of data");
    // The index is still the name an expression reaches an element by.
    assert_eq!(ev.node(&d, &[2]).unwrap().value.as_int(), Some(9));
    // Written out, the number stays in view as the working it is.
    assert_eq!(crate::template_text::expr(&E::words("page {}", E::idx().add(E::lit(2)))), "words(\"page {}\", index + 2)");
}

#[test]
fn a_number_in_words_is_text_and_not_a_number() {
    // Asked for as a number, the words fail and say where the number is,
    // rather than answering with it and leaving the words unread.
    let t = T::structure("Root", vec![("data", T::bytes(E::words("{} bytes", E::lit(3))))]);
    let mut ev = Evaluator::new(Template::new("t", t));
    assert_eq!(
        failure(ev.node(&doc(&[0; 4]), &[0]).unwrap_err()),
        "text where a number is needed; the number is the expression inside words(...)"
    );
}

/// A record whose name is inside a field of it, and a list named by its
/// first element: what `named_by` says when it is a path.
fn nested_names() -> Template {
    let word = T::structure_named("Word", "name", "", vec![("tag", T::u8()), ("name", T::utf8(E::lit(3)))]);
    let item = T::structure_named(
        "Item",
        "body",
        "body",
        vec![
            ("kind", T::u8()),
            // A word, or three bytes that say nothing about what they are.
            (
                "body",
                T::switch(E::field("kind"), vec![(1, word)], T::structure("Blob", vec![("a", T::u8()), ("b", T::u16(Big))])),
            ),
        ],
    );
    let group = T::structure_named("Group", "items.0", "items", vec![("items", T::array(item, E::lit(2)))]);
    Template::new("t", T::structure("Root", vec![("a", group.clone()), ("deep", T::structure_named("Deep", "body.name", "", vec![("body", group)]))]))
}

#[test]
fn a_name_that_is_a_path_is_read_from_where_the_path_lands() {
    // Two groups of two items: a word `abc`, then a blob that names nothing;
    // then a word `xyz` and the same.
    let bytes = [1, 0, b'a', b'b', b'c', 2, 9, 9, 9, 1, 0, b'x', b'y', b'z', 2, 9, 9, 9];
    let d = doc(&bytes);
    let mut ev = Evaluator::new(nested_names());
    // An item named by its body, and the body by its own `named_by`: two
    // steps of naming, the second one the structure's own.
    assert_eq!(ev.node(&d, &[0, 0, 0]).unwrap().name, "[0] abc");
    // A body with nothing that names it leaves the item with the bare index.
    assert_eq!(ev.node(&d, &[0, 0, 1]).unwrap().name, "[1]");
    // The group is named by its first item, through the list.
    assert_eq!(ev.node(&d, &[0]).unwrap().name, "a abc");
    // A path that reaches nothing keeps the declared name: `Deep` asks for
    // `body.name`, and a group has no field called `name`.
    assert_eq!(ev.node(&d, &[1]).unwrap().name, "deep");
}

#[test]
fn read_as_names_the_run_of_numbers_or_code_a_stretch_is_inside() {
    use crate::code::Isa;
    use crate::template::Until;
    // A name, a run of samples, some x86-64 code, and a stored stream.
    let mut b = b"NAME".to_vec();
    b.extend([0x41, 0x57, 0x41, 0x56, 0x41, 0x55, 0x41, 0x54]);
    // push r15; push r14; push r13; push r12; ret
    b.extend([0x41, 0x57, 0x41, 0x56, 0x41, 0x55, 0x41, 0x54, 0xc3]);
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("name", T::utf8(E::lit(4))),
                ("samples", T::array(Ty::Int { bits: 16, endian: Little }, E::lit(4))),
                ("code", T::sized(E::lit(9), T::repeat(T::insn(Isa::X86_64), Until::End))),
            ],
        ),
    );
    let d = doc(&b);
    let mut ev = Evaluator::new(t);

    let text = ev.read_as_under(&d, &[], 0, 4 * 8).unwrap();
    assert_eq!(text, None);

    let samples = ev.read_as_under(&d, &[], 5 * 8, 11 * 8).unwrap().unwrap();
    assert_eq!(samples.kind, NotText::Numbers);
    assert_eq!((samples.name.as_str(), samples.what.as_str()), ("samples", "i16 le"));
    assert_eq!((samples.offset_bits, samples.size_bits), (4 * 8, 8 * 8));

    let code = ev.read_as_under(&d, &[], 12 * 8, 20 * 8).unwrap().unwrap();
    assert_eq!(code.kind, NotText::Code);
    assert_eq!((code.name.as_str(), code.what.as_str()), ("code", "x86-64"));

    // From the last sample into the code is inside neither.
    assert_eq!(ev.read_as_under(&d, &[], 10 * 8, 14 * 8).unwrap(), None);
    // Nor is the name and the first sample together.
    assert_eq!(ev.read_as_under(&d, &[], 2 * 8, 6 * 8).unwrap(), None);
}
