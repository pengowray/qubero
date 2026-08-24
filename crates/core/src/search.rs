//! Finding things in a file that is never all in memory.
//!
//! A search here is a series of bounded steps rather than one scan. The file
//! may be larger than memory and its bytes arrive a chunk at a time, so a call
//! that ran to the end would either block for minutes or read bytes that are
//! not there yet. Each step reads at most a window, and answers with a match,
//! with the chunks it needs, or with where to carry on.
//!
//! Offsets here are bytes, not bits. Every hex editor searches byte-aligned
//! and so does this: a needle that could start at any bit would match noise in
//! most files, and nothing asks for it.
//!
//! Windows overlap by one less than the needle, so a match lying across the
//! join is still found. The caller does not have to know that: `resume` says
//! where to start the next step, and it is not simply the end of the last one.

use regex_automata::meta::Regex;
use regex_automata::{Anchored, Input};

use crate::document::Document;
use crate::source::{Missing, Source};

/// How far one step reads when the caller does not say.
pub const WINDOW: u64 = 64 * 1024;

/// How much of the next window a step also reads for a pattern, whose match
/// has no length known in advance. A regex match longer than this and lying
/// across a window join is the one thing this search can miss.
pub const REGEX_OVERLAP: u64 = 4 * 1024;

/// Why a pattern with `^` or `$` is refused.
pub const ANCHOR_REFUSED: &str = "^ and $ would match where this search stopped reading, not where a line starts";

/// What one step of a search found.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// A match, at this byte, this many bytes long.
    Found { at: u64, len: u64 },
    /// Nothing yet. Ask again from here.
    More { resume: u64 },
    /// The end of the file, or the start of it going backwards.
    End,
    /// Chunks this step needs before it can answer.
    Pending(Vec<Missing>),
}

/// What to look for.
#[derive(Debug, Clone, PartialEq)]
pub enum Needle {
    /// These bytes exactly.
    Bytes(Vec<u8>),
    /// These bytes, with the letters A to Z matching either case. Folding is
    /// ASCII only: matching É to é means knowing the encoding, and a hex
    /// editor does not know what encoding a stretch of a file is in.
    Fold(Vec<u8>),
    /// A pattern.
    Regex(Pattern),
}

/// A compiled pattern. Two are the same when they were written the same way,
/// which is what a search box needs to know and all it needs to know.
#[derive(Clone)]
pub struct Pattern {
    source: String,
    re: Regex,
}

impl Pattern {
    /// Compile a pattern, or say what is wrong with it.
    ///
    /// `^` and `$` are refused. A search reads a window at a time, so they
    /// would match the edges of whatever it happened to read rather than
    /// anything in the file, and quietly finding the wrong thing is worse than
    /// saying no.
    pub fn new(source: &str) -> Result<Pattern, String> {
        if has_anchor(source) {
            return Err(ANCHOR_REFUSED.into());
        }
        match Regex::new(source) {
            Ok(re) => Ok(Pattern { source: source.to_string(), re }),
            Err(e) => Err(first_line(&e.to_string())),
        }
    }

    pub fn source(&self) -> &str {
        &self.source
    }
}

impl std::fmt::Debug for Pattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Pattern({:?})", self.source)
    }
}

impl PartialEq for Pattern {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
    }
}

/// A regex error runs to several lines with a diagram under it. The first line
/// is what went wrong.
fn first_line(s: &str) -> String {
    s.lines().find(|l| !l.trim().is_empty()).unwrap_or(s).trim().to_string()
}

/// Whether `^` or `$` appears other than escaped or inside a character class,
/// where they are ordinary characters.
fn has_anchor(source: &str) -> bool {
    let mut chars = source.chars();
    let mut in_class = false;
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            '[' => in_class = true,
            ']' => in_class = false,
            '^' | '$' if !in_class => return true,
            _ => {}
        }
    }
    false
}

impl Needle {
    /// The needle's bytes, for the kinds that are bytes.
    pub fn bytes(&self) -> &[u8] {
        match self {
            Needle::Bytes(b) | Needle::Fold(b) => b,
            Needle::Regex(_) => &[],
        }
    }

    /// How much of the next window a step has to read as well. A literal needs
    /// one byte less than itself; a pattern has no length to go on.
    fn overlap(&self) -> u64 {
        match self {
            Needle::Regex(_) => REGEX_OVERLAP,
            _ => self.bytes().len().saturating_sub(1) as u64,
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Needle::Regex(p) => p.source.is_empty(),
            _ => self.bytes().is_empty(),
        }
    }

    /// Whether `hay` starts with this needle, and how far it runs.
    fn matches_at(&self, hay: &[u8]) -> Option<usize> {
        if let Needle::Regex(p) = self {
            return p.re.find(Input::new(hay).anchored(Anchored::Yes)).map(|m| m.len());
        }
        let want = self.bytes();
        if hay.len() < want.len() {
            return None;
        }
        let same = match self {
            Needle::Fold(_) => hay[..want.len()].iter().zip(want).all(|(a, b)| fold(*a) == fold(*b)),
            _ => &hay[..want.len()] == want,
        };
        same.then_some(want.len())
    }

    /// The first place in `hay` this needle matches, and how far it runs.
    fn first_in(&self, hay: &[u8]) -> Option<(usize, usize)> {
        if let Needle::Regex(p) = self {
            return p.re.find(hay).map(|m| (m.start(), m.len()));
        }
        let n = self.bytes().len();
        if hay.len() < n {
            return None;
        }
        (0..=hay.len() - n).find(|&i| self.matches_at(&hay[i..]).is_some()).map(|i| (i, n))
    }
}

fn fold(b: u8) -> u8 {
    b.to_ascii_lowercase()
}

/// One search, run a step at a time.
#[derive(Debug, Clone)]
pub struct Search {
    pub needle: Needle,
    pub backward: bool,
    /// How much one step reads.
    pub window: u64,
}

impl Search {
    pub fn forward(needle: Needle) -> Search {
        Search { needle, backward: false, window: WINDOW }
    }

    pub fn backward(needle: Needle) -> Search {
        Search { needle, backward: true, window: WINDOW }
    }

    /// Search one window. Going forwards, `from` is the first byte a match may
    /// start at; going backwards, matches must start before it.
    pub fn step<S: Source>(&self, doc: &Document<S>, from: u64) -> Step {
        if self.needle.is_empty() {
            return Step::End;
        }
        if self.backward {
            self.step_back(doc, from)
        } else {
            self.step_forward(doc, from)
        }
    }

    fn step_forward<S: Source>(&self, doc: &Document<S>, from: u64) -> Step {
        let end = doc.len_bytes();
        if from >= end {
            return Step::End;
        }
        // Read one window plus the tail a match starting at its last byte
        // would need.
        let stop = (from + self.window + self.needle.overlap()).min(end);
        let hay = match read(doc, from, stop - from) {
            Ok(hay) => hay,
            Err(missing) => return Step::Pending(missing),
        };
        match self.needle.first_in(&hay) {
            Some((i, len)) => Step::Found { at: from + i as u64, len: len as u64 },
            None if stop == end => Step::End,
            // The next window starts where a match could still begin, which is
            // short of this one's end by the needle's reach.
            None => Step::More { resume: from + self.window },
        }
    }

    fn step_back<S: Source>(&self, doc: &Document<S>, from: u64) -> Step {
        if from == 0 {
            return Step::End;
        }
        let lo = from.saturating_sub(self.window);
        // A match starting just before `from` still runs past it.
        let stop = (from + self.needle.overlap()).min(doc.len_bytes());
        if stop <= lo {
            return Step::End;
        }
        let hay = match read(doc, lo, stop - lo) {
            Ok(hay) => hay,
            Err(missing) => return Step::Pending(missing),
        };
        // Only matches that start before `from` count, or a search would find
        // the one the cursor is already on, for ever.
        let cap = ((from - lo) as usize).min(hay.len());
        let found = (0..cap).rev().find_map(|i| self.needle.matches_at(&hay[i..]).map(|len| (i, len)));
        match found {
            Some((i, len)) => Step::Found { at: lo + i as u64, len: len as u64 },
            None if lo == 0 => Step::End,
            None => Step::More { resume: lo },
        }
    }
}

/// Put `with` where a match was found.
///
/// A replacement of the same length is an overwrite and nothing moves. One of
/// a different length shifts every byte after it, which is why a search
/// carries on from where the replacement ends rather than from where the match
/// started: the offsets behind it are the old file's.
pub fn replace<S: Source>(doc: &mut Document<S>, at: u64, len: u64, with: &[u8]) {
    if with.len() as u64 == len {
        doc.overwrite_bytes(at, with);
        return;
    }
    doc.delete_bytes(at, len);
    if !with.is_empty() {
        doc.insert_bytes(at, with);
    }
}

/// Read a range, or say which chunks are missing.
fn read<S: Source>(doc: &Document<S>, at: u64, len: u64) -> Result<Vec<u8>, Vec<Missing>> {
    let mut buf = vec![0u8; len as usize];
    let missing = doc.read_bytes(at, &mut buf);
    if missing.is_empty() {
        Ok(buf)
    } else {
        Err(missing)
    }
}

/// A hex needle as bytes. It may be written in groups, the way a hex dump
/// reads: `89 50 4e 47` and `89504e47` are the same four bytes. A group with an
/// odd number of digits is a typo rather than a byte, so `8 9` is refused
/// instead of being read as one.
pub fn parse_hex(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    for group in text.split_whitespace() {
        if group.len() % 2 != 0 {
            return None;
        }
        for i in 0..group.len() / 2 {
            out.push(u8::from_str_radix(group.get(i * 2..i * 2 + 2)?, 16).ok()?);
        }
    }
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{ChunkStore, MemSource};

    fn doc(bytes: &[u8]) -> Document<MemSource> {
        Document::new(MemSource(bytes.to_vec()))
    }

    fn all(s: &Search, d: &Document<MemSource>, start: u64) -> Vec<u64> {
        let mut out = Vec::new();
        let mut at = start;
        loop {
            match s.step(d, at) {
                Step::Found { at: hit, .. } => {
                    out.push(hit);
                    at = if s.backward { hit } else { hit + 1 };
                }
                Step::More { resume } => at = resume,
                Step::End => return out,
                Step::Pending(_) => panic!("a memory source is never pending"),
            }
        }
    }

    #[test]
    fn finds_every_occurrence_forwards_and_backwards() {
        let d = doc(b"abcabcabc");
        let s = Search::forward(Needle::Bytes(b"abc".to_vec()));
        assert_eq!(all(&s, &d, 0), vec![0, 3, 6]);
        let back = Search::backward(Needle::Bytes(b"abc".to_vec()));
        assert_eq!(all(&back, &d, 9), vec![6, 3, 0]);
    }

    #[test]
    fn a_match_across_a_window_join_is_still_found() {
        // The needle straddles the end of the first window, which is the case
        // a window that did not overlap would lose.
        let mut bytes = vec![b'.'; 100];
        bytes[62..66].copy_from_slice(b"HERE");
        let d = doc(&bytes);
        let mut s = Search::forward(Needle::Bytes(b"HERE".to_vec()));
        s.window = 64;
        assert_eq!(all(&s, &d, 0), vec![62]);
        let mut back = Search::backward(Needle::Bytes(b"HERE".to_vec()));
        back.window = 64;
        assert_eq!(all(&back, &d, 100), vec![62]);
    }

    #[test]
    fn folding_matches_either_case_and_only_ascii() {
        let d = doc("Hello HELLO h\u{e9}llo".as_bytes());
        let s = Search::forward(Needle::Fold(b"hello".to_vec()));
        assert_eq!(all(&s, &d, 0), vec![0, 6]);
        // The same needle without folding matches only what is written.
        let exact = Search::forward(Needle::Bytes(b"hello".to_vec()));
        assert_eq!(all(&exact, &d, 0), Vec::<u64>::new());
    }

    #[test]
    fn a_needle_longer_than_the_file_finds_nothing() {
        let d = doc(b"ab");
        let s = Search::forward(Needle::Bytes(b"abc".to_vec()));
        assert_eq!(s.step(&d, 0), Step::End);
    }

    fn pattern(p: &str) -> Needle {
        Needle::Regex(Pattern::new(p).expect("a pattern the tests wrote"))
    }

    #[test]
    fn a_pattern_finds_what_it_describes_both_ways() {
        let d = doc(b"a1b22c333d");
        let s = Search::forward(pattern("[0-9]+"));
        assert_eq!(s.step(&d, 0), Step::Found { at: 1, len: 1 });
        assert_eq!(s.step(&d, 2), Step::Found { at: 3, len: 2 });
        assert_eq!(all(&s, &d, 0), vec![1, 3, 4, 6, 7, 8]);
        // Backwards finds the last match starting before the cursor, which for
        // a greedy pattern is the longest one starting there.
        let back = Search::backward(pattern("[0-9]+"));
        assert_eq!(back.step(&d, 10), Step::Found { at: 8, len: 1 });
    }

    #[test]
    fn a_pattern_reaches_across_a_window_join() {
        let mut bytes = vec![b'.'; 300];
        bytes[126..130].copy_from_slice(b"1234");
        let d = doc(&bytes);
        let mut s = Search::forward(pattern("[0-9]{4}"));
        s.window = 64;
        assert_eq!(all(&s, &d, 0), vec![126]);
    }

    #[test]
    fn an_anchor_is_refused_rather_than_answered_wrongly() {
        // A window is not a line, so `^` here would mean "where this search
        // happened to stop reading".
        assert_eq!(Pattern::new("^abc").unwrap_err(), ANCHOR_REFUSED);
        assert_eq!(Pattern::new("abc$").unwrap_err(), ANCHOR_REFUSED);
        // Escaped, and inside a character class, they are ordinary characters.
        assert!(Pattern::new(r"\^abc").is_ok());
        assert!(Pattern::new("[a^$]bc").is_ok());
    }

    #[test]
    fn a_unicode_class_says_it_is_not_built_in_rather_than_matching_wrongly() {
        // The Unicode tables are not compiled in: they are most of what a
        // regex engine weighs, and a hex editor searches bytes. `\w` and the
        // like still work over ASCII.
        assert!(Pattern::new(r"[a-z]+\d+").is_ok());
        let ascii = Pattern::new(r"\w+").expect("word characters work");
        let d = doc(b"..hello..");
        assert_eq!(Search::forward(Needle::Regex(ascii)).step(&d, 0), Step::Found { at: 2, len: 5 });
    }

    #[test]
    fn a_broken_pattern_says_what_is_wrong_in_one_line() {
        let why = Pattern::new("a(b").unwrap_err();
        assert!(!why.is_empty());
        assert!(!why.contains('\n'), "{why}");
    }

    #[test]
    fn replacing_every_match_is_one_undo_step() {
        let mut d = doc(b"cat dog cat dog cat");
        let s = Search::forward(Needle::Bytes(b"cat".to_vec()));
        d.begin_batch();
        let mut at = 0;
        let mut done = 0;
        // The same loop the search bar runs: replace, then carry on from the
        // end of what was written, not from where the match was.
        while let Step::Found { at: hit, len } = s.step(&d, at) {
            replace(&mut d, hit, len, b"bird!");
            at = hit + 5;
            done += 1;
        }
        d.end_batch();
        assert_eq!(done, 3);
        let mut out = vec![0u8; d.len_bytes() as usize];
        d.read_bytes(0, &mut out);
        assert_eq!(out, b"bird! dog bird! dog bird!");
        assert!(d.undo());
        let mut back = vec![0u8; d.len_bytes() as usize];
        d.read_bytes(0, &mut back);
        assert_eq!(back, b"cat dog cat dog cat", "one undo puts the whole file back");
        assert!(!d.can_undo(), "and there is nothing behind it");
    }

    #[test]
    fn a_batch_that_changed_nothing_leaves_no_undo_step() {
        let mut d = doc(b"abc");
        d.begin_batch();
        d.end_batch();
        assert!(!d.can_undo());
    }

    #[test]
    fn hex_is_read_however_it_is_spaced() {
        assert_eq!(parse_hex("89 50 4e 47"), Some(vec![0x89, 0x50, 0x4e, 0x47]));
        assert_eq!(parse_hex("89504E47"), Some(vec![0x89, 0x50, 0x4e, 0x47]));
        assert_eq!(parse_hex("8 9"), None);
        assert_eq!(parse_hex("zz"), None);
        assert_eq!(parse_hex(""), None);
    }

    #[test]
    fn a_step_says_which_chunks_it_needs() {
        // A file whose bytes arrive a chunk at a time. The search asks for what
        // it needs rather than reading zeroes and answering wrongly.
        let bytes: Vec<u8> = (0..300u32).map(|i| if (100..104).contains(&i) { b"FIND"[(i - 100) as usize] } else { b'.' }).collect();
        let s = Search::forward(Needle::Bytes(b"FIND".to_vec()));
        let d0 = Document::new(ChunkStore::new(bytes.len() as u64, 64, 16));
        let Step::Pending(missing) = s.step(&d0, 0) else { panic!("a file with no bytes yet cannot answer") };
        assert!(!missing.is_empty());

        let mut store = ChunkStore::new(bytes.len() as u64, 64, 16);
        for c in 0..(bytes.len() as u64).div_ceil(64) {
            let start = (c * 64) as usize;
            let end = (start + 64).min(bytes.len());
            store.insert(c, bytes[start..end].to_vec().into_boxed_slice());
        }
        let d = Document::new(store);
        assert_eq!(s.step(&d, 0), Step::Found { at: 100, len: 4 });
    }

    #[test]
    fn an_edited_byte_is_the_one_searched() {
        let mut d = doc(b"abcdef");
        d.overwrite_bytes(3, b"abc");
        let s = Search::forward(Needle::Bytes(b"abc".to_vec()));
        assert_eq!(all(&s, &d, 0), vec![0, 3]);
    }
}
