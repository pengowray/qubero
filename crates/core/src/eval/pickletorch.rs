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
//! **Why the records are walked here rather than declared in the template.**
//! A tensor is a field of one ZIP entry placed by the contents of another, and
//! the IR cannot say that: `Ty::At` places a field at an offset, and nothing
//! works an offset out from an entry's *name*. `formats/torchzip.rs` closes
//! half of it by placing the pickle in the archive's own space, so the pickle
//! and the storages share one set of addresses; the other half, resolving
//! `data/<key>` to a run, is done here. See `docs/DESIGN-pickle-containers.md`
//! for what closing it properly would need.

use super::pickleparts::{call_of, extent, said_flag, Label, Part, Says, DTYPE_FIELD, IS_FIELD, REQUIRES_GRAD_FIELD, SCALE_FIELD, SHAPE_FIELD, STORAGE_OFFSET_FIELD, STRIDE_FIELD, ZERO_POINT_FIELD};
use crate::formats::pickle::familiar::Match;
use std::sync::Arc;

use super::*;
use crate::formats::pickle::familiar::{Kind, Tensor, TensorType, Value as Captured};
use crate::formats::torchzip::{DATA_FOLDER, PICKLE_ENTRY};
use crate::template::Endian;

/// What the `stored at` row says when nothing in this file holds the numbers,
/// which is what a `data.pkl` opened on its own is.
const NOT_HERE: &str = "not in this file";

/// How many values there are, said the way English says it: a tensor with no
/// dimensions holds one value, not one values.
fn counted(n: u64) -> String {
    match n {
        1 => "1 value".to_string(),
        n => format!("{n} values"),
    }
}

/// The signatures the directory at the end of an archive is found by, and how
/// long the fixed part of each record is.
const END: &[u8] = b"PK\x05\x06";
const END64: &[u8] = b"PK\x06\x06";
const LOCATOR: &[u8] = b"PK\x06\x07";
const CENTRAL: &[u8] = b"PK\x01\x02";
const LOCAL: &[u8] = b"PK\x03\x04";
const END_RECORD: u64 = 22;
const CENTRAL_RECORD: u64 = 46;
const LOCAL_HEADER: u64 = 30;
const LOCATOR_RECORD: u64 = 20;
/// How far back from the end the end record may be: its own bytes and the
/// longest comment a ZIP may carry.
const MOST_COMMENT: u64 = (1 << 16) + END_RECORD + LOCATOR_RECORD;
/// The widest a 32-bit field may be before it is standing in for a 64-bit one
/// kept in the entry's extra field.
const WIDE32: u64 = 0xffff_ffff;
const WIDE16: u64 = 0xffff;
/// The extra field that holds those 64-bit numbers.
const ZIP64_EXTRA: u64 = 1;
/// The most entries the walk will follow. Far past any checkpoint: a storage
/// is one entry and a model of a few hundred million parameters has a few
/// hundred of them.
const MOST_ENTRIES: usize = 1 << 20;

/// One entry of the archive: what it is called, and the run its data is.
#[derive(Debug)]
pub(super) struct Held {
    pub(super) name: String,
    pub(super) at: u64,
    pub(super) len: u64,
    /// How the data was packed. Only a stored entry can be read where it lies;
    /// torch has never written any other kind.
    pub(super) method: u16,
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
        let mut out = vec![Held { name: PICKLE_ENTRY.to_string(), at, len, method: 0 }];
        out.extend(found.storages.iter().map(|s| Held { name: format!("{DATA_FOLDER}/{}", s.key), at: s.at, len: s.len, method: 0 }));
        Ok(out)
    }

    /// The entries the central directory names, or nothing at all when this
    /// is not an archive or the directory does not read.
    fn directory<S: Source>(&mut self, doc: &Document<S>, space: u32, end: u64) -> R<Vec<Held>> {
        let none = Vec::new();
        let look = end.min(MOST_COMMENT);
        if look < END_RECORD {
            return Ok(none);
        }
        let tail = self.read_in(doc, space, (end - look) * 8, look * 8)?;
        // The last end record, since a comment may hold the same four bytes.
        let Some(found) = (0..=tail.len() - END_RECORD as usize).rev().find(|i| tail[*i..*i + 4] == *END) else {
            return Ok(none);
        };
        let at = |i: usize| -> u64 { u32::from_le_bytes(tail[found + i..found + i + 4].try_into().unwrap_or([0; 4])) as u64 };
        let short = |i: usize| -> u64 { u16::from_le_bytes(tail[found + i..found + i + 2].try_into().unwrap_or([0; 2])) as u64 };
        let (mut count, mut size, mut start) = (short(10), at(12), at(16));
        // Past four gigabytes the numbers do not fit, and the real ones are in
        // a record of their own that a locator in front of the end record
        // points at. Every checkpoint of a large model is one of these.
        if count == WIDE16 || size == WIDE32 || start == WIDE32 {
            let Some((held, at)) = self.zip64_end(doc, space, &tail, found)? else { return Ok(none) };
            let _ = at;
            (count, size, start) = held;
        }
        if size == 0 || start.checked_add(size).is_none_or(|to| to > end) {
            return Ok(none);
        }
        let held = self.read_in(doc, space, start * 8, size * 8)?;
        let mut out = Vec::new();
        let mut cursor = 0usize;
        while out.len() < MOST_ENTRIES && (out.len() as u64) < count.max(1) {
            let Some(record) = held.get(cursor..cursor + CENTRAL_RECORD as usize) else { break };
            if record[..4] != *CENTRAL {
                break;
            }
            let short = |i: usize| u16::from_le_bytes([record[i], record[i + 1]]) as u64;
            let long = |i: usize| u32::from_le_bytes([record[i], record[i + 1], record[i + 2], record[i + 3]]) as u64;
            let (method, mut packed) = (short(10), long(20));
            let (name_len, extra_len, comment_len) = (short(28), short(30), short(32));
            let mut local = long(42);
            let names_at = cursor + CENTRAL_RECORD as usize;
            let Some(name) = held.get(names_at..names_at + name_len as usize) else { break };
            let name = String::from_utf8_lossy(name).replace('\\', "/");
            let extra = held.get(names_at + name_len as usize..names_at + name_len as usize + extra_len as usize).unwrap_or(&[]);
            // The 64-bit sizes, in the order ZIP64 writes them and only for the
            // fields whose 32-bit place holds the mark saying so.
            if packed == WIDE32 || local == WIDE32 {
                let wide = zip64_extra(extra);
                let mut next = wide.iter().copied();
                if long(24) == WIDE32 {
                    next.next();
                }
                if packed == WIDE32 {
                    packed = next.next().unwrap_or(packed);
                }
                if local == WIDE32 {
                    local = next.next().unwrap_or(local);
                }
            }
            cursor = names_at + (name_len + extra_len + comment_len) as usize;
            // Where the data begins, which only the local header says: the
            // extra field there is padded for alignment and is not the one the
            // directory carries.
            let Ok(head) = self.read_in(doc, space, local * 8, LOCAL_HEADER * 8) else { continue };
            if head.len() < LOCAL_HEADER as usize || head[..4] != *LOCAL {
                continue;
            }
            let here = |i: usize| u16::from_le_bytes([head[i], head[i + 1]]) as u64;
            let data_at = local + LOCAL_HEADER + here(26) + here(28);
            if data_at.checked_add(packed).is_none_or(|to| to > end) {
                continue;
            }
            out.push(Held { name, at: data_at, len: packed, method: method as u16 });
        }
        Ok(out)
    }

    /// The counts and the place of the directory as ZIP64 writes them, found
    /// through the locator that sits in front of the end record.
    #[allow(clippy::type_complexity)]
    fn zip64_end<S: Source>(&mut self, doc: &Document<S>, space: u32, tail: &[u8], found: usize) -> R<Option<((u64, u64, u64), u64)>> {
        let Some(at) = found.checked_sub(LOCATOR_RECORD as usize) else { return Ok(None) };
        let Some(locator) = tail.get(at..at + LOCATOR_RECORD as usize) else { return Ok(None) };
        if locator[..4] != *LOCATOR {
            return Ok(None);
        }
        let where_at = u64::from_le_bytes(locator[8..16].try_into().unwrap_or([0; 8]));
        let record = self.read_in(doc, space, where_at * 8, 56 * 8)?;
        if record.len() < 56 || record[..4] != *END64 {
            return Ok(None);
        }
        let long = |i: usize| u64::from_le_bytes(record[i..i + 8].try_into().unwrap_or([0; 8]));
        Ok(Some(((long(32), long(40), long(48)), where_at)))
    }

    /// The summary over a state dict: one row a tensor, saying what a reader
    /// opens a checkpoint to find out.
    ///
    /// Nothing is read out of the storages here. Every column is something the
    /// pickle already said, so the table costs one read a row, for the key.
    pub(super) fn tensor_rows<S: Source>(
        &mut self,
        doc: &Document<S>,
        r: &Resolved,
        base: u64,
        held: &[(&Captured, &Tensor)],
        from: u64,
        to: u64,
    ) -> R<Vec<Vec<Option<Value>>>> {
        let mut out = Vec::new();
        for (key, tensor) in held.iter().skip(from as usize).take(to.saturating_sub(from) as usize) {
            let name = self.pickle_text(doc, r, base, key)?;
            out.push(vec![
                name.map(Value::Str),
                Some(Value::Str(tensor.dtype.word().to_string())),
                Some(Value::Str(super::pickleparts::extent(&tensor.size))),
                Some(Value::UInt(u128::from(tensor.values()))),
            ]);
        }
        Ok(out)
    }

    /// A run of the pickle read as the text it is, for the two the tensor
    /// names: a run written once and referred to again is read where it was
    /// written, which is why this goes through the pickle field rather than
    /// through the tensor's own bytes.
    fn run_text<S: Source>(&self, doc: &Document<S>, r: &Resolved, base: u64, (at, len): (usize, usize)) -> R<String> {
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
    ) -> R<Vec<Vec<Option<Value>>>> {
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

    /// One value of a tensor, read where the stride puts it.
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
    ) -> R<Option<Value>> {
        let Some(elem) = tensor.element(row, column) else { return Ok(None) };
        let Some(at) = elem.checked_mul(width) else { return Ok(None) };
        if at.checked_add(width).is_none_or(|end| end > len) {
            return Ok(None);
        }
        let offset = (start + at) * 8;
        // A complex number is a pair: the real part and then the imaginary
        // one, each the width the type in hand reads. One cell, because one
        // element of the tensor is one number in Python's own spelling.
        let size = match tensor.dtype.complex() {
            true => width * 4,
            false => width * 8,
        };
        let one = Resolved { offset, cursor: offset, limit: offset + size, size: Some(size), ..r.clone() };
        let real = self.primitive_value(doc, &[], &one, ty, size)?;
        if !tensor.dtype.complex() {
            return Ok(Some(real));
        }
        let next = Resolved { offset: offset + size, cursor: offset + size, limit: offset + size * 2, size: Some(size), ..r.clone() };
        let imaginary = self.primitive_value(doc, &[], &next, ty, size)?;
        Ok(Some(Value::Str(complex_said(&real, &imaginary))))
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

/// The 64-bit numbers an entry's ZIP64 extra field holds, in the order it
/// writes them: the unpacked size, the packed size, where the local header is,
/// and which disk it is on. Only the ones whose 32-bit place held the mark are
/// written, so the caller takes them in turn.
fn zip64_extra(extra: &[u8]) -> Vec<u64> {
    let mut at = 0usize;
    while let Some(head) = extra.get(at..at + 4) {
        let id = u16::from_le_bytes([head[0], head[1]]) as u64;
        let len = u16::from_le_bytes([head[2], head[3]]) as usize;
        let Some(body) = extra.get(at + 4..at + 4 + len) else { return Vec::new() };
        if id == ZIP64_EXTRA {
            return body.chunks_exact(8).map(|b| u64::from_le_bytes(b.try_into().unwrap_or([0; 8]))).collect();
        }
        at += 4 + len;
    }
    Vec::new()
}

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
        notes.extend(
            [Says::Numbers, Says::StoredAt]
                .into_iter()
                .map(|says| (Label::Field(says.name()), Part::Summary { of: v, says })),
        );
        let kids = match call_of(found, v) {
            Some(call) => vec![(Label::Field(call.name), Part::Call(call, v))],
            None => Vec::new(),
        };
        (notes, kids)
}
