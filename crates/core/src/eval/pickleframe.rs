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
//! Nothing here runs anything, and nothing here reads the file: every part is
//! a value a Familiar Pickle Form already matched and placed, and this is the
//! shape of them. [`picklecells`](super::picklecells) reads the values out.

use crate::formats::pickle::familiar::{Dtype, Kind, Names, Shape, Value as Captured};

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
pub(super) const NO_CATEGORY: i128 = -1;
/// What the table calls the column holding the frame's row labels, when the
/// index has no name of its own, and what it calls a series' one value column
/// when the series has no name.
pub(super) const INDEX_COLUMN: &str = "index";
pub(super) const VALUE_COLUMN: &str = "value";
/// What one row of a frame is.
pub(super) const ROW_WORD: &str = "row";
/// What an index's dictionary calls its parts.
pub(super) const DATA: &str = "data";
pub(super) const NAME: &str = "name";
pub(super) const START: &str = "start";
pub(super) const STOP: &str = "stop";
pub(super) const STEP: &str = "step";

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
pub(super) fn class_name(value: &Captured) -> Option<&str> {
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
    // `_unpickle_block` is handed the number of axes as well; the partial
    // pandas 1.3 writes already carries it and is handed two.
    let (values, placement) = match items.as_slice() {
        [values, placement] | [values, placement, _] => (values, placement),
        _ => return None,
    };
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
    pub(super) fn at(&self, column: u64, row: u64) -> Option<u64> {
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
        // And an array of pandas' own is an ordinary object with the array it
        // wraps among its attributes, which is where a datetime column was
        // before 1.5.
        Kind::Instance { class, state: Some(state) } if class_name(class)?.ends_with("Array") => {
            let Kind::Dict(entries) = &state.kind else { return None };
            entries.iter().find_map(|(_, v)| values_of(v))
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
pub(super) fn dtype_word(dtype: &Dtype) -> String {
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
pub(super) fn range_len(start: i128, stop: i128, step: i128) -> Option<u64> {
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
