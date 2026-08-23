//! Reading Detect It Easy signature rules, for the ones written plainly enough
//! to read without running them.
//!
//! Where `file(1)` answers what format a file is, these answer what tool
//! produced it: which packer, which compiler, which protector. That is the
//! question someone opens a DOS or Windows executable to ask, and nothing in
//! the `file` database knows it.
//!
//! A rule is a small JavaScript program, so most of the database can only be
//! answered by running it. A large part of it does not need running: a rule
//! that tests one byte pattern and then assigns two strings says everything it
//! has to say in its own text. Those are read here. Everything else is counted
//! and skipped, never half-read.
//!
//! The pattern language is `die_script`'s, documented in `xbinary.h` of the
//! `Formats` project, and it is shared by every tool the same author builds on
//! that engine. It is implemented here as the language rather than as the
//! handful of shapes the DOS rules happen to use, so the same parser reads the
//! rest of the family.

use std::collections::BTreeMap;

/// One byte of a pattern. Most are an exact value or a gap, but the language
/// can also ask for a byte of a class without naming it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigByte {
    /// `4d`: this byte and no other.
    Exact(u8),
    /// `..`, `??`, and the address forms `$$` and `##`, whose contents are an
    /// address rather than a constant and so are never fixed.
    Any,
    /// `%%`: printable ASCII.
    Printable,
    /// `!%`: anything printable ASCII is not.
    NotPrintable,
    /// `_%`: not ASCII and not zero.
    NotAsciiNotNull,
    /// `%&`: a digit or a letter.
    AlphaNumeric,
    /// `**`: anything but zero.
    NotNull,
}

impl SigByte {
    pub fn accepts(self, b: u8) -> bool {
        match self {
            SigByte::Exact(v) => v == b,
            SigByte::Any => true,
            SigByte::Printable => (0x20..=0x7e).contains(&b),
            SigByte::NotPrintable => !(0x20..=0x7e).contains(&b),
            SigByte::NotAsciiNotNull => b >= 0x80,
            SigByte::AlphaNumeric => b.is_ascii_alphanumeric(),
            SigByte::NotNull => b != 0,
        }
    }
}

/// Where a pattern is measured from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    /// `Binary.compare`: the start of the file. A `.COM` is loaded flat, so for
    /// one of those this is also where it starts running.
    FileStart,
    /// `MSDOS.compareEP`: the instruction the loader jumps to, which for an
    /// `MZ` executable is worked out from the header rather than fixed.
    EntryPoint,
    /// `PE.compareEP`: the same for a Windows executable, where the header
    /// gives the entry point as an address in memory and the section table is
    /// what turns it back into a place in the file.
    PeEntryPoint,
    /// `PE.compareOverlay`: the bytes past the last section, which an
    /// installer or a self-extracting archive puts its payload in.
    Overlay,
}

/// What a branch asks of the file. Rules join these with `&&` and `||`, and
/// occasionally negate one, so a branch is a tree rather than a single test.
#[derive(Debug, Clone, PartialEq)]
pub enum Test {
    /// A run of bytes at a place the file decides.
    Pattern { anchor: Anchor, offset: i64, pattern: Vec<SigByte> },
    /// `PE.isSectionNamePresent(".rsrc")`: a section of exactly this name.
    SectionName(String),
    /// `PE.isNet()`: the file carries a .NET runtime header.
    IsNet,
    All(Vec<Test>),
    Any(Vec<Test>),
    Not(Box<Test>),
}

/// One test in a rule, and what the rule concludes when it passes.
#[derive(Debug, Clone, PartialEq)]
pub struct Branch {
    /// A name of this branch's own, replacing the rule's. Rules covering a
    /// family of tools use it to say which one this is: `sName = "crypt 95-97"`.
    pub name: Option<String>,
    /// Appended to whichever name applies, for a variant of the same tool:
    /// `sName += ' N2'`.
    pub name_suffix: String,
    pub test: Test,
    pub version: Option<String>,
    pub options: Option<String>,
}

/// A rule this module can read: what it detects, and the tests that detect it.
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    /// The database's own word: `packer`, `compiler`, `protector`.
    pub category: String,
    pub name: String,
    /// In order. The first that matches is the answer, which is how the rule
    /// itself is written: a chain of `else if`.
    pub branches: Vec<Branch>,
    /// The signature file it came from, for tracking an answer back.
    pub source: String,
}

/// What one rule concluded about a file.
#[derive(Debug, Clone, PartialEq)]
pub struct Detection {
    pub category: String,
    pub name: String,
    pub version: Option<String>,
    pub options: Option<String>,
    pub source: String,
}

/// Why a rule in the database was not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Skipped {
    /// It asks something of the file this module cannot answer: the imports,
    /// the sections, the entropy.
    NeedsMoreThanBytes,
    /// Its answer is computed from the file rather than written down.
    ComputedAnswer,
    /// A pattern using a form that is not a fixed run of bytes, such as the
    /// `+` search.
    UnreadablePattern,
    /// No `meta(...)`, so nothing to report even if it matched.
    NoMeta,
}

/// The result of reading a whole database.
#[derive(Debug, Clone, Default)]
pub struct Database {
    pub rules: Vec<Rule>,
    /// How many files were skipped, and why. Counted rather than ignored, so
    /// the share of the database being used is a number rather than a guess.
    pub skipped: BTreeMap<Skipped, usize>,
}

impl Database {
    fn skip(&mut self, why: Skipped) {
        *self.skipped.entry(why).or_default() += 1;
    }

    pub fn skipped_total(&self) -> usize {
        self.skipped.values().sum()
    }
}

/// What a file says about itself, worked out once and asked many times.
///
/// A rule may want the entry point, the section names, the overlay or whether
/// there is a .NET header. Reading those out of the headers for every one of a
/// thousand rules would be a thousand times the work for the same answer.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    /// Where the file starts running, as a file offset.
    pub entry: Option<u64>,
    /// Where the bytes past the last section begin, for a PE that has any.
    pub overlay: Option<u64>,
    /// Section names, with the padding taken off.
    pub sections: Vec<String>,
    /// Whether the file carries a .NET runtime header.
    pub is_net: bool,
}

impl Facts {
    /// Read what can be read. A file that is none of these things gets a
    /// `Facts` saying so rather than an error: the rules that care will not
    /// match, which is the right answer.
    pub fn of(head: &[u8], file_len: u64) -> Facts {
        let mut f = Facts {
            entry: pe_entry_point(head).or_else(|| mz_entry_point(head, file_len)),
            ..Facts::default()
        };
        let Some(pe) = pe_header_at(head) else { return f };
        let long = |at: usize| -> Option<u32> {
            let b = head.get(at..at + 4)?;
            Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        let word = |at: usize| -> Option<u32> {
            let b = head.get(at..at + 2)?;
            Some(u32::from(u16::from_le_bytes([b[0], b[1]])))
        };
        let Some(count) = word(pe + 6) else { return f };
        let Some(optional_size) = word(pe + 20) else { return f };
        let optional = pe + 24;
        // The fifteenth data directory is the .NET header. A PE32+ puts the
        // directories 16 bytes further on, since four of its fields are wider.
        if let Some(magic) = word(optional) {
            let dirs = optional + if magic == 0x20b { 112 } else { 96 };
            f.is_net = long(dirs + 14 * 8).is_some_and(|rva| rva != 0);
        }
        let table = optional + optional_size as usize;
        let mut end: u64 = 0;
        for i in 0..(count as usize).min(96) {
            let at = table + i * 40;
            let Some(name) = head.get(at..at + 8) else { break };
            let name = name.split(|b| *b == 0).next().unwrap_or(name);
            if let Ok(text) = std::str::from_utf8(name) {
                f.sections.push(text.to_string());
            }
            if let (Some(raw_size), Some(raw_offset)) = (long(at + 16), long(at + 20)) {
                end = end.max(u64::from(raw_offset) + u64::from(raw_size));
            }
        }
        // Bytes past the last section, if the file has any.
        f.overlay = if end > 0 && end < file_len { Some(end) } else { None };
        f
    }
}

/// Where a file's PE header sits, if it has one.
fn pe_header_at(head: &[u8]) -> Option<usize> {
    if !head.starts_with(b"MZ") {
        return None;
    }
    let b = head.get(0x3c..0x40)?;
    let at = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize;
    if head.get(at..at + 4)? == b"PE\0\0" { Some(at) } else { None }
}

/// The marker `tools/die.mjs` writes between the signature files it bundles.
/// The files themselves are copied byte for byte, so a new version of the
/// database drops in without anything being rewritten.
pub const FILE_MARKER: &str = "// >>> file: ";

/// Read a bundle of signature files.
pub fn parse_bundle(text: &str) -> Database {
    let mut db = Database::default();
    let mut name = String::new();
    let mut body = String::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix(FILE_MARKER) {
            finish(&mut db, &name, &body);
            name = rest.trim().to_string();
            body.clear();
        } else {
            body.push_str(line);
            body.push('\n');
        }
    }
    finish(&mut db, &name, &body);
    db
}

fn finish(db: &mut Database, name: &str, body: &str) {
    if name.is_empty() || body.trim().is_empty() {
        return;
    }
    match parse_rule(name, body) {
        Ok(rule) => db.rules.push(rule),
        Err(why) => db.skip(why),
    }
}

/// Read one signature file.
pub fn parse_rule(source: &str, text: &str) -> Result<Rule, Skipped> {
    let (category, name) = parse_meta(text).ok_or(Skipped::NoMeta)?;
    let mut branches: Vec<Branch> = Vec::new();
    let mut current: Option<Branch> = None;
    let mut computed = false;

    // A condition may run over several lines when it joins two tests, so the
    // lines are put back together before anything is read out of them.
    let joined = join_conditions(text);
    // How deep in braces the line is. A branch is an `if` directly inside
    // `detect()`; one deeper than that is a rule doing something conditional
    // within a branch, which is not a shape this module follows.
    let mut depth = 0i32;
    for raw in joined.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        let here = depth;
        depth += braces(line);
        if is_boilerplate(line) {
            continue;
        }
        if let Some(cond) = condition_of(line) {
            // `if` at the top of `detect()`, or `} else if` closing the branch
            // before it. Anything deeper is nested.
            let opens_branch = if line.starts_with("if (") { here == 1 } else { here == 2 };
            if !opens_branch {
                return Err(Skipped::NeedsMoreThanBytes);
            }
            let test = parse_test(cond).ok_or(Skipped::NeedsMoreThanBytes)?;
            let branch = Branch { name: None, name_suffix: String::new(), test, version: None, options: None };
            if let Some(b) = current.take() {
                branches.push(b);
            }
            current = Some(branch);
            continue;
        }
        if let Some((key, append, value)) = assignment_of(line) {
            let Some(b) = current.as_mut() else { continue };
            match (key, append) {
                ("sVersion", false) => match literal(value) {
                    Some(v) => b.version = Some(v),
                    None => computed = true,
                },
                ("sOptions", false) => match literal(value) {
                    Some(v) => b.options = Some(v),
                    None => computed = true,
                },
                // A rule may cover a family and name the member in the
                // branch, or name a variant by appending to it. Either way the
                // name belongs to the branch that matched, not to the rule.
                ("sName", false) => match literal(value) {
                    Some(v) => b.name = Some(v),
                    None => computed = true,
                },
                ("sName", true) => match literal(value) {
                    Some(v) => b.name_suffix.push_str(&v),
                    None => computed = true,
                },
                // The language the tool was written in, which is a fact about
                // the tool rather than about the file. Read and not reported.
                ("sLang", _) => {}
                ("bDetected", false) => {}
                // A rule that keeps other state is doing something this module
                // is not following.
                _ => return Err(Skipped::NeedsMoreThanBytes),
            }
            continue;
        }
        // Anything else in the body: a loop, a call, a nested test.
        return Err(Skipped::NeedsMoreThanBytes);
    }
    if let Some(b) = current.take() {
        branches.push(b);
    }
    if computed {
        return Err(Skipped::ComputedAnswer);
    }
    if branches.is_empty() {
        return Err(Skipped::NeedsMoreThanBytes);
    }
    Ok(Rule { category, name, branches, source: source.to_string() })
}

/// Put a condition that runs over several lines back onto one.
///
/// Rules break a long `if` after `&&`, and a line-by-line reading would see
/// half a test twice. Counting brackets says where the condition really ends.
fn join_conditions(text: &str) -> String {
    let mut out = String::new();
    let mut pending: Option<String> = None;
    for raw in text.lines() {
        let line = strip_comment(raw);
        match pending.take() {
            Some(mut open) => {
                open.push(' ');
                open.push_str(line.trim());
                if balanced(&open) {
                    out.push_str(&open);
                    out.push('\n');
                } else {
                    pending = Some(open);
                }
            }
            None => {
                let trimmed = line.trim();
                if (trimmed.starts_with("if (") || trimmed.starts_with("} else if (")) && !balanced(trimmed) {
                    pending = Some(trimmed.to_string());
                } else {
                    out.push_str(line);
                    out.push('\n');
                }
            }
        }
    }
    if let Some(open) = pending {
        out.push_str(&open);
        out.push('\n');
    }
    out
}

/// How much deeper in braces a line leaves things, ignoring braces in strings.
fn braces(line: &str) -> i32 {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    for c in line.chars() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '{') => depth += 1,
            (None, '}') => depth -= 1,
            _ => {}
        }
    }
    depth
}

/// Whether a line closes every bracket it opens, ignoring brackets in strings.
fn balanced(line: &str) -> bool {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    for c in line.chars() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => depth -= 1,
            _ => {}
        }
    }
    depth <= 0
}

fn strip_comment(line: &str) -> &str {
    match line.find("//") {
        Some(i) => &line[..i],
        None => line,
    }
}

/// Lines that carry no test and no answer.
fn is_boilerplate(line: &str) -> bool {
    matches!(line, "{" | "}" | "};" | "} else {" | "else {" | "return result();" | "function detect() {")
        || line.starts_with("meta(")
        || line.starts_with("includeScript(")
}

/// `meta("packer", "UPX");`
fn parse_meta(text: &str) -> Option<(String, String)> {
    let line = text.lines().map(strip_comment).find(|l| l.trim_start().starts_with("meta("))?;
    let mut parts = line.split('"');
    let category = parts.nth(1)?.to_string();
    let name = parts.nth(1)?.to_string();
    Some((category, name))
}

/// The text inside `if (...)`, for a line that opens a test.
///
/// The condition ends at the bracket that closes the `if`, which is not the
/// last bracket on the line: `if (a() && b())` ends three brackets from the
/// end, and taking the last one would cut a call in half.
fn condition_of(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("if (").or_else(|| line.strip_prefix("} else if ("))?;
    let b = rest.as_bytes();
    let mut depth = 0i32;
    let mut quote: Option<u8> = None;
    for (i, c) in b.iter().enumerate() {
        match (quote, *c) {
            (Some(q), _) if *c == q => quote = None,
            (Some(_), _) => {}
            (None, b'"' | b'\'') => quote = Some(*c),
            (None, b'(') => depth += 1,
            (None, b')') => {
                if depth == 0 {
                    return Some(rest[..i].trim());
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

/// Read a whole condition: calls joined by `&&` and `||`, any of them negated.
///
/// `||` binds loosest, so it is split first. A condition holding anything this
/// module does not implement fails as a whole rather than in part, because half
/// a test is not a weaker test, it is a different one.
fn parse_test(cond: &str) -> Option<Test> {
    let cond = unwrap_parens(cond.trim());
    if let Some(parts) = split_top(cond, "||") {
        let tests: Option<Vec<Test>> = parts.iter().map(|p| parse_test(p)).collect();
        return Some(Test::Any(tests?));
    }
    if let Some(parts) = split_top(cond, "&&") {
        let tests: Option<Vec<Test>> = parts.iter().map(|p| parse_test(p)).collect();
        return Some(Test::All(tests?));
    }
    if let Some(rest) = cond.strip_prefix('!') {
        return Some(Test::Not(Box::new(parse_test(rest)?)));
    }
    parse_call(cond)
}

/// Drop brackets that wrap the whole of a condition, so `(a && b)` reads as
/// what is inside it.
fn unwrap_parens(cond: &str) -> &str {
    let mut cond = cond.trim();
    while cond.starts_with('(') && cond.ends_with(')') && balanced(&cond[1..cond.len() - 1]) {
        let inner = cond[1..cond.len() - 1].trim();
        if inner.is_empty() {
            break;
        }
        cond = inner;
    }
    cond
}

/// Split on an operator, but only where it is not inside brackets or a string.
/// `None` means the operator does not appear at that level.
fn split_top(cond: &str, op: &str) -> Option<Vec<String>> {
    let b = cond.as_bytes();
    let mut parts: Vec<String> = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    let mut quote: Option<u8> = None;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, b'"' | b'\'') => quote = Some(c),
            (None, b'(') => depth += 1,
            (None, b')') => depth -= 1,
            (None, _) if depth == 0 && b[i..].starts_with(op.as_bytes()) => {
                parts.push(cond[start..i].to_string());
                i += op.len();
                start = i;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    if parts.is_empty() {
        return None;
    }
    parts.push(cond[start..].to_string());
    Some(parts)
}

/// One call: `MSDOS.compareEP("e9$$$$", 2)`, `PE.isNet()`, and the rest.
fn parse_call(cond: &str) -> Option<Test> {
    let cond = cond.trim();
    if cond == "PE.isNet()" {
        return Some(Test::IsNet);
    }
    if let Some(r) = cond.strip_prefix("PE.isSectionNamePresent(") {
        let start = r.find('"')? + 1;
        let end = r[start..].find('"')? + start;
        // Only a name written out in full. A name the rule works out is a name
        // this module does not know.
        if !r[end + 1..].trim_start().starts_with(')') {
            return None;
        }
        return Some(Test::SectionName(r[start..end].to_string()));
    }
    let (anchor, rest) = if let Some(r) = cond.strip_prefix("Binary.compare(") {
        (Anchor::FileStart, r)
    } else if let Some(r) = cond.strip_prefix("MSDOS.compareEP(") {
        (Anchor::EntryPoint, r)
    } else if let Some(r) = cond.strip_prefix("PE.compareEP(") {
        (Anchor::PeEntryPoint, r)
    } else if let Some(r) = cond.strip_prefix("PE.compareOverlay(") {
        (Anchor::Overlay, r)
    } else {
        return None;
    };
    let start = rest.find('"')? + 1;
    let end = rest[start..].find('"')? + start;
    let pattern = parse_pattern(&rest[start..end])?;
    // Whatever follows the pattern is the optional offset.
    let tail = rest[end + 1..].trim_start().trim_start_matches(',').trim();
    let offset = if tail.is_empty() || tail.starts_with(')') { 0 } else { parse_offset(tail)? };
    Some(Test::Pattern { anchor, offset, pattern })
}

fn parse_offset(tail: &str) -> Option<i64> {
    let t: String = tail.chars().take_while(|c| !matches!(c, ')' | ',' | ' ')).collect();
    let t = t.trim();
    if let Some(h) = t.strip_prefix("0x") {
        return i64::from_str_radix(h, 16).ok();
    }
    t.parse::<i64>().ok()
}

/// `sVersion = "2.1.0";` or `sName += ' N2';`. The middle value says which.
fn assignment_of(line: &str) -> Option<(&str, bool, &str)> {
    let (lhs, rhs) = line.split_once('=')?;
    let (lhs, append) = match lhs.trim_end().strip_suffix('+') {
        Some(l) => (l.trim(), true),
        None => (lhs.trim(), false),
    };
    if lhs.is_empty() || !lhs.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some((lhs, append, rhs.trim().trim_end_matches(';').trim()))
}

/// A plain string, as opposed to something worked out from the file. Rules
/// are written with either quote, and a few use both in the same file.
fn literal(value: &str) -> Option<String> {
    let v = value.trim();
    for q in ['"', '\''] {
        if let Some(inner) = v.strip_prefix(q).and_then(|r| r.strip_suffix(q)) {
            return if inner.contains(q) { None } else { Some(inner.to_string()) };
        }
    }
    None
}

/// Decode a `die_script` signature.
///
/// The forms are documented in `xbinary.h`: hex pairs, `'quoted ASCII'`, `..`
/// and `??` for any byte, `$$` and `##` for an address, and the byte classes
/// `%%`, `!%`, `_%`, `%&` and `**`. `+` asks for a search rather than a test,
/// so a pattern using it has no fixed length and is refused.
pub fn parse_pattern(s: &str) -> Option<Vec<SigByte>> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b' ' => i += 1,
            b'+' => return None,
            b'\'' => {
                i += 1;
                let start = i;
                while i < b.len() && b[i] != b'\'' {
                    out.push(SigByte::Exact(b[i]));
                    i += 1;
                }
                // An unterminated quote means the pattern was not written the
                // way it reads, so nothing here can be trusted.
                if i >= b.len() {
                    let _ = start;
                    return None;
                }
                i += 1;
            }
            _ => {
                if i + 1 >= b.len() {
                    return None;
                }
                let pair = (b[i], b[i + 1]);
                let sb = match pair {
                    (b'.', b'.') | (b'?', b'?') | (b'$', b'$') | (b'#', b'#') => SigByte::Any,
                    (b'%', b'%') => SigByte::Printable,
                    (b'!', b'%') => SigByte::NotPrintable,
                    (b'_', b'%') => SigByte::NotAsciiNotNull,
                    (b'%', b'&') => SigByte::AlphaNumeric,
                    (b'*', b'*') => SigByte::NotNull,
                    (hi, lo) => {
                        let hi = (hi as char).to_digit(16)?;
                        let lo = (lo as char).to_digit(16)?;
                        SigByte::Exact((hi * 16 + lo) as u8)
                    }
                };
                out.push(sb);
                i += 2;
            }
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

/// Where an `MZ` executable starts running, as a file offset.
///
/// The header gives it in paragraphs and words: the code segment is relative to
/// the end of the header, and both are counted in sixteens. Returns nothing
/// when the header is not all there, or when the answer lands outside the file.
pub fn mz_entry_point(head: &[u8], file_len: u64) -> Option<u64> {
    if head.len() < 0x18 || !head.starts_with(b"MZ") {
        return None;
    }
    let word = |at: usize| u64::from(u16::from_le_bytes([head[at], head[at + 1]]));
    let header_paragraphs = word(0x08);
    let cs = word(0x16);
    let ip = word(0x14);
    // `cs` is signed in principle, and packers do use a negative one. Such a
    // file is left alone rather than guessed at.
    if cs >= 0x8000 {
        return None;
    }
    let at = header_paragraphs.checked_add(cs)?.checked_mul(16)?.checked_add(ip)?;
    if at >= file_len { None } else { Some(at) }
}

/// Where a Windows executable starts running, as a file offset.
///
/// The header gives it as an address in memory, so the section table has to
/// turn it back into a place in the file: find the section the address falls
/// in, and count from where that section's bytes actually start. An address
/// before the first section is in the headers, where the two are the same.
///
/// Returns nothing when any of that is missing or does not add up, which is
/// the normal state of a deliberately broken file.
pub fn pe_entry_point(head: &[u8]) -> Option<u64> {
    let word = |at: usize| -> Option<u32> {
        let b = head.get(at..at + 2)?;
        Some(u32::from(u16::from_le_bytes([b[0], b[1]])))
    };
    let long = |at: usize| -> Option<u32> {
        let b = head.get(at..at + 4)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    if !head.starts_with(b"MZ") {
        return None;
    }
    let pe = long(0x3c)? as usize;
    if head.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let sections = word(pe + 6)? as usize;
    let optional_size = word(pe + 20)? as usize;
    let optional = pe + 24;
    let rva = long(optional + 16)?;
    let table = optional.checked_add(optional_size)?;

    let mut best: Option<u64> = None;
    let mut lowest = u32::MAX;
    for i in 0..sections.min(96) {
        let at = table.checked_add(i.checked_mul(40)?)?;
        let virtual_size = long(at + 8)?;
        let virtual_address = long(at + 12)?;
        let raw_size = long(at + 16)?;
        let raw_offset = long(at + 20)?;
        lowest = lowest.min(virtual_address);
        // A section holds whichever of the two sizes is larger: the memory
        // image may be padded past the bytes on disk, and packers rely on it.
        let span = virtual_size.max(raw_size);
        if rva >= virtual_address && rva < virtual_address.saturating_add(span) {
            let into = rva - virtual_address;
            if into >= raw_size {
                // The address is in the part of the section that exists only
                // once loaded, so there are no bytes here to compare.
                return None;
            }
            best = Some(u64::from(raw_offset) + u64::from(into));
        }
    }
    // Before the first section is the headers, where an address and an offset
    // are the same thing.
    if best.is_none() && sections > 0 && rva < lowest {
        best = Some(u64::from(rva));
    }
    best.filter(|at| (*at as usize) < head.len())
}

/// Every rule that recognises these bytes.
///
/// All matches are returned, because a file is often several things at once: a
/// packed Borland executable is both the packer and the compiler.
pub fn detect(db: &Database, head: &[u8], facts: &Facts) -> Vec<Detection> {
    let mut out = Vec::new();
    for rule in &db.rules {
        // The rule's own order decides: a chain of `else if` stops at the
        // first branch that passes.
        let Some(b) = rule.branches.iter().find(|b| holds(&b.test, head, facts)) else { continue };
        out.push(Detection {
            category: rule.category.clone(),
            name: format!("{}{}", b.name.as_deref().unwrap_or(&rule.name), b.name_suffix),
            version: b.version.clone(),
            options: b.options.clone(),
            source: rule.source.clone(),
        });
    }
    out
}

fn holds(test: &Test, head: &[u8], facts: &Facts) -> bool {
    match test {
        Test::All(parts) => parts.iter().all(|t| holds(t, head, facts)),
        Test::Any(parts) => parts.iter().any(|t| holds(t, head, facts)),
        Test::Not(inner) => !holds(inner, head, facts),
        Test::IsNet => facts.is_net,
        Test::SectionName(name) => facts.sections.iter().any(|s| s == name),
        Test::Pattern { anchor, offset, pattern } => {
            let base = match anchor {
                Anchor::FileStart => 0u64,
                // A rule that measures from somewhere the file does not have
                // is not satisfied by measuring from somewhere else.
                Anchor::EntryPoint | Anchor::PeEntryPoint => match facts.entry {
                    Some(e) => e,
                    None => return false,
                },
                Anchor::Overlay => match facts.overlay {
                    Some(o) => o,
                    None => return false,
                },
            };
            let Ok(base) = i64::try_from(base) else { return false };
            let Some(at) = base.checked_add(*offset).filter(|v| *v >= 0) else { return false };
            let at = at as usize;
            let Some(end) = at.checked_add(pattern.len()) else { return false };
            if end > head.len() {
                return false;
            }
            pattern.iter().zip(&head[at..end]).all(|(p, byte)| p.accepts(*byte))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UPX: &str = r#"// Detect It Easy: detection rule file
meta("packer", "UPX");

function detect() {
    if (Binary.compare("'UPX!'")) {
        sVersion = "3.96";
        bDetected = true;
    }

    return result();
}"#;

    const ADA: &str = r#"meta("compiler", "Ada89");

function detect() {
    if (MSDOS.compareEP("e9$$$$8cda")) {
        sOptions = "1989 by RR Software, Inc.";
        bDetected = true;
    }

    return result();
}"#;

    #[test]
    fn reads_a_rule_that_says_everything_in_its_text() {
        let r = parse_rule("upx", UPX).expect("readable");
        assert_eq!(r.category, "packer");
        assert_eq!(r.name, "UPX");
        assert_eq!(r.branches.len(), 1);
        assert_eq!(r.branches[0].version.as_deref(), Some("3.96"));
        let Test::Pattern { anchor, pattern, .. } = &r.branches[0].test else { panic!("a pattern") };
        assert_eq!(*anchor, Anchor::FileStart);
        assert_eq!(pattern, &[SigByte::Exact(b'U'), SigByte::Exact(b'P'), SigByte::Exact(b'X'), SigByte::Exact(b'!')]);
    }

    #[test]
    fn a_rule_measured_from_where_the_file_starts_running() {
        let r = parse_rule("ada", ADA).expect("readable");
        let Test::Pattern { anchor, .. } = &r.branches[0].test else { panic!("a pattern") };
        assert_eq!(*anchor, Anchor::EntryPoint);
        assert_eq!(r.branches[0].options.as_deref(), Some("1989 by RR Software, Inc."));
    }

    #[test]
    fn a_chain_of_tests_keeps_its_order() {
        let text = r#"meta("compiler", "X");
function detect() {
    if (Binary.compare("aa")) {
        sVersion = "1";
        bDetected = true;
    } else if (Binary.compare("bb")) {
        sVersion = "2";
        bDetected = true;
    }
    return result();
}"#;
        let r = parse_rule("x", text).expect("readable");
        assert_eq!(r.branches.len(), 2);
        assert_eq!(r.branches[0].version.as_deref(), Some("1"));
        assert_eq!(r.branches[1].version.as_deref(), Some("2"));
    }

    #[test]
    fn a_branch_may_name_a_variant_of_the_same_tool() {
        let text = "meta(\"packer\", \"Trojan\");\nfunction detect() {\n    if (Binary.compare(\"aa\")) {\n        bDetected = true;\n    } else if (Binary.compare(\"bb\")) {\n        sName += ' N2';\n        sOptions = \"by ZeroCoder\";\n        bDetected = true;\n    }\n    return result();\n}";
        let db = parse_bundle(&format!("{FILE_MARKER}t\n{text}\n"));
        assert_eq!(detect(&db, b"\xaa", &Facts::default())[0].name, "Trojan");
        let second = &detect(&db, b"\xbb", &Facts::default())[0];
        assert_eq!(second.name, "Trojan N2");
        assert_eq!(second.options.as_deref(), Some("by ZeroCoder"));
    }

    #[test]
    fn a_rule_may_cover_a_family_and_name_the_member() {
        let text = "meta(\"cryptor\", \"Cryptor\");\nfunction detect() {\n    if (Binary.compare(\"aa\")) {\n        bDetected = true;\n    } else if (Binary.compare(\"bb\")) {\n        sName = \"crypt 95-97\";\n        bDetected = true;\n    }\n    return result();\n}";
        let db = parse_bundle(&format!("{FILE_MARKER}c\n{text}\n"));
        assert_eq!(detect(&db, b"\xaa", &Facts::default())[0].name, "Cryptor");
        assert_eq!(detect(&db, b"\xbb", &Facts::default())[0].name, "crypt 95-97");
    }

    #[test]
    fn two_tests_joined_are_both_required() {
        let text = "meta(\"packer\", \"P\");\nfunction detect() {\n    if (Binary.compare(\"aa\") && PE.isSectionNamePresent(\".rsrc\")) {\n        bDetected = true;\n    }\n    return result();\n}";
        let db = parse_bundle(&format!("{FILE_MARKER}p\n{text}\n"));
        let with_section = Facts { sections: vec![".rsrc".into()], ..Facts::default() };
        assert_eq!(detect(&db, b"\xaa", &with_section).len(), 1);
        assert!(detect(&db, b"\xaa", &Facts::default()).is_empty(), "the section is missing");
        assert!(detect(&db, b"\xbb", &with_section).is_empty(), "the bytes are wrong");
    }

    #[test]
    fn a_condition_may_run_over_several_lines() {
        let text = "meta(\"packer\", \"P\");\nfunction detect() {\n    if (Binary.compare(\"aa\") &&\n        PE.isNet()) {\n        bDetected = true;\n    }\n    return result();\n}";
        let db = parse_bundle(&format!("{FILE_MARKER}p\n{text}\n"));
        assert_eq!(db.rules.len(), 1, "the rule should be read, not skipped");
        let net = Facts { is_net: true, ..Facts::default() };
        assert_eq!(detect(&db, b"\xaa", &net).len(), 1);
        assert!(detect(&db, b"\xaa", &Facts::default()).is_empty());
    }

    #[test]
    fn either_of_two_tests_will_do() {
        let text = "meta(\"packer\", \"P\");\nfunction detect() {\n    if (Binary.compare(\"aa\") || Binary.compare(\"bb\")) {\n        bDetected = true;\n    }\n    return result();\n}";
        let db = parse_bundle(&format!("{FILE_MARKER}p\n{text}\n"));
        assert_eq!(detect(&db, b"\xaa", &Facts::default()).len(), 1);
        assert_eq!(detect(&db, b"\xbb", &Facts::default()).len(), 1);
        assert!(detect(&db, b"\xcc", &Facts::default()).is_empty());
    }

    #[test]
    fn a_test_may_be_negated() {
        let text = "meta(\"packer\", \"P\");\nfunction detect() {\n    if (Binary.compare(\"aa\") && !PE.isSectionNamePresent(\".rsrc\")) {\n        bDetected = true;\n    }\n    return result();\n}";
        let db = parse_bundle(&format!("{FILE_MARKER}p\n{text}\n"));
        assert_eq!(detect(&db, b"\xaa", &Facts::default()).len(), 1);
        let with_section = Facts { sections: vec![".rsrc".into()], ..Facts::default() };
        assert!(detect(&db, b"\xaa", &with_section).is_empty());
    }

    #[test]
    fn the_overlay_starts_past_the_last_section() {
        let mut head = pe_sample(0x1010);
        head.resize(0x400, 0);
        // The one section holds 0x100 bytes from 0x180, so anything past 0x280
        // is overlay.
        let f = Facts::of(&head, 0x400);
        assert_eq!(f.overlay, Some(0x280));
        assert_eq!(f.sections, [".text"]);
    }

    #[test]
    fn a_test_inside_a_branch_is_not_a_branch_of_its_own() {
        // AZProtect's shape: matching sets a flag, and a second test inside
        // that only refines the answer. Reading the inner test as an
        // alternative would have it match every file without that section.
        let text = "meta(\"protector\", \"AZProtect\");\nfunction detect() {\n    if (PE.compareEP(\"eb70\")) {\n        bDetected = true;\n\n        if (!PE.isSectionNamePresent(\"AZPR0001\")) {\n            sOptions = \"modified\";\n        }\n    }\n    return result();\n}";
        assert_eq!(parse_rule("az", text), Err(Skipped::NeedsMoreThanBytes));
    }

    #[test]
    fn a_rule_that_needs_more_than_bytes_is_skipped() {
        let text = r#"meta("packer", "UPX");
function detect() {
    if (PE.getNumberOfImportThunks(0) > 1) {
        bDetected = true;
    }
    return result();
}"#;
        assert_eq!(parse_rule("x", text), Err(Skipped::NeedsMoreThanBytes));
    }

    #[test]
    fn a_rule_that_works_out_its_answer_is_skipped_rather_than_half_read() {
        let text = r#"meta("archive", "7-Zip");
function detect() {
    if (Binary.compare("'7z'")) {
        sVersion = Binary.readByte(6) + "." + Binary.readByte(7);
        bDetected = true;
    }
    return result();
}"#;
        assert_eq!(parse_rule("x", text), Err(Skipped::ComputedAnswer));
    }

    #[test]
    fn every_form_the_pattern_language_has() {
        assert_eq!(parse_pattern("4d5a"), Some(vec![SigByte::Exact(0x4d), SigByte::Exact(0x5a)]));
        assert_eq!(parse_pattern("'MZ'"), Some(vec![SigByte::Exact(b'M'), SigByte::Exact(b'Z')]));
        assert_eq!(parse_pattern(".. ?? $$ ##"), Some(vec![SigByte::Any; 4]));
        assert_eq!(parse_pattern("%%"), Some(vec![SigByte::Printable]));
        assert_eq!(parse_pattern("!%"), Some(vec![SigByte::NotPrintable]));
        assert_eq!(parse_pattern("_%"), Some(vec![SigByte::NotAsciiNotNull]));
        assert_eq!(parse_pattern("%&"), Some(vec![SigByte::AlphaNumeric]));
        assert_eq!(parse_pattern("**"), Some(vec![SigByte::NotNull]));
        // Uppercase hex, and spaces that mean nothing.
        assert_eq!(parse_pattern("4D 5A"), Some(vec![SigByte::Exact(0x4d), SigByte::Exact(0x5a)]));
    }

    #[test]
    fn a_pattern_that_searches_has_no_fixed_place() {
        assert_eq!(parse_pattern("++4D5A"), None);
    }

    #[test]
    fn byte_classes_accept_what_they_say() {
        assert!(SigByte::Printable.accepts(b'A') && !SigByte::Printable.accepts(0x01));
        assert!(SigByte::NotPrintable.accepts(0x01) && !SigByte::NotPrintable.accepts(b'A'));
        assert!(SigByte::AlphaNumeric.accepts(b'7') && !SigByte::AlphaNumeric.accepts(b'-'));
        assert!(SigByte::NotNull.accepts(1) && !SigByte::NotNull.accepts(0));
        assert!(SigByte::NotAsciiNotNull.accepts(0x80) && !SigByte::NotAsciiNotNull.accepts(b'A'));
    }

    #[test]
    fn a_bundle_keeps_each_file_apart() {
        let bundle = format!("{FILE_MARKER}upx\n{UPX}\n{FILE_MARKER}ada\n{ADA}\n");
        let db = parse_bundle(&bundle);
        assert_eq!(db.rules.len(), 2);
        assert_eq!(db.rules[0].source, "upx");
        assert_eq!(db.rules[1].source, "ada");
    }

    #[test]
    fn finds_what_is_there_and_says_where_it_came_from() {
        let db = parse_bundle(&format!("{FILE_MARKER}upx\n{UPX}\n"));
        let found = detect(&db, b"UPX!rest of the file", &Facts::default());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "UPX");
        assert_eq!(found[0].version.as_deref(), Some("3.96"));
        assert_eq!(found[0].source, "upx");
        assert!(detect(&db, b"not upx at all", &Facts::default()).is_empty());
    }

    #[test]
    fn an_entry_point_rule_needs_an_entry_point() {
        let db = parse_bundle(&format!("{FILE_MARKER}ada\n{ADA}\n"));
        let mut head = vec![0u8; 64];
        head[16..21].copy_from_slice(&[0xe9, 0x11, 0x22, 0x8c, 0xda]);
        // Without one, the rule is not tested rather than tested at zero.
        assert!(detect(&db, &head, &Facts::default()).is_empty());
        assert_eq!(detect(&db, &head, &Facts { entry: Some(16), ..Facts::default() }).len(), 1);
    }

    /// A PE with one section, whose entry point is inside it.
    fn pe_sample(rva: u32) -> Vec<u8> {
        let mut v = vec![0u8; 0x200];
        v[0..2].copy_from_slice(b"MZ");
        let pe = 0x80usize;
        v[0x3c..0x40].copy_from_slice(&(pe as u32).to_le_bytes());
        v[pe..pe + 4].copy_from_slice(b"PE\0\0");
        v[pe + 6..pe + 8].copy_from_slice(&1u16.to_le_bytes()); // one section
        v[pe + 20..pe + 22].copy_from_slice(&0xe0u16.to_le_bytes()); // optional size
        let optional = pe + 24;
        v[optional + 16..optional + 20].copy_from_slice(&rva.to_le_bytes());
        let table = optional + 0xe0;
        v[table..table + 5].copy_from_slice(b".text");
        v[table + 8..table + 12].copy_from_slice(&0x1000u32.to_le_bytes()); // virtual size
        v[table + 12..table + 16].copy_from_slice(&0x1000u32.to_le_bytes()); // virtual address
        v[table + 16..table + 20].copy_from_slice(&0x100u32.to_le_bytes()); // raw size
        v[table + 20..table + 24].copy_from_slice(&0x180u32.to_le_bytes()); // raw offset
        v
    }

    #[test]
    fn the_entry_point_of_a_pe_comes_back_through_its_section_table() {
        // 0x1010 is 0x10 into a section whose bytes start at 0x180.
        assert_eq!(pe_entry_point(&pe_sample(0x1010)), Some(0x190));
    }

    #[test]
    fn an_entry_point_in_the_headers_is_already_a_file_offset() {
        assert_eq!(pe_entry_point(&pe_sample(0x40)), Some(0x40));
    }

    #[test]
    fn an_entry_point_with_no_bytes_behind_it_is_no_answer() {
        // Past the section's bytes on disk, in the part that exists only once
        // the loader has made room for it.
        assert_eq!(pe_entry_point(&pe_sample(0x1500)), None);
        // Not a PE at all.
        assert_eq!(pe_entry_point(b"MZ and nothing else"), None);
    }

    #[test]
    fn the_entry_point_of_an_mz_comes_out_of_its_header() {
        let mut head = vec![0u8; 64];
        head[0..2].copy_from_slice(b"MZ");
        head[0x08..0x0a].copy_from_slice(&4u16.to_le_bytes()); // header is 4 paragraphs
        head[0x14..0x16].copy_from_slice(&0x10u16.to_le_bytes()); // ip
        head[0x16..0x18].copy_from_slice(&2u16.to_le_bytes()); // cs
        // (4 + 2) * 16 + 0x10
        assert_eq!(mz_entry_point(&head, 4096), Some(0x70));
    }

    #[test]
    fn an_entry_point_outside_the_file_is_no_entry_point() {
        let mut head = vec![0u8; 64];
        head[0..2].copy_from_slice(b"MZ");
        head[0x08..0x0a].copy_from_slice(&0xfffu16.to_le_bytes());
        assert_eq!(mz_entry_point(&head, 512), None);
        // A packer's negative code segment is left alone.
        head[0x16..0x18].copy_from_slice(&0xf000u16.to_le_bytes());
        assert_eq!(mz_entry_point(&head, 1 << 30), None);
    }
}
