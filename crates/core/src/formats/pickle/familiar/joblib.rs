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
use super::forms::Wrapped;
use super::memo::{Bound, Memo};
use super::{Dtype, Kind, Pickler, Shape, Storage, Value, JOBLIB_MODULE as MODULE};

/// The class joblib names for every array it writes, in the module
/// [`JOBLIB_MODULE`](super::JOBLIB_MODULE).
const WRAPPER: &str = "NumpyArrayWrapper";
/// The class joblib 0.9 and older named instead, whose array is not in this
/// file at all: it is a `.npy` file beside the pickle, one per array, and the
/// wrapper's whole job is to say which. `ZNDArrayWrapper` is the same class
/// for a compressed file and is not read: joblib wrote those inside its own
/// `ZF` container, which nothing here opens.
const FILE_WRAPPER: &str = "NDArrayWrapper";
/// The name of that file, which is the only place the numbers are.
const FILENAME_KEY: &str = "filename";
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

/// The protocols a nested pickle may declare. joblib 1.6 writes 5, which is
/// what `write_array` asks `pickle.dump` for; the number is the writer's to
/// choose and the grammar reads whichever it says, so every protocol with a
/// PROTO opener is taken. Below protocol 2 there is no opener and no way to
/// tell where the pickle begins.
const NESTED_PROTOCOLS: &[u8] = &[2, 3, 4, 5];

/// One place in a matched file where the run of opcodes stops and starts
/// again.
///
/// Kept so that the listing can walk the opcodes in the segments between
/// these rather than stop at the first of them, and so that the bytes that
/// are not opcodes have a row of their own. See [`Match::ops`](super::Match).
#[derive(Debug, Clone, Copy)]
pub(super) enum Break {
    /// The padding and an array's numbers, which are not opcodes at all:
    /// where the padding count sits, where the numbers start, and where they
    /// end.
    Numbers { pad_at: usize, data_at: usize, end: usize },
    /// A whole pickle of its own inside the stream. Its bytes are opcodes,
    /// but its STOP is in the middle of the file and a walk ends at a STOP,
    /// so the walk is started again on each side of it.
    Nested { at: usize, end: usize },
}

/// What the outer stream was keeping while a nested pickle is read, and what
/// is put back afterwards. Everything here belongs to one stream: the
/// protocol it declared, the slots it filed, the frames it was written in and
/// which pickler wrote it.
struct Outer {
    proto: u8,
    memo: Memo,
    memo_base: Option<usize>,
    skipped: usize,
    dicts: Pickler,
    batch: Option<usize>,
    framing: Framing,
    pickler: Pickler,
    joblib: Wrapped,
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
        // NEWOBJ arrived at protocol 2, so below it joblib would build the
        // wrapper some other way, and what that is has not been measured. No
        // file in the corpus is one, and the family has no name there.
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
        // An array of objects has no numbers to write. joblib hands the array
        // to `pickle.dump` instead, so what follows the wrapper is a whole
        // pickle rather than a run of bytes, and there is no padding in front
        // of it either: the branch that writes the padding is the other one.
        if dtype == Dtype::Objects {
            self.finish_call("wrapper", start, wrapper_ends);
            return self.nested_array(start, dimensions, fortran_order);
        }
        let pad_at = self.at;
        if let Some(align) = alignment {
            self.padding(align)?;
        }
        let data_at = self.at;
        let count = count_of(&dimensions)?;
        let len = usize::try_from(count.checked_mul(dtype.width()?)?).ok()?;
        self.take(len)?;
        // Where the array ends, which is where the value ends: a new frame
        // begins after the run, and that header belongs to whatever comes
        // next rather than to this. Below protocol 4 there are no frames at
        // all, and the file simply carries on.
        let end = self.at;
        if self.framing != Framing::Unframed {
            self.framing = Framing::Tail(end);
        }
        self.gate()?;

        let payload = self.fits(&dtype, &dimensions, len)?;
        self.breaks.push(Break::Numbers { pad_at, data_at, end });
        self.finish_call("wrapper", start, wrapper_ends);
        self.arrays += 1;
        self.wrappers += 1;
        self.reads(data_at, payload, Storage::Raw);
        Some(Value {
            at: start,
            len: end - start,
            kind: Kind::Array { at: data_at, len, dtype, dimensions, fortran_order, storage: Storage::Raw },
        })
    }

    /// The wrapper joblib 0.9 and older wrote, whose array is in a `.npy`
    /// file beside the pickle.
    ///
    /// Fixed instruction for instruction, the same way the newer wrapper is:
    /// the class named exactly, `cls.__new__(cls)` with no arguments, and a
    /// state dictionary whose three keys are these three in the order
    /// `__init__` sets them. Nothing follows it in the stream, because nothing
    /// of the array is in this file: the pickle carries straight on with
    /// whatever came next.
    ///
    /// What comes out says where the numbers are and no more than that. The
    /// wrapper carries no shape and no dtype, so the file named is the whole
    /// of what this pickle knows about the array, and that file is an ordinary
    /// `.npy` the reader opens on its own.
    pub(super) fn joblib_npy_file(&mut self) -> Option<Value> {
        // NEWOBJ arrived at protocol 2, and joblib 0.9 ran on interpreters
        // that wrote protocol 0 and 1 as well. What it wrote there has not
        // been measured, so the family has no name below protocol 2.
        if self.proto < 2 {
            return None;
        }
        let start = self.at;
        self.global(&[MODULE], FILE_WRAPPER, "wrapper module", "wrapper class")?;
        self.empty_tuple()?;
        self.exact(&[0x81])?;
        self.memoize(Bound::Made { what: Shape::Object, at: start, hashable: false })?;
        let dict_at = self.at;
        self.empty_dict()?;
        self.memoize(Bound::Made { what: Shape::Dict, at: dict_at, hashable: false })?;
        self.atoms(&[b"("])?;

        self.key(FILENAME_KEY)?;
        let filename = self.text()?;
        if let Kind::Text { at, len } = filename.kind {
            self.says("file", at, len);
        }
        self.key(SUBCLASS_KEY)?;
        self.global(&["numpy"], SUBCLASS, "array module", "array class")?;
        self.key(MMAP_KEY)?;
        // Whether the reader may memory-map that file. It says nothing about
        // where the numbers are, and both ways round are read.
        self.read_flag()?;
        self.atoms(&[b"u"])?;
        self.exact(b"b")?;

        self.wrappers += 1;
        self.besides += 1;
        self.finish_call("wrapper", start, self.at);
        Some(Value {
            at: start,
            len: self.at - start,
            kind: Kind::Made { what: Shape::ArrayFile, names: &["file"], callable: None, items: vec![filename], state: None, attrs: None },
        })
    }

    /// The pickle joblib writes where an array of objects would have had its
    /// numbers, and the array it holds.
    ///
    /// The value is the object array the wrapper described, spanning from the
    /// class the file named to the nested pickle's STOP, so a reader of the
    /// tree finds the same array a plain pickle of one holds. The wrapper and
    /// the pickle after it are how it was written and are rows inside it.
    fn nested_array(&mut self, start: usize, dimensions: Vec<u64>, fortran_order: bool) -> Option<Value> {
        let at = self.at;
        let inner = self.nested_pickle()?;
        let end = self.at;
        // Exactly one pickle and nothing else: the value it holds has to be
        // the array, and the array has to be the one the wrapper described.
        // A file whose two halves disagree is one nothing wrote.
        let Kind::Objects { dimensions: held, fortran_order: order, items, .. } = inner.kind else { return None };
        if held != dimensions || order != fortran_order {
            return None;
        }
        // A new frame begins after the nested pickle, and that header belongs
        // to whatever comes next rather than to this. Below protocol 4 there
        // are no frames at all and the stream simply carries on.
        if self.framing != Framing::Unframed {
            self.framing = Framing::Tail(end);
        }
        self.gate()?;
        self.breaks.push(Break::Nested { at, end });
        self.wrappers += 1;
        Some(Value { at: start, len: end - start, kind: Kind::Objects { dimensions, fortran_order, items, nested: Some(at) } })
    }

    /// A whole pickle inside the stream: its own PROTO, its own framing, its
    /// own memo numbered from nought and its own STOP.
    ///
    /// `write_array` hands the array to `pickle.dump` with the same open file,
    /// so what lands in the stream is a pickle written by a second writer that
    /// knows nothing of the first. It is read by the same grammar with
    /// everything the outer stream was keeping put aside and put back after,
    /// so the outer memo is untouched by anything the inner one filed and a
    /// slot number in either names what its own stream wrote.
    fn nested_pickle(&mut self) -> Option<Value> {
        let outer = Outer {
            proto: self.proto,
            memo: std::mem::replace(&mut self.memo, Memo::new()),
            memo_base: self.memo_base.take(),
            skipped: std::mem::take(&mut self.skipped),
            dicts: std::mem::replace(&mut self.dicts, Pickler::Undetermined),
            batch: self.batch.take(),
            framing: std::mem::replace(&mut self.framing, Framing::Unframed),
            // The two picklers are told apart by the tail of a long container,
            // and these are two files: joblib writes the outer stream with
            // `pickle.py` and the inner one is whatever `pickle.dump` imports.
            pickler: std::mem::replace(&mut self.pickler, Pickler::Undetermined),
            // `pickle.dump` writes no wrapper, so a wrapper inside the pickle
            // it wrote is a file joblib did not write.
            joblib: std::mem::replace(&mut self.allow.joblib, Wrapped::Refused),
        };
        let found = self.enveloped(NESTED_PROTOCOLS).map(|(_, value)| value).filter(|_| self.numbering().is_some());
        self.proto = outer.proto;
        self.memo = outer.memo;
        self.memo_base = outer.memo_base;
        self.skipped = outer.skipped;
        self.dicts = outer.dicts;
        self.batch = outer.batch;
        self.framing = outer.framing;
        self.pickler = outer.pickler;
        self.allow.joblib = outer.joblib;
        found
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
