//! One walk over a file for everything the generic report counts.
//!
//! The report needs four things the core did not compute: the format profile
//! of the file (`profile.rs`), the byte ledger (`ledger.rs`), the extent audit
//! (`extent.rs`) and the directories (`directory.rs`). The first three are
//! counts over every field of the file, and the walk that visits every field
//! exactly once already exists: the kind totals' (`kinds.rs`). So this follows
//! that walk (see `watch.rs`) and keeps all three counts at once, rather than
//! walking a large file three times, or four with the kind totals.
//!
//! After the walk come two stages that read more: the gaps and padding the
//! ledger found are read for their zero bytes, and the directories are found
//! and measured. Each goes a step at a time on the same allowance.
//!
//! Like the kind totals, the walk is kept by the caller between goes and
//! thrown away when the document changes.

use rustc_hash::FxHashMap;

use super::directory::{Build, Directories};
use super::extent::{ExtentAudit, ExtentCheck, Tally as Audit};
use super::ledger::{without_index, Ledger, Tally as Books};
use super::origin::Role;
use super::profile::{at_kind, from_end, sizing_kind, value_keys, Key, Profile, Tally as Counts};
use super::space::Opened;
use super::watch::{Closing, Watch};
use super::*;

/// How large a stream the profile unpacks to say what it comes to, and how
/// much it unpacks in all. Past these a codec's row says how many streams it
/// did not open. A stream the listing or a tab has already opened is counted
/// whatever its size.
const UNPACK_ONE: u64 = 1024 * 1024 * 8;
const UNPACK_ALL: u64 = 4 * 1024 * 1024 * 8;

/// The walk, kept between goes.
pub struct ReportWalk {
    kinds: KindWalk,
    follow: Follow,
    file_bits: u64,
    root: Vec<usize>,
    /// Set once every stage has finished.
    done: bool,
}

impl ReportWalk {
    pub fn new(file_bits: u64) -> ReportWalk {
        ReportWalk::under(file_bits, Vec::new())
    }

    /// A walk over the fields under `root`, which hold `file_bits` bits of the
    /// space they are counted in.
    pub fn under(file_bits: u64, root: Vec<usize>) -> ReportWalk {
        ReportWalk {
            kinds: KindWalk::under(file_bits, root.clone()),
            follow: Follow::new(file_bits),
            file_bits,
            root,
            done: false,
        }
    }

    pub fn file_bits(&self) -> u64 {
        self.file_bits
    }

    pub fn done(&self) -> bool {
        self.done
    }

    /// The file's profile so far. `done` once the walk is over.
    pub fn profile(&self) -> Profile {
        let mut counts = self.follow.counts.clone();
        let f = &mut counts.facts;
        f.every_field_follows = Some(f.backward == 0 && f.from_end == 0 && f.lengths_after == 0);
        counts.profile(self.walked())
    }

    /// The ledger so far. `done` once the walk is over and every gap has been
    /// read.
    pub fn ledger(&self) -> Ledger {
        self.follow.books.ledger(self.file_bits, self.walked() && self.follow.books.scanned())
    }

    /// The extent audit so far. `done` once the walk is over.
    pub fn audit(&self) -> ExtentAudit {
        self.follow.audit.audit(self.walked())
    }

    /// The directories. Found only once the walk is over, since the lists
    /// of offsets are found on it; empty and not `done` until then.
    pub fn directories(&self) -> Directories {
        self.follow.build.directories()
    }

    /// How far into the file the walk has reached.
    pub fn reached_bits(&self) -> u64 {
        self.kinds.totals().reached_bits
    }

    fn walked(&self) -> bool {
        self.kinds.done() || self.follow.audit.root_failed.is_some()
    }
}

impl Evaluator {
    /// Carry the report's walk on for one go. The go ends when the allowance
    /// runs out, the way `kind_totals_step` does; `done` on the walk says when
    /// to stop asking. Bytes that have not arrived come back as `Pending`.
    pub fn report_step<S: Source>(&mut self, doc: &Document<S>, walk: &mut ReportWalk) -> R<()> {
        match self.report_run(doc, walk) {
            Ok(()) | Err(EvalError::Busy { .. }) => Ok(()),
            Err(e) => Err(e),
        }
    }

    fn report_run<S: Source>(&mut self, doc: &Document<S>, walk: &mut ReportWalk) -> R<()> {
        if walk.done {
            return Ok(());
        }
        if !walk.walked() {
            match self.kind_totals_run(doc, &mut walk.kinds, &mut walk.follow) {
                Ok(()) => {}
                // The root would not read, so there is nothing to walk. What
                // the audit can still say is where it failed.
                Err(EvalError::Failed(why)) if !walk.follow.started => {
                    let check = self.root_failure(doc, &walk.root, &why)?;
                    if let Some(c) = check {
                        walk.follow.audit.add(c, 1);
                    }
                    walk.follow.audit.root_failed = Some(why);
                    walk.follow.audit.space_bits = walk.file_bits;
                }
                Err(e) => return Err(e),
            }
        }
        if walk.follow.started {
            let space = walk.follow.space;
            walk.follow.books.scan(self, doc, space)?;
        }
        walk.follow.build.step(self, doc, &walk.root)?;
        walk.done = true;
        Ok(())
    }

    /// Where a root that would not read failed: the first of its fields that
    /// would not, looked into until the one that did not fit.
    fn root_failure<S: Source>(&mut self, doc: &Document<S>, root: &[usize], why: &str) -> R<Option<ExtentCheck>> {
        match self.resolve(doc, root) {
            Err(e) if e.interrupted() => return Err(e),
            Err(_) => return Ok(None),
            Ok(()) => {}
        }
        let fields = match self.memo[root].ty.base() {
            Ty::Struct(s) if !s.overlap => s.fields.len(),
            _ => return Ok(None),
        };
        let mut at = self.memo[root].offset;
        for k in 0..fields {
            let mut g = root.to_vec();
            g.push(k);
            match self.resolve(doc, &g).and_then(|()| self.size_of(doc, &g)) {
                Ok(size) => at = self.memo[&g].cursor + size,
                Err(e) if e.interrupted() => return Err(e),
                Err(EvalError::Failed(w)) => return self.diagnose(doc, root, k, at, &w, 0),
                Err(e) => return Err(e),
            }
        }
        let _ = why;
        Ok(None)
    }
}

/// How the elements of a list get their group.
#[derive(Debug, Clone)]
enum Elements {
    /// Each element is a choice: the case it took.
    Choice(Ty),
    /// Each element is a structure whose field `k` is a choice: the case
    /// that field took.
    Field(usize, Ty),
    /// Every element is the same type: one group.
    Fixed(u32),
    /// The elements are plain values, and stay in the list's own group.
    Inherit,
}

/// A frame of the walk, as the report sees it.
#[derive(Debug)]
struct Ctx {
    path: Vec<usize>,
    part: u32,
    group: u32,
    /// Whether everything inside is machinery.
    machinery: bool,
    /// When the walk opened it. See `ledger::Placed`.
    open: u64,
    /// Its entry among the fields placed by an offset, for one that was.
    placed: Option<usize>,
    /// The extent check on this frame, settled when it closes.
    check: Option<ExtentCheck>,
    elements: Option<Elements>,
    /// The first child that would not read.
    failed_at: Option<u64>,
    /// The check on the element a run could not read, found as it closed.
    trouble: Option<ExtentCheck>,
    /// Where it starts and ends, and the furthest its children reached
    /// inside that, those placed by an offset included.
    offset: u64,
    end: u64,
    reach: u64,
}

/// What `ready` worked out about one node, for `leaf` or `open` to count.
#[derive(Debug, Default)]
struct Ready {
    path: Vec<usize>,
    part: u32,
    group: u32,
    /// Whether its own bits are machinery, and whether everything inside it
    /// is.
    machinery: bool,
    inherit: bool,
    /// How it was placed, for the profile, and whether an offset read from
    /// the file placed it.
    placement: Option<&'static str>,
    by_offset: bool,
    forward: Option<bool>,
    from_end: bool,
    sizing: Option<&'static str>,
    checks: Vec<&'static str>,
    choice: Option<(String, String, u64, Option<usize>)>,
    unpacked: Option<u64>,
    check: Option<ExtentCheck>,
    /// Whether the length giving this part's size comes after it.
    length_after: Option<bool>,
    elements: Option<Elements>,
    absent: bool,
}

/// What a structure's declaration says about its fields, worked out once per
/// structure rather than once per record.
#[derive(Debug)]
struct Fields {
    /// Whether each field is machinery: another field's shape depends on it,
    /// or the template says so.
    machinery: Vec<bool>,
    /// Whether the template says so, which makes everything inside the field
    /// machinery too. A structure another field depends on only by its size
    /// is not: a WAV chunk's body settles where its padding goes, and its
    /// samples are not plumbing.
    hinted: Vec<bool>,
    /// For each field, the sibling that gives its length or count.
    measured_by: Vec<Option<usize>>,
}

/// Everything the report keeps while it follows the walk.
struct Follow {
    space: u32,
    started: bool,
    stack: Vec<Ctx>,
    pending: Option<Ready>,
    /// The root's part and group, for a root with no frame of its own.
    root_part: (u32, u32),
    /// The structures whose fields are the file's top-level parts: the root,
    /// and the one field of it that holds nearly all of the file, when there
    /// is one, and so on down. An ELF file is seven bytes of identification
    /// and one header structure holding everything else, and the parts a
    /// reader means are that header's fields: its tables and its sections.
    part_chain: Vec<Vec<usize>>,
    /// Counts nodes in the order the walk meets them, for `Ctx::open`.
    seq: u64,
    unpacked: u64,
    books: Books,
    counts: Counts,
    audit: Audit,
    build: Build,
    fields: FxHashMap<usize, std::rc::Rc<Fields>>,
    keys: FxHashMap<usize, (Ty, String, u64)>,
}

impl Follow {
    fn new(file_bits: u64) -> Follow {
        let mut audit = Audit::default();
        audit.space_bits = file_bits;
        Follow {
            space: 0,
            started: false,
            stack: Vec::new(),
            pending: None,
            root_part: (0, 0),
            part_chain: Vec::new(),
            seq: 0,
            unpacked: 0,
            books: Books::default(),
            counts: Counts::default(),
            audit,
            build: Build::default(),
            fields: FxHashMap::default(),
            keys: FxHashMap::default(),
        }
    }

    /// What a structure's declaration says about each field.
    fn fields_of(&mut self, s: &std::sync::Arc<crate::template::StructDef>) -> std::rc::Rc<Fields> {
        let id = std::sync::Arc::as_ptr(s) as usize;
        self.fields
            .entry(id)
            .or_insert_with(|| {
                // Machinery is a field whose value another field's shape reads.
                // Not one whose size alone it reads: a string that padding
                // after it is measured from is still the string.
                let consumers = machinery::consumers(s);
                let machinery = (0..s.fields.len())
                    .map(|i| match machinery::hint(s, i) {
                        Some(m) => m,
                        None => match consumers.get(i).copied().flatten() {
                            Some(j) => s.fields.get(j).is_some_and(|g| value_used(&g.ty, &s.fields[i].name, 0)),
                            None => false,
                        },
                    })
                    .collect();
                let hinted = (0..s.fields.len()).map(|i| machinery::hint(s, i) == Some(true)).collect();
                // Of the siblings a field's length reads, the one that is a
                // number, and of those the nearest: a RIFF chunk's body reads
                // its `id` as well as its `size`, and the size is the length.
                let measured = machinery::measurers(s);
                let mut measured_by: Vec<Option<usize>> = vec![None; s.fields.len()];
                for (j, m) in measured.iter().enumerate() {
                    let Some(i) = *m else { continue };
                    let number = |k: usize| is_number(&s.fields[k].ty);
                    measured_by[i] = match measured_by[i] {
                        Some(k) if number(k) && !number(j) => Some(k),
                        _ => Some(j),
                    };
                }
                std::rc::Rc::new(Fields { machinery, hinted, measured_by })
            })
            .clone()
    }

    /// The diagram's key for a switch, how many shapes it can take, written
    /// out once per switch.
    fn choice_key(&mut self, t: &Template, decl: &Ty) -> Option<(String, u64)> {
        let id = super::diagram::box_identity(t, decl)?;
        let e = self.keys.entry(id).or_insert_with(|| {
            let key = super::diagram::box_key(t, decl).unwrap_or_default();
            let cases = switch_cases(t, decl).map_or(0, |n| n as u64 + 1);
            (decl.clone(), key, cases)
        });
        Some((e.1.clone(), e.2))
    }
}

/// Whether a declaration reads the value of the sibling called `name`, as a
/// length, a count, a place or a choice, rather than only its size or where
/// it starts.
fn value_used(ty: &Ty, name: &str, depth: u32) -> bool {
    if depth > 32 {
        return false;
    }
    let e = |x: &Expr| expr_reads_value(x, name, 0);
    let t = |x: &Ty| value_used(x, name, depth + 1);
    match ty {
        Ty::Bytes(x) => e(x),
        Ty::Str { len, .. } | Ty::TextInt { len, .. } => match len {
            crate::template::StrLen::Fixed(x) | crate::template::StrLen::Padded { size: x, .. } => e(x),
            _ => false,
        },
        Ty::UIntExpr { bits, .. } => e(bits),
        Ty::Array { elem, count } => e(count) || t(elem),
        Ty::Repeat { elem, until } => matches!(until, crate::template::Until::While(x) if e(x)) || t(elem),
        Ty::PointerList { offsets, adjust, elem, .. } => &**offsets == name || e(adjust) || t(elem),
        Ty::Chain { first, adjust, elem, .. } => e(first) || e(adjust) || t(elem),
        Ty::Gather { offset, adjust, elem, .. } => e(offset) || e(adjust) || t(elem),
        Ty::At { at, inner, .. } => e(at) || t(inner),
        Ty::Sized { size, inner } => e(size) || t(inner),
        Ty::SizedBits { bits, inner } => e(bits) || t(inner),
        Ty::When { cond, inner } => e(cond) || t(inner),
        Ty::Switch { on, cases, default } => e(on) || cases.iter().any(|(_, c)| t(c)) || t(default),
        Ty::Match { on, cases, default } => e(on) || cases.iter().any(|(_, c)| t(c)) || t(default),
        Ty::Origin { inner } | Ty::Enum { inner, .. } | Ty::Flags { inner, .. } | Ty::Nullable { inner, .. } => t(inner),
        Ty::Decoded { inner, .. } => t(inner),
        _ => false,
    }
}

/// Whether an expression reads the value of the field called `name`.
fn expr_reads_value(e: &Expr, name: &str, depth: u32) -> bool {
    if depth > 64 {
        return false;
    }
    let r = |x: &Expr| expr_reads_value(x, name, depth + 1);
    let first = |p: &[String]| p.first().is_some_and(|f| f == name);
    match e {
        Expr::Ref(n) | Expr::Prev(n) | Expr::SumOf(n) | Expr::MaxOf(n) | Expr::ProductOf(n) | Expr::PopCount(n) => &**n == name,
        Expr::Elem { array, index, .. } | Expr::Product { array, index, .. } => &**array == name || r(index),
        Expr::ElemWithin { path, index, .. } => first(path) || r(index),
        // A path further in reads a field of this one, not this one: a WAV
        // chunk's padding reads `body.data_size` where an RF64 file keeps the
        // real size, and the body is not machinery for that.
        Expr::Sibling(path) | Expr::Within(path) => path.len() == 1 && first(path),
        Expr::EntryOf { records, name: n } => first(records) || r(n),
        Expr::Add(a, b)
        | Expr::Sub(a, b)
        | Expr::Mul(a, b)
        | Expr::Div(a, b)
        | Expr::Mod(a, b)
        | Expr::DivCeil(a, b)
        | Expr::Or(a, b)
        | Expr::Either(a, b)
        | Expr::Both(a, b)
        | Expr::And(a, b)
        | Expr::BitOr(a, b)
        | Expr::BitXor(a, b)
        | Expr::Less(a, b)
        | Expr::Eq(a, b)
        | Expr::Ne(a, b)
        | Expr::Le(a, b)
        | Expr::Gt(a, b)
        | Expr::Ge(a, b)
        | Expr::Shl(a, b)
        | Expr::Shr(a, b)
        | Expr::Min(a, b)
        | Expr::Max(a, b) => r(a) || r(b),
        Expr::Cond { when, then, otherwise } => r(when) || r(then) || r(otherwise),
        Expr::Not(a) | Expr::BitNot(a) | Expr::Log2(a) | Expr::Pow2(a) | Expr::Pow10(a) | Expr::Trunc(a) | Expr::Bit(a, _) | Expr::RealText(a) | Expr::Placer(a) | Expr::StartOf(a) => r(a),
        Expr::PadTo { n, .. } => r(n),
        Expr::PeekAt { skip, .. } => r(skip),
        Expr::PeekIn { at, .. } => r(at),
        _ => false,
    }
}

/// Whether a declared type holds a number, which is what a length is.
fn is_number(ty: &Ty) -> bool {
    match ty {
        Ty::Enum { inner, .. } | Ty::Nullable { inner, .. } | Ty::Flags { inner, .. } => is_number(inner),
        Ty::UInt { .. }
        | Ty::Int { .. }
        | Ty::UIntExpr { .. }
        | Ty::SignMagnitude { .. }
        | Ty::Leb128 { .. }
        | Ty::Zigzag
        | Ty::Vlq
        | Ty::EbmlVint { .. }
        | Ty::SqliteVarint
        | Ty::SevenZipNumber
        | Ty::TextInt { .. }
        | Ty::Computed(_) => true,
        _ => false,
    }
}

/// How many cases a declared choice lists, not counting its default, looking
/// through the wrappers a field is declared in.
fn switch_cases(t: &Template, ty: &Ty) -> Option<usize> {
    match choice_in(t, ty)? {
        Ty::Switch { cases, .. } => Some(cases.len()),
        Ty::Match { cases, .. } => Some(cases.len()),
        _ => None,
    }
}

/// The switch a field is declared as, through a name, a window, an origin or
/// a condition, but not through a list: a list of choices is a list.
fn choice_in<'a>(t: &'a Template, ty: &'a Ty) -> Option<&'a Ty> {
    let mut ty = ty;
    for _ in 0..16 {
        ty = match ty {
            Ty::Switch { .. } | Ty::Match { .. } => return Some(ty),
            Ty::Named(n) => t.types.get(&**n)?,
            Ty::Sized { inner, .. } | Ty::SizedBits { inner, .. } | Ty::Origin { inner } | Ty::When { inner, .. } => inner,
            _ => return None,
        };
    }
    None
}

/// The structures whose fields are top-level parts, from the root down: see
/// `Follow::part_chain`. Empty for a root that is not a structure, whose
/// parts are not its children but the root itself.
fn part_chain<S: Source>(ev: &mut Evaluator, doc: &Document<S>, root: &[usize]) -> R<Vec<Vec<usize>>> {
    let mut chain = Vec::new();
    let mut at = root.to_vec();
    for _ in 0..8 {
        let fields = match ev.memo[&at].ty.base() {
            Ty::Struct(s) if !s.overlap => s.fields.len(),
            _ => break,
        };
        chain.push(at.clone());
        let whole = ev.size_of(doc, &at)?;
        let mut big = None;
        for k in 0..fields {
            let mut c = at.clone();
            c.push(k);
            let size = match ev.resolve(doc, &c).and_then(|()| ev.size_of(doc, &c)) {
                Ok(size) => size,
                Err(e) if e.interrupted() => return Err(e),
                Err(_) => break,
            };
            // Nine tenths, and a structure: a list of records is the part,
            // not a heading over a part per record.
            if size.saturating_mul(10) >= whole.saturating_mul(9) && matches!(ev.memo[&c].ty.base(), Ty::Struct(s) if !s.overlap) {
                big = Some(c);
            }
        }
        match big {
            Some(c) => at = c,
            None => break,
        }
    }
    Ok(chain)
}

/// Whether a field is a choice between records, rather than between ways of
/// writing one value.
fn picks_shape(t: &Template, ty: &Ty) -> bool {
    let cases: Vec<&Ty> = match choice_in(t, ty) {
        Some(Ty::Switch { cases, default, .. }) => cases.iter().map(|(_, c)| c).chain(std::iter::once(&**default)).collect(),
        Some(Ty::Match { cases, default, .. }) => cases.iter().map(|(_, c)| c).chain(std::iter::once(&**default)).collect(),
        _ => return false,
    };
    cases.into_iter().any(|c| matches!(named_through(t, c), Ty::Struct(_)) || element_of(named_through(t, c)).is_some())
}

/// A type with its name looked up, for the name of what a list holds.
fn named_through<'a>(t: &'a Template, ty: &'a Ty) -> &'a Ty {
    let mut ty = ty;
    for _ in 0..16 {
        ty = match ty {
            Ty::Named(n) => match t.types.get(&**n) {
                Some(inner) => inner,
                None => return ty,
            },
            Ty::Sized { inner, .. } | Ty::SizedBits { inner, .. } | Ty::Origin { inner } | Ty::When { inner, .. } => inner,
            _ => return ty,
        };
    }
    ty
}

/// The element type of a list, as declared.
fn element_of(ty: &Ty) -> Option<&Ty> {
    match ty {
        Ty::Array { elem, .. } | Ty::Repeat { elem, .. } | Ty::PointerList { elem, .. } | Ty::Chain { elem, .. } | Ty::Gather { elem, .. } => Some(elem),
        _ => None,
    }
}

impl<S: Source> Watch<S> for Follow {
    fn ready(&mut self, ev: &mut Evaluator, doc: &Document<S>, path: &[usize], r: &Resolved) -> R<()> {
        let mut out = Ready { path: path.to_vec(), absent: matches!(r.ty, Ty::When { .. }), ..Ready::default() };
        let parent = self.stack.last().map(|p| (p.path.clone(), p.part, p.group, p.machinery, p.elements.clone()));
        let elements = parent.as_ref().and_then(|p| p.4.clone());
        // The part: the root's own field, or the root itself.
        let (part, group, machinery) = match &parent {
            None => {
                self.space = r.space;
                self.part_chain = part_chain(ev, doc, path)?;
                let part = self.books.part(path, &r.name.text());
                let group = self.books.group("", "none");
                (part, group, r.machinery)
            }
            Some((ppath, ppart, pgroup, pmachinery, _)) => {
                let part = if self.part_chain.contains(ppath) { self.books.part(path, &r.name.text()) } else { *ppart };
                (part, *pgroup, *pmachinery)
            }
        };
        out.part = part;
        out.group = group;
        out.machinery = machinery || r.machinery;
        out.inherit = out.machinery;
        let mut own = false;
        let t = ev.template().clone();
        if let Some((&idx, up)) = path.split_last().filter(|_| parent.is_some()) {
            let pr = ev.memo[up].clone();
            // Whether this field is machinery, and whether a sibling gives its
            // length, from the declaration of the structure it is in.
            let mut measured_by = None;
            if let Ty::Struct(s) = pr.ty.base() {
                let f = self.fields_of(s);
                own = f.machinery.get(idx).copied().unwrap_or(false);
                out.machinery |= own;
                out.inherit |= f.hinted.get(idx).copied().unwrap_or(false);
                measured_by = f.measured_by.get(idx).copied().flatten();
                if let Some(field) = s.fields.get(idx) {
                    out.checks = field.checks.iter().map(|c| c.algorithm.as_str()).collect();
                }
            }
            // The group, for an element of a list.
            if let Some(elements) = elements {
                out.group = match elements {
                    Elements::Inherit => out.group,
                    Elements::Fixed(g) => g,
                    Elements::Choice(decl) => {
                        let (name, from) = variant(ev, doc, path, r, &decl, path, &r.ty)?;
                        self.books.group(&name, from)
                    }
                    Elements::Field(k, decl) => {
                        let mut child = path.to_vec();
                        child.push(k);
                        let (name, from) = match ev.resolve(doc, &child) {
                            Ok(()) => {
                                let ty = ev.memo[&child].ty.clone();
                                variant(ev, doc, path, r, &decl, &child, &ty)?
                            }
                            Err(e) if e.interrupted() => return Err(e),
                            Err(_) => (r.ty.display_name(), "type"),
                        };
                        self.books.group(&name, from)
                    }
                };
            }
            // How it was placed, and for a field an offset placed, whether the
            // offset points forward or back.
            let (kind, by_offset, pointer, end) = match &pr.ty {
                Ty::Struct(s) if s.overlap => ("overlap", false, None, false),
                Ty::Struct(_) | Ty::Json(..) | Ty::Pickle(..) => ("follows", false, None, false),
                Ty::Array { .. } | Ty::Repeat { .. } => ("element", false, None, false),
                Ty::At { anchor, at, .. } => (at_kind(*anchor, at), true, Some(pr.offset), from_end(at)),
                Ty::PointerList { adjust, .. } => ("pointer-list", true, pointer_from(ev, doc, path)?, from_end(adjust)),
                Ty::Chain { first, adjust, .. } => {
                    let before = match idx.checked_sub(1) {
                        Some(i) => ev.list(up).chain_starts.get(i).copied(),
                        None => Some(pr.offset),
                    };
                    ("chain", true, before, from_end(first) || from_end(adjust))
                }
                Ty::Gather { offset, adjust, .. } => {
                    let record = ev.list(up).gather.as_ref().and_then(|g| g.records.get(idx).cloned());
                    let at = match record {
                        Some(rec) => match ev.resolve(doc, &rec) {
                            Ok(()) => Some(ev.memo[&rec].offset),
                            Err(e) if e.interrupted() => return Err(e),
                            Err(_) => None,
                        },
                        None => None,
                    };
                    ("gather", true, at, from_end(offset) || from_end(adjust))
                }
                _ => ("other", false, None, false),
            };
            if !matches!(r.ty, Ty::At { .. }) {
                out.placement = Some(kind);
            }
            out.by_offset = by_offset || r.elsewhere;
            out.forward = pointer.map(|p| r.offset >= p);
            out.from_end = end;
            // The switch this field was declared as, and the case it took.
            let decl = match &pr.ty {
                Ty::Struct(s) => s.fields.get(idx).map(|f| f.ty.clone()),
                other => element_of(other).cloned(),
            };
            if let Some(decl) = decl.filter(|d| choice_in(&t, d).is_some()) {
                if let Some((key, cases)) = self.choice_key(&t, &decl) {
                    let name = match &pr.ty {
                        Ty::Struct(s) => s.fields.get(idx).map(|f| f.name.to_string()).unwrap_or_default(),
                        _ => pr.name.text(),
                    };
                    out.choice = Some((key, name, cases, ev.case_taken(&decl, &r.ty)));
                }
            }
            // The audit, for a part a sibling gives the length of.
            if let (Some(by), false) = (measured_by, out.absent) {
                if let Some(check) = ev.check_part(doc, path, by)? {
                    let mut length = up.to_vec();
                    length.push(by);
                    out.length_after = ev.memo.get(&length).map(|l| l.offset > r.offset);
                    out.check = Some(check);
                }
            }
        }
        // How long it is.
        if !out.absent {
            out.sizing = sizing_kind(ev.shape(doc, path)?.sized);
        }
        // A stream: what it came to, when it is open or small enough to open.
        if matches!(r.ty, Ty::Decoded { .. }) {
            let size = ev.size_of(doc, path)?;
            out.unpacked = match ev.spaces.get(path) {
                Some(Opened::Space(id)) => Some(ev.spaces.len_bits(id)),
                Some(Opened::Refused(_)) => None,
                None if size <= UNPACK_ONE && self.unpacked + size <= UNPACK_ALL => match ev.open_space_at(doc, path)? {
                    Opened::Space(id) => Some(ev.spaces.len_bits(id)),
                    Opened::Refused(_) => None,
                },
                None => None,
            };
        }
        // For a list, how its elements get their group.
        if let Some(elem) = element_of(&r.ty) {
            out.elements = Some(if choice_in(&t, elem).is_some() {
                Elements::Choice(elem.clone())
            } else {
                match named_through(&t, elem) {
                    // The field that picks a shape for the record, rather than
                    // one that picks how a number is written: a MIDI event's
                    // message, not its status byte.
                    Ty::Struct(s) => match s.fields.iter().position(|f| picks_shape(&t, &f.ty)) {
                        Some(k) => Elements::Field(k, s.fields[k].ty.clone()),
                        None => Elements::Fixed(self.books.group(&s.name, "type")),
                    },
                    // A run of plain values is not a run of records: a WAV's
                    // samples are the `data` chunk's, and grouping them as
                    // `i16 le` would say nothing the type column does not.
                    _ => Elements::Inherit,
                }
            });
            // A run of plain values another field depends on is machinery all
            // through: a SQLite page's cell pointers.
            if own && matches!(out.elements, Some(Elements::Inherit)) {
                out.inherit = true;
            }
        }
        self.pending = Some(out);
        Ok(())
    }

    /// A second reading is counted nowhere in the ledger, but an address that
    /// is one still says which way the file points: a ZIP's central directory
    /// points back at every local header.
    fn aside(&mut self, ev: &mut Evaluator, doc: &Document<S>, path: &[usize], r: &Resolved) -> R<()> {
        let Ty::At { anchor, at, .. } = &r.ty else { return Ok(()) };
        let (kind, end) = (at_kind(*anchor, at), from_end(at));
        let mut child = path.to_vec();
        child.push(0);
        let target = match ev.resolve(doc, &child) {
            Ok(()) => ev.memo[&child].offset,
            Err(e) if e.interrupted() => return Err(e),
            Err(_) => return Ok(()),
        };
        self.counts.add(Key::of("placement", kind), 1, 0);
        let f = &mut self.counts.facts;
        f.placed += 1;
        if target >= r.offset {
            f.forward += 1;
        } else {
            f.backward += 1;
        }
        if end {
            f.from_end += 1;
        }
        Ok(())
    }

    fn failed(&mut self, ev: &mut Evaluator, doc: &Document<S>, parent: &[usize], idx: u64, at: u64, why: &str) -> R<()> {
        let check = ev.diagnose(doc, parent, idx as usize, at, why, 0)?;
        if let Some(c) = check {
            self.audit.add(c, 1);
        }
        if let Some(p) = self.stack.last_mut() {
            p.failed_at.get_or_insert(idx);
        }
        Ok(())
    }

    fn closing(&mut self, ev: &mut Evaluator, doc: &Document<S>, f: &Closing) -> R<()> {
        // Where the root's own fields end, asked again now that they are all
        // read: a run stretched on the way moves it.
        if self.stack.len() == 1 {
            self.audit.root_end_bits = ev.memo[f.path].offset + ev.size_of(doc, f.path)?;
        }
        // A run the walk stretched to take in an element past its room: its
        // length said less than it holds.
        if let (Some(c), Some(r)) = (self.stack.last_mut().and_then(|p| p.check.as_mut()), ev.memo.get(f.path)) {
            if let (Some(read), Some(now)) = (c.read, r.declared_size) {
                if c.role == "length" && now > read {
                    c.read = Some(now);
                    c.verdict = super::extent::length_verdict(c.offset_bits, c.stated.unwrap_or(0), now, c.space_bits);
                }
            }
        }
        // A run that stopped at an element it could not read: what that
        // element's length said, which `walk.rs` did not keep.
        let trouble = {
            let l = ev.list(f.path);
            l.repeat_trouble.clone().map(|why| (why, l.repeat_len, l.repeat_end))
        };
        let Some((why, len, end)) = trouble else { return Ok(()) };
        let at = end.unwrap_or(f.offset);
        let check = ev.diagnose(doc, f.path, len, at, &why, 0)?;
        if let Some(p) = self.stack.last_mut() {
            p.trouble = check;
        }
        Ok(())
    }

    fn leaf(&mut self, ev: &Evaluator, path: &[usize], r: &Resolved, bits: u64, scale: u64) {
        let Some(ready) = self.take(path) else { return };
        self.seq += 1;
        let size = bits / scale.max(1);
        if ready.absent {
            return;
        }
        if self.stack.is_empty() {
            self.root_part = (ready.part, ready.group);
            self.started = true;
        }
        // The ledger.
        match r.ty.base() {
            Ty::Bytes(e) if super::profile::padding_align(e).is_some() => {
                let align = super::profile::padding_align(e).unwrap_or(0);
                self.books.padding(ready.part, ready.group, align, r.offset, r.offset + size, scale, path);
            }
            _ => {
                let role = if ready.machinery { "machinery" } else { "content" };
                self.books.field(ready.part, ready.group, role, 0, bits, scale, path, r.offset);
            }
        }
        if ready.by_offset && scale == 1 && size > 0 {
            self.books.placed(r.offset, r.offset + size, self.seq);
        }
        self.reached(r.offset, r.offset + size);
        // The profile.
        let mut keys = Vec::new();
        value_keys(&r.ty, Some(size), &mut keys);
        let number = keys.iter().any(|k| k.category == "number");
        let bitfield = keys.iter().any(|k| k.category == "bit-field");
        if number && !bitfield && (r.offset % 8 != 0) {
            keys.push(Key::of("bit-field", ""));
        }
        for k in keys {
            if k.category == "codec" {
                match ready.unpacked {
                    Some(u) => {
                        self.counts.unpacked(k.clone(), scale, u.saturating_mul(scale));
                        self.unpacked = self.unpacked.saturating_add(size);
                    }
                    None => {}
                }
            }
            self.counts.add(k, scale, bits);
        }
        self.count_node(&ready, bits, scale);
        // The audit: a part that is one value fits when its length does.
        if let Some(c) = ready.check {
            self.audit.add(c, scale);
        }
        let _ = ev;
    }

    fn open(&mut self, ev: &Evaluator, path: &[usize], r: &Resolved, scale: u64) {
        let Some(ready) = self.take(path) else { return };
        self.seq += 1;
        let size = r.size.unwrap_or(0);
        if self.stack.is_empty() {
            self.root_part = (ready.part, ready.group);
            self.started = true;
            self.audit.root_end_bits = r.offset + size;
        }
        let placed = (ready.by_offset && scale == 1 && size > 0).then(|| self.books.placed(r.offset, r.offset + size, self.seq));
        self.reached(r.offset, r.offset + size);
        self.count_node(&ready, size.saturating_mul(scale), scale);
        if scale == 1 {
            match &r.ty {
                Ty::PointerList { .. } => self.build.list(path, false),
                Ty::Gather { .. } => self.build.list(path, true),
                _ => {}
            }
        }
        self.stack.push(Ctx {
            path: path.to_vec(),
            part: ready.part,
            group: ready.group,
            machinery: ready.inherit,
            open: self.seq,
            placed,
            check: ready.check,
            elements: ready.elements,
            failed_at: None,
            trouble: None,
            offset: r.offset,
            end: r.offset + size,
            reach: r.offset,
        });
        let _ = ev;
    }

    fn gap(&mut self, parent: &[usize], from: u64, to: u64, scale: u64) {
        let (part, group, open) = match self.stack.last() {
            Some(c) => (c.part, c.group, c.open),
            None => (self.root_part.0, self.root_part.1, 0),
        };
        self.books.gap(part, group, from, to, scale, open, parent);
    }

    fn close(&mut self, _ev: &Evaluator, f: &Closing) {
        let Some(ctx) = self.stack.pop() else { return };
        // How far its fields reached: the ones laid out in order, and the
        // ones placed inside it by an offset, which a SQLite page's cells are.
        let content = f.cursor.max(ctx.reach).min(f.end);
        if let Some(p) = ctx.placed {
            self.books.closed(p, self.seq);
        }
        if f.leftover > 0 {
            if f.framed {
                self.books.field(ctx.part, ctx.group, "framing", 0, f.leftover.saturating_mul(f.scale), f.scale, f.path, f.cursor);
            } else if f.stretch {
                self.books.gap(ctx.part, ctx.group, f.cursor, f.end, f.scale, ctx.open, f.path);
            } else {
                // A region whose children an offset scattered: all of it, less
                // the children, which take themselves out. See `ledger.rs`.
                self.books.gap(ctx.part, ctx.group, f.offset, f.end, f.scale, ctx.open, f.path);
            }
        }
        if let Some(mut c) = ctx.check {
            if c.verdict == "fits" {
                if c.role == "count" {
                    if let Some(k) = ctx.failed_at {
                        c.read = Some(k);
                        c.verdict = "past-parent";
                    }
                } else if f.stretch && !f.framed && content < f.end {
                    c.verdict = "short";
                }
            }
            if f.stretch {
                c.content_bits = Some(content.saturating_sub(f.offset));
            }
            if let Some(t) = &ctx.trouble {
                if c.why.is_empty() {
                    c.why = t.why.clone();
                }
            }
            self.audit.add(c, f.scale);
        }
        if let Some(t) = ctx.trouble {
            self.audit.add(t, 1);
        }
    }
}

impl Follow {
    /// A child of the frame on top covers `from` to `to`. Only what lands
    /// inside the frame says how far the frame's own fields reach.
    fn reached(&mut self, from: u64, to: u64) {
        if let Some(c) = self.stack.last_mut() {
            if from >= c.offset && to <= c.end {
                c.reach = c.reach.max(to);
            }
        }
    }

    /// What `ready` worked out for the node at `path`.
    fn take(&mut self, path: &[usize]) -> Option<Ready> {
        match self.pending.take() {
            Some(r) if r.path == path => Some(r),
            _ => None,
        }
    }

    /// The profile rows every node counts in, whatever it holds: how it was
    /// placed, how it was sized, what checks it and what it chose.
    fn count_node(&mut self, ready: &Ready, bits: u64, scale: u64) {
        if ready.absent {
            return;
        }
        if let Some(p) = ready.placement {
            self.counts.add(Key::of("placement", p), scale, bits);
        }
        if let Some(s) = ready.sizing {
            self.counts.add(Key::of("sizing", s), scale, bits);
        }
        for c in &ready.checks {
            self.counts.add(Key::of("checksum", c), scale, bits);
        }
        if let Some((key, name, cases, case)) = &ready.choice {
            self.counts.chose(key, name, *cases, *case, scale);
        }
        let f = &mut self.counts.facts;
        if ready.by_offset {
            f.placed += scale;
            match ready.forward {
                Some(true) => f.forward += scale,
                Some(false) => f.backward += scale,
                None => {}
            }
        }
        if ready.from_end {
            f.from_end += scale;
        }
        match ready.length_after {
            Some(true) => f.lengths_after += scale,
            Some(false) => f.lengths_before += scale,
            None => {}
        }
    }
}

/// The name of the case a choice took, for a list element's group.
///
/// First, the value that picked it, where that is a field whose value has a
/// name: a MIDI event's status (`note on ch1`), a ZIP record's signature
/// (`local file`), a chunk's four letters (`IDAT`). Then the case's own type
/// where that is a record: a SQLite page's `TableLeaf`. Where the case is not
/// a record, which is usually "the bytes" a switch falls back to, the
/// element's own name says more than the type does: an ELF section of plain
/// bytes is `.rodata`.
///
/// `path` is the element, `at` the field that made the choice, which is the
/// element itself when the element is the choice.
#[allow(clippy::too_many_arguments)]
fn variant<S: Source>(ev: &mut Evaluator, doc: &Document<S>, path: &[usize], r: &Resolved, decl: &Ty, at: &[usize], taken: &Ty) -> R<(String, &'static str)> {
    let t = ev.template().clone();
    let on = match choice_in(&t, decl) {
        Some(Ty::Switch { on: Expr::Ref(name), .. }) | Some(Ty::Match { on: Expr::Ref(name), .. }) => Some(name.clone()),
        _ => None,
    };
    if let Some(key) = on.and_then(|name| ev.find_field(at, &name)) {
        let value = match ev.value_of(doc, &key) {
            Ok(v) => Some(v.value),
            Err(e) if e.interrupted() => return Err(e),
            Err(_) => None,
        };
        if let Some(name) = value.as_ref().and_then(named_value) {
            return Ok((name, "key"));
        }
    }
    // A record is named by its type. Anything else, the bytes a switch falls
    // back to or a run of values, is named better by the element itself,
    // where the listing has a name for it.
    if matches!(taken.base(), Ty::Struct(_)) {
        return Ok((taken.display_name(), "case"));
    }
    let label = ev.label(doc, path, r)?;
    Ok(match without_index(&label) {
        Some(name) => (name.to_string(), "name"),
        None => (taken.display_name(), "case"),
    })
}

/// A value read as a name, where it is one: an enum's name for its number,
/// text, or four bytes that are all letters. A plain number is not a name.
fn named_value(v: &Value) -> Option<String> {
    match v {
        Value::Enum { name: Some(n), .. } => Some(n.clone()),
        Value::Str(s) if !s.trim().is_empty() => Some(s.clone()),
        Value::Bytes { len, preview } if *len <= 8 && !preview.is_empty() && preview.iter().all(|b| b.is_ascii_graphic() || *b == b' ') => {
            Some(String::from_utf8_lossy(preview).into_owned())
        }
        Value::Magic { bytes, .. } if bytes.iter().all(|b| b.is_ascii_graphic() || *b == b' ') => Some(String::from_utf8_lossy(bytes).into_owned()),
        Value::Unset(inner) => named_value(inner),
        _ => None,
    }
}

/// Where the entry that placed child `path` of a list of offsets is: the
/// field its offset was read from.
fn pointer_from<S: Source>(ev: &mut Evaluator, doc: &Document<S>, path: &[usize]) -> R<Option<u64>> {
    let origins = match ev.origins_no_values(doc, path) {
        Ok(o) => o,
        Err(e) if e.interrupted() => return Err(e),
        Err(_) => return Ok(None),
    };
    let Some(o) = origins.into_iter().find(|o| o.role == Role::Position && !o.path.is_empty()) else { return Ok(None) };
    match ev.resolve(doc, &o.path) {
        Ok(()) => Ok(Some(ev.memo[&o.path].offset)),
        Err(e) if e.interrupted() => Err(e),
        Err(_) => Ok(None),
    }
}
