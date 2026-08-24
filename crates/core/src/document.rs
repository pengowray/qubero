//! A document: original source + piece table + add buffer + undo history.

use crate::piece::{AddBuffer, Piece, PieceTable, Src};
use crate::save::{save_plan, Run};
use crate::source::{Missing, Source};

pub struct Document<S: Source> {
    source: S,
    table: PieceTable,
    add: AddBuffer,
    undo: Vec<PieceTable>,
    redo: Vec<PieceTable>,
    /// While a batch is open, edits fold into one undo step.
    batching: bool,
    /// True until the batch's first edit, which is where its snapshot goes:
    /// opening a batch and changing nothing must not leave an undo step
    /// behind.
    batch_pending: bool,
}

impl<S: Source> Document<S> {
    pub fn new(source: S) -> Self {
        let table = PieceTable::new(source.len_bytes() * 8);
        Self { source, table, add: AddBuffer::default(), undo: vec![], redo: vec![], batching: false, batch_pending: false }
    }

    pub fn source(&self) -> &S {
        &self.source
    }
    pub fn source_mut(&mut self) -> &mut S {
        &mut self.source
    }

    pub fn len_bits(&self) -> u64 {
        self.table.len_bits()
    }
    pub fn len_bytes(&self) -> u64 {
        self.table.len_bits().div_ceil(8)
    }
    pub fn piece_count(&self) -> usize {
        self.table.piece_count()
    }
    pub fn is_modified(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    fn snapshot(&mut self) {
        if self.batching && !self.batch_pending {
            // Already inside this batch: still a new edit, so redo is stale.
            self.redo.clear();
            return;
        }
        self.batch_pending = false;
        self.undo.push(self.table.clone());
        self.redo.clear();
    }

    /// Start folding edits into one undo step. Replacing every match is one
    /// thing the user did, and undoing it should be one thing too.
    pub fn begin_batch(&mut self) {
        if !self.batching {
            self.batching = true;
            self.batch_pending = true;
        }
    }

    /// Stop folding. A batch that changed nothing leaves no undo step.
    pub fn end_batch(&mut self) {
        self.batching = false;
        self.batch_pending = false;
    }

    pub fn undo(&mut self) -> bool {
        match self.undo.pop() {
            Some(t) => {
                self.redo.push(std::mem::replace(&mut self.table, t));
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self) -> bool {
        match self.redo.pop() {
            Some(t) => {
                self.undo.push(std::mem::replace(&mut self.table, t));
                true
            }
            None => false,
        }
    }

    pub fn insert_bits(&mut self, at: u64, data: &[u8], n: u64) {
        if n == 0 {
            return;
        }
        self.snapshot();
        let off = self.add.push_bits(data, n);
        self.table.insert(at, Piece { src: Src::Add, bit_off: off, bit_len: n });
    }

    pub fn delete_bits(&mut self, at: u64, n: u64) {
        if n == 0 {
            return;
        }
        self.snapshot();
        self.table.delete(at, n);
    }

    /// Replace `n` bits at `at` with the first `n` bits of `data`. One undo step.
    /// Overwriting past the end extends the document.
    pub fn overwrite_bits(&mut self, at: u64, data: &[u8], n: u64) {
        self.overwrite_bits_inner(at, data, n, false);
    }

    /// Like `overwrite_bits`, but folds into the previous undo step (used when a
    /// single user action, such as typing the second hex digit of a byte, lands
    /// as two writes).
    pub fn amend_overwrite_bits(&mut self, at: u64, data: &[u8], n: u64) {
        self.overwrite_bits_inner(at, data, n, self.can_undo());
    }

    fn overwrite_bits_inner(&mut self, at: u64, data: &[u8], n: u64, amend: bool) {
        if n == 0 {
            return;
        }
        if amend {
            // Still a new edit from the user's point of view: redo history is stale.
            self.redo.clear();
        } else {
            self.snapshot();
        }
        let off = self.add.push_bits(data, n);
        let existing = n.min(self.table.len_bits().saturating_sub(at));
        self.table.delete(at, existing);
        self.table.insert(at, Piece { src: Src::Add, bit_off: off, bit_len: n });
    }

    pub fn insert_bytes(&mut self, at_byte: u64, data: &[u8]) {
        self.insert_bits(at_byte * 8, data, data.len() as u64 * 8);
    }
    pub fn delete_bytes(&mut self, at_byte: u64, n: u64) {
        self.delete_bits(at_byte * 8, n * 8);
    }
    pub fn overwrite_bytes(&mut self, at_byte: u64, data: &[u8]) {
        self.overwrite_bits(at_byte * 8, data, data.len() as u64 * 8);
    }
    pub fn amend_overwrite_bytes(&mut self, at_byte: u64, data: &[u8]) {
        self.amend_overwrite_bits(at_byte * 8, data, data.len() as u64 * 8);
    }

    pub fn add_bytes(&self) -> &[u8] {
        self.add.bytes()
    }

    pub fn save_plan(&self) -> Vec<Run> {
        save_plan(&self.table)
    }

    pub fn read_bits(&self, at: u64, n: u64, out: &mut [u8]) -> Vec<Missing> {
        self.table.read_bits(&self.source, &self.add, at, n, out)
    }

    /// Read bytes; the region past the end of the document is zero-filled.
    pub fn read_bytes(&self, at_byte: u64, out: &mut [u8]) -> Vec<Missing> {
        out.fill(0);
        let n = (out.len() as u64 * 8).min(self.len_bits().saturating_sub(at_byte * 8));
        if n == 0 {
            return vec![];
        }
        self.table.read_bits(&self.source, &self.add, at_byte * 8, n, out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::MemSource;

    fn all(d: &Document<MemSource>) -> Vec<u8> {
        let mut out = vec![0; d.len_bytes() as usize];
        d.read_bytes(0, &mut out);
        out
    }

    #[test]
    fn overwrite_then_undo_redo() {
        let mut d = Document::new(MemSource(b"abcdef".to_vec()));
        d.overwrite_bytes(2, b"XY");
        assert_eq!(all(&d), b"abXYef");
        assert!(d.undo());
        assert_eq!(all(&d), b"abcdef");
        assert!(d.redo());
        assert_eq!(all(&d), b"abXYef");
    }

    #[test]
    fn amend_folds_into_previous_undo_step() {
        let mut d = Document::new(MemSource(b"abcd".to_vec()));
        d.overwrite_bytes(1, b"X");
        d.amend_overwrite_bytes(1, b"Y");
        assert_eq!(all(&d), b"aYcd");
        assert!(d.undo());
        assert_eq!(all(&d), b"abcd");
        assert!(!d.can_undo());
        // An amend after an undo must invalidate redo like any other edit.
        d.overwrite_bytes(0, b"Q");
        d.undo();
        assert!(d.can_redo());
        d.overwrite_bytes(3, b"Z");
        d.amend_overwrite_bytes(3, b"W");
        assert!(!d.can_redo());
        assert_eq!(all(&d), b"abcW");
    }

    #[test]
    fn read_past_end_is_zero_filled() {
        let d = Document::new(MemSource(b"ab".to_vec()));
        let mut out = [0xFFu8; 4];
        d.read_bytes(1, &mut out);
        assert_eq!(out, [b'b', 0, 0, 0]);
    }

    #[test]
    fn random_edits_match_vec_model() {
        let mut model: Vec<u8> = (0..64u8).collect();
        let mut d = Document::new(MemSource(model.clone()));
        let mut seed = 12345u64;
        let mut rnd = |m: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % m
        };
        for _ in 0..500 {
            let len = model.len() as u64;
            match rnd(3) {
                0 => {
                    let at = rnd(len + 1);
                    let v = rnd(256) as u8;
                    model.insert(at as usize, v);
                    d.insert_bytes(at, &[v]);
                }
                1 if len > 0 => {
                    let at = rnd(len);
                    model.remove(at as usize);
                    d.delete_bytes(at, 1);
                }
                _ if len > 0 => {
                    let at = rnd(len);
                    let v = rnd(256) as u8;
                    model[at as usize] = v;
                    d.overwrite_bytes(at, &[v]);
                }
                _ => {}
            }
            assert_eq!(all(&d), model);
        }
    }

    #[test]
    fn random_bit_edits_match_bit_model() {
        // Model the document as a Vec<bool>.
        let orig: Vec<u8> = (0..32u8).map(|i| i.wrapping_mul(37)).collect();
        let mut model: Vec<bool> =
            orig.iter().flat_map(|b| (0..8).map(move |i| (b >> (7 - i)) & 1 == 1)).collect();
        let mut d = Document::new(MemSource(orig));
        let mut seed = 999u64;
        let mut rnd = |m: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % m
        };
        for _ in 0..300 {
            let len = model.len() as u64;
            let n = rnd(13) + 1;
            let data: Vec<u8> = (0..2).map(|_| rnd(256) as u8).collect();
            let bits: Vec<bool> = (0..n).map(|i| (data[(i / 8) as usize] >> (7 - i % 8)) & 1 == 1).collect();
            match rnd(2) {
                0 => {
                    let at = rnd(len + 1);
                    model.splice(at as usize..at as usize, bits);
                    d.insert_bits(at, &data, n);
                }
                _ if len > n => {
                    let at = rnd(len - n);
                    model.drain(at as usize..(at + n) as usize);
                    d.delete_bits(at, n);
                }
                _ => {}
            }
            let mut out = vec![0u8; model.len().div_ceil(8)];
            d.read_bits(0, model.len() as u64, &mut out);
            let got: Vec<bool> = (0..model.len()).map(|i| (out[i / 8] >> (7 - i % 8)) & 1 == 1).collect();
            assert_eq!(got, model);
        }
    }
}
