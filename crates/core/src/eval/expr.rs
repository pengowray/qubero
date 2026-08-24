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
                let n = self.child_count(doc, &p)?;
                let mut total: i128 = 1;
                let mut child = p.clone();
                for i in 0..n as usize {
                    child.push(i);
                    let v = self.node(doc, &child)?.value.as_int();
                    child.pop();
                    let Some(v) = v else { return fail(format!("{array} holds no number there")) };
                    let Some(next) = total.checked_mul(v) else { return fail("shape too large to count") };
                    total = next;
                }
                // Nothing to multiply describes nothing, not one of something.
                if n == 0 { 0 } else { total }
            }
            Expr::Ref(name) => match self.lookup(doc, at, name)? {
                (Some(v), _) => v,
                (None, _) => return fail(format!("{name} is not a number")),
            },
            Expr::SizeOf(name) => self.lookup(doc, at, name)?.1,
            // Read where this field starts without taking the bits: what a
            // field that exists only when the byte says so has to ask.
            Expr::Peek(bits) => {
                let Some((offset, limit)) = here else { return fail("nothing to look at") };
                if offset + u64::from(*bits) > limit {
                    return fail("looks past the end of its container");
                }
                let mut buf = vec![0u8; bytes_for(u64::from(*bits))];
                let missing = doc.read_bits(offset, u64::from(*bits), &mut buf);
                if !missing.is_empty() {
                    return Err(EvalError::Pending(missing));
                }
                read_uint(&buf, *bits, crate::template::Endian::Big) as i128
            }
            Expr::Prev(name) => self.prev_field(doc, at, name)?,
            Expr::Sibling(field) => self.sibling_field(doc, at, &field.clone())?,
            Expr::Or(a, b) => match self.eval_expr_at(doc, at, a, here)? {
                0 => self.eval_expr_at(doc, at, b, here)?,
                v => v,
            },
            Expr::Add(a, b) => self.eval_expr_at(doc, at, a, here)? + self.eval_expr_at(doc, at, b, here)?,
            Expr::Sub(a, b) => self.eval_expr_at(doc, at, a, here)? - self.eval_expr_at(doc, at, b, here)?,
            Expr::Mul(a, b) => self.eval_expr_at(doc, at, a, here)? * self.eval_expr_at(doc, at, b, here)?,
            Expr::Div(a, b) => {
                let d = self.eval_expr_at(doc, at, b, here)?;
                if d == 0 {
                    return fail("division by zero");
                }
                self.eval_expr_at(doc, at, a, here)? / d
            }
        })
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

    /// Follow `field` down from the node at `path`, through whatever the
    /// template resolved it to, and read the number at the end. None when this
    /// node has no such field, which is how a search over siblings passes over
    /// the ones that are something else.
    fn field_in<S: Source>(&mut self, doc: &Document<S>, path: &mut Vec<usize>, field: &[String]) -> R<Option<i128>> {
        for name in field {
            if self.resolve(doc, path).is_err() {
                return Ok(None);
            }
            let Ty::Struct(s) = self.memo[path.as_slice()].ty.base() else { return Ok(None) };
            let Some(j) = s.fields.iter().position(|f| *f.name == **name) else { return Ok(None) };
            path.push(j);
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
    fn elem_path<S: Source>(
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
        for name in field {
            self.resolve(doc, &p)?;
            let Ty::Struct(s) = self.memo[&p].ty.base() else {
                return fail(format!("{array}[{i}] has no fields to look in"));
            };
            let Some(j) = s.fields.iter().position(|f| *f.name == **name) else {
                return fail(format!("{array}[{i}] has no field named {name}"));
            };
            p.push(j);
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
                    let mut p = parent;
                    p.push(j);
                    let info = self.node(doc, &p)?;
                    // A field with no numeric reading can still be measured.
                    return Ok((info.value.as_int(), (info.size_bits / 8) as i128));
                }
            }
        }
        fail(format!("unknown field {name}"))
    }
}
