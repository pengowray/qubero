//! What a file looks like from a distance: the whole of it divided into
//! equal buckets, each classified by what its bytes are like. A view drawn
//! from this shows at a glance where the text is, where the padding is, and
//! where the data too dense to be either sits, without a template saying so.
//!
//! The scan runs the way a search does: a bounded step at a time, answering
//! with the chunks it needs or with where to carry on, because the file may be
//! larger than memory and its bytes arrive as they are read. See `search.rs`
//! for the shape this copies.
//!
//! Classes are judged per bucket, so a short run inside a bucket is charged to
//! whatever the bucket mostly is. That is the deal the whole view makes:
//! resolution is traded for being able to see all of a file at once.

use crate::document::Document;
use crate::source::{Missing, Source};

/// How many buckets a file is aimed to divide into. The size of one bucket is
/// the smallest power of two that gets the file under this, so a cell on
/// screen stands for a round number of bytes.
pub const TARGET_BUCKETS: u64 = 4096;

/// How many bytes one step reads before answering.
pub const WINDOW: u64 = 256 * 1024;

/// What one bucket's bytes turned out to be. The discriminants are the wire
/// format: they cross to the UI as digits, one per bucket, and the UI's
/// legend indexes by them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Every byte is zero.
    Zero = 0,
    /// Every byte is the same value, and it is not zero. Padding and unworn
    /// flash are this, most often as 0xff.
    Fill = 1,
    /// Mostly printable characters, with tabs and line breaks counted in.
    Text = 2,
    /// Bytes with structure to them: neither text nor dense enough to be
    /// compressed. Most headers, tables and machine code land here.
    Data = 3,
    /// Bytes using the whole range about evenly, which is what compressed and
    /// encrypted data look like. Nothing readable is in here.
    Random = 4,
}

/// What one step of the scan answered.
#[derive(Debug, Clone, PartialEq)]
pub enum ScanStep {
    /// Not finished. Ask again.
    More,
    /// Every bucket is classified.
    Done,
    /// Chunks this step needs before it can go on.
    Pending(Vec<Missing>),
}

/// The scan and what it has found so far. Built once per file and kept until
/// an edit, when the byte classes may no longer hold and the whole thing is
/// worked out again: it costs one pass over the file, which an edit already
/// paid for the parts it read.
#[derive(Debug, Clone)]
pub struct Scan {
    len: u64,
    bucket_bytes: u64,
    /// One class per finished bucket, in file order.
    classes: Vec<u8>,
    /// The next byte to read.
    next: u64,
    /// Byte counts for the bucket being read, which may span several windows
    /// when a bucket outweighs one.
    hist: [u32; 256],
    /// How many of the current bucket's bytes are counted so far.
    filled: u64,
    zero_bytes: u64,
    text_bytes: u64,
}

/// Printable in the way running text is: the visible ASCII range, plus the
/// whitespace text actually contains.
fn textual(b: u8) -> bool {
    (0x20..0x7f).contains(&b) || b == b'\t' || b == b'\n' || b == b'\r'
}

/// The share of `total` that printable bytes must reach for a bucket to read
/// as text, in percent.
const TEXT_PERCENT: u64 = 85;

/// The share of the highest entropy a bucket could have, above which it reads
/// as compressed or encrypted. Judged against the bucket's own ceiling: a
/// 4 KiB bucket can reach 8 bits per byte and reads as random past about 7.3,
/// while a 16 byte one can only reach 4. Machine code and packed tables run to
/// about 6.5 bits; DEFLATE and anything encrypted sit above 7.9.
const RANDOM_SHARE: f64 = 0.91;

impl Scan {
    pub fn new(len: u64) -> Scan {
        // The smallest power-of-two bucket that keeps the count under the
        // target, so a cell stands for 1 byte, 2, 4 … and never 3000.
        let mut bucket_bytes = 1u64;
        while len.div_ceil(bucket_bytes) > TARGET_BUCKETS {
            bucket_bytes *= 2;
        }
        Scan {
            len,
            bucket_bytes,
            classes: Vec::new(),
            next: 0,
            hist: [0; 256],
            filled: 0,
            zero_bytes: 0,
            text_bytes: 0,
        }
    }

    pub fn bucket_bytes(&self) -> u64 {
        self.bucket_bytes
    }

    pub fn total_buckets(&self) -> u64 {
        self.len.div_ceil(self.bucket_bytes)
    }

    /// Classes of the buckets finished so far, in file order.
    pub fn classes(&self) -> &[u8] {
        &self.classes
    }

    pub fn done(&self) -> bool {
        self.next >= self.len
    }

    /// Bytes that are zero, over the part of the file read so far.
    pub fn zero_bytes(&self) -> u64 {
        self.zero_bytes
    }

    /// Bytes printable as text, over the part read so far.
    pub fn text_bytes(&self) -> u64 {
        self.text_bytes
    }

    /// How far the scan has read, in bytes.
    pub fn read_bytes(&self) -> u64 {
        self.next
    }

    /// Read one window and classify the buckets it completes.
    pub fn step<S: Source>(&mut self, doc: &Document<S>) -> ScanStep {
        if self.done() {
            return ScanStep::Done;
        }
        let stop = (self.next + WINDOW).min(self.len);
        let mut buf = vec![0u8; (stop - self.next) as usize];
        let missing = doc.read_bytes(self.next, &mut buf);
        if !missing.is_empty() {
            return ScanStep::Pending(missing);
        }
        for &b in &buf {
            self.hist[b as usize] += 1;
            self.filled += 1;
            if self.filled == self.bucket_bytes {
                self.close_bucket();
            }
        }
        self.next = stop;
        if self.done() {
            // The last bucket may be short of a full one.
            if self.filled > 0 {
                self.close_bucket();
            }
            ScanStep::Done
        } else {
            ScanStep::More
        }
    }

    /// Judge the bucket the histogram describes, and start the next.
    fn close_bucket(&mut self) {
        let total = self.filled;
        let zeros = u64::from(self.hist[0]);
        let text: u64 = (0u16..256).filter(|&b| textual(b as u8)).map(|b| u64::from(self.hist[b as usize])).sum();
        self.zero_bytes += zeros;
        self.text_bytes += text;
        let distinct = self.hist.iter().filter(|&&n| n > 0).count();
        // The most a bucket this size can reach: 8 bits per byte, or fewer
        // distinct values than that allows when the bucket is small.
        let ceiling = (total as f64).log2().min(8.0);
        // A tiny bucket cannot support the finer judgements: one byte is not
        // "a repeated byte", and a handful of distinct bytes is not evidence
        // of compression. Small buckets fall back to data.
        let class = if zeros == total {
            Class::Zero
        } else if text * 100 >= total * TEXT_PERCENT {
            Class::Text
        } else if distinct == 1 && total >= 4 {
            Class::Fill
        } else if total >= 64 && bits(&self.hist, total) >= RANDOM_SHARE * ceiling {
            Class::Random
        } else {
            Class::Data
        };
        self.classes.push(class as u8);
        self.hist = [0; 256];
        self.filled = 0;
    }
}

/// Shannon entropy of the histogram, in bits per byte.
fn bits(hist: &[u32; 256], total: u64) -> f64 {
    let mut bits = 0.0f64;
    for &n in hist {
        if n == 0 {
            continue;
        }
        let p = n as f64 / total as f64;
        bits -= p * p.log2();
    }
    bits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{ChunkStore, MemSource};

    fn doc(bytes: &[u8]) -> Document<MemSource> {
        Document::new(MemSource(bytes.to_vec()))
    }

    fn run(bytes: &[u8]) -> Scan {
        let d = doc(bytes);
        let mut s = Scan::new(bytes.len() as u64);
        loop {
            match s.step(&d) {
                ScanStep::Done => return s,
                ScanStep::More => {}
                ScanStep::Pending(_) => panic!("a memory source is never pending"),
            }
        }
    }

    #[test]
    fn an_empty_file_is_done_before_it_starts() {
        let s = run(b"");
        assert!(s.done());
        assert_eq!(s.classes(), &[] as &[u8]);
    }

    #[test]
    fn a_small_file_gets_one_byte_per_bucket() {
        let s = run(b"A\0");
        assert_eq!(s.bucket_bytes(), 1);
        assert_eq!(s.classes(), &[Class::Text as u8, Class::Zero as u8]);
    }

    #[test]
    fn bucket_size_is_a_power_of_two_that_meets_the_target() {
        let s = Scan::new(TARGET_BUCKETS * 3);
        assert_eq!(s.bucket_bytes(), 4);
        assert_eq!(s.total_buckets(), TARGET_BUCKETS * 3 / 4);
    }

    #[test]
    fn each_kind_of_bucket_is_told_apart() {
        // Four buckets of 64: zeroes, a fill, text, and bytes using the whole
        // range, which is what compressed data reads as.
        let mut bytes = vec![0u8; 64];
        bytes.extend(std::iter::repeat_n(0xffu8, 64));
        bytes.extend(b"a readable line\n".repeat(4));
        bytes.extend((0..64u8).map(|i| i.wrapping_mul(37).wrapping_add(11)));
        let d = doc(&bytes);
        let mut s = Scan::new(256);
        s.bucket_bytes = 64;
        while s.step(&d) == ScanStep::More {}
        assert_eq!(
            s.classes(),
            &[Class::Zero as u8, Class::Fill as u8, Class::Text as u8, Class::Random as u8]
        );
    }

    #[test]
    fn ordinary_structured_bytes_read_as_data() {
        // A run of small integers with headroom left over: too repetitive to
        // be random, not letters enough to be text.
        let bytes: Vec<u8> = (0..64u32).map(|i| (i % 7) as u8).collect();
        let d = doc(&bytes);
        let mut s = Scan::new(64);
        s.bucket_bytes = 32;
        while s.step(&d) == ScanStep::More {}
        assert!(s.classes().iter().all(|&c| c == Class::Data as u8), "{:?}", s.classes());
    }

    #[test]
    fn the_zero_tail_of_a_file_shows_in_its_classes() {
        // The picture the whole feature is for: a file whose second half is
        // nothing, visible without anyone measuring for it.
        let mut bytes = vec![0x41u8; 512];
        bytes.extend(std::iter::repeat_n(0u8, 512));
        let s = run(&bytes);
        let n = s.classes().len();
        assert!(s.classes()[..n / 2].iter().all(|&c| c != Class::Zero as u8));
        assert!(s.classes()[n / 2..].iter().all(|&c| c == Class::Zero as u8));
        assert_eq!(s.zero_bytes(), 512);
    }

    #[test]
    fn a_short_last_bucket_is_still_classified() {
        let mut bytes = vec![0u8; 20];
        bytes.extend(b"xyz");
        let d = doc(&bytes);
        let mut s = Scan::new(23);
        s.bucket_bytes = 16;
        while s.step(&d) == ScanStep::More {}
        assert_eq!(s.classes().len(), 2);
        assert_eq!(s.classes()[1], Class::Data as u8, "4 zeroes and 3 letters are neither");
    }

    #[test]
    fn a_step_says_which_chunks_it_needs_and_resumes() {
        let bytes = vec![0xaau8; 300];
        let d0 = Document::new(ChunkStore::new(300, 64, 16));
        let mut s = Scan::new(300);
        let ScanStep::Pending(missing) = s.step(&d0) else { panic!("a file with no bytes yet cannot answer") };
        assert!(!missing.is_empty());
        // The same scan carries on once the bytes are there.
        let mut store = ChunkStore::new(300, 64, 16);
        for c in 0..300u64.div_ceil(64) {
            let start = (c * 64) as usize;
            let end = (start + 64).min(300);
            store.insert(c, bytes[start..end].to_vec().into_boxed_slice());
        }
        let d = Document::new(store);
        while s.step(&d) == ScanStep::More {}
        assert!(s.done());
        // One-byte buckets are too small to call "a repeated byte".
        assert!(s.classes().iter().all(|&c| c == Class::Data as u8));
    }
}
