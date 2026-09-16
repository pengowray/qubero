//! The `type::`, `std::` and `hex::` names the converter understands, by name.
//!
//! The upstream include files that declare these are GPL-2.0 and are never
//! bundled, and even where a caller supplies them their bodies are not what the
//! IR should say: `type::Magic<"PK">` is written as a `char` array and a
//! `std::assert`, and the IR has a magic; `type::time32_t` is a `u32` with a
//! display function on it, and the IR has a clock. So a name from these
//! namespaces is recognised here *before* the lowering looks at any
//! declaration, and the answer is the same whether the include files were
//! supplied or the stubs in [`super::includes`] stood in for them.
//!
//! A name that is not here is lowered from whatever declaration the parser
//! found, which for a stub is a shape with the right size and nothing else, so
//! anything from these namespaces that is not in the table below is a gap
//! instead. Saying "we do not know what this is" is the whole point.

use crate::template::{Endian, Expr, StrLen, Ty};
use crate::template::{Encoding, Time};

/// What one of the known names is, once the lowering has its arguments.
#[derive(Debug, Clone)]
pub(crate) enum Known {
	/// An IR type that needs nothing from the use site.
	Fixed(Ty),
	/// The same, and a reader would not guess how it was said.
	Noted(Ty, &'static str),
	/// A number that holds a moment.
	Moment(Ty, Time),
	/// A packed MS-DOS date, whose time of day is a field of its own. Paired
	/// with [`Known::DosTime`] when both are declared in one structure.
	DosDate,
	/// A packed MS-DOS time of day. See [`Known::DosDate`].
	DosTime,
	/// `type::Magic<"ABCD">`: the bytes of the one string argument.
	Magic,
	/// The one *type* argument, lowered unchanged, and a note saying what the
	/// reference would have displayed instead.
	Passthrough(&'static str),
	/// `std::mem::Bytes<N>`: as many bytes as the one value argument says.
	MemBytes,
	/// A length of the given type and then that many characters.
	SizedString(Encoding),
	/// Characters to the first NUL.
	NullString(Encoding),
	/// Known, and not something the IR can say.
	Gap(&'static str),
}

/// What `path` means, for a name in one of the three namespaces the converter
/// knows. `None` for every other name, which is lowered from its declaration.
///
/// `endian` is the file's `#pragma endian`, which settles the byte order of
/// every multi-byte scalar one of these stands for.
pub(crate) fn known(path: &str, endian: Endian) -> Option<Known> {
	if !(path.starts_with("type::") || path.starts_with("std::") || path.starts_with("hex::")) {
		return None;
	}
	Some(match path {
		// -------- type/magic.pat --------
		"type::Magic" => Known::Magic,

		// -------- type/guid.pat --------
		"type::GUID" | "type::UUID" => Known::Fixed(guid(endian)),

		// -------- type/time.pat, std/time.pat --------
		"type::time32_t" | "type::time_t" | "std::time::EpochTime" => {
			Known::Moment(Ty::UInt { bits: 32, endian }, Time::unix())
		}
		"type::time64_t" => Known::Moment(Ty::UInt { bits: 64, endian }, Time::unix()),
		"type::FILETIME" => Known::Moment(Ty::UInt { bits: 64, endian }, Time::filetime()),
		"type::DOSDate" => Known::DosDate,
		"type::DOSTime" => Known::DosTime,
		"std::time::Time" => Known::Gap(
			"std::time::Time is a structure the include file fills in from a call, not bytes of the file",
		),
		"std::time::DOSDate" | "std::time::DOSTime" => Known::Gap(
			"std::time's packed date and time are read through a function rather than declared as a field",
		),

		// -------- type/size.pat --------
		"type::Size" => Known::Passthrough("the reference shows this number as a size in bytes"),
		"type::Size8" => Known::Noted(Ty::u8(), SIZE_NOTE),
		"type::Size16" => Known::Noted(Ty::UInt { bits: 16, endian }, SIZE_NOTE),
		"type::Size32" => Known::Noted(Ty::UInt { bits: 32, endian }, SIZE_NOTE),
		"type::Size64" => Known::Noted(Ty::UInt { bits: 64, endian }, SIZE_NOTE),
		"type::Size128" => Known::Noted(Ty::UInt { bits: 128, endian }, SIZE_NOTE),

		// -------- type/base.pat --------
		"type::Hex" => Known::Passthrough("the reference shows this number in hexadecimal"),
		"type::Oct" => Known::Passthrough("the reference shows this number in octal"),
		"type::Dec" => Known::Passthrough("the reference shows this number in decimal"),
		"type::Bin" => Known::Passthrough("the reference shows this number in binary"),

		// -------- type/leb128.pat --------
		"type::uLEB128" | "type::LEB128" => Known::Fixed(Ty::leb_u()),
		"type::sLEB128" => Known::Fixed(Ty::zigzag()),

		// -------- type/float16.pat --------
		"type::float16" => Known::Fixed(Ty::F16(endian)),

		// -------- type/byte.pat --------
		"type::Nibbles" => Known::Fixed(Ty::structure(
			"Nibbles",
			vec![("low", Ty::UInt { bits: 4, endian }), ("high", Ty::UInt { bits: 4, endian })],
		)),

		// -------- type/color.pat --------
		"type::RGBA8" => Known::Fixed(rgba(&[("r", 8), ("g", 8), ("b", 8), ("a", 8)], endian)),
		"type::RGB8" => Known::Fixed(rgba(&[("r", 8), ("g", 8), ("b", 8)], endian)),

		// -------- std/mem.pat --------
		"std::mem::Bytes" => Known::MemBytes,
		"std::mem::Section" => Known::Gap("a section is memory the pattern creates, not bytes of the file"),
		"std::mem::AlignTo" => Known::Gap(
			"std::mem::AlignTo pads to an alignment worked out while the pattern runs",
		),
		"std::mem::MagicSearch" => Known::Gap("std::mem::MagicSearch scans the file for its type"),

		// -------- std/string.pat --------
		"std::string::NullString" => Known::NullString(Encoding::Ascii),
		"std::string::NullString16" => Known::NullString(match endian {
			Endian::Little => Encoding::Utf16(Endian::Little),
			Endian::Big => Encoding::Utf16(Endian::Big),
		}),
		"std::string::SizedString" => Known::SizedString(Encoding::Ascii),
		"std::string::SizedString16" => Known::SizedString(match endian {
			Endian::Little => Encoding::Utf16(Endian::Little),
			Endian::Big => Encoding::Utf16(Endian::Big),
		}),

		// -------- std/ptr.pat --------
		"std::ptr::NullablePtr" => Known::Gap(
			"std::ptr::NullablePtr places its target through a function the converter does not run",
		),

		// -------- hex/ --------
		_ if path.starts_with("hex::") => Known::Gap(
			"a hex:: type is read by an ImHex plugin, not by the pattern",
		),
		_ => Known::Gap("the converter does not know this name from the standard library"),
	})
}

const SIZE_NOTE: &str = "the reference shows this number as a size in bytes";

/// The structure `type/guid.pat` declares, with the line that prints a GUID the
/// way one is written.
fn guid(endian: Endian) -> Ty {
	Ty::structure(
		"GUID",
		vec![
			("time_low", Ty::UInt { bits: 32, endian }),
			("time_mid", Ty::UInt { bits: 16, endian }),
			("time_high_and_version", Ty::UInt { bits: 16, endian }),
			("clock_seq_and_reserved", Ty::u8()),
			("clock_seq_low", Ty::u8()),
			("node", Ty::array(Ty::u8(), Expr::lit(6))),
		],
	)
	.reads_as(&[
		("", "time_low", "-"),
		("", "time_mid", "-"),
		("", "time_high_and_version", "-"),
		("", "clock_seq_and_reserved", ""),
		("", "clock_seq_low", "-"),
		("", "node", ""),
	])
}

fn rgba(parts: &[(&str, u32)], endian: Endian) -> Ty {
	let fields: Vec<(&str, Ty)> =
		parts.iter().map(|(name, bits)| (*name, Ty::UInt { bits: *bits, endian })).collect();
	Ty::structure("RGBA", fields)
}

/// The text of a `std::string::SizedString<T>`, given the lowered length type.
pub(crate) fn sized_string(len: Ty, enc: Encoding) -> Ty {
	Ty::structure("SizedString", vec![("size", len), ("value", Ty::text(StrLen::Fixed(Expr::field("size")), enc))])
}

/// The text of a `std::string::NullString`.
pub(crate) fn null_string(enc: Encoding) -> Ty {
	Ty::text(StrLen::Terminated { end: 0, or_end: false }, enc)
}
