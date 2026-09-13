//! The Kaitai expression language as a tree, and how to write one back out.
//!
//! The node kinds are the compiler's, one for one, so that a reader who knows
//! `Ast.scala` knows this. Two of them are only shapes the parser produces and
//! the lowering refuses (`FloatNum`, `InterpolatedStr`): they are here because
//! a construct the IR cannot express has to be named in the report, and it
//! cannot be named if it was never read.
//!
//! [`Display`] writes an expression back in one canonical form, which is what
//! the report and every error message show. It is precedence-aware: it inserts
//! the parentheses the tree needs and no others, so printing and re-parsing
//! gives the same tree back.

use std::fmt;
use std::fmt::Write as _;

/// A dotted type name as it is written inside `.as<>`, `sizeof<>` and `type:`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeId {
	/// Written with a leading `::`, i.e. resolved from the top-level type.
	pub absolute: bool,
	pub names: Vec<String>,
	/// Written with a trailing `[]`.
	pub is_array: bool,
}

impl TypeId {
	pub fn plain(names: Vec<String>) -> Self {
		TypeId { absolute: false, names, is_array: false }
	}

	/// The last component, which is the type's own name.
	pub fn last(&self) -> &str {
		self.names.last().map_or("", |s| s.as_str())
	}
}

impl fmt::Display for TypeId {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		if self.absolute {
			f.write_str("::")?;
		}
		f.write_str(&self.names.join("::"))?;
		if self.is_array {
			f.write_str("[]")?;
		}
		Ok(())
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoolOp {
	And,
	Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
	Add,
	Sub,
	Mult,
	Div,
	Mod,
	LShift,
	RShift,
	BitOr,
	BitXor,
	BitAnd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
	/// `~`, bitwise.
	Invert,
	/// `not`, boolean.
	Not,
	/// `-`, arithmetic.
	Minus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
	Eq,
	NotEq,
	Lt,
	LtE,
	Gt,
	GtE,
}

/// One expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
	/// `a and b and c`, flattened the way the compiler flattens it: this is one
	/// node with three values, not two nested nodes.
	BoolOp { op: BoolOp, values: Vec<Expr> },
	BinOp { left: Box<Expr>, op: BinOp, right: Box<Expr> },
	UnaryOp { op: UnaryOp, operand: Box<Expr> },
	/// `condition ? if_true : if_false`.
	IfExp { condition: Box<Expr>, if_true: Box<Expr>, if_false: Box<Expr> },
	/// `left == right`. Never chained: `a < b < c` is not an expression.
	Compare { left: Box<Expr>, op: CmpOp, right: Box<Expr> },
	/// `f(a, b)`, which in practice is always a method call on something.
	Call { func: Box<Expr>, args: Vec<Expr> },
	IntNum(i128),
	FloatNum(f64),
	Str(String),
	/// `f"text {expr} text"`, read but not understood.
	InterpolatedStr(Vec<Expr>),
	Bool(bool),
	/// `animal::cat`, or `outer::animal::cat` with `in_type` naming the outer.
	EnumByLabel { enum_name: String, label: String, in_type: TypeId },
	Attribute { value: Box<Expr>, attr: String },
	/// `x.as<some_type>`.
	CastToType { value: Box<Expr>, type_name: TypeId },
	/// `sizeof<some_type>`.
	ByteSizeOfType(TypeId),
	/// `bitsizeof<some_type>`.
	BitSizeOfType(TypeId),
	Subscript { value: Box<Expr>, index: Box<Expr> },
	Name(String),
	List(Vec<Expr>),
}

/// How tightly a form binds, largest binding tightest.
///
/// The numbers are the rungs of the compiler's grammar: `test`, `or_test`,
/// `and_test`, `not_test`, `comparison`, then the binary operators, then the
/// unary ones, then an atom with its trailers. [`Display`] parenthesises a
/// child whose rung is below the one its position accepts, which is the whole
/// of the rule.
mod rung {
	pub const TEST: u8 = 0;
	pub const OR: u8 = 1;
	pub const AND: u8 = 2;
	pub const NOT: u8 = 3;
	pub const COMPARISON: u8 = 4;
	pub const BIT_OR: u8 = 5;
	pub const BIT_XOR: u8 = 6;
	pub const BIT_AND: u8 = 7;
	pub const SHIFT: u8 = 8;
	pub const ARITH: u8 = 9;
	pub const TERM: u8 = 10;
	pub const FACTOR: u8 = 11;
	pub const ATOM: u8 = 12;
}

impl BinOp {
	fn rung(self) -> u8 {
		match self {
			BinOp::BitOr => rung::BIT_OR,
			BinOp::BitXor => rung::BIT_XOR,
			BinOp::BitAnd => rung::BIT_AND,
			BinOp::LShift | BinOp::RShift => rung::SHIFT,
			BinOp::Add | BinOp::Sub => rung::ARITH,
			BinOp::Mult | BinOp::Div | BinOp::Mod => rung::TERM,
		}
	}

	fn symbol(self) -> &'static str {
		match self {
			BinOp::Add => "+",
			BinOp::Sub => "-",
			BinOp::Mult => "*",
			BinOp::Div => "/",
			BinOp::Mod => "%",
			BinOp::LShift => "<<",
			BinOp::RShift => ">>",
			BinOp::BitOr => "|",
			BinOp::BitXor => "^",
			BinOp::BitAnd => "&",
		}
	}
}

impl CmpOp {
	fn symbol(self) -> &'static str {
		match self {
			CmpOp::Eq => "==",
			CmpOp::NotEq => "!=",
			CmpOp::Lt => "<",
			CmpOp::LtE => "<=",
			CmpOp::Gt => ">",
			CmpOp::GtE => ">=",
		}
	}
}

impl Expr {
	fn rung(&self) -> u8 {
		match self {
			Expr::IfExp { .. } => rung::TEST,
			Expr::BoolOp { op: BoolOp::Or, .. } => rung::OR,
			Expr::BoolOp { op: BoolOp::And, .. } => rung::AND,
			Expr::UnaryOp { op: UnaryOp::Not, .. } => rung::NOT,
			Expr::Compare { .. } => rung::COMPARISON,
			Expr::BinOp { op, .. } => op.rung(),
			Expr::UnaryOp { .. } => rung::FACTOR,
			_ => rung::ATOM,
		}
	}

	/// Write this expression where a form of at least `min` is accepted,
	/// wrapping it in parentheses when it binds more loosely than that.
	fn write(&self, f: &mut fmt::Formatter<'_>, min: u8) -> fmt::Result {
		if self.rung() < min {
			f.write_str("(")?;
			self.write(f, 0)?;
			return f.write_str(")");
		}
		match self {
			Expr::IfExp { condition, if_true, if_false } => {
				condition.write(f, rung::OR)?;
				f.write_str(" ? ")?;
				if_true.write(f, rung::TEST)?;
				f.write_str(" : ")?;
				if_false.write(f, rung::TEST)
			}
			Expr::BoolOp { op, values } => {
				// `a and b and c` is one node of three; a nested node of the
				// same operator is a different tree and needs its parentheses
				// back, which asking for the next rung up does.
				let word = match op {
					BoolOp::And => " and ",
					BoolOp::Or => " or ",
				};
				let inner = self.rung() + 1;
				for (i, value) in values.iter().enumerate() {
					if i > 0 {
						f.write_str(word)?;
					}
					value.write(f, inner)?;
				}
				Ok(())
			}
			Expr::Compare { left, op, right } => {
				left.write(f, rung::BIT_OR)?;
				write!(f, " {} ", op.symbol())?;
				right.write(f, rung::BIT_OR)
			}
			Expr::BinOp { left, op, right } => {
				// Left-associative, so only the right side of a same-rung
				// operator needs parentheses.
				left.write(f, op.rung())?;
				write!(f, " {} ", op.symbol())?;
				right.write(f, op.rung() + 1)
			}
			Expr::UnaryOp { op, operand } => {
				match op {
					UnaryOp::Not => {
						f.write_str("not ")?;
						operand.write(f, rung::NOT)
					}
					UnaryOp::Minus => {
						f.write_str("-")?;
						operand.write(f, rung::FACTOR)
					}
					UnaryOp::Invert => {
						f.write_str("~")?;
						operand.write(f, rung::FACTOR)
					}
				}
			}
			Expr::Call { func, args } => {
				func.write(f, rung::ATOM)?;
				f.write_str("(")?;
				for (i, arg) in args.iter().enumerate() {
					if i > 0 {
						f.write_str(", ")?;
					}
					arg.write(f, rung::TEST)?;
				}
				f.write_str(")")
			}
			Expr::Attribute { value, attr } => {
				value.write(f, rung::ATOM)?;
				write!(f, ".{attr}")
			}
			Expr::CastToType { value, type_name } => {
				value.write(f, rung::ATOM)?;
				write!(f, ".as<{type_name}>")
			}
			Expr::Subscript { value, index } => {
				value.write(f, rung::ATOM)?;
				f.write_str("[")?;
				index.write(f, rung::TEST)?;
				f.write_str("]")
			}
			Expr::ByteSizeOfType(t) => write!(f, "sizeof<{t}>"),
			Expr::BitSizeOfType(t) => write!(f, "bitsizeof<{t}>"),
			// A literal is never written negative by the parser (that is a
			// unary minus), so this cannot round-trip a hand-built negative
			// one; the lowering does not build them.
			Expr::IntNum(n) => write!(f, "{n}"),
			Expr::FloatNum(x) => write!(f, "{x:?}"),
			Expr::Str(s) => write_quoted(f, s),
			Expr::InterpolatedStr(parts) => {
				f.write_str("f\"")?;
				for part in parts {
					match part {
						Expr::Str(s) => write_in_quotes(f, s)?,
						other => {
							f.write_str("{")?;
							other.write(f, rung::TEST)?;
							f.write_str("}")?;
						}
					}
				}
				f.write_str("\"")
			}
			Expr::Bool(b) => write!(f, "{b}"),
			Expr::Name(name) => f.write_str(name),
			Expr::EnumByLabel { enum_name, label, in_type } => {
				if in_type.absolute {
					f.write_str("::")?;
				}
				for part in &in_type.names {
					write!(f, "{part}::")?;
				}
				write!(f, "{enum_name}::{label}")
			}
			Expr::List(items) => {
				f.write_str("[")?;
				for (i, item) in items.iter().enumerate() {
					if i > 0 {
						f.write_str(", ")?;
					}
					item.write(f, rung::TEST)?;
				}
				f.write_str("]")
			}
		}
	}

	/// Call `f` on this node and every node under it, outermost first.
	pub fn walk(&self, f: &mut impl FnMut(&Expr)) {
		f(self);
		match self {
			Expr::BoolOp { values, .. } | Expr::List(values) | Expr::InterpolatedStr(values) => {
				for value in values {
					value.walk(f);
				}
			}
			Expr::BinOp { left, right, .. } | Expr::Compare { left, right, .. } => {
				left.walk(f);
				right.walk(f);
			}
			Expr::UnaryOp { operand, .. } => operand.walk(f),
			Expr::IfExp { condition, if_true, if_false } => {
				condition.walk(f);
				if_true.walk(f);
				if_false.walk(f);
			}
			Expr::Call { func, args } => {
				func.walk(f);
				for arg in args {
					arg.walk(f);
				}
			}
			Expr::Attribute { value, .. } | Expr::CastToType { value, .. } => value.walk(f),
			Expr::Subscript { value, index } => {
				value.walk(f);
				index.walk(f);
			}
			Expr::IntNum(_)
			| Expr::FloatNum(_)
			| Expr::Str(_)
			| Expr::Bool(_)
			| Expr::Name(_)
			| Expr::EnumByLabel { .. }
			| Expr::ByteSizeOfType(_)
			| Expr::BitSizeOfType(_) => {}
		}
	}
}

impl fmt::Display for Expr {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		self.write(f, 0)
	}
}

fn write_quoted(f: &mut fmt::Formatter<'_>, s: &str) -> fmt::Result {
	f.write_str("\"")?;
	write_in_quotes(f, s)?;
	f.write_str("\"")
}

/// The body of a double-quoted string: only what the language's own escape
/// table can say, so what comes out reads back as what went in.
fn write_in_quotes(f: &mut fmt::Formatter<'_>, s: &str) -> fmt::Result {
	for c in s.chars() {
		match c {
			'"' => f.write_str("\\\"")?,
			'\\' => f.write_str("\\\\")?,
			'\u{7}' => f.write_str("\\a")?,
			'\u{8}' => f.write_str("\\b")?,
			'\t' => f.write_str("\\t")?,
			'\n' => f.write_str("\\n")?,
			'\u{b}' => f.write_str("\\v")?,
			'\u{c}' => f.write_str("\\f")?,
			'\r' => f.write_str("\\r")?,
			'\u{1b}' => f.write_str("\\e")?,
			c if (c as u32) < 0x20 || c as u32 == 0x7f => write!(f, "\\u{:04x}", c as u32)?,
			c => f.write_char(c)?,
		}
	}
	Ok(())
}
