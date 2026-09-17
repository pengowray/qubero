//! Which Familiar Pickle Form each file matches, and its opcodes in order.
//!
//! `cargo run -p qubero-core --example pickle_forms -- <file>...` prints the
//! form a file matched, or the listing the recogniser had to work from when it
//! matched nothing. `--ops` prints the instruction sequence alone.

use qubero_core::formats::pickle;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let ops_only = args.iter().any(|a| a == "--ops");
    for path in args.iter().filter(|a| !a.starts_with("--")) {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) => {
                println!("{path}: {e}");
                continue;
            }
        };
        match pickle::familiar::recognise(&bytes) {
            Some(found) => println!("{path}: {}", found.form),
            None => println!("{path}: no form"),
        }
        if ops_only {
            for op in pickle::opcodes(&bytes) {
                println!("  {:>6} {}", op.at, pickle::opcode_name(op.code));
            }
        }
    }
}
