//! What an array's values are: the `numpy.dtype` construction, the BUILD that
//! finishes it, and the two things that construction can turn out to be.
//!
//! NumPy writes both the same way. A dtype is a call to the class with a
//! letter code, two flags and the state a BUILD hands it, and the state says
//! whether this is one number a value or a record of named columns. A plain
//! dtype's state has nothing where the names, the columns and the width would
//! be; a structured one has all three, and that is the whole of the
//! difference. A record array is what scikit-learn writes its tree of nodes
//! as, and it is what any structured array is.

use super::cursor::Cursor;
use super::memo::Bound;
use super::{Column, Dtype, Kind, MAX_BATCH};
use crate::formats::pickle::shapes;

/// The letter codes a plain dtype may be spelled with, which are the ones the
/// reader has a type for.
const PLAIN: &[&str] = &["b1", "i1", "i2", "i4", "i8", "u1", "u2", "u4", "u8", "f2", "f4", "f8", "c8", "c16"];
/// The most columns a structured dtype may name. NumPy's own limit is far
/// higher; this bounds the loop and is well past any record a file holds.
const MAX_COLUMNS: usize = 256;

impl Cursor<'_> {
    /// The dtype of an array: the whole `numpy.dtype` construction and the
    /// BUILD that finishes it, or a reference to a slot already holding a
    /// completed one.
    pub(super) fn dtype(&mut self) -> Option<Dtype> {
        self.dtype_construction(true)
    }

    /// The same, for a dtype standing inside another one.
    ///
    /// A structured dtype names a dtype per column, and each of those is this
    /// construction again. NumPy lets one of those be structured in turn; no
    /// file in the corpus writes that, so a column's dtype is a plain one and
    /// the recursion is one level deep and cannot grow.
    fn column_dtype(&mut self) -> Option<Dtype> {
        self.dtype_construction(false)
    }

    fn dtype_construction(&mut self, records: bool) -> Option<Dtype> {
        // A slot holding a finished dtype stands for the whole construction.
        // Any other reference here is the module name of the `numpy.dtype`
        // class, which the construction itself reads.
        self.gate()?;
        if self.at_reference() {
            let here = self.save();
            if let Some(Bound::Dtype(dtype)) = self.reference().cloned() {
                if records || matches!(dtype, Dtype::Plain(_)) {
                    return Some(dtype);
                }
            }
            self.restore(here);
        }
        self.global(&["numpy"], "dtype", "dtype module", "dtype class")?;
        let (kind_at, kind_len) = {
            self.gate()?;
            self.exact(&[0x8c])?;
            let len = self.byte()? as usize;
            let at = self.at;
            self.take(len)?;
            (at, len)
        };
        let kind = std::str::from_utf8(self.bytes.get(kind_at..kind_at + kind_len)?).ok()?;
        // `V` and a width is a record; everything else is one number a value,
        // and only the spellings the reader has a type for.
        let record = records && kind.strip_prefix('V').is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        if !record && !PLAIN.contains(&kind) {
            return None;
        }
        let kind = kind.to_string();
        self.says("dtype", kind_at, kind_len);
        self.memoize(Bound::Text { at: kind_at, len: kind_len })?;
        // NEWFALSE NEWTRUE TUPLE3, the two flags every dtype is built with,
        // and then the call that makes it.
        self.atoms(&[b"\x89", b"\x88"])?;
        self.exact(b"\x87")?;
        self.memoize(Bound::Opaque)?;
        self.exact(b"R")?;
        let slot = self.memoize(Bound::Opaque)?;
        // The state the BUILD sets: version 3, the byte order, no subarray,
        // and then either nothing where a record's names, columns and width
        // would be, or all three of them.
        self.atoms(&[b"(", b"K\x03"])?;
        let order = self.byte_order(&kind)?;
        self.atoms(&[b"N"])?;
        let dtype = match record {
            true => self.record_state(&kind)?,
            false => {
                self.atoms(&[b"N", b"N", b"J\xff\xff\xff\xff", b"J\xff\xff\xff\xff", b"K\0"])?;
                Dtype::Plain(format!("{order}{kind}"))
            }
        };
        self.exact(b"t")?;
        self.memoize(Bound::Opaque)?;
        self.exact(b"b")?;
        self.memo.fill(slot, Bound::Dtype(dtype.clone()));
        Some(dtype)
    }

    /// The rest of a structured dtype's state: the column names, the column
    /// dtypes and offsets, and the three numbers that close it.
    ///
    /// The names come first as a tuple, and the columns after them as a
    /// dictionary keyed by those same names, which the file names out of the
    /// memo rather than spelling twice. So the names are read once and the
    /// dictionary is checked against them.
    fn record_state(&mut self, kind: &str) -> Option<Dtype> {
        let names = self.column_names()?;
        let mut columns = self.columns(&names)?;
        let width = self.count()?;
        // The alignment and the flags, which are data. The width is not: it is
        // what the numbers of an array of this dtype are measured against, and
        // `V64` says the same thing in the letter code.
        self.count()?;
        self.count()?;
        if kind.strip_prefix('V')?.parse::<u64>().ok()? != width {
            return None;
        }
        // Every column has to fit inside a record, and they are written in the
        // order they sit in.
        let mut reached = 0;
        for column in &columns {
            let held = shapes::dtype(&column.dtype)?.1;
            if column.at < reached || column.at.checked_add(held)? > width {
                return None;
            }
            reached = column.at + held;
        }
        columns.shrink_to_fit();
        Some(Dtype::Record { columns, width })
    }

    /// The tuple of column names, in the order NumPy names them.
    fn column_names(&mut self) -> Option<Vec<(usize, usize)>> {
        self.gate()?;
        let marked = self.peek()? == b'(';
        if marked {
            self.byte()?;
        }
        let mut names = Vec::new();
        loop {
            if names.len() == MAX_COLUMNS {
                return None;
            }
            self.gate()?;
            self.exact(&[0x8c])?;
            let len = self.byte()? as usize;
            let at = self.at;
            self.take(len)?;
            std::str::from_utf8(self.bytes.get(at..at + len)?).ok()?;
            self.says("column", at, len);
            self.memoize(Bound::Text { at, len })?;
            names.push((at, len));
            if matches!(self.peek()?, b't' | 0x85..=0x87) {
                break;
            }
        }
        let end = self.byte()?;
        let expected = match names.len() {
            1..=3 if !marked => 0x84 + names.len() as u8,
            4..=MAX_COLUMNS if marked => b't',
            _ => return None,
        };
        if end != expected {
            return None;
        }
        self.memoize(Bound::Opaque)?;
        Some(names)
    }

    /// The dictionary of columns: each name, named out of the memo, against
    /// the pair of a dtype and an offset.
    fn columns(&mut self, names: &[(usize, usize)]) -> Option<Vec<Column>> {
        self.gate()?;
        self.exact(b"}")?;
        self.memoize(Bound::Opaque)?;
        // One entry is written with SETITEM and any longer run with a batch,
        // which a record short enough never has to split.
        let batched = names.len() > 1;
        if batched {
            self.gate()?;
            self.exact(b"(")?;
        }
        if names.len() > MAX_BATCH {
            return None;
        }
        let mut columns = Vec::with_capacity(names.len());
        for (at, len) in names {
            self.gate()?;
            let held = match self.reference()? {
                Bound::Text { at: was, len: held } if (*was, *held) == (*at, *len) => *held,
                _ => return None,
            };
            let name = std::str::from_utf8(self.bytes.get(*at..at + held)?).ok()?.to_string();
            let dtype = match self.column_dtype()? {
                Dtype::Plain(spelling) => spelling,
                Dtype::Record { .. } => return None,
            };
            let Kind::Int { value, .. } = self.integer()?.kind else { return None };
            self.exact(&[0x86])?;
            self.memoize(Bound::Opaque)?;
            columns.push(Column { name, dtype, at: u64::try_from(value).ok()? });
        }
        self.exact(if batched { b"u" } else { b"s" })?;
        Some(columns)
    }

    /// A nonnegative integer, in the width CPython writes it in.
    fn count(&mut self) -> Option<u64> {
        let Kind::Int { value, .. } = self.integer()?.kind else { return None };
        u64::try_from(value).ok()
    }

    /// The letter that says which way round a dtype's bytes go, spelled out
    /// here or referred to where it was spelled. A value with no multi-byte
    /// number in it has no order and says so with `|`, which is every
    /// single-byte type and every record.
    fn byte_order(&mut self, kind: &str) -> Option<char> {
        self.gate()?;
        let (at, len) = if self.at_reference() {
            let here = self.save();
            match self.reference().cloned() {
                Some(Bound::Text { at, len }) => (at, len),
                _ => {
                    self.restore(here);
                    return None;
                }
            }
        } else {
            self.exact(&[0x8c, 1])?;
            let at = self.at;
            self.byte()?;
            self.memoize(Bound::Text { at, len: 1 })?;
            self.says("byte order", at, 1);
            (at, 1)
        };
        if len != 1 {
            return None;
        }
        let order = *self.bytes.get(at)?;
        let orderless = matches!(kind, "b1" | "i1" | "u1") || kind.starts_with('V');
        if (orderless && order != b'|') || (!orderless && !matches!(order, b'<' | b'>')) {
            return None;
        }
        Some(char::from(order))
    }
}
