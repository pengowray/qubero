//! A torch tensor read as the numbers it stands for.
//!
//! The pickle says everything about a tensor but its values: which storage it
//! is a window onto, how far into that storage it starts, how far apart its
//! values are along each axis. The values are in another entry of the archive,
//! so this file is what finds that entry and reads it.
//!
//! Nothing here runs anything, and nothing here is a second reading of the
//! pickle: the persistent id named a storage and this walks the archive's own
//! local records for the entry of that name, the way
//! [`picklecells`](super::picklecells) walks a frame's blocks for a cell.
//!
//! A tensor whose elements run through its storage in order gets them as a
//! field: [`Evaluator::tensor_numbers`] works out where the run is and
//! `pickletree.rs` places a typed run there, so the numbers have byte
//! addresses, a listing row, the hex view and the ordinary array table. A view
//! that steps over its storage is no run, and its table is still worked out a
//! cell at a time in [`Evaluator::tensor_cells`].
//!
//! **Why the records are walked here rather than declared in the template.**
//! A tensor is a field of one ZIP entry placed by the contents of another, and
//! the IR cannot say that: `Ty::At` places a field at an offset, and nothing
//! works an offset out from an entry's *name*. `formats/torchzip.rs` closes
//! half of it by placing the pickle in the archive's own space, so the pickle
//! and the storages share one set of addresses; the other half, resolving
//! `data/<key>` to a run, is done here. So the field is placed at an address
//! this file worked out rather than at one the template can show a reader. See
//! `docs/DESIGN-pickle-containers.md` for what closing it properly would need.

use super::pickleparts::{call_of, extent, said_flag, Label, Part, Says, C_ORDER, DTYPE_FIELD, FORTRAN_ORDER, IS_FIELD, NUMBERS_FIELD, ORDER_FIELD, REQUIRES_GRAD_FIELD, SCALE_FIELD, SHAPE_FIELD, STORAGE_OFFSET_FIELD, STRIDE_FIELD, ZERO_POINT_FIELD};
use crate::formats::pickle::familiar::Match;
use std::sync::Arc;

use super::*;
use crate::formats::pickle::familiar::{Kind, Tensor, TensorType, Value as Captured};
use crate::formats::torchzip::{DATA_FOLDER, PICKLE_ENTRY};
use crate::template::{Endian, Expr as E, Ty as T};

/// What the `stored at` row says when nothing in this file holds the numbers,
/// which is what a `data.pkl` opened on its own is.
const NOT_HERE: &str = "not in this file";

/// What the `order` row says for a view whose elements are neither one run
/// forwards nor one run down the columns. An array says `C` or `Fortran`
/// there and a tensor says the same two words; this is the third answer, in
/// the word torch and NumPy both use for it, and the tensor carrying it has
/// no run of numbers under it.
const NOT_CONTIGUOUS: &str = "not contiguous";

/// What the `numbers` row says when the tensor's elements are a run and this
/// file does not hold the entry they are in, which is a `data.pkl` opened on
/// its own.
#[inline(never)]
pub(super) fn numbers_elsewhere(tensor: &Tensor, key: &str) -> String {
    format!("{} in {DATA_FOLDER}/{key}, {NOT_HERE}", counted(tensor.values()))
}

/// Which way a tensor's elements run through its storage, in the words the
/// `order` row of an array uses.
fn order_said(tensor: &Tensor) -> &'static str {
    match tensor.contiguous() {
        Some(true) => C_ORDER,
        Some(false) => FORTRAN_ORDER,
        None => NOT_CONTIGUOUS,
    }
}

/// How many values there are, said the way English says it: a tensor with no
/// dimensions holds one value, not one values.
fn counted(n: u64) -> String {
    match n {
        1 => "1 value".to_string(),
        n => format!("{n} values"),
    }
}

/// One entry of the archive: what it is called, and the run its data is.
#[derive(Debug)]
pub(super) struct Held {
    pub(super) name: String,
    pub(super) at: u64,
    pub(super) len: u64,
    /// How the data was packed. Only a stored entry can be read where it lies;
    /// torch has never written any other kind.
    pub(super) method: u16,
    /// True when this came from the layout of a legacy checkpoint rather than
    /// from an archive's directory, which says the storages of this space are
    /// fields of the template already. See
    /// [`Evaluator::storages_are_fields`].
    pub(super) legacy: bool,
}

/// What the summary over a state dict says about each tensor, in the order a
/// reader asks it: which weight this is, what one value of it is, how many
/// there are and how they are arranged.
pub(super) const TENSOR_COLUMNS: &[&str] = &["name", "dtype", "shape", "values"];
/// What one row of that table is.
pub(super) const TENSOR_ROW: &str = "tensor";
/// The fewest tensors that make a summary. One tensor is a tensor, and its own
/// rows already say all of this.
const FEWEST_TENSORS: usize = 2;

/// The tensors a mapping holds, keyed the way the file keyed them, or nothing
/// for a mapping holding anything else.
///
/// What a state dict is: `{"layer.weight": tensor, "layer.bias": tensor}`,
/// written as a dictionary or as the `OrderedDict` `nn.Module.state_dict()
/// returns. A checkpoint's top level holds an epoch and a loss beside it and
/// is not one; the state dict inside it is.
pub(super) fn tensor_table(value: &Captured) -> Option<Vec<(&Captured, &Tensor)>> {
    let entries = super::pickleparts::entries_of(value)?;
    if entries.len() < FEWEST_TENSORS {
        return None;
    }
    entries.iter().map(|(key, held)| Some((key, tensor_of(held)?))).collect()
}

/// The tensor a value is, for the rows and the table that read one.
pub(super) fn tensor_of(value: &Captured) -> Option<&Tensor> {
    match &value.kind {
        Kind::Tensor(t) => Some(t),
        _ => None,
    }
}

/// How one element of a tensor reads, as the type its storage class says.
///
/// Byte order is the machine that saved the file, which the archive's
/// `byteorder` record names. Every sample says `little`; a big-endian save is
/// read the same way and is untested.
pub(crate) fn element_ty(dtype: TensorType, endian: Endian) -> Ty {
    match dtype {
        TensorType::Float16 => Ty::F16(endian),
        TensorType::BFloat16 => Ty::BF16(endian),
        TensorType::Float32 => Ty::F32(endian),
        TensorType::Float64 => Ty::F64(endian),
        TensorType::Int8 => Ty::Int { bits: 8, endian },
        TensorType::Int16 => Ty::Int { bits: 16, endian },
        TensorType::Int32 => Ty::Int { bits: 32, endian },
        TensorType::Int64 => Ty::Int { bits: 64, endian },
        // A complex number is a pair of floats, and the table reads both: the
        // type here is one of the two, and the cell puts them together.
        TensorType::Complex64 => Ty::F32(endian),
        TensorType::Complex128 => Ty::F64(endian),
        TensorType::Float8E4M3FN => Ty::F8 { e4m3: true },
        TensorType::Float8E5M2 => Ty::F8 { e4m3: false },
        TensorType::UInt16 => Ty::UInt { bits: 16, endian },
        TensorType::UInt32 => Ty::UInt { bits: 32, endian },
        TensorType::UInt64 => Ty::UInt { bits: 64, endian },
        // A quantised element is the whole number the file holds. What the
        // scale and the zero point say it stands for is a row beside it, not
        // a number written in its place.
        TensorType::QInt8 => Ty::Int { bits: 8, endian },
        TensorType::QInt32 => Ty::Int { bits: 32, endian },
        TensorType::QUInt8 => Ty::UInt { bits: 8, endian },
        // A boolean is a byte holding 0 or 1, which is how NumPy's `|b1`
        // reads too.
        TensorType::UInt8 | TensorType::Bool => Ty::UInt { bits: 8, endian },
    }
}

/// Where a tensor's numbers go and how they read: the run's type, where it
/// starts in the space the pickle is in, how long it is, and whether another
/// field of this file already describes those bytes.
pub(super) struct Numbers {
    pub(super) ty: Ty,
    pub(super) at: u64,
    pub(super) len: u64,
    pub(super) aside: bool,
}

/// How one element of a tensor reads when it is a field rather than a cell of
/// a computed table: the type [`element_ty`] gives it, and for a complex
/// number the pair of halves Python writes it as.
fn element_run(dtype: TensorType, endian: Endian) -> Ty {
    let held = element_ty(dtype, endian);
    match dtype.complex() {
        true => T::structure(COMPLEX_NAME, vec![(REAL_FIELD, held.clone()), (IMAGINARY_FIELD, held)]),
        false => held,
    }
}

/// What one complex element is called, and its two halves, which are the
/// words Python's own `complex` uses for them.
const COMPLEX_NAME: &str = "Complex";
const REAL_FIELD: &str = "real";
const IMAGINARY_FIELD: &str = "imaginary";

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
            Says::Numbers => Ok(format!("{} in {DATA_FOLDER}/{key}", counted(tensor.values()))),
            // And where that entry is in this file, for a reader who wants to
            // go there. The tensor's own window inside it, since a view may
            // start past the front of its storage.
            Says::StoredAt => Ok(match self.tensor_run(doc, r, base, tensor)? {
                Some((at, len)) => format!("{at:#x}, {len} bytes"),
                None => NOT_HERE.to_string(),
            }),
            _ => Ok(String::new()),
        }
    }

    /// Where a tensor's numbers go, as the typed run they are.
    ///
    /// The one place in the reading where a field is placed outside the bytes
    /// of the node that named it: the pickle is the archive entry `data.pkl`
    /// and the numbers are in `data/<key>`, which is the same space and a long
    /// way off. The run is the tensor's own window rather than the whole
    /// storage, so two views onto one storage place two runs of their own and
    /// each says what its reader asked for. The entry's own reading of those
    /// bytes is the archive's, and `zip::records(true)` already says that one
    /// is the second reading, so nothing is counted twice.
    ///
    /// Nothing when this file does not hold the entry, which is a `data.pkl`
    /// opened on its own: the caller writes the row that says so.
    #[inline(never)]
    pub(super) fn tensor_numbers<S: Source>(
        &mut self,
        doc: &Document<S>,
        r: &Resolved,
        base: u64,
        tensor: &Tensor,
    ) -> R<Option<Numbers>> {
        let Some((at, len)) = self.tensor_run(doc, r, base, tensor)? else { return Ok(None) };
        let endian = self.byte_order(doc, r, base)?;
        let ty = T::array(element_run(tensor.dtype, endian), E::lit(tensor.values() as i128));
        let aside = self.storages_are_fields(doc, r.space)?;
        Ok(Some(Numbers { ty, at, len, aside }))
    }

    /// Whether the storages this tensor reads from are fields of the template
    /// already.
    ///
    /// A legacy checkpoint's are: `formats/torchlegacy.rs` places each one as
    /// an element count and a typed run of numbers, because everything is in
    /// one space there and the layout says where each of them is. So a
    /// tensor's own run over the same bytes is a second reading of what that
    /// field describes, and the field is where they are counted. In an
    /// archive nothing else reads them as numbers: the entry's own reading is
    /// the one put aside, which is what `zip::records(true)` says.
    ///
    /// Both halves are asked. The template has to be the one that places
    /// them, and the runs in hand have to be the ones its layout walk found:
    /// a legacy file read as the contents of something else is the same
    /// bytes with nothing placing them, and marking its tensors a second
    /// reading would leave the numbers counted nowhere.
    fn storages_are_fields<S: Source>(&mut self, doc: &Document<S>, space: u32) -> R<bool> {
        if self.template.name != crate::formats::torchlegacy::TEMPLATE {
            return Ok(false);
        }
        Ok(self.archive(doc, space)?.iter().any(|held| held.legacy))
    }

    /// The run this tensor's own values sit in: where its first element is and
    /// how far its last one reaches.
    ///
    /// Not the whole storage. Two tensors may be two windows onto one storage
    /// and a transposed one is not laid out in order, so what a reader wants
    /// pointed at is the bytes this tensor reads.
    pub(super) fn tensor_run<S: Source>(
        &mut self,
        doc: &Document<S>,
        r: &Resolved,
        base: u64,
        tensor: &Tensor,
    ) -> R<Option<(u64, u64)>> {
        let Some(storage) = self.storage_run(doc, r, base, tensor)? else { return Ok(None) };
        let width = tensor.dtype.width();
        let Some(reach) = tensor.reach() else { return Ok(None) };
        let (from, to) = (tensor.offset.saturating_mul(width), reach.saturating_mul(width));
        if to > storage.1 {
            return Ok(None);
        }
        Ok(Some((storage.0 + from, to - from)))
    }

    /// The whole storage this tensor names: the run of the archive entry
    /// `data/<key>`, in the space the pickle itself sits in.
    fn storage_run<S: Source>(&mut self, doc: &Document<S>, r: &Resolved, base: u64, tensor: &Tensor) -> R<Option<(u64, u64)>> {
        let key = self.run_text(doc, r, base, tensor.key)?;
        let entries = self.archive(doc, r.space)?;
        // Which entry the pickle is, which is what says the folder every other
        // entry of this checkpoint is under. Found by where it sits rather
        // than by its name, so a second checkpoint zipped beside this one
        // cannot answer for it.
        let here = base / 8;
        let Some(pickle) = entries.iter().find(|e| e.at == here) else { return Ok(None) };
        let Some(folder) = pickle.name.strip_suffix(PICKLE_ENTRY) else { return Ok(None) };
        let want = format!("{folder}{DATA_FOLDER}/{key}");
        Ok(entries.iter().find(|e| e.name == want && e.method == 0).map(|e| (e.at, e.len)))
    }

    /// Every entry of the archive in `space`, read out of the directory at
    /// the end of it and kept.
    ///
    /// The directory rather than a walk of the local records, because torch
    /// writes every entry as a stream: the local header says nought for both
    /// sizes and puts the real ones in a descriptor after the data, so a walk
    /// from the front has no way on to the next record. The directory has
    /// them, and the local header of each entry has the name and extra
    /// lengths that say where its data begins.
    fn archive<S: Source>(&mut self, doc: &Document<S>, space: u32) -> R<Arc<Vec<Held>>> {
        if let Some(held) = self.memo.archive(space) {
            return Ok(held.clone());
        }
        let end = match space {
            0 => doc.len_bytes(),
            _ => self.spaces.buf(space).map_or(0, |b| b.len() as u64),
        };
        let mut found = self.directory(doc, space, end)?;
        // A legacy checkpoint is no archive at all, and the numbers are still
        // somewhere else in the same space: five pickles along, after the one
        // that named them. The key that names a storage is the same key, so
        // the runs are handed back under the names the tensors look for and
        // nothing else here has to know which kind of file it is reading.
        if found.is_empty() {
            found = self.legacy_storages(doc, space, end)?;
        }
        let held = Arc::new(found);
        self.memo.remember_archive(space, held.clone());
        Ok(held)
    }

    /// The runs a legacy checkpoint's storages sit in, named the way an
    /// archive's entries are, or nothing at all for any other file.
    ///
    /// The first pickle is checked before anything else is read: it is the
    /// same run of bytes in every file torch has written this way, one
    /// spelling per protocol the caller may ask for, and every other file gets
    /// no further than that.
    fn legacy_storages<S: Source>(&mut self, doc: &Document<S>, space: u32, end: u64) -> R<Vec<Held>> {
        let want = crate::formats::torchlegacy::MAGIC_MOST as u64;
        if end < want {
            return Ok(Vec::new());
        }
        let opener = self.read_in(doc, space, 0, want * 8)?;
        if !crate::formats::torchlegacy::is_torch_legacy(&opener) {
            return Ok(Vec::new());
        }
        // Read through the evaluator, the way everything else here reads, so
        // that a chunk that has not arrived says `Pending` rather than
        // answering out of a run of noughts.
        let mut read = |at: u64, len: u64| self.read_in(doc, space, at * 8, len * 8);
        let Some(found) = crate::formats::torchlegacy::layout(&mut read, end)? else { return Ok(Vec::new()) };
        let (at, len) = found.data();
        let mut out = vec![Held { name: PICKLE_ENTRY.to_string(), at, len, method: 0, legacy: true }];
        out.extend(found.storages.iter().map(|s| Held { name: format!("{DATA_FOLDER}/{}", s.key), at: s.at, len: s.len, method: 0, legacy: true }));
        Ok(out)
    }

    /// The entries the central directory names, as the runs this file reads
    /// tensors out of. [`zipdirectory`](crate::formats::zipdirectory) is the
    /// walk; what belongs here is reading through the evaluator, so that a
    /// chunk that has not arrived says `Pending` rather than answering out of
    /// a run of noughts.
    fn directory<S: Source>(&mut self, doc: &Document<S>, space: u32, end: u64) -> R<Vec<Held>> {
        let mut read = |at: u64, len: u64| self.read_in(doc, space, at * 8, len * 8);
        let found = crate::formats::zipdirectory::entries(&mut read, end)?;
        Ok(found.into_iter().map(|e| Held { name: e.name, at: e.at, len: e.len, method: e.method, legacy: false }).collect())
    }

    /// The summary over a state dict: one row a tensor, saying what a reader
    /// opens a checkpoint to find out.
    ///
    /// Nothing is read out of the storages here. Every column is something the
    /// pickle already said, so the table costs one read a row, for the key.
    ///
    /// The rows are about tensors rather than about runs of bytes, and only
    /// two of the four columns are anywhere. The name is the run of
    /// instructions that spell it, which is where a reader looking for
    /// `layer.weight` in the hex view would find it; the count of values is
    /// the numbers it counts, which is the tensor's own window in the file and
    /// the one address a reader opens a checkpoint for. The dtype is
    /// where the pickle names what one element is, the storage class or the
    /// dtype an untyped storage's call names, and the shape is the size tuple.
    pub(super) fn tensor_rows<S: Source>(
        &mut self,
        doc: &Document<S>,
        r: &Resolved,
        base: u64,
        held: &[(&Captured, &Tensor)],
        from: u64,
        to: u64,
    ) -> R<Vec<Vec<FrameCell>>> {
        let mut out = Vec::new();
        for (key, tensor) in held.iter().skip(from as usize).take(to.saturating_sub(from) as usize) {
            let name = self.pickle_text(doc, r, base, key)?;
            let run = |(at, len): (usize, usize)| CellAt::Bytes { space: r.space, offset_bits: base + at as u64 * 8, size_bits: len as u64 * 8 };
            let spelled = run((key.at, key.len));
            let numbers = match self.tensor_run(doc, r, base, tensor)? {
                Some((at, len)) => CellAt::Bytes { space: r.space, offset_bits: at * 8, size_bits: len * 8 },
                None => CellAt::Nowhere,
            };
            out.push(vec![
                FrameCell { value: name.map(Value::Str), at: spelled },
                FrameCell { value: Some(Value::Str(tensor.dtype.word().to_string())), at: run(tensor.dtype_at) },
                FrameCell { value: Some(Value::Str(super::pickleparts::extent(&tensor.size))), at: run(tensor.size_at) },
                FrameCell { value: Some(Value::UInt(u128::from(tensor.values()))), at: numbers },
            ]);
        }
        Ok(out)
    }

    /// A run of the pickle read as the text it is, for the two the tensor
    /// names: a run written once and referred to again is read where it was
    /// written, which is why this goes through the pickle field rather than
    /// through the tensor's own bytes.
    pub(super) fn run_text<S: Source>(&self, doc: &Document<S>, r: &Resolved, base: u64, (at, len): (usize, usize)) -> R<String> {
        let bytes = self.read(doc, r, base + at as u64 * 8, len as u64 * 8)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// The rows of a tensor's table: rows along the first axis, columns along
    /// the last, every value read where the stride says it is.
    pub(super) fn tensor_cells<S: Source>(
        &mut self,
        doc: &Document<S>,
        r: &Resolved,
        base: u64,
        tensor: &Tensor,
        from: u64,
        to: u64,
    ) -> R<Vec<Vec<FrameCell>>> {
        let Some((start, len)) = self.storage_run(doc, r, base, tensor)? else {
            return fail("this file does not hold the entry the tensor names");
        };
        let width = tensor.dtype.width();
        let ty = element_ty(tensor.dtype, self.byte_order(doc, r, base)?);
        let (rows, columns) = (tensor.rows(), tensor.columns());
        let mut out = Vec::new();
        for row in from..to.min(rows) {
            let mut cells = Vec::new();
            for column in 0..columns {
                cells.push(self.one_element(doc, r, start, len, tensor, &ty, width, row, column)?);
            }
            out.push(cells);
        }
        Ok(out)
    }

    /// One value of a tensor, read where the stride puts it, and pointing at
    /// the bytes it was read from: the entry's data, the tensor's offset into
    /// its storage, and one step of the stride per axis.
    #[allow(clippy::too_many_arguments)]
    fn one_element<S: Source>(
        &mut self,
        doc: &Document<S>,
        r: &Resolved,
        start: u64,
        len: u64,
        tensor: &Tensor,
        ty: &Ty,
        width: u64,
        row: u64,
        column: u64,
    ) -> R<FrameCell> {
        let Some(elem) = tensor.element(row, column) else { return Ok(FrameCell::nowhere()) };
        let Some(at) = elem.checked_mul(width) else { return Ok(FrameCell::nowhere()) };
        if at.checked_add(width).is_none_or(|end| end > len) {
            return Ok(FrameCell::nowhere());
        }
        let offset = (start + at) * 8;
        // A complex number is a pair: the real part and then the imaginary
        // one, each the width the type in hand reads. One cell, because one
        // element of the tensor is one number in Python's own spelling.
        let size = match tensor.dtype.complex() {
            true => width * 4,
            false => width * 8,
        };
        // The cell covers the whole element, which for a complex number is
        // both halves of the pair.
        let cell = CellAt::Bytes { space: r.space, offset_bits: offset, size_bits: width * 8 };
        let one = Resolved { offset, cursor: offset, limit: offset + size, size: Some(size), ..r.clone() };
        let real = self.primitive_value(doc, &[], &one, ty, size)?;
        if !tensor.dtype.complex() {
            return Ok(FrameCell { value: Some(real), at: cell });
        }
        let next = Resolved { offset: offset + size, cursor: offset + size, limit: offset + size * 2, size: Some(size), ..r.clone() };
        let imaginary = self.primitive_value(doc, &[], &next, ty, size)?;
        Ok(FrameCell { value: Some(Value::Str(complex_said(&real, &imaginary))), at: cell })
    }

    /// Which way round the numbers are, which the archive says in a record of
    /// its own beside the pickle.
    ///
    /// torch writes the byte order of the machine that saved the file and
    /// reverses the numbers on the way in when a reader's differs. Little
    /// where the archive says nothing, which is what every torch before 2.1
    /// wrote and what `torch.load` falls back to.
    fn byte_order<S: Source>(&mut self, doc: &Document<S>, r: &Resolved, base: u64) -> R<Endian> {
        let entries = self.archive(doc, r.space)?;
        let here = base / 8;
        let Some(pickle) = entries.iter().find(|e| e.at == here) else { return Ok(Endian::Little) };
        let Some(folder) = pickle.name.strip_suffix(PICKLE_ENTRY) else { return Ok(Endian::Little) };
        let want = format!("{folder}{BYTEORDER_ENTRY}");
        let Some(held) = entries.iter().find(|e| e.name == want && e.method == 0) else { return Ok(Endian::Little) };
        let said = self.read_in(doc, r.space, held.at * 8, held.len.min(MOST_BYTEORDER) * 8)?;
        Ok(match said.as_slice() {
            b"big" => Endian::Big,
            _ => Endian::Little,
        })
    }
}

/// One complex number, in the spelling Python writes a complex literal in:
/// the real part, the sign of the imaginary part, and `j`.
fn complex_said(real: &Value, imaginary: &Value) -> String {
    let said = |v: &Value| match v {
        Value::Float(f) => f.to_string(),
        other => format!("{other:?}"),
    };
    let held = said(imaginary);
    match held.starts_with('-') {
        true => format!("{}{held}j", said(real)),
        false => format!("{}+{held}j", said(real)),
    }
}

/// The record naming the byte order, and the most of it worth reading: the
/// word is `little` or `big`.
const BYTEORDER_ENTRY: &str = "byteorder";
const MOST_BYTEORDER: u64 = 8;

/// The rows a tensor shows and the nodes under it.
///
/// A tensor says what it is and where its numbers are, and then the run of
/// instructions that rebuilt it. The numbers are in another entry of the
/// archive or further down the file, so there is nothing under it to read as
/// values: the rows say where to look and the table reads them from there.
pub(super) fn tensor_parts<'a>(
    found: &'a Match,
    v: &'a crate::formats::pickle::familiar::Value,
    t: &'a crate::formats::pickle::familiar::Tensor,
) -> (Vec<(Label, Part<'a>)>, Vec<(Label, Part<'a>)>) {

        let mut notes = vec![
            (Label::Field(DTYPE_FIELD), Part::Note(t.dtype.word().to_string())),
            (Label::Field(SHAPE_FIELD), Part::Note(extent(&t.size))),
            // Commas rather than the `x` a shape is written with: a
            // stride is a step per axis and not a shape, and `4 x 1`
            // beside `3 x 4` reads as a second shape.
            (Label::Field(STRIDE_FIELD), Part::Note(t.stride.iter().map(u64::to_string).collect::<Vec<_>>().join(", "))),
            // Which way the strides run, in the words an array's own `order`
            // row uses, and the third answer for a view that runs neither
            // way. What the row is worth is that the first two say the
            // elements are one run of the file and the third says they are
            // not.
            (Label::Field(ORDER_FIELD), Part::Note(order_said(t).to_string())),
            (Label::Field(STORAGE_OFFSET_FIELD), Part::Note(t.offset.to_string())),
        ];
        // Said only of a parameter: every tensor would carry the row
        // and a reader would learn to skip it.
        if t.parameter {
            notes.insert(0, (Label::Field(IS_FIELD), Part::Note(super::picklesaid::PARAMETER_WORD.to_string())));
        }
        notes.extend(
            [Says::Storage, Says::Location]
                .into_iter()
                .map(|says| (Label::Field(says.name()), Part::Summary { of: v, says })),
        );
        notes.push((Label::Field(REQUIRES_GRAD_FIELD), Part::Note(said_flag(t.requires_grad))));
        // What a quantised tensor's whole numbers stand for. The numbers
        // themselves stay what the file holds: a reader looking at a
        // quantised checkpoint is looking at the stored integers, and would
        // not be told that the file had been rewritten on the way out.
        if let Some(q) = t.quantizer {
            notes.push((Label::Field(SCALE_FIELD), Part::Note(q.scale.to_string())));
            notes.push((Label::Field(ZERO_POINT_FIELD), Part::Note(q.zero_point.to_string())));
        }
        // The numbers themselves, for a tensor whose elements run through its
        // storage one after another: a typed run placed where the entry it
        // named holds it, which is a field like any other. A view that steps
        // through its storage rather than reading it in order is no run, so it
        // keeps the two rows saying where its numbers are and the table reads
        // them one element at a time.
        match t.contiguous() {
            Some(_) => notes.push((Label::Field(NUMBERS_FIELD), Part::Numbers(t))),
            None => notes.extend(
                [Says::Numbers, Says::StoredAt]
                    .into_iter()
                    .map(|says| (Label::Field(says.name()), Part::Summary { of: v, says })),
            ),
        }
        let kids = match call_of(found, v) {
            Some(call) => vec![(Label::Field(call.name), Part::Call(call, v))],
            None => Vec::new(),
        };
        (notes, kids)
}
