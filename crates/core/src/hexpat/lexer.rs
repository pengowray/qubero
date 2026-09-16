//! `.hexpat` text in, a token stream out, with the preprocessor already run.
//!
//! The token set is the reference implementation's (`lib/source/pl/core/lexer.cpp`
//! and `lib/include/pl/core/tokens.hpp`), because the parser's decisions are
//! made on it: keywords beat identifiers, `padding`, `auto` and `any` are types
//! rather than names, `addressof`, `sizeof` and `typenameof` are operators, and
//! `true`, `false`, `nan` and `inf` are literals, so a field called `inf` is a
//! syntax error here exactly as it is there.
//!
//! Two-character operators stop at `::`, `==`, `!=`, `&&`, `||` and `^^`. The
//! comparison and shift pairs (`<=`, `>=`, `<<`, `>>`) are deliberately *not*
//! lexed: the parser composes them from two `<`/`>` tokens so that
//! `Foo<Bar<u8>>` closes two template lists rather than shifting right.
//!
//! The preprocessor runs here too, as it does there: `#define` substitutes
//! token lists (the substituted tokens keep the position they had at the
//! `#define`), `#ifdef`/`#ifndef` drop the inactive branch line by line,
//! `#pragma` keeps a key and the rest of its line as raw text, and `#include`
//! is collected for the caller to resolve. `import` is a keyword, not a
//! directive, so it survives into the token stream for the parser.

use std::collections::HashMap;
use std::fmt;

/// Where something is, in the one file being lexed.
///
/// `line` and `col` are one-based, as an editor counts them; `byte` is the
/// zero-based offset into the source, which is what a [`Statement`] span is
/// sliced with.
///
/// [`Statement`]: crate::hexpat::ast::Statement
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Pos {
	pub line: u32,
	pub col: u32,
	pub byte: u32,
}

impl Pos {
	pub fn new(line: u32, col: u32, byte: u32) -> Self {
		Pos { line, col, byte }
	}
}

impl fmt::Display for Pos {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}:{}", self.line, self.col)
	}
}

/// Anything wrong with a `.hexpat`, at the position the reference reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HexpatError {
	/// The file the position is in. Empty for a string parsed without a name.
	pub file: String,
	pub pos: Pos,
	pub message: String,
}

impl HexpatError {
	pub fn new(file: impl Into<String>, pos: Pos, message: impl Into<String>) -> Self {
		HexpatError { file: file.into(), pos, message: message.into() }
	}
}

impl fmt::Display for HexpatError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		if self.file.is_empty() {
			write!(f, "{}: {}", self.pos, self.message)
		} else {
			write!(f, "{}:{}: {}", self.file, self.pos, self.message)
		}
	}
}

impl std::error::Error for HexpatError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Keyword {
	If,
	Else,
	While,
	For,
	Match,
	Return,
	Break,
	Continue,
	Struct,
	Enum,
	Union,
	Function,
	Bitfield,
	Unsigned,
	Signed,
	LittleEndian,
	BigEndian,
	Parent,
	Namespace,
	Using,
	This,
	In,
	Out,
	Reference,
	Null,
	Const,
	Underscore,
	Try,
	Catch,
	Import,
	As,
	Is,
	From,
}

impl Keyword {
	pub fn name(self) -> &'static str {
		use Keyword::*;
		match self {
			If => "if",
			Else => "else",
			While => "while",
			For => "for",
			Match => "match",
			Return => "return",
			Break => "break",
			Continue => "continue",
			Struct => "struct",
			Enum => "enum",
			Union => "union",
			Function => "fn",
			Bitfield => "bitfield",
			Unsigned => "unsigned",
			Signed => "signed",
			LittleEndian => "le",
			BigEndian => "be",
			Parent => "parent",
			Namespace => "namespace",
			Using => "using",
			This => "this",
			In => "in",
			Out => "out",
			Reference => "ref",
			Null => "null",
			Const => "const",
			Underscore => "_",
			Try => "try",
			Catch => "catch",
			Import => "import",
			As => "as",
			Is => "is",
			From => "from",
		}
	}
}

/// A built-in type name, plus the three that name a shape rather than a type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValueType {
	Padding,
	Auto,
	Any,
	U8,
	U16,
	U24,
	U32,
	U48,
	U64,
	U96,
	U128,
	S8,
	S16,
	S24,
	S32,
	S48,
	S64,
	S96,
	S128,
	Float,
	Double,
	Bool,
	Char,
	Char16,
	Str,
}

impl ValueType {
	pub fn name(self) -> &'static str {
		use ValueType::*;
		match self {
			Padding => "padding",
			Auto => "auto",
			Any => "any",
			U8 => "u8",
			U16 => "u16",
			U24 => "u24",
			U32 => "u32",
			U48 => "u48",
			U64 => "u64",
			U96 => "u96",
			U128 => "u128",
			S8 => "s8",
			S16 => "s16",
			S24 => "s24",
			S32 => "s32",
			S48 => "s48",
			S64 => "s64",
			S96 => "s96",
			S128 => "s128",
			Float => "float",
			Double => "double",
			Bool => "bool",
			Char => "char",
			Char16 => "char16",
			Str => "str",
		}
	}

	/// Size in bits, or `None` for the ones that have no fixed size.
	pub fn bits(self) -> Option<u32> {
		use ValueType::*;
		Some(match self {
			U8 | S8 | Bool | Char => 8,
			U16 | S16 | Char16 => 16,
			U24 | S24 => 24,
			U32 | S32 | Float => 32,
			U48 | S48 => 48,
			U64 | S64 | Double => 64,
			U96 | S96 => 96,
			U128 | S128 => 128,
			Padding | Auto | Any | Str => return None,
		})
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Op {
	Plus,
	Minus,
	Star,
	Slash,
	Percent,
	BitAnd,
	BitOr,
	BitXor,
	BitNot,
	BoolEqual,
	BoolNotEqual,
	Less,
	Greater,
	BoolAnd,
	BoolOr,
	BoolNot,
	BoolXor,
	Dollar,
	Colon,
	ScopeResolution,
	Ternary,
	At,
	Assign,
	AddressOf,
	SizeOf,
	TypeNameOf,
}

impl Op {
	pub fn name(self) -> &'static str {
		use Op::*;
		match self {
			Plus => "+",
			Minus => "-",
			Star => "*",
			Slash => "/",
			Percent => "%",
			BitAnd => "&",
			BitOr => "|",
			BitXor => "^",
			BitNot => "~",
			BoolEqual => "==",
			BoolNotEqual => "!=",
			Less => "<",
			Greater => ">",
			BoolAnd => "&&",
			BoolOr => "||",
			BoolNot => "!",
			BoolXor => "^^",
			Dollar => "$",
			Colon => ":",
			ScopeResolution => "::",
			Ternary => "?",
			At => "@",
			Assign => "=",
			AddressOf => "addressof",
			SizeOf => "sizeof",
			TypeNameOf => "typenameof",
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sep {
	LeftParen,
	RightParen,
	LeftBrace,
	RightBrace,
	LeftBracket,
	RightBracket,
	Comma,
	Dot,
	Semicolon,
}

impl Sep {
	pub fn name(self) -> &'static str {
		use Sep::*;
		match self {
			LeftParen => "(",
			RightParen => ")",
			LeftBrace => "{",
			RightBrace => "}",
			LeftBracket => "[",
			RightBracket => "]",
			Comma => ",",
			Dot => ".",
			Semicolon => ";",
		}
	}
}

/// A literal as the lexer read it.
///
/// `nan` and `inf` are names in the source but arrive here as floats, and a
/// character literal arrives as an integer, both as the reference does it.
#[derive(Debug, Clone, PartialEq)]
pub enum Lit {
	/// An integer literal. `unsigned` is set by a `u`/`U` suffix.
	Int { value: u128, unsigned: bool },
	Float(f64),
	Bool(bool),
	Str(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
	Ident(String),
	Lit(Lit),
	Keyword(Keyword),
	Type(ValueType),
	Op(Op),
	Sep(Sep),
	/// The one token the parser stops at, never consumed.
	End,
}

impl Tok {
	/// How an error message names this token, in the reference's wording.
	pub fn describe(&self) -> String {
		match self {
			Tok::Ident(name) => format!("identifier '{name}'"),
			Tok::Lit(Lit::Int { value, .. }) => format!("integer literal {value}"),
			Tok::Lit(Lit::Float(value)) => format!("float literal {value}"),
			Tok::Lit(Lit::Bool(value)) => format!("'{value}'"),
			Tok::Lit(Lit::Str(text)) => format!("string literal \"{text}\""),
			Tok::Keyword(keyword) => format!("keyword '{}'", keyword.name()),
			Tok::Type(ty) => format!("type '{}'", ty.name()),
			Tok::Op(op) => format!("'{}'", op.name()),
			Tok::Sep(sep) => format!("'{}'", sep.name()),
			Tok::End => "end of program".to_string(),
		}
	}
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
	pub tok: Tok,
	pub pos: Pos,
}

/// One `#pragma key rest-of-line`, with the value left as the text it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pragma {
	pub key: String,
	pub value: String,
	pub pos: Pos,
}

/// One `#include <path>` or `#include "path"`, with the brackets stripped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Include {
	pub path: String,
	pub pos: Pos,
}

/// A `///`, `/** */` or `/*! */` comment, and the token it sits in front of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocComment {
	pub text: String,
	/// Written `/*!`, which documents the file rather than the next statement.
	pub global: bool,
	pub pos: Pos,
	/// Index into the token stream of the first token after this comment.
	pub before: usize,
}

/// What the lexer and the preprocessor between them produced.
#[derive(Debug, Clone, PartialEq)]
pub struct Lexed {
	pub tokens: Vec<Token>,
	pub pragmas: Vec<Pragma>,
	pub includes: Vec<Include>,
	pub docs: Vec<DocComment>,
}

/// One `#define`: the tokens it stands for, kept with their own positions.
#[derive(Debug, Clone, PartialEq)]
struct Define {
	tokens: Vec<Token>,
}

/// Read `text` into tokens, running the preprocessor over it.
///
/// `defines` are the macros defined before the first line, which is how the
/// host tells a pattern which of `__IMHEX__` and `__PL_UNIT_TESTS__` it is
/// being read by.
pub fn lex(file: &str, text: &str, defines: &[String]) -> Result<Lexed, HexpatError> {
	let mut lexer = Lexer::new(file, text);
	lexer.run()?;
	let raw = std::mem::take(&mut lexer.raw);
	let mut pre = Preprocessor::new(file, raw, defines);
	pre.run()?;
	Ok(Lexed { tokens: pre.out, pragmas: pre.pragmas, includes: pre.includes, docs: pre.docs })
}

/// A token as the lexer makes it, before the preprocessor has had it.
#[derive(Debug, Clone, PartialEq)]
enum Raw {
	Token(Token),
	Directive { name: Directive, pos: Pos },
	/// The word after a directive, or the rest of its line.
	Text { text: String, pos: Pos },
	Doc { text: String, global: bool, pos: Pos },
	Comment { pos: Pos },
}

impl Raw {
	fn pos(&self) -> Pos {
		match self {
			Raw::Token(token) => token.pos,
			Raw::Directive { pos, .. } => *pos,
			Raw::Text { pos, .. } => *pos,
			Raw::Doc { pos, .. } => *pos,
			Raw::Comment { pos } => *pos,
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Directive {
	Include,
	Define,
	Undef,
	IfDef,
	IfNDef,
	EndIf,
	Error,
	Pragma,
}

impl Directive {
	fn from_name(name: &str) -> Option<Directive> {
		Some(match name {
			"#include" => Directive::Include,
			"#define" => Directive::Define,
			"#undef" => Directive::Undef,
			"#ifdef" => Directive::IfDef,
			"#ifndef" => Directive::IfNDef,
			"#endif" => Directive::EndIf,
			"#error" => Directive::Error,
			"#pragma" => Directive::Pragma,
			_ => return None,
		})
	}
}

fn keyword(name: &str) -> Option<Keyword> {
	use Keyword::*;
	Some(match name {
		"if" => If,
		"else" => Else,
		"while" => While,
		"for" => For,
		"match" => Match,
		"return" => Return,
		"break" => Break,
		"continue" => Continue,
		"struct" => Struct,
		"enum" => Enum,
		"union" => Union,
		"fn" => Function,
		"bitfield" => Bitfield,
		"unsigned" => Unsigned,
		"signed" => Signed,
		"le" => LittleEndian,
		"be" => BigEndian,
		"parent" => Parent,
		"namespace" => Namespace,
		"using" => Using,
		"this" => This,
		"in" => In,
		"out" => Out,
		"ref" => Reference,
		"null" => Null,
		"const" => Const,
		"_" => Underscore,
		"try" => Try,
		"catch" => Catch,
		"import" => Import,
		"as" => As,
		"is" => Is,
		"from" => From,
		_ => return None,
	})
}

fn named_operator(name: &str) -> Option<Op> {
	Some(match name {
		"addressof" => Op::AddressOf,
		"sizeof" => Op::SizeOf,
		"typenameof" => Op::TypeNameOf,
		_ => return None,
	})
}

fn value_type(name: &str) -> Option<ValueType> {
	use ValueType::*;
	Some(match name {
		"padding" => Padding,
		"auto" => Auto,
		"any" => Any,
		"u8" => U8,
		"u16" => U16,
		"u24" => U24,
		"u32" => U32,
		"u48" => U48,
		"u64" => U64,
		"u96" => U96,
		"u128" => U128,
		"s8" => S8,
		"s16" => S16,
		"s24" => S24,
		"s32" => S32,
		"s48" => S48,
		"s64" => S64,
		"s96" => S96,
		"s128" => S128,
		"float" => Float,
		"double" => Double,
		"bool" => Bool,
		"char" => Char,
		"char16" => Char16,
		"str" => Str,
		_ => return None,
	})
}

fn constant(name: &str) -> Option<Lit> {
	Some(match name {
		"true" => Lit::Bool(true),
		"false" => Lit::Bool(false),
		"nan" => Lit::Float(f64::NAN),
		"inf" => Lit::Float(f64::INFINITY),
		_ => return None,
	})
}

fn is_identifier_char(c: u8) -> bool {
	c.is_ascii_alphanumeric() || c == b'_'
}

/// A byte that may sit inside a numeric literal, as `getIntegerLiteralLength`
/// counts them. `e` and `E` are not in the set, which is why the reference has
/// no exponent notation; the set is copied rather than corrected so that a
/// pattern rejected there is rejected here.
fn is_literal_char(c: u8) -> bool {
	c.is_ascii_digit()
		|| matches!(c, b'A'..=b'F' | b'a'..=b'f' | b'\'' | b'x' | b'X' | b'o' | b'O' | b'p' | b'P' | b'.' | b'u' | b'U' | b'+' | b'-')
}

struct Lexer<'a> {
	file: &'a str,
	src: &'a [u8],
	cursor: usize,
	line: u32,
	line_begin: usize,
	raw: Vec<Raw>,
}

impl<'a> Lexer<'a> {
	fn new(file: &'a str, text: &'a str) -> Self {
		Lexer { file, src: text.as_bytes(), cursor: 0, line: 1, line_begin: 0, raw: Vec::new() }
	}

	fn err(&self, pos: Pos, message: impl Into<String>) -> HexpatError {
		HexpatError::new(self.file, pos, message)
	}

	fn peek(&self, ahead: usize) -> u8 {
		*self.src.get(self.cursor + ahead).unwrap_or(&0)
	}

	fn pos(&self) -> Pos {
		let mut col = (self.cursor - self.line_begin) as u32;
		// There is no newline before the first line, so its columns start at 1.
		if self.line == 1 {
			col += 1;
		}
		Pos::new(self.line, col, self.cursor as u32)
	}

	/// Count a line ending, and say whether `c` was one.
	///
	/// Only `\n` counts, as the reference does it, which leaves a lone `\r`
	/// as ordinary whitespace and makes `\r\n` one line either way.
	fn line_ended(&mut self, c: u8) -> bool {
		if c == b'\n' {
			self.line += 1;
			self.line_begin = self.cursor;
			return true;
		}
		false
	}

	fn push(&mut self, tok: Tok, pos: Pos) {
		self.raw.push(Raw::Token(Token { tok, pos }));
	}

	fn run(&mut self) -> Result<(), HexpatError> {
		while self.cursor < self.src.len() {
			let c = self.src[self.cursor];
			if c == 0 {
				break;
			}

			if c.is_ascii_whitespace() {
				self.line_ended(c);
				self.cursor += 1;
				continue;
			}

			if is_identifier_char(c) && !c.is_ascii_digit() {
				let start = self.cursor;
				let mut length = 0;
				while is_identifier_char(self.peek(length)) {
					length += 1;
				}
				let word = std::str::from_utf8(&self.src[start..start + length]).unwrap_or("").to_string();
				let pos = self.pos();
				self.cursor += length;

				if let Some(word) = keyword(&word) {
					self.push(Tok::Keyword(word), pos);
				} else if let Some(op) = named_operator(&word) {
					self.push(Tok::Op(op), pos);
				} else if let Some(ty) = value_type(&word) {
					self.push(Tok::Type(ty), pos);
				} else if let Some(lit) = constant(&word) {
					self.push(Tok::Lit(lit), pos);
				} else {
					self.push(Tok::Ident(word), pos);
				}
				continue;
			}

			if c.is_ascii_digit() {
				let pos = self.pos();
				let size = self.numeric_length();
				let text = std::str::from_utf8(&self.src[self.cursor..self.cursor + size]).unwrap_or("").to_string();
				let lit = self.numeric_literal(&text, pos)?;
				self.push(Tok::Lit(lit), pos);
				self.cursor += size;
				continue;
			}

			if c == b'/' {
				let category = self.peek(1);
				let kind = self.peek(2);
				if category == b'/' {
					if kind == b'/' {
						self.one_line_comment(3, true);
					} else {
						self.one_line_comment(2, false);
					}
					continue;
				}
				if category == b'*' {
					let global = kind == b'!';
					let doc = kind == b'!' || (kind == b'*' && self.peek(3) != b'/');
					self.multi_line_comment(doc, global)?;
					continue;
				}
			}

			if let Some(op) = self.operator() {
				let pos = self.pos();
				self.cursor += op.name().len();
				self.push(Tok::Op(op), pos);
				continue;
			}

			if let Some(sep) = separator(c) {
				let pos = self.pos();
				self.cursor += 1;
				self.push(Tok::Sep(sep), pos);
				continue;
			}

			// A '#' only starts a directive when nothing else is on its line yet.
			if c == b'#' && self.raw.last().is_none_or(|last| last.pos().line < self.line) {
				let mut length = 1;
				while is_identifier_char(self.peek(length)) {
					length += 1;
				}
				let name = std::str::from_utf8(&self.src[self.cursor..self.cursor + length]).unwrap_or("").to_string();
				let pos = self.pos();
				let line = self.line;
				let Some(directive) = Directive::from_name(&name) else {
					return Err(self.err(pos, format!("Unknown directive: {name}")));
				};
				self.cursor += length;
				self.raw.push(Raw::Directive { name: directive, pos });

				// The token-valued directives take their arguments as ordinary
				// tokens; the rest take a word and then the rest of the line.
				// Each step stops if the line ran out, which is how `#pragma once`
				// gets a key and no value.
				if matches!(directive, Directive::Define | Directive::Undef | Directive::IfDef | Directive::IfNDef | Directive::EndIf) {
					continue;
				}
				if self.peek(0) == 0 {
					continue;
				}
				let ended = self.peek(0);
				if self.line_ended(ended) {
					self.cursor += 1;
					continue;
				}
				self.directive_value()?;
				if self.line != line || self.peek(0) == 0 {
					continue;
				}
				let ended = self.peek(0);
				if self.line_ended(ended) {
					self.cursor += 1;
					continue;
				}
				self.directive_argument()?;
				continue;
			}

			if c == b'"' {
				let pos = self.pos();
				let text = self.string_literal()?;
				self.push(Tok::Lit(Lit::Str(text)), pos);
				continue;
			}

			if c == b'\'' {
				let pos = self.pos();
				self.cursor += 1;
				let bytes = self.character()?;
				if bytes.len() != 1 {
					return Err(self.err(pos, "A char holds one byte. Use a string for this code point"));
				}
				if self.peek(0) != b'\'' {
					return Err(self.err(self.pos(), "Expected closing '"));
				}
				self.cursor += 1;
				self.push(Tok::Lit(Lit::Int { value: u128::from(bytes[0]), unsigned: false }), pos);
				continue;
			}

			return Err(self.err(self.pos(), format!("Unexpected character: {}", c as char)));
		}

		let pos = self.pos();
		self.push(Tok::End, pos);
		Ok(())
	}

	/// How many bytes of a numeric literal start here, `getIntegerLiteralLength`.
	fn numeric_length(&self) -> usize {
		let rest = &self.src[self.cursor..];
		let count = rest.iter().position(|&c| !is_literal_char(c)).unwrap_or(rest.len());
		let int_literal = &rest[..count];
		if let Some(sign) = int_literal.iter().position(|&c| c == b'+' || c == b'-') {
			let before = if sign == 0 { 0 } else { rest[sign - 1] };
			if (before != b'e' && before != b'E') || rest.starts_with(b"0x") {
				return sign;
			}
		}
		int_literal.len()
	}

	fn numeric_literal(&self, text: &str, pos: Pos) -> Result<Lit, HexpatError> {
		let float_suffix = text.ends_with(['f', 'F', 'd', 'D']);
		let unsigned_suffix = text.ends_with(['u', 'U']);
		let is_float = text.contains('.') || (!text.starts_with("0x") && float_suffix);

		if is_float {
			let body = if float_suffix { &text[..text.len() - 1] } else { text };
			return match body.parse::<f64>() {
				Ok(value) => Ok(Lit::Float(value)),
				Err(_) => Err(self.err(pos, format!("Invalid float literal: {text}"))),
			};
		}

		let body = if unsigned_suffix { &text[..text.len() - 1] } else { text };
		let value = self.integer(body, pos)?;
		Ok(Lit::Int { value, unsigned: unsigned_suffix })
	}

	fn integer(&self, literal: &str, pos: Pos) -> Result<u128, HexpatError> {
		let mut base = 10u128;
		let mut digits = literal;
		if literal.starts_with('0') {
			if literal.len() == 1 {
				return Ok(0);
			}
			let prefixed = match literal.as_bytes()[1] {
				b'x' | b'X' => {
					base = 16;
					true
				}
				b'o' | b'O' => {
					base = 8;
					true
				}
				b'b' | b'B' => {
					base = 2;
					true
				}
				_ => false,
			};
			if prefixed {
				digits = &literal[2..];
			}
		}

		let mut value: u128 = 0;
		for c in digits.bytes() {
			if c == b'\'' {
				continue;
			}
			let digit = match base {
				16 => (c as char).to_digit(16),
				10 => (c as char).to_digit(10),
				8 => (c as char).to_digit(8),
				2 => (c as char).to_digit(2),
				_ => None,
			};
			let Some(digit) = digit else {
				return Err(self.err(pos, format!("Invalid integer literal: {literal}")));
			};
			value = value.wrapping_mul(base).wrapping_add(u128::from(digit));
		}
		Ok(value)
	}

	/// One source character, with the escapes the reference understands.
	///
	/// `\x` gives one byte whatever the encoding; `\u` and `\U` give one code
	/// point, encoded as UTF-8, which is why this returns bytes rather than a
	/// byte.
	fn character(&mut self) -> Result<Vec<u8>, HexpatError> {
		if self.cursor >= self.src.len() {
			return Err(self.err(self.pos(), "Unexpected end of file"));
		}
		let c = self.src[self.cursor];
		self.cursor += 1;
		if c != b'\\' {
			return Ok(vec![c]);
		}
		let escape = self.peek(0);
		self.cursor += 1;
		let byte = match escape {
			b'a' => 0x07,
			b'b' => 0x08,
			b'f' => 0x0C,
			b'n' => b'\n',
			b't' => b'\t',
			b'r' => b'\r',
			b'0' => 0,
			b'\'' => b'\'',
			b'"' => b'"',
			b'\\' => b'\\',
			b'x' => {
				let value = self.hex_digits(2)?;
				return Ok(vec![value as u8]);
			}
			b'u' => {
				let value = self.hex_digits(4)?;
				return self.code_point(value);
			}
			b'U' => {
				let value = self.hex_digits(8)?;
				return self.code_point(value);
			}
			other => {
				return Err(self.err(self.pos(), format!("Unknown escape sequence: {}", other as char)));
			}
		};
		Ok(vec![byte])
	}

	fn hex_digits(&mut self, count: usize) -> Result<u32, HexpatError> {
		let mut value = 0u32;
		for _ in 0..count {
			let digit = self.peek(0);
			let Some(nibble) = (digit as char).to_digit(16) else {
				return Err(self.err(self.pos(), format!("Invalid hex digit in escape sequence: {}", digit as char)));
			};
			value = (value << 4) | nibble;
			self.cursor += 1;
		}
		Ok(value)
	}

	fn code_point(&self, value: u32) -> Result<Vec<u8>, HexpatError> {
		match char::from_u32(value) {
			Some(c) => {
				let mut buffer = [0u8; 4];
				Ok(c.encode_utf8(&mut buffer).as_bytes().to_vec())
			}
			None => Err(self.err(self.pos(), format!("Not a Unicode code point: U+{value:04X}"))),
		}
	}

	fn string_literal(&mut self) -> Result<String, HexpatError> {
		let pos = self.pos();
		self.cursor += 1;
		let mut bytes = Vec::new();
		loop {
			let c = self.peek(0);
			if c == b'"' {
				break;
			}
			if c == b'\n' || c == b'\r' {
				return Err(self.err(pos, "Unexpected newline in string literal"));
			}
			if c == 0 {
				return Err(self.err(pos, "Unexpected end of file in string literal"));
			}
			bytes.extend(self.character()?);
		}
		self.cursor += 1;
		Ok(String::from_utf8_lossy(&bytes).into_owned())
	}

	/// The word after a directive name, escapes and all.
	fn directive_value(&mut self) -> Result<(), HexpatError> {
		self.cursor += 1;
		let pos = self.pos();
		let mut bytes = Vec::new();
		while !matches!(self.peek(0), 0) && !self.peek(0).is_ascii_whitespace() {
			bytes.extend(self.character()?);
		}
		let ended = self.peek(0);
		if self.line_ended(ended) {
			self.cursor += 1;
		}
		self.raw.push(Raw::Text { text: String::from_utf8_lossy(&bytes).into_owned(), pos });
		Ok(())
	}

	/// Everything after that word, to the end of the line.
	fn directive_argument(&mut self) -> Result<(), HexpatError> {
		self.cursor += 1;
		let pos = self.pos();
		let mut bytes = Vec::new();
		while !matches!(self.peek(0), 0 | b'\n' | b'\r') {
			bytes.extend(self.character()?);
		}
		let ended = self.peek(0);
		if self.line_ended(ended) {
			self.cursor += 1;
		}
		self.raw.push(Raw::Text { text: String::from_utf8_lossy(&bytes).into_owned(), pos });
		Ok(())
	}

	fn one_line_comment(&mut self, skip: usize, doc: bool) {
		let pos = self.pos();
		self.cursor += skip;
		let start = self.cursor;
		while !matches!(self.peek(0), 0 | b'\n' | b'\r') {
			self.cursor += 1;
		}
		let text = String::from_utf8_lossy(&self.src[start..self.cursor]).into_owned();
		let ended = self.peek(0);
		if self.line_ended(ended) {
			self.cursor += 1;
		}
		if doc {
			self.raw.push(Raw::Doc { text, global: false, pos });
		} else {
			self.raw.push(Raw::Comment { pos });
		}
	}

	fn multi_line_comment(&mut self, doc: bool, global: bool) -> Result<(), HexpatError> {
		let pos = self.pos();
		self.cursor += if doc { 3 } else { 2 };
		let start = self.cursor;
		loop {
			let c = self.peek(0);
			self.line_ended(c);
			if self.peek(1) == 0 {
				return Err(self.err(pos, "Unexpected end of file while parsing multi line comment"));
			}
			if self.peek(0) == b'*' && self.peek(1) == b'/' {
				break;
			}
			self.cursor += 1;
		}
		let text = String::from_utf8_lossy(&self.src[start..self.cursor]).into_owned();
		self.cursor += 2;
		if doc {
			self.raw.push(Raw::Doc { text, global, pos });
		} else {
			self.raw.push(Raw::Comment { pos });
		}
		Ok(())
	}

	/// The longest operator that starts here, at most two characters. The
	/// cursor is left where it was: the caller advances it by the name's length.
	fn operator(&self) -> Option<Op> {
		let one = self.peek(0);
		two_char_operator([one, self.peek(1)]).or_else(|| one_char_operator(one))
	}
}

fn one_char_operator(c: u8) -> Option<Op> {
	Some(match c {
		b'+' => Op::Plus,
		b'-' => Op::Minus,
		b'*' => Op::Star,
		b'/' => Op::Slash,
		b'%' => Op::Percent,
		b'&' => Op::BitAnd,
		b'|' => Op::BitOr,
		b'^' => Op::BitXor,
		b'~' => Op::BitNot,
		b'<' => Op::Less,
		b'>' => Op::Greater,
		b'!' => Op::BoolNot,
		b'$' => Op::Dollar,
		b':' => Op::Colon,
		b'?' => Op::Ternary,
		b'@' => Op::At,
		b'=' => Op::Assign,
		_ => return None,
	})
}

fn two_char_operator(pair: [u8; 2]) -> Option<Op> {
	Some(match &pair {
		b"==" => Op::BoolEqual,
		b"!=" => Op::BoolNotEqual,
		b"&&" => Op::BoolAnd,
		b"||" => Op::BoolOr,
		b"^^" => Op::BoolXor,
		b"::" => Op::ScopeResolution,
		_ => return None,
	})
}

fn separator(c: u8) -> Option<Sep> {
	Some(match c {
		b'(' => Sep::LeftParen,
		b')' => Sep::RightParen,
		b'{' => Sep::LeftBrace,
		b'}' => Sep::RightBrace,
		b'[' => Sep::LeftBracket,
		b']' => Sep::RightBracket,
		b',' => Sep::Comma,
		b'.' => Sep::Dot,
		b';' => Sep::Semicolon,
		_ => return None,
	})
}

struct Preprocessor<'a> {
	file: &'a str,
	raw: Vec<Raw>,
	at: usize,
	defines: HashMap<String, Define>,
	/// Definition order, which is the order substitution is tried in.
	keys: Vec<String>,
	out: Vec<Token>,
	pragmas: Vec<Pragma>,
	includes: Vec<Include>,
	docs: Vec<DocComment>,
}

impl<'a> Preprocessor<'a> {
	fn new(file: &'a str, raw: Vec<Raw>, predefined: &[String]) -> Self {
		let mut defines = HashMap::new();
		let mut keys = Vec::new();
		for name in predefined {
			defines.insert(name.clone(), Define { tokens: Vec::new() });
			keys.push(name.clone());
		}
		Preprocessor {
			file,
			raw,
			at: 0,
			defines,
			keys,
			out: Vec::new(),
			pragmas: Vec::new(),
			includes: Vec::new(),
			docs: Vec::new(),
		}
	}

	fn err(&self, pos: Pos, message: impl Into<String>) -> HexpatError {
		HexpatError::new(self.file, pos, message)
	}

	fn eof(&self) -> bool {
		self.at >= self.raw.len()
	}

	fn peek(&self) -> Option<&Raw> {
		self.raw.get(self.at)
	}

	fn line(&self) -> u32 {
		self.peek().map_or(0, |raw| raw.pos().line)
	}

	/// Drop the rest of the line, keeping only what the output still wants.
	fn next_line(&mut self, line: u32) {
		while !self.eof() && self.raw[self.at].pos().line == line {
			if let Raw::Token(token) = &self.raw[self.at] {
				if token.tok == Tok::End {
					self.out.push(token.clone());
				}
			}
			self.at += 1;
		}
	}

	fn run(&mut self) -> Result<(), HexpatError> {
		while !self.eof() {
			self.process()?;
		}
		if self.out.last().map(|token| &token.tok) != Some(&Tok::End) {
			let pos = self.out.last().map_or(Pos::default(), |token| token.pos);
			self.out.push(Token { tok: Tok::End, pos });
		}
		Ok(())
	}

	fn process(&mut self) -> Result<(), HexpatError> {
		let line = self.line();
		match self.raw[self.at].clone() {
			Raw::Directive { name, pos } => {
				self.at += 1;
				self.directive(name, pos, line)?;
				self.next_line(line);
			}
			Raw::Comment { .. } => {
				self.at += 1;
			}
			Raw::Doc { text, global, pos } => {
				self.at += 1;
				self.docs.push(DocComment { text, global, pos, before: self.out.len() });
			}
			Raw::Text { pos, .. } => {
				// Only a directive produces one of these, and every directive
				// consumes its own.
				return Err(self.err(pos, "Stray directive argument"));
			}
			Raw::Token(token) => {
				self.at += 1;
				self.expand(token);
			}
		}
		Ok(())
	}

	/// Substitute macros into one token, one pass over the keys in order.
	fn expand(&mut self, token: Token) {
		let mut values = vec![token];
		for key in self.keys.clone() {
			let mut result = Vec::new();
			for value in values {
				match &value.tok {
					Tok::Ident(name) if *name == key => {
						result.extend(self.defines[&key].tokens.iter().cloned());
					}
					_ => result.push(value),
				}
			}
			values = result;
		}
		self.out.extend(values);
	}

	fn take_text(&mut self, line: u32) -> Option<(String, Pos)> {
		match self.raw.get(self.at) {
			Some(Raw::Text { text, pos }) if pos.line == line => {
				let value = (text.clone(), *pos);
				self.at += 1;
				Some(value)
			}
			_ => None,
		}
	}

	fn take_identifier(&mut self, line: u32) -> Option<String> {
		match self.raw.get(self.at) {
			Some(Raw::Token(Token { tok: Tok::Ident(name), pos })) if pos.line == line => {
				let name = name.clone();
				self.at += 1;
				Some(name)
			}
			_ => None,
		}
	}

	fn directive(&mut self, name: Directive, pos: Pos, line: u32) -> Result<(), HexpatError> {
		match name {
			Directive::Pragma => {
				let Some((key, key_pos)) = self.take_text(line) else {
					return Err(self.err(pos, "No instruction given in #pragma directive."));
				};
				let value = self.take_text(line).map(|(text, _)| text).unwrap_or_default();
				self.pragmas.push(Pragma { key, value, pos: key_pos });
			}
			Directive::Include => {
				let Some((path, path_pos)) = self.take_text(line) else {
					return Err(self.err(pos, "No file to include given in #include directive."));
				};
				let stripped = if path.starts_with('"') && path.ends_with('"') && path.len() >= 2 {
					&path[1..path.len() - 1]
				} else if path.starts_with('<') && path.ends_with('>') && path.len() >= 2 {
					&path[1..path.len() - 1]
				} else {
					return Err(self.err(path_pos, "Invalid file to include given in #include directive."));
				};
				self.includes.push(Include { path: stripped.to_string(), pos: path_pos });
			}
			Directive::Error => {
				let message = self.take_text(line).map(|(text, _)| text).unwrap_or_default();
				let rest = self.take_text(line).map(|(text, _)| text).unwrap_or_default();
				let message = if rest.is_empty() { message } else { format!("{message} {rest}") };
				return Err(self.err(pos, message));
			}
			Directive::Define => {
				let Some(name) = self.take_identifier(line) else {
					return Err(self.err(pos, "Expected identifier after #define"));
				};
				let mut tokens = Vec::new();
				while let Some(Raw::Token(token)) = self.raw.get(self.at) {
					if token.pos.line != line || token.tok == Tok::End {
						break;
					}
					tokens.push(token.clone());
					self.at += 1;
				}
				let define = Define { tokens };
				if let Some(previous) = self.defines.get(&name) {
					if *previous != define {
						return Err(self.err(pos, format!("Macro '{name}' is redefined.")));
					}
				} else {
					self.keys.push(name.clone());
				}
				self.defines.insert(name, define);
			}
			Directive::Undef => {
				let Some(name) = self.take_identifier(line) else {
					return Err(self.err(pos, "Expected identifier after #undef"));
				};
				self.defines.remove(&name);
				self.keys.retain(|key| *key != name);
			}
			Directive::IfDef | Directive::IfNDef => {
				let Some(macro_name) = self.take_identifier(line) else {
					return Err(self.err(pos, "Expected identifier after #ifdef"));
				};
				let defined = self.defines.contains_key(&macro_name);
				let active = if name == Directive::IfDef { defined } else { !defined };
				self.next_line(line);
				self.if_def(active, pos)?;
			}
			Directive::EndIf => {
				return Err(self.err(pos, "#endif without #ifdef"));
			}
		}
		Ok(())
	}

	fn if_def(&mut self, add: bool, start: Pos) -> Result<(), HexpatError> {
		let mut depth = 1u32;
		while !self.eof() && depth > 0 {
			if let Raw::Directive { name: Directive::EndIf, .. } = self.raw[self.at] {
				depth -= 1;
				let line = self.line();
				self.next_line(line);
				continue;
			}
			if add {
				self.process()?;
			} else {
				if let Raw::Directive { name: Directive::IfDef | Directive::IfNDef, .. } = self.raw[self.at] {
					depth += 1;
				}
				let line = self.line();
				self.next_line(line);
			}
		}
		if depth > 0 {
			return Err(self.err(start, "#ifdef without #endif"));
		}
		Ok(())
	}
}
