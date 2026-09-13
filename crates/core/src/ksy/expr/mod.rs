//! The `.ksy` expression language.
//!
//! A `size:`, an `if:`, a `repeat-expr:`, a switch's `switch-on:` and each of
//! its case keys are all written in this language, so a `.ksy` cannot be read
//! without it. The grammar is the Kaitai compiler's, rung for rung; see
//! [`parser`] for the order the rungs come in and [`ast`] for the tree.

pub mod ast;
pub mod lexer;
pub mod parser;

use std::fmt;

pub use ast::{BinOp, BoolOp, CmpOp, Expr, TypeId, UnaryOp};
pub use parser::{parse, parse_list, parse_type_ref};

/// An expression that could not be read, and where in it the reader stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
	/// The expression as written.
	pub source: String,
	/// Where the reader stopped, in bytes from the start of `source`.
	pub at: usize,
	pub message: String,
}

impl ParseError {
	/// Where the reader stopped, counted in characters from 1, which is what a
	/// person reading the expression counts.
	pub fn character(&self) -> usize {
		self.source[..self.at.min(self.source.len())].chars().count() + 1
	}
}

impl fmt::Display for ParseError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "expression `{}`: {} at character {}", self.source, self.message, self.character())
	}
}

impl std::error::Error for ParseError {}
