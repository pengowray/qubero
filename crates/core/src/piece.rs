//! Bit-granular piece table.
//!
//! The document is a sequence of pieces, each a bit range of either the immutable
//! original (`Orig`) or the append-only `add` buffer. Edits never touch the original.
//!
//! Pieces live in a `Vec` with cached cumulative offsets, so lookups are O(log n)
//! and edits O(n) in the number of pieces. That is plenty until a session has
//! tens of thousands of edits; swapping in a balanced tree (red-black piece
//! tree) is a contained change behind this type's API.

use crate::bits::{bytes_for, copy_bits};
use crate::source::{Missing, Source};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Src {
    Orig,
    Add,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Piece {
    pub src: Src,
    pub bit_off: u64,
    pub bit_len: u64,
}

#[derive(Debug, Clone)]
pub struct PieceTable {
    pieces: Vec<Piece>,
    /// starts[i] = bit offset of pieces[i] in the document; starts[len] = total bits.
    starts: Vec<u64>,
}

/// The append-only add buffer. Shared by the table and all its undo snapshots,
/// which is why it lives outside `PieceTable`.
#[derive(Debug, Default, Clone)]
pub struct AddBuffer {
    bytes: Vec<u8>,
    bits: u64,
}

impl AddBuffer {
    pub fn len_bits(&self) -> u64 {
        self.bits
    }

    /// Append `n` bits from `data` (MSB-first). Returns the bit offset they start at.
    pub fn push_bits(&mut self, data: &[u8], n: u64) -> u64 {
        let start = self.bits;
        let needed = bytes_for(self.bits + n);
        self.bytes.resize(needed, 0);
        copy_bits(data, 0, &mut self.bytes, start, n);
        self.bits += n;
        start
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl PieceTable {
    pub fn new(orig_bits: u64) -> Self {
        let pieces = if orig_bits == 0 {
            vec![]
        } else {
            vec![Piece { src: Src::Orig, bit_off: 0, bit_len: orig_bits }]
        };
        let mut t = Self { pieces, starts: vec![] };
        t.rebuild_starts();
        t
    }

    pub fn len_bits(&self) -> u64 {
        *self.starts.last().unwrap_or(&0)
    }

    pub fn piece_count(&self) -> usize {
        self.pieces.len()
    }

    pub fn pieces(&self) -> &[Piece] {
        &self.pieces
    }

    fn rebuild_starts(&mut self) {
        self.starts.clear();
        self.starts.reserve(self.pieces.len() + 1);
        let mut acc = 0;
        self.starts.push(0);
        for p in &self.pieces {
            acc += p.bit_len;
            self.starts.push(acc);
        }
    }

    /// Index of the piece containing document bit `bit` (or `pieces.len()` at the end).
    fn locate(&self, bit: u64) -> usize {
        match self.starts.binary_search(&bit) {
            Ok(i) => i.min(self.pieces.len()),
            Err(i) => i - 1,
        }
    }

    /// Split so that a piece boundary exists at `bit`. Returns the index of the
    /// piece that starts there.
    fn split_at(&mut self, bit: u64) -> usize {
        let i = self.locate(bit);
        if i == self.pieces.len() || self.starts[i] == bit {
            return i;
        }
        let p = self.pieces[i];
        let left_len = bit - self.starts[i];
        self.pieces[i].bit_len = left_len;
        self.pieces.insert(
            i + 1,
            Piece { src: p.src, bit_off: p.bit_off + left_len, bit_len: p.bit_len - left_len },
        );
        self.rebuild_starts();
        i + 1
    }

    pub fn insert(&mut self, at: u64, piece: Piece) {
        assert!(at <= self.len_bits(), "insert past end");
        if piece.bit_len == 0 {
            return;
        }
        let i = self.split_at(at);
        self.pieces.insert(i, piece);
        self.coalesce_around(i);
        self.rebuild_starts();
    }

    pub fn delete(&mut self, at: u64, n: u64) {
        assert!(at + n <= self.len_bits(), "delete past end");
        if n == 0 {
            return;
        }
        // Split the end first so the start split does not shift it.
        self.split_at(at + n);
        let start = self.split_at(at);
        let end = self.locate_boundary(at + n);
        self.pieces.drain(start..end);
        if start > 0 && start < self.pieces.len() {
            self.coalesce_around(start - 1);
        }
        self.rebuild_starts();
    }

    /// Index of the piece starting exactly at `bit` (a boundary must exist there).
    fn locate_boundary(&self, bit: u64) -> usize {
        self.starts.binary_search(&bit).expect("boundary exists")
    }

    /// Merge piece `i` with neighbours when they are contiguous in the same source.
    fn coalesce_around(&mut self, i: usize) {
        fn try_merge(pieces: &mut Vec<Piece>, a: usize) {
            if a + 1 < pieces.len() {
                let (l, r) = (pieces[a], pieces[a + 1]);
                if l.src == r.src && l.bit_off + l.bit_len == r.bit_off {
                    pieces[a].bit_len += r.bit_len;
                    pieces.remove(a + 1);
                }
            }
        }
        try_merge(&mut self.pieces, i);
        if i > 0 {
            try_merge(&mut self.pieces, i - 1);
        }
    }

    /// Read `n` bits starting at document bit `at` into `out` (MSB-first, from bit 0).
    /// `out` must hold at least `bytes_for(n)` bytes. Reports unloaded original chunks.
    pub fn read_bits(
        &self,
        src: &dyn Source,
        add: &AddBuffer,
        at: u64,
        n: u64,
        out: &mut [u8],
    ) -> Vec<Missing> {
        let mut missing = Vec::new();
        if n == 0 || at >= self.len_bits() {
            return missing;
        }
        let end = (at + n).min(self.len_bits());
        let mut cur = at;
        let mut i = self.locate(cur);
        let mut scratch: Vec<u8> = Vec::new();
        while cur < end && i < self.pieces.len() {
            let p = self.pieces[i];
            let in_piece = cur - self.starts[i];
            let take = (p.bit_len - in_piece).min(end - cur);
            let src_bit = p.bit_off + in_piece;
            match p.src {
                Src::Add => copy_bits(add.bytes(), src_bit, out, cur - at, take),
                Src::Orig => {
                    let first_byte = src_bit / 8;
                    let last_byte = (src_bit + take).div_ceil(8);
                    scratch.clear();
                    scratch.resize((last_byte - first_byte) as usize, 0);
                    for m in src.read_bytes(first_byte, &mut scratch) {
                        if !missing.contains(&m) {
                            missing.push(m);
                        }
                    }
                    copy_bits(&scratch, src_bit % 8, out, cur - at, take);
                }
            }
            cur += take;
            i += 1;
        }
        missing
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::MemSource;

    fn read(t: &PieceTable, s: &MemSource, a: &AddBuffer) -> Vec<u8> {
        let mut out = vec![0; bytes_for(t.len_bits())];
        t.read_bits(s, a, 0, t.len_bits(), &mut out);
        out
    }

    #[test]
    fn insert_and_delete_bytes() {
        let s = MemSource(b"hello world".to_vec());
        let mut add = AddBuffer::default();
        let mut t = PieceTable::new(s.len_bytes() * 8);
        let off = add.push_bits(b"big ", 32);
        t.insert(6 * 8, Piece { src: Src::Add, bit_off: off, bit_len: 32 });
        assert_eq!(read(&t, &s, &add), b"hello big world");
        t.delete(0, 6 * 8);
        assert_eq!(read(&t, &s, &add), b"big world");
        assert_eq!(t.piece_count(), 2);
    }

    #[test]
    fn delete_single_bit_shifts_rest() {
        let s = MemSource(vec![0b1000_0000, 0b0000_0001]);
        let add = AddBuffer::default();
        let mut t = PieceTable::new(16);
        t.delete(0, 1);
        assert_eq!(t.len_bits(), 15);
        let got = read(&t, &s, &add);
        assert_eq!(got, [0b0000_0000, 0b0000_0010]);
    }

    #[test]
    fn delete_in_middle_leaves_two_pieces() {
        let s = MemSource((0..=255u8).collect());
        let mut t = PieceTable::new(256 * 8);
        t.delete(8 * 10, 8);
        assert_eq!(t.piece_count(), 2);
        let add = AddBuffer::default();
        let got = read(&t, &s, &add);
        assert_eq!(got.len(), 255);
        assert_eq!(got[9], 9);
        assert_eq!(got[10], 11);
    }
}
