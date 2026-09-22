//! What a form matched, as runs of instructions rather than as values: the
//! names inside a fixed run, the run itself, one opcode of it, and the word
//! each kind of node goes by.
//!
//! Apart from [`captured`](super::captured) because that half is the values.
//! A value says which of these it is through [`Shape`], and nothing here knows
//! what a value holds, so a family of values added to the recogniser adds one
//! name here and one arm there.

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
    /// `time.struct_time` and `os.stat_result`, the two structseq classes the
    /// standard library writes: a fixed run of whole numbers, and a dictionary
    /// of the fields that run does not hold.
    StructTime,
    StatResult,
    /// One of the builtin exception classes, rebuilt from the arguments it was
    /// raised with. See [`exceptions`](super::exceptions).
    Exception,
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
    /// `numpy.matrix`, which is an array held to two dimensions. Everything
    /// else about it is an array's, down to the reconstructor that rebuilt it.
    Matrix,
    /// `numpy.memmap`, which is an array a reader may keep in a file rather
    /// than in memory. What it was pickled with is the numbers themselves.
    MemMap,
    /// `numpy.recarray`, which is a structured array whose columns are also
    /// attributes. Everything else about it is an array's, and its dtype is
    /// the one dtype written as a class rather than as letters.
    RecArray,
    /// `numpy.ma.MaskedArray`, which is an array, a mask of which of its
    /// entries count, and the value a masked entry reads as.
    MaskedArray,
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
    /// An array whose numbers are in a `.npy` file beside the pickle, which is
    /// how `joblib.dump` wrote one before 0.10. All the pickle holds is the
    /// name of that file.
    ArrayFile,
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
            Shape::StructTime => "struct_time",
            Shape::StatResult => "stat_result",
            Shape::Exception => "exception",
            Shape::Entry => "entry",
            Shape::List => "list",
            Shape::Tuple => "tuple",
            Shape::Set => "set",
            Shape::Ref => "reference",
            Shape::Array => "array",
            // NumPy's own names for its own array classes.
            Shape::Matrix => "matrix",
            Shape::MemMap => "memmap",
            Shape::RecArray => "recarray",
            Shape::MaskedArray => "masked array",
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
            Shape::ArrayFile => "array in another file",
        }
    }
}
