//! Convert every pattern of an ImHex-Patterns checkout, say what the
//! conversion could not express, and then run the converted templates over the
//! sample files the checkout ships.
//!
//! Two numbers come out of this and both go in `docs/HANDOVER-imhex.md`. The
//! first is how much of the language the IR can say: per pattern `clean` or a
//! gap count with the first three reasons, then the totals and a histogram of
//! the reasons, which is what says which IR addition is worth making next. The
//! second is whether the templates read: for each of the pattern-and-sample
//! pairs under `tests/patterns/test_data`, the converted template is evaluated
//! over the sample and the answer is one of three, defined once here so the
//! number means the same thing next time.
//!
//! * **reads to the end** -- every node resolved and the report has no gaps.
//! * **stopped at a gap** -- every node resolved, and the report has gaps, so
//!   part of the file is described and part of it is not.
//! * **error** -- a node did not resolve, with the path that failed.
//!
//! Neither number is asserted anywhere. A list curated until it is green says
//! nothing about how far the converter gets.
//!
//! Usage: `cargo run -p qubero-core --example hexpat_gaps -- <dir>`, or set
//! `IMHEX_PATTERNS` to the checkout. `<dir>` is the repository root, the one
//! holding `patterns/`, `includes/` and `tests/`.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::hexpat::{self, Includes};
use qubero_core::source::MemSource;

/// The include files of the checkout, which stand in for the built-in table
/// wherever the checkout has the real thing.
struct Checkout {
	roots: Vec<PathBuf>,
}

impl Includes for Checkout {
	fn load(&self, path: &str) -> Option<String> {
		for root in &self.roots {
			let full = root.join(path);
			let candidates: Vec<PathBuf> = if full.extension().is_some() {
				vec![full]
			} else {
				let base = if full.is_dir() { full.join("pattern") } else { full };
				vec![base.with_extension("hexpat"), base.with_extension("pat")]
			};
			for candidate in candidates {
				if candidate.is_file() {
					if let Ok(text) = std::fs::read_to_string(&candidate) {
						return Some(text.replace("\r\n", "\n"));
					}
				}
			}
		}
		None
	}
}

fn main() {
	// A deeply nested pattern recurses through the lowering and then through
	// the evaluator, and the default stack is not deep enough for either.
	std::thread::Builder::new()
		.stack_size(256 * 1024 * 1024)
		.spawn(run)
		.expect("a worker thread")
		.join()
		.expect("the worker finished");
}

fn run() {
	let dir = std::env::args()
		.nth(1)
		.or_else(|| std::env::var("IMHEX_PATTERNS").ok())
		.map(PathBuf::from)
		.unwrap_or_else(|| {
			eprintln!("usage: hexpat_gaps <ImHex-Patterns dir>   (or set IMHEX_PATTERNS)");
			std::process::exit(2);
		});

	let includes = Checkout { roots: vec![dir.join("includes"), dir.join("patterns"), dir.clone()] };

	let mut patterns = Vec::new();
	walk(&dir.join("patterns"), "hexpat", &mut patterns);
	patterns.sort();

	let (mut clean, mut with_gaps, mut refused, mut total_gaps) = (0usize, 0usize, 0usize, 0usize);
	let mut reasons: BTreeMap<String, (usize, String)> = BTreeMap::new();
	let mut templates: HashMap<String, (qubero_core::template::Template, bool)> = HashMap::new();

	for path in &patterns {
		let rel = relative(&dir, path);
		let text = match std::fs::read_to_string(path) {
			Ok(text) => text.replace("\r\n", "\n"),
			Err(error) => {
				println!("{rel}: unreadable: {error}");
				refused += 1;
				continue;
			}
		};
		match hexpat::convert_named(&rel, &text, &includes) {
			Err(error) => {
				refused += 1;
				println!("{rel}: refused: {}:{}: {}", error.pos.line, error.pos.col, error.message);
			}
			Ok(converted) => {
				let gaps = &converted.report.gaps;
				if gaps.is_empty() {
					clean += 1;
					println!("{rel}: clean");
				} else {
					with_gaps += 1;
					total_gaps += gaps.len();
					let first: Vec<String> =
						gaps.iter().take(3).map(|g| format!("{} {}", g.path, shorten(&g.reason))).collect();
					println!("{rel}: {} gaps: {}", gaps.len(), first.join("; "));
				}
				for gap in gaps {
					let entry = reasons
						.entry(shorten(&gap.reason))
						.or_insert_with(|| (0, format!("{rel}:{} {}", gap.path, gap.source)));
					entry.0 += 1;
				}
				if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
					let complete = converted.report.is_complete();
					templates.entry(name.to_string()).or_insert((converted.template, complete));
				}
			}
		}
	}

	println!();
	println!(
		"{clean} clean / {with_gaps} with gaps of {total_gaps} total, {refused} refused, of {} patterns",
		patterns.len()
	);
	println!("the commonest gap reasons:");
	let mut top: Vec<(&String, &(usize, String))> = reasons.iter().collect();
	top.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then(a.0.cmp(b.0)));
	for (reason, (count, example)) in top.iter().take(20) {
		println!("  {count:5}  {reason}");
		println!("         e.g. {example}");
	}
	println!("  ({} kinds of gap in all)", reasons.len());

	run_samples(&dir, &templates);
}

/// Every `<pattern>.hexpat.<ext>` under `tests/patterns/test_data`, read with
/// the template its name points at.
fn run_samples(dir: &Path, templates: &HashMap<String, (qubero_core::template::Template, bool)>) {
	let data = dir.join("tests/patterns/test_data");
	let mut samples = Vec::new();
	let Ok(entries) = std::fs::read_dir(&data) else {
		println!("\nno samples under {}", data.display());
		return;
	};
	for entry in entries.flatten() {
		let path = entry.path();
		if path.is_file() {
			samples.push(path);
		}
	}
	samples.sort();

	let (mut reads, mut stopped, mut errored, mut missing) = (0usize, 0usize, 0usize, 0usize);
	let mut failures: Vec<String> = Vec::new();
	for path in &samples {
		let file = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
		// `zip.hexpat.zip` names `patterns/zip.hexpat`, and `bcss.hexpat` names
		// `patterns/bcss.hexpat` and is its own sample.
		let Some(cut) = file.find(".hexpat") else { continue };
		let pattern = format!("{}.hexpat", &file[..cut]);
		let Some((template, complete)) = templates.get(&pattern) else {
			missing += 1;
			continue;
		};
		let Ok(bytes) = std::fs::read(path) else {
			missing += 1;
			continue;
		};
		let template = template.clone();
		// A template the converter built is not a template anyone wrote, and a
		// shape the evaluator has never been handed can still take it by
		// surprise. A panic here is a finding about the evaluator, so it is
		// caught and counted rather than ending the run.
		let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
			let document = Document::new(MemSource(bytes));
			let mut ev = Evaluator::new(template);
			read(&mut ev, &document, &[], 0)
		}))
		.unwrap_or_else(|_| Err("the evaluator panicked reading it".to_string()));
		match outcome {
			Err(why) => {
				errored += 1;
				failures.push(format!("{file}: {why}"));
			}
			Ok(()) => {
				// The template said everything the pattern said, or it said
				// part of it; the report is what tells the two apart.
				if *complete {
					reads += 1;
				} else {
					stopped += 1;
				}
			}
		}
	}
	let tried = reads + stopped + errored;
	println!();
	println!(
		"samples: {reads} read to the end, {stopped} stopped at a gap, {errored} errored, \
		 {missing} without a converted pattern, of {} pairs",
		samples.len()
	);
	if tried > 0 {
		println!("pass rate: {}/{} read without an error ({:.0}%)", reads + stopped, tried, (reads + stopped) as f64 * 100.0 / tried as f64);
	}
	for line in failures.iter().take(40) {
		println!("  {line}");
	}
	if failures.len() > 40 {
		println!("  ... and {} more", failures.len() - 40);
	}
}

/// Resolve a node and enough of what is under it to prove the file reads, the
/// way `samples_real.rs` does it: a long list at both ends rather than
/// throughout.
fn read(ev: &mut Evaluator, doc: &Document<MemSource>, path: &[usize], depth: usize) -> Result<(), String> {
	if depth > 3 {
		return Ok(());
	}
	let node = match ev.node(doc, path) {
		Ok(node) => node,
		Err(error) => return Err(format!("{path:?} does not read: {error:?}")),
	};
	let count = node.child_count as usize;
	let ends: Vec<usize> = if count > 8 { (0..4).chain(count - 4..count).collect() } else { (0..count).collect() };
	for i in ends {
		let mut child = path.to_vec();
		child.push(i);
		read(ev, doc, &child, depth + 1)?;
	}
	Ok(())
}

fn relative(dir: &Path, path: &Path) -> String {
	path.strip_prefix(dir).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

/// A reason cut down to the part that groups: the first clause, with any names
/// taken out of it.
fn shorten(reason: &str) -> String {
	let cut = reason.find(", which").unwrap_or(reason.len());
	let head = &reason[..cut];
	if head.len() > 90 {
		format!("{}...", &head[..87])
	} else {
		head.to_string()
	}
}

fn walk(dir: &Path, extension: &str, out: &mut Vec<PathBuf>) {
	let Ok(entries) = std::fs::read_dir(dir) else { return };
	for entry in entries.flatten() {
		let path = entry.path();
		if path.is_dir() {
			walk(&path, extension, out);
		} else if path.extension().and_then(|e| e.to_str()) == Some(extension) {
			out.push(path);
		}
	}
}
