//! qubero-core: the pure-Rust engine behind the editor.
//!
//! Conventions used everywhere in this crate:
//! * Offsets and lengths are in **bits** unless a name says `byte`.
//! * Bit `i` of the document is bit `7 - (i % 8)` of byte `i / 8`, i.e. bit 0 is the
//!   most significant bit of byte 0. This matches how a hex dump is read.
//! * The original file is never copied into memory. It is read on demand through a
//!   [`Source`], which may report that a range is not loaded yet.

pub mod bits;
pub mod document;
pub mod eval;
pub mod formats;
pub mod piece;
pub mod save;
pub mod source;
pub mod template;

pub use document::Document;
pub use eval::{EvalError, Evaluator, NodeInfo, Value};
pub use piece::PieceTable;
pub use save::{Run, RunKind};
pub use source::{ChunkStore, MemSource, Missing, Source};
pub use template::{Endian, Expr, Template, Ty, Until};
