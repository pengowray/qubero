//! Text inside what the template reads as numbers: the report's stage that
//! looks for it.
//!
//! The strings view marks a string that falls inside a run of numbers (see
//! `readas.rs`), and leaves the reader to judge it. This goes the other way:
//! it takes every run of numbers the report's walk met and looks inside it
//! for text, with the strings view's own scanner and a longer minimum. A few
//! printable bytes turn up in samples by chance; twenty printable characters
//! in a row do not, and a run of numbers that holds them is likely a template
//! reading the wrong bytes. A bat recorder's metadata block read as the first
//! 490 samples of its recording is the case that showed it.
//!
//! Only numbers. Machine code and packed streams hold printable runs by
//! chance at every length worth asking about, so text in them says nothing
//! about the template. And only some numbers, each rule from a false hit found
//! on the sample collection:
//!
//! * Numbers of a fixed width of 16 bits or more. An 8-bit number from 32 to
//!   126 is a printable byte, so a table of small values reads as text: a
//!   JPEG quantization table is `!2222…`, a tracker module's channel volumes
//!   are `@@@@…`. A variable-length number is mostly single bytes for the same
//!   reason, and a number written in digits is text on purpose.
//! * ASCII and UTF-8 only. UTF-16 Latin text has a zero in every other byte,
//!   which is what small 16-bit numbers look like, and 16-bit numbers whose
//!   high byte stays the same read as UTF-16 of some script or other.
//! * Not text whose bytes at one place in each number are all the same. That
//!   is numbers close to one another, printable by chance: 32-bit floats of
//!   one sign and size all start with the same byte, and read as
//!   `A#33A#3=A#3H…`. Real text does not repeat one character at a fixed
//!   stride for a whole run.

use crate::source::{Missing, Source};
use crate::stringscan::{self, Opts};

use super::*;

/// The shortest text counted, in characters (bytes, for ASCII). Chosen with
/// `examples/text_in_numbers.rs` over the 4,280 sample files a template
/// reads: the shortest length at which none of them has a hit (at 16 there
/// are 8, in a GGUF, a NIfTI and a Whisper model's weights), while the bat
/// recorder's WAV with its metadata block read as samples still has 12 of its
/// 16 lines of text found.
pub const TEXT_MIN_CHARS: usize = 20;

/// How much of the runs of numbers the stage reads, in all and in one run.
/// The same caps the ledger reads gaps under. A run longer than the second is
/// read at its two ends, half each: a length that starts the run in the wrong
/// place puts foreign bytes at its front, and one that is too long takes in
/// what follows the run at its back, which in a WAV is the `LIST` chunk.
const SCAN_CAP: u64 = 4 * 1024 * 1024;
const SCAN_ONE: u64 = 1024 * 1024;

/// Bytes read and scanned in one step. A step of the scanner costs about
/// what placing an element costs per 128 bytes, and is charged so.
const CHUNK: u64 = 256 * 1024;
const BYTES_PER_ELEMENT: u64 = 128;

/// Text runs listed for each run of numbers; the rest are counted.
const FIRST: usize = 5;
/// Characters of each listed text run kept for its preview.
const PREVIEW_CHARS: usize = 48;

/// Every run of numbers that holds text, and how much of them was read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextInNumbers {
    /// True once the walk is over and every run has been read or given up on.
    pub done: bool,
    /// The shortest text counted, in characters.
    pub min_chars: u32,
    /// Bytes of numbers the walk found, and how many of them were read.
    /// Fewer read than found means the answer is about part of them.
    pub numeric_bytes: u64,
    pub scanned_bytes: u64,
    /// The runs with text in them, in the order the walk met them.
    pub runs: Vec<NumbersWithText>,
}

/// One run of numbers with text in it.
#[derive(Debug, Clone, PartialEq)]
pub struct NumbersWithText {
    /// The run, in the tab's paths once the tab has said so.
    pub path: Vec<usize>,
    /// Its name as the listing gives it, `samples`.
    pub name: String,
    /// What one element is, the words the strings view uses: `i16 le`.
    pub what: String,
    /// Bits in one element, 0 where the run has none to measure.
    pub element_bits: u64,
    pub offset_bits: u64,
    pub size_bits: u64,
    /// How many bytes of the run were read, which is all of it unless it was
    /// longer than one run's cap or the total ran out.
    pub scanned_bytes: u64,
    /// How many runs of text, and their bytes together.
    pub texts: u64,
    pub text_bytes: u64,
    /// The first few, in file order.
    pub first: Vec<TextRun>,
}

/// One run of text inside a run of numbers.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    pub offset_bits: u64,
    pub size_bits: u64,
    /// `ASCII`, `UTF-8`, `UTF-16 LE` or `UTF-16 BE`.
    pub encoding: &'static str,
    /// The start of the text, cut at a few dozen characters.
    pub text: String,
}

/// A run of numbers the walk met, as `ready` worked it out.
#[derive(Debug, Clone)]
pub(super) struct Found {
    pub name: String,
    pub what: String,
    pub element_bits: u64,
    pub size_bits: u64,
}

/// A run of numbers, and what has been found in it so far.
#[derive(Debug, Clone)]
struct Run {
    path: Vec<usize>,
    found: Found,
    offset_bits: u64,
    scanned: u64,
    texts: u64,
    text_bytes: u64,
    first: Vec<TextRun>,
    /// Where the last text run found ended, when it was cut at the scanner's
    /// longest string: the next one starting there is the same text.
    cut_end: Option<u64>,
}

/// What the stage keeps between goes.
#[derive(Debug)]
pub(super) struct Tally {
    runs: Vec<Run>,
    /// Which run is being read, and the byte to carry on from.
    at: (usize, u64),
    scanned: u64,
    min_chars: usize,
}

impl Default for Tally {
    fn default() -> Self {
        Tally { runs: Vec::new(), at: (0, 0), scanned: 0, min_chars: TEXT_MIN_CHARS }
    }
}

impl Tally {
    pub(super) fn set_min_chars(&mut self, n: usize) {
        self.min_chars = n.max(1);
    }

    /// A run of numbers the walk has gone into, once.
    pub(super) fn add(&mut self, path: &[usize], offset_bits: u64, found: Found) {
        if found.size_bits < 8 || offset_bits % 8 != 0 {
            return;
        }
        self.runs.push(Run { path: path.to_vec(), found, offset_bits, scanned: 0, texts: 0, text_bytes: 0, first: Vec::new(), cut_end: None });
    }

    /// Read the runs for text, a chunk at a time, charging each chunk against
    /// the allowance. Bytes not yet fetched come back as `Pending`, and the
    /// chunk is read again on the next go.
    pub(super) fn scan<S: Source>(&mut self, ev: &mut Evaluator, doc: &Document<S>, space: u32) -> R<()> {
        let opts = Opts { min_chars: self.min_chars, ascii: true, utf16le: false, utf16be: false };
        while let Some(run) = self.runs.get(self.at.0) {
            let pieces = pieces(run.offset_bits, run.found.size_bits);
            let piece = pieces.iter().flatten().find(|(_, to)| *to > self.at.1).copied();
            let Some((from, to)) = piece.filter(|_| self.scanned < SCAN_CAP) else {
                self.at = (self.at.0 + 1, 0);
                continue;
            };
            let from = from.max(self.at.1);
            let stop = (from + CHUNK).min(to);
            // Past `stop`, as far as the longest string the scanner reports,
            // so that a string starting just before it is seen whole.
            let end = (stop + stringscan::OVERLAP).min(to);
            ev.spend_n(from * 8, (stop - from).div_ceil(BYTES_PER_ELEMENT))?;
            let bytes = ev.read_in(doc, space, from * 8, (end - from) * 8)?;
            let src = Window { base: from, bytes };
            let mut next = from;
            let mut hits = Vec::new();
            while next < stop {
                let s = stringscan::scan(&src, next, usize::MAX, opts);
                if !s.missing.is_empty() || s.next <= next {
                    break;
                }
                hits.extend(s.hits.into_iter().filter(|h| h.at < stop));
                next = s.next;
            }
            let carry = hits.iter().map(|h| h.at + h.len).max().unwrap_or(stop).max(stop);
            let run = &mut self.runs[self.at.0];
            let (start, width) = (run.offset_bits / 8, run.found.element_bits / 8);
            for h in hits {
                if strided(&src.bytes[(h.at - from) as usize..(h.at - from + h.len) as usize], (h.at - start) % width, width) {
                    run.cut_end = None;
                    continue;
                }
                if run.cut_end == Some(h.at) {
                    run.text_bytes += h.len;
                    if let Some(last) = run.first.last_mut().filter(|l| l.offset_bits + l.size_bits == h.at * 8) {
                        last.size_bits += h.len * 8;
                    }
                } else {
                    run.texts += 1;
                    run.text_bytes += h.len;
                    if run.first.len() < FIRST {
                        run.first.push(TextRun { offset_bits: h.at * 8, size_bits: h.len * 8, encoding: h.enc.name(), text: h.text.chars().take(PREVIEW_CHARS).collect() });
                    }
                }
                run.cut_end = h.cut.then_some(h.at + h.len);
            }
            let read = carry.min(to) - from;
            run.scanned += read;
            self.scanned += read;
            self.at.1 = carry;
        }
        Ok(())
    }

    /// The answer so far. `done` is the caller's to say.
    pub(super) fn answer(&self, done: bool) -> TextInNumbers {
        TextInNumbers {
            done,
            min_chars: self.min_chars as u32,
            numeric_bytes: self.runs.iter().map(|r| r.found.size_bits / 8).sum(),
            scanned_bytes: self.scanned,
            runs: self
                .runs
                .iter()
                .filter(|r| r.texts > 0)
                .map(|r| NumbersWithText {
                    path: r.path.clone(),
                    name: r.found.name.clone(),
                    what: r.found.what.clone(),
                    element_bits: r.found.element_bits,
                    offset_bits: r.offset_bits,
                    size_bits: r.found.size_bits,
                    scanned_bytes: r.scanned,
                    texts: r.texts,
                    text_bytes: r.text_bytes,
                    first: r.first.clone(),
                })
                .collect(),
        }
    }
}

/// Whether some byte of every number `text` covers is the same byte: numbers
/// close to one another rather than text. `phase` is where `text` starts
/// inside a number of `width` bytes. At least three numbers have to agree.
fn strided(text: &[u8], phase: u64, width: u64) -> bool {
    let (phase, width) = (phase as usize, width as usize);
    (0..width).any(|k| {
        // The first byte of `text` that is byte `k` of a number.
        let first = (k + width - phase) % width;
        let mut at = text.iter().skip(first).step_by(width);
        let Some(b) = at.next() else { return false };
        let mut n = 1;
        for c in at {
            if c != b {
                return false;
            }
            n += 1;
        }
        n >= 3
    })
}

/// The whole bytes of a run to read: all of it, or its two ends when it is
/// longer than one run's cap.
fn pieces(offset_bits: u64, size_bits: u64) -> [Option<(u64, u64)>; 2] {
    let from = offset_bits.div_ceil(8);
    let to = (offset_bits + size_bits) / 8;
    if to <= from {
        return [None, None];
    }
    if to - from <= SCAN_ONE {
        return [Some((from, to)), None];
    }
    let half = SCAN_ONE / 2;
    [Some((from, from + half)), Some((to - half, to))]
}

/// Bytes read from the walk's space, as a source the scanner can take: the
/// same offsets as the space, and nothing before or after what was read. The
/// scanner looks a few bytes before where it starts for a length in front of
/// a string, and finds zeros there.
struct Window {
    base: u64,
    bytes: Vec<u8>,
}

impl Source for Window {
    fn len_bytes(&self) -> u64 {
        self.base + self.bytes.len() as u64
    }

    fn read_bytes(&self, offset: u64, out: &mut [u8]) -> Vec<Missing> {
        for (i, b) in out.iter_mut().enumerate() {
            let at = offset + i as u64;
            *b = at.checked_sub(self.base).and_then(|k| self.bytes.get(k as usize)).copied().unwrap_or(0);
        }
        Vec::new()
    }
}

impl Evaluator {
    /// What the report needs to know about a run of numbers the walk is
    /// about to go into, or None for anything else.
    pub(super) fn numbers_found<S: Source>(&mut self, doc: &Document<S>, path: &[usize], r: &Resolved) -> R<Option<Found>> {
        let Some((NotText::Numbers, what)) = self.not_text(path) else { return Ok(None) };
        let fixed = match &self.memo[path].ty {
            Ty::Array { elem, .. } | Ty::Repeat { elem, .. } => fixed_width(&self.template, elem),
            _ => false,
        };
        if !fixed {
            return Ok(None);
        }
        let name = self.label(doc, path, r)?;
        let size_bits = self.size_of(doc, path)?;
        let mut first = path.to_vec();
        first.push(0);
        let element_bits = match self.resolve(doc, &first).and_then(|()| self.size_of(doc, &first)) {
            Ok(bits) => bits,
            Err(e) if e.interrupted() => return Err(e),
            Err(_) => 0,
        };
        if element_bits < 16 || element_bits % 8 != 0 {
            return Ok(None);
        }
        Ok(Some(Found { name, what, element_bits, size_bits }))
    }
}

/// Whether a list's element is a number of one fixed width: an integer, a
/// float or a fixed-point number, or a name or flags over one.
fn fixed_width(template: &Template, elem: &Ty) -> bool {
    match super::readas::plain(template, elem) {
        Ty::Enum { inner, .. } | Ty::Flags { inner, .. } => fixed_width(template, inner),
        Ty::UInt { .. }
        | Ty::Int { .. }
        | Ty::SignMagnitude { .. }
        | Ty::F16(_)
        | Ty::BF16(_)
        | Ty::F32(_)
        | Ty::F64(_)
        | Ty::F80(_)
        | Ty::IbmF32(_)
        | Ty::Fixed { .. } => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_close_together_are_not_text() {
        // 32-bit IBM floats of one sign and size, from a SEG-Y trace.
        assert!(strided(b"A#33A#3=A#3HA#3RA#3]", 0, 4));
        // The same, starting one byte into a number.
        assert!(strided(b"#33A#3=A#3HA#3RA#3]", 1, 4));
        // 32-bit floats in a WAV, little-endian: the last byte is the same.
        assert!(strided(b"^=3y7=3y7=3y7=", 3, 4));
        assert!(!strided(b"File Name:         M01671.WAV", 0, 2));
        assert!(!strided(b"FW Version:        D500X V2.2.6", 1, 2));
    }

    #[test]
    fn a_long_run_is_read_at_both_ends() {
        assert_eq!(pieces(0x2c * 8, 300_000 * 8), [Some((0x2c, 0x2c + 300_000)), None]);
        let big = 3 * SCAN_ONE;
        assert_eq!(pieces(0, big * 8), [Some((0, SCAN_ONE / 2)), Some((big - SCAN_ONE / 2, big))]);
        // Whole bytes only.
        assert_eq!(pieces(4, 20), [Some((1, 3)), None]);
    }
}
