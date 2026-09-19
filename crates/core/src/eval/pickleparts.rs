//! What a node of a recognised pickle is made of: the rows it shows, the
//! children under them, and where in the file each one sits.
//!
//! [`pickletree`](super::pickletree) places these as fields and names them;
//! this is the table of what each kind of value holds, a kind to an arm. A
//! family of values added to the recogniser lands here beside the ones it
//! reads like, and nowhere else in the reading.

use super::pickletree::leaf;
use crate::formats::pickle::familiar::{self, Call, Dtype, Kind, Match, Said, Shape, Storage, Value};

/// What the file holds: what matched, and then the object it matched.
pub(super) const HEADER_FIELD: &str = "header";
pub(super) const DATA_FIELD: &str = "data";
/// What the header says: the contract's sentence, the form that matched, and
/// the protocol the writer used.
pub(super) const MESSAGE_FIELD: &str = "message";
pub(super) const FORM_FIELD: &str = "form";
/// Which extensions of the basic grammar the file turned out to use, which
/// the form only
/// says for a file that holds one. A mixed file needs the row to say anything
/// at all, and every other file gets it for the same reason the `pickler` row
/// is always there: a reader comparing two files wants the same rows in both.
/// The first name is always `basic`, which is the grammar every form reads.
pub(super) const EXTENSIONS_FIELD: &str = "form extensions";
/// Which of CPython's two picklers wrote the file. Most files say nothing
/// either way, and the row says that rather than going missing: a reader
/// comparing two files wants to see the same rows in both.
pub(super) const PICKLER_FIELD: &str = "pickler";
pub(super) const PROTOCOL_FIELD: &str = "protocol";
/// What an array says about itself, and then the numbers themselves. Not
/// `data` again: the object the file holds is already called that, and one
/// name for two things is one thing a reader has to work out.
pub(super) const NUMBERS_FIELD: &str = "numbers";
/// What an array says about how its numbers reached the file, for an array
/// whose numbers are not in it as numbers. Protocol 2 has no opcode for a byte
/// string, so the values go out as the text they spell in latin-1, and the
/// `numbers` row is that text. The table over the array is the numbers
/// themselves, decoded when the form matched.
pub(super) const WRITTEN_FIELD: &str = "written as";
/// What a protocol 0 line's own run is called, under the value it spells. The
/// row above says what the string is; this one is the bytes the file holds.
pub(super) const LINE_FIELD: &str = "line";
/// The run a whole number too wide to be read as one sits in, under the digits
/// it comes to. `line` where the file spelled the digits, which is what
/// protocols 0 and 1 write, and this where it wrote two's-complement bytes.
pub(super) const BYTES_FIELD: &str = "bytes";
pub(super) const LATIN1_TEXT: &str = "latin-1 text";
/// The same at protocol 0, where that text is written as a line and escaped
/// again to fit on one.
pub(super) const ESCAPED_TEXT: &str = "latin-1 text, escaped";
pub(super) const DTYPE_FIELD: &str = "dtype";
pub(super) const SHAPE_FIELD: &str = "shape";
pub(super) const ORDER_FIELD: &str = "order";
pub(super) const KEY_FIELD: &str = "key";
pub(super) const VALUE_FIELD: &str = "value";
/// The two words STACK_GLOBAL joined to make a class's path. `class` is what
/// an object and a call name the class or callable they were made by.
pub(super) const MODULE_FIELD: &str = "module";
pub(super) const NAME_FIELD: &str = "name";
pub(super) const CLASS_FIELD: &str = "class";
/// What a BINGET says: the file wrote this value earlier and named it here
/// rather than writing it again. The row carries what is at the other end,
/// since the reference itself is two bytes that say nothing. A string or a
/// byte string is shown; a container is named and located, so the reader goes
/// to the bytes rather than being handed a copy of them.
pub(super) const REFERS_FIELD: &str = "refers to";
/// The storage orders, spelled the way NumPy spells them.
pub(super) const C_ORDER: &str = "C";
pub(super) const FORTRAN_ORDER: &str = "Fortran";
/// How a shape with no dimensions is written, which is NumPy's own spelling
/// for the shape of a single value.
pub(super) const NO_DIMENSIONS: &str = "()";
/// What a structured dtype's record is called, and what the bytes NumPy left
/// between two of its columns are called. The number after it is how far into
/// the record the run starts, so two runs of padding are told apart.
pub(super) const RECORD_NAME: &str = "record";
pub(super) const PADDING_FIELD: &str = "padding at";
/// What a tensor says about itself beyond its dtype and its shape: how far
/// apart two neighbouring values of each axis are, how far into the storage
/// its first value sits, which storage it is a window onto, which device that
/// storage was on, and whether gradients are kept for it.
pub(super) const STRIDE_FIELD: &str = "stride";
pub(super) const STORAGE_OFFSET_FIELD: &str = "storage offset";
pub(super) const STORAGE_FIELD: &str = "storage";
pub(super) const LOCATION_FIELD: &str = "location";
pub(super) const REQUIRES_GRAD_FIELD: &str = "requires grad";
/// What a tensor is, for the row that says so, when the reader wants the word
/// rather than the shape. The entry row over it says both.
pub(super) const IS_FIELD: &str = "is";
/// What a container was given beyond what is in it, which is what
/// `nn.Module.state_dict()` hangs its `_metadata` off.
pub(super) const ATTRIBUTES_FIELD: &str = "attributes";
/// The whole pickle `joblib.dump` writes in place of an array's numbers when
/// the array holds pickled objects. The values are inside it, and so is a
/// protocol of its own.
pub(super) const NESTED_FIELD: &str = "nested pickle";

/// The fewest dictionaries that make a list of records. One dictionary is a
/// record, not a list of them.
pub(super) const FEWEST_ROWS: usize = 2;
/// What one dictionary of such a list is, which is what the table calls its
/// rows. Not "field": a pickled list's children are instructions as well as
/// dictionaries, and counting those as rows counts neither.
pub(super) const ROW_WORD: &str = "row";

/// The largest file a form is run over. The recogniser reads the whole
/// document at once, the same limit the deduced readings work under.
pub(super) const MOST_BYTES: u64 = 256 << 20;

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
    /// The run a protocol 0 line spells its value in, read as the text it is.
    /// The value is on the node above, which is what the line spells.
    Line(&'a Value),
    /// The run a whole number too wide for any integer type sits in: the
    /// two's-complement bytes, or the digits where the file spelled them. The
    /// number itself is on the node above.
    Wide(&'a Value),
    /// A run of instructions the form matched as one act, and the array it
    /// rebuilt. The numbers are one of the call's arguments in NumPy's
    /// protocol 5 spelling and follow the call in its protocol 4 one, so the
    /// call has to know which.
    Call(&'a Call, &'a Value),
    /// A whole pickle written inside another one, which is what
    /// `joblib.dump` writes where an array of objects would have had its
    /// numbers. The value is the array it holds.
    Nested(&'a Value),
    /// The protocol number a pickle declared, which is the one byte of its
    /// envelope that says anything. The offset is that byte: the file's own
    /// is the second byte of the file, and a nested pickle has one of its own
    /// wherever it starts.
    Protocol(usize),
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
    /// Which storage a tensor is a window onto, which is the key naming the
    /// archive entry or the run that holds its numbers.
    Storage,
    /// The device the storage was on when it was saved.
    Location,
    /// Where a tensor's numbers are, which is not in the pickle.
    Numbers,
    /// Where those bytes are in this file, once the archive has been asked
    /// which entry the key names.
    StoredAt,
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
            Says::Storage => STORAGE_FIELD,
            Says::Location => LOCATION_FIELD,
            Says::Numbers => NUMBERS_FIELD,
            Says::StoredAt => "stored at",
        }
    }
}

/// The summary rows a library object opens with, or nothing for an object
/// whose attributes already are its summary, which is every estimator.
pub(super) fn summary(found: &Match, v: &Value) -> &'static [Says] {
    if super::pickleframe::frame_of(found, v).is_some() {
        return &[Says::Columns, Says::Rows, Says::Index, Says::Dtypes];
    }
    if super::picklecells::is_sparse(v) {
        return &[Says::Shape, Says::Stored, Says::Format];
    }
    &[]
}

/// A shape as a reader says it out loud: `3 x 4`, and NumPy's own spelling for
/// a shape with no dimensions at all.
pub(super) fn extent(dimensions: &[u64]) -> String {
    match dimensions.is_empty() {
        true => NO_DIMENSIONS.to_string(),
        false => dimensions.iter().map(u64::to_string).collect::<Vec<_>>().join(" x "),
    }
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
pub(super) fn span(found: &Match, part: &Part) -> (usize, usize) {
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
        Part::Line(v) => match v.kind {
            Kind::Spelled { at, len, .. } => (at, at + len),
            _ => (0, 0),
        },
        Part::Wide(v) => match v.kind {
            Kind::Wide { at, len, .. } | Kind::Int { at, len, .. } | Kind::Float { at, len, .. } => (at, at + len),
            _ => (0, 0),
        },
        Part::Call(c, _) => (c.at, c.at + c.len),
        // From the PROTO the nested pickle opens with to the STOP it ends
        // with, which is where the value it holds ends.
        Part::Nested(v) => match v.kind {
            Kind::Objects { nested: Some(at), .. } => (at, v.at + v.len),
            _ => (0, 0),
        },
        Part::Protocol(at) => (*at, at + 1),
        Part::Op { at, len } => (*at, at + len),
    }
}

/// The instructions covering `from..to`, clipped to it.
///
/// What is left of an instruction once a value has been taken out of it is the
/// opcode and the length that measured the value, which is the instruction
/// itself: `SHORT_BINUNICODE 5` in front of five bytes of text.
pub(super) fn between<'a>(found: &Match, from: usize, to: usize) -> Vec<(Label, Part<'a>)> {
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
pub(super) fn parts<'a>(found: &'a Match, here: &Part<'a>) -> Vec<(Label, Part<'a>)> {
    let (notes, kids): (Vec<(Label, Part)>, Vec<(Label, Part)>) = match here {
        Part::Doc => (
            Vec::new(),
            vec![
                (Label::Field(HEADER_FIELD), Part::Header),
                (Label::Field(DATA_FIELD), Part::Value(&found.value)),
            ],
        ),
        Part::Header => {
            let mut notes = vec![
                (Label::Field(MESSAGE_FIELD), Part::Note(familiar::MESSAGE.to_string())),
                (Label::Field(FORM_FIELD), Part::Note(found.form.to_string())),
                (Label::Field(EXTENSIONS_FIELD), Part::Note(found.extensions())),
                (Label::Field(PICKLER_FIELD), Part::Note(found.pickler.name().to_string())),
            ];
            // PROTO and the number arrived with protocol 2. Below that the
            // file says nothing, and the number is what the form that read it
            // says, so the row is worked out rather than read.
            let mut kids = Vec::new();
            match found.proto >= 2 {
                true => kids.push((Label::Field(PROTOCOL_FIELD), Part::Protocol(1))),
                false => notes.push((Label::Field(PROTOCOL_FIELD), Part::Note(found.proto.to_string()))),
            }
            (notes, kids)
        }
        // A pickle of its own inside the stream: the protocol it declared,
        // the run of instructions that rebuilt the array, and the values. The
        // FRAME header and the STOP are instructions like any other and are
        // filled in between them.
        Part::Nested(v) => {
            let at = span(found, here).0;
            let Kind::Objects { items, .. } = &v.kind else { return Vec::new() };
            let mut kids = vec![(Label::Field(PROTOCOL_FIELD), Part::Protocol(at + 1))];
            if let Some(call) = call_from(found, at) {
                kids.push((Label::Field(call.name), Part::Call(call, v)));
            }
            kids.extend(items.iter().enumerate().map(|(i, x)| (Label::Index(i), Part::Value(x))));
            (Vec::new(), kids)
        }
        Part::Value(v) if matches!(v.kind, Kind::Spelled { .. }) => (Vec::new(), vec![(Label::Field(LINE_FIELD), Part::Line(v))]),
        // A number no integer type is wide enough to read, with the run it was
        // written in beneath it: the digits where the file spelled them, and
        // the two's-complement bytes otherwise.
        Part::Value(v) if matches!(v.kind, Kind::Wide { .. } | Kind::Int { spelled: true, .. } | Kind::Float { spelled: true, .. }) => {
            let spelled = matches!(v.kind, Kind::Wide { spelled: true, .. } | Kind::Int { spelled: true, .. } | Kind::Float { spelled: true, .. });
            let name = if spelled { LINE_FIELD } else { BYTES_FIELD };
            (Vec::new(), vec![(Label::Field(name), Part::Wide(v))])
        }
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
                let notes = summary(found, v)
                    .iter()
                    .map(|says| (Label::Field(says.name()), Part::Summary { of: v, says: *says }))
                    .collect();
                let mut kids = vec![(Label::Field(CLASS_FIELD), Part::Value(class))];
                kids.extend(held(state));
                (notes, kids)
            }
            // What a call made, with the arguments it was written with: named
            // where Python names them and numbered where it does not. The
            // `class` row is there for a call whose callable is a value of the
            // file, and left out for a builtin a form matched inside a fixed
            // run, which already says what it is.
            Kind::Made { names, callable, items, state, attrs, .. } => {
                let mut kids: Vec<(Label, Part)> =
                    callable.iter().map(|c| (Label::Field(CLASS_FIELD), Part::Value(c))).collect();
                kids.extend(items.iter().enumerate().map(|(i, x)| match names.get(i) {
                    Some(name) => (Label::Field(name), Part::Value(x)),
                    None => (Label::Index(i), Part::Value(x)),
                }));
                kids.extend(held(state));
                // What a BUILD gave a container that was already full, kept
                // as the one dictionary it is rather than mixed in with the
                // contents: a state dict's entries are the weights, and
                // `_metadata` is not one of them.
                kids.extend(attrs.iter().map(|a| (Label::Field(ATTRIBUTES_FIELD), Part::Value(a))));
                (Vec::new(), kids)
            }
            // An array whose values are objects. They were pickled after it and
            // handed to it as a list, so they are nodes of their own rather
            // than a run of bytes to read.
            Kind::Objects { dimensions, fortran_order, items, nested } => {
                let notes = says_array(&Dtype::Objects, dimensions, *fortran_order, Storage::Raw);
                let mut kids = Vec::new();
                if let Some(call) = call_of(found, v) {
                    kids.push((Label::Field(call.name), Part::Call(call, v)));
                }
                // `joblib.dump` has no way to write these values as bytes, so
                // it pickles the array into the stream where the bytes would
                // go. The values are in there, a pickle deeper than the ones
                // beside them.
                match nested {
                    Some(_) => kids.push((Label::Field(NESTED_FIELD), Part::Nested(v))),
                    None => kids.extend(items.iter().enumerate().map(|(i, x)| (Label::Index(i), Part::Value(x)))),
                }
                (notes, kids)
            }
            // A tensor says what it is and where its numbers are, and then
            // the run of instructions that rebuilt it. The numbers are in
            // another entry of the archive or further down the file, so there
            // is nothing under it to read as values: the rows say where to
            // look and the table reads them from there.
            Kind::Tensor(t) => {
                let mut notes = vec![
                    (Label::Field(DTYPE_FIELD), Part::Note(t.dtype.word().to_string())),
                    (Label::Field(SHAPE_FIELD), Part::Note(extent(&t.size))),
                    // Commas rather than the `x` a shape is written with: a
                    // stride is a step per axis and not a shape, and `4 x 1`
                    // beside `3 x 4` reads as a second shape.
                    (Label::Field(STRIDE_FIELD), Part::Note(t.stride.iter().map(u64::to_string).collect::<Vec<_>>().join(", "))),
                    (Label::Field(STORAGE_OFFSET_FIELD), Part::Note(t.offset.to_string())),
                ];
                // Said only of a parameter: every tensor would carry the row
                // and a reader would learn to skip it.
                if t.parameter {
                    notes.insert(0, (Label::Field(IS_FIELD), Part::Note(super::picklesaid::PARAMETER_WORD.to_string())));
                }
                notes.extend(
                    [Says::Storage, Says::Location]
                        .into_iter()
                        .map(|says| (Label::Field(says.name()), Part::Summary { of: v, says })),
                );
                notes.push((Label::Field(REQUIRES_GRAD_FIELD), Part::Note(said_flag(t.requires_grad))));
                notes.extend(
                    [Says::Numbers, Says::StoredAt]
                        .into_iter()
                        .map(|says| (Label::Field(says.name()), Part::Summary { of: v, says })),
                );
                let kids = match call_of(found, v) {
                    Some(call) => vec![(Label::Field(call.name), Part::Call(call, v))],
                    None => Vec::new(),
                };
                (notes, kids)
            }
            Kind::Array { dtype, dimensions, fortran_order, storage, .. } => {
                let notes = says_array(dtype, dimensions, *fortran_order, *storage);
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

/// What a BUILD handed an object or a call, placed directly under it: the
/// entries of the dictionary of attributes, or the items of the tuple a class
/// that spells its own state out is handed.
pub(super) fn held<'a>(state: &'a Option<Box<Value>>) -> Vec<(Label, Part<'a>)> {
    match state.as_deref().map(|s| &s.kind) {
        Some(Kind::Dict(entries)) => entries.iter().enumerate().map(|(i, e)| (Label::Key(i), Part::Entry(e))).collect(),
        // A tuple is the state a class that spells its own out is handed; a
        // list is what the opcodes after a `collections.deque` filled it with.
        Some(Kind::Tuple(items)) | Some(Kind::List(items)) => {
            items.iter().enumerate().map(|(i, x)| (Label::Index(i), Part::Value(x))).collect()
        }
        _ => Vec::new(),
    }
}

/// What an array says about itself before its values: how one of them is read,
/// how many there are and which way round they run.
pub(super) fn says_array<'a>(dtype: &Dtype, dimensions: &[u64], fortran_order: bool, storage: Storage) -> Vec<(Label, Part<'a>)> {
    let shape = extent(dimensions);
    let order = match fortran_order {
        true => FORTRAN_ORDER,
        false => C_ORDER,
    };
    let mut rows = vec![
        (Label::Field(DTYPE_FIELD), Part::Note(dtype.name())),
        (Label::Field(SHAPE_FIELD), Part::Note(shape)),
        (Label::Field(ORDER_FIELD), Part::Note(order.to_string())),
    ];
    // Said only where it is worth saying: an array whose numbers are in the
    // file as numbers has nothing to explain, and a row on every array would
    // be a row every reader learns to skip.
    match storage {
        Storage::Raw => {}
        Storage::Latin1 => rows.push((Label::Field(WRITTEN_FIELD), Part::Note(LATIN1_TEXT.to_string()))),
        Storage::Escaped => rows.push((Label::Field(WRITTEN_FIELD), Part::Note(ESCAPED_TEXT.to_string()))),
    }
    rows
}

/// A flag as Python spells it, which is what a reader of a pickle is
/// comparing against.
fn said_flag(flag: bool) -> String {
    match flag {
        true => "True",
        false => "False",
    }
    .to_string()
}

/// The run of instructions that rebuilt this array, which sits inside it.
pub(super) fn call_of<'a>(found: &'a Match, v: &Value) -> Option<&'a Call> {
    found.calls.iter().find(|c| c.at >= v.at && c.at + c.len <= v.at + v.len)
}

/// The first run of instructions at or after this offset.
///
/// An array `joblib.dump` wrote holds two: the wrapper that stands in front
/// of it, which is what [`call_of`] finds, and the one that rebuilt the array
/// inside the nested pickle.
pub(super) fn call_from(found: &Match, at: usize) -> Option<&Call> {
    found.calls.iter().find(|c| c.at >= at)
}

/// The entries a node's children are keyed by: a dictionary's own, and the
/// state dictionary of an object, whose entries are the object's attributes and
/// are placed directly under it.
pub(super) fn keyed<'a>(part: &Part<'a>) -> Option<&'a Vec<(Value, Value)>> {
    let Part::Value(v) = part else { return None };
    entries_of(v)
}

/// The entries a value holds as a dictionary does: a dictionary's own, and the
/// ones an `OrderedDict`, a `defaultdict` or a `Counter` holds, which travel as
/// the state of the call that made it.
pub(super) fn entries_of(v: &Value) -> Option<&Vec<(Value, Value)>> {
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
pub(super) fn inside(call: &Call, at: usize) -> bool {
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

/// How far the search for a named value will walk before giving up.
///
/// A value is spelled out the first time the file writes it and named out of
/// the memo after that, and what a name points at is wherever the file first
/// wrote it rather than anywhere near the value naming it. So it is looked
/// for, and the look is bounded: a row of a table is not worth an unbounded
/// walk of a file, and a reading that costs too much is no reading.
const MOST_WALKED: usize = 20_000;

/// The value of this shape the file wrote at this offset, which is what a
/// `Names::Made` names, or nothing where the walk ran out.
///
/// A value's span is its whole production, so only what spans the offset is
/// walked into, and an array of a million texts written before or after it
/// is passed over. The walk is bounded and never recursive.
pub(super) fn made_at(found: &Match, what: Shape, at: usize) -> Option<&Value> {
    let spans = |value: &&Value| (value.at..value.at + value.len).contains(&at);
    let mut left = vec![&found.value];
    let mut budget = MOST_WALKED;
    while let Some(value) = left.pop() {
        budget = budget.checked_sub(1)?;
        // An object is a call or a class and the BUILD that filled it, and
        // the memo files both as the one shape. An array is filed as an array
        // whether its values are numbers or pickled after it.
        let is_it = match &value.kind {
            Kind::Made { what: made, .. } => *made == what,
            Kind::Instance { .. } => what == Shape::Object,
            Kind::Array { .. } | Kind::Objects { .. } => what == Shape::Array,
            _ => false,
        };
        if value.at == at && is_it {
            return Some(value);
        }
        // Only into the values that could hold it, which is anything that
        // holds others.
        match &value.kind {
            Kind::List(items) | Kind::Tuple(items) | Kind::Set(items) | Kind::FrozenSet(items) | Kind::Objects { items, .. } => {
                left.extend(items.iter().filter(spans))
            }
            Kind::Dict(entries) => left.extend(entries.iter().flat_map(|(k, v)| [k, v]).filter(spans)),
            Kind::Instance { state, .. } => left.extend(state.as_deref().filter(spans)),
            Kind::Made { items, state, .. } => {
                left.extend(items.iter().filter(spans));
                left.extend(state.as_deref().filter(spans));
            }
            _ => {}
        }
    }
    None
}

/// What a node of the tree is, for the type column. None for a leaf, which is
/// given the type its bytes are instead.
pub(super) fn shape_of(part: &Part) -> Option<Shape> {
    Some(match part {
        Part::Doc => Shape::Doc,
        Part::Header => Shape::Header,
        Part::Entry(_) => Shape::Entry,
        Part::Call(..) => Shape::Call,
        Part::Nested(_) => Shape::Nested,
        Part::Value(v) => match &v.kind {
            Kind::Dict(_) => Shape::Dict,
            Kind::List(_) => Shape::List,
            Kind::Tuple(_) => Shape::Tuple,
            Kind::Set(_) => Shape::Set,
            Kind::FrozenSet(_) => Shape::FrozenSet,
            Kind::Ref(_) => Shape::Ref,
            Kind::Wide { .. } | Kind::Int { spelled: true, .. } => Shape::Integer,
            Kind::Float { spelled: true, .. } => Shape::Real,
            // A protocol 0 line that spells its value rather than being it,
            // which is what the row above the `line` row is worth.
            Kind::Spelled { bytes: true, .. } => Shape::Bytes,
            Kind::Spelled { .. } => Shape::Text,
            Kind::Array { .. } | Kind::Objects { .. } => Shape::Array,
            Kind::Made { what, .. } => *what,
            Kind::Class { .. } => Shape::Class,
            Kind::DType(_) => Shape::DType,
            Kind::Instance { .. } => Shape::Object,
            Kind::Tensor(_) => Shape::Tensor,
            _ => return None,
        },
        _ => return None,
    })
}
