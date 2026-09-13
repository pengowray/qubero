//! Tokens of the Kaitai expression language.
//!
//! Three rules here are not the obvious ones, and all three come from the
//! compiler's own lexer:
//!
//! * `<` `>` `=` `!` are single tokens. The parser fuses touching pairs into
//!   `<<`, `>>`, `<=`, `>=`, `==` and `!=`. It has to: a cast closes with one
//!   `>`, so `x.as<u4>>3` is a shift of a cast, and a lexer that took `>>`
//!   greedily could not read it.
//! * A number is a float only where the compiler says so. `1.to_s` is the
//!   integer 1 with an attribute; `1.` and `1. + x` are floats; `.5` is a
//!   float. The rule is that a `.` after digits starts a float unless a name
//!   follows it.
//! * A single-quoted string has no escapes at all: `'\n'` is a backslash and
//!   an `n`. Only a double-quoted one reads the escape table.
//!
//! Whitespace is a space or a newline, plus a backslash before a newline, and
//! nothing else, which again is the compiler's rule. A tab inside an
//! expression is something it rejects.

use std::fmt;

use super::ParseError;

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
	Ident(String),
	Int(i128),
	Float(f64),
	/// A string literal with its escapes already resolved.
	Str(String),
	/// `f"` — the start of an interpolated string.
	FStrBegin,
	/// A run of literal text inside an interpolated string.
	FStrChunk(String),
	/// The `"` that closes an interpolated string.
	FStrEnd,
	LBrace,
	RBrace,
	LParen,
	RParen,
	LBracket,
	RBracket,
	Comma,
	Colon,
	ColonColon,
	Question,
	Dot,
	Plus,
	Minus,
	Star,
	Slash,
	Percent,
	Amp,
	Pipe,
	Caret,
	Tilde,
	Lt,
	Gt,
	Assign,
	Bang,
}

impl fmt::Display for Tok {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Tok::Ident(name) => f.write_str(name),
			Tok::Int(n) => write!(f, "{n}"),
			Tok::Float(x) => write!(f, "{x:?}"),
			Tok::Str(_) => f.write_str("a string"),
			Tok::FStrBegin => f.write_str("f\""),
			Tok::FStrChunk(_) => f.write_str("text"),
			Tok::FStrEnd => f.write_str("\""),
			Tok::LBrace => f.write_str("{"),
			Tok::RBrace => f.write_str("}"),
			Tok::LParen => f.write_str("("),
			Tok::RParen => f.write_str(")"),
			Tok::LBracket => f.write_str("["),
			Tok::RBracket => f.write_str("]"),
			Tok::Comma => f.write_str(","),
			Tok::Colon => f.write_str(":"),
			Tok::ColonColon => f.write_str("::"),
			Tok::Question => f.write_str("?"),
			Tok::Dot => f.write_str("."),
			Tok::Plus => f.write_str("+"),
			Tok::Minus => f.write_str("-"),
			Tok::Star => f.write_str("*"),
			Tok::Slash => f.write_str("/"),
			Tok::Percent => f.write_str("%"),
			Tok::Amp => f.write_str("&"),
			Tok::Pipe => f.write_str("|"),
			Tok::Caret => f.write_str("^"),
			Tok::Tilde => f.write_str("~"),
			Tok::Lt => f.write_str("<"),
			Tok::Gt => f.write_str(">"),
			Tok::Assign => f.write_str("="),
			Tok::Bang => f.write_str("!"),
		}
	}
}

/// A token and where it starts, in bytes from the start of the expression.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
	pub tok: Tok,
	pub at: usize,
	pub len: usize,
}

impl Token {
	/// Whether `next` starts exactly where this one ends, which is what makes
	/// two `>` a shift rather than two comparisons.
	pub fn touches(&self, next: &Token) -> bool {
		self.at + self.len == next.at
	}
}

/// Where the lexer is: inside an interpolated string, or inside a `{}` in one.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
	Normal,
	FString,
}

pub fn lex(src: &str) -> Result<Vec<Token>, ParseError> {
	Lexer { src: src.as_bytes(), text: src, pos: 0, modes: vec![Mode::Normal], out: Vec::new() }.run()
}

struct Lexer<'a> {
	src: &'a [u8],
	text: &'a str,
	pos: usize,
	modes: Vec<Mode>,
	out: Vec<Token>,
}

impl<'a> Lexer<'a> {
	fn run(mut self) -> Result<Vec<Token>, ParseError> {
		loop {
			if *self.modes.last().expect("mode stack is never empty") == Mode::FString {
				self.f_string_chunk()?;
				continue;
			}
			self.skip_ws();
			if self.pos >= self.src.len() {
				break;
			}
			self.token()?;
		}
		if self.modes.len() > 1 {
			return Err(self.error(self.pos, "unterminated interpolated string"));
		}
		Ok(self.out)
	}

	fn error(&self, at: usize, message: impl Into<String>) -> ParseError {
		ParseError { source: self.text.to_string(), at, message: message.into() }
	}

	fn peek(&self) -> Option<u8> {
		self.src.get(self.pos).copied()
	}

	fn at(&self, offset: usize) -> Option<u8> {
		self.src.get(self.pos + offset).copied()
	}

	fn push(&mut self, tok: Tok, at: usize) {
		let len = self.pos - at;
		self.out.push(Token { tok, at, len });
	}

	fn skip_ws(&mut self) {
		while let Some(c) = self.peek() {
			match c {
				b' ' | b'\n' => self.pos += 1,
				b'\\' if self.at(1) == Some(b'\n') => self.pos += 2,
				_ => break,
			}
		}
	}

	fn token(&mut self) -> Result<(), ParseError> {
		let start = self.pos;
		let c = self.peek().expect("caller checked for end of input");
		// `f"` opens an interpolated string; `f` alone is a name.
		if c == b'f' && self.at(1) == Some(b'"') {
			self.pos += 2;
			self.push(Tok::FStrBegin, start);
			self.modes.push(Mode::FString);
			return Ok(());
		}
		if c.is_ascii_alphabetic() || c == b'_' {
			while self.peek().is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_') {
				self.pos += 1;
			}
			let name = self.text[start..self.pos].to_string();
			self.push(Tok::Ident(name), start);
			return Ok(());
		}
		if c.is_ascii_digit() || (c == b'.' && self.at(1).is_some_and(|c| c.is_ascii_digit())) {
			return self.number();
		}
		if c == b'\'' || c == b'"' {
			return self.string();
		}
		let simple = match c {
			b'(' => Tok::LParen,
			b')' => Tok::RParen,
			b'[' => Tok::LBracket,
			b']' => Tok::RBracket,
			b',' => Tok::Comma,
			b'?' => Tok::Question,
			b'.' => Tok::Dot,
			b'+' => Tok::Plus,
			b'-' => Tok::Minus,
			b'*' => Tok::Star,
			b'/' => Tok::Slash,
			b'%' => Tok::Percent,
			b'&' => Tok::Amp,
			b'|' => Tok::Pipe,
			b'^' => Tok::Caret,
			b'~' => Tok::Tilde,
			b'<' => Tok::Lt,
			b'>' => Tok::Gt,
			b'=' => Tok::Assign,
			b'!' => Tok::Bang,
			b'{' => Tok::LBrace,
			b'}' => Tok::RBrace,
			b':' => {
				if self.at(1) == Some(b':') {
					self.pos += 2;
					self.push(Tok::ColonColon, start);
					return Ok(());
				}
				Tok::Colon
			}
			_ => {
				let ch = self.text[start..].chars().next().unwrap_or('?');
				return Err(self.error(start, format!("unexpected character `{ch}`")));
			}
		};
		self.pos += 1;
		if simple == Tok::RBrace {
			// The `}` that closes a `{expr}` inside an interpolated string.
			if self.modes.len() > 1 {
				self.modes.pop();
			}
		}
		self.push(simple, start);
		Ok(())
	}

	/// An integer or a float, with `_` allowed as a separator in either.
	fn number(&mut self) -> Result<(), ParseError> {
		let start = self.pos;
		if self.peek() == Some(b'0') {
			if let Some(radix_char) = self.at(1) {
				let radix = match radix_char {
					b'x' | b'X' => Some(16),
					b'o' | b'O' => Some(8),
					b'b' | b'B' => Some(2),
					_ => None,
				};
				if let Some(radix) = radix {
					let digits_at = self.pos + 2;
					let mut end = digits_at;
					while self
						.src
						.get(end)
						.is_some_and(|c| *c == b'_' || (*c as char).is_digit(radix))
					{
						end += 1;
					}
					if end > digits_at {
						let digits: String =
							self.text[digits_at..end].chars().filter(|c| *c != '_').collect();
						let value = i128::from_str_radix(&digits, radix).map_err(|_| {
							self.error(start, "integer literal does not fit in 128 bits")
						})?;
						self.pos = end;
						self.push(Tok::Int(value), start);
						return Ok(());
					}
				}
			}
		}
		if let Some(end) = self.float_end() {
			let digits: String = self.text[start..end].chars().filter(|c| *c != '_').collect();
			let value: f64 = digits
				.parse()
				.map_err(|_| self.error(start, "not a floating point literal"))?;
			self.pos = end;
			self.push(Tok::Float(value), start);
			return Ok(());
		}
		// A decimal integer is `0`, or a non-zero digit and then digits: a
		// leading zero on a longer number is not one.
		if self.peek() == Some(b'0') {
			self.pos += 1;
		} else {
			while self.peek().is_some_and(|c| c.is_ascii_digit() || c == b'_') {
				self.pos += 1;
			}
		}
		let digits: String = self.text[start..self.pos].chars().filter(|c| *c != '_').collect();
		let value: i128 = digits
			.parse()
			.map_err(|_| self.error(start, "integer literal does not fit in 128 bits"))?;
		self.push(Tok::Int(value), start);
		Ok(())
	}

	/// Where the float literal starting here ends, if there is one.
	fn float_end(&self) -> Option<usize> {
		let mut i = self.pos;
		while self.src.get(i).is_some_and(u8::is_ascii_digit) {
			i += 1;
		}
		let int_digits = i - self.pos;
		if self.src.get(i) == Some(&b'.') {
			let dot = i;
			i += 1;
			let frac_at = i;
			while self.src.get(i).is_some_and(u8::is_ascii_digit) {
				i += 1;
			}
			if i == frac_at {
				// `42.`, but not `42.abc` and not `42.  def`: a name after the
				// point makes this an integer with an attribute.
				if int_digits == 0 {
					return None;
				}
				let mut j = dot + 1;
				while matches!(self.src.get(j), Some(b' ' | b'\n')) {
					j += 1;
				}
				if self.src.get(j).is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_') {
					return None;
				}
			}
			return Some(self.exponent_end(i).unwrap_or(i));
		}
		if int_digits == 0 {
			return None;
		}
		// No point, so only an exponent makes this a float.
		self.exponent_end(i)
	}

	fn exponent_end(&self, from: usize) -> Option<usize> {
		let mut i = from;
		if !matches!(self.src.get(i), Some(b'e' | b'E')) {
			return None;
		}
		i += 1;
		if matches!(self.src.get(i), Some(b'+' | b'-')) {
			i += 1;
		}
		let digits_at = i;
		while self.src.get(i).is_some_and(u8::is_ascii_digit) {
			i += 1;
		}
		if i == digits_at { None } else { Some(i) }
	}

	fn string(&mut self) -> Result<(), ParseError> {
		let start = self.pos;
		let quote = self.peek().expect("caller checked the quote");
		self.pos += 1;
		let mut out = String::new();
		loop {
			let Some(c) = self.peek() else {
				return Err(self.error(start, "unterminated string literal"));
			};
			if c == quote {
				self.pos += 1;
				break;
			}
			if quote == b'"' && c == b'\\' {
				self.escape(&mut out)?;
				continue;
			}
			let ch = self.text[self.pos..].chars().next().expect("position is a char boundary");
			out.push(ch);
			self.pos += ch.len_utf8();
		}
		self.push(Tok::Str(out), start);
		Ok(())
	}

	/// One `\...` escape, from the compiler's table.
	fn escape(&mut self, out: &mut String) -> Result<(), ParseError> {
		let start = self.pos;
		self.pos += 1;
		let Some(c) = self.peek() else {
			return Err(self.error(start, "string ends in a backslash"));
		};
		let simple = match c {
			b'a' => Some('\u{7}'),
			b'b' => Some('\u{8}'),
			b'e' => Some('\u{1b}'),
			b'f' => Some('\u{c}'),
			b'n' => Some('\n'),
			b'r' => Some('\r'),
			b't' => Some('\t'),
			b'v' => Some('\u{b}'),
			b'\'' => Some('\''),
			b'"' => Some('"'),
			b'\\' => Some('\\'),
			_ => None,
		};
		if let Some(ch) = simple {
			out.push(ch);
			self.pos += 1;
			return Ok(());
		}
		if c == b'u' {
			self.pos += 1;
			let digits_at = self.pos;
			for _ in 0..4 {
				if !self.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
					return Err(self.error(start, "`\\u` takes exactly four hex digits"));
				}
				self.pos += 1;
			}
			let code = u32::from_str_radix(&self.text[digits_at..self.pos], 16)
				.expect("four hex digits fit in a u32");
			let ch = char::from_u32(code)
				.ok_or_else(|| self.error(start, "`\\u` names no character"))?;
			out.push(ch);
			return Ok(());
		}
		if (b'0'..=b'7').contains(&c) {
			let digits_at = self.pos;
			while self.peek().is_some_and(|c| (b'0'..=b'7').contains(&c)) {
				self.pos += 1;
			}
			let code = u32::from_str_radix(&self.text[digits_at..self.pos], 8)
				.map_err(|_| self.error(start, "octal escape is too large"))?;
			let ch = char::from_u32(code)
				.ok_or_else(|| self.error(start, "octal escape names no character"))?;
			out.push(ch);
			return Ok(());
		}
		Err(self.error(start, "unknown escape"))
	}

	/// The text between `{}` holes of an interpolated string, up to the next
	/// hole or the closing quote.
	fn f_string_chunk(&mut self) -> Result<(), ParseError> {
		let start = self.pos;
		let mut out = String::new();
		loop {
			let Some(c) = self.peek() else {
				return Err(self.error(start, "unterminated interpolated string"));
			};
			match c {
				b'"' => {
					if !out.is_empty() {
						let at = self.pos;
						self.out.push(Token { tok: Tok::FStrChunk(out), at: start, len: at - start });
					}
					self.pos += 1;
					self.push(Tok::FStrEnd, self.pos - 1);
					self.modes.pop();
					return Ok(());
				}
				b'{' => {
					if !out.is_empty() {
						let at = self.pos;
						self.out.push(Token { tok: Tok::FStrChunk(out), at: start, len: at - start });
					}
					let at = self.pos;
					self.pos += 1;
					self.push(Tok::LBrace, at);
					self.modes.push(Mode::Normal);
					return Ok(());
				}
				b'\\' => self.escape(&mut out)?,
				_ => {
					let ch =
						self.text[self.pos..].chars().next().expect("position is a char boundary");
					out.push(ch);
					self.pos += ch.len_utf8();
				}
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn toks(src: &str) -> Vec<Tok> {
		lex(src).expect("lexes").into_iter().map(|t| t.tok).collect()
	}

	#[test]
	fn numbers_in_every_base_with_separators() {
		assert_eq!(toks("123_456"), vec![Tok::Int(123456)]);
		assert_eq!(toks("0x1234_5678"), vec![Tok::Int(0x1234_5678)]);
		assert_eq!(toks("0b1101_0111"), vec![Tok::Int(0b1101_0111)]);
		assert_eq!(toks("0o755"), vec![Tok::Int(0o755)]);
		assert_eq!(toks("0"), vec![Tok::Int(0)]);
		assert_eq!(toks("0XcAfE"), vec![Tok::Int(0xcafe)]);
	}

	#[test]
	fn a_point_after_digits_is_a_float_only_when_no_name_follows() {
		assert_eq!(toks("1.to_s"), vec![Tok::Int(1), Tok::Dot, Tok::Ident("to_s".into())]);
		assert_eq!(toks("1."), vec![Tok::Float(1.0)]);
		assert_eq!(toks("1. + x"), vec![Tok::Float(1.0), Tok::Plus, Tok::Ident("x".into())]);
		assert_eq!(toks(".5"), vec![Tok::Float(0.5)]);
		assert_eq!(toks("4e2"), vec![Tok::Float(400.0)]);
		assert_eq!(toks("4.1607804e+72"), vec![Tok::Float(4.1607804e72)]);
		assert_eq!(toks("123.456"), vec![Tok::Float(123.456)]);
	}

	#[test]
	fn angle_brackets_are_single_characters() {
		assert_eq!(toks("a >> b"), vec![Tok::Ident("a".into()), Tok::Gt, Tok::Gt, Tok::Ident("b".into())]);
		assert_eq!(toks("a >= b"), vec![Tok::Ident("a".into()), Tok::Gt, Tok::Assign, Tok::Ident("b".into())]);
		assert_eq!(toks("a != b"), vec![Tok::Ident("a".into()), Tok::Bang, Tok::Assign, Tok::Ident("b".into())]);
	}

	#[test]
	fn a_single_quoted_string_has_no_escapes() {
		assert_eq!(toks(r"'a\nb'"), vec![Tok::Str(r"a\nb".into())]);
		assert_eq!(toks(r#""a\nb""#), vec![Tok::Str("a\nb".into())]);
		// `\101` is octal for `A`, and `A` names the same character.
		assert_eq!(toks(r#""A\101A""#), vec![Tok::Str("AAA".into())]);
	}

	#[test]
	fn an_interpolated_string_splits_into_text_and_holes() {
		assert_eq!(
			toks("f\"a{b + 1}c\""),
			vec![
				Tok::FStrBegin,
				Tok::FStrChunk("a".into()),
				Tok::LBrace,
				Tok::Ident("b".into()),
				Tok::Plus,
				Tok::Int(1),
				Tok::RBrace,
				Tok::FStrChunk("c".into()),
				Tok::FStrEnd,
			]
		);
	}

	#[test]
	fn f_alone_is_a_name() {
		assert_eq!(toks("f"), vec![Tok::Ident("f".into())]);
		assert_eq!(toks("foo"), vec![Tok::Ident("foo".into())]);
	}

	#[test]
	fn a_tab_is_not_whitespace_here() {
		assert!(lex("1 +\t2").is_err());
	}
}
