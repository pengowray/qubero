//! What every test file here is written with: the envelope a body is wrapped
//! in, the fixture and the edits made to it, the handwritten pickles, and the
//! rows the template reads. The tests themselves sit in the files beside this
//! one, a file to a topic.

use super::*;
use crate::formats::pickle::shapes;

mod arrays;
mod frames;
mod grammar;
mod objects;
mod tree;

/// A plain dtype's spelling, which is what an array test asserts about. A
/// structured one has columns instead and is asserted about by those.
fn spelling(dtype: &Dtype) -> &str {
    match dtype {
        Dtype::Plain(said) => said.as_str(),
        Dtype::Record { .. } => panic!("a plain dtype was expected, not {dtype:?}"),
    }
}

fn framed(body: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0x80, 4, 0x95];
    bytes.extend_from_slice(&(body.len() as u64).to_le_bytes());
    bytes.extend_from_slice(body);
    bytes
}

/// The same at protocol 5, for the productions that exist only there.
fn proto5(body: &[u8]) -> Vec<u8> {
    let mut bytes = framed(body);
    bytes[1] = 5;
    bytes
}

const MATRIX: &[u8] = include_bytes!("../../../../../tests/fixtures/pickle/numpy-f32-matrix.pickle");

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

/// The two values a pickle writes as an opcode and nothing else read as
/// the word Python spells them with.
fn named(raw: i128, name: &str) -> V {
    V::Enum { raw, name: Some(name.to_string()), hex: false }
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

/// The dtype construction and the BUILD that gives it its byte order.
/// `module` is how the class's module is named, which is a reference to
/// the slot it was written in where an array wrote it first and the word
/// itself where nothing did.
///
/// Counting from the slot its first word lands in: 1 is the `dtype` text,
/// 2 the `numpy.dtype` class, 5 the finished dtype, and 6 the byte order.
fn dtype_state(module: &[u8], kind: &str, order: u8) -> Vec<u8> {
    cat(&[
        module,
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
        &dtype_state(&get(base + 3), kind, order),
        b"\x89",
        &blob(data),
        b"t\x94b",
    ])
}

/// A file being written a production at a time, counting the memo slots as it
/// goes.
///
/// A structured dtype names its columns twice, as a tuple of words and then as
/// the keys of the dictionary that gives each a type and an offset, and the
/// second time it names the slots the first one filed. Working those slot
/// numbers out by hand is how a test stops saying what it means, so this
/// counts them instead.
#[derive(Default)]
struct Writing {
    out: Vec<u8>,
    slots: usize,
}

impl Writing {
    fn raw(&mut self, bytes: &[u8]) -> &mut Self {
        self.out.extend_from_slice(bytes);
        self
    }
    /// MEMOIZE, and the slot it files.
    fn mark(&mut self) -> usize {
        self.out.push(0x94);
        self.slots += 1;
        self.slots - 1
    }
    fn word(&mut self, text: &str) -> usize {
        self.out.push(0x8c);
        self.out.push(text.len() as u8);
        self.out.extend_from_slice(text.as_bytes());
        self.mark()
    }
    fn get(&mut self, slot: usize) -> &mut Self {
        self.raw(&[b'h', slot as u8])
    }
    /// A nonnegative integer in the width CPython writes it in, up to the two
    /// a test needs.
    fn count(&mut self, n: u64) -> &mut Self {
        match n {
            0..=255 => self.raw(&[b'K', n as u8]),
            _ => self.raw(&[b'M', n as u8, (n >> 8) as u8]),
        }
    }
    /// The `numpy.dtype` construction for one number a value.
    fn plain_dtype(&mut self, kind: &str, order: &str) -> &mut Self {
        self.word("numpy");
        self.word("dtype");
        self.raw(b"\x93");
        self.mark();
        self.word(kind);
        self.raw(b"\x89\x88\x87");
        self.mark();
        self.raw(b"R");
        self.mark();
        self.raw(b"(K\x03");
        self.word(order);
        self.raw(b"NNNJ\xff\xff\xff\xffJ\xff\xff\xff\xffK\0t");
        self.mark();
        self.raw(b"b")
    }
    /// The same construction for a record of named columns: the names, the
    /// dictionary of types and offsets, and the width, alignment and flags.
    fn record_dtype(&mut self, width: u64, columns: &[(&str, &str, &str, u64)]) -> &mut Self {
        self.word("numpy");
        self.word("dtype");
        self.raw(b"\x93");
        self.mark();
        self.word(&format!("V{width}"));
        self.raw(b"\x89\x88\x87");
        self.mark();
        self.raw(b"R");
        self.mark();
        self.raw(b"(K\x03");
        self.word("|");
        // Three names or fewer are written with TUPLE1 to TUPLE3 and any more
        // over a MARK, which is what CPython writes for a tuple of any size.
        let marked = columns.len() > 3;
        self.raw(if marked { b"N(" } else { b"N" });
        let named: Vec<usize> = columns.iter().map(|(name, ..)| self.word(name)).collect();
        match marked {
            true => self.raw(b"t"),
            false => self.raw(&[0x84 + columns.len() as u8]),
        };
        self.mark();
        self.raw(b"}");
        self.mark();
        let batched = columns.len() > 1;
        if batched {
            self.raw(b"(");
        }
        for (slot, (_, kind, order, at)) in named.iter().zip(columns) {
            self.get(*slot);
            self.plain_dtype(kind, order);
            self.count(*at);
            self.raw(b"\x86");
            self.mark();
        }
        self.raw(if batched { b"u" } else { b"s" });
        self.count(width);
        self.raw(b"K\x01K\x10t");
        self.mark();
        self.raw(b"b")
    }
    /// An array of records: the reconstructor, one dimension, the dtype, C
    /// order and the bytes.
    fn record_array(&mut self, rows: u64, width: u64, columns: &[(&str, &str, &str, u64)], data: &[u8]) -> &mut Self {
        self.word("numpy._core.multiarray");
        self.word("_reconstruct");
        self.raw(b"\x93");
        self.mark();
        self.word("numpy");
        self.word("ndarray");
        self.raw(b"\x93");
        self.mark();
        self.raw(b"K\0\x85");
        self.mark();
        self.raw(b"C\x01b");
        self.mark();
        self.raw(b"\x87");
        self.mark();
        self.raw(b"R");
        self.mark();
        self.raw(b"(K\x01");
        self.count(rows);
        self.raw(b"\x85");
        self.mark();
        self.record_dtype(width, columns);
        self.raw(b"\x89C");
        self.raw(&[data.len() as u8]);
        self.raw(data);
        self.mark();
        self.raw(b"t");
        self.mark();
        self.raw(b"b")
    }
}

/// BYTEARRAY8 and the memo mark after it.
fn mutable(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x96];
    out.extend_from_slice(&(data.len() as u64).to_le_bytes());
    out.extend_from_slice(data);
    out.push(0x94);
    out
}

/// `numpy.core.numeric._frombuffer(buffer, dtype, shape, order)`, which is
/// what NumPy writes at protocol 5. Nothing is written before it, so the
/// dtype class names its module with the word rather than out of a slot.
fn frombuffer(buffer: &[u8], kind: &str, order: u8, shape: &[u8], letter: &str) -> Vec<u8> {
    cat(&[
        &word("numpy.core.numeric"),
        &word("_frombuffer"),
        b"\x93\x94(",
        buffer,
        &dtype_state(&word("numpy"), kind, order),
        shape,
        &word(letter),
        b"t\x94R\x94",
    ])
}

/// A file in two frames, split at `at` bytes into the body.
fn in_two_frames(body: &[u8], at: usize) -> Vec<u8> {
    let mut out = vec![0x80, 4, 0x95];
    out.extend_from_slice(&(at as u64).to_le_bytes());
    out.extend_from_slice(&body[..at]);
    out.push(0x95);
    out.extend_from_slice(&((body.len() - at) as u64).to_le_bytes());
    out.extend_from_slice(&body[at..]);
    out
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
        &dtype_state(&get(5), "i1", b'|'),
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
