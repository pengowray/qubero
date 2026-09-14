//! How deep the evaluator recurses before a stack of a given size runs out.
//! `cargo run --example stack_probe -- <levels> [shape] [KiB]`
//!
//! Two kinds of shape. `array` and `repeat` nest the file: a list of lists,
//! and a run that holds a run, which is what `DEEPEST_PATH` was measured
//! against. The rest nest the questions instead: `levels` fields or elements,
//! each working out its value by asking the one before it, read from the far
//! end with nothing remembered, which is what `DEEPEST_QUESTION` was measured
//! against. What each of those asks through:
//!
//! - `computed`: a computed field naming the field before it.
//! - `text`: the same for text, and `real` for a real.
//! - `prev`: an element's computed field reading `prev` in a list placed by
//!   arithmetic, so that the last element can be read before any other.
//! - `sibling`: the same through a backwards search by name.
//! - `tagged`: the same through a search of the list by label.
//! - `switch`: an element whose type is a switch on the element before it.
//! - `length`: an element as long as the element before it.
//! - `within`: a switch on a path into the element before, found by `sibling`.
//! - `refused`: `sibling` again, with the first element's value nested 200
//!   deep in itself, so the read is refused however it is asked, and
//!   `refused-tagged` the same through `tagged`. These are the two reads of
//!   `a_search_back_does_not_read_a_refusal_as_nothing_found` in
//!   `deep_questions`. A refusal writes out the expression it stopped in, and
//!   in a debug build writing 200 levels of it costs about a megabyte.
//!
//! Prints the answer, or the error, and how many expressions were open at
//! most. A stack that runs out takes the process with it, which is the answer
//! too: run it again with fewer levels.
//!
//! With `paint` after the size, the stack below the read is filled with a
//! known byte first and looked at afterwards, and how much of it the read
//! wrote over is printed as `stack: <bytes>`. Two of those at two numbers of
//! levels, both under the limit, give what one level costs to the byte, where
//! finding where a stack runs out gives it to a level. The size has to be
//! larger than `PAINT`.
use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats;
use qubero_core::source::MemSource;
use qubero_core::template::{Endian, Expr as E, Template, Ty as T, Until};

/// How much of the stack `paint` fills, in bytes.
const PAINT: usize = 6 << 20;

/// The byte the stack is filled with.
const MARK: u8 = 0xA5;

/// Fill `PAINT` bytes of the stack below the caller with `MARK`, and say
/// where they start and end. They are left behind when this returns, under
/// whatever the caller calls next.
#[inline(never)]
fn paint() -> (usize, usize) {
    let mut block = [0u8; PAINT];
    block.fill(MARK);
    let low = std::hint::black_box(&mut block).as_ptr() as usize;
    (low, low + PAINT)
}

/// How many bytes at the top of what `paint` filled no longer hold `MARK`:
/// the most stack anything called since has had open at once.
#[inline(never)]
fn written_over(low: usize, high: usize) -> usize {
    let mut at = low;
    // SAFETY: the bytes are the stack this thread committed for `paint`, and
    // nothing has been freed; they are only read.
    while at < high && unsafe { std::ptr::read_volatile(at as *const u8) } == MARK {
        at += 1;
    }
    high - at
}

fn main() {
    let n: usize = std::env::args().nth(1).unwrap().parse().unwrap();
    let shape = std::env::args().nth(2).unwrap_or_else(|| "array".into());
    let kib: usize = std::env::args().nth(3).and_then(|d| d.parse().ok()).unwrap_or(1024);
    let painting = std::env::args().nth(4).is_some_and(|a| a == "paint");
    let h = std::thread::Builder::new()
        .stack_size(kib << 10)
        .spawn(move || {
            let painted = painting.then(paint);
            let names: Vec<String> = (0..n).map(|i| format!("f{i}")).collect();
            let chain = |first: T, next: &dyn Fn(&str) -> T| {
                let mut fields = vec![(names[0].as_str(), first)];
                for i in 1..n {
                    fields.push((names[i].as_str(), next(&names[i - 1])));
                }
                Template::new("chain", T::structure("Chain", fields))
            };
            // A list of `n` one-byte elements whose last one is read first.
            let list = |elem: T| Template::new("chain", T::array(T::sized(E::lit(1), elem), E::lit(n as i128)));
            let (t, bytes, path) = match &*shape {
                "repeat" => {
                    let item = T::structure(
                        "Item",
                        vec![
                            ("tag", T::u8()),
                            ("kids", T::repeat(T::Named("Item".into()), Until::FieldBytes { field: "tag".into(), bytes: vec![b'e'] })),
                        ],
                    );
                    let t = Template::new("nest", T::Named("Item".into())).with_type("Item", item);
                    let mut b = vec![b'd'; n];
                    b.extend(std::iter::repeat_n(b'e', n + 1));
                    (t, b, vec![])
                }
                "computed" => (chain(T::computed(E::lit(1)), &|p| T::computed(E::field(p))), vec![0], vec![n - 1]),
                "text" => (chain(T::utf8(E::lit(1)), &|p| T::computed_text(E::field(p))), vec![b'a'], vec![n - 1]),
                "real" => (chain(T::computed_real(E::real(1.5)), &|p| T::computed_real(E::field(p))), vec![0], vec![n - 1]),
                "prev" => {
                    let elem = T::structure("Elem", vec![("b", T::u8()), ("v", T::computed(E::prev("v").or(E::lit(1))))]);
                    (list(elem), vec![0; n], vec![n - 1, 1])
                }
                "sibling" => {
                    let elem = T::structure("Elem", vec![("b", T::u8()), ("v", T::computed(E::sibling(&["v"]).or(E::lit(1))))]);
                    (list(elem), vec![0; n], vec![n - 1, 1])
                }
                "tagged" => {
                    // Each element is labelled with its own index, and asks for
                    // the value of the element labelled one less.
                    let elem = T::structure(
                        "Elem",
                        vec![
                            ("id", T::u16(Endian::Little)),
                            ("v", T::computed(E::sibling_tagged(&["id"], E::idx().sub(E::lit(1)), &["v"]).or(E::lit(1)))),
                        ],
                    );
                    let t = Template::new("chain", T::array(T::sized(E::lit(2), elem), E::lit(n as i128)));
                    let bytes = (0..n as u16).flat_map(|i| i.to_le_bytes()).collect();
                    (t, bytes, vec![n - 1, 1])
                }
                "switch" => {
                    let elem = T::structure("Elem", vec![("b", T::switch(E::prev("b"), vec![(0, T::u8())], T::u8()))]);
                    (list(elem), vec![0; n], vec![n - 1, 0])
                }
                "refused" | "refused-tagged" => {
                    // A search back whose far end is refused however it is
                    // asked: the first element's value is nested 200 deep in
                    // itself, and every element after it takes the value of
                    // the one before.
                    let first = (0..200).fold(E::lit(1), |e, _| e.add(E::lit(0)));
                    let fallback = E::cond(E::idx().equal_to(E::lit(0)), first, E::lit(1));
                    if shape == "refused" {
                        let elem = T::structure("Elem", vec![("b", T::u8()), ("v", T::computed(E::sibling(&["v"]).or(fallback)))]);
                        (list(elem), vec![0; n], vec![n - 1, 1])
                    } else {
                        let v = E::sibling_tagged(&["id"], E::idx().sub(E::lit(1)), &["v"]).or(fallback);
                        let elem = T::structure("Elem", vec![("id", T::u16(Endian::Little)), ("v", T::computed(v))]);
                        let t = Template::new("chain", T::array(T::sized(E::lit(2), elem), E::lit(n as i128)));
                        let bytes = (0..n as u16).flat_map(|i| i.to_le_bytes()).collect();
                        (t, bytes, vec![n - 1, 1])
                    }
                }
                "length" => {
                    // A run of no bytes as long as the one before it, which is
                    // a question asked while the field is being measured.
                    let elem = T::structure("Elem", vec![("x", T::bytes(E::prev("x"))), ("b", T::u8())]);
                    (list(elem), vec![0; n], vec![n - 1, 0])
                }
                "within" => {
                    // A switch keyed on a path down into the element before,
                    // reached by searching back for it by name.
                    let inner = T::structure("Inner", vec![("k", T::switch(E::sibling(&["inner", "k"]), vec![(0, T::u8())], T::u8()))]);
                    let elem = T::structure("Elem", vec![("inner", inner)]);
                    (list(elem), vec![0; n], vec![n - 1, 0, 0])
                }
                _ => {
                    let mut b = vec![0x81u8; n];
                    b.push(0x01);
                    (formats::builtin("cbor").unwrap(), b, vec![])
                }
            };
            let doc = Document::new(MemSource(bytes));
            let mut ev = Evaluator::new(t);
            let got = ev.node(&doc, &path).map(|x| (x.size_bits, x.value));
            if let Some((low, high)) = painted {
                println!("stack: {}", written_over(low, high));
            }
            println!("{got:?}");
            println!("deepest: {}", ev.deepest_question());
        })
        .unwrap();
    let _ = h.join();
}
