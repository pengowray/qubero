use super::*;

// ----- the whole web of connections under one node -----

/// A count in front of a run and a length in front of some bytes: the two
/// shapes nearly every format is made of, and the two arrows a graph has to
/// get the right way round.
fn connected() -> Template {
    Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("n", T::u8()),
                ("len", T::u8()),
                ("items", T::array(T::u16(Big), E::field("n"))),
                ("data", T::bytes(E::field("len"))),
            ],
        ),
    )
}

/// Where the node with this path ended up in the list, so that a test can name
/// a field the way the template does rather than by counting the walk.
fn node_at(g: &Graph, path: &[usize]) -> usize {
    g.nodes.iter().position(|n| n.path == path).unwrap_or_else(|| panic!("no node at {path:?}"))
}

/// Every edge as the two fields it joins, which is what the assertion is
/// about: the indices are an artefact of the order the walk happened to run in.
fn joined(g: &Graph) -> Vec<(&[usize], &[usize], &'static str)> {
    g.edges.iter().map(|e| (g.nodes[e.from].path.as_slice(), g.nodes[e.to].path.as_slice(), e.role)).collect()
}

#[test]
fn a_graph_draws_an_arrow_from_the_field_that_decided() {
    // Two items and three bytes of data.
    let d = doc(&[2, 3, 0, 1, 0, 2, 7, 7, 7]);
    let mut ev = Evaluator::new(connected());
    let g = ev.graph(&d, &[], 100).unwrap();

    // The root, its four fields, and then the two items: breadth-first, so
    // every field arrives before anything inside one of them.
    let paths: Vec<&[usize]> = g.nodes.iter().map(|n| n.path.as_slice()).collect();
    assert_eq!(paths, vec![&[][..], &[0], &[1], &[2], &[3], &[2, 0], &[2, 1]]);
    assert_eq!(g.omitted, 0);
    assert!(g.nodes.iter().all(|n| !n.truncated));

    // What each node is called and where it sits comes from the memo, without
    // reading a single field's value.
    assert_eq!(g.nodes[node_at(&g, &[2])].name, "items");
    assert_eq!(g.nodes[node_at(&g, &[2, 1])].name, "[1]");
    assert_eq!(g.nodes[node_at(&g, &[3])].offset_bits / 8, 6);
    assert_eq!(g.nodes[node_at(&g, &[3])].size_bits / 8, 3);
    assert_eq!(g.nodes[node_at(&g, &[2])].child_count, 2);

    // The root has nothing above it; everything else names the node it hangs
    // under.
    assert_eq!(g.nodes[0].parent, NO_PARENT);
    assert_eq!(g.nodes[node_at(&g, &[2, 0])].parent, node_at(&g, &[2]));

    // The kinds a view groups by: the resolved type, said coarsely.
    let kinds: Vec<&str> = g.nodes.iter().map(|n| n.kind.as_str()).collect();
    assert_eq!(kinds, vec!["struct", "u8", "u8", "array", "bytes", "u16", "u16"]);

    // And the arrows, each running from the field that decided to the field it
    // decided about.
    assert_eq!(joined(&g), vec![(&[0][..], &[2][..], "count"), (&[1][..], &[3][..], "length")]);
}

#[test]
fn an_arrow_from_outside_the_subtree_is_dropped_rather_than_left_hanging() {
    // The same file, asked only about the run. What said how many items there
    // are is a field of the root, which is not in this graph, so the arrow has
    // nowhere to come from and is left out rather than pointed at nothing.
    let d = doc(&[2, 3, 0, 1, 0, 2, 7, 7, 7]);
    let mut ev = Evaluator::new(connected());
    let g = ev.graph(&d, &[2], 100).unwrap();
    let paths: Vec<&[usize]> = g.nodes.iter().map(|n| n.path.as_slice()).collect();
    assert_eq!(paths, vec![&[2][..], &[2, 0], &[2, 1]]);
    assert!(g.edges.is_empty(), "{:?}", joined(&g));
    // The subtree's own node is its root here, whatever it is a child of in
    // the file.
    assert_eq!(g.nodes[0].parent, NO_PARENT);
}

#[test]
fn a_capped_graph_keeps_the_shape_and_says_what_it_left_out() {
    let d = doc(&[2, 3, 0, 1, 0, 2, 7, 7, 7]);
    let mut ev = Evaluator::new(connected());
    let g = ev.graph(&d, &[], 2).unwrap();
    // Two nodes, and they are the top two. A cap on a depth-first walk would
    // have kept the root and then dived into the first field, and said nothing
    // about the shape of the file.
    let paths: Vec<&[usize]> = g.nodes.iter().map(|n| n.path.as_slice()).collect();
    assert_eq!(paths, vec![&[][..], &[0]]);
    // The root still says how many fields it has, and says that most of them
    // were not walked.
    assert_eq!(g.nodes[0].child_count, 4);
    assert!(g.nodes[0].truncated);
    assert!(!g.nodes[1].truncated);
    assert_eq!(g.omitted, 3);
    // With the deciding fields' targets outside the cap there is nothing for
    // an arrow to join.
    assert!(g.edges.is_empty(), "{:?}", joined(&g));
}

/// The same connections `origins` gives, minus what the fields say. This is
/// what makes a graph of a large file affordable: reading the values is the
/// expensive half, and no arrow shows them.
#[test]
fn a_graph_asks_for_the_connections_without_reading_the_values() {
    let d = doc(&[2, 3, 0, 1, 0, 2, 7, 7, 7]);
    let mut ev = Evaluator::new(connected());
    let told = ev.origins(&d, &[3]).unwrap();
    assert_eq!(told.len(), 1);
    assert_eq!((told[0].role, told[0].label.as_str(), told[0].value.as_str()), (Role::Length, "len", "3"));

    let mut bare = Evaluator::new(connected());
    let quiet = bare.origins_no_values(&d, &[3]).unwrap();
    assert_eq!(quiet.len(), 1);
    assert_eq!((quiet[0].role, quiet[0].label.as_str(), quiet[0].value.as_str()), (Role::Length, "len", ""));
    assert_eq!(quiet[0].path, told[0].path);
}

/// A pointer list's offset and the child it placed are two ends of one
/// connection, and the graph draws both: the offset placed the record, and the
/// offset points at it. The second is the only arrow that runs outward from
/// the field holding the number rather than inward to it.
#[test]
fn a_pointer_and_what_it_points_at_are_joined_both_ways() {
    let d = doc(POINTED);
    let mut ev = Evaluator::new(pointer_template());
    let g = ev.graph(&d, &[], 100).unwrap();
    let seen = joined(&g);
    assert!(seen.contains(&(&[1, 0][..], &[2, 0][..], "position")), "{seen:?}");
    assert!(seen.contains(&(&[1, 0][..], &[2, 0][..], "points")), "{seen:?}");
    assert!(seen.contains(&(&[1, 1][..], &[2, 1][..], "position")), "{seen:?}");
    // And a length inside one of the records reads the same as any other.
    assert!(seen.contains(&(&[2, 0, 0][..], &[2, 0, 1][..], "length")), "{seen:?}");
}
