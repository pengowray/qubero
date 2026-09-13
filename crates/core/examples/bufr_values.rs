//! Every value of every message in a BUFR file, as the side reader reads them:
//! `bufr_values <file>`.
//!
//! One line a value, tab-separated: the message, the subset, the descriptor,
//! what kind of value it is, and the value, or `MISSING`. Operators are left
//! out, since they hold no data. A compressed message's values are listed
//! subset by subset. That is the shape of the dump the cross-check against
//! pybufrkit and ecCodes writes, so the two can be compared with `diff`. The
//! steps and any problem go to standard error.

use qubero_core::formats::bufr_data::{self, Role};

fn main() {
    let path = std::env::args().nth(1).expect("usage: bufr_values <file>");
    let bytes = std::fs::read(&path).unwrap();
    let mut at = 0;
    let mut message = 0;
    while let Some(k) = bytes[at..].windows(4).position(|w| w == b"BUFR") {
        let start = at + k;
        let len = bytes.get(start + 4..start + 7).map_or(0, |b| (usize::from(b[0]) << 16) | (usize::from(b[1]) << 8) | usize::from(b[2]));
        let end = (start + len.max(8)).min(bytes.len());
        let r = bufr_data::read(&bytes[start..end]);
        for s in &r.steps {
            eprintln!("message {message}: {}", s.what);
        }
        if let Some(p) = &r.problem {
            eprintln!("message {message}: problem: {p}");
        }
        let per_list = if r.header.compressed { r.header.subsets as usize } else { 1 };
        for (list, items) in r.subsets.iter().enumerate() {
            for subset in 0..per_list {
                for item in items.iter().filter(|i| i.role != Role::Operator) {
                    let value = if item.missing(subset) { "MISSING".to_string() } else { item.text(subset) };
                    let s = if r.header.compressed { subset } else { list };
                    // Quality information a bitmap attached to an element is
                    // an element of its own in the data, and an attribute of
                    // the element it is about in ecCodes' keys.
                    let kind = if item.role == Role::Element && item.refers_to.is_some() { "quality" } else { item.role.as_str() };
                    let about = item.refers_to.and_then(|k| items.get(k)).map_or(String::new(), |t| format!("\t{:06}", t.code));
                    println!("{message}\t{s}\t{:06}\t{kind}\t{value}{about}", item.code);
                }
            }
        }
        message += 1;
        at = end;
    }
}
