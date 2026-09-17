//! The object a pickle builds, as nodes of the tree like any other field.
//!
//! A `Ty::Pickle` field is recognised once, when something first asks what is
//! inside it, and the captured tree is kept beside the memo. This is
//! [`jsontree`](super::jsontree) for a format whose values are nowhere in the
//! file as a run: a pickle is a program, and what it builds is scattered
//! through the opcodes that build it. A Familiar Pickle Form says where every
//! value went (see [`familiar`]), so every value can be a node at the bytes it
//! was written at.
//!
//! The instructions between the values are nodes too. A form fixes them: the
//! `MEMOIZE` after a string and the `SETITEM` that files it under its key are
//! not noise around the data, they are the shape of the data, written down and
//! matched exactly. So every one of them is a field named for what
//! `pickletools` calls it, and a matched file has no byte left over. The run
//! that rebuilds a NumPy array is a couple of dozen of them and one act, so it
//! is one field holding the names and letters the form matched inside it.
//!
//! The leaves are given the ordinary types their bytes are: a `BININT2` is a
//! little-endian `u16` at its two operand bytes. So reading, displaying and
//! editing one is the machinery every other field uses, and only the nodes
//! that hold others keep [`Ty::Pickle`].

use std::sync::Arc;

use super::*;
use crate::formats::pickle::familiar::{self, Call, Kind, Match, Said, Value};
use crate::formats::pickle::shapes;
use crate::template::{Encoding, Endian::*, Expr as E, PickleShape as Shape, StrLen, Ty as T};

/// What the file holds: what matched, and then the object it matched.
const HEADER_FIELD: &str = "header";
const DATA_FIELD: &str = "data";
/// What the header says: the contract's sentence, the form that matched, and
/// the protocol the writer used.
const MESSAGE_FIELD: &str = "message";
const FORM_FIELD: &str = "form";
const PROTOCOL_FIELD: &str = "protocol";
/// What an array says about itself, and then the numbers themselves. Not
/// `data` again: the object the file holds is already called that, and one
/// name for two things is one thing a reader has to work out.
const NUMBERS_FIELD: &str = "numbers";
const DTYPE_FIELD: &str = "dtype";
const SHAPE_FIELD: &str = "shape";
const ORDER_FIELD: &str = "order";
const KEY_FIELD: &str = "key";
const VALUE_FIELD: &str = "value";
/// The storage orders, spelled the way NumPy spells them.
const C_ORDER: &str = "C";
const FORTRAN_ORDER: &str = "Fortran";
/// How a shape with no dimensions is written, which is NumPy's own spelling
/// for the shape of a single value.
const NO_DIMENSIONS: &str = "()";

/// The largest file a form is run over. The recogniser reads the whole
/// document at once, the same limit the deduced readings work under.
const MOST_BYTES: u64 = 256 << 20;

/// One node of a recognised file: where a path lands, and what the node is.
#[derive(Clone)]
enum Part<'a> {
    /// The whole file, which holds the header and the object.
    Doc,
    /// The protocol envelope, read as what matched through it.
    Header,
    /// A row worked out from the match rather than read: no bytes of its own.
    Note(String),
    /// One value, which is a leaf or holds others.
    Value(&'a Value),
    /// One key and one value of a dictionary, kept as the pair it was written
    /// as: two keys spelled alike are two entries, not one.
    Entry(&'a (Value, Value)),
    /// The numbers of a matched array, read as its dtype says.
    Data(&'a Value),
    /// A named operand inside a run of instructions.
    Text(&'a Said),
    /// A run of instructions the form matched as one act.
    Call(&'a Call),
    /// The protocol number, which is the one byte of the envelope that says
    /// anything.
    Protocol,
    /// One instruction the form fixed, or what is left of one once the value
    /// inside it has been taken out.
    Op { at: usize, len: usize },
}

/// What a node is called by the node above it.
#[derive(Clone)]
enum Label {
    Field(&'static str),
    Index(usize),
    /// An entry of a dictionary, named by the key written in the file.
    Key(usize),
}

/// The bytes a node covers: where it starts and where it ends.
///
/// A node that holds others covers everything its production consumed, since
/// its children and the instructions between them tile it. A leaf covers what
/// it is worth, and the opcode that introduced it is an instruction of its own.
fn span(found: &Match, part: &Part) -> (usize, usize) {
    match part {
        Part::Doc => (0, found.ops.last().map_or(0, |op| op.end)),
        Part::Header => (0, found.body),
        Part::Note(_) => (0, 0),
        Part::Value(v) => match leaf(v) {
            Some((_, at, len)) => (at, at + len),
            None => (v.at, v.at + v.len),
        },
        Part::Entry(e) => (e.0.at, e.1.at + e.1.len),
        Part::Data(v) => match &v.kind {
            Kind::Array { at, len, .. } => (*at, at + len),
            _ => (0, 0),
        },
        Part::Text(s) => (s.at, s.at + s.len),
        Part::Call(c) => (c.at, c.at + c.len),
        Part::Protocol => (1, 2),
        Part::Op { at, len } => (*at, at + len),
    }
}

/// The instructions covering `from..to`, clipped to it.
///
/// What is left of an instruction once a value has been taken out of it is the
/// opcode and the length that measured the value, which is the instruction
/// itself: `SHORT_BINUNICODE 5` in front of five bytes of text.
fn between<'a>(found: &Match, from: usize, to: usize) -> Vec<(Label, Part<'a>)> {
    if from >= to {
        return Vec::new();
    }
    let first = found.ops.partition_point(|op| op.end <= from);
    found.ops[first..]
        .iter()
        .take_while(|op| op.at < to)
        .map(|op| {
            let (at, end) = (op.at.max(from), op.end.min(to));
            (Label::Field(op.name), Part::Op { at, len: end - at })
        })
        .collect()
}

/// Every node inside this one, in file order, with nothing left over.
///
/// The values the form captured, the runs of instructions between them, and
/// the rows worked out from the match, which have no bytes and come first.
fn parts<'a>(found: &'a Match, here: &Part<'a>) -> Vec<(Label, Part<'a>)> {
    let (notes, kids): (Vec<(Label, Part)>, Vec<(Label, Part)>) = match here {
        Part::Doc => (
            Vec::new(),
            vec![
                (Label::Field(HEADER_FIELD), Part::Header),
                (Label::Field(DATA_FIELD), Part::Value(&found.value)),
            ],
        ),
        Part::Header => (
            vec![
                (Label::Field(MESSAGE_FIELD), Part::Note(familiar::MESSAGE.to_string())),
                (Label::Field(FORM_FIELD), Part::Note(found.form.to_string())),
            ],
            vec![(Label::Field(PROTOCOL_FIELD), Part::Protocol)],
        ),
        Part::Entry(e) => (
            Vec::new(),
            vec![(Label::Field(KEY_FIELD), Part::Value(&e.0)), (Label::Field(VALUE_FIELD), Part::Value(&e.1))],
        ),
        Part::Call(c) => (
            Vec::new(),
            c.says.iter().map(|s| (Label::Field(s.name), Part::Text(s))).collect(),
        ),
        Part::Value(v) => match &v.kind {
            Kind::Dict(entries) => (
                Vec::new(),
                entries.iter().enumerate().map(|(i, e)| (Label::Key(i), Part::Entry(e))).collect(),
            ),
            Kind::List(items) | Kind::Tuple(items) => (
                Vec::new(),
                items.iter().enumerate().map(|(i, x)| (Label::Index(i), Part::Value(x))).collect(),
            ),
            // A builtin written as a call. Its parts are named where Python
            // names them and numbered where it does not.
            Kind::Object { names, items, .. } => (
                Vec::new(),
                items
                    .iter()
                    .enumerate()
                    .map(|(i, x)| match names.get(i) {
                        Some(name) => (Label::Field(name), Part::Value(x)),
                        None => (Label::Index(i), Part::Value(x)),
                    })
                    .collect(),
            ),
            Kind::Array { dtype, dimensions, fortran_order, .. } => {
                let shape = match dimensions.is_empty() {
                    true => NO_DIMENSIONS.to_string(),
                    false => dimensions.iter().map(u64::to_string).collect::<Vec<_>>().join(" x "),
                };
                let order = match fortran_order {
                    true => FORTRAN_ORDER,
                    false => C_ORDER,
                };
                let notes = vec![
                    (Label::Field(DTYPE_FIELD), Part::Note(dtype.clone())),
                    (Label::Field(SHAPE_FIELD), Part::Note(shape)),
                    (Label::Field(ORDER_FIELD), Part::Note(order.to_string())),
                ];
                let mut kids = Vec::new();
                // The call that rebuilt this array, when it is this array's:
                // a form matches one, and it sits inside the array's bytes.
                if let Some(call) = found.calls.iter().find(|c| c.at >= v.at && c.at + c.len <= v.at + v.len) {
                    kids.push((Label::Field(call.name), Part::Call(call)));
                }
                kids.push((Label::Field(NUMBERS_FIELD), Part::Data(v)));
                (notes, kids)
            }
            // A leaf, which holds nothing.
            _ => return Vec::new(),
        },
        // Everything else is read from its own bytes and holds nothing.
        _ => return Vec::new(),
    };
    let (from, to) = span(found, here);
    let mut out = notes;
    let mut cursor = from;
    for (label, part) in kids {
        let (at, end) = span(found, &part);
        out.extend(between(found, cursor, at));
        out.push((label, part));
        cursor = end;
    }
    out.extend(between(found, cursor, to));
    out
}

/// Where `path`, counted from the pickle field, lands.
fn spot<'a>(found: &'a Match, path: &[usize]) -> Option<(Label, Part<'a>)> {
    let mut here = (Label::Field("file"), Part::Doc);
    for &idx in path {
        here = parts(found, &here.1).into_iter().nth(idx)?;
    }
    Some(here)
}

/// What a node of the tree is, for the type column. None for a leaf, which is
/// given the type its bytes are instead.
fn shape_of(part: &Part) -> Option<Shape> {
    Some(match part {
        Part::Doc => Shape::Doc,
        Part::Header => Shape::Header,
        Part::Entry(_) => Shape::Entry,
        Part::Call(_) => Shape::Call,
        Part::Value(v) => match &v.kind {
            Kind::Dict(_) => Shape::Dict,
            Kind::List(_) => Shape::List,
            Kind::Tuple(_) => Shape::Tuple,
            Kind::Array { .. } => Shape::Array,
            Kind::Object { what, .. } => *what,
            _ => return None,
        },
        _ => return None,
    })
}

/// A leaf, as the type its bytes are and the bytes its value proper occupies.
/// The opcode that introduced it is not part of that, bar the two values that
/// are written as an opcode and nothing else.
fn leaf(value: &Value) -> Option<(T, usize, usize)> {
    Some(match &value.kind {
        Kind::None => (T::enumeration("null", T::u8(), &[(0x4e, "None")]), value.at, 1),
        Kind::Bool(_) => (T::enumeration("bool", T::u8(), &[(0x88, "True"), (0x89, "False")]), value.at, 1),
        Kind::Int { at, len, .. } => {
            let ty = match len {
                1 => T::u8(),
                2 => T::u16(Little),
                _ => T::i32(Little),
            };
            (ty, *at, *len)
        }
        // The one big-endian number in the format.
        Kind::Float { at, len, .. } => (T::F64(Big), *at, *len),
        Kind::Text { at, len } => (T::text(StrLen::Fixed(E::lit(*len as i128)), Encoding::Utf8), *at, *len),
        Kind::Bytes { at, len } => (T::bytes(E::lit(*len as i128)), *at, *len),
        _ => return None,
    })
}

/// How many values a shape holds. Zero when any dimension is, which is what
/// the recogniser already checked the payload against.
fn count_of(dimensions: &[u64]) -> u64 {
    match dimensions.contains(&0) {
        true => 0,
        false => dimensions.iter().product(),
    }
}

impl Evaluator {
    /// The path of the pickle field `path` sits in, and what the recogniser
    /// made of it. `path` may be the field itself or any value inside it.
    fn pickle_doc<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<(Vec<usize>, Arc<Match>)> {
        let root = (0..=path.len())
            .rev()
            .find(|k| matches!(self.memo.get(&path[..*k]).map(|r| &r.ty), Some(Ty::Pickle(Shape::Doc))));
        let Some(k) = root else { return fail("not inside a pickle field") };
        let root = path[..k].to_vec();
        if let Some(found) = self.memo.pickle(&root) {
            return Ok((root, found.clone()));
        }
        let r = self.memo[&root].clone();
        let size = r.declared_size.unwrap_or(r.limit - r.offset);
        if size / 8 > MOST_BYTES {
            return fail("this file is too large to match against a Familiar Pickle Form");
        }
        let bytes = self.read(doc, &r, r.offset, size)?;
        let Some(found) = familiar::recognise(&bytes) else {
            return fail("this isn't a Familiar Pickle Form: open it with the Python pickle template to read the program");
        };
        let found = Arc::new(found);
        self.memo.remember_pickle(root.clone(), found.clone());
        Ok((root, found))
    }

    /// How many nodes are inside the one at `path`.
    pub(super) fn pickle_child_count<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<u64> {
        let (root, found) = self.pickle_doc(doc, path)?;
        let Some((_, here)) = spot(&found, &path[root.len()..]) else { return fail("no such value") };
        Ok(parts(&found, &here).len() as u64)
    }

    /// Which child of the node at `path` is called `name`.
    pub(super) fn pickle_index<S: Source>(&mut self, doc: &Document<S>, path: &[usize], name: &str) -> R<Option<usize>> {
        let (root, found) = self.pickle_doc(doc, path)?;
        let Some((_, here)) = spot(&found, &path[root.len()..]) else { return fail("no such value") };
        let r = self.memo[path].clone();
        let base = self.memo[&root].offset;
        for (i, (label, part)) in parts(&found, &here).into_iter().enumerate() {
            let said = match label {
                Label::Field(f) => f == name,
                Label::Index(n) => name.parse::<usize>().ok() == Some(n),
                Label::Key(n) => match (&here, &part) {
                    (Part::Value(v), Part::Entry(_)) => match &v.kind {
                        Kind::Dict(entries) => {
                            let key = entries.get(n).map(|e| &e.0);
                            match key {
                                Some(key) => self.pickle_text(doc, &r, base, key)?.as_deref() == Some(name),
                                None => false,
                            }
                        }
                        _ => false,
                    },
                    _ => false,
                },
            };
            if said {
                return Ok(Some(i));
            }
        }
        Ok(None)
    }

    /// The text of a key, read from the file. The recogniser checked it was
    /// UTF-8 and kept where it is rather than a copy of it.
    fn pickle_text<S: Source>(&self, doc: &Document<S>, r: &Resolved, base: u64, key: &Value) -> R<Option<String>> {
        let Kind::Text { at, len } = key.kind else { return Ok(None) };
        let bytes = self.read(doc, r, base + at as u64 * 8, len as u64 * 8)?;
        Ok(String::from_utf8(bytes).ok())
    }

    /// Place child `idx` of the pickle node at `path`'s parent.
    ///
    /// A node that holds others is settled here, the way a value inside JSON
    /// is: the form already said where it is, so there is nothing to read.
    /// A leaf is handed back as a place, so that the type its bytes are is
    /// read, displayed and written back by the ordinary machinery.
    pub(super) fn place_pickle_child<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Option<Place>> {
        let (parent, idx) = (&path[..path.len() - 1], path[path.len() - 1]);
        let (root, found) = self.pickle_doc(doc, parent)?;
        let Some((_, above)) = spot(&found, &parent[root.len()..]) else { return fail("no such value") };
        let Some((label, part)) = parts(&found, &above).into_iter().nth(idx) else {
            return fail("no such value");
        };
        let pr = self.memo[parent].clone();
        let base = self.memo[&root].offset;
        let name = match label {
            Label::Field(f) => Name::Field(f.into()),
            Label::Index(n) => Name::Index(n),
            Label::Key(n) => match &above {
                Part::Value(v) => match &v.kind {
                    Kind::Dict(entries) => match entries.get(n).map(|e| &e.0) {
                        Some(key) => match self.pickle_text(doc, &pr, base, key)? {
                            Some(text) => Name::Field(text.into()),
                            None => Name::Index(n),
                        },
                        None => Name::Index(n),
                    },
                    _ => Name::Index(n),
                },
                _ => Name::Index(n),
            },
        };
        let (at, end) = span(&found, &part);
        // A node that holds others is placed here and read no further.
        if let Some(shape) = shape_of(&part) {
            self.pickle_node(path, &pr, name, shape, base, at, end - at);
            return Ok(None);
        }
        match part {
            Part::Note(text) => self.pickle_note(path, &pr, name, text),
            // The instructions the form fixed. Named for what they are, and
            // marked as the structure's own machinery: a reader following the
            // data wants them folded, and a reader following the program
            // wants them named.
            Part::Op { .. } => Ok(Some(self.pickle_place(&pr, name, T::bytes(E::lit((end - at) as i128)), base, at, end - at, true))),
            Part::Text(said) => {
                let ty = T::text(StrLen::Fixed(E::lit(said.len as i128)), Encoding::Utf8);
                Ok(Some(self.pickle_place(&pr, name, ty, base, at, end - at, false)))
            }
            // The one byte of the envelope with something in it. PROTO is the
            // instruction in front of it, and the frame's length is the
            // listing's business rather than the object's.
            Part::Protocol => Ok(Some(self.pickle_place(&pr, name, T::u8(), base, at, end - at, false))),
            Part::Data(v) => {
                let Kind::Array { dtype, dimensions, .. } = &v.kind else { return fail("no such value") };
                let Some(ty) = shapes::run(dtype, count_of(dimensions)) else {
                    return fail("this dtype has no type");
                };
                Ok(Some(self.pickle_place(&pr, name, ty, base, at, end - at, false)))
            }
            Part::Value(v) => match leaf(v) {
                Some((ty, at, len)) => Ok(Some(self.pickle_place(&pr, name, ty, base, at, len, false))),
                None => fail("this value has no type"),
            },
            _ => fail("no such value"),
        }
    }

    /// A row that says something about the file rather than reading it: no
    /// bytes of its own, and its text worked out when the form matched.
    fn pickle_note(&mut self, path: &[usize], pr: &Resolved, name: Name, text: String) -> R<Option<Place>> {
        let r = Resolved {
            name,
            // The expression is never asked: the text is on the node, which is
            // where a computed field's value is kept once it is worked out.
            ty: Ty::ComputedText(E::lit(0)),
            offset: pr.offset,
            cursor: pr.offset,
            limit: pr.limit,
            declared_size: None,
            sized_how: None,
            origin: false,
            size: Some(0),
            payload: None,
            computed: Some(Computed::Text(text.as_str().into())),
            space: pr.space,
            machinery: false,
        };
        self.remember(path, r);
        Ok(None)
    }

    /// A node that holds others, covering the bytes its production consumed.
    fn pickle_node(&mut self, path: &[usize], pr: &Resolved, name: Name, shape: Shape, base: u64, at: usize, len: usize) {
        let offset = base + at as u64 * 8;
        let size = len as u64 * 8;
        let r = Resolved {
            name,
            ty: Ty::Pickle(shape),
            offset,
            cursor: offset,
            limit: offset + size,
            declared_size: None,
            sized_how: None,
            origin: false,
            size: Some(size),
            payload: None,
            computed: None,
            space: pr.space,
            machinery: false,
        };
        self.remember(path, r);
    }

    /// Where a leaf goes, for the ordinary machinery to read it there.
    fn pickle_place(&self, pr: &Resolved, name: Name, ty: T, base: u64, at: usize, len: usize, machinery: bool) -> Place {
        let offset = base + at as u64 * 8;
        Place { name, ty, offset, limit: offset + len as u64 * 8, space: pr.space, machinery }
    }
}
