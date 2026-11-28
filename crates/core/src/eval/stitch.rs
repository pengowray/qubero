//! The walk behind a [`Ty::Stitched`], and the reads that join its parts.
//!
//! A stitched stream is one space made of runs that are somewhere else: the
//! blocks of a PDB stream, wherever the writer found room for them, or the
//! members of a BGZF file, each of which unpacks to the next 64 KB of one BAM.
//! Opening one is a walk, the same walk a [`Ty::Gather`] takes to its records
//! (see `gather.rs`, which both of them share), and each place it lands is a
//! part: its place in the joined bytes, the field it is, and where its bytes
//! come from.
//!
//! The walk has to reach every part before anything inside is placed, because
//! what is inside needs to know where the stream ends: a repeat to the end of
//! its room and a `Remaining` both ask. For stored parts that costs the size
//! of each run. For packed parts it costs a length the format writes beside
//! each one (`part_len`), so a BAM of sixteen thousand members is measured
//! without inflating any of them. Only reaching a part is charged, as only
//! reaching a record is for a gather, so a walk that runs out of go carries on
//! from the part it stood on.
//!
//! Reads come after, and only reads unpack anything. A read finds the part its
//! first byte is in by halving the table, copies across as many parts as it
//! covers, and asks each part for its bytes: a stored part from the space its
//! run is in, a packed one from the stream already open over its run if
//! something opened it, and otherwise from a cache that keeps the parts read
//! most recently under a cap and unpacks the rest on demand. A part that will
//! not unpack, or unpacks to another length than its run claimed, fails the
//! read that reached it and names the part; a run of records inside stops
//! there with the reason, the way it stops at any element it cannot read.
//! The reason is kept with the part, so the next read of it fails the same
//! way without unpacking it again.

use std::sync::Arc;

use super::gather::Landing;
use super::space::{Joined, JoinedBytes, JoinedRun, Opened, Part, PartSource, Stitch};
use super::*;
use crate::codec::lazy::{self, Seekable};
use crate::codec::{Codec, StepKind, Trace, TraceBuilder};

/// A stitched stream held whole: its bytes, the trace of every part end to
/// end, and where each part's run is. See [`Evaluator::join_whole`].
pub(super) struct Whole {
    pub(super) bytes: Vec<u8>,
    pub(super) trace: Trace,
    pub(super) runs: Vec<JoinedRun>,
}

/// A stitched stream made ready to open as a document of its own.
pub(super) enum Joining {
    Whole(Whole),
    /// Too large to hold whole, and read a part at a time. Each run's
    /// `in_start` is 0: there is no trace laid end to end for it to be a
    /// place on.
    Lazy { joined: Arc<Joined>, runs: Vec<JoinedRun> },
}

/// Joined streams being made ready to open as documents of their own, kept
/// from one go to the next by the path of the node that opened each. See
/// [`Evaluator::join_whole`].
///
/// Each walk holds what the stream is made of so far, up to the cap, so
/// there are places for two, as for index passes and measuring walks, and a
/// walk that has started is never let go of to make room for another.
#[derive(Default)]
pub(super) struct Joins {
    /// What each place holds, with the stamp it was put there with.
    places: Vec<(Vec<usize>, JoinPlace, u64)>,
    clock: u64,
}

/// What one of the places in [`Joins`] holds.
pub(super) enum JoinPlace {
    Walking(JoinWalk),
    /// A join that a call waiting for a place carried to its end, waiting
    /// for its own call to take it.
    Done(Result<Joining, Refusal>),
}

/// How many places there are for joins under way.
const JOIN_WALKS: usize = 2;

impl Joins {
    pub(super) fn take(&mut self, path: &[usize]) -> Option<JoinPlace> {
        let i = self.places.iter().position(|p| p.0 == path)?;
        Some(self.places.swap_remove(i).1)
    }

    /// Put a walk back, or a finished join for its call to take. A place has
    /// to be free, as in `Unfinished::put`.
    pub(super) fn put(&mut self, path: &[usize], held: JoinPlace) {
        debug_assert!(self.has_room(), "no free place to put a join in");
        self.clock += 1;
        self.places.push((path.to_vec(), held, self.clock));
    }

    pub(super) fn has_room(&self) -> bool {
        self.places.len() < JOIN_WALKS
    }

    /// The walk with the fewest bits of runs left to read, taken out to be
    /// carried on by a call waiting for a place.
    fn take_nearest_its_end(&mut self) -> Option<(Vec<usize>, JoinWalk)> {
        let left = |p: &JoinPlace| match p {
            JoinPlace::Walking(w) => Some(w.run_bits.saturating_sub(w.read_bits)),
            JoinPlace::Done(_) => None,
        };
        let i = (0..self.places.len()).filter(|&i| left(&self.places[i].1).is_some()).min_by_key(|&i| left(&self.places[i].1))?;
        let (path, held, _) = self.places.swap_remove(i);
        let JoinPlace::Walking(walk) = held else { unreachable!("chosen for being a walk") };
        Some((path, walk))
    }

    /// Let go of the finished join put in its place longest ago.
    fn drop_oldest_done(&mut self) {
        let oldest = (0..self.places.len()).filter(|&i| matches!(self.places[i].1, JoinPlace::Done(_))).min_by_key(|&i| self.places[i].2);
        let i = oldest.expect("a place holding a finished join");
        self.places.swap_remove(i);
    }

    pub(super) fn is_empty(&self) -> bool {
        self.places.is_empty()
    }
}

/// How far the join of one stitched stream has got. See
/// [`Evaluator::join_whole`].
pub(super) struct JoinWalk {
    /// The next part to read.
    next: usize,
    /// How many bits of the parts' runs have been read, and how many there
    /// are, for saying how far the walk has got.
    read_bits: u64,
    run_bits: u64,
    runs: Vec<JoinedRun>,
    made: Made,
}

/// What a join has made of the parts read so far.
enum Made {
    /// The bytes and trace of a stream held whole, and where on the trace's
    /// own axis the next part's run goes.
    Whole { bytes: Vec<u8>, trace: TraceBuilder, axis: u64 },
    /// What each part of a stream read a part at a time is read from.
    Lazy { parts: Vec<(std::ops::Range<u64>, JoinedBytes)> },
}

impl JoinWalk {
    fn new(stitch: &Stitch, lazy: bool) -> JoinWalk {
        let made = match lazy {
            true => Made::Lazy { parts: Vec::with_capacity(stitch.parts.len()) },
            false => Made::Whole { bytes: Vec::with_capacity(stitch.len_bytes as usize), trace: TraceBuilder::default(), axis: 0 },
        };
        let run_bits = stitch.parts.iter().map(|p| part_run(p).2).sum();
        JoinWalk { next: 0, read_bits: 0, run_bits, runs: Vec::with_capacity(stitch.parts.len()), made }
    }

    /// How far the walk has got, as an index pass says it: the bits of the
    /// parts' runs read, of all of them, and the bytes the parts read come
    /// to. Named by where the first part's run is, since a joined stream has
    /// no one run.
    fn progress(&self, stitch: &Stitch) -> super::Unpacking {
        let (run_space, run_offset_bits) = stitch.parts.first().map_or((0, 0), |p| (part_run(p).0, part_run(p).1));
        let unpacked_bytes = stitch.parts.get(self.next).map_or(stitch.len_bytes, |p| p.start);
        super::Unpacking { run_space, run_offset_bits, read_bits: self.read_bits, run_bits: self.run_bits, unpacked_bytes }
    }
}

/// What the walk charges for a part of `len` bytes copied, as a gather is
/// charged.
fn copy_cost(len: u64) -> u64 {
    1 + len / super::space::GATHER_BYTES_PER_ELEMENT
}

/// What the walk charges for a part that unpacks to `len` bytes, as an
/// index step is charged.
fn unpack_cost(len: u64) -> u64 {
    1 + len / (super::space::INDEX_STEP_BYTES / super::space::INDEX_STEP_ELEMENTS)
}

/// Where part `p`'s run is: the space, the bit it starts at there, and how
/// many bits it is.
fn part_run(p: &Part) -> (u32, u64, u64) {
    match &p.source {
        PartSource::Stored { space, at_bits } => (*space, *at_bits, p.len * 8),
        PartSource::Packed { space, at_bits, size_bits, .. } => (*space, *at_bits, *size_bits),
    }
}

/// Why a packed part could not be read, through an index or decoded whole.
/// Kept in [`Stitch::troubles`] once found, and by a joined stream read a
/// part at a time in `Joined::troubles`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PartTrouble {
    /// It unpacks to this many bytes, which is not what the file says.
    Length(u64),
    /// Its decoder refused it, for this reason. From an index pass,
    /// [`Refusal::TooLargeLazy`] is a part that unpacks to more than
    /// [`lazy::LAZY_CAP_BYTES`], [`Refusal::TooLargeUnit`] one with a zstd
    /// frame or xz block that unpacks to more than 64 MiB, and
    /// [`Refusal::Failed`] one that will not decode.
    Refused(Refusal),
}

impl PartTrouble {
    /// The refusal a read of the part fails with: the decoder's own, or
    /// failed for a part of the wrong length.
    pub(super) fn refusal(self) -> Refusal {
        match self {
            PartTrouble::Length(_) => Refusal::Failed,
            PartTrouble::Refused(why) => why,
        }
    }
}

/// What a read of a packed part named `label` that the file says gives
/// `claimed` bytes says when it fails for `trouble`, in the file's own
/// voice: `blocks[2].compressed unpacks to 20 bytes, not the 25 the file
/// says`, or `blocks[0].compressed could not be unpacked`. The listing says
/// it over the bytes a run of records could not read, and a joined stream's
/// tab over the bytes of the part.
#[cold]
#[inline(never)]
pub(super) fn trouble_said(label: &str, trouble: PartTrouble, claimed: u64) -> String {
    match trouble {
        PartTrouble::Length(n) => format!("{label} unpacks to {n} bytes, not the {claimed} the file says"),
        PartTrouble::Refused(why) => format!("{label} {}", part_refused(why)),
    }
}

/// Part `i` named by its index, marked as one so that `2` is not read as
/// the second, for where the field it is cannot be said.
#[cold]
#[inline(never)]
pub(super) fn unnamed_part(i: usize) -> String {
    format!("part index {i}")
}

/// The bytes `out` of a joined stream, which part `label` gives, read as
/// zeroes for `trouble`: see [`Source::unreadable`].
pub(super) fn unreadable_part(label: &str, out: std::ops::Range<u64>, trouble: PartTrouble) -> crate::source::Unreadable {
    let len = out.end - out.start;
    crate::source::Unreadable { at: out.start, len, why: trouble_said(label, trouble, len) }
}

/// What a read of a packed part says after the part's name when the part's
/// decoder refused it. A part too large, for a limit or for the file's chunk
/// store, says what [`Refusal::after_name`] says: the words a compressed
/// run's own row has (`DECODED_REFUSED` in the web's `strings.ts`) after
/// "is", or "has" and the frame that was too large. Any other refusal says
/// "could not be unpacked": the other messages, such as "unpacking failed",
/// do not read as a sentence after a field's name.
fn part_refused(why: Refusal) -> String {
    why.after_name().unwrap_or_else(|| "could not be unpacked".into())
}

/// What the walk to a stitched stream's parts has found so far.
#[derive(Debug, Default, Clone)]
pub(super) struct StitchWalk {
    pub(super) walk: Walk,
    pub(super) parts: Vec<Part>,
    /// Where the next part starts in the joined bytes.
    end: u64,
}

/// Where a stitched stream's first part was measured. See
/// [`Evaluator::first_part_frame`].
pub(super) struct PartFrame {
    pub(super) holder: Vec<usize>,
    pub(super) end: Vec<usize>,
    pub(super) here: Option<(u64, u64)>,
}

/// What came of reaching one run.
enum Reached {
    Part(Part),
    /// The run cannot be measured, so nothing after it can be placed either:
    /// the stream ends where this part would have begun.
    End,
    /// The run is not something any part can be, and the whole stream is
    /// refused for it.
    Refused(Refusal),
}

impl Evaluator {
    /// Open the stitched stream at `path`, or say why it would not open. The
    /// walk to its parts is done once and kept, and carries on across goes:
    /// see the module notes.
    pub(super) fn open_stitched_at<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Opened> {
        if let Some(known) = self.spaces.get(path) {
            return Ok(known);
        }
        self.resolve(doc, path)?;
        let Ty::Stitched { from, part_len, len, .. } = self.memo[path].ty.clone() else {
            return fail("not a stitched stream");
        };
        loop {
            let Some(landing) = self.walk_next(doc, path, &from, Landing::Runs)? else { break };
            let mut landing = landing;
            self.through_at(doc, &mut landing)?;
            // Charged for reaching a part and before anything is read from it,
            // so a go that runs out here stands on this part next time.
            let reached = self.memo.get(&landing).map_or(0, |r| r.offset);
            self.spend(reached)?;
            let start = self.list(path).stitch.as_deref().map_or(0, |s| s.end);
            match self.reach_part(doc, &landing, part_len.as_ref(), start)? {
                Reached::Part(part) => {
                    let s = self.list_mut(path).stitch.get_or_insert_with(Default::default);
                    s.end = part.start + part.len;
                    s.parts.push(part);
                }
                Reached::End => break,
                Reached::Refused(why) => {
                    self.list_mut(path).stitch = None;
                    self.spaces.refuse(path, why);
                    return Ok(Opened::Refused(why));
                }
            }
            self.walk_past(path, Landing::Runs);
        }
        // The total the format states, where it states one. Worked out where
        // the node is declared, like any other length.
        let cut = match &len {
            None => None,
            Some(e) => match self.eval_expr(doc, path, e) {
                Ok(n) if n >= 0 => Some(n as u64),
                Err(e) if e.interrupted() => return Err(e),
                // A length that will not read, or reads as less than nothing,
                // leaves nothing to say where the stream ends.
                _ => {
                    self.list_mut(path).stitch = None;
                    self.spaces.refuse(path, Refusal::Settings);
                    return Ok(Opened::Refused(Refusal::Settings));
                }
            },
        };
        let walked = self.list_mut(path).stitch.take().map(|s| s.parts).unwrap_or_default();
        let (parts, total) = cut_parts(walked, cut);
        Ok(Opened::Space(self.spaces.add_stitched(path, parts, total)))
    }

    /// One run the walk landed on, as a part starting at `start` of the joined
    /// bytes.
    fn reach_part<S: Source>(&mut self, doc: &Document<S>, landing: &[usize], part_len: Option<&Expr>, start: u64) -> R<Reached> {
        self.resolve(doc, landing)?;
        let size = match self.size_of(doc, landing) {
            Ok(size) => size,
            Err(e) if e.interrupted() => return Err(e),
            // A run that does not read, which is what a page past the end of a
            // cut-off file is. What came before it is still the stream.
            Err(_) => return Ok(Reached::End),
        };
        let r = self.memo[landing].clone();
        // No part is half a byte, whatever holds it, and a stream with one
        // in it has nowhere to put the bits either side.
        if r.offset % 8 != 0 || size % 8 != 0 {
            return Ok(Reached::Refused(Refusal::Unaligned));
        }
        let claimed = match part_len {
            None => None,
            Some(e) => self.claimed_len(doc, landing, e)?,
        };
        let path = landing.to_vec();
        let codec = match matches!(r.ty, Ty::Decoded { .. }) {
            true => Some(match self.codec_at(doc, landing)? {
                Some(codec) => Ok(codec),
                None => Err(Refusal::Settings),
            }),
            false => None,
        };
        // A run the format stored as it is, which a ZIP, a RAR and an LHA
        // archive declare as a stream whose codec copies, is read where it
        // sits like a run that is no stream at all. Copying it into the cache
        // first would hold a large stored file twice, and refuse one past the
        // cap a stream may unpack to.
        if matches!(codec, None | Some(Ok(Codec::Stored))) {
            // As long as the run, or as long as the format says the part is
            // where that is less: a claim of more than is there cannot be
            // read, and the bytes that are there still can.
            let len = claimed.map_or(size / 8, |n| n.min(size / 8));
            let source = PartSource::Stored { space: r.space, at_bits: r.offset };
            return Ok(Reached::Part(Part { start, len, path, source }));
        }
        let codec = codec.expect("a stream's codec was asked for above");
        let len = match (claimed, codec) {
            (Some(n), _) => n,
            // Nothing says how long it comes to, so it is unpacked to find
            // out. Not kept: a read that wants it unpacks it again, which
            // costs one more inflate of one part and keeps the walk from
            // holding every part it measured.
            (None, Ok(codec)) => match self.measure_run(doc, r.space, r.offset, size, codec)? {
                Ok(len) => len,
                Err(_) => return Ok(Reached::End),
            },
            (None, Err(_)) => return Ok(Reached::End),
        };
        let source = PartSource::Packed { space: r.space, at_bits: r.offset, size_bits: size, codec };
        Ok(Reached::Part(Part { start, len, path, source }))
    }

    /// How long the format says the part at `landing` is, worked out one past
    /// the last field of the nearest structure the run is inside: a BGZF
    /// member's `original_size` is a field after its `compressed` run, and
    /// this is where that name reaches. Nothing when it will not read.
    fn claimed_len<S: Source>(&mut self, doc: &Document<S>, landing: &[usize], e: &Expr) -> R<Option<u64>> {
        let Some(holder) = self.part_holder(landing) else { return Ok(None) };
        let (end, here) = self.record_frame(doc, &holder)?;
        match self.eval_expr_at(doc, &end, e, here) {
            Ok(n) if n >= 0 => Ok(Some(n as u64)),
            Ok(_) => Ok(None),
            Err(e) if e.interrupted() => Err(e),
            Err(_) => Ok(None),
        }
    }

    /// The structure a part's claimed length is worked out in: the nearest one
    /// above the run the walk landed on, from what the memo holds.
    fn part_holder(&self, landing: &[usize]) -> Option<Vec<usize>> {
        (0..landing.len())
            .rev()
            .find(|&k| matches!(self.memo.get(&landing[..k]).map(|r| r.ty.base()), Some(Ty::Struct(_))))
            .map(|k| landing[..k].to_vec())
    }

    /// Where the stitched stream at `path` worked out what its first part
    /// comes to, for saying what measured its parts: the structure that holds
    /// the part's run, and the frame one past that structure's last field. The first part stands for the rest, which are measured the same
    /// way in their own structures. Nothing when the stream did not open or
    /// has no parts.
    pub(super) fn first_part_frame<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Option<PartFrame>> {
        let Opened::Space(id) = self.open_stitched_at(doc, path)? else { return Ok(None) };
        let Some(part) = self.spaces.stitch(id).and_then(|s| s.parts.first()).map(|p| p.path.clone()) else {
            return Ok(None);
        };
        for k in 0..=part.len() {
            self.resolve(doc, &part[..k])?;
        }
        let Some(holder) = self.part_holder(&part) else { return Ok(None) };
        let (end, here) = self.record_frame(doc, &holder)?;
        Ok(Some(PartFrame { holder, end, here }))
    }

    /// How many bytes a packed run comes to. A run that can be indexed and
    /// comes to more than can be held whole is indexed, and the index kept,
    /// so that a read of it later does not decode it again to find a place.
    fn measure_run<S: Source>(&self, doc: &Document<S>, space: u32, at: u64, size: u64, codec: Codec) -> R<Result<u64, Refusal>> {
        if !lazy::can_index(codec) {
            return Ok(self.unpack_run(doc, space, at, size, codec)?.map(|bytes| bytes.len() as u64));
        }
        let key = (space, at, size);
        if let Some(index) = self.spaces.indexed(key, codec) {
            return Ok(Ok(index.len()));
        }
        Ok(match self.index_run(doc, key, codec, false)? {
            Ok(lazy::Decoded::Whole(bytes, _)) => Ok(bytes.len() as u64),
            Ok(lazy::Decoded::Indexed(index)) => {
                let len = index.len();
                self.spaces.keep_index(key, codec, Arc::new(index));
                Ok(len)
            }
            Err(why) => Err(why),
        })
    }

    /// The index of packed part `i`, for a part too large to decode whole.
    /// `Ok(None)` for a part that is decoded whole, which is every part the
    /// file says comes to no more than the cap, and every part of a codec
    /// that has no index.
    ///
    /// The index is the one already made for the part's run if there is one:
    /// by the part's own `Decoded` node, by the walk that measured it, or by
    /// an earlier read. One made here is kept the same way.
    ///
    /// A part that fails here is kept in [`Stitch::troubles`] with its
    /// reason, and later calls return that reason without decoding. A
    /// refusal is also kept by the run, in `refused_runs`. A part the file
    /// says is past the cap that decodes whole is not a refusal of the run,
    /// so only `troubles` keeps it: without that, every read would decode
    /// the run again, up to 64 MiB, to find the same length.
    fn index_part<S: Source>(&self, doc: &Document<S>, stitch: &Stitch, i: usize) -> R<Result<Option<Arc<Seekable>>, PartTrouble>> {
        let part = &stitch.parts[i];
        let PartSource::Packed { space, at_bits, size_bits, codec: Ok(codec) } = &part.source else { return Ok(Ok(None)) };
        let limits = self.spaces.lazy_limits();
        if !lazy::can_index(*codec) || part.len <= limits.whole as u64 {
            return Ok(Ok(None));
        }
        let known = stitch.troubles.borrow().get(&i).copied();
        if let Some(trouble) = known {
            return Ok(Err(trouble));
        }
        let key = (*space, *at_bits, *size_bits);
        let open = match self.spaces.get(&part.path) {
            Some(Opened::Space(id)) => self.spaces.lazy(id).cloned(),
            _ => None,
        };
        let found = match open.or_else(|| self.spaces.indexed(key, *codec)) {
            Some(index) => Ok(index),
            // Only a part the file says is past the cap gets here.
            None => match self.index_run(doc, key, *codec, true)? {
                Ok(lazy::Decoded::Indexed(index)) => {
                    let index = Arc::new(index);
                    self.spaces.keep_index(key, *codec, index.clone());
                    Ok(index)
                }
                // Small enough to hold whole after all, which a part the
                // file says is larger than the cap is not.
                Ok(lazy::Decoded::Whole(bytes, _)) => Err(PartTrouble::Length(bytes.len() as u64)),
                Err(why) => Err(PartTrouble::Refused(why)),
            },
        };
        let found = found.and_then(|index| if index.len() == part.len { Ok(index) } else { Err(PartTrouble::Length(index.len())) });
        if let Err(trouble) = found {
            stitch.troubles.borrow_mut().insert(i, trouble);
        }
        Ok(found.map(Some))
    }

    /// [`Self::index_part`], with its trouble said the way a read of the
    /// part says it.
    fn part_index<S: Source>(&self, doc: &Document<S>, stitch: &Stitch, i: usize) -> R<Option<Arc<Seekable>>> {
        match self.index_part(doc, stitch, i)? {
            Ok(index) => Ok(index),
            Err(trouble) => self.part_trouble(stitch, i, trouble),
        }
    }

    /// A read of packed part `i` failing for `trouble`. It is shown over the
    /// bytes a run of records could not read, so it names the part the way
    /// the listing names the field it is, and says what the file claimed in
    /// the file's own voice: `blocks[2].compressed unpacks to 20 bytes, not
    /// the 25 the file says`. Out of line for the same reason as
    /// [`Self::part_refusal`].
    #[cold]
    #[inline(never)]
    fn part_trouble<T>(&self, stitch: &Stitch, i: usize, trouble: PartTrouble) -> R<T> {
        fail(trouble_said(&self.part_label(stitch, i), trouble, stitch.parts[i].len))
    }

    /// A read of packed part `i` failing because its decoder refused it, said
    /// after the part's name: `blocks[0].compressed is too large to unpack
    /// (over 1 GiB unpacked)`. Out of line, so that the `format!` adds
    /// nothing to the stack frame of `read_stitched`, which a read deep
    /// inside an evaluation can reach.
    #[cold]
    #[inline(never)]
    fn part_refusal<T>(&self, stitch: &Stitch, i: usize, why: Refusal) -> R<T> {
        self.part_trouble(stitch, i, PartTrouble::Refused(why))
    }

    /// The bytes of a packed run, unpacked. The outer error is the run's bytes
    /// not being there yet; the inner one is the decoder refusing them. A
    /// long run is read a window at a time across goes ([`Self::gather_run`])
    /// and decoded in one go.
    fn unpack_run<S: Source>(&self, doc: &Document<S>, space: u32, at: u64, size: u64, codec: Codec) -> R<Result<Vec<u8>, Refusal>> {
        if size / 8 > crate::codec::CAP_BYTES as u64 {
            return Ok(Err(Refusal::TooLargeCopy));
        }
        let packed = match self.gather_run(doc, space, at, size)? {
            Ok(packed) => packed,
            Err(why) => return Ok(Err(why)),
        };
        #[cfg(test)]
        self.spaces.decodes.set(self.spaces.decodes.get() + 1);
        Ok(crate::codec::decode(codec, &packed))
    }

    /// Where the one child of the stitched stream at `parent` goes: the front
    /// of the space its parts make, with that space's end for its room.
    pub(super) fn place_stitched<S: Source>(
        &mut self,
        doc: &Document<S>,
        parent: &[usize],
        pr: &Resolved,
        idx: usize,
        inner: &Ty,
    ) -> R<Option<Place>> {
        if idx != 0 {
            return fail("no such field");
        }
        let space = match self.open_stitched_at(doc, parent)? {
            Opened::Space(id) => id,
            // `child_count` already said there is nothing inside, so nothing
            // should be asking.
            Opened::Refused(_) => return fail("this stream did not open"),
        };
        let limit = self.spaces.len_bits(space);
        Ok(Some(Place { name: pr.name.clone(), ty: inner.clone(), offset: 0, limit, space, machinery: false, elsewhere: false, aside: false, trace_kind: None }))
    }

    /// `n` bits of a stitched space from bit `at`, from as many parts as
    /// they cover.
    pub(super) fn read_stitched<S: Source>(&self, doc: &Document<S>, stitch: &Stitch, at: u64, n: u64) -> R<Vec<u8>> {
        if at + n > stitch.len_bytes * 8 {
            return fail("field extends beyond the end of the joined stream");
        }
        let mut buf = vec![0u8; crate::bits::bytes_for(n)];
        if n == 0 {
            return Ok(buf);
        }
        let (first, last) = (at / 8, (at + n).div_ceil(8));
        let mut bytes = Vec::with_capacity((last - first) as usize);
        let mut pos = first;
        while pos < last {
            let Some(i) = stitch.part_at(pos) else { return fail("field extends beyond the end of the joined stream") };
            let part = &stitch.parts[i];
            let take = (part.start + part.len).min(last) - pos;
            let from = pos - part.start;
            match &part.source {
                PartSource::Stored { space, at_bits } => {
                    bytes.extend_from_slice(&self.read_in(doc, *space, at_bits + from * 8, take * 8)?);
                }
                PartSource::Packed { .. } => match self.part_index(doc, stitch, i)? {
                    Some(index) => bytes.extend_from_slice(&self.read_part_index(stitch, i, &index, from, take)?),
                    None => {
                        let unpacked = self.unpacked_part(doc, stitch, i)?;
                        bytes.extend_from_slice(&unpacked[from as usize..(from + take) as usize]);
                    }
                },
            }
            pos += take;
        }
        crate::bits::copy_bits(&bytes, at % 8, &mut buf, 0, n);
        Ok(buf)
    }

    /// Bytes `from..from + take` of packed part `i`, read through its index.
    /// A chunk of the file the part's run is missing is `Pending`, unless the
    /// read is longer than the index keeps decoded at once and can never
    /// fit, which fails as such a read of the file does. Only the decoder
    /// refusing the run is the part's failure. Out of line for the same
    /// reason as [`Self::part_refusal`].
    #[cold]
    #[inline(never)]
    fn read_part_index(&self, stitch: &Stitch, i: usize, index: &Seekable, from: u64, take: u64) -> R<Vec<u8>> {
        let mut got = vec![0u8; take as usize];
        match index.read(from, &mut got) {
            Ok(()) => Ok(got),
            Err(lazy::ReadError::Missing(missing)) => super::read::pending_unless_too_much(index, take, missing),
            Err(lazy::ReadError::Refused(why)) => self.part_refusal(stitch, i, why),
        }
    }

    /// Everything packed part `i` unpacks to: from the stream already open over
    /// its run when something opened that, from the cache when it was read
    /// lately, and unpacked now otherwise.
    ///
    /// A part that will not decode, or decodes to another length than the
    /// file says, is kept in [`Stitch::troubles`], so a read that reaches it
    /// again fails the same way without decoding it again. The cache keeps
    /// only parts that decoded to the length the file says.
    fn unpacked_part<S: Source>(&self, doc: &Document<S>, stitch: &Stitch, i: usize) -> R<Arc<Vec<u8>>> {
        let part = &stitch.parts[i];
        let PartSource::Packed { space, at_bits, size_bits, codec } = &part.source else {
            return fail("not a packed part");
        };
        if let Some(Opened::Space(id)) = self.spaces.get(&part.path) {
            if let Some(bytes) = self.spaces.buf(id) {
                if bytes.len() as u64 != part.len {
                    return self.part_trouble(stitch, i, PartTrouble::Length(bytes.len() as u64));
                }
                return Ok(bytes.clone());
            }
        }
        if let Some(bytes) = stitch.cache.borrow_mut().get(i) {
            return Ok(bytes);
        }
        let known = stitch.troubles.borrow().get(&i).copied();
        if let Some(trouble) = known {
            return self.part_trouble(stitch, i, trouble);
        }
        // A run longer than a window, of a codec that can be indexed, goes
        // through its index pass, which reads it a window at a time and
        // decodes it a step at a time, each charged to the go, and which is
        // shared with the part's own `Decoded` node. A shorter run is read
        // and decoded in one go: a BGZF member is under 64 KiB, and a pass
        // for it would wait for a place behind passes over other runs.
        let stepped = match codec {
            Ok(codec) => lazy::can_index(*codec) && self.file_bytes_at_once(*space, *at_bits, *size_bits) > self.gather_window(doc),
            Err(_) => false,
        };
        let unpacked = match codec {
            Ok(codec) if stepped => match self.index_run(doc, (*space, *at_bits, *size_bits), *codec, false)? {
                Ok(lazy::Decoded::Whole(bytes, _)) => Ok(bytes),
                // More than the cap, which the file says the part is not.
                Ok(lazy::Decoded::Indexed(index)) => {
                    let trouble = PartTrouble::Length(index.len());
                    stitch.troubles.borrow_mut().insert(i, trouble);
                    return self.part_trouble(stitch, i, trouble);
                }
                Err(why) => Err(why),
            },
            Ok(codec) => self.unpack_run(doc, *space, *at_bits, *size_bits, *codec)?,
            Err(why) => Err(*why),
        };
        let found = match unpacked {
            Ok(bytes) if bytes.len() as u64 == part.len => Ok(Arc::new(bytes)),
            Ok(bytes) => Err(PartTrouble::Length(bytes.len() as u64)),
            Err(why) => Err(PartTrouble::Refused(why)),
        };
        match found {
            Ok(bytes) => {
                stitch.cache.borrow_mut().put(i, bytes.clone());
                Ok(bytes)
            }
            Err(trouble) => {
                stitch.troubles.borrow_mut().insert(i, trouble);
                self.part_trouble(stitch, i, trouble)
            }
        }
    }

    /// The stitched stream at `path` held whole, for opening it as a document
    /// of its own: its bytes, a trace of every part, and where each part's run
    /// is. Past [`Spaces::whole_cap`](super::space::Spaces::whole_cap) it is
    /// read a part at a time instead: see [`Self::join_part_lazily`].
    /// Refused when the stream would not open, when it comes to more than a
    /// decoded stream may (the same cap, and the same refusal), and when a
    /// part will not read.
    ///
    /// The parts are read one after another by a walk kept in
    /// [`Spaces::joins`](super::space::Spaces) from one go to the next, keyed
    /// by `path`. Each part is charged to the go before it is read, a go that
    /// runs out hands back `Busy`, and a part whose bytes have not arrived
    /// hands back `Pending`: either way the walk keeps the parts read so far,
    /// and the next call carries on from the part it stopped at. A walk that
    /// has started is never let go of to make room for another: one that
    /// would start while both places are taken waits, carrying on the others
    /// (see [`Self::wait_for_a_join_place`]).
    ///
    /// The trace is each part's laid end to end. A stored part is one step
    /// over its own bytes; a packed part is the trace its decoder kept; and
    /// each part is one member, the piece of the stream its container made as
    /// a unit. The bits are counted along an axis of the trace's own, each
    /// part's run after the one before, since the runs themselves are anywhere
    /// and in any order and a trace's steps have to go forward.
    /// [`JoinedRun`] is what takes a step back to the run it was read from.
    ///
    /// Every part is read and every packed one unpacked here, again, with its
    /// trace: the cache a listing reads through keeps bytes and not traces,
    /// and a trace is what a document of its own is for.
    pub(super) fn join_whole<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Result<Joining, Refusal>> {
        let id = match self.open_stitched_at(doc, path)? {
            Opened::Space(id) => id,
            Opened::Refused(why) => return Ok(Err(why)),
        };
        let taken = self.spaces.joins.borrow_mut().take(path);
        let walk = match taken {
            Some(JoinPlace::Done(found)) => return Ok(found),
            Some(JoinPlace::Walking(walk)) => walk,
            None => {
                let Some(stitch) = self.spaces.stitch(id) else { return fail("this stream is no longer open") };
                let lazy = stitch.len_bytes > self.spaces.whole_cap() as u64;
                if let Some(why) = self.too_large_to_join(doc, stitch, lazy) {
                    return Ok(Err(why));
                }
                // A walk that is never put back needs no place, as with an
                // index pass: one in a go with no allowance, over a source
                // that answers any read whole.
                if self.go.limited() || doc.resident_limit().is_some() {
                    self.go.charge(0)?;
                    self.wait_for_a_join_place(doc)?;
                }
                #[cfg(test)]
                self.spaces.walks.set(self.spaces.walks.get() + 1);
                JoinWalk::new(stitch, lazy)
            }
        };
        self.join_steps(doc, path, id, walk, true)
    }

    /// Why the stitched stream would be refused as too large, said before
    /// anything is read, since it does not depend on what the parts hold.
    ///
    /// Held whole, a trace keeps each step's start in 32 bits, which is half
    /// a gigabyte of runs laid end to end, and no packed run past the cap is
    /// decoded. Read a part at a time, the stream holds a copy only of what
    /// no handle reaches, and those copies are held to the same cap between
    /// them as the stream would have had to fit under whole: a stored part's
    /// bytes and a packed part's run that [`Self::joined_input`] gives no
    /// handle for, and the run of a packed part read through its index when
    /// no handle reaches the run, since its index then holds a copy. Which
    /// parts are read through an index is decided as [`Self::index_part`]
    /// decides it.
    fn too_large_to_join<S: Source>(&self, doc: &Document<S>, stitch: &Stitch, lazy: bool) -> Option<Refusal> {
        let run_bits = |p: &Part| part_run(p).2;
        let limits = self.spaces.lazy_limits();
        let copied = |p: &Part| match &p.source {
            PartSource::Packed { space, at_bits, size_bits, codec: Ok(codec) } if lazy::can_index(*codec) && p.len > limits.whole as u64 => {
                match self.shared_input(doc, (*space, *at_bits, *size_bits)) {
                    Some(_) => 0,
                    None => size_bits / 8,
                }
            }
            _ => match self.joined_input(doc, p) {
                Some(_) => 0,
                None => run_bits(p) / 8,
            },
        };
        let too_large = match lazy {
            true => stitch.parts.iter().map(copied).sum::<u64>() > self.spaces.whole_cap() as u64,
            false => {
                stitch.parts.iter().map(run_bits).sum::<u64>() > u64::from(u32::MAX)
                    || stitch.parts.iter().any(|p| matches!(p.source, PartSource::Packed { size_bits, .. } if size_bits / 8 > crate::codec::CAP_BYTES as u64))
            }
        };
        too_large.then_some(Refusal::TooLargeCopy)
    }

    /// Read the parts of the stitched stream in space `id`, opened by the
    /// node at `path`, from where `walk` stands to the end, and make the
    /// stream ready to open. A go that runs out, or a part waiting for bytes,
    /// puts the walk back in its place in `Spaces::joins` and hands back.
    /// `own` is as in an index pass: a go that runs out in another call's walk
    /// notes that its call waits with no progress of its own to give.
    fn join_steps<S: Source>(&self, doc: &Document<S>, path: &[usize], id: u32, mut walk: JoinWalk, own: bool) -> R<Result<Joining, Refusal>> {
        let Some(stitch) = self.spaces.stitch(id) else { return fail("this stream is no longer open") };
        while walk.next < stitch.parts.len() {
            let i = walk.next;
            let part = &stitch.parts[i];
            // Charged once it is done, by what it cost (see `copy_cost` and
            // `unpack_cost`), so that a part waiting for bytes costs nothing
            // each time it is asked again. A part read through its index
            // pass, or a window at a time, is charged by that as well.
            let lazily = matches!(walk.made, Made::Lazy { .. });
            let stepped = match self.go.charge(0) {
                Ok(()) => match lazily {
                    false => self.join_part_whole(doc, stitch, i, &mut walk),
                    true => self.join_part_lazily(doc, stitch, i, &mut walk),
                },
                Err(busy) => Err(busy),
            };
            let cost = match stepped {
                Ok(Ok(cost)) => cost,
                Ok(Err(why)) => return Ok(Err(why)),
                Err(e) if e.interrupted() => {
                    let (space, at_bits, run_bits) = part_run(part);
                    let short = self.file_bytes_at_once(space, at_bits, run_bits) <= self.gather_window(doc);
                    let e = match e {
                        // The chunks of the parts after this one as well, up
                        // to a window of the file, so that a stream of many
                        // short parts loads a window of them a round rather
                        // than one. A part read a window at a time has asked
                        // for a window already.
                        EvalError::Pending(missing) if short => EvalError::Pending(self.probe_parts(doc, stitch, i + 1, missing, lazily)),
                        other => other,
                    };
                    // How far the walk has got, for a wait for bytes as for a
                    // go that ran out, as an index pass says it for both.
                    match own {
                        true => self.go.ran_out_in(walk.progress(stitch)),
                        false => self.go.ran_out_waiting(),
                    }
                    self.spaces.joins.borrow_mut().put(path, JoinPlace::Walking(walk));
                    return Err(e);
                }
                // A part that will not read refuses the whole, as a stream
                // that will not unpack refuses.
                Err(_) => return Ok(Err(Refusal::Failed)),
            };
            self.go.spent(cost);
            walk.read_bits += part_run(part).2;
            walk.next += 1;
        }
        Ok(Ok(match walk.made {
            Made::Whole { bytes, mut trace, axis } => {
                trace.finish_at(axis, stitch.len_bytes);
                Joining::Whole(Whole { bytes, trace: trace.done(), runs: walk.runs })
            }
            Made::Lazy { parts } => {
                let joined = Arc::new(Joined::new(parts, stitch.len_bytes, self.spaces.cache_cap()));
                Joining::Lazy { joined, runs: walk.runs }
            }
        }))
    }

    /// Part `i` of a stream held whole, added to `walk`, and what it cost the
    /// go. Nothing is added unless the whole part is: a read that waits for
    /// bytes leaves the walk as it was.
    fn join_part_whole<S: Source>(&self, doc: &Document<S>, stitch: &Stitch, i: usize, walk: &mut JoinWalk) -> R<Result<u64, Refusal>> {
        let part = &stitch.parts[i];
        let (space, at_bits, run_bits) = part_run(part);
        let out = part.start;
        let (unpacked, t) = match &part.source {
            PartSource::Stored { .. } => match self.gather_run(doc, space, at_bits, run_bits)? {
                Ok(bytes) => (bytes, None),
                Err(why) => return Ok(Err(why)),
            },
            PartSource::Packed { codec, .. } => {
                let codec = match codec {
                    Ok(codec) => *codec,
                    Err(why) => return Ok(Err(*why)),
                };
                let packed = match self.gather_run(doc, space, at_bits, run_bits)? {
                    Ok(packed) => packed,
                    Err(why) => return Ok(Err(why)),
                };
                let Ok((unpacked, t)) = crate::codec::decode_traced(codec, &packed) else {
                    return Ok(Err(Refusal::Failed));
                };
                // The same test a read of the part makes: a part that
                // unpacks to another length than its run claimed is not the
                // part the stream was measured with.
                if unpacked.len() as u64 != part.len {
                    return Ok(Err(Refusal::Failed));
                }
                (unpacked, Some(t))
            }
        };
        let Made::Whole { bytes, trace, axis } = &mut walk.made else { unreachable!("a walk held whole") };
        bytes.extend_from_slice(&unpacked);
        match t {
            None if part.len > 0 => trace.push(*axis, out, StepKind::Stored),
            None => {}
            // A trace too long to keep names this part as one step, the way
            // a decoder past the same ceiling names a block.
            Some(t) if trace.steps() + t.len() > crate::codec::MAX_STEPS => {
                trace.push(*axis, out, StepKind::Block);
                trace.coarsen();
            }
            Some(t) => trace.absorb_part(&t, *axis, out),
        }
        trace.member(*axis..*axis + run_bits, out..out + part.len);
        walk.runs.push(JoinedRun {
            out_bytes: out..out + part.len,
            path: part.path.clone(),
            run_space: space,
            run_offset_bits: at_bits,
            run_bits,
            packed: matches!(part.source, PartSource::Packed { .. }),
            in_start: *axis,
        });
        *axis += run_bits;
        Ok(Ok(match part.source {
            PartSource::Packed { .. } => unpack_cost(part.len),
            PartSource::Stored { .. } => copy_cost(part.len),
        }))
    }

    /// Part `i` of a stream too large to hold whole, added to `walk`, and
    /// what it cost the go. A stored part's bytes and a packed part's run
    /// are read where they are when the stream is read, through the handle
    /// [`Self::joined_input`] gives, and copied and kept only where it gives
    /// none. A packed part too large to decode whole is read through its
    /// index. Refused as [`Refusal::TooLargeLazy`] when a part unpacks to
    /// more than [`lazy::LAZY_CAP_BYTES`], as [`Refusal::TooLargeUnit`] when
    /// a part has a zstd frame or xz block past 64 MiB, and as failed when a
    /// part read through its index will not read or unpacks to another
    /// length than its run claimed.
    ///
    /// A packed part small enough to decode whole is not decoded here. Its
    /// length is the one the file gives for it, a BGZF block's trailer, and
    /// it is checked when a read first unpacks it (`Joined::unpacked`): a
    /// part that will not unpack, or unpacks to another length, makes every
    /// read of its bytes fail, and [`Self::unreadable`] says why. Decoding
    /// every part here was most of the time a large BAM took to open, a
    /// whole inflate of the stream before its tab could show a byte. See
    /// "Faster opening of joined streams" in
    /// `docs/DESIGN-lazy-compressed-input.md`. The exception is a part the
    /// file says unpacks to nothing, which no read reaches: that one is
    /// decoded here, and refuses the stream as failed when it unpacks to
    /// anything.
    fn join_part_lazily<S: Source>(&self, doc: &Document<S>, stitch: &Stitch, i: usize, walk: &mut JoinWalk) -> R<Result<u64, Refusal>> {
        let part = &stitch.parts[i];
        let (space, at_bits, run_bits) = part_run(part);
        let out = part.start..part.start + part.len;
        let (bytes, cost) = match &part.source {
            PartSource::Stored { .. } => match self.joined_input(doc, part) {
                // Nothing of it is read until the stream is.
                Some(input) => (JoinedBytes::Stored(input), 1),
                None => match self.gather_run(doc, space, at_bits, run_bits)? {
                    Ok(bytes) => (JoinedBytes::Stored(lazy::RunInput::held(Arc::new(bytes))), copy_cost(part.len)),
                    Err(why) => return Ok(Err(why)),
                },
            },
            PartSource::Packed { codec, .. } => {
                let codec = match codec {
                    Ok(codec) => *codec,
                    Err(why) => return Ok(Err(*why)),
                };
                match self.index_part(doc, stitch, i)? {
                    Ok(Some(index)) => (JoinedBytes::Indexed(index), unpack_cost(part.len)),
                    // A part past `LAZY_CAP_BYTES`, with a zstd frame or xz
                    // block past 64 MiB, or whose run is past
                    // `Limits::input`, refuses the stream for that.
                    Err(PartTrouble::Refused(why @ (Refusal::TooLargeLazy | Refusal::TooLargeInput | Refusal::TooLargeUnit(_) | Refusal::TooMuchAtOnce))) => return Ok(Err(why)),
                    Err(_) => return Ok(Err(Refusal::Failed)),
                    Ok(None) => {
                        // As `unpack_run` refuses it.
                        if run_bits / 8 > crate::codec::CAP_BYTES as u64 {
                            return Ok(Err(Refusal::TooLargeCopy));
                        }
                        // A part the file says unpacks to nothing gives the
                        // stream no bytes, so no read ever lands on it and
                        // checks it. It is checked here, as every part was
                        // before, and one that unpacks to something refuses
                        // the stream, since those bytes would otherwise go
                        // missing without a word. A BGZF file's end block is
                        // one, 28 bytes.
                        let empty = match part.len {
                            0 => match self.gather_run(doc, space, at_bits, run_bits)? {
                                Ok(run) => match crate::codec::decode(codec, &run) {
                                    Ok(unpacked) if unpacked.is_empty() => Some(run),
                                    _ => return Ok(Err(Refusal::Failed)),
                                },
                                Err(why) => return Ok(Err(why)),
                            },
                            _ => None,
                        };
                        match self.joined_input(doc, part) {
                            // Nothing of it is read until the stream is.
                            Some(input) => (JoinedBytes::Packed { input, codec }, 1),
                            None => match empty.map_or_else(|| self.gather_run(doc, space, at_bits, run_bits), |run| Ok(Ok(run)))? {
                                Ok(run) => (JoinedBytes::Packed { input: lazy::RunInput::held(Arc::new(run)), codec }, copy_cost(run_bits / 8)),
                                Err(why) => return Ok(Err(why)),
                            },
                        }
                    }
                }
            }
        };
        let Made::Lazy { parts } = &mut walk.made else { unreachable!("a walk read a part at a time") };
        parts.push((out.clone(), bytes));
        let packed = matches!(part.source, PartSource::Packed { .. });
        walk.runs.push(JoinedRun { out_bytes: out, path: part.path.clone(), run_space: space, run_offset_bits: at_bits, run_bits, packed, in_start: 0 });
        Ok(Ok(cost))
    }

    /// What part `p` of a stream joined a part at a time is read through:
    /// a handle onto the source its bytes are in, when one reaches them
    /// ([`Self::shared_input`]). For a packed part decoded whole, that is
    /// its run, which is read whole in one read each time the part is
    /// decoded, so where the source keeps only part of the file the run has
    /// to be short: at most a gather window, and at most an eighth of what
    /// the source keeps, which is what `Joined::resident_limit` counts on.
    /// A BGZF block is under 64 KiB. Nothing when the part has to be copied:
    /// a part in a stitched space or an edited stretch, a part of a source
    /// that gives no handle, and a longer run. Reads nothing.
    ///
    /// A packed part read through its index is not asked: its index reads
    /// its run through a handle of its own when one reaches it.
    pub(super) fn joined_input<S: Source>(&self, doc: &Document<S>, p: &Part) -> Option<lazy::RunInput> {
        let (space, at_bits, run_bits) = part_run(p);
        let input = self.shared_input(doc, (space, at_bits, run_bits))?;
        let short = |keeps: u64| input.len() <= (keeps / 8).min(self.gather_window(doc));
        match p.source {
            PartSource::Stored { .. } => Some(input),
            PartSource::Packed { .. } => input.resident_limit().is_none_or(short).then_some(input),
        }
    }

    /// `missing`, and the chunks of the file that the parts from `from` on
    /// are missing, as far as a window of the file goes: read here only to
    /// ask for them, and read again when the walk reaches them. A walk
    /// joining a stream to read a part at a time (`lazily`) reads nothing of
    /// a part with a handle, stored or decoded whole, so nothing is asked
    /// for it.
    fn probe_parts<S: Source>(&self, doc: &Document<S>, stitch: &Stitch, from: usize, mut missing: Vec<crate::source::Missing>, lazily: bool) -> Vec<crate::source::Missing> {
        let window = self.gather_window(doc);
        let mut reach = 0;
        let limits = self.spaces.lazy_limits();
        // A packed part read through its index, or one the file says unpacks
        // to nothing, is read by the walk all the same.
        let read_anyway = |p: &Part| match p.source {
            PartSource::Packed { codec: Ok(codec), .. } => p.len == 0 || (lazy::can_index(codec) && p.len > limits.whole as u64),
            PartSource::Packed { .. } | PartSource::Stored { .. } => false,
        };
        for part in stitch.parts.iter().skip(from) {
            if lazily && !read_anyway(part) && self.joined_input(doc, part).is_some() {
                continue;
            }
            let (space, at_bits, run_bits) = part_run(part);
            let need = self.file_bytes_at_once(space, at_bits, run_bits);
            reach += need;
            if reach > window {
                break;
            }
            if need == 0 {
                continue;
            }
            if let Err(EvalError::Pending(more)) = self.read_in(doc, space, at_bits, run_bits) {
                missing.extend(more);
            }
        }
        missing.sort_by_key(|m| m.chunk);
        missing.dedup();
        missing
    }

    /// Wait for a free place in `Spaces::joins`, for a walk about to start,
    /// as an index pass waits for a place in `Unfinished`
    /// ([`Self::wait_for_a_place`](super::Evaluator)): while both places hold
    /// walks under way, this go carries on the one with the fewest bits of
    /// runs left to read, and what it comes to is kept in its place for its
    /// own call to take. When both places hold joins finished that way and
    /// not yet taken, the one put there first is let go of.
    fn wait_for_a_join_place<S: Source>(&self, doc: &Document<S>) -> R<()> {
        loop {
            if self.spaces.joins.borrow().has_room() {
                return Ok(());
            }
            let nearest = self.spaces.joins.borrow_mut().take_nearest_its_end();
            let Some((other, walk)) = nearest else {
                self.spaces.joins.borrow_mut().drop_oldest_done();
                continue;
            };
            let found = match self.spaces.get(&other) {
                Some(Opened::Space(id)) => match self.join_steps(doc, &other, id, walk, false) {
                    Ok(found) => found,
                    Err(e) if e.interrupted() => return Err(e),
                    Err(_) => Err(Refusal::Failed),
                },
                // Its stream is no longer open, so nothing would take it.
                _ => continue,
            };
            self.spaces.joins.borrow_mut().put(&other, JoinPlace::Done(found));
        }
    }

    /// Part `i` as a reader would name it, for a read that failed in it: the
    /// field the walk landed on, `blocks[2].compressed`. Where that cannot be
    /// said any more, its index, marked as one so that `2` is not read as the
    /// second.
    fn part_label(&self, stitch: &Stitch, i: usize) -> String {
        let label = match self.memo.get(&stitch.path).map(|r| &r.ty) {
            Some(Ty::Stitched { from, .. }) => self.walk_label_here(&stitch.path, from, &stitch.parts[i].path),
            _ => String::new(),
        };
        if label.is_empty() { unnamed_part(i) } else { label }
    }

    /// The first stretch of bytes `at..at + len` of the joined stream opened
    /// as space `id` that a read gave zeroes for, because the part they are
    /// in will not unpack or unpacks to another length than the file says,
    /// and why, with the part named as the listing names it:
    /// `blocks[50].compressed unpacks to 2098 bytes, not the 2099 the file
    /// says`. Asked after a read of those bytes that found no chunk of the
    /// file missing, since the read is what finds a part out: see
    /// [`Source::unreadable`]. Nothing for bytes that read, and for a space
    /// that is not a joined stream read a part at a time.
    pub fn unreadable(&self, id: SpaceId, at: u64, len: u64) -> Option<crate::source::Unreadable> {
        let space = self.space(id)?;
        let super::space::SpaceSource::Joined(joined) = space.source() else { return None };
        let (i, out, trouble) = joined.trouble_in(at, len)?;
        // The stream's parts are the stitched space's, in the same order,
        // when that space is open in this reading under the same path: only
        // a stream opened from the file's own fields is known to be.
        let stitch = match (space.parent, self.spaces.get(&space.path)) {
            (0, Some(Opened::Space(stitched))) => self.spaces.stitch(stitched).filter(|s| s.parts.len() == joined.part_count()),
            _ => None,
        };
        let label = stitch.map_or_else(|| unnamed_part(i), |s| self.part_label(s, i));
        Some(unreadable_part(&label, out, trouble))
    }
}

/// Which part of a stitched stream a byte of it came from, and where in that
/// part. See [`Evaluator::part_of`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartHit {
    /// Which part, counting from 0 in the order the stream goes, and how many
    /// parts the stream has.
    pub index: usize,
    pub parts: usize,
    /// The field the part is: a PDB page, or a BGZF block's compressed run. A
    /// place a reader can go to.
    pub path: Vec<usize>,
    /// That field as a reader would name it, the way a gathered element names
    /// its descriptor: `blocks[3].compressed`.
    pub label: String,
    /// The same field named by every step the file stores on the way to it,
    /// where an encoding's own steps make that differ from `label`. See
    /// [`crate::eval::Origin::stored`].
    pub stored: Option<String>,
    /// The byte's place inside what the part gives, and how much that is.
    pub in_part: u64,
    pub part_len: u64,
    /// Where the part's run starts, in the space it is a field of. 0 is the
    /// file.
    pub run_space: u32,
    pub run_offset_bits: u64,
    /// Whether the run was unpacked to give the part, rather than read as it
    /// sits.
    pub packed: bool,
    /// For a part that is a BGZF block, the byte as a BAI or a CSI names it:
    /// where the block starts in the file, shifted up sixteen bits, with the
    /// byte's place in what the block unpacks to below. The block, not the
    /// deflate run inside it, which starts eighteen bytes later.
    pub virtual_offset: Option<u64>,
}

impl Evaluator {
    /// Which part byte `byte` of `space` came from, when `space` is a stitched
    /// stream's. `space` is the number a node inside the stream carries
    /// ([`NodeInfo::space`]), not a tab's. Nothing for any other space, and
    /// for a byte past the end.
    ///
    /// The reverse of a read, from the part table alone: nothing is unpacked
    /// to answer, since the question is where the byte is kept rather than what
    /// it holds.
    pub fn part_of<S: Source>(&mut self, doc: &Document<S>, space: u32, byte: u64) -> R<Option<PartHit>> {
        let Some(stitch) = self.spaces.stitch(space) else { return Ok(None) };
        let Some(index) = stitch.part_at(byte) else { return Ok(None) };
        let (part, parts, owner) = (stitch.parts[index].clone(), stitch.parts.len(), stitch.path.clone());
        let (run_space, run_offset_bits, packed) = match part.source {
            PartSource::Stored { space, at_bits } => (space, at_bits, false),
            PartSource::Packed { space, at_bits, .. } => (space, at_bits, true),
        };
        self.resolve(doc, &owner)?;
        let Ty::Stitched { from, .. } = self.memo[&owner].ty.clone() else { return Ok(None) };
        let walked = self.walk_label(doc, &owner, &from, &part.path)?;
        let (label, stored) = match self.short_label(doc, &owner, &part.path) {
            Ok(Some(short)) if short != walked => (short, Some(walked)),
            Err(e) if e.interrupted() => return Err(e),
            _ => (walked, None),
        };
        let in_part = byte - part.start;
        let virtual_offset = match self.bgzf_block_of(doc, &part.path)? {
            Some(block_bits) if in_part < 1 << 16 => Some((block_bits / 8) << 16 | in_part),
            _ => None,
        };
        Ok(Some(PartHit {
            index,
            parts,
            path: part.path,
            label,
            stored,
            in_part,
            part_len: part.len,
            run_space,
            run_offset_bits,
            packed,
            virtual_offset,
        }))
    }

    /// Where in the file a write to the field `r` goes, when `r` is in a space
    /// that is not the file: the bit its first bit is at.
    ///
    /// Only a field of a joined stream that lies wholly in one run stored as
    /// it sits in the file has such a place. Those bits are a stretch of the
    /// file under another address, and writing them is writing the file. A
    /// field that runs from one run into the next is refused naming both,
    /// since the two are apart in the file and a write is one stretch; the hex
    /// view can change each half. A field in a run that had to be unpacked,
    /// and any field of an unpacked stream, is in no place in the file at all,
    /// and gets the refusal that says so.
    pub(super) fn joined_write(&self, r: &Resolved, size: u64) -> Result<u64, String> {
        let unpacked = || encode::UNPACKED_MSG.to_string();
        let Some(stitch) = self.spaces.stitch(r.space) else { return Err(unpacked()) };
        let first_byte = r.offset / 8;
        let last_byte = ((r.offset + size).div_ceil(8)).max(first_byte + 1) - 1;
        let Some(first) = stitch.part_at(first_byte) else { return Err(unpacked()) };
        let last = stitch.part_at(last_byte).unwrap_or(first);
        if last != first {
            // Named the way the listing names each run. Two apart or more is
            // a range, since naming two of three would leave one out.
            let joiner = if last == first + 1 { "and" } else { "to" };
            return Err(format!(
                "Can't edit here: this field is split across {} {joiner} {}. Use the hex view.",
                self.part_label(stitch, first),
                self.part_label(stitch, last)
            ));
        }
        let part = &stitch.parts[first];
        match part.source {
            PartSource::Stored { space: 0, at_bits } => Ok(at_bits + (r.offset - part.start * 8)),
            _ => Err(unpacked()),
        }
    }

    /// Where the BGZF block a run is a field of starts, when it is one: the
    /// nearest structure above the run, marked as the packing the BAM side
    /// reader answers for. The same mark the panel for a block's records goes
    /// by, so the two agree on what a block is.
    fn bgzf_block_of<S: Source>(&mut self, doc: &Document<S>, run: &[usize]) -> R<Option<u64>> {
        for k in (0..run.len()).rev() {
            self.resolve(doc, &run[..k])?;
            let r = &self.memo[&run[..k]];
            if let Ty::Struct(def) = r.ty.base() {
                let bgzf = def.packed.as_deref() == Some(crate::formats::bam_records::PACKING);
                return Ok(bgzf.then_some(r.offset));
            }
        }
        Ok(None)
    }
}

/// The parts cut at the stream's stated total, when it states one, and what
/// the total comes to. Parts past the cut go, and the one the cut falls in is
/// shortened. A total longer than the parts is not stretched to: those bytes
/// are nowhere, and the stream is as long as what there is of it.
fn cut_parts(mut parts: Vec<Part>, cut: Option<u64>) -> (Vec<Part>, u64) {
    let sum = parts.last().map_or(0, |p| p.start + p.len);
    let Some(cut) = cut.filter(|&c| c < sum) else { return (parts, sum) };
    parts.retain(|p| p.start < cut);
    if let Some(last) = parts.iter_mut().rev().find(|p| p.len > 0) {
        last.len = last.len.min(cut - last.start);
    }
    (parts, cut)
}

#[cfg(test)]
mod tests {
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;
    use crate::template::{Endian::Big, Expr as E, Step, Template, Ty as T};

    /// A made-up paged container, the shape of a PDB with the numbers made
    /// small: a page size, a count, that many page numbers in the order the
    /// stream's bytes go, the stream's length, and then the pages and the
    /// stream joined from them.
    ///
    /// `sizes` is the one thing a test varies: with it, each page is as long
    /// as the matching entry says rather than a whole page.
    fn paged(inner: T, sizes: bool) -> Template {
        let page = if sizes {
            T::bytes(E::elem("sizes", E::idx()))
        } else {
            T::bytes(E::field("page_size"))
        };
        let mut fields = vec![
            ("page_size", T::u8()),
            ("count", T::u8()),
            ("numbers", T::array(T::u8(), E::field("count"))),
            ("sizes", T::array(T::u8(), if sizes { E::field("count") } else { E::lit(0) })),
            ("total", T::u16(Big)),
        ];
        fields.push(("pages", T::array(T::at(E::elem("numbers", E::idx()).mul(E::field("page_size")), page), E::field("count"))));
        fields.push(("stream", T::stitched(vec![Step::field("pages"), Step::each()], None, Some(E::field("total")), inner)));
        Template::new("paged", T::structure("Paged", fields))
    }

    /// Field indices into the root of [`paged`].
    const PAGES: usize = 5;
    const STREAM: usize = 6;

    /// What the stream reads as: two numbers, one of them cut by the end of the
    /// first page, and the rest as bytes.
    fn record() -> T {
        T::structure(
            "Record",
            vec![("a", T::u16(Big)), ("b", T::u32(Big)), ("c", T::u32(Big)), ("rest", T::bytes(E::Remaining))],
        )
    }

    /// The stream the pages hold, joined: twenty bytes, and then four the
    /// length leaves out.
    const STREAM_BYTES: [u8; 24] = [
        0x11, 0x22, 0xaa, 0xbb, 0xcc, 0xdd, 0x01, 0x02, // page 3
        0x03, 0x04, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, // page 1
        0x61, 0x62, 0x63, 0x64, 0xee, 0xee, 0xee, 0xee, // page 2
    ];

    /// Eight-byte pages, the stream written across pages 3, 1 and 2 in that
    /// order, which is out of order on purpose: a stream that happened to be
    /// in order would read the same whether or not anything joined it.
    fn file(total: u16) -> Vec<u8> {
        let mut v = vec![8, 3, 3, 1, 2];
        v.extend_from_slice(&total.to_be_bytes());
        v.push(0);
        v.extend_from_slice(&STREAM_BYTES[8..16]);
        v.extend_from_slice(&STREAM_BYTES[16..24]);
        v.extend_from_slice(&STREAM_BYTES[0..8]);
        v
    }

    /// Three runs of four bytes, each a stream whose codec copies, as a ZIP
    /// declares a stored entry, and a stream joined from the third and the
    /// first, in that order, which is the order the walk names them in and not
    /// the order they are in the file.
    fn named_runs(inner: T) -> Template {
        let run = T::decoded(E::lit(4), crate::codec::Codec::Stored, T::bytes(E::Remaining));
        Template::new(
            "named",
            T::structure(
                "Named",
                vec![
                    ("runs", T::array(run, E::lit(3))),
                    ("joined", T::stitched(vec![Step::field("runs"), Step::elements(&[2, 7, 0])], None, None, inner)),
                ],
            ),
        )
    }

    #[test]
    fn a_stream_joined_from_named_elements_joins_them_in_that_order_and_counts_from_its_own_front() {
        let file: Vec<u8> = vec![0xa0, 0xa1, 0xa2, 0xa3, 0xb0, 0xb1, 0xb2, 0xb3, 0xc0, 0xc1, 0xc2, 0xc3];
        // Offsets from the front of the joined stream: from a field of the
        // stream, and from a field inside a record that has an origin of its
        // own, which `Anchor::Origin` would count from instead.
        let record = T::origin(T::structure(
            "Record",
            vec![("x", T::u8()), ("near", T::at_origin(E::lit(1), T::u8())), ("far", T::at_space(E::lit(6), T::u8()))],
        ));
        let inner = T::structure("Inner", vec![("head", T::u8()), ("pointed", T::at_space(E::lit(5), T::u8())), ("record", record)]);
        let d = Document::new(MemSource(file));
        let mut e = Evaluator::new(named_runs(inner));
        let joined = e.node(&d, &[1, 0]).unwrap();
        // Index 7 is past the end of the list and is passed over.
        assert_eq!(e.spaces.len_bits(joined.space), 8 * 8, "the third run and the first");
        assert_eq!(e.node(&d, &[1, 0, 0]).unwrap().value, Value::UInt(0xc0));
        let pointed = e.node(&d, &[1, 0, 1, 0]).unwrap();
        assert_eq!((pointed.value, pointed.space, pointed.offset_bits), (Value::UInt(0xa1), joined.space, 5 * 8));
        assert_eq!(e.node(&d, &[1, 0, 2, 1, 0]).unwrap().value, Value::UInt(0xc2), "from the record's front");
        assert_eq!(e.node(&d, &[1, 0, 2, 2, 0]).unwrap().value, Value::UInt(0xa2), "from the stream's front");
        // Runs the format stored as they are are read where they sit: nothing
        // was copied into the cache to read them.
        let stitch = e.spaces.stitch(joined.space).expect("a stitched space");
        assert!(stitch.parts.iter().all(|p| matches!(p.source, super::PartSource::Stored { .. })));
        assert_eq!(stitch.cache.borrow().peak, 0);
        let hit = e.part_of(&d, joined.space, 5).unwrap().unwrap();
        assert_eq!((hit.label.as_str(), hit.in_part, hit.packed), ("runs[0]", 1, false));
        // The walk is written with its indices, in the order they go, and the
        // pointers with what they count from.
        let text = crate::template_text::render(e.template());
        assert!(text.contains("runs[2, 7, 0]"), "{text}");
        assert!(text.contains("stream"), "{text}");
    }

    #[test]
    fn a_stream_kept_in_scattered_pages_reads_as_one() {
        let d = Document::new(MemSource(file(20)));
        let mut e = Evaluator::new(paged(record(), false));
        let stream = e.node(&d, &[STREAM]).unwrap();
        assert_eq!((stream.size_bits, stream.child_count), (0, 1), "no bytes where it is declared, and one thing inside");
        let inside = e.node(&d, &[STREAM, 0]).unwrap();
        assert_ne!(inside.space, 0);
        assert_eq!((inside.offset_bits, inside.size_bits), (0, 20 * 8));
        assert!(inside.space_root);
        assert_eq!(e.field_bytes(&d, &[STREAM, 0], 64).unwrap().0, STREAM_BYTES[..20]);
        assert_eq!(e.node(&d, &[STREAM, 0, 0]).unwrap().value, Value::UInt(0x1122));
        assert_eq!(e.node(&d, &[STREAM, 0, 1]).unwrap().value, Value::UInt(0xaabbccdd));
    }

    #[test]
    fn a_field_cut_across_two_parts_reads_whole() {
        let d = Document::new(MemSource(file(20)));
        let mut e = Evaluator::new(paged(record(), false));
        // Two bytes of page 3 and two of page 1, which are eight bytes apart
        // in the file and the wrong way round.
        let c = e.node(&d, &[STREAM, 0, 2]).unwrap();
        assert_eq!((c.offset_bits, c.value), (6 * 8, Value::UInt(0x0102_0304)));
        // And bit by bit: a read that does not start on a byte starts in the
        // right part all the same: the low half of page 3's last byte, 0x02,
        // and the high half of page 1's first, 0x03.
        assert_eq!(e.read_in(&d, c.space, 7 * 8 + 4, 8).unwrap(), [0x20]);
        assert_eq!(e.read_in(&d, c.space, 7 * 8 + 4, 12).unwrap(), [0x20, 0x30]);
    }

    #[test]
    fn a_declared_length_cuts_the_last_part() {
        let d = Document::new(MemSource(file(20)));
        let mut e = Evaluator::new(paged(record(), false));
        let rest = e.node(&d, &[STREAM, 0, 3]).unwrap();
        assert_eq!(rest.size_bits, 10 * 8, "the rest runs to the length and not to the end of the last page");
        assert_eq!(e.field_bytes(&d, &[STREAM, 0, 3], 64).unwrap().0, STREAM_BYTES[10..20]);
        // A length past what the pages hold is not stretched to.
        let d = Document::new(MemSource(file(100)));
        let mut e = Evaluator::new(paged(record(), false));
        assert_eq!(e.node(&d, &[STREAM, 0]).unwrap().size_bits, 24 * 8);
    }

    #[test]
    fn a_part_of_no_bytes_adds_nothing() {
        // Sixteen-byte pages, of which page 3 gives six bytes, page 1 none and
        // page 2 eight.
        let mut v = vec![16, 3, 3, 1, 2, 6, 0, 8];
        v.extend_from_slice(&14u16.to_be_bytes());
        v.resize(64, 0);
        v[32..40].copy_from_slice(&[0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68]);
        v[48..54].copy_from_slice(&[0x11, 0x22, 0xaa, 0xbb, 0xcc, 0xdd]);
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(paged(record(), true));
        let inside = e.node(&d, &[STREAM, 0]).unwrap();
        assert_eq!(inside.size_bits, 14 * 8);
        // `b` ends where page 3's six bytes do, and `c` starts in page 2 with
        // page 1 contributing nothing between them.
        assert_eq!(e.node(&d, &[STREAM, 0, 1]).unwrap().value, Value::UInt(0xaabbccdd));
        assert_eq!(e.node(&d, &[STREAM, 0, 2]).unwrap().value, Value::UInt(0x61626364));
        let space = inside.space;
        let stitch = e.spaces.stitch(space).expect("a stitched space");
        assert_eq!(stitch.parts.iter().map(|p| (p.start, p.len)).collect::<Vec<_>>(), [(0, 6), (6, 0), (6, 8)]);
        assert_eq!(stitch.part_at(6), Some(2), "a byte at the start of an empty part is in the next one");
    }

    #[test]
    fn the_cursor_on_a_page_lands_on_the_page_and_not_on_the_stream() {
        let d = Document::new(MemSource(file(20)));
        let mut e = Evaluator::new(paged(record(), false));
        e.node(&d, &[STREAM, 0, 2]).unwrap();
        // Byte 1 of page 3 is byte 1 of the stream, and no bit of the file is a
        // field of the stream: the cursor stops at the page.
        let at = e.locate(&d, 25 * 8).unwrap();
        assert_eq!(at, [PAGES, 0, 0]);
        let page = e.node(&d, &at).unwrap();
        assert_eq!((page.space, page.offset_bits), (0, 24 * 8));
    }

    #[test]
    fn a_field_inside_one_stored_part_writes_to_that_part() {
        let mut d = Document::new(MemSource(file(20)));
        let mut e = Evaluator::new(paged(record(), false));
        let b = e.node(&d, &[STREAM, 0, 1]).unwrap();
        assert!(b.editable, "all four bytes of `b` are in page 3");
        // `b` is two bytes into the stream and so two bytes into page 3, which
        // is at 0x18 in the file: the write goes there.
        let w = e.prepare_write(&d, &[STREAM, 0, 1], "305419896").unwrap();
        assert_eq!((w.offset_bits, w.n_bits), (0x1a * 8, 32));
        d.overwrite_bytes(w.offset_bits / 8, &w.data);
        e.invalidate_from(w.offset_bits);
        assert_eq!(e.node(&d, &[STREAM, 0, 1]).unwrap().value, Value::UInt(0x1234_5678));
        assert_eq!(e.field_bytes(&d, &[PAGES, 0, 0], 8).unwrap().0[2..6], [0x12, 0x34, 0x56, 0x78], "the page itself holds it");
        // A field of a joined stream whose run was unpacked says what any
        // unpacked field says: its bytes are in no place in the file.
        let d = Document::new(MemSource(bgzf_of(&[b"some text in one block"])));
        let mut e = Evaluator::new(crate::formats::bgzf());
        let text = e.child_named(&d, &JOINED, "text").unwrap().unwrap();
        assert!(!e.node(&d, &text).unwrap().editable);
        assert_eq!(e.prepare_write(&d, &text, "x"), Err(crate::eval::EvalError::Failed(crate::encode::UNPACKED_MSG.into())));
    }

    #[test]
    fn a_field_across_two_stored_parts_is_refused_naming_both() {
        let d = Document::new(MemSource(file(20)));
        let mut e = Evaluator::new(paged(record(), false));
        // `c` is the last two bytes of page 3 and the first two of page 1,
        // which are sixteen bytes apart the other way round in the file.
        assert!(!e.node(&d, &[STREAM, 0, 2]).unwrap().editable);
        assert_eq!(
            e.prepare_write(&d, &[STREAM, 0, 2], "1"),
            Err(crate::eval::EvalError::Failed(
                "Can't edit here: this field is split across pages[0] and pages[1]. Use the hex view.".into()
            ))
        );
        // And `rest` runs from page 1 into page 2 and no further, so it names
        // those two.
        match e.prepare_write(&d, &[STREAM, 0, 3], "x") {
            Err(crate::eval::EvalError::Failed(why)) => assert!(why.contains("split across pages[1] and pages[2]"), "{why}"),
            other => panic!("{other:?}"),
        }
    }

    /// A BGZF file of `pieces`, one block each, and the block every BGZF file
    /// ends with.
    fn bgzf_of(pieces: &[&[u8]]) -> Vec<u8> {
        use crate::formats::bam::tests::{bgzf_block, EOF_BLOCK};
        let mut file: Vec<u8> = pieces.iter().flat_map(|p| bgzf_block(p)).collect();
        file.extend_from_slice(&EOF_BLOCK);
        file
    }

    /// The joined stream of a `bgzf` reading, and its space.
    const JOINED: [usize; 2] = [1, 0];

    fn stitch_of(e: &Evaluator, space: u32) -> &super::Stitch {
        e.spaces.stitch(space).expect("a stitched space")
    }

    #[test]
    fn blocks_of_one_stream_read_as_the_stream() {
        let text = b"one line of a vcf\nand another that the writer cut\nand a third\n";
        let d = Document::new(MemSource(bgzf_of(&[&text[..20], &text[20..41], &text[41..]])));
        let mut e = Evaluator::new(crate::formats::bgzf());
        let node = e.node(&d, &JOINED).unwrap();
        assert_eq!(node.size_bits, text.len() as u64 * 8);
        assert_eq!(e.field_bytes(&d, &JOINED, 1024).unwrap().0, text);
        let stitch = stitch_of(&e, node.space);
        // Three blocks and the end block, which joins nothing on.
        let end = text.len() as u64;
        assert_eq!(stitch.parts.iter().map(|p| (p.start, p.len)).collect::<Vec<_>>(), [(0, 20), (20, 21), (41, end - 41), (end, 0)]);
    }

    #[test]
    fn a_member_is_measured_from_its_trailer_and_not_inflated() {
        let pieces: Vec<Vec<u8>> = (0..5).map(|i| vec![b'a' + i; 3000]).collect();
        let d = Document::new(MemSource(bgzf_of(&pieces.iter().map(|p| p.as_slice()).collect::<Vec<_>>())));
        let mut e = Evaluator::new(crate::formats::bgzf());
        // Opening the stream is enough to know how long it is, and nothing is
        // unpacked to find out: no block's run is open and the cache is empty.
        assert_eq!(e.node(&d, &[1]).unwrap().child_count, 1);
        let space = match e.spaces.get(&[1]) {
            Some(super::Opened::Space(id)) => id,
            other => panic!("the stream did not open: {other:?}"),
        };
        assert_eq!(e.spaces.len_bits(space), 15000 * 8);
        for part in &stitch_of(&e, space).parts {
            assert_eq!(e.spaces.get(&part.path), None, "block run {:?} was opened", part.path);
        }
        assert_eq!(stitch_of(&e, space).cache.borrow().peak, 0);
        // A read in the fourth block unpacks that block and no other.
        assert_eq!(e.read_in(&d, space, 9500 * 8, 8).unwrap(), [b'd']);
        assert_eq!(stitch_of(&e, space).cache.borrow().held(), (vec![3], 3000));
    }

    /// The structure a stream sits in reads as a row like any other, and
    /// asking what that row says unpacks nothing.
    ///
    /// The reading of a small structure is its fields' values in order, and a
    /// stream's value is whatever comes out of it. Walking into one to write a
    /// line would unpack every stream the annotation column drew a row for,
    /// which is every stream in the file as the reader scrolls past. So a
    /// stream reads as its size, the same as any other run of bytes.
    #[test]
    fn the_record_a_stream_sits_in_reads_without_unpacking_it() {
        let pieces: Vec<Vec<u8>> = (0..5).map(|i| vec![b'a' + i; 3000]).collect();
        let d = Document::new(MemSource(bgzf_of(&pieces.iter().map(|p| p.as_slice()).collect::<Vec<_>>())));
        let mut e = Evaluator::new(crate::formats::bgzf());
        assert!(e.node(&d, &[]).unwrap().composite);
        let space = match e.spaces.get(&[1]) {
            Some(super::Opened::Space(id)) => id,
            other => panic!("the stream did not open: {other:?}"),
        };
        for part in &stitch_of(&e, space).parts {
            assert_eq!(e.spaces.get(&part.path), None, "block run {:?} was opened", part.path);
        }
        assert_eq!(stitch_of(&e, space).cache.borrow().peak, 0);
    }

    /// A BAM of a header block and two blocks of records, the second of which
    /// is changed by `spoil` after it is written.
    fn bam_in_blocks(spoil: impl Fn(&mut Vec<u8>)) -> Vec<u8> {
        use crate::formats::bam::tests::{bam_header, bgzf_block, two_records, EOF_BLOCK};
        let (odd, even) = two_records();
        let mut file = bgzf_block(&bam_header("@HD\tVN:1.6\n", &[("chr1", 1000)]));
        let records = [odd.as_slice(), even.as_slice()].concat();
        file.extend_from_slice(&bgzf_block(&records));
        let mut last = bgzf_block(&records);
        spoil(&mut last);
        file.extend_from_slice(&last);
        file.extend_from_slice(&EOF_BLOCK);
        file
    }

    /// The records of the joined stream, and the evaluator reading it.
    fn records_of(d: &Document<MemSource>) -> (Evaluator, Vec<usize>) {
        let mut e = Evaluator::new(crate::formats::bgzf());
        let records = e.child_named(d, &JOINED, "records").unwrap().expect("a BAM");
        (e, records)
    }

    #[test]
    fn a_part_that_will_not_unpack_ends_the_stream_there_and_says_which() {
        // The first byte of the last block's deflate, made a block type that
        // does not exist.
        let file = bam_in_blocks(|b| b[18] = 0xff);
        let d = Document::new(MemSource(file));
        let (mut e, records) = records_of(&d);
        // The two records of the good block read, and the run stops where the
        // bad block begins, saying which part it was.
        assert_eq!(e.node(&d, &records).unwrap().child_count, 2);
        let why = e.list(&records).repeat_trouble.clone().expect("the run says why it stopped");
        assert!(why.contains("blocks[2].compressed could not be unpacked"), "{why}");
        // The blocks themselves are still what they are.
        assert_eq!(e.node(&d, &[0]).unwrap().child_count, 4);
    }

    #[test]
    fn a_part_that_comes_to_another_length_than_claimed_is_refused() {
        // The last block's trailer claims five bytes more than it unpacks to.
        let file = bam_in_blocks(|b| {
            let at = b.len() - 4;
            let size = u32::from_le_bytes(b[at..].try_into().unwrap()) + 5;
            b[at..].copy_from_slice(&size.to_le_bytes());
        });
        let d = Document::new(MemSource(file));
        let (mut e, records) = records_of(&d);
        assert_eq!(e.node(&d, &records).unwrap().child_count, 2);
        let why = e.list(&records).repeat_trouble.clone().expect("the run says why it stopped");
        let (odd, even) = crate::formats::bam::tests::two_records();
        let got = odd.len() + even.len();
        assert_eq!(why, format!("blocks[2].compressed unpacks to {got} bytes, not the {} the file says", got + 5));
    }

    #[test]
    fn parts_inflate_when_read_and_go_when_the_cache_is_full() {
        let pieces: Vec<Vec<u8>> = (0..4).map(|i| vec![b'a' + i; 1000]).collect();
        let d = Document::new(MemSource(bgzf_of(&pieces.iter().map(|p| p.as_slice()).collect::<Vec<_>>())));
        let mut e = Evaluator::new(crate::formats::bgzf());
        // Room for two blocks and a half.
        e.spaces.set_cache_cap(2500);
        let space = e.node(&d, &JOINED).unwrap().space;
        let byte = |e: &Evaluator, part: u64| e.read_in(&d, space, (part * 1000 + 7) * 8, 8).unwrap()[0];
        let held = |e: &Evaluator| {
            let (mut parts, bytes) = stitch_of(e, space).cache.borrow().held();
            parts.sort();
            (parts, bytes)
        };
        assert_eq!(byte(&e, 0), b'a');
        assert_eq!(byte(&e, 1), b'b');
        assert_eq!(held(&e), (vec![0, 1], 2000));
        // A third goes past the cap, and the part read longest ago makes room.
        assert_eq!(byte(&e, 2), b'c');
        assert_eq!(held(&e), (vec![1, 2], 2000));
        // Reading part 1 again makes part 2 the oldest, so part 0 coming back
        // puts part 2 out.
        assert_eq!(byte(&e, 1), b'b');
        assert_eq!(byte(&e, 0), b'a');
        assert_eq!(held(&e), (vec![0, 1], 2000));
        assert_eq!(stitch_of(&e, space).cache.borrow().peak, 2000);
        // A read across the edge of two parts takes a byte of each.
        assert_eq!(e.read_in(&d, space, 2999 * 8, 16).unwrap(), [b'c', b'd']);
    }

    #[test]
    fn a_walk_that_runs_out_carries_on_from_the_part_it_stood_on() {
        let pieces: Vec<Vec<u8>> = (0..60).map(|i| vec![i as u8; 100]).collect();
        let d = Document::new(MemSource(bgzf_of(&pieces.iter().map(|p| p.as_slice()).collect::<Vec<_>>())));
        let parts = |e: &Evaluator| {
            let Some(super::Opened::Space(id)) = e.spaces.get(&[1]) else { panic!("not open") };
            stitch_of(e, id).parts.iter().map(|p| (p.start, p.len, p.path.clone())).collect::<Vec<_>>()
        };
        let mut whole = Evaluator::new(crate::formats::bgzf());
        whole.node(&d, &[1]).unwrap();

        // Ten elements a go. A walk that went back over the parts an earlier
        // go had found would never get further than ten of the sixty-one.
        let mut e = Evaluator::new(crate::formats::bgzf());
        e.set_slice(Some(10));
        let mut goes = 0;
        loop {
            e.begin_slice();
            match e.node(&d, &[1]) {
                Ok(_) => break,
                Err(crate::eval::EvalError::Busy { .. }) => goes += 1,
                Err(other) => panic!("{other:?}"),
            }
            assert!(goes < 40, "the walk is not getting anywhere");
        }
        assert!(goes >= 6, "sixty-one parts in goes of ten took {goes} goes");
        assert_eq!(parts(&e), parts(&whole));
    }

    /// The memory a long stream costs, which is the point of reading it a part
    /// at a time: the last record of three hundred blocks read with the cache
    /// capped at four blocks' worth, and never more than that held.
    #[test]
    fn the_last_record_of_a_long_stream_is_read_with_at_most_the_cap_unpacked() {
        use crate::formats::bam::tests::{bam_header, bgzf_block, record_bytes, Rec, EOF_BLOCK};
        const BLOCKS: usize = 300;
        const PER_BLOCK: usize = 40;
        let mut file = bgzf_block(&bam_header("@HD\tVN:1.6\n", &[("chr1", 1_000_000)]));
        let (mut unpacked, mut block_bytes) = (0usize, 0usize);
        let mut last_name = String::new();
        for b in 0..BLOCKS {
            let mut data = Vec::new();
            for r in 0..PER_BLOCK {
                last_name = format!("read{b}.{r}");
                let rec = Rec {
                    name: &last_name,
                    ref_id: 0,
                    pos: (b * PER_BLOCK + r) as i32,
                    mapq: 60,
                    flag: 0,
                    cigar: &[(50, b'M')],
                    seq: &"ACGT".repeat(13)[..50],
                    qual: &[30; 50],
                    tags: b"NMC\x01",
                };
                data.extend_from_slice(&record_bytes(&rec));
            }
            block_bytes = block_bytes.max(data.len());
            unpacked += data.len();
            file.extend_from_slice(&bgzf_block(&data));
        }
        file.extend_from_slice(&EOF_BLOCK);
        let d = Document::new(MemSource(file));
        let mut e = Evaluator::new(crate::formats::bgzf());
        let cap = 4 * block_bytes;
        e.spaces.set_cache_cap(cap);
        let records = e.child_named(&d, &JOINED, "records").unwrap().unwrap();
        let n = e.node(&d, &records).unwrap().child_count as usize;
        assert_eq!(n, BLOCKS * PER_BLOCK);
        let name = e.child_named(&d, &[records.clone(), vec![n - 1]].concat(), "read_name").unwrap().unwrap();
        assert_eq!(e.node(&d, &name).unwrap().value, Value::Str(last_name));
        let space = e.node(&d, &records).unwrap().space;
        let peak = stitch_of(&e, space).cache.borrow().peak;
        eprintln!("{unpacked} bytes unpacked across {BLOCKS} blocks of at most {block_bytes}; cap {cap}, most held at once {peak}");
        assert!(peak <= cap, "held {peak} bytes with a cap of {cap}");
        assert!(peak >= block_bytes, "nothing was held at all");
        assert!(unpacked > 50 * cap, "the stream is not long enough for the cap to be what bounds it");
    }

    #[test]
    fn a_byte_of_a_stitched_space_names_its_part_and_offset() {
        let d = Document::new(MemSource(file(20)));
        let mut e = Evaluator::new(paged(record(), false));
        let c = e.node(&d, &[STREAM, 0, 2]).unwrap();
        // `c` starts two bytes before the end of page 3, which is the stream's
        // first part and the last page of the file.
        let hit = e.part_of(&d, c.space, c.offset_bits / 8).unwrap().expect("a byte of the stream");
        assert_eq!((hit.index, hit.parts, hit.label.as_str()), (0, 3, "pages[0]"));
        assert_eq!(hit.path, [PAGES, 0, 0]);
        assert_eq!((hit.in_part, hit.part_len, hit.run_space, hit.run_offset_bits), (6, 8, 0, 24 * 8));
        assert!(!hit.packed);
        assert_eq!(hit.virtual_offset, None, "a page is not a BGZF block");
        // Its third byte is the first of page 1, eight bytes into the file.
        let hit = e.part_of(&d, c.space, 8).unwrap().unwrap();
        assert_eq!((hit.index, hit.label.as_str(), hit.in_part, hit.run_offset_bits), (1, "pages[1]", 0, 8 * 8));
        // Past the length is no part, and so is a space that is not stitched.
        assert_eq!(e.part_of(&d, c.space, 20).unwrap(), None);
        assert_eq!(e.part_of(&d, 0, 0).unwrap(), None);

        // A BGZF block names its byte the way an index does: the block's own
        // place in the file, not its deflate run's, and the byte in what it
        // unpacks to.
        let first = [b'x'; 40];
        let d = Document::new(MemSource(bgzf_of(&[&first, b"the second block"])));
        let mut e = Evaluator::new(crate::formats::bgzf());
        let space = e.node(&d, &JOINED).unwrap().space;
        let hit = e.part_of(&d, space, 45).unwrap().unwrap();
        let block = e.node(&d, &[0, 1]).unwrap().offset_bits / 8;
        assert_eq!((hit.label.as_str(), hit.in_part, hit.packed), ("blocks[1].compressed", 5, true));
        assert_eq!(hit.run_offset_bits / 8, block + 18);
        assert_eq!(hit.virtual_offset, Some(block << 16 | 5));
    }

    #[test]
    fn a_joined_stream_under_the_cap_opens_as_a_document_of_its_own() {
        let d = Document::new(MemSource(file(20)));
        let mut e = Evaluator::new(paged(record(), false));
        let id = e.open_space(&d, 0, &[STREAM]).unwrap().expect("twenty bytes is well under the cap");
        let space = e.space(id).unwrap();
        assert_eq!(space.bytes(), &STREAM_BYTES[..20]);
        assert_eq!((space.parent, space.path.as_slice(), space.codec), (0, &[STREAM][..], crate::codec::Codec::Stored));
        // Pages 3, 1 and 2, the last cut at the length, and each where it is
        // in the file.
        let runs: Vec<_> = space.runs().iter().map(|r| (r.out_bytes.clone(), r.run_offset_bits / 8, r.packed)).collect();
        assert_eq!(runs, [(0..8, 24, false), (8..16, 8, false), (16..20, 16, false)]);
        let trace = space.trace();
        trace.check_tiles().unwrap();
        assert_eq!(trace.members().iter().map(|m| m.out_bytes.clone()).collect::<Vec<_>>(), [0..8, 8..16, 16..20]);
        // Asked again, the same document.
        assert_eq!(e.open_space(&d, 0, &[STREAM]).unwrap(), Some(id));
        // And it reads as what the stream declared it holds.
        assert_eq!(e.tab_node(&d, id, &[1]).unwrap().value, Value::UInt(0xaabbccdd));
    }

    #[test]
    fn blocks_of_one_stream_open_as_one_document() {
        let text = b"one line of a vcf\nand another that the writer cut\nand a third\n";
        let d = Document::new(MemSource(bgzf_of(&[&text[..20], &text[20..41], &text[41..]])));
        let mut e = Evaluator::new(crate::formats::bgzf());
        let id = e.open_space(&d, 0, &[1]).unwrap().expect("the stream opens");
        let space = e.space(id).unwrap();
        assert_eq!(space.bytes(), text);
        // Three blocks and the end block, each a member, each unpacked, and
        // each decoder's own steps kept: a deflate block per member.
        assert_eq!(space.runs().len(), 4);
        assert!(space.runs().iter().all(|r| r.packed));
        let trace = space.trace();
        trace.check_tiles().unwrap();
        assert_eq!(trace.members().len(), 4);
        assert!(trace.blocks().len() >= 3, "{} deflate blocks", trace.blocks().len());
    }

    #[test]
    fn a_byte_of_a_joined_document_maps_to_its_parts_own_step() {
        use crate::codec::StepKind;
        let d = Document::new(MemSource(file(20)));
        let mut e = Evaluator::new(paged(record(), false));
        let id = e.open_space(&d, 0, &[STREAM]).unwrap().unwrap();
        let space = e.space(id).unwrap();
        // Byte 9 is in the second part, page 1, which is eight bytes into the
        // file: one stored step over that page's eight bytes, counted from the
        // page's start.
        let step = space.map_out(9).unwrap().unwrap();
        assert_eq!((step.in_bits.clone(), step.out_bytes.clone(), step.kind), (0..64, 8..16, StepKind::Stored));
        let run = space.run_at(9).unwrap();
        assert_eq!((run.run_space, run.run_offset_bits, run.path.as_slice()), (0, 8 * 8, &[PAGES, 1, 0][..]));
        assert_eq!(e.map_out(id, 9).unwrap(), Some(step));
        assert_eq!(e.run_at(id, 9), Some(run));
        // And back: a bit of page 3, the last page of the file, is the first
        // part; a bit of the header is no part at all.
        let space = e.space(id).unwrap();
        assert_eq!(space.map_in(25 * 8).unwrap().map(|s| (s.in_bits, s.out_bytes)), Some((0..64, 0..8)));
        assert_eq!(space.map_in(4 * 8), Ok(None));

        // A byte of the second BGZF block is a literal of that block's own
        // deflate, at bits counted from where that deflate starts, which is the
        // run `part_of` names too.
        let d = Document::new(MemSource(bgzf_of(&[&[b'x'; 40], b"the second block"])));
        let mut e = Evaluator::new(crate::formats::bgzf());
        let joined = e.node(&d, &JOINED).unwrap().space;
        let hit = e.part_of(&d, joined, 45).unwrap().unwrap();
        let id = e.open_space(&d, 0, &[1]).unwrap().unwrap();
        let space = e.space(id).unwrap();
        let run = space.run_at(45).unwrap();
        assert_eq!((run.run_offset_bits, run.path.clone(), run.packed), (hit.run_offset_bits, hit.path, true));
        let step = space.map_out(45).unwrap().unwrap();
        assert_eq!(step.kind, StepKind::Literal(b'e'));
        assert!(step.out_bytes.contains(&45));
        assert!(step.in_bits.end <= run.run_bits, "{:?} is past the {} bits of the block's deflate", step.in_bits, run.run_bits);
        // The same bit of the file leads back to the same step.
        let back = space.map_in(run.run_offset_bits + step.in_bits.start).unwrap().unwrap();
        assert_eq!(back, step);
        assert_eq!(space.run_holding(run.run_offset_bits + step.in_bits.start), Some(run));
        // The end of the first block's deflate reads bits and makes nothing, so
        // its output is where the second block's starts: the run it was read
        // from is found by the bit, not by that output.
        let first = &space.runs()[0];
        let end = (first.run_offset_bits..first.run_offset_bits + first.run_bits)
            .find(|&bit| space.map_in(bit).unwrap().is_some_and(|s| s.kind == StepKind::EndOfBlock))
            .expect("the first block's deflate ends");
        assert_eq!(space.map_in(end).unwrap().unwrap().out_bytes.start, first.out_bytes.end);
        assert_eq!(space.run_holding(end), Some(first));
        assert_ne!(space.run_at(first.out_bytes.end), Some(first));
    }

    #[test]
    fn a_joined_stream_says_what_cut_it_and_what_measured_its_parts() {
        use crate::eval::Role;
        // A paged stream is cut by `total`, a field beside the pages.
        let d = Document::new(MemSource(file(20)));
        let mut e = Evaluator::new(paged(record(), false));
        let origins = e.origins(&d, &[STREAM, 0]).unwrap();
        let total: Vec<_> = origins.iter().filter(|o| o.role == Role::Length).map(|o| (o.label.as_str(), o.path.as_slice())).collect();
        assert_eq!(total, [("total", &[4][..])], "{origins:?}");

        // A BGZF stream has no total, and each block is measured by its own
        // trailer: the first block's, named with the block in front.
        let d = Document::new(MemSource(bgzf_of(&[b"first block", b"and the second"])));
        let mut e = Evaluator::new(crate::formats::bgzf());
        let origins = e.origins(&d, &JOINED).unwrap();
        let size = e.child_named(&d, &[0, 0], "original_size").unwrap().unwrap();
        let measured: Vec<_> = origins.iter().filter(|o| o.role == Role::Length).map(|o| (o.label.as_str(), o.path.clone(), o.value.as_str())).collect();
        assert_eq!(measured, [("blocks[0].original_size", size, "11")], "{origins:?}");
        // A plain field is a name and not a formula, so the relations panel
        // leaves it to the row above.
        assert!(e.relations(&d, &JOINED).unwrap().iter().all(|r| r.role != Role::Length));
    }

    /// A joined stream past the cap opens as a document all the same when
    /// what it is read from fits: the BGZF blocks' compressed runs are kept,
    /// each part is unpacked when it is read, and one part past the cap a run
    /// may be decoded whole at is read through its index. Every byte reads and
    /// maps to the step it does when the same stream is held whole.
    #[test]
    fn a_joined_stream_past_the_cap_opens_a_part_at_a_time() {
        use crate::codec::lazy::Limits;
        use crate::eval::space::SpaceSource;
        let big: Vec<u8> = (0..4000).flat_map(|i| format!("row {i}, ").into_bytes()).collect::<Vec<u8>>()[..30_000].to_vec();
        let text = [&big[..], b"and a short block after it"].concat();
        let d = Document::new(MemSource(bgzf_of(&[&big, b"and a short block after it"])));
        let mut whole = Evaluator::new(crate::formats::bgzf());
        let held = whole.open_space(&d, 0, &[1]).unwrap().expect("opens whole");
        let held = whole.space(held).unwrap();
        assert_eq!(held.bytes(), text);

        let mut e = Evaluator::new(crate::formats::bgzf());
        e.spaces.set_whole_cap(10_000);
        e.spaces.set_lazy_limits(Limits { whole: 4096, spacing: 2048, checkpoints: 8, span_cache: 8192, ..Limits::default() });
        let id = e.open_space(&d, 0, &[1]).unwrap().expect("opens a part at a time");
        let space = e.space(id).unwrap();
        let SpaceSource::Joined(_) = space.source() else { panic!("held whole") };
        // The first block is past the cap a part may be decoded whole at, so
        // it is read through an index of its run.
        let first = &space.runs()[0];
        assert!(e.spaces.indexed((first.run_space, first.run_offset_bits, first.run_bits), crate::codec::Codec::Deflate).is_some());
        let mut got = vec![0u8; text.len()];
        assert!(space.source().read(0, &mut got).is_ok());
        assert_eq!(got, text);
        assert_eq!(space.runs().len(), held.runs().len());
        for byte in 0..text.len() as u64 {
            assert_eq!(space.map_out(byte), held.map_out(byte), "byte {byte}");
        }
        for run in held.runs() {
            for bit in run.run_offset_bits..run.run_offset_bits + run.run_bits {
                assert_eq!(space.map_in(bit), held.map_in(bit), "bit {bit}");
            }
        }
        // Held to the cap on what it is read from as well: the blocks' runs
        // are about two hundred bytes, which a cap of a hundred refuses.
        let mut e = Evaluator::new(crate::formats::bgzf());
        e.spaces.set_whole_cap(100);
        assert_eq!(e.open_space(&d, 0, &[1]).unwrap(), None);
        assert_eq!(e.open_refusal(&[1]), Some(crate::codec::Refusal::TooLargeCopy));
    }

    /// A part read through an index whose run is missing chunks of the file
    /// answers `Pending` with them, read in the listing's stitched space and
    /// in the joined stream opened as a document, and reads once they are
    /// fed. The part is not kept as a part in trouble, nor its run as
    /// refused.
    #[test]
    fn a_part_whose_run_is_missing_chunks_waits_for_them() {
        use crate::codec::lazy::tests::Lossy;
        use crate::codec::lazy::{decode_or_index_with, Decoded, Limits, ReadError, RunInput};
        use crate::codec::Codec;
        use crate::eval::EvalError;
        let big: Vec<u8> = (0..4000).flat_map(|i| format!("row {i}, ").into_bytes()).collect::<Vec<u8>>()[..30_000].to_vec();
        let text = [&big[..], b"and a short block after it"].concat();
        let file = bgzf_of(&[&big, b"and a short block after it"]);
        let d = Document::new(MemSource(file.clone()));
        let limits = Limits { whole: 4096, spacing: 2048, checkpoints: 8, span_cache: 8192, ..Limits::default() };
        let fresh = || {
            let mut e = Evaluator::new(crate::formats::bgzf());
            e.spaces.set_whole_cap(10_000);
            e.spaces.set_lazy_limits(limits);
            e
        };
        let mut whole = Evaluator::new(crate::formats::bgzf());
        let held = whole.open_space(&d, 0, &[1]).unwrap().expect("opens whole");
        let held = whole.space(held).unwrap();
        let first = held.runs()[0].clone();
        assert_eq!(first.run_space, 0);
        let (at, len) = (first.run_offset_bits / 8, first.run_bits / 8);
        let key = (0, first.run_offset_bits, first.run_bits);
        let Ok(Decoded::Indexed(mut index)) = decode_or_index_with(Codec::Deflate, std::sync::Arc::new(file[at as usize..(at + len) as usize].to_vec()), limits) else {
            panic!("indexed")
        };
        let lossy = Lossy::new(file.clone(), 256);
        index.set_input(RunInput::new(lossy.clone(), at, len));
        let index = std::sync::Arc::new(index);
        let inside = lossy.chunks(at as usize..(at + len) as usize);
        /// Ask `f` until it stops naming chunks, feeding them, each of them
        /// a chunk of the file inside the part's run, and say how many times
        /// it named some.
        fn until_fed(lossy: &Lossy, inside: &[crate::source::Missing], what: &str, mut f: impl FnMut() -> Result<(), Vec<crate::source::Missing>>) -> usize {
            for waits in 0..50 {
                match f() {
                    Ok(()) => return waits,
                    Err(missing) => {
                        assert!(!missing.is_empty() && missing.iter().all(|m| inside.contains(m)), "{what}: {missing:?}");
                        lossy.feed(&missing);
                    }
                }
            }
            panic!("{what}: waited 50 times")
        }
        let pending = |r: Result<Vec<u8>, EvalError>, got: &mut Vec<u8>| match r {
            Ok(bytes) => {
                *got = bytes;
                Ok(())
            }
            Err(EvalError::Pending(missing)) => Err(missing),
            Err(other) => panic!("{other:?}"),
        };

        // Read where the stream is declared, a part at a time.
        let mut e = fresh();
        e.spaces.keep_index(key, Codec::Deflate, index.clone());
        e.node(&d, &[1]).unwrap();
        let Some(super::Opened::Space(space)) = e.spaces.get(&[1]) else { panic!("the stream did not open") };
        let mut got = Vec::new();
        let waits = until_fed(&lossy, &inside, "the stitched space", || pending(e.read_in(&d, space, 20_000 * 8, 64 * 8), &mut got));
        assert_eq!((got.as_slice(), waits > 0), (&text[20_000..20_064], true));
        assert!(stitch_of(&e, space).troubles.borrow().is_empty());
        assert_eq!(e.spaces.run_refusal(key, Codec::Deflate), None);

        // Opened as a document of its own, past the cap a joined stream is
        // held whole at.
        lossy.drop_all();
        let mut e = fresh();
        e.spaces.keep_index(key, Codec::Deflate, index.clone());
        // A stream whose template says only bytes has its first bytes looked
        // at to see what they are, which would wait too.
        let mut opened = None;
        until_fed(&lossy, &inside, "opening it", || match e.open_space(&d, 0, &[1]) {
            Ok(id) => {
                opened = id;
                Ok(())
            }
            Err(EvalError::Pending(missing)) => Err(missing),
            Err(other) => panic!("{other:?}"),
        });
        let id = opened.expect("opens a part at a time");
        let space = e.space(id).unwrap();
        lossy.drop_all();
        let mut got = vec![0u8; text.len()];
        let waits = until_fed(&lossy, &inside, "the joined stream", || match space.source().read(0, &mut got) {
            Ok(()) => Ok(()),
            Err(ReadError::Missing(missing)) => Err(missing),
            Err(ReadError::Refused(why)) => panic!("{why:?}"),
        });
        assert!(got == text && waits > 0);
        for byte in [0, 12_345, 29_999, 30_010] {
            lossy.drop_all();
            let mut step = None;
            until_fed(&lossy, &inside, "a byte's step", || space.map_out(byte).map(|s| step = s));
            assert_eq!(step, held.map_out(byte).unwrap(), "byte {byte}");
        }
        for bit in (first.run_offset_bits..first.run_offset_bits + first.run_bits).step_by(4099) {
            lossy.drop_all();
            let mut step = None;
            until_fed(&lossy, &inside, "a bit's step", || space.map_in(bit).map(|s| step = s));
            assert_eq!(step, held.map_in(bit).unwrap(), "bit {bit}");
        }
        assert_eq!(e.spaces.run_refusal(key, Codec::Deflate), None);
    }

    /// Four runs, each as many bytes as its entry in `lens` says, and a stream
    /// joined from them in the order they are declared: bytes stored as they
    /// are, a deflate run, a run whose codec copies (as a ZIP declares a stored
    /// entry), and another deflate run. A BP5 dataset in a ZIP can join
    /// stored and deflated entries too, but its template is built from the
    /// archive, and this one puts each kind of part where a test wants it.
    fn mixed() -> Template {
        use crate::codec::Codec;
        let len = |i: i128| E::elem("lens", E::lit(i));
        let runs = T::structure(
            "Runs",
            vec![
                ("head", T::bytes(len(0))),
                ("big", T::decoded(len(1), Codec::Deflate, T::bytes(E::Remaining))),
                ("kept", T::decoded(len(2), Codec::Stored, T::bytes(E::Remaining))),
                ("small", T::decoded(len(3), Codec::Deflate, T::bytes(E::Remaining))),
            ],
        );
        let parts = vec![Step::field("runs"), Step::fields(&["head", "big", "kept", "small"])];
        Template::new(
            "mixed",
            T::structure(
                "Mixed",
                vec![("lens", T::array(T::u16(Big), E::lit(4))), ("runs", runs), ("joined", T::stitched(parts, None, None, T::bytes(E::Remaining)))],
            ),
        )
    }

    /// A joined stream of stored and packed parts past the cap opens a part at
    /// a time: the stored parts' bytes are kept, one packed part is decoded
    /// whole when it is read and the other is read through its index. Every
    /// byte reads, and every byte and every bit of the file maps to the step
    /// it does when the same stream is held whole.
    #[test]
    fn a_joined_stream_of_stored_and_packed_parts_past_the_cap_opens_a_part_at_a_time() {
        use crate::codec::lazy::Limits;
        use crate::eval::space::SpaceSource;
        let head: Vec<u8> = (0..100).collect();
        let big: Vec<u8> = (0..4000).flat_map(|i| format!("row {i}, ").into_bytes()).collect::<Vec<u8>>()[..30_000].to_vec();
        let kept: Vec<u8> = (0..100).map(|i| 255 - i).collect();
        let small: Vec<u8> = (0..100).flat_map(|i| format!("{i:03} ").into_bytes()).collect();
        let packed = |data: &[u8]| miniz_oxide::deflate::compress_to_vec(data, 6);
        let runs = [head.clone(), packed(&big), kept.clone(), packed(&small)];
        let mut file: Vec<u8> = runs.iter().flat_map(|r| (r.len() as u16).to_be_bytes()).collect();
        for run in &runs {
            file.extend_from_slice(run);
        }
        let text = [head, big, kept, small].concat();
        let d = Document::new(MemSource(file.clone()));
        const MIXED: usize = 2;

        let mut whole = Evaluator::new(mixed());
        let held = whole.open_space(&d, 0, &[MIXED]).unwrap().expect("opens whole");
        let held = whole.space(held).unwrap();
        assert_eq!(held.bytes(), text);
        assert_eq!(held.runs().iter().map(|r| r.packed).collect::<Vec<_>>(), [false, true, false, true]);

        let mut e = Evaluator::new(mixed());
        // Under the cap: the stored parts and the two compressed runs come to
        // about 7,600 bytes, and the stream to 30,600.
        e.spaces.set_whole_cap(10_000);
        e.spaces.set_lazy_limits(Limits { whole: 4096, spacing: 2048, checkpoints: 8, span_cache: 8192, ..Limits::default() });
        let id = e.open_space(&d, 0, &[MIXED]).unwrap().expect("opens a part at a time");
        let space = e.space(id).unwrap();
        let SpaceSource::Joined(_) = space.source() else { panic!("held whole") };
        assert_eq!(space.runs(), held.runs().iter().map(|r| super::JoinedRun { in_start: 0, ..r.clone() }).collect::<Vec<_>>());
        // `big` is past the cap a part may be decoded whole at, and `small` is
        // not.
        let indexed = |r: &super::JoinedRun| e.spaces.indexed((r.run_space, r.run_offset_bits, r.run_bits), crate::codec::Codec::Deflate).is_some();
        assert_eq!(space.runs().iter().map(indexed).collect::<Vec<_>>(), [false, true, false, false]);
        let mut got = vec![0u8; text.len()];
        assert!(space.source().read(0, &mut got).is_ok());
        assert_eq!(got, text);
        // Across each edge between parts, read on its own.
        for r in &space.runs()[1..] {
            let at = r.out_bytes.start - 3;
            let mut got = [0u8; 6];
            assert!(space.source().read(at, &mut got).is_ok());
            assert_eq!(got, text[at as usize..at as usize + 6], "at {at}");
        }
        for byte in 0..text.len() as u64 {
            assert_eq!(space.map_out(byte), held.map_out(byte), "byte {byte}");
        }
        for bit in 0..file.len() as u64 * 8 {
            assert_eq!(space.map_in(bit), held.map_in(bit), "bit {bit}");
        }
    }

    #[test]
    fn a_joined_stream_over_the_cap_is_refused_as_a_document_and_still_reads() {
        let d = Document::new(MemSource(file(20)));
        let mut e = Evaluator::new(paged(record(), false));
        e.spaces.set_whole_cap(19);
        assert_eq!(e.open_space(&d, 0, &[STREAM]).unwrap(), None);
        assert_eq!(e.open_refusal(&[STREAM]), Some(crate::codec::Refusal::TooLargeCopy));
        // Where it is declared, it reads a part at a time as it did, and its
        // node says nothing of the refusal.
        assert_eq!(e.node(&d, &[STREAM]).unwrap().refused, None);
        assert_eq!(e.node(&d, &[STREAM, 0, 1]).unwrap().value, Value::UInt(0xaabbccdd));
        // At the cap exactly, it opens.
        let mut e = Evaluator::new(paged(record(), false));
        e.spaces.set_whole_cap(20);
        assert!(e.open_space(&d, 0, &[STREAM]).unwrap().is_some());
        assert_eq!(e.open_refusal(&[STREAM]), None);
    }

    /// Held whole, a joined stream is unpacked part by part before it opens,
    /// so a part that will not unpack refuses it then.
    #[test]
    fn a_joined_stream_with_a_part_that_will_not_unpack_is_refused_as_a_document() {
        let d = Document::new(MemSource(bam_in_blocks(|b| b[18] = 0xff)));
        let mut e = Evaluator::new(crate::formats::bgzf());
        assert_eq!(e.open_space(&d, 0, &[1]).unwrap(), None);
        assert_eq!(e.open_refusal(&[1]), Some(crate::codec::Refusal::Failed));
    }

    /// Read a part at a time, past the cap, a joined stream opens without
    /// unpacking a part, so a part that will not unpack, or comes to another
    /// length than its trailer says, does not refuse it. A read of the
    /// stream that reaches the part is refused instead, gives the other
    /// blocks' bytes and zeroes for that one, and the stream says why in the
    /// words a read of the part in the listing has.
    #[test]
    fn a_joined_stream_past_the_cap_with_a_part_that_will_not_unpack_refuses_reads_of_that_part() {
        use crate::codec::lazy::ReadError;
        use crate::codec::Refusal;
        use crate::source::ArcSource;
        let (odd, even) = crate::formats::bam::tests::two_records();
        let got = odd.len() + even.len();
        let long = |b: &mut Vec<u8>| {
            let at = b.len() - 4;
            b[at..].copy_from_slice(&(got as u32 + 5).to_le_bytes());
        };
        let good = bam_in_blocks(|_| {});
        let mut whole = Evaluator::new(crate::formats::bgzf());
        let held = whole.open_space(&Document::new(MemSource(good)), 0, &[1]).unwrap().expect("opens whole");
        let text = whole.space(held).unwrap().bytes().to_vec();
        for (file, claimed, said) in [
            (bam_in_blocks(|b| b[18] = 0xff), got, "blocks[2].compressed could not be unpacked".to_string()),
            (bam_in_blocks(long), got + 5, format!("blocks[2].compressed unpacks to {got} bytes, not the {} the file says", got + 5)),
        ] {
            let d = Document::new(ArcSource(std::sync::Arc::new(file)));
            let mut e = Evaluator::new(crate::formats::bgzf());
            e.spaces.set_whole_cap(10);
            let id = e.open_space(&d, 0, &[1]).unwrap().unwrap_or_else(|| panic!("{said}: refused as {:?}", e.open_refusal(&[1])));
            let source = e.space(id).unwrap().source().clone();
            let start = text.len() - got;
            let mut out = vec![0xee; start + claimed];
            assert_eq!(source.read(0, &mut out), Err(ReadError::Refused(Refusal::Failed)), "{said}");
            assert!(out[..start] == text[..start] && out[start..].iter().all(|&b| b == 0), "{said}");
            let why = e.unreadable(id, 0, out.len() as u64).expect("says why");
            assert_eq!((why.at, why.len, why.why), (start as u64, claimed as u64, said));
        }
    }

    /// A joined stream past the cap with a part that unpacks to more than
    /// `Limits::lazy` is refused as too large for that, and a read of the part
    /// says so. A part that large which will not unpack at all is refused as
    /// failed, as before.
    #[test]
    fn a_joined_stream_with_a_part_past_the_lazy_cap_is_refused_as_too_large() {
        use crate::codec::lazy::Limits;
        use crate::codec::Refusal;
        use crate::eval::EvalError;
        let big: Vec<u8> = (0..4000).flat_map(|i| format!("row {i}, ").into_bytes()).collect::<Vec<u8>>()[..30_000].to_vec();
        let limits = Limits { whole: 4096, lazy: 20_000, spacing: 2048, checkpoints: 8, span_cache: 8192, ..Limits::default() };
        let open = |file: Vec<u8>| {
            let d = Document::new(MemSource(file));
            let mut e = Evaluator::new(crate::formats::bgzf());
            e.spaces.set_whole_cap(10_000);
            e.spaces.set_lazy_limits(limits);
            let opened = e.open_space(&d, 0, &[1]).unwrap();
            let space = match e.spaces.get(&[1]) {
                Some(super::Opened::Space(id)) => id,
                other => panic!("the stream did not open: {other:?}"),
            };
            (d, e, opened, space)
        };

        let (d, e, opened, space) = open(bgzf_of(&[&big, b"and a short block after it"]));
        assert_eq!(opened, None);
        assert_eq!(e.open_refusal(&[1]), Some(Refusal::TooLargeLazy));
        // Where it is declared, the first block's bytes say why they are not
        // there, and the block after it still reads.
        let why = |e: &Evaluator, d: &Document<MemSource>, space| match e.read_in(d, space, 0, 8) {
            Err(EvalError::Failed(why)) => why,
            other => panic!("{other:?}"),
        };
        assert_eq!(why(&e, &d, space), "blocks[0].compressed is too large to unpack (over 1 GiB unpacked)");
        assert_eq!(e.read_in(&d, space, 30_000 * 8, 24).unwrap(), b"and");

        // The same first block, its deflate made a block type that does not
        // exist.
        let mut file = bgzf_of(&[&big, b"and a short block after it"]);
        file[18] = 0xff;
        let (d, e, opened, space) = open(file);
        assert_eq!(opened, None);
        assert_eq!(e.open_refusal(&[1]), Some(Refusal::Failed));
        assert_eq!(why(&e, &d, space), "blocks[0].compressed could not be unpacked");
    }

    /// A part refused by its index pass is refused from then on without
    /// another pass: by each read of it, and by the block's own compressed run
    /// opened as a stream of its own. Dropped with the indexes, so a file
    /// read again after an edit is indexed again.
    #[test]
    fn a_part_refused_by_its_index_pass_is_not_indexed_again() {
        use crate::codec::lazy::Limits;
        use crate::codec::{Codec, Refusal};
        let big: Vec<u8> = (0..4000).flat_map(|i| format!("row {i}, ").into_bytes()).collect::<Vec<u8>>()[..30_000].to_vec();
        let limits = Limits { whole: 4096, lazy: 20_000, spacing: 2048, checkpoints: 8, span_cache: 8192, ..Limits::default() };
        // Past `Limits::lazy`, and then the same block made a deflate block
        // type that does not exist.
        for (spoil, refusal) in [(false, Refusal::TooLargeLazy), (true, Refusal::Failed)] {
            let mut file = bgzf_of(&[&big, b"and a short block after it"]);
            if spoil {
                file[18] = 0xff;
            }
            let d = Document::new(MemSource(file));
            let mut e = Evaluator::new(crate::formats::bgzf());
            e.spaces.set_whole_cap(10_000);
            e.spaces.set_lazy_limits(limits);
            assert_eq!(e.open_space(&d, 0, &[1]).unwrap(), None);
            let Some(super::Opened::Space(space)) = e.spaces.get(&[1]) else { panic!("the stream did not open") };
            assert_eq!(e.spaces.passes.get(), 1, "opening it as a document indexed the first block");
            for _ in 0..2 {
                assert!(e.read_in(&d, space, 0, 8).is_err());
            }
            assert_eq!(e.spaces.passes.get(), 1, "two reads of the first block");
            let compressed = e.child_named(&d, &[0, 0], "compressed").unwrap().expect("a block has its compressed run");
            assert_eq!(e.open_space(&d, 0, &compressed).unwrap(), None);
            assert_eq!(e.open_refusal(&compressed), Some(refusal));
            assert_eq!(e.spaces.passes.get(), 1, "the block's own run, opened");
            // The block after it is under `Limits::whole` and needs no pass.
            assert_eq!(e.read_in(&d, space, 30_000 * 8, 24).unwrap(), b"and");
            assert_eq!(e.spaces.passes.get(), 1);

            let super::PartSource::Packed { space: run_space, at_bits, size_bits, .. } = e.spaces.stitch(space).unwrap().parts[0].source else {
                panic!("the first block is packed")
            };
            let key = (run_space, at_bits, size_bits);
            assert_eq!(e.spaces.run_refusal(key, Codec::Deflate), Some(refusal));
            e.spaces.forget();
            assert_eq!(e.spaces.run_refusal(key, Codec::Deflate), None);
            assert_eq!(e.open_space(&d, 0, &[1]).unwrap(), None);
            assert_eq!(e.spaces.passes.get(), 2, "after the indexes were dropped");
        }
    }

    /// A packed part small enough to decode whole that will not decode, or
    /// that decodes to another length than its block's trailer says, is
    /// decoded by the first read that reaches it and not by the next, which
    /// fails the same way. Dropped with the spaces, so a stream opened again
    /// after they are forgotten decodes the part again.
    #[test]
    fn a_part_that_fails_to_decode_is_decoded_once_however_often_it_is_read() {
        use crate::eval::EvalError;
        use crate::formats::bam::tests::bgzf_block;
        let (first, second) = (b"the first block's text".as_slice(), b"and the second block's".as_slice());
        let at = bgzf_block(first).len();
        let end = at + bgzf_block(second).len();
        let spoilt = |spoil: &dyn Fn(&mut Vec<u8>)| {
            let mut file = bgzf_of(&[first, second]);
            spoil(&mut file);
            file
        };
        let (damaged, long) = (
            spoilt(&|f| f[at + 18] = 0xff),
            spoilt(&|f| {
                let size = u32::from_le_bytes(f[end - 4..end].try_into().unwrap()) + 5;
                f[end - 4..end].copy_from_slice(&size.to_le_bytes());
            }),
        );
        let n = second.len();
        for (file, said) in [
            (damaged, "blocks[1].compressed could not be unpacked".to_string()),
            (long, format!("blocks[1].compressed unpacks to {n} bytes, not the {} the file says", n + 5)),
        ] {
            let d = Document::new(MemSource(file));
            let mut e = Evaluator::new(crate::formats::bgzf());
            let read = |e: &mut Evaluator| {
                let super::Opened::Space(space) = e.open_stitched_at(&d, &[1]).unwrap() else { panic!("the stream did not open") };
                let before = e.spaces.decodes.get();
                for _ in 0..2 {
                    match e.read_in(&d, space, first.len() as u64 * 8, 8) {
                        Err(EvalError::Failed(why)) => assert_eq!(why, said),
                        other => panic!("{other:?}"),
                    }
                }
                e.spaces.decodes.get() - before
            };
            assert_eq!(read(&mut e), 1, "two reads of the second block");
            e.spaces.forget();
            assert_eq!(read(&mut e), 1, "after the spaces were forgotten");
        }
    }

    /// An xz part the file says is past the cap a part may be decoded whole at,
    /// which decodes whole all the same, is refused for its length by the
    /// first read, and the second read says the same without another pass.
    #[test]
    fn a_part_shorter_than_the_file_says_is_not_decoded_again_to_say_so() {
        use crate::codec::lazy::Limits;
        use crate::codec::Codec;
        use crate::eval::EvalError;
        let text: Vec<u8> = (0..200).flat_map(|i| format!("line {i}\n").into_bytes()).collect();
        let mut packed = Vec::new();
        lzma_rs::xz_compress(&mut &text[..], &mut packed).expect("packs");
        let claimed: u32 = 10_000;
        let mut file = claimed.to_be_bytes().to_vec();
        file.extend_from_slice(&packed);
        let member = T::structure("Member", vec![("claimed", T::u32(Big)), ("run", T::decoded(E::Remaining, Codec::Xz, T::bytes(E::Remaining)))]);
        let joined = T::stitched(vec![Step::field("member"), Step::field("run")], Some(E::field("claimed")), None, T::bytes(E::Remaining));
        let template = Template::new("xz-part", T::structure("File", vec![("member", member), ("joined", joined)]));
        let d = Document::new(MemSource(file));
        let mut e = Evaluator::new(template);
        e.spaces.set_lazy_limits(Limits { whole: 4096, spacing: 2048, checkpoints: 8, span_cache: 8192, ..Limits::default() });
        let super::Opened::Space(space) = e.open_stitched_at(&d, &[1]).unwrap() else { panic!("the stream did not open") };
        assert_eq!(e.spaces.len_bits(space), u64::from(claimed) * 8);
        let before = e.spaces.passes.get();
        for _ in 0..2 {
            match e.read_in(&d, space, 0, 8) {
                Err(EvalError::Failed(why)) => assert_eq!(why, format!("member.run unpacks to {} bytes, not the {claimed} the file says", text.len())),
                other => panic!("{other:?}"),
            }
            assert_eq!(e.spaces.passes.get(), before + 1);
        }
    }

    /// A packed part whose codec has no index, and which comes to more than a
    /// run may be decoded whole at, says so when it is read rather than that
    /// it could not be unpacked.
    #[test]
    fn a_part_too_large_to_unpack_whole_says_it_is_too_large() {
        use crate::codec::Codec;
        use crate::eval::EvalError;
        // A Snappy block starts with how many bytes it comes to, as a varint,
        // and one that says more than 64 MiB is refused before anything is
        // decoded.
        let claimed: u32 = 100 << 20;
        let mut file = claimed.to_be_bytes().to_vec();
        let mut n = claimed;
        while n >= 0x80 {
            file.push(n as u8 | 0x80);
            n >>= 7;
        }
        file.push(n as u8);
        file.extend_from_slice(&[0x08, b'a', b'b', b'c']);
        let member = T::structure(
            "Member",
            vec![("claimed", T::u32(Big)), ("run", T::decoded(E::Remaining, Codec::Snappy, T::bytes(E::Remaining)))],
        );
        let joined = T::stitched(vec![Step::field("member"), Step::field("run")], Some(E::field("claimed")), None, T::bytes(E::Remaining));
        let template = Template::new("snappy-part", T::structure("File", vec![("member", member), ("joined", joined)]));
        let d = Document::new(MemSource(file));
        let mut e = Evaluator::new(template);
        let super::Opened::Space(space) = e.open_stitched_at(&d, &[1]).unwrap() else { panic!("the stream did not open") };
        assert_eq!(e.spaces.len_bits(space), u64::from(claimed) * 8);
        match e.read_in(&d, space, 0, 8) {
            Err(EvalError::Failed(why)) => assert_eq!(why, "member.run is too large to unpack (over 64 MiB unpacked)"),
            other => panic!("{other:?}"),
        }
    }

    /// A packed part of one zstd frame past the span limit refuses the joined
    /// stream for that, and a read of the part says it has such a frame.
    #[test]
    fn a_part_with_a_zstd_frame_too_large_says_so() {
        use crate::codec::lazy::Limits;
        use crate::codec::{Codec, Refusal, SpanUnit};
        use crate::eval::EvalError;
        let text: Vec<u8> = (0..40_000).flat_map(|i| format!("row {i}, ").into_bytes()).collect::<Vec<u8>>()[..300_000].to_vec();
        let mut file = (text.len() as u32).to_be_bytes().to_vec();
        file.extend(ruzstd::encoding::compress_to_vec(&text[..], ruzstd::encoding::CompressionLevel::Fastest));
        let member = T::structure("Member", vec![("claimed", T::u32(Big)), ("run", T::decoded(E::Remaining, Codec::Zstd, T::bytes(E::Remaining)))]);
        let joined = T::stitched(vec![Step::field("member"), Step::field("run")], Some(E::field("claimed")), None, T::bytes(E::Remaining));
        let template = Template::new("zstd-part", T::structure("File", vec![("member", member), ("joined", joined)]));
        let d = Document::new(MemSource(file));
        let mut e = Evaluator::new(template);
        // The stream is past the cap for one held whole, and the packed run
        // is under it.
        e.spaces.set_whole_cap(100_000);
        e.spaces.set_lazy_limits(Limits { whole: 4096, span: 65_536, spacing: 2048, checkpoints: 8, span_cache: 8192, ..Limits::default() });
        assert_eq!(e.open_space(&d, 0, &[1]).unwrap(), None);
        assert_eq!(e.open_refusal(&[1]), Some(Refusal::TooLargeUnit(SpanUnit::ZstdFrame)));
        let Some(super::Opened::Space(space)) = e.spaces.get(&[1]) else { panic!("the stream did not open") };
        match e.read_in(&d, space, 0, 8) {
            Err(EvalError::Failed(why)) => assert_eq!(why, "member.run has a zstd frame that is too large to unpack (over 64 MiB unpacked)"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_edit_to_the_file_closes_a_stitched_space() {
        let mut d = Document::new(MemSource(file(20)));
        let mut e = Evaluator::new(paged(record(), false));
        assert_eq!(e.node(&d, &[STREAM, 0, 2]).unwrap().value, Value::UInt(0x0102_0304));
        // The two bytes of `c` that are in page 1, which is before page 3 in
        // the file: an edit there is before where the stream's first part is.
        d.overwrite_bytes(8, &[0x33, 0x44]);
        e.invalidate_from(8 * 8);
        assert_eq!(e.node(&d, &[STREAM, 0, 2]).unwrap().value, Value::UInt(0x0102_3344));
    }
}
