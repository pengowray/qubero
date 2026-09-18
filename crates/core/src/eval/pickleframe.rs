//! A pandas frame, read as the table it holds rather than as the program that
//! rebuilds it.
//!
//! What a reader opens a pickled frame for is the data, and the data is not
//! where the tree puts it. A frame's values are in blocks, each block is an
//! array of shape `(columns in that block, rows)`, the block says which of the
//! frame's columns its rows are, and the column names are in an index beside
//! the blocks. A categorical column is codes into another array again, and a
//! `RangeIndex` is not written down at all: it is a start, a stop and a step.
//! So a cell of the table is not a run of bytes a reader can index into, and
//! this file works one out.
//!
//! Nothing here runs anything. Every part is a value a Familiar Pickle Form
//! already matched and placed, and this reads them in the order pandas wrote
//! them.

use std::sync::Arc;

use super::pickletree::{spot, Part, Says, MOST_SHOWN_TEXT as MOST_SHOWN};
use super::*;
use crate::formats::pickle::familiar::{Dtype, Kind, Names, Shape, Value as Captured};
use crate::formats::pickle::shapes;
use crate::template::{Cells, TableShape};

/// What the manager under a frame or a series is made of, however the release
/// that wrote it spelled the manager.
pub(super) struct Frame<'a> {
    /// The axis that names the frame's columns. A series has one axis and so
    /// has no such thing: its one column is named after the series.
    pub(super) columns: Option<&'a Captured>,
    /// The axis that names the rows.
    pub(super) index: &'a Captured,
    pub(super) blocks: Vec<Block<'a>>,
}

pub(super) struct Block<'a> {
    pub(super) values: &'a Captured,
    /// Which of the frame's columns this block's rows are, as the start and
    /// the step of the slice pandas placed it by.
    pub(super) placement: (i128, i128),
}

/// What one column of a frame is made of, once the wrappers pandas puts round
/// an array have been read through.
pub(super) enum Values<'a> {
    /// A run of numbers in the file.
    Numbers(Numbers<'a>),
    /// Values pickled after the array that holds them, which is the `O8`
    /// dtype: text, `None`, and names for text written earlier.
    Texts(&'a [Captured]),
    /// A run of codes, each naming one of a list of categories. `-1` is the
    /// code pandas writes where a categorical has no value.
    Coded(Numbers<'a>, &'a [Captured]),
}

/// A run of numbers: where it is, how one of them is read, and how they are
/// laid out.
pub(super) struct Numbers<'a> {
    pub(super) at: usize,
    pub(super) dtype: &'a Dtype,
    pub(super) dimensions: &'a [u64],
    pub(super) fortran_order: bool,
}

/// The code a categorical writes where it has no value.
const NO_CATEGORY: i128 = -1;
/// What the table calls the column holding the frame's row labels, when the
/// index has no name of its own, and what it calls a series' one value column
/// when the series has no name.
const INDEX_COLUMN: &str = "index";
const VALUE_COLUMN: &str = "value";
/// What one row of a frame is.
const ROW_WORD: &str = "row";
/// What an index's dictionary calls its parts.
const DATA: &str = "data";
const NAME: &str = "name";
const START: &str = "start";
const STOP: &str = "stop";
const STEP: &str = "step";

/// The whole dotted path of the class a value is an object of, or the callable
/// a call named.
fn class_path(value: &Captured) -> Option<&str> {
    match &value.kind {
        Kind::Class { path, .. } => Some(path),
        Kind::Instance { class, .. } => class_path(class),
        Kind::Made { callable, .. } => class_path(callable),
        _ => None,
    }
}

/// The last word of a dotted path, which is the class's own name. The module
/// in front of it moves between releases and the name does not.
fn class_name(value: &Captured) -> Option<&str> {
    Some(class_path(value)?.rsplit_once('.')?.1)
}

/// The frame or the series a matched pickle holds, as the parts of its
/// manager.
///
/// Read without touching the file. Which attribute holds the manager is
/// decided by what the value is rather than by the key's spelling, because the
/// keys are text in the file and this is asked of every node on its way to the
/// screen.
pub(super) fn frame_of(object: &Captured) -> Option<Frame<'_>> {
    let Kind::Instance { class, state: Some(state) } = &object.kind else { return None };
    let series = match class_name(class)? {
        "Series" => true,
        "DataFrame" => false,
        _ => return None,
    };
    let Kind::Dict(entries) = &state.kind else { return None };
    let manager = entries.iter().find_map(|(_, v)| managed(v))?;
    let (axes, blocks) = manager;
    // A frame names its columns and then its rows; a series has the rows
    // alone. Anything else is a manager this file has not seen.
    let (columns, index) = match (series, axes.len()) {
        (false, 2) => (Some(&axes[0]), &axes[1]),
        (true, 1) => (None, &axes[0]),
        _ => return None,
    };
    Some(Frame { columns, index, blocks })
}

/// The axes and the blocks of a manager, whichever of the two spellings it is.
///
/// pandas 1.5 and up call `_unpickle_block` once a block and hand the manager
/// the blocks and the axes; 1.1 and every series hand it a tuple whose last
/// item is a dictionary of the blocks under a version key. The two say the
/// same thing.
fn managed(value: &Captured) -> Option<(&[Captured], Vec<Block<'_>>)> {
    match &value.kind {
        // The call: (blocks, axes).
        Kind::Made { what: Shape::Object, callable, items, .. } if class_name(callable)? == "BlockManager" => {
            let [blocks, axes] = items.as_slice() else { return None };
            let (Kind::Tuple(blocks), Kind::List(axes)) = (&blocks.kind, &axes.kind) else { return None };
            let read: Option<Vec<Block>> = blocks.iter().map(called_block).collect();
            Some((axes, read?))
        }
        // The tuple: the axes first, and the blocks in the dictionary last.
        Kind::Instance { class, state: Some(state) } if matches!(class_name(class)?, "BlockManager" | "SingleBlockManager") => {
            let Kind::Tuple(items) = &state.kind else { return None };
            let (Some(Kind::List(axes)), Some(held)) = (items.first().map(|v| &v.kind), items.last()) else { return None };
            // One entry, whose key is the version and whose value holds the
            // axes and the blocks. The axes there are a name for the list this
            // tuple already holds, so the blocks are all that is read.
            let Kind::Dict(versioned) = &held.kind else { return None };
            let [(_, under)] = versioned.as_slice() else { return None };
            let Kind::Dict(parts) = &under.kind else { return None };
            let listed = parts.iter().find_map(|(_, v)| match &v.kind {
                Kind::List(rows) if rows.iter().all(|r| matches!(r.kind, Kind::Dict(_))) => Some(rows),
                _ => None,
            })?;
            // A block's values may be a name for an array written earlier,
            // which the tuple's second item holds in the order it wrote them.
            let written = items.get(1).map(|v| &v.kind);
            let arrays: &[Captured] = match written {
                Some(Kind::List(arrays)) => arrays,
                _ => &[],
            };
            let read: Option<Vec<Block>> = listed.iter().map(|row| stated_block(row, arrays)).collect();
            Some((axes, read?))
        }
        _ => None,
    }
}

/// One block as `_unpickle_block(values, placement, ndim)` wrote it.
fn called_block(value: &Captured) -> Option<Block<'_>> {
    let Kind::Made { what: Shape::Block, items, .. } = &value.kind else { return None };
    let [values, placement, _] = items.as_slice() else { return None };
    Some(Block { values, placement: slice_of(placement)? })
}

/// One block as the older managers wrote it: a dictionary of its values and
/// where they sit, found by what each is rather than by the key.
fn stated_block<'a>(value: &'a Captured, arrays: &'a [Captured]) -> Option<Block<'a>> {
    let Kind::Dict(entries) = &value.kind else { return None };
    let placement = entries.iter().find_map(|(_, v)| slice_of(v))?;
    let values = entries.iter().find_map(|(_, v)| match &v.kind {
        Kind::Array { .. } | Kind::Objects { .. } | Kind::Made { .. } | Kind::Instance { .. } => Some(v),
        // A name for one of the values the manager's tuple already wrote,
        // which is where the older managers keep a block's array.
        Kind::Ref(Names::Made { at, .. }) => arrays.iter().find(|a| a.at == *at),
        _ => None,
    })?;
    Some(Block { values, placement })
}

/// The start and the step of a `slice`, which is how a block says which of the
/// frame's columns it holds. A step of nought or less places nothing.
fn slice_of(value: &Captured) -> Option<(i128, i128)> {
    let Kind::Object { what: Shape::Slice, items, .. } = &value.kind else { return None };
    let [start, _, step] = items.as_slice() else { return None };
    let start = match start.kind {
        Kind::Int { value, .. } => value,
        Kind::None => 0,
        _ => return None,
    };
    let step = match step.kind {
        Kind::Int { value, .. } => value,
        Kind::None => 1,
        _ => return None,
    };
    (step > 0).then_some((start, step))
}

impl<'a> Frame<'a> {
    /// The column of the frame at `c`: which block holds it, and which of that
    /// block's rows it is.
    pub(super) fn column(&self, c: u64) -> Option<(&Block<'a>, u64)> {
        for block in &self.blocks {
            let (start, step) = block.placement;
            let inside = i128::from(c) - start;
            if inside < 0 || inside % step != 0 {
                continue;
            }
            let j = u64::try_from(inside / step).ok()?;
            let held = values_of(block.values)?;
            if j < held.rows_and_columns()?.1 {
                return Some((block, j));
            }
        }
        None
    }
}

impl Values<'_> {
    /// How many rows and how many of the frame's columns this run holds.
    ///
    /// A block of a frame is written the other way up from the frame: its
    /// shape is `(columns, rows)`, so the frame's rows run along the second
    /// axis. A series has one column and one dimension.
    pub(super) fn rows_and_columns(&self) -> Option<(u64, u64)> {
        let dimensions = match self {
            Values::Numbers(n) | Values::Coded(n, _) => n.dimensions,
            Values::Texts(items) => return Some((items.len() as u64, 1)),
        };
        match dimensions {
            [rows] => Some((*rows, 1)),
            [columns, rows] => Some((*rows, *columns)),
            _ => None,
        }
    }

    /// Where value `(column, row)` of this run sits, counted in values.
    fn at(&self, column: u64, row: u64) -> Option<u64> {
        let (rows, columns) = self.rows_and_columns()?;
        if row >= rows || column >= columns {
            return None;
        }
        let fortran = match self {
            Values::Numbers(n) | Values::Coded(n, _) => n.fortran_order,
            Values::Texts(_) => false,
        };
        match fortran {
            // Down the first axis, which is the block's columns.
            true => row.checked_mul(columns)?.checked_add(column),
            false => column.checked_mul(rows)?.checked_add(row),
        }
    }
}

/// What a block's or an index's values are, read through whatever pandas
/// wrapped them in.
pub(super) fn values_of(value: &Captured) -> Option<Values<'_>> {
    match &value.kind {
        Kind::Array { at, dtype, dimensions, fortran_order, .. } => {
            Some(Values::Numbers(Numbers { at: *at, dtype, dimensions, fortran_order: *fortran_order }))
        }
        Kind::Objects { items, .. } => Some(Values::Texts(items)),
        // `__pyx_unpickle_NDArrayBacked(cls, checksum, None)` and a BUILD that
        // hands it the array it wraps. Which class it is says how to read it.
        Kind::Made { callable, items, state: Some(state), .. } if class_name(callable)? == "__pyx_unpickle_NDArrayBacked" => {
            let Kind::Tuple(held) = &state.kind else { return None };
            match class_name(items.first()?)? {
                "StringArray" | "DatetimeArray" => held.iter().find_map(values_of),
                "Categorical" => {
                    let codes = held.iter().find_map(|v| match values_of(v)? {
                        Values::Numbers(n) => Some(n),
                        _ => None,
                    })?;
                    let names = held.iter().find_map(|v| match &v.kind {
                        Kind::Instance { .. } => categories(v),
                        _ => None,
                    })?;
                    Some(Values::Coded(codes, names))
                }
                _ => None,
            }
        }
        // Before pandas 1.5 a categorical is an ordinary object with its codes
        // and its dtype among its attributes.
        Kind::Instance { class, state: Some(state) } if class_name(class)? == "Categorical" => {
            let Kind::Dict(entries) = &state.kind else { return None };
            let codes = entries.iter().find_map(|(_, v)| match values_of(v)? {
                Values::Numbers(n) => Some(n),
                _ => None,
            })?;
            let names = entries.iter().find_map(|(_, v)| categories(v))?;
            Some(Values::Coded(codes, names))
        }
        _ => None,
    }
}

/// The categories a `CategoricalDtype` names, which are the values of the
/// index it holds.
fn categories(dtype: &Captured) -> Option<&[Captured]> {
    let Kind::Instance { class, state: Some(state) } = &dtype.kind else { return None };
    if class_name(class)? != "CategoricalDtype" {
        return None;
    }
    let Kind::Dict(entries) = &state.kind else { return None };
    let index = entries.iter().find_map(|(_, v)| match &v.kind {
        Kind::Made { callable, .. } if class_name(callable) == Some("_new_Index") => Some(v),
        _ => None,
    })?;
    let Kind::Made { items, .. } = &index.kind else { return None };
    let Kind::Dict(state) = &items.get(1)?.kind else { return None };
    // The categories are text, spelled as a bare array of objects up to
    // pandas 2.x and wrapped in a `StringArray` in 3.0.
    state.iter().find_map(|(_, v)| match values_of(v)? {
        Values::Texts(items) => Some(items),
        _ => None,
    })
}

/// Whether an index is one counted out rather than written down, which is a
/// `RangeIndex`.
pub(super) fn is_range(index: &Captured) -> bool {
    let Kind::Made { callable, items, .. } = &index.kind else { return false };
    class_name(callable) == Some("_new_Index") && items.first().and_then(class_name) == Some("RangeIndex")
}

/// The dictionary an index was rebuilt from, which holds either its values or
/// the start, stop and step it is counted out from.
pub(super) fn index_state(index: &Captured) -> Option<&Vec<(Captured, Captured)>> {
    let Kind::Made { callable, items, .. } = &index.kind else { return None };
    if !matches!(class_name(callable)?, "_new_Index" | "_new_DatetimeIndex") {
        return None;
    }
    match &items.get(1)?.kind {
        Kind::Dict(entries) => Some(entries),
        _ => None,
    }
}

/// What the column header says one value of a column is, which is pandas'
/// own word for it rather than NumPy's letters.
fn dtype_word(dtype: &Dtype) -> String {
    let Dtype::Plain(spelling) = dtype else { return dtype.name() };
    let (kind, width) = spelling[1..].split_at(1);
    let bits = width.parse::<u32>().unwrap_or(0) * 8;
    match kind {
        "b" => "bool".into(),
        "i" => format!("int{bits}"),
        "u" => format!("uint{bits}"),
        "f" => format!("float{bits}"),
        "c" => format!("complex{bits}"),
        _ => spelling.clone(),
    }
}

/// How long a Python `range` of these bounds is.
fn range_len(start: i128, stop: i128, step: i128) -> Option<u64> {
    if step == 0 {
        return None;
    }
    let span = stop.checked_sub(start)?;
    let reach = match step > 0 {
        true => span + step - 1,
        false => span + step + 1,
    };
    u64::try_from((reach / step).max(0)).ok()
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
        let Some(frame) = frame_of(object) else { return Ok(None) };
        let r = self.memo[&root].clone();
        let base = r.offset;
        let Some(rows) = self.index_len(doc, &r, base, frame.index)? else { return Ok(None) };
        let wide = match frame.columns {
            Some(axis) => match self.index_len(doc, &r, base, axis)? {
                Some(wide) => wide,
                None => return Ok(None),
            },
            None => 1,
        };
        // The index is the first column, so that a row can be read off against
        // the label the frame files it under.
        let mut names: Vec<Arc<str>> = vec![self.axis_name(doc, &r, base, frame.index)?.unwrap_or_else(|| INDEX_COLUMN.into()).into()];
        let mut units: Vec<Arc<str>> = vec![self.axis_word(doc, &r, base, frame.index)?.into()];
        for c in 0..wide {
            let Some((block, j)) = frame.column(c) else { return Ok(None) };
            let Some(held) = values_of(block.values) else { return Ok(None) };
            let Some((held_rows, _)) = held.rows_and_columns() else { return Ok(None) };
            if held_rows != rows {
                return Ok(None);
            }
            let _ = j;
            names.push(match frame.columns {
                Some(axis) => self.label_at(doc, &r, base, axis, c)?.unwrap_or_else(|| format!("column {c}")).into(),
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
    pub fn pickle_cells<S: Source>(&mut self, doc: &Document<S>, path: &[usize], from: u64, to: u64) -> R<Vec<Vec<Option<Value>>>> {
        let (root, found) = self.pickle_doc(doc, path)?;
        let Some((_, Part::Value(object))) = spot(&found, &path[root.len()..]) else { return fail("not a frame") };
        let Some(frame) = frame_of(object) else { return fail("not a frame") };
        let r = self.memo[&root].clone();
        let base = r.offset;
        let Some(rows) = self.index_len(doc, &r, base, frame.index)? else { return fail("this frame does not say how many rows it has") };
        let wide = match frame.columns {
            Some(axis) => self.index_len(doc, &r, base, axis)?.unwrap_or(0),
            None => 1,
        };
        // Which block each column is in, worked out once rather than per cell.
        let placed: Vec<(u64, Values)> = (0..wide)
            .filter_map(|c| {
                let (block, j) = frame.column(c)?;
                Some((j, values_of(block.values)?))
            })
            .collect();
        if placed.len() as u64 != wide {
            return fail("this frame's blocks do not cover its columns");
        }
        let mut out = Vec::new();
        for row in from..to.min(rows) {
            let mut cells = vec![self.index_label(doc, &r, base, frame.index, row)?];
            for (j, held) in &placed {
                cells.push(self.one_value(doc, &r, base, held, *j, row)?);
            }
            out.push(cells);
        }
        Ok(out)
    }

    /// How many labels an index has.
    fn index_len<S: Source>(&mut self, doc: &Document<S>, r: &Resolved, base: u64, index: &Captured) -> R<Option<u64>> {
        let Some(state) = index_state(index) else { return Ok(None) };
        if is_range(index) {
            let (start, stop, step) = self.range_bounds(doc, r, base, state)?;
            return Ok(match (start, stop, step) {
                (Some(start), Some(stop), Some(step)) => range_len(start, stop, step),
                _ => None,
            });
        }
        let Some(values) = self.index_values(doc, r, base, state)? else { return Ok(None) };
        let Some(held) = values_of(values) else { return Ok(None) };
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
    fn range_bounds<S: Source>(
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
    fn axis_word<S: Source>(&mut self, doc: &Document<S>, r: &Resolved, base: u64, index: &Captured) -> R<String> {
        if is_range(index) {
            return Ok("int64".into());
        }
        let Some(state) = index_state(index) else { return Ok(String::new()) };
        let Some(values) = self.index_values(doc, r, base, state)? else { return Ok(String::new()) };
        Ok(values_of(values).map(|held| word_of(&held)).unwrap_or_default())
    }

    /// What a series calls its one column.
    fn series_name<S: Source>(&self, doc: &Document<S>, r: &Resolved, base: u64, object: &Captured) -> R<Option<String>> {
        let Kind::Instance { state: Some(state), .. } = &object.kind else { return Ok(None) };
        let Kind::Dict(entries) = &state.kind else { return Ok(None) };
        for (key, value) in entries {
            // `_name` since pandas 1.5, `name` before it.
            if matches!(self.pickle_text(doc, r, base, key)?.as_deref(), Some("_name") | Some(NAME)) {
                if let Some(said) = self.pickle_text(doc, r, base, value)? {
                    return Ok(Some(said));
                }
            }
        }
        Ok(None)
    }

    /// The label the index files row `row` under.
    fn index_label<S: Source>(&mut self, doc: &Document<S>, r: &Resolved, base: u64, index: &Captured, row: u64) -> R<Option<Value>> {
        let Some(state) = index_state(index) else { return Ok(None) };
        if is_range(index) {
            let (start, _, step) = self.range_bounds(doc, r, base, state)?;
            let (Some(start), Some(step)) = (start, step) else { return Ok(None) };
            let at = start.checked_add(step.checked_mul(i128::from(row)).unwrap_or(0));
            return Ok(at.map(Value::Int));
        }
        let Some(values) = self.index_values(doc, r, base, state)? else { return Ok(None) };
        let Some(held) = values_of(values) else { return Ok(None) };
        self.one_value(doc, r, base, &held, 0, row)
    }

    /// One value of a column, or nothing where the frame has none.
    fn one_value<S: Source>(
        &mut self,
        doc: &Document<S>,
        r: &Resolved,
        base: u64,
        held: &Values,
        column: u64,
        row: u64,
    ) -> R<Option<Value>> {
        let Some(elem) = held.at(column, row) else { return Ok(None) };
        match held {
            Values::Numbers(n) => self.number_at(doc, r, base, n, elem),
            Values::Texts(items) => {
                let Some(item) = items.get(elem as usize) else { return Ok(None) };
                Ok(self.pickle_text(doc, r, base, item)?.map(Value::Str))
            }
            Values::Coded(codes, names) => {
                let code = match self.number_at(doc, r, base, codes, elem)? {
                    Some(Value::Int(code)) => code,
                    Some(Value::UInt(code)) => i128::try_from(code).unwrap_or(NO_CATEGORY),
                    _ => return Ok(None),
                };
                if code == NO_CATEGORY || code < 0 {
                    return Ok(None);
                }
                let Some(item) = usize::try_from(code).ok().and_then(|at| names.get(at)) else { return Ok(None) };
                Ok(self.pickle_text(doc, r, base, item)?.map(Value::Str))
            }
        }
    }

    /// One number of a run, read as the type its dtype says.
    fn number_at<S: Source>(&mut self, doc: &Document<S>, r: &Resolved, base: u64, n: &Numbers, elem: u64) -> R<Option<Value>> {
        let Dtype::Plain(spelling) = n.dtype else { return Ok(None) };
        let Some((ty, width)) = shapes::element(spelling) else { return Ok(None) };
        let Some(at) = (n.at as u64).checked_add(elem.checked_mul(width).unwrap_or(u64::MAX)) else { return Ok(None) };
        let offset = base + at * 8;
        let size = width * 8;
        let one = Resolved { offset, cursor: offset, limit: offset + size, size: Some(size), ..r.clone() };
        let read = self.primitive_value(doc, &[], &one, &ty, size)?;
        // A NaN is how pandas writes a number it has not got.
        Ok(match read {
            Value::Float(f) if f.is_nan() => None,
            other => Some(other),
        })
    }

    /// The label at position `c` of the axis that names a frame's columns.
    fn label_at<S: Source>(&mut self, doc: &Document<S>, r: &Resolved, base: u64, axis: &Captured, c: u64) -> R<Option<String>> {
        if is_range(axis) {
            let Some(state) = index_state(axis) else { return Ok(None) };
            let (start, _, step) = self.range_bounds(doc, r, base, state)?;
            let (Some(start), Some(step)) = (start, step) else { return Ok(None) };
            return Ok(Some((start + step * i128::from(c)).to_string()));
        }
        let Some(state) = index_state(axis) else { return Ok(None) };
        let Some(values) = self.index_values(doc, r, base, state)? else { return Ok(None) };
        let Some(held) = values_of(values) else { return Ok(None) };
        Ok(match self.one_value(doc, r, base, &held, 0, c)? {
            Some(Value::Str(said)) => Some(said),
            Some(Value::Int(n)) => Some(n.to_string()),
            Some(Value::UInt(n)) => Some(n.to_string()),
            _ => None,
        })
    }
}

/// What one value of a run is, in the header.
fn word_of(held: &Values) -> String {
    match held {
        Values::Numbers(n) => dtype_word(n.dtype),
        Values::Texts(_) => "str".into(),
        Values::Coded(..) => "category".into(),
    }
}

/// Whether an object is one of scipy's sparse matrices, which keep their
/// values, their positions and their shape as attributes.
pub(super) fn is_sparse(object: &Captured) -> bool {
    let Kind::Instance { class, state: Some(_) } = &object.kind else { return false };
    class_name(class).is_some_and(|name| name.ends_with("_matrix") || name.ends_with("_array"))
}

impl Evaluator {
    /// What one summary row of a library object says.
    ///
    /// Worked out here rather than where the rows are listed, because every
    /// one of these reads the file: a column's name is text somewhere else in
    /// it, and a counted index is not in it at all.
    pub(super) fn pickle_summary<S: Source>(
        &mut self,
        doc: &Document<S>,
        root: &[usize],
        path: &[usize],
        object: &Captured,
        says: Says,
    ) -> R<String> {
        let r = self.memo[root].clone();
        let base = r.offset;
        // The row belongs to the object, which is the node above this one.
        let of = &path[..path.len() - 1];
        match says {
            Says::Columns | Says::Rows | Says::Index | Says::Dtypes => {
                let Some(shape) = self.frame_shape(doc, of)? else { return Ok(String::new()) };
                let Some(Cells::Computed { rows }) = shape.cells else { return Ok(String::new()) };
                Ok(match says {
                    Says::Rows => rows.to_string(),
                    Says::Columns => cut(&shape.names[1..].join(", ")),
                    Says::Dtypes => cut(
                        &shape.names[1..]
                            .iter()
                            .zip(&shape.units[1..])
                            .map(|(name, word)| format!("{name} {word}"))
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                    _ => self.index_summary(doc, &r, base, object)?,
                })
            }
            Says::Shape | Says::Stored | Says::Format => self.sparse_summary(doc, &r, base, object, says),
        }
    }

    /// What an index is and where its labels run from and to.
    fn index_summary<S: Source>(&mut self, doc: &Document<S>, r: &Resolved, base: u64, object: &Captured) -> R<String> {
        let Some(frame) = frame_of(object) else { return Ok(String::new()) };
        let kind = index_kind(frame.index).unwrap_or("Index");
        if is_range(frame.index) {
            let Some(state) = index_state(frame.index) else { return Ok(kind.to_string()) };
            let (start, stop, step) = self.range_bounds(doc, r, base, state)?;
            let (Some(start), Some(stop), Some(step)) = (start, stop, step) else { return Ok(kind.to_string()) };
            let by = if step == 1 { String::new() } else { format!(" by {step}") };
            return Ok(format!("{kind} {start} to {stop}{by}"));
        }
        let Some(rows) = self.index_len(doc, r, base, frame.index)? else { return Ok(kind.to_string()) };
        if rows == 0 {
            return Ok(format!("{kind} of no labels"));
        }
        // The first and the last label, which is what a reader wants of an
        // index of dates and is still true of one of names.
        let first = self.index_label(doc, r, base, frame.index, 0)?;
        let last = self.index_label(doc, r, base, frame.index, rows - 1)?;
        Ok(match (label_text(&first), label_text(&last)) {
            (Some(first), Some(last)) if rows > 1 => format!("{kind} {first} to {last}"),
            (Some(first), _) => format!("{kind} {first}"),
            _ => format!("{kind} of {rows} labels"),
        })
    }

    /// What a sparse matrix holds: how big it is, how many values it stores,
    /// and which of scipy's layouts it is.
    fn sparse_summary<S: Source>(
        &mut self,
        doc: &Document<S>,
        r: &Resolved,
        base: u64,
        object: &Captured,
        says: Says,
    ) -> R<String> {
        let Kind::Instance { class, state: Some(state) } = &object.kind else { return Ok(String::new()) };
        if says == Says::Format {
            // `csr_matrix` is the CSR layout, and the class's name is where
            // the file says which layout it is.
            let name = class_name(class).unwrap_or_default();
            return Ok(name.rsplit_once('_').map(|(kind, _)| kind.to_string()).unwrap_or_default());
        }
        let Kind::Dict(entries) = &state.kind else { return Ok(String::new()) };
        if says == Says::Shape {
            // The one attribute that is a tuple of whole numbers.
            let shape = entries.iter().find_map(|(_, v)| match &v.kind {
                Kind::Tuple(items) if !items.is_empty() && items.iter().all(|x| matches!(x.kind, Kind::Int { .. })) => Some(items),
                _ => None,
            });
            let Some(shape) = shape else { return Ok(String::new()) };
            let said: Vec<String> = shape
                .iter()
                .map(|x| match x.kind {
                    Kind::Int { value, .. } => value.to_string(),
                    _ => String::new(),
                })
                .collect();
            return Ok(said.join(" x "));
        }
        // How many values are stored, which is the length of the run the
        // matrix calls `data`.
        for (key, value) in entries {
            if self.pickle_text(doc, r, base, key)?.as_deref() != Some("data") {
                continue;
            }
            let Some(held) = values_of(value) else { break };
            let Some((rows, _)) = held.rows_and_columns() else { break };
            return Ok(rows.to_string());
        }
        Ok(String::new())
    }
}

/// Which kind of index this is, by the class `_new_Index` was handed.
fn index_kind(index: &Captured) -> Option<&str> {
    let Kind::Made { items, .. } = &index.kind else { return None };
    class_name(items.first()?)
}

/// A label as a summary shows it.
fn label_text(label: &Option<Value>) -> Option<String> {
    match label {
        Some(Value::Str(said)) => Some(said.clone()),
        Some(Value::Int(n)) => Some(n.to_string()),
        Some(Value::UInt(n)) => Some(n.to_string()),
        Some(Value::Float(f)) => Some(f.to_string()),
        _ => None,
    }
}

/// A summary row read at a glance, cut where it stops being one.
fn cut(said: &str) -> String {
    match said.char_indices().nth(MOST_SHOWN) {
        Some((at, _)) => format!("{}...", &said[..at]),
        None => said.to_string(),
    }
}
