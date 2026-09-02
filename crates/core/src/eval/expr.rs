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
            Expr::Idx => self.enclosing_lists(at).first().map_or(0, |(_, i)| *i as i128),
            Expr::Elem { array, index, field } => {
                let p = self.elem_path(doc, at, array, index, field, here)?;
                match self.node(doc, &p)?.value.as_int() {
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
                    Some((p, _)) => self.node(doc, &p)?.value.as_int().unwrap_or(0),
                    None => 0,
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
            Expr::BitsOf(name) => self.lookup_bits(doc, at, name)?.1,
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
                let from = match lsb_packed(*bits, *endian, from) {
                    true => match lsb_offset(*bits, from) {
                        Some(at) => at,
                        None => return fail("a peek packed low-bit-first would cross a byte boundary"),
                    },
                    false => from,
                };
                let mut buf = vec![0u8; bytes_for(u64::from(*bits))];
                let missing = doc.read_bits(from, u64::from(*bits), &mut buf);
                if !missing.is_empty() {
                    return Err(EvalError::Pending(missing));
                }
                read_uint(&buf, *bits, *endian) as i128
            }
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
                let hit = scan_blocks(doc, offset, total, overlap, Dir::Forward, |b| {
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
                let p = self.within_path(doc, at, &field)?;
                match self.node(doc, &p)?.value.as_int() {
                    Some(v) => v,
                    None => return fail(format!("{} holds no number", field.join("."))),
                }
            }
            // A list inside an earlier field, indexed. Reached in two steps
            // because a name reaches only a sibling.
            Expr::ElemWithin { path, index, field } => {
                let p = self.elem_within_path(doc, at, path, index, field, here)?;
                match self.node(doc, &p)?.value.as_int() {
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
        match self.node(doc, path)?.value {
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
                Some(Ty::Array { .. } | Ty::Repeat { .. } | Ty::PointerList { .. } | Ty::Chain { .. })
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
    fn tag_now<S: Source>(&mut self, doc: &Document<S>, at: &[usize], tag: &Tag, here: Option<(u64, u64)>) -> R<Tag> {
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
        let mut tried: Vec<(Vec<usize>, usize)> = Vec::new();
        match &t.array {
            Some(array) => {
                let Some(list) = self.find_field(at, array) else { return fail(format!("unknown field {array}")) };
                let n = self.child_count(doc, &list)?;
                tried.extend((0..n as usize).map(|i| (list.clone(), i)));
            }
            None => {
                for (list, mine) in self.enclosing_lists(at) {
                    tried.extend((0..mine).rev().map(|i| (list.clone(), i)));
                }
            }
        }
        for (list, i) in tried {
            let mut p = list.clone();
            p.push(i);
            if !self.tag_matches(doc, &p, &t.key, &tag)? {
                continue;
            }
            // Named for the list it was found in, which for the enclosing list
            // is whatever that list is called where it was declared.
            let name = match &t.array {
                Some(array) => array.to_string(),
                None => self.memo.get(&list).map_or_else(String::new, |r| r.name.text()),
            };
            let mut label = format!("{name}[{i}]");
            if !self.descend(doc, &mut p, &t.field)? {
                return Ok(None);
            }
            for f in t.field.iter() {
                label = format!("{label}.{f}");
            }
            return Ok(Some((p, label)));
        }
        Ok(None)
    }

    /// Which child of the node at `path` is called `name`: a field of a
    /// structure, a key of a JSON object, or an index of a JSON array written
    /// as a number. None when it has no such child.
    pub(super) fn child_index<S: Source>(&mut self, doc: &Document<S>, path: &[usize], name: &str) -> R<Option<usize>> {
        self.resolve(doc, path)?;
        if matches!(self.memo[path].ty, Ty::Json(_)) {
            return self.json_index(doc, path, name);
        }
        // A list has no named children, so a number is the only thing a path
        // can mean there, and it means the same thing it means in JSON. What
        // needs it is a format that wraps a value in a list of parts: a FITS
        // quoted string is a run of pieces, and the text of one is reached by
        // saying which piece.
        if matches!(self.memo[path].ty, Ty::Array { .. } | Ty::Repeat { .. } | Ty::PointerList { .. } | Ty::Chain { .. }) {
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
        match tag {
            // A computed label was worked out before the search began, so what
            // arrives here is always a number. See `tag_now`.
            Tag::Computed(_) | Tag::ComputedText(_) => {
                fail("a computed label must be worked out before the search")
            }
            Tag::Int(want) => Ok(self.field_in(doc, &mut elem.to_vec(), key)? == Some(*want)),
            // Text against text, both sides read the same way. `Bytes`
            // compares what is written and so has the padding of a fixed-width
            // key in it; this compares what the two fields read as, which is
            // the only comparison a label worked out somewhere else can win.
            // An element with no such field, or one that cannot be read as
            // text, is not a match rather than an error, the same as for the
            // other two: a list holds records that are something else.
            Tag::Text(want) => {
                let mut p = elem.to_vec();
                match self.descend(doc, &mut p, key) {
                    Ok(true) => {}
                    Ok(false) => return Ok(false),
                    Err(e) if e.interrupted() => return Err(e),
                    Err(_) => return Ok(false),
                }
                match self.text_of(doc, &p) {
                    Ok(got) => Ok(got.trim_end() == want.trim_end()),
                    Err(e) if e.interrupted() => Err(e),
                    Err(_) => Ok(false),
                }
            }
            Tag::Bytes(want) => {
                let mut p = elem.to_vec();
                let Some((last, above)) = key.split_last() else { return Ok(false) };
                match self.descend(doc, &mut p, above) {
                    Ok(true) => {}
                    Ok(false) => return Ok(false),
                    Err(e) if e.interrupted() => return Err(e),
                    Err(_) => return Ok(false),
                }
                match self.child_raw_bytes(doc, &p, last) {
                    Ok(got) => Ok(got == *want),
                    Err(e) if e.interrupted() => Err(e),
                    Err(_) => Ok(false),
                }
            }
        }
    }

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
                    let info = self.node(doc, &p)?;
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
