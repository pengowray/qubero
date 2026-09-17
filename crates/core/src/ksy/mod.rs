//! Kaitai Struct `.ksy` in, a [`Template`](crate::template::Template) out.
//!
//! Half of that is here so far: [`parse`] reads a `.ksy` into the [`spec`]
//! model, which is the Kaitai compiler's own model of a format description,
//! and the expression language it is written in is in [`expr`]. Lowering that
//! model to the IR, and the [report](crate::report) of what could not be expressed, comes
//! next.
//!
//! A `.ksy` is never kept around at run time. Once converted, a Kaitai format
//! is indistinguishable from a hand-written one and every view works on it
//! unchanged. Nothing from the Kaitai compiler is ported: it was read for the
//! semantics, which is why the names here follow its names.
//!
//! This is the strict tier. A `.ksy` the compiler would reject is rejected
//! here, naming the same path.

pub mod bundled;
pub mod expr;
pub mod imports;
pub mod lower;
pub mod signature;
pub mod spec;
pub mod yaml;

pub use bundled::{Bundled, BundledImports};
pub use expr::Expr;
pub use imports::{Imports, MapImports, NoImports};
pub use lower::{convert, Converted};
pub use signature::signature;
pub use crate::report::{Became, Gap, Note, Report};
pub use spec::ClassSpec;
pub use yaml::KsyError;

/// Read a `.ksy` file into the spec model.
pub fn parse(text: &str) -> Result<ClassSpec, KsyError> {
	let document = yaml::load(text)?;
	ClassSpec::from_document(&document)
}
