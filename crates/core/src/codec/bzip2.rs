//! bzip2: a Burrows-Wheeler transform, a move-to-front pass and Huffman codes.
//!
//! The run handed here is the whole stream, from its `BZh` onwards, and not
//! one block. bzip2 packs its blocks to the bit: only the first starts on a
//! byte boundary, and where the second begins is only known by decoding the
//! first. So there is no run smaller than the stream that anything else in
//! Qubero can address, and the stream is what goes to the decoder.
//!
//! The trace is one step over all of it. `bzip2-rs` gives bytes and no shape:
//! it says nothing about where a block ended, and the block boundaries are the
//! only shape a bzip2 stream has. A map guessed from the byte positions of the
//! block magic would be right one time in eight, so this says what it knows,
//! which is that these bytes made those bytes. See `frames::whole`.
//!
//! One stream, not a file of them. A `.bz2` may hold several streams end to
//! end, which is what `pbzip2` writes and what concatenating two of them
//! gives, and the `bzip2` tool reads all of them. This reads the first and
//! stops at its end-of-stream marker, so the bytes after that are not opened
//! and nothing says they were skipped. Fixing it means knowing how many bytes
//! the first stream took, which this crate does not report; the low-level
//! `Decoder` keeps the number and does not expose it.

use std::io::Read;

use crate::codec::{frames, Refusal, Trace, CAP_BYTES};

/// A whole bzip2 stream, from its `BZh` onwards.
///
/// The decoder checks every block's CRC-32 against the bytes it produced, so a
/// stream whose contents were corrupted refuses rather than handing back the
/// wrong bytes. It reads the whole-stream CRC without checking it.
pub fn stream(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    let mut reader = bzip2_rs::DecoderReader::new(data);
    // One byte past the cap, so a stream that only just fits is told from one
    // that does not.
    let mut out = Vec::new();
    match reader.by_ref().take(CAP_BYTES as u64 + 1).read_to_end(&mut out) {
        Ok(_) if out.len() > CAP_BYTES => Err(Refusal::TooLarge),
        Ok(_) => {
            let n = out.len();
            Ok((out, frames::whole(data.len(), n)))
        }
        Err(_) => Err(Refusal::Failed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real stream over `data`, written by the other implementation of the
    /// format. `level` is the block size in hundreds of kilobytes, so a low
    /// one over enough data is how a multi-block stream is had.
    fn pack(data: &[u8], level: u32) -> Vec<u8> {
        use std::io::Write;
        let mut e = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::new(level));
        e.write_all(data).expect("writes");
        e.finish().expect("finishes")
    }

    #[test]
    fn a_stream_comes_back_as_what_went_in() {
        let packed = pack(b"bzip2 bzip2 bzip2 bzip2", 9);
        assert_eq!(stream(&packed).expect("a stream").0, b"bzip2 bzip2 bzip2 bzip2");
    }

    /// `bzip2` given nothing to compress writes the header, the end-of-stream
    /// marker and a combined CRC of zero, and no block at all.
    #[test]
    fn a_stream_that_compressed_nothing_opens_into_nothing() {
        let packed = pack(b"", 9);
        assert_eq!(packed, b"BZh9\x17\x72\x45\x38\x50\x90\x00\x00\x00\x00");
        let (out, trace) = stream(&packed).expect("a stream");
        assert!(out.is_empty());
        trace.check_tiles().expect("tiles");
    }

    #[test]
    fn bytes_that_are_not_a_stream_are_refused_rather_than_guessed_at() {
        assert_eq!(stream(b"not compressed").err(), Some(Refusal::Failed));
        // The magic alone, with no block behind it.
        assert_eq!(stream(b"BZh9").err(), Some(Refusal::Failed));
        assert_eq!(stream(b"").err(), Some(Refusal::Failed));
    }

    /// A block whose bytes were changed under it. The decoder sums what it
    /// produced against the CRC the block carries, so this refuses rather than
    /// handing back something that is not what was compressed.
    #[test]
    fn a_block_that_does_not_match_its_own_checksum_is_refused() {
        let mut packed = pack(&b"bzip2 ".repeat(400), 9);
        let last = packed.len() - 6;
        packed[last] ^= 0xff;
        assert_eq!(stream(&packed).err(), Some(Refusal::Failed));
    }

    /// A stream of several blocks, which is what a low block size over enough
    /// data gives. Past the first block nothing is byte-aligned, which is the
    /// case this decoder is here for.
    #[test]
    fn a_stream_of_several_blocks_comes_back_whole() {
        // No run of four repeated bytes anywhere, so bzip2's run-length pass
        // in front of the blocking does not shrink this before it is split.
        let text = "the quick brown fox jumps over the lazy dog, and again: ".repeat(4500);
        assert!(text.len() > 250_000);
        let packed = pack(text.as_bytes(), 1);
        let (out, trace) = stream(&packed).expect("a stream");
        assert_eq!(out.len(), text.len(), "the stream came back a different length");
        assert!(out == text.as_bytes(), "the stream came back as something else");
        trace.check_tiles().expect("tiles");
    }

    /// What this does *not* read, written down so the next person finds it
    /// here rather than in a file. Both of these come back `Ok` with the first
    /// stream's bytes and no note saying anything was left over.
    #[test]
    fn a_second_stream_after_the_first_is_not_read() {
        let one = pack(b"first stream", 9);

        // `pbzip2` writes a file like this, and so does `cat a.bz2 b.bz2`.
        // The `bzip2` tool reads both halves; this reads the first.
        let mut both = one.clone();
        both.extend_from_slice(&pack(b"second stream", 9));
        assert_eq!(stream(&both).expect("a stream").0, b"first stream");

        // The same stop, reached the same way: whatever follows the
        // end-of-stream marker is not looked at.
        let mut tail = one.clone();
        tail.extend_from_slice(b"trailing garbage");
        assert_eq!(stream(&tail).expect("a stream").0, b"first stream");
    }

    /// The trace tiles the run whatever came out of it: one step, from the
    /// first bit of the stream to the last byte it made.
    #[test]
    fn the_trace_is_one_step_over_the_whole_stream() {
        let packed = pack(b"bzip2 bzip2 bzip2 bzip2", 9);
        let (out, trace) = stream(&packed).expect("a stream");
        trace.check_tiles().expect("tiles");
        assert_eq!(trace.len(), 1);
        assert_eq!(trace.in_bits(), packed.len() as u64 * 8);
        assert_eq!(trace.out_bytes(), out.len() as u64);
    }
}
