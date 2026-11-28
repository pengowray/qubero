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
    /// The chunks a read of `len` bytes at `offset` would find not loaded,
    /// found without copying the bytes: what [`Source::read_bytes`] would
    /// report, deduplicated and ascending. What a reader asks before a long
    /// read, so that a read that would only find chunks missing copies
    /// nothing. A source that keeps chunks marks the ones it finds as read,
    /// as a read of them would, so that asking first keeps the same chunks.
    ///
    /// Every chunk named is one the read would report. A source whose bytes
    /// are decoded as they are read can name fewer, when finding a chunk
    /// would take a decode; so a caller told nothing is missing still checks
    /// what the read reports.
    ///
    /// This default makes the read into a buffer of its own and reports what
    /// it found, which is right for any source and costs the copy. A source
    /// that holds its bytes answers none, and every source that wraps
    /// another asks the one it wraps.
    fn missing(&self, offset: u64, len: u64) -> Vec<Missing> {
        let mut scratch = vec![0u8; len as usize];
        let mut found = self.read_bytes(offset, &mut scratch);
        found.sort_unstable_by_key(|m| m.chunk);
        found.dedup();
        found
    }
    /// Whether a read here can give zeroes for bytes that cannot be made,
    /// with no chunk missing: see [`Source::unreadable`]. False unless the
    /// source unpacks what it gives as it is read, so that a reader asks
    /// [`Source::unreadable`] only of a source that can answer something.
    fn may_be_unreadable(&self) -> bool {
        false
    }
    /// The first stretch of `offset..offset + len` that a read gave zeroes
    /// for because those bytes cannot be made, and why. A joined stream read
    /// a part at a time checks each part when a read first unpacks it, and
    /// a part that will not unpack, or unpacks to another length than the
    /// file says, reads as zeroes: this is what says so. Asked after a read
    /// that found no chunk missing, since the read is what finds a part out.
    /// Nothing for bytes that read, and for every source that holds or
    /// loads its bytes as they are.
    fn unreadable(&self, offset: u64, len: u64) -> Option<Unreadable> {
        let _ = (offset, len);
        None
    }
}

/// Bytes a source gave zeroes for because it could not make them: see
/// [`Source::unreadable`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unreadable {
    /// Where the stretch starts, in the source's own bytes, and how long it
    /// is. Part of it may be outside the read that was asked about.
    pub at: u64,
    pub len: u64,
    /// Why, said after the part's name, as a read of the part in a listing
    /// says it: `blocks[50].compressed unpacks to 2098 bytes, not the 2099
    /// the file says`.
    pub why: String,
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
    fn missing(&self, offset: u64, len: u64) -> Vec<Missing> {
        (*self).missing(offset, len)
    }
    fn may_be_unreadable(&self) -> bool {
        (*self).may_be_unreadable()
    }
    fn unreadable(&self, offset: u64, len: u64) -> Option<Unreadable> {
        (*self).unreadable(offset, len)
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
    fn missing(&self, offset: u64, len: u64) -> Vec<Missing> {
        (**self).missing(offset, len)
    }
    fn may_be_unreadable(&self) -> bool {
        (**self).may_be_unreadable()
    }
    fn unreadable(&self, offset: u64, len: u64) -> Option<Unreadable> {
        (**self).unreadable(offset, len)
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
    /// None: every byte is held.
    fn missing(&self, _: u64, _: u64) -> Vec<Missing> {
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
    /// None: every byte is held.
    fn missing(&self, _: u64, _: u64) -> Vec<Missing> {
        Vec::new()
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

    /// Each chunk the bytes are in that is not kept, found by looking each
    /// one up. A chunk that is kept is marked as read, as `read_bytes` marks
    /// it, so that the chunks of a read that waits for the rest are not the
    /// ones pushed out when the rest are fed.
    fn missing(&self, offset: u64, len: u64) -> Vec<Missing> {
        if len == 0 {
            return Vec::new();
        }
        let (first, last) = (offset / self.chunk_size, (offset + len - 1) / self.chunk_size);
        let mut chunks = self.lock();
        let mut missing = Vec::new();
        for chunk in first..=last {
            let now = chunks.tick();
            match chunks.map.get_mut(&chunk) {
                Some(cached) => cached.used = now,
                None => missing.push(Missing { chunk }),
            }
        }
        missing
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

    fn missing(&self, offset: u64, len: u64) -> Vec<Missing> {
        match &self.behind {
            Some(source) => source.missing(offset, len),
            None => self.kept.missing(offset, len),
        }
    }

    /// What the source behind it says. A store fed from outside loads its
    /// chunks as they are and can always give them once they are in.
    fn may_be_unreadable(&self) -> bool {
        self.behind.as_ref().is_some_and(|source| source.may_be_unreadable())
    }

    fn unreadable(&self, offset: u64, len: u64) -> Option<Unreadable> {
        self.behind.as_ref()?.unreadable(offset, len)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A source of `len` zero bytes in chunks of `chunk`, of which `gone`
    /// are not loaded, that answers [`Source::missing`] itself and counts
    /// the reads made of it: for tests that something asks which chunks are
    /// missing and reads nothing.
    pub(crate) struct Asked {
        pub(crate) len: u64,
        pub(crate) chunk: u64,
        pub(crate) gone: Vec<u64>,
        pub(crate) reads: AtomicU32,
    }

    impl Asked {
        pub(crate) fn new(len: u64, chunk: u64, gone: &[u64]) -> Asked {
            Asked { len, chunk, gone: gone.to_vec(), reads: AtomicU32::new(0) }
        }

        pub(crate) fn reads(&self) -> u32 {
            self.reads.load(Ordering::Relaxed)
        }
    }

    impl Source for Asked {
        fn len_bytes(&self) -> u64 {
            self.len
        }
        fn read_bytes(&self, offset: u64, out: &mut [u8]) -> Vec<Missing> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            out.fill(0);
            self.missing(offset, out.len() as u64)
        }
        fn missing(&self, offset: u64, len: u64) -> Vec<Missing> {
            match len {
                0 => Vec::new(),
                _ => (offset / self.chunk..=(offset + len - 1) / self.chunk).filter(|c| self.gone.contains(c)).map(|chunk| Missing { chunk }).collect(),
            }
        }
    }

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

    /// A store says which chunks a read would find missing, the same list
    /// the read reports, and a handle onto it says the same. Bytes held in
    /// memory are never missing.
    #[test]
    fn a_store_says_which_chunks_a_read_would_find_missing() {
        let bytes: Vec<u8> = (0..40u8).collect();
        let store = fed(&bytes, 8, 4, &[1, 3]);
        let handle = store.share().unwrap();
        for (at, n) in [(0, 40), (8, 8), (9, 1), (6, 4), (15, 10), (39, 1), (0, 0), (20, 3)] {
            let (_, reported) = read(&store, at, n);
            assert_eq!(store.missing(at, n as u64), reported, "{n} bytes at {at}");
            assert_eq!(handle.missing(at, n as u64), reported, "{n} bytes at {at}, through a handle");
        }
        assert_eq!(store.missing(0, 40), vec![Missing { chunk: 0 }, Missing { chunk: 2 }, Missing { chunk: 4 }]);
        assert!(MemSource(bytes.clone()).missing(0, 40).is_empty() && ArcSource(Arc::new(bytes)).missing(0, 40).is_empty());
    }

    /// Asking which chunks are missing marks the ones found as read, as a
    /// read of them does, so that the chunk a read that waits already has is
    /// not the one pushed out when the chunk it waits for arrives.
    #[test]
    fn asking_which_chunks_are_missing_keeps_the_ones_found() {
        let bytes: Vec<u8> = (0..40u8).collect();
        let mut store = fed(&bytes, 8, 3, &[0, 1, 3]);
        // Chunks 0 and 1 were fed before 3, and are asked about now through
        // a handle, with chunk 2, which is not there.
        assert_eq!(store.share().unwrap().missing(0, 24), vec![Missing { chunk: 2 }]);
        store.insert(2, bytes[16..24].into());
        assert!(store.has(0) && store.has(1) && store.has(2) && !store.has(3));
    }

    /// Every source that wraps another asks the one it wraps which chunks
    /// are missing, and reads none of its bytes: a borrow, an `Arc`, a store
    /// with a source behind it, gathered parts and a document, edited or not.
    #[test]
    fn every_wrapper_asks_the_source_it_wraps_and_reads_nothing() {
        use crate::document::Document;
        use crate::gather::{Extent, Gathered};
        let gone = [1, 4];
        let want = vec![Missing { chunk: 1 }, Missing { chunk: 4 }];
        let inner = Asked::new(64, 8, &gone);
        assert_eq!((&inner).missing(0, 64), want);
        assert_eq!(Gathered::new(&inner, [Extent::new(32, 8), Extent::new(8, 8)]).missing(0, 16), want);
        assert_eq!(Gathered::new(&inner, [Extent::new(32, 8), Extent::new(8, 8)]).missing(9, 7), vec![Missing { chunk: 1 }]);
        let mut doc = Document::new(&inner);
        assert_eq!(doc.missing(0, 64), want);
        doc.insert_bytes(0, &[7; 20]);
        assert_eq!(doc.missing(0, 20), vec![], "inserted bytes are there");
        assert_eq!(doc.missing(20, 16), vec![Missing { chunk: 1 }]);
        assert_eq!(inner.reads(), 0, "a wrapper read the bytes");
        let arc = Arc::new(Asked::new(64, 8, &gone));
        assert_eq!(arc.missing(0, 64), want);
        assert_eq!(arc.reads(), 0);
        let behind = ChunkStore::behind(Box::new(arc.clone()), 8);
        assert_eq!(behind.missing(30, 10), vec![Missing { chunk: 4 }]);
        assert_eq!(arc.reads(), 0);
    }
}
