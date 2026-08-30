//! The fast path: a dump regular enough that finding a line is arithmetic.
//!
//! Almost every dump anyone will open is a machine's output, unedited: the same
//! layout on every line, every line the same length, every address one line's
//! worth past the one above. Nothing about such a file needs to be remembered
//! line by line. Where line `n` starts is `at + n * stride`, which line an
//! address is on is a division, and the line itself is read when it is asked
//! for and not before.
//!
//! So this is a verifier rather than a second parser. It walks the lines once,
//! checks that they are as regular as they look, and keeps the answer as a
//! handful of [`Run`]s. What it does not do is decide how to read a line: that
//! is [`parse_row`](super::parse_row), the same one the slow path uses, so
//! there is one place where the format is understood and the two paths cannot
//! drift apart.
//!
//! When the check fails, everything falls to the slow path, which reads a line
//! at a time and copes: a shell prompt between two dumps, a column heading, a
//! line wrapped by a mail client, a screen of box drawing, a `*` standing for
//! a run of identical lines. That is the same division a browser makes between
//! a parser for well-formed markup and a parser for what people actually
//! write, and for the same reason: the strict one is fast because it is allowed
//! to refuse.
//!
//! What it refuses, exactly: a byte-order mark, an escape sequence, any byte
//! outside ASCII, a `*`, an address that does not follow the one above it,
//! lines of differing length in the middle of the dump, and more than a few
//! lines of heading or footing around it.

use super::layout::{tokens, Layout};

/// A stretch of lines that are all the same length and whose addresses step
/// evenly, so where any of them is can be worked out rather than looked up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Run {
    /// Where the first line of the run starts, in the file.
    pub at: u64,
    /// Bytes one line takes, its ending included.
    pub stride: u64,
    pub lines: u64,
    /// The address of the first byte the run describes.
    pub address: u64,
    /// Bytes on every line but the last.
    pub per_line: usize,
    /// Bytes on the last line, which is the only one allowed to be short.
    pub tail: usize,
}

impl Run {
    /// How many bytes of the described file this run covers.
    pub fn byte_count(&self) -> u64 {
        (self.lines - 1) * self.per_line as u64 + self.tail as u64
    }

    pub fn end(&self) -> u64 {
        self.address + self.byte_count()
    }

    /// Which line of the run an address is on, and how far into it, when the
    /// run covers that address at all.
    pub fn locate(&self, address: u64) -> Option<(u64, usize)> {
        if address < self.address || address >= self.end() {
            return None;
        }
        let off = address - self.address;
        Some((off / self.per_line as u64, (off % self.per_line as u64) as usize))
    }

    /// Where line `n` of the run sits in the file, as a byte range. The last
    /// line has no ending to speak of, so the range may run past the file and
    /// is clamped by the caller.
    pub fn line_at(&self, n: u64) -> (u64, u64) {
        (self.at + n * self.stride, self.stride)
    }
}

/// A dump found to be regular, and the few lines around it that are not part
/// of it: a heading, a length on its own, a prompt above the command.
#[derive(Debug, Clone)]
pub struct Regular {
    pub runs: Vec<Run>,
    pub skipped: Vec<u64>,
}

/// Lines of heading or footing tolerated around the dump. `certutil` writes
/// two, `Format-Hex` four; a file needing more than this is one to read a line
/// at a time.
const EDGE: usize = 16;

/// Check whether `bytes` is a dump regular enough to be read by arithmetic,
/// and describe it if it is.
///
/// `base` is where `bytes` starts in the file. `mark` is the byte-order mark's
/// length, and any mark at all is a refusal: a mark means an encoding whose
/// characters are not its bytes, and every offset here is a byte.
pub fn verify(bytes: &[u8], base: u64, mark: usize, layout: &Layout) -> Option<Regular> {
    if mark > 0 || bytes.iter().any(|b| *b >= 0x80 || *b == 0x1b) {
        return None;
    }
    let spans = line_spans(bytes);
    if spans.is_empty() {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?;

    // Where the dump starts and stops. A heading is a line no hypothesis can
    // use, and there are only ever a few of them.
    let first = spans.iter().position(|s| probe(&text[s.0..s.1], layout).is_some())?;
    let last = spans.iter().rposition(|s| probe(&text[s.0..s.1], layout).is_some())?;
    if first > EDGE || spans.len() - 1 - last > EDGE {
        return None;
    }

    let mut runs: Vec<Run> = Vec::new();
    let mut open: Option<(Run, u64)> = None; // the run being built, and the address after it
    for i in first..=last {
        let (from, to) = spans[i];
        let (address, count) = probe(&text[from..to], layout)?;
        let stride = stride_of(&spans, i, bytes.len());
        match open.as_mut() {
            // The run goes on while the lines are the same length, the address
            // follows the one above, and every line but the last is full.
            Some((run, next)) if run.stride == stride && address == *next && run.tail == run.per_line => {
                run.lines += 1;
                run.tail = count;
                *next = address + count as u64;
            }
            _ => {
                if let Some((run, _)) = open.take() {
                    runs.push(run);
                }
                let per_line = count.max(1);
                open = Some((
                    Run { at: base + from as u64, stride, lines: 1, address, per_line, tail: count },
                    address + count as u64,
                ));
            }
        }
    }
    if let Some((run, _)) = open {
        runs.push(run);
    }
    if runs.is_empty() {
        return None;
    }

    let skipped = spans
        .iter()
        .enumerate()
        .filter(|(i, _)| *i < first || *i > last)
        .map(|(_, s)| base + s.0 as u64)
        .collect();
    Some(Regular { runs, skipped })
}

/// Where each line's text starts and stops, the ending left off.
fn line_spans(bytes: &[u8]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'\n' {
            let mut end = i;
            if end > start && bytes[end - 1] == b'\r' {
                end -= 1;
            }
            out.push((start, end));
            start = i + 1;
        }
    }
    if start < bytes.len() {
        out.push((start, bytes.len()));
    }
    out
}

/// Bytes from the front of one line to the front of the next, which is what
/// makes a line's place arithmetic. The last line has no next line, so it is
/// measured to the end of what was read.
fn stride_of(spans: &[(usize, usize)], i: usize, len: usize) -> u64 {
    let next = spans.get(i + 1).map_or(len, |s| s.0);
    (next - spans[i].0) as u64
}

/// A line's address and how many bytes it holds, without building anything
/// that outlives the question. This is the check, and it is the reason the
/// strict path can be trusted: every line is looked at, and nothing is kept.
fn probe(line: &str, layout: &Layout) -> Option<(u64, usize)> {
    if line.trim() == "*" {
        return None;
    }
    let toks = tokens(line);
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
        None => return None,
    };
    let mut count = 0;
    for t in &toks[skip..] {
        if count >= layout.bytes_per_line {
            break;
        }
        if t.s.is_empty() || t.s.len() % 2 != 0 || !t.s.bytes().all(|b| b.is_ascii_hexdigit()) {
            break;
        }
        if layout.text_at.is_some_and(|c| t.at >= c) {
            break;
        }
        count += t.s.len() / 2;
    }
    (count > 0).then_some((address, count))
}

#[cfg(test)]
mod tests {
    use super::super::{read, read_irregular, Index, Tier};

    /// A dump written by xxd, with a ragged last line. Two runs: the full
    /// lines, and the short one, which is a different length and so cannot be
    /// found by the same arithmetic.
    const PLAIN: &str = "\
00000000: 0001 0203 0405 0607 0809 0a0b 0c0d 0e0f  ................
00000010: 1011 1213 1415 1617 1819 1a1b 1c1d 1e1f  ................
00000020: 2021 2223 2425 2627 2829 2a2b 2c2d 2e2f   !\"#$%&'()*+,-./
00000030: 3031 3233                                0123
";

    #[test]
    fn a_machines_output_is_read_by_arithmetic() {
        let d = read(PLAIN.as_bytes(), 0).unwrap();
        assert_eq!(d.tier(), Tier::Regular);
        let Index::Runs(runs) = &d.index else { panic!("not runs") };
        assert_eq!(runs.len(), 2, "the full lines, then the ragged one");
        assert_eq!(runs[0].lines, 3);
        assert_eq!(runs[0].per_line, 16);
        assert_eq!(runs[1].tail, 4);
        assert_eq!(d.byte_count(), 52);
    }

    #[test]
    fn a_line_is_read_when_it_is_asked_for() {
        let d = read(PLAIN.as_bytes(), 0).unwrap();
        let mut got = [0u8; 4];
        assert_eq!(d.read_at(0x1e, &mut got), 4);
        assert_eq!(got, [0x1e, 0x1f, 0x20, 0x21], "a read across two lines");
        assert_eq!(d.rows(0x10, 0x11).len(), 1, "one line asked for, one line read");
    }

    #[test]
    fn a_squeezed_run_is_not_regular() {
        let text = PLAIN.replace("00000020:", "*\n00000020:");
        assert_eq!(read(text.as_bytes(), 0).unwrap().tier(), Tier::Irregular);
    }

    #[test]
    fn a_line_that_is_not_a_dump_line_is_not_regular() {
        let text = PLAIN.replace("00000020:", "$ echo hello\nhello\n00000020:");
        assert_eq!(read(text.as_bytes(), 0).unwrap().tier(), Tier::Irregular);
    }

    #[test]
    fn colour_is_not_regular_but_still_reads() {
        let text = PLAIN.replace("0001", "\u{1b}[1;31m0001\u{1b}[0m");
        let d = read(text.as_bytes(), 0).unwrap();
        assert_eq!(d.tier(), Tier::Irregular);
        assert_eq!(d.byte_count(), 52);
    }

    #[test]
    fn a_heading_above_the_dump_is_stepped_over() {
        let text = format!("   Label: C:\\x.bin\n\n{PLAIN}");
        let d = read(text.as_bytes(), 0).unwrap();
        assert_eq!(d.tier(), Tier::Regular);
        assert_eq!(d.byte_count(), 52);
    }

    #[test]
    fn the_index_does_not_grow_with_the_dump() {
        // Ten thousand lines is one run, because that is all a run is.
        let mut text = String::new();
        for i in 0..10_000u64 {
            text.push_str(&format!("{:08x}: ", i * 16));
            for g in 0..8 {
                text.push_str(&format!("{:04x} ", (i as u16).wrapping_add(g)));
            }
            text.push_str(" ................\n");
        }
        let d = read(text.as_bytes(), 0).unwrap();
        assert_eq!(d.tier(), Tier::Regular);
        let Index::Runs(runs) = &d.index else { panic!("not runs") };
        assert_eq!(runs.len(), 1);
        assert_eq!(d.byte_count(), 160_000);
        assert_eq!(read_irregular(text.as_bytes(), 0).unwrap().byte_count(), 160_000);
    }
}
