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
pub mod bundled;
pub mod expr;
pub mod includes;
pub mod lexer;
pub mod lower;
pub mod parser;
pub mod types;

pub use ast::{Decl, Program, Statement, StatementKind};
pub use expr::Expr;
pub use includes::{Includes, MapIncludes, NoFiles};
pub use lexer::{HexpatError, Pos};
pub use lower::Converted;
pub use parser::{parse, parse_with, NoIncludes, Resolved, Resolver};

/// What ImHex defines before the first line of a pattern, and what the corpus
/// reads `#ifdef` on.
const DEFINES: &[&str] = &["__IMHEX__"];

/// Read a `.hexpat` and lower it to a [`Template`](crate::template::Template)
/// and a [`Report`](crate::report::Report).
///
/// `includes` says what the pattern's `#include`s and `import`s resolve to. A
/// path it does not answer for falls back on the built-in table of `std::`,
/// `type::` and `hex::` declarations, and a path neither answers for is an
/// error naming the path, at the line that asked for it.
pub fn convert(text: &str, includes: &dyn Includes) -> Result<Converted, HexpatError> {
	convert_named("pattern", text, includes)
}

/// The same, with the name the template goes by.
pub fn convert_named(name: &str, text: &str, includes: &dyn Includes) -> Result<Converted, HexpatError> {
	let resolver = includes::IncludeResolver { includes };
	let defines: Vec<String> = DEFINES.iter().map(|d| d.to_string()).collect();
	let program = parse_with(name, text, &resolver, &defines)?;
	lower::lower(name, &program)
}
