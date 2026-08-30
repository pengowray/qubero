//! Reading a hex dump back into the bytes it is a dump of.
//!
//! A hex dump is the oldest way of showing a file, and it is a lossy one only
//! by accident: `4d 5a` on the screen is two bytes, written out, and a capture
//! of that screen is those bytes in a form nothing can open. People send them
//! in bug reports, paste them into issues, print them in manuals and take them
//! off machines that have no other way to get a file out. What arrives is text
//! that describes a binary, and the binary is what the reader wanted.
//!
//! So a dump is read here as what it stands for. [`read`] takes the text and
//! gives back the bytes, the addresses they belong at, and where in the text
//! every one of them was written, so nothing is asserted that cannot be pointed
//! at. The layout it was written in is worked out in [`layout`] and never
//! guessed from a tool's name.
//!
//! Three things make this more than a hex parser:
//!
//! * **The two columns check each other.** Most dumps write the bytes twice,
//!   once as digits and once as characters. That is redundancy sitting unused
//!   in nearly every dump ever pasted anywhere. Read both and a corrupted
//!   digit, a line wrapped by an email client or a group written backwards by
//!   `xxd -e` stops being invisible. [`Agreement`] is the per-byte answer, and
//!   it has three cases rather than two: a full stop stands for so many bytes
//!   that most of the column can only ever confirm, never contradict.
//!
//! * **What is missing is not filled in.** A dump of part of a file, a
//!   transcript holding two dumps of different stretches, and a run of
//!   identical lines collapsed to a `*` are all ordinary. The result says which
//!   addresses it covers and leaves the rest as a hole, because a hole is what
//!   is there.
//!
//! * **The file says things about itself.** `Format-Hex` writes the path it
//!   dumped. `certutil` writes the length. A shell transcript has the command
//!   on the line above, arguments and all. That is metadata a hex parser throws
//!   away and a reader wants: it is how a dump of a stretch in the middle of a
//!   file knows it is one.
//!
//! What is not here yet is a lazy index. The dump is read in one go, up to
//! [`LIMIT`], and every line costs a row. A dump laid out regularly needs
//! nothing of the sort, since the line holding an address is arithmetic on the
//! line length; that is the upgrade when a dump arrives too big to hold.

pub mod layout;
pub mod lines;
pub mod write;

use crate::gather::Extent;
use crate::text::{cp437_char, Settled};
use layout::{Assumed, Layout, Order};
use lines::Line;

/// The largest dump this will read. Four times that is the file it describes,
/// and a dump past this size is a reason to build the lazy index rather than a
/// reason to allocate.
pub const LIMIT: usize = 64 << 20;

/// What one byte's two spellings had to say about each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agreement {
    /// The character column spelled the same byte the digits did.
    Confirmed,
    /// The character column wrote its placeholder, which stands for so many
    /// bytes that it rules none of them out.
    Unverifiable,
    /// The two columns describe different bytes. One of them is wrong and the
    /// dump does not say which.
    Conflict { wrote: char, digits: u8 },
}

/// One line of a dump, read.
#[derive(Debug, Clone)]
pub struct Row {
    /// Where the line is in the file, in bytes.
    pub at: u64,
    /// The address the line's first byte belongs at.
    pub address: u64,
    pub bytes: Vec<u8>,
    /// Where each byte's digits were written, as an offset in the file.
    pub digits_at: Vec<u64>,
    /// The character column as it was written, brackets and all.
    pub chars: Option<String>,
    /// What each byte's two spellings said about each other, empty when the
    /// dump has no character column.
    pub agreement: Vec<Agreement>,
    /// Set when the line was a `*` standing for lines the same as the one
    /// before it, in which case the bytes were not written anywhere and the
    /// length of the run comes from the address of the line after it.
    pub implied: bool,
}

/// Something the dump said about itself, in words rather than bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Note {
    /// A path or file name the dump named.
    Named(String),
    /// A length the dump stated, which is not the same as the length of what
    /// it went on to write.
    Length(u64),
    /// A command line the transcript kept.
    Command(String),
}

/// A dump, read.
#[derive(Debug, Clone)]
pub struct Dump {
    pub layout: Layout,
    pub rows: Vec<Row>,
    pub notes: Vec<Note>,
    /// Lines that were not part of any dump: prompts, headings, rules, the
    /// output of some other command. Kept because a reader who wondered why a
    /// stretch is missing is owed the line that was there instead.
    pub skipped: Vec<u64>,
}

impl Dump {
    /// The stretches of the described file this dump covers, in address order
    /// and joined up where they touch.
    pub fn extents(&self) -> Vec<Extent> {
        let mut out: Vec<Extent> = Vec::new();
        for r in &self.rows {
            let e = Extent::new(r.address, r.bytes.len() as u64);
            match out.last_mut() {
                Some(last) if last.end() == e.at => last.len += e.len,
                _ => out.push(e),
            }
        }
        out
    }

    /// The first address the dump describes, and the end of the last.
    pub fn span(&self) -> Option<(u64, u64)> {
        let first = self.rows.first()?;
        let last = self.rows.last()?;
        Some((first.address, last.address + last.bytes.len() as u64))
    }

    /// How many bytes the dump actually spells out.
    pub fn byte_count(&self) -> u64 {
        self.rows.iter().map(|r| r.bytes.len() as u64).sum()
    }

    /// Every place the two columns describe different bytes, as the address of
    /// the byte and what each column said.
    pub fn conflicts(&self) -> Vec<(u64, char, u8)> {
        let mut out = Vec::new();
        for r in &self.rows {
            for (i, a) in r.agreement.iter().enumerate() {
                if let Agreement::Conflict { wrote, digits } = *a {
                    out.push((r.address + i as u64, wrote, digits));
                }
            }
        }
        out
    }

    /// The bytes at `address`, as far as the dump covers them, into `out`.
    /// Returns how many were filled; a hole stops the read rather than being
    /// filled with anything.
    pub fn read_at(&self, address: u64, out: &mut [u8]) -> usize {
        let mut done = 0;
        let mut want = address;
        // The rows are in address order, so the one holding an address is a
        // search rather than a walk.
        let mut i = self.rows.partition_point(|r| r.address + r.bytes.len() as u64 <= want);
        while done < out.len() {
            let Some(r) = self.rows.get(i) else { break };
            if r.address > want {
                break;
            }
            let from = (want - r.address) as usize;
            let n = (r.bytes.len() - from).min(out.len() - done);
            out[done..done + n].copy_from_slice(&r.bytes[from..from + n]);
            done += n;
            want += n as u64;
            i += 1;
        }
        done
    }
}

/// Read a dump out of the text of one.
///
/// `base` is where `bytes` starts in the file, which is zero for a whole file
/// and something else for a dump found inside one.
pub fn read(bytes: &[u8], base: u64) -> Option<Dump> {
    let bytes = &bytes[..bytes.len().min(LIMIT)];
    let (settled, mark) = lines::reading(bytes);
    let text = lines::split(settled, mark, bytes, base);
    read_lines(&text)
}

/// Read a dump out of lines that have already been decoded.
pub fn read_lines(text: &[Line]) -> Option<Dump> {
    let sample: Vec<String> = text.iter().take(SAMPLE).map(|l| l.text.clone()).collect();
    let mut layout = layout::infer(&sample)?;
    let mut rows = Vec::new();
    let mut skipped = Vec::new();
    let mut squeezed = false;
    for line in text {
        if line.text.trim() == "*" {
            squeezed = true;
            rows.push(Row { at: line.at, address: 0, bytes: Vec::new(), digits_at: Vec::new(), chars: None, agreement: Vec::new(), implied: true });
            continue;
        }
        match parse_row(line, &layout) {
            Some(row) => rows.push(row),
            None => skipped.push(line.at),
        }
    }

    layout.squeeze = squeezed.then_some('*');

    // A run of identical lines has its length only in the addresses either side
    // of the `*`, so it is filled in once every real line has been read.
    fill_squeezed(&mut rows, layout.bytes_per_line);
    rows.retain(|r| !r.bytes.is_empty());

    // A dump with an address column places its rows; one without has them in
    // the order they were written.
    if layout.address.is_none() {
        let mut at = 0u64;
        for r in &mut rows {
            r.address = at;
            at += r.bytes.len() as u64;
        }
    }
    rows.sort_by_key(|r| r.address);

    // Which way round the groups read is settled first, and against plain
    // ASCII: every encoding here agrees on that range, and the encoding cannot
    // be judged from bytes that are still in the wrong order.
    settle_order(&mut layout, &mut rows);
    settle_text(&mut layout, &rows);
    agree(&layout, &mut rows);

    let end = rows.last().map_or(0, |r| r.address + r.bytes.len() as u64);
    skipped.sort_unstable();
    // A last line that is nothing but the address after the last byte, which is
    // how `od` says how long the file was.
    layout.end_address = text
        .iter()
        .rev()
        .find(|l| !l.text.trim().is_empty())
        .is_some_and(|l| skipped.binary_search(&l.at).is_ok() && lone_address(&l.text, &layout) == Some(end));
    let notes = notes(text, &skipped, end);
    Some(Dump { layout, rows, notes, skipped })
}

/// Lines read before deciding what the layout is. Enough to get past a header
/// and a prompt and still see a dump joining up with itself.
const SAMPLE: usize = 64;

/// Read one line as a row of the dump, or decide it is not one.
fn parse_row(line: &Line, layout: &Layout) -> Option<Row> {
    let toks = layout::tokens(&line.text);
    let (address, skip) = match &layout.address {
        Some(a) => {
            let first = toks.first()?;
            let digits = match a.suffix {
                Some(c) => first.s.strip_suffix(c)?,
                None => first.s,
            };
            if digits.is_empty() || !digits.bytes().all(|b| (b as char).is_digit(a.base.radix())) {
                return None;
            }
            if a.digits.is_some_and(|d| digits.chars().count() != d) {
                return None;
            }
            (u64::from_str_radix(digits, a.base.radix()).ok()?, 1)
        }
        None => (0, 0),
    };

    let mut bytes = Vec::new();
    let mut digits_at = Vec::new();
    for t in &toks[skip..] {
        if bytes.len() >= layout.bytes_per_line {
            break;
        }
        if t.s.is_empty() || t.s.len() % 2 != 0 || !t.s.bytes().all(|b| b.is_ascii_hexdigit()) {
            break;
        }
        // Stop before the character column, which on a line of printable bytes
        // can be a run of hex digits itself.
        if layout.text_at.is_some_and(|c| t.at >= c) {
            break;
        }
        for (j, pair) in t.s.as_bytes().chunks(2).enumerate() {
            let v = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
            bytes.push(v);
            digits_at.push(line.origin_of(t.at + j * 2));
        }
    }
    if bytes.is_empty() {
        return None;
    }
    if layout.order == Order::ReversedInGroup {
        reverse_groups(&mut bytes, &mut digits_at, layout.group);
    }
    let chars = layout.text_at.and_then(|c| {
        let s: String = line.text.chars().skip(c).collect();
        (!s.is_empty()).then_some(s)
    });
    Some(Row { at: line.at, address, bytes, digits_at, chars, agreement: Vec::new(), implied: false })
}

fn reverse_groups(bytes: &mut [u8], digits_at: &mut [u64], group: usize) {
    if group < 2 {
        return;
    }
    for c in bytes.chunks_mut(group) {
        c.reverse();
    }
    for c in digits_at.chunks_mut(group) {
        c.reverse();
    }
}

/// Give the `*` lines the bytes they stand for.
///
/// The marker means "as many more lines like the one above as it takes to reach
/// the line below", so the run's length is the difference of two addresses and
/// exists nowhere else in the file. A marker with nothing after it stands for
/// nothing that can be measured, and is dropped.
fn fill_squeezed(rows: &mut [Row], per_line: usize) {
    for i in 0..rows.len() {
        if !rows[i].implied {
            continue;
        }
        let (Some(before), Some(after)) = (i.checked_sub(1), rows.get(i + 1)) else { continue };
        if rows[before].bytes.is_empty() {
            continue;
        }
        let from = rows[before].address + rows[before].bytes.len() as u64;
        let to = after.address;
        if to <= from || per_line == 0 {
            continue;
        }
        let pattern = rows[before].bytes.clone();
        let count = (to - from) as usize;
        let mut bytes = Vec::with_capacity(count);
        while bytes.len() < count {
            let n = (count - bytes.len()).min(pattern.len());
            bytes.extend_from_slice(&pattern[..n]);
        }
        let at = rows[i].at;
        rows[i].address = from;
        rows[i].digits_at = vec![at; bytes.len()];
        rows[i].bytes = bytes;
    }
}

/// The characters of the column, with the brackets `od` puts round them off.
fn column<'a>(layout: &Layout, row: &'a Row) -> Option<&'a str> {
    let t = layout.text.as_ref()?;
    let s = row.chars.as_deref()?;
    let s = match t.open {
        Some(c) => s.strip_prefix(c)?,
        None => s,
    };
    Some(match t.close {
        Some(c) => s.split(c).next().unwrap_or(s),
        None => s,
    })
}

/// How many bytes the character column contradicts, under one encoding and one
/// group order, and what it wrote for the bytes it would not print.
fn disagreement(layout: &Layout, rows: &[Row], enc: Settled) -> (usize, Vec<char>) {
    let mut wrote_for_unprintable: Vec<char> = Vec::new();
    let mut pairs: Vec<(u8, char)> = Vec::new();
    for r in rows {
        let Some(col) = column(layout, r) else { continue };
        for (b, c) in r.bytes.iter().zip(col.chars()) {
            match printable(enc, *b) {
                None => {
                    if !wrote_for_unprintable.contains(&c) {
                        wrote_for_unprintable.push(c);
                    }
                }
                Some(_) => pairs.push((*b, c)),
            }
        }
    }
    // A tool with more than a handful of stand-ins is not using stand-ins; it
    // is being read in the wrong encoding.
    if wrote_for_unprintable.len() > 4 {
        return (usize::MAX, Vec::new());
    }
    let bad = pairs
        .iter()
        .filter(|(b, c)| printable(enc, *b) != Some(*c) && !wrote_for_unprintable.contains(c))
        .count();
    wrote_for_unprintable.sort_unstable();
    (bad, wrote_for_unprintable)
}

/// Which encoding the character column is in, decided by which one it
/// contradicts least.
fn settle_text(layout: &mut Layout, rows: &[Row]) {
    if layout.text.is_none() {
        return;
    }
    let mut best: Option<(usize, Settled, Vec<char>)> = None;
    for enc in [Settled::Ascii, Settled::Latin1, Settled::Cp437] {
        let (bad, holes) = disagreement(layout, rows, enc);
        if best.as_ref().is_none_or(|(n, _, _)| bad < *n) {
            best = Some((bad, enc, holes));
        }
    }
    let Some((bad, enc, holes)) = best else { return };
    // Every encoding agrees on the printable ASCII range, so a column holding
    // nothing else does not choose between them.
    let decided = [Settled::Ascii, Settled::Latin1, Settled::Cp437]
        .iter()
        .filter(|e| disagreement(layout, rows, **e).0 == bad)
        .count()
        == 1;
    if !decided {
        layout.assumed.push(Assumed::TextEncoding);
    }
    if let Some(t) = layout.text.as_mut() {
        t.encoding = enc;
        if !holes.is_empty() {
            t.placeholders = holes;
        }
    }
}

/// Which end of a group its first byte is at.
///
/// `xxd -e` writes each group as a little-endian number, so the digits read
/// backwards inside it. Nothing on the line says so. The character column does,
/// because it still reads in file order, and it is the only thing that can:
/// with no character column the digits are taken at their word.
fn settle_order(layout: &mut Layout, rows: &mut [Row]) {
    if layout.group < 2 || layout.text.is_none() {
        if layout.group >= 2 {
            layout.assumed.push(Assumed::Order);
        }
        return;
    }
    let forward = disagreement(layout, rows, Settled::Ascii).0;
    let mut flipped = rows.to_vec();
    for r in &mut flipped {
        reverse_groups(&mut r.bytes, &mut r.digits_at, layout.group);
    }
    let backward = disagreement(layout, &flipped, Settled::Ascii).0;
    if backward < forward {
        layout.order = Order::ReversedInGroup;
        rows.clone_from_slice(&flipped);
    } else if forward > 0 && forward == backward {
        layout.assumed.push(Assumed::Order);
    }
}

/// Say, for every byte, what its two spellings made of each other.
fn agree(layout: &Layout, rows: &mut [Row]) {
    let Some(t) = layout.text.clone() else { return };
    for r in rows {
        let Some(col) = column(layout, r).map(|s| s.to_string()) else { continue };
        r.agreement = r
            .bytes
            .iter()
            .zip(col.chars())
            .map(|(b, c)| match printable(t.encoding, *b) {
                _ if t.placeholders.contains(&c) => Agreement::Unverifiable,
                Some(want) if want == c => Agreement::Confirmed,
                Some(_) | None => Agreement::Conflict { wrote: c, digits: *b },
            })
            .collect();
    }
}

/// What the dump said about itself in words.
///
/// Only lines that were not part of the dump are read this way, and only the
/// few shapes that actually carry something: the label `Format-Hex` writes, a
/// length on a line of its own, and the command a transcript kept above its
/// output. Anything looser would read a sentence out of a shell prompt.
fn notes(text: &[Line], skipped: &[u64], end: u64) -> Vec<Note> {
    let mut out = Vec::new();
    for line in text.iter().filter(|l| skipped.binary_search(&l.at).is_ok()) {
        let t = line.text.trim();
        if let Some(rest) = t.strip_prefix("Label:") {
            let name = rest.trim();
            if !name.is_empty() {
                out.push(Note::Named(name.to_string()));
            }
            continue;
        }
        let toks = layout::tokens(t);
        if toks.len() == 1 && toks[0].s.bytes().all(|b| b.is_ascii_hexdigit()) && toks[0].s.len() >= 2 {
            if let Ok(v) = u64::from_str_radix(toks[0].s, 16) {
                if v >= end && v > 0 {
                    out.push(Note::Length(v));
                }
            }
            continue;
        }
        if let Some(cmd) = command(t) {
            out.push(Note::Command(cmd.to_string()));
            // The file is the last thing on the line, where it is anything:
            // taking the first argument that is not an option takes the value
            // of the option before it instead.
            if let Some(name) = cmd.split_whitespace().last().filter(|a| !a.starts_with('-') && cmd.split_whitespace().count() > 1) {
                out.push(Note::Named(name.to_string()));
            }
        }
    }
    out.dedup();
    out
}

/// The value of a line that is one address and nothing else.
fn lone_address(line: &str, layout: &Layout) -> Option<u64> {
    let a = layout.address.as_ref()?;
    let toks = layout::tokens(line.trim());
    let [t] = toks[..] else { return None };
    let digits = match a.suffix {
        Some(c) => t.s.strip_suffix(c).unwrap_or(t.s),
        None => t.s,
    };
    u64::from_str_radix(digits, a.base.radix()).ok()
}

/// The dumping command on a transcript's line, with the prompt in front of it
/// taken off. The tools are named rather than guessed at, because a line of a
/// transcript is a line of anything.
fn command(line: &str) -> Option<&str> {
    const TOOLS: [&str; 6] = ["xxd", "od", "hexdump", "certutil", "Format-Hex", "hexyl"];
    let body = match line.rfind(|c| c == '$' || c == '>' || c == '#') {
        Some(i) if i + 1 < line.len() => line[i + 1..].trim(),
        _ => line,
    };
    let first = body.split_whitespace().next()?;
    let name = first.rsplit(['/', '\\']).next()?;
    TOOLS.contains(&name).then_some(body)
}

/// The character a tool would write for a byte, or nothing when it would write
/// its placeholder instead.
pub fn printable(enc: Settled, b: u8) -> Option<char> {
    match enc {
        Settled::Latin1 => ((0x20..=0x7e).contains(&b) || b >= 0xa0).then(|| b as char),
        Settled::Cp437 => (b >= 0x20 && b != 0x7f).then(|| cp437_char(b)),
        _ => (0x20..=0x7e).contains(&b).then(|| b as char),
    }
}

