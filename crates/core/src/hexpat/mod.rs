//! ImHex pattern language (`.hexpat`) in, an AST out.
//!
//! This is the front half of the converter planned in `docs/HANDOVER-imhex.md`:
//! [`lexer`] turns text into tokens and runs the preprocessor, [`parser`] reads
//! those into the [`ast`], and [`expr`] holds the expression language. Lowering
//! that tree to a [`Template`](crate::template::Template) comes next and lives
//! elsewhere; nothing here knows the IR.
//!
//! Nothing is ported from the reference implementation
//! (`~/github/PatternLanguage`, LGPL-2.1). It was read for the grammar and the
//! semantics, which is why the names here follow its names.
//!
//! This is the strict tier: a pattern the reference rejects is rejected here,
//! at the same line.
//!
//! The imperative half of the language is parsed with the same grammar as the
//! declarative half but kept as an opaque [`Statement`](ast::Statement), which
//! carries a coarse kind and the source range, so lowering can report each one
//! as a gap and show the text it could not express.

pub mod ast;
pub mod expr;
pub mod lexer;
pub mod parser;

pub use ast::{Decl, Program, Statement, StatementKind};
pub use expr::Expr;
pub use lexer::{HexpatError, Pos};
pub use parser::{parse, parse_with, NoIncludes, Resolved, Resolver};
