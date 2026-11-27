//! Where an indexed run's compressed bytes are read from, and what a read of
//! them that finds some missing answers.

use std::ops::Range;
use std::sync::Arc;

use crate::codec::{Refusal, Window};
use crate::source::{ArcSource, Missing, Source};

/// Where a run's compressed bytes are read from, for as long as anything
/// reads the run: a shared handle onto the source the run is in, and where
/// the run starts there and how long it is.
///
/// A read that finds chunks of the source not loaded names them all, and
/// reads nothing. Every source a run can be in reads, in the end, from the
/// file's `ChunkStore`, so every chunk named is a chunk of the file. See
/// `docs/DESIGN-lazy-compressed-input.md`, "The index pass reads through a
/// `RunInput`".
///
/// The `Send + Sync` bound keeps a [`Seekable`](super::Seekable) `Send +
/// Sync`.
#[derive(Clone)]
pub struct RunInput {
    source: Arc<dyn Source + Send + Sync>,
    at: u64,
    len: u64,
    /// The copy of the run it reads, for an input made over a copy of its
    /// own rather than over a handle onto a source something else holds.
    copy: Option<Arc<Vec<u8>>>,
}

impl RunInput {
    /// Bytes `at..at + len` of `source`, a handle onto bytes something else
    /// holds: the input owns none of them.
    pub fn new(source: Arc<dyn Source + Send + Sync>, at: u64, len: u64) -> RunInput {
        RunInput { source, at, len, copy: None }
    }

    /// A run whose bytes are held in memory: all of `bytes`, a copy the input
    /// owns, read through an [`ArcSource`] over them, which never finds a
    /// chunk missing.
    pub fn held(bytes: Arc<Vec<u8>>) -> RunInput {
        let len = bytes.len() as u64;
        RunInput { source: Arc::new(ArcSource(bytes.clone())), at: 0, len, copy: Some(bytes) }
    }

    /// How many bytes the run is.
    pub fn len(&self) -> u64 {
        self.len
    }

    /// How many bytes of memory the input holds for itself: the run's length
    /// for a copy ([`RunInput::held`]), and none for a handle onto a source,
    /// whose bytes are the source's. What an index is weighed by while it is
    /// kept.
    pub fn owned_bytes(&self) -> u64 {
        self.copy.as_ref().map_or(0, |c| c.len() as u64)
    }

    /// The copy of the run, for an input made over one.
    pub fn held_copy(&self) -> Option<&Arc<Vec<u8>>> {
        self.copy.as_ref()
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The most bytes the source the run is read from holds at once, when it
    /// holds only part of the file, as the browser's chunk store does. Only
    /// then is a read of the run held to [`Limits::unit_input`] and
    /// [`Limits::span_input`]: a copy held in memory answers any read whole.
    ///
    /// [`Limits::unit_input`]: super::Limits::unit_input
    /// [`Limits::span_input`]: super::Limits::span_input
    pub fn resident_limit(&self) -> Option<u64> {
        self.source.resident_limit()
    }

    /// Whether a read of `bytes` of the run at once is more than `limit`
    /// allows, which it can be only when the source has a resident limit.
    pub(super) fn too_much(&self, bytes: usize, limit: usize) -> bool {
        bytes > limit && self.resident_limit().is_some()
    }

    /// The most bytes of the run one read at once may take: half of what the
    /// source keeps, so that the read fits beside the other reads a go
    /// makes. No limit for a source that answers any read whole.
    pub(super) fn most_at_once(&self) -> usize {
        self.resident_limit().map_or(usize::MAX, |keeps| usize::try_from(keeps / 2).unwrap_or(usize::MAX))
    }

    /// Whether a read of `bytes` of the run at once is within
    /// [`RunInput::most_at_once`].
    pub(super) fn fits(&self, bytes: usize) -> bool {
        bytes <= self.most_at_once()
    }

    /// Bytes `at..at + out.len()` of the run, or every chunk of the source
    /// the read is missing, all at once. On a miss `out` holds zeroes where
    /// the missing chunks are. Bytes past the run's end are not read, and
    /// read as zeroes: a caller never asks for them.
    pub fn read(&self, at: u64, out: &mut [u8]) -> Result<(), Vec<Missing>> {
        debug_assert!(at.saturating_add(out.len() as u64) <= self.len, "read past the end of the run");
        let inside = self.len.saturating_sub(at).min(out.len() as u64) as usize;
        out[inside..].fill(0);
        if inside == 0 {
            return Ok(());
        }
        let missing = self.source.read_bytes(self.at + at, &mut out[..inside]);
        match missing.is_empty() {
            true => Ok(()),
            false => Err(missing),
        }
    }

    /// A copy of bytes `range` of the run, cut short at the run's end, and
    /// the byte it starts at.
    pub(super) fn copy(&self, range: Range<usize>) -> Result<Copied, Vec<Missing>> {
        let end = range.end.min(self.len as usize);
        let start = range.start.min(end);
        let mut bytes = vec![0u8; end - start];
        self.read(start as u64, &mut bytes)?;
        Ok(Copied { bytes, at: start })
    }

    /// Copies of each of `ranges`, or every chunk any of them is missing,
    /// all at once.
    pub(super) fn copies<const N: usize>(&self, ranges: [Option<Range<usize>>; N]) -> Result<[Option<Copied>; N], Vec<Missing>> {
        let mut missing = Vec::new();
        let copies = ranges.map(|range| match self.copy(range?) {
            Ok(copy) => Some(copy),
            Err(m) => {
                missing.extend(m);
                None
            }
        });
        if missing.is_empty() {
            return Ok(copies);
        }
        missing.sort_unstable_by_key(|m| m.chunk);
        missing.dedup();
        Err(missing)
    }
}

/// Bytes of a run copied out through its [`RunInput`], and the byte of the
/// run they start at.
pub(super) struct Copied {
    pub(super) bytes: Vec<u8>,
    pub(super) at: usize,
}

impl Copied {
    /// The copy as a window of the run, which is `run_len` bytes long.
    pub(super) fn window(&self, run_len: u64) -> Window<'_> {
        Window::of(&self.bytes, self.at, run_len as usize).expect("a copy is cut short at the run's end")
    }
}

/// The bytes of a run an index pass holds between its steps: bytes `at..at +
/// bytes.len()`. A step asks for the stretch it decodes from, and reads
/// through the run's [`RunInput`] only the part of it that is not held. What
/// comes before where the step starts is let go of, since the pass never
/// reads it again, except when it goes back to a checkpoint at the cap, and
/// then it reads it again.
#[derive(Default)]
pub(super) struct PassWindow {
    at: usize,
    bytes: Vec<u8>,
}

impl PassWindow {
    /// Hold bytes `from..to` of the run, `to` cut short at the run's end, or
    /// name every chunk the part not held yet is missing, and change
    /// nothing. Bytes held past `to` are kept.
    pub(super) fn reach(&mut self, input: &RunInput, from: usize, to: usize) -> Result<(), Vec<Missing>> {
        let to = to.min(input.len() as usize).max(from);
        let end = self.at + self.bytes.len();
        let kept = from >= self.at && from <= end;
        let read_from = if kept { end.max(from) } else { from };
        let mut more = vec![0u8; to.saturating_sub(read_from)];
        if !more.is_empty() {
            input.read(read_from as u64, &mut more)?;
        }
        if kept {
            self.bytes.drain(..from - self.at);
            self.bytes.extend_from_slice(&more);
        } else {
            self.bytes = more;
        }
        self.at = from;
        Ok(())
    }

    /// The bytes held, as a window of the run, which is `run_len` bytes long.
    pub(super) fn window(&self, run_len: usize) -> Window<'_> {
        Window::of(&self.bytes, self.at, run_len).expect("a window is cut short at the run's end")
    }

    /// The bytes held.
    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// How many bytes are held.
    pub(super) fn len(&self) -> usize {
        self.bytes.len()
    }
}

/// Why a read of an indexed run gave nothing: chunks of the file it needs
/// are not loaded yet, or the decoder refused the bytes.
///
/// The two are kept apart all the way up. A missing chunk is "not yet": the
/// evaluator answers [`EvalError::Pending`](crate::eval::EvalError::Pending)
/// with it, and the caller loads the chunks and asks again. Nothing turns one
/// into a [`Refusal`], which would be kept as the run's answer for good.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    Missing(Vec<Missing>),
    Refused(Refusal),
}

impl From<Refusal> for ReadError {
    fn from(why: Refusal) -> ReadError {
        ReadError::Refused(why)
    }
}

impl From<Vec<Missing>> for ReadError {
    fn from(missing: Vec<Missing>) -> ReadError {
        ReadError::Missing(missing)
    }
}

/// A read that answers nothing when the decoder refuses, as an `Option`
/// does, and passes a missing chunk on.
pub fn refused_as_none<T>(r: Result<T, ReadError>) -> Result<Option<T>, Vec<Missing>> {
    match r {
        Ok(found) => Ok(Some(found)),
        Err(ReadError::Missing(missing)) => Err(missing),
        Err(ReadError::Refused(_)) => Ok(None),
    }
}
