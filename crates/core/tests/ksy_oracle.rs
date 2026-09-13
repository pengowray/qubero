//! The `.ksy` converter against the Kaitai corpus and its own test oracle.
//!
//! Point `KAITAI_STRUCT` at a recursive checkout of
//! <https://github.com/kaitai-io/kaitai_struct> and run:
//!
//! ```text
//! KAITAI_STRUCT=D:/github/kaitai_struct cargo test -p qubero-core --test ksy_oracle -- --nocapture
//! ```
//!
//! Two things happen. Every `.ksy` under `tests/formats/` is converted and then
//! *run*: `tests/spec/ks/*.kst` names an input file and a list of asserts, and
//! each assert whose left side is a plain field path and whose right side is a
//! literal is checked against what the evaluator reads. Everything else is
//! counted as skipped rather than quietly passed. And every format under
//! `formats/` is converted with the whole collection available as imports, to
//! count what the IR could and could not say about the formats people actually
//! wrote.
//!
//! Neither test asserts a pass rate. The numbers are the deliverable: a list
//! curated until it is green says nothing about how far the converter gets.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::ksy::imports::MapImports;
use qubero_core::ksy::yaml::{YamlNode, YamlValue};
use qubero_core::ksy::{self, NoImports};
use qubero_core::source::MemSource;

#[test]
fn the_test_formats_read_the_files_they_come_with() {
	let Some(root) = corpus() else { return };
	let formats = collect(&root.join("tests/formats"));
	let imports = imports_over(&[root.join("tests/formats"), root.join("formats")]);

	let mut converted: BTreeMap<String, qubero_core::template::Template> = BTreeMap::new();
	let (mut clean, mut with_gaps, mut errored) = (0usize, 0usize, 0usize);
	for path in &formats {
		let text = fs::read_to_string(path).expect("a readable .ksy");
		match ksy::convert(&text, &imports) {
			Err(e) => {
				errored += 1;
				println!("  convert failed: {}: {e}", name(path));
			}
			Ok(out) => {
				if out.report.is_complete() {
					clean += 1;
				} else {
					with_gaps += 1;
				}
				converted.insert(out.template.name.clone(), out.template);
			}
		}
	}
	println!("ksy: {clean} converted clean, {with_gaps} with gaps, {errored} refused, of {}", formats.len());

	let specs = collect_ext(&root.join("tests/spec/ks"), "kst");
	let (mut passed, mut failed, mut skipped, mut no_template, mut no_data) = (0, 0, 0, 0, 0);
	let mut failures: Vec<String> = Vec::new();
	for path in &specs {
		let text = fs::read_to_string(path).expect("a readable .kst");
		let Ok(doc) = ksy::yaml::load(&text) else { continue };
		let Some(id) = doc.get("id").and_then(|n| n.as_str().ok()) else { continue };
		let Some(template) = converted.get(&id) else {
			no_template += 1;
			continue;
		};
		let data = doc.get("data").and_then(|n| n.as_str().ok()).unwrap_or_default();
		let file = root.join("tests/src").join(&data);
		let Ok(bytes) = fs::read(&file) else {
			no_data += 1;
			continue;
		};
		let document = Document::new(MemSource(bytes));
		let mut ev = Evaluator::new(template.clone());
		let Some(asserts) = doc.get("asserts") else { continue };
		let Ok(items) = asserts.as_seq() else { continue };
		for item in items {
			let (Some(actual), Some(expected)) = (item.get("actual"), item.get("expected")) else {
				skipped += 1;
				continue;
			};
			let (Ok(actual), Ok(expected)) = (actual.as_str(), expected.as_str()) else {
				skipped += 1;
				continue;
			};
			let Some(steps) = field_path(&actual) else {
				skipped += 1;
				continue;
			};
			let Some(want) = literal(&expected) else {
				skipped += 1;
				continue;
			};
			match read_path(&mut ev, &document, &steps) {
				None => {
					failed += 1;
					failures.push(format!("{id}: {actual}: no such field, expected {expected}"));
				}
				Some(value) => match matches(&value, &want) {
					true => passed += 1,
					false => {
						failed += 1;
						failures.push(format!(
							"{id}: {actual}: expected {expected}, got {}",
							shown_value(&value)
						));
					}
				},
			}
		}
	}
	println!(
		"asserts: {passed} passed, {failed} failed, {skipped} skipped \
		 ({no_template} specs name a format that did not convert, {no_data} have no input file)"
	);
	if !failures.is_empty() {
		println!("failures ({}):", failures.len());
		for line in &failures {
			println!("  {line}");
		}
	}
}

#[test]
fn the_formats_collection_converts() {
	let Some(root) = corpus() else { return };
	let formats = collect(&root.join("formats"));
	let imports = imports_over(&[root.join("formats")]);

	let (mut clean, mut with_gaps, mut errored) = (0usize, 0usize, 0usize);
	let mut reasons: BTreeMap<String, (usize, String)> = BTreeMap::new();
	for path in &formats {
		let text = fs::read_to_string(path).expect("a readable .ksy");
		match ksy::convert(&text, &imports) {
			Err(e) => {
				errored += 1;
				println!("  convert failed: {}: {e}", name(path));
			}
			Ok(out) => {
				if out.report.is_complete() {
					clean += 1;
				} else {
					with_gaps += 1;
				}
				for gap in &out.report.gaps {
					let entry = reasons.entry(shorten(&gap.reason)).or_insert_with(|| {
						(0, format!("{}{} ({})", name(path), gap.path, gap.reason))
					});
					entry.0 += 1;
				}
			}
		}
	}
	println!(
		"formats: {clean} converted clean, {with_gaps} with gaps, {errored} refused, of {}",
		formats.len()
	);
	let mut top: Vec<(&String, &(usize, String))> = reasons.iter().collect();
	top.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(b.0)));
	println!("the twenty commonest gaps, with one example of each:");
	for (reason, (count, example)) in top.iter().take(20) {
		println!("  {count:5}  {reason}");
		println!("         e.g. {example}");
	}
	println!("  ({} kinds of gap in all)", reasons.len());
}

#[test]
fn a_format_with_no_imports_needs_no_resolver() {
	let Some(root) = corpus() else { return };
	let text = fs::read_to_string(root.join("formats/image/png.ksy")).expect("png.ksy");
	let out = ksy::convert(&text, &NoImports).expect("png converts");
	assert_eq!(out.template.name, "png");
	assert!(out.template.types.contains_key("chunk"), "{:?}", out.template.types.keys());
}

// ---------------------------------------------------------------------------

fn corpus() -> Option<PathBuf> {
	match std::env::var("KAITAI_STRUCT") {
		Ok(path) if !path.is_empty() => Some(PathBuf::from(path)),
		_ => {
			println!("skipped: set KAITAI_STRUCT to a kaitai_struct checkout");
			None
		}
	}
}

/// Every `.ksy` under these directories, keyed by the name a `meta/imports`
/// would write: its path from the directory, and its bare name.
fn imports_over(dirs: &[PathBuf]) -> MapImports {
	let mut map = MapImports::new();
	for dir in dirs {
		for path in collect(dir) {
			let Ok(text) = fs::read_to_string(&path) else { continue };
			let relative = path
				.strip_prefix(dir)
				.unwrap_or(&path)
				.with_extension("")
				.to_string_lossy()
				.replace('\\', "/");
			map.0.entry(relative).or_insert_with(|| text.clone());
			map.0.entry(name(&path)).or_insert(text);
		}
	}
	map
}

fn collect(dir: &Path) -> Vec<PathBuf> {
	collect_ext(dir, "ksy")
}

fn collect_ext(dir: &Path, ext: &str) -> Vec<PathBuf> {
	let mut out = Vec::new();
	walk(dir, ext, &mut out);
	out.sort();
	out
}

fn walk(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
	let Ok(entries) = fs::read_dir(dir) else { return };
	for entry in entries.flatten() {
		let path = entry.path();
		if path.is_dir() {
			if path.file_name().is_some_and(|n| n == "_build") {
				continue;
			}
			walk(&path, ext, out);
		} else if path.extension().is_some_and(|e| e == ext) {
			out.push(path);
		}
	}
}

fn name(path: &Path) -> String {
	path.file_stem().unwrap_or_default().to_string_lossy().to_string()
}

/// A gap reason with the quoted specifics taken out, so that the same kind of
/// gap counts as one kind however many fields hit it.
fn shorten(reason: &str) -> String {
	let mut out = String::new();
	let mut in_quotes = false;
	for c in reason.chars() {
		if c == '`' {
			if !in_quotes {
				out.push('`');
				out.push('…');
				out.push('`');
			}
			in_quotes = !in_quotes;
			continue;
		}
		if !in_quotes {
			out.push(c);
		}
	}
	out
}

// -- the asserts ------------------------------------------------------------

/// One step of a `.kst` field path.
enum Step {
	Name(String),
	Index(usize),
}

/// A `.kst` `actual` that is a plain path into the file, e.g. `hdr.entries[2].len`.
/// Anything with a method call, an operator or a special name is not one.
fn field_path(text: &str) -> Option<Vec<Step>> {
	let text = text.trim();
	if text.is_empty() || text.contains(|c: char| "+-*/%<>=!?:&|\"'".contains(c)) {
		return None;
	}
	let mut steps = Vec::new();
	for part in text.split('.') {
		let (name, rest) = match part.find('[') {
			None => (part, ""),
			Some(at) => (&part[..at], &part[at..]),
		};
		if name.is_empty() || name.starts_with('_') || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
			return None;
		}
		steps.push(Step::Name(name.to_string()));
		let mut rest = rest;
		while !rest.is_empty() {
			let close = rest.find(']')?;
			let index: usize = rest[1..close].parse().ok()?;
			steps.push(Step::Index(index));
			rest = &rest[close + 1..];
		}
	}
	Some(steps)
}

/// What a `.kst` expects, where that is a literal this can compare against.
enum Want {
	Int(i128),
	Bool(bool),
	Str(String),
	Bytes(Vec<u8>),
	Enum(String),
	Float(f64),
}

fn literal(text: &str) -> Option<Want> {
	let text = text.trim();
	if text == "true" {
		return Some(Want::Bool(true));
	}
	if text == "false" {
		return Some(Want::Bool(false));
	}
	if let Some(rest) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
		return i128::from_str_radix(&rest.replace('_', ""), 16).ok().map(Want::Int);
	}
	if let Some(rest) = text.strip_prefix("0b").or_else(|| text.strip_prefix("0B")) {
		return i128::from_str_radix(&rest.replace('_', ""), 2).ok().map(Want::Int);
	}
	if let Some(rest) = text.strip_prefix("0o").or_else(|| text.strip_prefix("0O")) {
		return i128::from_str_radix(&rest.replace('_', ""), 8).ok().map(Want::Int);
	}
	if let Ok(n) = text.replace('_', "").parse::<i128>() {
		return Some(Want::Int(n));
	}
	if text.starts_with('"') && text.ends_with('"') && text.len() >= 2 {
		return Some(Want::Str(unescape(&text[1..text.len() - 1])));
	}
	if text.starts_with('\'') && text.ends_with('\'') && text.len() >= 2 {
		return Some(Want::Str(text[1..text.len() - 1].to_string()));
	}
	if text.starts_with('[') && text.ends_with(']') {
		let mut bytes = Vec::new();
		for part in text[1..text.len() - 1].split(',') {
			let part = part.trim();
			if part.is_empty() {
				continue;
			}
			let v = match part.strip_prefix("0x") {
				Some(hex) => i64::from_str_radix(hex, 16).ok()?,
				None => part.parse::<i64>().ok()?,
			};
			bytes.push(u8::try_from(v & 0xff).ok()?);
		}
		return Some(Want::Bytes(bytes));
	}
	if let Some((_, label)) = text.rsplit_once("::") {
		if label.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
			return Some(Want::Enum(label.to_string()));
		}
	}
	if let Ok(x) = text.parse::<f64>() {
		return Some(Want::Float(x));
	}
	None
}

fn unescape(text: &str) -> String {
	let mut out = String::new();
	let mut chars = text.chars();
	while let Some(c) = chars.next() {
		if c != '\\' {
			out.push(c);
			continue;
		}
		match chars.next() {
			Some('n') => out.push('\n'),
			Some('r') => out.push('\r'),
			Some('t') => out.push('\t'),
			Some('0') => out.push('\0'),
			Some(other) => out.push(other),
			None => {}
		}
	}
	out
}

fn matches(value: &Value, want: &Want) -> bool {
	match (value, want) {
		(Value::UInt(v), Want::Int(w)) => i128::try_from(*v).map(|v| v == *w).unwrap_or(false),
		(Value::Int(v), Want::Int(w)) => v == w,
		(Value::Enum { raw, .. }, Want::Int(w)) => raw == w,
		(Value::UInt(v), Want::Bool(w)) => (*v != 0) == *w,
		(Value::Int(v), Want::Bool(w)) => (*v != 0) == *w,
		(Value::Str(v), Want::Str(w)) => v == w,
		(Value::Enum { name: Some(n), .. }, Want::Enum(w)) => n == w,
		(Value::Magic { bytes, .. }, Want::Bytes(w)) => bytes == w,
		(Value::Bytes { len, preview }, Want::Bytes(w)) => {
			*len as usize == preview.len() && preview == w
		}
		(Value::Float(v), Want::Float(w)) => (v - w).abs() < 1e-6,
		(Value::Float(v), Want::Int(w)) => (v - *w as f64).abs() < 1e-6,
		(Value::Unset(inner), _) => matches(inner, want),
		_ => false,
	}
}

fn shown_value(value: &Value) -> String {
	match value {
		Value::UInt(v) => v.to_string(),
		Value::Int(v) => v.to_string(),
		Value::Float(v) => format!("{v}"),
		Value::Str(v) => format!("{v:?}"),
		Value::Bytes { len, preview } => format!("{len} bytes {preview:02x?}"),
		Value::Magic { bytes, ok, .. } => format!("magic {bytes:02x?} ({})", if *ok { "matches" } else { "does not match" }),
		Value::Enum { raw, name, .. } => match name {
			Some(n) => format!("{n} ({raw})"),
			None => format!("{raw}"),
		},
		other => format!("{other:?}"),
	}
}

/// Follow a path of names and indexes through the tree the evaluator reads.
fn read_path(ev: &mut Evaluator, doc: &Document<MemSource>, steps: &[Step]) -> Option<Value> {
	let mut at: Vec<usize> = Vec::new();
	for step in steps {
		match step {
			Step::Index(i) => {
				at.push(*i);
				ev.node(doc, &at).ok()?;
			}
			Step::Name(want) => {
				// A field placed elsewhere is one node with the thing it
				// points at inside it, and so is a compressed run, so a name
				// that is not among the children may be one level in. The
				// evaluator resolves names the same way.
				let mut found = None;
				for _ in 0..3 {
					let here = ev.node(doc, &at).ok()?;
					for i in 0..here.child_count as usize {
						let mut child = at.clone();
						child.push(i);
						let Ok(info) = ev.node(doc, &child) else { continue };
						if info.name == *want {
							found = Some(i);
							break;
						}
					}
					if found.is_some() || here.child_count != 1 {
						break;
					}
					at.push(0);
				}
				at.push(found?);
			}
		}
	}
	ev.node(doc, &at).ok().map(|n| n.value)
}

/// Kept so the unused-import warning does not hide a real one: the oracle reads
/// `.kst` files with the converter's own YAML reader.
#[allow(dead_code)]
fn yaml_kinds(node: &YamlNode) -> bool {
	matches!(node.value, YamlValue::Map(_))
}
