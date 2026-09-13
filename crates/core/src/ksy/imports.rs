//! Where a `meta/imports` entry's text comes from.
//!
//! A `.ksy` may name other `.ksy` files and use the types in them. Nothing in
//! the converter knows what a file system is, so the caller says what an import
//! name resolves to: the bundled formats, a map the panel filled in from files
//! the user dropped, or nothing at all.
//!
//! An import name is written the way the compiler writes it: `/common/bcd` is
//! absolute, from the root of whatever the resolver considers its collection,
//! and `vlq_base128_le` is relative to the importing file. Neither form has the
//! `.ksy` on it. The resolver is handed the name as written.

use std::collections::HashMap;

/// What a `meta/imports` name resolves to.
pub trait Imports {
	/// The text of the imported `.ksy`, or nothing if it is not available.
	fn load(&self, name: &str) -> Option<String>;
}

/// A resolver with nothing in it. A `.ksy` that imports anything is a gap.
pub struct NoImports;

impl Imports for NoImports {
	fn load(&self, _name: &str) -> Option<String> {
		None
	}
}

/// A resolver over a map from import name to `.ksy` text.
///
/// The name is looked up as written, then with any leading `/` taken off, and
/// failing both by its last component against the last component of every key.
/// That last one is what lets a caller fill the map from a directory of files
/// without knowing which of them a given `.ksy` will ask for by path and which
/// by name alone.
pub struct MapImports(pub HashMap<String, String>);

impl MapImports {
	pub fn new() -> Self {
		MapImports(HashMap::new())
	}

	pub fn with(mut self, name: &str, text: &str) -> Self {
		self.0.insert(name.to_string(), text.to_string());
		self
	}
}

impl Default for MapImports {
	fn default() -> Self {
		MapImports::new()
	}
}

impl Imports for MapImports {
	fn load(&self, name: &str) -> Option<String> {
		if let Some(text) = self.0.get(name) {
			return Some(text.clone());
		}
		let trimmed = name.trim_start_matches('/');
		if let Some(text) = self.0.get(trimmed) {
			return Some(text.clone());
		}
		let base = trimmed.rsplit('/').next().unwrap_or(trimmed);
		if let Some(text) = self.0.get(base) {
			return Some(text.clone());
		}
		self.0
			.iter()
			.find(|(key, _)| key.rsplit('/').next().unwrap_or(key) == base)
			.map(|(_, text)| text.clone())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_name_is_found_by_path_or_by_its_last_part() {
		let imports = MapImports::new().with("common/bcd", "meta:\n  id: bcd\n");
		assert!(imports.load("common/bcd").is_some());
		assert!(imports.load("/common/bcd").is_some());
		assert!(imports.load("bcd").is_some());
		assert!(imports.load("other").is_none());
		assert!(NoImports.load("bcd").is_none());
	}
}
