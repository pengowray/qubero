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
//!
//! Prints the answer, or the error, and how many expressions were open at
//! most. A stack that runs out takes the process with it, which is the answer
//! too: run it again with fewer levels.
use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats;
use qubero_core::source::MemSource;
use qubero_core::template::{Endian, Expr as E, Template, Ty as T, Until};

fn main() {
    let n: usize = std::env::args().nth(1).unwrap().parse().unwrap();
    let shape = std::env::args().nth(2).unwrap_or_else(|| "array".into());
    let kib: usize = std::env::args().nth(3).and_then(|d| d.parse().ok()).unwrap_or(1024);
    let h = std::thread::Builder::new()
        .stack_size(kib << 10)
        .spawn(move || {
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
            println!("{:?}", ev.node(&doc, &path).map(|x| (x.size_bits, x.value)));
            println!("deepest: {}", ev.deepest_question());
        })
        .unwrap();
    let _ = h.join();
}
