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
        let t = Template::new(
            "t",
            T::structure("Root", vec![("n", T::u64(Little)), ("items", T::array(string, E::field("n")))]),
        );
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(t);
        let start = Instant::now();
        let items = ev.node(&d, &[1]).expect("items");
        println!(
            "{n:>7} strings: {:?}  ({} bytes, {} memo entries)",
            start.elapsed(),
            items.size_bits / 8,
            ev.memo_len()
        );
    }
}
