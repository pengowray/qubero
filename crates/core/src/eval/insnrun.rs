//! A run of machine instructions, measured from its bytes.
//!
//! Counting a list whose elements are not all one size means walking it, and
//! `walk.rs` walks by placing each element in the memo and asking its size.
//! For a run of instructions that is far more work than the answer needs.
//! Thumb says in the top five bits of an instruction's first halfword whether
//! it takes two bytes or four, RISC-V says the same in the low two bits of the
//! first byte, and 32-bit ARM and arm64 always take four. Counted the ordinary
//! way, the 10.9 MB of Thumb in a Tascam X8 firmware update took 75 seconds in
//! a debug build, and still takes 54 now that measuring an instruction no
//! longer writes out its text.
//!
//! So a run of plain instructions that runs to the end of its room is walked
//! here instead. Its bytes are read a chunk at a time and each instruction is
//! stepped over by its length, and nothing is placed in the memo. What the
//! walk learns is kept where the ordinary walk keeps it, in the run's
//! `ListState`: how many instructions there are and where the last one ends,
//! how far the last walk got, and the offset of every `CHECKPOINT`th
//! instruction. Placing instruction `k` later steps forward from the nearest
//! checkpoint before it, and an edit forgets what was learned past it the
//! same way it does for any other list.
//!
//! The answers are the ones the ordinary walk gives. An instruction is as long
//! as its machine says, cut short to what is left of the run, which is what
//! `read_insn` does: the last instruction of a run that ends part way through
//! one is the bytes that are left. In the file and in a stream unpacked whole,
//! a plain instruction fails to read only where the run has no bytes left.
//! This walk ends at the end of the run without reading there, so there the
//! run never has its room stretched and never stops early with a reason.
//!
//! A stream joined from packed parts, or unpacked a piece at a time, can fail
//! to read part way through: a part that will not unpack fails every read
//! that reaches into it. When one of this walk's reads fails, it stops at the
//! start of the chunk it was reading and hands the run to the ordinary walk,
//! which carries on from there one instruction at a time. That walk reaches
//! the same instruction that will not read, and ends the run there with the
//! reason, as it would have without this module.
//!
//! Anything this does not cover is left to the ordinary walk: x86, whose
//! length only a decoder can find; an element wrapped in a size or a
//! condition; a run that ends on what it reads; and a run that does not start
//! and end on a byte.

use super::walk::CHECKPOINT;
use super::*;
use crate::code::{self, Isa};

/// Bytes read at a time.
const CHUNK: u64 = 64 << 10;

/// Bytes stepped over for each element charged against a go's allowance.
/// The web app's go is 5,000 elements, meant as about a twentieth of a second
/// of work. At this rate a go steps over 20 MB, and stepping over the X8's
/// 10.9 MB of Thumb took 24 ms in a native release build.
const BYTES_PER_ELEMENT: u64 = 4096;

/// Where a walk along a run of instructions is going.
#[derive(Clone, Copy)]
enum Goal {
    /// To the start of instruction `k`.
    Start(usize),
    /// To the instruction covering `bit`, looking no further than the first
    /// `n` instructions.
    Covering { bit: u64, n: u64 },
    /// To the end of the run.
    End,
}

/// How far one chunk of the walk got.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    /// Neither the goal nor the end of the run is in this chunk.
    More,
    /// At the goal: the start of instruction `k`, or instruction `n` with
    /// nothing before it covering the bit.
    There,
    /// At the instruction that covers the bit.
    Covers,
    /// At the end of the run, before the goal.
    End,
    /// The chunk's bytes could not be read, so nothing in it was stepped
    /// over. The ordinary walk takes the run from here.
    Unreadable,
}

impl Evaluator {
    /// The machine whose instructions the run `r` holds, when the run is one
    /// this module can measure from its bytes: a run to the end of its room of
    /// instructions with nothing around them but names and origins, starting
    /// and ending on a byte, on a machine that says how long an instruction
    /// is in its first bytes.
    ///
    /// A size or a condition around the element changes how much room each
    /// instruction has, so neither is looked through here, unlike
    /// `readas::plain`, which asks only what an element holds.
    fn insn_run(&self, r: &Resolved) -> Option<Isa> {
        let Ty::Repeat { elem, until: Until::End } = &r.ty else { return None };
        if r.lsb || r.offset % 8 != 0 || r.limit % 8 != 0 {
            return None;
        }
        let mut ty: &Ty = elem;
        for _ in 0..64 {
            ty = match ty {
                Ty::Named(n) => self.template.types.get(&**n)?,
                Ty::Origin { inner } => inner,
                Ty::Insn { isa } => return isa.says_its_length().then_some(*isa),
                _ => return None,
            };
        }
        None
    }

    /// How many instructions the run at `path` holds, counted by stepping
    /// along its bytes and carrying on from where an earlier count stopped.
    /// None when the run is not one this module measures, and when a read
    /// fails part way: the ordinary walk then counts on from where this count
    /// stopped.
    ///
    /// Each of the three questions here is one call out of the ordinary walk,
    /// which is on the path the nesting limit was measured against, so what
    /// they need goes in their own frames rather than in the walk's.
    #[inline(never)]
    pub(super) fn count_insn_run<S: Source>(&mut self, doc: &Document<S>, path: &[usize], r: &Resolved) -> R<Option<u64>> {
        let Some(isa) = self.insn_run(r) else { return Ok(None) };
        loop {
            let (mut j, mut at) = {
                let l = self.list(path);
                if l.repeat_done {
                    return Ok(Some(l.repeat_len as u64));
                }
                (l.repeat_len, l.repeat_end.unwrap_or(r.offset))
            };
            if at >= r.limit {
                self.list_mut(path).repeat_done = true;
                continue;
            }
            if self.insn_chunk(doc, path, r, isa, &mut j, &mut at, Goal::End)? == Step::Unreadable {
                return Ok(None);
            }
            let m = self.list_mut(path);
            m.repeat_len = j;
            m.repeat_end = Some(at);
        }
    }

    /// Where instruction `idx` of the run at `path` starts, found by stepping
    /// along the bytes from the nearest known start before it. None when the
    /// run is not one this module measures, when it ends before `idx`, which
    /// the ordinary walk then reports, and when a read fails on the way, which
    /// the ordinary walk then steps past or stops at.
    #[inline(never)]
    pub(super) fn insn_run_start<S: Source>(&mut self, doc: &Document<S>, path: &[usize], r: &Resolved, idx: usize) -> R<Option<u64>> {
        let Some(isa) = self.insn_run(r) else { return Ok(None) };
        let (mut j, mut at) = self.nearest_start(path, idx);
        loop {
            match self.insn_chunk(doc, path, r, isa, &mut j, &mut at, Goal::Start(idx))? {
                Step::More => {}
                Step::End | Step::Unreadable => return Ok(None),
                _ => return Ok(Some(at)),
            }
        }
    }

    /// Which of the first `n` instructions of the run at `path` covers `bit`,
    /// found by stepping along the bytes from the nearest known start at or
    /// before it. The outer None is for a run this module does not measure,
    /// for a bit past the end of the run, which the ordinary walk then
    /// reports, and for a read that fails on the way, which the ordinary walk
    /// then steps past or stops at.
    #[inline(never)]
    pub(super) fn insn_run_covering<S: Source>(
        &mut self,
        doc: &Document<S>,
        path: &[usize],
        r: &Resolved,
        n: u64,
        bit: u64,
    ) -> R<Option<Option<usize>>> {
        let Some(isa) = self.insn_run(r) else { return Ok(None) };
        let (mut j, mut at) = self.nearest_start_before(path, bit);
        loop {
            match self.insn_chunk(doc, path, r, isa, &mut j, &mut at, Goal::Covering { bit, n })? {
                Step::More => {}
                Step::End | Step::Unreadable => return Ok(None),
                Step::There => return Ok(Some(None)),
                Step::Covers => return Ok(Some(Some(j))),
            }
        }
    }

    /// Step along one chunk of the run, from instruction `j` starting at `at`,
    /// towards `goal`. Leaves `j` and `at` at the instruction it stopped at,
    /// and notes that in the run's walk state and checkpoints the way the
    /// ordinary walk does. When the chunk's bytes cannot be read, nothing is
    /// changed or charged, and the answer is `Step::Unreadable`.
    #[allow(clippy::too_many_arguments)]
    fn insn_chunk<S: Source>(
        &mut self,
        doc: &Document<S>,
        path: &[usize],
        r: &Resolved,
        isa: Isa,
        j: &mut usize,
        at: &mut u64,
        goal: Goal,
    ) -> R<Step> {
        let reached = |j: usize| match goal {
            Goal::Start(k) => j >= k,
            Goal::Covering { n, .. } => j as u64 >= n,
            Goal::End => false,
        };
        if reached(*j) {
            return Ok(Step::There);
        }
        if *at >= r.limit {
            return Ok(Step::End);
        }
        // The instructions stepped over in this chunk start in its first
        // `span` bytes, which is no further than the goal can be. Placing the
        // next instruction of a listing reads a few bytes, not a whole chunk.
        // An instruction starting near the end of the span is read whole: the
        // read runs on past it by the longest an instruction can be, unless
        // the run ends first.
        let longest = isa.longest();
        let span = match goal {
            Goal::Start(k) => (k - *j) as u64 * longest as u64,
            Goal::Covering { bit, .. } => (bit - *at) / 8 + 1,
            Goal::End => CHUNK,
        }
        .min(CHUNK);
        let left = (r.limit - *at) / 8;
        let take = left.min(span + longest as u64);
        // Charged once the bytes are read. A read that fails hands the run to
        // the ordinary walk, and that walk needs what is left of the go's
        // allowance to step past even one instruction.
        let bytes = match self.read(doc, r, *at, take * 8) {
            Ok(bytes) => bytes,
            Err(EvalError::Failed(_)) => return Ok(Step::Unreadable),
            Err(e) => return Err(e),
        };
        self.spend_n(*at, take.min(span).div_ceil(BYTES_PER_ELEMENT))?;
        let upto = if take == left { bytes.len() } else { span as usize };
        let mut i = 0usize;
        let mut step = Step::More;
        while i < upto {
            if reached(*j) {
                step = Step::There;
                break;
            }
            let len = code::length(isa, &bytes[i..(i + longest).min(bytes.len())]);
            if let Goal::Covering { bit, .. } = goal {
                // The walk starts at or before the bit and the instructions
                // are end to end, so the first one that ends past it covers it.
                if bit < *at + (i + len) as u64 * 8 {
                    step = Step::Covers;
                    break;
                }
            }
            i += len;
            *j += 1;
            if *j % CHECKPOINT == 0 {
                self.checkpoint(path, *j, *at + i as u64 * 8);
            }
        }
        *at += i as u64 * 8;
        self.list_mut(path).walk_at = Some((*j, *at));
        if step == Step::More && reached(*j) {
            step = Step::There;
        }
        if step == Step::More && *at >= r.limit {
            step = Step::End;
        }
        Ok(step)
    }
}
