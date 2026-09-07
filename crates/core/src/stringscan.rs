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

/// The most readings of one prefix that are reported. Several are usually the
/// same number written at different widths, and the fourth adds nothing.
pub const MAX_PREFIX_READINGS: usize = 4;

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

/// How the number in front of a string is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrefixKind {
    U8,
    U16Le,
    U16Be,
    U32Le,
    U32Be,
    U64Le,
    U64Be,
    /// Seven bits a byte, low group first, high bit set on every byte but the
    /// last. What .NET's `BinaryWriter` writes, and what a protobuf or a DEX
    /// string is counted by.
    Leb128,
}

impl PrefixKind {
    pub fn name(self) -> &'static str {
        match self {
            PrefixKind::U8 => "u8",
            PrefixKind::U16Le => "u16 LE",
            PrefixKind::U16Be => "u16 BE",
            PrefixKind::U32Le => "u32 LE",
            PrefixKind::U32Be => "u32 BE",
            PrefixKind::U64Le => "u64 LE",
            PrefixKind::U64Be => "u64 BE",
            PrefixKind::Leb128 => "LEB128",
        }
    }

    /// Bytes it takes, or none for the one whose width is in its bytes.
    fn width(self) -> Option<usize> {
        match self {
            PrefixKind::U8 => Some(1),
            PrefixKind::U16Le | PrefixKind::U16Be => Some(2),
            PrefixKind::U32Le | PrefixKind::U32Be => Some(4),
            PrefixKind::U64Le | PrefixKind::U64Be => Some(8),
            PrefixKind::Leb128 => None,
        }
    }
}

/// Widest first, and little-endian before big: a `05 00 00 00` reads as a
/// 32-bit five and nothing else, while `00 00 00 05` reads as a 32-bit five, a
/// 16-bit five and an 8-bit five at once. Reporting the widest first puts the
/// whole number in front of the parts of it.
const PREFIX_ORDER: [PrefixKind; 8] = [
    PrefixKind::U64Le,
    PrefixKind::U64Be,
    PrefixKind::U32Le,
    PrefixKind::U32Be,
    PrefixKind::U16Le,
    PrefixKind::U16Be,
    PrefixKind::Leb128,
    PrefixKind::U8,
];

/// What the number in front of a string counts. They are the same number for
/// ASCII and differ for everything else, which is why a match says which one
/// it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Counts {
    Bytes,
    /// Code units: UTF-16 units, so half the bytes.
    Units,
    /// Characters, which a surrogate pair makes one of and two code units.
    Chars,
}

impl Counts {
    pub fn name(self) -> &'static str {
        match self {
            Counts::Bytes => "bytes",
            Counts::Units => "code units",
            Counts::Chars => "characters",
        }
    }
}

/// A number in front of a run that comes to the run's length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prefix {
    pub kind: PrefixKind,
    /// Where the number starts, which is before the text.
    pub at: u64,
    /// The bytes it was read from, so the reading can be checked.
    pub raw: Vec<u8>,
    pub value: u64,
    pub counts: Counts,
    /// True when the number counts the terminator as well as the text.
    pub with_terminator: bool,
    /// True when the number is no wider than one character of the run and
    /// nothing else vouches for it, which makes it the run's own boundary read
    /// a second time rather than a fact about the file.
    ///
    /// A run is maximal, so whatever sits in front of it is a value that could
    /// not be part of it, and those are the small values, which is also what a
    /// short string's length looks like. On the sample collection that
    /// coincidence happened thirteen thousand times. The reading is still
    /// shown, because the bytes are there and the arithmetic is the reader's
    /// to check, but it is shown for what it is.
    pub weak: bool,
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
    if opts.ascii {
        narrow_runs(buf, head, min, &mut runs);
    }
    if opts.utf16le {
        wide_runs(buf, head, min, Enc::Utf16Le, &mut runs);
    }
    if opts.utf16be {
        wide_runs(buf, head, min, Enc::Utf16Be, &mut runs);
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
    let vouch: Vec<bool> = runs
        .iter()
        .enumerate()
        .map(|(i, r)| !r.enc.wide() || vouched(buf, *r, &counted[i], table[i]))
        .collect();
    let mut ok = vouch.clone();
    shifted_readings(buf, base, &runs, &vouch, &mut ok);
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

/// Which kinds of number in front of each run read as its length, with how
/// many bytes each takes and what it came to.
fn counted_by(buf: &[u8], base: u64, runs: &[Run]) -> Vec<Vec<(PrefixKind, usize, u64)>> {
    runs.iter()
        .map(|run| {
            let term = terminator(buf, run.end, run.enc);
            let len = (run.end - run.start) as u64;
            prefix_readings(buf, base, run.start, len, run.units, run.chars, run.enc, term)
                .iter()
                .map(|p| (p.kind, p.raw.len(), p.value))
                .collect()
        })
        .collect()
}

/// How many strings a kind of number has to count, and what share of the runs
/// in the window, before it is taken to be how this file counts its strings.
///
/// A one-byte number matches a run's length by accident about one time in a
/// hundred and sixty, so a window of a thousand runs throws up half a dozen
/// for nothing. A file that really counts its strings that way counts nearly
/// all of them. An eighth of the runs is twenty times what chance produces and
/// well under what a format produces, and the floor stops a window holding
/// three runs from proving anything.
const TABLE_LEAST: usize = 3;
const TABLE_SHARE: usize = 8;

/// How many different numbers a kind has to take before it is counting
/// anything rather than delimiting it.
const TABLE_VALUES: usize = 3;

/// Which runs sit in a table of counted strings.
///
/// This is the only thing that makes a number no wider than one character
/// worth anything. A run is maximal, so whatever sits in front of it is a
/// value that could not be part of it, and the values that could not be are
/// the small ones, which is also what a short string's length looks like: a
/// match there is the run's own boundary read a second time, and on the sample
/// collection it happened eleven thousand times.
///
/// What says otherwise is the same kind of number counting a good share of
/// every string in the window, and taking a different value as it goes. A
/// pickle, a Thrift structure and a MATLAB file all look like that, their
/// strings scattered through metadata rather than packed together.
///
/// Both halves of that are needed. Without the share, six coincidences in a
/// thousand runs would speak for the file. Without the variation, a delimiter
/// would: a run of format strings separated by newlines has 0x0a in front of
/// every one of them, and `Access: %x`, `Modify: %y` and `Change: %z` are all
/// ten bytes long, so all three "match" and none of them is counted.
///
/// What this cannot do is speak for a table of five counted strings sitting in
/// a megabyte of code, since five matches in a window of five hundred runs is
/// what chance looks like too. Those readings are shown as the coincidences
/// they may be, with the bytes beside them.
fn in_a_table(runs: &[Run], counted: &[Vec<(PrefixKind, usize, u64)>]) -> Vec<bool> {
    use std::collections::HashMap;
    let mut uses: HashMap<PrefixKind, Vec<u64>> = HashMap::new();
    for row in counted {
        for &(kind, _, value) in row {
            uses.entry(kind).or_default().push(value);
        }
    }
    let n = runs.len();
    let counts = |kind: PrefixKind| {
        uses.get(&kind).is_some_and(|values| {
            if values.len() < TABLE_LEAST || values.len() * TABLE_SHARE < n {
                return false;
            }
            let mut seen: Vec<u64> = values.clone();
            seen.sort_unstable();
            seen.dedup();
            seen.len() >= TABLE_VALUES
        })
    };
    // One answer per kind rather than one per run: the question is about the
    // file, and a window holds thousands of runs.
    let told: HashMap<PrefixKind, bool> =
        uses.keys().map(|&kind| (kind, counts(kind))).collect();
    counted
        .iter()
        .map(|row| row.iter().any(|&(kind, _, _)| told.get(&kind).copied().unwrap_or(false)))
        .collect()
}

/// Settles which way round a stretch of wide text was written.
///
/// UTF-16 whose characters sit in one page of 256 reads the same at the other
/// endianness one byte over, less its first character: `70 00 72 00` is "pr"
/// little-endian and "r" big-endian starting a byte later. Every ASCII string
/// in a UTF-16 file is like that, so both readings are always found and one of
/// them has to go. The shifted one is always the later of the two, since what
/// it loses is the first character.
///
/// What tells them apart is the byte the later reading starts on, which is the
/// second byte of the earlier reading's first character. Little-endian puts
/// the character first and the zero second, so that byte is a zero and the
/// byte in front of it is a letter: the later reading has started halfway
/// through a character and is the shifted one. Big-endian puts the zero first,
/// so the byte in front is a zero and nothing is settled, which is right,
/// because for big-endian text the later reading is the shifted one for the
/// same reason and there is nothing to choose it by. Then the reading the file
/// vouched for wins, then the longer one, then the one on an even address.
///
/// Whichever survives is vouched for by anything either reading found, since a
/// terminator or a length belongs to the text rather than to one way of
/// reading it. A .NET `#US` string is a length, its characters, then a flag
/// byte, and that flag byte with the next string's length reads as `00 00` to
/// the shifted run and as nothing at all to the real one, so `System.dll`
/// reported three and a half thousand strings missing their first letter.
fn shifted_readings(buf: &[u8], base: u64, runs: &[Run], vouch: &[bool], ok: &mut [bool]) {
    use std::collections::HashMap;
    let mut at: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, r) in runs.iter().enumerate() {
        if r.enc.wide() {
            at.entry(r.start).or_default().push(i);
        }
    }
    for i in 0..runs.len() {
        if !runs[i].enc.wide() {
            continue;
        }
        let Some(js) = at.get(&(runs[i].start + 1)) else { continue };
        for &j in js {
            if runs[j].enc == runs[i].enc {
                continue;
            }
            let keep = if ascii_text(buf[runs[i].start]) {
                i
            } else if vouch[i] != vouch[j] {
                if vouch[i] { i } else { j }
            } else if runs[i].chars != runs[j].chars {
                if runs[i].chars > runs[j].chars { i } else { j }
            } else if (base + runs[i].start as u64) % 2 == 0 {
                // UTF-16 in a file is nearly always laid on two-byte
                // boundaries. The two starts are a byte apart, so exactly one
                // of them is even.
                i
            } else {
                j
            };
            let (keep, drop) = if keep == i { (i, j) } else { (j, i) };
            ok[keep] = vouch[i] || vouch[j];
            ok[drop] = false;
        }
    }
}

/// Whether anything around a wide run says it was written as a string.
///
/// A wide run gets its evidence too cheaply. Four printable ASCII bytes in a
/// row are four bytes of a file agreeing; four printable UTF-16 LE characters
/// of Latin text are four bytes agreeing and four bytes that only have to be
/// zero, and quiet audio, a depth buffer and a table of small integers all
/// supply those zeroes for nothing. Measured on a recording of a bat: three
/// hundred kilobytes of samples produced thirty eight-bit strings and four
/// thousand nine hundred wide ones, none of them a string.
///
/// So a wide run is reported only when something around it says it was written
/// as one. Three things can: a zero code unit after it, which is two bytes that
/// had to be anything and are zero; a number in front of it wider than one code
/// unit, which has to carry bytes the run's own boundary did not need; or a
/// place in a table, which is what [`in_a_table`] answers.
///
/// The eight-bit pass is deliberately not held to any of this. Four printable
/// bytes in a row is what `strings(1)` reports and what a reader of this view
/// expects, noise and all.
fn vouched(buf: &[u8], run: Run, counted: &[(PrefixKind, usize, u64)], table: bool) -> bool {
    let unit = run.enc.unit() as usize;
    table || terminator(buf, run.end, run.enc).is_some() || counted.iter().any(|&(_, w, _)| w > unit)
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

// --- what counts as text ---------------------------------------------------

/// Whether a character is one a string would hold. Tab is text; the other
/// controls are not, and a newline ends a run rather than joining two lines
/// into one string, which is what `strings(1)` does and what makes a list of
/// found strings readable.
fn printable(c: char) -> bool {
    if c == '\t' {
        return true;
    }
    if c.is_control() || c == '\u{fffd}' {
        return false;
    }
    // Noncharacters are guaranteed never to be a character, so a run of them
    // is never text. It is worth naming them: a firmware image padded with
    // 0xff reads two bytes at a time as U+FFFF over and over, which is
    // otherwise a perfectly coherent run of one Unicode page.
    let u = c as u32;
    !(0xfdd0..=0xfdef).contains(&u) && (u & 0xfffe) != 0xfffe
}

/// Whether a character can be the first one of a string.
///
/// A combining mark attaches to the character before it, and a format
/// character is invisible, so neither begins a string. Refusing them is worth
/// more than it looks: arbitrary bytes land on the combining block often
/// enough that letting one start a run makes the run one character longer
/// than it is, which is enough to lose a reading to a wrong one that covers
/// more of the file. What is refused is a start, not a character: a mark
/// inside a string is the string's, and stays.
fn starter(c: char) -> bool {
    if !printable(c) {
        return false;
    }
    let u = c as u32;
    !matches!(u,
        0x0300..=0x036f      // combining diacritics
        | 0x0483..=0x0489    // Cyrillic marks
        | 0x0590..=0x05bf    // Hebrew points
        | 0x0610..=0x061a    // Arabic marks
        | 0x064b..=0x065f
        | 0x0e31 | 0x0e34..=0x0e3a | 0x0e47..=0x0e4e   // Thai marks
        | 0x1ab0..=0x1aff    // combining extensions
        | 0x1dc0..=0x1dff
        | 0x00ad             // soft hyphen
        | 0x200b..=0x200f    // zero width and direction marks
        | 0x202a..=0x202e
        | 0x2060..=0x206f
        | 0x20d0..=0x20f0    // combining marks for symbols
        | 0xfe00..=0xfe0f    // variation selectors
        | 0xfe20..=0xfe2f
        | 0xfeff)
}

/// Whether a byte on its own would be printable ASCII, which is what tells a
/// genuine UTF-16 run from an ASCII one misread as wide characters.
fn ascii_text(b: u8) -> bool {
    b == b'\t' || (0x20..0x7f).contains(&b)
}

/// Whether a wide run's characters all come from one part of Unicode.
///
/// This is what tells wide text from arbitrary bytes read two at a time.
/// Almost every sixteen-bit number is some printable character, so a stretch
/// of compiled code read as UTF-16 is a run of them, and a scanner with
/// nothing to say about that reports a page of gibberish beside the strings
/// worth reading. Real text does not wander: a run of it is Latin, or Greek,
/// or Cyrillic, and the high byte of its characters says which.
///
/// How much wandering is allowed depends on how long the run is, because one
/// stray character out of four is a different claim from one out of forty. A
/// run is allowed one character from somewhere else for every eight it has,
/// which is none at all at the shortest length reported. That is what
/// separates a MIPS instruction word, which is three characters of one page
/// and one of another, from a sentence.
///
/// Half of a surrogate pair is never counted against a run: an emoji in a line
/// of Latin text is two units from a page of its own and is still the line the
/// reader wrote.
fn one_page(buf: &[u8], start: usize, end: usize, big: bool) -> bool {
    let Some(page) = dominant_page(buf, start, end, big) else { return false };
    if !text_page(page) {
        return false;
    }
    let mut units = 0u32;
    let mut stray = 0u32;
    let mut i = start;
    while i + 2 <= end {
        let u = unit_at(buf, i, big).unwrap_or(0);
        if (u >> 8) as u8 != page && !(0xd800..0xe000).contains(&u) {
            // A character from somewhere else, with a control byte for its
            // low half, is not a character at all: it is the end of one
            // string and the start of the number counting the next, read as
            // one unit. A .NET user string heap joins its lines that way,
            // three hundred of them, and the allowance below would let the
            // whole heap pass as one string of ten thousand characters.
            if u & 0xff < 0x20 {
                return false;
            }
            stray += 1;
        }
        units += 1;
        i += 2;
    }
    units > 0 && stray <= units / 8
}

/// The part of a wide run that is the string, without what it ran into at
/// either end.
///
/// A run is maximal, and what lies either side of a string in a binary is
/// whatever the compiler put there. Two bytes of that are usually some
/// character, so a run reaches back over the pointer in front of the string
/// and forward over the one behind it, and those characters are why an
/// otherwise coherent run stops looking coherent. Both ends are cut back:
/// first the units that are eight-bit text read wide, then the units that are
/// not from the part of Unicode the rest of the run is from. What is left is
/// the string, and the tests that follow are asked about that rather than
/// about it plus its neighbours.
///
/// A surrogate is never cut, since half of a pair at the end of a run is the
/// character the reader came for.
fn trim(buf: &[u8], start: usize, end: usize, big: bool) -> (usize, usize) {
    let high = usize::from(!big);
    let mut a = start;
    let mut b = end;
    while a + 2 <= b && ascii_text(buf[a + high]) {
        a += 2;
    }
    while b >= a + 2 && ascii_text(buf[b - 2 + high]) {
        b -= 2;
    }
    let Some(page) = dominant_page(buf, a, b, big) else { return (a, b) };
    let odd = |i: usize| {
        let u = unit_at(buf, i, big).unwrap_or(0);
        (u >> 8) as u8 != page && !(0xd800..0xe000).contains(&u)
    };
    while a + 2 <= b && odd(a) {
        a += 2;
    }
    while b >= a + 2 && odd(b - 2) {
        b -= 2;
    }
    (a, b)
}

/// Whether a part of Unicode is one a run of text is written in.
///
/// Everything above the surrogates is private use, compatibility forms,
/// halfwidth and fullwidth forms, and specials. Nobody writes a string in
/// those, and every one of them is where sixteen-bit numbers land: eight-bit
/// audio at low amplitude has a high byte of 0xff, and read two bytes at a
/// time that is a run of fullwidth punctuation, coherent and printable and not
/// a string. Measured on a recording of a bat, this is what the last three
/// hundred false readings had in common, and every one of them had a zero
/// after it as well, since a silent sample is two zero bytes.
///
/// The cost is a run written entirely in fullwidth forms or halfwidth katakana
/// and in nothing else, which goes the way CJK-only text goes and for the same
/// reason. A run with any ordinary text in it keeps its page and is found.
fn text_page(page: u8) -> bool {
    page < 0xd8
}

/// The Unicode page most of a run's characters are from.
fn dominant_page(buf: &[u8], start: usize, end: usize, big: bool) -> Option<u8> {
    let mut pages = [0u32; 256];
    let mut i = start;
    while i + 2 <= end {
        pages[(unit_at(buf, i, big).unwrap_or(0) >> 8) as usize] += 1;
        i += 2;
    }
    let (page, n) = pages.iter().enumerate().max_by_key(|(_, n)| **n)?;
    (*n > 0).then_some(page as u8)
}

/// Whether a wide run's characters are characters, or a column of small
/// numbers under a constant high byte.
///
/// A Thrift field header, a table of flags and a run of enum values all read
/// as wide text with the same page over and over and a low byte that never
/// leaves the control range: `15 10 15 04 15 06 15 08` is four perfectly good
/// Canadian Syllabics characters and is a Parquet record. In a script, the low
/// byte moves through the block: the letters are spread across it, and a word
/// of four cannot have all four sitting in the first thirty-two places.
fn letters(buf: &[u8], start: usize, end: usize, big: bool) -> bool {
    let mut i = start;
    while i + 2 <= end {
        if unit_at(buf, i, big).unwrap_or(0) & 0xff >= 0x20 {
            return true;
        }
        i += 2;
    }
    false
}

/// Whether a wide run says more than one thing.
///
/// A stretch of 90 90 90 90 is x86 padding and reads as a row of the same
/// character; so does a run of zeroes in a table, or a fill byte in a disk
/// image. None of them is a string, and every one of them passes every other
/// test here, because one character repeated is perfectly coherent. Real text
/// spreads itself: no character in a line of it takes two thirds of the line,
/// and even the shortest line has three different characters in it.
fn varied(buf: &[u8], start: usize, end: usize, big: bool) -> bool {
    let mut seen: std::collections::HashMap<u16, u32> = std::collections::HashMap::new();
    let mut units = 0u32;
    let mut most = 0u32;
    let mut i = start;
    while i + 2 <= end {
        let n = seen.entry(unit_at(buf, i, big).unwrap_or(0)).or_insert(0);
        *n += 1;
        most = most.max(*n);
        units += 1;
        i += 2;
    }
    // Three different characters, which every string of four has and an
    // alternating pair of numbers does not: `01 00 00 01 01 00 00 01` reads as
    // two characters taking turns, and that is a table, not a word.
    units > 0 && most * 3 <= units * 2 && (seen.len() >= 3 || units < 3)
}

/// Whether a run of wide characters is one, or eight-bit text read two bytes
/// at a time.
///
/// Every English sentence in a file is also a run of perfectly good CJK when
/// the bytes are taken in pairs, so something has to tell the two apart, and
/// it is the high byte of each unit. A UTF-16 character of a Latin, Greek,
/// Cyrillic, Hebrew, Arabic or Indic script has a high byte below 0x20, and
/// half of a surrogate pair has one above 0x7f. Eight-bit text read as wide
/// characters has a printable byte in both halves of every unit. Half the
/// units decide it, so one odd pair in a run does not lose the run.
///
/// The cost is that a file whose wide text is nothing but CJK or kana is not
/// found this way: those live where a pair of printable bytes lives, and
/// nothing in the bytes says which reading was meant. Such a file reads in the
/// text view with UTF-16 chosen by hand.
fn wide_enough(buf: &[u8], start: usize, end: usize, big: bool) -> bool {
    let high = usize::from(!big);
    let mut wide = 0usize;
    let mut units = 0usize;
    let mut i = start;
    while i + 2 <= end {
        units += 1;
        if !ascii_text(buf[i + high]) {
            wide += 1;
        }
        i += 2;
    }
    units > 0 && wide * 2 >= units
}

/// The character at `i`, read as UTF-8, and how many bytes it took.
fn utf8_char(buf: &[u8], i: usize) -> Option<(char, usize)> {
    let b = *buf.get(i)?;
    if b < 0x80 {
        return Some((b as char, 1));
    }
    let n = match b {
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return None,
    };
    let s = buf.get(i..i + n)?;
    let text = std::str::from_utf8(s).ok()?;
    let c = text.chars().next()?;
    Some((c, n))
}

/// Runs of printable bytes, taking a valid UTF-8 sequence as the one character
/// it is. A run holding one is UTF-8; a run of nothing but ASCII is ASCII,
/// which is true of it in every single-byte code page as well.
fn narrow_runs(buf: &[u8], from: usize, min: usize, out: &mut Vec<Run>) {
    let mut i = from;
    while i < buf.len() {
        let start = i;
        let mut chars = 0u32;
        let mut multi = 0u32;
        // The longest stretch of plain ASCII in the run, which is what the run
        // would be worth without any wide character in it.
        let mut plain = 0u32;
        let mut ascii = 0u32;
        let mut j = i;
        while let Some((c, n)) = utf8_char(buf, j) {
            if if chars == 0 { !starter(c) } else { !printable(c) } {
                break;
            }
            if n > 1 {
                multi += 1;
                ascii = 0;
            } else {
                ascii += 1;
                plain = plain.max(ascii);
            }
            chars += 1;
            j += n;
        }
        // One wide character does not make a run of compiled code into text,
        // and must not be what carries a run over the minimum. Two bytes of
        // x86 are a valid UTF-8 character often enough that taking one at its
        // word turns half a code section into strings: c6 8b is a perfectly
        // good character, and it welds ")" and "D$P" into a five-character
        // string that neither half was. So a run earns its place on a stretch
        // of ASCII long enough on its own, or on holding a run of wide
        // characters long enough on its own.
        if j > start {
            if chars as usize >= min && (plain as usize >= min || multi as usize >= min) {
                let enc = if multi > 0 { Enc::Utf8 } else { Enc::Ascii };
                out.push(Run { start, end: j, enc, chars, units: (j - start) as u32, lone: false, quality: 3 });
            }
            i = j;
        } else {
            i += 1;
        }
    }
}

fn unit_at(buf: &[u8], i: usize, big: bool) -> Option<u16> {
    let pair = buf.get(i..i + 2)?;
    Some(if big { u16::from_be_bytes([pair[0], pair[1]]) } else { u16::from_le_bytes([pair[0], pair[1]]) })
}

fn is_high_surrogate(u: u16) -> bool {
    (0xd800..0xdc00).contains(&u)
}

fn is_low_surrogate(u: u16) -> bool {
    (0xdc00..0xe000).contains(&u)
}

/// Runs of UTF-16, at both byte parities: nothing in a binary aligns a string,
/// and a UTF-16 run starting at an odd address is as common as one starting at
/// an even one.
///
/// A run whose bytes are all printable ASCII is not reported, because the
/// eight-bit reading already explains them and is the simpler account: "Hello
/// world" read two bytes at a time is a row of CJK characters, and a scanner
/// that believed that would report every English sentence twice.
fn wide_runs(buf: &[u8], from: usize, min: usize, enc: Enc, out: &mut Vec<Run>) {
    let big = enc.big();
    for parity in 0..2usize {
        let mut i = from + parity;
        while i + 1 < buf.len() {
            let start = i;
            let mut chars = 0u32;
            let mut j = i;
            while let Some(u) = unit_at(buf, j, big) {
                if is_high_surrogate(u) {
                    match unit_at(buf, j + 2, big) {
                        Some(v) if is_low_surrogate(v) => {
                            chars += 1;
                            j += 4;
                            continue;
                        }
                        // A surrogate with no partner is WTF-16, which is real
                        // and worth keeping. It cannot start a run, so a
                        // stretch of arbitrary bytes that happens to land in
                        // the surrogate block is not a string.
                        _ if chars > 0 => {
                            chars += 1;
                            j += 2;
                            continue;
                        }
                        _ => break,
                    }
                }
                if is_low_surrogate(u) {
                    if chars == 0 {
                        break;
                    }
                    chars += 1;
                    j += 2;
                    continue;
                }
                match char::from_u32(u as u32) {
                    Some(c) if if chars == 0 { starter(c) } else { printable(c) } => {
                        chars += 1;
                        j += 2;
                    }
                    _ => break,
                }
            }
            if j > start {
                let (a, b) = trim(buf, start, j, big);
                // The run, and then the run in pieces. Both, because a run is
                // allowed a stray character and a string that ends in a
                // separator with the next string behind it is a run with a
                // stray in it: three hundred lines of a .NET user string heap
                // read as one run of ten thousand characters. The whole
                // outranks its pieces and takes the bytes wherever it stands;
                // where it cannot say it is a string, the pieces are there to
                // be taken instead. See `page_pieces`.
                //
                // If the run held together, its ends have been trimmed and the
                // pieces come from what is left. If it did not, they come from
                // the raw run: trimming assumes a run is a string with rubbish
                // at its ends, and a run that failed may be rubbish with
                // strings in it. One run of thirteen thousand bytes in
                // `System.dll` was cut back to a point before the first of the
                // strings inside it.
                let (lo, hi) = match consider(buf, a, b, enc, min, out) {
                    true => (a, b),
                    false => (start, j),
                };
                let mut at = lo;
                while let Some((p, q)) = page_pieces(buf, at, hi, big) {
                    if (p, q) != (lo, hi) {
                        consider(buf, p, q, enc, min, out);
                    }
                    at = q;
                }
                i = j;
            } else {
                i += 2;
            }
        }
    }
}

/// Reports `[start, end)` as a wide run if everything about it says string,
/// and says whether it did.
fn consider(buf: &[u8], start: usize, end: usize, enc: Enc, min: usize, out: &mut Vec<Run>) -> bool {
    let big = enc.big();
    if start + 2 > end {
        return false;
    }
    // A piece can begin anywhere its page does, which is not always somewhere
    // a string can begin.
    if !unit_at(buf, start, big)
        .and_then(|u| char::from_u32(u as u32))
        .is_some_and(starter)
        && !unit_at(buf, start, big).is_some_and(is_high_surrogate)
    {
        return false;
    }
    let (chars, units, lone) = measure(buf, start, end, enc);
    // A surrogate with no partner is WTF-16 and is worth keeping, but a run
    // that is half of them is not a name with one bad character in it. It is
    // compressed bytes: a Godot pack is full of runs that read as two unpaired
    // halves and two Hangul syllables, and every other test here passes them.
    if chars as usize >= min
        && lone * 3 < chars
        && wide_enough(buf, start, end, big)
        && one_page(buf, start, end, big)
        && varied(buf, start, end, big)
        && letters(buf, start, end, big)
    {
        let latin = (start..end)
            .step_by(2)
            .all(|k| unit_at(buf, k, big).is_some_and(|u| u < 0x100 || u >= 0xd800));
        let quality = if latin { 3 } else { 2 };
        out.push(Run { start, end, enc, chars, units, lone: lone > 0, quality });
        return true;
    }
    false
}

/// The next stretch of `[at, end)` whose characters all come from one page.
///
/// A maximal run reaches over whatever the compiler left either side of the
/// string, and one character of the string's own page in among that rubbish
/// pins the trim in place: in a .NET `#US` heap the byte pairs before
/// "providerOptions" read as three Han characters with a 'z' between each of
/// them, so the cut stopped at the first 'z' and a run of four pages went to
/// the page test and lost. Cutting instead at every change of page offers the
/// string on its own, and offers the rubbish separately, where it fails on its
/// own account.
///
/// This is only reached when the run as a whole has already failed, so the
/// allowance for a stray character is untouched: a run that keeps its page is
/// never cut up.
fn page_pieces(buf: &[u8], at: usize, end: usize, big: bool) -> Option<(usize, usize)> {
    if at + 2 > end {
        return None;
    }
    // A surrogate has no page of its own worth the name, so it stays with
    // whatever it was found next to.
    let page_of = |i: usize| {
        let u = unit_at(buf, i, big).unwrap_or(0);
        (!(0xd800..0xe000).contains(&u)).then_some((u >> 8) as u8)
    };
    let page = (at..end).step_by(2).find_map(page_of);
    let mut b = at + 2;
    while b + 2 <= end && page_of(b).is_none_or(|p| Some(p) == page) {
        b += 2;
    }
    Some((at, b))
}

/// Characters, code units and how many surrogates went unpaired, over a range
/// already known to be text.
fn measure(buf: &[u8], start: usize, end: usize, enc: Enc) -> (u32, u32, u32) {
    if !enc.wide() {
        let mut chars = 0u32;
        let mut i = start;
        while i < end {
            match utf8_char(buf, i) {
                Some((_, n)) if i + n <= end => {
                    chars += 1;
                    i += n;
                }
                _ => {
                    chars += 1;
                    i += 1;
                }
            }
        }
        return (chars, (end - start) as u32, 0);
    }
    let big = enc.big();
    let mut chars = 0u32;
    let mut lone = 0u32;
    let mut i = start;
    while i + 2 <= end {
        let u = unit_at(buf, i, big).unwrap_or(0);
        if is_high_surrogate(u) && i + 4 <= end && unit_at(buf, i + 2, big).is_some_and(is_low_surrogate) {
            chars += 1;
            i += 4;
            continue;
        }
        if is_high_surrogate(u) || is_low_surrogate(u) {
            lone += 1;
        }
        chars += 1;
        i += 2;
    }
    (chars, ((end - start) / 2) as u32, lone)
}

/// The text of a range, with an unpaired surrogate standing as U+FFFD. It is
/// the one character that cannot come out of a valid string, so it says
/// "something was here that this is not" without pretending to be it.
fn decode(bytes: &[u8], enc: Enc) -> String {
    if !enc.wide() {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let big = enc.big();
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|p| if big { u16::from_be_bytes([p[0], p[1]]) } else { u16::from_le_bytes([p[0], p[1]]) })
        .collect();
    char::decode_utf16(units).map(|r| r.unwrap_or('\u{fffd}')).collect()
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

// --- the number in front ---------------------------------------------------

/// Every reading of a prefix of a given kind ending just before `at`: where it
/// starts and what it says.
///
/// One reading for a fixed-width number. A LEB128 has as many as it has
/// plausible lengths, because where one begins is not written down anywhere:
/// its last byte has the high bit clear and every byte before it has the high
/// bit set, and a byte with the high bit set in front of it may be part of the
/// same number or may be something else entirely. Rather than pick, each
/// length is offered and the one that comes to the run's length is the one
/// reported.
fn prefix_candidates(buf: &[u8], at: usize, kind: PrefixKind) -> Vec<(usize, u64)> {
    if let Some(w) = kind.width() {
        let Some(start) = at.checked_sub(w) else { return Vec::new() };
        let Some(b) = buf.get(start..at) else { return Vec::new() };
        let v = match kind {
            PrefixKind::U8 => b[0] as u64,
            PrefixKind::U16Le => u16::from_le_bytes([b[0], b[1]]) as u64,
            PrefixKind::U16Be => u16::from_be_bytes([b[0], b[1]]) as u64,
            PrefixKind::U32Le => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as u64,
            PrefixKind::U32Be => u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as u64,
            PrefixKind::U64Le => u64::from_le_bytes(b[..8].try_into().unwrap()),
            PrefixKind::U64Be => u64::from_be_bytes(b[..8].try_into().unwrap()),
            PrefixKind::Leb128 => unreachable!(),
        };
        return vec![(start, v)];
    }
    // Only a number of two bytes or more is offered here: a one-byte LEB128 is
    // the same byte as a u8, and saying so twice tells the reader nothing.
    let Some(last) = at.checked_sub(1) else { return Vec::new() };
    if buf[last] & 0x80 != 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut start = last;
    while start > 0 && buf[start - 1] & 0x80 != 0 && last + 1 - start < 10 {
        start -= 1;
        let mut v = 0u64;
        for (i, b) in buf[start..=last].iter().enumerate() {
            v |= ((b & 0x7f) as u64) << (7 * i);
        }
        out.push((start, v));
    }
    out
}

/// The one reading of a prefix, for the places that only need to know whether
/// there is one at all. The shortest LEB128, which is the likeliest.
fn read_prefix(buf: &[u8], at: usize, kind: PrefixKind) -> Option<(usize, u64)> {
    prefix_candidates(buf, at, kind).into_iter().next()
}

/// Every reading of the bytes in front of a run that comes to its length.
///
/// A run's first byte is not text, so a number in the byte before it is not
/// text either, and one that comes to the run's length is worth putting in
/// front of the reader. Which of the readings was meant is not decided here:
/// `00 00 00 05` is a 32-bit five, a 16-bit five and an eight-bit five, all
/// three true, and the bytes are carried so the reader can see that.
fn prefix_readings(
    buf: &[u8],
    base: u64,
    start: usize,
    len: u64,
    units: u32,
    chars: u32,
    enc: Enc,
    term: Option<Term>,
) -> Vec<Prefix> {
    let term_bytes = term.map_or(0, |t| t.bytes());
    let term_units = if enc.wide() { term_bytes / 2 } else { term_bytes };
    // Bytes, code units and characters are the same number for ASCII, so the
    // reading that says which one it counted only earns its place when they
    // differ.
    let mut wants: Vec<(u64, Counts, bool)> = vec![(len, Counts::Bytes, false)];
    if term_bytes > 0 {
        wants.push((len + term_bytes, Counts::Bytes, true));
    }
    if units as u64 != len {
        wants.push((units as u64, Counts::Units, false));
        if term_units > 0 {
            wants.push((units as u64 + term_units, Counts::Units, true));
        }
    }
    if chars as u64 != len && chars != units {
        wants.push((chars as u64, Counts::Chars, false));
        if term_units > 0 {
            wants.push((chars as u64 + term_units, Counts::Chars, true));
        }
    }
    let mut out = Vec::new();
    'kinds: for kind in PREFIX_ORDER {
        for (at, value) in prefix_candidates(buf, start, kind) {
            let Some(&(_, counts, with_terminator)) = wants.iter().find(|(want, _, _)| *want == value) else {
                continue;
            };
            out.push(Prefix {
                kind,
                at: base + at as u64,
                raw: buf[at..start].to_vec(),
                value,
                counts,
                with_terminator,
                weak: false,
            });
            if out.len() >= MAX_PREFIX_READINGS {
                break 'kinds;
            }
            break;
        }
    }
    out
}

/// How many bytes `n` of something takes, starting at `at`.
fn span_of(buf: &[u8], at: usize, limit: usize, enc: Enc, n: u64, counts: Counts) -> Option<usize> {
    match counts {
        Counts::Bytes => usize::try_from(n).ok(),
        Counts::Units => usize::try_from(n.checked_mul(enc.unit())?).ok(),
        Counts::Chars => {
            let mut i = at;
            for _ in 0..n {
                if i >= limit {
                    return None;
                }
                i += if enc.wide() {
                    let u = unit_at(buf, i, enc.big())?;
                    if is_high_surrogate(u) && unit_at(buf, i + 2, enc.big()).is_some_and(is_low_surrogate) {
                        4
                    } else {
                        2
                    }
                } else {
                    utf8_char(buf, i)?.1
                };
            }
            Some(i - at)
        }
    }
}

/// How many bytes a prefix of this kind takes, starting at `pos`.
fn prefix_width(buf: &[u8], pos: usize, limit: usize, kind: PrefixKind) -> Option<usize> {
    if let Some(w) = kind.width() {
        return Some(w);
    }
    let mut i = pos;
    while i < limit && buf[i] & 0x80 != 0 {
        i += 1;
    }
    (i < limit).then(|| i + 1 - pos)
}

/// Where the text after the chain's first number could start.
///
/// Two places, and both are worth trying. The number sits in front of the run
/// when it is a control byte, which is what a short string's length is. It
/// sits at the run's first byte when it is large enough to be printable,
/// which is what made the whole table one run to begin with.
fn first_prefix(buf: &[u8], run: Run, kind: PrefixKind) -> Vec<usize> {
    let mut out = Vec::new();
    if read_prefix(buf, run.start, kind).is_some() {
        out.push(run.start);
    }
    let inside = run.start + prefix_width(buf, run.start, run.end, kind).unwrap_or(0);
    if inside > run.start && inside < run.end {
        out.push(inside);
    }
    out
}

/// Whether a run is a chain of counted strings laid end to end, and where each
/// of them starts.
///
/// Two counted strings next to each other are usually two runs, because the
/// number between them is a control byte and a control byte is not text. They
/// are one run when the numbers are large enough to be printable, which is
/// what a table of strings of a few dozen bytes each looks like. Reading that
/// as one string is wrong, so a chain that tiles the run exactly splits it.
///
/// The chain must account for every byte of the run and hold at least two
/// strings, each of them long enough to have been reported on its own. A run
/// that only nearly tiles is left as the one string it looked like: half an
/// explanation is worse than none.
fn chain(buf: &[u8], base: u64, run: Run, opts: Opts) -> Vec<(usize, usize, Prefix)> {
    let min = opts.min();
    let unit = run.enc.unit() as usize;
    let counts: &[Counts] =
        if run.enc.wide() { &[Counts::Units, Counts::Chars, Counts::Bytes] } else { &[Counts::Bytes, Counts::Chars] };
    for kind in PREFIX_ORDER {
        for &c in counts {
        for first in first_prefix(buf, run, kind) {
            let mut pieces: Vec<(usize, usize, Prefix)> = Vec::new();
            let mut at = first;
            let ok = loop {
                let Some((pat, value)) = read_prefix(buf, at, kind) else { break false };
                let Some(span) = span_of(buf, at, run.end, run.enc, value, c) else { break false };
                if span == 0 || span % unit != 0 || at + span > run.end {
                    break false;
                }
                if at + span == run.end && pieces.is_empty() {
                    // One string with a number in front of it is not a chain.
                    // It is already reported as the one string it is.
                    break false;
                }
                let (chars, _, _) = measure(buf, at, at + span, run.enc);
                if (chars as usize) < min {
                    break false;
                }
                pieces.push((at, at + span, Prefix {
                    kind,
                    at: base + pat as u64,
                    raw: buf[pat..at].to_vec(),
                    value,
                    counts: c,
                    with_terminator: false,
                    // A chain is several numbers agreeing about where the
                    // strings after them end, which is a fact about the file
                    // whatever width they are written at.
                    weak: false,
                }));
                let over = at + span;
                if over == run.end {
                    break true;
                }
                if pieces.len() > 4096 {
                    break false;
                }
                // The next number sits where this string stopped, and the
                // string after it starts once the number is read.
                let Some(w) = prefix_width(buf, over, run.end, kind) else { break false };
                if over + w >= run.end {
                    break false;
                }
                at = over + w;
            };
            if ok && pieces.len() > 1 {
                return pieces;
            }
        }
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::MemSource;

    fn all(bytes: Vec<u8>) -> Vec<Hit> {
        scan(&MemSource(bytes), 0, 1000, Opts::default()).hits
    }

    fn utf16le(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    fn utf16be(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(u16::to_be_bytes).collect()
    }

    #[test]
    fn finds_a_c_string_and_its_terminator() {
        let mut b = vec![0x01, 0x02, 0x03];
        b.extend(b"version 1.4\0");
        b.extend([0xff, 0xff]);
        let hits = all(b);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].text, "version 1.4");
        assert_eq!(hits[0].at, 3);
        assert_eq!(hits[0].enc, Enc::Ascii);
        assert_eq!(hits[0].term, Some(Term::Nul));
    }

    #[test]
    fn a_run_shorter_than_the_minimum_is_not_a_string() {
        let hits = all(b"\x00abc\x00abcd\x00".to_vec());
        assert_eq!(hits.iter().map(|h| h.text.as_str()).collect::<Vec<_>>(), ["abcd"]);
    }

    #[test]
    fn a_newline_ends_a_string() {
        let hits = all(b"\x00first line\nsecond line\x00".to_vec());
        assert_eq!(hits.iter().map(|h| h.text.as_str()).collect::<Vec<_>>(), ["first line", "second line"]);
    }

    /// A .NET user string heap: a length in bytes, the characters, then a flag
    /// byte. Nothing terminates a string, so the flag byte and the next
    /// string's length are what the shifted big-endian reading takes for a
    /// terminator, and it is the reading with all the evidence.
    fn us_heap(words: &[&str]) -> Vec<u8> {
        let mut b = vec![0x00, 0x00];
        for w in words {
            let chars = utf16le(w);
            b.push(chars.len() as u8 + 1);
            b.extend(chars);
            b.push(0x00);
        }
        b.extend([0x00, 0x00]);
        b
    }

    #[test]
    fn packed_utf16_is_read_the_way_round_it_was_written() {
        let hits = all(us_heap(&["providerOptions", "Module", "Nothing", "vbc.exe"]));
        assert_eq!(hits.iter().map(|h| h.enc).collect::<Vec<_>>(), [Enc::Utf16Le; 4]);
        assert_eq!(
            hits.iter().map(|h| h.text.as_str()).collect::<Vec<_>>(),
            ["providerOptions", "Module", "Nothing", "vbc.exe"]
        );
    }

    #[test]
    fn the_shifted_reading_of_big_endian_text_does_not_take_its_place() {
        // The same heap the other way round, where the shifted reading is the
        // little-endian one. Nothing about the bytes says which way round they
        // were meant except the terminators, so this is what asks whether the
        // rule that settles it has a side.
        let mut b = vec![0x00, 0x00];
        for w in ["providerOptions", "Module", "Nothing"] {
            b.extend(utf16be(w));
            b.extend([0x00, 0x00]);
        }
        let hits = all(b);
        assert_eq!(hits.iter().map(|h| h.enc).collect::<Vec<_>>(), [Enc::Utf16Be; 3]);
        assert_eq!(hits[0].text, "providerOptions");
    }

    #[test]
    fn strings_run_together_by_a_separator_are_still_strings() {
        // A run is allowed a stray character, and the byte ending one line
        // read together with the byte counting the next is a stray character.
        // A heap of them is one run of every line at once, and the last line's
        // terminator speaks for the whole of it.
        let hits = all(us_heap(&["Do While ", "NotInheritable ", "Protected Friend ", "Structure "]));
        assert_eq!(
            hits.iter().map(|h| h.text.as_str()).collect::<Vec<_>>(),
            ["Do While ", "NotInheritable ", "Protected Friend ", "Structure "]
        );
    }

    #[test]
    fn a_string_between_two_stretches_of_rubbish_is_found() {
        // Read two bytes at a time, eight-bit text either side of a wide
        // string is a run of characters from all over Unicode, and it reaches
        // over the string. Trimming the ends of that run cannot reach the
        // string, since the run's own page sits in the rubbish as well.
        let mut b = vec![0x00, 0x00];
        b.extend(b"cmdidTileHorz csz psz ");
        b.push(0x1f);
        b.extend(utf16le("providerOptions"));
        b.extend([0x00, 0x05, 0x76, 0x00, 0x62, 0x00, 0x00, 0x00]);
        let hits = all(b);
        assert!(
            hits.iter().any(|h| h.enc == Enc::Utf16Le && h.text == "providerOptions"),
            "{:?}",
            hits.iter().map(|h| (h.enc, h.text.as_str())).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_length_eight_bytes_wide_says_so() {
        // What GGUF counts every key and every string value by.
        let mut b = vec![0x00, 0x00];
        b.extend(20u64.to_le_bytes());
        b.extend(b"general.architecture");
        b.extend([0x00, 0x00]);
        let readings = &all(b)[0].prefix;
        assert_eq!(readings[0].kind, PrefixKind::U64Le);
        assert_eq!(readings[0].value, 20);
        assert_eq!(readings[0].counts, Counts::Bytes);
    }

    #[test]
    fn a_pascal_string_says_which_byte_counted_it() {
        let mut b = vec![0xff, 0x0b];
        b.extend(b"version 1.4");
        b.push(0xff);
        let hits = all(b);
        assert_eq!(hits.len(), 1);
        let p = &hits[0].prefix;
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].kind, PrefixKind::U8);
        assert_eq!(p[0].value, 11);
        assert_eq!(p[0].counts, Counts::Bytes);
        assert_eq!(p[0].at, 1);
        assert!(!p[0].with_terminator);
    }

    #[test]
    fn a_length_that_counts_the_terminator_says_so() {
        let mut b = vec![0xff, 0x0c];
        b.extend(b"version 1.4\0");
        let hits = all(b);
        let p = &hits[0].prefix;
        assert_eq!(p[0].kind, PrefixKind::U8);
        assert_eq!(p[0].value, 12);
        assert!(p[0].with_terminator);
    }

    #[test]
    fn a_thirty_two_bit_length_is_read_before_the_bytes_inside_it() {
        let mut b = vec![0xff, 0x00, 0x00, 0x00, 0x0b];
        b.extend(b"version 1.4");
        b.push(0xff);
        let readings = &all(b)[0].prefix;
        assert_eq!(readings[0].kind, PrefixKind::U32Be);
        assert_eq!(readings[0].value, 11);
        // The same bytes are a 16-bit and an 8-bit eleven as well, and all
        // three readings are true of them.
        assert!(readings.iter().any(|p| p.kind == PrefixKind::U16Be));
        assert!(readings.iter().any(|p| p.kind == PrefixKind::U8));
    }

    #[test]
    fn a_little_endian_length_reads_only_one_way() {
        let mut b = vec![0xff, 0x0b, 0x00, 0x00, 0x00];
        b.extend(b"version 1.4");
        b.push(0xff);
        let readings = &all(b)[0].prefix;
        assert_eq!(readings.len(), 1);
        assert_eq!(readings[0].kind, PrefixKind::U32Le);
    }

    #[test]
    fn a_lone_byte_length_is_shown_for_what_it_is() {
        // The byte in front of any run is one that could not be part of it,
        // and the values that could not be are the small ones, which is what a
        // short length looks like. So a match here is worth showing and is not
        // worth believing on its own.
        let mut b = vec![0xff, 0x0b];
        b.extend(b"version 1.4");
        b.push(0xff);
        let hits = all(b);
        assert!(hits[0].prefix[0].weak);
    }

    #[test]
    fn a_table_of_counted_strings_vouches_for_every_number_in_it() {
        // The same byte-wide number, once a neighbour is counted the same way.
        // Two of them landing exactly where the string after them starts is a
        // table, and a table is a fact about the file.
        let mut b = vec![0xff];
        for word in ["version 1.4", "release", "beta 3"] {
            b.push(word.len() as u8);
            b.extend(word.as_bytes());
        }
        b.push(0xff);
        let hits = all(b);
        assert_eq!(hits.len(), 3);
        for h in &hits {
            assert_eq!(h.prefix[0].kind, PrefixKind::U8);
            assert!(!h.prefix[0].weak, "{:?} should be vouched for by its neighbours", h.text);
        }
    }

    #[test]
    fn a_number_wider_than_a_character_is_not_the_run_boundary() {
        let mut b = vec![0xff, 0x0b, 0x00];
        b.extend(b"version 1.4");
        b.push(0xff);
        let readings = &all(b)[0].prefix;
        let wide = readings.iter().find(|p| p.kind == PrefixKind::U16Le).expect("a u16 reading");
        assert!(!wide.weak);
    }

    #[test]
    fn a_dotnet_seven_bit_length_is_found() {
        // 200 characters, written as .NET's BinaryWriter writes a length.
        let text = "d".repeat(200);
        let mut b = vec![0x00, 0xc8, 0x01];
        b.extend(text.as_bytes());
        b.push(0x00);
        let readings = &all(b)[0].prefix;
        assert!(readings.iter().any(|p| p.kind == PrefixKind::Leb128 && p.value == 200 && p.raw == [0xc8, 0x01]));
    }

    #[test]
    fn utf16_le_is_found_at_an_odd_offset() {
        let mut b = vec![0x01, 0x02, 0x03];
        b.extend(utf16le("Open file"));
        b.extend([0x00, 0x00]);
        let hits = all(b);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].text, "Open file");
        assert_eq!(hits[0].enc, Enc::Utf16Le);
        assert_eq!(hits[0].at, 3);
        assert_eq!(hits[0].term, Some(Term::NulNul));
    }

    #[test]
    fn utf16_be_is_found() {
        let mut b = vec![0x00, 0x00];
        b.extend(utf16be("Cannot open"));
        b.extend([0x00, 0x00]);
        let hits = all(b);
        assert_eq!(hits.iter().map(|h| h.enc).collect::<Vec<_>>(), [Enc::Utf16Be]);
        assert_eq!(hits[0].text, "Cannot open");
    }

    #[test]
    fn a_utf16_length_counting_code_units_says_so() {
        let mut b = vec![0x00, 0x00, 0x0d, 0x00];
        b.extend(utf16le("Cannot open a"));
        b.extend([0x00, 0x00]);
        let readings = &all(b)[0].prefix;
        assert!(readings.iter().any(|p| p.kind == PrefixKind::U16Le && p.counts == Counts::Units && p.value == 13));
    }

    #[test]
    fn an_ascii_run_is_not_reported_a_second_time_as_wide_characters() {
        let hits = all(b"\x00the quick brown fox jumps\x00".to_vec());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].enc, Enc::Ascii);
    }

    #[test]
    fn a_lone_surrogate_does_not_end_a_wtf16_string() {
        let mut units: Vec<u16> = "photo".encode_utf16().collect();
        units.push(0xd83d);
        units.extend("done".encode_utf16());
        let mut b = vec![0x00, 0x00];
        b.extend(units.iter().flat_map(|u| u.to_le_bytes()));
        b.extend([0x00, 0x00]);
        let hits = all(b);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].lone_surrogates);
        assert_eq!(hits[0].text, "photo\u{fffd}done");
        assert_eq!(hits[0].chars, 10);
    }

    #[test]
    fn a_surrogate_pair_is_one_character_and_two_code_units() {
        let mut b = vec![0x00, 0x00];
        b.extend(utf16le("hi \u{1f600} there"));
        b.extend([0x00, 0x00]);
        let hits = all(b);
        assert!(!hits[0].lone_surrogates);
        assert_eq!(hits[0].text, "hi \u{1f600} there");
        assert_eq!(hits[0].chars, 10);
        assert_eq!(hits[0].units, 11);
    }

    #[test]
    fn utf8_is_told_from_ascii() {
        let mut b = vec![0x00];
        b.extend("Gr\u{fc}\u{df}e aus K\u{f6}ln".as_bytes());
        b.push(0x00);
        let hits = all(b);
        assert_eq!(hits[0].enc, Enc::Utf8);
        assert_eq!(hits[0].text, "Gr\u{fc}\u{df}e aus K\u{f6}ln");
        assert_eq!(hits[0].chars, 14);
        assert_eq!(hits[0].len, 17);
    }

    #[test]
    fn a_chain_of_counted_strings_is_split_into_the_strings_it_is() {
        // Two strings whose length bytes are themselves printable, so the
        // whole thing is one run of text and reading it as one string would
        // be wrong.
        let a = "a".repeat(40);
        let c = "c".repeat(50);
        let mut b = vec![0x00, 40u8];
        b.extend(a.as_bytes());
        b.push(50u8);
        b.extend(c.as_bytes());
        b.push(0x00);
        let hits = all(b);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].text, a);
        assert_eq!(hits[1].text, c);
        assert_eq!(hits[0].prefix[0].kind, PrefixKind::U8);
        assert_eq!(hits[0].at, 2);
        assert_eq!(hits[0].prefix[0].at, 1);
        assert_eq!(hits[1].at, 43);
        assert_eq!(hits[1].prefix[0].at, 42);
        assert_eq!(hits[1].prefix[0].value, 50);
    }

    #[test]
    fn a_run_that_only_nearly_tiles_is_left_as_one_string() {
        let a = "a".repeat(40);
        let c = "c".repeat(50);
        let mut b = vec![0x00, 40u8];
        b.extend(a.as_bytes());
        b.push(49u8); // one short: the chain does not reach the end
        b.extend(c.as_bytes());
        b.push(0x00);
        let hits = all(b);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].len, 92);
    }

    #[test]
    fn a_run_past_the_cap_is_cut_and_says_so() {
        let mut b = vec![0x00];
        b.extend(vec![b'z'; (MAX_BYTES as usize) + 100]);
        b.push(0x00);
        let hits = all(b);
        assert_eq!(hits.len(), 2);
        assert!(hits[0].cut);
        assert_eq!(hits[0].len, MAX_BYTES);
        assert_eq!(hits[1].at, 1 + MAX_BYTES);
        assert_eq!(hits[1].len, 100);
        assert!(!hits[1].cut);
    }

    #[test]
    fn a_run_longer_than_one_window_says_so_at_every_join() {
        // Longer than the window and than the lookahead past it, so the read
        // stops in the middle of it as well as the length limit does.
        let mut b = vec![0u8; (WINDOW - 10) as usize];
        b.extend(vec![b'z'; 20_000]);
        b.push(0);
        let hits = all(b);
        assert!(hits.len() > 4);
        for pair in hits.windows(2) {
            let (a, next) = (&pair[0], &pair[1]);
            assert!(a.cut, "a piece before another one must say the text carries on");
            assert_eq!(a.at + a.len, next.at, "the pieces have to be one run with no gap");
        }
        let last = hits.last().expect("a run this long is at least one string");
        assert!(!last.cut);
        assert_eq!(last.term, Some(Term::Nul));
        let total: u64 = hits.iter().map(|h| h.len).sum();
        assert_eq!(total, 20_000);
    }

    #[test]
    fn a_string_across_a_window_join_is_found_once_and_whole() {
        let filler = (WINDOW - 20) as usize;
        let mut b = vec![0u8; filler];
        b.extend(b"across the join here");
        b.extend(vec![0u8; 100]);
        let hits = all(b);
        let found: Vec<&str> = hits.iter().map(|h| h.text.as_str()).collect();
        assert_eq!(found, ["across the join here"]);
        assert_eq!(hits[0].at, filler as u64);
    }

    #[test]
    fn scanning_carries_on_from_where_it_stopped() {
        let mut b = vec![0u8; 4];
        b.extend(b"first\0");
        b.extend(b"second\0");
        b.extend(b"third\0");
        let src = MemSource(b);
        let one = scan(&src, 0, 2, Opts::default());
        assert_eq!(one.hits.len(), 2);
        let two = scan(&src, one.next, 10, Opts::default());
        assert_eq!(two.hits.iter().map(|h| h.text.as_str()).collect::<Vec<_>>(), ["third"]);
        assert_eq!(two.next, src.len_bytes());
    }

    #[test]
    fn turning_an_encoding_off_leaves_the_others() {
        let mut b = vec![0x00, 0x00];
        b.extend(utf16le("Open file"));
        b.extend([0x00, 0x00]);
        b.extend(b"plain ascii\0");
        let src = MemSource(b);
        let both = scan(&src, 0, 100, Opts::default()).hits;
        assert_eq!(both.iter().map(|h| h.text.as_str()).collect::<Vec<_>>(), ["Open file", "plain ascii"]);
        let opts = Opts { ascii: false, ..Opts::default() };
        let hits = scan(&src, 0, 100, opts).hits;
        assert_eq!(hits.iter().map(|h| h.text.as_str()).collect::<Vec<_>>(), ["Open file"]);
    }

    #[test]
    fn one_wide_character_repeated_is_padding_and_not_a_string() {
        // x86 pads with 0x90, which two bytes at a time is one character over
        // and over. Coherent, printable, and not a string.
        let mut b = vec![0x00, 0x00];
        b.extend(vec![0x90u8; 40]);
        b.extend([0x00, 0x00]);
        assert!(all(b).is_empty());
    }

    #[test]
    fn a_wide_string_keeps_itself_and_not_what_sits_in_front_of_it() {
        // The bytes around a string in a binary are whatever the compiler put
        // there, and two of them are usually some character. The run reaches
        // over them and has to be cut back, or the characters they add make an
        // otherwise coherent run look like six scripts at once.
        let mut b = vec![0xa0, 0xee, 0xe8, 0xb9];
        b.extend(utf16le("Open file"));
        b.extend([0x00, 0x00, 0x5c, 0x7c]);
        let hits = all(b);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].text, "Open file");
        assert_eq!(hits[0].at, 4);
        assert_eq!(hits[0].term, Some(Term::NulNul));
    }

    #[test]
    fn a_wide_string_keeps_itself_and_not_what_sits_after_it() {
        // The same at the other end, where the number in front is what says
        // the run is a string at all.
        // Twenty-six bytes, counted by a number four bytes wide: two bytes
        // more than the run's own boundary needed, which is what makes it
        // worth believing. See `evidence`.
        let mut b = vec![0xff, 0x1a, 0x00, 0x00, 0x00];
        b.extend(utf16le("Cannot open a"));
        b.extend([0x5c, 0x7c, 0x29, 0x99, 0x00]);
        let hits = all(b);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].text, "Cannot open a");
        assert_eq!(hits[0].prefix[0].kind, PrefixKind::U32Le);
        assert_eq!(hits[0].prefix[0].value, 26);
    }

    #[test]
    fn a_wide_run_with_nothing_to_say_for_itself_is_not_reported() {
        // Sixteen-bit numbers that happen to be printable characters. Quiet
        // audio and a table of small integers both look like this, and neither
        // is a string: no zero after it, no number in front that comes to its
        // length.
        let mut b = vec![0xa0, 0xee, 0xe8, 0xb9];
        b.extend(utf16le("Open file"));
        b.extend([0x5c, 0x7c, 0x29, 0x99]);
        assert!(all(b).is_empty());
    }

    #[test]
    fn a_wide_character_does_not_carry_compiled_code_over_the_minimum() {
        // c6 8b is a valid UTF-8 character and welds ")" onto "D$P". Neither
        // half is four characters long and neither is a string.
        let b = b"\x00)\xc6\x8bD$P\x00".to_vec();
        assert!(all(b).is_empty());
    }

    #[test]
    fn a_run_of_wide_characters_from_six_scripts_is_not_a_string() {
        // Every one of these is a printable character and no two are from the
        // same part of Unicode, which is what a stretch of compiled code read
        // two bytes at a time looks like.
        let units: [u16; 6] = [0x0045, 0x6400, 0x0a86, 0xa000, 0xfb2c, 0x0069];
        let mut b = vec![0x00, 0x00];
        b.extend(units.iter().flat_map(|u| u.to_le_bytes()));
        b.extend([0x00, 0x00]);
        assert!(all(b).is_empty());
    }

    #[test]
    fn an_unreadable_chunk_is_asked_for_rather_than_guessed_at() {
        use crate::source::Missing;
        struct Absent;
        impl Source for Absent {
            fn len_bytes(&self) -> u64 {
                1 << 20
            }
            fn read_bytes(&self, _offset: u64, out: &mut [u8]) -> Vec<Missing> {
                out.fill(0);
                vec![Missing { chunk: 0 }]
            }
        }
        let s = scan(&Absent, 0, 10, Opts::default());
        assert!(s.hits.is_empty());
        assert_eq!(s.next, 0);
        assert_eq!(s.missing.len(), 1);
    }
}
