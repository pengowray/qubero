//! NASA CDF's run-length encoding, which counts nothing but zeroes.
//!
//! The simplest compression in the library and the one a CDF written by IDL's
//! `cdf_create` reaches for: a zero byte is an escape, the byte after it is one
//! less than how many zeroes to write, and every other byte is itself. So a
//! long stretch of zeroes costs two bytes and everything else costs what it
//! was, which is why a file of mostly-empty variables shrinks by half and a
//! file of measurements does not shrink at all.
//!
//! Two bytes for a run means a run reaches 256 and no further, so a longer
//! stretch is written as several tokens in a row. Nothing in the stream says
//! how much comes out; the record that holds it does, and the template checks
//! what came out against it.
//!
//! **What the trace says.** One step over the whole run. A zero run is not a
//! copy from further back and not a literal, and there is no step kind that
//! says "this many zeroes"; saying `match` with a distance of nought would put
//! a back-reference in front of a reader where the format has none. So the
//! trace is one step, the way bzip2's is, and the honest thing it says is that
//! these bytes came out of that run.

use crate::codec::{frames, Refusal, Trace, CAP_BYTES};

/// One RLE.0 stream, which for a compressed CDF is the whole of the file after
/// its eight-byte signature.
///
/// Fails on a stream that ends on an escape with no count after it: a run of
/// zeroes half written is a file cut off, and guessing at the count would put
/// bytes in front of a reader that nothing wrote.
pub fn stream(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    let mut out: Vec<u8> = Vec::with_capacity(data.len());
    let mut at = 0usize;
    while at < data.len() {
        let b = data[at];
        if b != 0 {
            out.push(b);
            at += 1;
            continue;
        }
        let Some(&count) = data.get(at + 1) else { return Err(Refusal::Failed) };
        let run = count as usize + 1;
        if out.len() + run > CAP_BYTES {
            return Err(Refusal::TooLarge);
        }
        out.resize(out.len() + run, 0);
        at += 2;
    }
    let trace = frames::whole(data.len(), out.len());
    Ok((out, trace))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_byte_escapes_the_count_that_follows_it() {
        // Six zeroes, then two bytes of their own: the front of the descriptor
        // record a compressed CDF opens with, which is a size of 0x138.
        let (out, _) = stream(&[0x00, 0x05, 0x01, 0x38]).unwrap();
        assert_eq!(out, vec![0, 0, 0, 0, 0, 0, 0x01, 0x38]);
    }

    #[test]
    fn a_count_of_255_is_a_run_of_256() {
        let (out, _) = stream(&[0x00, 0xff]).unwrap();
        assert_eq!(out.len(), 256);
        assert!(out.iter().all(|b| *b == 0));
    }

    #[test]
    fn a_stream_with_no_zeroes_in_it_comes_out_as_it_went_in() {
        let (out, _) = stream(b"CDF").unwrap();
        assert_eq!(out, b"CDF".to_vec());
    }

    /// An escape with nothing after it is a file cut off mid-run, and the
    /// count it wanted is not somewhere else to be found.
    #[test]
    fn an_escape_at_the_very_end_is_refused() {
        assert_eq!(stream(&[0x41, 0x00]).err(), Some(Refusal::Failed));
    }
}
