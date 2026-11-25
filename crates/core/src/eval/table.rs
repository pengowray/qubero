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
use crate::template::{bracketed_unit, Cells, FactUnit, TableShape};

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
    /// What each of `names` means, parallel to it, or empty.
    pub titles: Vec<String>,
    /// The groups the columns come in, when the file says how many.
    pub groups: Option<ColumnGroupsInfo>,
    /// What one row is: "sample", "record".
    pub row_word: Option<String>,
    /// Rows per second, when the rows are spaced in time. A real, since a
    /// filterbank written every 0.9 s has 1.11 rows a second and one sampled
    /// every 15 s has less than one.
    pub rate: Option<f64>,
    /// The fields that describe the table, with where they are and what they
    /// say, as the At-cursor panel shows any other origin.
    pub facts: Vec<TableFactInfo>,
    /// Where a row's cells are, when a row is a node rather than a run of
    /// values. None for every table a template declares.
    pub cells: Option<Cells>,
}

/// The groups a table's columns come in, worked out. See
/// [`ColumnGroups`](crate::template::ColumnGroups).
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnGroupsInfo {
    pub count: u64,
    pub word: String,
    pub spelled: String,
}

/// One field that describes a table, worked out: what to call it, where it
/// is, what it says and what unit that is in.
#[derive(Debug, Clone, PartialEq)]
pub struct TableFactInfo {
    /// The name the template gave the fact, or the field's path where it gave
    /// none: `TBIN`, `body.sample_rate`.
    pub label: String,
    /// Where it is, so the reader can go there. Empty when the number came
    /// from somewhere with no field of its own.
    pub path: Vec<usize>,
    /// What it says, in brief. Empty when it could not be read.
    pub value: String,
    /// The unit, as a UCUM code, or empty when the fact has none.
    pub unit: String,
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
        let groups = match &shape.groups {
            Some(g) => self.count_at(doc, path, &g.count.clone())?.map(|count| ColumnGroupsInfo {
                count,
                word: g.word.to_string(),
                spelled: g.spelled.to_string(),
            }),
            None => None,
        };
        let rate = match &shape.rate {
            Some(e) => self.rate_at(doc, path, &e.clone())?,
            None => None,
        };
        let mut facts = Vec::new();
        for fact in shape.facts.iter() {
            let mut sink = Sink::told(true);
            self.from_expr(doc, path, &fact.value.clone(), Role::Describes, &mut sink)?;
            let unit = match &fact.unit {
                None => String::new(),
                Some(FactUnit::Fixed(u)) => u.to_string(),
                Some(FactUnit::Bracketed(e)) => match self.text_at(doc, path, &e.clone(), None) {
                    Ok(text) => bracketed_unit(&text).unwrap_or_default().to_string(),
                    Err(err) if err.interrupted() => return Err(err),
                    Err(_) => String::new(),
                },
            };
            // A name and a unit are said of one field. An expression that
            // reaches several keeps each of them under its own path.
            let one = sink.out.len() == 1;
            facts.extend(sink.out.into_iter().map(|o| TableFactInfo {
                label: fact.label.as_deref().filter(|_| one).map_or(o.label, str::to_string),
                path: o.path,
                value: o.value,
                unit: if one { unit.clone() } else { String::new() },
            }));
        }
        Ok(Some(TableShapeInfo {
            columns,
            names: shape.names.iter().map(|n| n.to_string()).collect(),
            units: shape.units.iter().map(|u| u.to_string()).collect(),
            column_word: shape.column_word.as_ref().map(|w| w.to_string()),
            titles: shape.titles.iter().map(|t| t.to_string()).collect(),
            groups,
            row_word: shape.row_word.as_ref().map(|w| w.to_string()),
            rate,
            facts,
            cells: shape.cells.clone(),
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
            // The numbers of a pickled array, or the rows of a pickled list of
            // records, whose columns are read out of the file here because the
            // cheap question below cannot read anything.
            //
            // A `Decoded` parent is the same question one step down: an array
            // whose numbers were spelled rather than written opens them as a
            // space, and the numbers are what the table is over.
            Some(Ty::Pickle(_) | Ty::Decoded { .. }) => match self.pickle_table(path) {
                Some(shape) => Ok(self.pickle_columns(doc, path, shape)?.map(Arc::new)),
                None => Ok(None),
            },
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

    /// Rows a second, as the real the expression comes to. Nothing for a rate
    /// that is not a number above nought, since a time column worked out from
    /// one would put every row at the same instant.
    ///
    /// Asked as a real first, because a rate is a real however the file wrote
    /// it: `1/TBIN` is 2,929,687.5 for a GUPPI block, and AIFF's sample rate is
    /// an 80-bit float. A rate the real reading will not have is asked again
    /// as a whole number.
    fn rate_at<S: Source>(&mut self, doc: &Document<S>, path: &[usize], e: &Expr) -> R<Option<f64>> {
        let here = self.memo.get(path).map(|r| (r.offset, r.limit));
        match self.eval_real_at(doc, path, e, here) {
            Ok(v) if v.is_finite() && v > 0.0 => return Ok(Some(v)),
            Ok(_) => return Ok(None),
            Err(err) if err.interrupted() => return Err(err),
            Err(_) => {}
        }
        Ok(self.count_at(doc, path, e)?.map(|n| n as f64))
    }

    /// A count asked for again as a real, for the file that wrote it as one,
    /// with the fraction dropped. A field that is a float either refuses the
    /// whole-number reading or, reached as a sibling, answers with the nought
    /// that means nothing was found.
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

#[cfg(test)]
mod tests {
    use crate::template::bracketed_unit;

    #[test]
    fn a_unit_is_read_from_the_brackets_a_comment_opens_with() {
        assert_eq!(bracketed_unit("/ [s] Time per bin or sample"), Some("s"));
        assert_eq!(bracketed_unit(" [MHz] Channel/sub-band width"), Some("MHz"));
        assert_eq!(bracketed_unit("/ Nr of bits/datum [bits]"), None);
        assert_eq!(bracketed_unit("/ [] nothing"), None);
        assert_eq!(bracketed_unit("/ [s unclosed"), None);
        assert_eq!(bracketed_unit(""), None);
    }
}
