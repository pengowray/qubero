//! Strict whole-document grammars, independent of the symbolic pickle machine.
//! Only declared opcode sequences are consumed. Captures are committed at EOF.
//!
//! This file holds what a match is made of and which forms there are. The
//! reading itself is split up: [`cursor`] for bytes and frames, [`memo`] for
//! the slots a file names things in, and [`basic`], [`numpy`] and [`builtins`]
//! for the value productions each form allows.

use std::sync::Arc;

use super::{known::Payload, machine};
use crate::template::{Deduce, Deduced, Deducer};

mod basic;
mod builtins;
mod codecs;
mod cursor;
mod dtype;
mod forms;
mod memo;
mod numpy;
mod object;
#[cfg(test)]
mod tests;

use cursor::{Cursor, Framing};
use forms::{forms, Allow, Family};
use memo::Memo;

pub const MESSAGE: &str = "Matched a Familiar Pickle Form: bypassed Pickle stack machine decoding.";
/// The label the other template goes by on screen, which the STOP row names so
/// that a reader looking at the program knows where the data went. Spelled the
/// same here as in `web/src/filetype.ts`.
pub const FAMILIAR_LABEL: &str = "Python pickle (familiar form)";
/// The work a whole recognition may cost, in opcodes, shared across every
/// form tried; and the tallest the stack may grow, which is the same number
/// because every opcode pushes at most one thing.
///
/// A million rather than the hundred thousand the earlier slices used. The
/// pass is linear now: the only thing tried and rewound is a NumPy or
/// builtins call at an object's first opcode, which fails on its first word
/// when it is not one. So the budget can be what a real file needs, and a
/// list of a few hundred thousand numbers is a real file. The cost of the
/// number is memory: a captured tree is a value per opcode.
const MAX_VALUES: usize = 1_000_000;
const MAX_DEPTH: usize = 64;
/// The most memo slots a form will follow. A slot is bound when the file
/// writes one, so this bounds the table rather than describing any file.
const MAX_MEMO: usize = 1_000_000;
/// The most entries one batch of a container carries. CPython writes a
/// thousand at a time and starts another batch after that, so a container of
/// any length is a run of batches and only the last of them is short.
const MAX_BATCH: usize = 1000;
/// No opcode is zero, so a length width named with this is never the one a
/// counted run is read at. It stands where [`Cursor::counted`] wants a width
/// that the opcode in hand does not have.
const NO_OPCODE: u8 = 0;
/// How NumPy spells the dtype whose values are pickled objects.
const OBJECT_DTYPE: &str = "|O8";
/// The most dimensions a shape may declare, which is NumPy's own limit.
const MAX_DIMENSIONS: usize = 32;
/// A payload this size or larger is written between frames rather than inside
/// one: CPython's framer commits the frame it is filling, writes the opcode
/// and the bytes straight out, and starts a new frame after them.
const BIG_PAYLOAD: usize = 1 << 16;
/// A frame's contents shorter than this are written without a FRAME header,
/// so a run of unframed bytes this short may end the file.
const MIN_FRAME: usize = 4;
/// The longest newline-terminated run a form reads as one. Protocols 2 and 3
/// write a module path, a callable's name and an integer too wide for BININT
/// that way, and none of those is anywhere near this long.
const MAX_LINE: usize = 512;
/// The encoding a pickler below protocol 3 hands a byte string to `_codecs`
/// under, which is the one that maps every byte to the character of the same
/// number.
const LATIN1: &str = "latin1";

/// What the STOP row of the opcode listing says about a match: the contract's
/// sentence, the form that matched, and where the decoded data is.
pub fn stop_message(form: &str) -> String {
    format!("Matched a Familiar Pickle Form ({form}): bypassed Pickle stack machine decoding. Switch to the \"{FAMILIAR_LABEL}\" template to see the data.")
}

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
    Float {
        value: f64,
        at: usize,
        len: usize,
    },
    Text {
        at: usize,
        len: usize,
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
    /// One of the builtin types a pickle has to write as a call rather than
    /// as a literal. `names` names its parts, in the order they were written.
    Object {
        what: Shape,
        names: &'static [&'static str],
        items: Vec<Value>,
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
    /// What a REDUCE of one of a form's enumerated callables made. `names`
    /// names the arguments, in the order the library writes them, and `state`
    /// is what a BUILD after the call handed the result.
    Made {
        what: Shape,
        names: &'static [&'static str],
        callable: Box<Value>,
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
}

impl Storage {
    /// How many bytes this run stands for. A latin-1 run is one byte a
    /// character, and a character is every byte of the run that is not the
    /// continuation of the one before it.
    pub fn decoded(self, bytes: &[u8]) -> usize {
        match self {
            Storage::Raw => bytes.len(),
            Storage::Latin1 => bytes.iter().filter(|b| **b & 0xc0 != 0x80).count(),
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
const BOUNDS: &[&str] = &["start", "stop", "step"];
const HALVES: &[&str] = &["real", "imaginary"];
/// A `bytearray` is written as the one byte string it was made from, named
/// for what it holds the way an array's numbers are.
const CONTENT: &[&str] = &["bytes"];
/// A byte string below protocol 3 is written as the text it spells and the
/// encoding that turns the one back into the other.
const SPELLED: &[&str] = &["text", "encoding"];

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
/// `pickle.py` is the pure Python one beside it, and the only one PyPy has.
/// They agree everywhere but the tail of a long container and the memo mark
/// after a bytearray, so most files say nothing either way. A file that shows
/// one of them at one batch edge and the other at another was written by
/// neither, and is a non-match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pickler {
    /// Nothing in the file tells the two apart, which is most files.
    Undetermined,
    C,
    Python,
}

impl Pickler {
    /// What the `pickler` row says.
    pub fn name(self) -> &'static str {
        match self {
            Pickler::C => "_pickle (CPython's C pickler)",
            Pickler::Python => "pickle.py (the pure Python pickler, the only one PyPy has)",
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

#[derive(Debug)]
pub struct Match {
    pub form: &'static str,
    /// Which pickler the file's spellings show, where they show one.
    pub pickler: Pickler,
    pub value: Value,
    /// Where the object starts, which is one past the protocol byte and past
    /// the frame header when there is one.
    pub body: usize,
    /// The runs of instructions this form matched as acts of their own, one
    /// per array or scalar it rebuilt.
    pub calls: Vec<Call>,
    /// Every instruction in the file, in order, so that the bytes no value
    /// covers can be named rather than left over.
    pub ops: Vec<Instr>,
    stop: usize,
    payloads: Vec<(usize, Payload)>,
}

impl Deduced for Match {
    fn int(&self, what: Deduce, at: u64) -> Option<i128> {
        let payload = self.payloads.iter().find(|(offset, _)| *offset as u64 == at).map(|(_, p)| p)?;
        match what {
            Deduce::PayloadShape => Some(payload.shape as i128),
            Deduce::PayloadCount => Some(payload.count as i128),
            Deduce::Builds => None,
        }
    }

    fn text(&self, what: Deduce, at: u64) -> Option<String> {
        (what == Deduce::Builds && at == (self.stop + 1) as u64).then(|| stop_message(self.form))
    }
}

/// FPF wins before the symbolic decoder is invoked. Non-matches retain the
/// existing inspection annotations; those never carry the FPF success label.
#[derive(Debug)]
pub struct Program;

impl Deducer for Program {
    fn run(&self, bytes: &[u8]) -> Arc<dyn Deduced> {
        match recognise(bytes) {
            Some(found) => Arc::new(found),
            None => machine::Program.run(bytes),
        }
    }
}

/// Initial envelope: protocol 4/5, unframed or framed the way CPython frames.
/// Each form is tried in turn over the same bytes, under one shared budget.
pub fn recognise(bytes: &[u8]) -> Option<Match> {
    let mut left = MAX_VALUES;
    let mut reached = 0;
    forms().into_iter().find_map(|(form, allow)| attempt(bytes, form, allow, &mut left, &mut reached))
}

/// How far into the file the forms read before none of them could go on.
///
/// This is not part of matching, and it says nothing about the file: a
/// non-match is a non-match. It is for whoever is writing the next production
/// and wants the byte the reading stopped at instead of bisecting for it.
/// `cargo run -p qubero-core --example pickle_forms` prints it beside a file
/// no form matched.
pub fn furthest(bytes: &[u8]) -> usize {
    let mut left = MAX_VALUES;
    let mut reached = 0;
    for (form, allow) in forms() {
        attempt(bytes, form, allow, &mut left, &mut reached);
    }
    reached
}

fn attempt(bytes: &[u8], form: &'static str, allow: Allow, left: &mut usize, reached: &mut usize) -> Option<Match> {
    let mut c = Cursor {
        bytes,
        at: 0,
        left: *left,
        proto: 0,
        says: Vec::new(),
        memo: Memo::new(),
        memo_base: None,
        skipped: 0,
        dicts: Pickler::Undetermined,
        framing: Framing::Unframed,
        pickler: Pickler::Undetermined,
        allow,
        calls: Vec::new(),
        payloads: Vec::new(),
        arrays: 0,
        objects: 0,
        instances: 0,
        furthest: 0,
    };
    let found = c.whole(form);
    *left = c.left;
    *reached = (*reached).max(c.furthest);
    found
}

/// Every instruction of a file a form has just matched.
///
/// The same walk the listing does, over bytes already known to be a whole
/// pickle: it reaches the STOP and stops there, so what comes back covers the
/// file exactly. Names are `pickletools`' own.
fn instructions(bytes: &[u8]) -> Vec<Instr> {
    super::opcodes(bytes)
        .iter()
        .map(|op| Instr {
            at: op.at as usize,
            end: op.end as usize,
            name: super::opcode_name(op.code),
        })
        .collect()
}

impl<'a> Cursor<'a> {
    fn whole(&mut self, form: &'static str) -> Option<Match> {
        self.exact(&[0x80])?;
        self.proto = match self.byte()? {
            proto if self.allow.protocols.contains(&proto) => proto,
            _ => return None,
        };
        // Framing arrived with protocol 4. Below it a file is one run of
        // instructions and a FRAME opcode is not one of them.
        if self.proto >= 4 && self.peek() == Some(0x95) {
            self.frame_header()?;
        }
        let body = self.at;
        let value = self.object()?;
        self.exact(b".")?;
        if self.at != self.bytes.len() {
            return None;
        }
        // The last frame ends where the STOP does. The one exception is a file
        // whose large payload left fewer than MIN_FRAME bytes to write after
        // it, since CPython writes those with no FRAME header in front. A
        // frame that simply stopped early is a non-match.
        match self.framing {
            Framing::Unframed => {}
            Framing::Inside(end) | Framing::Full(end) if end == self.at => {}
            Framing::Tail(from) if self.at - from < MIN_FRAME => {}
            _ => return None,
        }
        let plain = self.arrays == 0 && self.objects == 0 && self.instances == 0;
        let needed = match self.allow.family {
            Family::Basic => plain,
            Family::Numpy => self.arrays > 0 && self.objects == 0 && self.instances == 0,
            Family::Builtins => self.arrays == 0 && self.objects > 0 && self.instances == 0,
            // A library form is the one the file's classes came from, and it
            // has to have read at least one of them.
            Family::Library => self.instances > 0,
        };
        if !needed {
            return None;
        }
        Some(Match {
            form,
            pickler: self.pickler,
            value,
            body,
            calls: std::mem::take(&mut self.calls),
            ops: instructions(self.bytes),
            stop: self.at - 1,
            payloads: std::mem::take(&mut self.payloads),
        })
    }
}
