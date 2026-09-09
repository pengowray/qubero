//! The range coder LZMA is, and the symbols it codes, read one at a time.
//!
//! # What a step here can honestly claim
//!
//! Deflate reads a whole number of bits per symbol, so a trace of it can point
//! at the bit a literal came out of. **LZMA cannot be read that way.** It is a
//! range coder: a symbol is a narrowing of an interval, not a field, and the
//! decoder pulls a byte only when the interval has grown too thin to divide.
//! Decoding a literal may pull no input at all, and the byte that arrives on
//! the next symbol paid for both of them. There is no bit-level map of the
//! kind deflate has, and drawing one would mean inventing an answer.
//!
//! What is written down instead is the one thing that is true: **the input the
//! range decoder actually pulled while producing each symbol.** A step runs
//! from the decoder's input position before the symbol to its position after
//! it, so a symbol that pulled nothing is a step of zero width. Those steps
//! still tile the run exactly, which is what [`Trace::check_tiles`] asks of
//! them, and a reader standing on an output byte still learns which input
//! bytes were being consumed around it.
//!
//! That is a weaker claim than deflate's, and it is weaker for a reason that
//! belongs to the format rather than to this decoder. Nothing better exists to
//! say.
//!
//! # Pulling a byte early or late
//!
//! The reference decoder widens the range *after* each bit. This one widens it
//! *before*, which is the same sequence of operations rotated by one and gives
//! the same bytes: at the start the range is already at its widest, so the
//! first widening does nothing. What it changes is the attribution. Widening
//! afterwards charges a byte to the symbol that was already finished with it;
//! widening first charges it to the symbol that could not be decoded without
//! it, which is the one a reader is asking about. It also means the decoder
//! never pulls a byte it does not use, so the bytes left over at the end of a
//! stream are named as what they are rather than swallowed by the last symbol.
//!
//! # The dictionary is the output
//!
//! No window is allocated. A match reads out of what has already been written,
//! the way [`crate::codec::lha`] does, and what stops it reaching too far back
//! is `dict_start`: the point the dictionary was last reset, which is the
//! front of the output for LZMA1 and the last dictionary-resetting chunk for
//! LZMA2. LZMA needs no more than that. The dictionary size a container
//! declares is a bound on the encoder and a promise about memory, and it is
//! checked as a bound rather than used as a shape.
//!
//! [`Trace::check_tiles`]: crate::codec::Trace::check_tiles

use crate::codec::{Refusal, StepField, StepKind, TraceBuilder, CAP_BYTES};

/// Probabilities are eleven-bit, and move a thirty-second of the way to the
/// answer each time they are right.
const PROB_BITS: u32 = 11;
const PROB_ONE: u16 = 1 << PROB_BITS;
const PROB_INIT: u16 = PROB_ONE / 2;
const MOVE_BITS: u32 = 5;

/// The width below which the range can no longer be divided, and a byte is
/// pulled to widen it.
const TOP: u32 = 1 << 24;

/// How many states the machine that remembers what the last symbols were has.
const STATES: usize = 12;

/// The most position bits a stream may declare, which is what the tables
/// indexed by position state are sized for.
const POS_STATES: usize = 1 << 4;

/// How many length values get a distance model of their own before they all
/// share the last one.
const LEN_TO_POS_STATES: usize = 4;

/// The low bits of a long distance, which get a model shared by every slot.
const ALIGN_BITS: u32 = 4;

/// The slot from which a distance's middle bits stop being modelled.
const END_POS_MODEL: u32 = 14;

/// The distances that have a model of their own, and the size of the table
/// holding those models.
const FULL_DISTANCES: u32 = 1 << (END_POS_MODEL / 2);
const SPECIAL: usize = (1 + FULL_DISTANCES - END_POS_MODEL) as usize;

/// The shortest match the format can name. A distance is written as one less
/// than itself, since a match of distance zero would say nothing.
const MATCH_MIN_LEN: u32 = 2;

/// The largest `lc + lp` the properties byte can spell, which bounds the
/// literal table at three megabytes and stops a wrong byte asking for more.
const MAX_LIT_CONTEXT: u32 = 12;

/// The three numbers a stream is packed with, taken out of the one byte every
/// LZMA header spends on them.
///
/// `lc + 9 * (lp + 5 * pb)`, so the byte cannot exceed 224. LZMA2 asks for one
/// more thing than LZMA1 does: `lc + lp` no greater than four, which is what
/// its own decoder is written to, and which is why the check is a flag here
/// rather than a fact about the byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Props {
    pub lc: u32,
    pub lp: u32,
    pub pb: u32,
}

impl Props {
    pub(super) fn new(byte: u8, lzma2: bool) -> Result<Props, Refusal> {
        let mut v = u32::from(byte);
        if v >= 9 * 5 * 5 {
            return Err(Refusal::Failed);
        }
        let lc = v % 9;
        v /= 9;
        let lp = v % 5;
        let pb = v / 5;
        if lzma2 && lc + lp > 4 {
            return Err(Refusal::Failed);
        }
        Ok(Props { lc, lp, pb })
    }
}

/// The range decoder: an interval, the part of the stream that is inside it,
/// and where the next byte comes from.
///
/// `at` and `end` are absolute in the whole run, not in the stream handed to
/// this, so that a step can be written down without anything having to add an
/// offset back on. An LZMA2 chunk hands its own bounds and gets its own coder.
pub(super) struct Range<'a> {
    data: &'a [u8],
    at: usize,
    end: usize,
    range: u32,
    code: u32,
}

impl<'a> Range<'a> {
    /// Prime the coder from five bytes: one nothing reads, and four that fill
    /// the code. The first is zero in every stream an encoder wrote; the
    /// reference decoder notes a stream where it is not and reads on, so this
    /// does the same rather than turning away a file over a byte it ignores.
    pub(super) fn start(data: &'a [u8], at: usize, end: usize) -> Result<Range<'a>, Refusal> {
        if end > data.len() || at + 5 > end {
            return Err(Refusal::Failed);
        }
        let code = u32::from_be_bytes(data[at + 1..at + 5].try_into().expect("four bytes"));
        Ok(Range { data, at: at + 5, end, range: u32::MAX, code })
    }

    /// Where the next byte would come from, as a bit of the run.
    pub(super) fn bit_pos(&self) -> u64 {
        self.at as u64 * 8
    }

    pub(super) fn byte_pos(&self) -> usize {
        self.at
    }

    fn pull(&mut self) -> Result<u32, Refusal> {
        if self.at >= self.end {
            return Err(Refusal::Failed);
        }
        let b = self.data[self.at];
        self.at += 1;
        Ok(u32::from(b))
    }

    /// Widen the range if it has narrowed past what a bound can divide. Done
    /// before the bound is worked out rather than after; see the module doc.
    fn normalize(&mut self) -> Result<(), Refusal> {
        if self.range < TOP {
            let b = self.pull()?;
            self.range <<= 8;
            self.code = (self.code << 8) | b;
        }
        Ok(())
    }

    /// One bit, against the model that says how likely it was to be zero, and
    /// which the answer then moves.
    fn bit(&mut self, prob: &mut u16) -> Result<u32, Refusal> {
        self.normalize()?;
        let bound = (self.range >> PROB_BITS) * u32::from(*prob);
        if self.code < bound {
            self.range = bound;
            *prob += (PROB_ONE - *prob) >> MOVE_BITS;
            Ok(0)
        } else {
            self.range -= bound;
            self.code -= bound;
            *prob -= *prob >> MOVE_BITS;
            Ok(1)
        }
    }

    /// Bits with no model behind them: the high bits of a long distance, which
    /// are as likely one way as the other and are not worth a table.
    fn direct(&mut self, n: u32) -> Result<u32, Refusal> {
        let mut out = 0u32;
        for _ in 0..n {
            self.normalize()?;
            self.range >>= 1;
            self.code = self.code.wrapping_sub(self.range);
            let t = 0u32.wrapping_sub(self.code >> 31);
            self.code = self.code.wrapping_add(self.range & t);
            out = (out << 1).wrapping_add(t.wrapping_add(1));
        }
        Ok(out)
    }

    /// A binary tree of models, read from the most significant bit down. The
    /// path taken through the tree is the symbol.
    fn tree(&mut self, probs: &mut [u16], bits: u32) -> Result<u32, Refusal> {
        let mut m = 1u32;
        for _ in 0..bits {
            m = (m << 1) + self.bit(&mut probs[m as usize])?;
        }
        Ok(m - (1 << bits))
    }

    /// The same tree read for a symbol whose least significant bit comes
    /// first, which is how the low bits of a distance are written.
    fn tree_reverse(&mut self, probs: &mut [u16], bits: u32) -> Result<u32, Refusal> {
        let mut m = 1u32;
        let mut sym = 0u32;
        for i in 0..bits {
            let b = self.bit(&mut probs[m as usize])?;
            m = (m << 1) + b;
            sym |= b << i;
        }
        Ok(sym)
    }
}

/// The models a match length is read through: a choice of three ranges, and a
/// tree for each. Two of these exist, one for a match that named a distance
/// and one for a match that repeated an old one.
struct Len {
    choice: u16,
    choice2: u16,
    low: [[u16; 8]; POS_STATES],
    mid: [[u16; 8]; POS_STATES],
    high: [u16; 256],
}

impl Len {
    fn new() -> Len {
        Len {
            choice: PROB_INIT,
            choice2: PROB_INIT,
            low: [[PROB_INIT; 8]; POS_STATES],
            mid: [[PROB_INIT; 8]; POS_STATES],
            high: [PROB_INIT; 256],
        }
    }

    fn reset(&mut self) {
        *self = Len::new();
    }

    /// The length, less the two every match has.
    fn decode(&mut self, rc: &mut Range, pos_state: usize) -> Result<u32, Refusal> {
        if rc.bit(&mut self.choice)? == 0 {
            return rc.tree(&mut self.low[pos_state], 3);
        }
        if rc.bit(&mut self.choice2)? == 0 {
            return Ok(8 + rc.tree(&mut self.mid[pos_state], 3)?);
        }
        Ok(16 + rc.tree(&mut self.high, 8)?)
    }
}

/// Everything the decoder remembers between symbols.
///
/// Kept apart from the range coder because LZMA2 pairs one of these with a
/// fresh coder per chunk: a chunk may reset the state, the properties, both,
/// or neither, and the coder is primed again either way.
pub(super) struct State {
    props: Props,
    /// Which of the twelve shapes the last few symbols left the machine in.
    state: usize,
    /// The last four distances, most recent first.
    reps: [u32; 4],
    /// `0x300` models per literal context, and the context is `lc` bits of the
    /// byte before and `lp` bits of the position.
    lit: Vec<u16>,
    is_match: [u16; STATES * POS_STATES],
    is_rep: [u16; STATES],
    is_rep_g0: [u16; STATES],
    is_rep_g1: [u16; STATES],
    is_rep_g2: [u16; STATES],
    is_rep0_long: [u16; STATES * POS_STATES],
    pos_slot: [[u16; 64]; LEN_TO_POS_STATES],
    pos_special: [u16; SPECIAL],
    pos_align: [u16; 1 << ALIGN_BITS],
    len: Len,
    rep_len: Len,
}

impl State {
    pub(super) fn new(props: Props) -> State {
        let mut st = State {
            props,
            state: 0,
            reps: [0; 4],
            lit: Vec::new(),
            is_match: [PROB_INIT; STATES * POS_STATES],
            is_rep: [PROB_INIT; STATES],
            is_rep_g0: [PROB_INIT; STATES],
            is_rep_g1: [PROB_INIT; STATES],
            is_rep_g2: [PROB_INIT; STATES],
            is_rep0_long: [PROB_INIT; STATES * POS_STATES],
            pos_slot: [[PROB_INIT; 64]; LEN_TO_POS_STATES],
            pos_special: [PROB_INIT; SPECIAL],
            pos_align: [PROB_INIT; 1 << ALIGN_BITS],
            len: Len::new(),
            rep_len: Len::new(),
        };
        st.reset(props);
        st
    }

    /// Start over: every model back to even, every remembered distance gone,
    /// and the literal table resized if the properties changed. What an LZMA2
    /// chunk asks for when it says it resets the state.
    pub(super) fn reset(&mut self, props: Props) {
        self.props = props;
        self.state = 0;
        self.reps = [0; 4];
        // `lc + lp` is at most twelve, which the properties byte guarantees,
        // so this is at most three megabytes and cannot be asked to be more.
        let context = (props.lc + props.lp).min(MAX_LIT_CONTEXT);
        self.lit.clear();
        self.lit.resize(0x300usize << context, PROB_INIT);
        self.is_match = [PROB_INIT; STATES * POS_STATES];
        self.is_rep = [PROB_INIT; STATES];
        self.is_rep_g0 = [PROB_INIT; STATES];
        self.is_rep_g1 = [PROB_INIT; STATES];
        self.is_rep_g2 = [PROB_INIT; STATES];
        self.is_rep0_long = [PROB_INIT; STATES * POS_STATES];
        self.pos_slot = [[PROB_INIT; 64]; LEN_TO_POS_STATES];
        self.pos_special = [PROB_INIT; SPECIAL];
        self.pos_align = [PROB_INIT; 1 << ALIGN_BITS];
        self.len.reset();
        self.rep_len.reset();
    }

    pub(super) fn props(&self) -> Props {
        self.props
    }
}

/// Why a run of symbols stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Stop {
    /// The end-of-stream marker: a match whose distance is all ones, which
    /// names no distance and means there is nothing after it.
    Marker,
    /// As many bytes as the container said would come out.
    Size,
}

/// What a run of symbols is allowed to do, and where the trace of it goes.
pub(super) struct Limits<'a> {
    /// The point the dictionary was last reset. A match may not reach back
    /// past it.
    pub dict_start: usize,
    /// How far back a match may reach at all, which is what the container
    /// declared it packed with. `u32::MAX` where nothing declared one.
    pub dict_size: u32,
    /// The output length at which to stop, where the container says. `None`
    /// reads to the marker.
    pub limit: Option<usize>,
    /// Whether the end-of-stream marker may appear. An LZMA2 chunk says how
    /// long it is and forbids one.
    pub marker_allowed: bool,
    /// Where the symbols of this run start in the trace, so they can be
    /// replaced by one step if there turn out to be too many of them.
    pub steps_from: &'a mut Coarsening,
}

/// Whether the trace has given up naming symbols, and where the run being
/// named began, so that giving up can throw away what it had started.
pub(super) struct Coarsening {
    pub on: bool,
    pub step: usize,
    pub in_bits: u64,
    pub out_bytes: u64,
}

/// Read symbols until the stream says to stop.
///
/// The dictionary is `out` itself: everything already written, back as far as
/// `dict_start`. Bytes are appended, so a caller decoding an LZMA2 chunk hands
/// the same vector every time and the history carries across.
pub(super) fn run(
    st: &mut State,
    rc: &mut Range,
    out: &mut Vec<u8>,
    lim: Limits,
    b: &mut TraceBuilder,
) -> Result<Stop, Refusal> {
    let Props { lc, lp, pb } = st.props;
    let pb_mask = (1usize << pb) - 1;
    let lp_mask = (1usize << lp) - 1;
    let dict_start = lim.dict_start;
    let coarse = lim.steps_from;

    // A run that is already coarse says so once, up front, and then records
    // nothing until it ends.
    if coarse.on {
        b.push(coarse.in_bits, coarse.out_bytes, StepKind::Opaque);
    }

    loop {
        if let Some(limit) = lim.limit {
            if out.len() >= limit {
                return Ok(Stop::Size);
            }
        }
        // Too many symbols to name one at a time. The map stays, and the
        // trace says it stopped naming them; see `MAX_STEPS`.
        if !coarse.on && b.over_budget() {
            coarse.on = true;
            b.coarsen();
            b.truncate(coarse.step);
            b.push(coarse.in_bits, coarse.out_bytes, StepKind::Opaque);
        }
        if out.len() >= CAP_BYTES {
            return Err(Refusal::TooLarge);
        }

        let sym_in = rc.bit_pos();
        let sym_out = out.len() as u64;
        // The position the models are indexed by counts from the dictionary
        // reset, not from the front of the file.
        let pos = out.len() - dict_start;
        let pos_state = pos & pb_mask;

        if rc.bit(&mut st.is_match[(st.state << 4) + pos_state])? == 0 {
            let byte = literal(st, rc, out, dict_start, lc, lp_mask)?;
            out.push(byte);
            st.state = if st.state < 4 {
                0
            } else if st.state < 10 {
                st.state - 3
            } else {
                st.state - 6
            };
            if !coarse.on {
                b.push(sym_in, sym_out, StepKind::Literal(byte));
            }
            continue;
        }

        let len;
        if rc.bit(&mut st.is_rep[st.state])? != 0 {
            // A match that repeats one of the last four distances. There has
            // to be something to repeat.
            if out.len() == dict_start {
                return Err(Refusal::Failed);
            }
            if rc.bit(&mut st.is_rep_g0[st.state])? == 0 {
                if rc.bit(&mut st.is_rep0_long[(st.state << 4) + pos_state])? == 0 {
                    // One byte at the most recent distance, and no length at
                    // all: the shortest thing the format can say.
                    let byte = back(out, dict_start, st.reps[0] as usize + 1)?;
                    out.push(byte);
                    st.state = if st.state < 7 { 9 } else { 11 };
                    if !coarse.on {
                        b.push(sym_in, sym_out, StepKind::Match { len: 1, dist: st.reps[0] + 1 });
                    }
                    continue;
                }
            } else {
                // Which of the three older distances, shuffling the ones it
                // passed over down. Written as the branches rather than as a
                // comparison of the values: two of the four may hold the same
                // distance, and it is the branch and not the number that says
                // how the list moves.
                let dist;
                if rc.bit(&mut st.is_rep_g1[st.state])? == 0 {
                    dist = st.reps[1];
                } else {
                    if rc.bit(&mut st.is_rep_g2[st.state])? == 0 {
                        dist = st.reps[2];
                    } else {
                        dist = st.reps[3];
                        st.reps[3] = st.reps[2];
                    }
                    st.reps[2] = st.reps[1];
                }
                st.reps[1] = st.reps[0];
                st.reps[0] = dist;
            }
            len = st.rep_len.decode(rc, pos_state)?;
            st.state = if st.state < 7 { 8 } else { 11 };
        } else {
            st.reps[3] = st.reps[2];
            st.reps[2] = st.reps[1];
            st.reps[1] = st.reps[0];
            len = st.len.decode(rc, pos_state)?;
            st.state = if st.state < 7 { 7 } else { 10 };
            st.reps[0] = distance(st, rc, len)?;
            if st.reps[0] == u32::MAX {
                if !lim.marker_allowed {
                    return Err(Refusal::Failed);
                }
                if !coarse.on {
                    b.push(sym_in, sym_out, StepKind::EndOfBlock);
                }
                return Ok(Stop::Marker);
            }
            if st.reps[0] >= lim.dict_size || st.reps[0] as usize >= out.len() - dict_start {
                return Err(Refusal::Failed);
            }
        }

        let len = len + MATCH_MIN_LEN;
        let dist = st.reps[0] as usize + 1;
        // A match that would run past what the container said comes out is a
        // broken stream, not a match to be trimmed.
        if lim.limit.is_some_and(|l| out.len() + len as usize > l) {
            return Err(Refusal::Failed);
        }
        if out.len() + len as usize > CAP_BYTES {
            return Err(Refusal::TooLarge);
        }
        copy(out, dict_start, dist, len as usize)?;
        if !coarse.on {
            b.push(sym_in, sym_out, StepKind::Match { len, dist: dist as u32 });
        }
    }
}

/// One byte, coded against the byte before it and, when the last symbol was a
/// match, against the byte that match would have copied.
///
/// The second of those is what makes LZMA's literals cheap in the middle of a
/// near-miss: the decoder codes agreement with the expected byte until the two
/// part company, and only then falls back to coding the rest outright.
fn literal(
    st: &mut State,
    rc: &mut Range,
    out: &[u8],
    dict_start: usize,
    lc: u32,
    lp_mask: usize,
) -> Result<u8, Refusal> {
    let pos = out.len() - dict_start;
    let prev = if pos == 0 { 0u32 } else { u32::from(out[out.len() - 1]) };
    let context = ((pos & lp_mask) << lc) + (prev >> (8 - lc)) as usize;
    // The byte the last match would have copied, read before the model table
    // is borrowed: after a match, agreement with it is what gets coded.
    let matched = (st.state >= 7).then(|| back(out, dict_start, st.reps[0] as usize + 1)).transpose()?;
    let probs = &mut st.lit[0x300 * context..0x300 * (context + 1)];

    let mut sym = 1u32;
    if let Some(byte) = matched {
        let mut expect = u32::from(byte);
        loop {
            let expect_bit = (expect >> 7) & 1;
            expect = (expect << 1) & 0xff;
            let bit = rc.bit(&mut probs[(((1 + expect_bit) << 8) + sym) as usize])?;
            sym = (sym << 1) | bit;
            if expect_bit != bit {
                break;
            }
            if sym >= 0x100 {
                return Ok(sym as u8);
            }
        }
    }
    while sym < 0x100 {
        sym = (sym << 1) | rc.bit(&mut probs[sym as usize])?;
    }
    Ok(sym as u8)
}

/// How far back a match reads, less one. Written as a slot naming how many
/// bits follow, then those bits: the near distances get a model each, the far
/// ones get a model for their lowest four bits and raw bits above that.
fn distance(st: &mut State, rc: &mut Range, len: u32) -> Result<u32, Refusal> {
    let len_state = (len as usize).min(LEN_TO_POS_STATES - 1);
    let slot = rc.tree(&mut st.pos_slot[len_state], 6)?;
    if slot < 4 {
        return Ok(slot);
    }
    let direct_bits = (slot >> 1) - 1;
    let mut dist = (2 | (slot & 1)) << direct_bits;
    if slot < END_POS_MODEL {
        // The models for this distance sit at an offset that makes every slot's
        // tree start where the last one left off.
        let base = (dist - slot) as usize;
        let mut m = 1u32;
        let mut low = 0u32;
        for i in 0..direct_bits {
            let bit = rc.bit(&mut st.pos_special[base + m as usize])?;
            m = (m << 1) + bit;
            low |= bit << i;
        }
        dist += low;
    } else {
        dist += rc.direct(direct_bits - ALIGN_BITS)? << ALIGN_BITS;
        dist += rc.tree_reverse(&mut st.pos_align, ALIGN_BITS)?;
    }
    Ok(dist)
}

/// The byte `dist` back in the output, refusing to read past the dictionary.
fn back(out: &[u8], dict_start: usize, dist: usize) -> Result<u8, Refusal> {
    if dist == 0 || dist > out.len() - dict_start {
        return Err(Refusal::Failed);
    }
    Ok(out[out.len() - dist])
}

/// Copy a match out of the output into the output. Byte at a time on purpose:
/// a match may be longer than its distance, which is how a run of one byte is
/// written, and a block copy would read what it has not written yet.
fn copy(out: &mut Vec<u8>, dict_start: usize, dist: usize, len: usize) -> Result<(), Refusal> {
    if dist == 0 || dist > out.len() - dict_start {
        return Err(Refusal::Failed);
    }
    out.reserve(len);
    let mut from = out.len() - dist;
    for _ in 0..len {
        let byte = out[from];
        out.push(byte);
        from += 1;
    }
    Ok(())
}

/// Name the bytes of a stream the range coder never pulled.
///
/// There are almost always some. An encoder flushes its interval on the way
/// out, and a decoder that widens its range lazily stops needing input before
/// the bytes run out. Saying so is the difference between a trace that tiles
/// and one that quietly loses the last few bytes of every stream.
pub(super) fn padding(b: &mut TraceBuilder, from: usize, to: usize, out: u64) {
    if to > from {
        b.push(from as u64 * 8, out, StepKind::Header(StepField::Padding, (to - from) as u32));
    }
}
