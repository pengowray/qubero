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
mod captured;
mod builtins;
mod codecs;
mod cursor;
mod lines;
mod dtype;
mod forms;
mod joblib;
mod memo;
mod numpy;
mod object;
mod packs;
mod picklers;
mod stdlib;
mod torch;
mod values;
#[cfg(test)]
mod tests;

pub use captured::*;
pub use picklers::Pickler;

use cursor::{Cursor, Framing};
use forms::{forms, Allow, Family, Wrapped};
use packs::{Pack, Packs};
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
/// The module joblib puts its array wrapper in, which is the one name that
/// says a pickle was written by `joblib.dump`. See [`joblib`].
pub const JOBLIB_MODULE: &str = "joblib.numpy_pickle";
/// The most dimensions a shape may declare, which is NumPy's own limit.
const MAX_DIMENSIONS: usize = 32;
/// A payload this size or larger is written between frames rather than inside
/// one: CPython's framer commits the frame it is filling, writes the opcode
/// and the bytes straight out, and starts a new frame after them.
const BIG_PAYLOAD: usize = 1 << 16;
/// A frame's contents shorter than this are written without a FRAME header,
/// so a run of unframed bytes this short may end the file.
const MIN_FRAME: usize = 4;
/// The encoding a pickler below protocol 3 hands a byte string to `_codecs`
/// under, which is the one that maps every byte to the character of the same
/// number.
const LATIN1: &str = "latin1";

/// What the STOP row of the opcode listing says about a match: the contract's
/// sentence, the form that matched, and where the decoded data is.
pub fn stop_message(form: &str) -> String {
    format!("Matched a Familiar Pickle Form ({form}): bypassed Pickle stack machine decoding. Switch to the \"{FAMILIAR_LABEL}\" template to see the data.")
}

#[derive(Debug)]
pub struct Match {
    pub form: &'static str,
    /// The protocol the file was written at. Read from the PROTO opcode from
    /// protocol 2 up; below that a file has no such opcode and this is what
    /// the form that matched reads, which is the opcodes it used.
    pub proto: u8,
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
    /// Which families' own productions the file used, which the mixed form
    /// wants two of and the `families` row names.
    packs: Packs,
    stop: usize,
    payloads: Vec<(usize, Payload)>,
    /// The bytes of every run the file did not write as bytes, by where the
    /// run starts. Protocol 2 writes an array's numbers as the latin-1 text
    /// they spell, so the numbers are nowhere in the file and are decoded once
    /// here rather than per cell. Empty for every file at protocol 3 and up.
    runs: Vec<(usize, Arc<Vec<u8>>)>,
}

impl Match {
    /// The bytes a run stands for, for a run the file wrote as something else.
    /// Nothing when the run in the file already is the bytes.
    pub fn decoded(&self, at: usize) -> Option<&Arc<Vec<u8>>> {
        self.runs.iter().find(|(start, _)| *start == at).map(|(_, held)| held)
    }

    /// Which families of values the file turned out to hold, in a fixed order
    /// and starting with the grammar every one of them is read against.
    ///
    /// Said of every matched file and not only a mixed one. The form names a
    /// grammar and this names what the file used, and a reader comparing two
    /// files wants to see the same rows in both.
    pub fn families(&self) -> String {
        self.packs.names()
    }
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
        runs: Vec::new(),
        dicts: Pickler::Undetermined,
        batch: None,
        framing: Framing::Unframed,
        pickler: Pickler::Undetermined,
        allow,
        calls: Vec::new(),
        payloads: Vec::new(),
        arrays: 0,
        objects: 0,
        instances: 0,
        packs: Packs::default(),
        wrappers: 0,
        tensors: 0,
        breaks: Vec::new(),
        furthest: 0,
    };
    let found = c.whole(form);
    *left = c.left;
    *reached = (*reached).max(c.furthest);
    found
}

/// What the bytes in front of a joblib array's numbers are called: one byte
/// saying how many follow, and that many. Not an opcode and not a value, and
/// named so that a matched file still has no byte left over.
pub const PADDING: &str = "padding";

/// Every instruction of a file a form has just matched.
///
/// The same walk the listing does, over bytes already known to be a whole
/// pickle: it reaches the STOP and stops there, so what comes back covers the
/// file exactly. Names are `pickletools`' own.
///
/// A joblib file has runs in it that are not opcodes and pickles in it that
/// end before the file does, so the walk is done in the segments between
/// them. Each segment starts where the one before it ended, which is a
/// boundary between two instructions, so nothing straddles one.
fn instructions(bytes: &[u8], breaks: &[joblib::Break]) -> Vec<Instr> {
    let mut out = Vec::new();
    let mut from = 0;
    for stop in breaks {
        match *stop {
            joblib::Break::Numbers { pad_at, data_at, end } => {
                walked(bytes, from, pad_at, &mut out);
                if data_at > pad_at {
                    out.push(Instr { at: pad_at, end: data_at, name: PADDING });
                }
                from = end;
            }
            // The nested pickle is opcodes throughout, and its STOP ends the
            // walk it is in, so it is walked on its own and the outer stream
            // is walked again after it.
            joblib::Break::Nested { at, end } => {
                walked(bytes, from, at, &mut out);
                walked(bytes, at, end, &mut out);
                from = end;
            }
        }
    }
    walked(bytes, from, bytes.len(), &mut out);
    out
}

/// The opcodes of one segment, counted from the front of the file.
fn walked(bytes: &[u8], from: usize, to: usize, out: &mut Vec<Instr>) {
    let Some(window) = bytes.get(from..to) else { return };
    out.extend(super::opcodes(window).iter().map(|op| Instr {
        at: from + op.at as usize,
        end: from + op.end as usize,
        name: super::opcode_name(op.code),
    }));
}

/// Whether a class the file named is anywhere in what it built.
///
/// Walked with a list rather than by recursion: the tree is bounded in depth,
/// but so is the stack this runs on, and nothing here needs the call frames.
fn holds_class(value: &Value) -> bool {
    let mut left = vec![value];
    while let Some(value) = left.pop() {
        match &value.kind {
            Kind::Class { .. } => return true,
            Kind::List(items)
            | Kind::Tuple(items)
            | Kind::Set(items)
            | Kind::FrozenSet(items)
            | Kind::Objects { items, .. } => left.extend(items),
            Kind::Dict(entries) => left.extend(entries.iter().flat_map(|(k, v)| [k, v])),
            Kind::Instance { class, state } => {
                left.push(class);
                left.extend(state.as_deref());
            }
            // Not the callable a `Made` carries. That is the thing one of the
            // form's enumerated calls named in order to call it, checked
            // against the list when the REDUCE was read; a class handed to
            // the reader as data is one nothing called, and those are what
            // this is looking for.
            Kind::Made { items, state, .. } => {
                left.extend(items);
                left.extend(state.as_deref());
            }
            _ => {}
        }
    }
    false
}

impl<'a> Cursor<'a> {
    /// The envelope a pickle is written in and the one object inside it: the
    /// protocol opener, the frame the writer opened after it, the object, and
    /// the STOP that ends it.
    ///
    /// The whole of a file, and also the whole of a pickle written inside
    /// another one, which is what `joblib.dump` does with an array of
    /// objects. What the two do not share is the end: a file has to stop at
    /// its last byte, and a nested pickle stops at its STOP with the stream
    /// around it carrying on.
    /// Comes back with where the object started, which is one past the
    /// protocol byte and past the frame header when there is one.
    pub(super) fn enveloped(&mut self, protocols: &[u8]) -> Option<(usize, Value)> {
        // PROTO and the number arrived with protocol 2. Below it a file opens
        // at its first value and says nothing about which protocol it is, so
        // the form that reads it is what says: one using binary opcodes and no
        // opener is protocol 1, and one using none of them is protocol 0.
        match protocols {
            [first, ..] if *first < 2 => self.proto = *first,
            _ => {
                self.exact(&[0x80])?;
                self.proto = match self.byte()? {
                    proto if protocols.contains(&proto) => proto,
                    _ => return None,
                };
            }
        }
        // Framing arrived with protocol 4. Below it a file is one run of
        // instructions and a FRAME opcode is not one of them.
        if self.proto >= 4 && self.peek() == Some(0x95) {
            self.frame_header()?;
        }
        let body = self.at;
        let value = self.object()?;
        self.exact(b".")?;
        // The last frame ends where the STOP does. The one exception is a
        // pickle whose large payload left fewer than MIN_FRAME bytes to write
        // after it, since CPython writes those with no FRAME header in front.
        // A frame that simply stopped early is a non-match.
        match self.framing {
            Framing::Unframed => {}
            Framing::Inside(end) | Framing::Full(end) if end == self.at => {}
            Framing::Tail(from) if self.at - from < MIN_FRAME => {}
            _ => return None,
        }
        Some((body, value))
    }

    fn whole(&mut self, form: &'static str) -> Option<Match> {
        let protocols = self.allow.protocols;
        let (body, value) = self.enveloped(protocols)?;
        if self.at != self.bytes.len() {
            return None;
        }
        self.numbering()?;
        // A class a form names no classes for is there to be called, and the
        // call is what the form read. One that survived into the tree instead
        // of being folded away by its call is a class the file is handing the
        // reader as data, which is the one thing the contract rules out.
        if self.allow.classes.is_empty() && holds_class(&value) {
            return None;
        }
        // What the file used, which is the classes noted as they were built
        // and the three productions that name no class, each of which already
        // keeps a count of itself.
        let mut packs = self.packs;
        packs.set(Pack::Numpy, self.arrays > 0);
        packs.set(Pack::Builtins, self.objects > 0);
        packs.set(Pack::Joblib, self.wrappers > 0);
        let plain = self.arrays == 0 && self.objects == 0 && self.instances == 0;
        let needed = match self.allow.family {
            Family::Basic => plain,
            Family::Numpy => self.arrays > 0 && self.objects == 0 && self.instances == 0,
            Family::Builtins => self.arrays == 0 && self.objects > 0 && self.instances == 0,
            // A library form is the one the file's classes came from, and it
            // has to have read at least one of them.
            Family::Library => self.instances > 0,
            // A torch file is tensors and whatever plain data was saved
            // beside them, the way a joblib file is arrays and plain data. A
            // value of any other family in it, a date or a NumPy array, makes
            // it a mixture and it is read under the mixed form.
            Family::Torch => self.tensors > 0 && self.arrays == 0 && self.objects == 0 && self.instances == 0,
            // The mixed form reads every family at once, so what it requires
            // is the mixture: a file of one family is read under the form for
            // that family, which was tried before this one and says so in its
            // name. Two is the count whatever the pair is, so the widest
            // production the union allows, an array of pickled objects, still
            // needs something from a second family beside it.
            Family::Mixed => packs.families() >= 2,
        };
        if !needed {
            return None;
        }
        // A form that is *for* what `joblib.dump` writes requires the file to
        // hold one, the way every other production a form allows is one the
        // file has to use. The mixed form only permits it.
        if self.allow.joblib == Wrapped::Required && self.wrappers == 0 {
            return None;
        }
        Some(Match {
            form,
            proto: self.proto,
            pickler: self.pickler,
            value,
            body,
            calls: std::mem::take(&mut self.calls),
            ops: instructions(self.bytes, &self.breaks),
            packs,
            stop: self.at - 1,
            payloads: std::mem::take(&mut self.payloads),
            runs: std::mem::take(&mut self.runs),
        })
    }
}
