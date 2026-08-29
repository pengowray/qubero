//! xar archives, which is what a macOS installer package is: a header, a
//! table of contents, and a heap of everything the table points into.
//!
//! The table of contents is XML, and it is compressed, so almost everything
//! this format knows about itself is out of reach here: which files are in
//! the package, what they are called, where in the heap each one starts, how
//! long it is compressed and uncompressed, its checksum, and the signature
//! over the lot. All of that is in the table, and reading it means inflating
//! it first, which is the same open question the gzip template stands in
//! front of.
//!
//! What is left is the header, which is worth having anyway: it says how long
//! the table is both ways round, which is what makes the heap findable, and it
//! says which checksum the table's entries use. It also says how long it is
//! itself, so a version of the format with more in the header stays readable:
//! what a reader does not know about is the run between the fields it does
//! know and the table.

use crate::template::{Endian::Big, Expr as E, Template, Ty as T};

/// What every one of these starts with.
pub const MAGIC: &[u8] = b"xar!";

/// How much of the header this template reads. A file whose header is longer
/// than this was written by something newer, and the rest of it is left as the
/// run it is rather than guessed at.
const KNOWN_HEADER: i128 = 28;

pub fn xar() -> Template {
    Template::new(
        "xar",
        T::structure(
            "XarArchive",
            vec![
                ("magic", T::magic(MAGIC)),
                ("header_size", T::u16(Big)),
                ("version", T::u16(Big)),
                ("toc_compressed_size", T::u64(Big)),
                ("toc_uncompressed_size", T::u64(Big)),
                (
                    "checksum_algorithm",
                    T::enumeration(
                        "XarChecksum",
                        T::u32(Big),
                        &[(0, "none"), (1, "sha1"), (2, "md5"), (3, "sha256"), (4, "sha512")],
                    ),
                ),
                // Whatever a later version of the header holds, which this does
                // not read and does not have to: the header says where it ends.
                ("header_rest", T::bytes(E::field("header_size").sub(E::lit(KNOWN_HEADER)).at_least(E::lit(0)))),
                // The table of contents: XML, deflated, with a zlib header on
                // it. Its length is in the header rather than in the stream,
                // which is what makes the heap findable without inflating it.
                ("toc", T::structure("ZlibStream", vec![("data", T::bytes(E::field("toc_compressed_size")))])),
                // Every file in the package, one after another, at the offsets
                // the table gives. Without the table there is nothing to say
                // about them, so the heap is one run of bytes.
                ("heap", T::bytes(E::Remaining)),
            ],
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::Document,
        eval::{Evaluator, Value},
        source::MemSource,
    };

    fn archive(header_size: u16, toc: &[u8], heap: &[u8]) -> Vec<u8> {
        let mut v = MAGIC.to_vec();
        v.extend_from_slice(&header_size.to_be_bytes());
        v.extend_from_slice(&1u16.to_be_bytes());
        v.extend_from_slice(&(toc.len() as u64).to_be_bytes());
        v.extend_from_slice(&512u64.to_be_bytes());
        v.extend_from_slice(&3u32.to_be_bytes());
        v.resize(header_size as usize, 0);
        v.extend_from_slice(toc);
        v.extend_from_slice(heap);
        v
    }

    #[test]
    fn the_header_measures_the_table_and_what_is_left_is_the_heap() {
        let d = Document::new(MemSource(archive(28, b"\x78\x9c compressed toc", b"file data")));
        let mut e = Evaluator::new(xar());
        assert_eq!(e.node(&d, &[5]).unwrap().value, Value::Enum { raw: 3, name: Some("sha256".into()), hex: false });
        assert_eq!(e.node(&d, &[6]).unwrap().size_bits, 0);
        assert_eq!(e.node(&d, &[7]).unwrap().size_bits, 17 * 8);
        assert_eq!(e.node(&d, &[8]).unwrap().size_bits, 9 * 8);
    }

    /// A header longer than the fields anyone knows about is a newer file, not
    /// a broken one: the table starts where the header says it does.
    #[test]
    fn a_longer_header_moves_the_table_rather_than_breaking_it() {
        let d = Document::new(MemSource(archive(32, b"toc", b"heap")));
        let mut e = Evaluator::new(xar());
        assert_eq!(e.node(&d, &[6]).unwrap().size_bits, 4 * 8);
        assert_eq!(e.node(&d, &[7]).unwrap().offset_bits, 32 * 8);
        assert_eq!(e.node(&d, &[8]).unwrap().size_bits, 4 * 8);
    }
}
