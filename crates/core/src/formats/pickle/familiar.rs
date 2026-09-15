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
    F32Matrix {
        key: Box<Value>,
        at: usize,
        rows: u8,
        columns: u8,
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
                form: "basic-p4-p5-v1",
                value,
                stop: c.at - 1,
                payload: None,
            });
        }
    }
    // A second bounded, exact production, derived from the existing NumPy
    // matrix fixture. No memo interpreter: each reference has a fixed slot.
    c.at = body;
    let (value, at, count) = c.matrix()?;
    c.exact(b".")?;
    if c.at != bytes.len() {
        return None;
    }
    let (shape, _) = shapes::dtype("<f4")?;
    Some(Match {
        form: "numpy-f32-matrix-dict-p4-p5-v1",
        value,
        stop: c.at - 1,
        payload: Some((at, Payload { shape, count })),
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
        self.exact(&[0x8c])?;
        let len = self.byte()? as usize;
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
        if self.peek()? == 0x8c {
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
            b'C' => {
                let len = self.byte()? as usize;
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
                } else if !matches!(self.peek()?, b'.' | b'e' | b'u' | b'a' | b's') {
                    values.push(self.value(depth + 1)?);
                    self.exact(b"a")?;
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
                } else if self.peek() == Some(0x8c) {
                    let key = self.text()?;
                    entries.push((key, self.value(depth + 1)?));
                    self.exact(b"s")?;
                }
                Value::Dict(entries)
            }
            _ => return None,
        })
    }

    fn matrix(&mut self) -> Option<(Value, usize, u64)> {
        self.exact(b"}\x94")?;
        let key = self.text()?;
        // These exact strings and memo positions are part of this one form.
        self.exact(b"\x8c\x16numpy._core.multiarray\x94\x8c\x0c_reconstruct\x94\x93\x94")?;
        self.exact(b"\x8c\x05numpy\x94\x8c\x07ndarray\x94\x93\x94K\0\x85\x94C\x01b\x94\x87\x94R\x94(K\x01K")?;
        let rows = self.byte()?;
        self.exact(b"K")?;
        let columns = self.byte()?;
        self.exact(b"\x86\x94h\x05\x8c\x05dtype\x94\x93\x94\x8c\x02f4\x94\x89\x88\x87\x94R\x94")?;
        self.exact(b"(K\x03\x8c\x01<\x94NNNJ\xff\xff\xff\xffJ\xff\xff\xff\xffK\0t\x94b\x89C")?;
        let len = self.byte()? as usize;
        let count = u64::from(rows) * u64::from(columns);
        if len as u64 != count.checked_mul(4)? {
            return None;
        }
        let at = self.at;
        self.take(len)?;
        self.exact(b"\x94t\x94bs")?;
        Some((
            Value::F32Matrix {
                key: Box::new(key),
                at,
                rows,
                columns,
            },
            at,
            count,
        ))
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
        assert_eq!(found.form, "basic-p4-p5-v1");
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
        assert_eq!(found.form, "numpy-f32-matrix-dict-p4-p5-v1");
        let Value::F32Matrix {
            at, rows, columns, ..
        } = found.value
        else {
            panic!("matrix expected")
        };
        assert_eq!((rows, columns), (4, 6));
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
        for offset in [0x30, 0x40, 0x65, 0x6b, 0x79, 0x99, 0x100] {
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

    #[test]
    fn template_publishes_the_match_message_at_stop() {
        use crate::{document::Document, eval::Evaluator, source::MemSource};
        let doc = Document::new(MemSource(vec![0x80, 4, b'N', b'.']));
        let mut ev = Evaluator::new(super::super::pickle());
        let node = ev.node(&doc, &[2, 1]).unwrap();
        assert_eq!(node.value, crate::eval::Value::Str(MESSAGE.to_owned()));
    }
}
