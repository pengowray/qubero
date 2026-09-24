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
        // An element written as the name of a type is placed as the type the
        // name stands for, so it takes the same room that type would written
        // out in place.
        let named;
        let elem: &Ty = match &**elem {
            Ty::Named(_) => match self.through_names(elem) {
                Some(ty) => {
                    named = ty;
                    &named
                }
                None => return Ok(None),
            },
            elem => elem,
        };
        if let Some(f) = fixed_bits(elem) {
            // A fixed size can still be nought: a struct of computed fields,
            // or a run of no bytes. Counting a repeat by dividing by that is
            // no count, so it keeps the walk, which refuses the element.
            if f == 0 && !matches!(ty, Ty::Array { .. }) {
                return Ok(None);
            }
            return Ok(Some(f));
        }
        // A run of numbers packed to whatever width the header named. The
        // width is asked once, of the list, and every element is that wide, so
        // a grid of a million values is placed by arithmetic rather than by a
        // million reads. Only when the width asks nothing about the element,
        // which is the same test a window's size passes.
        if let Ty::UIntExpr { bits, .. } = elem.without_sentinel() {
            if !uniform(bits) {
                return Ok(None);
            }
            let n = self.eval_expr(doc, path, &bits.clone())?;
            if !(0..=128).contains(&n) {
                return Ok(None);
            }
            // A width of zero is a real answer for an array, whose count says
            // how many there are: a GRIB whose values are all the same writes
            // no data at all, and a million of them must not be a million
            // reads. A repeat has no count and would never end, so it keeps
            // the walk, which refuses a zero-size element.
            if n == 0 && !matches!(ty, Ty::Array { .. }) {
                return Ok(None);
            }
            return Ok(Some(n as u64));
        }
        // The same run with something worked out beside each number: a GRIB
        // value is the packed integer and what it is worth, and the worth
        // takes no bits. A record whose fields are all fixed or all as wide
        // as a field outside it says is as wide as they add up to, and a grid
        // of a million of them is still placed by arithmetic.
        //
        // Only a width that is a number or a name, and a name that is not one
        // of the record's own fields: asked of the list, a name finds the
        // field around the list, and a width the record says of itself would
        // be answered by the wrong field or by none.
        if let Ty::Struct(s) = elem {
            let mut total = 0u64;
            // A union is as wide as its widest field rather than as their
            // sum, since every one of them starts where the record does. See
            // `StructDef::overlap`.
            let add = |total: &mut u64, bits: u64| match s.overlap {
                true => *total = (*total).max(bits),
                false => *total += bits,
            };
            for f in &s.fields {
                if let Some(bits) = fixed_bits(&f.ty) {
                    add(&mut total, bits);
                    continue;
                }
                let Ty::UIntExpr { bits, .. } = f.ty.without_sentinel() else { return Ok(None) };
                match &**bits {
                    Expr::Lit(_) => {}
                    Expr::Ref(name) if !s.fields.iter().any(|g| *g.name == **name) => {}
                    _ => return Ok(None),
                }
                let n = self.eval_expr(doc, path, &bits.clone())?;
                if !(0..=128).contains(&n) {
                    return Ok(None);
                }
                add(&mut total, n as u64);
            }
            // Nothing at all, which an array can count and a repeat cannot,
            // for the reason a bare width of nought is kept to an array above.
            if total == 0 && !matches!(ty, Ty::Array { .. }) {
                return Ok(None);
            }
            return Ok(Some(total));
        }
        let Ty::Sized { size, .. } = elem else { return Ok(None) };
        if !uniform(size) {
            return Ok(None);
        }
        // The size is asked of the list rather than of an element, which is
        // the same question: it names a field of an enclosing struct, and an
        // element's own fields are not in scope for it. A size too large to
        // count in bits is no stride: the walk stops at the first element,
        // which says why.
        let n = self.eval_expr(doc, path, size)?;
        Ok(if n > 0 { bits_in(n) } else { None })
    }

    /// How far the furthest of a union's `n` fields reaches past `from`, which
    /// is how long the union is. Every field has to be measured rather than
    /// only the last: which of them is the widest is a fact about the file
    /// whenever any of them is sized by what it reads. See
    /// [`crate::template::StructDef::overlap`].
    ///
    /// Apart from `size_within` for the sake of the stack, which it sits at
    /// the bottom of: what measuring a union takes has no business in the
    /// frame of every node that is not one.
    #[inline(never)]
    fn longest_field<S: Source>(&mut self, doc: &Document<S>, path: &[usize], n: usize, from: u64) -> R<u64> {
        let mut longest = 0;
        let mut f = path.to_vec();
        for i in 0..n {
            f.push(i);
            self.resolve(doc, &f)?;
            longest = longest.max(self.memo[&f].offset + self.size_of(doc, &f)? - from);
            f.pop();
        }
        Ok(longest)
    }

    /// The type `ty` stands for once every name in front of it is looked up,
    /// or `ty` itself when it is not a name. `None` when a name has no type in
    /// this template, or when the names go on longer than resolving a field
    /// follows them, which is a name that comes back to itself.
    pub(super) fn through_names(&self, ty: &Ty) -> Option<Ty> {
        let mut ty = ty;
        for _ in 0..=64 {
            let Ty::Named(n) = ty else { return Some(ty.clone()) };
            ty = self.template.types.get(&**n)?;
        }
        None
    }

    pub(super) fn size_of<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<u64> {
        self.deeper(path.len(), |ev| ev.size_within(doc, path))
    }

    /// How much room the node at `path` takes. Apart from `size_of` only so
    /// that the count of open reads there is kept by one pair of statements
    /// with nothing between them that can return.
    fn size_within<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<u64> {
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
                // A union is as long as its longest field, so every one of
                // them has to be measured rather than only the last. There is
                // no other way round it: which field is the widest is a fact
                // about the file whenever any of them is sized by what it
                // reads. See `StructDef::overlap`.
                Ty::Struct(s) if s.overlap => self.longest_field(doc, path, s.fields.len(), r.offset)?,
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
                Ty::Array { .. } | Ty::Repeat { .. } => {
                    // Same-sized elements: the whole list is count × stride,
                    // with no element resolved. An array of a billion samples,
                    // or a database of a million pages, is sized by arithmetic.
                    // A run that stops on what it reads cannot be, since the
                    // element that ends it could be anywhere.
                    let stride = match &r.ty {
                        Ty::Array { .. } | Ty::Repeat { until: Until::End, .. } => self.stride(doc, path, &r.ty)?,
                        _ => None,
                    };
                    if let Some(stride) = stride {
                        match self.child_count(doc, path)?.checked_mul(stride) {
                            Some(bits) => bits,
                            None => return fail("field extends beyond its parent"),
                        }
                    } else {
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
                }
                // Everything else is as long as its own bytes say, and finding
                // that out opens nothing below this node. Kept to a call of its
                // own so that what the reading takes is off the stack while
                // whatever this node holds is being measured.
                _ => self.read_size(doc, path, &r)?,
            }
        };
        if r.offset.checked_add(size).is_none_or(|end| end > r.limit) {
            return fail("field extends beyond its parent");
        }
        self.memo.get_mut(path).expect("resolved").size = Some(size);
        Ok(size)
    }

    /// How long a node is when its length is written in its own bytes: a
    /// length field, a terminator, a variable-length integer, an instruction.
    /// Never a structure or a list, which are as long as what they hold.
    fn read_size<S: Source>(&mut self, doc: &Document<S>, path: &[usize], r: &Resolved) -> R<u64> {
        Ok(match &r.ty {
                Ty::Bytes(e) => {
                    let n = self.eval_expr(doc, path, e)?;
                    if n < 0 {
                        return fail("negative length");
                    }
                    match bits_in(n) {
                        Some(bits) => bits,
                        None => return fail("field extends beyond its parent"),
                    }
                }
                Ty::Str { len, .. } | Ty::TextInt { len, .. } => match len {
                    StrLen::Fixed(e) | StrLen::Padded { size: e, .. } => {
                        let n = self.eval_expr(doc, path, e)?;
                        if n < 0 {
                            return fail("negative length");
                        }
                        match bits_in(n) {
                            Some(bits) => bits,
                            None => return fail("field extends beyond its parent"),
                        }
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
                // Every row of every pass, by arithmetic, and no pixel placed
                // to find out. Asked here rather than beside the lists, whose
                // frame stays open the whole way down a nested file and has no
                // room to spare. See [`Ty::Raster`].
                Ty::Raster { .. } => self.raster_bits(doc, path)?,
                // As wide as the decoder said it read. Not measured by adding
                // up children: a symbol run of a million is one subtraction.
                Ty::Traced { part } => {
                    let part = *part;
                    let Some((base, trace)) = self.trace_for(path) else {
                        return fail("this stream is no longer open");
                    };
                    let span = match part {
                        crate::template::TracedPart::Blocks => match (trace.blocks().first(), trace.blocks().last()) {
                            (Some(a), Some(b)) => b.in_bits.end - a.in_bits.start,
                            _ => 0,
                        },
                        crate::template::TracedPart::Block(i) => match trace.blocks().get(i as usize) {
                            Some(b) => b.in_bits.end - b.in_bits.start,
                            None => 0,
                        },
                        crate::template::TracedPart::Symbols(i) => {
                            match super::traced::BlockView::of(trace, i) {
                                Some(v) => v.block.in_bits.end - v.symbols_at(trace),
                                None => 0,
                            }
                        }
                    };
                    let _ = base;
                    span
                }
                Ty::Leb128 { .. } | Ty::Zigzag => {
                    let (_, n) = self.read_leb(doc, &r)?;
                    n * 8
                }
                Ty::Vlq => {
                    let (_, n) = self.read_vlq(doc, &r)?;
                    n * 8
                }
                // As long as the decoder says the instruction is.
                Ty::Insn { isa } => self.read_insn(doc, &r, *isa)?.len as u64 * 8,
                Ty::EbmlVint { strip_marker } => {
                    let (_, n) = self.read_ebml_vint(doc, &r, *strip_marker)?;
                    n * 8
                }
                // A wrapper that names values, names bits, or names one value
                // as unset is as long as what it wraps. Asked of the inner
                // type rather than listed case by case, so that a wrapper over
                // a width read from the file works the same as one over a
                // variable-length integer.
                Ty::Enum { inner, .. } | Ty::Flags { inner, .. } | Ty::Nullable { inner, .. } => {
                    let inner = (**inner).clone();
                    match fixed_bits(&inner) {
                        Some(f) => f,
                        None => {
                            let mut ir = r.clone();
                            ir.ty = inner;
                            self.read_size(doc, path, &ir)?
                        }
                    }
                }
                // As many bits as an earlier field says. `Little` is refused
                // here rather than placed wrongly: a field packed from the
                // bottom of its byte has to be placed before it is read, and
                // where it goes depends on a width only reading gives.
                Ty::UIntExpr { bits, endian } => {
                    let (bits, endian) = (bits.clone(), *endian);
                    let n = self.eval_expr(doc, path, &bits)?;
                    if !(0..=128).contains(&n) {
                        return fail(format!("a number {n} bits wide is not one this can read"));
                    }
                    if endian == crate::template::Endian::Little && (n % 8 != 0 || r.offset % 8 != 0) {
                        return fail("a number packed low-bit-first has to have a width the template knows");
                    }
                    n as u64
                }
                // A JSON field is as long as the text it was given: what the
                // values inside it come to is what the parse says, and any
                // room left over is padding the format put there.
                Ty::Json(..) => r.limit - r.offset,
                // The whole file: a Familiar Pickle Form matches all of it or
                // matches nothing. Every node under it was given its size
                // when the recognised tree placed it.
                Ty::Pickle(..) => r.limit - r.offset,
                // A pointer list holds the stretch its offsets point into,
                // which runs to the end of its container.
                Ty::PointerList { .. } => r.limit - r.offset,
                Ty::SqliteVarint => self.read_sqlite_varint(doc, &r)?.1 * 8,
                Ty::SevenZipNumber => self.read_sevenzip_number(doc, &r)?.1 * 8,
                // Reached only through a wrapper over something that is
                // neither a number nor a run of bytes, which is a template
                // saying something it cannot mean. An error rather than a
                // panic: one field of one format should not take the reader
                // down with it.
                other => return fail(format!("{} has no length of its own", other.display_name())),
        })
    }

    /// How many children the node at `path` has, when answering does not mean
    /// walking the whole of it.
    ///
    /// A run that fills its container with elements whose length their own
    /// bytes give is the one case where it does: a code section is a repeat of
    /// instructions until the end, and counting them means decoding every one,
    /// which for a 66 MiB section is a minute of work for a number nothing on
    /// screen shows. None says so, and the caller finds what it wanted another
    /// way. Every other list either divides, or ends on something it reads and
    /// so stops of its own accord.
    pub(super) fn count_unless_walk<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Option<u64>> {
        self.resolve(doc, path)?;
        let ty = self.memo[path].ty.clone();
        if matches!(ty, Ty::Repeat { until: Until::End, .. }) && self.stride(doc, path, &ty)?.is_none() {
            return Ok(None);
        }
        self.child_count(doc, path).map(Some)
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
                let Ok(n) = u64::try_from(n) else { return fail(format!("count {n} does not fit in a u64")) };
                self.list_mut(path).expected_count = Some(n);
                Ok(n)
            }
            // As long as the chain turns out to be, which is only knowable by
            // following it to the end. See [`Ty::Chain`].
            Ty::Chain { .. } => Ok(self.chain_starts(doc, path)?.len() as u64),
            // The same for a gather: as many as the walk to its records finds.
            // See [`Ty::Gather`].
            Ty::Gather { .. } => self.gather_count(doc, path),
            // One a pixel, by arithmetic. See [`Ty::Raster`].
            Ty::Raster { .. } => self.raster_count(doc, path),
            // As many children as the array of offsets has entries.
            Ty::PointerList { offsets, .. } => {
                let n = self.eval_expr(doc, path, &Expr::Ref(offsets.clone()))?;
                if n < 0 {
                    return fail("negative count");
                }
                let Ok(n) = u64::try_from(n) else { return fail(format!("count {n} does not fit in a u64")) };
                Ok(n)
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
            // Asking what is inside a stream is what opens it. One child when
            // it opened, none when it would not: a refusal is a leaf, and the
            // node carries the reason.
            // Asking what is inside a stream is what opens it. Nothing when it
            // would not open: a refusal is a leaf, and the node carries the
            // reason. What came out of it, and, when the decoder kept a trace
            // with blocks in it, what it read to get there.
            Ty::Decoded { .. } => Ok(match self.open_space_at(doc, path)? {
                super::space::Opened::Space(_) if self.has_blocks(path) => 2,
                super::space::Opened::Space(_) => 1,
                super::space::Opened::Refused(_) => 0,
            }),
            // One child when the walk to its parts made a space, none when
            // it would not. See [`Ty::Stitched`].
            Ty::Stitched { .. } => Ok(match self.open_stitched_at(doc, path)? {
                super::space::Opened::Space(_) => 1,
                super::space::Opened::Refused(_) => 0,
            }),
            Ty::Traced { part } => {
                let part = *part;
                let Some((_, trace)) = self.trace_for(path) else { return Ok(0) };
                Ok(match part {
                    crate::template::TracedPart::Blocks => trace.blocks().len() as u64,
                    crate::template::TracedPart::Block(i) => match super::traced::BlockView::of(trace, i) {
                        Some(v) => v.head.len() as u64 + u64::from(!v.symbols.is_empty()),
                        None => 0,
                    },
                    crate::template::TracedPart::Symbols(i) => match super::traced::BlockView::of(trace, i) {
                        Some(v) => v.symbols.len() as u64,
                        None => 0,
                    },
                })
            }
            Ty::Json(shape, _) if shape.composite() => self.json_child_count(doc, path),
            Ty::Pickle(..) => self.pickle_child_count(doc, path),
            _ => Ok(0),
        }
    }
}

/// Whether every element of a run of `ty` has the same fields, at the same
/// places and of the same types, whatever its bytes say, so that walking the
/// first says what walking all of them would. The kind totals multiply element
/// 0's breakdown by this and the Diagram view's count multiplies element 0's
/// boxes and rows, so it is one rule for both.
///
/// A same stride is not that. A run of 4 KiB pages is all the same size and no
/// two pages hold the same fields: a window of a fixed size is the same shape
/// only when what is inside it is. Nor is a fixed number of bits quite that,
/// because of the fields it counts as no bits. A field pointing somewhere else
/// is none here, and each element points somewhere different, at something of
/// a different length: a minidump's directory is a run of twelve-byte entries,
/// each pointing at a stream of its own. And a window of a fixed size can hold
/// a type chosen when it is read, which is how an Arrow record batch keeps its
/// nodes sixteen bytes while each says in a field of no bytes which column it
/// is, or that it is none. Multiplying element 0 would count the first
/// element's stream once for every entry, and its choice for every node. So
/// those two are left out, and a name is looked through to what it stands for.
///
/// Checked against walking every element over the sample collection: for the
/// kind totals by `KindWalk`, and for the count by `examples/census_exact.rs`.
pub(super) fn same_shape(template: &Template, ty: &Ty) -> bool {
    same_shape_within(template, ty, 0)
}

/// [`same_shape`], `hops` names deep into the template.
fn same_shape_within(template: &Template, ty: &Ty, hops: usize) -> bool {
    match ty {
        Ty::Struct(s) => s.fields.iter().all(|f| same_shape_within(template, &f.ty, hops)),
        Ty::Array { elem, count: Expr::Lit(_) } => same_shape_within(template, elem, hops),
        Ty::Sized { size: Expr::Lit(_), inner } => same_shape_within(template, inner, hops),
        Ty::Enum { inner, .. } | Ty::Flags { inner, .. } | Ty::Nullable { inner, .. } => same_shape_within(template, inner, hops),
        Ty::Named(n) => hops < 64 && template.types.get(&**n).is_some_and(|t| same_shape_within(template, t, hops + 1)),
        Ty::At { .. } | Ty::Chain { .. } | Ty::Gather { .. } | Ty::Stitched { .. } => false,
        other => fixed_bits(other).is_some(),
    }
}

/// A length in bytes as bits, when bits can count it. The length is whatever a
/// field of the file said, and a corrupt one can say more than eight times it
/// fits in a u64: that is a length no container holds, not one to wrap round
/// to something small.
pub(super) fn bits_in(bytes: i128) -> Option<u64> {
    u64::try_from(bytes).ok()?.checked_mul(8)
}

/// Whether an expression asks nothing about the element it sits in, so that
/// every element of a list gets the same answer. A page size named in a file's
/// header is the same for every page; a length read from the element itself,
/// or one counted from where the element starts, is not.
pub(super) fn uniform(e: &Expr) -> bool {
    match e {
        Expr::Lit(_) | Expr::Ref(_) => true,
        Expr::Add(a, b)
        | Expr::Sub(a, b)
        | Expr::Mul(a, b)
        | Expr::Div(a, b)
        | Expr::DivCeil(a, b)
        | Expr::Or(a, b)
        | Expr::Less(a, b)
        | Expr::Shl(a, b)
        | Expr::Shr(a, b)
        | Expr::And(a, b)
        | Expr::BitOr(a, b)
        | Expr::BitXor(a, b)
        | Expr::Min(a, b)
        | Expr::Max(a, b) => {
            uniform(a) && uniform(b)
        }
        Expr::Log2(a) | Expr::BitNot(a) => uniform(a),
        // How big the whole space is, which is the same number wherever in it
        // the asking is done. `SpacePos` is not: it is where this element
        // starts, and no two elements start in the same place.
        Expr::SpaceSize => true,
        // A real reads nothing, and the three that take one apart ask what
        // their operand asks.
        Expr::Real(_) => true,
        Expr::Pow2(a) | Expr::Pow10(a) | Expr::Trunc(a) => uniform(a),
        // Padding asks nothing the run it follows did not already ask.
        Expr::PadTo { n, .. } => uniform(n),
        // Remaining and Idx count from the element; the peeks read it; Prev,
        // Sibling and Elem ask another one; SizeOf asks a field beside it.
        _ => false,
    }
}

/// The error for a run whose element takes no room. Out of line for the
/// reason `read::not_text` is: the walk's frame is on the path the deepest
/// nesting is measured against.
#[cold]
#[inline(never)]
pub(super) fn zero_size_element<T>(r: &Resolved) -> R<T> {
    fail(format!("{} repeats an element of zero size", r.name.text()))
}
