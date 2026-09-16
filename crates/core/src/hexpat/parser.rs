//! Recursive descent over the token stream, with a precedence climb for
//! expressions.
//!
//! The grammar is the reference implementation's, function for function, so
//! that a pattern it rejects is rejected here with the same message on the same
//! line. Three of its habits are load-bearing and easy to miss:
//!
//! * The parser is **type aware**. A name in type position has to have been
//!   declared, and how a `<...>` argument list is read depends on whether each
//!   declared parameter is a type or a value. That is why parsing resolves
//!   `#include` and `import` through a [`Resolver`] rather than leaving them
//!   for lowering: without `std/mem.pat` there is no `std::mem::Section` and
//!   half the corpus stops parsing.
//! * `<` and `>` are single tokens. `<=`, `>=`, `<<` and `>>` are composed
//!   here, and inside a template argument list the shift and relation levels
//!   stop at a bare `>` so that `Foo<Bar<u8>>` closes two lists. A `match`
//!   case stops the bit-or level at `|` for the same reason.
//! * `error()` in the reference reports at the token *before* the cursor and
//!   `errorHere()` at the cursor. "Expected ';'" therefore lands on the line
//!   the statement ended on, not the line after it. [`Parser::error_prev`] and
//!   [`Parser::error_here`] keep that split.
//!
//! The imperative half is parsed with the same grammar and then folded into one
//! [`Statement`] per construct, so nothing is skipped by counting braces.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::ast::*;
use super::expr::{BinOp, Expr, ExprKind, PathSeg, TypeOp, TypeOpArg, UnOp};
use super::lexer::{self, HexpatError, Keyword, Lexed, Lit, Op, Pos, Sep, Tok, Token, ValueType};

/// A file an `#include` or an `import` asked for, once it has been found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
	/// What to call this file in an error message.
	pub name: String,
	pub text: String,
}

/// Where `#include <std/mem.pat>` and `import std.mem;` find their files.
///
/// The path arrives the way the reference passes it on: `.` in an `import` has
/// already become `/`, and an `#include`'s brackets or quotes are stripped. A
/// path with no extension may name either a `.hexpat` or a `.pat`, and a
/// directory means the `pattern` file inside it, exactly as `resolvers.cpp`
/// does it.
pub trait Resolver {
	fn resolve(&self, path: &str) -> Option<Resolved>;
}

/// A resolver with nothing in it, for a pattern that includes nothing.
pub struct NoIncludes;

impl Resolver for NoIncludes {
	fn resolve(&self, _path: &str) -> Option<Resolved> {
		None
	}
}

/// What the parser knows about a declared type.
#[derive(Debug, Clone, PartialEq)]
struct TypeInfo {
	template: Vec<TemplateParam>,
	/// Declared by `using X;` and not yet given a body.
	forward: bool,
}

/// Read a `.hexpat` that needs no includes.
pub fn parse(text: &str) -> Result<Program, HexpatError> {
	parse_with("", text, &NoIncludes, &[])
}

/// Read a `.hexpat`, resolving its includes and imports through `resolver`.
///
/// `defines` are the macros defined before the first line: ImHex defines
/// `__IMHEX__`, the reference's own test runner defines `__PL_UNIT_TESTS__`,
/// and patterns in the corpus read `#ifdef` on both.
pub fn parse_with(file: &str, text: &str, resolver: &dyn Resolver, defines: &[String]) -> Result<Program, HexpatError> {
	let mut session = Session {
		resolver,
		defines: defines.to_vec(),
		imported: HashMap::new(),
		included: HashSet::new(),
		loading: HashSet::new(),
	};
	let (program, _types) = parse_unit(file, text, &mut session, HashMap::new())?;
	Ok(program)
}

/// The types the host registers under `builtin::`, with how many template
/// arguments each takes. ImHex's decode plugin declares them; the include files
/// in `includes/hex/type/` wrap them, so a pattern that reads JSON or
/// disassembly does not parse without them. Every parameter is a value.
const BUILTIN_TYPES: &[(&str, usize)] = &[
	("builtin::hex::dec::Bjdata", 1),
	("builtin::hex::dec::Bson", 1),
	("builtin::hex::dec::Cbor", 1),
	("builtin::hex::dec::EncodedString", 2),
	("builtin::hex::dec::Instruction", 4),
	("builtin::hex::dec::Json", 1),
	("builtin::hex::dec::Msgpack", 1),
	("builtin::hex::dec::Ubjson", 1),
];

fn builtin_types() -> HashMap<String, TypeInfo> {
	BUILTIN_TYPES
		.iter()
		.map(|(name, count)| {
			let template = (0..*count)
				.map(|index| TemplateParam { name: format!("$param{index}$"), is_type: false, pos: Pos::default() })
				.collect();
			(name.to_string(), TypeInfo { template, forward: false })
		})
		.collect()
}

struct Session<'r> {
	resolver: &'r dyn Resolver,
	defines: Vec<String>,
	/// Files an `import` has parsed, which are separate units and so cacheable.
	imported: HashMap<String, (Arc<Program>, HashMap<String, TypeInfo>)>,
	/// Files an `#include` has already spliced in, which happens once.
	included: HashSet<String>,
	/// Files being parsed right now, so a cycle stops instead of recurring.
	loading: HashSet<String>,
}

impl Session<'_> {
	/// Find and parse one imported file. `None` means the resolver has no such
	/// path. An `import` is its own translation unit, so its result is cached.
	fn import_file(&mut self, path: &str) -> Option<Res<(Arc<Program>, HashMap<String, TypeInfo>)>> {
		let found = self.resolver.resolve(path)?;
		if let Some(cached) = self.imported.get(&found.name) {
			return Some(Ok(cached.clone()));
		}
		if self.included.contains(&found.name) || !self.loading.insert(found.name.clone()) {
			// Already spliced in by an `#include`, or a cycle. The reference's
			// once-guards check each other the same way.
			return Some(Ok((Arc::new(empty_program(&found.name)), HashMap::new())));
		}
		// An imported file is its own translation unit, so it starts with an
		// empty include guard of its own.
		let outer = std::mem::take(&mut self.included);
		let parsed = parse_unit(&found.name, &found.text, self, HashMap::new());
		self.included = outer;
		self.loading.remove(&found.name);
		let (program, types) = match parsed {
			Ok(value) => value,
			Err(error) => return Some(Err(error)),
		};
		let entry = (Arc::new(program), types);
		self.imported.insert(found.name, entry.clone());
		Some(Ok(entry))
	}

	/// Find and parse one `#include`d file.
	///
	/// An `#include` splices tokens, so the file is parsed with the types the
	/// includer has so far and hands its whole table back: `#include <a>` then
	/// `#include <b>` lets `b` name a type `a` declared, which is what the
	/// GoldBox patterns rely on. That makes the result context-dependent, so it
	/// is not cached; only the once-guard stops the work repeating.
	fn include_file(&mut self, path: &str, types: HashMap<String, TypeInfo>) -> Option<Res<(Arc<Program>, HashMap<String, TypeInfo>)>> {
		let found = self.resolver.resolve(path)?;
		if self.included.contains(&found.name) || self.imported.contains_key(&found.name) || !self.loading.insert(found.name.clone()) {
			return Some(Ok((Arc::new(empty_program(&found.name)), types)));
		}
		let parsed = parse_unit(&found.name, &found.text, self, types);
		self.loading.remove(&found.name);
		self.included.insert(found.name.clone());
		let (program, types) = match parsed {
			Ok(value) => value,
			Err(error) => return Some(Err(error)),
		};
		Some(Ok((Arc::new(program), types)))
	}
}

fn empty_program(name: &str) -> Program {
	Program { file: name.to_string(), decls: Vec::new(), pragmas: Vec::new(), docs: Vec::new() }
}

fn parse_unit(
	file: &str,
	text: &str,
	session: &mut Session<'_>,
	types: HashMap<String, TypeInfo>,
) -> Res<(Program, HashMap<String, TypeInfo>)> {
	let defines = session.defines.clone();
	let lexed = lexer::lex(file, text, &defines)?;
	let mut parser = Parser::new(file, text, lexed, session);
	if types.is_empty() {
		parser.types = builtin_types();
	} else {
		parser.types = types;
	}
	parser.run()?;
	let types = std::mem::take(&mut parser.types);
	let program = Program {
		file: file.to_string(),
		decls: std::mem::take(&mut parser.decls),
		pragmas: std::mem::take(&mut parser.pragmas),
		docs: std::mem::take(&mut parser.global_docs),
	};
	Ok((program, types))
}

type Res<T> = Result<T, HexpatError>;

struct Parser<'a, 'r> {
	file: &'a str,
	src: &'a str,
	tokens: Vec<Token>,
	at: usize,
	/// Doc comment text by the index of the token it sits in front of.
	doc_before: HashMap<usize, String>,
	global_docs: Vec<String>,
	pragmas: Vec<lexer::Pragma>,
	includes: Vec<lexer::Include>,
	decls: Vec<Decl>,
	types: HashMap<String, TypeInfo>,
	/// The enclosing namespaces, innermost last.
	namespace: Vec<String>,
	/// The template parameters of the type being declared, if any.
	templates: Vec<Vec<TemplateParam>>,
	session: &'a mut Session<'r>,
}

impl<'a, 'r> Parser<'a, 'r> {
	fn new(file: &'a str, src: &'a str, lexed: Lexed, session: &'a mut Session<'r>) -> Self {
		let mut doc_before = HashMap::new();
		let mut global_docs = Vec::new();
		for doc in lexed.docs {
			if doc.global {
				global_docs.push(doc.text);
			} else {
				doc_before.entry(doc.before).or_insert(doc.text);
			}
		}
		Parser {
			file,
			src,
			tokens: lexed.tokens,
			at: 0,
			doc_before,
			global_docs,
			pragmas: lexed.pragmas,
			includes: lexed.includes,
			decls: Vec::new(),
			types: HashMap::new(),
			namespace: Vec::new(),
			templates: Vec::new(),
			session,
		}
	}

	fn run(&mut self) -> Res<()> {
		// An `#include` splices its file's tokens in where it stands, and in
		// this corpus that is always above everything else, so resolving them
		// first gives the same type table.
		for include in std::mem::take(&mut self.includes) {
			let decl = self.include(&include)?;
			self.decls.push(decl);
		}

		while !self.check(&Tok::End) {
			let statements = self.statements()?;
			self.decls.extend(statements);
		}
		Ok(())
	}

	fn include(&mut self, include: &lexer::Include) -> Res<Decl> {
		let types = std::mem::take(&mut self.types);
		match self.session.include_file(&include.path, types) {
			None => Err(HexpatError::new(self.file, include.pos, format!("Could not find file {}", include.path))),
			Some(Err(error)) => Err(error),
			Some(Ok((program, types))) => {
				self.types = types;
				Ok(Decl::Include { path: include.path.clone(), program: Some(program), pos: include.pos })
			}
		}
	}

	/* ---------------------------------------------------------------- */
	/* Tokens                                                            */
	/* ---------------------------------------------------------------- */

	fn tok(&self, ahead: usize) -> &Tok {
		self.tokens.get(self.at + ahead).map_or(&Tok::End, |token| &token.tok)
	}

	fn pos(&self) -> Pos {
		self.tokens.get(self.at).map_or(Pos::default(), |token| token.pos)
	}

	fn prev_pos(&self) -> Pos {
		let index = self.at.saturating_sub(1);
		self.tokens.get(index).map_or(Pos::default(), |token| token.pos)
	}

	fn next(&mut self) {
		if self.at < self.tokens.len() {
			self.at += 1;
		}
	}

	fn check(&self, tok: &Tok) -> bool {
		self.tok(0) == tok
	}

	fn check_at(&self, ahead: usize, tok: &Tok) -> bool {
		self.tok(ahead) == tok
	}

	fn eat(&mut self, tok: &Tok) -> bool {
		if self.check(tok) {
			self.next();
			return true;
		}
		false
	}

	fn sep(&mut self, sep: Sep) -> bool {
		self.eat(&Tok::Sep(sep))
	}

	fn op(&mut self, op: Op) -> bool {
		self.eat(&Tok::Op(op))
	}

	fn kw(&mut self, kw: Keyword) -> bool {
		self.eat(&Tok::Keyword(kw))
	}

	fn is_sep(&self, ahead: usize, sep: Sep) -> bool {
		self.check_at(ahead, &Tok::Sep(sep))
	}

	fn is_op(&self, ahead: usize, op: Op) -> bool {
		self.check_at(ahead, &Tok::Op(op))
	}

	fn is_kw(&self, ahead: usize, kw: Keyword) -> bool {
		self.check_at(ahead, &Tok::Keyword(kw))
	}

	fn is_ident(&self, ahead: usize) -> bool {
		matches!(self.tok(ahead), Tok::Ident(_))
	}

	/// A built-in type token, which is what the reference's `ValueType::Any`
	/// matches: every named type except `padding`.
	fn is_any_type(&self, ahead: usize) -> bool {
		matches!(self.tok(ahead), Tok::Type(ty) if *ty != ValueType::Padding)
	}

	fn ident(&mut self) -> Res<String> {
		match self.tok(0).clone() {
			Tok::Ident(name) => {
				self.next();
				Ok(name)
			}
			other => Err(self.error_here(format!("Expected identifier, got {}.", other.describe()))),
		}
	}

	/// The reference's `error()`: the token before the cursor.
	fn error_prev(&self, message: impl Into<String>) -> HexpatError {
		HexpatError::new(self.file, self.prev_pos(), message)
	}

	/// The reference's `errorHere()`: the cursor, or the token before it at
	/// the end of the program.
	fn error_here(&self, message: impl Into<String>) -> HexpatError {
		let pos = if self.check(&Tok::End) { self.prev_pos() } else { self.pos() };
		HexpatError::new(self.file, pos, message)
	}

	fn got(&self) -> String {
		self.tok(0).describe()
	}

	fn expect_sep(&mut self, want: Sep, message: &str) -> Res<()> {
		if self.sep(want) {
			return Ok(());
		}
		Err(self.error_prev(format!("{message}, got {}.", self.got())))
	}

	fn expect_op(&mut self, want: Op, message: &str) -> Res<()> {
		if self.op(want) {
			return Ok(());
		}
		Err(self.error_prev(format!("{message}, got {}.", self.got())))
	}

	fn doc_here(&self, index: usize) -> Option<String> {
		self.doc_before.get(&index).cloned()
	}

	/// The source text between two token indices, which is what a [`Statement`]
	/// keeps so lowering can show what it could not express.
	fn span_text(&self, from: usize, to: usize) -> (u32, u32, String) {
		let start = self.tokens.get(from).map_or(0, |token| token.pos.byte) as usize;
		let end = self.tokens.get(to).map_or(self.src.len() as u32, |token| token.pos.byte) as usize;
		if start >= end || end > self.src.len() || !self.src.is_char_boundary(start) || !self.src.is_char_boundary(end) {
			return (start as u32, start as u32, String::new());
		}
		let text = self.src[start..end].trim_end().trim_end_matches(';').trim_end().to_string();
		(start as u32, (start + text.len()) as u32, text)
	}

	fn statement(&mut self, kind: StatementKind, from: usize) -> Statement {
		let pos = self.tokens.get(from).map_or(Pos::default(), |token| token.pos);
		let (start, end, text) = self.span_text(from, self.at);
		Statement { kind, pos, span: (start, end), text }
	}

	/* ---------------------------------------------------------------- */
	/* Names and the type table                                          */
	/* ---------------------------------------------------------------- */

	/// The names a bare `name` could resolve to, least qualified first, which
	/// is the order the reference tries them in.
	fn candidates(&self, name: &str) -> Vec<String> {
		let mut result = vec![name.to_string()];
		let mut prefix = String::new();
		for part in &self.namespace {
			prefix.push_str(part);
			prefix.push_str("::");
			result.push(format!("{prefix}{name}"));
		}
		result
	}

	/// The name a declaration made here gets: the most qualified one.
	fn qualified(&self, name: &str) -> String {
		self.candidates(name).pop().unwrap_or_else(|| name.to_string())
	}

	fn lookup(&self, name: &str) -> Option<(String, TypeInfo)> {
		// A type parameter of the type being declared shadows everything.
		if let Some(params) = self.templates.first() {
			if params.iter().any(|param| param.is_type && param.name == name) {
				return Some((name.to_string(), TypeInfo { template: Vec::new(), forward: false }));
			}
		}
		for candidate in self.candidates(name) {
			if let Some(info) = self.types.get(&candidate) {
				return Some((candidate, info.clone()));
			}
		}
		None
	}

	fn add_type(&mut self, name: &str, template: Vec<TemplateParam>) -> Res<String> {
		let full = self.qualified(name);
		if let Some(existing) = self.types.get_mut(&full) {
			if existing.forward {
				existing.forward = false;
				existing.template = template;
				return Ok(full);
			}
			return Err(self.error_prev(format!("Type with name '{full}' has already been declared.")));
		}
		self.types.insert(full.clone(), TypeInfo { template, forward: false });
		Ok(full)
	}

	fn set_template(&mut self, full: &str, template: Vec<TemplateParam>) {
		if let Some(info) = self.types.get_mut(full) {
			info.template = template;
		}
	}

	/// `A::B::C`, consuming the chain. The cursor must be on an identifier.
	fn namespace_path(&mut self) -> Res<String> {
		let mut name = self.ident()?;
		while self.is_op(0, Op::ScopeResolution) && self.is_ident(1) {
			self.next();
			name.push_str("::");
			name.push_str(&self.ident()?);
		}
		Ok(name)
	}

	/* ---------------------------------------------------------------- */
	/* Expressions                                                       */
	/* ---------------------------------------------------------------- */

	fn expr(&mut self) -> Res<Expr> {
		self.ternary(false, false)
	}

	fn ternary(&mut self, in_template: bool, in_match: bool) -> Res<Expr> {
		let mut node = self.bool_or(in_template, in_match)?;
		while self.op(Op::Ternary) {
			let pos = node.pos;
			let then = self.bool_or(in_template, in_match)?;
			self.expect_op(Op::Colon, "Expected ':' after ternary condition")?;
			let otherwise = self.bool_or(in_template, in_match)?;
			node = Expr::new(
				ExprKind::Ternary { cond: Box::new(node), then: Box::new(then), otherwise: Box::new(otherwise) },
				pos,
			);
		}
		Ok(node)
	}

	fn bool_or(&mut self, in_template: bool, in_match: bool) -> Res<Expr> {
		let mut node = self.bool_xor(in_template, in_match)?;
		while self.op(Op::BoolOr) {
			let rhs = self.bool_xor(in_template, in_match)?;
			node = binary(BinOp::BoolOr, node, rhs);
		}
		Ok(node)
	}

	fn bool_xor(&mut self, in_template: bool, in_match: bool) -> Res<Expr> {
		let mut node = self.bool_and(in_template, in_match)?;
		while self.op(Op::BoolXor) {
			let rhs = self.bool_and(in_template, in_match)?;
			node = binary(BinOp::BoolXor, node, rhs);
		}
		Ok(node)
	}

	fn bool_and(&mut self, in_template: bool, in_match: bool) -> Res<Expr> {
		let mut node = self.equality(in_template, in_match)?;
		while self.op(Op::BoolAnd) {
			let rhs = self.equality(in_template, in_match)?;
			node = binary(BinOp::BoolAnd, node, rhs);
		}
		Ok(node)
	}

	fn equality(&mut self, in_template: bool, in_match: bool) -> Res<Expr> {
		let mut node = self.relation(in_template, in_match)?;
		loop {
			let op = if self.op(Op::BoolEqual) {
				BinOp::Eq
			} else if self.op(Op::BoolNotEqual) {
				BinOp::Ne
			} else {
				break;
			};
			let rhs = self.relation(in_template, in_match)?;
			node = binary(op, node, rhs);
		}
		Ok(node)
	}

	/// `< <= > >=`, composed from single tokens. Inside a template argument
	/// list a bare `>` belongs to the list, so this level stops there.
	fn relation(&mut self, in_template: bool, in_match: bool) -> Res<Expr> {
		let mut node = self.bit_or(in_template, in_match)?;
		if in_template && self.is_op(0, Op::Greater) {
			return Ok(node);
		}
		loop {
			let op = if self.is_op(0, Op::Greater) && self.is_op(1, Op::Assign) {
				self.next();
				self.next();
				BinOp::Ge
			} else if self.is_op(0, Op::Less) && self.is_op(1, Op::Assign) {
				self.next();
				self.next();
				BinOp::Le
			} else if self.is_op(0, Op::Greater) {
				self.next();
				BinOp::Gt
			} else if self.is_op(0, Op::Less) {
				self.next();
				BinOp::Lt
			} else {
				break;
			};
			let rhs = self.bit_or(in_template, in_match)?;
			node = binary(op, node, rhs);
		}
		Ok(node)
	}

	fn bit_or(&mut self, in_template: bool, in_match: bool) -> Res<Expr> {
		let mut node = self.bit_xor(in_template)?;
		if in_match && self.is_op(0, Op::BitOr) {
			return Ok(node);
		}
		while self.op(Op::BitOr) {
			let rhs = self.bit_xor(in_template)?;
			node = binary(BinOp::BitOr, node, rhs);
		}
		Ok(node)
	}

	fn bit_xor(&mut self, in_template: bool) -> Res<Expr> {
		let mut node = self.bit_and(in_template)?;
		while self.op(Op::BitXor) {
			let rhs = self.bit_and(in_template)?;
			node = binary(BinOp::BitXor, node, rhs);
		}
		Ok(node)
	}

	fn bit_and(&mut self, in_template: bool) -> Res<Expr> {
		let mut node = self.shift(in_template)?;
		while self.op(Op::BitAnd) {
			let rhs = self.shift(in_template)?;
			node = binary(BinOp::BitAnd, node, rhs);
		}
		Ok(node)
	}

	fn shift(&mut self, in_template: bool) -> Res<Expr> {
		let mut node = self.additive()?;
		if in_template && self.is_op(0, Op::Greater) {
			return Ok(node);
		}
		loop {
			let op = if self.is_op(0, Op::Greater) && self.is_op(1, Op::Greater) {
				self.next();
				self.next();
				BinOp::Shr
			} else if self.is_op(0, Op::Less) && self.is_op(1, Op::Less) {
				self.next();
				self.next();
				BinOp::Shl
			} else {
				break;
			};
			let rhs = self.additive()?;
			node = binary(op, node, rhs);
		}
		Ok(node)
	}

	fn additive(&mut self) -> Res<Expr> {
		let mut node = self.multiplicative()?;
		loop {
			let op = if self.op(Op::Plus) {
				BinOp::Add
			} else if self.op(Op::Minus) {
				BinOp::Sub
			} else {
				break;
			};
			let rhs = self.multiplicative()?;
			node = binary(op, node, rhs);
		}
		Ok(node)
	}

	fn multiplicative(&mut self) -> Res<Expr> {
		let mut node = self.unary()?;
		loop {
			let op = if self.op(Op::Star) {
				BinOp::Mul
			} else if self.op(Op::Slash) {
				BinOp::Div
			} else if self.op(Op::Percent) {
				BinOp::Rem
			} else {
				break;
			};
			let rhs = self.unary()?;
			node = binary(op, node, rhs);
		}
		Ok(node)
	}

	fn unary(&mut self) -> Res<Expr> {
		let pos = self.pos();
		let op = if self.op(Op::Plus) {
			Some(UnOp::Plus)
		} else if self.op(Op::Minus) {
			Some(UnOp::Neg)
		} else if self.op(Op::BoolNot) {
			Some(UnOp::Not)
		} else if self.op(Op::BitNot) {
			Some(UnOp::BitNot)
		} else {
			None
		};
		if let Some(op) = op {
			let value = self.unary()?;
			return Ok(Expr::new(ExprKind::Unary { op, value: Box::new(value) }, pos));
		}

		// A string literal is only ever read here, which is why `"MZ" == magic`
		// works and `f("a" + "b")` reads the way it does.
		if let Tok::Lit(Lit::Str(text)) = self.tok(0).clone() {
			self.next();
			let literal = Expr::new(ExprKind::Lit(Lit::Str(text)), pos);
			if self.is_ident(0) {
				return self.user_defined_literal(literal);
			}
			return Ok(literal);
		}

		self.cast()
	}

	/// `u32(x)`, a built-in type applied like a function.
	fn cast(&mut self) -> Res<Expr> {
		if self.is_kw(0, Keyword::BigEndian) || self.is_kw(0, Keyword::LittleEndian) || self.is_any_type(0) {
			let pos = self.pos();
			let ty = self.parse_type()?;
			if !matches!(ty.kind, TypeKind::Builtin(_)) {
				return Err(self.error_prev("Cannot use non-built-in type in cast expression."));
			}
			if !self.is_sep(0, Sep::LeftParen) {
				return Err(self.error_prev(format!("Expected '(' after type cast, got {}.", self.got())));
			}
			let value = self.factor()?;
			return Ok(Expr::new(ExprKind::Cast { ty: Box::new(ty), value: Box::new(value) }, pos));
		}
		self.reinterpret()
	}

	/// `x as T`, the same bytes read as another type.
	fn reinterpret(&mut self) -> Res<Expr> {
		let value = self.factor()?;
		if self.kw(Keyword::As) {
			let pos = value.pos;
			let ty = self.parse_type()?;
			return Ok(Expr::new(ExprKind::Reinterpret { value: Box::new(value), ty: Box::new(ty) }, pos));
		}
		Ok(value)
	}

	fn factor(&mut self) -> Res<Expr> {
		let pos = self.pos();

		if let Tok::Lit(lit) = self.tok(0).clone() {
			if !matches!(lit, Lit::Str(_)) {
				self.next();
				let literal = Expr::new(ExprKind::Lit(lit), pos);
				if self.is_ident(0) {
					return self.user_defined_literal(literal);
				}
				return Ok(literal);
			}
		}

		// The reference reaches its unary operators again here; it only
		// happens inside a cast's parentheses, and it reads the same either way.
		let op = if self.op(Op::Plus) {
			Some(UnOp::Plus)
		} else if self.op(Op::Minus) {
			Some(UnOp::Neg)
		} else if self.op(Op::BoolNot) {
			Some(UnOp::Not)
		} else if self.op(Op::BitNot) {
			Some(UnOp::BitNot)
		} else {
			None
		};
		if let Some(op) = op {
			let value = self.expr()?;
			return Ok(Expr::new(ExprKind::Unary { op, value: Box::new(value) }, pos));
		}

		if self.sep(Sep::LeftParen) {
			let node = self.expr()?;
			if !self.sep(Sep::RightParen) {
				return Err(self.error_prev("Mismatched '(' in mathematical expression."));
			}
			return Ok(node);
		}

		if self.is_ident(0) {
			let save = self.at;
			self.namespace_path()?;
			let is_call = self.is_sep(0, Sep::LeftParen);
			self.at = save;
			if is_call {
				return self.call_expr();
			}
			if self.is_op(1, Op::ScopeResolution) {
				return self.scope_resolution();
			}
			return self.rvalue();
		}

		if self.is_kw(0, Keyword::Parent) || self.is_kw(0, Keyword::This) || self.is_op(0, Op::Dollar) || self.is_kw(0, Keyword::Null) {
			return self.rvalue();
		}

		let type_op = match self.tok(0) {
			Tok::Op(Op::AddressOf) => Some(TypeOp::AddressOf),
			Tok::Op(Op::SizeOf) => Some(TypeOp::SizeOf),
			Tok::Op(Op::TypeNameOf) => Some(TypeOp::TypeNameOf),
			_ => None,
		};
		if let Some(type_op) = type_op {
			if self.is_sep(1, Sep::LeftParen) {
				self.next();
				self.next();
				return self.type_operator(type_op, pos);
			}
		}

		Err(self.error_prev(format!("Expected value, got {}.", self.got())))
	}

	fn type_operator(&mut self, op: TypeOp, pos: Pos) -> Res<Expr> {
		let arg = if self.is_ident(0) {
			let save = self.at;
			let mut found = None;
			if op != TypeOp::AddressOf {
				let name = self.namespace_path()?;
				if let Some((full, info)) = self.lookup(&name) {
					let args = self.template_args(&full, &info)?;
					let ty = TypeRef { endian: None, reference: false, kind: TypeKind::Named { path: full, args }, pos };
					found = Some(TypeOpArg::Type(Box::new(ty)));
				}
			}
			match found {
				Some(arg) => arg,
				None => {
					self.at = save;
					TypeOpArg::Value(Box::new(self.rvalue()?))
				}
			}
		} else if self.is_kw(0, Keyword::Parent) || self.is_kw(0, Keyword::This) {
			TypeOpArg::Value(Box::new(self.rvalue()?))
		} else if op == TypeOp::SizeOf && self.is_any_type(0) {
			let Tok::Type(ty) = self.tok(0).clone() else { unreachable!() };
			self.next();
			let bytes = u128::from(ty.bits().unwrap_or(0)) / 8;
			let node = Expr::new(ExprKind::Lit(Lit::Int { value: bytes, unsigned: true }), pos);
			self.expect_sep(Sep::RightParen, "Mismatched '(' of type operator expression")?;
			return Ok(node);
		} else if op == TypeOp::TypeNameOf && self.is_any_type(0) {
			let Tok::Type(ty) = self.tok(0).clone() else { unreachable!() };
			self.next();
			let node = Expr::new(ExprKind::Lit(Lit::Str(ty.name().to_string())), pos);
			self.expect_sep(Sep::RightParen, "Mismatched '(' of type operator expression")?;
			return Ok(node);
		} else if self.op(Op::Dollar) {
			TypeOpArg::Space
		} else {
			let message = match op {
				TypeOp::SizeOf => "Expected rvalue, type or '$' operator.",
				TypeOp::AddressOf => "Expected rvalue or '$' operator.",
				TypeOp::TypeNameOf => "Expected rvalue or type.",
			};
			return Err(self.error_prev(message));
		};

		if !self.sep(Sep::RightParen) {
			return Err(self.error_prev("Mismatched '(' of type operator expression."));
		}
		Ok(Expr::new(ExprKind::TypeOp { op, arg }, pos))
	}

	fn user_defined_literal(&mut self, literal: Expr) -> Res<Expr> {
		let pos = literal.pos;
		let name = self.namespace_path()?;
		if !name.starts_with('_') {
			return Err(self.error_prev(
				"Invalid use of user defined literal. User defined literals need to start with a underscore character (_).",
			));
		}
		let mut args = vec![literal];
		if self.sep(Sep::LeftParen) {
			args.extend(self.parameters()?);
		}
		Ok(Expr::new(ExprKind::Call { path: name, args }, pos))
	}

	fn call_expr(&mut self) -> Res<Expr> {
		let pos = self.pos();
		let path = self.namespace_path()?;
		if !self.sep(Sep::LeftParen) {
			return Err(self.error_prev(format!("Expected '(' after function name, got {}.", self.got())));
		}
		let args = self.parameters()?;
		Ok(Expr::new(ExprKind::Call { path, args }, pos))
	}

	/// An argument list, with the opening `(` already consumed.
	fn parameters(&mut self) -> Res<Vec<Expr>> {
		let mut args = Vec::new();
		while !self.sep(Sep::RightParen) {
			args.push(self.expr()?);
			if self.is_sep(0, Sep::Comma) && self.is_sep(1, Sep::RightParen) {
				self.next();
				return Err(self.error_prev(format!("Expected ')' at end of parameter list, got {}.", self.got())));
			}
			if self.sep(Sep::RightParen) {
				break;
			}
			if !self.sep(Sep::Comma) {
				return Err(self.error_prev(format!("Expected ',' in-between parameters, got {}.", self.got())));
			}
		}
		Ok(args)
	}

	fn scope_resolution(&mut self) -> Res<Expr> {
		let pos = self.pos();
		let mut name = self.ident()?;
		let mut type_name = String::new();
		loop {
			type_name.push_str(&name);
			if self.is_op(0, Op::ScopeResolution) && self.is_ident(1) {
				self.next();
				name = self.ident()?;
				if self.is_op(0, Op::ScopeResolution) && self.is_ident(1) {
					type_name.push_str("::");
					continue;
				}
				return match self.lookup(&type_name) {
					Some((full, _)) => Ok(Expr::new(ExprKind::ScopeRes { ty: full, name }, pos)),
					None => Err(self.error_prev("No namespace with this name found.")),
				};
			}
			break;
		}
		Err(self.error_prev("Invalid scope resolution."))
	}

	/// A path such as `parent.header.entries[i].tag`, `this`, `$` or `null`.
	fn rvalue(&mut self) -> Res<Expr> {
		let pos = self.pos();
		let mut path = Vec::new();
		loop {
			match self.tok(0).clone() {
				Tok::Ident(name) => {
					self.next();
					path.push(PathSeg::Name(name));
				}
				Tok::Keyword(Keyword::Parent) => {
					self.next();
					path.push(PathSeg::Parent);
				}
				Tok::Keyword(Keyword::This) => {
					self.next();
					path.push(PathSeg::This);
				}
				Tok::Op(Op::Dollar) => {
					self.next();
					path.push(PathSeg::Dollar);
				}
				Tok::Keyword(Keyword::Null) => {
					self.next();
					path.push(PathSeg::Null);
				}
				other => return Err(self.error_here(format!("Expected value, got {}.", other.describe()))),
			}

			// `[[` after a name is an attribute, not an index.
			if self.is_sep(0, Sep::LeftBracket) && !self.is_sep(1, Sep::LeftBracket) {
				self.next();
				let index = self.expr()?;
				self.expect_sep(Sep::RightBracket, "Expected ']' at end of array indexing")?;
				path.push(PathSeg::Index(Box::new(index)));
			}

			if self.sep(Sep::Dot) {
				if self.is_ident(0) || self.is_kw(0, Keyword::Parent) {
					continue;
				}
				return Err(self.error_prev("Invalid member access, expected variable identifier or parent keyword."));
			}
			break;
		}
		Ok(Expr::new(ExprKind::Path(path), pos))
	}

	/* ---------------------------------------------------------------- */
	/* Types                                                             */
	/* ---------------------------------------------------------------- */

	fn parse_type(&mut self) -> Res<TypeRef> {
		let pos = self.pos();
		let reference = self.kw(Keyword::Reference);
		let endian = if self.kw(Keyword::LittleEndian) {
			Some(Endian::Little)
		} else if self.kw(Keyword::BigEndian) {
			Some(Endian::Big)
		} else {
			None
		};

		let kind = if self.is_ident(0) {
			self.custom_type()?
		} else if self.is_any_type(0) {
			let Tok::Type(ty) = self.tok(0).clone() else { unreachable!() };
			self.next();
			TypeKind::Builtin(ty)
		} else {
			return Err(self.error_prev(format!(
				"Invalid type. Expected built-in type or custom type name, got {}.",
				self.got()
			)));
		};

		Ok(TypeRef { endian, reference, kind, pos })
	}

	fn custom_type(&mut self) -> Res<TypeKind> {
		let name = self.namespace_path()?;
		let Some((full, info)) = self.lookup(&name) else {
			return Err(self.error_prev(format!("Type {name} has not been declared yet.")));
		};
		let args = self.template_args(&full, &info)?;
		Ok(TypeKind::Named { path: full, args })
	}

	/// The `<...>` after a use of a templated type. A type with no template
	/// parameters takes none, and one with them may not be written without.
	fn template_args(&mut self, name: &str, info: &TypeInfo) -> Res<Vec<TemplateArg>> {
		if info.template.is_empty() {
			return Ok(Vec::new());
		}
		let _ = name;
		if !self.op(Op::Less) {
			return Err(self.error_prev("Cannot use template type without template parameters."));
		}
		let mut args = Vec::new();
		let expected = info.template.len();
		loop {
			if args.len() >= expected {
				return Err(self.error_prev(format!(
					"Provided more template parameters than expected. Type only has {expected} parameters"
				)));
			}
			if info.template[args.len()].is_type {
				args.push(TemplateArg::Type(self.parse_type()?));
			} else {
				args.push(TemplateArg::Value(self.ternary(true, false)?));
			}
			if !self.sep(Sep::Comma) {
				break;
			}
		}
		if args.len() < expected {
			return Err(self.error_prev(format!("Not enough template parameters provided, expected {expected} parameters.")));
		}
		self.expect_op(Op::Greater, "Expected '>' to close template list")?;
		Ok(args)
	}

	/// The `<T, auto N>` of a declaration head.
	fn template_list(&mut self) -> Res<Vec<TemplateParam>> {
		if !self.op(Op::Less) {
			return Ok(Vec::new());
		}
		let mut params = Vec::new();
		loop {
			let pos = self.pos();
			if self.is_ident(0) {
				let name = self.ident()?;
				params.push(TemplateParam { name, is_type: true, pos });
			} else if self.check(&Tok::Type(ValueType::Auto)) && self.is_ident(1) {
				self.next();
				let name = self.ident()?;
				params.push(TemplateParam { name, is_type: false, pos });
			} else {
				return Err(self.error_prev(format!("Expected identifier for template type, got {}.", self.got())));
			}
			if !self.sep(Sep::Comma) {
				break;
			}
		}
		self.expect_op(Op::Greater, "Expected '>' after template declaration")?;
		Ok(params)
	}

	/* ---------------------------------------------------------------- */
	/* Attributes                                                        */
	/* ---------------------------------------------------------------- */

	/// `[[a, b("x"), hex::visualize("y", z)]]`, with `[[` already consumed.
	fn attributes(&mut self) -> Res<Vec<Attribute>> {
		let mut attrs = Vec::new();
		loop {
			if !self.is_ident(0) {
				return Err(self.error_prev(format!("Expected attribute instruction name, got {}", self.got())));
			}
			let pos = self.pos();
			let path = self.namespace_path()?;
			let mut args = Vec::new();
			if self.sep(Sep::LeftParen) {
				loop {
					args.push(self.expr()?);
					if !self.sep(Sep::Comma) {
						break;
					}
				}
				if !self.sep(Sep::RightParen) {
					return Err(self.error_prev(format!("Expected ')', got {}", self.got())));
				}
			}
			attrs.push(Attribute { path, args, pos });
			if !self.sep(Sep::Comma) {
				break;
			}
		}
		if !(self.sep(Sep::RightBracket) && self.sep(Sep::RightBracket)) {
			return Err(self.error_prev(format!("Expected ']]' after attribute, got {}.", self.got())));
		}
		Ok(attrs)
	}

	/// Take a trailing `[[..]]`, if one is there.
	fn trailing_attributes(&mut self) -> Res<Vec<Attribute>> {
		if self.is_sep(0, Sep::LeftBracket) && self.is_sep(1, Sep::LeftBracket) {
			self.next();
			self.next();
			return self.attributes();
		}
		Ok(Vec::new())
	}

	fn semicolon(&mut self) -> Res<()> {
		if !self.sep(Sep::Semicolon) {
			return Err(self.error_prev(format!("Expected ';' at end of statement, got {}.", self.got())));
		}
		while self.sep(Sep::Semicolon) {}
		Ok(())
	}

	/* ---------------------------------------------------------------- */
	/* Assignments, which are all imperative                             */
	/* ---------------------------------------------------------------- */

	/// `<op>=` at `ahead`, and how many tokens it spans.
	fn compound_op_at(&self, ahead: usize) -> Option<usize> {
		let single = matches!(
			self.tok(ahead),
			Tok::Op(Op::Plus | Op::Minus | Op::Star | Op::Slash | Op::Percent | Op::BitOr | Op::BitAnd | Op::BitXor)
		);
		if single && self.is_op(ahead + 1, Op::Assign) {
			return Some(2);
		}
		for op in [Op::Less, Op::Greater] {
			if self.is_op(ahead, op) && self.is_op(ahead + 1, op) && self.is_op(ahead + 2, Op::Assign) {
				return Some(3);
			}
		}
		None
	}

	/// Whether an assignment starts here, given which keywords may lead one.
	fn assignment_ahead(&self, allow_parent: bool, allow_this: bool) -> bool {
		if self.is_op(0, Op::Dollar) && (self.is_op(1, Op::Assign) || self.compound_op_at(1).is_some()) {
			return true;
		}
		if self.is_ident(0) && (self.is_op(1, Op::Assign) || self.compound_op_at(1).is_some()) {
			return true;
		}
		let leads = self.is_ident(0)
			|| (allow_parent && self.is_kw(0, Keyword::Parent))
			|| (allow_this && self.is_kw(0, Keyword::This));
		leads && (self.is_sep(1, Sep::Dot) || (self.is_sep(1, Sep::LeftBracket) && !self.is_sep(2, Sep::LeftBracket)))
	}

	/// Read one assignment, whatever shape it has, as an opaque statement.
	fn assignment(&mut self) -> Res<Statement> {
		let from = self.at;
		if self.is_op(0, Op::Dollar) && (self.is_op(1, Op::Assign) || self.compound_op_at(1).is_some()) {
			self.next();
		} else if self.is_ident(0) && (self.is_op(1, Op::Assign) || self.compound_op_at(1).is_some()) {
			self.next();
		} else {
			self.rvalue()?;
		}
		if let Some(width) = self.compound_op_at(0) {
			for _ in 0..width {
				self.next();
			}
		} else if !self.op(Op::Assign) {
			return Err(self.error_here(format!("Expected value after '=' in variable assignment, got {}.", self.got())));
		}
		self.expr()?;
		Ok(self.statement(StatementKind::Assign, from))
	}

	/* ---------------------------------------------------------------- */
	/* Members of a struct or union                                      */
	/* ---------------------------------------------------------------- */

	fn member(&mut self) -> Res<Member> {
		let from = self.at;
		let pos = self.pos();
		let doc = self.doc_here(from);

		if self.assignment_ahead(true, true) {
			let statement = self.assignment()?;
			self.trailing_attributes()?;
			self.semicolon()?;
			return Ok(Member::Statement(statement));
		}

		if self.is_kw(0, Keyword::If) {
			self.next();
			let (cond, then, otherwise) = self.conditional_parts(Self::member)?;
			return Ok(Member::If { cond, then, otherwise, pos });
		}
		if self.is_kw(0, Keyword::Match) {
			self.next();
			let (scrutinee, arms) = self.match_parts(Self::member)?;
			return Ok(Member::Match { scrutinee, arms, pos });
		}
		if self.is_kw(0, Keyword::Try) && self.is_sep(1, Sep::LeftBrace) {
			self.next();
			self.next();
			let (body, catch) = self.try_parts(Self::member)?;
			return Ok(Member::Try { body, catch, pos });
		}

		if self.is_kw(0, Keyword::Return) || self.is_kw(0, Keyword::Break) || self.is_kw(0, Keyword::Continue) {
			let statement = self.control_flow()?;
			self.trailing_attributes()?;
			self.semicolon()?;
			return Ok(Member::Statement(statement));
		}

		if self.is_kw(0, Keyword::Const)
			|| self.is_kw(0, Keyword::BigEndian)
			|| self.is_kw(0, Keyword::LittleEndian)
			|| self.is_any_type(0)
			|| self.is_ident(0)
		{
			if self.is_ident(0) {
				let save = self.at;
				self.namespace_path()?;
				let is_call = self.is_sep(0, Sep::LeftParen);
				self.at = save;
				if is_call {
					let call = self.call_expr()?;
					let ExprKind::Call { path, args } = call.kind else { unreachable!() };
					self.trailing_attributes()?;
					self.semicolon()?;
					return Ok(Member::Call { path, args, pos });
				}
			}

			let constant = self.kw(Keyword::Const);
			let ty = self.parse_type()?;
			let mut field = self.variable_after_type(ty, constant, pos)?;
			field.doc = doc;
			field.attrs = self.trailing_attributes()?;
			self.semicolon()?;
			return Ok(Member::Field(field));
		}

		if self.check(&Tok::Type(ValueType::Padding)) && self.is_sep(1, Sep::LeftBracket) {
			self.next();
			self.next();
			let size = self.array_size(false)?;
			let attrs = self.trailing_attributes()?;
			self.semicolon()?;
			return Ok(Member::Padding { size, attrs, pos });
		}

		if matches!(self.tok(0), Tok::Keyword(_)) {
			return Err(self.error_here(format!("Invalid {} found in custom type.", self.got())));
		}
		Err(self.error_here("Invalid struct member definition."))
	}

	/// The part of a declaration after the type: the name, any array or
	/// pointer shape, and any placement or initialiser.
	fn variable_after_type(&mut self, ty: TypeRef, constant: bool, pos: Pos) -> Res<Field> {
		if self.is_ident(0) && self.is_sep(1, Sep::LeftBracket) && !self.is_sep(2, Sep::LeftBracket) {
			let name = self.ident()?;
			self.next();
			return self.array_variable(ty, name, constant, pos);
		}
		if self.is_op(0, Op::Star) && self.is_ident(1) && self.is_op(2, Op::Colon) {
			self.next();
			let name = self.ident()?;
			self.next();
			return self.pointer_variable(ty, name, None, pos);
		}
		if self.is_op(0, Op::Star) && self.is_ident(1) && self.is_sep(2, Sep::LeftBracket) {
			self.next();
			let name = self.ident()?;
			self.next();
			let size = self.array_size(true)?;
			self.expect_op(Op::Colon, "Expected ':' after pointer definition")?;
			return self.pointer_variable(ty, name, Some(size), pos);
		}
		let name = if self.is_ident(0) { self.ident()? } else { String::new() };
		self.plain_variable(ty, name, constant, pos)
	}

	fn plain_variable(&mut self, ty: TypeRef, name: String, constant: bool, pos: Pos) -> Res<Field> {
		let mut field = Field {
			ty,
			names: vec![name.clone()],
			array: None,
			pointer: None,
			placement: None,
			section: None,
			width: None,
			init: None,
			init_list: Vec::new(),
			kind: FieldKind::Normal,
			constant,
			attrs: Vec::new(),
			doc: None,
			pos,
		};

		if self.is_sep(0, Sep::Comma) {
			while self.sep(Sep::Comma) {
				if self.is_ident(0) {
					field.names.push(self.ident()?);
				}
			}
			return Ok(field);
		}

		if self.op(Op::At) {
			if constant {
				return Err(self.error_prev("Cannot mark placed variable as 'const'."));
			}
			field.placement = Some(self.expr()?);
			if self.kw(Keyword::In) {
				field.section = Some(self.expr()?);
			}
			return Ok(field);
		}

		if self.op(Op::Assign) {
			field.kind = FieldKind::Local;
			field.init = Some(self.expr()?);
			return Ok(field);
		}

		if constant {
			return Err(self.error_prev("Cannot mark placed variable as 'const'."));
		}
		Ok(field)
	}

	/// `[n]`, `[]` or `[while(c)]`, with the `[` already consumed. `padding[]`
	/// is not a shape the reference has, so an empty one is only allowed where
	/// the caller says so.
	fn array_size(&mut self, allow_empty: bool) -> Res<ArraySize> {
		if allow_empty && self.sep(Sep::RightBracket) {
			return Ok(ArraySize::Unbounded);
		}
		let size = if self.is_kw(0, Keyword::While) && self.is_sep(1, Sep::LeftParen) {
			self.next();
			self.next();
			let cond = self.expr()?;
			self.expect_sep(Sep::RightParen, "Expected ')' after while head")?;
			ArraySize::While(cond)
		} else {
			ArraySize::Count(self.expr()?)
		};
		self.expect_sep(Sep::RightBracket, "Expected ']' at end of array declaration")?;
		Ok(size)
	}

	fn array_variable(&mut self, ty: TypeRef, name: String, constant: bool, pos: Pos) -> Res<Field> {
		let size = self.array_size(true)?;
		let mut field = Field {
			ty,
			names: vec![name],
			array: Some(size),
			pointer: None,
			placement: None,
			section: None,
			width: None,
			init: None,
			init_list: Vec::new(),
			kind: FieldKind::Normal,
			constant,
			attrs: Vec::new(),
			doc: None,
			pos,
		};

		if self.op(Op::At) {
			if constant {
				return Err(self.error_prev("Cannot mark placed variable as 'const'."));
			}
			field.placement = Some(self.expr()?);
			if self.kw(Keyword::In) {
				field.section = Some(self.expr()?);
			}
			return Ok(field);
		}

		if self.op(Op::Assign) {
			field.kind = FieldKind::Local;
			field.init_list = self.array_init()?;
			return Ok(field);
		}

		if constant {
			return Err(self.error_prev("Cannot mark placed variable as 'const'."));
		}
		Ok(field)
	}

	/// `= { a, b, c }` after an array declaration.
	fn array_init(&mut self) -> Res<Vec<Expr>> {
		if !self.sep(Sep::LeftBrace) {
			return Err(self.error_prev(format!("Expected '{{' after array assignment, got {}.", self.got())));
		}
		let mut values = Vec::new();
		loop {
			if self.sep(Sep::RightBrace) {
				break;
			}
			values.push(self.expr()?);
			if self.sep(Sep::Comma) {
				if self.is_sep(0, Sep::RightBrace) {
					self.next();
					return Err(self.error_prev(format!("Expected value after ',' in array assignment, got {}.", self.got())));
				}
			} else if self.sep(Sep::RightBrace) {
				break;
			} else {
				return Err(self.error_prev(format!("Expected ',' or '}}' in array assignment, got {}.", self.got())));
			}
		}
		Ok(values)
	}

	fn pointer_variable(&mut self, ty: TypeRef, name: String, array: Option<ArraySize>, pos: Pos) -> Res<Field> {
		let size_type = self.parse_type()?;
		let mut field = Field {
			ty,
			names: vec![name],
			array,
			pointer: Some(size_type),
			placement: None,
			section: None,
			width: None,
			init: None,
			init_list: Vec::new(),
			kind: FieldKind::Normal,
			constant: false,
			attrs: Vec::new(),
			doc: None,
			pos,
		};
		if self.op(Op::At) {
			field.placement = Some(self.expr()?);
			if self.kw(Keyword::In) {
				field.section = Some(self.expr()?);
			}
		}
		Ok(field)
	}

	fn control_flow(&mut self) -> Res<Statement> {
		let from = self.at;
		let kind = if self.kw(Keyword::Return) {
			StatementKind::Return
		} else if self.kw(Keyword::Break) {
			StatementKind::Break
		} else if self.kw(Keyword::Continue) {
			StatementKind::Continue
		} else {
			return Err(self.error_prev("Invalid control flow statement."));
		};
		if self.is_sep(0, Sep::Semicolon) {
			return Ok(self.statement(kind, from));
		}
		if kind == StatementKind::Return {
			self.expr()?;
			return Ok(self.statement(kind, from));
		}
		Err(self.error_prev("Return value can only be passed to a 'return' statement."))
	}

	/* ---------------------------------------------------------------- */
	/* if / match / try, shared by members, bitfield entries and bodies   */
	/* ---------------------------------------------------------------- */

	/// A `{ .. }` list of entries, or one entry on its own.
	fn statement_body<T>(&mut self, item: fn(&mut Self) -> Res<T>) -> Res<Vec<T>> {
		if self.sep(Sep::LeftBrace) {
			let mut body = Vec::new();
			while !self.sep(Sep::RightBrace) {
				body.push(item(self)?);
			}
			return Ok(body);
		}
		Ok(vec![item(self)?])
	}

	/// `if (c) .. [else ..]`, with `if` already consumed.
	fn conditional_parts<T>(&mut self, item: fn(&mut Self) -> Res<T>) -> Res<(Expr, Vec<T>, Vec<T>)> {
		if !self.sep(Sep::LeftParen) {
			return Err(self.error_prev(format!("Expected '(' after 'if', got {}.", self.got())));
		}
		let cond = self.expr()?;
		self.expect_sep(Sep::RightParen, "Expected ')' after if head")?;
		let then = self.statement_body(item)?;
		let otherwise = if self.kw(Keyword::Else) { self.statement_body(item)? } else { Vec::new() };
		Ok((cond, then, otherwise))
	}

	/// `try { .. } [catch { .. }]`, with `try {` already consumed.
	fn try_parts<T>(&mut self, item: fn(&mut Self) -> Res<T>) -> Res<(Vec<T>, Vec<T>)> {
		let mut tried = Vec::new();
		while !self.sep(Sep::RightBrace) {
			tried.push(item(self)?);
		}
		let mut caught = Vec::new();
		if self.kw(Keyword::Catch) {
			if !self.sep(Sep::LeftBrace) {
				return Err(self.error_prev(format!("Expected '{{' after catch, got {}.", self.got())));
			}
			while !self.sep(Sep::RightBrace) {
				caught.push(item(self)?);
			}
		}
		Ok((tried, caught))
	}

	/// `match (a, b) { (1, _): .. }`, with `match` already consumed.
	fn match_parts<T>(&mut self, item: fn(&mut Self) -> Res<T>) -> Res<(Vec<Expr>, Vec<MatchArm<T>>)> {
		if !self.sep(Sep::LeftParen) {
			return Err(self.error_prev(format!("Expected '(' after 'match', got {}.", self.got())));
		}
		let scrutinee = self.parameters()?;
		if !self.sep(Sep::LeftBrace) {
			return Err(self.error_prev(format!("Expected '{{' after match head, got {}.", self.got())));
		}

		let mut arms = Vec::new();
		while !self.sep(Sep::RightBrace) {
			let arm_pos = self.pos();
			if !self.sep(Sep::LeftParen) {
				return Err(self.error_prev(format!("Expected '(', got {}.", self.got())));
			}
			let patterns = self.case_patterns(scrutinee.len())?;
			self.expect_op(Op::Colon, "Expected ':' after case condition")?;
			let body = self.statement_body(item)?;
			arms.push(MatchArm { patterns, body, pos: arm_pos });
			if self.sep(Sep::RightBrace) {
				break;
			}
		}
		Ok((scrutinee, arms))
	}

	fn case_patterns(&mut self, arity: usize) -> Res<Vec<CasePattern>> {
		let mut patterns = Vec::new();
		while !self.sep(Sep::RightParen) {
			if patterns.len() >= arity {
				return Err(self.error_prev("Size of case parameters bigger than size of match condition."));
			}
			if self.kw(Keyword::Underscore) {
				patterns.push(CasePattern::Wildcard);
			} else {
				let mut values = Vec::new();
				loop {
					let first = self.ternary(false, true)?;
					if self.is_sep(0, Sep::Dot) && self.is_sep(1, Sep::Dot) && self.is_sep(2, Sep::Dot) {
						self.next();
						self.next();
						self.next();
						let last = self.ternary(false, true)?;
						values.push(CaseValue::Range(first, last));
					} else {
						values.push(CaseValue::One(first));
					}
					if !self.op(Op::BitOr) {
						break;
					}
				}
				patterns.push(CasePattern::Values(values));
			}

			if self.is_sep(0, Sep::Comma) && self.is_sep(1, Sep::RightParen) {
				self.next();
				return Err(self.error_prev(format!("Expected ')' at end of parameter list, got {}.", self.got())));
			}
			if self.sep(Sep::RightParen) {
				break;
			}
			if !self.sep(Sep::Comma) {
				return Err(self.error_prev(format!("Expected ',' in-between parameters, got {}.", self.got())));
			}
		}
		if patterns.len() != arity {
			return Err(self.error_prev("Size of case parameters smaller than size of match condition."));
		}
		Ok(patterns)
	}

	/* ---------------------------------------------------------------- */
	/* Bitfields                                                         */
	/* ---------------------------------------------------------------- */

	fn bit_entry(&mut self) -> Res<BitEntry> {
		let from = self.at;
		let pos = self.pos();
		let doc = self.doc_here(from);

		if self.assignment_ahead(true, false) {
			let statement = self.assignment()?;
			self.trailing_attributes()?;
			self.semicolon()?;
			return Ok(BitEntry::Statement(statement));
		}

		// `[unsigned] name : n`, `signed name : n`, `padding : n`.
		let unsigned_lead = self.is_kw(0, Keyword::Unsigned);
		let lead = usize::from(unsigned_lead);
		if self.is_ident(lead) && self.is_op(lead + 1, Op::Colon) {
			if unsigned_lead {
				self.next();
			}
			let name = self.ident()?;
			self.next();
			let width = self.expr()?;
			let attrs = self.trailing_attributes()?;
			self.semicolon()?;
			return Ok(BitEntry::Bits { name, sign: BitSign::Unsigned, width, attrs, doc, pos });
		}
		if self.is_kw(0, Keyword::Signed) && self.is_ident(1) && self.is_op(2, Op::Colon) {
			self.next();
			let name = self.ident()?;
			self.next();
			let width = self.expr()?;
			let attrs = self.trailing_attributes()?;
			self.semicolon()?;
			return Ok(BitEntry::Bits { name, sign: BitSign::Signed, width, attrs, doc, pos });
		}
		if self.check(&Tok::Type(ValueType::Padding)) && self.is_op(1, Op::Colon) {
			self.next();
			self.next();
			let width = self.expr()?;
			let attrs = self.trailing_attributes()?;
			self.semicolon()?;
			return Ok(BitEntry::Bits { name: "padding".to_string(), sign: BitSign::Padding, width, attrs, doc, pos });
		}

		if self.is_ident(0) || self.is_any_type(0) {
			let ty = if self.is_any_type(0) {
				let Tok::Type(builtin) = self.tok(0).clone() else { unreachable!() };
				self.next();
				TypeRef { endian: None, reference: false, kind: TypeKind::Builtin(builtin), pos }
			} else {
				let save = self.at;
				let name = self.namespace_path()?;
				if self.is_sep(0, Sep::LeftParen) {
					self.at = save;
					let call = self.call_expr()?;
					let ExprKind::Call { path, args } = call.kind else { unreachable!() };
					self.trailing_attributes()?;
					self.semicolon()?;
					return Ok(BitEntry::Call { path, args, pos });
				}
				let Some((full, info)) = self.lookup(&name) else {
					return Err(self.error_prev(format!(
						"Expected a variable name followed by ':', a function call or a bitfield type name, got '{name}'."
					)));
				};
				let args = self.template_args(&full, &info)?;
				TypeRef { endian: None, reference: false, kind: TypeKind::Named { path: full, args }, pos }
			};

			if self.is_ident(0) && self.is_sep(1, Sep::LeftBracket) && !self.is_sep(2, Sep::LeftBracket) {
				let name = self.ident()?;
				self.next();
				let size = if self.is_kw(0, Keyword::While) && self.is_sep(1, Sep::LeftParen) {
					self.next();
					self.next();
					let cond = self.expr()?;
					self.expect_sep(Sep::RightParen, "Expected ')' after while head")?;
					ArraySize::While(cond)
				} else {
					ArraySize::Count(self.expr()?)
				};
				self.expect_sep(Sep::RightBracket, "Expected ']' at end of array declaration")?;
				let mut field = blank_field(ty, name, pos);
				field.array = Some(size);
				field.doc = doc;
				field.attrs = self.trailing_attributes()?;
				self.semicolon()?;
				return Ok(BitEntry::Typed(field));
			}

			if self.is_ident(0) {
				let name = self.ident()?;
				if self.is_op(0, Op::At) {
					return Err(self.error_here("Placement syntax is invalid within bitfields."));
				}
				let mut field = if self.op(Op::Colon) {
					let mut field = blank_field(ty, name, pos);
					field.width = Some(self.expr()?);
					field
				} else {
					self.plain_variable(ty, name, false, pos)?
				};
				field.doc = doc;
				field.attrs = self.trailing_attributes()?;
				self.semicolon()?;
				return Ok(BitEntry::Typed(field));
			}

			return Err(self.error_prev(format!("Expected a variable name, got {}.", self.got())));
		}

		if self.is_kw(0, Keyword::If) {
			self.next();
			let (cond, then, otherwise) = self.conditional_parts(Self::bit_entry)?;
			return Ok(BitEntry::If { cond, then, otherwise, pos });
		}
		if self.is_kw(0, Keyword::Match) {
			self.next();
			let (scrutinee, arms) = self.match_parts(Self::bit_entry)?;
			return Ok(BitEntry::Match { scrutinee, arms, pos });
		}
		if self.is_kw(0, Keyword::Try) && self.is_sep(1, Sep::LeftBrace) {
			self.next();
			self.next();
			let (body, catch) = self.try_parts(Self::bit_entry)?;
			return Ok(BitEntry::Try { body, catch, pos });
		}
		if self.is_kw(0, Keyword::Return) || self.is_kw(0, Keyword::Break) || self.is_kw(0, Keyword::Continue) {
			let statement = self.control_flow()?;
			self.trailing_attributes()?;
			self.semicolon()?;
			return Ok(BitEntry::Statement(statement));
		}

		Err(self.error_here("Invalid bitfield member definition."))
	}

	/* ---------------------------------------------------------------- */
	/* Function bodies, which stay opaque                                */
	/* ---------------------------------------------------------------- */

	fn one_statement(&mut self) -> Res<Statement> {
		self.function_statement(true)
	}

	fn function_statement(&mut self, needs_semicolon: bool) -> Res<Statement> {
		let from = self.at;
		let mut needs_semicolon = needs_semicolon;

		let kind = if self.assignment_ahead(true, false) {
			self.assignment()?;
			StatementKind::Assign
		} else if self.is_kw(0, Keyword::Return) || self.is_kw(0, Keyword::Break) || self.is_kw(0, Keyword::Continue) {
			self.control_flow()?.kind
		} else if self.is_kw(0, Keyword::If) {
			self.next();
			needs_semicolon = false;
			self.conditional_parts(Self::one_statement)?;
			StatementKind::If
		} else if self.is_kw(0, Keyword::Match) {
			self.next();
			needs_semicolon = false;
			self.match_parts(Self::one_statement)?;
			StatementKind::Match
		} else if self.is_kw(0, Keyword::Try) && self.is_sep(1, Sep::LeftBrace) {
			self.next();
			self.next();
			needs_semicolon = false;
			self.try_parts(Self::one_statement)?;
			StatementKind::Try
		} else if self.is_kw(0, Keyword::While) && self.is_sep(1, Sep::LeftParen) {
			self.next();
			self.next();
			needs_semicolon = false;
			let _cond = self.expr()?;
			self.expect_sep(Sep::RightParen, "Expected ')' at end of while head")?;
			self.statement_body(Self::one_statement)?;
			StatementKind::While
		} else if self.is_kw(0, Keyword::For) && self.is_sep(1, Sep::LeftParen) {
			self.next();
			self.next();
			needs_semicolon = false;
			self.function_statement(false)?;
			self.expect_sep(Sep::Comma, "Expected ',' after for loop expression")?;
			self.expr()?;
			self.expect_sep(Sep::Comma, "Expected ',' after for loop expression")?;
			self.function_statement(false)?;
			self.expect_sep(Sep::RightParen, "Expected ')' at end of for loop head")?;
			self.statement_body(Self::one_statement)?;
			StatementKind::For
		} else if self.is_ident(0) {
			let save = self.at;
			self.namespace_path()?;
			let is_call = self.is_sep(0, Sep::LeftParen);
			self.at = save;
			if is_call {
				self.call_expr()?;
				StatementKind::Call
			} else {
				self.function_variable_decl(false)?;
				StatementKind::Local
			}
		} else if self.is_kw(0, Keyword::BigEndian) || self.is_kw(0, Keyword::LittleEndian) || self.is_any_type(0) {
			self.function_variable_decl(false)?;
			StatementKind::Local
		} else if self.kw(Keyword::Const) {
			self.function_variable_decl(true)?;
			StatementKind::Local
		} else if matches!(self.tok(0), Tok::Keyword(_)) {
			return Err(self.error_here(format!("Invalid {} found in function.", self.got())));
		} else {
			return Err(self.error_here("Invalid function statement."));
		};

		if needs_semicolon {
			self.semicolon()?;
		}
		Ok(self.statement(kind, from))
	}

	fn function_variable_decl(&mut self, constant: bool) -> Res<()> {
		let pos = self.pos();
		let ty = self.parse_type()?;
		if !self.is_ident(0) {
			return Err(self.error_prev(format!("Expected identifier in variable declaration, got {}.", self.got())));
		}
		if self.is_sep(1, Sep::LeftBracket) && !self.is_sep(2, Sep::LeftBracket) {
			let name = self.ident()?;
			self.next();
			self.array_variable(ty, name, constant, pos)?;
			return Ok(());
		}
		let name = self.ident()?;
		self.plain_variable(ty, name, constant, pos)?;
		Ok(())
	}

	/* ---------------------------------------------------------------- */
	/* Declarations                                                      */
	/* ---------------------------------------------------------------- */

	fn statements(&mut self) -> Res<Vec<Decl>> {
		let decls = self.one_statements()?;
		// The reference eats superfluous semicolons at the end of every
		// top-level statement, which is what lets a `while (..) { .. };` carry
		// a semicolon it does not need.
		while self.sep(Sep::Semicolon) {}
		Ok(decls)
	}

	fn one_statements(&mut self) -> Res<Vec<Decl>> {
		let from = self.at;
		let pos = self.pos();
		let doc = self.doc_here(from);

		if self.assignment_ahead(false, false) {
			let statement = self.assignment()?;
			self.trailing_attributes()?;
			self.semicolon()?;
			return Ok(vec![Decl::Statement(statement)]);
		}

		if self.is_kw(0, Keyword::Using) && self.is_ident(1) {
			self.next();
			let name = self.ident()?;
			let decl = self.using_decl(name, doc, pos)?;
			let mut decls = Vec::new();
			if let Some(mut decl) = decl {
				decl.attrs = self.trailing_attributes()?;
				decls.push(Decl::Using(decl));
			} else {
				self.trailing_attributes()?;
			}
			self.semicolon()?;
			return Ok(decls);
		}

		if self.kw(Keyword::Import) {
			let decl = self.import(pos)?;
			self.trailing_attributes()?;
			self.semicolon()?;
			return Ok(vec![decl]);
		}

		if self.is_kw(0, Keyword::Namespace) {
			self.next();
			return Ok(vec![self.namespace_decl(pos)?]);
		}

		if self.is_kw(0, Keyword::Struct) && self.is_ident(1) {
			self.next();
			let name = self.ident()?;
			let mut def = self.struct_decl(name, false, doc, pos)?;
			def.attrs = self.trailing_attributes()?;
			self.semicolon()?;
			return Ok(vec![Decl::Struct(def)]);
		}
		if self.is_kw(0, Keyword::Union) && self.is_ident(1) {
			self.next();
			let name = self.ident()?;
			let mut def = self.struct_decl(name, true, doc, pos)?;
			def.attrs = self.trailing_attributes()?;
			self.semicolon()?;
			return Ok(vec![Decl::Struct(def)]);
		}
		if self.is_kw(0, Keyword::Enum) && self.is_ident(1) {
			self.next();
			let name = self.ident()?;
			let mut def = self.enum_decl(name, doc, pos)?;
			def.attrs = self.trailing_attributes()?;
			self.semicolon()?;
			return Ok(vec![Decl::Enum(def)]);
		}
		if self.is_kw(0, Keyword::Bitfield) && self.is_ident(1) {
			self.next();
			let name = self.ident()?;
			let mut def = self.bitfield_decl(name, doc, pos)?;
			def.attrs = self.trailing_attributes()?;
			self.semicolon()?;
			return Ok(vec![Decl::Bitfield(def)]);
		}
		if self.is_kw(0, Keyword::Function) && self.is_ident(1) {
			self.next();
			let name = self.ident()?;
			let mut def = self.function_decl(name, doc, pos)?;
			def.attrs = self.trailing_attributes()?;
			self.semicolon()?;
			return Ok(vec![Decl::Function(def)]);
		}

		if self.is_kw(0, Keyword::BigEndian) || self.is_kw(0, Keyword::LittleEndian) || self.is_any_type(0) {
			let decl = self.placement(doc, pos)?;
			self.semicolon()?;
			return Ok(vec![decl]);
		}

		if self.is_ident(0) && !self.is_op(1, Op::Assign) && !self.is_sep(1, Sep::Dot) && !self.is_sep(1, Sep::LeftBracket) {
			let save = self.at;
			self.namespace_path()?;
			let is_call = self.is_sep(0, Sep::LeftParen);
			self.at = save;
			if is_call {
				let call = self.call_expr()?;
				let ExprKind::Call { path, args } = call.kind else { unreachable!() };
				self.trailing_attributes()?;
				self.semicolon()?;
				return Ok(vec![Decl::Call { path, args, pos }]);
			}
			let decl = self.placement(doc, pos)?;
			self.semicolon()?;
			return Ok(vec![decl]);
		}

		// Anything else at the top level is an imperative statement, which
		// takes its own semicolon.
		let statement = self.function_statement(true)?;
		self.trailing_attributes()?;
		Ok(vec![Decl::Statement(statement)])
	}

	/// A top-level variable, which is the only thing that reads the file.
	fn placement(&mut self, doc: Option<String>, pos: Pos) -> Res<Decl> {
		let ty = self.parse_type()?;

		if self.is_ident(0) && self.is_sep(1, Sep::LeftBracket) {
			let name = self.ident()?;
			self.next();
			let mut field = self.array_variable(ty, name, false, pos)?;
			field.doc = doc;
			field.attrs = self.trailing_attributes()?;
			return Ok(Decl::Placement(field));
		}
		if self.is_op(0, Op::Star) && self.is_ident(1) && self.is_op(2, Op::Colon) {
			self.next();
			let name = self.ident()?;
			self.next();
			let size_type = self.parse_type()?;
			if !self.op(Op::At) {
				return Err(self.error_prev(format!("Expected '@' after pointer placement, got {}.", self.got())));
			}
			let mut field = blank_field(ty, name, pos);
			field.pointer = Some(size_type);
			field.placement = Some(self.expr()?);
			if self.kw(Keyword::In) {
				field.section = Some(self.expr()?);
			}
			field.doc = doc;
			field.attrs = self.trailing_attributes()?;
			return Ok(Decl::Placement(field));
		}
		if self.is_op(0, Op::Star) && self.is_ident(1) && self.is_sep(2, Sep::LeftBracket) {
			self.next();
			let name = self.ident()?;
			self.next();
			let size = self.array_size(true)?;
			self.expect_op(Op::Colon, "Expected ':' after pointer definition")?;
			let size_type = self.parse_type()?;
			if !self.op(Op::At) {
				return Err(self.error_prev(format!("Expected '@' after pointer placement, got {}.", self.got())));
			}
			let mut field = blank_field(ty, name, pos);
			field.array = Some(size);
			field.pointer = Some(size_type);
			field.placement = Some(self.expr()?);
			if self.kw(Keyword::In) {
				field.section = Some(self.expr()?);
			}
			field.doc = doc;
			field.attrs = self.trailing_attributes()?;
			return Ok(Decl::Placement(field));
		}
		if self.is_ident(0) {
			let name = self.ident()?;
			let mut field = self.variable_placement(ty, name, pos)?;
			field.doc = doc;
			field.attrs = self.trailing_attributes()?;
			return Ok(Decl::Placement(field));
		}

		Err(self.error_here("Invalid placement syntax."))
	}

	fn variable_placement(&mut self, ty: TypeRef, name: String, pos: Pos) -> Res<Field> {
		let mut field = blank_field(ty, name, pos);
		if self.op(Op::At) {
			field.placement = Some(self.expr()?);
			if self.kw(Keyword::In) {
				field.section = Some(self.expr()?);
			}
		} else if self.kw(Keyword::In) {
			field.kind = FieldKind::In;
			if self.op(Op::Assign) {
				field.init = Some(self.expr()?);
			}
		} else if self.kw(Keyword::Out) {
			field.kind = FieldKind::Out;
		} else if self.op(Op::Assign) {
			field.kind = FieldKind::Local;
			field.init = Some(self.expr()?);
		}
		Ok(field)
	}

	fn using_decl(&mut self, name: String, doc: Option<String>, pos: Pos) -> Res<Option<UsingDecl>> {
		let template = self.template_list()?;

		if !self.op(Op::Assign) {
			// A forward declaration, which only reserves the name.
			if self.lookup(&name).is_some() {
				return Ok(None);
			}
			let full = self.qualified(&name);
			self.types.insert(full, TypeInfo { template: template.clone(), forward: true });
			return Ok(Some(UsingDecl { name, template, ty: None, attrs: Vec::new(), doc, pos }));
		}

		let full = self.add_type(&name, template.clone())?;
		self.set_template(&full, template.clone());
		self.templates.push(template.clone());
		let ty = self.parse_type();
		self.templates.pop();
		let ty = ty?;
		Ok(Some(UsingDecl { name: full, template, ty: Some(ty), attrs: Vec::new(), doc, pos }))
	}

	fn struct_decl(&mut self, name: String, union: bool, doc: Option<String>, pos: Pos) -> Res<StructDef> {
		let full = self.add_type(&name, Vec::new())?;
		let template = self.template_list()?;
		self.set_template(&full, template.clone());
		self.templates.push(template.clone());

		let mut inherits = Vec::new();
		if !union && self.op(Op::Colon) {
			loop {
				if self.is_any_type(0) {
					self.templates.pop();
					return Err(self.error_prev("Cannot inherit from built-in type."));
				}
				if !self.is_ident(0) {
					self.templates.pop();
					return Err(self.error_prev(format!("Expected type to inherit from, got {}.", self.got())));
				}
				let kind = match self.custom_type() {
					Ok(kind) => kind,
					Err(error) => {
						self.templates.pop();
						return Err(error);
					}
				};
				inherits.push(TypeRef { endian: None, reference: false, kind, pos });
				if !self.sep(Sep::Comma) {
					break;
				}
			}
		}

		let result = (|parser: &mut Self| -> Res<Vec<Member>> {
			if !parser.sep(Sep::LeftBrace) {
				let what = if union { "union" } else { "struct" };
				return Err(parser.error_prev(format!("Expected '{{' after {what} declaration, got {}.", parser.got())));
			}
			let mut members = Vec::new();
			while !parser.sep(Sep::RightBrace) {
				members.push(parser.member()?);
			}
			Ok(members)
		})(self);
		self.templates.pop();
		let members = result?;

		Ok(StructDef { name: full, union, template, inherits, members, attrs: Vec::new(), doc, pos })
	}

	fn enum_decl(&mut self, name: String, doc: Option<String>, pos: Pos) -> Res<EnumDef> {
		if !self.op(Op::Colon) {
			return Err(self.error_prev(format!("Expected ':' after enum declaration, got {}.", self.got())));
		}
		let ty = self.parse_type()?;
		if ty.endian.is_some() {
			return Err(self.error_prev("Underlying enum type may not have an endian specifier."));
		}
		let full = self.add_type(&name, Vec::new())?;

		if !self.sep(Sep::LeftBrace) {
			return Err(self.error_prev(format!("Expected '{{' after enum declaration, got {}.", self.got())));
		}

		let mut entries = Vec::new();
		while !self.sep(Sep::RightBrace) {
			let entry_pos = self.pos();
			let (entry_name, value) = if self.is_ident(0) && self.is_op(1, Op::Assign) {
				let entry_name = self.ident()?;
				self.next();
				(entry_name, Some(self.expr()?))
			} else if self.is_ident(0) {
				(self.ident()?, None)
			} else {
				return Err(self.error_prev("Invalid enum entry definition."));
			};

			let upto = if self.is_sep(0, Sep::Dot) && self.is_sep(1, Sep::Dot) && self.is_sep(2, Sep::Dot) {
				self.next();
				self.next();
				self.next();
				Some(self.expr()?)
			} else {
				None
			};

			entries.push(EnumEntry { name: entry_name, value, upto, pos: entry_pos });

			if !self.sep(Sep::Comma) {
				if self.sep(Sep::RightBrace) {
					break;
				}
				return Err(self.error_prev(format!("Expected ',' at end of enum entry, got {}.", self.got())));
			}
		}

		Ok(EnumDef { name: full, ty, entries, attrs: Vec::new(), doc, pos })
	}

	fn bitfield_decl(&mut self, name: String, doc: Option<String>, pos: Pos) -> Res<BitfieldDef> {
		let full = self.add_type(&name, Vec::new())?;
		let template = self.template_list()?;
		self.set_template(&full, template.clone());
		self.templates.push(template.clone());

		let result = (|parser: &mut Self| -> Res<Vec<BitEntry>> {
			if !parser.sep(Sep::LeftBrace) {
				return Err(parser.error_prev(format!("Expected '{{' after bitfield declaration, got {}.", parser.got())));
			}
			let mut entries = Vec::new();
			while !parser.sep(Sep::RightBrace) {
				entries.push(parser.bit_entry()?);
				while parser.sep(Sep::Semicolon) {}
			}
			Ok(entries)
		})(self);
		self.templates.pop();
		let entries = result?;

		Ok(BitfieldDef { name: full, template, entries, attrs: Vec::new(), doc, pos })
	}

	fn function_decl(&mut self, name: String, doc: Option<String>, pos: Pos) -> Res<FunctionDef> {
		if !self.sep(Sep::LeftParen) {
			return Err(self.error_prev(format!("Expected '(' after function declaration, got {}.", self.got())));
		}

		let mut params = Vec::new();
		let mut param_pack = None;
		let mut seen_default = false;
		if !self.is_sep(0, Sep::RightParen) {
			loop {
				if self.check(&Tok::Type(ValueType::Auto))
					&& self.is_sep(1, Sep::Dot)
					&& self.is_sep(2, Sep::Dot)
					&& self.is_sep(3, Sep::Dot)
					&& self.is_ident(4)
				{
					for _ in 0..4 {
						self.next();
					}
					param_pack = Some(self.ident()?);
					if self.sep(Sep::Comma) {
						return Err(self.error_prev("Parameter pack can only appear at the end of the parameter list."));
					}
					break;
				}

				let param_pos = self.pos();
				let ty = self.parse_type()?;
				let param_name = if self.is_ident(0) { Some(self.ident()?) } else { None };
				let default = if self.op(Op::Assign) {
					seen_default = true;
					Some(self.expr()?)
				} else {
					if seen_default {
						let shown = param_name.clone().unwrap_or_default();
						return Err(self.error_prev(format!(
							"Expected default argument value for parameter '{shown}', got {}.",
							self.got()
						)));
					}
					None
				};
				params.push(Param { ty, name: param_name, default, pos: param_pos });

				if !self.sep(Sep::Comma) {
					break;
				}
			}
		}

		if !self.sep(Sep::RightParen) {
			return Err(self.error_prev(format!("Expected ')' after parameter list, got {}.", self.got())));
		}
		if !self.sep(Sep::LeftBrace) {
			return Err(self.error_prev(format!("Expected '{{' after function head, got {}.", self.got())));
		}

		let mut body = Vec::new();
		while !self.sep(Sep::RightBrace) {
			body.push(self.function_statement(true)?);
		}

		Ok(FunctionDef { name: self.qualified(&name), params, param_pack, body, attrs: Vec::new(), doc, pos })
	}

	fn namespace_decl(&mut self, pos: Pos) -> Res<Decl> {
		let auto = self.eat(&Tok::Type(ValueType::Auto));
		if !self.is_ident(0) {
			return Err(self.error_prev(format!("Expected identifier after 'namespace', got {}.", self.got())));
		}
		let mut path = Vec::new();
		loop {
			path.push(self.ident()?);
			if self.is_op(0, Op::ScopeResolution) && self.is_ident(1) {
				self.next();
				continue;
			}
			break;
		}

		if !self.sep(Sep::LeftBrace) {
			return Err(self.error_prev(format!("Expected '{{' at beginning of namespace, got {}.", self.got())));
		}

		let depth = self.namespace.len();
		self.namespace.extend(path.iter().cloned());
		let mut decls = Vec::new();
		let result = (|parser: &mut Self| -> Res<()> {
			while !parser.sep(Sep::RightBrace) {
				let inner = parser.statements()?;
				decls.extend(inner);
			}
			Ok(())
		})(self);
		self.namespace.truncate(depth);
		result?;

		Ok(Decl::Namespace(NamespaceDecl { path, auto, decls, pos }))
	}

	fn import(&mut self, pos: Pos) -> Res<Decl> {
		let all = self.op(Op::Star);
		if all && !self.kw(Keyword::From) {
			return Err(self.error_prev("Expected 'from' after import *."));
		}

		let path = if let Tok::Lit(Lit::Str(text)) = self.tok(0).clone() {
			self.next();
			text
		} else if self.is_ident(0) {
			let mut path = self.ident()?;
			while self.is_sep(0, Sep::Dot) && self.is_ident(1) {
				self.next();
				path.push('/');
				path.push_str(&self.ident()?);
			}
			path
		} else {
			return Err(self.error_prev(format!("Expected string or identifier after 'import', got {}.", self.got())));
		};

		if all {
			if !self.kw(Keyword::As) {
				return Err(self.error_prev("Alias name required for import *."));
			}
			if !self.is_ident(0) {
				return Err(self.error_prev(format!("Expected identifier after 'as', got {}.", self.got())));
			}
			let alias = self.ident()?;
			// The whole file becomes one type under the alias; its body is
			// resolved when the pattern runs, not here.
			let full = self.qualified(&alias);
			self.types.insert(full, TypeInfo { template: Vec::new(), forward: false });
			return Ok(Decl::Import(Import { path, alias: Some(alias), all: true, program: None, pos }));
		}

		let alias = if self.kw(Keyword::As) {
			if !self.is_ident(0) {
				return Err(self.error_prev(format!("Expected identifier after 'as', got {}.", self.got())));
			}
			Some(self.namespace_path()?)
		} else {
			None
		};

		let loaded = match self.session.import_file(&path) {
			None => return Err(HexpatError::new(self.file, pos, format!("Failed to resolve import: {path}"))),
			Some(Err(error)) => return Err(error),
			Some(Ok(loaded)) => loaded,
		};
		let (program, types) = loaded;
		for (name, info) in types {
			self.types.entry(name).or_insert(info);
		}

		Ok(Decl::Import(Import { path, alias, all: false, program: Some(program), pos }))
	}
}

fn binary(op: BinOp, lhs: Expr, rhs: Expr) -> Expr {
	let pos = lhs.pos;
	Expr::new(ExprKind::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs) }, pos)
}

fn blank_field(ty: TypeRef, name: String, pos: Pos) -> Field {
	Field {
		ty,
		names: vec![name],
		array: None,
		pointer: None,
		placement: None,
		section: None,
		width: None,
		init: None,
		init_list: Vec::new(),
		kind: FieldKind::Normal,
		constant: false,
		attrs: Vec::new(),
		doc: None,
		pos,
	}
}
