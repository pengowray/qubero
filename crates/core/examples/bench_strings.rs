//! How the walk over a long list of variable-length elements scales:
//! `cargo run --release --example bench_strings`

use std::time::Instant;

use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::source::MemSource;
use qubero_core::template::{Endian::Little, Expr as E, Template, Ty as T};

fn main() {
    for n in [25_000u64, 50_000, 100_000, 200_000] {
        let mut bytes = n.to_le_bytes().to_vec();
        for i in 0..n {
            let s = format!("token{i}");
            bytes.extend_from_slice(&(s.len() as u64).to_le_bytes());
            bytes.extend_from_slice(s.as_bytes());
        }
        let string = T::structure("String", vec![("len", T::u64(Little)), ("text", T::utf8(E::field("len")))]);
        // The same list twice: once as the element type itself, once reached
        // through a named type and a thirteen-case switch, which is how GGUF
        // says "a value of whichever kind the entry declared".
        let plain = T::array(string.clone(), E::field("n"));
        let cases: Vec<(i128, T)> = (0..13).map(|k| (k, if k == 8 { T::Named("String".into()) } else { T::u8() })).collect();
        let switched = T::array(T::switch(E::lit(8), cases, T::bytes(E::lit(0))), E::field("n"));
        for (what, elem) in [("plain", plain), ("switched", switched)] {
        let t = Template::new("t", T::structure("Root", vec![("n", T::u64(Little)), ("items", elem)]))
            .with_type("String", string.clone());
        let d = Document::new(MemSource(bytes.clone()));
        let mut ev = Evaluator::new(t);
        let start = Instant::now();
        let items = ev.node(&d, &[1]).expect("items");
        println!(
            "{n:>7} strings {what:<9}: {:?}  ({} bytes, {} memo entries)",
            start.elapsed(),
            items.size_bits / 8,
            ev.memo_len()
        );
        }
    }
}
