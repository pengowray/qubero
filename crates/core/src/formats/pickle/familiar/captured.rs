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
    ///
    /// `spelled` says the run at `at`/`len` is the decimal digits rather than
    /// the number packed into bytes, which is what protocols 0 and 1 write and
    /// what no integer type reads. Such a number is a node, the way one too
    /// wide for any type is, with the line beneath it.
    Int {
        value: i128,
        at: usize,
        len: usize,
        spelled: bool,
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
    /// `spelled` says the run is the digits Python's `repr` wrote on a
    /// protocol 0 line rather than the eight bytes of the number.
    Float {
        value: f64,
        at: usize,
        len: usize,
        spelled: bool,
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
        /// Where the whole pickle of its own that holds this array starts,
        /// which is what `joblib.dump` writes in place of a run of numbers
        /// when the array has no numbers to write. Nothing for an array
        /// written inside the one stream, which is every other file.
        nested: Option<usize>,
    },
    /// A NumPy dtype standing on its own rather than describing an array,
    /// which is what pandas hands a datetime column beside its numbers.
    DType(Dtype),
    /// A torch tensor: everything `_rebuild_tensor_v2` was called with, and
    /// nothing of the numbers, which are in another entry of the archive or
    /// further down the same file. See [`torch`](super::torch).
    Tensor(Tensor),
    /// What a call made: one of a form's enumerated callables, or one of the
    /// builtin types a pickle has to write as a call rather than as a literal.
    ///
    /// `names` names the arguments, in the order they are written. `callable`
    /// is the thing that was called, for the calls whose callable is a value
    /// of the file, and nothing for the ones a form matched inside a fixed run
    /// and folded away: a `slice` says it is a slice without a row saying so
    /// again. `state` is what the result holds beyond its arguments: what a
    /// BUILD after the call handed it, the entries or items the opcodes after
    /// it filled in, which is how an `OrderedDict` and a `deque` are written,
    /// or the mapping a `Counter` is called with, which is the counter.
    ///
    /// `attrs` is what a BUILD handed a call whose result was already full.
    /// `nn.Module.state_dict()` is an `OrderedDict` filled with the weights
    /// and then given a `_metadata` attribute, so the contents and the
    /// attributes are two different things arriving the same way and are kept
    /// apart. Nothing for every other call, where a BUILD's state is the
    /// result's own and goes in `state`.
    Made {
        what: Shape,
        names: &'static [&'static str],
        callable: Option<Box<Value>>,
        items: Vec<Value>,
        state: Option<Box<Value>>,
        attrs: Option<Box<Value>>,
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

/// A torch tensor, as the call that rebuilds it wrote it.
///
/// Every part is read out of the fixed run in [`torch`](super::torch); nothing
/// here is the numbers. A tensor is a window onto a storage: `offset` and
/// `stride` are counted in elements of `dtype`, and `count` is how many
/// elements the storage the persistent id names holds.
#[derive(Debug, PartialEq)]
pub struct Tensor {
    pub dtype: TensorType,
    /// The class the persistent id named, spelled whole: `torch.FloatStorage`.
    /// The dtype above is the plain word for the same thing.
    pub storage_class: &'static str,
    /// The run the storage key sits in, which names the archive entry
    /// `data/<key>` or, in a legacy file, an entry of the key list.
    pub key: (usize, usize),
    /// The run the device the storage was on sits in: `cpu`.
    pub location: (usize, usize),
    /// How many elements the whole storage holds.
    pub count: u64,
    /// How far into the storage this tensor's first element is, in elements.
    pub offset: u64,
    pub size: Vec<u64>,
    pub stride: Vec<u64>,
    pub requires_grad: bool,
    /// Whether `_rebuild_parameter` wrapped it, which is what a module's
    /// weights are.
    pub parameter: bool,
    /// What a quantised tensor's whole numbers stand for. Nothing for every
    /// other tensor, whose numbers are what they say.
    pub quantizer: Option<Quantizer>,
}

/// How a quantised tensor's stored integers map to real numbers:
/// `(stored - zero point) * scale`. Read out of the call and shown beside the
/// numbers rather than applied to them, so what a table shows is what the file
/// holds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quantizer {
    pub scale: f64,
    pub zero_point: i128,
}

/// What one element of a tensor is.
///
/// Torch's own set rather than NumPy's letters: `BFloat16Storage` has no NumPy
/// spelling at all, so a tensor says what it is in its own vocabulary and the
/// reading maps that to a type. [`torch::STORAGES`](super::torch::STORAGES) is
/// the table of which storage class is which.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TensorType {
    Float16,
    BFloat16,
    Float32,
    Float64,
    /// A pair of floats, the real part and then the imaginary one.
    Complex64,
    Complex128,
    /// The two eight-bit floats torch writes, told apart by how many bits of
    /// the byte are the exponent. `fn` is torch's own suffix and says the
    /// format has no infinities.
    Float8E4M3FN,
    Float8E5M2,
    Int8,
    UInt8,
    Int16,
    Int32,
    Int64,
    UInt16,
    UInt32,
    UInt64,
    Bool,
    /// A quantised element, which is a whole number that stands for a real
    /// one through the scale and the zero point the tensor carries.
    QInt8,
    QUInt8,
    QInt32,
}

impl TensorType {
    /// The plain word for it, which is what torch itself prints and what the
    /// `dtype` row and the entry row say.
    pub fn word(self) -> &'static str {
        match self {
            TensorType::Float16 => "float16",
            TensorType::BFloat16 => "bfloat16",
            TensorType::Float32 => "float32",
            TensorType::Float64 => "float64",
            TensorType::Complex64 => "complex64",
            TensorType::Complex128 => "complex128",
            TensorType::Float8E4M3FN => "float8_e4m3fn",
            TensorType::Float8E5M2 => "float8_e5m2",
            TensorType::Int8 => "int8",
            TensorType::UInt8 => "uint8",
            TensorType::Int16 => "int16",
            TensorType::Int32 => "int32",
            TensorType::Int64 => "int64",
            TensorType::UInt16 => "uint16",
            TensorType::UInt32 => "uint32",
            TensorType::UInt64 => "uint64",
            TensorType::Bool => "bool",
            TensorType::QInt8 => "qint8",
            TensorType::QUInt8 => "quint8",
            TensorType::QInt32 => "qint32",
        }
    }

    /// Whether one element is a pair of numbers rather than one, which is what
    /// a complex tensor holds and the only kind a table shows two readings of.
    pub fn complex(self) -> bool {
        matches!(self, TensorType::Complex64 | TensorType::Complex128)
    }

    /// How many bytes one element takes.
    pub fn width(self) -> u64 {
        match self {
            TensorType::Int8 | TensorType::UInt8 | TensorType::Bool => 1,
            TensorType::Float8E4M3FN | TensorType::Float8E5M2 => 1,
            TensorType::QInt8 | TensorType::QUInt8 => 1,
            TensorType::Float16 | TensorType::BFloat16 | TensorType::Int16 | TensorType::UInt16 => 2,
            TensorType::Float32 | TensorType::Int32 | TensorType::UInt32 | TensorType::QInt32 => 4,
            TensorType::Float64 | TensorType::Int64 | TensorType::UInt64 => 8,
            TensorType::Complex64 => 8,
            TensorType::Complex128 => 16,
        }
    }
}

impl Tensor {
    /// How many elements the tensor itself holds, which is its shape
    /// multiplied out. A tensor with no dimensions holds one.
    pub fn values(&self) -> u64 {
        match self.size.contains(&0) {
            true => 0,
            false => self.size.iter().product(),
        }
    }

    /// How far past the storage's start the last element of this view sits, in
    /// elements. What the storage has to be long enough for.
    ///
    /// A view need not be contiguous and need not start at nought: a transpose
    /// keeps the storage and swaps the strides, and two tensors may be two
    /// windows onto one storage. So the reach is the first element plus the
    /// furthest step each axis can take.
    pub fn reach(&self) -> Option<u64> {
        if self.values() == 0 {
            return Some(self.offset);
        }
        let mut last = self.offset;
        for (n, step) in self.size.iter().zip(&self.stride) {
            last = last.checked_add(n.checked_sub(1)?.checked_mul(*step)?)?;
        }
        last.checked_add(1)
    }

    /// Where element `(row, column)` of the table sits in the storage, counted
    /// in elements, for a tensor read as rows along the first axis and columns
    /// along the second.
    ///
    /// A tensor of more than two dimensions is read the way an N-D array's
    /// table is: the last axis is the columns and every axis before it is
    /// unwound into the rows.
    pub fn element(&self, row: u64, column: u64) -> Option<u64> {
        let mut at = self.offset;
        let (last, rest) = self.size.split_last()?;
        // One dimension is one column a row, and no dimensions is one value.
        if self.size.len() == 1 {
            if row >= *last || column > 0 {
                return None;
            }
            return at.checked_add(row.checked_mul(*self.stride.first()?)?);
        }
        if column >= *last {
            return None;
        }
        let mut left = row;
        for axis in (0..rest.len()).rev() {
            let n = rest[axis];
            if n == 0 {
                return None;
            }
            at = at.checked_add((left % n).checked_mul(self.stride[axis])?)?;
            left /= n;
        }
        if left > 0 {
            return None;
        }
        at.checked_add(column.checked_mul(*self.stride.last()?)?)
    }

    /// How many rows the table has: every axis but the last multiplied out.
    pub fn rows(&self) -> u64 {
        match self.size.split_last() {
            None => 0,
            Some((last, [])) => *last,
            Some((_, rest)) => rest.iter().product(),
        }
    }

    /// How many columns it has, which is the last axis, or one for a tensor of
    /// a single dimension.
    pub fn columns(&self) -> u64 {
        match self.size.len() {
            0 => 0,
            1 => 1,
            _ => *self.size.last().unwrap_or(&0),
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
    /// A float a protocol 0 line spells, which has no bytes to read as one.
    Real,
    /// The standard library's own classes, each read as the value it is:
    /// see [`stdlib`](super::stdlib).
    DateTime,
    Date,
    Time,
    TimeDelta,
    TimeZone,
    Decimal,
    Fraction,
    Counter,
    OrderedDict,
    DefaultDict,
    Deque,
    Path,
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
    /// text and the encoding it was handed inside it, or one a protocol 0 line
    /// spells rather than holds.
    Bytes,
    /// Text a protocol 0 line spells rather than holds, which is one with an
    /// escape in it or a character above 0x7f. The node is what the line
    /// spells and the `line` row inside it is the run the file wrote.
    Text,
    /// A class or a callable the file named, with the module and the name it
    /// was spelled by inside it.
    Class,
    /// An object of a named class, with the attributes a BUILD gave it.
    Object,
    /// One run of a pandas frame's columns, with where in the frame they sit.
    Block,
    /// A NumPy dtype on its own: how one value of an array is read.
    DType,
    /// A torch tensor: a window onto a storage kept somewhere else.
    Tensor,
    /// `torch.Size`, which is the shape of a tensor written as a value of its
    /// own. A tuple subclass, so it is a call of the class over a tuple.
    Size,
    /// A sparse tensor, which is two tensors and the shape they stand for:
    /// where the values are and what they are.
    SparseTensor,
    /// A whole pickle written inside another one, with a protocol, a memo and
    /// a STOP of its own. `joblib.dump` writes one where an array's numbers
    /// would go when the array holds pickled objects rather than numbers.
    Nested,
}

impl Shape {
    pub fn name(self) -> &'static str {
        match self {
            Shape::Doc => "pickle",
            Shape::Header => "header",
            Shape::Dict => "dict",
            Shape::Integer => "integer",
            Shape::Real => "float",
            // Python's own names for its own classes, which is what a reader
            // of a pickle is comparing against.
            Shape::DateTime => "datetime",
            Shape::Date => "date",
            Shape::Time => "time",
            Shape::TimeDelta => "timedelta",
            Shape::TimeZone => "timezone",
            Shape::Decimal => "Decimal",
            Shape::Fraction => "Fraction",
            Shape::Counter => "Counter",
            Shape::OrderedDict => "OrderedDict",
            Shape::DefaultDict => "defaultdict",
            Shape::Deque => "deque",
            Shape::Path => "path",
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
            Shape::Text => "text",
            Shape::Class => "class",
            Shape::Object => "object",
            Shape::Block => "block",
            Shape::DType => "dtype",
            Shape::Tensor => "tensor",
            Shape::Size => "Size",
            Shape::SparseTensor => "sparse tensor",
            Shape::Nested => "nested pickle",
        }
    }
}
