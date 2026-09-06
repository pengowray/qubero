//! Running a pickle far enough to say what its bytes are.
//!
//! The opcode listing places every byte and names every instruction, and for
//! most of a pickle that is the whole answer. It is not the answer for the
//! part a reader most wants: the payload. A numpy array's data is a
//! `BINBYTES` like any other, and nothing beside it says it holds 24
//! little-endian floats. What says so is the program:
//!
//! ```text
//! SHORT_BINUNICODE 'numpy._core.multiarray'   \
//! SHORT_BINUNICODE '_reconstruct'              > STACK_GLOBAL -> a callable
//! ...                                          > REDUCE       -> an ndarray
//! MARK  BININT1 1  (4, 6)  <dtype f4 '<'>  NEWFALSE  BINBYTES ...  TUPLE
//! BUILD                                        -> the array, filled in
//! ```
//!
//! The shape, the dtype, the byte order and the data are four items that meet
//! only on the stack. So this runs the stack.
//!
//! **It does not run the pickle.** Nothing is imported, nothing is called and
//! no object is built: unpickling is famously arbitrary code execution, and a
//! reader that did any of that would be the security hole rather than the tool
//! for looking at one. What is kept is a shape for each value -- a number, a
//! string, a tuple of these, a call to a named callable -- and where in the
//! file it was written. That is enough to recognise `_reconstruct` and its
//! arguments, and it is enough to say what a memo reference points at.
//!
//! It is also allowed to give up. A pickle whose stack does not balance, one
//! using an opcode this does not model, or one deeper or longer than the
//! bounds below, stops being annotated at that point and stays a listing of
//! opcodes. Nothing here can make a file unreadable.

use std::collections::HashMap;
use std::sync::Arc;

use super::known::{self, Payload};

/// How many values may be on the stack, and how many the memo may hold. A
/// pickle is written by a program that could be writing anything, so both are
/// bounded: past these the reading stops and the listing stands on its own.
const DEEPEST: usize = 4096;
const MOST_MEMO: usize = 1 << 20;

/// How many opcodes are run. Enough for a pickle of a large model's metadata,
/// and few enough that a file of nothing but opcodes cannot hold the thread.
const MOST_OPS: usize = 4 << 20;

/// A value the program pushed, as much of it as is worth keeping.
///
/// Everything here is either a number, a run of bytes the file holds, or a
/// shape built out of those. Nothing is evaluated: a `Call` is the fact that
/// `REDUCE` was reached with a callable and arguments, not the result of
/// calling anything.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    None,
    Bool(bool),
    Int(i128),
    Float(f64),
    /// Text, and where its bytes are in the file.
    Text(Arc<str>),
    /// A byte string, and where its bytes are: the offset of the first byte
    /// and how many there are. The bytes themselves are not copied, because
    /// this is exactly the case where they may be a gigabyte long.
    Bytes { at: u64, len: u64 },
    Tuple(Arc<[Value]>),
    /// A list, a set or a dict. What was appended to it after it was pushed is
    /// not tracked: nothing here needs it, and following `APPENDS` through a
    /// hundred thousand elements would cost what the file costs.
    Collection,
    /// A numpy dtype, spelled the way the `.npy` reader spells one: the byte
    /// order and then the kind and width, `<f4`. A value of its own because
    /// it takes two opcodes to make one -- a call to `numpy.dtype` naming the
    /// kind, and a `BUILD` giving it its byte order -- and no array can be
    /// typed until both have happened.
    Dtype(Arc<str>),
    /// A callable named by module and name, however it was named: `GLOBAL`
    /// writes the two as lines, `STACK_GLOBAL` takes them off the stack, and
    /// the extension registry writes a number this cannot resolve.
    Global { module: Arc<str>, name: Arc<str> },
    /// `callable(*args)`, unevaluated.
    Call { callable: Arc<Value>, args: Arc<[Value]> },
    /// Something this does not model: an extension code, a persistent id, an
    /// out-of-band buffer, an old-style class instance. It is a value and it
    /// has a place on the stack, and that is all that is claimed.
    Opaque,
}

impl Value {
    /// The text of a string value, for the recognisers, which compare module
    /// and dtype names.
    pub fn text(&self) -> Option<&str> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }

    pub fn int(&self) -> Option<i128> {
        match self {
            Value::Int(v) => Some(*v),
            Value::Bool(b) => Some(i128::from(*b)),
            _ => None,
        }
    }

    pub fn tuple(&self) -> Option<&[Value]> {
        match self {
            Value::Tuple(v) => Some(v),
            _ => None,
        }
    }

    /// `module.name` where this is a callable, for saying what a `REDUCE`
    /// calls.
    pub fn global(&self) -> Option<String> {
        match self {
            Value::Global { module, name } => Some(format!("{module}.{name}")),
            _ => None,
        }
    }
}

/// What running the pickle said about it, keyed by where in the file the
/// answer applies.
///
/// Kept apart rather than as one structure per opcode, because the two are
/// asked separately and most opcodes are in neither: a listing asks what a row
/// builds for every row it draws, and asks what a payload holds only for the
/// handful of rows holding one.
#[derive(Debug, Default)]
pub struct Reading {
    /// The byte offset of a byte string's first byte, and what it holds.
    pub arrays: HashMap<u64, Payload>,
    /// What an opcode does, in words, over the bytes the opcode covers.
    ///
    /// A range rather than the opcode's own offset, because the field asking
    /// is somewhere inside the opcode and not always at its front: a memo
    /// reference's word sits after the index, which is one byte wide or four.
    /// Sorted, so the answer is a binary search.
    pub builds: Vec<(u64, u64, String)>,
    /// Whether the run finished. A reading that gave up part way is still
    /// worth keeping: the opcodes before the trouble were read correctly.
    pub whole: bool,
    /// What was left on the stack. A pickle that ran properly leaves the one
    /// object it built, so anything else says the model of some opcode's
    /// stack effect is wrong. Nothing reads it but the tests, which is the
    /// point: it is the check `pickletools.dis` does and a listing cannot.
    pub left: usize,
}

impl Reading {
    /// What the opcode covering `at` does, in words. Nothing where the opcode
    /// there had nothing to say, which is most of them.
    pub fn builds_at(&self, at: u64) -> Option<&str> {
        let i = self.builds.partition_point(|(start, _, _)| *start <= at).checked_sub(1)?;
        let (_, end, word) = &self.builds[i];
        (at < *end).then_some(word.as_str())
    }
}

/// One opcode of the file, as the caller found it: where it is, where its
/// value is, and what that value holds.
///
/// Handed in rather than parsed again here, so there is one place that knows
/// how wide an operand is and the machine cannot drift from the listing.
#[derive(Debug, Clone)]
pub struct Op {
    pub code: u8,
    /// Where the opcode byte is, in bytes from the start of the file, and
    /// where the opcode after it starts.
    pub at: u64,
    pub end: u64,
    /// Where the value is and how long it is, with the length that measured it
    /// left out: a `SHORT_BINBYTES` of five bytes has one byte of length and
    /// then the five, and only the five are the value.
    pub data_at: u64,
    pub data_len: u64,
    /// The value's bytes, for the values worth having: a name, a dtype, a line
    /// of digits. Empty for a payload, which may be enormous and is never read
    /// as a value here.
    pub operand: Vec<u8>,
}

/// Run the opcodes and say what they build.
pub fn run(ops: &[Op]) -> Reading {
    let mut m = Run::default();
    for op in ops.iter().take(MOST_OPS) {
        if m.step(op).is_none() {
            m.settle();
            return m.out;
        }
    }
    m.out.whole = ops.len() <= MOST_OPS;
    m.settle();
    m.out
}

#[derive(Default)]
struct Run {
    stack: Vec<Value>,
    /// Where the open `MARK`s are in the stack.
    marks: Vec<usize>,
    memo: HashMap<u64, Value>,
    /// What the memo will be given next, which `MEMOIZE` needs: protocol 4
    /// writes no index and means "the next one".
    next_memo: u64,
    out: Reading,
}

impl Run {
    /// One opcode. `None` gives up on the rest of the file, which happens when
    /// the stack does not have what an opcode needs: that is a pickle this
    /// does not understand, and understanding it is optional.
    fn step(&mut self, op: &Op) -> Option<()> {
        if self.stack.len() > DEEPEST || self.memo.len() > MOST_MEMO {
            return None;
        }
        let text = || Arc::<str>::from(String::from_utf8_lossy(&op.operand).into_owned());
        match op.code {
            // Values that are their own operand.
            0x4e => self.push(Value::None),                                  // NONE
            0x88 => self.push(Value::Bool(true)),                            // NEWTRUE
            0x89 => self.push(Value::Bool(false)),                           // NEWFALSE
            0x4b | 0x4d => self.push(Value::Int(le(&op.operand) as i128)),   // BININT1, BININT2
            0x4a => self.push(Value::Int(le(&op.operand) as i32 as i128)),   // BININT
            0x47 => self.push(float(&op.operand)),                           // BINFLOAT
            0x49 | 0x4c => self.push(digits(&op.operand)),                   // INT, LONG
            0x46 => self.push(Value::Float(0.0)),                            // FLOAT
            0x8a | 0x8b => self.push(Value::Opaque),                         // LONG1, LONG4
            // Text, whichever way its length was written.
            0x53 | 0x56 | 0x54 | 0x55 | 0x58 | 0x8c | 0x8d => self.push(Value::Text(text())),
            // Bytes, which are the payload and are never copied.
            0x42 | 0x43 | 0x8e | 0x96 => {
                self.push(Value::Bytes { at: op.data_at, len: op.data_len })
            }
            // Empty containers.
            0x29 => self.push(Value::Tuple(Arc::from(&[][..]))),             // EMPTY_TUPLE
            0x5d | 0x7d | 0x8f => self.push(Value::Collection),              // EMPTY_LIST, DICT, SET

            0x28 => self.marks.push(self.stack.len()),                       // MARK
            0x31 => self.to_mark().map(|_| ())?,                             // POP_MARK
            0x30 => drop(self.stack.pop()?),                                 // POP
            0x32 => self.push(self.stack.last()?.clone()),                   // DUP

            // Tuples, at the four ways of writing one.
            0x85 | 0x86 | 0x87 => {
                let n = (op.code - 0x84) as usize;
                let at = self.stack.len().checked_sub(n)?;
                let items: Arc<[Value]> = Arc::from(self.stack.split_off(at));
                self.push(Value::Tuple(items));
            }
            0x74 => {
                let items: Arc<[Value]> = Arc::from(self.to_mark()?);
                self.push(Value::Tuple(items));
            }
            // Building a list or a dict from a mark, and everything that adds
            // to one already there. What ends up inside is not tracked.
            0x6c | 0x64 | 0x91 => {
                self.to_mark()?;
                self.push(Value::Collection);
            }
            0x65 | 0x75 | 0x90 => {
                self.to_mark()?;
            }
            0x61 | 0x73 => {
                self.stack.pop()?;
                if op.code == 0x73 {
                    self.stack.pop()?;
                }
            }

            // Naming a callable, the two ways.
            0x63 | 0x69 => {
                let (module, name) = pair(&op.operand);
                let named = Value::Global { module, name };
                if let Some(full) = named.global() {
                    if let Some(word) = known::what(&full) {
                        self.say(op, word.to_string());
                    }
                }
                match op.code {
                    // INST names a class and calls it in one opcode, on the
                    // arguments back to the mark. The mark is taken first: the
                    // class is the opcode's own operand and was never on the
                    // stack, so pushing it before closing the mark would put
                    // it among its own arguments.
                    0x69 => {
                        let args: Arc<[Value]> = Arc::from(self.to_mark()?);
                        self.note(op, &named, &args);
                        self.push(Value::Call { callable: Arc::new(named), args });
                    }
                    _ => self.push(named),
                }
            }
            0x93 => {
                let name = self.stack.pop()?;
                let module = self.stack.pop()?;
                let (Some(module), Some(name)) = (module.text(), name.text()) else {
                    self.push(Value::Opaque);
                    return Some(());
                };
                // The two strings are already rows above this one, but they
                // are rows apart and neither says it is half of a name. Where
                // the name is one anybody knows, saying what it is beats
                // saying it again.
                let full = format!("{module}.{name}");
                self.say(op, known::what(&full).map_or(format!("names {full}"), str::to_string));
                self.push(Value::Global { module: module.into(), name: name.into() });
            }

            // Calling one.
            0x52 => {
                let args = self.stack.pop()?;
                let callable = Arc::new(self.stack.pop()?);
                let args: Arc<[Value]> = match args.tuple() {
                    Some(items) => Arc::from(items),
                    None => Arc::from(&[][..]),
                };
                self.note(op, &callable, &args);
                self.push(Value::Call { callable, args });
            }
            0x81 | 0x92 => {
                // NEWOBJ takes a class and a tuple; NEWOBJ_EX a class, a tuple
                // and a dict. Both come to the same thing here.
                if op.code == 0x92 {
                    self.stack.pop()?;
                }
                let args = self.stack.pop()?;
                let callable = Arc::new(self.stack.pop()?);
                let args: Arc<[Value]> = match args.tuple() {
                    Some(items) => Arc::from(items),
                    None => Arc::from(&[][..]),
                };
                self.note(op, &callable, &args);
                self.push(Value::Call { callable, args });
            }
            // OBJ takes the class from *inside* the mark: the mark is opened,
            // the class pushed, then the arguments. So it is the first of what
            // the mark gives back, not the item under the mark.
            0x6f => {
                let mut items = self.to_mark()?;
                if items.is_empty() {
                    return None;
                }
                let callable = Arc::new(items.remove(0));
                let args: Arc<[Value]> = Arc::from(items);
                self.note(op, &callable, &args);
                self.push(Value::Call { callable, args });
            }
            // Filling an object in, which is where an ndarray's shape, dtype
            // and data arrive. The object stays on the stack.
            0x62 => {
                let state = self.stack.pop()?;
                let object = self.stack.last()?.clone();
                self.build(op, &object, &state);
            }

            // The memo, written and read.
            0x94 => {
                let v = self.stack.last()?.clone();
                self.memo.insert(self.next_memo, v);
                self.next_memo += 1;
            }
            0x70 | 0x71 | 0x72 => {
                let key = match op.code {
                    0x70 => digits(&op.operand).int().unwrap_or(-1),
                    _ => le(&op.operand) as i128,
                };
                let key = u64::try_from(key).ok()?;
                self.memo.insert(key, self.stack.last()?.clone());
                self.next_memo = self.next_memo.max(key + 1);
            }
            0x67 | 0x68 | 0x6a => {
                let key = match op.code {
                    0x67 => digits(&op.operand).int().unwrap_or(-1),
                    _ => le(&op.operand) as i128,
                };
                let v = u64::try_from(key).ok().and_then(|k| self.memo.get(&k)).cloned();
                if let Some(v) = &v {
                    if let Some(word) = describe(v) {
                        self.say(op, word);
                    }
                }
                self.push(v.unwrap_or(Value::Opaque));
            }

            // A persistent id: the pickle names an object instead of writing
            // it. `PERSID` has the name in its own operand; `BINPERSID` takes
            // it off the stack, so it replaces a value rather than adding one.
            0x51 => {
                self.stack.pop()?;
                self.push(Value::Opaque);
            }
            // Everything left is a value this does not model, or an
            // instruction that changes nothing it tracks.
            0x50 | 0x82 | 0x83 | 0x84 | 0x97 => self.push(Value::Opaque),
            0x98 => {}
            0x80 | 0x95 | 0x2e => {}
            _ => return None,
        }
        Some(())
    }

    fn push(&mut self, v: Value) {
        self.stack.push(v);
    }

    /// Note what an opcode does, over the bytes that opcode covers.
    fn say(&mut self, op: &Op, word: String) {
        self.out.builds.push((op.at, op.end, word));
    }

    /// Put the words in the order a lookup expects. They are not written in
    /// it: an array's data is recognised by an opcode a long way after the one
    /// that wrote it, so the word for the earlier bytes arrives last.
    fn settle(&mut self) {
        self.out.builds.sort_by_key(|(start, end, _)| (*start, *end));
        self.out.builds.dedup_by_key(|(start, _, _)| *start);
        self.out.left = self.stack.len();
    }

    /// Everything above the innermost open mark, with the mark closed.
    fn to_mark(&mut self) -> Option<Vec<Value>> {
        let at = self.marks.pop()?;
        (at <= self.stack.len()).then(|| self.stack.split_off(at))
    }

    /// What a call is, in words, and whether its arguments describe an array.
    fn note(&mut self, op: &Op, callable: &Value, args: &[Value]) {
        if let Some(name) = callable.global() {
            // What it is where anybody knows, and what it is called where
            // nobody does. Never both: the module and the name are already
            // two rows above this one.
            self.say(op, known::what(&name).map_or_else(|| format!("calls {name}"), str::to_string));
        }
        self.record(callable, args);
    }

    /// What a `BUILD` fills in. The object is a call this has already seen, so
    /// the recogniser is handed the same pair it would have got from `REDUCE`:
    /// what was called, and what it is being given.
    fn build(&mut self, op: &Op, object: &Value, state: &Value) {
        // A dtype is finished by its own `BUILD`: the call said `f4` and this
        // says which way round it reads. Both halves are needed before an
        // array can be typed, so what is on the stack becomes the finished
        // dtype rather than the call that started it.
        if let Some(descr) = finished_dtype(object, state) {
            let done = Value::Dtype(descr.as_str().into());
            if let Some(top) = self.stack.last_mut() {
                *top = done.clone();
            }
            // The dtype was memoised before it was finished, because
            // `MEMOIZE` comes between the call and the `BUILD`. A second array
            // of the same dtype takes it out of the memo, so what is in there
            // has to be the finished one or that array reads as bytes. Only
            // the entries holding this very call are touched.
            for held in self.memo.values_mut() {
                if held == object {
                    *held = done.clone();
                }
            }
            self.say(op, format!("the dtype {descr}"));
            return;
        }
        if let Value::Call { callable, .. } = object {
            if let Some(name) = callable.global() {
                let what = known::what(&name).unwrap_or(&name).to_string();
                self.say(op, format!("fills in {what}"));
            }
            let items = state.tuple().unwrap_or(std::slice::from_ref(state));
            self.record(callable, items);
        }
    }

    /// Ask the recognisers what this call makes of these arguments, and note
    /// where the answer's bytes are.
    fn record(&mut self, callable: &Value, args: &[Value]) {
        let Some(name) = callable.global() else { return };
        let Some((at, len, payload, what)) = known::recognise(&name, args) else { return };
        self.out.arrays.insert(at, payload);
        // Filed against the bytes rather than against the opcode that
        // recognised them: the data is somewhere earlier in the file, and it
        // is the row a reader will be looking at.
        self.out.builds.push((at, at + len, what));
    }
}

/// The dtype a `BUILD` finishes, where finishing one is what it is doing.
///
/// numpy pickles a dtype as `numpy.dtype('f4', False, True)` and then fills
/// the rest in with a state tuple whose second item is the byte order. `=`
/// means the writer's own, which numpy settles before it writes, so a file
/// holding one states no byte order at all: it reads as little-endian, which
/// is what every machine writing these has been for twenty years.
fn finished_dtype(object: &Value, state: &Value) -> Option<String> {
    let Value::Call { callable, args } = object else { return None };
    if callable.global().as_deref() != Some("numpy.dtype") {
        return None;
    }
    let kind = args.first()?.text()?;
    let order = match state.tuple()?.get(1)?.text()? {
        "=" => "<",
        settled @ ("<" | ">" | "|") => settled,
        _ => return None,
    };
    Some(format!("{order}{kind}"))
}

/// A little-endian number out of however many bytes there are, up to eight.
fn le(bytes: &[u8]) -> u64 {
    let mut wide = [0u8; 8];
    let n = bytes.len().min(8);
    wide[..n].copy_from_slice(&bytes[..n]);
    u64::from_le_bytes(wide)
}

fn float(bytes: &[u8]) -> Value {
    match bytes.try_into() {
        Ok(eight) => Value::Float(f64::from_be_bytes(eight)),
        Err(_) => Value::Opaque,
    }
}

/// A number written as digits, with the newline the format ends it with and
/// the `L` an old long integer carries taken off.
fn digits(bytes: &[u8]) -> Value {
    let s = String::from_utf8_lossy(bytes);
    let s = s.trim_end_matches('\n').trim_end_matches('L');
    match s.parse::<i128>() {
        // `I01` and `I00` are how protocol 0 writes True and False, and they
        // parse as 1 and 0, which is the same number either way.
        Ok(v) => Value::Int(v),
        Err(_) => Value::Opaque,
    }
}

/// The two lines `GLOBAL` and `INST` write: a module and a name.
fn pair(bytes: &[u8]) -> (Arc<str>, Arc<str>) {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = text.split('\n');
    let module = lines.next().unwrap_or("").trim_end_matches('\r');
    let name = lines.next().unwrap_or("").trim_end_matches('\r');
    (module.into(), name.into())
}

/// What a memo reference points at, said briefly enough for a row.
fn describe(v: &Value) -> Option<String> {
    Some(match v {
        Value::Text(s) if s.chars().count() <= 48 => format!("the string {s:?}"),
        Value::Int(n) => format!("the number {n}"),
        Value::Global { module, name } => format!("{module}.{name}"),
        Value::Call { callable, .. } => match callable.global() {
            Some(name) => format!("what {name} returned"),
            None => "an object".to_string(),
        },
        Value::Tuple(items) => format!("a tuple of {}", items.len()),
        Value::Collection => "a list, set or dict".to_string(),
        Value::Bytes { len, .. } => format!("{len} bytes"),
        // Longer than a row has room for, so the row says what it is and
        // leaves the reading of it to the bytes underneath.
        Value::Text(_) => "a string".to_string(),
        Value::Dtype(descr) => format!("the dtype {descr}"),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ops(bytes: &[u8]) -> Vec<Op> {
        super::super::opcodes(bytes)
    }

    #[test]
    fn a_tuple_closes_the_mark_it_opened() {
        // MARK, 1, 2, TUPLE, STOP
        let r = run(&ops(b"(K\x01K\x02t."));
        assert!(r.whole);
        assert_eq!(r.left, 1);
    }

    /// A pickle that ran properly leaves one object on the stack, and every
    /// opcode's stack effect has to be modelled right for that to come out.
    /// The opcodes with the awkward ones are all in the hand-written sample:
    /// `OBJ` takes its class from inside the mark, `INST` names its own and
    /// takes the mark first, and `BINPERSID` replaces a value rather than
    /// adding one.
    #[test]
    fn the_stack_balances_on_the_awkward_opcodes() {
        let handmade = b"(S'quoted'\nU\x05shortT\x04\x00\x00\x00four20\
                         (c__main__\nOldStyle\nK\x01o\
                         (i__main__\nOther\n(K\x021t.";
        let r = run(&ops(handmade));
        assert!(r.whole, "the run gave up");
        assert_eq!(r.left, 1, "a pickle leaves the one object it built");
    }

    /// A persistent id names an object rather than writing it, and takes the
    /// name off the stack. Leaking a slot per id would put every later opcode
    /// in a `torch.save` archive one out.
    #[test]
    fn a_persistent_id_replaces_the_name_it_used() {
        // 'a' BINPERSID 'b' BINPERSID TUPLE2 STOP
        let r = run(&ops(b"\x8c\x01aQ\x8c\x01bQ\x86."));
        assert!(r.whole);
        assert_eq!(r.left, 1);
    }

    #[test]
    fn a_pickle_that_does_not_balance_stops_rather_than_failing() {
        // TUPLE with no mark open.
        let r = run(&ops(b"K\x01t."));
        assert!(!r.whole, "the run should have given up");
        assert!(r.arrays.is_empty());
    }

    #[test]
    fn a_memo_reference_says_what_it_points_at() {
        // 'hi' PUT 0 POP GET 0 STOP, at protocol 0.
        let r = run(&ops(b"Vhi\np0\n0g0\n."));
        let word = r.builds.first().map(|(_, _, w)| w.clone()).unwrap_or_default();
        assert!(word.contains("\"hi\""), "said {word:?}");
    }
}
