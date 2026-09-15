//! Strict whole-document grammars, independent of the symbolic pickle machine.
//! Only declared opcode sequences are consumed. Captures are committed at EOF.

use std::sync::Arc;

use super::{known::Payload, machine, shapes};
use crate::template::{Deduce, Deduced, Deducer};

pub const MESSAGE: &str = "Matched a Familiar Pickle Form: bypassed Pickle stack machine decoding.";
const MAX_VALUES: usize = 100_000;
const MAX_DEPTH: usize = 64;

/// Captured values preserve Python distinctions and reference payload bytes.
#[derive(Debug, PartialEq)]
pub enum Value {
    None,
    Bool(bool),
    Int(i128),
    Float(f64),
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

#[derive(Debug)]
pub struct Match {
    pub form: &'static str,
    pub value: Value,
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
        (what == Deduce::Builds && at == (self.stop + 1) as u64).then(|| MESSAGE.to_owned())
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
        let code = self.byte()?;
        let len = self.length(code, 0x8c, 0x58, 0x8d)?;
        let at = self.at;
        std::str::from_utf8(self.take(len)?).ok()?;
        self.exact(&[0x94])?;
        Some(Value::Text { at, len })
    }
    fn value(&mut self, depth: usize) -> Option<Value> {
        if depth >= MAX_DEPTH {
            return None;
        }
        self.left = self.left.checked_sub(1)?;
        if matches!(self.peek()?, 0x8c | 0x58 | 0x8d) {
            return self.text();
        }
        Some(match self.byte()? {
            b'N' => Value::None,
            0x88 => Value::Bool(true),
            0x89 => Value::Bool(false),
            b'K' => Value::Int(self.byte()? as i128),
            b'M' => Value::Int(u16::from_le_bytes(self.take(2)?.try_into().ok()?) as i128),
            b'J' => Value::Int(i32::from_le_bytes(self.take(4)?.try_into().ok()?) as i128),
            b'G' => Value::Float(f64::from_be_bytes(self.take(8)?.try_into().ok()?)),
            code @ (b'C' | b'B' | 0x8e) => {
                let len = self.length(code, b'C', b'B', 0x8e)?;
                let at = self.at;
                self.take(len)?;
                self.exact(&[0x94])?;
                Value::Bytes { at, len }
            }
            b')' => Value::Tuple(Vec::new()),
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
                Value::List(values)
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
                Value::Dict(entries)
            }
            _ => return None,
        })
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
        let key = if self.peek() == Some(b'}') {
            self.exact(b"}\x94")?;
            Some(self.text()?)
        } else {
            None
        };
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
        let array = Value::Array {
            at,
            len,
            dtype,
            dimensions,
            fortran_order,
        };
        let value = if let Some(key) = key {
            self.exact(b"s")?;
            Value::Dict(vec![(key, array)])
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
        assert_eq!(
            found.value,
            Value::Dict(vec![(
                Value::Text { at: 15, len: 1 },
                Value::List(vec![Value::Int(1), Value::Int(2)])
            )])
        );
        assert_eq!(
            found.text(Deduce::Builds, bytes.len() as u64).as_deref(),
            Some(MESSAGE)
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
        let Value::Dict(entries) = &found.value else {
            panic!("dict expected")
        };
        let Value::Array {
            at,
            dimensions,
            dtype,
            fortran_order,
            ..
        } = &entries[0].1
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
        let Value::Array {
            at,
            len,
            dtype,
            dimensions,
            fortran_order,
        } = &found.value
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
        let Value::Array { dimensions, .. } = scalar.value else {
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
        let Value::List(values) = found.value else {
            panic!("list")
        };
        assert_eq!(
            &values[..3],
            &[Value::List(vec![]), Value::Int(1), Value::Dict(vec![])]
        );
        assert!(matches!(values[3], Value::Text { len: 1, .. }));
        for (code, width) in [(0x58, 4), (0x8d, 8)] {
            let mut body = vec![code];
            body.extend_from_slice(&300u64.to_le_bytes()[..width]);
            body.extend(std::iter::repeat_n(b'x', 300));
            body.extend_from_slice(b"\x94.");
            assert!(matches!(
                recognise(&framed(&body)).unwrap().value,
                Value::Text { len: 300, .. }
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
        assert_eq!(node.value, crate::eval::Value::Str(MESSAGE.to_owned()));
    }
}
