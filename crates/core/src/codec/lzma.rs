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

use crate::codec::{BlockKind, Refusal, StepField, StepKind, Trace, TraceBuilder, CAP_BYTES};

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
        return Err(Refusal::TooLarge);
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
    let mut rc = Range::start(data, from, to)?;

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
/// The rules a chunk header obeys are the ones liblzma enforces. The first
/// chunk has to reset the dictionary, and the first packed chunk after any
/// dictionary reset has to carry its properties, because there would otherwise
/// be nothing to read it with. A stream breaking either is refused rather than
/// read against whatever happened to be left in the decoder.
pub fn lzma2(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    lzma2_over(data, TraceBuilder::default())
}

/// The same against a trace builder the caller made. Only the test that has to
/// reach the coarsening path across a chunk boundary passes anything else.
#[cfg(test)]
fn lzma2_within(data: &[u8], budget: usize) -> Result<(Vec<u8>, Trace), Refusal> {
    lzma2_over(data, TraceBuilder::with_budget(budget))
}

fn lzma2_over(data: &[u8], mut b: TraceBuilder) -> Result<(Vec<u8>, Trace), Refusal> {
    let mut out: Vec<u8> = Vec::new();
    let mut st = State::new(Props { lc: 0, lp: 0, pb: 0 });
    let mut coarse = Coarsening { on: false, step: 0, in_bits: 0, out_bytes: 0 };

    let mut at = 0usize;
    let mut dict_start = 0usize;
    let mut need_dict_reset = true;
    let mut need_props = true;

    loop {
        let Some(&control) = data.get(at) else { return Err(Refusal::Failed) };
        if control == 0x00 {
            b.push(at as u64 * 8, out.len() as u64, StepKind::EndOfBlock);
            at += 1;
            break;
        }
        // Resetting everything is the only way to start, and the only way a
        // reader jumping into the middle of a stream could ever begin.
        if control >= 0xe0 || control == 0x01 {
            dict_start = out.len();
            need_dict_reset = false;
            need_props = true;
        } else if need_dict_reset {
            return Err(Refusal::Failed);
        }

        if control >= 0x80 {
            // A packed chunk: five bits of the size in the control byte, two
            // more bytes of it, and two bytes of how long the packed form is.
            // Both counts are written one less than they are, since a chunk of
            // nothing would say nothing.
            let Some(head) = data.get(at + 1..at + 5) else { return Err(Refusal::Failed) };
            let unpacked = ((usize::from(control & 0x1f) << 16) | be16(&head[0..2])) + 1;
            let packed = be16(&head[2..4]) + 1;
            let mut props_at = at + 5;

            if control >= 0xc0 {
                let Some(&byte) = data.get(props_at) else { return Err(Refusal::Failed) };
                st.reset(Props::new(byte, true)?);
                props_at += 1;
                need_props = false;
            } else if need_props {
                return Err(Refusal::Failed);
            } else if control >= 0xa0 {
                st.reset(st.props());
            }

            let end = props_at.checked_add(packed).filter(|&e| e <= data.len()).ok_or(Refusal::Failed)?;
            if out.len() + unpacked > CAP_BYTES {
                return Err(Refusal::TooLarge);
            }
            let mut rc = Range::start(data, props_at, end)?;

            // Opened first, so the chunk's own header is one of its steps
            // rather than bytes it covers and never names.
            b.open_block(at as u64 * 8, out.len() as u64);
            b.push(at as u64 * 8, out.len() as u64, StepKind::Header(StepField::BlockHeader, unpacked as u32));
            b.push((at + 5) as u64 * 8, out.len() as u64, StepKind::Header(StepField::LzmaProps, props_byte(st.props())));
            b.push(props_at as u64 * 8, out.len() as u64, StepKind::Header(StepField::RangeInit, RANGE_INIT as u32));

            coarse.step = b.steps();
            coarse.in_bits = rc.bit_pos();
            coarse.out_bytes = out.len() as u64;
            let want = out.len() + unpacked;
            let stop = decoder::run(
                &mut st,
                &mut rc,
                &mut out,
                Limits {
                    dict_start,
                    dict_size: u32::MAX,
                    limit: Some(want),
                    // A chunk says how long it is, so a marker inside one is
                    // two answers to the same question.
                    marker_allowed: false,
                    steps_from: &mut coarse,
                },
                &mut b,
            )?;
            debug_assert_eq!(stop, Stop::Size);
            if out.len() != want {
                return Err(Refusal::Failed);
            }
            b.close_block(rc.bit_pos(), out.len() as u64, BlockKind::Sequences, false);
            decoder::padding(&mut b, rc.byte_pos(), end, out.len() as u64);
            at = end;
        } else if control > 0x02 {
            // 0x03 to 0x7f is nothing the format defines.
            return Err(Refusal::Failed);
        } else {
            // A chunk stored as it is. It feeds the dictionary and leaves the
            // decoder's state alone; a chunk that reset the dictionary has
            // already said the next packed one must carry its properties.
            let Some(head) = data.get(at + 1..at + 3) else { return Err(Refusal::Failed) };
            let unpacked = be16(head) + 1;
            let from = at + 3;
            let end = from.checked_add(unpacked).filter(|&e| e <= data.len()).ok_or(Refusal::Failed)?;
            if out.len() + unpacked > CAP_BYTES {
                return Err(Refusal::TooLarge);
            }
            b.open_block(at as u64 * 8, out.len() as u64);
            b.push(at as u64 * 8, out.len() as u64, StepKind::Header(StepField::BlockHeader, unpacked as u32));
            b.push(from as u64 * 8, out.len() as u64, StepKind::Stored);
            out.extend_from_slice(&data[from..end]);
            b.close_block(end as u64 * 8, out.len() as u64, BlockKind::Stored, false);
            at = end;
        }
    }

    // Whatever is behind the chunk that ended the stream. An xz block pads its
    // LZMA2 out to a multiple of four bytes, so there usually is some.
    decoder::padding(&mut b, at, data.len(), out.len() as u64);
    b.finish_at(data.len() as u64 * 8, out.len() as u64);
    Ok((out, b.done()))
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
