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
/// The units a datetime may count in, which are NumPy's own and are the whole
/// of what its numbers mean.
const UNITS: &[&str] = &["Y", "M", "W", "D", "h", "m", "s", "ms", "us", "ns", "ps", "fs", "as"];

/// How wide one value of a fixed-width text dtype is, how NumPy aligns it and
/// the flags it is built with, or nothing when the letters are not text.
///
/// The letters count characters rather than bytes, so unlike every other plain
/// dtype the width has to come out of the state: `S5` is five bytes and `U5`
/// is five four-byte characters, which is UTF-32. A classifier fitted on
/// labels that are strings keeps them in an array of one of these.
fn text_bounds(kind: &str) -> Option<(i128, i128, i128)> {
    let (per, alignment, flags) = match kind.as_bytes().first()? {
        b'S' => (1, 1, 0),
        b'U' => (4, 4, 8),
        _ => return None,
    };
    let count: i128 = kind.get(1..)?.parse().ok()?;
    (count > 0).then_some((count * per, alignment, flags))
}

impl Cursor<'_> {
    /// The middle of a dtype's state, which every one of them writes the same
    /// way: no subarray, no names, no columns, the width and the alignment,
    /// and the flags the dtype was built with. A dtype whose width is in its
    /// letters writes minus one for both numbers; fixed-width text is the one
    /// plain kind that writes them out, because the letters say how many
    /// characters and not how many bytes.
    fn three_nones_and_bounds(&mut self, width: i128, alignment: i128, flags: i128) -> Option<()> {
        self.atoms(&[b"N", b"N"])?;
        self.number(width)?;
        self.number(alignment)?;
        self.number(flags)
    }

    /// Say what the slot the dtype's REDUCE filed holds, now that the BUILD
    /// after it has said what the dtype is. There is no slot when the file
    /// left the mark out, which only `cPickle` does and only for a value
    /// nothing else names.
    fn fill(&mut self, slot: Option<usize>, dtype: &Dtype) {
        if let Some(slot) = slot {
            self.memo.fill(slot, Bound::Dtype(dtype.clone()));
        }
    }

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
        self.open_tuple()?;
        let (kind_at, kind_len) = self.text_run()?;
        let kind = std::str::from_utf8(self.bytes.get(kind_at..kind_at + kind_len)?).ok()?;
        // `V` and a width is a record; everything else is one number a value,
        // and only the spellings the reader has a type for.
        let record = records && kind.strip_prefix('V').is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        // `O8` is one pickled object a value, which only a form that reads an
        // array of them allows. `M8` is a count of a unit of time, and the
        // unit is in the state rather than in the letters.
        let objects = kind == "O8" && self.allow.object_arrays;
        let datetime = kind == "M8";
        let text = text_bounds(kind);
        if !record && !objects && !datetime && text.is_none() && !PLAIN.contains(&kind) {
            return None;
        }
        let kind = kind.to_string();
        self.says("dtype", kind_at, kind_len);
        self.memoize(Bound::Text { at: kind_at, len: kind_len })?;
        // The two flags every dtype is built with, and then the call that
        // makes it out of them and the letters.
        self.flag(false)?;
        self.flag(true)?;
        self.close_tuple(3)?;
        self.memoize(Bound::Opaque)?;
        self.exact(b"R")?;
        let slot = self.memoize_at(Bound::Opaque)?;
        // The state the BUILD sets: the version, the byte order, no subarray,
        // and then either nothing where a record's names, columns and width
        // would be, or all three of them. A dtype carrying metadata is version
        // 4 and everything else is version 3, and the only metadata read here
        // is the unit a datetime counts in.
        self.atoms(&[b"("])?;
        self.number(if datetime { 4 } else { 3 })?;
        let order = self.byte_order(&kind)?;
        self.atoms(&[b"N"])?;
        if datetime {
            self.three_nones_and_bounds(-1, -1, 0)?;
            let unit = self.datetime_unit()?;
            self.exact(b"t")?;
            self.memoize(Bound::Opaque)?;
            self.exact(b"b")?;
            let dtype = Dtype::Datetime { spelling: format!("{order}{kind}[{unit}]"), unit };
            self.fill(slot, &dtype);
            return Some(dtype);
        }
        let dtype = match record {
            true => self.record_state(&kind)?,
            false => {
                // The flags NumPy builds the dtype with. A dtype of objects
                // needs the interpreter for everything it does and says so;
                // every plain one writes nothing.
                let (width, alignment, flags) = match (objects, text) {
                    (true, _) => (-1, -1, 0x3f),
                    (false, Some(bounds)) => bounds,
                    (false, None) => (-1, -1, 0),
                };
                self.three_nones_and_bounds(width, alignment, flags)?;
                match objects {
                    true => Dtype::Objects,
                    false => {
                        let spelling = format!("{order}{kind}");
                        // The reader has a type per width up to the one the
                        // `.npy` table stops at, and a wider text dtype has
                        // no reading rather than a wrong one.
                        shapes::dtype(&spelling)?;
                        Dtype::Plain(spelling)
                    }
                }
            }
        };
        self.exact(b"t")?;
        self.memoize(Bound::Opaque)?;
        self.exact(b"b")?;
        self.fill(slot, &dtype);
        Some(dtype)
    }

    /// The unit a datetime counts in, which NumPy writes beside a tuple of the
    /// unit, a numerator, a denominator and an event count. What stands beside
    /// it is an empty dictionary up to NumPy 1.x and `None` from 2.x, and
    /// neither says anything. Only a plain unit is read: a count of three days
    /// is a numerator this has no word for.
    fn datetime_unit(&mut self) -> Option<String> {
        self.gate()?;
        self.open_tuple()?;
        self.gate()?;
        // An empty dictionary up to NumPy 1.x and `None` from 2.x, and
        // neither says anything.
        match self.peek()? {
            b'N' => {
                self.byte()?;
            }
            _ => {
                self.empty_dict()?;
                self.memoize(Bound::Opaque)?;
            }
        }
        self.gate()?;
        self.exact(b"(")?;
        self.gate()?;
        // The unit is a byte string, so protocol 2 writes it as the call
        // every byte string goes through there.
        let (at, len) = match self.proto < 3 {
            true => {
                let (at, len, _) = self.bytes_run()?;
                (at, len)
            }
            false => {
                self.exact(b"C")?;
                let len = self.byte()? as usize;
                let at = self.at;
                self.take(len)?;
                (at, len)
            }
        };
        let unit = std::str::from_utf8(self.bytes.get(at..at + len)?).ok()?;
        if !UNITS.contains(&unit) {
            return None;
        }
        let unit = unit.to_string();
        self.says("unit", at, len);
        self.memoize(Bound::Bytes { at, len })?;
        self.number(1)?;
        self.number(1)?;
        self.number(1)?;
        self.exact(b"t")?;
        self.memoize(Bound::Opaque)?;
        self.close_tuple(2)?;
        self.memoize(Bound::Opaque)?;
        Some(unit)
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

    /// One column's name, spelled here or named where the file spelled the
    /// same word before. A column called after something the file has already
    /// written is one slot to Python, and a histogram gradient boosting
    /// model's nodes have a `is_categorical` column beside the estimator's own
    /// attribute of that name.
    fn column_name(&mut self) -> Option<(usize, usize)> {
        self.gate()?;
        if self.at_reference() {
            let here = self.save();
            if let Some(Bound::Text { at, len }) = self.reference().cloned() {
                return Some((at, len));
            }
            self.restore(here);
            return None;
        }
        let (at, len) = self.text_run()?;
        self.says("column", at, len);
        self.memoize(Bound::Text { at, len })?;
        Some((at, len))
    }

    /// The tuple of column names, in the order NumPy names them.
    fn column_names(&mut self) -> Option<Vec<(usize, usize)>> {
        self.gate()?;
        let marked = self.peek()? == b'(';
        if self.proto <= 1 && !marked {
            return None;
        }
        if marked {
            self.byte()?;
        }
        let mut names = Vec::new();
        loop {
            if names.len() == MAX_COLUMNS {
                return None;
            }
            let (at, len) = self.column_name()?;
            std::str::from_utf8(self.bytes.get(at..at + len)?).ok()?;
            names.push((at, len));
            if matches!(self.peek()?, b't' | 0x85..=0x87) {
                break;
            }
        }
        let end = self.byte()?;
        let expected = match names.len() {
            1..=MAX_COLUMNS if marked && self.proto <= 1 => b't',
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
        self.empty_dict()?;
        self.memoize(Bound::Opaque)?;
        // One entry is written with SETITEM and any longer run with a batch,
        // which a record short enough never has to split. Protocol 0 has no
        // batching at all and writes a SETITEM for every entry.
        let batched = names.len() > 1 && self.proto > 0;
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
            // The key is the column's name and the value is the pair of its
            // dtype and where it sits, so the tuple opens after the name.
            self.open_tuple()?;
            let dtype = match self.column_dtype()? {
                Dtype::Plain(spelling) => spelling,
                _ => return None,
            };
            let Kind::Int { value, .. } = self.integer()?.kind else { return None };
            self.close_tuple(2)?;
            self.memoize(Bound::Opaque)?;
            if self.proto == 0 {
                self.exact(b"s")?;
            }
            columns.push(Column { name, dtype, at: u64::try_from(value).ok()? });
        }
        if self.proto > 0 {
            self.exact(if batched { b"u" } else { b"s" })?;
        }
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
            let (at, len) = self.text_run()?;
            self.memoize(Bound::Text { at, len })?;
            self.says("byte order", at, len);
            (at, len)
        };
        if len != 1 {
            return None;
        }
        let order = *self.bytes.get(at)?;
        // A run of bytes has no order either, so `S5` is written `|S5` where
        // `U5` is a run of four-byte characters and has one.
        let orderless = matches!(kind, "b1" | "i1" | "u1" | "O8") || kind.starts_with('V') || kind.starts_with('S');
        if (orderless && order != b'|') || (!orderless && !matches!(order, b'<' | b'>')) {
            return None;
        }
        Some(char::from(order))
    }
}
