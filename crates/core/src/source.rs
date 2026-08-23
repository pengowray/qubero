//! Read-only access to the original file, without holding all of it in memory.

use std::collections::{HashMap, VecDeque};

/// A byte range of the original that is not currently loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Missing {
    pub chunk: u64,
}

pub trait Source {
    fn len_bytes(&self) -> u64;
    /// Read bytes into `out`. Any chunk that is not loaded is zero-filled and
    /// reported in the returned list (deduplicated, ascending).
    fn read_bytes(&self, offset: u64, out: &mut [u8]) -> Vec<Missing>;
}

/// In-memory source, for tests and small files.
pub struct MemSource(pub Vec<u8>);

impl Source for MemSource {
    fn len_bytes(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_bytes(&self, offset: u64, out: &mut [u8]) -> Vec<Missing> {
        let o = offset as usize;
        out.copy_from_slice(&self.0[o..o + out.len()]);
        Vec::new()
    }
}

/// Fixed-size chunk cache fed from outside (the JS host reads the File/Blob and
/// pushes chunks in). Evicts least recently used chunks beyond `capacity`.
pub struct ChunkStore {
    len: u64,
    chunk_size: u64,
    capacity: usize,
    chunks: HashMap<u64, Box<[u8]>>,
    lru: VecDeque<u64>,
}

impl ChunkStore {
    pub fn new(len: u64, chunk_size: u64, capacity: usize) -> Self {
        assert!(chunk_size > 0);
        Self { len, chunk_size, capacity, chunks: HashMap::new(), lru: VecDeque::new() }
    }

    pub fn chunk_size(&self) -> u64 {
        self.chunk_size
    }

    pub fn has(&self, chunk: u64) -> bool {
        self.chunks.contains_key(&chunk)
    }

    pub fn insert(&mut self, chunk: u64, data: Box<[u8]>) {
        if self.chunks.insert(chunk, data).is_none() {
            self.lru.push_back(chunk);
        }
        while self.chunks.len() > self.capacity {
            if let Some(old) = self.lru.pop_front() {
                self.chunks.remove(&old);
            }
        }
    }
}

impl Source for ChunkStore {
    fn len_bytes(&self) -> u64 {
        self.len
    }

    fn read_bytes(&self, offset: u64, out: &mut [u8]) -> Vec<Missing> {
        let mut missing = Vec::new();
        let mut pos = 0usize;
        let end = offset + out.len() as u64;
        let mut cur = offset;
        while cur < end {
            let chunk = cur / self.chunk_size;
            let in_chunk = (cur % self.chunk_size) as usize;
            let take = ((self.chunk_size - in_chunk as u64).min(end - cur)) as usize;
            match self.chunks.get(&chunk) {
                Some(data) => {
                    let avail = data.len().saturating_sub(in_chunk).min(take);
                    out[pos..pos + avail].copy_from_slice(&data[in_chunk..in_chunk + avail]);
                    out[pos + avail..pos + take].fill(0);
                }
                None => {
                    out[pos..pos + take].fill(0);
                    missing.push(Missing { chunk });
                }
            }
            pos += take;
            cur += take as u64;
        }
        missing
    }
}
