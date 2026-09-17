//! The `.hexpat` converter, one test per row of the mapping in
//! `docs/HANDOVER-imhex.md`.
//!
//! Every row is checked twice over: what the converter said the pattern became,
//! and what the template then reads out of bytes written by hand for it. A test
//! that only looked at the IR would pass on a lowering that says the right
//! words and reads the wrong bytes, which is the whole failure this converter
//! exists to avoid.
//!
//! The imperative half has its own tests below: each construct the converter
//! does not run has to arrive in the report as a gap, at its own line, with the
//! text it came from. A construct that quietly disappeared would be a file read
//! wrongly with nothing said about it.

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, Value};
use qubero_core::hexpat::{self, Converted, MapIncludes, NoFiles};
use qubero_core::source::MemSource;
use qubero_core::template_text::render;

/// Convert a pattern with no include files but the built-in table.
fn convert(text: &str) -> Converted {
	hexpat::convert(text, &NoFiles).unwrap_or_else(|e| panic!("{text}\n\n{e}"))
}

/// Convert, and fail if anything could not be expressed. Every mapping test
/// uses this: a row that lowers with a gap has not been lowered.
fn clean(text: &str) -> Converted {
	let out = convert(text);
	assert!(
		out.report.is_complete(),
		"gaps: {:?}",
		out.report.gaps.iter().map(|g| format!("{} {}", g.path, g.reason)).collect::<Vec<_>>()
	);
	out
}

/// What the template reads out of `bytes` at the named path, as a string that
/// says what the value is rather than only what it is worth.
fn read(out: &Converted, bytes: &[u8], path: &[&str]) -> String {
	let doc = Document::new(MemSource(bytes.to_vec()));
	let mut ev = Evaluator::new(out.template.clone());
	let mut at: Vec<usize> = Vec::new();
	for name in path {
		// A placed field holds what it points at and nothing else, so naming
		// the field means what is there. The same for the inline structure an
		// `if` block became, which the pattern did not write a name for.
		for _ in 0..4 {
			match ev.child_named(&doc, &at, name).unwrap_or_else(|e| panic!("{name}: {e:?}")) {
				Some(found) => {
					at = found;
					break;
				}
				None => {
					let node = ev.node(&doc, &at).unwrap_or_else(|e| panic!("{at:?}: {e:?}"));
					assert_eq!(node.child_count, 1, "no field {name} under {at:?}\n{}", render(&out.template));
					at.push(0);
				}
			}
		}
	}
	// The last step may land on a placed field, which holds what it points at
	// and takes no room of its own. What the field is worth is what is there.
	let mut node = ev.node(&doc, &at).unwrap_or_else(|e| panic!("{path:?}: {e:?}"));
	while node.size_bits == 0 && node.child_count == 1 && node.composite {
		at.push(0);
		node = ev.node(&doc, &at).unwrap_or_else(|e| panic!("{path:?}: {e:?}"));
	}
	shown(&node.value)
}

fn shown(value: &Value) -> String {
	match value {
		Value::UInt(v) => v.to_string(),
		Value::Int(v) => v.to_string(),
		Value::Str(s) => format!("{s:?}"),
		Value::Enum { raw, name, .. } => match name {
			Some(name) => name.clone(),
			None => format!("<{raw}>"),
		},
		Value::Composite { count } => format!("{count} children"),
		Value::Magic { ok, .. } => format!("magic {ok}"),
		Value::Bytes { len, .. } => format!("{len} bytes"),
		Value::Float(v) => format!("{v}"),
		other => format!("{other:?}"),
	}
}

/// The reasons of every gap, joined, for the tests that check one is reported.
fn gap_reasons(out: &Converted) -> String {
	out.report.gaps.iter().map(|g| format!("{} {} {}", g.path, g.source, g.reason)).collect::<Vec<_>>().join("\n")
}

/* ------------------------------------------------------------------ */
/* The declarative rows                                                */
/* ------------------------------------------------------------------ */

#[test]
fn a_placement_reads_at_the_address_it_names() {
	let out = clean("struct Head { u16 tag; };\nHead head @ 0x04;\n");
	assert_eq!(read(&out, &[0, 0, 0, 0, 0x34, 0x12], &["head", "tag"]), "0x1234".replace("0x1234", "4660"));
}

#[test]
fn the_pragma_endian_settles_every_unsuffixed_type_and_a_prefix_overrides_it() {
	let out = clean("#pragma endian big\nstruct S { u16 a; le u16 b; };\nS s @ 0x00;\n");
	assert_eq!(read(&out, &[0x12, 0x34, 0x12, 0x34], &["s", "a"]), "4660");
	assert_eq!(read(&out, &[0x12, 0x34, 0x12, 0x34], &["s", "b"]), "13330");
}

#[test]
fn every_scalar_width_reads() {
	let out = clean(
		"#pragma endian big\nstruct S { u8 a; u16 b; u24 c; u32 d; s8 e; s16 f; float g; double h; bool i; };\nS s @ 0x00;\n",
	);
	let mut bytes = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0xff, 0xff, 0xfe];
	bytes.extend_from_slice(&1.5f32.to_be_bytes());
	bytes.extend_from_slice(&2.5f64.to_be_bytes());
	bytes.push(1);
	assert_eq!(read(&out, &bytes, &["s", "a"]), "1");
	assert_eq!(read(&out, &bytes, &["s", "b"]), "515");
	assert_eq!(read(&out, &bytes, &["s", "e"]), "-1");
	assert_eq!(read(&out, &bytes, &["s", "g"]), "1.5");
	assert_eq!(read(&out, &bytes, &["s", "h"]), "2.5");
	assert_eq!(read(&out, &bytes, &["s", "i"]), "true");
}

/// Confirmed against the reference: the field owns all N bytes and its value is
/// all of them, embedded NULs included.
///
/// `StrLen::Fixed`, therefore, and not the `StrLen::Padded` the plan asked for:
/// the IR's padded text ends its value at the *first* pad byte, so `ab\0cd`
/// would read as `ab` and a comparison against `"ab"` would come out true where
/// the reference says false. The one thing the two do differently is the
/// display, where the reference drops trailing NULs before printing and the IR
/// prints the bytes the field holds.
#[test]
fn a_char_array_is_all_its_bytes_including_the_nuls_inside_it() {
	let out = clean("struct S { char name[6]; u8 next; };\nS s @ 0x00;\n");
	let bytes = b"ab\0cd\0\x7f";
	assert_eq!(read(&out, bytes, &["s", "name"]), "\"ab\\0cd\\0\"");
	// All six bytes belong to the field, so the byte after it is the seventh.
	assert_eq!(read(&out, bytes, &["s", "next"]), "127");
}

#[test]
fn an_unbounded_char_array_runs_to_the_nul() {
	let out = clean("struct S { char name[]; u8 next; };\nS s @ 0x00;\n");
	assert_eq!(read(&out, b"hi\0\x2a", &["s", "name"]), "\"hi\"");
	assert_eq!(read(&out, b"hi\0\x2a", &["s", "next"]), "42");
}

#[test]
fn an_array_reads_as_many_elements_as_its_count_says() {
	let out = clean("struct S { u8 count; u16 values[count]; };\nS s @ 0x00;\n");
	assert_eq!(read(&out, &[2, 1, 0, 2, 0], &["s", "values"]), "2 children");
}

#[test]
fn an_unbounded_array_runs_to_the_end_of_the_file() {
	let out = clean("struct S { u8 values[]; };\nS s @ 0x00;\n");
	assert_eq!(read(&out, &[1, 2, 3, 4, 5], &["s", "values"]), "5 children");
}

/// Confirmed against the reference: the condition is checked *before* each
/// element, and `$` in it is the absolute position that element would start at.
#[test]
fn a_while_array_checks_its_condition_before_each_element() {
	let out = clean("struct S { u8 values[while($ < 4)]; u8 after; };\nS s @ 0x00;\n");
	assert_eq!(read(&out, &[1, 2, 3, 4, 9], &["s", "values"]), "4 children");
	assert_eq!(read(&out, &[1, 2, 3, 4, 9], &["s", "after"]), "9");
}

#[test]
fn a_while_array_over_eof_is_a_run_to_the_end() {
	let out = clean("struct S { u8 values[while(!std::mem::eof())]; };\nS s @ 0x00;\n");
	let text = render(&out.template);
	assert!(text.contains("until end"), "{text}");
	assert_eq!(read(&out, &[1, 2, 3], &["s", "values"]), "3 children");
}

#[test]
fn padding_is_read_and_not_shown() {
	let out = clean("struct S { u8 a; padding[3]; u8 b; };\nS s @ 0x00;\n");
	assert_eq!(read(&out, &[1, 0, 0, 0, 7], &["s", "b"]), "7");
}

/// Bit order, confirmed against the reference: bits pack from the low bit up
/// under `little`, which is the default. Over `0x49` the first two-bit field is
/// 1 and the three-bit field after it is 2.
#[test]
fn a_bitfield_packs_from_the_low_bit_up_by_default() {
	let out = clean("bitfield B { a : 2; b : 3; padding : 3; };\nB b @ 0x00;\n");
	assert_eq!(read(&out, &[0x49], &["b", "a"]), "1");
	assert_eq!(read(&out, &[0x49], &["b", "b"]), "2");
}

/// And from the high bit down under `big`, which is what the reference's own
/// `TestPatternBitfields` reads out of `0x49`.
#[test]
fn a_bitfield_packs_from_the_high_bit_down_under_big() {
	let out = clean("#pragma endian big\nbitfield B { a : 2; b : 3; padding : 3; };\nB b @ 0x00;\n");
	assert_eq!(read(&out, &[0x49], &["b", "a"]), "1");
	assert_eq!(read(&out, &[0x49], &["b", "b"]), "1");
}

#[test]
fn a_bitfield_holds_signed_fields_and_named_ones() {
	let out = clean(
		"#pragma endian big\nenum E : u8 { Zero, One, Two, Three };\n\
		 bitfield B { signed s : 4; E e : 2; bool f : 1; padding : 1; };\nB b @ 0x00;\n",
	);
	assert_eq!(read(&out, &[0b1111_10_1_0], &["b", "s"]), "-1");
	assert_eq!(read(&out, &[0b1111_10_1_0], &["b", "e"]), "Two");
	assert_eq!(read(&out, &[0b1111_10_1_0], &["b", "f"]), "true");
}

/// The one thing the IR cannot say about a bitfield: a field packed from the
/// bottom of its byte that runs into the next one is not a single range of bits
/// in an address space numbered from the top of each byte.
#[test]
fn a_low_bit_first_field_crossing_a_byte_is_a_gap() {
	let out = convert("bitfield B { a : 4; b : 10; padding : 2; };\nB b @ 0x00;\n");
	assert!(gap_reasons(&out).contains("crosses into the next byte"), "{}", gap_reasons(&out));
}

#[test]
fn a_union_lays_every_field_over_the_same_bytes() {
	let out = clean("union U { u32 packed; u8 bytes[4]; };\nU u @ 0x00;\nu8 after @ 0x04;\n");
	let text = render(&out.template);
	assert!(text.contains("(overlap)"), "{text}");
	assert_eq!(read(&out, &[1, 0, 0, 0, 9], &["u", "packed"]), "1");
	assert_eq!(read(&out, &[1, 0, 0, 0, 9], &["u", "bytes"]), "4 children");
	assert_eq!(read(&out, &[1, 0, 0, 0, 9], &["after"]), "9");
}

#[test]
fn a_pointer_is_an_offset_and_what_it_leads_to() {
	let out = clean("struct S { u16 *target : u16; };\nS s @ 0x00;\n");
	let bytes = [0x04, 0x00, 0x00, 0x00, 0x2a, 0x00];
	assert_eq!(read(&out, &bytes, &["s", "target", "offset"]), "4");
	assert_eq!(read(&out, &bytes, &["s", "target", "target"]), "42");
}

#[test]
fn an_if_else_is_one_condition_over_each_block() {
	let out = clean("struct S { u8 tag; if (tag == 1) { u16 one; } else { u32 other; } };\nS s @ 0x00;\n");
	let text = render(&out.template);
	assert!(text.contains("when tag == 1"), "{text}");
	assert_eq!(read(&out, &[1, 0x0a, 0x00], &["s", "if_1", "one"]), "10");
	assert_eq!(read(&out, &[2, 1, 0, 0, 0], &["s", "else_2", "other"]), "1");
}

#[test]
fn a_match_on_one_value_is_a_switch() {
	let out = clean(
		"struct S { u8 tag; match (tag) { (1): u8 one; (2 ... 4): u16 few; (_): u32 rest; } };\nS s @ 0x00;\n",
	);
	let text = render(&out.template);
	assert!(text.contains("switch tag"), "{text}");
	assert_eq!(read(&out, &[1, 7], &["s", "chosen", "one"]), "7");
	assert_eq!(read(&out, &[3, 7, 0], &["s", "chosen", "few"]), "7");
	assert_eq!(read(&out, &[9, 1, 0, 0, 0], &["s", "chosen", "rest"]), "1");
}

#[test]
fn a_template_is_monomorphised_per_instantiation() {
	let out = clean(
		"struct Run<auto N, T> { T values[N]; };\nstruct S { Run<2, u8> small; Run<3, u16> wide; };\nS s @ 0x00;\n",
	);
	let bytes = [1, 2, 3, 0, 4, 0, 5, 0];
	assert_eq!(read(&out, &bytes, &["s", "small", "values"]), "2 children");
	assert_eq!(read(&out, &bytes, &["s", "wide", "values"]), "3 children");
	// Two copies, one per instantiation, and the wide one reads two-byte
	// elements where the small one reads one.
	let text = render(&out.template);
	assert!(text.contains("u8be[2]"), "{text}");
	assert!(text.contains("u16le[3]"), "{text}");
}

#[test]
fn inheritance_reads_the_base_members_first() {
	let out = clean("struct Base { u8 first; };\nstruct S : Base { u8 second; };\nS s @ 0x00;\n");
	assert_eq!(read(&out, &[1, 2], &["s", "first"]), "1");
	assert_eq!(read(&out, &[1, 2], &["s", "second"]), "2");
}

#[test]
fn a_struct_local_is_a_value_with_no_bytes_of_its_own() {
	let out = clean("struct S { u8 half; u32 whole = half * 2; u8 after; };\nS s @ 0x00;\n");
	assert_eq!(read(&out, &[5, 9], &["s", "whole"]), "10");
	// The local read nothing, so the next field is the second byte.
	assert_eq!(read(&out, &[5, 9], &["s", "after"]), "9");
}

#[test]
fn an_enum_names_its_values_and_a_range_names_every_value_in_it() {
	let out = clean(
		"#pragma endian big\nenum E : u16 { A, B = 5, C = 0x10 ... 0x1F };\nstruct S { E one; E two; E three; };\nS s @ 0x00;\n",
	);
	let bytes = [0, 0, 0, 5, 0, 0x1f];
	assert_eq!(read(&out, &bytes, &["s", "one"]), "A");
	assert_eq!(read(&out, &bytes, &["s", "two"]), "B");
	assert_eq!(read(&out, &bytes, &["s", "three"]), "C");
	// And nothing above the range is named, which an unbounded span would have
	// got wrong.
	let out2 = clean("enum E : u8 { C = 0x10 ... 0x1f };\nE e @ 0x00;\n");
	assert_eq!(read(&out2, &[0x20], &["e"]), "<32>");
}

#[test]
fn a_type_magic_is_a_magic() {
	let out = clean("#include <type/magic.pat>\ntype::Magic<\"PK\"> magic @ 0x00;\n");
	assert_eq!(read(&out, b"PK", &["magic"]), "magic true");
	assert_eq!(read(&out, b"XX", &["magic"]), "magic false");
}

#[test]
fn a_type_time_is_a_moment() {
	let out = clean("#include <type/time.pat>\n#pragma endian big\ntype::time32_t when @ 0x00;\n");
	let text = render(&out.template);
	assert!(text.contains("seconds from 1970") || text.contains("time"), "{text}");
	assert_eq!(read(&out, &[0, 0, 0, 1, 0], &["when", "value"]), "1");
}

#[test]
fn the_pragma_magic_becomes_a_signature_at_the_front_of_the_root() {
	let out = clean("#pragma magic [ 50 4B ] @ 0x00\nu8 first @ 0x00;\n");
	assert_eq!(read(&out, b"PK\0", &["magic"]), "magic true");
	assert_eq!(read(&out, b"PK\0", &["first"]), "80");
}

#[test]
fn an_include_the_caller_supplies_wins_over_the_built_in_table() {
	let includes = MapIncludes::new().with("my/own.pat", "#pragma once\nstruct Mine { u8 value; };\n");
	let out = hexpat::convert("#include <my/own.pat>\nMine mine @ 0x00;\n", &includes).expect("converted");
	assert!(out.report.is_complete(), "{}", gap_reasons(&out));
	assert_eq!(read(&out, &[3], &["mine", "value"]), "3");
}

#[test]
fn an_include_nobody_has_is_an_error_naming_the_path() {
	let Err(error) = hexpat::convert("#include <nowhere/at/all.pat>\n", &NoFiles) else {
		panic!("there is no such file, so this should not have converted");
	};
	assert!(error.message.contains("nowhere/at/all.pat"), "{}", error.message);
	assert_eq!(error.pos.line, 1);
}

/* ------------------------------------------------------------------ */
/* Attributes                                                          */
/* ------------------------------------------------------------------ */

#[test]
fn a_comment_attribute_becomes_the_fields_prose() {
	let out = clean("struct S { u8 a [[comment(\"how many\")]]; };\nS s @ 0x00;\n");
	assert!(render(&out.template).contains("how many"), "{}", render(&out.template));
}

#[test]
fn a_name_attribute_is_a_note_and_the_declared_name_stays_the_name() {
	let out = clean("struct S { u8 a [[name(\"Count\")]]; };\nS s @ 0x00;\n");
	assert_eq!(read(&out, &[4], &["s", "a"]), "4");
	assert!(out.report.notes.iter().any(|n| n.message.contains("another name")), "{:?}", out.report.notes);
}

/// `[[transform]]` replaces the value, so a length or a condition reading the
/// field reads what the function returned. The plan called it a note; it is a
/// gap, and a `cpio` header is why.
#[test]
fn a_transform_attribute_is_a_gap_because_it_changes_the_value() {
	let out = convert("struct S { u8 a [[transform(\"f\")]]; };\nS s @ 0x00;\n");
	assert!(gap_reasons(&out).contains("[[transform]]"), "{}", gap_reasons(&out));
}

/// `[[format]]` only changes what is printed, so the raw value is still the
/// value and a note says the function was not run.
#[test]
fn a_format_attribute_is_a_note() {
	let out = clean("struct S { u8 a [[format(\"f\")]]; };\nS s @ 0x00;\n");
	assert!(out.report.notes.iter().any(|n| n.message.contains("raw value")), "{:?}", out.report.notes);
}

#[test]
fn a_no_unique_address_field_leaves_the_next_one_where_it_was() {
	let out = clean("struct S { u32 whole [[no_unique_address]]; u8 first; };\nS s @ 0x00;\n");
	assert_eq!(read(&out, &[1, 2, 3, 4], &["s", "whole"]), "67305985");
	assert_eq!(read(&out, &[1, 2, 3, 4], &["s", "first"]), "1");
}

/// The reference ignores an attribute it does not know rather than rejecting
/// the pattern, and so does this. `job.hexpat` writes one.
#[test]
fn an_attribute_nobody_knows_is_read_and_noted() {
	let out = clean("struct S { u8 a [[attribute(\"hidden\")]]; };\nS s @ 0x00;\n");
	assert!(out.report.notes.iter().any(|n| n.source.contains("attribute")), "{:?}", out.report.notes);
}

/// And the same for a pragma, which `Preprocessor::process` passes over when
/// nothing registered a handler for it.
#[test]
fn a_pragma_nobody_knows_is_read_and_noted() {
	let out = clean("#pragma organization somebody\nu8 a @ 0x00;\n");
	assert!(out.report.notes.iter().any(|n| n.source.contains("organization")), "{:?}", out.report.notes);
}

/* ------------------------------------------------------------------ */
/* The imperative half, which is gaps                                  */
/* ------------------------------------------------------------------ */

/// One per `StatementKind`: the construct has to arrive in the report with its
/// own line and the text it came from.
#[test]
fn every_imperative_construct_becomes_a_gap_at_its_own_line() {
	// A `while` and a `for` are only legal at the top level; the reference
	// refuses either inside a structure, and so does the parser.
	let cases: &[(&str, &str)] = &[
		("struct S { u8 first; first = 2; };\nS s @ 0x00;\n", "an assignment"),
		("struct S { u8 first; break; };\nS s @ 0x00;\n", "a `break`"),
		("struct S { u8 first; continue; };\nS s @ 0x00;\n", "a `continue`"),
		("struct S { u8 first; return; };\nS s @ 0x00;\n", "a `return`"),
		("u8 a @ 0x00;\nwhile (a < 2) { a = 1; }\n", "a `while` statement"),
		("u8 a @ 0x00;\nfor (u8 i = 0, i < 2, i = i + 1) { a = 1; }\n", "a `for` loop"),
		("u8 a @ 0x00;\nu8 b;\nb = 2;\n", "an assignment"),
	];
	for (text, want) in cases {
		let out = convert(text);
		let reasons = gap_reasons(&out);
		assert!(reasons.contains(want), "{text}\nwanted {want}, got:\n{reasons}");
		// Every gap says where it is.
		for gap in &out.report.gaps {
			assert!(gap.path.contains(':'), "{} has no line and column", gap.path);
		}
	}
}

#[test]
fn a_try_catch_is_a_gap() {
	let out = convert("struct S { try { u8 a; } catch { u16 b; } };\nS s @ 0x00;\n");
	assert!(gap_reasons(&out).contains("try/catch"), "{}", gap_reasons(&out));
}

#[test]
fn an_in_variable_is_a_gap() {
	let out = convert("u8 setting in;\nu8 a @ 0x00;\n");
	assert!(gap_reasons(&out).contains("in/out"), "{}", gap_reasons(&out));
}

#[test]
fn a_call_that_computes_is_a_gap_and_one_that_only_says_something_is_a_note() {
	let out = convert("fn f() { return 1; };\nstruct S { u8 a; u8 b[f()]; };\nS s @ 0x00;\n");
	assert!(gap_reasons(&out).contains("the converter runs nothing"), "{}", gap_reasons(&out));

	let checked = convert("struct S { u8 a; std::assert(a == 1, \"no\"); u8 b; };\nS s @ 0x00;\n");
	assert!(checked.report.is_complete(), "{}", gap_reasons(&checked));
	assert!(checked.report.notes.iter().any(|n| n.message.contains("checks that")), "{:?}", checked.report.notes);
}

/// A field that should have read bytes and could not moves everything after it,
/// so the structure ends there and the report says so. Carrying on would put
/// every later field at an offset nothing in the file agrees with.
#[test]
fn a_structure_ends_at_the_member_that_could_not_be_placed() {
	let out = convert("fn f() { return 1; };\nstruct S { u8 a[f()]; u8 b; };\nS s @ 0x00;\n");
	assert!(gap_reasons(&out).contains("the rest of the structure"), "{}", gap_reasons(&out));
	assert!(!render(&out.template).contains("b:"), "{}", render(&out.template));
}

/* ------------------------------------------------------------------ */
/* Assignments                                                         */
/* ------------------------------------------------------------------ */

/// `$ += n` moves the cursor forward and reads nothing, which is what
/// `padding[n]` says, so the two lower the same way and the field after it is
/// where the pattern puts it.
#[test]
fn a_forward_cursor_move_is_the_bytes_it_skips() {
	let out = clean("struct S { u8 first; $ += 2; u8 last; };\nS s @ 0x00;\n");
	assert_eq!(read(&out, &[1, 0xaa, 0xbb, 9], &["s", "first"]), "1");
	assert_eq!(read(&out, &[1, 0xaa, 0xbb, 9], &["s", "last"]), "9");
	let padded = clean("struct S { u8 first; padding[2]; u8 last; };\nS s @ 0x00;\n");
	assert_eq!(render(&out.template), render(&padded.template));
}

/// The skipped length may be worked out from the fields before it, the same as
/// any other `padding`.
#[test]
fn a_cursor_move_may_skip_a_length_the_file_states() {
	let out = clean("struct S { u8 skip; $ += skip; u8 last; };\nS s @ 0x00;\n");
	assert_eq!(read(&out, &[3, 0, 0, 0, 7], &["s", "last"]), "7");
}

/// `$ = e` and `$ -= e` put the cursor where the structure cannot follow. The
/// reference sizes a structure as the distance from where it started to where
/// the cursor ended, and an `At` in the IR advances nothing, so a structure
/// wrapped in one would read the right bytes and report the wrong length. The
/// structure ends at the move instead, and the report says both things.
#[test]
fn a_cursor_move_the_ir_cannot_follow_ends_the_structure() {
	for text in [
		"struct S { u8 first; $ = 0x10; u8 last; };\nS s @ 0x00;\n",
		"struct S { u8 first; $ -= 1; u8 last; };\nS s @ 0x00;\n",
	] {
		let out = convert(text);
		let reasons = gap_reasons(&out);
		assert!(reasons.contains("the cursor moved to an address of its own"), "{text}\n{reasons}");
		assert!(reasons.contains("the rest of the structure"), "{text}\n{reasons}");
		assert!(!render(&out.template).contains("last"), "{text}\n{}", render(&out.template));
	}
}

/// A local set in the two halves of one `if` and nowhere else is a value that
/// depends on the condition and on nothing else, which `Cond` says exactly. It
/// is emitted where the `if` ends, because that is where the pattern has
/// settled it.
#[test]
fn a_local_two_halves_of_one_if_settle_is_a_choice_between_them() {
	let out = clean(
		"struct S {\n\
		 \tu8 kind;\n\
		 \tu8 size = 0;\n\
		 \tif (kind == 1) {\n\
		 \t\tsize = 4;\n\
		 \t} else {\n\
		 \t\tsize = 2;\n\
		 \t}\n\
		 \tu8 body[size];\n\
		 };\n\
		 S s @ 0x00;\n",
	);
	assert_eq!(read(&out, &[1, 1, 2, 3, 4], &["s", "size"]), "4");
	assert_eq!(read(&out, &[1, 1, 2, 3, 4], &["s", "body"]), "4 children");
	assert_eq!(read(&out, &[2, 1, 2, 3, 4], &["s", "size"]), "2");
	assert_eq!(read(&out, &[2, 1, 2, 3, 4], &["s", "body"]), "2 children");
}

/// Only the `then` half need write it: the other half leaves the value the
/// declaration gave.
#[test]
fn a_local_only_one_half_settles_keeps_its_declared_value_in_the_other() {
	let out = clean(
		"struct S { u8 kind; u8 size = 1; if (kind == 1) { size = 3; } u8 body[size]; };\nS s @ 0x00;\n",
	);
	assert_eq!(read(&out, &[1, 9, 9, 9], &["s", "size"]), "3");
	assert_eq!(read(&out, &[0, 9, 9, 9], &["s", "size"]), "1");
}

/// The fold is only taken when one `if` holds every assignment. A local two
/// `if`s write to, or one a loop writes to, is still a value that changes as
/// the pattern runs.
#[test]
fn a_local_more_than_one_if_writes_to_is_still_a_gap() {
	let out = convert(
		"struct S {\n\
		 \tu8 kind;\n\
		 \tu8 size = 0;\n\
		 \tif (kind == 1) { size = 4; }\n\
		 \tif (kind == 2) { size = 2; }\n\
		 };\n\
		 S s @ 0x00;\n",
	);
	assert!(gap_reasons(&out).contains("a local the pattern assigns to again"), "{}", gap_reasons(&out));
}

/// A name declared inside an `if` block is in the pattern's scope from there on
/// and sits one structure deeper in the IR, where a later sibling's name does
/// not reach it. Naming it anyway would read the wrong field, so it is a gap.
#[test]
fn a_name_from_inside_an_if_block_is_a_gap_where_it_is_used_after_it() {
	let out = convert("struct S { u8 tag; if (tag == 1) { u8 size; } u8 rest[size]; };\nS s @ 0x00;\n");
	assert!(gap_reasons(&out).contains("declared inside an `if` block"), "{}", gap_reasons(&out));
}

/* ------------------------------------------------------------------ */
/* Snapshots                                                           */
/* ------------------------------------------------------------------ */

/// Three small patterns whose whole IR text is kept, for the reason the
/// built-in formats' snapshots are kept: the notation is for a reader, and a
/// line that gets worse should show up as a diff somebody can read. Rewrite
/// them with `UPDATE_SNAPSHOTS=1 cargo test --test hexpat_lower`, and read what
/// changed before committing it.
const SNAPSHOTS: &[(&str, &str)] = &[
	(
		"records",
		"#pragma description A run of tagged records\n\
		 #pragma endian big\n\
		 #pragma magic [ 52 45 43 ] @ 0x00\n\
		 \n\
		 enum Kind : u8 {\n\
		 \tEmpty = 0,\n\
		 \tText = 1,\n\
		 \tNumbers = 0x10 ... 0x12\n\
		 };\n\
		 \n\
		 struct Record {\n\
		 \tKind kind;\n\
		 \tu16 length;\n\
		 \tif (kind == Kind::Text) {\n\
		 \t\tchar text[length];\n\
		 \t} else {\n\
		 \t\tu32 numbers[length];\n\
		 \t}\n\
		 };\n\
		 \n\
		 struct File {\n\
		 \tchar magic[3];\n\
		 \tRecord records[while(!std::mem::eof())];\n\
		 };\n\
		 \n\
		 File file @ 0x00;\n",
	),
	(
		"flags",
		"bitfield Flags {\n\
		 \tvisible : 1;\n\
		 \tsigned weight : 3;\n\
		 \tpadding : 4;\n\
		 };\n\
		 \n\
		 union Colour {\n\
		 \tu32 packed;\n\
		 \tu8 channels[4];\n\
		 };\n\
		 \n\
		 struct Head {\n\
		 \tFlags flags;\n\
		 \tColour colour;\n\
		 \tu16 *label : u16;\n\
		 };\n\
		 \n\
		 Head head @ 0x00;\n",
	),
	(
		"templates",
		"struct Run<auto Count, T> {\n\
		 \tT values[Count];\n\
		 };\n\
		 \n\
		 struct Head {\n\
		 \tu8 count;\n\
		 \tu32 doubled = count * 2;\n\
		 \tRun<2, u8> pair;\n\
		 \tRun<3, u16> triple;\n\
		 \tmatch (count) {\n\
		 \t\t(0): u8 none;\n\
		 \t\t(_): u8 some;\n\
		 \t}\n\
		 };\n\
		 \n\
		 Head head @ 0x00;\n",
	),
];

#[test]
fn the_snapshots_still_read_the_same_way() {
	let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
		.join("tests")
		.join("snapshots")
		.join("template_text");
	let update = std::env::var("UPDATE_SNAPSHOTS").is_ok_and(|v| v == "1");
	let mut stale = Vec::new();
	for (name, pattern) in SNAPSHOTS {
		let out = convert(pattern);
		let mut text = render(&out.template);
		text.push_str("\nreport\n");
		for note in &out.report.notes {
			text.push_str(&format!("  note {} {}: {}\n", note.path, note.source, note.message));
		}
		for gap in &out.report.gaps {
			text.push_str(&format!("  gap {} {}: {}\n", gap.path, gap.source, gap.reason));
		}
		let path = dir.join(format!("hexpat_{name}.txt"));
		if update {
			std::fs::create_dir_all(&dir).expect("snapshot directory");
			std::fs::write(&path, &text).expect("write snapshot");
			continue;
		}
		match std::fs::read_to_string(&path) {
			Err(_) => stale.push(format!("{name}: no snapshot at {}", path.display())),
			Ok(want) if want.replace("\r\n", "\n") != text => {
				stale.push(format!("{name}: the rendering has changed"))
			}
			Ok(_) => {}
		}
	}
	assert!(stale.is_empty(), "{}\nRerun with UPDATE_SNAPSHOTS=1 to rewrite them.", stale.join("\n"));
}
