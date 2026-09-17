//! A read whose fields ask one another in a chain longer than the stack holds
//! is read from the far end all the same, a stretch of the chain at a time,
//! and what cannot be read that way is refused with an error rather than
//! taking the process down.
//!
//! Each test reads on a thread with a stack of its own size. The one the tests
//! would otherwise get is whatever `.cargo/config.toml` sets `RUST_MIN_STACK`
//! to, which is set for rustc and is far larger than anything a reading is
//! given where it ships, so a chain that overflows wasm would pass here.
//!
//! The size is chosen so the test means something both ways: ten thousand
//! links need hundreds of megabytes of stack in a debug build and tens in a
//! release one, so without the guard every one of these crashes; with it, the
//! guard's 110 expressions fit, with too little room over for a read whose
//! stack per expression grows by a fifteenth in a release build or an eighth
//! in a debug one. See `STACK`.

use qubero_core::document::Document;
use qubero_core::eval::{EvalError, Evaluator, NodeInfo, Value};
use qubero_core::source::MemSource;
use qubero_core::eval::ValidVerdict;
use qubero_core::template::{Endian, Expr as E, Template, Ty as T, Valid};

/// Links in every chain here: far past what any stack holds unguarded.
const LINKS: usize = 10_000;

/// The stack each read gets: 640 KiB in a release build, the room a read is
/// given where it ships (`STACK_BUDGET` in the evaluator), and 4 MiB in a
/// debug one, whose frames are six to ten times as large.
///
/// Measured on 2026-09-15 by filling the stack with a known byte and counting
/// how much of it a read wrote over (`stack_probe <links> <shape> <KiB>
/// paint`). Read to the limit, the dearest shape here took 571 KiB in a
/// release build, a switch on the element before, and 3.5 MiB in a debug
/// one, a computed field naming the one before. A thread of 640 KiB gives a
/// read between 608 and 613 KiB, and one of 4 MiB between 3.93 and 3.98 MiB,
/// about a fifteenth and an eighth more than that. The same reads overflow a
/// release thread of 576 KiB and a debug one of 3.5 MiB.
const STACK: usize = if cfg!(debug_assertions) { 4 << 20 } else { 640 << 10 };

/// The most expressions a read may have open inside one another, which is
/// `DEEPEST_QUESTION` in the evaluator.
const LIMIT: usize = 110;

/// Read `path` of `bytes` with `template`, on a thread with `STACK` of stack,
/// from a cold start: nothing else has been asked first. Also how many
/// expressions were open inside one another at most.
fn read_small(template: Template, bytes: Vec<u8>, path: Vec<usize>) -> (Result<NodeInfo, EvalError>, usize) {
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(move || {
            let doc = Document::new(MemSource(bytes));
            let mut ev = Evaluator::new(template);
            let got = ev.node(&doc, &path);
            (got, ev.deepest_question())
        })
        .unwrap()
        .join()
        .unwrap()
}

/// The same, for a verdict about a field's value rather than for the field:
/// a constraint's expression is worked out on the same stack and counted the
/// same way, and this is what says so.
fn judge_small(
    template: Template,
    bytes: Vec<u8>,
    path: Vec<usize>,
) -> (Result<Option<ValidVerdict>, EvalError>, usize) {
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(move || {
            let doc = Document::new(MemSource(bytes));
            let mut ev = Evaluator::new(template);
            let got = ev.valid_of(&doc, &path);
            (got, ev.deepest_question())
        })
        .unwrap()
        .join()
        .unwrap()
}

fn refused((got, _): (Result<NodeInfo, EvalError>, usize)) -> String {
    match got {
        Err(EvalError::Failed(why)) => {
            assert!(why.contains("nested too deep"), "{why}");
            why
        }
        other => panic!("this read should be refused, not {other:?}"),
    }
}

/// The value read at the far end of a chain, which no more than the limit of
/// expressions were open to reach.
fn read_through((got, deepest): (Result<NodeInfo, EvalError>, usize)) -> Value {
    assert!(deepest <= LIMIT, "{deepest} expressions open at once");
    match got {
        Ok(info) => info.value,
        Err(e) => panic!("a chain of {LINKS} should read from the far end, not {e:?}"),
    }
}

/// `inner` inside `levels` sums with nought: an expression nested that deep
/// in itself, with no field between one level and the next.
fn nested(inner: E, levels: usize) -> E {
    (0..levels).fold(inner, |e, _| e.add(E::lit(0)))
}

/// A structure of `LINKS` fields, the first `first` and each after it `next`
/// of the name of the one before.
fn chain(first: T, next: impl Fn(&str) -> T) -> Template {
    let names: Vec<String> = (0..LINKS).map(|i| format!("f{i}")).collect();
    let mut fields = vec![(names[0].as_str(), first)];
    for i in 1..LINKS {
        fields.push((names[i].as_str(), next(&names[i - 1])));
    }
    Template::new("chain", T::structure("Chain", fields))
}

/// A list of `LINKS` one-byte elements, placed by arithmetic so that the last
/// can be read before any other.
fn list(elem: T) -> Template {
    Template::new("chain", T::array(T::sized(E::lit(1), elem), E::lit(LINKS as i128)))
}

#[test]
fn a_chain_of_computed_fields_reads_from_the_far_end() {
    let t = chain(T::computed(E::lit(1)), |before| T::computed(E::field(before)));
    assert_eq!(read_through(read_small(t, vec![0], vec![LINKS - 1])), Value::Int(1));
}

#[test]
fn a_chain_of_text_reads_from_the_far_end() {
    let t = chain(T::utf8(E::lit(1)), |before| T::computed_text(E::field(before)));
    assert_eq!(read_through(read_small(t, b"a".to_vec(), vec![LINKS - 1])), Value::Str("a".into()));
}

#[test]
fn a_chain_of_reals_reads_from_the_far_end() {
    let t = chain(T::computed_real(E::real(1.5)), |before| T::computed_real(E::field(before)));
    assert_eq!(read_through(read_small(t, vec![0], vec![LINKS - 1])), Value::Float(1.5));
}

#[test]
fn a_switch_on_the_element_before_reads_from_the_far_end() {
    let elem = T::structure("Elem", vec![("b", T::switch(E::prev("b"), vec![(0, T::u8())], T::u8()))]);
    let mut bytes = vec![0; LINKS];
    bytes[LINKS - 1] = 7;
    assert_eq!(read_through(read_small(list(elem), bytes, vec![LINKS - 1, 0])), Value::UInt(7));
}

/// A length taken from the element before is asked while that element is
/// being measured, which the stack budget rather than the count stops first.
#[test]
fn a_length_from_the_element_before_reads_from_the_far_end() {
    let elem = T::structure("Elem", vec![("x", T::bytes(E::prev("x"))), ("b", T::u8())]);
    let mut bytes = vec![0; LINKS];
    bytes[LINKS - 1] = 7;
    assert_eq!(read_through(read_small(list(elem), bytes, vec![LINKS - 1, 1])), Value::UInt(7));
}

/// A search back by name and a search back by label, each finding the element
/// before, whose answer is a search of its own.
#[test]
fn a_search_back_reads_from_the_far_end() {
    let elem = T::structure("Elem", vec![("b", T::u8()), ("v", T::computed(E::sibling(&["v"]).add(E::lit(1))))]);
    assert_eq!(read_through(read_small(list(elem), vec![0; LINKS], vec![LINKS - 1, 1])), Value::Int(LINKS as i128));

    let elem = T::structure(
        "Elem",
        vec![
            ("id", T::u16(Endian::Little)),
            ("v", T::computed(E::sibling_tagged(&["id"], E::idx().sub(E::lit(1)), &["v"]).add(E::lit(1)))),
        ],
    );
    let t = Template::new("chain", T::array(T::sized(E::lit(2), elem), E::lit(LINKS as i128)));
    let bytes = (0..LINKS as u16).flat_map(|i| i.to_le_bytes()).collect();
    assert_eq!(read_through(read_small(t, bytes, vec![LINKS - 1, 1])), Value::Int(LINKS as i128));
}

/// An expression nested past the limit inside itself has no field between one
/// level and the next, so nothing is remembered on the way down however it is
/// asked, and it is refused, named for where it stopped.
#[test]
fn an_expression_nested_past_the_limit_in_itself_is_refused() {
    let t = Template::new("deep", T::structure("Deep", vec![("v", T::computed(nested(E::lit(1), 200)))]));
    let why = refused(read_small(t, vec![0], vec![0]));
    assert!(why.contains(&format!("more than {LIMIT} expressions")), "{why}");
}

/// A refusal says what expression it stopped at, and that is written once the
/// read has come back up, not where it stopped. Writing an expression out
/// costs a frame for every level of it. Here four hundred levels are left
/// when the read is refused: written where it stopped, on top of every
/// expression open, they overflow a debug build's thread, and written at the
/// top they fit. A release build fits both ways, so only a debug build tests
/// this.
#[test]
fn a_refusal_writes_its_expression_after_the_read_comes_back_up() {
    let t = Template::new("deep", T::structure("Deep", vec![("v", T::computed(nested(E::lit(1), LIMIT + 400)))]));
    let why = refused(read_small(t, vec![0], vec![0]));
    // All of what was left, not a part of it.
    assert!(why.contains("; stopped at v while evaluating 1 + 0 + 0"), "{why}");
    assert_eq!(why.matches(" + 0").count(), 400, "{why}");
}

/// A search back through the list passes over an element that will not read,
/// and must not pass over one that was refused: that answer would be the
/// default after the `or`, which nothing in the file says. The first element
/// here is refused however it is asked, and every element after it takes the
/// value of the one before.
#[test]
fn a_search_back_does_not_read_a_refusal_as_nothing_found() {
    let first = nested(E::lit(1), 200);
    let v = E::sibling(&["v"]).or(E::cond(E::idx().equal_to(E::lit(0)), first.clone(), E::lit(1)));
    let elem = T::structure("Elem", vec![("b", T::u8()), ("v", T::computed(v))]);
    refused(read_small(list(elem), vec![0; LINKS], vec![LINKS - 1, 1]));

    let v = E::sibling_tagged(&["id"], E::idx().sub(E::lit(1)), &["v"]).or(E::cond(E::idx().equal_to(E::lit(0)), first, E::lit(1)));
    let elem = T::structure("Elem", vec![("id", T::u16(Endian::Little)), ("v", T::computed(v))]);
    let t = Template::new("chain", T::array(T::sized(E::lit(2), elem), E::lit(LINKS as i128)));
    let bytes = (0..LINKS as u16).flat_map(|i| i.to_le_bytes()).collect();
    refused(read_small(t, bytes, vec![LINKS - 1, 1]));
}

/// The same chains read in order are one link deep, since what came before is
/// already known, and read to the end with the same small stack.
#[test]
fn the_same_chains_read_in_order_reach_the_end() {
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(|| {
            // Every tenth field, so that each read works out the nine before
            // it that nothing has asked for yet: twenty expressions deep, and
            // no deeper at the end of the chain than at the start.
            let t = chain(T::computed(E::lit(1)), |before| T::computed(E::field(before).add(E::lit(1))));
            let doc = Document::new(MemSource(vec![0]));
            let mut ev = Evaluator::new(t);
            for i in (0..LINKS).step_by(10) {
                assert_eq!(ev.node(&doc, &[i]).unwrap().value, Value::Int(i as i128 + 1));
            }

            let elem = T::structure("Elem", vec![("b", T::u8()), ("v", T::computed(E::prev("v").add(E::lit(1))))]);
            let doc = Document::new(MemSource(vec![0; LINKS]));
            let mut ev = Evaluator::new(list(elem));
            for i in 0..LINKS {
                assert_eq!(ev.node(&doc, &[i, 1]).unwrap().value, Value::Int(i as i128 + 1));
            }

            // Text taken from the element before, found by a search back. Its
            // answer is kept like a number's, so each read is one link deep;
            // worked out afresh every time, the chain would be as deep as the
            // element's index and refused before the hundredth.
            let elem = T::structure("Elem", vec![("b", T::u8()), ("name", T::computed_text(E::sibling(&["name"])))]);
            let doc = Document::new(MemSource(vec![0; LINKS]));
            let mut ev = Evaluator::new(list(elem));
            for i in 0..LINKS {
                assert_eq!(ev.node(&doc, &[i, 1]).unwrap().value, Value::Str(String::new()));
            }
            assert!(ev.deepest_question() < 8, "{} deep", ev.deepest_question());

            // A chain just under the limit reads from a cold start: forty
            // computed fields, each one expression deep.
            let names: Vec<String> = (0..40).map(|i| format!("g{i}")).collect();
            let mut fields = vec![(names[0].as_str(), T::computed(E::lit(1)))];
            for i in 1..40 {
                fields.push((names[i].as_str(), T::computed(E::field(&names[i - 1]))));
            }
            let t = Template::new("short", T::structure("Short", fields));
            let doc = Document::new(MemSource(vec![0]));
            let mut ev = Evaluator::new(t);
            assert_eq!(ev.node(&doc, &[39]).unwrap().value, Value::Int(1));
        })
        .unwrap()
        .join()
        .unwrap();
}

/// A constraint's expression is worked out on the same stack a read is, and
/// `this` reads a field, so a bound nested deep in itself has to be counted
/// and refused like any other expression rather than taking the thread down.
/// The read itself is fine; it is the verdict that goes deep.
#[test]
fn a_constraint_nested_past_the_limit_is_refused_rather_than_overflowing() {
    let deep = |levels| {
        let ty = T::structure("Deep", vec![("v", T::u8())])
            .field_valid("v", Valid::Expr { expr: nested(E::This, levels), msg: None });
        Template::new("deep", ty)
    };
    // Well inside the limit: the bound is worked out and holds.
    let (got, deepest) = judge_small(deep(40), vec![7], vec![0]);
    assert!(deepest <= LIMIT, "{deepest} expressions open at once");
    assert_eq!(got.unwrap().map(|v| v.ok), Some(true));

    // Past it: a refusal naming where it stopped, and a thread still standing.
    let (got, _) = judge_small(deep(LIMIT + 400), vec![7], vec![0]);
    match got {
        Err(EvalError::Failed(why)) => assert!(why.contains("nested too deep"), "{why}"),
        other => panic!("this verdict should be refused, not {other:?}"),
    }
}
