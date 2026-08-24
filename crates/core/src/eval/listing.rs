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
    /// How many fields this entry stands for, when a run of numbers is shown
    /// as one. Zero for a single field.
    pub count: u64,
    /// A structure marked to read on one row, already joined: `local.get 0`
    /// rather than an `op` row and an `imm` row. None for everything else,
    /// which reads as its own value.
    pub line: Option<String>,
    /// The first few values of a run shown as one entry. `512 values` says how
    /// many and nothing about what, and a run of zeroes and a run of samples
    /// are worth telling apart without opening either.
    pub sample: Vec<String>,
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
        // Nothing to say when the bytes are what the format asked for. The
        // mismatch is the only half worth a reader's attention.
        Value::Magic { ok } => (if *ok { "" } else { "does not match" }).to_string(),
        Value::Composite { .. } => String::new(),
    }
}

/// A run of these is worth one entry rather than one each.
const COLLAPSE_RUN: u64 = 8;

/// A type that holds one number or one run of bytes, and nothing inside it.
pub(super) fn plain(ty: &Ty) -> bool {
    match ty {
        Ty::Enum { inner, .. } | Ty::Flags { inner, .. } => plain(inner),
        Ty::UInt { .. }
        | Ty::Int { .. }
        | Ty::F16(_)
        | Ty::F32(_)
        | Ty::F64(_)
        | Ty::Fixed { .. }
        | Ty::Leb128 { .. }
        | Ty::Vlq
        | Ty::SqliteVarint
        | Ty::Magic(_)
        | Ty::Bytes(_) => true,
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
        let Ty::Struct(s) = r.ty.base() else { return Ok(r.name.clone()) };
        let Some(by) = s.named_by.clone() else { return Ok(r.name.clone()) };
        let Some(i) = s.fields.iter().position(|f| f.name == by) else { return Ok(r.name.clone()) };
        let mut child = path.to_vec();
        child.push(i);
        // A field that cannot be read yet leaves the node with the name it had.
        let Ok(mut info) = self.node(doc, &child) else { return Ok(r.name.clone()) };
        // A name a format wraps in a structure of its own, as GGUF wraps every
        // string in a length and then its bytes, is still the name. Follow the
        // field that is only the structure's contents until a value turns up.
        while info.composite {
            let Ty::Struct(inner) = self.memo[&child].ty.base().clone() else { break };
            let Some(c) = inner.contents.clone() else { break };
            let Some(j) = inner.fields.iter().position(|f| f.name == c) else { break };
            child.push(j);
            let Ok(next) = self.node(doc, &child) else { break };
            info = next;
        }
        let text = brief(&info.value);
        let text = text.trim_end();
        Ok(if text.is_empty() { r.name.clone() } else { format!("{} {text}", r.name) })
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

    pub fn locate<S: Source>(&mut self, doc: &Document<S>, bit: u64) -> R<Vec<usize>> {
        let mut path: Vec<usize> = Vec::new();
        self.resolve(doc, &path)?;
        let size = self.size_of(doc, &path)?;
        let root = self.memo[&path].clone();
        if bit < root.offset || bit >= root.offset + size {
            return fail("outside the template");
        }
        loop {
            let n = self.child_count(doc, &path)?;
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
    /// plain numbers, such as W4V's 512 codes, comes back as the run itself:
    /// several hundred entries reading `[0]`, `[1]`, `[2]` would fill the
    /// column with less than one entry saying what the run is.
    pub fn spans<S: Source>(&mut self, doc: &Document<S>, from: u64, to: u64, max: usize) -> R<Vec<Span>> {
        self.resolve(doc, &[])?;
        let root_size = self.size_of(doc, &[])?;
        let root_offset = self.memo[&Vec::new()].offset;
        let end = to.min(root_offset + root_size);
        let mut at = from.max(root_offset);
        let mut out: Vec<Span> = Vec::new();
        while at < end && out.len() < max {
            let path = self.locate(doc, at)?;
            // A structure marked to read on one row stands for its fields here.
            let path = self.inline_ancestor(&path);
            let inline = matches!(self.memo[&path].ty.base(), Ty::Struct(s) if s.inline);
            let info = self.node(doc, &path)?;
            let mut span = self.span_of(doc, &path, &info)?;
            if inline {
                let mut parts = Vec::new();
                self.one_line(doc, &path, &mut parts)?;
                span.line = Some(parts.join(" "));
            } else if info.composite {
                // Inside it, but in none of its children: the template has
                // nothing to say about these bytes.
                span.gap = true;
                span.offset_bits = at;
                let mut ends = info.offset_bits + info.size_bits;
                if matches!(self.memo[&path].ty, Ty::PointerList { .. }) {
                    if let Some(next) = self.next_child_start(doc, &path, at)? {
                        ends = ends.min(next);
                    }
                }
                span.size_bits = ends - at;
                span.count = 0;
            } else if let Some((run, count)) = self.collapsible(doc, &path)? {
                let run_info = self.node(doc, &run)?;
                span = self.span_of(doc, &run, &run_info)?;
                span.count = count;
                for i in 0..count.min(SAMPLE) {
                    let mut elem = run.clone();
                    elem.push(i as usize);
                    span.sample.push(brief(&self.node(doc, &elem)?.value));
                }
                // A run of values that each read as nothing, such as matching
                // signatures, is better left to say only how many there are.
                if span.sample.iter().all(|s| s.is_empty()) {
                    span.sample.clear();
                }
            }
            let next = span.offset_bits + span.size_bits;
            at = if next > at { next } else { at + 8 };
            if span.size_bits > 0 {
                out.push(span);
            }
        }
        Ok(out)
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
        })
    }

    /// Whether this node is the field its parent calls its own contents.
    pub(super) fn is_contents(&self, path: &[usize]) -> bool {
        let Some((&last, parent)) = path.split_last() else { return false };
        let Some(r) = self.memo.get(parent) else { return false };
        let Ty::Struct(s) = r.ty.base() else { return false };
        let Some(by) = &s.contents else { return false };
        s.fields.get(last).is_some_and(|f| &f.name == by)
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

    /// The nearest run of plain numbers `path` sits in, if it is long enough to
    /// be worth showing as one entry.
    pub(super) fn collapsible<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Option<(Vec<usize>, u64)>> {
        for k in 0..path.len() {
            let ty = self.memo[&path[..k]].ty.clone();
            let elem = match &ty {
                Ty::Array { elem, .. } | Ty::Repeat { elem, .. } => (**elem).clone(),
                _ => continue,
            };
            // Text stays one entry per line: GUANO lines are each worth reading.
            // A type that only says what it is once it is placed, such as a
            // WAVE sample whose width an earlier chunk declared, is judged by
            // what the first element turned out to be.
            if !plain(&elem) {
                let mut first = path[..k].to_vec();
                first.push(0);
                match self.node(doc, &first) {
                    Ok(info) if !info.composite => {}
                    _ => continue,
                }
            }
            let n = self.child_count(doc, &path[..k])?;
            if n >= COLLAPSE_RUN {
                return Ok(Some((path[..k].to_vec(), n)));
            }
        }
        Ok(None)
    }

    /// Which child of `path` covers `bit`, if any.
    pub(super) fn child_at<S: Source>(&mut self, doc: &Document<S>, path: &[usize], n: u64, bit: u64) -> R<Option<usize>> {
        let r = self.memo[path].clone();
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
        // Children of a pointer list are in the order their offsets are in,
        // not the order they sit in, so every one has to be looked at, and one
        // that does not parse is passed over rather than taking the page with it.
        let scattered = matches!(r.ty, Ty::PointerList { .. });
        let mut p = path.to_vec();
        for i in 0..n as usize {
            p.push(i);
            let placed = match self.resolve(doc, &p) {
                Ok(()) => self.size_of(doc, &p).map(|size| (self.memo[&p].offset, size)),
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

    /// The first child of a pointer list that starts after `bit`. What is
    /// between them belongs to no field, and saying so needs to know where the
    /// next one begins: free space inside a page sits between cells, not after
    /// all of them.
    pub(super) fn next_child_start<S: Source>(&mut self, doc: &Document<S>, path: &[usize], bit: u64) -> R<Option<u64>> {
        let n = self.child_count(doc, path)?;
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

}
