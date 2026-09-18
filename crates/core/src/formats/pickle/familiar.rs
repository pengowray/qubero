//! Strict whole-document grammars, independent of the symbolic pickle machine.
//! Only declared opcode sequences are consumed. Captures are committed at EOF.

use std::sync::Arc;

use super::{known::Payload, machine, shapes};
use crate::template::{Deduce, Deduced, Deducer};

pub const MESSAGE: &str = "Matched a Familiar Pickle Form: bypassed Pickle stack machine decoding.";
/// The label the other template goes by on screen, which the STOP row names so
/// that a reader looking at the program knows where the data went. Spelled the
/// same here as in `web/src/filetype.ts`.
pub const FAMILIAR_LABEL: &str = "Python pickle (familiar form)";
const MAX_VALUES: usize = 100_000;
const MAX_DEPTH: usize = 64;
/// The most memo slots a form will follow. A slot is bound when the file
/// writes one, so this bounds the table rather than describing any file.
const MAX_MEMO: usize = 100_000;
/// The most entries one batch of a container may carry. CPython's batching
/// writes a thousand at a time and starts another batch after that; the forms
/// here take one batch, so a longer one is a non-match rather than a slice.
const MAX_BATCH: usize = 1000;
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
const BASIC: &str = "basic-p4-p5-v3";
const NUMPY: &str = "numpy-numeric-array-p4-p5-v3";
const BUILTINS: &str = "builtins-values-p4-p5-v1";

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
    List(Vec<Value>),
    Tuple(Vec<Value>),
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
/// A frozenset's members are written in no order the file can be trusted for,
/// so they are numbered rather than named.
const MEMBERS: &[&str] = &[];

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
    /// One key and one value of a dictionary, kept as the pair it is written
    /// as: two keys spelled alike are two entries, not one.
    Entry,
    List,
    Tuple,
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

/// What a memo slot holds, as far as a form is prepared to say.
///
/// A slot a form cannot name is [`Bound::Opaque`], and a reference to one is
/// a non-match. Only the things a form spelled out itself can be referred to
/// again, which is what keeps `BINGET` from being an arbitrary stack effect.
#[derive(Debug, Clone, PartialEq)]
enum Bound {
    Opaque,
    Text { at: usize, len: usize },
    Bytes { at: usize, len: usize },
    /// A module and a callable the form named exactly, spelled `module.name`.
    Global(String),
    /// A completed NumPy dtype, as the `<f4` spelling it ends up with. The
    /// slot is written at the REDUCE that makes the dtype and the byte order
    /// arrives at the BUILD just after, so the binding is filled in there.
    Dtype(String),
}

/// Where a frame boundary stands. CPython writes a pickle as a run of frames:
/// a frame is committed when it fills, and a payload of [`BIG_PAYLOAD`] bytes
/// or more is written between frames instead of inside one.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Framing {
    /// The file has no frames at all, which protocol 4 permits.
    Unframed,
    /// Inside a frame ending at this offset.
    Inside(usize),
    /// A frame has ended here and the next has not begun. The next thing is a
    /// FRAME header, or the payload too large to frame that ended it.
    Between(usize),
    /// After a payload written between frames. What follows is a new frame, or
    /// the end of the file within [`MIN_FRAME`] bytes, which is the one run of
    /// unframed bytes CPython writes without a header in front of it.
    Tail(usize),
}

/// Everything a rewind has to put back. The work budget is deliberately not
/// in it: an alternative that failed still cost what it cost.
#[derive(Debug, Clone, Copy)]
struct Save {
    at: usize,
    memo: usize,
    says: usize,
    calls: usize,
    payloads: usize,
    framing: Framing,
    arrays: usize,
    objects: usize,
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
        says: Vec::new(),
        memo: Vec::new(),
        framing: Framing::Unframed,
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

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
    left: usize,
    /// The named operands the form has matched so far inside its fixed runs.
    says: Vec<Said>,
    /// What the file has filed under each memo slot, in the order it wrote
    /// them. A slot number is the count of memo marks before it, so every
    /// mark a production consumes has to land here or the numbering drifts.
    memo: Vec<Bound>,
    framing: Framing,
    allow: Allow,
    calls: Vec<Call>,
    payloads: Vec<(usize, Payload)>,
    /// How many of each specific production fired, which is what says the
    /// file belongs to the form that allows it.
    arrays: usize,
    objects: usize,
}

impl<'a> Cursor<'a> {
    fn whole(&mut self, form: &'static str) -> Option<Match> {
        self.exact(&[0x80])?;
        if !matches!(self.byte()?, 4 | 5) {
            return None;
        }
        if self.peek() == Some(0x95) {
            self.frame_header()?;
        }
        let body = self.at;
        let value = self.value(0)?;
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
            value,
            body,
            calls: std::mem::take(&mut self.calls),
            ops: instructions(self.bytes),
            stop: self.at - 1,
            payloads: std::mem::take(&mut self.payloads),
        })
    }

    fn take(&mut self, len: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(len)?;
        let out = self.bytes.get(self.at..end)?;
        self.at = end;
        Some(out)
    }
    fn byte(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }
    fn exact(&mut self, expected: &[u8]) -> Option<()> {
        (self.take(expected.len())? == expected).then_some(())
    }

    fn save(&self) -> Save {
        Save {
            at: self.at,
            memo: self.memo.len(),
            says: self.says.len(),
            calls: self.calls.len(),
            payloads: self.payloads.len(),
            framing: self.framing,
            arrays: self.arrays,
            objects: self.objects,
        }
    }

    /// Put everything back except the work already spent, which an
    /// alternative that failed does not get to spend again.
    fn restore(&mut self, s: Save) {
        self.at = s.at;
        self.memo.truncate(s.memo);
        self.says.truncate(s.says);
        self.calls.truncate(s.calls);
        self.payloads.truncate(s.payloads);
        self.framing = s.framing;
        self.arrays = s.arrays;
        self.objects = s.objects;
    }

    /// FRAME and its eight-byte length, which must land inside the file.
    fn frame_header(&mut self) -> Option<()> {
        self.exact(&[0x95])?;
        let size = u64::from_le_bytes(self.take(8)?.try_into().ok()?);
        let end = self.at.checked_add(usize::try_from(size).ok()?)?;
        if end > self.bytes.len() {
            return None;
        }
        self.framing = Framing::Inside(end);
        Some(())
    }

    /// The boundary between two objects, where a frame may end and the next
    /// one begin. Every object production starts here.
    fn gate(&mut self) -> Option<()> {
        if let Framing::Inside(end) = self.framing {
            if self.at > end {
                return None;
            }
            if self.at == end {
                self.framing = Framing::Between(end);
            }
        }
        if matches!(self.framing, Framing::Between(_) | Framing::Tail(_)) && self.peek() == Some(0x95) {
            self.frame_header()?;
        }
        Some(())
    }

    /// A counted run of bytes, and the framing a large one demands.
    ///
    /// The opcode byte has already been read and sits at `self.at - 1`. A
    /// payload of [`BIG_PAYLOAD`] bytes or more begins exactly where a frame
    /// ended and a new frame begins after it; a smaller one is inside a frame.
    fn counted(&mut self, code: u8, short: u8, wide: u8, widest: u8) -> Option<(usize, usize)> {
        let opcode_at = self.at - 1;
        let framed = !matches!(self.framing, Framing::Unframed);
        let between = matches!(self.framing, Framing::Between(from) if from == opcode_at);
        let len = self.length(code, short, wide, widest)?;
        if framed && (len >= BIG_PAYLOAD) != between {
            return None;
        }
        let at = self.at;
        self.take(len)?;
        if between {
            // The writer starts a new frame right after the bytes it wrote
            // between frames, unless what is left is too short to frame.
            self.framing = Framing::Tail(self.at);
            self.gate()?;
        }
        Some((at, len))
    }

    fn length(&mut self, code: u8, short: u8, wide: u8, widest: u8) -> Option<usize> {
        let n = if code == short {
            u64::from(self.byte()?)
        } else if code == wide {
            u32::from_le_bytes(self.take(4)?.try_into().ok()?) as u64
        } else if code == widest {
            u64::from_le_bytes(self.take(8)?.try_into().ok()?)
        } else {
            return None;
        };
        usize::try_from(n).ok()
    }

    /// MEMOIZE, which files the value just built in the next slot. Protocol 4
    /// writes no index: the slot is the count of marks before it.
    fn memoize(&mut self, bound: Bound) -> Option<usize> {
        self.exact(&[0x94])?;
        if self.memo.len() >= MAX_MEMO {
            return None;
        }
        self.memo.push(bound);
        Some(self.memo.len() - 1)
    }

    /// BINGET or LONG_BINGET, and what the slot it names holds. A slot past
    /// the end, or one the file has not written, is a non-match.
    fn reference(&mut self) -> Option<&Bound> {
        let slot = match self.byte()? {
            b'h' => self.byte()? as usize,
            b'j' => u32::from_le_bytes(self.take(4)?.try_into().ok()?) as usize,
            _ => return None,
        };
        self.memo.get(slot)
    }

    fn at_reference(&self) -> bool {
        matches!(self.peek(), Some(b'h') | Some(b'j'))
    }

    /// SHORT_BINUNICODE with exactly this spelling, and the memo mark that
    /// files it. Comes back as the bytes the word sits in.
    fn word(&mut self, text: &str) -> Option<(usize, usize)> {
        self.exact(&[0x8c, u8::try_from(text.len()).ok()?])?;
        let at = self.at;
        self.exact(text.as_bytes())?;
        self.memoize(Bound::Text { at, len: text.len() })?;
        Some((at, text.len()))
    }

    /// The same word, spelled out here or referred to where it was spelled.
    /// A reference has no bytes of its own to read as text, so it comes back
    /// as nothing to name and the BINGET stays an instruction.
    fn word_or_reference(&mut self, text: &str) -> Option<Option<(usize, usize)>> {
        if self.at_reference() {
            let here = self.save();
            let held = self.reference().cloned();
            if let Some(Bound::Text { at, len }) = held {
                if self.bytes.get(at..at + len) == Some(text.as_bytes()) {
                    return Some(None);
                }
            }
            self.restore(here);
            return None;
        }
        Some(Some(self.word(text)?))
    }

    /// A module and a callable, joined by STACK_GLOBAL, or a reference to the
    /// slot the same pair was filed in earlier. `modules` is every spelling
    /// the form accepts, each written out.
    fn global(
        &mut self,
        modules: &[&str],
        name: &str,
        module_says: &'static str,
        name_says: &'static str,
    ) -> Option<String> {
        // A slot holding this exact module and callable stands for the pair.
        // Any other reference here is the module name on its own, which the
        // spelled-out production below reads.
        if self.at_reference() {
            let here = self.save();
            if let Some(Bound::Global(full)) = self.reference().cloned() {
                if modules.iter().any(|m| full == format!("{m}.{name}")) {
                    return Some(full);
                }
            }
            self.restore(here);
        }
        let here = self.save();
        let module = modules.iter().find_map(|m| {
            self.restore(here);
            self.word_or_reference(m).map(|said| {
                if let Some((at, len)) = said {
                    self.says(module_says, at, len);
                }
                (*m).to_string()
            })
        })?;
        let (at, len) = self.word(name)?;
        self.says(name_says, at, len);
        self.exact(&[0x93])?;
        let full = format!("{module}.{name}");
        self.memoize(Bound::Global(full.clone()))?;
        Some(full)
    }

    fn text(&mut self) -> Option<Value> {
        self.gate()?;
        let start = self.at;
        let code = self.byte()?;
        let (at, len) = self.counted(code, 0x8c, 0x58, 0x8d)?;
        std::str::from_utf8(self.bytes.get(at..at + len)?).ok()?;
        self.memoize(Bound::Text { at, len })?;
        Some(self.span(start, Kind::Text { at, len }))
    }

    /// A value and the bytes it was written in, which is everything the
    /// production consumed.
    fn span(&self, start: usize, kind: Kind) -> Value {
        Value {
            at: start,
            len: self.at - start,
            kind,
        }
    }

    /// Remember a named operand inside the run of instructions being matched.
    fn says(&mut self, name: &'static str, at: usize, len: usize) {
        self.says.push(Said { name, at, len });
    }

    fn value(&mut self, depth: usize) -> Option<Value> {
        if depth >= MAX_DEPTH {
            return None;
        }
        self.left = self.left.checked_sub(1)?;
        self.gate()?;
        // The productions a form adds to the basic ones. Each is tried whole
        // and rewound whole, and the budget above is spent either way.
        if self.allow.numpy {
            let here = self.save();
            match self.numpy() {
                Some(value) => return Some(value),
                None => self.restore(here),
            }
        }
        if self.allow.builtins {
            let here = self.save();
            match self.builtin(depth) {
                Some(value) => return Some(value),
                None => self.restore(here),
            }
        }
        if matches!(self.peek()?, 0x8c | 0x58 | 0x8d) {
            return self.text();
        }
        let start = self.at;
        let kind = match self.byte()? {
            b'N' => Kind::None,
            0x88 => Kind::Bool(true),
            0x89 => Kind::Bool(false),
            b'K' => Kind::Int {
                value: self.byte()? as i128,
                at: start + 1,
                len: 1,
            },
            b'M' => Kind::Int {
                value: u16::from_le_bytes(self.take(2)?.try_into().ok()?) as i128,
                at: start + 1,
                len: 2,
            },
            b'J' => Kind::Int {
                value: i32::from_le_bytes(self.take(4)?.try_into().ok()?) as i128,
                at: start + 1,
                len: 4,
            },
            b'G' => Kind::Float {
                value: f64::from_be_bytes(self.take(8)?.try_into().ok()?),
                at: start + 1,
                len: 8,
            },
            code @ (b'C' | b'B' | 0x8e) => {
                let (at, len) = self.counted(code, b'C', b'B', 0x8e)?;
                self.memoize(Bound::Bytes { at, len })?;
                Kind::Bytes { at, len }
            }
            b')' => Kind::Tuple(Vec::new()),
            b']' => {
                self.memoize(Bound::Opaque)?;
                let mut values = Vec::new();
                if self.peek() == Some(b'(') {
                    self.byte()?;
                    while self.peek()? != b'e' {
                        values.push(self.value(depth + 1)?);
                    }
                    self.byte()?;
                    if values.len() < 2 || values.len() > MAX_BATCH {
                        return None;
                    }
                } else {
                    // EMPTY_LIST and a single-item list share a prefix. Try
                    // the whole single-item production before accepting empty.
                    // Rewinding never restores the shared work budget.
                    let here = self.save();
                    if let Some(value) = self.value(depth + 1).filter(|_| self.exact(b"a").is_some()) {
                        values.push(value);
                    } else {
                        self.restore(here);
                    }
                }
                Kind::List(values)
            }
            b'}' => {
                self.memoize(Bound::Opaque)?;
                let mut entries = Vec::new();
                if self.peek() == Some(b'(') {
                    self.byte()?;
                    while self.peek()? != b'u' {
                        entries.push((self.text()?, self.value(depth + 1)?));
                    }
                    self.byte()?;
                    if entries.len() < 2 || entries.len() > MAX_BATCH {
                        return None;
                    }
                } else {
                    let here = self.save();
                    let entry = (|| {
                        let key = self.text()?;
                        let value = self.value(depth + 1)?;
                        self.exact(b"s")?;
                        Some((key, value))
                    })();
                    match entry {
                        Some(entry) => entries.push(entry),
                        None => self.restore(here),
                    }
                }
                Kind::Dict(entries)
            }
            _ => return None,
        };
        Some(self.span(start, kind))
    }

    /// One dimension of a shape, which is written as a nonnegative integer in
    /// whichever of the three widths holds it.
    fn dimension(&mut self) -> Option<u64> {
        match self.byte()? {
            b'K' => Some(self.byte()? as u64),
            b'M' => Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?) as u64),
            b'J' => u64::try_from(i32::from_le_bytes(self.take(4)?.try_into().ok()?)).ok(),
            _ => None,
        }
    }

    /// A shape, as the tuple its arity is written with: EMPTY_TUPLE for none,
    /// TUPLE1 to TUPLE3 for one to three, and MARK..TUPLE past that. Only the
    /// counted tuples carry a memo mark; CPython never files an empty one.
    fn dimensions(&mut self) -> Option<Vec<u64>> {
        if self.peek()? == b')' {
            self.byte()?;
            return Some(Vec::new());
        }
        let marked = self.peek()? == b'(';
        if marked {
            self.byte()?;
        }
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
        if self.at_reference() {
            let here = self.save();
            if let Some(Bound::Dtype(dtype)) = self.reference().cloned() {
                return Some(dtype);
            }
            self.restore(here);
        }
        self.global(&["numpy"], "dtype", "dtype module", "dtype class")?;
        let (kind_at, kind_len) = {
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
        self.exact(b"\x89\x88\x87")?;
        self.memoize(Bound::Opaque)?;
        self.exact(b"R")?;
        let slot = self.memoize(Bound::Opaque)?;
        // The state the BUILD sets: version 3, the byte order, three Nones,
        // no field offsets, and an alignment of zero.
        self.exact(b"(K\x03")?;
        let order = self.byte_order(kind.as_str())?;
        self.exact(b"NNNJ\xff\xff\xff\xffJ\xff\xff\xff\xffK\0t")?;
        self.memoize(Bound::Opaque)?;
        self.exact(b"b")?;
        let dtype = format!("{order}{kind}");
        self.memo[slot] = Bound::Dtype(dtype.clone());
        Some(dtype)
    }

    /// The letter that says which way round a dtype's bytes go, spelled out
    /// here or referred to where it was spelled. A single-byte dtype has no
    /// order and says so with `|`.
    fn byte_order(&mut self, kind: &str) -> Option<char> {
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
        let (shape, width) = shapes::dtype(dtype)?;
        let code = self.byte()?;
        let (at, len) = self.counted(code, b'C', b'B', 0x8e)?;
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
        Some((at, len, Payload { shape, count }))
    }

    /// A NumPy array or a NumPy scalar, each rebuilt by the one call the form
    /// names and fixed instruction for instruction around it.
    fn numpy(&mut self) -> Option<Value> {
        let start = self.at;
        let here = self.save();
        match self.reconstructed(start) {
            Some(value) => Some(value),
            None => {
                self.restore(here);
                self.scalar(start)
            }
        }
    }

    /// `numpy._core.multiarray._reconstruct(ndarray, (0,), b'b')` and the
    /// BUILD that hands the result its shape, dtype, storage order and bytes.
    fn reconstructed(&mut self, start: usize) -> Option<Value> {
        const MODULES: &[&str] = &["numpy._core.multiarray", "numpy.core.multiarray"];
        self.global(MODULES, "_reconstruct", "module", "callable")?;
        self.global(&["numpy"], "ndarray", "class module", "class")?;
        // The placeholder the reconstructor is given: shape (0,) and a dtype
        // letter, both replaced by the BUILD that follows.
        self.exact(b"K\0\x85")?;
        self.memoize(Bound::Opaque)?;
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
        self.exact(b"(K\x01")?;
        let dimensions = self.dimensions()?;
        let dtype = self.dtype()?;
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

    /// The run of instructions just matched, with the names spelled inside it.
    fn finish_call(&mut self, name: &'static str, at: usize, end: usize) {
        let says = std::mem::take(&mut self.says);
        self.calls.push(Call { name, at, len: end - at, says });
    }

    /// The builtin types a pickle writes as a call rather than as a literal.
    fn builtin(&mut self, depth: usize) -> Option<Value> {
        let start = self.at;
        // A frozenset is the one of these with an opcode of its own, and a
        // MARK can begin nothing else where a value is expected.
        if self.peek()? == b'(' {
            self.byte()?;
            let mut items = Vec::new();
            while self.peek()? != 0x91 {
                if items.len() == MAX_BATCH {
                    return None;
                }
                items.push(self.value(depth + 1)?);
            }
            self.byte()?;
            self.memoize(Bound::Opaque)?;
            self.objects += 1;
            return Some(self.span(start, Kind::Object { what: Shape::FrozenSet, names: MEMBERS, items }));
        }
        let here = self.save();
        for (name, what, names, arity) in [
            ("slice", Shape::Slice, BOUNDS, 3usize),
            ("range", Shape::Range, BOUNDS, 3),
            ("complex", Shape::Complex, HALVES, 2),
            ("bytearray", Shape::ByteArray, CONTENT, 1),
        ] {
            self.restore(here);
            let made = (|| {
                self.global(&["builtins"], name, "module", "class")?;
                let mut items = Vec::new();
                for _ in 0..arity {
                    items.push(match what {
                        // A slice's bounds are integers or None; a range's are
                        // always integers; a complex is two floats; and a
                        // bytearray is made from one byte string.
                        Shape::Slice => self.small_int_or_none()?,
                        Shape::Range => self.small_int()?,
                        Shape::Complex => self.binfloat()?,
                        _ => self.byte_string()?,
                    });
                }
                self.exact(&[0x84 + arity as u8])?;
                self.memoize(Bound::Opaque)?;
                self.exact(b"R")?;
                self.memoize(Bound::Opaque)?;
                Some(self.span(start, Kind::Object { what, names, items }))
            })();
            if let Some(value) = made {
                // The names the call was made with stay as the instructions
                // they are; the value already says what it is.
                self.says.truncate(here.says);
                self.objects += 1;
                return Some(value);
            }
        }
        self.restore(here);
        None
    }

    fn small_int(&mut self) -> Option<Value> {
        let start = self.at;
        let kind = match self.byte()? {
            b'K' => Kind::Int { value: self.byte()? as i128, at: start + 1, len: 1 },
            b'M' => Kind::Int {
                value: u16::from_le_bytes(self.take(2)?.try_into().ok()?) as i128,
                at: start + 1,
                len: 2,
            },
            b'J' => Kind::Int {
                value: i32::from_le_bytes(self.take(4)?.try_into().ok()?) as i128,
                at: start + 1,
                len: 4,
            },
            _ => return None,
        };
        Some(self.span(start, kind))
    }

    fn small_int_or_none(&mut self) -> Option<Value> {
        if self.peek()? == b'N' {
            let start = self.at;
            self.byte()?;
            return Some(self.span(start, Kind::None));
        }
        self.small_int()
    }

    fn binfloat(&mut self) -> Option<Value> {
        let start = self.at;
        self.exact(b"G")?;
        let value = f64::from_be_bytes(self.take(8)?.try_into().ok()?);
        Some(self.span(start, Kind::Float { value, at: start + 1, len: 8 }))
    }

    fn byte_string(&mut self) -> Option<Value> {
        let start = self.at;
        let code = self.byte()?;
        let (at, len) = self.counted(code, b'C', b'B', 0x8e)?;
        self.memoize(Bound::Bytes { at, len })?;
        Some(self.span(start, Kind::Bytes { at, len }))
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn framed(body: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0x80, 4, 0x95];
        bytes.extend_from_slice(&(body.len() as u64).to_le_bytes());
        bytes.extend_from_slice(body);
        bytes
    }

    #[test]
    fn captures_basic_values_without_a_machine() {
        let bytes = framed(b"}\x94\x8c\x01a\x94]\x94(K\x01K\x02es.");
        let found = recognise(&bytes).unwrap();
        assert_eq!(found.form, "basic-p4-p5-v3");
        // Every node spans the bytes its production consumed, and a leaf says
        // where inside that its value proper sits.
        assert_eq!((found.value.at, found.value.len), (11, 15));
        let Kind::Dict(entries) = &found.value.kind else {
            panic!("dict expected")
        };
        let (key, value) = &entries[0];
        assert_eq!(entries.len(), 1);
        assert_eq!((key.at, key.len, &key.kind), (13, 4, &Kind::Text { at: 15, len: 1 }));
        assert_eq!((value.at, value.len), (17, 8));
        let Kind::List(items) = &value.kind else {
            panic!("list expected")
        };
        let seen: Vec<_> = items.iter().map(|v| (v.at, v.len, &v.kind)).collect();
        assert_eq!(
            seen,
            vec![
                (20, 2, &Kind::Int { value: 1, at: 21, len: 1 }),
                (22, 2, &Kind::Int { value: 2, at: 23, len: 1 }),
            ]
        );
        assert_eq!(
            found.text(Deduce::Builds, bytes.len() as u64).as_deref(),
            Some(stop_message("basic-p4-p5-v3").as_str())
        );
        assert!(recognise(b"\x80\x05N.").is_some());
    }

    #[test]
    fn rejects_incomplete_or_unfamiliar_programs() {
        let bytes = framed(b"}\x94\x8c\x01a\x94K\x01s.");
        for end in 0..bytes.len() {
            assert!(recognise(&bytes[..end]).is_none());
        }
        let mut trailing = bytes.clone();
        trailing.push(b'N');
        assert!(recognise(&trailing).is_none());
        for body in [
            &b"N0N."[..],
            b"N",
            b"N..",
            b"]\x94h\0a.",
            b"\x8c\x01\xff\x94.",
            b"\x95\0\0\0\0\0\0\0\0N.",
        ] {
            assert!(recognise(&framed(body)).is_none(), "{body:?}");
        }
        let mut wrong_frame = bytes;
        wrong_frame[3] -= 1;
        assert!(recognise(&wrong_frame).is_none());
    }

    #[test]
    fn recursion_is_bounded() {
        let mut bytes = vec![0x80, 4];
        for _ in 0..MAX_DEPTH + 1 {
            bytes.extend_from_slice(b"]\x94");
        }
        bytes.push(b'N');
        bytes.extend(std::iter::repeat_n(b'a', MAX_DEPTH + 1));
        bytes.push(b'.');
        assert!(recognise(&bytes).is_none());
    }

    const MATRIX: &[u8] = include_bytes!("../../../tests/fixtures/pickle/numpy-f32-matrix.pickle");

    #[test]
    fn captures_numpy_payload_and_rejects_changed_structure() {
        let found = recognise(MATRIX).unwrap();
        assert_eq!(found.form, "numpy-numeric-array-p4-p5-v3");
        let Kind::Dict(entries) = &found.value.kind else {
            panic!("dict expected")
        };
        let Kind::Array {
            at,
            dimensions,
            dtype,
            fortran_order,
            ..
        } = &entries[0].1.kind
        else {
            panic!("array expected")
        };
        let at = *at;
        assert_eq!(dimensions, &[4, 6]);
        assert_eq!(dtype, "<f4");
        assert!(!fortran_order);
        assert_eq!(found.int(Deduce::PayloadCount, at as u64), Some(24));
        let floats: Vec<_> = MATRIX[at..at + 96]
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        assert_eq!(floats, (0..24).map(|n| n as f32).collect::<Vec<_>>());
        for end in 0..MATRIX.len() {
            assert!(recognise(&MATRIX[..end]).is_none());
        }
        // Mutate fixed control bytes, shape, callable, memo reference and dtype.
        for offset in [0x30, 0x40, 0x65, 0x6b, 0x79, 0x9b, 0x100] {
            let mut changed = MATRIX.to_vec();
            changed[offset] ^= 1;
            assert!(
                recognise(&changed).is_none(),
                "accepted mutation at {offset:x}"
            );
        }
        let mut payload = MATRIX.to_vec();
        payload[at] = b'R'; // An opcode byte inside captured data is just data.
        assert!(recognise(&payload).is_some());
        let mut appended = MATRIX.to_vec();
        appended.push(b'N');
        assert!(recognise(&appended).is_none());
    }

    #[test]
    fn unmatched_input_keeps_symbolic_inspection() {
        let bytes = b"\x80\x04\x8c\x02os\x94\x8c\x06system\x94\x93.";
        assert!(recognise(bytes).is_none());
        let fallback = Program.run(bytes);
        let old = machine::Program.run(bytes);
        for at in 0..=bytes.len() as u64 {
            assert_eq!(
                fallback.text(Deduce::Builds, at),
                old.text(Deduce::Builds, at)
            );
        }
    }

    fn replace(bytes: &mut Vec<u8>, from: &[u8], to: &[u8]) {
        let positions: Vec<_> = bytes
            .windows(from.len())
            .enumerate()
            .filter_map(|(i, b)| (b == from).then_some(i))
            .collect();
        assert_eq!(
            positions.len(),
            1,
            "fixture replacement must be unambiguous"
        );
        bytes.splice(positions[0]..positions[0] + from.len(), to.iter().copied());
    }

    /// A standalone variant derived from the corpus fixture: remove the
    /// dictionary/key and SETITEM, and adjust its one explicit memo reference.
    fn standalone() -> Vec<u8> {
        assert_eq!(&MATRIX[11..23], b"}\x94\x8c\x07weights\x94");
        assert_eq!(&MATRIX[MATRIX.len() - 2..], b"s.");
        let mut body = MATRIX[23..MATRIX.len() - 2].to_vec();
        replace(&mut body, b"h\x05\x8c\x05dtype", b"h\x03\x8c\x05dtype");
        body.push(b'.');
        body
    }

    #[test]
    fn standalone_arrays_preserve_dimensions_dtype_and_storage_order() {
        let mut body = standalone();
        replace(&mut body, b"K\x04K\x06\x86\x94", b"K\x02K\x03K\x04\x87\x94");
        replace(
            &mut body,
            b"\x8c\x16numpy._core.multiarray",
            b"\x8c\x15numpy.core.multiarray",
        );
        replace(&mut body, b"\x8c\x02f4\x94", b"\x8c\x02i4\x94");
        replace(&mut body, b"\x8c\x01<\x94NNN", b"\x8c\x01>\x94NNN");
        replace(&mut body, b"t\x94b\x89C", b"t\x94b\x88C");
        let bytes = framed(&body);
        let found = recognise(&bytes).unwrap();
        let Kind::Array {
            at,
            len,
            dtype,
            dimensions,
            fortran_order,
        } = &found.value.kind
        else {
            panic!("array")
        };
        assert_eq!(
            (*len, dtype.as_str(), dimensions.as_slice(), *fortran_order),
            (96, ">i4", &[2, 3, 4][..], true)
        );
        assert_eq!(found.int(Deduce::PayloadCount, *at as u64), Some(24));
        assert_eq!(
            found.int(Deduce::PayloadShape, *at as u64),
            Some(shapes::dtype(">i4").unwrap().0 as i128)
        );
        // The standalone memo reference must match the slot for numpy, not
        // the dictionary variant's slot or any other existing slot.
        replace(&mut body, b"h\x03\x8c\x05dtype", b"h\x05\x8c\x05dtype");
        assert!(recognise(&framed(&body)).is_none());
    }

    #[test]
    fn numeric_dtype_branches_check_byte_order_and_payload_width() {
        for (kind, width, order) in [
            ("b1", 1, b'|'),
            ("i1", 1, b'|'),
            ("u1", 1, b'|'),
            ("i2", 2, b'<'),
            ("u2", 2, b'>'),
            ("i4", 4, b'<'),
            ("u4", 4, b'>'),
            ("i8", 8, b'<'),
            ("u8", 8, b'>'),
            ("f2", 2, b'<'),
            ("f4", 4, b'>'),
            ("f8", 8, b'<'),
            ("c8", 8, b'>'),
            ("c16", 16, b'<'),
        ] {
            let mut body = standalone();
            let mut dtype = vec![0x8c, kind.len() as u8];
            dtype.extend_from_slice(kind.as_bytes());
            dtype.push(0x94);
            replace(&mut body, b"\x8c\x02f4\x94", &dtype);
            replace(
                &mut body,
                b"\x8c\x01<\x94NNN",
                &[0x8c, 1, order, 0x94, b'N', b'N', b'N'],
            );
            let mut dims = vec![b'K', (96 / width) as u8, 0x85, 0x94];
            replace(&mut body, b"K\x04K\x06\x86\x94", &dims);
            assert!(recognise(&framed(&body)).is_some(), "{kind}");
            dims[1] += 1;
            let before = [b'K', (96 / width) as u8, 0x85, 0x94];
            replace(&mut body, &before, &dims);
            assert!(
                recognise(&framed(&body)).is_none(),
                "wrong length for {kind}"
            );
        }
    }

    #[test]
    fn scalar_empty_and_wide_arrays_obey_length_and_shape_constraints() {
        let mut base = standalone();
        // Replace the complete payload with one float (the scalar form).
        let payload = &MATRIX[0x9b..0xfd];
        assert_eq!(&payload[..2], b"C\x60");
        replace(&mut base, payload, b"C\x04\0\0\x80\x3f");
        replace(&mut base, b"K\x04K\x06\x86\x94", b")");
        let scalar = recognise(&framed(&base)).unwrap();
        let Kind::Array { dimensions, .. } = scalar.value.kind else {
            panic!("array")
        };
        assert!(dimensions.is_empty());

        let mut empty = standalone();
        replace(&mut empty, payload, b"C\0");
        replace(&mut empty, b"K\x04K\x06\x86\x94", b"K\0\x85\x94");
        assert!(recognise(&framed(&empty)).is_some());

        for code in [b'B', 0x8e] {
            let mut wide = standalone();
            replace(&mut wide, b"K\x04K\x06\x86\x94", b"M\0\x01\x85\x94");
            let mut data = vec![code];
            if code == b'B' {
                data.extend_from_slice(&1024u32.to_le_bytes());
            } else {
                data.extend_from_slice(&1024u64.to_le_bytes());
            }
            data.resize(data.len() + 1024, 0);
            replace(&mut wide, payload, &data);
            assert!(recognise(&framed(&wide)).is_some());
        }
        let mut overflow = standalone();
        replace(
            &mut overflow,
            b"K\x04K\x06\x86\x94",
            b"J\xff\xff\xff\x7fJ\xff\xff\xff\x7fJ\xff\xff\xff\x7f\x87\x94",
        );
        assert!(recognise(&framed(&overflow)).is_none());
        let mut negative = standalone();
        replace(
            &mut negative,
            b"K\x04K\x06\x86\x94",
            b"J\xff\xff\xff\xff\x85\x94",
        );
        assert!(recognise(&framed(&negative)).is_none());
    }

    #[test]
    fn nested_empty_containers_and_wide_text_are_captured() {
        let found = recognise(&framed(b"]\x94(]\x94K\x01}\x94\x8c\x01x\x94e.")).unwrap();
        let Kind::List(values) = found.value.kind else {
            panic!("list")
        };
        let kinds: Vec<_> = values[..3].iter().map(|v| &v.kind).collect();
        assert_eq!(
            kinds,
            vec![
                &Kind::List(vec![]),
                &Kind::Int { value: 1, at: 17, len: 1 },
                &Kind::Dict(vec![])
            ]
        );
        assert!(matches!(values[3].kind, Kind::Text { len: 1, .. }));
        for (code, width) in [(0x58, 4), (0x8d, 8)] {
            let mut body = vec![code];
            body.extend_from_slice(&300u64.to_le_bytes()[..width]);
            body.extend(std::iter::repeat_n(b'x', 300));
            body.extend_from_slice(b"\x94.");
            assert!(matches!(
                recognise(&framed(&body)).unwrap().value.kind,
                Kind::Text { len: 300, .. }
            ));
            body[1..1 + width].fill(255);
            assert!(recognise(&framed(&body)).is_none());
        }
    }

    #[test]
    fn template_publishes_the_match_message_at_stop() {
        use crate::{document::Document, eval::Evaluator, source::MemSource};
        let doc = Document::new(MemSource(vec![0x80, 4, b'N', b'.']));
        let mut ev = Evaluator::new(super::super::pickle());
        let node = ev.node(&doc, &[2, 1]).unwrap();
        // The contract's sentence, the form that matched, and where the data
        // is. The first sentence is the one the design document fixes.
        let said = stop_message("basic-p4-p5-v3");
        assert_eq!(node.value, crate::eval::Value::Str(said.clone()));
        assert!(said.starts_with("Matched a Familiar Pickle Form ("));
        assert!(said.contains(": bypassed Pickle stack machine decoding."));
        assert!(said.ends_with(&format!("Switch to the \"{FAMILIAR_LABEL}\" template to see the data.")));
    }

    use crate::document::Document;
    use crate::eval::{Evaluator, Value as V};
    use crate::source::MemSource;

    fn read(bytes: &[u8]) -> (Document<MemSource>, Evaluator) {
        (Document::new(MemSource(bytes.to_vec())), Evaluator::new(super::super::familiar_pickle()))
    }

    /// One row of the template, as a reader sees it.
    #[derive(Debug, PartialEq)]
    struct Row {
        depth: usize,
        name: String,
        ty: String,
        at: u64,
        len: u64,
        value: V,
        machinery: bool,
    }

    /// Every row under `path`, in file order, with the rows inside a node
    /// after it.
    fn rows(doc: &Document<MemSource>, ev: &mut Evaluator, path: &[usize], depth: usize, out: &mut Vec<Row>) {
        let node = ev.node(doc, path).unwrap_or_else(|e| panic!("{path:?}: {e:?}"));
        out.push(Row {
            depth,
            name: node.name.clone(),
            ty: node.type_name.clone(),
            at: node.offset_bits / 8,
            len: node.size_bits / 8,
            value: node.value.clone(),
            machinery: node.machinery == Some(true),
        });
        // An array's numbers are a run of values rather than the shape of the
        // file, so they are counted and not walked.
        if node.type_name.ends_with("[]") {
            return;
        }
        for i in 0..node.child_count as usize {
            let mut next = path.to_vec();
            next.push(i);
            rows(doc, ev, &next, depth + 1, out);
        }
    }

    fn dump(bytes: &[u8]) -> Vec<Row> {
        let (doc, mut ev) = read(bytes);
        let mut out = Vec::new();
        rows(&doc, &mut ev, &[], 0, &mut out);
        out
    }

    /// The first row of this name, which is what an assertion about a named
    /// field means when the name is not repeated.
    fn named_row<'a>(rows: &'a [Row], name: &str) -> &'a Row {
        rows.iter().find(|r| r.name == name).unwrap_or_else(|| panic!("no {name} row in {rows:#?}"))
    }

    /// Every row the familiar-form template shows for a small dictionary, in
    /// file order: the header, the instructions the form fixed, the names and
    /// the values.
    #[test]
    fn the_familiar_template_places_the_decoded_values() {
        let bytes = framed(b"}\x94\x8c\x01a\x94]\x94(K\x01K\x02es.");
        let seen = dump(&bytes);
        let said: Vec<(usize, &str, &str, u64, u64)> =
            seen.iter().map(|r| (r.depth, r.name.as_str(), r.ty.as_str(), r.at, r.len)).collect();
        assert_eq!(
            said,
            vec![
                (0, "file", "pickle", 0, 27),
                (1, "header", "header", 0, 11),
                (2, "message", "computed text", 0, 0),
                (2, "form", "computed text", 0, 0),
                (2, "proto", "bytes[]", 0, 1),
                (2, "protocol", "u8", 1, 1),
                (2, "frame", "bytes[]", 2, 9),
                // The dictionary, from its EMPTY_DICT to the SETITEM that
                // filled it, with both of those inside it.
                (1, "data", "dict", 11, 15),
                (2, "empty_dict", "bytes[]", 11, 1),
                (2, "memoize", "bytes[]", 12, 1),
                (2, "a", "entry", 13, 12),
                (3, "short_binunicode", "bytes[]", 13, 2),
                (3, "key", "utf8[]", 15, 1),
                (3, "memoize", "bytes[]", 16, 1),
                (3, "value", "list", 17, 8),
                (4, "empty_list", "bytes[]", 17, 1),
                (4, "memoize", "bytes[]", 18, 1),
                (4, "mark", "bytes[]", 19, 1),
                (4, "binint1", "bytes[]", 20, 1),
                (4, "[0]", "u8", 21, 1),
                (4, "binint1", "bytes[]", 22, 1),
                (4, "[1]", "u8", 23, 1),
                (4, "appends", "bytes[]", 24, 1),
                (2, "setitem", "bytes[]", 25, 1),
                (1, "stop", "bytes[]", 26, 1),
            ]
        );
        assert_eq!(named_row(&seen, "message").value, V::Str(MESSAGE.into()));
        assert_eq!(named_row(&seen, "form").value, V::Str("basic-p4-p5-v3".into()));
        assert_eq!(named_row(&seen, "protocol").value, V::UInt(4));
        assert_eq!(named_row(&seen, "key").value, V::Str("a".into()));
        assert_eq!(named_row(&seen, "[0]").value, V::UInt(1));
        assert_eq!(named_row(&seen, "[1]").value, V::UInt(2));
        // The instructions fold away; the values do not.
        assert!(named_row(&seen, "setitem").machinery, "an instruction is the value's machinery");
        assert!(!named_row(&seen, "key").machinery);
        assert!(!named_row(&seen, "data").machinery);
    }

    /// Nothing a matched form consumed is left over. A form fixes its
    /// instructions, so a byte no field covers would be a byte the form
    /// matched and the template could not name.
    #[test]
    fn a_matched_file_has_no_unmapped_bytes() {
        for bytes in [
            framed(b"}\x94\x8c\x01a\x94]\x94(K\x01K\x02es."),
            framed(b"]\x94(N\x88\x89M\x39\x30J\xff\xff\xff\xffG\x3f\xf0\x00\x00\x00\x00\x00\x00C\x02\xde\xad\x94e."),
            framed(b"}\x94."),
            MATRIX.to_vec(),
            // An array of shape 0: its numbers are a field the file wrote and
            // a run of no bytes at once.
            framed(&cat(&[b"}\x94", &word("a"), &one_array(2, "f8", b'<', b"K\0\x85\x94", &[]), b"s."])),
        ] {
            let seen = dump(&bytes);
            assert_eq!(seen[0].len, bytes.len() as u64, "the root is the file");
            tiles(&seen);
        }
    }

    /// Every node's children tile it: they start where it does, they follow
    /// each other, and the last of them ends where it ends. A row worked out
    /// from the match is not read from the file and sits where its parent
    /// starts; a field the file did write takes its turn in the tiling even
    /// when it reads no bytes.
    fn tiles(seen: &[Row]) {
        for (i, row) in seen.iter().enumerate() {
            let kids: Vec<&Row> = seen[i + 1..]
                .iter()
                .take_while(|r| r.depth > row.depth)
                .filter(|r| r.depth == row.depth + 1)
                .collect();
            if kids.is_empty() {
                continue;
            }
            let mut want = row.at;
            for kid in &kids {
                if kid.ty.starts_with("computed") {
                    assert_eq!(kid.at, row.at, "{}: {} is worked out from the match, so it belongs where {} starts", row.name, kid.name, row.name);
                    continue;
                }
                assert_eq!(kid.at, want, "{}: {} leaves bytes over at {want:#x}", row.name, kid.name);
                want = kid.at + kid.len;
            }
            assert_eq!(want, row.at + row.len, "{}: bytes left over at {want:#x}", row.name);
        }
    }

    /// An array whose shape is 0 holds no numbers, and the row that says so is
    /// still a field of the file: it sits at the byte the payload would have
    /// started at, right after the SHORT_BINBYTES that wrote a length of zero,
    /// rather than being pushed back to the start of the array.
    #[test]
    fn an_empty_array_reads_its_numbers_where_they_would_have_been() {
        let bytes = framed(&cat(&[b"}\x94", &word("a"), &one_array(2, "f8", b'<', b"K\0\x85\x94", &[]), b"s."]));
        let seen = dump(&bytes);
        assert_eq!(named_row(&seen, "form").value, V::Str("numpy-numeric-array-p4-p5-v3".into()));
        assert_eq!(named_row(&seen, "shape").value, V::Str("0".into()));
        let numbers = named_row(&seen, "numbers");
        assert_eq!((numbers.ty.as_str(), numbers.len, &numbers.value), ("f64 le[]", 0, &V::Composite { count: 0 }));
        // The SHORT_BINBYTES before it is the opcode and the length byte, so
        // the numbers begin two bytes later and end where they begin.
        let header = seen.iter().rev().find(|r| r.name == "short_binbytes" && r.at < numbers.at).unwrap();
        assert_eq!((header.len, numbers.at), (2, header.at + 2));
        let array = named_row(&seen, "value");
        assert!(numbers.at > array.at, "the numbers are placed, not pushed back to {:#x}", array.at);
        tiles(&seen);
    }

    /// The two values a pickle writes as an opcode and nothing else read as
    /// the word Python spells them with.
    fn named(raw: i128, name: &str) -> V {
        V::Enum { raw, name: Some(name.to_string()), hex: false }
    }

    /// The other leaf types, each at the bytes it was written in.
    #[test]
    fn every_leaf_reads_as_the_type_its_bytes_are() {
        let bytes = framed(b"]\x94(N\x88\x89M\x39\x30J\xff\xff\xff\xffG\x3f\xf0\x00\x00\x00\x00\x00\x00C\x02\xde\xad\x94e.");
        // The values of the list, which is what the file holds: the header
        // says what matched and is not part of it.
        let seen: Vec<(String, u64, V)> = dump(&bytes)
            .into_iter()
            .skip_while(|r| r.name != "data")
            .filter(|r| !r.machinery && r.depth == 2)
            .map(|r| (r.ty, r.len, r.value))
            .collect();
        assert_eq!(
            seen,
            vec![
                ("null".into(), 1, named(0x4e, "None")),
                ("bool".into(), 1, named(0x88, "True")),
                ("bool".into(), 1, named(0x89, "False")),
                ("u16 le".into(), 2, V::UInt(12345)),
                ("i32 le".into(), 4, V::Int(-1)),
                ("f64 be".into(), 8, V::Float(1.0)),
                ("bytes[]".into(), 2, V::Bytes { len: 2, preview: vec![0xde, 0xad] }),
            ]
        );
    }

    /// A matched array says what it is before it says what it holds, and the
    /// call that rebuilt it holds the names and letters the form matched.
    #[test]
    fn a_matched_array_carries_its_dtype_shape_and_order() {
        let seen = dump(MATRIX);
        assert_eq!(named_row(&seen, "form").value, V::Str("numpy-numeric-array-p4-p5-v3".into()));
        assert_eq!(named_row(&seen, "value").ty, "array");
        assert_eq!(named_row(&seen, "dtype").value, V::Str("<f4".into()));
        assert_eq!(named_row(&seen, "shape").value, V::Str("4 x 6".into()));
        assert_eq!(named_row(&seen, "order").value, V::Str("C".into()));
        // The call, and the names it was made with.
        let call = named_row(&seen, "ndarray reconstruct call");
        assert_eq!(call.ty, "call");
        assert_eq!(named_row(&seen, "module").value, V::Str("numpy._core.multiarray".into()));
        assert_eq!(named_row(&seen, "callable").value, V::Str("_reconstruct".into()));
        assert_eq!(named_row(&seen, "class module").value, V::Str("numpy".into()));
        assert_eq!(named_row(&seen, "class").value, V::Str("ndarray".into()));
        assert_eq!(named_row(&seen, "dtype class").value, V::Str("dtype".into()));
        assert_eq!(named_row(&seen, "byte order").value, V::Str("<".into()));
        let numbers = named_row(&seen, "numbers");
        assert_eq!((numbers.ty.as_str(), numbers.len, &numbers.value), ("f32 le[]", 96, &V::Composite { count: 24 }));
    }

    /// The numbers of a matched array are a table of its rows, six to a row
    /// for a 4 x 6 array, and nothing else in the file is.
    #[test]
    fn a_matched_array_is_a_table_of_its_rows() {
        let (doc, mut ev) = read(MATRIX);
        let mut tables = Vec::new();
        let mut stack = vec![Vec::new()];
        while let Some(path) = stack.pop() {
            let info = ev.node(&doc, &path).unwrap();
            if info.table {
                tables.push((info.name.clone(), ev.table_shape(&doc, &path).unwrap().expect("a shape").columns));
            }
            if path.len() < 5 {
                stack.extend((0..info.child_count.min(80) as usize).map(|i| [path.as_slice(), &[i]].concat()));
            }
        }
        assert_eq!(tables, vec![("numbers".to_string(), Some(6))]);
    }

    /// A pickle no form matches has nothing for this template to show, and
    /// says so rather than showing part of a reading.
    #[test]
    fn an_unfamiliar_program_has_no_tree() {
        let (doc, mut ev) = read(b"\x80\x04\x8c\x02os\x94\x8c\x06system\x94\x93.");
        let root = ev.node(&doc, &[]);
        assert!(root.is_err(), "an unmatched file resolved to {root:?}");
        assert!(ev.node(&doc, &[0]).is_err());
        assert!(ev.node(&doc, &[1]).is_err());
    }

    // Everything below builds its own bytes rather than editing a fixture, so
    // that a test says which alternative it is exercising. A memo slot is the
    // count of memo marks before it, so the comments count them out.

    fn cat(pieces: &[&[u8]]) -> Vec<u8> {
        pieces.concat()
    }

    /// SHORT_BINUNICODE and the memo mark after it.
    fn word(text: &str) -> Vec<u8> {
        let mut out = vec![0x8c, text.len() as u8];
        out.extend_from_slice(text.as_bytes());
        out.push(0x94);
        out
    }

    /// SHORT_BINBYTES and the memo mark after it.
    fn blob(data: &[u8]) -> Vec<u8> {
        let mut out = vec![b'C', data.len() as u8];
        out.extend_from_slice(data);
        out.push(0x94);
        out
    }

    /// BINGET.
    fn get(slot: u8) -> Vec<u8> {
        vec![b'h', slot]
    }

    /// `numpy._core.multiarray._reconstruct(numpy.ndarray, (0,), b'b')` with
    /// every name written out, which is what the first array of a file does.
    ///
    /// Counting from the slot its first word lands in: 2 is the reconstructor,
    /// 3 the text `numpy`, 5 the `ndarray` class, 7 the placeholder byte
    /// string, and 9 the call's result.
    fn reconstruct() -> Vec<u8> {
        cat(&[
            &word("numpy._core.multiarray"),
            &word("_reconstruct"),
            b"\x93\x94",
            &word("numpy"),
            &word("ndarray"),
            b"\x93\x94",
            b"K\0\x85\x94",
            &blob(b"b"),
            b"\x87\x94R\x94",
        ])
    }

    /// The dtype construction and the BUILD that gives it its byte order, with
    /// the class's module taken from the slot it was written in.
    ///
    /// Counting from the slot its first word lands in: 1 is the `dtype` text,
    /// 2 the `numpy.dtype` class, 5 the finished dtype, and 6 the byte order.
    fn dtype_state(numpy_slot: u8, kind: &str, order: u8) -> Vec<u8> {
        cat(&[
            &get(numpy_slot),
            &word("dtype"),
            b"\x93\x94",
            &word(kind),
            b"\x89\x88\x87\x94R\x94",
            b"(K\x03",
            &word(std::str::from_utf8(&[order]).unwrap()),
            b"NNNJ\xff\xff\xff\xffJ\xff\xff\xff\xffK\0t\x94b",
        ])
    }

    /// One array written out in full. `base` is the slot the reconstructor's
    /// first word lands in, since whatever the file wrote before it has taken
    /// slots of its own.
    fn one_array(base: u8, kind: &str, order: u8, shape: &[u8], data: &[u8]) -> Vec<u8> {
        cat(&[
            &reconstruct(),
            b"(K\x01",
            shape,
            &dtype_state(base + 3, kind, order),
            b"\x89",
            &blob(data),
            b"t\x94b",
        ])
    }

    /// Two arrays under one dictionary, the second naming what the first
    /// wrote: both globals, the placeholder byte string, and whatever
    /// `second_dtype` says about the dtype. This is the shape
    /// `proto4-numpy-shared-dtype` has.
    ///
    /// Slot 0 is the dictionary and 1 the first key, so the reconstructor runs
    /// from slot 2: 4 is the `_reconstruct` global, 5 the text `numpy`, 7 the
    /// `ndarray` class, 9 the placeholder byte string, 14 the `numpy.dtype`
    /// class, 17 the finished dtype and 18 the byte order.
    fn two_arrays(second_dtype: &[u8]) -> Vec<u8> {
        cat(&[
            b"}\x94(",
            &word("a"),
            &reconstruct(),
            b"(K\x01K\x02\x85\x94",
            &dtype_state(5, "i1", b'|'),
            b"\x89",
            &blob(&[1, 2]),
            b"t\x94b",
            &word("b"),
            &get(4),
            &get(7),
            b"K\0\x85\x94",
            &get(9),
            b"\x87\x94R\x94",
            b"(K\x01K\x03\x85\x94",
            second_dtype,
            b"\x89",
            &blob(&[1, 2, 3]),
            b"t\x94b",
            b"u.",
        ])
    }

    /// Every way the second array of a file may name what the first one wrote,
    /// and the ones that are not a way.
    #[test]
    fn a_later_array_may_name_what_an_earlier_one_wrote() {
        // The whole finished dtype, out of the slot its REDUCE filed it in.
        let shared = two_arrays(&get(17));
        let found = recognise(&framed(&shared)).unwrap();
        assert_eq!(found.form, "numpy-numeric-array-p4-p5-v3");
        let Kind::Dict(entries) = &found.value.kind else { panic!("dict") };
        let dtypes: Vec<&str> = entries
            .iter()
            .map(|(_, v)| match &v.kind {
                Kind::Array { dtype, .. } => dtype.as_str(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(dtypes, vec!["|i1", "|i1"]);
        // The class and the letter separately: the dtype class out of its
        // slot and the byte order out of the slot the first array wrote it in.
        let apart = two_arrays(&cat(&[
            &get(14),
            &word("i1"),
            b"\x89\x88\x87\x94R\x94(K\x03",
            &get(18),
            b"NNNJ\xff\xff\xff\xffJ\xff\xff\xff\xffK\0t\x94b",
        ]));
        assert!(recognise(&framed(&apart)).is_some());
        // And the whole dtype written out again, which is what a file with two
        // unlike arrays does. Its module name may be a reference or a word.
        let again = two_arrays(&dtype_state(5, "i1", b'|'));
        assert!(recognise(&framed(&again)).is_some());
        let again = two_arrays(&dtype_state(9, "i1", b'|'));
        assert!(recognise(&framed(&again)).is_none(), "slot 9 holds a byte string, not a module name");
    }

    /// A reference is only ever to a slot the form itself filled with the
    /// thing the grammar expects to find there.
    #[test]
    fn a_reference_to_the_wrong_slot_is_a_non_match() {
        assert!(recognise(&framed(&two_arrays(&get(17)))).is_some());
        // Past the end of the memo, at a slot the file has not written yet,
        // and at slots holding the dictionary, a key, a global, a byte string
        // and the byte order: none of those is a dtype.
        for slot in [255u8, 30, 0, 1, 4, 9, 18] {
            assert!(recognise(&framed(&two_arrays(&get(slot)))).is_none(), "accepted a dtype at slot {slot}");
        }
        // The ndarray class where the reconstructor belongs, and the other way
        // round: both slots hold a global, and neither holds the right one.
        let swapped = cat(&[
            b"}\x94(",
            &word("a"),
            &reconstruct(),
            b"(K\x01K\x02\x85\x94",
            &dtype_state(5, "i1", b'|'),
            b"\x89",
            &blob(&[1, 2]),
            b"t\x94b",
            &word("b"),
            &get(7),
            &get(4),
            b"K\0\x85\x94",
            &get(9),
            b"\x87\x94R\x94(K\x01K\x03\x85\x94",
            &get(17),
            b"\x89",
            &blob(&[1, 2, 3]),
            b"t\x94bu.",
        ]);
        assert!(recognise(&framed(&swapped)).is_none());
        // LONG_BINGET reaches the same slots the long way round, which is what
        // a file with more than 256 of them has to do.
        let long = cat(&[
            b"}\x94(",
            &word("a"),
            &reconstruct(),
            b"(K\x01K\x02\x85\x94",
            &dtype_state(5, "i1", b'|'),
            b"\x89",
            &blob(&[1, 2]),
            b"t\x94b",
            &word("b"),
            b"j\x04\0\0\0j\x07\0\0\0K\0\x85\x94j\x09\0\0\0\x87\x94R\x94(K\x01K\x03\x85\x94j\x11\0\0\0\x89",
            &blob(&[1, 2, 3]),
            b"t\x94b",
            b"u.",
        ]);
        assert!(recognise(&framed(&long)).is_some());
    }

    /// A NumPy scalar is one number written as a call of its own.
    #[test]
    fn a_numpy_scalar_is_one_number_of_its_dtype() {
        let scalar = |data: &[u8]| {
            cat(&[
                b"}\x94(",
                &word("a"),
                &reconstruct(),
                b"(K\x01K\x02\x85\x94",
                &dtype_state(5, "i2", b'<'),
                b"\x89",
                &blob(&[1, 0, 2, 0]),
                b"t\x94b",
                &word("b"),
                &get(2),
                &word("scalar"),
                b"\x93\x94",
                &get(17),
                &blob(data),
                b"\x86\x94R\x94",
                b"u.",
            ])
        };
        let found = recognise(&framed(&scalar(&[7, 0]))).unwrap();
        let Kind::Dict(entries) = &found.value.kind else { panic!("dict") };
        let Kind::Array { dtype, dimensions, len, .. } = &entries[1].1.kind else { panic!("array") };
        assert_eq!((dtype.as_str(), dimensions.as_slice(), *len), ("<i2", &[][..], 2));
        assert_eq!(found.calls.len(), 2);
        assert_eq!(found.calls[1].name, "numpy scalar call");
        // One value of the dtype and no more: a scalar is not an array of one.
        assert!(recognise(&framed(&scalar(&[7, 0, 8, 0]))).is_none(), "two values in a scalar");
        assert!(recognise(&framed(&scalar(&[7]))).is_none(), "half a value in a scalar");
    }

    /// An instruction moved, dropped or added, a length written in another
    /// width, bytes after the STOP, and every truncation of the file.
    #[test]
    fn a_changed_instruction_leaves_no_match() {
        let body = two_arrays(&get(17));
        let whole = framed(&body);
        assert!(recognise(&whole).is_some());

        for end in 0..whole.len() {
            assert!(recognise(&whole[..end]).is_none(), "accepted {end} bytes of the file");
        }
        let mut after = whole.clone();
        after.push(b'N');
        assert!(recognise(&after).is_none(), "accepted a value after the STOP");

        // One instruction dropped: the memo mark that files the dictionary,
        // the TUPLE1 that closes the placeholder shape, and a BUILD.
        for cut in [0x94u8, 0x85, b'b'] {
            let at = body.iter().position(|b| *b == cut).unwrap();
            let mut short = body.clone();
            short.remove(at);
            assert!(recognise(&framed(&short)).is_none(), "accepted the file without {cut:#x}");
        }
        // One instruction inserted, beside every memo mark in the file.
        for at in 0..body.len() {
            if body[at] != 0x94 {
                continue;
            }
            let mut longer = body.clone();
            longer.insert(at, 0x94);
            assert!(recognise(&framed(&longer)).is_none(), "accepted an extra memo mark at {at}");
        }
        // Two instructions swapped: the flags a dtype is built with.
        let mut reordered = body.clone();
        let at = reordered.windows(2).position(|w| w == b"\x89\x88").unwrap();
        reordered.swap(at, at + 1);
        assert!(recognise(&framed(&reordered)).is_none(), "accepted NEWTRUE before NEWFALSE");

        // A length in another width. BINBYTES is an alternative of its own, so
        // it is matched; a length that no longer comes to the declared shape
        // is not.
        let at = body.windows(4).position(|w| w == b"C\x03\x01\x02").unwrap();
        let mut wide = body.clone();
        wide.splice(at..at + 2, [b'B', 3, 0, 0, 0]);
        assert!(recognise(&framed(&wide)).is_some());
        let mut mismatched = wide.clone();
        mismatched[at + 1] = 4;
        assert!(recognise(&framed(&mismatched)).is_none(), "a length past the shape it declared");
    }

    /// Frames, as CPython writes them: a run of them, with a payload of
    /// [`BIG_PAYLOAD`] bytes or more written between two rather than inside
    /// one.
    #[test]
    fn a_payload_too_large_to_frame_sits_between_frames() {
        let head = cat(&[b"}\x94(", &word("a"), b"K\x01", &word("big")]);
        let tail = cat(&[b"\x94", &word("c"), b"K\x02u."]);
        let carrying = |size: usize| {
            let mut out = vec![b'B'];
            out.extend_from_slice(&(size as u32).to_le_bytes());
            out.resize(out.len() + size, 0xa5);
            out
        };
        let build = |payload: &[u8]| {
            let mut out = vec![0x80, 4, 0x95];
            out.extend_from_slice(&(head.len() as u64).to_le_bytes());
            out.extend_from_slice(&head);
            out.extend_from_slice(payload);
            out.push(0x95);
            out.extend_from_slice(&(tail.len() as u64).to_le_bytes());
            out.extend_from_slice(&tail);
            out
        };
        let whole = build(&carrying(BIG_PAYLOAD));
        let found = recognise(&whole).unwrap();
        assert_eq!(found.form, "basic-p4-p5-v3");
        let Kind::Dict(entries) = &found.value.kind else { panic!("dict") };
        assert_eq!(entries.len(), 3);
        assert!(matches!(entries[1].1.kind, Kind::Bytes { len, .. } if len == BIG_PAYLOAD));

        // The first frame one byte short of the payload's opcode, so the
        // opcode would begin inside a frame rather than after one.
        let mut early = whole.clone();
        early[3] -= 1;
        assert!(recognise(&early).is_none(), "a frame that ends before the payload");
        // The whole body inside one frame, payload and all.
        let inside = cat(&[&head, &carrying(BIG_PAYLOAD), &tail]);
        let mut once = vec![0x80, 4, 0x95];
        once.extend_from_slice(&(inside.len() as u64).to_le_bytes());
        once.extend_from_slice(&inside);
        assert!(recognise(&once).is_none(), "a large payload inside a frame");
        // A payload one byte short of large, written between frames anyway.
        assert!(recognise(&build(&carrying(BIG_PAYLOAD - 1))).is_none(), "a small payload between frames");
        // No new frame after the payload.
        let mut headless = vec![0x80, 4, 0x95];
        headless.extend_from_slice(&(head.len() as u64).to_le_bytes());
        headless.extend_from_slice(&head);
        headless.extend_from_slice(&carrying(BIG_PAYLOAD));
        headless.extend_from_slice(&tail);
        assert!(recognise(&headless).is_none(), "no frame after the payload");
        // Truncation, since a frame's length is a claim about bytes that may
        // not have arrived.
        for end in [0, 3, 11, head.len() + 11, whole.len() - 1] {
            assert!(recognise(&whole[..end]).is_none(), "accepted {end} bytes");
        }

        // A frame that ends early with no large payload behind it. The short
        // unframed tail is only ever what CPython leaves after one of those,
        // so an empty frame followed by the whole object is a non-match, and
        // so is a frame that stops one value before the STOP.
        let mut empty = vec![0x80, 4, 0x95];
        empty.extend_from_slice(&0u64.to_le_bytes());
        empty.extend_from_slice(b"N.");
        assert!(recognise(&empty).is_none(), "a frame holding none of the object");
        let mut early_stop = vec![0x80, 4, 0x95];
        let body = cat(&[b"]\x94(K\x01K\x02e."]);
        early_stop.extend_from_slice(&((body.len() - 1) as u64).to_le_bytes());
        early_stop.extend_from_slice(&body);
        assert!(recognise(&early_stop).is_none(), "a frame that ends before the STOP");
    }

    /// The builtins a pickle writes as a call: what each one accepts, and what
    /// none of them do.
    #[test]
    fn the_builtin_calls_take_what_python_writes_and_nothing_else() {
        let call = |name: &str, args: &[u8], arity: u8| {
            cat(&[&word("builtins"), &word(name), b"\x93\x94", args, &[0x84 + arity, 0x94, b'R', 0x94]])
        };
        let one = |body: Vec<u8>| framed(&cat(&[&body, b"."]));
        let cases: Vec<(Vec<u8>, Shape)> = vec![
            (call("slice", b"K\x01K\x0aK\x02", 3), Shape::Slice),
            (call("slice", b"NNN", 3), Shape::Slice),
            (call("slice", b"M\x39\x30J\xff\xff\xff\xffK\x02", 3), Shape::Slice),
            (call("range", b"K\0K\x0aK\x02", 3), Shape::Range),
            (call("complex", b"G\x3f\xf8\0\0\0\0\0\0G\xc0\x04\0\0\0\0\0\0", 2), Shape::Complex),
            (b"(K\x01K\x02\x91\x94".to_vec(), Shape::FrozenSet),
            (b"(\x91\x94".to_vec(), Shape::FrozenSet),
            (call("bytearray", &blob(b"ab"), 1), Shape::ByteArray),
        ];
        for (body, want) in &cases {
            let found = recognise(&one(body.clone())).unwrap_or_else(|| panic!("{want:?} was not matched"));
            assert_eq!(found.form, "builtins-values-p4-p5-v1", "{want:?}");
            let Kind::Object { what, .. } = &found.value.kind else { panic!("{want:?} is not an object") };
            assert_eq!(what, want);
        }

        // The values themselves, read from the bytes they were written in.
        let found = recognise(&one(call("complex", b"G\x3f\xf8\0\0\0\0\0\0G\xc0\x04\0\0\0\0\0\0", 2))).unwrap();
        let Kind::Object { items, names, .. } = &found.value.kind else { panic!("object") };
        assert_eq!(*names, HALVES);
        let halves: Vec<f64> = items
            .iter()
            .map(|v| match v.kind {
                Kind::Float { value, .. } => value,
                _ => panic!("not a float"),
            })
            .collect();
        assert_eq!(halves, vec![1.5, -2.5]);

        // And what none of them take: another callable of the same module, the
        // wrong arity, a value of the wrong kind, and a module that is not
        // builtins.
        for body in [
            call("eval", b"K\x01K\x0aK\x02", 3),
            call("slice", b"K\x01K\x0a", 2),
            call("slice", b"K\x01K\x0aK\x02K\x03", 3),
            call("range", b"NK\x0aK\x02", 3),
            call("complex", b"K\x01K\x02", 2),
            call("bytearray", &word("ab"), 1),
            cat(&[&word("os"), &word("system"), b"\x93\x94)\x94R\x94"]),
        ] {
            assert!(recognise(&one(body.clone())).is_none(), "accepted {body:?}");
        }
    }

    /// A form is the productions it allows, and a file is read under exactly
    /// one of them.
    #[test]
    fn a_form_is_the_productions_it_allows() {
        assert_eq!(recognise(&framed(b"}\x94.")).unwrap().form, "basic-p4-p5-v3");
        let slice = cat(&[
            b"}\x94",
            &word("s"),
            &word("builtins"),
            &word("slice"),
            b"\x93\x94K\x01K\x02K\x03\x87\x94R\x94s.",
        ]);
        assert_eq!(recognise(&framed(&slice)).unwrap().form, "builtins-values-p4-p5-v1");
        let array = cat(&[b"}\x94", &word("a"), &one_array(2, "i1", b'|', b"K\x02\x85\x94", &[1, 2]), b"s."]);
        assert_eq!(recognise(&framed(&array)).unwrap().form, "numpy-numeric-array-p4-p5-v3");
        // A file holding both is read under neither: no form that allows both
        // has been reviewed.
        let both = cat(&[
            b"}\x94(",
            &word("a"),
            &one_array(2, "i1", b'|', b"K\x02\x85\x94", &[1, 2]),
            &word("s"),
            &word("builtins"),
            &word("slice"),
            b"\x93\x94K\x01K\x02K\x03\x87\x94R\x94u.",
        ]);
        assert!(recognise(&framed(&both)).is_none());
    }
}
