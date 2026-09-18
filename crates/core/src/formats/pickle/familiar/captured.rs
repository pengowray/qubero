//! What a match is made of: the values a form captured, how each is written,
//! and the words the reading uses for them.
//!
//! Apart from [`mod`](super) because that file is how a match is *made*, and
//! because a family of values added to the recogniser declares itself here:
//! one arm of [`Kind`], one name in [`Shape`], and the reading follows.

use super::OBJECT_DTYPE;

/// Captured values preserve Python distinctions and reference payload bytes.
///
/// `at`/`len` are the whole production: the first opcode byte of the value
/// through the last byte that belongs to it, memo marks included. What a leaf
/// is *worth* sits inside that, and [`Kind`] says where.
#[derive(Debug, PartialEq)]
pub struct Value {
    pub at: usize,
    pub len: usize,
    pub kind: Kind,
}

#[derive(Debug, PartialEq)]
pub enum Kind {
    None,
    Bool(bool),
    /// The number, and the operand bytes it was written in. `None` has no
    /// operand and neither has a bool: those two are the opcode byte itself,
    /// which is what their span already covers.
    Int {
        value: i128,
        at: usize,
        len: usize,
    },
    /// A whole number too wide for the integer type above, which is a `LONG1`
    /// of more than sixteen bytes or a line of more digits than one holds.
    /// `digits` is what it comes to, worked out once as the form reads the run;
    /// `at`/`len` are the run, and `spelled` says that run is those digits
    /// rather than the two's-complement bytes, which is what protocols 0 and 1
    /// write. Nothing here is read back out of the file: no integer type is
    /// that wide, so the number is a node with its run under it.
    Wide {
        at: usize,
        len: usize,
        digits: String,
        spelled: bool,
    },
    Float {
        value: f64,
        at: usize,
        len: usize,
    },
    Text {
        at: usize,
        len: usize,
    },
    /// A text or a byte string the file wrote as a spelling of itself rather
    /// than as itself, which is a protocol 0 line with an escape in it or a
    /// character above 0x7f. `at`/`len` are the line's run; what it spells is
    /// decoded once as the form reads it and kept beside the match, and
    /// [`Match::decoded`] hands it back. `bytes` says it spells a byte string,
    /// which a Python 2 `str` holding bytes that are not UTF-8 does.
    Spelled {
        at: usize,
        len: usize,
        bytes: bool,
    },
    Bytes {
        at: usize,
        len: usize,
    },
    /// A value the file wrote once and named again. The production is the
    /// BINGET alone, which is what the value's own span covers; what it names
    /// sits somewhere earlier in the file, and [`Names`] says where.
    Ref(Names),
    List(Vec<Value>),
    Tuple(Vec<Value>),
    Set(Vec<Value>),
    FrozenSet(Vec<Value>),
    Dict(Vec<(Value, Value)>),
    Array {
        at: usize,
        len: usize,
        dtype: Dtype,
        dimensions: Vec<u64>,
        fortran_order: bool,
        /// How the numbers at `at`/`len` are written, which protocol 2 leaves
        /// no way of writing directly.
        storage: Storage,
    },
    /// A class or a callable the file named by STACK_GLOBAL, from a module a
    /// form allows. `parts` is the module word and the name word when the file
    /// spelled them here, and nothing when it named the slot they are in.
    Class {
        path: String,
        parts: Vec<Value>,
    },
    /// An object made by `EMPTY_TUPLE NEWOBJ` and given its attributes by
    /// `BUILD` of a dictionary. The state is missing only for an object the
    /// file wrote no BUILD for, which is one that had no attributes to set.
    Instance {
        class: Box<Value>,
        state: Option<Box<Value>>,
    },
    /// An array whose values are objects rather than numbers, which is the
    /// `O8` dtype. Its data is not a buffer: the values are pickled after the
    /// array and handed to it as a list.
    Objects {
        dimensions: Vec<u64>,
        fortran_order: bool,
        items: Vec<Value>,
    },
    /// A NumPy dtype standing on its own rather than describing an array,
    /// which is what pandas hands a datetime column beside its numbers.
    DType(Dtype),
    /// What a call made: one of a form's enumerated callables, or one of the
    /// builtin types a pickle has to write as a call rather than as a literal.
    ///
    /// `names` names the arguments, in the order they are written. `callable`
    /// is the thing that was called, for the calls whose callable is a value
    /// of the file, and nothing for the ones a form matched inside a fixed run
    /// and folded away: a `slice` says it is a slice without a row saying so
    /// again. `state` is what a BUILD after the call handed the result.
    Made {
        what: Shape,
        names: &'static [&'static str],
        callable: Option<Box<Value>>,
        items: Vec<Value>,
        state: Option<Box<Value>>,
    },
}

/// How a run of bytes a form captured is written in the file.
///
/// Protocol 3 gave a pickle an opcode for a byte string. Protocol 2 has none,
/// so a pickler there hands the bytes to `_codecs.encode` as the text they
/// spell in latin-1, and that text is written UTF-8: a byte under 0x80 is
/// itself and every byte above it is two. So the run in the file is not the
/// bytes it stands for, and what is read out of it has to say which it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Storage {
    /// The bytes themselves, which is every protocol from 3 up.
    Raw,
    /// The text they spell in latin-1, which is protocol 2.
    Latin1,
    /// The same, written as a protocol 0 line: the latin-1 text escaped the
    /// way that protocol escapes one. Two layers, so the bytes are worked out
    /// as the form reads the run and kept beside the match rather than read
    /// back out of the file.
    Escaped,
}

impl Storage {
    /// How many bytes this run stands for. A latin-1 run is one byte a
    /// character, and a character is every byte of the run that is not the
    /// continuation of the one before it.
    pub fn decoded(self, bytes: &[u8]) -> usize {
        match self {
            Storage::Raw | Storage::Escaped => bytes.len(),
            Storage::Latin1 => bytes.iter().filter(|b| **b & 0xc0 != 0x80).count(),
        }
    }

    /// The bytes this run stands for, for a run that is not them. One pass,
    /// one byte a character, and nothing for a run that already is the bytes.
    ///
    /// Every character is under 0x100, which the production checked when it
    /// read the run, so each is one byte and the result is as long as
    /// [`Storage::decoded`] said.
    pub fn read(self, bytes: &[u8]) -> Option<Vec<u8>> {
        match self {
            Storage::Raw | Storage::Escaped => None,
            Storage::Latin1 => {
                let text = std::str::from_utf8(bytes).ok()?;
                text.chars().map(|c| u8::try_from(u32::from(c)).ok()).collect()
            }
        }
    }
}

/// What one value of an array is.
///
/// NumPy describes both kinds the same way, as a `dtype` built by a call and
/// finished by a BUILD, and the state of that BUILD says which this is.
#[derive(Debug, Clone, PartialEq)]
pub enum Dtype {
    /// One number a value, spelled the way NumPy spells it: `<f4`.
    Plain(String),
    /// A record a value, which is what a structured array holds and what
    /// scikit-learn writes its tree of nodes as. The columns are in the order
    /// NumPy names them, and `width` is the whole record, padding included.
    Record { columns: Vec<Column>, width: u64 },
    /// One pickled object a value, which is NumPy's `O8`. There is nothing to
    /// measure: the values are written after the array rather than in it.
    Objects,
    /// A count of a unit of time from 1970, which is NumPy's `M8`. `spelling`
    /// is `<M8[ns]` and `unit` the `ns` in it, which is the whole of what the
    /// count means and is carried in the dtype's state rather than in its
    /// letters.
    Datetime { spelling: String, unit: String },
}

/// One column of a structured dtype: what NumPy calls it, what one of them
/// is, and how far into a record it sits.
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub name: String,
    pub dtype: String,
    pub at: u64,
}

impl Dtype {
    /// How wide one value is, which is what a shape is checked against.
    pub fn width(&self) -> Option<u64> {
        match self {
            Dtype::Plain(spelling) => Some(crate::formats::pickle::shapes::dtype(spelling)?.1),
            Dtype::Record { width, .. } => Some(*width),
            Dtype::Objects => None,
            Dtype::Datetime { spelling, .. } => Some(crate::formats::pickle::shapes::dtype(spelling)?.1),
        }
    }

    /// The spelling the `dtype` row shows.
    pub fn name(&self) -> String {
        match self {
            Dtype::Plain(spelling) => spelling.clone(),
            Dtype::Record { width, columns } => {
                let named: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
                format!("V{width} ({})", named.join(", "))
            }
            Dtype::Objects => OBJECT_DTYPE.to_string(),
            Dtype::Datetime { unit, .. } => format!("datetime64[{unit}]"),
        }
    }
}

/// What a `BINGET` names, which is what the `refers to` row beside it says.
///
/// A string or a byte string is short enough to show, so the row shows it. A
/// container is not: the reader is sent to the bytes the file wrote it in
/// rather than shown a copy of them, and a container a reference sits inside
/// has not been finished yet, which is what a list holding itself is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Names {
    Text { at: usize, len: usize },
    Bytes { at: usize, len: usize },
    Made {
        what: Shape,
        at: usize,
        /// Whether Python could hash the thing, which is what says a
        /// reference to it could stand where a dictionary key belongs.
        hashable: bool,
    },
}

/// What a `slice` and a `range` are written as, in the order Python writes
/// them. A `complex` is written as its two halves.
pub(super) const BOUNDS: &[&str] = &["start", "stop", "step"];
pub(super) const HALVES: &[&str] = &["real", "imaginary"];
/// A `bytearray` is written as the one byte string it was made from, named
/// for what it holds the way an array's numbers are.
pub(super) const CONTENT: &[&str] = &["bytes"];
/// A byte string below protocol 3 is written as the text it spells and the
/// encoding that turns the one back into the other.
pub(super) const SPELLED: &[&str] = &["text", "encoding"];

/// A named operand inside a run of fixed instructions: the module and class
/// names a form matched exactly, and the letters that spell a dtype. The
/// instructions around it are the grammar's; these bytes are what varies.
///
/// A name the file wrote once and referred to again is spelled only where it
/// was written, so the reference is named as the instruction it is and no
/// `Said` points outside the run it belongs to.
#[derive(Debug, PartialEq)]
pub struct Said {
    pub name: &'static str,
    pub at: usize,
    pub len: usize,
}

/// A run of instructions a form matched as one act. Rebuilding a NumPy array
/// is a couple of dozen opcodes and one thing happening, and a reader wants
/// the thing, with the names and letters it was given.
#[derive(Debug, PartialEq)]
pub struct Call {
    pub name: &'static str,
    pub at: usize,
    pub len: usize,
    pub says: Vec<Said>,
}

/// One instruction of a matched file: where it is, where it ends, and what
/// `pickletools` calls it.
///
/// A form fixes its instructions, so every byte a match consumed that is not
/// a value is one of these. That is what lets the template name them instead
/// of leaving them as bytes nothing accounts for.
#[derive(Debug, PartialEq)]
pub struct Instr {
    pub at: usize,
    pub end: usize,
    pub name: &'static str,
}

/// Which of CPython's two picklers wrote the file, as far as its bytes say.
///
/// `_pickle` is the C one, which `pickle.dump` uses wherever it imports;
/// `pickle.py` is the pure Python one beside it, and the only one PyPy 3 has.
/// They agree everywhere but the tail of a long container and the memo mark
/// after a bytearray, so most files say nothing either way. A file that shows
/// one of them at one batch edge and the other at another was written by
/// neither, and is a non-match.
///
/// Python 2 had a third, `cPickle`, which is a different program from
/// Python 3's `_pickle` and is known by the slot it starts numbering the memo
/// at. PyPy 2.7 ships a Python copy of it under the same name, and the two
/// agree on the numbering, so a file says `cPickle` and not which of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pickler {
    /// Nothing in the file tells them apart, which is most files.
    Undetermined,
    C,
    Python,
    CPickle,
}

impl Pickler {
    /// What the `pickler` row says.
    pub fn name(self) -> &'static str {
        match self {
            Pickler::C => "_pickle (CPython's C pickler)",
            // PyPy 3 has this pickler and no other. PyPy 2.7 is the one that
            // does not fit: it ships a Python copy of `cPickle` beside this,
            // and that copy is read as `cPickle`, which is what it is.
            Pickler::Python => "pickle.py (the pure Python pickler, the only one PyPy 3 has)",
            Pickler::CPickle => "cPickle (Python 2's C pickler, or PyPy 2.7's Python copy of it)",
            Pickler::Undetermined => "_pickle or pickle.py (they write this data identically)",
        }
    }
}

/// What a node of a recognised tree holds, for the nodes that hold others.
/// A leaf is read as the type its bytes are and never carries one of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// The whole file, read as the one object it holds.
    Doc,
    /// What matched, and the protocol envelope the match was made through:
    /// the bytes before the object, which say nothing about the object.
    Header,
    Dict,
    /// A whole number no integer type here is wide enough to read, shown as
    /// the digits it comes to with its run beneath it.
    Integer,
    /// One key and one value of a dictionary, kept as the pair it is written
    /// as: two keys spelled alike are two entries, not one.
    Entry,
    List,
    Tuple,
    Set,
    /// A value the file named instead of writing again, which is a BINGET and
    /// the note saying what it names.
    Ref,
    /// A typed run of numbers, with the dtype, shape and storage order that
    /// say how to read it.
    Array,
    /// A run of instructions the form matched as one act. See [`Call`].
    Call,
    Slice,
    Range,
    Complex,
    FrozenSet,
    ByteArray,
    /// A byte string protocol 2 had to write as text and a call, with the
    /// text and the encoding it was handed inside it.
    Bytes,
    /// A class or a callable the file named, with the module and the name it
    /// was spelled by inside it.
    Class,
    /// An object of a named class, with the attributes a BUILD gave it.
    Object,
    /// One run of a pandas frame's columns, with where in the frame they sit.
    Block,
    /// A NumPy dtype on its own: how one value of an array is read.
    DType,
}

impl Shape {
    pub fn name(self) -> &'static str {
        match self {
            Shape::Doc => "pickle",
            Shape::Header => "header",
            Shape::Dict => "dict",
            Shape::Integer => "integer",
            Shape::Entry => "entry",
            Shape::List => "list",
            Shape::Tuple => "tuple",
            Shape::Set => "set",
            Shape::Ref => "reference",
            Shape::Array => "array",
            Shape::Call => "call",
            Shape::Slice => "slice",
            Shape::Range => "range",
            Shape::Complex => "complex",
            Shape::FrozenSet => "frozenset",
            Shape::ByteArray => "bytearray",
            Shape::Bytes => "bytes",
            Shape::Class => "class",
            Shape::Object => "object",
            Shape::Block => "block",
            Shape::DType => "dtype",
        }
    }
}
