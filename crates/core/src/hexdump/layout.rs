//! Working out how a dump is laid out, from the dump.
//!
//! Nothing in a hex dump says how to read it. `xxd` writes an eight-digit hex
//! address, a colon, eight groups of two bytes and a text column; `od` writes a
//! six-digit hex address, no colon, sixteen groups of one byte and a text
//! column inside angle brackets; `certutil` indents, splits the hex in half
//! with a wider gap and writes no text column at all; and each of them will
//! write something else entirely if it is asked to. A reader that knows one of
//! them knows one of them.
//!
//! So the layout is read off the lines rather than looked up by name. The one
//! thing a dump cannot lie about is arithmetic: the address of a line plus the
//! bytes on it is the address of the next line. That is the whole of the
//! method. Every hypothesis about which token is the address and what base it
//! is written in is checked against that sum, and the one that survives is the
//! answer. A dump with no address column survives nothing and says so.
//!
//! The names (`xxd`, `od -Ax`, `Format-Hex`) are attached afterwards, to a
//! layout that was already settled. They are a label on the answer and never a
//! route to it, so a tool nobody here has heard of reads the same as one that
//! ships with every system.

use crate::text::Settled;

/// What base a line's address is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Base {
    Hex,
    Octal,
    Decimal,
}

impl Base {
    pub fn radix(self) -> u32 {
        match self {
            Base::Hex => 16,
            Base::Octal => 8,
            Base::Decimal => 10,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Base::Hex => "hexadecimal",
            Base::Octal => "octal",
            Base::Decimal => "decimal",
        }
    }
}

/// The column of addresses down the left.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Address {
    pub base: Base,
    /// Digits every address is padded to, where they all have the same width.
    pub digits: Option<usize>,
    /// A character closing the address, which is xxd's colon.
    pub suffix: Option<char>,
    /// Whether its digits are upper case, which is a separate question from
    /// the digits of the bytes: `xxd -u` writes the bytes in upper case and
    /// the address in lower.
    pub upper: bool,
}

/// The column of characters down the right.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextColumn {
    /// Characters around the column, which are od's angle brackets.
    pub open: Option<char>,
    pub close: Option<char>,
    /// Which encoding the characters were written in.
    pub encoding: Settled,
    /// What a byte with no character of its own was written as, which is
    /// What the tool wrote for a byte that has no character of its own. It is
    /// almost always a full stop, and it is the reason the column can confirm
    /// a byte but rarely contradict one. There can be more than one:
    /// `Format-Hex` writes a space for a zero and a replacement character for
    /// everything else it will not print.
    pub placeholders: Vec<char>,
}

/// Which end of a group the first byte of it is at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    /// The digits read in file order, which is what a hex dump means.
    Forward,
    /// Each group is written as a little-endian number, so the bytes inside it
    /// read backwards. `xxd -e` does this and nothing on the line says so
    /// except the text column, which does not.
    ReversedInGroup,
}

/// Something the dump did not settle, which was taken as the usual thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Assumed {
    /// No two addresses to subtract, so the base is whatever it looks like.
    Base,
    /// No address column at all, so the bytes are placed in the order they
    /// were written and the first of them is byte zero.
    Start,
    /// The text column never disagreed with the hex under either order, so
    /// the groups are taken to read forwards.
    Order,
    /// No text column, or one that agreed with more than one encoding.
    TextEncoding,
}

/// How a dump is laid out.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub address: Option<Address>,
    pub bytes_per_line: usize,
    /// Bytes written without a space between them.
    pub group: usize,
    pub upper: bool,
    pub order: Order,
    pub text: Option<TextColumn>,
    /// Spaces before the address, which `certutil` indents by.
    pub indent: usize,
    /// Where the digits start, counted in characters from the front of the
    /// line. A short last line keeps its columns by being padded to it.
    pub hex_at: usize,
    /// Whether an extra space falls halfway along the digits, which is how
    /// `certutil` and `hexdump -C` split a line into two halves.
    pub half_gap: bool,
    /// Where the character column starts, counted in characters from the front
    /// of the line. It cannot be found by splitting on spaces, because a space
    /// is a byte and one at the front of the column would be eaten with the
    /// separator.
    pub text_at: Option<usize>,
    /// The character the dump collapsed a run of identical lines to, where it
    /// did. `*` is what every tool that does this writes, but a dump with no
    /// such run in it is not one that would have written the marker, and a
    /// dump written from this layout should not invent one.
    pub squeeze: Option<char>,
    /// Whether the dump ends with the address after its last byte on a line of
    /// its own, which is how `od` says how long the file was.
    pub end_address: bool,
    pub assumed: Vec<Assumed>,
}

impl Layout {
    /// The name of the tool this layout is what comes out of, where it is one
    /// that is recognised. A label on an answer already reached: nothing is
    /// read differently because of it.
    pub fn looks_like(&self) -> Option<&'static str> {
        let a = self.address.as_ref()?;
        let t = self.text.as_ref();
        match (a.base, a.digits, a.suffix, self.group, self.bytes_per_line, t.map(|t| t.open)) {
            (Base::Hex, Some(8), Some(':'), 2, 16, Some(None)) => Some("xxd"),
            (Base::Hex, Some(8), Some(':'), 1, 16, Some(None)) => Some("xxd -g1"),
            (Base::Hex, Some(6), None, 1, 16, Some(Some('>'))) => Some("od -Ax -tx1z"),
            (Base::Octal, Some(7), None, 1, 16, None) => Some("od -tx1"),
            (Base::Decimal, Some(7), None, 1, 16, None) => Some("od -Ad -tx1"),
            (Base::Hex, Some(16), None, 1, 16, Some(None)) => Some("Format-Hex"),
            (Base::Hex, Some(4), None, 1, 16, Some(None)) => Some("certutil -dump"),
            _ => None,
        }
    }
}

/// One token of a line, with where in the line it starts.
#[derive(Debug, Clone, Copy)]
pub struct Tok<'a> {
    pub at: usize,
    pub s: &'a str,
}

/// Split a line on white space, keeping where each piece started.
pub fn tokens(line: &str) -> Vec<Tok<'_>> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in line.char_indices().chain(std::iter::once((line.len(), ' '))) {
        match (c.is_whitespace(), start) {
            (false, None) => start = Some(i),
            (true, Some(s)) => {
                out.push(Tok { at: char_count(&line[..s]), s: &line[s..i] });
                start = None;
            }
            _ => {}
        }
    }
    out
}

fn char_count(s: &str) -> usize {
    s.chars().count()
}

/// Whether a token is a run of hex digits that could be whole bytes.
fn is_hex_group(s: &str) -> bool {
    !s.is_empty() && s.len() % 2 == 0 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// How many hex-group tokens start the list, and how many bytes they hold.
fn hex_prefix(toks: &[Tok<'_>]) -> (usize, usize) {
    let mut count = 0;
    let mut bytes = 0;
    for t in toks {
        if !is_hex_group(t.s) {
            break;
        }
        count += 1;
        bytes += t.s.len() / 2;
    }
    (count, bytes)
}

/// A line's address under one hypothesis, and how many bytes could follow it.
struct Candidate {
    address: Option<u64>,
    /// Bytes in the run of hex tokens after the address.
    bytes: usize,
    /// Tokens the address took, which is zero or one.
    skipped: usize,
}

fn candidate(toks: &[Tok<'_>], base: Option<Base>) -> Option<Candidate> {
    match base {
        None => {
            let (_, bytes) = hex_prefix(toks);
            (bytes > 0).then_some(Candidate { address: None, bytes, skipped: 0 })
        }
        Some(b) => {
            let first = toks.first()?;
            let digits = first.s.strip_suffix(':').unwrap_or(first.s);
            if digits.is_empty() || !digits.bytes().all(|c| (c as char).is_digit(b.radix())) {
                return None;
            }
            let address = u64::from_str_radix(digits, b.radix()).ok()?;
            let (_, bytes) = hex_prefix(&toks[1..]);
            Some(Candidate { address: Some(address), bytes, skipped: 1 })
        }
    }
}

/// How a hypothesis fared.
struct Fit {
    /// Lines whose address plus their bytes is the address of the next line.
    agreed: usize,
    /// Lines where what is left over after the bytes is itself hex groups,
    /// which means the hypothesis took digits for characters.
    hexy: usize,
    /// Bytes a full line holds.
    width: usize,
}

/// The addresses a hypothesis gives, judged against the bytes between them.
///
/// A hypothesis is right when the address of one line plus the bytes on it is
/// the address of the next. Nothing else about a dump is checkable from the
/// dump, and this is checkable on every line but the last.
///
/// One thing more has to be looked at, because arithmetic alone can be fooled.
/// A dump with no address column whose bytes happen to climb evenly reads as
/// one that has an address column with fewer bytes on the line: `00 01 .. 0f`
/// followed by `10 11 .. 1f` gives a first token that steps by eight in octal.
/// What gives it away is what is left over. A hypothesis that takes half a line
/// of digits and calls the other half a character column has left hex where
/// characters should be, and one that does that on more lines than it explains
/// is the wrong hypothesis.
fn fit(lines: &[Vec<Tok<'_>>], base: Option<Base>) -> Option<Fit> {
    let cands: Vec<Option<Candidate>> = lines.iter().map(|t| candidate(t, base)).collect();
    let counts: Vec<usize> = cands.iter().flatten().map(|c| c.bytes).filter(|b| *b > 0).collect();
    let widest = counts.iter().copied().max()?;
    if base.is_none() {
        // Nothing to measure the line against, so it is as long as it looks.
        // The usual length rather than the longest, because a line whose
        // character column happens to read as hex digits holds more hex than
        // the line does.
        return Some(Fit { agreed: 0, hexy: 0, width: modal(&counts).unwrap_or(widest) });
    }

    // How far apart two lines' addresses are is how many bytes are on a line.
    // This is the one measurement the dump itself makes, and taking it from the
    // addresses rather than from the digits is what stops a character column of
    // hex digits being counted as more bytes.
    let deltas: Vec<usize> = cands
        .windows(2)
        .filter_map(|p| match (p[0].as_ref()?.address?, p[1].as_ref()?.address?) {
            (x, y) if y > x && y - x <= 256 => Some((y - x) as usize),
            _ => None,
        })
        .collect();
    let width = modal(&deltas).filter(|w| *w > 0 && *w <= widest)?;

    let agreed = cands
        .windows(2)
        .filter(|p| match (&p[0], &p[1]) {
            (Some(a), Some(b)) => matches!((a.address, b.address), (Some(x), Some(y)) if y == x + width as u64),
            _ => false,
        })
        .count();
    // A hypothesis that takes half a line of digits and calls the rest a
    // character column has left hex where characters should be. One that does
    // that on more lines than it explains is the wrong hypothesis: a dump with
    // no address column whose bytes climb evenly reads as one that has an
    // address column with fewer bytes on the line.
    let hexy = lines
        .iter()
        .zip(&cands)
        .filter(|(t, c)| {
            let Some(c) = c else { return false };
            if c.bytes <= width {
                return false;
            }
            let taken = taken_tokens(&t[c.skipped..], width);
            let rest = &t[c.skipped + taken..];
            !rest.is_empty() && rest.iter().all(|r| is_hex_group(r.s))
        })
        .count();
    (agreed > 0).then_some(Fit { agreed, hexy, width })
}

/// The commonest value, and nothing when there is a tie for it.
fn modal(v: &[usize]) -> Option<usize> {
    let mut best: Option<(usize, usize)> = None;
    let mut tied = false;
    for x in v {
        let n = v.iter().filter(|y| *y == x).count();
        match best {
            Some((_, m)) if n < m => {}
            Some((y, m)) if n == m && y != *x => tied = true,
            _ => {
                best = Some((*x, n));
                tied = false;
            }
        }
    }
    best.filter(|_| !tied).map(|(x, _)| x)
}

/// How many tokens hold the first `width` bytes.
fn taken_tokens(toks: &[Tok<'_>], width: usize) -> usize {
    let mut bytes = 0;
    for (i, t) in toks.iter().enumerate() {
        if bytes >= width || !is_hex_group(t.s) {
            return i;
        }
        bytes += t.s.len() / 2;
    }
    toks.len()
}

/// Read a layout off a sample of lines.
///
/// The sample is the front of the file, and lines that are not part of a dump
/// are simply lines no hypothesis can use: a shell prompt has no hex on it and
/// a column heading joins up with nothing.
pub fn infer(sample: &[String]) -> Option<Layout> {
    let toks: Vec<Vec<Tok<'_>>> = sample.iter().map(|l| tokens(l)).collect();
    let mut assumed = Vec::new();

    // Which hypothesis joins the most lines together. An address is preferred
    // to no address only when its sums actually come out.
    let mut best: Option<(Option<Base>, Fit)> = None;
    for base in [Some(Base::Hex), Some(Base::Octal), Some(Base::Decimal)] {
        if let Some(f) = fit(&toks, base) {
            if best.as_ref().is_none_or(|(_, b)| (f.hexy, std::cmp::Reverse(f.agreed)) < (b.hexy, std::cmp::Reverse(b.agreed))) {
                best = Some((base, f));
            }
        }
    }
    let (base, bytes_per_line) = match best {
        Some((base, f)) if f.hexy <= f.agreed => (base, f.width),
        _ => {
            assumed.push(Assumed::Start);
            assumed.push(Assumed::Base);
            (None, fit(&toks, None)?.width)
        }
    };

    // The lines this hypothesis actually reads, which is what everything else
    // is measured on.
    let rows: Vec<(&Vec<Tok<'_>>, Candidate)> =
        toks.iter().filter_map(|t| candidate(t, base).map(|c| (t, c))).filter(|(_, c)| c.bytes > 0).collect();
    if rows.is_empty() {
        return None;
    }

    let indent = rows.iter().map(|(t, _)| t[0].at).min().unwrap_or(0);
    let hex_at = rows.iter().filter_map(|(t, c)| t.get(c.skipped).map(|g| g.at)).min().unwrap_or(indent);
    let address = base.map(|b| {
        let widths: Vec<usize> = rows.iter().map(|(t, _)| t[0].s.chars().count() - usize::from(t[0].s.ends_with(':'))).collect();
        // The usual width rather than a width every line shares, because a
        // column heading above the dump has an address-shaped token on it and
        // is not a line of the dump. A line whose address is the wrong width
        // is then refused when the dump is read.
        let digits = modal(&widths).filter(|d| widths.iter().filter(|w| *w == d).count() * 5 >= widths.len() * 4);
        let suffix = rows[0].0[0].s.ends_with(':').then_some(':');
        let upper = rows.iter().any(|(t, _)| t[0].s.bytes().any(|c| c.is_ascii_uppercase()));
        Address { base: b, digits, suffix, upper }
    });

    let group = rows
        .iter()
        .filter_map(|(t, c)| t.get(c.skipped).map(|g| g.s.len() / 2))
        .find(|g| *g > 0)
        .unwrap_or(1);

    let upper = rows
        .iter()
        .flat_map(|(t, c)| t[c.skipped..].iter().take_while(|g| is_hex_group(g.s)))
        .any(|g| g.s.bytes().any(|b| b.is_ascii_uppercase()))
        ;

    // A wider space halfway along the digits, which is a line split into two
    // halves rather than a group boundary.
    let half = bytes_per_line / 2;
    let half_gap = rows.iter().filter(|(_, c)| c.bytes >= bytes_per_line).any(|(t, c)| {
        let before = taken_tokens(&t[c.skipped..], half);
        match (t.get(c.skipped + before - 1), t.get(c.skipped + before)) {
            (Some(a), Some(b)) => b.at > a.at + a.s.chars().count() + 1,
            _ => false,
        }
    });

    let (text, text_at) = text_column(&rows, bytes_per_line, group);
    if text.is_none() {
        assumed.push(Assumed::TextEncoding);
    }

    Some(Layout { address, bytes_per_line, group, upper, order: Order::Forward, indent, hex_at, half_gap, text, text_at, squeeze: None, end_address: false, assumed })
}

/// What is left on a line after the bytes, if anything, and where it starts.
///
/// The column is found by position rather than by splitting, because a space is
/// a byte: a column opening with one would lose it to the separator. On a full
/// line the digits stop at a fixed place, so the separator is the fewest spaces
/// any full line has after them, and a line whose first byte is a space simply
/// has one more.
fn text_column(rows: &[(&Vec<Tok<'_>>, Candidate)], bytes_per_line: usize, group: usize) -> (Option<TextColumn>, Option<usize>) {
    let full = bytes_per_line.div_ceil(group);
    let mut start: Option<usize> = None;
    let mut opens = Vec::new();
    for (t, c) in rows {
        if c.bytes < bytes_per_line {
            continue;
        }
        let Some(last) = t.get(c.skipped + full - 1) else { continue };
        let end = last.at + last.s.chars().count();
        let Some(rest) = t.get(c.skipped + full) else { continue };
        start = Some(start.map_or(rest.at, |s: usize| s.min(rest.at)));
        opens.push((rest.s.chars().next(), end));
    }
    let Some(start) = start else { return (None, None) };
    if opens.is_empty() {
        return (None, None);
    }
    // od wraps its column in a pair of characters; every other tool writes it
    // bare. Only a character opening every line counts as a wrapper.
    let open = opens[0].0.filter(|c| *c == '>' || *c == '|').filter(|c| opens.iter().all(|o| o.0 == Some(*c)));
    let close = open.map(|c| if c == '>' { '<' } else { '|' });
    (Some(TextColumn { open, close, encoding: Settled::Ascii, placeholders: vec!['.'] }), Some(start))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<String> {
        s.lines().map(|l| l.to_string()).collect()
    }

    #[test]
    fn xxd_reads_as_xxd() {
        let l = lines(
            "00000000: 0001 0203 0405 0607 0809 0a0b 0c0d 0e0f  ................\n\
             00000010: 1011 1213 1415 1617 1819 1a1b 1c1d 1e1f  ................\n\
             00000020: 2021 2223 2425 2627 2829 2a2b 2c2d 2e2f   !\"#$%&'()*+,-./",
        );
        let got = infer(&l).unwrap();
        assert_eq!(got.bytes_per_line, 16);
        assert_eq!(got.group, 2);
        assert_eq!(got.address.as_ref().unwrap().base, Base::Hex);
        assert_eq!(got.address.as_ref().unwrap().suffix, Some(':'));
        assert_eq!(got.looks_like(), Some("xxd"));
        assert!(got.assumed.is_empty());
    }

    #[test]
    fn an_octal_address_is_not_read_as_hex() {
        let l = lines(
            "0000000 00 01 02 03 04 05 06 07 08 09 0a 0b 0c 0d 0e 0f\n\
             0000020 10 11 12 13 14 15 16 17 18 19 1a 1b 1c 1d 1e 1f\n\
             0000040 20 21 22 23 24 25 26 27 28 29 2a 2b 2c 2d 2e 2f",
        );
        let got = infer(&l).unwrap();
        assert_eq!(got.address.as_ref().unwrap().base, Base::Octal);
        assert_eq!(got.bytes_per_line, 16);
        assert!(got.text.is_none());
    }

    #[test]
    fn a_decimal_address_is_not_read_as_octal() {
        let l = lines(
            "0000000 00 01 02 03 04 05 06 07 08 09 0a 0b 0c 0d 0e 0f\n\
             0000016 10 11 12 13 14 15 16 17 18 19 1a 1b 1c 1d 1e 1f\n\
             0000032 20 21 22 23 24 25 26 27 28 29 2a 2b 2c 2d 2e 2f",
        );
        assert_eq!(infer(&l).unwrap().address.unwrap().base, Base::Decimal);
    }

    #[test]
    fn hex_with_no_address_says_it_assumed_where_it_starts() {
        let l = lines(
            "00 01 02 03 04 05 06 07  08 09 0a 0b 0c 0d 0e 0f\n\
             10 11 12 13 14 15 16 17  18 19 1a 1b 1c 1d 1e 1f",
        );
        let got = infer(&l).unwrap();
        assert!(got.address.is_none());
        assert!(got.assumed.contains(&Assumed::Start));
        assert_eq!(got.bytes_per_line, 16);
    }

    #[test]
    fn od_keeps_its_brackets_out_of_the_text() {
        let l = lines(
            "000000 00 01 02 03 04 05 06 07 08 09 0a 0b 0c 0d 0e 0f  >................<\n\
             000010 10 11 12 13 14 15 16 17 18 19 1a 1b 1c 1d 1e 1f  >................<",
        );
        let got = infer(&l).unwrap();
        assert_eq!(got.text.as_ref().unwrap().open, Some('>'));
        assert_eq!(got.text.as_ref().unwrap().close, Some('<'));
        assert_eq!(got.looks_like(), Some("od -Ax -tx1z"));
    }

    #[test]
    fn a_prompt_is_not_a_line_of_a_dump() {
        let l = lines(
            "pengo@workstation MINGW64 ~/x\n\
             $ xxd -l 32 source-bytes.bin\n\
             00000000: 0001 0203 0405 0607 0809 0a0b 0c0d 0e0f  ................\n\
             00000010: 1011 1213 1415 1617 1819 1a1b 1c1d 1e1f  ................",
        );
        let got = infer(&l).unwrap();
        assert_eq!(got.bytes_per_line, 16);
        assert_eq!(got.looks_like(), Some("xxd"));
    }
}
