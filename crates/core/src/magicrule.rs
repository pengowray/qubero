//! Reading `file(1)` magic rules, and turning what fits into a template.
//!
//! A magic rule file is a mini template language of its own: a test at an
//! offset, of a named type, against a value, with `>` for nesting. That is
//! close enough to this crate's IR to be worth translating, so a format nobody
//! has written a template for still shows the one field the rule proves is
//! there: its signature.
//!
//! Only a small part of the language is translated. A rule that searches, that
//! reads an offset out of the file, or that counts back from the end says
//! nothing this IR can place, and is skipped rather than guessed at. Rules are
//! still parsed in full, because knowing a line exists and cannot be used is
//! the difference between skipping it and mistaking the next line for it.
//!
//! Nothing here identifies a file. The rule database that ships in the browser
//! does that, and names the rule file it used; this reads that one file to find
//! out what the format's first bytes mean.

use crate::template::{Endian, Expr, Template, Ty};

/// One test line of a magic file.
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    /// How many `>` the line carries. Level 0 starts a format; deeper lines are
    /// only tried when the line above them matched.
    pub level: u8,
    /// Where the test reads from, when that is a plain count of bytes from the
    /// start of the file. `None` covers every other form: relative to the last
    /// match, read out of the file, or counted back from the end.
    pub offset: Option<u64>,
    pub kind: Kind,
    pub test: Test,
    /// What the rule prints when it matches. Often a printf template, and often
    /// empty on the line that only establishes the format.
    pub message: String,
    /// From a following `!:mime` line.
    pub mime: Option<String>,
    /// From a following `!:ext` line, in the order it lists them.
    pub ext: Vec<String>,
}

/// What a test line reads.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// Bytes compared as they stand.
    Bytes,
    /// An unsigned integer `width` bytes wide, in a stated byte order. `mask`
    /// is the `&0xffffff00` a rule may narrow the comparison with, which leaves
    /// only some of the bytes on disk known.
    Uint { width: u32, endian: Endian, mask: Option<u64> },
    /// A type this module does not read: a search, a regex, a date, a
    /// subroutine definition or call, or a number in host byte order, which
    /// names no order a file could be said to have.
    Other,
}

/// What the value has to be for the line to match.
#[derive(Debug, Clone, PartialEq)]
pub enum Test {
    /// Equal to these bytes.
    Bytes(Vec<u8>),
    /// Equal to this number.
    Num(u64),
    /// `x`: matches whatever is there.
    Any,
    /// A comparison this module does not evaluate: `<`, `>`, `&`, `^`, `!`, `~`.
    Other,
}

/// A format's signature: the bytes a rule proves are at a known offset.
#[derive(Debug, Clone, PartialEq)]
pub struct Signature {
    pub offset: u64,
    pub bytes: Vec<u8>,
    /// The matching rule's own message, which is empty more often than not.
    pub message: String,
    pub mime: Option<String>,
    pub ext: Vec<String>,
}

/// The shortest signature worth showing. One byte proves too little to be worth
/// a field of its own, and matches far too much.
const MIN_SIGNATURE: usize = 2;

/// Parse a whole magic file. Lines this module cannot read still come back, as
/// `Kind::Other` or with no offset, so a caller counts the same lines the rule
/// database did.
pub fn parse(text: &str) -> Vec<Rule> {
    let mut out: Vec<Rule> = Vec::new();
    for line in text.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("!:") {
            if let Some(last) = out.last_mut() {
                apply_annotation(last, rest);
            }
            continue;
        }
        if let Some(rule) = parse_rule(line) {
            out.push(rule);
        }
    }
    out
}

/// `mime application/zip`, `ext zip/zipx`, and the rest, which say nothing
/// about layout.
fn apply_annotation(rule: &mut Rule, rest: &str) {
    let mut it = rest.splitn(2, char::is_whitespace);
    let (key, value) = (it.next().unwrap_or(""), it.next().unwrap_or("").trim());
    match key {
        "mime" if !value.is_empty() => rule.mime = Some(value.to_string()),
        "ext" if !value.is_empty() => {
            rule.ext = value.split('/').filter(|e| !e.is_empty()).map(str::to_string).collect();
        }
        _ => {}
    }
}

fn parse_rule(line: &str) -> Option<Rule> {
    let mut level = 0u8;
    let body = {
        let mut rest = line.trim_start();
        while let Some(r) = rest.strip_prefix('>') {
            level = level.saturating_add(1);
            rest = r;
        }
        rest
    };
    let mut tokens = Tokens::new(body);
    let offset_tok = tokens.next()?;
    let type_tok = tokens.next()?;
    let test_tok = tokens.next();
    let message = tokens.rest().trim().to_string();

    let kind = parse_kind(&type_tok);
    let test = match test_tok {
        None => Test::Any,
        Some(t) => parse_test(&t, &kind),
    };
    Some(Rule { level, offset: parse_offset(&offset_tok), kind, test, message, mime: None, ext: Vec::new() })
}

/// A plain count of bytes from the start of the file, or nothing. `&` counts
/// from the last match, `(` reads the offset out of the file and `-` counts
/// back from the end; none of those is a fixed place.
fn parse_offset(tok: &str) -> Option<u64> {
    let t = tok.strip_prefix('+').unwrap_or(tok);
    if t.is_empty() || t.starts_with('&') || t.starts_with('(') || t.starts_with('-') {
        return None;
    }
    parse_number(t).and_then(|n| u64::try_from(n).ok())
}

/// C's number forms, which is what magic files are written in: `0x` hex, a
/// leading `0` for octal, otherwise decimal. A trailing `L` or `U` is noise.
fn parse_number(t: &str) -> Option<u128> {
    let t = t.trim_end_matches(['L', 'l', 'U', 'u']);
    if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return u128::from_str_radix(h, 16).ok();
    }
    if t.len() > 1 && t.starts_with('0') {
        return u128::from_str_radix(&t[1..], 8).ok();
    }
    t.parse::<u128>().ok()
}

fn parse_kind(tok: &str) -> Kind {
    // `%` is a modulo test, which says nothing about any one byte.
    if tok.contains('%') {
        return Kind::Other;
    }
    // A mask (`belong&0xffffff00`) tests part of a value. The bytes the mask
    // covers whole are still bytes the rule proves; the rest are not, and
    // `signature_of` keeps only the run it can vouch for.
    let (tok, mask) = match tok.split_once('&') {
        None => (tok, None),
        Some((base, m)) => match parse_number(m).and_then(|n| u64::try_from(n).ok()) {
            Some(m) => (base, Some(m)),
            None => return Kind::Other,
        },
    };
    let (name, flags) = tok.split_once('/').unwrap_or((tok, ""));
    // Of the string flags, only `b` and `t` leave the bytes alone: they say the
    // rule applies to a binary or to a text file, which is a fact about the
    // file rather than about the comparison. Every other flag (`c` for case,
    // `w` for optional blanks, and the rest) means the bytes on disk need not
    // be the bytes written in the rule. A width, as in `search/1024`, fails the
    // same test and is meant to.
    if !flags.chars().all(|c| c == 'b' || c == 't') {
        return Kind::Other;
    }
    let name = name.strip_prefix('u').unwrap_or(name);
    let (endian, base) = if let Some(b) = name.strip_prefix("be") {
        (Some(Endian::Big), b)
    } else if let Some(b) = name.strip_prefix("le") {
        (Some(Endian::Little), b)
    } else {
        (None, name)
    };
    // `ubelong` parses as `u` + `belong`; `beshort` has no `u` to strip, and a
    // second pass over the base name catches the unsigned spelling either way.
    let base = base.strip_prefix('u').unwrap_or(base);
    match (base, endian) {
        // Bytes compared as they stand have nothing to mask.
        ("string", None) if mask.is_none() => Kind::Bytes,
        ("byte", _) => Kind::Uint { width: 1, endian: Endian::Big, mask },
        ("short", Some(e)) => Kind::Uint { width: 2, endian: e, mask },
        ("long", Some(e)) => Kind::Uint { width: 4, endian: e, mask },
        ("quad", Some(e)) => Kind::Uint { width: 8, endian: e, mask },
        // A bare `short` or `long` is whatever order the machine running
        // `file` happens to use. A file has no such order, so there is nothing
        // to write down.
        _ => Kind::Other,
    }
}

fn parse_test(tok: &str, kind: &Kind) -> Test {
    if tok == "x" {
        return Test::Any;
    }
    let (op, value) = match tok.as_bytes().first() {
        Some(b'=') => ('=', &tok[1..]),
        Some(b'<' | b'>' | b'&' | b'^' | b'~' | b'!') => return Test::Other,
        _ => ('=', tok),
    };
    debug_assert_eq!(op, '=');
    match kind {
        Kind::Bytes => Test::Bytes(unescape(value)),
        Kind::Uint { .. } => match parse_number(value) {
            Some(n) => u64::try_from(n).map(Test::Num).unwrap_or(Test::Other),
            None => Test::Other,
        },
        Kind::Other => Test::Other,
    }
}

/// The escapes a magic string is written with. Octal is the common one:
/// `PK\003\004` is how the zip rule spells its four bytes.
fn unescape(s: &str) -> Vec<u8> {
    let src = s.as_bytes();
    let mut out = Vec::with_capacity(src.len());
    let mut i = 0;
    while i < src.len() {
        if src[i] != b'\\' || i + 1 >= src.len() {
            out.push(src[i]);
            i += 1;
            continue;
        }
        i += 1;
        let c = src[i];
        i += 1;
        match c {
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'b' => out.push(0x08),
            b'f' => out.push(0x0c),
            b'v' => out.push(0x0b),
            b'a' => out.push(0x07),
            b'x' | b'X' => {
                let mut v = 0u32;
                let mut n = 0;
                while n < 2 && i < src.len() && (src[i] as char).is_ascii_hexdigit() {
                    v = v * 16 + (src[i] as char).to_digit(16).unwrap_or(0);
                    i += 1;
                    n += 1;
                }
                // A lone `\x` is not an escape at all; keep it as written.
                if n == 0 { out.push(b'x') } else { out.push(v as u8) }
            }
            b'0'..=b'7' => {
                let mut v = u32::from(c - b'0');
                let mut n = 1;
                while n < 3 && i < src.len() && (b'0'..=b'7').contains(&src[i]) {
                    v = v * 8 + u32::from(src[i] - b'0');
                    i += 1;
                    n += 1;
                }
                out.push(v as u8);
            }
            other => out.push(other),
        }
    }
    out
}

/// Tokens are runs of non-whitespace, except that a backslash takes the next
/// character whatever it is, so `\ ` stays inside a value.
struct Tokens<'a> {
    s: &'a str,
    at: usize,
}

impl<'a> Tokens<'a> {
    fn new(s: &'a str) -> Tokens<'a> {
        Tokens { s, at: 0 }
    }

    fn skip_space(&mut self) {
        let b = self.s.as_bytes();
        while self.at < b.len() && (b[self.at] == b' ' || b[self.at] == b'\t') {
            self.at += 1;
        }
    }

    fn next(&mut self) -> Option<String> {
        self.skip_space();
        let b = self.s.as_bytes();
        if self.at >= b.len() {
            return None;
        }
        let start = self.at;
        while self.at < b.len() && b[self.at] != b' ' && b[self.at] != b'\t' {
            if b[self.at] == b'\\' && self.at + 1 < b.len() {
                self.at += 1;
            }
            self.at += 1;
        }
        Some(self.s[start..self.at.min(self.s.len())].to_string())
    }

    /// Everything left, which is the message.
    fn rest(&mut self) -> &'a str {
        self.skip_space();
        &self.s[self.at.min(self.s.len())..]
    }
}

/// The signature of the format `head` is in, according to `text`.
///
/// Only level-0 rules are considered, and only the ones that pin fixed bytes to
/// a fixed place. A file matching none is a format whose rule this module
/// cannot read. Several may match, but only when they describe the same bytes
/// in differing detail, as a JPEG-LS file matches both the rule for its own
/// four bytes and the rule for the three every JPEG starts with; the fuller
/// one is the answer. Rules covering different ground mean this module has
/// misread the file, which is not something to show anybody.
pub fn match_signature(text: &str, head: &[u8]) -> Option<Signature> {
    let rules = parse(text);
    let mut found: Option<Signature> = None;
    for rule in rules.iter().filter(|r| r.level == 0) {
        let Some(sig) = signature_of(rule) else { continue };
        let end = usize::try_from(sig.offset).ok()?.checked_add(sig.bytes.len())?;
        if end > head.len() || head[sig.offset as usize..end] != sig.bytes[..] {
            continue;
        }
        found = match found {
            None => Some(sig),
            Some(prev) => Some(fuller(prev, sig)?),
        };
    }
    found
}

/// Of two signatures the same file matched, the one that says more, when one
/// covers everything the other does. None when they cover different bytes.
fn fuller(a: Signature, b: Signature) -> Option<Signature> {
    let (outer, inner) = if a.bytes.len() >= b.bytes.len() { (a, b) } else { (b, a) };
    let from = usize::try_from(inner.offset.checked_sub(outer.offset)?).ok()?;
    (from + inner.bytes.len() <= outer.bytes.len()).then_some(outer)
}

/// A number as it sits in the file: `width` bytes of it, in the rule's order.
fn on_disk(n: u64, width: u32, endian: Endian) -> Option<Vec<u8>> {
    let be = n.to_be_bytes();
    let mut b = be[8usize.checked_sub(width as usize)?..].to_vec();
    if endian == Endian::Little {
        b.reverse();
    }
    Some(b)
}

/// The longest run of bytes a mask leaves fully known, as a start and a length.
/// A byte the mask covers only in part says nothing certain about the byte on
/// disk, so it ends the run rather than joining it.
fn known_run(mask: &[u8]) -> Option<(usize, usize)> {
    let mut best = (0usize, 0usize);
    let mut i = 0;
    while i < mask.len() {
        if mask[i] != 0xff {
            i += 1;
            continue;
        }
        let from = i;
        while i < mask.len() && mask[i] == 0xff {
            i += 1;
        }
        if i - from > best.1 {
            best = (from, i - from);
        }
    }
    (best.1 > 0).then_some(best)
}

/// The bytes a rule requires, when it requires any.
fn signature_of(rule: &Rule) -> Option<Signature> {
    let at = rule.offset?;
    let (offset, bytes) = match (&rule.kind, &rule.test) {
        (Kind::Bytes, Test::Bytes(b)) => (at, b.clone()),
        (Kind::Uint { width, endian, mask }, Test::Num(n)) => {
            let bytes = on_disk(*n, *width, *endian)?;
            match mask {
                None => (at, bytes),
                Some(m) => {
                    // A test asking for bits the mask throws away matches no
                    // file at all, so the bytes it names prove nothing.
                    let whole = if *width >= 8 { u64::MAX } else { (1u64 << (width * 8)) - 1 };
                    if n & !m & whole != 0 {
                        return None;
                    }
                    let (from, len) = known_run(&on_disk(*m, *width, *endian)?)?;
                    (at + from as u64, bytes.get(from..from + len)?.to_vec())
                }
            }
        }
        _ => return None,
    };
    if bytes.len() < MIN_SIGNATURE {
        return None;
    }
    Some(Signature {
        offset,
        bytes,
        message: rule.message.clone(),
        mime: rule.mime.clone(),
        ext: rule.ext.clone(),
    })
}

/// A template holding the signature and nothing else.
///
/// The rest of the file is deliberately left out rather than covered by a
/// catch-all field: the rule says nothing about it, and the annotation column
/// already shows what no field describes.
pub fn signature_template(name: &str, sig: &Signature) -> Template {
    let mut fields: Vec<(&str, Ty)> = Vec::new();
    if sig.offset > 0 {
        fields.push(("before signature", Ty::bytes(Expr::lit(i128::from(sig.offset)))));
    }
    fields.push(("signature", Ty::magic(&sig.bytes)));
    Template::new(name, Ty::structure(name, fields))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_line_with_every_column() {
        let r = &parse("0\tstring\t\\x89PNG\tPNG image data")[0];
        assert_eq!(r.level, 0);
        assert_eq!(r.offset, Some(0));
        assert_eq!(r.kind, Kind::Bytes);
        assert_eq!(r.test, Test::Bytes(b"\x89PNG".to_vec()));
        assert_eq!(r.message, "PNG image data");
    }

    #[test]
    fn counts_nesting() {
        let rules = parse("0\tstring\tPK\n>4\tleshort\t20\n>>8\tbyte\t1\n");
        assert_eq!(rules.iter().map(|r| r.level).collect::<Vec<_>>(), [0, 1, 2]);
    }

    #[test]
    fn a_line_may_have_no_message() {
        // The PNG rule's first line establishes the format and prints nothing.
        let r = &parse("0\tstring\t\\x89PNG\\x0d\\x0a\\x1a\\x0a")[0];
        assert_eq!(r.message, "");
        assert_eq!(r.test, Test::Bytes(b"\x89PNG\r\n\x1a\n".to_vec()));
    }

    #[test]
    fn octal_escapes_are_the_common_spelling() {
        assert_eq!(unescape("PK\\003\\004"), b"PK\x03\x04");
        assert_eq!(unescape("\\0\\1\\2"), b"\x00\x01\x02");
    }

    #[test]
    fn hex_and_control_escapes() {
        assert_eq!(unescape("\\x41\\x7a"), b"Az");
        assert_eq!(unescape("a\\nb\\tc"), b"a\nb\tc");
        assert_eq!(unescape("a\\ b"), b"a b");
        assert_eq!(unescape("\\\\"), b"\\");
    }

    #[test]
    fn an_escaped_space_stays_inside_the_value() {
        let r = &parse("0\tstring\tGIF\\ 89\tsomething")[0];
        assert_eq!(r.test, Test::Bytes(b"GIF 89".to_vec()));
        assert_eq!(r.message, "something");
    }

    #[test]
    fn offsets_in_every_base() {
        assert_eq!(parse_offset("0"), Some(0));
        assert_eq!(parse_offset("257"), Some(257));
        assert_eq!(parse_offset("0x1c"), Some(0x1c));
        assert_eq!(parse_offset("010"), Some(8));
    }

    #[test]
    fn offsets_that_name_no_fixed_place() {
        assert_eq!(parse_offset("&0"), None);
        assert_eq!(parse_offset("(0x3c.l)"), None);
        assert_eq!(parse_offset("-22"), None);
    }

    #[test]
    fn integer_types_carry_their_byte_order() {
        assert_eq!(parse_kind("beshort"), Kind::Uint { width: 2, endian: Endian::Big, mask: None });
        assert_eq!(parse_kind("ulelong"), Kind::Uint { width: 4, endian: Endian::Little, mask: None });
        assert_eq!(parse_kind("bequad"), Kind::Uint { width: 8, endian: Endian::Big, mask: None });
        assert_eq!(parse_kind("ubyte"), Kind::Uint { width: 1, endian: Endian::Big, mask: None });
        assert_eq!(
            parse_kind("belong&0xffffff00"),
            Kind::Uint { width: 4, endian: Endian::Big, mask: Some(0xffffff00) }
        );
    }

    #[test]
    fn types_without_a_byte_order_a_file_could_have() {
        // Host order, so there is nothing to write into a template.
        assert_eq!(parse_kind("long"), Kind::Other);
        assert_eq!(parse_kind("short"), Kind::Other);
    }

    #[test]
    fn masked_and_flagged_types_are_not_plain_values() {
        assert_eq!(parse_kind("belong%0x10"), Kind::Other);
        assert_eq!(parse_kind("belong&nonsense"), Kind::Other);
        assert_eq!(parse_kind("string/c"), Kind::Other);
        assert_eq!(parse_kind("string/fwt"), Kind::Other);
        assert_eq!(parse_kind("string/W"), Kind::Other);
        assert_eq!(parse_kind("search/1024"), Kind::Other);
        assert_eq!(parse_kind("regex"), Kind::Other);
        assert_eq!(parse_kind("name"), Kind::Other);
        assert_eq!(parse_kind("use"), Kind::Other);
    }

    #[test]
    fn the_binary_and_text_flags_leave_the_bytes_alone() {
        // `0 string/b MZ` is how every DOS and Windows executable is found.
        // The flag says the rule is for binary files, not that MZ is optional.
        assert_eq!(parse_kind("string/b"), Kind::Bytes);
        assert_eq!(parse_kind("string/t"), Kind::Bytes);
        assert_eq!(parse_kind("string/bt"), Kind::Bytes);
        let sig = match_signature("0\tstring/b\tMZ\tMS-DOS executable", b"MZ and more").unwrap();
        assert_eq!(sig.bytes, b"MZ");
    }

    #[test]
    fn comparisons_other_than_equality_are_left_alone() {
        let r = &parse("0\tbelong\t>100\tbig")[0];
        assert_eq!(r.test, Test::Other);
        let r = &parse("0\tbelong\t=0x1234\tequal")[0];
        assert_eq!(r.test, Test::Num(0x1234));
    }

    #[test]
    fn annotations_attach_to_the_line_above() {
        let rules = parse("0\tstring\tPK\\003\\004\tZip\n!:mime\tapplication/zip\n!:ext zip/zipx\n");
        assert_eq!(rules[0].mime.as_deref(), Some("application/zip"));
        assert_eq!(rules[0].ext, ["zip", "zipx"]);
    }

    #[test]
    fn comments_and_blank_lines_are_not_rules() {
        assert_eq!(parse("# a comment\n\n\t\n").len(), 0);
    }

    const ZIP: &str = "0\tname\t\tzipcd\n>0\tstring\t\tPK\\001\\002\tZip archive data\n-22\tstring\t\tPK\\005\\006\n0\tstring\t\tPK\\003\\004\tZip archive data\n!:mime\tapplication/zip\n!:ext zip\n";

    #[test]
    fn finds_the_signature_a_file_actually_starts_with() {
        let sig = match_signature(ZIP, b"PK\x03\x04rest of the file").unwrap();
        assert_eq!(sig.offset, 0);
        assert_eq!(sig.bytes, b"PK\x03\x04");
        assert_eq!(sig.mime.as_deref(), Some("application/zip"));
        assert_eq!(sig.ext, ["zip"]);
    }

    #[test]
    fn a_nested_line_is_not_a_signature() {
        // `>0 string PK\001\002` sits inside a subroutine and describes a
        // record in the middle of a zip, not the start of one.
        assert!(match_signature(ZIP, b"PK\x01\x02rest").is_none());
    }

    #[test]
    fn no_match_is_no_template() {
        assert!(match_signature(ZIP, b"not a zip at all").is_none());
    }

    #[test]
    fn a_signature_needs_more_than_one_byte() {
        assert!(match_signature("0\tbyte\t\t0x0a\tPCX", b"\x0a\x00\x01").is_none());
    }

    #[test]
    fn one_rule_inside_another_is_read_at_its_fullest() {
        // What the jpeg file does: a rule for one format's own bytes, beside a
        // rule for the bytes its whole family shares.
        let text = "0\tstring\tPK\\003\\004\tone\n0\tstring\tPK\\003\tanother\n";
        let sig = match_signature(text, b"PK\x03\x04").unwrap();
        assert_eq!(sig.bytes, b"PK\x03\x04");
    }

    #[test]
    fn two_rules_covering_different_bytes_mean_the_file_was_misread() {
        let text = "0\tstring\tPK\tone\n4\tstring\tZZ\tanother\n";
        assert!(match_signature(text, b"PK\x03\x04ZZ").is_none());
    }

    #[test]
    fn a_masked_rule_keeps_the_bytes_the_mask_covers_whole() {
        // The rule every JPEG matches: three bytes known, the fourth masked off.
        let jpeg = "0\tbelong&0xffffff00\t0xffd8ff00\tJPEG image data\n";
        let sig = match_signature(jpeg, &[0xff, 0xd8, 0xff, 0xe0]).unwrap();
        assert_eq!(sig.offset, 0);
        assert_eq!(sig.bytes, [0xff, 0xd8, 0xff]);
    }

    #[test]
    fn a_jpeg_ls_file_is_read_by_its_own_rule_rather_than_the_familys() {
        let text = "0\tbelong\t0xffd8fff7\tJPEG-LS\n0\tbelong&0xffffff00\t0xffd8ff00\tJPEG\n";
        let sig = match_signature(text, &[0xff, 0xd8, 0xff, 0xf7]).unwrap();
        assert_eq!(sig.bytes, [0xff, 0xd8, 0xff, 0xf7]);
        let plain = match_signature(text, &[0xff, 0xd8, 0xff, 0xe0]).unwrap();
        assert_eq!(plain.bytes, [0xff, 0xd8, 0xff]);
    }

    #[test]
    fn a_mask_in_the_middle_of_a_value_keeps_only_its_longest_run() {
        // ARC: two bytes known, then a byte the mask only half covers.
        let arc = "0\tlelong&0x8080ffff\t0x0000081a\tARC archive data\n";
        let sig = match_signature(arc, &[0x1a, 0x08, 0x00, 0x00]).unwrap();
        assert_eq!(sig.offset, 0);
        assert_eq!(sig.bytes, [0x1a, 0x08]);
        // Apple Mechanic: the run the mask covers whole starts two bytes in.
        let mech = "0\tbelong&0xFF00FFFF\t0x6400D000\tApple Mechanic font\n";
        let sig = match_signature(mech, &[0x64, 0x11, 0xd0, 0x00]).unwrap();
        assert_eq!(sig.offset, 2);
        assert_eq!(sig.bytes, [0xd0, 0x00]);
    }

    #[test]
    fn a_mask_leaving_no_whole_byte_pair_proves_too_little() {
        // MPEG ADTS: fifteen bits, which is one whole byte and part of another.
        let adts = "0\tbeshort&0xFFFE\t0xFFFA\tMPEG\n";
        assert!(match_signature(adts, &[0xff, 0xfa]).is_none());
        // Alternating bytes: no two known bytes in a row.
        let alt = "0\tbelong&0x00ff00ff\t0x00080000\tsomething\n";
        assert!(match_signature(alt, &[0, 8, 0, 0]).is_none());
    }

    #[test]
    fn a_test_asking_for_bits_the_mask_throws_away_matches_nothing() {
        let broken = "0\tbelong&0xffffff00\t0xffd8ff07\tbroken\n";
        assert!(match_signature(broken, &[0xff, 0xd8, 0xff, 0x07]).is_none());
    }

    #[test]
    fn a_number_signature_is_written_in_its_own_byte_order() {
        let sig = match_signature("0\tbelong\t0x1f8b0800\tgzip", &[0x1f, 0x8b, 0x08, 0x00]).unwrap();
        assert_eq!(sig.bytes, [0x1f, 0x8b, 0x08, 0x00]);
        let sig = match_signature("0\tlelong\t0x1f8b0800\tgzip", &[0x00, 0x08, 0x8b, 0x1f]).unwrap();
        assert_eq!(sig.bytes, [0x00, 0x08, 0x8b, 0x1f]);
    }

    #[test]
    fn the_template_describes_the_signature_and_claims_nothing_else() {
        let sig = match_signature(ZIP, b"PK\x03\x04more").unwrap();
        let t = signature_template("zip", &sig);
        assert_eq!(t.name, "zip");
        let Ty::Struct(s) = &t.root else { panic!("expected a struct") };
        assert_eq!(s.fields.len(), 1);
        assert_eq!(&*s.fields[0].name, "signature");
        assert!(matches!(&s.fields[0].ty, Ty::Magic(b) if b == b"PK\x03\x04"));
    }

    #[test]
    fn a_signature_past_the_start_keeps_its_place() {
        // A tar's `ustar` sits 257 bytes in, so the field cannot be first.
        let mut head = vec![0u8; 300];
        head[257..262].copy_from_slice(b"ustar");
        let sig = match_signature("257\tstring\tustar\tPOSIX tar archive", &head).unwrap();
        assert_eq!(sig.offset, 257);
        let t = signature_template("tar", &sig);
        let Ty::Struct(s) = &t.root else { panic!("expected a struct") };
        assert_eq!(s.fields.len(), 2);
        assert_eq!(&*s.fields[0].name, "before signature");
        assert_eq!(&*s.fields[1].name, "signature");
    }
}
