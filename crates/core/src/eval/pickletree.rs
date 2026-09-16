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
//! One difference from JSON, and it is the whole difference. A JSON value's
//! text holds its members, so the members tile it; a pickle container's bytes
//! hold the opcodes that assemble it as well, and those belong to the
//! container. So nothing here is framed and nothing tiles: a node covers what
//! its production consumed, and its children sit inside that with the
//! instructions between them.
//!
//! The leaves are given the ordinary types their bytes are: a `BININT2` is a
//! little-endian `u16` at its two operand bytes. So reading, displaying and
//! editing one is the machinery every other field uses, and only the nodes
//! that hold others keep [`Ty::Pickle`].

use std::sync::Arc;

use super::*;
use crate::formats::pickle::familiar::{self, Kind, Match, Value};
use crate::formats::pickle::shapes;
use crate::template::{Encoding, Endian::*, Expr as E, PickleShape as Shape, StrLen, Ty as T};

/// What the file holds: what matched, and then the object it matched.
const HEADER_FIELD: &str = "header";
const DATA_FIELD: &str = "data";
/// What the header says: the contract's sentence, the form that matched, and
/// the protocol the writer used, which is the one byte of the envelope that
/// says anything.
const HEADER_FIELDS: [&str; 3] = ["message", "form", "protocol"];
/// What an array says about itself before its numbers.
const ARRAY_FIELDS: [&str; 4] = ["dtype", "shape", "order", "data"];
const ENTRY_FIELDS: [&str; 2] = ["key", "value"];
/// The storage orders, spelled the way NumPy spells them.
const C_ORDER: &str = "C";
const FORTRAN_ORDER: &str = "Fortran";
/// How a shape with no dimensions is written, which is NumPy's own spelling
/// for the shape of a single value.
const NO_DIMENSIONS: &str = "()";

/// The largest file a form is run over. The recogniser reads the whole
/// document at once, the same limit the deduced readings work under.
const MOST_BYTES: u64 = 256 << 20;

/// Where in a recognised tree a path lands.
enum Spot<'a> {
    /// The whole file, which holds the header and the object.
    Doc,
    /// The protocol envelope, read as what matched it.
    Header,
    /// One of the three things the header says.
    Said(usize),
    /// One value, which is a leaf or holds others.
    Value(&'a Value),
    /// One key and one value of a dictionary, kept as the pair it was written
    /// as.
    Entry(&'a (Value, Value)),
    /// One of the four things an array says: its dtype, its shape, its storage
    /// order, and its numbers. The first three are worked out rather than read.
    Part(&'a Value, usize),
}

/// Where `path`, counted from the pickle field, lands in `found`.
fn spot<'a>(found: &'a Match, path: &[usize]) -> Option<Spot<'a>> {
    match path.split_first() {
        None => Some(Spot::Doc),
        Some((0, [])) => Some(Spot::Header),
        Some((0, [i])) if *i < HEADER_FIELDS.len() => Some(Spot::Said(*i)),
        Some((1, rest)) => descend(&found.value, rest),
        _ => None,
    }
}

fn descend<'a>(value: &'a Value, path: &[usize]) -> Option<Spot<'a>> {
    let Some((&idx, rest)) = path.split_first() else { return Some(Spot::Value(value)) };
    match &value.kind {
        // An entry is a node of its own, so that a key spelled like another
        // key stays a second entry rather than replacing the first.
        Kind::Dict(entries) => {
            let entry = entries.get(idx)?;
            match rest.split_first() {
                None => Some(Spot::Entry(entry)),
                Some((0, under)) => descend(&entry.0, under),
                Some((1, under)) => descend(&entry.1, under),
                _ => None,
            }
        }
        Kind::List(items) | Kind::Tuple(items) => descend(items.get(idx)?, rest),
        Kind::Array { .. } if rest.is_empty() && idx < ARRAY_FIELDS.len() => Some(Spot::Part(value, idx)),
        _ => None,
    }
}

/// How many nodes sit under this one.
fn children(spot: &Spot) -> u64 {
    match spot {
        Spot::Doc => 2,
        Spot::Header => HEADER_FIELDS.len() as u64,
        Spot::Said(_) | Spot::Part(..) => 0,
        Spot::Entry(_) => ENTRY_FIELDS.len() as u64,
        Spot::Value(v) => match &v.kind {
            Kind::Dict(entries) => entries.len() as u64,
            Kind::List(items) | Kind::Tuple(items) => items.len() as u64,
            Kind::Array { .. } => ARRAY_FIELDS.len() as u64,
            _ => 0,
        },
    }
}

/// What a value that holds others is, for the type column and for the count.
/// None for a leaf, which is given the type its bytes are instead.
fn shape_of(value: &Value) -> Option<Shape> {
    match &value.kind {
        Kind::Dict(_) => Some(Shape::Dict),
        Kind::List(_) => Some(Shape::List),
        Kind::Tuple(_) => Some(Shape::Tuple),
        Kind::Array { .. } => Some(Shape::Array),
        _ => None,
    }
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

    /// Where `path` lands in the recognised tree, with the field it counts
    /// from.
    fn pickle_spot<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<(Vec<usize>, Arc<Match>)> {
        let (root, found) = self.pickle_doc(doc, path)?;
        match spot(&found, &path[root.len()..]).is_some() {
            true => Ok((root, found)),
            false => fail("no such value"),
        }
    }

    /// How many values are inside the node at `path`.
    pub(super) fn pickle_child_count<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<u64> {
        let (root, found) = self.pickle_spot(doc, path)?;
        let Some(here) = spot(&found, &path[root.len()..]) else { return fail("no such value") };
        Ok(children(&here))
    }

    /// Which child of the node at `path` is called `name`.
    pub(super) fn pickle_index<S: Source>(&mut self, doc: &Document<S>, path: &[usize], name: &str) -> R<Option<usize>> {
        let (root, found) = self.pickle_spot(doc, path)?;
        let Some(here) = spot(&found, &path[root.len()..]) else { return fail("no such value") };
        let named = |names: &[&str]| Ok(names.iter().position(|n| *n == name));
        match here {
            Spot::Doc => named(&[HEADER_FIELD, DATA_FIELD]),
            Spot::Header => named(&HEADER_FIELDS),
            Spot::Entry(_) => named(&ENTRY_FIELDS),
            Spot::Value(v) => match &v.kind {
                Kind::Array { .. } => named(&ARRAY_FIELDS),
                // A dictionary is reached by the key as it is written in the
                // file, and a list by the index written out as a number.
                Kind::Dict(entries) => {
                    let r = self.memo[path].clone();
                    let base = self.memo[&root].offset;
                    for (i, (key, _)) in entries.iter().enumerate() {
                        if self.pickle_text(doc, &r, base, key)?.as_deref() == Some(name) {
                            return Ok(Some(i));
                        }
                    }
                    Ok(None)
                }
                Kind::List(items) | Kind::Tuple(items) => {
                    Ok(name.parse::<usize>().ok().filter(|i| *i < items.len()))
                }
                _ => Ok(None),
            },
            _ => Ok(None),
        }
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
        let Some(here) = spot(&found, &path[root.len()..]) else { return fail("no such value") };
        let pr = self.memo[parent].clone();
        let base = self.memo[&root].offset;
        // What the parent calls this child, which is the one thing the child
        // itself does not say.
        let name = match spot(&found, &parent[root.len()..]) {
            Some(Spot::Doc) => Name::Field([HEADER_FIELD, DATA_FIELD][idx.min(1)].into()),
            Some(Spot::Header) => Name::Field(HEADER_FIELDS[idx.min(2)].into()),
            Some(Spot::Entry(_)) => Name::Field(ENTRY_FIELDS[idx.min(1)].into()),
            Some(Spot::Value(v)) => match &v.kind {
                Kind::Array { .. } => Name::Field(ARRAY_FIELDS[idx.min(3)].into()),
                Kind::Dict(entries) => match entries.get(idx) {
                    Some((key, _)) => match self.pickle_text(doc, &pr, base, key)? {
                        Some(text) => Name::Field(text.into()),
                        None => Name::Index(idx),
                    },
                    None => Name::Index(idx),
                },
                _ => Name::Index(idx),
            },
            _ => Name::Index(idx),
        };
        match here {
            // The bytes before the object: the protocol byte, and the frame
            // header when the writer wrote one. They say nothing about the
            // object, so what this row holds is what matched it.
            Spot::Header => {
                self.pickle_node(path, &pr, name, Shape::Header, base, 0, found.body);
                Ok(None)
            }
            Spot::Said(0) => self.pickle_note(path, &pr, name, familiar::MESSAGE.to_string()),
            Spot::Said(1) => self.pickle_note(path, &pr, name, found.form.to_string()),
            // The one byte of the envelope with something in it. PROTO is the
            // opcode before it, and the frame's length is the listing's
            // business rather than the object's.
            Spot::Said(_) => Ok(Some(self.pickle_place(&pr, name, T::u8(), base, 1, 1))),
            Spot::Entry(entry) => {
                // The pair, from the first byte of the key to the last byte of
                // the value: the SETITEM that joins them belongs to the
                // dictionary, which is what wrote it.
                let at = entry.0.at;
                let end = entry.1.at + entry.1.len;
                self.pickle_node(path, &pr, name, Shape::Entry, base, at, end - at);
                Ok(None)
            }
            Spot::Value(value) => match shape_of(value) {
                Some(shape) => {
                    self.pickle_node(path, &pr, name, shape, base, value.at, value.len);
                    Ok(None)
                }
                None => match leaf(value) {
                    Some((ty, at, len)) => Ok(Some(self.pickle_place(&pr, name, ty, base, at, len))),
                    None => fail("this value has no type"),
                },
            },
            Spot::Part(value, part) => {
                let Kind::Array { at, len, dtype, dimensions, fortran_order } = &value.kind else {
                    return fail("no such value");
                };
                let note = match part {
                    0 => dtype.clone(),
                    1 => match dimensions.is_empty() {
                        true => NO_DIMENSIONS.to_string(),
                        false => dimensions.iter().map(u64::to_string).collect::<Vec<_>>().join(" x "),
                    },
                    2 => match fortran_order {
                        true => FORTRAN_ORDER.to_string(),
                        false => C_ORDER.to_string(),
                    },
                    // The numbers themselves, read as the dtype says: the same
                    // table the `.npy` reader and the opcode listing use.
                    _ => {
                        let Some(ty) = shapes::run(dtype, count_of(dimensions)) else {
                            return fail("this dtype has no type");
                        };
                        return Ok(Some(self.pickle_place(&pr, name, ty, base, *at, *len)));
                    }
                };
                self.pickle_note(path, &pr, name, note)
            }
            Spot::Doc => fail("a pickle is not inside itself"),
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
        };
        self.remember(path, r);
    }

    /// Where a leaf goes, for the ordinary machinery to read it there.
    fn pickle_place(&self, pr: &Resolved, name: Name, ty: T, base: u64, at: usize, len: usize) -> Place {
        let offset = base + at as u64 * 8;
        Place { name, ty, offset, limit: offset + len as u64 * 8, space: pr.space }
    }
}
