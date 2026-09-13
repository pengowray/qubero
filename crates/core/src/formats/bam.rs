//! BGZF, and the genomics formats written in it.
//!
//! **BGZF** is gzip cut into blocks. Each block is a whole gzip member of at
//! most 64 KB, so an ordinary gunzip reads the file and gets the whole of what
//! went in, and each member carries one extra subfield, `BC`, holding the size
//! of the block less one. That number is the point of the format: a reader can
//! step from block to block without inflating anything, and an index can name
//! a place in the file as the block it starts in and a byte inside what that
//! block unpacks to. The file ends with a 28-byte block that unpacks to
//! nothing, so a file cut short can be told from one that finished.
//!
//! Every block is read here as the gzip member it is, through
//! [`super::gzip::member`], with the extra field's subfields named and the
//! member sized by its `BC` number rather than by where the file ends.
//!
//! The SAM/BAM specification (samtools/hts-specs, `SAMv1.tex`) is what this
//! follows.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Until, Ty as T};

/// The subfield identifier BGZF writes into every member's extra field.
const BC: &[u8; 2] = b"BC";

/// Where the `BC` number is in a block, in bytes, when `BC` is the first
/// subfield: twelve bytes of gzip header, two of identifier and two of
/// subfield length. Every writer in use puts it first, and the block's size
/// has to be known before its extra field has been read, so this is where it
/// is looked for.
const BSIZE_AT: i128 = 16;

pub fn bgzf() -> Template {
    Template::new("bgzf", T::structure("BGZF", vec![("blocks", T::repeat(block(), Until::End))]))
}

/// One block: a gzip member, as long as its `BC` number says.
///
/// The number is read ahead, at [`BSIZE_AT`], because a `Sized` needs its size
/// before anything inside it is placed; the same two bytes are read again as a
/// field of the extra field, where they have a name. When the identifier and
/// the subfield length there are not `BC` and 2, the block runs to the end of
/// the file instead, which is how a plain gzip member reads: a block that does
/// not say how long it is has not been measured, and reading it as the rest of
/// the file shows that rather than guessing.
fn block() -> T {
    let peek16 = |at: i128, endian| E::peek_at(E::lit(at * 8), 16, endian);
    let is_bc = E::lit(BSIZE_AT + 1)
        .less_than(E::Remaining)
        .both(peek16(BSIZE_AT - 4, Big).equal_to(E::lit(u16::from_be_bytes(*BC) as i128)))
        .both(peek16(BSIZE_AT - 2, Little).equal_to(E::lit(2)));
    let size = E::cond(is_bc, peek16(BSIZE_AT, Little).add(E::lit(1)), E::Remaining);
    T::sized(size, super::gzip::member("BgzfBlock", extra(), payload())).counted_as("block")
}

/// The extra field, as the subfields RFC 1952 says it holds: two letters, a
/// length, and that many bytes. BGZF's is `BC` and holds a sixteen-bit number;
/// anything else a writer put beside it is left as its bytes.
fn extra() -> T {
    let subfield = T::structure_named(
        "ExtraSubfield",
        "id",
        "data",
        vec![
            ("id", T::text(StrLen::Fixed(E::lit(2)), Encoding::Latin1)),
            ("length", T::u16(Little)),
            (
                "data",
                T::sized(
                    E::field("length"),
                    T::matches(
                        E::field("id"),
                        // The size of the whole block, less one, so that a
                        // block of 65,536 bytes still fits in sixteen bits.
                        vec![("BC", T::structure("BgzfSize", vec![("bsize", T::u16(Little))]))],
                        T::bytes(E::Remaining),
                    ),
                ),
            ),
        ],
    );
    T::structure(
        "Extra",
        vec![("length", T::u16(Little)), ("subfields", T::sized(E::field("length"), T::repeat(subfield, Until::End)))],
    )
}

/// What a block unpacks to. Text, which reads as bytes where the bytes are not
/// text, until the formats written in BGZF are read below.
fn payload() -> T {
    super::decoded_text()
}

/// Whether the first bytes of a file are a BGZF block: a gzip header with the
/// extra flag set and a `BC` subfield of two bytes somewhere in its extra
/// field. A plain gzip file has no such subfield, and this is what tells the
/// two apart; both open with the same three bytes.
pub(crate) fn is_bgzf(head: &[u8]) -> bool {
    if head.len() < 18 || head[..3] != [0x1f, 0x8b, 8] || head[3] & 4 == 0 {
        return false;
    }
    let xlen = u16::from_le_bytes([head[10], head[11]]) as usize;
    let Some(extra) = head.get(12..12 + xlen) else { return false };
    let mut at = 0;
    while at + 4 <= extra.len() {
        let len = u16::from_le_bytes([extra[at + 2], extra[at + 3]]) as usize;
        if extra[at..at + 2] == *BC && len == 2 {
            return true;
        }
        at += 4 + len;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// One BGZF block holding `data`, compressed at the default level, the way
    /// htslib writes one.
    pub(crate) fn bgzf_block(data: &[u8]) -> Vec<u8> {
        let deflated = miniz_oxide::deflate::compress_to_vec(data, 6);
        let total = 12 + 6 + deflated.len() + 8;
        let mut v = vec![0x1f, 0x8b, 8, 4, 0, 0, 0, 0, 0, 0xff, 6, 0, b'B', b'C', 2, 0];
        v.extend_from_slice(&((total - 1) as u16).to_le_bytes());
        v.extend_from_slice(&deflated);
        v.extend_from_slice(&crate::checksum::crc32(data).to_le_bytes());
        v.extend_from_slice(&(data.len() as u32).to_le_bytes());
        v
    }

    /// The block every BGZF file ends with, as the specification writes it out.
    pub(crate) const EOF_BLOCK: [u8; 28] = [
        0x1f, 0x8b, 0x08, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0x06, 0x00, 0x42, 0x43, 0x02, 0x00, 0x1b, 0x00,
        0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn a_bgzf_file_is_a_list_of_blocks_each_as_long_as_its_bc_number() {
        let mut file = bgzf_block(b"first block of text");
        let first_len = file.len();
        file.extend_from_slice(&bgzf_block(b"and the second"));
        file.extend_from_slice(&EOF_BLOCK);
        let d = Document::new(MemSource(file.clone()));
        let mut ev = Evaluator::new(bgzf());
        let blocks = ev.node(&d, &[0]).unwrap();
        assert_eq!(blocks.child_count, 3);
        let first = ev.node(&d, &[0, 0]).unwrap();
        assert_eq!(first.size_bits, first_len as u64 * 8);
        let eof = ev.node(&d, &[0, 2]).unwrap();
        assert_eq!((eof.offset_bits / 8, eof.size_bits / 8), (file.len() as u64 - 28, 28));

        // The extra field names its subfield and the number in it.
        let extra = ev.child_named(&d, &[0, 0], "extra").unwrap().unwrap();
        let id = [extra.clone(), vec![1, 0, 0]].concat();
        assert_eq!(ev.node(&d, &id).unwrap().value, Value::Str("BC".into()));
        let bsize = [extra, vec![1, 0, 2, 0]].concat();
        assert_eq!(ev.node(&d, &bsize).unwrap().value.as_int(), Some(first_len as i128 - 1));

        // Each member's checksum covers what that member unpacks to.
        for i in 0..3 {
            let crc = ev.child_named(&d, &[0, i], "crc32").unwrap().unwrap();
            assert!(ev.run_check(&d, &crc).unwrap().expect("a verdict").ok, "block {i}");
        }
    }

    #[test]
    fn a_bgzf_file_is_told_from_a_plain_gzip_by_its_bc_subfield() {
        let file = bgzf_block(b"x");
        assert!(is_bgzf(&file));
        assert_eq!(super::super::sniff(&file, file.len() as u64), Some("bgzf"));
        // A gzip member with a name and no extra field.
        let mut plain = vec![0x1f, 0x8b, 8, 0x08, 0, 0, 0, 0, 0, 3];
        plain.extend_from_slice(b"hello.txt\0\x03\x00");
        plain.extend_from_slice(&[0; 8]);
        assert!(!is_bgzf(&plain));
        assert_eq!(super::super::sniff(&plain, plain.len() as u64), Some("gzip"));
        // An extra field with some other subfield in it is still plain gzip.
        let mut other = vec![0x1f, 0x8b, 8, 0x04, 0, 0, 0, 0, 0, 3, 6, 0, b'A', b'p', 2, 0, 1, 2];
        other.extend_from_slice(&[0x03, 0x00]);
        other.extend_from_slice(&[0; 8]);
        assert!(!is_bgzf(&other));
        assert_eq!(super::super::sniff(&other, other.len() as u64), Some("gzip"));
    }
}
