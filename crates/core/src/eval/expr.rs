//! Working out the numbers a template asks for: how long a field is, how many
//! elements a list holds, which case a switch takes.
//!
//! Every expression looks backwards or inwards, never forwards: at a field
//! before this one, at an element before this one, at the bytes this field
//! starts with. That is what lets an edit keep everything the bytes before it
//! settled, and what makes a file readable from the front.

use super::*;

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
        Ok(match e {
            Expr::Lit(v) => *v,
            Expr::Remaining => match here {
                Some((offset, limit)) if limit >= offset => ((limit - offset) / 8) as i128,
                _ => return fail("nothing to measure the rest of"),
            },
            // The index of the element this sits in, which is what a field
            // whose type comes from a list read earlier needs.
            Expr::Idx => {
                let mut cur = at.to_vec();
                let mut found = 0;
                while let Some(idx) = cur.pop() {
                    let listy = self
                        .memo
                        .get(&cur)
                        .map(|r| matches!(r.ty, Ty::Array { .. } | Ty::Repeat { .. } | Ty::PointerList { .. }));
                    if listy == Some(true) {
                        found = idx as i128;
                        break;
                    }
                }
                found
            }
            Expr::Elem { array, index, field } => {
                let p = self.elem_path(doc, at, array, index, field, here)?;
                match self.node(doc, &p)?.value.as_int() {
                    Some(v) => v,
                    None => return fail(format!("{array} holds no number there")),
                }
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
            Expr::Ref(name) => match self.lookup(doc, at, name)? {
                (Some(v), _) => v,
                (None, _) => return fail(format!("{name} is not a number")),
            },
            Expr::SizeOf(name) => self.lookup(doc, at, name)?.1,
            // Read where this field starts without taking the bits: what a
            // field that exists only when the byte says so has to ask.
            Expr::Peek { bits, endian } => {
                let Some((offset, limit)) = here else { return fail("nothing to look at") };
                if offset + u64::from(*bits) > limit {
                    return fail("looks past the end of its container");
                }
                let mut buf = vec![0u8; bytes_for(u64::from(*bits))];
                let missing = doc.read_bits(offset, u64::from(*bits), &mut buf);
                if !missing.is_empty() {
                    return Err(EvalError::Pending(missing));
                }
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
                let mut buf = vec![0u8; bytes_for(u64::from(*bits))];
                let missing = doc.read_bits(from, u64::from(*bits), &mut buf);
                if !missing.is_empty() {
                    return Err(EvalError::Pending(missing));
                }
                read_uint(&buf, *bits, *endian) as i128
            }
            // Walk forward for the byte that ends an unmeasured stream. A lead
            // byte is told apart from a marker by the byte after it, so blocks
            // overlap by one: a lead byte at the end of one block is the first
            // byte of the next, where its successor has arrived.
            Expr::ToMarker { lead, unless } => {
                let Some((offset, limit)) = here else { return fail("nothing to measure") };
                if limit < offset {
                    return fail("nothing to measure");
                }
                let total = (limit - offset) / 8;
                let (lead, unless) = (*lead, unless.clone());
                let hit = scan_blocks(doc, offset, total, 1, Dir::Forward, |b| {
                    (0..b.len()).find(|&i| b[i] == lead && b.get(i + 1).is_some_and(|n| !unless.contains(n)))
                })?;
                // A lead byte with nothing after it is not a marker: nothing
                // has said so, so the run measures to the end.
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
                let hit = scan_blocks(doc, offset, total, n as u64 - 1, dir, |b| match dir {
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
                let Some((first, rest)) = field.split_first() else { return fail("no field named") };
                let Some(mut p) = self.find_field(at, first) else {
                    return fail(format!("unknown field {first}"));
                };
                if !self.descend(doc, &mut p, rest)? {
                    return fail(format!("{first} has no field named {}", rest.join(".")));
                }
                match self.node(doc, &p)?.value.as_int() {
                    Some(v) => v,
                    None => return fail(format!("{} holds no number", field.join("."))),
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
            Expr::Less(a, b) => {
                i128::from(self.eval_expr_at(doc, at, a, here)? < self.eval_expr_at(doc, at, b, here)?)
            }
            Expr::Div(a, b) => {
                let d = self.eval_expr_at(doc, at, b, here)?;
                if d == 0 {
                    return fail("division by zero");
                }
                self.eval_expr_at(doc, at, a, here)? / d
            }
        })
    }

    /// The text of the field an expression names, for a switch keyed on words
    /// rather than on numbers. Only an expression that names a field can be
    /// read as text: `Ref` for one beside it, `Elem` for one inside a list.
    pub(super) fn text_at<S: Source>(
        &mut self,
        doc: &Document<S>,
        at: &[usize],
        e: &Expr,
        here: Option<(u64, u64)>,
    ) -> R<String> {
        let p = match e {
            Expr::Ref(name) => match self.find_field(at, name) {
                Some(p) => p,
                None => return fail(format!("unknown field {name}")),
            },
            Expr::Elem { array, index, field } => self.elem_path(doc, at, array, index, field, here)?,
            _ => return fail("a switch on text has to name a field"),
        };
        match self.node(doc, &p)?.value {
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
            let v = self.node(doc, &child)?.value.as_int();
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
            let v = self.node(doc, &child)?.value.as_int();
            child.pop();
            let Some(v) = v else { return fail(format!("{what} holds no number there")) };
            let Some(next) = total.checked_add(v) else { return fail("too many to count") };
            total = next;
        }
        Ok(total)
    }

    /// The largest number in a list. An empty list answers zero.
    fn maximum<S: Source>(&mut self, doc: &Document<S>, path: &[usize], what: &str) -> R<i128> {
        let n = self.child_count(doc, path)?;
        let mut largest = 0i128;
        let mut child = path.to_vec();
        for i in 0..n as usize {
            child.push(i);
            let value = self.node(doc, &child)?.value.as_int();
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
        let mut cur = at.to_vec();
        while let Some(idx) = cur.pop() {
            let listy = matches!(
                self.memo.get(&cur).map(|r| &r.ty),
                Some(Ty::Array { .. } | Ty::Repeat { .. } | Ty::PointerList { .. })
            );
            if !listy {
                continue;
            }
            if idx == 0 {
                return Ok(0);
            }
            let mut elem = cur.clone();
            elem.push(idx - 1);
            self.resolve(doc, &elem)?;
            let Ty::Struct(s) = self.memo[&elem].ty.base() else { return Ok(0) };
            let Some(j) = s.fields.iter().position(|f| *f.name == *name) else { return Ok(0) };
            elem.push(j);
            return Ok(self.node(doc, &elem)?.value.as_int().unwrap_or(0));
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
        let mut cur = at.to_vec();
        while let Some(idx) = cur.pop() {
            let listy = matches!(
                self.memo.get(&cur).map(|r| &r.ty),
                Some(Ty::Array { .. } | Ty::Repeat { .. } | Ty::PointerList { .. })
            );
            if !listy {
                continue;
            }
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

    /// Which child of the node at `path` is called `name`: a field of a
    /// structure, a key of a JSON object, or an index of a JSON array written
    /// as a number. None when it has no such child.
    pub(super) fn child_index<S: Source>(&mut self, doc: &Document<S>, path: &[usize], name: &str) -> R<Option<usize>> {
        self.resolve(doc, path)?;
        if matches!(self.memo[path].ty, Ty::Json(_)) {
            return self.json_index(doc, path, name);
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
        }
        Ok(true)
    }

    /// Follow `field` down from the node at `path`, through whatever the
    /// template resolved it to, and read the number at the end. None when this
    /// node has no such field, which is how a search over siblings passes over
    /// the ones that are something else.
    fn field_in<S: Source>(&mut self, doc: &Document<S>, path: &mut Vec<usize>, field: &[String]) -> R<Option<i128>> {
        match self.descend(doc, path, field) {
            Ok(true) => {}
            Ok(false) => return Ok(None),
            Err(e) if e.interrupted() => return Err(e),
            Err(_) => return Ok(None),
        }
        Ok(match self.node(doc, path) {
            Ok(info) => info.value.as_int(),
            // A field that cannot be read yet is not an answer, and must not be
            // taken for the absence of one.
            Err(e) if e.interrupted() => return Err(e),
            Err(_) => None,
        })
    }

    /// The path of the field named `name`, found the way `lookup` finds it.
    /// The path to `array[index]`, then down the named fields inside it:
    /// `tensors[i].offset` is a number, `tensors[i].dims` is an array, and
    /// getting to either is the same walk.
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
                    let info = self.node(doc, &p)?;
                    // A field with no numeric reading can still be measured.
                    return Ok((info.value.as_int(), (info.size_bits / 8) as i128));
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
    doc: &Document<S>,
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
    let read = |from: u64, want: u64, buf: &mut Vec<u8>| -> R<()> {
        buf.resize(want as usize, 0);
        let missing = doc.read_bits(offset + from * 8, want * 8, buf);
        if missing.is_empty() { Ok(()) } else { Err(EvalError::Pending(missing)) }
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
