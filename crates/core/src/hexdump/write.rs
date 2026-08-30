//! Writing a file out as a hex dump: the plain text view.
//!
//! This is the other direction of [`read`](super::read), and it is here for
//! three reasons. It is what a reader wants when the dump is the deliverable:
//! a stretch of a file pasted into a bug report, a mail, a manual. It is the
//! plain text view of a file, which is the oldest view there is and the one
//! every terminal can show. And it is how the reader is tested, because a dump
//! read and written again has to come back the same text, character for
//! character, and a file dumped and read back has to come back the same bytes.
//!
//! A [`Layout`] is what to write as much as it is what was read, so a file can
//! be written out the way `xxd` writes one, or `od`, or the way the dump that
//! was opened was written. Nothing here is a special case for a tool: the
//! columns are placed from the same fields the reader settled.

use super::layout::{Base, Layout, Order};
use super::printable;
use crate::text::Settled;

/// Write `bytes` as a dump, with the first of them at `address`.
///
/// `squeeze` collapses a run of identical lines to the layout's marker, which
/// is what `xxd -a` and `od` do by default. Lines are separated by `\n`; a
/// tool writing two characters is a property of the terminal it ran in rather
/// than of the dump.
pub fn dump(layout: &Layout, bytes: &[u8], address: u64, squeeze: bool) -> String {
    let mut out = String::new();
    let per = layout.bytes_per_line.max(1);
    let mut last: Option<&[u8]> = None;
    let mut squeezing = false;
    for (i, chunk) in bytes.chunks(per).enumerate() {
        let at = address + (i * per) as u64;
        let full = chunk.len() == per;
        if squeeze && full && last == Some(chunk) && i + 1 < bytes.len().div_ceil(per) {
            if !squeezing {
                out.push(layout.squeeze.unwrap_or('*'));
                out.push('\n');
                squeezing = true;
            }
            continue;
        }
        squeezing = false;
        last = Some(chunk);
        line(&mut out, layout, chunk, at);
        out.push('\n');
    }
    let _ = squeezing;
    // The address after the last byte, on a line of its own, which is how `od`
    // says how long the file was.
    if layout.end_address {
        if let Some(a) = &layout.address {
            out.push_str(&" ".repeat(layout.indent));
            out.push_str(&address_text(a, address + bytes.len() as u64));
            out.push('\n');
        }
    }
    out
}

/// One line, columns and all.
fn line(out: &mut String, layout: &Layout, chunk: &[u8], at: u64) {
    let start = out.len();
    out.push_str(&" ".repeat(layout.indent));
    if let Some(a) = &layout.address {
        out.push_str(&address_text(a, at));
        if let Some(c) = a.suffix {
            out.push(c);
        }
    }
    pad_to(out, start, layout.hex_at);

    let group = layout.group.max(1);
    let half = layout.bytes_per_line / 2;
    let mut written = 0;
    while written < chunk.len() {
        if written > 0 {
            out.push(' ');
            if layout.half_gap && written == half {
                out.push(' ');
            }
        }
        let n = group.min(chunk.len() - written);
        let mut g: Vec<u8> = chunk[written..written + n].to_vec();
        if layout.order == Order::ReversedInGroup {
            g.reverse();
            // A group written as a little-endian number is short at the front
            // when the file runs out inside it, because the bytes it is missing
            // are the high ones.
            out.push_str(&" ".repeat((group - n) * 2));
        }
        for b in g {
            out.push_str(&hex_byte(b, layout.upper));
        }
        written += n;
    }

    let Some(t) = &layout.text else { return };
    let Some(text_at) = layout.text_at else { return };
    pad_to(out, start, text_at);
    if let Some(c) = t.open {
        out.push(c);
    }
    for b in chunk {
        out.push(match printable(t.encoding, *b) {
            Some(c) => c,
            None => stand_in(t.encoding, *b, &t.placeholders),
        });
    }
    if let Some(c) = t.close {
        out.push(c);
    }
}

/// What to write for a byte with no character of its own. A tool that uses two
/// stand-ins uses the first for a zero, which is the only such rule seen here;
/// a tool with one uses it for everything.
fn stand_in(_enc: Settled, b: u8, placeholders: &[char]) -> char {
    match placeholders {
        [] => '.',
        [one] => *one,
        [zero, rest @ ..] => {
            if b == 0 {
                *zero
            } else {
                rest[0]
            }
        }
    }
}

fn address_text(a: &super::layout::Address, at: u64) -> String {
    let digits = match a.base {
        Base::Hex if a.upper => format!("{at:X}"),
        Base::Hex => format!("{at:x}"),
        Base::Octal => format!("{at:o}"),
        Base::Decimal => format!("{at}"),
    };
    match a.digits {
        Some(w) if digits.len() < w => format!("{}{digits}", "0".repeat(w - digits.len())),
        _ => digits,
    }
}

fn hex_byte(b: u8, upper: bool) -> String {
    if upper {
        format!("{b:02X}")
    } else {
        format!("{b:02x}")
    }
}

/// Spaces out to a column, counted from the front of this line. At least one
/// space, so a line that has overrun its column still has its parts apart.
fn pad_to(out: &mut String, start: usize, col: usize) {
    let have = out[start..].chars().count();
    out.push_str(&" ".repeat(col.saturating_sub(have).max(1)));
}

#[cfg(test)]
mod tests {
    use super::super::read;
    use super::*;

    /// A dump read and written again is the same text. This is the check that
    /// the layout the reader settled on is the whole of the layout, since
    /// anything it failed to notice comes back in the wrong place.
    fn round_trip(text: &str) -> String {
        let dump = read(text.as_bytes(), 0).expect("a dump");
        let (from, _) = dump.span().expect("bytes");
        let mut bytes = vec![0u8; dump.byte_count() as usize];
        let n = dump.read_at(from, &mut bytes);
        super::dump(&dump.layout, &bytes[..n], from, false)
    }

    #[test]
    fn xxd_comes_back_as_xxd_wrote_it() {
        let text = "00000000: 0001 0203 0405 0607 0809 0a0b 0c0d 0e0f  ................\n\
                    00000010: 1011 1213 1415 1617 1819 1a1b 1c1d 1e1f  ................\n\
                    00000020: 2021 2223 2425 2627 2829 2a2b 2c2d 2e2f   !\"#$%&'()*+,-./\n";
        assert_eq!(round_trip(text), text);
    }

    #[test]
    fn od_comes_back_as_od_wrote_it() {
        let text = "000000 00 01 02 03 04 05 06 07 08 09 0a 0b 0c 0d 0e 0f  >................<\n\
                    000010 10 11 12 13 14 15 16 17 18 19 1a 1b 1c 1d 1e 1f  >................<\n";
        assert_eq!(round_trip(text), text);
    }

    #[test]
    fn a_run_of_the_same_line_collapses_and_says_where_it_stopped() {
        let layout = read(
            b"00000000: 0001 0203 0405 0607 0809 0a0b 0c0d 0e0f  ................\n\
              00000010: 1011 1213 1415 1617 1819 1a1b 1c1d 1e1f  ................\n",
            0,
        )
        .unwrap()
        .layout;
        let bytes = vec![0u8; 64];
        let text = dump(&layout, &bytes, 0, true);
        assert_eq!(text.lines().count(), 3);
        assert!(text.lines().nth(1).unwrap().starts_with('*'));
        assert!(text.lines().nth(2).unwrap().starts_with("00000030"));
    }

    #[test]
    fn what_was_written_reads_back_as_the_same_bytes() {
        let layout = read(b"00000000: 0001 0203 0405 0607 0809 0a0b 0c0d 0e0f  ................\n00000010: 1011 1213 1415 1617 1819 1a1b 1c1d 1e1f  ................\n", 0).unwrap().layout;
        let bytes: Vec<u8> = (0..=255u8).chain(0..70).collect();
        let text = dump(&layout, &bytes, 0, false);
        let back = read(text.as_bytes(), 0).unwrap();
        let mut got = vec![0u8; bytes.len()];
        assert_eq!(back.read_at(0, &mut got), bytes.len());
        assert_eq!(got, bytes);
        assert!(back.conflicts().is_empty());
    }
}
