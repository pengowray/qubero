//! Read-only access to the original file, without holding all of it in memory.

use std::cell::Cell;
use std::collections::HashMap;

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

/// One chunk, and when it was last read. Reading is what keeps a chunk alive:
/// the head of a file is loaded first and read constantly, and dropping it
/// because it was loaded first would take the top of the structure away every
/// time someone looked at the end of the file.
struct Cached {
    data: Box<[u8]>,
    used: Cell<u64>,
}

/// Fixed-size chunk cache fed from outside (the JS host reads the File/Blob and
/// pushes chunks in). Beyond `capacity`, the chunk read longest ago goes.
pub struct ChunkStore {
    len: u64,
    chunk_size: u64,
    capacity: usize,
    chunks: HashMap<u64, Cached>,
    /// Counts reads, so "longest ago" has something to compare.
    clock: Cell<u64>,
}

impl ChunkStore {
    pub fn new(len: u64, chunk_size: u64, capacity: usize) -> Self {
        assert!(chunk_size > 0);
        Self { len, chunk_size, capacity, chunks: HashMap::new(), clock: Cell::new(0) }
    }

    pub fn chunk_size(&self) -> u64 {
        self.chunk_size
    }

    pub fn has(&self, chunk: u64) -> bool {
        self.chunks.contains_key(&chunk)
    }

    pub fn insert(&mut self, chunk: u64, data: Box<[u8]>) {
        let now = self.tick();
        self.chunks.insert(chunk, Cached { data, used: Cell::new(now) });
        while self.chunks.len() > self.capacity {
            let Some(oldest) = self.chunks.iter().min_by_key(|(_, c)| c.used.get()).map(|(k, _)| *k) else {
                break;
            };
            self.chunks.remove(&oldest);
        }
    }

    fn tick(&self) -> u64 {
        let now = self.clock.get() + 1;
        self.clock.set(now);
        now
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
                Some(cached) => {
                    cached.used.set(self.tick());
                    let data = &cached.data;
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
