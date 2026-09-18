//! The NumPy productions: the calls that rebuild an array or a scalar, the
//! dtype, shape and storage order they are given, and the bytes their numbers
//! sit in. Only the NumPy form allows any of them.

use super::cursor::Cursor;
use super::memo::Bound;
use super::{Kind, Value, MAX_DIMENSIONS, NO_OPCODE};
use crate::formats::pickle::{known::Payload, shapes};

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

    /// The dtype of an array: the whole `numpy.dtype` construction and the
    /// BUILD that gives it its byte order, or a reference to a slot already
    /// holding a completed one.
    ///
    /// Comes back as the `<f4` spelling, which is the one thing that says how
    /// to read the numbers.
    fn dtype(&mut self) -> Option<String> {
        // A slot holding a finished dtype stands for the whole construction.
        // Any other reference here is the module name of the `numpy.dtype`
        // class, which the construction itself reads.
        self.gate()?;
        if self.at_reference() {
            let here = self.save();
            if let Some(Bound::Dtype(dtype)) = self.reference().cloned() {
                return Some(dtype);
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
        // Only these plain numeric dtype productions have this exact state.
        if !matches!(
            kind,
            "b1" | "i1" | "i2" | "i4" | "i8" | "u1" | "u2" | "u4" | "u8" | "f2" | "f4" | "f8" | "c8" | "c16"
        ) {
            return None;
        }
        let kind = kind.to_string();
        self.says("dtype", kind_at, kind_len);
        self.memoize(Bound::Text { at: kind_at, len: kind_len })?;
        // NEWFALSE NEWTRUE TUPLE3, the two flags every plain dtype is built
        // with, and then the call that makes it.
        self.atoms(&[b"\x89", b"\x88"])?;
        self.exact(b"\x87")?;
        self.memoize(Bound::Opaque)?;
        self.exact(b"R")?;
        let slot = self.memoize(Bound::Opaque)?;
        // The state the BUILD sets: version 3, the byte order, three Nones,
        // no field offsets, and an alignment of zero.
        self.atoms(&[b"(", b"K\x03"])?;
        let order = self.byte_order(kind.as_str())?;
        self.atoms(&[b"N", b"N", b"N", b"J\xff\xff\xff\xff", b"J\xff\xff\xff\xff", b"K\0"])?;
        self.exact(b"t")?;
        self.memoize(Bound::Opaque)?;
        self.exact(b"b")?;
        let dtype = format!("{order}{kind}");
        self.memo.fill(slot, Bound::Dtype(dtype.clone()));
        Some(dtype)
    }

    /// The letter that says which way round a dtype's bytes go, spelled out
    /// here or referred to where it was spelled. A single-byte dtype has no
    /// order and says so with `|`.
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
        let one_byte = matches!(kind, "b1" | "i1" | "u1");
        if (one_byte && order != b'|') || (!one_byte && !matches!(order, b'<' | b'>')) {
            return None;
        }
        Some(char::from(order))
    }

    /// The bytes an array's numbers sit in, checked against its shape.
    fn numbers(&mut self, dtype: &str, dimensions: &[u64]) -> Option<(usize, usize, Payload)> {
        self.gate()?;
        let code = self.byte()?;
        let (at, len) = self.counted(code, b'C', b'B', 0x8e)?;
        let payload = self.fits(dtype, dimensions, len)?;
        Some((at, len, payload))
    }

    /// Whether this many bytes is what the dtype and the shape come to.
    fn fits(&self, dtype: &str, dimensions: &[u64], len: usize) -> Option<Payload> {
        let (shape, width) = shapes::dtype(dtype)?;
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
        for production in [Cursor::reconstructed, Cursor::frombuffer, Cursor::scalar] {
            let here = self.save();
            match production(self, start) {
                Some(value) => return Some(value),
                None => self.restore(here),
            }
        }
        None
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
        self.memoize(Bound::Opaque)?;
        let payload = self.fits(&dtype, &dimensions, len)?;
        // The buffer is one of the call's arguments rather than something
        // handed to the array afterwards, so the call covers it.
        self.finish_call("ndarray frombuffer call", start, self.at);
        self.arrays += 1;
        self.payloads.push((at, payload));
        Some(self.span(
            start,
            Kind::Array { at, len, dtype, dimensions, fortran_order },
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
        if self.at_reference() {
            match self.reference().cloned() {
                Some(Bound::Bytes { at, len: 1 }) if self.bytes.get(at) == Some(&b'b') => {}
                _ => return None,
            }
        } else {
            self.exact(b"C\x01b")?;
            let at = self.at - 1;
            self.memoize(Bound::Bytes { at, len: 1 })?;
        }
        self.exact(b"\x87")?;
        self.memoize(Bound::Opaque)?;
        self.exact(b"R")?;
        self.memoize(Bound::Opaque)?;
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
        let (at, len, payload) = self.numbers(&dtype, &dimensions)?;
        self.finish_call("ndarray reconstruct call", start, call_ends);
        self.memoize(Bound::Bytes { at, len })?;
        self.exact(b"t")?;
        self.memoize(Bound::Opaque)?;
        self.exact(b"b")?;
        self.arrays += 1;
        self.payloads.push((at, payload));
        Some(self.span(
            start,
            Kind::Array { at, len, dtype, dimensions, fortran_order },
        ))
    }

    /// `numpy._core.multiarray.scalar(dtype, bytes)`, which is how a single
    /// NumPy number is written. Its shape has no dimensions, so its bytes are
    /// exactly one value wide.
    fn scalar(&mut self, start: usize) -> Option<Value> {
        const MODULES: &[&str] = &["numpy._core.multiarray", "numpy.core.multiarray"];
        self.global(MODULES, "scalar", "module", "callable")?;
        let dtype = self.dtype()?;
        let call_ends = self.at;
        let (at, len, payload) = self.numbers(&dtype, &[1])?;
        self.finish_call("numpy scalar call", start, call_ends);
        self.memoize(Bound::Bytes { at, len })?;
        self.exact(b"\x86")?;
        self.memoize(Bound::Opaque)?;
        self.exact(b"R")?;
        self.memoize(Bound::Opaque)?;
        self.arrays += 1;
        self.payloads.push((at, payload));
        Some(self.span(
            start,
            Kind::Array { at, len, dtype, dimensions: Vec::new(), fortran_order: false },
        ))
    }
}
