use super::*;

/// A tagged search reaches a list that lives behind an address, and finds the
/// element whose label the record asking works out for itself.
///
/// Both halves matter and neither was proven before. The list is named by a
/// path down into a field whose contents are somewhere else, so the path has
/// to step through the `At` the way every other path does. The label is not a
/// number the template fixed but one written in the record asking, which is
/// how a variable-length element says which object of a heap collection holds
/// its bytes.
#[test]
fn a_tagged_search_reaches_a_list_through_an_at() {
    let element = || {
        T::structure(
            "Element",
            vec![("index", T::u8()), ("size", T::u8()), ("payload", T::bytes(E::field("size")))],
        )
    };
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("collection_address", T::u8()),
                ("want", T::u8()),
                (
                    "collection",
                    T::at(
                        E::field("collection_address"),
                        T::structure("Collection", vec![("elements", T::repeat(element(), Until::End))]),
                    ),
                ),
                (
                    "found",
                    T::computed(E::tagged_in_by(
                        E::within(&["collection", "elements"]),
                        &["index"],
                        E::field("want"),
                        &["size"],
                    )),
                ),
            ],
        ),
    );
    // At byte 4: element 7 of one byte, element 3 of two, element 5 of three.
    let d = doc(&[4, 3, 0, 0, 7, 1, b'a', 3, 2, b'b', b'c', 5, 3, b'd', b'e', b'f']);
    let mut ev = Evaluator::new(t);
    let found = ev.node(&d, &[3]).unwrap();
    assert_eq!(found.value.as_int(), Some(2), "the search should land on the element labelled 3");

    // And the path it lands on is the one inside the collection, so a reader
    // asking where the answer came from is sent to those bytes.
    let here = ev.memo.get(&vec![3usize]).map(|r| (r.offset, r.limit));
    let search = match &ev.template.root {
        Ty::Struct(s) => match &s.fields[3].ty {
            Ty::Computed(Expr::Tagged(t)) => t.clone(),
            other => panic!("not a tagged search: {other:?}"),
        },
        other => panic!("not a structure: {other:?}"),
    };
    let (path, label) =
        ev.tagged_path(&d, &[3], &search, here).unwrap().expect("the search should find an element");
    // Root, then the collection field, then the one child of its `At`, then
    // the list, then the second element, then the field read from it.
    assert_eq!(path, vec![2, 0, 0, 1, 1]);
    assert_eq!(label, "collection.elements[1].size");
}

/// A collection several records reach by address is read once, however many of
/// them reach it.
///
/// This is the difference between a column of variable-length strings that can
/// be read and one that cannot. Each of them carries the address of the heap
/// collection its bytes are in, so each is a path of its own to the same
/// stretch of the file, and walking that stretch once per string is quadratic
/// in the length of the column.
#[test]
fn a_tagged_search_over_a_list_reached_by_address_is_walked_once() {
    let searching = |field: &[&str]| {
        T::computed(E::tagged_in_by(
            E::within(&["collection", "elements"]),
            &["index"],
            E::field("want"),
            field,
        ))
    };
    let reference = T::structure(
        "Reference",
        vec![
            ("address", T::u8()),
            ("want", T::u8()),
            (
                "collection",
                T::at(
                    E::field("address"),
                    T::structure(
                        "Collection",
                        vec![(
                            "elements",
                            T::repeat(
                                T::structure("Element", vec![("index", T::u8()), ("size", T::u8())]),
                                Until::End,
                            ),
                        )],
                    ),
                ),
            ),
            ("found", searching(&["size"])),
        ],
    );
    let t = Template::new("t", T::structure("Root", vec![("refs", T::array(reference, E::lit(2)))]));

    // Two references to one collection of two hundred elements, both after
    // the same one, which is a long way in.
    let mut bytes = vec![4u8, 150, 4, 150];
    for i in 0..200u8 {
        bytes.push(i);
        bytes.push(i / 4);
    }
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);

    assert_eq!(ev.node(&d, &[0, 0, 3]).unwrap().value.as_int(), Some(150 / 4));
    let after_first = ev.memo.len();
    assert_eq!(ev.node(&d, &[0, 1, 3]).unwrap().value.as_int(), Some(150 / 4));
    let grew = ev.memo.len() - after_first;

    // The second reference places its own note and its own view of the
    // collection, and nothing else: the elements it would have walked are
    // already read, under the first reference's path.
    assert!(grew < 12, "the second reference read {grew} more nodes, so it walked the collection again");
    assert!(!ev.memo.contains_key(&vec![0usize, 1, 2, 0, 0, 0]), "the second reference placed an element of its own");
}

/// A label is found wherever in the list it was written, and a search that
/// picks up where an earlier one stopped still answers with the first element
/// carrying the label.
///
/// A global heap does not renumber itself: rewriting a string leaves a new
/// object at the end of the collection with the index the old one had, so the
/// indices are in whatever order the writing happened in.
#[test]
fn a_tagged_search_finds_an_element_written_out_of_order() {
    let reference = T::structure(
        "Reference",
        vec![
            ("address", T::u8()),
            ("want", T::u8()),
            (
                "collection",
                T::at(
                    E::field("address"),
                    T::structure(
                        "Collection",
                        vec![(
                            "elements",
                            T::repeat(
                                T::structure(
                                    "Element",
                                    vec![("index", T::u8()), ("size", T::u8()), ("payload", T::bytes(E::field("size")))],
                                ),
                                Until::End,
                            ),
                        )],
                    ),
                ),
            ),
            (
                "found",
                T::computed(E::tagged_in_by(
                    E::within(&["collection", "elements"]),
                    &["index"],
                    E::field("want"),
                    &["size"],
                )),
            ),
        ],
    );
    let t = Template::new("t", T::structure("Root", vec![("refs", T::array(reference, E::lit(3)))]));

    // Three references, and a list with the label 7 in it twice. The first
    // search stops in the middle of the list, the second carries on from there
    // and passes the second 7 on its way, and the third asks for 7: the answer
    // is the first element carrying it, which is the one the search would have
    // reached and is behind where the walk had got to.
    let d = doc(&[
        6, 5, 6, 9, 6, 7, //
        7, 1, b'a', //
        3, 2, b'b', b'c', //
        5, 3, b'd', b'e', b'f', //
        7, 4, b'g', b'h', b'i', b'j', //
        9, 1, b'k',
    ]);
    let mut ev = Evaluator::new(t);
    assert_eq!(ev.node(&d, &[0, 0, 3]).unwrap().value.as_int(), Some(3), "label 5 is the third element");
    assert_eq!(ev.node(&d, &[0, 1, 3]).unwrap().value.as_int(), Some(1), "label 9 is the last");
    assert_eq!(ev.node(&d, &[0, 2, 3]).unwrap().value.as_int(), Some(1), "label 7 is the first, not the fourth");
}

/// Where a field is, rather than what it says.
#[test]
fn start_of_names_where_an_earlier_field_begins() {
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("head", T::u16(Big)),
                ("body", T::bytes(E::lit(4))),
                ("body_at", T::computed(E::start_of(E::field("body")))),
                // Through a path into an earlier field, and through a list,
                // which is what a search hands it.
                ("inner", T::structure("Inner", vec![("a", T::u8()), ("b", T::u8())])),
                ("b_at", T::computed(E::start_of(E::within(&["inner", "b"])))),
            ],
        ),
    );
    let d = doc(&[0, 1, 2, 3, 4, 5, 6, 7]);
    let mut ev = Evaluator::new(t);
    assert_eq!(ev.node(&d, &[2]).unwrap().value.as_int(), Some(2));
    assert_eq!(ev.node(&d, &[4]).unwrap().value.as_int(), Some(7));
}

/// And counted from where this copy of the format begins, so that it reads as
/// an address of the format rather than as a place in whatever holds it.
///
/// An HDF5 file behind a 512-byte user block writes every address as if the
/// block were not there, and a MATLAB 7.3 file is exactly that. An answer
/// counted from the front of the file would be 512 too large and would be
/// silently right on every file without one.
#[test]
fn start_of_counts_from_the_nearest_origin() {
    let inner = || {
        T::structure(
            "Inner",
            vec![("head", T::u16(Big)), ("body", T::bytes(E::lit(4))), ("body_at", T::computed(E::start_of(E::field("body"))))],
        )
    };
    let d = doc(&[0; 16]);

    let plain = Template::new("t", T::structure("Root", vec![("preamble", T::bytes(E::lit(3))), ("inner", inner())]));
    let mut ev = Evaluator::new(plain);
    assert_eq!(ev.node(&d, &[1, 2]).unwrap().value.as_int(), Some(5), "from the front of the file");

    let framed =
        Template::new("t", T::structure("Root", vec![("preamble", T::bytes(E::lit(3))), ("inner", T::origin(inner()))]));
    let mut ev = Evaluator::new(framed);
    // An origin is a marker and takes no level of its own, so the field is in
    // the same place it was.
    assert_eq!(ev.node(&d, &[1, 2]).unwrap().value.as_int(), Some(2), "from the front of this copy of the format");
}

/// A field placed at where a search landed covers that element's bytes: the
/// two halves together are how the Nth element of a list whose elements vary
/// in size is read.
#[test]
fn an_at_placed_at_a_found_element_covers_its_bytes() {
    let found = |field: &[&str]| {
        E::tagged_in_by(E::within(&["collection", "elements"]), &["index"], E::field("want"), field)
    };
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("address", T::u8()),
                ("want", T::u8()),
                (
                    "collection",
                    T::at(
                        E::field("address"),
                        T::structure(
                            "Collection",
                            vec![(
                                "elements",
                                T::repeat(
                                    T::structure(
                                        "Element",
                                        vec![
                                            ("index", T::u8()),
                                            ("size", T::u8()),
                                            ("payload", T::bytes(E::field("size"))),
                                        ],
                                    ),
                                    Until::End,
                                ),
                            )],
                        ),
                    ),
                ),
                (
                    "object",
                    T::at(
                        E::start_of(found(&["payload"])),
                        T::sized(found(&["size"]), T::text(StrLen::Fixed(E::Remaining), Encoding::Utf8)),
                    ),
                ),
            ],
        ),
    );
    // At byte 2: element 7 of one byte, element 3 of two, element 5 of three.
    let d = doc(&[2, 5, 7, 1, b'a', 3, 2, b'b', b'c', 5, 3, b'd', b'e', b'f']);
    let mut ev = Evaluator::new(t);
    let object = ev.node(&d, &[3, 0]).unwrap();
    assert_eq!(object.value, Value::Str("def".into()));
    assert_eq!(object.offset_bits / 8, 11, "the third element's payload starts at byte 11");
    assert_eq!(object.size_bits, 3 * 8);
}

/// A source that says how many times it was asked for bytes, so that a saving
/// in reading rather than in memory can be seen.
struct Counting {
    bytes: Vec<u8>,
    reads: std::cell::Cell<usize>,
}

impl crate::source::Source for Counting {
    fn len_bytes(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn read_bytes(&self, offset: u64, out: &mut [u8]) -> Vec<crate::source::Missing> {
        self.reads.set(self.reads.get() + 1);
        let o = offset as usize;
        out.copy_from_slice(&self.bytes[o..o + out.len()]);
        Vec::new()
    }
}

/// The other shape that asks, and the other kind of saving. Every cell of a
/// FITS table reads its column's `TFORM` card out of the header, and the
/// header is one list at one path, so nothing was placed twice: what a second
/// cell used to cost was reading every card's keyword again and comparing it.
///
/// Measured in reads rather than in nodes for that reason, and against two
/// headers of different lengths: what the second cell costs is now the same in
/// both, which is the whole claim.
#[test]
fn a_cell_finds_its_card_without_rewalking_the_header() {
    let cost_of_the_second_cell = |cards: usize| -> usize {
        let card = T::structure("Card", vec![("key", T::bytes(E::lit(8))), ("body", T::u8())]);
        let t = Template::new(
            "t",
            T::structure(
                "Root",
                vec![
                    ("cards", T::array(card, E::lit(cards as i128))),
                    (
                        "cells",
                        T::array(
                            T::structure(
                                "Cell",
                                vec![("form", T::computed(E::tagged_bytes("cards", &["key"], b"TFORM1  ", &["body"])))],
                            ),
                            E::lit(2),
                        ),
                    ),
                ],
            ),
        );
        // Filler cards, and then the one every cell is after, written last so
        // that a search from the front reads all of them.
        let mut bytes = Vec::new();
        for i in 0..cards - 1 {
            bytes.extend_from_slice(format!("FILLER{i:02}").as_bytes());
            bytes.push(i as u8);
        }
        bytes.extend_from_slice(b"TFORM1  ");
        bytes.push(9);
        let d = Document::new(Counting { bytes, reads: std::cell::Cell::new(0) });
        let mut ev = Evaluator::new(t);
        assert_eq!(ev.node(&d, &[1, 0, 0]).unwrap().value.as_int(), Some(9));
        let before = d.source().reads.get();
        assert_eq!(ev.node(&d, &[1, 1, 0]).unwrap().value.as_int(), Some(9));
        d.source().reads.get() - before
    };
    let short = cost_of_the_second_cell(4);
    let long = cost_of_the_second_cell(40);
    assert_eq!(short, long, "the second cell still pays for the length of the header");
}

/// A field placed past the end of the root, by a choice made on text, is found
/// by the placement index. ROOT picks what a key's offset leads to by the class
/// name written in the key, and the index pruned every choice made that way as
/// placing nothing, so an RNTuple's envelopes and pages read as one gap.
#[test]
fn a_placement_behind_a_choice_by_text_is_indexed() {
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("class", T::utf8(E::lit(1))),
                ("offset", T::u8()),
                ("pick", T::matches(E::field("class"), vec![("k", T::at(E::field("offset"), T::u32(Big)))], T::bytes(E::lit(0)))),
            ],
        ),
    );
    let d = doc(&[b'k', 8, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4]);
    let mut ev = Evaluator::new(t);
    let spans = ev.spans(&d, 8 * 8, 12 * 8, 100).unwrap();
    assert_eq!(spans.len(), 1, "{spans:?}");
    assert!(!spans[0].gap, "the placed number reads as a gap: {spans:?}");
    assert_eq!((spans[0].offset_bits, spans[0].value.clone()), (8 * 8, Value::UInt(0x0102_0304)));
}

/// A field placed inside what the root covers, by a field nested below the
/// structure the walk down stops in, is found by the placement index as well.
/// An AppleDouble attribute's value is placed by the attribute, three levels
/// inside the entry before it, and the entry does not cover it.
#[test]
fn a_placement_inside_the_root_by_a_nested_field_is_found() {
    let entry = T::structure(
        "Entry",
        vec![("offset", T::u8()), ("len", T::u8()), ("value", T::at(E::field("offset"), T::bytes(E::field("len"))))],
    );
    let t = Template::new("t", T::sized(E::lit(8), T::structure("Root", vec![("entry", entry)])));
    let d = doc(&[4, 3, 0, 0, 7, 8, 9, 0]);
    let mut ev = Evaluator::new(t);
    let path = ev.locate(&d, 5 * 8).unwrap();
    let found = ev.node(&d, &path).unwrap();
    assert_eq!((found.offset_bits, found.size_bits), (4 * 8, 3 * 8), "{path:?}");
    let spans = ev.spans(&d, 0, 8 * 8, 100).unwrap();
    let named: Vec<_> = spans.iter().map(|s| (s.offset_bits / 8, s.size_bits / 8, s.gap)).collect();
    assert_eq!(named, vec![(0, 1, false), (1, 1, false), (2, 2, true), (4, 3, false), (7, 1, true)]);
}

/// Two lists placed over the same stretch are both walked, and a bit only the
/// second has an element at is that element. An Impulse Tracker module puts
/// its instruments, samples and patterns each in a list over the whole file,
/// and with no instruments the first is empty: the index kept one of the
/// three, and every sample header read as a gap.
#[test]
fn lists_placed_over_the_same_stretch_are_each_asked() {
    let item = |name| T::structure(name, vec![("x", T::u16(Big))]);
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("na", T::u8()),
                ("a_offsets", T::array(T::u8(), E::field("na"))),
                ("nb", T::u8()),
                ("b_offsets", T::array(T::u8(), E::field("nb"))),
                ("a", T::at(E::lit(0), T::pointer_list("a_offsets", Anchor::File, E::lit(0), item("A")))),
                ("b", T::at(E::lit(0), T::pointer_list("b_offsets", Anchor::File, E::lit(0), item("B")))),
            ],
        ),
    );
    let d = doc(&[0, 1, 6, 0, 0, 0, 0xbe, 0xef]);
    let mut ev = Evaluator::new(t);
    let path = ev.locate(&d, 6 * 8).unwrap();
    assert_eq!(ev.node(&d, &path).unwrap().value, Value::UInt(0xbeef), "{path:?}");
    // The stretch between the root and the element is a gap, from where the
    // root ends to where the element begins.
    let spans = ev.spans(&d, 0, 8 * 8, 100).unwrap();
    let gaps: Vec<_> = spans.iter().filter(|s| s.gap).map(|s| (s.offset_bits / 8, s.size_bits / 8)).collect();
    assert_eq!(gaps, vec![(3, 3)], "{spans:?}");
}
