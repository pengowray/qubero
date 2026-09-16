//! What a conversion did, said out loud.
//!
//! Shared by every converter that reads a format description into the IR: the
//! Kaitai one in [`crate::ksy`] and the ImHex pattern one in [`crate::hexpat`].
//!
//! The report is as much the deliverable as the template. For every field it
//! says what the field became; for anything the IR cannot express it gives the
//! path, the source text and the reason, and the field is left as bytes. A
//! silent approximation is the defect this exists to prevent: a `repeat-until`
//! whose predicate was quietly turned into "until the end" reads a file wrongly
//! and says nothing about it.
//!
//! Where the mapping is exact but indirect, that is a note rather than a gap: a
//! string compared as its bytes read big-endian is still the same comparison,
//! but a reader looking at the IR would not guess where the number came from.
//!
//! This file is the shape only. Each lowering fills it in.

/// Everything the conversion has to say.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Report {
	/// One per field converted, in file order.
	pub fields: Vec<Became>,
	/// Everything the IR could not express.
	pub gaps: Vec<Gap>,
	/// Everything expressed a way a reader would not guess.
	pub notes: Vec<Note>,
}

/// A field, and what it turned into.
#[derive(Debug, Clone, PartialEq)]
pub struct Became {
	/// Where in the source this field is written: a `.ksy` path such as
	/// `/types/chunk/seq/2`, or a `.hexpat` line and column.
	pub path: String,
	/// The source text this is about: the type, the expression, the key.
	pub source: String,
	/// What it became, in the IR's own words.
	pub message: String,
}

/// Something the IR cannot say.
#[derive(Debug, Clone, PartialEq)]
pub struct Gap {
	pub path: String,
	pub source: String,
	/// Why it cannot be said, and what was left behind instead.
	pub reason: String,
}

/// Something said exactly, but not in the way the `.ksy` said it.
#[derive(Debug, Clone, PartialEq)]
pub struct Note {
	pub path: String,
	pub source: String,
	pub message: String,
}

impl Report {
	pub fn new() -> Self {
		Report::default()
	}

	pub fn became(
		&mut self,
		path: impl Into<String>,
		source: impl Into<String>,
		message: impl Into<String>,
	) {
		self.fields.push(Became {
			path: path.into(),
			source: source.into(),
			message: message.into(),
		});
	}

	pub fn gap(
		&mut self,
		path: impl Into<String>,
		source: impl Into<String>,
		reason: impl Into<String>,
	) {
		self.gaps.push(Gap { path: path.into(), source: source.into(), reason: reason.into() });
	}

	pub fn note(
		&mut self,
		path: impl Into<String>,
		source: impl Into<String>,
		message: impl Into<String>,
	) {
		self.notes.push(Note {
			path: path.into(),
			source: source.into(),
			message: message.into(),
		});
	}

	/// Whether the whole description was expressed. A report with notes is still
	/// clean; a report with gaps is not.
	pub fn is_complete(&self) -> bool {
		self.gaps.is_empty()
	}
}
