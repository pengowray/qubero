//! The expression parser: precedence climbing over the token stream.
//!
//! The rungs are the compiler's grammar rules, loosest first: the ternary,
//! then `or`, `and`, `not`, a comparison, then `|`, `^`, `&`, the shifts,
//! `+ -`, `* / %`, the unary `- ~`, and finally an atom with its trailers
//! (`.attr`, `.method(args)`, `[i]`, `.as<t>`).
//!
//! Two rules are easy to get wrong and are worth saying out loud. A comparison
//! does not chain: `a < b < c` is not an expression here, and the loop below
//! takes at most one comparison operator. And `a and b and c` is one node with
//! three values rather than two nested nodes, which is what the compiler
//! builds, so `(a and b) and c` is a different tree and prints with its
//! parentheses.

use super::ast::{BinOp, BoolOp, CmpOp, Expr, TypeId, UnaryOp};
use super::lexer::{Tok, Token, lex};
use super::ParseError;

/// Parse one expression. `Expressions.parse`.
pub fn parse(src: &str) -> Result<Expr, ParseError> {
	let mut p = Parser::new(src)?;
	let expr = p.test()?;
	p.expect_end()?;
	Ok(expr)
}

/// Parse a comma-separated list of expressions. `Expressions.parseList`, which
/// is what a custom `process` reads its arguments with.
pub fn parse_list(src: &str) -> Result<Vec<Expr>, ParseError> {
	let mut p = Parser::new(src)?;
	let mut out = vec![p.test()?];
	while p.eat(&Tok::Comma) {
		out.push(p.test()?);
	}
	p.expect_end()?;
	Ok(out)
}

/// Parse a `type:` string that names a user type, with its arguments.
/// `Expressions.parseTypeRef`.
pub fn parse_type_ref(src: &str) -> Result<(TypeId, Vec<Expr>), ParseError> {
	let mut p = Parser::new(src)?;
	let name = p.type_name()?;
	let mut args = Vec::new();
	if p.eat(&Tok::LParen) {
		// No trailing comma: `my_str(2 + 3, )` is what the compiler rejects in
		// its own tests/formats_err/params_call_malformed.ksy.
		args.push(p.test()?);
		while p.eat(&Tok::Comma) {
			args.push(p.test()?);
		}
		p.expect(&Tok::RParen)?;
	}
	p.expect_end()?;
	Ok((name, args))
}

/// Rungs of the grammar; see the module comment.
const BIT_OR: u8 = 5;
const TERM: u8 = 10;

struct Parser {
	src: String,
	toks: Vec<Token>,
	pos: usize,
}

impl Parser {
	fn new(src: &str) -> Result<Self, ParseError> {
		let src = src.trim().to_string();
		let toks = lex(&src)?;
		Ok(Parser { src, toks, pos: 0 })
	}

	fn error(&self, message: impl Into<String>) -> ParseError {
		let at = self.toks.get(self.pos).map_or(self.src.len(), |t| t.at);
		ParseError { source: self.src.clone(), at, message: message.into() }
	}

	fn peek(&self) -> Option<&Tok> {
		self.toks.get(self.pos).map(|t| &t.tok)
	}

	fn peek_is(&self, tok: &Tok) -> bool {
		self.peek() == Some(tok)
	}

	fn eat(&mut self, tok: &Tok) -> bool {
		if self.peek_is(tok) {
			self.pos += 1;
			return true;
		}
		false
	}

	fn expect(&mut self, tok: &Tok) -> Result<(), ParseError> {
		if self.eat(tok) {
			return Ok(());
		}
		Err(self.error(format!("expected `{tok}`")))
	}

	fn expect_end(&self) -> Result<(), ParseError> {
		if self.pos < self.toks.len() {
			return Err(self.error("expected the end of the expression"));
		}
		Ok(())
	}

	/// The next token as a word, when it is one.
	fn keyword(&self) -> Option<&str> {
		match self.peek() {
			Some(Tok::Ident(name)) => Some(name.as_str()),
			_ => None,
		}
	}

	fn eat_keyword(&mut self, word: &str) -> bool {
		if self.keyword() == Some(word) {
			self.pos += 1;
			return true;
		}
		false
	}

	/// Whether the token at `pos` and the one after it are written without a
	/// gap, which is what turns two `>` into a shift.
	fn joined(&self, first: &Tok, second: &Tok) -> bool {
		let (Some(a), Some(b)) = (self.toks.get(self.pos), self.toks.get(self.pos + 1)) else {
			return false;
		};
		&a.tok == first && &b.tok == second && a.touches(b)
	}

	// ---- the rungs ----------------------------------------------------

	fn test(&mut self) -> Result<Expr, ParseError> {
		let condition = self.or_test()?;
		if !self.eat(&Tok::Question) {
			return Ok(condition);
		}
		let if_true = self.test()?;
		self.expect(&Tok::Colon)?;
		let if_false = self.test()?;
		Ok(Expr::IfExp {
			condition: Box::new(condition),
			if_true: Box::new(if_true),
			if_false: Box::new(if_false),
		})
	}

	fn or_test(&mut self) -> Result<Expr, ParseError> {
		let first = self.and_test()?;
		if self.keyword() != Some("or") {
			return Ok(first);
		}
		let mut values = vec![first];
		while self.eat_keyword("or") {
			values.push(self.and_test()?);
		}
		Ok(Expr::BoolOp { op: BoolOp::Or, values })
	}

	fn and_test(&mut self) -> Result<Expr, ParseError> {
		let first = self.not_test()?;
		if self.keyword() != Some("and") {
			return Ok(first);
		}
		let mut values = vec![first];
		while self.eat_keyword("and") {
			values.push(self.not_test()?);
		}
		Ok(Expr::BoolOp { op: BoolOp::And, values })
	}

	fn not_test(&mut self) -> Result<Expr, ParseError> {
		if self.eat_keyword("not") {
			let operand = self.not_test()?;
			return Ok(Expr::UnaryOp { op: UnaryOp::Not, operand: Box::new(operand) });
		}
		self.comparison()
	}

	fn comparison(&mut self) -> Result<Expr, ParseError> {
		let left = self.binary(BIT_OR)?;
		let Some(op) = self.comparison_op() else { return Ok(left) };
		let right = self.binary(BIT_OR)?;
		Ok(Expr::Compare { left: Box::new(left), op, right: Box::new(right) })
	}

	/// One comparison operator, built from the single characters the lexer
	/// leaves behind. Nothing here loops: a comparison takes one operator.
	fn comparison_op(&mut self) -> Option<CmpOp> {
		let pairs = [
			(Tok::Lt, Tok::Assign, CmpOp::LtE),
			(Tok::Gt, Tok::Assign, CmpOp::GtE),
			(Tok::Assign, Tok::Assign, CmpOp::Eq),
			(Tok::Bang, Tok::Assign, CmpOp::NotEq),
		];
		for (a, b, op) in pairs {
			if self.joined(&a, &b) {
				self.pos += 2;
				return Some(op);
			}
		}
		// A lone `<` or `>`: a shift would have been taken at its own rung.
		match self.peek() {
			Some(Tok::Lt) => {
				self.pos += 1;
				Some(CmpOp::Lt)
			}
			Some(Tok::Gt) => {
				self.pos += 1;
				Some(CmpOp::Gt)
			}
			_ => None,
		}
	}

	/// The binary operators, tightest rung last.
	fn binary(&mut self, rung: u8) -> Result<Expr, ParseError> {
		if rung > TERM {
			return self.factor();
		}
		let mut left = self.binary(rung + 1)?;
		while let Some(op) = self.binary_op(rung) {
			let right = self.binary(rung + 1)?;
			left = Expr::BinOp { left: Box::new(left), op, right: Box::new(right) };
		}
		Ok(left)
	}

	fn binary_op(&mut self, rung: u8) -> Option<BinOp> {
		let single = |tok: &Tok| -> Option<BinOp> {
			Some(match (rung, tok) {
				(5, Tok::Pipe) => BinOp::BitOr,
				(6, Tok::Caret) => BinOp::BitXor,
				(7, Tok::Amp) => BinOp::BitAnd,
				(9, Tok::Plus) => BinOp::Add,
				(9, Tok::Minus) => BinOp::Sub,
				(10, Tok::Star) => BinOp::Mult,
				(10, Tok::Slash) => BinOp::Div,
				(10, Tok::Percent) => BinOp::Mod,
				_ => return None,
			})
		};
		if rung == 8 {
			if self.joined(&Tok::Lt, &Tok::Lt) {
				self.pos += 2;
				return Some(BinOp::LShift);
			}
			if self.joined(&Tok::Gt, &Tok::Gt) {
				self.pos += 2;
				return Some(BinOp::RShift);
			}
			return None;
		}
		let op = single(self.peek()?)?;
		self.pos += 1;
		Some(op)
	}

	fn factor(&mut self) -> Result<Expr, ParseError> {
		// A leading `+` is dropped, which is what the compiler does with it.
		if self.eat(&Tok::Plus) {
			return self.factor();
		}
		if self.eat(&Tok::Minus) {
			let operand = self.factor()?;
			return Ok(Expr::UnaryOp { op: UnaryOp::Minus, operand: Box::new(operand) });
		}
		if self.eat(&Tok::Tilde) {
			let operand = self.factor()?;
			return Ok(Expr::UnaryOp { op: UnaryOp::Invert, operand: Box::new(operand) });
		}
		self.power()
	}

	fn power(&mut self) -> Result<Expr, ParseError> {
		let mut value = self.atom()?;
		loop {
			if self.eat(&Tok::LParen) {
				let mut args = Vec::new();
				if !self.peek_is(&Tok::RParen) {
					args.push(self.test()?);
					while self.eat(&Tok::Comma) {
						args.push(self.test()?);
					}
				}
				self.expect(&Tok::RParen)?;
				value = Expr::Call { func: Box::new(value), args };
				continue;
			}
			if self.eat(&Tok::LBracket) {
				let index = self.test()?;
				self.expect(&Tok::RBracket)?;
				value = Expr::Subscript { value: Box::new(value), index: Box::new(index) };
				continue;
			}
			if self.peek_is(&Tok::Dot) {
				// `.as<t>` is a cast; anything else after the point is an
				// attribute, including a field actually named `as...`.
				let save = self.pos;
				self.pos += 1;
				if self.keyword() == Some("as") {
					self.pos += 1;
					if self.eat(&Tok::Lt) {
						let type_name = self.type_name()?;
						self.expect(&Tok::Gt)?;
						value = Expr::CastToType { value: Box::new(value), type_name };
						continue;
					}
				}
				self.pos = save + 1;
				let Some(Tok::Ident(attr)) = self.peek().cloned() else {
					return Err(self.error("expected a name after `.`"));
				};
				self.pos += 1;
				value = Expr::Attribute { value: Box::new(value), attr };
				continue;
			}
			break;
		}
		Ok(value)
	}

	fn atom(&mut self) -> Result<Expr, ParseError> {
		match self.peek().cloned() {
			None => Err(self.error("expected an expression")),
			Some(Tok::LBracket) => {
				self.pos += 1;
				if self.eat(&Tok::RBracket) {
					return Ok(Expr::List(Vec::new()));
				}
				let mut items = vec![self.test()?];
				while self.eat(&Tok::Comma) {
					if self.peek_is(&Tok::RBracket) {
						break;
					}
					items.push(self.test()?);
				}
				self.expect(&Tok::RBracket)?;
				Ok(Expr::List(items))
			}
			Some(Tok::LParen) => {
				self.pos += 1;
				let inner = self.test()?;
				self.expect(&Tok::RParen)?;
				Ok(inner)
			}
			Some(Tok::Str(_)) => {
				// Literals written next to each other are one string.
				let mut text = String::new();
				while let Some(Tok::Str(part)) = self.peek() {
					text.push_str(part);
					self.pos += 1;
				}
				Ok(Expr::Str(text))
			}
			Some(Tok::FStrBegin) => self.f_string(),
			Some(Tok::Int(n)) => {
				self.pos += 1;
				Ok(Expr::IntNum(n))
			}
			Some(Tok::Float(x)) => {
				self.pos += 1;
				Ok(Expr::FloatNum(x))
			}
			Some(Tok::Ident(_) | Tok::ColonColon) => {
				if let Some(label) = self.enum_by_label() {
					return Ok(label);
				}
				let Some(Tok::Ident(name)) = self.peek().cloned() else {
					return Err(self.error("expected a name"));
				};
				if (name == "sizeof" || name == "bitsizeof")
					&& self.toks.get(self.pos + 1).is_some_and(|t| t.tok == Tok::Lt)
				{
					self.pos += 2;
					let type_name = self.type_name()?;
					self.expect(&Tok::Gt)?;
					return Ok(if name == "sizeof" {
						Expr::ByteSizeOfType(type_name)
					} else {
						Expr::BitSizeOfType(type_name)
					});
				}
				self.pos += 1;
				Ok(match name.as_str() {
					"true" => Expr::Bool(true),
					"false" => Expr::Bool(false),
					_ => Expr::Name(name),
				})
			}
			Some(tok) => Err(self.error(format!("`{tok}` cannot start an expression"))),
		}
	}

	/// `enum::label`, or `outer::enum::label`, with an optional leading `::`.
	/// Two names at least, or this is a plain name and the caller carries on.
	fn enum_by_label(&mut self) -> Option<Expr> {
		let save = self.pos;
		let absolute = self.eat(&Tok::ColonColon);
		let mut names = Vec::new();
		loop {
			let Some(Tok::Ident(name)) = self.peek().cloned() else { break };
			names.push(name);
			self.pos += 1;
			if !self.eat(&Tok::ColonColon) {
				break;
			}
		}
		if names.len() < 2 {
			self.pos = save;
			return None;
		}
		let label = names.pop().expect("two names at least");
		let enum_name = names.pop().expect("two names at least");
		Some(Expr::EnumByLabel { enum_name, label, in_type: TypeId { absolute, names, is_array: false } })
	}

	/// `[::]name[::name...][[]]`, the type name inside `as<>`, `sizeof<>` and
	/// a `type:` key.
	fn type_name(&mut self) -> Result<TypeId, ParseError> {
		let absolute = self.eat(&Tok::ColonColon);
		let mut names = Vec::new();
		loop {
			let Some(Tok::Ident(name)) = self.peek().cloned() else {
				return Err(self.error("expected a type name"));
			};
			names.push(name);
			self.pos += 1;
			if !self.eat(&Tok::ColonColon) {
				break;
			}
		}
		let mut is_array = false;
		if self.joined(&Tok::LBracket, &Tok::RBracket) || self.peek_is(&Tok::LBracket) {
			let save = self.pos;
			self.pos += 1;
			if self.eat(&Tok::RBracket) {
				is_array = true;
			} else {
				self.pos = save;
			}
		}
		Ok(TypeId { absolute, names, is_array })
	}

	fn f_string(&mut self) -> Result<Expr, ParseError> {
		self.expect(&Tok::FStrBegin)?;
		let mut parts = Vec::new();
		loop {
			match self.peek().cloned() {
				Some(Tok::FStrEnd) => {
					self.pos += 1;
					// A hole holding nothing but a string literal is that
					// text, so it joins the text around it: `f"a={'b'}"` and
					// `f"a=b"` are the same string, and writing them the same
					// way is what lets one be printed and read back.
					let mut merged: Vec<Expr> = Vec::with_capacity(parts.len());
					for part in parts {
						match (merged.last_mut(), &part) {
							(Some(Expr::Str(last)), Expr::Str(next)) => last.push_str(next),
							_ => merged.push(part),
						}
					}
					return Ok(Expr::InterpolatedStr(merged));
				}
				Some(Tok::FStrChunk(text)) => {
					self.pos += 1;
					parts.push(Expr::Str(text));
				}
				Some(Tok::LBrace) => {
					self.pos += 1;
					parts.push(self.test()?);
					self.expect(&Tok::RBrace)?;
				}
				_ => return Err(self.error("expected the end of the interpolated string")),
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn p(src: &str) -> Expr {
		parse(src).unwrap_or_else(|e| panic!("{src}: {e}"))
	}

	/// What the expression prints as, which is what the report shows.
	fn shown(src: &str) -> String {
		p(src).to_string()
	}

	fn round_trips(src: &str) {
		let first = p(src);
		let printed = first.to_string();
		let again = parse(&printed).unwrap_or_else(|e| panic!("{src} printed as {printed}: {e}"));
		assert_eq!(first, again, "{src} printed as {printed}");
	}

	#[test]
	fn multiplication_binds_tighter_than_addition() {
		assert_eq!(shown("1 + 2 * 3"), "1 + 2 * 3");
		assert_eq!(shown("(1 + 2) * 3"), "(1 + 2) * 3");
		assert_eq!(
			p("1 + 2 * 3"),
			Expr::BinOp {
				left: Box::new(Expr::IntNum(1)),
				op: BinOp::Add,
				right: Box::new(Expr::BinOp {
					left: Box::new(Expr::IntNum(2)),
					op: BinOp::Mult,
					right: Box::new(Expr::IntNum(3)),
				}),
			}
		);
	}

	#[test]
	fn a_ternary_nests_to_the_right() {
		assert_eq!(
			p("a ? b : c ? d : e"),
			Expr::IfExp {
				condition: Box::new(Expr::Name("a".into())),
				if_true: Box::new(Expr::Name("b".into())),
				if_false: Box::new(Expr::IfExp {
					condition: Box::new(Expr::Name("c".into())),
					if_true: Box::new(Expr::Name("d".into())),
					if_false: Box::new(Expr::Name("e".into())),
				}),
			}
		);
		assert_eq!(shown("a ? b : c ? d : e"), "a ? b : c ? d : e");
		assert_eq!(shown("(a ? b : c) ? d : e"), "(a ? b : c) ? d : e");
	}

	#[test]
	fn not_binds_tighter_than_and() {
		assert_eq!(
			p("not a and b"),
			Expr::BoolOp {
				op: BoolOp::And,
				values: vec![
					Expr::UnaryOp { op: UnaryOp::Not, operand: Box::new(Expr::Name("a".into())) },
					Expr::Name("b".into()),
				],
			}
		);
		assert_eq!(shown("not a and b"), "not a and b");
		assert_eq!(shown("not (a and b)"), "not (a and b)");
	}

	#[test]
	fn trailers_chain_left_to_right() {
		assert_eq!(shown("x.y[2].z.as<t>.size"), "x.y[2].z.as<t>.size");
		let expr = p("x.y[2].z.as<t>.size");
		assert!(matches!(expr, Expr::Attribute { attr, .. } if attr == "size"));
	}

	#[test]
	fn unary_minus_binds_tighter_than_modulo() {
		assert_eq!(
			p("-1 % 3"),
			Expr::BinOp {
				left: Box::new(Expr::UnaryOp {
					op: UnaryOp::Minus,
					operand: Box::new(Expr::IntNum(1)),
				}),
				op: BinOp::Mod,
				right: Box::new(Expr::IntNum(3)),
			}
		);
		assert_eq!(shown("-1 % 3"), "-1 % 3");
	}

	#[test]
	fn a_boolean_chain_is_one_node_and_a_nested_one_keeps_its_brackets() {
		let flat = p("a and b and c");
		assert!(matches!(&flat, Expr::BoolOp { values, .. } if values.len() == 3));
		assert_eq!(flat.to_string(), "a and b and c");
		assert_eq!(shown("(a and b) and c"), "(a and b) and c");
		assert_ne!(p("(a and b) and c"), flat);
	}

	#[test]
	fn a_comparison_does_not_chain() {
		assert!(parse("a < b < c").is_err());
		assert_eq!(shown("(a < b) == c"), "(a < b) == c");
	}

	#[test]
	fn shifts_and_comparisons_share_their_characters() {
		assert_eq!(shown("a >> 2"), "a >> 2");
		assert_eq!(shown("a > 2"), "a > 2");
		assert_eq!(shown("a >= 2"), "a >= 2");
		assert_eq!(shown("a << 2 | b"), "a << 2 | b");
		assert!(parse("a > > 2").is_err());
	}

	#[test]
	fn literals_of_every_kind() {
		assert_eq!(p("0x1234_5678"), Expr::IntNum(0x1234_5678));
		assert_eq!(p("0b1101"), Expr::IntNum(13));
		assert_eq!(p("0o17"), Expr::IntNum(15));
		assert_eq!(p("'a' \"b\""), Expr::Str("ab".into()));
		assert_eq!(p("[0x00, 1]"), Expr::List(vec![Expr::IntNum(0), Expr::IntNum(1)]));
		assert_eq!(p("[]"), Expr::List(Vec::new()));
		assert_eq!(p("true"), Expr::Bool(true));
		assert_eq!(
			p("animal::cat"),
			Expr::EnumByLabel {
				enum_name: "animal".into(),
				label: "cat".into(),
				in_type: TypeId::plain(Vec::new()),
			}
		);
		assert_eq!(shown("outer::animal::cat"), "outer::animal::cat");
		assert_eq!(shown("::outer::animal::cat"), "::outer::animal::cat");
	}

	#[test]
	fn sizeof_and_a_field_named_sizeof_are_different_things() {
		assert_eq!(p("sizeof<block>"), Expr::ByteSizeOfType(TypeId::plain(vec!["block".into()])));
		assert_eq!(p("bitsizeof<block>"), Expr::BitSizeOfType(TypeId::plain(vec!["block".into()])));
		assert_eq!(p("sizeof"), Expr::Name("sizeof".into()));
		assert_eq!(p("order"), Expr::Name("order".into()));
		assert_eq!(p("not_done"), Expr::Name("not_done".into()));
	}

	#[test]
	fn an_interpolated_string_keeps_its_holes() {
		assert_eq!(
			p("f\"v{major}.{minor}\""),
			Expr::InterpolatedStr(vec![
				Expr::Str("v".into()),
				Expr::Name("major".into()),
				Expr::Str(".".into()),
				Expr::Name("minor".into()),
			])
		);
		round_trips("f\"v{major}.{minor}\"");
	}

	#[test]
	fn printing_and_reading_again_gives_the_same_tree() {
		for src in [
			"1 + 2 * 3",
			"(1 + 2) * 3",
			"a - (b - c)",
			"a - b - c",
			"a ? b : c ? d : e",
			"(a ? b : c) ? d : e",
			"not a and b",
			"not (a and b)",
			"x.y[2].z.as<t>.size",
			"-1 % 3",
			"~(a | b)",
			"(a and b) or c and d",
			"(a < b) == c",
			"_io.pos + _parent.len_body",
			"_root.hdr.magic == [0x1f, 0x8b]",
			"blocks[_index - 1].size",
			"\"a\\nb\" == foo",
			"'it''s'",
			"to_s.substring(0, 4)",
			"sizeof<block> + bitsizeof<flags>",
			"animal::cat.to_i",
			"(a + b).as<u4>",
			"x.as<some::inner>",
			"f\"{a}\"",
			"1.5e-3 * 2.0",
			"[1, 2, 3]",
		] {
			round_trips(src);
		}
	}

	#[test]
	fn what_the_compiler_rejects_is_rejected_here() {
		for src in ["1 *", "42/", "(1 + 5", "\"hello ", "foo == 1 && bar == 2", "^AHEM", "really("] {
			assert!(parse(src).is_err(), "{src} should not parse");
		}
	}

	#[test]
	fn a_type_reference_carries_its_arguments() {
		let (name, args) = parse_type_ref("block").unwrap();
		assert_eq!(name.names, vec!["block".to_string()]);
		assert!(args.is_empty());
		let (name, args) = parse_type_ref("outer::block(1, len - 2)").unwrap();
		assert_eq!(name.to_string(), "outer::block");
		assert_eq!(args.len(), 2);
		assert!(parse_type_ref("really(").is_err());
	}
}
