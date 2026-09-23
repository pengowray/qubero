//! Where every byte of a file goes, grouped the way a reader asks about it.
//!
//! The report's "Where the bytes go" section and its full ledger are drawn
//! from this. One grouping rule and no judgment (see `docs/DESIGN-report-view.md`,
//! "Where the bytes go"):
//!
//! - **Part.** The top-level part a byte is in: the root's field it is under,
//!   or the root itself when the root is a list or a single value. A view
//!   that draws parts at a finer grain sums the rows under each of its own.
//! - **Group.** The variant of the nearest ancestor that is an element of a
//!   list: the case its switch took. MIDI events group as the kinds of event,
//!   SQLite pages as leaf and interior pages. Where the switch fell to its
//!   default, the element's own name says more than "bytes" does, so a PNG's
//!   `IDAT` chunks are `IDAT` and not the raw bytes every unknown chunk is.
//! - **Role.** `content`; `machinery`, which is every field another field's
//!   length, count, type or place depends on, and everything inside one;
//!   `padding`, a run sized to an alignment; `framing`, a JSON value's own
//!   braces and commas; and `gap`, bits no field covers.
//!
//! Gaps and padding are split into zero bytes and the rest, which takes
//! reading them. That is done after the walk, a step at a time, because the
//! walk must not read anything once it has started writing a node down (see
//! `watch.rs`).
//!
//! **Every byte once.** A field placed by an offset covers bits that the
//! structure around it may have left as a gap: an HDF5 root is ninety-six
//! bytes and every object after it is placed by an address, and a SQLite
//! page's cells are placed inside the stretch its header leaves over. The
//! kind totals live with that and hold the gap total down afterwards. The
//! ledger cannot, because it says which bytes are the gap, so every gap has
//! the fields placed over it taken out of it, except fields it is inside of:
//! a gap in a placed field's own structure is a gap that field left.

use std::collections::BTreeMap;

use super::*;

/// One line of the ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerRow {
    /// The top-level part, as a path from the walk's root.
    pub part: Vec<usize>,
    pub part_name: String,
    /// The variant these bytes are grouped under. Empty when no list element
    /// is above them, where the part is the only grouping there is.
    pub group: String,
    /// Where the group's name came from: `key` (the value that picked the
    /// case, read as a name: an enum's name, a chunk's four letters), `case`
    /// (the type of the case a switch took), `name` (the case is not a record,
    /// so the element's own name, as the listing labels it: an ELF section's
    /// `.rodata`), `type` (a list whose elements are all one type), `none`,
    /// and `other` for every group past the first [`GROUP_CAP`].
    pub group_from: &'static str,
    /// `content`, `machinery`, `padding`, `framing` or `gap`.
    pub role: &'static str,
    pub bits: u64,
    /// How many fields, or for a gap how many separate stretches.
    pub count: u64,
    /// For `gap` and `padding`: how many of the bits are in bytes that are
    /// zero, and how many were not looked at (see `Ledger::done`). The rest
    /// are in bytes that hold something.
    pub zero_bits: u64,
    pub unscanned_bits: u64,
    /// The first field or stretch counted here, to go to.
    pub first_path: Vec<usize>,
    pub first_offset_bits: u64,
    /// For `padding`: the alignment in bytes. 0 for everything else.
    pub align: u32,
}

/// The whole ledger so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ledger {
    /// Largest first.
    pub rows: Vec<LedgerRow>,
    /// The space's length, which the rows are shares of.
    pub file_bits: u64,
    /// Bits the rows add up to. More than `file_bits` only where the template
    /// reads one stretch twice in a way nothing marks as a second reading.
    pub counted_bits: u64,
    /// True once every byte is in a row and every gap has been read.
    pub done: bool,
}

/// A row's key: part, group, role, alignment.
type RowKey = (u32, u32, &'static str, u32);

#[derive(Debug, Clone, Default)]
struct RowCount {
    bits: u64,
    count: u64,
    zero_bits: u64,
    unscanned_bits: u64,
    first_path: Vec<usize>,
    first_offset: u64,
}

/// A stretch whose bytes are still to be read to split it into zero and not,
/// once the walk is over.
#[derive(Debug, Clone)]
struct Stretch {
    from: u64,
    to: u64,
    scale: u64,
    row: RowKey,
    /// When the frame the stretch was found in was opened, in the walk's
    /// order: what tells a field placed over the stretch from one the stretch
    /// is inside. See `Placed`. None for padding, which is a field and never
    /// has anything placed over it.
    frame: Option<u64>,
    path: Vec<usize>,
}

/// A field that an offset put somewhere, and when the walk opened and closed
/// it: a gap is inside it when the gap's frame opened between the two.
#[derive(Debug, Clone, Copy)]
struct Placed {
    from: u64,
    to: u64,
    open: u64,
    close: u64,
}

/// What the ledger has found, kept between goes.
#[derive(Debug, Default)]
pub(super) struct Tally {
    rows: BTreeMap<RowKey, RowCount>,
    parts: Vec<(Vec<usize>, String)>,
    groups: Vec<(String, &'static str)>,
    group_index: rustc_hash::FxHashMap<(String, &'static str), u32>,
    stretches: Vec<Stretch>,
    placed: Vec<Placed>,
    /// Set once the gaps have had what was placed over them taken out.
    settled: bool,
    /// How far the scan of `stretches` has got: which one, and how far in.
    scan_at: (usize, u64),
    /// Bits read so far for the zero split, against `SCAN_CAP`.
    scanned: u64,
}

/// How much gap and padding the ledger reads to split it into zero and
/// nonzero bytes. A template that describes a header and nothing else leaves
/// the whole file a gap, and reading a few gigabytes to say how much of it is
/// zero is a question for the overview's byte classes, which read in the
/// background already. Past this the bits are counted as not looked at.
const SCAN_CAP: u64 = 256 * 1024 * 1024 * 8;

/// How many groups the ledger keeps apart. A variant is a name the file
/// chose, and a file that names every record differently would make a ledger
/// as long as the file.
pub const GROUP_CAP: usize = 256;

/// How much one step of the scan reads at a time.
const SCAN_CHUNK: u64 = 64 * 1024 * 8;

impl Tally {
    /// The part a path names, added the first time it is met.
    pub(super) fn part(&mut self, path: &[usize], name: &str) -> u32 {
        if let Some(i) = self.parts.iter().position(|(p, _)| p == path) {
            return i as u32;
        }
        self.parts.push((path.to_vec(), name.to_string()));
        (self.parts.len() - 1) as u32
    }

    /// The group a variant name names. Group 0 is none. Past [`GROUP_CAP`]
    /// groups, every new name goes in one group called `other`.
    pub(super) fn group(&mut self, name: &str, from: &'static str) -> u32 {
        if self.groups.is_empty() {
            self.groups.push((String::new(), "none"));
        }
        if let Some(i) = self.group_index.get(&(name.to_string(), from)) {
            return *i;
        }
        let (name, from) = if self.groups.len() >= GROUP_CAP { (String::new(), "other") } else { (name.to_string(), from) };
        if let Some(i) = self.group_index.get(&(name.clone(), from)) {
            return *i;
        }
        self.groups.push((name.clone(), from));
        let i = (self.groups.len() - 1) as u32;
        self.group_index.insert((name, from), i);
        i
    }

    /// A field covering `bits` bits, counted `count` times over.
    pub(super) fn field(&mut self, part: u32, group: u32, role: &'static str, align: u32, bits: u64, count: u64, path: &[usize], offset: u64) {
        let e = self.rows.entry((part, group, role, align)).or_default();
        if e.count == 0 {
            e.first_path = path.to_vec();
            e.first_offset = offset;
        }
        e.bits = e.bits.saturating_add(bits);
        e.count = e.count.saturating_add(count);
    }

    /// Padding from `from` to `to`, to be read for its zero bytes later.
    pub(super) fn padding(&mut self, part: u32, group: u32, align: u32, from: u64, to: u64, scale: u64, path: &[usize]) {
        let bits = (to - from).saturating_mul(scale);
        self.field(part, group, "padding", align, bits, scale, path, from);
        if to > from {
            self.stretches.push(Stretch { from, to, scale, row: (part, group, "padding", align), frame: None, path: path.to_vec() });
        }
    }

    /// Bits from `from` to `to` that no field of the frame opened at `frame`
    /// covers. Not counted yet: what was placed over it comes out first.
    pub(super) fn gap(&mut self, part: u32, group: u32, from: u64, to: u64, scale: u64, frame: u64, path: &[usize]) {
        if to > from {
            self.stretches.push(Stretch { from, to, scale, row: (part, group, "gap", 0), frame: Some(frame), path: path.to_vec() });
        }
    }

    /// A field an offset placed, from `from` to `to`, opened at `open`. Its
    /// `close` is filled in when it closes; see [`Tally::closed`].
    pub(super) fn placed(&mut self, from: u64, to: u64, open: u64) -> usize {
        self.placed.push(Placed { from, to, open, close: open });
        self.placed.len() - 1
    }

    pub(super) fn closed(&mut self, placed: usize, close: u64) {
        if let Some(p) = self.placed.get_mut(placed) {
            p.close = close;
        }
    }

    /// Take out of every gap the fields placed over it, and count what is
    /// left. Once, when the walk is over.
    fn settle(&mut self) {
        if self.settled {
            return;
        }
        self.settled = true;
        let mut placed = std::mem::take(&mut self.placed);
        placed.sort_by_key(|p| (p.from, p.to));
        let stretches = std::mem::take(&mut self.stretches);
        let mut kept = Vec::with_capacity(stretches.len());
        for s in stretches {
            let Some(frame) = s.frame else {
                kept.push(s);
                continue;
            };
            // What was placed over this gap: everything that starts inside
            // it, and a few before it that may reach into it. A field whose
            // own structure the gap is in is not taken out, since the gap is
            // bytes that field left.
            let lo = placed.partition_point(|p| p.from < s.from);
            let hi = placed.partition_point(|p| p.from < s.to);
            let mut over: Vec<(u64, u64)> = placed[lo.saturating_sub(64)..hi]
                .iter()
                .filter(|p| p.to > s.from && p.from < s.to && !(p.open <= frame && frame <= p.close))
                .map(|p| (p.from.max(s.from), p.to.min(s.to)))
                .collect();
            over.sort_unstable();
            let mut at = s.from;
            let mut pieces = Vec::new();
            for (a, b) in over {
                if a > at {
                    pieces.push((at, a));
                }
                at = at.max(b);
            }
            if at < s.to {
                pieces.push((at, s.to));
            }
            for (from, to) in pieces {
                let e = self.rows.entry(s.row).or_default();
                if e.count == 0 {
                    e.first_path = s.path.clone();
                    e.first_offset = from;
                }
                e.bits = e.bits.saturating_add((to - from).saturating_mul(s.scale));
                e.count = e.count.saturating_add(s.scale);
                kept.push(Stretch { from, to, ..s.clone() });
            }
        }
        // Everything starts out not looked at, and comes off as it is read.
        for s in &kept {
            if let Some(e) = self.rows.get_mut(&s.row) {
                e.unscanned_bits = e.unscanned_bits.saturating_add((s.to - s.from).saturating_mul(s.scale));
            }
        }
        self.stretches = kept;
    }

    /// Whether every gap has been read, or given up on.
    pub(super) fn scanned(&self) -> bool {
        self.settled && self.scan_at.0 >= self.stretches.len()
    }

    /// Read the gaps and padding for zero bytes, a chunk at a time, charging
    /// each chunk against the allowance. Only in the walk's own space.
    pub(super) fn scan<S: Source>(&mut self, ev: &mut Evaluator, doc: &Document<S>, space: u32) -> R<()> {
        self.settle();
        while let Some(s) = self.stretches.get(self.scan_at.0) {
            let s = s.clone();
            let at = s.from.max(self.scan_at.1);
            if at >= s.to || self.scanned >= SCAN_CAP {
                self.scan_at = (self.scan_at.0 + 1, 0);
                continue;
            }
            // Whole bytes only: a gap that starts or ends inside a byte has its
            // odd bits left as not looked at.
            let from = at.div_ceil(8) * 8;
            let to = (s.to / 8 * 8).min(from + SCAN_CHUNK);
            if to <= from {
                self.scan_at = (self.scan_at.0 + 1, 0);
                continue;
            }
            ev.spend(from)?;
            let bytes = ev.read_in(doc, space, from, to - from)?;
            let zeros = bytes.iter().filter(|b| **b == 0).count() as u64 * 8;
            let read = to - from;
            if let Some(e) = self.rows.get_mut(&s.row) {
                e.zero_bits = e.zero_bits.saturating_add(zeros.saturating_mul(s.scale));
                e.unscanned_bits = e.unscanned_bits.saturating_sub(read.saturating_mul(s.scale));
            }
            self.scanned = self.scanned.saturating_add(read);
            self.scan_at = if to >= s.to / 8 * 8 { (self.scan_at.0 + 1, 0) } else { (self.scan_at.0, to) };
        }
        Ok(())
    }

    pub(super) fn ledger(&self, file_bits: u64, done: bool) -> Ledger {
        let mut rows: Vec<LedgerRow> = self
            .rows
            .iter()
            // A field of no bits is a field and not a stretch of the file:
            // nothing to put in a ledger of where the bits go.
            .filter(|(_, c)| c.bits > 0)
            .map(|(&(part, group, role, align), c)| {
                let (part_path, part_name) = self.parts.get(part as usize).cloned().unwrap_or_default();
                let (group, group_from) = self.groups.get(group as usize).cloned().unwrap_or((String::new(), "none"));
                LedgerRow {
                    part: part_path,
                    part_name,
                    group,
                    group_from,
                    role,
                    bits: c.bits,
                    count: c.count,
                    zero_bits: c.zero_bits,
                    unscanned_bits: c.unscanned_bits,
                    first_path: c.first_path.clone(),
                    first_offset_bits: c.first_offset,
                    align,
                }
            })
            .collect();
        rows.sort_by(|a, b| b.bits.cmp(&a.bits).then_with(|| (&a.part, &a.group, a.role).cmp(&(&b.part, &b.group, b.role))));
        let counted_bits = rows.iter().map(|r| r.bits).fold(0u64, u64::saturating_add);
        Ledger { rows, file_bits, counted_bits, done }
    }
}

/// A list element's name with its index taken off: `[3] IDAT` is `IDAT`.
/// Nothing when the name is only the index.
pub(super) fn without_index(label: &str) -> Option<&str> {
    let rest = match label.strip_prefix('[') {
        Some(r) => match r.find(']') {
            Some(i) if r[..i].bytes().all(|b| b.is_ascii_digit()) => r[i + 1..].trim_start(),
            _ => label,
        },
        None => label,
    };
    (!rest.is_empty()).then_some(rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_loses_its_index() {
        assert_eq!(without_index("[3] IDAT"), Some("IDAT"));
        assert_eq!(without_index("[12]"), None);
        assert_eq!(without_index(".text"), Some(".text"));
        assert_eq!(without_index("[a] b"), Some("[a] b"));
    }

    #[test]
    fn a_gap_loses_what_was_placed_over_it_but_not_what_it_is_inside() {
        let mut t = Tally::default();
        let part = t.part(&[], "root");
        let g = t.group("", "none");
        // The root's frame, opened at 1, leaves 0..800 as a gap; a field placed
        // at 100..300, opened at 5, covers some of it; a frame inside that
        // field, opened at 6, leaves 200..250 as a gap of its own.
        t.gap(part, g, 0, 800, 1, 1, &[]);
        let p = t.placed(100, 300, 5);
        t.gap(part, g, 200, 250, 1, 6, &[0]);
        t.closed(p, 9);
        t.settle();
        let l = t.ledger(800, true);
        let gap = l.rows.iter().find(|r| r.role == "gap").expect("a gap row");
        assert_eq!(gap.bits, 600 + 50, "0..100 and 300..800 of the root's, and the field's own 200..250");
        assert_eq!(gap.count, 3);
    }
}
