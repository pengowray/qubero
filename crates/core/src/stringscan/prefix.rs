//! The number in front of a string.
//!
//! A Pascal, Java, .NET or protobuf string is counted rather than terminated.
//! Every reading of the bytes before a run is checked against the run, and the
//! arithmetic is reported rather than scored.

use super::*;
use super::runs::*;

/// The most readings of one prefix that are reported. Several are usually the
/// same number written at different widths, and the fourth adds nothing.
pub const MAX_PREFIX_READINGS: usize = 4;
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
pub(super) const PREFIX_ORDER: [PrefixKind; 8] = [
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
/// Which kinds of number in front of each run read as its length, with how
/// many bytes each takes and what it came to.
pub(super) fn counted_by(buf: &[u8], base: u64, runs: &[Run]) -> Vec<Vec<(PrefixKind, usize, u64)>> {
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
pub(super) fn prefix_candidates(buf: &[u8], at: usize, kind: PrefixKind) -> Vec<(usize, u64)> {
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
pub(super) fn read_prefix(buf: &[u8], at: usize, kind: PrefixKind) -> Option<(usize, u64)> {
    prefix_candidates(buf, at, kind).into_iter().next()
}
/// Every reading of the bytes in front of a run that comes to its length.
///
/// A run's first byte is not text, so a number in the byte before it is not
/// text either, and one that comes to the run's length is worth putting in
/// front of the reader. Which of the readings was meant is not decided here:
/// `00 00 00 05` is a 32-bit five, a 16-bit five and an eight-bit five, all
/// three true, and the bytes are carried so the reader can see that.
pub(super) fn prefix_readings(
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
pub(super) fn span_of(buf: &[u8], at: usize, limit: usize, enc: Enc, n: u64, counts: Counts) -> Option<usize> {
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
pub(super) fn prefix_width(buf: &[u8], pos: usize, limit: usize, kind: PrefixKind) -> Option<usize> {
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
pub(super) fn first_prefix(buf: &[u8], run: Run, kind: PrefixKind) -> Vec<usize> {
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
pub(super) fn chain(buf: &[u8], base: u64, run: Run, opts: Opts) -> Vec<(usize, usize, Prefix)> {
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
