//! What a program was built to withstand, the way `checksec` reports it, with
//! every answer tied to the bytes it was read from.
//!
//! `checksec` prints a verdict per protection: Full RELRO, Canary found, NX
//! enabled. A verdict on its own asks to be taken on trust, and the point of
//! reading one here is that it need not be: each [`Row`] carries the
//! [`Evidence`] it rests on, a field or a run of bytes the view can go to, so
//! "full RELRO" is a `GNU_RELRO` segment and a `BIND_NOW` entry that a reader
//! can open and check.
//!
//! The pass reads the parsed tree where the template reads a structure, and
//! the file's bytes where it does not: the strings of a large symbol table,
//! a PE's load configuration, a Mach-O's string table. Every raw read goes
//! through [`read_at`], which answers `Pending` for bytes not loaded yet the
//! way a field does, so a caller that fetches what was asked for and asks
//! again gets the answer. Nothing here reads every record of a symbol table
//! that can run to millions: a static symbol table is searched as the bytes
//! of its string table, which is what `checksec`'s `grep` over `readelf -s`
//! amounts to.
//!
//! Facts only. A row's `key`, `state` and `verdict` are short fixed words,
//! and what they are called on screen is the view's to say.

mod elf;

use crate::document::Document;
use crate::eval::{EvalError, Evaluator, R, Value};
use crate::source::{Missing, Source};

/// Everything the pass found in one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hardening {
    /// `elf`, `pe` or `macho`.
    pub format: &'static str,
    /// One program, or one per slice of a universal Mach-O.
    pub parts: Vec<Part>,
}

/// One program's rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    /// Empty for a file holding one program; the CPU a universal Mach-O's
    /// slice is for otherwise.
    pub name: String,
    /// The program's header in the parsed tree.
    pub path: Vec<usize>,
    pub rows: Vec<Row>,
}

/// One protection, or one fact about how the program is put together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Which protection: `relro`, `canary`, `nx`, and so on.
    pub key: &'static str,
    /// What was found, in a word fixed per key: `full`, `partial`, `none`.
    pub state: &'static str,
    pub rating: Rating,
    /// A count where the row has one: fortified functions, symbols, writable
    /// and executable segments. `total` is what `count` is out of.
    pub count: Option<u64>,
    pub total: Option<u64>,
    /// The names the row is about, and the ones it is not: fortified calls in
    /// `items` and the unfortified ones in `more`, a library search path's
    /// directories, the libraries a program needs.
    pub items: Vec<String>,
    pub more: Vec<String>,
    /// What the answer rests on, most direct first. At most [`EVIDENCE`].
    pub evidence: Vec<Evidence>,
}

/// How good a row's answer is for the program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rating {
    Good,
    Partial,
    Bad,
    /// A fact worth knowing that is neither good nor bad.
    Info,
    /// The file does not hold what the answer would need: a program with no
    /// symbols at all cannot say whether it was built with a stack canary.
    Unknown,
    /// The question does not apply: an object file is not loaded, so how it
    /// would be loaded has no answer.
    NotApplicable,
}

impl Rating {
    pub fn as_str(self) -> &'static str {
        match self {
            Rating::Good => "good",
            Rating::Partial => "partial",
            Rating::Bad => "bad",
            Rating::Info => "info",
            Rating::Unknown => "unknown",
            Rating::NotApplicable => "n/a",
        }
    }
}

/// A field or a run of bytes an answer rests on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    /// The node in the parsed tree that holds the bytes, or empty when no
    /// field covers them: a Mach-O's string table, a PE's load configuration.
    pub path: Vec<usize>,
    pub offset_bits: u64,
    pub size_bits: u64,
    /// What kind of thing it is: `segment`, `section`, `header`, `dynamic`,
    /// `symbol`, `string`, `note`, `directory`, `command` or `bytes`.
    pub what: &'static str,
    /// Its name as the format writes it: `GNU_STACK`, `BIND_NOW`,
    /// `__stack_chk_fail`, `.symtab`, `dll_characteristics`.
    pub name: String,
}

/// How many pieces of evidence a row carries at most. A program that imports
/// two hundred fortified functions is shown to by its first dozen.
pub const EVIDENCE: usize = 12;

impl Row {
    fn new(key: &'static str, state: &'static str, rating: Rating) -> Row {
        Row { key, state, rating, count: None, total: None, items: Vec::new(), more: Vec::new(), evidence: Vec::new() }
    }

    fn count(mut self, n: u64) -> Row {
        self.count = Some(n);
        self
    }

    fn total(mut self, n: u64) -> Row {
        self.total = Some(n);
        self
    }

    fn items(mut self, items: Vec<String>) -> Row {
        self.items = items;
        self
    }

    fn more(mut self, more: Vec<String>) -> Row {
        self.more = more;
        self
    }

    fn evidence(mut self, evidence: impl IntoIterator<Item = Evidence>) -> Row {
        self.evidence = evidence.into_iter().take(EVIDENCE).collect();
        self
    }
}

/// The protections of the program the evaluator is reading, or nothing when
/// its template is not one this knows: `elf`, `pe` or `macho`.
pub fn read<S: Source>(ev: &mut Evaluator, doc: &Document<S>) -> R<Option<Hardening>> {
    let parts = match ev.template().name.as_str() {
        "elf" => ("elf", elf::read(ev, doc)?),
        _ => return Ok(None),
    };
    Ok(Some(Hardening { format: parts.0, parts: parts.1 }))
}

/// The `_chk` functions of glibc 2.39's `libc.so.6`, each without the `__` in
/// front and the `_chk` after: `printf` here is `__printf_chk` there.
///
/// `checksec` reads this list out of whatever libc the machine running it has,
/// so its fortifiable count depends on the machine. This is the list from the
/// libc it was checked against, kept here so that the count depends on the
/// program alone. A function is fortifiable when a program calls it by either
/// name, and fortified when it calls the `_chk` one.
const FORTIFIABLE: &[&str] = &[
    "asprintf", "confstr", "dprintf", "explicit_bzero", "fdelt", "fgets", "fgets_unlocked", "fgetws",
    "fgetws_unlocked", "fprintf", "fread", "fread_unlocked", "fwprintf", "getcwd", "getdomainname", "getgroups",
    "gethostname", "getlogin_r", "gets", "getwd", "longjmp", "mbsnrtowcs", "mbsrtowcs", "mbstowcs", "memcpy",
    "memmove", "mempcpy", "memset", "obstack_printf", "obstack_vprintf", "poll", "ppoll", "pread64", "pread",
    "printf", "ptsname_r", "read", "readlinkat", "readlink", "realpath", "recv", "recvfrom", "snprintf", "sprintf",
    "stpcpy", "stpncpy", "strcat", "strcpy", "strlcat", "strlcpy", "strncat", "strncpy", "swprintf", "syslog",
    "ttyname_r", "vasprintf", "vdprintf", "vfprintf", "vfwprintf", "vprintf", "vsnprintf", "vsprintf", "vswprintf",
    "vsyslog", "vwprintf", "wcpcpy", "wcpncpy", "wcrtomb", "wcscat", "wcscpy", "wcslcat", "wcslcpy", "wcsncat",
    "wcsncpy", "wcsnrtombs", "wcsrtombs", "wcstombs", "wctomb", "wmemcpy", "wmemmove", "wmempcpy", "wmemset",
    "wprintf",
];

/// A name the way `checksec` compares it: leading underscores and any
/// `@version` dropped, so `__printf_chk@GLIBC_2.3.4` is `printf_chk`.
fn bare(name: &str) -> &str {
    let name = name.trim_start_matches('_');
    name.split('@').next().unwrap_or(name)
}

/// Which imported names are fortified and which could have been.
///
/// `checksec` counts lines of `readelf --dyn-syms`, so a name listed twice is
/// counted twice. This counts each name once: the question is which functions
/// the program calls, and two entries for one function are still one function.
struct Fortify<'a> {
    /// Names as the program writes them, each with what it was found at.
    fortified: Vec<(&'a str, usize)>,
    unfortified: Vec<(&'a str, usize)>,
    /// Whether any name ends in `_chk`, the libc list or not: `checksec`'s
    /// "FORTIFY: Yes".
    any_chk: bool,
}

impl<'a> Fortify<'a> {
    /// `names` are the imported symbols, each with an index the caller can
    /// turn back into evidence.
    fn of(names: impl IntoIterator<Item = (&'a str, usize)>) -> Fortify<'a> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Fortify { fortified: Vec::new(), unfortified: Vec::new(), any_chk: false };
        for (name, at) in names {
            let b = bare(name);
            if b.ends_with("_chk") {
                out.any_chk = true;
            }
            if !seen.insert(b) {
                continue;
            }
            match b.strip_suffix("_chk") {
                Some(f) if FORTIFIABLE.contains(&f) => out.fortified.push((name, at)),
                _ if FORTIFIABLE.contains(&b) => out.unfortified.push((name, at)),
                _ => {}
            }
        }
        out
    }

    /// The row, given a way to turn a symbol's index into evidence.
    fn row(&self, evidence: impl Fn(usize) -> Evidence) -> Row {
        let count = self.fortified.len() as u64;
        let total = count + self.unfortified.len() as u64;
        let rating = match (count, total) {
            (0, 0) => Rating::Info,
            (0, _) => Rating::Bad,
            _ => Rating::Good,
        };
        // The calls that make the verdict, first: the fortified ones where
        // there are any, and the plain ones that could have been where not.
        let shown = if count > 0 { &self.fortified } else { &self.unfortified };
        Row::new("fortify", if self.any_chk { "yes" } else { "no" }, rating)
            .count(count)
            .total(total)
            .items(self.fortified.iter().map(|(n, _)| n.to_string()).collect())
            .more(self.unfortified.iter().map(|(n, _)| n.to_string()).collect())
            .evidence(shown.iter().map(|&(_, at)| evidence(at)))
    }
}

/// `len` bytes at byte `at` of the document, as many as the file has. Bytes
/// not loaded yet are asked for, the way a field asks for them.
fn read_at<S: Source>(doc: &Document<S>, at: u64, len: u64) -> R<Vec<u8>> {
    let len = len.min(doc.len_bytes().saturating_sub(at));
    let mut out = vec![0u8; len as usize];
    let missing = doc.read_bytes(at, &mut out);
    if !missing.is_empty() {
        return Err(EvalError::Pending(missing));
    }
    Ok(out)
}

/// The NUL-terminated string at `at` in `table`, or empty past its end.
fn cstr(table: &[u8], at: u64) -> &str {
    let Some(rest) = usize::try_from(at).ok().and_then(|at| table.get(at..)) else { return "" };
    let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
    std::str::from_utf8(&rest[..end]).unwrap_or("")
}

/// A string found in a string table by searching its bytes.
#[derive(Debug, Clone)]
struct Hit {
    /// Which of the needles it holds.
    needle: usize,
    /// Where the whole string starts, in bytes of the file, and the string.
    at: u64,
    text: String,
    /// How many strings come before it in the table, counting one per NUL,
    /// which is its index in a table the template reads as a run of strings.
    /// None where the string's start was not found.
    index: Option<u64>,
}

/// How much a search over a string table reads at once, and how far either
/// side of that it looks to find where a matched string starts and ends.
const WINDOW: u64 = 1 << 20;
const PAD: u64 = 4096;

/// Every string in the bytes `from..to` of the file holding one of `needles`,
/// up to `keep` of them for each needle, and how many there were in all.
///
/// The bytes are read a window at a time, so a debug build's string table of
/// a hundred megabytes is never in memory at once. Whatever is not loaded yet
/// is asked for in one go: a first pass over the windows collects every chunk
/// missing, and only a table that is all there is searched.
fn search<S: Source>(doc: &Document<S>, from: u64, to: u64, needles: &[&[u8]], keep: usize) -> R<(Vec<Hit>, Vec<u64>)> {
    let to = to.min(doc.len_bytes());
    let mut missing: Vec<u64> = Vec::new();
    let mut at = from;
    while at < to {
        let len = WINDOW.min(to - at);
        let mut buf = vec![0u8; len as usize];
        missing.extend(doc.read_bytes(at, &mut buf).into_iter().map(|m| m.chunk));
        at += len;
    }
    if !missing.is_empty() {
        missing.sort_unstable();
        missing.dedup();
        return Err(EvalError::Pending(missing.into_iter().map(|chunk| Missing { chunk }).collect()));
    }
    let mut hits = Vec::new();
    let mut counts = vec![0u64; needles.len()];
    let mut nuls_before = 0u64;
    let mut core = from;
    while core < to {
        let core_end = (core + WINDOW).min(to);
        let lo = core.saturating_sub(PAD).max(from);
        let hi = (core_end + PAD).min(to);
        let buf = read_at(doc, lo, hi - lo)?;
        let mut first = [false; 256];
        for n in needles {
            first[n[0] as usize] = true;
        }
        let nuls = |a: u64, b: u64| buf[(a - lo) as usize..(b - lo) as usize].iter().filter(|&&c| c == 0).count() as u64;
        for i in (core - lo) as usize..(core_end - lo) as usize {
            if !first[buf[i] as usize] {
                continue;
            }
            for (n, needle) in needles.iter().enumerate() {
                if !buf[i..].starts_with(needle) {
                    continue;
                }
                counts[n] += 1;
                if hits.iter().filter(|h: &&Hit| h.needle == n).count() >= keep {
                    continue;
                }
                let start = buf[..i].iter().rposition(|&c| c == 0).map(|p| p + 1);
                let end = buf[i..].iter().position(|&c| c == 0).map_or(buf.len(), |p| i + p);
                let begin = start.unwrap_or(0);
                let text = String::from_utf8_lossy(&buf[begin..end]).into_owned();
                let s = lo + begin as u64;
                let index = start.map(|_| if s >= core { nuls_before + nuls(core, s) } else { nuls_before - nuls(s, core) });
                hits.push(Hit { needle: n, at: s, text, index });
            }
        }
        nuls_before += nuls(core, core_end);
        core = core_end;
    }
    Ok((hits, counts))
}

/// A path one step further in.
fn child(path: &[usize], i: usize) -> Vec<usize> {
    let mut p = path.to_vec();
    p.push(i);
    p
}

/// The child a structure calls `name`, or an error naming what was missing.
fn named<S: Source>(ev: &mut Evaluator, doc: &Document<S>, path: &[usize], name: &str) -> R<Vec<usize>> {
    match ev.child_named(doc, path, name)? {
        Some(p) => Ok(p),
        None => Err(EvalError::Failed(format!("no field {name} at {path:?}"))),
    }
}

/// The number a node holds, read through the one-child node an `at` makes.
fn int_at<S: Source>(ev: &mut Evaluator, doc: &Document<S>, path: &[usize]) -> R<u64> {
    let n = ev.node(doc, path)?;
    match n.value {
        Value::Composite { count: 1 } if n.composite => int_at(ev, doc, &child(path, 0)),
        v => match v.as_int() {
            Some(i) => Ok(i as u64),
            None => Err(EvalError::Failed(format!("{path:?} is not a number: {v:?}"))),
        },
    }
}

fn int_field<S: Source>(ev: &mut Evaluator, doc: &Document<S>, path: &[usize], name: &str) -> R<u64> {
    let p = named(ev, doc, path, name)?;
    int_at(ev, doc, &p)
}

/// The text a node holds, read through the one-child node an `at` makes, or
/// nothing for a node that is not text: a field that is not there, or the
/// empty bytes a name with nowhere to come from reads as.
fn text_at<S: Source>(ev: &mut Evaluator, doc: &Document<S>, path: &[usize]) -> R<Option<(String, Vec<usize>)>> {
    let n = ev.node(doc, path)?;
    match n.value {
        Value::Str(s) => Ok(Some((s, path.to_vec()))),
        Value::Composite { count: 1 } if n.composite => text_at(ev, doc, &child(path, 0)),
        _ => Ok(None),
    }
}

/// A node as evidence: where it is and what it is.
fn node_evidence<S: Source>(ev: &mut Evaluator, doc: &Document<S>, path: &[usize], what: &'static str, name: &str) -> R<Evidence> {
    let n = ev.node(doc, path)?;
    Ok(Evidence { path: path.to_vec(), offset_bits: n.offset_bits, size_bits: n.size_bits, what, name: name.to_string() })
}

/// Bytes no field covers, as evidence.
fn bytes_evidence(at: u64, len: u64, what: &'static str, name: &str) -> Evidence {
    Evidence { path: Vec::new(), offset_bits: at * 8, size_bits: len * 8, what, name: name.to_string() }
}

/// A word of `len` bytes at `at` in `b`, little- or big-endian. Zero past the
/// end, which a caller has measured against already.
fn word(b: &[u8], at: usize, len: usize, little: bool) -> u64 {
    let Some(bytes) = b.get(at..at + len) else { return 0 };
    let mut v = 0u64;
    for i in 0..len {
        let byte = if little { bytes[len - 1 - i] } else { bytes[i] };
        v = (v << 8) | byte as u64;
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_compared_without_its_underscores_or_its_version() {
        assert_eq!(bare("__printf_chk@GLIBC_2.3.4"), "printf_chk");
        assert_eq!(bare("memcpy"), "memcpy");
        assert_eq!(bare("___stack_chk_fail"), "stack_chk_fail");
    }

    /// Each function counted once, however many times the table lists it, and
    /// `FORTIFY: yes` for any `_chk` name at all, which is what `checksec`
    /// says too.
    #[test]
    fn a_function_is_fortified_once_however_often_it_is_listed() {
        let names = ["__printf_chk", "__printf_chk", "memcpy", "strlen", "__foo_chk", "_memcpy"];
        let f = Fortify::of(names.iter().enumerate().map(|(i, n)| (*n, i)));
        let row = f.row(|i| bytes_evidence(i as u64, 1, "symbol", names[i]));
        assert_eq!((row.state, row.count, row.total), ("yes", Some(1), Some(2)));
        assert_eq!(row.items, ["__printf_chk"]);
        assert_eq!(row.more, ["memcpy"]);
        assert_eq!(row.rating, Rating::Good);
        assert_eq!(row.evidence.len(), 1);
    }

    #[test]
    fn a_search_finds_whole_strings_and_counts_them_all() {
        use crate::source::MemSource;
        let table = b"\0foo\0__stack_chk_fail@GLIBC_2.4\0bar.cfi\0baz.cfi\0".to_vec();
        let doc = Document::new(MemSource(table.clone()));
        let (hits, counts) = search(&doc, 0, table.len() as u64, &[b"__stack_chk_fail", b".cfi\0"], 1).unwrap();
        assert_eq!(counts, [1, 2]);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].text, "__stack_chk_fail@GLIBC_2.4");
        assert_eq!((hits[0].at, hits[0].index), (5, Some(2)));
        assert_eq!((hits[1].text.as_str(), hits[1].index), ("bar.cfi", Some(3)));
    }
}
