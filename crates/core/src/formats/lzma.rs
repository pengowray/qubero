//! A raw LZMA file, the format `lzma_alone` writes and `.lzma` names: a
//! thirteen-byte header and then LZMA1 to the end.
//!
//! Nothing marks the front of one. The header is a settings byte, a dictionary
//! size and what comes out, and every one of those could be any bytes at all.
//! What recognises one is the four of them agreeing: the settings byte is the
//! `5d` every writer of this format emits, the dictionary is a size a coder
//! would use, the output size is either stated or the eight bytes that say it
//! is not, and the first byte of an LZMA1 stream is a nought the encoder
//! writes and the decoder throws away.
//!
//! The settings byte is held to that one value rather than to the 225 the
//! field allows, and this is why: a PlayStation texture opens `10 00 00 00`
//! and its next twelve bytes pass every other test here. `5d` is what xz
//! `--format=lzma`, Python's `FORMAT_ALONE` and 7-Zip all write, and a file
//! with other settings is still read by naming the template or by its
//! extension. Claiming one file wrongly costs more than missing one.
//!
//! The settings byte is `(pb * 5 + lp) * 9 + lc`: how many low bits of the
//! position the coder keys its probabilities on, how many of the position for
//! a literal, and how many bits of the byte before. A decoder needs all three
//! and the stream carries none of them, which is why they are here.
//!
//! xz and lzip wrap the same coder in a container with a magic number and a
//! checksum. This one has neither, so a file that will not unpack is a file
//! that was never this, and the run says so where it stands.

use crate::template::{Endian::Little, Expr as E, Packing, Template, Ty as T};

/// The smallest and largest dictionary a coder writes, which is 4 KiB to
/// 1 GiB. LZMA's own minimum is 4 KiB and its maximum is 1.5 GiB; nothing
/// writes the top of that range.
const LEAST_DICT: u32 = 1 << 12;
const MOST_DICT: u32 = 1 << 30;
/// The settings byte every writer of this format emits: `lc` 3, `lp` 0 and
/// `pb` 2. The field holds 225 different values and the template reads
/// whichever is there; only this one is evidence that a file is one of these.
const WRITTEN_PROPS: u8 = 0x5d;
/// What the size field says when the writer did not know: all ones. The stream
/// then ends at a marker only the decoder can see.
const UNKNOWN_SIZE: i128 = u64::MAX as i128;
/// The largest stated output size this will believe, which is a terabyte. A
/// number past that in a file with no magic number is not a size.
const MOST_UNPACKED: i128 = 1 << 40;
/// How long the header is, and so where the stream starts.
const HEADER: usize = 13;

pub fn lzma() -> Template {
    let packing = |unpacked| Packing::Lzma1 { props: E::field("props"), dict_size: E::field("dict_size"), unpacked };
    // What comes out, which the decoder is told when the header states it and
    // finds for itself when it does not. Two arms rather than one, because a
    // decoder handed all ones as a length would read for as long as the bytes
    // lasted instead of stopping at the marker.
    let stream = T::switch(
        E::field("uncompressed_size").equals(E::lit(UNKNOWN_SIZE)),
        vec![(1, T::decoded_as(E::Remaining, packing(None), super::decoded_text()))],
        T::decoded_as(E::Remaining, packing(Some(E::field("uncompressed_size"))), super::decoded_text()),
    );
    Template::new(
        "lzma",
        T::structure(
            "LzmaFile",
            vec![
                // The three coder settings in one byte. Left as the number it
                // is: a decoder wants the byte, and the three numbers in it
                // are `props % 9`, `props / 9 % 5` and `props / 45`.
                ("props", T::u8()),
                ("dict_size", T::u32(Little)),
                // All ones for a stream that ends at its marker, which is what
                // a writer that did not know the size ahead of time writes.
                ("uncompressed_size", T::u64(Little)),
                ("compressed", stream),
            ],
        ),
    )
}

/// Whether a dictionary size is one a coder would have written. Every encoder
/// picks a power of two or three halves of one, which is the whole of what the
/// `-0` to `-9` presets and 7-Zip's own sizes are, and four bytes that happen
/// to hold a number in the right range are not a header.
fn written_dict(dict: u32) -> bool {
    dict.is_power_of_two() || (dict % 3 == 0 && (dict / 3).is_power_of_two())
}

/// Whether these bytes open a raw LZMA file: the header's four fields
/// agreeing, and the nought an LZMA1 stream always begins with.
pub fn is_lzma(head: &[u8]) -> bool {
    let Some(raw) = head.get(..HEADER + 1) else { return false };
    if raw[0] != WRITTEN_PROPS {
        return false;
    }
    let dict = u32::from_le_bytes(raw[1..5].try_into().expect("four bytes"));
    if !(LEAST_DICT..=MOST_DICT).contains(&dict) || !written_dict(dict) {
        return false;
    }
    let size = u64::from_le_bytes(raw[5..13].try_into().expect("eight bytes")) as i128;
    if size != UNKNOWN_SIZE && !(1..=MOST_UNPACKED).contains(&size) {
        return false;
    }
    // The range coder's first byte. The encoder shifts a nought out of it
    // before anything else, and every decoder ignores whatever is there, so
    // every LZMA1 stream ever written begins with this.
    raw[HEADER] == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, eval::Evaluator, source::MemSource};

    /// `python3 -c "import lzma; ..."` with the alone format and the default
    /// settings, over the word below.
    fn alone(text: &[u8]) -> Vec<u8> {
        let mut out = vec![0x5d];
        out.extend_from_slice(&(1u32 << 23).to_le_bytes());
        out.extend_from_slice(&u64::MAX.to_le_bytes());
        // Not a real stream; the header is what these tests are about.
        out.extend_from_slice(&[0, 0, 0, 0, 0]);
        out.extend_from_slice(text);
        out
    }

    #[test]
    fn the_header_is_four_fields_and_the_stream_is_the_rest() {
        let bytes = alone(b"whatever");
        let d = Document::new(MemSource(bytes.clone()));
        let mut e = Evaluator::new(lzma());
        assert_eq!(e.node(&d, &[0]).unwrap().value.as_int(), Some(0x5d));
        assert_eq!(e.node(&d, &[1]).unwrap().value.as_int(), Some(1 << 23));
        assert_eq!(e.node(&d, &[2]).unwrap().value.as_int(), Some(UNKNOWN_SIZE));
        let run = e.node(&d, &[3]).unwrap();
        assert_eq!(run.offset_bits, HEADER as u64 * 8);
        assert_eq!(run.size_bits, (bytes.len() - HEADER) as u64 * 8);
    }

    #[test]
    fn the_four_fields_have_to_agree_before_a_file_is_one() {
        assert!(is_lzma(&alone(b"")));
        // Any settings byte but the one every writer emits, which is how a
        // PlayStation texture opening `10 00 00 00` stays a texture.
        for props in [0x10u8, 0x00, 0x5c, 0xff] {
            let mut wrong = alone(b"");
            wrong[0] = props;
            assert!(!is_lzma(&wrong), "settings byte {props:#x}");
        }
        assert!(!is_lzma(b"\x10\x00\x00\x00\x02\x00\x00\x00\xec\x01\x00\x00\x00\x00"));
        // A dictionary smaller than the coder's own minimum, and one larger
        // than anything writes.
        for dict in [0u32, 1 << 11, 1 << 31] {
            let mut wrong = alone(b"");
            wrong[1..5].copy_from_slice(&dict.to_le_bytes());
            assert!(!is_lzma(&wrong), "dictionary of {dict}");
        }
        // A dictionary in the range and not a size any coder writes, which
        // is what four bytes of something else look like.
        let mut wrong = alone(b"");
        wrong[1..5].copy_from_slice(&6_780_269u32.to_le_bytes());
        assert!(!is_lzma(&wrong));
        // Three halves of a power of two, which 7-Zip writes.
        let mut sevenzip = alone(b"");
        sevenzip[1..5].copy_from_slice(&(3u32 << 22).to_le_bytes());
        assert!(is_lzma(&sevenzip));
        // A size that is neither stated nor said to be unknown, and a stated
        // size of nothing, which is not a file anybody compressed.
        for size in [1u64 << 50, 0] {
            let mut wrong = alone(b"");
            wrong[5..13].copy_from_slice(&size.to_le_bytes());
            assert!(!is_lzma(&wrong), "a size of {size}");
        }
        // A stated size is read, and so is the nought the stream opens with.
        let mut stated = alone(b"");
        stated[5..13].copy_from_slice(&64u64.to_le_bytes());
        assert!(is_lzma(&stated));
        let mut no_nought = alone(b"");
        no_nought[HEADER] = 1;
        assert!(!is_lzma(&no_nought));
        // And a file too short to hold the header is not one.
        assert!(!is_lzma(&alone(b"")[..HEADER]));
    }
}
