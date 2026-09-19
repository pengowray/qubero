//! The pandas productions: the calls a frame is assembled from, the array of
//! objects a column of names is, and the calls the form turns away.
//!
//! The fixtures here are the shapes read out of the corpus rather than whole
//! frames, which `crates/core/tests/pickle_real.rs` covers file by file.

use super::*;

/// `STACK_GLOBAL` of a module and a name, and the memo mark after it.
fn class(module: &str, name: &str) -> Vec<u8> {
    cat(&[&word(module), &word(name), b"\x93\x94"])
}

/// A frame: the class, the object NEWOBJ makes, a state dictionary holding
/// `_mgr`, and the BUILD that hands it over.
fn frame(mgr: &[u8]) -> Vec<u8> {
    cat(&[&class("pandas", "DataFrame"), b")\x81\x94}\x94", &word("_mgr"), mgr, b"sb."])
}

/// An array of objects, which is what a column of names is: the `O8` dtype and
/// the list of values pickled after it.
fn object_array(names: &[&str]) -> Vec<u8> {
    let mut w = Writing::default();
    w.word("numpy._core.multiarray");
    w.word("_reconstruct");
    w.raw(b"\x93");
    w.mark();
    w.word("numpy");
    w.word("ndarray");
    w.raw(b"\x93");
    w.mark();
    w.raw(b"K\0\x85");
    w.mark();
    w.raw(b"C\x01b");
    w.mark();
    w.raw(b"\x87");
    w.mark();
    w.raw(b"R");
    w.mark();
    w.raw(b"(K\x01");
    w.count(names.len() as u64);
    w.raw(b"\x85");
    w.mark();
    w.word("numpy");
    w.word("dtype");
    w.raw(b"\x93");
    w.mark();
    w.word("O8");
    w.raw(b"\x89\x88\x87");
    w.mark();
    w.raw(b"R");
    w.mark();
    w.raw(b"(K\x03");
    w.word("|");
    w.raw(b"NNNJ\xff\xff\xff\xffJ\xff\xff\xff\xffK\x3ft");
    w.mark();
    w.raw(b"b\x89]");
    w.mark();
    match names {
        [] => {}
        [one] => {
            w.word(one);
            w.raw(b"a");
        }
        many => {
            w.raw(b"(");
            for name in many {
                w.word(name);
            }
            w.raw(b"e");
        }
    }
    w.raw(b"t");
    w.mark();
    w.raw(b"b");
    w.out
}

/// `pandas.core.indexes.base._new_Index(Index, {"data": <array>, "name": None})`,
/// which is how a frame writes the names of its columns.
fn names_index(names: &[&str]) -> Vec<u8> {
    cat(&[
        &class("pandas.core.indexes.base", "_new_Index"),
        &class("pandas", "Index"),
        b"}\x94",
        &word("data"),
        &object_array(names),
        b"s\x86\x94R\x94",
    ])
}

#[test]
fn a_column_of_names_is_an_array_of_objects() {
    // On its own an array of objects is what `pickle.dumps` writes for one,
    // which is NumPy's own writing and the NumPy form's to read.
    let bytes = framed(&cat(&[&object_array(&["id", "score"]), b"."]));
    assert_eq!(recognise(&bytes).unwrap().form, "numpy-array-p4-p5-v6");
    let bytes = framed(&frame(&names_index(&["id", "score"])));
    let found = recognise(&bytes).unwrap();
    assert_eq!(found.form, "pandas-frame-p4-p5-v1");
    let Kind::Instance { state: Some(state), .. } = &found.value.kind else { panic!("object expected") };
    let Kind::Dict(entries) = &state.kind else { panic!("state expected") };
    let Kind::Made { names, items, .. } = &entries[0].1.kind else { panic!("call expected") };
    assert_eq!(*names, ["type", "state"]);
    let Kind::Dict(index) = &items[1].kind else { panic!("index state expected") };
    let Kind::Objects { dimensions, items, .. } = &index[0].1.kind else { panic!("object array expected") };
    assert_eq!(dimensions, &[2]);
    assert_eq!(items.len(), 2);
    assert!(matches!(items[0].kind, Kind::Text { .. }));
}

/// The values have to come to the shape the array declares, and a shape that
/// counts them wrong is a non-match rather than a short read.
#[test]
fn an_array_of_objects_holds_as_many_values_as_its_shape_says() {
    let three = object_array(&["id", "score", "extra"]);
    let miscounted = {
        let mut bytes = three.clone();
        // The one dimension of the shape, written as BININT1 3.
        let at = bytes.windows(2).rposition(|w| w == b"K\x03").unwrap();
        bytes[at + 1] = 2;
        bytes
    };
    assert!(recognise(&framed(&frame(&names_index(&["id", "score", "extra"])))).is_some());
    assert!(recognise(&framed(&frame(&cat(&[&class("pandas.core.indexes.base", "_new_Index"), &class("pandas", "Index"), b"}\x94", &word("data"), &miscounted, b"s\x86\x94R\x94"])))).is_none());
    // A value of a kind the form has not written down, which here is a number
    // where a name belongs.
    let numbered = {
        let one = object_array(&["id"]);
        let at = one.windows(4).rposition(|w| w == b"\x8c\x02id").unwrap();
        cat(&[&one[..at], b"K\x01\x94", &one[at + 5..]])
    };
    assert!(recognise(&framed(&cat(&[&numbered, b"."]))).is_none());
}

/// A REDUCE of a pandas global the form does not list, with arguments that
/// look like a call the form does list. The module prefix names classes; it
/// never says which of a library's callables may be called.
#[test]
fn a_reduce_of_an_unlisted_pandas_callable_is_a_non_match() {
    for (module, name) in [
        ("pandas.io.pickle", "read_pickle"),
        ("pandas.core.internals.blocks", "new_block"),
        ("pandas.core.indexes.base", "_new_index"),
    ] {
        let call = cat(&[&class(module, name), &word("/etc/passwd"), b"\x85\x94R\x94"]);
        let bytes = framed(&frame(&call));
        assert!(recognise(&bytes).is_none(), "{module}.{name}");
    }
}

/// The arguments of a block are checked against what pandas writes. A block
/// placed by an array of positions rather than a slice is a non-match until
/// there is a file with one in it.
#[test]
fn a_block_is_values_a_slice_and_a_number() {
    let slice = cat(&[&class("builtins", "slice"), b"K\0K\x01K\x01\x87\x94R\x94"]);
    let good = cat(&[
        &class("pandas._libs.internals", "_unpickle_block"),
        &object_array(&["a", "b"]),
        &slice,
        b"K\x02\x87\x94R\x94",
    ]);
    let blocks = cat(&[&class("pandas.core.internals.managers", "BlockManager"), &good, b"\x85\x94]\x94\x86\x94R\x94"]);
    assert!(recognise(&framed(&frame(&blocks))).is_some());
    // The same block with the placement left out, which is two arguments where
    // pandas writes three.
    let short = cat(&[
        &class("pandas._libs.internals", "_unpickle_block"),
        &object_array(&["a", "b"]),
        b"K\x02\x86\x94R\x94",
    ]);
    assert!(recognise(&framed(&frame(&short))).is_none());
    // And with a number where the values belong.
    let bare = cat(&[&class("pandas._libs.internals", "_unpickle_block"), b"K\x01", &slice, b"K\x02\x87\x94R\x94"]);
    assert!(recognise(&framed(&frame(&bare))).is_none());
}

/// A date is a count of the unit its dtype names, and the unit is in the
/// dtype's state rather than in its letters. So a datetime dtype is version 4
/// with a ninth part, and everything else is version 3 with eight.
#[test]
fn a_datetime_dtype_carries_the_unit_it_counts_in() {
    let dtype = |version: &[u8], unit: &str, beside: &[u8]| {
        let mut w = Writing::default();
        w.word("numpy");
        w.word("dtype");
        w.raw(b"\x93");
        w.mark();
        w.word("M8");
        w.raw(b"\x89\x88\x87");
        w.mark();
        w.raw(b"R");
        w.mark();
        w.raw(b"(");
        w.raw(version);
        w.word("<");
        w.raw(b"NNNJ\xff\xff\xff\xffJ\xff\xff\xff\xffK\0");
        w.raw(beside);
        w.raw(b"(C");
        w.raw(&[unit.len() as u8]);
        w.raw(unit.as_bytes());
        w.mark();
        w.raw(b"K\x01K\x01K\x01t");
        w.mark();
        w.raw(b"\x86");
        w.mark();
        w.raw(b"t");
        w.mark();
        w.raw(b"b");
        w.out
    };
    // Standing on its own, which is where pandas writes one beside a column's
    // numbers. `None` beside the unit is numpy 2.x and an empty dictionary is
    // 1.x; both are read.
    for beside in [&b"N"[..], b"}\x94"] {
        let bytes = framed(&cat(&[&frame(&dtype(b"K\x04", "ns", beside)), b""]));
        let found = recognise(&bytes).unwrap();
        let Kind::Instance { state: Some(state), .. } = &found.value.kind else { panic!("object expected") };
        let Kind::Dict(entries) = &state.kind else { panic!("state expected") };
        let Kind::DType(dtype) = &entries[0].1.kind else { panic!("dtype expected") };
        assert_eq!(dtype.name(), "datetime64[ns]");
    }
    // Version 3, which is what every dtype with nothing to say writes, and a
    // unit numpy does not have.
    assert!(recognise(&framed(&frame(&dtype(b"K\x03", "ns", b"N")))).is_none());
    assert!(recognise(&framed(&frame(&dtype(b"K\x04", "zz", b"N")))).is_none());
}

/// pandas 1.3 writes a block as a `functools.partial` over `new_block` and
/// then calls the partial, which is a REDUCE of what another REDUCE made.
///
/// That is the one call whose callable is a call, and it is a list of two
/// rather than a rule: the partial may be made over one global and only what
/// the partial made may be called. A partial over anything else is a
/// non-match, which is what keeps this from being a way in.
#[test]
fn a_block_may_be_a_partial_over_new_block_and_nothing_else_may() {
    let partial = |over: &str| {
        cat(&[
            &class("functools", "partial"),
            &class("pandas.core.internals.blocks", over),
            b"\x85\x94R\x94",
            // The state a partial writes: the callable, no arguments, the
            // keywords it was made with, and no dictionary of its own.
            b"(",
            &get(5),
            b")}\x94",
            &word("ndim"),
            b"K\x02sNt\x94b",
        ])
    };
    let slice = cat(&[&class("builtins", "slice"), b"K\0K\x01K\x01\x87\x94R\x94"]);
    let block = |over: &str| cat(&[&partial(over), &object_array(&["a", "b"]), &slice, b"\x86\x94R\x94"]);
    let manager = |over: &str| {
        cat(&[&class("pandas.core.internals.managers", "BlockManager"), &block(over), b"\x85\x94]\x94\x86\x94R\x94"])
    };
    assert!(recognise(&framed(&frame(&manager("new_block")))).is_some());
    // A partial over any other global, including one that looks like a block
    // maker and one that is not pandas' at all.
    for over in ["make_block", "new_block_2d", "Block"] {
        assert!(recognise(&framed(&frame(&manager(over)))).is_none(), "{over}");
    }
    let elsewhere = cat(&[
        &class("functools", "partial"),
        &class("os", "system"),
        b"\x85\x94R\x94(",
        &get(5),
        b")}\x94Nt\x94b",
    ]);
    assert!(recognise(&framed(&frame(&elsewhere))).is_none());
}
