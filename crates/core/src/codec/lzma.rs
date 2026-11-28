//! LZMA, and the three containers that carry one.
//!
//! An LZMA1 stream is range-coded bits and nothing else: no magic, no version,
//! no length, and no statement of how it was packed. Everything a decoder has
//! to know before it can read the first bit comes from whatever wrapped it,
//! which is why the run handed to [`lzip`] is the whole member and not the
//! stream inside it.
//!
//! What a decoder needs is three numbers and a size. lzip fixes the three:
//! three literal context bits, no literal position bits, two position bits,
//! which is the byte 0x5D wherever an LZMA header writes them. The dictionary
//! size is the one thing lzip's own header byte says. How much comes out is
//! not written anywhere before the stream, so the stream is read to its
//! end-of-stream marker, which lzip always writes. lzip's trailer does give
//! the size, but it sits *after* the stream: reading it would mean measuring
//! backwards to answer a question the format already answers forwards.
//!
//! 7z writes the same three numbers into the archive's header instead, and
//! states the size separately; [`lzma2`] is the one shape that carries its own
//! settings, in a header per chunk.
//!
//! What reads the symbols is [`decoder`], and its module doc is the one to
//! read: it says what a step of an LZMA trace can and cannot claim, which is
//! less than a deflate step claims and is the honest amount.

mod decoder;

use decoder::{Coarsening, Limits, Props, Range, State, Stop};

use crate::codec::{BlockKind, Refusal, StepField, StepKind, Trace, TraceBuilder, Window, CAP_BYTES};

/// What an lzip member starts with.
const MAGIC: &[u8] = b"LZIP";

/// The only version with this shape. Version 0 was written before the format
/// settled and puts a different trailer on the end, so its bytes are not
/// these bytes and are refused rather than read wrongly.
const VERSION: u8 = 1;

/// The header: the magic, the version, and the dictionary size byte.
const HEADER: usize = 6;

/// The trailer: a CRC-32 of what came out, how much came out, and how long
/// the member is.
const TRAILER: usize = 20;

/// `lc + 9 * (lp + 5 * pb)` for lc=3, lp=0, pb=2, which is what lzip packs
/// with and never writes down. One byte for all three, the way every LZMA
/// header spends it.
const PROPS: u8 = 0x5d;

/// The smallest dictionary the format names, and the size below which its
/// header byte's top three bits mean nothing.
const MIN_DICT: u32 = 1 << 12;

/// The five bytes that prime a range coder.
const RANGE_INIT: usize = 5;

/// An lzip member: four bytes of magic, a version, a dictionary size, the raw
/// LZMA1 stream, and a trailer saying what came out.
pub fn lzip(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    if data.len() < HEADER + TRAILER || !data.starts_with(MAGIC) || data[4] != VERSION {
        return Err(Refusal::Failed);
    }
    let Some(dict) = dict_size(data[5]) else { return Err(Refusal::Failed) };
    // Everything between the header and the trailer. The stream has no length
    // on it, so this is the only way to say where it stops.
    let end = data.len() - TRAILER;

    let mut b = TraceBuilder::default();
    let mut out = Vec::new();
    b.push(0, 0, StepKind::Header(StepField::FrameHeader, u32::from(data[5])));
    stream(data, HEADER, end, PROPS, dict, None, &mut out, &mut b)?;
    b.push(end as u64 * 8, out.len() as u64, StepKind::Header(StepField::Footer, 0));
    b.finish_at(data.len() as u64 * 8, out.len() as u64);
    Ok((out, b.done()))
}

/// One coder's worth of raw LZMA1, the way 7z writes it: five property bytes
/// and a size the header states separately. The run is the packed stream and
/// nothing else, since 7z says where it starts and how long it is.
pub fn lzma1(data: &[u8], props: u8, dict: u32, unpacked: Option<u64>) -> Result<(Vec<u8>, Trace), Refusal> {
    lzma1_over(data, props, dict, unpacked, TraceBuilder::default())
}

/// The same against a trace builder the caller made. Only the test that has to
/// reach the coarsening path passes anything but a default one.
#[cfg(test)]
fn lzma1_within(
    data: &[u8],
    props: u8,
    dict: u32,
    unpacked: Option<u64>,
    budget: usize,
) -> Result<(Vec<u8>, Trace), Refusal> {
    lzma1_over(data, props, dict, unpacked, TraceBuilder::with_budget(budget))
}

fn lzma1_over(
    data: &[u8],
    props: u8,
    dict: u32,
    unpacked: Option<u64>,
    mut b: TraceBuilder,
) -> Result<(Vec<u8>, Trace), Refusal> {
    if unpacked.is_some_and(|n| n > CAP_BYTES as u64) {
        return Err(Refusal::TooLargeOutput);
    }
    let mut out = Vec::new();
    stream(data, 0, data.len(), props, dict, unpacked.map(|n| n as usize), &mut out, &mut b)?;
    b.finish_at(data.len() as u64 * 8, out.len() as u64);
    Ok((out, b.done()))
}

/// One LZMA1 stream inside whatever is carrying it: the properties the
/// container supplies, the five bytes that prime the coder, the symbols, and
/// whatever the coder never got round to pulling.
fn stream(
    data: &[u8],
    from: usize,
    to: usize,
    props: u8,
    dict: u32,
    unpacked: Option<usize>,
    out: &mut Vec<u8>,
    b: &mut TraceBuilder,
) -> Result<(), Refusal> {
    let props = Props::new(props, false)?;
    let mut st = State::new(props);
    let mut rc = Range::start(data, 0, from, to)?;

    // The block is opened before its own machinery is written down, so the
    // five bytes priming the coder belong to it. A block whose steps started
    // after them would still cover their bytes, and a listing would call bytes
    // it holds and does not name unmapped.
    let at_out = out.len() as u64;
    b.open_block(from as u64 * 8, at_out);
    // The settings, which the run does not hold: a step of no width saying
    // what the decoder was told. Then the five bytes it did hold.
    b.push(from as u64 * 8, at_out, StepKind::Header(StepField::LzmaProps, props_byte(props)));
    b.push(from as u64 * 8, at_out, StepKind::Header(StepField::RangeInit, RANGE_INIT as u32));

    let mut coarse = Coarsening { on: false, step: b.steps(), in_bits: rc.bit_pos(), out_bytes: at_out };
    let stop = decoder::run(
        &mut st,
        &mut rc,
        out,
        Limits {
            dict_start: 0,
            dict_size: dict.max(MIN_DICT),
            limit: unpacked,
            marker_allowed: true,
            steps_from: &mut coarse,
        },
        b,
    )?;
    // A marker before the container's count is a stream that stopped short,
    // which is a broken file and not a shorter one.
    if stop == Stop::Marker && unpacked.is_some_and(|n| out.len() != n) {
        return Err(Refusal::Failed);
    }
    b.close_block(rc.bit_pos(), out.len() as u64, BlockKind::Sequences, true);
    decoder::padding(b, rc.byte_pos(), to, out.len() as u64);
    Ok(())
}

/// An LZMA2 stream: chunks, each saying whether it is packed and whether it
/// resets the state, with the packing written into the chunk headers.
///
/// Nothing has to be told how it was made, which is the difference from
/// LZMA1 and the reason 7z and xz both moved to it. The dictionary size a
/// container writes down is only a hint about memory, so it is not asked for
/// here: a decoder that allocates what the stream turns out to need reads
/// every stream a smaller one would, and refuses none it should not.
///
/// The rules a chunk header obeys are the ones liblzma enforces: see
/// [`Lzma2::chunk`].
pub fn lzma2(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    lzma2_over(data, TraceBuilder::default())
}

/// The same with a ceiling on how many steps the trace may hold.
///
/// What a container made of several LZMA2 streams needs, which is what an xz
/// stream of several blocks is: the budget is the whole file's, so each block
/// is handed what the blocks before it left rather than a fresh cap of its
/// own, and a hundred blocks cannot record a hundred times the limit. A budget
/// of nothing is the other end of the same lever: the decoder gives up naming
/// symbols before it names one, which is how [`crate::codec::xz::bytes`] asks
/// for bytes without paying for a map nobody wanted.
///
/// The bytes do not change with the budget. Giving up naming symbols is
/// something the trace does; see [`decoder::run`].
pub(super) fn lzma2_within(data: &[u8], budget: usize) -> Result<(Vec<u8>, Trace), Refusal> {
    lzma2_over(data, TraceBuilder::with_budget(budget))
}

fn lzma2_over(data: &[u8], mut b: TraceBuilder) -> Result<(Vec<u8>, Trace), Refusal> {
    let mut out: Vec<u8> = Vec::new();
    let mut stream = Lzma2::new();
    while !stream.chunk(data, &mut out, &mut b)? {}

    // Whatever is behind the chunk that ended the stream. An xz block pads its
    // LZMA2 out to a multiple of four bytes, so there usually is some.
    decoder::padding(&mut b, stream.at, data.len(), out.len() as u64);
    b.finish_at(data.len() as u64 * 8, out.len() as u64);
    Ok((out, b.done()))
}

/// The most one LZMA2 chunk comes to: its size is 21 bits, written one less
/// than it is.
pub(super) const LZMA2_CHUNK_MAX: usize = 1 << 21;

/// An LZMA2 stream part way through, between two chunks: what the decoder
/// keeps from one chunk to the next.
///
/// [`lzma2`] reads a stream to its end with one of these. A reader that wants
/// only the first part of a stream stops after any chunk, and carries on later
/// with the same one. The output is the dictionary, so carrying on needs the
/// same output vector and the same stream as the chunks before.
pub(super) struct Lzma2 {
    st: State,
    coarse: Coarsening,
    /// The byte of the stream the next chunk starts at.
    at: usize,
    /// Where the dictionary was last reset, in the output.
    dict_start: usize,
    need_dict_reset: bool,
    need_props: bool,
}

/// What a chunk's header says, read without decoding the chunk.
struct Header {
    /// The first byte: 0x00 ends the stream, 0x01 and 0x02 are a stored
    /// chunk, and from 0x80 on a packed chunk, whose top three bits say what
    /// it resets.
    control: u8,
    /// How many bytes the chunk comes to. Nothing for the end of the stream.
    unpacked: usize,
    /// The properties byte, which a packed chunk resetting them carries.
    props: Option<u8>,
    /// Where the chunk's data starts: the stored bytes, or the five bytes that
    /// prime the range coder.
    from: usize,
    /// Where the next chunk starts.
    end: usize,
}

/// The header of the chunk at byte `at` of the stream, read from `window`.
/// Refused when the header or the chunk runs past the end of the stream, when
/// the window does not hold the header, or when the control byte is one the
/// format does not define.
fn header(window: Window<'_>, at: usize) -> Result<Header, Refusal> {
    let byte = |at: usize| window.get(at..at + 1).map(|b| b[0]).ok_or(Refusal::Failed);
    let stream = window.run_len();
    let control = byte(at)?;
    if control == 0x00 {
        return Ok(Header { control, unpacked: 0, props: None, from: at + 1, end: at + 1 });
    }
    if control >= 0x80 {
        // A packed chunk: five bits of the size in the control byte, two more
        // bytes of it, and two bytes of how long the packed form is. Both
        // counts are written one less than they are, since a chunk of nothing
        // would say nothing.
        let Some(head) = window.get(at + 1..at + 5) else { return Err(Refusal::Failed) };
        let unpacked = ((usize::from(control & 0x1f) << 16) | be16(&head[0..2])) + 1;
        let packed = be16(&head[2..4]) + 1;
        let mut from = at + 5;
        let mut props = None;
        if control >= 0xc0 {
            props = Some(byte(from)?);
            from += 1;
        }
        let end = from.checked_add(packed).filter(|&e| e <= stream).ok_or(Refusal::Failed)?;
        return Ok(Header { control, unpacked, props, from, end });
    }
    // 0x03 to 0x7f is nothing the format defines.
    if control > 0x02 {
        return Err(Refusal::Failed);
    }
    // A chunk stored as it is: two bytes of its length, then the bytes.
    let Some(head) = window.get(at + 1..at + 3) else { return Err(Refusal::Failed) };
    let unpacked = be16(head) + 1;
    let from = at + 3;
    let end = from.checked_add(unpacked).filter(|&e| e <= stream).ok_or(Refusal::Failed)?;
    Ok(Header { control, unpacked, props: None, from, end })
}

/// How many bytes an LZMA2 stream comes to, from its chunk headers alone:
/// each says how long it is and how much it makes, so nothing is decoded.
/// Refused when the headers run past the end of `data` before the byte that
/// ends the stream. Only the tests ask this of a whole stream; a read of an
/// xz span asks [`lzma2_len_in`].
#[cfg(test)]
pub(super) fn lzma2_len(data: &[u8]) -> Result<u64, Refusal> {
    lzma2_len_in(Window::whole(data))
}

/// How many bytes an LZMA2 stream comes to, from its chunk headers alone,
/// read from `window`, which has to hold every one of them: refused when it
/// does not, or when the headers run past the end of the stream before the
/// byte that ends it.
pub(super) fn lzma2_len_in(window: Window<'_>) -> Result<u64, Refusal> {
    let mut at = 0usize;
    let mut len = 0u64;
    loop {
        let h = header(window, at)?;
        if h.control == 0x00 {
            return Ok(len);
        }
        len += h.unpacked as u64;
        at = h.end;
    }
}

impl Lzma2 {
    pub(super) fn new() -> Lzma2 {
        Lzma2 {
            st: State::new(Props { lc: 0, lp: 0, pb: 0 }),
            coarse: Coarsening { on: false, step: 0, in_bits: 0, out_bytes: 0 },
            at: 0,
            dict_start: 0,
            need_dict_reset: true,
            need_props: true,
        }
    }

    /// How much memory the decoder holds, for a cache that weighs what it
    /// keeps.
    pub(super) fn held_bytes(&self) -> usize {
        std::mem::size_of::<Lzma2>() + self.st.heap_bytes()
    }

    /// Read the next chunk of `data` onto the end of `out`: true when it was
    /// the byte that ends the stream. `data` is the stream from its first
    /// byte, and `out` holds everything the chunks before this one made.
    ///
    /// The rules a chunk header obeys are the ones liblzma enforces. The first
    /// chunk has to reset the dictionary, and the first packed chunk after any
    /// dictionary reset has to carry its properties, because there would
    /// otherwise be nothing to read it with. A stream breaking either is
    /// refused rather than read against whatever happened to be left in the
    /// decoder.
    pub(super) fn chunk(&mut self, data: &[u8], out: &mut Vec<u8>, b: &mut TraceBuilder) -> Result<bool, Refusal> {
        self.chunk_in(Window::whole(data), out, b)
    }

    /// [`Lzma2::chunk`] read from `window`, a window of the stream that holds
    /// the whole of the next chunk, its header included: from the byte the
    /// chunk starts at, at the least. Positions in the trace still count from
    /// the stream's first byte. A window that does not hold the chunk is
    /// refused before the decoder changes, as [`Refusal::Failed`].
    pub(super) fn chunk_in(&mut self, window: Window<'_>, out: &mut Vec<u8>, b: &mut TraceBuilder) -> Result<bool, Refusal> {
        let at = self.at;
        let h = header(window, at)?;
        if window.get(at..h.end).is_none() {
            return Err(Refusal::Failed);
        }
        if h.control == 0x00 {
            b.push(at as u64 * 8, out.len() as u64, StepKind::EndOfBlock);
            self.at = h.end;
            return Ok(true);
        }
        // Resetting everything is the only way to start, and the only way a
        // reader jumping into the middle of a stream could ever begin.
        if h.control >= 0xe0 || h.control == 0x01 {
            self.dict_start = out.len();
            self.need_dict_reset = false;
            self.need_props = true;
        } else if self.need_dict_reset {
            return Err(Refusal::Failed);
        }

        if h.control < 0x80 {
            // A chunk stored as it is. It feeds the dictionary and leaves the
            // decoder's state alone; a chunk that reset the dictionary has
            // already said the next packed one must carry its properties.
            if out.len() + h.unpacked > CAP_BYTES {
                return Err(Refusal::TooLargeOutput);
            }
            b.open_block(at as u64 * 8, out.len() as u64);
            b.push(at as u64 * 8, out.len() as u64, StepKind::Header(StepField::BlockHeader, h.unpacked as u32));
            b.push(h.from as u64 * 8, out.len() as u64, StepKind::Stored);
            out.extend_from_slice(window.get(h.from..h.end).ok_or(Refusal::Failed)?);
            b.close_block(h.end as u64 * 8, out.len() as u64, BlockKind::Stored, false);
            self.at = h.end;
            return Ok(false);
        }

        match h.props {
            Some(byte) => {
                self.st.reset(Props::new(byte, true)?);
                self.need_props = false;
            }
            None if self.need_props => return Err(Refusal::Failed),
            None if h.control >= 0xa0 => self.st.reset(self.st.props()),
            None => {}
        }
        if out.len() + h.unpacked > CAP_BYTES {
            return Err(Refusal::TooLargeOutput);
        }
        let mut rc = Range::start(window.bytes(), window.start(), h.from, h.end)?;

        // Opened first, so the chunk's own header is one of its steps rather
        // than bytes it covers and never names.
        b.open_block(at as u64 * 8, out.len() as u64);
        b.push(at as u64 * 8, out.len() as u64, StepKind::Header(StepField::BlockHeader, h.unpacked as u32));
        b.push((at + 5) as u64 * 8, out.len() as u64, StepKind::Header(StepField::LzmaProps, props_byte(self.st.props())));
        b.push(h.from as u64 * 8, out.len() as u64, StepKind::Header(StepField::RangeInit, RANGE_INIT as u32));

        self.coarse.step = b.steps();
        self.coarse.in_bits = rc.bit_pos();
        self.coarse.out_bytes = out.len() as u64;
        let want = out.len() + h.unpacked;
        let stop = decoder::run(
            &mut self.st,
            &mut rc,
            out,
            Limits {
                dict_start: self.dict_start,
                dict_size: u32::MAX,
                limit: Some(want),
                // A chunk says how long it is, so a marker inside one is two
                // answers to the same question.
                marker_allowed: false,
                steps_from: &mut self.coarse,
            },
            b,
        )?;
        debug_assert_eq!(stop, Stop::Size);
        if out.len() != want {
            return Err(Refusal::Failed);
        }
        b.close_block(rc.bit_pos(), out.len() as u64, BlockKind::Sequences, false);
        decoder::padding(b, rc.byte_pos(), h.end, out.len() as u64);
        self.at = h.end;
        Ok(false)
    }
}

/// Two bytes, most significant first, which is how LZMA2 writes its sizes and
/// the one place in the format that is not little-endian.
fn be16(bytes: &[u8]) -> usize {
    usize::from(bytes[0]) << 8 | usize::from(bytes[1])
}

/// The three numbers back in the one byte they are written as, so a step can
/// report what a container said without the reader having to hold the
/// arithmetic in their head.
fn props_byte(p: Props) -> u32 {
    p.lc + 9 * (p.lp + 5 * p.pb)
}

/// The dictionary size the header byte names, or nothing when it names one
/// the format does not have.
///
/// Two numbers in one byte. The low five bits are a power of two, and the top
/// three subtract that many sixteenths of it, which is what lets a size
/// between two powers be named without a second field: seven sixteenths off
/// reaches below the power beneath, so every size in the range has a spelling.
fn dict_size(byte: u8) -> Option<u32> {
    let bits = byte & 0x1f;
    if !(12..=29).contains(&bits) {
        return None;
    }
    let base = 1u32 << bits;
    // At the smallest size there is nothing under it to reach for, and lzip's
    // own reader leaves the top three bits alone there.
    Some(if base > MIN_DICT { base - (base / 16) * u32::from(byte >> 5) } else { base })
}

#[cfg(test)]
mod tests;
