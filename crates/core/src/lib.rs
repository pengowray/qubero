//! qubero-core: the pure-Rust engine behind the editor.
//!
//! Conventions used everywhere in this crate:
//! * Offsets and lengths are in **bits** unless a name says `byte`.
//! * Bit `i` of the document is bit `7 - (i % 8)` of byte `i / 8`, i.e. bit 0 is the
//!   most significant bit of byte 0. This matches how a hex dump is read.
//! * The original file is never copied into memory. It is read on demand through a
//!   [`Source`], which may report that a range is not loaded yet.

pub mod bits;
pub mod code;
pub mod decode;
pub mod diescript;
pub mod dosbasic;
pub mod document;
pub mod encode;
pub mod eval;
pub mod formats;
pub mod gather;
pub mod hexdump;
pub mod json;
pub mod machinery;
pub mod magicrule;
pub mod overview;
pub mod piece;
pub mod riscv;
pub mod save;
pub mod search;
pub mod source;
pub mod template;
pub mod text;
pub mod textview;
pub mod thumb;
pub mod varintbits;

pub use document::Document;
pub use encode::EDIT_LIMIT_BYTES;
pub use eval::{EvalError, Evaluator, ExtentEstimate, NodeInfo, Span, SpanPart, Value, Write};
pub use piece::PieceTable;
pub use save::{Run, RunKind};
pub use search::{Needle, Search, Step};
pub use source::{ChunkStore, MemSource, Missing, Source};
pub use template::{Encoding, Endian, Expr, Part, StrLen, Template, Ty, Until};
