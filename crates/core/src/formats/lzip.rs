//! lzip: an LZMA stream with a header that says how large a dictionary it
//! needs and a trailer that says what came out.
//!
//! The dictionary size is one byte holding two numbers. The low five bits are
//! a power of two, and the top three subtract that many sixteenths of it, so a
//! size between two powers can be named without a second field. The template
//! reads the two halves; multiplying them out is arithmetic a reader can do
//! with both numbers in front of them.
//!
//! The stream between header and trailer has no length on it, so it is
//! measured from the end: the last twenty bytes are the trailer, and
//! everything before them is LZMA.
//!
//! What opens is the member and not that stream. LZMA1 carries no header of
//! its own, so the packing and the dictionary size a decoder needs are things
//! the six bytes in front of it say, and a run starting after them could not
//! be read by anything. So the member is the run, and the header's fields are
//! laid over the same bytes: `xz` does this for the same reason, and the
//! field that holds the answer costs no bytes where it stands.
//!
//! A file may hold several members one after another, each with its own
//! header and trailer. This reads the first, whose trailer says how long it
//! is, and a second member would be past where this template stops.

use crate::codec::Codec;
use crate::template::{Check, Checksum, Covers, Endian::{Big, Little}, Expr as E, Named, Template, Ty as T};

/// What one of these starts with.
pub const MAGIC: &[u8] = b"LZIP";

/// The trailer: a checksum, what came out, and how long the member is.
const TRAILER: i128 = 20;

pub fn lzip() -> Template {
    Template::new(
        "lzip",
        T::structure(
            "LzipMember",
            vec![
                ("magic", T::magic(MAGIC)),
                ("version", T::u8()),
                // Two numbers in one byte: subtract this many thirty-seconds
                // of the base below from it.
                ("dict_size_fraction", T::UInt { bits: 3, endian: Big }),
                // The base, as a power of two: 12 is 4 KiB, 29 is 512 MiB.
                ("dict_size_base", T::UInt { bits: 5, endian: Big }),
                // The LZMA stream. It ends with a marker rather than a length,
                // so it is measured backwards from the trailer. What it holds
                // is opened by `decoded` below, over the member rather than
                // over these bytes alone.
                ("lzma_stream", T::bytes(E::Remaining.sub(E::lit(TRAILER)))),
                ("crc32", T::u32(Little)),
                ("data_size", T::u64(Little)),
                // The whole member including this field, which is what makes
                // the next member in a multi-member file findable.
                ("member_size", T::u64(Little)),
                // What the member comes to. The stream above cannot be opened
                // on its own, since LZMA1 has no header and the two numbers a
                // decoder needs are in the six bytes in front of it, so the
                // field that holds the answer is the member. It costs no bytes
                // where it stands and covers the member from its first one.
                ("decoded", T::at_in_window(E::lit(0), T::decoded(E::Remaining, Codec::Lzip, super::decoded_text()))),
            ],
        )
        // What went in, not the stream it came out as. `data_size` is only for
        // deciding whether to unpack it unasked; the sum is over whatever the
        // decoder produced.
        .field_check(
            "crc32",
            Check::of(Checksum::Crc32, Covers::Unpacked { name: Named::here("decoded"), len: Some(E::field("data_size")) }),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, eval::{Evaluator, Value}, source::MemSource};

    fn member(lzma: &[u8]) -> Vec<u8> {
        let mut v = MAGIC.to_vec();
        v.push(1);
        v.push(0x0c);
        v.extend_from_slice(lzma);
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&0u64.to_le_bytes());
        let size = (v.len() + 8) as u64;
        v.extend_from_slice(&size.to_le_bytes());
        v
    }

    #[test]
    fn the_stream_is_measured_back_from_the_trailer() {
        let d = Document::new(MemSource(member(b"\x00\x01\x02\x03\x04")));
        let mut e = Evaluator::new(lzip());
        assert_eq!(e.node(&d, &[2]).unwrap().value.as_int(), Some(0));
        assert_eq!(e.node(&d, &[3]).unwrap().value.as_int(), Some(12));
        assert_eq!(e.node(&d, &[4]).unwrap().size_bits, 5 * 8);
        assert_eq!(e.node(&d, &[7]).unwrap().value.as_int(), Some(31));
    }

    /// A member holding a real stream. `lzma-rs` writes the thirteen-byte
    /// "alone" header in front of what it packs; lzip's own six bytes say the
    /// same two things in less room, so the header comes off and the member
    /// goes on.
    fn packed_member(payload: &[u8]) -> Vec<u8> {
        let mut alone = Vec::new();
        lzma_rs::lzma_compress(&mut &payload[..], &mut alone).expect("packs");
        let mut v = MAGIC.to_vec();
        v.push(1);
        // 8 MiB with nothing taken off it, which is the dictionary the header
        // being dropped here named.
        v.push(0x17);
        v.extend_from_slice(&alone[13..]);
        v.extend_from_slice(&crate::checksum::crc32(payload).to_le_bytes());
        v.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        let size = (v.len() + 8) as u64;
        v.extend_from_slice(&size.to_le_bytes());
        v
    }

    #[test]
    fn the_member_opens_into_what_went_in_and_the_sum_agrees() {
        let payload = "lzip carries an LZMA stream with no header of its own.\n".repeat(40);
        let d = Document::new(MemSource(packed_member(payload.as_bytes())));
        let mut e = Evaluator::new(lzip());
        // What came out is a document of its own, counted from its own first
        // byte rather than from anywhere in the file. The whole of it is
        // there; only what the value shows is cut, at the same 256 bytes
        // every text field is cut at.
        let text = e.node(&d, &[8, 0, 0, 0]).unwrap();
        assert_ne!(text.space, 0);
        assert_eq!(text.offset_bits, 0);
        assert_eq!(text.size_bits, payload.len() as u64 * 8);
        let Value::Str(shown) = &text.value else { panic!("the stream held text") };
        assert!(payload.starts_with(shown.trim_end_matches('\u{2026}')));
        // The sum is of what went in, so it covers bytes that are nowhere in
        // the file, and it is over every one of them rather than over the 256
        // the value shows. What a reader can be sent to is the member the
        // stream came out of.
        let info = e.check_of(&d, &[5]).unwrap().expect("crc32 checks something");
        assert_eq!(info.algorithm, "crc32");
        assert_eq!(info.over, None);
        assert_eq!(info.unpacked_from, Some((0, d.len_bytes())));
        assert_eq!(info.covered_bytes, payload.len() as u64);
        assert!(e.run_check(&d, &[5]).unwrap().expect("a verdict").ok);
    }
}
