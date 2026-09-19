//! The compressed file joblib wrote before 0.10: `ZF`, a length, and a zlib
//! stream holding the pickle.
//!
//! `write_zfile` in `joblib/numpy_pickle_utils.py` writes `ZF`, then the
//! unpacked length as `hex()` spells it, left-justified in a field as wide as
//! `hex(2 ** 64)` is, and then `zlib.compress` of the whole pickle. So the
//! header is twenty-one bytes and everything after it is one zlib stream.
//! joblib 0.10 dropped the container and wrote the plain compressor's own file
//! instead, which is why a `.joblib` from 2016 and one from now share nothing
//! at the front.
//!
//! The length is written as text and never read back as a number here: it is
//! `0x226` and fourteen spaces, which no integer type reads and which the
//! stream's own end says again.

use crate::codec::Codec;
use crate::template::{Encoding, Expr as E, StrLen, Template, Ty as T};

/// What the file opens with.
pub const MAGIC: &[u8] = b"ZF";

/// How wide the length field is: `len(hex(2 ** 64))`, which is
/// `_MAX_LEN` in joblib's own source.
const LENGTH_WIDTH: usize = 19;

/// Where the zlib stream starts, which is the same offset in every one of
/// these files.
const STREAM_AT: usize = MAGIC.len() + LENGTH_WIDTH;

pub fn joblibzfile() -> Template {
    Template::new(
        "joblibzfile",
        T::structure(
            "JoblibZFile",
            vec![
                ("magic", T::text(StrLen::Fixed(E::lit(MAGIC.len() as i128)), Encoding::Ascii)),
                // The unpacked length, as `hex()` spells it and padded with
                // spaces to the width above.
                ("unpacked size", T::text(StrLen::Fixed(E::lit(LENGTH_WIDTH as i128)), Encoding::Ascii)),
                // The pickle, compressed. The run stays where it is and what
                // comes out of it opens as a space of its own, which sniffs as
                // the joblib file it holds.
                ("decoded", T::decoded(E::Remaining, Codec::Zlib, super::decoded_text())),
            ],
        ),
    )
}

/// Whether these bytes open one of these.
///
/// `ZF` is two bytes and would say very little on its own, so the whole header
/// is read: the length has to be what `hex()` writes, padded with spaces to
/// its width and nothing else, and a zlib stream has to start exactly where
/// that field ends.
pub fn is_joblib_zfile(head: &[u8]) -> bool {
    if !head.starts_with(MAGIC) {
        return false;
    }
    let Some(field) = head.get(MAGIC.len()..STREAM_AT) else { return false };
    let said = field.trim_ascii_end();
    let Some(digits) = said.strip_prefix(b"0x") else { return false };
    if digits.is_empty() || !digits.iter().all(|b| b.is_ascii_hexdigit()) {
        return false;
    }
    head.get(STREAM_AT..).is_some_and(super::zlib::is_zlib)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, eval::Evaluator, source::MemSource};

    /// The header joblib writes over `packed`, with `len` as the length it
    /// declares.
    fn zfile(len: usize, packed: &[u8]) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(format!("{:<width$}", format!("{len:#x}"), width = LENGTH_WIDTH).as_bytes());
        out.extend_from_slice(packed);
        out
    }

    #[test]
    fn the_header_is_twenty_one_bytes_and_the_rest_is_the_stream() {
        let held = b"a pickle would be here";
        let packed = miniz_oxide::deflate::compress_to_vec_zlib(held, 6);
        let bytes = zfile(held.len(), &packed);
        assert!(is_joblib_zfile(&bytes));
        let d = Document::new(MemSource(bytes.clone()));
        let mut e = Evaluator::new(joblibzfile());
        let length = e.node(&d, &[1]).unwrap();
        assert_eq!(length.name, "unpacked size");
        assert_eq!(length.value, crate::eval::Value::Str("0x16               ".into()));
        let run = e.node(&d, &[2]).unwrap();
        assert_eq!((run.offset_bits, run.size_bits), (STREAM_AT as u64 * 8, packed.len() as u64 * 8));
        // And what came out of it counts from its own start.
        let id = e.open_space(&d, 0, &[2]).unwrap().expect("the stream opens");
        assert_eq!(e.space(id).unwrap().bytes(), held);
    }

    /// Two bytes would claim any file starting `ZF`, so the whole header is
    /// the evidence.
    #[test]
    fn a_header_that_is_not_one_is_refused() {
        let packed = miniz_oxide::deflate::compress_to_vec_zlib(b"x", 6);
        assert!(is_joblib_zfile(&zfile(1, &packed)));
        // No `0x` in front of the digits, digits that are not hexadecimal,
        // nothing where the length goes, and a stream that is not zlib.
        let mut decimal = zfile(1, &packed);
        decimal.splice(2..4, *b"  ");
        assert!(!is_joblib_zfile(&decimal));
        let mut wrong = zfile(1, &packed);
        wrong[4] = b'z';
        assert!(!is_joblib_zfile(&wrong));
        let mut empty = zfile(1, &packed);
        empty.splice(2..STREAM_AT, [b' '; LENGTH_WIDTH]);
        assert!(!is_joblib_zfile(&empty));
        let mut plain = zfile(1, b"not a stream at all");
        plain.truncate(STREAM_AT + 4);
        assert!(!is_joblib_zfile(&plain));
        assert!(!is_joblib_zfile(b"ZF"));
        assert!(!is_joblib_zfile(b""));
    }
}
