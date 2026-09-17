//! The bytes a file has to open with for a converted format to claim it.
//!
//! This is what `tools/ksy_bundle.mjs` writes into the `signature` of each
//! [`Bundled`](super::Bundled) entry, by way of `examples/ksy_gaps.rs`, and
//! what `tests/ksy_bundled.rs` re-derives to prove the table is not stale.
//! One function, so the two cannot disagree.

use std::sync::Arc;

use crate::decode::fixed_bits;
use crate::template::{Template, Ty};

/// The bytes a dropped file must open with for this format to claim it, as
/// `(offset, bytes)` pairs.
///
/// Eligibility is the rule from DESIGN.md: the first thing the format reads
/// has to be a magic at offset 0, and a format that starts with anything else
/// is never offered for a dropped file. The first thing read is found through
/// nested types: a format whose first `seq` field is a `type: header` whose
/// own first field is a `contents` opens with that magic just as surely as one
/// that writes the `contents` at its root. Everything after that is evidence:
/// the walk carries on through fields whose width does not depend on the
/// data, into nested types the same way, and every further magic it steps
/// over joins the pattern. That is what keeps `avi` from claiming every RIFF
/// file, since a RIFF's fourth word is what says which RIFF it is.
///
/// Empty when the first field is not a magic.
pub fn signature(template: &Template) -> Vec<(u64, Vec<u8>)> {
	let mut out = Vec::new();
	let mut seen = Vec::new();
	walk(template, &template.root, 0, &mut out, &mut seen);
	out
}

/// Walk one type's fields from bit `at`, collecting magics. `Some(width)` if
/// every field was fixed and the walk may carry on after this type, `None` if
/// it stopped inside.
fn walk(
	template: &Template,
	ty: &Ty,
	mut at: u64,
	out: &mut Vec<(u64, Vec<u8>)>,
	seen: &mut Vec<Arc<str>>,
) -> Option<u64> {
	let from = at;
	match ty {
		Ty::Magic(bytes) => {
			// A magic straddling a byte is not one a file can be matched on.
			if at % 8 != 0 {
				return None;
			}
			out.push((at / 8, bytes.clone()));
			return Some(bytes.len() as u64 * 8);
		}
		Ty::Named(name) => {
			// A type that holds itself would walk forever.
			if seen.contains(name) || seen.len() > 32 {
				return None;
			}
			let inner = template.types.get(name.as_ref())?;
			seen.push(name.clone());
			let width = walk(template, inner, at, out, seen);
			seen.pop();
			return width;
		}
		Ty::Struct(def) if !def.overlap => {
			for field in &def.fields {
				match walk(template, &field.ty, at, out, seen) {
					Some(bits) => at += bits,
					None => return None,
				}
			}
			return Some(at - from);
		}
		_ => {}
	}
	// Anything else: no magic here, so it is evidence only once there is a
	// magic to be evidence for, and only if its width is known.
	if out.is_empty() {
		return None;
	}
	let bits = fixed_bits(ty)?;
	((at + bits) % 8 == 0).then_some(bits)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::ksy::NoImports;

	fn sig(text: &str) -> Vec<(u64, Vec<u8>)> {
		let converted = crate::ksy::convert(text, &NoImports).expect("converts");
		signature(&converted.template)
	}

	#[test]
	fn a_magic_at_the_root_is_the_signature() {
		let t = "meta:\n  id: t\nseq:\n  - id: m\n    contents: [0x41, 0x42]\n  - id: n\n    type: u2le\n  - id: k\n    contents: [0x43]\n";
		assert_eq!(sig(t), vec![(0, vec![0x41, 0x42]), (4, vec![0x43])]);
	}

	#[test]
	fn a_magic_inside_the_first_nested_type_counts() {
		let t = "meta:\n  id: t\nseq:\n  - id: header\n    type: header\n  - id: k\n    contents: [0x43]\ntypes:\n  header:\n    seq:\n      - id: m\n        contents: [0x41, 0x42]\n      - id: n\n        type: u2le\n";
		assert_eq!(sig(t), vec![(0, vec![0x41, 0x42]), (4, vec![0x43])]);
	}

	#[test]
	fn a_format_that_starts_with_anything_else_has_none() {
		let t = "meta:\n  id: t\nseq:\n  - id: n\n    type: u2le\n  - id: m\n    contents: [0x41, 0x42]\n";
		assert!(sig(t).is_empty());
		let nested = "meta:\n  id: t\nseq:\n  - id: header\n    type: header\ntypes:\n  header:\n    seq:\n      - id: n\n        type: u2le\n      - id: m\n        contents: [0x41, 0x42]\n";
		assert!(sig(nested).is_empty());
	}

	#[test]
	fn the_walk_stops_at_the_first_field_of_unknown_width() {
		let t = "meta:\n  id: t\nseq:\n  - id: m\n    contents: [0x41, 0x42]\n  - id: len\n    type: u1\n  - id: body\n    size: len\n  - id: k\n    contents: [0x43]\n";
		assert_eq!(sig(t), vec![(0, vec![0x41, 0x42])]);
	}
}
