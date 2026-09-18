//! joblib's array wrapper: the object `joblib.dump` writes in place of an
//! array, and the run of bytes it measures.
//!
//! joblib subclasses the pure Python pickler. For an array it hands the
//! pickler a small object describing the array, commits the frame that object
//! went in, and writes the array's bytes straight into the file after it. So
//! the stream is a pickle, a run of numbers, more pickle, and the numbers are
//! at an offset the file never states: the wrapper's own shape and dtype
//! measure them, which is what this production does.
//!
//! Nothing new comes out of it. The value is the same [`Kind::Array`] a plain
//! pickled NumPy array is, at the offset the numbers really sit at, so the
//! tree, the table and the reading of the array need nothing added.

use super::cursor::{Cursor, Framing};
use super::memo::Bound;
use super::{Kind, Shape, Storage, Value};

/// The module and the class joblib names for every array it writes.
const MODULE: &str = "joblib.numpy_pickle";
const WRAPPER: &str = "NumpyArrayWrapper";
/// The class of the array the wrapper stands for. `numpy.matrix` and
/// `numpy.memmap` reach the same writer and would be named here; no file in
/// the corpus holds one, so only the class every file names is read.
const SUBCLASS: &str = "ndarray";

/// The attributes the wrapper's state holds, in the order `__init__` sets
/// them, which is the order a `__dict__` is written in. The last is missing
/// from files joblib 1.1 and older wrote, and those have no padding either.
const SUBCLASS_KEY: &str = "subclass";
const SHAPE_KEY: &str = "shape";
const ORDER_KEY: &str = "order";
const DTYPE_KEY: &str = "dtype";
const MMAP_KEY: &str = "allow_mmap";
const ALIGNMENT_KEY: &str = "numpy_array_alignment_bytes";

/// The byte joblib pads with, which is not nought: a run of these is visible
/// in a hex view where a run of noughts reads as the numbers' own zeroes.
const PAD: u8 = 0xff;
/// The widest alignment a file may ask for. joblib writes 16 and says in its
/// own source that it may change its mind, so the number is read from the
/// file; this bounds it to something a byte of padding can reach.
const MOST_ALIGNMENT: u64 = 128;

/// One run of bytes in a joblib file that is not opcodes: where the padding
/// count sits, where the array's numbers start, and where they end.
///
/// Kept so that the listing can name those bytes rather than stop at them:
/// the opcodes are walked in the segments between these runs, and the padding
/// becomes a row of its own. See [`Match::raws`](super::Match).
#[derive(Debug, Clone, Copy)]
pub(super) struct Raw {
    pub(super) pad_at: usize,
    pub(super) data_at: usize,
    pub(super) end: usize,
}

impl Cursor<'_> {
    /// The wrapper joblib writes in place of an array, and the array's bytes
    /// after it.
    ///
    /// Fixed instruction for instruction: the class named exactly,
    /// `cls.__new__(cls)` with no arguments, a state dictionary whose six keys
    /// are these six in this order, and then the run the state measures.
    /// Nothing is called and nothing is run; the object is folded away here
    /// and what comes out is the array it stood for.
    pub(super) fn joblib_array(&mut self) -> Option<Value> {
        // NEWOBJ arrived at protocol 2 and SETITEMS at protocol 1, so below
        // protocol 2 joblib would write a shape this has not been measured
        // against. No file in the corpus is one.
        if self.proto < 2 {
            return None;
        }
        let start = self.at;
        self.global(&[MODULE], WRAPPER, "wrapper module", "wrapper class")?;
        self.empty_tuple()?;
        self.exact(&[0x81])?;
        self.memoize(Bound::Made { what: Shape::Object, at: start, hashable: false })?;
        let dict_at = self.at;
        self.empty_dict()?;
        self.memoize(Bound::Made { what: Shape::Dict, at: dict_at, hashable: false })?;
        self.atoms(&[b"("])?;

        self.key(SUBCLASS_KEY)?;
        self.global(&["numpy"], SUBCLASS, "array module", "array class")?;
        self.key(SHAPE_KEY)?;
        let dimensions = self.dimensions()?;
        self.key(ORDER_KEY)?;
        let fortran_order = self.storage_order()?;
        self.key(DTYPE_KEY)?;
        let dtype = self.dtype()?;
        self.key(MMAP_KEY)?;
        // Whether the reader may memory-map the run. joblib writes it true for
        // a seekable file and false for one it is compressing into; both are
        // read, and it says nothing about where the numbers are.
        self.read_flag()?;
        let alignment = self.alignment()?;
        self.atoms(&[b"u"])?;
        self.exact(b"b")?;
        let wrapper_ends = self.at;

        // The frame the wrapper went in was committed before these bytes, so
        // a frame boundary falls exactly here.
        self.gate()?;
        let pad_at = self.at;
        if let Some(align) = alignment {
            self.padding(align)?;
        }
        let data_at = self.at;
        let count = count_of(&dimensions)?;
        let len = usize::try_from(count.checked_mul(dtype.width()?)?).ok()?;
        self.take(len)?;
        // A new frame begins after the run, unless what is left of the file is
        // too short to frame. Below protocol 4 there are no frames at all.
        if self.framing != Framing::Unframed {
            self.framing = Framing::Tail(self.at);
        }
        self.gate()?;

        let payload = self.fits(&dtype, &dimensions, len)?;
        self.raws.push(Raw { pad_at, data_at, end: self.at });
        self.finish_call("wrapper", start, wrapper_ends);
        self.arrays += 1;
        self.wrappers += 1;
        self.reads(data_at, payload, Storage::Raw);
        Some(self.span(
            start,
            Kind::Array { at: data_at, len, dtype, dimensions, fortran_order, storage: Storage::Raw },
        ))
    }

    /// One of the wrapper's attribute names, spelled here or named where an
    /// earlier wrapper spelled it. A file holding two arrays writes the six
    /// words once and refers to them after that.
    fn key(&mut self, name: &str) -> Option<()> {
        self.word_or_reference(name).map(|_| ())
    }

    /// How far the numbers are aligned, which is the last thing the state
    /// says, or nothing for a file that leaves the key out.
    ///
    /// joblib 1.1 and older had no such attribute and wrote no padding byte
    /// either, so the two go together: the run starts at the BUILD either way,
    /// and what the key says is whether there is a byte in front of it. No
    /// file in the corpus is one of those; the variant is named so that a file
    /// from before 1.2 is a form that does not fit rather than a run read at
    /// the wrong offset.
    fn alignment(&mut self) -> Option<Option<u64>> {
        let here = self.save();
        if self.key(ALIGNMENT_KEY).is_none() {
            self.restore(here);
            return Some(None);
        }
        let Kind::Int { value, .. } = self.integer()?.kind else { return None };
        let align = u64::try_from(value).ok()?;
        // A power of two, and one a single byte of padding can reach.
        if align == 0 || align > MOST_ALIGNMENT || !align.is_power_of_two() {
            return None;
        }
        Some(Some(align))
    }

    /// The padding in front of the numbers: one byte saying how many bytes
    /// follow, and that many of them.
    ///
    /// The count is not read and believed. joblib works it out from where the
    /// byte itself sits, so the file has to say exactly what that formula
    /// gives, which is never nought: a run already on the alignment is pushed
    /// a whole alignment further.
    fn padding(&mut self, align: u64) -> Option<()> {
        let at = self.at as u64;
        let declared = u64::from(self.byte()?);
        if declared != align - ((at + 1) % align) {
            return None;
        }
        let run = self.take(usize::try_from(declared).ok()?)?;
        run.iter().all(|b| *b == PAD).then_some(())
    }
}

/// How many values a shape holds. Zero when any dimension is.
fn count_of(dimensions: &[u64]) -> Option<u64> {
    match dimensions.contains(&0) {
        true => Some(0),
        false => dimensions.iter().try_fold(1u64, |n, dim| n.checked_mul(*dim)),
    }
}
