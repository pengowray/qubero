//! The object a pickle builds, as nodes of the tree like any other field.
//!
//! A `Ty::Pickle` field is recognised once, when something first asks what is
//! inside it, and the captured tree is kept beside the memo. This is
//! [`jsontree`](super::jsontree) for a format whose values are nowhere in the
//! file as a run: a pickle is a program, and what it builds is scattered
//! through the opcodes that build it. A Familiar Pickle Form says where every
//! value went (see [`familiar`]), so every value can be a node at the bytes it
//! was written at.
//!
//! The instructions between the values are nodes too. A form fixes them: the
//! `MEMOIZE` after a string and the `SETITEM` that files it under its key are
//! not noise around the data, they are the shape of the data, written down and
//! matched exactly. So every one of them is a field named for what
//! `pickletools` calls it, and a matched file has no byte left over. The run
//! that rebuilds a NumPy array is a couple of dozen of them and one act, so it
//! is one field holding the names and letters the form matched inside it.
//!
//! The leaves are given the ordinary types their bytes are: a `BININT2` is a
//! little-endian `u16` at its two operand bytes. So reading, displaying and
//! editing one is the machinery every other field uses, and only the nodes
//! that hold others keep [`Ty::Pickle`].

use std::sync::Arc;

use super::*;
use crate::formats::pickle::familiar::{self, Dtype, Kind, Match, Names, Storage, Value};
use crate::formats::pickle::shapes;
use crate::codec::Codec;
use crate::template::{Cells, Encoding, Endian::*, Expr as E, PickleShape as Shape, StrLen, Ty as T};

use super::pickleparts::*;
use super::picklesaid::*;

/// A leaf, as the type its bytes are and the bytes its value proper occupies.
/// The opcode that introduced it is not part of that, bar the two values that
/// are written as an opcode and nothing else.
pub(super) fn leaf(value: &Value) -> Option<(T, usize, usize)> {
    Some(match &value.kind {
        Kind::None => (T::enumeration("null", T::u8(), &[(0x4e, "None")]), value.at, 1),
        Kind::Bool(_) => (T::enumeration("bool", T::u8(), &[(0x88, "True"), (0x89, "False")]), value.at, 1),
        // A number written as a line of digits is no type at all: the run is
        // the spelling and there is nothing to read it as. It is a node whose
        // value is the number, with the line beneath it.
        Kind::Int { spelled: true, .. } => return None,
        // BININT1 is unsigned and BININT2 is too; BININT is signed, and so is
        // LONG1, whose run of bytes is as long as the number needs.
        Kind::Int { at, len, .. } => {
            let ty = match len {
                1 => T::u8(),
                2 => T::u16(Little),
                4 => T::i32(Little),
                bytes => T::Int { bits: *bytes as u32 * 8, endian: Little },
            };
            (ty, *at, *len)
        }
        // A float a line spells is the same: `F5.5` is three characters, and
        // eight bytes read from there as a number run past the line.
        Kind::Float { spelled: true, .. } => return None,
        // The one big-endian number in the format.
        Kind::Float { at, len, .. } => (T::F64(Big), *at, *len),
        Kind::Text { at, len } => (T::text(StrLen::Fixed(E::lit(*len as i128)), Encoding::Utf8), *at, *len),
        Kind::Bytes { at, len } => (T::bytes(E::lit(*len as i128)), *at, *len),
        _ => return None,
    })
}

/// The type a run of an array's values reads as: one element repeated by the
/// shape, where an element is one number or, for a structured dtype, a record
/// of named columns with the padding NumPy left between them named too.
fn numbers_ty(dtype: &Dtype, count: u64) -> Option<T> {
    match dtype {
        // A datetime is a count of its unit, and the type NumPy's table gives
        // it says which unit, so a reader of the numbers sees that too.
        Dtype::Plain(spelling) | Dtype::Datetime { spelling, .. } => shapes::run(spelling, count),
        Dtype::Record { columns, width } => {
            let mut fields: Vec<(String, T)> = Vec::new();
            let mut reached = 0u64;
            for column in columns {
                if column.at > reached {
                    fields.push((format!("{PADDING_FIELD} {reached}"), T::bytes(E::lit((column.at - reached) as i128))));
                }
                let (elem, held) = shapes::element(&column.dtype)?;
                fields.push((column.name.clone(), elem));
                reached = column.at + held;
            }
            if *width > reached {
                fields.push((format!("{PADDING_FIELD} {reached}"), T::bytes(E::lit((width - reached) as i128))));
            }
            let named: Vec<(&str, T)> = fields.iter().map(|(n, t)| (n.as_str(), t.clone())).collect();
            Some(T::array(T::structure(RECORD_NAME, named), E::lit(count as i128)))
        }
        // An array of objects has no run of bytes to read: its values are
        // nodes of their own, wherever the file pickled them.
        Dtype::Objects => None,
    }
}

/// How many values a shape holds. Zero when any dimension is, which is what
/// the recogniser already checked the payload against.
fn count_of(dimensions: &[u64]) -> u64 {
    match dimensions.contains(&0) {
        true => 0,
        false => dimensions.iter().product(),
    }
}

impl Evaluator {
    /// The table the numbers of a matched array are, or nothing for any other
    /// node. A pickled array is the same rows and columns a `.npy` file holds,
    /// and the shape that says so is in the match rather than in a field of a
    /// structure, which is where [`Evaluator::table_shape`] looks otherwise.
    ///
    /// Read from what the match left behind and never from the file: this is
    /// asked of every node on its way to the screen, and a node under a
    /// matched pickle was placed by that match, so it is already there.
    ///
    /// The run is written along the last dimension in C order and along the
    /// first in Fortran order, which is what a row of it is either way. One
    /// dimension is one value a row. No dimensions is one value and no table.
    pub(super) fn pickle_table(&self, path: &[usize]) -> Option<crate::template::TableShape> {
        // An array whose numbers were spelled rather than written opens them
        // as a space, so the run of numbers is one step below the node the
        // match placed. The shape belongs on the numbers and not on the run:
        // the run holds one thing, and a table over it would be one row.
        let (path, inside) = match path.split_last() {
            Some((0, above)) if matches!(self.memo.get(above).map(|r| &r.ty), Some(Ty::Decoded { .. })) => (above, true),
            _ => (path, false),
        };
        let k = (0..=path.len()).rev().find(|k| matches!(self.memo.get(&path[..*k]).map(|r| &r.ty), Some(Ty::Pickle(Shape::Doc))))?;
        let found = self.memo.pickle(&path[..k])?;
        let here = spot(found, &path[k..])?;
        if inside && !matches!(here, (_, Part::Data(_))) {
            return None;
        }
        // A pandas frame or series, whose cells are spread across blocks and
        // worked out rather than read. Recognised without touching the file,
        // because this is asked of every node on its way to the screen; the
        // columns and the row count are read in
        // [`Evaluator::pickle_columns`].
        if let (_, Part::Value(v)) = &here {
            if super::pickleframe::frame_of(found, v).is_some() {
                return Some(crate::template::TableShape {
                    row_word: Some(ROW_WORD.into()),
                    cells: Some(Cells::Computed { rows: 0 }),
                    ..Default::default()
                });
            }
        }
        // A tensor's numbers, which are a run of their own wherever the entry
        // holding them is. The same table an array's numbers are: rows along
        // the first axis of the shape and columns along the last, or the other
        // way round for a tensor laid out in Fortran order.
        if let (_, Part::Numbers(tensor)) = &here {
            let inner = match (tensor.size.len(), tensor.contiguous()) {
                (0, _) => return None,
                (1, _) => None,
                (_, Some(false)) => tensor.size.first().copied(),
                _ => tensor.size.last().copied(),
            };
            return Some(crate::template::TableShape {
                columns: inner.filter(|n| *n > 0).map(|n| E::lit(n as i128)),
                row_word: inner.map(|_| "row".into()),
                ..Default::default()
            });
        }
        // A tensor, whose numbers are in another entry of the archive. How
        // many rows and columns it has is in the match, so the shape is
        // settled here and the cells are read in
        // [`Evaluator::tensor_cells`]. Only a view that steps through its
        // storage: a tensor whose elements are one run has them as a field of
        // its own, and the table is over that.
        if let (_, Part::Value(v)) = &here {
            if let Some(tensor) = super::pickletorch::tensor_of(v) {
                if tensor.contiguous().is_some() {
                    return None;
                }
                if tensor.columns() > 0 {
                    return Some(crate::template::TableShape {
                        row_word: Some(ROW_WORD.into()),
                        cells: Some(Cells::Computed { rows: tensor.rows() }),
                        ..Default::default()
                    });
                }
                // A tensor of no dimensions is one value, and one value is
                // not a table.
                return None;
            }
        }
        // A state dict, which is a mapping of nothing but tensors. What a
        // reader opens a checkpoint for is which weights are in it and how
        // big each one is, and that is a table before it is a tree.
        if let (_, Part::Value(v)) = &here {
            if let Some(held) = super::pickletorch::tensor_table(v) {
                return Some(crate::template::TableShape {
                    names: super::pickletorch::TENSOR_COLUMNS.iter().map(|n| (*n).into()).collect(),
                    row_word: Some(super::pickletorch::TENSOR_ROW.into()),
                    cells: Some(Cells::Computed { rows: held.len() as u64 }),
                    ..Default::default()
                });
            }
        }
        // A list of dictionaries, which is how rows are pickled when nobody
        // reached for pandas. Its columns are the keys, which are written in
        // the file beside the values, so the shape says where a cell is rather
        // than how many values make a row. The names are read in
        // [`Evaluator::pickle_columns`], which can reach the bytes.
        if let (_, Part::Value(v)) = &here {
            // A row is a dictionary, or one of the standard library's own
            // mappings, which holds its entries exactly as a dictionary does
            // and is named for the class that made it.
            if let Kind::List(items) = &v.kind {
                let row = items.first().and_then(|x| shape_of(&Part::Value(x)));
                let same = |x: &Value| shape_of(&Part::Value(x)) == row && entries_of(x).is_some();
                if items.len() >= FEWEST_ROWS && items.iter().all(same) {
                    return Some(crate::template::TableShape {
                        row_word: Some(ROW_WORD.into()),
                        cells: Some(Cells::Named {
                            row: row.unwrap_or(Shape::Dict).name().into(),
                            cell: Shape::Entry.name().into(),
                            value: Some(VALUE_FIELD.into()),
                        }),
                        ..Default::default()
                    });
                }
            }
        }
        let (_, Part::Data(v)) = here else { return None };
        let Kind::Array { dimensions, fortran_order, storage, .. } = &v.kind else { return None };
        // A run that was spelled rather than written says this one step down,
        // and a run the file holds as numbers says it here. Asking the other
        // way round would put a table over a node whose children are not the
        // numbers.
        if inside != (*storage != Storage::Raw) {
            return None;
        }
        let inner = match (dimensions.len(), fortran_order) {
            (0, _) => return None,
            (1, _) => None,
            (_, true) => dimensions.first().copied(),
            (_, false) => dimensions.last().copied(),
        };
        Some(crate::template::TableShape {
            columns: inner.filter(|n| *n > 0).map(|n| E::lit(n as i128)),
            // Several numbers to a row is a row. One to a row is a value,
            // which is what the table calls it when it is told nothing.
            row_word: inner.map(|_| "row".into()),
            ..Default::default()
        })
    }

    /// The columns of a pickled table, read where the file wrote them.
    ///
    /// Only for a shape that says its cells are named: a run of numbers has no
    /// names to read. The columns are every key any row has, in the order they
    /// are first met, so a row missing one has an empty cell under it rather
    /// than the next key's value.
    pub(super) fn pickle_columns<S: Source>(
        &mut self,
        doc: &Document<S>,
        path: &[usize],
        shape: crate::template::TableShape,
    ) -> R<Option<crate::template::TableShape>> {
        // A frame's columns, its row count and what one value of each column
        // is are all read at once, because none of them is in the node.
        if matches!(shape.cells, Some(Cells::Computed { .. })) {
            let (root, found) = self.pickle_doc(doc, path)?;
            // A tensor has said all of it already: its columns are places
            // along an axis rather than names written in the file.
            match spot(&found, &path[root.len()..]) {
                Some((_, Part::Value(v))) if super::pickletorch::tensor_of(v).is_some() => return Ok(Some(shape)),
                // A state dict's summary names its own columns, so there is
                // nothing under it to read them from.
                Some((_, Part::Value(v))) if super::pickletorch::tensor_table(v).is_some() => return Ok(Some(shape)),
                _ => {}
            }
            return self.frame_shape(doc, path);
        }
        if !matches!(shape.cells, Some(Cells::Named { .. })) {
            return Ok(Some(shape));
        }
        let (root, found) = self.pickle_doc(doc, path)?;
        let Some((_, Part::Value(v))) = spot(&found, &path[root.len()..]) else { return Ok(Some(shape)) };
        let Kind::List(items) = &v.kind else { return Ok(Some(shape)) };
        let r = self.memo[&root].clone();
        let base = r.offset;
        let mut names: Vec<Arc<str>> = Vec::new();
        for row in items {
            let Some(entries) = entries_of(row) else { continue };
            for (key, _) in entries {
                if let Some(name) = self.pickle_text(doc, &r, base, key)? {
                    if !names.iter().any(|seen| **seen == *name) {
                        names.push(name.into());
                    }
                }
            }
        }
        Ok(Some(crate::template::TableShape { names, ..shape }))
    }

    /// The path of the pickle field `path` sits in, and what the recogniser
    /// made of it. `path` may be the field itself or any value inside it.
    pub(super) fn pickle_doc<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<(Vec<usize>, Arc<Match>)> {
        let root = (0..=path.len())
            .rev()
            .find(|k| matches!(self.memo.get(&path[..*k]).map(|r| &r.ty), Some(Ty::Pickle(Shape::Doc))));
        let Some(k) = root else { return fail("not inside a pickle field") };
        let root = path[..k].to_vec();
        if let Some(found) = self.memo.pickle(&root) {
            return Ok((root, found.clone()));
        }
        let r = self.memo[&root].clone();
        let size = r.declared_size.unwrap_or(r.limit - r.offset);
        if size / 8 > MOST_BYTES {
            return fail("this file is too large to match against a Familiar Pickle Form");
        }
        let bytes = self.read(doc, &r, r.offset, size)?;
        let Some(found) = familiar::recognise(&bytes) else {
            return fail("this isn't a Familiar Pickle Form: open it with the Python pickle template to read the program");
        };
        let found = Arc::new(found);
        self.memo.remember_pickle(root.clone(), found.clone());
        Ok((root, found))
    }

    /// How many nodes are inside the one at `path`.
    pub(super) fn pickle_child_count<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<u64> {
        let (root, found) = self.pickle_doc(doc, path)?;
        let Some((_, here)) = spot(&found, &path[root.len()..]) else { return fail("no such value") };
        Ok(parts(&found, &here).len() as u64)
    }

    /// Which child of the node at `path` is called `name`.
    pub(super) fn pickle_index<S: Source>(&mut self, doc: &Document<S>, path: &[usize], name: &str) -> R<Option<usize>> {
        let (root, found) = self.pickle_doc(doc, path)?;
        let Some((_, here)) = spot(&found, &path[root.len()..]) else { return fail("no such value") };
        // A key may be a reference to a string written anywhere else in the
        // file, so a key is read through the pickle field rather than through
        // the node it is a key of.
        let r = self.memo[&root].clone();
        let base = r.offset;
        for (i, (label, _)) in parts(&found, &here).into_iter().enumerate() {
            let said = match label {
                Label::Field(f) => f == name,
                Label::Index(n) => name.parse::<usize>().ok() == Some(n),
                Label::Key(n) => match keyed(&here).and_then(|entries| entries.get(n)) {
                    Some((key, _)) => self.pickle_text(doc, &r, base, key)?.as_deref() == Some(name),
                    None => false,
                },
            };
            if said {
                return Ok(Some(i));
            }
        }
        Ok(None)
    }

    /// What a dictionary entry is called, which is its key when the key is
    /// something a name can be.
    ///
    /// A string spelled out, a string the file named instead of spelling
    /// again, and a whole number, which is the key a table of records keyed
    /// by row number has. Anything else leaves the entry numbered by its
    /// place, because a float, a byte string or a tuple written out as a name
    /// would read as something the file says and is not.
    ///
    /// `r` is the pickle field itself: a named string sits wherever the file
    /// first wrote it, which is outside the entry that names it.
    pub(super) fn pickle_text<S: Source>(&self, doc: &Document<S>, r: &Resolved, base: u64, key: &Value) -> R<Option<String>> {
        let (at, len) = match key.kind {
            Kind::Text { at, len } | Kind::Ref(Names::Text { at, len }) => (at, len),
            Kind::Int { value, .. } => return Ok(Some(value.to_string())),
            Kind::Wide { ref digits, .. } => return Ok(Some(digits.clone())),
            _ => return Ok(None),
        };
        let bytes = self.read(doc, r, base + at as u64 * 8, len as u64 * 8)?;
        Ok(String::from_utf8(bytes).ok())
    }

    /// Place child `idx` of the pickle node at `path`'s parent.
    ///
    /// A node that holds others is settled here, the way a value inside JSON
    /// is: the form already said where it is, so there is nothing to read.
    /// A leaf is handed back as a place, so that the type its bytes are is
    /// read, displayed and written back by the ordinary machinery.
    pub(super) fn place_pickle_child<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Option<Place>> {
        let (parent, idx) = (&path[..path.len() - 1], path[path.len() - 1]);
        let (root, found) = self.pickle_doc(doc, parent)?;
        let Some((_, above)) = spot(&found, &parent[root.len()..]) else { return fail("no such value") };
        let Some((label, part)) = parts(&found, &above).into_iter().nth(idx) else {
            return fail("no such value");
        };
        let pr = self.memo[parent].clone();
        // The whole pickle field, which is what a reference is read through:
        // what it names sits wherever the file first wrote it.
        let whole = self.memo[&root].clone();
        let base = whole.offset;
        let name = match label {
            Label::Field(f) => Name::Field(f.into()),
            Label::Index(n) => Name::Index(n),
            Label::Key(n) => match keyed(&above).and_then(|entries| entries.get(n)) {
                Some((key, _)) => match self.pickle_text(doc, &whole, base, key)? {
                    Some(text) => Name::Field(text.into()),
                    None => Name::Index(n),
                },
                None => Name::Index(n),
            },
        };
        let (at, end) = span(&found, &part);
        // A node that holds others is placed here and read no further.
        if let Some(shape) = shape_of(&part) {
            let said = match &part {
                Part::Value(v) => match &v.kind {
                    Kind::Class { path, .. } => Some(path.clone()),
                    // A number too wide to be read as one: the digits are what
                    // it is, and the run beneath it is how it was written.
                    Kind::Wide { digits, .. } => Some(digits.clone()),
                    // One of the standard library's own values, which is a run
                    // of packed bytes or a run of digits handed to a class and
                    // reads as the text Python writes the value in.
                    Kind::Made { what, .. } if super::picklestd::is_stdlib(*what) => {
                        self.pickle_stdlib(doc, &found, &whole, base, v)?
                    }
                    Kind::Instance { .. } if super::picklestd::is_uuid(v) => self.pickle_stdlib(doc, &found, &whole, base, v)?,
                    // A protocol 0 line that spells its value rather than
                    // being it, which is worked out when the form matches, and
                    // a number written as a line of digits.
                    Kind::Spelled { .. } | Kind::Int { spelled: true, .. } | Kind::Float { spelled: true, .. } => self.pickle_said(doc, &found, &whole, base, v)?,
                    _ => None,
                },
                // An entry reads as what it holds. A fitted model is thirty
                // attributes, and a row each saying `2 fields` makes a reader
                // open all thirty to find the one that is `True`.
                Part::Entry(e) => self.pickle_said(doc, &found, &whole, base, &e.1)?,
                _ => None,
            };
            self.pickle_node(path, &pr, name, shape, base, at, end - at, said);
            return Ok(None);
        }
        match part {
            Part::Note(text) => self.pickle_note(path, &pr, name, text),
            // The instructions the form fixed. Named for what they are, and
            // marked as the structure's own machinery: a reader following the
            // data wants them folded, and a reader following the program
            // wants them named.
            Part::Op { .. } => Ok(Some(self.pickle_place(&pr, name, T::bytes(E::lit((end - at) as i128)), base, at, end - at, true))),
            // What a reference names, read where the file wrote it. The row
            // is worked out rather than read in place: its bytes are not
            // inside the reference, which is only the BINGET.
            Part::Refers(v) => {
                let Kind::Ref(names) = v.kind else { return fail("no such value") };
                let said = match names {
                    // A container is not copied into the row. It is named for
                    // what it is and for where the file wrote it, so that the
                    // reader is sent to those bytes: a copy here would be a
                    // value the file says twice and holds once, and a
                    // container a reference sits inside is not finished yet.
                    Names::Made { what, at, .. } => format!("{} at {:#04x}", what.name(), base as usize / 8 + at),
                    Names::Text { at, len } | Names::Bytes { at, len } => {
                        let text = matches!(names, Names::Text { .. });
                        let read = len.min(if text { MOST_SHOWN_TEXT } else { MOST_SHOWN_BYTES });
                        let bytes = self.read(doc, &whole, base + at as u64 * 8, read as u64 * 8)?;
                        shown(&bytes, text, len)
                    }
                };
                self.pickle_note(path, &pr, name, said)
            }
            // A tensor's numbers, which are in another entry of the archive or
            // further down the file. Placed where they are, so they are read,
            // drawn and edited as the ordinary run of numbers they are; a row
            // saying where to look, for a pickle whose file does not hold
            // them.
            Part::Numbers(tensor) => match self.tensor_numbers(doc, &whole, base, tensor)? {
                Some((ty, at, len)) => {
                    let offset = at * 8;
                    Ok(Some(Place { name, ty, offset, limit: offset + len * 8, space: pr.space, machinery: false, elsewhere: true }))
                }
                None => {
                    let key = self.run_text(doc, &whole, base, tensor.key)?;
                    self.pickle_note(path, &pr, name, super::pickletorch::numbers_elsewhere(tensor, &key))
                }
            },
            // What a reader came for, before the structure that holds it.
            Part::Summary { of, says } => {
                let said = self.pickle_summary(doc, &root, path, &found, of, says)?;
                self.pickle_note(path, &pr, name, said)
            }
            Part::Text(said) => {
                let ty = T::text(StrLen::Fixed(E::lit(said.len as i128)), Encoding::Utf8);
                Ok(Some(self.pickle_place(&pr, name, ty, base, at, end - at, false)))
            }
            // The run a line spells its value in, which is text of an encoding
            // nobody declared: it is what `repr` or `raw-unicode-escape` made
            // of the value, and the row above holds the value itself.
            Part::Line(_) => {
                let ty = T::text(StrLen::Fixed(E::lit((end - at) as i128)), Encoding::Unknown);
                Ok(Some(self.pickle_place(&pr, name, ty, base, at, end - at, false)))
            }
            // The run a number too wide for any integer type was written in.
            // The two's-complement bytes read as bytes, since no type here is
            // that wide; the digits a protocol 0 or 1 line spells read as the
            // text they are, and the number itself is the row above.
            Part::Wide(v) => {
                let spelled = matches!(v.kind, Kind::Wide { spelled: true, .. } | Kind::Int { spelled: true, .. } | Kind::Float { spelled: true, .. });
                let len = E::lit((end - at) as i128);
                let ty = match spelled {
                    true => T::text(StrLen::Fixed(len), Encoding::Ascii),
                    false => T::bytes(len),
                };
                Ok(Some(self.pickle_place(&pr, name, ty, base, at, end - at, false)))
            }
            // The one byte of the envelope with something in it. PROTO is the
            // instruction in front of it, and the frame's length is the
            // listing's business rather than the object's.
            Part::Protocol(_) => Ok(Some(self.pickle_place(&pr, name, T::u8(), base, at, end - at, false))),
            Part::Data(v) => {
                let Kind::Array { dtype, dimensions, storage, .. } = &v.kind else { return fail("no such value") };
                let Some(numbers) = numbers_ty(dtype, count_of(dimensions)) else { return fail("this dtype has no type") };
                // Below protocol 3 the run is the latin-1 spelling of the
                // numbers rather than the numbers, so the numbers are nowhere
                // in the file. The run opens as a space of its own and they
                // are ordinary fields of it, at ordinary offsets, which is
                // what every other stream in a file gets. The run itself stays
                // exactly where it is and exactly as long as it is.
                let ty = match storage {
                    Storage::Latin1 => T::decoded(E::lit((end - at) as i128), Codec::Latin1Text, numbers),
                    // A protocol 0 line is the same text with the escaping
                    // that protocol writes over the top of it, so the space
                    // opens through both layers at once.
                    Storage::Escaped => T::decoded(E::lit((end - at) as i128), Codec::EscapedLatin1Text, numbers),
                    Storage::Raw => numbers,
                };
                Ok(Some(self.pickle_place(&pr, name, ty, base, at, end - at, false)))
            }
            Part::Value(v) => match leaf(v) {
                Some((ty, at, len)) => Ok(Some(self.pickle_place(&pr, name, ty, base, at, len, false))),
                None => fail("this value has no type"),
            },
            _ => fail("no such value"),
        }
    }


    /// A row that says something about the file rather than reading it: no
    /// bytes of its own, and its text worked out when the form matched.
    fn pickle_note(&mut self, path: &[usize], pr: &Resolved, name: Name, text: String) -> R<Option<Place>> {
        let r = Resolved {
            name,
            // The expression is never asked: the text is on the node, which is
            // where a computed field's value is kept once it is worked out.
            ty: Ty::ComputedText(E::lit(0)),
            offset: pr.offset,
            cursor: pr.offset,
            limit: pr.limit,
            declared_size: None,
            sized_how: None,
            origin: false,
            size: Some(0),
            payload: None,
            computed: Some(Computed::Text(text.as_str().into())),
            space: pr.space,
            machinery: false,
            elsewhere: false,
        };
        self.remember(path, r);
        Ok(None)
    }

    /// A node that holds others, covering the bytes its production consumed.
    fn pickle_node(&mut self, path: &[usize], pr: &Resolved, name: Name, shape: Shape, base: u64, at: usize, len: usize, said: Option<String>) {
        let offset = base + at as u64 * 8;
        let size = len as u64 * 8;
        let r = Resolved {
            name,
            ty: Ty::Pickle(shape),
            offset,
            cursor: offset,
            limit: offset + size,
            declared_size: None,
            sized_how: None,
            origin: false,
            size: Some(size),
            payload: None,
            computed: said.map(|said| Computed::Text(said.as_str().into())),
            space: pr.space,
            machinery: false,
            elsewhere: false,
        };
        self.remember(path, r);
    }

    /// Where a leaf goes, for the ordinary machinery to read it there.
    fn pickle_place(&self, pr: &Resolved, name: Name, ty: T, base: u64, at: usize, len: usize, machinery: bool) -> Place {
        let offset = base + at as u64 * 8;
        Place { name, ty, offset, limit: offset + len as u64 * 8, space: pr.space, machinery, elsewhere: false }
    }
}
