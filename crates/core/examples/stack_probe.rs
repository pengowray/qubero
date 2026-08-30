//! Temporary: how deep the evaluator recurses before a 1 MiB stack runs out.
//! `cargo run --example stack_probe -- <levels> [array|repeat]`
use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats;
use qubero_core::source::MemSource;
use qubero_core::template::{Template, Ty as T, Until};

fn main() {
    let n: usize = std::env::args().nth(1).unwrap().parse().unwrap();
    let shape = std::env::args().nth(2).unwrap_or_else(|| "array".into());
    let mib: usize = std::env::args().nth(3).and_then(|d| d.parse().ok()).unwrap_or(1);
    let h = std::thread::Builder::new()
        .stack_size(mib << 20)
        .spawn(move || {
            let (t, bytes) = match &*shape {
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
                    (t, b)
                }
                _ => {
                    let mut b = vec![0x81u8; n];
                    b.push(0x01);
                    (formats::builtin("cbor").unwrap(), b)
                }
            };
            let doc = Document::new(MemSource(bytes));
            let mut ev = Evaluator::new(t);
            println!("{:?}", ev.node(&doc, &[]).map(|x| x.size_bits));
        })
        .unwrap();
    let _ = h.join();
}
