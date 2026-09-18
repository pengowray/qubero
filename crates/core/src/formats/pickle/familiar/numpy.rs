//! The NumPy productions: the calls that rebuild an array or a scalar, the
//! dtype, shape and storage order they are given, and the bytes their numbers
//! sit in. Only the NumPy form allows any of them.

use super::cursor::Cursor;
use super::memo::Bound;
use super::{Dtype, Kind, Names, Shape, Storage, Value, MAX_BATCH, MAX_DIMENSIONS, NO_OPCODE};
use crate::formats::pickle::known::Payload;

/// How many values a shape holds, which is what a list of objects has to come
/// to. Zero when any dimension is.
fn count_of(dimensions: &[u64]) -> Option<u64> {
    match dimensions.contains(&0) {
        true => Some(0),
        false => dimensions.iter().try_fold(1u64, |n, dim| n.checked_mul(*dim)),
    }
}

impl Cursor<'_> {
    /// One dimension of a shape, which is written as a nonnegative integer in
    /// whichever of the three widths holds it.
    fn dimension(&mut self) -> Option<u64> {
        let Kind::Int { value, .. } = self.integer()?.kind else { return None };
        u64::try_from(value).ok()
    }

    /// A shape, as the tuple its arity is written with: EMPTY_TUPLE for none,
    /// TUPLE1 to TUPLE3 for one to three, and MARK..TUPLE past that. Only the
    /// counted tuples carry a memo mark; CPython never files an empty one.
    fn dimensions(&mut self) -> Option<Vec<u64>> {
        self.gate()?;
        if self.peek()? == b')' {
            self.byte()?;
            return Some(Vec::new());
        }
        let marked = self.peek()? == b'(';
        if marked {
            self.byte()?;
        }
        self.gate()?;
        let mut dims = Vec::new();
        loop {
            if dims.len() == MAX_DIMENSIONS {
                return None;
            }
            dims.push(self.dimension()?);
            if matches!(self.peek()?, b't' | 0x85..=0x87) {
                break;
            }
        }
        let end = self.byte()?;
        let expected = match dims.len() {
            1..=3 if !marked => 0x84 + dims.len() as u8,
            4..=MAX_DIMENSIONS if marked => b't',
            _ => return None,
        };
        if end != expected {
            return None;
        }
        self.memoize(Bound::Opaque)?;
        Some(dims)
    }

    /// The bytes an array's numbers sit in, checked against its shape.
    ///
    /// Protocol 2 has no opcode for a byte string, so the numbers arrive there
    /// as the latin-1 text they spell, handed to `_codecs.encode`. The run in
    /// the file is then that text rather than the numbers, and what comes back
    /// says which it is. The memo mark the run is filed under is written by
    /// the caller either way: it is the byte string's slot at protocol 3 and
    /// the REDUCE's at protocol 2, and both hold the same byte string.
    fn numbers(&mut self, dtype: &Dtype, dimensions: &[u64]) -> Option<(usize, usize, Payload, Storage)> {
        self.gate()?;
        if self.proto < 3 {
            let (at, len, storage) = self.bytes_run()?;
            if storage == Storage::Raw {
                let payload = self.fits(dtype, dimensions, len)?;
                return Some((at, len, payload, storage));
            }
            // Read once here rather than per cell: the numbers are nowhere in
            // the file, and everything that wants them wants all of them.
            let held = storage.read(self.bytes.get(at..at + len)?)?;
            let payload = self.fits(dtype, dimensions, held.len())?;
            self.runs.push((at, std::sync::Arc::new(held)));
            return Some((at, len, payload, storage));
        }
        let code = self.byte()?;
        let (at, len) = self.counted(code, b'C', b'B', if self.proto >= 4 { 0x8e } else { NO_OPCODE })?;
        let payload = self.fits(dtype, dimensions, len)?;
        Some((at, len, payload, Storage::Raw))
    }

    /// Whether this many bytes is what the dtype and the shape come to.
    fn fits(&self, dtype: &Dtype, dimensions: &[u64], len: usize) -> Option<Payload> {
        // A record has no case in the payload table, so the opcode listing
        // shows it as the bytes it is and the familiar form reads the columns.
        let shape = match dtype {
            Dtype::Plain(spelling) => crate::formats::pickle::shapes::dtype(spelling)?.0,
            Dtype::Record { .. } => 0,
            Dtype::Datetime { spelling, .. } => crate::formats::pickle::shapes::dtype(spelling)?.0,
            // An array of objects never reaches here: its values are read
            // rather than measured.
            Dtype::Objects => return None,
        };
        let width = dtype.width()?;
        // Check every dimension even when another one is zero. Dimensions
        // originate from nonnegative i32 values; the product is bounded too.
        let count = if dimensions.contains(&0) {
            0
        } else {
            dimensions.iter().try_fold(1u64, |n, dim| n.checked_mul(*dim))?
        };
        if len as u64 != count.checked_mul(width)? {
            return None;
        }
        Some(Payload { shape, count })
    }

    /// A NumPy array or a NumPy scalar, each rebuilt by the one call the form
    /// names and fixed instruction for instruction around it.
    pub(super) fn numpy(&mut self) -> Option<Value> {
        let start = self.at;
        for production in [Cursor::reconstructed, Cursor::frombuffer, Cursor::scalar, Cursor::dtype_value] {
            let here = self.save();
            match production(self, start) {
                Some(value) => return Some(value),
                None => self.restore(here),
            }
        }
        None
    }

    /// A dtype standing on its own rather than describing an array, which is
    /// what pandas hands a datetime column beside its numbers. Tried after the
    /// three array productions, which name their own callable first, so a
    /// dtype inside one of them is never read as one of these.
    fn dtype_value(&mut self, start: usize) -> Option<Value> {
        let dtype = self.dtype()?;
        // The names it was built from belong to this run and not to whatever
        // call comes next, so the run is closed here like any other.
        self.finish_call("numpy dtype call", start, self.at);
        Some(self.span(start, Kind::DType(dtype)))
    }

    /// `numpy.core.numeric._frombuffer(buffer, dtype, shape, order)`, which
    /// is what NumPy writes at protocol 5.
    ///
    /// The numbers travel as a buffer rather than inside the reconstruct
    /// call: a bytearray when the array was writable, so that the array the
    /// reader builds is writable too, and a byte string when it was not. An
    /// array that is not laid out contiguously has no buffer to hand over and
    /// is written the protocol 4 way even at protocol 5, so both productions
    /// belong to one form.
    fn frombuffer(&mut self, start: usize) -> Option<Value> {
        if self.proto < 5 {
            return None;
        }
        const MODULES: &[&str] = &["numpy._core.numeric", "numpy.core.numeric"];
        self.global(MODULES, "_frombuffer", "module", "callable")?;
        self.atoms(&[b"("])?;
        self.gate()?;
        let code = self.byte()?;
        let (at, len) = match code {
            0x96 => self.counted(code, NO_OPCODE, NO_OPCODE, 0x96)?,
            b'C' | b'B' | 0x8e => self.counted(code, b'C', b'B', 0x8e)?,
            _ => return None,
        };
        // A bytearray is not a byte string, and nothing ever names one of
        // these again, so the slot it goes in stays opaque. Its memo mark is
        // the one `pickle.py` did not write before Python 3.10, which is why
        // that pickler's protocol 5 arrays are a byte shorter.
        match code {
            0x96 => self.bytearray_memoize(Bound::Opaque)?,
            _ => {
                self.memoize(Bound::Bytes { at, len })?;
            }
        }
        let dtype = self.dtype()?;
        let dimensions = self.dimensions()?;
        let fortran_order = self.storage_order()?;
        self.exact(b"t")?;
        self.memoize(Bound::Opaque)?;
        self.exact(b"R")?;
        self.memoize(Bound::Made { what: Shape::Array, at: start, hashable: false })?;
        let payload = self.fits(&dtype, &dimensions, len)?;
        // The buffer is one of the call's arguments rather than something
        // handed to the array afterwards, so the call covers it.
        self.finish_call("ndarray frombuffer call", start, self.at);
        self.arrays += 1;
        // A buffer handed to `_frombuffer` is the bytes as they are: this
        // production is protocol 5's, and protocol 5 has a type for them.
        self.reads(at, payload, Storage::Raw);
        Some(self.span(
            start,
            Kind::Array { at, len, dtype, dimensions, fortran_order, storage: Storage::Raw },
        ))
    }

    /// The letter `_frombuffer` is told to lay the numbers out by, spelled
    /// out here or named where an earlier array spelled it.
    fn storage_order(&mut self) -> Option<bool> {
        for (letter, fortran) in [("C", false), ("F", true)] {
            let here = self.save();
            if let Some(said) = self.word_or_reference(letter) {
                if let Some((at, len)) = said {
                    self.says("order letter", at, len);
                }
                return Some(fortran);
            }
            self.restore(here);
        }
        None
    }

    /// `numpy._core.multiarray._reconstruct(ndarray, (0,), b'b')` and the
    /// BUILD that hands the result its shape, dtype, storage order and bytes.
    fn reconstructed(&mut self, start: usize) -> Option<Value> {
        const MODULES: &[&str] = &["numpy._core.multiarray", "numpy.core.multiarray"];
        self.global(MODULES, "_reconstruct", "module", "callable")?;
        self.global(&["numpy"], "ndarray", "class module", "class")?;
        // The placeholder the reconstructor is given: shape (0,) and a dtype
        // letter, both replaced by the BUILD that follows.
        self.atoms(&[b"K\0"])?;
        self.exact(b"\x85")?;
        self.memoize(Bound::Opaque)?;
        self.gate()?;
        // Protocol 2 writes the placeholder byte string the way it writes
        // every other one, as a call to `_codecs.encode`, and the call itself
        // names its callable and its encoding out of the memo once the file
        // has written them once. So the call is tried first there, and a
        // reference stands for the whole byte string only when it is not one.
        let spelled = self.proto < 3 && {
            let here = self.save();
            match self.bytes_run() {
                Some((at, 1, _)) if self.bytes.get(at) == Some(&b'b') => {
                    self.memoize(Bound::Bytes { at, len: 1 })?;
                    true
                }
                _ => {
                    self.restore(here);
                    false
                }
            }
        };
        if spelled {
        } else if self.at_reference() {
            match self.reference().cloned() {
                Some(Bound::Bytes { at, len: 1 }) if self.bytes.get(at) == Some(&b'b') => {}
                _ => return None,
            }
        } else if self.proto < 3 {
            return None;
        } else {
            self.exact(b"C\x01b")?;
            let at = self.at - 1;
            self.memoize(Bound::Bytes { at, len: 1 })?;
        }
        self.exact(b"\x87")?;
        self.memoize(Bound::Opaque)?;
        self.exact(b"R")?;
        // The array itself, which a later part of the file may name: a pandas
        // block manager writes its values once and names them again in the
        // dictionary it versions its state with.
        self.memoize(Bound::Made { what: Shape::Array, at: start, hashable: false })?;
        self.atoms(&[b"(", b"K\x01"])?;
        let dimensions = self.dimensions()?;
        let dtype = self.dtype()?;
        self.gate()?;
        let fortran_order = match self.byte()? {
            0x89 => false,
            0x88 => true,
            _ => return None,
        };
        let call_ends = self.at;
        // An array of objects has no buffer. Its values are pickled after it,
        // as the list the array is handed, and they are read as values rather
        // than measured against a width.
        if dtype == Dtype::Objects {
            let items = self.object_values(count_of(&dimensions)?)?;
            self.finish_call("ndarray reconstruct call", start, call_ends);
            self.exact(b"t")?;
            self.memoize(Bound::Opaque)?;
            self.exact(b"b")?;
            self.arrays += 1;
            return Some(self.span(start, Kind::Objects { dimensions, fortran_order, items }));
        }
        let (at, len, payload, storage) = self.numbers(&dtype, &dimensions)?;
        self.finish_call("ndarray reconstruct call", start, call_ends);
        self.memoize(Bound::Bytes { at, len })?;
        self.exact(b"t")?;
        self.memoize(Bound::Opaque)?;
        self.exact(b"b")?;
        self.arrays += 1;
        self.reads(at, payload, storage);
        Some(self.span(
            start,
            Kind::Array { at, len, dtype, dimensions, fortran_order, storage },
        ))
    }

    /// Tell the opcode listing that this run of bytes is an array's numbers,
    /// which it shows as the values they are.
    ///
    /// Only where the run really is those bytes. A protocol 2 array's numbers
    /// are a text run that spells them in latin-1, and reading that run as
    /// numbers would be reading the spelling rather than the numbers.
    fn reads(&mut self, at: usize, payload: Payload, storage: Storage) {
        if storage == Storage::Raw {
            self.payloads.push((at, payload));
        }
    }

    /// The list of values an array of objects is handed, which is an ordinary
    /// list written the way CPython writes one: nothing at all, one value and
    /// APPEND, or a run of batches of a thousand.
    ///
    /// The values are text, the singletons, and names for text written earlier,
    /// which is what a pandas index of column names holds. Anything else is a
    /// non-match: an array of objects can hold whatever was pickled into it,
    /// and only what a form has written down is read.
    fn object_values(&mut self, count: u64) -> Option<Vec<Value>> {
        self.gate()?;
        self.exact(b"]")?;
        let at = self.at - 1;
        self.memoize(Bound::Made { what: Shape::List, at, hashable: false })?;
        let mut items = Vec::new();
        if count == 0 {
            return Some(items);
        }
        if count == 1 {
            items.push(self.object_value()?);
            self.exact(b"a")?;
            return Some(items);
        }
        while (items.len() as u64) < count {
            let want = (count - items.len() as u64).min(MAX_BATCH as u64);
            // A single value left over after a full batch is APPEND in
            // `pickle.py` and a batch of one in the C pickler; both are read,
            // and neither says which wrote the file, since the list inside an
            // array is not written by the loop the tells were measured on.
            if want == 1 && self.peek()? != b'(' {
                items.push(self.object_value()?);
                self.exact(b"a")?;
                continue;
            }
            self.gate()?;
            self.exact(b"(")?;
            for _ in 0..want {
                items.push(self.object_value()?);
            }
            self.exact(b"e")?;
        }
        Some(items)
    }

    /// One value inside an array of objects.
    fn object_value(&mut self) -> Option<Value> {
        self.gate()?;
        match self.peek()? {
            0x8c | 0x58 | 0x8d => self.text(),
            b'h' | b'j' => {
                let start = self.at;
                let names = match self.reference()?.clone() {
                    Bound::Text { at, len } => Names::Text { at, len },
                    _ => return None,
                };
                Some(self.span(start, Kind::Ref(names)))
            }
            b'N' => {
                let start = self.at;
                self.byte()?;
                Some(self.span(start, Kind::None))
            }
            _ => None,
        }
    }

    /// `numpy._core.multiarray.scalar(dtype, bytes)`, which is how a single
    /// NumPy number is written. Its shape has no dimensions, so its bytes are
    /// exactly one value wide.
    fn scalar(&mut self, start: usize) -> Option<Value> {
        const MODULES: &[&str] = &["numpy._core.multiarray", "numpy.core.multiarray"];
        self.global(MODULES, "scalar", "module", "callable")?;
        let dtype = self.dtype()?;
        let call_ends = self.at;
        let (at, len, payload, storage) = self.numbers(&dtype, &[1])?;
        self.finish_call("numpy scalar call", start, call_ends);
        self.memoize(Bound::Bytes { at, len })?;
        self.exact(b"\x86")?;
        self.memoize(Bound::Opaque)?;
        self.exact(b"R")?;
        // A scalar is one number, and Python hashes it.
        self.memoize(Bound::Made { what: Shape::Array, at: start, hashable: true })?;
        self.arrays += 1;
        self.reads(at, payload, storage);
        Some(self.span(
            start,
            Kind::Array { at, len, dtype, dimensions: Vec::new(), fortran_order: false, storage },
        ))
    }
}
