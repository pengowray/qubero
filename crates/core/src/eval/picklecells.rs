//! The cells of a pandas frame, read a window of rows at a time.
//!
//! [`pickleframe`](super::pickleframe) says what a frame is made of without
//! touching the file; this reads it. Every cell is a value the file holds
//! somewhere, or a number counted out from a start and a step, or a category a
//! code names, and every one of them is worked out here rather than in the
//! interface: a cell is not a node of the tree, so there is nothing for a view
//! to walk.

use std::sync::Arc;

use super::pickleframe::*;
use super::pickleparts::{spot, Part};
use super::picklesummary::iso_time;
use super::*;
use crate::formats::pickle::familiar::{Column, Dtype, Kind, Match, Names, Value as Captured};
use crate::formats::pickle::shapes;
use crate::template::{Cells, Endian, TableShape};

/// How one entry of a mask is spelled, which is one byte saying whether the
/// cell beside it counts. The same for a plain dtype and for a column of a
/// structured one: `make_mask_descr` writes `?` either way.
const MASK_ELEMENT: &str = "|b1";

/// One cell of a computed table: what it says, and where the bytes it was read
/// from are.
///
/// The address is the cell's own. A frame's row is one value out of each of
/// several blocks, scattered through the file, so there is no run of bytes the
/// row is and nothing but a per-cell address would be true.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameCell {
    /// What the cell says, or nothing where the file holds no value for it: a
    /// NaN, a `None`, a categorical code of -1.
    pub value: Option<Value>,
    pub at: CellAt,
    /// True for an entry a masked array's mask hides. Such a cell has no
    /// value to show and its bytes all the same: the number is in the file,
    /// the array simply says not to count it. So it is neither an absent cell
    /// nor a reason to have no address, and the reader is told which of the
    /// two nothings it is looking at.
    pub masked: bool,
}

/// Where a computed cell's bytes are, for the three reasons a cell may have
/// none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellAt {
    /// A run of the address space `space`. 0 is the file; anything else is a
    /// space the file opened inside itself, and the offset counts from that
    /// space's own start.
    Bytes { space: u32, offset_bits: u64, size_bits: u64 },
    /// Counted out rather than written down, which is a `RangeIndex` label: a
    /// start and a step, and no bytes anywhere holding the number.
    Counted,
    /// Nowhere this reading can point at: a run whose dtype it does not read,
    /// a value past the end of the bytes the file holds.
    Nowhere,
}

impl FrameCell {
    /// A cell with a value and a run of bytes behind it.
    pub(super) fn at(value: Option<Value>, at: CellAt) -> Self {
        FrameCell { value, at, masked: false }
    }

    /// A cell this reading could not place.
    pub(super) fn nowhere() -> Self {
        FrameCell { value: None, at: CellAt::Nowhere, masked: false }
    }

    /// An entry a mask hides: no value, and the bytes it would have read.
    pub(super) fn masked(at: CellAt) -> Self {
        FrameCell { value: None, at, masked: true }
    }
}

impl Evaluator {
    /// The table a pandas frame or series is: the index and then the columns,
    /// with what one value of each is, over as many rows as the index says.
    ///
    /// Nothing is published unless every block agrees with the index about how
    /// many rows there are. A table drawn from blocks that disagree would put
    /// one column's values beside another column's.
    pub(super) fn frame_shape<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Option<TableShape>> {
        let (root, found) = self.pickle_doc(doc, path)?;
        let Some((_, Part::Value(object))) = spot(&found, &path[root.len()..]) else { return Ok(None) };
        let Some(frame) = frame_of(&found, object) else { return Ok(None) };
        let r = self.memo[&root].clone();
        let base = r.offset;
        let Some(rows) = self.index_len(doc, &r, base, &found, frame.index)? else { return Ok(None) };
        let wide = match frame.columns {
            Some(axis) => match self.index_len(doc, &r, base, &found, axis)? {
                Some(wide) => wide,
                None => return Ok(None),
            },
            None => 1,
        };
        // The index is the first column, so that a row can be read off against
        // the label the frame files it under.
        let mut names: Vec<Arc<str>> = vec![self.axis_name(doc, &r, base, frame.index)?.unwrap_or_else(|| INDEX_COLUMN.into()).into()];
        let mut units: Vec<Arc<str>> = vec![self.axis_word(doc, &r, base, &found, frame.index)?.into()];
        for c in 0..wide {
            let Some((block, j)) = frame.column(&found, c) else { return Ok(None) };
            let Some(held) = values_of(&found, block.values) else { return Ok(None) };
            let Some((held_rows, _)) = held.rows_and_columns() else { return Ok(None) };
            if held_rows != rows {
                return Ok(None);
            }
            let _ = j;
            names.push(match frame.columns {
                Some(axis) => self.label_at(doc, &root, &r, base, &found, axis, c)?.unwrap_or_else(|| format!("column {c}")).into(),
                None => self.series_name(doc, &r, base, object)?.unwrap_or_else(|| VALUE_COLUMN.into()).into(),
            });
            units.push(word_of(&held).into());
        }
        Ok(Some(TableShape {
            names,
            units,
            row_word: Some(ROW_WORD.into()),
            cells: Some(Cells::Computed { rows }),
            ..Default::default()
        }))
    }

    /// The rows `from` up to `to`, each as one value a column, with nothing
    /// where the frame has no value.
    pub fn pickle_cells<S: Source>(&mut self, doc: &Document<S>, path: &[usize], from: u64, to: u64) -> R<Vec<Vec<FrameCell>>> {
        let (root, found) = self.pickle_doc(doc, path)?;
        let Some((_, Part::Value(object))) = spot(&found, &path[root.len()..]) else { return fail("not a frame") };
        // A tensor, whose values are in another entry of the archive and are
        // read there rather than anywhere under this node.
        if let Some(tensor) = super::pickletorch::tensor_of(object) {
            let r = self.memo[&root].clone();
            let base = r.offset;
            return self.tensor_cells(doc, &r, base, tensor, from, to);
        }
        // A state dict, whose rows are what the pickle already said about each
        // tensor rather than any of their values.
        if let Some(held) = super::pickletorch::tensor_table(object) {
            let r = self.memo[&root].clone();
            let base = r.offset;
            return self.tensor_rows(doc, &r, base, &held, from, to);
        }
        // A masked array, which is two runs read into one cell.
        if masked_of(object).is_some() {
            let r = self.memo[&root].clone();
            let base = r.offset;
            return self.masked_cells(doc, &root, &r, base, &found, object, from, to);
        }
        let Some(frame) = frame_of(&found, object) else { return fail("not a frame") };
        let r = self.memo[&root].clone();
        let base = r.offset;
        let Some(rows) = self.index_len(doc, &r, base, &found, frame.index)? else { return fail("this frame does not say how many rows it has") };
        let wide = match frame.columns {
            Some(axis) => self.index_len(doc, &r, base, &found, axis)?.unwrap_or(0),
            None => 1,
        };
        // Which block each column is in, worked out once rather than per cell.
        let placed: Vec<(u64, Values)> = (0..wide)
            .filter_map(|c| {
                let (block, j) = frame.column(&found, c)?;
                Some((j, values_of(&found, block.values)?))
            })
            .collect();
        if placed.len() as u64 != wide {
            return fail("this frame's blocks do not cover its columns");
        }
        let mut out = Vec::new();
        for row in from..to.min(rows) {
            let mut cells = vec![self.index_label(doc, &root, &r, base, &found, frame.index, row)?];
            for (j, held) in &placed {
                cells.push(self.one_value(doc, &root, &r, base, &found, held, *j, row)?);
            }
            out.push(cells);
        }
        Ok(out)
    }

    /// The cells of a masked array: the numbers, with the entries the mask
    /// hides shown empty.
    ///
    /// Two runs go into one cell, so the table is worked out here rather than
    /// laid out as a run of fields. Everything else is an array's own table:
    /// the run is chunked into rows the way [`Evaluator::pickle_table`] chunks
    /// one, and a value is read through
    /// [`Evaluator::number_at`], so a protocol 0 or 2 array reads through the
    /// space its numbers opened and every cell carries the address of the
    /// bytes it came from.
    ///
    /// A masked cell keeps that address. The number is in the file; the array
    /// says not to count it, which is a different thing from a value the file
    /// does not hold, and a reader who wants the stored number can still reach
    /// it. A mask this reading cannot read leaves every cell showing its
    /// number rather than blanking the table on a doubt.
    #[allow(clippy::too_many_arguments)]
    fn masked_cells<S: Source>(
        &mut self,
        doc: &Document<S>,
        root: &[usize],
        r: &Resolved,
        base: u64,
        found: &Match,
        object: &Captured,
        from: u64,
        to: u64,
    ) -> R<Vec<Vec<FrameCell>>> {
        let Some((data, mask)) = masked_of(object) else { return fail("not a masked array") };
        let Kind::Array { dimensions, fortran_order, .. } = &data.kind else { return fail("this masked array holds no numbers") };
        let Some((rows, columns)) = masked_shape(dimensions, *fortran_order) else { return fail("this masked array has no table") };
        let Some(Values::Numbers(numbers)) = values_of(found, data) else { return fail("these numbers do not read") };
        let hides = match values_of(found, mask) {
            Some(Values::Numbers(held)) => Some(held),
            _ => None,
        };
        // A structured dtype makes a row out of the named columns of one
        // record rather than out of the run's own elements, and its mask hides
        // a column of a row rather than an entry.
        if let Dtype::Record { columns, width } = numbers.dtype {
            return self.masked_record_cells(doc, root, r, base, &numbers, hides.as_ref(), columns, *width, rows, from, to);
        }
        let mut out = Vec::new();
        for row in from..to.min(rows) {
            let mut cells = Vec::new();
            for column in 0..columns {
                let Some(elem) = row.checked_mul(columns).and_then(|at| at.checked_add(column)) else {
                    cells.push(FrameCell::nowhere());
                    continue;
                };
                let cell = self.number_at(doc, root, r, base, &numbers, elem)?;
                let hidden = match &hides {
                    Some(held) => self.mask_at(doc, root, r, base, held, elem)?,
                    None => false,
                };
                cells.push(match hidden {
                    true => FrameCell::masked(cell.at),
                    false => cell,
                });
            }
            out.push(cells);
        }
        Ok(out)
    }

    /// The rows of a masked array whose dtype is a record: one cell per named
    /// column, blank where the mask hides that column of that row.
    ///
    /// The numbers and the mask are two runs of records over the same rows,
    /// and the two have different widths: `make_mask_descr` packs one boolean
    /// a column with no padding, so a mask cell is at `row * columns + column`
    /// where a value cell is at `row * width + column.at`.
    #[allow(clippy::too_many_arguments)]
    fn masked_record_cells<S: Source>(
        &mut self,
        doc: &Document<S>,
        root: &[usize],
        r: &Resolved,
        base: u64,
        numbers: &Numbers,
        hides: Option<&Numbers>,
        columns: &[Column],
        width: u64,
        rows: u64,
        from: u64,
        to: u64,
    ) -> R<Vec<Vec<FrameCell>>> {
        let mut out = Vec::new();
        for row in from..to.min(rows) {
            let mut cells = Vec::new();
            for (at, column) in columns.iter().enumerate() {
                let Some(into) = row.checked_mul(width).and_then(|start| start.checked_add(column.at)) else {
                    cells.push(FrameCell::nowhere());
                    continue;
                };
                let Some((ty, held)) = shapes::element(&column.dtype) else {
                    cells.push(FrameCell::nowhere());
                    continue;
                };
                let (read, where_it_is) = self.at_bytes(doc, root, r, base, numbers, into, &ty, held)?;
                let hidden = match hides {
                    Some(mask) => self.mask_record_at(doc, root, r, base, mask, row, at as u64, columns.len() as u64)?,
                    None => false,
                };
                cells.push(match hidden {
                    true => FrameCell::masked(where_it_is),
                    false => FrameCell::at(read, where_it_is),
                });
            }
            out.push(cells);
        }
        Ok(out)
    }

    /// Whether the mask hides column `column` of row `row`, which is one byte
    /// of a mask laid out one boolean a column.
    #[allow(clippy::too_many_arguments)]
    fn mask_record_at<S: Source>(
        &mut self,
        doc: &Document<S>,
        root: &[usize],
        r: &Resolved,
        base: u64,
        mask: &Numbers,
        row: u64,
        column: u64,
        columns: u64,
    ) -> R<bool> {
        let Some(into) = row.checked_mul(columns).and_then(|start| start.checked_add(column)) else { return Ok(false) };
        let Some((ty, width)) = shapes::element(MASK_ELEMENT) else { return Ok(false) };
        let (read, _) = self.at_bytes(doc, root, r, base, mask, into, &ty, width)?;
        Ok(matches!(read, Some(Value::UInt(n)) if n != 0) || matches!(read, Some(Value::Int(n)) if n != 0))
    }

    /// Whether the mask hides entry `elem`, which is one byte of the mask's
    /// own run. Anything this cannot read is not a mask saying yes.
    fn mask_at<S: Source>(&mut self, doc: &Document<S>, root: &[usize], r: &Resolved, base: u64, mask: &Numbers, elem: u64) -> R<bool> {
        let Dtype::Plain(spelling) = mask.dtype else { return Ok(false) };
        let Some((ty, width)) = shapes::element(spelling) else { return Ok(false) };
        let (read, _) = self.element_at(doc, root, r, base, mask, elem, &ty, width)?;
        Ok(matches!(read, Some(Value::UInt(n)) if n != 0) || matches!(read, Some(Value::Int(n)) if n != 0))
    }

    /// How many labels an index has.
    pub(super) fn index_len<S: Source>(&mut self, doc: &Document<S>, r: &Resolved, base: u64, found: &Match, index: &Captured) -> R<Option<u64>> {
        let Some(state) = index_state(index) else { return Ok(None) };
        if is_range(index) {
            let (start, stop, step) = self.range_bounds(doc, r, base, state)?;
            return Ok(match (start, stop, step) {
                (Some(start), Some(stop), Some(step)) => range_len(start, stop, step),
                _ => None,
            });
        }
        let Some(values) = self.index_values(doc, r, base, state)? else { return Ok(None) };
        let Some(held) = values_of(&found, values) else { return Ok(None) };
        Ok(held.rows_and_columns().map(|(rows, _)| rows))
    }

    /// The value an index's dictionary holds under `data`.
    fn index_values<'a, S: Source>(
        &self,
        doc: &Document<S>,
        r: &Resolved,
        base: u64,
        state: &'a [(Captured, Captured)],
    ) -> R<Option<&'a Captured>> {
        for (key, value) in state {
            if self.pickle_text(doc, r, base, key)?.as_deref() == Some(DATA) {
                return Ok(Some(value));
            }
        }
        Ok(None)
    }

    /// The start, stop and step a counted index is written as.
    pub(super) fn range_bounds<S: Source>(
        &self,
        doc: &Document<S>,
        r: &Resolved,
        base: u64,
        state: &[(Captured, Captured)],
    ) -> R<(Option<i128>, Option<i128>, Option<i128>)> {
        let (mut start, mut stop, mut step) = (None, None, None);
        for (key, value) in state {
            let Kind::Int { value: n, .. } = value.kind else { continue };
            match self.pickle_text(doc, r, base, key)?.as_deref() {
                Some(START) => start = Some(n),
                Some(STOP) => stop = Some(n),
                Some(STEP) => step = Some(n),
                _ => {}
            }
        }
        Ok((start, stop, step))
    }

    /// What an index calls itself, which is the header of the table's first
    /// column when it has one.
    fn axis_name<S: Source>(&self, doc: &Document<S>, r: &Resolved, base: u64, index: &Captured) -> R<Option<String>> {
        let Some(state) = index_state(index) else { return Ok(None) };
        for (key, value) in state {
            if self.pickle_text(doc, r, base, key)?.as_deref() == Some(NAME) {
                return self.pickle_text(doc, r, base, value);
            }
        }
        Ok(None)
    }

    /// What one label of an index is, for the first column's header.
    fn axis_word<S: Source>(&mut self, doc: &Document<S>, r: &Resolved, base: u64, found: &Match, index: &Captured) -> R<String> {
        if is_range(index) {
            return Ok("int64".into());
        }
        let Some(state) = index_state(index) else { return Ok(String::new()) };
        let Some(values) = self.index_values(doc, r, base, state)? else { return Ok(String::new()) };
        Ok(values_of(&found, values).map(|held| word_of(&held)).unwrap_or_default())
    }

    /// What a series calls its one column.
    fn series_name<S: Source>(&self, doc: &Document<S>, r: &Resolved, base: u64, object: &Captured) -> R<Option<String>> {
        let Kind::Instance { state: Some(state), .. } = &object.kind else { return Ok(None) };
        let Kind::Dict(entries) = &state.kind else { return Ok(None) };
        for (key, value) in entries {
            // `_name` since pandas 1.5, `name` before it.
            if matches!(self.pickle_text(doc, r, base, key)?.as_deref(), Some("_name") | Some("name")) {
                if let Some(said) = self.pickle_text(doc, r, base, value)? {
                    return Ok(Some(said));
                }
            }
        }
        Ok(None)
    }

    /// The label the index files row `row` under.
    ///
    /// A counted index is the one with no bytes: it is a start, a stop and a
    /// step, and the number in the cell was never written anywhere. The cell
    /// says so rather than pointing at the bounds it was counted from, which
    /// are three numbers somewhere else and not this row's.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn index_label<S: Source>(&mut self, doc: &Document<S>, root: &[usize], r: &Resolved, base: u64, found: &Match, index: &Captured, row: u64) -> R<FrameCell> {
        let Some(state) = index_state(index) else { return Ok(FrameCell::nowhere()) };
        if is_range(index) {
            let (start, _, step) = self.range_bounds(doc, r, base, state)?;
            let (Some(start), Some(step)) = (start, step) else { return Ok(FrameCell::at(None, CellAt::Counted)) };
            let at = start.checked_add(step.checked_mul(i128::from(row)).unwrap_or(0));
            return Ok(FrameCell::at(at.map(Value::Int), CellAt::Counted));
        }
        let Some(values) = self.index_values(doc, r, base, state)? else { return Ok(FrameCell::nowhere()) };
        let Some(held) = values_of(&found, values) else { return Ok(FrameCell::nowhere()) };
        self.one_value(doc, root, r, base, found, &held, 0, row)
    }

    /// One value of a column, or nothing where the frame has none.
    ///
    /// Three kinds of cell and three kinds of address. A number is in a run,
    /// either in the file or in the space the file spelled it into. A pickled
    /// object is the instructions that rebuild it, and the cell points at all
    /// of them: the opcode as well as its payload, since that run is what the
    /// file says this value is. A category is a code in one run naming a name
    /// in another, and the code is the byte this row holds.
    #[allow(clippy::too_many_arguments)]
    fn one_value<S: Source>(
        &mut self,
        doc: &Document<S>,
        root: &[usize],
        r: &Resolved,
        base: u64,
        found: &Match,
        held: &Values,
        column: u64,
        row: u64,
    ) -> R<FrameCell> {
        let Some(elem) = held.at(column, row) else { return Ok(FrameCell::nowhere()) };
        match held {
            Values::Numbers(n) => self.number_at(doc, root, r, base, n, elem),
            Values::Texts(items) => {
                let Some(item) = items.get(elem as usize) else { return Ok(FrameCell::nowhere()) };
                let at = CellAt::Bytes { space: r.space, offset_bits: base + item.at as u64 * 8, size_bits: item.len as u64 * 8 };
                if let Some(said) = self.pickle_text(doc, r, base, item)? {
                    return Ok(FrameCell::at(Some(Value::Str(said)), at));
                }
                // Anything else a column of objects holds: a date, a list, an
                // exact number. The cell says what the tree's own row for that
                // value says, which is the value itself where it is one thing
                // and what kind of thing and how much of it where it is not.
                Ok(FrameCell::at(self.pickle_said(doc, found, r, base, item)?.map(Value::Str), at))
            }
            Values::Coded(codes, names) => {
                let held = self.number_at(doc, root, r, base, codes, elem)?;
                let code = match held.value {
                    Some(Value::Int(code)) => code,
                    Some(Value::UInt(code)) => i128::try_from(code).unwrap_or(NO_CATEGORY),
                    _ => return Ok(FrameCell::at(None, held.at)),
                };
                if code == NO_CATEGORY || code < 0 {
                    return Ok(FrameCell::at(None, held.at));
                }
                let Some(item) = usize::try_from(code).ok().and_then(|at| names.get(at)) else { return Ok(FrameCell::at(None, held.at)) };
                Ok(FrameCell::at(self.pickle_text(doc, r, base, item)?.map(Value::Str), held.at))
            }
        }
    }

    /// One number of a run, read as the type its dtype says.
    ///
    /// A date is the one that is not read as its bytes: it is a count of the
    /// unit its dtype names, and a reader wants the date rather than the
    /// count, so it is read as a whole number and written out.
    #[allow(clippy::too_many_arguments)]
    fn number_at<S: Source>(&mut self, doc: &Document<S>, root: &[usize], r: &Resolved, base: u64, n: &Numbers, elem: u64) -> R<FrameCell> {
        let spelling = match n.dtype {
            Dtype::Plain(spelling) => spelling,
            Dtype::Datetime { unit, .. } => {
                let (count, at) = self.date_count(doc, root, r, base, n, elem)?;
                return Ok(FrameCell::at(count.and_then(|count| iso_time(count, unit)).map(Value::Str), at));
            }
            _ => return Ok(FrameCell::nowhere()),
        };
        let Some((ty, width)) = shapes::element(spelling) else { return Ok(FrameCell::nowhere()) };
        let (read, at) = self.element_at(doc, root, r, base, n, elem, &ty, width)?;
        // A NaN is how pandas writes a number it has not got. The bytes are
        // still there, so the cell still says where they are.
        Ok(FrameCell::at(
            match read {
                Some(Value::Float(f)) if f.is_nan() => None,
                other => other,
            },
            at,
        ))
    }

    /// One value of a run of dates, as the whole number it is written as, and
    /// where that number is.
    #[allow(clippy::too_many_arguments)]
    fn date_count<S: Source>(&mut self, doc: &Document<S>, root: &[usize], r: &Resolved, base: u64, n: &Numbers, elem: u64) -> R<(Option<i64>, CellAt)> {
        let Dtype::Datetime { spelling, .. } = n.dtype else { return Ok((None, CellAt::Nowhere)) };
        let Some((_, width)) = shapes::element(spelling) else { return Ok((None, CellAt::Nowhere)) };
        let endian = if spelling.starts_with('>') { Endian::Big } else { Endian::Little };
        let ty = Ty::Int { bits: 64, endian };
        let (read, at) = self.element_at(doc, root, r, base, n, elem, &ty, width)?;
        Ok((
            match read {
                Some(Value::Int(count)) => i64::try_from(count).ok(),
                _ => None,
            },
            at,
        ))
    }

    /// Element `elem` of a run, read as `ty`, and where it is: a run of the
    /// file at protocol 3 and up, and a run of the space the numbers were
    /// spelled into below it.
    #[allow(clippy::too_many_arguments)]
    fn element_at<S: Source>(
        &mut self,
        doc: &Document<S>,
        root: &[usize],
        r: &Resolved,
        base: u64,
        n: &Numbers,
        elem: u64,
        ty: &Ty,
        width: u64,
    ) -> R<(Option<Value>, CellAt)> {
        let Some(into) = elem.checked_mul(width) else { return Ok((None, CellAt::Nowhere)) };
        self.at_bytes(doc, root, r, base, n, into, ty, width)
    }

    /// The same, `into` bytes into the run rather than at element `elem`.
    ///
    /// A column of a structured run is not at a multiple of its own width: a
    /// record of an `i4` and an `f8` puts the second column four bytes in and
    /// the next record's first column twelve. So the offset is worked out by
    /// the caller and handed over, and reading one value is the same work
    /// either way.
    #[allow(clippy::too_many_arguments)]
    fn at_bytes<S: Source>(
        &mut self,
        doc: &Document<S>,
        root: &[usize],
        r: &Resolved,
        base: u64,
        n: &Numbers,
        into: u64,
        ty: &Ty,
        width: u64,
    ) -> R<(Option<Value>, CellAt)> {
        if let Some(spelled) = &n.spelled {
            return self.space_value(doc, root, spelled, into, ty, width);
        }
        let Some(at) = (n.at as u64).checked_add(into) else { return Ok((None, CellAt::Nowhere)) };
        let offset = base + at * 8;
        let size = width * 8;
        let one = Resolved { offset, cursor: offset, limit: offset + size, size: Some(size), ..r.clone() };
        let read = self.primitive_value(doc, &[], &one, ty, size)?;
        Ok((Some(read), CellAt::Bytes { space: r.space, offset_bits: offset, size_bits: size }))
    }

    /// One value out of a run the file did not write as bytes.
    ///
    /// Protocol 2 writes an array's numbers as the latin-1 text they spell and
    /// protocol 0 escapes that again, so at neither protocol are they in the
    /// file. The node at `at` opens them as a space, and a value is read out
    /// of that space by the same machinery every other field uses, at the
    /// offset it sits at there. Nothing is copied: the space holds the bytes
    /// once, and every cell of every row reads the same buffer.
    ///
    /// `at` is counted from the pickle field, so the node's own path is that
    /// under the pickle's root.
    ///
    /// The address the cell gets is a run of that space rather than of the
    /// file, and is numbered the way every other address in the space is: the
    /// space's own id, counted from its own start.
    #[allow(clippy::too_many_arguments)]
    fn space_value<S: Source>(&mut self, doc: &Document<S>, root: &[usize], at: &[usize], into: u64, ty: &Ty, width: u64) -> R<(Option<Value>, CellAt)> {
        let path = [root, at].concat();
        self.resolve(doc, &path)?;
        let super::space::Opened::Space(id) = self.open_space_at(doc, &path)? else {
            return fail("this array's numbers did not open");
        };
        let Some(held) = self.spaces.buf(id).cloned() else { return fail("this array's numbers are no longer open") };
        let size = width * 8;
        let Some(offset) = into.checked_mul(8) else { return Ok((None, CellAt::Nowhere)) };
        if offset.checked_add(size).is_none_or(|end| end > held.len() as u64 * 8) {
            return Ok((None, CellAt::Nowhere));
        }
        let run = Document::new(crate::source::ArcSource(held));
        let r = self.memo[&path].clone();
        let one = Resolved { offset, cursor: offset, limit: offset + size, size: Some(size), space: 0, ..r };
        let read = self.primitive_value(&run, &[], &one, ty, size)?;
        Ok((Some(read), CellAt::Bytes { space: id, offset_bits: offset, size_bits: size }))
    }

    /// The label at position `c` of the axis that names a frame's columns.
    #[allow(clippy::too_many_arguments)]
    fn label_at<S: Source>(&mut self, doc: &Document<S>, root: &[usize], r: &Resolved, base: u64, found: &Match, axis: &Captured, c: u64) -> R<Option<String>> {
        if is_range(axis) {
            let Some(state) = index_state(axis) else { return Ok(None) };
            let (start, _, step) = self.range_bounds(doc, r, base, state)?;
            let (Some(start), Some(step)) = (start, step) else { return Ok(None) };
            return Ok(Some((start + step * i128::from(c)).to_string()));
        }
        let Some(state) = index_state(axis) else { return Ok(None) };
        let Some(values) = self.index_values(doc, r, base, state)? else { return Ok(None) };
        let Some(held) = values_of(&found, values) else { return Ok(None) };
        Ok(match self.one_value(doc, root, r, base, found, &held, 0, c)?.value {
            Some(Value::Str(said)) => Some(said),
            Some(Value::Int(n)) => Some(n.to_string()),
            Some(Value::UInt(n)) => Some(n.to_string()),
            _ => None,
        })
    }
}

/// What one value of a run is, in the header.
///
/// A column of pickled objects is whatever was pickled into it. `str` where
/// every value is text, which is what such a column usually holds and what a
/// reader wants told apart from a column of numbers; `object`, pandas' own
/// word for the dtype, where the values are dates, lists or a mixture.
fn word_of(held: &Values) -> String {
    match held {
        Values::Numbers(n) => dtype_word(n.dtype),
        Values::Texts(items) => match items.iter().all(is_text) {
            true => "str".into(),
            false => "object".into(),
        },
        Values::Coded(..) => "category".into(),
    }
}

/// Whether one value of a column of objects is text, or the missing entry
/// between two of them, which pandas writes as `None`.
fn is_text(value: &Captured) -> bool {
    matches!(value.kind, Kind::Text { .. } | Kind::Ref(Names::Text { .. }) | Kind::None)
}

/// Whether an object is one of scipy's sparse matrices, which keep their
/// values, their positions and their shape as attributes.
pub(super) fn is_sparse(object: &Captured) -> bool {
    let Kind::Instance { class, state: Some(_) } = &object.kind else { return false };
    class_name(class).is_some_and(|name| name.ends_with("_matrix") || name.ends_with("_array"))
}

impl Evaluator {
}

/// The numbers and the mask of a masked array, which are the two runs a cell
/// of its table is read from.
pub(super) fn masked_of(object: &Captured) -> Option<(&Captured, &Captured)> {
    let Kind::Masked { data, mask, .. } = &object.kind else { return None };
    Some((data, mask))
}

/// How many rows a masked array's table has and how wide one is.
///
/// The same chunking an array's own table gets: one value a row for a shape of
/// one dimension, and otherwise as many columns as the axis the run moves
/// along fastest. Nothing for an array of no dimensions, which is one value
/// and not a table, and nothing for one of no values.
pub(super) fn masked_shape(dimensions: &[u64], fortran_order: bool) -> Option<(u64, u64)> {
    if dimensions.is_empty() || dimensions.contains(&0) {
        return None;
    }
    let count: u64 = dimensions.iter().product();
    let columns = match (dimensions.len(), fortran_order) {
        (1, _) => 1,
        (_, true) => *dimensions.first()?,
        (_, false) => *dimensions.last()?,
    };
    Some((count / columns.max(1), columns))
}
