//! Encodings for text fields.
//!
//! Hand-rolled rather than pulled in: `encoding_rs` carries the whole WHATWG
//! set (weight this never needs in a wasm bundle) and still does not have
//! CP437, which a hex editor meets constantly in DOS-era formats. What is here
//! is five encodings, a BOM sniffer and a guess, each a few lines.

use crate::template::{Encoding, Endian};

/// CP437 above 0x7F, generated from Python's codec so the table is not typed
/// from memory. Below 0x80 CP437 is ASCII.
const CP437_HIGH: [char; 128] = [
    '\u{00c7}', '\u{00fc}', '\u{00e9}', '\u{00e2}', '\u{00e4}', '\u{00e0}', '\u{00e5}', '\u{00e7}',
    '\u{00ea}', '\u{00eb}', '\u{00e8}', '\u{00ef}', '\u{00ee}', '\u{00ec}', '\u{00c4}', '\u{00c5}',
    '\u{00c9}', '\u{00e6}', '\u{00c6}', '\u{00f4}', '\u{00f6}', '\u{00f2}', '\u{00fb}', '\u{00f9}',
    '\u{00ff}', '\u{00d6}', '\u{00dc}', '\u{00a2}', '\u{00a3}', '\u{00a5}', '\u{20a7}', '\u{0192}',
    '\u{00e1}', '\u{00ed}', '\u{00f3}', '\u{00fa}', '\u{00f1}', '\u{00d1}', '\u{00aa}', '\u{00ba}',
    '\u{00bf}', '\u{2310}', '\u{00ac}', '\u{00bd}', '\u{00bc}', '\u{00a1}', '\u{00ab}', '\u{00bb}',
    '\u{2591}', '\u{2592}', '\u{2593}', '\u{2502}', '\u{2524}', '\u{2561}', '\u{2562}', '\u{2556}',
    '\u{2555}', '\u{2563}', '\u{2551}', '\u{2557}', '\u{255d}', '\u{255c}', '\u{255b}', '\u{2510}',
    '\u{2514}', '\u{2534}', '\u{252c}', '\u{251c}', '\u{2500}', '\u{253c}', '\u{255e}', '\u{255f}',
    '\u{255a}', '\u{2554}', '\u{2569}', '\u{2566}', '\u{2560}', '\u{2550}', '\u{256c}', '\u{2567}',
    '\u{2568}', '\u{2564}', '\u{2565}', '\u{2559}', '\u{2558}', '\u{2552}', '\u{2553}', '\u{256b}',
    '\u{256a}', '\u{2518}', '\u{250c}', '\u{2588}', '\u{2584}', '\u{258c}', '\u{2590}', '\u{2580}',
    '\u{03b1}', '\u{00df}', '\u{0393}', '\u{03c0}', '\u{03a3}', '\u{03c3}', '\u{00b5}', '\u{03c4}',
    '\u{03a6}', '\u{0398}', '\u{03a9}', '\u{03b4}', '\u{221e}', '\u{03c6}', '\u{03b5}', '\u{2229}',
    '\u{2261}', '\u{00b1}', '\u{2265}', '\u{2264}', '\u{2320}', '\u{2321}', '\u{00f7}', '\u{2248}',
    '\u{00b0}', '\u{2219}', '\u{00b7}', '\u{221a}', '\u{207f}', '\u{00b2}', '\u{25a0}', '\u{00a0}',
];

/// The encoding once the vague cases have been settled by looking at the bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settled {
    Utf8,
    Ascii,
    Latin1,
    Cp437,
    Utf16(Endian),
}

impl Settled {
    /// Bytes per code unit: what a terminator, a pad and the scan step are made of.
    pub fn unit(self) -> usize {
        match self {
            Settled::Utf16(_) => 2,
            _ => 1,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Settled::Utf8 => "UTF-8",
            Settled::Ascii => "ASCII",
            Settled::Latin1 => "Latin-1",
            Settled::Cp437 => "CP437",
            Settled::Utf16(Endian::Little) => "UTF-16 LE",
            Settled::Utf16(Endian::Big) => "UTF-16 BE",
        }
    }
}

/// What a field was read as, and how that was decided.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub text: String,
    pub settled: Settled,
    /// Bytes taken by a byte-order mark: part of the field, not of the value.
    pub bom: usize,
    /// True when the bytes do not fit the encoding and the text is a repair.
    pub lossy: bool,
    /// Set when the template did not name the encoding outright.
    pub note: Option<String>,
}

/// Decide the encoding from the first bytes without decoding them: what the
/// scanner needs before it can tell how long the field is.
pub fn settle(enc: &Encoding, head: &[u8]) -> (Settled, usize, Option<String>) {
    match enc {
        Encoding::Utf8 => (Settled::Utf8, 0, None),
        Encoding::Ascii => (Settled::Ascii, 0, None),
        Encoding::Latin1 => (Settled::Latin1, 0, None),
        Encoding::Cp437 => (Settled::Cp437, 0, None),
        Encoding::Utf16(e) => (Settled::Utf16(*e), 0, None),
        Encoding::Bom { fallback } => match head {
            [0xef, 0xbb, 0xbf, ..] => (Settled::Utf8, 3, Some("Read as UTF-8, from a byte-order mark".into())),
            [0xff, 0xfe, ..] => (Settled::Utf16(Endian::Little), 2, Some("Read as UTF-16 LE, from a byte-order mark".into())),
            [0xfe, 0xff, ..] => (Settled::Utf16(Endian::Big), 2, Some("Read as UTF-16 BE, from a byte-order mark".into())),
            _ => {
                let (s, _, _) = settle(fallback, head);
                (s, 0, Some(format!("Read as {}; no byte-order mark found", s.name())))
            }
        },
        // Not stated by the format: take UTF-8 if the bytes are valid UTF-8,
        // since arbitrary bytes rarely are, and Latin-1 otherwise.
        Encoding::Unknown => {
            if std::str::from_utf8(head).is_ok() {
                (Settled::Utf8, 0, Some("Read as UTF-8, a guess (valid UTF-8)".into()))
            } else {
                (Settled::Latin1, 0, Some("Read as Latin-1, a guess (not valid UTF-8)".into()))
            }
        }
    }
}

pub fn decode(enc: &Encoding, bytes: &[u8]) -> Reading {
    let (settled, bom, note) = settle(enc, bytes);
    let body = &bytes[bom.min(bytes.len())..];
    let (text, lossy) = decode_settled(settled, body);
    Reading { text, settled, bom, lossy, note }
}

pub fn decode_settled(settled: Settled, bytes: &[u8]) -> (String, bool) {
    match settled {
        Settled::Utf8 => match std::str::from_utf8(bytes) {
            Ok(s) => (s.to_string(), false),
            Err(_) => (String::from_utf8_lossy(bytes).into_owned(), true),
        },
        Settled::Ascii => {
            let lossy = bytes.iter().any(|b| *b > 0x7f);
            let text = bytes
                .iter()
                .map(|b| if *b > 0x7f { char::REPLACEMENT_CHARACTER } else { *b as char })
                .collect();
            (text, lossy)
        }
        Settled::Latin1 => (bytes.iter().map(|b| *b as char).collect(), false),
        Settled::Cp437 => (bytes.iter().map(|b| cp437_char(*b)).collect(), false),
        Settled::Utf16(e) => {
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|p| match e {
                    Endian::Little => u16::from_le_bytes([p[0], p[1]]),
                    Endian::Big => u16::from_be_bytes([p[0], p[1]]),
                })
                .collect();
            // A trailing half unit means the field does not hold whole characters.
            let odd = bytes.len() % 2 != 0;
            match String::from_utf16(&units) {
                Ok(s) => (s, odd),
                Err(_) => (String::from_utf16_lossy(&units), true),
            }
        }
    }
}

pub fn cp437_char(b: u8) -> char {
    if b < 0x80 {
        b as char
    } else {
        CP437_HIGH[(b - 0x80) as usize]
    }
}

/// Text to bytes in the encoding it was read as. A character the encoding
/// cannot hold is returned rather than quietly replaced.
pub fn encode_settled(settled: Settled, text: &str) -> Result<Vec<u8>, char> {
    match settled {
        Settled::Utf8 => Ok(text.as_bytes().to_vec()),
        Settled::Ascii => text.chars().map(|c| if c.is_ascii() { Ok(c as u8) } else { Err(c) }).collect(),
        Settled::Latin1 => text.chars().map(|c| if (c as u32) <= 0xff { Ok(c as u8) } else { Err(c) }).collect(),
        Settled::Cp437 => text
            .chars()
            .map(|c| match CP437_HIGH.iter().position(|x| *x == c) {
                Some(i) => Ok(0x80 + i as u8),
                None if c.is_ascii() => Ok(c as u8),
                None => Err(c),
            })
            .collect(),
        Settled::Utf16(e) => {
            let mut out = Vec::with_capacity(text.len() * 2);
            for u in text.encode_utf16() {
                out.extend_from_slice(&match e {
                    Endian::Little => u.to_le_bytes(),
                    Endian::Big => u.to_be_bytes(),
                });
            }
            Ok(out)
        }
    }
}

/// Bytes written the way C writes a string: the printable ones as they are,
/// the rest as escapes. A PNG's signature reads `"\x89PNG\r\n\x1a\n"`, which
/// says both that it starts with a byte no text file has and that the rest of
/// it is the word PNG.
///
/// The escapes are the ones C defines, and are kept unambiguous the way C
/// needs: `\x89` swallows every hex digit after it, so a byte that would be
/// read into the escape before it is written in octal instead, which is
/// always three digits and stops there.
///
/// That costs a signature two bases at once: Matroska reads `"\032E\xdf\xa3"`,
/// where `\032` and the `0x1a` in the gutter are the same byte written two
/// ways. Rust and Python stop `\x` after two digits and would write `\x1a`
/// there, and which language's rules to follow is worth offering as a setting
/// rather than deciding here. Until it is one, C's rules stand: a string that
/// is wrong in C is wrong without saying so, and this is the safe direction to
/// be wrong in.
pub fn c_string(bytes: &[u8]) -> String {
    let hex = |b: u8| b.is_ascii_hexdigit();
    let octal = |b: u8| b.is_ascii_digit() && b < b'8';
    let mut out = String::with_capacity(bytes.len() + 2);
    out.push('"');
    for (i, &b) in bytes.iter().enumerate() {
        let next = bytes.get(i + 1).copied();
        match b {
            b'"' => out.push_str(r#"\""#),
            b'\\' => out.push_str(r"\\"),
            0x20..=0x7e => out.push(b as char),
            0x07 => out.push_str(r"\a"),
            0x08 => out.push_str(r"\b"),
            0x09 => out.push_str(r"\t"),
            0x0a => out.push_str(r"\n"),
            0x0b => out.push_str(r"\v"),
            0x0c => out.push_str(r"\f"),
            0x0d => out.push_str(r"\r"),
            0 if !next.is_some_and(octal) => out.push_str(r"\0"),
            _ if next.is_some_and(hex) => {
                let _ = std::fmt::Write::write_fmt(&mut out, format_args!(r"\{b:03o}"));
            }
            _ => {
                let _ = std::fmt::Write::write_fmt(&mut out, format_args!(r"\x{b:02x}"));
            }
        }
    }
    out.push('"');
    out
}

/// The bytes a pad or terminator takes in this encoding: one byte, or a whole
/// code unit for UTF-16.
pub fn unit_bytes(settled: Settled, byte: u8) -> Vec<u8> {
    match settled {
        Settled::Utf16(Endian::Little) => vec![byte, 0],
        Settled::Utf16(Endian::Big) => vec![0, byte],
        _ => vec![byte],
    }
}

/// Every way a run of bytes reads as text.
///
/// A hex editor's reader picks out some bytes and wants to know what they say.
/// Answering with six rows, one per encoding, is answering with noise: most
/// runs are printable ASCII, and every encoding here agrees on that range, so
/// five of the six rows would be the same sentence. So the readings that agree
/// are gathered together and the encodings that agree on one are named beside
/// it, which is a fact worth having on its own: bytes that read the same
/// whatever you assume are bytes nobody can misread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Readings {
    /// One entry per distinct reading, with the encodings giving it. In the
    /// order the encodings are tried, so the first entry is the first
    /// encoding that produced it.
    pub agreed: Vec<(Vec<Settled>, String)>,
    /// Encodings the bytes do not fit: a high byte where ASCII has none, half
    /// a code unit at the end of a UTF-16 run, a byte sequence UTF-8 does not
    /// allow. Named rather than shown, since what they produce is a row of
    /// replacement characters that says nothing.
    pub refused: Vec<Settled>,
}

/// The encodings a run of bytes is offered as, in the order they are tried.
/// The same six the text view offers, so a reading found here can be turned on
/// there.
pub const OFFERED: [Settled; 6] = [
    Settled::Utf8,
    Settled::Ascii,
    Settled::Latin1,
    Settled::Cp437,
    Settled::Utf16(Endian::Little),
    Settled::Utf16(Endian::Big),
];

/// Read `bytes` every offered way, gathering the encodings that agree.
///
/// `first` puts one encoding at the front of the order, which is what the
/// reader is most likely reading the file in. It changes which encoding gets
/// named first on a shared row and nothing else.
pub fn readings(bytes: &[u8], first: Option<Settled>) -> Readings {
    let mut order: Vec<Settled> = first.into_iter().collect();
    order.extend(OFFERED.iter().copied().filter(|s| Some(*s) != first));
    let mut agreed: Vec<(Vec<Settled>, String)> = Vec::new();
    let mut refused = Vec::new();
    for enc in order {
        let (text, lossy) = decode_settled(enc, bytes);
        if lossy {
            refused.push(enc);
            continue;
        }
        match agreed.iter_mut().find(|(_, t)| *t == text) {
            Some((who, _)) => who.push(enc),
            None => agreed.push((vec![enc], text)),
        }
    }
    Readings { agreed, refused }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printable_bytes_read_the_same_whatever_you_assume() {
        let r = readings(b"hello", None);
        assert_eq!(r.agreed.len(), 1, "one reading, four encodings agreeing on it");
        let (who, text) = &r.agreed[0];
        assert_eq!(text, "hello");
        assert_eq!(who, &[Settled::Utf8, Settled::Ascii, Settled::Latin1, Settled::Cp437]);
        // Five bytes is not a whole number of UTF-16 code units.
        assert_eq!(r.refused, vec![Settled::Utf16(Endian::Little), Settled::Utf16(Endian::Big)]);
    }

    #[test]
    fn a_high_byte_is_where_the_encodings_part() {
        let r = readings(&[0xb0, 0xb1], None);
        let texts: Vec<&str> = r.agreed.iter().map(|(_, t)| t.as_str()).collect();
        assert!(texts.contains(&"\u{b0}\u{b1}"), "Latin-1 reads them as its own characters");
        assert!(texts.contains(&"\u{2591}\u{2592}"), "CP437 reads them as shading");
        assert!(r.refused.contains(&Settled::Ascii), "ASCII has no room for either");
        assert!(r.refused.contains(&Settled::Utf8), "and they are not valid UTF-8");
    }

    #[test]
    fn the_encoding_asked_for_first_is_named_first() {
        let r = readings(b"hi", Some(Settled::Cp437));
        assert_eq!(r.agreed[0].0[0], Settled::Cp437);
    }

    #[test]
    fn bytes_read_as_c_writes_them() {
        assert_eq!(c_string(b"GGUF"), r#""GGUF""#);
        assert_eq!(c_string(b"\x89PNG\r\n\x1a\n"), r#""\x89PNG\r\n\x1a\n""#);
        assert_eq!(c_string(b"\0asm"), r#""\0asm""#);
        assert_eq!(c_string(b"say \"hi\"\\"), r#""say \"hi\"\\""#);
        // A byte C would read into the escape before it goes in octal, which
        // ends after three digits: `\x1f` then `e` would be one escape.
        assert_eq!(c_string(&[0x1f, b'e']), r#""\037e""#);
        assert_eq!(c_string(&[0, b'7']), r#""\0007""#);
        assert_eq!(c_string(&[0, b'x']), r#""\0x""#);
    }

    #[test]
    fn cp437_landmarks_and_round_trip() {
        // Checked against Python's codec, which generated the table.
        assert_eq!(cp437_char(0x80), '\u{00c7}');
        assert_eq!(cp437_char(0xe1), '\u{00df}');
        assert_eq!(cp437_char(0xfd), '\u{00b2}');
        let all: Vec<u8> = (0u8..=255).collect();
        let (text, lossy) = decode_settled(Settled::Cp437, &all);
        assert!(!lossy);
        assert_eq!(encode_settled(Settled::Cp437, &text).unwrap(), all);
    }

    #[test]
    fn latin1_and_ascii() {
        let bytes = [0x41, 0xe9, 0xff];
        assert_eq!(decode_settled(Settled::Latin1, &bytes).0, "A\u{00e9}\u{00ff}");
        assert_eq!(encode_settled(Settled::Latin1, "A\u{00e9}\u{00ff}").unwrap(), bytes);
        assert_eq!(encode_settled(Settled::Latin1, "\u{20ac}"), Err('\u{20ac}'));
        let (text, lossy) = decode_settled(Settled::Ascii, &bytes);
        assert!(lossy);
        assert_eq!(text.chars().next(), Some('A'));
        assert_eq!(encode_settled(Settled::Ascii, "\u{00e9}"), Err('\u{00e9}'));
    }

    #[test]
    fn utf16_both_ways() {
        let le = encode_settled(Settled::Utf16(Endian::Little), "Hi").unwrap();
        assert_eq!(le, vec![0x48, 0, 0x69, 0]);
        assert_eq!(decode_settled(Settled::Utf16(Endian::Little), &le).0, "Hi");
        let be = encode_settled(Settled::Utf16(Endian::Big), "Hi").unwrap();
        assert_eq!(be, vec![0, 0x48, 0, 0x69]);
        // Half a code unit at the end is a broken field, and says so.
        assert!(decode_settled(Settled::Utf16(Endian::Big), &[0, 0x48, 0]).1);
        assert_eq!(unit_bytes(Settled::Utf16(Endian::Little), 0), vec![0, 0]);
    }

    #[test]
    fn boms_and_guesses() {
        let bom = Encoding::Bom { fallback: Box::new(Encoding::Latin1) };
        let r = decode(&bom, &[0xff, 0xfe, 0x48, 0x00]);
        assert_eq!(r.settled, Settled::Utf16(Endian::Little));
        assert_eq!(r.bom, 2);
        assert_eq!(r.text, "H");
        assert_eq!(r.note.as_deref(), Some("Read as UTF-16 LE, from a byte-order mark"));

        let plain = decode(&bom, &[0x48, 0xe9]);
        assert_eq!(plain.settled, Settled::Latin1);
        assert_eq!(plain.bom, 0);
        assert_eq!(plain.text, "H\u{00e9}");

        let utf8 = decode(&Encoding::Unknown, "caf\u{00e9}".as_bytes());
        assert_eq!(utf8.settled, Settled::Utf8);
        assert_eq!(utf8.text, "caf\u{00e9}");
        let latin = decode(&Encoding::Unknown, &[0x63, 0x61, 0x66, 0xe9]);
        assert_eq!(latin.settled, Settled::Latin1);
        assert_eq!(latin.text, "caf\u{00e9}");
    }
}
