//! A torch tensor read as the numbers it stands for.
//!
//! The pickle says everything about a tensor but its values: which storage it
//! is a window onto, how far into that storage it starts, how far apart its
//! values are along each axis. The values are somewhere else, and where that
//! is depends on how the file was written, so this file is the two answers.
//!
//! Nothing here runs anything. The persistent id named a storage and this
//! finds the bytes that storage is, the way [`picklecells`](super::picklecells)
//! finds a frame's.

use super::pickleparts::Says;
use super::*;
use crate::formats::pickle::familiar::{Kind, Tensor, Value as Captured};

/// What the entries holding a tensor's numbers are called inside a torch
/// archive: `data/` and the storage key. torch's own name for them, and what
/// the archive listing shows, so a reader can go from the row to the entry.
pub(super) const DATA_FOLDER: &str = "data";

/// What the `stored at` row says when nothing in this file holds the numbers,
/// which is what a `data.pkl` opened on its own is.
const NOT_HERE: &str = "not in this file";

/// The tensor a value is, for the rows and the table that read one.
pub(super) fn tensor_of(value: &Captured) -> Option<&Tensor> {
    match &value.kind {
        Kind::Tensor(t) => Some(t),
        _ => None,
    }
}

impl Evaluator {
    /// What one of a tensor's rows says.
    pub(super) fn tensor_summary<S: Source>(
        &mut self,
        doc: &Document<S>,
        r: &Resolved,
        base: u64,
        tensor: &Tensor,
        says: Says,
    ) -> R<String> {
        let key = self.run_text(doc, r, base, tensor.key)?;
        match says {
            Says::Storage => Ok(key),
            Says::Location => self.run_text(doc, r, base, tensor.location),
            // Where the numbers are in the format, which is true of the file
            // whether or not this reading can reach them.
            Says::Numbers => Ok(format!("{} values in {DATA_FOLDER}/{key}", tensor.values())),
            Says::StoredAt => Ok(NOT_HERE.to_string()),
            _ => Ok(String::new()),
        }
    }

    /// A run of the pickle read as the text it is, for the two the tensor
    /// names: a run written once and referred to again is read where it was
    /// written, which is why this goes through the pickle field rather than
    /// through the tensor's own bytes.
    fn run_text<S: Source>(&self, doc: &Document<S>, r: &Resolved, base: u64, (at, len): (usize, usize)) -> R<String> {
        let bytes = self.read(doc, r, base + at as u64 * 8, len as u64 * 8)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}
