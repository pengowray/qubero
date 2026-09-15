//! BIFF8 cell annotations. Formula tokens are read in reverse Polish order;
//! this renders expressions, never recalculates them or executes macros.
use crate::template::{Deduce, Deduced, Deducer};
use std::{collections::HashMap, sync::Arc};

#[derive(Debug)]
pub struct Cells;
struct Reading(HashMap<u64, String>);
impl Deduced for Reading {
    fn int(&self, _: Deduce, _: u64) -> Option<i128> {
        None
    }
    fn text(&self, _: Deduce, at: u64) -> Option<String> {
        self.0.get(&at).cloned()
    }
}
impl Deducer for Cells {
    fn run(&self, bytes: &[u8]) -> Arc<dyn Deduced> {
        let unpacked;
        let bytes = if bytes.starts_with(crate::codec::cfb::MAGIC) {
            unpacked = crate::codec::cfb::workbook(bytes).unwrap_or_default();
            &unpacked
        } else {
            bytes
        };
        Arc::new(Reading(read(bytes)))
    }
}
fn word(b: &[u8], p: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(p..p + 2)?.try_into().ok()?))
}
fn dword(b: &[u8], p: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(p..p + 4)?.try_into().ok()?))
}
fn number(b: &[u8], p: usize) -> Option<f64> {
    Some(f64::from_le_bytes(b.get(p..p + 8)?.try_into().ok()?))
}
fn error(n: u8) -> &'static str {
    match n {
        0 => "#NULL!",
        7 => "#DIV/0!",
        15 => "#VALUE!",
        23 => "#REF!",
        29 => "#NAME?",
        36 => "#NUM!",
        42 => "#N/A",
        _ => "unknown error",
    }
}
fn boolean(n: u8) -> &'static str {
    if n == 0 { "FALSE" } else { "TRUE" }
}
fn address(row: u16, col: u16) -> String {
    let mut n = col as u32 + 1;
    let mut s = String::new();
    while n > 0 {
        n -= 1;
        s.insert(0, (b'A' + (n % 26) as u8) as char);
        n /= 26;
    }
    format!("{s}{}", row as u32 + 1)
}
fn reference(row: u16, flags: u16) -> String {
    let a = address(row, flags & 0x3fff);
    let split = a.find(|c: char| c.is_ascii_digit()).unwrap();
    format!(
        "{}{}{}{}",
        if flags & 0x4000 == 0 { "$" } else { "" },
        &a[..split],
        if flags & 0x8000 == 0 { "$" } else { "" },
        &a[split..]
    )
}
fn rk(n: u32) -> f64 {
    let value = if n & 2 != 0 {
        ((n as i32) >> 2) as f64
    } else {
        f64::from_bits(((n & !3) as u64) << 32)
    };
    if n & 1 != 0 { value / 100.0 } else { value }
}

/// A string cursor retains record boundaries. CONTINUE adds an encoding
/// flag only when it interrupts character data, not lengths or rich runs.
struct Strings<'a> {
    parts: Vec<&'a [u8]>,
    part: usize,
    at: usize,
}
impl<'a> Strings<'a> {
    fn byte(&mut self) -> Option<u8> {
        while self.at == self.parts.get(self.part)?.len() {
            self.part += 1;
            self.at = 0;
        }
        let v = *self.parts.get(self.part)?.get(self.at)?;
        self.at += 1;
        Some(v)
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes([self.byte()?, self.byte()?]))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes([
            self.byte()?,
            self.byte()?,
            self.byte()?,
            self.byte()?,
        ]))
    }
    fn string(&mut self, short: bool, rich: bool) -> Option<String> {
        let n = if short {
            self.byte()? as usize
        } else {
            self.u16()? as usize
        };
        let flags = self.byte()?;
        let runs = if rich && flags & 8 != 0 {
            self.u16()? as usize
        } else {
            0
        };
        let ext = if rich && flags & 4 != 0 {
            self.u32()? as usize
        } else {
            0
        };
        let mut wide = flags & 1 != 0;
        let mut chars = Vec::with_capacity(n);
        for _ in 0..n {
            if self.at == self.parts.get(self.part)?.len() {
                self.part += 1;
                self.at = 0;
                wide = self.byte()? & 1 != 0;
            }
            // BIFF does not split a UTF-16 code unit between records.
            if wide && self.parts.get(self.part)?.len().saturating_sub(self.at) < 2 {
                return None;
            }
            chars.push(if wide {
                self.u16()?
            } else {
                self.byte()? as u16
            });
        }
        for _ in 0..runs.checked_mul(4)?.checked_add(ext)? {
            self.byte()?;
        }
        String::from_utf16(&chars).ok()
    }
}
fn string(b: &[u8], short: bool) -> Option<String> {
    Strings {
        parts: vec![b],
        part: 0,
        at: 0,
    }
    .string(short, false)
}

fn read(bytes: &[u8]) -> HashMap<u64, String> {
    let mut out = HashMap::new();
    let mut records = Vec::new();
    let mut at = 0;
    while let (Some(id), Some(n)) = (word(bytes, at), word(bytes, at + 2)) {
        let end = at + 4 + n as usize;
        let Some(body) = bytes.get(at + 4..end) else {
            break;
        };
        records.push((at, id, body));
        at = end;
    }
    if !matches!(records.first(), Some((_, 0x0809, b)) if word(b, 0) == Some(0x0600)) {
        return out;
    }
    if let Some((at, _, _)) = records.iter().find(|(_, id, _)| *id == 0x002f) {
        out.insert(
            *at as u64,
            "Encrypted workbook; cell decoding unavailable".into(),
        );
        return out;
    }
    let mut strings = Vec::new();
    let mut sheets = HashMap::new();
    let mut sheet_names = Vec::new();
    let mut supbooks = Vec::new();
    let mut externs = Vec::new();
    let mut shared = HashMap::new();
    let mut substream = 0;
    for (i, &(_, id, b)) in records.iter().enumerate() {
        if id == 0x0085 {
            if let Some((pos, name)) = dword(b, 0).zip(b.get(6..).and_then(|s| string(s, true))) {
                sheet_names.push(name.clone());
                sheets.insert(pos as usize, name);
            }
        }
        if id == 0x01ae {
            supbooks.push(word(b, 2) == Some(0x0401));
        }
        if id == 0x0017 && b.len() >= 2 {
            for e in b[2..].chunks_exact(6).take(word(b, 0).unwrap() as usize) {
                externs.push((
                    word(e, 0).unwrap(),
                    word(e, 2).unwrap(),
                    word(e, 4).unwrap(),
                ));
            }
        }
        if id == 0x0809 {
            substream = records[i].0;
        }
        if id == 0x04bc && b.len() >= 10 {
            if let Some((_, 6, previous)) = i.checked_sub(1).and_then(|p| records.get(p)) {
                if let Some(tokens) = b.get(10..10 + word(b, 8).unwrap() as usize) {
                    if let Some((row, col)) = word(previous, 0).zip(word(previous, 2)) {
                        shared.insert((substream, row, col), (b, tokens));
                    }
                }
            }
        }
        if id == 0x00fc && b.len() >= 8 {
            let mut parts = vec![&b[8..]];
            parts.extend(
                records[i + 1..]
                    .iter()
                    .take_while(|(_, id, _)| *id == 0x003c)
                    .map(|(_, _, b)| *b),
            );
            let mut cursor = Strings {
                parts,
                part: 0,
                at: 0,
            };
            for _ in 0..dword(b, 4).unwrap_or(0).min(1_000_000) {
                let Some(s) = cursor.string(false, true) else {
                    break;
                };
                strings.push(s);
            }
        }
    }
    let mut sheet = String::new();
    let mut worksheet = false;
    let external_sheets: Vec<Option<String>> = externs
        .iter()
        .map(|&(book, first, last)| {
            if supbooks.get(book as usize) != Some(&true) {
                return None;
            }
            let a = sheet_names.get(first as usize)?;
            let name = if first == last {
                a.clone()
            } else {
                format!("{a}:{}", sheet_names.get(last as usize)?)
            };
            Some(format!("'{}'", name.replace('\'', "''")))
        })
        .collect();
    for (i, &(at, id, b)) in records.iter().enumerate() {
        if id == 0x0809 {
            substream = at;
            worksheet = word(b, 2) == Some(16);
            sheet = sheets.get(&at).cloned().unwrap_or_default();
        }
        if id == 0x000a {
            worksheet = false;
        }
        if !worksheet {
            continue;
        }
        let Some((row, col)) = word(b, 0).zip(word(b, 2)) else {
            continue;
        };
        let value = match id {
            0x0203 => number(b, 6).map(|v| v.to_string()),
            0x027e => dword(b, 6).map(|v| rk(v).to_string()),
            0x00fd => dword(b, 6).map(|v| {
                strings
                    .get(v as usize)
                    .map(|s| format!("{s:?}"))
                    .unwrap_or_else(|| format!("unresolved shared string #{v}"))
            }),
            0x0204 => b
                .get(6..)
                .and_then(|s| string(s, false))
                .map(|s| format!("{s:?}")),
            0x0205 => b.get(6..8).map(|v| {
                if v[1] == 0 {
                    boolean(v[0])
                } else {
                    error(v[0])
                }
                .to_string()
            }),
            0x0201 => Some("blank".into()),
            0x00bd => {
                let mut values = Vec::new();
                if b.len() < 6 || (b.len() - 6) % 6 != 0 {
                    continue;
                }
                for (j, c) in b[4..b.len() - 2].chunks_exact(6).enumerate() {
                    values.push(format!(
                        "{}: {}",
                        address(row, col.saturating_add(j as u16)),
                        rk(dword(c, 2).unwrap())
                    ));
                }
                Some(values.join(", "))
            }
            0x0006 if b.len() >= 22 => {
                let cached = if word(b, 12) == Some(0xffff) {
                    match b[6] {
                        0 => records
                            .get(i + 1)
                            .filter(|(_, id, _)| *id == 0x0207)
                            .and_then(|(_, _, s)| string(s, false))
                            .map(|s| format!("{s:?}"))
                            .unwrap_or_else(|| "string result follows".into()),
                        1 => boolean(b[8]).into(),
                        2 => error(b[8]).into(),
                        3 => "empty".into(),
                        _ => "unknown cached result".into(),
                    }
                } else {
                    number(b, 6).unwrap().to_string()
                };
                let tokens = b.get(22..22 + word(b, 20).unwrap() as usize);
                let tokens = tokens.and_then(|t| {
                    if t.first() != Some(&1) {
                        return Some(t);
                    }
                    if t.len() != 5 || word(b, 14)? & 8 == 0 {
                        return None;
                    }
                    let (range, body) = shared.get(&(substream, word(t, 1)?, word(t, 3)?))?;
                    if row < word(range, 0)?
                        || row > word(range, 2)?
                        || col < range[4] as u16
                        || col > range[5] as u16
                    {
                        return None;
                    }
                    Some(*body)
                });
                let formula = tokens
                    .and_then(|t| formula_at(t, row, col, &external_sheets))
                    .unwrap_or_else(|| "[unsupported or truncated formula; see tokens]".into());
                Some(format!("{formula} (cached: {cached})"))
            }
            _ => None,
        };
        if let Some(v) = value {
            if id == 0x00bd {
                out.insert(
                    at as u64,
                    format!("{}{}{v}", sheet, if sheet.is_empty() { "" } else { "!" }),
                );
                continue;
            }
            out.insert(
                at as u64,
                format!(
                    "{}{}{}: {v}",
                    sheet,
                    if sheet.is_empty() { "" } else { "!" },
                    address(row, col)
                ),
            );
        }
    }
    out
}

fn function(id: u16) -> Option<(&'static str, usize)> {
    Some(match id {
        0 => ("COUNT", 1),
        1 => ("IF", 3),
        4 => ("SUM", 1),
        5 => ("AVERAGE", 1),
        6 => ("MIN", 1),
        7 => ("MAX", 1),
        15 => ("SIN", 1),
        16 => ("COS", 1),
        17 => ("TAN", 1),
        20 => ("SQRT", 1),
        24 => ("ABS", 1),
        25 => ("INT", 1),
        27 => ("ROUND", 2),
        32 => ("LEN", 1),
        34 => ("TRUE", 0),
        35 => ("FALSE", 0),
        36 => ("AND", 1),
        37 => ("OR", 1),
        38 => ("NOT", 1),
        74 => ("NOW", 0),
        169 => ("COUNTA", 1),
        _ => return None,
    })
}
fn call(stack: &mut Vec<String>, id: u16, argc: Option<usize>) -> Option<()> {
    let (name, arity) = function(id)?;
    let n = argc.unwrap_or(arity);
    let args = stack.split_off(stack.len().checked_sub(n)?);
    stack.push(format!("{name}({})", args.join(",")));
    Some(())
}
#[cfg(test)]
fn formula(b: &[u8]) -> Option<String> {
    formula_at(b, 0, 0, &[])
}
fn relative(row: u16, flags: u16, base_row: u16, base_col: u16) -> String {
    let row = if flags & 0x8000 != 0 {
        base_row.wrapping_add(row)
    } else {
        row
    };
    let col = if flags & 0x4000 != 0 {
        base_col.wrapping_add(flags & 0xff) & 0xff
    } else {
        flags & 0xff
    };
    reference(row, col | (flags & 0xc000))
}
fn formula_at(b: &[u8], row: u16, col: u16, sheets: &[Option<String>]) -> Option<String> {
    let mut at = 0;
    let mut stack: Vec<String> = Vec::new();
    while let Some(&raw) = b.get(at) {
        if raw >= 0x80 {
            return None;
        }
        at += 1;
        let op = if raw >= 0x20 {
            (raw & 0x1f) | 0x20
        } else {
            raw
        };
        match op {
            0x03..=0x11 => {
                let operator = [
                    "+", "-", "*", "/", "^", "&", "<", "<=", "=", ">=", ">", "<>", " ", ",", ":",
                ][(op - 3) as usize];
                let right = stack.pop()?;
                let left = stack.pop()?;
                stack.push(format!("({left}{operator}{right})"));
            }
            0x12..=0x15 => {
                let v = stack.pop()?;
                stack.push(match op {
                    0x12 => format!("(+{v})"),
                    0x13 => format!("(-{v})"),
                    0x14 => format!("({v}%)"),
                    _ => format!("({v})"),
                });
            }
            0x16 => stack.push(String::new()),
            0x17 => {
                let n = *b.get(at)? as usize;
                let wide = *b.get(at + 1)? & 1 != 0;
                let end = at + 2 + n * if wide { 2 } else { 1 };
                let s = string(b.get(at..end)?, true)?;
                stack.push(format!("\"{}\"", s.replace('"', "\"\"")));
                at = end;
            }
            0x19 => {
                // The SUM optimization is equivalent to one-argument SUM.
                // Control-flow attributes require a fuller formula reader.
                let flags = *b.get(at)?;
                b.get(at..at + 3)?;
                match flags {
                    0x10 => call(&mut stack, 4, Some(1))?,
                    0x01 | 0x40 | 0x41 => (),
                    _ => return None,
                }
                at += 3;
            }
            0x1c => {
                stack.push(error(*b.get(at)?).into());
                at += 1;
            }
            0x1d => {
                stack.push(boolean(*b.get(at)?).into());
                at += 1;
            }
            0x1e => {
                stack.push(word(b, at)?.to_string());
                at += 2;
            }
            0x1f => {
                stack.push(number(b, at)?.to_string());
                at += 8;
            }
            0x21 => {
                call(&mut stack, word(b, at)?, None)?;
                at += 2;
            }
            0x22 => {
                call(&mut stack, word(b, at + 1)?, Some(*b.get(at)? as usize))?;
                at += 3;
            }
            0x24 => {
                stack.push(reference(word(b, at)?, word(b, at + 2)?));
                at += 4;
            }
            0x25 => {
                stack.push(format!(
                    "{}:{}",
                    reference(word(b, at)?, word(b, at + 4)?),
                    reference(word(b, at + 2)?, word(b, at + 6)?)
                ));
                at += 8;
            }
            0x2c => {
                stack.push(relative(word(b, at)?, word(b, at + 2)?, row, col));
                at += 4;
            }
            0x2d => {
                stack.push(format!(
                    "{}:{}",
                    relative(word(b, at)?, word(b, at + 4)?, row, col),
                    relative(word(b, at + 2)?, word(b, at + 6)?, row, col)
                ));
                at += 8;
            }
            0x3a => {
                let sheet = sheets.get(word(b, at)? as usize)?.as_ref()?;
                stack.push(format!(
                    "{sheet}!{}",
                    reference(word(b, at + 2)?, word(b, at + 4)?)
                ));
                at += 6;
            }
            0x3b => {
                let sheet = sheets.get(word(b, at)? as usize)?.as_ref()?;
                stack.push(format!(
                    "{sheet}!{}:{}",
                    reference(word(b, at + 2)?, word(b, at + 6)?),
                    reference(word(b, at + 4)?, word(b, at + 8)?)
                ));
                at += 10;
            }
            _ => return None,
        }
        // A malicious RPN program can repeatedly copy a growing expression.
        if stack.last().is_some_and(|s| s.len() > 65_536) {
            return None;
        }
    }
    if stack.len() == 1 {
        Some(format!("={}", stack.pop()?))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::Document,
        eval::{Evaluator, Value},
        source::MemSource,
    };
    fn record(out: &mut Vec<u8>, id: u16, b: &[u8]) -> usize {
        let at = out.len();
        out.extend(id.to_le_bytes());
        out.extend((b.len() as u16).to_le_bytes());
        out.extend(b);
        at
    }
    fn bof(out: &mut Vec<u8>, kind: u16) {
        let mut b = vec![0; 16];
        b[..2].copy_from_slice(&0x600u16.to_le_bytes());
        b[2..4].copy_from_slice(&kind.to_le_bytes());
        record(out, 0x809, &b);
    }
    fn cell(row: u16, col: u16, value: &[u8]) -> Vec<u8> {
        [
            row.to_le_bytes().as_slice(),
            col.to_le_bytes().as_slice(),
            &[0, 0],
            value,
        ]
        .concat()
    }
    #[test]
    fn numbers_strings_and_formulae_reach_the_template() {
        let mut b = Vec::new();
        bof(&mut b, 5);
        record(&mut b, 0xfc, &[1, 0, 0, 0, 1, 0, 0, 0, 3, 0, 0, b'c']);
        record(&mut b, 0x3c, &[1, b'a', 0, b't', 0]);
        record(&mut b, 0xa, &[]);
        bof(&mut b, 16);
        let n = record(&mut b, 0x203, &cell(0, 0, &42.5f64.to_le_bytes()));
        let s = record(&mut b, 0xfd, &cell(1, 0, &0u32.to_le_bytes()));
        let mut f = cell(2, 0, &85f64.to_le_bytes());
        f.extend([0; 6]);
        let tokens = [0x44, 0, 0, 0, 0xc0, 0x1e, 2, 0, 0x05];
        f.extend((tokens.len() as u16).to_le_bytes());
        f.extend(tokens);
        let pos = record(&mut b, 6, &f);
        record(&mut b, 0xa, &[]);
        let result = read(&b);
        assert_eq!(result[&(n as u64)], "A1: 42.5");
        assert_eq!(result[&(s as u64)], "A2: \"cat\"");
        assert_eq!(result[&(pos as u64)], "A3: =(A1*2) (cached: 85)");
        // Walk every visible node, including decoded address spaces. This
        // catches a parser that works alone but is disconnected from the UI.
        for file in [
            b.clone(),
            crate::codec::cfb::tests::fixture(&b, true, false),
        ] {
            let d = Document::new(MemSource(file));
            let mut ev = Evaluator::new(super::super::xls());
            let mut pending = vec![vec![]];
            let mut texts = Vec::new();
            while let Some(path) = pending.pop() {
                let node = ev
                    .node(&d, &path)
                    .unwrap_or_else(|e| panic!("{path:?}: {e:?}"));
                if let Value::Str(s) = node.value {
                    texts.push(s);
                }
                for i in 0..node.child_count as usize {
                    let mut p = path.clone();
                    p.push(i);
                    pending.push(p);
                }
            }
            assert!(
                texts.iter().any(|s| s == "A3: =(A1*2) (cached: 85)"),
                "{texts:?}"
            );
        }
    }
    #[test]
    fn formulas_have_references_functions_and_honest_fallbacks() {
        assert_eq!(
            formula(&[0x44, 4, 0, 1, 0xc0, 0x1e, 2, 0, 5, 0x41, 20, 0]).as_deref(),
            Some("=SQRT((B5*2))")
        );
        assert_eq!(
            formula(&[0x25, 0, 0, 2, 0, 0, 0, 1, 0, 0x19, 0x10, 0, 0]).as_deref(),
            Some("=SUM($A$1:$B$3)")
        );
        assert_eq!(formula(&[0x01, 0, 0, 0, 0]), None); // shared formula
        for b in [&[0x1f, 0][..], &[3][..], &[0x1e, 1, 0, 0x1e, 2, 0][..]] {
            assert_eq!(formula(b), None);
        }
    }
    #[test]
    fn rk_and_rich_string_continuations() {
        assert_eq!(rk(((-12345i32 as u32) << 2) | 3), -123.45);
        assert_eq!(rk((1.5f64.to_bits() >> 32) as u32), 1.5);
        let mut s = Strings {
            parts: vec![
                &[2, 0, 8, 1, 0, b'a'],
                &[0, b'b', 0, 0],
                &[0, 0, 1, 0, 0, b'z'],
            ],
            part: 0,
            at: 0,
        };
        assert_eq!(s.string(false, true).as_deref(), Some("ab"));
        assert_eq!(s.string(false, true).as_deref(), Some("z"));
    }
    #[test]
    fn shared_formula_offsets_follow_the_cell_using_them() {
        let mut b = Vec::new();
        bof(&mut b, 16);
        let mut positions = Vec::new();
        for row in [1u16, 2] {
            let mut f = cell(row, 3, &6f64.to_le_bytes());
            f.extend([8, 0, 0, 0, 0, 0, 5, 0, 1, 1, 0, 3, 0]);
            positions.push(record(&mut b, 6, &f));
            if row == 1 {
                // B(row)*C(row), both columns relative to column D.
                let tokens = [0x4c, 0, 0, 0xfe, 0xff, 0x4c, 0, 0, 0xff, 0xff, 5];
                let mut shared = vec![1, 0, 2, 0, 3, 3, 0, 2, 11, 0];
                shared.extend(tokens);
                record(&mut b, 0x4bc, &shared);
            }
        }
        record(&mut b, 0xa, &[]);
        let cells = read(&b);
        assert_eq!(cells[&(positions[0] as u64)], "D2: =(B2*C2) (cached: 6)");
        assert_eq!(cells[&(positions[1] as u64)], "D3: =(B3*C3) (cached: 6)");
        assert_eq!(formula_at(&[0x3a, 0, 0, 0, 0, 0, 0], 0, 0, &[None]), None);
    }
    #[test]
    fn encrypted_and_truncated_workbooks_do_not_invent_cells() {
        let mut b = Vec::new();
        bof(&mut b, 5);
        let p = record(&mut b, 0x2f, &[0, 0]);
        bof(&mut b, 16);
        record(&mut b, 0x203, &cell(0, 0, &1f64.to_le_bytes()));
        let r = read(&b);
        assert_eq!(r.len(), 1);
        assert!(r[&(p as u64)].contains("Encrypted"));
        for len in 0..b.len() {
            let _ = read(&b[..len]);
        }
    }

    #[test]
    fn excel_and_libreoffice_biff8_samples() {
        let samples = std::env::var_os("QUBERO_SAMPLES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../../qubero-samples"
                ))
            });
        for name in ["excel16-biff8.xls", "libreoffice7-biff8.xls"] {
            let path = samples.join("xls").join(name);
            let Ok(b) = std::fs::read(&path) else {
                eprintln!("skipped {}", path.display());
                continue;
            };
            let workbook = crate::codec::cfb::workbook(&b).unwrap();
            let cells = read(&workbook);
            for expected in [
                "Data!A3: \"Café\"",
                "Data!D2: =(B2*C2) (cached: 37.5)",
                "Data!D3: =(B3*C3) (cached: 12)",
                "Data!D4: =(B4*C4) (cached: 16)",
                "Summary!B3: =SUM('Data'!D2:D4) (cached: 65.5)",
                "Summary!B4: =COUNTA('Data'!A2:A4) (cached: 3)",
            ] {
                assert!(
                    cells.values().any(|v| v == expected),
                    "{name}: missing {expected}"
                );
            }
            assert!(!cells.values().any(|v| v.contains("unsupported")), "{name}");
        }
    }
}
