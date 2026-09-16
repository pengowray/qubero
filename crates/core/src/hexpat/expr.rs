//! The pattern language's expression tree.
//!
//! The precedence the parser applies is the reference's, which is *not* C's:
//! the bitwise operators bind tighter than the comparisons, so `a & 1 == 0`
//! parses as `(a & 1) == 0`. Lowest to highest:
//!
//! ```text
//! ?:   ||   ^^   &&   == !=   < <= > >=   |   ^   &   << >>   + -   * / %   unary + - ! ~
//! ```
//!
//! Two of the levels stop early where a `>` or a `|` belongs to the syntax
//! around the expression rather than to the expression: inside a template
//! argument list a bare `>` closes the list, and inside a `match` case a bare
//! `|` separates alternatives. The parser carries both as flags.
//!
//! [`Display`] writes an expression back out in one canonical form, with the
//! parentheses the tree needs and no others, which is what the conversion
//! report and the error messages show.

use std::fmt;

use super::ast::TypeRef;
use super::lexer::{Lit, Pos};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
	Add,
	Sub,
	Mul,
	Div,
	Rem,
	Shl,
	Shr,
	BitAnd,
	BitOr,
	BitXor,
	Eq,
	Ne,
	Lt,
	Le,
	Gt,
	Ge,
	BoolAnd,
	BoolOr,
	BoolXor,
}

impl BinOp {
	pub fn name(self) -> &'static str {
		use BinOp::*;
		match self {
			Add => "+",
			Sub => "-",
			Mul => "*",
			Div => "/",
			Rem => "%",
			Shl => "<<",
			Shr => ">>",
			BitAnd => "&",
			BitOr => "|",
			BitXor => "^",
			Eq => "==",
			Ne => "!=",
			Lt => "<",
			Le => "<=",
			Gt => ">",
			Ge => ">=",
			BoolAnd => "&&",
			BoolOr => "||",
			BoolXor => "^^",
		}
	}

	/// Binding power, higher binds tighter. Used only to place parentheses.
	pub fn level(self) -> u8 {
		use BinOp::*;
		match self {
			BoolOr => 1,
			BoolXor => 2,
			BoolAnd => 3,
			Eq | Ne => 4,
			Lt | Le | Gt | Ge => 5,
			BitOr => 6,
			BitXor => 7,
			BitAnd => 8,
			Shl | Shr => 9,
			Add | Sub => 10,
			Mul | Div | Rem => 11,
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
	/// `+x`, which the reference reads as `0 + x`.
	Plus,
	Neg,
	Not,
	BitNot,
}

impl UnOp {
	pub fn name(self) -> &'static str {
		match self {
			UnOp::Plus => "+",
			UnOp::Neg => "-",
			UnOp::Not => "!",
			UnOp::BitNot => "~",
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeOp {
	SizeOf,
	AddressOf,
	TypeNameOf,
}

impl TypeOp {
	pub fn name(self) -> &'static str {
		match self {
			TypeOp::SizeOf => "sizeof",
			TypeOp::AddressOf => "addressof",
			TypeOp::TypeNameOf => "typenameof",
		}
	}
}

/// What `sizeof`, `addressof` and `typenameof` were applied to.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeOpArg {
	Value(Box<Expr>),
	Type(Box<TypeRef>),
	/// `sizeof($)` and `addressof($)`, which mean the whole space.
	Space,
}

/// One step of a path such as `parent.header.entries[i].tag`.
#[derive(Debug, Clone, PartialEq)]
pub enum PathSeg {
	Name(String),
	/// `parent`, which climbs one struct.
	Parent,
	/// `this`, the struct being read.
	This,
	/// `$`, the position in the enclosing space.
	Dollar,
	Null,
	/// `[i]`, which indexes the segment before it.
	Index(Box<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
	pub kind: ExprKind,
	pub pos: Pos,
}

impl Expr {
	pub fn new(kind: ExprKind, pos: Pos) -> Self {
		Expr { kind, pos }
	}
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
	Lit(Lit),
	/// An rvalue: one or more path segments, read left to right.
	Path(Vec<PathSeg>),
	/// `E::Label`, where `E` is a declared type.
	ScopeRes { ty: String, name: String },
	Call { path: String, args: Vec<Expr> },
	Unary { op: UnOp, value: Box<Expr> },
	Binary { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr> },
	Ternary { cond: Box<Expr>, then: Box<Expr>, otherwise: Box<Expr> },
	TypeOp { op: TypeOp, arg: TypeOpArg },
	/// `u32(x)`, which converts the value.
	Cast { ty: Box<TypeRef>, value: Box<Expr> },
	/// `x as T`, which reads the same bytes as another type.
	Reinterpret { value: Box<Expr>, ty: Box<TypeRef> },
}

impl fmt::Display for Expr {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		self.write(f, 0)
	}
}

impl Expr {
	fn write(&self, f: &mut fmt::Formatter<'_>, outer: u8) -> fmt::Result {
		match &self.kind {
			ExprKind::Lit(lit) => write_lit(f, lit),
			ExprKind::Path(segments) => write_path(f, segments),
			ExprKind::ScopeRes { ty, name } => write!(f, "{ty}::{name}"),
			ExprKind::Call { path, args } => {
				write!(f, "{path}(")?;
				for (index, arg) in args.iter().enumerate() {
					if index > 0 {
						f.write_str(", ")?;
					}
					arg.write(f, 0)?;
				}
				f.write_str(")")
			}
			ExprKind::Unary { op, value } => {
				f.write_str(op.name())?;
				value.write(f, 12)
			}
			ExprKind::Binary { op, lhs, rhs } => {
				let level = op.level();
				let wrap = level < outer;
				if wrap {
					f.write_str("(")?;
				}
				lhs.write(f, level)?;
				write!(f, " {} ", op.name())?;
				rhs.write(f, level + 1)?;
				if wrap {
					f.write_str(")")?;
				}
				Ok(())
			}
			ExprKind::Ternary { cond, then, otherwise } => {
				let wrap = outer > 0;
				if wrap {
					f.write_str("(")?;
				}
				cond.write(f, 1)?;
				f.write_str(" ? ")?;
				then.write(f, 1)?;
				f.write_str(" : ")?;
				otherwise.write(f, 1)?;
				if wrap {
					f.write_str(")")?;
				}
				Ok(())
			}
			ExprKind::TypeOp { op, arg } => match arg {
				TypeOpArg::Value(value) => {
					write!(f, "{}(", op.name())?;
					value.write(f, 0)?;
					f.write_str(")")
				}
				TypeOpArg::Type(ty) => write!(f, "{}({})", op.name(), type_name(ty)),
				TypeOpArg::Space => write!(f, "{}($)", op.name()),
			},
			ExprKind::Cast { ty, value } => {
				write!(f, "{}(", type_name(ty))?;
				value.write(f, 0)?;
				f.write_str(")")
			}
			ExprKind::Reinterpret { value, ty } => {
				value.write(f, 12)?;
				write!(f, " as {}", type_name(ty))
			}
		}
	}
}

fn write_lit(f: &mut fmt::Formatter<'_>, lit: &Lit) -> fmt::Result {
	match lit {
		Lit::Int { value, .. } => write!(f, "{value}"),
		Lit::Float(value) => write!(f, "{value}"),
		Lit::Bool(value) => write!(f, "{value}"),
		Lit::Str(text) => write!(f, "{text:?}"),
	}
}

fn write_path(f: &mut fmt::Formatter<'_>, segments: &[PathSeg]) -> fmt::Result {
	let mut first = true;
	for segment in segments {
		match segment {
			PathSeg::Index(index) => {
				f.write_str("[")?;
				write!(f, "{index}")?;
				f.write_str("]")?;
				continue;
			}
			_ => {
				if !first {
					f.write_str(".")?;
				}
			}
		}
		first = false;
		match segment {
			PathSeg::Name(name) => f.write_str(name)?,
			PathSeg::Parent => f.write_str("parent")?,
			PathSeg::This => f.write_str("this")?,
			PathSeg::Dollar => f.write_str("$")?,
			PathSeg::Null => f.write_str("null")?,
			PathSeg::Index(_) => unreachable!(),
		}
	}
	Ok(())
}

/// A type as the report writes it, which is the source form without the
/// template arguments' own spacing.
pub fn type_name(ty: &TypeRef) -> String {
	use super::ast::{Endian, TemplateArg, TypeKind};
	let mut text = String::new();
	if ty.reference {
		text.push_str("ref ");
	}
	match ty.endian {
		Some(Endian::Little) => text.push_str("le "),
		Some(Endian::Big) => text.push_str("be "),
		None => {}
	}
	match &ty.kind {
		TypeKind::Builtin(builtin) => text.push_str(builtin.name()),
		TypeKind::Named { path, args } => {
			text.push_str(path);
			if !args.is_empty() {
				text.push('<');
				for (index, arg) in args.iter().enumerate() {
					if index > 0 {
						text.push_str(", ");
					}
					match arg {
						TemplateArg::Type(inner) => text.push_str(&type_name(inner)),
						TemplateArg::Value(value) => text.push_str(&value.to_string()),
					}
				}
				text.push('>');
			}
		}
	}
	text
}
