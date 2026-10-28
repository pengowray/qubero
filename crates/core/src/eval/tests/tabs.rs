use super::*;

/// What the two streams of [`two_streams`] hold: a number and the rest, then
/// a length, that many letters and a byte for each stream in the file.
const FIRST_STREAM: [u8; 3] = [0x12, 0x34, 0xff];
const SECOND_STREAM: [u8; 6] = [3, b'a', b'b', b'c', 7, 8];

/// A file holding a list of streams the way a PDB does: the sizes are a table
/// at the front, and each stream's contents name that table, the list's own
/// count, and which element of the list they are in. `packed` says whether
/// each stream is a zlib run or a run of the file joined as a stream of its
/// own.
///
/// None of those names is inside the stream, so a tab over one reads only if
/// it asks the reading of the file.
fn two_streams(packed: bool) -> (Template, Document<MemSource>) {
    let size = || E::elem("sizes", E::idx());
    let inner = T::sized(
        size(),
        T::switch(
            E::idx(),
            vec![
                (0, T::structure("First", vec![("a", T::u16(Big)), ("rest", T::bytes(E::Remaining))])),
                (
                    1,
                    T::structure(
                        "Second",
                        vec![
                            ("len", T::u8()),
                            ("text", T::text(StrLen::Fixed(E::field("len")), Encoding::Ascii)),
                            ("each", T::array(T::u8(), E::field("count"))),
                        ],
                    ),
                ),
            ],
            T::bytes(E::Remaining),
        ),
    );
    let runs: Vec<Vec<u8>> = if packed {
        [&FIRST_STREAM[..], &SECOND_STREAM[..]].iter().map(|s| miniz_oxide::deflate::compress_to_vec_zlib(s, 6)).collect()
    } else {
        vec![FIRST_STREAM.to_vec(), SECOND_STREAM.to_vec()]
    };
    let stream = if packed {
        vec![("stream", T::decoded(E::elem("runs", E::idx()), crate::codec::Codec::Zlib, inner))]
    } else {
        vec![
            ("data", T::bytes(size())),
            ("stream", T::stitched(vec![Step::field("data")], None, Some(size()), inner)),
        ]
    };
    let t = Template::new(
        "streams",
        T::structure(
            "Streams",
            vec![
                ("count", T::u8()),
                ("sizes", T::array(T::u8(), E::field("count"))),
                ("runs", T::array(T::u8(), E::field("count"))),
                ("streams", T::array(T::structure("Stream", stream), E::field("count"))),
            ],
        ),
    );
    let mut bytes = vec![2, FIRST_STREAM.len() as u8, SECOND_STREAM.len() as u8, runs[0].len() as u8, runs[1].len() as u8];
    bytes.extend(runs.concat());
    (t, doc(&bytes))
}

/// The path of the stream in element `i` of [`two_streams`]' list.
fn stream_at(packed: bool, i: usize) -> Vec<usize> {
    vec![3, i, if packed { 0 } else { 1 }]
}

/// A stream opened as a tab reads the fields it read inside the file, names
/// from outside it and all, and reads them as the tab's own: counted from the
/// front of the tab, and in the tab's own bytes.
fn a_tab_reads_names_from_outside_its_stream(packed: bool) {
    let (t, d) = two_streams(packed);
    let mut e = Evaluator::new(t);
    let second = e.open_space(&d, 0, &stream_at(packed, 1)).unwrap().expect("the second stream opens");
    assert_eq!(e.space(second).unwrap().bytes(), SECOND_STREAM);
    let root = e.tab_node(&d, second, &[]).unwrap();
    // Which element of the list the stream is in is what picked the type, so a
    // tab that lost its place reads the first stream's type over the second's
    // bytes rather than failing.
    assert_eq!((root.type_name.as_str(), root.child_count), ("Second", 3));
    assert_eq!((root.path.as_slice(), root.offset_bits, root.space), (&[][..], 0, 0));
    assert!(!root.space_root, "the tab's own root is not a stream inside it");
    let text = e.tab_node(&d, second, &[1]).unwrap();
    assert_eq!((text.value, text.offset_bits, text.path), (Value::Str("abc".into()), 8, vec![1]));
    // A count taken from the top of the file.
    let each = e.tab_node(&d, second, &[2]).unwrap();
    assert_eq!(each.child_count, 2);
    assert_eq!(e.tab_node(&d, second, &[2, 1]).unwrap().value, Value::UInt(8));

    let first = e.open_space(&d, 0, &stream_at(packed, 0)).unwrap().expect("the first stream opens");
    assert_eq!(e.tab_node(&d, first, &[]).unwrap().type_name, "First");
    assert_eq!(e.tab_node(&d, first, &[0]).unwrap().value, Value::UInt(0x1234));

    // The views over the bytes ask by the tab's bits and are answered in the
    // tab's paths: the byte under the cursor, and the column beside them.
    let root = e.space(second).unwrap().view().expect("read where it was declared").root.clone();
    assert_eq!(root, [stream_at(packed, 1), vec![0]].concat());
    let mut tab = Tab::new(&mut e, &d, root);
    assert_eq!(tab.locate(2 * 8).unwrap(), [1]);
    assert_eq!(tab.locate(5 * 8).unwrap(), [2, 1]);
    let spans: Vec<_> = tab.spans(0, 6 * 8, 16).unwrap().into_iter().map(|s| (s.path, s.offset_bits / 8, s.size_bits / 8)).collect();
    assert_eq!(spans, [(vec![0], 0, 1), (vec![1], 1, 3), (vec![2, 0], 4, 1), (vec![2, 1], 5, 1)]);
    // What settled a field inside the tab keeps its place; what settled it
    // from outside keeps its name and has no row in the tab to go to.
    let each: Vec<_> = tab.origins(&[2]).unwrap().into_iter().map(|o| (o.label, o.path)).collect();
    assert!(each.contains(&("count".into(), vec![])), "{each:?}");
    let text: Vec<_> = tab.origins(&[1]).unwrap().into_iter().map(|o| (o.label, o.path)).collect();
    assert!(text.contains(&("len".into(), vec![0])), "{text:?}");
}

/// An edit to the file drops the tab with the rest of the reading, and the
/// stream opened again reads what the edit left.
#[test]
fn a_tab_opened_again_after_an_edit_reads_the_edit() {
    let (t, mut d) = two_streams(false);
    let mut e = Evaluator::new(t);
    let second = e.open_space(&d, 0, &stream_at(false, 1)).unwrap().expect("the second stream opens");
    assert_eq!(e.tab_node(&d, second, &[1]).unwrap().value, Value::Str("abc".into()));
    let at = e.node(&d, &[3, 1, 0]).unwrap().offset_bits / 8 + 1;
    d.overwrite_bytes(at, b"z");
    e.invalidate_from(at * 8);
    assert!(e.space(second).is_none(), "the tab went with the reading");
    let again = e.open_space(&d, 0, &stream_at(false, 1)).unwrap().expect("it opens again");
    assert_eq!(e.tab_node(&d, again, &[1]).unwrap().value, Value::Str("zbc".into()));
    assert_eq!(&e.space(again).unwrap().bytes()[1..4], b"zbc");
}

#[test]
fn a_joined_stream_open_as_a_tab_reads_names_from_outside_it() {
    a_tab_reads_names_from_outside_its_stream(false);
}

#[test]
fn an_unpacked_stream_open_as_a_tab_reads_names_from_outside_it() {
    a_tab_reads_names_from_outside_its_stream(true);
}
