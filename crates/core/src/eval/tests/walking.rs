use super::*;

#[test]
fn a_run_of_same_sized_blocks_is_counted_by_division() {
    // What a paged file is: a header saying how big a block is, then blocks of
    // that size until the file runs out. Nothing here is fixed at template
    // time, and the count is still arithmetic.
    let block = T::sized(E::field("block_size"), T::structure("Block", vec![("kind", T::u8())]));
    let t = Template::new(
        "t",
        T::structure("Root", vec![("block_size", T::u16(Little)), ("blocks", T::repeat(block, Until::End))]),
    );
    let (size, count) = (4096usize, 50_000usize);
    let mut bytes = (size as u16).to_le_bytes().to_vec();
    bytes.resize(2 + size * count, 0);
    bytes[2] = 9;
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);
    let all = ev.node(&d, &[1]).unwrap();
    assert_eq!((all.child_count, all.size_bits), (count as u64, (size * count) as u64 * 8));
    assert!(ev.memo.len() < 10, "counted by division, not by a walk: {} entries", ev.memo.len());
    // A block near the end is one division away, and so is the cursor
    // standing in it.
    let at = (2 + size * 49_999) as u64 * 8;
    assert_eq!(ev.node(&d, &[1, 49_999, 0]).unwrap().offset_bits, at);
    assert_eq!(ev.locate(&d, at).unwrap(), vec![1, 49_999, 0]);
    assert!(ev.memo.len() < 20, "{} entries", ev.memo.len());
}

#[test]
fn a_file_cut_off_mid_block_keeps_the_blocks_it_has() {
    // Half a block at the end is not a block. It belongs to nothing, rather
    // than taking the whole run down with it.
    let block = T::sized(E::field("block_size"), T::bytes(E::Remaining));
    let t = Template::new(
        "t",
        T::structure("Root", vec![("block_size", T::u16(Little)), ("blocks", T::repeat(block, Until::End))]),
    );
    let mut bytes = 16u16.to_le_bytes().to_vec();
    bytes.resize(2 + 16 * 3 + 5, 0);
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);
    let all = ev.node(&d, &[1]).unwrap();
    assert_eq!((all.child_count, all.size_bits), (3, 3 * 16 * 8));
    // The half block at the end belongs to no field, which is a gap: the root
    // and nothing under it, rather than an error.
    assert!(ev.locate(&d, (2 + 16 * 3 + 1) as u64 * 8).unwrap().is_empty());
}

#[test]
fn a_run_that_stops_on_what_it_reads_is_still_walked() {
    // The elements are all the same size, but the run ends at the one holding
    // a chosen value, and where that is cannot be divided out.
    let block = T::sized(E::field("block_size"), T::structure("Block", vec![("kind", T::u8())]));
    let until = Until::FieldBytes { field: "kind".into(), bytes: vec![0xff] };
    let t = Template::new(
        "t",
        T::structure("Root", vec![("block_size", T::u16(Little)), ("blocks", T::repeat(block, until))]),
    );
    let mut bytes = 4u16.to_le_bytes().to_vec();
    bytes.resize(2 + 4 * 10, 0);
    bytes[2 + 4 * 2] = 0xff;
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);
    assert_eq!(ev.node(&d, &[1]).unwrap().child_count, 3);
}

#[test]
fn counting_a_long_run_costs_the_same_however_long_it_is() {
    // A run that ends where the file does, of elements no two of which are the
    // same length. How many there are can only be found by walking, and the
    // walk must not leave one node per element behind: this is the shape a
    // file whose contents are a list of a billion things has.
    let count = |n: usize| {
        let mut bytes = Vec::new();
        for i in 0..n {
            let s = "x".repeat(i % 17 + 1);
            bytes.extend_from_slice(&(s.len() as u64).to_le_bytes());
            bytes.extend_from_slice(s.as_bytes());
        }
        let string = T::structure("String", vec![("len", T::u64(Little)), ("text", T::utf8(E::field("len")))]);
        let d = doc(&bytes);
        let mut ev = Evaluator::new(Template::new("t", T::repeat(string, Until::End)));
        assert_eq!(ev.node(&d, &[]).unwrap().child_count, n as u64);
        ev.memo_len()
    };
    // Three times the elements, and what is left in memory is the same: the
    // few thousand a short run is remembered by, and the window after them.
    let (small, large) = (count(20_000), count(60_000));
    assert!(large < 3 * small / 2, "counting kept {small} nodes for 20,000 and {large} for 60,000");
    assert!(large < 20_000, "counting kept {large} nodes");
}

#[test]
fn counting_a_long_run_a_bit_at_a_time_costs_no_more_than_counting_it_at_once() {
    // The browser counts in goes of a few thousand elements, so that the page
    // can draw between them. Every go but the first carries on a count that
    // already knows its run is long, and must keep dropping what it walks
    // past: this is the shape the count actually has when a file is open.
    let n = 40_000usize;
    let mut bytes = Vec::new();
    for i in 0..n {
        let s = "x".repeat(i % 17 + 1);
        bytes.extend_from_slice(&(s.len() as u64).to_le_bytes());
        bytes.extend_from_slice(s.as_bytes());
    }
    let string = T::structure("String", vec![("len", T::u64(Little)), ("text", T::utf8(E::field("len")))]);
    let d = doc(&bytes);
    let mut ev = Evaluator::new(Template::new("t", T::repeat(string, Until::End)));
    ev.set_slice(Some(500));
    let mut goes = 0;
    let counted = loop {
        goes += 1;
        assert!(goes < 1000, "the count is not getting anywhere");
        ev.begin_slice();
        match ev.node(&d, &[]) {
            Ok(info) => break info.child_count,
            Err(e) if e.interrupted() => continue,
            Err(e) => panic!("{e:?}"),
        }
    };
    assert_eq!(counted, n as u64);
    assert!(goes > 10, "the count was not interrupted, so this proves nothing");
    assert!(ev.memo_len() < 20_000, "counting in goes kept {} nodes", ev.memo_len());
}

#[test]
fn a_listing_asked_again_in_goes_gets_further_every_go() {
    // The shape of a Thrift footer: runs that end on a stop record, inside
    // runs, so every row of the listing sits two walks deep. `spans` starts
    // again from the top of its window each time it is asked, and going back
    // over the rows the last go reached used to be charged as if they were
    // being read for the first time. Once that cost more than a go allows,
    // every go ran out in the same place: a Parquet file of 73 KB never
    // finished at 5,000 a go and finished in one at 12,000.
    let item = T::structure("Item", vec![("kind", T::u8()), ("v", T::u8())]);
    let group = T::structure("Group", vec![("items", T::repeat(item, Until::FieldValue { field: "kind".into(), value: 0 }))]);
    let t = Template::new("t", T::repeat(group, Until::End));
    let mut bytes = Vec::new();
    for _ in 0..30 {
        for f in 1..=20u8 {
            bytes.extend_from_slice(&[1, f]);
        }
        bytes.extend_from_slice(&[0, 0]);
    }
    let d = doc(&bytes);
    let len = d.len_bits();
    let want = Evaluator::new(t.clone()).spans(&d, 0, len, 4000).unwrap();
    assert!(want.len() > 1000, "one row per field, not one per run: {}", want.len());

    let mut ev = Evaluator::new(t);
    ev.set_slice(Some(200));
    let mut goes = 0;
    let got = loop {
        goes += 1;
        assert!(goes <= 50, "asking again is not getting anywhere");
        ev.begin_slice();
        match ev.spans(&d, 0, len, 4000) {
            Ok(v) => break v,
            Err(e) if e.interrupted() => continue,
            Err(e) => panic!("{e:?}"),
        }
    };
    assert!(goes > 1, "the listing was not interrupted, so this proves nothing");
    assert_eq!(got.len(), want.len());
    for (g, w) in got.iter().zip(&want) {
        assert_eq!((g.offset_bits, g.size_bits, &g.name), (w.offset_bits, w.size_bits, &w.name));
    }
}

#[test]
fn a_long_list_of_uneven_elements_is_walked_without_being_remembered() {
    // Strings of growing length, so every element sits at an offset only the
    // walk can find, and one the test can work out for itself.
    let n = 20_000u64;
    let mut bytes = n.to_le_bytes().to_vec();
    let mut starts = vec![8u64];
    for i in 0..n {
        let s = "x".repeat((i % 17) as usize + 1);
        bytes.extend_from_slice(&(s.len() as u64).to_le_bytes());
        bytes.extend_from_slice(s.as_bytes());
        starts.push(bytes.len() as u64);
    }
    let string = T::structure("String", vec![("len", T::u64(Little)), ("text", T::utf8(E::field("len")))]);
    let t = Template::new(
        "t",
        T::structure("Root", vec![("n", T::u64(Little)), ("items", T::array(string, E::field("n")))]),
    );
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);

    let items = ev.node(&d, &[1]).unwrap();
    assert_eq!(items.child_count, n);
    assert_eq!(items.offset_bits + items.size_bits, bytes.len() as u64 * 8);
    // Twenty thousand elements, and what is left behind is the window the walk
    // keeps rather than one node per element.
    assert!(ev.memo_len() < 100, "the walk kept {} nodes", ev.memo_len());

    // An element in the middle, reached long after the walk passed it, is
    // where the file says it is and reads as what it holds.
    for i in [15_000usize, 3, 19_999, 7_777] {
        let e = ev.node(&d, &[1, i]).unwrap();
        assert_eq!(e.offset_bits, starts[i] * 8, "element {i}");
        let text = ev.node(&d, &[1, i, 1]).unwrap();
        assert_eq!(text.value, Value::Str("x".repeat(i % 17 + 1)));
    }
    assert!(ev.memo_len() < 100, "reaching into it kept {} nodes", ev.memo_len());
}

#[test]
fn a_walk_past_a_node_read_into_keeps_the_node() {
    // Four records, each a length and then a structure whose second field is
    // that long. The run is guarded from its first element, so every walk
    // along it keeps a journal of what it placed and drops that at the end.
    let mut bytes = Vec::new();
    for i in 0..4u8 {
        bytes.extend([i + 1, 0xa0 + i]);
        bytes.extend(std::iter::repeat_n(0xb0 + i, i as usize + 1));
    }
    let inner = T::structure("Inner", vec![("a", T::u8()), ("b", T::bytes(E::field("len")))]);
    let item = T::structure("Item", vec![("len", T::u8()), ("inner", inner)]);
    let t = Template::new("t", T::structure("Root", vec![("items", T::repeat(item, Until::End))]));
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);

    // Reading into record 1 places it, with nothing asking how long it is.
    assert_eq!(ev.node(&d, &[0, 1, 1, 0]).unwrap().value, Value::UInt(0xa1));
    // Reaching record 3 walks over record 1 and places it again. It was here
    // before the walk, so the walk must not take it away when it ends while
    // what was read inside it stays.
    ev.node(&d, &[0, 3]).unwrap();
    assert_eq!(ev.memo.without_parent(), Vec::<Vec<usize>>::new());
    // A field of record 1 read now looks up past its structure for `len`,
    // which is only there if record 1 is.
    let b = ev.node(&d, &[0, 1, 1, 1]).unwrap();
    assert_eq!(b.size_bits, 2 * 8);
}

#[test]
fn a_walk_inside_a_walk_keeps_nothing_its_parent_let_go_of() {
    // Records of a width, seven bytes of entries, and a tail as long as the
    // third entry says. Each entry is a count and that many values of the
    // record's width. Sizing a record finds its third entry, which walks the
    // entries inside the walk along the records.
    let entry = T::structure(
        "Entry",
        vec![("len", T::u8()), ("body", T::array(T::sized(E::field("w"), T::bytes(E::Remaining)), E::field("len")))],
    );
    let item = T::structure(
        "Item",
        vec![
            ("w", T::u8()),
            ("entries", T::sized(E::lit(7), T::repeat(entry, Until::End))),
            ("tail", T::bytes(E::elem_field("entries", E::lit(2), &["len"]))),
        ],
    );
    let t = Template::new("t", T::structure("Root", vec![("items", T::repeat(item, Until::End))]));
    let mut bytes = Vec::new();
    for i in 0..4u8 {
        bytes.extend([1, /* entries */ 1, 0xa0 + i, 2, 0xb0 + i, 0xb8 + i, 1, 0xc0 + i, /* tail */ 0xd0 + i]);
    }
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);

    // Reaching record 3 walks over records 0 to 2, and sizing each walks its
    // entries to the third. That inner walk keeps the second entry for the
    // third to ask, and the outer walk then lets record 0 go. The entry must
    // go with it: left behind, it has no record above it to look up to.
    ev.node(&d, &[0, 3]).unwrap();
    assert_eq!(ev.memo.without_parent(), Vec::<Vec<usize>>::new());
    // A value of record 0's second entry, asked for by its path. Where it
    // starts is worked out from `w`, which is in the record.
    let v = ev.node(&d, &[0, 0, 1, 1, 1, 1]).unwrap();
    assert_eq!((v.offset_bits, v.size_bits), (5 * 8, 8));
}

#[test]
fn an_element_that_ends_a_run_leaves_none_of_its_fields_behind() {
    // Records of a length and that many bytes. The third says nine, and nine
    // bytes are not there: its length is read before its bytes fail, and the
    // run ends before it.
    let rec = || T::structure("Rec", vec![("len", T::u8()), ("body", T::bytes(E::field("len")))]);
    let bytes = [1, 0xa0, 2, 0xb0, 0xb1, 9, 0xc0];

    // A run with no room outside it to try the element again in.
    let d = doc(&bytes);
    let mut ev = Evaluator::new(Template::new("t", T::repeat(rec(), Until::End)));
    assert_eq!(ev.node(&d, &[]).unwrap().child_count, 2);
    assert_eq!(ev.memo.without_parent(), Vec::<Vec<usize>>::new());

    // A run in a window one byte short of the file, which is tried again with
    // that byte and fails again.
    let t = T::structure("Root", vec![("items", T::sized(E::lit(6), T::repeat(rec(), Until::End))), ("tail", T::bytes(E::Remaining))]);
    let mut ev = Evaluator::new(Template::new("t", t));
    assert_eq!(ev.node(&d, &[0]).unwrap().child_count, 2);
    assert_eq!(ev.memo.without_parent(), Vec::<Vec<usize>>::new());
}

#[test]
fn an_overwrite_keeps_the_nodes_above_what_it_keeps() {
    // A length, three records of a byte and that many bytes in a room of
    // three each, and a tail for the edit to land in.
    let rec = T::sized(E::lit(3), T::structure("Rec", vec![("a", T::u8()), ("b", T::bytes(E::field("n")))]));
    let t = T::structure("Root", vec![("n", T::u8()), ("items", T::array(rec, E::lit(3))), ("tail", T::bytes(E::Remaining))]);
    let mut d = doc(&[2, 0xa0, 0xa1, 0xa2, 0xb0, 0xb1, 0xb2, 0xc0, 0xc1, 0xc2, 0xee, 0xee]);
    let mut ev = Evaluator::new(Template::new("t", t));
    // Record 1 is placed and sized by its room, with nothing inside it read
    // and nothing above it sized.
    assert_eq!(ev.node(&d, &[1, 1]).unwrap().offset_bits, 4 * 8);

    d.overwrite_bytes(11, &[0x55]);
    ev.invalidate_from(11 * 8);
    assert_eq!(ev.memo.without_parent(), Vec::<Vec<usize>>::new());
    // How long its second field is comes from `n`, found through the root.
    let b = ev.node(&d, &[1, 1, 1]).unwrap();
    assert_eq!((b.offset_bits, b.size_bits), (5 * 8, 2 * 8));
}

#[test]
fn a_node_kept_above_an_overwrite_is_measured_again() {
    // Records of a length and that many bytes, then a tail.
    let rec = T::structure("Rec", vec![("a", T::u8()), ("b", T::bytes(E::field("a")))]);
    let t = || Template::new("t", T::structure("Root", vec![("n", T::u8()), ("items", T::array(rec.clone(), E::lit(3))), ("tail", T::bytes(E::Remaining))]));
    let mut d = doc(&[3, 1, 0xa0, 2, 0xb0, 0xb1, 1, 0xc0, 0xee, 0xee, 0xee, 0xee]);
    let mut ev = Evaluator::new(t());
    assert_eq!(ev.node(&d, &[2]).unwrap().offset_bits, 8 * 8);

    // Records 1 and 2 shorter. Record 0 ended before them and stays, and so
    // do the list and the root, but the list is not the length it was.
    d.overwrite_bytes(3, &[1, 0xb0, 0]);
    ev.invalidate_from(3 * 8);
    assert_eq!(ev.memo.without_parent(), Vec::<Vec<usize>>::new());
    assert!(ev.memo.contains_key(&[1, 0]));
    let mut fresh = Evaluator::new(t());
    for path in [&[1][..], &[2], &[1, 2, 1], &[]] {
        assert_eq!(ev.node(&d, path).unwrap(), fresh.node(&d, path).unwrap(), "{path:?}");
    }
    assert_eq!(ev.node(&d, &[1]).unwrap().size_bits, 5 * 8);
}

#[test]
fn an_overwrite_of_a_pointer_drops_what_it_pointed_at() {
    // A byte, a pointer to a byte, and the pointed-at byte, declared after the
    // pointer but lying before it.
    let t = T::structure("Root", vec![("hdr", T::u8()), ("p", T::u8()), ("pad", T::u8()), ("t", T::at(E::field("p"), T::u8()))]);
    let mut d = doc(&[0x11, 0, 0x33]);
    let mut ev = Evaluator::new(Template::new("t", t));
    assert_eq!(ev.node(&d, &[3, 0]).unwrap().value, Value::UInt(0x11));

    // The byte it points at ended before the edit, but where it is was read
    // after it.
    d.overwrite_bytes(1, &[2]);
    ev.invalidate_from(8);
    assert_eq!(ev.memo.without_parent(), Vec::<Vec<usize>>::new());
    let pointed = ev.node(&d, &[3, 0]).unwrap();
    assert_eq!((pointed.offset_bits, pointed.value), (2 * 8, Value::UInt(0x33)));
}

#[test]
fn an_overwrite_of_an_element_that_overran_its_room_gives_the_room_back() {
    // Records of a length and that many bytes, in a room one byte short of the
    // second one, which is read past the room since there is file outside it.
    let rec = T::structure("Rec", vec![("len", T::u8()), ("body", T::bytes(E::field("len")))]);
    let t = || Template::new("t", T::structure("Root", vec![("items", T::sized(E::lit(4), T::repeat(rec.clone(), Until::End))), ("tail", T::bytes(E::Remaining))]));
    let mut d = doc(&[1, 0xa0, 2, 0xb0, 0xb1, 0xee, 0xee, 0xee]);
    let mut ev = Evaluator::new(t());
    assert_eq!(ev.node(&d, &[0]).unwrap().child_count, 2);
    assert_eq!(ev.node(&d, &[1]).unwrap().offset_bits, 5 * 8);

    // The second record now fits the room, so the room is what the template
    // said it was.
    d.overwrite_bytes(2, &[1]);
    ev.invalidate_from(2 * 8);
    assert_eq!(ev.memo.without_parent(), Vec::<Vec<usize>>::new());
    let mut fresh = Evaluator::new(t());
    for path in [&[0][..], &[1], &[0, 1]] {
        assert_eq!(ev.node(&d, path).unwrap(), fresh.node(&d, path).unwrap(), "{path:?}");
    }
    assert_eq!(ev.node(&d, &[1]).unwrap().offset_bits, 4 * 8);
}

#[test]
fn an_overwrite_inside_json_drops_every_value_the_parse_placed() {
    // The first element of `a` ends before the edit, but where `a` ends is
    // where the member after it starts, and the edit moves that.
    let text = br#"{"a":[1,2,3],"c":4}"#;
    let mut d = doc(text);
    let mut ev = Evaluator::new(Template::new("json", T::json()));
    ev.node(&d, &[0]).unwrap();
    ev.node(&d, &[0, 0]).unwrap();

    d.overwrite_bytes(9, br#"],"d":5}  "#);
    ev.invalidate_from(9 * 8);
    assert_eq!(ev.memo.without_parent(), Vec::<Vec<usize>>::new());
    let mut fresh = Evaluator::new(Template::new("json", T::json()));
    for path in [&[0][..], &[0, 0], &[1]] {
        assert_eq!(ev.node(&d, path).unwrap(), fresh.node(&d, path).unwrap(), "{path:?}");
    }
}

#[test]
fn the_field_under_a_bit_is_found_without_the_list_coming_back() {
    // The same long list of uneven strings, asked the question the hex cursor
    // asks: what is under this bit, in the middle of ten thousand elements.
    let n = 20_000u64;
    let mut bytes = n.to_le_bytes().to_vec();
    let mut starts = vec![8u64];
    for i in 0..n {
        let s = "y".repeat((i % 13) as usize + 1);
        bytes.extend_from_slice(&(s.len() as u64).to_le_bytes());
        bytes.extend_from_slice(s.as_bytes());
        starts.push(bytes.len() as u64);
    }
    let string = T::structure("String", vec![("len", T::u64(Little)), ("text", T::utf8(E::field("len")))]);
    let t = Template::new(
        "t",
        T::structure("Root", vec![("n", T::u64(Little)), ("items", T::array(string, E::field("n")))]),
    );
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);
    ev.node(&d, &[1]).unwrap();

    // A bit inside element 12,345: the length, and then a byte of its text.
    let elem = 12_345usize;
    assert_eq!(ev.locate(&d, starts[elem] * 8).unwrap(), vec![1, elem, 0]);
    assert_eq!(ev.locate(&d, (starts[elem] + 9) * 8).unwrap(), vec![1, elem, 1]);
    // The answer came from a walk between checkpoints, not from putting twelve
    // thousand elements back in the memo.
    assert!(ev.memo_len() < 5_000, "locating kept {} nodes", ev.memo_len());
}

#[test]
fn an_overwrite_keeps_what_it_could_not_have_changed() {
    // A count, then that many strings of uneven length, then a run of bytes
    // standing in for the part of a file an edit usually lands in.
    let n = 20_000u64;
    let mut bytes = n.to_le_bytes().to_vec();
    for i in 0..n {
        let s = "z".repeat((i % 11) as usize + 1);
        bytes.extend_from_slice(&(s.len() as u64).to_le_bytes());
        bytes.extend_from_slice(s.as_bytes());
    }
    let list_end = bytes.len() as u64;
    bytes.extend_from_slice(&[0x11; 64]);
    let string = T::structure("String", vec![("len", T::u64(Little)), ("text", T::utf8(E::field("len")))]);
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("n", T::u64(Little)),
                ("items", T::array(string, E::field("n"))),
                ("tail", T::bytes(E::Remaining)),
            ],
        ),
    );
    let mut d = doc(&bytes);
    let mut ev = Evaluator::new(t);
    assert_eq!(ev.node(&d, &[2]).unwrap().offset_bits, list_end * 8);
    let after_sizing = ev.memo_len();

    // An overwrite in the tail: everything the walk learned about the list
    // still holds, so asking where the tail starts does not walk it again.
    d.overwrite_bytes(list_end + 8, &[0x22]);
    ev.invalidate_from((list_end + 8) * 8);
    assert_eq!(ev.memo.without_parent(), Vec::<Vec<usize>>::new());
    assert!(ev.memo_len() >= after_sizing / 2, "the edit threw away work it did not have to");
    assert_eq!(ev.node(&d, &[2]).unwrap().offset_bits, list_end * 8);
    assert!(ev.memo_len() < 200, "and it is still not the whole list: {}", ev.memo_len());

    // The edited bytes read as what was written, and a field before the edit
    // still reads as what the file says.
    let tail = ev.node(&d, &[2]).unwrap();
    let Value::Bytes { preview, .. } = tail.value else { panic!("not bytes") };
    assert_eq!(preview[8], 0x22);
    assert_eq!(ev.node(&d, &[1, 5, 1]).unwrap().value, Value::Str("zzzzzz".into()));

    // An overwrite inside the list drops what came after it and keeps the rest.
    let kept = ev.memo_len();
    ev.invalidate_from(8 * 8);
    assert_eq!(ev.memo.without_parent(), Vec::<Vec<usize>>::new());
    assert!(ev.memo_len() < kept);
    assert_eq!(ev.node(&d, &[2]).unwrap().offset_bits, list_end * 8);
}

#[test]
fn an_edit_that_moves_bytes_throws_the_whole_memo_away() {
    let bytes = [4u8, 0, 0, 0, 1, 2, 3, 4];
    let t = Template::new(
        "t",
        T::structure("Root", vec![("n", T::u32(Little)), ("data", T::bytes(E::field("n")))]),
    );
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);
    ev.node(&d, &[1]).unwrap();
    assert!(ev.memo_len() > 0);
    ev.invalidate();
    assert_eq!(ev.memo_len(), 0);
}

#[test]
fn work_done_in_goes_reaches_the_same_answer() {
    // A list long enough that a small allowance cannot finish it in one go.
    let n = 20_000u64;
    let mut bytes = n.to_le_bytes().to_vec();
    for i in 0..n {
        let s = "w".repeat((i % 7) as usize + 1);
        bytes.extend_from_slice(&(s.len() as u64).to_le_bytes());
        bytes.extend_from_slice(s.as_bytes());
    }
    let end = bytes.len() as u64;
    let string = T::structure("String", vec![("len", T::u64(Little)), ("text", T::utf8(E::field("len")))]);
    let t = Template::new(
        "t",
        T::structure("Root", vec![("n", T::u64(Little)), ("items", T::array(string, E::field("n")))]),
    );
    let d = doc(&bytes);

    let mut whole = Evaluator::new(t.clone());
    let want = whole.node(&d, &[1]).unwrap();
    assert_eq!(want.offset_bits + want.size_bits, end * 8);

    // The same question, answered five hundred elements at a time.
    let mut sliced = Evaluator::new(t);
    sliced.set_slice(Some(500));
    let mut goes = 0;
    let got = loop {
        goes += 1;
        assert!(goes < 500, "asking again is not getting anywhere");
        sliced.begin_slice();
        match sliced.node(&d, &[1]) {
            Ok(node) => break node,
            Err(EvalError::Busy { reached_bits }) => {
                // Each go says how far it has got, and it only ever goes forward.
                assert!(reached_bits <= end * 8);
                assert_eq!(reached_bits, sliced.reached_bits());
                let estimate = sliced.extent_estimate().expect("the array walk has a projection");
                assert_eq!(estimate.path, vec![1]);
                assert_eq!(estimate.total_items, n);
                assert!(estimate.measured_items > 0 && estimate.measured_items < n);
                // The elements vary from nine to fifteen bytes. An average of
                // the prefix should stay comfortably around the actual total.
                assert!(estimate.estimated_bits > want.size_bits / 2);
                assert!(estimate.estimated_bits < want.size_bits * 2);
            }
            Err(e) => panic!("{e:?}"),
        }
    };
    assert!(goes > 5, "a small allowance should have taken several goes, took {goes}");
    assert_eq!(got.offset_bits, want.offset_bits);
    assert_eq!(got.size_bits, want.size_bits);
    assert_eq!(got.child_count, want.child_count);
    assert!(sliced.memo_len() < 500, "and it is still bounded: {}", sliced.memo_len());

    // Reading one element after the goes is the same either way.
    sliced.begin_slice();
    let mut mid = loop {
        sliced.begin_slice();
        match sliced.node(&d, &[1, 9_999, 1]) {
            Ok(node) => break node,
            Err(EvalError::Busy { .. }) => continue,
            Err(e) => panic!("{e:?}"),
        }
    };
    let want_mid = whole.node(&d, &[1, 9_999, 1]).unwrap();
    mid.path.clone_from(&want_mid.path);
    assert_eq!(mid.value, want_mid.value);
    assert_eq!(mid.offset_bits, want_mid.offset_bits);
}

#[test]
fn the_chunk_read_longest_ago_is_the_one_that_goes() {
    use crate::source::ChunkStore;
    // Room for two chunks. The first is read again and again; the second is
    // loaded and left alone. Loading a third must take the idle one.
    let mut store = ChunkStore::new(3 * 8, 8, 2);
    store.insert(0, vec![1u8; 8].into_boxed_slice());
    store.insert(1, vec![2u8; 8].into_boxed_slice());
    let mut buf = [0u8; 8];
    for _ in 0..3 {
        assert!(store.read_bytes(0, &mut buf).is_empty());
    }
    store.insert(2, vec![3u8; 8].into_boxed_slice());
    assert!(store.has(0), "the chunk being read is the one to keep");
    assert!(!store.has(1), "the one nothing has looked at since it arrived goes");
    assert!(store.has(2));
}
