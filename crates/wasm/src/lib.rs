//! wasm-bindgen surface over `qubero-core`.
//!
//! Offsets cross the boundary as `f64` (exact up to 2^53, far past any file size)
//! to avoid BigInt friction on the JS side.

use qubero_core::{ChunkStore, Document};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct Editor {
    doc: Document<ChunkStore>,
}

#[wasm_bindgen]
impl Editor {
    /// `len` is the original file length in bytes. Chunks of `chunk_size` bytes
    /// are pushed in by the host with `feed_chunk`; at most `capacity` are kept.
    #[wasm_bindgen(constructor)]
    pub fn new(len: f64, chunk_size: u32, capacity: u32) -> Editor {
        let store = ChunkStore::new(len as u64, chunk_size as u64, capacity as usize);
        Editor { doc: Document::new(store) }
    }

    pub fn feed_chunk(&mut self, chunk: f64, data: &[u8]) {
        self.doc.source_mut().insert(chunk as u64, data.into());
    }

    pub fn has_chunk(&self, chunk: f64) -> bool {
        self.doc.source().has(chunk as u64)
    }

    pub fn chunk_size(&self) -> u32 {
        self.doc.source().chunk_size() as u32
    }

    pub fn len_bytes(&self) -> f64 {
        self.doc.len_bytes() as f64
    }

    pub fn len_bits(&self) -> f64 {
        self.doc.len_bits() as f64
    }

    /// Fill `out` with document bytes from `at`. Returns the chunk indices that
    /// were not loaded (those bytes are zero). Empty list means the read is complete.
    pub fn read_bytes(&self, at: f64, out: &mut [u8]) -> Vec<f64> {
        self.doc.read_bytes(at as u64, out).into_iter().map(|m| m.chunk as f64).collect()
    }

    pub fn read_bits(&self, at_bit: f64, n: f64, out: &mut [u8]) -> Vec<f64> {
        self.doc.read_bits(at_bit as u64, n as u64, out).into_iter().map(|m| m.chunk as f64).collect()
    }

    pub fn overwrite_bytes(&mut self, at: f64, data: &[u8]) {
        self.doc.overwrite_bytes(at as u64, data);
    }
    pub fn insert_bytes(&mut self, at: f64, data: &[u8]) {
        self.doc.insert_bytes(at as u64, data);
    }
    pub fn delete_bytes(&mut self, at: f64, n: f64) {
        self.doc.delete_bytes(at as u64, n as u64);
    }
    pub fn overwrite_bits(&mut self, at_bit: f64, data: &[u8], n: f64) {
        self.doc.overwrite_bits(at_bit as u64, data, n as u64);
    }
    pub fn insert_bits(&mut self, at_bit: f64, data: &[u8], n: f64) {
        self.doc.insert_bits(at_bit as u64, data, n as u64);
    }
    pub fn delete_bits(&mut self, at_bit: f64, n: f64) {
        self.doc.delete_bits(at_bit as u64, n as u64);
    }

    pub fn undo(&mut self) -> bool {
        self.doc.undo()
    }
    pub fn redo(&mut self) -> bool {
        self.doc.redo()
    }
    pub fn can_undo(&self) -> bool {
        self.doc.can_undo()
    }
    pub fn can_redo(&self) -> bool {
        self.doc.can_redo()
    }
    pub fn is_modified(&self) -> bool {
        self.doc.is_modified()
    }
    pub fn piece_count(&self) -> u32 {
        self.doc.piece_count() as u32
    }
}
