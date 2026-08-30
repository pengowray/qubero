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
//! There are two ways through, and which one a dump gets is the dump's own
//! doing. Almost everything anyone opens is a machine's output, unedited: the
//! same layout on every line, every line the same length, every address one
//! line's worth past the one above. Such a file is checked once by [`strict`]
//! and then kept as a handful of runs, and a line is read when it is asked for
//! and not before. Everything else is read a line at a time and every line is
//! kept, which is what a shell prompt between two dumps, a column heading, a
//! wrapped line, a screen of box drawing or a `*` standing for a run of
//! identical lines needs.
//!
//! That is the same division a browser makes between a parser for well-formed
//! markup and a parser for what people write, and for the same reason: the
//! strict one is fast because it is allowed to refuse. Both use one line
//! parser, so the two cannot drift apart, and `read_irregular` exists so that
//! a dump can be read both ways and the answers compared.

pub mod glyphs;
pub mod layout;
pub mod lines;
pub mod source;
pub mod strict;
pub mod write;

use crate::gather::Extent;
use crate::text::Settled;
use glyphs::Glyphs;
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

/// Which way the dump was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// Every line the same length and every address following the one above,
    /// so a line is found by arithmetic and read when it is asked for. See
    /// [`strict`].
    Regular,
    /// Something in the dump is not regular, so it was read a line at a time
    /// and every line is remembered.
    Irregular,
}

/// What is kept of a dump once it has been read.
///
/// The two are the same dump and answer the same questions. They differ in
/// what they cost: a run is six numbers however many lines it stands for, and
/// a row is the line's bytes, its digits' places and its agreement, all held.
#[derive(Debug, Clone)]
pub enum Index {
    Runs(Vec<strict::Run>),
    Rows(Vec<Row>),
}

/// A dump, read.
///
/// It borrows the text it was read from, because on the fast path the lines
/// are not kept and a byte is read by going back to the digits that spell it.
#[derive(Debug, Clone)]
pub struct Dump<'a> {
    pub layout: Layout,
    pub index: Index,
    pub notes: Vec<Note>,
    /// Lines that were not part of any dump: prompts, headings, rules, the
    /// output of some other command. Kept because a reader who wondered why a
    /// stretch is missing is owed the line that was there instead.
    pub skipped: Vec<u64>,
    text: &'a [u8],
    base: u64,
}

impl<'a> Dump<'a> {
    pub fn tier(&self) -> Tier {
        match self.index {
            Index::Runs(_) => Tier::Regular,
            Index::Rows(_) => Tier::Irregular,
        }
    }

    /// The stretches of the described file this dump covers, in address order
    /// and joined up where they touch. Answered from the index alone: no line
    /// is read to say what a dump covers.
    pub fn extents(&self) -> Vec<Extent> {
        let mut out: Vec<Extent> = Vec::new();
        let mut push = |at: u64, len: u64| {
            if len == 0 {
                return;
            }
            match out.last_mut() {
                Some(last) if last.end() == at => last.len += len,
                _ => out.push(Extent::new(at, len)),
            }
        };
        match &self.index {
            Index::Runs(runs) => runs.iter().for_each(|r| push(r.address, r.byte_count())),
            Index::Rows(rows) => rows.iter().for_each(|r| push(r.address, r.bytes.len() as u64)),
        }
        out
    }

    /// The first address the dump describes, and the end of the last.
    pub fn span(&self) -> Option<(u64, u64)> {
        let first = self.extents().first().copied()?;
        let last = self.extents().last().copied()?;
        Some((first.at, last.end()))
    }

    /// How many bytes the dump actually spells out.
    pub fn byte_count(&self) -> u64 {
        match &self.index {
            Index::Runs(runs) => runs.iter().map(|r| r.byte_count()).sum(),
            Index::Rows(rows) => rows.iter().map(|r| r.bytes.len() as u64).sum(),
        }
    }

    /// The rows covering an address range, read now.
    ///
    /// On the slow path they are already there. On the fast path the lines
    /// they stand for are found by arithmetic and read here, which is the
    /// whole point of it: a dump of a gigabyte costs an index and a screenful.
    pub fn rows(&self, from: u64, to: u64) -> Vec<Row> {
        match &self.index {
            Index::Rows(rows) => rows
                .iter()
                .filter(|r| r.address < to && r.address + r.bytes.len() as u64 > from)
                .cloned()
                .collect(),
            Index::Runs(runs) => {
                let mut out = Vec::new();
                for run in runs {
                    if run.address >= to || run.end() <= from {
                        continue;
                    }
                    let first = run.locate(from.max(run.address)).map_or(0, |(n, _)| n);
                    let last = run.locate((to - 1).min(run.end() - 1)).map_or(run.lines - 1, |(n, _)| n);
                    for n in first..=last {
                        if let Some(mut row) = self.row_of(run, n) {
                            agree_row(&self.layout, &mut row);
                            out.push(row);
                        }
                    }
                }
                out
            }
        }
    }

    /// Line `n` of a run, read off the text.
    fn row_of(&self, run: &strict::Run, n: u64) -> Option<Row> {
        let (at, len) = run.line_at(n);
        let from = (at - self.base) as usize;
        let to = (from + len as usize).min(self.text.len());
        let line = line_of(self.text, self.base, from, to)?;
        parse_row(&line, &self.layout)
    }

    /// Every place the two columns describe different bytes, as the address of
    /// the byte and what each column said.
    ///
    /// On the fast path this reads every line, which is the one question here
    /// that costs what the dump is long. Ask [`Dump::rows`] over a range
    /// instead when only part of it is in view.
    pub fn conflicts(&self) -> Vec<(u64, char, u8)> {
        let mut out = Vec::new();
        let rows = match &self.index {
            Index::Rows(rows) => rows.clone(),
            Index::Runs(_) => match self.span() {
                Some((from, to)) => self.rows(from, to),
                None => Vec::new(),
            },
        };
        for r in &rows {
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
        if out.is_empty() {
            return 0;
        }
        let rows = self.rows(address, address + out.len() as u64);
        let mut done = 0;
        let mut want = address;
        let mut i = rows.partition_point(|r| r.address + r.bytes.len() as u64 <= want);
        while done < out.len() {
            let Some(r) = rows.get(i) else { break };
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
/// A dump captured off a DOS screen may arrive either as the bytes it was
/// drawn in or as the Unicode something translated them to, and the file does
/// not say which. So both are read and the one whose two columns agree better
/// is kept. A dump that is plainly UTF-8, or carries a byte-order mark, reads
/// the same either way and costs a second pass over a small file.
pub fn read(bytes: &[u8], base: u64) -> Option<Dump<'_>> {
    either_way(bytes, base, false)
}

/// Read a dump line by line, whether or not it is regular enough to be read
/// the other way. Here so the two paths can be made to answer the same
/// questions about the same file, which is the only check that they agree.
pub fn read_irregular(bytes: &[u8], base: u64) -> Option<Dump<'_>> {
    either_way(bytes, base, true)
}

/// Read the text both ways round where the encoding is in doubt, and keep the
/// reading whose two columns agree better.
fn either_way(bytes: &[u8], base: u64, slow: bool) -> Option<Dump<'_>> {
    let bytes = &bytes[..bytes.len().min(LIMIT)];
    let (settled, mark) = lines::reading(bytes);
    let first = read_as(bytes, base, settled, mark, slow);
    if settled == Settled::Cp437 || mark > 0 {
        return first;
    }
    let other = read_as(bytes, base, Settled::Cp437, 0, slow);
    match (first, other) {
        (Some(a), Some(b)) => Some(if b.conflicts().len() < a.conflicts().len() { b } else { a }),
        (a, b) => a.or(b),
    }
}

/// Read the text one way round, taking the fast path when the dump earns it.
fn read_as(bytes: &[u8], base: u64, settled: Settled, mark: usize, slow: bool) -> Option<Dump<'_>> {
    // Working out the layout needs a few lines, not the file. On the fast path
    // these are the only lines that are ever decoded twice.
    let head = &bytes[..bytes.len().min(HEAD_BYTES)];
    let layout = layout::infer(&sample_of(&lines::split(settled, mark, head, base)))?;

    if !slow {
        if let Some(regular) = strict::verify(bytes, base, mark, &layout) {
            return Some(assemble(bytes, base, layout, regular));
        }
    }
    let text = lines::split(settled, mark, bytes, base);
    irregular(bytes, base, &text, layout)
}

/// Finish a dump the fast path found: settle what the layout could not say
/// without looking at the bytes, and read the notes off the lines around it.
fn assemble<'a>(bytes: &'a [u8], base: u64, mut layout: Layout, regular: strict::Regular) -> Dump<'a> {
    let strict::Regular { runs, skipped } = regular;
    let mut dump =
        Dump { layout: layout.clone(), index: Index::Runs(runs), notes: Vec::new(), skipped, text: bytes, base };

    // The order the groups read and the way the characters were written are
    // both judged from rows, so a bounded sample of them is read and dropped.
    let mut sample = match dump.span() {
        Some((from, to)) => dump.rows(from, to.min(from + (SETTLE_ROWS * layout.bytes_per_line) as u64)),
        None => Vec::new(),
    };
    settle_order(&mut layout, &mut sample);
    settle_text(&mut layout, &sample);
    layout.squeeze = None;

    let edges: Vec<Line> =
        dump.skipped.iter().filter_map(|at| line_of(bytes, base, (*at - base) as usize, bytes.len())).collect();
    let end = dump.extents().last().map_or(0, |e| e.end());
    layout.end_address =
        edges.iter().last().is_some_and(|l| l.at > dump.span().map_or(0, |s| s.1) && lone_address(&l.text, &layout) == Some(end));
    dump.notes = notes(&edges, &dump.skipped, end);
    dump.layout = layout;
    dump
}

/// The lines a layout is worked out from: enough to get past a heading and a
/// prompt and still see a dump joining up with itself.
fn sample_of(text: &[Line]) -> Vec<String> {
    text.iter().take(SAMPLE).map(|l| l.text.clone()).collect()
}

/// One line of ASCII text, taken straight off the bytes. Only the fast path
/// uses this, and only after it has refused everything that is not ASCII, so
/// a character index and a byte offset are the same number.
fn line_of(bytes: &[u8], base: u64, from: usize, limit: usize) -> Option<Line> {
    if from >= bytes.len() {
        return None;
    }
    let stop = limit.min(bytes.len());
    let end = bytes[from..stop].iter().position(|b| *b == b'\n').map_or(stop, |i| from + i);
    let text_end = if end > from && bytes[end - 1] == b'\r' { end - 1 } else { end };
    let text = std::str::from_utf8(&bytes[from..text_end]).ok()?.to_string();
    Some(Line { at: base + from as u64, len: (end - from) as u64, text, origin: None })
}

/// Read a dump a line at a time, which copes with anything.
fn irregular<'a>(bytes: &'a [u8], base: u64, text: &[Line], mut layout: Layout) -> Option<Dump<'a>> {
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
    // Both paths settle from at most this many rows, so that a dump read one
    // way is read the same the other way.
    settle_text(&mut layout, &rows[..rows.len().min(SETTLE_ROWS)]);
    for r in &mut rows {
        agree_row(&layout, r);
    }

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
    Some(Dump { layout, index: Index::Rows(rows), notes, skipped, text: bytes, base })
}

/// Lines read before deciding what the layout is. Enough to get past a header
/// and a prompt and still see a dump joining up with itself.
const SAMPLE: usize = 64;

/// How much of the front of the file the layout is worked out from. A heading,
/// a prompt and a screenful of dump fit in far less than this.
const HEAD_BYTES: usize = 16 << 10;

/// Rows read to settle what the layout could not say on its own: which way
/// round a group reads, and how the characters were written. Bounded so that a
/// dump too big to hold is not held in order to answer it, and the same bound
/// on both paths so they answer alike.
const SETTLE_ROWS: usize = 4096;

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

/// How many bytes the character column contradicts, under one set of glyphs
/// and one group order, and what it wrote for the bytes it had none for.
fn disagreement(layout: &Layout, rows: &[Row], glyphs: Glyphs) -> (usize, Vec<char>) {
    let mut stand_ins: Vec<char> = Vec::new();
    let mut pairs: Vec<(u8, char)> = Vec::new();
    for r in rows {
        let Some(col) = column(layout, r) else { continue };
        for (b, c) in r.bytes.iter().zip(col.chars()) {
            match glyphs.of(*b) {
                None => {
                    if !stand_ins.contains(&c) {
                        stand_ins.push(c);
                    }
                }
                Some(_) => pairs.push((*b, c)),
            }
        }
    }
    // A column with more than a handful of stand-ins is not using stand-ins;
    // it is being read the wrong way.
    if stand_ins.len() > 4 {
        return (usize::MAX, Vec::new());
    }
    let bad = pairs.iter().filter(|(b, c)| glyphs.of(*b) != Some(*c) && !stand_ins.contains(c)).count();
    stand_ins.sort_unstable();
    (bad, stand_ins)
}

/// How the character column turned bytes into characters, decided by which way
/// of doing it the column contradicts least.
fn settle_text(layout: &mut Layout, rows: &[Row]) {
    if layout.text.is_none() {
        return;
    }
    let mut best: Option<(usize, Glyphs, Vec<char>)> = None;
    for g in Glyphs::EVERY {
        let (bad, holes) = disagreement(layout, rows, g);
        if best.as_ref().is_none_or(|(n, _, _)| bad < *n) {
            best = Some((bad, g, holes));
        }
    }
    let Some((bad, glyphs, holes)) = best else { return };
    // They all agree on the printable ASCII range, so a column holding nothing
    // else does not choose between them.
    let decided = Glyphs::EVERY.iter().filter(|g| disagreement(layout, rows, **g).0 == bad).count() == 1;
    if !decided {
        layout.assumed.push(Assumed::TextEncoding);
    }
    if let Some(t) = layout.text.as_mut() {
        t.glyphs = glyphs;
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
    let forward = disagreement(layout, rows, Glyphs::Printable(Settled::Ascii)).0;
    let mut flipped = rows.to_vec();
    for r in &mut flipped {
        reverse_groups(&mut r.bytes, &mut r.digits_at, layout.group);
    }
    let backward = disagreement(layout, &flipped, Glyphs::Printable(Settled::Ascii)).0;
    if backward < forward {
        layout.order = Order::ReversedInGroup;
        rows.clone_from_slice(&flipped);
    } else if forward > 0 && forward == backward {
        layout.assumed.push(Assumed::Order);
    }
}

/// Say, for every byte, what its two spellings made of each other.
fn agree_row(layout: &Layout, r: &mut Row) {
    let Some(t) = layout.text.as_ref() else { return };
    let Some(col) = column(layout, r).map(|s| s.to_string()) else { return };
    r.agreement = r
        .bytes
        .iter()
        .zip(col.chars())
        .map(|(b, c)| match t.glyphs.of(*b) {
            _ if t.placeholders.contains(&c) => Agreement::Unverifiable,
            Some(want) if want == c => Agreement::Confirmed,
            Some(_) | None => Agreement::Conflict { wrote: c, digits: *b },
        })
        .collect();
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
        // `Format-Hex` writes "Label:" above its dump and XTree writes "File:"
        // across the top of the screen. Both name what is being looked at.
        if let Some(rest) = t.strip_prefix("Label:").or_else(|| t.strip_prefix("File:")) {
            // XTree writes the mode across the same line, far to the right, so
            // the name stops where the run of spaces after it begins. A path
            // with a space in it survives; two spaces in a row are a column
            // gap rather than part of a name.
            let name = rest.trim().split("  ").next().unwrap_or("").trim();
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


