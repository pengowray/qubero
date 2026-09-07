//! Finding the text inside a file that is not a text file.
//!
//! The text view reads a file that was written to be read. This is for the
//! other kind: an executable, a game archive, a save file, a firmware image.
//! Such a file is mostly not text, and the text it does hold is the part a
//! reader can recognise without a template: error messages, format names,
//! paths, symbol names, the copyright line that says who wrote it.
//!
//! `strings(1)` answers that question with printable ASCII and a minimum
//! length, and stops there. Three things it does not say are the three things
//! worth knowing about a string in a binary:
//!
//! * **Which encoding.** A Windows binary's user-visible text is UTF-16 LE and
//!   its internal names are ASCII, in the same file, often in the same
//!   section. UTF-16 is scanned at both byte parities, because nothing aligns
//!   a string in a binary. A lone surrogate does not end a run: that is
//!   WTF-16, which is what a Windows filename or a V8 heap dump is full of,
//!   and a scanner that refused it would drop exactly the strings worth
//!   finding. The run says it held one.
//! * **Where the string ends, and who says so.** A C string ends at a zero
//!   byte. A Pascal, Java, .NET or protobuf string is counted by a number in
//!   front of it. Both are recorded, and the number is checked against the
//!   run every way it could have been meant: bytes, code units or characters,
//!   with or without the terminator counted in.
//! * **Where one string stops and the next starts.** Two counted strings laid
//!   end to end are one printable run, and reading them as one string is
//!   wrong. Where a chain of counted strings tiles a run exactly, the run is
//!   split into the strings it is.
//!
//! Nothing here scores a guess. A prefix that matches is reported with the
//! bytes it was read from and what it came to, so the reader can see the
//! arithmetic and disagree with it. A byte before a run is not text by
//! construction, so a number in it that happens to be the run's length is
//! worth reporting; whether it was meant as a length is the reader's call.
//!
//! Scanning is windowed, like [`search`](crate::search) and
//! [`textview`](crate::textview): the file is not loaded, a window of it is
//! read, and the next window is asked for when it is wanted.

use crate::source::{Missing, Source};

mod pairs;
mod prefix;
mod runs;
mod tables;
mod text;
#[cfg(test)]
mod tests;

use pairs::*;
use prefix::*;
use runs::*;
use tables::*;
use text::*;
pub use prefix::{Counts, Prefix, PrefixKind, MAX_PREFIX_READINGS};

/// Bytes scanned in one window.
pub const WINDOW: u64 = 64 * 1024;
/// How much past a window is also read, so a run starting at a window's last
/// byte is seen whole rather than cut by where the read stopped. Twice
/// [`MAX_BYTES`], which is the longest a run can be before it is cut on
/// purpose.
pub const OVERLAP: u64 = 8 * 1024;
/// Bytes read before a window, for the length prefix of a run that starts at
/// its first byte. Wider than the widest prefix (four bytes) and than the
/// longest LEB128 (ten).
pub const LOOKBACK: u64 = 16;
/// The longest run reported as one string. A run longer than this is cut and
/// says it was cut, and the next window carries on from the cut. Without a cap
/// a base64 blob is one string a megabyte long, which is not a string.
pub const MAX_BYTES: u64 = 4096;
/// How many windows one [`scan`] call reads before answering with what it has,
/// however few strings that was. A binary can hold megabytes with nothing
/// printable in them, and a call that scanned to the next hit whatever it cost
/// would hold the thread for as long as that took. Half a megabyte is about
/// forty milliseconds of scanning with every reading turned on, which is what
/// a caller can spend without the page being felt to stop.
pub const MAX_WINDOWS: usize = 8;
/// What `strings(1)` uses, and for the same reason: three printable bytes in a
/// row happen by accident often enough to bury the ones that did not.
pub const MIN_CHARS_DEFAULT: usize = 4;
/// How a run of text was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Enc {
    /// Printable ASCII, which is also every single-byte code page's lower half.
    Ascii,
    /// Printable ASCII with at least one multi-byte UTF-8 character in it.
    Utf8,
    Utf16Le,
    Utf16Be,
}
impl Enc {
    pub fn name(self) -> &'static str {
        match self {
            Enc::Ascii => "ASCII",
            Enc::Utf8 => "UTF-8",
            Enc::Utf16Le => "UTF-16 LE",
            Enc::Utf16Be => "UTF-16 BE",
        }
    }

    /// Bytes in one code unit, which is what a run's length is a multiple of.
    pub fn unit(self) -> u64 {
        match self {
            Enc::Utf16Le | Enc::Utf16Be => 2,
            _ => 1,
        }
    }

    fn wide(self) -> bool {
        matches!(self, Enc::Utf16Le | Enc::Utf16Be)
    }

    fn big(self) -> bool {
        matches!(self, Enc::Utf16Be)
    }
}
/// The zero that ends a string, where there is one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Term {
    /// One zero byte, after an eight-bit string.
    Nul,
    /// Two, after a UTF-16 one.
    NulNul,
}
impl Term {
    pub fn bytes(self) -> u64 {
        match self {
            Term::Nul => 1,
            Term::NulNul => 2,
        }
    }
}
/// One string found in the file.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    /// First byte of the text. A prefix sits before this and a terminator
    /// after it; neither is part of the text.
    pub at: u64,
    /// Bytes of text, not counting the prefix or the terminator.
    pub len: u64,
    pub enc: Enc,
    pub chars: u32,
    pub units: u32,
    pub text: String,
    /// True when the run held a UTF-16 surrogate with no partner, which is
    /// WTF-16 rather than UTF-16. Such a character reads as U+FFFD in `text`.
    pub lone_surrogates: bool,
    /// Every reading of the bytes in front that comes to this run's length.
    /// More than one usually means the same number written at several widths.
    pub prefix: Vec<Prefix>,
    pub term: Option<Term>,
    /// True when the run went past [`MAX_BYTES`] and was cut here. The text
    /// carries on in the string after this one.
    pub cut: bool,
}
/// What to look for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Opts {
    /// Shortest run reported, in characters.
    pub min_chars: usize,
    /// ASCII and UTF-8, which are one pass: UTF-8 is ASCII with wider
    /// characters allowed in it.
    pub ascii: bool,
    pub utf16le: bool,
    pub utf16be: bool,
}
impl Default for Opts {
    fn default() -> Self {
        Opts { min_chars: MIN_CHARS_DEFAULT, ascii: true, utf16le: true, utf16be: true }
    }
}
impl Opts {
    fn min(self) -> usize {
        self.min_chars.clamp(1, 1024)
    }
}
/// What one scan found, and where the next one starts.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scan {
    pub hits: Vec<Hit>,
    /// Where to carry on from. Equal to `from` when nothing was readable.
    pub next: u64,
    /// Chunks this scan needs before it can answer.
    pub missing: Vec<Missing>,
}
/// Strings from `from` onwards, stopping at `want` of them, at
/// [`MAX_WINDOWS`] windows, or at the end of the file.
///
/// `from` must be where a previous scan said to carry on from, or zero. It is
/// allowed to be in the middle of a run: that is what a run cut at
/// [`MAX_BYTES`] leaves behind, and the string starting there is the rest of
/// the one before it.
pub fn scan<S: Source>(src: &S, from: u64, want: usize, opts: Opts) -> Scan {
    let end = src.len_bytes();
    let mut at = from.min(end);
    let mut hits: Vec<Hit> = Vec::new();
    for _ in 0..MAX_WINDOWS {
        if at >= end || hits.len() >= want {
            break;
        }
        let lo = at.saturating_sub(LOOKBACK);
        let hi = (at + WINDOW + OVERLAP).min(end);
        let mut buf = vec![0u8; (hi - lo) as usize];
        let missing = src.read_bytes(lo, &mut buf);
        if !missing.is_empty() {
            return Scan { hits, next: at, missing };
        }
        let stop = (at + WINDOW).min(end);
        let before = hits.len();
        // Whether the file carries on past what was read. A run that fills the
        // buffer is cut by the read rather than by the limit, and only this
        // says which.
        window(&buf, lo, at, stop, hi < end, opts, &mut hits);
        // A run that reaches past this window was reported here, so the next
        // window starts after it rather than finding its tail and calling that
        // a string of its own. A run cut at MAX_BYTES ends at the cut, and the
        // next window carries the rest.
        let reached = hits[before..].iter().map(|h| h.at + h.len).max().unwrap_or(0);
        at = stop.max(reached).min(end);
    }
    if hits.len() > want {
        hits.truncate(want.max(1));
        // The ones dropped all start after the last one kept, so carrying on
        // from its end finds them again rather than losing them.
        at = hits.last().map_or(at, |h| h.at + h.len);
    }
    Scan { hits, next: at, missing: Vec::new() }
}
/// A run of text found in a window, before it is read or explained.
#[derive(Debug, Clone, Copy)]
struct Run {
    /// Buffer index, not a file offset.
    start: usize,
    end: usize,
    enc: Enc,
    chars: u32,
    units: u32,
    lone: bool,
    /// How good an account of the bytes this reading is, for deciding between
    /// two readings that cover the same stretch. Three where nothing has to be
    /// assumed: an eight-bit run, or a wide one whose characters are all in
    /// the first Unicode page, which is what wide Latin text is. Two where the
    /// reading is possible but less plain.
    quality: u64,
}
impl Run {
    fn len(&self) -> usize {
        self.end - self.start
    }

    /// What decides between overlapping readings. Length carries it, because a
    /// reading that accounts for more of the file is the better account; the
    /// quality breaks the ties, which is where `01 02 03` in front of UTF-16
    /// LE text is otherwise read as two wide characters and the rest big-endian.
    fn rank(&self) -> u64 {
        self.quality * self.len() as u64
    }
}
/// Every string starting in `[from, stop)`, appended to `out`.
///
/// `buf` holds `[base, ...)`, which starts before `from` so that a run at
/// `from` can have its prefix read. Runs are looked for from `from` only: a
/// run reaching back before it belongs to the window that already reported it.
fn window(buf: &[u8], base: u64, from: u64, stop: u64, more: bool, opts: Opts, out: &mut Vec<Hit>) {
    let head = (from - base) as usize;
    let stop_i = (stop - base) as usize;
    let min = opts.min();
    let mut runs: Vec<Run> = Vec::new();
    // Bytes that read as a column of numbers rather than as characters. A
    // table is a property of the bytes and not of one reading of them, so
    // every reading of a stretch marked here goes: refusing only the reading
    // that gave the table away hands the bytes to the shifted one, which is
    // the same table with its bytes paired up differently and passes.
    let mut tables = vec![false; buf.len()];
    if opts.ascii {
        narrow_runs(buf, head, min, &mut runs);
    }
    if opts.utf16le {
        wide_runs(buf, head, min, Enc::Utf16Le, &mut runs, &mut tables);
    }
    if opts.utf16be {
        wide_runs(buf, head, min, Enc::Utf16Be, &mut runs, &mut tables);
    }
    // What counts each run, and which of them are part of a table of counted
    // strings. Both answers are wanted twice over: they decide whether a wide
    // run is reported at all, and whether a number exactly one code unit wide
    // is worth repeating to the reader. See `counted_by` and `in_a_table`.
    let counted = counted_by(buf, base, &runs);
    let table = in_a_table(&runs, &counted);
    // A wide run has to say why it is one, and is asked before the readings
    // compete for the bytes, so a run that cannot answer does not take a
    // stretch away from an eight-bit reading that could.
    runs.retain(|r| {
        !r.enc.wide() || tables[r.start..r.end].iter().filter(|t| **t).count() * 2 <= r.end - r.start
    });
    let vouch: Vec<bool> = runs
        .iter()
        .enumerate()
        .map(|(i, r)| !r.enc.wide() || vouched(buf, *r, &counted[i], table[i]))
        .collect();
    let mut ok = vouch.clone();
    shifted_readings(buf, base, &runs, &vouch, &table, &mut ok);
    let mut ranked: Vec<(Run, bool)> = runs
        .into_iter()
        .enumerate()
        .filter(|(i, _)| ok[*i])
        .map(|(i, r)| (r, table[i]))
        .collect();
    // One stretch of bytes is one string, so where two readings of it overlap
    // only one of them is reported. The best account of the bytes takes them,
    // and what is left goes to whatever still fits beside it.
    // A run starting on an even address wins a tie, because UTF-16 in a file
    // is nearly always laid on two-byte boundaries. It only settles ties: a
    // run at an odd address that is the better reading is still the one taken,
    // which is what finds the wide strings a linker put wherever they fitted.
    let aligned = |r: &Run| (base + r.start as u64) % 2 == 0;
    ranked.sort_by(|(a, _), (b, _)| {
        b.rank().cmp(&a.rank()).then(aligned(b).cmp(&aligned(a))).then(a.start.cmp(&b.start))
    });
    let mut claimed = vec![false; buf.len()];
    let mut taken: Vec<(Run, bool)> = Vec::new();
    for (run, weak) in ranked {
        if run.start >= stop_i || claimed[run.start..run.end].iter().any(|c| *c) {
            continue;
        }
        claimed[run.start..run.end].fill(true);
        taken.push((run, weak));
    }
    taken.sort_by_key(|(r, _)| r.start);
    for (run, weak) in taken {
        emit(buf, base, run, more, weak, opts, out);
    }
}
/// Split a run where a chain of counted strings tiles it, and read out each
/// piece. Most runs are one piece.
fn emit(buf: &[u8], base: u64, run: Run, more: bool, weak: bool, opts: Opts, out: &mut Vec<Hit>) {
    let cap = MAX_BYTES as usize;
    // A run that fills the buffer was stopped by the read and not by its own
    // end, so its last piece carries on in the next window just as a cut one
    // does. Saying otherwise would put a string on screen that starts in the
    // middle of a word with nothing to say why.
    let unfinished = more && run.end == buf.len();
    if run.len() > cap || unfinished {
        // Cut on a code unit, so half a UTF-16 character is never a string's
        // last byte. Each piece says it was cut, and the text carries on in
        // the piece after it.
        let unit = run.enc.unit() as usize;
        let keep = cap - cap % unit;
        let mut at = run.start;
        while at < run.end {
            let end = (at + keep).min(run.end);
            let (chars, units, lone) = measure(buf, at, end, run.enc);
            let mut hit = read_hit(buf, base, at, end, run.enc, chars, units, lone > 0);
            hit.cut = end < run.end || unfinished;
            // Only the last piece can have one: a cut falls inside the text,
            // where the next byte is text and not a zero.
            hit.term = terminator(buf, end, run.enc);
            if at == run.start {
                hit.prefix = prefix_readings(buf, base, at, hit.len, units, chars, run.enc, None);
            }
            out.push(hit);
            at = end;
        }
        return;
    }
    let pieces = chain(buf, base, run, opts);
    if pieces.len() > 1 {
        for (start, end, prefix) in pieces {
            let (chars, units, lone) = measure(buf, start, end, run.enc);
            let mut hit = read_hit(buf, base, start, end, run.enc, chars, units, lone > 0);
            hit.prefix = vec![prefix];
            out.push(hit);
        }
        return;
    }
    let mut hit = read_hit(buf, base, run.start, run.end, run.enc, run.chars, run.units, run.lone);
    hit.term = terminator(buf, run.end, run.enc);
    hit.prefix = prefix_readings(buf, base, run.start, hit.len, run.units, run.chars, run.enc, hit.term);
    // A number exactly as wide as one character is the run's own boundary read
    // a second time unless a neighbour is counted the same way. It is still
    // shown; it is marked for what it is. See `Prefix::weak` and `in_a_table`.
    let unit = run.enc.unit() as usize;
    for p in &mut hit.prefix {
        p.weak = !weak && p.raw.len() <= unit;
    }
    out.push(hit);
}
/// A hit with its text decoded, and nothing said yet about what is around it.
fn read_hit(buf: &[u8], base: u64, start: usize, end: usize, enc: Enc, chars: u32, units: u32, lone: bool) -> Hit {
    Hit {
        at: base + start as u64,
        len: (end - start) as u64,
        enc,
        chars,
        units,
        text: decode(&buf[start..end], enc),
        lone_surrogates: lone,
        prefix: Vec::new(),
        term: None,
        cut: false,
    }
}
/// The zero that ends a string, if one follows the text.
fn terminator(buf: &[u8], end: usize, enc: Enc) -> Option<Term> {
    if enc.wide() {
        match buf.get(end..end + 2) {
            Some([0, 0]) => Some(Term::NulNul),
            _ => None,
        }
    } else if buf.get(end) == Some(&0) {
        Some(Term::Nul)
    } else {
        None
    }
}
