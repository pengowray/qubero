//! Turning bytes into values: how far a field runs, and what it says.

use super::*;

/// How far into a scanned field the value starts: any byte in `skip`, and a
/// comment running from one byte to another when the format has them. Kept in
/// one place because two callers need the same answer, one measuring the field
/// and one saying where inside it the value sits, and an answer that differed
/// between them would put the value in the wrong place.
///
/// The one place they can still differ is a comment longer than the 4096 bytes
/// an editable field is read up to: the measuring reads on, the placing stops,
/// and the value lands short. Nothing writes a comment that long.
#[derive(Default, Clone, Copy)]
pub(super) struct Skipping {
    in_comment: bool,
}

impl Skipping {
    /// Whether this byte is still part of the run before the value. The byte
    /// that ends a comment belongs to it, and what follows may be more
    /// separators, so this keeps its state between calls and across the blocks
    /// a long field is read in.
    fn steps_over(&mut self, b: u8, skip: &[u8], comment: Option<(u8, u8)>) -> bool {
        if self.in_comment {
            if comment.is_some_and(|(_, end)| b == end) {
                self.in_comment = false;
            }
            return true;
        }
        if comment.is_some_and(|(start, _)| b == start) {
            self.in_comment = true;
            return true;
        }
        skip.contains(&b)
    }
}

/// Index of `term` in `hay`, aligned to whole units of its length.
pub(super) fn find_unit(hay: &[u8], term: &[u8]) -> Option<usize> {
    let unit = term.len();
    (0..hay.len().saturating_sub(unit - 1)).step_by(unit).find(|i| hay[*i..*i + unit] == *term)
}

impl Evaluator {
    pub(super) fn read<S: Source>(&self, doc: &Document<S>, r: &Resolved, at: u64, n: u64) -> R<Vec<u8>> {
        if at + n > r.limit {
            return fail("runs past the end of its container");
        }
        let mut buf = vec![0u8; bytes_for(n)];
        let missing = doc.read_bits(at, n, &mut buf);
        if missing.is_empty() {
            Ok(buf)
        } else {
            Err(EvalError::Pending(missing))
        }
    }

    /// Where the value sits inside a text field, and how the bytes read.
    ///
    /// A byte-order mark belongs to the field but not to the value, and the
    /// padding or terminator ends it. Everything is measured in whole code
    /// units, so UTF-16LE text does not stop at the first zero byte of "H".
    pub(super) fn str_span<S: Source>(&self, doc: &Document<S>, r: &Resolved, size: u64) -> R<Option<StrSpan>> {
        let Ty::Str { len, enc } = &r.ty else { return Ok(None) };
        let n = size / 8;
        let cap = n.min(crate::encode::EDIT_LIMIT_BYTES);
        let bytes = if cap == 0 { Vec::new() } else { self.read(doc, r, r.offset, cap * 8)? };
        let (settled, bom, note) = text::settle(enc, &bytes);
        let bom = (bom as u64).min(cap);
        let body = &bytes[bom as usize..];
        let unit = settled.unit();
        let rest = cap - bom;
        // `skipped` is what sits before the value and is not part of it: the
        // separators a scanned field stepped over, on top of any byte-order
        // mark.
        let mut skipped = 0u64;
        let (text_len, dirty) = match len {
            StrLen::Fixed(_) => (rest, false),
            // The field is the separators, the value and the byte that ends
            // it, and the sizing pass has already measured all three.
            StrLen::Scan { skip, comment, .. } => {
                let mut over = Skipping::default();
                skipped = body.iter().take_while(|b| over.steps_over(**b, skip, *comment)).count() as u64;
                (rest.saturating_sub(skipped).saturating_sub(1), true)
            }
            StrLen::Padded { pad, .. } => {
                let term = text::unit_bytes(settled, *pad);
                match find_unit(body, &term) {
                    None => (rest, false),
                    Some(i) => {
                        let tail = &body[i..];
                        // Anything in the padding that is not padding would be
                        // lost by writing back only what is shown.
                        let dirty = !tail.chunks(unit).all(|u| u == term);
                        (i as u64, dirty)
                    }
                }
            }
            StrLen::Terminated { end, .. } => {
                let term = text::unit_bytes(settled, *end);
                match find_unit(body, &term) {
                    Some(i) => (i as u64, false),
                    // No terminator to write back, so this one is read-only.
                    None => (rest, true),
                }
            }
        };
        Ok(Some(StrSpan { start: bom + skipped, len: text_len, settled, dirty, note }))
    }

    /// Where a field's value sits, whether the bytes fit the encoding, and how
    /// the encoding was decided. Everything a node needs beyond its value.
    #[allow(clippy::type_complexity)]
    pub(super) fn reading<S: Source>(&self, doc: &Document<S>, r: &Resolved, size: u64) -> R<((u64, u64), bool, Option<String>)> {
        let Some(span) = self.str_span(doc, r, size)? else { return Ok(((r.offset, size / 8), false, None)) };
        let shown = span.len.min(crate::encode::EDIT_LIMIT_BYTES);
        let bytes = self.read(doc, r, r.offset + span.start * 8, shown * 8)?;
        let (_, lossy) = text::decode_settled(span.settled, &bytes);
        let note = if lossy {
            Some(format!(
                "Not valid {}; the bad bytes show as \u{fffd}. Edit it in the hex view.",
                span.settled.name()
            ))
        } else {
            span.note
        };
        Ok(((r.offset + span.start * 8, span.len), lossy, note))
    }

    /// A padded text field shows only what is before its first pad byte. If the
    /// rest is not all padding, writing back what is shown would drop bytes the
    /// reader never saw, so such a field is not editable here.
    pub(super) fn padding_is_clean<S: Source>(&self, doc: &Document<S>, r: &Resolved, size: u64) -> R<bool> {
        if size > crate::encode::EDIT_LIMIT_BYTES * 8 {
            return Ok(true); // too long to edit anyway
        }
        Ok(!self.str_span(doc, r, size)?.map(|s| s.dirty).unwrap_or(false))
    }

    /// How a text field reads before its length is known: the encoding the
    /// scanner should step in, and the bytes any byte-order mark takes.
    pub(super) fn str_head<S: Source>(&self, doc: &Document<S>, r: &Resolved, enc: &Encoding) -> R<(Settled, u64)> {
        let want = 4u64.min((r.limit - r.offset) / 8);
        let head = if want == 0 { Vec::new() } else { self.read(doc, r, r.offset, want * 8)? };
        let (settled, bom, _) = text::settle(enc, &head);
        Ok((settled, bom as u64))
    }

    /// Scan for the terminator, whole code units at a time, and return the
    /// bytes of text and the bytes of the whole field. Read in blocks: a long
    /// string should not be one call per unit.
    pub(super) fn read_terminated<S: Source>(&self, doc: &Document<S>, r: &Resolved, term: &[u8], bom: u64) -> R<(u64, u64)> {
        const BLOCK: u64 = 256;
        /// A file with no terminator in it must fail rather than walk to the end.
        const CAP: u64 = 64 * 1024;
        let unit = term.len() as u64;
        let start = r.offset + bom * 8;
        let stop = r.limit.min(start + CAP * 8);
        let mut at = start;
        let mut text_bytes = 0u64;
        while at < stop {
            let mut n = BLOCK.min((stop - at) / 8);
            n -= n % unit;
            if n == 0 {
                break;
            }
            let block = self.read(doc, r, at, n * 8)?;
            for i in (0..block.len()).step_by(unit as usize) {
                if block[i..i + unit as usize] == *term {
                    let len = text_bytes + i as u64;
                    return Ok((len, bom + len + unit));
                }
            }
            text_bytes += n;
            at += n * 8;
        }
        fail(format!("no 0x{:02x} terminator within {} bytes", term[0], (stop - start) / 8))
    }

    /// A field that steps over a run of separators and then reads up to the
    /// next one. Answers how much of it is the value and how long the whole
    /// field is, the terminator included.
    ///
    /// A run of separators with nothing after it is a value of no bytes, which
    /// is what a header with two spaces between its numbers writes and what
    /// the field before it has already stepped over. Running out of container
    /// without meeting a separator is an error, the same answer a terminated
    /// field gives: the number is not finished, so there is no number.
    pub(super) fn read_scan<S: Source>(
        &self,
        doc: &Document<S>,
        r: &Resolved,
        skip: &[u8],
        ends: &[u8],
        comment: Option<(u8, u8)>,
    ) -> R<(u64, u64)> {
        const BLOCK: u64 = 256;
        /// A field with no separator in it must fail rather than walk to the end.
        const CAP: u64 = 64 * 1024;
        let stop = r.limit.min(r.offset + CAP * 8);
        let mut at = r.offset;
        let (mut skipped, mut seen) = (0u64, 0u64);
        let mut skipping = true;
        let mut over = Skipping::default();
        while at < stop {
            let n = BLOCK.min((stop - at) / 8);
            if n == 0 {
                break;
            }
            let block = self.read(doc, r, at, n * 8)?;
            for (i, b) in block.iter().enumerate() {
                if skipping && over.steps_over(*b, skip, comment) {
                    skipped += 1;
                    continue;
                }
                skipping = false;
                if ends.contains(b) {
                    let len = seen + i as u64 - skipped;
                    return Ok((len, skipped + len + 1));
                }
            }
            seen += n;
            at += n * 8;
        }
        fail(format!("no separator within {} bytes", (stop - r.offset) / 8))
    }

    pub(super) fn read_leb<S: Source>(&self, doc: &Document<S>, r: &Resolved) -> R<(u128, u64)> {
        let mut value: u128 = 0;
        let mut shift = 0;
        for i in 0..10u64 {
            let b = self.read(doc, r, r.offset + i * 8, 8)?[0];
            value |= ((b & 0x7f) as u128) << shift;
            shift += 7;
            if b & 0x80 == 0 {
                if let Ty::Leb128 { signed: true } = r.ty.base() {
                    if b & 0x40 != 0 {
                        let v = value as i128 - (1i128 << shift);
                        return Ok((v as u128, i + 1));
                    }
                }
                return Ok((value, i + 1));
            }
        }
        fail("LEB128 longer than 10 bytes")
    }

    /// A variable-length quantity: the high bit says another byte follows, and
    /// the seven bits below it are the next group down. Four bytes is the most
    /// a Standard MIDI File is allowed to use.
    pub(super) fn read_vlq<S: Source>(&self, doc: &Document<S>, r: &Resolved) -> R<(u128, u64)> {
        let mut value: u128 = 0;
        for i in 0..4u64 {
            let b = self.read(doc, r, r.offset + i * 8, 8)?[0];
            value = (value << 7) | (b & 0x7f) as u128;
            if b & 0x80 == 0 {
                return Ok((value, i + 1));
            }
        }
        fail("variable-length number longer than 4 bytes")
    }

    /// SQLite's varint: seven bits per byte, most significant group first, and
    /// a ninth byte that contributes all eight of its bits. The result is
    /// 64-bit two's complement, so a negative row id reads as one.
    pub(super) fn read_sqlite_varint<S: Source>(&self, doc: &Document<S>, r: &Resolved) -> R<(i128, u64)> {
        let mut value: u64 = 0;
        for i in 0..8u64 {
            let b = self.read(doc, r, r.offset + i * 8, 8)?[0];
            value = (value << 7) | (b & 0x7f) as u64;
            if b & 0x80 == 0 {
                return Ok((value as i64 as i128, i + 1));
            }
        }
        let last = self.read(doc, r, r.offset + 64, 8)?[0];
        value = (value << 8) | last as u64;
        Ok((value as i64 as i128, 9))
    }

    pub(super) fn primitive_value<S: Source>(&mut self, doc: &Document<S>, at: &[usize], r: &Resolved, ty: &Ty, size: u64) -> R<Value> {
        Ok(match ty {
            // A value inside JSON was read when its text was parsed.
            Ty::Json(_) => self.json_value(doc, at)?,
            Ty::UInt { bits, endian } => Value::UInt(read_uint(&self.read(doc, r, r.offset, size)?, *bits, *endian)),
            Ty::Int { bits, endian } => Value::Int(read_int(&self.read(doc, r, r.offset, size)?, *bits, *endian)),
            Ty::Fixed { bits, frac, endian, signed } => {
                let buf = self.read(doc, r, r.offset, size)?;
                let raw = if *signed { read_int(&buf, *bits, *endian) as f64 } else { read_uint(&buf, *bits, *endian) as f64 };
                Value::Float(raw / (1u64 << frac) as f64)
            }
            Ty::F16(e) => Value::Float(narrow_f16(read_uint(&self.read(doc, r, r.offset, 16)?, 16, *e) as u16)),
            Ty::BF16(e) => Value::Float(narrow_bf16(read_uint(&self.read(doc, r, r.offset, 16)?, 16, *e) as u16)),
            Ty::F8 { e4m3 } => Value::Float(f8_to_f64(self.read(doc, r, r.offset, 8)?[0], *e4m3)),
            Ty::F32(e) => Value::Float(narrow_f32(f32::from_bits(read_uint(&self.read(doc, r, r.offset, 32)?, 32, *e) as u32))),
            Ty::F64(e) => Value::Float(f64::from_bits(read_uint(&self.read(doc, r, r.offset, 64)?, 64, *e) as u64)),
            Ty::F80(e) => Value::Float(f80_to_f64(read_uint(&self.read(doc, r, r.offset, 80)?, 80, *e))),
            Ty::Leb128 { signed } => {
                let (v, _) = self.read_leb(doc, r)?;
                if *signed { Value::Int(v as i128) } else { Value::UInt(v) }
            }
            Ty::Vlq => Value::UInt(self.read_vlq(doc, r)?.0),
            Ty::Computed(e) => {
                if let Some(v) = self.memo.get(at).and_then(|m| m.computed) {
                    return Ok(Value::Int(v));
                }
                let v = self.eval_expr_at(doc, at, e, Some((r.offset, r.limit)))?;
                if let Some(m) = self.memo.get_mut(at) {
                    m.computed = Some(v);
                }
                Value::Int(v)
            }
            Ty::SqliteVarint => Value::Int(self.read_sqlite_varint(doc, r)?.0),
            Ty::Magic(want) => {
                let bytes = self.read(doc, r, r.offset, size)?;
                Value::Magic { ok: bytes == *want, bytes }
            }
            Ty::Bytes(_) => {
                let len = size / 8;
                match self.read(doc, r, r.offset, len.min(16) * 8) {
                    Ok(preview) => Value::Bytes { len, preview },
                    // A run of bytes too long to mean anything as a number is
                    // only ever shown, never read from. Where it is and how
                    // long it is are known already, so the row is worth having
                    // now, with the first few bytes filled in when they come.
                    // Short fields stay strict: a switch may be keying on them.
                    Err(EvalError::Pending(m)) if len > 15 => {
                        self.want(m);
                        Value::Unread { len }
                    }
                    Err(e) => return Err(e),
                }
            }
            Ty::Str { .. } => {
                let span = self.str_span(doc, r, size)?.expect("text field");
                let shown = span.len.min(256);
                let bytes = self.read(doc, r, r.offset + span.start * 8, shown * 8)?;
                let (mut text, _) = text::decode_settled(span.settled, &bytes);
                if span.len > shown {
                    text.push('\u{2026}');
                }
                Value::Str(text)
            }
            Ty::Enum { inner, def } => {
                let raw = match self.primitive_value(doc, at, r, inner, size)? {
                    Value::UInt(v) => i128::try_from(v).unwrap_or(i128::MAX),
                    Value::Int(v) => v,
                    _ => return fail("an enum must sit on an integer"),
                };
                Value::Enum { raw, name: def.name_of(raw), hex: def.hex }
            }
            Ty::Flags { inner, def } => {
                let raw = match self.primitive_value(doc, at, r, inner, size)? {
                    Value::UInt(v) => v,
                    Value::Int(v) => v as u128,
                    _ => return fail("flags must sit on an integer"),
                };
                let mut set: Vec<String> = Vec::new();
                let mut unnamed = 0u32;
                for bit in 0..size.min(128) as u32 {
                    if raw >> bit & 1 == 0 {
                        continue;
                    }
                    match def.label(bit) {
                        Some(n) => set.push(n.to_string()),
                        None => unnamed += 1,
                    }
                }
                Value::Flags { raw, set, unnamed }
            }
            _ => unreachable!("composite handled by caller"),
        })
    }
}
