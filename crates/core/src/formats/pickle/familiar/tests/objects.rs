//! The library object productions: the class a form may name, the object it
//! makes with no arguments, the attributes a BUILD gives it, and the calls it
//! refuses. The refusals are the point: a module prefix says which classes may
//! be named, and it never says which callables may be run.

use super::*;

/// `STACK_GLOBAL` of a module and a name, and the memo mark after it.
///
/// Counting from the slot the module word lands in: 0 is the module, 1 is the
/// name, 2 is the pair.
fn class(module: &str, name: &str) -> Vec<u8> {
    cat(&[&word(module), &word(name), b"\x93\x94"])
}

/// An object of a class with the attributes in `state`, which is the whole of
/// the plain object production: the class, `EMPTY_TUPLE NEWOBJ`, a dictionary
/// and `BUILD`.
fn object(module: &str, name: &str, state: &[u8]) -> Vec<u8> {
    cat(&[&class(module, name), b")\x81\x94", state, b"b"])
}

/// A dictionary of one text key to one value, written the way CPython writes a
/// dictionary of one entry.
fn one_entry(key: &str, value: &[u8]) -> Vec<u8> {
    cat(&[b"}\x94", &word(key), value, b"s"])
}

#[test]
fn reads_an_estimator_as_its_class_and_its_attributes() {
    let bytes = framed(&cat(&[&object("sklearn.dummy", "Thing", &one_entry("copy", b"\x88")), b"."]));
    let found = recognise(&bytes).unwrap();
    assert_eq!(found.form, "sklearn-estimator-p4-p5-v1");
    let Kind::Instance { class, state } = &found.value.kind else { panic!("object expected") };
    let Kind::Class { path, parts } = &class.kind else { panic!("class expected") };
    assert_eq!(path, "sklearn.dummy.Thing");
    // The module and the name are the two words the file spelled, read as the
    // text they are rather than folded into the instruction.
    assert_eq!(parts.len(), 2);
    let Kind::Dict(entries) = &state.as_ref().unwrap().kind else { panic!("state expected") };
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].1.kind, Kind::Bool(true));
}

/// A sparse matrix is the same production under another prefix, and is read
/// under its own library's form.
#[test]
fn a_scipy_matrix_is_read_under_the_scipy_form() {
    let bytes = framed(&cat(&[&object("scipy.sparse._csr", "csr_matrix", &one_entry("maxprint", b"K2")), b"."]));
    assert_eq!(recognise(&bytes).unwrap().form, "scipy-sparse-p4-p5-v1");
    // The prefix is the module's and not the name's: `scipy.linalg` is not
    // `scipy.sparse`, and no form names a class from it.
    let other = framed(&cat(&[&object("scipy.linalg", "Thing", &one_entry("n", b"K2")), b"."]));
    assert!(recognise(&other).is_none());
}

/// A second object of the same class names the slot the first one filed it in
/// rather than spelling it again, which is what a pipeline of two estimators
/// does.
#[test]
fn a_class_may_be_named_out_of_the_memo() {
    let first = object("sklearn.dummy", "Thing", &one_entry("n", b"K\x01"));
    // The list itself takes slot 0, so the pair the first STACK_GLOBAL made
    // is slot 3.
    let again = cat(&[&get(3), b")\x81\x94", &one_entry("n", b"K\x02"), b"b"]);
    let bytes = framed(&cat(&[b"]\x94(", &first, &again, b"e."]));
    let found = recognise(&bytes).unwrap();
    assert_eq!(found.form, "sklearn-estimator-p4-p5-v1");
    let Kind::List(items) = &found.value.kind else { panic!("list expected") };
    assert_eq!(items.len(), 2);
    for item in items {
        let Kind::Instance { class, .. } = &item.kind else { panic!("object expected") };
        let Kind::Class { path, .. } = &class.kind else { panic!("class expected") };
        assert_eq!(path, "sklearn.dummy.Thing");
    }
    // The second class was named, not spelled, so it has no words of its own.
    let Kind::Instance { class, .. } = &items[1].kind else { panic!("object expected") };
    let Kind::Class { parts, .. } = &class.kind else { panic!("class expected") };
    assert!(parts.is_empty());
}

/// A module outside every form's prefixes is not a class any form names, so a
/// file naming one matches nothing however ordinary the rest of it is.
#[test]
fn a_class_outside_the_whitelist_is_a_non_match() {
    for module in ["os", "posix", "subprocess", "sklearnish", "numpy.core.multiarray"] {
        let bytes = framed(&cat(&[&object(module, "Thing", &one_entry("n", b"K\x01")), b"."]));
        assert!(recognise(&bytes).is_none(), "{module}");
    }
}

/// A REDUCE of a global under a whitelisted module is still a call, and no
/// form has said which of a library's callables do work when they are called.
/// So the module prefix names classes and never authorises a call.
#[test]
fn a_reduce_of_an_unnamed_callable_is_a_non_match() {
    for (module, name) in [("sklearn.utils", "check_array"), ("sklearn.dummy", "Thing"), ("scipy.sparse._csr", "csr_matrix")] {
        // The callable, an empty tuple of arguments, and the call.
        let bytes = framed(&cat(&[&class(module, name), b")R\x94."]));
        assert!(recognise(&bytes).is_none(), "{module}.{name}");
        // And with an argument, which is the shape a payload would have.
        let with_arg = framed(&cat(&[&class(module, name), &word("rm -rf /"), b"\x85\x94R\x94."]));
        assert!(recognise(&with_arg).is_none(), "{module}.{name} of one argument");
    }
}

/// A call to something outside the whitelist, hidden where an estimator's
/// attributes go. The state dictionary is data, and a value inside it is read
/// by the same productions as any other value, so this is turned away where a
/// top-level one is.
#[test]
fn a_command_dressed_as_an_estimator_s_state_is_a_non_match() {
    let payload = cat(&[&class("os", "system"), &word("rm -rf /"), b"\x85\x94R\x94"]);
    let bytes = framed(&cat(&[&object("sklearn.dummy", "Thing", &one_entry("copy", &payload)), b"."]));
    assert!(recognise(&bytes).is_none());
    // The same with `posix.system`, which is what the module is really called,
    // and with a module that only looks like a whitelisted one.
    for module in ["posix", "sklearn.__init__.os", "nt"] {
        let payload = cat(&[&class(module, "system"), &word("rm -rf /"), b"\x85\x94R\x94"]);
        let bytes = framed(&cat(&[&object("sklearn.dummy", "Thing", &one_entry("copy", &payload)), b"."]));
        assert!(recognise(&bytes).is_none(), "{module}");
    }
}

/// An object is made with no arguments and given a dictionary of attributes.
/// A NEWOBJ handed values, or a BUILD handed anything but a dictionary, is a
/// class being asked to construct itself, which is the class's business.
#[test]
fn only_an_empty_tuple_makes_an_object_and_only_a_dict_fills_one() {
    let with_args = framed(&cat(&[&class("sklearn.dummy", "Thing"), b"K\x01\x85\x94\x81\x94", &one_entry("n", b"K\x01"), b"b."]));
    assert!(recognise(&with_args).is_none());
    let list_state = framed(&cat(&[&class("sklearn.dummy", "Thing"), b")\x81\x94]\x94K\x01ab."]));
    assert!(recognise(&list_state).is_none());
    // Two BUILDs, the second of which has no object under it that wants one.
    let twice = framed(&cat(&[&object("sklearn.dummy", "Thing", &one_entry("n", b"K\x01")), b"}\x94b."]));
    assert!(recognise(&twice).is_none());
}

/// An object with nothing to set writes no BUILD at all, which is a complete
/// object and not a truncated one.
#[test]
fn an_object_with_no_attributes_needs_no_build() {
    let bytes = framed(&cat(&[&class("sklearn.dummy", "Thing"), b")\x81\x94."]));
    let found = recognise(&bytes).unwrap();
    assert!(matches!(found.value.kind, Kind::Instance { state: None, .. }));
}

/// The rows a reader sees for an estimator: the class with the two words it
/// was named by, and then the attributes, named by their keys and sitting
/// where the file wrote them.
#[test]
fn the_template_places_an_object_s_attributes_beside_its_class() {
    let bytes = framed(&cat(&[&object("sklearn.dummy", "Thing", &one_entry("copy", b"\x88")), b"."]));
    let seen = dump(&bytes);
    tiles(&seen);
    let said: Vec<(usize, &str, &str)> = seen.iter().map(|r| (r.depth, r.name.as_str(), r.ty.as_str())).collect();
    assert_eq!(
        said,
        vec![
            (0, "file", "pickle"),
            (1, "header", "header"),
            (2, "message", "computed text"),
            (2, "form", "computed text"),
            (2, "pickler", "computed text"),
            (2, "proto", "bytes[]"),
            (2, "protocol", "u8"),
            (2, "frame", "bytes[]"),
            (1, "data", "object"),
            (2, "class", "class"),
            (3, "path", "computed text"),
            (3, "short_binunicode", "bytes[]"),
            (3, "module", "utf8[]"),
            (3, "memoize", "bytes[]"),
            (3, "short_binunicode", "bytes[]"),
            (3, "name", "utf8[]"),
            (3, "memoize", "bytes[]"),
            (3, "stack_global", "bytes[]"),
            (3, "memoize", "bytes[]"),
            (2, "empty_tuple", "bytes[]"),
            (2, "newobj", "bytes[]"),
            (2, "memoize", "bytes[]"),
            (2, "empty_dict", "bytes[]"),
            (2, "memoize", "bytes[]"),
            (2, "copy", "entry"),
            (3, "short_binunicode", "bytes[]"),
            (3, "key", "utf8[]"),
            (3, "memoize", "bytes[]"),
            (3, "value", "bool"),
            (2, "setitem", "bytes[]"),
            (2, "build", "bytes[]"),
            (1, "stop", "bytes[]"),
        ]
    );
    assert_eq!(named_row(&seen, "path").value, V::Str("sklearn.dummy.Thing".into()));
    assert_eq!(named_row(&seen, "module").value, V::Str("sklearn.dummy".into()));
    assert_eq!(named_row(&seen, "name").value, V::Str("Thing".into()));
    assert_eq!(named_row(&seen, "form").value, V::Str("sklearn-estimator-p4-p5-v1".into()));
}

/// A structured dtype, which is what a record array's values are and what
/// scikit-learn writes its tree of nodes with. The columns are named in the
/// file, twice, and the second naming is the memo slots of the first.
#[test]
fn a_record_array_reads_the_columns_its_dtype_names() {
    const COLUMNS: &[(&str, &str, &str, u64)] = &[("left", "i8", "<", 0), ("value", "f8", "<", 8)];
    let mut w = Writing::default();
    w.record_array(2, 16, COLUMNS, &[0; 32]);
    let bytes = framed(&cat(&[&w.out, b"."]));
    let found = recognise(&bytes).unwrap();
    // A record array belongs to the NumPy form like any other array.
    assert_eq!(found.form, "numpy-array-p4-p5-v6");
    let Kind::Array { dtype, dimensions, .. } = &found.value.kind else { panic!("array expected") };
    assert_eq!(dimensions, &[2]);
    assert_eq!(
        dtype,
        &Dtype::Record {
            width: 16,
            columns: vec![
                Column { name: "left".into(), dtype: "<i8".into(), at: 0 },
                Column { name: "value".into(), dtype: "<f8".into(), at: 8 },
            ],
        }
    );
    assert_eq!(dtype.name(), "V16 (left, value)");
}

/// A record's columns have to fit inside it, in the order they are written,
/// and the width in the letter code has to be the width in the state.
#[test]
fn a_record_whose_columns_do_not_fit_is_a_non_match() {
    for (width, columns, rows) in [
        // A column past the end of the record.
        (16u64, &[("left", "i8", "<", 0u64), ("value", "f8", "<", 12)][..], 2u64),
        // Two columns in the same bytes.
        (16, &[("left", "i8", "<", 0), ("value", "f8", "<", 4)][..], 2),
        // Columns out of the order they are named in.
        (16, &[("left", "i8", "<", 8), ("value", "f8", "<", 0)][..], 2),
    ] {
        let mut w = Writing::default();
        w.record_array(rows, width, columns, &vec![0; (rows * width) as usize]);
        let bytes = framed(&cat(&[&w.out, b"."]));
        assert!(recognise(&bytes).is_none(), "{width} {columns:?}");
    }
    // The width the letter code declares and the width the state declares
    // have to agree, and the numbers have to come to the shape times the
    // width.
    let mut w = Writing::default();
    w.record_array(2, 16, &[("left", "i8", "<", 0), ("value", "f8", "<", 8)], &[0; 24]);
    assert!(recognise(&framed(&cat(&[&w.out, b"."]))).is_none());
}

/// A decision tree's arrays live in a `sklearn.tree._tree.Tree`, which is the
/// one callable the scikit-learn form accepts a REDUCE of. It is called with
/// the three numbers the tree was fitted on and handed its arrays by the BUILD
/// after it.
#[test]
fn a_tree_is_a_call_of_three_numbers_and_a_build() {
    const COLUMNS: &[(&str, &str, &str, u64)] = &[("left", "i8", "<", 0), ("value", "f8", "<", 8)];
    // An estimator holding a tree, which is where a real one is.
    let mut w = Writing::default();
    w.word("sklearn.dummy");
    w.word("Thing");
    w.raw(b"\x93");
    w.mark();
    w.raw(b")\x81");
    w.mark();
    w.raw(b"}");
    w.mark();
    w.word("tree_");
    let inner = {
        let mut t = Writing::default();
        t.slots = w.slots;
        t.word("sklearn.tree._tree");
        t.word("Tree");
        t.raw(b"\x93");
        t.mark();
        t.raw(b"K\x02");
        t.record_array(1, 16, COLUMNS, &[0; 16]);
        t.raw(b"K\x01\x87");
        t.mark();
        t.raw(b"R");
        t.mark();
        t.raw(b"}");
        t.mark();
        t.word("node_count");
        t.raw(b"K\x01sb");
        t.out
    };
    w.raw(&inner);
    w.raw(b"sb");
    let bytes = framed(&cat(&[&w.out, b"."]));
    let found = recognise(&bytes).unwrap();
    assert_eq!(found.form, "sklearn-estimator-p4-p5-v1");
    let Kind::Instance { state: Some(state), .. } = &found.value.kind else { panic!("object expected") };
    let Kind::Dict(entries) = &state.kind else { panic!("state expected") };
    let Kind::Made { names, items, state: Some(tree_state), .. } = &entries[0].1.kind else { panic!("call expected") };
    assert_eq!(*names, ["n_features", "n_classes", "n_outputs"]);
    assert!(matches!(items[1].kind, Kind::Array { .. }));
    assert!(matches!(tree_state.kind, Kind::Dict(_)));
}

/// The arguments are checked against what scikit-learn writes. A `Tree` of
/// three plain numbers is not the call the form named, however familiar the
/// name in front of it is.
#[test]
fn a_tree_called_with_the_wrong_arguments_is_a_non_match() {
    let mut w = Writing::default();
    w.word("sklearn.dummy");
    w.word("Thing");
    w.raw(b"\x93");
    w.mark();
    w.raw(b")\x81");
    w.mark();
    w.raw(b"}");
    w.mark();
    w.word("tree_");
    w.word("sklearn.tree._tree");
    w.word("Tree");
    w.raw(b"\x93");
    w.mark();
    w.raw(b"K\x02K\x02K\x01\x87");
    w.mark();
    w.raw(b"R");
    w.mark();
    w.raw(b"sb");
    assert!(recognise(&framed(&cat(&[&w.out, b"."]))).is_none());
    // And a call of the right shape to a name the form does not list.
    let mut w = Writing::default();
    w.word("sklearn.dummy");
    w.word("Trees");
    w.raw(b"\x93");
    w.mark();
    w.raw(b"K\x02K\x02K\x01\x87");
    w.mark();
    w.raw(b"R");
    w.mark();
    assert!(recognise(&framed(&cat(&[&w.out, b"."]))).is_none());
}
