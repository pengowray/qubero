//! How much room a node takes, and how many children it has.
//!
//! One pass rather than two. A structure is as long as its last field ends, an
//! array is its count times its stride when the elements agree and a walk when
//! they do not, and a text field is as long as the scan for its terminator
//! says: the same question every time, asked of a different type. Both answers
//! are settled here and remembered on the node, so a list opened twice is
//! measured once.
//!
//! What makes it worth keeping apart from resolving is the arithmetic. A run
//! of same-sized elements is counted by division rather than by walking it,
//! which is the difference between opening a database of a million pages and
//! reading one, and `stride` is what decides whether that shortcut is honest.

use super::*;

impl Evaluator {
    /// The distance from one element of the list at `path` to the next, when
    /// every element takes the same room. A file of same-sized pages says how
    /// big a page is once, in its header, and then never again: a database is
    /// a run of 4 KiB pages, a disc image a run of 2 KiB sectors. Knowing the
    /// stride turns "which page is byte 900,000,000 in" from a walk through
    /// two hundred thousand pages into a division.
    ///
    /// `None` when the elements can differ, which is when the walk is the only
    /// way to find out.
    pub(super) fn stride<S: Source>(&mut self, doc: &Document<S>, path: &[usize], ty: &Ty) -> R<Option<u64>> {
        let elem = match ty {
            Ty::Array { elem, .. } => elem,
            // A run that stops on what it reads cannot be counted by division:
            // the element that ends it could be anywhere.
            Ty::Repeat { elem, until: Until::End } => elem,
            _ => return Ok(None),
        };
        if let Some(f) = fixed_bits(elem) {
            return Ok(Some(f));
        }
        let Ty::Sized { size, .. } = &**elem else { return Ok(None) };
        if !uniform(size) {
            return Ok(None);
        }
        // The size is asked of the list rather than of an element, which is
        // the same question: it names a field of an enclosing struct, and an
        // element's own fields are not in scope for it.
        let n = self.eval_expr(doc, path, size)?;
        Ok(if n > 0 { Some(n as u64 * 8) } else { None })
    }

    pub(super) fn size_of<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<u64> {
        self.resolve(doc, path)?;
        let r = self.memo[path].clone();
        if let Some(s) = r.size {
            return Ok(s);
        }
        let size = if let Some(d) = r.declared_size {
            d
        } else if let Some(f) = fixed_bits(&r.ty) {
            f
        } else {
            match &r.ty {
                Ty::Bytes(e) => {
                    let n = self.eval_expr(doc, path, e)?;
                    if n < 0 {
                        return fail("negative length");
                    }
                    n as u64 * 8
                }
                Ty::Str { len, .. } | Ty::TextInt { len, .. } => match len {
                    StrLen::Fixed(e) | StrLen::Padded { size: e, .. } => {
                        let n = self.eval_expr(doc, path, e)?;
                        if n < 0 {
                            return fail("negative length");
                        }
                        n as u64 * 8
                    }
                    // Whitespace, then the value, then the byte that ends it.
                    StrLen::Scan { skip, ends, comment } => self.read_scan(doc, &r, skip, ends, *comment)?.1 * 8,
                    StrLen::Terminated { end, or_end } => {
                        // Digits are ASCII, and ASCII is a byte a character,
                        // so the terminator is one byte either way.
                        let enc = match &r.ty {
                            Ty::Str { enc, .. } => enc.clone(),
                            _ => Encoding::Ascii,
                        };
                        let (settled, bom) = self.str_head(doc, &r, &enc)?;
                        let term = text::unit_bytes(settled, *end);
                        match self.read_terminated(doc, &r, &term, bom) {
                            Ok((_, n)) => n * 8,
                            // No terminator: the field runs to the end of its
                            // container, if the format allows for that.
                            Err(e) => {
                                if *or_end && !e.interrupted() {
                                    r.limit - r.offset
                                } else {
                                    return Err(e);
                                }
                            }
                        }
                    }
                },
                Ty::Leb128 { .. } => {
                    let (_, n) = self.read_leb(doc, &r)?;
                    n * 8
                }
                Ty::Vlq => {
                    let (_, n) = self.read_vlq(doc, &r)?;
                    n * 8
                }
                Ty::EbmlVint { strip_marker } => {
                    let (_, n) = self.read_ebml_vint(doc, &r, *strip_marker)?;
                    n * 8
                }
                Ty::Enum { inner, .. } | Ty::Flags { inner, .. } => match **inner {
                    Ty::Leb128 { .. } => {
                        let (_, n) = self.read_leb(doc, &r)?;
                        n * 8
                    }
                    Ty::Vlq => {
                        let (_, n) = self.read_vlq(doc, &r)?;
                        n * 8
                    }
                    Ty::EbmlVint { strip_marker } => {
                        let (_, n) = self.read_ebml_vint(doc, &r, strip_marker)?;
                        n * 8
                    }
                    Ty::SqliteVarint => self.read_sqlite_varint(doc, &r)?.1 * 8,
                    _ => return fail("enum over a type with no fixed size"),
                },
                // A JSON field is as long as the text it was given: what the
                // values inside it come to is what the parse says, and any
                // room left over is padding the format put there.
                Ty::Json(_) => r.limit - r.offset,
                // A pointer list holds the stretch its offsets point into,
                // which runs to the end of its container.
                Ty::PointerList { .. } => r.limit - r.offset,
                Ty::SqliteVarint => self.read_sqlite_varint(doc, &r)?.1 * 8,
                Ty::Struct(s) => {
                    if s.fields.is_empty() {
                        0
                    } else {
                        let mut last = path.to_vec();
                        last.push(s.fields.len() - 1);
                        self.resolve_upto(doc, path, s.fields.len() - 1)?;
                        self.resolve(doc, &last)?;
                        let end = self.memo[&last].offset + self.size_of(doc, &last)?;
                        end - r.offset
                    }
                }
                // Same-sized elements: the whole list is count × stride,
                // with no element resolved. An array of a billion samples, or
                // a database of a million pages, is sized by arithmetic.
                Ty::Array { .. } | Ty::Repeat { until: Until::End, .. }
                    if self.stride(doc, path, &r.ty)?.is_some() =>
                {
                    let stride = self.stride(doc, path, &r.ty)?.expect("checked");
                    self.child_count(doc, path)? * stride
                }
                Ty::Array { .. } | Ty::Repeat { .. } => {
                    let n = self.child_count(doc, path)?;
                    if n == 0 {
                        0
                    } else {
                        let mut last = path.to_vec();
                        last.push(n as usize - 1);
                        self.resolve(doc, &last)?;
                        let end = self.memo[&last].offset + self.size_of(doc, &last)?;
                        end - r.offset
                    }
                }
                _ => unreachable!("fixed-size types handled above"),
            }
        };
        if r.offset + size > r.limit {
            return fail("runs past the end of its container");
        }
        self.memo.get_mut(path).expect("resolved").size = Some(size);
        Ok(size)
    }

    pub(super) fn child_count<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<u64> {
        self.resolve(doc, path)?;
        let r = self.memo[path].clone();
        match &r.ty {
            Ty::Struct(s) => Ok(s.fields.len() as u64),
            // What it points at, and nothing else.
            Ty::At { .. } => Ok(1),
            Ty::Array { count, .. } => {
                let n = self.eval_expr(doc, path, count)?;
                if n < 0 {
                    return fail("negative count");
                }
                Ok(n as u64)
            }
            // As many children as the array of offsets has entries.
            Ty::PointerList { offsets, .. } => {
                let n = self.eval_expr(doc, path, &Expr::Ref(offsets.clone()))?;
                if n < 0 {
                    return fail("negative count");
                }
                Ok(n as u64)
            }
            // A run of same-sized elements filling its container is as
            // long as the room divides. Anything left over at the end is less
            // than one element and belongs to no element, so it reads as a gap
            // rather than taking the whole run down with it.
            Ty::Repeat { until: Until::End, .. } if self.stride(doc, path, &r.ty)?.is_some() => {
                let stride = self.stride(doc, path, &r.ty)?.expect("checked");
                Ok((r.limit - r.offset) / stride)
            }
            // Counting a run means walking it, and a run of a million things
            // walked without forgetting any of them is a million nodes. The
            // walk keeps a window and its checkpoints instead; see `walk.rs`.
            Ty::Repeat { until, .. } => {
                let until = until.clone();
                self.count_repeat(doc, path, &r, &until)
            }
            Ty::Json(shape) if shape.composite() => self.json_child_count(doc, path),
            _ => Ok(0),
        }
    }
}

/// Whether an expression asks nothing about the element it sits in, so that
/// every element of a list gets the same answer. A page size named in a file's
/// header is the same for every page; a length read from the element itself,
/// or one counted from where the element starts, is not.
pub(super) fn uniform(e: &Expr) -> bool {
    match e {
        Expr::Lit(_) | Expr::Ref(_) => true,
        Expr::Add(a, b) | Expr::Sub(a, b) | Expr::Mul(a, b) | Expr::Div(a, b) | Expr::Or(a, b) | Expr::Less(a, b) => {
            uniform(a) && uniform(b)
        }
        // Remaining and Idx count from the element; the peeks read it; Prev,
        // Sibling and Elem ask another one; SizeOf asks a field beside it.
        _ => false,
    }
}
