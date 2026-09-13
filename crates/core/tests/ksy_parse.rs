//! The `.ksy` reader against the Kaitai corpus.
//!
//! Point `KAITAI_STRUCT` at a checkout of <https://github.com/kaitai-io/kaitai_struct>
//! (the recursive one, so that `formats/` and `tests/` are populated) and this
//! reads every format in it:
//!
//! ```text
//! KAITAI_STRUCT=D:/github/kaitai_struct cargo test -p qubero-core --test ksy_parse -- --nocapture
//! ```
//!
//! Three things are checked. Every format under `formats/` and every one under
//! `tests/formats/` has to parse, because this is the strict tier and those are
//! the files the Kaitai compiler itself accepts. Every expression written in
//! any of them has to survive being printed and read again as the same tree,
//! because the report and every error message show the printed form. And every
//! file under `tests/formats_err/` has to fail, except the ones listed here as
//! failing later than parsing: type resolution and expression typing are
//! separate passes in the compiler too, and this reader does not do them.
//!
//! Without the variable the whole thing is skipped with a printed notice.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use qubero_core::ksy::{self, expr};

/// Files under `tests/formats_err/` that this reader accepts, each with the
/// pass that actually rejects them in the Kaitai compiler.
///
/// The list is the point: it says exactly how far a parser alone can get. Do
/// not add a file here to make the test green without reading its own header
/// comment, which states the error the compiler gives and therefore which pass
/// gives it.
const ERR_PARSES: &[(&str, &str)] = &[
	// Expression typing: the expression is well formed, and it is the types of
	// the things in it that do not work out.
	("attr_bad_if", TYPED),
	("attr_bad_if2", TYPED),
	("attr_bad_size", TYPED),
	("attr_bad_size_str", TYPED),
	("attr_bad_to_string", TYPED),
	("attr_bad_valid_any_of", TYPED),
	("attr_bad_valid_eq_long", TYPED),
	("attr_bad_valid_eq_short", TYPED),
	("attr_bad_valid_expr", TYPED),
	("attr_bad_valid_range", TYPED),
	("attr_bad_valid_repeat_eq_short", TYPED),
	("attr_invalid_repeat_expr", TYPED),
	("attr_invalid_repeat_until", TYPED),
	("attr_invalid_switch_eq", TYPED),
	("attr_invalid_switch_inner", TYPED),
	("enum_bad_type_inst_value", TYPED),
	("expr_bad_not", TYPED),
	("expr_bytes_to_s_arg0", TYPED),
	("expr_bytes_to_s_arg2", TYPED),
	("expr_bytes_to_s_dynamic_encoding", TYPED),
	("expr_bytes_to_s_type", TYPED),
	("expr_compare_enum", TYPED),
	("expr_compare_enum2", TYPED),
	("expr_unknown_method1", TYPED),
	("expr_unknown_method2", TYPED),
	("instance_pos_bad", TYPED),
	("io_on_bytes", TYPED),
	("many_type_validator", TYPED),
	("nav_parent_multi", TYPED),
	("nav_parent_multi_one_unused", TYPED),
	("nav_parent_unused", TYPED),
	("nav_parent_unused_import", TYPED),
	("params_call_bad_type_subtype_import", TYPED),
	("params_call_bad_type_subtype_local", TYPED),
	("params_call_bad_type_top_import", TYPED),
	("params_call_bad_type_top_local", TYPED),
	// Name resolution: a type, an enum, an enum member or a field that is
	// written correctly and does not exist.
	("enum_unknown_inst_value", UNRESOLVED),
	("enum_unknown_seq", UNRESOLVED),
	("expr_bad_id_inst_value", UNRESOLVED),
	("expr_bad_id_size", UNRESOLVED),
	("expr_cast_type_unknown", UNRESOLVED),
	("expr_enum_member_unknown", UNRESOLVED),
	("expr_enum_unknown", UNRESOLVED),
	("expr_field_unknown_endian_switch_cases", UNRESOLVED),
	("expr_field_unknown_endian_switch_on", UNRESOLVED),
	("expr_field_unknown_if_inst_pos", UNRESOLVED),
	("expr_field_unknown_if_inst_value", UNRESOLVED),
	("expr_field_unknown_if_seq", UNRESOLVED),
	("expr_field_unknown_inst_value_enum", UNRESOLVED),
	("expr_field_unknown_params_call", UNRESOLVED),
	("expr_field_unknown_switch_cases", UNRESOLVED),
	("expr_field_unknown_switch_on", UNRESOLVED),
	("expr_field_unknown_switch_params_call", UNRESOLVED),
	("expr_field_unknown_valid_any_of", UNRESOLVED),
	("expr_field_unknown_valid_eq_long", UNRESOLVED),
	("expr_field_unknown_valid_eq_short", UNRESOLVED),
	("expr_field_unknown_valid_expr", UNRESOLVED),
	("expr_field_unknown_valid_range", UNRESOLVED),
	("params_def_type_unknown", UNRESOLVED),
	("switch_cases_malformed_quoting2", UNRESOLVED),
	("type_unknown", UNRESOLVED),
	("type_unknown_many", UNRESOLVED),
	("type_unknown_switch", UNRESOLVED),
	// The declared type is there; the call does not match it.
	("params_call_too_many_subtype_import", "the type is declared with fewer parameters than the call passes"),
	("params_call_too_many_subtype_local", "the type is declared with fewer parameters than the call passes"),
	("params_call_too_many_top_import", "the type is declared with fewer parameters than the call passes"),
	("params_call_too_many_top_local", "the type is declared with fewer parameters than the call passes"),
	// `sizeof<>` of a type whose size is not fixed, which is only known once
	// every field of that type has a size.
	("expr_sizeof_value_dynamic1", "sizeof<> of a type with no fixed size"),
	("expr_sizeof_value_dynamic2", "sizeof<> of a type with no fixed size"),
	("expr_sizeof_value_dynamic3", "sizeof<> of a type with no fixed size"),
	// Warnings, not errors: the compiler reads these files and compiles them.
	("encoding_meta_bad", WARNING),
	("encoding_meta_warnings", WARNING),
	("encoding_str_bad", WARNING),
	("encoding_str_warnings", WARNING),
	("style_bad_len_inst_pos", WARNING),
	("style_bad_len_inst_value", WARNING),
	("style_bad_len_seq", WARNING),
	("style_bad_num_inst_pos", WARNING),
	("style_bad_num_inst_value", WARNING),
	("style_bad_num_seq", WARNING),
	// The rest.
	("meta_imports_abs_unknown", "the imported .ksy is not found, which needs a resolver"),
	("meta_imports_rel2", "the imported .ksy is not found, which needs a resolver"),
	("meta_imports_rel_unknown", "the imported .ksy is not found, which needs a resolver"),
	("meta_high_ks_version", "asks for a compiler newer than 0.11; this reader claims no version"),
	("params_def_subtype_imported", "not an error itself: another test imports it"),
	("params_def_top_imported", "not an error itself: another test imports it"),
];

const TYPED: &str = "the expression is well formed and its types are wrong, which is a later pass";
const UNRESOLVED: &str = "names a type, enum, member or field that does not exist, which is a later pass";
const WARNING: &str = "a warning in the compiler, not an error";

#[test]
fn every_format_in_the_corpus_parses() {
	let Some(root) = corpus() else { return };

	let formats = collect(&root.join("formats"));
	let test_formats = collect(&root.join("tests/formats"));
	let err_formats = collect(&root.join("tests/formats_err"));
	assert!(!formats.is_empty(), "no .ksy files under {}", root.join("formats").display());

	let mut failures = Vec::new();
	let mut expressions = 0usize;
	let mut round_trip_failures = Vec::new();

	for (label, files) in [("formats", &formats), ("tests/formats", &test_formats)] {
		let mut parsed = 0usize;
		for path in files {
			let text = fs::read_to_string(path).expect("a readable .ksy");
			match ksy::parse(&text) {
				Err(e) => failures.push(format!("{}: {e}", shown(&root, path))),
				Ok(spec) => {
					parsed += 1;
					spec.for_each_expr(&mut |where_, e| {
						expressions += 1;
						let printed = e.to_string();
						match expr::parse(&printed) {
							Ok(again) if again == *e => {}
							Ok(_) => round_trip_failures.push(format!(
								"{}{where_}: `{printed}` reads back as a different expression",
								shown(&root, path)
							)),
							Err(err) => round_trip_failures.push(format!(
								"{}{where_}: `{printed}` does not read back: {err}",
								shown(&root, path)
							)),
						}
					});
				}
			}
		}
		println!("{label}: {parsed}/{} parsed", files.len());
	}
	println!("expressions: {expressions} parsed and printed back");

	// The files that are supposed to fail.
	let mut unexpected_parses = Vec::new();
	let mut refused = 0usize;
	for path in &err_formats {
		let text = fs::read_to_string(path).expect("a readable .ksy");
		match ksy::parse(&text) {
			Err(_) => refused += 1,
			Ok(_) => unexpected_parses.push(name(path)),
		}
	}
	println!(
		"tests/formats_err: {refused}/{} refused, {} left to a later pass",
		err_formats.len(),
		unexpected_parses.len()
	);

	let allowed: BTreeMap<&str, &str> = ERR_PARSES.iter().copied().collect();
	let mut surprises = Vec::new();
	for file in &unexpected_parses {
		if !allowed.contains_key(file.as_str()) {
			surprises.push(file.clone());
		}
	}
	let mut vanished = Vec::new();
	for file in allowed.keys() {
		if !unexpected_parses.iter().any(|f| f == file) {
			vanished.push(file.to_string());
		}
	}
	if !surprises.is_empty() {
		println!("not refused and not listed ({}):", surprises.len());
		for file in &surprises {
			println!("  {file}");
		}
	}

	assert!(failures.is_empty(), "{} formats did not parse:\n{}", failures.len(), failures.join("\n"));
	assert!(
		round_trip_failures.is_empty(),
		"{} expressions did not survive printing:\n{}",
		round_trip_failures.len(),
		round_trip_failures.join("\n")
	);
	assert!(
		surprises.is_empty(),
		"{} files under tests/formats_err parsed without being listed as later-pass errors:\n  {}",
		surprises.len(),
		surprises.join("\n  ")
	);
	assert!(
		vanished.is_empty(),
		"{} files are listed as later-pass errors but are now refused at parse time; \
		 take them out of ERR_PARSES:\n  {}",
		vanished.len(),
		vanished.join("\n  ")
	);
}

/// The checkout to read, or nothing and a notice.
fn corpus() -> Option<PathBuf> {
	match std::env::var("KAITAI_STRUCT") {
		Ok(path) if !path.is_empty() => Some(PathBuf::from(path)),
		_ => {
			println!(
				"skipped: set KAITAI_STRUCT to a kaitai_struct checkout to read its 185 formats \
				 and 339 test formats"
			);
			None
		}
	}
}

/// Every `.ksy` under `dir`, sorted, skipping the compiler's own build output.
fn collect(dir: &Path) -> Vec<PathBuf> {
	let mut out = Vec::new();
	walk(dir, &mut out);
	out.sort();
	out
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
	let Ok(entries) = fs::read_dir(dir) else { return };
	for entry in entries.flatten() {
		let path = entry.path();
		if path.is_dir() {
			if path.file_name().is_some_and(|n| n == "_build") {
				continue;
			}
			walk(&path, out);
		} else if path.extension().is_some_and(|e| e == "ksy") {
			out.push(path);
		}
	}
}

fn name(path: &Path) -> String {
	path.file_stem().unwrap_or_default().to_string_lossy().to_string()
}

fn shown(root: &Path, path: &Path) -> String {
	path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/")
}
