//! W4V, the compressed recordings Wildlife Acoustics detectors write.
//!
//! The container is an ordinary RIFF/WAVE file; the only thing that says W4V is
//! the format tag 0x5741 ("AW") in `fmt `. The `data` chunk is then a run of
//! fixed-size blocks rather than samples: a block carries one predictor value,
//! one scale, five bytes nobody has explained, and 512 codes packed six bits
//! each, MSB first. A sample is `predictor + code * scale`, so this is
//! block-floating-point rather than a running difference.
//!
//! Only the six-bit flavour is described here, which is what
//! `nBlockAlign = 392` means and what the recordings in hand use. The wider
//! ones would need the code width taken from `fmt `, and a field cannot yet
//! read one from a sibling chunk.
//!
//! None of this is from a specification. The layout follows the decoder in the
//! batchi project, which was reverse-engineered and is corroborated by the
//! `WA|Song Meter|Compression:W4V-6` line those files carry in their GUANO.

use super::wav;
use crate::template::{Endian::*, Expr as E, Template, Ty as T, Until};

/// Codes per block, and the bits each one takes in the six-bit flavour.
const BLOCK_SAMPLES: i128 = 512;
const CODE_BITS: u32 = 6;

pub fn w4v() -> Template {
    wav::riff("w4v", wav::chunk_body(Some(T::repeat(block(), Until::End))))
}

fn block() -> T {
    T::structure(
        "Block",
        vec![
            ("predictor", T::Int { bits: 16, endian: Little }),
            ("scale", T::u8()),
            // Not zero in real files, and not documented as reserved either:
            // nobody outside Wildlife Acoustics knows what these five carry.
            ("unknown", T::bytes(E::lit(5))),
            ("codes", T::array(T::Int { bits: CODE_BITS, endian: Big }, E::lit(BLOCK_SAMPLES))),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// One block: a predictor, a scale, and 512 six-bit codes.
    fn block_bytes(predictor: i16, scale: u8, codes: &[i8]) -> Vec<u8> {
        let mut v = predictor.to_le_bytes().to_vec();
        v.push(scale);
        v.extend_from_slice(&[0xee, 0xfa, 0, 0, 0]);
        let mut bits: Vec<u8> = Vec::new();
        for i in 0..BLOCK_SAMPLES as usize {
            let code = codes.get(i).copied().unwrap_or(0) as u8 & 0x3f;
            for b in (0..CODE_BITS).rev() {
                bits.push((code >> b) & 1);
            }
        }
        for byte in bits.chunks(8) {
            v.push(byte.iter().fold(0u8, |acc, b| (acc << 1) | b));
        }
        v
    }

    fn file() -> Vec<u8> {
        let mut fmt = Vec::new();
        fmt.extend_from_slice(&0x5741u16.to_le_bytes());
        fmt.extend_from_slice(&1u16.to_le_bytes());
        fmt.extend_from_slice(&256_000u32.to_le_bytes());
        fmt.extend_from_slice(&195_488u32.to_le_bytes());
        fmt.extend_from_slice(&392u16.to_le_bytes());
        fmt.extend_from_slice(&16u16.to_le_bytes());
        fmt.extend_from_slice(&0u16.to_le_bytes());

        let data = block_bytes(100, 2, &[0, 1, -1, 31, -32]);

        let mut body = b"WAVE".to_vec();
        body.extend_from_slice(b"fmt ");
        body.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
        body.extend_from_slice(&fmt);
        body.extend_from_slice(b"data");
        body.extend_from_slice(&(data.len() as u32).to_le_bytes());
        body.extend_from_slice(&data);

        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    #[test]
    fn data_reads_as_blocks_of_six_bit_codes() {
        let d = Document::new(MemSource(file()));
        let mut ev = Evaluator::new(w4v());
        assert_eq!(ev.node(&d, &[3]).unwrap().child_count, 2);

        // The format tag is what marks the file as W4V.
        assert_eq!(
            ev.node(&d, &[3, 0, 2, 0]).unwrap().value,
            Value::Enum { raw: 0x5741, name: Some("w4v".into()), hex: true }
        );

        let blocks = ev.node(&d, &[3, 1, 2]).unwrap();
        assert_eq!(blocks.child_count, 1);
        let block = ev.node(&d, &[3, 1, 2, 0]).unwrap();
        assert_eq!(block.size_bits, 392 * 8);
        assert_eq!(ev.node(&d, &[3, 1, 2, 0, 0]).unwrap().value, Value::Int(100));
        assert_eq!(ev.node(&d, &[3, 1, 2, 0, 1]).unwrap().value, Value::UInt(2));

        // The codes are signed and packed six bits at a time, MSB first, so the
        // third one straddles the first and second bytes of the run.
        let codes = ev.node(&d, &[3, 1, 2, 0, 3]).unwrap();
        assert_eq!(codes.child_count, BLOCK_SAMPLES as u64);
        let mut read = |i: usize| ev.node(&d, &[3, 1, 2, 0, 3, i]).unwrap().value;
        assert_eq!(read(0), Value::Int(0));
        assert_eq!(read(1), Value::Int(1));
        assert_eq!(read(2), Value::Int(-1));
        assert_eq!(read(3), Value::Int(31));
        assert_eq!(read(4), Value::Int(-32));
        assert_eq!(read(5), Value::Int(0));
    }
}
