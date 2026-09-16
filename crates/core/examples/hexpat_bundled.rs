//! Convert every bundled `.hexpat` and say what each one became, as JSON.
//!
//! `tools/hexpat_bundle.mjs` reads this to fill in the table in
//! `crates/core/formats-hexpat/README.md`, and `tests/hexpat_bundled.rs`
//! asserts the same numbers, so the README cannot drift from the code.
//!
//! Usage: `cargo run -p qubero-core --example hexpat_bundled`.

use qubero_core::hexpat::bundled;

fn main() {
	// A deeply nested pattern recurses through the lowering, and the default
	// stack is not deep enough for it.
	std::thread::Builder::new()
		.stack_size(64 * 1024 * 1024)
		.spawn(run)
		.expect("a worker thread")
		.join()
		.expect("the worker finished");
}

fn run() {
	let mut out = String::from("[\n");
	for (index, entry) in bundled::all().iter().enumerate() {
		if index > 0 {
			out.push_str(",\n");
		}
		match bundled::template(entry.id) {
			Some(Ok(converted)) => out.push_str(&format!(
				"  {{ \"id\": {:?}, \"gaps\": {}, \"notes\": {}, \"fields\": {} }}",
				entry.id,
				converted.report.gaps.len(),
				converted.report.notes.len(),
				converted.report.fields.len()
			)),
			Some(Err(error)) => {
				out.push_str(&format!("  {{ \"id\": {:?}, \"error\": {:?} }}", entry.id, error.to_string()))
			}
			None => out.push_str(&format!("  {{ \"id\": {:?}, \"error\": \"not in the table\" }}", entry.id)),
		}
	}
	out.push_str("\n]\n");
	print!("{out}");
}
