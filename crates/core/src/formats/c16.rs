//! `.c16` captures: what an SDR wrote down off the air, and nothing else.
//!
//! A radio receiver mixes the signal down to a pair of streams a quarter-cycle
//! apart, in phase and in quadrature, and a capture is those two streams
//! written one sample after the other: I, Q, I, Q, each a signed 16-bit
//! little-endian number. HackRF's PortaPack writes these, and so do the
//! recording modes of gqrx, SDR#, SDRangel and the rest.
//!
//! There is no header. Nothing in the file says what it was tuned to, how fast
//! it was sampled, or even that it is a capture at all: a recorder that keeps
//! those puts them in a `.txt` beside it. So this is a template to pick rather
//! than one to guess at, and picking it is what says the file is a capture.
//! A file whose length is not a whole number of samples leaves its last bytes
//! outside the run, where a gap is the honest reading.

use crate::template::{Endian::*, Expr as E, Template, Ty as T, Until};

pub fn c16() -> Template {
    let pair = T::inline_structure(
        "Sample",
        vec![("i", T::Int { bits: 16, endian: Little }), ("q", T::Int { bits: 16, endian: Little })],
    );
    Template::new("c16", T::repeat(pair.counted_as("sample"), Until::End))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    #[test]
    fn the_pairs_read_as_signed_numbers() {
        let mut v = Vec::new();
        for (i, q) in [(0i16, 0i16), (1, -1), (i16::MAX, i16::MIN)] {
            v.extend_from_slice(&i.to_le_bytes());
            v.extend_from_slice(&q.to_le_bytes());
        }
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(c16());
        assert_eq!(ev.node(&d, &[]).unwrap().child_count, 3);
        assert_eq!(ev.node(&d, &[1, 0]).unwrap().value, Value::Int(1));
        assert_eq!(ev.node(&d, &[1, 1]).unwrap().value, Value::Int(-1));
        assert_eq!(ev.node(&d, &[2, 0]).unwrap().value, Value::Int(32767));
        assert_eq!(ev.node(&d, &[2, 1]).unwrap().value, Value::Int(-32768));
    }

    /// A recording cut off mid-sample is still a recording. The samples that
    /// are whole read; the bytes after the last of them belong to no sample.
    #[test]
    fn a_part_sample_at_the_end_is_left_out_of_the_run() {
        let d = Document::new(MemSource(vec![1, 0, 2, 0, 3, 0]));
        let mut ev = Evaluator::new(c16());
        let root = ev.node(&d, &[]).unwrap();
        assert_eq!(root.child_count, 1);
        assert_eq!(root.size_bits, 4 * 8);
    }
}
