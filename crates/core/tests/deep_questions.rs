//! A read whose fields ask one another in a chain longer than the stack holds
//! is refused with an error rather than taking the process down.
//!
//! Each test reads on a thread with a stack of its own size. The one the tests
//! would otherwise get is whatever `.cargo/config.toml` sets `RUST_MIN_STACK`
//! to, which is set for rustc and is far larger than anything a reading is
//! given where it ships, so a chain that overflows wasm would pass here.
//!
//! The size is chosen so the test means something both ways: ten thousand
//! links need hundreds of megabytes of stack in a debug build and tens in a
//! release one, so without the guard every one of these crashes; with it, the
//! guard's 88 expressions fit with room to spare. A debug frame is about seven
//! times a release one, so a debug build is given more.

use qubero_core::document::Document;
use qubero_core::eval::{EvalError, Evaluator, NodeInfo, Value};
use qubero_core::source::MemSource;
use qubero_core::template::{Endian, Expr as E, Template, Ty as T};

/// Links in every chain here: far past what any stack holds unguarded.
const LINKS: usize = 10_000;

/// The stack each read gets: 8 MiB in a debug build, 1 MiB in a release one.
const STACK: usize = if cfg!(debug_assertions) { 8 << 20 } else { 1 << 20 };

/// Read `path` of `bytes` with `template`, on a thread with `STACK` of stack.
fn read_small(template: Template, bytes: Vec<u8>, path: Vec<usize>) -> Result<NodeInfo, EvalError> {
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(move || {
            let doc = Document::new(MemSource(bytes));
            let mut ev = Evaluator::new(template);
            ev.node(&doc, &path)
        })
        .unwrap()
        .join()
        .unwrap()
}

fn refused(got: Result<NodeInfo, EvalError>) -> String {
    match got {
        Err(EvalError::Failed(why)) => {
            assert!(why.contains("nested too deep"), "{why}");
            why
        }
        other => panic!("a chain of {LINKS} should be refused, not {other:?}"),
    }
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
fn a_chain_of_computed_fields_is_refused_rather_than_overflowing() {
    let t = chain(T::computed(E::lit(1)), |before| T::computed(E::field(before)));
    let why = refused(read_small(t, vec![0], vec![LINKS - 1]));
    // Named for the field that was asking and what it asked, which is the
    // deepest one reached rather than the one asked for.
    assert!(why.contains("88"), "{why}");
}

#[test]
fn a_chain_of_text_is_refused() {
    let t = chain(T::utf8(E::lit(1)), |before| T::computed_text(E::field(before)));
    refused(read_small(t, b"a".to_vec(), vec![LINKS - 1]));
}

#[test]
fn a_chain_of_reals_is_refused() {
    let t = chain(T::computed_real(E::real(1.5)), |before| T::computed_real(E::field(before)));
    refused(read_small(t, vec![0], vec![LINKS - 1]));
}

#[test]
fn a_switch_on_the_element_before_is_refused() {
    let elem = T::structure("Elem", vec![("b", T::switch(E::prev("b"), vec![(0, T::u8())], T::u8()))]);
    refused(read_small(list(elem), vec![0; LINKS], vec![LINKS - 1, 0]));
}

#[test]
fn a_length_from_the_element_before_is_refused() {
    let elem = T::structure("Elem", vec![("x", T::bytes(E::prev("x"))), ("b", T::u8())]);
    refused(read_small(list(elem), vec![0; LINKS], vec![LINKS - 1, 0]));
}

/// A search back through the list passes over an element that will not read,
/// and must not pass over one that was refused: that answer would be the
/// default after the `or`, which nothing in the file says.
#[test]
fn a_search_back_does_not_read_a_refusal_as_nothing_found() {
    let elem = T::structure("Elem", vec![("b", T::u8()), ("v", T::computed(E::sibling(&["v"]).or(E::lit(1))))]);
    refused(read_small(list(elem), vec![0; LINKS], vec![LINKS - 1, 1]));

    let elem = T::structure(
        "Elem",
        vec![
            ("id", T::u16(Endian::Little)),
            ("v", T::computed(E::sibling_tagged(&["id"], E::idx().sub(E::lit(1)), &["v"]).or(E::lit(1)))),
        ],
    );
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
