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

use std::sync::Arc;

use super::gather::Landing;
use super::space::{JoinedRun, Opened, Part, PartSource, Stitch};
use super::*;
use crate::codec::{Codec, StepKind, Trace, TraceBuilder};

/// A stitched stream held whole: its bytes, the trace of every part end to
/// end, and where each part's run is. See [`Evaluator::join_whole`].
pub(super) struct Whole {
    pub(super) bytes: Vec<u8>,
    pub(super) trace: Trace,
    pub(super) runs: Vec<JoinedRun>,
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
        if !matches!(r.ty, Ty::Decoded { .. }) {
            // As long as the run, or as long as the format says the part is
            // where that is less: a claim of more than is there cannot be
            // read, and the bytes that are there still can.
            let len = claimed.map_or(size / 8, |n| n.min(size / 8));
            let source = PartSource::Stored { space: r.space, at_bits: r.offset };
            return Ok(Reached::Part(Part { start, len, path, source }));
        }
        let codec = match self.codec_at(doc, landing)? {
            Some(codec) => Ok(codec),
            None => Err(Refusal::Settings),
        };
        let len = match (claimed, codec) {
            (Some(n), _) => n,
            // Nothing says how long it comes to, so it is unpacked to find
            // out. Not kept: a read that wants it unpacks it again, which
            // costs one more inflate of one part and keeps the walk from
            // holding every part it measured.
            (None, Ok(codec)) => match self.unpack_run(doc, r.space, r.offset, size, codec)? {
                Ok(bytes) => bytes.len() as u64,
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

    /// The bytes of a packed run, unpacked. The outer error is the run's bytes
    /// not being there yet; the inner one is the decoder refusing them.
    fn unpack_run<S: Source>(&self, doc: &Document<S>, space: u32, at: u64, size: u64, codec: Codec) -> R<Result<Vec<u8>, Refusal>> {
        if size / 8 > crate::codec::CAP_BYTES as u64 {
            return Ok(Err(Refusal::TooLarge));
        }
        let packed = self.read_in(doc, space, at, size)?;
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
        Ok(Some(Place { name: pr.name.clone(), ty: inner.clone(), offset: 0, limit, space }))
    }

    /// `n` bits of a stitched space from bit `at`, from as many parts as
    /// they cover.
    pub(super) fn read_stitched<S: Source>(&self, doc: &Document<S>, stitch: &Stitch, at: u64, n: u64) -> R<Vec<u8>> {
        if at + n > stitch.len_bytes * 8 {
            return fail("runs past the end of the joined stream");
        }
        let mut buf = vec![0u8; crate::bits::bytes_for(n)];
        if n == 0 {
            return Ok(buf);
        }
        let (first, last) = (at / 8, (at + n).div_ceil(8));
        let mut bytes = Vec::with_capacity((last - first) as usize);
        let mut pos = first;
        while pos < last {
            let Some(i) = stitch.part_at(pos) else { return fail("runs past the end of the joined stream") };
            let part = &stitch.parts[i];
            let take = (part.start + part.len).min(last) - pos;
            let from = pos - part.start;
            match &part.source {
                PartSource::Stored { space, at_bits } => {
                    bytes.extend_from_slice(&self.read_in(doc, *space, at_bits + from * 8, take * 8)?);
                }
                PartSource::Packed { .. } => {
                    let unpacked = self.unpacked_part(doc, stitch, i)?;
                    bytes.extend_from_slice(&unpacked[from as usize..(from + take) as usize]);
                }
            }
            pos += take;
        }
        crate::bits::copy_bits(&bytes, at % 8, &mut buf, 0, n);
        Ok(buf)
    }

    /// Everything packed part `i` unpacks to: from the stream already open over
    /// its run when something opened that, from the cache when it was read
    /// lately, and unpacked now otherwise.
    fn unpacked_part<S: Source>(&self, doc: &Document<S>, stitch: &Stitch, i: usize) -> R<Arc<Vec<u8>>> {
        let part = &stitch.parts[i];
        let PartSource::Packed { space, at_bits, size_bits, codec } = &part.source else {
            return fail("not a packed part");
        };
        // Both of these are shown over the bytes a run of records could not
        // read, so they name the part the way the listing names the field it
        // is, and say what the file claimed in the file's own voice.
        let checked = |bytes: Arc<Vec<u8>>| -> R<Arc<Vec<u8>>> {
            if bytes.len() as u64 != part.len {
                return fail(format!(
                    "{} unpacks to {} bytes, not the {} the file says",
                    self.part_label(stitch, i),
                    bytes.len(),
                    part.len
                ));
            }
            Ok(bytes)
        };
        if let Some(Opened::Space(id)) = self.spaces.get(&part.path) {
            if let Some(bytes) = self.spaces.buf(id) {
                return checked(bytes.clone());
            }
        }
        if let Some(bytes) = stitch.cache.borrow_mut().get(i) {
            return Ok(bytes);
        }
        let unpacked = match codec {
            Ok(codec) => self.unpack_run(doc, *space, *at_bits, *size_bits, *codec)?,
            Err(why) => Err(*why),
        };
        let bytes = match unpacked {
            Ok(bytes) => checked(Arc::new(bytes))?,
            Err(_) => return fail(format!("{} could not be unpacked", self.part_label(stitch, i))),
        };
        stitch.cache.borrow_mut().put(i, bytes.clone());
        Ok(bytes)
    }

    /// The stitched stream at `path` held whole, for opening it as a document
    /// of its own: its bytes, a trace of every part, and where each part's run
    /// is. Refused when the stream would not open, when it comes to more than
    /// a decoded stream may (the same cap, and the same refusal), and when a
    /// part will not read.
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
    pub(super) fn join_whole<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Result<Whole, Refusal>> {
        let id = match self.open_stitched_at(doc, path)? {
            Opened::Space(id) => id,
            Opened::Refused(why) => return Ok(Err(why)),
        };
        let Some(stitch) = self.spaces.stitch(id) else { return fail("this stream is no longer open") };
        if stitch.len_bytes > self.spaces.whole_cap() as u64 {
            return Ok(Err(Refusal::TooLarge));
        }
        // A read that has to wait for bytes waits; one that fails is a part
        // that will not read, which refuses the whole as a stream that will
        // not unpack refuses.
        macro_rules! read {
            ($space:expr, $at:expr, $bits:expr) => {
                match self.read_in(doc, $space, $at, $bits) {
                    Ok(bytes) => bytes,
                    Err(e) if e.interrupted() => return Err(e),
                    Err(_) => return Ok(Err(Refusal::Failed)),
                }
            };
        }
        let mut bytes = Vec::with_capacity(stitch.len_bytes as usize);
        let mut trace = TraceBuilder::default();
        let mut runs = Vec::with_capacity(stitch.parts.len());
        let mut axis = 0u64;
        for part in &stitch.parts {
            let (space, at_bits, run_bits, packed) = match &part.source {
                PartSource::Stored { space, at_bits } => (*space, *at_bits, part.len * 8, false),
                PartSource::Packed { space, at_bits, size_bits, .. } => (*space, *at_bits, *size_bits, true),
            };
            // A trace keeps each step's start in 32 bits, which is half a
            // gigabyte of runs laid end to end.
            if axis + run_bits > u64::from(u32::MAX) {
                return Ok(Err(Refusal::TooLarge));
            }
            let out = part.start;
            match &part.source {
                PartSource::Stored { .. } => {
                    bytes.extend_from_slice(&read!(space, at_bits, run_bits));
                    if part.len > 0 {
                        trace.push(axis, out, StepKind::Stored);
                    }
                }
                PartSource::Packed { codec, .. } => {
                    let codec = match codec {
                        Ok(codec) => *codec,
                        Err(why) => return Ok(Err(*why)),
                    };
                    if run_bits / 8 > crate::codec::CAP_BYTES as u64 {
                        return Ok(Err(Refusal::TooLarge));
                    }
                    let packed = read!(space, at_bits, run_bits);
                    let Ok((unpacked, t)) = crate::codec::decode_traced(codec, &packed) else {
                        return Ok(Err(Refusal::Failed));
                    };
                    // The same test a read of the part makes: a part that
                    // unpacks to another length than its run claimed is not
                    // the part the stream was measured with.
                    if unpacked.len() as u64 != part.len {
                        return Ok(Err(Refusal::Failed));
                    }
                    bytes.extend_from_slice(&unpacked);
                    // A trace too long to keep names this part as one step,
                    // the way a decoder past the same ceiling names a block.
                    if trace.steps() + t.len() > crate::codec::MAX_STEPS {
                        trace.push(axis, out, StepKind::Block);
                        trace.coarsen();
                    } else {
                        trace.absorb_part(&t, axis, out);
                    }
                }
            }
            trace.member(axis..axis + run_bits, out..out + part.len);
            runs.push(JoinedRun {
                out_bytes: out..out + part.len,
                path: part.path.clone(),
                run_space: space,
                run_offset_bits: at_bits,
                run_bits,
                packed,
                in_start: axis,
            });
            axis += run_bits;
        }
        trace.finish_at(axis, stitch.len_bytes);
        Ok(Ok(Whole { bytes, trace: trace.done(), runs }))
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
        if label.is_empty() { format!("part index {i}") } else { label }
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
        let step = space.map_out(9).unwrap();
        assert_eq!((step.in_bits.clone(), step.out_bytes.clone(), step.kind), (0..64, 8..16, StepKind::Stored));
        let run = space.run_at(9).unwrap();
        assert_eq!((run.run_space, run.run_offset_bits, run.path.as_slice()), (0, 8 * 8, &[PAGES, 1, 0][..]));
        assert_eq!(e.map_out(id, 9), Some(step));
        assert_eq!(e.run_at(id, 9), Some(run));
        // And back: a bit of page 3, the last page of the file, is the first
        // part; a bit of the header is no part at all.
        let space = e.space(id).unwrap();
        assert_eq!(space.map_in(25 * 8).map(|s| (s.in_bits, s.out_bytes)), Some((0..64, 0..8)));
        assert_eq!(space.map_in(4 * 8), None);

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
        let step = space.map_out(45).unwrap();
        assert_eq!(step.kind, StepKind::Literal(b'e'));
        assert!(step.out_bytes.contains(&45));
        assert!(step.in_bits.end <= run.run_bits, "{:?} is past the {} bits of the block's deflate", step.in_bits, run.run_bits);
        // The same bit of the file leads back to the same step.
        let back = space.map_in(run.run_offset_bits + step.in_bits.start).unwrap();
        assert_eq!(back, step);
        assert_eq!(space.run_holding(run.run_offset_bits + step.in_bits.start), Some(run));
        // The end of the first block's deflate reads bits and makes nothing, so
        // its output is where the second block's starts: the run it was read
        // from is found by the bit, not by that output.
        let first = &space.runs()[0];
        let end = (first.run_offset_bits..first.run_offset_bits + first.run_bits)
            .find(|&bit| space.map_in(bit).is_some_and(|s| s.kind == StepKind::EndOfBlock))
            .expect("the first block's deflate ends");
        assert_eq!(space.map_in(end).unwrap().out_bytes.start, first.out_bytes.end);
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

    #[test]
    fn a_joined_stream_over_the_cap_is_refused_as_a_document_and_still_reads() {
        let d = Document::new(MemSource(file(20)));
        let mut e = Evaluator::new(paged(record(), false));
        e.spaces.set_whole_cap(19);
        assert_eq!(e.open_space(&d, 0, &[STREAM]).unwrap(), None);
        assert_eq!(e.open_refusal(&[STREAM]), Some(crate::codec::Refusal::TooLarge));
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

    #[test]
    fn a_joined_stream_with_a_part_that_will_not_unpack_is_refused_as_a_document() {
        let d = Document::new(MemSource(bam_in_blocks(|b| b[18] = 0xff)));
        let mut e = Evaluator::new(crate::formats::bgzf());
        assert_eq!(e.open_space(&d, 0, &[1]).unwrap(), None);
        assert_eq!(e.open_refusal(&[1]), Some(crate::codec::Refusal::Failed));
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
