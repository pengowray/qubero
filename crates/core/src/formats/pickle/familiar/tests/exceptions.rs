//! The builtin exception classes: the shape one is rebuilt from, the three
//! module spellings, and the classes and arguments the form turns away.
//!
//! Every body here is built rather than edited from a fixture, so that a test
//! says which alternative it is exercising.

use super::*;

/// `cls(*args)` at protocol 4: the class named by STACK_GLOBAL, the arguments,
/// and REDUCE. `arity` is how many arguments were written; none at all is the
/// empty tuple, which carries no memo mark.
fn raised(module: &str, name: &str, args: &[u8], arity: u8) -> Vec<u8> {
    let mut out = cat(&[&word(module), &word(name), b"\x93\x94"]);
    out.extend_from_slice(args);
    match arity {
        0 => out.push(b')'),
        n => out.extend_from_slice(&[0x84 + n, 0x94]),
    }
    out.extend_from_slice(b"R\x94");
    out
}

/// The whole file: one exception and nothing else.
fn only(body: &[u8]) -> Vec<u8> {
    framed(&cat(&[body, b"."]))
}

/// The same below protocol 4, where text is counted four bytes wide, a memo
/// mark is BINPUT of a numbered slot, and a class is GLOBAL's two lines.
fn older(proto: u8, body: &[u8]) -> Vec<u8> {
    cat(&[&[0x80, proto], body, b"."])
}

fn global_line(module: &str, name: &str, slot: u8) -> Vec<u8> {
    cat(&[b"c", module.as_bytes(), b"\n", name.as_bytes(), b"\n", &[0x71, slot]])
}

/// BINUNICODE and its memo mark, which is how protocols 2 and 3 write text.
fn wide(text: &str, slot: u8) -> Vec<u8> {
    let mut bytes = vec![0x58];
    bytes.extend_from_slice(&(text.len() as u32).to_le_bytes());
    bytes.extend_from_slice(text.as_bytes());
    bytes.extend_from_slice(&[0x71, slot]);
    bytes
}

#[test]
fn an_exception_is_its_class_and_the_arguments_it_was_raised_with() {
    let body = raised("builtins", "ValueError", &word("bad value"), 1);
    let bytes = only(&body);
    let found = recognise(&bytes).unwrap_or_else(|| panic!("read as far as {:#x}", furthest(&bytes)));
    assert_eq!(found.form, "stdlib-values-p4-p5-v2");
    let Kind::Made { what, names, callable: Some(callable), items, state, .. } = &found.value.kind else { panic!("a call expected") };
    assert_eq!((*what, *names), (Shape::Exception, &["args"][..]));
    assert!(matches!(&callable.kind, Kind::Class { path, .. } if path == "builtins.ValueError"));
    // However many arguments there are, they are the one tuple the class was
    // called with, which is what `BaseException.__reduce__` hands over.
    assert!(matches!(&items[0].kind, Kind::Tuple(args) if args.len() == 1));
    assert!(state.is_none());
    // Raised with nothing at all, and with the three an `OSError` carries.
    assert!(recognise(&only(&raised("builtins", "ValueError", b"", 0))).is_some());
    let errno = only(&raised("builtins", "FileNotFoundError", &cat(&[b"K\x02", &word("No such file"), &word("/tmp/nope")]), 3));
    assert!(recognise(&errno).is_some());
}

/// The instance dictionary an exception carries, which `__reduce__` writes
/// after the arguments and a BUILD hands back to the object.
#[test]
fn an_exception_takes_the_attributes_a_build_gives_it() {
    let built = only(&cat(&[&raised("builtins", "RuntimeError", &word("x"), 1), b"}\x94", &word("custom"), b"K\x01sb"]));
    let found = recognise(&built).unwrap_or_else(|| panic!("read as far as {:#x}", furthest(&built)));
    let Kind::Made { what, state: Some(state), .. } = &found.value.kind else { panic!("a call expected") };
    assert_eq!(*what, Shape::Exception);
    assert!(matches!(&state.kind, Kind::Dict(entries) if entries.len() == 1));
}

/// The whole hierarchy is named, and nothing under `builtins` outside it.
#[test]
fn every_class_of_the_hierarchy_is_named_and_nothing_else_under_builtins_is() {
    for name in super::super::exceptions::HIERARCHY {
        let bytes = only(&raised("builtins", name, &word("m"), 1));
        assert!(recognise(&bytes).is_some(), "{name} was refused");
    }
    // Callables that do work when they are called, and a class of the writing
    // program's own however it is named.
    for wrong in ["eval", "exec", "open", "getattr", "type", "Error", "ValueErrorX"] {
        let bytes = only(&raised("builtins", wrong, &word("m"), 1));
        assert!(recognise(&bytes).is_none(), "{wrong} was read as an exception");
    }
    assert!(recognise(&only(&raised("__main__", "MyError", &word("m"), 1))).is_none());
    assert!(recognise(&only(&raised("numpy", "AxisError", &word("m"), 1))).is_none());
}

/// Each of the three module spellings belongs to one side of protocol 3, and a
/// file using the other side's is a file no pickler wrote.
#[test]
fn each_module_spelling_belongs_to_the_protocol_that_writes_it() {
    // `builtins` is protocol 3 and up, and never below it.
    assert!(recognise(&only(&raised("builtins", "ValueError", &word("m"), 1))).is_some());
    let old = |module: &str, name: &str| {
        older(2, &cat(&[&global_line(module, name, 0), &wide("m", 1), b"\x85q\x02Rq\x03"]))
    };
    assert!(recognise(&old("exceptions", "ValueError")).is_some());
    assert!(recognise(&old("builtins", "ValueError")).is_none());
    // The classes Python 2 never had keep the module name Python 2 knew the
    // builtins by, and the ones it did have are never written that way.
    assert!(recognise(&old("__builtin__", "RecursionError")).is_some());
    assert!(recognise(&old("__builtin__", "ValueError")).is_none());
    assert!(recognise(&old("exceptions", "RecursionError")).is_none());
    // And `exceptions` is below protocol 3 only.
    assert!(recognise(&only(&raised("exceptions", "ValueError", &word("m"), 1))).is_none());
}

/// What an exception was raised with is a message. A class, or an object of
/// one, is not, and a form that read one would be reading a class it never
/// wrote down.
#[test]
fn the_arguments_are_a_message_and_not_a_class() {
    let named = cat(&[&word("builtins"), &word("ValueError"), b"\x93\x94"]);
    assert!(recognise(&only(&raised("builtins", "ValueError", &named, 1))).is_none());
    // Numbers, words, byte strings, None and a tuple of them are a message.
    let held = cat(&[&word("bad"), b"(", &word("f.py"), b"K\x01K\x02", &word("text"), b"t\x94"]);
    assert!(recognise(&only(&raised("builtins", "SyntaxError", &held, 2))).is_some());
    assert!(recognise(&only(&raised("builtins", "ValueError", &cat(&[&blob(b"\xff"), b"N"]), 2))).is_some());
    // One exception may hold others, which is what a group is.
    let group = cat(&[&word("boom"), b"]\x94", &raised("builtins", "ValueError", &word("a"), 1), b"a"]);
    assert!(recognise(&only(&raised("builtins", "ExceptionGroup", &group, 2))).is_some());
}

/// An exception is not something Python hashes by value, so a name for one may
/// not stand where a dictionary key belongs.
#[test]
fn an_exception_is_not_a_dictionary_key() {
    // The exception under a key, which is the ordinary way round.
    let held = framed(&cat(&[b"}\x94", &word("e"), &raised("builtins", "ValueError", &word("m"), 1), b"s."]));
    assert!(recognise(&held).is_some());
    // And the same file with the exception named as the key of a second
    // entry. Counting the memo marks: 0 is the dictionary, 1 the key `e`, 4
    // the class, 5 the message, 6 the tuple, and 7 the exception itself.
    let keyed = framed(&cat(&[b"}\x94(", &word("e"), &raised("builtins", "ValueError", &word("m"), 1), &get(7), b"K\x01u."]));
    assert!(recognise(&keyed).is_none());
}

/// What the reader sees: the class and the message in the words Python's own
/// `repr` writes them in, and the rows the call was written with under it.
#[test]
fn an_exception_reads_as_python_writes_one() {
    let seen = dump(&only(&raised("builtins", "ValueError", &word("bad value"), 1)));
    let node = seen.iter().find(|r| r.ty == "exception").unwrap_or_else(|| panic!("{seen:#?}"));
    assert_eq!(node.value, V::Str("ValueError('bad value')".into()), "{seen:#?}");
    assert_eq!(named_row(&seen, "class").value, V::Str("builtins.ValueError".into()));
    assert_eq!(named_row(&seen, "args").ty, "tuple");
    tiles(&seen);
    // An OSError reads as its three arguments, the numbers among them plain.
    let errno = only(&raised("builtins", "FileNotFoundError", &cat(&[b"K\x02", &word("No such file"), &word("/tmp/nope")]), 3));
    let seen = dump(&errno);
    assert!(
        seen.iter().any(|r| r.value == V::Str("FileNotFoundError(2, 'No such file', '/tmp/nope')".into())),
        "{seen:#?}"
    );
}
