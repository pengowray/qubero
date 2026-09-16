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
    stop: usize,
    payload: Option<(usize, Payload)>,
}

impl Deduced for Match {
    fn int(&self, what: Deduce, at: u64) -> Option<i128> {
        let (offset, payload) = self.payload?;
        if at != offset as u64 {
            return None;
        }
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

/// Initial envelope: protocol 4/5, either unframed or exactly one whole-body
/// frame. Other framing arrangements remain available through inspection.
pub fn recognise(bytes: &[u8]) -> Option<Match> {
    let mut c = Cursor {
        bytes,
        at: 0,
        left: MAX_VALUES,
    };
    c.exact(&[0x80])?;
    if !matches!(c.byte()?, 4 | 5) {
        return None;
    }
    if c.peek() == Some(0x95) {
        c.byte()?;
        let size = u64::from_le_bytes(c.take(8)?.try_into().ok()?);
        if size != (bytes.len() - c.at) as u64 {
            return None;
        }
    }
    let body = c.at;
    if let Some(value) = c.value(0) {
        if c.exact(b".").is_some() && c.at == bytes.len() {
            return Some(Match {
                form: "basic-p4-p5-v2",
                value,
                body,
                stop: c.at - 1,
                payload: None,
            });
        }
    }
    // A second bounded, exact production, derived from the existing NumPy
    // matrix fixture. No memo interpreter: each reference has a fixed slot.
    c.at = body;
    let (value, at, payload) = c.array()?;
    c.exact(b".")?;
    if c.at != bytes.len() {
        return None;
    }
    Some(Match {
        form: "numpy-numeric-array-p4-p5-v2",
        value,
        body,
        stop: c.at - 1,
        payload: Some((at, payload)),
    })
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
    left: usize,
}

impl<'a> Cursor<'a> {
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
    fn text(&mut self) -> Option<Value> {
        let start = self.at;
        let code = self.byte()?;
        let len = self.length(code, 0x8c, 0x58, 0x8d)?;
        let at = self.at;
        std::str::from_utf8(self.take(len)?).ok()?;
        self.exact(&[0x94])?;
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
    fn value(&mut self, depth: usize) -> Option<Value> {
        if depth >= MAX_DEPTH {
            return None;
        }
        self.left = self.left.checked_sub(1)?;
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
                let len = self.length(code, b'C', b'B', 0x8e)?;
                let at = self.at;
                self.take(len)?;
                self.exact(&[0x94])?;
                Kind::Bytes { at, len }
            }
            b')' => Kind::Tuple(Vec::new()),
            b']' => {
                self.exact(&[0x94])?;
                let mut values = Vec::new();
                if self.peek() == Some(b'(') {
                    self.byte()?;
                    while self.peek()? != b'e' {
                        values.push(self.value(depth + 1)?);
                    }
                    self.byte()?;
                    if values.len() < 2 || values.len() > 1000 {
                        return None;
                    }
                } else {
                    // EMPTY_LIST and a single-item list share a prefix. Try
                    // the whole single-item production before accepting empty.
                    // Rewinding never restores the shared work budget.
                    let start = self.at;
                    if let Some(value) =
                        self.value(depth + 1).filter(|_| self.exact(b"a").is_some())
                    {
                        values.push(value);
                    } else {
                        self.at = start;
                    }
                }
                Kind::List(values)
            }
            b'}' => {
                self.exact(&[0x94])?;
                let mut entries = Vec::new();
                if self.peek() == Some(b'(') {
                    self.byte()?;
                    while self.peek()? != b'u' {
                        entries.push((self.text()?, self.value(depth + 1)?));
                    }
                    self.byte()?;
                    if entries.len() < 2 || entries.len() > 1000 {
                        return None;
                    }
                } else {
                    let start = self.at;
                    let entry = (|| {
                        let key = self.text()?;
                        let value = self.value(depth + 1)?;
                        self.exact(b"s")?;
                        Some((key, value))
                    })();
                    if let Some(entry) = entry {
                        entries.push(entry);
                    } else {
                        self.at = start;
                    }
                }
                Kind::Dict(entries)
            }
            _ => return None,
        };
        Some(self.span(start, kind))
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

    fn dimension(&mut self) -> Option<u64> {
        match self.byte()? {
            b'K' => Some(self.byte()? as u64),
            b'M' => Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?) as u64),
            b'J' => u64::try_from(i32::from_le_bytes(self.take(4)?.try_into().ok()?)).ok(),
            _ => None,
        }
    }

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
            if dims.len() == 32 {
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
            4..=32 if marked => b't',
            _ => return None,
        };
        if end != expected {
            return None;
        }
        self.exact(&[0x94])?;
        Some(dims)
    }

    fn array(&mut self) -> Option<(Value, usize, Payload)> {
        let outer = self.at;
        let key = if self.peek() == Some(b'}') {
            self.exact(b"}\x94")?;
            Some(self.text()?)
        } else {
            None
        };
        let start = self.at;
        // The dictionary and key add exactly two memo slots. No arbitrary
        // memo lookup or stack effect is supported by this production.
        let numpy_slot = if key.is_some() { 5 } else { 3 };
        self.exact(b"\x8c")?;
        match self.byte()? {
            22 => self.exact(b"numpy._core.multiarray")?,
            21 => self.exact(b"numpy.core.multiarray")?,
            _ => return None,
        }
        self.exact(b"\x94\x8c\x0c_reconstruct\x94\x93\x94")?;
        self.exact(
            b"\x8c\x05numpy\x94\x8c\x07ndarray\x94\x93\x94K\0\x85\x94C\x01b\x94\x87\x94R\x94(K\x01",
        )?;
        let dimensions = self.dimensions()?;
        self.exact(&[b'h', numpy_slot])?;
        self.exact(b"\x8c\x05dtype\x94\x93\x94\x8c")?;
        let dtype_len = self.byte()? as usize;
        let kind = std::str::from_utf8(self.take(dtype_len)?).ok()?;
        // Only these plain numeric dtype productions have this exact state.
        if !matches!(
            kind,
            "b1" | "i1"
                | "i2"
                | "i4"
                | "i8"
                | "u1"
                | "u2"
                | "u4"
                | "u8"
                | "f2"
                | "f4"
                | "f8"
                | "c8"
                | "c16"
        ) {
            return None;
        }
        self.exact(b"\x94\x89\x88\x87\x94R\x94(K\x03\x8c\x01")?;
        let order = self.byte()?;
        let one_byte = matches!(kind, "b1" | "i1" | "u1");
        if (one_byte && order != b'|') || (!one_byte && !matches!(order, b'<' | b'>')) {
            return None;
        }
        let dtype = format!("{}{kind}", char::from(order));
        let (shape, width) = shapes::dtype(&dtype)?;
        self.exact(b"\x94NNNJ\xff\xff\xff\xffJ\xff\xff\xff\xffK\0t\x94b")?;
        let fortran_order = match self.byte()? {
            0x89 => false,
            0x88 => true,
            _ => return None,
        };
        let code = self.byte()?;
        let len = self.length(code, b'C', b'B', 0x8e)?;
        // Check every dimension even when another one is zero. Dimensions
        // originate from nonnegative i32 values; the product is bounded too.
        let count = if dimensions.contains(&0) {
            0
        } else {
            dimensions
                .iter()
                .try_fold(1u64, |n, dim| n.checked_mul(*dim))?
        };
        if len as u64 != count.checked_mul(width)? {
            return None;
        }
        let at = self.at;
        self.take(len)?;
        self.exact(b"\x94t\x94b")?;
        let array = self.span(
            start,
            Kind::Array {
                at,
                len,
                dtype,
                dimensions,
                fortran_order,
            },
        );
        let value = if let Some(key) = key {
            self.exact(b"s")?;
            self.span(outer, Kind::Dict(vec![(key, array)]))
        } else {
            array
        };
        Some((value, at, Payload { shape, count }))
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
        assert_eq!(found.form, "basic-p4-p5-v2");
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
            Some(stop_message("basic-p4-p5-v2").as_str())
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
        assert_eq!(found.form, "numpy-numeric-array-p4-p5-v2");
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
        let said = stop_message("basic-p4-p5-v2");
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

    /// Every row the familiar-form template shows for a small dictionary: the
    /// header, the names, the values and the bytes each one sits at.
    #[test]
    fn the_familiar_template_places_the_decoded_values() {
        let bytes = framed(b"}\x94\x8c\x01a\x94]\x94(K\x01K\x02es.");
        let (doc, mut ev) = read(&bytes);
        let seen = |ev: &mut Evaluator, path: &[usize]| {
            let n = ev.node(&doc, path).unwrap();
            (n.name, n.type_name, n.offset_bits / 8, n.size_bits / 8, n.value)
        };
        let root = ev.node(&doc, &[]).unwrap();
        assert_eq!((root.type_name.as_str(), root.child_count), ("pickle", 2));
        // The envelope, read as what matched through it: the protocol byte
        // and the frame header come to eleven bytes here.
        assert_eq!(
            seen(&mut ev, &[0]),
            ("header".into(), "header".into(), 0, 11, V::Composite { count: 3 })
        );
        assert_eq!(
            seen(&mut ev, &[0, 0]),
            ("message".into(), "computed text".into(), 0, 0, V::Str(MESSAGE.into()))
        );
        assert_eq!(
            seen(&mut ev, &[0, 1]),
            ("form".into(), "computed text".into(), 0, 0, V::Str("basic-p4-p5-v2".into()))
        );
        assert_eq!(seen(&mut ev, &[0, 2]), ("protocol".into(), "u8".into(), 1, 1, V::UInt(4)));
        // The dictionary, from its EMPTY_DICT to the SETITEM that filled it.
        assert_eq!(
            seen(&mut ev, &[1]),
            ("data".into(), "dict".into(), 11, 15, V::Composite { count: 1 })
        );
        // The one entry, named by its key, holding the pair it was written as.
        assert_eq!(
            seen(&mut ev, &[1, 0]),
            ("a".into(), "entry".into(), 13, 12, V::Composite { count: 2 })
        );
        assert_eq!(seen(&mut ev, &[1, 0, 0]), ("key".into(), "utf8[]".into(), 15, 1, V::Str("a".into())));
        assert_eq!(
            seen(&mut ev, &[1, 0, 1]),
            ("value".into(), "list".into(), 17, 8, V::Composite { count: 2 })
        );
        // The numbers are at their operand bytes, read as the width the
        // opcode that wrote them gave them.
        assert_eq!(seen(&mut ev, &[1, 0, 1, 0]), ("[0]".into(), "u8".into(), 21, 1, V::UInt(1)));
        assert_eq!(seen(&mut ev, &[1, 0, 1, 1]), ("[1]".into(), "u8".into(), 23, 1, V::UInt(2)));
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
        let (doc, mut ev) = read(&bytes);
        let seen: Vec<_> = (0..7)
            .map(|i| {
                let n = ev.node(&doc, &[1, i]).unwrap();
                (n.type_name, n.size_bits / 8, n.value)
            })
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

    /// A matched array says what it is before it says what it holds, and its
    /// numbers read as the dtype rather than as a run of bytes.
    #[test]
    fn a_matched_array_carries_its_dtype_shape_and_order() {
        let (doc, mut ev) = read(MATRIX);
        assert_eq!(ev.node(&doc, &[0, 1]).unwrap().value, V::Str("numpy-numeric-array-p4-p5-v2".into()));
        let array = ev.node(&doc, &[1, 0, 1]).unwrap();
        assert_eq!((array.name.as_str(), array.type_name.as_str(), array.child_count), ("value", "array", 4));
        let said = |ev: &mut Evaluator, i: usize| ev.node(&doc, &[1, 0, 1, i]).unwrap();
        assert_eq!(said(&mut ev, 0).value, V::Str("<f4".into()));
        assert_eq!(said(&mut ev, 1).value, V::Str("4 x 6".into()));
        assert_eq!(said(&mut ev, 2).value, V::Str("C".into()));
        let data = said(&mut ev, 3);
        assert_eq!((data.name.as_str(), data.size_bits / 8, data.child_count), ("data", 96, 24));
        assert_eq!(ev.node(&doc, &[1, 0, 1, 3, 23]).unwrap().value, V::Float(23.0));
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
}
