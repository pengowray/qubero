//! What a run of values is a table of.
//!
//! One question, [`Evaluator::table_shape`], and it is the template's claim
//! read against this file: how many elements make a row, what the columns and
//! rows are called, how fast the rows come, and which fields a reader wants to
//! see above the table. Nothing here decides that a node *is* a table; the
//! template said so by hanging a [`TableShape`] on the field, and this works
//! out the numbers in it.
//!
//! The same rule the rest of the evaluator is built on: what cannot be
//! established is answered as nothing. A shape whose `columns` names a field
//! this file has not got comes back with `columns: None` and the rest intact,
//! because the names and the words are still true and a made-up column count
//! would put the samples in the wrong rows. A read that has not arrived is
//! passed up as pending, the way every other question here answers it.

use std::sync::Arc;

use super::origin::{Role, Sink};
use super::*;
use crate::template::TableShape;

/// A table shape with its expressions worked out in the file at hand.
#[derive(Debug, Clone, PartialEq)]
pub struct TableShapeInfo {
    /// Elements per row. None when the template did not say, or when what it
    /// said cannot be read here: either way one element is one row.
    pub columns: Option<u64>,
    /// What the columns are called, used when there are exactly this many.
    pub names: Vec<String>,
    /// The units the columns are measured in, parallel to `names`.
    pub units: Vec<String>,
    /// What to call a column `names` does not reach: "channel".
    pub column_word: Option<String>,
    /// What one row is: "sample", "record".
    pub row_word: Option<String>,
    /// Rows per second, when the rows are spaced in time.
    pub rate: Option<u64>,
    /// The fields that describe the table, with where they are and what they
    /// say, as the At-cursor panel shows any other origin.
    pub facts: Vec<Origin>,
}

impl Evaluator {
    /// The table the field at `path` is, or nothing when the template made no
    /// such claim about it.
    ///
    /// The field is the parent structure's, at the last step of the path, and
    /// the walk stops there: unlike [`Evaluator::time_of`], a declaration on a
    /// list is about the list and not about each of its elements. A sample of
    /// a run is not a table of its own.
    pub fn table_shape<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Option<TableShapeInfo>> {
        let Some(shape) = self.table_at(doc, path)? else { return Ok(None) };
        // The two numbers, each answered as nothing when the expression names
        // something this file has not got. A read that has not arrived is a
        // pending answer for the whole question, not a missing column count.
        let columns = match &shape.columns {
            Some(e) => self.count_at(doc, path, &e.clone())?,
            None => None,
        };
        let rate = match &shape.rate {
            Some(e) => self.count_at(doc, path, &e.clone())?,
            None => None,
        };
        let mut sink = Sink::told(true);
        for e in shape.facts.iter() {
            let e = e.clone();
            self.from_expr(doc, path, &e, Role::Describes, &mut sink)?;
        }
        Ok(Some(TableShapeInfo {
            columns,
            names: shape.names.iter().map(|n| n.to_string()).collect(),
            units: shape.units.iter().map(|u| u.to_string()).collect(),
            column_word: shape.column_word.as_ref().map(|w| w.to_string()),
            row_word: shape.row_word.as_ref().map(|w| w.to_string()),
            rate,
            facts: sink.out,
        }))
    }

    /// Whether the field at `path` carries a shape at all, without working any
    /// of it out. What [`NodeInfo::table`] is.
    pub(super) fn table_declared(&self, path: &[usize]) -> bool {
        let Some((&last, parent)) = path.split_last() else { return false };
        matches!(self.memo.get(parent).map(|r| &r.ty),
            Some(Ty::Struct(s)) if s.fields.get(last).is_some_and(|f| f.table.is_some()))
            || self.pickle_table(path).is_some()
    }

    /// The shape the template hung on the field at `path`, if any.
    fn table_at<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Option<Arc<TableShape>>> {
        self.resolve(doc, path)?;
        let Some((&last, parent)) = path.split_last() else { return Ok(None) };
        match self.memo.get(parent).map(|r| &r.ty) {
            Some(Ty::Struct(s)) => Ok(s.fields.get(last).and_then(|f| f.table.clone())),
            // The numbers of a pickled array, whose shape the match holds.
            Some(Ty::Pickle(_)) => Ok(self.pickle_table(path).map(Arc::new)),
            _ => Ok(None),
        }
    }

    /// One of the shape's numbers, as a count. Nothing rather than an error for
    /// an expression that will not read here, and nothing for a number that is
    /// not a count: a rate of nought or of less than nought is no rate, and
    /// showing a time column worked out from one would put every row at the
    /// same instant.
    fn count_at<S: Source>(&mut self, doc: &Document<S>, path: &[usize], e: &Expr) -> R<Option<u64>> {
        match self.eval_expr(doc, path, e) {
            Ok(v) if v > 0 => Ok(u64::try_from(v).ok()),
            // No count, which for a field written as a float is the whole
            // number reading declining to round it rather than the field
            // saying nought. Asked again as a real before it is given up on.
            Ok(_) => self.real_count_at(doc, path, e),
            Err(err) if err.interrupted() => Err(err),
            Err(_) => self.real_count_at(doc, path, e),
        }
    }

    /// The same number asked for again as a real, for the file that wrote it
    /// as one.
    ///
    /// AIFF's sample rate is an 80-bit extended float, so it says 44100.0 and
    /// the whole-number reading of a field will not have it: a field that is a
    /// float either refuses outright or, reached as a sibling, answers with
    /// the nought that means nothing was found. That is right everywhere else,
    /// because a length or an offset that is not whole is a misread rather
    /// than a number to truncate. Rows a second is a count either way, and a
    /// rate the file states exactly is the one place where dropping the
    /// fraction is what the reader meant.
    fn real_count_at<S: Source>(&mut self, doc: &Document<S>, path: &[usize], e: &Expr) -> R<Option<u64>> {
        let here = self.memo.get(path).map(|r| (r.offset, r.limit));
        match self.eval_real_at(doc, path, e, here) {
            Ok(v) if v.is_finite() && v >= 1.0 => Ok(u64::try_from(v.trunc() as i128).ok()),
            Ok(_) => Ok(None),
            Err(err) if err.interrupted() => Err(err),
            Err(_) => Ok(None),
        }
    }
}
