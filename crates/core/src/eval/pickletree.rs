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
use crate::formats::pickle::familiar::{self, Call, Dtype, Kind, Match, Names, Said, Value};
use crate::formats::pickle::shapes;
use crate::template::{Cells, Encoding, Endian::*, Expr as E, PickleShape as Shape, StrLen, Ty as T};

/// What the file holds: what matched, and then the object it matched.
const HEADER_FIELD: &str = "header";
const DATA_FIELD: &str = "data";
/// What the header says: the contract's sentence, the form that matched, and
/// the protocol the writer used.
const MESSAGE_FIELD: &str = "message";
const FORM_FIELD: &str = "form";
/// Which of CPython's two picklers wrote the file. Most files say nothing
/// either way, and the row says that rather than going missing: a reader
/// comparing two files wants to see the same rows in both.
const PICKLER_FIELD: &str = "pickler";
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
/// The two words STACK_GLOBAL joined to make a class's path. `class` is what
/// an object and a call name the class or callable they were made by.
const MODULE_FIELD: &str = "module";
const NAME_FIELD: &str = "name";
const CLASS_FIELD: &str = "class";
/// What a BINGET says: the file wrote this value earlier and named it here
/// rather than writing it again. The row carries what is at the other end,
/// since the reference itself is two bytes that say nothing. A string or a
/// byte string is shown; a container is named and located, so the reader goes
/// to the bytes rather than being handed a copy of them.
const REFERS_FIELD: &str = "refers to";
/// How much of what a reference names the `refers to` row shows. A repeated
/// dictionary key is a word or two; anything longer is cut rather than filling
/// a row meant to be read at a glance. Fewer bytes than characters, because a
/// byte string is shown in hex and takes three columns a byte.
pub(super) const MOST_SHOWN_TEXT: usize = 120;
const MOST_SHOWN_BYTES: usize = 32;
/// The storage orders, spelled the way NumPy spells them.
const C_ORDER: &str = "C";
const FORTRAN_ORDER: &str = "Fortran";
/// How a shape with no dimensions is written, which is NumPy's own spelling
/// for the shape of a single value.
const NO_DIMENSIONS: &str = "()";
/// What a structured dtype's record is called, and what the bytes NumPy left
/// between two of its columns are called. The number after it is how far into
/// the record the run starts, so two runs of padding are told apart.
const RECORD_NAME: &str = "record";
const PADDING_FIELD: &str = "padding at";

/// The fewest dictionaries that make a list of records. One dictionary is a
/// record, not a list of them.
const FEWEST_ROWS: usize = 2;
/// What one dictionary of such a list is, which is what the table calls its
/// rows. Not "field": a pickled list's children are instructions as well as
/// dictionaries, and counting those as rows counts neither.
const ROW_WORD: &str = "row";

/// The largest file a form is run over. The recogniser reads the whole
/// document at once, the same limit the deduced readings work under.
const MOST_BYTES: u64 = 256 << 20;

/// One node of a recognised file: where a path lands, and what the node is.
#[derive(Clone)]
pub(super) enum Part<'a> {
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
    /// What a BINGET names, read where the file wrote it. The row has no
    /// bytes of its own: the reference is two bytes and the thing it names is
    /// somewhere else entirely.
    Refers(&'a Value),
    /// A named operand inside a run of instructions.
    Text(&'a Said),
    /// A run of instructions the form matched as one act, and the array it
    /// rebuilt. The numbers are one of the call's arguments in NumPy's
    /// protocol 5 spelling and follow the call in its protocol 4 one, so the
    /// call has to know which.
    Call(&'a Call, &'a Value),
    /// The protocol number, which is the one byte of the envelope that says
    /// anything.
    Protocol,
    /// One instruction the form fixed, or what is left of one once the value
    /// inside it has been taken out.
    Op { at: usize, len: usize },
    /// One thing worth knowing about a library object before its structure:
    /// what a frame's columns are, how many rows it has. Worked out from the
    /// match and the file, so it has no bytes of its own.
    Summary { of: &'a Value, says: Says },
}

/// What a summary row of a library object says.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Says {
    /// A frame's column names, a series' one.
    Columns,
    /// How many rows it has.
    Rows,
    /// What its row labels are, and where they run from and to.
    Index,
    /// What one value of each column is.
    Dtypes,
    /// A sparse matrix's shape, how many values it stores, and which of the
    /// sparse layouts it is.
    Shape,
    Stored,
    Format,
}

impl Says {
    /// What the row is called.
    fn name(self) -> &'static str {
        match self {
            Says::Columns => "columns",
            Says::Rows => "rows",
            Says::Index => "index",
            Says::Dtypes => "dtypes",
            Says::Shape => "shape",
            Says::Stored => "stored values",
            Says::Format => "format",
        }
    }
}

/// The summary rows a library object opens with, or nothing for an object
/// whose attributes already are its summary, which is every estimator.
fn summary(v: &Value) -> &'static [Says] {
    if super::pickleframe::frame_of(v).is_some() {
        return &[Says::Columns, Says::Rows, Says::Index, Says::Dtypes];
    }
    if super::picklecells::is_sparse(v) {
        return &[Says::Shape, Says::Stored, Says::Format];
    }
    &[]
}

/// What a node is called by the node above it.
#[derive(Clone)]
pub(super) enum Label {
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
        Part::Refers(_) => (0, 0),
        Part::Summary { .. } => (0, 0),
        Part::Text(s) => (s.at, s.at + s.len),
        Part::Call(c, _) => (c.at, c.at + c.len),
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
                (Label::Field(PICKLER_FIELD), Part::Note(found.pickler.name().to_string())),
            ],
            vec![(Label::Field(PROTOCOL_FIELD), Part::Protocol)],
        ),
        Part::Entry(e) => (
            Vec::new(),
            vec![(Label::Field(KEY_FIELD), Part::Value(&e.0)), (Label::Field(VALUE_FIELD), Part::Value(&e.1))],
        ),
        // The names the call was made with, and the numbers when they are one
        // of its arguments rather than something handed over after it. Both
        // are placed in file order, which is the order the call wrote them.
        Part::Call(c, array) => {
            let mut kids: Vec<(Label, Part)> = c.says.iter().map(|s| (Label::Field(s.name), Part::Text(s))).collect();
            if inside(c, span(found, &Part::Data(array)).0) {
                kids.push((Label::Field(NUMBERS_FIELD), Part::Data(array)));
            }
            kids.sort_by_key(|(_, part)| span(found, part).0);
            (Vec::new(), kids)
        }
        Part::Value(v) => match &v.kind {
            Kind::Dict(entries) => (
                Vec::new(),
                entries.iter().enumerate().map(|(i, e)| (Label::Key(i), Part::Entry(e))).collect(),
            ),
            Kind::List(items) | Kind::Tuple(items) | Kind::Set(items) | Kind::FrozenSet(items) => (
                Vec::new(),
                items.iter().enumerate().map(|(i, x)| (Label::Index(i), Part::Value(x))).collect(),
            ),
            // A reference is the BINGET and a row saying what is at the other
            // end of it, since the two bytes themselves say nothing.
            Kind::Ref(_) => (vec![(Label::Field(REFERS_FIELD), Part::Refers(v))], Vec::new()),
            // A dtype on its own, which says how one value is read. The run
            // of instructions that built it is inside it, with the letters it
            // was given named in that.
            Kind::DType(dtype) => {
                let notes = vec![(Label::Field(DTYPE_FIELD), Part::Note(dtype.name()))];
                let kids = match call_of(found, v) {
                    Some(call) => vec![(Label::Field(call.name), Part::Call(call, v))],
                    None => Vec::new(),
                };
                (notes, kids)
            }
            // A class the file named. The whole dotted path is the node's own
            // value rather than a row under it, so that an object says what it
            // is on one line: these trees are deep, and a row that has to be
            // opened to say `pandas.core.frame.DataFrame` is a row a reader
            // reads twice. The module and the name are the words the file
            // spelled; a class named out of the memo spelled neither.
            Kind::Class { parts, .. } => (
                Vec::new(),
                parts
                    .iter()
                    .zip([MODULE_FIELD, NAME_FIELD])
                    .map(|(x, name)| (Label::Field(name), Part::Value(x)))
                    .collect(),
            ),
            // An object of a named class. Its attributes are the entries of
            // the dictionary the BUILD handed it, placed here rather than a
            // level down: the dictionary is how the state travels and the
            // attributes are what the object is.
            Kind::Instance { class, state } => {
                let notes = summary(v)
                    .iter()
                    .map(|says| (Label::Field(says.name()), Part::Summary { of: v, says: *says }))
                    .collect();
                let mut kids = vec![(Label::Field(CLASS_FIELD), Part::Value(class))];
                kids.extend(held(state));
                (notes, kids)
            }
            // What one of a form's enumerated calls made, with the arguments
            // the library writes it with.
            Kind::Made { names, callable, items, state, .. } => {
                let mut kids = vec![(Label::Field(CLASS_FIELD), Part::Value(callable))];
                kids.extend(items.iter().enumerate().map(|(i, x)| match names.get(i) {
                    Some(name) => (Label::Field(name), Part::Value(x)),
                    None => (Label::Index(i), Part::Value(x)),
                }));
                kids.extend(held(state));
                (Vec::new(), kids)
            }
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
            // An array whose values are objects. They were pickled after it and
            // handed to it as a list, so they are nodes of their own rather
            // than a run of bytes to read.
            Kind::Objects { dimensions, fortran_order, items } => {
                let notes = says_array(&Dtype::Objects, dimensions, *fortran_order);
                let mut kids = Vec::new();
                if let Some(call) = call_of(found, v) {
                    kids.push((Label::Field(call.name), Part::Call(call, v)));
                }
                kids.extend(items.iter().enumerate().map(|(i, x)| (Label::Index(i), Part::Value(x))));
                (notes, kids)
            }
            Kind::Array { dtype, dimensions, fortran_order, .. } => {
                let notes = says_array(dtype, dimensions, *fortran_order);
                let mut kids = Vec::new();
                // The call that rebuilt this array, when it is this array's:
                // a form matches one, and it sits inside the array's bytes.
                let call = call_of(found, v);
                if let Some(call) = call {
                    kids.push((Label::Field(call.name), Part::Call(call, v)));
                }
                // The numbers, unless the call was handed them and holds them.
                if !call.is_some_and(|c| inside(c, span(found, &Part::Data(v)).0)) {
                    kids.push((Label::Field(NUMBERS_FIELD), Part::Data(v)));
                }
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

/// What a reference names, as the row saying so shows it: the text itself,
/// or the bytes in hex when the slot holds a byte string. `whole` is how long
/// the thing is, so that a long one says it was cut.
fn shown(bytes: &[u8], text: bool, whole: usize) -> String {
    let mut said = match text {
        // The read may have stopped inside a character, so take what is whole.
        true => match std::str::from_utf8(bytes) {
            Ok(said) => said.to_string(),
            Err(e) => String::from_utf8_lossy(&bytes[..e.valid_up_to()]).into_owned(),
        },
        false => bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "),
    };
    if bytes.len() < whole {
        said.push_str("...");
    }
    said
}

/// What a BUILD handed an object or a call, placed directly under it: the
/// entries of the dictionary of attributes, or the items of the tuple a class
/// that spells its own state out is handed.
fn held<'a>(state: &'a Option<Box<Value>>) -> Vec<(Label, Part<'a>)> {
    match state.as_deref().map(|s| &s.kind) {
        Some(Kind::Dict(entries)) => entries.iter().enumerate().map(|(i, e)| (Label::Key(i), Part::Entry(e))).collect(),
        Some(Kind::Tuple(items)) => items.iter().enumerate().map(|(i, x)| (Label::Index(i), Part::Value(x))).collect(),
        _ => Vec::new(),
    }
}

/// What an array says about itself before its values: how one of them is read,
/// how many there are and which way round they run.
fn says_array<'a>(dtype: &Dtype, dimensions: &[u64], fortran_order: bool) -> Vec<(Label, Part<'a>)> {
    let shape = match dimensions.is_empty() {
        true => NO_DIMENSIONS.to_string(),
        false => dimensions.iter().map(u64::to_string).collect::<Vec<_>>().join(" x "),
    };
    let order = match fortran_order {
        true => FORTRAN_ORDER,
        false => C_ORDER,
    };
    vec![
        (Label::Field(DTYPE_FIELD), Part::Note(dtype.name())),
        (Label::Field(SHAPE_FIELD), Part::Note(shape)),
        (Label::Field(ORDER_FIELD), Part::Note(order.to_string())),
    ]
}

/// The run of instructions that rebuilt this array, which sits inside it.
fn call_of<'a>(found: &'a Match, v: &Value) -> Option<&'a Call> {
    found.calls.iter().find(|c| c.at >= v.at && c.at + c.len <= v.at + v.len)
}

/// The entries a node's children are keyed by: a dictionary's own, and the
/// state dictionary of an object, whose entries are the object's attributes and
/// are placed directly under it.
fn keyed<'a>(part: &Part<'a>) -> Option<&'a Vec<(Value, Value)>> {
    let Part::Value(v) = part else { return None };
    match &v.kind {
        Kind::Dict(entries) => Some(entries),
        Kind::Instance { state: Some(state), .. } | Kind::Made { state: Some(state), .. } => match &state.kind {
            Kind::Dict(entries) => Some(entries),
            _ => None,
        },
        _ => None,
    }
}

/// Whether a byte of the file sits inside a matched run of instructions.
fn inside(call: &Call, at: usize) -> bool {
    (call.at..call.at + call.len).contains(&at)
}

/// Where `path`, counted from the pickle field, lands.
pub(super) fn spot<'a>(found: &'a Match, path: &[usize]) -> Option<(Label, Part<'a>)> {
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
        Part::Call(..) => Shape::Call,
        Part::Value(v) => match &v.kind {
            Kind::Dict(_) => Shape::Dict,
            Kind::List(_) => Shape::List,
            Kind::Tuple(_) => Shape::Tuple,
            Kind::Set(_) => Shape::Set,
            Kind::FrozenSet(_) => Shape::FrozenSet,
            Kind::Ref(_) => Shape::Ref,
            Kind::Array { .. } | Kind::Objects { .. } => Shape::Array,
            Kind::Object { what, .. } => *what,
            Kind::Class { .. } => Shape::Class,
            Kind::DType(_) => Shape::DType,
            Kind::Instance { .. } => Shape::Object,
            Kind::Made { what, .. } => *what,
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
        // BININT1 is unsigned and BININT2 is too; BININT is signed, and so is
        // LONG1, whose run of bytes is as long as the number needs.
        Kind::Int { at, len, .. } => {
            let ty = match len {
                1 => T::u8(),
                2 => T::u16(Little),
                4 => T::i32(Little),
                bytes => T::Int { bits: *bytes as u32 * 8, endian: Little },
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

/// The type a run of an array's values reads as: one element repeated by the
/// shape, where an element is one number or, for a structured dtype, a record
/// of named columns with the padding NumPy left between them named too.
fn numbers_ty(dtype: &Dtype, count: u64) -> Option<T> {
    match dtype {
        // A datetime is a count of its unit, and the type NumPy's table gives
        // it says which unit, so a reader of the numbers sees that too.
        Dtype::Plain(spelling) | Dtype::Datetime { spelling, .. } => shapes::run(spelling, count),
        Dtype::Record { columns, width } => {
            let mut fields: Vec<(String, T)> = Vec::new();
            let mut reached = 0u64;
            for column in columns {
                if column.at > reached {
                    fields.push((format!("{PADDING_FIELD} {reached}"), T::bytes(E::lit((column.at - reached) as i128))));
                }
                let (elem, held) = shapes::element(&column.dtype)?;
                fields.push((column.name.clone(), elem));
                reached = column.at + held;
            }
            if *width > reached {
                fields.push((format!("{PADDING_FIELD} {reached}"), T::bytes(E::lit((width - reached) as i128))));
            }
            let named: Vec<(&str, T)> = fields.iter().map(|(n, t)| (n.as_str(), t.clone())).collect();
            Some(T::array(T::structure(RECORD_NAME, named), E::lit(count as i128)))
        }
        // An array of objects has no run of bytes to read: its values are
        // nodes of their own, wherever the file pickled them.
        Dtype::Objects => None,
    }
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
    /// The table the numbers of a matched array are, or nothing for any other
    /// node. A pickled array is the same rows and columns a `.npy` file holds,
    /// and the shape that says so is in the match rather than in a field of a
    /// structure, which is where [`Evaluator::table_shape`] looks otherwise.
    ///
    /// Read from what the match left behind and never from the file: this is
    /// asked of every node on its way to the screen, and a node under a
    /// matched pickle was placed by that match, so it is already there.
    ///
    /// The run is written along the last dimension in C order and along the
    /// first in Fortran order, which is what a row of it is either way. One
    /// dimension is one value a row. No dimensions is one value and no table.
    pub(super) fn pickle_table(&self, path: &[usize]) -> Option<crate::template::TableShape> {
        let k = (0..=path.len()).rev().find(|k| matches!(self.memo.get(&path[..*k]).map(|r| &r.ty), Some(Ty::Pickle(Shape::Doc))))?;
        let found = self.memo.pickle(&path[..k])?;
        let here = spot(found, &path[k..])?;
        // A pandas frame or series, whose cells are spread across blocks and
        // worked out rather than read. Recognised without touching the file,
        // because this is asked of every node on its way to the screen; the
        // columns and the row count are read in
        // [`Evaluator::pickle_columns`].
        if let (_, Part::Value(v)) = &here {
            if super::pickleframe::frame_of(v).is_some() {
                return Some(crate::template::TableShape {
                    row_word: Some(ROW_WORD.into()),
                    cells: Some(Cells::Computed { rows: 0 }),
                    ..Default::default()
                });
            }
        }
        // A list of dictionaries, which is how rows are pickled when nobody
        // reached for pandas. Its columns are the keys, which are written in
        // the file beside the values, so the shape says where a cell is rather
        // than how many values make a row. The names are read in
        // [`Evaluator::pickle_columns`], which can reach the bytes.
        if let (_, Part::Value(v)) = &here {
            if let Kind::List(items) = &v.kind {
                if items.len() >= FEWEST_ROWS && items.iter().all(|x| matches!(x.kind, Kind::Dict(_))) {
                    return Some(crate::template::TableShape {
                        row_word: Some(ROW_WORD.into()),
                        cells: Some(Cells::Named {
                            row: Shape::Dict.name().into(),
                            cell: Shape::Entry.name().into(),
                            value: Some(VALUE_FIELD.into()),
                        }),
                        ..Default::default()
                    });
                }
            }
        }
        let (_, Part::Data(v)) = here else { return None };
        let Kind::Array { dimensions, fortran_order, .. } = &v.kind else { return None };
        let inner = match (dimensions.len(), fortran_order) {
            (0, _) => return None,
            (1, _) => None,
            (_, true) => dimensions.first().copied(),
            (_, false) => dimensions.last().copied(),
        };
        Some(crate::template::TableShape {
            columns: inner.filter(|n| *n > 0).map(|n| E::lit(n as i128)),
            // Several numbers to a row is a row. One to a row is a value,
            // which is what the table calls it when it is told nothing.
            row_word: inner.map(|_| "row".into()),
            ..Default::default()
        })
    }

    /// The columns of a pickled table, read where the file wrote them.
    ///
    /// Only for a shape that says its cells are named: a run of numbers has no
    /// names to read. The columns are every key any row has, in the order they
    /// are first met, so a row missing one has an empty cell under it rather
    /// than the next key's value.
    pub(super) fn pickle_columns<S: Source>(
        &mut self,
        doc: &Document<S>,
        path: &[usize],
        shape: crate::template::TableShape,
    ) -> R<Option<crate::template::TableShape>> {
        // A frame's columns, its row count and what one value of each column
        // is are all read at once, because none of them is in the node.
        if matches!(shape.cells, Some(Cells::Computed { .. })) {
            return self.frame_shape(doc, path);
        }
        if !matches!(shape.cells, Some(Cells::Named { .. })) {
            return Ok(Some(shape));
        }
        let (root, found) = self.pickle_doc(doc, path)?;
        let Some((_, Part::Value(v))) = spot(&found, &path[root.len()..]) else { return Ok(Some(shape)) };
        let Kind::List(items) = &v.kind else { return Ok(Some(shape)) };
        let r = self.memo[&root].clone();
        let base = r.offset;
        let mut names: Vec<Arc<str>> = Vec::new();
        for row in items {
            let Kind::Dict(entries) = &row.kind else { continue };
            for (key, _) in entries {
                if let Some(name) = self.pickle_text(doc, &r, base, key)? {
                    if !names.iter().any(|seen| **seen == *name) {
                        names.push(name.into());
                    }
                }
            }
        }
        Ok(Some(crate::template::TableShape { names, ..shape }))
    }

    /// The path of the pickle field `path` sits in, and what the recogniser
    /// made of it. `path` may be the field itself or any value inside it.
    pub(super) fn pickle_doc<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<(Vec<usize>, Arc<Match>)> {
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
        // A key may be a reference to a string written anywhere else in the
        // file, so a key is read through the pickle field rather than through
        // the node it is a key of.
        let r = self.memo[&root].clone();
        let base = r.offset;
        for (i, (label, _)) in parts(&found, &here).into_iter().enumerate() {
            let said = match label {
                Label::Field(f) => f == name,
                Label::Index(n) => name.parse::<usize>().ok() == Some(n),
                Label::Key(n) => match keyed(&here).and_then(|entries| entries.get(n)) {
                    Some((key, _)) => self.pickle_text(doc, &r, base, key)?.as_deref() == Some(name),
                    None => false,
                },
            };
            if said {
                return Ok(Some(i));
            }
        }
        Ok(None)
    }

    /// What a dictionary entry is called, which is its key when the key is
    /// something a name can be.
    ///
    /// A string spelled out, a string the file named instead of spelling
    /// again, and a whole number, which is the key a table of records keyed
    /// by row number has. Anything else leaves the entry numbered by its
    /// place, because a float, a byte string or a tuple written out as a name
    /// would read as something the file says and is not.
    ///
    /// `r` is the pickle field itself: a named string sits wherever the file
    /// first wrote it, which is outside the entry that names it.
    pub(super) fn pickle_text<S: Source>(&self, doc: &Document<S>, r: &Resolved, base: u64, key: &Value) -> R<Option<String>> {
        let (at, len) = match key.kind {
            Kind::Text { at, len } | Kind::Ref(Names::Text { at, len }) => (at, len),
            Kind::Int { value, .. } => return Ok(Some(value.to_string())),
            _ => return Ok(None),
        };
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
        // The whole pickle field, which is what a reference is read through:
        // what it names sits wherever the file first wrote it.
        let whole = self.memo[&root].clone();
        let base = whole.offset;
        let name = match label {
            Label::Field(f) => Name::Field(f.into()),
            Label::Index(n) => Name::Index(n),
            Label::Key(n) => match keyed(&above).and_then(|entries| entries.get(n)) {
                Some((key, _)) => match self.pickle_text(doc, &whole, base, key)? {
                    Some(text) => Name::Field(text.into()),
                    None => Name::Index(n),
                },
                None => Name::Index(n),
            },
        };
        let (at, end) = span(&found, &part);
        // A node that holds others is placed here and read no further.
        if let Some(shape) = shape_of(&part) {
            let said = match &part {
                Part::Value(v) => match &v.kind {
                    Kind::Class { path, .. } => Some(path.clone()),
                    _ => None,
                },
                // An entry reads as what it holds. A fitted model is thirty
                // attributes, and a row each saying `2 fields` makes a reader
                // open all thirty to find the one that is `True`.
                Part::Entry(e) => self.pickle_said(doc, &whole, base, &e.1)?,
                _ => None,
            };
            self.pickle_node(path, &pr, name, shape, base, at, end - at, said);
            return Ok(None);
        }
        match part {
            Part::Note(text) => self.pickle_note(path, &pr, name, text),
            // The instructions the form fixed. Named for what they are, and
            // marked as the structure's own machinery: a reader following the
            // data wants them folded, and a reader following the program
            // wants them named.
            Part::Op { .. } => Ok(Some(self.pickle_place(&pr, name, T::bytes(E::lit((end - at) as i128)), base, at, end - at, true))),
            // What a reference names, read where the file wrote it. The row
            // is worked out rather than read in place: its bytes are not
            // inside the reference, which is only the BINGET.
            Part::Refers(v) => {
                let Kind::Ref(names) = v.kind else { return fail("no such value") };
                let said = match names {
                    // A container is not copied into the row. It is named for
                    // what it is and for where the file wrote it, so that the
                    // reader is sent to those bytes: a copy here would be a
                    // value the file says twice and holds once, and a
                    // container a reference sits inside is not finished yet.
                    Names::Made { what, at, .. } => format!("{} at {:#04x}", what.name(), base as usize / 8 + at),
                    Names::Text { at, len } | Names::Bytes { at, len } => {
                        let text = matches!(names, Names::Text { .. });
                        let read = len.min(if text { MOST_SHOWN_TEXT } else { MOST_SHOWN_BYTES });
                        let bytes = self.read(doc, &whole, base + at as u64 * 8, read as u64 * 8)?;
                        shown(&bytes, text, len)
                    }
                };
                self.pickle_note(path, &pr, name, said)
            }
            // What a reader came for, before the structure that holds it.
            Part::Summary { of, says } => {
                let said = self.pickle_summary(doc, &root, path, of, says)?;
                self.pickle_note(path, &pr, name, said)
            }
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
                let Some(ty) = numbers_ty(dtype, count_of(dimensions)) else {
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

    /// One value in a few words, for the row of the entry that holds it: the
    /// value itself when it is a single thing, and what kind of thing and how
    /// much of it otherwise. Nothing for a value there is no short word for,
    /// which leaves the row counting its fields as it did.
    fn pickle_said<S: Source>(&self, doc: &Document<S>, whole: &Resolved, base: u64, v: &Value) -> R<Option<String>> {
        let mut read = |at: usize, len: usize, text: bool| -> R<String> {
            let most = len.min(if text { MOST_SHOWN_TEXT } else { MOST_SHOWN_BYTES });
            Ok(shown(&self.read(doc, whole, base + at as u64 * 8, most as u64 * 8)?, text, len))
        };
        let many = |what: &str, n: usize| if n == 0 { format!("empty {what}") } else { format!("{what} of {n}") };
        let across = |dimensions: &[u64]| dimensions.iter().map(u64::to_string).collect::<Vec<_>>().join(" x ");
        let path_of = |v: &Value| match &v.kind {
            Kind::Class { path, .. } => Some(path.clone()),
            _ => None,
        };
        Ok(match &v.kind {
            Kind::None => Some("None".into()),
            Kind::Bool(b) => Some(if *b { "True" } else { "False" }.into()),
            Kind::Int { value, .. } => Some(value.to_string()),
            Kind::Float { value, .. } => Some(value.to_string()),
            Kind::Text { at, len } | Kind::Ref(Names::Text { at, len }) => Some(read(*at, *len, true)?),
            Kind::Bytes { at, len } | Kind::Ref(Names::Bytes { at, len }) => Some(read(*at, *len, false)?),
            Kind::Ref(Names::Made { what, at, .. }) => Some(format!("{} at {:#04x}", what.name(), base as usize / 8 + at)),
            Kind::List(items) => Some(many("list", items.len())),
            Kind::Tuple(items) => Some(many("tuple", items.len())),
            Kind::Set(items) => Some(many("set", items.len())),
            Kind::FrozenSet(items) => Some(many("frozenset", items.len())),
            Kind::Dict(entries) => Some(many("dict", entries.len())),
            Kind::Array { dtype: Dtype::Plain(spelling) | Dtype::Datetime { spelling, .. }, dimensions, .. } => Some(format!("{spelling} array {}", across(dimensions))),
            Kind::Array { dimensions, .. } => Some(format!("record array {}", across(dimensions))),
            Kind::Objects { dimensions, .. } => Some(format!("object array {}", across(dimensions))),
            Kind::Class { path, .. } => Some(path.clone()),
            Kind::Instance { class, .. } => path_of(class),
            Kind::Made { callable, .. } => path_of(callable),
            Kind::Object { what, .. } => Some(what.name().into()),
            Kind::DType(Dtype::Plain(spelling) | Dtype::Datetime { spelling, .. }) => Some(spelling.clone()),
            Kind::DType(_) => None,
        })
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
    fn pickle_node(&mut self, path: &[usize], pr: &Resolved, name: Name, shape: Shape, base: u64, at: usize, len: usize, said: Option<String>) {
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
            computed: said.map(|said| Computed::Text(said.as_str().into())),
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
