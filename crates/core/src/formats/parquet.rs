//! Parquet: a columnar table, read from the back.
//!
//! The file opens with `PAR1` and closes with it, and everything that says
//! where anything is sits in between the two. The last eight bytes are how
//! long the footer is and the magic again; the footer ends where those eight
//! bytes begin, so the whole structure is found by measuring backwards from
//! the end of the file. That is the point of the layout: a reader on a network
//! store fetches the last kilobyte, learns where every column chunk of every
//! row group is, and then fetches only the columns it was asked for.
//!
//! The footer is a Thrift compact-protocol `FileMetaData`, and it stays one
//! region here. Nothing decodes it: compact protocol writes its integers
//! zigzagged, and reading them any other way answers with a number no writer
//! wrote. Even the version, the very first field, would read as 2 where the
//! file says 1. A wrong number is worse than no number, so the footer is
//! bytes and the schema, the row groups and the column statistics inside it
//! are a gap.
//!
//! Everything between the opening magic and the footer is the row groups: the
//! pages of every column, each with its own header and its own encoding. The
//! footer is what places them, so this reads as one region until the footer is
//! read.
//!
//! An encrypted file closes with `PARE` rather than `PAR1`. Nothing before it
//! can be read at all, since the footer itself is what is encrypted, so this
//! says so and stops.

use crate::template::{Endian::{Big, Little}, Expr as E, Template, Ty as T};

/// What one of these opens with, and what an unencrypted one closes with.
pub const MAGIC: &[u8] = b"PAR1";

/// What an encrypted one closes with instead.
const ENCRYPTED: &[u8] = b"PARE";

/// The four bytes at the very end, as a number, without reading them.
fn trailing_magic() -> E {
    E::peek_at(E::lit(-32), 32, Big)
}

/// How long the footer is: the four bytes before the closing magic, which is
/// the only length in the file that is not itself in the footer.
fn footer_length() -> E {
    E::peek_at(E::lit(-64), 32, Little)
}

/// The bytes of a plain file: the row groups, the footer, and the trailer that
/// found the footer.
///
/// Both lengths are floored at nothing. A file cut off in the middle, or one
/// whose footer length is larger than the file, would otherwise ask for a run
/// of bytes measured backwards past where it started, and refusing to place
/// the bytes that are there would hide the very thing that went wrong.
fn plain() -> T {
    T::structure(
        "ParquetBody",
        vec![
            // Every page of every column, undecoded: the footer is what says
            // where one row group ends and the next begins.
            ("row_groups", T::bytes(E::Remaining.sub(E::lit(8)).sub(footer_length()).at_least(E::lit(0)))),
            // The Thrift `FileMetaData`, which runs from here to the eight
            // bytes that measured it.
            ("footer", T::bytes(E::Remaining.sub(E::lit(8)).at_least(E::lit(0)))),
            ("footer_length", T::u32(Little)),
            ("footer_magic", T::magic(MAGIC)),
        ],
    )
}

/// An encrypted file: the footer is ciphertext, so nothing in it says where
/// anything is and there is nothing to place.
fn encrypted() -> T {
    T::structure(
        "ParquetEncrypted",
        vec![
            ("encrypted", T::bytes(E::Remaining.sub(E::lit(4)).at_least(E::lit(0)))),
            ("footer_magic", T::magic(ENCRYPTED)),
        ],
    )
}

pub fn parquet() -> Template {
    let root = T::structure(
        "Parquet",
        vec![
            ("magic", T::magic(MAGIC)),
            (
                "body",
                T::switch(trailing_magic(), vec![(0x5041_5245, encrypted())], plain()),
            ),
        ],
    );
    Template::new("parquet", root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, eval::{Evaluator, Value}, source::MemSource};

    /// A file of `rows` bytes of row group and `footer` bytes of footer,
    /// closed the way `end` says.
    fn file(rows: usize, footer: usize, end: &[u8]) -> Vec<u8> {
        let mut v = MAGIC.to_vec();
        v.extend(std::iter::repeat(0xAB).take(rows));
        v.extend(std::iter::repeat(0x15).take(footer));
        v.extend_from_slice(&(footer as u32).to_le_bytes());
        v.extend_from_slice(end);
        v
    }

    #[test]
    fn the_footer_is_measured_back_from_the_end() {
        let d = Document::new(MemSource(file(40, 12, MAGIC)));
        let mut e = Evaluator::new(parquet());
        assert_eq!(e.node(&d, &[1, 0]).unwrap().size_bits, 40 * 8);
        assert_eq!(e.node(&d, &[1, 1]).unwrap().size_bits, 12 * 8);
        assert_eq!(e.node(&d, &[1, 2]).unwrap().value, Value::UInt(12));
    }

    #[test]
    fn a_file_with_no_row_groups_still_reads() {
        let d = Document::new(MemSource(file(0, 4, MAGIC)));
        let mut e = Evaluator::new(parquet());
        assert_eq!(e.node(&d, &[1, 0]).unwrap().size_bits, 0);
        assert_eq!(e.node(&d, &[1, 1]).unwrap().size_bits, 4 * 8);
    }

    #[test]
    fn a_footer_longer_than_the_file_places_what_is_there() {
        let mut bytes = file(8, 4, MAGIC);
        let n = bytes.len();
        bytes[n - 8..n - 4].copy_from_slice(&0xFFFF_u32.to_le_bytes());
        let d = Document::new(MemSource(bytes));
        let mut e = Evaluator::new(parquet());
        assert_eq!(e.node(&d, &[1, 0]).unwrap().size_bits, 0);
        assert_eq!(e.node(&d, &[1, 1]).unwrap().size_bits, 12 * 8);
    }

    #[test]
    fn an_encrypted_file_says_so_and_stops() {
        let d = Document::new(MemSource(file(40, 12, ENCRYPTED)));
        let mut e = Evaluator::new(parquet());
        let body = e.node(&d, &[1]).unwrap();
        assert_eq!(body.type_name, "ParquetEncrypted");
        assert_eq!(e.node(&d, &[1, 0]).unwrap().size_bits, 56 * 8);
    }
}
