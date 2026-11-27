//! Read-only access to the original file, without holding all of it in memory.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

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
    /// The most bytes this source holds at once, for a source that holds only
    /// some of the file and drops what was read longest ago to make room. A
    /// read longer than this can never find all its bytes there together.
    /// `None` for a source that answers any read whole.
    fn resident_limit(&self) -> Option<u64> {
        None
    }
    /// A handle that reads the same bytes as this source and can be kept
    /// after the borrow ends, by an index that reads a compressed run of
    /// the source long after the reading that made it has moved on. A handle
    /// onto a store reads the chunks fed in after it was made, and reports
    /// the ones pushed out as missing, as the store does.
    ///
    /// `None` for a source that cannot give one; whoever wanted the handle
    /// then copies the bytes it needs. See
    /// `docs/DESIGN-lazy-compressed-input.md`, "Where a `RunInput` comes from".
    fn share(&self) -> Option<Arc<dyn Source + Send + Sync>> {
        None
    }
}

/// A borrowed source is a source. This is what lets one file be read both as
/// itself and as the parts of it something else assembles: see
/// [`Gathered`](crate::gather::Gathered), which holds the source it gathers
/// from and would otherwise have to own it.
impl<S: Source> Source for &S {
    fn len_bytes(&self) -> u64 {
        (*self).len_bytes()
    }
    fn read_bytes(&self, offset: u64, out: &mut [u8]) -> Vec<Missing> {
        (*self).read_bytes(offset, out)
    }
    fn resident_limit(&self) -> Option<u64> {
        (*self).resident_limit()
    }
    fn share(&self) -> Option<Arc<dyn Source + Send + Sync>> {
        (*self).share()
    }
}

/// A source held behind an `Arc` is a source: what a handle from
/// [`Source::share`] is, so that something holding one can be built over it
/// as over any other source.
impl<S: Source + ?Sized> Source for Arc<S> {
    fn len_bytes(&self) -> u64 {
        (**self).len_bytes()
    }
    fn read_bytes(&self, offset: u64, out: &mut [u8]) -> Vec<Missing> {
        (**self).read_bytes(offset, out)
    }
    fn resident_limit(&self) -> Option<u64> {
        (**self).resident_limit()
    }
    fn share(&self) -> Option<Arc<dyn Source + Send + Sync>> {
        (**self).share()
    }
}

/// In-memory source, for tests and small files.
///
/// It gives no handle ([`Source::share`]): its field is public, so it cannot
/// move its bytes behind an `Arc` without changing every place that makes
/// one, and it holds the whole file in memory anyway, so a copy of a run
/// costs what the run costs.
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

/// Bytes somebody else already holds. What a decoded stream is read through:
/// the buffer belongs to the reading that unpacked it, and the document over
/// it borrows rather than copies, which for a 64 MiB stream is the difference
/// between opening a tab and opening two of them.
pub struct ArcSource(pub std::sync::Arc<Vec<u8>>);

impl Source for ArcSource {
    fn len_bytes(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_bytes(&self, offset: u64, out: &mut [u8]) -> Vec<Missing> {
        let o = offset as usize;
        out.copy_from_slice(&self.0[o..o + out.len()]);
        Vec::new()
    }
    /// Another over the same bytes, which copies none of them.
    fn share(&self) -> Option<Arc<dyn Source + Send + Sync>> {
        Some(Arc::new(ArcSource(self.0.clone())))
    }
}

/// One chunk, and when it was last read. Reading is what keeps a chunk alive:
/// the head of a file is loaded first and read constantly, and dropping it
/// because it was loaded first would take the top of the structure away every
/// time someone looked at the end of the file.
struct Cached {
    data: Box<[u8]>,
    used: u64,
}

/// The chunks a store keeps, and the count of reads that says which was read
/// longest ago. Behind a lock, so that a handle onto the store can read them
/// and keep them fresh as the store does.
#[derive(Default)]
struct Chunks {
    map: HashMap<u64, Cached>,
    /// Counts reads, so "longest ago" has something to compare.
    clock: u64,
}

impl Chunks {
    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }
}

/// The chunks of a store fed from outside, shared by the store and every
/// handle onto it ([`Source::share`]). A read takes the lock only while it
/// copies out of the chunks it finds, and calls nothing while it holds it, so
/// no read through a handle can wait on another read.
#[derive(Clone)]
struct Kept {
    len: u64,
    chunk_size: u64,
    capacity: usize,
    chunks: Arc<Mutex<Chunks>>,
}

impl Kept {
    fn lock(&self) -> MutexGuard<'_, Chunks> {
        self.chunks.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Source for Kept {
    fn len_bytes(&self) -> u64 {
        self.len
    }

    fn read_bytes(&self, offset: u64, out: &mut [u8]) -> Vec<Missing> {
        let mut missing = Vec::new();
        let mut pos = 0usize;
        let end = offset + out.len() as u64;
        let mut cur = offset;
        let mut chunks = self.lock();
        while cur < end {
            let chunk = cur / self.chunk_size;
            let in_chunk = (cur % self.chunk_size) as usize;
            let take = ((self.chunk_size - in_chunk as u64).min(end - cur)) as usize;
            let now = chunks.tick();
            match chunks.map.get_mut(&chunk) {
                Some(cached) => {
                    cached.used = now;
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

    fn resident_limit(&self) -> Option<u64> {
        Some(self.capacity as u64 * self.chunk_size)
    }

    fn share(&self) -> Option<Arc<dyn Source + Send + Sync>> {
        Some(Arc::new(self.clone()))
    }
}

/// Fixed-size chunk cache fed from outside (the JS host reads the File/Blob and
/// pushes chunks in). Beyond `capacity`, the chunk read longest ago goes.
///
/// Or, for a store with a source behind it, nothing is fed in: a read goes to
/// that source, and a chunk is missing only when that source says so. What
/// the interface opens an unpacked stream's tab over, so the stream's bytes
/// are not copied into a second buffer, and a stream read a piece at a time
/// is read a piece at a time.
///
/// A store hands out a handle onto its chunks ([`Source::share`]), which an
/// index keeps to read its compressed run from long after the reading that
/// made it has moved on. The handle sees every chunk fed in later, and a
/// chunk pushed out is missing through it as through the store. A store with
/// a source behind it hands out that source's handle.
pub struct ChunkStore {
    kept: Kept,
    behind: Option<Box<dyn Source>>,
}

impl ChunkStore {
    pub fn new(len: u64, chunk_size: u64, capacity: usize) -> Self {
        assert!(chunk_size > 0);
        Self { kept: Kept { len, chunk_size, capacity, chunks: Default::default() }, behind: None }
    }

    /// A store that reads everything from `source`. Nothing is fed in, and a
    /// read reports a chunk missing only when `source` does: a space read
    /// through an index reports the chunks of the file its compressed run is
    /// missing. `chunk_size` is only what a caller counting in chunks counts
    /// in.
    pub fn behind(source: Box<dyn Source>, chunk_size: u64) -> Self {
        let mut store = ChunkStore::new(source.len_bytes(), chunk_size, 0);
        store.behind = Some(source);
        store
    }

    pub fn chunk_size(&self) -> u64 {
        self.kept.chunk_size
    }

    /// Whether a read of `chunk` would find it. Always, for a store with a
    /// source behind it.
    pub fn has(&self, chunk: u64) -> bool {
        self.behind.is_some() || self.kept.lock().map.contains_key(&chunk)
    }

    pub fn insert(&mut self, chunk: u64, data: Box<[u8]>) {
        let mut chunks = self.kept.lock();
        let now = chunks.tick();
        chunks.map.insert(chunk, Cached { data, used: now });
        while chunks.map.len() > self.kept.capacity {
            let Some(oldest) = chunks.map.iter().min_by_key(|(_, c)| c.used).map(|(k, _)| *k) else {
                break;
            };
            chunks.map.remove(&oldest);
        }
    }
}

impl Source for ChunkStore {
    fn len_bytes(&self) -> u64 {
        self.kept.len
    }

    fn read_bytes(&self, offset: u64, out: &mut [u8]) -> Vec<Missing> {
        match &self.behind {
            Some(source) => source.read_bytes(offset, out),
            None => self.kept.read_bytes(offset, out),
        }
    }

    /// Every chunk it keeps. A store with a source behind it keeps nothing,
    /// and says what that source says: a tab over a stream read through an
    /// index keeps as much as the index keeps decoded.
    fn resident_limit(&self) -> Option<u64> {
        match &self.behind {
            Some(source) => source.resident_limit(),
            None => self.kept.resident_limit(),
        }
    }

    fn share(&self) -> Option<Arc<dyn Source + Send + Sync>> {
        match &self.behind {
            Some(source) => source.share(),
            None => self.kept.share(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(s: &dyn Source, at: u64, n: usize) -> (Vec<u8>, Vec<Missing>) {
        let mut out = vec![0xee; n];
        let missing = s.read_bytes(at, &mut out);
        (out, missing)
    }

    fn fed(bytes: &[u8], chunk: u64, capacity: usize, chunks: &[u64]) -> ChunkStore {
        let mut store = ChunkStore::new(bytes.len() as u64, chunk, capacity);
        for &c in chunks {
            let from = (c * chunk) as usize;
            store.insert(c, bytes[from..(from + chunk as usize).min(bytes.len())].into());
        }
        store
    }

    /// A handle onto a store reads its chunks, the ones fed in after the
    /// handle was made as well, and finds missing the ones it pushed out, as
    /// the store does. It says it keeps what the store keeps.
    #[test]
    fn a_handle_onto_a_store_reads_its_chunks_as_they_come_and_go() {
        let bytes: Vec<u8> = (0..40u8).collect();
        let mut store = fed(&bytes, 8, 2, &[0]);
        let handle = store.share().expect("a store hands out a handle");
        assert_eq!(handle.resident_limit(), Some(16));
        assert_eq!(handle.len_bytes(), 40);
        assert_eq!(read(&*handle, 2, 4), (bytes[2..6].to_vec(), vec![]));
        assert_eq!(read(&*handle, 6, 4), (vec![6, 7, 0, 0], vec![Missing { chunk: 1 }]));
        store.insert(1, bytes[8..16].into());
        assert_eq!(read(&*handle, 6, 4), (bytes[6..10].to_vec(), vec![]));
        // A third chunk pushes out the one read longest ago, which is 0.
        store.insert(2, bytes[16..24].into());
        assert_eq!(read(&*handle, 0, 1).1, vec![Missing { chunk: 0 }]);
        assert_eq!(read(&store, 0, 1).1, vec![Missing { chunk: 0 }]);
        assert!(store.has(1) && store.has(2) && !store.has(0));
    }

    /// A read through a handle keeps a chunk fresh in the store, as a read of
    /// the store itself does: the store and its handles keep one clock.
    #[test]
    fn a_read_through_a_handle_keeps_a_chunk_from_being_pushed_out() {
        let bytes: Vec<u8> = (0..40u8).collect();
        let mut store = fed(&bytes, 8, 2, &[0, 1]);
        let handle = store.share().unwrap();
        // Chunk 0 was fed first, and is read now through the handle, so the
        // one read longest ago is 1.
        assert!(read(&*handle, 0, 8).1.is_empty());
        store.insert(2, bytes[16..24].into());
        assert!(store.has(0) && !store.has(1));
    }

    /// Bytes in memory behind an `Arc` give another over the same bytes; a
    /// `MemSource` gives none. A store with a source behind it gives that
    /// source's handle, or none when it has none. A borrowed source gives
    /// what it gives.
    #[test]
    fn which_sources_hand_out_a_handle() {
        let bytes: Vec<u8> = (0..16u8).collect();
        let arc = ArcSource(Arc::new(bytes.clone()));
        let handle = arc.share().expect("an ArcSource hands out a handle");
        assert_eq!(read(&*handle, 3, 5), (bytes[3..8].to_vec(), vec![]));
        assert!(MemSource(bytes.clone()).share().is_none());
        let behind = ChunkStore::behind(Box::new(ArcSource(Arc::new(bytes.clone()))), 4);
        assert_eq!(read(&*behind.share().expect("the handle of the source behind it"), 10, 6), (bytes[10..].to_vec(), vec![]));
        assert!(ChunkStore::behind(Box::new(MemSource(bytes.clone())), 4).share().is_none());
        assert!((&arc).share().is_some() && (&MemSource(bytes)).share().is_none());
    }
}
