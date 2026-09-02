//! Unpacking a compressed run so the fields inside it can be read.
//!
//! A file that holds a compressed stream holds structure the reader cannot see:
//! a ROOT record is a nine-byte block header and then a zlib stream, and
//! everything the record is *for* is on the other side of it. Reading the run
//! as `bytes[3824]` is honest and useless.
//!
//! So a template may say what a run is compressed with, and the reading opens
//! it. The compressed bytes stay exactly where they are and stay exactly as
//! long as they are; what comes out of them is a second address space, and the
//! fields declared over it count from its own start. See
//! [`Ty::Decoded`](crate::template::Ty::Decoded).
//!
//! Every decoder here is pure Rust and builds for wasm32. Nothing streams: a
//! stream is opened whole or not at all, which is why there is a cap.

/// The largest a decoded stream may come to. Past this the run is left as the
/// bytes it is and the node says why: a zip bomb is one line in a file and
/// gigabytes in memory, and a hex editor that opens one has stopped being a
/// hex editor.
pub const CAP_BYTES: usize = 64 * 1024 * 1024;

/// What a run is compressed with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    /// RFC 1950: two header bytes, deflate, an Adler-32.
    Zlib,
    /// RFC 1951 on its own, with nothing wrapped round it.
    Deflate,
    Zstd,
    /// One LZ4 block, with no frame header and no length in front of it. What
    /// ROOT hands to LZ4 and what an LZ4 frame's blocks hold.
    Lz4Block,
    Xz,
}

impl Codec {
    pub fn as_str(self) -> &'static str {
        match self {
            Codec::Zlib => "zlib",
            Codec::Deflate => "deflate",
            Codec::Zstd => "zstd",
            Codec::Lz4Block => "lz4",
            Codec::Xz => "xz",
        }
    }
}

/// Why a run was left as bytes. A kind rather than a sentence: the core says
/// what happened and the interface says it in words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The run, or what it would come to, is past [`CAP_BYTES`].
    TooLarge,
    /// The decoder would not read it.
    Failed,
    /// The run does not start on a byte, and no decoder reads half a byte.
    Unaligned,
}

impl Refusal {
    /// The word the interface looks the message up by.
    pub fn as_str(self) -> &'static str {
        match self {
            Refusal::TooLarge => "too-large",
            Refusal::Failed => "failed",
            Refusal::Unaligned => "unaligned",
        }
    }
}

/// Open a compressed run. `data` is the whole of it.
pub fn decode(codec: Codec, data: &[u8]) -> Result<Vec<u8>, Refusal> {
    if data.len() > CAP_BYTES {
        return Err(Refusal::TooLarge);
    }
    let out = match codec {
        Codec::Zlib => miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(data, CAP_BYTES)
            .map_err(|e| limit_or_failed(e.status)),
        Codec::Deflate => miniz_oxide::inflate::decompress_to_vec_with_limit(data, CAP_BYTES)
            .map_err(|e| limit_or_failed(e.status)),
        Codec::Zstd => zstd(data),
        Codec::Lz4Block => lz4_block(data),
        Codec::Xz => xz(data),
    }?;
    if out.len() > CAP_BYTES {
        return Err(Refusal::TooLarge);
    }
    Ok(out)
}

fn limit_or_failed(status: miniz_oxide::inflate::TINFLStatus) -> Refusal {
    match status {
        miniz_oxide::inflate::TINFLStatus::HasMoreOutput => Refusal::TooLarge,
        _ => Refusal::Failed,
    }
}

/// Zstandard, read a frame at a time. A file compressed by `zstd` is one
/// frame; ROOT writes one per block. Concatenated frames are read through to
/// the end, which is what the format says a decoder does.
fn zstd(data: &[u8]) -> Result<Vec<u8>, Refusal> {
    use std::io::Read;
    let mut decoder = ruzstd::decoding::StreamingDecoder::new(data).map_err(|_| Refusal::Failed)?;
    // One byte past the cap, so a stream that only just fits is told from one
    // that does not.
    let mut out = Vec::new();
    match decoder.by_ref().take(CAP_BYTES as u64 + 1).read_to_end(&mut out) {
        Ok(_) if out.len() > CAP_BYTES => Err(Refusal::TooLarge),
        Ok(_) => Ok(out),
        Err(_) => Err(Refusal::Failed),
    }
}

/// One LZ4 block. Nothing in the block says how long it unpacks to, and the
/// header that would is not part of it, so the size is found by trying: four
/// times the compressed run, doubling until it fits. Asking for the cap
/// outright would allocate 64 MiB for every block in the file.
fn lz4_block(data: &[u8]) -> Result<Vec<u8>, Refusal> {
    let mut room = (data.len().saturating_mul(4)).clamp(1024, CAP_BYTES);
    loop {
        match lz4_flex::block::decompress(data, room) {
            Ok(out) => return Ok(out),
            Err(_) if room >= CAP_BYTES => return Err(Refusal::TooLarge),
            Err(_) => room = room.saturating_mul(2).min(CAP_BYTES),
        }
    }
}

/// A whole xz stream, header through footer. The LZMA2 inside one block is a
/// step of a decoder's state and does not stand on its own, which is why the
/// template opens the stream and not the block.
fn xz(data: &[u8]) -> Result<Vec<u8>, Refusal> {
    let mut input = std::io::BufReader::new(data);
    let mut out = Vec::new();
    match lzma_rs::xz_decompress(&mut input, &mut out) {
        Ok(()) if out.len() > CAP_BYTES => Err(Refusal::TooLarge),
        Ok(()) => Ok(out),
        Err(_) => Err(Refusal::Failed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zlib_stream_comes_back_as_what_went_in() {
        let packed = miniz_oxide::deflate::compress_to_vec_zlib(b"hello hello hello", 6);
        assert_eq!(decode(Codec::Zlib, &packed).unwrap(), b"hello hello hello");
    }

    #[test]
    fn raw_deflate_has_no_header_on_it() {
        let packed = miniz_oxide::deflate::compress_to_vec(b"deflate me", 6);
        assert_eq!(decode(Codec::Deflate, &packed).unwrap(), b"deflate me");
        // The same bytes read as zlib are not a zlib stream.
        assert_eq!(decode(Codec::Zlib, &packed), Err(Refusal::Failed));
    }

    #[test]
    fn an_lz4_block_is_sized_by_trying_since_nothing_in_it_says() {
        let long = "lz4 block ".repeat(5000);
        let packed = lz4_flex::block::compress(long.as_bytes());
        assert_eq!(decode(Codec::Lz4Block, &packed).unwrap(), long.as_bytes());
    }

    #[test]
    fn bytes_that_are_not_a_stream_are_refused_rather_than_guessed_at() {
        assert_eq!(decode(Codec::Zlib, b"not compressed"), Err(Refusal::Failed));
        assert_eq!(decode(Codec::Zstd, b"not compressed"), Err(Refusal::Failed));
        assert_eq!(decode(Codec::Xz, b"not compressed"), Err(Refusal::Failed));
    }

}
