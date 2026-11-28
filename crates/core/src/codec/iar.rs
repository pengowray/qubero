//! The two ways IAR's linker packs the bytes a program copies into RAM when it
//! starts.
//!
//! A program built with IAR Embedded Workbench for Arm keeps its initialised
//! data, and any code it runs from RAM, in flash until startup copies it out.
//! The linker may pack those runs on the way in, `initialize by copy with
//! packing = ...` in its configuration file, and the startup code unpacks them
//! through a table of routines and ranges the linker writes, which IAR calls
//! the init table. Two of the packings are here: `lz77` and `packbits`.
//!
//! Neither format is documented anywhere. What is here was read out of the
//! unpacking routines in the Tascam Portacapture X8's firmware, which has one
//! of each, and is checked against them: every run in both releases unpacks to
//! exactly the length its init table entry says, using up exactly the bytes
//! the entry gives it.
//!
//! Neither stream says how long it is or how much comes out. The init table
//! entry says both, so a template sizes the run from the entry and checks
//! what came out against it.

use crate::codec::{Refusal, StepField, StepKind, Trace, TraceBuilder, BlockKind, CAP_BYTES};

/// An `lz77` run, unpacked, with a step for every byte of it the decoder read.
///
/// The run is tokens one after another to its end. A token is one byte: its
/// low two bits say how many literals come next, its top four how long the
/// match after them is, and the two bits between are the top of the match's
/// distance. A field that does not fit is the byte after the token, so in
/// order a token is: the token, a literal count, a match length, the literals,
/// the low byte of the distance, the high byte of the distance.
///
/// - Literals: 1 to 3 in the token mean 0 to 2 literals; 0 means the byte
///   after the token holds the count, less 2.
/// - Match length: 0 means no match, and the next token follows the literals;
///   1 to 14 mean a match of 3 to 16 bytes; 15 means the byte after the
///   literal count holds the length, less 17.
/// - Distance: the low byte always follows the literals when there is a
///   match. The high byte is the token's middle two bits, unless those are 3,
///   in which case it is the byte after the low byte.
///
/// A match may overlap what it is copying, so a distance of 1 repeats a byte.
pub fn lz77(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    let mut b = TraceBuilder::default();
    let mut out: Vec<u8> = Vec::new();
    let mut at = 0usize;
    let mut coarse = false;
    let bit = |at: usize| at as u64 * 8;
    let byte = |at: &mut usize| -> Result<u8, Refusal> {
        let &v = data.get(*at).ok_or(Refusal::Failed)?;
        *at += 1;
        Ok(v)
    };
    b.open_block(0, 0);
    while at < data.len() {
        if !coarse && b.over_budget() {
            coarse = true;
            b.coarsen();
            b.push(bit(at), out.len() as u64, StepKind::Opaque);
        }
        let token = data[at];
        if !coarse {
            b.push(bit(at), out.len() as u64, StepKind::Header(StepField::Token, token as u32));
        }
        at += 1;
        let mut literals = (token & 3) as usize;
        if literals == 0 {
            let start = at;
            literals = byte(&mut at)? as usize + 3;
            if !coarse {
                b.push(bit(start), out.len() as u64, StepKind::Header(StepField::LengthExtra, literals as u32 - 1));
            }
        }
        literals -= 1;
        let mut len = (token >> 4) as usize;
        if len == 15 {
            let start = at;
            len = byte(&mut at)? as usize + 15;
            if !coarse {
                b.push(bit(start), out.len() as u64, StepKind::Header(StepField::LengthExtra, len as u32 + 2));
            }
        }
        if at + literals > data.len() {
            return Err(Refusal::Failed);
        }
        if out.len() + literals > CAP_BYTES {
            return Err(Refusal::TooLargeOutput);
        }
        if literals > 0 {
            if !coarse {
                b.push(bit(at), out.len() as u64, StepKind::Stored);
            }
            out.extend_from_slice(&data[at..at + literals]);
            at += literals;
        }
        if len == 0 {
            continue;
        }
        let offset_at = at;
        let lo = byte(&mut at)? as usize;
        let hi = match (token >> 2) & 3 {
            3 => byte(&mut at)? as usize,
            h => h as usize,
        };
        let dist = lo | hi << 8;
        if !coarse {
            b.push(bit(offset_at), out.len() as u64, StepKind::Header(StepField::Offset, dist as u32));
        }
        let len = len + 2;
        if dist == 0 || dist > out.len() {
            return Err(Refusal::Failed);
        }
        if out.len() + len > CAP_BYTES {
            return Err(Refusal::TooLargeOutput);
        }
        let from = out.len() - dist;
        let was = out.len();
        for k in 0..len {
            let v = out[from + k];
            out.push(v);
        }
        if !coarse {
            b.push(bit(at), was as u64, StepKind::Match { len: len as u32, dist: dist as u32 });
        }
    }
    b.close_block(data.len() as u64 * 8, out.len() as u64, BlockKind::Sequences, true);
    b.finish_at(data.len() as u64 * 8, out.len() as u64);
    Ok((out, b.done()))
}

/// A `packbits` run, unpacked, with a step for every control byte and every
/// run of bytes after one.
///
/// A control byte is signed. From 0 to 127 it is followed by that many bytes
/// and one more, copied as they are; from -1 to -128 it is followed by one
/// byte, written that many times. Apple's PackBits, which this is named
/// after, writes a byte one time more than that and skips -128; IAR's does
/// neither.
///
/// The byte after a negative control byte is traced as read without a name:
/// it is not a literal, since one byte in the run makes many in the output,
/// and not a match, since nothing is copied from earlier output.
pub fn packbits(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    let mut b = TraceBuilder::default();
    let mut out: Vec<u8> = Vec::new();
    let mut at = 0usize;
    let mut coarse = false;
    let bit = |at: usize| at as u64 * 8;
    b.open_block(0, 0);
    while at < data.len() {
        if !coarse && b.over_budget() {
            coarse = true;
            b.coarsen();
            b.push(bit(at), out.len() as u64, StepKind::Opaque);
        }
        let control = data[at] as i8;
        if !coarse {
            b.push(bit(at), out.len() as u64, StepKind::Header(StepField::Token, data[at] as u32));
        }
        at += 1;
        if control >= 0 {
            let n = control as usize + 1;
            if at + n > data.len() {
                return Err(Refusal::Failed);
            }
            if out.len() + n > CAP_BYTES {
                return Err(Refusal::TooLargeOutput);
            }
            if !coarse {
                b.push(bit(at), out.len() as u64, StepKind::Stored);
            }
            out.extend_from_slice(&data[at..at + n]);
            at += n;
        } else {
            let n = -(control as isize) as usize;
            let &v = data.get(at).ok_or(Refusal::Failed)?;
            if out.len() + n > CAP_BYTES {
                return Err(Refusal::TooLargeOutput);
            }
            if !coarse {
                b.push(bit(at), out.len() as u64, StepKind::Opaque);
            }
            out.resize(out.len() + n, v);
            at += 1;
        }
    }
    b.close_block(data.len() as u64 * 8, out.len() as u64, BlockKind::Sequences, true);
    b.finish_at(data.len() as u64 * 8, out.len() as u64);
    Ok((out, b.done()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_lz77_token_with_one_literal_and_no_match() {
        // Literal count 2 in the token is one literal; match length 0.
        let (out, _) = lz77(&[0x02, b'A']).unwrap();
        assert_eq!(out, b"A");
    }

    #[test]
    fn an_lz77_match_overlapping_itself_repeats_a_byte() {
        // One literal, then a match of 1 + 2 = 3 bytes from 1 back; the
        // distance's high bits are 0 in the token.
        let (out, _) = lz77(&[0x12, b'z', 0x01]).unwrap();
        assert_eq!(out, b"zzzz");
    }

    #[test]
    fn an_lz77_literal_count_and_match_length_past_the_token() {
        // Literal count from the next byte (0 + 2 = 2 literals), match length
        // from the byte after that (2 + 15 + 2 = 19), distance 2 with its high
        // byte written out because the token's middle bits are 3.
        let (out, _) = lz77(&[0xfc, 0x00, 0x02, b'a', b'b', 0x02, 0x00]).unwrap();
        assert_eq!(out, b"ab".repeat(11)[..21].to_vec());
    }

    #[test]
    fn an_lz77_match_reaching_before_the_start_is_refused() {
        assert_eq!(lz77(&[0x12, b'z', 0x05]).err(), Some(Refusal::Failed));
    }

    #[test]
    fn packbits_copies_and_repeats() {
        // Copy 3 bytes, then write 0xff five times, then copy 1 byte.
        let (out, _) = packbits(&[0x02, 1, 2, 3, 0xfb, 0xff, 0x00, 9]).unwrap();
        assert_eq!(out, vec![1, 2, 3, 0xff, 0xff, 0xff, 0xff, 0xff, 9]);
    }

    #[test]
    fn packbits_cut_off_after_a_control_byte_is_refused() {
        assert_eq!(packbits(&[0x03, 1, 2]).err(), Some(Refusal::Failed));
        assert_eq!(packbits(&[0xfb]).err(), Some(Refusal::Failed));
    }
}
