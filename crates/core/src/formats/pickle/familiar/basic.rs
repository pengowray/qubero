//! The value productions every form allows: the stack a pickle is read
//! against, tuples, lists, dictionaries, sets, integers, floats, text, byte
//! strings and bytearrays. A form that allows more than these adds its
//! productions in a file beside this one.

use super::cursor::{Cursor, Framing};
use super::memo::Bound;
use super::{Kind, Names, Pickler, Shape, Value, CONTENT, MAX_BATCH, MAX_DEPTH, MAX_VALUES, NO_OPCODE};

/// A MARK, and what it holds back: where the opcode itself is, so that a
/// tuple or a frozenset made over it spans from there, and how tall the
/// stack was when it was written, which is what the closing opcode folds
/// back to.
#[derive(Debug, Clone, Copy)]
struct Mark {
    at: usize,
    floor: usize,
}

/// One thing the run has pushed, and what may still be done to it.
#[derive(Debug)]
struct Slot {
    value: Value,
    /// How deep the tree under this value goes. Nesting is bounded by this
    /// rather than by recursion, since the run does not recurse.
    deep: usize,
    fill: Fill,
}

/// How much of a container the file has written into it.
///
/// CPython creates a list, a dictionary or a set empty and fills it with the
/// opcodes after it: APPEND or SETITEM for a container holding exactly one
/// thing, and otherwise a run of batches of a thousand, only the last of
/// which is short. So which opcode may come next depends on what came before,
/// and this is that.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Fill {
    /// Not a container the file fills, or one that APPEND or SETITEM has
    /// filled and closed.
    Shut,
    /// A container with nothing in it yet.
    Open,
    /// A container filled by batches, the last of them this long.
    Batched(usize),
}

/// Whether this opcode begins an object.
///
/// CPython's framer ends a frame where the next object begins and nowhere
/// else, so this is the list of places a frame boundary may fall. The
/// opcodes that fold what is already on the stack are not on it.
fn opens_object(code: u8) -> bool {
    matches!(
        code,
        b'N' | 0x88 | 0x89 | b'K' | b'M' | b'J' | 0x8a | b'G' | 0x8c | 0x58 | 0x8d | b'C' | b'B' | 0x8e | 0x96 | b')' | b']' | b'}' | 0x8f | b'h' | b'j' | b'('
    )
}

/// A run of little-endian two's-complement bytes, as the number it spells.
///
/// LONG1 declares a length of up to 255, which reaches numbers no integer
/// type here holds. Sixteen bytes is where that stops, so a longer one is a
/// non-match rather than a number read wrong.
fn two_complement(bytes: &[u8]) -> Option<i128> {
    if bytes.is_empty() || bytes.len() > 16 {
        return None;
    }
    let mut value: i128 = if bytes[bytes.len() - 1] & 0x80 == 0 { 0 } else { -1 };
    for byte in bytes.iter().rev() {
        value = (value << 8) | i128::from(*byte);
    }
    Some(value)
}

/// How many two's-complement bytes a number needs, which is how many CPython
/// writes: it takes one more byte than the magnitude's bits fill and drops it
/// again when the sign bits in it are redundant.
fn shortest(value: i128) -> usize {
    (1..16)
        .find(|len| {
            let bits = len * 8 - 1;
            value >= -(1i128 << bits) && value < (1i128 << bits)
        })
        .unwrap_or(16)
}

/// Whether a value may be a dictionary key or a set member.
///
/// Python hashes numbers, strings, byte strings, tuples of hashable things,
/// frozensets and the three singletons, and nothing else a form builds. A key
/// of any other kind is not something CPython could have been asked to write.
fn hashable(value: &Value) -> bool {
    match &value.kind {
        Kind::None | Kind::Bool(_) | Kind::Int { .. } | Kind::Float { .. } => true,
        Kind::Text { .. } | Kind::Bytes { .. } => true,
        // A name stands for whatever the slot holds, which hashes or does
        // not for the same reasons the thing itself does.
        Kind::Ref(names) => match names {
            Names::Text { .. } | Names::Bytes { .. } => true,
            Names::Made { hashable, .. } => *hashable,
        },
        Kind::Tuple(items) | Kind::FrozenSet(items) => items.iter().all(hashable),
        // A NumPy scalar hashes; an array does not.
        Kind::Array { dimensions, .. } => dimensions.is_empty(),
        Kind::Object { what, items, .. } => {
            matches!(what, Shape::Slice | Shape::Range | Shape::Complex) && items.iter().all(hashable)
        }
        // A class hashes in Python and an object of one usually does, but no
        // file in the corpus writes either as a key, so neither is read as
        // one until something does.
        Kind::List(_) | Kind::Set(_) | Kind::Dict(_) => false,
        Kind::Class { .. } | Kind::Instance { .. } | Kind::Made { .. } | Kind::Objects { .. } | Kind::DType(_) => false,
    }
}

impl Cursor<'_> {
    pub(super) fn text(&mut self) -> Option<Value> {
        self.gate()?;
        let start = self.at;
        let code = self.byte()?;
        let (at, len) = self.counted(code, 0x8c, 0x58, 0x8d)?;
        std::str::from_utf8(self.bytes.get(at..at + len)?).ok()?;
        self.memoize(Bound::Text { at, len })?;
        Some(self.span(start, Kind::Text { at, len }))
    }

    /// The one object the file holds, read as the run of stack pushes a
    /// pickle writes it as.
    ///
    /// A pickle builds postfix. A tuple's elements are written before the
    /// opcode that makes the tuple out of them, and a list is created empty
    /// and filled by the opcodes that come after it, so a value is not a run
    /// of bytes that a reader can descend into. The productions are therefore
    /// read against one bounded stack rather than one value at a time.
    ///
    /// This is not a general stack machine. Every opcode here is one a form
    /// enumerated; each checks that what it is folding is what CPython writes
    /// in front of it, down to the length of a batch and the memo mark after
    /// it; and anything else ends the match. Nothing is interpreted, and no
    /// opcode is skipped.
    pub(super) fn object(&mut self) -> Option<Value> {
        let mut stack: Vec<Slot> = Vec::new();
        let mut marks: Vec<Mark> = Vec::new();
        loop {
            self.left = self.left.checked_sub(1)?;
            let mut code = self.peek()?;
            if code == 0x95 || opens_object(code) {
                // A frame may end here, and the next one begin.
                self.gate()?;
                code = self.peek()?;
                if !opens_object(code) {
                    return None;
                }
            } else if let Framing::Inside(end) | Framing::Full(end) = self.framing {
                // Everything else continues an object already begun, so the
                // frame it is in has to reach past it.
                if self.at >= end {
                    return None;
                }
            }
            if code == b'.' {
                break;
            }
            let start = self.at;
            let floor = marks.last().map_or(0, |mark| mark.floor);
            match code {
                b'(' => {
                    if marks.len() >= MAX_DEPTH {
                        return None;
                    }
                    self.byte()?;
                    marks.push(Mark { at: start, floor: stack.len() });
                }
                // TUPLE1, TUPLE2 and TUPLE3, which take the one to three
                // things above them. A tuple of four or more is written over
                // a MARK instead, and an empty one has an opcode of its own.
                0x85..=0x87 => {
                    let arity = (code - 0x84) as usize;
                    self.byte()?;
                    if stack.len() < floor + arity {
                        return None;
                    }
                    let items = stack.split_off(stack.len() - arity);
                    let at = items[0].value.at;
                    let holds = items.iter().all(|slot| hashable(&slot.value));
                    self.memoize(Bound::Made { what: Shape::Tuple, at, hashable: holds })?;
                    stack.push(self.folded(at, items, Kind::Tuple)?);
                }
                // TUPLE and FROZENSET, each over everything since its MARK.
                b't' | 0x91 => {
                    self.byte()?;
                    let mark = marks.pop()?;
                    let items = stack.split_off(mark.floor);
                    if code == b't' && items.len() < 4 {
                        // Three or fewer are written with TUPLE1 to TUPLE3.
                        return None;
                    }
                    let holds = items.iter().all(|slot| hashable(&slot.value));
                    if code == 0x91 && !holds {
                        return None;
                    }
                    let what = if code == b't' { Shape::Tuple } else { Shape::FrozenSet };
                    self.memoize(Bound::Made { what, at: mark.at, hashable: holds })?;
                    let make = if code == b't' { Kind::Tuple } else { Kind::FrozenSet };
                    stack.push(self.folded(mark.at, items, make)?);
                }
                // The opcodes that fill a container the file made empty.
                b'a' | b's' => {
                    self.byte()?;
                    let wants = if code == b'a' { 1 } else { 2 };
                    if stack.len() < floor + wants + 1 {
                        return None;
                    }
                    let items = stack.split_off(stack.len() - wants);
                    self.one(&mut stack, items)?;
                }
                b'e' | b'u' | 0x90 => {
                    self.byte()?;
                    let mark = marks.pop()?;
                    if mark.floor == 0 {
                        return None;
                    }
                    let items = stack.split_off(mark.floor);
                    self.batch(&mut stack, items, code)?;
                }
                // STACK_GLOBAL, NEWOBJ, REDUCE and BUILD, which only a form
                // that reads a library object allows. Each folds exactly the
                // two things written in front of it, in the same order: the
                // thing being named, called or filled first, and what it is
                // named, called or filled with second.
                0x93 | 0x81 | b'R' | b'b' if !self.allow.classes.is_empty() => {
                    self.byte()?;
                    if stack.len() < floor + 2 {
                        return None;
                    }
                    let items = stack.split_off(stack.len() - 2);
                    let at = items[0].value.at;
                    self.shut(&items)?;
                    let deep = 1 + items.iter().map(|slot| slot.deep).max().unwrap_or(0);
                    if deep > MAX_DEPTH {
                        return None;
                    }
                    let values = items.into_iter().map(|slot| slot.value).collect();
                    let kind = self.library(code, at, values)?;
                    stack.push(Slot { value: Value { at, len: self.at - at, kind }, deep, fill: Fill::Shut });
                }
                _ => {
                    if stack.len() >= MAX_VALUES {
                        return None;
                    }
                    let slot = self.push(code)?;
                    stack.push(slot);
                }
            }
        }
        if stack.len() != 1 || !marks.is_empty() {
            return None;
        }
        let whole = stack.pop()?;
        self.shut(std::slice::from_ref(&whole))?;
        Some(whole.value)
    }

    /// Every value here has been taken off the stack and is finished, so a
    /// container among them says how its writer ended it.
    ///
    /// A dictionary's and a set's loop runs again whenever the batch it wrote
    /// was full, so the C pickler always follows a full batch with another,
    /// empty when there was nothing left. `pickle.py` stops instead, and a
    /// container that ends on a full batch is that pickler's spelling.
    fn shut(&mut self, items: &[Slot]) -> Option<()> {
        for slot in items {
            if slot.fill == Fill::Batched(MAX_BATCH) && !matches!(slot.value.kind, Kind::List(_)) {
                self.wrote(Pickler::Python)?;
            }
        }
        Some(())
    }

    /// One value made out of the things just taken off the stack, spanning
    /// from `at` to wherever the file has got to.
    fn folded(&mut self, at: usize, items: Vec<Slot>, make: fn(Vec<Value>) -> Kind) -> Option<Slot> {
        self.shut(&items)?;
        let deep = 1 + items.iter().map(|slot| slot.deep).max().unwrap_or(0);
        if deep > MAX_DEPTH {
            return None;
        }
        let values = items.into_iter().map(|slot| slot.value).collect();
        Some(Slot {
            value: Value { at, len: self.at - at, kind: make(values) },
            deep,
            fill: Fill::Shut,
        })
    }

    /// APPEND or SETITEM: the shorthand for a list of one item or a
    /// dictionary of one entry, and for the one item `pickle.py` has left
    /// over after a full batch.
    ///
    /// Both picklers write it for a container holding exactly one thing.
    /// `pickle.py` also writes it for the last item of a container a batch
    /// longer than a multiple of a thousand, where the C pickler writes a
    /// batch of one instead. A container filled any other way does not take
    /// one.
    fn one(&mut self, stack: &mut [Slot], items: Vec<Slot>) -> Option<()> {
        self.shut(&items)?;
        let deep = 1 + items.iter().map(|slot| slot.deep).max().unwrap_or(0);
        let mut items = items.into_iter();
        let into = stack.last_mut()?;
        if deep > MAX_DEPTH {
            return None;
        }
        match into.fill {
            Fill::Open => {}
            Fill::Batched(MAX_BATCH) => self.wrote(Pickler::Python)?,
            _ => return None,
        }
        match &mut into.value.kind {
            Kind::List(values) => values.push(items.next()?.value),
            Kind::Dict(entries) => {
                let key = items.next()?.value;
                if !hashable(&key) {
                    return None;
                }
                entries.push((key, items.next()?.value));
            }
            _ => return None,
        }
        into.deep = into.deep.max(deep);
        into.value.len = self.at - into.value.at;
        into.fill = Fill::Shut;
        Some(())
    }

    /// APPENDS, SETITEMS or ADDITEMS: one batch of what a container holds.
    ///
    /// CPython writes a thousand entries, closes the batch and opens another,
    /// so every batch but the last is exactly that long. A dictionary and a
    /// set are written by a loop that runs again whenever the batch it just
    /// wrote was full, so a full batch of theirs is followed by another; a
    /// list's loop stops when it runs out, so a full batch of a list may be
    /// its last.
    ///
    /// What the two picklers do with the tail is where they part. The C one
    /// writes a batch for whatever is left over, even when that is nothing;
    /// `pickle.py` writes APPEND or SETITEM for a single item left over, and
    /// nothing at all for none. A set is the exception: neither has a
    /// shorthand for it, so both write a batch of one.
    fn batch(&mut self, stack: &mut [Slot], items: Vec<Slot>, code: u8) -> Option<()> {
        self.shut(&items)?;
        let deep = 1 + items.iter().map(|slot| slot.deep).max().unwrap_or(0);
        let into = stack.last_mut()?;
        if deep > MAX_DEPTH {
            return None;
        }
        let taken = items.len();
        let mut items = items.into_iter();
        let entries = match (code, &mut into.value.kind) {
            (b'e', Kind::List(values)) => {
                values.extend(items.map(|slot| slot.value));
                taken
            }
            (0x90, Kind::Set(values)) => {
                let members: Vec<Value> = items.map(|slot| slot.value).collect();
                if !members.iter().all(hashable) {
                    return None;
                }
                values.extend(members);
                taken
            }
            (b'u', Kind::Dict(pairs)) => {
                if taken % 2 != 0 {
                    return None;
                }
                while let (Some(key), Some(value)) = (items.next(), items.next()) {
                    if !hashable(&key.value) {
                        return None;
                    }
                    pairs.push((key.value, value.value));
                }
                taken / 2
            }
            _ => return None,
        };
        // A first batch is as long as the container, up to a thousand. A list
        // or a dictionary holding one thing is written with APPEND or SETITEM
        // instead, so a first batch of one is only ever a set's. A later
        // batch may be empty, which is what a dictionary or a set whose
        // length is a multiple of a thousand ends with; a list's never is.
        let least = if code == 0x90 { 1 } else { 2 };
        let fits = match into.fill {
            Fill::Open => (least..=MAX_BATCH).contains(&entries),
            Fill::Batched(before) => before == MAX_BATCH && entries <= MAX_BATCH && (entries > 0 || code != b'e'),
            Fill::Shut => false,
        };
        if !fits {
            return None;
        }
        // A batch after a full one is the tail, and says which pickler wrote
        // it. An empty one, or one holding the single item a list or a
        // dictionary had left over, is the C pickler's.
        if matches!(into.fill, Fill::Batched(_)) && matches!((entries, code), (0, _) | (1, b'e' | b'u')) {
            self.wrote(Pickler::C)?;
        }
        into.deep = into.deep.max(deep);
        into.value.len = self.at - into.value.at;
        into.fill = Fill::Batched(entries);
        Some(())
    }

    /// One thing pushed on the stack: a value written out, a value the file
    /// named out of the memo, or a container created empty.
    fn push(&mut self, code: u8) -> Option<Slot> {
        // The productions a form adds to the basic ones. Both begin with the
        // module name of the callable they name, spelled out or named out of
        // the memo, so they are tried where a string or a reference would be.
        // Each is tried whole and rewound whole, and the work budget is spent
        // either way.
        if matches!(code, 0x8c | 0x58 | 0x8d | b'h' | b'j') {
            if self.allow.numpy {
                let here = self.save();
                match self.numpy() {
                    Some(value) => return Some(Slot { value, deep: 1, fill: Fill::Shut }),
                    None => self.restore(here),
                }
            }
            if self.allow.builtins {
                let here = self.save();
                match self.builtin() {
                    Some(value) => return Some(Slot { value, deep: 1, fill: Fill::Shut }),
                    None => self.restore(here),
                }
            }
        }
        let start = self.at;
        let (kind, fill) = match code {
            0x8c | 0x58 | 0x8d => return Some(Slot { value: self.text()?, deep: 1, fill: Fill::Shut }),
            b'h' | b'j' => return Some(Slot { value: self.named()?, deep: 1, fill: Fill::Shut }),
            b'K' | b'M' | b'J' | 0x8a => return Some(Slot { value: self.integer()?, deep: 1, fill: Fill::Shut }),
            b'G' => return Some(Slot { value: self.binfloat()?, deep: 1, fill: Fill::Shut }),
            b'C' | b'B' | 0x8e => return Some(Slot { value: self.byte_string()?, deep: 1, fill: Fill::Shut }),
            0x96 => return Some(Slot { value: self.bytearray()?, deep: 1, fill: Fill::Shut }),
            b'N' => {
                self.byte()?;
                (Kind::None, Fill::Shut)
            }
            0x88 | 0x89 => {
                self.byte()?;
                (Kind::Bool(code == 0x88), Fill::Shut)
            }
            // The empty tuple is the one container CPython does not memoize:
            // it is a singleton, so there is nothing to file.
            b')' => {
                self.byte()?;
                (Kind::Tuple(Vec::new()), Fill::Shut)
            }
            // A container is filed in the memo when it is created, before
            // anything is put in it, so a name for it is available to the
            // values it holds. That is what a list holding itself is.
            b']' | b'}' | 0x8f => {
                self.byte()?;
                let (kind, what) = match code {
                    b']' => (Kind::List(Vec::new()), Shape::List),
                    b'}' => (Kind::Dict(Vec::new()), Shape::Dict),
                    _ => (Kind::Set(Vec::new()), Shape::Set),
                };
                self.memoize(Bound::Made { what, at: start, hashable: false })?;
                (kind, Fill::Open)
            }
            _ => return None,
        };
        Some(Slot { value: self.span(start, kind), deep: 1, fill })
    }

    /// An integer, in the width CPython writes it in.
    ///
    /// BININT1 takes nought to 255, BININT2 the next two bytes' worth, and
    /// BININT anything else a signed four-byte integer holds. Past that comes
    /// LONG1, a run of two's-complement bytes as short as the number allows.
    /// Each range belongs to exactly one of them, so a small number written
    /// in a wide field is a non-match.
    pub(super) fn integer(&mut self) -> Option<Value> {
        self.gate()?;
        let start = self.at;
        let kind = match self.byte()? {
            b'K' => Kind::Int { value: i128::from(self.byte()?), at: start + 1, len: 1 },
            b'M' => {
                let value = i128::from(u16::from_le_bytes(self.take(2)?.try_into().ok()?));
                if value < 256 {
                    return None;
                }
                Kind::Int { value, at: start + 1, len: 2 }
            }
            b'J' => {
                let value = i128::from(i32::from_le_bytes(self.take(4)?.try_into().ok()?));
                if (0..=0xffff).contains(&value) {
                    return None;
                }
                Kind::Int { value, at: start + 1, len: 4 }
            }
            0x8a => {
                let len = usize::from(self.byte()?);
                let at = self.at;
                let value = two_complement(self.take(len)?)?;
                // Anything a four-byte integer holds is written as one, and
                // CPython writes no byte a number does not need.
                if i32::try_from(value).is_ok() || shortest(value) != len {
                    return None;
                }
                Kind::Int { value, at, len }
            }
            _ => return None,
        };
        Some(self.span(start, kind))
    }

    /// BINGET or LONG_BINGET where a value belongs: the file naming
    /// something it wrote earlier rather than writing it again.
    ///
    /// Any value the basic productions built may be named, which covers one
    /// list under several keys and a list holding itself: a container is
    /// filed when it is created, so a name for it exists while it is still
    /// being filled. What a slot a form could not name holds is opaque, and a
    /// reference to one is a non-match as before.
    fn named(&mut self) -> Option<Value> {
        self.gate()?;
        let start = self.at;
        let kind = match self.reference()?.clone() {
            Bound::Text { at, len } => Kind::Ref(Names::Text { at, len }),
            Bound::Bytes { at, len } => Kind::Ref(Names::Bytes { at, len }),
            Bound::Made { what, at, hashable } => Kind::Ref(Names::Made { what, at, hashable }),
            // A class the file named earlier, which is how the second block of
            // a frame names the callable the first one spelled out. Only the
            // modules this form may name a class from: a slot holding one of
            // the globals a NumPy call names inside its own fixed run is not a
            // class this form has anything to say about.
            Bound::Global(path) if self.may_name(&path) => Kind::Class { path, parts: Vec::new() },
            _ => return None,
        };
        Some(self.span(start, kind))
    }

    /// BYTEARRAY8, which protocol 5 writes for a bytearray where protocol 4
    /// calls the class. The bytes it was made from are a field of their own,
    /// so both spellings read the same way.
    fn bytearray(&mut self) -> Option<Value> {
        if self.proto < 5 {
            return None;
        }
        self.gate()?;
        let start = self.at;
        self.exact(&[0x96])?;
        let (at, len) = self.counted(0x96, NO_OPCODE, NO_OPCODE, 0x96)?;
        self.bytearray_memoize(Bound::Made { what: Shape::ByteArray, at: start, hashable: false })?;
        let held = Value { at, len, kind: Kind::Bytes { at, len } };
        Some(self.span(start, Kind::Object { what: Shape::ByteArray, names: CONTENT, items: vec![held] }))
    }

    pub(super) fn binfloat(&mut self) -> Option<Value> {
        self.gate()?;
        let start = self.at;
        self.exact(b"G")?;
        let value = f64::from_be_bytes(self.take(8)?.try_into().ok()?);
        Some(self.span(start, Kind::Float { value, at: start + 1, len: 8 }))
    }

    pub(super) fn byte_string(&mut self) -> Option<Value> {
        self.gate()?;
        let start = self.at;
        let code = self.byte()?;
        let (at, len) = self.counted(code, b'C', b'B', 0x8e)?;
        self.memoize(Bound::Bytes { at, len })?;
        Some(self.span(start, Kind::Bytes { at, len }))
    }
}
