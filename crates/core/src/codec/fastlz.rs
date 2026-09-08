//! FastLZ: a byte-oriented LZ77 with no entropy coding behind it.
//!
//! A block is nothing but a control byte and what that byte says: a run of
//! literals given as themselves, or a match copied from further back. No
//! headers, no tables, no bit packing, which puts it beside LZ4 as a format
//! where the trace is exact down to the byte and worth every line it costs.
//! `lz4.rs` next door is the same shape and was the model for this one.
//!
//! **Two levels, told apart by the first byte.** The top three bits of it are
//! the level less one, so a level 1 stream opens with a byte under 32 and a
//! level 2 stream with one from 32 to 63; the rest of that byte is already the
//! first control, which is why a stream always begins with literals. The two
//! differ in three places and nowhere else: level 2 extends a length with as
//! many bytes as it takes rather than one, spends a distance byte of 255 on
//! two more bytes and a further 8191 of reach, and reads its input to the last
//! byte where level 1 stops two short.
//!
//! **Which one a file holds.** `fastlz_compress` picks level 1 under 64 KiB
//! and level 2 at or above it, and Godot hands it one block at a time at a
//! default block size of 4096, so a resource compressed by the engine is level
//! 1 unless a caller asked `FileAccessCompressed` for blocks sixteen times the
//! usual size. Both are read here; only one is likely to be seen.
//!
//! **Short inputs.** `Compression::compress` pads anything under sixteen bytes
//! out to sixteen with zeroes before handing it over, so a block holding a few
//! bytes unpacks to sixteen and the last of them are zeroes the file never
//! had. That is the engine's doing and not something a decoder can undo: the
//! padding is indistinguishable from data by the time it is compressed.

use crate::codec::{BlockKind, Refusal, StepField, StepKind, Trace, TraceBuilder, CAP_BYTES};

/// How far back level 2 reaches before it starts spending two extra bytes on
/// the distance, and the amount those two bytes are counted on top of.
const MAX_L2_DISTANCE: u32 = 8191;

/// The shortest match either level writes, which is what a length of zero in
/// the control byte means.
const MIN_MATCH: u32 = 3;

/// The length that says the control byte ran out of room and more follows.
const LEN_FULL: u32 = 6;

/// Which of the two shapes a stream is written in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    One,
    Two,
}

/// One FastLZ block, with nothing wrapped round it.
pub fn block(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    // The level is the top three bits of the first byte and the only thing in
    // the format that stands outside a control byte. A third level would be
    // read as neither, which is what the reference decoder does with it.
    match data.first().ok_or(Refusal::Failed)? >> 5 {
        0 => run(data, Level::One),
        1 => run(data, Level::Two),
        _ => Err(Refusal::Failed),
    }
}

fn run(data: &[u8], level: Level) -> Result<(Vec<u8>, Trace), Refusal> {
    let mut b = TraceBuilder::default();
    let mut out: Vec<u8> = Vec::new();
    // The first control is the low five bits of the byte that named the level,
    // so it is under 32 and a stream opens with literals every time.
    let mut ctrl = (data[0] & 31) as u32;
    let mut token_at = 0usize;
    let mut at = 1usize;
    let mut coarse = false;
    // One block holding every sequence. FastLZ has no block structure of its
    // own, and a trace with no blocks in it has no rows for the reader to open.
    b.open_block(0, 0);
    loop {
        if !coarse && b.over_budget() {
            coarse = true;
            b.coarsen();
            b.push(token_at as u64 * 8, out.len() as u64, StepKind::Opaque);
        }
        if !coarse {
            b.push(token_at as u64 * 8, out.len() as u64, StepKind::Header(StepField::Token, ctrl));
        }
        if ctrl >= 32 {
            // A match: the top three bits are its length and the low five the
            // high bits of how far back it reads.
            let mut len = (ctrl >> 5) - 1;
            let ofs = (ctrl & 31) << 8;
            if len == LEN_FULL {
                let ext_at = at;
                match level {
                    Level::One => len += byte(data, &mut at)? as u32,
                    // As many bytes as it takes, each 255 saying another
                    // follows. Capped as it goes: a handful of bytes can ask
                    // for more output than the whole run could hold.
                    Level::Two => loop {
                        let code = byte(data, &mut at)?;
                        len += code as u32;
                        if code != 255 {
                            break;
                        }
                        if len as usize > CAP_BYTES {
                            return Err(Refusal::TooLarge);
                        }
                    },
                }
                if !coarse {
                    b.push(ext_at as u64 * 8, out.len() as u64, StepKind::Header(StepField::LengthExtra, len + MIN_MATCH));
                }
            }
            let ofs_at = at;
            let code = byte(data, &mut at)?;
            let mut back = ofs + code as u32;
            len += MIN_MATCH;
            // Level 2's far match: a distance byte of 255 against a full five
            // bits above it is not a distance at all but a flag saying the
            // real one is the two bytes after it, counted on top of every
            // distance the short form could have said.
            if level == Level::Two && code == 255 && ofs == 31 << 8 {
                let hi = byte(data, &mut at)? as u32;
                let lo = byte(data, &mut at)? as u32;
                back = (hi << 8) + lo + MAX_L2_DISTANCE;
            }
            // The distance is one more than the bytes say, since a match may
            // read the byte immediately behind it and nothing may read zero
            // bytes back.
            let dist = back + 1;
            if !coarse {
                b.push(ofs_at as u64 * 8, out.len() as u64, StepKind::Header(StepField::Offset, dist));
            }
            if dist as usize > out.len() {
                return Err(Refusal::Failed);
            }
            if out.len() + len as usize > CAP_BYTES {
                return Err(Refusal::TooLarge);
            }
            let from = out.len() - dist as usize;
            let was = out.len();
            // Overlapping copies are the ordinary case: a distance of one
            // fills, which is how a run of the same byte is written.
            for k in 0..len as usize {
                let byte = out[from + k];
                out.push(byte);
            }
            if !coarse {
                b.push(at as u64 * 8, was as u64, StepKind::Match { len, dist });
            }
        } else {
            // A run of bytes given as themselves, one more than the control
            // says, so the shortest run is one byte and the longest is 32.
            let n = ctrl as usize + 1;
            if at + n > data.len() {
                return Err(Refusal::Failed);
            }
            if out.len() + n > CAP_BYTES {
                return Err(Refusal::TooLarge);
            }
            if !coarse {
                b.push(at as u64 * 8, out.len() as u64, StepKind::Stored);
            }
            out.extend_from_slice(&data[at..at + n]);
            at += n;
        }
        // Where the two levels part for the third time. Level 1 stops as soon
        // as fewer than two bytes are left, since the shortest thing it could
        // read is a control and a distance; level 2 reads to the last byte. An
        // encoder leaves neither short, so this only tells a truncated stream
        // from a whole one.
        let more = match level {
            Level::One => at + 2 <= data.len(),
            Level::Two => at < data.len(),
        };
        if !more {
            break;
        }
        token_at = at;
        ctrl = data[at] as u32;
        at += 1;
    }
    // A byte level 1 stopped in front of. Nothing an encoder writes ends this
    // way, and the trace has to tile whatever it is handed, so it is named
    // rather than folded into the step before it.
    if at < data.len() && !coarse {
        b.push(at as u64 * 8, out.len() as u64, StepKind::Header(StepField::Padding, 0));
    }
    b.close_block(data.len() as u64 * 8, out.len() as u64, BlockKind::Sequences, true);
    b.finish_at(data.len() as u64 * 8, out.len() as u64);
    Ok((out, b.done()))
}

/// The next byte, or a refusal when the stream ended in the middle of
/// something that needed one.
fn byte(data: &[u8], at: &mut usize) -> Result<u8, Refusal> {
    let &b = data.get(*at).ok_or(Refusal::Failed)?;
    *at += 1;
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stream written the one way the format leaves no room to get wrong:
    /// every byte given as itself, in runs of at most 32. Written from the
    /// format's own description rather than by reversing the decoder, so a
    /// round trip through it says something.
    fn literals(level: Level, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut first = true;
        for chunk in data.chunks(32) {
            let mut ctrl = chunk.len() as u8 - 1;
            // The first control carries the level in its top three bits.
            if first && level == Level::Two {
                ctrl |= 1 << 5;
            }
            first = false;
            out.push(ctrl);
            out.extend_from_slice(chunk);
        }
        out
    }

    /// The bytes and the map together, as `lz4.rs` checks its own.
    fn reads(packed: &[u8], expect: &[u8]) {
        let (out, trace) = block(packed).expect("we read it");
        assert_eq!(out, expect, "the bytes differ");
        trace.check_tiles().expect("the trace tiles");
        assert_eq!(trace.in_bits(), packed.len() as u64 * 8);
        assert_eq!(trace.out_bytes(), expect.len() as u64);
    }

    #[test]
    fn a_stream_of_nothing_but_literals_comes_back_as_what_went_in() {
        for level in [Level::One, Level::Two] {
            for n in [1usize, 2, 31, 32, 33, 64, 100, 1000] {
                let data: Vec<u8> = (0..n).map(|i| (i * 7 + 3) as u8).collect();
                reads(&literals(level, &data), &data);
            }
        }
    }

    /// A match, hand-assembled from the format's description: the control byte
    /// is `(len - 3) << 5` with the top five bits of the distance under it, and
    /// the byte after it is the low eight of a distance that is one more than
    /// what is written.
    #[test]
    fn a_match_copies_from_where_the_control_byte_says() {
        // Two literals, `ab`, then a match of three bytes one back, which is
        // `bbb`: a distance of one fills.
        let packed = [1u8, b'a', b'b', (3 - 3 + 1) << 5, 0];
        reads(&packed, b"abbbb");
        // The same match reaching two back, which copies `ab` and then the `a`
        // it has just written.
        let packed = [1u8, b'a', b'b', (3 - 3 + 1) << 5, 1];
        reads(&packed, b"ababa");
        // A length that fills the control byte and spills into the byte after
        // it: seven in the top bits means read one more, and 3 + 6 + 4 is 13,
        // which is thirteen bytes copied on top of the two literals.
        let packed = [1u8, b'a', b'b', 7 << 5, 4, 1];
        reads(&packed, b"abababababababa");
    }

    /// Level 2's far match: a distance byte of 255 under five set bits is not a
    /// distance but a flag, and the two bytes after it hold the real one.
    #[test]
    fn level_two_reaches_past_eight_thousand_with_two_more_bytes() {
        let long: Vec<u8> = (0..9000u32).map(|i| (i % 251) as u8).collect();
        let mut packed = literals(Level::Two, &long);
        // Copy three bytes from 9000 back, which is past what five bits and a
        // byte could have said. The written distance is one less than the real
        // one and 8191 less again.
        let want = 9000u32;
        let written = want - 1 - MAX_L2_DISTANCE;
        // Length 3 in the top three bits, and the five distance bits all set,
        // which is what the flag byte after them is read against.
        packed.push(1 << 5 | 31);
        packed.push(255);
        packed.push((written >> 8) as u8);
        packed.push(written as u8);
        let mut expect = long.clone();
        expect.extend_from_slice(&long[0..3]);
        reads(&packed, &expect);
    }

    /// The steps say where, which is the whole reason this decoder exists
    /// rather than a call into somebody else's.
    #[test]
    fn a_sequence_is_a_control_byte_then_literals_or_a_match() {
        let packed = [1u8, b'a', b'b', 7 << 5, 4, 1];
        let (_, trace) = block(&packed).expect("reads");
        let kinds: Vec<_> = trace.steps().map(|s| s.kind).collect();
        assert!(matches!(kinds[0], StepKind::Header(StepField::Token, 1)));
        assert_eq!(kinds[1], StepKind::Stored);
        assert!(matches!(kinds[3], StepKind::Header(StepField::LengthExtra, 13)));
        assert!(matches!(kinds[4], StepKind::Header(StepField::Offset, 2)));
        assert_eq!(kinds[5], StepKind::Match { len: 13, dist: 2 });
        // The first byte out was a literal and a later one was copied.
        assert_eq!(trace.map_out(0).unwrap().kind, StepKind::Stored);
        assert_eq!(trace.map_out(6).unwrap().kind, StepKind::Match { len: 13, dist: 2 });
        // Going the other way stops at the byte that named the match rather
        // than at the match, and that is the truth about the format: thirteen
        // bytes came out of a control byte, a length and a distance, and the
        // copy itself read nothing at all.
        assert!(matches!(
            trace.map_in(5 * 8).map(|s| s.kind),
            Some(StepKind::Header(StepField::Offset, 2))
        ));
    }

    #[test]
    fn broken_blocks_are_refused_rather_than_panicking() {
        assert_eq!(block(&[]).err(), Some(Refusal::Failed));
        // A level neither decoder knows.
        assert_eq!(block(&[0x40, 0, 0, 0]).err(), Some(Refusal::Failed));
        assert_eq!(block(&[0xe0, 0, 0, 0]).err(), Some(Refusal::Failed));
        // A literal run asking for more bytes than there are.
        assert!(block(&[31, b'a']).is_err());
        // A match before there is anything to copy from. Level 2, since level
        // 1 stops two bytes short and would never read it.
        assert!(block(&[0x20 | 0, b'a', 1 << 5, 200]).is_err());
        // Every prefix of something that does read, none of which may panic.
        let data: Vec<u8> = (0..500u32).map(|i| (i % 7) as u8).collect();
        let whole = literals(Level::One, &data);
        for n in 0..whole.len() {
            let _ = block(&whole[..n]);
        }
    }

    /// A stream that runs out of steps keeps decoding and keeps the map; it
    /// just stops naming every sequence. The tiling has to survive that.
    #[test]
    fn a_trace_that_gives_up_naming_symbols_still_tiles() {
        let data: Vec<u8> = (0..4000u32).map(|i| (i % 13) as u8).collect();
        let packed = literals(Level::One, &data);
        let (out, trace) = block(&packed).expect("reads");
        assert_eq!(out, data);
        trace.check_tiles().expect("tiles");
        assert!(!trace.coarse());
    }
}
