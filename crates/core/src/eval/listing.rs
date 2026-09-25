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
    /// What one of those is called, singular, when the format has a word for
    /// it: a deflate block holds symbols, not values. None when it has none.
    pub unit: Option<String>,
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
    /// True when the template wrote this structure to hold one value in
    /// several fields, so the name it gave the structure is its own
    /// bookkeeping: `Elsewhere` is not a word the TIFF specification uses, and
    /// a tooltip offering it names a thing nobody can look up. See
    /// [`crate::template::StructDef::inline`].
    pub inline: bool,
    /// What is wrong with this field's value, when something is. The node's
    /// own, since a span is a node: the hex view's chips are built from these
    /// and not from nodes, and a chip that cannot say a signature is wrong is
    /// a view the reader has to leave to find out. See `NodeInfo::problem`.
    pub problem: Option<Problem>,
    /// Wrong values found under this span so far, invalid then undefined. See
    /// `NodeInfo::problems_within`.
    pub problems_within: (u32, u32),
    /// True when these bytes are a document of their own and can be opened as
    /// one: a compressed run that unpacked, or the contents of one.
    ///
    /// The listing has always been able to work this out, because it holds the
    /// node and the node carries `decoded` and `space_root`. A span does not
    /// carry the node, so the hex view's annotation column could not, and a
    /// reader running down the bytes had no way of knowing there was a file in
    /// front of them. It is the same question either surface asks, so it is
    /// answered once, here.
    pub opens: bool,
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
        // A named value gives its name and drops the number behind it, which
        // is this reading's whole point. A value with no name has only the
        // number, and says so in the words the node says them in, so that a
        // row and the panel beside it do not word the same value two ways.
        Value::Enum { raw, name, hex } => match name {
            Some(n) => n.clone(),
            None if *hex && *raw >= 0 => format!("0x{raw:02x} (unknown)"),
            None => format!("{raw} (unknown)"),
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
        Value::Magic { bytes, .. } => magic_reading(bytes),
        Value::Composite { .. } => String::new(),
    }
}

/// How many bytes something is, where its bytes are what there is to say
/// about it. Counted in bytes rather than rounded to KiB: this stands beside
/// the bytes themselves.
pub(super) fn byte_text(n: u64) -> String {
    if n == 1 { "1 byte".to_string() } else { format!("{} bytes", grouped(n)) }
}

/// An address as the reader sees it everywhere else: `@0x9e4`, with the bit
/// spelled out as `@0x9e4+3b` where the field starts inside a byte, and a `+`
/// after the mark for an address counted inside an unpacked stream rather than
/// in the file.
///
/// The same spelling `formatAddress` writes in the web app, and here rather
/// than there because a reading built in the core has to say an address in the
/// middle of a line: a chip and the panel beside it both show that line, and
/// two formatters would be two chances for them to disagree.
pub fn address_text(bits: u64, space: u32) -> String {
    let (byte, rem) = (bits / 8, bits % 8);
    let plus = if space == 0 { "" } else { "+" };
    let bit = if rem == 0 { String::new() } else { format!("+{rem}b") };
    format!("@{plus}0x{byte:x}{bit}")
}

/// A number with its thousands marked off, which is what makes `626,038`
/// readable at a glance and `626038` a thing to be counted.
pub(super) fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// How many of something, named by what they are: `64 values`, `3 components`.
pub(super) fn count_text(n: u64, unit: &str) -> String {
    if n == 1 { format!("1 {unit}") } else { format!("{} {}", grouped(n), plural(unit)) }
}

/// What a reader sees said in the empty space where a reading should be, for
/// a field that was read and holds nothing: ELF section 0 has a name, the name
/// is at an address, and the string at that address is the empty string.
pub(super) const EMPTY: &str = "(empty)";

/// What a field read somewhere else reads as, after the address it was read
/// at: the last step of the reading, shared by the line drawn over the pointer
/// and the clause under the Points to row, so the two cannot disagree.
///
/// `said` is what the walk over the far end came back with. Where that is
/// nothing there are two more answers to try before giving up. A list has no
/// line of its own, since its elements are a table rather than a row, so it
/// says how many it holds in the word the panel counts them by. And a value
/// that is genuinely empty says so: without this an address with nothing after
/// it reads as a reading the core could not produce, which is the one thing
/// that is not the case.
pub(super) fn far_end(there: &NodeInfo, said: String) -> String {
    if !said.is_empty() {
        return said;
    }
    if there.list {
        return count_text(there.child_count, there.unit.as_deref().unwrap_or("value"));
    }
    // Only a value the core read and found empty. A composite has no value of
    // its own, and bytes still on their way read as `…` rather than as nothing.
    match &there.value {
        Value::Str(s) if s.is_empty() => EMPTY.to_string(),
        Value::Bytes { len: 0, .. } => EMPTY.to_string(),
        _ => String::new(),
    }
}

/// More than one of them. The nouns here are the words formats use for what
/// they hold, so this covers the endings those run to and no more.
fn plural(noun: &str) -> String {
    let last = noun.chars().last().unwrap_or(' ');
    let before = noun.chars().rev().nth(1).unwrap_or(' ');
    if last == 'y' && !matches!(before, 'a' | 'e' | 'i' | 'o' | 'u') {
        return format!("{}ies", &noun[..noun.len() - 1]);
    }
    if noun.ends_with('s') || noun.ends_with('x') || noun.ends_with('z') || noun.ends_with("ch") || noun.ends_with("sh") {
        return format!("{noun}es");
    }
    format!("{noun}s")
}

/// How a signature reads in one line.
///
/// The bytes as C would write a string, so a reader sees the name in them and
/// the bytes that are not a name at once. What was wanted instead, for a
/// signature that is not what the template asked for, is not said here: the
/// value column says what the bytes are and `NodeInfo::problem` says what is
/// wrong with them, in one place every view reads. See `eval::problem`.
///
/// One caveat, and it is not this function's: `text::c_string` puts two bases
/// in one line, so Matroska's reads `"\032E\xdf\xa3"` where `\032` is the byte
/// the gutter calls `0x1a`. See the gap of its own about that.
pub fn magic_reading(bytes: &[u8]) -> String {
    crate::text::c_string(bytes)
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

/// How many placed stretches covering one bit `locate` walks down from before
/// it settles for a gap. They nest, so this is how deep the nesting has to be
/// before an answer is missed, and each try is a descent from a stretch to the
/// bit. An HDF5 name inside a heap inside an object header is three.
const PLACEMENTS_TRIED: usize = 8;

/// Whether a node of this type is made of other nodes, so that one with none
/// of them covering a bit is a gap rather than the field at that bit. The
/// same types `Evaluator::node` calls composite.
fn holds_fields(ty: &Ty) -> bool {
    match ty {
        Ty::Struct(_)
        | Ty::Array { .. }
        | Ty::Repeat { .. }
        | Ty::PointerList { .. }
        | Ty::Chain { .. }
        | Ty::Gather { .. }
        | Ty::At { .. }
        | Ty::Decoded { .. }
        | Ty::Traced { .. }
        | Ty::Stitched { .. }
        | Ty::Raster { .. } => true,
        Ty::Json(shape, _) => shape.composite(),
        Ty::Pickle(..) => true,
        _ => false,
    }
}

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
        | Ty::IbmF32(_)
        | Ty::Fixed { .. }
        | Ty::Leb128 { .. }
        | Ty::Zigzag
        | Ty::EbmlVint { .. }
        | Ty::Vlq
        | Ty::SqliteVarint
        | Ty::SevenZipNumber
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
        // A field whose displayed name the format writes somewhere: a FITS
        // column is `col3` in every path and `col3 flux` on the row. The
        // declared name comes first, because that is the one an expression or
        // an edit has to be written with. See `Field::name_from`, and
        // `Field::elem_name_from` for the elements of a list, which read
        // `[1] y`. Asked before the borrowed name below, since this one the
        // template asked for by name.
        if let Some(from) = self.name_from(path) {
            let here = Some((r.offset, r.limit));
            if let Ok(text) = self.text_at(doc, path, &from, here) {
                let text = text.trim();
                if !text.is_empty() {
                    return Ok(format!("{} {text}", r.name.text()));
                }
            }
        }
        // A pointer-list child borrows the name of the record its offset came
        // from: tensor data called `[7] x_embedder.bias` says which weights
        // these are, where `[7]` says nothing. A record with no name of its
        // own contributes nothing, and the child keeps its index.
        if let Some(name) = self.pointed_from_name(doc, path) {
            return Ok(name);
        }
        let Ty::Struct(s) = r.ty.base() else { return Ok(r.name.text()) };
        let Some(by) = s.named_by.clone() else { return Ok(r.name.text()) };
        // A field that cannot be read yet leaves the node with the name it had.
        let Some(value) = self.naming_value_along(doc, path, &by) else { return Ok(r.name.text()) };
        let text = brief(&value);
        let text = text.trim_end();
        Ok(if text.is_empty() { r.name.text() } else { format!("{} {text}", r.name.text()) })
    }

    /// What field `field` of the structure at `path` says, read as a name: the
    /// value at the end of whatever the format wrapped it in. None when it
    /// cannot be read yet.
    ///
    /// A name a format wraps in something of its own is still the name. Two
    /// wrappers to step through, and a name may be behind both:
    ///
    ///   - a structure whose `contents` field is the whole of it, which is how
    ///     GGUF writes every string as a length and then its bytes;
    ///   - a placement, which is how a format that keeps its names in one
    ///     table writes them. An ELF section header holds an offset into the
    ///     section name table and no name at all, so the name is read with
    ///     `at`, and `at` is a node with the string inside it. Without this
    ///     step `named_by` reached the wrapper, found a composite, and labelled
    ///     the record with the number of things in it.
    pub(super) fn naming_value<S: Source>(&mut self, doc: &Document<S>, path: &[usize], field: usize) -> Option<Value> {
        let mut child = path.to_vec();
        child.push(field);
        self.name_read(doc, child)
    }

    /// The same, for a name that is a path rather than a field: `body.name`
    /// goes into `body`, through whatever it turned out to be, and reads
    /// `name` there. None when some step of the path is not there. See
    /// [`StructDef::named_by`].
    pub(super) fn naming_value_along<S: Source>(&mut self, doc: &Document<S>, path: &[usize], by: &str) -> Option<Value> {
        // The first path that reaches something and reads as something: a
        // ZIP record is named by the file name inside its body, and the
        // records that hold no file by what their signature says they are.
        for alternative in by.split('|') {
            let steps: Vec<String> = alternative.split('.').map(|s| s.trim().to_string()).collect();
            let mut child = path.to_vec();
            if !self.descend(doc, &mut child, &steps).ok()? {
                continue;
            }
            if let Some(v) = self.name_read(doc, child) {
                if !brief(&v).trim().is_empty() {
                    return Some(v);
                }
            }
        }
        None
    }

    /// What the node at `child` reads as when it is read for a name: its own
    /// value where it has one, and where it is a structure, whatever the
    /// structure says names it, and failing that its contents. A wrapper
    /// named by a field of no bytes and holding its name in its contents,
    /// which is what a bencode byte string is, reads as the contents.
    fn name_read<S: Source>(&mut self, doc: &Document<S>, mut child: Vec<usize>) -> Option<Value> {
        let mut info = self.node(doc, &child).ok()?;
        while info.composite {
            let inner = self.memo[&child].ty.base().clone();
            match inner {
                Ty::Struct(s) => {
                    // A structure that says what names it is named by that,
                    // when it reads and says something. The recursion is
                    // bounded the way the file is: a path only goes down.
                    if let Some(by) = s.named_by.clone() {
                        if let Some(v) = self.naming_value_along(doc, &child, &by) {
                            if !brief(&v).trim().is_empty() {
                                return Some(v);
                            }
                        }
                    }
                    let Some(c) = s.contents.clone() else { break };
                    let Some(j) = s.fields.iter().position(|f| *f.name == *c) else { break };
                    child.push(j);
                }
                // One child and nothing of its own: the placement is where the
                // value is, not what it is.
                Ty::At { .. } | Ty::Origin { .. } => child.push(0),
                _ => break,
            }
            let Ok(next) = self.node(doc, &child) else { break };
            info = next;
        }
        Some(info.value)
    }

    /// Where the field at `path` gets its displayed name from, when its
    /// structure says the file holds one. A property of the parent's
    /// declaration, the same as `contents` and `machinery` are.
    ///
    /// An element of a list is asked one level further out: the list is a
    /// field of a structure, and that field says what its elements are called.
    /// See [`crate::template::Field::elem_name_from`]. Either way the
    /// expression is worked out from `path` itself, so for an element `Idx`
    /// is its own index.
    pub(super) fn name_from(&self, path: &[usize]) -> Option<Expr> {
        let (&idx, parent) = path.split_last()?;
        match self.memo.get(parent)?.ty.base() {
            Ty::Struct(s) => s.fields.get(idx)?.name_from.clone(),
            Ty::Array { .. } | Ty::Repeat { .. } | Ty::PointerList { .. } | Ty::Chain { .. } | Ty::Gather { .. } | Ty::Raster { .. } => {
                let (&list, grand) = parent.split_last()?;
                let Ty::Struct(s) = self.memo.get(grand)?.ty.base() else { return None };
                s.fields.get(list)?.elem_name_from.clone()
            }
            _ => None,
        }
    }

    /// Whether the field at `path` is a second reading of bytes something else
    /// describes, and so belongs in no total. A property of the parent's
    /// declaration, the same as `name_from` is. See [`crate::template::Field::aside`].
    pub(super) fn aside(&self, path: &[usize]) -> bool {
        // A node a parse made says it on itself: nothing declared it, so there
        // is no field for the rule below to read. See [`Resolved::aside`].
        if self.memo.get(path).is_some_and(|r| r.aside) {
            return true;
        }
        let Some((&idx, parent)) = path.split_last() else { return false };
        let Some(r) = self.memo.get(parent) else { return false };
        let Ty::Struct(s) = r.ty.base() else { return false };
        // Every field of a union but the first is a second reading of bytes
        // the first already describes, so only the first is counted. Without
        // this a union of four readings of sixteen bytes would be sixty-four
        // bytes as far as any total is concerned. See `StructDef::overlap`.
        if s.overlap && idx > 0 {
            return true;
        }
        s.fields.get(idx).is_some_and(|f| f.aside)
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
    ///
    /// The index is asked about a bit inside the root as well, when the walk
    /// down from the root ends in a structure none of whose fields cover it.
    /// Being inside what the root covers is no promise that walking down finds
    /// everything there: an AppleDouble attribute's value is placed by a field
    /// three levels inside the entry before it, a COFF section's relocations
    /// by a field of the section, and neither the entry nor the section covers
    /// the bytes it points at. The descent stops at whatever does cover them,
    /// and that reads as a gap unless the index has a narrower answer. Asking
    /// adds no walk: a gap already asks the same index where the placed
    /// stretches around it begin and end.
    ///
    /// Several placed stretches can cover one bit, and the narrowest is not
    /// always the one with a field there. An Impulse Tracker module puts its
    /// instruments, its samples and its patterns each in a list reaching over
    /// the whole file, and a module with no instruments has an empty one
    /// first. So each is tried in turn, narrowest first, until one has a field
    /// at the bit, and when none has, the answer is where the first one led.
    pub fn locate<S: Source>(&mut self, doc: &Document<S>, bit: u64) -> R<Vec<usize>> {
        self.locate_under(doc, &[], bit)
    }

    /// The same for the fields under `root`, and a bit of the space those are
    /// counted in: what a stream opened as a tab asks, with its contents read
    /// where the stream was declared. See [`Tab`](super::Tab).
    ///
    /// The index of placed stretches is of the file, so a tab does without it
    /// and a bit its own fields do not cover is its root.
    pub(super) fn locate_under<S: Source>(&mut self, doc: &Document<S>, root: &[usize], bit: u64) -> R<Vec<usize>> {
        self.locate_down(doc, root, bit, false)
    }

    /// The same, stopping at a run of numbers or instructions, or a packed
    /// stream, rather than going on to the one element or block holding the
    /// bit. Which instruction covers a byte takes decoding every one before
    /// it, and which run it is in takes nothing. See [`Evaluator::read_as_under`].
    pub(super) fn locate_run_under<S: Source>(&mut self, doc: &Document<S>, root: &[usize], bit: u64) -> R<Vec<usize>> {
        self.locate_down(doc, root, bit, true)
    }

    fn locate_down<S: Source>(&mut self, doc: &Document<S>, root: &[usize], bit: u64, runs: bool) -> R<Vec<usize>> {
        self.resolve(doc, root)?;
        let size = self.size_of(doc, root)?;
        let top = self.memo[root].clone();
        if bit >= self.len_in(doc, top.space) {
            return fail("past the end of the file");
        }
        let inside = top.offset <= bit && bit < top.offset + size;
        let (found, settled) = if inside { self.walk_down_to(doc, root.to_vec(), bit, runs)? } else { (root.to_vec(), false) };
        if !root.is_empty() {
            return Ok(found);
        }
        // A field the template calls a second reading is not where its bytes
        // belong, so a field placed over the same bit that reads them as what
        // they are is the better answer, even where the walk ended on a field.
        // A ZIP holding a BP5 directory reads each entry as bytes and places
        // the dataset's files over them.
        let second = if inside { self.outermost_aside(&found) } else { None };
        if settled && second.is_none() {
            return Ok(found);
        }
        // Only a stretch narrower than the structure the walk stopped in says
        // more about the bit than that structure does, and a wider one would
        // lead back down to it. Narrower, for a second reading, than the
        // structure holding it.
        let widest = if let Some(holder) = second {
            self.size_of(doc, &holder)?
        } else if inside {
            let at = self.memo[&found].offset;
            let size = self.size_of(doc, &found)?;
            if at <= bit && bit < at + size { size } else { u64::MAX }
        } else {
            u64::MAX
        };
        let mut answer = if inside { Some(found) } else { None };
        for (width, placed) in self.placements_at(doc, bit)?.into_iter().take(PLACEMENTS_TRIED) {
            if width >= widest {
                break;
            }
            let (deeper, settled) = self.walk_down_to(doc, placed, bit, runs)?;
            if settled {
                return Ok(deeper);
            }
            answer.get_or_insert(deeper);
        }
        Ok(answer.unwrap_or_default())
    }

    /// The structure holding the outermost field on `path` that is a second
    /// reading of its bytes, where there is one.
    fn outermost_aside(&self, path: &[usize]) -> Option<Vec<usize>> {
        (1..=path.len()).find(|&k| self.aside(&path[..k])).map(|k| path[..k - 1].to_vec())
    }

    /// Walk down from `path` to the deepest field covering `bit`, and say
    /// whether the walk ended on a field, rather than in a structure or a
    /// list none of whose children cover the bit.
    ///
    /// With `runs`, a run of numbers or instructions, or a packed stream, is
    /// where the walk ends.
    fn walk_down_to<S: Source>(&mut self, doc: &Document<S>, mut path: Vec<usize>, bit: u64, runs: bool) -> R<(Vec<usize>, bool)> {
        loop {
            // The cursor stops at a decoded stream's *contents*: those are at
            // offsets of the decoded bytes, and no bit of the file is any one
            // of them. What the decoder read to produce them is bits of the
            // file, though, and is what the cursor lands on: a bit of the run
            // is a bit of some block's header, or of its tables, or of one
            // literal. For a codec whose trace has no blocks the run is still
            // the answer, whole.
            self.resolve(doc, &path)?;
            if runs && self.not_text(&path).is_some() {
                return Ok((path, true));
            }
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
                return Ok((path, true));
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
                    Ok(None) => return Ok((path, false)),
                    Err(e) if e.interrupted() => return Err(e),
                    // The run ended on something that would not parse, which
                    // is what the bytes after the last whole element of a run
                    // look like. Counting the run would have stopped there
                    // too, and the answer for a bit past the end of it is the
                    // run itself, which reads as a gap.
                    Err(_) => return Ok((path, false)),
                }
            };
            if n == 0 {
                // A field with nothing inside it is the answer. A structure or
                // a list with nothing inside it is bytes nothing describes.
                let holds = holds_fields(&self.memo[&path].ty);
                return Ok((path, !holds));
            }
            match self.child_at(doc, &path, n, bit)? {
                Some(i) => path.push(i),
                // Inside the parent but in none of its children: padding, or a
                // struct whose fields do not fill it.
                None => return Ok((path, false)),
            }
        }
    }

    /// The outermost enclosing structure that reads as one row, or the field
    /// itself when nothing on the way to it is marked that way. `locate` has
    /// already resolved every step, so this only reads what it left behind.
    pub(super) fn inline_ancestor(&self, root: &[usize], path: &[usize]) -> Vec<usize> {
        for n in root.len()..path.len() {
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
        self.spans_under(doc, &[], from, to, max)
    }

    /// The same for the fields under `root`, across a stretch of the space
    /// those are counted in. See [`Evaluator::locate_under`].
    pub(super) fn spans_under<S: Source>(&mut self, doc: &Document<S>, root: &[usize], from: u64, to: u64, max: usize) -> R<Vec<Span>> {
        self.resolve(doc, root)?;
        let root_size = self.size_of(doc, root)?;
        let (root_offset, space) = (self.memo[root].offset, self.memo[root].space);
        // As far as the file goes rather than as far as the root's own fields
        // go: what a placed field covers is past the second and inside the
        // first, and it is most of an HDF5 file.
        let _ = root_size;
        let end = to.min(self.len_in(doc, space));
        let mut at = from.max(root_offset);
        let mut out: Vec<Span> = Vec::new();
        while at < end && out.len() < max {
            let path = self.locate_under(doc, root, at)?;
            // A structure marked to read on one row stands for its fields here.
            let path = self.inline_ancestor(root, &path);
            // Everything a decoder's trace laid down stands for the block it
            // belongs to. The cursor goes all the way to one literal, because
            // a reader who clicks a byte of a deflate stream is asking which
            // symbol it is; the column does not, because a screenful of them
            // is three hundred entries reading `9 bits` and none of them says
            // what the stream is doing.
            let path = self.traced_block(&path);
            let inline = matches!(self.memo[&path].ty.base(), Ty::Struct(s) if s.inline);
            let info = self.node(doc, &path)?;
            let span = if at < info.offset_bits || at >= info.offset_bits + info.size_bits {
                self.gap_before_the_next_placement(doc, root, &path, &info, at)?
            } else if inline {
                // A structure that reads on one row is still one of many when
                // it is an element of a long run: a quantised tensor is
                // thousands of packed blocks, and the run of them says more
                // than a row per block, and gives the value table its cells.
                match self.run_of(doc, &path, at)? {
                    Some(run) => run,
                    None => self.one_row(doc, &path, &info)?,
                }
            } else if matches!(self.memo[&path].ty, Ty::Decoded { .. }) {
                // The whole run, as one entry that says what it is and how
                // many fields are inside. Before `composite`: its one child is
                // at offset 0 of the decoded bytes, and treating that as a bit
                // of the file would put the stream's contents at the front of
                // it.
                self.decoded_run(doc, &path, &info, at)?
            } else if matches!(self.memo[&path].ty, Ty::Traced { part: TracedPart::Block(_) }) {
                self.traced_block_span(doc, &path, &info)?
            } else if info.composite {
                self.gap_inside(doc, root, &path, &info, at)?
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
        root: &[usize],
        path: &[usize],
        info: &NodeInfo,
        at: u64,
    ) -> R<Span> {
        let mut span = self.span_of(doc, path, info)?;
        // The gap is the whole stretch between what ends before `at` and what
        // begins after it, not the part of it from `at` on: `at` is wherever
        // the view happens to start, and a gap that began at the top of the
        // screen would shrink as the view scrolled down through it.
        let root_end = self.memo[root].offset + self.size_of(doc, root)?;
        let mut begins = if root_end <= at { root_end } else { 0 };
        if let Some(end) = self.placement_end_before(doc, root, at)? {
            begins = begins.max(end);
        }
        let len = self.len_in(doc, self.memo[root].space);
        let mut ends = self.placement_after(doc, root, at)?.unwrap_or(len);
        let (before, after) = self.scattered_around(doc, root, at)?;
        if let Some(end) = before {
            begins = begins.max(end);
        }
        if let Some(next) = after {
            ends = ends.min(next);
        }
        let ends = ends.max(at + 8);
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
    fn gap_inside<S: Source>(&mut self, doc: &Document<S>, root: &[usize], path: &[usize], info: &NodeInfo, at: u64) -> R<Span> {
        let mut span = self.span_of(doc, path, info)?;
        // From where the last thing before `at` ends, not from `at` itself,
        // which is only where the view starts: see `gap_before_the_next_placement`.
        let mut begins = info.offset_bits;
        if let Some(end) = self.prev_child_end(doc, path, at)? {
            begins = begins.max(end);
        }
        if let Some(end) = self.placement_end_before(doc, root, at)? {
            begins = begins.max(end);
        }
        // A list placed over the whole file begins where the file does, and
        // the root's own fields are not part of a gap past them.
        let root_end = self.memo[root].offset + self.size_of(doc, root)?;
        if root_end <= at {
            begins = begins.max(root_end);
        }
        let mut ends = info.offset_bits + info.size_bits;
        for k in (root.len()..=path.len()).rev() {
            if let Some(next) = self.next_child_start(doc, &path[..k], at)? {
                ends = ends.min(next);
            }
        }
        if let Some(next) = self.placement_after(doc, root, at)? {
            ends = ends.min(next);
        }
        let (before, after) = self.scattered_around(doc, root, at)?;
        if let Some(end) = before {
            begins = begins.max(end);
        }
        if let Some(next) = after {
            ends = ends.min(next);
        }
        // A framed structure's leftovers are its own punctuation rather than
        // bytes nothing describes: the braces of a JSON object are the object,
        // and the column says so instead of calling them unmapped.
        span.gap = !info.framed;
        span.offset_bits = begins;
        span.size_bits = ends - begins;
        span.count = 0;
        // A run that stopped short says why: the stretch after its last
        // element is not bytes nobody described but bytes the template could
        // not read, and the gap carries the reason.
        span.value = match self.list(path).repeat_trouble.clone() {
            Some(why) if begins >= self.list(path).repeat_end.unwrap_or(0) => Value::Str(why),
            _ => Value::Str(String::new()),
        };
        Ok(span)
    }

    /// Where the nearest element before `at` ends and the nearest after it
    /// begins, among the lists of scattered elements a placed stretch over
    /// `at` holds.
    ///
    /// A gap is bounded by the structure it was found in and by the placed
    /// stretches around it, and neither of those sees a list that is placed
    /// over the gap without being the one `locate` found. An Impulse Tracker
    /// module's instrument list, sample list and pattern list each reach over
    /// the whole file: the bytes between the last instrument and the first
    /// sample are in the instrument list and in none of its elements, and
    /// ending that gap where the next placed stretch begins ran it over every
    /// sample header to the first sample's data. Only lists that know where
    /// their elements start in order are asked, which is a halving each.
    fn scattered_around<S: Source>(&mut self, doc: &Document<S>, root: &[usize], at: u64) -> R<(Option<u64>, Option<u64>)> {
        let mut before: Option<u64> = None;
        let mut after: Option<u64> = None;
        if !root.is_empty() {
            return Ok((before, after));
        }
        for (_, mut placed) in self.placements_at(doc, at)?.into_iter().take(PLACEMENTS_TRIED) {
            self.resolve(doc, &placed)?;
            if matches!(self.memo[&placed].ty, Ty::At { .. }) {
                placed.push(0);
                self.resolve(doc, &placed)?;
            }
            if !matches!(self.memo[&placed].ty, Ty::PointerList { .. } | Ty::Chain { .. } | Ty::Gather { .. }) {
                continue;
            }
            if let Some(end) = self.prev_child_end(doc, &placed, at)? {
                before = Some(before.map_or(end, |b| b.max(end)));
            }
            if let Some(next) = self.next_child_start(doc, &placed, at)? {
                after = Some(after.map_or(next, |a| a.min(next)));
            }
        }
        Ok((before, after))
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

    /// The block of a decoder's trace this path is inside, or the path itself
    /// when it is not inside one. The outermost such node, since a trace is
    /// one level of blocks and the rest is what is in them.
    fn traced_block(&self, path: &[usize]) -> Vec<usize> {
        for k in 0..path.len() {
            if matches!(self.memo.get(&path[..k + 1]).map(|r| &r.ty), Some(Ty::Traced { part: TracedPart::Block(_) })) {
                return path[..k + 1].to_vec();
            }
        }
        path.to_vec()
    }

    /// One block of a decoded stream, as the one entry it is: what coded it,
    /// and how many codes it holds.
    ///
    /// The whole block rather than its parts. A dynamic block's header is five
    /// fields and then three hundred code lengths, and its payload is tens of
    /// thousands of codes; every one of those is worth a row to a reader who
    /// opens the block, and none of them is worth a chip beside the bytes.
    fn traced_block_span<S: Source>(&mut self, doc: &Document<S>, path: &[usize], info: &NodeInfo) -> R<Span> {
        // A stored block codes nothing: its one step is the bytes copied
        // through, and counting it says less than the block's size does. So
        // no count for those, and the chip falls back to the size.
        // A JPEG MCU is counted in the 8×8 blocks it holds, which is what it is
        // made of; its codes are one level further in.
        let (symbols, unit) = match (self.trace_for(path), &self.memo[path].ty) {
            (Some((_, trace)), Ty::Traced { part: TracedPart::Block(i) }) if !trace.units().is_empty() => {
                (trace.units_of(*i as usize).len() as u64, "block")
            }
            (Some((_, trace)), Ty::Traced { part: TracedPart::Block(i) }) => (
                super::traced::BlockView::of(trace, *i)
                    .filter(|v| !matches!(v.block.kind, crate::codec::BlockKind::Stored | crate::codec::BlockKind::Scanline))
                    .map_or(0, |v| v.symbols.len() as u64),
                "code",
            ),
            _ => (0, "code"),
        };
        let mut span = self.span_of(doc, path, info)?;
        span.count = symbols;
        span.unit = (symbols > 0).then(|| unit.to_string());
        // The block's own value is its number in the stream, which beside the
        // bytes reads as a count of something. The size says more.
        if symbols == 0 {
            span.value = Value::Str(String::new());
        }
        Ok(span)
    }

    /// A structure marked to read on one row, as the one row it reads as.
    fn one_row<S: Source>(&mut self, doc: &Document<S>, path: &[usize], info: &NodeInfo) -> R<Span> {
        let mut span = self.span_of(doc, path, info)?;
        // What the format says one of these reads as, where it has said. The
        // same declaration the value table's cells are written from, so the
        // chip beside the bytes and the cell over them say the same thing
        // about the same record.
        let line = match self.struct_of(&self.memo[path].ty.clone()) {
            Some(def) if !def.line.is_empty() => Some(def.line.clone()),
            _ => None,
        };
        span.line = Some(match line {
            Some(line) => self.record_line(doc, path, &line)?,
            None => {
                let mut parts = Vec::new();
                self.one_line(doc, path, &mut parts)?;
                parts.join(" ")
            }
        });
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
        match self.run_of(doc, path, at)? {
            Some(run) => Ok(run),
            None => self.span_of(doc, path, info),
        }
    }

    /// The long run of values or records a field belongs to, as one entry, or
    /// None when it belongs to none that is worth folding.
    fn run_of<S: Source>(&mut self, doc: &Document<S>, path: &[usize], at: u64) -> R<Option<Span>> {
        let Some((run, count)) = self.collapsible(doc, path)? else { return Ok(None) };
        let run_info = self.node(doc, &run)?;
        // Pointer-heavy formats can reach a leaf through an overlapping
        // placement whose repeated ancestor has already ended. Collapsing that
        // ancestor would return a span behind `at` forever. Only substitute the
        // run when it covers the byte this request is actually advancing from.
        if run_info.offset_bits > at || at >= run_info.offset_bits + run_info.size_bits {
            return Ok(None);
        }
        let mut span = self.span_of(doc, &run, &run_info)?;
        span.count = count;
        // What the format calls one of these, so a run of records counts in its
        // own word rather than in `value`, which is what a run of numbers is
        // made of and a run of packets is not.
        let ty = self.memo[&run].ty.clone();
        span.unit = self.unit_of(&run, &ty).map(str::to_string);
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
        Ok(Some(span))
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
            // Nor does a field that reads its contents somewhere else: the one
            // thing it points at keeps its name, so the step is that name
            // twice over. See the `Ty::At` arm of `place_child`.
            if matches!(self.memo[&path[..k]].ty, Ty::At { .. }) {
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
            unit: None,
            line: None,
            sample: Vec::new(),
            parts: Vec::new(),
            bits: self.bit_roles(doc, path, info),
            inline: matches!(self.memo[path].ty.base(), Ty::Struct(s) if s.inline),
            problem: info.problem.clone(),
            problems_within: info.problems_within,
            // A stream that would not open is not an offer. `space_root` is
            // the node the stream holds, which is what the listing hangs Open
            // unpacked off; a template may fold the stream itself away and
            // show only its contents, so both spellings have to be caught.
            opens: (info.decoded && info.refused.is_none()) || info.space_root,
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

    /// What a small structure reads as on one line, for [`NodeInfo::line`].
    ///
    /// Only a structure of a few fields. The walk goes through every leaf
    /// under the node, and a panel listing a header's three hundred fields
    /// asks this of every row it draws: a line for a record that long would
    /// be unreadable anyway, so the count it falls back to is both cheaper and
    /// better. A list is left out for the same reason and one more: its
    /// elements are a table, and `one_line` answers for one with a count.
    ///
    /// A reading that cannot be had yet is no reading rather than an error.
    /// The panel is drawn again as the bytes land, and a structure whose
    /// values are still coming has the count to show in the meantime.
    pub(super) fn node_line<S: Source>(
        &mut self,
        doc: &Document<S>,
        path: &[usize],
        worth_a_line: bool,
        child_count: u64,
    ) -> R<Option<String>> {
        const FIELDS: u64 = 8;
        if self.lining || !worth_a_line || child_count == 0 {
            return Ok(None);
        }
        // A structure that says what it reads as reads only the fields it
        // names, so how many fields it has costs nothing: an ELF section
        // header is ten fields and a line of three.
        let declared = self.struct_of(&self.memo[path].ty.clone()).is_some_and(|d| !d.line.is_empty());
        if !declared && child_count > FIELDS {
            return Ok(None);
        }
        match self.record_reading(doc, path) {
            Ok(said) => Ok(Some(said).filter(|s| !s.is_empty())),
            Err(e) if e.interrupted() => Err(e),
            Err(_) => Ok(None),
        }
    }

    /// What a node reads as on one line: the line its structure declares, or
    /// else its fields in order.
    ///
    /// A structure drawn on one row reads as its values alone, the way an
    /// instruction reads as its operands. Any other structure with no line of
    /// its own says which field each number is: a MIDI note is `note 72 ·
    /// velocity 0`, and `72 0` is two numbers the reader has to count off
    /// against the field tree.
    pub(super) fn record_reading<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<String> {
        // Past this many parts a line is a list of the record's fields, and
        // the field tree already is one.
        const PARTS: usize = 6;
        let def = self.struct_of(&self.memo[path].ty.clone());
        let labels = def.as_ref().is_some_and(|d| d.line.is_empty() && !d.inline);
        let mut said = Vec::new();
        let was = std::mem::replace(&mut self.lining, true);
        let got = self.one_line_walk(doc, path, labels, &mut said);
        self.lining = was;
        got?;
        if !labels {
            return Ok(said.join(" "));
        }
        if said.len() > PARTS {
            said.truncate(PARTS);
            said.push("\u{2026}".to_string());
        }
        Ok(said.join(" \u{b7} "))
    }

    /// A structure that reads on one row, as its fields' values in order. A
    /// field that is itself a structure contributes its own fields, so a wasm
    /// instruction whose immediate has two parts still reads as one line.
    pub(super) fn one_line<S: Source>(&mut self, doc: &Document<S>, path: &[usize], out: &mut Vec<String>) -> R<()> {
        // A line is a walk through every leaf under the node, and each step of
        // it asks for a node. Working out a line for each of those as well
        // would read the same leaves once per level of nesting, so the walk
        // says it is under way and the nodes it asks for come back without one.
        let was = std::mem::replace(&mut self.lining, true);
        let got = self.one_line_walk(doc, path, false, out);
        self.lining = was;
        got
    }

    /// The walk under [`Self::one_line`]. With `labels`, a part that does
    /// not say what it is on its own goes after its field's name: a number,
    /// a count, a run of bytes. A named value, text and a magic number say
    /// what they are already.
    fn one_line_walk<S: Source>(&mut self, doc: &Document<S>, path: &[usize], labels: bool, out: &mut Vec<String>) -> R<()> {
        let info = self.node(doc, path)?;
        // A structure drawn on one row inside a record still reads as its
        // values alone.
        let labels = labels && !(info.composite && self.struct_of(&self.memo[path].ty.clone()).is_some_and(|d| d.inline));
        let named = |text: String| if labels { format!("{} {text}", info.name) } else { text };
        if !info.composite {
            // A field of no bits is an absence, not an empty value: the switch
            // for an opcode with no immediate selects one.
            if info.size_bits > 0 {
                // Raw bytes read as how many there are. A JPEG scan is two
                // hundred kilobytes of entropy-coded data, and the first
                // sixteen of them written out in hex say nothing that the
                // sixteen in the column to the left do not.
                let text = match &info.value {
                    Value::Bytes { .. } | Value::Unread { .. } => byte_text(info.size_bits / 8),
                    v => brief(v),
                };
                let says_itself = matches!(&info.value, Value::Enum { name: Some(_), .. } | Value::Str(_) | Value::Magic { .. });
                if !text.is_empty() {
                    out.push(if says_itself { text } else { named(text) });
                }
            }
            return Ok(());
        }
        // A list on a line says how many it holds. A quantisation table is
        // sixty-four numbers, and sixty-four numbers written across a chip is
        // not a reading of the table: it is the table with the reader left to
        // do the work. The field tree opens it for anyone who wants them.
        let ty = self.memo[path].ty.clone();
        // A stream reads as how many bytes it is. Asking what is inside it
        // would unpack it, and the line for the record a stream sits in is
        // asked for every row the annotation column draws: a reader scrolling
        // past a file of streams would unpack the file. The same answer raw
        // bytes give, and for the same reason.
        if matches!(ty, Ty::Decoded { .. } | Ty::Stitched { .. }) {
            if info.size_bits > 0 {
                out.push(named(byte_text(info.size_bits / 8)));
            }
            return Ok(());
        }
        // A field read somewhere else says where before it says what. Without
        // the address the line hands back a value from the far side of the
        // file with nothing to say it did not come from the bytes the line is
        // drawn over, and the offset beside it, which is the only clue, is
        // exactly the field a reading folds away as machinery.
        if matches!(ty, Ty::At { .. }) && info.child_count == 1 {
            let mut child = path.to_vec();
            child.push(0);
            let there = self.node(doc, &child)?;
            let said = self.record_reading(doc, &child)?;
            let at = address_text(there.offset_bits, there.space);
            let reading = far_end(&there, said);
            out.push(named(if reading.is_empty() { at } else { format!("{at} · {reading}") }));
            return Ok(());
        }
        if matches!(
            ty.base(),
            Ty::Array { .. } | Ty::Repeat { .. } | Ty::PointerList { .. } | Ty::Chain { .. } | Ty::Gather { .. } | Ty::Raster { .. }
        ) {
            let unit = self.unit_of(path, &ty).unwrap_or("value").to_string();
            out.push(named(count_text(info.child_count, &unit)));
            return Ok(());
        }
        // A structure that has said what it reads as says it here too, so a
        // field naming another structure gets that structure's reading rather
        // than a walk through its leaves.
        if let Some(def) = self.struct_of(&ty) {
            if !def.line.is_empty() {
                let said = self.record_line(doc, path, &def.line.clone())?;
                if !said.is_empty() {
                    out.push(said);
                }
                return Ok(());
            }
        }
        // A field that exists to say how long or how many another field is has
        // nothing to say on a line beside the field it measures: every JPEG
        // segment's reading opened with its own length, which is the extent
        // the row is already drawn at. A field that picks a shape is left
        // alone, since that is usually the word the record is about, and a
        // format that wants a measurement back on the line says so with
        // `payload`.
        let quiet: Vec<bool> = match self.struct_of(&ty) {
            Some(def) => {
                let m = crate::machinery::measurers(&def);
                // A line that names its fields also leaves out the ones the
                // structure calls its own plumbing: a b-tree page's free-block
                // offsets are numbers, and a label does not make them worth
                // reading.
                (0..def.fields.len())
                    .map(|i| {
                        let hint = crate::machinery::hint(&def, i);
                        (m[i].is_some() && hint != Some(false)) || (labels && hint == Some(true))
                    })
                    .collect()
            }
            None => Vec::new(),
        };
        for i in 0..info.child_count as usize {
            if quiet.get(i) == Some(&true) {
                continue;
            }
            let mut child = path.to_vec();
            child.push(i);
            self.one_line_walk(doc, &child, labels, out)?;
        }
        Ok(())
    }

    /// The structure a type is, following the names in the template's own
    /// table. None for anything that is not one.
    fn struct_of(&self, ty: &Ty) -> Option<std::sync::Arc<crate::template::StructDef>> {
        let mut ty = ty.base();
        for _ in 0..8 {
            match ty {
                Ty::Struct(s) => return Some(s.clone()),
                Ty::Named(n) => ty = self.template.types.get(&**n)?.base(),
                _ => return None,
            }
        }
        None
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
            P::Block(i) if !trace.units().is_empty() => {
                let view = super::traced::UnitsView::of(trace, i)?;
                let block = trace.blocks().get(i as usize)?;
                if !block.in_bits.contains(&want) {
                    return None;
                }
                view.index_of_step(trace, trace.index_in(want)? as u32)
            }
            P::Unit(j) => {
                let unit = trace.units().get(j as usize)?;
                let k = trace.index_in(want)? as u32;
                unit.steps.contains(&k).then(|| (k - unit.steps.start) as usize)
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
        // A raster knows which pixel holds a bit the same way it knows where
        // a pixel is, by arithmetic, whatever order it keeps them in.
        if let Ty::Raster { .. } = &r.ty {
            return Ok(self.raster_at(doc, path, bit)?.filter(|&i| (i as u64) < n));
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
        // which is not the order they are numbered in. So are a gather's, once
        // its walk has reached every record.
        if matches!(r.ty, Ty::Chain { .. } | Ty::PointerList { .. } | Ty::Gather { .. }) {
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
        let scattered = matches!(r.ty, Ty::PointerList { .. } | Ty::Chain { .. } | Ty::Gather { .. })
            || self.has_pointing_field(&r.ty)
            || self.has_low_bit_first_field(&r.ty);
        // A field that is a second reading of its bytes is asked only when no
        // other field covers the bit. A SQLite freelist is read from the
        // header before the pages, and the trunk it reaches is also a page of
        // the run declared after it: the page is where those bytes belong. And
        // a second reading may be a list the length of the file, which a bit
        // that some page holds then never has to ask.
        let later: Vec<usize> = match r.ty.base() {
            Ty::Struct(s) => s.fields.iter().enumerate().filter(|(_, f)| f.aside).map(|(i, _)| i).collect(),
            _ => Vec::new(),
        };
        let mut p = path.to_vec();
        for i in (0..n as usize).filter(|i| !later.contains(i)) {
            match self.child_covers(doc, &mut p, i, bit, scattered)? {
                Cover::Yes => return Ok(Some(i)),
                Cover::Past => break,
                Cover::No => {}
            }
        }
        for i in later {
            if (i as u64) < n && matches!(self.child_covers(doc, &mut p, i, bit, scattered)?, Cover::Yes) {
                return Ok(Some(i));
            }
        }
        Ok(None)
    }

    /// Whether child `i` of the node at `p` covers `bit`, for [`Self::child_at`]
    /// looking at the children one at a time. `p` is the node's path and is
    /// left as it was found.
    ///
    /// `Past` is a child that starts after the bit, in a node whose children
    /// are in the order they sit in, so none of the rest can cover it either.
    fn child_covers<S: Source>(&mut self, doc: &Document<S>, p: &mut Vec<usize>, i: usize, bit: u64, scattered: bool) -> R<Cover> {
        p.push(i);
        // A chain covers no bytes where it is declared, so asking how long
        // it is says nothing about whether the bit is inside it: what
        // covers bytes is the elements the walk found, wherever they are.
        // Asking the chain itself is the same halving `child_at` does for
        // any list of scattered children.
        // A gather with no region of its own is the same. One a `Sized`
        // made a region covers its bytes, and is asked the ordinary way.
        let chain = match self.resolve(doc, p) {
            Ok(()) => {
                let r = &self.memo[p.as_slice()];
                matches!(r.ty, Ty::Chain { .. }) || (matches!(r.ty, Ty::Gather { .. }) && r.declared_size.is_none())
            }
            Err(e) if scattered && !e.interrupted() => {
                p.pop();
                return Ok(Cover::No);
            }
            Err(e) => {
                p.pop();
                return Err(e);
            }
        };
        if chain {
            let inside = match self.child_count(doc, p) {
                Ok(count) => self.child_at(doc, p, count, bit),
                Err(e) => Err(e),
            };
            p.pop();
            return match inside {
                Ok(Some(_)) => Ok(Cover::Yes),
                Err(e) if e.interrupted() => Err(e),
                _ => Ok(Cover::No),
            };
        }
        let mut elsewhere = false;
        let placed = match self.resolve(doc, p) {
            Ok(()) => match self.memo[p.as_slice()].ty {
                // The field covers nothing where it is declared; what it
                // points at is what the cursor can be inside of.
                Ty::At { .. } => {
                    elsewhere = true;
                    p.push(0);
                    let inner = match self.resolve(doc, p) {
                        Ok(()) => self.size_of(doc, p).map(|size| (self.memo[p.as_slice()].offset, size)),
                        Err(e) => Err(e),
                    };
                    p.pop();
                    inner
                }
                _ => self.size_of(doc, p).map(|size| (self.memo[p.as_slice()].offset, size)),
            },
            Err(e) => Err(e),
        };
        p.pop();
        let (off, size) = match placed {
            Ok(v) => v,
            Err(e) if scattered && !e.interrupted() => return Ok(Cover::No),
            Err(e) => return Err(e),
        };
        // A child placed somewhere else says nothing about where the
        // children declared after it are. `scattered` sees an `At` only
        // when it is the declared field, and a template that follows an
        // offset only when it is set writes a switch round it: a COFF
        // object's symbol table is one, placed past the section data that
        // is declared after it, and stopping there left every section's
        // bytes reading as a gap.
        if bit < off && !scattered && !elsewhere {
            return Ok(Cover::Past);
        }
        Ok(if bit >= off && bit < off + size { Cover::Yes } else { Cover::No })
    }

    /// Whether a structure has a field whose contents are somewhere else in
    /// the file, which is what stops its children from being in order.
    fn has_pointing_field(&self, ty: &Ty) -> bool {
        matches!(ty.base(), Ty::Struct(s) if s.fields.iter().any(|f| matches!(f.ty, Ty::At { .. } | Ty::Chain { .. } | Ty::Gather { .. })))
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
        if matches!(self.memo[path].ty, Ty::PointerList { .. } | Ty::Chain { .. } | Ty::Gather { .. }) {
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
        let last: Option<usize> = if matches!(self.memo[path].ty, Ty::PointerList { .. } | Ty::Chain { .. } | Ty::Gather { .. }) {
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

/// What [`Evaluator::child_covers`] found out about one child.
enum Cover {
    Yes,
    No,
    /// Starts after the bit, among children in the order they sit in.
    Past,
}
