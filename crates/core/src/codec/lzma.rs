//! LZMA, and the lzip member that carries one.
//!
//! An LZMA1 stream is range-coded bits and nothing else: no magic, no version,
//! no length, and no statement of how it was packed. Everything a decoder has
//! to know before it can read the first bit comes from whatever wrapped it,
//! which is why the run handed here is the whole member and not the stream
//! inside it.
//!
//! What a decoder needs is three numbers and a size. lzip fixes the three:
//! three literal context bits, no literal position bits, two position bits,
//! which is the byte 0x5D wherever an LZMA header writes them. The dictionary
//! size is the one thing lzip's own header byte says. How much comes out is
//! not written anywhere before the stream, so the header says it is unknown
//! and the stream is read to its end-of-stream marker, which lzip always
//! writes. lzip's trailer does give the size, but it sits *after* the stream:
//! reading it would mean measuring backwards to answer a question the format
//! already answers forwards.
//!
//! Those thirteen bytes are the "alone" header `lzma` wrote before xz existed,
//! and they are what `lzma-rs` reads. Building one and handing it the stream
//! is the whole of this: the bytes in front of it are lzip's, and the bytes
//! after it are the ones a `.lzma` file would have started with.

use std::io::Read;

use crate::codec::{frames, Refusal, Trace, CAP_BYTES};

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

/// An lzip member: four bytes of magic, a version, a dictionary size, the raw
/// LZMA1 stream, and a trailer saying what came out.
pub fn lzip(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    if data.len() < HEADER + TRAILER || !data.starts_with(MAGIC) || data[4] != VERSION {
        return Err(Refusal::Failed);
    }
    let Some(dict) = dict_size(data[5]) else { return Err(Refusal::Failed) };
    // Everything between the header and the trailer. The stream has no length
    // on it, so this is the only way to say where it stops.
    let stream = &data[HEADER..data.len() - TRAILER];

    // The thirteen bytes an LZMA1 stream never carries: how it was packed,
    // how large a dictionary it wants, and how much comes out. All ones in the
    // last eight is how the header says the size is unknown and the stream
    // ends at a marker, which is what lzip writes.
    let mut header = [0xffu8; 13];
    header[0] = PROPS;
    header[1..5].copy_from_slice(&dict.to_le_bytes());

    let mut input = (&header[..]).chain(stream);
    let mut out = Vec::new();
    lzma_rs::lzma_decompress(&mut input, &mut out).map_err(|_| Refusal::Failed)?;
    if out.len() > CAP_BYTES {
        return Err(Refusal::TooLarge);
    }
    // One step over the whole run. lzma-rs hands back the bytes and says
    // nothing about which bits of the stream made which of them, and a map
    // drawn from a guess would be worse than no map: this is the honest shape
    // until an LZMA decoder of our own reads the symbols.
    let n = out.len();
    Ok((out, frames::whole(data.len(), n)))
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
mod tests {
    use super::*;

    #[test]
    fn the_top_three_bits_take_sixteenths_off_the_power_below_them() {
        // 8 MiB with nothing off it, which is what lzip's default level packs
        // with, and the byte every sample here carries.
        assert_eq!(dict_size(0x17), Some(1 << 23));
        // The same power with seven sixteenths off, which lands below the
        // power beneath it.
        assert_eq!(dict_size(0xf7), Some((1 << 23) - (1 << 23) / 16 * 7));
        // The smallest size, whose top bits mean nothing.
        assert_eq!(dict_size(0xec), Some(1 << 12));
        // Powers the format does not have.
        assert_eq!(dict_size(0x0b), None);
        assert_eq!(dict_size(0x1e), None);
    }

    #[test]
    fn bytes_that_are_not_a_member_are_refused_rather_than_sliced_up() {
        assert_eq!(lzip(b"not an lzip member at all, no").err(), Some(Refusal::Failed));
        // A member of the right shape whose version is one nothing here reads.
        let mut short = b"LZIP\x00\x17".to_vec();
        short.extend_from_slice(&[0; 20]);
        assert_eq!(lzip(&short).err(), Some(Refusal::Failed));
        // Too short to hold a header and a trailer, let alone a stream.
        assert_eq!(lzip(b"LZIP\x01\x17").err(), Some(Refusal::Failed));
    }
}
