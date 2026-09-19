//! The standard library's own classes: what each call is written with, and
//! the calls and arguments the form turns away.
//!
//! Every body here is built rather than edited from a fixture, so that a test
//! says which alternative it is exercising. A memo mark at protocol 4 carries
//! no slot number, so the counting is the file's own.

use super::*;

/// A REDUCE of a class named by STACK_GLOBAL, which is how every one of these
/// values is written at protocol 4. `arity` is how many arguments were written
/// in front of it; none at all is the empty tuple, which carries no memo mark.
fn call(module: &str, name: &str, args: &[u8], arity: u8) -> Vec<u8> {
    let mut out = cat(&[&word(module), &word(name), b"\x93\x94"]);
    out.extend_from_slice(args);
    match arity {
        0 => out.push(b')'),
        n => out.extend_from_slice(&[0x84 + n, 0x94]),
    }
    out.extend_from_slice(b"R\x94");
    out
}

/// The whole file: one of these values and nothing else.
fn only(body: &[u8]) -> Vec<u8> {
    framed(&cat(&[body, b"."]))
}

/// `datetime(2020, 1, 2, 3, 4, 5, 678901)` as `_getstate` packs it: the year
/// most significant byte first, the month, the day, the clock, and the
/// microsecond in three bytes the same way round.
const MOMENT: &[u8] = b"\x07\xe4\x01\x02\x03\x04\x05\n[\xf5";

#[test]
fn a_packed_datetime_is_read_and_a_run_that_is_not_one_is_not() {
    let made = |packed: &[u8]| only(&call("datetime", "datetime", &blob(packed), 1));
    let found = recognise(&made(MOMENT)).unwrap_or_else(|| panic!("read as far as {:#x}", furthest(&made(MOMENT))));
    assert_eq!(found.form, "stdlib-values-p4-p5-v1");
    let Kind::Made { what, names, items, .. } = &found.value.kind else { panic!("a call expected") };
    assert_eq!((*what, *names), (Shape::DateTime, &["packed"][..]));
    assert_eq!(items[0].kind, Kind::Bytes { at: 37, len: 10 });
    // A datetime is ten bytes and nothing else, and every field in them is a
    // number the calendar or the clock has.
    for wrong in [
        &b"\x07\xe4\x01\x02\x03\x04\x05\n["[..],
        b"\x07\xe4\x01\x02\x03\x04\x05\n[\xf5\x00",
        b"",
        // Month thirteen, month nought, day nought, day thirty-two.
        b"\x07\xe4\x0d\x02\x03\x04\x05\n[\xf5",
        b"\x07\xe4\x00\x02\x03\x04\x05\n[\xf5",
        b"\x07\xe4\x01\x00\x03\x04\x05\n[\xf5",
        b"\x07\xe4\x01\x20\x03\x04\x05\n[\xf5",
        // Year nought, the twenty-fourth hour, the sixtieth minute and the
        // sixtieth second.
        b"\x00\x00\x01\x02\x03\x04\x05\n[\xf5",
        b"\x07\xe4\x01\x02\x18\x04\x05\n[\xf5",
        b"\x07\xe4\x01\x02\x03\x3c\x05\n[\xf5",
        b"\x07\xe4\x01\x02\x03\x04\x3c\n[\xf5",
        // A million microseconds, which is the second after this one.
        b"\x07\xe4\x01\x02\x03\x04\x05\x0f\x42\x40",
    ] {
        assert!(recognise(&made(wrong)).is_none(), "{wrong:02x?} was read as a datetime");
    }
    // A date is four of those bytes and a time is six, and neither takes the
    // other's run.
    assert!(recognise(&only(&call("datetime", "date", &blob(b"\x07\xe4\x01\x02"), 1))).is_some());
    assert!(recognise(&only(&call("datetime", "time", &blob(b"\x03\x04\x05\n[\xf5"), 1))).is_some());
    assert!(recognise(&only(&call("datetime", "date", &blob(MOMENT), 1))).is_none());
    assert!(recognise(&only(&call("datetime", "time", &blob(b"\x07\xe4\x01\x02"), 1))).is_none());
}

/// From Python 3.6 a datetime says which side of a repeated hour it fell on,
/// and `_getstate` writes that bit in the top of the month byte rather than in
/// a byte of its own. It is written only from protocol 4, so below that the
/// high bit is a month no calendar has.
#[test]
fn the_fold_bit_is_protocol_4_s_and_is_not_the_month() {
    let folded = b"\x07\xe4\x81\x02\x03\x04\x05\n[\xf5";
    assert!(recognise(&only(&call("datetime", "datetime", &blob(folded), 1))).is_some());
    // The same value at protocol 3, where a callable is two lines and a memo
    // mark carries the slot it fills. Counting them out: the callable is 0,
    // the packed run 1, the tuple 2 and what the call made 3.
    let older = |packed: &[u8]| {
        let mut out = b"\x80\x03cdatetime\ndatetime\nq\x00C".to_vec();
        out.push(packed.len() as u8);
        out.extend_from_slice(packed);
        out.extend_from_slice(b"q\x01\x85q\x02Rq\x03.");
        out
    };
    assert_eq!(recognise(&older(MOMENT)).unwrap().form, "stdlib-values-p2-p3-v1");
    assert!(recognise(&older(folded)).is_none(), "the fold bit is not written below protocol 4");
}

/// A `Decimal` is built from the text its own `str` writes, so the text is the
/// value and a text Python would not have written is not one.
#[test]
fn a_decimal_is_the_number_python_spells() {
    let made = |said: &str| only(&call("decimal", "Decimal", &word(said), 1));
    for right in ["1.50", "-0", "0", "NaN", "sNaN", "-NaN", "NaN123", "Infinity", "-Infinity", "1E+30", "-1.5E-7", "123456789012345678901234567890"] {
        assert!(recognise(&made(right)).is_some(), "{right} was not read as a decimal");
    }
    for wrong in ["", ".", "1.2.3", "nan", "inf", "1E", "1E+", "0x10", "1_000", "one", "1 2", "NaN12a"] {
        assert!(recognise(&made(wrong)).is_none(), "{wrong:?} was read as a decimal");
    }
}

/// A `Fraction` is two whole numbers, or the one text `str` writes, which is
/// the numerator, a slash and a denominator that is never one and never
/// negative: the sign lives on the numerator.
#[test]
fn a_fraction_is_two_numbers_or_the_text_between_them() {
    let spelled = |said: &str| only(&call("fractions", "Fraction", &word(said), 1));
    let counted = |args: &[u8]| only(&call("fractions", "Fraction", args, 2));
    assert!(recognise(&counted(b"K\x01K\x03")).is_some());
    assert!(recognise(&counted(b"J\xf9\xff\xff\xffK\x02")).is_some());
    for right in ["1/3", "-7/2", "5", "-5", "0"] {
        assert!(recognise(&spelled(right)).is_some(), "{right} was not read as a fraction");
    }
    for wrong in ["", "/", "1/", "/3", "1/1", "1/-3", "01/3", "+1/3", "1.5", "1/2/3"] {
        assert!(recognise(&spelled(wrong)).is_none(), "{wrong:?} was read as a fraction");
    }
    // Two arguments have to be two numbers.
    assert!(recognise(&counted(&cat(&[&word("1"), b"K\x03"]))).is_none());
}

/// The three containers a pickler creates empty and fills with the opcodes
/// after the call, which is how every other container in a pickle is built.
#[test]
fn an_ordered_dict_a_defaulting_one_and_a_queue_are_filled_after_the_call() {
    // `OrderedDict([("b", 1), ("a", 2)])`: the call, then a batch of entries.
    let ordered = only(&cat(&[&call("collections", "OrderedDict", b"", 0), b"(", &word("b"), b"K\x01", &word("a"), b"K\x02u"]));
    let found = recognise(&ordered).unwrap_or_else(|| panic!("read as far as {:#x}", furthest(&ordered)));
    let Kind::Made { what, state: Some(state), .. } = &found.value.kind else { panic!("a call expected") };
    assert_eq!(*what, Shape::OrderedDict);
    let Kind::Dict(entries) = &state.kind else { panic!("a dictionary expected") };
    assert_eq!(entries.len(), 2);
    // One entry is SETITEM, and a call nothing filled is the empty one.
    assert!(recognise(&only(&cat(&[&call("collections", "OrderedDict", b"", 0), &word("b"), b"K\x01s"]))).is_some());
    assert!(recognise(&only(&call("collections", "OrderedDict", b"", 0))).is_some());
    // A deque is filled the way a list is, and carries how long it may grow.
    let queue = only(&cat(&[&call("collections", "deque", b")K\x05", 2), b"(K\x01K\x02e"]));
    let found = recognise(&queue).unwrap_or_else(|| panic!("read as far as {:#x}", furthest(&queue)));
    let Kind::Made { what, names, state: Some(state), .. } = &found.value.kind else { panic!("a call expected") };
    assert_eq!((*what, *names), (Shape::Deque, &["iterable", "maxlen"][..]));
    assert!(matches!(&state.kind, Kind::List(items) if items.len() == 2));
    // The empty tuple in front of the cap is where the items would have been,
    // and a deque handed anything there is a file no pickler wrote.
    assert!(recognise(&only(&cat(&[&call("collections", "deque", b"]\x94K\x05", 2), b"(K\x01K\x02e"]))).is_none());
    // A dictionary's entries do not go into a queue, nor a list's into a
    // mapping: each is filled the way its own class is.
    assert!(recognise(&only(&cat(&[&call("collections", "deque", b"", 0), &word("b"), b"K\x01s"]))).is_none());
    assert!(recognise(&only(&cat(&[&call("collections", "OrderedDict", b"", 0), b"(K\x01K\x02e"]))).is_none());
}

/// A `defaultdict`'s factory is one of a short list of builtin classes, or
/// nothing at all. It is a callable the file names and this reader never
/// calls, so what may stand there is a list rather than a rule.
#[test]
fn a_defaulting_dictionary_takes_only_the_factories_on_the_list() {
    let made = |factory: &[u8]| only(&cat(&[&call("collections", "defaultdict", factory, 1), &word("a"), b"K\x01s"]));
    for right in ["list", "dict", "set", "int", "float", "str", "tuple", "bool"] {
        let named = cat(&[&word("builtins"), &word(right), b"\x93\x94"]);
        assert!(recognise(&made(&named)).is_some(), "{right} was refused as a factory");
    }
    // `None` is what a `defaultdict` with no factory carries, and it is
    // written with no arguments at all rather than with one.
    assert!(recognise(&only(&cat(&[&call("collections", "defaultdict", b"", 0), &word("a"), b"K\x01s"]))).is_some());
    for wrong in ["eval", "exec", "open", "object", "type", "bytes"] {
        let named = cat(&[&word("builtins"), &word(wrong), b"\x93\x94"]);
        assert!(recognise(&made(&named)).is_none(), "{wrong} was read as a factory");
    }
    // A class of the writing program's own, and one from a module the form
    // does name, are both a factory the list does not have.
    assert!(recognise(&made(&cat(&[&word("__main__"), &word("Row"), b"\x93\x94"]))).is_none());
    assert!(recognise(&made(&cat(&[&word("collections"), &word("OrderedDict"), b"\x93\x94"]))).is_none());
    // And the factory is a class rather than a value.
    assert!(recognise(&made(b"K\x01")).is_none());
}

/// A `Counter` is called with the mapping it holds, and a path with however
/// many words it is made of.
#[test]
fn a_counter_holds_a_mapping_and_a_path_holds_its_parts() {
    let counted = only(&call("collections", "Counter", &cat(&[b"}\x94", &word("a"), b"K\x05s"]), 1));
    let found = recognise(&counted).unwrap_or_else(|| panic!("read as far as {:#x}", furthest(&counted)));
    let Kind::Made { what, items, state: Some(state), .. } = &found.value.kind else { panic!("a call expected") };
    assert_eq!((*what, items.len()), (Shape::Counter, 0));
    assert!(matches!(&state.kind, Kind::Dict(entries) if entries.len() == 1));
    assert!(recognise(&only(&call("collections", "Counter", b"]\x94", 1))).is_none());
    // A path of three words, of one, and of none at all, which is the empty
    // path. The whole tuple is the one argument.
    let path = |parts: &[u8], arity: u8| only(&call("pathlib", "PurePosixPath", parts, arity));
    let three = path(&cat(&[&word("/"), &word("usr"), &word("bin")]), 3);
    let found = recognise(&three).unwrap_or_else(|| panic!("read as far as {:#x}", furthest(&three)));
    let Kind::Made { what, names, items, .. } = &found.value.kind else { panic!("a call expected") };
    assert_eq!((*what, *names, items.len()), (Shape::Path, &["parts"][..], 1));
    assert!(matches!(&items[0].kind, Kind::Tuple(parts) if parts.len() == 3));
    assert!(recognise(&path(&word("/"), 1)).is_some());
    assert!(recognise(&path(b"", 0)).is_some());
    // Every part is a word the file wrote.
    assert!(recognise(&path(&cat(&[&word("/"), b"K\x01"]), 2)).is_none());
    assert!(recognise(&only(&call("pathlib", "PureWindowsPath", &cat(&[&word("C:\\"), &word("x")]), 2))).is_some());
}

/// A `uuid.UUID` is the plain object production with a 128-bit number in its
/// state, which is what the wider integer was for.
#[test]
fn an_id_is_an_object_holding_one_wide_number() {
    // `STACK_GLOBAL`, `EMPTY_TUPLE`, `NEWOBJ`, a state dictionary and `BUILD`.
    let made = |number: &[u8]| {
        let mut out = cat(&[&word("uuid"), &word("UUID"), b"\x93\x94)\x81\x94}\x94", &word("int")]);
        out.extend_from_slice(number);
        out.extend_from_slice(b"sb.");
        framed(&out)
    };
    // Sixteen bytes, which the reader's own integer type holds.
    let mut narrow = vec![0x8a, 16];
    narrow.extend_from_slice(&[0x78, 0x56, 0x34, 0x12, 0x78, 0x56, 0x34, 0x12, 0x78, 0x56, 0x34, 0x12, 0x78, 0x56, 0x34, 0x12]);
    assert_eq!(recognise(&made(&narrow)).unwrap().form, "stdlib-values-p4-p5-v1");
    // Seventeen, which is every id whose top bit is set: the last byte is the
    // nought that says the number is not negative.
    let mut wide = vec![0x8a, 17];
    wide.extend_from_slice(&[0xff; 16]);
    wide.push(0x00);
    let found = recognise(&made(&wide)).unwrap_or_else(|| panic!("read as far as {:#x}", furthest(&made(&wide))));
    let Kind::Instance { state: Some(state), .. } = &found.value.kind else { panic!("an object expected") };
    let Kind::Dict(entries) = &state.kind else { panic!("a dictionary expected") };
    let Kind::Wide { digits, .. } = &entries[0].1.kind else { panic!("a wide number expected") };
    assert_eq!(digits, "340282366920938463463374607431768211455");
    // Python 3.4 writes NEWOBJ_EX where every release after it writes NEWOBJ:
    // the same call, with an empty place for keyword arguments.
    let ex = framed(&cat(&[&word("uuid"), &word("UUID"), b"\x93\x94)}\x94\x92\x94}\x94", &word("int"), &narrow, b"sb."]));
    assert_eq!(recognise(&ex).unwrap().form, "stdlib-values-p4-p5-v1");
    // Arguments of either kind are a class being told to construct itself out
    // of values, which is the class's business and not this reader's.
    let armed = framed(&cat(&[&word("uuid"), &word("UUID"), b"\x93\x94}\x94", &word("a"), b"K\x01s\x85\x94\x81\x94."]));
    assert!(recognise(&armed).is_none());
}

/// A REDUCE is accepted only of a callable the form named. A module whitelist
/// says which classes may be *named*, never which callables may be *called*.
#[test]
fn a_call_of_anything_not_on_the_list_is_a_non_match() {
    // Both of these are classes the form may name, since their modules are on
    // its list, and neither is a call it has written down.
    for (module, name) in [("collections", "namedtuple"), ("datetime.datetime", "now"), ("datetime", "MINYEAR"), ("decimal", "getcontext")] {
        let called = only(&call(module, name, b")", 1));
        assert!(recognise(&called).is_none(), "{module}.{name} was called");
    }
    // A module the form does not list at all, called or not.
    assert!(recognise(&only(&call("os", "system", &word("ls"), 1))).is_none());
    assert!(recognise(&framed(&cat(&[&word("os"), &word("system"), b"\x93\x94."]))).is_none());
    // And one dressed as a value of a file that is otherwise familiar.
    let mixed = framed(&cat(&[
        b"}\x94(",
        &word("when"),
        &call("datetime", "date", &blob(b"\x07\xe4\x01\x02"), 1),
        &word("run"),
        &word("os"),
        &word("system"),
        b"\x93\x94u.",
    ]));
    assert!(recognise(&mixed).is_none());
}

/// Only the three containers a pickler creates empty are filled by what comes
/// after them. Everything else a call made is finished when the call is.
#[test]
fn a_call_that_is_not_one_of_the_three_is_not_filled_afterwards() {
    // A `Counter` of nothing, which is a call with an empty dictionary in it
    // and not a call waiting to be filled.
    assert!(recognise(&only(&cat(&[&call("collections", "Counter", b"}\x94", 1), &word("a"), b"K\x01s"]))).is_none());
    assert!(recognise(&only(&call("collections", "Counter", b"}\x94", 1))).is_some());
    // And a value, which nothing fills either.
    assert!(recognise(&only(&cat(&[&call("datetime", "date", &blob(b"\x07\xe4\x01\x02"), 1), &word("a"), b"K\x01s"]))).is_none());
}

/// Python hashes a date, a span, an exact number and a path, so a dictionary
/// may be keyed by one. It hashes none of the library's containers, which are
/// mutable, and neither does this.
#[test]
fn a_dictionary_may_be_keyed_by_the_values_python_hashes() {
    let keyed = |key: &[u8]| {
        let mut body = b"}\x94".to_vec();
        body.extend_from_slice(key);
        body.extend_from_slice(b"K\x01s.");
        framed(&body)
    };
    for hashes in [
        call("datetime", "date", &blob(b"\x07\xe4\x01\x02"), 1),
        call("datetime", "datetime", &blob(MOMENT), 1),
        call("datetime", "timedelta", b"K\x01K\x02K\x03", 3),
        call("decimal", "Decimal", &word("1.50"), 1),
        call("fractions", "Fraction", &word("1/3"), 1),
        call("pathlib", "PurePosixPath", &word("/"), 1),
    ] {
        assert!(recognise(&keyed(&hashes)).is_some(), "a key Python hashes was refused");
    }
    // A counter and a queue are mutable, and Python refuses them as keys the
    // way it refuses a dictionary and a list.
    assert!(recognise(&keyed(&call("collections", "Counter", b"}\x94", 1))).is_none());
    assert!(recognise(&keyed(&cat(&[&call("collections", "deque", b"", 0), b"(K\x01K\x02e"]))).is_none());
}

/// The mixed form is the union of the families' tables and not a hole in
/// them: a file holding two families and one call nobody enumerated is
/// refused, the same way each family alone refuses it.
///
/// The extra value is a class from a module a form may name a class *from*,
/// called. That is the one place a module whitelist could be mistaken for a
/// call list, and it is the distinction the safety line rests on, so the
/// widest form is the one to make it against.
#[test]
fn a_mixed_file_holding_an_unenumerated_call_is_still_refused() {
    let entries = |extra: &[u8]| {
        let mut body = cat(&[
            b"}\x94(",
            &word("a"),
            &one_array(2, "i1", b'|', b"K\x02\x85\x94", &[1, 2]),
            &word("d"),
            &call("datetime", "datetime", &blob(MOMENT), 1),
        ]);
        body.extend_from_slice(extra);
        body.extend_from_slice(b"u.");
        framed(&body)
    };
    // The array and the date on their own: two families, so the mixed form
    // reads it and says which two.
    let plain = entries(b"");
    let found = recognise(&plain).unwrap_or_else(|| panic!("read as far as {:#x}", furthest(&plain)));
    assert_eq!(found.form, "mixed-values-p4-p5-v1");
    assert_eq!(found.extensions(), "stdlib, numpy");
    // One more entry, and the only thing that changed is a REDUCE of a class
    // `collections` has and the calls table does not.
    let extra = cat(&[&word("x"), &call("collections", "ChainMap", b"", 0)]);
    assert!(recognise(&entries(&extra)).is_none());
}
