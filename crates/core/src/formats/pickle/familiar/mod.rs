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
mod cursor;
mod memo;
mod numpy;
#[cfg(test)]
mod tests;

use cursor::{Cursor, Framing};
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
/// The most dimensions a shape may declare, which is NumPy's own limit.
const MAX_DIMENSIONS: usize = 32;
/// A payload this size or larger is written between frames rather than inside
/// one: CPython's framer commits the frame it is filling, writes the opcode
/// and the bytes straight out, and starts a new frame after them.
const BIG_PAYLOAD: usize = 1 << 16;
/// A frame's contents shorter than this are written without a FRAME header,
/// so a run of unframed bytes this short may end the file.
const MIN_FRAME: usize = 4;

/// The forms, each a named grammar over the same envelope. They differ in
/// which value productions they allow, and every one of those is enumerated.
const BASIC: &str = "basic-p4-p5-v5";
const NUMPY: &str = "numpy-numeric-array-p4-p5-v5";
const BUILTINS: &str = "builtins-values-p4-p5-v3";

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
    /// A string or a byte string the file wrote once and named again. The
    /// production is the BINGET alone, which is what the value's own span
    /// covers; `at`/`len` are the bytes the thing it names was written in,
    /// somewhere earlier in the file.
    Ref {
        at: usize,
        len: usize,
        /// Whether the slot holds text. The other kind a slot may be named
        /// for is a byte string.
        text: bool,
    },
    List(Vec<Value>),
    Tuple(Vec<Value>),
    Set(Vec<Value>),
    FrozenSet(Vec<Value>),
    Dict(Vec<(Value, Value)>),
    Array {
        at: usize,
        len: usize,
        dtype: String,
        dimensions: Vec<u64>,
        fortran_order: bool,
    },
    /// One of the builtin types a pickle has to write as a call rather than
    /// as a literal. `names` names its parts, in the order they were written.
    Object {
        what: Shape,
        names: &'static [&'static str],
        items: Vec<Value>,
    },
}

/// What a `slice` and a `range` are written as, in the order Python writes
/// them. A `complex` is written as its two halves.
const BOUNDS: &[&str] = &["start", "stop", "step"];
const HALVES: &[&str] = &["real", "imaginary"];
/// A `bytearray` is written as the one byte string it was made from, named
/// for what it holds the way an array's numbers are.
const CONTENT: &[&str] = &["bytes"];

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

/// Which value productions a form allows beyond the basic ones. A form that
/// allows a production also requires the file to use it, so a file holding
/// nothing but basic values is read under the basic form and not another.
#[derive(Debug, Clone, Copy)]
struct Allow {
    numpy: bool,
    builtins: bool,
}

/// Initial envelope: protocol 4/5, unframed or framed the way CPython frames.
/// Each form is tried in turn over the same bytes, under one shared budget.
pub fn recognise(bytes: &[u8]) -> Option<Match> {
    let mut left = MAX_VALUES;
    let forms = [
        (BASIC, Allow { numpy: false, builtins: false }),
        (NUMPY, Allow { numpy: true, builtins: false }),
        (BUILTINS, Allow { numpy: false, builtins: true }),
    ];
    forms.into_iter().find_map(|(form, allow)| attempt(bytes, form, allow, &mut left))
}

/// One form's whole grammar over the whole file.
fn attempt(bytes: &[u8], form: &'static str, allow: Allow, left: &mut usize) -> Option<Match> {
    let mut c = Cursor {
        bytes,
        at: 0,
        left: *left,
        proto: 0,
        says: Vec::new(),
        memo: Memo::new(),
        framing: Framing::Unframed,
        pickler: Pickler::Undetermined,
        allow,
        calls: Vec::new(),
        payloads: Vec::new(),
        arrays: 0,
        objects: 0,
    };
    let found = c.whole(form);
    *left = c.left;
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
            proto @ (4 | 5) => proto,
            _ => return None,
        };
        if self.peek() == Some(0x95) {
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
            Framing::Inside(end) if end == self.at => {}
            Framing::Tail(from) if self.at - from < MIN_FRAME => {}
            _ => return None,
        }
        let needed = match (form, self.arrays, self.objects) {
            (BASIC, 0, 0) => true,
            (NUMPY, arrays, 0) => arrays > 0,
            (BUILTINS, 0, objects) => objects > 0,
            _ => false,
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
