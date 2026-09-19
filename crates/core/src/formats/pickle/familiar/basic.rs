//! The value productions every form allows: the stack a pickle is read
//! against, tuples, lists, dictionaries, sets, integers, floats, text, byte
//! strings and bytearrays. A form that allows more than these adds its
//! productions in a file beside this one.

use super::cursor::{Cursor, Framing};
use super::values::hashable;
use super::memo::Bound;
use super::picklers::WIDE_BATCH;
use super::{Kind, Pickler, Shape, Value, MAX_BATCH, MAX_DEPTH, MAX_VALUES};

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

/// What a container's entries go into: the value itself, or the state of a
/// call whose result the opcodes after it fill.
///
/// `collections.OrderedDict()` is a call, and the SETITEMS after it fills what
/// the call made. So the fold looks through a call's result to the empty
/// container the calls table put there, and everything else about filling is
/// the same code.
fn filled(kind: &mut Kind) -> &mut Kind {
    match kind {
        Kind::Made { state: Some(state), .. } => &mut state.kind,
        other => other,
    }
}

/// The same, for a reading that changes nothing.
fn inside(kind: &Kind) -> &Kind {
    match kind {
        Kind::Made { state: Some(state), .. } => &state.kind,
        other => other,
    }
}

/// Whether the opcodes after this value may fill it, which is what the calls
/// table says by handing the result an empty container to be filled.
fn opens(kind: &Kind) -> Fill {
    match kind {
        // Named rather than read off the empty container, so that a `Counter`
        // of nothing and a call whose BUILD handed it an empty dictionary are
        // not filled by whatever comes next: only these three classes are
        // written empty and filled afterwards.
        Kind::Made { what: Shape::OrderedDict | Shape::DefaultDict | Shape::Deque, state: Some(state), .. } => match state.kind {
            Kind::Dict(ref entries) if entries.is_empty() => Fill::Open,
            Kind::List(ref items) if items.is_empty() => Fill::Open,
            _ => Fill::Shut,
        },
        _ => Fill::Shut,
    }
}

/// What one opcode of the run did.
enum Step {
    /// It folded or pushed something, and the run goes on.
    Went,
    /// It was the STOP, so what is on the stack is the whole of the object.
    Stopped,
}

/// Whether this opcode begins an object.
///
/// CPython's framer ends a frame where the next object begins and nowhere
/// else, so this is the list of places a frame boundary may fall. The
/// opcodes that fold what is already on the stack are not on it.
fn opens_object(code: u8) -> bool {
    matches!(
        code,
        b'N' | 0x88 | 0x89 | b'K' | b'M' | b'J' | 0x8a | b'G' | 0x8c | 0x58 | 0x8d | b'C' | b'B' | 0x8e | 0x96 | b')' | b']' | b'}' | 0x8f | b'h' | b'j' | b'(' | b'c' | b'I' | b'U' | b'T' | b'L' | b'V' | b'S' | b'F' | b'g'
    )
}

impl Cursor<'_> {
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
        while let Step::Went = self.step(&mut stack, &mut marks)? {}
        if stack.len() != 1 || !marks.is_empty() {
            return None;
        }
        let whole = stack.pop()?;
        self.shut(std::slice::from_ref(&whole))?;
        Some(whole.value)
    }

    /// The list of values an array of pickled objects is handed.
    ///
    /// An ordinary list, created empty and filled by the opcodes after it, and
    /// read against the same stack the rest of the file is read against. So a
    /// value in it is whatever the file's form allows anywhere else, under the
    /// same bounds, rather than a leaf read one at a time.
    ///
    /// How many values the array's shape comes to is what says where the list
    /// ends, since the opcode after the last one belongs to the run that made
    /// the array. A batch that overshoots the count is a non-match.
    pub(super) fn filled_list(&mut self, count: u64) -> Option<Vec<Value>> {
        self.gate()?;
        let at = self.at;
        // Protocol 0 has no opcode for an empty list and writes a MARK with a
        // LIST behind it.
        match self.proto {
            0 => self.exact(b"(l")?,
            _ => self.exact(b"]")?,
        }
        self.memoize(Bound::Made { what: Shape::List, at, hashable: false })?;
        // An array of objects holding another one is read from inside the run
        // that made the outer array, which is this program's own stack as well
        // as depth in the tree, so both are counted against the same bound.
        self.nesting += 1;
        if self.nesting > MAX_DEPTH {
            return None;
        }
        let mut stack = vec![Slot { value: Value { at, len: self.at - at, kind: Kind::List(Vec::new()) }, deep: 1, fill: Fill::Open }];
        let mut marks: Vec<Mark> = Vec::new();
        loop {
            let held = match stack.first().map(|slot| &slot.value.kind) {
                Some(Kind::List(items)) => items.len() as u64,
                // The list is no longer what the run is filling, which is a
                // fold that reached past it.
                _ => return None,
            };
            if held >= count {
                break;
            }
            if let Step::Stopped = self.step(&mut stack, &mut marks)? {
                return None;
            }
        }
        if stack.len() != 1 || !marks.is_empty() {
            return None;
        }
        self.nesting -= 1;
        let Kind::List(items) = stack.pop()?.value.kind else { return None };
        (items.len() as u64 == count).then_some(items)
    }

    /// One opcode of the run: what it folds off the stack, what it pushes back,
    /// and whether the run goes on.
    fn step(&mut self, stack: &mut Vec<Slot>, marks: &mut Vec<Mark>) -> Option<Step> {
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
            return Some(Step::Stopped);
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
            0x85..=0x87 if self.proto >= 2 => {
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
            // FROZENSET arrived with protocol 4; below it a frozenset is
            // a call of the class, which the calls table reads.
            b't' | 0x91 => {
                if code == 0x91 && self.proto < 4 {
                    return None;
                }
                self.byte()?;
                let mark = marks.pop()?;
                let items = stack.split_off(mark.floor);
                if code == b't' && items.len() < 4 && self.proto >= 2 {
                    // Three or fewer are written with TUPLE1 to TUPLE3
                    // from protocol 2, which is where those opcodes are.
                    return None;
                }
                let holds = items.iter().all(|slot| hashable(&slot.value));
                if code == 0x91 && !holds {
                    return None;
                }
                let what = if code == b't' { Shape::Tuple } else { Shape::FrozenSet };
                // Protocol 0 writes an empty tuple as a bare MARK and
                // TUPLE and files nothing: it is a singleton, as the
                // opcode for it above protocol 0 is. Jython's `cPickle` is
                // the one that files it, because `save_tuple` there takes
                // the same path for an empty tuple as for any other and
                // ends it with a `put`.
                let singleton = self.proto == 0 && items.is_empty();
                if !singleton || self.peek() == Some(b'p') {
                    if singleton {
                        self.wrote(Pickler::Jython)?;
                    }
                    self.memoize(Bound::Made { what, at: mark.at, hashable: holds })?;
                }
                let make = if code == b't' { Kind::Tuple } else { Kind::FrozenSet };
                stack.push(self.folded(mark.at, items, make)?);
            }
            // DICT and LIST, which is how protocol 0 creates one: a MARK
            // and the opcode, with nothing between them. CPython writes
            // them empty and fills them with SETITEM and APPEND, so a
            // container made with anything between the two is a file no
            // pickler wrote.
            b'd' | b'l' if self.proto == 0 => {
                self.byte()?;
                let mark = marks.pop()?;
                if stack.len() != mark.floor {
                    return None;
                }
                let (kind, what) = match code {
                    b'd' => (Kind::Dict(Vec::new()), Shape::Dict),
                    _ => (Kind::List(Vec::new()), Shape::List),
                };
                self.memoize(Bound::Made { what, at: mark.at, hashable: false })?;
                stack.push(Slot { value: Value { at: mark.at, len: self.at - mark.at, kind }, deep: 1, fill: Fill::Open });
            }
            // The opcodes that fill a container the file made empty.
            b'a' | b's' => {
                self.byte()?;
                let wants = if code == b'a' { 1 } else { 2 };
                if stack.len() < floor + wants + 1 {
                    return None;
                }
                let items = stack.split_off(stack.len() - wants);
                self.one(stack, items, code == b'a')?;
            }
            // ADDITEMS arrived with protocol 4, along with the set opcode
            // it fills.
            b'e' | b'u' | 0x90 => {
                if self.proto == 0 || (code == 0x90 && self.proto < 4) {
                    // Protocol 0 fills a container one entry at a time.
                    return None;
                }
                self.byte()?;
                let mark = marks.pop()?;
                if mark.floor == 0 {
                    return None;
                }
                let items = stack.split_off(mark.floor);
                self.batch(stack, items, code)?;
            }
            // STACK_GLOBAL, NEWOBJ, NEWOBJ_EX, REDUCE and BUILD, which only
            // a form that reads a library object allows. Each folds exactly
            // the things written in front of it, in the same order: the
            // thing being named, called or filled first, and what it is
            // named, called or filled with after. NEWOBJ_EX takes three,
            // since it has a place for keyword arguments; the rest take
            // two. Both arrived with protocol 4.
            0x93 | 0x81 | 0x92 | b'R' | b'b' if self.reads_calls() && (!matches!(code, 0x93 | 0x92) || self.proto >= 4) => {
                self.byte()?;
                let wants = if code == 0x92 { 3 } else { 2 };
                if stack.len() < floor + wants {
                    return None;
                }
                let items = stack.split_off(stack.len() - wants);
                let at = items[0].value.at;
                self.shut(&items)?;
                let deep = 1 + items.iter().map(|slot| slot.deep).max().unwrap_or(0);
                if self.too_deep(deep) {
                    return None;
                }
                let values = items.into_iter().map(|slot| slot.value).collect();
                let kind = self.library(code, at, values)?;
                let fill = opens(&kind);
                stack.push(Slot { value: Value { at, len: self.at - at, kind }, deep, fill });
            }
            _ => {
                if stack.len() >= MAX_VALUES {
                    return None;
                }
                let slot = self.push(code)?;
                stack.push(slot);
            }
        }
        Some(Step::Went)
    }

    /// Whether a value this deep is deeper than a form reads.
    ///
    /// Counted from the top of the file rather than from the run in hand. An
    /// array of pickled objects is read from inside the run that made it, and
    /// the run below starts its own count at one, so what that run holds is
    /// [`Cursor::nesting`] deeper than it says.
    fn too_deep(&self, deep: usize) -> bool {
        deep.saturating_add(self.nesting) > MAX_DEPTH
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
            if matches!(slot.fill, Fill::Batched(n) if self.batch_full(n)) && !matches!(inside(&slot.value.kind), Kind::List(_)) {
                self.tail(false, Pickler::Python)?;
            }
        }
        Some(())
    }

    /// What the tail of a container a batch longer than a multiple of a
    /// thousand says about who wrote the file.
    ///
    /// For every pickler but one, each tail is that pickler's throughout, so
    /// the first one seen fixes the reading and a file showing both was
    /// written by neither.
    ///
    /// Python 2's `cPickle` is the one, and it is known by the memo slot it
    /// starts counting at. CPython's walks a list through an iterator, the way
    /// `pickle.py` does, and a dictionary the way the C picklers do; PyPy's is
    /// a Python copy that numbers the memo the same way and walks both the
    /// `pickle.py` way. So under that numbering a list ends one way only, a
    /// dictionary ends either way, and the same way throughout the file.
    fn tail(&mut self, list_like: bool, which: Pickler) -> Option<()> {
        if self.memo_base != Some(1) {
            return self.wrote(which);
        }
        if list_like {
            return (which == Pickler::Python).then_some(());
        }
        match self.dicts {
            Pickler::Undetermined => {
                self.dicts = which;
                Some(())
            }
            seen => (seen == which).then_some(()),
        }
    }

    /// One value made out of the things just taken off the stack, spanning
    /// from `at` to wherever the file has got to.
    fn folded(&mut self, at: usize, items: Vec<Slot>, make: fn(Vec<Value>) -> Kind) -> Option<Slot> {
        self.shut(&items)?;
        let deep = 1 + items.iter().map(|slot| slot.deep).max().unwrap_or(0);
        if self.too_deep(deep) {
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
    fn one(&mut self, stack: &mut [Slot], items: Vec<Slot>, list_like: bool) -> Option<()> {
        self.shut(&items)?;
        let deep = 1 + items.iter().map(|slot| slot.deep).max().unwrap_or(0);
        if self.too_deep(deep) {
            return None;
        }
        let mut items = items.into_iter();
        let proto = self.proto;
        let into = stack.last_mut()?;
        match into.fill {
            Fill::Open => {}
            Fill::Batched(n) if self.batch_full(n) => self.tail(list_like, Pickler::Python)?,
            _ => return None,
        }
        // APPEND fills a list and SETITEM a dictionary, and neither fills the
        // other: a container is filled the way its own class is.
        match (list_like, filled(&mut into.value.kind)) {
            (true, Kind::List(values)) => values.push(items.next()?.value),
            (false, Kind::Dict(entries)) => {
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
        // Protocol 0 has no batching: a container is filled one entry at a
        // time, however many it holds, so it stays open for the next one.
        // Every protocol above it writes APPEND or SETITEM once and only for
        // a container holding exactly one thing, or as the tail of a batched
        // one, and either way nothing more may be put in.
        into.fill = match proto {
            0 => Fill::Open,
            _ => Fill::Shut,
        };
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
        if self.too_deep(deep) {
            return None;
        }
        let into = stack.last_mut()?;
        let taken = items.len();
        let mut items = items.into_iter();
        let entries = match (code, filled(&mut into.value.kind)) {
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
        // A first batch is as long as the container, up to one batch. A list
        // or a dictionary holding one thing is written with APPEND or SETITEM
        // instead, so a first batch of one is only ever a set's, or Jython's,
        // whose lists take no APPEND at all. A later batch may be empty, which
        // is what a dictionary or a set whose length is a multiple of a batch
        // ends with; a list's never is.
        // A dictionary of one entry is written with SETITEM by every pickler
        // there is; a set has no shorthand at all; and a list of one is
        // written with APPEND by every pickler but Jython's, which exists
        // only at the protocols Jython writes.
        let least = match code {
            b'u' => 2,
            b'e' if self.proto > 2 => 2,
            _ => 1,
        };
        let fits = match into.fill {
            Fill::Open => (least..=self.batch_cap()).contains(&entries),
            Fill::Batched(before) => self.batch_fixed(before).is_some() && entries <= self.batch_len() && (entries > 0 || code != b'e'),
            Fill::Shut => false,
        };
        if !fits {
            return None;
        }
        // Jython's `cPickle` writes a MARK and an APPENDS however short a list
        // is, and it batches at 1,024 rather than at a thousand. Either one is
        // the whole tell.
        if matches!(into.fill, Fill::Open) && ((code == b'e' && entries == 1) || entries > MAX_BATCH) {
            self.wrote(Pickler::Jython)?;
            self.batch = Some(WIDE_BATCH);
        }
        // A batch after a full one is the tail, and says which pickler wrote
        // it. An empty one, or one holding the single item a list or a
        // dictionary had left over, is the C pickler's.
        if matches!(into.fill, Fill::Batched(_)) && matches!((entries, code), (0, _) | (1, b'e' | b'u')) {
            self.tail(code == b'e', Pickler::C)?;
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
        if matches!(code, 0x8c | 0x58 | 0x8d | b'h' | b'j' | b'g' | b'c') {
            // A byte string below protocol 3 is a call to `_codecs.encode`,
            // which is a fixed run like the two below it.
            if self.proto < 3 {
                let here = self.save();
                match self.spelled_bytes() {
                    Some(value) => return Some(Slot { value, deep: 1, fill: Fill::Shut }),
                    None => self.restore(here),
                }
            }
            // An array `joblib.dump` wrote, which names its own class first
            // and so fails at its first word when the value is anything else.
            if self.allow.joblib != super::forms::Wrapped::Refused {
                let here = self.save();
                match self.joblib_array() {
                    Some(value) => return Some(Slot { value, deep: 1, fill: Fill::Shut }),
                    None => self.restore(here),
                }
            }
            // A tensor, which names `torch._utils` first and so fails at its
            // first word when the value is anything else.
            if self.allow.torch {
                let here = self.save();
                match self.torch_value() {
                    Some(value) => return Some(Slot { value, deep: 1, fill: Fill::Shut }),
                    None => self.restore(here),
                }
            }
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
            b'V' if self.proto == 0 => return Some(Slot { value: self.text_line()?, deep: 1, fill: Fill::Shut }),
            b'S' if self.proto == 0 => return Some(Slot { value: self.string_line()?, deep: 1, fill: Fill::Shut }),
            b'F' if self.proto == 0 => return Some(Slot { value: self.float_line()?, deep: 1, fill: Fill::Shut }),
            b'U' | b'T' if self.proto > 0 => return Some(Slot { value: self.py2_string()?, deep: 1, fill: Fill::Shut }),
            b'h' | b'j' | b'g' => return Some(Slot { value: self.named()?, deep: 1, fill: Fill::Shut }),
            b'K' | b'M' | b'J' | 0x8a | b'I' | b'L' => return Some(Slot { value: self.integer()?, deep: 1, fill: Fill::Shut }),
            b'G' if self.proto > 0 => return Some(Slot { value: self.binfloat()?, deep: 1, fill: Fill::Shut }),
            b'C' | b'B' | 0x8e => return Some(Slot { value: self.byte_string()?, deep: 1, fill: Fill::Shut }),
            0x96 => return Some(Slot { value: self.bytearray()?, deep: 1, fill: Fill::Shut }),
            // GLOBAL, which is how protocols 2 and 3 name a class or a
            // callable. Only a form that reads one allows it, and only the
            // names that form listed.
            b'c' if self.proto < 4 && self.reads_calls() => {
                return Some(Slot { value: self.global_line()?, deep: 1, fill: Fill::Shut })
            }
            b'N' => {
                self.byte()?;
                (Kind::None, Fill::Shut)
            }
            // The two singletons got an opcode each at protocol 2; below it
            // they are integer lines, which [`Cursor::integer`] reads.
            0x88 | 0x89 if self.proto >= 2 => {
                self.byte()?;
                (Kind::Bool(code == 0x88), Fill::Shut)
            }
            // The empty tuple is the one container CPython does not memoize:
            // it is a singleton, so there is nothing to file. Protocol 0 has
            // no opcode for it and writes a MARK with a TUPLE behind it, which
            // the stack reads as a tuple of nothing like any other.
            b')' if self.proto > 0 => {
                self.byte()?;
                (Kind::Tuple(Vec::new()), Fill::Shut)
            }
            // A container is filed in the memo when it is created, before
            // anything is put in it, so a name for it is available to the
            // values it holds. That is what a list holding itself is.
            // EMPTY_SET arrived with protocol 4. Below it a set is a call.
            b']' | b'}' | 0x8f => {
                if code == 0x8f && self.proto < 4 {
                    return None;
                }
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
}
