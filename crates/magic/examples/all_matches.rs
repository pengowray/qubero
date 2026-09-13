//! Every rule that matches a file, with its strength. For seeing why one
//! answer beat another.
//!
//!   cargo run --release -p qubero-magic --example all_matches -- <file>
fn main() {
    let path = std::env::args().nth(1).expect("file");
    let bytes = std::fs::read(&path).unwrap();
    let db = magic_db::global().unwrap();
    match db.all_magics_slice(&bytes[..bytes.len().min(65536)]) {
        Ok(all) => {
            for m in all {
                println!("{}\t{}\t{}", m.strength(), m.source().unwrap_or_default(), m.message());
            }
        }
        Err(e) => println!("error: {e}"),
    }
}
