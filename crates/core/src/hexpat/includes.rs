//! Where an `#include <std/mem.pat>` or an `import type.magic;` finds its text.
//!
//! The upstream include files are GPL-2.0 and are never bundled, so the
//! converter has to get through a pattern that asks for them without having
//! them. Two things make that work. The caller may hand over text it was given
//! -- a panel fills a map from files the user dropped -- and that is the
//! [`Includes`] trait. Whatever the caller cannot supply falls back on the
//! table in [`builtin`], which is a set of *declarations only*: the names, the
//! template parameters and their kinds, and a body just real enough to parse.
//!
//! Declarations are all the parser needs. It is type aware -- a name in type
//! position must already be declared, and whether a `<...>` argument is read as
//! a type or as a value depends on the declared parameter's kind -- and it
//! never looks a function call up. Nothing here is copied from upstream: what a
//! declaration says is the interface, and the stubs are written out fresh.
//!
//! The bodies below are never lowered. [`super::types`] recognises every one of
//! these names before the lowering ever reaches a declaration, so a run with the
//! real include files supplied through a map lowers `type::Magic` to a magic
//! and a run without them lowers it the same way.

use std::collections::HashMap;

use super::parser::{Resolved, Resolver};

/// What an `#include` or an `import` path resolves to.
///
/// The path arrives the way the parser passes it on: an `import`'s dots have
/// already become slashes and an `#include`'s brackets are stripped, so both
/// `std/mem.pat` and `std/mem` reach here for the same file.
pub trait Includes {
	/// The text of the file at `path`, or nothing if the caller has not got it.
	fn load(&self, path: &str) -> Option<String>;
}

/// A resolver with nothing in it. Everything falls back on [`builtin`].
pub struct NoFiles;

impl Includes for NoFiles {
	fn load(&self, _path: &str) -> Option<String> {
		None
	}
}

/// A resolver over a map from include path to text.
///
/// The path is looked up as written, then without its extension, then by its
/// last two components, which is what lets a caller fill the map from a
/// directory of files without knowing which spelling a pattern will use.
pub struct MapIncludes(pub HashMap<String, String>);

impl MapIncludes {
	pub fn new() -> Self {
		MapIncludes(HashMap::new())
	}

	pub fn with(mut self, path: &str, text: &str) -> Self {
		self.0.insert(path.to_string(), text.to_string());
		self
	}
}

impl Default for MapIncludes {
	fn default() -> Self {
		MapIncludes::new()
	}
}

impl Includes for MapIncludes {
	fn load(&self, path: &str) -> Option<String> {
		let path = path.replace('\\', "/");
		if let Some(text) = self.0.get(&path) {
			return Some(text.clone());
		}
		let key = strip_extension(&path);
		self.0
			.iter()
			.find(|(name, _)| strip_extension(&name.replace('\\', "/")) == key)
			.map(|(_, text)| text.clone())
	}
}

/// The path without a trailing `.pat` or `.hexpat`, which is the form the two
/// spellings of an include agree on.
fn strip_extension(path: &str) -> String {
	for extension in [".hexpat", ".pat"] {
		if let Some(stem) = path.strip_suffix(extension) {
			return stem.to_string();
		}
	}
	path.to_string()
}

/// The [`Resolver`] the parser wants, over an [`Includes`] and the built-in
/// table behind it.
pub(crate) struct IncludeResolver<'a> {
	pub includes: &'a dyn Includes,
}

impl Resolver for IncludeResolver<'_> {
	fn resolve(&self, path: &str) -> Option<Resolved> {
		if let Some(text) = self.includes.load(path) {
			return Some(Resolved { name: path.to_string(), text });
		}
		let key = strip_extension(&path.replace('\\', "/"));
		builtin(&key).map(|text| Resolved { name: format!("<built-in>/{key}.pat"), text: text.to_string() })
	}
}

/// The declarations the converter knows without the upstream include files,
/// by path with no extension.
///
/// Only the files the corpus asks for in type position are here. A pattern
/// including something else, `std/hash.pat` say, is told the file could not be
/// found and names it, which is the honest answer: nothing here knows what is
/// in it.
pub fn builtin(path: &str) -> Option<&'static str> {
	Some(match path {
		"std/mem" => STD_MEM,
		"std/core" => STD_CORE,
		"std/string" => STD_STRING,
		"std/time" => STD_TIME,
		"std/io" | "std/math" | "std/sys" | "std/limits" | "std/ctype" | "std/bit" | "std/array"
		| "std/file" | "std/random" | "std/hash" | "std/fxpt" | "std/attrs" => STD_EMPTY,
		"std/ptr" => STD_PTR,
		"type/magic" => TYPE_MAGIC,
		"type/guid" => TYPE_GUID,
		"type/time" => TYPE_TIME,
		"type/size" => TYPE_SIZE,
		"type/base" => TYPE_BASE,
		"type/leb128" => TYPE_LEB128,
		"type/float16" => TYPE_FLOAT16,
		"type/byte" => TYPE_BYTE,
		"type/color" => TYPE_COLOR,
		"type/types/c" | "type/types/win32" | "type/types/linux" | "type/types/rust"
		| "type/types/010" => TYPE_TYPES,
		_ => return None,
	})
}

/// Every path the table answers for, for a caller listing what it does not have
/// to supply.
pub const BUILTIN_PATHS: &[&str] = &[
	"std/array", "std/attrs", "std/bit", "std/core", "std/ctype", "std/file", "std/fxpt",
	"std/hash", "std/io", "std/limits", "std/math", "std/mem", "std/ptr", "std/random",
	"std/string", "std/sys", "std/time", "type/base", "type/byte", "type/color", "type/float16",
	"type/guid", "type/leb128", "type/magic", "type/size", "type/time", "type/types/010",
	"type/types/c", "type/types/linux", "type/types/rust", "type/types/win32",
];

/// A file that declares nothing but functions, which the parser never checks.
const STD_EMPTY: &str = "#pragma once\n";

const STD_MEM: &str = r#"#pragma once
namespace auto std {
	namespace mem {
		using Section = u128;
		enum Endian : u8 { Native, Big, Little };
		struct AlignTo<auto Alignment> { };
		struct Bytes<auto Size> { u8 data[Size]; };
		struct MagicSearch<auto Magic, T> { T data; };
	}
}
"#;

const STD_CORE: &str = r#"#pragma once
namespace auto std {
	namespace core {
		enum BitfieldOrder : u8 { LeastToMostSignificant, MostToLeastSignificant };
	}
}
"#;

const STD_STRING: &str = r#"#pragma once
namespace auto std {
	namespace string {
		struct SizedStringBase<SizeType, DataType> {
			SizeType size;
			DataType value[size];
		};
		using SizedString<SizeType> = SizedStringBase<SizeType, char>;
		using SizedString16<SizeType> = SizedStringBase<SizeType, char16>;
		struct NullStringBase<DataType> { DataType value[]; };
		using NullString = NullStringBase<char>;
		using NullString16 = NullStringBase<char16>;
	}
}
"#;

const STD_TIME: &str = r#"#pragma once
namespace auto std {
	namespace time {
		using EpochTime = u32;
		enum TimeZone : u8 { Local, UTC };
		struct Time {
			u8 sec;
			u8 min;
			u8 hour;
			u8 mday;
			u8 mon;
			u16 year;
			u8 wday;
			u16 yday;
		};
		bitfield DOSDate {
			day : 5;
			month : 4;
			year : 7;
		};
		bitfield DOSTime {
			second : 5;
			minute : 6;
			hour : 5;
		};
	}
}
"#;

const STD_PTR: &str = r#"#pragma once
namespace auto std {
	namespace ptr {
		struct NullablePtr<T, PointerType> { PointerType offset; };
	}
}
"#;

const TYPE_MAGIC: &str = r#"#pragma once
namespace auto type {
	struct Magic<auto ExpectedValue> { };
}
"#;

const TYPE_GUID: &str = r#"#pragma once
namespace auto type {
	struct GUID {
		u32 time_low;
		u16 time_mid;
		u16 time_high_and_version;
		u8 clock_seq_and_reserved;
		u8 clock_seq_low;
		u8 node[6];
	};
	using UUID = GUID;
}
"#;

const TYPE_TIME: &str = r#"#pragma once
namespace auto type {
	using time32_t = u32;
	using time_t = time32_t;
	using time64_t = u64;
	using DOSDate = u16;
	using DOSTime = u16;
	using FILETIME = u64;
}
"#;

const TYPE_SIZE: &str = r#"#pragma once
namespace auto type {
	using Size<T> = T;
	using Size8 = Size<u8>;
	using Size16 = Size<u16>;
	using Size32 = Size<u32>;
	using Size64 = Size<u64>;
	using Size128 = Size<u128>;
}
"#;

const TYPE_BASE: &str = r#"#pragma once
namespace auto type {
	using Hex<T> = T;
	using Oct<T> = T;
	using Dec<T> = T;
	using Bin<T> = T;
}
"#;

const TYPE_LEB128: &str = r#"#pragma once
namespace auto type {
	struct LEB128Base { };
	using uLEB128 = LEB128Base;
	using sLEB128 = LEB128Base;
	using LEB128 = uLEB128;
}
"#;

const TYPE_FLOAT16: &str = r#"#pragma once
namespace auto type {
	using float16 = u16;
}
"#;

const TYPE_BYTE: &str = r#"#pragma once
namespace auto type {
	bitfield Nibbles {
		low : 4;
		high : 4;
	};
	union Byte {
		u8 value;
		Nibbles nibbles;
	};
}
"#;

const TYPE_COLOR: &str = r#"#pragma once
namespace auto type {
	bitfield RGBA<auto R, auto G, auto B, auto A> {
		r : R;
		g : G;
		b : B;
		a : A;
	};
	using RGB<auto R, auto G, auto B> = RGBA<R, G, B, 0>;
	using RGBA8 = RGBA<8, 8, 8, 8>;
	using RGB8 = RGB<8, 8, 8>;
	using RGB565 = RGB<5, 6, 5>;
	using RGB4444 = RGBA<4, 4, 4, 4>;
	using RGBA5551 = RGBA<5, 5, 5, 1>;
}
"#;

/// The C, Win32, Linux, Rust and 010 Editor spelling files, which are nothing
/// but aliases for the built-in scalars. Enough of each that the patterns using
/// them parse; a name that is not here is reported as undeclared, at its line.
const TYPE_TYPES: &str = r#"#pragma once
namespace auto type {
	using BYTE = u8;
	using WORD = u16;
	using DWORD = u32;
	using QWORD = u64;
	using CHAR = char;
	using UCHAR = u8;
	using SHORT = s16;
	using USHORT = u16;
	using INT = s32;
	using UINT = u32;
	using LONG = s32;
	using ULONG = u32;
	using LONGLONG = s64;
	using ULONGLONG = u64;
	using FLOAT = float;
	using DOUBLE = double;
	using BOOLEAN = bool;
	using int8_t = s8;
	using int16_t = s16;
	using int32_t = s32;
	using int64_t = s64;
	using uint8_t = u8;
	using uint16_t = u16;
	using uint32_t = u32;
	using uint64_t = u64;
	using size_t = u64;
	using ssize_t = s64;
	using i8 = s8;
	using i16 = s16;
	using i32 = s32;
	using i64 = s64;
	using f32 = float;
	using f64 = double;
	using usize = u64;
	using isize = s64;
	using ubyte = u8;
	using ushort = u16;
	using uint = u32;
	using uquad = u64;
	using byte = s8;
	using quad = s64;
	using int64 = s64;
	using uint64 = u64;
}
"#;

#[cfg(test)]
mod tests {
	use super::*;
	use crate::hexpat::parse_with;

	#[test]
	fn a_path_is_found_with_or_without_its_extension() {
		let includes = MapIncludes::new().with("std/mem.pat", "#pragma once\n");
		assert!(includes.load("std/mem.pat").is_some());
		assert!(includes.load("std/mem").is_some());
		assert!(includes.load("std/other").is_none());
		assert!(NoFiles.load("std/mem").is_none());
	}

	#[test]
	fn every_built_in_file_parses_on_its_own() {
		let resolver = IncludeResolver { includes: &NoFiles };
		for path in BUILTIN_PATHS {
			let text = builtin(path).unwrap_or_else(|| panic!("no built-in {path}"));
			parse_with(path, text, &resolver, &[]).unwrap_or_else(|e| panic!("{path}: {e}"));
		}
	}

	#[test]
	fn a_pattern_reaches_the_built_in_names_without_the_upstream_files() {
		let resolver = IncludeResolver { includes: &NoFiles };
		let text = "#include <type/magic.pat>\n#include <type/guid.pat>\n\
			type::Magic<\"PK\"> magic @ 0x00;\ntype::GUID id @ 0x02;\n";
		parse_with("t.hexpat", text, &resolver, &[]).expect("the built-in table covers this");
	}

	#[test]
	fn what_the_caller_supplies_wins_over_the_table() {
		let includes = MapIncludes::new()
			.with("type/magic.pat", "#pragma once\nnamespace auto type { using Magic<auto V> = u8; }\n");
		let resolver = IncludeResolver { includes: &includes };
		let found = resolver.resolve("type/magic.pat").expect("found");
		assert!(found.text.contains("using Magic"));
	}
}
