//! Working out the numbers a template asks for: how long a field is, how many
//! elements a list holds, which case a switch takes.
//!
//! Every expression looks backwards or inwards, never forwards: at a field
//! before this one, at an element before this one, at the bytes this field
//! starts with. That is what lets an edit keep everything the bytes before it
//! settled, and what makes a file readable from the front.

use std::sync::Arc;

use super::go::Asked;
use super::memo::TagKey;
use super::*;

/// A label as a map can hold it, for the index a search over a named list
/// keeps. None for a label that has still to be worked out, which is not a
/// label yet. Text is trimmed at the end because that is how it is compared.
fn tag_key(tag: &Tag) -> Option<TagKey> {
    match tag {
        Tag::Int(v) => Some(TagKey::Int(*v)),
        Tag::Text(s) => Some(TagKey::Text(s.trim_end().to_string())),
        Tag::Bytes(b) => Some(TagKey::Bytes(b.clone())),
        Tag::Computed(_) | Tag::ComputedText(_) => None,
    }
}

/// What a whole-number expression says when it meets a real, which is a
/// template asking for a size, a count or an address in a number with a
/// fraction. The one way through is named, since it is the fix.
const REAL_IN_WHOLE: &str = "a real number where a whole number is needed; trunc(...) drops the fraction";

/// The same said of a field, after its name: a float asked for as a whole
/// number is not "not a number", which would send a reader looking for text.
const REAL_IS_NOT_WHOLE: &str = "is a real number, but a whole number is needed here; trunc(...) drops the fraction";

/// What a leaf that names a field found: the field's value and what to call it
/// in a refusal, or no field at all. See [`Evaluator::field_value`].
pub(super) enum Leaf {
    Value(Value, String),
    Nothing,
}

/// A value read as a real, for the values that are numbers: a float as itself
/// and a whole number as the real it is. Not text or bytes, whose second
/// reading as a big-endian number is for a switch keying on a tag and would be
/// nonsense multiplied by a scale; [`Expr::RealText`] reads the number text
/// spells.
pub(super) fn real_reading(v: &Value) -> Option<f64> {
    match v {
        Value::Float(f) => Some(*f),
        Value::Unset(inner) => real_reading(inner),
        Value::Str(_) | Value::Bytes { .. } => None,
        other => other.as_int().map(|i| i as f64),
    }
}

/// Whether an error has to reach whoever asked, rather than be taken for the
/// answer "not here". A search passes over an element that will not read,
/// since the one before it may still be the right answer. It must not pass
/// over bytes that have not arrived, or a go that ran out, which are no answer
/// at all. Nor over a read that gave up on depth: the element before is as
/// deep again, so passing over it is the same refusal once per element, and
/// what the search came back with at the end would be a nought nothing wrote.
fn passes_up(e: &EvalError) -> bool {
    match e {
        EvalError::Failed(why) => Evaluator::is_refusal(why),
        other => other.interrupted(),
    }
}

/// Ten to a whole power, as near as a double holds it.
///
/// Every power from nought to twenty-two is a double exactly, so those are
/// multiplied out and a negative one is a single division, which a double
/// rounds correctly. Past that the product would pick up the rounding of each
/// step, so the power is written out and read back, which rounds once.
fn ten_to(n: i128) -> f64 {
    match n {
        0..=22 => 10f64.powi(n as i32),
        -22..=-1 => 1.0 / 10f64.powi(-n as i32),
        _ => format!("1e{}", n.clamp(-100_000, 100_000)).parse().unwrap_or(if n > 0 { f64::INFINITY } else { 0.0 }),
    }
}

/// The real number a run of text spells, the way FITS writes one: blanks
/// around it, an exponent after `E` or Fortran's `D`, and digits on only one
/// side of the point, `.5` and `1.`, allowed. Nothing at all is nought, so a
/// card that is not there can be given a default by `Or`.
///
/// Not a number and infinity are refused: Rust would read `nan` and `inf`, and
/// no format that writes a scale as text means either.
pub(super) fn real_from_text(text: &str) -> R<f64> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(0.0);
    }
    let spelled: String = t.chars().map(|c| match c {
        'D' => 'E',
        'd' => 'e',
        c => c,
    }).collect();
    match spelled.parse::<f64>() {
        Ok(v) if v.is_finite() => Ok(v),
        _ => fail(format!("{t:?} is not a number")),
    }
}

/// The whole part of a real, towards nought, for a real that has to become a
/// size, a count or an address. See [`Expr::Trunc`].
fn whole_part(v: f64) -> R<i128> {
    if v.is_nan() {
        return fail("trunc(...) of NaN");
    }
    // 2^127 is a double exactly, and so is its negative, which is the least
    // an i128 holds; anything from there up is past the most it holds.
    let limit = 2f64.powi(127);
    if !(-limit..limit).contains(&v) {
        return fail(if v.is_infinite() { "trunc(...) of infinity" } else { "trunc(...) of a number too large to hold" });
    }
    Ok(v.trunc() as i128)
}

impl Evaluator {

    pub(super) fn eval_expr<S: Source>(&mut self, doc: &Document<S>, at: &[usize], e: &Expr) -> R<i128> {
        let here = self.memo.get(at).map(|r| (r.offset, r.limit));
        self.eval_expr_at(doc, at, e, here)
    }

    /// `here` is the field's own start and its container's limit, which is what
    /// `Remaining` measures. It has to be passed in while a node is still being
    /// resolved, since it is not in the memo yet.
    pub(super) fn eval_expr_at<S: Source>(
        &mut self,
        doc: &Document<S>,
        at: &[usize],
        e: &Expr,
        here: Option<(u64, u64)>,
    ) -> R<i128> {
        if self.go.outermost() {
            return self.outermost_whole(doc, at, e, here);
        }
        self.whole_question(doc, at, e, here)
    }

    /// The outermost expression of a read, asked so that a refusal for depth
    /// is asked again. One of these for each reading rather than a closure
    /// handed over where the expression is asked, since what that closure
    /// holds would sit in the frame of every expression inside another.
    #[inline(never)]
    fn outermost_whole<S: Source>(&mut self, doc: &Document<S>, at: &[usize], e: &Expr, here: Option<(u64, u64)>) -> R<i128> {
        self.outermost(doc, |ev| ev.whole_question(doc, at, e, here))
    }

    #[inline(never)]
    fn outermost_real<S: Source>(&mut self, doc: &Document<S>, at: &[usize], e: &Expr, here: Option<(u64, u64)>) -> R<f64> {
        self.outermost(doc, |ev| ev.real_question(doc, at, e, here))
    }

    #[inline(never)]
    fn outermost_text<S: Source>(&mut self, doc: &Document<S>, at: &[usize], e: &Expr, here: Option<(u64, u64)>) -> R<String> {
        self.outermost(doc, |ev| ev.text_question(doc, at, e, here))
    }

    /// The whole-number reading of an expression, counted as one more open.
    #[inline(always)]
    fn whole_question<S: Source>(&mut self, doc: &Document<S>, at: &[usize], e: &Expr, here: Option<(u64, u64)>) -> R<i128> {
        self.ask(at, e, here, Asked::Whole)?;
        let out = self.whole_at(doc, at, e, here);
        self.go.answered(&out);
        out
    }

    /// One more expression open inside the ones already being worked out, or
    /// the refusal that says there are too many.
    ///
    /// Every question that goes from one field to another passes through an
    /// expression: a computed value naming the field before it, a switch
    /// keyed on a sibling, a length, a count, a search by label, the record
    /// that placed a gathered element. Reading the field it lands on can ask
    /// another, and that one another, with nothing on the way down finished
    /// and so nothing remembered. Counting here counts every one of those
    /// hops whichever of them it is, and the arithmetic between them too,
    /// which spends the stack the same way. See `go::DEEPEST_QUESTION`.
    #[inline(always)]
    fn ask(&mut self, at: &[usize], e: &Expr, here: Option<(u64, u64)>, asked: Asked) -> R<()> {
        if !self.go.ask() {
            return self.refused_here(at, e, here, asked);
        }
        if self.go.keeps_this_one() {
            self.keep(at, e, here, asked);
        }
        Ok(())
    }

    /// Keep the expression just opened on the trail of a read being asked
    /// again. Out of line, since it is rare and every expression's frame would
    /// otherwise carry it.
    #[cold]
    #[inline(never)]
    fn keep(&mut self, at: &[usize], e: &Expr, here: Option<(u64, u64)>, asked: Asked) {
        self.go.keep(at, e, here, asked);
    }

    /// The refusal for one expression too many, with the question kept as the
    /// deepest on the trail when there is one. Out of line for the reason
    /// `keep` is.
    ///
    /// What it says here names the field and not the expression. Writing an
    /// expression out costs a frame for every level of it, and this is the
    /// deepest the stack gets: written here, the rest of an expression two
    /// hundred sums deep took more than a megabyte of stack in a debug build.
    /// So the expression is kept, and `Evaluator::outermost` writes it into
    /// the refusal once the read has come back up.
    #[cold]
    #[inline(never)]
    fn refused_here(&mut self, at: &[usize], e: &Expr, here: Option<(u64, u64)>, asked: Asked) -> R<()> {
        self.go.refused_here(at, e, here, asked);
        let said = Self::too_deep(self.asking_field(at), None);
        self.go.refused_at(&said, at, e);
        fail(said)
    }

    /// What a read that gave up on depth says, naming the field that was
    /// asking and what it asked. For the top of a read: see `refused_here`.
    pub(super) fn asked_too_deep(&self, at: &[usize], e: &Expr) -> String {
        Self::too_deep(self.asking_field(at), write_expr(e).as_deref())
    }

    /// The field an expression at `at` belongs to, by name, or by its index
    /// in a list.
    fn asking_field(&self, at: &[usize]) -> Option<String> {
        self.memo.get(at).map(|r| r.name.text()).or_else(|| {
            let (&last, parent) = at.split_last()?;
            match &self.memo.get(parent)?.ty {
                Ty::Struct(s) => s.fields.get(last).map(|f| f.name.to_string()),
                _ => Some(format!("[{last}]")),
            }
        })
    }

    fn too_deep(field: Option<String>, expr: Option<&str>) -> String {
        // Named for the class of trouble first, so a row cut short still says
        // it, and never with the expression in front of "nested too deep",
        // which would read as the expression itself being that deep. The
        // field and expression are the deepest reached, not the one asked
        // for, so they come last and as where it stopped rather than as what
        // is wrong. `is_refusal` knows it by "nested too deep".
        let limit = super::go::DEEPEST_QUESTION;
        let head = format!("dependency chain nested too deep: more than {limit} expressions");
        match (field, expr) {
            (Some(field), Some(expr)) => format!("{head}; stopped at {field} while evaluating {expr}"),
            (Some(field), None) => format!("{head}; stopped at {field}"),
            (None, Some(expr)) => format!("{head}; stopped while evaluating {expr}"),
            (None, None) => head,
        }
    }

    /// The most expressions that have been open inside one another at once
    /// since this evaluator was made: how close a file came to the limit.
    /// For the probes that measure it over a collection.
    pub fn deepest_question(&self) -> usize {
        self.go.deepest_asked()
    }

    fn whole_at<S: Source>(
        &mut self,
        doc: &Document<S>,
        at: &[usize],
        e: &Expr,
        here: Option<(u64, u64)>,
    ) -> R<i128> {
        Ok(match e {
            Expr::Lit(v) => *v,
            Expr::Remaining => match here {
                Some((offset, limit)) if limit >= offset => ((limit - offset) / 8) as i128,
                _ => return fail("nothing to measure the rest of"),
            },
            // The index of the element this sits in, which is what a field
            // whose type comes from a list read earlier needs.
            Expr::Idx => self.enclosing_lists(at).first().map_or(0, |(_, i)| *i as i128),
            // How far into the window this field starts, and how big that
            // window is. Both in bytes, rounded down: see `Expr::Pos`.
            Expr::Pos => {
                let Some((offset, _)) = here else { return fail("nothing to measure from") };
                let (start, _) = self.window_of(doc, at);
                match offset.checked_sub(start) {
                    Some(n) => (n / 8) as i128,
                    // A field placed outside the window it was declared in,
                    // which an `At` counted from the file can be. There is no
                    // honest distance to answer with.
                    None => return fail("this field starts before the window it sits in"),
                }
            }
            Expr::WindowSize => {
                let (start, end) = self.window_of(doc, at);
                (end.saturating_sub(start) / 8) as i128
            }
            // The same two asked of the whole space rather than of the nearest
            // window: offsets inside a space are already counted from its
            // front, so this is the offset itself and the space's own length.
            // See `Expr::SpacePos`.
            Expr::SpacePos => {
                let Some((offset, _)) = here else { return fail("nothing to measure from") };
                (offset / 8) as i128
            }
            Expr::SpaceSize => (self.space_len(doc, at) / 8) as i128,
            // How many elements a list holds, which is not how many bytes it
            // took: see `Expr::LenOf`.
            Expr::LenOf(name) => {
                let Some(p) = self.find_field(at, name) else { return fail(format!("unknown field {name}")) };
                self.resolve(doc, &p)?;
                if !matches!(
                    self.memo[&p].ty,
                    Ty::Array { .. } | Ty::Repeat { .. } | Ty::PointerList { .. } | Ty::Chain { .. } | Ty::Gather { .. }
                ) {
                    return fail(format!("{name} is not a list, so it has no count of elements"));
                }
                self.child_count(doc, &p)? as i128
            }
            // An answer no field holds, worked out by running the container.
            // See `eval::deduced`.
            Expr::Deduced(what) => self.deduced_int(doc, at, *what, here)?,
            Expr::Elem { array, index, field } => {
                let p = self.elem_path(doc, at, array, index, field, here)?;
                match self.int_at(doc, &p, array)? {
                    Some(v) => v,
                    None => return fail(format!("{array} holds no number there")),
                }
            }
            // Search a list for the element that says what it is. The whole
            // list is read to find it, which is what a handful of tagged
            // records costs; nothing that carries thousands of them asks.
            //
            // Zero when nothing in the list is labelled that way, or when what
            // was found holds no number, so `Or` can name what to do without
            // one.
            Expr::Tagged(t) => {
                let t = t.clone();
                match self.tagged_path(doc, at, &t, here)? {
                    Some((p, _)) => self.value_of(doc, &p)?.value.as_int().unwrap_or(0),
                    None => 0,
                }
            }
            // Asked of the record that placed the gathered element this sits
            // in, from the frame that record's offset was worked out in.
            Expr::Placer(e) => {
                let (end, frame) = self.placer_frame(doc, at)?;
                self.eval_expr_at(doc, &end, &e.clone(), frame)?
            }
            Expr::Product { array, index, field } => {
                let p = self.elem_path(doc, at, array, index, field, here)?;
                self.multiply(doc, &p, array)?
            }
            Expr::ProductOf(name) => {
                let Some(p) = self.find_field(at, name) else { return fail(format!("unknown field {name}")) };
                self.multiply(doc, &p, name)?
            }
            Expr::SumOf(name) => {
                let Some(p) = self.find_field(at, name) else { return fail(format!("unknown field {name}")) };
                self.add_up(doc, &p, name)?
            }
            Expr::MaxOf(name) => {
                let Some(p) = self.find_field(at, name) else { return fail(format!("unknown field {name}")) };
                self.maximum(doc, &p, name)?
            }
            Expr::PopCount(name) => {
                let Some(p) = self.find_field(at, name) else { return fail(format!("unknown field {name}")) };
                self.set_bits(doc, &p, name)?
            }
            Expr::Ref(name) => match self.lookup(doc, at, name)? {
                (Some(v), _) => v,
                // Said apart from text for the reason `int_at` says it.
                (None, _) => match self.field_value(doc, at, e, here) {
                    Ok(Leaf::Value(v, _)) if real_reading(&v).is_some() => {
                        return fail(format!("{name} {REAL_IS_NOT_WHOLE}"))
                    }
                    Err(err) if passes_up(&err) => return Err(err),
                    _ => return fail(format!("{name} is not a number")),
                },
            },
            Expr::SizeOf(name) => self.lookup(doc, at, name)?.1,
            Expr::BitsOf(name) => self.lookup_bits(doc, at, name)?.1,
            // Where a field is rather than what it says, as an address of this
            // format: counted from the nearest origin, so that handing it to
            // an `At` anchored the same way lands on those bytes again. See
            // `Expr::StartOf`.
            Expr::StartOf(inner) => {
                let inner = inner.clone();
                let Some(p) = self.text_path(doc, at, &inner, here)? else {
                    return fail("nothing there to be the start of");
                };
                self.resolve(doc, &p)?;
                let found = &self.memo[&p];
                // Offsets of two address spaces are two different numbers.
                // Answering across them would give the start of a field in an
                // unpacked stream as if it were a place in the file.
                let space = self.memo.get(at).map_or(0, |r| r.space);
                if found.space != space {
                    return fail("that field is read in another space");
                }
                let offset = found.offset;
                let base = self.origin_of(at).map_or(0, |(offset, _)| offset);
                if offset < base {
                    return fail("that field starts before this copy of the format does");
                }
                ((offset - base) / 8) as i128
            }
            // Read where this field starts without taking the bits: what a
            // field that exists only when the byte says so has to ask.
            Expr::Peek { bits, endian } => {
                let Some((offset, limit)) = here else { return fail("nothing to look at") };
                if offset + u64::from(*bits) > limit {
                    return fail("looks past the end of its container");
                }
                // A peek narrower than a byte is placed the same way a field
                // of the same width would be: see `decode::lsb_offset`.
                let offset = match lsb_packed(*bits, *endian, offset) {
                    true => match lsb_offset(*bits, offset) {
                        Some(at) => at,
                        None => return fail("a peek packed low-bit-first would cross a byte boundary"),
                    },
                    false => offset,
                };
                // In whatever space this field is being read in. A switch
                // peeking at the byte it is about to read, inside a decoded
                // stream, must look at that stream and not at the file.
                let buf = self.read_in(doc, self.space_at(at), offset, u64::from(*bits))?;
                read_uint(&buf, *bits, *endian) as i128
            }
            // The same, further on: what a record whose shape is settled by a
            // byte after the fields it settles has to ask.
            Expr::PeekAt { skip, bits, endian } => {
                let Some((offset, limit)) = here else { return fail("nothing to look at") };
                let skip = self.eval_expr_at(doc, at, &skip.clone(), here)?;
                // Backwards means from the end of the container: what a format
                // that signs itself at the far end of the file needs, without
                // the asking field having to know where it is.
                let from = if skip < 0 {
                    match limit.checked_sub(skip.unsigned_abs() as u64) {
                        Some(from) if from >= offset => from,
                        _ => return fail("looks back past where it is"),
                    }
                } else {
                    offset + skip as u64
                };
                if from + u64::from(*bits) > limit {
                    return fail("looks past the end of its container");
                }
                let from = match lsb_packed(*bits, *endian, from) {
                    true => match lsb_offset(*bits, from) {
                        Some(at) => at,
                        None => return fail("a peek packed low-bit-first would cross a byte boundary"),
                    },
                    false => from,
                };
                let buf = self.read_in(doc, self.space_at(at), from, u64::from(*bits))?;
                read_uint(&buf, *bits, *endian) as i128
            }
            // A peek at an address rather than at a distance. Held to the
            // space and not to the container the field sits in: an address is
            // an address of the whole file, and a record that names one is
            // usually a record inside a window that does not hold it.
            Expr::PeekIn { at: addr, bits, endian } => self.peek_in(doc, at, addr, *bits, *endian, here)?,
            // Walk forward for what ends an unmeasured stream. A lead is told
            // apart from an escape by the byte after it, so blocks overlap by
            // the length of the lead: one straddling the seam between two
            // blocks, or ending at it with its successor in the next, is whole
            // in one of them.
            Expr::ToMarker { lead, unless } => {
                let Some((offset, limit)) = here else { return fail("nothing to measure") };
                if limit < offset {
                    return fail("nothing to measure");
                }
                if lead.is_empty() {
                    return fail("nothing to measure to");
                }
                let total = (limit - offset) / 8;
                let (lead, unless) = (lead.clone(), unless.clone());
                let n = lead.len();
                // The lead alone when there is nothing to tell it apart from,
                // the lead and the byte after it when there is.
                let overlap = if unless.is_empty() { n as u64 - 1 } else { n as u64 };
                let hit = scan_blocks(self, doc, self.space_at(at), offset, total, overlap, Dir::Forward, |b| {
                    (0..b.len().saturating_sub(n - 1)).find(|&i| {
                        b[i..i + n] == lead[..]
                            && (unless.is_empty() || b.get(i + n).is_some_and(|next| !unless.contains(next)))
                    })
                })?;
                // A lead with nothing after it to tell it from an escape is
                // not a marker: nothing has said so, so the run measures to
                // the end.
                hit.unwrap_or(total) as i128
            }
            Expr::Prev(name) => self.prev_field(doc, at, name)?,
            // Walk for a word rather than for a byte. Blocks overlap by all
            // but one byte of the needle, so a word written across the seam
            // between two of them is still found.
            Expr::Find { needle, last } => {
                let Some((offset, limit)) = here else { return fail("nothing to search") };
                if limit < offset {
                    return fail("nothing to search");
                }
                if needle.is_empty() {
                    return fail("nothing to look for");
                }
                let total = (limit - offset) / 8;
                let n = needle.len();
                let dir = if *last { Dir::Backward } else { Dir::Forward };
                let hit = scan_blocks(self, doc, self.space_at(at), offset, total, n as u64 - 1, dir, |b| match dir {
                    Dir::Backward => b.windows(n).rposition(|w| w == needle.as_slice()),
                    Dir::Forward => b.windows(n).position(|w| w == needle.as_slice()),
                })?;
                // Not written again: the run measures to the end of its
                // container, as a stream with no marker after it does. A file
                // cut off before the word it promised is still worth showing.
                hit.unwrap_or(total) as i128
            }
            Expr::Sibling(field) => self.sibling_field(doc, at, &field.clone())?,
            // A field beside this one, and a path down into it.
            Expr::Within(field) => {
                let field = field.clone();
                let p = self.within_path(doc, at, &field)?;
                match self.int_at(doc, &p, &field.join("."))? {
                    Some(v) => v,
                    None => return fail(format!("{} holds no number", field.join("."))),
                }
            }
            // A list inside an earlier field, indexed. Reached in two steps
            // because a name reaches only a sibling.
            Expr::ElemWithin { path, index, field } => {
                let p = self.elem_within_path(doc, at, path, index, field, here)?;
                match self.int_at(doc, &p, &path.join("."))? {
                    Some(v) => v,
                    None => return fail(format!("{} holds no number there", path.join("."))),
                }
            }
            Expr::Or(a, b) => match self.eval_expr_at(doc, at, a, here)? {
                0 => self.eval_expr_at(doc, at, b, here)?,
                v => v,
            },
            Expr::Add(a, b) => self.eval_expr_at(doc, at, a, here)? + self.eval_expr_at(doc, at, b, here)?,
            Expr::Sub(a, b) => self.eval_expr_at(doc, at, a, here)? - self.eval_expr_at(doc, at, b, here)?,
            Expr::Mul(a, b) => self.eval_expr_at(doc, at, a, here)? * self.eval_expr_at(doc, at, b, here)?,
            Expr::Bit(a, n) => (self.eval_expr_at(doc, at, a, here)? >> n) & 1,
            Expr::Min(a, b) => self.eval_expr_at(doc, at, a, here)?.min(self.eval_expr_at(doc, at, b, here)?),
            Expr::Max(a, b) => self.eval_expr_at(doc, at, a, here)?.max(self.eval_expr_at(doc, at, b, here)?),
            // What is left of a boundary, which is nothing at all when the
            // run before it already ended on one.
            Expr::PadTo { n, align } => {
                if *align == 0 {
                    return fail("padded to a boundary of nothing");
                }
                let align = i128::from(*align);
                let n = self.eval_expr_at(doc, at, &n.clone(), here)?;
                (align - n.rem_euclid(align)).rem_euclid(align)
            }
            Expr::Shl(a, b) => {
                let by = self.eval_expr_at(doc, at, b, here)?;
                if !(0..64).contains(&by) {
                    return fail("shift of more than a machine word");
                }
                self.eval_expr_at(doc, at, a, here)? << by
            }
            // Down rather than up, and by the same rule: a shift of more than
            // a machine word is a template saying something it cannot mean.
            Expr::Shr(a, b) => {
                let by = self.eval_expr_at(doc, at, b, here)?;
                if !(0..64).contains(&by) {
                    return fail("shift of more than a machine word");
                }
                self.eval_expr_at(doc, at, a, here)? >> by
            }
            Expr::And(a, b) => self.eval_expr_at(doc, at, a, here)? & self.eval_expr_at(doc, at, b, here)?,
            // Both sides worked out, unlike `Either` and `Both`, which are
            // truths and may stand in front of what they guard. These are
            // arithmetic: every bit of both sides is part of the answer.
            Expr::BitOr(a, b) => self.eval_expr_at(doc, at, a, here)? | self.eval_expr_at(doc, at, b, here)?,
            Expr::BitXor(a, b) => self.eval_expr_at(doc, at, a, here)? ^ self.eval_expr_at(doc, at, b, here)?,
            // Over the whole 128-bit number, which is what makes `~0` come to
            // -1. See `Expr::BitNot`.
            Expr::BitNot(a) => !self.eval_expr_at(doc, at, a, here)?,
            Expr::Less(a, b) => {
                i128::from(self.eval_expr_at(doc, at, a, here)? < self.eval_expr_at(doc, at, b, here)?)
            }
            Expr::Eq(a, b) => {
                i128::from(self.eval_expr_at(doc, at, a, here)? == self.eval_expr_at(doc, at, b, here)?)
            }
            Expr::Ne(a, b) => {
                i128::from(self.eval_expr_at(doc, at, a, here)? != self.eval_expr_at(doc, at, b, here)?)
            }
            Expr::Le(a, b) => {
                i128::from(self.eval_expr_at(doc, at, a, here)? <= self.eval_expr_at(doc, at, b, here)?)
            }
            Expr::Gt(a, b) => {
                i128::from(self.eval_expr_at(doc, at, a, here)? > self.eval_expr_at(doc, at, b, here)?)
            }
            Expr::Ge(a, b) => {
                i128::from(self.eval_expr_at(doc, at, a, here)? >= self.eval_expr_at(doc, at, b, here)?)
            }
            // Short-circuiting, and that is part of what they say rather than
            // an optimisation: a guard in front of a read only guards while
            // what it guards is left unread. See [`Expr::Both`].
            Expr::Both(a, b) => match self.eval_expr_at(doc, at, a, here)? {
                0 => 0,
                _ => i128::from(self.eval_expr_at(doc, at, b, here)? != 0),
            },
            Expr::Either(a, b) => match self.eval_expr_at(doc, at, a, here)? {
                0 => i128::from(self.eval_expr_at(doc, at, b, here)? != 0),
                _ => 1,
            },
            Expr::Not(a) => i128::from(self.eval_expr_at(doc, at, a, here)? == 0),
            // Only the branch taken is asked, so a question that cannot be
            // answered in the other branch never comes up.
            Expr::Cond { when, then, otherwise } => {
                let taken = match self.eval_expr_at(doc, at, when, here)? {
                    0 => otherwise,
                    _ => then,
                };
                self.eval_expr_at(doc, at, taken, here)?
            }
            Expr::Div(a, b) => {
                let d = self.eval_expr_at(doc, at, b, here)?;
                if d == 0 {
                    return fail("division by zero");
                }
                self.eval_expr_at(doc, at, a, here)? / d
            }
            // With the sign of the divisor, which is the rule the formats
            // that use one were written against. `rem_euclid` is a third rule
            // again, always non-negative, and would disagree here whenever
            // the divisor is negative. See [`Expr::Mod`].
            Expr::Mod(a, b) => {
                let d = self.eval_expr_at(doc, at, b, here)?;
                if d == 0 {
                    return fail("division by zero");
                }
                let n = self.eval_expr_at(doc, at, a, here)?;
                // `i128::MIN % -1` overflows; the answer is nought either way.
                let r = n.checked_rem(d).unwrap_or(0);
                if r != 0 && (r < 0) != (d < 0) { r + d } else { r }
            }
            Expr::DivCeil(a, b) => {
                let d = self.eval_expr_at(doc, at, b, here)?;
                if d == 0 {
                    return fail("division by zero");
                }
                let n = self.eval_expr_at(doc, at, a, here)?;
                let q = n / d;
                if n % d != 0 && ((n < 0) == (d < 0)) { q + 1 } else { q }
            }
            Expr::Log2(a) => {
                let n = self.eval_expr_at(doc, at, a, here)?;
                if n <= 0 {
                    return fail("logarithm of a number that is not positive");
                }
                i128::from(n.ilog2())
            }
            // A real where a whole number is wanted is a template saying
            // something it cannot mean, and rounding it quietly would place
            // bytes at an offset nobody wrote. `trunc` is the way in.
            Expr::Real(_) | Expr::RealText(_) => return fail(REAL_IN_WHOLE),
            // In a whole number these are the shift and the power they would
            // be. A negative power is a fraction, and says so rather than
            // pointing at `trunc`, which would make it nought.
            Expr::Pow2(a) => {
                let n = self.eval_expr_at(doc, at, a, here)?;
                if n < 0 {
                    return fail("two to a negative power is not a whole number");
                }
                if n >= 127 {
                    return fail(format!("two to the power {n} is too large to hold"));
                }
                1i128 << n
            }
            Expr::Pow10(a) => {
                let n = self.eval_expr_at(doc, at, a, here)?;
                if n < 0 {
                    return fail("ten to a negative power is not a whole number");
                }
                if n > 38 {
                    return fail(format!("ten to the power {n} is too large to hold"));
                }
                10i128.pow(n as u32)
            }
            Expr::Trunc(a) => {
                let v = self.eval_real_at(doc, at, a, here)?;
                whole_part(v)?
            }
        })
    }

    /// The same expression worked out as reals, for a [`Ty::ComputedReal`].
    ///
    /// One language and two readings of it, chosen by the field asking rather
    /// than by the expression: `stored * scale` is a whole number in a
    /// `computed` field and a real in a `computed real` one. What is real is
    /// the arithmetic that makes the value. What decides something inside it
    /// stays whole: which branch a condition takes, what power of two or ten
    /// to scale by. Anything this has no real reading of is worked out whole
    /// and then taken as a real, which is every leaf that is not a field: a
    /// size, a count, an index.
    pub(super) fn eval_real_at<S: Source>(
        &mut self,
        doc: &Document<S>,
        at: &[usize],
        e: &Expr,
        here: Option<(u64, u64)>,
    ) -> R<f64> {
        if self.go.outermost() {
            return self.outermost_real(doc, at, e, here);
        }
        self.real_question(doc, at, e, here)
    }

    /// The real reading of an expression, counted as one more open.
    #[inline(always)]
    fn real_question<S: Source>(&mut self, doc: &Document<S>, at: &[usize], e: &Expr, here: Option<(u64, u64)>) -> R<f64> {
        self.ask(at, e, here, Asked::Real)?;
        let out = self.real_at(doc, at, e, here);
        self.go.answered(&out);
        out
    }

    fn real_at<S: Source>(
        &mut self,
        doc: &Document<S>,
        at: &[usize],
        e: &Expr,
        here: Option<(u64, u64)>,
    ) -> R<f64> {
        Ok(match e {
            Expr::Real(v) => *v,
            Expr::Lit(v) => *v as f64,
            Expr::Add(a, b) => self.eval_real_at(doc, at, a, here)? + self.eval_real_at(doc, at, b, here)?,
            Expr::Sub(a, b) => self.eval_real_at(doc, at, a, here)? - self.eval_real_at(doc, at, b, here)?,
            Expr::Mul(a, b) => self.eval_real_at(doc, at, a, here)? * self.eval_real_at(doc, at, b, here)?,
            // Dividing by nought fails here as it does in a whole number,
            // rather than answering with an infinity nobody wrote.
            Expr::Div(a, b) => {
                let d = self.eval_real_at(doc, at, b, here)?;
                if d == 0.0 {
                    return fail("division by zero");
                }
                self.eval_real_at(doc, at, a, here)? / d
            }
            Expr::Min(a, b) => self.eval_real_at(doc, at, a, here)?.min(self.eval_real_at(doc, at, b, here)?),
            Expr::Max(a, b) => self.eval_real_at(doc, at, a, here)?.max(self.eval_real_at(doc, at, b, here)?),
            // The left side, or the right when the left comes to nought, and
            // the nought is the real one: a scale of 0.5 is an answer.
            Expr::Or(a, b) => match self.eval_real_at(doc, at, a, here)? {
                v if v == 0.0 => self.eval_real_at(doc, at, b, here)?,
                v => v,
            },
            // Which branch is a whole-number question; the branch is not.
            Expr::Cond { when, then, otherwise } => {
                let taken = match self.eval_expr_at(doc, at, when, here)? {
                    0 => otherwise,
                    _ => then,
                };
                self.eval_real_at(doc, at, taken, here)?
            }
            // The power is a whole number, and what it comes to is not.
            Expr::Pow2(a) => {
                let n = self.eval_expr_at(doc, at, a, here)?;
                2f64.powi(n.clamp(-4096, 4096) as i32)
            }
            Expr::Pow10(a) => ten_to(self.eval_expr_at(doc, at, a, here)?),
            Expr::RealText(inner) => {
                let text = self.text_at(doc, at, &inner.clone(), here)?;
                real_from_text(&text)?
            }
            Expr::Ref(_)
            | Expr::Within(_)
            | Expr::Elem { .. }
            | Expr::ElemWithin { .. }
            | Expr::Tagged(_)
            | Expr::Placer(_)
            | Expr::Sibling(_)
            | Expr::Prev(_) => match self.field_value(doc, at, e, here)? {
                Leaf::Value(v, what) => match real_reading(&v) {
                    Some(f) => f,
                    None if matches!(v, Value::Str(_) | Value::Bytes { .. }) => {
                        return fail(format!("{what} is text; real(...) reads the number it spells"))
                    }
                    None => return fail(format!("{what} is not a number")),
                },
                Leaf::Nothing => 0.0,
            },
            other => self.eval_expr_at(doc, at, other, here)? as f64,
        })
    }

    /// What a leaf that names a field lands on, read as the value it is rather
    /// than as a whole number: a real reads a float field as the float, and
    /// the relations panel writes one in as it reads.
    ///
    /// `Nothing` is a search or a walk back that found no field, which a
    /// whole number reads as nought and so does a real. A field that is there
    /// and the file did not write fails, for the reason [`Evaluator::int_at`]
    /// gives.
    pub(super) fn field_value<S: Source>(
        &mut self,
        doc: &Document<S>,
        at: &[usize],
        e: &Expr,
        here: Option<(u64, u64)>,
    ) -> R<Leaf> {
        let (path, what) = match e {
            Expr::Ref(name) => match self.find_field(at, name) {
                Some(p) => (p, name.to_string()),
                None => return fail(format!("unknown field {name}")),
            },
            Expr::Within(field) => (self.within_path(doc, at, &field.clone())?, field.join(".")),
            Expr::Elem { array, index, field } => (self.elem_path(doc, at, array, index, field, here)?, array.to_string()),
            Expr::ElemWithin { path, index, field } => {
                (self.elem_within_path(doc, at, path, index, field, here)?, path.join("."))
            }
            Expr::Tagged(t) => {
                let t = t.clone();
                match self.tagged_path(doc, at, &t, here)? {
                    Some((p, label)) => (p, label),
                    None => return Ok(Leaf::Nothing),
                }
            }
            // Asked of the record that placed this element, from the frame its
            // offset was worked out in.
            Expr::Placer(inner) => {
                let (end, frame) = self.placer_frame(doc, at)?;
                return self.field_value(doc, &end, &inner.clone(), frame);
            }
            Expr::Sibling(field) => match self.sibling_field_path(doc, at, &field.clone())? {
                Some(p) => (p, field.join(".")),
                None => return Ok(Leaf::Nothing),
            },
            // The element before this one in the nearest list, and nothing for
            // the first, the way `previous` reads as a whole number.
            Expr::Prev(name) => {
                let Some((list, idx)) = self.enclosing_lists(at).into_iter().next() else { return Ok(Leaf::Nothing) };
                if idx == 0 {
                    return Ok(Leaf::Nothing);
                }
                let mut elem = list;
                elem.push(idx - 1);
                self.resolve(doc, &elem)?;
                let Ty::Struct(s) = self.memo[&elem].ty.base() else { return Ok(Leaf::Nothing) };
                let Some(j) = s.fields.iter().position(|f| *f.name == **name) else { return Ok(Leaf::Nothing) };
                elem.push(j);
                (elem, name.to_string())
            }
            _ => return fail("that expression names no field"),
        };
        let info = self.value_of(doc, &path)?;
        if info.absent {
            return fail(format!("{what} is not in this file"));
        }
        Ok(Leaf::Value(info.value, what))
    }

    /// The number the node at `path` holds, for an expression that reached it
    /// by a path. `None` when it has a reading that is not a number, which
    /// each caller words its own refusal for.
    ///
    /// A field the file did not write is refused here rather than answered:
    /// see the note in [`Evaluator::lookup_bits`], which is the same trap one
    /// name further out.
    fn int_at<S: Source>(&mut self, doc: &Document<S>, path: &[usize], what: &str) -> R<Option<i128>> {
        let info = self.value_of(doc, path)?;
        if info.absent {
            return fail(format!("{what} is not in this file"));
        }
        // A float is a number, and what is wrong is where it was asked for:
        // the same field reads as the float it is in a `computed real`. So
        // the refusal says which, rather than calling it not a number.
        if real_reading(&info.value).is_some() && info.value.as_int().is_none() {
            return fail(format!("{what} {REAL_IS_NOT_WHOLE}"));
        }
        Ok(info.value.as_int())
    }

    /// Where the window around the field at `at` starts and ends, in bits of
    /// whatever space that field is read in. Bits because every offset here is
    /// one; the two expressions built on this answer in bytes.
    ///
    /// The window is the nearest [`Ty::Sized`] above the field, which is the
    /// node the evaluator recorded a `declared_size` on. Above rather than
    /// including: a window declared *on* this field is the window its contents
    /// sit in, and the field itself sits in the one around that, which is the
    /// same stretch a Kaitai `_io` names at each level.
    ///
    /// In the same space only. A decoded stream's children count from zero in
    /// the stream's own bytes, and an outer `Sized` counted in the file would
    /// answer with offsets from another numbering entirely. Where the space
    /// has no window in it, the window is the whole space.
    /// A peek at an address rather than at a distance. Held to the space and
    /// not to the container the field sits in: an address is an address of the
    /// whole file, and a record that names one is usually a record inside a
    /// window that does not hold it. See [`Expr::PeekIn`].
    ///
    /// Apart from `whole_at` for the sake of the stack, which is open once for
    /// every expression inside another: what one reading takes has no business
    /// in the frame of every expression that reads nothing.
    #[inline(never)]
    fn peek_in<S: Source>(
        &mut self,
        doc: &Document<S>,
        at: &[usize],
        addr: &Expr,
        bits: u32,
        endian: crate::template::Endian,
        here: Option<(u64, u64)>,
    ) -> R<i128> {
        let addr = self.eval_expr_at(doc, at, &addr.clone(), here)?;
        if addr < 0 {
            return fail("looks at an address before the start of the file");
        }
        let Ok(from) = u64::try_from(addr) else { return fail("looks past the end of the file") };
        if from.checked_add(u64::from(bits)).is_none_or(|end| end > self.space_len(doc, at)) {
            return fail("looks past the end of the file");
        }
        let from = match lsb_packed(bits, endian, from) {
            true => match lsb_offset(bits, from) {
                Some(at) => at,
                None => return fail("a peek packed low-bit-first would cross a byte boundary"),
            },
            false => from,
        };
        let buf = self.read_in(doc, self.space_at(at), from, u64::from(bits))?;
        Ok(read_uint(&buf, bits, endian) as i128)
    }

    /// How many bits the space the field at `at` is read in holds: the file at
    /// the top level, and what a compressed run unpacked to inside one.
    fn space_len<S: Source>(&self, doc: &Document<S>, at: &[usize]) -> u64 {
        match self.space_at(at) {
            0 => doc.len_bits(),
            other => self.spaces.len_bits(other),
        }
    }

    fn window_of<S: Source>(&self, doc: &Document<S>, at: &[usize]) -> (u64, u64) {
        let space = self.space_at(at);
        let found = (0..at.len()).rev().find_map(|k| match self.memo.get(&at[..k]) {
            Some(r) if r.space == space => r.declared_size.map(|n| (r.offset, r.offset + n)),
            _ => None,
        });
        found.unwrap_or(match space {
            0 => (0, doc.len_bits()),
            other => (0, self.spaces.len_bits(other)),
        })
    }

    /// The text an expression reaches, wherever text is wanted: the case a
    /// `Match` takes, the value of a `ComputedText` field, the name a field is
    /// displayed under, the label a text-keyed search is looking for.
    ///
    /// One primitive, so that every one of those asks the same question and
    /// gets the same answer. Which expressions can be read as text is decided
    /// once, in [`Evaluator::text_path`], and a new way of reaching a field
    /// works everywhere text is wanted the moment it is added there.
    ///
    /// Empty when the expression reaches nothing, which is not an error: a
    /// search that found no matching record answers no text, a `Match` takes
    /// its default, and a name that could not be read leaves the field with
    /// the one it had.
    pub(super) fn text_at<S: Source>(
        &mut self,
        doc: &Document<S>,
        at: &[usize],
        e: &Expr,
        here: Option<(u64, u64)>,
    ) -> R<String> {
        if self.go.outermost() {
            return self.outermost_text(doc, at, e, here);
        }
        self.text_question(doc, at, e, here)
    }

    /// The text an expression reaches, counted as one more open.
    #[inline(always)]
    fn text_question<S: Source>(&mut self, doc: &Document<S>, at: &[usize], e: &Expr, here: Option<(u64, u64)>) -> R<String> {
        self.ask(at, e, here, Asked::Text)?;
        let out = self.text_in(doc, at, e, here);
        self.go.answered(&out);
        out
    }

    fn text_in<S: Source>(
        &mut self,
        doc: &Document<S>,
        at: &[usize],
        e: &Expr,
        here: Option<(u64, u64)>,
    ) -> R<String> {
        // The one expression whose text is not in any field: it is worked out
        // by running the container, so there is no path to read it off.
        if let Expr::Deduced(what) = e {
            return self.deduced_text(doc, at, *what, here);
        }
        // Text asked of a record is read from that record, which is how a
        // gathered element is typed by a word its record holds.
        if let Expr::Placer(inner) = e {
            let (end, frame) = self.placer_frame(doc, at)?;
            return self.text_at(doc, &end, &inner.clone(), frame);
        }
        match self.text_path(doc, at, e, here)? {
            Some(p) => self.text_of(doc, &p),
            None => Ok(String::new()),
        }
    }

    /// Where the field an expression names is, for the expressions that name
    /// one. `None` when the expression reaches nothing that is there.
    ///
    /// Every expression that lands on a field belongs here: `Ref` for one
    /// beside it, `Elem` for one inside a list, `Within` for a path down into
    /// a sibling, `Tagged` for the one a search found. Arithmetic does not:
    /// there is no text in a sum, and answering with the digits of one would
    /// be inventing a reading the file does not have.
    pub(super) fn text_path<S: Source>(
        &mut self,
        doc: &Document<S>,
        at: &[usize],
        e: &Expr,
        here: Option<(u64, u64)>,
    ) -> R<Option<Vec<usize>>> {
        Ok(Some(match e {
            Expr::Ref(name) => match self.find_field(at, name) {
                Some(p) => p,
                None => return fail(format!("unknown field {name}")),
            },
            Expr::Elem { array, index, field } => self.elem_path(doc, at, array, index, field, here)?,
            // A field declared before this one, and a path down into it. What
            // a format that writes its element type inside its header needs:
            // an NPY says `'descr': '<f8'` in a dict of its own, and the array
            // that reads as f64 because of it is that dict's sibling.
            Expr::Within(field) => self.within_path(doc, at, &field.clone())?,
            // One element of a list inside an earlier field, read as text:
            // what types the numbers of an NPY's structured dtype, where the
            // word that names the type is in one element of a list in the
            // header and the numbers are the header's sibling.
            Expr::ElemWithin { path, index, field } => {
                let (path, index, field) = (path.clone(), index.clone(), field.clone());
                self.elem_within_path(doc, at, &path, &index, &field, here)?
            }
            // The element a search found, which is what a format that names
            // its own record types needs: the number a record carries selects
            // an earlier record, and the word written in that one is the type.
            //
            // Nothing carrying that label is no more an error here than it is
            // when the answer is a number. The first record of a stream that
            // defines its own record types has nothing behind it to look in.
            Expr::Tagged(t) => {
                let t = t.clone();
                match self.tagged_path(doc, at, &t, here)? {
                    Some((p, _)) => p,
                    None => return Ok(None),
                }
            }
            // The field the expression names in the record that placed this.
            Expr::Placer(inner) => {
                let (end, frame) = self.placer_frame(doc, at)?;
                return self.text_path(doc, &end, &inner.clone(), frame);
            }
            // The field the backwards search lands on, which is what an HDF5
            // dataset needs to place its datatype message a second time: the
            // message is an earlier element of the object's messages, beside
            // the layout message that places the elements, and how those
            // elements are laid out is a list inside it. Nothing found is no
            // field, as it is for a search by label.
            Expr::Sibling(field) => match self.sibling_field_path(doc, at, &field.clone())? {
                Some(p) => p,
                None => return Ok(None),
            },
            _ => return fail("text has to come from a field, not from arithmetic"),
        }))
    }

    /// The whole text of the field at `path`.
    ///
    /// Not the node's value, which for a long text field is a preview with an
    /// ellipsis on the end: a name matched against three characters of its
    /// first two hundred and fifty-six is a name matched against something the
    /// file does not say. Bytes read as text too, lossily, since a format that
    /// writes a fixed-width label often declares it as bytes.
    pub(super) fn text_of<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<String> {
        self.resolve(doc, path)?;
        if matches!(self.memo[path].ty.base(), Ty::Str { .. }) {
            return Ok(self.text_value(doc, path)?.0);
        }
        let size = self.size_of(doc, path)?;
        let r = self.memo[path].clone();
        if matches!(r.ty.base(), Ty::Bytes(_) | Ty::Magic(_)) {
            let shown = (size / 8).min(crate::encode::EDIT_LIMIT_BYTES);
            let bytes = self.read(doc, &r, r.offset, shown * 8)?;
            return Ok(String::from_utf8_lossy(&bytes).into_owned());
        }
        match self.value_of(doc, path)?.value {
            Value::Str(s) => Ok(s),
            other => fail(format!("{other:?} is not text")),
        }
    }

    /// Every number in the array at `path`, multiplied together: what a shape
    /// describes.
    fn multiply<S: Source>(&mut self, doc: &Document<S>, path: &[usize], what: &str) -> R<i128> {
        let n = self.child_count(doc, path)?;
        let mut total: i128 = 1;
        let mut child = path.to_vec();
        for i in 0..n as usize {
            child.push(i);
            let v = self.value_of(doc, &child)?.value.as_int();
            child.pop();
            let Some(v) = v else { return fail(format!("{what} holds no number there")) };
            let Some(next) = total.checked_mul(v) else { return fail("shape too large to count") };
            total = next;
        }
        // An empty shape is one weight, not none: a tensor of no dimensions
        // holds a single number. A shape of `[0]` is a different thing and
        // does come to nothing, which the multiplying already says.
        Ok(total)
    }

    /// The numbers of a list, added up. An empty list comes to nothing, which
    /// is the right answer and not the same as the empty product being one.
    fn add_up<S: Source>(&mut self, doc: &Document<S>, path: &[usize], what: &str) -> R<i128> {
        let n = self.child_count(doc, path)?;
        let mut total: i128 = 0;
        let mut child = path.to_vec();
        for i in 0..n as usize {
            child.push(i);
            let v = self.value_of(doc, &child)?.value.as_int();
            child.pop();
            let Some(v) = v else { return fail(format!("{what} holds no number there")) };
            let Some(next) = total.checked_add(v) else { return fail("too many to count") };
            total = next;
        }
        Ok(total)
    }

    /// How many bits are set in a field's own bytes.
    ///
    /// Over the field's bits rather than its bytes: a vector of five items
    /// lives in one byte, and the three bits past the end of it are padding
    /// that a format is free to leave as it finds them. Counting the whole
    /// byte would let that padding decide how many rows the table after it
    /// has.
    ///
    /// Bits are numbered from the top of each byte, which is how every format
    /// that writes one of these lays it out: 7z, ZIP's extra-field vectors and
    /// PNG's interlace passes all read the first item out of the high bit.
    fn set_bits<S: Source>(&mut self, doc: &Document<S>, path: &[usize], what: &str) -> R<i128> {
        self.resolve(doc, path)?;
        let size = self.size_of(doc, path)?;
        let r = self.memo[path].clone();
        let bytes = self.read(doc, &r, r.offset, size)?;
        let mut set: i128 = 0;
        for bit in 0..size {
            let byte = bytes.get((bit / 8) as usize).copied().unwrap_or(0);
            if byte >> (7 - (bit % 8)) & 1 == 1 {
                set += 1;
            }
        }
        let _ = what;
        Ok(set)
    }

    /// The largest number in a list. An empty list answers zero.
    fn maximum<S: Source>(&mut self, doc: &Document<S>, path: &[usize], what: &str) -> R<i128> {
        let n = self.child_count(doc, path)?;
        let mut largest = 0i128;
        let mut child = path.to_vec();
        for i in 0..n as usize {
            child.push(i);
            let value = self.value_of(doc, &child)?.value.as_int();
            child.pop();
            let Some(value) = value else { return fail(format!("{what} holds no number there")) };
            largest = largest.max(value);
        }
        Ok(largest)
    }

    /// Field `name` of the element before this one, in the nearest enclosing
    /// list. Zero for the first element and outside a list, which is what lets
    /// `Or` fall through to the case for a message with no state behind it.
    ///
    /// The elements of a list are resolved in order, so by the time element `n`
    /// asks, element `n - 1` is already in the memo: this is a lookup, not a
    /// walk back to the start.
    fn prev_field<S: Source>(&mut self, doc: &Document<S>, at: &[usize], name: &str) -> R<i128> {
        // Only the innermost list: the element before this one is a question
        // about the list this element is in, and nothing outside it.
        if let Some((cur, idx)) = self.enclosing_lists(at).into_iter().next() {
            if idx == 0 {
                return Ok(0);
            }
            let mut elem = cur.clone();
            elem.push(idx - 1);
            self.resolve(doc, &elem)?;
            let Ty::Struct(s) = self.memo[&elem].ty.base() else { return Ok(0) };
            let Some(j) = s.fields.iter().position(|f| *f.name == *name) else { return Ok(0) };
            elem.push(j);
            return Ok(self.value_of(doc, &elem)?.value.as_int().unwrap_or(0));
        }
        Ok(0)
    }

    /// The value at `field` in the nearest earlier element of the enclosing
    /// list that has one. Elements between are passed over: a WAVE file can put
    /// `fact` or `LIST` between `fmt ` and `data`, and the samples are still
    /// the width `fmt ` gave them.
    ///
    /// This reads earlier elements, which in a list long enough to be walked
    /// with its middle dropped means placing them again. Formats that need it
    /// have tens of elements, not millions; it does not belong in a long list.
    fn sibling_field<S: Source>(&mut self, doc: &Document<S>, at: &[usize], field: &[String]) -> R<i128> {
        for (cur, idx) in self.enclosing_lists(at) {
            for earlier in (0..idx).rev() {
                let mut elem = cur.clone();
                elem.push(earlier);
                if let Some(v) = self.field_in(doc, &mut elem, field)? {
                    return Ok(v);
                }
            }
            // Nothing in this list has it, so ask the list this one sits in.
            // A WAVE sample is inside a frame, inside the samples of a chunk,
            // and what declared its width is a chunk two levels out.
        }
        Ok(0)
    }

    /// Where the field [`Expr::Sibling`] would read is, rather than what it
    /// says. The same backwards walk, over the same lists, stopping at the
    /// first element the path goes all the way down in.
    ///
    /// A check needs the place and not the number: what it covers is a run of
    /// bytes, and a ZIP data descriptor's `crc32` is over the data of the
    /// local entry written before it. Kept beside `sibling_field` and sharing
    /// `enclosing_lists` with it, so the two cannot come to disagree about
    /// which elements count as earlier.
    ///
    /// One difference, and it is deliberate. `sibling_field` walks past an
    /// element that has the field but holds no number in it, because a value
    /// is what it was asked for; this stops there, because the field is there
    /// and its bytes are what was asked for.
    pub(super) fn sibling_field_path<S: Source>(
        &mut self,
        doc: &Document<S>,
        at: &[usize],
        field: &[String],
    ) -> R<Option<Vec<usize>>> {
        for (cur, idx) in self.enclosing_lists(at) {
            for earlier in (0..idx).rev() {
                let mut elem = cur.clone();
                elem.push(earlier);
                match self.descend(doc, &mut elem, field) {
                    Ok(true) => return Ok(Some(elem)),
                    Ok(false) => {}
                    Err(e) if passes_up(&e) => return Err(e),
                    // An element that will not read is not an element that
                    // answers no: it is one this search cannot see into, and
                    // the one before it may still be the right answer.
                    Err(_) => {}
                }
            }
        }
        Ok(None)
    }

    /// The lists this node sits in, innermost first, each with the index this
    /// node has in it.
    ///
    /// Four things ask this question and three of them used to answer it
    /// themselves: `Idx` wants the innermost index, `Prev` the element before,
    /// `Sibling` the elements before that one, and a tagged search over an
    /// enclosing list all of them. Two walks that disagreed about what counts
    /// as a list would put two of those answers in different lists.
    pub(super) fn enclosing_lists(&self, at: &[usize]) -> Vec<(Vec<usize>, usize)> {
        let mut out = Vec::new();
        let mut cur = at.to_vec();
        while let Some(idx) = cur.pop() {
            let listy = matches!(
                self.memo.get(&cur).map(|r| &r.ty),
                Some(Ty::Array { .. } | Ty::Repeat { .. } | Ty::PointerList { .. } | Ty::Chain { .. } | Ty::Gather { .. })
            );
            if listy {
                out.push((cur.clone(), idx));
            }
        }
        out
    }

    /// The label a tagged search is looking for, worked out once before the
    /// search rather than once per element: a computed label reads a field of
    /// the record asking, and that answer is the same however many elements
    /// are tried against it.
    pub(super) fn tag_now<S: Source>(&mut self, doc: &Document<S>, at: &[usize], tag: &Tag, here: Option<(u64, u64)>) -> R<Tag> {
        Ok(match tag {
            Tag::Computed(e) => Tag::Int(self.eval_expr_at(doc, at, e, here)?),
            Tag::ComputedText(e) => Tag::Text(self.text_at(doc, at, &e.clone(), here)?),
            other => other.clone(),
        })
    }

    /// Where a tagged search lands: the path of the field it names, and how a
    /// reader would name it. `None` when no element carries the label, or when
    /// the one that does has no such field.
    ///
    /// Two searches, by the same rule. A named list is read from the start,
    /// because a list of records the format fixed the numbering of has no
    /// order worth respecting. The enclosing list is read backwards from the
    /// element asking, and never past it: the elements after this one have not
    /// been placed, and placing them is what is asking. A record that defines
    /// another is written before it in every format that has both.
    pub(super) fn tagged_path<S: Source>(
        &mut self,
        doc: &Document<S>,
        at: &[usize],
        t: &TaggedRef,
        here: Option<(u64, u64)>,
    ) -> R<Option<(Vec<usize>, String)>> {
        let tag = self.tag_now(doc, at, &t.tag, here)?;
        let Some(array) = t.array.clone() else {
            for (list, mine) in self.enclosing_lists(at) {
                for i in (0..mine).rev() {
                    let mut p = list.clone();
                    p.push(i);
                    if self.tag_matches(doc, &p, &t.key, &tag)? {
                        return self.tagged_landing(doc, &list, i, t, None);
                    }
                }
            }
            return Ok(None);
        };
        let Some(list) = self.text_path(doc, at, &array, here)? else { return Ok(None) };
        let Some((found, i)) = self.tag_search(doc, &list, &t.key, &tag)? else { return Ok(None) };
        self.tagged_landing(doc, &found, i, t, Some(&array))
    }

    /// Where a search that has found its element ends up: the field it names
    /// inside that element, and the reading a person would give the whole
    /// journey.
    fn tagged_landing<S: Source>(
        &mut self,
        doc: &Document<S>,
        list: &[usize],
        i: usize,
        t: &TaggedRef,
        array: Option<&Expr>,
    ) -> R<Option<(Vec<usize>, String)>> {
        let mut p = list.to_vec();
        p.push(i);
        // Named for the list it was found in, which for the enclosing list is
        // whatever that list is called where it was declared.
        let name = match array {
            Some(array) => write_expr(array).unwrap_or_else(|| "collection".into()),
            None => self.memo.get(list).map_or_else(String::new, |r| r.name.text()),
        };
        let mut label = format!("{name}[{i}]");
        if !self.descend(doc, &mut p, &t.field)? {
            return Ok(None);
        }
        for f in t.field.iter() {
            label = format!("{label}.{f}");
        }
        Ok(Some((p, label)))
    }

    /// Which element of a named list carries this label, and which walk of
    /// that list answered.
    ///
    /// The answer is not always a path inside the list that was asked for. A
    /// list is bytes, and several fields can be readings of one stretch of
    /// them: an HDF5 global heap collection is reached through the address
    /// each variable-length element carries, so a column of two thousand
    /// strings holds two thousand paths to one collection. Answering each of
    /// them from its own walk is quadratic in the length of the column, and
    /// there is nothing to be gained by it, since every walk reads the same
    /// bytes and finds the same thing.
    ///
    /// So what was learned is kept by the stretch rather than by the path, and
    /// the path that comes back is whichever one walked it. See
    /// [`super::memo::TagIndex`].
    fn tag_search<S: Source>(
        &mut self,
        doc: &Document<S>,
        list: &[usize],
        key: &Arc<[String]>,
        tag: &Tag,
    ) -> R<Option<(Vec<usize>, usize)>> {
        let Some(want) = tag_key(tag) else { return self.tag_scan(doc, list, key, tag) };
        self.resolve(doc, list)?;
        let r = &self.memo[list];
        let slot = (r.space, r.offset, r.limit, key.clone());
        // A label the index has seen, checked against the element it names
        // before it is believed. The index is keyed by where the bytes are and
        // not by what read them, which is the whole saving; this is what makes
        // that safe where two readings of one stretch disagree.
        let hit = self.memo.tag_index(&slot).and_then(|ix| ix.found.get(&want).map(|i| (ix.list.clone(), *i)));
        if let Some((found, i)) = hit {
            let mut p = found.clone();
            p.push(i);
            if self.tag_matches(doc, &p, key, tag)? {
                return Ok(Some((found, i)));
            }
            self.memo.forget_tags(&slot);
        }
        let (walking, from, full) = {
            let ix = self.memo.tag_index_mut(slot.clone(), list);
            (ix.list.clone(), ix.scanned, ix.full)
        };
        // An index that stopped growing can say nothing about the elements
        // past where it stopped, so a search that misses goes back to reading
        // the list it was handed.
        if full {
            return self.tag_scan(doc, list, key, tag);
        }
        let n = self.child_count(doc, &walking)? as usize;
        for i in from..n {
            let mut p = walking.clone();
            p.push(i);
            let Some(k) = self.tag_key_of(doc, &p, key, tag)? else { continue };
            self.memo.remember_tag(&slot, k.clone(), i);
            if k == want {
                self.memo.tag_scanned(&slot, i + 1);
                return Ok(Some((walking, i)));
            }
        }
        self.memo.tag_scanned(&slot, n);
        Ok(None)
    }

    /// The same question asked of one list from the front, with nothing
    /// remembered. What a label that cannot be written down as a key gets, and
    /// what a list too large to index falls back to.
    fn tag_scan<S: Source>(
        &mut self,
        doc: &Document<S>,
        list: &[usize],
        key: &[String],
        tag: &Tag,
    ) -> R<Option<(Vec<usize>, usize)>> {
        let n = self.child_count(doc, list)? as usize;
        for i in 0..n {
            let mut p = list.to_vec();
            p.push(i);
            if self.tag_matches(doc, &p, key, tag)? {
                return Ok(Some((list.to_vec(), i)));
            }
        }
        Ok(None)
    }

    /// Which child of the node at `path` is called `name`: a field of a
    /// structure, a key of a JSON object, or an index of a JSON array written
    /// as a number. None when it has no such child.
    pub(super) fn child_index<S: Source>(&mut self, doc: &Document<S>, path: &[usize], name: &str) -> R<Option<usize>> {
        self.resolve(doc, path)?;
        if matches!(self.memo[path].ty, Ty::Json(..)) {
            return self.json_index(doc, path, name);
        }
        if matches!(self.memo[path].ty, Ty::Pickle(..)) {
            return self.pickle_index(doc, path, name);
        }
        // A list has no named children, so a number is the only thing a path
        // can mean there, and it means the same thing it means in JSON. What
        // needs it is a format that wraps a value in a list of parts: a FITS
        // quoted string is a run of pieces, and the text of one is reached by
        // saying which piece.
        if matches!(
            self.memo[path].ty,
            Ty::Array { .. } | Ty::Repeat { .. } | Ty::PointerList { .. } | Ty::Chain { .. } | Ty::Gather { .. }
        ) {
            let Ok(i) = name.parse::<usize>() else { return Ok(None) };
            let n = self.child_count(doc, path)?;
            return Ok(((i as u64) < n).then_some(i));
        }
        let Ty::Struct(s) = self.memo[path].ty.base() else { return Ok(None) };
        Ok(s.fields.iter().position(|f| *f.name == *name))
    }

    /// Walk `field` down from the node at `path`, a name at a time. False when
    /// one of the names is not there, leaving `path` as far as it got.
    pub(super) fn descend<S: Source>(&mut self, doc: &Document<S>, path: &mut Vec<usize>, field: &[String]) -> R<bool> {
        for name in field {
            match self.child_index(doc, path, name)? {
                Some(j) => path.push(j),
                None => return Ok(false),
            }
            // A field whose contents are somewhere else in the file is its
            // contents, here as in `find_field`: naming it means the table it
            // points at, not the nothing that stands in its place. Without
            // this a path could name such a field and then go no further.
            self.resolve(doc, path)?;
            if matches!(self.memo[path].ty, Ty::At { .. }) {
                path.push(0);
            }
        }
        Ok(true)
    }

    /// Where a path down into an earlier field lands: the first name is a
    /// field declared before this one, and the rest go inside it. What
    /// [`Expr::Within`] and [`Expr::ElemWithin`] both start with.
    pub(super) fn within_path<S: Source>(
        &mut self,
        doc: &Document<S>,
        at: &[usize],
        field: &[String],
    ) -> R<Vec<usize>> {
        let Some((first, rest)) = field.split_first() else { return fail("no field named") };
        let Some(mut p) = self.find_field(at, first) else {
            return fail(format!("unknown field {first}"));
        };
        // A field whose contents are somewhere else in the file is its
        // contents, here as in `descend`. `find_field` steps through an `At`
        // the declaration shows it; a switch that chose one shows nothing
        // until the field has been read, and an HDF5 address is written that
        // way because the format spells "nowhere" as an address of its own.
        self.resolve(doc, &p)?;
        if matches!(self.memo[&p].ty, Ty::At { .. }) {
            p.push(0);
        }
        if !self.descend(doc, &mut p, rest)? {
            return fail(format!("{first} has no field named {}", rest.join(".")));
        }
        Ok(p)
    }

    /// Follow `field` down from the node at `path`, through whatever the
    /// template resolved it to, and read the number at the end. None when this
    /// node has no such field, which is how a search over siblings passes over
    /// the ones that are something else.
    /// Whether the element at `elem` is the one a tagged lookup is after: the
    /// number at `key` matches, or the bytes at `key` are written exactly that
    /// way. An element that has no such field, or one that cannot be read, is
    /// not a match, which is how the search passes over the records of a list
    /// that are something else.
    pub(super) fn tag_matches<S: Source>(&mut self, doc: &Document<S>, elem: &[usize], key: &[String], tag: &Tag) -> R<bool> {
        // A computed label was worked out before the search began, so what
        // arrives here is always written down. See `tag_now`.
        let Some(want) = tag_key(tag) else {
            return fail("a computed label must be worked out before the search");
        };
        Ok(self.tag_key_of(doc, elem, key, tag)? == Some(want))
    }

    /// What this element is labelled, read the way the label being looked for
    /// is written: a number against a number, text against text, bytes against
    /// bytes. None where the element has no such field or it cannot be read,
    /// which is how a search passes over the records of a list that are
    /// something else.
    ///
    /// The one place an element's label is read, so that what a search
    /// compares and what an index remembers cannot drift apart.
    fn tag_key_of<S: Source>(
        &mut self,
        doc: &Document<S>,
        elem: &[usize],
        key: &[String],
        tag: &Tag,
    ) -> R<Option<TagKey>> {
        match tag {
            Tag::Computed(_) | Tag::ComputedText(_) => {
                fail("a computed label must be worked out before the search")
            }
            Tag::Int(_) => Ok(self.field_in(doc, &mut elem.to_vec(), key)?.map(TagKey::Int)),
            // Text against text, both sides read the same way. `Bytes`
            // compares what is written and so has the padding of a fixed-width
            // key in it; this compares what the two fields read as, which is
            // the only comparison a label worked out somewhere else can win.
            Tag::Text(_) => {
                let mut p = elem.to_vec();
                match self.descend(doc, &mut p, key) {
                    Ok(true) => {}
                    Ok(false) => return Ok(None),
                    Err(e) if passes_up(&e) => return Err(e),
                    Err(_) => return Ok(None),
                }
                match self.text_of(doc, &p) {
                    Ok(got) => Ok(Some(TagKey::Text(got.trim_end().to_string()))),
                    Err(e) if passes_up(&e) => Err(e),
                    Err(_) => Ok(None),
                }
            }
            Tag::Bytes(_) => {
                let mut p = elem.to_vec();
                let Some((last, above)) = key.split_last() else { return Ok(None) };
                match self.descend(doc, &mut p, above) {
                    Ok(true) => {}
                    Ok(false) => return Ok(None),
                    Err(e) if passes_up(&e) => return Err(e),
                    Err(_) => return Ok(None),
                }
                match self.child_raw_bytes(doc, &p, last) {
                    Ok(got) => Ok(Some(TagKey::Bytes(got))),
                    Err(e) if passes_up(&e) => Err(e),
                    Err(_) => Ok(None),
                }
            }
        }
    }

    fn field_in<S: Source>(&mut self, doc: &Document<S>, path: &mut Vec<usize>, field: &[String]) -> R<Option<i128>> {
        match self.descend(doc, path, field) {
            Ok(true) => {}
            Ok(false) => return Ok(None),
            Err(e) if passes_up(&e) => return Err(e),
            Err(_) => return Ok(None),
        }
        Ok(match self.value_of(doc, path) {
            Ok(info) => info.value.as_int(),
            // A field that cannot be read yet is not an answer, and must not be
            // taken for the absence of one.
            Err(e) if passes_up(&e) => return Err(e),
            Err(_) => None,
        })
    }

    /// The path of the field named `name`, found the way `lookup` finds it.
    /// The path to `array[index]`, then down the named fields inside it:
    /// `tensors[i].offset` is a number, `tensors[i].dims` is an array, and
    /// getting to either is the same walk.
    /// The path to `path[index].field`, where `path` goes down into a field
    /// declared before this one rather than naming a sibling. See
    /// [`Expr::ElemWithin`].
    pub(super) fn elem_within_path<S: Source>(
        &mut self,
        doc: &Document<S>,
        at: &[usize],
        path: &[String],
        index: &Expr,
        field: &[String],
        here: Option<(u64, u64)>,
    ) -> R<Vec<usize>> {
        let i = self.eval_expr_at(doc, at, index, here)?;
        if i < 0 {
            return fail("negative index");
        }
        let mut p = self.within_path(doc, at, path)?;
        p.push(i as usize);
        if !self.descend(doc, &mut p, field)? {
            return fail(format!("{}[{i}] has no field named {}", path.join("."), field.join(".")));
        }
        Ok(p)
    }

    pub(super) fn elem_path<S: Source>(
        &mut self,
        doc: &Document<S>,
        at: &[usize],
        array: &str,
        index: &Expr,
        field: &[String],
        here: Option<(u64, u64)>,
    ) -> R<Vec<usize>> {
        let i = self.eval_expr_at(doc, at, index, here)?;
        if i < 0 {
            return fail("negative index");
        }
        let Some(mut p) = self.find_field(at, array) else {
            return fail(format!("unknown field {array}"));
        };
        p.push(i as usize);
        if !self.descend(doc, &mut p, field)? {
            return fail(format!("{array}[{i}] has no field named {}", field.join(".")));
        }
        Ok(p)
    }

    pub(super) fn find_field(&self, at: &[usize], name: &str) -> Option<Vec<usize>> {
        let mut cur = at.to_vec();
        while let Some(idx) = cur.pop() {
            if let Some(Ty::Struct(s)) = self.memo.get(&cur).map(|r| &r.ty) {
                if let Some(j) = s.fields.iter().take(idx).position(|f| *f.name == *name) {
                    let mut p = cur.clone();
                    p.push(j);
                    // A field whose contents are elsewhere is its contents:
                    // naming it means the table it points at, not the nothing
                    // that stands in its place.
                    if matches!(s.fields[j].ty, Ty::At { .. }) {
                        p.push(0);
                    }
                    return Some(p);
                }
            }
        }
        None
    }

    /// Find `name` among the fields before `at` in its struct, then in
    /// enclosing structs. Returns its value and its size in bytes.
    pub(super) fn lookup<S: Source>(&mut self, doc: &Document<S>, at: &[usize], name: &str) -> R<(Option<i128>, i128)> {
        let (v, bits) = self.lookup_bits(doc, at, name)?;
        Ok((v, bits / 8))
    }

    /// The same, measured in bits, which is what a field packed tighter than a
    /// byte has to be measured in.
    pub(super) fn lookup_bits<S: Source>(&mut self, doc: &Document<S>, at: &[usize], name: &str) -> R<(Option<i128>, i128)> {
        let mut cur = at.to_vec();
        while !cur.is_empty() {
            let idx = cur.pop().expect("non-empty");
            let parent = cur.clone();
            if let Ty::Struct(s) = &self.memo[&parent].ty {
                if let Some(j) = s.fields[..idx].iter().position(|f| *f.name == *name) {
                    let pointing = matches!(s.fields[j].ty, Ty::At { .. });
                    let mut p = parent;
                    p.push(j);
                    // As in `find_field`: what it points at is what it is.
                    if pointing {
                        p.push(0);
                    }
                    let info = self.value_of(doc, &p)?;
                    // A field the file did not write holds nothing, and
                    // nothing is not zero. Left to read as the empty node it
                    // is, a switch keyed on an absent field would quietly
                    // take case 0 and a length would quietly be none, which
                    // is the file being read wrongly with nothing said. See
                    // [`NodeInfo::absent`].
                    if info.absent {
                        return fail(format!("{name} is not in this file"));
                    }
                    // A field with no numeric reading can still be measured.
                    return Ok((info.value.as_int(), info.size_bits as i128));
                }
            }
        }
        fail(format!("unknown field {name}"))
    }
}

/// Which end of a container a block walk starts from.
///
/// This is most of what a search costs. Reading forward and stopping at the
/// first hit is what finding the first of something means; reading backward and
/// stopping at the first hit is what finding the last of something means, and
/// reading forward to the end to be sure nothing came after is not. For a
/// reader holding a window rather than a whole file that is the difference
/// between opening a file and not opening it: a PDF's pointer to its table is
/// written forty bytes from the end, and looking for it forwards means fetching
/// every chunk of a three hundred megabyte file to reach it, dropping the ones
/// fetched first to make room, and starting over from the front the next time
/// it is asked, which never finishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Dir {
    Forward,
    Backward,
}

/// Read a container a block at a time and hand each block to `look`, which says
/// where in that block it found what it was after. The answer is counted from
/// `offset`; `None` means the container held no such thing.
///
/// Consecutive blocks overlap by `overlap` bytes, so something written across
/// the seam between two of them is whole in one of the two. A search for a word
/// overlaps by all but one byte of it; one that decides a byte by the byte
/// after it overlaps by one.
///
/// Reading stops at the first block whose bytes have not been loaded, and the
/// caller fetches them and asks again. Which end the walk starts from decides
/// whether that ever ends: see [`Dir`].
fn scan_blocks<S: Source>(
    ev: &Evaluator,
    doc: &Document<S>,
    space: u32,
    offset: u64,
    total: u64,
    overlap: u64,
    dir: Dir,
    mut look: impl FnMut(&[u8]) -> Option<usize>,
) -> R<Option<u64>> {
    const BLOCK: u64 = 4096;
    // A block has to be longer than the overlap, or the walk never moves on.
    let step = BLOCK.max(overlap + 1);
    let mut buf = Vec::new();
    // Through the evaluator rather than the document: the run being walked may
    // be a decoded stream's, and the file at the same offset is other bytes.
    let read = |from: u64, want: u64, buf: &mut Vec<u8>| -> R<()> {
        *buf = ev.read_in(doc, space, offset + from * 8, want * 8)?;
        Ok(())
    };
    match dir {
        Dir::Forward => {
            let mut at = 0u64;
            while at + overlap < total {
                let want = step.min(total - at);
                read(at, want, &mut buf)?;
                if let Some(i) = look(&buf) {
                    return Ok(Some(at + i as u64));
                }
                if want < step {
                    break;
                }
                at += step - overlap;
            }
        }
        Dir::Backward => {
            let mut end = total;
            while end > overlap {
                let want = step.min(end);
                let from = end - want;
                read(from, want, &mut buf)?;
                if let Some(i) = look(&buf) {
                    return Ok(Some(from + i as u64));
                }
                if from == 0 {
                    break;
                }
                end = from + overlap;
            }
        }
    }
    Ok(None)
}
