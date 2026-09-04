//! What the linear views read: the name a row is given, the field under a
//! bit, and the run of spans that covers a stretch of the file.

use super::*;

/// One entry in the annotation column: a field, a run of them, or a stretch
/// the template does not describe. Produced by `Evaluator::spans`.
#[derive(Debug, Clone)]
pub struct Span {
    pub path: Vec<usize>,
    pub offset_bits: u64,
    pub size_bits: u64,
    pub name: String,
    /// What it sits inside, outermost first.
    pub trail: Vec<String>,
    pub type_name: String,
    pub value: Value,
    /// No field covers these bits.
    pub gap: bool,
    /// How many fields this entry stands for, when a large run of values or
    /// records is shown as one. Zero for a single field.
    pub count: u64,
    /// A structure marked to read on one row, already joined: `local.get 0`
    /// rather than an `op` row and an `imm` row. None for everything else,
    /// which reads as its own value.
    pub line: Option<String>,
    /// The first few values of a run shown as one entry. `512 values` says how
    /// many and nothing about what, and a run of zeroes and a run of samples
    /// are worth telling apart without opening either.
    pub sample: Vec<String>,
    /// The first few element extents of a collapsed run. These let a compact
    /// view show the run's on-disk rhythm without spelling it as `8|12|...`.
    pub parts: Vec<SpanPart>,
    /// How the field's bits divide into framing and value, for the numbers
    /// whose bytes do not read as bytes. None for everything else, and for a
    /// varint whose bytes have not been read yet: the split is worth drawing
    /// when it is known and worth nothing guessed.
    pub bits: Option<crate::varintbits::BitRoles>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpanPart {
    pub size_bits: u64,
    pub label: String,
    /// The uninspected remainder of the run rather than one element.
    pub rest: bool,
}

/// Values from the front of a collapsed run, at most this many.
const SAMPLE: u64 = 4;

/// One value on a shared row, which is terser than the same value on a row of
/// its own: a named number gives its name and drops the number behind it,
/// because the row already has several values competing for the eye.
pub(super) fn brief(v: &Value) -> String {
    match v {
        Value::UInt(n) => n.to_string(),
        Value::Int(n) => n.to_string(),
        Value::Float(n) => format!("{n}"),
        Value::Str(s) => s.clone(),
        Value::Enum { raw, name, hex } => match name {
            Some(n) => n.clone(),
            None if *hex => format!("0x{raw:x}"),
            None => raw.to_string(),
        },
        Value::Flags { set, unnamed, .. } => {
            let mut s = set.join("|");
            if *unnamed > 0 {
                if !s.is_empty() {
                    s.push('|');
                }
                let _ = std::fmt::Write::write_fmt(&mut s, format_args!("{unnamed} more"));
            }
            s
        }
        Value::Bytes { len, preview } => {
            let s: String = preview.iter().map(|b| format!("{b:02x} ")).collect();
            let s = s.trim_end().to_string();
            if *len as usize > preview.len() { format!("{s}…") } else { s }
        }
        // The bytes are on their way; the row says where and how long.
        Value::Unread { .. } => "\u{2026}".to_string(),
        // A slot nobody filled in. The number underneath is in the file and in
        // the hex view; saying -12345 here would read as a measurement.
        Value::Unset(_) => "unset".to_string(),
        Value::Magic { ok, bytes, expected } => magic_reading(*ok, bytes, expected),
        Value::Composite { .. } => String::new(),
    }
}

/// How a signature reads in one line.
///
/// The bytes as C would write a string, so a reader sees the name in them and
/// the bytes that are not a name at once. A file that has what the format
/// asked for needs nothing further said about it. One that does not is told
/// what was wanted as well as what is there, since a signature that is wrong
/// is only worth reading beside the one it should have been; before this the
/// line said the bytes did not match without saying what they did not match.
///
/// One caveat, and it is not this function's: `text::c_string` puts two bases
/// in one line, so Matroska's reads `"\032E\xdf\xa3"` where `\032` is the byte
/// the gutter calls `0x1a`. See the gap of its own about that.
pub fn magic_reading(ok: bool, bytes: &[u8], expected: &[u8]) -> String {
    let text = crate::text::c_string(bytes);
    if ok { text } else { format!("{text} does not match {}", crate::text::c_string(expected)) }
}

/// A run of these is worth one entry rather than one each.
const COLLAPSE_RUN: u64 = 8;
/// Records carry more meaning than scalar samples, so short lists stay open.
/// Beyond this point the list's section row and a few record names are a more
/// useful first view than scores of repeated internal fields.
const COLLAPSE_COMPLEX_RUN: u64 = 32;
/// A run of records folds only when the records are small. A model's tensor
/// table is hundreds of records a few dozen bytes each, and a row per field
/// would fill the column with nothing but its insides; a frame file's hundred
/// and sixty structures are kilobytes each, and one chip standing for the
/// whole megabyte says nothing about any of them.
const COLLAPSE_ELEMENT_BYTES: u64 = 256;

/// A type that holds one number or one run of bytes, and nothing inside it.
pub(super) fn plain(ty: &Ty) -> bool {
    match ty {
        Ty::Enum { inner, .. } | Ty::Flags { inner, .. } | Ty::Nullable { inner, .. } => plain(inner),
        Ty::UInt { .. }
        | Ty::Int { .. }
        | Ty::SignMagnitude { .. }
        | Ty::UIntExpr { .. }
        | Ty::F16(_)
        | Ty::BF16(_)
        | Ty::F32(_)
        | Ty::F64(_)
        | Ty::Fixed { .. }
        | Ty::Leb128 { .. }
        | Ty::EbmlVint { .. }
        | Ty::Vlq
        | Ty::SqliteVarint
        | Ty::F8 { .. }
        | Ty::Magic(_)
        | Ty::TextInt { .. }
        | Ty::Bytes(_)
        // An instruction is one thing, however many bytes it took to write.
        | Ty::Insn { .. } => true,
        // A number or a piece of text inside JSON is a value like any other;
        // an object or an array holds them.
        Ty::Json(shape, _) => !shape.composite(),
        _ => false,
    }
}

impl Evaluator {
    /// The deepest field containing `bit`, as a path from the root. Its
    /// ancestors are the prefixes of that path, so one call gives the whole
    /// chain the hex cursor is standing in.
    ///
    /// Walking a repeat has to resolve its elements, so on a large templated
    /// file this costs what displaying it costs; the memo makes the second call
    /// cheap until the next edit.
    /// What to call this node: its field name, and what the structure says
    /// names it. `[9]` alone does not say which section it is; `[9] code`
    /// does, and keeping the index says which of the two custom ones this is.
    ///
    /// This is worked out here rather than when the node is resolved, because
    /// it means reading a sibling and resolving has to stay cheap.
    pub(super) fn label<S: Source>(&mut self, doc: &Document<S>, path: &[usize], r: &Resolved) -> R<String> {
        // A pointer-list child borrows the name of the record its offset came
        // from: tensor data called `[7] x_embedder.bias` says which weights
        // these are, where `[7]` says nothing. A record with no name of its
        // own contributes nothing, and the child keeps its index.
        if let Some(name) = self.pointed_from_name(doc, path) {
            return Ok(name);
        }
        // A field whose displayed name the format writes somewhere: a FITS
        // column is `col3` in every path and `col3 flux` on the row. The
        // declared name comes first, because that is the one an expression or
        // an edit has to be written with. See `Field::name_from`.
        if let Some(from) = self.name_from(path) {
            let here = Some((r.offset, r.limit));
            if let Ok(text) = self.text_at(doc, path, &from, here) {
                let text = text.trim();
                if !text.is_empty() {
                    return Ok(format!("{} {text}", r.name.text()));
                }
            }
        }
        let Ty::Struct(s) = r.ty.base() else { return Ok(r.name.text()) };
        let Some(by) = s.named_by.clone() else { return Ok(r.name.text()) };
        let Some(i) = s.fields.iter().position(|f| *f.name == *by) else { return Ok(r.name.text()) };
        let mut child = path.to_vec();
        child.push(i);
        // A field that cannot be read yet leaves the node with the name it had.
        let Ok(mut info) = self.node(doc, &child) else { return Ok(r.name.text()) };
        // A name a format wraps in a structure of its own, as GGUF wraps every
        // string in a length and then its bytes, is still the name. Follow the
        // field that is only the structure's contents until a value turns up.
        while info.composite {
            let Ty::Struct(inner) = self.memo[&child].ty.base().clone() else { break };
            let Some(c) = inner.contents.clone() else { break };
            let Some(j) = inner.fields.iter().position(|f| *f.name == *c) else { break };
            child.push(j);
            let Ok(next) = self.node(doc, &child) else { break };
            info = next;
        }
        let text = brief(&info.value);
        let text = text.trim_end();
        Ok(if text.is_empty() { r.name.text() } else { format!("{} {text}", r.name.text()) })
    }

    /// Where the field at `path` gets its displayed name from, when its
    /// structure says the file holds one. A property of the parent's
    /// declaration, the same as `contents` and `machinery` are.
    pub(super) fn name_from(&self, path: &[usize]) -> Option<Expr> {
        let (&idx, parent) = path.split_last()?;
        let Ty::Struct(s) = self.memo.get(parent)?.ty.base() else { return None };
        s.fields.get(idx)?.name_from.clone()
    }

    /// The name of the record whose offset placed this pointer-list child, when
    /// that record says more than the child's own index does.
    fn pointed_from_name<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> Option<String> {
        let (&idx, parent) = path.split_last()?;
        let Ty::PointerList { offsets, .. } = &self.memo.get(parent)?.ty else { return None };
        let offsets = offsets.clone();
        let mut p = self.find_field(parent, &offsets)?;
        p.push(idx);
        let name = self.node(doc, &p).ok()?.name;
        (name != format!("[{idx}]")).then_some(name)
    }

    /// The deepest field covering `bit`.
    ///
    /// A bit the root's own extent covers is found by walking down from it. A
    /// bit outside it may still be covered by a field that reads its contents
    /// somewhere else, which is how every object in an HDF5 file is placed, so
    /// the index of those stretches is asked next and the walk carries on from
    /// whichever placed the bit. A bit nothing covers is the root, which is
    /// what a gap has always been.
    pub fn locate<S: Source>(&mut self, doc: &Document<S>, bit: u64) -> R<Vec<usize>> {
        let mut path: Vec<usize> = Vec::new();
        self.resolve(doc, &path)?;
        let size = self.size_of(doc, &path)?;
        let root = self.memo[&path].clone();
        if bit >= doc.len_bits() {
            return fail("past the end of the file");
        }
        if bit < root.offset || bit >= root.offset + size {
            match self.placement_at(doc, bit)? {
                Some(p) => path = p,
                None => return Ok(Vec::new()),
            }
        }
        loop {
            // The cursor stops at a decoded stream's *contents*: those are at
            // offsets of the decoded bytes, and no bit of the file is any one
            // of them. What the decoder read to produce them is bits of the
            // file, though, and is what the cursor lands on: a bit of the run
            // is a bit of some block's header, or of its tables, or of one
            // literal. For a codec whose trace has no blocks the run is still
            // the answer, whole.
            self.resolve(doc, &path)?;
            if matches!(self.memo[&path].ty, Ty::Decoded { .. }) {
                let blocks = self.child_count(doc, &path)? >= 2;
                if blocks {
                    let mut into = path.clone();
                    into.push(1);
                    self.resolve(doc, &into)?;
                    let (at, size) = (self.memo[&into].offset, self.size_of(doc, &into)?);
                    // Bits of the run a wrapper put there -- zlib's two header
                    // bytes, its Adler-32 -- are not in any block, and the
                    // answer for those is still the run.
                    if (at..at + size).contains(&bit) {
                        path = into;
                        continue;
                    }
                }
                return Ok(path);
            }
            // How many elements a code section holds is a question only the
            // decoding of every one of them answers, and which one covers a
            // byte is not that question: the walk stops at the byte. So the
            // count is asked only where it is cheap, and where it is not the
            // element is found by walking to it from the nearest kept offset.
            let Some(n) = self.count_unless_walk(doc, &path)? else {
                match self.child_covering(doc, &path, u64::MAX, bit) {
                    Ok(Some(i)) => {
                        path.push(i);
                        continue;
                    }
                    Ok(None) => return Ok(path),
                    Err(e) if e.interrupted() => return Err(e),
                    // The run ended on something that would not parse, which
                    // is what the bytes after the last whole element of a run
                    // look like. Counting the run would have stopped there
                    // too, and the answer for a bit past the end of it is the
                    // run itself, which reads as a gap.
                    Err(_) => return Ok(path),
                }
            };
            if n == 0 {
                return Ok(path);
            }
            match self.child_at(doc, &path, n, bit)? {
                Some(i) => path.push(i),
                // Inside the parent but in none of its children: padding, or a
                // struct whose fields do not fill it.
                None => return Ok(path),
            }
        }
    }

    /// The outermost enclosing structure that reads as one row, or the field
    /// itself when nothing on the way to it is marked that way. `locate` has
    /// already resolved every step, so this only reads what it left behind.
    pub(super) fn inline_ancestor(&self, path: &[usize]) -> Vec<usize> {
        for n in 0..path.len() {
            let prefix = &path[..n];
            if let Some(r) = self.memo.get(prefix) {
                if matches!(r.ty.base(), Ty::Struct(s) if s.inline) {
                    return prefix.to_vec();
                }
            }
        }
        path.to_vec()
    }

    /// Every field across a stretch of the file, in order, for the annotation
    /// column. One call covers what is on screen rather than one field, so the
    /// column can be drawn without a round trip per byte.
    ///
    /// Two things are not one field each. A stretch no field covers, which is
    /// the slack at the end of a structure, comes back as a gap. A long run of
    /// values, such as W4V's 512 codes, or records, such as a model's tensor
    /// table, comes back as the run itself: several hundred internal rows would
    /// fill the column with less than one entry saying what the section is.
    pub fn spans<S: Source>(&mut self, doc: &Document<S>, from: u64, to: u64, max: usize) -> R<Vec<Span>> {
        self.resolve(doc, &[])?;
        let root_size = self.size_of(doc, &[])?;
        let root_offset = self.memo[&Vec::new()].offset;
        // As far as the file goes rather than as far as the root's own fields
        // go: what a placed field covers is past the second and inside the
        // first, and it is most of an HDF5 file.
        let _ = root_size;
        let end = to.min(doc.len_bits());
        let mut at = from.max(root_offset);
        let mut out: Vec<Span> = Vec::new();
        while at < end && out.len() < max {
            let path = self.locate(doc, at)?;
            // A structure marked to read on one row stands for its fields here.
            let path = self.inline_ancestor(&path);
            let inline = matches!(self.memo[&path].ty.base(), Ty::Struct(s) if s.inline);
            let info = self.node(doc, &path)?;
            let span = if at < info.offset_bits || at >= info.offset_bits + info.size_bits {
                self.gap_before_the_next_placement(doc, &path, &info, at)?
            } else if inline {
                self.one_row(doc, &path, &info)?
            } else if matches!(self.memo[&path].ty, Ty::Decoded { .. }) {
                // The whole run, as one entry that says what it is and how
                // many fields are inside. Before `composite`: its one child is
                // at offset 0 of the decoded bytes, and treating that as a bit
                // of the file would put the stream's contents at the front of
                // it.
                self.decoded_run(doc, &path, &info, at)?
            } else if info.composite {
                self.gap_inside(doc, &path, &info, at)?
            } else {
                self.field_or_its_run(doc, &path, &info, at)?
            };
            let next = span.offset_bits + span.size_bits;
            at = if next > at { next } else { at + 8 };
            if span.size_bits > 0 {
                out.push(span);
            }
        }
        Ok(out)
    }

    /// A bit no field covers at all, which in a format whose objects are all
    /// placed by address is the space between two of them. It runs to wherever
    /// the next placed stretch begins.
    fn gap_before_the_next_placement<S: Source>(
        &mut self,
        doc: &Document<S>,
        path: &[usize],
        info: &NodeInfo,
        at: u64,
    ) -> R<Span> {
        let mut span = self.span_of(doc, path, info)?;
        // The gap is the whole stretch between what ends before `at` and what
        // begins after it, not the part of it from `at` on: `at` is wherever
        // the view happens to start, and a gap that began at the top of the
        // screen would shrink as the view scrolled down through it.
        let root_end = self.memo[&Vec::new()].offset + self.size_of(doc, &[])?;
        let mut begins = if root_end <= at { root_end } else { 0 };
        if let Some(end) = self.placement_end_before(doc, at)? {
            begins = begins.max(end);
        }
        let ends = self.placement_after(doc, at)?.unwrap_or(doc.len_bits()).max(at + 8);
        span.gap = true;
        span.offset_bits = begins;
        span.size_bits = ends - begins;
        span.count = 0;
        Ok(span)
    }

    /// Inside a structure, but in none of its children: the template has
    /// nothing to say about these bytes.
    ///
    /// The gap runs to whatever comes next, which need not be a child of the
    /// structure it is in. In a PDF whose objects a cross-reference stream
    /// places, the list of them is empty and stretches to the end of the file,
    /// while the table, the trailer and the end marker are placed beside it
    /// rather than inside it. Ending the gap where the list ends would swallow
    /// all three and leave most of the file unannotated.
    fn gap_inside<S: Source>(&mut self, doc: &Document<S>, path: &[usize], info: &NodeInfo, at: u64) -> R<Span> {
        let mut span = self.span_of(doc, path, info)?;
        // From where the last thing before `at` ends, not from `at` itself,
        // which is only where the view starts: see `gap_before_the_next_placement`.
        let mut begins = info.offset_bits;
        if let Some(end) = self.prev_child_end(doc, path, at)? {
            begins = begins.max(end);
        }
        if let Some(end) = self.placement_end_before(doc, at)? {
            begins = begins.max(end);
        }
        let mut ends = info.offset_bits + info.size_bits;
        for k in (0..=path.len()).rev() {
            if let Some(next) = self.next_child_start(doc, &path[..k], at)? {
                ends = ends.min(next);
            }
        }
        if let Some(next) = self.placement_after(doc, at)? {
            ends = ends.min(next);
        }
        span.gap = true;
        span.offset_bits = begins;
        span.size_bits = ends - begins;
        span.count = 0;
        Ok(span)
    }

    /// A compressed run, as the one entry it is: the whole run, named by its
    /// codec, standing for however many fields are inside it.
    ///
    /// The count comes from the template rather than from opening the stream.
    /// The hex view scrolls past every stream in the file and wants none of
    /// them unpacked to draw a row, and what a template declares inside a
    /// stream is the same whatever the bytes turn out to be.
    fn decoded_run<S: Source>(&mut self, doc: &Document<S>, path: &[usize], info: &NodeInfo, at: u64) -> R<Span> {
        let mut span = self.span_of(doc, path, info)?;
        // A run whose decoder kept a trace is not one entry: the blocks are
        // entries of their own, and what is left of the run is whatever a
        // wrapper put in front of them and after them. Drawing the whole run
        // here would cover the blocks and the view would step past all of
        // them.
        if self.has_blocks(path) {
            let mut blocks = path.to_vec();
            blocks.push(1);
            self.resolve(doc, &blocks)?;
            let (from, size) = (self.memo[&blocks].offset, self.size_of(doc, &blocks)?);
            let end = info.offset_bits + info.size_bits;
            let (a, b) = if at < from { (info.offset_bits, from) } else { (from + size, end) };
            span.offset_bits = a;
            span.size_bits = b.saturating_sub(a);
        }
        let Ty::Decoded { inner, .. } = &self.memo[path].ty else { return Ok(span) };
        let mut inner = (**inner).clone();
        for _ in 0..8 {
            let Ty::Named(n) = &inner else { break };
            match self.template.types.get(&**n) {
                Some(t) => inner = t.clone(),
                None => break,
            }
        }
        span.count = match inner.base() {
            Ty::Struct(s) => s.fields.len() as u64,
            _ => 1,
        };
        Ok(span)
    }

    /// A structure marked to read on one row, as the one row it reads as.
    fn one_row<S: Source>(&mut self, doc: &Document<S>, path: &[usize], info: &NodeInfo) -> R<Span> {
        let mut span = self.span_of(doc, path, info)?;
        let mut parts = Vec::new();
        self.one_line(doc, path, &mut parts)?;
        span.line = Some(parts.join(" "));
        Ok(span)
    }

    /// A field, or the long run of values or records it belongs to. Several
    /// hundred rows of a model's tensor table would fill the column with less
    /// than one entry saying what the section is, so the run stands for them,
    /// with the first few sampled to say what is in it.
    fn field_or_its_run<S: Source>(
        &mut self,
        doc: &Document<S>,
        path: &[usize],
        info: &NodeInfo,
        at: u64,
    ) -> R<Span> {
        let span = self.span_of(doc, path, info)?;
        let Some((run, count)) = self.collapsible(doc, path)? else { return Ok(span) };
        let run_info = self.node(doc, &run)?;
        // Pointer-heavy formats can reach a leaf through an overlapping
        // placement whose repeated ancestor has already ended. Collapsing that
        // ancestor would return a span behind `at` forever. Only substitute the
        // run when it covers the byte this request is actually advancing from.
        if run_info.offset_bits > at || at >= run_info.offset_bits + run_info.size_bits {
            return Ok(span);
        }
        let mut span = self.span_of(doc, &run, &run_info)?;
        span.count = count;
        let mut covered = 0u64;
        for i in 0..count.min(SAMPLE) {
            let mut elem = run.clone();
            elem.push(i as usize);
            let info = self.node(doc, &elem)?;
            let value = brief(&info.value);
            if !value.is_empty() {
                span.sample.push(value);
            } else if info.composite {
                // A named record such as `[81] phonemizer.rules.keys`
                // contributes the useful name, not its array index.
                let index = format!("[{i}]");
                let named = info.name.strip_prefix(&index).unwrap_or(&info.name).trim();
                if !named.is_empty() {
                    span.sample.push(named.to_string());
                }
            }
            if info.size_bits > 0 {
                span.parts.push(SpanPart { size_bits: info.size_bits, label: info.name, rest: false });
                covered = covered.saturating_add(info.size_bits);
            }
        }
        // A run of values that each read as nothing, such as matching
        // signatures, is better left to say only how many there are.
        if span.sample.iter().all(|s| s.is_empty()) {
            span.sample.clear();
        }
        if covered < span.size_bits {
            span.parts.push(SpanPart {
                size_bits: span.size_bits - covered,
                label: format!("{} more", count.saturating_sub(SAMPLE)),
                rest: true,
            });
        }
        Ok(span)
    }

    pub(super) fn span_of<S: Source>(&mut self, doc: &Document<S>, path: &[usize], info: &NodeInfo) -> R<Span> {
        let mut trail = Vec::new();
        for k in 1..path.len() {
            self.resolve(doc, &path[..k])?;
            // A field a structure calls its contents adds a step to the trail
            // and nothing to what it says.
            if self.is_contents(&path[..k]) {
                continue;
            }
            let r = self.memo[&path[..k]].clone();
            trail.push(self.label(doc, &path[..k], &r)?);
        }
        Ok(Span {
            path: path.to_vec(),
            offset_bits: info.offset_bits,
            size_bits: info.size_bits,
            name: info.name.clone(),
            trail,
            type_name: info.type_name.clone(),
            value: info.value.clone(),
            gap: false,
            count: 0,
            line: None,
            sample: Vec::new(),
            parts: Vec::new(),
            bits: self.bit_roles(doc, path, info),
        })
    }

    /// The framing-and-value split of a field that stores a variable-length
    /// number. Reading its bytes is a second read of bytes the value was
    /// already decoded from, so a range that is not loaded means the split is
    /// simply not offered on this pass; the view redraws when the bytes land.
    fn bit_roles<S: Source>(
        &mut self,
        doc: &Document<S>,
        path: &[usize],
        info: &NodeInfo,
    ) -> Option<crate::varintbits::BitRoles> {
        if info.size_bits == 0 || info.size_bits % 8 != 0 || info.offset_bits % 8 != 0 {
            return None;
        }
        let r = self.memo.get(path)?.clone();
        let ty = r.ty.clone();
        if !crate::varintbits::splits(&ty) {
            return None;
        }
        let bytes = self.read(doc, &r, info.offset_bits, info.size_bits).ok()?;
        crate::varintbits::bit_roles(&ty, &bytes)
    }

    /// Whether this node is the field its parent calls its own contents.
    pub(super) fn is_contents(&self, path: &[usize]) -> bool {
        let Some((&last, parent)) = path.split_last() else { return false };
        let Some(r) = self.memo.get(parent) else { return false };
        let Ty::Struct(s) = r.ty.base() else { return false };
        let Some(by) = &s.contents else { return false };
        s.fields.get(last).is_some_and(|f| *f.name == *by)
    }

    /// A structure that reads on one row, as its fields' values in order. A
    /// field that is itself a structure contributes its own fields, so a wasm
    /// instruction whose immediate has two parts still reads as one line.
    pub(super) fn one_line<S: Source>(&mut self, doc: &Document<S>, path: &[usize], out: &mut Vec<String>) -> R<()> {
        let info = self.node(doc, path)?;
        if !info.composite {
            // A field of no bits is an absence, not an empty value: the switch
            // for an opcode with no immediate selects one.
            if info.size_bits > 0 {
                let text = brief(&info.value);
                if !text.is_empty() {
                    out.push(text);
                }
            }
            return Ok(());
        }
        for i in 0..info.child_count as usize {
            let mut child = path.to_vec();
            child.push(i);
            self.one_line(doc, &child, out)?;
        }
        Ok(())
    }

    /// The nearest repeated run `path` sits in, if it is long enough to be
    /// worth showing as one entry. Records use the higher threshold above.
    pub(super) fn collapsible<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Option<(Vec<usize>, u64)>> {
        for k in 0..path.len() {
            let ty = self.memo[&path[..k]].ty.clone();
            let elem = match &ty {
                Ty::Array { elem, .. } | Ty::Repeat { elem, .. } => (**elem).clone(),
                _ => continue,
            };
            // Instructions stay one entry per line however many there are.
            // A run of them is a program, and a row saying "245,678
            // instructions" is the one thing a reader of a program does not
            // want in place of the program.
            if matches!(elem.base(), Ty::Insn { .. }) {
                continue;
            }
            // Text stays one entry per line: GUANO lines are each worth reading.
            // A type that only says what it is once it is placed, such as a
            // WAVE sample whose width an earlier chunk declared, is judged by
            // what the first element turned out to be.
            let complex = if !plain(&elem) {
                let mut first = path[..k].to_vec();
                first.push(0);
                match self.node(doc, &first) {
                    Ok(info) => info.composite,
                    _ => continue,
                }
            } else {
                false
            };
            let n = self.child_count(doc, &path[..k])?;
            let threshold = if complex { COLLAPSE_COMPLEX_RUN } else { COLLAPSE_RUN };
            if complex && n > 0 && self.size_of(doc, &path[..k])? / n > COLLAPSE_ELEMENT_BYTES * 8 {
                continue;
            }
            if n >= threshold {
                return Ok(Some((path[..k].to_vec(), n)));
            }
        }
        Ok(None)
    }

    /// Which child of `path` covers `bit`, if any.
    /// Which child of a `Traced` node covers a bit of the file, worked out
    /// from the trace rather than by placing anything.
    fn traced_at(&self, path: &[usize], part: crate::template::TracedPart, bit: u64) -> Option<usize> {
        use crate::template::TracedPart as P;
        // The trace counts bits from the front of the run, and the cursor
        // counts them from the front of the file.
        let (base, trace) = self.trace_for(path)?;
        let want = bit.checked_sub(base)?;
        match part {
            P::Blocks => {
                let k = trace.blocks().partition_point(|b| b.in_bits.start <= want);
                let i = k.checked_sub(1)?;
                (want < trace.blocks()[i].in_bits.end).then_some(i)
            }
            P::Block(i) => {
                let view = super::traced::BlockView::of(trace, i)?;
                if want >= view.symbols_at(trace) {
                    return (!view.symbols.is_empty()).then_some(view.head.len());
                }
                let k = trace.index_in(want)?;
                (view.head.contains(&(k as u32))).then(|| k - view.head.start as usize)
            }
            P::Symbols(i) => {
                let view = super::traced::BlockView::of(trace, i)?;
                let k = trace.index_in(want)?;
                (view.symbols.contains(&(k as u32))).then(|| k - view.symbols.start as usize)
            }
        }
    }

    pub(super) fn child_at<S: Source>(&mut self, doc: &Document<S>, path: &[usize], n: u64, bit: u64) -> R<Option<usize>> {
        let r = self.memo[path].clone();
        // A trace already knows which step covers a bit, and knows it by
        // halving. So a symbol run of a hundred thousand answers the cursor
        // without placing a single symbol before the one under it.
        if let Ty::Traced { part } = &r.ty {
            return Ok(self.traced_at(path, *part, bit));
        }
        // Same-sized elements: go straight to the one that covers the bit,
        // without putting a single other element in memory.
        if let Some(each) = self.stride(doc, path, &r.ty)? {
            if each > 0 {
                let i = (bit - r.offset) / each;
                return Ok(if i < n { Some(i as usize) } else { None });
            }
        }
        // A long list is walked from the nearest kept offset instead, so that
        // finding the element under the cursor does not put the list back in
        // memory element by element.
        if matches!(r.ty, Ty::Array { .. } | Ty::Repeat { .. }) && self.guarded(doc, path, &r)? {
            return self.child_covering(doc, path, n, bit);
        }
        // A pointer list knows where its children start, in order: the one
        // that covers a bit is the last one starting at or before it. Asking
        // all four hundred tensors of a model which of them the cursor is in,
        // for every row of every screen, is what this is instead of.
        // A chain's children are scattered the same way, and are found the
        // same way once the walk has followed it: sorted by where they start,
        // which is not the order they are numbered in.
        if matches!(r.ty, Ty::Chain { .. } | Ty::PointerList { .. }) {
            let starts = self.scattered_starts(doc, path, &r)?;
            let k = starts.partition_point(|(s, _)| *s <= bit);
            if k > 0 {
                let i = starts[k - 1].1;
                let mut p = path.to_vec();
                p.push(i);
                let covers = match self.resolve(doc, &p) {
                    Ok(()) => self.size_of(doc, &p).map(|size| bit < self.memo[&p].offset + size),
                    Err(e) => Err(e),
                };
                match covers {
                    Ok(true) => return Ok(Some(i)),
                    Err(e) if e.interrupted() => return Err(e),
                    // Between two children, or a child that will not parse:
                    // fall through and look properly, since children may
                    // overlap or be missing and the nearest start is only a
                    // good guess.
                    _ => {}
                }
            }
        }
        // Children of a pointer list are in the order their offsets are in,
        // not the order they sit in, so every one has to be looked at, and one
        // that does not parse is passed over rather than taking the page with it.
        // A structure holding a field that points elsewhere has children that
        // are not in the order they sit in, the same as a pointer list, so
        // every one has to be looked at rather than stopping at the first that
        // starts past the bit.
        let scattered = matches!(r.ty, Ty::PointerList { .. } | Ty::Chain { .. })
            || self.has_pointing_field(&r.ty)
            || self.has_low_bit_first_field(&r.ty);
        let mut p = path.to_vec();
        for i in 0..n as usize {
            p.push(i);
            // A chain covers no bytes where it is declared, so asking how long
            // it is says nothing about whether the bit is inside it: what
            // covers bytes is the elements the walk found, wherever they are.
            // Asking the chain itself is the same halving `child_at` does for
            // any list of scattered children.
            let chain = match self.resolve(doc, &p) {
                Ok(()) => matches!(self.memo[&p].ty, Ty::Chain { .. }),
                Err(e) if scattered && !e.interrupted() => {
                    p.pop();
                    continue;
                }
                Err(e) => {
                    p.pop();
                    return Err(e);
                }
            };
            if chain {
                let inside = match self.child_count(doc, &p) {
                    Ok(count) => self.child_at(doc, &p, count, bit),
                    Err(e) => Err(e),
                };
                p.pop();
                match inside {
                    Ok(Some(_)) => return Ok(Some(i)),
                    Err(e) if e.interrupted() => return Err(e),
                    _ => continue,
                }
            }
            let placed = match self.resolve(doc, &p) {
                Ok(()) => match self.memo[&p].ty {
                    // The field covers nothing where it is declared; what it
                    // points at is what the cursor can be inside of.
                    Ty::At { .. } => {
                        p.push(0);
                        let inner = match self.resolve(doc, &p) {
                            Ok(()) => self.size_of(doc, &p).map(|size| (self.memo[&p].offset, size)),
                            Err(e) => Err(e),
                        };
                        p.pop();
                        inner
                    }
                    _ => self.size_of(doc, &p).map(|size| (self.memo[&p].offset, size)),
                },
                Err(e) => Err(e),
            };
            p.pop();
            let (off, size) = match placed {
                Ok(v) => v,
                Err(e) if scattered && !e.interrupted() => continue,
                Err(e) => return Err(e),
            };
            if bit < off && !scattered {
                return Ok(None);
            }
            if bit >= off && bit < off + size {
                return Ok(Some(i));
            }
        }
        Ok(None)
    }

    /// Whether a structure has a field whose contents are somewhere else in
    /// the file, which is what stops its children from being in order.
    fn has_pointing_field(&self, ty: &Ty) -> bool {
        matches!(ty.base(), Ty::Struct(s) if s.fields.iter().any(|f| matches!(f.ty, Ty::At { .. } | Ty::Chain { .. })))
    }

    /// Whether a structure packs any of its fields from the bottom of a byte.
    ///
    /// Such a structure's fields are not in the order they sit in either: the
    /// first field declared inside a byte is the last one in it, so the search
    /// for what covers a bit cannot stop at the first field starting past it.
    /// See [`crate::decode::lsb_offset`].
    fn has_low_bit_first_field(&self, ty: &Ty) -> bool {
        matches!(ty.base(), Ty::Struct(s) if s.fields.iter().any(|f| {
            matches!(crate::decode::packed_int(&f.ty), Some((bits, e)) if bits % 8 != 0 && e == crate::template::Endian::Little)
        }))
    }

    /// The first child of a pointer list that starts after `bit`. What is
    /// between them belongs to no field, and saying so needs to know where the
    /// next one begins: free space inside a page sits between cells, not after
    /// all of them.
    pub(super) fn next_child_start<S: Source>(&mut self, doc: &Document<S>, path: &[usize], bit: u64) -> R<Option<u64>> {
        // The starts are in order, so the first one past the bit is a halving.
        if matches!(self.memo[path].ty, Ty::PointerList { .. } | Ty::Chain { .. }) {
            let r = self.memo[path].clone();
            let starts = self.scattered_starts(doc, path, &r)?;
            return Ok(starts.get(starts.partition_point(|(s, _)| *s <= bit)).map(|(s, _)| *s));
        }
        // A run whose count is a walk of the whole container says nothing here
        // rather than decoding a code section to bound a gap. The gap keeps the
        // wider edge the node's own extent gives it, which is what it had
        // before there was anything to narrow it with.
        let Some(n) = self.count_unless_walk(doc, path)? else { return Ok(None) };
        let mut best: Option<u64> = None;
        let mut p = path.to_vec();
        for i in 0..n as usize {
            p.push(i);
            let placed = self.resolve(doc, &p).map(|()| self.memo[&p].offset);
            p.pop();
            match placed {
                Ok(off) if off > bit => best = Some(best.map_or(off, |b: u64| b.min(off))),
                Ok(_) => {}
                Err(e) if e.interrupted() => return Err(e),
                Err(_) => {}
            }
        }
        Ok(best)
    }

    /// Where the child of `path` that begins last at or before `bit` ends, if
    /// it ends at or before `bit`. None when no child begins by then, or when
    /// the one that does runs past `bit`, which means `bit` is inside it.
    pub(super) fn prev_child_end<S: Source>(&mut self, doc: &Document<S>, path: &[usize], bit: u64) -> R<Option<u64>> {
        let mut p = path.to_vec();
        let last: Option<usize> = if matches!(self.memo[path].ty, Ty::PointerList { .. } | Ty::Chain { .. }) {
            let r = self.memo[path].clone();
            let starts = self.scattered_starts(doc, path, &r)?;
            let n = starts.partition_point(|(s, _)| *s <= bit);
            starts[..n].iter().max_by_key(|(s, _)| *s).map(|(_, i)| *i)
        } else {
            // A run whose count is a walk of its container is asked where the
            // bit is instead, which walks no further than the bit. The walk
            // leaves behind where the element before it ended, which is
            // exactly what this wanted: the slack at the end of a run of
            // records begins where the last whole record stopped, not where
            // the run did.
            let Some(n) = self.count_unless_walk(doc, path)? else {
                return match self.child_covering(doc, path, u64::MAX, bit) {
                    // Inside a child, so nothing ends before the bit.
                    Ok(Some(_)) => Ok(None),
                    Err(e) if e.interrupted() => Err(e),
                    _ => Ok(self.list(path).walk_at.map(|(_, at)| at).filter(|at| *at <= bit)),
                };
            };
            let mut best: Option<(u64, usize)> = None;
            for i in 0..n as usize {
                p.push(i);
                let placed = self.resolve(doc, &p).map(|()| self.memo[&p].offset);
                p.pop();
                match placed {
                    Ok(off) if off <= bit && best.map_or(true, |(b, _)| off >= b) => best = Some((off, i)),
                    Ok(_) => {}
                    Err(e) if e.interrupted() => return Err(e),
                    Err(_) => {}
                }
            }
            best.map(|(_, i)| i)
        };
        let Some(i) = last else { return Ok(None) };
        p.push(i);
        self.resolve(doc, &p)?;
        let end = self.memo[&p].offset + self.size_of(doc, &p)?;
        Ok((end <= bit).then_some(end))
    }
}
